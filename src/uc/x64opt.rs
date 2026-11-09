//! Strike's optimizing code generator. The IR's stack is kept at compile
//! time: a constant, a local, a local plus a constant (one `lea` when it is
//! needed) or a result in rax stays where it is until an operation needs
//! it, and a comparison followed by a jump is one `cmp` and one `jcc`. Only
//! what must cross a call is pushed; a value that crosses a jump stays in
//! rax. The frame has no rbp: saved registers are pushed and popped.
//!
//! HotMotor chooses which locals live in callee-saved registers
//! (codegen.toml): every local offers a register or memory, the judge
//! scores the memory operations a register saves (a use in a loop weighs
//! more), and the beam keeps the best.
//!
//! The plain generator (x64.rs) is the reference: every program compiled
//! here must print what it prints.

use super::ir::{BinOp, Func, Op, UnOp};
use super::x64::{call, Asm, Code, Target};

pub const RULES: &str = include_str!("../../codegen.toml");

#[derive(serde::Deserialize, Clone)]
pub struct Rules {
    pub loop_weight: f64,
    pub save_cost: f64,
    pub beam: usize,
    pub inline: InlineRules,
    pub unroll: UnrollRules,
    pub division: DivisionRules,
    pub release: Release,
}

#[derive(serde::Deserialize, Clone)]
pub struct DivisionRules {
    pub bypass_32: bool,
}

#[derive(serde::Deserialize, Clone)]
pub struct UnrollRules {
    pub levels: u32,
    pub max_ops: usize,
}

#[derive(serde::Deserialize, Clone)]
pub struct Release {
    pub magic_division: bool,
    pub evaluate: super::eval::Limits,
    pub inline: InlineRules,
}

#[derive(serde::Deserialize, Clone)]
pub struct InlineRules {
    pub call_cost: f64,
    pub size_cost: f64,
    pub max_ops: usize,
}

impl Rules {
    /// codegen.toml, or the file STRIKE_CODEGEN names instead.
    pub fn read() -> Result<Rules, String> {
        let (name, text) = match std::env::var_os("STRIKE_CODEGEN") {
            Some(p) => {
                let t = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.to_string_lossy()))?;
                (p.to_string_lossy().to_string(), t)
            }
            None => ("codegen.toml".to_string(), RULES.to_string()),
        };
        toml::from_str(&text).map_err(|e| format!("{name}: {e}"))
    }
    /// What the rules add to a brand: other numbers give other code.
    pub fn key(&self, release: bool) -> String {
        let i = if release { &self.release.inline } else { &self.inline };
        format!(
            "x64opt-21 release={release} magic={} div32={} unroll={},{} loop={} save={} beam={} inline={},{},{}",
            release && self.release.magic_division,
            self.division.bypass_32,
            self.unroll.levels,
            self.unroll.max_ops,
            self.loop_weight,
            self.save_cost,
            self.beam,
            i.call_cost,
            i.size_cost,
            i.max_ops
        )
    }
    pub fn inline_for(&self, release: bool) -> &InlineRules {
        if release {
            &self.release.inline
        } else {
            &self.inline
        }
    }
}

const RAX: u8 = 0;
const RCX: u8 = 1;
const R10: u8 = 10;
const R11: u8 = 11;
/// rbx, rsi, rdi, r12 to r15: kept across calls (Windows x64)
const SAVED: [u8; 7] = [3, 6, 7, 12, 13, 14, 15];
const ARGS: [u8; 4] = [1, 2, 8, 9];
/// r10, r9, r8, rdx: not kept across calls, so a local lives in one only
/// where no operation that writes it comes while the local is still to be
/// read (`clobbers`); such a register costs nothing to save
const VOL: [u8; 4] = [10, 9, 8, 2];

/// Which of the VOL registers an operation may write (bit k for VOL[k]): a
/// call takes all, a division and a checked shift rdx and r8.
fn clobbers(op: &Op) -> u8 {
    match op {
        Op::Call(..) | Op::PrintInt | Op::PrintStr(_) | Op::PrintMem | Op::ReadToken | Op::ReadLine | Op::Fail(..) | Op::FailMem(_) => 0b1111,
        Op::Bin(BinOp::DivTrunc | BinOp::ModTrunc | BinOp::DivFloor | BinOp::ModFloor | BinOp::ShlOv) => 0b1100,
        _ => 0,
    }
}

