//! The universal syntax to Strike IR. Values are 64-bit integers; a truth
//! value is an integer too, but where the language prints it as a word
//! (Python's True, Go's true) the lowering follows which values are truth
//! values, through variables, parameters and results, until nothing changes.
//! A list, a slice or an array is the address of its memory (lib.rs says
//! how each is laid out), and its kind is followed the same way; so is a
//! string (strs.rs).
//! Whatever Strike cannot yet do the way the language does is an error,
//! never a guess.

use super::ir::{BinOp, Func, Op, UnOp};
use super::lang::{Lang, Strings, Style};
use super::lib::{Elem, Printer, MAX_LEN};
use super::parse::{Expr, FuncDef, Program, Stmt};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub enum Ty {
    Int,
    Bool,
    /// n elements in a row, each of the given width when it is a number (a
    /// C or Go array; a nested array is its rows in a row): the value is
    /// the address of the first
    Arr(Box<Ty>, u8, u64),
    /// the address of elements (a C array parameter)
    Ptr(Box<Ty>, u8),
    /// Go: the address of a header (data, length, capacity) never changed
    Slice(Box<Ty>, u8),
    /// Python: the address of a header (length, capacity, data)
    List(Box<Ty>),
    /// the elements of a Python list before any is seen
    Unknown,
    /// a Python or Go string: the address of its length and its bytes
    Str,
}

impl Ty {
    /// The bytes the value takes where it is kept: an array its elements,
    /// anything else 8.
    pub fn bytes(&self) -> u64 {
        match self {
            Ty::Arr(e, w, n) => n * size(e, *w),
            _ => 8,
        }
    }
    /// The 8-byte words that hold it (an array's rounded up).
    fn words(&self) -> u64 {
        self.bytes().div_ceil(8)
    }
    fn elem(&self) -> Option<(Ty, u8)> {
        match self {
            Ty::Arr(e, w, _) | Ty::Ptr(e, w) | Ty::Slice(e, w) => Some(((**e).clone(), *w)),
            Ty::List(e) => Some(((**e).clone(), 64)),
            _ => None,
        }
    }
    fn scalar(&self) -> bool {
        matches!(self, Ty::Int | Ty::Bool | Ty::Unknown)
    }
    /// Is the zero of this kind not all zero bits? (A Go slice's zero is
    /// the address of the empty header.)
    fn zero_is_not_zero(&self) -> bool {
        match self {
            Ty::Slice(..) | Ty::Str => true,
            Ty::Arr(e, _, _) => e.zero_is_not_zero(),
            _ => false,
        }
    }
    fn describe(&self) -> String {
        match self {
            Ty::Int => "a number".into(),
            Ty::Bool => "a truth value".into(),
            Ty::Arr(..) => "an array".into(),
            Ty::Ptr(..) => "a pointer".into(),
            Ty::Slice(..) => "a slice".into(),
            Ty::List(..) => "a list".into(),
            Ty::Unknown => "an element not yet known".into(),
            Ty::Str => "a string".into(),
        }
    }
}

/// The width of Go's byte (uint8): its values are 0 to 255, so a value
/// going there is cut by masking, not by sign-extending.
pub const U8: u8 = 0x88;

/// The bits a width takes.
fn bits(w: u8) -> u8 {
    if w == U8 {
        8
    } else {
        w
    }
}

/// The bytes an element of kind e and width w takes: a number its own
/// width (a C char 1, an int 4, a Go int32 4, a bool 1), anything else as
/// it is kept.
fn size(e: &Ty, w: u8) -> u64 {
    if e.scalar() {
        (bits(w) as u64 / 8).max(1)
    } else {
        e.bytes()
    }
}

/// The operation that reads an element of kind e: none for an array held
/// in place (its address is its value).
fn load_of(e: &Ty, w: u8) -> Option<Op> {
    match e {
        Ty::Arr(..) => None,
        _ => Some(Op::LoadM(size(e, w) as u8)),
    }
}

/// Two kinds made one: an unknown element takes the other's, a list's
/// elements are made one in turn. None where they are different kinds.
pub fn unify(a: &Ty, b: &Ty) -> Option<Ty> {
    match (a, b) {
        (Ty::Unknown, x) | (x, Ty::Unknown) => Some(x.clone()),
        (Ty::List(x), Ty::List(y)) => Some(Ty::List(Box::new(unify(x, y)?))),
        (x, y) if x == y => Some(x.clone()),
        _ => None,
    }
}

/// A C array given where its elements' address is wanted (an array
/// parameter): the same elements.
fn decays(from: &Ty, to: &Ty) -> bool {
    match (from, to) {
        (Ty::Arr(e, w, _), Ty::Ptr(f, v)) => e == f && w == v,
        (Ty::Ptr(e, w), Ty::Ptr(f, v)) => e == f && w == v,
        _ => false,
    }
}

#[derive(Clone, PartialEq, Debug)]
struct Sig {
    params: Vec<Ty>,
    ret: Ty,
}

