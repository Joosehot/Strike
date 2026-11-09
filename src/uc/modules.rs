//! A program of several files. Each language's description says how its
//! files make one program ([program] model):
//!
//!   units     C: every file is a translation unit; a function or a
//!             variable outside a function is shared by every file unless
//!             it is static
//!   packages  Go: the files of a directory are one package; a package of
//!             the same module is imported by its path, and only its names
//!             that begin with a capital letter are seen outside it
//!   modules   Python: one file runs; `import m` runs m.py next to it where
//!             m is first imported, and only then
//!
//! The files become one universal program before it is lowered. A name of
//! a module other than the main one gets the module's prefix (`m.f`,
//! `example.com/calc.Add`), and a C static one its file's, so names from
//! different files never meet in the IR, and a function's brand is still
//! its IR. Every line read is numbered on from the files before it, so the
//! line of an error names its file too. What a language does with its
//! files that Strike does not do the same way is refused.

use super::cpp;
use super::hot;
use super::ir::BinOp;
use super::lang::{Lang, Model};
use super::lex;
use super::lower::{self, assigned_names};
use super::parse::{self, Expr, FuncDef, Program, Stmt};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Why a description could not make the program: the file and line, why,
/// and how far into its file the reading got (0 to 1).
#[derive(Clone, Debug)]
pub struct Fail {
    pub file: PathBuf,
    pub line: u32,
    pub message: String,
    pub reach: f64,
}

/// A line of the whole program (every file's lines numbered on) and why.
type E = (u32, String);

struct Reader<'a> {
    l: &'a Lang,
    include: &'a [PathBuf],
    files: Vec<PathBuf>,
    /// every line read: its file and its line there
    at: Vec<(u32, u32)>,
}

impl Reader<'_> {
    fn locate(&self, line: u32) -> (PathBuf, u32) {
        if self.at.is_empty() {
            return (self.files.first().cloned().unwrap_or_default(), line);
        }
        let (f, l) = self.at[(line.max(1) as usize - 1).min(self.at.len() - 1)];
        (self.files[f as usize].clone(), l)
    }
    fn fail(&self, (line, message): E, reach: f64) -> Fail {
        let (file, line) = self.locate(line);
        Fail { file, line, message, reach }
    }
    fn index(&mut self, p: &Path) -> u32 {
        match self.files.iter().position(|f| f == p) {
            Some(i) => i as u32,
            None => {
                self.files.push(p.to_path_buf());
                self.files.len() as u32 - 1
            }
        }
    }
    /// A file read to its universal syntax (preprocessed first, where the
    /// language has a preprocessor).
    fn read(&mut self, path: &Path) -> Result<Program, Fail> {
        let text = hot::read_file(path).map_err(|e| Fail { file: path.to_path_buf(), line: 0, message: e.to_string(), reach: 0.0 })?;
        let base = self.at.len() as u32;
        let text = match &self.l.preprocessor {
            Some(p) => {
                let o = cpp::run(p, path, &text, self.include).map_err(|e| Fail { file: e.file, line: e.line, message: e.message, reach: 0.0 })?;
                for (f, line) in &o.lines {
                    let i = self.index(f);
                    self.at.push((i, *line));
                }
                if o.lines.is_empty() {
                    let i = self.index(path);
                    self.at.push((i, 1));
                }
                for f in &o.files {
                    self.index(f);
                }
                o.text
            }
            None => {
                let i = self.index(path);
                let n = text.split('\n').count() as u32;
                self.at.extend((1..=n).map(|k| (i, k)));
                text
            }
        };
        let mut toks = lex::lex(&text, self.l).map_err(|(line, m)| self.fail((base + line, m), 0.0))?;
        for t in toks.iter_mut() {
            t.line += base;
        }
        parse::parse(&toks, self.l).map_err(|f| self.fail((f.line, f.message), f.reach))
    }
}

/// The files a description takes from what it is given: the files
/// themselves, or what a directory is in the language (none: it has no
/// file there).
pub fn units(l: &Lang, files: &[PathBuf], dir: Option<&Path>) -> Vec<PathBuf> {
    let Some(d) = dir else { return files.to_vec() };
    let rule = &l.program;
    match rule.directory.strip_prefix("*.") {
        Some(ext) => {
            let mut v: Vec<PathBuf> = hot::list(d);
            v.retain(|p| {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) && !rule.skip_suffixes.iter().any(|s| name.ends_with(s.as_str()))
            });
            v.sort();
            v
        }
        None => {
            let p = d.join(&rule.directory);
            if hot::is_file(&p) {
                vec![p]
            } else {
                Vec::new()
            }
        }
    }
}

/// The program's files, made one program and lowered; and every file read.
pub fn front(l: &Lang, units: &[PathBuf], include: &[PathBuf]) -> Result<(lower::Lowered, Vec<PathBuf>), Fail> {
    let mut r = Reader { l, include, files: Vec::new(), at: Vec::new() };
    let prog = match l.program.model {
        Model::Units => link_units(&mut r, units)?,
        Model::Packages => link_packages(&mut r, units)?,
        Model::Modules => link_modules(&mut r, units)?,
    };
    let low = lower::lower(&prog, l).map_err(|e| r.fail(e, 1.0))?;
    let read = r.files.iter().filter(|f| f.is_file()).cloned().collect();
    Ok((low, read))
}