/// For every local, the VOL registers it may live in (bit k for VOL[k]):
/// none that an operation writes while the local is live after it, or
/// while a value read from the local may still wait on the generator's
/// stack (read since the IR's stack was last empty, as the values on it
/// are worked out lazily).
fn volatile_ok(code: &[Op], slots: u32, live: &[Vec<u64>], at_depth: &[Option<i32>]) -> Vec<u8> {
    let mut ok = vec![0b1111u8; slots as usize];
    let mut window: Vec<u32> = Vec::new();
    for (i, op) in code.iter().enumerate() {
        if let Op::Load(s) = op {
            window.push(*s);
        }
        let c = clobbers(op);
        if c != 0 {
            for s in 0..slots {
                if live[i][s as usize / 64] & (1u64 << (s % 64)) != 0 {
                    ok[s as usize] &= !c;
                }
            }
            for &s in &window {
                ok[s as usize] &= !c;
            }
        }
        // nothing waits where the IR's stack is empty (on every way there),
        // and at a label or a jump everything waiting is pushed or in rax
        if at_depth.get(i + 1) == Some(&Some(0)) || matches!(op, Op::Label(_) | Op::Jmp(_) | Op::Jz(_) | Op::Jnz(_) | Op::Ret) {
            window.clear();
        }
    }
    ok
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum V {
    Imm(i64),
    Slot(u32),
    Rax,
    /// the flags of a comparison; the byte is its setcc opcode (0x94 = e)
    Flags(u8),
    /// pushed on the machine's stack
    Stack,
    /// a local held in a register, plus a constant: not worked out until it
    /// is needed, and then one `lea` into wherever it goes
    Lea(u32, i32),
    /// a local held in a register, shifted left by 0 to 3 (an index times
    /// its element's size)
    Scaled(u32, u8, i32),
    /// base + index << k + d, both locals in registers: an element's
    /// address, used where it is as [base + index * size + d]
    Index(u32, u32, u8, i32),
    /// rax + index << k + d, the index a local in a register: an element's
    /// address whose base (a local in memory) was loaded into rax. It holds
    /// rax, as V::Rax does.
    RIdx(u32, u8, i32),
    /// a float in xmm2 to xmm5 (a result, or an element loaded for a
    /// float operation): the next float operation takes it there, anything
    /// else has it moved out
    X(u8),
    /// a global's or the frame's memory plus d bytes: used where it is, as
    /// [address] (an absolute displacement) or [rsp + d]
    At(B, i32),
    /// the same plus index << k, the index a local in a register:
    /// [index * size + address], [rsp + index * size + d]
    AtIdx(B, u32, u8, i32),
}

/// Memory whose address the code knows without a register: the zeros at a
/// bss offset (a global), or the function's own memory at a frame offset.
#[derive(Clone, Copy, PartialEq, Debug)]
enum B {
    G(u32),
    F(i32),
}

/// The xmm registers floats wait in (volatile on Windows, so a call takes
/// them out first); xmm0 and xmm1 are the scratch of single operations.
const XMMS: std::ops::RangeInclusive<u8> = 2..=5;

/// Where memory is: a register plus a displacement, or base + index << k
/// + displacement (no base: None).
#[derive(Clone, Copy)]
enum Addr {
    Base(u8, i32),
    Index(Option<u8>, u8, u8, i32),
    /// an absolute address in the zeros (its bss offset, wrapping), plus
    /// index << k
    Abs(Option<(u8, u8)>, u32),
}

#[derive(Clone, Copy)]
enum Loc {
    Reg(u8),
    Mem(i32),
    /// a float local in xmm6 to xmm15 (callee-saved on Windows)
    Xmm(u8),
}

/// The xmm registers float locals live in: kept across calls by the
/// callee, so a function that uses one saves it.
const XSAVED: [u8; 10] = [6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

/// Where HotMotor puts a local.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Place {
    Mem,
    Gpr,
    Xmm,
    /// VOL[k], a register nothing writes while the local is live
    Vol(u8),
}

#[derive(Clone, Copy)]
enum Opd {
    Imm(i32),
    Reg(u8),
    Mem(i32),
}

fn rex(r: u8, b: u8) -> u8 {
    0x48 | ((r >> 3) & 1) << 2 | ((b >> 3) & 1)
}

/// How much each local is used, a use in a loop counting `loop_weight`
/// times per level (a loop: the ops between a label and a jump back to it).
/// It is split in two: the uses a float operation makes (a load it takes,
/// a store of its result) and all the others.
fn weights(f: &Func, r: &Rules) -> (Vec<f64>, Vec<f64>) {
    let at = super::ir::loop_depths(&f.code);
    let float = float_uses(&f.code);
    let mut w = vec![0.0; f.slots as usize];
    let mut wf = vec![0.0; f.slots as usize];
    for (i, op) in f.code.iter().enumerate() {
        if let Op::Load(s) | Op::Store(s) = op {
            let x = r.loop_weight.powi(at[i].min(6) as i32);
            if float[i] {
                wf[*s as usize] += x;
            } else {
                w[*s as usize] += x;
            }
        }
    }
    for p in 0..f.params.min(4) {
        w[p as usize] += 1.0; // a register parameter is already in a register
    }
    (w, wf)
}

/// HotMotor: the locals in order of weight, each a general register, an
/// xmm register or memory; the beam keeps the `beam` best partial choices,
/// scored by the memory operations saved less the cost of saving the
/// registers used. A general register saves the uses that are not a
/// float operation's (a float there is moved to xmm for each), an xmm
/// register those that are (a float in memory is an operand as it is, so a
/// general register saves a float nothing).
fn choose(f: &Func, r: &Rules, vol_ok: &[u8]) -> Vec<Place> {
    let (w, wf) = weights(f, r);
    let mut order: Vec<usize> = (0..w.len()).collect();
    order.sort_by(|&a, &b| (w[b] + wf[b]).total_cmp(&(w[a] + wf[a])).then(a.cmp(&b)));
    let mut beam: Vec<(f64, Vec<Place>)> = vec![(0.0, vec![Place::Mem; w.len()])];
    for &s in &order {
        let mut next = Vec::new();
        for (score, pick) in &beam {
            next.push((*score, pick.clone()));
            for (kind, gain, limit) in [(Place::Gpr, w[s], SAVED.len()), (Place::Xmm, wf[s], XSAVED.len())] {
                if pick.iter().filter(|&&p| p == kind).count() < limit && gain > 0.0 {
                    let mut p = pick.clone();
                    p[s] = kind;
                    next.push((score + gain - r.save_cost, p));
                }
            }
            // a volatile register saves the same uses and costs no saving;
            // a parameter's own argument register spares its move too
            if w[s] > 0.0 {
                for k in 0..VOL.len() as u8 {
                    if vol_ok[s] & (1 << k) != 0 && !pick.contains(&Place::Vol(k)) {
                        let own = (s as u32) < f.params.min(4) && ARGS[s] == VOL[k as usize];
                        let mut p = pick.clone();
                        p[s] = Place::Vol(k);
                        next.push((score + w[s] + if own { 0.5 } else { 0.0 }, p));
                    }
                }
            }
        }
        next.sort_by(|a, b| b.0.total_cmp(&a.0));
        next.truncate(r.beam.max(1));
        beam = next;
    }
    beam.swap_remove(0).1
}

/// For every operation, the locals live after it (read later before they
/// are written again), by the usual backward dataflow over the jumps. A
/// local that is not live after an operation may have its register taken.
fn live_after(code: &[Op], slots: u32) -> Vec<Vec<u64>> {
    let words = (slots as usize).div_ceil(64).max(1);
    let n = code.len();
    let mut label_at = vec![n; super::ir::label_count(code) as usize];
    for (i, op) in code.iter().enumerate() {
        if let Op::Label(l) = op {
            label_at[*l as usize] = i;
        }
    }
    let succ = |i: usize| -> Vec<usize> {
        match &code[i] {
            Op::Jmp(l) => vec![label_at[*l as usize]],
            Op::Jz(l) | Op::Jnz(l) => vec![label_at[*l as usize], i + 1],
            Op::Ret | Op::Fail(..) | Op::FailMem(_) => vec![],
            _ => vec![i + 1],
        }
    };
    let mut live_in = vec![vec![0u64; words]; n + 1];
    let mut out = vec![vec![0u64; words]; n];
    loop {
        let mut changed = false;
        for i in (0..n).rev() {
            let mut o = vec![0u64; words];
            for s in succ(i) {
                for w in 0..words {
                    o[w] |= live_in[s][w];
                }
            }
            let mut li = o.clone();
            match &code[i] {
                Op::Store(s) => li[*s as usize / 64] &= !(1u64 << (s % 64)),
                Op::Load(s) => li[*s as usize / 64] |= 1u64 << (s % 64),
                _ => {}
            }
            if li != live_in[i] || o != out[i] {
                live_in[i] = li;
                out[i] = o;
                changed = true;
            }
        }
        if !changed {
            return out;
        }
    }
}

/// Which operations can be reached from the start, and the depth of the
/// IR's stack at each, found by following every way through the code: a
/// label reached only by jumps from later on (a loop's top) is reached even
/// where the code just before it is not, and a jump that is never reached
/// decides nothing.
pub(super) fn stack_depths(code: &[Op]) -> Vec<Option<i32>> {
    let labels = super::ir::label_count(code);
    let mut pos = vec![0usize; labels as usize];
    for (i, op) in code.iter().enumerate() {
        if let Op::Label(l) = op {
            pos[*l as usize] = i;
        }
    }
    let mut at: Vec<Option<i32>> = vec![None; code.len()];
    let mut work = vec![(0usize, 0i32)];
    while let Some((i, d)) = work.pop() {
        if i >= code.len() || at[i].is_some() {
            continue;
        }
        at[i] = Some(d);
        let Some(effect) = super::ir::stack_effect(&code[i]) else { continue }; // a return or a stop
        let after = d + effect;
        match &code[i] {
            Op::Jmp(l) => work.push((pos[*l as usize], after)),
            Op::Jz(l) | Op::Jnz(l) => {
                work.push((pos[*l as usize], after));
                work.push((i + 1, after));
            }
            _ => work.push((i + 1, after)),
        }
    }
    at
}

/// Locals that are never live at once share one slot, so HotMotor's beam
/// chooses a register for them together: a local is "live" from a store to
/// its last read, and also while a value read from it may still wait on the
/// generator's stack (read since the IR's stack was last empty). Two locals
/// meet where one is stored while the other is live. Parameters, and locals
/// read before they are stored (they hold their zero from the start), keep
/// their own slots; a float local shares only with float locals. The slots
/// are numbered anew, the parameters first.
fn coalesce(f: &Func) -> Func {
    let n = f.slots as usize;
    let words = n.div_ceil(64).max(1);
    if n == 0 {
        return f.clone();
    }
    let code = &f.code;
    let live = live_after(code, f.slots);
    let bit = |v: &[u64], s: usize| v[s / 64] & (1u64 << (s % 64)) != 0;
    // live where the function begins
    let mut entry = vec![0u64; words];
    if let Some(after) = live.first() {
        entry.clone_from(after);
        match code[0] {
            Op::Store(s) => entry[s as usize / 64] &= !(1u64 << (s % 64)),
            Op::Load(s) => entry[s as usize / 64] |= 1u64 << (s % 64),
            _ => {}
        }
    }
    // float or not, by most of its uses
    let fl = float_uses(code);
    let mut votes = vec![0i64; n];
    for (i, op) in code.iter().enumerate() {
        if let Op::Load(s) | Op::Store(s) = op {
            votes[*s as usize] += if fl[i] { 1 } else { -1 };
        }
    }
    let mut meet = vec![vec![0u64; words]; n];
    let at_depth = stack_depths(code);
    let mut window: Vec<u32> = Vec::new();
    for (i, op) in code.iter().enumerate() {
        match op {
            Op::Load(s) => window.push(*s),
            Op::Store(b) => {
                let b = *b as usize;
                let mut with = live[i].clone();
                for &a in &window {
                    with[a as usize / 64] |= 1u64 << (a % 64);
                }
                for a in 0..n {
                    if a != b && bit(&with, a) {
                        meet[a][b / 64] |= 1u64 << (b % 64);
                        meet[b][a / 64] |= 1u64 << (a % 64);
                    }
                }
            }
            _ => {}
        }
        // the window as volatile_ok keeps it
        if at_depth.get(i + 1) == Some(&Some(0)) || matches!(op, Op::Label(_) | Op::Jmp(_) | Op::Jz(_) | Op::Jnz(_) | Op::Ret) {
            window.clear();
        }
    }
    let mut rep = vec![usize::MAX; n];
    // (the slot that stands for them, every slot met by one of them, float)
    let mut groups: Vec<(usize, Vec<u64>, bool)> = Vec::new();
    for s in 0..n {
        if s < f.params as usize || bit(&entry, s) {
            rep[s] = s;
            continue;
        }
        let float = votes[s] > 0;
        match groups.iter_mut().find(|(_, m, k)| *k == float && !bit(m, s)) {
            Some((r, m, _)) => {
                rep[s] = *r;
                for w in 0..words {
                    m[w] |= meet[s][w];
                }
            }
            None => {
                rep[s] = s;
                groups.push((s, meet[s].clone(), float));
            }
        }
    }
    // numbered anew: the parameters, then each slot that stands for others
    let mut new = vec![u32::MAX; n];
    let mut next = 0u32;
    for s in 0..n {
        if rep[s] == s {
            new[s] = next;
            next += 1;
        }
    }
    let mut g = f.clone();
    g.slots = next;
    for op in g.code.iter_mut() {
        if let Op::Load(s) | Op::Store(s) = op {
            *s = new[rep[*s as usize]];
        }
    }
    g
}

/// For every Load and Store, whether it is a float operation's: the value
/// loaded goes to one, or the value stored comes from one. Followed on the
/// IR's stack between labels and jumps.
fn float_uses(code: &[Op]) -> Vec<bool> {
    let mut out = vec![false; code.len()];
    let into = float_operands(code);
    let mut st: Vec<Option<usize>> = Vec::new();
    for (i, op) in code.iter().enumerate() {
        if let Op::Load(_) = op {
            out[i] = into[i];
        }
        let (ins, outs) = match op {
            Op::Push(_) | Op::Load(_) | Op::LoadG(_) | Op::Frame(_) | Op::AddrG(_) | Op::ReadLine | Op::Data(_) => (0, 1),
            Op::Store(_) | Op::StoreG(_) | Op::Pop | Op::PrintInt | Op::Jz(_) | Op::Jnz(_) => (1, 0),
            Op::Bin(_) => (2, 1),
            Op::Un(_) | Op::Sext(_) | Op::LoadM(_) => (1, 1),
            Op::StoreM(_) | Op::OpM(..) | Op::PrintMem => (2, 0),
            Op::Call(_, n) => (*n as usize, 1),
            Op::ReadToken => (0, 2),
            Op::PrintStr(_) => (0, 0),
            Op::Dup | Op::Label(_) | Op::Jmp(_) | Op::Ret | Op::Fail(..) | Op::FailMem(_) => {
                st.clear();
                if matches!(op, Op::Dup) {
                    st.push(None);
                    st.push(None);
                }
                continue;
            }
        };
        if let Op::Store(_) = op {
            if let Some(Some(p)) = st.last() {
                let made = match &code[*p] {
                    Op::Bin(b) => b.is_float() && !b.is_float_comparison(),
                    Op::Un(u) => matches!(u, UnOp::FNeg | UnOp::FAbs | UnOp::FSqrt | UnOp::FromInt | UnOp::FromInt32 | UnOp::ToF32),
                    Op::LoadM(8) => into[*p],
                    _ => false,
                };
                out[i] = made;
            }
        }
        for _ in 0..ins {
            st.pop();
        }
        for _ in 0..outs {
            st.push(Some(i));
        }
    }
    // an element stored into a local that only float operations read is a
    // float's use too: it is loaded straight into the xmm register
    let mut read = vec![(false, true); code.iter().filter_map(|o| if let Op::Load(s) | Op::Store(s) = o { Some(*s as usize + 1) } else { None }).max().unwrap_or(0)];
    for (i, op) in code.iter().enumerate() {
        if let Op::Load(s) = op {
            let r = &mut read[*s as usize];
            *r = (true, r.1 && out[i]);
        }
    }
    for i in 1..code.len() {
        if let (Op::LoadM(8), Op::Store(s)) = (&code[i - 1], &code[i]) {
            if read[*s as usize] == (true, true) {
                out[i] = true;
            }
        }
    }
    out
}

/// HotMotor's judge for each division by a constant (--release): the
/// divider, or the multiplication by the magic number. A division whose
/// value comes back round its loop into what it divides waits on itself,
/// and there the divider's shorter wait wins (measured: 30 million
/// `s = (s * 31 + i) % 1000003`, 0.085 s with it against 0.095 s). Any
/// other gets the multiplication, whose instructions can overlap from one
/// time round to the next (measured: `s += (k ^ j) % 1000`, 0.0285 s
/// against 0.040 s). True where the multiplication is taken.
fn magic_where(f: &Func) -> Vec<bool> {
    use std::collections::{HashMap, HashSet};
    let code = &f.code;
    let st = super::opt::starts(code);
    let mut label_at = HashMap::new();
    for (i, op) in code.iter().enumerate() {
        if let Op::Label(l) = op {
            label_at.insert(*l, i);
        }
    }
    let loops: Vec<(usize, usize)> = code
        .iter()
        .enumerate()
        .filter_map(|(j, op)| match op {
            Op::Jmp(l) | Op::Jz(l) | Op::Jnz(l) => label_at.get(l).filter(|&&t| t < j).map(|&t| (t, j)),
            _ => None,
        })
        .collect();
    let loads = |from: usize, to: usize| -> HashSet<u32> {
        code[from..to].iter().filter_map(|o| if let Op::Load(s) = o { Some(*s) } else { None }).collect()
    };
    let mut take = vec![false; code.len()];
    for i in 1..code.len() {
        let Op::Bin(BinOp::DivTrunc | BinOp::ModTrunc | BinOp::DivFloor | BinOp::ModFloor) = code[i] else { continue };
        let Op::Push(_) = code[i - 1] else { continue };
        // the innermost loop around it; outside every loop it waits on nothing
        let Some(&(t, j)) = loops.iter().filter(|(t, j)| *t < i && i < *j).min_by_key(|(t, j)| j - t) else {
            take[i] = true;
            continue;
        };
        let Some(s) = st[i] else { continue };
        let divided = loads(s, i - 1);
        // where its value goes: the store that ends its statement
        let Some(dest) = code[i + 1..j].iter().find_map(|o| if let Op::Store(x) = o { Some(*x) } else { None }) else { continue };
        // what each local stored in the loop is made from
        let mut from: HashMap<u32, Option<HashSet<u32>>> = HashMap::new();
        for k in t..=j {
            if let Op::Store(x) = code[k] {
                let made = st.get(k.wrapping_sub(1)).copied().flatten().map(|s0| loads(s0, k));
                let e = from.entry(x).or_insert_with(|| Some(HashSet::new()));
                match (e.as_mut(), made) {
                    (Some(set), Some(m)) => set.extend(m),
                    _ => *e = None, // not known: taken as made from anything
                }
            }
        }
        // does dest reach what is divided, through the loop's stores?
        let mut seen: HashSet<u32> = HashSet::new();
        let mut todo: Vec<u32> = divided.into_iter().collect();
        let mut chain = false;
        while let Some(x) = todo.pop() {
            if x == dest {
                chain = true;
                break;
            }
            if !seen.insert(x) {
                continue;
            }
            match from.get(&x) {
                Some(Some(m)) => todo.extend(m.iter().copied()),
                Some(None) => {
                    chain = true;
                    break;
                }
                None => {}
            }
        }
        take[i] = !chain;
    }
    take
}

/// The magic number and shift that divide by d with a multiplication
/// (Hacker's Delight, 10-1), for |d| >= 2.
pub fn magic(d: i64) -> (i64, u32) {
    let two63: u64 = 1 << 63;
    let ad = d.unsigned_abs();
    let t = two63 + ((d as u64) >> 63);
    let anc = t - 1 - t % ad;
    let mut p = 63u32;
    let (mut q1, mut r1) = (two63 / anc, two63 - (two63 / anc) * anc);
    let (mut q2, mut r2) = (two63 / ad, two63 - (two63 / ad) * ad);
    loop {
        p += 1;
        q1 = q1.wrapping_mul(2);
        r1 = r1.wrapping_mul(2);
        if r1 >= anc {
            q1 = q1.wrapping_add(1);
            r1 = r1.wrapping_sub(anc);
        }
        q2 = q2.wrapping_mul(2);
        r2 = r2.wrapping_mul(2);
        if r2 >= ad {
            q2 = q2.wrapping_add(1);
            r2 = r2.wrapping_sub(ad);
        }
        let delta = ad - r2;
        if !(q1 < delta || (q1 == delta && r1 == 0)) {
            break;
        }
    }
    let m = q2.wrapping_add(1) as i64;
    (if d < 0 { m.wrapping_neg() } else { m }, p - 64)
}

#[cfg(test)]
mod tests {
    use super::magic;

    /// The emitted sequence, step by step: q = high(n * m) (+n or -n),
    /// shifted, plus one if negative.
    fn divide(n: i64, d: i64) -> i64 {
        let (m, s) = magic(d);
        let mut hi = ((n as i128 * m as i128) >> 64) as i64;
        if d > 0 && m < 0 {
            hi = hi.wrapping_add(n);
        } else if d < 0 && m > 0 {
            hi = hi.wrapping_sub(n);
        }
        hi >>= s;
        hi + ((hi as u64) >> 63) as i64
    }

    #[test]
    fn a_magic_multiplication_divides() {
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as i64
        };
        let mut ds = vec![2, 3, 5, 7, 10, 1000003, -2, -3, -7, -1000003, i64::MAX, i64::MIN + 1, 1 << 40, -(1 << 40)];
        for _ in 0..200 {
            let d = next() >> (next() as u64 % 63);
            if d.unsigned_abs() >= 2 && d != i64::MIN {
                ds.push(d);
            }
        }
        for &d in &ds {
            let mut ns = vec![0, 1, -1, i64::MAX, i64::MIN, d, -d, d - 1, d + 1];
            ns.extend((0..200).map(|_| next() >> (next() as u64 % 64)));
            for n in ns {
                assert_eq!(divide(n, d), n.wrapping_div(d), "{n} / {d}");
            }
        }
    }
}

/// The frame has no rbp: the saved registers are pushed, then rsp goes down
/// by `frame`, and a local in memory is at its offset above the call area,
/// counted from that rsp. Every value pushed since (and the room a call
/// takes for its arguments) moves rsp further, and the generator knows how
/// far at every operation, so each address is [rsp + offset + what was
/// pushed].
struct G {
    a: Asm,
    v: Vec<V>,
    loc: Vec<Loc>,
    /// the callee-saved registers this function uses, in the order pushed
    saved: Vec<u8>,
    /// the callee-saved xmm registers this function uses, and where in the
    /// frame each is kept
    xsaved: Vec<(u8, i32)>,
    frame: i32,
    /// bytes taken below the pushed values (a call's argument room)
    extra: i32,
    /// division by a constant as a multiplication (codegen.toml), at the
    /// operations magic_where takes
    magic: bool,
    magic_at: Vec<bool>,
    /// the operation being compiled
    op_at: usize,
    /// where checked operations jump when a number would not fit
    trap_at: Option<u32>,
    /// division of numbers that fit 32 bits by the 32-bit instruction
    div32: bool,
    /// the frame offset of the function's own memory (Op::Frame)
    memory: i32,
    /// the locals live after each operation (computed where a local is in
    /// an xmm register: its register may then take a result in place)
    live: Vec<Vec<u64>>,
}

