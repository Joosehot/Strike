//! --release: the algebraic simplifier, in the way of Nuome's (the owner's
//! math engine, whose search.rs and ring rules this ports): a beam search
//! over rewrite paths, where rules offer moves and a judge scores each
//! state. Here the judge is
//! the cost of the x86-64 code x64opt.rs makes of it.
//!
//! Every pure integer expression of the IR (Push, Load, a global's or the
//! frame's address, Bin, Un, Sext) is read as a tree. Its ring part (+, -,
//! *, unary -, ~, and << by a constant) is a polynomial over Z/2^64 in its
//! atoms: the leaves and every other operation (division, remainder,
//! comparisons, bitwise and/or/xor, shifts right, shifts by a variable,
//! Python's checked operations), whose operands are simplified on their
//! own. The rules (each one an identity of the ring Z/2^64, so of the
//! machine's wrapping arithmetic; Nuome's rule names in brackets):
//!
//! - associativity and commutativity: a sum is a list of terms, a term a
//!   coefficient times a list of factors, kept sorted [ring_add_assoc,
//!   ring_add_comm, ring_mul_assoc]; `x - y = x + (-1)y`, `-x = (-1)x`,
//!   `~x = (-1)x + (-1)` (two's complement), `x << k = 2^(k mod 64) x`
//!   [ring_neg, ring_mul_neg];
//! - multiply out: `a(b + c) = ab + ac` [multiply_out, distribute];
//! - collect like terms: `ca + da = (c + d)a`, and every product expanded
//!   and collected at once: the polynomial's normal form [collect];
//! - take out a common factor: `ab + ac = a(b + c)`, for any factor shared
//!   by two terms or more, which done again and again is Horner's form
//!   [factor_common];
//! - take out a common constant: `4a + 8b = 4(a + 2b)`, for a power of two
//!   or the coefficients' gcd;
//! - split a constant: `(2^k + 1)x = 2^k x + x` and `(2^k - 1)x = 2^k x -
//!   x`, which x64opt makes one `lea` or a shift and a `sub`;
//! - `0x = 0`, `1x = x`, `x + 0 = x` [ring_zero, ring_mul_zero, ring_zero_mul].
//!
//! Narrow widths: a C int's result is cut by Sext(32). The low n bits of a
//! ring expression depend only on the low n bits of its inputs, so under a
//! Sext(n) any Sext(m) with m >= n on the way to the atoms changes nothing
//! and is left out, and the coefficients are taken modulo 2^n. A Sext is
//! never moved across an atom (a comparison, a division, a shift right).
//!
//! The judge (`cost`) follows x64opt.rs's compile-time stack: a local in a
//! register plus a constant, times 1, 2, 4 or 8, plus another local, is one
//! `lea` or an address mode; a value that must wait under another goes to
//! the machine's stack; imul takes 3 cycles, a scaled lea 2; `t = t + rest`
//! is worked out in t's own register. It counts instructions plus the
//! latency to the result, a local stored just before (or round the loop)
//! being ready as late as the expression that made it: what is ready first
//! goes first. Inside a loop it knows what LICM (opt::hoist,
//! which runs next) will take out of the loop: an invariant part costs once,
//! before the loop, so the rules gather invariant terms and factors into one
//! subexpression that hoist can move.
//!
//! A rewrite is kept only when it is strictly cheaper, and only after the
//! original and the new code have given the same value on 200 sets of
//! inputs (0, 1, -1, the extremes, powers of two and random numbers),
//! evaluated with the machine's own rules: a guard, the proof being the
//! rules themselves. An expression with an operation that can fail (a
//! division by a variable, a checked Python operation) is never rewritten
//! around it, so it still stops exactly where it would; its operands are.

use super::ir::{label_count, loop_depths, BinOp, Func, Op, UnOp};
use std::collections::{BTreeMap, HashMap, HashSet};

/// How many states the beam keeps each round, and how many rounds.
const WIDTH: usize = 4;
const STEPS: usize = 8;
/// The most states judged for one expression (compile time stays small).
const BUDGET: usize = 30;
/// The most moves taken from one state.
const MOVES: usize = 16;
/// A polynomial with more terms than this is not multiplied out.
const MAX_TERMS: usize = 48;
/// When a local stored just before is taken to be ready, in cycles.
const LATE: f64 = 4.0;
/// A rewrite must save at least this much to be kept.
const MARGIN: f64 = 0.5;

// ---------------------------------------------------------------------------
// The expression tree

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(super) enum E {
    Push(i64),
    Load(u32),
    AddrG(u32),
    Frame(u32),
    Bin(BinOp, Box<E>, Box<E>),
    Un(UnOp, Box<E>),
    Sext(u8, Box<E>),
}

/// The run of operations as a tree (None if it is not one whole value).
pub(super) fn tree(run: &[Op]) -> Option<E> {
    let mut st: Vec<E> = Vec::new();
    for op in run {
        let e = match op {
            Op::Push(c) => E::Push(*c),
            Op::Load(s) => E::Load(*s),
            Op::AddrG(g) => E::AddrG(*g),
            Op::Frame(n) => E::Frame(*n),
            Op::Bin(b) => {
                let r = st.pop()?;
                let l = st.pop()?;
                E::Bin(*b, Box::new(l), Box::new(r))
            }
            Op::Un(u) => E::Un(*u, Box::new(st.pop()?)),
            Op::Sext(n) => E::Sext(*n, Box::new(st.pop()?)),
            _ => return None,
        };
        st.push(e);
    }
    (st.len() == 1).then(|| st.pop().unwrap())
}

fn ops(e: &E, out: &mut Vec<Op>) {
    match e {
        E::Push(c) => out.push(Op::Push(*c)),
        E::Load(s) => out.push(Op::Load(*s)),
        E::AddrG(g) => out.push(Op::AddrG(*g)),
        E::Frame(n) => out.push(Op::Frame(*n)),
        E::Bin(b, l, r) => {
            ops(l, out);
            ops(r, out);
            out.push(Op::Bin(*b));
        }
        E::Un(u, x) => {
            ops(x, out);
            out.push(Op::Un(*u));
        }
        E::Sext(n, x) => {
            ops(x, out);
            out.push(Op::Sext(*n));
        }
    }
}

pub(super) fn to_ops(e: &E) -> Vec<Op> {
    let mut v = Vec::new();
    ops(e, &mut v);
    v
}

/// Can evaluating it stop the program? (A division by anything but a
/// constant other than 0 and -1, or a checked operation.)
pub(super) fn fallible(e: &E) -> bool {
    match e {
        E::Bin(BinOp::AddOv | BinOp::SubOv | BinOp::MulOv | BinOp::ShlOv, ..) | E::Un(UnOp::NegOv, _) => true,
        E::Bin(BinOp::DivTrunc | BinOp::ModTrunc | BinOp::DivFloor | BinOp::ModFloor, l, r) => !matches!(**r, E::Push(d) if d != 0 && d != -1) || fallible(l),
        E::Bin(_, l, r) => fallible(l) || fallible(r),
        E::Un(_, x) | E::Sext(_, x) => fallible(x),
        _ => false,
    }
}

fn loads(e: &E, f: &mut dyn FnMut(u32)) {
    match e {
        E::Load(s) => f(*s),
        E::Bin(_, l, r) => {
            loads(l, f);
            loads(r, f);
        }
        E::Un(_, x) | E::Sext(_, x) => loads(x, f),
        _ => {}
    }
}

/// x cut to its low n bits, sign-extended (n = 64: x).
fn red(c: i64, n: u8) -> i64 {
    if n >= 64 {
        c
    } else {
        super::ir::sext(c, n)
    }
}

/// Is the node a ring operation in a context that keeps n bits? A Sext(m)
/// with m >= n is one: the low n bits pass through it unchanged.
fn is_ring(e: &E, n: u8) -> bool {
    match e {
        E::Bin(BinOp::Add | BinOp::Sub | BinOp::Mul, ..) => true,
        E::Bin(BinOp::Shl, _, r) => matches!(**r, E::Push(_)),
        E::Un(UnOp::Neg | UnOp::Bnot | UnOp::Plus, _) => true,
        E::Sext(m, _) => *m >= n,
        _ => false,
    }
}

