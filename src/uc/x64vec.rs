//! The machine code of the IR's vector operations (SSE2 only: what every
//! x86-64 has, as clang -O2 targets without -march). Both generators use
//! these: x64.rs keeps every vector local in its frame and works in xmm3
//! and xmm4, x64opt.rs keeps each in a callee-saved xmm register. xmm0 and
//! xmm1 are the scratch of single operations, as for floats.
//!
//! Every operation is exact lane by lane:
//! - addpd, subpd, mulpd, divpd and sqrtpd round each lane as addsd and the
//!   rest do, and with NaNs take the first operand's, as the scalar ones;
//!   the operands are never swapped.
//! - a 64-bit product has no SSE2 instruction: it is three pmuludq, as
//!   clang makes it (lo*lo + (hi*lo + lo*hi) << 32, modulo 2^64);
//! - a 32-bit product is two pmuludq, the odd lanes and the even;
//! - a 64-bit arithmetic shift right has none either: the logical shift,
//!   with the sign shifted in from the top.

use super::ir::{BinOp, Lanes, Op, UnOp};
use super::x64::{Asm, Target};

const X0: u8 = 0;
const X1: u8 = 1;

/// prefix, a REX for xmm8 and up, 0F, the opcode, and the register form's
/// ModRM (reg, rm).
pub(super) fn rr(a: &mut Asm, prefix: u8, op: u8, reg: u8, rm: u8) {
    if prefix != 0 {
        a.e(&[prefix]);
    }
    if reg >= 8 || rm >= 8 {
        a.e(&[0x40 | (reg >> 3) << 2 | (rm >> 3)]);
    }
    a.e(&[0x0F, op, 0xC0 | (reg & 7) << 3 | (rm & 7)]);
}

/// movdqa d, x (nothing when they are the same register)
pub(super) fn copy(a: &mut Asm, d: u8, x: u8) {
    if d != x {
        rr(a, 0x66, 0x6F, d, x);
    }
}

/// A shift of every lane by the constant k: 0x73 /6 psllq, /2 psrlq; 0x72
/// /6 pslld, /4 psrad, /2 psrld.
fn shift_imm(a: &mut Asm, op: u8, ext: u8, x: u8, k: u8) {
    rr(a, 0x66, op, ext, x);
    a.e(&[k]);
}

fn pshufd(a: &mut Asm, d: u8, x: u8, imm: u8) {
    rr(a, 0x66, 0x70, d, x);
    a.e(&[imm]);
}

/// The opcode of a lane-wise operation that SSE2 has as one instruction,
/// and whether its operands may change places (exactly: the integer ones).
fn simple(l: Lanes, b: BinOp) -> Option<(u8, bool)> {
    use BinOp::*;
    Some(match (l, b) {
        (Lanes::I64, Add) => (0xD4, true),  // paddq
        (Lanes::I64, Sub) => (0xFB, false), // psubq
        (Lanes::I32, Add) => (0xFE, true),  // paddd
        (Lanes::I32, Sub) => (0xFA, false), // psubd
        (Lanes::I64 | Lanes::I32, Band) => (0xDB, true),
        (Lanes::I64 | Lanes::I32, Bor) => (0xEB, true),
        (Lanes::I64 | Lanes::I32, Bxor) => (0xEF, true),
        (Lanes::F64, Band) => (0x54, false), // andpd: bits, but kept in order
        (Lanes::F64, Bor) => (0x56, false),
        (Lanes::F64, Bxor) => (0x57, false),
        (Lanes::F64, FAdd) => (0x58, false),
        (Lanes::F64, FMul) => (0x59, false),
        (Lanes::F64, FSub) => (0x5C, false),
        (Lanes::F64, FDiv) => (0x5E, false),
        _ => return None,
    })
}

