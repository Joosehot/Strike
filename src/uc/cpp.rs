//! C's preprocessor, run on a C file before it is cut into C's tokens:
//! #include, #define (object-like and function-like, with # and ##),
//! #undef, the conditionals with defined and integer constant
//! expressions, #error, and #pragma once. Expansion is the standard's, by
//! hide sets (Prosser's algorithm, as clang does it). The macros clang
//! defines before any file come first (languages/c-predefined.h), so a
//! program that tests one finds what clang has; a header Strike knows gives
//! what the description says it gives, and any other is refused. Every
//! token keeps the file and line it came from, so an error found later
//! still points at the line that was written.

use super::lang::Preprocessor;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The preprocessed text: one line per line of the program it came from,
/// and for each line (from 1) the file and line it came from.
pub struct Output {
    pub text: String,
    pub lines: Vec<(PathBuf, u32)>,
    /// every file read: the one preprocessed and what it includes
    pub files: Vec<PathBuf>,
}

/// Why preprocessing stopped: where, and why.
#[derive(Debug)]
pub struct Error {
    pub file: PathBuf,
    pub line: u32,
    pub message: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum K {
    Ident,
    Num,
    Char,
    Str,
    Punct,
    /// a character that is no token of C's (`@`, a quote that never closes)
    Other,
    /// what an empty argument is next to ##: nothing, that pastes as nothing
    Placemarker,
}

type Hide = Rc<BTreeSet<String>>;

#[derive(Clone, Debug)]
struct Tok {
    k: K,
    s: String,
    /// whitespace before it on its line
    space: bool,
    /// where it was written: (file, line)
    at: (u32, u32),
    /// the macros it came out of, which it may not call again
    hide: Hide,
}

impl Tok {
    fn is(&self, p: &str) -> bool {
        matches!(self.k, K::Punct | K::Ident) && self.s == p
    }
}

/// C's punctuators, longest first. The digraphs are read so that they are
/// refused, not taken for two tokens they are not.
const PUNCTS: [&str; 54] = [
    "%:%:", "...", "<<=", ">>=", "->", "++", "--", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "*=", "/=", "%=", "+=", "-=", "&=", "^=", "|=", "##", "<:", ":>", "<%",
    "%>", "%:", "[", "]", "(", ")", "{", "}", ".", "&", "*", "+", "-", "~", "!", "/", "%", "<", ">", "^", "|", "?", ":", ";", "=", ",", "#",
];
const DIGRAPHS: [&str; 6] = ["<:", ":>", "<%", "%>", "%:", "%:%:"];

/// Names the preprocessor itself gives a meaning: __LINE__ and __COUNTER__
/// are given; the others depend on how and when clang is run, so they are
/// refused.
const BUILT_IN: [&str; 8] = ["__LINE__", "__COUNTER__", "__FILE__", "__FILE_NAME__", "__BASE_FILE__", "__DATE__", "__TIME__", "__TIMESTAMP__"];

/// The lines of a file, after backslash-newlines are joined and comments
/// are made spaces: each line's tokens. A comment over several lines
/// joins them into one, as C says.
fn tokenize(text: &str, file: u32) -> Result<Vec<Vec<Tok>>, (u32, String)> {
    // phase 2: a backslash at the end of a line joins the next to it; each
    // character keeps the line it was written on
    let mut ch: Vec<char> = Vec::with_capacity(text.len());
    let mut ln: Vec<u32> = Vec::with_capacity(text.len());
    let raw: Vec<char> = text.chars().collect();
    let mut line = 1u32;
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == '\\' {
            let mut j = i + 1;
            if raw.get(j) == Some(&'\r') {
                j += 1;
            }
            if raw.get(j) == Some(&'\n') {
                line += 1;
                i = j + 1;
                continue;
            }
        }
        ch.push(raw[i]);
        ln.push(line);
        if raw[i] == '\n' {
            line += 1;
        }
        i += 1;
    }
    let empty: Hide = Rc::new(BTreeSet::new());
    let mut lines = Vec::new();
    let mut cur: Vec<Tok> = Vec::new();
    let mut space = false;
    let mut i = 0;
    let at = |i: usize| ln.get(i).copied().unwrap_or(line);
    while i < ch.len() {
        let c = ch[i];
        if c == '\n' {
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            space = false;
            i += 1;
            continue;
        }
        if c == ' ' || c == '\t' || c == '\r' || c == '\x0c' || c == '\x0b' {
            space = true;
            i += 1;
            continue;
        }
        if c == '/' && ch.get(i + 1) == Some(&'/') {
            while i < ch.len() && ch[i] != '\n' {
                i += 1;
            }
            space = true;
            continue;
        }
        if c == '/' && ch.get(i + 1) == Some(&'*') {
            let start = at(i);
            let mut j = i + 2;
            loop {
                if j + 1 >= ch.len() {
                    return Err((start, "a comment that never closes".into()));
                }
                if ch[j] == '*' && ch[j + 1] == '/' {
                    break;
                }
                j += 1;
            }
            i = j + 2;
            space = true;
            continue;
        }
        let start = i;
        let k = if c.is_ascii_digit() || (c == '.' && ch.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            // a pp-number: digits, letters, _, ., and e+ e- p+ p-
            i += 1;
            while i < ch.len() {
                let d = ch[i];
                if matches!(d, 'e' | 'E' | 'p' | 'P') && matches!(ch.get(i + 1), Some('+' | '-')) {
                    i += 2;
                } else if d.is_ascii_alphanumeric() || d == '_' || d == '.' {
                    i += 1;
                } else {
                    break;
                }
            }
            K::Num
        } else if c.is_ascii_alphabetic() || c == '_' {
            while i < ch.len() && (ch[i].is_ascii_alphanumeric() || ch[i] == '_') {
                i += 1;
            }
            K::Ident
        } else if c == '"' || c == '\'' {
            let mut j = i + 1;
            let mut closed = false;
            while j < ch.len() && ch[j] != '\n' {
                if ch[j] == '\\' && j + 1 < ch.len() && ch[j + 1] != '\n' {
                    j += 2;
                    continue;
                }
                if ch[j] == c {
                    closed = true;
                    break;
                }
                j += 1;
            }
            if closed {
                i = j + 1;
                if c == '"' {
                    K::Str
                } else {
                    K::Char
                }
            } else {
                // a quote that never closes: a character of its own, refused
                // where it is read (a group that is skipped may hold one)
                i += 1;
                K::Other
            }
        } else if let Some(p) = PUNCTS.iter().find(|p| p.chars().enumerate().all(|(n, pc)| ch.get(i + n) == Some(&pc))) {
            i += p.chars().count();
            K::Punct
        } else {
            i += 1;
            K::Other
        };
        cur.push(Tok { k, s: ch[start..i].iter().collect(), space, at: (file, at(start)), hide: empty.clone() });
        space = false;
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    Ok(lines)
}

#[derive(Clone, Debug)]
struct Macro {
    /// none: object-like
    params: Option<Vec<String>>,
    variadic: bool,
    body: Vec<Tok>,
}

struct Cond {
    /// the group this one is in is read
    outer: bool,
    /// this branch is read
    on: bool,
    /// a branch of it has been taken
    taken: bool,
    seen_else: bool,
    line: u32,
}

struct Pp<'a> {
    rule: &'a Preprocessor,
    macros: HashMap<String, Macro>,
    files: Vec<PathBuf>,
    /// files with #pragma once, and the headers Strike knows (read once)
    once: HashSet<PathBuf>,
    include: &'a [PathBuf],
    out: Vec<Tok>,
    depth: usize,
    counter: i64,
}

