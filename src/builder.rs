//! Steps 4 to 8: the motor chooses the build, then every job of the graph
//! (build scripts, their runs, libraries, proc macros, binaries, tests) runs
//! as soon as what it uses is done; the lightning strikes before each one.

use crate::graph::{DepKind, Graph, Node, Target};
use crate::lightning::{self, hex, Learned, Lightning};
use crate::manifest::{Kind, Package, Unit};
use crate::motor::{self, Branded, Candidate, Rules};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Condvar, Mutex};
use std::time::Instant;

pub struct Options {
    pub profile: String,
    pub explain: bool,
    pub test: bool,
    pub jobs: usize,
}

/// What happened to one job.
pub struct Done {
    pub job: String,
    pub root: bool,
    pub struck: bool,
    pub seconds: f64,
    pub key: String,
}

pub struct Report {
    pub chosen: Candidate,
    pub done: Vec<Done>,
    pub out: PathBuf,
    pub seconds: f64,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JobKind {
    Lib,
    ScriptBuild,
    ScriptRun,
    /// A binary or test harness of the root package.
    Root,
}

struct Job {
    kind: JobKind,
    node: usize,
    /// The root package's unit, for JobKind::Root.
    unit: Option<Unit>,
    /// The jobs whose results this one needs.
    needs: Vec<usize>,
    /// The memory's name for this job.
    id: String,
}

/// What a build script's run said.
#[derive(Clone, Default)]
struct ScriptOut {
    cfgs: Vec<String>,
    envs: Vec<(String, String)>,
    link_libs: Vec<String>,
    link_search: Vec<String>,
}

#[derive(Clone, Default)]
struct Result_ {
    key: String,
    script: ScriptOut,
}

/// A node's metadata hash: its name, version, features and where it is.
fn meta(n: &Node) -> String {
    let f: Vec<&str> = n.features.iter().map(String::as_str).collect();
    hex(&Sha256::digest(format!("{}|{}|{}|{}", n.name, n.version, f.join(","), n.dir.display()).as_bytes()))[..16].to_string()
}

fn is_registry(n: &Node) -> bool {
    n.dir.components().any(|c| c.as_os_str() == "registry")
}

fn plan(g: &Graph, pkg: &Package, test: bool) -> Result<Vec<Job>, String> {
    let order = g.order()?;
    let mut jobs: Vec<Job> = Vec::new();
    let mut lib_job: BTreeMap<usize, usize> = BTreeMap::new();
    let mut run_job: BTreeMap<usize, usize> = BTreeMap::new();
    let ident = |n: &Node, what: &str| {
        if is_registry(n) {
            format!("{}-{}|{what}|{}", n.name, n.version, meta(n))
        } else {
            format!("{}|{what}|{}", n.dir.display(), meta(n))
        }
    };
    for &i in &order {
        let n = &g.nodes[i];
        if n.build_script.is_some() {
            let needs = n
                .active
                .iter()
                .filter(|(d, _)| d.kind == DepKind::Build)
                .filter_map(|(_, x)| lib_job.get(x).copied())
                .collect();
            jobs.push(Job { kind: JobKind::ScriptBuild, node: i, unit: None, needs, id: ident(n, "script") });
            let mut needs = vec![jobs.len() - 1];
            for (d, x) in &n.active {
                if d.kind == DepKind::Normal && g.nodes[*x].links.is_some() {
                    if let Some(r) = run_job.get(x) {
                        needs.push(*r);
                    }
                }
            }
            jobs.push(Job { kind: JobKind::ScriptRun, node: i, unit: None, needs, id: ident(n, "run") });
            run_job.insert(i, jobs.len() - 1);
        }
        if n.lib.is_some() {
            let mut needs: Vec<usize> = n
                .active
                .iter()
                .filter(|(d, _)| d.kind == DepKind::Normal)
                .filter_map(|(_, x)| lib_job.get(x).copied())
                .collect();
            if let Some(r) = run_job.get(&i) {
                needs.push(*r);
            }
            jobs.push(Job { kind: JobKind::Lib, node: i, unit: None, needs, id: ident(n, "lib") });
            lib_job.insert(i, jobs.len() - 1);
        }
    }
    let root = g.root;
    let mut units: Vec<Unit> = Vec::new();
    if test {
        if let Some(lib) = &pkg.lib {
            units.push(Unit {
                crate_name: lib.crate_name.clone(),
                out_name: format!("{}-libtest", lib.crate_name),
                kind: Kind::Test,
                root: lib.root.clone(),
                uses_lib: false,
            });
        }
        units.extend(pkg.tests.iter().cloned());
    } else {
        units.extend(pkg.bins.iter().cloned());
    }
    for u in units {
        let mut needs: Vec<usize> = g.nodes[root]
            .active
            .iter()
            .filter(|(d, _)| d.kind == DepKind::Normal || (test && d.kind == DepKind::Dev))
            .filter_map(|(_, x)| lib_job.get(x).copied())
            .collect();
        if u.uses_lib {
            needs.push(*lib_job.get(&root).ok_or("the library was not planned")?);
        }
        if let Some(r) = run_job.get(&root) {
            needs.push(*r);
        }
        let id = format!("{}|{}", pkg.dir.display(), u.id());
        jobs.push(Job { kind: JobKind::Root, node: root, unit: Some(u), needs, id });
    }
    Ok(jobs)
}

/// The flags a job is built with: the root's own code gets the motor's
/// choice; the dependencies the same without incremental work, as their
/// sources do not change.
fn flags_for(job: &Job, root: usize, chosen: &[String]) -> Vec<String> {
    if job.node == root {
        chosen.to_vec()
    } else {
        chosen.iter().filter(|f| *f != "incremental").cloned().collect()
    }
}

struct Ctx<'a> {
    g: &'a Graph,
    target: &'a Target,
    lightning: &'a Lightning,
    jobs: &'a [Job],
    out: PathBuf,
    profile: String,
    chosen: Vec<String>,
}

