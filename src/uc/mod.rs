//! Strike's universal compiler. It knows no language: every language is a
//! description in languages/, and HotMotor's judges pick the one that reads
//! the file. The file becomes Strike IR, every function of it is branded by
//! its IR, a function already branded is struck from the brand instead of
//! compiled, and the executable is written by Strike itself.
//!
//!   text --lex, parse (the description)--> universal syntax
//!        --lower--> Strike IR --x64opt (or x64, plain)--> machine code
//!        --pe--> .exe
//!
//! The brands are one pack (~/.strike/uc/pack), read whole once per
//! compile, with new brands added at its end.

mod eval;
mod fold;
pub mod hot;
mod inline;
mod tail;
pub mod ir;
mod lang;
mod lex;
mod lib;
mod lower;
mod opt;
mod parse;
mod pe;
mod rotate;
mod scev;
mod x64;
mod x64opt;

use lang::Lang;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;
use x64::{Code, Target};

/// Which code generator: the optimizing one, or the plain one every
/// optimized program is checked against.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Gen {
    Opt,
    /// --release: the optimizing generator with the work a release build
    /// may spend more time on (codegen.toml, [release])
    Release,
    Plain,
}

const PACK_MAGIC: &[u8; 2] = b"SB";

/// The brand pack: every record is SB, a 32-byte key, a length and the
/// code. A record cut short at the end (a write that did not finish) is
/// left out.
struct Pack {
    path: PathBuf,
    bytes: Vec<u8>,
    at: HashMap<[u8; 32], (usize, usize)>,
    new: Vec<u8>,
}

impl Pack {
    fn open(path: PathBuf) -> Pack {
        let bytes = std::fs::read(&path).unwrap_or_default();
        let mut at = HashMap::new();
        let mut i = 0;
        while i + 38 <= bytes.len() && &bytes[i..i + 2] == PACK_MAGIC {
            let key: [u8; 32] = bytes[i + 2..i + 34].try_into().unwrap();
            let len = u32::from_le_bytes(bytes[i + 34..i + 38].try_into().unwrap()) as usize;
            if i + 38 + len > bytes.len() {
                break;
            }
            at.insert(key, (i + 38, len));
            i += 38 + len;
        }
        Pack { path, bytes, at, new: Vec::new() }
    }
    fn get(&self, key: &[u8; 32]) -> Option<Code> {
        let &(s, n) = self.at.get(key)?;
        load(&self.bytes[s..s + n])
    }
    fn add(&mut self, key: [u8; 32], c: &Code) {
        let data = save(c);
        self.new.extend_from_slice(PACK_MAGIC);
        self.new.extend_from_slice(&key);
        self.new.extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.new.extend_from_slice(&data);
    }
    fn write(self) {
        if self.new.is_empty() {
            return;
        }
        let _ = std::fs::create_dir_all(self.path.parent().unwrap());
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = f.write_all(&self.new);
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
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
    /// calls HotMotor chose to copy the callee into
    pub inlined: usize,
    /// calls of a function by itself at a return, made jumps
    pub tail_calls: usize,
    /// calls of a function by itself replaced by a copy of its body
    pub unrolled: usize,
    /// --release: how the program was worked out while compiling
    pub evaluated: Option<String>,
    /// --release: operations the algebra changed, expressions moved out of loops
    pub simplified: usize,
    pub hoisted: usize,
    /// --release: loops replaced by the closed form of their sums
    pub closed: usize,
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
        let offset: [u8; 4];
        let (kind, data) = match t {
            Target::Func(s) => (0u8, s.as_bytes()),
            Target::Import(s) => (1, s.as_bytes()),
            Target::Data(d) => (2, d.as_slice()),
            Target::Writable(o) => {
                offset = o.to_le_bytes();
                (3, &offset[..])
            }
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
                3 => Target::Writable(u32::from_le_bytes(data.try_into().ok()?)),
                _ => return None,
            },
        ));
    }
    (i == b.len()).then_some(Code { bytes, relocs })
}