type R<T> = Result<T, ((u32, u32), String)>;

/// Gives the input the next line, if there is one an invocation may take.
type Pull<'a> = dyn FnMut(&mut VecDeque<Tok>) -> bool + 'a;

fn fail<T>(at: (u32, u32), m: impl Into<String>) -> R<T> {
    Err((at, m.into()))
}

/// The same file however it is named.
fn once_key(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or(p.to_path_buf())
}

/// How a token is written back: the text of tokens.
fn spell(toks: &[Tok]) -> String {
    let mut s = String::new();
    for (i, t) in toks.iter().enumerate() {
        if i > 0 && t.space {
            s.push(' ');
        }
        s.push_str(&t.s);
    }
    s
}

impl Pp<'_> {
    fn file_id(&mut self, p: PathBuf) -> u32 {
        self.files.push(p);
        self.files.len() as u32 - 1
    }

    /// One file, with its own conditionals, which must close in it.
    fn run(&mut self, id: u32, text: &str) -> R<()> {
        let lines = tokenize(text, id).map_err(|(line, m)| ((id, line), m))?;
        let mut conds: Vec<Cond> = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let line = &lines[i];
            i += 1;
            let on = conds.last().is_none_or(|c| c.on);
            if !(line[0].is(&self.rule.marker) && line[0].k == K::Punct) {
                if on {
                    self.text(&lines, &mut i, line.clone())?;
                }
                continue;
            }
            let at = line[0].at;
            let Some(d) = line.get(1) else { continue }; // the null directive
            let rest = &line[2..];
            let name = if d.k == K::Ident { d.s.as_str() } else { "" };
            match name {
                "if" | "ifdef" | "ifndef" => {
                    let v = if !on {
                        false
                    } else if name == "if" {
                        self.condition(rest, d.at)?
                    } else {
                        let n = self.macro_name(rest, d.at, name)?;
                        self.is_defined(&n) == (name == "ifdef")
                    };
                    conds.push(Cond { outer: on, on: on && v, taken: !on || v, seen_else: false, line: at.1 });
                }
                "elif" => {
                    let Some(c) = conds.last() else { return fail(at, "#elif without #if") };
                    if c.seen_else {
                        return fail(at, "#elif after #else");
                    }
                    // a branch after the one taken is not even evaluated
                    let v = if c.outer && !c.taken { self.condition(rest, d.at)? } else { false };
                    let c = conds.last_mut().unwrap();
                    c.on = v;
                    c.taken |= v;
                }
                "else" => {
                    let Some(c) = conds.last_mut() else { return fail(at, "#else without #if") };
                    if c.seen_else {
                        return fail(at, "#else after #else");
                    }
                    c.seen_else = true;
                    c.on = c.outer && !c.taken;
                    c.taken = true;
                }
                "endif" => {
                    if conds.pop().is_none() {
                        return fail(at, "#endif without #if");
                    }
                }
                _ if !on => {} // a skipped group's other lines are not read
                "define" => self.define(rest, d.at)?,
                "undef" => {
                    let n = self.macro_name(rest, d.at, name)?;
                    self.macros.remove(&n);
                }
                "include" => self.include(id, rest, d.at)?,
                "error" => return fail(at, format!("#error {}", spell(rest))),
                // a warning changes nothing in the program; clang only prints it
                "warning" => {}
                "pragma" => {
                    if rest.len() == 1 && rest[0].s == "once" {
                        self.once.insert(once_key(&self.files[id as usize]));
                    } else {
                        return fail(at, format!("#pragma {} is not in Strike yet (only #pragma once)", spell(rest)));
                    }
                }
                "line" | "include_next" | "import" | "ident" | "sccs" | "assert" | "unassert" | "embed" => {
                    return fail(at, format!("#{name} is not in Strike yet"));
                }
                _ => return fail(at, format!("invalid preprocessing directive #{}", d.s)),
            }
        }
        if let Some(c) = conds.last() {
            return fail((id, c.line), "unterminated conditional directive (an #if without its #endif)");
        }
        Ok(())
    }

    fn macro_name(&self, rest: &[Tok], at: (u32, u32), what: &str) -> R<String> {
        match rest.first() {
            Some(t) if t.k == K::Ident => {
                if t.s == "defined" {
                    return fail(t.at, "'defined' cannot be used as a macro name");
                }
                Ok(t.s.clone())
            }
            Some(t) => fail(t.at, format!("#{what}: macro name must be an identifier")),
            None => fail(at, format!("#{what}: macro name missing")),
        }
    }

    fn is_defined(&self, name: &str) -> bool {
        self.macros.contains_key(name) || BUILT_IN.contains(&name)
    }

    fn define(&mut self, rest: &[Tok], at: (u32, u32)) -> R<()> {
        let name = self.macro_name(rest, at, "define")?;
        let mut j = 1;
        let mut params = None;
        let mut variadic = false;
        // a ( right after the name, with no space: a function-like macro
        if rest.get(1).is_some_and(|t| t.is("(") && !t.space) {
            let mut ps: Vec<String> = Vec::new();
            j = 2;
            loop {
                let Some(t) = rest.get(j) else { return fail(at, "a macro's parameter list that never closes") };
                j += 1;
                if t.is(")") && ps.is_empty() && !variadic {
                    break;
                }
                if t.is("...") {
                    variadic = true;
                } else if t.k == K::Ident && !variadic {
                    if ps.contains(&t.s) {
                        return fail(t.at, format!("duplicate macro parameter name {}", t.s));
                    }
                    if t.s == "__VA_ARGS__" {
                        return fail(t.at, "__VA_ARGS__ can only appear in the expansion of a variadic macro");
                    }
                    ps.push(t.s.clone());
                    if rest.get(j).is_some_and(|t| t.is("...")) {
                        return fail(t.at, "a named variadic parameter (GNU's name...) is not in Strike yet");
                    }
                } else {
                    return fail(t.at, format!("invalid token {} in a macro's parameter list", t.s));
                }
                let Some(t) = rest.get(j) else { return fail(at, "a macro's parameter list that never closes") };
                j += 1;
                if t.is(")") {
                    break;
                }
                if !t.is(",") || variadic {
                    return fail(t.at, "expected , or ) in a macro's parameter list");
                }
            }
            if variadic {
                ps.push("__VA_ARGS__".into());
            }
            params = Some(ps);
        }
        let body: Vec<Tok> = rest[j..].to_vec();
        if body.first().is_some_and(|t| t.is("##")) || body.last().is_some_and(|t| t.is("##")) {
            return fail(at, "'##' cannot appear at either end of a macro expansion");
        }
        for (k, t) in body.iter().enumerate() {
            if t.k == K::Ident && (t.s == "__VA_ARGS__") && !variadic {
                return fail(t.at, "__VA_ARGS__ can only appear in the expansion of a variadic macro");
            }
            if t.k == K::Ident && t.s == "__VA_OPT__" {
                return fail(t.at, "__VA_OPT__ is not in Strike yet");
            }
            if let Some(ps) = &params {
                if t.is("#") && !body.get(k + 1).is_some_and(|n| n.k == K::Ident && ps.contains(&n.s)) {
                    return fail(t.at, "'#' is not followed by a macro parameter");
                }
                // GNU's , ## __VA_ARGS__ drops the comma when there are no
                // arguments; the standard's ## does not
                if t.is("##") && variadic && body.get(k + 1).is_some_and(|n| n.s == "__VA_ARGS__") && k > 0 && body[k - 1].is(",") {
                    return fail(t.at, "GNU's , ## __VA_ARGS__ is not in Strike yet");
                }
            }
            if DIGRAPHS.contains(&t.s.as_str()) && t.k == K::Punct {
                return fail(t.at, format!("the digraph {} is not in Strike yet", t.s));
            }
        }
        let mut body = body;
        if let Some(f) = body.first_mut() {
            f.space = false;
        }
        self.macros.insert(name, Macro { params, variadic, body });
        Ok(())
    }

    fn include(&mut self, id: u32, rest: &[Tok], at: (u32, u32)) -> R<()> {
        let (name, quoted) = match rest.first() {
            Some(t) if t.k == K::Str => (t.s[1..t.s.len() - 1].to_string(), true),
            Some(t) if t.is("<") => {
                let Some(close) = rest.iter().position(|t| t.is(">")) else { return fail(at, "expected > after the header's name") };
                (spell(&rest[1..close]), false)
            }
            Some(_) => return fail(at, "a computed #include (a macro that names the file) is not in Strike yet"),
            None => return fail(at, "#include expects \"FILENAME\" or <FILENAME>"),
        };
        if self.depth >= 200 {
            return fail(at, "#include nested too deeply");
        }
        let here = &self.files[id as usize];
        let mut found = None;
        if quoted && super::hot::is_file(here) {
            if let Some(dir) = here.parent() {
                let p = dir.join(&name);
                if super::hot::is_file(&p) {
                    found = Some(p);
                }
            }
        }
        if found.is_none() {
            found = self.include.iter().map(|d| d.join(&name)).find(|p| super::hot::is_file(p));
        }
        let (path, text) = match found {
            Some(p) => {
                let text = super::hot::read_file(&p).map_err(|e| (at, format!("{}: {e}", p.display())))?;
                (p, text)
            }
            None => match self.rule.headers.get(&name) {
                // a header Strike knows: what the description says it gives
                Some(t) => (PathBuf::from(format!("<{name}>")), t.clone()),
                None if quoted => {
                    return fail(at, format!("'{name}' file not found (next to this file, in -I, or among the headers Strike knows: {})", self.known()))
                }
                None => return fail(at, format!("the header <{name}> is not one Strike knows (it knows {})", self.known())),
            },
        };
        if self.once.contains(&once_key(&path)) {
            return Ok(());
        }
        if path.to_string_lossy().starts_with('<') {
            self.once.insert(path.clone()); // a header Strike knows is read once
        }
        let sub = self.file_id(path);
        self.depth += 1;
        let r = self.run(sub, &text);
        self.depth -= 1;
        r
    }

    fn known(&self) -> String {
        let mut v: Vec<&String> = self.rule.headers.keys().collect();
        v.sort();
        v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
    }

    /// A line of text, expanded; an invocation that goes on past the line
    /// takes the lines after it that are not directives.
    fn text(&mut self, lines: &[Vec<Tok>], i: &mut usize, line: Vec<Tok>) -> R<()> {
        let marker = self.rule.marker.clone();
        let mut more = |input: &mut VecDeque<Tok>| -> bool {
            match lines.get(*i) {
                Some(l) if !(l[0].is(&marker) && l[0].k == K::Punct) => {
                    *i += 1;
                    let mut l = l.clone();
                    l[0].space = true; // the newline between is whitespace
                    input.extend(l);
                    true
                }
                _ => false,
            }
        };
        let out = self.expand(line.into(), Some(&mut more))?;
        for t in &out {
            if t.k == K::Other {
                if t.s == "\"" || t.s == "'" {
                    return fail(t.at, "a string or character that never closes");
                }
                return fail(t.at, format!("C has no {:?}", t.s));
            }
            if t.k == K::Punct && DIGRAPHS.contains(&t.s.as_str()) {
                return fail(t.at, format!("the digraph {} is not in Strike yet", t.s));
            }
        }
        self.out.extend(out);
        Ok(())
    }

    /// Expands the tokens: every macro called, and what it gives read
    /// again with the tokens after it. `more` gives further lines where
    /// an invocation needs them (none inside an argument, which is whole).
    fn expand(&mut self, mut input: VecDeque<Tok>, mut more: Option<&mut Pull>) -> R<Vec<Tok>> {
        let mut out = Vec::new();
        while let Some(t) = input.pop_front() {
            if t.k != K::Ident || t.hide.contains(&t.s) {
                out.push(t);
                continue;
            }
            match t.s.as_str() {
                "__LINE__" if !self.macros.contains_key("__LINE__") => {
                    out.push(Tok { k: K::Num, s: t.at.1.to_string(), ..t });
                    continue;
                }
                "__COUNTER__" if !self.macros.contains_key("__COUNTER__") => {
                    self.counter += 1;
                    out.push(Tok { k: K::Num, s: (self.counter - 1).to_string(), ..t });
                    continue;
                }
                s if BUILT_IN.contains(&s) && !self.macros.contains_key(s) => {
                    return fail(t.at, format!("{s} depends on how and when clang is run, so it is not in Strike"));
                }
                "_Pragma" | "__pragma" => return fail(t.at, format!("{} is not in Strike yet", t.s)),
                _ => {}
            }
            let Some(m) = self.macros.get(&t.s).cloned() else {
                out.push(t);
                continue;
            };
            let Some(params) = &m.params else {
                let mut hide = (*t.hide).clone();
                hide.insert(t.s.clone());
                let body = self.subst(&m, &[], Rc::new(hide), &t)?;
                for b in body.into_iter().rev() {
                    input.push_front(b);
                }
                continue;
            };
            // a function-like macro is called only by a ( after its name
            if input.is_empty() {
                if let Some(pull) = more.as_mut() {
                    while input.is_empty() && pull(&mut input) {}
                }
            }
            if !input.front().is_some_and(|n| n.is("(")) {
                out.push(t);
                continue;
            }
            input.pop_front();
            let mut args: Vec<Vec<Tok>> = vec![Vec::new()];
            let mut depth = 0;
            let close = loop {
                let Some(a) = input.pop_front() else {
                    if more.as_mut().is_some_and(|pull| pull(&mut input)) {
                        continue;
                    }
                    return fail(t.at, format!("unterminated function-like macro invocation of {}", t.s));
                };
                if a.is("(") {
                    depth += 1;
                } else if a.is(")") {
                    if depth == 0 {
                        break a;
                    }
                    depth -= 1;
                } else if a.is(",") && depth == 0 && !(m.variadic && args.len() == params.len()) {
                    args.push(Vec::new());
                    continue;
                }
                args.last_mut().unwrap().push(a);
            };
            let named = params.len() - m.variadic as usize;
            if params.is_empty() && args.len() == 1 && args[0].is_empty() {
                args.clear();
            }
            if m.variadic && args.len() == named {
                args.push(Vec::new()); // F(a) for F(a, ...): no variable arguments
            }
            if args.len() != params.len() {
                return fail(t.at, format!("the macro {} takes {} arguments, given {}", t.s, params.len(), args.len()));
            }
            let mut hide: BTreeSet<String> = t.hide.intersection(&close.hide).cloned().collect();
            hide.insert(t.s.clone());
            let body = self.subst(&m, &args, Rc::new(hide), &t)?;
            for b in body.into_iter().rev() {
                input.push_front(b);
            }
        }
        Ok(out)
    }

    /// A macro's body with its parameters replaced: by the argument
    /// expanded, by its text as a string after #, or as written next to
    /// ##; then ## pastes the tokens on its sides into one.
    fn subst(&mut self, m: &Macro, args: &[Vec<Tok>], hide: Hide, call: &Tok) -> R<Vec<Tok>> {
        let params: &[String] = m.params.as_deref().unwrap_or(&[]);
        let arg = |t: &Tok| if t.k == K::Ident { params.iter().position(|p| *p == t.s) } else { None };
        let mut expanded: HashMap<usize, Vec<Tok>> = HashMap::new();
        let mut os: Vec<Tok> = Vec::new();
        let body = &m.body;
        let place = |t: &Tok| Tok { k: K::Placemarker, s: String::new(), ..t.clone() };
        let mut j = 0;
        while j < body.len() {
            let b = &body[j];
            if m.params.is_some() && b.is("#") && b.k == K::Punct {
                let p = arg(&body[j + 1]).unwrap();
                os.push(Tok { k: K::Str, s: stringize(&args[p]), ..b.clone() });
                j += 2;
                continue;
            }
            if b.is("##") && b.k == K::Punct {
                let r = &body[j + 1];
                let rhs: Vec<Tok> = match arg(r) {
                    Some(p) if args[p].is_empty() => vec![place(r)],
                    Some(p) => args[p].clone(),
                    None => vec![r.clone()],
                };
                let lhs = os.pop().unwrap();
                os.push(paste(&lhs, &rhs[0])?);
                os.extend(rhs[1..].iter().cloned());
                j += 2;
                continue;
            }
            if let Some(p) = arg(b) {
                let next_paste = body.get(j + 1).is_some_and(|n| n.is("##"));
                let mut toks = if next_paste {
                    if args[p].is_empty() {
                        vec![place(b)]
                    } else {
                        args[p].clone()
                    }
                } else {
                    if let std::collections::hash_map::Entry::Vacant(v) = expanded.entry(p) {
                        v.insert(self.expand(args[p].iter().cloned().collect(), None)?);
                    }
                    expanded[&p].clone()
                };
                if let Some(f) = toks.first_mut() {
                    f.space = b.space;
                }
                os.extend(toks);
                j += 1;
                continue;
            }
            os.push(b.clone());
            j += 1;
        }
        let mut out: Vec<Tok> = os.into_iter().filter(|t| t.k != K::Placemarker).collect();
        for (n, t) in out.iter_mut().enumerate() {
            let mut h = (*t.hide).clone();
            h.extend(hide.iter().cloned());
            t.hide = Rc::new(h);
            t.at = call.at;
            if n == 0 {
                t.space = call.space;
            }
        }
        Ok(out)
    }

    /// An #if's or #elif's condition.
    fn condition(&mut self, rest: &[Tok], at: (u32, u32)) -> R<bool> {
        // defined X and defined(X) are read before any macro is expanded
        let mut toks: VecDeque<Tok> = VecDeque::new();
        let mut j = 0;
        while j < rest.len() {
            let t = &rest[j];
            if t.k == K::Ident && t.s.starts_with("__has_") || t.s == "__is_identifier" {
                return fail(t.at, format!("{} is not in Strike yet", t.s));
            }
            if t.k == K::Ident && t.s == "defined" {
                let (name, next) = match (rest.get(j + 1), rest.get(j + 2), rest.get(j + 3)) {
                    (Some(p), Some(n), Some(c)) if p.is("(") && n.k == K::Ident && c.is(")") => (n.s.clone(), j + 4),
                    (Some(n), _, _) if n.k == K::Ident => (n.s.clone(), j + 2),
                    _ => return fail(t.at, "defined: macro name must be an identifier"),
                };
                toks.push_back(Tok { k: K::Num, s: if self.is_defined(&name) { "1" } else { "0" }.into(), ..t.clone() });
                j = next;
                continue;
            }
            toks.push_back(t.clone());
            j += 1;
        }
        let toks = self.expand(toks, None)?;
        if let Some(t) = toks.iter().find(|t| t.k == K::Ident && t.s == "defined") {
            return fail(t.at, "a macro that gives `defined` in #if is not in Strike yet");
        }
        if toks.is_empty() {
            return fail(at, "#if with no expression");
        }
        let mut e = Eval { t: &toks, i: 0 };
        let v = e.cond(true)?;
        if let Some(t) = toks.get(e.i) {
            return fail(t.at, format!("{} is not a binary operator in an #if", t.s));
        }
        Ok(v.v != 0)
    }
}

