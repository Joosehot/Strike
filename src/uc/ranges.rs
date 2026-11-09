//! --release: the range of every local, and the cuts it makes needless.
//!
//! Each local's values lie in an interval: the union of the intervals of
//! every value ever stored into it (a parameter: anything; a local never
//! stored holds 0). Every value a local ever holds was stored, so by
//! induction over the run the intervals hold at every point. A counter
//! would grow without end that way, so a loop's own test is used, as
//! opt::counters does for one shape: in a loop entered only by its top,
//! whose test is `c < bound` (or `<=`) and which stores c once, c is below
//! the bound wherever the body reads it before that store. A local still
//! growing after a number of rounds is taken to be anything (widening).
//!
//! A cut `Sext(n)` whose operand's interval fits n bits changes nothing and
//! is left out: exactly, by the arithmetic of the machine (no undefined
//! behaviour assumed; an operation that could wrap gives "anything").

use super::ir::{label_count, BinOp, Func, Op, UnOp};
use super::simplify::{tree, E};

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct R {
    pub(super) lo: i64,
    pub(super) hi: i64,
}

const ALL: R = R { lo: i64::MIN, hi: i64::MAX };

impl R {
    fn of(lo: i128, hi: i128) -> R {
        if lo < i64::MIN as i128 || hi > i64::MAX as i128 {
            ALL
        } else {
            R { lo: lo as i64, hi: hi as i64 }
        }
    }
    fn union(self, o: R) -> R {
        R { lo: self.lo.min(o.lo), hi: self.hi.max(o.hi) }
    }
    fn bits(n: u8) -> R {
        if n >= 64 {
            ALL
        } else {
            R { lo: -(1i64 << (n - 1)), hi: (1i64 << (n - 1)) - 1 }
        }
    }
    fn within(self, o: R) -> bool {
        o.lo <= self.lo && self.hi <= o.hi
    }
    fn nonneg(self) -> bool {
        self.lo >= 0
    }
}

/// The smallest 2^k - 1 at or above x (x >= 0).
fn ones(x: i64) -> i64 {
    if x <= 0 {
        0
    } else {
        (u64::MAX >> (x as u64).leading_zeros()) as i64
    }
}

/// The interval of a value, its locals' intervals given by `at`.
pub(super) fn range(e: &E, at: &dyn Fn(u32) -> R) -> R {
    match e {
        E::Push(c) => R { lo: *c, hi: *c },
        E::Load(s) => at(*s),
        E::Bin(b, l, r) => {
            let (a, c) = (range(l, at), range(r, at));
            let (al, ah, cl, ch) = (a.lo as i128, a.hi as i128, c.lo as i128, c.hi as i128);
            match b {
                BinOp::Add => R::of(al + cl, ah + ch),
                BinOp::Sub => R::of(al - ch, ah - cl),
                BinOp::Mul => {
                    let p = [al * cl, al * ch, ah * cl, ah * ch];
                    R::of(*p.iter().min().unwrap(), *p.iter().max().unwrap())
                }
                b if b.is_comparison() => R { lo: 0, hi: 1 },
                BinOp::Band if a.nonneg() || c.nonneg() => {
                    let hi = match (a.nonneg(), c.nonneg()) {
                        (true, true) => a.hi.min(c.hi),
                        (true, false) => a.hi,
                        _ => c.hi,
                    };
                    R { lo: 0, hi }
                }
                BinOp::Bor | BinOp::Bxor if a.nonneg() && c.nonneg() => R { lo: 0, hi: ones(a.hi.max(c.hi)) },
                BinOp::Shr => match **r {
                    E::Push(k) => {
                        let k = k & 63;
                        R { lo: a.lo >> k, hi: a.hi >> k }
                    }
                    _ => ALL,
                },
                BinOp::Shl => match **r {
                    E::Push(k) => {
                        let k = (k & 63) as u32;
                        if k >= 62 {
                            ALL
                        } else {
                            R::of(al << k, ah << k)
                        }
                    }
                    _ => ALL,
                },
                // a division by a constant other than 0 and -1: toward zero
                BinOp::DivTrunc | BinOp::DivFloor if matches!(**r, E::Push(d) if d > 0) => {
                    let E::Push(d) = **r else { unreachable!() };
                    R { lo: a.lo.div_euclid(d).min(a.lo / d), hi: a.hi / d }
                }
                BinOp::ModTrunc if matches!(**r, E::Push(d) if d != 0 && d != i64::MIN) => {
                    let E::Push(d) = **r else { unreachable!() };
                    let m = d.abs() - 1;
                    R { lo: if a.nonneg() { 0 } else { -m }, hi: if a.hi <= 0 { 0 } else { m } }
                }
                _ => ALL,
            }
        }
        E::Un(u, x) => {
            let a = range(x, at);
            match u {
                UnOp::Plus => a,
                UnOp::Neg | UnOp::NegOv => R::of(-(a.hi as i128), -(a.lo as i128)),
                UnOp::Not => R { lo: 0, hi: 1 },
                UnOp::Bnot => R::of(-(a.hi as i128) - 1, -(a.lo as i128) - 1),
                _ => ALL,
            }
        }
        E::Sext(n, x) => {
            let a = range(x, at);
            if a.within(R::bits(*n)) {
                a
            } else {
                R::bits(*n)
            }
        }
        _ => ALL,
    }
}

