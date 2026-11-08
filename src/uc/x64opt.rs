//! Strike's optimizing code generator. The IR's stack is kept at compile
//! time: a constant, a local or a result in rax stays where it is until an
//! operation needs it, so `n - 1` is one `lea`-free `mov` and `sub`, and a
//! comparison followed by a jump is one `cmp` and one `jcc`. Only what must
//! be on the machine's stack (across a call, a jump or a label) is pushed.
//!
//! HotMotor chooses which locals live in callee-saved registers
//! (codegen.toml): every local offers a register or memory, the judge
//! scores the memory operations a register saves (a use in a loop weighs
//! more), and the beam keeps the best.
//!
//! The plain generator (x64.rs) is the reference: every program compiled
//! here must print what it prints.

use super::ir::{BinOp, Func, Op, UnOp};
use super::x64::{call, Asm, Code, Target};

pub const RULES: &str = include_str!("../../codegen.toml");

#[derive(serde::Deserialize, Clone)]
pub struct Rules {
    pub loop_weight: f64,
    pub save_cost: f64,
    pub beam: usize,
    pub inline: InlineRules,
    pub unroll: UnrollRules,
    pub release: Release,
}

#[derive(serde::Deserialize, Clone)]
pub struct UnrollRules {
    pub levels: u32,
    pub max_ops: usize,
}

#[derive(serde::Deserialize, Clone)]
pub struct Release {
    pub magic_division: bool,
    pub inline: InlineRules,
}

#[derive(serde::Deserialize, Clone)]
pub struct InlineRules {
    pub call_cost: f64,
    pub size_cost: f64,
    pub max_ops: usize,
}

impl Rules {
    /// codegen.toml, or the file STRIKE_CODEGEN names instead.
    pub fn read() -> Result<Rules, String> {
        let (name, text) = match std::env::var_os("STRIKE_CODEGEN") {
            Some(p) => {
                let t = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.to_string_lossy()))?;
                (p.to_string_lossy().to_string(), t)
            }
            None => ("codegen.toml".to_string(), RULES.to_string()),
        };
        toml::from_str(&text).map_err(|e| format!("{name}: {e}"))
    }
    /// What the rules add to a brand: other numbers give other code.
    pub fn key(&self, release: bool) -> String {
        let i = if release { &self.release.inline } else { &self.inline };
        format!(
            "x64opt-4 release={release} magic={} unroll={},{} loop={} save={} beam={} inline={},{},{}",
            release && self.release.magic_division,
            self.unroll.levels,
            self.unroll.max_ops,
            self.loop_weight,
            self.save_cost,
            self.beam,
            i.call_cost,
            i.size_cost,
            i.max_ops
        )
    }
    pub fn inline_for(&self, release: bool) -> &InlineRules {
        if release {
            &self.release.inline
        } else {
            &self.inline
        }
    }
}

const RAX: u8 = 0;
const RCX: u8 = 1;
const R10: u8 = 10;
const R11: u8 = 11;
/// rbx, rsi, rdi, r12 to r15: kept across calls (Windows x64)
const SAVED: [u8; 7] = [3, 6, 7, 12, 13, 14, 15];
const ARGS: [u8; 4] = [1, 2, 8, 9];

#[derive(Clone, Copy, PartialEq, Debug)]
enum V {
    Imm(i64),
    Slot(u32),
    Rax,
    /// the flags of a comparison; the byte is its setcc opcode (0x94 = e)
    Flags(u8),
    /// pushed on the machine's stack
    Stack,
}

#[derive(Clone, Copy)]
enum Loc {
    Reg(u8),
    Mem(i32),
}

#[derive(Clone, Copy)]
enum Opd {
    Imm(i32),
    Reg(u8),
    Mem(i32),
}

fn rex(r: u8, b: u8) -> u8 {
    0x48 | ((r >> 3) & 1) << 2 | ((b >> 3) & 1)
}

/// How much each local is used, a use in a loop counting `loop_weight`
/// times per level (a loop: the ops between a label and a jump back to it).
fn weights(f: &Func, r: &Rules) -> Vec<f64> {
    let at = super::ir::loop_depths(&f.code);
    let mut w = vec![0.0; f.slots as usize];
    for (i, op) in f.code.iter().enumerate() {
        if let Op::Load(s) | Op::Store(s) = op {
            w[*s as usize] += r.loop_weight.powi(at[i].min(6) as i32);
        }
    }
    for p in 0..f.params.min(4) {
        w[p as usize] += 1.0; // a register parameter is already in a register
    }
    w
}

