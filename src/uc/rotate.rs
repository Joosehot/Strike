//! --release: a loop's test moved to its bottom, as clang does, so it goes
//! round with one jump less every time.
//!
//!   top: test; Jz end; body; Jmp top; end:
//!     becomes
//!   test; Jz end; again: body; top: test; Jnz again; end:
//!
//! Only a plain test is copied (loads, constants and operations: no call,
//! no store, no jump), so working it out once more changes nothing, and no
//! division by a variable, which could fail where it did not. `top` stays
//! where the test is, so a `continue` that jumps there still tests first.

use super::ir::{label_count, BinOp, Func, Op};

const MAX_TEST: usize = 16;
const MAX_BODY: usize = 40;

/// Rotates the function's loops; returns how many.
pub fn run(f: &mut Func) -> usize {
    let mut rotated = 0;
    loop {
        let code = &f.code;
        let mut found = None;
        for j in 0..code.len().saturating_sub(1) {
            let (Op::Jmp(top), Op::Label(end)) = (&code[j], &code[j + 1]) else { continue };
            let Some(t) = code[..j].iter().rposition(|o| *o == Op::Label(*top)) else { continue };
            let Some(k) = code[t + 1..j].iter().position(|o| !matches!(o, Op::Push(_) | Op::Load(_) | Op::Bin(_) | Op::Un(_))) else {
                continue;
            };
            let z = t + 1 + k;
            let test = &code[t + 1..z];
            let divides = test.iter().any(|o| matches!(o, Op::Bin(BinOp::DivTrunc | BinOp::ModTrunc | BinOp::DivFloor | BinOp::ModFloor)));
            if code[z] != Op::Jz(*end) || k == 0 || k > MAX_TEST || divides {
                continue;
            }
            found = Some((t, z, j, *top, *end));
            break;
        }
        let Some((t, z, j, top, end)) = found else { return rotated };
        let again = label_count(&f.code);
        let test: Vec<Op> = f.code[t + 1..z].to_vec();
        let body: Vec<Op> = f.code[z + 1..j].to_vec();
        // a short body with no jump in it goes round twice per jump back
        // (as clang unrolls): body, test, body, then the test at the bottom
        let twice = body.len() <= MAX_BODY && !body.iter().any(|o| matches!(o, Op::Label(_) | Op::Jmp(_) | Op::Jz(_) | Op::Jnz(_) | Op::Ret));
        let mut out = Vec::with_capacity(f.code.len() + 2 * test.len() + body.len() + 4);
        out.extend_from_slice(&f.code[..t]);
        out.extend(test.iter().cloned());
        out.push(Op::Jz(end));
        out.push(Op::Label(again));
        out.extend(body.iter().cloned());
        if twice {
            out.extend(test.iter().cloned());
            out.push(Op::Jz(end));
            out.extend(body);
        }
        out.push(Op::Label(top));
        out.extend(test);
        out.push(Op::Jnz(again));
        out.extend_from_slice(&f.code[j + 1..]);
        f.code = out;
        rotated += 1;
    }
}
