//! --release: constant expressions worked out while compiling, by the same
//! rules the machine follows (wrapping arithmetic, a shift's count taken
//! modulo 64). A division by zero, or the lowest number by -1, is left for
//! the program to meet as it would.

use super::ir::{BinOp, Func, Op, UnOp};

fn bin(op: BinOp, a: i64, b: i64) -> Option<i64> {
    use BinOp::*;
    // a checked operation is folded only where it fits; else the program
    // must stop there when it runs
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
        Band => a & b,
        Bor => a | b,
        Bxor => a ^ b,
        Shl => a.wrapping_shl(b as u32 & 63),
        Shr => a.wrapping_shr(b as u32 & 63),
        _ => return None,
    })
}

/// Folds the function's constants; returns how many operations went away.
pub fn run(f: &mut Func) -> usize {
    let before = f.code.len();
    let mut out: Vec<Op> = Vec::with_capacity(before);
    for op in std::mem::take(&mut f.code) {
        out.push(op);
        loop {
            let n = out.len();
            let folded: Option<(usize, Vec<Op>)> = match &out[..] {
                [.., Op::Push(a), Op::Push(b), Op::Bin(op)] => bin(*op, *a, *b).map(|v| (3, vec![Op::Push(v)])),
                [.., Op::Push(a), Op::Un(u)] => match u {
                    UnOp::Neg => Some(a.wrapping_neg()),
                    UnOp::Plus => Some(*a),
                    UnOp::Not => Some((*a == 0) as i64),
                    UnOp::Bnot => Some(!*a),
                    UnOp::NegOv => a.checked_neg(),
                }
                .map(|v| (2, vec![Op::Push(v)])),
                [.., Op::Push(a), Op::Jz(l)] => Some((2, if *a == 0 { vec![Op::Jmp(*l)] } else { vec![] })),
                [.., Op::Push(a), Op::Jnz(l)] => Some((2, if *a != 0 { vec![Op::Jmp(*l)] } else { vec![] })),
                [.., Op::Push(_), Op::Pop] => Some((2, vec![])),
                [.., Op::Push(a), Op::Sext(n)] => Some((2, vec![Op::Push(super::ir::sext(*a, *n))])),
                _ => None,
            };
            match folded {
                Some((k, with)) => {
                    out.truncate(n - k);
                    out.extend(with);
                }
                None => break,
            }
        }
    }
    f.code = out;
    before - f.code.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_division_is_pythons() {
        for (a, b) in [(-7, 2), (7, -2), (-7, -2), (7, 2), (6, -3), (-6, 3)] {
            let q = bin(BinOp::DivFloor, a, b).unwrap();
            let r = bin(BinOp::ModFloor, a, b).unwrap();
            assert_eq!(q * b + r, a, "{a} {b}");
            assert!(r == 0 || (r < 0) == (b < 0), "{a} {b}");
        }
        assert_eq!(bin(BinOp::DivFloor, -7, 2), Some(-4));
        assert_eq!(bin(BinOp::DivTrunc, 1, 0), None);
    }
}