/// `#x`: the argument's text as a string, a space where there was
/// whitespace, and the quotes and backslashes of strings escaped.
fn stringize(arg: &[Tok]) -> String {
    let mut s = String::from("\"");
    for (i, t) in arg.iter().enumerate() {
        if i > 0 && t.space {
            s.push(' ');
        }
        if matches!(t.k, K::Str | K::Char) {
            for c in t.s.chars() {
                if c == '"' || c == '\\' {
                    s.push('\\');
                }
                s.push(c);
            }
        } else {
            s.push_str(&t.s);
        }
    }
    s.push('"');
    s
}

/// a ## b: the two tokens' text read as one token, which it must be.
fn paste(a: &Tok, b: &Tok) -> R<Tok> {
    if a.k == K::Placemarker {
        return Ok(Tok { space: a.space, ..b.clone() });
    }
    if b.k == K::Placemarker {
        return Ok(a.clone());
    }
    let text = format!("{}{}", a.s, b.s);
    let lines = tokenize(&text, a.at.0).map_err(|(_, m)| (a.at, m))?;
    match lines.as_slice() {
        [one] if one.len() == 1 && one[0].k != K::Other => Ok(Tok { k: one[0].k, s: text, ..a.clone() }),
        _ => fail(a.at, format!("pasting formed '{text}', an invalid preprocessing token")),
    }
}

