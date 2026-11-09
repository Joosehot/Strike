//! --release: work across statements, after the loops are rotated and
//! unrolled (so an unrolled loop's copies are one straight run):
//!
//! - **Known low bits.** For every local, how many of its lowest bits are
//!   known to be zero, from every value ever stored into it (a counter that
//!   starts at 0 and goes up by 2 is even). Where x has its t lowest bits
//!   zero and 0 <= c < 2^t, x + c has no carry: it is x | c, its bits apart
//!   from x's. So, exactly:
//!     (x + c) >> k = (x >> k) + (c >> k)
//!     (x + c) ^ d  = x ^ (c ^ d)
//!     (x + c) & d  = (x & d) + (c & d)
//!   which is how an unrolled copy's `(i + 1) >> 1` becomes the first
//!   copy's `i >> 1`.
//! - **Value numbering.** Each local's value is known as an expression of
//!   the values its locals had then (every store gives a local a new
//!   version). A store of what the local already holds is left out; an
//!   expression another local already holds is read from it.
//! - **Common subexpressions.** An expression worked out in two places of
//!   the same straight code, from the same versions of its locals, is
//!   worked out once into a new local before the first.
//!
//! Every rewrite is checked like the simplifier's: the old and the new run
//! must agree on 200 sets of inputs (each local's known zero bits kept).
//! Only expressions that cannot fail are moved or shared; one that is left
//! out had already been worked out with the same values, so it could not
//! fail either.

use super::ir::{BinOp, Func, Op, UnOp};
use super::simplify::{fallible, same_masked, to_ops, tree, E};
use std::collections::HashMap;

/// How many of a value's lowest bits are known to be zero (64: it is 0).
fn tz(e: &E, z: &[u32]) -> u32 {
    match e {
        E::Push(c) => c.trailing_zeros(),
        E::Load(s) => z.get(*s as usize).copied().unwrap_or(0),
        E::Bin(b, l, r) => {
            let (a, c) = (tz(l, z), tz(r, z));
            match b {
                BinOp::Add | BinOp::Sub | BinOp::Bor | BinOp::Bxor => a.min(c),
                BinOp::Mul => (a + c).min(64),
                BinOp::Band => a.max(c),
                BinOp::Shl => match **r {
                    E::Push(k) => (a + (k as u32 & 63)).min(64),
                    _ => 0,
                },
                _ => 0,
            }
        }
        // -x has x's zero bits (two's complement)
        E::Un(UnOp::Neg | UnOp::Plus, x) => tz(x, z),
        // the low n bits kept: all zero is 0
        E::Sext(n, x) => {
            let t = tz(x, z);
            if t >= *n as u32 {
                64
            } else {
                t
            }
        }
        _ => 0,
    }
}

/// Every local's known zero bits: the fewest of any value stored into it
/// (a parameter: none; a local never stored holds 0). The largest
/// solution, found from above: every value a local ever holds was stored,
/// so by induction over the run it has at least that many.
pub(super) fn zeros(f: &Func) -> Vec<u32> {
    let st = super::opt::starts(&f.code);
    let stores: Vec<(u32, Option<E>)> = f
        .code
        .iter()
        .enumerate()
        .filter_map(|(k, o)| {
            let Op::Store(x) = o else { return None };
            let e = k.checked_sub(1).and_then(|p| st[p]).and_then(|s| tree(&f.code[s..k]));
            Some((*x, e))
        })
        .collect();
    let mut z: Vec<u32> = (0..f.slots).map(|s| if s < f.params { 0 } else { 64 }).collect();
    loop {
        let mut changed = false;
        for (x, e) in &stores {
            let t = e.as_ref().map_or(0, |e| tz(e, &z));
            if t < z[*x as usize] {
                z[*x as usize] = t;
                changed = true;
            }
        }
        if !changed {
            return z;
        }
    }
}

/// Does the run hold a float operation? Such a value is kept where it is
/// made: no new local is given one (a float local is x64opt's to place).
fn floats(run: &[Op]) -> bool {
    run.iter().any(|o| match o {
        Op::Bin(b) => b.is_float(),
        Op::Un(u) => u.is_float(),
        _ => false,
    })
}

