//! The universal syntax to Strike IR. Values are 64-bit integers; a truth
//! value is an integer too, but where the language prints it as a word
//! (Python's True, Go's true) the lowering follows which values are truth
//! values, through variables, parameters and results, until nothing changes.
//! A list, a slice or an array is the address of its memory (lib.rs says
//! how each is laid out), and its kind is followed the same way; so is a
//! string (strs.rs). A float is its 64 bits (a float32 is held as the
//! double of the same value), and each language's conversions between
//! numbers and floats are made where it makes them: C's usual arithmetic
//! conversions, Go's constants taking the type they meet (and nothing
//! else), Python's whole numbers made floats in arithmetic but compared
//! exactly.
//! Whatever Strike cannot yet do the way the language does is an error,
//! never a guess.

use super::ir::{BinOp, Func, Op, UnOp};
use super::lang::{FloatPrint, Lang, Strings, Style};
use super::lex::FloatLit;
use super::lib::{Elem, FloatElem, Printer, MAX_LEN};
use super::parse::{Expr, FuncDef, Program, Stmt, TypeDef};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub enum Ty {
    Int,
    Bool,
    /// a double, or a float32 where its width is 32
    Float,
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
    /// a C or Go struct, by its name: the address of its fields, held in
    /// place as an array is (Layout says where each field is)
    Struct(String),
    /// a Python object of the class: the address of its attributes, shared
    /// by every name that holds it
    Obj(String),
}

