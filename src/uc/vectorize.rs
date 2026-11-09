//! --release: loop vectorization (SSE2), before loops are rotated.
//!
//! An innermost counted loop (`c < bound`, `c + 1` at its bottom) whose
//! body only stores expressions gets a vector loop before it: two 64-bit
//! lanes or four 32-bit ones an instruction, four (or two, or one) vectors
//! each time round, and the scalar loop finishes what is left. Lane by
//! lane every operation is the scalar one, bit for bit: integers wrap at
//! the lane's width (a C int's cut is the 32-bit lane), floats round as
//! addsd does and keep their operands' order. Integer reductions (+ - ^ |
//! &) are reordered, which wrapping arithmetic allows; float reductions
//! never are. A store and another access are at the same address every
//! time round, or apart as alias.rs finds them, or checked apart before
//! the vector loop runs (else the scalar loop does it all). STRIKE_DUMP_VEC
//! prints each function before and after.

use super::ir::{sext, BinOp, Func, Lanes, Op, UnOp};

/// A vector's 16 bytes, as two 64-bit halves (the low one first).
pub type V128 = [u64; 2];

/// The lanes of a vector, each as the IR holds a number (an I32 lane
/// sign-extended).
pub fn lanes(l: Lanes, v: V128) -> Vec<i64> {
    match l {
        Lanes::I32 => (0..4).map(|i| sext((v[i / 2] >> (32 * (i % 2))) as i64, 32)).collect(),
        Lanes::I64 | Lanes::F64 => vec![v[0] as i64, v[1] as i64],
    }
}

/// The vector of these lanes (an I32 lane: its low 32 bits).
pub fn from_lanes(l: Lanes, xs: &[i64]) -> V128 {
    match l {
        Lanes::I32 => {
            let w = |i: usize| xs[i] as u64 & 0xFFFF_FFFF;
            [w(0) | w(1) << 32, w(2) | w(3) << 32]
        }
        Lanes::I64 | Lanes::F64 => [xs[0] as u64, xs[1] as u64],
    }
}

fn lane_bin(l: Lanes, op: BinOp, a: i64, b: i64) -> Option<i64> {
    use BinOp::*;
    let r = match (l, op) {
        (Lanes::F64, FAdd | FSub | FMul | FDiv) => op.float_eval(a, b)?,
        (Lanes::I32 | Lanes::I64, Add) => a.wrapping_add(b),
        (Lanes::I32 | Lanes::I64, Sub) => a.wrapping_sub(b),
        (Lanes::I32 | Lanes::I64, Mul) => a.wrapping_mul(b),
        (_, Band) => a & b,
        (_, Bor) => a | b,
        (_, Bxor) => a ^ b,
        _ => return None,
    };
    Some(if l == Lanes::I32 { sext(r, 32) } else { r })
}

/// VBin's work: None for an operation it does not have.
pub fn bin(l: Lanes, op: BinOp, a: V128, b: V128) -> Option<V128> {
    let (x, y) = (lanes(l, a), lanes(l, b));
    let r: Option<Vec<i64>> = x.iter().zip(&y).map(|(&p, &q)| lane_bin(l, op, p, q)).collect();
    Some(from_lanes(l, &r?))
}

/// VShift's work.
pub fn shift(l: Lanes, op: BinOp, a: V128, k: u8) -> Option<V128> {
    let bits = 8 * l.width() as u32;
    if l == Lanes::F64 || k as u32 >= bits {
        return None;
    }
    let r: Option<Vec<i64>> = lanes(l, a)
        .iter()
        .map(|&x| match op {
            BinOp::Shl => Some(sext(x.wrapping_shl(k as u32), bits as u8)),
            BinOp::Shr => Some(x >> k),
            _ => None,
        })
        .collect();
    Some(from_lanes(l, &r?))
}

/// VUn's work.
pub fn un(l: Lanes, u: UnOp, a: V128) -> Option<V128> {
    if l != Lanes::F64 || !matches!(u, UnOp::FSqrt | UnOp::FNeg | UnOp::FAbs) {
        return None;
    }
    let r: Option<Vec<i64>> = lanes(l, a).iter().map(|&x| u.float_eval(x)).collect();
    Some(from_lanes(l, &r?))
}

/// VSplat's work.
pub fn splat(l: Lanes, x: i64) -> V128 {
    from_lanes(l, &[x; 4])
}

/// VReduce's work: the lanes combined, from the first.
pub fn reduce(l: Lanes, op: BinOp, a: V128) -> Option<i64> {
    if l == Lanes::F64 || !matches!(op, BinOp::Add | BinOp::Band | BinOp::Bor | BinOp::Bxor) {
        return None;
    }
    let xs = lanes(l, a);
    let mut r = xs[0];
    for &x in &xs[1..] {
        r = lane_bin(l, op, r, x)?;
    }
    Some(r)
}

// ---------------------------------------------------------------------------
// The pass

/// The most operations a loop's body may have to be vectorized.
const MAX_BODY: usize = 200;
/// The most pairs of accesses a vectorized loop checks apart before it runs.
const MAX_CHECKS: usize = 6;
/// The most vector locals a vectorized loop takes (each an xmm register):
/// the first KEPT may hold a value for the whole loop (callee-saved xmm15
/// down to xmm6), the rest only a value of the body (xmm5 down to xmm2,
/// which no scalar float code uses while the body runs).
const MAX_VECTORS: u8 = 14;
const KEPT: u8 = 10;

/// A value of the loop's body, as a tree (shared where a local made in
/// the body is read again).
#[derive(Clone, Debug, PartialEq)]
enum N {
    Push(i64),
    Load(u32),
    AddrG(u32),
    Frame(u32),
    Bin(BinOp, usize, usize),
    Un(UnOp, usize),
    Sext(u8, usize),
    LoadM(u8, usize),
}

/// A statement of the body.
#[derive(Clone, Debug)]
enum S {
    Store(u32, usize),
    StoreM(u8, usize, usize),
    OpM(BinOp, u8, usize, usize),
}

