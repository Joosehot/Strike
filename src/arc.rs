//! ArcStrike: big folders from one device to another on the same network,
//! never sending what the receiver already has.
//!
//! The sender cuts every file into content-defined chunks (FastCDC, 64 KiB
//! to 1 MiB, about 256 KiB) and names each by its BLAKE3 hash; the file as a
//! whole is named by its SHA-256, as in Nimbus. The receiver answers an offer
//! with the chunks it lacks: a file already in its Nimbus costs nothing, and a
//! changed file takes its unchanged chunks from the old version on its own
//! disk. Only the rest goes over the wire, encrypted (Noise XX, X25519,
//! ChaCha20-Poly1305), on several streams. Every chunk is checked by its hash
//! on arrival and every file by its SHA-256 before it enters Nimbus, then it
//! is struck into the target folder.
//!
//! Devices are approved once on both sides: both show the same six-digit
//! code, taken from the handshake, and both must accept. A device not
//! approved never sees a file. A cut transfer resumes: the receiver keeps
//! what came, and the same offer again asks only for the rest.

use crate::fstrike;
use crate::lightning::{hex, home};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const NOISE: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const DISCOVERY_PORT: u16 = 47821;
pub const DEFAULT_PORT: u16 = 47820;
const MIN_CHUNK: usize = 64 << 10;
const AVG_CHUNK: usize = 256 << 10;
const MAX_CHUNK: usize = 1 << 20;
/// A Noise message holds at most 65535 bytes, 16 of them the tag.
const PLAIN: usize = 65535 - 16;

fn arc_dir() -> PathBuf {
    home().join("arc")
}

type Hash32 = [u8; 32];

fn unhex(s: &str) -> Option<Hash32> {
    let mut out = [0u8; 32];
    if s.len() != 64 {
        return None;
    }
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

// ---- positional file access ----------------------------------------------

#[cfg(windows)]
fn read_at(f: &File, buf: &mut [u8], at: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < buf.len() {
        let n = f.seek_read(&mut buf[done..], at + done as u64)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        done += n;
    }
    Ok(())
}

#[cfg(windows)]
fn write_at(f: &File, buf: &[u8], at: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < buf.len() {
        done += f.seek_write(&buf[done..], at + done as u64)?;
    }
    Ok(())
}

#[cfg(unix)]
fn read_at(f: &File, buf: &mut [u8], at: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(f, buf, at)
}

#[cfg(unix)]
fn write_at(f: &File, buf: &[u8], at: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(f, buf, at)
}

// ---- the chunk index -----------------------------------------------------

/// Where each known chunk lies in a Nimbus file: BLAKE3 of the chunk ->
/// (SHA-256 of the file, offset, length). Kept as fixed records in
/// `nimbus/chunks.idx`, appended when a file enters Nimbus through ArcStrike.
struct ChunkIndex {
    map: HashMap<Hash32, (Hash32, u64, u32)>,
}

const RECORD: usize = 32 + 32 + 8 + 4;

impl ChunkIndex {
    fn path() -> PathBuf {
        fstrike::store_dir().join("chunks.idx")
    }

    fn load() -> ChunkIndex {
        let mut map = HashMap::new();
        let data = std::fs::read(Self::path()).unwrap_or_default();
        for r in data.chunks_exact(RECORD) {
            let c: Hash32 = r[..32].try_into().unwrap();
            let f: Hash32 = r[32..64].try_into().unwrap();
            let off = u64::from_le_bytes(r[64..72].try_into().unwrap());
            let len = u32::from_le_bytes(r[72..76].try_into().unwrap());
            map.insert(c, (f, off, len));
        }
        ChunkIndex { map }
    }

    fn add(&mut self, file: &Hash32, chunks: &[(Hash32, u32)]) -> Result<(), String> {
        let mut out = Vec::new();
        let mut off = 0u64;
        for (c, len) in chunks {
            if !self.map.contains_key(c) {
                self.map.insert(*c, (*file, off, *len));
                out.extend_from_slice(c);
                out.extend_from_slice(file);
                out.extend_from_slice(&off.to_le_bytes());
                out.extend_from_slice(&len.to_le_bytes());
            }
            off += *len as u64;
        }
        std::fs::create_dir_all(fstrike::store_dir()).map_err(|e| e.to_string())?;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(Self::path()).map_err(|e| e.to_string())?;
        f.write_all(&out).map_err(|e| e.to_string())
    }

    /// The chunk's bytes from Nimbus, if a stored file still holds them.
    fn read(&self, c: &Hash32) -> Option<Vec<u8>> {
        let (file, off, len) = self.map.get(c)?;
        let f = File::open(fstrike::blob(&hex(file))).ok()?;
        let mut buf = vec![0u8; *len as usize];
        read_at(&f, &mut buf, *off).ok()?;
        (blake3::hash(&buf).as_bytes() == c).then_some(buf)
    }
}

// ---- the offer -------------------------------------------------------------

#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Relative to the receiver's folder, `/` between parts.
    pub path: String,
    pub size: u64,
    pub sha256: String,
    /// (BLAKE3, length) of each chunk, in order.
    pub chunks: Vec<(String, u32)>,
}

