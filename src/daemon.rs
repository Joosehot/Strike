//! The Strike daemon: one resident process per user (per Strike home) that
//! keeps work hot between commands.
//!
//!   strike daemon start [--idle SECONDS]   start it (default idle exit: 30 min)
//!   strike daemon stop | status
//!   strike daemon watch <dir> | unwatch <dir>
//!
//! What it keeps was chosen by measuring where a small command's time goes
//! (README, The daemon):
//!
//! - `strike compile`: the readings of texts and the --release evaluations
//!   (uc/hot.rs), found by the hash of what they were made from;
//! - `strike build`: rustc's facts (`rustc -vV`, `rustc --print cfg`), three
//!   rustc runs that are 85 of a nothing-changed build's 90 ms, found by
//!   the hash of everything that decides which rustc answers (below).
//!
//! The CLI uses a daemon when one runs and works alone when none does, or
//! when it does not answer: the output is the same either way. The channel
//! is a named pipe that refuses remote clients, so nothing listens on the
//! network. Both sides prove they know a token that only this user can
//! read (`<home>/daemon/state.json`), with a keyed hash over a fresh nonce:
//! a pipe made by someone else is never believed, and another user's
//! client is never served. A daemon from another Strike binary (by the
//! content of the binary) is never used, and `start` replaces it.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

const DEFAULT_IDLE: u64 = 30 * 60;
/// Facts kept for `strike build`; each is a few hundred bytes.
const MAX_FACTS: usize = 256;
const MAX_FRAME: usize = 1 << 28;

fn dir() -> PathBuf {
    crate::lightning::home().join("daemon")
}

fn state_path() -> PathBuf {
    dir().join("state.json")
}

/// What a running daemon writes about itself for its clients.
#[derive(Serialize, Deserialize, Clone)]
struct State {
    pid: u32,
    pipe: String,
    token: String,
    /// BLAKE3 of the Strike binary it runs
    hash: String,
    /// the binary it was started from, its size and time then: a client
    /// that is this file, unchanged, is the same binary without hashing it
    origin: String,
    origin_len: u64,
    origin_mtime: u64,
    idle: u64,
}

#[derive(Serialize, Deserialize)]
enum Req {
    Ping,
    Status,
    Stop,
    Compile { cwd: String, env: Vec<(String, String)>, src: String, out: String, brands: bool, gen: String, evaluate: bool },
    Watch { dir: String },
    Unwatch { dir: String },
    FactGet { key: String },
    FactPut { key: String, value: Vec<u8> },
}

#[derive(Serialize, Deserialize)]
struct Hello {
    nonce: String,
    proof: String,
    req: Req,
}

#[derive(Serialize, Deserialize)]
struct Info {
    pid: u32,
    hash: String,
    up: f64,
    idle_for: f64,
    idle_limit: u64,
    compiles: u64,
    facts: usize,
    readings: usize,
    evaluations: usize,
    hits: u64,
    misses: u64,
    watched: Vec<String>,
}

#[derive(Serialize, Deserialize)]
enum Body {
    Pong { pid: u32 },
    Status(Info),
    Stopping,
    Compiled(Result<crate::uc::Report, String>),
    Refused(String),
    Fact(Option<Vec<u8>>),
    Watching(usize),
    Done,
}

#[derive(Serialize, Deserialize)]
struct Answer {
    proof: String,
    body: Body,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut k = [0u8; 32];
    for (i, b) in k.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(k)
}

fn proof(token: &[u8; 32], side: &str, nonce: &str) -> String {
    hex(blake3::keyed_hash(token, format!("{side}\0{nonce}").as_bytes()).as_bytes())
}

fn stat(p: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(p).ok()?;
    let t = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos() as u64;
    Some((m.len(), t))
}

fn hash_file(p: &Path) -> Option<String> {
    let bytes = std::fs::read(p).ok()?;
    Some(hex(blake3::hash(&bytes).as_bytes()))
}

/// One pipe per user and Strike home.
fn pipe_name() -> String {
    let home = crate::lightning::home();
    let home = std::path::absolute(&home).unwrap_or(home);
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    let h = blake3::hash(format!("{user}\0{}", home.to_string_lossy().to_lowercase()).as_bytes());
    format!(r"\\.\pipe\strike-daemon-{}", &hex(h.as_bytes())[..16])
}