/// The names one file sees.
#[derive(Default)]
struct Space {
    /// what Python's __name__ is in it
    name: String,
    /// what its own names outside functions are given in front
    prefix: String,
    /// names outside functions -> their name in the program
    vars: HashMap<String, String>,
    /// functions -> their name in the program
    funcs: HashMap<String, String>,
    /// the modules or packages it imports, by the name they have here
    /// (the Space of each)
    mods: HashMap<String, usize>,
    /// the language's own packages it imports (Go's fmt)
    standard: HashSet<String>,
    /// every name not a function's own is the module's (Python)
    open: bool,
    /// only a name that begins with a capital letter is seen from outside
    capital: bool,
}

/// Renames a body's names as one file sees them: a function's own names
/// stay, and the others become their names in the program.
struct Walk<'a> {
    spaces: &'a [Space],
    me: usize,
    /// a block has its own names (C, Go); else a function has (Python)
    block: bool,
    /// x.name(...) on a value is a method (Python's lists)
    methods: bool,
    /// pkg.name of the language's own packages that Strike provides
    /// (strconv.Itoa): left for the lowering
    own: HashSet<String>,
    /// the names of each block (or comprehension) in, innermost last; a C
    /// static one has its name in the program
    scopes: Vec<HashMap<String, Option<String>>>,
    /// Python: the names the function assigns, so its own
    locals: HashSet<String>,
    func: Option<String>,
    /// C's static variables in functions, as variables of the program
    hoisted: Vec<Stmt>,
    /// Python: other modules' names the function assigns (m.x = 1)
    dotted: Vec<(String, u32)>,
    /// the imports used, by their name here
    used: HashSet<String>,
}