/// HotMotor: the locals in order of weight, each a register or memory; the
/// beam keeps the `beam` best partial choices, scored by the memory
/// operations saved less the cost of saving the registers used.
fn choose(f: &Func, r: &Rules) -> Vec<bool> {
    let w = weights(f, r);
    let mut order: Vec<usize> = (0..w.len()).collect();
    order.sort_by(|&a, &b| w[b].total_cmp(&w[a]).then(a.cmp(&b)));
    let mut beam: Vec<(f64, Vec<bool>)> = vec![(0.0, vec![false; w.len()])];
    for &s in &order {
        let mut next = Vec::new();
        for (score, pick) in &beam {
            next.push((*score, pick.clone()));
            if pick.iter().filter(|&&p| p).count() < SAVED.len() {
                let mut p = pick.clone();
                p[s] = true;
                next.push((score + w[s] - r.save_cost, p));
            }
        }
        next.sort_by(|a, b| b.0.total_cmp(&a.0));
        next.truncate(r.beam.max(1));
        beam = next;
    }
    beam.swap_remove(0).1
}

/// The magic number and shift that divide by d with a multiplication
/// (Hacker's Delight, 10-1), for |d| >= 2.
pub fn magic(d: i64) -> (i64, u32) {
    let two63: u64 = 1 << 63;
    let ad = d.unsigned_abs();
    let t = two63 + ((d as u64) >> 63);
    let anc = t - 1 - t % ad;
    let mut p = 63u32;
    let (mut q1, mut r1) = (two63 / anc, two63 - (two63 / anc) * anc);
    let (mut q2, mut r2) = (two63 / ad, two63 - (two63 / ad) * ad);
    loop {
        p += 1;
        q1 = q1.wrapping_mul(2);
        r1 = r1.wrapping_mul(2);
        if r1 >= anc {
            q1 = q1.wrapping_add(1);
            r1 = r1.wrapping_sub(anc);
        }
        q2 = q2.wrapping_mul(2);
        r2 = r2.wrapping_mul(2);
        if r2 >= ad {
            q2 = q2.wrapping_add(1);
            r2 = r2.wrapping_sub(ad);
        }
        let delta = ad - r2;
        if !(q1 < delta || (q1 == delta && r1 == 0)) {
            break;
        }
    }
    let m = q2.wrapping_add(1) as i64;
    (if d < 0 { m.wrapping_neg() } else { m }, p - 64)
}

#[cfg(test)]
mod tests {
    use super::magic;

    /// The emitted sequence, step by step: q = high(n * m) (+n or -n),
    /// shifted, plus one if negative.
    fn divide(n: i64, d: i64) -> i64 {
        let (m, s) = magic(d);
        let mut hi = ((n as i128 * m as i128) >> 64) as i64;
        if d > 0 && m < 0 {
            hi = hi.wrapping_add(n);
        } else if d < 0 && m > 0 {
            hi = hi.wrapping_sub(n);
        }
        hi >>= s;
        hi + ((hi as u64) >> 63) as i64
    }

    #[test]
    fn a_magic_multiplication_divides() {
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as i64
        };
        let mut ds = vec![2, 3, 5, 7, 10, 1000003, -2, -3, -7, -1000003, i64::MAX, i64::MIN + 1, 1 << 40, -(1 << 40)];
        for _ in 0..200 {
            let d = next() >> (next() as u64 % 63);
            if d.unsigned_abs() >= 2 && d != i64::MIN {
                ds.push(d);
            }
        }
        for &d in &ds {
            let mut ns = vec![0, 1, -1, i64::MAX, i64::MIN, d, -d, d - 1, d + 1];
            ns.extend((0..200).map(|_| next() >> (next() as u64 % 64)));
            for n in ns {
                assert_eq!(divide(n, d), n.wrapping_div(d), "{n} / {d}");
            }
        }
    }
}

struct G {
    a: Asm,
    v: Vec<V>,
    loc: Vec<Loc>,
    saved: Vec<(u8, i32)>,
    /// division by a constant as a multiplication (codegen.toml)
    magic: bool,
}

impl G {
    fn rr(&mut self, op: &[u8], reg: u8, rm: u8) {
        self.a.e(&[rex(reg, rm)]);
        self.a.e(op);
        self.a.e(&[0xC0 | (reg & 7) << 3 | (rm & 7)]);
    }
    /// op with [rbp + d]
    fn rm(&mut self, op: &[u8], reg: u8, d: i32) {
        self.a.e(&[rex(reg, 0)]);
        self.a.e(op);
        self.a.e(&[0x80 | (reg & 7) << 3 | 5]);
        self.a.d32(d);
    }
    fn mov_imm(&mut self, r: u8, v: i64) {
        if let Ok(i) = i32::try_from(v) {
            self.a.e(&[rex(0, r), 0xC7, 0xC0 | (r & 7)]);
            self.a.d32(i);
        } else {
            self.a.e(&[rex(0, r), 0xB8 | (r & 7)]);
            self.a.e(&v.to_le_bytes());
        }
    }
    fn mov_rr(&mut self, dst: u8, src: u8) {
        if dst != src {
            self.rr(&[0x89], src, dst);
        }
    }
    fn push_r(&mut self, r: u8) {
        if r >= 8 {
            self.a.e(&[0x41]);
        }
        self.a.e(&[0x50 | (r & 7)]);
    }
    fn pop_r(&mut self, r: u8) {
        if r >= 8 {
            self.a.e(&[0x41]);
        }
        self.a.e(&[0x58 | (r & 7)]);
    }
    fn setcc_rax(&mut self, cc: u8) {
        self.setcc_into(RAX, cc);
    }
    /// setcc r8; movzx r32, r8 (touches no other register)
    fn setcc_into(&mut self, r: u8, cc: u8) {
        let b = (r >> 3) & 1;
        self.a.e(&[0x40 | b, 0x0F, cc, 0xC0 | (r & 7)]);
        self.a.e(&[0x40 | b << 2 | b, 0x0F, 0xB6, 0xC0 | (r & 7) << 3 | (r & 7)]);
    }