/// The locals that only ever hold numbers that fit 32 bits (sign-extended
/// from them): every store into one is of a value cut to 32 bits or less,
/// a constant that fits, a comparison, a narrow element, or another such
/// local; also a counter's `c + 1` where `ok_inc` says the loop's bound
/// keeps it within. Parameters are not known to be.
fn narrow32(f: &Func, ok_inc: &dyn Fn(usize) -> bool) -> Vec<bool> {
    let mut narrow: Vec<bool> = (0..f.slots).map(|s| s >= f.params).collect();
    loop {
        let mut changed = false;
        for (k, op) in f.code.iter().enumerate() {
            let Op::Store(s) = op else { continue };
            if !narrow[*s as usize] {
                continue;
            }
            let fits = match k.checked_sub(1).map(|p| &f.code[p]) {
                Some(Op::Sext(w)) => *w <= 32,
                Some(Op::Push(c)) => i32::try_from(*c).is_ok(),
                Some(Op::Load(t)) => narrow[*t as usize],
                Some(Op::LoadM(w)) => *w <= 4,
                Some(Op::Bin(b)) if b.is_comparison() || b.is_float_comparison() => true,
                _ => ok_inc(k),
            };
            if !fits {
                narrow[*s as usize] = false;
                changed = true;
            }
        }
        if !changed {
            return narrow;
        }
    }
}

/// A vectorizable loop's shape: `top: Load c; bound; Lt; Jz end; body;
/// c = c + 1 (cut to 32 bits or not); Jmp top; end:`.
struct Shape {
    t: usize,
    j: usize,
    top: u32,
    c: u32,
    bound: Op,
    cut: bool,
    /// the body without the counter's step
    body: std::ops::Range<usize>,
}

fn shape(code: &[Op], j: usize) -> Option<Shape> {
    let (Op::Jmp(top), Some(Op::Label(end))) = (&code[j], code.get(j + 1)) else { return None };
    let t = code[..j].iter().rposition(|o| *o == Op::Label(*top))?;
    if t == 0 || matches!(code[t - 1], Op::Jmp(_) | Op::Ret | Op::Fail(..) | Op::FailMem(_)) || j < t + 5 {
        return None;
    }
    let (Op::Load(c), bound, Op::Bin(BinOp::Lt), Op::Jz(e)) = (&code[t + 1], &code[t + 2], &code[t + 3], &code[t + 4]) else { return None };
    if e != end || !matches!(bound, Op::Push(_) | Op::Load(_)) || *bound == Op::Load(*c) {
        return None;
    }
    let b = &code[t + 5..j];
    let (inc, cut) = match b {
        [.., Op::Load(a), Op::Push(1), Op::Bin(BinOp::Add), Op::Sext(32), Op::Store(z)] if a == c && z == c => (5, true),
        [.., Op::Load(a), Op::Push(1), Op::Bin(BinOp::Add), Op::Store(z)] if a == c && z == c => (4, false),
        _ => return None,
    };
    if b.len() > MAX_BODY {
        return None;
    }
    Some(Shape { t, j, top: *top, c: *c, bound: bound.clone(), cut, body: t + 5..j - inc })
}

/// An address: the sum of invariant terms, plus the counter times coef,
/// plus a displacement.
#[derive(Clone, Debug)]
struct Aff {
    terms: Vec<usize>,
    coef: i64,
    disp: i64,
}

struct Vz<'a> {
    f: &'a Func,
    sh: &'a Shape,
    nodes: Vec<N>,
    stmts: Vec<S>,
    /// slots stored in the body
    stored: Vec<u32>,
    /// reductions: (statement, accumulator, the operation, the other
    /// operand, cut to 32 bits)
    reductions: Vec<(usize, u32, BinOp, usize, bool)>,
    narrow: Vec<bool>,
    /// the lanes of the integers (I32 for elements of 4 bytes, else I64)
    k: Lanes,
    lanes: i64,
    /// code before the vector loop, its body
    pre: Vec<Op>,
    body: Vec<Op>,
    slots: u32,
    used: Vec<bool>,
    pinned: Vec<bool>,
    /// registers the body has used for its own values
    touched: Vec<bool>,
    memo: std::collections::HashMap<usize, u8>,
    splats: std::collections::HashMap<String, u8>,
    bases: std::collections::HashMap<String, Op>,
    /// how many uses of each node are still to be compiled
    uses: Vec<u32>,
    /// affine values of the counter: (key, a register per copy, the step's)
    induction: Vec<(String, Vec<u8>, u8)>,
    /// how many vectors' worth each time round, and which one is compiled
    unroll: i64,
    copy: i64,
}

impl<'a> Vz<'a> {
    fn is_counter(&self, n: usize) -> bool {
        self.nodes[n] == N::Load(self.sh.c)
    }

    /// Worked out the same in every lane: constants, locals the loop does
    /// not store, and operations on them that cannot fail.
    fn uniform(&self, n: usize) -> bool {
        match &self.nodes[n] {
            N::Push(_) | N::AddrG(_) | N::Frame(_) => true,
            N::Load(s) => !self.stored.contains(s) && *s != self.sh.c,
            N::Bin(b, x, y) => {
                let safe = match b {
                    BinOp::DivTrunc | BinOp::ModTrunc | BinOp::DivFloor | BinOp::ModFloor => matches!(self.nodes[*y], N::Push(d) if d != 0 && d != -1),
                    BinOp::AddOv | BinOp::SubOv | BinOp::MulOv | BinOp::ShlOv | BinOp::AndBool | BinOp::OrBool | BinOp::AndValue | BinOp::OrValue | BinOp::DivTrue | BinOp::In => false,
                    _ => true,
                };
                safe && self.uniform(*x) && self.uniform(*y)
            }
            N::Un(u, x) => *u != UnOp::NegOv && self.uniform(*x),
            N::Sext(_, x) => self.uniform(*x),
            N::LoadM(..) => false,
        }
    }

    /// The node's scalar code.
    fn scalar(&self, n: usize, out: &mut Vec<Op>) {
        match &self.nodes[n] {
            N::Push(c) => out.push(Op::Push(*c)),
            N::Load(s) => out.push(Op::Load(*s)),
            N::AddrG(g) => out.push(Op::AddrG(*g)),
            N::Frame(m) => out.push(Op::Frame(*m)),
            N::Bin(b, x, y) => {
                self.scalar(*x, out);
                self.scalar(*y, out);
                out.push(Op::Bin(*b));
            }
            N::Un(u, x) => {
                self.scalar(*x, out);
                out.push(Op::Un(*u));
            }
            N::Sext(w, x) => {
                self.scalar(*x, out);
                out.push(Op::Sext(*w));
            }
            N::LoadM(w, x) => {
                self.scalar(*x, out);
                out.push(Op::LoadM(*w));
            }
        }
    }

    fn key(&self, n: usize) -> String {
        let mut v = Vec::new();
        self.scalar(n, &mut v);
        format!("{v:?}")
    }