/// x + c with c's bits below x's known zero bits: (x, c).
fn disjoint<'a>(e: &'a E, z: &[u32]) -> Option<(&'a E, i64)> {
    let E::Bin(BinOp::Add, l, r) = e else { return None };
    let (x, c) = match (&**l, &**r) {
        (x, E::Push(c)) | (E::Push(c), x) => (x, *c),
        _ => return None,
    };
    let t = tz(x, z);
    (c >= 0 && (t >= 63 || c < 1i64 << t)).then_some((x, c))
}

/// The tree with the known-bits rules applied, from the leaves up.
fn bits(e: &E, z: &[u32]) -> E {
    let e = match e {
        E::Bin(b, l, r) => E::Bin(*b, Box::new(bits(l, z)), Box::new(bits(r, z))),
        E::Un(u, x) => E::Un(*u, Box::new(bits(x, z))),
        E::Sext(n, x) => E::Sext(*n, Box::new(bits(x, z))),
        _ => return e.clone(),
    };
    let push = |c: i64| Box::new(E::Push(c));
    match &e {
        // (x + c) >> k = (x >> k) + (c >> k)
        E::Bin(BinOp::Shr, p, k) => {
            if let (Some((x, c)), E::Push(k)) = (disjoint(p, z), &**k) {
                let k = *k & 63;
                let s = E::Bin(BinOp::Shr, Box::new(x.clone()), push(k));
                return if c >> k == 0 { s } else { E::Bin(BinOp::Add, Box::new(s), push(c >> k)) };
            }
        }
        // (x + c) ^ d = x ^ (c ^ d); (x + c) & d = (x & d) + (c & d)
        E::Bin(b @ (BinOp::Bxor | BinOp::Band), l, r) => {
            let (p, d) = match (&**l, &**r) {
                (p, E::Push(d)) | (E::Push(d), p) => (p, *d),
                _ => return e,
            };
            if let Some((x, c)) = disjoint(p, z) {
                let x = Box::new(x.clone());
                return match b {
                    BinOp::Bxor if c ^ d == 0 => *x,
                    BinOp::Bxor => E::Bin(BinOp::Bxor, x, push(c ^ d)),
                    _ if c & d == 0 => E::Bin(BinOp::Band, x, push(d)),
                    _ => E::Bin(BinOp::Add, Box::new(E::Bin(BinOp::Band, x, push(d))), push(c & d)),
                };
            }
        }
        _ => {}
    }
    e
}

/// The largest plain expressions of the code: (start, end), each not inside
/// another.
fn runs(code: &[Op], st: &[Option<usize>]) -> Vec<(usize, usize)> {
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
    runs
}

/// The known-bits rules on every expression; returns how many changed.
fn known_bits(f: &mut Func) -> usize {
    let z = zeros(f);
    if z.iter().all(|t| *t == 0 || *t == 64) {
        return 0;
    }
    let st = super::opt::starts(&f.code);
    let mut changes = Vec::new();
    for (s, i) in runs(&f.code, &st) {
        let run = &f.code[s..=i];
        let Some(e) = tree(run) else { continue };
        let new = bits(&e, &z);
        if new == e {
            continue;
        }
        let new_run = to_ops(&new);
        let mask = |s: u32| {
            let t = z.get(s as usize).copied().unwrap_or(0);
            if t >= 64 {
                0
            } else {
                -1i64 << t
            }
        };
        if new_run.len() <= run.len() && same_masked(run, &new_run, &mask) {
            changes.push((s, i, new_run));
        }
    }
    let n = changes.len();
    for (s, i, new_run) in changes.into_iter().rev() {
        f.code.splice(s..=i, new_run);
    }
    n
}

/// A run's value as a key: its operations, each local with its version.
fn key(run: &[Op], ver: &[u32]) -> Vec<i64> {
    let mut k = Vec::with_capacity(run.len() * 2);
    for op in run {
        match op {
            Op::Push(c) => k.extend([1, *c]),
            Op::Load(s) => k.extend([2, *s as i64, ver[*s as usize] as i64]),
            Op::AddrG(g) => k.extend([3, *g as i64]),
            Op::Frame(n) => k.extend([4, *n as i64]),
            Op::Bin(b) => k.extend([5, *b as i64]),
            Op::Un(u) => k.extend([6, *u as i64]),
            Op::Sext(n) => k.extend([7, *n as i64]),
            _ => k.extend([8, 0]),
        }
    }
    k
}

