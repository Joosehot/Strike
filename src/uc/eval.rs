//! --release: a program that reads nothing is worked out while compiling.
//! Its result is fixed: what it prints and what it returns. Strike runs its
//! IR here, by the machine's own rules (wrapping arithmetic, a shift's count
//! modulo 64, each language's division), and when it ends within the
//! limits of codegen.toml ([release.evaluate]) the executable only prints
//! that output and returns that number. Nothing is recognised or guessed:
//! it is the program's own work, done once, before it ships.
//!
//! Not finished in time, too much output, recursion too deep, or a division
//! by zero (which must fail when the program runs, as it would): the
//! program is compiled as it is.

use super::ir::{label_count, BinOp, Func, Op, UnOp};
use std::collections::HashMap;

#[derive(serde::Deserialize, Clone)]
pub struct Limits {
    pub max_steps: u64,
    pub max_output: usize,
    pub max_depth: usize,
    /// past the steps: the program compiled and run once, for this long
    pub max_seconds: f64,
}

/// Runs a baking executable (x64::bake_start) for at most `lim.max_seconds`:
/// its output and the number it returned, or None if it did not end in
/// time, crashed, or printed too much.
pub fn bake(exe: &std::path::Path, lim: &Limits) -> Option<(Vec<u8>, i64)> {
    use std::io::Read;
    let mut child = std::process::Command::new(exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let max = lim.max_output + super::x64::RETURNED.len() + 32;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 65536];
        loop {
            match out.read(&mut chunk) {
                Ok(0) | Err(_) => return buf,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.len() > max {
                        return buf;
                    }
                }
            }
        }
    });
    let until = std::time::Instant::now() + std::time::Duration::from_secs_f64(lim.max_seconds);
    let status = loop {
        if let Ok(Some(s)) = child.try_wait() {
            break Some(s);
        }
        if std::time::Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    let buf = reader.join().ok()?;
    if !status?.success() || buf.len() > max {
        return None;
    }
    let mark = super::x64::RETURNED;
    let at = buf.windows(mark.len()).rposition(|w| w == mark)?;
    let returned: i64 = std::str::from_utf8(&buf[at + mark.len()..]).ok()?.trim().parse().ok()?;
    let mut output = buf;
    output.truncate(at);
    (output.len() <= lim.max_output).then_some((output, returned))
}

pub struct Done {
    pub output: Vec<u8>,
    pub returned: i64,
    pub steps: u64,
}

fn bin(op: BinOp, a: i64, b: i64) -> Option<i64> {
    use BinOp::*;
    // a checked operation that does not fit: the program stops there,
    // which only the program itself can do
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

struct Frame {
    func: usize,
    pc: usize,
    base: usize,
}

/// Runs the program from its entry; None when it cannot be finished here.
pub fn run(funcs: &[Func], entry: &str, globals: u32, lim: &Limits) -> Option<Done> {
    let index: HashMap<&str, usize> = funcs.iter().enumerate().map(|(i, f)| (f.name.as_str(), i)).collect();
    let labels: Vec<Vec<usize>> = funcs
        .iter()
        .map(|f| {
            let mut at = vec![0; label_count(&f.code) as usize];
            for (i, op) in f.code.iter().enumerate() {
                if let Op::Label(l) = op {
                    at[*l as usize] = i;
                }
            }
            at
        })
        .collect();
    let first = *index.get(entry)?;
    if funcs[first].params != 0 {
        return None;
    }
    let mut locals: Vec<i64> = vec![0; funcs[first].slots as usize];
    let mut frames = vec![Frame { func: first, pc: 0, base: 0 }];
    let mut stack: Vec<i64> = Vec::new();
    let mut global = vec![0i64; globals as usize];
    let mut output = Vec::new();
    let mut steps = 0u64;
    loop {
        steps += 1;
        if steps > lim.max_steps {
            return None;
        }
        let fr = frames.last_mut().unwrap();
        let f = &funcs[fr.func];
        let op = f.code.get(fr.pc)?;
        fr.pc += 1;
        match op {
            Op::Push(v) => stack.push(*v),
            Op::Load(s) => stack.push(locals[fr.base + *s as usize]),
            Op::Store(s) => locals[fr.base + *s as usize] = stack.pop()?,
            Op::Bin(b) => {
                let r = stack.pop()?;
                let l = stack.pop()?;
                stack.push(bin(*b, l, r)?);
            }
            Op::Un(u) => {
                let x = stack.pop()?;
                stack.push(match u {
                    UnOp::Neg => x.wrapping_neg(),
                    UnOp::Plus => x,
                    UnOp::Not => (x == 0) as i64,
                    UnOp::Bnot => !x,
                    UnOp::NegOv => x.checked_neg()?,
                });
            }
            Op::Dup => stack.push(*stack.last()?),
            // what the program reads is not known here; a global is not kept
            // what the program reads is not known here
            Op::ReadToken | Op::ReadLine => return None,
            Op::LoadG(g) => stack.push(global[*g as usize]),
            Op::StoreG(g) => global[*g as usize] = stack.pop()?,
            Op::Sext(n) => {
                let x = stack.pop()?;
                stack.push(super::ir::sext(x, *n));
            }
            Op::Pop => {
                stack.pop()?;
            }
            Op::Jmp(l) => fr.pc = labels[fr.func][*l as usize],
            Op::Jz(l) => {
                if stack.pop()? == 0 {
                    fr.pc = labels[fr.func][*l as usize];
                }
            }
            Op::Jnz(l) => {
                if stack.pop()? != 0 {
                    fr.pc = labels[fr.func][*l as usize];
                }
            }
            Op::Label(_) => {}
            Op::PrintInt => {
                let v = stack.pop()?;
                output.extend_from_slice(v.to_string().as_bytes());
            }
            Op::PrintStr(s) => output.extend_from_slice(s),
            Op::Call(sym, n) => {
                if frames.len() >= lim.max_depth {
                    return None;
                }
                let callee = *index.get(sym.as_str())?;
                let c = &funcs[callee];
                let base = locals.len();
                locals.resize(base + c.slots as usize, 0);
                let n = *n as usize;
                let args = stack.len().checked_sub(n)?;
                locals[base..base + n].copy_from_slice(&stack[args..]);
                stack.truncate(args);
                frames.push(Frame { func: callee, pc: 0, base });
            }
            Op::Ret => {
                let v = stack.pop()?;
                let done = frames.pop()?;
                locals.truncate(done.base);
                if frames.is_empty() {
                    return Some(Done { output, returned: v, steps });
                }
                stack.push(v);
            }
        }
        if output.len() > lim.max_output {
            return None;
        }
    }
}