impl G {
    fn rr(&mut self, op: &[u8], reg: u8, rm: u8) {
        self.a.e(&[rex(reg, rm)]);
        self.a.e(op);
        self.a.e(&[0xC0 | (reg & 7) << 3 | (rm & 7)]);
    }
    fn phys(&self) -> i32 {
        self.v.iter().filter(|x| **x == V::Stack).count() as i32
    }
    /// The address of a frame offset now.
    fn at(&self, off: i32) -> i32 {
        off + 8 * self.phys() + self.extra
    }
    /// op with [rsp + the frame offset d]
    fn rm(&mut self, op: &[u8], reg: u8, d: i32) {
        let disp = self.at(d);
        self.a.e(&[rex(reg, 0)]);
        self.a.e(op);
        self.a.e(&[0x84 | (reg & 7) << 3, 0x24]);
        self.a.d32(disp);
    }
    /// lea r, [base + c]
    fn lea(&mut self, r: u8, base: u8, c: i32) {
        if c == 0 {
            self.mov_rr(r, base);
            return;
        }
        self.a.e(&[rex(r, base), 0x8D, 0x80 | (r & 7) << 3 | (base & 7)]);
        if base & 7 == 4 {
            self.a.e(&[0x24]);
        }
        self.a.d32(c);
    }
    fn slot_reg(&self, s: u32) -> u8 {
        match self.loc[s as usize] {
            Loc::Reg(y) => y,
            Loc::Mem(_) | Loc::Xmm(_) => unreachable!("a Lea is made only of a local in a register"),
        }
    }
    /// Is local s read no more after the operation being compiled, and no
    /// value waiting on the stack is it? Then its register may be taken.
    fn dead(&self, s: u32) -> bool {
        let Some(after) = self.live.get(self.op_at) else { return false };
        after[s as usize / 64] & (1u64 << (s % 64)) == 0 && self.reads(s).is_none()
    }
    /// A float result waiting in xmm register k is pushed before k is
    /// written (k is a float local's, which a dead local's result took).
    fn keep_xmm(&mut self, k: u8) {
        if let Some(i) = self.v.iter().rposition(|x| *x == V::X(k)) {
            self.flush_to(i);
        }
    }
    /// An SSE operation xmm dst, xmm src (prefix 0x66, 0xF2, 0xF3 or none;
    /// a REX for xmm8 and up).
    fn sse_rr(&mut self, prefix: Option<u8>, op: u8, dst: u8, src: u8) {
        if let Some(p) = prefix {
            self.a.e(&[p]);
        }
        if dst >= 8 || src >= 8 {
            self.a.e(&[0x40 | (dst >> 3) << 2 | (src >> 3)]);
        }
        self.a.e(&[0x0F, op, 0xC0 | (dst & 7) << 3 | (src & 7)]);
    }
    /// An SSE operation xmm, [rsp + the frame offset d].
    fn sse_rsp(&mut self, prefix: Option<u8>, op: u8, xmm: u8, d: i32) {
        let disp = self.at(d);
        if let Some(p) = prefix {
            self.a.e(&[p]);
        }
        if xmm >= 8 {
            self.a.e(&[0x44]);
        }
        self.a.e(&[0x0F, op, 0x84 | (xmm & 7) << 3, 0x24]);
        self.a.d32(disp);
    }
    /// An SSE operation xmm, [rip + the 8 bytes of c] (a float constant in
    /// the read-only data, as clang keeps it).
    fn sse_const(&mut self, prefix: Option<u8>, op: u8, xmm: u8, c: i64) {
        if let Some(p) = prefix {
            self.a.e(&[p]);
        }
        if xmm >= 8 {
            self.a.e(&[0x44]);
        }
        self.a.e(&[0x0F, op, 0x05 | (xmm & 7) << 3]);
        self.a.reloc(Target::Data(c.to_le_bytes().to_vec()));
    }
    /// movq xmm, r
    fn movq_to_xmm(&mut self, xmm: u8, r: u8) {
        self.a.e(&[0x66, 0x48 | (xmm >> 3) << 2 | (r >> 3), 0x0F, 0x6E, 0xC0 | (xmm & 7) << 3 | (r & 7)]);
    }
    /// movq r, xmm
    fn movq_from_xmm(&mut self, r: u8, xmm: u8) {
        self.a.e(&[0x66, 0x48 | (xmm >> 3) << 2 | (r >> 3), 0x0F, 0x7E, 0xC0 | (xmm & 7) << 3 | (r & 7)]);
    }
    /// Is any value still to be worked out from slot s?
    fn reads(&self, s: u32) -> Option<usize> {
        self.v.iter().rposition(|y| match y {
            V::Slot(t) | V::Lea(t, _) | V::Scaled(t, _, _) => *t == s,
            V::Index(b, i, _, _) => *b == s || *i == s,
            V::RIdx(i, _, _) | V::AtIdx(_, i, _, _) => *i == s,
            _ => false,
        })
    }
    /// Is this local in a register?
    fn in_reg(&self, s: u32) -> bool {
        matches!(self.loc[s as usize], Loc::Reg(_))
    }
    /// op r, [address]: the operand size by `prefix` (0x66 for 16 bits) and
    /// REX.W (`wide`).
    fn mem(&mut self, prefix: bool, wide: bool, op: &[u8], r: u8, at: Addr) {
        if prefix {
            self.a.e(&[0x66]);
        }
        let (b, x) = match at {
            Addr::Base(b, _) => (b, 0),
            Addr::Index(b, i, _, _) => (b.unwrap_or(0), i),
            Addr::Abs(i, _) => (0, i.map_or(0, |(i, _)| i)),
        };
        self.a.e(&[0x40 | (wide as u8) << 3 | ((r >> 3) & 1) << 2 | ((x >> 3) & 1) << 1 | ((b >> 3) & 1)]);
        self.a.e(op);
        match at {
            Addr::Abs(i, o) => {
                // no base: mod 00, SIB base 101 is a 32-bit displacement;
                // index 100 is none
                let sib = match i {
                    Some((i, k)) => k << 6 | (i & 7) << 3 | 5,
                    None => 0x25,
                };
                self.a.e(&[0x04 | (r & 7) << 3, sib]);
                self.a.reloc(Target::Abs(o));
            }
            Addr::Base(b, d) => {
                self.a.e(&[0x80 | (r & 7) << 3 | (b & 7)]);
                if b & 7 == 4 {
                    self.a.e(&[0x24]);
                }
                self.a.d32(d);
            }
            Addr::Index(Some(b), i, k, d) => {
                self.a.e(&[0x84 | (r & 7) << 3, k << 6 | (i & 7) << 3 | (b & 7)]);
                self.a.d32(d);
            }
            Addr::Index(None, i, k, d) => {
                // no base: mod 00 with base 101 is a 32-bit displacement
                self.a.e(&[0x04 | (r & 7) << 3, k << 6 | (i & 7) << 3 | 5]);
                self.a.d32(d);
            }
        }
    }
    fn mov_imm(&mut self, r: u8, v: i64) {
        if let Ok(i) = i32::try_from(v) {
            self.a.e(&[rex(0, r), 0xC7, 0xC0 | (r & 7)]);
            self.a.d32(i);
        } else {
            self.a.e(&[rex(0, r), 0xB8 | (r & 7)]);
            self.a.e(&v.to_le_bytes());
        }
    }
    fn mov_rr(&mut self, dst: u8, src: u8) {
        if dst != src {
            self.rr(&[0x89], src, dst);
        }
    }
    fn push_r(&mut self, r: u8) {
        if r >= 8 {
            self.a.e(&[0x41]);
        }
        self.a.e(&[0x50 | (r & 7)]);
    }
    fn pop_r(&mut self, r: u8) {
        if r >= 8 {
            self.a.e(&[0x41]);
        }
        self.a.e(&[0x58 | (r & 7)]);
    }
    fn setcc_rax(&mut self, cc: u8) {
        self.setcc_into(RAX, cc);
    }
    /// setcc r8; movzx r32, r8 (touches no other register)
    fn setcc_into(&mut self, r: u8, cc: u8) {
        let b = (r >> 3) & 1;
        self.a.e(&[0x40 | b, 0x0F, cc, 0xC0 | (r & 7)]);
        self.a.e(&[0x40 | b << 2 | b, 0x0F, 0xB6, 0xC0 | (r & 7) << 3 | (r & 7)]);
    }

    /// The value into register r (rax only once nothing else lives there).
    fn to_reg(&mut self, x: V, r: u8) {
        match x {
            V::Imm(c) => self.mov_imm(r, c),
            V::Slot(s) => match self.loc[s as usize] {
                Loc::Reg(y) => self.mov_rr(r, y),
                Loc::Mem(d) => self.rm(&[0x8B], r, d),
                Loc::Xmm(k) => self.movq_from_xmm(r, k),
            },
            V::Rax => self.mov_rr(r, RAX),
            V::Stack => self.pop_r(r),
            V::X(k) => self.movq_from_xmm(r, k),
            V::Flags(cc) => self.setcc_into(r, cc),
            V::Lea(s, c) => {
                let y = self.slot_reg(s);
                self.lea(r, y, c)
            }
            V::Scaled(s, 0, 0) => {
                let y = self.slot_reg(s);
                self.mov_rr(r, y)
            }
            V::Scaled(s, k, d) => {
                let y = self.slot_reg(s);
                self.mem(false, true, &[0x8D], r, Addr::Index(None, y, k, d));
            }
            V::Index(b, i, k, d) => {
                let (yb, yi) = (self.slot_reg(b), self.slot_reg(i));
                self.mem(false, true, &[0x8D], r, Addr::Index(Some(yb), yi, k, d));
            }
            V::RIdx(i, k, d) => {
                let yi = self.slot_reg(i);
                self.mem(false, true, &[0x8D], r, Addr::Index(Some(RAX), yi, k, d));
            }
            V::At(..) | V::AtIdx(..) => {
                let at = self.address(x);
                self.mem(false, true, &[0x8D], r, at);
            }
        }
    }

    /// Where global or frame memory is, plus index << k and d.
    fn at_addr(&self, b: B, idx: Option<(u32, u8)>, d: i32) -> Addr {
        let idx = idx.map(|(i, k)| (self.slot_reg(i), k));
        match (b, idx) {
            (B::G(o), idx) => Addr::Abs(idx, o.wrapping_add(d as u32)),
            (B::F(off), None) => Addr::Base(4, self.at(off) + d),
            (B::F(off), Some((i, k))) => Addr::Index(Some(4), i, k, self.at(off) + d),
        }
    }

    fn push_physical(&mut self, x: V) {
        match x {
            V::Imm(c) => {
                if let Ok(i) = i32::try_from(c) {
                    self.a.e(&[0x68]);
                    self.a.d32(i);
                } else {
                    self.mov_imm(R11, c);
                    self.push_r(R11);
                }
            }
            V::Slot(s) => match self.loc[s as usize] {
                Loc::Reg(y) => self.push_r(y),
                Loc::Mem(d) => {
                    let disp = self.at(d);
                    self.a.e(&[0xFF, 0xB4, 0x24]); // push qword [rsp + d]
                    self.a.d32(disp);
                }
                Loc::Xmm(k) => {
                    self.movq_from_xmm(R11, k);
                    self.push_r(R11);
                }
            },
            V::Rax => self.push_r(RAX),
            V::Flags(cc) => {
                self.setcc_into(R11, cc);
                self.push_r(R11);
            }
            V::Lea(..) | V::Scaled(..) | V::Index(..) | V::RIdx(..) | V::X(_) | V::At(..) | V::AtIdx(..) => {
                self.to_reg(x, R11);
                self.push_r(R11);
            }
            V::Stack => {}
        }
    }

    /// Everything up to index k onto the machine's stack, in order.
    fn flush_to(&mut self, k: usize) {
        let first = self.v.iter().position(|x| *x != V::Stack).unwrap_or(self.v.len());
        for i in first..=k.min(self.v.len().wrapping_sub(1)) {
            if i >= self.v.len() {
                break;
            }
            let x = self.v[i];
            self.push_physical(x);
            self.v[i] = V::Stack;
        }
    }
    fn flush(&mut self) {
        if !self.v.is_empty() {
            self.flush_to(self.v.len() - 1);
        }
    }
    fn need_rax(&mut self) {
        if let Some(i) = self.v.iter().rposition(|x| matches!(x, V::Rax | V::RIdx(..))) {
            self.flush_to(i);
        }
    }
    /// A free xmm register for a float to wait in, other than those in
    /// `avoid`: when all are taken, the deepest float waiting is pushed.
    fn alloc_xmm(&mut self, avoid: &[u8]) -> u8 {
        loop {
            let used = |k: u8, v: &[V]| v.contains(&V::X(k)) || avoid.contains(&k);
            if let Some(k) = XMMS.clone().find(|&k| !used(k, &self.v)) {
                return k;
            }
            let i = self.v.iter().position(|x| matches!(x, V::X(_))).expect("an xmm register is taken by a value");
            self.flush_to(i);
        }
    }
    fn push(&mut self, x: V) {
        if let Some(&V::Flags(cc)) = self.v.last() {
            self.v.pop();
            self.need_rax();
            self.setcc_rax(cc);
            self.v.push(V::Rax);
        }
        self.v.push(x);
    }
    fn pop(&mut self) -> V {
        self.v.pop().expect("the IR's stack is balanced")
    }
    fn depth(&self) -> u32 {
        self.v.len() as u32
    }