fn ring_ops(e: &E, n: u8) -> usize {
    if !is_ring(e, n) {
        return 0;
    }
    match e {
        E::Bin(BinOp::Shl, l, _) => 1 + ring_ops(l, n),
        E::Bin(_, l, r) => 1 + ring_ops(l, n) + ring_ops(r, n),
        E::Sext(_, x) => 1 + ring_ops(x, n),
        E::Un(_, x) => 1 + ring_ops(x, n),
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// The algebraic form: a sum of terms, each a coefficient times factors

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
enum A {
    Atom(u32),
    Sum(Vec<(i64, Vec<A>)>),
}

/// One expression's ring part: its atoms and the width kept.
struct Region<'a> {
    atoms: Vec<E>,
    index: HashMap<E, u32>,
    n: u8,
    cx: &'a Cx,
    /// which atoms are loop-invariant (hoist can take them out)
    inv: Vec<bool>,
    /// each atom's operations, and when it is ready
    atom_ops: Vec<Vec<Op>>,
    atom_late: Vec<u32>,
    /// the whole run: its value goes into cx.target
    top: bool,
}

impl Region<'_> {
    fn atom(&mut self, e: E) -> A {
        if let Some(&i) = self.index.get(&e) {
            return A::Atom(i);
        }
        let i = self.atoms.len() as u32;
        self.inv.push(self.cx.invariant(&e));
        self.atom_ops.push(to_ops(&e));
        self.atom_late.push(self.cx.late(&e));
        self.index.insert(e.clone(), i);
        self.atoms.push(e);
        A::Atom(i)
    }

    /// The tree as a sum, its atoms simplified first. Every case is a ring
    /// identity mod 2^n (see the module's head).
    #[cfg(test)]
    fn read(&mut self, e: &E) -> A {
        let (a, _) = self.read_kept(e);
        canon(a, self.n)
    }

    /// The tree as a sum (not yet in canonical form), and the tree itself
    /// with only its atoms simplified (the shape kept).
    fn read_kept(&mut self, e: &E) -> (A, E) {
        let n = self.n;
        if !is_ring(e, n) {
            if let E::Push(c) = e {
                return (konst(red(*c, n)), e.clone());
            }
            let s = simplify(e, n, self.cx, false);
            return (self.atom(s.clone()), s);
        }
        let sum = |a: A, b: A, sign: i64| {
            let mut ts = terms(a);
            ts.extend(terms(b).into_iter().map(|(c, f)| (c.wrapping_mul(sign), f)));
            A::Sum(ts)
        };
        let prod = |a: A, b: A| A::Sum(vec![(1, vec![a, b])]);
        match e {
            E::Bin(b @ (BinOp::Add | BinOp::Sub | BinOp::Mul), l, r) => {
                let (la, le) = self.read_kept(l);
                let (ra, re) = self.read_kept(r);
                let a = match b {
                    BinOp::Add => sum(la, ra, 1),
                    BinOp::Sub => sum(la, ra, -1),
                    _ => prod(la, ra),
                };
                (a, E::Bin(*b, Box::new(le), Box::new(re)))
            }
            E::Bin(BinOp::Shl, l, r) => {
                let E::Push(k) = **r else { unreachable!() };
                let (la, le) = self.read_kept(l);
                // x << k = x * 2^(k mod 64), as the machine takes the count
                (prod(la, konst(red(1i64.wrapping_shl(k as u32 & 63), n))), E::Bin(BinOp::Shl, Box::new(le), r.clone()))
            }
            E::Un(u, x) => {
                let (xa, xe) = self.read_kept(x);
                let a = match u {
                    UnOp::Neg => prod(xa, konst(-1)),
                    // ~x = -x - 1
                    UnOp::Bnot => sum(prod(xa, konst(-1)), konst(-1), 1),
                    _ => xa,
                };
                (a, E::Un(*u, Box::new(xe)))
            }
            // the low n bits pass a Sext(m >= n) unchanged
            E::Sext(m, x) => {
                let (xa, xe) = self.read_kept(x);
                (xa, E::Sext(*m, Box::new(xe)))
            }
            _ => unreachable!(),
        }
    }
}

fn konst(c: i64) -> A {
    A::Sum(if c == 0 { vec![] } else { vec![(c, vec![])] })
}

fn terms(a: A) -> Vec<(i64, Vec<A>)> {
    match a {
        A::Sum(ts) => ts,
        x => vec![(1, vec![x])],
    }
}

/// The form kept for every state: zero terms gone, a factor that is one
/// term merged into its term (associativity), a term that is a sum with
/// coefficient 1 merged into its sum, factors and terms sorted
/// (commutativity), coefficients modulo 2^n.
fn canon(a: A, n: u8) -> A {
    let A::Sum(ts) = a else { return a };
    let mut out: Vec<(i64, Vec<A>)> = Vec::with_capacity(ts.len());
    for (c, fs) in ts {
        let mut c = red(c, n);
        let mut nf: Vec<A> = Vec::with_capacity(fs.len());
        for f in fs {
            match canon(f, n) {
                A::Sum(mut inner) if inner.len() == 1 => {
                    let (c2, f2) = inner.pop().unwrap();
                    c = red(c.wrapping_mul(c2), n);
                    nf.extend(f2);
                }
                // a factor that is zero
                A::Sum(inner) if inner.is_empty() => c = 0,
                x => nf.push(x),
            }
        }
        if c == 0 {
            continue;
        }
        nf.sort();
        if c == 1 && nf.len() == 1 && matches!(nf[0], A::Sum(_)) {
            let Some(A::Sum(inner)) = nf.pop() else { unreachable!() };
            out.extend(inner);
        } else {
            out.push((c, nf));
        }
    }
    out.sort();
    if out.len() == 1 && out[0].0 == 1 && out[0].1.len() == 1 {
        return out.pop().unwrap().1.pop().unwrap();
    }
    A::Sum(out)
}

/// A polynomial: sorted atoms -> coefficient.
type Poly = BTreeMap<Vec<u32>, i64>;

fn poly(a: &A, n: u8) -> Option<Poly> {
    match a {
        A::Atom(i) => Some(Poly::from([(vec![*i], 1)])),
        A::Sum(ts) => {
            let mut p = Poly::new();
            for (c, fs) in ts {
                let mut t = Poly::from([(vec![], *c)]);
                for f in fs {
                    let q = poly(f, n)?;
                    let mut r = Poly::new();
                    for (ma, ca) in &t {
                        for (mb, cb) in &q {
                            let mut m = ma.clone();
                            m.extend(mb);
                            m.sort();
                            let e = r.entry(m).or_insert(0);
                            *e = red(e.wrapping_add(ca.wrapping_mul(*cb)), n);
                        }
                    }
                    r.retain(|_, c| *c != 0);
                    if r.len() > MAX_TERMS {
                        return None;
                    }
                    t = r;
                }
                for (m, c) in t {
                    let e = p.entry(m).or_insert(0);
                    *e = red(e.wrapping_add(c), n);
                }
                p.retain(|_, c| *c != 0);
                if p.len() > MAX_TERMS {
                    return None;
                }
            }
            Some(p)
        }
    }
}

fn from_poly(p: &Poly, n: u8) -> A {
    canon(A::Sum(p.iter().map(|(m, c)| (*c, m.iter().map(|i| A::Atom(*i)).collect())).collect()), n)
}

// ---------------------------------------------------------------------------
// The rules: every move one rewrite at one sum of the state