/// d = x op y in registers (d may be x or y).
pub(super) fn bin(a: &mut Asm, l: Lanes, b: BinOp, d: u8, x: u8, y: u8) {
    if let Some((op, swaps)) = simple(l, b) {
        if d == x {
            rr(a, 0x66, op, d, y);
        } else if d == y && swaps {
            rr(a, 0x66, op, d, x);
        } else if d == y {
            copy(a, X0, y);
            copy(a, d, x);
            rr(a, 0x66, op, d, X0);
        } else {
            copy(a, d, x);
            rr(a, 0x66, op, d, y);
        }
        return;
    }
    match (l, b) {
        (Lanes::I64, BinOp::Mul) => {
            // hi(x) lo(y) + lo(x) hi(y), shifted up, plus lo(x) lo(y)
            copy(a, X0, x);
            shift_imm(a, 0x73, 2, X0, 32);
            rr(a, 0x66, 0xF4, X0, y); // pmuludq
            copy(a, X1, y);
            shift_imm(a, 0x73, 2, X1, 32);
            rr(a, 0x66, 0xF4, X1, x);
            rr(a, 0x66, 0xD4, X0, X1);
            shift_imm(a, 0x73, 6, X0, 32);
            low_products(a, d, x, y);
            rr(a, 0x66, 0xD4, d, X0);
        }
        (Lanes::I32, BinOp::Mul) => {
            // the odd lanes' products, then the even ones', their low halves
            // put back between each other
            pshufd(a, X0, x, 0xF5);
            pshufd(a, X1, y, 0xF5);
            rr(a, 0x66, 0xF4, X0, X1);
            pshufd(a, X0, X0, 0x08);
            low_products(a, d, x, y);
            pshufd(a, d, d, 0x08);
            rr(a, 0x66, 0x62, d, X0); // punpckldq
        }
        _ => unreachable!("vectorize makes no vector {b:?} of {l:?}"),
    }
}

/// d = pmuludq(x, y), which takes either order
fn low_products(a: &mut Asm, d: u8, x: u8, y: u8) {
    if d == y {
        rr(a, 0x66, 0xF4, d, x);
    } else {
        copy(a, d, x);
        rr(a, 0x66, 0xF4, d, y);
    }
}

/// d = x shifted by k (Shl, or Shr arithmetic).
pub(super) fn shift(a: &mut Asm, l: Lanes, b: BinOp, d: u8, x: u8, k: u8) {
    match (l, b) {
        (_, _) if k == 0 => copy(a, d, x),
        (Lanes::I64, BinOp::Shl) => {
            copy(a, d, x);
            shift_imm(a, 0x73, 6, d, k);
        }
        (Lanes::I32, BinOp::Shl) => {
            copy(a, d, x);
            shift_imm(a, 0x72, 6, d, k);
        }
        (Lanes::I32, BinOp::Shr) => {
            copy(a, d, x);
            shift_imm(a, 0x72, 4, d, k);
        }
        (Lanes::I64, BinOp::Shr) => {
            // every lane's sign in all its bits, put at the top k bits, over
            // the logical shift
            pshufd(a, X0, x, 0xF5);
            shift_imm(a, 0x72, 4, X0, 31);
            shift_imm(a, 0x73, 6, X0, 64 - k);
            copy(a, d, x);
            shift_imm(a, 0x73, 2, d, k);
            rr(a, 0x66, 0xEB, d, X0);
        }
        _ => unreachable!("vectorize makes no vector shift {b:?} of {l:?}"),
    }
}

/// The constant xmm0 is loaded with for FNeg (the sign bits) and FAbs (all
/// but them).
fn sign_mask(a: &mut Asm, keep_rest: bool) {
    let m: u64 = if keep_rest { i64::MAX as u64 } else { 1 << 63 };
    let mut bytes = m.to_le_bytes().to_vec();
    bytes.extend_from_slice(&m.to_le_bytes());
    a.e(&[0xF3, 0x0F, 0x6F, 0x05]); // movdqu xmm0, [rip + mask]
    a.reloc(Target::Data(bytes));
}

/// d = op x.
pub(super) fn un(a: &mut Asm, l: Lanes, u: UnOp, d: u8, x: u8) {
    match (l, u) {
        (Lanes::F64, UnOp::FSqrt) => rr(a, 0x66, 0x51, d, x),
        (Lanes::F64, UnOp::FNeg | UnOp::FAbs) => {
            sign_mask(a, u == UnOp::FAbs);
            copy(a, d, x);
            rr(a, 0x66, if u == UnOp::FAbs { 0x54 } else { 0x57 }, d, X0); // andpd / xorpd
        }
        _ => unreachable!("vectorize makes no vector {u:?} of {l:?}"),
    }
}

/// d = every lane the low lane of d (which a movq or movd has just filled).
fn broadcast(a: &mut Asm, l: Lanes, d: u8) {
    match l {
        Lanes::I32 => pshufd(a, d, d, 0x00),
        Lanes::I64 | Lanes::F64 => rr(a, 0x66, 0x6C, d, d), // punpcklqdq
    }
}

