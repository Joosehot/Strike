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
/// files a description names for what is too long to write in it
const DATA: [(&str, &str); 1] = [("c-predefined.h", include_str!("../../languages/c-predefined.h"))];

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
    /// x.name(...) on a list or a string: the name -> what it is
    #[serde(default)]
    methods: BTreeMap<String, String>,
    /// package.name(...): the two names -> what it is (Go's strconv.Itoa)
    #[serde(default)]
    qualified: BTreeMap<String, String>,
    /// what a string is: "c" (chars ended by a zero byte), "bytes" (Go),
    /// "ascii" (Python's code points, which Strike holds to ASCII)
    #[serde(default)]
    strings: Option<String>,
    #[serde(default)]
    bool_constants: BTreeMap<String, i64>,
    #[serde(default)]
    binary: BTreeMap<String, (u8, String)>,
    #[serde(default)]
    unary: BTreeMap<String, (u8, String)>,
    #[serde(default)]
    form: Vec<RawForm>,
    #[serde(default)]
    preprocessor: Option<RawPre>,
    #[serde(default)]
    program: Option<RawProgram>,
    #[serde(default)]
    records: Option<RawRecords>,
}

/// Structs and classes: how a value's fields are reached, and the words
/// that make or take addresses.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRecords {
    /// "structs" (C, Go: values held in place, copied) or "classes"
    /// (Python: objects on the heap, reached by reference)
    model: String,
    /// the words after which a name is a type's own (C's struct, Go's
    /// type, Python's class), found before the file is read
    #[serde(default)]
    type_intro: Vec<String>,
    /// the word whose declaration names a type by its last name (C's typedef)
    #[serde(default)]
    typedef: Option<String>,
    /// p->x: a field through a pointer
    #[serde(default)]
    arrow: Option<String>,
    /// &x and *p
    #[serde(default)]
    address_of: Option<String>,
    #[serde(default)]
    deref: Option<String>,
    /// p.x through a pointer (Go)
    #[serde(default)]
    auto_deref: bool,
    /// {.x = 1} (C) or P{X: 1} (Go): an element given to a field by name
    #[serde(default)]
    keyed: Option<String>,
    /// sizeof(type), sizeof x
    #[serde(default)]
    sizeof: Option<String>,
    /// offsetof(type, field), the name the header's macro gives it
    #[serde(default)]
    offsetof: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPre {
    marker: String,
    /// a file in languages/ of #define lines: the compiler's own macros
    predefined: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProgram {
    model: String,
    directory: String,
    #[serde(default)]
    skip_suffixes: Vec<String>,
    #[serde(default)]
    module_file: Option<String>,
    #[serde(default)]
    standard: Vec<String>,
    #[serde(default)]
    found_first: Vec<String>,
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
    /// import m, import "mod/pkg": another module or package of the program
    Import,
    /// Python's from m import a, b
    ImportFrom,
    /// Go's package clause
    Package,
    /// a struct or a class: its name (and a second name, C's typedef or
    /// Python's base class), its fields or members
    Struct,
    /// typedef struct P Q: a second name for a struct
    Alias,
    /// Go's func (p *P) Move(...): a function of a type's values
    Method,
}

/// What a $name of a for_each form is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Index,
    Value,
    /// a struct's: C's tag (written after struct), its name alone, Python's
    /// base class
    Tag,
    Alias,
    Base,
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

/// What the language's strings are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Strings {
    /// none: a string can only be printed
    None,
    /// C: chars in a row, ended by a zero byte; a literal is their address
    C,
    /// Go: bytes, s[i] a byte (uint8), ranged over as UTF-8
    Bytes,
    /// Python: code points, s[i] a string of one; Strike holds them to
    /// ASCII, so each is one byte
    Ascii,
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
    /// a struct's fields, to the closing brace
    Fields,
    /// a class's block: its methods and its attributes
    Members,
}