fn local_moves(ts: &[(i64, Vec<A>)], _n: u8, out: &mut Vec<A>) {
    // collect like terms of this sum only: ca + da = (c + d)a
    {
        let mut m: BTreeMap<&Vec<A>, i64> = BTreeMap::new();
        for (c, fs) in ts {
            let e = m.entry(fs).or_insert(0);
            *e = e.wrapping_add(*c);
        }
        if m.len() < ts.len() {
            out.push(A::Sum(m.into_iter().map(|(f, c)| (c, f.clone())).collect()));
        }
    }
    // multiply out one factor: c * F * (g1 + g2) = c g1 F + c g2 F
    for (i, (c, fs)) in ts.iter().enumerate() {
        for (j, f) in fs.iter().enumerate() {
            let A::Sum(inner) = f else { continue };
            let rest: Vec<A> = fs.iter().enumerate().filter(|(k, _)| *k != j).map(|(_, x)| x.clone()).collect();
            let mut nt: Vec<(i64, Vec<A>)> = ts.iter().enumerate().filter(|(k, _)| *k != i).map(|(_, t)| t.clone()).collect();
            for (c2, f2) in inner {
                let mut g = rest.clone();
                g.extend(f2.iter().cloned());
                nt.push((c.wrapping_mul(*c2), g));
            }
            out.push(A::Sum(nt));
        }
    }
    // take out a factor two terms or more share: F a + F b = F (a + b)
    let mut shared: Vec<&A> = Vec::new();
    for (_, fs) in ts {
        for (j, f) in fs.iter().enumerate() {
            if fs[..j].contains(f) || shared.contains(&f) {
                continue;
            }
            if ts.iter().filter(|(_, g)| g.contains(f)).count() >= 2 {
                shared.push(f);
            }
        }
    }
    for f in shared {
        let mut inner = Vec::new();
        let mut rest = Vec::new();
        for (c, fs) in ts {
            match fs.iter().position(|g| g == f) {
                Some(p) => {
                    let mut g = fs.clone();
                    g.remove(p);
                    inner.push((*c, g));
                }
                None => rest.push((*c, fs.clone())),
            }
        }
        rest.push((1, vec![f.clone(), A::Sum(inner)]));
        out.push(A::Sum(rest));
    }
    // take out a constant: g a + 2g b = g (a + 2b), for 2, 4, 8 (an
    // address's scale) and the gcd of the coefficients
    let mut gs: Vec<i64> = vec![2, 4, 8];
    let g = ts.iter().filter(|(_, f)| !f.is_empty()).fold(0i64, |g, (c, _)| gcd(g, *c));
    if g > 1 && !gs.contains(&g) {
        gs.push(g);
    }
    for g in gs {
        let (inner, rest): (Vec<_>, Vec<_>) = ts.iter().cloned().partition(|(c, f)| !f.is_empty() && c % g == 0);
        if inner.len() < 2 {
            continue;
        }
        let mut nt = rest;
        nt.push((g, vec![A::Sum(inner.into_iter().map(|(c, f)| (c / g, f)).collect())]));
        out.push(A::Sum(nt));
    }
    // split a constant: (2^k +- 1) x = 2^k x +- x
    for (i, (c, fs)) in ts.iter().enumerate() {
        if fs.len() != 1 || !matches!(fs[0], A::Atom(_)) || *c < 3 {
            continue;
        }
        for (p, s) in [(c - 1, 1), (c + 1, -1)] {
            if p > 0 && p & (p - 1) == 0 {
                let mut nt: Vec<(i64, Vec<A>)> = ts.iter().enumerate().filter(|(k, _)| *k != i).map(|(_, t)| t.clone()).collect();
                nt.push((p, fs.clone()));
                nt.push((s, fs.clone()));
                out.push(A::Sum(nt));
            }
        }
    }
}

fn gcd(a: i64, b: i64) -> i64 {
    let (mut a, mut b) = (a.unsigned_abs(), b.unsigned_abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    i64::try_from(a).unwrap_or(1)
}

/// Every state one move away: a local move at the root, or at any sum
/// inside it.
fn moves(a: &A, n: u8, limit: usize) -> Vec<A> {
    let A::Sum(ts) = a else { return vec![] };
    let mut out = Vec::new();
    local_moves(ts, n, &mut out);
    out.truncate(limit);
    'all: for (i, (c, fs)) in ts.iter().enumerate() {
        for (j, f) in fs.iter().enumerate() {
            if out.len() >= limit {
                break 'all;
            }
            for m in moves(f, n, limit - out.len()) {
                let mut nt = ts.clone();
                nt[i] = (*c, {
                    let mut g = fs.clone();
                    g[j] = m;
                    g
                });
                out.push(A::Sum(nt));
            }
        }
    }
    out.into_iter().map(|x| canon(x, n)).filter(|x| x != a).collect()
}

// ---------------------------------------------------------------------------
// Back to a tree, in the order x64opt.rs makes the best code of

impl Region<'_> {
    fn invariant(&self, a: &A) -> bool {
        match a {
            A::Atom(i) => self.inv[*i as usize],
            A::Sum(ts) => ts.iter().all(|(_, f)| f.iter().all(|x| self.invariant(x))),
        }
    }
    fn hoisting(&self) -> bool {
        self.cx.stored.is_some()
    }

    /// The whole run's form: `t = rest + t`, t read last, which
    /// x64opt works out in t's own register (`add t, rax`).
    fn emit_root(&self, a: &A) -> Vec<Op> {
        if let (true, Some(t), A::Sum(ts)) = (self.top, self.cx.target, a) {
            let own = ts.iter().position(|(c, f)| *c == 1 && f.len() == 1 && matches!(f[0], A::Atom(k) if self.atoms[k as usize] == E::Load(t)));
            if let Some(p) = own.filter(|_| ts.len() >= 2) {
                let mut rest = ts.clone();
                rest.remove(p);
                return bin(BinOp::Add, self.emit_sum(&rest), (vec![Op::Load(t)], 0)).0;
            }
        }
        self.emit(a).0
    }

    fn emit(&self, a: &A) -> P {
        match a {
            A::Atom(i) => (self.atom_ops[*i as usize].clone(), self.atom_late[*i as usize]),
            A::Sum(ts) => self.emit_sum(ts),
        }
    }

    /// A sum: the invariant terms together first (one expression hoist can
    /// take out of the loop); then what is ready first (a value just worked
    /// out is added last, so the chain through it is short), the larger
    /// first, what is added before what is subtracted; the constant after
    /// what is ready (a local plus a constant is a lea).
    fn emit_sum(&self, ts: &[(i64, Vec<A>)]) -> P {
        let refs: Vec<(i64, &[A])> = ts.iter().map(|(c, f)| (*c, f.as_slice())).collect();
        self.emit_terms(&refs)
    }

    fn emit_terms(&self, ts: &[(i64, &[A])]) -> P {
        let konst: i64 = ts.iter().filter(|(_, f)| f.is_empty()).fold(0i64, |s, (c, _)| s.wrapping_add(*c));
        let mut rest: Vec<(i64, &[A])> = ts.iter().filter(|(_, f)| !f.is_empty()).copied().collect();
        let mut items: Vec<(bool, P)> = Vec::new();
        if self.hoisting() && rest.len() >= 2 {
            let (inv, var): (Vec<_>, Vec<_>) = rest.iter().copied().partition(|(_, f)| f.iter().all(|x| self.invariant(x)));
            let worth = inv.len() >= 2 || inv.iter().any(|(c, f)| *c != 1 && *c != -1 || f.len() >= 2 || matches!(f[0], A::Sum(_)));
            if !var.is_empty() && worth && !inv.is_empty() {
                let neg = inv.iter().all(|(c, _)| negative(*c));
                let group: Vec<(i64, &[A])> = inv.into_iter().map(|(c, f)| (if neg { c.wrapping_neg() } else { c }, f)).collect();
                items.push((neg, if group.len() == 1 { self.emit_term(group[0].0, &group[0].1.iter().collect::<Vec<_>>()) } else { self.emit_terms(&group) }));
                rest = var;
            }
        }
        for (c, f) in &rest {
            let f: Vec<&A> = f.iter().collect();
            if negative(*c) {
                items.push((true, self.emit_term(c.wrapping_neg(), &f)));
            } else {
                items.push((false, self.emit_term(*c, &f)));
            }
        }
        // the order is otherwise kept (stable)
        items.sort_by_key(|(neg, p)| (p.1, *neg, std::cmp::Reverse(p.0.len())));
        // something added goes first: `-a + b` would need a neg
        if let Some(p) = items.iter().position(|(neg, _)| !*neg).filter(|&p| p > 0) {
            let x = items.remove(p);
            items.insert(0, x);
        }
        let early = items.iter().filter(|(_, p)| p.1 == 0).count();
        if items.first().is_some_and(|(neg, _)| *neg) && items.len() == rest.len() {
            // nothing added: -(a + b + ...)
            let all: Vec<(i64, &[A])> = rest.iter().map(|(c, f)| (c.wrapping_neg(), *f)).collect();
            let e = un(UnOp::Neg, self.emit_terms(&all));
            return if konst != 0 { bin(BinOp::Add, e, (vec![Op::Push(konst)], 0)) } else { e };
        }
        let mut konst = konst;
        let mut acc: Option<P> = None;
        for (k, (neg, p)) in items.into_iter().enumerate() {
            if k == early && k > 0 && konst != 0 {
                acc = acc.map(|x| bin(BinOp::Add, x, (vec![Op::Push(konst)], 0)));
                konst = 0;
            }
            acc = Some(match acc {
                None if neg => un(UnOp::Neg, p),
                None => p,
                Some(x) => bin(if neg { BinOp::Sub } else { BinOp::Add }, x, p),
            });
        }
        match acc {
            None => (vec![Op::Push(konst)], 0),
            Some(x) if konst == 0 => x,
            Some(x) => bin(BinOp::Add, x, (vec![Op::Push(konst)], 0)),
        }
    }

    /// c times the factors: the invariant ones together first (with the
    /// constant: (8m) i, where hoist takes 8m out of the loop), then what is
    /// ready first, the larger first; the constant as a shift where it is a
    /// power of two.
    fn emit_term(&self, c: i64, fs: &[&A]) -> P {
        if fs.is_empty() {
            return (vec![Op::Push(c)], 0);
        }
        let mut items: Vec<P> = Vec::new();
        let mut rest: Vec<&A> = fs.to_vec();
        let mut c = c;
        if self.hoisting() && fs.len() >= 2 {
            let (inv, var): (Vec<&A>, Vec<&A>) = rest.iter().partition(|x| self.invariant(x));
            if !inv.is_empty() && !var.is_empty() && (inv.len() >= 2 || (c != 1 && c != -1)) {
                items.push(self.emit_term(c, &inv));
                c = 1;
                rest = var;
            }
        }
        let mut more: Vec<P> = rest.iter().map(|x| self.emit(x)).collect();
        more.sort_by_key(|p| (p.1, std::cmp::Reverse(p.0.len())));
        items.extend(more);
        let mut acc = items.remove(0);
        for p in items {
            acc = bin(BinOp::Mul, acc, p);
        }
        if c == 1 {
            acc
        } else if c == -1 {
            un(UnOp::Neg, acc)
        } else if c > 0 && c & (c - 1) == 0 {
            bin(BinOp::Shl, acc, (vec![Op::Push(c.trailing_zeros() as i64)], 0))
        } else if matches!(c, 3 | 5 | 9) && matches!(acc.0[..], [Op::Load(_)]) && acc.1 == 0 {
            let x = acc.clone();
            bin(BinOp::Add, bin(BinOp::Shl, acc, (vec![Op::Push((c - 1).trailing_zeros() as i64)], 0)), x)
        } else {
            // (2^k + 1) x as one lea [x + x * 2^k] is the split rule's to
            // offer: always taking it was measured slower on a Zen 4, where
            // a scaled lea takes 2 cycles and imul's port is otherwise free
            bin(BinOp::Mul, acc, (vec![Op::Push(c)], 0))
        }
    }
}