/// d = every lane the general register r.
pub(super) fn splat_gpr(a: &mut Asm, l: Lanes, d: u8, r: u8) {
    // movq xmm, r64 (movd for 32-bit lanes: the low half is enough)
    let w = if l == Lanes::I32 { 0 } else { 0x08 };
    a.e(&[0x66]);
    if w != 0 || d >= 8 || r >= 8 {
        a.e(&[0x40 | w | (d >> 3) << 2 | (r >> 3)]);
    }
    a.e(&[0x0F, 0x6E, 0xC0 | (d & 7) << 3 | (r & 7)]);
    broadcast(a, l, d);
}

/// d = every lane the low double of xmm x.
pub(super) fn splat_xmm(a: &mut Asm, d: u8, x: u8) {
    copy(a, d, x);
    rr(a, 0x66, 0x14, d, d); // unpcklpd
}

/// d = the 16 bytes (from the read-only data).
pub(super) fn constant(a: &mut Asm, d: u8, lo: i64, hi: i64) {
    let mut bytes = lo.to_le_bytes().to_vec();
    bytes.extend_from_slice(&hi.to_le_bytes());
    a.e(&[0xF3]);
    if d >= 8 {
        a.e(&[0x44]);
    }
    a.e(&[0x0F, 0x6F, 0x05 | (d & 7) << 3]); // movdqu xmm, [rip + c]
    a.reloc(Target::Data(bytes));
}

/// The lanes of x combined by op, into the general register r (an I32
/// lane's sign-extended). Uses xmm0 and xmm1.
pub(super) fn reduce(a: &mut Asm, l: Lanes, b: BinOp, r: u8, x: u8) {
    let (op, _) = simple(l, b).expect("an integer reduction");
    pshufd(a, X0, x, 0xEE); // the high half down
    rr(a, 0x66, op, X0, x);
    if l == Lanes::I32 {
        pshufd(a, X1, X0, 0x55); // lane 1 down
        rr(a, 0x66, op, X0, X1);
        // movd r32, xmm0; movsxd r, r32
        a.e(&[0x66]);
        if r >= 8 {
            a.e(&[0x41]);
        }
        a.e(&[0x0F, 0x7E, 0xC0 | (r & 7)]);
        a.e(&[0x48 | (r >> 3) << 2 | (r >> 3), 0x63, 0xC0 | (r & 7) << 3 | (r & 7)]);
    } else {
        a.e(&[0x66, 0x48 | (r >> 3), 0x0F, 0x7E, 0xC0 | (r & 7)]); // movq r, xmm0
    }
}

/// The opcode of a load (to the register) and of a store of 16 bytes
/// anywhere (no alignment asked): movupd for doubles, movdqu for integers.
pub(super) fn moves(l: Lanes) -> (u8, u8, u8) {
    match l {
        Lanes::F64 => (0x66, 0x10, 0x11),
        Lanes::I64 | Lanes::I32 => (0xF3, 0x6F, 0x7F),
    }
}

/// The plain generator's vector local v, below the function's own memory.
pub(super) fn plain_slot(memory: i32, v: u8) -> i32 {
    memory - 16 * (v as i32 + 1)
}

/// op xmm, [rbp + d] (xmm below 8)
fn rbp(a: &mut Asm, prefix: u8, op: u8, xmm: u8, d: i32) {
    a.e(&[prefix, 0x0F, op, 0x85 | xmm << 3]);
    a.d32(d);
}