    /// The value into register r (rax only once nothing else lives there).
    fn to_reg(&mut self, x: V, r: u8) {
        match x {
            V::Imm(c) => self.mov_imm(r, c),
            V::Slot(s) => match self.loc[s as usize] {
                Loc::Reg(y) => self.mov_rr(r, y),
                Loc::Mem(d) => self.rm(&[0x8B], r, d),
            },
            V::Rax => self.mov_rr(r, RAX),
            V::Stack => self.pop_r(r),
            V::Flags(cc) => self.setcc_into(r, cc),
        }
    }

    fn push_physical(&mut self, x: V) {
        match x {
            V::Imm(c) => {
                if let Ok(i) = i32::try_from(c) {
                    self.a.e(&[0x68]);
                    self.a.d32(i);
                } else {
                    self.mov_imm(R11, c);
                    self.push_r(R11);
                }
            }
            V::Slot(s) => match self.loc[s as usize] {
                Loc::Reg(y) => self.push_r(y),
                Loc::Mem(d) => {
                    self.a.e(&[0xFF, 0xB5]);
                    self.a.d32(d);
                }
            },
            V::Rax => self.push_r(RAX),
            V::Flags(cc) => {
                self.setcc_into(R11, cc);
                self.push_r(R11);
            }
            V::Stack => {}
        }
    }

    /// Everything up to index k onto the machine's stack, in order.
    fn flush_to(&mut self, k: usize) {
        let first = self.v.iter().position(|x| *x != V::Stack).unwrap_or(self.v.len());
        for i in first..=k.min(self.v.len().wrapping_sub(1)) {
            if i >= self.v.len() {
                break;
            }
            let x = self.v[i];
            self.push_physical(x);
            self.v[i] = V::Stack;
        }
    }
    fn flush(&mut self) {
        if !self.v.is_empty() {
            self.flush_to(self.v.len() - 1);
        }
    }
    fn need_rax(&mut self) {
        if let Some(i) = self.v.iter().rposition(|x| *x == V::Rax) {
            self.flush_to(i);
        }
    }
    fn push(&mut self, x: V) {
        if let Some(&V::Flags(cc)) = self.v.last() {
            self.v.pop();
            self.need_rax();
            self.setcc_rax(cc);
            self.v.push(V::Rax);
        }
        self.v.push(x);
    }
    fn pop(&mut self) -> V {
        self.v.pop().expect("the IR's stack is balanced")
    }
    fn depth(&self) -> u32 {
        self.v.len() as u32
    }

    /// The right operand of an operation with rax.
    fn operand(&mut self, x: V) -> Opd {
        match x {
            V::Imm(c) => match i32::try_from(c) {
                Ok(i) => Opd::Imm(i),
                Err(_) => {
                    self.mov_imm(RCX, c);
                    Opd::Reg(RCX)
                }
            },
            V::Slot(s) => match self.loc[s as usize] {
                Loc::Reg(y) => Opd::Reg(y),
                Loc::Mem(d) => Opd::Mem(d),
            },
            other => {
                self.to_reg(other, RCX);
                Opd::Reg(RCX)
            }
        }
    }
    fn into_rcx(&mut self, o: Opd) {
        match o {
            Opd::Imm(i) => self.mov_imm(RCX, i as i64),
            Opd::Reg(r) => self.mov_rr(RCX, r),
            Opd::Mem(d) => self.rm(&[0x8B], RCX, d),
        }
    }
    /// add 0, or 1, and 4, sub 5, xor 6, cmp 7
    fn alu(&mut self, ext: u8, dst: u8, o: Opd) {
        match o {
            Opd::Imm(i) => {
                if let Ok(b) = i8::try_from(i) {
                    self.a.e(&[rex(0, dst), 0x83, 0xC0 | ext << 3 | (dst & 7), b as u8]);
                } else {
                    self.a.e(&[rex(0, dst), 0x81, 0xC0 | ext << 3 | (dst & 7)]);
                    self.a.d32(i);
                }
            }
            Opd::Reg(r) => self.rr(&[ext * 8 + 1], r, dst),
            Opd::Mem(d) => self.rm(&[ext * 8 + 3], dst, d),
        }
    }

