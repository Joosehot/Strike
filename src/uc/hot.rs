//! The daemon's memory for the universal compiler (src/daemon.rs). Off in
//! a plain `strike compile`, where a process lives for one compile; on in
//! the daemon, which lives for many.
//!
//! What it keeps was measured as the work of a compile that does not
//! depend on anything but its inputs:
//!
//! - **readings**: a description's reading of a program (preprocess, lex,
//!   parse, the files made one, lower), under the hash of the description's
//!   name, the files given and their contents. A reading pulls in other
//!   files too (headers, imported modules and packages, go.mod), and looks
//!   for some that are not there; every such look is kept with the reading
//!   (a file's content, or that there is none; a directory's listing), and
//!   the reading is used again only while every look still finds the same.
//!   On a program of 2000 functions this is 65 of a compile's 73 ms.
//! - **evaluations** (--release): what Strike's IR runner made of a
//!   program, under the hash of the whole IR and the runner's limits; and
//!   what a baking executable printed, under the hash of its bytes.
//!
//! Every entry is found by content, never by a file's name or time, so an
//! entry is either exactly what the work would give or not found. The
//! descriptions themselves are built into the binary, and a daemon from
//! another binary is never used (src/daemon.rs), so a description's name
//! names its content.

use super::eval::{Done, Limits};
use super::ir::Func;
use super::lang::Lang;
use super::lower::Lowered;
use super::modules::Fail;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

/// A description's reading of a program: its IR and every file read, or
/// where it stopped.
pub type Reading = Result<(Lowered, Vec<PathBuf>), Fail>;

/// What a reading looked at, and what it found there.
#[derive(Clone)]
enum Look {
    /// a file's content (none: no file there to read)
    File(PathBuf, Option<[u8; 32]>),
    /// a directory's listing (none: no directory there)
    Dir(PathBuf, Option<[u8; 32]>),
}

static ON: AtomicBool = AtomicBool::new(false);
static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);

/// At most this many readings, and this many bytes of the texts read; the
/// oldest goes first.
const MAX_READINGS: usize = 1024;
const MAX_TEXT: usize = 32 << 20;
const MAX_EVALS: usize = 256;

struct Memory {
    readings: HashMap<[u8; 32], (Reading, Vec<Look>, usize)>,
    order: VecDeque<[u8; 32]>,
    text: usize,
    evals: HashMap<[u8; 32], Option<(Vec<u8>, i64, u64)>>,
    eval_order: VecDeque<[u8; 32]>,
    bakes: HashMap<[u8; 32], (Vec<u8>, i64)>,
    bake_order: VecDeque<[u8; 32]>,
}

static MEMORY: Mutex<Option<Memory>> = Mutex::new(None);

fn memory() -> std::sync::MutexGuard<'static, Option<Memory>> {
    // a compile that panicked holding it leaves whole entries only
    let mut m = MEMORY.lock().unwrap_or_else(|e| e.into_inner());
    if m.is_none() {
        *m = Some(Memory {
            readings: HashMap::new(),
            order: VecDeque::new(),
            text: 0,
            evals: HashMap::new(),
            eval_order: VecDeque::new(),
            bakes: HashMap::new(),
            bake_order: VecDeque::new(),
        });
    }
    m
}

/// Turns the memory on (the daemon does, once).
pub fn enable() {
    ON.store(true, Ordering::SeqCst);
}

fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// (readings kept, evaluations kept, hits, misses)
pub fn stats() -> (usize, usize, u64, u64) {
    let m = memory();
    let m = m.as_ref().unwrap();
    (m.readings.len(), m.evals.len() + m.bakes.len(), HITS.load(Ordering::Relaxed), MISSES.load(Ordering::Relaxed))
}

thread_local! {
    /// the looks of the reading being made on this thread, if one is kept
    static LOOKS: RefCell<Option<Vec<Look>>> = const { RefCell::new(None) };
}

fn note(l: Look) {
    LOOKS.with(|s| {
        if let Some(v) = s.borrow_mut().as_mut() {
            v.push(l);
        }
    });
}

fn hash(b: &[u8]) -> [u8; 32] {
    *blake3::hash(b).as_bytes()
}