/// The plain generator's code for a vector operation: the IR's stack is the
/// machine's, every vector local is 16 bytes of the frame, the work is
/// done in xmm3 and xmm4.
pub(super) fn plain(a: &mut Asm, op: &Op, memory: i32, depth: &mut u32) {
    const A: u8 = 3;
    const B: u8 = 4;
    let at = |v: u8| plain_slot(memory, v);
    let load = |a: &mut Asm, x: u8, v: u8| rbp(a, 0xF3, 0x6F, x, at(v));
    let store = |a: &mut Asm, x: u8, v: u8| rbp(a, 0xF3, 0x7F, x, at(v));
    match op {
        Op::VLoad(l, v) => {
            let (p, ld, _) = moves(*l);
            a.e(&[0x58, p, 0x0F, ld, A << 3]); // pop rax; movdqu xmm3, [rax]
            store(a, A, *v);
            *depth -= 1;
        }
        Op::VStore(l, v) => {
            let (p, _, st) = moves(*l);
            load(a, A, *v);
            a.e(&[0x58, p, 0x0F, st, A << 3]); // pop rax; movdqu [rax], xmm3
            *depth -= 1;
        }
        Op::VSplat(l, v) => {
            a.e(&[0x58]);
            splat_gpr(a, *l, A, 0);
            store(a, A, *v);
            *depth -= 1;
        }
        Op::VConst(v, lo, hi) => {
            constant(a, A, *lo, *hi);
            store(a, A, *v);
        }
        Op::VBin(l, b, d, x, y) => {
            load(a, A, *x);
            load(a, B, *y);
            bin(a, *l, *b, A, A, B);
            store(a, A, *d);
        }
        Op::VShift(l, b, d, x, k) => {
            load(a, A, *x);
            shift(a, *l, *b, A, A, *k);
            store(a, A, *d);
        }
        Op::VUn(l, u, d, x) => {
            load(a, A, *x);
            un(a, *l, *u, A, A);
            store(a, A, *d);
        }
        Op::VReduce(l, b, v) => {
            load(a, A, *v);
            reduce(a, *l, *b, 0, A);
            a.e(&[0x50]); // push rax
            *depth += 1;
        }
        _ => unreachable!("not a vector operation"),
    }
}

/// Zero for every vector local of the plain generator's frame.
pub(super) fn plain_zero(a: &mut Asm, memory: i32, count: u32) {
    if count == 0 {
        return;
    }
    a.e(&[0x66, 0x0F, 0xEF, 0xC0]); // pxor xmm0, xmm0
    for v in 0..count {
        rbp(a, 0xF3, 0x7F, 0, plain_slot(memory, v as u8));
    }
}

/// The optimizing generator's register for vector local v: the last of
/// the callee-saved xmm6 to xmm15 (float locals take the first).
pub(super) fn reg(v: u8) -> u8 {
    assert!(v < 10, "vectorize keeps to ten vector locals");
    15 - v
}

#[cfg(test)]
mod tests {
    use super::super::ir::{sext, BinOp, Func, Lanes, Op, UnOp, DEFAULT_NAN};
    use super::super::{eval, pe, x64, x64opt};

    pub(crate) struct Rng(pub u64);
    impl Rng {
        pub fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    fn edges(l: Lanes) -> Vec<i64> {
        let f = |x: f64| x.to_bits() as i64;
        match l {
            Lanes::I64 => vec![0, 1, -1, 2, i64::MIN, i64::MAX, i32::MIN as i64, i32::MAX as i64, 1 << 32, (1 << 32) - 1, -(1 << 32), 0x1234_5678_9ABC_DEF0, 3, -7],
            Lanes::I32 => vec![0, 1, -1, 2, i32::MIN as i64, i32::MAX as i64, 0x7FFF, -0x8000, 65536, 3, -7, 0x1234_5678],
            Lanes::F64 => vec![
                f(0.0),
                f(-0.0),
                f(1.0),
                f(-1.5),
                f(f64::INFINITY),
                f(f64::NEG_INFINITY),
                f(f64::MIN_POSITIVE),
                f(5e-324),
                f(f64::MAX),
                f(0.1),
                f(3.0),
                DEFAULT_NAN,
                0x7FF8_0000_0000_0001,           // a quiet NaN with a payload
                0x7FF0_0000_0000_0001,           // a signalling NaN
                0xFFF4_0000_0000_0123u64 as i64, // a negative signalling NaN
                f(-2.0),
            ],
        }
    }

    fn value(l: Lanes, r: &mut Rng) -> i64 {
        let e = edges(l);
        let x = r.next();
        if x % 3 == 0 {
            return e[(x >> 8) as usize % e.len()];
        }
        let v = r.next() as i64;
        match l {
            Lanes::I32 => sext(v, 32),
            Lanes::I64 => v,
            // doubles of every size, and now and then any bits (a NaN's too)
            Lanes::F64 if x % 3 == 1 => (r.next() as f64 / 1e9 - 9e9).to_bits() as i64,
            Lanes::F64 => v,
        }
    }