/// Value numbering: a store of the value the local already holds is left
/// out, and a value another local holds is read from it. Returns how many.
fn numbering(f: &mut Func) -> usize {
    let code = &f.code;
    let st = super::opt::starts(code);
    let mut ver = vec![0u32; f.slots as usize];
    let mut holds: Vec<Option<Vec<i64>>> = vec![None; f.slots as usize];
    // (start, end, what takes the place: None = nothing)
    let mut edits: Vec<(usize, usize, Option<Op>)> = Vec::new();
    for (k, op) in code.iter().enumerate() {
        match op {
            // another way in: nothing is known
            Op::Label(_) => holds.iter_mut().for_each(|h| *h = None),
            Op::Store(x) => {
                let x = *x as usize;
                let run = k.checked_sub(1).and_then(|p| st[p].map(|s| (s, p)));
                let Some((s, p)) = run.filter(|&(s, p)| tree(&code[s..=p]).is_some()) else {
                    ver[x] += 1;
                    holds[x] = None;
                    continue;
                };
                let kk = key(&code[s..=p], &ver);
                if holds[x].as_ref() == Some(&kk) {
                    edits.push((s, k, None));
                    continue;
                }
                // another local holds it: read from there (worth it for an
                // operation or more)
                if p > s {
                    if let Some(y) = (0..holds.len()).find(|&y| y != x && holds[y].as_ref() == Some(&kk)) {
                        edits.push((s, p, Some(Op::Load(y as u32))));
                    }
                }
                ver[x] += 1;
                holds[x] = Some(kk);
            }
            _ => {}
        }
    }
    let n = edits.len();
    for (s, e, with) in edits.into_iter().rev() {
        f.code.splice(s..=e, with);
    }
    n
}

/// The fewest operations an expression worth sharing has.
const SHARE_OPS: usize = 2;
/// The most new locals one straight stretch of code gets.
const SHARE_MAX: usize = 4;