    /// The right operand of an operation with rax.
    fn operand(&mut self, x: V) -> Opd {
        match x {
            V::Imm(c) => match i32::try_from(c) {
                Ok(i) => Opd::Imm(i),
                Err(_) => {
                    self.mov_imm(RCX, c);
                    Opd::Reg(RCX)
                }
            },
            V::Slot(s) => match self.loc[s as usize] {
                Loc::Reg(y) => Opd::Reg(y),
                Loc::Mem(d) => Opd::Mem(d),
                Loc::Xmm(k) => {
                    self.movq_from_xmm(RCX, k);
                    Opd::Reg(RCX)
                }
            },
            other => {
                self.to_reg(other, RCX);
                Opd::Reg(RCX)
            }
        }
    }
    fn into_rcx(&mut self, o: Opd) {
        match o {
            Opd::Imm(i) => self.mov_imm(RCX, i as i64),
            Opd::Reg(r) => self.mov_rr(RCX, r),
            Opd::Mem(d) => self.rm(&[0x8B], RCX, d),
        }
    }
    /// add 0, or 1, and 4, sub 5, xor 6, cmp 7
    fn alu(&mut self, ext: u8, dst: u8, o: Opd) {
        match o {
            Opd::Imm(i) => {
                if let Ok(b) = i8::try_from(i) {
                    self.a.e(&[rex(0, dst), 0x83, 0xC0 | ext << 3 | (dst & 7), b as u8]);
                } else {
                    self.a.e(&[rex(0, dst), 0x81, 0xC0 | ext << 3 | (dst & 7)]);
                    self.a.d32(i);
                }
            }
            Opd::Reg(r) => self.rr(&[ext * 8 + 1], r, dst),
            Opd::Mem(d) => self.rm(&[ext * 8 + 3], dst, d),
        }
    }

    fn bin(&mut self, b: BinOp, done: &mut dyn FnMut(&mut Asm) -> u32) {
        if b.is_float() {
            self.float_bin(b);
            return;
        }
        // a local in a register plus or minus a constant waits as a Lea
        let n = self.v.len();
        if matches!(b, BinOp::Add | BinOp::Sub) && n >= 2 {
            if let V::Imm(c) = self.v[n - 1] {
                let c = if b == BinOp::Sub { c.checked_neg() } else { Some(c) };
                let lea = match (self.v[n - 2], c.and_then(|c| i32::try_from(c).ok())) {
                    (V::Slot(s), Some(c)) if matches!(self.loc[s as usize], Loc::Reg(_)) => Some(V::Lea(s, c)),
                    (V::Lea(s, d), Some(c)) => d.checked_add(c).map(|e| V::Lea(s, e)),
                    _ => None,
                };
                if let Some(x) = lea {
                    self.v.truncate(n - 2);
                    self.v.push(x);
                    return;
                }
            }
        }
        // an element's address: index << k, then base + that, waits to be
        // used as [base + index * size]
        if n >= 2 {
            let (l, r) = (self.v[n - 2], self.v[n - 1]);
            let reg = |s: u32| matches!(self.loc[s as usize], Loc::Reg(_));
            let lazy = match (b, l, r) {
                (BinOp::Shl, V::Slot(s), V::Imm(k @ 0..=3)) if reg(s) => Some(V::Scaled(s, k as u8, 0)),
                // (s + d) << k is (s << k) + (d << k): an unrolled copy's element
                (BinOp::Shl, V::Lea(s, d), V::Imm(k @ 0..=3)) if d.unsigned_abs() < 1 << 27 => Some(V::Scaled(s, k as u8, d << k)),
                (BinOp::Add, V::Slot(base), V::Scaled(i, k, d)) | (BinOp::Add, V::Scaled(i, k, d), V::Slot(base)) if reg(base) => Some(V::Index(base, i, k, d)),
                (BinOp::Add, V::Slot(base), V::Slot(i)) if reg(base) && reg(i) && base != i => Some(V::Index(base, i, 0, 0)),
                (BinOp::Add, V::Lea(base, d), V::Scaled(i, k, e)) | (BinOp::Add, V::Scaled(i, k, e), V::Lea(base, d)) => d.checked_add(e).map(|d| V::Index(base, i, k, d)),
                (BinOp::Add | BinOp::Sub, V::Index(base, i, k, d), V::Imm(c)) => {
                    let c = if b == BinOp::Sub { c.checked_neg() } else { Some(c) };
                    c.and_then(|c| i32::try_from(c).ok()).and_then(|c| d.checked_add(c)).map(|d| V::Index(base, i, k, d))
                }
                // global or frame memory: plus a constant, plus an index
                (BinOp::Add | BinOp::Sub, V::At(m, d), V::Imm(c)) => {
                    let c = if b == BinOp::Sub { c.checked_neg() } else { Some(c) };
                    c.and_then(|c| i32::try_from(c).ok()).and_then(|c| d.checked_add(c)).filter(|d| d.unsigned_abs() < 1 << 30).map(|d| V::At(m, d))
                }
                (BinOp::Add | BinOp::Sub, V::AtIdx(m, i, k, d), V::Imm(c)) => {
                    let c = if b == BinOp::Sub { c.checked_neg() } else { Some(c) };
                    c.and_then(|c| i32::try_from(c).ok()).and_then(|c| d.checked_add(c)).filter(|d| d.unsigned_abs() < 1 << 30).map(|d| V::AtIdx(m, i, k, d))
                }
                (BinOp::Add, V::At(m, d), V::Scaled(i, k, e)) | (BinOp::Add, V::Scaled(i, k, e), V::At(m, d)) => d.checked_add(e).filter(|d| d.unsigned_abs() < 1 << 30).map(|d| V::AtIdx(m, i, k, d)),
                (BinOp::Add, V::At(m, d), V::Lea(i, e)) | (BinOp::Add, V::Lea(i, e), V::At(m, d)) => d.checked_add(e).filter(|d| d.unsigned_abs() < 1 << 30).map(|d| V::AtIdx(m, i, 0, d)),
                (BinOp::Add, V::At(m, d), V::Slot(i)) | (BinOp::Add, V::Slot(i), V::At(m, d)) if reg(i) => Some(V::AtIdx(m, i, 0, d)),
                _ => None,
            };
            if let Some(x) = lazy {
                self.v.truncate(n - 2);
                self.v.push(x);
                return;
            }
            // a base in memory (or in rax already) plus index << k: the
            // base into rax, and the address waits as [rax + index * size]
            let mem = |s: u32| matches!(self.loc[s as usize], Loc::Mem(_));
            let base = match (b, l, r) {
                (BinOp::Add, V::Slot(s), V::Scaled(i, k, d)) | (BinOp::Add, V::Scaled(i, k, d), V::Slot(s)) if mem(s) => Some((Some(s), i, k, d)),
                (BinOp::Add, V::Rax, V::Scaled(i, k, d)) | (BinOp::Add, V::Scaled(i, k, d), V::Rax) => Some((None, i, k, d)),
                _ => None,
            };
            if let Some((s, i, k, d)) = base {
                self.v.truncate(n - 2);
                if let Some(s) = s {
                    self.need_rax();
                    self.to_reg(V::Slot(s), RAX);
                }
                self.v.push(V::RIdx(i, k, d));
                return;
            }
            if let (BinOp::Add | BinOp::Sub, V::RIdx(i, k, d), V::Imm(c)) = (b, l, r) {
                let c = if b == BinOp::Sub { c.checked_neg() } else { Some(c) };
                if let Some(d) = c.and_then(|c| i32::try_from(c).ok()).and_then(|c| d.checked_add(c)) {
                    self.v.truncate(n - 2);
                    self.v.push(V::RIdx(i, k, d));
                    return;
                }
            }
        }
        let mut right = self.pop();
        let mut left = self.pop();
        // a result already in rax stays there: these operations do not mind
        // the order
        if right == V::Rax && left != V::Rax && matches!(b, BinOp::Add | BinOp::Mul | BinOp::Band | BinOp::Bor | BinOp::Bxor) {
            std::mem::swap(&mut left, &mut right);
        }
        self.need_rax();
        let o = self.operand(right);
        if b.is_comparison() {
            let dst = match left {
                V::Slot(s) => match self.loc[s as usize] {
                    Loc::Reg(y) => y,
                    Loc::Mem(_) | Loc::Xmm(_) => {
                        self.to_reg(left, RAX);
                        RAX
                    }
                },
                _ => {
                    self.to_reg(left, RAX);
                    RAX
                }
            };
            if matches!(o, Opd::Imm(0)) {
                self.rr(&[0x85], dst, dst); // test: the same flags as cmp with 0
            } else {
                self.alu(7, dst, o);
            }
            let cc = match b {
                BinOp::Eq => 0x94,
                BinOp::Ne => 0x95,
                BinOp::Lt => 0x9C,
                BinOp::Le => 0x9E,
                BinOp::Gt => 0x9F,
                BinOp::Ult => 0x92,
                BinOp::Ule => 0x96,
                _ => 0x9D,
            };
            self.v.push(V::Flags(cc));
            return;
        }
        self.to_reg(left, RAX);
        let zero = matches!(o, Opd::Imm(0));
        if matches!(b, BinOp::AddOv | BinOp::SubOv | BinOp::MulOv | BinOp::ShlOv) {
            self.into_rcx(o);
            let t = self.trap();
            super::x64::checked(&mut self.a, Some(b), t);
            self.v.push(V::Rax);
            return;
        }
        match b {
            BinOp::Add | BinOp::Sub | BinOp::Bor | BinOp::Bxor if zero => {}
            BinOp::Add => self.alu(0, RAX, o),
            BinOp::Sub => self.alu(5, RAX, o),
            BinOp::Band => self.alu(4, RAX, o),
            BinOp::Bor => self.alu(1, RAX, o),
            BinOp::Bxor => self.alu(6, RAX, o),
            BinOp::Mul => match o {
                Opd::Imm(i) => {
                    self.rr(&[0x69], RAX, RAX);
                    self.a.d32(i);
                }
                Opd::Reg(r) => self.rr(&[0x0F, 0xAF], RAX, r),
                Opd::Mem(d) => self.rm(&[0x0F, 0xAF], RAX, d),
            },
            BinOp::Shl | BinOp::Shr => {
                let ext = if b == BinOp::Shl { 4 } else { 7 };
                match o {
                    Opd::Imm(i) => self.a.e(&[0x48, 0xC1, 0xC0 | ext << 3, (i & 63) as u8]),
                    o => {
                        self.into_rcx(o);
                        self.a.e(&[0x48, 0xD3, 0xC0 | ext << 3]);
                    }
                }
            }
            BinOp::DivTrunc | BinOp::ModTrunc | BinOp::DivFloor | BinOp::ModFloor
                if self.magic && self.magic_at.get(self.op_at) == Some(&true) && matches!(o, Opd::Imm(d) if d.unsigned_abs() >= 2) =>
            {
                let Opd::Imm(d) = o else { unreachable!() };
                let (m, s) = magic(d as i64);
                self.a.e(&[0x49, 0x89, 0xC0]); // mov r8, rax (n)
                self.mov_imm(RCX, m); // (read from memory instead: measured no faster)
                self.a.e(&[0x48, 0xF7, 0xE9]); // imul rcx: rdx = high(n * m)
                if d > 0 && m < 0 {
                    self.a.e(&[0x4C, 0x01, 0xC2]); // add rdx, r8
                } else if d < 0 && m > 0 {
                    self.a.e(&[0x4C, 0x29, 0xC2]); // sub rdx, r8
                }
                if s > 0 {
                    self.a.e(&[0x48, 0xC1, 0xFA, s as u8]); // sar rdx, s
                }
                self.a.e(&[0x48, 0x89, 0xD0, 0x48, 0xC1, 0xE8, 0x3F, 0x48, 0x01, 0xD0]); // rax = q (+1 if negative)
                if b == BinOp::ModTrunc {
                    // the remainder is n - q * d, and nothing more is needed
                    self.a.e(&[0x48, 0x69, 0xC0]); // imul rax, rax, d
                    self.a.d32(d);
                    self.a.e(&[0x49, 0x29, 0xC0, 0x4C, 0x89, 0xC0]); // sub r8, rax; mov rax, r8 (one step shorter than neg, add)
                    self.v.push(V::Rax);
                    return;
                }
                if b != BinOp::DivTrunc {
                    // rdx = n - q * d, rcx = d: as idiv leaves them
                    self.a.e(&[0x48, 0x89, 0xC2, 0x48, 0x69, 0xD2]); // mov rdx, rax; imul rdx, rdx, d
                    self.a.d32(d);
                    self.a.e(&[0x4C, 0x89, 0xC1, 0x48, 0x29, 0xD1, 0x48, 0x89, 0xCA]); // rdx = n - rdx
                    self.mov_imm(RCX, d as i64);
                }
                if matches!(b, BinOp::DivFloor | BinOp::ModFloor) {
                    let l = done(&mut self.a);
                    self.a.e(&[0x48, 0x85, 0xD2]);
                    self.a.jcc(4, l);
                    self.a.e(&[0x49, 0x89, 0xD0, 0x49, 0x31, 0xC8]);
                    self.a.jcc(9, l);
                    self.a.e(&[0x48, 0xFF, 0xC8, 0x48, 0x01, 0xCA]);
                    self.a.bind(l);
                }
                if matches!(b, BinOp::ModTrunc | BinOp::ModFloor) {
                    self.mov_rr(RAX, 2);
                }
            }
            BinOp::DivTrunc | BinOp::ModTrunc | BinOp::DivFloor | BinOp::ModFloor => {
                self.into_rcx(o);
                if self.div32 {
                    // both at least zero and below 2^32: the 32-bit unsigned
                    // division gives the same quotient and remainder, and is
                    // the faster instruction
                    let (wide, done32) = (done(&mut self.a), done(&mut self.a));
                    self.a.e(&[0x48, 0x89, 0xC2, 0x48, 0x09, 0xCA, 0x48, 0xC1, 0xEA, 0x20]); // rdx = (rax | rcx) >> 32
                    self.a.jcc(5, wide);
                    self.a.e(&[0x31, 0xD2, 0xF7, 0xF1]); // xor edx, edx; div ecx
                    self.a.jmp(done32);
                    self.a.bind(wide);
                    self.a.e(&[0x48, 0x99, 0x48, 0xF7, 0xF9]); // cqo; idiv rcx
                    self.a.bind(done32);
                } else {
                    self.a.e(&[0x48, 0x99, 0x48, 0xF7, 0xF9]); // cqo; idiv rcx
                }
                if matches!(b, BinOp::DivFloor | BinOp::ModFloor) {
                    let l = done(&mut self.a);
                    self.a.e(&[0x48, 0x85, 0xD2]);
                    self.a.jcc(4, l);
                    self.a.e(&[0x49, 0x89, 0xD0, 0x49, 0x31, 0xC8]);
                    self.a.jcc(9, l);
                    self.a.e(&[0x48, 0xFF, 0xC8, 0x48, 0x01, 0xCA]);
                    self.a.bind(l);
                }
                if matches!(b, BinOp::ModTrunc | BinOp::ModFloor) {
                    self.mov_rr(RAX, 2);
                }
            }
            _ => unreachable!("logic operators are lowered to jumps"),
        }
        self.v.push(V::Rax);
    }