#[derive(Serialize, Deserialize)]
struct Offer {
    from: String,
    entries: Vec<Entry>,
}

/// A scanned file, remembered by (size, time) so a resend does not read it.
#[derive(Clone, Serialize, Deserialize)]
struct Scanned {
    size: u64,
    modified: u128,
    sha256: String,
    chunks: Vec<(String, u32)>,
}

fn scan_cache_path() -> PathBuf {
    arc_dir().join("scanned.json")
}

fn modified_ns(m: &std::fs::Metadata) -> u128 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0)
}

/// Cuts one file into chunks; returns its SHA-256 and the chunks.
fn scan_file(path: &Path) -> Result<(String, Vec<(String, u32)>), String> {
    let f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut sha = Sha256::new();
    let mut chunks = Vec::new();
    for c in fastcdc::v2020::StreamCDC::new(std::io::BufReader::with_capacity(4 << 20, f), MIN_CHUNK, AVG_CHUNK, MAX_CHUNK) {
        let c = c.map_err(|e| format!("{}: {e}", path.display()))?;
        sha.update(&c.data);
        chunks.push((blake3::hash(&c.data).to_hex().to_string(), c.length as u32));
    }
    Ok((hex(&sha.finalize()), chunks))
}

/// Every file under `root` (or `root` itself), as entries named from
/// `root`'s own name, and where each lies on this disk.
fn scan(root: &Path, jobs: usize) -> Result<(Vec<Entry>, Vec<PathBuf>), String> {
    let root = std::fs::canonicalize(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let base = root.parent().map(Path::to_path_buf).unwrap_or_default();
    let files = if root.is_dir() { crate::lightning::files_under(&root) } else { vec![root.clone()] };
    let cache: BTreeMap<String, Scanned> =
        std::fs::read_to_string(scan_cache_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let cache = Mutex::new(cache);
    let next = AtomicU64::new(0);
    let out: Mutex<Vec<Option<Result<Entry, String>>>> = Mutex::new(vec![None; files.len()]);
    std::thread::scope(|s| {
        for _ in 0..jobs.max(1) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed) as usize;
                let Some(path) = files.get(i) else { break };
                let r = (|| {
                    let m = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
                    let key = path.display().to_string();
                    let known = cache.lock().unwrap().get(&key).cloned();
                    let s = match known {
                        Some(s) if s.size == m.len() && s.modified == modified_ns(&m) => s,
                        _ => {
                            let (sha256, chunks) = scan_file(path)?;
                            let s = Scanned { size: m.len(), modified: modified_ns(&m), sha256, chunks };
                            cache.lock().unwrap().insert(key, s.clone());
                            s
                        }
                    };
                    let rel = path.strip_prefix(&base).unwrap_or(path).to_string_lossy().replace('\\', "/");
                    Ok(Entry { path: rel, size: s.size, sha256: s.sha256, chunks: s.chunks })
                })();
                out.lock().unwrap()[i] = Some(r);
            });
        }
    });
    std::fs::create_dir_all(arc_dir()).map_err(|e| e.to_string())?;
    let text = serde_json::to_string(&*cache.lock().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(scan_cache_path(), text).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for r in out.into_inner().unwrap() {
        entries.push(r.unwrap()?);
    }
    Ok((entries, files))
}