fn file_state(p: &Path) -> Option<[u8; 32]> {
    if !p.is_file() {
        return None;
    }
    std::fs::read(p).ok().map(|b| hash(&b))
}

fn dir_state(d: &Path) -> Option<[u8; 32]> {
    let mut names: Vec<(String, bool)> = std::fs::read_dir(d).ok()?.flatten().map(|e| (e.file_name().to_string_lossy().to_string(), e.path().is_file())).collect();
    names.sort();
    Some(hash(format!("{names:?}").as_bytes()))
}

/// A file read by a reading (the reading notes what it found).
pub fn read_file(p: &Path) -> std::io::Result<String> {
    let r = std::fs::read(p);
    note(Look::File(p.to_path_buf(), r.as_ref().ok().filter(|_| p.is_file()).map(|b| hash(b))));
    String::from_utf8(r?).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Is there a file? (the reading notes what it found)
pub fn is_file(p: &Path) -> bool {
    let s = file_state(p);
    note(Look::File(p.to_path_buf(), s));
    s.is_some() || p.is_file()
}

/// A directory's entries (the reading notes its listing).
pub fn list(d: &Path) -> Vec<PathBuf> {
    note(Look::Dir(d.to_path_buf(), dir_state(d)));
    std::fs::read_dir(d).map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).collect()).unwrap_or_default()
}

fn still(l: &Look) -> bool {
    match l {
        Look::File(p, h) => file_state(p) == *h,
        Look::Dir(d, h) => dir_state(d) == *h,
    }
}

/// The key of a reading: the description, the files given and their
/// contents, where #include looks, and the folder a relative path is
/// read from. None: a file given cannot be read.
fn reading_key(l: &Lang, units: &[PathBuf], include: &[PathBuf]) -> Option<([u8; 32], usize)> {
    let mut h = blake3::Hasher::new();
    h.update(b"reading\0");
    h.update(l.name.as_bytes());
    let mut size = 0;
    for u in units {
        let b = std::fs::read(u).ok()?;
        size += b.len();
        h.update(b"\0unit\0");
        h.update(u.to_string_lossy().as_bytes());
        h.update(&hash(&b));
    }
    for d in include {
        h.update(b"\0include\0");
        h.update(d.to_string_lossy().as_bytes());
    }
    if units.iter().chain(include).any(|p| p.is_relative()) {
        h.update(b"\0cwd\0");
        h.update(std::env::current_dir().ok()?.to_string_lossy().as_bytes());
    }
    Some((*h.finalize().as_bytes(), size))
}

type Read = fn(&Lang, &[PathBuf], &[PathBuf]) -> Reading;

/// A reading made now, with what it looked at.
fn made(read: Read, l: &Lang, units: &[PathBuf], include: &[PathBuf]) -> (Reading, Vec<Look>) {
    LOOKS.with(|s| *s.borrow_mut() = Some(Vec::new()));
    let r = read(l, units, include);
    let looks = LOOKS.with(|s| s.borrow_mut().take()).unwrap_or_default();
    (r, looks)
}

/// A description's reading of a program: from the memory while everything
/// it looked at is still the same, or read now (and kept). `read` must be
/// a function of the description and the files only.
pub fn front(l: &Lang, units: &[PathBuf], include: &[PathBuf], read: Read) -> Reading {
    if !on() {
        return read(l, units, include);
    }
    let Some((key, size)) = reading_key(l, units, include) else { return read(l, units, include) };
    let kept = memory().as_ref().unwrap().readings.get(&key).map(|(r, looks, _)| (r.clone(), looks.clone()));
    if let Some((r, looks)) = kept {
        if looks.iter().all(still) {
            HITS.fetch_add(1, Ordering::Relaxed);
            return r;
        }
    }
    MISSES.fetch_add(1, Ordering::Relaxed);
    let (r, looks) = made(read, l, units, include);
    keep_reading(key, r.clone(), looks, size);
    r
}

