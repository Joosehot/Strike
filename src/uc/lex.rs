//! Text to tokens, as a language's description says: its comments, its
//! strings, its punctuation, and where a line ends a statement or opens a
//! block by its indentation.

use super::lang::{End, Lang};

#[derive(Clone, Debug, PartialEq)]
pub enum T {
    Ident(String),
    Num(i64),
    Str(Vec<u8>),
    Punct(String),
    /// the end of a statement made by a newline
    End,
    Indent,
    Dedent,
    Eof,
}

#[derive(Clone, Debug)]
pub struct Tok {
    pub t: T,
    pub line: u32,
}

fn ends_value(t: &T, l: &Lang) -> bool {
    match t {
        // a type's name ends a line as any name does (Go's `var x int`)
        T::Ident(s) => !l.keywords.contains(s) || l.end_after.contains(s) || l.type_words.contains(s),
        T::Num(_) | T::Str(_) => true,
        T::Punct(p) => l.end_after.contains(p),
        _ => false,
    }
}

fn escape(c: u8) -> Option<u8> {
    Some(match c {
        b'n' => b'\n',
        b't' => b'\t',
        b'r' => b'\r',
        b'0' => 0,
        b'\\' => b'\\',
        b'"' => b'"',
        b'\'' => b'\'',
        _ => return None,
    })
}

pub fn lex(src: &str, l: &Lang) -> Result<Vec<Tok>, (u32, String)> {
    let s = src.as_bytes();
    let mut out: Vec<Tok> = Vec::new();
    let mut i = 0;
    let mut line = 1u32;
    let mut depth = 0i32;
    let mut at_line_start = true;
    let mut line_has_tokens = false;
    let mut indents = vec![0usize];

    while i < s.len() {
        if at_line_start && depth == 0 {
            // a new line: its indentation, or a blank line
            let mut j = i;
            let mut col = 0;
            while j < s.len() && (s[j] == b' ' || s[j] == b'\t') {
                col = if s[j] == b'\t' { (col / 8 + 1) * 8 } else { col + 1 };
                j += 1;
            }
            let rest = &src[j..];
            let blank = j >= s.len()
                || s[j] == b'\n'
                || s[j] == b'\r'
                || (l.indent && l.line_comments.iter().any(|c| rest.starts_with(c.as_str())));
            if blank {
                while j < s.len() && s[j] != b'\n' {
                    j += 1;
                }
                if j < s.len() {
                    line += 1;
                    j += 1;
                }
                i = j;
                continue;
            }
            if l.indent {
                let top = *indents.last().unwrap();
                if col > top {
                    indents.push(col);
                    out.push(Tok { t: T::Indent, line });
                } else {
                    while col < *indents.last().unwrap() {
                        indents.pop();
                        out.push(Tok { t: T::Dedent, line });
                    }
                    if col != *indents.last().unwrap() {
                        return Err((line, "this line's indentation matches no line before it".into()));
                    }
                }
            }
            at_line_start = false;
            i = j;
            continue;
        }
        let c = s[i];
        if c == b'\n' {
            if depth == 0 {
                let ends = match l.end {
                    End::Newline => line_has_tokens,
                    End::NewlineAfterValue => out.last().is_some_and(|t| ends_value(&t.t, l)) && line_has_tokens,
                    End::Semicolon => false,
                };
                if ends {
                    out.push(Tok { t: T::End, line });
                }
                at_line_start = true;
                line_has_tokens = false;
            }
            line += 1;
            i += 1;
            continue;
        }
        if c == b' ' || c == b'\t' || c == b'\r' {
            i += 1;
            continue;
        }
        let rest = &src[i..];
        if l.line_comments.iter().any(|p| rest.starts_with(p.as_str())) {
            while i < s.len() && s[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if let Some((_, close)) = l.block_comments.iter().find(|(open, _)| rest.starts_with(open.as_str())) {
            let start = line;
            match rest.find(close.as_str()) {
                Some(k) => {
                    line += rest[..k].bytes().filter(|&b| b == b'\n').count() as u32;
                    i += k + close.len();
                }
                None => return Err((start, "a comment that never closes".into())),
            }
            continue;
        }
        line_has_tokens = true;
        if l.string_quotes.contains(&c) || l.char_quotes.contains(&c) {
            let start = line;
            let mut j = i + 1;
            let mut bytes = Vec::new();
            loop {
                match s.get(j) {
                    None | Some(b'\n') => return Err((start, "a string that never closes".into())),
                    Some(&q) if q == c => break,
                    Some(b'\\') => {
                        let e = s.get(j + 1).copied().and_then(escape).ok_or((line, "an unknown escape".to_string()))?;
                        bytes.push(e);
                        j += 2;
                    }
                    Some(&b) => {
                        bytes.push(b);
                        j += 1;
                    }
                }
            }
            i = j + 1;
            let t = if l.string_quotes.contains(&c) {
                T::Str(bytes)
            } else if bytes.len() == 1 {
                T::Num(bytes[0] as i64)
            } else {
                return Err((start, "a character literal holds one character".into()));
            };
            out.push(Tok { t, line: start });
            continue;
        }
        if c.is_ascii_digit() {
            let mut j = i;
            let (radix, from) = if rest.starts_with("0x") || rest.starts_with("0X") { (16, i + 2) } else { (10, i) };
            j = j.max(from);
            let mut digits = String::new();
            while j < s.len() && (s[j].is_ascii_alphanumeric() || s[j] == b'_') {
                if s[j] != b'_' {
                    digits.push(s[j] as char);
                }
                j += 1;
            }
            let v = i64::from_str_radix(&digits, radix)
                .map_err(|_| (line, format!("{} is not a 64-bit integer", &src[i..j])))?;
            out.push(Tok { t: T::Num(v), line });
            i = j;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let mut j = i;
            while j < s.len() && (s[j].is_ascii_alphanumeric() || s[j] == b'_') {
                j += 1;
            }
            out.push(Tok { t: T::Ident(src[i..j].to_string()), line });
            i = j;
            continue;
        }
        match l.puncts.iter().find(|p| rest.starts_with(p.as_str())) {
            Some(p) => {
                match p.as_str() {
                    "(" | "[" => depth += 1,
                    ")" | "]" => depth -= 1,
                    "{" if l.indent => depth += 1,
                    "}" if l.indent => depth -= 1,
                    _ => {}
                }
                out.push(Tok { t: T::Punct(p.clone()), line });
                i += p.len();
            }
            None => {
                let ch = rest.chars().next().unwrap();
                return Err((line, format!("{} has no {ch:?}", l.name)));
            }
        }
    }
    let ends = match l.end {
        End::Newline => line_has_tokens,
        End::NewlineAfterValue => line_has_tokens && out.last().is_some_and(|t| ends_value(&t.t, l)),
        End::Semicolon => false,
    };
    if ends {
        out.push(Tok { t: T::End, line });
    }
    while indents.len() > 1 {
        indents.pop();
        out.push(Tok { t: T::Dedent, line });
    }
    out.push(Tok { t: T::Eof, line });
    Ok(out)
}