/// Common subexpressions of straight code, worked out once into a new
/// local before their first place. Returns how many.
fn common(f: &mut Func) -> usize {
    let code = &f.code;
    let st = super::opt::starts(code);
    let all = runs(code, &st);
    let mut ver = vec![0u32; f.slots as usize];
    // per stretch (between labels): key -> places (start, end, run start)
    let mut places: Vec<HashMap<Vec<i64>, Vec<(usize, usize, usize)>>> = vec![HashMap::new()];
    let mut ri = 0;
    for (k, op) in code.iter().enumerate() {
        if let Op::Label(_) = op {
            places.push(HashMap::new());
        }
        while ri < all.len() && all[ri].1 < k {
            ri += 1;
        }
        // the subexpressions of a run, noted at its last operation
        if ri < all.len() && all[ri].1 == k {
            let (rs, re) = all[ri];
            for i in rs..=re {
                let Some(s) = st[i] else { continue };
                if !matches!(code[i], Op::Bin(_) | Op::Un(_) | Op::Sext(_)) {
                    continue;
                }
                let part = &code[s..=i];
                let ops = part.iter().filter(|o| matches!(o, Op::Bin(_) | Op::Un(_))).count();
                let has = part.iter().any(|o| matches!(o, Op::Load(_)));
                if ops < SHARE_OPS || !has || floats(part) || tree(part).is_none_or(|e| fallible(&e)) {
                    continue;
                }
                // a vector's address that x64opt makes an address mode as it
                // is: a local would only cost a lea (vectorize.rs)
                if matches!(code.get(re + 1), Some(Op::VLoad(..) | Op::VStore(..))) && tree(part).is_some_and(|e| lazy(&e).is_some()) {
                    continue;
                }
                places.last_mut().unwrap().entry(key(part, &ver)).or_default().push((s, i, rs));
            }
        }
        if let Op::Store(x) = op {
            ver[*x as usize] += 1;
        }
    }
    let mut taken: Vec<(usize, usize)> = Vec::new();
    let mut inserts: Vec<(usize, Vec<Op>)> = Vec::new();
    let mut replaces: Vec<(usize, usize, u32)> = Vec::new();
    let overlaps = |taken: &[(usize, usize)], s: usize, e: usize| taken.iter().any(|&(a, b)| s <= b && a <= e);
    let mut slots = f.slots;
    for stretch in places {
        let mut keys: Vec<(Vec<i64>, Vec<(usize, usize, usize)>)> = stretch.into_iter().filter(|(_, p)| p.len() >= 2).collect();
        // the largest first, then the first place: deterministic
        keys.sort_by(|a, b| (b.1[0].1 - b.1[0].0).cmp(&(a.1[0].1 - a.1[0].0)).then(a.1[0].0.cmp(&b.1[0].0)));
        let mut made = 0;
        for (_, ps) in keys {
            if made >= SHARE_MAX {
                break;
            }
            let free: Vec<_> = ps.iter().copied().filter(|&(s, e, _)| !overlaps(&taken, s, e)).collect();
            // two places that do not hold one another
            if free.len() < 2 || free.windows(2).any(|w| w[1].0 <= w[0].1) {
                continue;
            }
            let tmp = slots;
            slots += 1;
            made += 1;
            let (s0, e0, r0) = free[0];
            let mut ops = code[s0..=e0].to_vec();
            ops.push(Op::Store(tmp));
            inserts.push((r0, ops));
            for &(s, e, _) in &free {
                taken.push((s, e));
                replaces.push((s, e, tmp));
            }
        }
    }
    if replaces.is_empty() {
        return 0;
    }
    let n = slots - f.slots;
    inserts.sort_by_key(|x| x.0);
    replaces.sort_by_key(|x| x.0);
    let mut out = Vec::with_capacity(code.len() + 8);
    let (mut ii, mut rj) = (0, 0);
    let mut k = 0;
    while k < code.len() {
        while ii < inserts.len() && inserts[ii].0 == k {
            out.extend(inserts[ii].1.iter().cloned());
            ii += 1;
        }
        if rj < replaces.len() && replaces[rj].0 == k {
            let (_, e, t) = replaces[rj];
            out.push(Op::Load(t));
            rj += 1;
            k = e + 1;
            continue;
        }
        out.push(code[k].clone());
        k += 1;
    }
    f.code = out;
    f.slots = slots;
    n as usize
}

/// The passes, in order; returns how many changes.
pub fn run(f: &mut Func) -> usize {
    let a = known_bits(f);
    let b = numbering(f);
    let c = common(f);
    let d = spill(f);
    a + b + c + d
}

/// What x64opt keeps a value as without working it out: a constant, a
/// local, a local plus a constant (a lea), a local times 1, 2, 4 or 8, or
/// base + index * scale + displacement.
#[derive(Clone, Copy, PartialEq)]
enum Lazy {
    Imm,
    Slot(u32),
    Lea,
    Scaled,
    Index,
}

fn lazy(e: &E) -> Option<Lazy> {
    use Lazy::*;
    let fits = |c: i64| i32::try_from(c).is_ok();
    Some(match e {
        E::Push(_) => Imm,
        E::Load(s) => Slot(*s),
        // a global's or the frame's address: one lea, hoisted out of loops
        E::AddrG(_) | E::Frame(_) => Slot(u32::MAX),
        E::Bin(BinOp::Add | BinOp::Sub, l, r) if matches!(**r, E::Push(c) if fits(c)) => match lazy(l)? {
            Slot(_) | Lea => Lea,
            Index => Index,
            _ => return None,
        },
        E::Bin(BinOp::Shl, l, r) if matches!(**r, E::Push(0..=3)) => match lazy(l)? {
            Slot(_) | Lea => Scaled,
            _ => return None,
        },
        E::Bin(BinOp::Add, l, r) => match (lazy(l)?, lazy(r)?) {
            (Slot(_) | Lea, Scaled) | (Scaled, Slot(_) | Lea) => Index,
            (Slot(a), Slot(b)) if a != b => Index,
            _ => return None,
        },
        _ => return None,
    })
}