// ---- identity, peers, the encrypted channel ---------------------------------

fn device_name() -> String {
    std::env::var("STRIKE_DEVICE")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "device".into())
}

fn identity() -> Result<Vec<u8>, String> {
    let path = arc_dir().join("identity.key");
    if let Ok(k) = std::fs::read(&path) {
        if k.len() == 32 {
            return Ok(k);
        }
    }
    let kp = snow::Builder::new(NOISE.parse().unwrap()).generate_keypair().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(arc_dir()).map_err(|e| e.to_string())?;
    std::fs::write(&path, &kp.private).map_err(|e| e.to_string())?;
    Ok(kp.private)
}

fn peers_path() -> PathBuf {
    arc_dir().join("peers.json")
}

fn peers() -> BTreeMap<String, String> {
    std::fs::read_to_string(peers_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

static PROMPT: Mutex<()> = Mutex::new(());

/// One TCP stream, encrypted. A message is a kind byte and a body of any
/// length, carried in as many Noise messages as it needs.
struct Chan {
    s: TcpStream,
    t: snow::TransportState,
    peer: String,
    buf: Vec<u8>,
}

const K_OFFER: u8 = 1;
const K_NEED: u8 = 2;
const K_CHUNK: u8 = 3;
const K_FINISH: u8 = 4;
const K_RESULT: u8 = 5;
const K_JOIN: u8 = 6;
const K_APPROVE: u8 = 7;

fn frame_write(s: &mut TcpStream, msg: &[u8]) -> std::io::Result<()> {
    s.write_all(&(msg.len() as u16).to_le_bytes())?;
    s.write_all(msg)
}

fn frame_read(s: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut n = [0u8; 2];
    s.read_exact(&mut n)?;
    let mut m = vec![0u8; u16::from_le_bytes(n) as usize];
    s.read_exact(&mut m)?;
    Ok(m)
}

impl Chan {
    /// The Noise XX handshake, then approval: a device seen before is
    /// approved; a new one is approved only when the person on this side
    /// accepts the code both sides show, and the other side does the same.
    fn open(mut s: TcpStream, initiator: bool) -> Result<Chan, String> {
        s.set_nodelay(true).ok();
        let key = identity()?;
        let b = snow::Builder::new(NOISE.parse().unwrap()).local_private_key(&key).map_err(|e| e.to_string())?;
        let mut hs = if initiator { b.build_initiator() } else { b.build_responder() }.map_err(|e| e.to_string())?;
        let me = format!("{}\t{}", device_name(), std::env::consts::OS);
        let mut buf = vec![0u8; 65535];
        let them: String;
        let io = |e: std::io::Error| format!("handshake: {e}");
        let ns = |e: snow::Error| format!("handshake: {e}");
        if initiator {
            let n = hs.write_message(&[], &mut buf).map_err(ns)?;
            frame_write(&mut s, &buf[..n]).map_err(io)?;
            let m = frame_read(&mut s).map_err(io)?;
            let n = hs.read_message(&m, &mut buf).map_err(ns)?;
            them = String::from_utf8_lossy(&buf[..n]).to_string();
            let n = hs.write_message(me.as_bytes(), &mut buf).map_err(ns)?;
            frame_write(&mut s, &buf[..n]).map_err(io)?;
        } else {
            let m = frame_read(&mut s).map_err(io)?;
            hs.read_message(&m, &mut buf).map_err(ns)?;
            let n = hs.write_message(me.as_bytes(), &mut buf).map_err(ns)?;
            frame_write(&mut s, &buf[..n]).map_err(io)?;
            let m = frame_read(&mut s).map_err(io)?;
            let n = hs.read_message(&m, &mut buf).map_err(ns)?;
            them = String::from_utf8_lossy(&buf[..n]).to_string();
        }
        let remote = hex(hs.get_remote_static().ok_or("handshake: no remote key")?);
        let h = hs.get_handshake_hash();
        let code = u32::from_le_bytes(h[..4].try_into().unwrap()) % 1_000_000;
        let t = hs.into_transport_mode().map_err(ns)?;
        let (name, os) = them.split_once('\t').unwrap_or((&them, "?"));
        let mut c = Chan { s, t, peer: name.to_string(), buf: vec![0u8; 65535] };

        let known = peers().contains_key(&remote);
        let ok = known || {
            let _g = PROMPT.lock().unwrap();
            print!("Pair with {name} ({os})? Code {:03} {:03}. Accept with y or the code: ", code / 1000, code % 1000);
            std::io::stdout().flush().ok();
            let mut line = String::new();
            match std::io::stdin().lock().read_line(&mut line) {
                Ok(0) | Err(_) => {
                    println!();
                    eprintln!("arc: no answer could be read here (no keyboard input); run strike in a terminal window");
                    false
                }
                Ok(_) => {
                    let a = line.trim().to_lowercase();
                    let ok = matches!(a.as_str(), "y" | "yes" | "k" | "kyllä" | "kylla") || a.replace(' ', "") == format!("{code:06}");
                    if !ok {
                        eprintln!("arc: declined (read {a:?}); accept with y or the code");
                    }
                    ok
                }
            }
        };
        c.send(K_APPROVE, &[ok as u8])?;
        let (k, body) = c.recv()?;
        let theirs = k == K_APPROVE && body.first() == Some(&1);
        if !ok || !theirs {
            return Err(format!("{name}: not paired ({})", if ok { "the other side declined" } else { "declined here" }));
        }
        if !known {
            let mut p = peers();
            p.insert(remote, name.to_string());
            std::fs::create_dir_all(arc_dir()).map_err(|e| e.to_string())?;
            std::fs::write(peers_path(), serde_json::to_string_pretty(&p).unwrap()).map_err(|e| e.to_string())?;
            println!("paired with {name}");
        }
        Ok(c)
    }

    fn send(&mut self, kind: u8, body: &[u8]) -> Result<(), String> {
        let mut head = vec![kind];
        head.extend_from_slice(&(body.len() as u64).to_le_bytes());
        let mut out = Vec::with_capacity(body.len() + body.len() / PLAIN * 20 + 64);
        let mut piece = Vec::with_capacity(PLAIN);
        let mut rest = body;
        piece.extend_from_slice(&head);
        loop {
            let take = (PLAIN - piece.len()).min(rest.len());
            piece.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            let n = self.t.write_message(&piece, &mut self.buf).map_err(|e| e.to_string())?;
            out.extend_from_slice(&(n as u16).to_le_bytes());
            out.extend_from_slice(&self.buf[..n]);
            piece.clear();
            if rest.is_empty() {
                break;
            }
        }
        self.s.write_all(&out).map_err(|e| format!("{}: {e}", self.peer))
    }

    fn recv(&mut self) -> Result<(u8, Vec<u8>), String> {
        let err = |e: std::io::Error| e.to_string();
        let mut first = self.piece().map_err(err)?;
        if first.len() < 9 {
            return Err("a message cut short".into());
        }
        let kind = first[0];
        let len = u64::from_le_bytes(first[1..9].try_into().unwrap()) as usize;
        let mut body = first.split_off(9);
        while body.len() < len {
            body.extend(self.piece().map_err(err)?);
        }
        Ok((kind, body))
    }

    fn piece(&mut self) -> std::io::Result<Vec<u8>> {
        let m = frame_read(&mut self.s)?;
        let n = self.t.read_message(&m, &mut self.buf).map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(self.buf[..n].to_vec())
    }
}

// ---- discovery -----------------------------------------------------------

/// Devices that announce themselves: name -> (OS, address).
pub fn devices(wait: Duration) -> Result<BTreeMap<String, (String, String)>, String> {
    let sock = UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)).map_err(|e| format!("discovery port {DISCOVERY_PORT}: {e}"))?;
    sock.set_read_timeout(Some(Duration::from_millis(200))).ok();
    let end = Instant::now() + wait;
    let mut out = BTreeMap::new();
    let mut buf = [0u8; 512];
    while Instant::now() < end {
        if let Ok((n, from)) = sock.recv_from(&mut buf) {
            let text = String::from_utf8_lossy(&buf[..n]);
            let mut f = text.split('\t');
            if f.next() == Some("ARCSTRIKE1") {
                if let (Some(name), Some(os), Some(port)) = (f.next(), f.next(), f.next()) {
                    out.insert(name.to_string(), (os.to_string(), format!("{}:{port}", from.ip())));
                }
            }
        }
    }
    Ok(out)
}