    /// A float operation in xmm registers: each side taken from where it
    /// is (a float waiting in its xmm register, a local in memory straight
    /// from there) and the result left in the left side's register. A
    /// comparison leaves its flags. FRem (the x87's) and a side on the
    /// machine's stack take rax and rcx, as the plain generator does.
    fn float_bin(&mut self, b: BinOp) {
        let right = self.pop();
        let left = self.pop();
        self.need_rax();
        if b == BinOp::FRem || left == V::Stack || right == V::Stack {
            self.to_reg(right, RCX);
            self.to_reg(left, RAX);
            match super::x64::float_compare(&mut self.a, b) {
                Some(cc) => self.v.push(V::Flags(cc)),
                None => self.v.push(V::Rax),
            }
            return;
        }
        let op = match b {
            BinOp::FAdd => 0x58,
            BinOp::FMul => 0x59,
            BinOp::FSub => 0x5C,
            BinOp::FDiv => 0x5E,
            _ => 0,
        };
        // the left side's register, which the result takes: a float local's
        // own where the local is not read again
        let a = match left {
            V::X(k) => k,
            V::Slot(s) if matches!(self.loc[s as usize], Loc::Xmm(_)) && self.dead(s) => {
                let Loc::Xmm(k) = self.loc[s as usize] else { unreachable!() };
                k
            }
            x => {
                let avoid = if let V::X(k) = right { vec![k] } else { vec![] };
                let k = self.alloc_xmm(&avoid);
                self.to_xmm(x, k);
                k
            }
        };
        // arithmetic takes a local in memory, and a constant, as they are
        if let (V::Slot(s), true) = (right, op != 0) {
            if let Loc::Mem(d) = self.loc[s as usize] {
                self.sse_rsp(Some(0xF2), op, a, d); // op xmm, [rsp + d]
                self.v.push(V::X(a));
                return;
            }
        }
        if let (V::Imm(c), true) = (right, op != 0) {
            self.sse_const(Some(0xF2), op, a, c); // op xmm, [rip + c]
            self.v.push(V::X(a));
            return;
        }
        // the right side's register: a float local's own, used where it is
        let c = match right {
            V::X(k) => k,
            V::Slot(s) if matches!(self.loc[s as usize], Loc::Xmm(_)) => {
                let Loc::Xmm(k) = self.loc[s as usize] else { unreachable!() };
                k
            }
            x => {
                self.to_xmm(x, 1);
                1
            }
        };
        match b {
            BinOp::FLt | BinOp::FLe => {
                self.sse_rr(Some(0x66), 0x2E, c, a); // ucomisd right, left: a < b is b above a
                self.v.push(V::Flags(if b == BinOp::FLt { 0x97 } else { 0x93 }));
            }
            BinOp::FGt | BinOp::FGe => {
                self.sse_rr(Some(0x66), 0x2E, a, c); // ucomisd left, right
                self.v.push(V::Flags(if b == BinOp::FGt { 0x97 } else { 0x93 }));
            }
            BinOp::FEq | BinOp::FNe => {
                // cmpsd's mask, made 0 or 1 in rax
                self.sse_rr(Some(0xF2), 0xC2, a, c);
                self.a.e(&[if b == BinOp::FEq { 0 } else { 4 }]);
                self.movq_from_xmm(RAX, a);
                self.a.e(&[0x83, 0xE0, 0x01]); // and eax, 1
                self.v.push(V::Rax);
            }
            _ => {
                self.sse_rr(Some(0xF2), op, a, c);
                self.v.push(V::X(a));
            }
        }
    }

    /// A float operation whose result goes straight into a float local in
    /// an xmm register (`s = s + x` is one addsd); false where the shape
    /// does not allow it, and nothing is emitted then.
    fn float_into(&mut self, b: BinOp, s: u32) -> bool {
        let op = match b {
            BinOp::FAdd => 0x58,
            BinOp::FMul => 0x59,
            BinOp::FSub => 0x5C,
            BinOp::FDiv => 0x5E,
            _ => return false,
        };
        let Loc::Xmm(k) = self.loc[s as usize] else { return false };
        let n = self.v.len();
        if n < 2 {
            return false;
        }
        // the sides stay in their order (x + s is not s + x: with two NaNs
        // the first one comes out)
        let (left, right) = (self.v[n - 2], self.v[n - 1]);
        let ok = |x: V| !matches!(x, V::Stack | V::Flags(_)) && x != V::X(k);
        if !ok(left) || !ok(right) || (right == V::Slot(s) && left != V::Slot(s)) {
            return false;
        }
        self.v.truncate(n - 2);
        if let Some(i) = self.reads(s) {
            self.flush_to(i);
        }
        self.keep_xmm(k);
        // the right side where it is: held first when it is a pending
        // float or the local itself, before the local is written
        let c = match right {
            V::X(j) => j,
            V::Slot(t) if matches!(self.loc[t as usize], Loc::Xmm(_)) && t != s => {
                let Loc::Xmm(j) = self.loc[t as usize] else { unreachable!() };
                j
            }
            V::Slot(t) if t == s => k,
            V::Slot(t) if matches!(self.loc[t as usize], Loc::Mem(_)) => {
                if left != V::Slot(s) {
                    self.to_xmm_any(left, k);
                }
                let Loc::Mem(d) = self.loc[t as usize] else { unreachable!() };
                self.sse_rsp(Some(0xF2), op, k, d);
                return true;
            }
            V::Imm(c) => {
                if left != V::Slot(s) {
                    self.to_xmm_any(left, k);
                }
                self.sse_const(Some(0xF2), op, k, c);
                return true;
            }
            x => {
                self.to_xmm(x, 1);
                1
            }
        };
        if left != V::Slot(s) {
            self.to_xmm_any(left, k);
        }
        self.sse_rr(Some(0xF2), op, k, c);
        true
    }

    /// Any value (not on the machine's stack) into xmm register `xmm`.
    fn to_xmm_any(&mut self, x: V, xmm: u8) {
        match x {
            V::X(j) => self.sse_rr(None, 0x28, xmm, j), // movaps
            x => self.to_xmm(x, xmm),
        }
    }

    /// An element that is at once the right side of float arithmetic: the
    /// operation takes it from memory (`mulsd xmm, [address]`). False, and
    /// nothing emitted, where the left side is not where this can be done.
    /// With `into`, a float local in an xmm register the result is stored
    /// into at once: worked out there (nothing is pushed then).
    fn load_into_op(&mut self, b: BinOp, into: Option<u32>) -> bool {
        let op = match b {
            BinOp::FAdd => 0x58,
            BinOp::FMul => 0x59,
            BinOp::FSub => 0x5C,
            BinOp::FDiv => 0x5E,
            _ => return false,
        };
        let n = self.v.len();
        if n < 2 || matches!(self.v[n - 2], V::Stack | V::Flags(_)) || self.v[n - 1] == V::Stack {
            return false;
        }
        let addr = self.pop();
        let left = self.pop();
        if let Some(s) = into {
            let Loc::Xmm(k) = self.loc[s as usize] else { unreachable!() };
            if let Some(i) = self.reads(s) {
                self.flush_to(i);
            }
            self.keep_xmm(k);
            match left {
                V::Slot(t) if t == s => {}
                x => self.to_xmm_any(x, k),
            }
            let at = self.address(addr);
            self.a.e(&[0xF2]);
            self.mem(false, false, &[0x0F, op], k, at);
            return true;
        }
        let a = match left {
            V::X(k) => k,
            V::Slot(s) if matches!(self.loc[s as usize], Loc::Xmm(_)) && self.dead(s) => {
                let Loc::Xmm(k) = self.loc[s as usize] else { unreachable!() };
                self.keep_xmm(k);
                k
            }
            x => {
                let k = self.alloc_xmm(&[]);
                self.to_xmm(x, k);
                k
            }
        };
        let at = self.address(addr);
        self.a.e(&[0xF2]);
        self.mem(false, false, &[0x0F, op], a, at); // op xmm, [address]
        self.v.push(V::X(a));
        true
    }

    /// An element a float operation is to use, loaded from memory straight
    /// into an xmm register (rax stays free for the next one's address).
    fn load_float(&mut self) {
        let x = self.pop();
        let k = self.alloc_xmm(&[]);
        let at = self.address(x);
        self.a.e(&[0xF2]);
        self.mem(false, false, &[0x0F, 0x10], k, at); // movsd xmm, [address]
        self.v.push(V::X(k));
    }

    /// A value (not on the machine's stack, not waiting in an xmm register)
    /// into xmm register `xmm`.
    fn to_xmm(&mut self, x: V, xmm: u8) {
        match x {
            V::Slot(s) => match self.loc[s as usize] {
                Loc::Mem(d) => self.sse_rsp(Some(0xF2), 0x10, xmm, d), // movsd xmm, [rsp + d]
                Loc::Xmm(k) => self.sse_rr(None, 0x28, xmm, k),        // movaps xmm, xmm
                Loc::Reg(r) => self.movq_to_xmm(xmm, r),
            },
            V::Imm(0) => self.sse_rr(None, 0x57, xmm, xmm), // xorps xmm, xmm
            V::Imm(c) => self.sse_const(Some(0xF2), 0x10, xmm, c), // movsd xmm, [rip + c]
            V::Rax => self.movq_to_xmm(xmm, RAX),
            x => {
                self.to_reg(x, R11);
                self.movq_to_xmm(xmm, R11);
            }
        }
    }

    fn store(&mut self, s: u32) {
        let x = self.pop();
        if let Some(i) = self.reads(s) {
            self.flush_to(i);
        }
        match self.loc[s as usize] {
            Loc::Reg(r) => self.to_reg(x, r),
            Loc::Mem(d) => match x {
                V::Imm(c) if i32::try_from(c).is_ok() => {
                    self.rm(&[0xC7], 0, d);
                    self.a.d32(c as i32);
                }
                V::Stack => {
                    // pop's address is taken after rsp has gone up: what
                    // phys counts now, the value already popped
                    let disp = self.at(d);
                    self.a.e(&[0x8F, 0x84, 0x24]); // pop qword [rsp + d]
                    self.a.d32(disp);
                }
                V::Slot(t) if t == s => {}
                V::Slot(t) => match self.loc[t as usize] {
                    Loc::Reg(y) => self.rm(&[0x89], y, d),
                    Loc::Mem(_) => {
                        self.to_reg(x, R11);
                        self.rm(&[0x89], R11, d);
                    }
                    Loc::Xmm(k) => self.sse_rsp(Some(0xF2), 0x11, k, d), // movsd [rsp + d], xmm
                },
                V::X(k) => self.sse_rsp(Some(0xF2), 0x11, k, d),
                V::Rax => self.rm(&[0x89], RAX, d),
                other => {
                    self.to_reg(other, R11);
                    self.rm(&[0x89], R11, d);
                }
            },
            Loc::Xmm(k) => match x {
                _ if self.v.contains(&V::X(k)) => {
                    // a result waiting in this register is kept first
                    self.keep_xmm(k);
                    self.v.push(x);
                    self.store(s);
                }
                V::Slot(t) if t == s => {}
                V::Imm(0) => self.sse_rr(None, 0x57, k, k), // xorps
                V::X(j) => self.sse_rr(None, 0x28, k, j),   // movaps
                V::Slot(t) => match self.loc[t as usize] {
                    Loc::Xmm(j) => self.sse_rr(None, 0x28, k, j),
                    Loc::Mem(d) => self.sse_rsp(Some(0xF2), 0x10, k, d), // movsd xmm, [rsp + d]
                    Loc::Reg(y) => self.movq_to_xmm(k, y),
                },
                V::Rax => self.movq_to_xmm(k, RAX),
                other => {
                    self.to_reg(other, R11);
                    self.movq_to_xmm(k, R11);
                }
            },
        }
    }

