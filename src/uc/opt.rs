//! --release: two of the optimizations clang -O2 makes, on Strike IR.
//!
//! - Algebra (clang's instcombine): `x + 0`, `x * 1`, `x | 0` are `x`;
//!   `x * 2^k` is `x << k`; a floor division by 2^k (Python's //) is
//!   `x >> k`, and its remainder `x & (2^k - 1)`.
//! - Loop-invariant code motion (LICM): an expression in a loop whose
//!   locals the loop never stores is worked out once, before the loop, into
//!   a new slot. Only an expression that cannot fail is moved (no division
//!   by a variable), since the loop may not run at all, and only out of a
//!   loop entered by falling into its top.
//!
//! The IR is a stack code made from expression trees, so within a stretch
//! with no label or jump every value's expression is a contiguous run of
//! operations, found by following the stack.

use super::ir::{label_count, BinOp, Func, Op, UnOp};

/// For every operation that leaves a value, where the run that makes that
/// value begins; None where it is not a plain expression (a label, a jump,
/// a call or a Dup is on the way).
pub(super) fn starts(code: &[Op]) -> Vec<Option<usize>> {
    let mut at = vec![None; code.len()];
    let mut st: Vec<Option<usize>> = Vec::new();
    for (i, op) in code.iter().enumerate() {
        let pop = |st: &mut Vec<Option<usize>>| st.pop().flatten();
        match op {
            // a global's or the frame's address is a constant of the function
            Op::Push(_) | Op::Load(_) | Op::AddrG(_) | Op::Frame(_) => {
                st.push(Some(i));
                at[i] = Some(i);
            }
            Op::Un(_) | Op::Sext(_) => {
                let s = pop(&mut st);
                st.push(s);
                at[i] = s;
            }
            Op::Bin(_) => {
                let r = pop(&mut st);
                let l = pop(&mut st);
                let s = if r.is_some() { l } else { None };
                st.push(s);
                at[i] = s;
            }
            Op::Call(_, n) => {
                for _ in 0..*n {
                    st.pop();
                }
                st.push(None);
            }
            // what memory holds is not a plain expression: a store may
            // change it
            Op::LoadM(_) => {
                st.pop();
                st.push(None);
            }
            Op::StoreM(_) | Op::OpM(..) | Op::PrintMem => {
                st.pop();
                st.pop();
            }
            Op::Dup | Op::LoadG(_) | Op::ReadLine | Op::Data(_) => st.push(None),
            Op::Fail(..) | Op::FailMem(_) => st.iter_mut().for_each(|x| *x = None),
            Op::ReadToken => {
                st.push(None);
                st.push(None);
            }
            Op::Store(_) | Op::StoreG(_) | Op::Pop | Op::PrintInt => {
                st.pop();
            }
            Op::PrintStr(_) => {}
            Op::Jz(_) | Op::Jnz(_) => {
                st.pop();
                st.iter_mut().for_each(|x| *x = None);
            }
            Op::Jmp(_) | Op::Label(_) | Op::Ret => {
                if matches!(op, Op::Ret) {
                    st.pop();
                }
                st.iter_mut().for_each(|x| *x = None);
            }
        }
    }
    at
}

/// The algebra; returns how many operations it changed.
pub fn simplify(f: &mut Func) -> usize {
    let mut changed = 0;
    let mut out: Vec<Op> = Vec::with_capacity(f.code.len());
    for op in std::mem::take(&mut f.code) {
        let c = match (out.last(), &op) {
            (Some(Op::Push(c)), Op::Bin(_)) => Some(*c),
            _ => None,
        };
        let (Some(c), Op::Bin(b)) = (c, &op) else {
            out.push(op);
            continue;
        };
        let pow = (c > 0 && c & (c - 1) == 0).then(|| c.trailing_zeros() as i64);
        use BinOp::*;
        let with: Option<Vec<Op>> = match (b, c, pow) {
            (Add | Sub | Bor | Bxor | Shl | Shr, 0, _) => Some(vec![]),
            (Mul | DivTrunc | DivFloor, 1, _) => Some(vec![]),
            (Band, -1, _) => Some(vec![]),
            (Mul, _, Some(k)) => Some(vec![Op::Push(k), Op::Bin(Shl)]),
            (DivFloor, _, Some(k)) => Some(vec![Op::Push(k), Op::Bin(Shr)]),
            (ModFloor, _, Some(_)) => Some(vec![Op::Push(c - 1), Op::Bin(Band)]),
            _ => None,
        };
        match with {
            Some(w) => {
                out.pop();
                out.extend(w);
                changed += 1;
            }
            None => out.push(op),
        }
    }
    f.code = out;
    changed
}

