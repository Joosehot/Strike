//! The JVM driver: the Java and Kotlin compile tasks of a build, read from
//! its export (one JSON file per project, written once by the build tool,
//! as a lockfile is), compiled with javac and kotlinc under the same options,
//! every task struck or branded by the lightning.
//!
//! Strike writes to its own folder: every output of the export is moved
//! there, and so is every path to it in a classpath or an option.

use crate::lightning::{self, hex, Learned, Lightning};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Condvar, Mutex};
use std::time::Instant;

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct Task {
    project: String,
    task: String,
    kind: String,
    #[serde(default)]
    depends_on: Vec<String>,
    sources: Vec<String>,
    #[serde(default)]
    java_sources: Vec<String>,
    classpath: Vec<String>,
    #[serde(default)]
    args: Vec<String>,
    release: Option<u32>,
    encoding: Option<String>,
    #[serde(default)]
    infer_module_path: bool,
    output: String,
    java_home: Option<String>,
    jvm_target: Option<String>,
    api_version: Option<String>,
    language_version: Option<String>,
    module_name: Option<String>,
    #[serde(default)]
    compiler_classpath: Vec<String>,
}

impl Task {
    fn name(&self) -> String {
        format!("{}:{}", self.project.trim_end_matches(':'), self.task)
    }
}

pub struct Done {
    pub task: String,
    pub struck: bool,
    pub seconds: f64,
}

pub struct Report {
    pub done: Vec<Done>,
    pub seconds: f64,
    pub out: PathBuf,
}

fn read_export(dir: &Path) -> Result<Vec<Task>, String> {
    let mut tasks = Vec::new();
    for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten() {
        if e.path().extension().is_some_and(|x| x == "json") {
            let text = std::fs::read_to_string(e.path()).map_err(|x| x.to_string())?;
            let list: Vec<Task> = serde_json::from_str(&text).map_err(|x| format!("{}: {x}", e.path().display()))?;
            // A task with nothing to compile is no work.
            tasks.extend(list.into_iter().filter(|t| !t.sources.is_empty()));
        }
    }
    tasks.sort_by_key(|t| t.name());
    Ok(tasks)
}