fn frame_write(w: &mut impl Write, data: &[u8]) -> std::io::Result<()> {
    let mut b = Vec::with_capacity(data.len() + 4);
    b.extend_from_slice(&(data.len() as u32).to_le_bytes());
    b.extend_from_slice(data);
    w.write_all(&b)
}

fn frame_read(r: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut n = [0u8; 4];
    r.read_exact(&mut n)?;
    let n = u32::from_le_bytes(n) as usize;
    if n > MAX_FRAME {
        return Err(std::io::Error::other("frame too large"));
    }
    let mut b = vec![0u8; n];
    r.read_exact(&mut b)?;
    Ok(b)
}

// ---------------------------------------------------------------- client

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// STRIKE_DAEMON=off: never
    Off,
    /// the default: when one runs
    On,
    /// STRIKE_DAEMON=auto: and start one when none runs
    Auto,
}

fn mode() -> Mode {
    match std::env::var("STRIKE_DAEMON").unwrap_or_default().to_lowercase().as_str() {
        "off" | "0" | "no" | "false" => Mode::Off,
        "auto" => Mode::Auto,
        _ => Mode::On,
    }
}

enum Found {
    None,
    /// from another binary
    Stale(State),
    Ready(State),
}

fn find() -> Found {
    let Ok(text) = std::fs::read_to_string(state_path()) else { return Found::None };
    let Ok(st) = serde_json::from_str::<State>(&text) else { return Found::None };
    if same_binary(&st) {
        Found::Ready(st)
    } else {
        Found::Stale(st)
    }
}

fn same_binary(st: &State) -> bool {
    let Ok(me) = std::env::current_exe() else { return false };
    if me.to_string_lossy().eq_ignore_ascii_case(&st.origin) && stat(&me) == Some((st.origin_len, st.origin_mtime)) {
        return true;
    }
    hash_file(&me).is_some_and(|h| h == st.hash)
}

fn nonce() -> String {
    hex(&random::<16>())
}

/// One request on its own connection. A daemon that does not answer in
/// `wait` is given up on (the caller then works alone).
fn request(st: &State, req: Req, wait: Duration) -> Result<Body, String> {
    let token = unhex(&st.token).ok_or("the daemon's state has no token")?;
    let n = nonce();
    let hello = Hello { proof: proof(&token, "client", &n), nonce: n.clone(), req };
    let data = serde_json::to_vec(&hello).map_err(|e| e.to_string())?;
    let pipe = st.pipe.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = (|| -> std::io::Result<Vec<u8>> {
            let mut f = transport::open(&pipe)?;
            frame_write(&mut f, &data)?;
            frame_read(&mut f)
        })();
        let _ = tx.send(r);
    });
    let bytes = rx.recv_timeout(wait).map_err(|_| "the daemon did not answer in time".to_string())?.map_err(|e| e.to_string())?;
    let a: Answer = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if a.proof != proof(&token, "server", &n) {
        return Err("the pipe did not answer as this user's daemon".into());
    }
    Ok(a.body)
}

fn ping(st: &State) -> bool {
    matches!(request(st, Req::Ping, Duration::from_secs(3)), Ok(Body::Pong { pid }) if pid == st.pid)
}

/// `strike compile`'s compile: through the daemon when one runs, else (or
/// when it fails to answer) alone. Both give the same executable.
pub fn compile(src: &Path, out: &Path, brands: bool, gen: crate::uc::Gen, evaluate: bool) -> Result<crate::uc::Report, String> {
    if let Some(r) = compile_through(src, out, brands, gen, evaluate) {
        return r;
    }
    crate::uc::compile(src, out, brands, gen, evaluate)
}

fn gen_name(gen: crate::uc::Gen) -> &'static str {
    match gen {
        crate::uc::Gen::Opt => "opt",
        crate::uc::Gen::Release => "release",
        crate::uc::Gen::Plain => "plain",
    }
}

