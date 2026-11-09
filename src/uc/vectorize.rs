//! --release: loop vectorization (SSE2).

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
/// The most vector locals a vectorized loop takes (each an xmm register).
const MAX_VECTORS: u8 = 10;

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

/// What an address points into: no object (a number), a global, the
/// function's own memory from an offset, or anything (a parameter, an
/// address read from memory). Distinct globals and distinct places of the
/// function's own memory never overlap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Prov {
    Int,
    Global(u32),
    Frame(u32),
    Any,
}

fn is_object(p: Prov) -> bool {
    matches!(p, Prov::Global(_) | Prov::Frame(_))
}

/// x + y: an object plus a number is in the object (C indexes an array
/// only from its start; Strike makes no other address arithmetic).
fn add_prov(x: Prov, y: Prov) -> Prov {
    match (x, y) {
        (Prov::Int, Prov::Int) => Prov::Int,
        (o, Prov::Int | Prov::Any) | (Prov::Int | Prov::Any, o) if is_object(o) => o,
        (Prov::Int, Prov::Any) | (Prov::Any, Prov::Int) | (Prov::Any, Prov::Any) => Prov::Any,
        _ => Prov::Any,
    }
}

fn join(a: Prov, b: Prov) -> Prov {
    match (a, b) {
        (Prov::Int, x) | (x, Prov::Int) => x,
        (x, y) if x == y => x,
        _ => Prov::Any,
    }
}

/// The provenance of a run of operations, given the locals'.
fn run_prov(run: &[Op], slot: &[Prov]) -> Prov {
    let mut st: Vec<Prov> = Vec::new();
    for op in run {
        match op {
            Op::Push(_) => st.push(Prov::Int),
            Op::Load(s) => st.push(slot[*s as usize]),
            Op::AddrG(g) => st.push(Prov::Global(*g)),
            Op::Frame(n) => st.push(Prov::Frame(*n)),
            Op::Bin(b) => {
                let (Some(y), Some(x)) = (st.pop(), st.pop()) else { return Prov::Any };
                st.push(match b {
                    BinOp::Add => add_prov(x, y),
                    BinOp::Sub if is_object(x) && !is_object(y) => x,
                    _ if is_object(x) || is_object(y) => Prov::Any,
                    _ => Prov::Int,
                });
            }
            Op::Un(_) | Op::Sext(_) => {
                let Some(x) = st.pop() else { return Prov::Any };
                st.push(if is_object(x) { Prov::Any } else { Prov::Int });
            }
            _ => return Prov::Any,
        }
    }
    if st.len() == 1 {
        st[0]
    } else {
        Prov::Any
    }
}