#[derive(Clone)]
pub struct Lowered {
    pub funcs: Vec<Func>,
    pub entry: String,
    /// how many program-wide words there are (variables, and the words of
    /// global arrays)
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

/// Every name a body assigns whole, in its blocks too (an element's
/// assignment does not make a name the function's own).
pub(super) fn assigned_names(body: &[Stmt]) -> HashSet<String> {
    let mut out = HashSet::new();
    fn walk(body: &[Stmt], out: &mut HashSet<String>) {
        for s in body {
            match s {
                Stmt::Var { name, .. } | Stmt::ForRange { name, .. } | Stmt::Read { name, .. } => {
                    out.insert(name.clone());
                }
                Stmt::Assign { name, index, .. } if index.is_empty() => {
                    out.insert(name.clone());
                }
                Stmt::ForEach { index, value, .. } => {
                    out.extend(index.iter().cloned());
                    out.extend(value.iter().cloned());
                }
                _ => {}
            }
            match s {
                Stmt::If { then, els, .. } => {
                    walk(then, out);
                    walk(els, out);
                }
                Stmt::While { body, .. } | Stmt::ForRange { body, .. } | Stmt::ForEach { body, .. } => walk(body, out),
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

/// The names a body slices (Go's a[i:j]): an array sliced must outlive
/// the call, so it is not kept in the function's own memory.
fn sliced_names(body: &[Stmt]) -> HashSet<String> {
    fn expr(e: &Expr, out: &mut HashSet<String>) {
        match e {
            Expr::Slice(b, lo, hi, _) => {
                if let Expr::Var(n, _) = &**b {
                    out.insert(n.clone());
                }
                expr(b, out);
                lo.iter().chain(hi.iter()).for_each(|x| expr(x, out));
            }
            Expr::Call(_, a, _) | Expr::List(a, _) | Expr::Brace(a, _) | Expr::Composite(_, a, _) => a.iter().for_each(|x| expr(x, out)),
            Expr::Method(b, _, a, _) => {
                expr(b, out);
                a.iter().for_each(|x| expr(x, out));
            }
            Expr::Un(_, x) => expr(x, out),
            Expr::Bin(_, a, b) | Expr::Index(a, b, _) => {
                expr(a, out);
                expr(b, out);
            }
            Expr::Comp { elem, iter, cond, .. } => {
                expr(elem, out);
                expr(iter, out);
                cond.iter().for_each(|x| expr(x, out));
            }
            Expr::Num(..) | Expr::Str(_) | Expr::Var(..) | Expr::Type(..) => {}
        }
    }
    fn walk(body: &[Stmt], out: &mut HashSet<String>) {
        for s in body {
            match s {
                Stmt::Var { value, dims, .. } => {
                    value.iter().for_each(|x| expr(x, out));
                    dims.iter().flatten().for_each(|x| expr(x, out));
                }
                Stmt::Assign { index, value, .. } => {
                    index.iter().for_each(|x| expr(x, out));
                    expr(value, out);
                }
                Stmt::If { cond, then, els } => {
                    expr(cond, out);
                    walk(then, out);
                    walk(els, out);
                }
                Stmt::While { cond, body } => {
                    cond.iter().for_each(|x| expr(x, out));
                    walk(body, out);
                }
                Stmt::For { init, cond, step, body } => {
                    walk(std::slice::from_ref(init), out);
                    expr(cond, out);
                    walk(std::slice::from_ref(step), out);
                    walk(body, out);
                }
                Stmt::ForRange { args, body, .. } => {
                    args.iter().for_each(|x| expr(x, out));
                    walk(body, out);
                }
                Stmt::ForEach { iter, body, .. } => {
                    expr(iter, out);
                    walk(body, out);
                }
                Stmt::Return(e, _) => e.iter().for_each(|x| expr(x, out)),
                Stmt::Print { args, .. } => args.iter().for_each(|x| expr(x, out)),
                Stmt::Expr(e) => expr(e, out),
                _ => {}
            }
        }
    }
    let mut out = HashSet::new();
    walk(body, &mut out);
    out
}

/// Is the expression free of calls (nothing it does can change memory)?
fn calls_nothing(e: &Expr) -> bool {
    match e {
        Expr::Num(..) | Expr::Str(_) | Expr::Var(..) | Expr::Type(..) => true,
        Expr::Un(_, x) => calls_nothing(x),
        Expr::Bin(_, a, b) | Expr::Index(a, b, _) => calls_nothing(a) && calls_nothing(b),
        _ => false,
    }
}

/// Does the body assign to name or an element of it?
fn changes(body: &[Stmt], name: &str) -> bool {
    body.iter().any(|s| match s {
        Stmt::Assign { name: n, .. } | Stmt::Var { name: n, .. } | Stmt::Read { name: n, .. } if n == name => true,
        Stmt::If { then, els, .. } => changes(then, name) || changes(els, name),
        Stmt::While { body, .. } | Stmt::ForRange { body, .. } | Stmt::ForEach { body, .. } => changes(body, name),
        Stmt::For { init, step, body, .. } => changes(std::slice::from_ref(init), name) || changes(std::slice::from_ref(step), name) || changes(body, name),
        _ => false,
    })
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

/// The value of a constant expression (an array's size).
fn constant(e: &Expr) -> Option<i64> {
    match e {
        Expr::Num(v, _) => Some(*v),
        Expr::Un(UnOp::Neg, x) => constant(x).map(|v| v.wrapping_neg()),
        Expr::Un(UnOp::Plus, x) => constant(x),
        Expr::Bin(op, a, b) => {
            let (a, b) = (constant(a)?, constant(b)?);
            match op {
                BinOp::Add => a.checked_add(b),
                BinOp::Sub => a.checked_sub(b),
                BinOp::Mul => a.checked_mul(b),
                BinOp::DivTrunc if b != 0 => a.checked_div(b),
                BinOp::ModTrunc if b != 0 => a.checked_rem(b),
                BinOp::Shl if (0..63).contains(&b) => a.checked_shl(b as u32),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The bits of a number type the language writes (64 where it writes
/// none, or where its numbers have no width of their own).
fn width_of(l: &Lang, words: &[String]) -> Result<u8, String> {
    if l.int_widths.is_empty() || words.is_empty() {
        return Ok(64);
    }
    let w: Vec<&str> = words.iter().map(String::as_str).filter(|s| !matches!(*s, "static" | "const" | "signed" | "register" | "volatile")).collect();
    if matches!(w.as_slice(), ["byte"] | ["uint8"]) {
        return Ok(U8);
    }
    if w.iter().any(|s| *s == "unsigned" || s.starts_with("uint") || *s == "byte") {
        return Err("unsigned numbers are not in Strike yet: they wrap and compare differently, so they are not guessed at".into());
    }
    if w.iter().any(|s| *s == "*" || *s == "[" || *s == "]") {
        return Err("pointers are not in Strike yet (but arrays are)".into());
    }
    let key = if w.len() > 1 && w.last() == Some(&"int") { w[..w.len() - 1].join(" ") } else { w.join(" ") };
    if key == "void" || key.is_empty() {
        return Ok(64);
    }
    l.int_widths.get(&key).copied().ok_or(format!("the type {key} is not in Strike yet"))
}

/// A type as the language writes it: Go's []T and [n]T before the element,
/// C's * and [n] after it (a parameter's `int a[]`, `int g[][10]`, `int *p`:
/// the address of elements, as C makes an array parameter). The kind and,
/// for a number, its width.
fn type_of(l: &Lang, words: &[String], bools: bool) -> Result<(Ty, u8), String> {
    if words.first().map(String::as_str) == Some("[") {
        // Go: the brackets, outermost first, then the element
        let close = words.iter().position(|w| w == "]").ok_or("a [ without its ]")?;
        let (inner, w) = type_of(l, &words[close + 1..], bools)?;
        if w == U8 {
            return Err("a slice or an array of bytes is not in Strike yet (but a string is)".into());
        }
        let w = if inner.scalar() { w } else { 64 };
        return Ok(match close {
            1 => (Ty::Slice(Box::new(inner), w), 64),
            2 => {
                let n: u64 = words[1].parse().map_err(|_| format!("{} is not an array's size", words[1]))?;
                (Ty::Arr(Box::new(inner), w, n), 64)
            }
            _ => return Err("this array type is not in Strike yet".into()),
        });
    }
    let base_end = words.iter().position(|w| w == "[" || w == "*").unwrap_or(words.len());
    let base = &words[..base_end];
    if base.len() == 1 && base[0] == "string" && matches!(l.strings, Strings::Bytes | Strings::Ascii) {
        if base_end != words.len() {
            return Err("pointers are not in Strike yet (but arrays are)".into());
        }
        return Ok((Ty::Str, 64));
    }
    let w = width_of(l, base)?;
    let is_bool = base.iter().any(|b| b == "bool" || b == "_Bool");
    let scalar = if is_bool && bools { Ty::Bool } else { Ty::Int };
    let mut rest = &words[base_end..];
    let mut stars = 0;
    while rest.first().map(String::as_str) == Some("*") {
        stars += 1;
        rest = &rest[1..];
    }
    let mut dims: Vec<Option<u64>> = Vec::new();
    while rest.first().map(String::as_str) == Some("[") {
        match (rest.get(1).map(String::as_str), rest.get(2).map(String::as_str)) {
            (Some("]"), _) => {
                dims.push(None);
                rest = &rest[2..];
            }
            (Some(n), Some("]")) => {
                dims.push(Some(n.parse().map_err(|_| format!("{n} is not an array's size"))?));
                rest = &rest[3..];
            }
            _ => return Err("this array type is not in Strike yet".into()),
        }
    }
    if !rest.is_empty() || stars > 1 || (stars > 0 && !dims.is_empty()) {
        return Err("pointers are not in Strike yet (but arrays are)".into());
    }
    if stars == 1 {
        return Ok((Ty::Ptr(Box::new(scalar), w), 64));
    }
    if dims.is_empty() {
        return Ok((scalar, w));
    }
    // the sizes after the first, innermost last: the rows the pointer is to
    let mut inner = scalar;
    let mut ew = w;
    for d in dims[1..].iter().rev() {
        let n = d.ok_or("only an array's first size may be left out")?;
        inner = Ty::Arr(Box::new(inner), ew, n);
        ew = 64;
    }
    Ok((Ty::Ptr(Box::new(inner), ew), 64))
}

/// Go's element size in bytes and whether it holds no pointers (for the
/// capacity its slices grow to).
fn go_size(t: &Ty, w: u8) -> (i64, bool) {
    match t {
        Ty::Int | Ty::Unknown => ((w / 8).max(1) as i64, true),
        Ty::Bool => (1, true),
        // a Go string is two words (its bytes' address and its length)
        Ty::Str => (16, false),
        Ty::Arr(e, ew, n) => {
            let (s, ns) = go_size(e, *ew);
            (s * *n as i64, ns)
        }
        _ => (24, false),
    }
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
    /// the library functions used: the printers, and the others by name
    printers: BTreeMap<String, Printer>,
    basic: HashSet<String>,
    /// what each variable's kind came to last round: (function, name)
    known: &'a HashMap<(String, String), Ty>,
    seen: HashMap<(String, String), Ty>,
    /// the global words of Go's empty slice header
    nil_slice: Option<u32>,
}

struct Fx<'s, 'a> {
    sh: &'s mut Shared<'a>,
    /// the function's own name (for the kinds kept between rounds)
    fname: String,
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
    /// the width of what this function returns, and its kind where the
    /// language declares one that is not a number
    ret_w: u8,
    ret_ty: Option<Ty>,
    /// this is the code at the top level: what it declares is the program's
    globals_here: bool,
    /// Python's `global` names in this function
    global_names: HashSet<String>,
    /// every name this function assigns (Python: they are its own)
    assigned: HashSet<String>,
    /// the bytes of the function's own memory (C's arrays, Go's arrays not sliced)
    frame: u32,
    sliced: HashSet<String>,
}

impl Fx<'_, '_> {
    fn emit(&mut self, op: Op) {
        if let (Op::PrintStr(new), Some(Op::PrintStr(last))) = (&op, self.code.last_mut()) {
            last.extend_from_slice(new);
            return;
        }
        self.code.push(op);
    }
    fn ops(&mut self, ops: &[Op]) {
        for o in ops {
            self.emit(o.clone());
        }
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
    fn lib(&mut self, name: &str, n: u32) {
        self.sh.basic.insert(name.to_string());
        self.emit(Op::Call(name.to_string(), n));
    }
    /// A name as the language finds it: the function's own first, then the
    /// program's. In Python a name the function assigns anywhere is its own
    /// even before that assignment (unless it is declared `global`).
    fn lookup(&self, name: &str) -> Option<(u32, Ty)> {
        if !self.global_names.contains(name) {
            if let Some(x) = self.scopes.iter().rev().find_map(|s| s.get(name).cloned()) {
                return Some(x);
            }
            if !self.sh.l.block_scope && !self.globals_here && self.assigned.contains(name) {
                return None;
            }
        }
        self.sh.globals.get(name).cloned()
    }
    /// The key a variable's kind is kept under between rounds.
    fn key(&self, name: &str, slot: u32) -> (String, String) {
        if slot & GLOBAL != 0 {
            (String::new(), name.to_string())
        } else {
            (self.fname.clone(), name.to_string())
        }
    }
    /// A new variable: the program's where the code at the top level makes
    /// it (or Python's `global` names it), else the function's own; `words`
    /// global words for it (an array's).
    fn declare_words(&mut self, name: &str, ty: Ty, words: u32) -> u32 {
        if self.globals_here || self.global_names.contains(name) {
            let g = GLOBAL | self.sh.nglobals;
            self.sh.nglobals += words;
            let ty = self.known_kind(name, g, ty);
            self.sh.globals.insert(name.to_string(), (g, ty));
            return g;
        }
        let slot = self.slots;
        self.slots += 1;
        let ty = self.known_kind(name, slot, ty);
        self.scopes.last_mut().unwrap().insert(name.to_string(), (slot, ty));
        slot
    }
    fn declare(&mut self, name: &str, ty: Ty) -> u32 {
        self.declare_words(name, ty, 1)
    }
    /// The kind a variable is known to come to (an empty list's elements,
    /// from what is appended later or in another function).
    fn known_kind(&self, name: &str, slot: u32, ty: Ty) -> Ty {
        if ty.scalar() && ty != Ty::Unknown {
            return ty;
        }
        match self.sh.known.get(&self.key(name, slot)) {
            Some(k) => unify(&ty, k).unwrap_or(ty),
            None => ty,
        }
    }
    /// Records what a variable has been seen to be, for the next round.
    fn saw(&mut self, k: (String, String), t: Ty) {
        let merged = match self.sh.seen.get(&k) {
            Some(old) => unify(old, &t).unwrap_or(t),
            None => t,
        };
        self.sh.seen.insert(k, merged);
    }
    /// What a variable is now known to be: its kind made one with `ty`.
    fn refine(&mut self, name: &str, ty: &Ty) -> Result<(), E> {
        let Some((slot, was)) = self.lookup(name) else { return Ok(()) };
        let Some(now) = unify(&was, ty) else {
            return self.err(format!("{name} holds {} in one place and {} in another; Strike keeps one kind per variable", was.describe(), ty.describe()));
        };
        if now != was {
            if slot & GLOBAL != 0 {
                self.sh.globals.insert(name.to_string(), (slot, now.clone()));
            } else if let Some(s) = self.scopes.iter_mut().rev().find(|s| s.contains_key(name)) {
                s.insert(name.to_string(), (slot, now.clone()));
            }
        }
        if !now.scalar() {
            let k = self.key(name, slot);
            self.saw(k, now);
        }
        Ok(())
    }
    fn load(&mut self, slot: u32) {
        self.emit(if slot & GLOBAL != 0 { Op::LoadG(slot & !GLOBAL) } else { Op::Load(slot) });
    }
    fn store(&mut self, slot: u32) {
        self.emit(if slot & GLOBAL != 0 { Op::StoreG(slot & !GLOBAL) } else { Op::Store(slot) });
    }
    /// A variable's value: an array that is a global is its address.
    fn load_var(&mut self, slot: u32, ty: &Ty) {
        if slot & GLOBAL != 0 && matches!(ty, Ty::Arr(..)) {
            self.emit(Op::AddrG(slot & !GLOBAL));
        } else {
            self.load(slot);
        }
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
    /// Does the language check its indexes (Go, Python), so its arrays are
    /// values (Go) and not C's?
    fn checks(&self) -> bool {
        self.sh.l.index_error.is_some()
    }
    /// A value of width `from` going where `to` bits are kept: cut to them
    /// and sign-extended back, as C and Go convert.
    fn narrow(&mut self, to: u8, from: u8) -> Result<(), E> {
        if to == U8 {
            // Go's byte: 0 to 255, a constant that is not one an error
            if !self.typed() || from == U8 {
                return Ok(());
            }
            if from == 0 && self.sh.l.untyped_literals {
                if let Some(Op::Push(c)) = self.code.last() {
                    if !(0..=255).contains(c) {
                        return self.err(format!("{c} does not fit a byte"));
                    }
                    return Ok(());
                }
            }
            self.ops(&[Op::Push(255), Op::Bin(BinOp::Band)]);
            return Ok(());
        }
        // a byte fits any signed width above 8
        let from = if from == U8 { 9 } else { from };
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
    fn same(&mut self, name: &str, was: &Ty, now: &Ty) -> Result<(), E> {
        if was.scalar() && now.scalar() {
            if self.sh.bools && was != now && *was != Ty::Unknown && *now != Ty::Unknown {
                return self.err(format!("{name} holds a number in one place and a truth value in another; Strike keeps one kind per variable"));
            }
            return Ok(());
        }
        self.refine(name, now)
    }
    /// Bytes of the function's own memory for `words` words: where they begin.
    fn frame_words(&mut self, words: u64) -> Result<u32, E> {
        let bytes = words.checked_mul(8).filter(|b| *b < 1 << 30).ok_or((self.line, "an array this large is not in Strike yet".to_string()))?;
        let at = self.frame;
        self.frame = self.frame.checked_add(bytes as u32).filter(|f| *f < 1 << 30).ok_or((self.line, "arrays this large are not in Strike yet".to_string()))?;
        Ok(at)
    }
    /// Stops with the language's index error unless the hidden slot `i`
    /// is below what the code `len` pushes (read unsigned: a negative index
    /// is out of range too).
    fn check(&mut self, i: u32, len: &[Op]) {
        let Some((code, msg)) = self.sh.l.index_error.clone() else { return };
        let ok = self.label();
        self.emit(Op::Load(i));
        self.ops(len);
        self.ops(&[Op::Bin(BinOp::Ult), Op::Jnz(ok), Op::Fail(code, msg.into_bytes()), Op::Label(ok)]);
    }
    fn fail_unless(&mut self, code: u8, msg: &str) {
        let ok = self.label();
        self.ops(&[Op::Jnz(ok), Op::Fail(code, msg.as_bytes().to_vec()), Op::Label(ok)]);
    }
    fn go_fail(&self) -> u8 {
        self.sh.l.index_error.as_ref().map_or(2, |e| e.0)
    }
    /// Go's empty slice: the address of three zero words every program has.
    fn nil_slice(&mut self) {
        let g = match self.sh.nil_slice {
            Some(g) => g,
            None => {
                let g = self.sh.nglobals;
                self.sh.nglobals += 3;
                self.sh.nil_slice = Some(g);
                g
            }
        };
        self.emit(Op::AddrG(g));
    }
    /// The zero of a kind, pushed (a slice's is the empty header, a
    /// string's the empty string; an array's the zero of its elements, the
    /// word each of its words is given).
    fn zero(&mut self, t: &Ty) {
        match t {
            Ty::Slice(..) => self.nil_slice(),
            Ty::Str => self.emit(super::strs::empty()),
            Ty::Arr(e, _, _) => {
                let e = (**e).clone();
                self.zero(&e)
            }
            _ => self.emit(Op::Push(0)),
        }
    }
    /// The zeros of `words` words of kind t at the address in slot a.
    fn fill_zero(&mut self, a: u32, t: &Ty, words: u64) {
        let z = self.hidden();
        self.zero(t);
        self.emit(Op::Store(z));
        self.ops(&[Op::Load(a), Op::Push(words as i64), Op::Load(z)]);
        self.lib("lib.fill", 3);
        self.emit(Op::Pop);
    }
    /// Memory for a value of kind t, its address on the stack: the
    /// function's own (frame) where it cannot outlive the call, else new.
    fn memory_for(&mut self, t: &Ty, own: bool) -> Result<(), E> {
        if own && !self.top {
            let at = self.frame_words(t.words())?;
            self.emit(Op::Frame(at));
        } else {
            self.emit(Op::Push(t.bytes() as i64));
            self.lib("lib.bytes", 1);
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

    /// A condition: its value, where a list is true when it has elements.
    fn cond(&mut self, e: &Expr) -> Result<(), E> {
        let t = self.expr(e, true)?;
        match t {
            // a Python list or string is true when it has elements
            Ty::List(_) => self.emit(Op::LoadM(8)),
            Ty::Str if self.sh.l.strings == Strings::Ascii => self.emit(Op::LoadM(8)),
            Ty::Slice(..) | Ty::Arr(..) | Ty::Ptr(..) | Ty::Str => return self.err(format!("{} is not a condition", t.describe())),
            _ => {}
        }
        Ok(())
    }

    /// A value going to a new home (a variable, an argument, an element):
    /// a Go array is copied there, as Go copies arrays, unless it is new.
    fn value(&mut self, e: &Expr) -> Result<Ty, E> {
        let t = self.expr(e, true)?;
        if let Ty::Arr(..) = &t {
            if self.checks() && matches!(e, Expr::Var(..) | Expr::Index(..)) {
                self.copy_new(&t)?;
            }
        }
        Ok(t)
    }

    /// The array whose address is on top, copied to new memory.
    fn copy_new(&mut self, t: &Ty) -> Result<(), E> {
        let src = self.hidden();
        self.emit(Op::Store(src));
        let dst = self.hidden();
        self.memory_for(t, false)?;
        self.emit(Op::Store(dst));
        self.ops(&[Op::Load(dst), Op::Load(src), Op::Push(t.bytes() as i64)]);
        self.lib("lib.copy", 3);
        self.ops(&[Op::Pop, Op::Load(dst)]);
        Ok(())
    }

    /// The value on top stored at the address in slot `a`: a number cut to
    /// the element's width, an array (held in place) copied in.
    fn put(&mut self, a: u32, t: &Ty, w: u8, from: u8) -> Result<(), E> {
        if let Ty::Arr(..) = t {
            let src = self.hidden();
            self.emit(Op::Store(src));
            self.ops(&[Op::Load(a), Op::Load(src), Op::Push(t.bytes() as i64)]);
            self.lib("lib.copy", 3);
            self.emit(Op::Pop);
            return Ok(());
        }
        // a number is cut to the element's width by the store itself; only
        // a constant that does not fit (Go) is checked
        let v = self.hidden();
        if t.scalar() && from == 0 {
            self.narrow(w, from)?;
        }
        self.emit(Op::Store(v));
        self.ops(&[Op::Load(a), Op::Load(v), Op::StoreM(size(t, w) as u8)]);
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt) -> Result<(), E> {
        match s {
            Stmt::Var { name, ty: words, dims, value, line } => {
                self.line = *line;
                self.var(name, words, dims, value.as_ref())?;
            }
            Stmt::Assign { name, index, op, value, line } if !index.is_empty() => {
                self.line = *line;
                self.assign_elem(name, index, *op, value)?;
            }
            Stmt::Assign { name, op, value, line, .. } => {
                self.line = *line;
                match op {
                    Some(op) => self.assign_op(name, *op, value)?,
                    None => self.assign(name, value)?,
                }
            }
            Stmt::If { cond, then, els } => {
                self.cond(cond)?;
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
                    self.cond(c)?;
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
                self.cond(cond)?;
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
                let (counter, end, by) = (self.hidden(), self.hidden(), self.hidden());
                let fixed = self.range_start(args, counter, end, by)?;
                let var = match self.lookup(name) {
                    Some((slot, was)) => {
                        self.same(name, &was, &Ty::Int)?;
                        slot
                    }
                    None => self.declare(name, Ty::Int),
                };
                let (top, cont, done) = (self.label(), self.label(), self.label());
                self.emit(Op::Label(top));
                self.range_test(fixed, counter, end, by);
                self.emit(Op::Jz(done));
                self.load(counter);
                self.store(var);
                self.loops.push((cont, done));
                self.block(body)?;
                self.loops.pop();
                self.emit(Op::Label(cont));
                self.ops(&[Op::Load(counter), Op::Load(by), Op::Bin(BinOp::Add), Op::Store(counter), Op::Jmp(top), Op::Label(done)]);
            }
            Stmt::ForEach { index, value, iter, body, line } => {
                self.line = *line;
                self.for_each(index.as_deref(), value.as_deref(), iter, body)?;
            }
            Stmt::Return(e, line) => {
                self.line = *line;
                if self.top {
                    return self.err("return outside a function");
                }
                match e {
                    Some(e) => {
                        let t = self.expr(e, true)?;
                        if let Some(want) = self.ret_ty.clone() {
                            if unify(&want, &t).is_none() {
                                return self.err(format!("this function returns {}, not {}", want.describe(), t.describe()));
                            }
                        }
                        if !t.scalar() && self.ret_ty.is_none() && self.typed() {
                            return self.err(format!("this function returns a number, not {}", t.describe()));
                        }
                        // a Go array is returned in new memory: the function's
                        // own is gone when it returns
                        if let Ty::Arr(..) = &t {
                            if !self.checks() {
                                return self.err("a C function does not return an array");
                            }
                            self.copy_new(&t)?;
                        }
                        let (to, from) = (self.ret_w, self.last_w);
                        if t.scalar() {
                            self.narrow(to, from)?;
                        }
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
                if !ty.scalar() {
                    return self.err(format!("{name} is {}; a number is read into a number", ty.describe()));
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

    fn assign_op(&mut self, name: &str, op: BinOp, value: &Expr) -> Result<(), E> {
        let Some((slot, was)) = self.lookup(name) else {
            return self.unknown(name);
        };
        if was == Ty::Bool {
            return self.err(format!("{name} holds a truth value; Strike does not do arithmetic into it"));
        }
        if let Ty::List(_) = was {
            // Python's a += b: b's elements appended to a itself
            if op != BinOp::Add {
                return self.err("only += of a list is in Strike yet");
            }
            self.load(slot);
            let t = self.expr(value, true)?;
            if !matches!(t, Ty::List(_)) {
                return self.err(format!("{name} is a list; += takes a list"));
            }
            self.refine(name, &t)?;
            self.lib("lib.list_extend", 2);
            self.emit(Op::Pop);
            return Ok(());
        }
        if was == Ty::Str {
            // s += t, Python's s *= n: a new string in s
            self.load(slot);
            let t = self.expr(value, true)?;
            let r = self.str_op(op, &Ty::Str, &t)?;
            if r != Ty::Str {
                return self.err(format!("{name} is a string; this operator does not make one"));
            }
            self.store(slot);
            return Ok(());
        }
        if !was.scalar() {
            return self.err(format!("{name} is {}; Strike does not do arithmetic into it", was.describe()));
        }
        self.load(slot);
        self.int_expr(value)?;
        let op = if self.sh.l.checked { op.checked() } else { op };
        self.emit(Op::Bin(op));
        // done at the promoted width, then put back in the variable's
        let w = self.width(slot);
        self.narrow(w, 64)?;
        self.store(slot);
        Ok(())
    }

    fn assign(&mut self, name: &str, value: &Expr) -> Result<(), E> {
        let known = self.lookup(name);
        if let Some((slot, t @ Ty::Arr(..))) = &known {
            // an array assigned whole: Go copies the elements in
            if !self.checks() {
                return self.err("an array is not assigned whole in C");
            }
            let got = self.expr(value, true)?;
            if got != *t {
                return self.err(format!("{name} is an array; it is given {}", got.describe()));
            }
            let dst = self.hidden();
            let t = t.clone();
            let slot = *slot;
            self.load_var(slot, &t);
            self.emit(Op::Store(dst));
            return self.put(dst, &t, 64, 64);
        }
        let ty = self.value(value)?;
        let from = self.last_w;
        let slot = match known {
            Some((slot, was)) => {
                if self.typed() && !was.scalar() && unify(&was, &ty).is_none() && !decays(&ty, &was) {
                    return self.err(format!("{name} is {}; it is given {}", was.describe(), ty.describe()));
                }
                if self.typed() && was.scalar() && !ty.scalar() {
                    return self.err(format!("{name} is a number; it is given {}", ty.describe()));
                }
                self.same(name, &was, &ty)?;
                slot
            }
            None if !self.sh.l.block_scope => {
                let s = self.declare(name, ty.clone());
                self.refine(name, &ty)?;
                s
            }
            None => return self.unknown(name),
        };
        if ty.scalar() {
            let w = self.width(slot);
            self.narrow(w, from)?;
        }
        self.store(slot);
        Ok(())
    }

    /// range(...)'s start, stop and step into their slots; the step when it
    /// is a constant.
    fn range_start(&mut self, args: &[Expr], counter: u32, end: u32, by: u32) -> Result<Option<i64>, E> {
        let (start, stop, step) = match args {
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
        match start {
            Some(a) => self.int_expr(a)?,
            None => self.emit(Op::Push(0)),
        }
        self.store(counter);
        self.int_expr(stop)?;
        self.store(end);
        match step {
            Some(c) => self.int_expr(c)?,
            None => self.emit(Op::Push(1)),
        }
        self.store(by);
        Ok(fixed)
    }

    fn range_test(&mut self, fixed: Option<i64>, counter: u32, end: u32, by: u32) {
        match fixed {
            Some(v) => self.ops(&[Op::Load(counter), Op::Load(end), Op::Bin(if v > 0 { BinOp::Lt } else { BinOp::Gt })]),
            None => {
                // (step > 0 and i < end) or (step < 0 and i > end)
                for (sign, cmp) in [(BinOp::Gt, BinOp::Lt), (BinOp::Lt, BinOp::Gt)] {
                    self.ops(&[Op::Load(by), Op::Push(0), Op::Bin(sign), Op::Load(counter), Op::Load(end), Op::Bin(cmp), Op::Bin(BinOp::Band)]);
                }
                self.emit(Op::Bin(BinOp::Bor));
            }
        }
    }

    /// An expression that must be a number.
    fn int_expr(&mut self, e: &Expr) -> Result<(), E> {
        let t = self.expr(e, true)?;
        if !t.scalar() {
            return self.err(format!("a number is wanted here, not {}", t.describe()));
        }
        Ok(())
    }

    fn var(&mut self, name: &str, words: &[String], dims: &[Option<Expr>], value: Option<&Expr>) -> Result<(), E> {
        let (mut dt, mut dw) = type_of(self.sh.l, words, self.sh.bools).map_err(|m| (self.line, m))?;
        // a C pointer variable: only one given an array or a string to
        // point at (no arithmetic on it, no & or *)
        if matches!(dt, Ty::Ptr(..)) && (value.is_none() || !dims.is_empty()) {
            return self.err("pointer variables are not in Strike yet, but one given an array or a string to point at is");
        }
        // C: int a[3][4]: the sizes after the name, the first outermost
        if !dims.is_empty() {
            let mut sizes = Vec::new();
            for (i, d) in dims.iter().enumerate() {
                match d {
                    Some(e) => match constant(e) {
                        Some(n) if n > 0 => sizes.push(n as u64),
                        Some(_) => return self.err("an array's size is at least 1"),
                        None => return self.err("an array's size is a constant in Strike (no variable-length arrays)"),
                    },
                    None if i == 0 => match value {
                        Some(Expr::Brace(elems, _)) => sizes.push(elems.len().max(1) as u64),
                        // char s[] = "abc": the chars and their zero
                        Some(Expr::Str(b)) if self.sh.l.strings == Strings::C => sizes.push(b.len() as u64 + 1),
                        _ => return self.err("an array without its size needs an initializer to count"),
                    },
                    None => return self.err("only an array's first size may be left out"),
                }
            }
            let mut ew = dw;
            for n in sizes.into_iter().rev() {
                dt = Ty::Arr(Box::new(dt), ew, n);
                ew = 64;
            }
        }
        if let Ty::Arr(..) = &dt {
            return self.var_array(name, dt, value);
        }
        let twice = if self.globals_here { self.sh.globals.contains_key(name) } else { self.scopes.last().unwrap().contains_key(name) };
        if self.sh.l.block_scope && twice {
            return self.err(format!("{name} is declared twice in one block"));
        }
        let ty = match value {
            Some(e) => {
                let t = self.value(e)?;
                let from = self.last_w;
                if !words.is_empty() && (!dt.scalar() || !t.scalar()) && unify(&dt, &t).is_none() && !decays(&t, &dt) {
                    return self.err(format!("{name} is {}; it is given {}", dt.describe(), t.describe()));
                }
                // Go's `x := 5` is an int: the literal's own width; `c :=
                // s[0]` is a byte, `x := int32(y)` an int32: the value's
                if t.scalar() {
                    self.narrow(dw, from)?;
                }
                if words.is_empty() && self.typed() && t == Ty::Int && from != 0 {
                    dw = from;
                }
                if words.is_empty() || (dt.scalar() && (dt == Ty::Bool || t == Ty::Bool)) {
                    t
                } else {
                    dt.clone()
                }
            }
            None => {
                self.zero(&dt);
                dt.clone()
            }
        };
        let slot = match (self.sh.l.block_scope, self.lookup(name)) {
            (false, Some((slot, was))) => {
                self.same(name, &was, &ty)?;
                slot
            }
            _ => {
                let s = self.declare(name, ty.clone());
                self.set_width(s, if ty.scalar() { dw } else { 64 });
                if !ty.scalar() && !self.typed() {
                    self.refine(name, &ty)?;
                }
                s
            }
        };
        self.store(slot);
        Ok(())
    }

    /// A C or Go array variable: its memory (the function's own where it
    /// cannot outlive the call, a global's where it is one), zeros where
    /// the language gives them, then the initializer's elements.
    fn var_array(&mut self, name: &str, t: Ty, value: Option<&Expr>) -> Result<(), E> {
        let words = t.words();
        if self.globals_here {
            let w = u32::try_from(words).ok().filter(|w| *w < 1 << 26).ok_or((self.line, "a global array this large is not in Strike yet".to_string()))?;
            if self.sh.globals.contains_key(name) {
                return self.err(format!("{name} is declared twice"));
            }
            let g = self.declare_words(name, t.clone(), w);
            let a = self.hidden();
            self.emit(Op::AddrG(g & !GLOBAL));
            self.emit(Op::Store(a));
            if t.zero_is_not_zero() {
                self.fill_zero(a, &t, words);
            }
            if let Some(v) = value {
                self.init_array(a, &t, v)?;
            }
            return Ok(());
        }
        if self.scopes.last().unwrap().contains_key(name) {
            return self.err(format!("{name} is declared twice in one block"));
        }
        let own = !self.sliced.contains(name);
        self.memory_for(&t, own)?;
        let slot = self.declare(name, t.clone());
        self.store(slot);
        let a = self.hidden();
        self.ops(&[Op::Load(slot), Op::Store(a)]);
        // Go gives zeros every time the declaration runs; C gives them only
        // with an initializer (its other elements are zero)
        if self.checks() || value.is_some() {
            self.fill_zero(a, &t, words);
        }
        if let Some(v) = value {
            self.init_array(a, &t, v)?;
        }
        Ok(())
    }

    /// The elements of an initializer ({...}, a Go composite of the
    /// array's type, or another array) into the array at slot a.
    fn init_array(&mut self, a: u32, t: &Ty, v: &Expr) -> Result<(), E> {
        let Ty::Arr(e, w, n) = t else { unreachable!() };
        if let Expr::Str(bytes) = v {
            if self.sh.l.strings != Strings::C || **e != Ty::Int || *w != 8 {
                return self.err("only a char array is given a string");
            }
            // C keeps the zero byte where there is room for it
            if bytes.len() as u64 > *n {
                return self.err(format!("a string of {} chars is given to an array of {n}", bytes.len()));
            }
            for (i, c) in bytes.iter().enumerate() {
                self.ops(&[Op::Load(a), Op::Push(i as i64), Op::Bin(BinOp::Add), Op::Push(*c as i8 as i64), Op::StoreM(1)]);
            }
            return Ok(());
        }
        let elems = match v {
            Expr::Brace(elems, _) => elems,
            Expr::Composite(words, elems, _) => {
                let (ct, _) = type_of(self.sh.l, words, self.sh.bools).map_err(|m| (self.line, m))?;
                if ct != *t {
                    return self.err(format!("the array is {}, given another kind", t.describe()));
                }
                elems
            }
            other => {
                let got = self.expr(other, true)?;
                if got != *t {
                    return self.err(format!("the array is given {}", got.describe()));
                }
                return self.put(a, t, 64, 64);
            }
        };
        if elems.len() as u64 > *n {
            return self.err(format!("{} elements are given to an array of {n}", elems.len()));
        }
        let stride = size(e, *w);
        for (i, x) in elems.iter().enumerate() {
            let at = self.hidden();
            self.ops(&[Op::Load(a), Op::Push((i as u64 * stride) as i64), Op::Bin(BinOp::Add), Op::Store(at)]);
            self.elem_value(at, e, *w, x)?;
        }
        Ok(())
    }

    /// One element of an initializer or composite stored at the address in
    /// slot `at`: a nested {...} is an element of the element's own type.
    fn elem_value(&mut self, at: u32, e: &Ty, w: u8, x: &Expr) -> Result<(), E> {
        match (e, x) {
            (Ty::Arr(..), Expr::Brace(..)) => self.init_array(at, e, x),
            (Ty::Arr(..), Expr::Str(_)) if self.sh.l.strings == Strings::C => self.init_array(at, e, x),
            (Ty::Slice(ie, iw), Expr::Brace(inner, _)) => {
                let (ie, iw) = ((**ie).clone(), *iw);
                self.composite_slice(&ie, iw, inner)?;
                self.put(at, e, 64, 64)
            }
            (_, Expr::Brace(..)) => self.err("a nested initializer for an element that is not an array"),
            _ => {
                let t = self.value(x)?;
                let from = self.last_w;
                if unify(e, &t).is_none() && !(e.scalar() && t.scalar() && !self.sh.bools) {
                    return self.err(format!("an element is {}, given {}", e.describe(), t.describe()));
                }
                self.put(at, e, w, from)
            }
        }
    }

    /// Go's []T{...}: a new slice of the elements, its address on the stack.
    fn composite_slice(&mut self, e: &Ty, w: u8, elems: &[Expr]) -> Result<(), E> {
        let n = elems.len() as i64;
        let h = self.hidden();
        self.ops(&[Op::Push(n), Op::Push(n), Op::Push(size(e, w) as i64)]);
        self.lib("lib.slice_new", 3);
        self.emit(Op::Store(h));
        let data = self.hidden();
        self.ops(&[Op::Load(h), Op::LoadM(8), Op::Store(data)]);
        if e.zero_is_not_zero() {
            self.fill_zero(data, e, n as u64 * e.words());
        }
        for (i, x) in elems.iter().enumerate() {
            let at = self.hidden();
            self.ops(&[Op::Load(data), Op::Push(i as i64 * size(e, w) as i64), Op::Bin(BinOp::Add), Op::Store(at)]);
            self.elem_value(at, e, w, x)?;
        }
        self.emit(Op::Load(h));
        Ok(())
    }

    /// base[idx]'s address on the stack: the index checked where the
    /// language checks it, negative from the end in Python. The element's
    /// kind and width.
    fn elem_addr(&mut self, base: &Expr, idx: &Expr) -> Result<(Ty, u8), E> {
        let start = self.code.len();
        let bt = self.expr(base, true)?;
        if bt == Ty::Str {
            return self.err("a string is not changed in place: Python's and Go's strings never change");
        }
        self.elem_addr_of(start, bt, idx)
    }

    /// elem_addr for the base whose code begins at `start` and is of kind bt.
    fn elem_addr_of(&mut self, start: usize, bt: Ty, idx: &Expr) -> Result<(Ty, u8), E> {
        let Some((e, w)) = bt.elem() else {
            return self.err(format!("{} has no elements to index", bt.describe()));
        };
        let stride = size(&e, w) as i64;
        let scale = |fx: &mut Self| {
            if stride.count_ones() == 1 {
                if stride > 1 {
                    fx.ops(&[Op::Push(stride.trailing_zeros() as i64), Op::Bin(BinOp::Shl)]);
                }
            } else {
                fx.ops(&[Op::Push(stride), Op::Bin(BinOp::Mul)]);
            }
            fx.emit(Op::Bin(BinOp::Add));
        };
        // nothing checked (C): base, index, the address
        if matches!(bt, Ty::Ptr(..)) || (matches!(bt, Ty::Arr(..)) && !self.checks()) {
            self.int_expr(idx)?;
            scale(self);
            return Ok((e, w));
        }
        let hs = self.into_slot(start);
        let at = self.code.len();
        self.int_expr(idx)?;
        // Python moves a negative index into range: never in the variable itself
        let i = if matches!(bt, Ty::List(_)) && self.sh.l.negative_index {
            let h = self.hidden();
            self.emit(Op::Store(h));
            h
        } else {
            self.into_slot(at)
        };
        match &bt {
            Ty::Arr(_, _, n) => {
                self.check(i, &[Op::Push(*n as i64)]);
                self.emit(Op::Load(hs));
            }
            Ty::Slice(..) => {
                self.check(i, &[Op::Load(hs), Op::Push(8), Op::Bin(BinOp::Add), Op::LoadM(8)]);
                self.ops(&[Op::Load(hs), Op::LoadM(8)]);
            }
            Ty::List(_) => {
                if self.sh.l.negative_index {
                    let pos = self.label();
                    self.ops(&[Op::Load(i), Op::Push(0), Op::Bin(BinOp::Lt), Op::Jz(pos), Op::Load(i), Op::Load(hs), Op::LoadM(8), Op::Bin(BinOp::Add), Op::Store(i), Op::Label(pos)]);
                }
                self.check(i, &[Op::Load(hs), Op::LoadM(8)]);
                self.ops(&[Op::Load(hs), Op::Push(16), Op::Bin(BinOp::Add), Op::LoadM(8)]);
            }
            _ => unreachable!(),
        }
        self.emit(Op::Load(i));
        scale(self);
        Ok((e, w))
    }

    /// The value the code from `start` left, in a slot: the variable's own
    /// where that code is only its load, else a new one.
    fn into_slot(&mut self, start: usize) -> u32 {
        if self.code.len() == start + 1 {
            if let Some(Op::Load(s)) = self.code.last() {
                let s = *s;
                self.code.pop();
                return s;
            }
        }
        let h = self.hidden();
        self.emit(Op::Store(h));
        h
    }

    /// name[i][j] = value (or op=): the element's address worked out once.
    fn assign_elem(&mut self, name: &str, index: &[Expr], op: Option<BinOp>, value: &Expr) -> Result<(), E> {
        let line = self.line;
        // the target as an expression: name[i][j] is (name[i])[j]
        let mut target = Expr::Var(name.to_string(), line);
        for i in &index[..index.len() - 1] {
            target = Expr::Index(Box::new(target), Box::new(i.clone()), line);
        }
        let last = &index[index.len() - 1];
        // Python works out the value first, then where it goes
        let early = if op.is_none() && !self.sh.l.block_scope {
            let t = self.value(value)?;
            let from = self.last_w;
            let v = self.hidden();
            self.emit(Op::Store(v));
            Some((t, v, from))
        } else {
            None
        };
        let (e, w) = self.elem_addr(&target, last)?;
        // a number or an address goes straight to where the address on the
        // stack points
        if op.is_none() && !matches!(e, Ty::Arr(..)) {
            let (t, from) = match early {
                Some((t, v, from)) => {
                    self.emit(Op::Load(v));
                    (t, from)
                }
                None => {
                    let t = self.value(value)?;
                    (t, self.last_w)
                }
            };
            self.elem_kind(&target, &e, &t)?;
            if e.scalar() && from == 0 {
                self.narrow(w, from)?;
            }
            self.emit(Op::StoreM(size(&e, w) as u8));
            return Ok(());
        }
        // a[i] += x worked on the memory itself, where x calls nothing (so
        // when the element is read does not matter)
        if let Some(op @ (BinOp::Add | BinOp::Sub | BinOp::Band | BinOp::Bor | BinOp::Bxor)) = op {
            if e.scalar() && e != Ty::Bool && calls_nothing(value) {
                self.int_expr(value)?;
                let op = if self.sh.l.checked { op.checked() } else { op };
                self.emit(Op::OpM(op, size(&e, w) as u8));
                return Ok(());
            }
        }
        let a = self.hidden();
        self.emit(Op::Store(a));
        match op {
            None => {
                let (t, from) = match early {
                    Some((t, v, from)) => {
                        self.emit(Op::Load(v));
                        (t, from)
                    }
                    None => {
                        let t = self.value(value)?;
                        (t, self.last_w)
                    }
                };
                self.elem_kind(&target, &e, &t)?;
                self.put(a, &e, w, from)
            }
            Some(op) => {
                if !e.scalar() || e == Ty::Bool {
                    return self.err(format!("an element that is {} is not worked on with arithmetic", e.describe()));
                }
                let n = size(&e, w) as u8;
                // the element first, as it was, then the value: the old
                // element waits in a slot, not under the value on the stack
                let old = self.hidden();
                self.ops(&[Op::Load(a), Op::LoadM(n), Op::Store(old), Op::Load(a), Op::Load(old)]);
                self.int_expr(value)?;
                let op = if self.sh.l.checked { op.checked() } else { op };
                self.emit(Op::Bin(op));
                // the store keeps the element's own width
                self.emit(Op::StoreM(n));
                Ok(())
            }
        }
    }

    /// An element of the container `c` is given a value of kind t: Python
    /// learns the container's elements from it.
    fn elem_kind(&mut self, c: &Expr, e: &Ty, t: &Ty) -> Result<(), E> {
        if unify(e, t).is_some() || (e.scalar() && t.scalar() && !self.sh.bools) {
            if let Expr::Var(n, _) = c {
                if let Some((_, ct @ Ty::List(_))) = self.lookup(n) {
                    let _ = ct;
                    let n = n.clone();
                    self.refine(&n, &Ty::List(Box::new(t.clone())))?;
                }
            }
            return Ok(());
        }
        self.err(format!("an element is {}, given {}; Strike keeps one kind per list", e.describe(), t.describe()))
    }

    fn for_each(&mut self, index: Option<&str>, value: Option<&str>, iter: &Expr, body: &[Stmt]) -> Result<(), E> {
        self.push_scope();
        let t = self.expr(iter, true)?;
        if t == Ty::Str {
            self.for_str(index, value, body)?;
            self.pop_scope();
            return Ok(());
        }
        let Some((e, ew)) = t.elem() else {
            return self.err(format!("a loop goes over a list, a slice or an array, not {}", t.describe()));
        };
        let hs = self.hidden();
        self.emit(Op::Store(hs));
        // Go goes over the array as it was: a copy, where the loop may change it
        if let (Ty::Arr(..), Some(_)) = (&t, value) {
            let copy = match iter {
                Expr::Var(n, _) => changes(body, n) || matches!(self.lookup(n), Some((s, _)) if s & GLOBAL != 0),
                _ => false,
            };
            if copy {
                self.emit(Op::Load(hs));
                self.copy_new(&t)?;
                self.emit(Op::Store(hs));
            }
        }
        let (i, n) = (self.hidden(), self.hidden());
        self.ops(&[Op::Push(0), Op::Store(i)]);
        // Go's slice and array: the length when the loop starts; Python's
        // list: the length every time round (an append is seen)
        let len: Vec<Op> = match &t {
            Ty::Arr(_, _, k) => vec![Op::Push(*k as i64)],
            Ty::Slice(..) => {
                self.ops(&[Op::Load(hs), Op::Push(8), Op::Bin(BinOp::Add), Op::LoadM(8), Op::Store(n)]);
                vec![Op::Load(n)]
            }
            Ty::List(_) => vec![Op::Load(hs), Op::LoadM(8)],
            _ => return self.err("a C pointer has no length to loop over"),
        };
        let iv = match index {
            Some(name) => Some(self.loop_var(name, Ty::Int)?),
            None => None,
        };
        let vv = match value {
            Some(name) => Some(self.loop_var(name, e.clone())?),
            None => None,
        };
        let (top, cont, done) = (self.label(), self.label(), self.label());
        self.ops(&[Op::Label(top), Op::Load(i)]);
        self.ops(&len);
        self.ops(&[Op::Bin(BinOp::Lt), Op::Jz(done)]);
        if let Some(s) = iv {
            self.emit(Op::Load(i));
            self.store(s);
        }
        if let Some(s) = vv {
            match &t {
                Ty::Arr(..) => self.emit(Op::Load(hs)),
                Ty::Slice(..) => self.ops(&[Op::Load(hs), Op::LoadM(8)]),
                _ => self.ops(&[Op::Load(hs), Op::Push(16), Op::Bin(BinOp::Add), Op::LoadM(8)]),
            }
            self.ops(&[Op::Load(i), Op::Push(size(&e, ew) as i64), Op::Bin(BinOp::Mul), Op::Bin(BinOp::Add)]);
            match load_of(&e, ew) {
                Some(l) => self.emit(l),
                // an array element: the value is a copy of it
                None => self.copy_new(&e)?,
            }
            self.store(s);
        }
        self.loops.push((cont, done));
        self.block(body)?;
        self.loops.pop();
        self.ops(&[Op::Label(cont), Op::Load(i), Op::Push(1), Op::Bin(BinOp::Add), Op::Store(i), Op::Jmp(top), Op::Label(done)]);
        self.pop_scope();
        Ok(())
    }

    /// A loop's variable: Go declares it for the loop, Python assigns it.
    fn loop_var(&mut self, name: &str, t: Ty) -> Result<u32, E> {
        if self.sh.l.block_scope {
            return Ok(self.declare(name, t));
        }
        match self.lookup(name) {
            Some((slot, was)) => {
                self.same(name, &was, &t)?;
                Ok(slot)
            }
            None => {
                let s = self.declare(name, t.clone());
                self.refine(name, &t)?;
                Ok(s)
            }
        }
    }

    fn print_value(&mut self, e: &Expr) -> Result<(), E> {
        if let Expr::Str(s) = e {
            // Python writes its strings in the encoding of where they go,
            // which is not known while compiling
            if self.sh.l.strings == Strings::Ascii && !s.is_ascii() {
                return self.err("a string past ASCII is not in Strike yet: Python prints it in the encoding of where its output goes");
            }
            self.emit(Op::PrintStr(s.clone()));
            return Ok(());
        }
        let t = self.expr(e, true)?;
        match (&self.sh.l.bool_print, &t) {
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
            (_, Ty::List(_) | Ty::Slice(..) | Ty::Arr(..)) => {
                if !self.checks() {
                    return self.err("C does not print an array");
                }
                let f = self.printer(&t);
                self.emit(Op::Call(f, 1));
                self.emit(Op::Pop);
            }
            (_, Ty::Str) => {
                self.lib("lib.str_print", 1);
                self.emit(Op::Pop);
            }
            (_, Ty::Ptr(..)) => return self.err("a pointer is not printed"),
            _ => self.emit(Op::PrintInt),
        }
        Ok(())
    }

    /// The library function that prints a list, slice or array of kind t.
    fn printer(&mut self, t: &Ty) -> String {
        let name = format!("lib.print.{t:?}");
        if self.sh.printers.contains_key(&name) {
            return name;
        }
        let (e, ew) = t.elem().unwrap();
        let elem = match &e {
            Ty::Bool => Elem::Bool,
            Ty::List(_) | Ty::Slice(..) => Elem::Nested(self.printer(&e), false),
            Ty::Arr(..) => Elem::Nested(self.printer(&e), true),
            // Python shows a string in a list as its repr, Go as it is
            Ty::Str => Elem::Str(self.sh.l.strings == Strings::Ascii),
            _ => Elem::Int,
        };
        let (open, sep, close) = self.sh.l.list_print.clone();
        let p = Printer {
            array: if let Ty::Arr(_, _, n) = t { Some(*n) } else { None },
            list: matches!(t, Ty::List(_)),
            elem,
            stride: size(&e, ew),
            width: size(&e, ew) as u8,
            open: open.into_bytes(),
            sep: sep.into_bytes(),
            close: close.into_bytes(),
            words: self.sh.l.bool_print.clone().map(|(a, b)| (a.into_bytes(), b.into_bytes())),
        };
        self.sh.printers.insert(name.clone(), p);
        name
    }

    /// The text whose value is on top printed: a Python or Go string, or
    /// C's chars to their zero byte (a char array, or a char pointer).
    fn print_text(&mut self, t: &Ty, what: &str) -> Result<(), E> {
        match t {
            Ty::Str => self.lib("lib.str_print", 1),
            Ty::Ptr(e, 8) | Ty::Arr(e, 8, _) if **e == Ty::Int && self.sh.l.strings == Strings::C => self.lib("lib.c_print", 1),
            _ => return self.err(format!("{what} is given {}, not a string", t.describe())),
        }
        self.emit(Op::Pop);
        Ok(())
    }

    /// Is this C expression an unsigned size (strlen's size_t, or
    /// arithmetic on one)? It wraps as the signed numbers do, but it
    /// compares unsigned, and it divides and shifts unsigned.
    fn unsigned(&self, e: &Expr) -> bool {
        match e {
            Expr::Call(n, _, _) => !self.sh.sigs.contains_key(n) && self.sh.l.builtins.get(n).map(String::as_str) == Some("c_strlen"),
            Expr::Bin(op, a, b) if !op.is_comparison() && !matches!(op, BinOp::AndBool | BinOp::OrBool) => self.unsigned(a) || self.unsigned(b),
            Expr::Un(UnOp::Neg | UnOp::Plus | UnOp::Bnot, x) => self.unsigned(x),
            _ => false,
        }
    }

    /// The C text (a char array or a char pointer) for strlen, strcmp.
    fn c_text(&mut self, e: &Expr, name: &str) -> Result<(), E> {
        let t = self.expr(e, true)?;
        match &t {
            Ty::Ptr(x, 8) | Ty::Arr(x, 8, _) if **x == Ty::Int => Ok(()),
            _ => self.err(format!("{name} is given {}, not chars", t.describe())),
        }
    }

    /// An expression that must be a string.
    fn str_expr(&mut self, e: &Expr, what: &str) -> Result<(), E> {
        let t = self.expr(e, true)?;
        if t != Ty::Str {
            return self.err(format!("{what} is given {}, not a string", t.describe()));
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
                [a] => {
                    let t = self.expr(a, true)?;
                    self.print_text(&t, "this print")?;
                    self.emit(Op::PrintStr(b"\n".to_vec()));
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
                    let mut sized = false;
                    while matches!(f.get(i), Some(&b'l') | Some(&b'z')) {
                        sized |= f[i] == b'z';
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
                            if !t.scalar() {
                                return self.err(format!("%d is given {}", t.describe()));
                            }
                            self.emit(Op::PrintInt);
                        }
                        b'v' => self.print_value(a)?,
                        b's' => match a {
                            Expr::Str(s) => self.emit(Op::PrintStr(s.clone())),
                            _ => {
                                let t = self.expr(a, true)?;
                                // Go's %s of a slice or array of strings: as %v
                                let mut inner = &t;
                                while let Ty::Slice(e, _) | Ty::Arr(e, _, _) = inner {
                                    inner = &**e;
                                }
                                if self.sh.l.strings == Strings::Bytes && t != Ty::Str && *inner == Ty::Str {
                                    let f = self.printer(&t);
                                    self.emit(Op::Call(f, 1));
                                    self.emit(Op::Pop);
                                } else {
                                    self.print_text(&t, "%s")?;
                                }
                            }
                        },
                        // a character: C's int made a byte, Go's code point
                        // as its UTF-8
                        b'c' if self.sh.l.strings != Strings::None => {
                            let t = self.expr(a, true)?;
                            if !t.scalar() || (t == Ty::Bool && self.sh.bools) {
                                return self.err(format!("%c is given {}", t.describe()));
                            }
                            if self.sh.l.strings == Strings::C {
                                self.lib("lib.print_byte", 1);
                            } else {
                                self.lib("lib.rune_str", 1);
                                self.lib("lib.str_print", 1);
                            }
                            self.emit(Op::Pop);
                        }
                        b'q' if self.sh.l.strings == Strings::Bytes => {
                            let t = self.expr(a, true)?;
                            if t != Ty::Str {
                                return self.err(format!("%q is given {}; it quotes a string in this stage of Strike", t.describe()));
                            }
                            self.lib("lib.go_quote", 1);
                            self.lib("lib.str_print", 1);
                            self.emit(Op::Pop);
                        }
                        // %zu of strlen: a size, never negative
                        b'u' if sized && self.unsigned(a) => {
                            self.int_expr(a)?;
                            self.emit(Op::PrintInt);
                        }
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

    /// A built-in function of the language (len, append, make, cap).
    fn builtin(&mut self, what: &str, name: &str, args: &[Expr]) -> Result<Ty, E> {
        match what {
            "len" | "cap" => {
                let [x] = args else { return self.err(format!("{name} takes one value")) };
                let t = self.expr(x, true)?;
                match (&t, what) {
                    (Ty::List(_) | Ty::Str, "len") => self.emit(Op::LoadM(8)),
                    (Ty::Slice(..), _) => self.ops(&[Op::Push(if what == "len" { 8 } else { 16 }), Op::Bin(BinOp::Add), Op::LoadM(8)]),
                    (Ty::Arr(_, _, n), _) => self.ops(&[Op::Pop, Op::Push(*n as i64)]),
                    _ => return self.err(format!("{name} of {} is not in Strike yet", t.describe())),
                }
                self.last_w = 64;
                Ok(Ty::Int)
            }
            "make" => {
                let (words, n, c) = match args {
                    [Expr::Type(w, _), n] => (w, n, None),
                    [Expr::Type(w, _), n, c] => (w, n, Some(c)),
                    _ => return self.err("make takes a slice type and a length (and a capacity)"),
                };
                let (t, _) = type_of(self.sh.l, words, self.sh.bools).map_err(|m| (self.line, m))?;
                let Ty::Slice(e, ew) = &t else { return self.err("make makes slices in this stage of Strike") };
                let (e, ew) = ((**e).clone(), *ew);
                let (ns, cs) = (self.hidden(), self.hidden());
                self.int_expr(n)?;
                self.emit(Op::Store(ns));
                match c {
                    Some(c) => self.int_expr(c)?,
                    None => self.emit(Op::Load(ns)),
                }
                self.emit(Op::Store(cs));
                // 0 <= len <= cap, and a capacity that can be had
                let code = self.go_fail();
                self.ops(&[Op::Load(ns), Op::Load(cs), Op::Bin(BinOp::Ule)]);
                self.fail_unless(code, "panic: runtime error: makeslice: len out of range\n");
                self.ops(&[Op::Load(cs), Op::Push(MAX_LEN), Op::Bin(BinOp::Ule)]);
                self.fail_unless(code, "panic: runtime error: makeslice: cap out of range\n");
                let h = self.hidden();
                self.ops(&[Op::Load(ns), Op::Load(cs), Op::Push(size(&e, ew) as i64)]);
                self.lib("lib.slice_new", 3);
                self.emit(Op::Store(h));
                if e.zero_is_not_zero() {
                    let (d, all, z) = (self.hidden(), self.hidden(), self.hidden());
                    self.ops(&[Op::Load(h), Op::LoadM(8), Op::Store(d)]);
                    self.ops(&[Op::Load(cs), Op::Push((size(&e, ew) / 8) as i64), Op::Bin(BinOp::Mul), Op::Store(all)]);
                    self.zero(&e);
                    self.ops(&[Op::Store(z), Op::Load(d), Op::Load(all), Op::Load(z)]);
                    self.lib("lib.fill", 3);
                    self.emit(Op::Pop);
                }
                self.emit(Op::Load(h));
                self.last_w = 64;
                Ok(t)
            }
            "append" => {
                let Some((s, vals)) = args.split_first() else { return self.err("append takes a slice") };
                let t = self.expr(s, true)?;
                let Ty::Slice(e, w) = &t else { return self.err(format!("append takes a slice, not {}", t.describe())) };
                let (e, w) = ((**e).clone(), *w);
                let hs = self.hidden();
                self.emit(Op::Store(hs));
                // the values first, as Go works them out, then the room for them
                let mut held = Vec::new();
                for v in vals {
                    let vt = self.value(v)?;
                    let from = self.last_w;
                    if unify(&e, &vt).is_none() && !(e.scalar() && vt.scalar()) {
                        return self.err(format!("the slice holds {}, given {}", e.describe(), vt.describe()));
                    }
                    if e.scalar() {
                        self.narrow(w, from)?;
                    }
                    let x = self.hidden();
                    self.emit(Op::Store(x));
                    held.push(x);
                }
                let (go, noscan) = go_size(&e, w);
                let nh = self.hidden();
                self.ops(&[Op::Load(hs), Op::Push(held.len() as i64), Op::Push(go), Op::Push(noscan as i64), Op::Push(size(&e, w) as i64)]);
                self.lib("lib.slice_grow", 5);
                self.emit(Op::Store(nh));
                let old = self.hidden();
                self.ops(&[Op::Load(hs), Op::Push(8), Op::Bin(BinOp::Add), Op::LoadM(8), Op::Store(old)]);
                for (k, x) in held.iter().enumerate() {
                    let at = self.hidden();
                    self.ops(&[Op::Load(nh), Op::LoadM(8), Op::Load(old), Op::Push(k as i64), Op::Bin(BinOp::Add), Op::Push(size(&e, w) as i64), Op::Bin(BinOp::Mul), Op::Bin(BinOp::Add), Op::Store(at)]);
                    self.emit(Op::Load(*x));
                    self.put(at, &e, 64, 64)?;
                }
                self.emit(Op::Load(nh));
                self.last_w = 64;
                Ok(t)
            }
            // Python's str(x) of a number, a truth value or a string
            "str" => {
                let [x] = args else { return self.err("str takes one value") };
                let t = self.expr(x, true)?;
                match t {
                    Ty::Str => {}
                    Ty::Bool if self.sh.bools => {
                        let (yes, no) = self.sh.l.bool_print.clone().unwrap_or_default();
                        let (other, end) = (self.label(), self.label());
                        self.emit(Op::Jz(other));
                        self.emit(super::strs::literal(yes.as_bytes()));
                        self.emit(Op::Jmp(end));
                        self.emit(Op::Label(other));
                        self.emit(super::strs::literal(no.as_bytes()));
                        self.emit(Op::Label(end));
                    }
                    t if t.scalar() => self.lib("lib.int_str", 1),
                    t => return self.err(format!("str of {} is not in Strike yet", t.describe())),
                }
                self.last_w = 64;
                Ok(Ty::Str)
            }
            // Go's strconv.Itoa
            "itoa" => {
                let [x] = args else { return self.err(format!("{name} takes one number")) };
                self.int_expr(x)?;
                self.lib("lib.int_str", 1);
                self.last_w = 64;
                Ok(Ty::Str)
            }
            "ord" => {
                let [x] = args else { return self.err("ord takes one string") };
                self.str_expr(x, "ord")?;
                self.lib("lib.ord", 1);
                self.last_w = 64;
                Ok(Ty::Int)
            }
            "chr" => {
                let [x] = args else { return self.err("chr takes one number") };
                self.int_expr(x)?;
                self.lib("lib.chr", 1);
                self.last_w = 64;
                Ok(Ty::Str)
            }
            // Python's input(): its prompt printed first, as Python does
            "input" => {
                match args {
                    [] => {}
                    [p] => {
                        self.str_expr(p, "input")?;
                        self.lib("lib.str_print", 1);
                        self.emit(Op::Pop);
                    }
                    _ => return self.err("input takes at most a prompt"),
                }
                self.lib("lib.read_str", 0);
                self.last_w = 64;
                Ok(Ty::Str)
            }
            "c_strlen" => {
                let [x] = args else { return self.err("strlen takes one string") };
                self.c_text(x, "strlen")?;
                self.lib("lib.c_strlen", 1);
                self.last_w = 64;
                Ok(Ty::Int)
            }
            "c_strcmp" => {
                let [x, y] = args else { return self.err("strcmp takes two strings") };
                self.c_text(x, "strcmp")?;
                self.c_text(y, "strcmp")?;
                self.lib("lib.c_strcmp", 2);
                self.last_w = 32;
                Ok(Ty::Int)
            }
            other => self.err(format!("{name} ({other}) is not in Strike yet")),
        }
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
            Expr::Str(bytes) => {
                self.last_w = 64;
                match self.sh.l.strings {
                    Strings::C => {
                        let mut d = bytes.clone();
                        d.push(0);
                        self.emit(Op::Data(d));
                        Ty::Ptr(Box::new(Ty::Int), 8)
                    }
                    Strings::Bytes => {
                        self.emit(super::strs::literal(bytes));
                        Ty::Str
                    }
                    Strings::Ascii => {
                        if !bytes.is_ascii() {
                            return self.err("a string past ASCII is not in Strike yet: Strike's Python strings hold ASCII only, so each code point is one byte");
                        }
                        self.emit(super::strs::literal(bytes));
                        Ty::Str
                    }
                    Strings::None => return self.err("a string can only be printed in this stage of Strike"),
                }
            }
            Expr::Var(name, line) => {
                self.line = *line;
                let Some((slot, ty)) = self.lookup(name) else {
                    return self.unknown(name);
                };
                self.load_var(slot, &ty);
                self.last_w = if ty.scalar() { self.width(slot) } else { 64 };
                ty
            }
            Expr::Index(base, idx, line) => {
                self.line = *line;
                let start = self.code.len();
                let bt = self.expr(base, true)?;
                if bt == Ty::Str {
                    return self.str_index(start, idx);
                }
                let (t, w) = self.elem_addr_of(start, bt, idx)?;
                if let Some(l) = load_of(&t, w) {
                    self.emit(l);
                }
                self.last_w = if t.scalar() { w } else { 64 };
                t
            }
            Expr::Slice(base, lo, hi, line) => {
                self.line = *line;
                self.slice(base, lo.as_deref(), hi.as_deref())?
            }
            Expr::Method(base, name, args, line) => {
                self.line = *line;
                self.method(base, name, args)?
            }
            Expr::List(elems, line) => {
                self.line = *line;
                let h = self.hidden();
                self.emit(Op::Push(elems.len() as i64));
                self.lib("lib.list_new", 1);
                self.emit(Op::Store(h));
                let mut et = Ty::Unknown;
                for (i, x) in elems.iter().enumerate() {
                    self.ops(&[Op::Load(h), Op::Push(16), Op::Bin(BinOp::Add), Op::LoadM(8), Op::Push(8 * i as i64), Op::Bin(BinOp::Add)]);
                    let t = self.value(x)?;
                    et = match unify(&et, &t) {
                        Some(u) => u,
                        None => return self.err(format!("a list of {} and {}: Strike keeps one kind per list", et.describe(), t.describe())),
                    };
                    self.emit(Op::StoreM(8));
                }
                self.emit(Op::Load(h));
                self.last_w = 64;
                Ty::List(Box::new(et))
            }
            Expr::Comp { elem, name, iter, cond, line } => {
                self.line = *line;
                self.comprehension(elem, name, iter, cond.as_deref())?
            }
            Expr::Composite(words, elems, line) => {
                self.line = *line;
                let (t, _) = type_of(self.sh.l, words, self.sh.bools).map_err(|m| (self.line, m))?;
                match &t {
                    Ty::Slice(el, w) => {
                        let (el, w) = ((**el).clone(), *w);
                        self.composite_slice(&el, w, elems)?;
                    }
                    Ty::Arr(..) => {
                        let a = self.hidden();
                        self.memory_for(&t, false)?;
                        self.emit(Op::Store(a));
                        if t.zero_is_not_zero() {
                            self.fill_zero(a, &t, t.words());
                        }
                        self.init_array(a, &t, e)?;
                        self.emit(Op::Load(a));
                    }
                    _ => return self.err("this composite literal is not in Strike yet"),
                }
                self.last_w = 64;
                t
            }
            Expr::Brace(_, line) => {
                self.line = *line;
                return self.err("a {...} list is only an array's initializer");
            }
            Expr::Type(_, line) => {
                self.line = *line;
                return self.err("a type is not a value");
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
                    }
                }
                // a conversion: Go's int32(x), Python's int(x) of a number
                if !self.sh.sigs.contains_key(name) && self.sh.l.type_words.contains(name) && args.len() == 1 {
                    match type_of(self.sh.l, std::slice::from_ref(name), self.sh.bools) {
                        Ok((Ty::Int, w)) => {
                            let t = self.expr(&args[0], true)?;
                            // Python's int(s): the string read as a whole number
                            if t == Ty::Str && self.sh.l.strings == Strings::Ascii {
                                self.lib("lib.str_int", 1);
                                self.last_w = 64;
                                return Ok(Ty::Int);
                            }
                            if !t.scalar() {
                                return self.err(format!("a number is wanted here, not {}", t.describe()));
                            }
                            let from = self.last_w;
                            self.narrow(w, from)?;
                            self.last_w = w;
                            return Ok(Ty::Int);
                        }
                        // Go's string(r): the code point's UTF-8
                        Ok((Ty::Str, _)) => {
                            let t = self.expr(&args[0], true)?;
                            match t {
                                Ty::Str => {}
                                Ty::Bool if self.sh.bools => return self.err("a truth value is not made a string"),
                                t if t.scalar() => self.lib("lib.rune_str", 1),
                                t => return self.err(format!("{} is not made a string in Strike yet", t.describe())),
                            }
                            self.last_w = 64;
                            return Ok(Ty::Str);
                        }
                        _ => {}
                    }
                }
                let Some(sig) = self.sh.sigs.get(name).cloned() else {
                    if let Some(what) = self.sh.l.builtins.get(name).cloned() {
                        return self.builtin(&what, name, args);
                    }
                    return self.err(format!("there is no function {name}"));
                };
                if sig.params.len() != args.len() {
                    return self.err(format!("{name} takes {} values, given {}", sig.params.len(), args.len()));
                }
                let mut tys = Vec::new();
                let (pw, rw) = self.sh.fw.get(name).cloned().unwrap_or_default();
                for (i, a) in args.iter().enumerate() {
                    // a Go array is copied by the function it is given to
                    let t = self.expr(a, true)?;
                    let want = sig.params[i].clone();
                    if !want.scalar() || !t.scalar() {
                        if unify(&want, &t).is_none() && !decays(&t, &want) {
                            return self.err(format!("{name}'s parameter {} is {}, given {}", i + 1, want.describe(), t.describe()));
                        }
                        if let (Expr::Var(n, _), Ty::List(_)) = (a, &t) {
                            let n = n.clone();
                            self.refine(&n, &want)?;
                        }
                    } else {
                        // an argument is converted to its parameter's type
                        let from = self.last_w;
                        self.narrow(pw.get(i).copied().unwrap_or(64), from)?;
                    }
                    tys.push(t);
                }
                let seen = self.sh.args.entry(name.clone()).or_insert_with(|| vec![Vec::new(); args.len()]);
                for (i, t) in tys.into_iter().enumerate() {
                    seen[i].push(t);
                }
                if used {
                    self.sh.used.push((name.clone(), *line));
                }
                self.emit(Op::Call(symbol(name), args.len() as u32));
                self.last_w = if rw == 0 || !sig.ret.scalar() { 64 } else { rw };
                sig.ret
            }
            Expr::Un(op, x) => {
                if *op == UnOp::Not {
                    self.cond(x)?;
                    self.emit(Op::Un(UnOp::Not));
                    self.last_w = 8;
                    return Ok(self.bool_ty());
                }
                let t = self.expr(x, true)?;
                if !t.scalar() {
                    return self.err(format!("arithmetic on {} is not in Strike yet", t.describe()));
                }
                let w = self.arith_width(self.last_w, self.last_w);
                match op {
                    UnOp::Plus => {
                        self.last_w = w;
                        Ty::Int
                    }
                    _ => {
                        let op = if self.sh.l.checked && *op == UnOp::Neg { UnOp::NegOv } else { *op };
                        self.emit(Op::Un(op));
                        // -INT_MIN is INT_MIN again at 32 bits; a byte's
                        // negation and complement wrap at 256
                        if (op == UnOp::Neg || (op == UnOp::Bnot && w == U8)) && w > 0 {
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
                    self.cond(a)?;
                    self.emit(jump(cut));
                    self.cond(b)?;
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
                    if !ta.scalar() {
                        return self.err("and/or of lists is not in Strike yet");
                    }
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
                    // C's size_t (strlen): divided and shifted unsigned, which
                    // Strike does not do yet; compared unsigned below
                    let uns = self.sh.l.strings == Strings::C && (self.unsigned(a) || self.unsigned(b));
                    if uns && matches!(op, BinOp::DivTrunc | BinOp::ModTrunc | BinOp::Shr) {
                        return self.err("strlen's size is unsigned, and unsigned division and shifts are not in Strike yet (store it in an int first)");
                    }
                    let ta = self.expr(a, true)?;
                    let wa = self.last_w;
                    let tb = self.expr(b, true)?;
                    let wb = self.last_w;
                    if !ta.scalar() || !tb.scalar() || *op == BinOp::In {
                        return self.list_op(*op, &ta, &tb);
                    }
                    if uns && matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) {
                        // both sides as 64-bit unsigned numbers, as C converts them
                        if matches!(op, BinOp::Gt | BinOp::Ge) {
                            let (x, y) = (self.hidden(), self.hidden());
                            self.ops(&[Op::Store(y), Op::Store(x), Op::Load(y), Op::Load(x)]);
                        }
                        self.emit(Op::Bin(if matches!(op, BinOp::Lt | BinOp::Gt) { BinOp::Ult } else { BinOp::Ule }));
                        self.last_w = 8;
                        return Ok(self.bool_ty());
                    }
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

    /// s[i] of the string whose code begins at `start`: Python's string of
    /// one character (negative from the end), Go's byte.
    fn str_index(&mut self, start: usize, idx: &Expr) -> Result<Ty, E> {
        if self.sh.l.strings == Strings::Ascii {
            self.int_expr(idx)?;
            self.lib("lib.py_index", 2);
            self.last_w = 64;
            return Ok(Ty::Str);
        }
        let hs = self.into_slot(start);
        let at = self.code.len();
        self.int_expr(idx)?;
        let i = self.into_slot(at);
        self.check(i, &[Op::Load(hs), Op::LoadM(8)]);
        self.ops(&[Op::Load(hs), Op::Push(8), Op::Bin(BinOp::Add), Op::Load(i), Op::Bin(BinOp::Add), Op::LoadM(1), Op::Push(255), Op::Bin(BinOp::Band)]);
        self.last_w = U8;
        Ok(Ty::Int)
    }

    /// s[lo:hi] of the string in slot hs: Python's held to the string,
    /// Go's checked (a panic where it does not fit).
    fn str_slice(&mut self, hs: u32, lo: Option<&Expr>, hi: Option<&Expr>) -> Result<Ty, E> {
        self.last_w = 64;
        if self.sh.l.strings == Strings::Ascii {
            self.emit(Op::Load(hs));
            for x in [lo, hi] {
                match x {
                    Some(x) => self.int_expr(x)?,
                    None => self.emit(Op::Push(i64::MIN)),
                }
            }
            self.lib("lib.str_slice", 3);
            self.last_w = 64;
            return Ok(Ty::Str);
        }
        let (l, h) = (self.hidden(), self.hidden());
        match lo {
            Some(x) => self.int_expr(x)?,
            None => self.emit(Op::Push(0)),
        }
        self.emit(Op::Store(l));
        match hi {
            Some(x) => self.int_expr(x)?,
            None => self.ops(&[Op::Load(hs), Op::LoadM(8)]),
        }
        self.emit(Op::Store(h));
        let code = self.go_fail();
        self.ops(&[Op::Load(h), Op::Load(hs), Op::LoadM(8), Op::Bin(BinOp::Ule)]);
        self.fail_unless(code, "panic: runtime error: slice bounds out of range\n");
        self.ops(&[Op::Load(l), Op::Load(h), Op::Bin(BinOp::Ule)]);
        self.fail_unless(code, "panic: runtime error: slice bounds out of range\n");
        self.ops(&[Op::Load(hs), Op::Load(l), Op::Load(h), Op::Load(l), Op::Bin(BinOp::Sub)]);
        self.lib("lib.str_sub", 3);
        self.last_w = 64;
        Ok(Ty::Str)
    }

    /// A loop over the string on top: Python's characters, Go's code
    /// points (decoded as Go decodes UTF-8) and the byte each begins at.
    fn for_str(&mut self, index: Option<&str>, value: Option<&str>, body: &[Stmt]) -> Result<(), E> {
        let go = self.sh.l.strings == Strings::Bytes;
        let (hs, i, n, step) = (self.hidden(), self.hidden(), self.hidden(), self.hidden());
        self.ops(&[Op::Store(hs), Op::Push(0), Op::Store(i), Op::Load(hs), Op::LoadM(8), Op::Store(n)]);
        let iv = match index {
            Some(name) => Some(self.loop_var(name, Ty::Int)?),
            None => None,
        };
        let vv = match value {
            Some(name) => {
                let s = self.loop_var(name, if go { Ty::Int } else { Ty::Str })?;
                if go {
                    self.set_width(s, 32); // a rune
                }
                Some(s)
            }
            None => None,
        };
        let (top, cont, done) = (self.label(), self.label(), self.label());
        self.ops(&[Op::Label(top), Op::Load(i), Op::Load(n), Op::Bin(BinOp::Lt), Op::Jz(done)]);
        if go {
            let d = self.hidden();
            self.ops(&[Op::Load(hs), Op::Load(i)]);
            self.lib("lib.go_decode", 2);
            self.ops(&[Op::Store(d), Op::Load(d), Op::Push(32), Op::Bin(BinOp::Shr), Op::Store(step)]);
            if let Some(s) = vv {
                self.ops(&[Op::Load(d), Op::Push(0xFFFF_FFFF), Op::Bin(BinOp::Band)]);
                self.store(s);
            }
        } else {
            self.ops(&[Op::Push(1), Op::Store(step)]);
            if let Some(s) = vv {
                self.ops(&[Op::Load(hs), Op::Push(8), Op::Bin(BinOp::Add), Op::Load(i), Op::Bin(BinOp::Add), Op::LoadM(1), Op::Push(255), Op::Bin(BinOp::Band)]);
                self.lib("lib.str_char", 1);
                self.store(s);
            }
        }
        if let Some(s) = iv {
            self.emit(Op::Load(i));
            self.store(s);
        }
        self.loops.push((cont, done));
        self.block(body)?;
        self.loops.pop();
        self.ops(&[Op::Label(cont), Op::Load(i), Op::Load(step), Op::Bin(BinOp::Add), Op::Store(i), Op::Jmp(top), Op::Label(done)]);
        Ok(())
    }

    /// An operator on strings (both values are on the stack): + joins,
    /// Python's * repeats, the comparisons go byte by byte, `in` finds.
    fn str_op(&mut self, op: BinOp, ta: &Ty, tb: &Ty) -> Result<Ty, E> {
        let ascii = self.sh.l.strings == Strings::Ascii;
        let swap = |fx: &mut Self| {
            let (x, y) = (fx.hidden(), fx.hidden());
            fx.ops(&[Op::Store(y), Op::Store(x), Op::Load(y), Op::Load(x)]);
        };
        self.last_w = 64;
        match (op, ta, tb) {
            (BinOp::Add, Ty::Str, Ty::Str) => {
                self.lib("lib.str_concat", 2);
                Ok(Ty::Str)
            }
            (BinOp::Mul, Ty::Str, t) if ascii && t.scalar() && (*t != Ty::Bool || !self.sh.bools) => {
                self.lib("lib.str_repeat", 2);
                Ok(Ty::Str)
            }
            (BinOp::Mul, t, Ty::Str) if ascii && t.scalar() && (*t != Ty::Bool || !self.sh.bools) => {
                swap(self);
                self.lib("lib.str_repeat", 2);
                Ok(Ty::Str)
            }
            (op, Ty::Str, Ty::Str) if op.is_comparison() => {
                match op {
                    BinOp::Eq => self.lib("lib.str_eq", 2),
                    BinOp::Ne => {
                        self.lib("lib.str_eq", 2);
                        self.emit(Op::Un(UnOp::Not));
                    }
                    BinOp::In => {
                        swap(self);
                        self.lib("lib.str_find", 2);
                        self.ops(&[Op::Push(0), Op::Bin(BinOp::Ge)]);
                    }
                    op => {
                        self.lib("lib.str_cmp", 2);
                        self.ops(&[Op::Push(0), Op::Bin(op)]);
                    }
                }
                self.last_w = 8;
                Ok(self.bool_ty())
            }
            _ => self.err(format!("this operator on {} and {} is not in Strike yet", ta.describe(), tb.describe())),
        }
    }

    /// s.name(args) of Python's string on top.
    fn str_method(&mut self, what: &str, name: &str, args: &[Expr]) -> Result<Ty, E> {
        self.last_w = 64;
        let t = match (what, args) {
            ("str_upper" | "str_lower" | "str_strip", []) => {
                self.lib(&format!("lib.{what}"), 1);
                Ty::Str
            }
            ("str_split", []) => {
                self.lib("lib.str_split", 1);
                Ty::List(Box::new(Ty::Str))
            }
            ("str_join", [x]) => {
                let t = self.expr(x, true)?;
                if !matches!(&t, Ty::List(e) if matches!(**e, Ty::Str | Ty::Unknown)) {
                    return self.err(format!(".join() is given {}, not a list of strings", t.describe()));
                }
                self.lib("lib.str_join", 2);
                Ty::Str
            }
            ("str_startswith", [x]) => {
                self.str_expr(x, ".startswith()")?;
                self.lib("lib.str_startswith", 2);
                self.last_w = 8;
                self.bool_ty()
            }
            ("str_find", [x]) => {
                self.str_expr(x, ".find()")?;
                self.lib("lib.str_find", 2);
                Ty::Int
            }
            _ => return self.err(format!(".{name}() with these values is not in Strike yet")),
        };
        Ok(t)
    }

    /// An operator on lists (both values are on the stack): Python's +
    /// and *.
    fn list_op(&mut self, op: BinOp, ta: &Ty, tb: &Ty) -> Result<Ty, E> {
        if *ta == Ty::Str || *tb == Ty::Str {
            return self.str_op(op, ta, tb);
        }
        self.last_w = 64;
        match (op, ta, tb) {
            (BinOp::Add, Ty::List(_), Ty::List(_)) => {
                let Some(t) = unify(ta, tb) else { return self.err("+ of lists of two kinds: Strike keeps one kind per list") };
                self.lib("lib.list_concat", 2);
                Ok(t)
            }
            (BinOp::Mul, Ty::List(_), t) if t.scalar() => {
                self.lib("lib.list_repeat", 2);
                Ok(ta.clone())
            }
            (BinOp::Mul, t, Ty::List(_)) if t.scalar() => {
                // n * list: the same list, the operands the other way round
                let (n, l) = (self.hidden(), self.hidden());
                self.ops(&[Op::Store(l), Op::Store(n), Op::Load(l), Op::Load(n)]);
                self.lib("lib.list_repeat", 2);
                Ok(tb.clone())
            }
            _ => self.err(format!("this operator on {} and {} is not in Strike yet", ta.describe(), tb.describe())),
        }
    }

    /// a[lo:hi]: Go makes a slice of the same elements, Python a new list.
    fn slice(&mut self, base: &Expr, lo: Option<&Expr>, hi: Option<&Expr>) -> Result<Ty, E> {
        let t = self.expr(base, true)?;
        let hs = self.hidden();
        self.emit(Op::Store(hs));
        if t == Ty::Str {
            return self.str_slice(hs, lo, hi);
        }
        if let Ty::List(_) = &t {
            self.emit(Op::Load(hs));
            for x in [lo, hi] {
                match x {
                    Some(x) => self.int_expr(x)?,
                    None => self.emit(Op::Push(i64::MIN)),
                }
            }
            self.lib("lib.list_slice", 3);
            self.last_w = 64;
            return Ok(t);
        }
        let (e, w) = match &t {
            Ty::Slice(e, w) | Ty::Arr(e, w, _) => ((**e).clone(), *w),
            _ => return self.err(format!("{} is not sliced in Strike yet", t.describe())),
        };
        let (l, h, cap, data) = (self.hidden(), self.hidden(), self.hidden(), self.hidden());
        match &t {
            Ty::Arr(_, _, n) => self.ops(&[Op::Push(*n as i64), Op::Store(cap), Op::Load(hs), Op::Store(data)]),
            _ => self.ops(&[Op::Load(hs), Op::Push(16), Op::Bin(BinOp::Add), Op::LoadM(8), Op::Store(cap), Op::Load(hs), Op::LoadM(8), Op::Store(data)]),
        }
        match lo {
            Some(x) => self.int_expr(x)?,
            None => self.emit(Op::Push(0)),
        }
        self.emit(Op::Store(l));
        match (hi, &t) {
            (Some(x), _) => self.int_expr(x)?,
            (None, Ty::Arr(_, _, n)) => self.emit(Op::Push(*n as i64)),
            (None, _) => self.ops(&[Op::Load(hs), Op::Push(8), Op::Bin(BinOp::Add), Op::LoadM(8)]),
        }
        self.emit(Op::Store(h));
        // 0 <= lo <= hi <= cap, read unsigned
        let code = self.go_fail();
        self.ops(&[Op::Load(h), Op::Load(cap), Op::Bin(BinOp::Ule)]);
        self.fail_unless(code, "panic: runtime error: slice bounds out of range\n");
        self.ops(&[Op::Load(l), Op::Load(h), Op::Bin(BinOp::Ule)]);
        self.fail_unless(code, "panic: runtime error: slice bounds out of range\n");
        let stride = size(&e, w) as i64;
        self.ops(&[Op::Load(data), Op::Load(l), Op::Push(stride), Op::Bin(BinOp::Mul), Op::Bin(BinOp::Add)]);
        self.ops(&[Op::Load(h), Op::Load(l), Op::Bin(BinOp::Sub), Op::Load(cap), Op::Load(l), Op::Bin(BinOp::Sub)]);
        self.lib("lib.header", 3);
        self.last_w = 64;
        Ok(Ty::Slice(Box::new(e), w))
    }

    /// x.name(args): Python's list methods.
    fn method(&mut self, base: &Expr, name: &str, args: &[Expr]) -> Result<Ty, E> {
        // Go's package.name(...): a package is not a variable
        if let Expr::Var(pkg, _) = base {
            if !self.sh.l.qualified.is_empty() && self.lookup(pkg).is_none() {
                let key = format!("{pkg}.{name}");
                let Some(what) = self.sh.l.qualified.get(&key).cloned() else {
                    return self.err(format!("{key} is not in Strike yet"));
                };
                return self.builtin(&what, &key, args);
            }
        }
        let Some(what) = self.sh.l.methods.get(name).cloned() else {
            return self.err(format!(".{name}() is not in Strike yet"));
        };
        let t = self.expr(base, true)?;
        if what.starts_with("str_") {
            if t != Ty::Str {
                return self.err(format!(".{name}() is a string's, not {}'s", t.describe()));
            }
            return self.str_method(&what, name, args);
        }
        let Ty::List(e) = &t else { return self.err(format!(".{name}() is a list's, not {}'s", t.describe())) };
        let e = (**e).clone();
        let r = match (what.as_str(), args) {
            ("list_append", [x]) => {
                let vt = self.value(x)?;
                self.elem_kind(base, &e, &vt)?;
                self.lib("lib.list_append", 2);
                Ty::Int
            }
            ("list_pop", []) => {
                self.lib("lib.list_pop", 1);
                e
            }
            _ => return self.err(format!(".{name}() with these values is not in Strike yet")),
        };
        self.last_w = 64;
        Ok(r)
    }

    /// [elem for name in iter if cond]: a new list; the name is the
    /// comprehension's own.
    fn comprehension(&mut self, elem: &Expr, name: &str, iter: &Expr, cond: Option<&Expr>) -> Result<Ty, E> {
        let r = self.hidden();
        self.emit(Op::Push(0));
        self.lib("lib.list_new", 1);
        self.emit(Op::Store(r));
        let var = self.hidden();
        let (top, cont, done) = (self.label(), self.label(), self.label());
        let mut own = HashMap::new();
        let (counter, end, by) = (self.hidden(), self.hidden(), self.hidden());
        let (hs, i) = (self.hidden(), self.hidden());
        let range = matches!(iter, Expr::Call(n, _, _) if n == "range" && !self.sh.sigs.contains_key("range"));
        if range {
            let Expr::Call(_, args, _) = iter else { unreachable!() };
            let fixed = self.range_start(args, counter, end, by)?;
            own.insert(name.to_string(), (var, Ty::Int));
            self.emit(Op::Label(top));
            self.range_test(fixed, counter, end, by);
            self.emit(Op::Jz(done));
            self.ops(&[Op::Load(counter), Op::Store(var)]);
        } else {
            let t = self.expr(iter, true)?;
            let e = match &t {
                Ty::List(e) => (**e).clone(),
                Ty::Str => Ty::Str,
                _ => return self.err(format!("a comprehension goes over a list, a string or a range, not {}", t.describe())),
            };
            own.insert(name.to_string(), (var, e));
            self.ops(&[Op::Store(hs), Op::Push(0), Op::Store(i), Op::Label(top), Op::Load(i), Op::Load(hs), Op::LoadM(8), Op::Bin(BinOp::Lt), Op::Jz(done)]);
            if t == Ty::Str {
                // a string's characters, each a string of one
                self.ops(&[Op::Load(hs), Op::Push(8), Op::Bin(BinOp::Add), Op::Load(i), Op::Bin(BinOp::Add), Op::LoadM(1), Op::Push(255), Op::Bin(BinOp::Band)]);
                self.lib("lib.str_char", 1);
                self.emit(Op::Store(var));
            } else {
                self.ops(&[Op::Load(hs), Op::Push(16), Op::Bin(BinOp::Add), Op::LoadM(8), Op::Load(i), Op::Push(3), Op::Bin(BinOp::Shl), Op::Bin(BinOp::Add), Op::LoadM(8), Op::Store(var)]);
            }
        }
        // the comprehension's name is seen only inside it, even where a
        // module or `global` name is the same
        self.scopes.push(own);
        let shadow = std::mem::take(&mut self.global_names);
        let at_top = std::mem::replace(&mut self.globals_here, false);
        let result = (|| -> Result<Ty, E> {
            if let Some(c) = cond {
                self.cond(c)?;
                self.emit(Op::Jz(cont));
            }
            self.emit(Op::Load(r));
            let et = self.value(elem)?;
            self.lib("lib.list_append", 2);
            self.emit(Op::Pop);
            Ok(et)
        })();
        self.globals_here = at_top;
        self.global_names = shadow;
        self.scopes.pop();
        let et = result?;
        self.emit(Op::Label(cont));
        if range {
            self.ops(&[Op::Load(counter), Op::Load(by), Op::Bin(BinOp::Add), Op::Store(counter)]);
        } else {
            self.ops(&[Op::Load(i), Op::Push(1), Op::Bin(BinOp::Add), Op::Store(i)]);
        }
        self.ops(&[Op::Jmp(top), Op::Label(done), Op::Load(r)]);
        self.last_w = 64;
        Ok(Ty::List(Box::new(et)))
    }
}

fn combine(what: &str, tys: &[Ty], bools: bool) -> Result<Ty, String> {
    if tys.iter().any(|t| !t.scalar()) {
        let mut u = Ty::Unknown;
        for t in tys {
            u = unify(&u, t).ok_or(format!("{what} is {} in one place and {} in another", u.describe(), t.describe()))?;
        }
        return Ok(u);
    }
    let has = |t: &Ty| tys.contains(t);
    Ok(match (has(&Ty::Int), has(&Ty::Bool)) {
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
        let mut seen = HashSet::new();
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
    let bools = l.bool_print.is_some();
    let typed = !l.int_widths.is_empty();
    let mut fw: HashMap<String, (Vec<u8>, u8)> = HashMap::new();
    // the kinds a typed language declares that are not numbers: fixed
    let mut declared: HashMap<String, (Vec<Option<Ty>>, Option<Ty>)> = HashMap::new();
    for f in &p.funcs {
        let mut ps = Vec::new();
        let mut kinds = Vec::new();
        for t in &f.param_types {
            let (k, w) = type_of(l, t, bools).map_err(|m| (f.line, m))?;
            ps.push(w);
            kinds.push((typed && !k.scalar()).then_some(k));
        }
        let (rk, rw) = type_of(l, &f.ret, bools).map_err(|m| (f.line, m))?;
        if matches!(rk, Ty::Ptr(..)) {
            return Err((f.line, "a function that returns a pointer is not in Strike yet".into()));
        }
        fw.insert(f.name.clone(), (ps, rw));
        declared.insert(f.name.clone(), (kinds, (typed && !rk.scalar()).then_some(rk)));
    }
    let mut sigs: HashMap<String, Sig> = p
        .funcs
        .iter()
        .map(|f| {
            // a language that writes no types learns them: not known at first
            let (kinds, ret) = &declared[&f.name];
            let first = if typed { Ty::Int } else { Ty::Unknown };
            let params = kinds.iter().map(|k| k.clone().unwrap_or(first.clone())).collect();
            (f.name.clone(), Sig { params, ret: ret.clone().unwrap_or(first.clone()) })
        })
        .collect();
    let mut known: HashMap<(String, String), Ty> = HashMap::new();
    for _round in 0..10 {
        let mut sh = Shared {
            l,
            bools,
            sigs: &sigs,
            fw: &fw,
            args: HashMap::new(),
            used: Vec::new(),
            globals: HashMap::new(),
            gwidths: HashMap::new(),
            nglobals: 0,
            printers: BTreeMap::new(),
            basic: HashSet::new(),
            known: &known,
            seen: HashMap::new(),
            nil_slice: None,
        };
        let mut funcs = Vec::new();
        let mut errors: Vec<E> = Vec::new();
        let mut rets: HashMap<String, Vec<(Ty, u32)>> = HashMap::new();
        // the top level first: the globals it declares are seen by every
        // function
        let mut bodies: Vec<(String, u32, Vec<(String, Ty)>, &[Stmt], bool)> = Vec::new();
        if l.toplevel_entry || has_init {
            bodies.push((String::new(), 1, Vec::new(), p.top.as_slice(), true));
        }
        bodies.extend(p.funcs.iter().map(|f| {
            let ps = f.params.iter().cloned().zip(sigs[&f.name].params.iter().cloned()).collect();
            (f.name.clone(), f.line, ps, f.body.as_slice(), false)
        }));
        for (name, line, params, body, top) in bodies {
            let (pw, ret_w) = fw.get(&name).cloned().unwrap_or_default();
            let ret_ty = declared.get(&name).and_then(|d| d.1.clone());
            let mut fx = Fx {
                sh: &mut sh,
                fname: name.clone(),
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
                ret_ty,
                globals_here: top,
                global_names: Default::default(),
                assigned: if top { Default::default() } else { assigned_names(body) },
                frame: 0,
                sliced: sliced_names(body),
            };
            let mut arrays = Vec::new();
            for (i, (n, t)) in params.iter().enumerate() {
                let s = fx.declare(n, t.clone());
                fx.set_width(s, pw.get(i).copied().unwrap_or(64));
                if let Ty::Arr(..) = t {
                    arrays.push((s, t.clone(), n.clone()));
                }
            }
            let done = (|| -> Result<(), E> {
                // a Go array parameter is the function's own copy
                for (s, t, n) in arrays {
                    let own = !fx.sliced.contains(&n);
                    let src = fx.hidden();
                    fx.ops(&[Op::Load(s), Op::Store(src)]);
                    fx.memory_for(&t, own)?;
                    fx.emit(Op::Store(s));
                    fx.ops(&[Op::Load(s), Op::Load(src), Op::Push(t.bytes() as i64)]);
                    fx.lib("lib.copy", 3);
                    fx.emit(Op::Pop);
                }
                fx.push_scope();
                for s in body {
                    fx.stmt(s)?;
                }
                Ok(())
            })();
            // an error may come from a kind not learned yet: it counts only
            // once the kinds have settled
            if let Err(e) = done {
                errors.push(e);
                continue;
            }
            // what each list variable came to, for the next round
            let vars: Vec<(String, (u32, Ty))> = fx.scopes.iter().flat_map(|s| s.iter().map(|(k, v)| (k.clone(), v.clone()))).collect();
            for (n, (slot, t)) in vars {
                if !t.scalar() {
                    let k = fx.key(&n, slot);
                    fx.saw(k, t);
                }
            }
            fx.pop_scope();
            if top && has_init {
                fx.emit(Op::Call(symbol("main"), 0));
            } else {
                fx.emit(Op::Push(0));
            }
            fx.emit(Op::Ret);
            let mut f = Func::new(
                match (top, has_init) {
                    (true, true) => INIT.to_string(),
                    (true, false) => TOP.to_string(),
                    _ => symbol(&name),
                },
                params.len() as u32,
                fx.slots,
                fx.code,
            );
            f.frame = fx.frame;
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
        let lists: Vec<(String, Ty)> = sh.globals.iter().filter(|(_, (_, t))| !t.scalar()).map(|(n, (_, t))| (n.clone(), t.clone())).collect();
        for (n, t) in lists {
            let k = (String::new(), n);
            let merged = match sh.seen.get(&k) {
                Some(old) => unify(old, &t).unwrap_or(t),
                None => t,
            };
            sh.seen.insert(k, merged);
        }
        let globals = sh.nglobals;
        let mut next = sigs.clone();
        for (name, rs) in &rets {
            if declared[name].1.is_some() {
                continue;
            }
            let tys: Vec<Ty> = rs.iter().map(|r| r.0.clone()).collect();
            let line = rs.first().map(|r| r.1).unwrap_or(1);
            next.get_mut(name).unwrap().ret = combine(&format!("what {name} returns"), &tys, bools).map_err(|m| (line, m))?;
        }
        for (callee, per) in &sh.args {
            let line = table[callee.as_str()].line;
            for (i, tys) in per.iter().enumerate() {
                if declared[callee].0[i].is_some() {
                    continue;
                }
                let what = format!("parameter {} of {callee}", table[callee.as_str()].params[i]);
                next.get_mut(callee).unwrap().params[i] = combine(&what, tys, bools).map_err(|m| (line, m))?;
            }
        }
        // a parameter whose kind the function itself learns (a list it
        // appends to) tells its callers
        for (k, t) in &sh.seen {
            if let Some(f) = table.get(&k.0) {
                if let Some(i) = f.params.iter().position(|p| *p == k.1) {
                    if declared[&k.0].0[i].is_none() {
                        let s = next.get_mut(&k.0).unwrap();
                        if let Some(u) = unify(&s.params[i], t) {
                            s.params[i] = u;
                        }
                    }
                }
            }
        }
        let settled = next == sigs && sh.seen == known;
        if !errors.is_empty() && (settled || _round == 9) {
            return Err(errors.swap_remove(0));
        }
        if settled {
            funcs.extend(super::lib::make(&sh.printers, &sh.basic));
            return Ok(Lowered { funcs, entry, globals });
        }
        let seen = std::mem::take(&mut sh.seen);
        drop(sh);
        sigs = next;
        known = seen;
    }
    Err((1, "which values are truth values did not settle in ten rounds".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_and_go_types_are_read_as_written() {
        let l = super::super::languages().unwrap();
        let c = l.iter().find(|x| x.name == "C").unwrap();
        let go = l.iter().find(|x| x.name == "Go").unwrap();
        let w = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert_eq!(type_of(c, &w("int [ ]"), false).unwrap().0, Ty::Ptr(Box::new(Ty::Int), 32));
        assert_eq!(type_of(c, &w("long long *"), false).unwrap().0, Ty::Ptr(Box::new(Ty::Int), 64));
        assert_eq!(type_of(c, &w("int [ ] [ 10 ]"), false).unwrap().0, Ty::Ptr(Box::new(Ty::Arr(Box::new(Ty::Int), 32, 10)), 64));
        assert_eq!(type_of(go, &w("[ ] int"), true).unwrap().0, Ty::Slice(Box::new(Ty::Int), 64));
        assert_eq!(type_of(go, &w("[ 3 ] [ ] int32"), true).unwrap().0, Ty::Arr(Box::new(Ty::Slice(Box::new(Ty::Int), 32)), 64, 3));
        assert_eq!(type_of(go, &w("[ ] bool"), true).unwrap().0, Ty::Slice(Box::new(Ty::Bool), 8));
    }
}