/// A number of #if: intmax_t, or uintmax_t once a value is unsigned.
#[derive(Clone, Copy)]
struct V {
    v: u64,
    u: bool,
}

struct Eval<'a> {
    t: &'a [Tok],
    i: usize,
}

impl Eval<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn at(&self) -> (u32, u32) {
        self.t.get(self.i).or(self.t.last()).map(|t| t.at).unwrap_or((0, 0))
    }
    fn eat(&mut self, p: &str) -> bool {
        if self.peek().is_some_and(|t| t.k == K::Punct && t.s == p) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    /// a ? b : c, where only the branch taken is evaluated
    fn cond(&mut self, live: bool) -> R<V> {
        let c = self.bin(1, live)?;
        if !self.eat("?") {
            return Ok(c);
        }
        let a = self.cond(live && c.v != 0)?;
        if !self.eat(":") {
            return fail(self.at(), "expected : in an #if's ?:");
        }
        let b = self.cond(live && c.v == 0)?;
        let u = a.u || b.u;
        Ok(V { v: if c.v != 0 { a.v } else { b.v }, u })
    }
    fn bin(&mut self, min: u8, live: bool) -> R<V> {
        let mut a = self.unary(live)?;
        while let Some(t) = self.peek() {
            if t.k != K::Punct {
                break;
            }
            let p = match t.s.as_str() {
                "||" => 1,
                "&&" => 2,
                "|" => 3,
                "^" => 4,
                "&" => 5,
                "==" | "!=" => 6,
                "<" | ">" | "<=" | ">=" => 7,
                "<<" | ">>" => 8,
                "+" | "-" => 9,
                "*" | "/" | "%" => 10,
                "," => return fail(t.at, "the comma operator in an #if is not in Strike yet"),
                _ => break,
            };
            if p < min {
                break;
            }
            let op = t.s.clone();
            let at = t.at;
            self.i += 1;
            // && and || evaluate their right side only where it decides
            let rlive = match op.as_str() {
                "&&" => live && a.v != 0,
                "||" => live && a.v == 0,
                _ => live,
            };
            let b = self.bin(p + 1, rlive)?;
            a = binary(&op, a, b, rlive, at)?;
        }
        Ok(a)
    }
    fn unary(&mut self, live: bool) -> R<V> {
        let Some(t) = self.peek().cloned() else { return fail(self.at(), "an #if's expression ends early") };
        self.i += 1;
        match (t.k, t.s.as_str()) {
            (K::Punct, "(") => {
                let v = self.cond(live)?;
                if !self.eat(")") {
                    return fail(self.at(), "expected ) in an #if");
                }
                Ok(v)
            }
            (K::Punct, "+") => self.unary(live),
            (K::Punct, "-") => {
                let v = self.unary(live)?;
                Ok(V { v: v.v.wrapping_neg(), u: v.u })
            }
            (K::Punct, "~") => {
                let v = self.unary(live)?;
                Ok(V { v: !v.v, u: v.u })
            }
            (K::Punct, "!") => {
                let v = self.unary(live)?;
                Ok(V { v: (v.v == 0) as u64, u: false })
            }
            (K::Num, s) => number(s, t.at),
            (K::Char, s) => character(s, t.at),
            // a name left after expansion is 0, as C says
            (K::Ident, _) => Ok(V { v: 0, u: false }),
            _ => fail(t.at, format!("{} cannot begin an #if's expression", t.s)),
        }
    }
}

