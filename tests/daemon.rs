//! The Strike daemon. Every test has its own Strike home, so its own
//! daemon and pipe, and a guard that stops that daemon however the test
//! ends: no test leaves a daemon running.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

const STRIKE: &str = env!("CARGO_BIN_EXE_strike");

fn root(name: &str) -> PathBuf {
    let r = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("daemon-{name}"));
    let _ = std::fs::remove_dir_all(&r);
    std::fs::create_dir_all(&r).unwrap();
    r
}

fn suite(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/uc").join(file)
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr)
}

/// `strike <args>` with this home; `mode` is STRIKE_DAEMON.
fn strike_as(exe: &Path, home: &Path, mode: &str, args: &[&str]) -> Output {
    Command::new(exe).args(args).env("STRIKE_HOME", home).env("STRIKE_DAEMON", mode).env_remove("STRIKE_DAEMON_IDLE").output().unwrap()
}

fn strike(home: &Path, mode: &str, args: &[&str]) -> Output {
    strike_as(Path::new(STRIKE), home, mode, args)
}

/// Stops the home's daemon when dropped, also when the test fails.
struct Daemon {
    home: PathBuf,
}

impl Daemon {
    fn start(home: &Path, idle: u64) -> Daemon {
        let d = Daemon { home: home.to_path_buf() };
        let o = strike(home, "on", &["daemon", "start", "--idle", &idle.to_string()]);
        assert!(o.status.success() && text(&o).contains("started"), "{}", text(&o));
        d
    }
    fn status(&self) -> String {
        text(&strike(&self.home, "on", &["daemon", "status"]))
    }
    /// how many compiles the daemon has served
    fn compiles(&self) -> usize {
        let s = self.status();
        let at = s.find(" compiles;").unwrap_or_else(|| panic!("{s}"));
        s[..at].rsplit(' ').next().unwrap().parse().unwrap()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = strike(&self.home, "on", &["daemon", "stop"]);
    }
}