/// x64opt works an expression out in rax: where both operands of an
/// operation must be worked out, the left one waits on the machine's stack
/// (a push and a pop), with every value under it. Such a right operand is
/// worked out first, into a new local (a register), when it cannot fail
/// (so nothing that could stop the program moves). Returns how many.
fn spill(f: &mut Func) -> usize {
    let st = super::opt::starts(&f.code);
    let all = runs(&f.code, &st);
    let first = f.slots;
    let mut most = 0u32;
    let mut edits: Vec<(usize, usize, Vec<Op>)> = Vec::new();
    for (s, i) in all {
        let Some(e) = tree(&f.code[s..=i]) else { continue };
        if floats(&f.code[s..=i]) {
            continue;
        }
        // the local the value goes into
        let target = match f.code.get(i + 1) {
            Some(Op::Store(t)) => Some(*t),
            Some(Op::Sext(_)) => match f.code.get(i + 2) {
                Some(Op::Store(t)) => Some(*t),
                _ => None,
            },
            _ => None,
        };
        let mut pre: Vec<Op> = Vec::new();
        let mut used = 0u32;
        let new = split(&e, first, target, &mut used, &mut pre);
        if used == 0 {
            continue;
        }
        most = most.max(used);
        let mut with = pre;
        with.extend(to_ops(&new));
        edits.push((s, i, with));
    }
    let n = edits.len();
    for (s, i, with) in edits.into_iter().rev() {
        f.code.splice(s..=i, with);
    }
    f.slots += most;
    n
}