fn compile_through(src: &Path, out: &Path, brands: bool, gen: crate::uc::Gen, evaluate: bool) -> Option<Result<crate::uc::Report, String>> {
    let mode = mode();
    // STRIKE_DUMP_IR prints while compiling, and the daemon's prints go nowhere
    if mode == Mode::Off || std::env::var_os("STRIKE_DUMP_IR").is_some() {
        return None;
    }
    let st = match find() {
        Found::Ready(st) => st,
        Found::None => {
            if mode == Mode::Auto {
                start_detached();
            }
            return None;
        }
        Found::Stale(st) => {
            if mode == Mode::Auto {
                let _ = request(&st, Req::Stop, Duration::from_secs(3));
                start_detached();
            } else {
                eprintln!("strike: the daemon (pid {}) runs another Strike binary, so it is not used; `strike daemon start` replaces it", st.pid);
            }
            return None;
        }
    };
    // the request carries text: a path or variable that is not Unicode is compiled alone
    let cwd = std::env::current_dir().ok()?.to_str()?.to_string();
    let mut env = Vec::new();
    for (k, v) in std::env::vars_os() {
        let (k, v) = (k.into_string().ok()?, v.into_string().ok()?);
        let up = k.to_uppercase();
        if up.starts_with("STRIKE_") && up != "STRIKE_HOME" && !up.starts_with("STRIKE_DAEMON") {
            env.push((k, v));
        }
    }
    let req = Req::Compile {
        cwd,
        env,
        src: src.to_str()?.to_string(),
        out: out.to_str()?.to_string(),
        brands,
        gen: gen_name(gen).to_string(),
        evaluate,
    };
    match request(&st, req, Duration::from_secs(300)) {
        Ok(Body::Compiled(r)) => Some(r),
        _ => None,
    }
}

/// What rustc printed for `args`, and whether it succeeded.
pub struct RustcOut {
    pub ok: bool,
    pub stdout: Vec<u8>,
}

/// `rustc <args>` for a fact about rustc (`-vV`, `--print cfg`): from the
/// daemon when it holds the answer under the same key, else run (and given
/// to the daemon). The key is the hash of everything that decides which
/// rustc answers: see `toolchain_key`.
pub fn rustc(args: &[&str]) -> Result<RustcOut, String> {
    let run = || {
        std::process::Command::new("rustc")
            .args(args)
            .output()
            .map(|o| RustcOut { ok: o.status.success(), stdout: o.stdout })
            .map_err(|e| e.to_string())
    };
    if mode() == Mode::Off {
        return run();
    }
    let Found::Ready(st) = find() else { return run() };
    let Some(tk) = toolchain_key() else { return run() };
    let mut h = blake3::Hasher::new();
    h.update(tk);
    for a in args {
        h.update(&(a.len() as u32).to_le_bytes());
        h.update(a.as_bytes());
    }
    let key = hex(h.finalize().as_bytes());
    if let Ok(Body::Fact(Some(v))) = request(&st, Req::FactGet { key: key.clone() }, Duration::from_secs(3)) {
        return Ok(RustcOut { ok: true, stdout: v });
    }
    let r = run()?;
    if r.ok {
        let _ = request(&st, Req::FactPut { key, value: r.stdout.clone() }, Duration::from_secs(3));
    }
    Ok(r)
}

/// Everything that decides which rustc `Command::new("rustc")` runs and
/// what it says about itself: the variables rustup and rustc read (every
/// RUST*, CARGO* and PATH); every rustc that the search could find (the
/// program's own folder, Windows' folders, each folder of PATH), by path,
/// size and time; rustup's settings and the size and time of each
/// toolchain's rustc; and the rust-toolchain files from the working
/// folder up. A change to any of them is another key, so the daemon's
/// facts are asked for again from rustc.
fn toolchain_key() -> Option<&'static [u8; 32]> {
    static KEY: OnceLock<Option<[u8; 32]>> = OnceLock::new();
    KEY.get_or_init(|| {
        let mut h = blake3::Hasher::new();
        let mut put = |b: &[u8]| {
            h.update(&(b.len() as u64).to_le_bytes());
            h.update(b);
        };
        put(b"rustc facts 1");
        let mut vars: Vec<(String, String)> = std::env::vars_os()
            .map(|(k, v)| (k.to_string_lossy().to_string(), v.to_string_lossy().to_string()))
            .filter(|(k, _)| {
                let k = k.to_uppercase();
                k.starts_with("RUST") || k.starts_with("CARGO") || k == "PATH" || k == "PATHEXT"
            })
            .collect();
        vars.sort();
        for (k, v) in &vars {
            put(k.as_bytes());
            put(v.as_bytes());
        }
        let stat_of = |p: &Path, put: &mut dyn FnMut(&[u8])| {
            put(p.to_string_lossy().as_bytes());
            match stat(p) {
                Some((len, t)) => put(format!("{len} {t}").as_bytes()),
                None => put(b"-"),
            }
        };
        let mut dirs: Vec<PathBuf> = Vec::new();
        dirs.push(std::env::current_exe().ok()?.parent()?.to_path_buf());
        if let Some(root) = std::env::var_os("SystemRoot") {
            dirs.push(PathBuf::from(&root).join("System32"));
            dirs.push(PathBuf::from(&root));
        }
        if let Some(path) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&path));
        }
        for d in &dirs {
            for name in ["rustc.exe", "rustc"] {
                stat_of(&d.join(name), &mut put);
            }
        }
        let user = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
        let rustup = std::env::var_os("RUSTUP_HOME").map(PathBuf::from).unwrap_or_else(|| user.join(".rustup"));
        put(&std::fs::read(rustup.join("settings.toml")).unwrap_or_else(|_| b"-".to_vec()));
        let mut chains: Vec<PathBuf> = std::fs::read_dir(rustup.join("toolchains")).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        chains.sort();
        for c in &chains {
            stat_of(&c.join("bin").join("rustc.exe"), &mut put);
            stat_of(&c.join("bin").join("rustc"), &mut put);
        }
        let cwd = std::env::current_dir().ok()?;
        for d in cwd.ancestors() {
            for name in ["rust-toolchain", "rust-toolchain.toml"] {
                let p = d.join(name);
                put(p.to_string_lossy().as_bytes());
                put(&std::fs::read(&p).map(|mut b| {
                    b.insert(0, b'+');
                    b
                }).unwrap_or_else(|_| b"-".to_vec()));
            }
        }
        Some(*h.finalize().as_bytes())
    })
    .as_ref()
}

