//! A language as data (languages/*.toml): how its text is cut into tokens,
//! how its forms are written, and which universal construct each form is.
//! The compiler holds no language of its own; a new language is a new file.

use super::ir::{BinOp, UnOp};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const BUILT_IN: [&str; 3] = [
    include_str!("../../languages/c.toml"),
    include_str!("../../languages/go.toml"),
    include_str!("../../languages/python.toml"),
];
pub const JUDGES: &str = include_str!("../../languages/judges.toml");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    name: String,
    extensions: Vec<String>,
    #[serde(default)]
    line_comments: Vec<String>,
    #[serde(default)]
    block_comments: Vec<[String; 2]>,
    #[serde(default)]
    skip_lines: Vec<String>,
    #[serde(default)]
    string_quotes: Vec<String>,
    #[serde(default)]
    char_quotes: Vec<String>,
    blocks: String,
    statement_end: String,
    #[serde(default)]
    end_after: Vec<String>,
    scope: String,
    entry: String,
    param_name: String,
    #[serde(default)]
    type_words: Vec<String>,
    chained_comparisons: String,
    #[serde(default)]
    bool_print: Vec<String>,
    #[serde(default)]
    overflow: Option<String>,
    #[serde(default)]
    read_line: Option<[String; 2]>,
    #[serde(default)]
    promote: Option<u8>,
    #[serde(default)]
    literals: Option<String>,
    #[serde(default)]
    int_widths: BTreeMap<String, u8>,
    /// the float types and their widths (64: a double, 32: a float32)
    #[serde(default)]
    float_widths: BTreeMap<String, u8>,
    /// the letters after a float literal that make it a float32 (C's 1.5f)
    #[serde(default)]
    float_suffixes: Vec<String>,
    /// how print (Println) prints a float: "repr" (Python's) or "shortest"
    /// (Go's %v)
    #[serde(default)]
    float_print: Option<String>,
    /// whose words a format prints for NaN and the infinities: "ucrt"
    /// (C's printf on Windows) or "go"
    #[serde(default)]
    printf_floats: Option<String>,
    /// (type) value is a conversion (C's casts)
    #[serde(default)]
    casts: bool,
    /// module.name(...) calls: "module.name" -> what it is
    #[serde(default)]
    qualified: BTreeMap<String, String>,
    /// what an index out of range does: the exit code and the message
    /// (none: nothing is checked, as C does nothing)
    #[serde(default)]
    index_error: Option<(u8, String)>,
    /// what a division by zero does: the exit code and the message (none:
    /// a float's infinity or NaN, and a whole number's as the machine does)
    #[serde(default)]
    zero_division: Option<(u8, String)>,
    /// a[-1] is the last element (Python)
    #[serde(default)]
    negative_index: bool,
    /// [a, b] is a list (Python)
    #[serde(default)]
    list_literals: bool,
    /// [e for x in xs] (Python)
    #[serde(default)]
    comprehensions: bool,
    /// []int{a, b}, [3]int{a, b, c} (Go)
    #[serde(default)]
    composite_literals: bool,
    /// {a, b} as a value (C's initializers)
    #[serde(default)]
    brace_lists: bool,
    /// how a list prints: before, between, after
    #[serde(default)]
    list_print: Option<[String; 3]>,
    /// functions every program has: the language's name -> what it is
    #[serde(default)]
    builtins: BTreeMap<String, String>,
    /// x.name(...) on a list: the name -> what it is
    #[serde(default)]
    methods: BTreeMap<String, String>,
    #[serde(default)]
    bool_constants: BTreeMap<String, i64>,
    #[serde(default)]
    binary: BTreeMap<String, (u8, String)>,
    #[serde(default)]
    unary: BTreeMap<String, (u8, String)>,
    #[serde(default)]
    form: Vec<RawForm>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawForm {
    #[serde(default)]
    name: Option<String>,
    kind: String,
    #[serde(default)]
    place: Option<String>,
    #[serde(default)]
    op: Option<String>,
    #[serde(default)]
    value: Option<i64>,
    #[serde(default)]
    style: Option<String>,
    /// for_each: what each $name of the pattern is ("index", "value")
    #[serde(default)]
    names: Option<Vec<String>>,
    pattern: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum End {
    Semicolon,
    Newline,
    NewlineAfterValue,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Function,
    Ignore,
    Var,
    Assign,
    If,
    While,
    Loop,
    For,
    ForRange,
    Return,
    Break,
    Continue,
    Nothing,
    Print,
    Expr,
    /// Python's `global x`: x is the module's in this function
    Global,
    /// a whole number read from standard input into a variable
    Read,
    /// a loop over the elements of a list, a slice or an array
    ForEach,
}

/// What a $name of a for_each form is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Index,
    Value,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    Top,
    Stmt,
    /// only reached through @name
    Sub,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    None,
    /// printf: the first argument is the format
    Format,
    /// puts: one string and a newline
    Line,
    /// print, Println: the values with spaces between, and a newline
    SpacedLine,
}