impl Walk<'_> {
    fn sp(&self) -> &Space {
        &self.spaces[self.me]
    }
    fn local(&self, n: &str) -> Option<String> {
        for s in self.scopes.iter().rev() {
            if let Some(r) = s.get(n) {
                return Some(r.clone().unwrap_or_else(|| n.to_string()));
            }
        }
        if self.func.is_some() && self.locals.contains(n) {
            return Some(n.to_string());
        }
        None
    }
    /// The module or package `a` of a.b: imported here, and b seen from
    /// outside it.
    fn module(&mut self, a: &str, b: &str, line: u32) -> Result<usize, E> {
        match self.sp().mods.get(a) {
            Some(&t) => {
                self.used.insert(a.to_string());
                if self.sp().capital && !b.starts_with(|c: char| c.is_ascii_uppercase()) {
                    return Err((line, format!("name {b} not exported by package {a}")));
                }
                Ok(t)
            }
            None if self.sp().standard.contains(a) => Err((line, format!("{a}.{b} is not in Strike yet"))),
            None => Err((line, format!("{a} is not a module imported here, so {a}.{b} names nothing"))),
        }
    }
    /// A variable's name in the program.
    fn name(&mut self, n: &str, line: u32) -> Result<String, E> {
        if let Some((a, b)) = n.split_once('.') {
            if self.local(a).is_some() {
                return Err((line, format!("{n}: {a} is a variable here, and only a module's names are read with a dot in Strike")));
            }
            let t = self.module(a, b, line)?;
            let ts = &self.spaces[t];
            if let Some(q) = ts.vars.get(b) {
                return Ok(q.clone());
            }
            if ts.funcs.contains_key(b) {
                return Err((line, format!("{n} is a function; a function as a value is not in Strike yet")));
            }
            return Err((line, if ts.open { format!("module {a} has no name {b}") } else { format!("{b} is not declared by package {a}") }));
        }
        if let Some(r) = self.local(n) {
            return Ok(r);
        }
        if self.sp().mods.contains_key(n) || self.sp().standard.contains(n) {
            return Err((
                line,
                if self.sp().open { format!("{n} is a module; only its names (like {n}.x) are values in Strike") } else { format!("use of package {n} without selector") },
            ));
        }
        if self.sp().open && n == "__name__" {
            return Err((line, "__name__ is a string, and strings are not in Strike yet (but __name__ == \"__main__\" is read)".into()));
        }
        if let Some(q) = self.sp().vars.get(n) {
            return Ok(q.clone());
        }
        if self.sp().open {
            return Ok(format!("{}{n}", self.sp().prefix));
        }
        Ok(n.to_string())
    }
    fn func_name(&self, f: &str) -> String {
        self.sp().funcs.get(f).cloned().unwrap_or_else(|| f.to_string())
    }
    fn declare(&mut self, n: &str, as_: Option<String>) {
        if let Some(s) = self.scopes.last_mut() {
            s.insert(n.to_string(), as_);
        }
    }
    fn scoped(&mut self, body: &mut [Stmt]) -> Result<(), E> {
        if self.block {
            self.scopes.push(HashMap::new());
        }
        let r = body.iter_mut().try_for_each(|s| self.stmt(s));
        if self.block {
            self.scopes.pop();
        }
        r
    }
    fn at_top(&self) -> bool {
        self.func.is_none() && self.scopes.is_empty()
    }

    fn stmt(&mut self, s: &mut Stmt) -> Result<(), E> {
        let mut hoist = None;
        match s {
            Stmt::Var { name, ty, dims, value, line } => {
                for d in dims.iter_mut().flatten() {
                    self.expr(d)?;
                }
                if let Some(v) = value {
                    self.expr(v)?;
                }
                if self.at_top() || !self.block {
                    *name = self.name(name, *line)?;
                } else if ty.iter().any(|w| w == "static") {
                    // C: a static variable in a function keeps its value from
                    // call to call; it is the program's, given its first value
                    // once, before main
                    let f = self.func.clone().unwrap_or_default();
                    let base = format!("{}{f}.{name}", self.sp().prefix);
                    let mut q = base.clone();
                    let mut k = 2;
                    while self.hoisted.iter().any(|h| matches!(h, Stmt::Var { name, .. } if *name == q)) {
                        q = format!("{base}#{k}");
                        k += 1;
                    }
                    self.declare(name, Some(q.clone()));
                    let ty = ty.iter().filter(|w| *w != "static").cloned().collect();
                    hoist = Some(Stmt::Var { name: q, ty, dims: dims.clone(), value: value.clone(), line: *line });
                } else if ty.iter().any(|w| w == "extern") {
                    return Err((*line, "extern in a function is not in Strike yet".into()));
                } else {
                    self.declare(name, None);
                }
            }
            Stmt::Assign { name, index, value, line, .. } => {
                for i in index.iter_mut() {
                    self.expr(i)?;
                }
                self.expr(value)?;
                let dotted = name.contains('.');
                *name = self.name(name, *line)?;
                if dotted && self.func.is_some() && !self.block {
                    self.dotted.push((name.clone(), *line));
                }
            }
            Stmt::If { cond, then, els } => {
                self.expr(cond)?;
                self.scoped(then)?;
                self.scoped(els)?;
            }
            Stmt::While { cond, body } => {
                if let Some(c) = cond {
                    self.expr(c)?;
                }
                self.scoped(body)?;
            }
            Stmt::For { init, cond, step, body } => {
                if self.block {
                    self.scopes.push(HashMap::new());
                }
                let r = (|| {
                    self.stmt(init)?;
                    self.expr(cond)?;
                    self.stmt(step)?;
                    self.scoped(body)
                })();
                if self.block {
                    self.scopes.pop();
                }
                r?;
            }
            Stmt::ForRange { name, args, body, line } => {
                for a in args.iter_mut() {
                    self.expr(a)?;
                }
                if self.block {
                    self.scopes.push(HashMap::new());
                    self.declare(name, None);
                    let r = self.scoped(body);
                    self.scopes.pop();
                    r?;
                } else {
                    *name = self.name(name, *line)?;
                    self.scoped(body)?;
                }
            }
            Stmt::ForEach { index, value, iter, body, line } => {
                self.expr(iter)?;
                if self.block {
                    self.scopes.push(HashMap::new());
                    for n in index.iter().chain(value.iter()) {
                        self.declare(&n.clone(), None);
                    }
                    let r = self.scoped(body);
                    self.scopes.pop();
                    r?;
                } else {
                    for n in index.iter_mut().chain(value.iter_mut()) {
                        *n = self.name(n, *line)?;
                    }
                    self.scoped(body)?;
                }
            }
            Stmt::Return(e, _) => {
                if let Some(e) = e {
                    self.expr(e)?;
                }
            }
            Stmt::Print { args, .. } => {
                for a in args.iter_mut() {
                    self.expr(a)?;
                }
            }
            Stmt::Global(n, _) => *n = format!("{}{n}", self.sp().prefix),
            Stmt::Read { name, line, .. } => *name = self.name(name, *line)?,
            Stmt::Expr(e) => self.expr(e)?,
            Stmt::Break(_) | Stmt::Continue(_) | Stmt::Nothing => {}
        }
        if let Some(h) = hoist {
            self.hoisted.push(h);
            *s = Stmt::Nothing;
        }
        Ok(())
    }

    fn expr(&mut self, e: &mut Expr) -> Result<(), E> {
        // Python's __name__ == "__main__": what the module is run as
        if let Expr::Bin(op @ (BinOp::Eq | BinOp::Ne), a, b) = e {
            let s = match (&**a, &**b) {
                (Expr::Var(n, _), Expr::Str(s)) | (Expr::Str(s), Expr::Var(n, _)) if n == "__name__" && self.sp().open && self.local(n).is_none() => Some(s.clone()),
                _ => None,
            };
            if let Some(s) = s {
                let same = s == self.sp().name.as_bytes();
                let eq = *op == BinOp::Eq;
                *e = Expr::Num((same == eq) as i64, true);
                return Ok(());
            }
        }
        let mut call = None;
        match e {
            Expr::Var(n, line) => *n = self.name(n, *line)?,
            Expr::Call(f, args, _) => {
                for a in args.iter_mut() {
                    self.expr(a)?;
                }
                *f = self.func_name(f);
            }
            Expr::Method(base, m, args, line) => {
                let module = match &**base {
                    Expr::Var(a, _) if !a.contains('.') && self.local(a).is_none() && (self.sp().mods.contains_key(a) || self.sp().standard.contains(a)) => Some(a.clone()),
                    _ => None,
                };
                for a in args.iter_mut() {
                    self.expr(a)?;
                }
                match module {
                    // strconv.Itoa(...): the language's own, which the lowering knows
                    Some(a) if self.sp().standard.contains(&a) && self.own.contains(&format!("{a}.{m}")) => {}
                    // m.f(...), pkg.F(...): a function of another module
                    Some(a) => {
                        let t = self.module(&a, m, *line)?;
                        let ts = &self.spaces[t];
                        let Some(q) = ts.funcs.get(m.as_str()).cloned() else {
                            let n = format!("{a}.{m}");
                            return Err((
                                *line,
                                if ts.vars.contains_key(m.as_str()) {
                                    format!("{n} is a variable; calling one is not in Strike yet")
                                } else if ts.open {
                                    format!("module {a} has no function {m}")
                                } else {
                                    format!("{m} is not declared by package {a}")
                                },
                            ));
                        };
                        call = Some(Expr::Call(q, std::mem::take(args), *line));
                    }
                    None if !self.methods => return Err((*line, format!(".{m}(...) after a value: methods are not in Strike yet"))),
                    None => self.expr(base)?,
                }
            }
            Expr::Comp { elem, name, iter, cond, .. } => {
                self.expr(iter)?;
                self.scopes.push(HashMap::from([(name.clone(), None)]));
                let r = (|| {
                    self.expr(elem)?;
                    if let Some(c) = cond {
                        self.expr(c)?;
                    }
                    Ok(())
                })();
                self.scopes.pop();
                r?;
            }
            Expr::Un(_, x) | Expr::Cast(_, x, _) => self.expr(x)?,
            Expr::Bin(_, a, b) | Expr::Index(a, b, _) => {
                self.expr(a)?;
                self.expr(b)?;
            }
            Expr::Slice(b, lo, hi, _) => {
                self.expr(b)?;
                for x in lo.iter_mut().chain(hi.iter_mut()) {
                    self.expr(x)?;
                }
            }
            Expr::List(xs, _) | Expr::Brace(xs, _) | Expr::Composite(_, xs, _) => {
                for x in xs.iter_mut() {
                    self.expr(x)?;
                }
            }
            Expr::Num(..) | Expr::Float(_) | Expr::Str(_) | Expr::Type(..) => {}
        }
        if let Some(c) = call {
            *e = c;
        }
        Ok(())
    }
}