/// (`target`: a right operand that reads the local the value goes into is
/// on the chain from one time round the loop to the next, and a move
/// into a new local would lengthen it: measured slower, it stays.)
fn split(e: &E, first: u32, target: Option<u32>, used: &mut u32, pre: &mut Vec<Op>) -> E {
    let reads = |e: &E, t: u32| to_ops(e).contains(&Op::Load(t));
    match e {
        E::Bin(b, l, r) => {
            let l = split(l, first, target, used, pre);
            let r = split(r, first, target, used, pre);
            if lazy(&l).is_none() && lazy(&r).is_none() && !fallible(&r) && !fallible(&l) && !target.is_some_and(|t| reads(&r, t)) {
                // the right operand first, into the next new local; the
                // locals are new for every expression (used only inside it)
                let t = first + *used;
                *used += 1;
                pre.extend(to_ops(&r));
                pre.push(Op::Store(t));
                return E::Bin(*b, Box::new(l), Box::new(E::Load(t)));
            }
            E::Bin(*b, Box::new(l), Box::new(r))
        }
        E::Un(u, x) => E::Un(*u, Box::new(split(x, first, target, used, pre))),
        E::Sext(n, x) => E::Sext(*n, Box::new(split(x, first, target, used, pre))),
        _ => e.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use BinOp::*;

    /// i = 0; top: s = s + ((i >> 1) ^ (i + 1 >> 1)); i = i + 2; ...
    fn even_counter(body: Vec<Op>) -> Func {
        let mut code = vec![Op::Push(0), Op::Store(0), Op::Label(0)];
        code.extend(body);
        code.extend([Op::Load(0), Op::Push(2), Op::Bin(Add), Op::Store(0), Op::Load(0), Op::Push(100), Op::Bin(Lt), Op::Jnz(0), Op::Load(1), Op::Ret]);
        Func::new("t", 0, 4, code)
    }

    #[test]
    fn a_counter_going_up_by_two_is_even() {
        let f = even_counter(vec![]);
        assert_eq!(zeros(&f)[0], 1);
        let mut g = f.clone();
        g.code.insert(0, Op::Store(0));
        g.code.insert(0, Op::Push(1));
        g.code[2] = Op::Push(1);
        assert_eq!(zeros(&g)[0], 0, "starting at 1 it is not");
    }

    #[test]
    fn an_unrolled_copy_reads_the_first_copys_value() {
        // b = i >> 1; c = (i + 1) >> 1 ^ 7 ... with i even: (i + 1) >> 1 = i >> 1
        let body = vec![
            Op::Load(0), Op::Push(1), Op::Bin(Shr), Op::Store(2),
            Op::Load(0), Op::Push(1), Op::Bin(Add), Op::Push(1), Op::Bin(Shr), Op::Store(2),
            Op::Load(0), Op::Push(1), Op::Bin(Add), Op::Push(5), Op::Bin(Bxor), Op::Store(3),
            Op::Load(1), Op::Load(2), Op::Bin(Add), Op::Load(3), Op::Bin(Add), Op::Store(1),
        ];
        let mut f = even_counter(body);
        assert!(run(&mut f) >= 2);
        // the second store of slot 2 went; (i + 1) ^ 5 is i ^ 4
        assert_eq!(f.code.iter().filter(|o| **o == Op::Store(2)).count(), 1, "{:?}", f.code);
        assert!(f.code.windows(3).any(|w| w == [Op::Load(0), Op::Push(4), Op::Bin(Bxor)]), "{:?}", f.code);
    }

    #[test]
    fn the_rules_hold_for_every_even_number() {
        let z = vec![1u32];
        let x = || Box::new(E::Load(0));
        let plus = |c: i64| Box::new(E::Bin(Add, x(), Box::new(E::Push(c))));
        for c in [0, 1] {
            for d in [0, 1, 2, 3, -1, 12345, i64::MIN] {
                for e in [
                    E::Bin(Shr, plus(c), Box::new(E::Push(d & 63))),
                    E::Bin(Bxor, plus(c), Box::new(E::Push(d))),
                    E::Bin(Band, plus(c), Box::new(E::Push(d))),
                ] {
                    let new = bits(&e, &z);
                    assert!(same_masked(&to_ops(&e), &to_ops(&new), &|_| -2), "{e:?} -> {new:?}");
                }
            }
        }
        // an odd constant is not below 2^1... 3 is not: (x + 3) >> 1 stays
        let e = E::Bin(Shr, plus(3), Box::new(E::Push(1)));
        assert_eq!(bits(&e, &z), e);
    }

    #[test]
    fn a_shared_expression_is_worked_out_once() {
        // s = s + (a + b) * c * 3; t = t + (a + b) * c * 5: (a + b) * c once
        let part = || vec![Op::Load(2), Op::Load(3), Op::Bin(Add), Op::Load(4), Op::Bin(Mul)];
        let mut code = vec![Op::Load(0)];
        code.extend(part());
        code.extend([Op::Push(3), Op::Bin(Mul), Op::Bin(Add), Op::Store(0), Op::Load(1)]);
        code.extend(part());
        code.extend([Op::Push(5), Op::Bin(Mul), Op::Bin(Add), Op::Store(1), Op::Load(0), Op::Load(1), Op::Bin(Add), Op::Ret]);
        let mut f = Func::new("t", 5, 5, code.clone());
        assert_eq!(common(&mut f), 1);
        assert_eq!(f.slots, 6);
        assert_eq!(f.code.iter().filter(|o| **o == Op::Bin(Mul)).count(), 3, "{:?}", f.code);
        // a store between: not the same value
        let mut g = Func::new("t", 5, 5, code);
        g.code.insert(10, Op::Store(2));
        g.code.insert(10, Op::Push(9));
        assert_eq!(common(&mut g), 0);
    }

    #[test]
    fn a_store_of_what_is_there_is_left_out_but_not_after_a_label() {
        let mut f = Func::new(
            "t",
            2,
            3,
            vec![
                Op::Load(0), Op::Load(1), Op::Bin(Mul), Op::Store(2),
                Op::Load(0), Op::Load(1), Op::Bin(Mul), Op::Store(2),
                Op::Label(0),
                Op::Load(0), Op::Load(1), Op::Bin(Mul), Op::Store(2),
                Op::Load(2), Op::Ret,
            ],
        );
        assert_eq!(numbering(&mut f), 1);
        assert_eq!(f.code.iter().filter(|o| **o == Op::Store(2)).count(), 2);
    }
}