fn keep_reading(key: [u8; 32], r: Reading, looks: Vec<Look>, size: usize) {
    let mut g = memory();
    let m = g.as_mut().unwrap();
    if size > MAX_TEXT {
        return;
    }
    // one whose looks no longer find the same is replaced
    if let Some((_, _, n)) = m.readings.remove(&key) {
        m.text -= n;
        m.order.retain(|k| *k != key);
    }
    while m.readings.len() >= MAX_READINGS || m.text + size > MAX_TEXT {
        let Some(old) = m.order.pop_front() else { break };
        if let Some((_, _, n)) = m.readings.remove(&old) {
            m.text -= n;
        }
    }
    m.readings.insert(key, (r, looks, size));
    m.order.push_back(key);
    m.text += size;
}

/// Reads a file ahead, as a program of one file, in every description that
/// names its extension, so that the next compile of it finds the reading.
/// How many were read.
pub fn warm(path: &Path) -> usize {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let Ok(langs) = super::languages() else { return 0 };
    let units = [path.to_path_buf()];
    let mut n = 0;
    for l in langs.iter().filter(|l| l.extensions.iter().any(|e| e.eq_ignore_ascii_case(&ext))) {
        let Some((key, size)) = reading_key(l, &units, &[]) else { return n };
        if memory().as_ref().unwrap().readings.contains_key(&key) {
            continue;
        }
        let (r, looks) = made(super::modules::front, l, &units, &[]);
        keep_reading(key, r, looks, size);
        n += 1;
    }
    n
}

/// Strike's IR runner on a program: from the memory, or run now. The key
/// is the whole IR (its Debug form holds every field) and the limits.
pub fn eval(funcs: &[Func], entry: &str, globals: u32, lim: &Limits) -> Option<Done> {
    if !on() {
        return super::eval::run(funcs, entry, globals, lim);
    }
    let mut h = blake3::Hasher::new();
    h.update(b"eval\0");
    h.update(format!("{funcs:?}\0{entry}\0{globals}\0{} {} {}", lim.max_steps, lim.max_output, lim.max_depth).as_bytes());
    let key = *h.finalize().as_bytes();
    if let Some(r) = memory().as_ref().unwrap().evals.get(&key) {
        HITS.fetch_add(1, Ordering::Relaxed);
        return r.clone().map(|(output, returned, steps)| Done { output, returned, steps });
    }
    MISSES.fetch_add(1, Ordering::Relaxed);
    // the runner counts steps, not time: its answer, found or not, is the same every time
    let d = super::eval::run(funcs, entry, globals, lim);
    let mut g = memory();
    let m = g.as_mut().unwrap();
    if m.evals.len() >= MAX_EVALS {
        if let Some(old) = m.eval_order.pop_front() {
            m.evals.remove(&old);
        }
    }
    m.evals.insert(key, d.as_ref().map(|d| (d.output.clone(), d.returned, d.steps)));
    m.eval_order.push_back(key);
    d
}

/// A baking executable run once: from the memory, or run now. Only an
/// answer is kept: a run that did not end in time may end on a quieter
/// machine, so it is run again next time, as it would be without the
/// daemon.
pub fn bake(exe: &[u8], path: &std::path::Path, lim: &Limits) -> Option<(Vec<u8>, i64)> {
    if !on() {
        return super::eval::bake(path, lim);
    }
    let mut h = blake3::Hasher::new();
    h.update(b"bake\0");
    h.update(format!("{} {} {} {}\0", lim.max_steps, lim.max_output, lim.max_depth, lim.max_seconds).as_bytes());
    h.update(exe);
    let key = *h.finalize().as_bytes();
    if let Some(r) = memory().as_ref().unwrap().bakes.get(&key) {
        HITS.fetch_add(1, Ordering::Relaxed);
        return Some(r.clone());
    }
    MISSES.fetch_add(1, Ordering::Relaxed);
    let r = super::eval::bake(path, lim)?;
    let mut g = memory();
    let m = g.as_mut().unwrap();
    if m.bakes.len() >= MAX_EVALS {
        if let Some(old) = m.bake_order.pop_front() {
            m.bakes.remove(&old);
        }
    }
    m.bakes.insert(key, r.clone());
    m.bake_order.push_back(key);
    Some(r)
}