    fn bin(&mut self, b: BinOp, done: &mut dyn FnMut(&mut Asm) -> u32) {
        let right = self.pop();
        let left = self.pop();
        self.need_rax();
        let o = self.operand(right);
        if b.is_comparison() {
            let dst = match left {
                V::Slot(s) => match self.loc[s as usize] {
                    Loc::Reg(y) => y,
                    Loc::Mem(_) => {
                        self.to_reg(left, RAX);
                        RAX
                    }
                },
                _ => {
                    self.to_reg(left, RAX);
                    RAX
                }
            };
            self.alu(7, dst, o);
            let cc = match b {
                BinOp::Eq => 0x94,
                BinOp::Ne => 0x95,
                BinOp::Lt => 0x9C,
                BinOp::Le => 0x9E,
                BinOp::Gt => 0x9F,
                _ => 0x9D,
            };
            self.v.push(V::Flags(cc));
            return;
        }
        self.to_reg(left, RAX);
        let zero = matches!(o, Opd::Imm(0));
        match b {
            BinOp::Add | BinOp::Sub | BinOp::Bor | BinOp::Bxor if zero => {}
            BinOp::Add => self.alu(0, RAX, o),
            BinOp::Sub => self.alu(5, RAX, o),
            BinOp::Band => self.alu(4, RAX, o),
            BinOp::Bor => self.alu(1, RAX, o),
            BinOp::Bxor => self.alu(6, RAX, o),
            BinOp::Mul => match o {
                Opd::Imm(i) => {
                    self.rr(&[0x69], RAX, RAX);
                    self.a.d32(i);
                }
                Opd::Reg(r) => self.rr(&[0x0F, 0xAF], RAX, r),
                Opd::Mem(d) => self.rm(&[0x0F, 0xAF], RAX, d),
            },
            BinOp::Shl | BinOp::Shr => {
                let ext = if b == BinOp::Shl { 4 } else { 7 };
                match o {
                    Opd::Imm(i) => self.a.e(&[0x48, 0xC1, 0xC0 | ext << 3, (i & 63) as u8]),
                    o => {
                        self.into_rcx(o);
                        self.a.e(&[0x48, 0xD3, 0xC0 | ext << 3]);
                    }
                }
            }
            BinOp::DivTrunc | BinOp::ModTrunc | BinOp::DivFloor | BinOp::ModFloor
                if self.magic && matches!(o, Opd::Imm(d) if d.unsigned_abs() >= 2) =>
            {
                let Opd::Imm(d) = o else { unreachable!() };
                let (m, s) = magic(d as i64);
                self.a.e(&[0x49, 0x89, 0xC0]); // mov r8, rax (n)
                self.mov_imm(RCX, m);
                self.a.e(&[0x48, 0xF7, 0xE9]); // imul rcx: rdx = high(n * m)
                if d > 0 && m < 0 {
                    self.a.e(&[0x4C, 0x01, 0xC2]); // add rdx, r8
                } else if d < 0 && m > 0 {
                    self.a.e(&[0x4C, 0x29, 0xC2]); // sub rdx, r8
                }
                if s > 0 {
                    self.a.e(&[0x48, 0xC1, 0xFA, s as u8]); // sar rdx, s
                }
                self.a.e(&[0x48, 0x89, 0xD0, 0x48, 0xC1, 0xE8, 0x3F, 0x48, 0x01, 0xD0]); // rax = q (+1 if negative)
                if b != BinOp::DivTrunc {
                    // rdx = n - q * d, rcx = d: as idiv leaves them
                    self.a.e(&[0x48, 0x89, 0xC2, 0x48, 0x69, 0xD2]); // mov rdx, rax; imul rdx, rdx, d
                    self.a.d32(d);
                    self.a.e(&[0x4C, 0x89, 0xC1, 0x48, 0x29, 0xD1, 0x48, 0x89, 0xCA]); // rdx = n - rdx
                    self.mov_imm(RCX, d as i64);
                }
                if matches!(b, BinOp::DivFloor | BinOp::ModFloor) {
                    let l = done(&mut self.a);
                    self.a.e(&[0x48, 0x85, 0xD2]);
                    self.a.jcc(4, l);
                    self.a.e(&[0x49, 0x89, 0xD0, 0x49, 0x31, 0xC8]);
                    self.a.jcc(9, l);
                    self.a.e(&[0x48, 0xFF, 0xC8, 0x48, 0x01, 0xCA]);
                    self.a.bind(l);
                }
                if matches!(b, BinOp::ModTrunc | BinOp::ModFloor) {
                    self.mov_rr(RAX, 2);
                }
            }
            BinOp::DivTrunc | BinOp::ModTrunc | BinOp::DivFloor | BinOp::ModFloor => {
                self.into_rcx(o);
                self.a.e(&[0x48, 0x99, 0x48, 0xF7, 0xF9]); // cqo; idiv rcx
                if matches!(b, BinOp::DivFloor | BinOp::ModFloor) {
                    let l = done(&mut self.a);
                    self.a.e(&[0x48, 0x85, 0xD2]);
                    self.a.jcc(4, l);
                    self.a.e(&[0x49, 0x89, 0xD0, 0x49, 0x31, 0xC8]);
                    self.a.jcc(9, l);
                    self.a.e(&[0x48, 0xFF, 0xC8, 0x48, 0x01, 0xCA]);
                    self.a.bind(l);
                }
                if matches!(b, BinOp::ModTrunc | BinOp::ModFloor) {
                    self.mov_rr(RAX, 2);
                }
            }
            _ => unreachable!("logic operators are lowered to jumps"),
        }
        self.v.push(V::Rax);
    }

