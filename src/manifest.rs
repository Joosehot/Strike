//! Step 1: reading the package's Cargo.toml into the units Strike builds.

use std::path::{Path, PathBuf};

/// What one rustc call makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Lib,
    Bin,
    /// A test harness: the library's own tests, or one file of tests/.
    Test,
}

#[derive(Clone, Debug)]
pub struct Unit {
    /// The crate name rustc sees (dashes turned to underscores).
    pub crate_name: String,
    /// The file name of what it makes (a bin keeps its dashes).
    pub out_name: String,
    pub kind: Kind,
    pub root: PathBuf,
    /// A bin or a test of tests/ links against the package's library.
    pub uses_lib: bool,
}

impl Unit {
    /// Names the unit apart from every other unit of the package.
    pub fn id(&self) -> String {
        format!("{:?}:{}", self.kind, self.out_name)
    }
}

#[derive(Debug)]
pub struct Package {
    pub dir: PathBuf,
    pub name: String,
    pub lib: Option<Unit>,
    pub bins: Vec<Unit>,
    /// tests/*.rs, each its own harness.
    pub tests: Vec<Unit>,
}

pub fn read(dir: &Path) -> Result<Package, String> {
    let dir = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    // Windows gives canonical paths as \\?\C:\...; the plain form is the same place.
    let dir = match dir.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(plain) if plain.as_bytes().get(1) == Some(&b':') => PathBuf::from(plain),
        _ => dir,
    };
    let path = dir.join("Cargo.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc: toml::Table = text.parse().map_err(|e| format!("{}: {e}", path.display()))?;

    let pkg = doc
        .get("package")
        .and_then(|p| p.as_table())
        .ok_or("Cargo.toml has no [package]")?;
    let name = pkg
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("[package] has no name")?
        .to_string();

    let crate_name = name.replace('-', "_");
    let lib_path = doc
        .get("lib")
        .and_then(|l| l.get("path"))
        .and_then(|p| p.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("src/lib.rs"));
    let lib = dir.join(&lib_path).exists().then(|| Unit {
        crate_name: crate_name.clone(),
        out_name: crate_name.clone(),
        kind: Kind::Lib,
        root: dir.join(&lib_path),
        uses_lib: false,
    });

    let bin = |out: &str, root: PathBuf| Unit {
        crate_name: out.replace('-', "_"),
        out_name: out.to_string(),
        kind: Kind::Bin,
        root,
        uses_lib: lib.is_some(),
    };
    let mut bins = Vec::new();
    if let Some(list) = doc.get("bin").and_then(|b| b.as_array()) {
        for b in list {
            let out = b.get("name").and_then(|v| v.as_str()).ok_or("a [[bin]] has no name")?;
            let root = match b.get("path").and_then(|v| v.as_str()) {
                Some(p) => dir.join(p),
                None if out == name => dir.join("src/main.rs"),
                None => dir.join("src/bin").join(format!("{out}.rs")),
            };
            bins.push(bin(out, root));
        }
    }
    if dir.join("src/main.rs").exists() && !bins.iter().any(|b| b.root == dir.join("src/main.rs")) {
        bins.push(bin(&name, dir.join("src/main.rs")));
    }
    for root in rs_files(&dir.join("src/bin")) {
        let out = root.file_stem().unwrap().to_string_lossy().to_string();
        if !bins.iter().any(|b| b.root == root || b.out_name == out) {
            bins.push(bin(&out, root));
        }
    }

    let mut tests = Vec::new();
    for root in rs_files(&dir.join("tests")) {
        let out = root.file_stem().unwrap().to_string_lossy().to_string();
        tests.push(Unit {
            crate_name: out.replace('-', "_"),
            out_name: out,
            kind: Kind::Test,
            root,
            uses_lib: lib.is_some(),
        });
    }

    Ok(Package { dir, name, lib, bins, tests })
}

/// The .rs files directly in a folder, in name order.
fn rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.sort();
    files
}
