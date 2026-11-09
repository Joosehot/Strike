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
}

#[derive(Clone, Debug)]
pub enum Stmt {
    /// ty: the type words written (none where the language writes none)
    Var { name: String, ty: Vec<String>, value: Option<Expr>, line: u32 },
    Assign { name: String, op: Option<BinOp>, value: Expr, line: u32 },
    If { cond: Expr, then: Vec<Stmt>, els: Vec<Stmt> },
    /// no condition: for ever
    While { cond: Option<Expr>, body: Vec<Stmt> },
    For { init: Box<Stmt>, cond: Expr, step: Box<Stmt>, body: Vec<Stmt> },
    ForRange { name: String, args: Vec<Expr>, body: Vec<Stmt>, line: u32 },
    Return(Option<Expr>, u32),
    Break(u32),
    Continue(u32),
    Print { style: Style, args: Vec<Expr>, line: u32 },
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

#[derive(Default, Debug)]
pub struct Program {
    pub funcs: Vec<FuncDef>,
    pub top: Vec<Stmt>,
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
}

impl Caps {
    fn mark(&self) -> [usize; 8] {
        [
            self.exprs.len(),
            self.bodies.len(),
            self.names.len(),
            self.params.len(),
            self.args.len(),
            self.simples.len(),
            self.strs.len(),
            self.types.len(),
        ]
    }
    fn reset(&mut self, m: [usize; 8]) {
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
        while let Some(s) = self.text() {
            if self.l.type_words.contains(s) || s == "*" {
                words.push(s.to_string());
                self.i += 1;
            } else {
                break;
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
            Kind::Var => Stmt::Var { name: names.next().unwrap_or_default(), ty: c.types.first().cloned().unwrap_or_default(), value: exprs.next(), line },
            Kind::Assign => {
                let name = names.next().unwrap_or_default();
                let value = match f.value {
                    Some(v) => Expr::Num(v, false),
                    None => exprs.next().unwrap_or(Expr::Num(0, false)),
                };
                Stmt::Assign { name, op: f.op, value, line }
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
            Kind::Expr => Stmt::Expr(exprs.next().unwrap_or(Expr::Num(0, false))),
        };
        Parsed::Stmt(s)
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
                (e, false)
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
                } else if matches!(t, T::Ident(_)) && !l.keywords.contains(&word) {
                    self.i += 1;
                    if self.eat("(") {
                        let a = self.args()?;
                        self.expect(")")?;
                        (Expr::Call(word, a, line), false)
                    } else {
                        (Expr::Var(word, line), false)
                    }
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
            Ok(Parsed::Ignore) => {}
            Err(Miss) => return Err(fail(&p)),
        }
        if p.fatal.is_some() {
            return Err(fail(&p));
        }
    }
}