/// How print (Println) prints a float.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FloatPrint {
    /// Python's repr: the shortest digits, the point from 1e-4 to 1e16
    Repr,
    /// Go's %v: the shortest digits as %g, an exponent below 1e-4 and
    /// from 1e6 on
    Shortest,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cap {
    Name,
    Expr,
    Block,
    Body,
    Params,
    Args,
    Type,
    End,
    Simple,
    Str,
    /// a name with indexes after it: what an assignment changes (a[i][j])
    Place,
    /// zero or more [size] after a declared name (C's arrays)
    Dims,
}

#[derive(Clone, Debug)]
pub enum Pat {
    Lit(String),
    Cap(Cap),
    Sub(String),
    /// one of the alternatives, or nothing
    Opt(Vec<Vec<Pat>>),
}

pub struct Form {
    pub name: Option<String>,
    pub kind: Kind,
    pub place: Place,
    pub op: Option<BinOp>,
    pub value: Option<i64>,
    pub style: Style,
    pub names: Vec<Role>,
    pub pat: Vec<Pat>,
}

pub struct Lang {
    pub name: String,
    pub extensions: Vec<String>,
    pub line_comments: Vec<String>,
    pub block_comments: Vec<(String, String)>,
    pub skip_lines: Vec<String>,
    pub string_quotes: Vec<u8>,
    pub char_quotes: Vec<u8>,
    pub indent: bool,
    pub end: End,
    pub end_after: HashSet<String>,
    pub block_scope: bool,
    pub toplevel_entry: bool,
    pub param_first: bool,
    pub type_words: HashSet<String>,
    pub chain_error: bool,
    /// the language's numbers do not wrap (Python): arithmetic that would
    /// leave 64 bits stops with an error
    pub checked: bool,
    /// bits of each integer type (none: every number is 64 bits)
    pub int_widths: HashMap<String, u8>,
    pub float_widths: HashMap<String, u8>,
    pub float_suffixes: Vec<String>,
    pub float_print: Option<FloatPrint>,
    /// a format prints NaN and the infinities as Go does (else as the UCRT)
    pub printf_go: Option<bool>,
    pub casts: bool,
    pub qualified: HashMap<String, String>,
    /// the narrowest width arithmetic is done at (C: int)
    pub promote: u8,
    /// a literal takes the width of what it meets (Go) instead of its own
    pub untyped_literals: bool,
    /// the call that reads a whole line as a number: outer(inner())
    pub read_line: Option<(String, String)>,
    pub bool_print: Option<(String, String)>,
    pub index_error: Option<(u8, String)>,
    pub zero_division: Option<(u8, String)>,
    pub negative_index: bool,
    pub list_literals: bool,
    pub comprehensions: bool,
    pub composite_literals: bool,
    pub brace_lists: bool,
    pub list_print: (String, String, String),
    pub builtins: HashMap<String, String>,
    pub methods: HashMap<String, String>,
    pub bool_constants: HashMap<String, i64>,
    pub binary: HashMap<String, (u8, BinOp)>,
    pub unary: HashMap<String, (u8, UnOp)>,
    pub forms: Vec<Form>,
    /// every punctuation token, longest first
    pub puncts: Vec<String>,
    /// words that are never a name
    pub keywords: HashSet<String>,
}

fn is_word(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
}

fn one_byte(list: &[String], what: &str) -> Result<Vec<u8>, String> {
    list.iter()
        .map(|q| if q.len() == 1 { Ok(q.as_bytes()[0]) } else { Err(format!("{what} must be one character, got {q:?}")) })
        .collect()
}

fn compile_seq(words: &[&str], i: &mut usize) -> Result<Vec<Pat>, String> {
    let mut seq = Vec::new();
    while let Some(&w) = words.get(*i) {
        match w {
            "]" | "|" => break,
            "[" => {
                *i += 1;
                let mut alts = Vec::new();
                loop {
                    alts.push(compile_seq(words, i)?);
                    match words.get(*i) {
                        Some(&"|") => *i += 1,
                        Some(&"]") => {
                            *i += 1;
                            break;
                        }
                        _ => return Err("a [ without its ]".into()),
                    }
                }
                seq.push(Pat::Opt(alts));
                continue;
            }
            _ => {}
        }
        seq.push(if let Some(c) = w.strip_prefix('$') {
            Pat::Cap(match c {
                "name" => Cap::Name,
                "expr" => Cap::Expr,
                "block" => Cap::Block,
                "body" => Cap::Body,
                "params" => Cap::Params,
                "args" => Cap::Args,
                "type" => Cap::Type,
                "end" => Cap::End,
                "simple" => Cap::Simple,
                "str" => Cap::Str,
                "place" => Cap::Place,
                "dims" => Cap::Dims,
                _ => return Err(format!("unknown capture ${c}")),
            })
        } else if let Some(n) = w.strip_prefix('@') {
            Pat::Sub(n.to_string())
        } else {
            Pat::Lit(w.to_string())
        });
        *i += 1;
    }
    Ok(seq)
}