impl Ctx<'_> {
    fn node(&self, j: &Job) -> &Node {
        &self.g.nodes[j.node]
    }

    fn script_dir(&self, n: &Node) -> PathBuf {
        self.out.join("build").join(format!("{}-{}", n.name, meta(n)))
    }

    /// The variables Cargo gives the compiler and the build scripts.
    fn pkg_env(&self, n: &Node) -> BTreeMap<String, String> {
        let mut e = BTreeMap::new();
        let pkg = n.manifest.get("package").and_then(|p| p.as_table());
        let s = |k: &str| pkg.and_then(|p| p.get(k)).and_then(|v| v.as_str()).unwrap_or("").to_string();
        e.insert("CARGO_PKG_NAME".into(), n.name.clone());
        e.insert("CARGO_PKG_VERSION".into(), n.version.clone());
        let (core, pre) = n.version.split_once('-').unwrap_or((&n.version, ""));
        let mut parts = core.split('.');
        for k in ["MAJOR", "MINOR", "PATCH"] {
            e.insert(format!("CARGO_PKG_VERSION_{k}"), parts.next().unwrap_or("0").to_string());
        }
        e.insert("CARGO_PKG_VERSION_PRE".into(), pre.to_string());
        let authors = pkg
            .and_then(|p| p.get("authors"))
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(":"))
            .unwrap_or_default();
        e.insert("CARGO_PKG_AUTHORS".into(), authors);
        for (var, key) in [
            ("CARGO_PKG_DESCRIPTION", "description"),
            ("CARGO_PKG_HOMEPAGE", "homepage"),
            ("CARGO_PKG_REPOSITORY", "repository"),
            ("CARGO_PKG_LICENSE", "license"),
            ("CARGO_PKG_LICENSE_FILE", "license-file"),
            ("CARGO_PKG_RUST_VERSION", "rust-version"),
            ("CARGO_PKG_README", "readme"),
        ] {
            e.insert(var.into(), s(key));
        }
        e.insert("CARGO_MANIFEST_DIR".into(), n.dir.display().to_string());
        e.insert("CARGO_MANIFEST_PATH".into(), n.dir.join("Cargo.toml").display().to_string());
        if n.build_script.is_some() {
            e.insert("OUT_DIR".into(), self.script_dir(n).join("out").display().to_string());
        }
        e
    }

    /// What a build script's run gets besides the package's variables.
    fn script_env(&self, n: &Node) -> BTreeMap<String, String> {
        let mut e = self.pkg_env(n);
        let release = self.chosen.iter().any(|f| f.starts_with("-Copt-level=") && !f.ends_with("=0"));
        let opt = self.chosen.iter().find_map(|f| f.strip_prefix("-Copt-level=")).unwrap_or("0");
        let debug = self.chosen.iter().find_map(|f| f.strip_prefix("-Cdebuginfo=")).unwrap_or("0") != "0";
        e.insert("TARGET".into(), self.target.triple.clone());
        e.insert("HOST".into(), self.target.triple.clone());
        e.insert("PROFILE".into(), if release { "release" } else { "debug" }.into());
        e.insert("OPT_LEVEL".into(), opt.into());
        e.insert("DEBUG".into(), debug.to_string());
        e.insert("NUM_JOBS".into(), "1".into());
        e.insert("RUSTC".into(), "rustc".into());
        e.insert("CARGO_ENCODED_RUSTFLAGS".into(), String::new());
        let mut cfg: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (k, v) in &self.target.cfg {
            cfg.entry(k.clone()).or_default().extend(v.clone());
        }
        for (k, vs) in cfg {
            e.insert(format!("CARGO_CFG_{}", k.to_uppercase()), vs.join(","));
        }
        for f in &n.features {
            e.insert(format!("CARGO_FEATURE_{}", f.to_uppercase().replace('-', "_")), "1".into());
        }
        e
    }

    fn links(&self, j: &Job, results: &[Option<Result_>]) -> Vec<String> {
        j.needs.iter().map(|&x| results[x].as_ref().map(|r| r.key.clone()).unwrap_or_default()).collect()
    }

    /// The env a job's key is taken over, and its flags.
    fn key_parts(&self, j: &Job) -> (Vec<String>, BTreeMap<String, String>) {
        let n = self.node(j);
        let mut flags = flags_for(j, self.g.root, &self.chosen);
        let env = match j.kind {
            JobKind::ScriptRun => {
                let e = self.script_env(n);
                // A run's whole environment decides it, not only what it read.
                flags.extend(e.iter().map(|(k, v)| format!("env {k}={v}")));
                e
            }
            _ => self.pkg_env(n),
        };
        (flags, env)
    }

    /// Every unit linked in the end needs the native search paths of every
    /// build script below it.
    fn search_paths(&self, j: usize, results: &[Option<Result_>]) -> Vec<String> {
        let mut seen = vec![false; self.jobs.len()];
        let mut stack = vec![j];
        let mut out = Vec::new();
        while let Some(x) = stack.pop() {
            if std::mem::replace(&mut seen[x], true) {
                continue;
            }
            if let Some(r) = &results[x] {
                for s in &r.script.link_search {
                    if !out.contains(s) {
                        out.push(s.clone());
                    }
                }
            }
            stack.extend(self.jobs[x].needs.iter().copied());
        }
        out
    }

    fn rustc(&self, j: &Job, flags: &[String], results: &[Option<Result_>], tmp: &Path) -> Command {
        let n = self.node(j);
        let mut cmd = Command::new("rustc");
        let m = meta(n);
        cmd.arg("--edition").arg(&n.edition).arg("--error-format=short");
        for f in flags {
            if f == "incremental" {
                let dir = lightning::home().join("incr").join(&hex(&Sha256::digest(j.id.as_bytes()))[..16]);
                cmd.arg(format!("-Cincremental={}", dir.display()));
            } else if !f.starts_with("env ") {
                cmd.arg(f);
            }
        }
        if j.node != self.g.root {
            cmd.arg("--cap-lints").arg("allow");
        }
        for f in &n.features {
            cmd.arg("--cfg").arg(format!("feature=\"{f}\""));
        }
        // What the build script said.
        let run = self.jobs.iter().position(|x| x.kind == JobKind::ScriptRun && x.node == j.node);
        if j.kind != JobKind::ScriptBuild {
            if let Some(r) = run.and_then(|r| results[r].as_ref()) {
                for c in &r.script.cfgs {
                    cmd.arg("--cfg").arg(c);
                }
                for (k, v) in &r.script.envs {
                    cmd.env(k, v);
                }
                for l in &r.script.link_libs {
                    cmd.arg("-l").arg(l);
                }
            }
        }
        for s in self.search_paths(self.jobs.iter().position(|x| std::ptr::eq(x, j)).unwrap(), results) {
            cmd.arg("-L").arg(s);
        }
        cmd.envs(self.pkg_env(n));
        cmd.arg("-L").arg(format!("dependency={}", self.out.join("deps").display()));
        // The libraries it may use, under the names its code uses.
        let kinds: &[DepKind] = match j.kind {
            JobKind::ScriptBuild => &[DepKind::Build],
            JobKind::Root if self.is_test(j) => &[DepKind::Normal, DepKind::Dev],
            _ => &[DepKind::Normal],
        };
        for (d, x) in &n.active {
            if !kinds.contains(&d.kind) {
                continue;
            }
            if let Some(p) = self.lib_file(&self.g.nodes[*x]) {
                cmd.arg("--extern").arg(format!("{}={}", d.extern_name, p.display()));
            }
        }
        if j.kind == JobKind::Root && j.unit.as_ref().unwrap().uses_lib {
            let p = self.lib_file(n).unwrap();
            cmd.arg("--extern").arg(format!("{}={}", n.lib.as_ref().unwrap().0, p.display()));
        }
        match j.kind {
            JobKind::Lib => {
                let (name, root) = n.lib.as_ref().unwrap();
                cmd.arg("--crate-name").arg(name);
                if n.proc_macro {
                    cmd.arg("--crate-type").arg("proc-macro").arg("--extern").arg("proc_macro");
                } else {
                    cmd.arg("--crate-type").arg("lib");
                }
                cmd.arg(root)
                    .arg(format!("-Cmetadata={m}"))
                    .arg(format!("-Cextra-filename=-{m}"))
                    .arg("--emit=dep-info,link")
                    .arg("--out-dir")
                    .arg(tmp.join("deps"))
                    .env("CARGO_CRATE_NAME", name);
            }
            JobKind::ScriptBuild => {
                cmd.arg("--crate-name").arg("build_script_build").arg("--crate-type").arg("bin");
                cmd.arg(n.build_script.as_ref().unwrap())
                    .arg(format!("-Cmetadata={m}-script"))
                    .arg("--emit=dep-info,link")
                    .arg("--out-dir")
                    .arg(tmp.join(self.script_dir(n).strip_prefix(&self.out).unwrap()))
                    .env("CARGO_CRATE_NAME", "build_script_build");
            }
            JobKind::Root => {
                let u = j.unit.as_ref().unwrap();
                cmd.arg("--crate-name").arg(&u.crate_name);
                if u.kind == Kind::Test {
                    cmd.arg("--test");
                } else {
                    cmd.arg("--crate-type").arg("bin");
                }
                cmd.arg(&u.root)
                    .arg(format!("-Cmetadata={m}-{}", u.out_name))
                    .arg("--emit=dep-info,link")
                    .arg("--out-dir")
                    .arg(tmp)
                    .env("CARGO_CRATE_NAME", &u.crate_name);
            }
            JobKind::ScriptRun => unreachable!(),
        }
        cmd
    }

    fn is_test(&self, j: &Job) -> bool {
        j.unit.as_ref().is_some_and(|u| u.kind == Kind::Test)
    }

    /// Where a node's library is, as others link it.
    fn lib_file(&self, n: &Node) -> Option<PathBuf> {
        let (name, _) = n.lib.as_ref()?;
        let m = meta(n);
        Some(if n.proc_macro {
            self.out.join("deps").join(format!("{name}-{m}{}", std::env::consts::DLL_SUFFIX))
        } else {
            self.out.join("deps").join(format!("lib{name}-{m}.rlib"))
        })
    }

    fn script_out(&self, n: &Node) -> Result<ScriptOut, String> {
        let text = std::fs::read_to_string(self.script_dir(n).join("output")).map_err(|e| e.to_string())?;
        Ok(parse_script(&text, n))
    }

    /// Strike, or do the job and brand it.
    fn run(&self, ji: usize, results: &[Option<Result_>]) -> Result<(Result_, bool), String> {
        let j = &self.jobs[ji];
        let n = self.node(j);
        let (flags, env) = self.key_parts(j);
        let links = self.links(j, results);
        if let Some(k) = self.lightning.key(&j.id, &flags, &links, &env) {
            if self.lightning.strike(&k) {
                self.lightning.restore(&k, &self.out)?;
                let script = if j.kind == JobKind::ScriptRun { self.script_out(n)? } else { ScriptOut::default() };
                return Ok((Result_ { key: k, script }, true));
            }
        }

        let tmp = lightning::home().join("work").join(&hex(&Sha256::digest(format!("{}|{}", j.id, flags.join(" ")).as_bytes()))[..16]);
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("deps")).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(tmp.join(self.script_dir(n).strip_prefix(&self.out).unwrap())).map_err(|e| e.to_string())?;

        let (learned, outputs, root) = if j.kind == JobKind::ScriptRun {
            let dir = self.script_dir(n);
            let out_dir = dir.join("out");
            let _ = std::fs::remove_dir_all(&out_dir);
            std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
            let mut cmd = Command::new(dir.join(format!("build-script{}", std::env::consts::EXE_SUFFIX)));
            cmd.current_dir(&n.dir).envs(&env);
            // What a dependency's build script told its dependents.
            for (d, x) in &n.active {
                let dn = &self.g.nodes[*x];
                if d.kind != DepKind::Normal {
                    continue;
                }
                if let Some(l) = &dn.links {
                    if let Ok(text) = std::fs::read_to_string(self.script_dir(dn).join("output")) {
                        for (k, v) in metadata(&text) {
                            cmd.env(format!("DEP_{}_{}", l.to_uppercase().replace('-', "_"), k.to_uppercase().replace('-', "_")), v);
                        }
                    }
                }
            }
            let o = cmd.output().map_err(|e| format!("{}'s build script: {e}", n.name))?;
            std::fs::write(dir.join("output"), &o.stdout).map_err(|e| e.to_string())?;
            if !o.status.success() {
                return Err(format!(
                    "{}'s build script failed:\n{}{}",
                    n.name,
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                ));
            }
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            let mut learned = rerun_of(&text, n);
            if learned.inputs.is_empty() && !is_registry(n) {
                // No rerun-if-changed: Cargo runs it again when any file of the package changes.
                learned.inputs = lightning::files_under(&n.dir)
                    .into_iter()
                    .filter(|p| !p.components().any(|c| c.as_os_str() == "target" || c.as_os_str() == "strike-out"))
                    .collect();
            }
            if learned.inputs.is_empty() {
                learned.inputs.push(n.build_script.clone().unwrap());
            }
            let mut outputs = lightning::files_under(&out_dir);
            outputs.push(dir.join("output"));
            (learned, outputs, self.out.clone())
        } else {
            let mut cmd = self.rustc(j, &flags, results, &tmp);
            let o = cmd.output().map_err(|e| format!("rustc: {e}"))?;
            if !o.status.success() {
                return Err(format!("rustc could not build {}:\n{}", j.id, String::from_utf8_lossy(&o.stderr)));
            }
            let mut outputs: Vec<PathBuf> = lightning::files_under(&tmp)
                .into_iter()
                .filter(|p| p.extension().is_none_or(|x| x != "d"))
                .collect();
            // Name what was made the way the others look for it.
            match j.kind {
                JobKind::ScriptBuild => {
                    let dir = tmp.join(self.script_dir(n).strip_prefix(&self.out).unwrap());
                    let from = dir.join(format!("build_script_build{}", std::env::consts::EXE_SUFFIX));
                    let to = dir.join(format!("build-script{}", std::env::consts::EXE_SUFFIX));
                    std::fs::rename(&from, &to).map_err(|e| e.to_string())?;
                    outputs = lightning::files_under(&tmp).into_iter().filter(|p| p.extension().is_none_or(|x| x != "d")).collect();
                }
                JobKind::Root => {
                    let u = j.unit.as_ref().unwrap();
                    if u.out_name != u.crate_name {
                        let from = tmp.join(format!("{}{}", u.crate_name, std::env::consts::EXE_SUFFIX));
                        std::fs::rename(&from, tmp.join(format!("{}{}", u.out_name, std::env::consts::EXE_SUFFIX))).map_err(|e| e.to_string())?;
                        outputs = lightning::files_under(&tmp).into_iter().filter(|p| p.extension().is_none_or(|x| x != "d")).collect();
                    }
                }
                _ => {}
            }
            let dep_info = lightning::files_under(&tmp)
                .into_iter()
                .find(|p| p.extension().is_some_and(|x| x == "d"))
                .ok_or("rustc wrote no dep-info")?;
            (lightning::read_dep_info(&dep_info)?, outputs, tmp.clone())
        };
        let key = self.lightning.brand(&j.id, &flags, &links, &env, learned, &root, &outputs)?;
        if j.kind != JobKind::ScriptRun {
            self.lightning.restore(&key, &self.out)?;
        }
        let _ = std::fs::remove_dir_all(&tmp);
        let script = if j.kind == JobKind::ScriptRun { self.script_out(n)? } else { ScriptOut::default() };
        Ok((Result_ { key, script }, false))
    }
}