/// A piece of emitted code: its operations, and when the latest local it
/// reads is ready.
type P = (Vec<Op>, u32);

fn bin(b: BinOp, l: P, r: P) -> P {
    let (mut ops, late) = l;
    ops.extend(r.0);
    ops.push(Op::Bin(b));
    (ops, late.max(r.1))
}

fn un(u: UnOp, x: P) -> P {
    let (mut ops, late) = x;
    ops.push(Op::Un(u));
    (ops, late)
}

/// Subtracted rather than added (the lowest number is its own negation:
/// it is added).
fn negative(c: i64) -> bool {
    c < 0 && c != i64::MIN
}


// ---------------------------------------------------------------------------
// The search

/// The tree with only its atoms' insides simplified (the shape kept).
fn keep(e: &E, n: u8, cx: &Cx) -> E {
    if !is_ring(e, n) {
        return simplify(e, n, cx, false);
    }
    match e {
        E::Bin(BinOp::Shl, l, r) => E::Bin(BinOp::Shl, Box::new(keep(l, n, cx)), r.clone()),
        E::Bin(b, l, r) => E::Bin(*b, Box::new(keep(l, n, cx)), Box::new(keep(r, n, cx))),
        E::Un(u, x) => E::Un(*u, Box::new(keep(x, n, cx))),
        E::Sext(m, x) => E::Sext(*m, Box::new(keep(x, n, cx))),
        _ => e.clone(),
    }
}

/// The tree simplified, in a context that keeps its low n bits.
/// `top`: the whole run (or what is under its last cut), whose value goes
/// into `cx.target`.
fn simplify(e: &E, n: u8, cx: &Cx, top: bool) -> E {
    if is_ring(e, n) {
        return region(e, n, cx, top);
    }
    match e {
        // a narrower cut: what is under it keeps m bits
        E::Sext(m, x) => E::Sext(*m, Box::new(simplify(x, *m, cx, top))),
        // an atom: its operands are exact numbers again
        E::Bin(b, l, r) => E::Bin(*b, Box::new(simplify(l, 64, cx, false)), Box::new(simplify(r, 64, cx, false))),
        E::Un(u, x) => E::Un(*u, Box::new(simplify(x, 64, cx, false))),
        _ => e.clone(),
    }
}

/// A ring expression: Nuome's beam over rewrite paths, judged by `cost`.
fn region(e: &E, n: u8, cx: &Cx, top: bool) -> E {
    let by_constant = |e: &E| matches!(e, E::Bin(BinOp::Mul, l, r) if matches!(**l, E::Push(_)) || matches!(**r, E::Push(_)));
    if (ring_ops(e, n) < 2 && !by_constant(e)) || fallible(e) {
        return keep(e, n, cx);
    }
    let mut r = Region { atoms: Vec::new(), index: HashMap::new(), n, cx, inv: Vec::new(), atom_ops: Vec::new(), atom_late: Vec::new(), top };
    let (start, kept) = r.read_kept(e);
    let start = canon(start, n);
    let judge = |a: &A| cx.cost(&r.emit_root(a), top);
    let mut best = (cx.cost(&to_ops(&kept), top), None::<A>);
    let mut seen: HashSet<A> = HashSet::new();
    let mut beam: Vec<(f64, A)> = Vec::new();
    let mut starts = vec![start.clone()];
    if let Some(p) = poly(&start, n) {
        starts.push(from_poly(&p, n));
    }
    for s in starts {
        if seen.insert(s.clone()) {
            let c = judge(&s);
            beam.push((c, s));
        }
    }
    let mut judged = beam.len();
    // a larger expression is judged in fewer states (compile time stays
    // small)
    let budget = (BUDGET * 16 / to_ops(e).len().max(16)).max(12);
    let mut stale = 0;
    for _ in 0..STEPS {
        for (c, s) in &beam {
            if *c < best.0 {
                best = (*c, Some(s.clone()));
            }
        }
        let mut next: Vec<(f64, A)> = Vec::new();
        'states: for (_, s) in &beam {
            for m in moves(s, n, MOVES) {
                if judged >= budget {
                    break 'states;
                }
                if seen.insert(m.clone()) {
                    judged += 1;
                    let c = judge(&m);
                    next.push((c, m));
                }
            }
        }
        if next.is_empty() {
            break;
        }
        // ties keep the order the moves came in: the result is deterministic
        next.sort_by(|a, b| a.0.total_cmp(&b.0));
        next.truncate(WIDTH);
        // two rounds with nothing better than the best so far: done
        if next[0].0 >= best.0 {
            stale += 1;
            if stale >= 2 {
                break;
            }
        } else {
            stale = 0;
        }
        beam = next;
    }
    for (c, s) in &beam {
        if *c < best.0 {
            best = (*c, Some(s.clone()));
        }
    }
    match best.1 {
        Some(a) => tree(&r.emit_root(&a)).expect("the emitted code is one value"),
        None => kept,
    }
}

// ---------------------------------------------------------------------------
// The judge: what x64opt.rs makes of a run, instructions plus latency

/// Where the expression is: which locals the loop around it stores (None:
/// not in a loop hoist takes code out of), and how much a cost there weighs.
struct Cx {
    stored: Option<HashSet<u32>>,
    weight: f64,
    outer: f64,
    /// the value is an address (LoadM, StoreM, OpM use it as one)
    address: bool,
    /// which locals are taken to live in registers
    regs: std::rc::Rc<Vec<bool>>,
    /// the local the value is stored into
    target: Option<u32>,
    /// locals stored just before, in the same straight code: when their
    /// values are ready, in cycles
    ready: HashMap<u32, f64>,
}

#[derive(Clone, Copy, PartialEq)]
enum K {
    Imm(i64),
    /// a local, in a register or not
    Slot(u32, bool),
    Lea,
    Scaled(u8),
    Index,
    Rax,
    Stack,
    Flags,
}

/// The cost of moving a value into a register: instructions, latency.
fn mat(k: K) -> (f64, f64) {
    match k {
        K::Rax => (0.0, 0.0),
        K::Imm(_) | K::Slot(_, true) => (1.0, 0.0),
        K::Slot(_, false) => (1.0, 4.0),
        // a lea of base + displacement takes 1 cycle; with a scaled index,
        // 2 (Zen 4, and most cores of three parts)
        K::Lea | K::Scaled(0) => (1.0, 1.0),
        K::Scaled(_) | K::Index => (1.0, 2.0),
        // a push and a pop: the value goes round memory
        K::Stack => (1.0, 4.0),
        K::Flags => (2.0, 1.0),
    }
}