    /// The lanes stored to globals g and g + 1.
    fn put(code: &mut Vec<Op>, l: Lanes, g: u32, xs: &[i64]) {
        let v = super::super::vectorize::from_lanes(l, xs);
        for h in 0..2 {
            code.extend([Op::AddrG(g + h), Op::Push(v[h as usize] as i64), Op::StoreM(8)]);
        }
    }

    fn space() -> Op {
        Op::PrintStr(b" ".to_vec())
    }
    fn line() -> Op {
        Op::PrintStr(b"\n".to_vec())
    }

    /// The lanes at global g printed.
    fn print(code: &mut Vec<Op>, l: Lanes, g: u32) {
        for i in 0..l.count() {
            code.extend([Op::AddrG(g), Op::Push(i * l.width() as i64), Op::Bin(BinOp::Add), Op::LoadM(l.width()), Op::PrintInt, space()]);
        }
        code.push(line());
    }

    fn scalar_bin(code: &mut Vec<Op>, l: Lanes, b: BinOp, x: &[i64], y: &[i64]) {
        for i in 0..l.count() as usize {
            code.extend([Op::Push(x[i]), Op::Push(y[i]), Op::Bin(b)]);
            if l == Lanes::I32 {
                code.push(Op::Sext(32));
            }
            code.extend([Op::PrintInt, space()]);
        }
        code.push(line());
    }

    fn finish(mut code: Vec<Op>) -> Func {
        code.extend([Op::Push(0), Op::Ret]);
        Func::new("t.main", 0, 0, code)
    }

    /// The output of the machine code (plain or optimized) of f.
    fn run(f: &Func, opt: bool, tag: &str) -> String {
        let rules = x64opt::Rules::read().unwrap();
        let code = if opt { x64opt::compile(f, &rules, true) } else { x64::compile(f) };
        let mut funcs = x64::runtime(&f.name);
        funcs.push((f.name.clone(), code));
        let exe = pe::link(&funcs, "rt.start", x64::GLOBALS + 8 * 16).unwrap();
        let path = std::env::temp_dir().join(format!("strike-vec-{}-{tag}-{opt}.exe", std::process::id()));
        std::fs::write(&path, exe).unwrap();
        let out = std::process::Command::new(&path).output().unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(out.status.success(), "{tag}: the program failed");
        String::from_utf8(out.stdout).unwrap()
    }

    fn evaluated(f: &Func) -> String {
        let lim = eval::Limits { max_steps: 10_000_000, max_output: 1 << 20, max_depth: 100, max_seconds: 0.0 };
        let d = eval::run(std::slice::from_ref(f), &f.name, 16, &lim).expect("the runner finishes");
        String::from_utf8(d.output).unwrap()
    }

