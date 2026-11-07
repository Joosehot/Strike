//! The lightning memory: every build is branded once, by what went into it,
//! and a strike looks there before rustc is called again.
//!
//! A brand's key is the hash of everything that decides the output: the
//! rustc that makes it, the unit, the flags, the content (not the time) of
//! every file rustc read, the environment variables it read, and the keys
//! of the brands it links against. The list of files comes from rustc's own
//! dep-info of the last build: a file not yet on it can only enter through a
//! file on it that changed, so the key changes too.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct Lightning {
    home: PathBuf,
    rustc: String,
    memory: Memory,
}

/// What Strike remembers of each unit between runs.
#[derive(Default, Serialize, Deserialize)]
struct Memory {
    units: BTreeMap<String, Remembered>,
}

#[derive(Default, Clone, Serialize, Deserialize)]
struct Remembered {
    /// The files rustc read last time.
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

impl Lightning {
    pub fn open(rustc_version: String) -> Lightning {
        let home = home();
        let memory = std::fs::read_to_string(home.join("memory.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Lightning { home, rustc: rustc_version, memory }
    }

    pub fn save(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.home).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(&self.memory).map_err(|e| e.to_string())?;
        std::fs::write(self.home.join("memory.json"), text).map_err(|e| e.to_string())
    }

    fn brand_dir(&self, key: &str) -> PathBuf {
        self.home.join("brand").join(key)
    }

    /// The key a unit would have with these flags, if its inputs are known
    /// and all still exist.
    pub fn key(&self, unit: &str, flags: &[String], links: &[String]) -> Option<String> {
        let rem = self.memory.units.get(unit)?;
        if rem.inputs.is_empty() {
            return None;
        }
        let mut h = Sha256::new();
        h.update(b"strike-key-1\n");
        h.update(self.rustc.as_bytes());
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
        let mut env = rem.env.clone();
        env.sort();
        for name in &env {
            let value = std::env::var(name).unwrap_or_default();
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
        self.memory.units.get(unit).map(|r| r.flag_sets.clone()).unwrap_or_default()
    }

    /// Copies a brand's files into `out`.
    pub fn restore(&self, key: &str, out: &Path) -> Result<Vec<PathBuf>, String> {
        let dir = self.brand_dir(key);
        let seal: Seal = serde_json::from_str(
            &std::fs::read_to_string(dir.join("seal.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
        let mut made = Vec::new();
        for f in &seal.files {
            let to = out.join(f);
            std::fs::copy(dir.join(f), &to).map_err(|e| format!("{f}: {e}"))?;
            made.push(to);
        }
        Ok(made)
    }

    /// After rustc built a unit: learn what it read from its dep-info, then
    /// brand its outputs under the key that gives. Returns the key.
    pub fn brand(
        &mut self,
        unit: &str,
        flags: &[String],
        links: &[String],
        dep_info: &Path,
        outputs: &[PathBuf],
    ) -> Result<String, String> {
        let (inputs, env) = read_dep_info(dep_info)?;
        let rem = self.memory.units.entry(unit.to_string()).or_default();
        rem.inputs = inputs;
        rem.env = env;
        if !rem.flag_sets.iter().any(|f| f == flags) {
            rem.flag_sets.push(flags.to_vec());
        }
        let key = self
            .key(unit, flags, links)
            .ok_or("rustc read a file that is gone")?;
        let dir = self.brand_dir(&key);
        // A half-written brand must never be struck: the seal comes last.
        let tmp = dir.with_extension("part");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let mut files = Vec::new();
        for o in outputs {
            let name = o.file_name().unwrap().to_string_lossy().to_string();
            std::fs::copy(o, tmp.join(&name)).map_err(|e| format!("{name}: {e}"))?;
            files.push(name);
        }
        let seal = Seal { unit: unit.to_string(), flags: flags.to_vec(), files };
        std::fs::write(tmp.join("seal.json"), serde_json::to_string_pretty(&seal).unwrap())
            .map_err(|e| e.to_string())?;
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
pub fn read_dep_info(path: &Path) -> Result<(Vec<PathBuf>, Vec<String>), String> {
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
    Ok((inputs, env))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