fn literals(p: &[Pat], out: &mut Vec<String>) {
    for x in p {
        match x {
            Pat::Lit(s) => out.push(s.clone()),
            Pat::Opt(alts) => alts.iter().for_each(|a| literals(a, out)),
            _ => {}
        }
    }
}

impl Lang {
    pub fn parse(text: &str) -> Result<Lang, String> {
        let r: Raw = toml::from_str(text).map_err(|e| e.to_string())?;
        let err = |m: String| format!("{}: {m}", r.name);
        let mut forms = Vec::new();
        for f in &r.form {
            let kind = match f.kind.as_str() {
                "function" => Kind::Function,
                "ignore" => Kind::Ignore,
                "var" => Kind::Var,
                "assign" => Kind::Assign,
                "if" => Kind::If,
                "while" => Kind::While,
                "loop" => Kind::Loop,
                "for" => Kind::For,
                "for_range" => Kind::ForRange,
                "return" => Kind::Return,
                "break" => Kind::Break,
                "continue" => Kind::Continue,
                "nothing" => Kind::Nothing,
                "print" => Kind::Print,
                "expr" => Kind::Expr,
                "global" => Kind::Global,
                "read" => Kind::Read,
                "for_each" => Kind::ForEach,
                k => return Err(err(format!("unknown kind {k}"))),
            };
            let names = match &f.names {
                None => vec![Role::Value],
                Some(v) => v
                    .iter()
                    .map(|n| match n.as_str() {
                        "index" => Ok(Role::Index),
                        "value" => Ok(Role::Value),
                        x => Err(err(format!("unknown name role {x}"))),
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            };
            let place = match f.place.as_deref() {
                None | Some("stmt") => Place::Stmt,
                Some("top") => Place::Top,
                Some("sub") => Place::Sub,
                Some(p) => return Err(err(format!("unknown place {p}"))),
            };
            let style = match f.style.as_deref() {
                None => Style::None,
                Some("format") => Style::Format,
                Some("line") => Style::Line,
                Some("spaced_line") => Style::SpacedLine,
                Some(s) => return Err(err(format!("unknown print style {s}"))),
            };
            let op = match &f.op {
                None => None,
                Some(o) => Some(BinOp::parse(o).ok_or_else(|| err(format!("unknown op {o}")))?),
            };
            let words: Vec<&str> = f.pattern.split_whitespace().collect();
            let mut i = 0;
            let pat = compile_seq(&words, &mut i).map_err(|m| err(format!("{m} in {:?}", f.pattern)))?;
            if i != words.len() {
                return Err(err(format!("a stray {} in {:?}", words[i], f.pattern)));
            }
            forms.push(Form { name: f.name.clone(), kind, place, op, value: f.value, style, names, pat });
        }
        for f in &forms {
            let mut subs = Vec::new();
            fn walk(p: &[Pat], out: &mut Vec<String>) {
                for x in p {
                    match x {
                        Pat::Sub(n) => out.push(n.clone()),
                        Pat::Opt(a) => a.iter().for_each(|s| walk(s, out)),
                        _ => {}
                    }
                }
            }
            walk(&f.pat, &mut subs);
            for s in subs {
                if !forms.iter().any(|g| g.name.as_deref() == Some(s.as_str())) {
                    return Err(err(format!("@{s} names no form")));
                }
            }
        }

        let mut lits = Vec::new();
        forms.iter().for_each(|f| literals(&f.pat, &mut lits));
        let binary = r
            .binary
            .iter()
            .map(|(k, (p, o))| Ok((k.clone(), (*p, BinOp::parse(o).ok_or_else(|| err(format!("unknown op {o}")))?))))
            .collect::<Result<HashMap<_, _>, String>>()?;
        let unary = r
            .unary
            .iter()
            .map(|(k, (p, o))| Ok((k.clone(), (*p, UnOp::parse(o).ok_or_else(|| err(format!("unknown op {o}")))?))))
            .collect::<Result<HashMap<_, _>, String>>()?;
        let mut tokens: Vec<String> = lits.clone();
        tokens.extend(binary.keys().cloned());
        tokens.extend(unary.keys().cloned());
        tokens.extend(r.end_after.iter().cloned());
        let mut puncts: Vec<String> = tokens.iter().filter(|t| !is_word(t)).cloned().collect();
        puncts.extend(["(", ")", "{", "}", ",", ";", "[", "]"].map(String::from));
        if !r.methods.is_empty() {
            puncts.push(".".into());
        }
        if !r.qualified.is_empty() {
            puncts.push(".".into());
        }
        if r.composite_literals || r.list_literals {
            puncts.push(":".into()); // a[i:j]
        }
        puncts.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        puncts.dedup();
        let mut keywords: HashSet<String> = tokens.into_iter().filter(|t| is_word(t)).collect();
        keywords.extend(r.type_words.iter().cloned());
        keywords.extend(r.bool_constants.keys().cloned());

        let bool_print = match r.bool_print.as_slice() {
            [] => None,
            [t, f] => Some((t.clone(), f.clone())),
            _ => return Err(err("bool_print is the words for true and false".into())),
        };
        Ok(Lang {
            indent: match r.blocks.as_str() {
                "indent" => true,
                "braces" => false,
                b => return Err(err(format!("unknown blocks {b}"))),
            },
            end: match r.statement_end.as_str() {
                ";" => End::Semicolon,
                "newline" => End::Newline,
                "newline_after_value" => End::NewlineAfterValue,
                e => return Err(err(format!("unknown statement_end {e}"))),
            },
            block_scope: match r.scope.as_str() {
                "block" => true,
                "function" => false,
                s => return Err(err(format!("unknown scope {s}"))),
            },
            toplevel_entry: match r.entry.as_str() {
                "toplevel" => true,
                "main" => false,
                e => return Err(err(format!("unknown entry {e}"))),
            },
            param_first: match r.param_name.as_str() {
                "first" => true,
                "last" => false,
                p => return Err(err(format!("unknown param_name {p}"))),
            },
            chain_error: match r.chained_comparisons.as_str() {
                "error" => true,
                "allow" => false,
                c => return Err(err(format!("unknown chained_comparisons {c}"))),
            },
            int_widths: r.int_widths.clone().into_iter().collect(),
            float_widths: r.float_widths.clone().into_iter().collect(),
            float_suffixes: r.float_suffixes.clone(),
            float_print: match r.float_print.as_deref() {
                None => None,
                Some("repr") => Some(FloatPrint::Repr),
                Some("shortest") => Some(FloatPrint::Shortest),
                Some(x) => return Err(err(format!("unknown float_print {x}"))),
            },
            printf_go: match r.printf_floats.as_deref() {
                None => None,
                Some("ucrt") => Some(false),
                Some("go") => Some(true),
                Some(x) => return Err(err(format!("unknown printf_floats {x}"))),
            },
            casts: r.casts,
            qualified: r.qualified.clone().into_iter().collect(),
            read_line: r.read_line.clone().map(|[a, b]| (a, b)),
            promote: r.promote.unwrap_or(0),
            untyped_literals: match r.literals.as_deref() {
                None | Some("typed") => false,
                Some("untyped") => true,
                Some(x) => return Err(err(format!("unknown literals {x}"))),
            },
            checked: match r.overflow.as_deref() {
                None | Some("wrap") => false,
                Some("error") => true,
                Some(o) => return Err(err(format!("unknown overflow {o}"))),
            },
            string_quotes: one_byte(&r.string_quotes, "a string quote").map_err(err)?,
            char_quotes: one_byte(&r.char_quotes, "a character quote").map_err(err)?,
            name: r.name,
            extensions: r.extensions,
            line_comments: r.line_comments,
            block_comments: r.block_comments.into_iter().map(|[a, b]| (a, b)).collect(),
            skip_lines: r.skip_lines,
            end_after: r.end_after.into_iter().collect(),
            type_words: r.type_words.into_iter().collect(),
            bool_print,
            index_error: r.index_error,
            zero_division: r.zero_division,
            negative_index: r.negative_index,
            list_literals: r.list_literals,
            comprehensions: r.comprehensions,
            composite_literals: r.composite_literals,
            brace_lists: r.brace_lists,
            list_print: match r.list_print {
                Some([a, b, c]) => (a, b, c),
                None => ("[".into(), " ".into(), "]".into()),
            },
            builtins: r.builtins.into_iter().collect(),
            methods: r.methods.into_iter().collect(),
            bool_constants: r.bool_constants.into_iter().collect(),
            binary,
            unary,
            forms,
            puncts,
            keywords,
        })
    }

    pub fn form(&self, name: &str) -> Option<&Form> {
        self.forms.iter().find(|f| f.name.as_deref() == Some(name))
    }
}