/// The lines a build script prints: `cargo:key=value` or `cargo::key=value`.
fn script_lines(text: &str) -> impl Iterator<Item = (String, String)> + '_ {
    text.lines().filter_map(|l| {
        let rest = l.strip_prefix("cargo::").or_else(|| l.strip_prefix("cargo:"))?;
        let (k, v) = rest.split_once('=')?;
        Some((k.to_string(), v.to_string()))
    })
}

const KNOWN: [&str; 12] = [
    "rustc-cfg",
    "rustc-env",
    "rustc-link-lib",
    "rustc-link-search",
    "rustc-flags",
    "rustc-check-cfg",
    "rustc-link-arg",
    "rerun-if-changed",
    "rerun-if-env-changed",
    "warning",
    "error",
    "metadata",
];

fn parse_script(text: &str, n: &Node) -> ScriptOut {
    let mut o = ScriptOut::default();
    for (k, v) in script_lines(text) {
        match k.as_str() {
            "rustc-cfg" => o.cfgs.push(v),
            "rustc-env" => {
                if let Some((a, b)) = v.split_once('=') {
                    o.envs.push((a.to_string(), b.to_string()));
                }
            }
            "rustc-link-lib" => o.link_libs.push(v),
            "rustc-link-search" => o.link_search.push(v),
            "rustc-flags" => {
                let mut it = v.split_whitespace();
                while let Some(f) = it.next() {
                    let (flag, val) = match (f, f.get(2..)) {
                        ("-l" | "-L", _) => (f, it.next().unwrap_or("").to_string()),
                        (_, Some(rest)) => (&f[..2], rest.to_string()),
                        _ => continue,
                    };
                    match flag {
                        "-l" => o.link_libs.push(val),
                        "-L" => o.link_search.push(val),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let _ = n;
    o
}

/// `cargo:KEY=VALUE` lines that are not instructions, given to dependents
/// as DEP_<links>_<KEY> (and `cargo::metadata=KEY=VALUE`).
fn metadata(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (k, v) in script_lines(text) {
        if k == "metadata" {
            if let Some((a, b)) = v.split_once('=') {
                out.push((a.to_string(), b.to_string()));
            }
        } else if !KNOWN.contains(&k.as_str()) {
            out.push((k, v));
        }
    }
    out
}

/// What a build script asked to be run again for.
fn rerun_of(text: &str, n: &Node) -> Learned {
    let mut inputs = Vec::new();
    let mut env = Vec::new();
    for (k, v) in script_lines(text) {
        match k.as_str() {
            "rerun-if-changed" => {
                let p = n.dir.join(&v);
                if p.is_dir() {
                    inputs.extend(lightning::files_under(&p));
                } else if p.exists() {
                    inputs.push(p);
                }
            }
            "rerun-if-env-changed" => env.push(v),
            _ => {}
        }
    }
    inputs.sort();
    inputs.dedup();
    Learned { inputs, env }
}

/// Every job's key under one flag set, without doing anything (None where
/// something is not known yet).
fn keys_for(ctx: &Ctx, chosen: &[String]) -> Vec<Option<String>> {
    let mut ctx = ctx.clone();
    ctx.chosen = chosen.to_vec();
    let mut results: Vec<Option<Result_>> = vec![None; ctx.jobs.len()];
    let mut keys = Vec::new();
    for (i, j) in ctx.jobs.iter().enumerate() {
        let known = j.needs.iter().all(|&x| results[x].is_some());
        let k = if known {
            let (flags, env) = ctx.key_parts(j);
            ctx.lightning.key(&j.id, &flags, &ctx.links(j, &results), &env)
        } else {
            None
        };
        if let Some(k) = &k {
            results[i] = Some(Result_ { key: k.clone(), script: ScriptOut::default() });
        }
        keys.push(k);
    }
    keys
}

impl Clone for Ctx<'_> {
    fn clone(&self) -> Self {
        Ctx { out: self.out.clone(), profile: self.profile.clone(), chosen: self.chosen.clone(), ..*self }
    }
}

pub fn build(pkg: &Package, rules: &Rules, opt: &Options) -> Result<Report, String> {
    let start = Instant::now();
    let profile = *rules.profiles.get(&opt.profile).ok_or(format!(
        "no profile {} in rules.toml (there are: {})",
        opt.profile,
        rules.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
    ))?;
    let target = Target::host()?;
    let g = crate::graph::resolve(&pkg.dir, &target, opt.test)?;
    let jobs = plan(&g, pkg, opt.test)?;
    let lightning = Lightning::open(rustc_version()?);
    let out = pkg.dir.join("strike-out").join(&opt.profile);
    std::fs::create_dir_all(out.join("deps")).map_err(|e| e.to_string())?;
    let base = Ctx { g: &g, target: &target, lightning: &lightning, jobs: &jobs, out: out.clone(), profile: opt.profile.clone(), chosen: Vec::new() };

    // The lightning judge: how many jobs each remembered flag set would strike.
    let mut sets: Vec<Vec<String>> = Vec::new();
    for j in jobs.iter().filter(|j| j.node == g.root) {
        for s in lightning.flag_sets(&j.id) {
            let s: Vec<String> = s.into_iter().filter(|f| !f.starts_with("env ")).collect();
            if !sets.contains(&s) {
                sets.push(s);
            }
        }
    }
    let branded = Branded {
        sets: sets
            .into_iter()
            .map(|s| {
                let n = keys_for(&base, &s).iter().filter(|k| k.as_ref().is_some_and(|k| lightning.strike(k))).count();
                (s, n)
            })
            .filter(|(_, n)| *n > 0)
            .collect(),
        units: jobs.len(),
    };
    let finalists = motor::search(rules, &profile, &branded);
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
    let mut ctx = base.clone();
    ctx.chosen = motor::flags(&chosen.choice);

    // The jobs run as soon as what they need is done.
    struct State {
        results: Vec<Option<Result_>>,
        started: Vec<bool>,
        done: Vec<Done>,
        error: Option<String>,
        running: usize,
    }
    let state = Mutex::new(State {
        results: vec![None; jobs.len()],
        started: vec![false; jobs.len()],
        done: Vec::new(),
        error: None,
        running: 0,
    });
    let wake = Condvar::new();
    std::thread::scope(|s| {
        for _ in 0..opt.jobs.max(1) {
            s.spawn(|| loop {
                let (ji, snapshot) = {
                    let mut st = state.lock().unwrap();
                    loop {
                        if st.error.is_some() {
                            return;
                        }
                        let ready = (0..jobs.len()).find(|&i| !st.started[i] && jobs[i].needs.iter().all(|&x| st.results[x].is_some()));
                        if let Some(i) = ready {
                            st.started[i] = true;
                            st.running += 1;
                            break (i, st.results.clone());
                        }
                        if st.started.iter().all(|s| *s) || (st.running == 0) {
                            return;
                        }
                        st = wake.wait(st).unwrap();
                    }
                };
                let t = Instant::now();
                let r = ctx.run(ji, &snapshot);
                let mut st = state.lock().unwrap();
                st.running -= 1;
                match r {
                    Ok((res, struck)) => {
                        let j = &jobs[ji];
                        let name = match (&j.unit, j.kind) {
                            (Some(u), _) => u.id(),
                            (None, k) => format!("{} {} ({:?})", g.nodes[j.node].name, g.nodes[j.node].version, k),
                        };
                        st.done.push(Done { job: name, root: j.node == g.root, struck, seconds: t.elapsed().as_secs_f64(), key: res.key.clone() });
                        st.results[ji] = Some(res);
                    }
                    Err(e) => st.error = Some(e),
                }
                wake.notify_all();
            });
        }
    });
    let st = state.into_inner().unwrap();
    lightning.save()?;
    if let Some(e) = st.error {
        return Err(e);
    }
    if let Some(i) = st.results.iter().position(|r| r.is_none()) {
        return Err(format!("{} never ran", jobs[i].id));
    }

    // The check: what each root unit makes is there; a test must pass.
    let mut tests = Vec::new();
    if let Some(p) = ctx.lib_file(&g.nodes[g.root]) {
        if !p.exists() {
            return Err(format!("check failed: {} was not made", p.display()));
        }
    }
    for j in jobs.iter().filter(|j| j.kind == JobKind::Root) {
        let u = j.unit.as_ref().unwrap();
        let exe = out.join(format!("{}{}", u.out_name, std::env::consts::EXE_SUFFIX));
        if !exe.exists() {
            return Err(format!("check failed: {} was not made", exe.display()));
        }
        if u.kind == Kind::Test {
            let status = Command::new(&exe).current_dir(&pkg.dir).status().map_err(|e| format!("{}: {e}", exe.display()))?;
            tests.push((u.out_name.clone(), status.success()));
        }
    }
    let mut done = st.done;
    done.sort_by(|a, b| b.root.cmp(&a.root).then(a.job.cmp(&b.job)));
    Ok(Report { chosen, done, out, seconds: start.elapsed().as_secs_f64(), tests })
}