    fn affine(&self, n: usize) -> Option<Aff> {
        if self.uniform(n) {
            return Some(match self.nodes[n] {
                N::Push(c) => Aff { terms: vec![], coef: 0, disp: c },
                _ => Aff { terms: vec![n], coef: 0, disp: 0 },
            });
        }
        if self.is_counter(n) {
            return Some(Aff { terms: vec![], coef: 1, disp: 0 });
        }
        let scale = |a: Aff, m: i64| -> Option<Aff> {
            if !a.terms.is_empty() {
                return None;
            }
            Some(Aff { terms: vec![], coef: a.coef.checked_mul(m)?, disp: a.disp.checked_mul(m)? })
        };
        match self.nodes[n] {
            N::Bin(BinOp::Add, x, y) => {
                let (a, b) = (self.affine(x)?, self.affine(y)?);
                let mut terms = a.terms;
                terms.extend(b.terms);
                Some(Aff { terms, coef: a.coef.checked_add(b.coef)?, disp: a.disp.checked_add(b.disp)? })
            }
            N::Bin(BinOp::Sub, x, y) => match self.nodes[y] {
                N::Push(c) => {
                    let a = self.affine(x)?;
                    Some(Aff { disp: a.disp.checked_sub(c)?, ..a })
                }
                _ => None,
            },
            N::Bin(BinOp::Shl, x, y) => match self.nodes[y] {
                N::Push(k @ 0..=16) => scale(self.affine(x)?, 1 << k),
                _ => None,
            },
            N::Bin(BinOp::Mul, x, y) => match (&self.nodes[x], &self.nodes[y]) {
                (_, N::Push(m)) => scale(self.affine(x)?, *m),
                (N::Push(m), _) => scale(self.affine(y)?, *m),
                _ => None,
            },
            // a C int counter plus a constant cut to 32 bits, where the
            // counter fits 32 bits and the sum does below the bound: the cut
            // changes nothing
            N::Sext(32, x) => {
                let a = self.affine(x)?;
                let Op::Push(bound) = self.sh.bound else { return None };
                let c = self.sh.c as usize;
                let fits = a.terms.is_empty() && a.coef == 1 && self.narrow[c] && a.disp >= 0 && bound.checked_add(a.disp).is_some_and(|e| e - 1 <= i32::MAX as i64);
                fits.then_some(a)
            }
            _ => None,
        }
    }

    /// Does the value fit 32 bits (sign-extended), in every lane?
    fn exact32(&self, n: usize) -> bool {
        match &self.nodes[n] {
            N::Push(c) => i32::try_from(*c).is_ok(),
            N::Load(s) => self.narrow[*s as usize] && !self.stored.contains(s),
            N::Sext(w, _) => *w <= 32,
            N::LoadM(w, _) => *w <= 4,
            N::Bin(BinOp::Shr, x, _) => self.exact32(*x),
            N::Bin(BinOp::Band | BinOp::Bor | BinOp::Bxor, x, y) => self.exact32(*x) && self.exact32(*y),
            N::Un(UnOp::Plus | UnOp::Bnot, x) => self.exact32(*x),
            _ => false,
        }
    }

    /// A register for a result: one of `like` that is free (the result
    /// worked out in place), else a free one that is not `avoid` (which the
    /// operation still reads after it begins writing).
    fn alloc_for(&mut self, like: &[u8], avoid: Option<u8>) -> Option<u8> {
        let v = like.iter().copied().find(|&v| !self.used[v as usize]).or_else(|| (KEPT..MAX_VECTORS).chain(0..KEPT).find(|&v| !self.used[v as usize] && Some(v) != avoid))?;
        self.used[v as usize] = true;
        self.pinned[v as usize] = false;
        self.touched[v as usize] = true;
        Some(v)
    }

    /// A free register; one kept for the whole loop (`pin`: set before
    /// it) must be one the body has not used yet.
    fn alloc(&mut self, pin: bool) -> Option<u8> {
        let v = if pin { (0..KEPT).find(|&v| !self.used[v as usize] && !self.touched[v as usize])? } else { (KEPT..MAX_VECTORS).chain(0..KEPT).find(|&v| !self.used[v as usize])? };
        self.used[v as usize] = true;
        self.pinned[v as usize] = pin;
        self.touched[v as usize] |= !pin;
        Some(v)
    }

    /// One use of node n compiled: its register may be taken now.
    fn release(&mut self, n: usize, v: u8) {
        self.uses[n] = self.uses[n].saturating_sub(1);
        // (a local made in the body is kept until its last use)
        let local = self.memo.get(&n) == Some(&v);
        if self.uses[n] == 0 && (local || !self.pinned[v as usize]) {
            self.used[v as usize] = false;
            self.pinned[v as usize] = false;
            self.memo.remove(&n);
        }
    }

    /// A register holding the constant c in every lane.
    fn splat_const(&mut self, l: Lanes, c: i64) -> Option<u8> {
        let key = format!("const {l:?} {c}");
        if let Some(&v) = self.splats.get(&key) {
            return Some(v);
        }
        let v = self.alloc(true)?;
        let x = splat(l, c);
        self.pre.push(Op::VConst(v, x[0] as i64, x[1] as i64));
        self.splats.insert(key, v);
        Some(v)
    }

    /// The scalar code of an element's address in the vector loop: lane
    /// 0's, unit stride (the counter times the element's width).
    fn address(&mut self, n: usize, w: u8, store: bool) -> Option<Vec<Op>> {
        let a = self.affine(n)?;
        if a.coef != w as i64 || a.terms.is_empty() {
            return None;
        }
        let mut keys: Vec<String> = a.terms.iter().map(|&t| self.key(t)).collect();
        keys.sort();
        let key = keys.join(" + ");
        let base = match self.bases.get(&key) {
            Some(b) => b.clone(),
            None => {
                let b = match (&a.terms[..], &self.nodes[a.terms[0]]) {
                    ([_], N::Load(s)) => Op::Load(*s),
                    _ => {
                        // worked out once before the loop (it cannot fail)
                        for (i, &t) in a.terms.iter().enumerate() {
                            let mut v = Vec::new();
                            self.scalar(t, &mut v);
                            self.pre.extend(v);
                            if i > 0 {
                                self.pre.push(Op::Bin(BinOp::Add));
                            }
                        }
                        let s = self.slots;
                        self.slots += 1;
                        self.pre.push(Op::Store(s));
                        Op::Load(s)
                    }
                };
                self.bases.insert(key, b.clone());
                b
            }
        };
        let _ = store;
        let mut out = vec![base, Op::Load(self.sh.c)];
        if w > 1 {
            out.extend([Op::Push(w.trailing_zeros() as i64), Op::Bin(BinOp::Shl)]);
        }
        out.push(Op::Bin(BinOp::Add));
        // copy m of an unrolled body: m vectors further
        let disp = a.disp.checked_add(self.copy * self.lanes * w as i64)?;
        if disp != 0 {
            out.extend([Op::Push(disp), Op::Bin(BinOp::Add)]);
        }
        Some(out)
    }

