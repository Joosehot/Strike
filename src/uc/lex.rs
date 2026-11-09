//! Text to tokens, as a language's description says: its comments, its
//! strings, its punctuation, and where a line ends a statement or opens a
//! block by its indentation.

use super::lang::{End, Lang};

/// A float literal: the nearest double, the nearest float32 (as a double:
/// a literal made a float32 is rounded once, from its own digits), whether
/// it was written as a float32 (C's 1.5f), and whether the double is the
/// literal exactly (Go's constants are exact numbers, so this decides what
/// may be worked out with them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatLit {
    pub bits: u64,
    pub bits32: u64,
    pub f32: bool,
    pub exact: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum T {
    Ident(String),
    Num(i64),
    Float(FloatLit),
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
        T::Num(_) | T::Float(_) | T::Str(_) => true,
        T::Punct(p) => l.end_after.contains(p),
        _ => false,
    }
}

/// Is the decimal literal (digits, a point, an exponent) the double x
/// exactly? Both are made digits without leading or trailing zeros and an
/// exponent, and compared.
fn exactly(text: &str, x: f64) -> bool {
    if !x.is_finite() {
        return false;
    }
    let (mant, exp) = match text.find(['e', 'E']) {
        Some(k) => (&text[..k], text[k + 1..].parse::<i64>().unwrap_or(i64::MAX / 2)),
        None => (text, 0),
    };
    let (int, frac) = mant.split_once('.').unwrap_or((mant, ""));
    let norm = |digits: String, mut e: i64| -> (String, i64) {
        let d = digits.trim_start_matches('0');
        let t = d.trim_end_matches('0');
        e += (d.len() - t.len()) as i64;
        (t.to_string(), e)
    };
    let (a, ea) = norm(format!("{int}{frac}"), exp - frac.len() as i64);
    if a.is_empty() {
        return x == 0.0;
    }
    // Rust writes a double's digits exactly when it is asked for enough
    let s = format!("{:.1100e}", x);
    let (m, e) = s.split_once('e').unwrap();
    let (b, eb) = norm(m.replace('.', ""), e.parse::<i64>().unwrap() - 1100);
    a == b && ea == eb
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
        // a float: digits with a point or an exponent (1.5, .5, 2., 1e9)
        let float_start = c.is_ascii_digit() || (c == b'.' && s.get(i + 1).is_some_and(|d| d.is_ascii_digit()));
        if float_start && !rest.starts_with("0x") && !rest.starts_with("0X") {
            let mut j = i;
            let digits = |j: &mut usize| {
                while *j < s.len() && (s[*j].is_ascii_digit() || s[*j] == b'_') {
                    *j += 1;
                }
            };
            digits(&mut j);
            let mut is_float = false;
            if j < s.len() && s[j] == b'.' && !(j + 1 < s.len() && s[j + 1] == b'.') {
                is_float = true;
                j += 1;
                digits(&mut j);
            }
            if j < s.len() && (s[j] == b'e' || s[j] == b'E') {
                let mut k = j + 1;
                if k < s.len() && (s[k] == b'+' || s[k] == b'-') {
                    k += 1;
                }
                if k < s.len() && s[k].is_ascii_digit() {
                    is_float = true;
                    j = k;
                    digits(&mut j);
                }
            }
            if is_float {
                let text: String = src[i..j].chars().filter(|c| *c != '_').collect();
                let mut f32 = false;
                let mut end = j;
                if let Some(sfx) = l.float_suffixes.iter().find(|x| src[j..].starts_with(x.as_str())) {
                    f32 = true;
                    end += sfx.len();
                }
                if end < s.len() && (s[end].is_ascii_alphanumeric() || s[end] == b'_') {
                    return Err((line, format!("{} is not a float literal Strike knows", &src[i..end + 1])));
                }
                let x: f64 = text.parse().map_err(|_| (line, format!("{text} is not a float literal Strike knows")))?;
                let y: f32 = text.parse().map_err(|_| (line, format!("{text} is not a float literal Strike knows")))?;
                let lit = FloatLit { bits: x.to_bits(), bits32: (y as f64).to_bits(), f32, exact: exactly(&text, x) };
                out.push(Tok { t: T::Float(lit), line });
                i = end;
                continue;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_literal_is_exact_only_where_its_double_is_it() {
        for (text, exact) in [("0.5", true), ("2.25e1", true), ("1e22", true), ("1e23", false), ("0.1", false), ("123.000", true), ("0.0", true), ("1e400", false), (".375", true)] {
            assert_eq!(exactly(text, text.parse().unwrap()), exact, "{text}");
        }
    }
}
