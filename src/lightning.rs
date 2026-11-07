//! The lightning memory: every build is branded once, by what went into it,
//! and a strike looks there before the work is done again.
//!
//! A brand's key is the hash of everything that decides the output: the
//! compiler that makes it, the unit, the flags, the content (not the time)
//! of every file the work read, the environment variables it read, and the
//! keys of the brands it uses. The list of files comes from the last time
//! the work ran (rustc's dep-info, a build script's rerun-if-changed): a file
//! not yet on it can only enter through a file on it that changed, so the
//! key changes too.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct Lightning {
    home: PathBuf,
    tool: String,
    memory: Mutex<Memory>,
}

/// What Strike remembers of each unit between runs.
#[derive(Default, Serialize, Deserialize)]
struct Memory {
    units: BTreeMap<String, Remembered>,
}

#[derive(Default, Clone, Serialize, Deserialize)]
struct Remembered {
    /// The files the work read last time.
    inputs: Vec<PathBuf>,
    /// The environment variables it read.
    env: Vec<String>,
    /// Every flag set this unit was ever branded with.
    flag_sets: Vec<Vec<String>>,
}

/// What a brand records besides its files.
#[derive(Serialize, Deserialize)]
struct Seal {
    unit: String,
    flags: Vec<String>,
    files: Vec<String>,
}

pub fn home() -> PathBuf {
    if let Some(h) = std::env::var_os("STRIKE_HOME") {
        return PathBuf::from(h);
    }
    let base = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(".strike")
}

/// Where a unit's work learned what it read.
pub struct Learned {
    pub inputs: Vec<PathBuf>,
    pub env: Vec<String>,
}

impl Lightning {
    pub fn open(tool_version: String) -> Lightning {
        let home = home();
        let memory = std::fs::read_to_string(home.join("memory.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Lightning { home, tool: tool_version, memory: Mutex::new(memory) }
    }

    pub fn save(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.home).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(&*self.memory.lock().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(self.home.join("memory.json"), text).map_err(|e| e.to_string())
    }

    fn brand_dir(&self, key: &str) -> PathBuf {
        self.home.join("brand").join(key)
    }

    /// The key a unit would have with these flags, if what it reads is known
    /// and all of it still exists. `env` is what Strike gives the work; a
    /// variable it read that is not there comes from Strike's own.
    pub fn key(&self, unit: &str, flags: &[String], links: &[String], env: &BTreeMap<String, String>) -> Option<String> {
        let rem = self.memory.lock().unwrap().units.get(unit)?.clone();
        if rem.inputs.is_empty() && rem.flag_sets.is_empty() {
            return None;
        }
        let mut h = Sha256::new();
        h.update(b"strike-key-2\n");
        h.update(self.tool.as_bytes());
        h.update(format!("\nunit {unit}\n").as_bytes());
        for f in flags {
            h.update(format!("flag {f}\n").as_bytes());
        }
        for l in links {
            h.update(format!("link {l}\n").as_bytes());
        }
        let mut inputs = rem.inputs.clone();
        inputs.sort();
        for path in &inputs {
            let bytes = std::fs::read(path).ok()?;
            h.update(format!("file {} {}\n", path.display(), hex(&Sha256::digest(&bytes))).as_bytes());
        }
        let mut names = rem.env.clone();
        names.sort();
        for name in &names {
            let value = env.get(name).cloned().or_else(|| std::env::var(name).ok()).unwrap_or_default();
            h.update(format!("env {name}={value}\n").as_bytes());
        }
        Some(hex(&h.finalize()))
    }

    /// The strike: is this exact build already branded?
    pub fn strike(&self, key: &str) -> bool {
        self.brand_dir(key).join("seal.json").exists()
    }

    /// Every flag set this unit was ever branded with.
    pub fn flag_sets(&self, unit: &str) -> Vec<Vec<String>> {
        self.memory.lock().unwrap().units.get(unit).map(|r| r.flag_sets.clone()).unwrap_or_default()
    }

    /// Copies a brand's files into `root`, at the places they were taken from.
    pub fn restore(&self, key: &str, root: &Path) -> Result<Vec<PathBuf>, String> {
        let dir = self.brand_dir(key);
        let seal: Seal = serde_json::from_str(
            &std::fs::read_to_string(dir.join("seal.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let mut made = Vec::new();
        for f in &seal.files {
            let to = root.join(f);
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            std::fs::copy(dir.join("files").join(f), &to).map_err(|e| format!("{f}: {e}"))?;
            made.push(to);
        }
        Ok(made)
    }

    /// After the work ran: remember what it read, then brand its outputs
    /// (paths relative to `root`) under the key that gives. Returns the key.
    #[allow(clippy::too_many_arguments)]
    pub fn brand(
        &self,
        unit: &str,
        flags: &[String],
        links: &[String],
        env: &BTreeMap<String, String>,
        learned: Learned,
        root: &Path,
        outputs: &[PathBuf],
    ) -> Result<String, String> {
        {
            let mut mem = self.memory.lock().unwrap();
            let rem = mem.units.entry(unit.to_string()).or_default();
            rem.inputs = learned.inputs;
            rem.env = learned.env;
            if !rem.flag_sets.iter().any(|f| f == flags) {
                rem.flag_sets.push(flags.to_vec());
            }
        }
        let key = self.key(unit, flags, links, env).ok_or(format!("{unit}: the work read a file that is gone"))?;
        if self.strike(&key) {
            return Ok(key);
        }
        let dir = self.brand_dir(&key);
        // A half-written brand must never be struck: the seal comes last.
        let tmp = dir.with_extension(format!("part{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mut files = Vec::new();
        for o in outputs {
            let rel = o.strip_prefix(root).map_err(|_| format!("{} is not under {}", o.display(), root.display()))?;
            let to = tmp.join("files").join(rel);
            std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::copy(o, &to).map_err(|e| format!("{}: {e}", o.display()))?;
            files.push(rel.to_string_lossy().replace('\\', "/"));
        }
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let seal = Seal { unit: unit.to_string(), flags: flags.to_vec(), files };
        std::fs::write(tmp.join("seal.json"), serde_json::to_string_pretty(&seal).unwrap()).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(&tmp, &dir).map_err(|e| e.to_string())?;
        Ok(key)
    }

    /// Every brand: (key, unit, flags).
    pub fn list(&self) -> Vec<(String, String, Vec<String>)> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(self.home.join("brand")).into_iter().flatten().flatten() {
            let Ok(text) = std::fs::read_to_string(e.path().join("seal.json")) else { continue };
            let Ok(seal) = serde_json::from_str::<Seal>(&text) else { continue };
            out.push((e.file_name().to_string_lossy().to_string(), seal.unit, seal.flags));
        }
        out.sort();
        out
    }
}

/// The files and environment variables a rustc dep-info file names. Every
/// input gets an empty rule of its own ("path:"), which is what is read here.
pub fn read_dep_info(path: &Path) -> Result<Learned, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut inputs = Vec::new();
    let mut env = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# env-dep:") {
            let name = rest.split('=').next().unwrap_or(rest);
            env.push(name.to_string());
        } else if let Some(p) = line.strip_suffix(':') {
            if !p.is_empty() && !p.contains(": ") {
                inputs.push(PathBuf::from(p.replace("\\ ", " ")));
            }
        }
    }
    inputs.sort();
    inputs.dedup();
    env.sort();
    env.dedup();
    Ok(Learned { inputs, env })
}

/// Every file under a folder.
pub fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