    fn store(&mut self, s: u32) {
        let x = self.pop();
        if let Some(i) = self.v.iter().rposition(|y| *y == V::Slot(s)) {
            self.flush_to(i);
        }
        match self.loc[s as usize] {
            Loc::Reg(r) => self.to_reg(x, r),
            Loc::Mem(d) => match x {
                V::Imm(c) if i32::try_from(c).is_ok() => {
                    self.rm(&[0xC7], 0, d);
                    self.a.d32(c as i32);
                }
                V::Stack => {
                    self.a.e(&[0x8F, 0x85]); // pop qword [rbp + d]
                    self.a.d32(d);
                }
                V::Slot(t) if t == s => {}
                V::Slot(t) => match self.loc[t as usize] {
                    Loc::Reg(y) => self.rm(&[0x89], y, d),
                    Loc::Mem(_) => {
                        self.to_reg(x, R11);
                        self.rm(&[0x89], R11, d);
                    }
                },
                V::Rax => self.rm(&[0x89], RAX, d),
                other => {
                    self.to_reg(other, R11);
                    self.rm(&[0x89], R11, d);
                }
            },
        }
    }

    /// A call whose arguments go straight to rcx, rdx, r8, r9 and the
    /// stack slots above the shadow space, from wherever they are: nothing
    /// is pushed but what must outlive the call (a result in rax below the
    /// arguments). Arguments already on the machine's stack take the
    /// plain way.
    fn call_direct(&mut self, sym: &str, n: usize) {
        let at = self.v.len() - n;
        if self.v[at..].contains(&V::Stack) {
            self.flush();
            let mut d = self.depth();
            call(&mut self.a, sym, n as u32, &mut d, false);
            self.v.truncate(d as usize);
            self.v.push(V::Rax);
            return;
        }
        let args: Vec<V> = self.v.split_off(at);
        if let Some(i) = self.v.iter().rposition(|x| *x == V::Rax) {
            self.flush_to(i);
        }
        // a comparison's flags become a number before anything changes them
        let flags_in_r10 = if let Some(&V::Flags(cc)) = args.last() {
            self.setcc_into(R10, cc);
            true
        } else {
            false
        };
        let physical = self.v.iter().filter(|x| **x == V::Stack).count() as u32;
        let k = n.saturating_sub(4) as u32;
        // with nothing pushed, the frame's own call area is at rsp, aligned
        let mut s = if physical == 0 { 0 } else { 32 + 8 * k };
        if s > 0 && (physical + s / 8) % 2 == 1 {
            s += 8;
        }
        if s > 0 {
            self.a.sub_rsp(s as i32);
        }
        // no source is an argument register, so the order does not matter
        for (i, x) in args.iter().enumerate() {
            let src = if flags_in_r10 && i == n - 1 { None } else { Some(*x) };
            if i < 4 {
                match src {
                    None => self.mov_rr(ARGS[i], R10),
                    Some(x) => self.to_reg(x, ARGS[i]),
                }
            } else {
                let disp = 32 + 8 * (i as i32 - 4);
                let reg = match src {
                    None => R10,
                    Some(V::Rax) => RAX,
                    Some(V::Imm(c)) if i32::try_from(c).is_ok() => {
                        self.a.e(&[0x48, 0xC7, 0x84, 0x24]); // mov qword [rsp + d], imm32
                        self.a.d32(disp);
                        self.a.d32(c as i32);
                        continue;
                    }
                    Some(V::Slot(t)) if matches!(self.loc[t as usize], Loc::Reg(_)) => {
                        let Loc::Reg(y) = self.loc[t as usize] else { unreachable!() };
                        y
                    }
                    Some(x) => {
                        self.to_reg(x, R11);
                        R11
                    }
                };
                self.a.e(&[rex(reg, 0), 0x89, 0x84 | (reg & 7) << 3, 0x24]); // mov [rsp + d], reg
                self.a.d32(disp);
            }
        }
        self.a.call(sym);
        if s > 0 {
            self.a.add_rsp(s as i32);
        }
        self.v.push(V::Rax);
    }

    /// A call of the runtime whose arguments are already in rcx (and rdx).
    fn call_runtime(&mut self, sym: &str) {
        let mut d = self.depth();
        if d == 0 {
            self.a.call(sym);
        } else {
            call(&mut self.a, sym, 0, &mut d, false);
        }
    }