fn is_lazy(k: K) -> bool {
    matches!(k, K::Lea | K::Scaled(_) | K::Index)
}

/// x64opt.rs's compile-time stack, followed over a pure run; every local is
/// taken to be in a register.
fn sim(run: &[Op], address: bool, regs: &[bool], target: Option<u32>, ready: &HashMap<u32, f64>) -> f64 {
    let (ins, lat) = sim2(run, address, regs, target, ready);
    ins + lat
}

/// sim's instructions and latency apart.
fn sim2(run: &[Op], address: bool, regs: &[bool], target: Option<u32>, ready: &HashMap<u32, f64>) -> (f64, f64) {
    let reg = |s: u32| regs.get(s as usize).copied().unwrap_or(true);
    let target = target.filter(|t| reg(*t));
    // x64opt's bin_into: the last operation (under its cut) worked out in
    // the target's register
    let into_at = match run.last() {
        Some(Op::Sext(_)) => run.len().checked_sub(2),
        _ => run.len().checked_sub(1),
    };
    let mut v: Vec<(K, f64)> = Vec::new();
    let mut ins = 0.0;
    let need_rax = |v: &mut Vec<(K, f64)>, ins: &mut f64| {
        if let Some(i) = v.iter().rposition(|x| x.0 == K::Rax) {
            for x in v[..=i].iter_mut() {
                if x.0 != K::Stack {
                    *ins += match x.0 {
                        K::Flags => 3.0,
                        k if is_lazy(k) => 2.0,
                        _ => 1.0,
                    };
                    x.0 = K::Stack;
                }
            }
        }
    };
    let flags_top = |v: &mut Vec<(K, f64)>, ins: &mut f64| {
        if let Some(&(K::Flags, t)) = v.last() {
            v.pop();
            need_rax(v, ins);
            *ins += 2.0;
            v.push((K::Rax, t + 1.0));
        }
    };
    for (at, op) in run.iter().enumerate() {
        if let (Some(t), true, Op::Bin(b @ (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Band | BinOp::Bor | BinOp::Bxor))) = (target, Some(at) == into_at, op) {
            let n = v.len();
            if n >= 2 {
                let (mut l, mut r) = (v[n - 2], v[n - 1]);
                let own = |x: (K, f64)| matches!(x.0, K::Slot(s, _) if s == t);
                if own(r) && !own(l) && *b != BinOp::Sub {
                    std::mem::swap(&mut l, &mut r);
                }
                let fits = |x: (K, f64)| matches!(x.0, K::Imm(c) if i32::try_from(c).is_ok()) || matches!(x.0, K::Slot(..) | K::Rax);
                if fits(l) && fits(r) && !(own(r) && !own(l)) {
                    let (li, ll) = if own(l) { (0.0, 0.0) } else { mat(l.0) };
                    let (i, lat) = match (b, r.0) {
                        (BinOp::Add | BinOp::Sub | BinOp::Bor | BinOp::Bxor, K::Imm(0)) => (0.0, 0.0),
                        (BinOp::Mul, _) => (1.0, 3.0),
                        _ => (1.0, 1.0),
                    };
                    let cut = if at + 1 < run.len() { 1.0 } else { 0.0 };
                    return (ins + li + i + cut, (l.1 + ll).max(r.1) + lat + cut);
                }
            }
        }
        match op {
            Op::Push(c) => {
                flags_top(&mut v, &mut ins);
                v.push((K::Imm(*c), 0.0));
            }
            Op::Load(_) => {
                flags_top(&mut v, &mut ins);
                let Op::Load(s) = op else { unreachable!() };
                // a slot past the function's (a hoisted value) is in a register
                v.push((K::Slot(*s, reg(*s)), ready.get(s).copied().unwrap_or(0.0)));
            }
            Op::AddrG(_) | Op::Frame(_) => {
                flags_top(&mut v, &mut ins);
                need_rax(&mut v, &mut ins);
                ins += 1.0;
                v.push((K::Rax, 1.0));
            }
            Op::Sext(_) => {
                let (k, t) = v.pop().unwrap_or((K::Rax, 0.0));
                if let K::Imm(c) = k {
                    v.push((K::Imm(c), t));
                    continue;
                }
                need_rax(&mut v, &mut ins);
                let (i, l) = mat(k);
                ins += i + 1.0;
                v.push((K::Rax, t + l + 1.0));
            }
            Op::Un(u) => {
                let (k, t) = v.pop().unwrap_or((K::Rax, 0.0));
                match u {
                    UnOp::Plus => v.push((k, t)),
                    UnOp::Not => {
                        if k == K::Flags {
                            v.push((K::Flags, t));
                        } else {
                            need_rax(&mut v, &mut ins);
                            let (i, l) = mat(k);
                            ins += i + 1.0;
                            v.push((K::Flags, t + l + 1.0));
                        }
                    }
                    _ => {
                        need_rax(&mut v, &mut ins);
                        let (i, l) = mat(k);
                        let extra = if *u == UnOp::NegOv { 3.0 } else { 1.0 };
                        ins += i + extra;
                        v.push((K::Rax, t + l + 1.0));
                    }
                }
            }
            Op::Bin(b) => {
                let n = v.len();
                if n < 2 {
                    return (f64::INFINITY, 0.0);
                }
                let (l, r) = (v[n - 2], v[n - 1]);
                let fits = |c: i64| i32::try_from(c).is_ok();
                // x64opt's lazy forms: a local plus a constant, a local
                // shifted by 0 to 3, base + index * scale + displacement
                let lazy = match (b, l.0, r.0) {
                    (BinOp::Add | BinOp::Sub, K::Slot(_, true) | K::Lea, K::Imm(c)) if fits(c) => Some(K::Lea),
                    (BinOp::Shl, K::Slot(_, true) | K::Lea, K::Imm(k @ 0..=3)) => Some(K::Scaled(k as u8)),
                    (BinOp::Add, K::Slot(_, true) | K::Lea, K::Scaled(_)) | (BinOp::Add, K::Scaled(_), K::Slot(_, true) | K::Lea) => Some(K::Index),
                    (BinOp::Add, K::Slot(a, true), K::Slot(b, true)) if a != b => Some(K::Index),
                    (BinOp::Add | BinOp::Sub, K::Index, K::Imm(c)) if fits(c) => Some(K::Index),
                    _ => None,
                };
                if let Some(k) = lazy {
                    v.truncate(n - 2);
                    v.push((k, l.1.max(r.1)));
                    continue;
                }
                let mut right = v.pop().unwrap();
                let mut left = v.pop().unwrap();
                if right.0 == K::Rax && left.0 != K::Rax && matches!(b, BinOp::Add | BinOp::Mul | BinOp::Band | BinOp::Bor | BinOp::Bxor) {
                    std::mem::swap(&mut left, &mut right);
                }
                need_rax(&mut v, &mut ins);
                // the right operand: an immediate or a register stays, else rcx
                let (ri, rl) = match right.0 {
                    K::Imm(c) if fits(c) => (0.0, 0.0),
                    K::Slot(_, true) => (0.0, 0.0),
                    // a memory operand: no instruction, the load's wait
                    K::Slot(_, false) => (0.0, 4.0),
                    k => mat(k),
                };
                ins += ri;
                let tr = right.1 + rl;
                if b.is_comparison() {
                    let (li, ll) = if matches!(left.0, K::Slot(_, true)) { (0.0, 0.0) } else { mat(left.0) };
                    ins += li + 1.0;
                    v.push((K::Flags, (left.1 + ll).max(tr) + 1.0));
                    continue;
                }
                let (li, ll) = mat(left.0);
                ins += li;
                let tl = left.1 + ll;
                let by_imm = matches!(right.0, K::Imm(_));
                let zero = right.0 == K::Imm(0);
                let (i, lat) = match b {
                    BinOp::Add | BinOp::Sub | BinOp::Bor | BinOp::Bxor if zero => (0.0, 0.0),
                    BinOp::Add | BinOp::Sub | BinOp::Band | BinOp::Bor | BinOp::Bxor => (1.0, 1.0),
                    BinOp::Mul => (1.0, 3.0),
                    BinOp::Shl | BinOp::Shr if by_imm => (1.0, 1.0),
                    BinOp::Shl | BinOp::Shr => (2.0, 1.0),
                    BinOp::AddOv | BinOp::SubOv | BinOp::MulOv | BinOp::ShlOv => (4.0, 3.0),
                    BinOp::DivTrunc | BinOp::ModTrunc | BinOp::DivFloor | BinOp::ModFloor if by_imm => (7.0, 9.0),
                    _ => (10.0, 25.0),
                };
                ins += i;
                v.push((K::Rax, tl.max(tr) + lat));
            }
            _ => return (f64::INFINITY, 0.0),
        }
    }
    let Some(&(k, t)) = v.last() else { return (0.0, 0.0) };
    // the value where it goes: a lazy one is one lea, or free in an address;
    // into a local's register, one mov or lea
    let fin = if target.is_some() {
        match k {
            K::Rax | K::Slot(..) | K::Imm(_) => 1.0,
            k if is_lazy(k) => 2.0,
            _ => 0.0,
        }
    } else if is_lazy(k) && !address {
        2.0
    } else {
        0.0
    };
    (ins + fin, t + fin.min(1.0))
}