// ---------------------------------------------------------------- the CLI

pub fn command(args: &[String]) -> Result<ExitCode, String> {
    let usage = "strike daemon start [--idle SECONDS] | stop | status | watch <dir> | unwatch <dir>";
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    match args.first().map(String::as_str) {
        Some("start") => {
            let idle = match flag("--idle") {
                Some(s) => s.parse().map_err(|_| "--idle needs whole seconds")?,
                None => std::env::var("STRIKE_DAEMON_IDLE").ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_IDLE),
            };
            start(idle, true)?;
            Ok(ExitCode::SUCCESS)
        }
        Some("stop") => {
            match stop()? {
                Some(pid) => println!("strike daemon: stopped (pid {pid})"),
                None => println!("strike daemon: not running"),
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("status") => {
            match find() {
                Found::None => println!("strike daemon: not running"),
                Found::Stale(st) => {
                    if ping(&st) {
                        println!("strike daemon: stale: pid {} runs another Strike binary ({}), so it is not used; `strike daemon start` replaces it", st.pid, &st.hash[..12]);
                    } else {
                        println!("strike daemon: not running");
                    }
                }
                Found::Ready(st) => match request(&st, Req::Status, Duration::from_secs(3)) {
                    Ok(Body::Status(i)) => {
                        let watched = if i.watched.is_empty() { "nothing".to_string() } else { i.watched.join(", ") };
                        println!(
                            "strike daemon: running, pid {}, binary {}, up {:.1} s, idle {:.1} s of {} s; {} compiles; {} rustc facts; memory: {} readings, {} evaluations, {} hits, {} misses; watching {watched}",
                            i.pid, &i.hash[..12], i.up, i.idle_for, i.idle_limit, i.compiles, i.facts, i.readings, i.evaluations, i.hits, i.misses
                        );
                    }
                    _ => println!("strike daemon: not running (pid {} does not answer)", st.pid),
                },
            }
            Ok(ExitCode::SUCCESS)
        }
        Some(w @ ("watch" | "unwatch")) => {
            let d = args.get(1).ok_or(usage)?;
            let d = std::path::absolute(d).map_err(|e| e.to_string())?;
            if w == "watch" && !d.is_dir() {
                return Err(format!("{}: not a folder", d.display()));
            }
            let Found::Ready(st) = find() else { return Err("the daemon is not running (strike daemon start)".into()) };
            let dir = d.to_str().ok_or("the folder's name is not Unicode")?.to_string();
            let req = if w == "watch" { Req::Watch { dir } } else { Req::Unwatch { dir } };
            match request(&st, req, Duration::from_secs(60))? {
                Body::Watching(n) => println!("strike daemon: watching {}; {n} files read ahead", d.display()),
                Body::Done => println!("strike daemon: no longer watching {}", d.display()),
                _ => return Err("the daemon refused".into()),
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("run") => {
            let idle = flag("--idle").and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_IDLE);
            let origin = PathBuf::from(flag("--origin").ok_or("daemon run needs --origin")?);
            server::serve(idle, &origin)?;
            Ok(ExitCode::SUCCESS)
        }
        _ => Err(usage.into()),
    }
}

/// Starts a daemon (replacing one from another binary) and waits until it
/// answers. It runs from a copy of this binary in `<home>/daemon/bin`, so
/// the binary itself can be rebuilt while the daemon runs (a new one is
/// then another binary, and is not served by the old daemon).
fn start(idle: u64, say: bool) -> Result<(), String> {
    match find() {
        Found::Ready(st) if ping(&st) => {
            if say {
                println!("strike daemon: already running (pid {})", st.pid);
            }
            return Ok(());
        }
        Found::Stale(_) => {
            if let Some(pid) = stop()? {
                if say {
                    println!("strike daemon: stopped pid {pid}, which ran another Strike binary");
                }
            }
        }
        _ => {}
    }
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&me).map_err(|e| format!("{}: {e}", me.display()))?;
    let hash = hex(blake3::hash(&bytes).as_bytes());
    let bin = dir().join("bin");
    std::fs::create_dir_all(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let copy = bin.join(format!("strike-{}{}", &hash[..16], std::env::consts::EXE_SUFFIX));
    if hash_file(&copy).as_deref() != Some(hash.as_str()) {
        let tmp = bin.join(format!("new-{}-{}", std::process::id(), &hash[..16]));
        std::fs::write(&tmp, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
        if std::fs::rename(&tmp, &copy).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    // copies of older binaries; one still running is kept by Windows
    if let Ok(rd) = std::fs::read_dir(&bin) {
        for e in rd.flatten() {
            if e.path() != copy && e.file_name().to_string_lossy().starts_with("strike-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let mut child = transport::spawn_detached(&copy, &["daemon", "run", "--idle", &idle.to_string(), "--origin", me.to_str().ok_or("the binary's path is not Unicode")?], &dir())?;
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if let Found::Ready(st) = find() {
            if st.pid == child.id() && ping(&st) {
                if say {
                    println!("strike daemon: started, pid {}, binary {}, idle exit after {idle} s", st.pid, &hash[..12]);
                }
                return Ok(());
            }
        }
        if let Ok(Some(s)) = child.try_wait() {
            return Err(format!("the daemon stopped at once ({s}); another may hold its pipe"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err("the daemon did not answer within 10 s".into())
}

/// For STRIKE_DAEMON=auto: `strike daemon start` in the background; this
/// command does not wait for it.
fn start_detached() {
    if let Ok(me) = std::env::current_exe() {
        let _ = transport::spawn_detached(&me, &["daemon", "start"], &std::env::temp_dir());
    }
}

/// Stops the daemon by its own command and waits until its pipe is gone.
/// The pid it had, or None when none was running.
fn stop() -> Result<Option<u32>, String> {
    let st = match find() {
        Found::None => return Ok(None),
        Found::Stale(st) | Found::Ready(st) => st,
    };
    match request(&st, Req::Stop, Duration::from_secs(10)) {
        Ok(Body::Stopping) => {}
        _ => {
            // nothing answers: a daemon that crashed left its state
            if !transport::exists(&st.pipe) {
                let _ = std::fs::remove_file(state_path());
                return Ok(None);
            }
            return Err(format!("the daemon (pid {}) does not answer", st.pid));
        }
    }
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if !transport::exists(&st.pipe) {
            return Ok(Some(st.pid));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(format!("the daemon (pid {}) did not stop within 10 s", st.pid))
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    transport::random(&mut b);
    b
}

// ---------------------------------------------------------------- the daemon

mod server {
    use super::*;
    use std::collections::HashMap;

    struct Server {
        token: [u8; 32],
        hash: String,
        home: PathBuf,
        started: Instant,
        idle: Duration,
        last: Mutex<Instant>,
        compiles: AtomicU64,
        facts: Mutex<(HashMap<String, Vec<u8>>, Vec<String>)>,
        watched: Mutex<Vec<PathBuf>>,
        seen: Mutex<HashMap<PathBuf, (u64, u64)>>,
        extensions: Vec<String>,
    }

    /// Compiles run one at a time: each takes the client's working folder
    /// and STRIKE_ variables, which belong to the whole process.
    static COMPILE: Mutex<()> = Mutex::new(());

    fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn serve(idle: u64, origin: &Path) -> Result<(), String> {
        let home = dir();
        std::fs::create_dir_all(&home).map_err(|e| format!("{}: {e}", home.display()))?;
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        let hash = hash_file(&me).ok_or("cannot read the daemon's own binary")?;
        let (origin_len, origin_mtime) = stat(origin).ok_or("cannot read the binary it was started from")?;
        if hash_file(origin).as_deref() != Some(hash.as_str()) {
            return Err("the binary changed while the daemon started".into());
        }
        let pipe = pipe_name();
        // the first instance: if another process holds the name, this is not the daemon
        let mut instance = transport::Instance::create(&pipe, true).map_err(|e| format!("{pipe}: {e} (is another daemon running?)"))?;
        let token = random::<32>();
        let st = State {
            pid: std::process::id(),
            pipe: pipe.clone(),
            token: hex(&token),
            hash: hash.clone(),
            origin: origin.to_string_lossy().to_string(),
            origin_len,
            origin_mtime,
            idle,
        };
        let tmp = home.join(format!("state.{}.json", std::process::id()));
        std::fs::write(&tmp, serde_json::to_vec_pretty(&st).unwrap()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, state_path()).map_err(|e| e.to_string())?;
        crate::uc::hot::enable();
        // its own folder, never a user's: a working folder cannot be deleted on Windows
        let _ = std::env::set_current_dir(&home);
        let extensions = crate::uc::languages().map(|ls| ls.iter().flat_map(|l| l.extensions.iter().map(|e| e.to_lowercase())).collect()).unwrap_or_default();
        let srv: &'static Server = Box::leak(Box::new(Server {
            token,
            hash,
            home,
            started: Instant::now(),
            idle: Duration::from_secs(idle),
            last: Mutex::new(Instant::now()),
            compiles: AtomicU64::new(0),
            facts: Mutex::new((HashMap::new(), Vec::new())),
            watched: Mutex::new(Vec::new()),
            seen: Mutex::new(HashMap::new()),
            extensions,
        }));
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(100));
            if lock(&srv.last).elapsed() >= srv.idle {
                shutdown();
            }
        });
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(500));
            let dirs = lock(&srv.watched).clone();
            for d in &dirs {
                scan(srv, d);
            }
        });
        loop {
            let connected = instance.connect();
            let next = loop {
                match transport::Instance::create(&pipe, false) {
                    Ok(i) => break i,
                    Err(_) => std::thread::sleep(Duration::from_millis(50)),
                }
            };
            let this = std::mem::replace(&mut instance, next);
            if connected {
                let f = this.into_file();
                std::thread::spawn(move || serve_one(srv, f));
            }
        }
    }

    /// Ends the daemon once no compile is running (one that is waiting gets
    /// no answer, and its client works alone).
    fn shutdown() -> ! {
        let _g = lock(&COMPILE);
        if let Ok(text) = std::fs::read_to_string(state_path()) {
            if serde_json::from_str::<State>(&text).is_ok_and(|s| s.pid == std::process::id()) {
                let _ = std::fs::remove_file(state_path());
            }
        }
        std::process::exit(0)
    }

    fn touch(srv: &Server) {
        *lock(&srv.last) = Instant::now();
    }

    fn serve_one(srv: &'static Server, mut f: std::fs::File) {
        let Ok(bytes) = frame_read(&mut f) else { return };
        let Ok(hello) = serde_json::from_slice::<Hello>(&bytes) else { return };
        if hello.proof != proof(&srv.token, "client", &hello.nonce) {
            return; // not this user's client: no answer at all
        }
        let mut stopping = false;
        let body = match hello.req {
            Req::Ping => Body::Pong { pid: std::process::id() },
            Req::Status => {
                let (readings, evaluations, hits, misses) = crate::uc::hot::stats();
                Body::Status(Info {
                    pid: std::process::id(),
                    hash: srv.hash.clone(),
                    up: srv.started.elapsed().as_secs_f64(),
                    idle_for: lock(&srv.last).elapsed().as_secs_f64(),
                    idle_limit: srv.idle.as_secs(),
                    compiles: srv.compiles.load(Ordering::Relaxed),
                    facts: lock(&srv.facts).0.len(),
                    readings,
                    evaluations,
                    hits,
                    misses,
                    watched: lock(&srv.watched).iter().map(|d| d.display().to_string()).collect(),
                })
            }
            Req::Stop => {
                stopping = true;
                Body::Stopping
            }
            Req::Compile { cwd, env, src, out, brands, gen, evaluate } => {
                touch(srv);
                let gen = match gen.as_str() {
                    "opt" => Some(crate::uc::Gen::Opt),
                    "release" => Some(crate::uc::Gen::Release),
                    "plain" => Some(crate::uc::Gen::Plain),
                    _ => None,
                };
                let body = match gen {
                    None => Body::Refused("unknown generator".into()),
                    Some(gen) => {
                        let _g = lock(&COMPILE);
                        let r = if std::env::set_current_dir(&cwd).is_err() {
                            Body::Refused(format!("{cwd}: not a folder here"))
                        } else {
                            for (k, _) in std::env::vars_os() {
                                let up = k.to_string_lossy().to_uppercase();
                                if up.starts_with("STRIKE_") && up != "STRIKE_HOME" {
                                    std::env::remove_var(&k);
                                }
                            }
                            for (k, v) in &env {
                                std::env::set_var(k, v);
                            }
                            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::uc::compile(Path::new(&src), Path::new(&out), brands, gen, evaluate)));
                            srv.compiles.fetch_add(1, Ordering::Relaxed);
                            match r {
                                Ok(r) => Body::Compiled(r),
                                // the client compiles alone, and meets the same panic there
                                Err(_) => Body::Refused("the compile panicked".into()),
                            }
                        };
                        let _ = std::env::set_current_dir(&srv.home);
                        r
                    }
                };
                touch(srv);
                body
            }
            Req::Watch { dir } => {
                touch(srv);
                let d = PathBuf::from(dir);
                {
                    let mut w = lock(&srv.watched);
                    if !w.contains(&d) {
                        w.push(d.clone());
                    }
                }
                Body::Watching(scan(srv, &d))
            }
            Req::Unwatch { dir } => {
                let d = PathBuf::from(dir);
                lock(&srv.watched).retain(|x| *x != d);
                lock(&srv.seen).retain(|p, _| !p.starts_with(&d));
                Body::Done
            }
            Req::FactGet { key } => {
                touch(srv);
                Body::Fact(lock(&srv.facts).0.get(&key).cloned())
            }
            Req::FactPut { key, value } => {
                touch(srv);
                let mut g = lock(&srv.facts);
                let (map, order) = &mut *g;
                if !map.contains_key(&key) {
                    if map.len() >= MAX_FACTS && !order.is_empty() {
                        let old = order.remove(0);
                        map.remove(&old);
                    }
                    order.push(key.clone());
                    map.insert(key, value);
                }
                Body::Done
            }
        };
        let a = Answer { proof: proof(&srv.token, "server", &hello.nonce), body };
        let Ok(data) = serde_json::to_vec(&a) else { return };
        if frame_write(&mut f, &data).is_ok() {
            // waits until the client has read it
            let _ = f.sync_all();
        }
        if stopping {
            shutdown();
        }
    }

    /// One look at a watched folder: every file of a language whose size or
    /// time changed is read ahead. How many readings were made.
    fn scan(srv: &Server, root: &Path) -> usize {
        let mut made = 0;
        let mut stack = vec![root.to_path_buf()];
        let mut budget = 20_000usize;
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                if budget == 0 {
                    return made;
                }
                budget -= 1;
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_dir() {
                    if !name.starts_with('.') && !matches!(name.as_str(), "target" | "strike-out" | "node_modules") {
                        stack.push(p);
                    }
                    continue;
                }
                let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
                if !srv.extensions.contains(&ext) {
                    continue;
                }
                let Some(now) = stat(&p) else { continue };
                if lock(&srv.seen).get(&p) == Some(&now) {
                    continue;
                }
                made += crate::uc::hot::warm(&p);
                lock(&srv.seen).insert(p, now);
            }
        }
        made
    }
}