    /// An operation whose result is stored at once into a local that lives
    /// in a register: worked out in that register (`x = x - 2` is one
    /// `sub`), leaving rax and what it holds alone. False when the shape
    /// does not allow it; nothing is emitted then.
    fn bin_into(&mut self, b: BinOp, s: u32) -> bool {
        let ext = match b {
            BinOp::Add => 0,
            BinOp::Bor => 1,
            BinOp::Band => 4,
            BinOp::Sub => 5,
            BinOp::Bxor => 6,
            BinOp::Mul => 8,
            _ => return false,
        };
        let Loc::Reg(d) = self.loc[s as usize] else { return false };
        let n = self.v.len();
        if n < 2 {
            return false;
        }
        let (mut left, mut right) = (self.v[n - 2], self.v[n - 1]);
        // `acc = x + acc` is `acc += x`: these operations do not mind the order
        if right == V::Slot(s) && left != V::Slot(s) && ext != 5 {
            std::mem::swap(&mut left, &mut right);
        }
        let fits = |x: V| match x {
            V::Imm(c) => i32::try_from(c).is_ok(),
            V::Slot(_) | V::Rax => true,
            _ => false,
        };
        if !fits(left) || !fits(right) || (right == V::Slot(s) && left != V::Slot(s)) {
            return false;
        }
        self.v.truncate(n - 2);
        if let Some(i) = self.v.iter().rposition(|y| *y == V::Slot(s)) {
            self.flush_to(i);
        }
        if left != V::Slot(s) {
            self.to_reg(left, d);
        }
        let o = match right {
            V::Imm(c) => Opd::Imm(c as i32),
            V::Rax => Opd::Reg(RAX),
            V::Slot(t) => match self.loc[t as usize] {
                Loc::Reg(y) => Opd::Reg(y),
                Loc::Mem(m) => Opd::Mem(m),
            },
            _ => unreachable!(),
        };
        match (ext, o) {
            (8, Opd::Imm(i)) => {
                self.rr(&[0x69], d, d);
                self.a.d32(i);
            }
            (8, Opd::Reg(r)) => self.rr(&[0x0F, 0xAF], d, r),
            (8, Opd::Mem(m)) => self.rm(&[0x0F, 0xAF], d, m),
            (0 | 1 | 5 | 6, Opd::Imm(0)) => {}
            (e, o) => self.alu(e, d, o),
        }
        true
    }

    fn un(&mut self, u: UnOp) {
        let x = self.pop();
        match (u, x) {
            (UnOp::Plus, x) => self.v.push(x),
            (UnOp::Not, V::Flags(cc)) => self.v.push(V::Flags(cc ^ 1)),
            (UnOp::Not, x) => {
                self.need_rax();
                self.to_reg(x, RAX);
                self.a.e(&[0x48, 0x85, 0xC0]);
                self.v.push(V::Flags(0x94));
            }
            (u, x) => {
                self.need_rax();
                self.to_reg(x, RAX);
                self.a.e(if u == UnOp::Neg { &[0x48, 0xF7, 0xD8] } else { &[0x48, 0xF7, 0xD0] });
                self.v.push(V::Rax);
            }
        }
    }

    /// The fast path's operations (fast_path checked them).
    fn fast_ops(&mut self, ops: &[Op]) {
        for op in ops {
            match op {
                Op::Push(c) => self.push(V::Imm(*c)),
                Op::Load(s) => self.push(V::Slot(*s)),
                Op::Bin(b) => self.bin(*b, &mut |a: &mut Asm| a.label()),
                Op::Un(u) => self.un(*u),
                _ => unreachable!("fast_path takes only plain operations"),
            }
        }
    }

    /// The value on top into a register and tested: a jump to `to` when it
    /// is zero (`on_zero`) or when it is not.
    fn branch(&mut self, on_zero: bool, to: u32) {
        match self.pop() {
            V::Flags(cc) => self.a.jcc(if on_zero { (cc ^ 1) & 0x0F } else { cc & 0x0F }, to),
            x => {
                let r = match x {
                    V::Slot(s) => match self.loc[s as usize] {
                        Loc::Reg(y) => y,
                        Loc::Mem(_) => {
                            self.need_rax();
                            self.to_reg(x, RAX);
                            RAX
                        }
                    },
                    x => {
                        self.need_rax();
                        self.to_reg(x, RAX);
                        RAX
                    }
                };
                self.rr(&[0x85], r, r);
                self.a.jcc(if on_zero { 4 } else { 5 }, to);
            }
        }
    }

    fn epilogue(&mut self) {
        for (r, d) in self.saved.clone() {
            self.rm(&[0x8B], r, d);
        }
        self.a.epilogue();
    }
}

/// A function that begins by testing its parameters and returning at once
/// (fib's `if (n < 2) return n`): the test and that return, worked out from
/// the argument registers before any frame is made or register saved. The
/// slots other than the parameters hold, at the start, the constants the
/// function's first operations store in them (zero otherwise), so their
/// loads become those constants. Only what cannot touch an argument
/// register is taken: loads of the first four parameters, constants, and
/// operations whose right side is a constant or a parameter (no division,
/// no shift by a variable, no call).
struct Fast {
    /// the test, ending where the branch to the rest of the function is
    test: Vec<Op>,
    /// true: the rest runs when the test is zero (the IR's Jz)
    rest_on_zero: bool,
    /// the return's value
    value: Vec<Op>,
}

