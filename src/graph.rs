//! Steps 2 and 3: the package graph from Cargo.lock and the sources already
//! on this machine, with the features every package ends up with.
//!
//! Features are unified over the whole graph (Cargo's first resolver): a
//! package built once for everyone who uses it gets every feature any of them
//! asked for.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Dep {
    /// The name the code uses (`extern crate` name, dashes turned).
    pub extern_name: String,
    /// The package's name in the lockfile.
    pub package: String,
    pub req: String,
    pub features: Vec<String>,
    pub default_features: bool,
    pub optional: bool,
    pub path: Option<PathBuf>,
    pub kind: DepKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Build,
    Dev,
}

#[derive(Debug)]
pub struct Node {
    pub name: String,
    pub version: String,
    pub dir: PathBuf,
    pub edition: String,
    pub manifest: toml::Table,
    /// The library target: crate name and root file.
    pub lib: Option<(String, PathBuf)>,
    pub proc_macro: bool,
    pub build_script: Option<PathBuf>,
    pub links: Option<String>,
    pub deps: Vec<Dep>,
    pub features: BTreeSet<String>,
    /// The dependencies in use (optional ones only once a feature turns them
    /// on), as indices into the graph: (dep, node).
    pub active: Vec<(Dep, usize)>,
}

pub struct Graph {
    pub nodes: Vec<Node>,
    pub root: usize,
}

/// What rustc says of the target: `windows`, `target_os="windows"`, ...
pub struct Target {
    pub triple: String,
    pub cfg: BTreeSet<(String, Option<String>)>,
}

impl Target {
    pub fn host() -> Result<Target, String> {
        let out = std::process::Command::new("rustc").args(["--print", "cfg"]).output().map_err(|e| e.to_string())?;
        let mut cfg = BTreeSet::new();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            match line.split_once('=') {
                Some((k, v)) => cfg.insert((k.to_string(), Some(v.trim_matches('"').to_string()))),
                None => cfg.insert((line.to_string(), None)),
            };
        }
        let vv = std::process::Command::new("rustc").arg("-vV").output().map_err(|e| e.to_string())?;
        let triple = String::from_utf8_lossy(&vv.stdout)
            .lines()
            .find_map(|l| l.strip_prefix("host: ").map(str::to_string))
            .ok_or("rustc -vV gives no host")?;
        Ok(Target { triple, cfg })
    }

    /// Does `[target.'<spec>'.dependencies]` apply here?
    pub fn matches(&self, spec: &str) -> Result<bool, String> {
        match spec.strip_prefix("cfg(").and_then(|s| s.strip_suffix(')')) {
            Some(expr) => {
                let tokens = tokenize(expr);
                let mut at = 0;
                let v = self.eval(&tokens, &mut at)?;
                Ok(v)
            }
            None => Ok(spec == self.triple),
        }
    }

    fn eval(&self, t: &[String], at: &mut usize) -> Result<bool, String> {
        let word = t.get(*at).ok_or("cfg: ends too soon")?.clone();
        *at += 1;
        if matches!(word.as_str(), "all" | "any" | "not") && t.get(*at).map(String::as_str) == Some("(") {
            *at += 1;
            let mut vals = Vec::new();
            while t.get(*at).map(String::as_str) != Some(")") {
                vals.push(self.eval(t, at)?);
                if t.get(*at).map(String::as_str) == Some(",") {
                    *at += 1;
                }
            }
            *at += 1;
            return match word.as_str() {
                "all" => Ok(vals.iter().all(|v| *v)),
                "any" => Ok(vals.iter().any(|v| *v)),
                _ => Ok(!vals.first().copied().ok_or("cfg: not() is empty")?),
            };
        }
        if t.get(*at).map(String::as_str) == Some("=") {
            *at += 1;
            let value = t.get(*at).ok_or("cfg: no value after =")?.trim_matches('"').to_string();
            *at += 1;
            return Ok(self.cfg.contains(&(word, Some(value))));
        }
        Ok(self.cfg.contains(&(word, None)))
    }
}

fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if "(),=".contains(c) {
            out.push(c.to_string());
            chars.next();
        } else if c == '"' {
            let mut w = String::from('"');
            chars.next();
            for c in chars.by_ref() {
                w.push(c);
                if c == '"' {
                    break;
                }
            }
            out.push(w);
        } else {
            let mut w = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || "(),=\"".contains(c) {
                    break;
                }
                w.push(c);
                chars.next();
            }
            out.push(w);
        }
    }
    out
}

