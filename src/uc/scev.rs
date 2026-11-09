//! --release: a loop that only adds up a polynomial of its counter is
//! replaced by the sum's closed form, as clang does from its scalar
//! evolution (SCEV).
//!
//!   for (c = a; c < N; c++) { s = s + P(c); }      deg P <= 3
//!
//! P(a + k) is written in the binomial basis, sum_m b_m C(k, m), and
//! sum_{k<T} C(k, m) = C(T, m + 1), so the loop adds sum_m b_m C(T, m + 1)
//! with T = max(0, N - a) turns. Everything is worked out modulo 2^64, as
//! the loop's own wrapping arithmetic would: the binomials are taken
//! exactly by dividing m! out of their factors before multiplying. The loop
//! may add to several locals; a local that copies the counter at the top of
//! the body (Python's `for i in range(...)`) gets the counter's last value.
//!
//! N known while compiling: any degree up to 3. N in a local the loop never
//! stores (the counter starting at zero or more): degree up to 1, worked
//! out when the program runs. Anything else in the loop, and it stays.

use super::ir::{label_count, BinOp, Func, Op, UnOp};
use std::collections::HashMap;

/// A polynomial in the accumulator (s) and the counter (c): (deg s, deg c)
/// -> coefficient, with wrapping arithmetic.
type Poly = HashMap<(u8, u8), i64>;

fn constant(v: i64) -> Poly {
    let mut p = Poly::new();
    if v != 0 {
        p.insert((0, 0), v);
    }
    p
}

fn add(a: &Poly, b: &Poly, sign: i64) -> Poly {
    let mut p = a.clone();
    for (k, v) in b {
        let e = p.entry(*k).or_insert(0);
        *e = e.wrapping_add(v.wrapping_mul(sign));
    }
    p.retain(|_, v| *v != 0);
    p
}

fn mul(a: &Poly, b: &Poly) -> Option<Poly> {
    let mut p = Poly::new();
    for ((sa, ca), va) in a {
        for ((sb, cb), vb) in b {
            let k = (sa + sb, ca + cb);
            if k.0 > 1 || k.1 > 3 {
                return None;
            }
            let e = p.entry(k).or_insert(0);
            *e = e.wrapping_add(va.wrapping_mul(*vb));
        }
    }
    p.retain(|_, v| *v != 0);
    Some(p)
}

/// The run of operations as a polynomial; `var` says what a load is.
fn poly(run: &[Op], var: &dyn Fn(u32) -> Option<Poly>) -> Option<Poly> {
    let mut st: Vec<Poly> = Vec::new();
    for op in run {
        match op {
            Op::Push(v) => st.push(constant(*v)),
            Op::Load(x) => st.push(var(*x)?),
            Op::Un(UnOp::Neg) => {
                let a = st.pop()?;
                st.push(add(&Poly::new(), &a, -1));
            }
            Op::Un(UnOp::Plus) => {}
            Op::Un(UnOp::Bnot) => {
                // ~x = -x - 1
                let a = st.pop()?;
                st.push(add(&add(&Poly::new(), &a, -1), &constant(-1), 1));
            }
            Op::Bin(b) => {
                let r = st.pop()?;
                let l = st.pop()?;
                st.push(match b {
                    BinOp::Add => add(&l, &r, 1),
                    BinOp::Sub => add(&l, &r, -1),
                    BinOp::Mul => mul(&l, &r)?,
                    BinOp::Shl => {
                        // only by a constant: a multiplication by 2^(k mod 64)
                        if r.keys().any(|k| *k != (0, 0)) {
                            return None;
                        }
                        let k = r.get(&(0, 0)).copied().unwrap_or(0) as u32 & 63;
                        mul(&l, &constant(1i64.wrapping_shl(k)))?
                    }
                    _ => return None,
                });
            }
            _ => return None,
        }
    }
    if st.len() == 1 {
        st.pop()
    } else {
        None
    }
}