fn fast_path(f: &Func) -> Option<Fast> {
    let mut known = vec![0i64; f.slots as usize];
    let mut i = 0;
    let code = &f.code;
    // stores of constants and labels the start passes through
    loop {
        match (code.get(i), code.get(i + 1)) {
            (Some(Op::Push(c)), Some(Op::Store(s))) if *s >= f.params => {
                known[*s as usize] = *c;
                i += 2;
            }
            (Some(Op::Label(_)), _) => i += 1,
            _ => break,
        }
    }
    // a run of plain operations, checked on a stack of what each value is
    #[derive(Clone, Copy, PartialEq)]
    enum K {
        Imm,
        Param,
        Done,
    }
    let plain = |from: usize, ops: &mut Vec<Op>| -> Option<(usize, usize)> {
        let mut st: Vec<K> = Vec::new();
        let mut j = from;
        loop {
            match code.get(j)? {
                Op::Push(c) => {
                    i32::try_from(*c).ok()?;
                    st.push(K::Imm);
                    ops.push(Op::Push(*c));
                }
                Op::Load(s) if *s < f.params => {
                    if *s >= 4 {
                        return None;
                    }
                    st.push(K::Param);
                    ops.push(Op::Load(*s));
                }
                Op::Load(s) => {
                    let c = known[*s as usize];
                    i32::try_from(c).ok()?;
                    st.push(K::Imm);
                    ops.push(Op::Push(c));
                }
                Op::Un(u) => {
                    st.pop()?;
                    st.push(K::Done);
                    ops.push(Op::Un(*u));
                }
                Op::Bin(b) => {
                    use BinOp::*;
                    let r = st.pop()?;
                    st.pop()?;
                    let ok = match b {
                        Add | Sub | Mul | Band | Bor | Bxor | Eq | Ne | Lt | Le | Gt | Ge => r != K::Done,
                        Shl | Shr => r == K::Imm,
                        _ => false,
                    };
                    if !ok {
                        return None;
                    }
                    st.push(K::Done);
                    ops.push(Op::Bin(*b));
                }
                _ => return Some((j, st.len())),
            }
            j += 1;
        }
    };
    let mut test = Vec::new();
    let (j, depth) = plain(i, &mut test)?;
    let rest_on_zero = match code.get(j)? {
        Op::Jz(_) => true,
        Op::Jnz(_) => false,
        _ => return None,
    };
    if depth != 1 || test.is_empty() {
        return None;
    }
    let mut value = Vec::new();
    let (k, depth) = plain(j + 1, &mut value)?;
    if depth != 1 || code.get(k) != Some(&Op::Ret) {
        return None;
    }
    if let [Op::Push(_)] = test.as_slice() {
        return None; // a constant test: one way or the other always
    }
    Some(Fast { test, rest_on_zero, value })
}