fn str_of<'a>(t: &'a toml::Table, key: &str) -> Option<&'a str> {
    t.get(key).and_then(|v| v.as_str())
}

/// The dependency tables of a manifest that apply to this target.
fn deps_of(m: &toml::Table, dir: &Path, target: &Target) -> Result<Vec<Dep>, String> {
    let mut out = Vec::new();
    let mut read = |table: &toml::Table, kind: DepKind| -> Result<(), String> {
        for (key, v) in table {
            let (req, t) = match v {
                toml::Value::String(s) => (s.clone(), None),
                toml::Value::Table(t) => (str_of(t, "version").unwrap_or("*").to_string(), Some(t)),
                _ => return Err(format!("dependency {key}: neither a version nor a table")),
            };
            let get_bool = |k: &str| t.and_then(|t| t.get(k)).and_then(|v| v.as_bool());
            let package = t.and_then(|t| str_of(t, "package")).unwrap_or(key).to_string();
            out.push(Dep {
                extern_name: key.replace('-', "_"),
                package,
                req,
                features: t
                    .and_then(|t| t.get("features"))
                    .and_then(|f| f.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default(),
                default_features: get_bool("default-features").or(get_bool("default_features")).unwrap_or(true),
                optional: get_bool("optional").unwrap_or(false),
                path: t.and_then(|t| str_of(t, "path")).map(|p| dir.join(p)),
                kind,
            });
        }
        Ok(())
    };
    let kinds = [
        ("dependencies", DepKind::Normal),
        ("build-dependencies", DepKind::Build),
        ("build_dependencies", DepKind::Build),
        ("dev-dependencies", DepKind::Dev),
        ("dev_dependencies", DepKind::Dev),
    ];
    for (name, kind) in kinds {
        if let Some(t) = m.get(name).and_then(|d| d.as_table()) {
            read(t, kind)?;
        }
    }
    if let Some(targets) = m.get("target").and_then(|t| t.as_table()) {
        for (spec, t) in targets {
            if !target.matches(spec)? {
                continue;
            }
            let Some(t) = t.as_table() else { continue };
            for (name, kind) in kinds {
                if let Some(d) = t.get(name).and_then(|d| d.as_table()) {
                    read(d, kind)?;
                }
            }
        }
    }
    Ok(out)
}

fn node_from(dir: &Path, target: &Target) -> Result<Node, String> {
    let path = dir.join("Cargo.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let m: toml::Table = text.parse().map_err(|e| format!("{}: {e}", path.display()))?;
    let pkg = m.get("package").and_then(|p| p.as_table()).ok_or(format!("{}: no [package]", path.display()))?;
    let name = str_of(pkg, "name").ok_or("a package has no name")?.to_string();
    let version = str_of(pkg, "version").unwrap_or("0.0.0").to_string();
    let edition = str_of(pkg, "edition").unwrap_or("2015").to_string();
    let lib_t = m.get("lib").and_then(|l| l.as_table());
    let lib_root = dir.join(lib_t.and_then(|l| str_of(l, "path")).unwrap_or("src/lib.rs"));
    let lib_name = lib_t.and_then(|l| str_of(l, "name")).map(str::to_string).unwrap_or_else(|| name.replace('-', "_"));
    let proc_macro = lib_t
        .and_then(|l| l.get("proc-macro").or(l.get("proc_macro")))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let build_script = match pkg.get("build") {
        Some(toml::Value::String(s)) => Some(dir.join(s)),
        Some(toml::Value::Boolean(false)) => None,
        _ => dir.join("build.rs").exists().then(|| dir.join("build.rs")),
    };
    Ok(Node {
        lib: lib_root.exists().then_some((lib_name.replace('-', "_"), lib_root)),
        proc_macro,
        build_script,
        links: str_of(pkg, "links").map(str::to_string),
        deps: deps_of(&m, dir, target)?,
        name,
        version,
        dir: dir.to_path_buf(),
        edition,
        manifest: m,
        features: BTreeSet::new(),
        active: Vec::new(),
    })
}

/// A locked package: name, version, source, and its locked dependencies
/// ("name" or "name version").
struct Locked {
    name: String,
    version: String,
    registry: bool,
    deps: Vec<String>,
}

fn read_lock(dir: &Path) -> Result<Vec<Locked>, String> {
    let path = dir.join("Cargo.lock");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("{}: {e} (Strike reads the versions from Cargo.lock; it does not resolve them)", path.display()))?;
    let doc: toml::Table = text.parse().map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    for p in doc.get("package").and_then(|p| p.as_array()).into_iter().flatten() {
        let Some(t) = p.as_table() else { continue };
        out.push(Locked {
            name: str_of(t, "name").unwrap_or_default().to_string(),
            version: str_of(t, "version").unwrap_or_default().to_string(),
            registry: str_of(t, "source").is_some_and(|s| s.starts_with("registry+") || s.starts_with("sparse+")),
            deps: t
                .get("dependencies")
                .and_then(|d| d.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
        });
    }
    Ok(out)
}

/// The folders the registry's sources were unpacked to: fStrike's first,
/// then Cargo's.
fn registry_dirs() -> Vec<PathBuf> {
    let base = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").or(std::env::var_os("HOME")).map(|h| PathBuf::from(h).join(".cargo")))
        .unwrap_or_default()
        .join("registry")
        .join("src");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(base).into_iter().flatten().flatten().map(|e| e.path()).collect();
    dirs.sort();
    dirs.insert(0, crate::fstrike::registry_dir());
    dirs
}

/// Is `version` the one a parent locked for `name`? A lock entry names the
/// version only when two versions of the package are in the graph.
fn lock_pick<'a>(lock: &'a [Locked], parent: &Locked, dep: &Dep) -> Option<&'a Locked> {
    let candidates: Vec<&str> = parent
        .deps
        .iter()
        .filter(|d| d.split(' ').next() == Some(dep.package.as_str()))
        .map(String::as_str)
        .collect();
    let version = match candidates.as_slice() {
        [one] => one.split(' ').nth(1).map(str::to_string),
        [] => return None,
        many => {
            // Two versions locked: the one whose leading numbers the
            // requirement names.
            let want: Vec<&str> = dep.req.trim_start_matches(['^', '=', '~', ' ']).split('.').collect();
            let lead = if want.first() == Some(&"0") { 2 } else { 1 };
            many.iter()
                .filter_map(|c| c.split(' ').nth(1))
                .find(|v| v.split('.').take(lead).eq(want.iter().take(lead).copied()))
                .map(str::to_string)
        }
    };
    lock.iter().find(|l| l.name == dep.package && version.as_ref().is_none_or(|v| &l.version == v))
}