    /// The (base terms' key, displacement) of an access.
    fn access(&self, n: usize) -> Option<(String, i64)> {
        let a = self.affine(n)?;
        let mut keys: Vec<String> = a.terms.iter().map(|&t| self.key(t)).collect();
        keys.sort();
        Some((keys.join(" + "), a.disp))
    }

    /// The vector register holding node n's lanes (compiled into the body,
    /// or before the loop when it is the same in every lane). `l` is what
    /// the user takes it as (doubles or integers).
    fn value(&mut self, n: usize, l: Lanes) -> Option<u8> {
        if let Some(&v) = self.memo.get(&n) {
            return Some(v);
        }
        if self.uniform(n) {
            if matches!(self.nodes[n], N::AddrG(_) | N::Frame(_)) {
                return None;
            }
            let key = format!("{l:?} {}", self.key(n));
            if let Some(&v) = self.splats.get(&key) {
                return Some(v);
            }
            let v = self.alloc(true)?;
            let mut code = Vec::new();
            self.scalar(n, &mut code);
            self.pre.extend(code);
            self.pre.push(Op::VSplat(l, v));
            self.splats.insert(key, v);
            return Some(v);
        }
        let k = self.k;
        // an affine function of the counter (c, c + r, 8c + 3...): its own
        // lanes, one register per copy, stepped each time round
        if let Some(a) = self.affine(n) {
            if a.coef != 0 {
                return self.induction(n, a);
            }
        }
        let v = match self.nodes[n].clone() {
            N::LoadM(w, a) => {
                if w != k.width() {
                    return None;
                }
                let addr = self.address(a, w, false)?;
                let d = self.alloc(false)?;
                self.body.extend(addr);
                self.body.push(Op::VLoad(l, d));
                d
            }
            N::Bin(BinOp::Mul, x, y) if matches!((&self.nodes[x], &self.nodes[y]), (_, N::Push(_)) | (N::Push(_), _)) && self.by_shifts(x, y).is_some() => {
                let (o, c) = self.by_shifts(x, y)?;
                self.mul_const(o, c)?
            }
            N::Bin(b, x, y) => {
                let lanes = if b.is_float() { Lanes::F64 } else { k };
                let shift = matches!(b, BinOp::Shl | BinOp::Shr);
                let ok = match b {
                    BinOp::FAdd | BinOp::FSub | BinOp::FMul | BinOp::FDiv => k == Lanes::I64,
                    BinOp::Add | BinOp::Sub | BinOp::Band | BinOp::Bor | BinOp::Bxor => true,
                    // SSE2 has no 64-bit product: three pmuludq lose to imul
                    BinOp::Mul => k == Lanes::I32,
                    BinOp::Shl => true,
                    BinOp::Shr => k == Lanes::I64 || self.exact32(x),
                    _ => false,
                };
                if !ok {
                    return None;
                }
                if shift {
                    let N::Push(c) = self.nodes[y] else { return None };
                    if !(0..8 * k.width() as i64).contains(&c) {
                        return None;
                    }
                    let rx = self.value(x, lanes)?;
                    self.release(x, rx);
                    let d = self.alloc_for(&[rx], None)?;
                    self.body.push(Op::VShift(k, b, d, rx, c as u8));
                    d
                } else {
                    let rx = self.value(x, lanes)?;
                    let ry = self.value(y, lanes)?;
                    self.release(x, rx);
                    self.release(y, ry);
                    // the integer operations take their operands either way
                    // round; a float's keep their order (which NaN comes out)
                    let swaps = matches!(b, BinOp::Add | BinOp::Mul | BinOp::Band | BinOp::Bor | BinOp::Bxor);
                    let d = if swaps { self.alloc_for(&[rx, ry], None)? } else { self.alloc_for(&[rx], Some(ry))? };
                    self.body.push(Op::VBin(lanes, b, d, rx, ry));
                    d
                }
            }
            N::Un(u, x) => {
                let (lanes, with) = match u {
                    UnOp::Plus => return self.value(x, l),
                    UnOp::FSqrt => (Lanes::F64, None),
                    // a sign's bit turned or taken away: exactly the scalar one
                    UnOp::FNeg if k == Lanes::I64 => (Lanes::F64, Some((BinOp::Bxor, i64::MIN))),
                    UnOp::FAbs if k == Lanes::I64 => (Lanes::F64, Some((BinOp::Band, i64::MAX))),
                    UnOp::Bnot => (k, Some((BinOp::Bxor, -1))),
                    UnOp::Neg => (k, Some((BinOp::Sub, 0))),
                    _ => return None,
                };
                if lanes == Lanes::F64 && k != Lanes::I64 {
                    return None;
                }
                let rx = self.value(x, lanes)?;
                let d = match with {
                    None => {
                        self.release(x, rx);
                        let d = self.alloc_for(&[rx], None)?;
                        self.body.push(Op::VUn(lanes, u, d, rx));
                        d
                    }
                    Some((BinOp::Sub, c)) => {
                        let z = self.splat_const(k, c)?;
                        self.release(x, rx);
                        let d = self.alloc_for(&[], Some(rx))?;
                        self.body.push(Op::VBin(k, BinOp::Sub, d, z, rx));
                        d
                    }
                    Some((b, c)) => {
                        let m = self.splat_const(k, c)?;
                        self.release(x, rx);
                        let d = self.alloc_for(&[rx], None)?;
                        self.body.push(Op::VBin(lanes, b, d, rx, m));
                        d
                    }
                };
                d
            }
            // 32-bit lanes are the cut already
            N::Sext(32, x) if k == Lanes::I32 => return self.value(x, l),
            _ => return None,
        };
        Some(v)
    }

