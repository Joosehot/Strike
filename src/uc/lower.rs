//! The universal syntax to Strike IR. Values are 64-bit integers; a truth
//! value is an integer too, but where the language prints it as a word
//! (Python's True, Go's true) the lowering follows which values are truth
//! values, through variables, parameters and results, until nothing changes.
//! Whatever Strike cannot yet do the way the language does is an error,
//! never a guess.

use super::ir::{BinOp, Func, Op, UnOp};
use super::lang::{Lang, Style};
use super::parse::{Expr, FuncDef, Program, Stmt};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    Int,
    Bool,
}

#[derive(Clone, PartialEq, Debug)]
struct Sig {
    params: Vec<Ty>,
    ret: Ty,
}

pub struct Lowered {
    pub funcs: Vec<Func>,
    pub entry: String,
    /// how many program-wide variables there are
    pub globals: u32,
}

type E = (u32, String);

const TOP: &str = "top.";
/// where a C or Go program's globals get their first values, then main runs
const INIT: &str = "init.";
/// the bit that marks a slot number as a global's
const GLOBAL: u32 = 1 << 31;

fn symbol(name: &str) -> String {
    format!("u.{name}")
}

/// Every name a body assigns, in its blocks too.
fn assigned_names(body: &[Stmt]) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    fn walk(body: &[Stmt], out: &mut std::collections::HashSet<String>) {
        for s in body {
            match s {
                Stmt::Var { name, .. } | Stmt::Assign { name, .. } | Stmt::ForRange { name, .. } | Stmt::Read { name, .. } => {
                    out.insert(name.clone());
                }
                _ => {}
            }
            match s {
                Stmt::If { then, els, .. } => {
                    walk(then, out);
                    walk(els, out);
                }
                Stmt::While { body, .. } | Stmt::ForRange { body, .. } => walk(body, out),
                Stmt::For { init, step, body, .. } => {
                    walk(std::slice::from_ref(init), out);
                    walk(std::slice::from_ref(step), out);
                    walk(body, out);
                }
                _ => {}
            }
        }
    }
    walk(body, &mut out);
    out
}

fn always_returns(body: &[Stmt]) -> bool {
    fn breaks(body: &[Stmt]) -> bool {
        body.iter().any(|s| match s {
            Stmt::Break(_) => true,
            Stmt::If { then, els, .. } => breaks(then) || breaks(els),
            _ => false,
        })
    }
    body.iter().any(|s| match s {
        Stmt::Return(..) => true,
        Stmt::If { then, els, .. } => always_returns(then) && always_returns(els),
        Stmt::While { cond, body } => {
            matches!(cond, None | Some(Expr::Num(1.., _))) && !breaks(body)
        }
        _ => false,
    })
}

fn constant(e: &Expr) -> Option<i64> {
    match e {
        Expr::Num(v, _) => Some(*v),
        Expr::Un(UnOp::Neg, x) => constant(x).map(|v| -v),
        _ => None,
    }
}

/// The bits of a type the language writes (64 where it writes none, or
/// where its numbers have no width of their own).
fn width_of(l: &Lang, words: &[String]) -> Result<u8, String> {
    if l.int_widths.is_empty() || words.is_empty() {
        return Ok(64);
    }
    let w: Vec<&str> = words.iter().map(String::as_str).filter(|s| !matches!(*s, "static" | "const" | "signed" | "register" | "volatile")).collect();
    if w.iter().any(|s| *s == "unsigned" || s.starts_with("uint") || *s == "byte") {
        return Err("unsigned numbers are not in Strike yet: they wrap and compare differently, so they are not guessed at".into());
    }
    if w.iter().any(|s| *s == "*" || *s == "[" || *s == "]") {
        return Err("pointers and arrays are not in Strike yet".into());
    }
    let key = if w.len() > 1 && w.last() == Some(&"int") { w[..w.len() - 1].join(" ") } else { w.join(" ") };
    if key == "void" || key.is_empty() {
        return Ok(64);
    }
    l.int_widths.get(&key).copied().ok_or(format!("the type {key} is not in Strike yet"))
}