/// Python's `global x` names anywhere in a body.
fn globals_declared(body: &[Stmt], out: &mut HashSet<String>) {
    for s in body {
        match s {
            Stmt::Global(n, _) => {
                out.insert(n.clone());
            }
            Stmt::If { then, els, .. } => {
                globals_declared(then, out);
                globals_declared(els, out);
            }
            Stmt::While { body, .. } | Stmt::ForRange { body, .. } | Stmt::ForEach { body, .. } | Stmt::For { body, .. } => globals_declared(body, out),
            _ => {}
        }
    }
}

fn walker<'a>(spaces: &'a [Space], me: usize, l: &Lang) -> Walk<'a> {
    Walk {
        spaces,
        me,
        block: l.block_scope,
        methods: !l.methods.is_empty(),
        own: l.qualified.keys().cloned().collect(),
        scopes: Vec::new(),
        locals: HashSet::new(),
        func: None,
        hoisted: Vec::new(),
        dotted: Vec::new(),
        used: HashSet::new(),
    }
}

/// Renames one file's names: its top-level code and its functions. Gives
/// back C's static variables in functions, and the imports used.
fn rename(spaces: &[Space], me: usize, l: &Lang, p: &mut Program) -> Result<(Vec<Stmt>, HashSet<String>), E> {
    let mut w = walker(spaces, me, l);
    for s in p.top.iter_mut() {
        w.stmt(s)?;
    }
    for f in p.funcs.iter_mut() {
        rename_func(&mut w, f)?;
    }
    Ok((w.hoisted, w.used))
}

fn rename_func(w: &mut Walk, f: &mut FuncDef) -> Result<(), E> {
    w.func = Some(f.name.clone());
    w.scopes.clear();
    w.locals.clear();
    if w.block {
        w.scopes.push(f.params.iter().map(|p| (p.clone(), None)).collect());
    } else {
        let mut globals = HashSet::new();
        globals_declared(&f.body, &mut globals);
        w.locals = assigned_names(&f.body).into_iter().filter(|n| !n.contains('.') && !globals.contains(n)).collect();
        w.locals.extend(f.params.iter().cloned());
    }
    let r = w.scoped(&mut f.body);
    w.func = None;
    w.scopes.clear();
    r?;
    // Python: another module's name the function assigns is that module's,
    // not the function's own
    let mut seen = HashSet::new();
    for (q, line) in std::mem::take(&mut w.dotted).into_iter().rev() {
        if seen.insert(q.clone()) {
            f.body.insert(0, Stmt::Global(q, line));
        }
    }
    f.name = w.func_name(&f.name);
    Ok(())
}

fn has(words: &[String], w: &str) -> bool {
    words.iter().any(|x| x == w)
}