/// How many rounds before a local still growing is taken to be anything.
const ROUNDS: usize = 12;

/// Every local's interval.
pub(super) fn ranges(f: &Func) -> Vec<R> {
    sites(f).at
}

/// Every local's interval, and the counted loops whose tests bound their
/// counters where the body reads them.
pub(super) struct Sites {
    at: Vec<R>,
    /// (top, the counter's store, counter, bound, `<=`)
    loops: Vec<(usize, usize, u32, E, bool)>,
}

impl Sites {
    /// The interval of a local read at operation k: its own, and below the
    /// bound of a counted loop whose test came before k and whose store
    /// comes at k or after.
    pub(super) fn local_at(&self, k: usize, s: u32) -> R {
        let mut v = self.at.get(s as usize).copied().unwrap_or(ALL);
        for (t, sc, c, bound, le) in &self.loops {
            if *c == s && *t + 4 < k && k <= *sc {
                let b = range(bound, &|q| self.at.get(q as usize).copied().unwrap_or(ALL));
                let hi = if *le { b.hi } else { b.hi.saturating_sub(1) };
                v.hi = v.hi.min(hi);
                v.lo = v.lo.min(v.hi);
            }
        }
        v
    }
    /// The interval of an expression whose locals are read at operation k.
    pub(super) fn at(&self, k: usize, e: &E) -> R {
        range(e, &|s| self.local_at(k, s))
    }
}

