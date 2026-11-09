//! The daemon's memory for the universal compiler (src/daemon.rs). Off in
//! a plain `strike compile`, where a process lives for one file; on in the
//! daemon, which lives for many.
//!
//! What it keeps was measured as the work of a compile that does not
//! depend on anything but its inputs:
//!
//! - **readings**: a description's reading of a text (lex, parse, lower),
//!   under the hash of the description's name and the text. On a program
//!   of 2000 functions this is 65 of a compile's 73 ms.
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
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

type Reading = Result<Lowered, (u32, String, f64)>;

static ON: AtomicBool = AtomicBool::new(false);
static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);

/// At most this many readings, and this many bytes of the texts read; the
/// oldest goes first.
const MAX_READINGS: usize = 1024;
const MAX_TEXT: usize = 32 << 20;
const MAX_EVALS: usize = 256;

struct Memory {
    readings: HashMap<[u8; 32], (Reading, usize)>,
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

fn reading_key(text: &str, l: &Lang) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"reading\0");
    h.update(l.name.as_bytes());
    h.update(b"\0");
    h.update(text.as_bytes());
    *h.finalize().as_bytes()
}

/// A description's reading of a text: from the memory, or read now (and
/// kept). `read` must be a function of the text and the description only.
pub fn front(text: &str, l: &Lang, read: fn(&str, &Lang) -> Reading) -> Reading {
    if !on() {
        return read(text, l);
    }
    let key = reading_key(text, l);
    if let Some((r, _)) = memory().as_ref().unwrap().readings.get(&key) {
        HITS.fetch_add(1, Ordering::Relaxed);
        return r.clone();
    }
    MISSES.fetch_add(1, Ordering::Relaxed);
    let r = read(text, l);
    keep_reading(key, r.clone(), text.len());
    r
}

fn keep_reading(key: [u8; 32], r: Reading, size: usize) {
    let mut g = memory();
    let m = g.as_mut().unwrap();
    if m.readings.contains_key(&key) || size > MAX_TEXT {
        return;
    }
    while m.readings.len() >= MAX_READINGS || m.text + size > MAX_TEXT {
        let Some(old) = m.order.pop_front() else { break };
        if let Some((_, n)) = m.readings.remove(&old) {
            m.text -= n;
        }
    }
    m.readings.insert(key, (r, size));
    m.order.push_back(key);
    m.text += size;
}

/// Reads a file ahead, in every description that names its extension, so
/// that the next compile of it finds the reading. How many were read.
pub fn warm(path: &std::path::Path) -> usize {
    let Ok(text) = std::fs::read_to_string(path) else { return 0 };
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let Ok(langs) = super::languages() else { return 0 };
    let mut n = 0;
    for l in langs.iter().filter(|l| l.extensions.iter().any(|e| e.eq_ignore_ascii_case(&ext))) {
        let key = reading_key(&text, l);
        if memory().as_ref().unwrap().readings.contains_key(&key) {
            continue;
        }
        keep_reading(key, super::read_front(&text, l), text.len());
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
