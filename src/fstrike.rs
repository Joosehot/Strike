//! fStrike (file strike): every downloaded file is kept once, in Nimbus (a
//! store named by the hash of its content), and struck into place as a hard
//! link.
//!
//! A file whose hash is known before it is needed (a lockfile's checksum)
//! never touches the network once it is in the store. A plain URL is
//! remembered with the server's version tag (ETag, Last-Modified), so the
//! next fetch only asks whether it changed. Stored files are read-only: a
//! hard link is the same file under a second name, and one edit would change
//! every place it was struck to.

use crate::lightning::{hex, home};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Nimbus, the store: `nimbus/<first two of the hash>/<hash>`.
pub fn store_dir() -> PathBuf {
    home().join("nimbus")
}

/// Where packages are struck to, one folder per package and version.
pub fn registry_dir() -> PathBuf {
    home().join("registry")
}

fn blob(hash: &str) -> PathBuf {
    store_dir().join(&hash[..2]).join(hash)
}

/// A file name in the store's own temporary folder, unique in this run.
fn temp_file() -> Result<PathBuf, String> {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = store_dir().join("tmp");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir.join(format!("{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))))
}

/// How a file came to be in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// From the store, without the network.
    Struck,
    /// The server said it had not changed; then from the store.
    NotModified,
    /// Downloaded (this many bytes) into the store, then struck.
    Fetched(u64),
}

/// Moves a finished file into the store under its hash. A file already
/// there (the same content) wins and the new one is dropped.
fn keep(tmp: &Path, hash: &str) -> Result<(), String> {
    let to = blob(hash);
    if to.exists() {
        let _ = std::fs::remove_file(tmp);
        return Ok(());
    }
    std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
    if std::fs::rename(tmp, &to).is_err() {
        // Another thread put the same content there first.
        let _ = std::fs::remove_file(tmp);
        if !to.exists() {
            return Err(format!("{}: could not be stored", to.display()));
        }
    }
    read_only(&to, true);
    Ok(())
}

fn read_only(path: &Path, on: bool) {
    if let Ok(m) = std::fs::metadata(path) {
        let mut p = m.permissions();
        if p.readonly() != on {
            #[allow(clippy::permissions_set_readonly_false)]
            p.set_readonly(on);
            let _ = std::fs::set_permissions(path, p);
        }
    }
}

/// Puts these bytes in the store; returns their hash.
pub fn put_bytes(bytes: &[u8]) -> Result<String, String> {
    let hash = hex(&Sha256::digest(bytes));
    if blob(&hash).exists() {
        // Removing a struck file with force (Windows) takes the read-only
        // mark off the stored file too; it goes back on when it is used.
        read_only(&blob(&hash), true);
    } else {
        let tmp = temp_file()?;
        std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
        keep(&tmp, &hash)?;
    }
    Ok(hash)
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// The strike: makes `dest` the stored file `hash`, as a hard link (a copy
/// only where a link cannot be, as onto another drive).
pub fn strike_to(hash: &str, dest: &Path) -> Result<(), String> {
    let from = blob(hash);
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    if let (Ok(a), Ok(b)) = (std::fs::metadata(&from), std::fs::metadata(dest)) {
        // Links of one file share its size and time: already in place.
        if a.len() == b.len() && a.modified().ok() == b.modified().ok() {
            return Ok(());
        }
        // A read-only file cannot be removed on Windows; if it is a link of
        // another stored file, that file gets its read-only mark back.
        read_only(dest, false);
        std::fs::remove_file(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        restore_read_only();
    }
    read_only(&from, true);
    if std::fs::hard_link(&from, dest).is_err() {
        std::fs::copy(&from, dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    }
    Ok(())
}

/// Hard links share the read-only mark, so taking it off a struck file took
/// it off its stored file too. Puts it back on every stored file that lost it.
fn restore_read_only() {
    for d in std::fs::read_dir(store_dir()).into_iter().flatten().flatten() {
        if d.file_name() == "tmp" {
            continue;
        }
        for f in std::fs::read_dir(d.path()).into_iter().flatten().flatten() {
            read_only(&f.path(), true);
        }
    }
}

// ---- URLs ----------------------------------------------------------------

/// What a URL gave last time.
#[derive(Clone, Serialize, Deserialize)]
struct Seen {
    sha256: String,
    etag: Option<String>,
    modified: Option<String>,
}

static URLS: Mutex<()> = Mutex::new(());

fn urls_file() -> PathBuf {
    home().join("urls.json")
}

fn read_urls() -> BTreeMap<String, Seen> {
    std::fs::read_to_string(urls_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Downloads `url` into `out` with curl. With `seen`, the server is asked to
/// send the file only if it changed. Returns the HTTP status and the version
/// tags of the last response.
fn curl(url: &str, out: &Path, seen: Option<&Seen>) -> Result<(u16, Option<String>, Option<String>), String> {
    let headers = out.with_extension("headers");
    let mut c = Command::new("curl");
    c.args(["-sSL", "--fail", "--retry", "2", "-o"]).arg(out).arg("-D").arg(&headers).args(["-w", "%{http_code}"]);
    if let Some(s) = seen {
        if let Some(e) = &s.etag {
            c.arg("-H").arg(format!("If-None-Match: {e}"));
        }
        if let Some(m) = &s.modified {
            c.arg("-H").arg(format!("If-Modified-Since: {m}"));
        }
    }
    let o = c.arg(url).output().map_err(|e| format!("curl: {e}"))?;
    let text = std::fs::read_to_string(&headers).unwrap_or_default();
    let _ = std::fs::remove_file(&headers);
    if !o.status.success() {
        let _ = std::fs::remove_file(out);
        return Err(format!("{url}: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    let code: u16 = String::from_utf8_lossy(&o.stdout).trim().parse().unwrap_or(0);
    // After redirects the headers hold one block per response; the last counts.
    let last = text.rsplit("HTTP/").next().unwrap_or("");
    let header = |name: &str| {
        last.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    };
    Ok((code, header("etag"), header("last-modified")))
}

/// Fetches `url` into the store. With `want`, the content must have that
/// hash, and a stored file with it is struck without the network. Returns
/// the hash and how it came.
pub fn fetch_url(url: &str, want: Option<&str>) -> Result<(String, How), String> {
    if let Some(w) = want {
        if blob(w).exists() {
            return Ok((w.to_string(), How::Struck));
        }
        // A known hash the store does not hold: no question to ask.
        return download(url, want, None);
    }
    let seen = {
        let _g = URLS.lock().unwrap();
        read_urls().get(url).cloned().filter(|s| blob(&s.sha256).exists())
    };
    download(url, None, seen.as_ref())
}

fn download(url: &str, want: Option<&str>, seen: Option<&Seen>) -> Result<(String, How), String> {
    let tmp = temp_file()?;
    let (code, etag, modified) = curl(url, &tmp, seen)?;
    if code == 304 {
        let _ = std::fs::remove_file(&tmp);
        let s = seen.ok_or(format!("{url}: 304 to a question not asked"))?;
        return Ok((s.sha256.clone(), How::NotModified));
    }
    let bytes = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
    let hash = hash_file(&tmp)?;
    if let Some(w) = want {
        if hash != w {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("{url}: content hash {hash}, expected {w}; not stored"));
        }
    }
    keep(&tmp, &hash)?;
    let _g = URLS.lock().unwrap();
    let mut urls = read_urls();
    urls.insert(url.to_string(), Seen { sha256: hash.clone(), etag, modified });
    let text = serde_json::to_string_pretty(&urls).unwrap();
    std::fs::write(urls_file(), text).map_err(|e| e.to_string())?;
    Ok((hash, How::Fetched(bytes)))
}

// ---- Cargo ---------------------------------------------------------------

/// A crates.io package locked in Cargo.lock, with its archive's hash.
pub struct LockedCrate {
    pub name: String,
    pub version: String,
    pub checksum: String,
}

pub fn cargo_lock(dir: &Path) -> Result<Vec<LockedCrate>, String> {
    let path = dir.join("Cargo.lock");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc: toml::Table = text.parse().map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    for p in doc.get("package").and_then(|p| p.as_array()).into_iter().flatten() {
        let Some(t) = p.as_table() else { continue };
        let s = |k: &str| t.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let Some(source) = s("source") else { continue }; // a path package
        if source != "registry+https://github.com/rust-lang/crates.io-index" && source != "sparse+https://index.crates.io/" {
            return Err(format!("{}: from {source}; fStrike knows crates.io only", s("name").unwrap_or_default()));
        }
        out.push(LockedCrate {
            name: s("name").unwrap_or_default(),
            version: s("version").unwrap_or_default(),
            checksum: s("checksum").ok_or(format!("{}: no checksum in Cargo.lock", s("name").unwrap_or_default()))?,
        });
    }
    Ok(out)
}

/// A crate's sources, struck from the store into `registry/<name>-<version>`.
/// A folder with its `.fstrike` mark is whole and is used as it is.
pub fn fetch_crate(c: &LockedCrate) -> Result<How, String> {
    let label = format!("{}-{}", c.name, c.version);
    let tree = registry_dir().join(&label);
    if std::fs::read_to_string(tree.join(".fstrike")).is_ok_and(|t| t.trim() == c.checksum) {
        return Ok(How::Struck);
    }
    let url = format!("https://static.crates.io/crates/{}/{label}.crate", c.name);
    let (hash, how) = fetch_url(&url, Some(&c.checksum))?;
    let archive = std::fs::read(blob(&hash)).map_err(|e| e.to_string())?;
    let mut tar = Vec::new();
    flate2::read::GzDecoder::new(&archive[..]).read_to_end(&mut tar).map_err(|e| format!("{label}: {e}"))?;

    // Every file goes to the store by its own content, then into a folder
    // that becomes the package's only when it is whole.
    let part = PathBuf::from(format!("{}.part{}", tree.display(), std::process::id()));
    let _ = std::fs::remove_dir_all(&part);
    for (path, body) in tar_files(&tar)? {
        let rel = Path::new(&path);
        let mut parts = rel.components();
        if parts.next().map(|p| p.as_os_str().to_string_lossy() != label.as_str()).unwrap_or(true) {
            return Err(format!("{label}: {path} is outside the package's folder"));
        }
        let rest = parts.as_path();
        if rest.as_os_str().is_empty() || !rest.components().all(|p| matches!(p, Component::Normal(_))) {
            return Err(format!("{label}: unsafe path {path}"));
        }
        let hash = put_bytes(&body)?;
        strike_to(&hash, &part.join(rest))?;
    }
    std::fs::write(part.join(".fstrike"), &c.checksum).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&tree);
    std::fs::rename(&part, &tree).map_err(|e| format!("{}: {e}", tree.display()))?;
    Ok(how)
}

/// Every crates.io package of a Cargo.lock, `jobs` at a time.
pub fn fetch_lock(dir: &Path, jobs: usize) -> Result<Vec<(String, How)>, String> {
    let crates = cargo_lock(dir)?;
    let next = AtomicU64::new(0);
    let done: Mutex<Vec<(String, Result<How, String>)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..jobs.min(crates.len()).max(1) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed) as usize;
                let Some(c) = crates.get(i) else { break };
                let r = fetch_crate(c);
                done.lock().unwrap().push((format!("{} {}", c.name, c.version), r));
            });
        }
    });
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (name, r) in done.into_inner().unwrap() {
        match r {
            Ok(h) => out.push((name, h)),
            Err(e) => errors.push(e),
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

// ---- tar -----------------------------------------------------------------

/// The plain files of a tar archive: (path, content). Folders, links and the
/// rest are left out. Long names come from GNU `L` entries or pax `path`.
fn tar_files(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    let mut at = 0;
    let mut long: Option<String> = None;
    while at + 512 <= data.len() {
        let h = &data[at..at + 512];
        if h.iter().all(|&b| b == 0) {
            break;
        }
        let size = octal(&h[124..136])?;
        let body = data.get(at + 512..at + 512 + size).ok_or("tar: cut short")?;
        at += 512 + size.div_ceil(512) * 512;
        match h[156] {
            b'L' => long = Some(c_str(body)),
            b'x' => {
                if let Some(p) = pax_path(body) {
                    long = Some(p);
                }
            }
            b'0' | b'7' | 0 => {
                let name = long.take().unwrap_or_else(|| {
                    let name = c_str(&h[0..100]);
                    let prefix = if &h[257..262] == b"ustar" { c_str(&h[345..500]) } else { String::new() };
                    if prefix.is_empty() { name } else { format!("{prefix}/{name}") }
                });
                out.push((name, body.to_vec()));
            }
            _ => long = None,
        }
    }
    Ok(out)
}

fn c_str(b: &[u8]) -> String {
    String::from_utf8_lossy(&b[..b.iter().position(|&x| x == 0).unwrap_or(b.len())]).to_string()
}

fn octal(field: &[u8]) -> Result<usize, String> {
    let s = c_str(field);
    let s = s.trim();
    if s.is_empty() {
        return Ok(0);
    }
    usize::from_str_radix(s, 8).map_err(|_| format!("tar: bad size {s:?}"))
}

/// The `path` record of a pax header ("<length> path=<value>\n" records).
fn pax_path(body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    let mut rest: &str = &text;
    while !rest.is_empty() {
        let (len, _) = rest.split_once(' ')?;
        let n: usize = len.parse().ok()?;
        let record = rest.get(..n)?;
        rest = &rest[n..];
        let (_, kv) = record.split_once(' ')?;
        if let Some(v) = kv.strip_prefix("path=") {
            return Some(v.trim_end_matches('\n').to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(name: &str, size: usize, kind: u8) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        let s = format!("{size:011o}");
        h[124..135].copy_from_slice(s.as_bytes());
        h[156] = kind;
        h[257..262].copy_from_slice(b"ustar");
        h
    }

    fn entry(t: &mut Vec<u8>, name: &str, kind: u8, body: &[u8]) {
        t.extend(header(name, body.len(), kind));
        t.extend(body);
        t.resize(t.len().div_ceil(512) * 512, 0);
    }

    #[test]
    fn tar_plain_long_and_pax_names() {
        let mut t = Vec::new();
        entry(&mut t, "a-1.0/", b'5', b"");
        entry(&mut t, "a-1.0/src/lib.rs", b'0', b"pub fn a() {}\n");
        let long = format!("a-1.0/{}.rs", "x".repeat(120));
        entry(&mut t, "././@LongLink", b'L', format!("{long}\0").as_bytes());
        entry(&mut t, "cut", b'0', b"long");
        let rec = "a-1.0/pax.txt";
        let line = format!(" path={rec}\n");
        let n = line.len() + 2; // two digits for the length itself
        entry(&mut t, "PaxHeader", b'x', format!("{n}{line}").as_bytes());
        entry(&mut t, "short", b'0', b"pax");
        entry(&mut t, "a-1.0/link", b'2', b"");
        t.extend([0u8; 1024]);
        let files = tar_files(&t).unwrap();
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["a-1.0/src/lib.rs", long.as_str(), rec]);
        assert_eq!(files[0].1, b"pub fn a() {}\n");
        assert_eq!(files[2].1, b"pax");
    }

    #[test]
    fn a_cut_archive_is_refused() {
        let mut t = header("a/b", 2000, b'0');
        t.extend([1u8; 100]);
        assert!(tar_files(&t).is_err());
    }
}