    /// The register of copy `self.copy` of an affine value a = coef c +
    /// terms + disp: worked out before the loop as splat(a at c) + coef
    /// (m L + lane), stepped by coef U L each time round (wrapping: the
    /// lanes are exact modulo their width).
    fn induction(&mut self, n: usize, a: Aff) -> Option<u8> {
        let mut keys: Vec<String> = a.terms.iter().map(|&t| self.key(t)).collect();
        keys.sort();
        let key = format!("{} * c + {} + {}", a.coef, keys.join(" + "), a.disp);
        let _ = n;
        if let Some((_, regs, _)) = self.induction.iter().find(|x| x.0 == key) {
            return Some(regs[self.copy as usize]);
        }
        let k = self.k;
        let step = self.splat_const(k, a.coef.wrapping_mul(self.lanes * self.unroll))?;
        let mut base = vec![Op::Load(self.sh.c)];
        if a.coef != 1 {
            base.extend([Op::Push(a.coef), Op::Bin(BinOp::Mul)]);
        }
        for &t in &a.terms {
            self.scalar(t, &mut base);
            base.push(Op::Bin(BinOp::Add));
        }
        if a.disp != 0 {
            base.extend([Op::Push(a.disp), Op::Bin(BinOp::Add)]);
        }
        let mut regs = Vec::new();
        for _ in 0..self.unroll {
            regs.push(self.alloc(true)?);
        }
        let tmp = self.alloc(true)?;
        for (m, &r) in regs.iter().enumerate() {
            self.pre.extend(base.iter().cloned());
            self.pre.push(Op::VSplat(k, r));
            let lanes: Vec<i64> = (0..4).map(|i| a.coef.wrapping_mul(m as i64 * self.lanes + i)).collect();
            let x = from_lanes(k, &lanes);
            self.pre.extend([Op::VConst(tmp, x[0] as i64, x[1] as i64), Op::VBin(k, BinOp::Add, r, r, tmp)]);
        }
        self.used[tmp as usize] = false;
        self.pinned[tmp as usize] = false;
        let r = regs[self.copy as usize];
        self.induction.push((key, regs, step));
        Some(r)
    }

    /// A product by a constant that two shifts and an add or a subtraction
    /// make (and a negation): (the other operand, the constant).
    fn by_shifts(&self, x: usize, y: usize) -> Option<(usize, i64)> {
        let (o, c) = match (&self.nodes[x], &self.nodes[y]) {
            (_, N::Push(c)) => (x, *c),
            (N::Push(c), _) => (y, *c),
            _ => return None,
        };
        let m = c.checked_abs()? as u64;
        if m == 0 {
            return None;
        }
        let lo = m.trailing_zeros();
        let rest = m >> lo;
        let bits = 8 * self.k.width() as u32;
        let top = if rest == 1 {
            lo
        } else if (rest - 1).is_power_of_two() {
            (rest - 1).trailing_zeros() + lo
        } else if (rest + 1).is_power_of_two() {
            (rest + 1).trailing_zeros() + lo
        } else {
            return None;
        };
        (m > 1 && top < bits).then_some((o, c))
    }

    /// o * c as by_shifts makes it: (o << a) ± (o << b), negated for c < 0;
    /// exact modulo the lane's width.
    fn mul_const(&mut self, o: usize, c: i64) -> Option<u8> {
        let k = self.k;
        let m = c.unsigned_abs();
        let lo = m.trailing_zeros() as u8;
        let rest = m >> lo;
        let rx = self.value(o, k)?;
        let low = if lo == 0 {
            rx
        } else {
            let d = self.alloc(false)?;
            self.body.push(Op::VShift(k, BinOp::Shl, d, rx, lo));
            d
        };
        let r = if rest == 1 {
            low
        } else {
            let (a, op) = if (rest - 1).is_power_of_two() { ((rest - 1).trailing_zeros() as u8 + lo, BinOp::Add) } else { ((rest + 1).trailing_zeros() as u8 + lo, BinOp::Sub) };
            let hi = self.alloc(false)?;
            self.body.push(Op::VShift(k, BinOp::Shl, hi, rx, a));
            self.body.push(Op::VBin(k, op, hi, hi, low));
            if low != rx {
                self.used[low as usize] = false;
            }
            hi
        };
        self.release(o, rx);
        if c > 0 {
            return Some(r);
        }
        let zero = self.splat_const(k, 0)?;
        self.body.push(Op::VBin(k, BinOp::Sub, r, zero, r));
        Some(r)
    }

    /// Counts every node's uses as a value.
    fn count(&mut self, n: usize) {
        self.uses[n] += 1;
        if self.uses[n] > 1 || self.uniform(n) {
            return;
        }
        match self.nodes[n].clone() {
            N::Bin(_, x, y) => {
                self.count(x);
                self.count(y);
            }
            N::Un(_, x) | N::Sext(_, x) => self.count(x),
            _ => {}
        }
    }
}

/// Reads the body into trees and statements; None where it is not only
/// stores of expressions (a call, a jump, a print, a Dup...).
/// (Also where each node and statement is in the code.)
#[allow(clippy::type_complexity)]
fn read(code: &[Op], sh: &Shape) -> Option<(Vec<N>, Vec<S>, Vec<usize>, Vec<usize>)> {
    let mut nodes = Vec::new();
    let mut stmts = Vec::new();
    let mut node_at: Vec<usize> = Vec::new();
    let mut stmt_at: Vec<usize> = Vec::new();
    let mut st: Vec<usize> = Vec::new();
    let mut temp: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let mut read_before: Vec<u32> = Vec::new();
    let new = |nodes: &mut Vec<N>, n: N| {
        nodes.push(n);
        nodes.len() - 1
    };
    for k in sh.body.clone() {
        let op = &code[k];
        let made = nodes.len();
        let stmts_before = stmts.len();
        match op {
            Op::Push(v) => st.push(new(&mut nodes, N::Push(*v))),
            Op::Load(s) => match temp.get(s) {
                Some(&n) => st.push(n),
                None => {
                    read_before.push(*s);
                    st.push(new(&mut nodes, N::Load(*s)))
                }
            },
            Op::AddrG(g) => st.push(new(&mut nodes, N::AddrG(*g))),
            Op::Frame(m) => st.push(new(&mut nodes, N::Frame(*m))),
            Op::Bin(b) => {
                let y = st.pop()?;
                let x = st.pop()?;
                st.push(new(&mut nodes, N::Bin(*b, x, y)));
            }
            Op::Un(u) => {
                let x = st.pop()?;
                st.push(new(&mut nodes, N::Un(*u, x)));
            }
            Op::Sext(w) => {
                let x = st.pop()?;
                st.push(new(&mut nodes, N::Sext(*w, x)));
            }
            Op::LoadM(w) => {
                let x = st.pop()?;
                st.push(new(&mut nodes, N::LoadM(*w, x)));
            }
            Op::Store(s) => {
                let v = st.pop()?;
                if !st.is_empty() || *s == sh.c || temp.contains_key(s) || stmts.iter().any(|x| matches!(x, S::Store(t, _) if t == s)) {
                    return None;
                }
                stmts.push(S::Store(*s, v));
                // a local made here and read later in the body (never read
                // before it here: that would carry a value round the loop)
                if !read_before.contains(s) {
                    temp.insert(*s, v);
                }
            }
            Op::StoreM(w) => {
                let v = st.pop()?;
                let a = st.pop()?;
                if !st.is_empty() {
                    return None;
                }
                stmts.push(S::StoreM(*w, a, v));
            }
            Op::OpM(b, w) => {
                let v = st.pop()?;
                let a = st.pop()?;
                if !st.is_empty() {
                    return None;
                }
                stmts.push(S::OpM(*b, *w, a, v));
            }
            _ => return None,
        }
        node_at.resize(made, 0);
        node_at.resize(nodes.len(), k);
        if stmts.len() > stmts_before {
            stmt_at.push(k);
        }
    }
    node_at.resize(nodes.len(), 0);
    st.is_empty().then_some((nodes, stmts, node_at, stmt_at))
}