/// C(t, m) modulo 2^64, exactly: m! divided out of the factors first.
pub fn binomial(t: u64, m: u32) -> u64 {
    if (t as u128) < m as u128 {
        return 0;
    }
    let mut f: Vec<u64> = (0..m as u64).map(|i| t - i).collect();
    // m! = 2^a 3^b (m <= 4)
    let (twos, threes) = match m {
        0 | 1 => (0, 0),
        2 => (1, 0),
        3 => (1, 1),
        4 => (3, 1),
        _ => unreachable!("degree up to 3"),
    };
    for (p, times) in [(2u64, twos), (3, threes)] {
        for _ in 0..times {
            let i = f.iter().position(|x| x % p == 0).expect("m consecutive numbers hold m!");
            f[i] /= p;
        }
    }
    f.iter().fold(1u64, |a, x| a.wrapping_mul(*x))
}

/// P(a + k) in the binomial basis of k: [b0, b1, b2, b3].
fn shifted_binomial(p: &[i64; 4], a: i64) -> [i64; 4] {
    // q(k) = p(a + k), in powers of k
    let mut q = [0i64; 4];
    let choose = [[1, 0, 0, 0], [1, 1, 0, 0], [1, 2, 1, 0], [1, 3, 3, 1]];
    for d in 0..4 {
        for e in 0..=d {
            let apow = (0..d - e).fold(1i64, |x, _| x.wrapping_mul(a));
            q[e] = q[e].wrapping_add(p[d].wrapping_mul(choose[d][e]).wrapping_mul(apow));
        }
    }
    // k = C1, k^2 = C1 + 2 C2, k^3 = C1 + 6 C2 + 6 C3
    [
        q[0],
        q[1].wrapping_add(q[2]).wrapping_add(q[3]),
        q[2].wrapping_mul(2).wrapping_add(q[3].wrapping_mul(6)),
        q[3].wrapping_mul(6),
    ]
}

/// The sum over t turns, from a.
pub fn sum(p: &[i64; 4], a: i64, t: u64) -> i64 {
    let b = shifted_binomial(p, a);
    (0..4).fold(0i64, |s, m| s.wrapping_add(b[m].wrapping_mul(binomial(t, m as u32 + 1) as i64)))
}

/// The constant last stored in a slot on the straight way into `at`.
fn known_before(code: &[Op], at: usize, slot: u32) -> Option<i64> {
    let mut i = at;
    while i > 0 {
        i -= 1;
        match &code[i] {
            Op::Store(x) if *x == slot => {
                return match code.get(i.wrapping_sub(1)) {
                    Some(Op::Push(v)) if i > 0 => Some(*v),
                    _ => None,
                };
            }
            Op::Label(_) | Op::Jmp(_) | Op::Jz(_) | Op::Jnz(_) | Op::Ret | Op::Call(..) => return None,
            _ => {}
        }
    }
    None
}

struct Found {
    t: usize,
    j: usize,
    counter: u32,
    from: i64,
    /// Ok: known; Err: the slot that holds it
    bound: Result<i64, u32>,
    inclusive: bool,
    sums: Vec<(u32, [i64; 4])>,
    copy: Option<u32>,
}