struct Shared<'a> {
    l: &'a Lang,
    bools: bool,
    sigs: &'a HashMap<String, Sig>,
    /// each function's parameter widths and result width
    fw: &'a HashMap<String, (Vec<u8>, u8)>,
    /// callee -> the types seen at each parameter
    args: HashMap<String, Vec<Vec<Ty>>>,
    /// calls whose result is used: (callee, line)
    used: Vec<(String, u32)>,
    /// the program's variables: name -> (slot with GLOBAL set, kind)
    globals: HashMap<String, (u32, Ty)>,
    gwidths: HashMap<u32, u8>,
    nglobals: u32,
}

struct Fx<'s, 'a> {
    sh: &'s mut Shared<'a>,
    code: Vec<Op>,
    scopes: Vec<HashMap<String, (u32, Ty)>>,
    slots: u32,
    labels: u32,
    loops: Vec<(u32, u32)>,
    rets: Vec<(Ty, u32)>,
    top: bool,
    line: u32,
    /// the bits of each slot's type (64 where not known narrower)
    widths: HashMap<u32, u8>,
    /// the width of the value expr just left: the bits it is known to fit,
    /// sign-extended (0: a literal that takes the width of what it meets)
    last_w: u8,
    /// the width of what this function returns
    ret_w: u8,
    /// this is the code at the top level: what it declares is the program's
    globals_here: bool,
    /// Python's `global` names in this function
    global_names: std::collections::HashSet<String>,
    /// every name this function assigns (Python: they are its own)
    assigned: std::collections::HashSet<String>,
}

impl Fx<'_, '_> {
    fn emit(&mut self, op: Op) {
        if let (Op::PrintStr(new), Some(Op::PrintStr(last))) = (&op, self.code.last_mut()) {
            last.extend_from_slice(new);
            return;
        }
        self.code.push(op);
    }
    fn label(&mut self) -> u32 {
        self.labels += 1;
        self.labels - 1
    }
    fn err<X>(&self, m: impl Into<String>) -> Result<X, E> {
        Err((self.line, m.into()))
    }
    fn bool_ty(&self) -> Ty {
        if self.sh.bools {
            Ty::Bool
        } else {
            Ty::Int
        }
    }
    /// A name as the language finds it: the function's own first, then the
    /// program's. In Python a name the function assigns anywhere is its own
    /// even before that assignment (unless it is declared `global`).
    fn lookup(&self, name: &str) -> Option<(u32, Ty)> {
        if !self.global_names.contains(name) {
            if let Some(x) = self.scopes.iter().rev().find_map(|s| s.get(name).copied()) {
                return Some(x);
            }
            if !self.sh.l.block_scope && !self.globals_here && self.assigned.contains(name) {
                return None;
            }
        }
        self.sh.globals.get(name).copied()
    }
    /// A new variable: the program's where the code at the top level makes
    /// it (or Python's `global` names it), else the function's own.
    fn declare(&mut self, name: &str, ty: Ty) -> u32 {
        if self.globals_here || self.global_names.contains(name) {
            let g = GLOBAL | self.sh.nglobals;
            self.sh.nglobals += 1;
            self.sh.globals.insert(name.to_string(), (g, ty));
            return g;
        }
        let slot = self.slots;
        self.slots += 1;
        self.scopes.last_mut().unwrap().insert(name.to_string(), (slot, ty));
        slot
    }
    fn load(&mut self, slot: u32) {
        self.emit(if slot & GLOBAL != 0 { Op::LoadG(slot & !GLOBAL) } else { Op::Load(slot) });
    }
    fn store(&mut self, slot: u32) {
        self.emit(if slot & GLOBAL != 0 { Op::StoreG(slot & !GLOBAL) } else { Op::Store(slot) });
    }
    fn width(&self, slot: u32) -> u8 {
        let w = if slot & GLOBAL != 0 { self.sh.gwidths.get(&slot) } else { self.widths.get(&slot) };
        w.copied().unwrap_or(64)
    }
    fn set_width(&mut self, slot: u32, w: u8) {
        if slot & GLOBAL != 0 {
            self.sh.gwidths.insert(slot, w);
        } else {
            self.widths.insert(slot, w);
        }
    }
    /// The error for a name that cannot be found, saying why.
    fn unknown<X>(&self, name: &str) -> Result<X, E> {
        if !self.sh.l.block_scope && self.assigned.contains(name) && self.sh.globals.contains_key(name) {
            return self.err(format!(
                "{name} is read before this function assigns it; Python makes it the function's own because it is assigned here (say `global {name}` to mean the module's)"
            ));
        }
        self.err(format!("{name} is not declared"))
    }
    fn typed(&self) -> bool {
        !self.sh.l.int_widths.is_empty()
    }
    /// A value of width `from` going where `to` bits are kept: cut to them
    /// and sign-extended back, as C and Go convert.
    fn narrow(&mut self, to: u8, from: u8) -> Result<(), E> {
        if !self.typed() || to >= 64 || (from != 0 && from <= to) {
            return Ok(());
        }
        if from == 0 && self.sh.l.untyped_literals {
            // a constant that does not fit is an error in Go, not a wrap
            if let Some(Op::Push(c)) = self.code.last() {
                let (lo, hi) = (-(1i64 << (to - 1)), (1i64 << (to - 1)) - 1);
                if *c < lo || *c > hi {
                    return self.err(format!("{c} does not fit {to} bits"));
                }
                return Ok(());
            }
        }
        self.emit(Op::Sext(to));
        Ok(())
    }
    /// The width arithmetic on two values is done at.
    fn arith_width(&self, a: u8, b: u8) -> u8 {
        if !self.typed() {
            return 64;
        }
        let w = match (a, b) {
            (0, x) | (x, 0) => x,
            (x, y) => x.max(y),
        };
        if w == 0 {
            0
        } else {
            w.max(self.sh.l.promote)
        }
    }
    fn hidden(&mut self) -> u32 {
        self.slots += 1;
        self.slots - 1
    }
    fn push_scope(&mut self) {
        if self.sh.l.block_scope {
            self.scopes.push(HashMap::new());
        }
    }
    fn pop_scope(&mut self) {
        if self.sh.l.block_scope {
            self.scopes.pop();
        }
    }
    fn same(&self, name: &str, was: Ty, now: Ty) -> Result<(), E> {
        if self.sh.bools && was != now {
            return self.err(format!("{name} holds a number in one place and a truth value in another; Strike keeps one kind per variable"));
        }
        Ok(())
    }

