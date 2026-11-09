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
/// the most operations four copies of a counted loop's body may take
const MAX_UNROLLED: usize = 100;

/// How many times a loop goes round when that is known: its test is
/// `Load c; Push n; Lt`, c is set to a constant just before its top, and
/// the body changes c only by `c = c + 1` at its end (cut to 32 bits or
/// not: the count fits).
fn counted(code: &[Op], t: usize, test: &[Op], body: &[Op]) -> Option<i64> {
    let [Op::Load(c), Op::Push(n), Op::Bin(BinOp::Lt)] = test else { return None };
    // the constant last stored in c on the straight way into the loop
    let mut k = t;
    let s = loop {
        k = k.checked_sub(1)?;
        match &code[k] {
            Op::Store(x) if x == c => match code.get(k.wrapping_sub(1)) {
                Some(Op::Push(v)) if k > 0 => break v,
                _ => return None,
            },
            Op::Label(_) | Op::Jmp(_) | Op::Jz(_) | Op::Jnz(_) | Op::Ret | Op::Call(..) | Op::Fail(..) => return None,
            _ => {}
        }
    };
    let tail: &[Op] = match body {
        [.., Op::Load(a), Op::Push(1), Op::Bin(BinOp::Add), Op::Sext(32), Op::Store(b)] if a == c && b == c => &body[body.len() - 5..],
        [.., Op::Load(a), Op::Push(1), Op::Bin(BinOp::Add), Op::Store(b)] if a == c && b == c => &body[body.len() - 4..],
        _ => return None,
    };
    let stores = body.iter().filter(|o| **o == Op::Store(*c)).count();
    // with a cut, c must not pass 2^31 - 1 on its way: n fits 32 bits
    let cut = tail.len() == 5;
    if stores != 1 || (cut && (i32::try_from(*n).is_err() || i32::try_from(*s).is_err())) {
        return None;
    }
    let turns = n.checked_sub(*s)?;
    (turns > 0).then_some(turns)
}

/// Rotates the function's loops; returns how many.
pub fn run(f: &mut Func) -> usize {
    let mut rotated = 0;
    loop {
        let code = &f.code;
        let mut found = None;
        for j in 0..code.len().saturating_sub(1) {
            let (Op::Jmp(top), Op::Label(end)) = (&code[j], &code[j + 1]) else { continue };
            let Some(t) = code[..j].iter().rposition(|o| *o == Op::Label(*top)) else { continue };
            let Some(k) = code[t + 1..j].iter().position(|o| !matches!(o, Op::Push(_) | Op::Load(_) | Op::Bin(_) | Op::Un(_) | Op::Sext(_))) else {
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
        // a counted loop (c from a constant, c < n, c + 1 only at the
        // bottom) whose count the copies divide: the copies need no test
        // between them
        let exact = twice.then(|| counted(&f.code, t, &test, &body)).flatten().map(|turns| {
            if turns % 4 == 0 && body.len() * 4 <= MAX_UNROLLED {
                4
            } else if turns % 2 == 0 {
                2
            } else {
                1
            }
        });
        let mut out = Vec::with_capacity(f.code.len() + 2 * test.len() + 4 * body.len() + 4);
        out.extend_from_slice(&f.code[..t]);
        out.extend(test.iter().cloned());
        out.push(Op::Jz(end));
        out.push(Op::Label(again));
        out.extend(body.iter().cloned());
        match exact {
            Some(k) if k > 1 => {
                // copy m reads c as c + m, and only the last adds k: the
                // copies' addresses differ by their displacement alone
                let Some(Op::Load(c)) = test.first() else { unreachable!() };
                let cut = matches!(body[body.len() - 2], Op::Sext(_));
                let inc = if cut { 5 } else { 4 };
                let plain = &body[..body.len() - inc];
                out.truncate(out.len() - body.len());
                for m in 0..k as i64 {
                    for o in plain {
                        out.push(o.clone());
                        if m > 0 && *o == Op::Load(*c) {
                            out.extend([Op::Push(m), Op::Bin(BinOp::Add)]);
                        }
                    }
                }
                out.extend([Op::Load(*c), Op::Push(k as i64), Op::Bin(BinOp::Add)]);
                if cut {
                    out.push(Op::Sext(32));
                }
                out.push(Op::Store(*c));
            }
            _ if twice => {
                // a very short body goes round four times per jump back
                let copies = if body.len() <= MAX_BODY / 4 { 4 } else { 2 };
                for _ in 1..copies {
                    out.extend(test.iter().cloned());
                    out.push(Op::Jz(end));
                    out.extend(body.iter().cloned());
                }
            }
            _ => {}
        }
        out.push(Op::Label(top));
        out.extend(test);
        out.push(Op::Jnz(again));
        out.extend_from_slice(&f.code[j + 1..]);
        f.code = out;
        rotated += 1;
    }
}