fn find(f: &Func) -> Option<Found> {
    let code = &f.code;
    let starts = super::opt::starts(code);
    for j in 0..code.len().saturating_sub(1) {
        let (Op::Jmp(top), Op::Label(end)) = (&code[j], &code[j + 1]) else { continue };
        let Some(t) = code[..j].iter().rposition(|o| *o == Op::Label(*top)) else { continue };
        if t + 5 > j || j < 4 {
            continue;
        }
        // top: Load c; bound; Lt|Le; Jz end
        let (Op::Load(c), bound_op, Op::Bin(cmp @ (BinOp::Lt | BinOp::Le)), Op::Jz(e)) = (&code[t + 1], &code[t + 2], &code[t + 3], &code[t + 4]) else {
            continue;
        };
        if e != end {
            continue;
        }
        // bottom: Load c; one; Add; Store c
        let (Op::Load(c2), one, Op::Bin(BinOp::Add), Op::Store(c3)) = (&code[j - 4], &code[j - 3], &code[j - 2], &code[j - 1]) else {
            continue;
        };
        if c2 != c || c3 != c || j - 4 < t + 5 {
            continue;
        }
        let stored: Vec<u32> = code[t..j].iter().filter_map(|o| if let Op::Store(x) = o { Some(*x) } else { None }).collect();
        let invariant = |x: u32| !stored.contains(&x);
        let step_one = match one {
            Op::Push(1) => true,
            Op::Load(by) => invariant(*by) && known_before(code, t, *by) == Some(1),
            _ => false,
        };
        if !step_one {
            continue;
        }
        let bound = match bound_op {
            Op::Push(n) => Ok(*n),
            Op::Load(n) if invariant(*n) && *n != *c => known_before(code, t, *n).ok_or(*n),
            _ => continue,
        };
        let Some(from) = known_before(code, t, *c) else { continue };
        // nothing jumps into the loop from outside, or out of it but its test
        let inside = |l: u32| code[t..=j].contains(&Op::Label(l));
        let jumps_ok = code.iter().enumerate().all(|(k, o)| match o {
            Op::Jmp(l) | Op::Jz(l) | Op::Jnz(l) => (k == j && l == top) || (k == t + 4) || !inside(*l),
            _ => true,
        });
        if !jumps_ok {
            continue;
        }
        // the body: a copy of the counter, then sums
        let mut k = t + 5;
        let mut copy = None;
        if let (Some(Op::Load(x)), Some(Op::Store(y))) = (code.get(k), code.get(k + 1)) {
            if x == c && stored.iter().filter(|s| **s == *y).count() == 1 {
                copy = Some(*y);
                k += 2;
            }
        }
        let mut sums: Vec<(u32, [i64; 4])> = Vec::new();
        let mut ok = true;
        while k < j - 4 {
            match &code[k] {
                Op::Label(_) => {
                    k += 1;
                    continue;
                }
                Op::Store(_) => {
                    ok = false;
                    break;
                }
                _ => {}
            }
            // a statement: the run that ends at the next Store
            let Some(e) = (k..j - 4).find(|&i| matches!(code[i], Op::Store(_))) else {
                ok = false;
                break;
            };
            let Op::Store(s) = code[e] else { unreachable!() };
            if starts[e - 1] != Some(k) || s == *c || Some(s) == copy || stored.iter().filter(|x| **x == s).count() != 1 {
                ok = false;
                break;
            }
            let var = |x: u32| -> Option<Poly> {
                if x == *c || Some(x) == copy {
                    Some([((0u8, 1u8), 1i64)].into_iter().collect())
                } else if x == s {
                    Some([((1u8, 0u8), 1i64)].into_iter().collect())
                } else {
                    None
                }
            };
            let Some(p) = poly(&code[k..e], &var) else {
                ok = false;
                break;
            };
            // s + P(c): s once, alone
            if p.get(&(1, 0)) != Some(&1) || p.keys().any(|(ds, dc)| *ds == 1 && *dc > 0) {
                ok = false;
                break;
            }
            let mut coeffs = [0i64; 4];
            for ((ds, dc), v) in &p {
                if *ds == 0 {
                    coeffs[*dc as usize] = *v;
                }
            }
            sums.push((s, coeffs));
            k = e + 1;
        }
        if !ok || sums.is_empty() {
            continue;
        }
        if bound.is_err() && (from < 0 || *cmp == BinOp::Le || sums.iter().any(|(_, p)| p[2] != 0 || p[3] != 0)) {
            continue;
        }
        return Some(Found { t, j, counter: *c, from, bound, inclusive: *cmp == BinOp::Le, sums, copy });
    }
    None
}

