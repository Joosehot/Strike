//! Tokens to the universal syntax, by the language's forms: at every
//! statement each form is tried in the description's order, and the first
//! whose whole pattern matches is the statement. Expressions are read by one
//! precedence reader with the language's operator table.

use super::ir::{BinOp, UnOp};
use super::lang::{Cap, End, Form, Kind, Lang, Pat, Place, Style};
use super::lex::{Tok, T};

#[derive(Clone, Debug)]
pub enum Expr {
    /// a number; true when it is a truth value (True, false)
    Num(i64, bool),
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
}

#[derive(Clone, Debug)]
pub enum Stmt {
    /// ty: the type words written (none where the language writes none);
    /// dims: C's array sizes after the name
    Var { name: String, ty: Vec<String>, dims: Vec<Option<Expr>>, value: Option<Expr>, line: u32 },
    /// name, or an element of it (index: the subscripts in order)
    Assign { name: String, index: Vec<Expr>, op: Option<BinOp>, value: Expr, line: u32 },
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
    places: Vec<(String, Vec<Expr>)>,
    dims: Vec<Vec<Option<Expr>>>,
    imports: Vec<Import>,
}

impl Caps {
    fn mark(&self) -> [usize; 11] {
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
        ]
    }
    fn reset(&mut self, m: [usize; 11]) {
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
}

impl<'a> P<'a> {
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
                    T::Ident(s) if self.l.type_words.contains(s) => ty.push(s.clone()),
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
                Some(s) if self.l.type_words.contains(s) || s == "*" => {
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
                    let mut n = self.name()?;
                    // pkg.X = v, m.x = v: a name of another module
                    if self.l.qualified && self.text() == Some(".") {
                        self.i += 1;
                        let m = self.name()?;
                        n = format!("{n}.{m}");
                    }
                    let mut index = Vec::new();
                    while self.eat("[") {
                        index.push(self.expr(0)?.0);
                        self.expect("]")?;
                    }
                    caps.places.push((n, index));
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
                })
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
                let (name, index) = match c.places.pop() {
                    Some(p) => p,
                    None => (names.next().unwrap_or_default(), Vec::new()),
                };
                let value = match f.value {
                    Some(v) => Expr::Num(v, false),
                    None => exprs.next().unwrap_or(Expr::Num(0, false)),
                };
                Stmt::Assign { name, index, op: f.op, value, line }
            }
            Kind::ForEach => {
                let (mut index, mut value) = (None, None);
                for role in &f.names {
                    let n = names.next().filter(|n| n != "_");
                    match role {
                        super::lang::Role::Index => index = n,
                        super::lang::Role::Value => value = n,
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
            } else if (!self.l.methods.is_empty() || self.l.qualified) && self.text() == Some(".") {
                self.i += 1;
                let name = match self.peek().clone() {
                    T::Ident(s) => {
                        self.i += 1;
                        s
                    }
                    _ => return self.miss("a method's name"),
                };
                // m.x, pkg.X: a name of another module, not a call
                if self.l.qualified && self.text() != Some("(") {
                    if let Expr::Var(n, l) = &e {
                        e = Expr::Var(format!("{n}.{name}"), *l);
                        continue;
                    }
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
            let e = if self.text() == Some("{") { Expr::Brace(self.braces()?, line) } else { self.expr(0)?.0 };
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
            T::Str(s) => {
                self.i += 1;
                (Expr::Str(s), false)
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
                if let Some(&(p, op)) = l.unary.get(&word) {
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
                        || (l.qualified && matches!(self.t.get(self.i + 1).map(|t| &t.t), Some(T::Punct(p)) if p == ".")))
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

pub fn parse(toks: &[Tok], l: &Lang) -> Result<Program, Failure> {
    let mut p = P { t: toks, i: 0, l, far: 0, far_msg: String::new(), fatal: None };
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
            Ok(Parsed::Ignore) => {}
            Err(Miss) => return Err(fail(&p)),
        }
        if p.fatal.is_some() {
            return Err(fail(&p));
        }
    }
}