/// C: every file a translation unit, linked by the names that are not
/// static.
fn link_units(r: &mut Reader, units: &[PathBuf]) -> Result<Program, Fail> {
    let mut progs = Vec::new();
    for u in units {
        progs.push(r.read(u)?);
    }
    // a file's static names get its name in front (one file needs none)
    let mut seen = HashSet::new();
    let prefixes: Vec<String> = units
        .iter()
        .map(|u| {
            if units.len() == 1 {
                return String::new();
            }
            let stem = u.file_stem().and_then(|s| s.to_str()).unwrap_or("c").to_string();
            let mut p = stem.clone();
            let mut k = 2;
            while !seen.insert(p.clone()) {
                p = format!("{stem}{k}");
                k += 1;
            }
            format!("{p}.")
        })
        .collect();
    let mut shared_funcs: HashMap<String, usize> = HashMap::new();
    let mut shared_vars: HashMap<String, usize> = HashMap::new();
    let mut spaces = Vec::new();
    for (k, p) in progs.iter_mut().enumerate() {
        let mut sp = Space { prefix: prefixes[k].clone(), ..Default::default() };
        for f in p.funcs.iter_mut() {
            if has(&f.ret, "static") {
                sp.funcs.insert(f.name.clone(), format!("{}{}", sp.prefix, f.name));
            } else if let Some(&o) = shared_funcs.get(&f.name) {
                if o != k {
                    let m = format!("{} is defined in {} too; a function that is not static is one for the whole program", f.name, units[o].display());
                    return Err(r.fail((f.line, m), 1.0));
                }
            } else {
                shared_funcs.insert(f.name.clone(), k);
            }
            f.ret.retain(|w| w != "static" && w != "extern");
        }
        let mut top = Vec::new();
        for mut s in std::mem::take(&mut p.top) {
            if let Stmt::Var { name, ty, value, line, .. } = &mut s {
                // extern int x; says x is defined in some file: it is no
                // variable of its own
                if has(ty, "extern") && value.is_none() {
                    continue;
                }
                if has(ty, "static") {
                    sp.vars.insert(name.clone(), format!("{}{name}", sp.prefix));
                } else if let Some(&o) = shared_vars.get(name.as_str()) {
                    if o != k {
                        let m = format!("{name} is defined in {} too (clang's linker refuses a variable defined twice)", units[o].display());
                        return Err(r.fail((*line, m), 1.0));
                    }
                } else {
                    shared_vars.insert(name.clone(), k);
                }
                ty.retain(|w| w != "extern");
            }
            top.push(s);
        }
        p.top = top;
        spaces.push(sp);
    }
    let mut all = Program::default();
    let mut hoisted = Vec::new();
    for (k, mut p) in progs.into_iter().enumerate() {
        let (h, _) = rename(&spaces, k, r.l, &mut p).map_err(|e| r.fail(e, 1.0))?;
        hoisted.extend(h);
        all.funcs.extend(p.funcs);
        all.top.extend(p.top);
    }
    all.top.extend(hoisted);
    Ok(all)
}

/// Go: a package per directory, imported by its path in the module.
fn link_packages(r: &mut Reader, units: &[PathBuf]) -> Result<Program, Fail> {
    let dir = units[0].parent().map(Path::to_path_buf).unwrap_or_default();
    if let Some(u) = units.iter().find(|u| u.parent().map(Path::to_path_buf).unwrap_or_default() != dir) {
        return Err(Fail { file: u.clone(), line: 1, message: "the files of a Go package are all in one directory".into(), reach: 1.0 });
    }
    let mut g = Packages { pkgs: Vec::new(), by_path: HashMap::new(), loading: Vec::new(), module: None };
    // the module: the nearest go.mod at or above the directory
    if let Some(name) = &r.l.program.module_file {
        let abs = std::path::absolute(&dir).unwrap_or(dir.clone());
        let mut d = Some(abs.as_path());
        while let Some(x) = d {
            let f = x.join(name);
            if hot::is_file(&f) {
                g.module = Some(read_go_mod(&f)?);
                break;
            }
            d = x.parent();
        }
    }
    let mut sorted = units.to_vec();
    sorted.sort();
    g.load(r, "main", &sorted, None)?;
    // the order the packages are given their variables in: a package after
    // the ones it imports, and of those ready the first by its path (Go 1.21)
    let mut order = Vec::new();
    let mut done = vec![false; g.pkgs.len()];
    while order.len() < g.pkgs.len() {
        let next = (0..g.pkgs.len())
            .filter(|&i| !done[i] && g.pkgs[i].deps.iter().all(|&d| done[d]))
            .min_by(|&a, &b| g.pkgs[a].path.cmp(&g.pkgs[b].path))
            .expect("an import cycle is refused when it is read");
        done[next] = true;
        order.push(next);
    }
    // a Space per file: the package's names, and the file's own imports
    let mut spaces: Vec<Space> = Vec::new();
    let mut first: Vec<usize> = Vec::new();
    for p in &g.pkgs {
        first.push(spaces.len());
        let prefix = if p.path == "main" { String::new() } else { format!("{}.", p.path) };
        let mut vars = HashMap::new();
        let mut funcs = HashMap::new();
        let mut lines: HashMap<String, u32> = HashMap::new();
        for (_, prog) in &p.files {
            let names = prog.funcs.iter().map(|f| (&f.name, f.line, true)).chain(prog.top.iter().filter_map(|s| match s {
                Stmt::Var { name, line, .. } => Some((name, *line, false)),
                _ => None,
            }));
            for (n, line, is_func) in names {
                if n == "init" && is_func {
                    return Err(r.fail((line, "a package's init function is not in Strike yet".into()), 1.0));
                }
                if let Some(was) = lines.insert(n.clone(), line) {
                    let (file, l) = r.locate(was);
                    return Err(r.fail((line, format!("{n} redeclared in this block (it is also at {}:{l})", file.display())), 1.0));
                }
                if is_func {
                    funcs.insert(n.clone(), format!("{prefix}{n}"));
                } else {
                    vars.insert(n.clone(), format!("{prefix}{n}"));
                }
            }
        }
        for _ in &p.files {
            spaces.push(Space { prefix: prefix.clone(), vars: vars.clone(), funcs: funcs.clone(), capital: true, ..Default::default() });
        }
    }
    for (pi, p) in g.pkgs.iter().enumerate() {
        for (fi, imports) in p.imports.iter().enumerate() {
            let sp = &mut spaces[first[pi] + fi];
            for im in imports {
                match im.pkg {
                    Some(t) => {
                        sp.mods.insert(im.name.clone(), first[t]);
                    }
                    None => {
                        sp.standard.insert(im.name.clone());
                    }
                }
            }
        }
    }
    let mut all = Program::default();
    let mut tops: Vec<Vec<Stmt>> = vec![Vec::new(); g.pkgs.len()];
    let pkgs = std::mem::take(&mut g.pkgs);
    for (pi, p) in pkgs.into_iter().enumerate() {
        for (fi, (_, mut prog)) in p.files.into_iter().enumerate() {
            let (_, used) = rename(&spaces, first[pi] + fi, r.l, &mut prog).map_err(|e| r.fail(e, 1.0))?;
            for im in &p.imports[fi] {
                if im.pkg.is_some() && !used.contains(&im.name) {
                    return Err(r.fail((im.line, format!("\"{}\" imported and not used", im.path)), 1.0));
                }
            }
            all.funcs.extend(prog.funcs);
            tops[pi].extend(prog.top);
        }
    }
    for i in order {
        all.top.extend(std::mem::take(&mut tops[i]));
    }
    Ok(all)
}