fn announce(port: u16, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let Ok(sock) = UdpSocket::bind(("0.0.0.0", 0)) else { return };
        sock.set_broadcast(true).ok();
        let msg = format!("ARCSTRIKE1\t{}\t{}\t{port}", device_name(), std::env::consts::OS);
        while !stop.load(Ordering::Relaxed) {
            let _ = sock.send_to(msg.as_bytes(), ("255.255.255.255", DISCOVERY_PORT));
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

// ---- sending ---------------------------------------------------------------

pub struct Sent {
    pub files: usize,
    pub bytes: u64,
    pub sent_bytes: u64,
    pub chunks_sent: usize,
    pub scan_seconds: f64,
    pub seconds: f64,
    pub peer: String,
}

/// Sends `root` to the device at `to` (a name it announces, or host:port).
/// With `stop_after`, stops after that many chunk bytes, as a cut line would.
pub fn send(root: &Path, to: &str, streams: usize, stop_after: Option<u64>) -> Result<Sent, String> {
    let start = Instant::now();
    let addr = if to.contains(':') {
        to.to_string()
    } else {
        let found = devices(Duration::from_secs(4))?;
        found.get(to).map(|d| d.1.clone()).ok_or(format!("{to}: not found on the network (strike arc devices)"))?
    };
    let jobs = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let (entries, paths) = scan(root, jobs)?;
    let scan_seconds = start.elapsed().as_secs_f64();
    // Where each chunk lies here.
    let mut at: HashMap<String, (usize, u64, u32)> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        let mut off = 0;
        for (c, len) in &e.chunks {
            at.entry(c.clone()).or_insert((i, off, *len));
            off += *len as u64;
        }
    }
    let connect = || TcpStream::connect(&addr).map_err(|e| format!("{addr}: {e}"));
    let mut ctl = Chan::open(connect()?, true)?;
    let offer = Offer { from: device_name(), entries: entries.clone() };
    let body = serde_json::to_vec(&offer).map_err(|e| e.to_string())?;
    let id = hex(&Sha256::digest(&body));
    ctl.send(K_OFFER, &body)?;
    let (k, need) = ctl.recv()?;
    if k != K_NEED {
        return Err(format!("{}: {}", ctl.peer, String::from_utf8_lossy(&need)));
    }
    let need: Vec<String> = serde_json::from_slice(&need).map_err(|e| e.to_string())?;

    // The chunks go round the streams; the control stream carries its share.
    let streams = streams.clamp(1, 16).min(need.len().max(1));
    let shares: Vec<Vec<&String>> = (0..streams).map(|s| need.iter().skip(s).step_by(streams).collect()).collect();
    let sent = AtomicU64::new(0);
    let push = |c: &mut Chan, share: &[&String]| -> Result<(), String> {
        let mut open: HashMap<usize, File> = HashMap::new();
        for h in share {
            let &(i, off, len) = at.get(*h).ok_or("asked for a chunk not offered")?;
            if !open.contains_key(&i) {
                open.insert(i, File::open(&paths[i]).map_err(|e| format!("{}: {e}", paths[i].display()))?);
            }
            let mut body = vec![0u8; 32 + len as usize];
            body[..32].copy_from_slice(&unhex(h).unwrap());
            read_at(&open[&i], &mut body[32..], off).map_err(|e| format!("{}: {e}", paths[i].display()))?;
            if blake3::hash(&body[32..]).to_hex().as_str() != h.as_str() {
                return Err(format!("{} changed while it was sent", paths[i].display()));
            }
            c.send(K_CHUNK, &body)?;
            let total = sent.fetch_add(len as u64, Ordering::Relaxed) + len as u64;
            if stop_after.is_some_and(|s| total >= s) {
                eprintln!("stopped after {total} bytes, as asked");
                std::process::exit(3);
            }
        }
        Ok(())
    };
    std::thread::scope(|s| -> Result<(), String> {
        let mut handles = Vec::new();
        for share in shares.iter().skip(1) {
            let push = &push;
            let connect = &connect;
            let id = &id;
            handles.push(s.spawn(move || -> Result<(), String> {
                let mut c = Chan::open(connect()?, true)?;
                c.send(K_JOIN, id.as_bytes())?;
                push(&mut c, share)?;
                c.send(K_FINISH, &[])?;
                // Wait for the receiver to take it all before closing.
                c.recv().map(|_| ())
            }));
        }
        let mine = push(&mut ctl, &shares[0]);
        for h in handles {
            h.join().map_err(|_| "a stream panicked".to_string())??;
        }
        mine
    })?;
    ctl.send(K_FINISH, &[])?;
    let (k, result) = ctl.recv()?;
    let text = String::from_utf8_lossy(&result).to_string();
    if k != K_RESULT || !text.starts_with("ok") {
        return Err(format!("{}: {text}", ctl.peer));
    }
    Ok(Sent {
        files: entries.len(),
        bytes: entries.iter().map(|e| e.size).sum(),
        sent_bytes: sent.load(Ordering::Relaxed),
        chunks_sent: need.len(),
        scan_seconds,
        seconds: start.elapsed().as_secs_f64(),
        peer: ctl.peer.clone(),
    })
}

// ---- receiving -------------------------------------------------------------

/// One offer being received: its files under `arc/incoming/<id>/`, the
/// chunks still missing, and the log of chunks that came (for a resume).
struct Incoming {
    dir: PathBuf,
    /// Chunk -> every (file, offset) it goes to.
    places: HashMap<Hash32, Vec<(usize, u64)>>,
    missing: Mutex<HashSet<Hash32>>,
    arrived: Condvar,
    log: Mutex<File>,
    streams: AtomicU64,
}

impl Incoming {
    fn take(&self, files: &mut HashMap<usize, File>, body: &[u8]) -> Result<(), String> {
        if body.len() < 32 {
            return Err("a chunk cut short".into());
        }
        let h: Hash32 = body[..32].try_into().unwrap();
        let data = &body[32..];
        if blake3::hash(data).as_bytes() != &h {
            return Err("a chunk does not match its hash; refused".into());
        }
        let places = self.places.get(&h).ok_or("a chunk not asked for")?;
        for &(i, off) in places {
            if !files.contains_key(&i) {
                let f = std::fs::OpenOptions::new().write(true).open(self.dir.join(format!("f{i}"))).map_err(|e| e.to_string())?;
                files.insert(i, f);
            }
            write_at(&files[&i], data, off).map_err(|e| e.to_string())?;
        }
        self.log.lock().unwrap().write_all(&h).map_err(|e| e.to_string())?;
        let mut m = self.missing.lock().unwrap();
        m.remove(&h);
        if m.is_empty() {
            self.arrived.notify_all();
        }
        Ok(())
    }
}

type Table = Arc<Mutex<HashMap<String, Arc<Incoming>>>>;

fn safe(path: &str) -> bool {
    let p = Path::new(path);
    !path.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// Answers an offer: which chunks are missing. Prepares every file not yet
/// in Nimbus, with its chunks known here already written into it.
fn prepare(id: &str, offer: &Offer, index: &ChunkIndex) -> Result<(Arc<Incoming>, Vec<String>), String> {
    let dir = arc_dir().join("incoming").join(&id[..16]);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let got: HashSet<Hash32> = std::fs::read(dir.join("got.log"))
        .unwrap_or_default()
        .chunks_exact(32)
        .map(|c| c.try_into().unwrap())
        .collect();
    let mut places: HashMap<Hash32, Vec<(usize, u64)>> = HashMap::new();
    let mut missing = HashSet::new();
    for (i, e) in offer.entries.iter().enumerate() {
        if !safe(&e.path) {
            return Err(format!("unsafe path {}", e.path));
        }
        if fstrike::blob(&e.sha256).exists() {
            continue;
        }
        let path = dir.join(format!("f{i}"));
        let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path).map_err(|e| e.to_string())?;
        f.set_len(e.size).map_err(|e| e.to_string())?;
        let mut off = 0;
        for (c, len) in &e.chunks {
            let h = unhex(c).ok_or("a bad chunk name")?;
            if !got.contains(&h) {
                match index.read(&h) {
                    Some(data) => write_at(&f, &data, off).map_err(|e| e.to_string())?,
                    None => {
                        missing.insert(h);
                        places.entry(h).or_default().push((i, off));
                    }
                }
            }
            off += *len as u64;
        }
    }
    let need: Vec<String> = missing.iter().map(|h| hex(h)).collect();
    let log = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("got.log")).map_err(|e| e.to_string())?;
    let inc = Incoming { dir, places, missing: Mutex::new(missing), arrived: Condvar::new(), log: Mutex::new(log), streams: AtomicU64::new(0) };
    Ok((Arc::new(inc), need))
}

