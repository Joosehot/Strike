//! Strike's universal compiler. It knows no language: every language is a
//! description in languages/, and HotMotor's judges pick the one that reads
//! the file. The file becomes Strike IR, every function of it is branded by
//! its IR, a function already branded is struck from the brand instead of
//! compiled, and the executable is written by Strike itself.
//!
//!   text --lex, parse (the description)--> universal syntax
//!        --lower--> Strike IR --x64--> machine code --pe--> .exe

pub mod ir;
mod lang;
mod lex;
mod lower;
mod parse;
mod pe;
mod x64;

use lang::Lang;
use std::path::{Path, PathBuf};
use std::time::Instant;
use x64::{Code, Target};

pub struct Report {
    pub lang: String,
    /// how the judges scored each description
    pub explain: Vec<String>,
    pub funcs: usize,
    pub struck: usize,
    pub read_seconds: f64,
    pub code_seconds: f64,
    pub exe_seconds: f64,
    pub seconds: f64,
    pub exe_bytes: usize,
}

#[derive(serde::Deserialize)]
struct Judges {
    whole: f64,
    extension: f64,
    reach: f64,
}

fn brand_dir() -> PathBuf {
    crate::lightning::home().join("uc")
}

fn save(c: &Code) -> Vec<u8> {
    let mut b = Vec::with_capacity(c.bytes.len() + 16);
    b.extend_from_slice(&(c.bytes.len() as u32).to_le_bytes());
    b.extend_from_slice(&c.bytes);
    b.extend_from_slice(&(c.relocs.len() as u32).to_le_bytes());
    for (at, t) in &c.relocs {
        b.extend_from_slice(&at.to_le_bytes());
        let (kind, data) = match t {
            Target::Func(s) => (0u8, s.as_bytes()),
            Target::Import(s) => (1, s.as_bytes()),
            Target::Data(d) => (2, d.as_slice()),
        };
        b.push(kind);
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(data);
    }
    b
}

fn load(b: &[u8]) -> Option<Code> {
    let mut i = 0;
    let u32_at = |i: &mut usize| -> Option<u32> {
        let v = u32::from_le_bytes(b.get(*i..*i + 4)?.try_into().ok()?);
        *i += 4;
        Some(v)
    };
    let n = u32_at(&mut i)? as usize;
    let bytes = b.get(i..i + n)?.to_vec();
    i += n;
    let r = u32_at(&mut i)?;
    let mut relocs = Vec::new();
    for _ in 0..r {
        let at = u32_at(&mut i)?;
        let kind = *b.get(i)?;
        i += 1;
        let len = u32_at(&mut i)? as usize;
        let data = b.get(i..i + len)?.to_vec();
        i += len;
        relocs.push((
            at,
            match kind {
                0 => Target::Func(String::from_utf8(data).ok()?),
                1 => Target::Import(String::from_utf8(data).ok()?),
                2 => Target::Data(data),
                _ => return None,
            },
        ));
    }
    (i == b.len()).then_some(Code { bytes, relocs })
}

/// One description's reading of the file: the IR, or how far it got.
fn front(text: &str, l: &Lang) -> Result<lower::Lowered, (u32, String, f64)> {
    let toks = lex::lex(text, l).map_err(|(line, m)| (line, m, 0.0))?;
    let prog = parse::parse(&toks, l).map_err(|f| (f.line, f.message, f.reach))?;
    lower::lower(&prog, l).map_err(|(line, m)| (line, m, 1.0))
}

pub fn languages() -> Result<Vec<Lang>, String> {
    let mut v = lang::BUILT_IN.iter().map(|t| Lang::parse(t)).collect::<Result<Vec<_>, _>>()?;
    v.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(v)
}