/// Replaces every such loop; returns how many.
pub fn run(f: &mut Func) -> usize {
    let mut replaced = 0;
    while let Some(l) = find(f) {
        let mut code = Vec::new();
        match l.bound {
            Ok(n) => {
                let turns = (n as i128 - l.from as i128 + if l.inclusive { 1 } else { 0 }).max(0) as u64;
                for (s, p) in &l.sums {
                    code.extend([Op::Load(*s), Op::Push(sum(p, l.from, turns)), Op::Bin(BinOp::Add), Op::Store(*s)]);
                }
                code.extend([Op::Push(l.from.wrapping_add(turns as i64)), Op::Store(l.counter)]);
                if let (Some(x), true) = (l.copy, turns > 0) {
                    code.extend([Op::Push(l.from.wrapping_add(turns as i64 - 1)), Op::Store(x)]);
                }
            }
            Err(n) => {
                // t = n < a ? 0 : n - a; then sum b0 t + b1 C(t, 2), with
                // C(t, 2) = (t >> 1)(t - 1) + (t & 1)((t - 1) >> 1) for t >= 0
                let t = f.slots;
                f.slots += 1;
                let mut labels = label_count(&f.code);
                let (skip, none) = (labels, labels + 1);
                labels += 2;
                let _ = labels;
                let a = l.from;
                code.extend([Op::Push(0), Op::Store(t), Op::Load(n), Op::Push(a), Op::Bin(BinOp::Lt), Op::Jnz(skip)]);
                code.extend([Op::Load(n), Op::Push(a), Op::Bin(BinOp::Sub), Op::Store(t), Op::Label(skip)]);
                let c2 = [
                    Op::Load(t),
                    Op::Push(1),
                    Op::Bin(BinOp::Shr),
                    Op::Load(t),
                    Op::Push(1),
                    Op::Bin(BinOp::Sub),
                    Op::Bin(BinOp::Mul),
                    Op::Load(t),
                    Op::Push(1),
                    Op::Bin(BinOp::Band),
                    Op::Load(t),
                    Op::Push(1),
                    Op::Bin(BinOp::Sub),
                    Op::Push(1),
                    Op::Bin(BinOp::Shr),
                    Op::Bin(BinOp::Mul),
                    Op::Bin(BinOp::Add),
                ];
                for (s, p) in &l.sums {
                    let b = shifted_binomial(p, a);
                    code.extend([Op::Load(*s), Op::Push(b[0]), Op::Load(t), Op::Bin(BinOp::Mul), Op::Bin(BinOp::Add)]);
                    code.extend(c2.iter().cloned());
                    code.extend([Op::Push(b[1]), Op::Bin(BinOp::Mul), Op::Bin(BinOp::Add), Op::Store(*s)]);
                }
                code.extend([Op::Push(a), Op::Load(t), Op::Bin(BinOp::Add), Op::Store(l.counter)]);
                if let Some(x) = l.copy {
                    code.extend([Op::Load(t), Op::Jz(none), Op::Push(a), Op::Load(t), Op::Bin(BinOp::Add), Op::Push(1), Op::Bin(BinOp::Sub), Op::Store(x), Op::Label(none)]);
                }
            }
        }
        let mut out = Vec::with_capacity(f.code.len());
        out.extend_from_slice(&f.code[..l.t]);
        out.extend(code);
        out.extend_from_slice(&f.code[l.j + 1..]); // from the label after the loop
        f.code = out;
        replaced += 1;
    }
    replaced
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binomials_are_exact() {
        // the exact product must fit 128 bits: t below 2^31 for four factors
        for t in [0u64, 1, 2, 3, 4, 5, 10, 1000, 1 << 20, (1 << 31) - 3, (1 << 30) + 7] {
            for m in 0..=4u32 {
                let exact = (0..m as u128).fold(1u128, |a, i| a * (t as u128).saturating_sub(i)) / (1..=m as u128).product::<u128>().max(1);
                assert_eq!(binomial(t, m), exact as u64, "C({t}, {m})");
            }
        }
    }

    #[test]
    fn the_closed_form_is_the_loop_modulo_2_64() {
        let mut x: u64 = 0x1234_5678_9ABC_DEF1;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as i64
        };
        for _ in 0..300 {
            let p = [next() >> (next() as u64 % 64), next() >> (next() as u64 % 64), next() % 1000, next() >> (next() as u64 % 64)];
            let a = next() % 50;
            let t = (next() as u64) % 2000;
            let mut s = 0i64;
            for k in 0..t as i64 {
                let c = a.wrapping_add(k);
                let v = p[0].wrapping_add(p[1].wrapping_mul(c)).wrapping_add(p[2].wrapping_mul(c).wrapping_mul(c)).wrapping_add(p[3].wrapping_mul(c).wrapping_mul(c).wrapping_mul(c));
                s = s.wrapping_add(v);
            }
            assert_eq!(sum(&p, a, t), s, "p {p:?} a {a} t {t}");
        }
    }
}