/// go.mod: the module's path, from its root. What else it may say that
/// would change the program (another module required) is refused.
fn read_go_mod(f: &Path) -> Result<(PathBuf, String), Fail> {
    let text = hot::read_file(f).map_err(|e| Fail { file: f.to_path_buf(), line: 0, message: e.to_string(), reach: 1.0 })?;
    let mut path = None;
    for (i, line) in text.lines().enumerate() {
        let line = line.split("//").next().unwrap_or("").trim();
        let word = line.split_whitespace().next().unwrap_or("");
        let fail = |m: String| Fail { file: f.to_path_buf(), line: i as u32 + 1, message: m, reach: 1.0 };
        match word {
            "" | "go" | "toolchain" => {}
            "module" => path = Some(line["module".len()..].trim().trim_matches('"').to_string()),
            w => return Err(fail(format!("{w} in go.mod is not in Strike yet (only the module's own packages are)"))),
        }
    }
    let path = path.ok_or_else(|| Fail { file: f.to_path_buf(), line: 1, message: "go.mod names no module".into(), reach: 1.0 })?;
    Ok((f.parent().map(Path::to_path_buf).unwrap_or_default(), path))
}

struct GoImport {
    /// the name it has in the file
    name: String,
    path: String,
    /// the package of the program it is (none: one of the language's own)
    pkg: Option<usize>,
    line: u32,
}

struct Package {
    path: String,
    /// the name its package clause gives it
    name: String,
    files: Vec<(PathBuf, Program)>,
    /// each file's imports
    imports: Vec<Vec<GoImport>>,
    deps: Vec<usize>,
}

struct Packages {
    pkgs: Vec<Package>,
    by_path: HashMap<String, usize>,
    loading: Vec<String>,
    /// go.mod's directory and module path
    module: Option<(PathBuf, String)>,
}