impl Cx {
    /// When the latest local the tree reads is ready (0: from the start).
    fn late(&self, e: &E) -> u32 {
        let mut t = 0.0f64;
        loads(e, &mut |s| t = t.max(self.ready.get(&s).copied().unwrap_or(0.0)));
        // a cycle or less (a counter's + 1) is as good as ready
        if t < 2.0 {
            0
        } else {
            t as u32
        }
    }

    fn invariant(&self, e: &E) -> bool {
        let Some(stored) = &self.stored else { return false };
        let mut ok = !fallible(e);
        loads(e, &mut |s| ok &= !stored.contains(&s));
        ok
    }

    /// The run's cost where it is: inside a loop hoist can take code out
    /// of, every largest invariant part (made of locals the loop never
    /// stores, unable to fail, holding a local or an address) is paid once,
    /// before the loop, and read from a register inside it.
    fn cost(&self, run: &[Op], top: bool) -> f64 {
        let target = if top { self.target } else { None };
        let Some(stored) = &self.stored else { return self.weight * sim(run, self.address, &self.regs, target, &self.ready) };
        // per operation: where its value's run starts, and whether that run
        // is one hoist takes (invariant, holding a local or an address,
        // unable to fail), followed in one pass over the stack
        let mut st: Vec<(usize, bool, bool)> = Vec::with_capacity(8);
        let mut whole: Vec<(usize, bool)> = Vec::with_capacity(run.len());
        for (i, op) in run.iter().enumerate() {
            let (s, inv, has) = match op {
                Op::Load(x) => (i, !stored.contains(x), true),
                Op::AddrG(_) | Op::Frame(_) => (i, true, true),
                Op::Push(_) => (i, true, false),
                Op::Un(u) => {
                    let Some((s, inv, has)) = st.pop() else { return f64::INFINITY };
                    (s, inv && *u != UnOp::NegOv, has)
                }
                Op::Sext(_) => match st.pop() {
                    Some(x) => x,
                    None => return f64::INFINITY,
                },
                Op::Bin(b) => {
                    let (Some((_, ri, rh)), Some((s, li, lh))) = (st.pop(), st.pop()) else { return f64::INFINITY };
                    let ok = match b {
                        BinOp::AddOv | BinOp::SubOv | BinOp::MulOv | BinOp::ShlOv => false,
                        BinOp::DivTrunc | BinOp::ModTrunc | BinOp::DivFloor | BinOp::ModFloor => matches!(run[i - 1], Op::Push(d) if d != 0 && d != -1),
                        _ => true,
                    };
                    (s, li && ri && ok, lh || rh)
                }
                _ => return f64::INFINITY,
            };
            st.push((s, inv, has));
            let root = matches!(op, Op::Bin(_) | Op::Un(_) | Op::Sext(_) | Op::AddrG(_) | Op::Frame(_));
            whole.push((s, root && inv && has));
        }
        let mut inside: Vec<Op> = Vec::with_capacity(run.len());
        let mut outside = 0.0;
        let mut i = run.len();
        // from the end: the outermost invariant expression first
        let mut parts: Vec<(usize, usize)> = Vec::new();
        while i > 0 {
            i -= 1;
            let (s, take) = whole[i];
            if take {
                parts.push((s, i));
                i = s;
            }
        }
        parts.reverse();
        let mut at = 0;
        for (s, e) in parts {
            inside.extend_from_slice(&run[at..s]);
            inside.push(Op::Load(u32::MAX));
            outside += sim(&run[s..=e], false, &self.regs, None, &HashMap::new());
            at = e + 1;
        }
        inside.extend_from_slice(&run[at..]);
        self.weight * sim(&inside, self.address, &self.regs, target, &self.ready) + self.outer * outside
    }
}

// ---------------------------------------------------------------------------
// The guard: the old and the new run on the same inputs

/// The run's value with these inputs, by the machine's rules (None: it
/// stops, a division by zero or a checked operation that does not fit).
#[cfg(test)]
pub(super) fn eval_ops(run: &[Op], input: &dyn Fn(u8, u32) -> i64) -> Option<i64> {
    eval(run, input)
}

fn eval(run: &[Op], input: &dyn Fn(u8, u32) -> i64) -> Option<i64> {
    let mut st: Vec<i64> = Vec::new();
    for op in run {
        let v = match op {
            Op::Push(c) => *c,
            Op::Load(s) => input(0, *s),
            Op::AddrG(g) => input(1, *g),
            Op::Frame(n) => input(2, *n),
            Op::Sext(n) => super::ir::sext(st.pop()?, *n),
            Op::Un(u) => {
                let a = st.pop()?;
                match u {
                    UnOp::Neg => a.wrapping_neg(),
                    UnOp::Plus => a,
                    UnOp::Not => (a == 0) as i64,
                    UnOp::Bnot => !a,
                    UnOp::NegOv => a.checked_neg()?,
                    u => u.float_eval(a)?,
                }
            }
            Op::Bin(b) => {
                let r = st.pop()?;
                let l = st.pop()?;
                value(*b, l, r)?
            }
            _ => return None,
        };
        st.push(v);
    }
    st.pop()
}

fn value(op: BinOp, a: i64, b: i64) -> Option<i64> {
    use BinOp::*;
    if let Some(r) = op.float_eval(a, b) {
        return Some(r);
    }
    if let Some(r) = op.checked_eval(a, b) {
        return r;
    }
    let div_ok = b != 0 && !(a == i64::MIN && b == -1);
    Some(match op {
        Add => a.wrapping_add(b),
        Sub => a.wrapping_sub(b),
        Mul => a.wrapping_mul(b),
        DivTrunc if div_ok => a / b,
        ModTrunc if div_ok => a % b,
        DivFloor if div_ok => {
            let r = a % b;
            a / b - (r != 0 && (r ^ b) < 0) as i64
        }
        ModFloor if div_ok => {
            let r = a % b;
            if r != 0 && (r ^ b) < 0 {
                r + b
            } else {
                r
            }
        }
        Eq => (a == b) as i64,
        Ne => (a != b) as i64,
        Lt => (a < b) as i64,
        Le => (a <= b) as i64,
        Gt => (a > b) as i64,
        Ge => (a >= b) as i64,
        Ult => ((a as u64) < b as u64) as i64,
        Ule => ((a as u64) <= b as u64) as i64,
        Band => a & b,
        Bor => a | b,
        Bxor => a ^ b,
        Shl => a.wrapping_shl(b as u32 & 63),
        Shr => a.wrapping_shr(b as u32 & 63),
        _ => return None,
    })
}

const EDGES: [i64; 12] = [0, 1, -1, 2, -2, i64::MIN, i64::MAX, i32::MIN as i64, i32::MAX as i64, 1 << 32, (1 << 31) + 1, 0x7FFF_FFFF_FFFF_FFFE];

/// Do the two runs give the same value (or both stop) on 200 sets of
/// inputs? The first sets are edge values, the rest random.
fn same(a: &[Op], b: &[Op]) -> bool {
    same_masked(a, b, &|_| -1)
}