pub fn compile(src: &Path, out: &Path, brands: bool) -> Result<Report, String> {
    let start = Instant::now();
    let text = std::fs::read_to_string(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let langs = languages()?;
    let j: Judges = toml::from_str(lang::JUDGES).map_err(|e| e.to_string())?;
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();

    // HotMotor: every description is a candidate; the judges score it. The
    // most a description can score bounds the order: one that cannot beat
    // the best so far is not read at all.
    let ext_score = |l: &Lang| if l.extensions.iter().any(|e| e.eq_ignore_ascii_case(&ext)) { j.extension } else { 0.0 };
    let mut order: Vec<usize> = (0..langs.len()).collect();
    let bound = |l: &Lang| j.whole + ext_score(l) + j.reach;
    order.sort_by(|&a, &b| bound(&langs[b]).total_cmp(&bound(&langs[a])).then(a.cmp(&b)));
    let mut explain = Vec::new();
    let mut best: Option<(f64, usize, lower::Lowered)> = None;
    let mut nearest: Option<(f64, usize, (u32, String))> = None;
    for &i in &order {
        let l = &langs[i];
        if let Some((s, _, _)) = &best {
            if *s >= bound(l) {
                explain.push(format!("{:<8} not read: it can score at most {:.2}", l.name, bound(l)));
                continue;
            }
        }
        match front(&text, l) {
            Ok(low) => {
                let s = j.whole + ext_score(l) + j.reach;
                explain.push(format!("{:<8} {s:.2}  reads the whole file", l.name));
                if best.as_ref().is_none_or(|(b, _, _)| s > *b) {
                    best = Some((s, i, low));
                }
            }
            Err((line, m, reach)) => {
                let s = ext_score(l) + j.reach * reach;
                explain.push(format!("{:<8} {s:.2}  stops at line {line}: {m}", l.name));
                if nearest.as_ref().is_none_or(|(b, _, _)| s > *b) {
                    nearest = Some((s, i, (line, m)));
                }
            }
        }
    }
    let Some((_, chosen, low)) = best else {
        let (_, i, (line, m)) = nearest.ok_or("there are no languages")?;
        return Err(format!("{}:{line}: {m} (read as {}; no description reads the whole file)", src.display(), langs[i].name));
    };
    let read_seconds = start.elapsed().as_secs_f64();

    // every function: struck from its brand, or compiled and branded
    let t = Instant::now();
    let dir = brand_dir();
    let mut funcs: Vec<(String, Code)> = x64::runtime(&low.entry);
    let mut struck = 0;
    for f in &low.funcs {
        let key = f.brand();
        let path = dir.join(&key[..2]).join(&key);
        let kept = if brands { std::fs::read(&path).ok().and_then(|b| load(&b)) } else { None };
        let code = match kept {
            Some(c) => {
                struck += 1;
                c
            }
            None => {
                let c = x64::compile(f);
                if brands {
                    let _ = std::fs::create_dir_all(path.parent().unwrap());
                    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
                    if std::fs::write(&tmp, save(&c)).is_ok() {
                        let _ = std::fs::rename(&tmp, &path);
                    }
                }
                c
            }
        };
        funcs.push((f.name.clone(), code));
    }
    let code_seconds = t.elapsed().as_secs_f64();

    let t = Instant::now();
    let exe = pe::link(&funcs, "rt.start")?;
    std::fs::write(out, &exe).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(Report {
        lang: langs[chosen].name.clone(),
        explain,
        funcs: low.funcs.len(),
        struck,
        read_seconds,
        code_seconds,
        exe_seconds: t.elapsed().as_secs_f64(),
        seconds: start.elapsed().as_secs_f64(),
        exe_bytes: exe.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_description_reads() {
        assert_eq!(languages().unwrap().len(), 3);
    }

    #[test]
    fn a_brand_comes_back_as_it_went() {
        let c = Code {
            bytes: vec![1, 2, 3],
            relocs: vec![(0, Target::Func("u.f".into())), (1, Target::Data(b"hi\n".to_vec())), (2, Target::Import("WriteFile".into()))],
        };
        assert_eq!(load(&save(&c)), Some(c));
    }

    #[test]
    fn the_same_function_in_three_languages_is_the_same_ir() {
        let progs = [
            ("c", "int sq(int x) { return x * x + 1; }\nint main() { return sq(3); }\n"),
            ("go", "package main\nfunc sq(x int) int {\n\treturn x*x + 1\n}\nfunc main() {\n\tsq(3)\n}\n"),
            ("python", "def sq(x):\n    return x * x + 1\nsq(3)\n"),
        ];
        let langs = languages().unwrap();
        let mut brands = Vec::new();
        for (name, text) in progs {
            let l = langs.iter().find(|l| l.name.to_lowercase() == name).unwrap();
            let low = front(text, l).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let f = low.funcs.iter().find(|f| f.name == "u.sq").unwrap();
            brands.push(f.brand());
        }
        assert_eq!(brands[0], brands[1]);
        assert_eq!(brands[1], brands[2]);
    }
}