impl Packages {
    /// A package's files read, and the packages they import, first.
    fn load(&mut self, r: &mut Reader, path: &str, files: &[PathBuf], from: Option<(u32, &str)>) -> Result<usize, Fail> {
        let at = from.map(|f| f.0).unwrap_or(1);
        if files.is_empty() {
            return Err(r.fail((at, format!("there are no Go files for the package {path}")), 1.0));
        }
        self.loading.push(path.to_string());
        let mut read = Vec::new();
        let mut name: Option<(String, PathBuf)> = None;
        for f in files {
            let prog = r.read(f)?;
            let Some((n, line)) = prog.package.clone() else {
                let first = prog.funcs.first().map(|f| f.line).unwrap_or(1);
                return Err(r.fail((first, "a Go file begins with its package clause".into()), 1.0));
            };
            match &name {
                Some((m, other)) if *m != n => {
                    return Err(r.fail((line, format!("found packages {m} ({}) and {n} in one directory", other.display())), 1.0));
                }
                None => name = Some((n.clone(), f.clone())),
                _ => {}
            }
            if path == "main" && n != "main" {
                return Err(r.fail((line, format!("package {n} is not a main package")), 1.0));
            }
            if path != "main" && n == "main" {
                return Err(r.fail((at, format!("import \"{path}\" is a program, not an importable package")), 1.0));
            }
            read.push((f.clone(), prog));
        }
        let mut imports = Vec::new();
        let mut deps = Vec::new();
        for (_, prog) in &read {
            let mut mine = Vec::new();
            for im in &prog.imports {
                for s in &im.specs {
                    if r.l.program.standard.contains(&s.path) {
                        if s.alias.is_some() {
                            return Err(r.fail((s.line, format!("a name for {} other than its own is not in Strike yet", s.path)), 1.0));
                        }
                        let n = s.path.rsplit('/').next().unwrap_or(&s.path).to_string();
                        mine.push(GoImport { name: n, path: s.path.clone(), pkg: None, line: s.line });
                        continue;
                    }
                    let Some((root, module)) = self.module.clone() else {
                        return Err(r.fail((s.line, format!("the package {} is not in Strike yet (only {} and the packages of a module with its go.mod)", s.path, r.l.program.standard.join(", "))), 1.0));
                    };
                    let rel = if s.path == module {
                        Some("")
                    } else {
                        s.path.strip_prefix(&format!("{module}/"))
                    };
                    let Some(rel) = rel else {
                        return Err(r.fail((s.line, format!("the package {} is not in Strike yet (only {} and the packages of the module {module})", s.path, r.l.program.standard.join(", "))), 1.0));
                    };
                    if self.loading.contains(&s.path) {
                        return Err(r.fail((s.line, format!("import cycle not allowed ({} imports itself through {})", s.path, self.loading.join(" -> "))), 1.0));
                    }
                    let t = match self.by_path.get(&s.path) {
                        Some(&t) => t,
                        None => {
                            let d = rel.split('/').filter(|x| !x.is_empty()).fold(root.clone(), |d, x| d.join(x));
                            let files = units(r.l, &[], Some(&d));
                            self.load(r, &s.path, &files, Some((s.line, &s.path)))?
                        }
                    };
                    let n = match &s.alias {
                        Some(a) if a == "_" => return Err(r.fail((s.line, "an import for its side effects alone (_) is not in Strike yet".into()), 1.0)),
                        Some(a) => a.clone(),
                        None => self.pkgs[t].name.clone(),
                    };
                    deps.push(t);
                    mine.push(GoImport { name: n, path: s.path.clone(), pkg: Some(t), line: s.line });
                }
            }
            imports.push(mine);
        }
        self.loading.pop();
        let name = name.map(|n| n.0).unwrap_or_default();
        self.pkgs.push(Package { path: path.to_string(), name, files: read, imports, deps });
        let i = self.pkgs.len() - 1;
        self.by_path.insert(path.to_string(), i);
        Ok(i)
    }
}

/// Python: the main file runs, and imports the others next to it.
fn link_modules(r: &mut Reader, units: &[PathBuf]) -> Result<Program, Fail> {
    if units.len() > 1 {
        return Err(Fail { file: units[1].clone(), line: 1, message: "Python runs one file; the others are imported from it".into(), reach: 1.0 });
    }
    let main = &units[0];
    let dir = main.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut m = Modules { mods: Vec::new(), by_name: HashMap::new(), loading: Vec::new(), dir, main: main.clone() };
    m.load(r, "__main__", main)?;
    let spaces: Vec<Space> = m.mods.iter_mut().map(|x| std::mem::take(&mut x.space)).collect();
    let mut progs = Vec::new();
    let mut all = Program::default();
    for (k, x) in m.mods.iter_mut().enumerate() {
        let mut p = std::mem::take(&mut x.prog);
        rename(&spaces, k, r.l, &mut p).map_err(|e| r.fail(e, 1.0))?;
        all.funcs.extend(std::mem::take(&mut p.funcs));
        progs.push(p);
    }
    // the code that runs: the main file's, with each module's run where it
    // is first imported
    let mut done = vec![false; progs.len()];
    let main_index = m.by_name["__main__"];
    run_module(main_index, &mut progs, &spaces, &m.by_name, &mut done, &mut all.top);
    Ok(all)
}

fn run_module(k: usize, progs: &mut [Program], spaces: &[Space], by_name: &HashMap<String, usize>, done: &mut [bool], out: &mut Vec<Stmt>) {
    done[k] = true;
    let top = std::mem::take(&mut progs[k].top);
    let imports = std::mem::take(&mut progs[k].imports);
    let n = top.len();
    let mut top = top.into_iter();
    let mut ii = 0;
    for pos in 0..=n {
        while ii < imports.len() && imports[ii].at <= pos {
            let im = &imports[ii];
            ii += 1;
            let targets: Vec<&str> = match &im.from {
                Some(f) => vec![f.as_str()],
                None => im.specs.iter().map(|s| s.path.as_str()).collect(),
            };
            for t in targets {
                // a module of the language's own (math) runs nothing
                let Some(&t) = by_name.get(t) else { continue };
                if !done[t] {
                    run_module(t, progs, spaces, by_name, done, out);
                }
            }
            // from m import x: x here is m.x as it is now
            if let Some(f) = &im.from {
                let t = &spaces[by_name[f.as_str()]];
                for s in &im.specs {
                    if let Some(q) = t.vars.get(&s.path) {
                        let alias = s.alias.clone().unwrap_or(s.path.clone());
                        let name = spaces[k].vars.get(&alias).cloned().unwrap_or(alias);
                        out.push(Stmt::Assign { name, index: Vec::new(), op: None, value: Expr::Var(q.clone(), s.line), line: s.line });
                    }
                }
            }
        }
        if let Some(s) = top.next() {
            out.push(s);
        }
    }
}

