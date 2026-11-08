//! HotMotor's inlining rule (codegen.toml, [inline]) on Strike IR. Every
//! call of a function that calls nothing itself offers two variants: the
//! call, or the callee's body copied in its place. The judge scores the
//! cost of the call saved, weighted by the loops around the call, less the
//! cost of the operations copied; the copy is made where it scores above
//! zero. Done on the IR, before branding, so a caller's brand covers the
//! code copied into it.

use super::ir::{label_count, loop_depths, Func, Op};
use super::x64opt::Rules;
use std::collections::HashMap;

/// Copies the chosen calls; returns how many.
pub fn run(funcs: &mut [Func], r: &Rules, release: bool) -> usize {
    let rule = r.inline_for(release);
    let leaves: HashMap<String, Func> = funcs
        .iter()
        .filter(|f| !f.code.iter().any(|o| matches!(o, Op::Call(..))))
        .map(|f| (f.name.clone(), f.clone()))
        .collect();
    let mut copied = 0;
    for f in funcs.iter_mut() {
        // every copy uses the same slots after the caller's own: a copy calls
        // nothing, so no two are ever live at once (and each starts its
        // slots at zero)
        let base = f.slots;
        let code = std::mem::take(&mut f.code);
        let depth = loop_depths(&code);
        let mut labels = label_count(&code);
        let mut out = Vec::with_capacity(code.len());
        for (i, op) in code.iter().enumerate() {
            let callee = match op {
                Op::Call(sym, _) => leaves.get(sym).filter(|c| c.name != f.name),
                _ => None,
            };
            let Some(c) = callee else {
                out.push(op.clone());
                continue;
            };
            let score = rule.call_cost * r.loop_weight.powi(depth[i].min(6) as i32) - rule.size_cost * c.code.len() as f64;
            if c.code.len() > rule.max_ops || score <= 0.0 {
                out.push(op.clone());
                continue;
            }
            // the arguments into the callee's slots (the last is on top),
            // its other slots to zero, as its own prologue would
            f.slots = f.slots.max(base + c.slots);
            for p in (0..c.params).rev() {
                out.push(Op::Store(base + p));
            }
            for s in c.params..c.slots {
                out.push(Op::Push(0));
                out.push(Op::Store(base + s));
            }
            let lbase = labels;
            labels += label_count(&c.code);
            let end = labels;
            labels += 1;
            for o in &c.code {
                out.push(match o {
                    Op::Load(s) => Op::Load(s + base),
                    Op::Store(s) => Op::Store(s + base),
                    Op::Jmp(l) => Op::Jmp(l + lbase),
                    Op::Jz(l) => Op::Jz(l + lbase),
                    Op::Jnz(l) => Op::Jnz(l + lbase),
                    Op::Label(l) => Op::Label(l + lbase),
                    Op::Ret => Op::Jmp(end),
                    o => o.clone(),
                });
            }
            out.push(Op::Label(end));
            copied += 1;
        }
        f.code = out;
    }
    copied
}
