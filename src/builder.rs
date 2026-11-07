//! Steps 4 to 8: the motor chooses the build, the lightning strikes before
//! each rustc call, rustc builds what is not branded, and the check runs.

use crate::lightning::{hex, Lightning};
use crate::manifest::{Kind, Package, Unit};
use crate::motor::{self, Branded, Candidate, Rules};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub struct Options {
    pub profile: String,
    pub explain: bool,
    pub test: bool,
}

/// What happened to one unit.
pub struct Done {
    pub unit: String,
    pub struck: bool,
    pub seconds: f64,
    pub key: String,
}

pub struct Report {
    pub chosen: Candidate,
    pub done: Vec<Done>,
    pub out: PathBuf,
    /// (test, passed) when testing.
    pub tests: Vec<(String, bool)>,
}

pub fn rustc_version() -> Result<String, String> {
    let out = Command::new("rustc").arg("-vV").output().map_err(|e| format!("rustc: {e}"))?;
    if !out.status.success() {
        return Err("rustc -vV failed".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// The units in build order: the library first, as the others link to it.
fn units(pkg: &Package, test: bool) -> Vec<Unit> {
    let mut list: Vec<Unit> = pkg.lib.iter().cloned().collect();
    if test {
        if let Some(lib) = &pkg.lib {
            list.push(Unit {
                crate_name: lib.crate_name.clone(),
                out_name: format!("{}-libtest", lib.crate_name),
                kind: Kind::Test,
                root: lib.root.clone(),
                uses_lib: false,
            });
        }
        list.extend(pkg.tests.iter().cloned());
    } else {
        list.extend(pkg.bins.iter().cloned());
    }
    list
}

fn unit_key(pkg: &Package, u: &Unit) -> String {
    format!("{}|{}", pkg.dir.display(), u.id())
}

/// The keys of every unit under one flag set (None where a unit's inputs
/// are not known yet).
fn keys(lightning: &Lightning, pkg: &Package, list: &[Unit], flags: &[String]) -> Vec<Option<String>> {
    let lib_key = list
        .iter()
        .find(|u| u.kind == Kind::Lib)
        .and_then(|u| lightning.key(&unit_key(pkg, u), flags, &[]));
    list.iter()
        .map(|u| {
            if u.uses_lib {
                let l = lib_key.clone()?;
                lightning.key(&unit_key(pkg, u), flags, &[l])
            } else {
                lightning.key(&unit_key(pkg, u), flags, &[])
            }
        })
        .collect()
}

/// What the lightning knows, for the motor's judge.
fn branded(lightning: &Lightning, pkg: &Package, list: &[Unit]) -> Branded {
    let mut all: Vec<Vec<String>> = Vec::new();
    for u in list {
        // Each remembered set is checked below, with the library's key for
        // the units that link it.
        for set in lightning.flag_sets(&unit_key(pkg, u)) {
            if !all.contains(&set) {
                all.push(set);
            }
        }
    }
    let sets = all
        .into_iter()
        .map(|set| {
            let n = keys(lightning, pkg, list, &set)
                .iter()
                .filter(|k| k.as_ref().is_some_and(|k| lightning.strike(k)))
                .count();
            (set, n)
        })
        .filter(|(_, n)| *n > 0)
        .collect();
    Branded { sets, units: list.len() }
}

pub fn build(pkg: &Package, rules: &Rules, opt: &Options) -> Result<Report, String> {
    let profile = *rules.profiles.get(&opt.profile).ok_or(format!(
        "no profile {} in rules.toml (there are: {})",
        opt.profile,
        rules.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
    ))?;
    // The variables Cargo gives a crate; env!() reads them, and the
    // lightning hashes what it read.
    std::env::set_var("CARGO_PKG_NAME", &pkg.name);
    std::env::set_var("CARGO_PKG_VERSION", &pkg.version);
    std::env::set_var("CARGO_MANIFEST_DIR", &pkg.dir);

    let mut lightning = Lightning::open(rustc_version()?);
    let list = units(pkg, opt.test);
    if list.is_empty() {
        return Err("the package has nothing to build (no src/lib.rs, src/main.rs or [[bin]])".into());
    }

    let finalists = motor::search(rules, &profile, &branded(&lightning, pkg, &list));
    if opt.explain {
        println!("finalists ({}, profile {}):", finalists.len(), opt.profile);
        for c in &finalists {
            let choice: Vec<String> = c.choice.iter().map(|(r, v)| format!("{r}={v}")).collect();
            println!("  {:.3} = {:.3} + judges {:.3}  {}", c.score(), c.local, c.judged, choice.join(" "));
            for n in &c.notes {
                println!("      {n}");
            }
        }
    }
    let chosen = finalists.into_iter().next().ok_or("the beam is empty")?;
    let flags = motor::flags(&chosen.choice);

    let out = pkg.dir.join("strike-out").join(&opt.profile);
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let mut done = Vec::new();
    let mut lib_key: Option<String> = None;
    for u in &list {
        let id = unit_key(pkg, u);
        let links: Vec<String> = if u.uses_lib {
            vec![lib_key.clone().ok_or("the library was not built")?]
        } else {
            Vec::new()
        };
        let t = Instant::now();
        let (key, struck) = match lightning.key(&id, &flags, &links) {
            Some(k) if lightning.strike(&k) => {
                lightning.restore(&k, &out)?;
                (k, true)
            }
            _ => {
                let k = compile(&mut lightning, pkg, u, &id, &flags, &links, &out)?;
                lightning.restore(&k, &out)?;
                (k, false)
            }
        };
        if u.kind == Kind::Lib {
            lib_key = Some(key.clone());
        }
        done.push(Done { unit: u.id(), struck, seconds: t.elapsed().as_secs_f64(), key });
    }
    lightning.save()?;

    // The check: what each unit makes is there; a test must pass.
    let mut tests = Vec::new();
    for u in &list {
        let exe = out.join(exe_name(u));
        let made = match u.kind {
            Kind::Lib => out.join(format!("lib{}.rlib", u.crate_name)),
            _ => exe.clone(),
        };
        if !made.exists() {
            return Err(format!("check failed: {} was not made", made.display()));
        }
        if u.kind == Kind::Test {
            let status = Command::new(&exe)
                .current_dir(&pkg.dir)
                .status()
                .map_err(|e| format!("{}: {e}", exe.display()))?;
            tests.push((u.out_name.clone(), status.success()));
        }
    }
    Ok(Report { chosen, done, out, tests })
}

fn exe_name(u: &Unit) -> String {
    format!("{}{}", u.out_name, std::env::consts::EXE_SUFFIX)
}

/// One rustc call, into a fresh folder, then branded.
fn compile(
    lightning: &mut Lightning,
    pkg: &Package,
    u: &Unit,
    id: &str,
    flags: &[String],
    links: &[String],
    out: &Path,
) -> Result<String, String> {
    let tag = hex(&Sha256::digest(format!("{id}|{}", flags.join(" ")).as_bytes()))[..16].to_string();
    let tmp = crate::lightning::home().join("work").join(&tag);
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;

    let mut cmd = Command::new("rustc");
    cmd.arg("--crate-name").arg(&u.crate_name).arg("--edition").arg(&pkg.edition);
    match u.kind {
        Kind::Lib => {
            cmd.arg("--crate-type").arg("lib");
        }
        Kind::Bin => {
            cmd.arg("--crate-type").arg("bin");
        }
        Kind::Test => {
            cmd.arg("--test");
        }
    }
    cmd.arg(&u.root)
        .arg("--emit=dep-info,link")
        .arg("--out-dir")
        .arg(&tmp)
        .env("CARGO_CRATE_NAME", &u.crate_name);
    // Symbol names hang on the metadata: the same unit always gets the same.
    let meta = hex(&Sha256::digest(format!("{}|{}|{}", pkg.name, pkg.version, u.id()).as_bytes()))[..16].to_string();
    cmd.arg(format!("-Cmetadata={meta}"));
    for f in flags {
        if f == "incremental" {
            let dir = crate::lightning::home().join("incr").join(&tag);
            cmd.arg(format!("-Cincremental={}", dir.display()));
        } else {
            cmd.arg(f);
        }
    }
    if u.uses_lib {
        let lib = pkg.lib.as_ref().unwrap();
        cmd.arg("--extern")
            .arg(format!("{}={}", lib.crate_name, out.join(format!("lib{}.rlib", lib.crate_name)).display()))
            .arg("-L")
            .arg(format!("dependency={}", out.display()));
    }
    let status = cmd.status().map_err(|e| format!("rustc: {e}"))?;
    if !status.success() {
        return Err(format!("rustc could not build {} (its errors are above)", u.id()));
    }

    // rustc names a binary by its crate name; give it the unit's own.
    if u.kind != Kind::Lib && u.out_name != u.crate_name {
        let from = tmp.join(format!("{}{}", u.crate_name, std::env::consts::EXE_SUFFIX));
        let _ = std::fs::rename(&from, tmp.join(exe_name(u)));
    }
    let dep_info = tmp.join(format!("{}.d", u.crate_name));
    let mut outputs: Vec<PathBuf> = std::fs::read_dir(&tmp)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_none_or(|x| x != "d"))
        .collect();
    outputs.sort();
    let key = lightning.brand(id, flags, links, &dep_info, &outputs)?;
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(key)
}