/// Can this run of operations never fail? (Only a division can: by zero,
/// or the lowest number by -1.)
fn cannot_fail(run: &[Op]) -> bool {
    run.iter().enumerate().all(|(i, op)| match op {
        // a checked operation stops the program where a number would not fit
        Op::Bin(BinOp::AddOv | BinOp::SubOv | BinOp::MulOv | BinOp::ShlOv) | Op::Un(UnOp::NegOv) => false,
        Op::Bin(BinOp::DivTrunc | BinOp::ModTrunc | BinOp::DivFloor | BinOp::ModFloor) => {
            matches!(run.get(i.wrapping_sub(1)), Some(Op::Push(d)) if *d != 0 && *d != -1)
        }
        _ => true,
    })
}

/// LICM; returns how many expressions it moved.
pub fn hoist(f: &mut Func) -> usize {
    let mut moved = 0;
    'again: loop {
        let code = &f.code;
        let st = starts(code);
        let mut label_at = vec![usize::MAX; label_count(code) as usize];
        for (i, op) in code.iter().enumerate() {
            if let Op::Label(l) = op {
                label_at[*l as usize] = i;
            }
        }
        // innermost loops first: the back jump closest after its top
        let mut loops: Vec<(usize, usize, u32)> = Vec::new();
        for (j, op) in code.iter().enumerate() {
            if let Op::Jmp(l) = op {
                let t = label_at[*l as usize];
                if t < j {
                    loops.push((t, j, *l));
                }
            }
        }
        loops.sort_by_key(|&(t, j, _)| j - t);
        for (t, j, top) in loops {
            // entered only by falling into the top; every other jump to it
            // comes from inside
            if t == 0 || matches!(code[t - 1], Op::Jmp(_) | Op::Ret) {
                continue;
            }
            let outside = code.iter().enumerate().any(|(k, o)| matches!(o, Op::Jmp(l) | Op::Jz(l) | Op::Jnz(l) if *l == top) && (k < t || k > j));
            if outside {
                continue;
            }
            let stored: Vec<u32> = code[t..=j].iter().filter_map(|o| if let Op::Store(s) = o { Some(*s) } else { None }).collect();
            // the largest invariant expression in the loop (one that holds
            // a smaller one is larger, so the smaller goes with it)
            let mut best: Option<(usize, usize)> = None;
            for i in t + 1..j {
                let Some(s) = st[i] else { continue };
                if s <= t || !matches!(code[i], Op::Bin(_) | Op::Un(_) | Op::Sext(_) | Op::AddrG(_) | Op::Frame(_)) {
                    continue;
                }
                let run = &code[s..=i];
                let plain = run.iter().all(|o| matches!(o, Op::Push(_) | Op::Load(_) | Op::Bin(_) | Op::Un(_) | Op::Sext(_) | Op::AddrG(_) | Op::Frame(_)));
                let invariant = run.iter().all(|o| !matches!(o, Op::Load(x) if stored.contains(x)));
                // a row's address (a global array plus a row offset) is worth a register
                let has_load = run.iter().any(|o| matches!(o, Op::Load(_) | Op::AddrG(_) | Op::Frame(_)));
                if plain && invariant && has_load && cannot_fail(run) && best.is_none_or(|(bs, bi)| i - s > bi - bs) {
                    best = Some((s, i));
                }
            }
            let Some((s, i)) = best else { continue };
            let tmp = f.slots;
            f.slots += 1;
            let run: Vec<Op> = f.code[s..=i].to_vec();
            let mut out = Vec::with_capacity(f.code.len() + 2);
            out.extend_from_slice(&f.code[..t]);
            out.extend(run);
            out.push(Op::Store(tmp));
            out.extend_from_slice(&f.code[t..s]);
            out.push(Op::Load(tmp));
            out.extend_from_slice(&f.code[i + 1..]);
            f.code = out;
            moved += 1;
            continue 'again;
        }
        return moved;
    }
}