    fn block(&mut self, body: &[Stmt]) -> Result<(), E> {
        self.push_scope();
        for s in body {
            self.stmt(s)?;
        }
        self.pop_scope();
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt) -> Result<(), E> {
        match s {
            Stmt::Var { name, ty: words, value, line } => {
                self.line = *line;
                let dw = width_of(self.sh.l, words).map_err(|m| (*line, m))?;
                let ty = match value {
                    Some(e) => {
                        let t = self.expr(e, true)?;
                        let from = self.last_w;
                        // Go's `x := 5` is an int: the literal's own width
                        self.narrow(dw, from)?;
                        t
                    }
                    None => {
                        self.emit(Op::Push(0));
                        Ty::Int
                    }
                };
                let twice = if self.globals_here { self.sh.globals.contains_key(name) } else { self.scopes.last().unwrap().contains_key(name) };
                if self.sh.l.block_scope && twice {
                    return self.err(format!("{name} is declared twice in one block"));
                }
                let slot = match (self.sh.l.block_scope, self.lookup(name)) {
                    (false, Some((slot, was))) => {
                        self.same(name, was, ty)?;
                        slot
                    }
                    _ => {
                        let s = self.declare(name, ty);
                        self.set_width(s, dw);
                        s
                    }
                };
                self.store(slot);
            }
            Stmt::Assign { name, op, value, line } => {
                self.line = *line;
                match op {
                    Some(op) => {
                        let Some((slot, was)) = self.lookup(name) else {
                            return self.unknown(name);
                        };
                        if was == Ty::Bool {
                            return self.err(format!("{name} holds a truth value; Strike does not do arithmetic into it"));
                        }
                        self.load(slot);
                        self.expr(value, true)?;
                        let op = if self.sh.l.checked { op.checked() } else { *op };
                        self.emit(Op::Bin(op));
                        // done at the promoted width, then put back in the variable's
                        let w = self.width(slot);
                        self.narrow(w, 64)?;
                        self.store(slot);
                    }
                    None => {
                        let ty = self.expr(value, true)?;
                        let from = self.last_w;
                        let slot = match self.lookup(name) {
                            Some((slot, was)) => {
                                self.same(name, was, ty)?;
                                slot
                            }
                            None if !self.sh.l.block_scope => self.declare(name, ty),
                            None => return self.unknown(name),
                        };
                        let w = self.width(slot);
                        self.narrow(w, from)?;
                        self.store(slot);
                    }
                }
            }
            Stmt::If { cond, then, els } => {
                self.expr(cond, true)?;
                let (other, end) = (self.label(), self.label());
                self.emit(Op::Jz(other));
                self.block(then)?;
                if els.is_empty() {
                    self.emit(Op::Label(other));
                } else {
                    self.emit(Op::Jmp(end));
                    self.emit(Op::Label(other));
                    self.block(els)?;
                    self.emit(Op::Label(end));
                }
            }
            Stmt::While { cond, body } => {
                let (top, end) = (self.label(), self.label());
                self.emit(Op::Label(top));
                if let Some(c) = cond {
                    self.expr(c, true)?;
                    self.emit(Op::Jz(end));
                }
                self.loops.push((top, end));
                self.block(body)?;
                self.loops.pop();
                self.emit(Op::Jmp(top));
                self.emit(Op::Label(end));
            }
            Stmt::For { init, cond, step, body } => {
                self.push_scope();
                self.stmt(init)?;
                let (top, cont, end) = (self.label(), self.label(), self.label());
                self.emit(Op::Label(top));
                self.expr(cond, true)?;
                self.emit(Op::Jz(end));
                self.loops.push((cont, end));
                self.block(body)?;
                self.loops.pop();
                self.emit(Op::Label(cont));
                self.stmt(step)?;
                self.emit(Op::Jmp(top));
                self.emit(Op::Label(end));
                self.pop_scope();
            }
            Stmt::ForRange { name, args, body, line } => {
                self.line = *line;
                let (start, stop, step) = match args.as_slice() {
                    [b] => (None, b, None),
                    [a, b] => (Some(a), b, None),
                    [a, b, c] => (Some(a), b, Some(c)),
                    _ => return self.err("range takes one to three numbers"),
                };
                let fixed = match step {
                    None => Some(1),
                    Some(c) => constant(c),
                };
                if fixed == Some(0) {
                    return self.err("range's step must not be zero");
                }
                let (counter, end, by) = (self.hidden(), self.hidden(), self.hidden());
                match start {
                    Some(a) => {
                        self.expr(a, true)?;
                    }
                    None => self.emit(Op::Push(0)),
                }
                self.store(counter);
                self.expr(stop, true)?;
                self.store(end);
                match step {
                    Some(c) => {
                        self.expr(c, true)?;
                    }
                    None => self.emit(Op::Push(1)),
                }
                self.store(by);
                let var = match self.lookup(name) {
                    Some((slot, was)) => {
                        self.same(name, was, Ty::Int)?;
                        slot
                    }
                    None => self.declare(name, Ty::Int),
                };
                let (top, cont, done) = (self.label(), self.label(), self.label());
                self.emit(Op::Label(top));
                match fixed {
                    Some(v) => {
                        self.load(counter);
                        self.load(end);
                        self.emit(Op::Bin(if v > 0 { BinOp::Lt } else { BinOp::Gt }));
                    }
                    None => {
                        // (step > 0 and i < end) or (step < 0 and i > end)
                        for (sign, cmp) in [(BinOp::Gt, BinOp::Lt), (BinOp::Lt, BinOp::Gt)] {
                            self.load(by);
                            self.emit(Op::Push(0));
                            self.emit(Op::Bin(sign));
                            self.load(counter);
                            self.load(end);
                            self.emit(Op::Bin(cmp));
                            self.emit(Op::Bin(BinOp::Band));
                        }
                        self.emit(Op::Bin(BinOp::Bor));
                    }
                }
                self.emit(Op::Jz(done));
                self.load(counter);
                self.store(var);
                self.loops.push((cont, done));
                self.block(body)?;
                self.loops.pop();
                self.emit(Op::Label(cont));
                self.load(counter);
                self.load(by);
                self.emit(Op::Bin(BinOp::Add));
                self.store(counter);
                self.emit(Op::Jmp(top));
                self.emit(Op::Label(done));
            }
            Stmt::Return(e, line) => {
                self.line = *line;
                if self.top {
                    return self.err("return outside a function");
                }
                match e {
                    Some(e) => {
                        let t = self.expr(e, true)?;
                        let (to, from) = (self.ret_w, self.last_w);
                        self.narrow(to, from)?;
                        self.rets.push((t, *line));
                    }
                    None => self.emit(Op::Push(0)),
                }
                self.emit(Op::Ret);
            }
            Stmt::Break(line) | Stmt::Continue(line) => {
                self.line = *line;
                let Some(&(cont, end)) = self.loops.last() else {
                    return self.err("break or continue outside a loop");
                };
                self.emit(Op::Jmp(if matches!(s, Stmt::Break(_)) { end } else { cont }));
            }
            Stmt::Print { style, args, line } => {
                self.line = *line;
                self.print(*style, args)?;
            }
            Stmt::Expr(e) => {
                self.expr(e, false)?;
                self.emit(Op::Pop);
            }
            Stmt::Global(name, line) => {
                self.line = *line;
                self.global_names.insert(name.clone());
            }
            Stmt::Read { name, fmt, line } => {
                self.line = *line;
                if let Some(f) = fmt {
                    if !matches!(f.as_slice(), b"%d" | b"%i" | b"%ld" | b"%li" | b"%lld" | b"%lli") {
                        return self.err("scanf reads one whole number (%d) in this stage of Strike");
                    }
                }
                let Some((slot, ty)) = self.lookup(name) else { return self.unknown(name) };
                if ty == Ty::Bool && self.sh.bools {
                    return self.err(format!("{name} holds a truth value; a number is read into it"));
                }
                // the number and whether there was one; with none the
                // variable keeps its value, as scanf and Scan leave it
                let (none, end) = (self.label(), self.label());
                self.emit(Op::ReadToken);
                self.emit(Op::Jz(none));
                let w = self.width(slot);
                self.narrow(w, 64)?;
                self.store(slot);
                self.emit(Op::Jmp(end));
                self.emit(Op::Label(none));
                self.emit(Op::Pop);
                self.emit(Op::Label(end));
            }
            Stmt::Nothing => {}
        }
        Ok(())
    }

