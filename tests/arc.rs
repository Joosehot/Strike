//! ArcStrike between two devices on this machine: each its own STRIKE_HOME
//! (its own Nimbus, identity and approved peers), over 127.0.0.1.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

struct Pair {
    root: PathBuf,
    port: u16,
}

fn pair(name: &str, port: u16) -> Pair {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("arc-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("game/data")).unwrap();
    Pair { root, port }
}

/// Bytes that do not repeat, so every chunk is its own.
fn noise(seed: u32, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2654435761) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

impl Pair {
    fn game(&self) -> PathBuf {
        self.root.join("game")
    }

    fn received(&self) -> PathBuf {
        self.root.join("received")
    }

    /// The receiver, started and ready; `answer` goes to its pairing question.
    fn receiver(&self, answer: &str) -> (Child, BufReader<std::process::ChildStdout>) {
        let mut c = Command::new(env!("CARGO_BIN_EXE_strike"))
            .args(["arc", "receive", "--once", "--port", &self.port.to_string(), "--into"])
            .arg(self.received())
            .env("STRIKE_HOME", self.root.join("laptop"))
            .env("STRIKE_DEVICE", "laptop")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin.take().unwrap().write_all(answer.as_bytes()).unwrap();
        let mut out = BufReader::new(c.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        assert!(line.contains("receiving into"), "{line}");
        (c, out)
    }

    fn send(&self, extra: &[&str]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_strike"))
            .args(["arc", "send"])
            .arg(self.game())
            .args(["--to", &format!("127.0.0.1:{}", self.port)])
            .args(extra)
            .env("STRIKE_HOME", self.root.join("desktop"))
            .env("STRIKE_DEVICE", "desktop")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin.take().unwrap().write_all(b"y\n").unwrap();
        c.wait_with_output().unwrap()
    }

    /// Receiver and sender, once; returns what the sender said.
    fn transfer(&self, extra: &[&str]) -> String {
        let (mut r, mut out) = self.receiver("y\n");
        let s = self.send(extra);
        let text = String::from_utf8_lossy(&s.stdout).to_string();
        assert!(s.status.success(), "{text}\n{}", String::from_utf8_lossy(&s.stderr));
        let mut rest = String::new();
        std::io::Read::read_to_string(&mut out, &mut rest).unwrap();
        assert!(r.wait().unwrap().success(), "{rest}");
        assert!(rest.contains("all checked"), "{rest}");
        text
    }

    fn same(&self) {
        for f in files(&self.game()) {
            let rel = f.strip_prefix(self.root.as_path()).unwrap();
            let got = self.received().join(rel);
            assert_eq!(std::fs::read(&f).unwrap(), std::fs::read(&got).unwrap_or_default(), "{}", rel.display());
        }
    }
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        if e.path().is_dir() {
            out.extend(files(&e.path()));
        } else {
            out.push(e.path());
        }
    }
    out
}

/// "over the wire" in MB, from the sender's line.
fn wire_mb(text: &str) -> f64 {
    let at = text.find(" MB over the wire").unwrap();
    text[..at].rsplit(' ').next().unwrap().parse().unwrap()
}

#[test]
fn a_folder_arrives_whole_and_a_resend_sends_only_the_change() {
    let p = pair("resend", 47911);
    std::fs::write(p.game().join("data/a.pak"), noise(1, 6_000_000)).unwrap();
    std::fs::write(p.game().join("data/b.pak"), noise(2, 3_000_000)).unwrap();
    std::fs::write(p.game().join("readme.txt"), "hello").unwrap();
    std::fs::write(p.game().join("empty"), "").unwrap();
    let first = p.transfer(&[]);
    assert!(wire_mb(&first) >= 9.0, "{first}");
    p.same();

    // A few bytes put into the middle of one file.
    let mut a = noise(1, 6_000_000);
    a.splice(3_000_000..3_000_000, *b"patch");
    std::fs::write(p.game().join("data/a.pak"), &a).unwrap();
    let again = p.transfer(&[]);
    assert!(wire_mb(&again) < 2.5, "{again}");
    p.same();

    // Nothing changed: nothing goes over the wire.
    let none = p.transfer(&[]);
    assert_eq!(wire_mb(&none), 0.0, "{none}");
    p.same();
}

#[test]
fn a_cut_transfer_resumes_where_it_stopped() {
    let p = pair("resume", 47912);
    std::fs::write(p.game().join("data/big.pak"), noise(3, 12_000_000)).unwrap();
    let (mut r, _out) = p.receiver("y\n");
    let cut = p.send(&["--stop-after", "5000000", "--streams", "1"]);
    assert_eq!(cut.status.code(), Some(3), "{}", String::from_utf8_lossy(&cut.stderr));
    r.kill().ok();
    r.wait().ok();
    let rest = p.transfer(&["--streams", "1"]);
    assert!(wire_mb(&rest) < 8.0, "{rest}");
    p.same();
}

#[test]
fn a_device_not_approved_gets_nothing() {
    let p = pair("refuse", 47913);
    std::fs::write(p.game().join("secret.txt"), "only for friends").unwrap();
    let (mut r, _out) = p.receiver("n\n");
    let s = p.send(&[]);
    assert!(!s.status.success());
    assert!(String::from_utf8_lossy(&s.stderr).contains("not paired"), "{}", String::from_utf8_lossy(&s.stderr));
    r.kill().ok();
    r.wait().ok();
    assert!(!p.received().exists());
}