    /// A call whose arguments go straight to rcx, rdx, r8, r9 and the
    /// stack slots above the shadow space, from wherever they are: nothing
    /// is pushed but what must outlive the call (a result in rax below the
    /// arguments). Arguments already on the machine's stack take the
    /// plain way.
    fn call_direct(&mut self, sym: &str, n: usize) {
        // a comparison's flags waiting under a call of no arguments (`(a <
        // b) ^ f()`) become a number first: the callee changes the flags
        // (with arguments, pushing the first one has made it one already)
        if n == 0 {
            if let Some(&V::Flags(cc)) = self.v.last() {
                self.v.pop();
                self.need_rax();
                self.setcc_rax(cc);
                self.v.push(V::Rax);
            }
        }
        let at = self.v.len() - n;
        if self.v[at..].contains(&V::Stack) {
            self.flush();
            let mut d = self.depth();
            call(&mut self.a, sym, n as u32, &mut d, false);
            self.v.truncate(d as usize);
            self.v.push(V::Rax);
            return;
        }
        let args: Vec<V> = self.v.split_off(at);
        if let Some(i) = self.v.iter().rposition(|x| matches!(x, V::Rax | V::RIdx(..) | V::X(_))) {
            self.flush_to(i);
        }
        // a comparison's flags become a number before anything changes them
        let flags_in_r10 = if let Some(&V::Flags(cc)) = args.last() {
            self.setcc_into(R10, cc);
            true
        } else {
            false
        };
        let physical = self.v.iter().filter(|x| **x == V::Stack).count() as u32;
        let k = n.saturating_sub(4) as u32;
        // with nothing pushed, the frame's own call area is at rsp, aligned
        let mut s = if physical == 0 { 0 } else { 32 + 8 * k };
        if s > 0 && (physical + s / 8) % 2 == 1 {
            s += 8;
        }
        if s > 0 {
            self.a.sub_rsp(s as i32);
        }
        self.extra = s as i32;
        // no source is an argument register, so the order does not matter
        for (i, x) in args.iter().enumerate() {
            let src = if flags_in_r10 && i == n - 1 { None } else { Some(*x) };
            if i < 4 {
                match src {
                    None => self.mov_rr(ARGS[i], R10),
                    Some(x) => self.to_reg(x, ARGS[i]),
                }
            } else {
                let disp = 32 + 8 * (i as i32 - 4);
                let reg = match src {
                    None => R10,
                    Some(V::Rax) => RAX,
                    Some(V::Imm(c)) if i32::try_from(c).is_ok() => {
                        self.a.e(&[0x48, 0xC7, 0x84, 0x24]); // mov qword [rsp + d], imm32
                        self.a.d32(disp);
                        self.a.d32(c as i32);
                        continue;
                    }
                    Some(V::Slot(t)) if matches!(self.loc[t as usize], Loc::Reg(_)) => {
                        let Loc::Reg(y) = self.loc[t as usize] else { unreachable!() };
                        y
                    }
                    Some(x) => {
                        self.to_reg(x, R11);
                        R11
                    }
                };
                self.a.e(&[rex(reg, 0), 0x89, 0x84 | (reg & 7) << 3, 0x24]); // mov [rsp + d], reg
                self.a.d32(disp);
            }
        }
        self.extra = 0;
        self.a.call(sym);
        if s > 0 {
            self.a.add_rsp(s as i32);
        }
        self.v.push(V::Rax);
    }

    /// A call of the runtime whose arguments are already in rcx (and rdx).
    fn call_runtime(&mut self, sym: &str) {
        let mut d = self.depth();
        if d == 0 {
            self.a.call(sym);
        } else {
            call(&mut self.a, sym, 0, &mut d, false);
        }
    }

    /// An operation whose result is stored at once into a local that lives
    /// in a register: worked out in that register (`x = x - 2` is one
    /// `sub`), leaving rax and what it holds alone. False when the shape
    /// does not allow it; nothing is emitted then.
    fn bin_into(&mut self, b: BinOp, s: u32) -> bool {
        if b.is_float() {
            return self.float_into(b, s);
        }
        let ext = match b {
            BinOp::Add => 0,
            BinOp::Bor => 1,
            BinOp::Band => 4,
            BinOp::Sub => 5,
            BinOp::Bxor => 6,
            BinOp::Mul => 8,
            BinOp::Shl => 9,
            BinOp::Shr => 10,
            _ => return false,
        };
        let Loc::Reg(d) = self.loc[s as usize] else { return false };
        let n = self.v.len();
        if n < 2 {
            return false;
        }
        let (mut left, mut right) = (self.v[n - 2], self.v[n - 1]);
        // `acc = x + acc` is `acc += x`: these operations do not mind the order
        if right == V::Slot(s) && left != V::Slot(s) && !matches!(ext, 5 | 9 | 10) {
            std::mem::swap(&mut left, &mut right);
        }
        let fits = |x: V| match x {
            V::Imm(c) => i32::try_from(c).is_ok(),
            V::Slot(_) | V::Rax => true,
            _ => false,
        };
        // a shift takes a constant count (a variable one needs cl)
        if !fits(left) || !fits(right) || (right == V::Slot(s) && left != V::Slot(s)) || (ext >= 9 && !matches!(right, V::Imm(_))) {
            return false;
        }
        self.v.truncate(n - 2);
        if let Some(i) = self.reads(s) {
            self.flush_to(i);
        }
        // d = left * c in one imul, from where left is
        if let (8, V::Slot(t), V::Imm(c)) = (ext, left, right) {
            if t != s {
                match self.loc[t as usize] {
                    Loc::Reg(y) => self.rr(&[0x69], d, y),
                    Loc::Mem(m) => self.rm(&[0x69], d, m),
                    Loc::Xmm(k) => {
                        self.movq_from_xmm(d, k);
                        self.rr(&[0x69], d, d);
                    }
                }
                self.a.d32(c as i32);
                return true;
            }
        }
        if left != V::Slot(s) {
            self.to_reg(left, d);
        }
        if ext >= 9 {
            let V::Imm(c) = right else { unreachable!() };
            if c & 63 != 0 {
                self.a.e(&[rex(0, d), 0xC1, 0xC0 | (if ext == 9 { 4 } else { 7 }) << 3 | (d & 7), (c & 63) as u8]);
            }
            return true;
        }
        let o = match right {
            V::Imm(c) => Opd::Imm(c as i32),
            V::Rax => Opd::Reg(RAX),
            V::Slot(t) => match self.loc[t as usize] {
                Loc::Reg(y) => Opd::Reg(y),
                Loc::Mem(m) => Opd::Mem(m),
                Loc::Xmm(k) => {
                    self.movq_from_xmm(RCX, k);
                    Opd::Reg(RCX)
                }
            },
            _ => unreachable!(),
        };
        match (ext, o) {
            (8, Opd::Imm(i)) => {
                self.rr(&[0x69], d, d);
                self.a.d32(i);
            }
            (8, Opd::Reg(r)) => self.rr(&[0x0F, 0xAF], d, r),
            (8, Opd::Mem(m)) => self.rm(&[0x0F, 0xAF], d, m),
            (0 | 1 | 5 | 6, Opd::Imm(0)) => {}
            (e, o) => self.alu(e, d, o),
        }
        true
    }

    /// An address: locals in registers (plus a constant, or base + index
    /// times a size) are used where they are, anything else is worked out
    /// into rax.
    fn address(&mut self, x: V) -> Addr {
        match x {
            V::Lea(s, c) => Addr::Base(self.slot_reg(s), c),
            V::Slot(s) if self.in_reg(s) => Addr::Base(self.slot_reg(s), 0),
            V::Scaled(s, k, d) => Addr::Index(None, self.slot_reg(s), k, d),
            V::Index(b, i, k, d) => Addr::Index(Some(self.slot_reg(b)), self.slot_reg(i), k, d),
            V::RIdx(i, k, d) => Addr::Index(Some(RAX), self.slot_reg(i), k, d),
            V::At(b, d) => self.at_addr(b, None, d),
            V::AtIdx(b, i, k, d) => self.at_addr(b, Some((i, k)), d),
            x => {
                self.need_rax();
                self.to_reg(x, RAX);
                Addr::Base(RAX, 0)
            }
        }
    }

    /// The w bytes at the address on top, sign-extended into rax.
    fn load_mem(&mut self, w: u8) {
        let x = self.pop();
        // rax made free first: a push moves rsp, which a frame address counts
        self.need_rax();
        let at = self.address(x);
        let op: &[u8] = match w {
            1 => &[0x0F, 0xBE],
            2 => &[0x0F, 0xBF],
            4 => &[0x63],
            _ => &[0x8B],
        };
        self.mem(false, true, op, RAX, at);
        self.v.push(V::Rax);
    }

    /// The value on top written to the address under it, or (op) worked
    /// into what is there: one instruction on the memory either way.
    fn store_mem(&mut self, w: u8, op: Option<BinOp>) {
        let x = self.pop();
        let a = self.pop();
        // a float waiting in an xmm register: stored from there
        if let (V::X(k), 8) = (x, w) {
            let at = self.address(a);
            self.a.e(&[0xF2]);
            self.mem(false, false, &[0x0F, 0x11], k, at); // movsd [address], xmm
            return;
        }
        let imm = match x {
            V::Imm(c) => i32::try_from(c).ok(),
            _ => None,
        };
        // an address that needs no register of its own leaves rax alone
        let in_place = match a {
            V::Lea(..) | V::Scaled(..) | V::Index(..) | V::At(..) | V::AtIdx(..) => true,
            V::Slot(s) => self.in_reg(s),
            _ => false,
        };
        let mut src = None;
        if imm.is_none() {
            src = Some(match x {
                V::Slot(s) if self.in_reg(s) => self.slot_reg(s),
                V::Rax if in_place => RAX,
                // the address is popped first where it is pushed, so every
                // later address counts what is really pushed
                x if a != V::Stack || matches!(x, V::Rax | V::Flags(_) | V::Stack) => {
                    self.to_reg(x, R11);
                    R11
                }
                _ => R11, // worked out after the address
            });
        }
        let at = self.address(a);
        if let (Some(R11), true) = (src, a == V::Stack && !matches!(x, V::Rax | V::Flags(_) | V::Stack)) {
            self.to_reg(x, R11);
        }
        // (r/m, r) opcode for 8 bits, and the /ext of the immediate form
        let (byte_op, ext) = match op {
            None => (0x88u8, 0u8),
            Some(BinOp::Add | BinOp::AddOv) => (0x00, 0),
            Some(BinOp::Bor) => (0x08, 1),
            Some(BinOp::Band) => (0x20, 4),
            Some(BinOp::Sub | BinOp::SubOv) => (0x28, 5),
            Some(BinOp::Bxor) => (0x30, 6),
            Some(b) => unreachable!("{b:?} is not worked in memory"),
        };
        match (imm, src) {
            (Some(c), _) => {
                let opc: u8 = match (op, w) {
                    (None, 1) => 0xC6,
                    (None, _) => 0xC7,
                    (Some(_), 1) => 0x80,
                    (Some(_), _) => 0x81,
                };
                self.mem(w == 2, w == 8, &[opc], ext, at);
                match w {
                    1 => self.a.e(&[c as u8]),
                    2 => self.a.e(&(c as i16).to_le_bytes()),
                    _ => self.a.d32(c),
                }
            }
            (None, Some(r)) => {
                let opc = if w == 1 { byte_op } else { byte_op + 1 };
                self.mem(w == 2, w == 8, &[opc], r, at);
            }
            (None, None) => unreachable!(),
        }
        if matches!(op, Some(BinOp::AddOv | BinOp::SubOv)) {
            let t = self.trap();
            self.a.jcc(0, t); // jo
        }
    }
    fn trap(&mut self) -> u32 {
        match self.trap_at {
            Some(t) => t,
            None => {
                let t = self.a.label();
                self.trap_at = Some(t);
                t
            }
        }
    }

    /// rax made free for a new value: flags on top become a number first,
    /// and a result already in rax is pushed.
    fn fresh_rax(&mut self) {
        if let Some(&V::Flags(cc)) = self.v.last() {
            self.v.pop();
            self.need_rax();
            self.setcc_rax(cc);
            self.v.push(V::Rax);
        }
        self.need_rax();
    }

    fn sext(&mut self, n: u8) {
        match self.pop() {
            V::Imm(c) => self.v.push(V::Imm(super::ir::sext(c, n))),
            x => {
                self.need_rax();
                self.to_reg(x, RAX);
                super::x64::sext_reg(&mut self.a, RAX, n);
                self.v.push(V::Rax);
            }
        }
    }

    fn un(&mut self, u: UnOp) {
        let x = self.pop();
        match (u, x) {
            // a square root where the float waits
            (UnOp::FSqrt, V::X(k)) => {
                self.sse_rr(Some(0xF2), 0x51, k, k); // sqrtsd xmm, xmm
                self.v.push(V::X(k));
            }
            // a whole number made a float straight in an xmm register
            (UnOp::FromInt | UnOp::FromInt32, x) if !matches!(x, V::Stack | V::Flags(_)) => {
                let k = self.alloc_xmm(&[]);
                let r = match x {
                    V::Slot(s) if self.in_reg(s) => self.slot_reg(s),
                    V::Rax => RAX,
                    x => {
                        self.to_reg(x, R11);
                        R11
                    }
                };
                self.sse_rr(None, 0x57, k, k); // xorps: no wait on the register's old value
                // cvtsi2sd (cvtsi2ss) xmm, r64
                let p = if u == UnOp::FromInt { 0xF2 } else { 0xF3 };
                self.a.e(&[p, 0x48 | (k >> 3) << 2 | (r >> 3), 0x0F, 0x2A, 0xC0 | (k & 7) << 3 | (r & 7)]);
                if u == UnOp::FromInt32 {
                    self.sse_rr(Some(0xF3), 0x5A, k, k); // cvtss2sd
                }
                self.v.push(V::X(k));
            }
            // of a float local: into a new xmm register, from its own
            (UnOp::FSqrt, V::Slot(s)) if matches!(self.loc[s as usize], Loc::Xmm(_)) => {
                let Loc::Xmm(j) = self.loc[s as usize] else { unreachable!() };
                let k = self.alloc_xmm(&[]);
                self.sse_rr(Some(0xF2), 0x51, k, j); // sqrtsd xmm, xmm
                self.v.push(V::X(k));
            }
            (u, x) if u.is_float() => {
                self.need_rax();
                self.to_reg(x, RAX);
                super::x64::float_un(&mut self.a, u);
                self.v.push(V::Rax);
            }
            (UnOp::Plus, x) => self.v.push(x),
            (UnOp::Not, V::Flags(cc)) => self.v.push(V::Flags(cc ^ 1)),
            (UnOp::Not, x) => {
                self.need_rax();
                self.to_reg(x, RAX);
                self.a.e(&[0x48, 0x85, 0xC0]);
                self.v.push(V::Flags(0x94));
            }
            (UnOp::NegOv, x) => {
                self.need_rax();
                self.to_reg(x, RAX);
                let t = self.trap();
                super::x64::checked(&mut self.a, None, t);
                self.v.push(V::Rax);
            }
            (u, x) => {
                self.need_rax();
                self.to_reg(x, RAX);
                self.a.e(if u == UnOp::Neg { &[0x48, 0xF7, 0xD8] } else { &[0x48, 0xF7, 0xD0] });
                self.v.push(V::Rax);
            }
        }
    }