/// `same`, every local's value taken through its mask (a local known to be
/// even is only given even values).
pub(super) fn same_masked(a: &[Op], b: &[Op], mask: &dyn Fn(u32) -> i64) -> bool {
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    for k in 0..200u64 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let s = seed;
        let input = |kind: u8, id: u32| -> i64 {
            let h = (s ^ (kind as u64) << 40 ^ (id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)).wrapping_mul(0xD6E8_FEB8_6659_FD93);
            let h = h ^ h >> 29;
            let m = if kind == 0 { mask(id) } else { -1 };
            m & match k % 3 {
                // the same edge value everywhere, then mixed edge values
                0 if k < 36 => EDGES[(k / 3) as usize],
                0 | 1 => EDGES[(h % EDGES.len() as u64) as usize],
                _ if h & 1 == 0 => (h >> 1) as i64 >> (h % 60),
                _ => h as i64,
            }
        };
        if eval(a, &input) != eval(b, &input) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// The pass

/// What pops the value each operation leaves, and in which place from the
/// top: (the consumer, its operand) per operation, within straight code.
pub(super) fn consumers(code: &[Op]) -> Vec<Option<(usize, usize)>> {
    let mut used = vec![None; code.len()];
    let mut st: Vec<Option<usize>> = Vec::new();
    for (i, op) in code.iter().enumerate() {
        let (pops, pushes) = match op {
            Op::Push(_) | Op::Load(_) | Op::AddrG(_) | Op::Frame(_) | Op::LoadG(_) | Op::ReadLine | Op::Data(_) => (0, 1),
            Op::Dup => (1, 2),
            Op::Store(_) | Op::StoreG(_) | Op::Pop | Op::PrintInt | Op::Jz(_) | Op::Jnz(_) | Op::Ret => (1, 0),
            Op::Bin(_) => (2, 1),
            Op::Un(_) | Op::Sext(_) | Op::LoadM(_) => (1, 1),
            Op::StoreM(_) | Op::OpM(..) | Op::PrintMem => (2, 0),
            Op::Call(_, n) => (*n as usize, 1),
            Op::ReadToken => (0, 2),
            Op::PrintStr(_) => (0, 0),
            Op::Label(_) | Op::Jmp(_) | Op::Fail(..) | Op::FailMem(_) => {
                st.iter_mut().for_each(|x| *x = None);
                (0, 0)
            }
        };
        for p in 0..pops {
            if let Some(Some(j)) = st.pop() {
                used[j] = Some((i, p));
            }
        }
        for _ in 0..pushes {
            st.push(Some(i));
        }
    }
    used
}

/// The loops hoist works on, innermost first: (top, back jump).
fn hoistable_loops(code: &[Op]) -> Vec<(usize, usize)> {
    let mut label_at = vec![usize::MAX; label_count(code) as usize];
    for (i, op) in code.iter().enumerate() {
        if let Op::Label(l) = op {
            label_at[*l as usize] = i;
        }
    }
    let mut loops = Vec::new();
    for (j, op) in code.iter().enumerate() {
        let Op::Jmp(l) = op else { continue };
        let t = label_at[*l as usize];
        if t >= j || t == 0 || matches!(code[t - 1], Op::Jmp(_) | Op::Ret) {
            continue;
        }
        let outside = code.iter().enumerate().any(|(k, o)| matches!(o, Op::Jmp(m) | Op::Jz(m) | Op::Jnz(m) if m == l) && (k < t || k > j));
        if !outside {
            loops.push((t, j));
        }
    }
    loops.sort_by_key(|&(t, j)| j - t);
    loops
}

/// Which locals x64opt.rs will likely keep in registers: the seven used
/// most, a use in a loop weighing `loop_weight` per level (as its
/// HotMotor judge weighs them).
fn registers(f: &Func, loop_weight: f64) -> Vec<bool> {
    let at = loop_depths(&f.code);
    let mut w = vec![0.0f64; f.slots as usize];
    for (i, op) in f.code.iter().enumerate() {
        if let Op::Load(s) | Op::Store(s) = op {
            w[*s as usize] += loop_weight.powi(at[i].min(6) as i32);
        }
    }
    let mut order: Vec<usize> = (0..w.len()).collect();
    order.sort_by(|&a, &b| w[b].total_cmp(&w[a]).then(a.cmp(&b)));
    let mut regs = vec![false; w.len()];
    for &s in order.iter().take(7) {
        // saving a register costs about two memory operations
        regs[s] = w[s] > 2.0;
    }
    regs
}

/// Simplifies every expression of the function where that makes cheaper
/// code; returns how many expressions changed.
pub fn run(f: &mut Func, loop_weight: f64) -> usize {
    let code = &f.code;
    let st = super::opt::starts(code);
    // the largest expressions: each one not inside another
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for i in 0..code.len() {
        let Some(s) = st[i] else { continue };
        if !matches!(code[i], Op::Bin(_) | Op::Un(_) | Op::Sext(_)) {
            continue;
        }
        while runs.last().is_some_and(|&(rs, _)| rs >= s) {
            runs.pop();
        }
        runs.push((s, i));
    }
    if runs.is_empty() {
        return 0;
    }
    let depths = loop_depths(code);
    let loops = hoistable_loops(code);
    let used = consumers(code);
    let regs = std::rc::Rc::new(registers(f, loop_weight));
    // how late each store's value is: the latency of the run it stores
    let mut made: BTreeMap<usize, f64> = BTreeMap::new();
    for &(s, i) in &runs {
        if let Some((c, _)) = used[i] {
            if matches!(code[c], Op::Store(_)) {
                made.insert(c, sim2(&code[s..=i], false, &regs, None, &HashMap::new()).1.max(1.0));
            }
        }
    }
    let mut changes: Vec<(usize, usize, Vec<Op>)> = Vec::new();
    for &(s, i) in &runs {
        // too small to gain anything
        if i - s < 2 {
            continue;
        }
        let run = &code[s..=i];
        let Some(e) = tree(run) else { continue };
        let depth = depths[i] as i32;
        let stored = if depth > 0 {
            loops.iter().find(|&&(t, j)| t < s && i < j).map(|&(t, j)| code[t..=j].iter().filter_map(|o| if let Op::Store(x) = o { Some(*x) } else { None }).collect::<HashSet<u32>>())
        } else {
            None
        };
        let address = matches!(used[i], Some((c, p)) if matches!(code[c], Op::LoadM(_)) || (matches!(code[c], Op::StoreM(_) | Op::OpM(..)) && p == 1));
        let target = match used[i] {
            Some((c, _)) => if let Op::Store(t) = code[c] { Some(t) } else { None },
            None => None,
        };
        // when each local's value is ready: in a loop, a local the loop
        // stores comes round from its last time, as late as the expression
        // that makes it; outside, a local stored just before in the same
        // straight code
        let mut ready: HashMap<u32, f64> = HashMap::new();
        let mut note = |x: u32, t: f64| {
            let e = ready.entry(x).or_insert(0.0);
            *e = e.max(t);
        };
        match loops.iter().find(|&&(t, j)| t < s && i < j) {
            Some(&(t, j)) => {
                for (k, l) in &made {
                    if (t..=j).contains(k) {
                        if let Op::Store(x) = code[*k] {
                            note(x, *l);
                        }
                    }
                }
            }
            None => {
                for k in (0..s).rev() {
                    match &code[k] {
                        Op::Label(_) | Op::Jmp(_) | Op::Jz(_) | Op::Jnz(_) | Op::Call(..) => break,
                        Op::Store(x) => note(*x, made.get(&k).copied().unwrap_or(LATE)),
                        _ => {}
                    }
                }
            }
        }
        let cx = Cx { stored, weight: loop_weight.powi(depth), outer: loop_weight.powi((depth - 1).max(0)), address, regs: regs.clone(), target, ready };
        let new = simplify(&e, 64, &cx, true);
        if new == e {
            continue;
        }
        let new_run = to_ops(&new);
        if cx.cost(&new_run, true) < cx.cost(run, true) - MARGIN * cx.weight && same(run, &new_run) {
            changes.push((s, i, new_run));
        }
    }
    let n = changes.len();
    for (s, i, new_run) in changes.into_iter().rev() {
        f.code.splice(s..=i, new_run);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use BinOp::*;

    fn cx() -> Cx {
        Cx { stored: None, weight: 1.0, outer: 1.0, address: false, regs: Default::default(), target: None, ready: HashMap::new() }
    }
    fn simp(run: &[Op]) -> Vec<Op> {
        let e = tree(run).unwrap();
        to_ops(&simplify(&e, 64, &cx(), true))
    }
    fn mul_count(run: &[Op]) -> usize {
        run.iter().filter(|o| matches!(o, Op::Bin(Mul))).count()
    }

    #[test]
    fn a_common_factor_comes_out() {
        // a*b + a*c -> a*(b + c): one multiplication fewer
        let run = [Op::Load(0), Op::Load(1), Op::Bin(Mul), Op::Load(0), Op::Load(2), Op::Bin(Mul), Op::Bin(Add)];
        let new = simp(&run);
        assert_eq!(mul_count(&new), 1, "{new:?}");
        assert!(same(&run, &new));
        assert!(cx().cost(&new, true) < cx().cost(&run, true));
    }

    #[test]
    fn a_polynomial_takes_horners_form() {
        // 3x^3 + 5x^2 + 7x + 11 with four multiplications by x and three by
        // constants -> ((3x + 5)x + 7)x + 11
        let x = || Op::Load(0);
        let run = [
            Op::Push(3), x(), Op::Bin(Mul), x(), Op::Bin(Mul), x(), Op::Bin(Mul),
            Op::Push(5), x(), Op::Bin(Mul), x(), Op::Bin(Mul), Op::Bin(Add),
            Op::Push(7), x(), Op::Bin(Mul), Op::Bin(Add),
            Op::Push(11), Op::Bin(Add),
        ];
        let new = simp(&run);
        assert!(mul_count(&new) <= 5, "{new:?}");
        assert!(same(&run, &new));
        assert!(cx().cost(&new, true) < cx().cost(&run, true));
    }

    #[test]
    fn like_terms_are_collected() {
        // x*3 + x*5 - y + y -> x << 3
        let run = [Op::Load(0), Op::Push(3), Op::Bin(Mul), Op::Load(0), Op::Push(5), Op::Bin(Mul), Op::Bin(Add), Op::Load(1), Op::Bin(Sub), Op::Load(1), Op::Bin(Add)];
        let new = simp(&run);
        assert_eq!(new, vec![Op::Load(0), Op::Push(3), Op::Bin(Shl)]);
    }

    #[test]
    fn a_c_int_is_cut_once() {
        // int: ((a*b cut) + (c*d cut)) cut -> (a*b + c*d) cut
        let run = [
            Op::Load(0), Op::Load(1), Op::Bin(Mul), Op::Sext(32),
            Op::Load(2), Op::Load(3), Op::Bin(Mul), Op::Sext(32),
            Op::Bin(Add), Op::Sext(32),
        ];
        let new = simp(&run);
        assert_eq!(new.iter().filter(|o| matches!(o, Op::Sext(_))).count(), 1, "{new:?}");
        assert!(same(&run, &new));
    }

    #[test]
    fn a_cut_is_not_moved_across_a_division_or_a_narrower_cut() {
        // (a*b cut 32) / 3: the cut stays under the division
        let run = [Op::Load(0), Op::Load(1), Op::Bin(Mul), Op::Sext(32), Op::Push(3), Op::Bin(DivTrunc), Op::Load(0), Op::Bin(Add), Op::Sext(32)];
        let new = simp(&run);
        assert!(same(&run, &new));
        assert!(new.windows(3).any(|w| w == [Op::Sext(32), Op::Push(3), Op::Bin(DivTrunc)]), "{new:?}");
        // a 16-bit cut under a 32-bit one stays
        let run = [Op::Load(0), Op::Load(1), Op::Bin(Mul), Op::Sext(16), Op::Load(2), Op::Bin(Add), Op::Sext(32)];
        let new = simp(&run);
        assert!(new.contains(&Op::Sext(16)) && same(&run, &new), "{new:?}");
    }

    #[test]
    fn a_checked_operation_is_left_where_it_is() {
        // Python: a*b + a*c with checked operations stays as it is
        let run = [Op::Load(0), Op::Load(1), Op::Bin(MulOv), Op::Load(0), Op::Load(2), Op::Bin(MulOv), Op::Bin(AddOv)];
        assert_eq!(simp(&run), run.to_vec());
        // a division by a variable is not cancelled: x/y - x/y stays
        let run = [Op::Load(0), Op::Load(1), Op::Bin(DivTrunc), Op::Load(0), Op::Load(1), Op::Bin(DivTrunc), Op::Bin(Sub), Op::Load(2), Op::Bin(Add)];
        let new = simp(&run);
        assert_eq!(new.iter().filter(|o| matches!(o, Op::Bin(DivTrunc))).count(), 2, "{new:?}");
    }

    #[test]
    fn invariant_terms_are_gathered_for_hoist() {
        // in a loop over i (slot 0): (i*m + j)*8 + base -> i*(8m) + (8j + base),
        // whose second part hoist takes out of the loop
        let stored: HashSet<u32> = [0].into();
        let cx = Cx { stored: Some(stored), weight: 8.0, outer: 1.0, address: true, regs: Default::default(), target: None, ready: HashMap::new() };
        let run = [Op::Load(0), Op::Load(1), Op::Bin(Mul), Op::Load(2), Op::Bin(Add), Op::Push(3), Op::Bin(Shl), Op::Load(3), Op::Bin(Add)];
        let e = tree(&run).unwrap();
        let new = to_ops(&simplify(&e, 64, &cx, true));
        assert!(same(&run, &new));
        assert!(cx.cost(&new, true) < cx.cost(&run, true), "{new:?}");
    }

    #[test]
    fn the_guard_finds_a_wrong_rewrite() {
        // x*2 is not x+1, and (x cut 32) + 1 is not (x + 1) cut 32... it is,
        // but x cut 16 + y is not (x + y) cut 16
        assert!(!same(&[Op::Load(0), Op::Push(2), Op::Bin(Mul)], &[Op::Load(0), Op::Push(1), Op::Bin(Add)]));
        assert!(!same(&[Op::Load(0), Op::Sext(16), Op::Load(1), Op::Bin(Add)], &[Op::Load(0), Op::Load(1), Op::Bin(Add), Op::Sext(16)]));
        assert!(same(&[Op::Load(0), Op::Sext(32), Op::Push(1), Op::Bin(Add), Op::Sext(32)], &[Op::Load(0), Op::Push(1), Op::Bin(Add), Op::Sext(32)]));
        // x / 0 stops in both
        assert!(same(&[Op::Load(0), Op::Push(0), Op::Bin(DivTrunc)], &[Op::Load(1), Op::Push(0), Op::Bin(ModTrunc)]));
        assert!(!same(&[Op::Load(0), Op::Load(1), Op::Bin(DivTrunc)], &[Op::Push(0)]));
    }

    /// Random ring expressions: each rule's result evaluated against the
    /// original, with every move of the beam checked, not only the best.
    #[test]
    fn every_move_is_an_identity() {
        let mut seed = 0x1234_5678_9ABC_DEF1u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        fn gen(next: &mut dyn FnMut() -> u64, d: u32) -> E {
            if d == 0 || next() % 4 == 0 {
                return match next() % 5 {
                    0 => E::Push((next() % 21) as i64 - 10),
                    _ => E::Load((next() % 3) as u32),
                };
            }
            let l = Box::new(gen(next, d - 1));
            let r = Box::new(gen(next, d - 1));
            match next() % 9 {
                0 | 1 => E::Bin(Add, l, r),
                2 => E::Bin(Sub, l, r),
                3 | 4 => E::Bin(Mul, l, r),
                5 => E::Bin(Shl, l, Box::new(E::Push((next() % 70) as i64))),
                6 => E::Un(if next() % 2 == 0 { UnOp::Neg } else { UnOp::Bnot }, l),
                7 => E::Sext([8, 16, 32][(next() % 3) as usize], l),
                _ => E::Bin([Lt, DivTrunc, Bxor][(next() % 3) as usize], l, Box::new(E::Push((next() % 7) as i64 + 2))),
            }
        }
        let c = cx();
        for _ in 0..150 {
            let e = gen(&mut next, 4);
            let n = 64;
            let run = to_ops(&e);
            let new = to_ops(&simplify(&e, n, &c, true));
            assert!(same(&run, &new), "{run:?}\n{new:?}");
            if !is_ring(&e, n) {
                continue;
            }
            let mut r = Region { atoms: Vec::new(), index: HashMap::new(), n, cx: &c, inv: Vec::new(), atom_ops: Vec::new(), atom_late: Vec::new(), top: false };
            let a = r.read(&e);
            let back = r.emit(&a).0;
            assert!(same(&run, &back), "read: {run:?}\n{back:?}");
            for m in moves(&a, n, 40) {
                let out = r.emit(&m).0;
                assert!(same(&run, &out), "{a:?} -> {m:?}\n{run:?}\n{out:?}");
                for m2 in moves(&m, n, 5) {
                    assert!(same(&run, &r.emit(&m2).0), "{m:?} -> {m2:?}");
                }
            }
        }
    }
}
