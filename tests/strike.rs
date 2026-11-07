//! Strike on small packages written here, each with its own brand store.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Pkg {
    dir: PathBuf,
    home: PathBuf,
}

fn pkg(name: &str) -> Pkg {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("pkg");
    for d in ["src/bin", "tests"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    write(&dir, "Cargo.toml", "[package]\nname = \"demo-pkg\"\nversion = \"0.3.1\"\nedition = \"2021\"\n");
    write(&dir, "src/lib.rs", "pub mod sum;\npub fn version() -> &'static str { env!(\"CARGO_PKG_VERSION\") }\n#[cfg(test)] mod t { #[test] fn adds() { assert_eq!(crate::sum::sum(&[1, 2, 3]), 6); } }\n");
    write(&dir, "src/sum.rs", "pub fn sum(x: &[i64]) -> i64 { x.iter().sum() }\n");
    write(&dir, "src/main.rs", "fn main() { println!(\"demo {} sum {}\", demo_pkg::version(), demo_pkg::sum::sum(&[4, 5])); }\n");
    write(&dir, "src/bin/other-tool.rs", "fn main() { println!(\"other {}\", demo_pkg::sum::sum(&[1])); }\n");
    write(&dir, "tests/outer.rs", "#[test] fn outer() { assert_eq!(demo_pkg::version(), \"0.3.1\"); }\n");
    write(&dir, "Cargo.lock", "version = 3\n\n[[package]]\nname = \"demo-pkg\"\nversion = \"0.3.1\"\n");
    Pkg { dir, home: root.join("home") }
}

fn write(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).unwrap();
}

impl Pkg {
    fn strike(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_strike"))
            .args(args)
            .arg(&self.dir)
            .env("STRIKE_HOME", &self.home)
            .output()
            .unwrap()
    }

    /// (units struck, units built) from a build's report.
    fn counts(&self, args: &[&str]) -> (usize, usize, String) {
        let out = self.strike(args);
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(out.status.success(), "{text}\n{}", String::from_utf8_lossy(&out.stderr));
        let struck = text.lines().filter(|l| l.trim_start().starts_with("struck")).count();
        let built = text.lines().filter(|l| l.trim_start().starts_with("built ")).count();
        (struck, built, text)
    }

    fn run(&self, profile: &str, exe: &str) -> String {
        let path = self.dir.join("strike-out").join(profile).join(format!("{exe}{}", std::env::consts::EXE_SUFFIX));
        String::from_utf8_lossy(&Command::new(path).output().unwrap().stdout).trim().to_string()
    }
}

#[test]
fn second_build_is_struck_and_the_programs_run() {
    let p = pkg("second");
    assert_eq!(p.counts(&["build"]).1, 3);
    assert_eq!(p.counts(&["build"]).0, 3);
    assert_eq!(p.run("build", "demo-pkg"), "demo 0.3.1 sum 9");
    assert_eq!(p.run("build", "other-tool"), "other 1");
}

#[test]
fn a_new_time_is_not_a_change_but_new_content_is() {
    let p = pkg("content");
    p.counts(&["build"]);
    // The same content written again: only the file's time changes.
    write(&p.dir, "src/sum.rs", "pub fn sum(x: &[i64]) -> i64 { x.iter().sum() }\n");
    assert_eq!(p.counts(&["build"]).0, 3);
    // A bin of its own: only that bin is built.
    write(&p.dir, "src/bin/other-tool.rs", "fn main() { println!(\"other {}\", demo_pkg::sum::sum(&[1, 1])); }\n");
    let (struck, built, text) = p.counts(&["build"]);
    assert_eq!((struck, built), (2, 1), "{text}");
    assert!(text.contains("built  Bin:other-tool"), "{text}");
    assert_eq!(p.run("build", "other-tool"), "other 2");
    // The library: it and everything linking it.
    write(&p.dir, "src/sum.rs", "pub fn sum(x: &[i64]) -> i64 { x.iter().sum::<i64>() * 10 }\n");
    assert_eq!(p.counts(&["build"]).1, 3);
    assert_eq!(p.run("build", "demo-pkg"), "demo 0.3.1 sum 90");
}

#[test]
fn an_environment_variable_read_is_part_of_the_key() {
    let p = pkg("env");
    p.counts(&["build"]);
    write(&p.dir, "Cargo.toml", "[package]\nname = \"demo-pkg\"\nversion = \"0.3.2\"\nedition = \"2021\"\n");
    assert_eq!(p.counts(&["build"]).1, 3);
    assert_eq!(p.run("build", "demo-pkg"), "demo 0.3.2 sum 9");
}

