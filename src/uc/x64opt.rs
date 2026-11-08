//! Strike's optimizing code generator. The IR's stack is kept at compile
//! time: a constant, a local or a result in rax stays where it is until an
//! operation needs it, so `n - 1` is one `lea`-free `mov` and `sub`, and a
//! comparison followed by a jump is one `cmp` and one `jcc`. Only what must
//! be on the machine's stack (across a call, a jump or a label) is pushed.
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
}

#[derive(serde::Deserialize, Clone)]
pub struct InlineRules {
    pub call_cost: f64,
    pub size_cost: f64,
    pub max_ops: usize,
}

impl Rules {
    pub fn read() -> Result<Rules, String> {
        toml::from_str(RULES).map_err(|e| format!("codegen.toml: {e}"))
    }
    /// What the rules add to a brand: other numbers give other code.
    pub fn key(&self) -> String {
        format!(
            "x64opt-2 loop={} save={} beam={} inline={},{},{}",
            self.loop_weight, self.save_cost, self.beam, self.inline.call_cost, self.inline.size_cost, self.inline.max_ops
        )
    }
}

const RAX: u8 = 0;
const RCX: u8 = 1;
const R10: u8 = 10;
const R11: u8 = 11;
/// rbx, rsi, rdi, r12 to r15: kept across calls (Windows x64)
const SAVED: [u8; 7] = [3, 6, 7, 12, 13, 14, 15];
const ARGS: [u8; 4] = [1, 2, 8, 9];

#[derive(Clone, Copy, PartialEq, Debug)]
enum V {
    Imm(i64),
    Slot(u32),
    Rax,
    /// the flags of a comparison; the byte is its setcc opcode (0x94 = e)
    Flags(u8),
    /// pushed on the machine's stack
    Stack,
}