    /// Every vector operation, in both generators and in the IR runner,
    /// gives what the scalar operations give lane by lane: random lanes and
    /// the edges (the extremes, NaNs with payloads, both zeros, the
    /// infinities), every way the registers can be shared.
    #[test]
    fn every_vector_operation_is_the_scalar_one_lane_by_lane() {
        use BinOp::*;
        let mut r = Rng(0x9E37_79B9_7F4A_7C15);
        let mut vector = Vec::new();
        let mut scalar = Vec::new();
        let kinds: [(Lanes, &[BinOp]); 3] = [
            (Lanes::I64, &[Add, Sub, Mul, Band, Bor, Bxor]),
            (Lanes::I32, &[Add, Sub, Mul, Band, Bor, Bxor]),
            (Lanes::F64, &[FAdd, FSub, FMul, FDiv]),
        ];
        // (d, x, y): all three apart, d the left, d the right, x the right too
        let shapes: [(u8, u8, u8); 4] = [(2, 0, 1), (0, 0, 1), (1, 0, 1), (2, 0, 0)];
        for (l, ops) in kinds {
            let n = l.count() as usize;
            for &b in ops {
                for round in 0..16 {
                    let x: Vec<i64> = (0..n).map(|_| value(l, &mut r)).collect();
                    let mut y: Vec<i64> = (0..n).map(|_| value(l, &mut r)).collect();
                    let (d, p, q) = shapes[round % 4];
                    if p == q {
                        y = x.clone();
                    }
                    put(&mut vector, l, 0, &x);
                    put(&mut vector, l, 2, &y);
                    vector.extend([Op::AddrG(0), Op::VLoad(l, p), Op::AddrG(2), Op::VLoad(l, if p == q { 3 } else { q })]);
                    vector.extend([Op::VBin(l, b, d, p, q), Op::AddrG(4), Op::VStore(l, d)]);
                    print(&mut vector, l, 4);
                    scalar_bin(&mut scalar, l, b, &x, &y);
                }
            }
            if l != Lanes::F64 {
                // shifts by every kind of count
                let bits = 8 * l.width();
                for b in [Shl, Shr] {
                    for k in [0, 1, 2, 7, 31, bits - 1, bits / 2, 13] {
                        let x: Vec<i64> = (0..n).map(|_| value(l, &mut r)).collect();
                        let (d, p) = if k % 2 == 0 { (1, 0) } else { (0, 0) };
                        put(&mut vector, l, 0, &x);
                        vector.extend([Op::AddrG(0), Op::VLoad(l, p), Op::VShift(l, b, d, p, k), Op::AddrG(4), Op::VStore(l, d)]);
                        print(&mut vector, l, 4);
                        scalar_bin(&mut scalar, l, b, &x, &vec![k as i64; n]);
                    }
                }
                // reductions
                for b in [Add, Band, Bor, Bxor] {
                    for _ in 0..6 {
                        let x: Vec<i64> = (0..n).map(|_| value(l, &mut r)).collect();
                        put(&mut vector, l, 0, &x);
                        vector.extend([Op::AddrG(0), Op::VLoad(l, 1), Op::VReduce(l, b, 1), Op::PrintInt, line()]);
                        scalar.push(Op::Push(x[0]));
                        for &xi in &x[1..] {
                            scalar.extend([Op::Push(xi), Op::Bin(b)]);
                        }
                        if l == Lanes::I32 {
                            scalar.push(Op::Sext(32));
                        }
                        scalar.extend([Op::PrintInt, line()]);
                    }
                }
            } else {
                for u in [UnOp::FSqrt, UnOp::FNeg, UnOp::FAbs] {
                    for round in 0..10 {
                        let mut x: Vec<i64> = (0..n).map(|_| value(l, &mut r)).collect();
                        if round == 0 {
                            x = vec![(-4.0f64).to_bits() as i64, 0x7FF0_0000_0000_0001];
                        }
                        let (d, p) = if round % 2 == 0 { (1, 0) } else { (0, 0) };
                        put(&mut vector, l, 0, &x);
                        vector.extend([Op::AddrG(0), Op::VLoad(l, p), Op::VUn(l, u, d, p), Op::AddrG(4), Op::VStore(l, d)]);
                        print(&mut vector, l, 4);
                        for &xi in &x {
                            scalar.extend([Op::Push(xi), Op::Un(u), Op::PrintInt, space()]);
                        }
                        scalar.push(line());
                    }
                }
            }
            // splats and constants
            for _ in 0..4 {
                let x = value(l, &mut r);
                vector.extend([Op::Push(x), Op::VSplat(l, 2), Op::AddrG(4), Op::VStore(l, 2)]);
                print(&mut vector, l, 4);
                let lane = if l == Lanes::I32 { sext(x, 32) } else { x };
                for _ in 0..n {
                    scalar.extend([Op::Push(lane), Op::PrintInt, space()]);
                }
                scalar.push(line());
                let xs: Vec<i64> = (0..n).map(|_| value(l, &mut r)).collect();
                let v = super::super::vectorize::from_lanes(l, &xs);
                vector.extend([Op::VConst(3, v[0] as i64, v[1] as i64), Op::AddrG(4), Op::VStore(l, 3)]);
                print(&mut vector, l, 4);
                for &xi in &xs {
                    scalar.extend([Op::Push(xi), Op::PrintInt, space()]);
                }
                scalar.push(line());
            }
        }
        let (vector, scalar) = (finish(vector), finish(scalar));
        let want = evaluated(&scalar);
        assert_eq!(run(&scalar, false, "scalar"), want, "the scalar program itself");
        let lines = |s: &str| s.lines().map(str::to_string).collect::<Vec<_>>();
        for (name, got) in [("the IR runner", evaluated(&vector)), ("plain", run(&vector, false, "vector")), ("optimized", run(&vector, true, "vector"))] {
            for (i, (g, w)) in lines(&got).iter().zip(lines(&want)).enumerate() {
                assert_eq!(*g, w, "{name}, line {i}");
            }
            assert_eq!(lines(&got).len(), lines(&want).len(), "{name}");
        }
    }
}