/// The locals that only ever hold numbers that fit 32 bits: every store
/// into one is of a value cut to 32 bits or less, a constant that fits, a
/// comparison, a narrow element, or another such local. Parameters are not
/// known to be.
fn narrow32(f: &Func) -> Vec<bool> {
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
                Some(Op::Bin(b)) => b.is_comparison(),
                _ => false,
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

/// A local every store of which gives it the same constant (C's
/// `int n = 300`) is read as that constant: its loops' counts become known.
/// (Before its first store a C local holds nothing it may be read for, and
/// Go and Python never read a variable before its first value.) Returns
/// how many loads it replaced.
pub fn constants(f: &mut Func) -> usize {
    let mut value: Vec<Option<Option<i64>>> = vec![None; f.slots as usize];
    for (k, op) in f.code.iter().enumerate() {
        let Op::Store(s) = op else { continue };
        let c = match k.checked_sub(1).map(|p| &f.code[p]) {
            Some(Op::Push(c)) => Some(*c),
            _ => None,
        };
        let v = &mut value[*s as usize];
        *v = match (*v, c) {
            (None, c) => Some(c),
            (Some(Some(a)), Some(b)) if a == b => Some(Some(a)),
            _ => Some(None),
        };
    }
    // a local starts at zero: a constant other than zero counts only where
    // it is stored before anything reads the local, on the straight way
    // from the start (nothing jumps there, so every other load comes later)
    let mut first = vec![false; f.slots as usize];
    let mut read = vec![false; f.slots as usize];
    for (k, op) in f.code.iter().enumerate() {
        match op {
            Op::Load(s) => read[*s as usize] = true,
            Op::Store(s) if !read[*s as usize] && matches!(k.checked_sub(1).map(|p| &f.code[p]), Some(Op::Push(_))) => first[*s as usize] = true,
            Op::Label(_) | Op::Jmp(_) | Op::Jz(_) | Op::Jnz(_) | Op::Ret | Op::Fail(..) => break,
            _ => {}
        }
    }
    let mut n = 0;
    for op in f.code.iter_mut() {
        if let Op::Load(s) = op {
            if *s >= f.params {
                if let Some(Some(c)) = value[*s as usize].filter(|v| *v == Some(0) || first[*s as usize]) {
                    *op = Op::Push(c);
                    n += 1;
                }
            }
        }
    }
    n
}

/// The largest constant a local is ever given, where every store into it
/// is of a constant (a C `int n = 20000000` used as a bound).
fn most(f: &Func, s: u32) -> Option<i64> {
    if s < f.params {
        return None;
    }
    let mut m: Option<i64> = None;
    for (k, op) in f.code.iter().enumerate() {
        if *op == Op::Store(s) {
            let Some(Op::Push(c)) = k.checked_sub(1).map(|p| &f.code[p]) else { return None };
            m = Some(m.map_or(*c, |x| x.max(*c)));
        }
    }
    // never stored: it holds zero
    Some(m.unwrap_or(0))
}

/// A C int counter (`for (int i = 0; i < n; i++)`) is cut back to 32 bits
/// after every `i + 1`; where the loop has just found i below a number
/// that fits 32 bits, i + 1 fits too, and the cut changes nothing. Such a
/// cut is left out (clang leaves it out because a signed overflow cannot
/// happen; here it is shown not to). Returns how many.
pub fn counters(f: &mut Func) -> usize {
    let narrow = narrow32(f);
    let code = &f.code;
    let mut label_at = vec![usize::MAX; label_count(code) as usize];
    for (i, op) in code.iter().enumerate() {
        if let Op::Label(l) = op {
            label_at[*l as usize] = i;
        }
    }
    let mut cut = Vec::new();
    for (j, op) in code.iter().enumerate() {
        let Op::Jmp(top) = op else { continue };
        let t = label_at[*top as usize];
        if t >= j || j < 5 || t + 5 > j {
            continue;
        }
        // top: Load c; a bound that fits 32 bits; Lt (or Le with a bound
        // below the largest int); Jz end
        let (Op::Load(c), bound, Op::Bin(cmp @ (BinOp::Lt | BinOp::Le)), Op::Jz(_)) = (&code[t + 1], &code[t + 2], &code[t + 3], &code[t + 4]) else { continue };
        let fits = match (cmp, bound) {
            (BinOp::Lt, Op::Push(n)) => i32::try_from(*n).is_ok(),
            (BinOp::Lt, Op::Load(n)) => narrow[*n as usize],
            (_, Op::Push(n)) => (i32::MIN as i64..i32::MAX as i64).contains(n),
            (_, Op::Load(n)) => most(f, *n).is_some_and(|m| m < i32::MAX as i64) && narrow[*n as usize],
            _ => false,
        };
        // bottom: Load c; Push 1; Add; Sext 32; Store c
        if !fits || !narrow[*c as usize] || code[j - 5..j] != [Op::Load(*c), Op::Push(1), Op::Bin(BinOp::Add), Op::Sext(32), Op::Store(*c)] {
            continue;
        }
        // c is changed only there, and the loop is entered only by its top
        let stores = code[t..j].iter().filter(|o| **o == Op::Store(*c)).count();
        let inside = |l: u32| (t..=j).contains(&label_at[l as usize]);
        let entered = code.iter().enumerate().any(|(k, o)| matches!(o, Op::Jmp(l) | Op::Jz(l) | Op::Jnz(l) if inside(*l) && *l != *top && !(t..=j).contains(&k)));
        if stores == 1 && !entered {
            cut.push(j - 2);
        }
    }
    for &k in cut.iter().rev() {
        f.code.remove(k);
    }
    cut.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn func(code: Vec<Op>, slots: u32) -> Func {
        Func::new("t", 0, slots, code)
    }

    #[test]
    fn an_invariant_product_leaves_the_loop() {
        // s = 0; top: if i < 10 { s = s + a * b; i = i + 1; goto top }
        let mut f = func(
            vec![
                Op::Label(0),
                Op::Load(0),
                Op::Push(10),
                Op::Bin(BinOp::Lt),
                Op::Jz(1),
                Op::Load(1),
                Op::Load(2),
                Op::Load(3),
                Op::Bin(BinOp::Mul),
                Op::Bin(BinOp::Add),
                Op::Store(1),
                Op::Load(0),
                Op::Push(1),
                Op::Bin(BinOp::Add),
                Op::Store(0),
                Op::Jmp(0),
                Op::Label(1),
                Op::Load(1),
                Op::Ret,
            ],
            4,
        );
        f.code.insert(0, Op::Store(3));
        f.code.insert(0, Op::Push(7));
        assert_eq!(hoist(&mut f), 1);
        assert_eq!(&f.code[2..6], &[Op::Load(2), Op::Load(3), Op::Bin(BinOp::Mul), Op::Store(4)]);
        assert!(f.code.contains(&Op::Load(4)));
    }

    #[test]
    fn a_division_by_a_variable_stays_in_the_loop() {
        let mut f = func(
            vec![
                Op::Push(0),
                Op::Store(0),
                Op::Label(0),
                Op::Load(0),
                Op::Jz(1),
                Op::Load(2),
                Op::Load(3),
                Op::Bin(BinOp::DivTrunc),
                Op::Store(1),
                Op::Jmp(0),
                Op::Label(1),
                Op::Push(0),
                Op::Ret,
            ],
            4,
        );
        assert_eq!(hoist(&mut f), 0);
    }
}
