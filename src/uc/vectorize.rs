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

pub fn run(f: &mut Func) -> usize {
    if std::env::var_os("STRIKE_DUMP_VEC").is_some() {
        eprintln!("{} ({} params, {} slots) before vectorizing:", f.name, f.params, f.slots);
        for (i, op) in f.code.iter().enumerate() {
            eprintln!("  {i:4} {op:?}");
        }
    }
    let _ = Op::Ret;
    0
}