fn binary(op: &str, a: V, b: V, live: bool, at: (u32, u32)) -> R<V> {
    let u = a.u || b.u;
    let (x, y) = (a.v, b.v);
    let (sx, sy) = (x as i64, y as i64);
    let truth = |c: bool| V { v: c as u64, u: false };
    Ok(match op {
        "||" => truth(x != 0 || y != 0),
        "&&" => truth(x != 0 && y != 0),
        "|" => V { v: x | y, u },
        "^" => V { v: x ^ y, u },
        "&" => V { v: x & y, u },
        "==" => truth(x == y),
        "!=" => truth(x != y),
        "<" => truth(if u { x < y } else { sx < sy }),
        ">" => truth(if u { x > y } else { sx > sy }),
        "<=" => truth(if u { x <= y } else { sx <= sy }),
        ">=" => truth(if u { x >= y } else { sx >= sy }),
        "<<" | ">>" => {
            if live && (b.u && y >= 64 || !b.u && !(0..64).contains(&sy)) {
                return fail(at, "a shift by a count outside 0 to 63 in an #if is not in Strike");
            }
            let n = (y & 63) as u32;
            let v = match (op, a.u) {
                ("<<", _) => x.wrapping_shl(n),
                (_, true) => x >> n,
                (_, false) => (sx >> n) as u64,
            };
            V { v, u: a.u }
        }
        "+" => V { v: x.wrapping_add(y), u },
        "-" => V { v: x.wrapping_sub(y), u },
        "*" => V { v: x.wrapping_mul(y), u },
        "/" | "%" => {
            if y == 0 {
                if live {
                    return fail(at, format!("{} by zero in an #if", if op == "/" { "division" } else { "remainder" }));
                }
                return Ok(V { v: 0, u });
            }
            let v = match (op, u) {
                ("/", true) => x / y,
                ("/", false) => sx.wrapping_div(sy) as u64,
                (_, true) => x % y,
                (_, false) => sx.wrapping_rem(sy) as u64,
            };
            V { v, u }
        }
        _ => unreachable!(),
    })
}