pub fn compile(f: &Func, rules: &Rules, release: bool) -> Code {
    let in_reg = choose(f, rules);
    let mut loc = Vec::new();
    let mut saved = Vec::new();
    let mut mem = 0i32;
    let mut free = SAVED.iter();
    for s in 0..f.slots as usize {
        if in_reg[s] {
            let r = *free.next().expect("the beam keeps to the registers there are");
            loc.push(Loc::Reg(r));
            saved.push(r);
        } else {
            mem += 1;
            loc.push(Loc::Mem(-8 * mem));
        }
    }
    let saved: Vec<(u8, i32)> = saved
        .into_iter()
        .map(|r| {
            mem += 1;
            (r, -8 * mem)
        })
        .collect();
    // the call area below the locals: shadow space and the stack arguments
    // of the widest call, so a call with nothing pushed needs no rsp change
    let widest = f
        .code
        .iter()
        .map(|o| match o {
            Op::Call(_, n) => 32 + 8 * n.saturating_sub(4) as i32,
            Op::PrintInt | Op::PrintStr(_) => 32,
            _ => 0,
        })
        .max()
        .unwrap_or(0);
    let frame = (mem * 8 + widest + 15) / 16 * 16;
    let mut g = G { a: Asm::new(), v: Vec::new(), loc, saved, magic: release && rules.release.magic_division };
    if let Some(fp) = fast_path(f) {
        // before the frame: the parameters are still in rcx, rdx, r8, r9
        let at_entry = (0..f.slots).map(|s| if s < f.params.min(4) { Loc::Reg(ARGS[s as usize]) } else { Loc::Mem(0) }).collect();
        let real = std::mem::replace(&mut g.loc, at_entry);
        let rest = g.a.label();
        g.fast_ops(&fp.test);
        g.branch(fp.rest_on_zero, rest);
        g.v.clear();
        g.fast_ops(&fp.value);
        let x = g.pop();
        g.to_reg(x, RAX);
        g.a.e(&[0xC3]); // ret: no frame was made
        g.a.bind(rest);
        g.v.clear();
        g.loc = real;
    }
    g.a.prologue(frame);
    for (r, d) in g.saved.clone() {
        g.rm(&[0x89], r, d);
    }
    for i in 0..f.params {
        let l = g.loc[i as usize];
        if i < 4 {
            match l {
                Loc::Reg(r) => g.mov_rr(r, ARGS[i as usize]),
                Loc::Mem(d) => g.rm(&[0x89], ARGS[i as usize], d),
            }
        } else {
            let from = 48 + 8 * (i as i32 - 4);
            match l {
                Loc::Reg(r) => g.rm(&[0x8B], r, from),
                Loc::Mem(d) => {
                    g.rm(&[0x8B], RAX, from);
                    g.rm(&[0x89], RAX, d);
                }
            }
        }
    }
    for s in f.params..f.slots {
        match g.loc[s as usize] {
            Loc::Reg(r) => g.mov_imm(r, 0),
            Loc::Mem(d) => {
                g.rm(&[0xC7], 0, d);
                g.a.d32(0);
            }
        }
    }

    let labels = f.code.iter().filter_map(|o| if let Op::Label(l) = o { Some(*l + 1) } else { None }).max().unwrap_or(0);
    let labels_of: Vec<u32> = (0..labels).map(|_| g.a.label()).collect();
    let mut depth_at: Vec<Option<u32>> = vec![None; labels as usize];
    let mut new_label = |a: &mut Asm| a.label();
    let mut stored_already = false;
    for (at, op) in f.code.iter().enumerate() {
        if std::mem::take(&mut stored_already) {
            continue; // the Store bin_into did
        }
        if let (Op::Bin(b), Some(Op::Store(s))) = (op, f.code.get(at + 1)) {
            if g.bin_into(*b, *s) {
                stored_already = true;
                continue;
            }
        }
        match op {
            Op::Push(c) => g.push(V::Imm(*c)),
            Op::Load(s) => g.push(V::Slot(*s)),
            Op::Store(s) => g.store(*s),
            Op::Bin(b) => g.bin(*b, &mut new_label),
            Op::Un(u) => g.un(*u),
            Op::Dup => match g.v.last().copied() {
                Some(x @ (V::Imm(_) | V::Slot(_))) => g.push(x),
                _ => {
                    g.flush();
                    g.a.e(&[0xFF, 0x34, 0x24]);
                    g.v.push(V::Stack);
                }
            },
            Op::Pop => {
                if g.pop() == V::Stack {
                    g.a.e(&[0x48, 0x83, 0xC4, 0x08]);
                }
            }
            Op::Call(sym, n) => g.call_direct(sym, *n as usize),
            Op::PrintInt => {
                let x = g.pop();
                g.need_rax();
                g.to_reg(x, RCX);
                g.flush();
                g.call_runtime("rt.print_int");
            }
            Op::PrintStr(s) => {
                g.flush();
                g.a.e(&[0x48, 0x8D, 0x0D]);
                g.a.reloc(Target::Data(s.clone()));
                g.a.e(&[0x48, 0xC7, 0xC2]);
                g.a.d32(s.len() as i32);
                g.call_runtime("rt.print_str");
            }
            Op::Jmp(l) => {
                g.flush();
                depth_at[*l as usize].get_or_insert(g.depth());
                g.a.jmp(labels_of[*l as usize]);
            }
            Op::Jz(l) | Op::Jnz(l) => {
                let jz = matches!(op, Op::Jz(_));
                let x = g.pop();
                let target = labels_of[*l as usize];
                match x {
                    V::Flags(cc) => {
                        g.flush();
                        let cc = if jz { cc ^ 1 } else { cc };
                        depth_at[*l as usize].get_or_insert(g.depth());
                        g.a.jcc(cc & 0x0F, target);
                    }
                    V::Imm(c) => {
                        g.flush();
                        depth_at[*l as usize].get_or_insert(g.depth());
                        if (c == 0) == jz {
                            g.a.jmp(target);
                        }
                    }
                    x => {
                        let r = match x {
                            V::Slot(s) => match g.loc[s as usize] {
                                Loc::Reg(y) => y,
                                Loc::Mem(_) => {
                                    g.need_rax();
                                    g.to_reg(x, RAX);
                                    RAX
                                }
                            },
                            _ => {
                                g.need_rax();
                                g.to_reg(x, RAX);
                                RAX
                            }
                        };
                        g.rr(&[0x85], r, r); // test r, r
                        g.flush();
                        depth_at[*l as usize].get_or_insert(g.depth());
                        g.a.jcc(if jz { 4 } else { 5 }, target);
                    }
                }
            }
            Op::Label(l) => {
                g.flush();
                match depth_at[*l as usize] {
                    Some(d) => g.v = vec![V::Stack; d as usize],
                    None => depth_at[*l as usize] = Some(g.depth()),
                }
                g.a.bind(labels_of[*l as usize]);
            }
            Op::Ret => {
                let x = g.pop();
                g.to_reg(x, RAX);
                g.epilogue();
                g.v = vec![V::Stack; g.v.len()];
            }
        }
    }
    g.a.finish()
}