#[test]
fn a_brand_does_not_outweigh_asking_for_a_fast_program() {
    let p = pkg("profile");
    let (_, _, text) = p.counts(&["build"]);
    assert!(text.contains("opt_level=0"), "{text}");
    let (_, _, text) = p.counts(&["build", "--for", "run", "--explain"]);
    assert!(text.contains("chose opt_level=3"), "{text}");
    // Back to a quick build: the opt-level 0 brand is struck.
    assert_eq!(p.counts(&["build", "--explain"]).0, 3);
}

#[test]
fn tests_run_and_a_failing_one_fails_the_command() {
    let p = pkg("tests");
    let (_, _, text) = p.counts(&["test"]);
    assert!(text.contains("tests: 2 of 2 harnesses passed"), "{text}");
    write(&p.dir, "tests/outer.rs", "#[test] fn outer() { assert_eq!(demo_pkg::version(), \"9.9.9\"); }\n");
    let out = p.strike(&["test"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("tests: 1 of 2 harnesses passed"));
}

/// A package with a path dependency (and a feature of it), a proc macro and
/// a build script that writes code into OUT_DIR and sets a cfg.
fn graph_pkg(name: &str) -> Pkg {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    for d in ["app/src", "helper/src", "mac/src"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let app = root.join("app");
    write(&app, "Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\nbuild = \"build.rs\"\n\n[dependencies]\nhelper = { path = \"../helper\", features = [\"loud\"] }\nmac = { path = \"../mac\" }\n");
    write(&app, "build.rs", "fn main() {\n    let out = std::env::var(\"OUT_DIR\").unwrap();\n    std::fs::write(format!(\"{out}/gen.rs\"), \"pub const MADE: u32 = 7;\").unwrap();\n    println!(\"cargo:rustc-cfg=from_script\");\n    println!(\"cargo:rerun-if-changed=build.rs\");\n}\n");
    write(&app, "src/main.rs", "include!(concat!(env!(\"OUT_DIR\"), \"/gen.rs\"));\nfn main() {\n    let s = if cfg!(from_script) { \"cfg\" } else { \"none\" };\n    println!(\"{} {} {} {}\", helper::word(), mac::answer!(), MADE, s);\n}\n");
    write(&root.join("helper"), "Cargo.toml", "[package]\nname = \"helper\"\nversion = \"0.2.0\"\nedition = \"2021\"\n\n[features]\nloud = []\n");
    write(&root.join("helper"), "src/lib.rs", "pub fn word() -> &'static str { if cfg!(feature = \"loud\") { \"LOUD\" } else { \"quiet\" } }\n");
    write(&root.join("mac"), "Cargo.toml", "[package]\nname = \"mac\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nproc-macro = true\n");
    write(&root.join("mac"), "src/lib.rs", "use proc_macro::TokenStream;\n#[proc_macro]\npub fn answer(_: TokenStream) -> TokenStream { \"42\".parse().unwrap() }\n");
    write(&app, "Cargo.lock", "version = 3\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\ndependencies = [\n \"helper\",\n \"mac\",\n]\n\n[[package]]\nname = \"helper\"\nversion = \"0.2.0\"\n\n[[package]]\nname = \"mac\"\nversion = \"0.1.0\"\n");
    Pkg { dir: app, home: root.join("home") }
}

#[test]
fn dependencies_features_proc_macros_and_build_scripts() {
    let p = graph_pkg("graph");
    let (_, built, text) = p.counts(&["build", "--verbose"]);
    // helper, mac, the script's build and run, app's bin: 5 jobs.
    assert_eq!(built, 5, "{text}");
    assert_eq!(p.run("build", "app"), "LOUD 42 7 cfg");
    assert_eq!(p.counts(&["build", "--verbose"]).0, 5);
    // The build script changes: it runs again, and what it made is used.
    let script = p.dir.join("build.rs");
    let s = std::fs::read_to_string(&script).unwrap().replace("MADE: u32 = 7", "MADE: u32 = 8");
    std::fs::write(&script, s).unwrap();
    let (struck, built, text) = p.counts(&["build", "--verbose"]);
    assert_eq!((struck, built), (2, 3), "{text}");
    assert_eq!(p.run("build", "app"), "LOUD 42 8 cfg");
}

#[test]
fn a_package_missing_from_the_lockfile_is_refused_with_why() {
    let p = graph_pkg("nolock");
    write(&p.dir, "Cargo.lock", "version = 3\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\ndependencies = [\n \"mac\",\n]\n\n[[package]]\nname = \"mac\"\nversion = \"0.1.0\"\n");
    let out = p.strike(&["build"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("app needs helper, which Cargo.lock does not have"));
}

#[test]
fn a_typo_in_the_rules_fails_at_load() {
    let p = pkg("rules");
    let bad = include_str!("../rules.toml").replace("[rule.debuginfo]", "[rule.debuginf]");
    let file = p.dir.join("bad-rules.toml");
    std::fs::write(&file, bad).unwrap();
    let out = p.strike(&["build", "--rules", file.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("rules.toml: no [rule.debuginfo]"));
}