impl Ty {
    /// The bytes the value takes where it is kept: an array its elements,
    /// anything else 8.
    pub fn bytes(&self) -> u64 {
        match self {
            Ty::Arr(e, w, n) => n * size(e, *w),
            Ty::Struct(s) => layout(s).size,
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
        matches!(self, Ty::Int | Ty::Bool | Ty::Float | Ty::Unknown)
    }
    /// A whole number (or a truth value, which is one)?
    fn whole(&self) -> bool {
        matches!(self, Ty::Int | Ty::Bool | Ty::Unknown)
    }
    /// Is the zero of this kind not all zero bits? (A Go slice's zero is
    /// the address of the empty header.)
    fn zero_is_not_zero(&self) -> bool {
        match self {
            Ty::Slice(..) | Ty::Str => true,
            Ty::Arr(e, _, _) => e.zero_is_not_zero(),
            Ty::Struct(s) => layout(s).fields.iter().any(|f| f.ty.zero_is_not_zero()),
            _ => false,
        }
    }
    /// Held where it is kept (an array, a struct): its value is its address,
    /// and giving it a new home copies its bytes.
    fn in_place(&self) -> bool {
        matches!(self, Ty::Arr(..) | Ty::Struct(_))
    }
    fn describe(&self) -> String {
        match self {
            Ty::Int => "a number".into(),
            Ty::Bool => "a truth value".into(),
            Ty::Float => "a float".into(),
            Ty::Arr(..) => "an array".into(),
            Ty::Ptr(..) => "a pointer".into(),
            Ty::Slice(..) => "a slice".into(),
            Ty::List(..) => "a list".into(),
            Ty::Unknown => "an element not yet known".into(),
            Ty::Str => "a string".into(),
            Ty::Struct(s) => format!("a struct {s}"),
            Ty::Obj(c) => format!("an object of class {c}"),
        }
    }
}

/// A struct as C lays it out (clang on x86-64), and as Strike keeps a Go
/// struct (Go does not show its layout; a float32 is held as a double).
#[derive(Debug)]
pub struct Layout {
    pub size: u64,
    pub align: u64,
    pub fields: Vec<FieldL>,
    /// Go's own size of it and whether it holds no pointers: what its
    /// slices' capacities grow by
    pub go: (i64, bool),
}

#[derive(Debug, Clone)]
pub struct FieldL {
    pub name: String,
    pub ty: Ty,
    pub w: u8,
    pub off: u64,
}

/// The program's structs: the struct each written type names ("struct P",
/// C's tag, or "P"), and each one's layout.
#[derive(Default)]
struct Types {
    names: HashMap<String, String>,
    layouts: HashMap<String, std::rc::Rc<Layout>>,
}

thread_local! {
    /// The structs of the program being lowered: a Ty names its struct, and
    /// its size and fields are found here.
    static TYPES: std::cell::RefCell<std::rc::Rc<Types>> = std::cell::RefCell::new(std::rc::Rc::new(Types::default()));
}

fn types() -> std::rc::Rc<Types> {
    TYPES.with(|t| t.borrow().clone())
}

fn layout(name: &str) -> std::rc::Rc<Layout> {
    types().layouts.get(name).cloned().unwrap_or_else(|| panic!("the struct {name} is laid out before it is used"))
}

/// The structs of the lowering that was going on before this one (the float
/// library is lowered inside another program's lowering), given back when
/// this one ends.
struct Installed(std::rc::Rc<Types>);

impl Drop for Installed {
    fn drop(&mut self) {
        let before = self.0.clone();
        TYPES.with(|t| *t.borrow_mut() = before);
    }
}

fn install(t: Types) -> Installed {
    Installed(TYPES.with(|c| std::mem::replace(&mut *c.borrow_mut(), std::rc::Rc::new(t))))
}

/// Size and alignment of a value kept in memory: C's for a C struct (a
/// field's own width), Strike's own for a Go struct.
fn size_align(t: &Ty, w: u8, done: &HashMap<String, std::rc::Rc<Layout>>) -> (u64, u64) {
    match t {
        Ty::Arr(e, ew, n) => {
            let (s, a) = size_align(e, *ew, done);
            (s * n, a)
        }
        Ty::Struct(s) => {
            let l = &done[s];
            (l.size, l.align)
        }
        _ => {
            let s = size(t, w);
            (s, s)
        }
    }
}

/// Go's own size, alignment, and whether it holds no pointers.
fn go_layout(t: &Ty, w: u8, done: &HashMap<String, std::rc::Rc<Layout>>) -> (i64, i64, bool) {
    match t {
        Ty::Struct(s) => {
            let l = &done[s];
            let align = l.fields.iter().map(|f| go_layout(&f.ty, f.w, done).1).max().unwrap_or(1);
            (l.go.0, align, l.go.1)
        }
        Ty::Arr(e, ew, n) => {
            let (s, a, ns) = go_layout(e, *ew, done);
            (s * *n as i64, a, ns)
        }
        Ty::Str => (16, 8, false),
        Ty::Slice(..) => (24, 8, false),
        Ty::Ptr(..) => (8, 8, false),
        _ => {
            let (s, ns) = go_size(t, w);
            (s, s, ns)
        }
    }
}

/// Every struct of the program laid out: its fields' types read as the
/// language writes them, the fields placed in order, each at the next
/// multiple of its alignment, the whole rounded up to the largest.
fn lay_out(p: &Program, l: &Lang) -> Result<Types, E> {
    let mut t = Types::default();
    let Some(rec) = &l.records else { return Ok(t) };
    if rec.classes {
        return Ok(t);
    }
    let c = l.strings == Strings::C;
    let mut defs: HashMap<String, &TypeDef> = HashMap::new();
    for d in &p.types {
        let canon = d.tag.clone().or(d.name.clone()).unwrap_or_default();
        if defs.insert(canon.clone(), d).is_some() {
            return Err((d.line, format!("the struct {canon} is defined twice")));
        }
        if let Some(g) = &d.tag {
            t.names.insert(format!("struct {g}"), canon.clone());
        }
        if let Some(n) = &d.name {
            if t.names.insert(n.clone(), canon.clone()).is_some() {
                return Err((d.line, format!("{n} names two structs")));
            }
        }
    }
    for (a, b, line) in &p.aliases {
        let Some(s) = t.names.get(&format!("struct {a}")).cloned() else {
            return Err((*line, format!("struct {a} is not defined here; a typedef of a struct defined later is not in Strike yet")));
        };
        if t.names.insert(b.clone(), s).is_some() {
            return Err((*line, format!("{b} names two types")));
        }
    }
    // the field types: what each names, read with the struct names known
    let mut fields: HashMap<String, Vec<(String, Ty, u8, u32)>> = HashMap::new();
    let names_only = install(Types { names: t.names.clone(), layouts: HashMap::new() });
    for (canon, d) in &defs {
        if d.fields.is_empty() {
            return Err((d.line, format!("a struct without fields ({canon}) is not in Strike yet")));
        }
        let mut seen = HashSet::new();
        let mut v = Vec::new();
        for f in &d.fields {
            if !seen.insert(f.name.clone()) {
                return Err((f.line, format!("{canon} has two fields named {}", f.name)));
            }
            let (mut ty, mut w) = type_of(l, &f.ty, l.bool_print.is_some()).map_err(|m| (f.line, m))?;
            for d in f.dims.iter().rev() {
                let n = match d.as_ref().and_then(constant) {
                    Some(n) if n > 0 => n as u64,
                    _ => return Err((f.line, "a field's array size is a constant of at least 1 in Strike".into())),
                };
                ty = Ty::Arr(Box::new(ty), w, n);
                w = 64;
            }
            let (mut inner, mut iw) = (&ty, w);
            while let Ty::Arr(e, ew, _) = inner {
                (inner, iw) = (e, *ew);
            }
            if c && *inner == Ty::Float && iw == 32 {
                return Err((f.line, "a float field is not in Strike yet: Strike holds a float as a double, so its 4 bytes are not where clang puts them".into()));
            }
            if iw == U8 {
                return Err((f.line, "a byte field is not in Strike yet".into()));
            }
            if matches!(inner, Ty::Ptr(e, _) if !matches!(**e, Ty::Struct(_))) && !c {
                return Err((f.line, "a pointer field to anything but a struct is not in Strike yet".into()));
            }
            v.push((f.name.clone(), ty, w, f.line));
        }
        fields.insert(canon.clone(), v);
    }
    drop(names_only);
    // laid out in the order they need each other: a struct after the
    // structs it holds (not those it only points at)
    let mut done: HashMap<String, std::rc::Rc<Layout>> = HashMap::new();
    fn holds(t: &Ty) -> Option<&str> {
        match t {
            Ty::Struct(s) => Some(s),
            Ty::Arr(e, _, _) => holds(e),
            _ => None,
        }
    }
    let mut order: Vec<String> = fields.keys().cloned().collect();
    order.sort();
    while done.len() < fields.len() {
        let ready = order.iter().find(|n| !done.contains_key(*n) && fields[*n].iter().all(|f| holds(&f.1).is_none_or(|h| done.contains_key(h)))).cloned();
        let Some(n) = ready else {
            let n = order.iter().find(|n| !done.contains_key(*n)).unwrap();
            return Err((defs[n].line, format!("the struct {n} holds itself (only a pointer to it can be its field)")));
        };
        let mut off = 0u64;
        let mut align = 1u64;
        let mut ls = Vec::new();
        let (mut go_off, mut go_align, mut noscan) = (0i64, 1i64, true);
        for (name, ty, w, _) in &fields[&n] {
            let (s, a) = size_align(ty, *w, &done);
            off = off.div_ceil(a) * a;
            ls.push(FieldL { name: name.clone(), ty: ty.clone(), w: *w, off });
            off += s;
            align = align.max(a);
            let (gs, ga, gn) = go_layout(ty, *w, &done);
            go_off = (go_off + ga - 1) / ga * ga + gs;
            go_align = go_align.max(ga);
            noscan &= gn;
        }
        let size = off.div_ceil(align) * align;
        let go = ((go_off + go_align - 1) / go_align * go_align, noscan);
        done.insert(n.clone(), std::rc::Rc::new(Layout { size, align, fields: ls, go }));
    }
    t.layouts = done;
    Ok(t)
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
    // a float32 is held as a double
    if *e == Ty::Float {
        return 8;
    }
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
        Ty::Arr(..) | Ty::Struct(_) => None,
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
/// the words of the float buffer: the longest a float prints (a %f of
/// 1e308 with a precision of 100) fits
const FLOAT_BUF_WORDS: u32 = 64;
const INEXACT_DIVISION: &str = "strike: a division of whole numbers past 2^53 is not worked out exactly by Strike yet, so the program stops\n";
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
            Expr::Un(_, x) | Expr::Cast(_, x, _) | Expr::Field(x, _, _) | Expr::Addr(x, _) | Expr::Deref(x, _) | Expr::Key(_, x, _) | Expr::SizeOf(x, _) => expr(x, out),
            Expr::Bin(_, a, b) | Expr::Index(a, b, _) => {
                expr(a, out);
                expr(b, out);
            }
            Expr::Comp { elem, iter, cond, .. } => {
                expr(elem, out);
                expr(iter, out);
                cond.iter().for_each(|x| expr(x, out));
            }
            Expr::Num(..) | Expr::Float(_) | Expr::Str(_) | Expr::Var(..) | Expr::Type(..) | Expr::SizeOfType(..) | Expr::OffsetOf(..) => {}
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
                Stmt::Set { target, value, .. } => {
                    expr(target, out);
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
        Stmt::Set { target, .. } if target.root() == Some(name) => true,
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

/// A Go constant: a whole number, or a float (exact or not).
enum K {
    Int(i64),
    Float(FloatLit),
}

/// A Go constant expression worked out as Go works it out: exactly. An
/// operation is taken only on constants whose doubles are the numbers
/// exactly; its one rounding is then Go's (1.0 / 3.0), and its result may be
/// worked on again only where it is exact too. So 0.1 + 0.2, which is 0.3
/// in Go and not the sum of two doubles, is refused. None where the
/// expression is not a constant (or is only whole numbers, which the
/// lowering takes as before).
fn go_const(e: &Expr) -> Option<Result<K, String>> {
    Some(Ok(match e {
        Expr::Num(v, false) => K::Int(*v),
        Expr::Float(f) => K::Float(*f),
        Expr::Un(UnOp::Plus, x) => return go_const(x),
        Expr::Un(UnOp::Neg, x) => match go_const(x)? {
            Ok(K::Int(v)) => K::Int(v.checked_neg()?),
            Ok(K::Float(f)) if f64::from_bits(f.bits) == 0.0 => K::Float(f),
            Ok(K::Float(f)) => K::Float(FloatLit { bits: f.bits ^ (1 << 63), bits32: f.bits32 ^ (1 << 63), ..f }),
            Err(m) => return Some(Err(m)),
        },
        Expr::Bin(op, a, b) => {
            let (a, b) = match (go_const(a)?, go_const(b)?) {
                (Ok(a), Ok(b)) => (a, b),
                (Err(m), _) | (_, Err(m)) => return Some(Err(m)),
            };
            let exact = |k: &K| match k {
                K::Int(v) => (v.unsigned_abs() <= 1 << 53, *v as f64),
                K::Float(f) => (f.exact, f64::from_bits(f.bits)),
            };
            match (&a, &b) {
                (K::Int(x), K::Int(y)) => return constant(&Expr::Bin(*op, Box::new(Expr::Num(*x, false)), Box::new(Expr::Num(*y, false)))).map(|v| Ok(K::Int(v))),
                _ => {
                    let ((ea, x), (eb, y)) = (exact(&a), exact(&b));
                    let refused = Err("Go works out constant expressions exactly (0.1 + 0.2 is 0.3 there); this one is not exact as doubles, and Strike does not work it out exactly yet".to_string());
                    if !ea || !eb {
                        return Some(refused);
                    }
                    // one operation on two exact numbers rounds once, as Go
                    // rounds the exact result: the double is Go's. Whether
                    // it is exact itself decides whether it may be worked
                    // on again.
                    let tiny = |r: f64| r != 0.0 && r.abs() < f64::MIN_POSITIVE;
                    let (r, exact) = match op {
                        BinOp::Add | BinOp::Sub => {
                            let y = if *op == BinOp::Sub { -y } else { y };
                            // the sum is exact when nothing is lost (two-sum)
                            let s = x + y;
                            let bb = s - x;
                            (s, (x - (s - bb)) + (y - bb) == 0.0)
                        }
                        BinOp::Mul => {
                            let p = x * y;
                            (p, !tiny(p) && (p != 0.0 || x == 0.0 || y == 0.0) && x.mul_add(y, -p) == 0.0)
                        }
                        BinOp::DivTrunc if y == 0.0 => return Some(Err("division by zero".to_string())),
                        BinOp::DivTrunc => {
                            let q = x / y;
                            (q, !tiny(q) && (q != 0.0 || x == 0.0) && q.mul_add(y, -x) == 0.0)
                        }
                        _ => return Some(Err("this operator on Go's float constants is not in Strike yet".to_string())),
                    };
                    if !r.is_finite() {
                        return Some(Err("the constant overflows a float64".to_string()));
                    }
                    let r = if r == 0.0 { 0.0 } else { r };
                    // its float32: rounded from the double, which is the
                    // exact number's rounding unless the double lies just
                    // between two float32s (then the exact number decides)
                    let f = r as f32;
                    if !exact && (f as f64) != r {
                        let other = if (f as f64) < r { f.next_up() } else { f.next_down() };
                        if ((f as f64) + (other as f64)) / 2.0 == r {
                            return Some(refused);
                        }
                    }
                    K::Float(FloatLit { bits: r.to_bits(), bits32: (f as f64).to_bits(), f32: false, exact })
                }
            }
        }
        _ => return None,
    }))
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
    // Go's *P: a pointer to a struct
    if l.param_first && words.first().map(String::as_str) == Some("*") {
        let (inner, _) = type_of(l, &words[1..], bools)?;
        if !matches!(inner, Ty::Struct(_)) {
            return Err("a pointer to anything but a struct is not in Strike yet".into());
        }
        return Ok((Ty::Ptr(Box::new(inner), 64), 64));
    }
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
    // a struct: C's struct P (or its typedef's name), Go's P
    let named: Vec<&str> = base.iter().map(String::as_str).filter(|s| !matches!(*s, "static" | "const" | "register" | "volatile")).collect();
    let key = named.join(" ");
    let ts = types();
    let strukt = match ts.names.get(&key) {
        Some(s) => Some(Ty::Struct(s.clone())),
        None if named.first() == Some(&"struct") => return Err(format!("{key} is not defined here")),
        None if named.len() == 1 && ts.names.contains_key(&format!("struct {key}")) => return Err(format!("{key} is a struct's tag: C writes it struct {key}")),
        None => None,
    };
    if base.len() == 1 && base[0] == "string" && matches!(l.strings, Strings::Bytes | Strings::Ascii) {
        if base_end != words.len() {
            return Err("pointers are not in Strike yet (but arrays are)".into());
        }
        return Ok((Ty::Str, 64));
    }
    let plain: Vec<&String> = base.iter().filter(|s| !matches!(s.as_str(), "static" | "const" | "register" | "volatile")).collect();
    let (scalar, w) = match base.iter().find_map(|b| l.float_widths.get(b).copied()) {
        _ if strukt.is_some() => (strukt.unwrap(), 64),
        Some(_) if plain.len() > 1 => return Err(format!("the type {} is not in Strike yet", plain.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" "))),
        Some(fw) => (Ty::Float, fw),
        None => {
            let w = width_of(l, base)?;
            let is_bool = base.iter().any(|b| b == "bool" || b == "_Bool");
            (if is_bool && bools { Ty::Bool } else { Ty::Int }, w)
        }
    };
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
        return Err("pointers are not in Strike yet (but arrays are, and a pointer to a struct)".into());
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
        Ty::Int | Ty::Unknown | Ty::Float => ((w / 8).max(1) as i64, true),
        Ty::Bool => (1, true),
        // a Go string is two words (its bytes' address and its length)
        Ty::Str => (16, false),
        Ty::Arr(e, ew, n) => {
            let (s, ns) = go_size(e, *ew);
            (s * *n as i64, ns)
        }
        Ty::Struct(s) => layout(s).go,
        Ty::Ptr(..) => (8, false),
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
    /// the global words a float's digits are made in before they print
    float_buf: Option<u32>,
    /// Python's classes, and their definitions by name
    classes: &'a HashMap<String, Class>,
    defs: &'a HashMap<String, TypeDef>,
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
    /// sign-extended (0: a literal that takes the width of what it meets);
    /// for a float, 64 or 32
    last_w: u8,
    /// the value expr just left when it is a Go float constant (its
    /// float32 is rounded from the constant itself)
    last_k: Option<FloatLit>,
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
    /// this function returns a struct: into the caller's memory, whose
    /// address is its first parameter (slot 0)
    sret: bool,
    /// Go's names whose address is taken: their memory is new each time
    escapes: HashSet<String>,
    /// a Python method: the class it is called on and the class it is
    /// written in
    class_ctx: Option<(String, String)>,
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
        if slot & GLOBAL != 0 && ty.in_place() {
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
        if was.scalar() && now.scalar() && (*was == Ty::Float) != (*now == Ty::Float) && *was != Ty::Unknown && *now != Ty::Unknown {
            return self.err(format!("{name} holds {} in one place and {} in another; Strike keeps one kind per variable", was.describe(), now.describe()));
        }
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
    /// The global words a float's digits are made in before they print.
    fn float_buf(&mut self) -> u32 {
        match self.sh.float_buf {
            Some(g) => g,
            None => {
                let g = self.sh.nglobals;
                self.sh.nglobals += FLOAT_BUF_WORDS;
                self.sh.float_buf = Some(g);
                g
            }
        }
    }
    /// Stops the program as the language does on a division by zero
    /// (Python's ZeroDivisionError) where the divisor on top is zero (0.0
    /// and -0.0 for a float); the divisor stays on top. Nothing for a
    /// language that goes on (C, Go: a float's infinity or NaN).
    fn zero_division(&mut self, float: bool) {
        let Some((code, msg)) = self.sh.l.zero_division.clone() else { return };
        if let Some(Op::Push(c)) = self.code.last() {
            let zero = if float { f64::from_bits(*c as u64) == 0.0 } else { *c == 0 };
            if !zero {
                return;
            }
        }
        let h = self.hidden();
        self.ops(&[Op::Store(h), Op::Load(h)]);
        if float {
            self.ops(&[Op::Push(0), Op::Bin(BinOp::FNe)]);
        }
        self.fail_unless(code, &msg);
        self.emit(Op::Load(h));
    }

    /// Is the expression surely a whole number, by its names' kinds (no
    /// float in it)? Read without lowering it; false where not known.
    fn whole_value(&self, e: &Expr) -> bool {
        match e {
            Expr::Num(..) => true,
            Expr::Var(n, _) => self.lookup(n).is_some_and(|(_, t)| t.whole()),
            Expr::Un(_, x) => self.whole_value(x),
            Expr::Bin(op, a, b) => *op != BinOp::DivTrue && self.whole_value(a) && self.whole_value(b),
            Expr::Index(b, i, _) => {
                let mut base = &**b;
                while let Expr::Index(inner, _, _) = base {
                    base = inner;
                }
                let scalar_elems = |t: &Ty| {
                    let mut t = t.clone();
                    while let Some((e, _)) = t.elem() {
                        t = e;
                    }
                    t.whole()
                };
                matches!(base, Expr::Var(n, _) if self.lookup(n).is_some_and(|(_, t)| scalar_elems(&t))) && self.whole_value(i)
            }
            _ => false,
        }
    }

    /// The value whose code ends at `at`, when it is a constant: its last
    /// operation is what makes it, so a Push there is the whole value.
    fn constant_at(&self, at: usize) -> Option<i64> {
        match at.checked_sub(1).and_then(|i| self.code.get(i)) {
            Some(Op::Push(c)) => Some(*c),
            _ => None,
        }
    }
    /// The whole number whose code ends at `at` made a float of width w
    /// (64, or 32 for a float32): a constant at once, else by the machine.
    fn to_float_at(&mut self, at: usize, w: u8) {
        let op = if w == 32 { UnOp::FromInt32 } else { UnOp::FromInt };
        match self.constant_at(at) {
            Some(c) => self.code[at - 1] = Op::Push(op.float_eval(c).unwrap()),
            None => self.code.insert(at, Op::Un(op)),
        }
    }
    /// The float whose code ends at `at` (width fw; k its constant, Go's)
    /// made a float of width w: a double rounded to a float32 where w is 32.
    /// A Go constant is rounded from its own value, not from its double.
    fn float_width_at(&mut self, at: usize, fw: u8, k: Option<FloatLit>, w: u8) {
        if w != 32 || fw == 32 {
            return;
        }
        match (k, self.constant_at(at)) {
            // a constant that rounds to zero is zero, never -0 (Go's
            // constants have no negative zero)
            (Some(k), Some(_)) if fw == 0 => self.code[at - 1] = Op::Push(if f64::from_bits(k.bits32) == 0.0 { 0 } else { k.bits32 as i64 }),
            (_, Some(c)) => self.code[at - 1] = Op::Push(UnOp::ToF32.float_eval(c).unwrap()),
            _ => self.code.insert(at, Op::Un(UnOp::ToF32)),
        }
    }
    /// Go refuses a constant that a float32 cannot hold.
    fn go_fits32(&self, fw: u8, k: Option<FloatLit>, w: u8) -> Result<(), E> {
        match k {
            Some(k) if fw == 0 && w == 32 && f64::from_bits(k.bits32).is_infinite() => self.err(format!("the constant {:?} overflows float32", f64::from_bits(k.bits))),
            _ => Ok(()),
        }
    }
    /// A Go float constant whose code ends at `at`, where a whole number is
    /// wanted: the number, if the constant is one exactly (Go: else it
    /// would be truncated, which is an error).
    fn float_const_as_int(&mut self, at: usize, k: FloatLit) -> Result<(), E> {
        let x = f64::from_bits(k.bits);
        if !k.exact || x.fract() != 0.0 || !(-9.2e18..9.2e18).contains(&x) {
            return self.err(format!("the constant {x:?} is not a whole number (Go: truncated to integer)"));
        }
        self.code[at - 1] = Op::Push(x as i64);
        Ok(())
    }
    /// The float on top cut toward zero to a whole number of w bits, as C's
    /// conversion and Go's int(x) do it (the machine's cvttsd2si: 32 bits
    /// for a width of 32 or less, as clang and Go convert to an int).
    fn float_to_int(&mut self, w: u8) {
        let op = if w <= 32 { UnOp::ToInt32 } else { UnOp::ToInt };
        match self.constant_at(self.code.len()) {
            Some(c) => *self.code.last_mut().unwrap() = Op::Push(op.float_eval(c).unwrap()),
            None => self.emit(Op::Un(op)),
        }
        if w < 32 {
            self.emit(Op::Sext(w));
        }
    }
    /// The value on top, of kind `from` (width fw), made what is kept where
    /// it goes (kind `to`, width tw), as a typed language converts by itself
    /// at an assignment, an argument, a return or an element: a whole number
    /// cut to its width, C's whole numbers and floats into each other, a Go
    /// constant taking the type it meets. An error where the language does
    /// not convert; nothing for a language without types (Python).
    fn convert(&mut self, to: &Ty, tw: u8, from: &Ty, fw: u8) -> Result<(), E> {
        if !self.typed() {
            return Ok(());
        }
        let at = self.code.len();
        let k = self.last_k;
        let go = self.sh.l.untyped_literals;
        match (*to == Ty::Float, *from == Ty::Float) {
            (true, true) => {
                if go && fw != 0 && tw != fw {
                    return self.err(format!("a float{fw} is given where a float{tw} is wanted; Go wants a conversion"));
                }
                self.go_fits32(fw, k, tw)?;
                self.float_width_at(at, fw, k, tw);
            }
            (true, false) => {
                if go && fw != 0 {
                    return self.err("Go does not make a whole number a float by itself; write float64(x)");
                }
                self.to_float_at(at, tw);
            }
            (false, true) if go => match k {
                Some(k) if fw == 0 => {
                    self.float_const_as_int(at, k)?;
                    self.narrow(tw, 0)?;
                }
                _ => return self.err("Go does not make a float a whole number by itself; write int(x)"),
            },
            (false, true) => self.float_to_int(tw),
            (false, false) => self.narrow(tw, fw)?,
        }
        Ok(())
    }
    /// An explicit conversion of the value on top (kind `from`, width fw)
    /// to kind `to` (width tw): C's (double)x, Go's float64(i) and int(f),
    /// Python's float(x) and int(x).
    fn explicit_convert(&mut self, to: &Ty, tw: u8, from: &Ty, fw: u8) -> Result<(), E> {
        if !from.scalar() {
            return self.err(format!("{} is not converted to a number", from.describe()));
        }
        let at = self.code.len();
        let k = self.last_k;
        match (*to == Ty::Float, *from == Ty::Float) {
            (true, true) => {
                self.go_fits32(fw, k, tw)?;
                self.float_width_at(at, fw, k, tw);
            }
            (true, false) => {
                if *from == Ty::Bool && self.sh.l.untyped_literals {
                    return self.err("Go does not convert a bool to a float");
                }
                self.to_float_at(at, tw);
            }
            (false, true) if !self.typed() => self.py_int_of_float(),
            (false, true) => match k {
                Some(k) if fw == 0 && self.sh.l.untyped_literals => {
                    self.float_const_as_int(at, k)?;
                    self.narrow(tw, 0)?;
                }
                _ => self.float_to_int(tw),
            },
            (false, false) => self.narrow(tw, fw)?,
        }
        Ok(())
    }
    /// Python's int() of the float on top: cut toward zero, but NaN, an
    /// infinity and a number past 64 bits stop the program (ValueError,
    /// OverflowError, and Strike's own stop where Python would go on with a
    /// larger number).
    fn py_int_of_float(&mut self) {
        let h = self.hidden();
        self.emit(Op::Store(h));
        self.ops(&[Op::Load(h), Op::Load(h), Op::Bin(BinOp::FEq)]);
        self.fail_unless(1, "ValueError: cannot convert float NaN to integer\n");
        self.ops(&[Op::Load(h), Op::Un(UnOp::FAbs), Op::Push(f64::INFINITY.to_bits() as i64), Op::Bin(BinOp::FNe)]);
        self.fail_unless(1, "OverflowError: cannot convert float infinity to integer\n");
        let (lo, hi) = ((-9223372036854775808.0f64).to_bits() as i64, 9223372036854775808.0f64.to_bits() as i64);
        self.ops(&[Op::Load(h), Op::Push(lo), Op::Bin(BinOp::FGe), Op::Load(h), Op::Push(hi), Op::Bin(BinOp::FLt), Op::Bin(BinOp::Band)]);
        let ok = self.label();
        self.ops(&[Op::Jnz(ok), Op::Fail(1, super::x64::OVERFLOW.to_vec()), Op::Label(ok)]);
        self.ops(&[Op::Load(h), Op::Un(UnOp::ToInt)]);
    }
    /// Prints the float on top: by print's own rule (fmt None: Python's
    /// repr, Go's %v), or in a format ('e', 'f', 'g', 'E', 'G' and its
    /// precision; -1 is Go's shortest). The library makes the digits in
    /// the program's float buffer, and they print from there.
    fn print_float(&mut self, f32: bool, fmt: Option<(u8, i64)>) -> Result<(), E> {
        let v = self.hidden();
        self.emit(Op::Store(v));
        let buf = self.float_buf();
        let (c, p, go) = match (fmt, self.sh.l.float_print) {
            (None, Some(FloatPrint::Repr)) => {
                self.ops(&[Op::AddrG(buf), Op::Load(v), Op::AddrG(buf)]);
                self.lib("lib.float.py_repr", 2);
                self.emit(Op::PrintMem);
                return Ok(());
            }
            (None, Some(FloatPrint::Shortest)) => (b'g', -1, true),
            (None, None) => return self.err(format!("{} prints a float only by a format (printf's %f, %e, %g)", self.sh.l.name)),
            (Some((c, p)), _) => (c, p, self.sh.l.printf_go == Some(true)),
        };
        self.ops(&[Op::AddrG(buf), Op::Load(v), Op::Push(f32 as i64), Op::Push(c as i64), Op::Push(p), Op::Push(go as i64), Op::AddrG(buf)]);
        self.lib("lib.float.fmt_float", 6);
        self.emit(Op::PrintMem);
        Ok(())
    }

    /// A binary operator whose left value (kind ta, width wa, k its Go
    /// constant) is on the stack, its code ending at a_end: the right one
    /// is lowered, both are made one kind as the language makes them, and
    /// the operation is done. `uns`: C's size_t is on a side, so an order
    /// is compared unsigned.
    #[allow(clippy::too_many_arguments)]
    fn binary(&mut self, op: BinOp, ta: Ty, wa: u8, ka: Option<FloatLit>, a_end: usize, b: &Expr, uns: bool) -> Result<Ty, E> {
        let tb = self.expr(b, true)?;
        let (wb, kb) = (self.last_w, self.last_k);
        if !ta.scalar() || !tb.scalar() || op == BinOp::In {
            return self.list_op(op, &ta, &tb);
        }
        if uns && ta != Ty::Float && tb != Ty::Float && matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) {
            // both sides as 64-bit unsigned numbers, as C converts them
            if matches!(op, BinOp::Gt | BinOp::Ge) {
                let (x, y) = (self.hidden(), self.hidden());
                self.ops(&[Op::Store(y), Op::Store(x), Op::Load(y), Op::Load(x)]);
            }
            self.emit(Op::Bin(if matches!(op, BinOp::Lt | BinOp::Gt) { BinOp::Ult } else { BinOp::Ule }));
            self.last_w = 8;
            return Ok(self.bool_ty());
        }
        if ta == Ty::Float || tb == Ty::Float || op == BinOp::DivTrue {
            return self.float_binary(op, (&ta, wa, ka, a_end), (&tb, wb, kb));
        }
        self.int_binary(op, wa, wb)
    }

    /// An operator on two whole numbers (on the stack, widths wa and wb).
    fn int_binary(&mut self, op: BinOp, wa: u8, wb: u8) -> Result<Ty, E> {
        let op = if self.sh.l.checked { op.checked() } else { op };
        if matches!(op, BinOp::DivFloor | BinOp::ModFloor) {
            self.zero_division(false);
        }
        self.emit(Op::Bin(op));
        if op.is_comparison() {
            self.last_w = 8;
            return Ok(self.bool_ty());
        }
        // the operation at its width: a result that can leave it is cut
        // back, as the machine of that width would
        let w = self.arith_width(wa, wb);
        if w > 0 && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Shl | BinOp::DivTrunc) {
            self.narrow(w, 64)?;
        }
        self.last_w = if w == 0 { 0 } else { w };
        Ok(Ty::Int)
    }

    /// A binary operator where a float is met (or Python's /): both sides
    /// made floats of one width, as the language makes them, then the
    /// operation. C makes a whole number a float, and a float32 stays one
    /// only with another or with a whole number; Go mixes nothing but lets a
    /// constant take the type it meets; Python makes a whole number a float
    /// but compares the two exactly. The left side's code ends at a_end, the
    /// right side's at the end.
    fn float_binary(&mut self, op: BinOp, (ta, wa, ka, a_end): (&Ty, u8, Option<FloatLit>, usize), (tb, wb, kb): (&Ty, u8, Option<FloatLit>)) -> Result<Ty, E> {
        let go = self.sh.l.untyped_literals;
        let python = !self.typed();
        let (fa, fb) = (*ta == Ty::Float, *tb == Ty::Float);
        if op == BinOp::DivTrue && !fa && !fb {
            return self.int_true_div();
        }
        let fop = match op {
            BinOp::DivFloor | BinOp::ModFloor if python => op,
            o => match o.float() {
                Some(f) => f,
                None => return self.err(format!("this operator is not one {} has for floats", self.sh.l.name)),
            },
        };
        if go && (*ta == Ty::Bool || *tb == Ty::Bool) {
            return self.err("a bool and a float meet here; Go does not mix them");
        }
        let w = if go {
            match (fa, wa, fb, wb) {
                (true, 0, true, w) | (true, w, true, 0) => if w == 0 { 64 } else { w },
                (true, x, true, y) if x != y => return self.err(format!("a float{x} and a float{y} meet here; Go wants a conversion")),
                (true, x, true, _) => x,
                (true, w, false, 0) | (false, 0, true, w) => if w == 0 { 64 } else { w },
                // a whole number and a float constant: the constant must be
                // a whole number, and the operation is a whole number's
                (true, 0, false, _) => {
                    let k = ka.expect("a Go float of width 0 is a constant");
                    self.float_const_as_int(a_end, k)?;
                    return self.int_binary(op, 0, wb);
                }
                (false, _, true, 0) => {
                    let k = kb.expect("a Go float of width 0 is a constant");
                    let at = self.code.len();
                    self.float_const_as_int(at, k)?;
                    return self.int_binary(op, wa, 0);
                }
                _ => return self.err("Go does not mix whole numbers and floats; convert one (float64(i))"),
            }
        } else if python {
            64
        } else {
            match (fa.then_some(wa), fb.then_some(wb)) {
                (Some(32), Some(32)) | (Some(32), None) | (None, Some(32)) => 32,
                _ => 64,
            }
        };
        // Python compares a float with a whole number exactly; one that a
        // double holds exactly (a constant within 2^53) is made a float
        if python && fop.is_float_comparison() && fa != fb {
            let int_end = if fa { self.code.len() } else { a_end };
            if !matches!(self.constant_at(int_end), Some(c) if c.unsigned_abs() <= 1 << 53) {
                return self.py_compare(fop, fa);
            }
        }
        self.go_fits32(wa, ka, w)?;
        self.go_fits32(wb, kb, w)?;
        if fa {
            self.float_width_at(a_end, wa, ka, w);
        } else {
            self.to_float_at(a_end, w);
        }
        let end = self.code.len();
        if fb {
            self.float_width_at(end, wb, kb, w);
        } else {
            self.to_float_at(end, w);
        }
        match fop {
            BinOp::DivFloor | BinOp::ModFloor => {
                self.zero_division(true);
                self.lib(if fop == BinOp::DivFloor { "lib.float.py_floordiv" } else { "lib.float.py_mod" }, 2);
            }
            f => {
                if f == BinOp::FDiv {
                    self.zero_division(true);
                }
                self.emit(Op::Bin(f));
            }
        }
        if fop.is_float_comparison() {
            self.last_w = 8;
            return Ok(self.bool_ty());
        }
        if w == 32 {
            self.emit(Op::Un(UnOp::ToF32));
        }
        self.last_w = w;
        Ok(Ty::Float)
    }

    /// Python's comparison of a float and a whole number (both on the
    /// stack, the float first when float_left): exact, by lib.py_cmp.
    fn py_compare(&mut self, op: BinOp, float_left: bool) -> Result<Ty, E> {
        use BinOp::*;
        let op = if float_left {
            op
        } else {
            let (f, i) = (self.hidden(), self.hidden());
            self.ops(&[Op::Store(f), Op::Store(i), Op::Load(f), Op::Load(i)]);
            match op {
                FLt => FGt,
                FGt => FLt,
                FLe => FGe,
                FGe => FLe,
                o => o,
            }
        };
        self.lib("lib.float.py_cmp", 2);
        // py_cmp gives -1, 0, 1, or 2 for NaN
        let (c, test) = match op {
            FLt => (-1, Eq),
            FLe => (0, Le),
            FGt => (1, Eq),
            FGe => (2, Ult),
            FEq => (0, Eq),
            _ => (0, Ne),
        };
        self.ops(&[Op::Push(c), Op::Bin(test)]);
        self.last_w = 8;
        Ok(self.bool_ty())
    }

    /// Python's / of two whole numbers: exactly Python's while both fit 53
    /// bits (a double holds them exactly, and one division rounds once);
    /// past that the program stops instead of guessing.
    fn int_true_div(&mut self) -> Result<Ty, E> {
        let (b, a) = (self.hidden(), self.hidden());
        self.ops(&[Op::Store(b), Op::Store(a), Op::Load(b)]);
        match self.sh.l.zero_division.clone() {
            Some((code, msg)) => self.fail_unless(code, &msg),
            None => self.emit(Op::Pop),
        }
        for s in [a, b] {
            self.ops(&[Op::Load(s), Op::Push(1 << 53), Op::Bin(BinOp::Add), Op::Push(1 << 54), Op::Bin(BinOp::Ule)]);
            self.fail_unless(1, INEXACT_DIVISION);
        }
        self.ops(&[Op::Load(a), Op::Un(UnOp::FromInt), Op::Load(b), Op::Un(UnOp::FromInt), Op::Bin(BinOp::FDiv)]);
        self.last_w = 64;
        Ok(Ty::Float)
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
            // not zero (0.0 and -0.0 are false, NaN is true)
            Ty::Float => self.ops(&[Op::Push(0), Op::Bin(BinOp::FNe)]),
            Ty::Str if self.sh.l.strings == Strings::Ascii => self.emit(Op::LoadM(8)),
            Ty::Slice(..) | Ty::Arr(..) | Ty::Ptr(..) | Ty::Str | Ty::Struct(_) | Ty::Obj(_) => return self.err(format!("{} is not a condition in Strike", t.describe())),
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

    /// The value on top (of kind vt, width `from`) stored at the address in
    /// slot `a`: a number cut to the element's width, a float made the
    /// element's kind, an array (held in place) copied in.
    fn put(&mut self, a: u32, t: &Ty, w: u8, vt: &Ty, from: u8) -> Result<(), E> {
        if t.in_place() {
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
        if t.whole() && vt.whole() {
            if from == 0 {
                self.narrow(w, from)?;
            }
        } else if t.scalar() {
            self.convert(t, w, vt, from)?;
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
            Stmt::Set { target, op, value, line } => {
                self.line = *line;
                self.set(target, *op, value)?;
            }
            Stmt::Class(c, line) => {
                self.line = *line;
                self.class_stmt(c)?;
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
                            if unify(&want, &t).is_none() && !(want.scalar() && t.scalar()) {
                                return self.err(format!("this function returns {}, not {}", want.describe(), t.describe()));
                            }
                        }
                        if !t.scalar() && self.ret_ty.is_none() && self.typed() {
                            return self.err(format!("this function returns a number, not {}", t.describe()));
                        }
                        // a struct is returned in the caller's memory (slot 0)
                        if let Ty::Struct(_) = &t {
                            if !self.sret {
                                return self.err(format!("this function returns a number, not {}", t.describe()));
                            }
                            let src = self.hidden();
                            self.ops(&[Op::Store(src), Op::Load(0), Op::Load(src), Op::Push(t.bytes() as i64)]);
                            self.lib("lib.copy", 3);
                            self.ops(&[Op::Pop, Op::Load(0)]);
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
                        let want = self.ret_ty.clone().unwrap_or(Ty::Int);
                        if t.scalar() && (want == Ty::Float || t == Ty::Float) {
                            self.convert(&want, to, &t, from)?;
                        } else if t.scalar() {
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
                if !ty.whole() {
                    return self.err(format!("{name} is {}; only a whole number is read in this stage of Strike", ty.describe()));
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
        // done as `name op value` is, then put back as the variable's kind:
        // converted (C, Go), or the same kind (Python)
        let w = self.width(slot);
        self.load(slot);
        let at = self.code.len();
        let t = self.binary(op, was.clone(), w, None, at, value, false)?;
        let from = self.last_w;
        if self.typed() {
            self.convert(&was, w, &t, from)?;
        } else {
            self.same(name, &was, &t)?;
        }
        self.store(slot);
        Ok(())
    }

    fn assign(&mut self, name: &str, value: &Expr) -> Result<(), E> {
        let known = self.lookup(name);
        if let Some((slot, t @ (Ty::Arr(..) | Ty::Struct(_)))) = &known {
            // an array assigned whole: Go copies the elements in (and a
            // struct is copied in, in C too)
            if !self.checks() && matches!(t, Ty::Arr(..)) {
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
            return self.put(dst, &t, 64, &got, 64);
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
                // a typed variable keeps its kind: the value is converted to it
                if self.typed() && was.scalar() && (was == Ty::Float || ty == Ty::Float) {
                    let w = self.width(slot);
                    self.convert(&was, w, &ty, from)?;
                    self.store(slot);
                    return Ok(());
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
        if !t.whole() {
            return self.err(format!("a whole number is wanted here, not {}", t.describe()));
        }
        Ok(())
    }

    fn var(&mut self, name: &str, words: &[String], dims: &[Option<Expr>], value: Option<&Expr>) -> Result<(), E> {
        let (mut dt, mut dw) = type_of(self.sh.l, words, self.sh.bools).map_err(|m| (self.line, m))?;
        // a C pointer variable: only one given an array or a string to
        // point at (no arithmetic on it, no & or *)
        let to_struct = matches!(&dt, Ty::Ptr(e, _) if matches!(**e, Ty::Struct(_)));
        if matches!(dt, Ty::Ptr(..)) && ((value.is_none() && !to_struct) || !dims.is_empty()) {
            return self.err("pointer variables are not in Strike yet, but one given an array or a string to point at is, and a pointer to a struct");
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
        if let Ty::Struct(_) = &dt {
            return self.var_struct(name, dt, value);
        }
        // Go's p := P{...}: a struct variable of the composite's type
        if let (true, Some(Expr::Composite(cw, _, _))) = (words.is_empty(), value) {
            if let Ok((t @ Ty::Struct(_), _)) = type_of(self.sh.l, cw, self.sh.bools) {
                return self.var_struct(name, t, value);
            }
        }
        let twice = if self.globals_here { self.sh.globals.contains_key(name) } else { self.scopes.last().unwrap().contains_key(name) };
        if self.sh.l.block_scope && twice {
            return self.err(format!("{name} is declared twice in one block"));
        }
        let ty = match value {
            Some(e) => {
                let t = self.value(e)?;
                // a struct given whole: the variable's own copy (a global's
                // in its own words)
                if let Ty::Struct(_) = &t {
                    if self.globals_here {
                        let src = self.hidden();
                        self.emit(Op::Store(src));
                        let words = u32::try_from(t.words()).ok().filter(|w| *w < 1 << 26).ok_or((self.line, "a global struct this large is not in Strike yet".to_string()))?;
                        if self.sh.globals.contains_key(name) {
                            return self.err(format!("{name} is declared twice"));
                        }
                        let g = self.declare_words(name, t.clone(), words);
                        let a = self.hidden();
                        self.ops(&[Op::AddrG(g & !GLOBAL), Op::Store(a), Op::Load(src)]);
                        return self.put(a, &t, 64, &t, 64);
                    }
                    let own = !self.escapes.contains(name);
                    self.copy_to_new(&t, own)?;
                }
                let from = self.last_w;
                if !words.is_empty() && (!dt.scalar() || !t.scalar()) && unify(&dt, &t).is_none() && !decays(&t, &dt) {
                    return self.err(format!("{name} is {}; it is given {}", dt.describe(), t.describe()));
                }
                // Go's `x := 5` is an int: the literal's own width; `c :=
                // s[0]` is a byte, `x := int32(y)` an int32: the value's. A
                // declared kind converts the value to it
                if t.scalar() && !words.is_empty() && dt.scalar() && (dt == Ty::Float || t == Ty::Float) {
                    self.convert(&dt, dw, &t, from)?;
                } else if t.scalar() {
                    self.narrow(dw, from)?;
                }
                if words.is_empty() && self.typed() && t == Ty::Int && from != 0 {
                    dw = from;
                }
                // a float declared by its value has the value's width
                if words.is_empty() && t == Ty::Float {
                    dw = if from == 0 { 64 } else { from };
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
            self.fix_zero(a, &t)?;
            if let Some(v) = value {
                self.init_array(a, &t, v)?;
            }
            return Ok(());
        }
        if self.scopes.last().unwrap().contains_key(name) {
            return self.err(format!("{name} is declared twice in one block"));
        }
        let own = !self.sliced.contains(name) && !self.escapes.contains(name);
        self.memory_for(&t, own)?;
        let slot = self.declare(name, t.clone());
        self.store(slot);
        let a = self.hidden();
        self.ops(&[Op::Load(slot), Op::Store(a)]);
        // Go gives zeros every time the declaration runs; C gives them only
        // with an initializer (its other elements are zero)
        if self.checks() || value.is_some() {
            self.clear(a, &t)?;
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
                return self.put(a, t, 64, &got, 64);
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
            (Ty::Struct(_), Expr::Brace(..) | Expr::Composite(..)) => self.init_struct(at, e, x),
            (_, Expr::Key(..)) => self.err("a key in an array's or a slice's elements is not in Strike yet"),
            (Ty::Slice(ie, iw), Expr::Brace(inner, _)) => {
                let (ie, iw) = ((**ie).clone(), *iw);
                self.composite_slice(&ie, iw, inner)?;
                self.put(at, e, 64, e, 64)
            }
            (_, Expr::Brace(..)) => self.err("a nested initializer for an element that is not an array"),
            _ => {
                let t = self.value(x)?;
                let from = self.last_w;
                let numbers = e.scalar() && t.scalar() && (*e == Ty::Float || t == Ty::Float || !self.sh.bools);
                if unify(e, &t).is_none() && !numbers && !decays(&t, e) {
                    return self.err(format!("an element is {}, given {}", e.describe(), t.describe()));
                }
                self.put(at, e, w, &t, from)
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
        if holds_struct(e) {
            self.fix_each(data, e, w, &[Op::Push(n)])?;
        } else if e.zero_is_not_zero() {
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
        self.store_at(&target, e, w, op, value, early)
    }

    /// The value (or op= value) stored where the address on top points,
    /// an element of kind e and width w of the container (whose elements'
    /// kind Python learns from it); `early`: the value already worked out
    /// (Python's order), in its slot.
    fn store_at(&mut self, target: &Expr, e: Ty, w: u8, op: Option<BinOp>, value: &Expr, early: Option<(Ty, u32, u8)>) -> Result<(), E> {
        // a number or an address goes straight to where the address on the
        // stack points
        if op.is_none() && !e.in_place() {
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
            self.elem_kind(target, &e, &t)?;
            if e.whole() && t.whole() {
                if from == 0 {
                    self.narrow(w, from)?;
                }
            } else if e.scalar() {
                // a float made the element's kind (C, Go)
                self.convert(&e, w, &t, from)?;
            }
            self.emit(Op::StoreM(size(&e, w) as u8));
            return Ok(());
        }
        // a[i] += x worked on the memory itself, where x calls nothing (so
        // when the element is read does not matter); whole numbers only
        if let Some(op @ (BinOp::Add | BinOp::Sub | BinOp::Band | BinOp::Bor | BinOp::Bxor)) = op {
            if e.whole() && e != Ty::Bool && calls_nothing(value) && self.whole_value(value) {
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
                self.elem_kind(target, &e, &t)?;
                self.put(a, &e, w, &t, from)
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
                let at = self.code.len();
                let t = self.binary(op, e.clone(), w, None, at, value, false)?;
                let from = self.last_w;
                // the store keeps a whole number's own width; a float is
                // made the element's kind
                if !(e.whole() && t.whole()) {
                    if self.typed() {
                        self.convert(&e, w, &t, from)?;
                    } else if e != t {
                        return self.err(format!("an element is {}, given {}; Strike keeps one kind per list", e.describe(), t.describe()));
                    }
                }
                self.emit(Op::StoreM(n));
                Ok(())
            }
        }
    }

    /// An element of the container `c` is given a value of kind t: Python
    /// learns the container's elements from it.
    fn elem_kind(&mut self, c: &Expr, e: &Ty, t: &Ty) -> Result<(), E> {
        // a typed language converts a number to its element's type (or
        // refuses it) where it is stored
        if unify(e, t).is_some() || decays(t, e) || (e.scalar() && t.scalar() && (!self.sh.bools || self.typed())) {
            if let Expr::Var(n, _) = c {
                if let Some((_, ct @ Ty::List(_))) = self.lookup(n) {
                    let _ = ct;
                    let n = n.clone();
                    self.refine(&n, &Ty::List(Box::new(t.clone())))?;
                }
            }
            // a list an object's attribute holds: the attribute learns it
            if let (Expr::Field(b, a, _), true) = (c, self.classes()) {
                let (code, slots, labels) = (self.code.len(), self.slots, self.labels);
                let bt = self.expr(b, true)?;
                self.code.truncate(code);
                (self.slots, self.labels) = (slots, labels);
                if let Ty::Obj(k) = bt {
                    let was = self.attr_kind(&k, a);
                    if let Some(now) = unify(&was, &Ty::List(Box::new(t.clone()))) {
                        self.saw(Self::attr_key(&k, a), now);
                    }
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
            Some(name) => {
                let s = self.loop_var(name, e.clone())?;
                if e == Ty::Float {
                    self.set_width(s, ew);
                }
                Some(s)
            }
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
                // a struct element: the value is a copy of it, the loop's
                // own (new each time round where Go takes its address)
                None if matches!(e, Ty::Struct(_)) => {
                    let own = !value.is_some_and(|v| self.escapes.contains(v));
                    self.copy_to_new(&e, own)?;
                }
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
        self.print_value_as(e, false)
    }

    /// print_value; plus: Go's %+v, a struct's fields by their names.
    fn print_value_as(&mut self, e: &Expr, plus: bool) -> Result<(), E> {
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
                let f = self.printer_plus(&t, plus)?;
                self.emit(Op::Call(f, 1));
                self.emit(Op::Pop);
            }
            (_, Ty::Str) => {
                self.lib("lib.str_print", 1);
                self.emit(Op::Pop);
            }
            (_, Ty::Struct(_)) if self.checks() => self.print_struct(&t, plus)?,
            (_, Ty::Ptr(p, _)) if self.checks() && matches!(**p, Ty::Struct(_)) => self.print_struct(&t, plus)?,
            (_, Ty::Obj(c)) => {
                let c = c.clone();
                self.obj_text(&c, false)?;
                self.lib("lib.str_print", 1);
                self.emit(Op::Pop);
            }
            (_, Ty::Struct(_)) => return self.err("C does not print a struct"),
            (_, Ty::Ptr(..)) => return self.err("a pointer is not printed"),
            (_, Ty::Float) => {
                let f32 = self.last_w == 32;
                self.print_float(f32, None)?;
            }
            _ => self.emit(Op::PrintInt),
        }
        Ok(())
    }

    /// The library function that prints a list, slice or array of kind t.
    fn printer(&mut self, t: &Ty) -> Result<String, E> {
        self.printer_plus(t, false)
    }

    /// printer; plus: its structs print their fields' names (Go's %+v).
    fn printer_plus(&mut self, t: &Ty, plus: bool) -> Result<String, E> {
        let name = format!("lib.print.{t:?}{}", if plus { "+" } else { "" });
        if self.sh.printers.contains_key(&name) {
            return Ok(name);
        }
        let (e, ew) = t.elem().unwrap();
        let elem = match &e {
            Ty::Bool => Elem::Bool,
            Ty::Float => Elem::Float(FloatElem { f32: ew == 32, repr: self.sh.l.float_print == Some(FloatPrint::Repr), buf: self.float_buf() }),
            Ty::List(_) | Ty::Slice(..) => Elem::Nested(self.printer_plus(&e, plus)?, false),
            Ty::Arr(..) => Elem::Nested(self.printer_plus(&e, plus)?, true),
            Ty::Struct(s) => {
                self.printed_plainly(s)?;
                Elem::Nested(self.struct_printer(s, plus)?, true)
            }
            // Python shows an object in a list by its __repr__
            Ty::Obj(c) => {
                let Some(d) = self.resolve(c, "__repr__") else {
                    return self.err(format!("a list shows an object of class {c} by its __repr__, and {c} has none: it would print an address, which Strike cannot give as Python does"));
                };
                let f = instance(c, &d, "__repr__");
                if self.sh.sigs.get(&f).is_some_and(|s| s.ret != Ty::Str && s.ret != Ty::Unknown) {
                    return self.err(format!("{f} returns something other than a string (TypeError)"));
                }
                Elem::Repr(symbol(&f))
            }
            Ty::Ptr(..) => return self.err("a pointer in a slice prints as its address, which Strike cannot give as Go does"),
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
            fields: None,
        };
        self.sh.printers.insert(name.clone(), p);
        Ok(name)
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
            Expr::SizeOf(..) | Expr::SizeOfType(..) | Expr::OffsetOf(..) => true,
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
                    // a precision: .digits (none after the point is 0, as in C)
                    let mut prec: Option<i64> = None;
                    if f.get(i) == Some(&b'.') {
                        i += 1;
                        let mut p = 0i64;
                        while let Some(d) = f.get(i).filter(|d| d.is_ascii_digit()) {
                            p = (p * 10 + (d - b'0') as i64).min(1 << 20);
                            i += 1;
                        }
                        prec = Some(p);
                    }
                    // Go's %+v: a struct's fields by their names
                    let plus = self.sh.l.printf_go == Some(true) && f.get(i) == Some(&b'+') && f.get(i + 1) == Some(&b'v');
                    if plus {
                        i += 1;
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
                    if prec.is_some() && !matches!(spec, b'f' | b'e' | b'E' | b'g' | b'G') {
                        return self.err(format!("a precision for %{} is not in Strike yet", spec as char));
                    }
                    match spec {
                        b'd' | b'i' => {
                            if matches!(a, Expr::Str(_)) {
                                return self.err("%d is given a string");
                            }
                            let t = self.expr(a, true)?;
                            if t == Ty::Bool && self.sh.bools {
                                return self.err("%d is given a truth value");
                            }
                            if !t.whole() {
                                return self.err(format!("%d is given {}", t.describe()));
                            }
                            self.emit(Op::PrintInt);
                        }
                        b'v' => self.print_value_as(a, plus)?,
                        b'f' | b'e' | b'E' | b'g' | b'G' if self.sh.l.printf_go.is_some() => {
                            if matches!(a, Expr::Str(_)) {
                                return self.err(format!("%{} is given a string", spec as char));
                            }
                            let t = self.expr(a, true)?;
                            if t != Ty::Float {
                                return self.err(format!("%{} is given {}, not a float", spec as char, t.describe()));
                            }
                            let f32 = self.last_w == 32;
                            // Go's %g without a precision is its shortest
                            let go = self.sh.l.printf_go == Some(true);
                            let p = prec.unwrap_or(if go && matches!(spec, b'g' | b'G') { -1 } else { 6 });
                            if p > 100 {
                                return self.err("a precision above 100 is not in Strike yet");
                            }
                            self.print_float(f32, Some((spec, p)))?;
                        }
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
                                    let f = self.printer(&t)?;
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
                if holds_struct(&e) {
                    let d = self.hidden();
                    self.ops(&[Op::Load(h), Op::LoadM(8), Op::Store(d)]);
                    self.fix_each(d, &e, ew, &[Op::Load(cs)])?;
                } else if e.zero_is_not_zero() {
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
                    if e.whole() && vt.whole() {
                        self.narrow(w, from)?;
                    } else if e.scalar() {
                        self.convert(&e, w, &vt, from)?;
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
                    self.put(at, &e, 64, &e, 64)?;
                }
                self.emit(Op::Load(nh));
                self.last_w = 64;
                Ok(t)
            }
            // the square root and the size of a float: exact, one rounding
            // (sqrtsd) and none (the sign bit)
            "sqrt" | "fabs" => {
                let [x] = args else { return self.err(format!("{name} takes one value")) };
                let t = self.expr(x, true)?;
                let w = self.last_w;
                if !t.scalar() {
                    return self.err(format!("{name} takes a float, not {}", t.describe()));
                }
                if self.sh.l.untyped_literals && t == Ty::Float && w == 32 {
                    return self.err(format!("{name} takes a float64, not a float32; Go wants a conversion"));
                }
                if t != Ty::Float {
                    if self.typed() {
                        self.convert(&Ty::Float, 64, &t, w)?;
                    } else {
                        let at = self.code.len();
                        self.to_float_at(at, 64);
                    }
                }
                self.emit(Op::Un(if what == "sqrt" { UnOp::FSqrt } else { UnOp::FAbs }));
                self.last_w = 64;
                Ok(Ty::Float)
            }
            // Python's math.sqrt: a negative number stops the program with
            // its ValueError (its message says the number)
            "py_sqrt" => {
                let [x] = args else { return self.err(format!("{name} takes one value")) };
                let t = self.expr(x, true)?;
                if !t.scalar() {
                    return self.err(format!("{name} takes a number, not {}", t.describe()));
                }
                if t != Ty::Float {
                    let at = self.code.len();
                    self.to_float_at(at, 64);
                }
                let (v, ok) = (self.hidden(), self.label());
                let buf = self.float_buf();
                self.ops(&[Op::Store(v), Op::Load(v), Op::Push(0), Op::Bin(BinOp::FLt), Op::Jz(ok)]);
                self.ops(&[Op::AddrG(buf), Op::Load(v), Op::AddrG(buf)]);
                self.lib("lib.float.py_sqrt_error", 2);
                self.ops(&[Op::FailMem(1), Op::Label(ok), Op::Load(v), Op::Un(UnOp::FSqrt)]);
                self.last_w = 64;
                Ok(Ty::Float)
            }
            // Python's abs: a whole number's (checked: the lowest has no
            // positive), a float's
            "abs" => {
                let [x] = args else { return self.err(format!("{name} takes one value")) };
                let t = self.expr(x, true)?;
                match t {
                    Ty::Float => {
                        self.emit(Op::Un(UnOp::FAbs));
                        Ok(Ty::Float)
                    }
                    t if t.whole() => {
                        let (h, done) = (self.hidden(), self.label());
                        let neg = if self.sh.l.checked { UnOp::NegOv } else { UnOp::Neg };
                        self.ops(&[Op::Store(h), Op::Load(h), Op::Push(0), Op::Bin(BinOp::Lt), Op::Jz(done), Op::Load(h), Op::Un(neg), Op::Store(h), Op::Label(done), Op::Load(h)]);
                        self.last_w = 64;
                        Ok(Ty::Int)
                    }
                    t => self.err(format!("{name} takes a number, not {}", t.describe())),
                }
            }
            // C's fmod: given to the float library only (lib.rs), not to C
            // programs, whose fmod is the UCRT's
            "fmod" => {
                let [x, y] = args else { return self.err(format!("{name} takes two values")) };
                for e in [x, y] {
                    if self.expr(e, true)? != Ty::Float {
                        return self.err(format!("{name} takes floats"));
                    }
                }
                self.emit(Op::Bin(BinOp::FRem));
                self.last_w = 64;
                Ok(Ty::Float)
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
                    Ty::Obj(c) => self.obj_text(&c, false)?,
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
    /// only for an expression statement. In Go a constant expression with a
    /// float in it is worked out here, where it is exact (Go's constants
    /// are exact numbers), and refused where it is not.
    fn expr(&mut self, e: &Expr, used: bool) -> Result<Ty, E> {
        self.last_k = None;
        if self.sh.l.untyped_literals && matches!(e, Expr::Un(..) | Expr::Bin(..)) {
            match go_const(e) {
                Some(Err(m)) => return self.err(m),
                Some(Ok(K::Float(f))) => {
                    self.emit(Op::Push(f.bits as i64));
                    self.last_w = 0;
                    self.last_k = Some(f);
                    return Ok(Ty::Float);
                }
                _ => {}
            }
        }
        let t = self.expr_case(e, used)?;
        if !matches!(e, Expr::Float(_)) {
            self.last_k = None;
        }
        Ok(t)
    }

    fn expr_case(&mut self, e: &Expr, used: bool) -> Result<Ty, E> {
        // every case leaves the width of its value in last_w; a truth
        // value fits 8 bits
        Ok(match e {
            Expr::Float(f) => {
                if self.sh.l.untyped_literals && f64::from_bits(f.bits).is_infinite() {
                    return self.err("the constant overflows float64");
                }
                self.emit(Op::Push(if f.f32 { f.bits32 } else { f.bits } as i64));
                self.last_w = if self.sh.l.untyped_literals {
                    0
                } else if f.f32 {
                    32
                } else {
                    64
                };
                self.last_k = Some(*f);
                Ty::Float
            }
            Expr::Cast(words, x, line) => {
                self.line = *line;
                if words.iter().any(|w| w == "bool" || w == "_Bool") {
                    return self.err("a cast to bool is not in Strike yet");
                }
                let (to, tw) = type_of(self.sh.l, words, self.sh.bools).map_err(|m| (self.line, m))?;
                if !to.scalar() {
                    return self.err("a cast to this type is not in Strike yet");
                }
                let t = self.expr(x, true)?;
                let fw = self.last_w;
                self.explicit_convert(&to, tw, &t, fw)?;
                self.last_w = tw;
                to
            }
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
                self.method(base, name, args, used)?
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
                            self.clear(a, &t)?;
                        }
                        self.init_array(a, &t, e)?;
                        self.emit(Op::Load(a));
                    }
                    // a struct: made in the function's own memory, where
                    // whatever takes it copies it from
                    Ty::Struct(_) => {
                        let a = self.hidden();
                        self.memory_for(&t, true)?;
                        self.emit(Op::Store(a));
                        self.init_struct(a, &t, e)?;
                        self.emit(Op::Load(a));
                    }
                    _ => return self.err("this composite literal is not in Strike yet"),
                }
                self.last_w = 64;
                t
            }
            Expr::Brace(_, line) => {
                self.line = *line;
                return self.err("a {...} list is only an array's or a struct's initializer");
            }
            Expr::Field(base, name, line) => {
                self.line = *line;
                if self.classes() {
                    return self.attr_read(base, name);
                }
                let (t, w) = self.field_addr(base, name)?;
                if let Some(l) = load_of(&t, w) {
                    self.emit(l);
                }
                self.last_w = if t.scalar() { w } else { 64 };
                t
            }
            Expr::Addr(x, line) => {
                self.line = *line;
                self.addr_of(x)?
            }
            Expr::Deref(x, line) => {
                self.line = *line;
                let t = self.expr(x, true)?;
                match t {
                    Ty::Ptr(e, _) if matches!(*e, Ty::Struct(_)) => {
                        self.nil_check();
                        self.last_w = 64;
                        *e
                    }
                    t => return self.err(format!("* of {} is not in Strike yet (only a pointer to a struct is followed)", t.describe())),
                }
            }
            Expr::Key(n, _, line) => {
                self.line = *line;
                return self.err(format!("{n}: a field is named so only in a struct's {{...}}"));
            }
            Expr::SizeOfType(..) | Expr::SizeOf(..) | Expr::OffsetOf(..) => {
                let n = self.size_of(e)?;
                self.emit(Op::Push(n as i64));
                self.last_w = 64;
                Ty::Int
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
                            self.explicit_convert(&Ty::Int, w, &t, from)?;
                            self.last_w = w;
                            return Ok(Ty::Int);
                        }
                        // Go's float64(i), Python's float(x)
                        Ok((Ty::Float, w)) => {
                            let t = self.expr(&args[0], true)?;
                            let from = self.last_w;
                            self.explicit_convert(&Ty::Float, w, &t, from)?;
                            self.last_w = w;
                            return Ok(Ty::Float);
                        }
                        // Go's string(r): the code point's UTF-8
                        Ok((Ty::Str, _)) => {
                            let t = self.expr(&args[0], true)?;
                            match t {
                                Ty::Str => {}
                                Ty::Bool if self.sh.bools => return self.err("a truth value is not made a string"),
                                Ty::Float => return self.err("a float is not made a string in Strike yet"),
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
                    // Python's P(...): a new object of the class
                    if self.class(name).is_some() && self.lookup(name).is_none() {
                        return self.construct(name, args, *line);
                    }
                    if let Some(what) = self.sh.l.builtins.get(name).cloned() {
                        return self.builtin(&what, name, args);
                    }
                    return self.err(format!("there is no function {name}"));
                };
                self.call_sig(name, &sig, None, args, *line, used)?
            }
            Expr::Un(op, x) => {
                if *op == UnOp::Not {
                    self.cond(x)?;
                    self.emit(Op::Un(UnOp::Not));
                    self.last_w = 8;
                    return Ok(self.bool_ty());
                }
                let t = self.expr(x, true)?;
                if t == Ty::Float {
                    match op {
                        UnOp::Plus => {}
                        UnOp::Neg | UnOp::NegOv => self.emit(Op::Un(UnOp::FNeg)),
                        _ => return self.err("this operator is not one a float has"),
                    }
                    return Ok(Ty::Float);
                }
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
                    if ta == Ty::Float {
                        self.ops(&[Op::Push(0), Op::Bin(BinOp::FNe)]);
                    }
                    self.emit(if *op == BinOp::AndValue { Op::Jz(end) } else { Op::Jnz(end) });
                    self.emit(Op::Pop);
                    let tb = self.expr(b, true)?;
                    self.emit(Op::Label(end));
                    if ta != tb && (self.sh.bools || ta == Ty::Float || tb == Ty::Float) {
                        return self.err(format!("and/or between {} and {} is not in Strike yet", ta.describe(), tb.describe()));
                    }
                    self.last_w = 64;
                    ta
                }
                _ => {
                    // C's size_t (strlen): divided and shifted unsigned, which
                    // Strike does not do yet; compared unsigned below
                    let uns = self.sh.l.strings == Strings::C && (self.unsigned(a) || self.unsigned(b));
                    // sizes known while compiling (sizeof a / sizeof a[0]):
                    // worked out as the unsigned numbers they are
                    if uns && matches!(op, BinOp::DivTrunc | BinOp::ModTrunc | BinOp::Shr) {
                        if let Some(v) = self.size_const(e)? {
                            self.emit(Op::Push(v as i64));
                            self.last_w = 64;
                            return Ok(Ty::Int);
                        }
                    }
                    if uns && matches!(op, BinOp::DivTrunc | BinOp::ModTrunc | BinOp::Shr) {
                        return self.err("strlen's size is unsigned, and unsigned division and shifts are not in Strike yet (store it in an int first)");
                    }
                    let ta = self.expr(a, true)?;
                    let (wa, ka, at) = (self.last_w, self.last_k, self.code.len());
                    self.binary(*op, ta, wa, ka, at, b, uns)?
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
    fn method(&mut self, base: &Expr, name: &str, args: &[Expr], used: bool) -> Result<Ty, E> {
        let line = self.line;
        // Go's package.name(...): a package is not a variable
        if let Expr::Var(pkg, _) = base {
            if !self.sh.l.qualified.is_empty() && self.lookup(pkg).is_none() && self.class(pkg).is_none() {
                let key = format!("{pkg}.{name}");
                let Some(what) = self.sh.l.qualified.get(&key).cloned() else {
                    return self.err(format!("{key} is not in Strike yet"));
                };
                return self.builtin(&what, &key, args);
            }
        }
        // a Go struct's method; a Python object's
        if self.sh.l.records.is_some() && !self.classes() {
            return self.struct_method(base, name, args, line, used);
        }
        if self.classes() {
            if let Some(t) = self.obj_method(base, name, args, line, used)? {
                return Ok(t);
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

/// Go's stop where a nil pointer is followed (exit code 2, as Go's panic).
const NIL: &[u8] = b"panic: runtime error: invalid memory address or nil pointer dereference\n";

/// A Python class: its base, the attributes its objects are given (by any
/// method of it or of a base), its own methods and its own attributes.
#[derive(Debug, Default, Clone)]
pub struct Class {
    base: Option<String>,
    attrs: Vec<String>,
    methods: HashSet<String>,
    class_attrs: Vec<String>,
}

/// The most attributes an object has: which are set is one word of bits.
const MAX_ATTRS: usize = 63;

/// The name of the method m of class d, called on an object of class r
/// (d is r or one of its bases): each is lowered for the class it is
/// called on, so self's attributes are where r keeps them.
fn instance(r: &str, d: &str, m: &str) -> String {
    if r == d {
        format!("{r}.{m}")
    } else {
        format!("{r}.{m}@{d}")
    }
}

/// Does the kind hold a struct anywhere in itself (not through a pointer)?
fn holds_struct(t: &Ty) -> bool {
    match t {
        Ty::Struct(_) => true,
        Ty::Arr(e, _, _) => holds_struct(e),
        _ => false,
    }
}

/// The bytes C gives a value of kind t and width w (sizeof): a float its
/// own 4 where Strike holds it as a double.
fn c_size(t: &Ty, w: u8) -> u64 {
    match t {
        Ty::Float => w as u64 / 8,
        Ty::Arr(e, ew, n) => n * c_size(e, *ew),
        Ty::Struct(s) => layout(s).size,
        Ty::Ptr(..) | Ty::Str => 8,
        _ => (bits(w) as u64 / 8).max(1),
    }
}

/// The names of a Go body whose address is taken (&v, or a method that
/// wants a pointer called on v, or on an element or field of it): their
/// memory must outlive the call and be new each time they are declared, as
/// Go's escape analysis puts them on the heap.
fn escaping_names(body: &[Stmt], ptr_methods: &HashSet<String>) -> HashSet<String> {
    fn expr(e: &Expr, pm: &HashSet<String>, out: &mut HashSet<String>) {
        match e {
            Expr::Addr(x, _) => {
                if let Some(n) = x.root() {
                    out.insert(n.to_string());
                }
                expr(x, pm, out);
            }
            Expr::Method(b, m, a, _) => {
                if pm.contains(m) {
                    if let Some(n) = b.root() {
                        out.insert(n.to_string());
                    }
                }
                expr(b, pm, out);
                a.iter().for_each(|x| expr(x, pm, out));
            }
            Expr::Call(_, a, _) | Expr::List(a, _) | Expr::Brace(a, _) | Expr::Composite(_, a, _) => a.iter().for_each(|x| expr(x, pm, out)),
            Expr::Un(_, x) | Expr::Cast(_, x, _) | Expr::Field(x, _, _) | Expr::Deref(x, _) | Expr::Key(_, x, _) | Expr::SizeOf(x, _) => expr(x, pm, out),
            Expr::Bin(_, a, b) | Expr::Index(a, b, _) => {
                expr(a, pm, out);
                expr(b, pm, out);
            }
            Expr::Slice(b, lo, hi, _) => {
                expr(b, pm, out);
                lo.iter().chain(hi.iter()).for_each(|x| expr(x, pm, out));
            }
            Expr::Comp { elem, iter, cond, .. } => {
                expr(elem, pm, out);
                expr(iter, pm, out);
                cond.iter().for_each(|x| expr(x, pm, out));
            }
            _ => {}
        }
    }
    fn walk(body: &[Stmt], pm: &HashSet<String>, out: &mut HashSet<String>) {
        for s in body {
            match s {
                Stmt::Var { value, .. } => value.iter().for_each(|x| expr(x, pm, out)),
                Stmt::Assign { index, value, .. } => {
                    index.iter().for_each(|x| expr(x, pm, out));
                    expr(value, pm, out);
                }
                Stmt::Set { target, value, .. } => {
                    expr(target, pm, out);
                    expr(value, pm, out);
                }
                Stmt::If { cond, then, els } => {
                    expr(cond, pm, out);
                    walk(then, pm, out);
                    walk(els, pm, out);
                }
                Stmt::While { cond, body } => {
                    cond.iter().for_each(|x| expr(x, pm, out));
                    walk(body, pm, out);
                }
                Stmt::For { init, cond, step, body } => {
                    walk(std::slice::from_ref(init), pm, out);
                    expr(cond, pm, out);
                    walk(std::slice::from_ref(step), pm, out);
                    walk(body, pm, out);
                }
                Stmt::ForRange { args, body, .. } => {
                    args.iter().for_each(|x| expr(x, pm, out));
                    walk(body, pm, out);
                }
                Stmt::ForEach { iter, body, .. } => {
                    expr(iter, pm, out);
                    walk(body, pm, out);
                }
                Stmt::Return(e, _) => e.iter().for_each(|x| expr(x, pm, out)),
                Stmt::Print { args, .. } => args.iter().for_each(|x| expr(x, pm, out)),
                Stmt::Expr(e) => expr(e, pm, out),
                _ => {}
            }
        }
    }
    let mut out = HashSet::new();
    walk(body, ptr_methods, &mut out);
    out
}

/// The attributes a method gives self (self.x = ...), in the order written.
fn self_attrs(f: &FuncDef, out: &mut Vec<String>) {
    let Some(me) = f.params.first() else { return };
    fn walk(body: &[Stmt], me: &str, out: &mut Vec<String>) {
        for s in body {
            match s {
                Stmt::Set { target: Expr::Field(b, a, _), .. } if matches!(&**b, Expr::Var(n, _) if n == me) => {
                    if !out.contains(a) {
                        out.push(a.clone());
                    }
                }
                Stmt::If { then, els, .. } => {
                    walk(then, me, out);
                    walk(els, me, out);
                }
                Stmt::While { body, .. } | Stmt::ForRange { body, .. } | Stmt::ForEach { body, .. } => walk(body, me, out),
                Stmt::For { init, step, body, .. } => {
                    walk(std::slice::from_ref(init), me, out);
                    walk(std::slice::from_ref(step), me, out);
                    walk(body, me, out);
                }
                _ => {}
            }
        }
    }
    walk(&f.body, me, out);
}

impl Fx<'_, '_> {
    /// Does the language have Python's classes (objects by reference)?
    fn classes(&self) -> bool {
        self.sh.l.records.as_ref().is_some_and(|r| r.classes)
    }

    /// The address on top checked not to be nil, as Go checks it (C does
    /// not check, as C does not).
    fn nil_check(&mut self) {
        if !self.checks() {
            return;
        }
        let ok = self.label();
        let code = self.go_fail();
        self.ops(&[Op::Dup, Op::Jnz(ok), Op::Fail(code, NIL.to_vec()), Op::Label(ok)]);
    }

    /// The zero of kind t (C's zero bytes, Go's zero value) at the address
    /// in slot a.
    fn clear(&mut self, a: u32, t: &Ty) -> Result<(), E> {
        if !holds_struct(t) {
            self.fill_zero(a, t, t.words());
            return Ok(());
        }
        self.ops(&[Op::Load(a), Op::Push(t.words() as i64), Op::Push(0)]);
        self.lib("lib.fill", 3);
        self.emit(Op::Pop);
        self.fix_zero(a, t)
    }

    /// Over zero bytes at the address in slot a, the zeros of kind t that
    /// are not zero bytes: Go's empty string and empty slice.
    fn fix_zero(&mut self, a: u32, t: &Ty) -> Result<(), E> {
        if !t.zero_is_not_zero() {
            return Ok(());
        }
        match t {
            Ty::Str | Ty::Slice(..) => {
                self.emit(Op::Load(a));
                self.zero(t);
                self.emit(Op::StoreM(8));
            }
            Ty::Arr(..) if !holds_struct(t) => self.fill_zero(a, t, t.words()),
            Ty::Arr(e, w, n) => {
                let (e, w, n) = ((**e).clone(), *w, *n);
                self.fix_each(a, &e, w, &[Op::Push(n as i64)])?;
            }
            Ty::Struct(s) => {
                for f in layout(s).fields.iter() {
                    if f.ty.zero_is_not_zero() {
                        let at = self.hidden();
                        self.ops(&[Op::Load(a), Op::Push(f.off as i64), Op::Bin(BinOp::Add), Op::Store(at)]);
                        self.fix_zero(at, &f.ty)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// fix_zero for each of the elements (kind e, width w) at the address
    /// in slot a, as many as the code `n` pushes.
    fn fix_each(&mut self, a: u32, e: &Ty, w: u8, n: &[Op]) -> Result<(), E> {
        let (i, at) = (self.hidden(), self.hidden());
        let (top, done) = (self.label(), self.label());
        self.ops(&[Op::Push(0), Op::Store(i), Op::Label(top), Op::Load(i)]);
        self.ops(n);
        self.ops(&[Op::Bin(BinOp::Lt), Op::Jz(done)]);
        self.ops(&[Op::Load(a), Op::Load(i), Op::Push(size(e, w) as i64), Op::Bin(BinOp::Mul), Op::Bin(BinOp::Add), Op::Store(at)]);
        self.fix_zero(at, e)?;
        self.ops(&[Op::Load(i), Op::Push(1), Op::Bin(BinOp::Add), Op::Store(i), Op::Jmp(top), Op::Label(done)]);
        Ok(())
    }

    /// The struct whose address is on top copied to memory of its own: the
    /// function's (own) or new.
    fn copy_to_new(&mut self, t: &Ty, own: bool) -> Result<(), E> {
        let src = self.hidden();
        self.emit(Op::Store(src));
        let dst = self.hidden();
        self.memory_for(t, own)?;
        self.emit(Op::Store(dst));
        self.ops(&[Op::Load(dst), Op::Load(src), Op::Push(t.bytes() as i64)]);
        self.lib("lib.copy", 3);
        self.ops(&[Op::Pop, Op::Load(dst)]);
        Ok(())
    }

    /// A struct's fields given their first values at the address in slot
    /// a: from C's {...} (in order, or .x = v), Go's composite (in order,
    /// every field, or X: v), or a struct of the same kind (copied). What
    /// is not given is zero.
    fn init_struct(&mut self, a: u32, t: &Ty, v: &Expr) -> Result<(), E> {
        let Ty::Struct(s) = t else { unreachable!() };
        let elems = match v {
            Expr::Brace(elems, _) => elems,
            Expr::Composite(words, elems, _) => {
                let (ct, _) = type_of(self.sh.l, words, self.sh.bools).map_err(|m| (self.line, m))?;
                if ct != *t {
                    return self.err(format!("{} is given {}", t.describe(), ct.describe()));
                }
                elems
            }
            other => {
                let got = self.expr(other, true)?;
                if got != *t {
                    return self.err(format!("{} is given {}", t.describe(), got.describe()));
                }
                return self.put(a, t, 64, &got, 64);
            }
        };
        let l = layout(s);
        self.clear(a, t)?;
        let go = self.checks();
        let keyed = elems.iter().filter(|e| matches!(e, Expr::Key(..))).count();
        if go && keyed > 0 && keyed < elems.len() {
            return self.err("mixture of field:value and value elements in struct literal");
        }
        if go && keyed == 0 && !elems.is_empty() && elems.len() < l.fields.len() {
            return self.err(format!("too few values in struct literal of type {s}"));
        }
        let mut next = 0;
        let mut given = HashSet::new();
        for x in elems {
            let (i, x) = match x {
                Expr::Key(n, v, line) => {
                    self.line = *line;
                    match l.fields.iter().position(|f| f.name == *n) {
                        Some(i) => (i, &**v),
                        None => return self.err(format!("unknown field {n} in struct literal of type {s}")),
                    }
                }
                x => (next, x),
            };
            if i >= l.fields.len() {
                return self.err(if go { format!("too many values in struct literal of type {s}") } else { format!("excess elements in the initializer of struct {s}") });
            }
            if go && !given.insert(i) {
                return self.err(format!("duplicate field name {} in struct literal", l.fields[i].name));
            }
            next = i + 1;
            let f = &l.fields[i];
            let at = self.hidden();
            self.ops(&[Op::Load(a), Op::Push(f.off as i64), Op::Bin(BinOp::Add), Op::Store(at)]);
            self.elem_value(at, &f.ty, f.w, x)?;
        }
        Ok(())
    }

    /// A C or Go struct variable: its memory (the function's own, unless Go
    /// takes its address, or a global's), its first value worked out before
    /// its name is declared (Go's p := P{X: p.Y} reads the p before).
    fn var_struct(&mut self, name: &str, t: Ty, value: Option<&Expr>) -> Result<(), E> {
        if self.globals_here {
            let words = u32::try_from(t.words()).ok().filter(|w| *w < 1 << 26).ok_or((self.line, "a global struct this large is not in Strike yet".to_string()))?;
            if self.sh.globals.contains_key(name) {
                return self.err(format!("{name} is declared twice"));
            }
            let g = self.declare_words(name, t.clone(), words);
            let a = self.hidden();
            self.ops(&[Op::AddrG(g & !GLOBAL), Op::Store(a)]);
            // a global's bytes are zero before main
            self.fix_zero(a, &t)?;
            if let Some(v) = value {
                self.init_struct(a, &t, v)?;
            }
            return Ok(());
        }
        if self.sh.l.block_scope && self.scopes.last().unwrap().contains_key(name) {
            return self.err(format!("{name} is declared twice in one block"));
        }
        let own = !self.escapes.contains(name);
        let a = self.hidden();
        match value {
            Some(v @ (Expr::Brace(..) | Expr::Composite(..))) => {
                self.memory_for(&t, own)?;
                self.emit(Op::Store(a));
                self.init_struct(a, &t, v)?;
            }
            Some(v) => {
                let got = self.expr(v, true)?;
                if got != t {
                    return self.err(format!("{name} is {}; it is given {}", t.describe(), got.describe()));
                }
                self.copy_to_new(&t, own)?;
                self.emit(Op::Store(a));
            }
            None => {
                self.memory_for(&t, own)?;
                self.emit(Op::Store(a));
                // Go gives the zero value every time; C leaves a local's
                // bytes as they were
                if self.checks() {
                    self.clear(a, &t)?;
                }
            }
        }
        let slot = self.declare(name, t);
        self.emit(Op::Load(a));
        self.store(slot);
        Ok(())
    }

    /// The struct the value on top is: a struct's address, or a pointer
    /// Go follows (checked not nil).
    fn struct_value(&mut self, t: Ty, what: &str) -> Result<String, E> {
        match t {
            Ty::Struct(s) => Ok(s),
            Ty::Ptr(e, _) if matches!(*e, Ty::Struct(_)) => {
                let Ty::Struct(s) = *e else { unreachable!() };
                if !self.sh.l.records.as_ref().is_some_and(|r| r.auto_deref) {
                    return self.err(format!("{what} is a pointer to struct {s}; C reaches its fields with ->"));
                }
                self.nil_check();
                Ok(s)
            }
            t => self.err(format!("{what} is {}, which has no fields", t.describe())),
        }
    }

    /// base.name's address on the stack; the field's kind and width.
    fn field_addr(&mut self, base: &Expr, name: &str) -> Result<(Ty, u8), E> {
        let t = self.expr(base, true)?;
        let s = self.struct_value(t, "this")?;
        let l = layout(&s);
        let Some(f) = l.fields.iter().find(|f| f.name == name) else {
            return self.err(format!("struct {s} has no field {name}"));
        };
        if f.off != 0 {
            self.ops(&[Op::Push(f.off as i64), Op::Bin(BinOp::Add)]);
        }
        Ok((f.ty.clone(), f.w))
    }

    /// The address of a place on the stack (a field, an element, what a
    /// pointer points at): its kind and width.
    fn place(&mut self, target: &Expr) -> Result<(Ty, u8), E> {
        match target {
            Expr::Field(b, n, line) => {
                self.line = *line;
                self.field_addr(b, n)
            }
            Expr::Index(b, i, line) => {
                self.line = *line;
                self.elem_addr(b, i)
            }
            Expr::Deref(p, line) => {
                self.line = *line;
                let t = self.expr(p, true)?;
                match t {
                    Ty::Ptr(e, _) if matches!(*e, Ty::Struct(_)) => {
                        self.nil_check();
                        Ok((*e, 64))
                    }
                    t => self.err(format!("* of {} is not in Strike yet (only a pointer to a struct is followed)", t.describe())),
                }
            }
            _ => self.err("a value is given only to a name, an element or a field"),
        }
    }

    /// target = value (or op=) for a target that is a field or what a
    /// pointer points at.
    fn set(&mut self, target: &Expr, op: Option<BinOp>, value: &Expr) -> Result<(), E> {
        if self.classes() {
            if let Expr::Field(b, n, _) = target {
                return self.attr_set(b, n, op, value);
            }
        }
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
        let (e, w) = self.place(target)?;
        let container = match target {
            Expr::Index(b, _, _) => (**b).clone(),
            t => t.clone(),
        };
        self.store_at(&container, e, w, op, value, early)
    }

    /// &x: the address of a struct (a variable, a field, an element), or
    /// Go's &P{...}, a new one.
    fn addr_of(&mut self, x: &Expr) -> Result<Ty, E> {
        self.last_w = 64;
        match x {
            Expr::Composite(words, _, line) => {
                self.line = *line;
                let (t, _) = type_of(self.sh.l, words, self.sh.bools).map_err(|m| (self.line, m))?;
                if !matches!(t, Ty::Struct(_)) {
                    return self.err(format!("& of {} is not in Strike yet: only a struct's address is taken", t.describe()));
                }
                let a = self.hidden();
                self.memory_for(&t, false)?;
                self.emit(Op::Store(a));
                self.init_struct(a, &t, x)?;
                self.emit(Op::Load(a));
                self.last_w = 64;
                Ok(Ty::Ptr(Box::new(t), 64))
            }
            Expr::Var(..) | Expr::Field(..) | Expr::Index(..) | Expr::Deref(..) => {
                let t = self.expr(x, true)?;
                self.last_w = 64;
                match t {
                    Ty::Struct(_) => Ok(Ty::Ptr(Box::new(t), 64)),
                    t => self.err(format!("& of {} is not in Strike yet: only a struct's address is taken", t.describe())),
                }
            }
            _ => self.err("& of this is not in Strike yet: only a struct's address is taken"),
        }
    }

    /// The type of an expression C's sizeof is given, without its code
    /// (sizeof does not work its operand out): a variable, a field, an
    /// element, what a pointer points at.
    fn type_only(&mut self, x: &Expr) -> Result<(Ty, u8), E> {
        if x.root().is_none() && !matches!(x, Expr::Deref(..)) {
            return self.err("sizeof of this expression is not in Strike yet (of a type, a variable, a field or an element it is)");
        }
        let (code, slots, labels) = (self.code.len(), self.slots, self.labels);
        let t = self.expr(x, true)?;
        let w = self.last_w;
        self.code.truncate(code);
        (self.slots, self.labels) = (slots, labels);
        Ok((t, w))
    }

    /// The number C's sizeof or offsetof gives, when the expression is
    /// one (so an unsigned size can be divided: sizeof a / sizeof a[0]).
    fn size_const(&mut self, e: &Expr) -> Result<Option<u64>, E> {
        Ok(match e {
            Expr::SizeOfType(..) | Expr::SizeOf(..) | Expr::OffsetOf(..) => Some(self.size_of(e)?),
            Expr::Num(v, false) if *v >= 0 => Some(*v as u64),
            Expr::Bin(op, a, b) => {
                let (Some(x), Some(y)) = (self.size_const(a)?, self.size_const(b)?) else { return Ok(None) };
                match op {
                    BinOp::Add => Some(x.wrapping_add(y)),
                    BinOp::Sub => Some(x.wrapping_sub(y)),
                    BinOp::Mul => Some(x.wrapping_mul(y)),
                    BinOp::DivTrunc if y != 0 => Some(x / y),
                    BinOp::ModTrunc if y != 0 => Some(x % y),
                    BinOp::Shr if y < 64 => Some(x >> y),
                    _ => None,
                }
            }
            _ => None,
        })
    }

    /// sizeof(type), sizeof x, offsetof(type, field): C's bytes, as clang
    /// counts them.
    fn size_of(&mut self, e: &Expr) -> Result<u64, E> {
        match e {
            Expr::SizeOfType(words, line) => {
                self.line = *line;
                if words.iter().all(|w| w == "void") {
                    return self.err("sizeof(void) is not C's");
                }
                let (t, w) = type_of(self.sh.l, words, self.sh.bools).map_err(|m| (self.line, m))?;
                Ok(c_size(&t, w))
            }
            Expr::SizeOf(x, line) => {
                self.line = *line;
                let (t, w) = self.type_only(x)?;
                Ok(c_size(&t, w))
            }
            Expr::OffsetOf(words, f, line) => {
                self.line = *line;
                let (t, _) = type_of(self.sh.l, words, self.sh.bools).map_err(|m| (self.line, m))?;
                let Ty::Struct(s) = &t else { return self.err(format!("offsetof is given {}, not a struct", t.describe())) };
                match layout(s).fields.iter().find(|x| x.name == *f) {
                    Some(x) => Ok(x.off),
                    None => self.err(format!("struct {s} has no field {f}")),
                }
            }
            _ => unreachable!(),
        }
    }

    /// x.name(args) on a Go struct or a pointer to one: the method of its
    /// type, given the value's address. A method that wants a pointer is
    /// given the value's own address (Go takes it), one that wants a value
    /// is given the address its copy is made from.
    fn struct_method(&mut self, base: &Expr, name: &str, args: &[Expr], line: u32, used: bool) -> Result<Ty, E> {
        let addressable = base.root().is_some() || matches!(base, Expr::Deref(..));
        let bt = self.expr(base, true)?;
        let (s, ptr) = match &bt {
            Ty::Struct(s) => (s.clone(), false),
            Ty::Ptr(e, _) => match &**e {
                Ty::Struct(s) => (s.clone(), true),
                _ => return self.err(format!("{} has no method {name}", bt.describe())),
            },
            t => return self.err(format!("{} has no method {name} in Strike", t.describe())),
        };
        let key = format!("{s}.{name}");
        let Some(sig) = self.sh.sigs.get(&key).cloned() else {
            return self.err(format!("{s} has no method {name}"));
        };
        match (matches!(sig.params[0], Ty::Ptr(..)), ptr) {
            (true, false) if !addressable => return self.err(format!("cannot call pointer method {name} on {s}: the value has no address")),
            // the value a pointer points at: the method copies it
            (false, true) => self.nil_check(),
            _ => {}
        }
        let r = self.hidden();
        self.emit(Op::Store(r));
        let t = sig.params[0].clone();
        self.call_sig(&key, &sig, Some((r, t)), args, line, used)
    }

    /// A call of the function `name` (its signature sig): the struct it
    /// returns gets the caller's memory first, then the value it is called
    /// on (a method's, in slot), then the arguments, each converted to its
    /// parameter's type.
    fn call_sig(&mut self, name: &str, sig: &Sig, first: Option<(u32, Ty)>, args: &[Expr], line: u32, used: bool) -> Result<Ty, E> {
        let skip = first.is_some() as usize;
        if sig.params.len() != args.len() + skip {
            return self.err(format!("{name} takes {} values, given {}", sig.params.len() - skip, args.len()));
        }
        let sret = matches!(sig.ret, Ty::Struct(_));
        if sret {
            self.memory_for(&sig.ret, true)?;
        }
        let mut tys = Vec::new();
        if let Some((slot, t)) = first {
            self.emit(Op::Load(slot));
            tys.push(t);
        }
        let (pw, rw) = self.sh.fw.get(name).cloned().unwrap_or_default();
        for (k, a) in args.iter().enumerate() {
            let i = k + skip;
            // a Go array or a struct is copied by the function it is given to
            let t = self.expr(a, true)?;
            let want = sig.params[i].clone();
            if !want.scalar() || !t.scalar() {
                if unify(&want, &t).is_none() && !decays(&t, &want) {
                    return self.err(format!("{name}'s parameter {} is {}, given {}", k + 1, want.describe(), t.describe()));
                }
                if let (Expr::Var(n, _), Ty::List(_)) = (a, &t) {
                    let n = n.clone();
                    self.refine(&n, &want)?;
                }
            } else {
                // an argument is converted to its parameter's type
                let from = self.last_w;
                let w = pw.get(i).copied().unwrap_or(64);
                if want == Ty::Float || t == Ty::Float {
                    self.convert(&want, w, &t, from)?;
                } else {
                    self.narrow(w, from)?;
                }
            }
            tys.push(t);
        }
        let seen = self.sh.args.entry(name.to_string()).or_insert_with(|| vec![Vec::new(); sig.params.len()]);
        for (i, t) in tys.into_iter().enumerate() {
            seen[i].push(t);
        }
        if used {
            self.sh.used.push((name.to_string(), line));
        }
        self.emit(Op::Call(symbol(name), (sig.params.len() + sret as usize) as u32));
        self.last_w = if rw == 0 || !sig.ret.scalar() { 64 } else { rw };
        Ok(sig.ret.clone())
    }

    /// The struct printer of Go's fmt: {1 2 hi}, and {X:1 Y:2 Name:hi} for
    /// %+v; a pointer in a struct prints as an address, which Strike cannot
    /// give as Go gives it, so it is refused.
    fn struct_printer(&mut self, s: &str, plus: bool) -> Result<String, E> {
        let name = format!("lib.print.{s}{}", if plus { "+" } else { "" });
        if self.sh.printers.contains_key(&name) {
            return Ok(name);
        }
        let mut fields = Vec::new();
        for f in layout(s).fields.iter() {
            let elem = match &f.ty {
                Ty::Bool => Elem::Bool,
                Ty::Float => Elem::Float(FloatElem { f32: f.w == 32, repr: false, buf: self.float_buf() }),
                Ty::Str => Elem::Str(false),
                Ty::Struct(inner) => {
                    self.printed_plainly(inner)?;
                    Elem::Nested(self.struct_printer(inner, plus)?, true)
                }
                Ty::Slice(..) => Elem::Nested(self.printer_plus(&f.ty, plus)?, false),
                Ty::Arr(..) => Elem::Nested(self.printer_plus(&f.ty, plus)?, true),
                Ty::Ptr(..) => return self.err(format!("struct {s} holds a pointer ({}), which Go prints as its address; Strike cannot print that as Go does", f.name)),
                _ => Elem::Int,
            };
            let label = if plus { format!("{}:", f.name).into_bytes() } else { Vec::new() };
            fields.push((f.off, elem, size(&f.ty, f.w) as u8, label));
        }
        let p = Printer {
            array: None,
            list: false,
            elem: Elem::Int,
            stride: 0,
            width: 0,
            open: b"{".to_vec(),
            sep: b" ".to_vec(),
            close: b"}".to_vec(),
            words: self.sh.l.bool_print.clone().map(|(a, b)| (a.into_bytes(), b.into_bytes())),
            fields: Some(fields),
        };
        self.sh.printers.insert(name.clone(), p);
        Ok(name)
    }

    /// Go's fmt prints a value through its String method (or Error,
    /// Format, GoString) where its type has one: refused where Strike does
    /// not do it as fmt does (inside another value, or the others).
    fn printed_plainly(&self, s: &str) -> Result<(), E> {
        for m in ["String", "Error", "Format", "GoString"] {
            if self.sh.sigs.contains_key(&format!("{s}.{m}")) {
                return self.err(format!("{s} has a {m} method, which Go's fmt calls; Strike prints it so only as the value printed itself (String with a value receiver)"));
            }
        }
        Ok(())
    }

    /// Prints a value as print, Println or %v does; plus: %+v (a struct's
    /// fields by name).
    fn print_struct(&mut self, t: &Ty, plus: bool) -> Result<(), E> {
        let s = match t {
            Ty::Struct(s) => s.clone(),
            Ty::Ptr(e, _) => match &**e {
                Ty::Struct(s) => s.clone(),
                _ => return self.err("a pointer is not printed"),
            },
            _ => unreachable!(),
        };
        for m in ["Error", "Format", "GoString"] {
            if self.sh.sigs.contains_key(&format!("{s}.{m}")) {
                return self.err(format!("{s} has a {m} method, which Go's fmt calls; that is not in Strike yet"));
            }
        }
        // a String method: what fmt prints (a value's only when it takes a
        // value; a nil pointer's <nil>, as fmt gives when it would panic)
        if let Some(sig) = self.sh.sigs.get(&format!("{s}.String")).cloned() {
            let by_ptr = matches!(sig.params[0], Ty::Ptr(..));
            let ptr = matches!(t, Ty::Ptr(..));
            if ptr && by_ptr {
                return self.err(format!("{s}'s String takes a pointer, and fmt gives a nil one <nil> only where String panics; that is not in Strike yet"));
            }
            if !by_ptr {
                let (h, end) = (self.hidden(), self.label());
                self.emit(Op::Store(h));
                if ptr {
                    let some = self.label();
                    self.ops(&[Op::Load(h), Op::Jnz(some), Op::PrintStr(b"<nil>".to_vec()), Op::Jmp(end), Op::Label(some)]);
                }
                let line = self.line;
                let r = self.call_sig(&format!("{s}.String"), &sig, Some((h, sig.params[0].clone())), &[], line, true)?;
                if r != Ty::Str {
                    return self.err(format!("{s}'s String returns {}, not a string", r.describe()));
                }
                self.lib("lib.str_print", 1);
                self.ops(&[Op::Pop, Op::Label(end)]);
                return Ok(());
            }
        }
        match t {
            Ty::Struct(s) => {
                let f = self.struct_printer(s, plus)?;
                self.emit(Op::Call(f, 1));
                self.emit(Op::Pop);
            }
            // a pointer to a struct: &{1 2}, or <nil>
            Ty::Ptr(e, _) => {
                let Ty::Struct(s) = &**e else { return self.err("a pointer is not printed") };
                let f = self.struct_printer(s, plus)?;
                let (h, some, end) = (self.hidden(), self.label(), self.label());
                self.ops(&[Op::Store(h), Op::Load(h), Op::Jnz(some), Op::PrintStr(b"<nil>".to_vec()), Op::Jmp(end), Op::Label(some), Op::PrintStr(b"&".to_vec()), Op::Load(h)]);
                self.emit(Op::Call(f, 1));
                self.ops(&[Op::Pop, Op::Label(end)]);
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    // Python's classes

    fn class(&self, c: &str) -> Option<&Class> {
        self.sh.classes.get(c)
    }
    /// The class and its bases, nearest first.
    fn chain(&self, c: &str) -> Vec<String> {
        let mut out = vec![c.to_string()];
        while let Some(b) = self.class(out.last().unwrap()).and_then(|k| k.base.clone()) {
            out.push(b);
        }
        out
    }
    /// The class whose method m an object of class c has (c's own, or a base's).
    fn resolve(&self, c: &str, m: &str) -> Option<String> {
        self.chain(c).into_iter().find(|k| self.class(k).is_some_and(|x| x.methods.contains(m)))
    }
    /// The class of c's chain whose own attribute a is (the global it is).
    fn class_attr(&self, c: &str, a: &str) -> Option<String> {
        self.chain(c).into_iter().find(|k| self.class(k).is_some_and(|x| x.class_attrs.iter().any(|y| y == a)))
    }
    /// The attributes an object of class c has a word for: those its
    /// class's methods set, then those set elsewhere (learned as kinds are,
    /// round by round), each at 8 * (1 + its place).
    fn obj_attrs(&self, c: &str) -> Vec<String> {
        let mut v = self.class(c).map(|k| k.attrs.clone()).unwrap_or_default();
        let key = format!("attrs {c}");
        let mut extra: Vec<&String> = self.sh.known.keys().filter(|k| k.0 == key).map(|k| &k.1).collect();
        extra.sort();
        for e in extra {
            if !v.contains(e) {
                v.push(e.clone());
            }
        }
        v
    }
    fn attr_key(c: &str, a: &str) -> (String, String) {
        (format!("class {c}"), a.to_string())
    }
    /// What an object's attribute is known to hold so far.
    fn attr_kind(&self, c: &str, a: &str) -> Ty {
        let k = Self::attr_key(c, a);
        match (self.sh.seen.get(&k), self.sh.known.get(&k)) {
            (Some(x), Some(y)) => unify(x, y).unwrap_or(x.clone()),
            (Some(x), None) | (None, Some(x)) => x.clone(),
            (None, None) => Ty::Unknown,
        }
    }

    /// P(args): a new object, its attributes not set yet, given to the
    /// class's __init__ (its own, or a base's).
    fn construct(&mut self, c: &str, args: &[Expr], line: u32) -> Result<Ty, E> {
        let n = self.obj_attrs(c).len();
        if n > MAX_ATTRS {
            return self.err(format!("{c}'s objects have more than {MAX_ATTRS} attributes, which is not in Strike yet"));
        }
        let h = self.hidden();
        self.emit(Op::Push(8 * (1 + n as i64)));
        self.lib("lib.bytes", 1);
        self.emit(Op::Store(h));
        match self.resolve(c, "__init__") {
            Some(d) => {
                let f = instance(c, &d, "__init__");
                let sig = self.sh.sigs[&f].clone();
                self.call_sig(&f, &sig, Some((h, Ty::Obj(c.to_string()))), args, line, false)?;
                self.emit(Op::Pop);
            }
            None if args.is_empty() => {}
            None => return self.err(format!("{c}() takes no arguments")),
        }
        self.emit(Op::Load(h));
        self.last_w = 64;
        Ok(Ty::Obj(c.to_string()))
    }

    /// obj.name(args), and super().name(args) in a method: the method the
    /// object's class has, given the object first.
    fn obj_method(&mut self, base: &Expr, name: &str, args: &[Expr], line: u32, used: bool) -> Result<Option<Ty>, E> {
        if let Expr::Call(s, a, _) = base {
            if s == "super" && a.is_empty() && !self.sh.sigs.contains_key("super") {
                let Some((r, d)) = self.class_ctx.clone() else { return self.err("super() is used outside a class's method") };
                let Some(b) = self.class(&d).and_then(|k| k.base.clone()) else {
                    return self.err(format!("{d} has no base class: super().{name}() is object's, which is not in Strike yet"));
                };
                let Some(a) = self.resolve(&b, name) else { return self.err(format!("'super' object has no attribute '{name}'")) };
                let f = instance(&r, &a, name);
                let sig = self.sh.sigs[&f].clone();
                let me = self.hidden();
                // self: the method's first parameter
                self.ops(&[Op::Load(0), Op::Store(me)]);
                return Ok(Some(self.call_sig(&f, &sig, Some((me, Ty::Obj(r.clone()))), args, line, used)?));
            }
        }
        if let Expr::Var(n, _) = base {
            if self.class(n).is_some() && self.lookup(n).is_none() {
                return self.err(format!("{n}.{name}(): a method called on its class is not in Strike yet"));
            }
        }
        let start = self.code.len();
        let (code_at, slots, labels) = (start, self.slots, self.labels);
        let t = self.expr(base, true)?;
        let Ty::Obj(c) = t else {
            // not an object: the base is lowered again as a list's or a string's
            self.code.truncate(code_at);
            (self.slots, self.labels) = (slots, labels);
            return Ok(None);
        };
        let Some(d) = self.resolve(&c, name) else {
            return self.err(format!("'{c}' object has no method {name} in Strike"));
        };
        let f = instance(&c, &d, name);
        let sig = self.sh.sigs[&f].clone();
        let r = self.hidden();
        self.emit(Op::Store(r));
        Ok(Some(self.call_sig(&f, &sig, Some((r, Ty::Obj(c))), args, line, used)?))
    }

    /// The text of the object on top, by its class's __str__ (or __repr__
    /// where it has none; repr: only __repr__, as a list shows its
    /// elements): a string.
    fn obj_text(&mut self, c: &str, repr: bool) -> Result<(), E> {
        let d = if repr { None } else { self.resolve(c, "__str__").map(|d| (d, "__str__")) };
        let Some((d, m)) = d.or(self.resolve(c, "__repr__").map(|d| (d, "__repr__"))) else {
            return self.err(format!(
                "an object of class {c} without {} prints as <__main__.{c} object at 0x...>, an address Strike cannot give as Python does",
                if repr { "__repr__" } else { "__str__ or __repr__" }
            ));
        };
        let f = instance(c, &d, m);
        let sig = self.sh.sigs[&f].clone();
        let h = self.hidden();
        self.emit(Op::Store(h));
        let t = self.call_sig(&f, &sig, Some((h, Ty::Obj(c.to_string()))), &[], self.line, true)?;
        // not known yet: the method's own lowering says why, or a later round
        if t != Ty::Str && t != Ty::Unknown {
            return self.err(format!("{m} returned {}, not a string (TypeError)", t.describe()));
        }
        Ok(())
    }

    /// obj.a, or the class's own attribute (P.a): the object's where it is
    /// set, else its class's, else Python's AttributeError.
    fn attr_read(&mut self, base: &Expr, a: &str) -> Result<Ty, E> {
        if let Expr::Var(n, _) = base {
            if self.class(n).is_some() && self.lookup(n).is_none() {
                let Some(k) = self.class_attr(n, a) else {
                    return self.err(format!("type object '{n}' has no attribute '{a}'"));
                };
                let g = format!("{k}::{a}");
                let Some((slot, t)) = self.lookup(&g) else { return self.err(format!("{n}.{a} is read before the class is made")) };
                self.load(slot);
                self.last_w = 64;
                return Ok(t);
            }
        }
        let t = self.expr(base, true)?;
        let Ty::Obj(c) = t else { return self.err(format!("{} has no attribute {a} in Strike", t.describe())) };
        let attrs = self.obj_attrs(&c);
        let owner = self.class_attr(&c, a);
        let missing = format!("AttributeError: '{c}' object has no attribute '{a}'\n").into_bytes();
        let h = self.hidden();
        self.emit(Op::Store(h));
        self.last_w = 64;
        let Some(i) = attrs.iter().position(|x| x == a) else {
            // an attribute no method sets: the class's own, or none
            return match owner {
                Some(k) => {
                    let (slot, t) = self.lookup(&format!("{k}::{a}")).ok_or((self.line, format!("{c}.{a} is read before the class is made")))?;
                    self.load(slot);
                    Ok(t)
                }
                None => {
                    self.ops(&[Op::Fail(1, missing), Op::Push(0)]);
                    Ok(Ty::Unknown)
                }
            };
        };
        let (have, done) = (self.label(), self.label());
        self.ops(&[Op::Load(h), Op::LoadM(8), Op::Push(1 << i), Op::Bin(BinOp::Band), Op::Jnz(have)]);
        let mut t = self.attr_kind(&c, a);
        match &owner {
            Some(k) => {
                let (slot, ct) = self.lookup(&format!("{k}::{a}")).ok_or((self.line, format!("{c}.{a} is read before the class is made")))?;
                t = unify(&t, &ct).ok_or((self.line, format!("{a} holds {} in the class and {} in its objects; Strike keeps one kind per attribute", ct.describe(), t.describe())))?;
                self.load(slot);
                self.emit(Op::Jmp(done));
            }
            None => self.emit(Op::Fail(1, missing)),
        }
        self.ops(&[Op::Label(have), Op::Load(h), Op::Push(8 * (i as i64 + 1)), Op::Bin(BinOp::Add), Op::LoadM(8), Op::Label(done)]);
        self.last_w = 64;
        Ok(t)
    }

    /// obj.a = value (or op=), P.a = value: the attribute set, its kind
    /// learned as a list's elements are.
    fn attr_set(&mut self, base: &Expr, a: &str, op: Option<BinOp>, value: &Expr) -> Result<(), E> {
        if let Expr::Var(n, line) = base {
            if self.class(n).is_some() && self.lookup(n).is_none() {
                if !self.class(n).is_some_and(|k| k.class_attrs.iter().any(|x| x == a)) {
                    return self.err(format!("{n}.{a}: an attribute a class does not make itself is not given to it in Strike"));
                }
                let g = format!("{n}::{a}");
                let v = match op {
                    None => value.clone(),
                    Some(op) => Expr::Bin(op, Box::new(Expr::Field(base.clone().into(), a.to_string(), *line)), Box::new(value.clone())),
                };
                let _ = line;
                return self.assign(&g, &v);
            }
        }
        // the value first, then the object (Python's order)
        let (vt, v) = match op {
            None => {
                let vt = self.value(value)?;
                let v = self.hidden();
                self.emit(Op::Store(v));
                (vt, v)
            }
            Some(op) => {
                let bt = self.attr_read(base, a)?;
                let at = self.code.len();
                let w = self.last_w;
                let vt = self.binary(op, bt, w, None, at, value, false)?;
                let v = self.hidden();
                self.emit(Op::Store(v));
                (vt, v)
            }
        };
        let t = self.expr(base, true)?;
        let Ty::Obj(c) = t else { return self.err(format!("{} has no attribute {a} in Strike", t.describe())) };
        let attrs = self.obj_attrs(&c);
        // an attribute given outside the class's methods: the class's
        // objects get a word for it from the next round on
        if !self.class(&c).is_some_and(|k| k.attrs.iter().any(|x| x == a)) {
            self.saw((format!("attrs {c}"), a.to_string()), Ty::Int);
        }
        let Some(i) = attrs.iter().position(|x| x == a) else {
            return self.err(format!("{a} is given to an object of class {c}, which has no word for it yet"));
        };
        let was = self.attr_kind(&c, a);
        let Some(now) = unify(&was, &vt) else {
            return self.err(format!("the attribute {a} holds {} in one place and {} in another; Strike keeps one kind per attribute", was.describe(), vt.describe()));
        };
        if let Some(k) = self.class_attr(&c, a) {
            let (_, ct) = self.lookup(&format!("{k}::{a}")).unwrap_or((0, Ty::Unknown));
            if unify(&ct, &now).is_none() {
                return self.err(format!("{a} holds {} in the class and {} in its objects; Strike keeps one kind per attribute", ct.describe(), now.describe()));
            }
        }
        self.saw(Self::attr_key(&c, a), now);
        let h = self.hidden();
        self.emit(Op::Store(h));
        self.ops(&[Op::Load(h), Op::Push(8 * (i as i64 + 1)), Op::Bin(BinOp::Add), Op::Load(v), Op::StoreM(8)]);
        self.ops(&[Op::Load(h), Op::Load(h), Op::LoadM(8), Op::Push(1 << i), Op::Bin(BinOp::Bor), Op::StoreM(8)]);
        Ok(())
    }

    /// Python's class statement: the class's own attributes given their
    /// values, as globals of the program (P::a).
    fn class_stmt(&mut self, c: &str) -> Result<(), E> {
        let Some(def) = self.sh.defs.get(c).cloned() else { return Ok(()) };
        // a name the class's body has given so far is the class's own there
        // (b = a + 1 reads the class's a), others are the module's
        fn own(e: &Expr, c: &str, given: &HashSet<String>) -> Result<Expr, (u32, String)> {
            Ok(match e {
                Expr::Var(n, l) if given.contains(n) => Expr::Var(format!("{c}::{n}"), *l),
                Expr::Num(..) | Expr::Float(_) | Expr::Str(_) | Expr::Var(..) => e.clone(),
                Expr::Un(op, x) => Expr::Un(*op, Box::new(own(x, c, given)?)),
                Expr::Bin(op, a, b) => Expr::Bin(*op, Box::new(own(a, c, given)?), Box::new(own(b, c, given)?)),
                Expr::List(xs, l) => Expr::List(xs.iter().map(|x| own(x, c, given)).collect::<Result<_, _>>()?, *l),
                Expr::Call(f, xs, l) if !given.contains(f) => Expr::Call(f.clone(), xs.iter().map(|x| own(x, c, given)).collect::<Result<_, _>>()?, *l),
                _ => return Err((0, format!("this value in the body of class {c} is not in Strike yet (a number, a string, a list, or arithmetic of them and of names, is)"))),
            })
        }
        let mut given = HashSet::new();
        for s in &def.attrs {
            match s {
                Stmt::Assign { name, index, op: None, value, line } if index.is_empty() => {
                    self.line = *line;
                    let v = own(value, c, &given).map_err(|(_, m)| (*line, m))?;
                    self.assign(&format!("{c}::{name}"), &v)?;
                    given.insert(name.clone());
                }
                Stmt::Nothing => {}
                Stmt::Expr(Expr::Str(_)) => {}
                _ => return self.err(format!("only methods and attributes (name = value) are in a class's body in Strike ({c})")),
            }
        }
        Ok(())
    }
}

fn combine(what: &str, tys: &[Ty], bools: bool) -> Result<Ty, String> {
    // a value not known yet (a parameter passed on before its own kind is
    // learned) says nothing: the next round tells
    if !tys.is_empty() && tys.iter().all(|t| *t == Ty::Unknown) {
        return Ok(Ty::Unknown);
    }
    if tys.iter().any(|t| !t.scalar()) {
        let mut u = Ty::Unknown;
        for t in tys {
            u = unify(&u, t).ok_or(format!("{what} is {} in one place and {} in another", u.describe(), t.describe()))?;
        }
        return Ok(u);
    }
    let has = |t: &Ty| tys.contains(t);
    if has(&Ty::Float) {
        if has(&Ty::Int) || has(&Ty::Bool) {
            return Err(format!("{what} is a float in one place and a whole number in another"));
        }
        return Ok(Ty::Float);
    }
    Ok(match (has(&Ty::Int), has(&Ty::Bool)) {
        (_, false) => Ty::Int,
        (false, true) => Ty::Bool,
        (true, true) if bools => return Err(format!("{what} is a number in one place and a truth value in another")),
        (true, true) => Ty::Int,
    })
}

/// Python's classes, each method as a function for every class it is
/// called on (its own, and each class below it), and where each was
/// written.
#[allow(clippy::type_complexity)]
fn class_table(p: &Program, l: &Lang) -> Result<(HashMap<String, Class>, HashMap<String, TypeDef>, Vec<FuncDef>, HashMap<String, (String, String)>), E> {
    let (mut cs, mut defs) = (HashMap::new(), HashMap::new());
    let (mut funcs, mut ctx) = (Vec::new(), HashMap::new());
    if !l.records.as_ref().is_some_and(|r| r.classes) {
        return Ok((cs, defs, funcs, ctx));
    }
    let mut order = Vec::new();
    for t in &p.types {
        let name = t.name.clone().unwrap_or_default();
        if cs.contains_key(&name) {
            return Err((t.line, format!("the class {name} is defined twice; Strike keeps one class of a name")));
        }
        let base = t.base.clone().filter(|b| b != "object");
        if let Some(b) = &base {
            if !cs.contains_key(b) {
                return Err((t.line, format!("{name}'s base {b} is not a class defined before it in this program")));
            }
        }
        let mut methods = HashSet::new();
        for m in &t.methods {
            let short = m.name.strip_prefix(&format!("{name}.")).unwrap_or(&m.name).to_string();
            if m.params.is_empty() {
                return Err((m.line, format!("the method {short} takes no self; a static method is not in Strike yet")));
            }
            if !methods.insert(short.clone()) {
                return Err((m.line, format!("{name} defines {short} twice; Strike keeps one method of a name")));
            }
        }
        let class_attrs = t.attrs.iter().filter_map(|s| if let Stmt::Assign { name, .. } = s { Some(name.clone()) } else { None }).collect();
        cs.insert(name.clone(), Class { base, attrs: Vec::new(), methods, class_attrs });
        defs.insert(name.clone(), t.clone());
        order.push(name);
    }
    let chain = |c: &str, cs: &HashMap<String, Class>| {
        let mut out = vec![c.to_string()];
        while let Some(b) = cs[out.last().unwrap()].base.clone() {
            out.push(b);
        }
        out
    };
    for c in &order {
        let mut attrs = Vec::new();
        for d in chain(c, &cs).iter().rev() {
            for m in &defs[d].methods {
                self_attrs(m, &mut attrs);
            }
        }
        if attrs.len() > MAX_ATTRS {
            return Err((defs[c].line, format!("{c}'s objects have more than {MAX_ATTRS} attributes, which is not in Strike yet")));
        }
        cs.get_mut(c).unwrap().attrs = attrs;
        for d in chain(c, &cs) {
            for m in &defs[&d].methods {
                let short = m.name.strip_prefix(&format!("{d}.")).unwrap_or(&m.name);
                let name = instance(c, &d, short);
                ctx.insert(name.clone(), (c.clone(), d.clone()));
                funcs.push(FuncDef { name, method_of: Some(c.clone()), ..m.clone() });
            }
        }
    }
    Ok((cs, defs, funcs, ctx))
}

pub fn lower(p: &Program, l: &Lang) -> Result<Lowered, E> {
    // the program's structs, laid out before anything is lowered
    let _types = install(lay_out(p, l)?);
    let (classes, defs, methods, class_ctx) = class_table(p, l)?;
    let all: Vec<&FuncDef> = p.funcs.iter().chain(methods.iter()).collect();
    // Go's methods that want a pointer: a value they are called on has its
    // address taken
    let ptr_methods: HashSet<String> = all
        .iter()
        .filter(|f| f.method_of.is_some() && f.param_types.first().is_some_and(|t| t.first().map(String::as_str) == Some("*")))
        .map(|f| f.name.rsplit('.').next().unwrap_or("").to_string())
        .collect();
    let mut table: HashMap<String, &FuncDef> = HashMap::new();
    for &f in &all {
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
    for &f in &all {
        let mut ps = Vec::new();
        let mut kinds = Vec::new();
        for t in &f.param_types {
            let (k, w) = type_of(l, t, bools).map_err(|m| (f.line, m))?;
            ps.push(w);
            kinds.push((typed && (!k.scalar() || k == Ty::Float)).then_some(k));
        }
        // a method's first parameter: the object, or Go's receiver (a
        // struct of the method's type, or a pointer to one)
        if let Some(of) = &f.method_of {
            if classes.contains_key(of) {
                kinds[0] = Some(Ty::Obj(of.clone()));
            } else {
                let s = Ty::Struct(of.clone());
                if !matches!(&kinds[0], Some(k) if *k == s || matches!(k, Ty::Ptr(e, _) if **e == s)) {
                    return Err((f.line, format!("a method of {of}, which is not a struct of this package, is not in Strike yet")));
                }
            }
        }
        let (rk, rw) = type_of(l, &f.ret, bools).map_err(|m| (f.line, m))?;
        if matches!(&rk, Ty::Ptr(e, _) if !matches!(**e, Ty::Struct(_))) {
            return Err((f.line, "a function that returns a pointer (but to a struct) is not in Strike yet".into()));
        }
        fw.insert(f.name.clone(), (ps, rw));
        declared.insert(f.name.clone(), (kinds, (typed && (!rk.scalar() || rk == Ty::Float)).then_some(rk)));
    }
    let mut sigs: HashMap<String, Sig> = all
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
            float_buf: None,
            classes: &classes,
            defs: &defs,
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
        bodies.extend(all.iter().map(|f| {
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
                last_k: None,
                ret_w: if top { 64 } else { ret_w },
                ret_ty,
                globals_here: top,
                global_names: Default::default(),
                assigned: if top { Default::default() } else { assigned_names(body) },
                frame: 0,
                sliced: sliced_names(body),
                sret: !top && matches!(declared.get(&name), Some((_, Some(Ty::Struct(_))))),
                escapes: if l.records.is_some() && l.index_error.is_some() { escaping_names(body, &ptr_methods) } else { HashSet::new() },
                class_ctx: class_ctx.get(&name).cloned(),
            };
            // a struct returned: the caller's memory for it comes first
            if fx.sret {
                fx.slots = 1;
            }
            let mut arrays = Vec::new();
            for (i, (n, t)) in params.iter().enumerate() {
                let s = fx.declare(n, t.clone());
                fx.set_width(s, pw.get(i).copied().unwrap_or(64));
                if t.in_place() {
                    arrays.push((s, t.clone(), n.clone()));
                }
            }
            let done = (|| -> Result<(), E> {
                // a Go array or a struct parameter is the function's own copy
                for (s, t, n) in arrays {
                    let own = !fx.sliced.contains(&n) && !fx.escapes.contains(&n);
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
                params.len() as u32 + fx.sret as u32,
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
            // a typed language's result is a whole number where no float is
            // declared: a float returned is converted to it
            let tys: Vec<Ty> = rs.iter().map(|r| if typed && r.0 == Ty::Float { Ty::Int } else { r.0.clone() }).collect();
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
                let tys: Vec<Ty> = tys.iter().map(|t| if typed && *t == Ty::Float { Ty::Int } else { t.clone() }).collect();
                next.get_mut(callee).unwrap().params[i] = combine(&what, &tys, bools).map_err(|m| (line, m))?;
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