/// Every file checked by its SHA-256, put in Nimbus, then struck into `into`.
fn finish(inc: &Incoming, offer: &Offer, into: &Path, index: &mut ChunkIndex) -> Result<usize, String> {
    for (i, e) in offer.entries.iter().enumerate() {
        if !fstrike::blob(&e.sha256).exists() {
            let tmp = inc.dir.join(format!("f{i}"));
            let sha = fstrike::hash_file(&tmp)?;
            if sha != e.sha256 {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("{}: SHA-256 {sha}, the sender's {}; not kept", e.path, e.sha256));
            }
            fstrike::keep(&tmp, &e.sha256)?;
        }
        let chunks: Vec<(Hash32, u32)> = e.chunks.iter().filter_map(|(c, l)| Some((unhex(c)?, *l))).collect();
        index.add(&unhex(&e.sha256).ok_or("bad file hash")?, &chunks)?;
        fstrike::strike_to(&e.sha256, &into.join(&e.path))?;
        // The struck file is checked once more against the sender's hash.
        if fstrike::hash_file(&into.join(&e.path))? != e.sha256 {
            return Err(format!("{}: not the sender's content after the strike", e.path));
        }
    }
    let _ = std::fs::remove_dir_all(&inc.dir);
    Ok(offer.entries.len())
}

