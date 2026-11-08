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
    let me = f.name.clone();
    let n = f.params;
    let code = &f.code;
    let tail = |i: usize| matches!(&code[i], Op::Call(s, k) if *s == me && *k == n);
    let mut acc_op: Option<BinOp> = None;
    let mut sites = 0;
    for i in 0..code.len() {
        if !tail(i) {
            continue;
        }
        match (code.get(i + 1), code.get(i + 2)) {
            (Some(Op::Ret), _) => sites += 1,
            (Some(Op::Bin(b @ (BinOp::Add | BinOp::Mul))), Some(Op::Ret)) => {
                if acc_op.is_some_and(|a| a != *b) {
                    return 0; // one accumulator, of one kind
                }
                acc_op = Some(*b);
                sites += 1;
            }
            _ => {}
        }
    }
    if sites == 0 {
        return 0;
    }
    let old = std::mem::take(&mut f.code);
    let start = label_count(&old);
    let acc = f.slots;
    f.slots += if acc_op.is_some() { 1 } else { 0 };
    let mut out = Vec::with_capacity(old.len() + 8);
    if let Some(b) = acc_op {
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
            (true, Some(Op::Bin(b @ (BinOp::Add | BinOp::Mul))), Some(Op::Ret)) => {
                // the stack holds x under the arguments
                to_params(&mut out);
                out.push(Op::Load(acc));
                out.push(Op::Bin(*b));
                out.push(Op::Store(acc));
                out.push(Op::Jmp(start));
                i += 3;
            }
            (_, _, _) if old[i] == Op::Ret => {
                if let Some(b) = acc_op {
                    out.push(Op::Load(acc));
                    out.push(Op::Bin(b));
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