/// One description's reading of the file: the IR, or how far it got. In
/// the daemon, from its memory (hot.rs).
fn front(text: &str, l: &Lang) -> Result<lower::Lowered, (u32, String, f64)> {
    hot::front(text, l, read_front)
}

/// The reading itself: a function of the text and the description only.
fn read_front(text: &str, l: &Lang) -> Result<lower::Lowered, (u32, String, f64)> {
    let toks = lex::lex(text, l).map_err(|(line, m)| (line, m, 0.0))?;
    let prog = parse::parse(&toks, l).map_err(|f| (f.line, f.message, f.reach))?;
    lower::lower(&prog, l).map_err(|(line, m)| (line, m, 1.0))
}

pub fn languages() -> Result<Vec<Lang>, String> {
    let mut v = lang::BUILT_IN.iter().map(|t| Lang::parse(t)).collect::<Result<Vec<_>, _>>()?;
    v.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(v)
}

pub fn compile(src: &Path, out: &Path, brands: bool, gen: Gen, evaluate: bool) -> Result<Report, String> {
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
    let mut nearest: Option<(f64, usize, (u32, String, bool))> = None;
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
                    nearest = Some((s, i, (line, m, reach >= 1.0)));
                }
            }
        }
    }
    let Some((_, chosen, low)) = best else {
        let (_, i, (line, m, whole)) = nearest.ok_or("there are no languages")?;
        // read whole but not lowered: the language is known, the program is not
        // what Strike can compile yet
        let why = if whole { String::new() } else { "; no description reads the whole file".to_string() };
        return Err(format!("{}:{line}: {m} (read as {}{why})", src.display(), langs[i].name));
    };
    let read_seconds = start.elapsed().as_secs_f64();

    // every function: struck from its brand, or compiled and branded. The
    // brand is the function's IR and everything that decides its code (the
    // generator and its rules).
    let t = Instant::now();
    let rules = x64opt::Rules::read()?;
    let generator = match gen {
        Gen::Opt => rules.key(false),
        Gen::Release => rules.key(true),
        Gen::Plain => "x64-plain".to_string(),
    };
    let mut low = low;
    // --release: a program that reads nothing, worked out now
    let mut evaluated = None;
    // a program that reads its input is never worked out while compiling:
    // what it does depends on what it is given
    let reads = low.funcs.iter().any(|f| f.code.iter().any(|o| matches!(o, ir::Op::ReadToken | ir::Op::ReadLine)));
    let evaluate = evaluate && !reads;
    if gen == Gen::Release && evaluate {
        if let Some(d) = hot::eval(&low.funcs, &low.entry, low.globals, &rules.release.evaluate) {
            let mut code = Vec::new();
            if !d.output.is_empty() {
                code.push(ir::Op::PrintStr(d.output));
            }
            code.push(ir::Op::Push(d.returned));
            code.push(ir::Op::Ret);
            low.funcs = vec![ir::Func::new(low.entry.clone(), 0, 0, code)];
            evaluated = Some(format!("worked out by Strike's IR runner in {} steps", d.steps));
        }
    }
    let tail_calls: usize = if gen == Gen::Plain { 0 } else { low.funcs.iter_mut().map(tail::run).sum() };
    let unrolled = if gen == Gen::Plain { 0 } else { inline::unroll(&mut low.funcs, &rules) };
    let inlined = if gen == Gen::Plain { 0 } else { inline::run(&mut low.funcs, &rules, gen == Gen::Release) };
    let (mut simplified, mut hoisted, mut closed) = (0, 0, 0);
    if gen == Gen::Release {
        for f in low.funcs.iter_mut() {
            fold::run(f);
            simplified += opt::simplify(f);
            hoisted += opt::hoist(f);
            closed += scev::run(f);
            fold::run(f);
            rotate::run(f);
        }
    }
    let mut pack = if brands { Some(Pack::open(brand_dir().join("pack"))) } else { None };
    let mut funcs: Vec<(String, Code)> = x64::runtime(&low.entry);
    let mut struck = 0;
    for f in &low.funcs {
        let mut h = blake3::Hasher::new();
        h.update(f.brand().as_bytes());
        h.update(generator.as_bytes());
        let key: [u8; 32] = *h.finalize().as_bytes();
        let code = match pack.as_ref().and_then(|p| p.get(&key)) {
            Some(c) => {
                struck += 1;
                c
            }
            None => {
                let c = match gen {
                    Gen::Opt => x64opt::compile(f, &rules, false),
                    Gen::Release => x64opt::compile(f, &rules, true),
                    Gen::Plain => x64::compile(f),
                };
                if let Some(p) = pack.as_mut() {
                    p.add(key, &c);
                }
                c
            }
        };
        funcs.push((f.name.clone(), code));
    }
    if let Some(p) = pack {
        p.write();
    }
    let code_seconds = t.elapsed().as_secs_f64();

    let t = Instant::now();
    let bss = x64::GLOBALS + 8 * low.globals;
    let mut exe = pe::link(&funcs, "rt.start", bss)?;
    // --release, too long for the IR runner: the program run once as it
    // was compiled, and the executable only gives its output
    if gen == Gen::Release && evaluate && evaluated.is_none() && rules.release.evaluate.max_seconds > 0.0 {
        let run = Instant::now();
        let mut baking = funcs.clone();
        baking[0] = ("rt.start".to_string(), x64::bake_start(&low.entry));
        // a path with its folder: a bare name would be looked for on PATH
        let path = std::path::absolute(out.with_extension(format!("bake{}.exe", std::process::id()))).map_err(|e| e.to_string())?;
        let baked = pe::link(&baking, "rt.start", bss)?;
        if std::fs::write(&path, &baked).is_ok() {
            let result = hot::bake(&baked, &path, &rules.release.evaluate);
            let _ = std::fs::remove_file(&path);
            if let Some((output, returned)) = result {
                let mut code = Vec::new();
                if !output.is_empty() {
                    code.push(ir::Op::PrintStr(output));
                }
                code.push(ir::Op::Push(returned));
                code.push(ir::Op::Ret);
                let f = ir::Func::new(low.entry.clone(), 0, 0, code);
                let mut only = x64::runtime(&low.entry);
                only.push((f.name.clone(), x64opt::compile(&f, &rules, true)));
                exe = pe::link(&only, "rt.start", bss)?;
                evaluated = Some(format!("run once while compiling ({:.3} s)", run.elapsed().as_secs_f64()));
            }
        }
    }
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
        inlined,
        tail_calls,
        unrolled,
        evaluated,
        simplified,
        hoisted,
        closed,
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

    /// The same work in two languages is the same IR and shares its brand;
    /// a function that means something else in a third does not.
    #[test]
    fn the_same_function_is_the_same_ir_and_a_different_one_is_not() {
        let progs = [
            ("c", "long long sq(long long x) { return x * x + 1; }\nint main() { return sq(3); }\n"),
            ("go", "package main\nfunc sq(x int) int {\n\treturn x*x + 1\n}\nfunc main() {\n\tsq(3)\n}\n"),
            // Python stops where a C int would wrap at 32 bits and a Go int at 64
            ("python", "def sq(x):\n    return x * x + 1\nsq(3)\n"),
            ("c", "int sq(int x) { return x * x + 1; }\nint main() { return sq(3); }\n"),
        ];
        let langs = languages().unwrap();
        let mut brands = Vec::new();
        for (name, text) in progs {
            let l = langs.iter().find(|l| l.name.to_lowercase() == name).unwrap();
            let low = front(text, l).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let f = low.funcs.iter().find(|f| f.name == "u.sq").unwrap();
            brands.push(f.brand());
        }
        assert_eq!(brands[0], brands[1], "a C long long and a Go int are the same work");
        assert_ne!(brands[1], brands[2], "a Python int does not wrap");
        assert_ne!(brands[1], brands[3], "a C int wraps at 32 bits");
        assert_ne!(brands[2], brands[3]);
    }
}