#[derive(Clone, Debug)]
pub enum Pat {
    Lit(String),
    Cap(Cap),
    Sub(String),
    /// one of the alternatives, or nothing
    Opt(Vec<Vec<Pat>>),
    /// [ a | b ]*: the alternatives, as many times as they match
    Many(Vec<Vec<Pat>>),
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
    pub qualified: HashMap<String, String>,
    pub strings: Strings,
    pub bool_constants: HashMap<String, i64>,
    pub binary: HashMap<String, (u8, BinOp)>,
    pub unary: HashMap<String, (u8, UnOp)>,
    pub forms: Vec<Form>,
    /// every punctuation token, longest first
    pub puncts: Vec<String>,
    /// words that are never a name
    pub keywords: HashSet<String>,
    pub preprocessor: Option<Preprocessor>,
    pub program: ProgramRule,
    pub records: Option<Records>,
}

/// How a language's structs or classes are written ([records]).
pub struct Records {
    /// objects on the heap, reached by reference (Python's classes); else
    /// values held in place and copied (C's and Go's structs)
    pub classes: bool,
    pub type_intro: Vec<String>,
    pub typedef: Option<String>,
    pub arrow: Option<String>,
    pub address_of: Option<String>,
    pub deref: Option<String>,
    pub auto_deref: bool,
    /// "." before the name (C's designators) or ":" after it (Go's keys)
    pub keyed: Option<String>,
    pub sizeof: Option<String>,
    pub offsetof: Option<String>,
}

/// C's preprocessor, as the description gives it.
pub struct Preprocessor {
    /// the token a directive line begins with
    pub marker: String,
    /// #define lines: the macros the compiler defines before any file
    pub predefined: String,
    /// the headers Strike knows: name -> the text including it gives
    pub headers: HashMap<String, String>,
}

/// How the files of a program make one program.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Model {
    /// every file on its own, sharing the names outside functions that are
    /// not static (C's translation units)
    Units,
    /// a directory is a package; others are imported by their path (Go)
    Packages,
    /// one file runs, and imports the others by name (Python)
    Modules,
}