struct Module {
    prog: Program,
    space: Space,
}

struct Modules {
    mods: Vec<Module>,
    by_name: HashMap<String, usize>,
    loading: Vec<String>,
    dir: PathBuf,
    main: PathBuf,
}

impl Modules {
    /// A module read, the modules it imports first, and the names it has.
    fn load(&mut self, r: &mut Reader, name: &str, path: &Path) -> Result<usize, Fail> {
        self.loading.push(name.to_string());
        let prog = r.read(path)?;
        let prefix = if name == "__main__" { String::new() } else { format!("{name}.") };
        let mut sp = Space { name: name.to_string(), prefix: prefix.clone(), open: true, ..Default::default() };
        for f in &prog.funcs {
            sp.funcs.insert(f.name.clone(), format!("{prefix}{}", f.name));
        }
        let mut globals: HashSet<String> = assigned_names(&prog.top).into_iter().filter(|n| !n.contains('.')).collect();
        for f in &prog.funcs {
            globals_declared(&f.body, &mut globals);
        }
        for g in globals {
            sp.vars.insert(g.clone(), format!("{prefix}{g}"));
        }
        for im in &prog.imports {
            // Python's own modules that Strike has (math): their functions
            // are the language's own, as Go's standard packages are
            let standard = |t: &str| r.l.program.standard.iter().any(|x| x == t);
            if let Some(f) = im.from.as_deref().filter(|f| standard(f)) {
                return Err(r.fail((im.line, format!("from {f} import ... is not in Strike yet (import {f} is)")), 1.0));
            }
            let mut files = Vec::new();
            for s in &im.specs {
                if im.from.is_some() || !standard(&s.path) {
                    files.push(s);
                } else if s.alias.is_some() {
                    return Err(r.fail((s.line, format!("import {} as ... is not in Strike yet (import {} is)", s.path, s.path)), 1.0));
                } else {
                    sp.standard.insert(s.path.clone());
                }
            }
            let targets: Vec<(&str, u32)> = match &im.from {
                Some(f) => vec![(f.as_str(), im.line)],
                None => files.iter().map(|s| (s.path.as_str(), s.line)).collect(),
            };
            let mut found = Vec::new();
            for (t, line) in targets {
                found.push(self.module(r, t, line)?);
            }
            match &im.from {
                None => {
                    for (s, &t) in files.iter().zip(&found) {
                        let n = s.alias.clone().unwrap_or(s.path.clone());
                        if sp.vars.contains_key(&n) || sp.funcs.contains_key(&n) {
                            return Err(r.fail((s.line, format!("{n} names a module here and is also assigned or defined; that is not in Strike yet")), 1.0));
                        }
                        sp.mods.insert(n, t);
                    }
                }
                Some(f) => {
                    if im.specs.is_empty() {
                        return Err(r.fail((im.line, format!("from {f} import * is not in Strike yet")), 1.0));
                    }
                    let t = &self.mods[found[0]].space;
                    for s in &im.specs {
                        let n = s.alias.clone().unwrap_or(s.path.clone());
                        if sp.funcs.contains_key(&n) {
                            return Err(r.fail((s.line, format!("{n} is imported from {f} and also defined here; that is not in Strike yet")), 1.0));
                        }
                        if let Some(q) = t.funcs.get(&s.path) {
                            sp.funcs.insert(n, q.clone());
                        } else if t.vars.contains_key(&s.path) {
                            sp.vars.insert(n.clone(), format!("{prefix}{n}"));
                        } else if t.mods.contains_key(&s.path) {
                            return Err(r.fail((s.line, format!("{} is a module {f} imports; taking it from {f} is not in Strike yet", s.path)), 1.0));
                        } else {
                            return Err(r.fail((s.line, format!("cannot import name {} from {f}", s.path)), 1.0));
                        }
                    }
                }
            }
        }
        self.loading.pop();
        self.mods.push(Module { prog, space: sp });
        let i = self.mods.len() - 1;
        self.by_name.insert(name.to_string(), i);
        Ok(i)
    }

    /// The module `import t` names: the file t.py next to the main file.
    fn module(&mut self, r: &mut Reader, t: &str, line: u32) -> Result<usize, Fail> {
        if let Some(&i) = self.by_name.get(t) {
            return Ok(i);
        }
        if self.loading.iter().any(|x| x == t) {
            return Err(r.fail((line, format!("{t} is imported while it is being imported ({} -> {t}): a circular import is not in Strike yet", self.loading.join(" -> "))), 1.0));
        }
        let path = self.dir.join(format!("{t}.py"));
        let main = self.main.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        if r.l.program.found_first.iter().any(|x| x == t) {
            return Err(r.fail((line, format!("{t} is one of Python's own modules, which Python finds before any file, and those are not in Strike yet")), 1.0));
        }
        if !hot::is_file(&path) {
            return Err(r.fail((line, format!("there is no {t}.py next to {main}, and Python's own modules are not in Strike yet")), 1.0));
        }
        self.load(r, t, &path)
    }
}