    /// The fast path's operations (fast_path checked them).
    fn fast_ops(&mut self, ops: &[Op]) {
        for op in ops {
            match op {
                Op::Push(c) => self.push(V::Imm(*c)),
                Op::Load(s) => self.push(V::Slot(*s)),
                Op::Bin(b) => self.bin(*b, &mut |a: &mut Asm| a.label()),
                Op::Un(u) => self.un(*u),
                Op::Sext(n) => self.sext(*n),
                _ => unreachable!("fast_path takes only plain operations"),
            }
        }
    }

    /// The value on top into a register and tested: a jump to `to` when it
    /// is zero (`on_zero`) or when it is not.
    fn branch(&mut self, on_zero: bool, to: u32) {
        match self.pop() {
            V::Flags(cc) => self.a.jcc(if on_zero { (cc ^ 1) & 0x0F } else { cc & 0x0F }, to),
            x => {
                let r = match x {
                    V::Slot(s) => match self.loc[s as usize] {
                        Loc::Reg(y) => y,
                        Loc::Mem(_) | Loc::Xmm(_) => {
                            self.need_rax();
                            self.to_reg(x, RAX);
                            RAX
                        }
                    },
                    x => {
                        self.need_rax();
                        self.to_reg(x, RAX);
                        RAX
                    }
                };
                self.rr(&[0x85], r, r);
                self.a.jcc(if on_zero { 4 } else { 5 }, to);
            }
        }
    }

    /// rsp back above the frame and what is pushed, the saved registers
    /// popped, ret.
    fn epilogue(&mut self) {
        for (k, d) in self.xsaved.clone() {
            self.sse_rsp(None, 0x10, k, d); // movups xmm, [rsp + d]
        }
        let up = self.frame + 8 * self.phys();
        if up > 0 {
            self.a.add_rsp(up);
        }
        for r in self.saved.clone().into_iter().rev() {
            self.pop_r(r);
        }
        self.a.e(&[0xC3]);
    }

    /// What a label expects: with `carry`, the top value in rax and the rest
    /// pushed; without, everything pushed.
    fn settle(&mut self, carry: bool) {
        if carry {
            let x = self.pop();
            self.flush();
            self.to_reg(x, RAX);
            self.v.push(V::Rax);
        } else {
            self.flush();
        }
    }
}

/// A function that begins by testing its parameters and returning at once
/// (fib's `if (n < 2) return n`): the test and that return, worked out from
/// the argument registers before any frame is made or register saved. The
/// slots other than the parameters hold, at the start, the constants the
/// function's first operations store in them (zero otherwise), so their
/// loads become those constants. Only what cannot touch an argument
/// register is taken: loads of the first four parameters, constants, and
/// operations whose right side is a constant or a parameter (no division,
/// no shift by a variable, no call).
struct Fast {
    /// the test, ending where the branch to the rest of the function is
    test: Vec<Op>,
    /// true: the rest runs when the test is zero (the IR's Jz)
    rest_on_zero: bool,
    /// the return's value
    value: Vec<Op>,
    /// where the test begins: before it, only stores of constants and labels
    test_at: usize,
    /// the IR label the rest of the function begins at
    rest: u32,
}

fn fast_path(f: &Func) -> Option<Fast> {
    let mut known = vec![0i64; f.slots as usize];
    let mut i = 0;
    let code = &f.code;
    // stores of constants and labels the start passes through
    loop {
        match (code.get(i), code.get(i + 1)) {
            (Some(Op::Push(c)), Some(Op::Store(s))) if *s >= f.params => {
                known[*s as usize] = *c;
                i += 2;
            }
            (Some(Op::Label(_)), _) => i += 1,
            _ => break,
        }
    }
    // a run of plain operations, checked on a stack of what each value is
    #[derive(Clone, Copy, PartialEq)]
    enum K {
        Imm,
        Param,
        Done,
    }
    let plain = |from: usize, ops: &mut Vec<Op>| -> Option<(usize, usize)> {
        let mut st: Vec<K> = Vec::new();
        let mut j = from;
        loop {
            match code.get(j)? {
                Op::Push(c) => {
                    i32::try_from(*c).ok()?;
                    st.push(K::Imm);
                    ops.push(Op::Push(*c));
                }
                Op::Load(s) if *s < f.params => {
                    if *s >= 4 {
                        return None;
                    }
                    st.push(K::Param);
                    ops.push(Op::Load(*s));
                }
                Op::Load(s) => {
                    let c = known[*s as usize];
                    i32::try_from(c).ok()?;
                    st.push(K::Imm);
                    ops.push(Op::Push(c));
                }
                Op::Un(u) => {
                    st.pop()?;
                    st.push(K::Done);
                    ops.push(Op::Un(*u));
                }
                Op::Sext(n) => {
                    st.pop()?;
                    st.push(K::Done);
                    ops.push(Op::Sext(*n));
                }
                Op::Bin(b) => {
                    use BinOp::*;
                    let r = st.pop()?;
                    st.pop()?;
                    let ok = match b {
                        Add | Sub | Mul | Band | Bor | Bxor | Eq | Ne | Lt | Le | Gt | Ge => r != K::Done,
                        Shl | Shr => r == K::Imm,
                        _ => false,
                    };
                    if !ok {
                        return None;
                    }
                    st.push(K::Done);
                    ops.push(Op::Bin(*b));
                }
                _ => return Some((j, st.len())),
            }
            j += 1;
        }
    };
    let mut test = Vec::new();
    let (j, depth) = plain(i, &mut test)?;
    let (rest_on_zero, rest) = match code.get(j)? {
        Op::Jz(l) => (true, *l),
        Op::Jnz(l) => (false, *l),
        _ => return None,
    };
    if depth != 1 || test.is_empty() {
        return None;
    }
    let mut value = Vec::new();
    let (k, depth) = plain(j + 1, &mut value)?;
    if depth != 1 || code.get(k) != Some(&Op::Ret) {
        return None;
    }
    if let [Op::Push(_)] = test.as_slice() {
        return None; // a constant test: one way or the other always
    }
    Some(Fast { test, rest_on_zero, value, test_at: i, rest })
}

/// For every operation, whether its value goes to a float operation (so it
/// is better loaded into an xmm register than into rax). Followed on the
/// IR's stack between labels and jumps; across them, nothing is known.
fn float_operands(code: &[Op]) -> Vec<bool> {
    let mut out = vec![false; code.len()];
    let mut st: Vec<Option<usize>> = Vec::new();
    for (i, op) in code.iter().enumerate() {
        let (ins, outs) = match op {
            Op::Push(_) | Op::Load(_) | Op::LoadG(_) | Op::Frame(_) | Op::AddrG(_) | Op::ReadLine | Op::Data(_) => (0, 1),
            Op::Store(_) | Op::StoreG(_) | Op::Pop | Op::PrintInt | Op::Jz(_) | Op::Jnz(_) => (1, 0),
            Op::Bin(_) => (2, 1),
            Op::Un(_) | Op::Sext(_) | Op::LoadM(_) => (1, 1),
            Op::StoreM(_) | Op::OpM(..) | Op::PrintMem => (2, 0),
            Op::Call(_, n) => (*n as usize, 1),
            Op::ReadToken => (0, 2),
            Op::PrintStr(_) => (0, 0),
            Op::Dup | Op::Label(_) | Op::Jmp(_) | Op::Ret | Op::Fail(..) | Op::FailMem(_) => {
                st.clear();
                if matches!(op, Op::Dup) {
                    st.push(None);
                    st.push(None);
                }
                continue;
            }
        };
        let float = matches!(op, Op::Bin(b) if b.is_float()) || matches!(op, Op::Un(u) if u.is_float());
        for _ in 0..ins {
            if let Some(Some(p)) = st.pop() {
                out[p] = float;
            }
        }
        for _ in 0..outs {
            st.push(Some(i));
        }
    }
    out
}