pub fn resolve(root_dir: &Path, target: &Target, with_dev: bool) -> Result<Graph, String> {
    let lock = read_lock(root_dir)?;
    let registries = registry_dirs();
    let root_node = node_from(root_dir, target)?;
    let root_locked = lock
        .iter()
        // By name: a version just bumped in Cargo.toml is still this package.
        .position(|l| l.name == root_node.name && !l.registry)
        .ok_or("the package is not in its own Cargo.lock")?;

    let mut nodes = vec![root_node];
    let mut lock_of = vec![root_locked];
    let mut index: BTreeMap<usize, usize> = BTreeMap::from([(root_locked, 0)]);

    // Find (or load) the node a dependency points at.
    let mut node_for = |nodes: &mut Vec<Node>, lock_of: &mut Vec<usize>, parent: usize, dep: &Dep| -> Result<usize, String> {
        let parent_lock = &lock[lock_of[parent]];
        let l = lock_pick(&lock, parent_lock, dep)
            .ok_or(format!("{} needs {}, which Cargo.lock does not have (run cargo generate-lockfile once)", nodes[parent].name, dep.package))?;
        let li = lock.iter().position(|x| std::ptr::eq(x, l)).unwrap();
        if let Some(&n) = index.get(&li) {
            return Ok(n);
        }
        let dir = if l.registry {
            registries
                .iter()
                .map(|r| r.join(format!("{}-{}", l.name, l.version)))
                .find(|d| d.join("Cargo.toml").exists())
                .ok_or(format!("{} {} is not downloaded (strike fetch once)", l.name, l.version))?
        } else {
            dep.path.clone().ok_or(format!("{}: not from the registry and no path", l.name))?
        };
        nodes.push(node_from(&dir, target)?);
        lock_of.push(li);
        index.insert(li, nodes.len() - 1);
        Ok(nodes.len() - 1)
    };

    // Turn on what the root asks for, then go round until nothing changes.
    nodes[0].features.insert("default".to_string());
    loop {
        let mut changed = false;
        let mut i = 0;
        while i < nodes.len() {
            // The features this node has turn on features and optional deps.
            let table = nodes[i].manifest.get("features").and_then(|f| f.as_table()).cloned().unwrap_or_default();
            let mut on_deps: BTreeSet<String> = BTreeSet::new();
            let mut dep_features: Vec<(String, String, bool)> = Vec::new();
            let mut todo: Vec<String> = nodes[i].features.iter().cloned().collect();
            let mut seen: BTreeSet<String> = BTreeSet::new();
            while let Some(f) = todo.pop() {
                if !seen.insert(f.clone()) {
                    continue;
                }
                match table.get(&f).and_then(|v| v.as_array()) {
                    Some(items) => {
                        for it in items.iter().filter_map(|x| x.as_str()) {
                            if let Some(d) = it.strip_prefix("dep:") {
                                on_deps.insert(d.to_string());
                            } else if let Some((d, df)) = it.split_once('/') {
                                let weak = d.ends_with('?');
                                let d = d.trim_end_matches('?');
                                if !weak {
                                    on_deps.insert(d.to_string());
                                }
                                dep_features.push((d.to_string(), df.to_string(), weak));
                            } else {
                                todo.push(it.to_string());
                            }
                        }
                    }
                    // A feature that is not in the table is an optional dependency's own.
                    None => {
                        on_deps.insert(f.clone());
                    }
                }
            }
            for f in &seen {
                if nodes[i].features.insert(f.clone()) {
                    changed = true;
                }
            }

            let deps = nodes[i].deps.clone();
            for dep in deps {
                if dep.kind == DepKind::Dev && !(i == 0 && with_dev) {
                    continue;
                }
                let key = dep.extern_name.replace('_', "-");
                let named = |n: &str| n == dep.extern_name || n == key || n.replace('-', "_") == dep.extern_name;
                if dep.optional && !on_deps.iter().any(|n| named(n)) {
                    continue;
                }
                let n = node_for(&mut nodes, &mut lock_of, i, &dep)?;
                if !nodes[i].active.iter().any(|(d, x)| *x == n && d.extern_name == dep.extern_name && d.kind == dep.kind) {
                    nodes[i].active.push((dep.clone(), n));
                    changed = true;
                }
                let mut want: Vec<String> = dep.features.clone();
                if dep.default_features {
                    want.push("default".to_string());
                }
                for (d, f, _) in &dep_features {
                    if named(d) {
                        want.push(f.clone());
                    }
                }
                for f in want {
                    if nodes[n].features.insert(f) {
                        changed = true;
                    }
                }
            }
            i += 1;
        }
        if !changed {
            break;
        }
    }
    // "default" is only a feature when the package has one.
    for n in &mut nodes {
        let has = n.manifest.get("features").and_then(|f| f.as_table()).is_some_and(|t| t.contains_key("default"));
        if !has {
            n.features.remove("default");
        }
        // An optional dependency's own name is a feature only while it is on.
        let declared: BTreeSet<String> =
            n.manifest.get("features").and_then(|f| f.as_table()).map(|t| t.keys().cloned().collect()).unwrap_or_default();
        let actives: BTreeSet<String> = n.active.iter().map(|(d, _)| d.extern_name.clone()).collect();
        let optional: BTreeSet<String> = n.deps.iter().filter(|d| d.optional).map(|d| d.extern_name.replace('_', "-")).collect();
        n.features.retain(|f| {
            declared.contains(f)
                || (optional.contains(&f.replace('_', "-")) && actives.contains(&f.replace('-', "_")))
        });
    }
    Ok(Graph { nodes, root: 0 })
}

impl Graph {
    /// The nodes in an order where everything a node uses comes first.
    pub fn order(&self) -> Result<Vec<usize>, String> {
        let mut state = vec![0u8; self.nodes.len()];
        let mut out = Vec::new();
        fn visit(g: &Graph, i: usize, state: &mut [u8], out: &mut Vec<usize>) -> Result<(), String> {
            match state[i] {
                2 => return Ok(()),
                1 => return Err(format!("a cycle through {}", g.nodes[i].name)),
                _ => {}
            }
            state[i] = 1;
            for (_, n) in &g.nodes[i].active {
                if *n != i {
                    visit(g, *n, state, out)?;
                }
            }
            state[i] = 2;
            out.push(i);
            Ok(())
        }
        visit(self, self.root, &mut state, &mut out)?;
        Ok(out)
    }
}