/// The report without its times: the same either way.
fn report(o: &Output) -> String {
    let t = String::from_utf8_lossy(&o.stdout).to_string();
    t.lines()
        .map(|l| match (l.find(" s (read"), l.rfind("; ")) {
            (Some(_), Some(_)) => {
                let head = &l[..l.find("unrolled;").map(|i| i + 9).unwrap_or(l.len())];
                let tail = &l[l.rfind('(').unwrap()..];
                format!("{head} {tail}")
            }
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn compile(home: &Path, mode: &str, src: &Path, exe: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["compile", src.to_str().unwrap(), "-o", exe.to_str().unwrap(), "--explain"];
    args.extend_from_slice(extra);
    strike(home, mode, &args)
}

#[test]
fn start_status_stop() {
    let r = root("life");
    let home = r.join("home");
    assert!(strike(&home, "on", &["daemon", "status"]).stdout.starts_with(b"strike daemon: not running"));
    let d = Daemon::start(&home, 60);
    let s = d.status();
    assert!(s.contains("running, pid") && s.contains("of 60 s"), "{s}");
    // a second start finds it
    assert!(text(&strike(&home, "on", &["daemon", "start"])).contains("already running"));
    let o = strike(&home, "on", &["daemon", "stop"]);
    assert!(text(&o).contains("stopped (pid"), "{}", text(&o));
    assert!(d.status().contains("not running"), "{}", d.status());
    assert!(!home.join("daemon/state.json").exists());
    assert!(text(&strike(&home, "on", &["daemon", "stop"])).contains("not running"));
}

/// Every mode, every language: the daemon writes the executable byte for
/// byte as Strike alone writes it, and says the same.
#[test]
fn a_compile_through_the_daemon_is_the_compile_without_it() {
    let r = root("same");
    let (home, alone) = (r.join("home"), r.join("alone"));
    let d = Daemon::start(&home, 60);
    let mut n = 0;
    for file in ["suite.c", "suite.go", "suite.py", "arrays.c", "arrays.py", "globals.go", "widths.c"] {
        for extra in [&[][..], &["--release"][..], &["--plain"][..], &["--release", "--no-evaluate"][..]] {
            // twice through the daemon: the second from its memory
            for again in 0..2 {
                let tag = format!("{file}{}{again}", extra.join(""));
                let (a, b) = (r.join(format!("{tag}-d.exe")), r.join(format!("{tag}-a.exe")));
                let od = compile(&home, "on", &suite(file), &a, extra);
                // a home of its own, whose brands grow as the daemon's do
                let oa = compile(&alone, "off", &suite(file), &b, extra);
                assert!(od.status.success() && oa.status.success(), "{}\n{}", text(&od), text(&oa));
                assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap(), "{tag}");
                assert_eq!(report(&od), report(&oa), "{tag}");
                n += 1;
            }
        }
    }
    assert_eq!(d.compiles(), n, "{}", d.status());
    let s = d.status();
    assert!(!s.contains(" 0 hits"), "the memory was used: {s}");
    // an error is the same error
    let bad = r.join("bad.c");
    std::fs::write(&bad, "int main() { return 1 +; }\n").unwrap();
    let od = compile(&home, "on", &bad, &r.join("bad.exe"), &[]);
    let oa = compile(&alone, "off", &bad, &r.join("bad.exe"), &[]);
    assert!(!od.status.success());
    assert_eq!(text(&od), text(&oa));
    assert_eq!(d.compiles(), n + 1);
}

/// A relative path is the client's, as without the daemon.
#[test]
fn a_relative_path_is_read_from_the_clients_folder() {
    let r = root("relative");
    let home = root("relative-home");
    std::fs::copy(suite("suite.go"), r.join("prog.go")).unwrap();
    let d = Daemon::start(&home, 60);
    let o = Command::new(STRIKE).args(["compile", "prog.go"]).current_dir(&r).env("STRIKE_HOME", &home).env("STRIKE_DAEMON", "on").output().unwrap();
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("strike: prog.go read as Go;"), "{}", text(&o));
    assert_eq!(d.compiles(), 1);
    let run = Command::new(r.join("prog.exe")).output().unwrap();
    assert_eq!(run.stdout, std::fs::read(suite("suite.go.out")).unwrap());
    // the daemon has left the client's folder: it can be removed while the daemon runs
    std::fs::remove_dir_all(&r).unwrap();
    assert!(d.status().contains("running, pid"));
}

/// A daemon from another Strike binary is never used; `start` replaces it.
#[test]
fn a_daemon_from_another_binary_is_not_used() {
    let r = root("stale");
    let home = r.join("home");
    // another binary: this one with bytes after its end, which Windows ignores
    let other = r.join("other-strike.exe");
    let mut bytes = std::fs::read(STRIKE).unwrap();
    bytes.extend_from_slice(b"another build");
    std::fs::write(&other, &bytes).unwrap();
    let _guard = Daemon { home: home.clone() };
    let o = strike_as(&other, &home, "on", &["daemon", "start", "--idle", "60"]);
    assert!(text(&o).contains("started"), "{}", text(&o));

    let s = text(&strike(&home, "on", &["daemon", "status"]));
    assert!(s.contains("stale"), "{s}");
    let (a, b) = (r.join("a.exe"), r.join("b.exe"));
    let o = compile(&home, "on", &suite("suite.c"), &a, &[]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(String::from_utf8_lossy(&o.stderr).contains("runs another Strike binary"), "{}", text(&o));
    let other_status = text(&strike_as(&other, &home, "on", &["daemon", "status"]));
    assert!(other_status.contains(" 0 compiles;"), "the stale daemon compiled: {other_status}");
    assert!(compile(&r.join("alone"), "off", &suite("suite.c"), &b, &[]).status.success());
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());

    let o = strike(&home, "on", &["daemon", "start", "--idle", "60"]);
    assert!(text(&o).contains("which ran another Strike binary") && text(&o).contains("started"), "{}", text(&o));
    let s = text(&strike(&home, "on", &["daemon", "status"]));
    assert!(s.contains("running, pid"), "{s}");
}

/// No daemon, a dead daemon's state, a pipe that cannot prove itself: the
/// build works alone every time and writes the same executable.
#[test]
fn without_a_daemon_that_answers_strike_works_alone() {
    let r = root("alone");
    let home = r.join("home");
    let want = {
        let o = compile(&r.join("ref"), "off", &suite("suite.py"), &r.join("ref.exe"), &[]);
        assert!(o.status.success());
        std::fs::read(r.join("ref.exe")).unwrap()
    };
    let check = |name: &str| {
        let exe = r.join(format!("{name}.exe"));
        let o = compile(&home, "on", &suite("suite.py"), &exe, &[]);
        assert!(o.status.success(), "{name}: {}", text(&o));
        assert_eq!(std::fs::read(&exe).unwrap(), want, "{name}");
    };
    check("none");

    // a daemon that died and left its state behind
    std::fs::create_dir_all(home.join("daemon")).unwrap();
    let dead = format!(
        r#"{{"pid":1,"pipe":"\\\\.\\pipe\\strike-daemon-dead{}","token":"{}","hash":"{}","origin":"{}","origin_len":0,"origin_mtime":0,"idle":60}}"#,
        std::process::id(),
        "00".repeat(32),
        blake3::hash(&std::fs::read(STRIKE).unwrap()).to_hex(),
        STRIKE.replace('\\', "\\\\")
    );
    std::fs::write(home.join("daemon/state.json"), &dead).unwrap();
    check("dead");
    assert!(text(&strike(&home, "on", &["daemon", "status"])).contains("not running"));
    assert!(text(&strike(&home, "on", &["daemon", "stop"])).contains("not running"));
    assert!(!home.join("daemon/state.json").exists(), "the dead daemon's state is removed");

    // a client that does not know the token gets no answer, and works alone
    let d = Daemon::start(&home, 60);
    let state = std::fs::read_to_string(home.join("daemon/state.json")).unwrap();
    let token = state.split("\"token\": \"").nth(1).unwrap()[..64].to_string();
    std::fs::write(home.join("daemon/state.json"), state.replace(&token, &"ab".repeat(32))).unwrap();
    check("wrong token");
    std::fs::write(home.join("daemon/state.json"), &state).unwrap();
    assert_eq!(d.compiles(), 0, "{}", d.status());
    check("right token");
    assert_eq!(d.compiles(), 1);
}

/// Several clients at once, in several languages.
#[test]
fn several_clients_at_once() {
    let r = root("many");
    let home = r.join("home");
    let d = Daemon::start(&home, 60);
    let files = ["suite.c", "suite.go", "suite.py", "arrays.c", "arrays.go", "arrays.py", "globals.c", "globals.py"];
    let threads: Vec<_> = files
        .iter()
        .flat_map(|f| [(f, 0), (f, 1)])
        .map(|(f, i)| {
            let (home, r) = (home.clone(), r.clone());
            let f = f.to_string();
            std::thread::spawn(move || {
                let exe = r.join(format!("{f}-{i}.exe"));
                let o = compile(&home, "on", &suite(&f), &exe, &["--release"]);
                assert!(o.status.success(), "{f}: {}", text(&o));
                let run = Command::new(&exe).output().unwrap();
                assert_eq!(run.stdout, std::fs::read(suite(&format!("{f}.out"))).unwrap(), "{f}");
                std::fs::read(&exe).unwrap()
            })
        })
        .collect();
    let exes: Vec<Vec<u8>> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    for (i, f) in files.iter().enumerate() {
        assert_eq!(exes[2 * i], exes[2 * i + 1], "{f}: the same file twice at once");
        let alone = r.join(format!("{f}-alone.exe"));
        assert!(compile(&r.join("alone"), "off", &suite(f), &alone, &["--release"]).status.success());
        assert_eq!(exes[2 * i], std::fs::read(&alone).unwrap(), "{f}");
    }
    assert_eq!(d.compiles(), 2 * files.len());
}

#[test]
fn the_daemon_ends_itself_when_idle() {
    let r = root("idle");
    let home = r.join("home");
    let d = Daemon::start(&home, 2);
    assert!(compile(&home, "on", &suite("suite.c"), &r.join("a.exe"), &[]).status.success());
    assert_eq!(d.compiles(), 1);
    let t = Instant::now();
    while home.join("daemon/state.json").exists() {
        assert!(t.elapsed() < Duration::from_secs(15), "still running: {}", d.status());
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(t.elapsed() >= Duration::from_secs(1), "it ended too soon");
    assert!(d.status().contains("not running"), "{}", d.status());
    // and compiling goes on alone
    assert!(compile(&home, "on", &suite("suite.c"), &r.join("b.exe"), &[]).status.success());
}

/// A watched folder is read ahead: a changed file's reading is ready
/// before it is compiled, and the executable is still the same.
#[test]
fn a_watched_folder_is_read_ahead() {
    let r = root("watch");
    let home = r.join("home");
    let src = r.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let prog = src.join("prog.c");
    std::fs::copy(suite("suite.c"), &prog).unwrap();
    let d = Daemon::start(&home, 60);
    let o = strike(&home, "on", &["daemon", "watch", src.to_str().unwrap()]);
    assert!(text(&o).contains("1 files read ahead"), "{}", text(&o));
    assert!(d.status().contains("1 readings, 0 evaluations, 0 hits"), "{}", d.status());
    assert!(compile(&home, "on", &prog, &r.join("a.exe"), &[]).status.success());
    assert!(d.status().contains("1 hits, 0 misses"), "{}", d.status());

    let mut changed = std::fs::read_to_string(&prog).unwrap();
    changed.push_str("// a change\n");
    std::fs::write(&prog, &changed).unwrap();
    let t = Instant::now();
    while !d.status().contains("2 readings") {
        assert!(t.elapsed() < Duration::from_secs(10), "{}", d.status());
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(compile(&home, "on", &prog, &r.join("b.exe"), &[]).status.success());
    assert!(d.status().contains("2 hits, 0 misses"), "{}", d.status());
    assert!(compile(&r.join("alone"), "off", &prog, &r.join("c.exe"), &[]).status.success());
    assert_eq!(std::fs::read(r.join("b.exe")).unwrap(), std::fs::read(r.join("c.exe")).unwrap());
    assert!(text(&strike(&home, "on", &["daemon", "unwatch", src.to_str().unwrap()])).contains("no longer watching"));
    assert!(d.status().contains("watching nothing"), "{}", d.status());
}

/// `strike build` asks the daemon for rustc's facts, and builds the same.
#[test]
fn a_build_takes_rustcs_facts_from_the_daemon() {
    let r = root("build");
    let home = r.join("home");
    let pkg = r.join("pkg");
    std::fs::create_dir_all(pkg.join("src")).unwrap();
    std::fs::write(pkg.join("Cargo.toml"), "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(pkg.join("Cargo.lock"), "version = 3\n\n[[package]]\nname = \"tiny\"\nversion = \"0.1.0\"\n").unwrap();
    std::fs::write(pkg.join("src/main.rs"), "fn main() { println!(\"tiny\"); }\n").unwrap();
    let alone = strike(&home, "on", &["build", pkg.to_str().unwrap()]);
    assert!(alone.status.success(), "{}", text(&alone));
    let d = Daemon::start(&home, 60);
    for _ in 0..2 {
        let o = strike(&home, "on", &["build", pkg.to_str().unwrap()]);
        assert!(o.status.success(), "{}", text(&o));
        assert!(text(&o).contains("struck Bin:tiny"), "{}", text(&o));
        assert_eq!(text(&o).lines().next(), text(&alone).lines().next(), "the same choice");
    }
    assert!(d.status().contains("2 rustc facts"), "{}", d.status());
    let run = Command::new(pkg.join("strike-out/build/tiny.exe")).output().unwrap();
    assert_eq!(run.stdout, b"tiny\n");
}

/// STRIKE_DAEMON=auto starts one when none runs; this compile works alone.
#[test]
fn auto_starts_a_daemon_for_the_next_command() {
    let r = root("auto");
    let home = r.join("home");
    let d = Daemon { home: home.clone() };
    let o = compile(&home, "auto", &suite("suite.go"), &r.join("a.exe"), &[]);
    assert!(o.status.success(), "{}", text(&o));
    let t = Instant::now();
    while !d.status().contains("running, pid") {
        assert!(t.elapsed() < Duration::from_secs(15), "{}", d.status());
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(compile(&home, "auto", &suite("suite.go"), &r.join("b.exe"), &[]).status.success());
    assert_eq!(d.compiles(), 1);
    assert_eq!(std::fs::read(r.join("a.exe")).unwrap(), std::fs::read(r.join("b.exe")).unwrap());
}