fn reaches(nodes: &[N], n: usize, target: &N) -> usize {
    let here = (nodes[n] == *target) as usize;
    here + match &nodes[n] {
        N::Bin(_, x, y) => reaches(nodes, *x, target) + reaches(nodes, *y, target),
        N::Un(_, x) | N::Sext(_, x) | N::LoadM(_, x) => reaches(nodes, *x, target),
        _ => 0,
    }
}

/// The loop vectorized: the code to put before its top (the vector loop,
/// and after it the reductions' lanes added in), and the slots it made.
/// `checked`: only ever entered past the checks of the loop before it,
/// which covered every time round left.
fn vectorize(f: &Func, sh: &Shape, globals: u32, unroll: i64, checked: bool) -> Option<(Vec<Op>, u32)> {
    let code = &f.code;
    let (nodes, stmts, node_at, stmt_at) = read(code, sh)?;
    let stored: Vec<u32> = stmts.iter().filter_map(|s| if let S::Store(x, _) = s { Some(*x) } else { None }).collect();
    if let Op::Load(nb) = sh.bound {
        if stored.contains(&nb) {
            return None;
        }
    }
    // the widths of the elements: one for the whole loop
    let mut widths: Vec<u8> = nodes.iter().filter_map(|n| if let N::LoadM(w, _) = n { Some(*w) } else { None }).collect();
    for s in &stmts {
        if let S::StoreM(w, ..) | S::OpM(_, w, ..) = s {
            widths.push(*w);
        }
    }
    widths.sort();
    widths.dedup();
    // the counter's step keeps it within 32 bits where the bound fits them
    let inc_at = sh.j - 1;
    let bound_fits = |narrow: &[bool]| match sh.bound {
        Op::Push(n) => i32::try_from(n).is_ok(),
        Op::Load(nb) => narrow[nb as usize],
        _ => false,
    };
    let pre_narrow = narrow32(f, &|_| false);
    let fits = bound_fits(&pre_narrow);
    let narrow = narrow32(f, &|k| k == inc_at && fits);
    // the reductions: s = s op E (or E op s), s read nowhere else in the body
    let mut reductions = Vec::new();
    for (i, s) in stmts.iter().enumerate() {
        let S::Store(x, v) = s else { continue };
        let me = N::Load(*x);
        let all: usize = stmts.iter().map(|t| match t {
            S::Store(_, v) => reaches(&nodes, *v, &me),
            S::StoreM(_, a, v) | S::OpM(_, _, a, v) => reaches(&nodes, *a, &me) + reaches(&nodes, *v, &me),
        }).sum();
        if all == 0 {
            continue; // a local made in the body
        }
        let (top, cut) = match nodes[*v] {
            N::Sext(32, y) => (y, true),
            _ => (*v, false),
        };
        let N::Bin(op, l, r) = nodes[top] else { return None };
        let other = match op {
            BinOp::Add | BinOp::Band | BinOp::Bor | BinOp::Bxor if nodes[l] == me => r,
            BinOp::Add | BinOp::Band | BinOp::Bor | BinOp::Bxor if nodes[r] == me => l,
            BinOp::Sub if nodes[l] == me => r,
            _ => return None,
        };
        if all != 1 || (cut && !narrow[*x as usize]) {
            return None;
        }
        reductions.push((i, *x, op, other, cut));
    }
    // a local made in the body is read nowhere else
    for &x in &stored {
        if reductions.iter().any(|r| r.1 == x) {
            continue;
        }
        let outside = code.iter().enumerate().any(|(k, o)| *o == Op::Load(x) && !sh.body.contains(&k));
        if outside {
            return None;
        }
    }
    let k = match widths[..] {
        [4] => Lanes::I32,
        [8] => Lanes::I64,
        [] if !reductions.is_empty() && reductions.iter().all(|r| r.4) => Lanes::I32,
        [] => Lanes::I64,
        _ => return None,
    };
    // in 32-bit lanes, a reduction is a C int's (cut to 32 bits)
    if k == Lanes::I32 && reductions.iter().any(|r| !r.4) {
        return None;
    }
    let lanes = k.count();
    let worth = !reductions.is_empty() || stmts.iter().any(|s| !matches!(s, S::Store(..)));
    if !worth {
        return None;
    }
    let mut z = Vz {
        f,
        sh,
        uses: vec![0; nodes.len()],
        nodes,
        stmts,
        stored,
        reductions,
        narrow,
        k,
        lanes,
        pre: Vec::new(),
        body: Vec::new(),
        slots: f.slots,
        used: vec![false; MAX_VECTORS as usize],
        pinned: vec![false; MAX_VECTORS as usize],
        touched: vec![false; MAX_VECTORS as usize],
        memo: Default::default(),
        splats: Default::default(),
        bases: Default::default(),
        induction: Vec::new(),
        unroll,
        copy: 0,
    };
    let _ = z.f;
    // the loads of a local made in the body are of nodes, not of the local
    for n in 0..z.nodes.len() {
        if let N::Load(s) = z.nodes[n] {
            if s != sh.c && z.stored.contains(&s) && !z.reductions.iter().any(|r| r.1 == s) {
                return None;
            }
        }
    }
    // the memory the body reads and writes: a store and another access of
    // the same object (or of one that may be anything) must be at the same
    // address every time round, so no time round reads what another wrote
    // (a store, its address node, where it is in the code)
    let mut accesses: Vec<(bool, usize, usize)> = Vec::new();
    for n in 0..z.nodes.len() {
        if let N::LoadM(_, a) = z.nodes[n] {
            accesses.push((false, a, node_at[n]));
        }
    }
    for (i, s) in z.stmts.iter().enumerate() {
        if let S::StoreM(_, a, _) | S::OpM(_, _, a, _) = s {
            accesses.push((true, *a, stmt_at[i]));
        }
    }
    let addr_nodes: Vec<usize> = accesses.iter().map(|x| x.1).collect();
    let facts = super::alias::Facts::new(f, globals);
    let mut seen = Vec::new();
    for &(store, a, at) in &accesses {
        let mem = if store { facts.writes(at) } else { facts.reads(at) };
        seen.push((store, a, z.access(a)?, mem));
    }
    // a store and another access: at the same address every time round
    // (each time round reads only what it wrote itself), or apart as
    // alias.rs finds them over the whole loop, or else checked apart
    // before the vector loop runs, exactly
    let mut checks: Vec<(usize, usize)> = Vec::new();
    for (i, (si, ai, (ki, di), mi)) in seen.iter().enumerate() {
        for (j, (sj, aj, (kj, dj), mj)) in seen.iter().enumerate() {
            if i == j || !(*si || *sj) || (ki == kj && di == dj) {
                continue;
            }
            // the same base a few elements apart: what one time round
            // writes, another reads
            if ki == kj {
                return None;
            }
            if let (Some(x), Some(y)) = (mi, mj) {
                if !super::alias::may_alias(*x, *y) {
                    continue;
                }
            }
            let pair = (*ai.min(aj), *ai.max(aj));
            if !checks.contains(&pair) {
                checks.push(pair);
            }
        }
    }
    if checks.len() > MAX_CHECKS {
        return None;
    }
    // a node used as an address is never also a value
    for (i, s) in z.stmts.clone().iter().enumerate() {
        // (a local made in the body counts where it is read)
        let vals: Vec<usize> = match s {
            S::Store(..) => match z.reductions.iter().find(|r| r.0 == i) {
                Some(r) => vec![r.3],
                None => vec![],
            },
            S::StoreM(_, _, v) | S::OpM(_, _, _, v) => vec![*v],
        };
        for v in vals {
            z.count(v);
        }
    }
    for &a in &addr_nodes {
        if z.uses[a] > 0 {
            return None;
        }
    }
    // the limit: c + lanes - 1 < bound is c < bound - (lanes - 1), worked
    // out without wrapping
    let top = sh.top;
    let labels = super::ir::label_count(code);
    let (vtop, vend) = (labels, labels + 1);
    let step = lanes * unroll;
    let limit = match sh.bound {
        Op::Push(n) => Op::Push(n.checked_sub(step - 1)?),
        Op::Load(nb) => {
            if !z.narrow[nb as usize] {
                // a bound this low leaves the vector loop out
                z.pre.extend([Op::Load(nb), Op::Push(i64::MIN + step - 1), Op::Bin(BinOp::Lt), Op::Jnz(top)]);
            }
            let s = z.slots;
            z.slots += 1;
            z.pre.extend([Op::Load(nb), Op::Push(step - 1), Op::Bin(BinOp::Sub), Op::Store(s)]);
            Op::Load(s)
        }
        _ => return None,
    };
    // the accumulators start at the operation's identity
    // (one for each copy of an unrolled body: their chains are apart)
    let mut accs: Vec<Vec<u8>> = Vec::new();
    for r in z.reductions.clone() {
        let mut each = Vec::new();
        for _ in 0..unroll {
            let v = z.alloc(true)?;
            let id = if r.2 == BinOp::Band { -1 } else { 0 };
            z.pre.push(Op::VConst(v, id, id));
            each.push(v);
        }
        accs.push(each);
    }
    for m in 0..unroll {
    z.copy = m;
    if m > 0 {
        z.uses.iter_mut().for_each(|u| *u = 0);
        for (i, s) in z.stmts.clone().iter().enumerate() {
            let vals: Vec<usize> = match s {
                S::Store(..) => match z.reductions.iter().find(|r| r.0 == i) {
                    Some(r) => vec![r.3],
                    None => vec![],
                },
                S::StoreM(_, _, v) | S::OpM(_, _, _, v) => vec![*v],
            };
            for v in vals {
                z.count(v);
            }
        }
    }
    for (i, s) in z.stmts.clone().iter().enumerate() {
        match s {
            S::Store(_, v) => {
                if let Some(ri) = z.reductions.iter().position(|r| r.0 == i) {
                    let r = z.reductions[ri];
                    let x = z.value(r.3, k)?;
                    z.release(r.3, x);
                    let acc = accs[ri][m as usize];
                    z.body.push(Op::VBin(k, if r.2 == BinOp::Sub { BinOp::Sub } else { r.2 }, acc, acc, x));
                } else if !addr_nodes.contains(v) && z.uses[*v] > 0 {
                    // worked out here, where the scalar code works it out
                    // an element a float operation takes is loaded as doubles
                    let taken = z.nodes.iter().any(|m| matches!(m, N::Bin(b, p, q) if b.is_float() && (p == v || q == v)) || matches!(m, N::Un(u, p) if u.is_float() && p == v));
                    let float = taken || matches!(z.nodes[*v], N::Bin(b, ..) if b.is_float()) || matches!(z.nodes[*v], N::Un(u, _) if u.is_float());
                    let x = z.value(*v, if float { Lanes::F64 } else { k })?;
                    // (a register already kept, a splat's or the counter's, stays so)
                    if !z.pinned[x as usize] {
                        z.memo.insert(*v, x);
                        z.pinned[x as usize] = true;
                    }
                }
            }
            S::StoreM(w, a, v) => {
                let float = matches!(z.nodes[*v], N::Bin(b, ..) if b.is_float()) || matches!(z.nodes[*v], N::Un(u, _) if u.is_float());
                let l = if float { Lanes::F64 } else { k };
                if *w != k.width() {
                    return None;
                }
                let x = z.value(*v, l)?;
                z.release(*v, x);
                let addr = z.address(*a, *w, true)?;
                z.body.extend(addr);
                z.body.push(Op::VStore(l, x));
            }
            S::OpM(b, w, a, v) => {
                let ok = match b {
                    BinOp::Add | BinOp::Sub | BinOp::Band | BinOp::Bor | BinOp::Bxor => true,
                    BinOp::Mul => k == Lanes::I32,
                    BinOp::FAdd | BinOp::FSub | BinOp::FMul | BinOp::FDiv => k == Lanes::I64,
                    _ => false,
                };
                if !ok || *w != k.width() {
                    return None;
                }
                let l = if b.is_float() { Lanes::F64 } else { k };
                let x = z.value(*v, l)?;
                let addr = z.address(*a, *w, false)?;
                let back = z.address(*a, *w, true)?;
                let d = z.alloc(false)?;
                z.body.extend(addr);
                z.body.push(Op::VLoad(l, d));
                z.body.push(Op::VBin(l, *b, d, d, x));
                z.release(*v, x);
                z.body.extend(back);
                z.body.push(Op::VStore(l, d));
                z.used[d as usize] = false;
            }
        }
        // a local made in the body whose last use has passed gives its
        // register back
        for (&n, &v) in &z.memo.clone() {
            if z.uses[n] == 0 {
                z.used[v as usize] = false;
                z.pinned[v as usize] = false;
                z.memo.remove(&n);
            }
        }
    }
    }
    // the checks: the accesses of all the times round left touch the bytes
    // [lo, lo + (bound - c) w) from lane 0's address now; two such runs
    // meet where either's start lies less than the other's length past
    // it, counted round the addresses modulo 2^64 (exact: memory's own
    // addresses wrap so); bound - c must be below 2^40, so the lengths do
    // not wrap. Where they may meet, only the scalar loop runs.
    if !checks.is_empty() && !checked {
        z.copy = 0;
        let w = k.width();
        let len = z.slots;
        z.slots += 1;
        z.pre.extend([sh.bound.clone(), Op::Load(sh.c), Op::Bin(BinOp::Sub), Op::Push(1 << 40), Op::Bin(BinOp::Ult), Op::Jz(top)]);
        z.pre.extend([sh.bound.clone(), Op::Load(sh.c), Op::Bin(BinOp::Sub), Op::Push(w.trailing_zeros() as i64), Op::Bin(BinOp::Shl), Op::Store(len)]);
        let mut lo: Vec<(usize, u32)> = Vec::new();
        for &(x, y) in &checks.clone() {
            for n in [x, y] {
                if !lo.iter().any(|l| l.0 == n) {
                    let addr = z.address(n, w, false)?;
                    let s = z.slots;
                    z.slots += 1;
                    z.pre.extend(addr);
                    z.pre.push(Op::Store(s));
                    lo.push((n, s));
                }
            }
            let at = |n: usize| lo.iter().find(|l| l.0 == n).unwrap().1;
            let (lx, ly) = (at(x), at(y));
            for (p, q) in [(lx, ly), (ly, lx)] {
                z.pre.extend([Op::Load(p), Op::Load(q), Op::Bin(BinOp::Sub), Op::Load(len), Op::Bin(BinOp::Ult), Op::Jnz(top)]);
            }
        }
    }
    let mut out = std::mem::take(&mut z.pre);
    out.push(Op::Label(vtop));
    out.extend([Op::Load(sh.c), limit, Op::Bin(BinOp::Lt), Op::Jz(vend)]);
    out.extend(std::mem::take(&mut z.body));
    for (_, regs, step) in &z.induction {
        for &v in regs {
            out.push(Op::VBin(k, BinOp::Add, v, v, *step));
        }
    }
    out.extend([Op::Load(sh.c), Op::Push(step), Op::Bin(BinOp::Add)]);
    if sh.cut {
        out.push(Op::Sext(32));
    }
    out.extend([Op::Store(sh.c), Op::Jmp(vtop), Op::Label(vend)]);
    for (ri, r) in z.reductions.iter().enumerate() {
        let op = if r.2 == BinOp::Sub { BinOp::Add } else { r.2 };
        let a = accs[ri][0];
        for &b in &accs[ri][1..] {
            out.push(Op::VBin(k, op, a, a, b));
        }
        out.extend([Op::Load(r.1), Op::VReduce(k, op, a), Op::Bin(op)]);
        if r.4 {
            out.push(Op::Sext(32));
        }
        out.push(Op::Store(r.1));
    }
    Some((out, z.slots))
}