/// The version of a tool, for the key: a new compiler is a new build.
fn version_of(cmd: &mut Command) -> String {
    cmd.output()
        .map(|o| format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
        .unwrap_or_default()
}

fn java_bin(home: &Option<String>, tool: &str, fallback: &Path) -> PathBuf {
    let base = home.as_ref().map(PathBuf::from).unwrap_or_else(|| fallback.to_path_buf());
    base.join("bin").join(format!("{tool}{}", std::env::consts::EXE_SUFFIX))
}

pub fn build(export: &Path, out: &Path, jobs: usize) -> Result<Report, String> {
    let start = Instant::now();
    let tasks = read_export(export)?;
    if tasks.is_empty() {
        return Err(format!("no compile tasks in {}", export.display()));
    }
    let default_home = PathBuf::from(std::env::var("JAVA_HOME").map_err(|_| "JAVA_HOME is not set")?);

    // Strike's own place for every output of the export.
    let place: BTreeMap<String, PathBuf> = tasks
        .iter()
        .map(|t| (t.output.clone(), out.join(t.project.trim_start_matches(':').replace(':', "_")).join(&t.task)))
        .collect();
    // Longest first: one output's path can begin another's (main, mainRelease9).
    let mut by_length: Vec<(&String, &PathBuf)> = place.iter().collect();
    by_length.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
    let remap = |s: &str| -> String {
        let mut s = s.to_string();
        for (from, to) in &by_length {
            s = s.replace(from.as_str(), &to.display().to_string());
        }
        s
    };

    // A task needs another when its classpath or options name the other's
    // output, or the build says it comes first.
    let mut needs: Vec<Vec<usize>> = vec![Vec::new(); tasks.len()];
    for (i, t) in tasks.iter().enumerate() {
        for (k, o) in tasks.iter().enumerate() {
            if i == k {
                continue;
            }
            // A path in an option names an output only as a whole entry.
            let in_args = t.args.iter().any(|a| {
                a.split(';').any(|part| part.split_once('=').map(|(_, p)| p).unwrap_or(part) == o.output)
            });
            let named = t.classpath.iter().any(|c| c == &o.output)
                || in_args
                || t.depends_on.iter().any(|d| d.trim_start_matches(':') == o.name().trim_start_matches(':'));
            if named {
                needs[i].push(k);
            }
        }
    }

    let mut versions: BTreeMap<String, String> = BTreeMap::new();
    for t in &tasks {
        let javac = java_bin(&t.java_home, "javac", &default_home);
        versions
            .entry(javac.display().to_string())
            .or_insert_with(|| version_of(Command::new(&javac).arg("-version")));
    }
    let lightning = Lightning::open(format!("strike-jvm-1\n{versions:?}"));

    struct State {
        keys: Vec<Option<String>>,
        started: Vec<bool>,
        done: Vec<Done>,
        error: Option<String>,
        running: usize,
    }
    let state = Mutex::new(State { keys: vec![None; tasks.len()], started: vec![false; tasks.len()], done: Vec::new(), error: None, running: 0 });
    let wake = Condvar::new();

    let run = |i: usize, links: Vec<String>| -> Result<(String, bool), String> {
        let t = &tasks[i];
        let id = format!("jvm|{}|{}", export.display(), t.name());
        let dest = &place[&t.output];
        // As Gradle does, a classpath entry that does not exist (the output
        // of a task with no sources) is left out.
        let classpath: Vec<String> = t
            .classpath
            .iter()
            .map(|c| remap(c))
            .filter(|c| Path::new(c).exists())
            .collect();
        let args: Vec<String> = t.args.iter().map(|a| remap(a)).collect();

        // What decides the output, besides the files: the tool and options.
        let mut flags = vec![format!("kind {}", t.kind)];
        flags.extend(args.iter().map(|a| format!("arg {a}")));
        flags.push(format!("release {:?} encoding {:?} modules {}", t.release, t.encoding, t.infer_module_path));
        flags.push(format!("kotlin {:?} {:?} {:?} {:?}", t.jvm_target, t.api_version, t.language_version, t.module_name));
        flags.push(format!("javac {}", java_bin(&t.java_home, "javac", &default_home).display()));
        // Every file read: the sources, and every jar or folder of the
        // classpath that no task of this build makes.
        let mut inputs: Vec<PathBuf> = t.sources.iter().chain(&t.java_sources).map(PathBuf::from).collect();
        for c in &t.classpath {
            if place.contains_key(c) {
                continue;
            }
            let p = PathBuf::from(c);
            if p.is_dir() {
                inputs.extend(lightning::files_under(&p));
            } else if p.exists() {
                inputs.push(p);
            }
        }
        if t.kind == "kotlin" {
            inputs.extend(t.compiler_classpath.iter().map(PathBuf::from));
        }
        // Paths in the options are read too: other modules' module-info
        // through --module-source-path, sources through --patch-module.
        for w in t.args.windows(2) {
            if matches!(w[0].as_str(), "--module-source-path" | "--patch-module" | "--module-path" | "-classpath" | "--class-path" | "-cp") {
                for part in w[1].split(';') {
                    let path = part.split_once('=').map(|(_, p)| p).unwrap_or(part);
                    if place.contains_key(path) || path.is_empty() {
                        continue;
                    }
                    let p = PathBuf::from(path);
                    if p.is_dir() {
                        inputs.extend(lightning::files_under(&p));
                    } else if p.is_file() {
                        inputs.push(p);
                    }
                }
            }
        }
        inputs.sort();
        inputs.dedup();
        let learned = Learned { inputs, env: Vec::new() };
        lightning.learn(&id, &flags, &learned);
        let env = BTreeMap::new();
        let key = lightning.key(&id, &flags, &links, &env).ok_or(format!("{}: a source is gone", t.name()))?;
        if lightning.strike(&key) {
            let _ = std::fs::remove_dir_all(dest);
            lightning.restore(&key, dest)?;
            return Ok((key, true));
        }

        let tmp = lightning::home().join("work").join(&hex(&Sha256::digest(id.as_bytes()))[..16]);
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let mut argfile: Vec<String> = Vec::new();
        let mut cmd;
        if t.kind == "java" {
            cmd = Command::new(java_bin(&t.java_home, "javac", &default_home));
            argfile.extend(["-d".into(), tmp.display().to_string()]);
            // What Gradle gives javac by default, outside the task's own
            // options: full debug information, and no source path.
            argfile.push("-g".into());
            if !t.args.iter().any(|a| a == "--module-source-path") {
                argfile.extend(["-sourcepath".into(), String::new()]);
            }
            if let Some(e) = &t.encoding {
                argfile.extend(["-encoding".into(), e.clone()]);
            }
            if let Some(r) = t.release {
                argfile.extend(["--release".into(), r.to_string()]);
            }
            if !classpath.is_empty() {
                let modular = t.infer_module_path && t.sources.iter().any(|s| s.ends_with("module-info.java"));
                argfile.push(if modular { "--module-path".into() } else { "-classpath".into() });
                argfile.push(classpath.join(";"));
            }
            argfile.extend(args.iter().cloned());
            if t.release.is_some() {
                // A newer javac warns that an old --release is obsolete, and
                // -Werror would make that an error; Gradle's compile shows no
                // such warning. Only these option warnings are silenced.
                argfile.push("-Xlint:-options".into());
            }
            argfile.extend(t.sources.iter().cloned());
        } else {
            cmd = Command::new(java_bin(&None, "java", &default_home));
            cmd.arg("-cp").arg(t.compiler_classpath.join(";")).arg("org.jetbrains.kotlin.cli.jvm.K2JVMCompiler");
            argfile.extend(["-d".into(), tmp.display().to_string(), "-no-stdlib".into(), "-no-reflect".into()]);
            if !classpath.is_empty() {
                argfile.extend(["-classpath".into(), classpath.join(";")]);
            }
            for (flag, v) in [("-jvm-target", &t.jvm_target), ("-api-version", &t.api_version), ("-language-version", &t.language_version), ("-module-name", &t.module_name)] {
                if let Some(v) = v.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                    argfile.extend([flag.to_string(), v.to_string()]);
                }
            }
            argfile.extend(args.iter().cloned());
            argfile.extend(t.sources.iter().cloned());
            argfile.extend(t.java_sources.iter().cloned());
        }
        // One argument a line, quoted: the lists are long and paths have spaces.
        let file = tmp.with_extension("args");
        let text: String = argfile.iter().map(|a| format!("\"{}\"\n", a.replace('\\', "\\\\"))).collect();
        std::fs::write(&file, text).map_err(|e| e.to_string())?;
        let o = cmd.arg(format!("@{}", file.display())).output().map_err(|e| format!("{}: {e}", t.name()))?;
        if !o.status.success() {
            return Err(format!("{} failed:\n{}{}", t.name(), String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)));
        }
        let outputs = lightning::files_under(&tmp);
        let key = lightning.brand(&id, &flags, &links, &env, learned, &tmp, &outputs)?;
        let _ = std::fs::remove_dir_all(dest);
        lightning.restore(&key, dest)?;
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_file(&file);
        Ok((key, false))
    };

    std::thread::scope(|s| {
        for _ in 0..jobs.max(1) {
            s.spawn(|| loop {
                let (i, links) = {
                    let mut st = state.lock().unwrap();
                    loop {
                        if st.error.is_some() {
                            return;
                        }
                        let ready = (0..tasks.len()).find(|&i| !st.started[i] && needs[i].iter().all(|&k| st.keys[k].is_some()));
                        if let Some(i) = ready {
                            st.started[i] = true;
                            st.running += 1;
                            let links = needs[i].iter().map(|&k| st.keys[k].clone().unwrap()).collect();
                            break (i, links);
                        }
                        if st.started.iter().all(|s| *s) || st.running == 0 {
                            return;
                        }
                        st = wake.wait(st).unwrap();
                    }
                };
                let t0 = Instant::now();
                let r = run(i, links);
                let mut st = state.lock().unwrap();
                st.running -= 1;
                match r {
                    Ok((key, struck)) => {
                        st.done.push(Done { task: tasks[i].name(), struck, seconds: t0.elapsed().as_secs_f64() });
                        st.keys[i] = Some(key);
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
    if let Some(i) = st.keys.iter().position(|k| k.is_none()) {
        return Err(format!("{} never ran (a cycle?)", tasks[i].name()));
    }
    let mut done = st.done;
    done.sort_by(|a, b| a.task.cmp(&b.task));
    Ok(Report { done, seconds: start.elapsed().as_secs_f64(), out: out.to_path_buf() })
}
