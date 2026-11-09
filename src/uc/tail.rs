//! A function's calls of itself at its returns become jumps to its start.
//!
//!   return f(args)        the arguments into the parameters, and back to
//!                         the start
//!   return x + f(args)    x into an accumulator, then the same; every other
//!   return x * f(args)    return gives the accumulator with its value
//!
//! Wrapping addition and multiplication are associative and commutative, so
//! the result is the same number; x and the arguments are worked out in the
//! same order as before, and nothing is left to do after the call they
//! replace. fib(n) = fib(n-1) + fib(n-2) makes half the calls.

use super::ir::{label_count, BinOp, Func, Op};

/// Rewrites the function; returns how many calls became jumps.
pub fn run(f: &mut Func) -> usize {
    // a call may be given the address of this one's own memory (C's
    // `return f(a)` with a local array), which a jump would reuse
    if f.frame > 0 {
        return 0;
    }
    let me = f.name.clone();
    let n = f.params;
    let code = &f.code;
    let tail = |i: usize| matches!(&code[i], Op::Call(s, k) if *s == me && *k == n);
    // the accumulator's operation, and the width a narrow result is cut to
    // (C's int: the low bits of a sum do not depend on its order either)
    let mut acc_op: Option<(BinOp, Option<u8>)> = None;
    let mut sites = 0;
    for i in 0..code.len() {
        if !tail(i) {
            continue;
        }
        let kind = match (code.get(i + 1), code.get(i + 2), code.get(i + 3)) {
            (Some(Op::Ret), _, _) => {
                sites += 1;
                continue;
            }
            (Some(Op::Bin(b @ (BinOp::Add | BinOp::Mul))), Some(Op::Ret), _) => (*b, None),
            (Some(Op::Bin(b @ (BinOp::Add | BinOp::Mul))), Some(Op::Sext(w)), Some(Op::Ret)) => (*b, Some(*w)),
            _ => continue,
        };
        if acc_op.is_some_and(|a| a != kind) {
            return 0; // one accumulator, of one kind
        }
        acc_op = Some(kind);
        sites += 1;
    }
    if sites == 0 {
        return 0;
    }
    let old = std::mem::take(&mut f.code);
    let start = label_count(&old);
    let acc = f.slots;
    f.slots += if acc_op.is_some() { 1 } else { 0 };
    let mut out = Vec::with_capacity(old.len() + 8);
    if let Some((b, _)) = acc_op {
        out.push(Op::Push(if b == BinOp::Add { 0 } else { 1 }));
        out.push(Op::Store(acc));
    }
    out.push(Op::Label(start));
    // every local but the parameters and the accumulator starts at zero, as
    // a new call's would
    for s in n..acc {
        out.push(Op::Push(0));
        out.push(Op::Store(s));
    }
    let to_params = |out: &mut Vec<Op>| {
        for p in (0..n).rev() {
            out.push(Op::Store(p));
        }
    };
    let mut i = 0;
    while i < old.len() {
        let is_tail = matches!(&old[i], Op::Call(s, k) if *s == me && *k == n);
        match (is_tail, old.get(i + 1), old.get(i + 2)) {
            (true, Some(Op::Ret), _) => {
                to_params(&mut out);
                out.push(Op::Jmp(start));
                i += 2;
            }
            (true, Some(Op::Bin(b @ (BinOp::Add | BinOp::Mul))), Some(next)) if acc_op.is_some_and(|(a, cut)| a == *b && (*next == Op::Ret || (cut.is_some() && matches!(next, Op::Sext(_)) && old.get(i + 3) == Some(&Op::Ret)))) => {
                // the stack holds x under the arguments; the accumulator
                // keeps all 64 bits, and a return cuts them
                to_params(&mut out);
                out.push(Op::Load(acc));
                out.push(Op::Bin(*b));
                out.push(Op::Store(acc));
                out.push(Op::Jmp(start));
                i += if *next == Op::Ret { 3 } else { 4 };
            }
            (_, _, _) if old[i] == Op::Ret => {
                if let Some((b, cut)) = acc_op {
                    out.push(Op::Load(acc));
                    out.push(Op::Bin(b));
                    if let Some(w) = cut {
                        out.push(Op::Sext(w));
                    }
                }
                out.push(Op::Ret);
                i += 1;
            }
            _ => {
                out.push(old[i].clone());
                i += 1;
            }
        }
    }
    f.code = out;
    sites
}