    fn print_value(&mut self, e: &Expr) -> Result<(), E> {
        if let Expr::Str(s) = e {
            self.emit(Op::PrintStr(s.clone()));
            return Ok(());
        }
        let t = self.expr(e, true)?;
        match (&self.sh.l.bool_print, t) {
            (Some((yes, no)), Ty::Bool) => {
                let (yes, no) = (yes.clone().into_bytes(), no.clone().into_bytes());
                let (other, end) = (self.label(), self.label());
                self.emit(Op::Jz(other));
                self.emit(Op::PrintStr(yes));
                self.emit(Op::Jmp(end));
                self.emit(Op::Label(other));
                self.emit(Op::PrintStr(no));
                self.emit(Op::Label(end));
            }
            _ => self.emit(Op::PrintInt),
        }
        Ok(())
    }

    fn print(&mut self, style: Style, args: &[Expr]) -> Result<(), E> {
        match style {
            Style::SpacedLine | Style::None => {
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        self.emit(Op::PrintStr(b" ".to_vec()));
                    }
                    self.print_value(a)?;
                }
                self.emit(Op::PrintStr(b"\n".to_vec()));
            }
            Style::Line => match args {
                [Expr::Str(s)] => {
                    let mut s = s.clone();
                    s.push(b'\n');
                    self.emit(Op::PrintStr(s));
                }
                _ => return self.err("this print takes one string"),
            },
            Style::Format => {
                let Some((Expr::Str(f), rest)) = args.split_first() else {
                    return self.err("the first thing printed by a format is its string");
                };
                let mut rest = rest.iter();
                let mut lit = Vec::new();
                let mut i = 0;
                while i < f.len() {
                    if f[i] != b'%' {
                        lit.push(f[i]);
                        i += 1;
                        continue;
                    }
                    i += 1;
                    if f.get(i) == Some(&b'%') {
                        lit.push(b'%');
                        i += 1;
                        continue;
                    }
                    while f.get(i) == Some(&b'l') {
                        i += 1;
                    }
                    let spec = f.get(i).copied().unwrap_or(b'?');
                    i += 1;
                    if !lit.is_empty() {
                        self.emit(Op::PrintStr(std::mem::take(&mut lit)));
                    }
                    let Some(a) = rest.next() else {
                        return self.err("the format wants more values than it is given");
                    };
                    match spec {
                        b'd' | b'i' => {
                            if matches!(a, Expr::Str(_)) {
                                return self.err("%d is given a string");
                            }
                            let t = self.expr(a, true)?;
                            if t == Ty::Bool && self.sh.bools {
                                return self.err("%d is given a truth value");
                            }
                            self.emit(Op::PrintInt);
                        }
                        b'v' => self.print_value(a)?,
                        b's' => match a {
                            Expr::Str(s) => self.emit(Op::PrintStr(s.clone())),
                            _ => return self.err("%s is given something that is not a string"),
                        },
                        c => return self.err(format!("the format %{} is not in Strike yet", c as char)),
                    }
                }
                if !lit.is_empty() {
                    self.emit(Op::PrintStr(lit));
                }
                if rest.next().is_some() {
                    return self.err("the format is given more values than it prints");
                }
            }
        }
        Ok(())
    }

    /// Lowers an expression, leaving its value on the stack; `used` is false
    /// only for an expression statement.
    fn expr(&mut self, e: &Expr, used: bool) -> Result<Ty, E> {
        // every case leaves the width of its value in last_w; a truth
        // value fits 8 bits
        Ok(match e {
            Expr::Num(v, b) => {
                self.emit(Op::Push(*v));
                self.last_w = if *b || !self.typed() {
                    if *b { 8 } else { 64 }
                } else if self.sh.l.untyped_literals {
                    0
                } else if i32::try_from(*v).is_ok() {
                    32
                } else {
                    64
                };
                if *b {
                    self.bool_ty()
                } else {
                    Ty::Int
                }
            }
            Expr::Str(_) => return self.err("a string can only be printed in this stage of Strike"),
            Expr::Var(name, line) => {
                self.line = *line;
                let Some((slot, ty)) = self.lookup(name) else {
                    return self.unknown(name);
                };
                self.load(slot);
                self.last_w = self.width(slot);
                ty
            }
            Expr::Call(name, args, line) => {
                self.line = *line;
                // Python's int(input()): a line read as a whole number
                if let Some((outer, inner)) = &self.sh.l.read_line {
                    if !self.sh.sigs.contains_key(name) {
                        if let [Expr::Call(i, a, _)] = args.as_slice() {
                            if name == outer && i == inner && a.is_empty() {
                                self.emit(Op::ReadLine);
                                self.last_w = 64;
                                return Ok(Ty::Int);
                            }
                        }
                        if name == inner {
                            return self.err(format!("{inner}() gives a string, and strings are not in Strike yet; {outer}({inner}()) reads a whole number"));
                        }
                    }
                }
                let Some(sig) = self.sh.sigs.get(name).cloned() else {
                    return self.err(format!("there is no function {name}"));
                };
                if sig.params.len() != args.len() {
                    return self.err(format!("{name} takes {} values, given {}", sig.params.len(), args.len()));
                }
                let mut tys = Vec::new();
                let (pw, rw) = self.sh.fw.get(name).cloned().unwrap_or_default();
                for (i, a) in args.iter().enumerate() {
                    tys.push(self.expr(a, true)?);
                    // an argument is converted to its parameter's type
                    let from = self.last_w;
                    self.narrow(pw.get(i).copied().unwrap_or(64), from)?;
                }
                let seen = self.sh.args.entry(name.clone()).or_insert_with(|| vec![Vec::new(); args.len()]);
                for (i, t) in tys.into_iter().enumerate() {
                    seen[i].push(t);
                }
                if used {
                    self.sh.used.push((name.clone(), *line));
                }
                self.emit(Op::Call(symbol(name), args.len() as u32));
                self.last_w = if rw == 0 { 64 } else { rw };
                sig.ret
            }
            Expr::Un(op, x) => {
                self.expr(x, true)?;
                let w = self.arith_width(self.last_w, self.last_w);
                match op {
                    UnOp::Plus => {
                        self.last_w = w;
                        Ty::Int
                    }
                    UnOp::Not => {
                        self.emit(Op::Un(UnOp::Not));
                        self.last_w = 8;
                        self.bool_ty()
                    }
                    _ => {
                        let op = if self.sh.l.checked && *op == UnOp::Neg { UnOp::NegOv } else { *op };
                        self.emit(Op::Un(op));
                        // -INT_MIN is INT_MIN again at 32 bits
                        if op == UnOp::Neg && w > 0 {
                            self.narrow(w, 64)?;
                        }
                        self.last_w = w;
                        Ty::Int
                    }
                }
            }
            Expr::Bin(op, a, b) => match op {
                BinOp::AndBool | BinOp::OrBool => {
                    let (jump, short): (fn(u32) -> Op, i64) =
                        if *op == BinOp::AndBool { (Op::Jz, 0) } else { (Op::Jnz, 1) };
                    let (cut, end) = (self.label(), self.label());
                    self.expr(a, true)?;
                    self.emit(jump(cut));
                    self.expr(b, true)?;
                    self.emit(jump(cut));
                    self.emit(Op::Push(1 - short));
                    self.emit(Op::Jmp(end));
                    self.emit(Op::Label(cut));
                    self.emit(Op::Push(short));
                    self.emit(Op::Label(end));
                    self.last_w = 8;
                    self.bool_ty()
                }
                BinOp::AndValue | BinOp::OrValue => {
                    let end = self.label();
                    let ta = self.expr(a, true)?;
                    self.emit(Op::Dup);
                    self.emit(if *op == BinOp::AndValue { Op::Jz(end) } else { Op::Jnz(end) });
                    self.emit(Op::Pop);
                    let tb = self.expr(b, true)?;
                    self.emit(Op::Label(end));
                    if ta != tb && self.sh.bools {
                        return self.err("and/or between a number and a truth value is not in Strike yet");
                    }
                    self.last_w = 64;
                    ta
                }
                _ => {
                    self.expr(a, true)?;
                    let wa = self.last_w;
                    self.expr(b, true)?;
                    let wb = self.last_w;
                    let op = if self.sh.l.checked { op.checked() } else { *op };
                    self.emit(Op::Bin(op));
                    if op.is_comparison() {
                        self.last_w = 8;
                        self.bool_ty()
                    } else {
                        // the operation at its width: a result that can leave
                        // it is cut back, as the machine of that width would
                        let w = self.arith_width(wa, wb);
                        if w > 0 && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Shl | BinOp::DivTrunc) {
                            self.narrow(w, 64)?;
                        }
                        self.last_w = if w == 0 { 0 } else { w };
                        Ty::Int
                    }
                }
            },
        })
    }
}