/// Receives into `into` until stopped (or after one transfer with `once`).
pub fn receive(into: &Path, port: u16, once: bool) -> Result<(), String> {
    let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| format!("port {port}: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let stop = Arc::new(AtomicBool::new(false));
    announce(port, stop.clone());
    println!("{} receiving into {} on port {port}", device_name(), into.display());
    std::io::stdout().flush().ok();
    let table: Table = Arc::new(Mutex::new(HashMap::new()));
    let index = Arc::new(Mutex::new(ChunkIndex::load()));
    for conn in listener.incoming() {
        let Ok(conn) = conn else { continue };
        let (table, index, into) = (table.clone(), index.clone(), into.to_path_buf());
        std::thread::spawn(move || {
            match serve(conn, &table, &index, &into) {
                Ok(Some(line)) => {
                    println!("{line}");
                    std::io::stdout().flush().ok();
                    if once {
                        std::process::exit(0);
                    }
                }
                Ok(None) => {}
                Err(e) => eprintln!("arc: {e}"),
            }
        });
    }
    stop.store(true, Ordering::Relaxed);
    Ok(())
}

/// One connection: a control stream (offer, chunks, finish) or a data
/// stream (join, chunks, finish). Returns a line to print when a transfer ends.
fn serve(conn: TcpStream, table: &Table, index: &Arc<Mutex<ChunkIndex>>, into: &Path) -> Result<Option<String>, String> {
    let mut c = Chan::open(conn, false)?;
    let (k, body) = c.recv()?;
    let mut files = HashMap::new();
    match k {
        K_JOIN => {
            let id = String::from_utf8_lossy(&body).to_string();
            let inc = table.lock().unwrap().get(&id).cloned().ok_or("join: no such transfer")?;
            inc.streams.fetch_add(1, Ordering::Relaxed);
            let r = (|| loop {
                let (k, body) = c.recv()?;
                match k {
                    K_CHUNK => inc.take(&mut files, &body)?,
                    K_FINISH => return c.send(K_RESULT, b"ok"),
                    _ => return Err("unexpected message".to_string()),
                }
            })();
            inc.streams.fetch_sub(1, Ordering::Relaxed);
            inc.arrived.notify_all();
            r.map(|_| None)
        }
        K_OFFER => {
            let start = Instant::now();
            let id = hex(&Sha256::digest(&body));
            let offer: Offer = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
            let (inc, need) = {
                let idx = index.lock().unwrap();
                prepare(&id, &offer, &idx)?
            };
            table.lock().unwrap().insert(id.clone(), inc.clone());
            c.send(K_NEED, &serde_json::to_vec(&need).unwrap())?;
            loop {
                let (k, body) = c.recv()?;
                match k {
                    K_CHUNK => inc.take(&mut files, &body)?,
                    K_FINISH => break,
                    _ => return Err("unexpected message".into()),
                }
            }
            // The other streams finished before this one did; wait for the
            // last chunks they carried to be written.
            let mut m = inc.missing.lock().unwrap();
            while !m.is_empty() && inc.streams.load(Ordering::Relaxed) > 0 {
                m = inc.arrived.wait_timeout(m, Duration::from_millis(100)).unwrap().0;
            }
            let left = m.len();
            drop(m);
            drop(files);
            if left > 0 {
                let msg = format!("{left} chunks did not come; send again to resume");
                c.send(K_RESULT, msg.as_bytes())?;
                return Err(msg);
            }
            let n = finish(&inc, &offer, into, &mut index.lock().unwrap());
            table.lock().unwrap().remove(&id);
            match n {
                Ok(n) => {
                    c.send(K_RESULT, format!("ok {n}").as_bytes())?;
                    let bytes: u64 = offer.entries.iter().map(|e| e.size).sum();
                    Ok(Some(format!(
                        "received {n} files ({:.1} MB) from {}, {} chunks over the wire, all checked; {:.2} s",
                        bytes as f64 / 1e6,
                        offer.from,
                        need.len(),
                        start.elapsed().as_secs_f64()
                    )))
                }
                Err(e) => {
                    c.send(K_RESULT, e.as_bytes())?;
                    Err(e)
                }
            }
        }
        _ => Err("unexpected first message".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_in_the_middle_keeps_most_chunks() {
        let dir = std::env::temp_dir().join(format!("arc-cdc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut x = 0x9e3779b9u32;
        let mut data: Vec<u8> = (0..8_000_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        std::fs::write(dir.join("a"), &data).unwrap();
        data.splice(4_000_000..4_000_000, b"inserted bytes".iter().copied());
        std::fs::write(dir.join("b"), &data).unwrap();
        let (_, a) = scan_file(&dir.join("a")).unwrap();
        let (_, b) = scan_file(&dir.join("b")).unwrap();
        let old: HashSet<_> = a.iter().map(|c| &c.0).collect();
        let new = b.iter().filter(|c| !old.contains(&c.0)).count();
        assert!(new <= 3, "{new} of {} chunks new", b.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_plain_relative_paths_are_safe() {
        assert!(safe("Game/data/a.pak"));
        assert!(!safe("../x"));
        assert!(!safe("C:/Windows/x"));
        assert!(!safe("/etc/passwd"));
        assert!(!safe(""));
    }
}