pub struct ProgramRule {
    pub model: Model,
    /// the files a directory is: "*.ext" for every such file, or one name
    pub directory: String,
    /// files of the directory that are not part of it (Go's _test.go)
    pub skip_suffixes: Vec<String>,
    /// the file that names the module and its root (go.mod)
    pub module_file: Option<String>,
    /// the language's own packages that Strike provides (fmt)
    pub standard: Vec<String>,
    /// module names the language finds before any file of the program
    pub found_first: Vec<String>,
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
            "]" | "]*" | "|" => break,
            "[" => {
                *i += 1;
                let mut alts = Vec::new();
                let many = loop {
                    alts.push(compile_seq(words, i)?);
                    match words.get(*i) {
                        Some(&"|") => *i += 1,
                        Some(&"]") => {
                            *i += 1;
                            break false;
                        }
                        Some(&"]*") => {
                            *i += 1;
                            break true;
                        }
                        _ => return Err("a [ without its ]".into()),
                    }
                };
                seq.push(if many { Pat::Many(alts) } else { Pat::Opt(alts) });
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
                "fields" => Cap::Fields,
                "members" => Cap::Members,
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
            Pat::Opt(alts) | Pat::Many(alts) => alts.iter().for_each(|a| literals(a, out)),
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
                "import" => Kind::Import,
                "import_from" => Kind::ImportFrom,
                "package" => Kind::Package,
                "struct" => Kind::Struct,
                "alias" => Kind::Alias,
                "method" => Kind::Method,
                k => return Err(err(format!("unknown kind {k}"))),
            };
            let names = match &f.names {
                None => vec![Role::Value],
                Some(v) => v
                    .iter()
                    .map(|n| match n.as_str() {
                        "index" => Ok(Role::Index),
                        "value" => Ok(Role::Value),
                        "tag" => Ok(Role::Tag),
                        "alias" => Ok(Role::Alias),
                        "base" => Ok(Role::Base),
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
                        Pat::Opt(a) | Pat::Many(a) => a.iter().for_each(|s| walk(s, out)),
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
        // a.b: a name of a module, the language's own (qualified) or one of
        // the program's (a program of packages or modules)
        let dotted = r.program.as_ref().is_some_and(|p| p.model != "units");
        if !r.methods.is_empty() || !r.qualified.is_empty() || dotted {
            puncts.push(".".into());
        }
        if r.composite_literals || r.list_literals {
            puncts.push(":".into()); // a[i:j]
        }
        // p.x, p->x, &x, *p: a struct's fields and addresses
        if let Some(rec) = &r.records {
            puncts.push(".".into());
            puncts.extend(rec.arrow.iter().chain(rec.address_of.iter()).chain(rec.deref.iter()).cloned());
        }
        puncts.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        puncts.dedup();
        let mut keywords: HashSet<String> = tokens.into_iter().filter(|t| is_word(t)).collect();
        keywords.extend(r.type_words.iter().cloned());
        keywords.extend(r.bool_constants.keys().cloned());
        if let Some(rec) = &r.records {
            keywords.extend(rec.sizeof.iter().chain(rec.offsetof.iter()).cloned());
        }

        let strings = match r.strings.as_deref() {
            None => Strings::None,
            Some("c") => Strings::C,
            Some("bytes") => Strings::Bytes,
            Some("ascii") => Strings::Ascii,
            Some(x) => return Err(err(format!("unknown strings {x}"))),
        };
        let bool_print = match r.bool_print.as_slice() {
            [] => None,
            [t, f] => Some((t.clone(), f.clone())),
            _ => return Err(err("bool_print is the words for true and false".into())),
        };
        let preprocessor = match r.preprocessor {
            None => None,
            Some(p) => Some(Preprocessor {
                marker: p.marker,
                predefined: DATA
                    .iter()
                    .find(|(n, _)| *n == p.predefined)
                    .ok_or_else(|| err(format!("there is no file {} in languages/", p.predefined)))?
                    .1
                    .to_string(),
                headers: p.headers.into_iter().collect(),
            }),
        };
        let program = match r.program {
            None => ProgramRule {
                model: Model::Units,
                directory: format!("*.{}", r.extensions.first().cloned().unwrap_or_default()),
                skip_suffixes: Vec::new(),
                module_file: None,
                standard: Vec::new(),
                found_first: Vec::new(),
            },
            Some(p) => ProgramRule {
                model: match p.model.as_str() {
                    "units" => Model::Units,
                    "packages" => Model::Packages,
                    "modules" => Model::Modules,
                    m => return Err(err(format!("unknown program model {m}"))),
                },
                directory: p.directory,
                skip_suffixes: p.skip_suffixes,
                module_file: p.module_file,
                standard: p.standard,
                found_first: p.found_first,
            },
        };
        let records = match r.records {
            None => None,
            Some(x) => Some(Records {
                classes: match x.model.as_str() {
                    "structs" => false,
                    "classes" => true,
                    m => return Err(err(format!("unknown records model {m}"))),
                },
                type_intro: x.type_intro,
                typedef: x.typedef,
                arrow: x.arrow,
                address_of: x.address_of,
                deref: x.deref,
                auto_deref: x.auto_deref,
                keyed: x.keyed,
                sizeof: x.sizeof,
                offsetof: x.offsetof,
            }),
        };
        Ok(Lang {
            records,
            preprocessor,
            program,
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
            qualified: r.qualified.into_iter().collect(),
            strings,
            bool_constants: r.bool_constants.into_iter().collect(),
            binary,
            unary,
            forms,
            puncts,
            keywords,
        })
    }

    /// a.b is a name b of the module a: one of the language's own
    /// ([qualified]) or one of the program's (Go's packages, Python's
    /// modules)
    pub fn dotted(&self) -> bool {
        !self.qualified.is_empty() || self.program.model != Model::Units
    }

    pub fn form(&self, name: &str) -> Option<&Form> {
        self.forms.iter().find(|f| f.name.as_deref() == Some(name))
    }
}