fn combine(what: &str, tys: &[Ty], bools: bool) -> Result<Ty, String> {
    let has = |t| tys.contains(&t);
    Ok(match (has(Ty::Int), has(Ty::Bool)) {
        (_, false) => Ty::Int,
        (false, true) => Ty::Bool,
        (true, true) if bools => return Err(format!("{what} is a number in one place and a truth value in another")),
        (true, true) => Ty::Int,
    })
}

pub fn lower(p: &Program, l: &Lang) -> Result<Lowered, E> {
    let mut table: HashMap<String, &FuncDef> = HashMap::new();
    for f in &p.funcs {
        if table.insert(f.name.clone(), f).is_some() {
            return Err((f.line, format!("the function {} is defined twice", f.name)));
        }
        let mut seen = std::collections::HashSet::new();
        if let Some(d) = f.params.iter().find(|n| !seen.insert(n.as_str())) {
            return Err((f.line, format!("{} has two parameters named {d}", f.name)));
        }
    }
    // a C or Go program with variables outside its functions: init. gives
    // them their first values, then runs main
    let has_init = !l.toplevel_entry && !p.top.is_empty();
    if has_init {
        if let Some(Stmt::Print { line, .. } | Stmt::Assign { line, .. } | Stmt::Return(_, line)) = p.top.iter().find(|s| !matches!(s, Stmt::Var { .. })) {
            return Err((*line, "only variables are declared outside a function".into()));
        }
    }
    let entry = if l.toplevel_entry {
        TOP.to_string()
    } else {
        match table.get("main") {
            Some(m) if m.params.is_empty() => if has_init { INIT.to_string() } else { symbol("main") },
            Some(m) => return Err((m.line, "main takes no parameters in this stage of Strike".into())),
            None => return Err((1, "there is no main function".into())),
        }
    };
    let mut fw: HashMap<String, (Vec<u8>, u8)> = HashMap::new();
    for f in &p.funcs {
        let ps = f.param_types.iter().map(|t| width_of(l, t)).collect::<Result<Vec<u8>, String>>().map_err(|m| (f.line, m))?;
        let ret = width_of(l, &f.ret).map_err(|m| (f.line, m))?;
        fw.insert(f.name.clone(), (ps, ret));
    }
    let bools = l.bool_print.is_some();
    let mut sigs: HashMap<String, Sig> =
        p.funcs.iter().map(|f| (f.name.clone(), Sig { params: vec![Ty::Int; f.params.len()], ret: Ty::Int })).collect();
    for _round in 0..10 {
        let mut sh = Shared { l, bools, sigs: &sigs, fw: &fw, args: HashMap::new(), used: Vec::new(), globals: HashMap::new(), gwidths: HashMap::new(), nglobals: 0 };
        let mut funcs = Vec::new();
        let mut rets: HashMap<String, Vec<(Ty, u32)>> = HashMap::new();
        // the top level first: the globals it declares are seen by every
        // function
        let mut bodies: Vec<(String, u32, Vec<(String, Ty)>, &[Stmt], bool)> = Vec::new();
        if l.toplevel_entry || has_init {
            bodies.push((String::new(), 1, Vec::new(), p.top.as_slice(), true));
        }
        bodies.extend(p.funcs.iter().map(|f| {
            let ps = f.params.iter().cloned().zip(sigs[&f.name].params.iter().copied()).collect();
            (f.name.clone(), f.line, ps, f.body.as_slice(), false)
        }));
        for (name, line, params, body, top) in bodies {
            let (pw, ret_w) = fw.get(&name).cloned().unwrap_or_default();
            let mut fx = Fx {
                sh: &mut sh,
                code: Vec::new(),
                scopes: vec![HashMap::new()],
                slots: 0,
                labels: 0,
                loops: Vec::new(),
                rets: Vec::new(),
                top,
                line,
                widths: HashMap::new(),
                last_w: 64,
                ret_w: if top { 64 } else { ret_w },
                globals_here: top,
                global_names: Default::default(),
                assigned: if top { Default::default() } else { assigned_names(body) },
            };
            for (i, (n, t)) in params.iter().enumerate() {
                let s = fx.declare(n, *t);
                fx.set_width(s, pw.get(i).copied().unwrap_or(64));
            }
            fx.push_scope();
            for s in body {
                fx.stmt(s)?;
            }
            fx.pop_scope();
            if top && has_init {
                fx.emit(Op::Call(symbol("main"), 0));
            } else {
                fx.emit(Op::Push(0));
            }
            fx.emit(Op::Ret);
            let f = Func {
                name: match (top, has_init) {
                    (true, true) => INIT.to_string(),
                    (true, false) => TOP.to_string(),
                    _ => symbol(&name),
                },
                params: params.len() as u32,
                slots: fx.slots,
                code: fx.code,
            };
            if !top {
                rets.insert(name, fx.rets);
            }
            funcs.push(f);
        }
        for (callee, line) in &sh.used {
            if !always_returns(&table[callee.as_str()].body) {
                return Err((*line, format!("the value of {callee} is used, but {callee} can end without returning one")));
            }
        }
        let globals = sh.nglobals;
        let mut next = sigs.clone();
        for (name, rs) in &rets {
            let tys: Vec<Ty> = rs.iter().map(|r| r.0).collect();
            let line = rs.first().map(|r| r.1).unwrap_or(1);
            next.get_mut(name).unwrap().ret = combine(&format!("what {name} returns"), &tys, bools).map_err(|m| (line, m))?;
        }
        for (callee, per) in &sh.args {
            let line = table[callee.as_str()].line;
            for (i, tys) in per.iter().enumerate() {
                let what = format!("parameter {} of {callee}", table[callee.as_str()].params[i]);
                next.get_mut(callee).unwrap().params[i] = combine(&what, tys, bools).map_err(|m| (line, m))?;
            }
        }
        if next == sigs {
            return Ok(Lowered { funcs, entry, globals });
        }
        sigs = next;
    }
    Err((1, "which values are truth values did not settle in ten rounds".into()))
}