// ---------------------------------------------------------------- the pipe

#[cfg(windows)]
mod transport {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::FromRawHandle;
    use std::os::windows::process::CommandExt;
    use std::path::Path;

    type Handle = *mut c_void;
    const INVALID: Handle = -1isize as Handle;
    const PIPE_ACCESS_DUPLEX: u32 = 3;
    const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
    const PIPE_REJECT_REMOTE_CLIENTS: u32 = 8;
    const PIPE_UNLIMITED_INSTANCES: u32 = 255;
    const ERROR_PIPE_CONNECTED: i32 = 535;
    const ERROR_PIPE_BUSY: i32 = 231;
    /// the daemon may know who its client is, never act as it
    const SECURITY_IDENTIFICATION: u32 = 1 << 16;
    const DETACHED_PROCESS: u32 = 0x8;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x200;
    const HANDLE_FLAG_INHERIT: u32 = 1;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateNamedPipeW(name: *const u16, open: u32, mode: u32, max: u32, out: u32, inb: u32, timeout: u32, sa: *mut c_void) -> Handle;
        fn ConnectNamedPipe(h: Handle, ov: *mut c_void) -> i32;
        fn WaitNamedPipeW(name: *const u16, timeout: u32) -> i32;
        fn CloseHandle(h: Handle) -> i32;
        fn GetStdHandle(n: u32) -> Handle;
        fn SetHandleInformation(h: Handle, mask: u32, flags: u32) -> i32;
    }
    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptGenRandom(alg: *mut c_void, buf: *mut u8, len: u32, flags: u32) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    pub fn random(b: &mut [u8]) {
        // BCRYPT_USE_SYSTEM_PREFERRED_RNG
        let r = unsafe { BCryptGenRandom(std::ptr::null_mut(), b.as_mut_ptr(), b.len() as u32, 2) };
        assert_eq!(r, 0, "no random numbers from Windows");
    }

    pub struct Instance(Handle);

    // a handle is only a number; one thread uses it at a time
    unsafe impl Send for Instance {}

    impl Instance {
        pub fn create(name: &str, first: bool) -> std::io::Result<Instance> {
            let open = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
            let h = unsafe { CreateNamedPipeW(wide(name).as_ptr(), open, PIPE_REJECT_REMOTE_CLIENTS, PIPE_UNLIMITED_INSTANCES, 65536, 65536, 0, std::ptr::null_mut()) };
            if h == INVALID {
                return Err(std::io::Error::last_os_error());
            }
            Ok(Instance(h))
        }
        /// Waits for a client; false if the one that came is already gone.
        pub fn connect(&mut self) -> bool {
            let ok = unsafe { ConnectNamedPipe(self.0, std::ptr::null_mut()) };
            ok != 0 || std::io::Error::last_os_error().raw_os_error() == Some(ERROR_PIPE_CONNECTED)
        }
        pub fn into_file(self) -> std::fs::File {
            let h = self.0;
            std::mem::forget(self);
            unsafe { std::fs::File::from_raw_handle(h) }
        }
    }

    impl Drop for Instance {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    pub fn open(name: &str) -> std::io::Result<std::fs::File> {
        let mut tries = 0;
        loop {
            match std::fs::OpenOptions::new().read(true).write(true).security_qos_flags(SECURITY_IDENTIFICATION).open(name) {
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && tries < 50 => {
                    tries += 1;
                    unsafe { WaitNamedPipeW(wide(name).as_ptr(), 200) };
                }
                r => return r,
            }
        }
    }

    /// Is there a pipe of this name? (one busy with a client counts)
    pub fn exists(name: &str) -> bool {
        match std::fs::OpenOptions::new().read(true).write(true).security_qos_flags(SECURITY_IDENTIFICATION).open(name) {
            Ok(_) => true,
            Err(e) => e.raw_os_error() == Some(ERROR_PIPE_BUSY),
        }
    }

    /// A process of its own, with no console and nothing of this one's:
    /// this command's output handles are made uninheritable first, so a
    /// caller reading them is not kept waiting by the daemon.
    pub fn spawn_detached(exe: &Path, args: &[&str], cwd: &Path) -> Result<std::process::Child, String> {
        for n in [-10i32, -11, -12] {
            unsafe {
                let h = GetStdHandle(n as u32);
                if !h.is_null() && h != INVALID {
                    SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0);
                }
            }
        }
        std::process::Command::new(exe)
            .args(args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .map_err(|e| format!("{}: {e}", exe.display()))
    }
}

#[cfg(not(windows))]
mod transport {
    use std::path::Path;
    const NO: &str = "the Strike daemon needs Windows' named pipes for now";
    pub fn random(b: &mut [u8]) {
        use std::io::Read;
        std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(b)).expect("no /dev/urandom");
    }
    pub struct Instance;
    impl Instance {
        pub fn create(_: &str, _: bool) -> std::io::Result<Instance> {
            Err(std::io::Error::other(NO))
        }
        pub fn connect(&mut self) -> bool {
            false
        }
        pub fn into_file(self) -> std::fs::File {
            unreachable!()
        }
    }
    pub fn open(_: &str) -> std::io::Result<std::fs::File> {
        Err(std::io::Error::other(NO))
    }
    pub fn exists(_: &str) -> bool {
        false
    }
    pub fn spawn_detached(_: &Path, _: &[&str], _: &Path) -> Result<std::process::Child, String> {
        Err(NO.into())
    }
}