/// Every local's provenance: the join of what every store gives it (a
/// local starts at zero, a number; a parameter may be anything).
fn provenance(f: &Func) -> Vec<Prov> {
    let st = super::opt::starts(&f.code);
    let mut p: Vec<Prov> = (0..f.slots).map(|s| if s < f.params { Prov::Any } else { Prov::Int }).collect();
    loop {
        let mut changed = false;
        for (k, op) in f.code.iter().enumerate() {
            let Op::Store(s) = op else { continue };
            let got = match k.checked_sub(1).and_then(|q| st[q].map(|b| (b, q))) {
                Some((b, q)) => run_prov(&f.code[b..=q], &p),
                None => Prov::Any,
            };
            let new = join(p[*s as usize], got);
            if new != p[*s as usize] {
                p[*s as usize] = new;
                changed = true;
            }
        }
        if !changed {
            return p;
        }
    }
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
    prov: Vec<Prov>,
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
    induction: Option<(u8, u8)>,
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

    fn prov_of(&self, n: usize) -> Prov {
        let mut v = Vec::new();
        self.scalar(n, &mut v);
        run_prov(&v, &self.prov)
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
        let v = like.iter().copied().find(|&v| !self.used[v as usize]).or_else(|| (0..MAX_VECTORS).find(|&v| !self.used[v as usize] && Some(v) != avoid))?;
        self.used[v as usize] = true;
        self.pinned[v as usize] = false;
        self.touched[v as usize] = true;
        Some(v)
    }

    /// A free register; one kept for the whole loop (`pin`: set before
    /// it) must be one the body has not used yet.
    fn alloc(&mut self, pin: bool) -> Option<u8> {
        let v = (0..MAX_VECTORS).find(|&v| !self.used[v as usize] && !(pin && self.touched[v as usize]))?;
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
    fn address(&mut self, n: usize, w: u8) -> Option<Vec<Op>> {
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
        let mut out = vec![base, Op::Load(self.sh.c)];
        if w > 1 {
            out.extend([Op::Push(w.trailing_zeros() as i64), Op::Bin(BinOp::Shl)]);
        }
        out.push(Op::Bin(BinOp::Add));
        if a.disp != 0 {
            out.extend([Op::Push(a.disp), Op::Bin(BinOp::Add)]);
        }
        Some(out)
    }

    /// The (base terms' key, provenance, displacement) of an access.
    fn access(&self, n: usize) -> Option<(String, Prov, i64)> {
        let a = self.affine(n)?;
        let mut keys: Vec<String> = a.terms.iter().map(|&t| self.key(t)).collect();
        keys.sort();
        let mut p = Prov::Int;
        for &t in &a.terms {
            p = add_prov(p, self.prov_of(t));
        }
        if p == Prov::Int {
            p = Prov::Any;
        }
        Some((keys.join(" + "), p, a.disp))
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
        let v = match self.nodes[n].clone() {
            N::Load(s) if s == self.sh.c => {
                if let Some((v, _)) = self.induction {
                    return Some(v);
                }
                // the lanes c, c + 1, ...; each time round, the lanes' count added
                let v = self.alloc(true)?;
                let step = self.alloc(true)?;
                let iota: Vec<i64> = (0..4).collect();
                let i = from_lanes(k, &iota);
                let st = splat(k, self.lanes);
                self.pre.extend([Op::Load(s), Op::VSplat(k, v), Op::VConst(step, i[0] as i64, i[1] as i64), Op::VBin(k, BinOp::Add, v, v, step)]);
                self.pre.push(Op::VConst(step, st[0] as i64, st[1] as i64));
                self.induction = Some((v, step));
                return Some(v);
            }
            N::LoadM(w, a) => {
                if w != k.width() {
                    return None;
                }
                let addr = self.address(a, w)?;
                let d = self.alloc(false)?;
                self.body.extend(addr);
                self.body.push(Op::VLoad(l, d));
                d
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
fn read(code: &[Op], sh: &Shape) -> Option<(Vec<N>, Vec<S>)> {
    let mut nodes = Vec::new();
    let mut stmts = Vec::new();
    let mut st: Vec<usize> = Vec::new();
    let mut temp: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let mut read_before: Vec<u32> = Vec::new();
    let new = |nodes: &mut Vec<N>, n: N| {
        nodes.push(n);
        nodes.len() - 1
    };
    for op in &code[sh.body.clone()] {
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
    }
    st.is_empty().then_some((nodes, stmts))
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
fn vectorize(f: &Func, sh: &Shape, prov: &[Prov]) -> Option<(Vec<Op>, u32)> {
    let code = &f.code;
    let (nodes, stmts) = read(code, sh)?;
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
        prov: prov.to_vec(),
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
        induction: None,
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
    let mut accesses: Vec<(bool, usize, u8)> = Vec::new();
    for n in 0..z.nodes.len() {
        if let N::LoadM(w, a) = z.nodes[n] {
            accesses.push((false, a, w));
        }
    }
    let mut addr_nodes = Vec::new();
    for s in &z.stmts {
        if let S::StoreM(w, a, _) | S::OpM(_, w, a, _) = s {
            accesses.push((true, *a, *w));
        }
    }
    for &(_, a, _) in &accesses {
        addr_nodes.push(a);
    }
    let mut seen = Vec::new();
    for &(store, a, _) in &accesses {
        let acc = z.access(a)?;
        seen.push((store, acc));
    }
    for (i, (si, (ki, pi, di))) in seen.iter().enumerate() {
        for (j, (sj, (kj, pj, dj))) in seen.iter().enumerate() {
            if i == j || !(*si || *sj) {
                continue;
            }
            let apart = is_object(*pi) && is_object(*pj) && pi != pj;
            if !apart && (ki != kj || di != dj) {
                return None;
            }
        }
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
    let limit = match sh.bound {
        Op::Push(n) => Op::Push(n.checked_sub(lanes - 1)?),
        Op::Load(nb) => {
            if !z.narrow[nb as usize] {
                // a bound this low leaves the vector loop out
                z.pre.extend([Op::Load(nb), Op::Push(i64::MIN + lanes - 1), Op::Bin(BinOp::Lt), Op::Jnz(top)]);
            }
            let s = z.slots;
            z.slots += 1;
            z.pre.extend([Op::Load(nb), Op::Push(lanes - 1), Op::Bin(BinOp::Sub), Op::Store(s)]);
            Op::Load(s)
        }
        _ => return None,
    };
    // the accumulators start at the operation's identity
    let mut accs = Vec::new();
    for r in z.reductions.clone() {
        let v = z.alloc(true)?;
        let id = if r.2 == BinOp::Band { -1 } else { 0 };
        z.pre.push(Op::VConst(v, id, id));
        accs.push(v);
    }
    for (i, s) in z.stmts.clone().iter().enumerate() {
        match s {
            S::Store(_, v) => {
                if let Some(ri) = z.reductions.iter().position(|r| r.0 == i) {
                    let r = z.reductions[ri];
                    let x = z.value(r.3, k)?;
                    z.release(r.3, x);
                    let acc = accs[ri];
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
                let addr = z.address(*a, *w)?;
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
                let addr = z.address(*a, *w)?;
                let d = z.alloc(false)?;
                z.body.extend(addr.iter().cloned());
                z.body.push(Op::VLoad(l, d));
                z.body.push(Op::VBin(l, *b, d, d, x));
                z.release(*v, x);
                z.body.extend(addr);
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
    let mut out = std::mem::take(&mut z.pre);
    out.push(Op::Label(vtop));
    out.extend([Op::Load(sh.c), limit, Op::Bin(BinOp::Lt), Op::Jz(vend)]);
    out.extend(std::mem::take(&mut z.body));
    if let Some((v, step)) = z.induction {
        out.push(Op::VBin(k, BinOp::Add, v, v, step));
    }
    out.extend([Op::Load(sh.c), Op::Push(lanes), Op::Bin(BinOp::Add)]);
    if sh.cut {
        out.push(Op::Sext(32));
    }
    out.extend([Op::Store(sh.c), Op::Jmp(vtop), Op::Label(vend)]);
    for (ri, r) in z.reductions.iter().enumerate() {
        let op = if r.2 == BinOp::Sub { BinOp::Add } else { r.2 };
        out.extend([Op::Load(r.1), Op::VReduce(k, op, accs[ri]), Op::Bin(op)]);
        if r.4 {
            out.push(Op::Sext(32));
        }
        out.push(Op::Store(r.1));
    }
    Some((out, z.slots))
}

/// Vectorizes the function's innermost counted loops; returns how many.
pub fn run(f: &mut Func) -> usize {
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
        let prov = provenance(f);
        for j in 0..f.code.len() {
            let Some(sh) = shape(&f.code, j) else { continue };
            if done.contains(&sh.top) {
                continue;
            }
            done.push(sh.top);
            let Some((vector, slots)) = vectorize(f, &sh, &prov) else { continue };
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