#[derive(Clone, Copy)]
enum Loc {
    Reg(u8),
    Mem(i32),
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
fn weights(f: &Func, r: &Rules) -> Vec<f64> {
    let at = super::ir::loop_depths(&f.code);
    let mut w = vec![0.0; f.slots as usize];
    for (i, op) in f.code.iter().enumerate() {
        if let Op::Load(s) | Op::Store(s) = op {
            w[*s as usize] += r.loop_weight.powi(at[i].min(6) as i32);
        }
    }
    for p in 0..f.params.min(4) {
        w[p as usize] += 1.0; // a register parameter is already in a register
    }
    w
}

/// HotMotor: the locals in order of weight, each a register or memory; the
/// beam keeps the `beam` best partial choices, scored by the memory
/// operations saved less the cost of saving the registers used.
fn choose(f: &Func, r: &Rules) -> Vec<bool> {
    let w = weights(f, r);
    let mut order: Vec<usize> = (0..w.len()).collect();
    order.sort_by(|&a, &b| w[b].total_cmp(&w[a]).then(a.cmp(&b)));
    let mut beam: Vec<(f64, Vec<bool>)> = vec![(0.0, vec![false; w.len()])];
    for &s in &order {
        let mut next = Vec::new();
        for (score, pick) in &beam {
            next.push((*score, pick.clone()));
            if pick.iter().filter(|&&p| p).count() < SAVED.len() {
                let mut p = pick.clone();
                p[s] = true;
                next.push((score + w[s] - r.save_cost, p));
            }
        }
        next.sort_by(|a, b| b.0.total_cmp(&a.0));
        next.truncate(r.beam.max(1));
        beam = next;
    }
    beam.swap_remove(0).1
}

struct G {
    a: Asm,
    v: Vec<V>,
    loc: Vec<Loc>,
    saved: Vec<(u8, i32)>,
}

impl G {
    fn rr(&mut self, op: &[u8], reg: u8, rm: u8) {
        self.a.e(&[rex(reg, rm)]);
        self.a.e(op);
        self.a.e(&[0xC0 | (reg & 7) << 3 | (rm & 7)]);
    }
    /// op with [rbp + d]
    fn rm(&mut self, op: &[u8], reg: u8, d: i32) {
        self.a.e(&[rex(reg, 0)]);
        self.a.e(op);
        self.a.e(&[0x80 | (reg & 7) << 3 | 5]);
        self.a.d32(d);
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
            },
            V::Rax => self.mov_rr(r, RAX),
            V::Stack => self.pop_r(r),
            V::Flags(cc) => self.setcc_into(r, cc),
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
                    self.a.e(&[0xFF, 0xB5]);
                    self.a.d32(d);
                }
            },
            V::Rax => self.push_r(RAX),
            V::Flags(cc) => {
                self.setcc_into(R11, cc);
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
        if let Some(i) = self.v.iter().rposition(|x| *x == V::Rax) {
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
        let right = self.pop();
        let left = self.pop();
        self.need_rax();
        let o = self.operand(right);
        if b.is_comparison() {
            let dst = match left {
                V::Slot(s) => match self.loc[s as usize] {
                    Loc::Reg(y) => y,
                    Loc::Mem(_) => {
                        self.to_reg(left, RAX);
                        RAX
                    }
                },
                _ => {
                    self.to_reg(left, RAX);
                    RAX
                }
            };
            self.alu(7, dst, o);
            let cc = match b {
                BinOp::Eq => 0x94,
                BinOp::Ne => 0x95,
                BinOp::Lt => 0x9C,
                BinOp::Le => 0x9E,
                BinOp::Gt => 0x9F,
                _ => 0x9D,
            };
            self.v.push(V::Flags(cc));
            return;
        }
        self.to_reg(left, RAX);
        match b {
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
            BinOp::DivTrunc | BinOp::ModTrunc | BinOp::DivFloor | BinOp::ModFloor => {
                self.into_rcx(o);
                self.a.e(&[0x48, 0x99, 0x48, 0xF7, 0xF9]); // cqo; idiv rcx
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

    fn store(&mut self, s: u32) {
        let x = self.pop();
        if let Some(i) = self.v.iter().rposition(|y| *y == V::Slot(s)) {
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
                    self.a.e(&[0x8F, 0x85]); // pop qword [rbp + d]
                    self.a.d32(d);
                }
                V::Slot(t) if t == s => {}
                V::Slot(t) => match self.loc[t as usize] {
                    Loc::Reg(y) => self.rm(&[0x89], y, d),
                    Loc::Mem(_) => {
                        self.to_reg(x, R11);
                        self.rm(&[0x89], R11, d);
                    }
                },
                V::Rax => self.rm(&[0x89], RAX, d),
                other => {
                    self.to_reg(other, R11);
                    self.rm(&[0x89], R11, d);
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
        if let Some(i) = self.v.iter().rposition(|x| *x == V::Rax) {
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
        let mut s = 32 + 8 * k;
        if (physical + s / 8) % 2 == 1 {
            s += 8;
        }
        self.a.sub_rsp(s as i32);
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
        self.a.call(sym);
        self.a.add_rsp(s as i32);
        self.v.push(V::Rax);
    }

    fn epilogue(&mut self) {
        for (r, d) in self.saved.clone() {
            self.rm(&[0x8B], r, d);
        }
        self.a.epilogue();
    }
}

pub fn compile(f: &Func, rules: &Rules) -> Code {
    let in_reg = choose(f, rules);
    let mut loc = Vec::new();
    let mut saved = Vec::new();
    let mut mem = 0i32;
    let mut free = SAVED.iter();
    for s in 0..f.slots as usize {
        if in_reg[s] {
            let r = *free.next().expect("the beam keeps to the registers there are");
            loc.push(Loc::Reg(r));
            saved.push(r);
        } else {
            mem += 1;
            loc.push(Loc::Mem(-8 * mem));
        }
    }
    let saved: Vec<(u8, i32)> = saved
        .into_iter()
        .map(|r| {
            mem += 1;
            (r, -8 * mem)
        })
        .collect();
    let frame = (mem * 8 + 15) / 16 * 16;
    let mut g = G { a: Asm::new(), v: Vec::new(), loc, saved };
    g.a.prologue(frame);
    for (r, d) in g.saved.clone() {
        g.rm(&[0x89], r, d);
    }
    for i in 0..f.params {
        let l = g.loc[i as usize];
        if i < 4 {
            match l {
                Loc::Reg(r) => g.mov_rr(r, ARGS[i as usize]),
                Loc::Mem(d) => g.rm(&[0x89], ARGS[i as usize], d),
            }
        } else {
            let from = 48 + 8 * (i as i32 - 4);
            match l {
                Loc::Reg(r) => g.rm(&[0x8B], r, from),
                Loc::Mem(d) => {
                    g.rm(&[0x8B], RAX, from);
                    g.rm(&[0x89], RAX, d);
                }
            }
        }
    }
    for s in f.params..f.slots {
        match g.loc[s as usize] {
            Loc::Reg(r) => g.mov_imm(r, 0),
            Loc::Mem(d) => {
                g.rm(&[0xC7], 0, d);
                g.a.d32(0);
            }
        }
    }

    let labels = f.code.iter().filter_map(|o| if let Op::Label(l) = o { Some(*l + 1) } else { None }).max().unwrap_or(0);
    let labels_of: Vec<u32> = (0..labels).map(|_| g.a.label()).collect();
    let mut depth_at: Vec<Option<u32>> = vec![None; labels as usize];
    let mut new_label = |a: &mut Asm| a.label();
    for op in &f.code {
        match op {
            Op::Push(c) => g.push(V::Imm(*c)),
            Op::Load(s) => g.push(V::Slot(*s)),
            Op::Store(s) => g.store(*s),
            Op::Bin(b) => g.bin(*b, &mut new_label),
            Op::Un(u) => {
                let x = g.pop();
                match (u, x) {
                    (UnOp::Plus, x) => g.v.push(x),
                    (UnOp::Not, V::Flags(cc)) => g.v.push(V::Flags(cc ^ 1)),
                    (UnOp::Not, x) => {
                        g.need_rax();
                        g.to_reg(x, RAX);
                        g.a.e(&[0x48, 0x85, 0xC0]);
                        g.v.push(V::Flags(0x94));
                    }
                    (u, x) => {
                        g.need_rax();
                        g.to_reg(x, RAX);
                        g.a.e(if *u == UnOp::Neg { &[0x48, 0xF7, 0xD8] } else { &[0x48, 0xF7, 0xD0] });
                        g.v.push(V::Rax);
                    }
                }
            }
            Op::Dup => match g.v.last().copied() {
                Some(x @ (V::Imm(_) | V::Slot(_))) => g.push(x),
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
                let mut d = g.depth();
                call(&mut g.a, "rt.print_int", 0, &mut d, false);
            }
            Op::PrintStr(s) => {
                g.flush();
                g.a.e(&[0x48, 0x8D, 0x0D]);
                g.a.reloc(Target::Data(s.clone()));
                g.a.e(&[0x48, 0xC7, 0xC2]);
                g.a.d32(s.len() as i32);
                let mut d = g.depth();
                call(&mut g.a, "rt.print_str", 0, &mut d, false);
            }
            Op::Jmp(l) => {
                g.flush();
                depth_at[*l as usize].get_or_insert(g.depth());
                g.a.jmp(labels_of[*l as usize]);
            }
            Op::Jz(l) | Op::Jnz(l) => {
                let jz = matches!(op, Op::Jz(_));
                let x = g.pop();
                let target = labels_of[*l as usize];
                match x {
                    V::Flags(cc) => {
                        g.flush();
                        let cc = if jz { cc ^ 1 } else { cc };
                        depth_at[*l as usize].get_or_insert(g.depth());
                        g.a.jcc(cc & 0x0F, target);
                    }
                    V::Imm(c) => {
                        g.flush();
                        depth_at[*l as usize].get_or_insert(g.depth());
                        if (c == 0) == jz {
                            g.a.jmp(target);
                        }
                    }
                    x => {
                        let r = match x {
                            V::Slot(s) => match g.loc[s as usize] {
                                Loc::Reg(y) => y,
                                Loc::Mem(_) => {
                                    g.need_rax();
                                    g.to_reg(x, RAX);
                                    RAX
                                }
                            },
                            _ => {
                                g.need_rax();
                                g.to_reg(x, RAX);
                                RAX
                            }
                        };
                        g.rr(&[0x85], r, r); // test r, r
                        g.flush();
                        depth_at[*l as usize].get_or_insert(g.depth());
                        g.a.jcc(if jz { 4 } else { 5 }, target);
                    }
                }
            }
            Op::Label(l) => {
                g.flush();
                match depth_at[*l as usize] {
                    Some(d) => g.v = vec![V::Stack; d as usize],
                    None => depth_at[*l as usize] = Some(g.depth()),
                }
                g.a.bind(labels_of[*l as usize]);
            }
            Op::Ret => {
                let x = g.pop();
                g.to_reg(x, RAX);
                g.epilogue();
                g.v = vec![V::Stack; g.v.len()];
            }
        }
    }
    g.a.finish()
}
