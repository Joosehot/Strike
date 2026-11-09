//! Tokens to the universal syntax, by the language's forms: at every
//! statement each form is tried in the description's order, and the first
//! whose whole pattern matches is the statement. Expressions are read by one
//! precedence reader with the language's operator table.

use super::ir::{BinOp, UnOp};
use super::lang::{Cap, End, Form, Kind, Lang, Pat, Place, Role, Style};
use super::lex::{FloatLit, Tok, T};

#[derive(Clone, Debug)]
pub enum Expr {
    /// a number; true when it is a truth value (True, false)
    Num(i64, bool),
    Float(FloatLit),
    /// (type) value: C's cast, the type's words
    Cast(Vec<String>, Box<Expr>, u32),
    Str(Vec<u8>),
    Var(String, u32),
    Call(String, Vec<Expr>, u32),
    Un(UnOp, Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    /// a[i]
    Index(Box<Expr>, Box<Expr>, u32),
    /// a[i:j], either side left out
    Slice(Box<Expr>, Option<Box<Expr>>, Option<Box<Expr>>, u32),
    /// a.name(args)
    Method(Box<Expr>, String, Vec<Expr>, u32),
    /// Python's [a, b]
    List(Vec<Expr>, u32),
    /// Python's [elem for name in iter if cond]
    Comp { elem: Box<Expr>, name: String, iter: Box<Expr>, cond: Option<Box<Expr>>, line: u32 },
    /// {a, b}: C's initializer, an element of a Go composite whose type is
    /// the composite's own
    Brace(Vec<Expr>, u32),
    /// Go's []int{a, b}: the type's words and the elements
    Composite(Vec<String>, Vec<Expr>, u32),
    /// a type given as a value (Go's make([]int, n))
    Type(Vec<String>, u32),
    /// a.name: a struct's field, an object's attribute
    Field(Box<Expr>, String, u32),
    /// &x
    Addr(Box<Expr>, u32),
    /// *p (C's p->x is (*p).x)
    Deref(Box<Expr>, u32),
    /// an element given to a field by its name: C's {.x = 1}, Go's P{X: 1}
    Key(String, Box<Expr>, u32),
    /// sizeof(type): the type's words
    SizeOfType(Vec<String>, u32),
    /// sizeof x
    SizeOf(Box<Expr>, u32),
    /// offsetof(type, field)
    OffsetOf(Vec<String>, String, u32),
}

impl Expr {
    /// The name a chain of fields and elements begins with (p.q[i].x is
    /// p's), if it begins with a name.
    pub fn root(&self) -> Option<&str> {
        match self {
            Expr::Var(n, _) => Some(n),
            Expr::Field(b, _, _) | Expr::Index(b, _, _) | Expr::Slice(b, _, _, _) | Expr::Deref(b, _) => b.root(),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Stmt {
    /// ty: the type words written (none where the language writes none);
    /// dims: C's array sizes after the name
    Var { name: String, ty: Vec<String>, dims: Vec<Option<Expr>>, value: Option<Expr>, line: u32 },
    /// name, or an element of it (index: the subscripts in order)
    Assign { name: String, index: Vec<Expr>, op: Option<BinOp>, value: Expr, line: u32 },
    /// a place that is not a name with indexes: a field (p.x, p->a[i],
    /// ps[i].x), or what a pointer points at (*p)
    Set { target: Expr, op: Option<BinOp>, value: Expr, line: u32 },
    /// Python's class statement: the class (Program.types) is made here,
    /// its attributes given their values
    Class(String, u32),
    /// for over the elements: their index, their value, or both
    ForEach { index: Option<String>, value: Option<String>, iter: Expr, body: Vec<Stmt>, line: u32 },
    If { cond: Expr, then: Vec<Stmt>, els: Vec<Stmt> },
    /// no condition: for ever
    While { cond: Option<Expr>, body: Vec<Stmt> },
    For { init: Box<Stmt>, cond: Expr, step: Box<Stmt>, body: Vec<Stmt> },
    ForRange { name: String, args: Vec<Expr>, body: Vec<Stmt>, line: u32 },
    Return(Option<Expr>, u32),
    Break(u32),
    Continue(u32),
    Print { style: Style, args: Vec<Expr>, line: u32 },
    Global(String, u32),
    /// a whole number read into name; fmt is C's scanf format
    Read { name: String, fmt: Option<Vec<u8>>, line: u32 },
    Expr(Expr),
    Nothing,
}

#[derive(Clone, Debug)]
pub struct FuncDef {
    pub name: String,
    pub params: Vec<String>,
    /// each parameter's type words, and the result's
    pub param_types: Vec<Vec<String>>,
    pub ret: Vec<String>,
    pub body: Vec<Stmt>,
    pub line: u32,
    /// a method: the type whose method it is (its first parameter is the
    /// value it is called on; its name is Type.name)
    pub method_of: Option<String>,
}

/// A field of a struct: its name, its type's words, C's sizes after it.
#[derive(Clone, Debug)]
pub struct FieldDef {
    pub name: String,
    pub ty: Vec<String>,
    pub dims: Vec<Option<Expr>>,
    pub line: u32,
}

/// A struct (C, Go) or a class (Python).
#[derive(Clone, Debug)]
pub struct TypeDef {
    /// the name it is written by alone (Go's, Python's, C's typedef)
    pub name: Option<String>,
    /// C's struct tag: written after `struct`
    pub tag: Option<String>,
    /// Python's base class
    pub base: Option<String>,
    pub fields: Vec<FieldDef>,
    /// Python: the methods and the class's own attributes (name = value)
    pub methods: Vec<FuncDef>,
    pub attrs: Vec<Stmt>,
    pub line: u32,
}

/// One module or package an import names, and the name it is given here.
#[derive(Clone, Debug)]
pub struct Spec {
    pub path: String,
    pub alias: Option<String>,
    pub line: u32,
}

/// import m, import "mod/pkg", from m import a: where it stands among the
/// top-level statements (`at`: how many come before it).
#[derive(Clone, Debug)]
pub struct Import {
    /// Python's from m: the module the names are taken from (the specs are
    /// the names; none: `from m import *`)
    pub from: Option<String>,
    pub specs: Vec<Spec>,
    pub line: u32,
    pub at: usize,
}

#[derive(Default, Debug)]
pub struct Program {
    pub funcs: Vec<FuncDef>,
    pub top: Vec<Stmt>,
    pub imports: Vec<Import>,
    /// Go's package clause
    pub package: Option<(String, u32)>,
    /// the structs and classes, and C's second names for them
    pub types: Vec<TypeDef>,
    pub aliases: Vec<(String, String, u32)>,
}

/// Why a file could not be read: where the reading got furthest, and what
/// it wanted there.
pub struct Failure {
    pub line: u32,
    pub message: String,
    /// how far into the tokens the reading got (0 to 1)
    pub reach: f64,
}

struct Miss;
type R<T> = Result<T, Miss>;

#[derive(Default)]
struct Caps {
    exprs: Vec<Expr>,
    bodies: Vec<Vec<Stmt>>,
    names: Vec<String>,
    params: Vec<(Vec<String>, Vec<Vec<String>>)>,
    args: Vec<Vec<Expr>>,
    simples: Vec<Stmt>,
    strs: Vec<Vec<u8>>,
    types: Vec<Vec<String>>,
    places: Vec<Lv>,
    dims: Vec<Vec<Option<Expr>>>,
    imports: Vec<Import>,
    fields: Vec<Vec<FieldDef>>,
    members: Vec<(Vec<FuncDef>, Vec<Stmt>)>,
}

/// What an assignment changes: a name with indexes after it, or another
/// place (a field, what a pointer points at).
enum Lv {
    Name(String, Vec<Expr>),
    Target(Expr),
}

impl Caps {
    fn mark(&self) -> [usize; 13] {
        [
            self.exprs.len(),
            self.bodies.len(),
            self.names.len(),
            self.params.len(),
            self.args.len(),
            self.simples.len(),
            self.strs.len(),
            self.types.len(),
            self.places.len(),
            self.dims.len(),
            self.imports.len(),
            self.fields.len(),
            self.members.len(),
        ]
    }
    fn reset(&mut self, m: [usize; 13]) {
        self.fields.truncate(m[11]);
        self.members.truncate(m[12]);
        self.imports.truncate(m[10]);
        self.places.truncate(m[8]);
        self.dims.truncate(m[9]);
        self.exprs.truncate(m[0]);
        self.bodies.truncate(m[1]);
        self.names.truncate(m[2]);
        self.params.truncate(m[3]);
        self.args.truncate(m[4]);
        self.simples.truncate(m[5]);
        self.strs.truncate(m[6]);
        self.types.truncate(m[7]);
    }
}

enum Parsed {
    Stmt(Stmt),
    Func(FuncDef),
    Type(TypeDef),
    Alias(String, String, u32),
    Import(Import),
    Package(String, u32),
    Ignore,
}

#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    Top,
    Stmt,
    Simple,
}

struct P<'a> {
    t: &'a [Tok],
    i: usize,
    l: &'a Lang,
    far: usize,
    far_msg: String,
    fatal: Option<(u32, String)>,
    /// the file's own types (its structs and classes), found before it is read
    types: std::collections::HashSet<String>,
}

impl<'a> P<'a> {
    /// A word of a type: the language's, or a struct the file declares.
    fn is_type(&self, s: &str) -> bool {
        self.l.type_words.contains(s) || self.types.contains(s)
    }
    fn peek(&self) -> &T {
        &self.t[self.i].t
    }
    fn line(&self) -> u32 {
        self.t[self.i.min(self.t.len() - 1)].line
    }
    fn miss<X>(&mut self, want: &str) -> R<X> {
        if self.i > self.far || self.far_msg.is_empty() {
            self.far = self.i;
            let got = match self.peek() {
                T::Ident(s) => format!("`{s}`"),
                T::Num(n) => format!("{n}"),
                T::Float(f) => format!("{}", f64::from_bits(f.bits)),
                T::Str(_) => "a string".into(),
                T::Punct(p) => format!("`{p}`"),
                T::End => "the end of the line".into(),
                T::Indent => "a deeper indentation".into(),
                T::Dedent => "a shallower indentation".into(),
                T::Eof => "the end of the file".into(),
            };
            self.far_msg = format!("expected {want}, found {got}");
        }
        Err(Miss)
    }
    fn fatal<X>(&mut self, line: u32, m: String) -> R<X> {
        if self.fatal.is_none() {
            self.fatal = Some((line, m));
        }
        Err(Miss)
    }
    fn text(&self) -> Option<&str> {
        match self.peek() {
            T::Ident(s) | T::Punct(s) => Some(s),
            _ => None,
        }
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.text() == Some(s) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> R<()> {
        if self.eat(s) {
            Ok(())
        } else {
            self.miss(&format!("`{s}`"))
        }
    }
    fn skip_ends(&mut self) {
        while matches!(self.peek(), T::End) || (self.l.end == End::NewlineAfterValue && self.text() == Some(";")) {
            self.i += 1;
        }
    }

    fn name(&mut self) -> R<String> {
        match self.peek().clone() {
            T::Ident(s) if !self.l.keywords.contains(&s) => {
                self.i += 1;
                Ok(s)
            }
            _ => self.miss("a name"),
        }
    }

    fn end(&mut self) -> R<()> {
        match self.l.end {
            End::Semicolon => self.expect(";"),
            End::Newline => match self.peek() {
                T::End => {
                    self.i += 1;
                    Ok(())
                }
                T::Dedent | T::Eof => Ok(()),
                _ => self.miss("the end of the line"),
            },
            End::NewlineAfterValue => match self.peek() {
                T::End => {
                    self.i += 1;
                    Ok(())
                }
                T::Punct(p) if p == ";" => {
                    self.i += 1;
                    Ok(())
                }
                T::Punct(p) if p == "}" => Ok(()),
                T::Eof => Ok(()),
                _ => self.miss("the end of the statement"),
            },
        }
    }

    fn block(&mut self) -> R<Vec<Stmt>> {
        let mut out = Vec::new();
        if self.l.indent {
            if !matches!(self.peek(), T::End) {
                return self.miss("a new line and an indented block");
            }
            self.i += 1;
            if !matches!(self.peek(), T::Indent) {
                return self.miss("an indented block");
            }
            self.i += 1;
            loop {
                self.skip_ends();
                match self.peek() {
                    T::Dedent => {
                        self.i += 1;
                        return Ok(out);
                    }
                    T::Eof => return Ok(out),
                    _ => match self.statement(Ctx::Stmt)? {
                        Parsed::Stmt(s) => out.push(s),
                        Parsed::Import(i) => return self.fatal(i.line, "an import inside a function or a block is not in Strike yet".into()),
                        _ => return self.miss("a statement"),
                    },
                }
            }
        } else {
            self.expect("{")?;
            loop {
                self.skip_ends();
                if self.eat("}") {
                    return Ok(out);
                }
                match self.statement(Ctx::Stmt)? {
                    Parsed::Stmt(s) => out.push(s),
                    _ => return self.miss("a statement"),
                }
            }
        }
    }

    fn block_starts(&self) -> bool {
        if self.l.indent {
            matches!(self.peek(), T::End)
        } else {
            self.text() == Some("{")
        }
    }

    fn params(&mut self) -> R<(Vec<String>, Vec<Vec<String>>)> {
        let mut pieces: Vec<Vec<T>> = vec![Vec::new()];
        let mut depth = 0;
        loop {
            match self.peek().clone() {
                T::Punct(p) if p == ")" && depth == 0 => break,
                T::Punct(p) if p == "," && depth == 0 => pieces.push(Vec::new()),
                T::Eof => return self.miss("`)`"),
                t => {
                    if let T::Punct(p) = &t {
                        match p.as_str() {
                            "(" | "[" => depth += 1,
                            ")" | "]" => depth -= 1,
                            _ => {}
                        }
                    }
                    pieces.last_mut().unwrap().push(t);
                }
            }
            self.i += 1;
        }
        if pieces.len() == 1 && pieces[0].is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let line = self.line();
        let mut names = Vec::new();
        let mut types: Vec<Vec<String>> = Vec::new();
        for piece in &pieces {
            let mut found = Vec::new();
            let mut ty = Vec::new();
            for t in piece {
                match t {
                    T::Ident(s) if self.is_type(s) => ty.push(s.clone()),
                    T::Ident(s) if !self.l.keywords.contains(s) => found.push(s.clone()),
                    T::Punct(p) if matches!(p.as_str(), "*" | "[" | "]") => ty.push(p.clone()),
                    // an array's size, in C after the name, in Go before the type
                    T::Num(n) if ty.last().is_some_and(|w| w == "[") => ty.push(n.to_string()),
                    T::Punct(p) if p == ":" => {}
                    _ => return self.fatal(line, format!("a parameter like {piece:?} is not in Strike yet (only names and their types)")),
                }
            }
            let pick = if self.l.param_first { found.first() } else { found.last() };
            match pick {
                Some(n) => {
                    names.push(n.clone());
                    types.push(ty);
                }
                None if pieces.len() == 1 => {} // f(void)
                None => return self.miss("a parameter name"),
            }
        }
        // Go's , b int: a name with no type of its own has the next one's
        if self.l.param_first {
            for i in (0..types.len().saturating_sub(1)).rev() {
                if types[i].is_empty() {
                    types[i] = types[i + 1].clone();
                }
            }
        }
        Ok((names, types))
    }

    fn args(&mut self) -> R<Vec<Expr>> {
        let mut out = Vec::new();
        if self.text() == Some(")") {
            return Ok(out);
        }
        loop {
            out.push(self.expr(0)?.0);
            if !self.eat(",") {
                return Ok(out);
            }
        }
    }

    fn ty(&mut self) -> R<Vec<String>> {
        let start = self.i;
        // C's type begins with a word (*p = v is no declaration); Go's *P
        // begins with its star
        if !self.l.param_first && self.text() == Some("*") {
            return self.miss("a type");
        }
        let mut words = Vec::new();
        loop {
            // Go's []T and [n]T: the brackets before the element's type
            if self.l.composite_literals && self.text() == Some("[") {
                let at = self.i;
                self.i += 1;
                let size = match self.peek().clone() {
                    T::Num(n) => {
                        self.i += 1;
                        Some(n.to_string())
                    }
                    _ => None,
                };
                if !self.eat("]") {
                    self.i = at;
                    break;
                }
                words.push("[".into());
                words.extend(size);
                words.push("]".into());
                continue;
            }
            match self.text() {
                Some(s) if self.is_type(s) || s == "*" => {
                    words.push(s.to_string());
                    self.i += 1;
                }
                _ => break,
            }
        }
        if self.i == start {
            self.miss("a type")
        } else {
            Ok(words)
        }
    }

    fn seq(&mut self, pat: &[Pat], caps: &mut Caps, ctx: Ctx) -> R<()> {
        for p in pat {
            if self.fatal.is_some() {
                return Err(Miss);
            }
            match p {
                Pat::Lit(s) => self.expect(s)?,
                Pat::Cap(Cap::Name) => {
                    let n = self.name()?;
                    caps.names.push(n);
                }
                Pat::Cap(Cap::Expr) => {
                    let e = self.expr(0)?.0;
                    caps.exprs.push(e);
                }
                Pat::Cap(Cap::Block) => {
                    let b = self.block()?;
                    caps.bodies.push(b);
                }
                Pat::Cap(Cap::Body) => {
                    let b = if self.block_starts() {
                        self.block()?
                    } else {
                        match self.statement(Ctx::Stmt)? {
                            Parsed::Stmt(s) => vec![s],
                            _ => return self.miss("a statement"),
                        }
                    };
                    caps.bodies.push(b);
                }
                Pat::Cap(Cap::Params) => {
                    let v = self.params()?;
                    caps.params.push(v);
                }
                Pat::Cap(Cap::Args) => {
                    let v = self.args()?;
                    caps.args.push(v);
                }
                Pat::Cap(Cap::Type) => {
                    let w = self.ty()?;
                    caps.types.push(w);
                }
                Pat::Cap(Cap::End) => {
                    if ctx != Ctx::Simple {
                        self.end()?
                    }
                }
                Pat::Cap(Cap::Simple) => match self.statement(Ctx::Simple)? {
                    Parsed::Stmt(s) => caps.simples.push(s),
                    _ => return self.miss("a simple statement"),
                },
                Pat::Cap(Cap::Place) => {
                    let line = self.line();
                    // *p = v: what a pointer points at
                    if let Some(d) = self.l.records.as_ref().and_then(|r| r.deref.clone()) {
                        if self.eat(&d) {
                            let p = self.l.unary.values().map(|u| u.0).max().unwrap_or(0);
                            let e = self.expr(p)?.0;
                            caps.places.push(Lv::Target(Expr::Deref(Box::new(e), line)));
                            continue;
                        }
                    }
                    // (*p).x = v
                    if self.l.records.is_some() && self.text() == Some("(") {
                        let p = self.l.unary.values().map(|u| u.0).max().unwrap_or(0);
                        let e = self.expr(p)?.0;
                        if !matches!(e, Expr::Field(..) | Expr::Index(..) | Expr::Deref(..)) {
                            return self.miss("a place a value is given to");
                        }
                        caps.places.push(Lv::Target(e));
                        continue;
                    }
                    let mut n = self.name()?;
                    // pkg.X = v, m.x = v: a name of another module, or a
                    // field (p.x = v): which, the program's files say
                    while self.l.dotted() && self.text() == Some(".") && matches!(self.t.get(self.i + 1).map(|t| &t.t), Some(T::Ident(_))) {
                        self.i += 1;
                        let m = self.name()?;
                        n = format!("{n}.{m}");
                    }
                    let mut index = Vec::new();
                    while self.eat("[") {
                        index.push(self.expr(0)?.0);
                        self.expect("]")?;
                    }
                    // fields after that: ps[i].x, p->a[2]
                    let arrow = self.l.records.as_ref().and_then(|r| r.arrow.clone());
                    let field = |p: &Self| p.l.records.is_some() && (p.text() == Some(".") || (arrow.is_some() && p.text() == arrow.as_deref()));
                    if field(self) {
                        let mut e = Expr::Var(n, line);
                        for i in index {
                            e = Expr::Index(Box::new(e), Box::new(i), line);
                        }
                        loop {
                            if self.eat("[") {
                                let i = self.expr(0)?.0;
                                self.expect("]")?;
                                e = Expr::Index(Box::new(e), Box::new(i), line);
                            } else if field(self) {
                                let through = self.text() != Some(".");
                                self.i += 1;
                                let m = self.field_name()?;
                                if through {
                                    e = Expr::Deref(Box::new(e), line);
                                }
                                e = Expr::Field(Box::new(e), m, line);
                            } else {
                                break;
                            }
                        }
                        caps.places.push(Lv::Target(e));
                    } else {
                        caps.places.push(Lv::Name(n, index));
                    }
                }
                Pat::Cap(Cap::Fields) => {
                    let f = self.fields()?;
                    caps.fields.push(f);
                }
                Pat::Cap(Cap::Members) => {
                    let m = self.members()?;
                    caps.members.push(m);
                }
                Pat::Cap(Cap::Dims) => {
                    let mut dims = Vec::new();
                    while self.eat("[") {
                        if self.eat("]") {
                            dims.push(None);
                            continue;
                        }
                        dims.push(Some(self.expr(0)?.0));
                        self.expect("]")?;
                    }
                    caps.dims.push(dims);
                }
                Pat::Cap(Cap::Str) => match self.peek().clone() {
                    T::Str(s) => {
                        self.i += 1;
                        caps.strs.push(s);
                    }
                    _ => return self.miss("a string"),
                },
                Pat::Sub(name) => {
                    let l = self.l;
                    let f = l.form(name).expect("checked when the language was read");
                    let line = self.line();
                    let mut sub = Caps::default();
                    self.seq(&f.pat, &mut sub, Ctx::Stmt)?;
                    match self.build(f, sub, line) {
                        Parsed::Stmt(s) => caps.bodies.push(vec![s]),
                        Parsed::Import(i) => caps.imports.push(i),
                        _ => return self.miss("a statement"),
                    }
                }
                Pat::Opt(alts) => {
                    let (at, m) = (self.i, caps.mark());
                    for a in alts {
                        if self.seq(a, caps, ctx).is_ok() {
                            break;
                        }
                        if self.fatal.is_some() {
                            return Err(Miss);
                        }
                        self.i = at;
                        caps.reset(m);
                    }
                }
                Pat::Many(alts) => loop {
                    let (at, m) = (self.i, caps.mark());
                    let mut matched = false;
                    for a in alts {
                        if self.seq(a, caps, ctx).is_ok() && self.i > at {
                            matched = true;
                            break;
                        }
                        if self.fatal.is_some() {
                            return Err(Miss);
                        }
                        self.i = at;
                        caps.reset(m);
                    }
                    if !matched {
                        break;
                    }
                },
            }
        }
        Ok(())
    }

    /// A field's name after `.` or `->` (a word, even one the language
    /// keeps for itself elsewhere).
    fn field_name(&mut self) -> R<String> {
        match self.peek().clone() {
            T::Ident(s) => {
                self.i += 1;
                Ok(s)
            }
            _ => self.miss("a field's name"),
        }
    }

    /// A struct's fields to its closing brace: Go's `X, Y int` on lines of
    /// their own, C's `int x, *p, a[3];`.
    fn fields(&mut self) -> R<Vec<FieldDef>> {
        let mut out = Vec::new();
        loop {
            self.skip_ends();
            if self.text() == Some("}") {
                return Ok(out);
            }
            let line = self.line();
            if self.l.param_first {
                let mut names = vec![self.name()?];
                while self.eat(",") {
                    names.push(self.name()?);
                }
                let ty = self.ty()?;
                for name in names {
                    out.push(FieldDef { name, ty: ty.clone(), dims: Vec::new(), line });
                }
                self.end()?;
            } else {
                // a * belongs to the name after it, not to the type
                let mut base = self.ty()?;
                let mut stars = 0;
                while base.last().map(String::as_str) == Some("*") {
                    base.pop();
                    stars += 1;
                }
                loop {
                    let mut ty = base.clone();
                    ty.extend(std::iter::repeat_n("*".to_string(), stars));
                    stars = 0;
                    while self.eat("*") {
                        ty.push("*".into());
                    }
                    let name = self.name()?;
                    let mut dims = Vec::new();
                    while self.eat("[") {
                        dims.push(Some(self.expr(0)?.0));
                        self.expect("]")?;
                    }
                    out.push(FieldDef { name, ty, dims, line });
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(";")?;
            }
        }
    }

    /// A class's indented block: its methods and its own attributes.
    fn members(&mut self) -> R<(Vec<FuncDef>, Vec<Stmt>)> {
        if !matches!(self.peek(), T::End) {
            return self.miss("a new line and an indented block");
        }
        self.i += 1;
        if !matches!(self.peek(), T::Indent) {
            return self.miss("an indented block");
        }
        self.i += 1;
        let (mut funcs, mut stmts) = (Vec::new(), Vec::new());
        loop {
            self.skip_ends();
            match self.peek() {
                T::Dedent => {
                    self.i += 1;
                    return Ok((funcs, stmts));
                }
                T::Eof => return Ok((funcs, stmts)),
                _ => {
                    let line = self.line();
                    match self.statement(Ctx::Top)? {
                        Parsed::Func(f) => funcs.push(f),
                        Parsed::Stmt(s) => stmts.push(s),
                        Parsed::Type(_) => return self.fatal(line, "a class inside a class is not in Strike yet".into()),
                        Parsed::Import(_) => return self.fatal(line, "an import inside a class is not in Strike yet".into()),
                        _ => return self.miss("a method or an attribute"),
                    }
                }
            }
        }
    }

    fn statement(&mut self, ctx: Ctx) -> R<Parsed> {
        let l = self.l;
        let start = self.i;
        let line = self.line();
        for f in &l.forms {
            let fits = match ctx {
                Ctx::Top => f.place == Place::Top || (l.toplevel_entry && f.place == Place::Stmt),
                Ctx::Stmt => f.place == Place::Stmt,
                Ctx::Simple => f.place == Place::Stmt && matches!(f.kind, Kind::Var | Kind::Assign | Kind::Expr),
            };
            if !fits {
                continue;
            }
            let mut caps = Caps::default();
            if self.seq(&f.pat, &mut caps, ctx).is_ok() {
                return Ok(self.build(f, caps, line));
            }
            if self.fatal.is_some() {
                return Err(Miss);
            }
            self.i = start;
        }
        self.miss("a statement")
    }

    fn build(&mut self, f: &Form, mut c: Caps, line: u32) -> Parsed {
        let mut bodies = c.bodies.drain(..);
        let mut exprs = c.exprs.drain(..);
        let mut names = c.names.drain(..);
        let s = match f.kind {
            Kind::Function => {
                let (params, param_types) = c.params.pop().unwrap_or_default();
                return Parsed::Func(FuncDef {
                    name: names.next().unwrap_or_default(),
                    params,
                    param_types,
                    ret: c.types.first().cloned().unwrap_or_default(),
                    body: bodies.next().unwrap_or_default(),
                    line,
                    method_of: None,
                });
            }
            Kind::Method => {
                // func (p *P) Move(dx int): the receiver is the first parameter
                let mut ps = c.params.drain(..);
                let (recv, recv_ty) = ps.next().unwrap_or_default();
                let (params, param_types) = ps.next().unwrap_or_default();
                let of: String = recv_ty.first().map(|t| t.iter().filter(|w| *w != "*").cloned().collect::<Vec<_>>().join(" ")).unwrap_or_default();
                let name = names.next().unwrap_or_default();
                return Parsed::Func(FuncDef {
                    name: format!("{of}.{name}"),
                    params: recv.into_iter().chain(params).collect(),
                    param_types: recv_ty.into_iter().chain(param_types).collect(),
                    ret: c.types.first().cloned().unwrap_or_default(),
                    body: bodies.next().unwrap_or_default(),
                    line,
                    method_of: Some(of),
                });
            }
            Kind::Struct => {
                let (mut tag, mut name, mut base) = (None, None, None);
                for role in &f.names {
                    let n = names.next();
                    match role {
                        Role::Tag => tag = n,
                        Role::Base => base = n,
                        _ => name = n,
                    }
                }
                let (methods, attrs) = c.members.pop().unwrap_or_default();
                let of = name.clone().or(tag.clone()).unwrap_or_default();
                let methods = methods
                    .into_iter()
                    .map(|m| FuncDef { name: format!("{of}.{}", m.name), method_of: Some(of.clone()), ..m })
                    .collect();
                return Parsed::Type(TypeDef { name, tag, base, fields: c.fields.pop().unwrap_or_default(), methods, attrs, line });
            }
            Kind::Alias => {
                let (a, b) = (names.next().unwrap_or_default(), names.next().unwrap_or_default());
                return Parsed::Alias(a, b, line);
            }
            Kind::Ignore => return Parsed::Ignore,
            Kind::Package => return Parsed::Package(names.next().unwrap_or_default(), line),
            Kind::Import | Kind::ImportFrom => {
                let mut specs: Vec<Spec> = c.imports.drain(..).flat_map(|i| i.specs).collect();
                let from = if f.kind == Kind::ImportFrom { names.next() } else { None };
                // a spec itself: Go's [name] "path", Python's name [as name]
                if specs.is_empty() && f.kind == Kind::Import {
                    let n: Vec<String> = names.collect();
                    let (path, alias) = match c.strs.pop() {
                        Some(p) => (String::from_utf8_lossy(&p).to_string(), n.first().cloned()),
                        None => (n.first().cloned().unwrap_or_default(), n.get(1).cloned()),
                    };
                    specs.push(Spec { path, alias, line });
                }
                return Parsed::Import(Import { from, specs, line, at: 0 });
            }
            Kind::Var => Stmt::Var {
                name: names.next().unwrap_or_default(),
                ty: c.types.first().cloned().unwrap_or_default(),
                dims: c.dims.pop().unwrap_or_default(),
                value: exprs.next(),
                line,
            },
            Kind::Assign => {
                let value = match f.value {
                    Some(v) => Expr::Num(v, false),
                    None => exprs.next().unwrap_or(Expr::Num(0, false)),
                };
                match c.places.pop() {
                    Some(Lv::Target(target)) => Stmt::Set { target, op: f.op, value, line },
                    Some(Lv::Name(name, index)) => Stmt::Assign { name, index, op: f.op, value, line },
                    None => Stmt::Assign { name: names.next().unwrap_or_default(), index: Vec::new(), op: f.op, value, line },
                }
            }
            Kind::ForEach => {
                let (mut index, mut value) = (None, None);
                for role in &f.names {
                    let n = names.next().filter(|n| n != "_");
                    match role {
                        Role::Index => index = n,
                        _ => value = n,
                    }
                }
                Stmt::ForEach { index, value, iter: exprs.next().unwrap_or(Expr::Num(0, false)), body: bodies.next().unwrap_or_default(), line }
            }
            Kind::If => Stmt::If {
                cond: exprs.next().unwrap_or(Expr::Num(0, false)),
                then: bodies.next().unwrap_or_default(),
                els: bodies.next().unwrap_or_default(),
            },
            Kind::While => Stmt::While { cond: exprs.next(), body: bodies.next().unwrap_or_default() },
            Kind::Loop => Stmt::While { cond: None, body: bodies.next().unwrap_or_default() },
            Kind::For => {
                let mut s = c.simples.drain(..);
                let init = s.next().unwrap_or(Stmt::Nothing);
                let step = s.next().unwrap_or(Stmt::Nothing);
                Stmt::For {
                    init: Box::new(init),
                    cond: exprs.next().unwrap_or(Expr::Num(1, false)),
                    step: Box::new(step),
                    body: bodies.next().unwrap_or_default(),
                }
            }
            Kind::ForRange => Stmt::ForRange {
                name: names.next().unwrap_or_default(),
                args: c.args.pop().unwrap_or_default(),
                body: bodies.next().unwrap_or_default(),
                line,
            },
            Kind::Return => Stmt::Return(exprs.next(), line),
            Kind::Break => Stmt::Break(line),
            Kind::Continue => Stmt::Continue(line),
            Kind::Nothing => Stmt::Nothing,
            Kind::Print => Stmt::Print { style: f.style, args: c.args.pop().unwrap_or_default(), line },
            Kind::Global => Stmt::Global(names.next().unwrap_or_default(), line),
            Kind::Read => Stmt::Read { name: names.next().unwrap_or_default(), fmt: c.strs.pop(), line },
            Kind::Expr => Stmt::Expr(exprs.next().unwrap_or(Expr::Num(0, false))),
        };
        Parsed::Stmt(s)
    }

    /// What follows a value and binds tighter than any operator: [i],
    /// [i:j], .name(args).
    fn postfix(&mut self, mut e: Expr) -> R<Expr> {
        loop {
            let line = self.line();
            if self.eat("[") {
                let lo = if self.text() == Some(":") { None } else { Some(Box::new(self.expr(0)?.0)) };
                if self.eat(":") {
                    let hi = if self.text() == Some("]") { None } else { Some(Box::new(self.expr(0)?.0)) };
                    self.expect("]")?;
                    e = Expr::Slice(Box::new(e), lo, hi, line);
                } else {
                    self.expect("]")?;
                    let Some(i) = lo else { return self.miss("an index") };
                    e = Expr::Index(Box::new(e), i, line);
                }
            } else if self.l.records.as_ref().and_then(|r| r.arrow.as_deref()).is_some_and(|a| self.text() == Some(a)) {
                // p->x: the field of what p points at
                self.i += 1;
                let name = self.field_name()?;
                e = Expr::Field(Box::new(Expr::Deref(Box::new(e), line)), name, line);
            } else if (!self.l.methods.is_empty() || self.l.dotted() || self.l.records.is_some()) && self.text() == Some(".") {
                self.i += 1;
                let name = match self.peek().clone() {
                    T::Ident(s) => {
                        self.i += 1;
                        s
                    }
                    _ => return self.miss("a method's name"),
                };
                // m.x, pkg.X: a name of another module, not a call (or a
                // field, p.x: which, the program's files say)
                if self.l.dotted() && self.text() != Some("(") {
                    if let Expr::Var(n, l) = &e {
                        e = Expr::Var(format!("{n}.{name}"), *l);
                        continue;
                    }
                }
                if self.l.records.is_some() && self.text() != Some("(") {
                    e = Expr::Field(Box::new(e), name, line);
                    continue;
                }
                self.expect("(")?;
                let a = self.args()?;
                self.expect(")")?;
                e = Expr::Method(Box::new(e), name, a, line);
            } else {
                return Ok(e);
            }
        }
    }

    /// Python's list after its `[`: the elements, or a comprehension.
    fn list(&mut self, line: u32) -> R<Expr> {
        if self.eat("]") {
            return Ok(Expr::List(Vec::new(), line));
        }
        let first = self.expr(0)?.0;
        if self.l.comprehensions && self.eat("for") {
            let name = self.name()?;
            self.expect("in")?;
            // range is a word of the for form, so its call is read here
            let at = self.line();
            let iter = if self.eat("range") {
                self.expect("(")?;
                let a = self.args()?;
                self.expect(")")?;
                Expr::Call("range".into(), a, at)
            } else {
                self.expr(0)?.0
            };
            let cond = if self.eat("if") { Some(Box::new(self.expr(0)?.0)) } else { None };
            self.expect("]")?;
            return Ok(Expr::Comp { elem: Box::new(first), name, iter: Box::new(iter), cond, line });
        }
        let mut elems = vec![first];
        while self.eat(",") {
            if self.text() == Some("]") {
                break;
            }
            elems.push(self.expr(0)?.0);
        }
        self.expect("]")?;
        Ok(Expr::List(elems, line))
    }

    /// { a, b, {c, d} }: the elements of an initializer or a composite.
    fn braces(&mut self) -> R<Vec<Expr>> {
        self.expect("{")?;
        let mut elems = Vec::new();
        loop {
            while matches!(self.peek(), T::End) {
                self.i += 1;
            }
            if self.eat("}") {
                return Ok(elems);
            }
            let line = self.line();
            let keyed = self.l.records.as_ref().and_then(|r| r.keyed.clone());
            let e = if self.text() == Some("{") {
                Expr::Brace(self.braces()?, line)
            } else if keyed.as_deref() == Some(".") && self.text() == Some(".") {
                // C's designator: .x = value
                self.i += 1;
                let name = self.field_name()?;
                self.expect("=")?;
                let v = if self.text() == Some("{") { Expr::Brace(self.braces()?, line) } else { self.expr(0)?.0 };
                Expr::Key(name, Box::new(v), line)
            } else {
                let e = self.expr(0)?.0;
                match (&e, keyed.as_deref()) {
                    // Go's key: X: value
                    (Expr::Var(name, _), Some(k)) if k != "." && self.text() == Some(k) => {
                        self.i += 1;
                        let v = if self.text() == Some("{") { Expr::Brace(self.braces()?, line) } else { self.expr(0)?.0 };
                        Expr::Key(name.clone(), Box::new(v), line)
                    }
                    _ => e,
                }
            };
            elems.push(e);
            while matches!(self.peek(), T::End) {
                self.i += 1;
            }
            if !self.eat(",") {
                while matches!(self.peek(), T::End) {
                    self.i += 1;
                }
                self.expect("}")?;
                return Ok(elems);
            }
        }
    }

    /// Does a cast begin here: `(`, type words (and `*`), `)`?
    fn cast_ahead(&self) -> bool {
        let mut j = self.i + 1;
        let mut words = 0;
        while let Some(T::Ident(s) | T::Punct(s)) = self.t.get(j).map(|t| &t.t) {
            if self.is_type(s) || s == "*" {
                words += 1;
                j += 1;
            } else {
                return words > 0 && s == ")";
            }
        }
        false
    }

    /// An expression whose operators bind at least as tight as `min`. The
    /// flag says it is a comparison not in parentheses (for languages where
    /// a < b < c is not what C makes of it).
    fn expr(&mut self, min: u8) -> R<(Expr, bool)> {
        let l = self.l;
        let line = self.line();
        let (mut left, mut bare_cmp) = match self.peek().clone() {
            T::Num(n) => {
                self.i += 1;
                (Expr::Num(n, false), false)
            }
            T::Float(f) => {
                self.i += 1;
                (Expr::Float(f), false)
            }
            T::Str(s) => {
                self.i += 1;
                (self.postfix(Expr::Str(s))?, false)
            }
            // C's (type) value: type words, and nothing else, up to the `)`
            T::Punct(p) if p == "(" && l.casts && self.cast_ahead() => {
                self.i += 1;
                let ty = self.ty()?;
                self.expect(")")?;
                let p = l.unary.values().map(|u| u.0).max().unwrap_or(0);
                let (e, _) = self.expr(p)?;
                (Expr::Cast(ty, Box::new(e), line), false)
            }
            T::Punct(p) if p == "(" => {
                self.i += 1;
                let e = self.expr(0)?.0;
                self.expect(")")?;
                (self.postfix(e)?, false)
            }
            T::Punct(p) if p == "[" && l.list_literals => {
                self.i += 1;
                let e = self.list(line)?;
                (self.postfix(e)?, false)
            }
            T::Punct(p) if p == "[" && l.composite_literals => {
                let ty = self.ty()?;
                if self.text() == Some("{") {
                    let elems = self.braces()?;
                    (self.postfix(Expr::Composite(ty, elems, line))?, false)
                } else {
                    (Expr::Type(ty, line), false)
                }
            }
            T::Punct(p) if p == "{" && l.brace_lists => {
                let elems = self.braces()?;
                (Expr::Brace(elems, line), false)
            }
            t => {
                let word = match &t {
                    T::Ident(s) | T::Punct(s) => s.clone(),
                    _ => return self.miss("an expression"),
                };
                let top = l.unary.values().map(|u| u.0).max().unwrap_or(0);
                let rec = l.records.as_ref();
                if rec.and_then(|r| r.address_of.as_deref()) == Some(word.as_str()) {
                    self.i += 1;
                    let (e, _) = self.expr(top)?;
                    (Expr::Addr(Box::new(e), line), false)
                } else if rec.and_then(|r| r.deref.as_deref()) == Some(word.as_str()) {
                    self.i += 1;
                    let (e, _) = self.expr(top)?;
                    (Expr::Deref(Box::new(e), line), false)
                } else if rec.and_then(|r| r.sizeof.as_deref()) == Some(word.as_str()) {
                    self.i += 1;
                    if self.text() == Some("(") && self.cast_ahead() {
                        self.i += 1;
                        let ty = self.ty()?;
                        self.expect(")")?;
                        (Expr::SizeOfType(ty, line), false)
                    } else {
                        let (e, _) = self.expr(top)?;
                        (Expr::SizeOf(Box::new(e), line), false)
                    }
                } else if rec.and_then(|r| r.offsetof.as_deref()) == Some(word.as_str()) {
                    self.i += 1;
                    self.expect("(")?;
                    let ty = self.ty()?;
                    self.expect(",")?;
                    let f = self.field_name()?;
                    self.expect(")")?;
                    (Expr::OffsetOf(ty, f, line), false)
                } else if l.composite_literals && self.types.contains(&word) && matches!(self.t.get(self.i + 1).map(|t| &t.t), Some(T::Punct(p)) if p == "{") {
                    // Go's P{1, 2}: a struct's composite
                    self.i += 1;
                    let elems = self.braces()?;
                    (self.postfix(Expr::Composite(vec![word], elems, line))?, false)
                } else if let Some(&(p, op)) = l.unary.get(&word) {
                    self.i += 1;
                    let (e, _) = self.expr(p)?;
                    (Expr::Un(op, Box::new(e)), false)
                } else if let Some(&v) = l.bool_constants.get(&word) {
                    self.i += 1;
                    (Expr::Num(v, true), false)
                } else if matches!(t, T::Ident(_))
                    && (!l.keywords.contains(&word)
                        // a type called like a function: Python's int(...)
                        || (l.type_words.contains(&word) && matches!(self.t.get(self.i + 1).map(|t| &t.t), Some(T::Punct(p)) if p == "("))
                        // a package a form names, used otherwise: fmt.Sprint(x)
                        || (l.dotted() && matches!(self.t.get(self.i + 1).map(|t| &t.t), Some(T::Punct(p)) if p == ".")))
                {
                    self.i += 1;
                    let e = if self.eat("(") {
                        let a = self.args()?;
                        self.expect(")")?;
                        Expr::Call(word, a, line)
                    } else {
                        Expr::Var(word, line)
                    };
                    (self.postfix(e)?, false)
                } else {
                    return self.miss("an expression");
                }
            }
        };
        loop {
            let Some(word) = self.text().map(str::to_string) else { break };
            let Some(&(p, op)) = l.binary.get(&word) else { break };
            if p < min {
                break;
            }
            let at = self.line();
            self.i += 1;
            let (right, right_cmp) = self.expr(p + 1)?;
            if op.is_comparison() && l.chain_error && (bare_cmp || right_cmp) {
                return self.fatal(at, "a chained comparison (a < b < c) is not in Strike yet".into());
            }
            left = Expr::Bin(op, Box::new(left), Box::new(right));
            bare_cmp = op.is_comparison();
        }
        Ok((left, bare_cmp))
    }
}

/// The names a file gives its types (struct P, type P, class P, and a
/// typedef's last name), found before the file is read, so that a type is
/// read as one wherever it is written.
fn type_names(toks: &[Tok], l: &Lang) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let Some(r) = &l.records else { return out };
    let word = |i: usize| match toks.get(i).map(|t| &t.t) {
        Some(T::Ident(s)) => Some(s.as_str()),
        _ => None,
    };
    for i in 0..toks.len() {
        let Some(w) = word(i) else { continue };
        if r.type_intro.iter().any(|x| x == w) {
            if let Some(n) = word(i + 1) {
                out.insert(n.to_string());
            }
        }
        if r.typedef.as_deref() == Some(w) {
            // the last name before the `;` that ends it, outside braces
            let mut depth = 0;
            let mut last = None;
            for t in &toks[i + 1..] {
                match &t.t {
                    T::Punct(p) if p == "{" => depth += 1,
                    T::Punct(p) if p == "}" => depth -= 1,
                    T::Punct(p) if p == ";" && depth == 0 => break,
                    T::Ident(s) if depth == 0 => last = Some(s.clone()),
                    T::Eof => break,
                    _ => {}
                }
            }
            out.extend(last);
        }
    }
    out
}

pub fn parse(toks: &[Tok], l: &Lang) -> Result<Program, Failure> {
    let mut p = P { t: toks, i: 0, l, far: 0, far_msg: String::new(), fatal: None, types: type_names(toks, l) };
    let mut prog = Program::default();
    let fail = |p: &P| {
        let reach = p.far as f64 / toks.len().max(1) as f64;
        match &p.fatal {
            Some((line, m)) => Failure { line: *line, message: m.clone(), reach },
            None => Failure { line: toks[p.far.min(toks.len() - 1)].line, message: p.far_msg.clone(), reach },
        }
    };
    loop {
        p.skip_ends();
        if matches!(p.peek(), T::Eof) {
            return Ok(prog);
        }
        match p.statement(Ctx::Top) {
            Ok(Parsed::Func(f)) => prog.funcs.push(f),
            Ok(Parsed::Stmt(s)) => prog.top.push(s),
            Ok(Parsed::Import(mut i)) => {
                i.at = prog.top.len();
                prog.imports.push(i);
            }
            Ok(Parsed::Package(n, line)) => prog.package = Some((n, line)),
            Ok(Parsed::Type(t)) => {
                // Python's class runs where it stands: its attributes are given there
                if l.records.as_ref().is_some_and(|r| r.classes) {
                    prog.top.push(Stmt::Class(t.name.clone().unwrap_or_default(), t.line));
                }
                prog.types.push(t);
            }
            Ok(Parsed::Alias(a, b, line)) => prog.aliases.push((a, b, line)),
            Ok(Parsed::Ignore) => {}
            Err(Miss) => return Err(fail(&p)),
        }
        if p.fatal.is_some() {
            return Err(fail(&p));
        }
    }
}