/// A number in an #if: decimal, octal, hex or binary, with u and l
/// suffixes; a value past intmax_t is unsigned, as clang reads it.
fn number(s: &str, at: (u32, u32)) -> R<V> {
    let lower = s.to_ascii_lowercase();
    let end = lower.trim_end_matches(['u', 'l']);
    let suffix = &lower[end.len()..];
    if !matches!(suffix, "" | "u" | "l" | "ul" | "lu" | "ll" | "ull" | "llu") {
        return fail(at, format!("{s} is not a whole number an #if can read"));
    }
    let (digits, radix) = if let Some(h) = end.strip_prefix("0x") {
        (h, 16)
    } else if let Some(b) = end.strip_prefix("0b") {
        (b, 2)
    } else if end.len() > 1 && end.starts_with('0') {
        (&end[1..], 8)
    } else {
        (end, 10)
    };
    let v = u64::from_str_radix(digits, radix).map_err(|_| (at, format!("{s} is not a whole number an #if can read")))?;
    Ok(V { v, u: suffix.contains('u') || v > i64::MAX as u64 })
}

/// A character constant in an #if: an int, from a char that is signed.
fn character(s: &str, at: (u32, u32)) -> R<V> {
    let inner: Vec<char> = s[1..s.len() - 1].chars().collect();
    let c = match inner.as_slice() {
        [c] if *c != '\\' && c.is_ascii() => *c as u8,
        ['\\', e] => match e {
            'n' => b'\n',
            't' => b'\t',
            'r' => b'\r',
            '0' => 0,
            '\\' => b'\\',
            '\'' => b'\'',
            '"' => b'"',
            'a' => 7,
            'b' => 8,
            'f' => 12,
            'v' => 11,
            _ => return fail(at, format!("the character {s} is not in Strike yet")),
        },
        _ => return fail(at, format!("the character {s} is not in Strike yet")),
    };
    Ok(V { v: c as i8 as i64 as u64, u: false })
}