/// The vector loop put before the scalar one's top.
fn insert(f: &mut Func, sh: &Shape, vector: Vec<Op>, slots: u32, done: &mut Vec<u32>) {
    // the vector loop's labels are not tried again
    for op in &vector {
        if let Op::Label(l) = op {
            done.push(*l);
        }
    }
    let mut code = Vec::with_capacity(f.code.len() + vector.len());
    code.extend_from_slice(&f.code[..sh.t]);
    code.extend(vector);
    code.extend_from_slice(&f.code[sh.t..]);
    f.code = code;
    f.slots = slots;
}

/// Vectorizes the function's innermost counted loops; returns how many.
pub fn run(f: &mut Func, globals: u32) -> usize {
    if std::env::var_os("STRIKE_DUMP_VEC").is_some() {
        eprintln!("{} ({} params, {} slots) before vectorizing:", f.name, f.params, f.slots);
        for (i, op) in f.code.iter().enumerate() {
            eprintln!("  {i:4} {op:?}");
        }
    }
    // a function that already has vector locals is left as it is
    if super::ir::vector_locals(&f.code) > 0 {
        return 0;
    }
    let mut done: Vec<u32> = Vec::new();
    let mut count = 0;
    'again: loop {
        for j in 0..f.code.len() {
            let Some(sh) = shape(&f.code, j) else { continue };
            if done.contains(&sh.top) {
                continue;
            }
            done.push(sh.top);
            // four or two vectors each time round (independent work, one
            // test for all), or one where the registers do not suffice; after
            // an unrolled loop, a loop of one vector each time round takes
            // what is left of whole vectors
            let mut made = None;
            for u in [4, 2, 1] {
                if let Some(v) = vectorize(f, &sh, globals, u, false) {
                    made = Some((v, u));
                    break;
                }
            }
            let Some(((vector, slots), u)) = made else { continue };
            insert(f, &sh, vector, slots, &mut done);
            if u > 1 {
                        let again = (0..f.code.len()).find_map(|j| shape(&f.code, j).filter(|s| s.top == sh.top));
                if let Some(sh) = again {
                    if let Some((vector, slots)) = vectorize(f, &sh, globals, 1, true) {
                        insert(f, &sh, vector, slots, &mut done);
                    }
                }
            }
            count += 1;
            continue 'again;
        }
        break;
    }
    if count > 0 && std::env::var_os("STRIKE_DUMP_VEC").is_some() {
        eprintln!("{} vectorized:", f.name);
        for (i, op) in f.code.iter().enumerate() {
            eprintln!("  {i:4} {op:?}");
        }
    }
    count
}