pub(super) fn sites(f: &Func) -> Sites {
    let code = &f.code;
    let st = super::opt::starts(code);
    // the counted loops: (top, back jump, counter, bound, `<=`)
    let mut label_at = vec![usize::MAX; label_count(code) as usize];
    for (i, op) in code.iter().enumerate() {
        if let Op::Label(l) = op {
            label_at[*l as usize] = i;
        }
    }
    // (top, the counter's store, counter, bound, `<=`)
    let mut loops: Vec<(usize, usize, u32, E, bool)> = Vec::new();
    for (j, op) in code.iter().enumerate() {
        let Op::Jmp(top) = op else { continue };
        let t = label_at[*top as usize];
        if t >= j || t + 4 > j {
            continue;
        }
        let (Op::Load(c), bound, Op::Bin(cmp @ (BinOp::Lt | BinOp::Le)), Op::Jz(_)) = (&code[t + 1], &code[t + 2], &code[t + 3], &code[t + 4]) else { continue };
        let bound = match bound {
            Op::Push(n) => E::Push(*n),
            Op::Load(n) if n != c => E::Load(*n),
            _ => continue,
        };
        let stores: Vec<usize> = (t..j).filter(|&k| code[k] == Op::Store(*c)).collect();
        let inside = |l: u32| (t..=j).contains(&label_at[l as usize]);
        let entered = code.iter().enumerate().any(|(k, o)| matches!(o, Op::Jmp(l) | Op::Jz(l) | Op::Jnz(l) if inside(*l) && *l != *top && !(t..=j).contains(&k)));
        let [sc] = stores[..] else { continue };
        // no way back between the test and the store (an inner loop round
        // the store would add to c without testing it)
        let back = code.iter().enumerate().any(|(k, o)| matches!(o, Op::Jmp(l) | Op::Jz(l) | Op::Jnz(l) if *l != *top && (t + 4..sc).contains(&label_at[*l as usize]) && k >= sc));
        if !entered && !back {
            loops.push((t, sc, *c, bound, *cmp == BinOp::Le));
        }
    }
    let stores: Vec<(usize, u32, Option<E>, Option<R>)> = code
        .iter()
        .enumerate()
        .filter_map(|(k, o)| {
            let Op::Store(x) = o else { return None };
            let e = k.checked_sub(1).and_then(|p| st[p]).and_then(|s| tree(&code[s..k]));
            // a narrow element read from memory
            let r = match k.checked_sub(1).map(|p| &code[p]) {
                Some(Op::LoadM(w)) if *w < 8 => Some(R::bits(8 * w)),
                _ => None,
            };
            Some((k, *x, e, r))
        })
        .collect();
    let n = f.slots as usize;
    // None: nothing stored yet (a local starts at 0)
    let start: Vec<R> = (0..n).map(|s| if (s as u32) < f.params { ALL } else { R { lo: 0, hi: 0 } }).collect();
    // the interval of what one store stores, the locals' intervals `at`
    let stored = |k: usize, e: &Option<E>, r: &Option<R>, at: &[R]| -> R {
        match (e, r) {
            (_, Some(r)) => *r,
            (Some(e), None) => {
                let lookup = |s: u32| -> R {
                    let mut v = at.get(s as usize).copied().unwrap_or(ALL);
                    // below the loop's bound where its one store reads it
                    for (t, sc, c, bound, le) in &loops {
                        // read after the test and before the store
                        if *c == s && *t + 4 < k && k <= *sc {
                            let b = range(bound, &|q| at.get(q as usize).copied().unwrap_or(ALL));
                            let hi = if *le { b.hi } else { b.hi.saturating_sub(1) };
                            v.hi = v.hi.min(hi);
                            v.lo = v.lo.min(v.hi);
                        }
                    }
                    v
                };
                range(e, &lookup)
            }
            (None, None) => ALL,
        }
    };
    let mut at = start.clone();
    // up: every store's interval added in, a bound still moving after
    // ROUNDS rounds taken to its end (widening)
    for round in 0.. {
        let mut changed = false;
        for (k, x, e, r) in &stores {
            let got = stored(*k, e, r, &at);
            let old = at[*x as usize];
            let mut new = old.union(got);
            if new != old {
                if round >= ROUNDS {
                    if new.lo < old.lo {
                        new.lo = i64::MIN;
                    }
                    if new.hi > old.hi {
                        new.hi = i64::MAX;
                    }
                }
                at[*x as usize] = new;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // down: each local's interval made again from its stores alone, from
    // intervals that hold (narrowing: each round still holds)
    for _ in 0..2 {
        let mut next = start.clone();
        for (k, x, e, r) in &stores {
            let got = stored(*k, e, r, &at);
            next[*x as usize] = next[*x as usize].union(got);
        }
        at = next.iter().zip(&at).map(|(a, b)| R { lo: a.lo.max(b.lo), hi: a.hi.min(b.hi) }).collect();
    }
    Sites { at, loops }
}

/// Leaves out every cut that cannot change its value; returns how many.
pub fn run(f: &mut Func) -> usize {
    if !f.code.iter().any(|o| matches!(o, Op::Sext(_))) {
        return 0;
    }
    let at = ranges(f);
    let st = super::opt::starts(&f.code);
    let mut drop = Vec::new();
    for (i, op) in f.code.iter().enumerate() {
        let Op::Sext(n) = op else { continue };
        let Some(s) = i.checked_sub(1).and_then(|p| st[p]) else { continue };
        let Some(e) = tree(&f.code[s..i]) else { continue };
        if range(&e, &|x| at.get(x as usize).copied().unwrap_or(ALL)).within(R::bits(*n)) {
            drop.push(i);
        }
    }
    for &i in drop.iter().rev() {
        f.code.remove(i);
    }
    drop.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use BinOp::*;

    #[test]
    fn a_counted_loop_s_cuts_go() {
        // i = 0; top: if !(i < 1000) goto end; s = (s + i * 3) cut 32... s is
        // not bounded (it adds up), but i * 3 + 7 cut 32 is
        let code = vec![
            Op::Push(0), Op::Store(0),
            Op::Label(0), Op::Load(0), Op::Push(1000), Op::Bin(Lt), Op::Jz(1),
            Op::Load(1), Op::Load(0), Op::Push(3), Op::Bin(Mul), Op::Push(7), Op::Bin(Add), Op::Sext(32), Op::Bin(Add), Op::Sext(32), Op::Store(1),
            Op::Load(0), Op::Push(1), Op::Bin(Add), Op::Sext(32), Op::Store(0),
            Op::Jmp(0), Op::Label(1), Op::Load(1), Op::Ret,
        ];
        let mut f = Func::new("t", 0, 2, code);
        let r = ranges(&f);
        assert_eq!(r[0], R { lo: 0, hi: 1000 });
        assert_eq!(run(&mut f), 2, "{:?}", f.code);
        // the sum's own cut stays
        assert!(f.code.windows(2).any(|w| w == [Op::Bin(Add), Op::Sext(32)]));
    }

    #[test]
    fn a_parameter_or_a_wrapping_value_keeps_its_cut() {
        let mut f = Func::new("t", 1, 1, vec![Op::Load(0), Op::Push(1), Op::Bin(Add), Op::Sext(32), Op::Ret]);
        assert_eq!(run(&mut f), 0);
        // x = x * 3 for ever: anything
        let code = vec![Op::Push(1), Op::Store(0), Op::Label(0), Op::Load(0), Op::Push(3), Op::Bin(Mul), Op::Store(0), Op::Load(0), Op::Sext(32), Op::Jnz(0), Op::Push(0), Op::Ret];
        let mut f = Func::new("t", 0, 1, code);
        assert_eq!(ranges(&f)[0], ALL);
        assert_eq!(run(&mut f), 0);
    }

    /// Random expressions: every value found by evaluating lies in the
    /// interval.
    #[test]
    fn the_interval_holds_every_value() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        fn gen(next: &mut dyn FnMut() -> u64, d: u32) -> E {
            if d == 0 || next() % 4 == 0 {
                return if next() % 3 == 0 { E::Push(next() as i64 >> (next() % 64)) } else { E::Load((next() % 2) as u32) };
            }
            let l = Box::new(gen(next, d - 1));
            let r = Box::new(gen(next, d - 1));
            let k = Box::new(E::Push((next() % 70) as i64 - 3));
            match next() % 12 {
                0 => E::Bin(Add, l, r),
                1 => E::Bin(Sub, l, r),
                2 => E::Bin(Mul, l, r),
                3 => E::Bin(Band, l, r),
                4 => E::Bin(Bxor, l, r),
                5 => E::Bin(Shr, l, k),
                6 => E::Bin(Shl, l, k),
                7 => E::Bin(DivTrunc, l, k),
                8 => E::Bin(ModTrunc, l, k),
                9 => E::Un(UnOp::Bnot, l),
                10 => E::Sext([8, 16, 32][(next() % 3) as usize], l),
                _ => E::Bin(Lt, l, r),
            }
        }
        for _ in 0..2000 {
            let e = gen(&mut next, 4);
            let (lo0, lo1) = ((next() % 2000) as i64 - 1000, (next() % 70000) as i64 - 35000);
            let rs = [R { lo: lo0, hi: lo0 + (next() % 500) as i64 }, R { lo: lo1, hi: lo1 + (next() % 70000) as i64 }];
            let r = range(&e, &|s| rs[s as usize]);
            let ops = super::super::simplify::to_ops(&e);
            for _ in 0..20 {
                let v = [rs[0].lo + (next() % (rs[0].hi - rs[0].lo + 1) as u64) as i64, rs[1].lo + (next() % (rs[1].hi - rs[1].lo + 1) as u64) as i64];
                if let Some(x) = super::super::simplify::eval_ops(&ops, &|_, s| v[s as usize % 2]) {
                    assert!(r.lo <= x && x <= r.hi, "{e:?}: {x} not in {r:?} with {v:?}");
                }
            }
        }
    }
}