/// Preprocesses a C file: the compiler's own macros first, then the file.
pub fn run(rule: &Preprocessor, path: &Path, text: &str, include: &[PathBuf]) -> Result<Output, Error> {
    let mut pp = Pp { rule, macros: HashMap::new(), files: Vec::new(), once: HashSet::new(), include, out: Vec::new(), depth: 0, counter: 0 };
    let pre = pp.file_id(PathBuf::from("<built-in>"));
    let main = pp.file_id(path.to_path_buf());
    let err = |pp: &Pp, ((f, line), message): ((u32, u32), String)| Error { file: pp.files[f as usize].clone(), line, message };
    pp.run(pre, &rule.predefined).map_err(|e| err(&pp, e))?;
    pp.out.clear();
    pp.run(main, text).map_err(|e| err(&pp, e))?;
    let mut text = String::new();
    let mut lines = Vec::new();
    let mut cur = None;
    for t in &pp.out {
        if cur != Some(t.at) {
            if cur.is_some() {
                text.push('\n');
            }
            cur = Some(t.at);
            lines.push((pp.files[t.at.0 as usize].clone(), t.at.1));
        } else {
            text.push(' ');
        }
        text.push_str(&t.s);
    }
    text.push('\n');
    let files = pp.files.into_iter().filter(|f| !f.to_string_lossy().starts_with('<')).collect();
    Ok(Output { text, lines, files })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> Preprocessor {
        let langs = super::super::languages().unwrap();
        let c = langs.iter().find(|l| l.name == "C").unwrap();
        let p = c.preprocessor.as_ref().unwrap();
        Preprocessor { marker: p.marker.clone(), predefined: p.predefined.clone(), headers: p.headers.clone() }
    }

    /// The tokens of a text, as C's preprocessor cuts them.
    fn tokens(text: &str) -> Vec<String> {
        tokenize(text, 0).unwrap().into_iter().flatten().map(|t| t.s).collect()
    }

    fn pp(text: &str) -> Result<Vec<String>, String> {
        let r = rule();
        run(&r, Path::new("t.c"), text, &[]).map(|o| tokens(&o.text)).map_err(|e| format!("{}:{}: {}", e.file.display(), e.line, e.message))
    }

    /// Every file in tests/uc/cpp/ gives, token for token, what `clang -E
    /// -P` gave for it (the .E file next to it, written by clang).
    #[test]
    fn the_preprocessor_gives_what_clang_gives() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/uc/cpp");
        let r = rule();
        let mut n = 0;
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "c") {
                let text = std::fs::read_to_string(&p).unwrap();
                let got = run(&r, &p, &text, &[dir.join("include")]).unwrap_or_else(|e| panic!("{}:{}: {}", e.file.display(), e.line, e.message));
                let want = std::fs::read_to_string(p.with_extension("E")).unwrap();
                assert_eq!(tokens(&got.text), tokens(&want), "{}", p.display());
                n += 1;
            }
        }
        assert!(n >= 3);
    }

    #[test]
    fn lines_are_kept_through_macros_and_comments() {
        let r = rule();
        let o = run(&r, Path::new("t.c"), "#define ADD(a, b) \\\n  ((a) + (b))\n/* two\n lines */ int x =\nADD(1,\n 2);\n", &[]).unwrap();
        let lines: Vec<u32> = o.lines.iter().map(|l| l.1).collect();
        // `int x =` is on line 4, the call on line 5 with what it gives, and
        // the `;` after it on line 6
        assert_eq!(lines, vec![4, 5, 6]);
        assert_eq!(o.text, "int x =\n( ( 1 ) + ( 2 ) )\n;\n");
    }

    #[test]
    fn what_the_preprocessor_cannot_do_is_refused() {
        for (src, says) in [
            ("#pragma pack(1)\n", "#pragma pack"),
            ("#include <windows.h>\n", "not one Strike knows"),
            ("#include \"nowhere.h\"\n", "file not found"),
            ("#if 1\nint x;\n", "unterminated conditional"),
            ("#error stop here\n", "#error stop here"),
            ("#line 7\n", "#line is not in Strike"),
            ("#define F(x) x\nF(1\n", "unterminated function-like macro"),
            ("#define F(x, y) x\nF(1)\n", "takes 2 arguments, given 1"),
            ("#define P(a, b) a ## b\nP(+, /)\n", "pasting formed '+/'"),
            ("#if 1 / 0\n#endif\n", "division by zero"),
            ("#if __has_include(<x.h>)\n#endif\n", "__has_include"),
            ("const char *f = __FILE__;\n", "__FILE__"),
            ("#foo\n", "invalid preprocessing directive"),
            ("int a<:3:>;\n", "digraph"),
        ] {
            let e = pp(src).unwrap_err();
            assert!(e.contains(says), "{src:?}: {e}");
        }
        // the line of a refusal is the line it is on
        assert!(pp("int a;\n\n#pragma weak f\n").unwrap_err().starts_with("t.c:3:"));
    }

    #[test]
    fn conditionals_and_expressions_are_read_as_c_reads_them() {
        let t = |s: &str| pp(s).unwrap().join(" ");
        assert_eq!(t("#if -1 < 0u\nyes\n#else\nno\n#endif\n"), "no"); // -1 becomes unsigned
        assert_eq!(t("#if 0 && 1 / 0\n#elif 2 > 1 ? 3 : 1 / 0\nyes\n#endif\n"), "yes");
        assert_eq!(t("#define A\n#if defined A && !defined(B)\nyes\n#endif\n"), "yes");
        assert_eq!(t("#if UNDEFINED_NAME == 0 && '\\n' == 10\nyes\n#endif\n"), "yes");
        assert_eq!(t("#ifdef _WIN32\nwin\n#endif\n#ifndef __linux__\nnot_linux\n#endif\n"), "win not_linux");
        assert_eq!(t("#if 0\n#if garbage (\n#bogus\n#endif\n#else\nok\n#endif\n"), "ok");
        assert_eq!(t("__LINE__\n\n__LINE__ __COUNTER__ __COUNTER__\n"), "1 3 0 1");
    }
}