pub fn compile(f: &Func, rules: &Rules, release: bool) -> Code {
    if std::env::var_os("STRIKE_DUMP_IR").is_some() {
        eprintln!("{} ({} params, {} slots):", f.name, f.params, f.slots);
        for (i, op) in f.code.iter().enumerate() {
            eprintln!("  {i:4} {op:?}");
        }
    }
    let merged = coalesce(f);
    let f = &merged;
    let labels = super::ir::label_count(&f.code);
    let at_depth = stack_depths(&f.code);
    let live = live_after(&f.code, f.slots);
    let vol_ok = volatile_ok(&f.code, f.slots, &live, &at_depth);
    let place = choose(f, rules, &vol_ok);
    // the call area at the bottom of the frame: shadow space and the stack
    // arguments of the widest call, so a call with nothing pushed needs no
    // rsp change
    let widest = f
        .code
        .iter()
        .map(|o| match o {
            Op::Call(_, n) => 32 + 8 * n.saturating_sub(4) as i32,
            Op::PrintInt | Op::PrintStr(_) | Op::PrintMem => 32,
            _ => 0,
        })
        .max()
        .unwrap_or(0);
    let mut loc = Vec::new();
    let mut saved = Vec::new();
    let mut mem = 0i32;
    let mut free = SAVED.iter();
    let mut xfree = XSAVED.iter();
    let mut xused = Vec::new();
    for s in 0..f.slots as usize {
        match place[s] {
            Place::Gpr => {
                let r = *free.next().expect("the beam keeps to the registers there are");
                loc.push(Loc::Reg(r));
                saved.push(r);
            }
            Place::Xmm => {
                let k = *xfree.next().expect("the beam keeps to the xmm registers there are");
                loc.push(Loc::Xmm(k));
                xused.push(k);
            }
            Place::Vol(k) => loc.push(Loc::Reg(VOL[k as usize])),
            Place::Mem => {
                loc.push(Loc::Mem(widest + 8 * mem));
                mem += 1;
            }
        }
    }
    // rsp is 8 past a multiple of 16 at the entry: the pushes and the frame
    // bring it back to one
    let pushed = saved.len() as i32;
    // the function's own memory above its locals, and above it the 16
    // bytes of each xmm register it saves
    let memory = widest + 8 * mem;
    let xbase = memory + (f.frame as i32 + 7) / 8 * 8;
    let xsaved: Vec<(u8, i32)> = xused.iter().enumerate().map(|(i, &k)| (k, xbase + 16 * i as i32)).collect();
    let mut frame = xbase + 16 * xsaved.len() as i32;
    if (8 + 8 * pushed + frame) % 16 != 0 {
        frame += 8;
    }
    let mut g = G { a: Asm::new(), v: Vec::new(), loc, saved, xsaved, frame, extra: 0, magic: release && rules.release.magic_division, magic_at: if release && rules.release.magic_division { magic_where(f) } else { Vec::new() }, op_at: 0, trap_at: None, div32: rules.division.bypass_32, memory, live };
    let fast = fast_path(f);
    if let Some(fp) = &fast {
        // before the frame: the parameters are still in rcx, rdx, r8, r9
        let at_entry = (0..f.slots).map(|s| if s < f.params.min(4) { Loc::Reg(ARGS[s as usize]) } else { Loc::Mem(0) }).collect();
        let real = std::mem::replace(&mut g.loc, at_entry);
        let rest = g.a.label();
        g.fast_ops(&fp.test);
        g.branch(fp.rest_on_zero, rest);
        g.v.clear();
        g.fast_ops(&fp.value);
        let x = g.pop();
        g.to_reg(x, RAX);
        g.a.e(&[0xC3]); // ret: no frame was made
        g.a.bind(rest);
        g.v.clear();
        g.loc = real;
    }
    for r in g.saved.clone() {
        g.push_r(r);
    }
    g.a.grow_stack(frame);
    // the callee-saved xmm registers used, all 128 bits
    for (k, d) in g.xsaved.clone() {
        g.sse_rsp(None, 0x11, k, d); // movups [rsp + d], xmm
    }
    // the parameters in rcx, rdx, r8, r9 go where they live; a register
    // parameter may live in another's argument register, so none is written
    // before every parameter it holds has been read (a cycle through r11)
    let mut moves: Vec<(u8, u8)> = Vec::new();
    for i in 0..f.params.min(4) {
        let from = ARGS[i as usize];
        match g.loc[i as usize] {
            Loc::Reg(r) => moves.push((r, from)),
            Loc::Mem(d) => g.rm(&[0x89], from, d),
            Loc::Xmm(k) => g.movq_to_xmm(k, from),
        }
    }
    moves.retain(|(d, s)| d != s);
    while !moves.is_empty() {
        if let Some(k) = moves.iter().position(|(d, _)| !moves.iter().any(|(_, s)| s == d)) {
            let (d, s) = moves.remove(k);
            g.mov_rr(d, s);
        } else {
            let s = moves[0].1;
            g.mov_rr(R11, s);
            for m in moves.iter_mut() {
                if m.1 == s {
                    m.1 = R11;
                }
            }
        }
    }
    for i in 0..f.params {
        let l = g.loc[i as usize];
        if i < 4 {
            continue;
        } else {
            // above the frame, the pushes, the return address and the
            // caller's shadow space
            let from = frame + 8 * pushed + 40 + 8 * (i as i32 - 4);
            match l {
                Loc::Reg(r) => g.rm(&[0x8B], r, from),
                Loc::Mem(d) => {
                    g.rm(&[0x8B], RAX, from);
                    g.rm(&[0x89], RAX, d);
                }
                Loc::Xmm(k) => g.sse_rsp(Some(0xF2), 0x10, k, from),
            }
        }
    }
    // which slots are known to hold zero: a store of zero into one of them
    // is left out
    let mut zero = vec![false; f.slots as usize];
    for s in f.params..f.slots {
        match g.loc[s as usize] {
            Loc::Reg(r) => g.mov_imm(r, 0),
            Loc::Mem(d) => {
                g.rm(&[0xC7], 0, d);
                g.a.d32(0);
            }
            Loc::Xmm(k) => g.sse_rr(None, 0x57, k, k), // xorps
        }
        zero[s as usize] = true;
    }

    let labels_of: Vec<u32> = (0..labels).map(|_| g.a.label()).collect();
    // what each label expects: how much is pushed, and whether the top is
    // in rax
    let mut state: Vec<Option<(i32, bool)>> = vec![None; labels as usize];
    // the labels a later jump goes back to: loops' tops
    let mut loop_top = vec![false; labels as usize];
    {
        let mut seen = vec![false; labels as usize];
        for op in &f.code {
            match op {
                Op::Label(l) => seen[*l as usize] = true,
                Op::Jmp(l) | Op::Jz(l) | Op::Jnz(l) if seen[*l as usize] => loop_top[*l as usize] = true,
                _ => {}
            }
        }
    }
    let mut live = true;
    if let Some(fp) = &fast {
        // the fast path's test failed: the constants stored before it, and
        // straight to the rest, not testing again
        for pair in f.code[..fp.test_at].windows(2) {
            if let [Op::Push(c), Op::Store(s)] = pair {
                if !(*c == 0 && zero[*s as usize]) {
                    g.push(V::Imm(*c));
                    g.store(*s);
                }
                zero[*s as usize] = *c == 0;
            }
        }
        g.flush();
        state[fp.rest as usize] = Some((g.phys(), false));
        g.a.jmp(labels_of[fp.rest as usize]);
        live = false;
    }
    let for_float = float_operands(&f.code);
    let mut new_label = |a: &mut Asm| a.label();
    let mut skip = 0;
    for (at, op) in f.code.iter().enumerate() {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        if !live && !matches!(op, Op::Label(_)) {
            continue; // no way reaches here
        }
        g.op_at = at;
        // a local in a register this operation writes holds zero no more
        // (it is not live here, so it is stored before it is read again)
        if clobbers(op) != 0 {
            for (s, z) in zero.iter_mut().enumerate() {
                if matches!(g.loc[s], Loc::Reg(r) if VOL.contains(&r)) {
                    *z = false;
                }
            }
        }
        match (op, f.code.get(at + 1)) {
            (Op::Push(0), Some(Op::Store(s))) if zero[*s as usize] => {
                skip = 1;
                continue;
            }
            // an element that is float arithmetic's right side: its memory
            // operand
            (Op::LoadM(8), Some(Op::Bin(b))) if b.is_float() => {
                g.op_at = at + 1;
                // stored at once into a float local in an xmm register
                if let Some(&Op::Store(s)) = f.code.get(at + 2) {
                    if matches!(g.loc[s as usize], Loc::Xmm(_)) && g.load_into_op(*b, Some(s)) {
                        zero[s as usize] = false;
                        skip = 2;
                        continue;
                    }
                }
                if g.load_into_op(*b, None) {
                    skip = 1;
                    continue;
                }
                g.op_at = at;
            }
            (Op::Bin(b), Some(Op::Store(s))) => {
                if g.bin_into(*b, *s) {
                    zero[*s as usize] = false;
                    skip = 1;
                    continue;
                }
            }
            // a narrow result (a C int): worked out in the local's register
            // and cut there
            (Op::Bin(b), Some(Op::Sext(n))) => {
                if let Some(Op::Store(s)) = f.code.get(at + 2) {
                    if g.bin_into(*b, *s) {
                        let d = g.slot_reg(*s);
                        super::x64::sext_reg(&mut g.a, d, *n);
                        zero[*s as usize] = false;
                        skip = 2;
                        continue;
                    }
                }
            }
            // an element only tested (`if (!comp[p])`, `while (s[i])`): one
            // cmp of the memory with zero, its flags for the jump
            (Op::LoadM(w), Some(next @ (Op::Un(UnOp::Not) | Op::Jz(_) | Op::Jnz(_)))) => {
                let x = g.pop();
                let addr = g.address(x);
                match w {
                    1 => g.mem(false, false, &[0x80], 7, addr),
                    2 => g.mem(true, false, &[0x83], 7, addr),
                    4 => g.mem(false, false, &[0x83], 7, addr),
                    _ => g.mem(false, true, &[0x83], 7, addr),
                }
                g.a.e(&[0]);
                if matches!(next, Op::Un(_)) {
                    g.v.push(V::Flags(0x94)); // zero: true
                    skip = 1;
                } else {
                    g.v.push(V::Flags(0x95)); // not zero: true
                }
                continue;
            }
            _ => {}
        }
        match op {
            Op::Push(c) => g.push(V::Imm(*c)),
            Op::Load(s) => g.push(V::Slot(*s)),
            Op::Store(s) => {
                let to_zero = g.v.last() == Some(&V::Imm(0));
                g.store(*s);
                zero[*s as usize] = to_zero;
            }
            Op::Bin(b) => g.bin(*b, &mut new_label),
            Op::Un(u) => g.un(*u),
            Op::Sext(n) => g.sext(*n),
            Op::LoadG(gl) => {
                g.fresh_rax();
                g.a.e(&[0x48, 0x8B, 0x05]); // mov rax, [global]
                g.a.reloc(super::x64::global(*gl));
                g.v.push(V::Rax);
            }
            Op::StoreG(gl) => {
                let x = g.pop();
                let r = if x == V::Rax { RAX } else { g.to_reg(x, R11); R11 };
                g.a.e(&[rex(r, 0), 0x89, 0x05 | (r & 7) << 3]); // mov [global], r
                g.a.reloc(super::x64::global(*gl));
            }
            Op::ReadToken => {
                g.flush();
                g.call_runtime("rt.read_token");
                g.push_r(RAX);
                g.v.push(V::Stack);
                g.mov_rr(RAX, 2);
                g.v.push(V::Rax);
            }
            Op::ReadLine => {
                g.flush();
                g.call_runtime("rt.read_line");
                g.v.push(V::Rax);
            }
            Op::LoadM(8) if for_float[at] => g.load_float(),
            // an element stored at once into a float local in an xmm register:
            // loaded straight into it
            Op::LoadM(8) if matches!(f.code.get(at + 1), Some(Op::Store(s)) if matches!(g.loc[*s as usize], Loc::Xmm(_))) => {
                let Some(&Op::Store(s)) = f.code.get(at + 1) else { unreachable!() };
                let Loc::Xmm(k) = g.loc[s as usize] else { unreachable!() };
                let x = g.pop();
                if let Some(i) = g.reads(s) {
                    g.flush_to(i);
                }
                g.keep_xmm(k);
                let addr = g.address(x);
                g.a.e(&[0xF2]);
                g.mem(false, false, &[0x0F, 0x10], k, addr); // movsd xmm, [address]
                zero[s as usize] = false;
                skip = 1;
            }
            Op::LoadM(w) => g.load_mem(*w),
            Op::StoreM(w) => g.store_mem(*w, None),
            Op::OpM(b, w) => g.store_mem(*w, Some(*b)),
            // an address the code knows: it waits, and is used where it is
            Op::Frame(n) => g.push(V::At(B::F(g.memory + *n as i32), 0)),
            Op::AddrG(gl) => g.push(V::At(B::G(super::x64::GLOBALS + 8 * *gl), 0)),
            Op::Data(d) => {
                g.fresh_rax();
                g.a.e(&[0x48, 0x8D, 0x05]); // lea rax, [rip + data]
                g.a.reloc(Target::Data(d.clone()));
                g.v.push(V::Rax);
            }
            Op::Fail(code, m) => {
                super::x64::fail_here(&mut g.a, *code, m);
                live = false;
            }
            Op::FailMem(code) => {
                let len = g.pop();
                let at = g.pop();
                g.need_rax();
                g.to_reg(len, 2); // rdx
                g.to_reg(at, RCX);
                super::x64::fail_mem(&mut g.a, *code);
                live = false;
            }
            Op::Dup => match g.v.last().copied() {
                Some(x @ (V::Imm(_) | V::Slot(_) | V::Lea(..) | V::Scaled(..) | V::Index(..) | V::At(..) | V::AtIdx(..))) => g.push(x),
                _ => {
                    g.flush();
                    g.a.e(&[0xFF, 0x34, 0x24]);
                    g.v.push(V::Stack);
                }
            },
            Op::Pop => {
                if g.pop() == V::Stack {
                    g.a.e(&[0x48, 0x83, 0xC4, 0x08]);
                }
            }
            Op::Call(sym, n) => g.call_direct(sym, *n as usize),
            Op::PrintInt => {
                let x = g.pop();
                g.need_rax();
                g.to_reg(x, RCX);
                g.flush();
                g.call_runtime("rt.print_int");
            }
            Op::PrintMem => {
                let len = g.pop();
                let at = g.pop();
                g.need_rax();
                g.to_reg(len, 2); // rdx
                g.to_reg(at, RCX);
                g.flush();
                g.call_runtime("rt.print_str");
            }
            Op::PrintStr(s) => {
                g.flush();
                g.a.e(&[0x48, 0x8D, 0x0D]);
                g.a.reloc(Target::Data(s.clone()));
                g.a.e(&[0x48, 0xC7, 0xC2]);
                g.a.d32(s.len() as i32);
                g.call_runtime("rt.print_str");
            }
            Op::Jmp(l) => {
                let carry = state[*l as usize].map_or(!g.v.is_empty(), |s| s.1);
                g.settle(carry);
                state[*l as usize].get_or_insert((g.phys(), carry));
                // a jump to the very next operation is no jump
                if f.code.get(at + 1) != Some(&Op::Label(*l)) {
                    g.a.jmp(labels_of[*l as usize]);
                }
                live = false;
            }
            Op::Jz(l) | Op::Jnz(l) => {
                let jz = matches!(op, Op::Jz(_));
                let x = g.pop();
                let target = labels_of[*l as usize];
                let carry = state[*l as usize].map_or(!g.v.is_empty(), |s| s.1);
                match x {
                    V::Flags(cc) => {
                        g.settle(carry); // moves and pushes leave the flags alone
                        state[*l as usize].get_or_insert((g.phys(), carry));
                        g.a.jcc(if jz { (cc ^ 1) & 0x0F } else { cc & 0x0F }, target);
                    }
                    V::Imm(c) => {
                        g.settle(carry);
                        state[*l as usize].get_or_insert((g.phys(), carry));
                        if (c == 0) == jz {
                            g.a.jmp(target);
                            live = false;
                        }
                    }
                    x => {
                        let r = match x {
                            V::Slot(s) if matches!(g.loc[s as usize], Loc::Reg(_)) => g.slot_reg(s),
                            x => {
                                g.need_rax();
                                g.to_reg(x, RAX);
                                RAX
                            }
                        };
                        g.rr(&[0x85], r, r); // test r, r
                        g.settle(carry);
                        state[*l as usize].get_or_insert((g.phys(), carry));
                        g.a.jcc(if jz { 4 } else { 5 }, target);
                    }
                }
            }
            Op::Label(l) => {
                let l = *l as usize;
                if live {
                    let carry = state[l].map_or(!g.v.is_empty(), |s| s.1);
                    g.settle(carry);
                    state[l].get_or_insert((g.phys(), carry));
                }
                if state[l].is_none() {
                    if let Some(d) = at_depth[at] {
                        // reached, but only by jumps from later on: they
                        // will deliver what is decided now
                        state[l] = Some(if d > 0 { (d - 1, true) } else { (0, false) });
                    }
                }
                match state[l] {
                    Some((phys, carry)) => {
                        g.v = vec![V::Stack; phys as usize];
                        if carry {
                            g.v.push(V::Rax);
                        }
                        live = true;
                    }
                    None => {} // nothing jumps here and nothing falls in
                }
                zero.iter_mut().for_each(|z| *z = false);
                // a loop's top starts a 16-byte line, as clang places it:
                // the loop is fetched in fewer pieces (a function starts at 16)
                if loop_top[l] && live {
                    let pad = (16 - g.a.b.len() % 16) % 16;
                    const NOPS: [&[u8]; 9] = [&[], &[0x90], &[0x66, 0x90], &[0x0F, 0x1F, 0x00], &[0x0F, 0x1F, 0x40, 0x00], &[0x0F, 0x1F, 0x44, 0x00, 0x00], &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00], &[0x0F, 0x1F, 0x80, 0, 0, 0, 0], &[0x0F, 0x1F, 0x84, 0, 0, 0, 0, 0]];
                    let mut left = pad;
                    while left > 0 {
                        let k = left.min(8);
                        g.a.e(NOPS[k]);
                        left -= k;
                    }
                }
                g.a.bind(labels_of[l]);
            }
            Op::Ret => {
                let x = g.pop();
                g.to_reg(x, RAX);
                g.epilogue();
                live = false;
            }
        }
    }
    if let Some(t) = g.trap_at {
        super::x64::trap(&mut g.a, t);
    }
    g.a.finish()
}