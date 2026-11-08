//! strike: a Rust build system that calls rustc itself.
//!
//!   strike build <dir> [--for build|run] [--rules FILE] [--explain]
//!   strike test  <dir> [--for build|run] [--rules FILE] [--explain]
//!   strike jvm <export-dir> <out-dir>   a JVM build's compile tasks
//!   strike brand          every branded build
//!   strike fetch <dir>    fStrike: the packages of a Cargo.lock
//!   strike fetch <url> <file> [--sha256 HASH]
//!   strike arc devices|receive|send   ArcStrike: folders to a nearby device
//!   strike compile <file> [-o EXE]    the universal compiler

mod arc;
mod builder;
mod fstrike;
mod graph;
mod jvm;
mod lightning;
mod manifest;
mod motor;
mod times;
mod uc;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage:
  strike build <dir> [--for build|run] [--rules FILE] [--explain]
  strike test  <dir> [--for build|run] [--rules FILE] [--explain]
  strike jvm <export-dir> <out-dir>
  strike fetch <dir>
  strike fetch <url> <file> [--sha256 HASH]
  strike arc devices
  strike arc receive [--into DIR] [--port N] [--once]
  strike arc send <path> --to <device|host:port> [--streams N]
  strike compile <file> [-o EXE] [--explain] [--no-brand] [--plain]
  strike brand";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<ExitCode, String> {
    let Some(command) = args.first() else { return Err(USAGE.into()) };
    match command.as_str() {
        "brand" => {
            let l = lightning::Lightning::open(builder::rustc_version()?);
            let list = l.list();
            for (key, unit, flags) in &list {
                println!("{}  {unit}  {}", &key[..12], flags.join(" "));
            }
            println!("{} brands in {}", list.len(), lightning::home().display());
            Ok(ExitCode::SUCCESS)
        }
        "build" | "test" => {
            let mut dir = None;
            let mut profile = "build".to_string();
            let mut rules_file: Option<PathBuf> = None;
            let mut explain = false;
            let mut verbose = false;
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--for" => profile = it.next().ok_or("--for needs build or run")?.clone(),
                    "--rules" => rules_file = Some(PathBuf::from(it.next().ok_or("--rules needs a file")?)),
                    "--explain" => explain = true,
                    "--verbose" => verbose = true,
                    s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}")),
                    s if dir.is_none() => dir = Some(PathBuf::from(s)),
                    s => return Err(format!("one folder only, got also {s}\n{USAGE}")),
                }
            }
            let dir = dir.unwrap_or_else(|| PathBuf::from("."));
            let text = match &rules_file {
                Some(f) => std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?,
                None => motor::DEFAULT_RULES.to_string(),
            };
            let rules = motor::Rules::parse(&text)?;
            let pkg = manifest::read(&dir)?;
            let jobs = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
            let opt = builder::Options { profile, explain, test: command == "test", jobs };
            let r = builder::build(&pkg, &rules, &opt);
            times::report();
            let r = r?;

            let choice: Vec<String> = r.chosen.choice.iter().map(|(n, v)| format!("{n}={v}")).collect();
            println!("strike: {} chose {}", pkg.name, choice.join(" "));
            for d in r.done.iter().filter(|d| d.root || verbose) {
                let how = if d.struck { "struck" } else { "built " };
                println!("  {how} {:<40} {:>7.2} s  {}", d.job, d.seconds, &d.key[..12]);
            }
            let deps: Vec<_> = r.done.iter().filter(|d| !d.root).collect();
            if !deps.is_empty() {
                let built = deps.iter().filter(|d| !d.struck).count();
                println!("  dependencies: {} jobs, {} struck, {built} built", deps.len(), deps.len() - built);
            }
            let built = r.done.iter().filter(|d| !d.struck).count();
            println!(
                "  {} of {} from the brand, {built} built, {:.2} s; out: {}",
                r.done.len() - built,
                r.done.len(),
                r.seconds,
                r.out.display()
            );
            let failed: Vec<&String> = r.tests.iter().filter(|(_, ok)| !ok).map(|(t, _)| t).collect();
            if !r.tests.is_empty() {
                println!("  tests: {} of {} harnesses passed", r.tests.len() - failed.len(), r.tests.len());
            }
            Ok(if failed.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        "jvm" => {
            let (Some(export), Some(out)) = (args.get(1), args.get(2)) else { return Err(USAGE.into()) };
            let jobs = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
            let r = jvm::build(std::path::Path::new(export), std::path::Path::new(out), jobs)?;
            for d in &r.done {
                let how = if d.struck { "struck" } else { "built " };
                println!("  {how} {:<55} {:>7.2} s", d.task, d.seconds);
            }
            let built = r.done.iter().filter(|d| !d.struck).count();
            println!("  {} of {} from the brand, {built} built, {:.2} s; out: {}", r.done.len() - built, r.done.len(), r.seconds, r.out.display());
            Ok(ExitCode::SUCCESS)
        }
        "fetch" => {
            let start = std::time::Instant::now();
            let Some(what) = args.get(1) else { return Err(USAGE.into()) };
            if what.starts_with("https://") || what.starts_with("http://") {
                let Some(dest) = args.get(2) else { return Err(USAGE.into()) };
                let want = match args.get(3).map(String::as_str) {
                    Some("--sha256") => Some(args.get(4).ok_or("--sha256 needs a hash")?.to_lowercase()),
                    Some(s) => return Err(format!("unknown option {s}\n{USAGE}")),
                    None => None,
                };
                let (hash, how) = fstrike::fetch_url(what, want.as_deref())?;
                fstrike::strike_to(&hash, std::path::Path::new(dest))?;
                println!("  {} {dest}  {}  {:.2} s", how_word(how), &hash[..12], start.elapsed().as_secs_f64());
                return Ok(ExitCode::SUCCESS);
            }
            let jobs = 8;
            let done = fstrike::fetch_lock(std::path::Path::new(what), jobs)?;
            let fetched: Vec<u64> = done.iter().filter_map(|(_, h)| if let fstrike::How::Fetched(b) = h { Some(*b) } else { None }).collect();
            for (name, how) in done.iter().filter(|(_, h)| *h != fstrike::How::Struck) {
                println!("  {} {name}", how_word(*how));
            }
            println!(
                "  {} packages: {} struck, {} fetched ({:.1} MB), {:.2} s; in {}",
                done.len(),
                done.len() - fetched.len(),
                fetched.len(),
                fetched.iter().sum::<u64>() as f64 / 1e6,
                start.elapsed().as_secs_f64(),
                fstrike::registry_dir().display()
            );
            Ok(ExitCode::SUCCESS)
        }
        "arc" => arc_command(&args[1..]),
        "compile" => {
            let mut file = None;
            let mut out = None;
            let mut explain = false;
            let mut brands = true;
            let mut gen = uc::Gen::Opt;
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "-o" => out = Some(PathBuf::from(it.next().ok_or("-o needs a file")?)),
                    "--explain" => explain = true,
                    "--no-brand" => brands = false,
                    "--plain" => gen = uc::Gen::Plain,
                    s if s.starts_with('-') => return Err(format!("unknown option {s}\n{USAGE}")),
                    s if file.is_none() => file = Some(PathBuf::from(s)),
                    s => return Err(format!("one file only, got also {s}\n{USAGE}")),
                }
            }
            let file = file.ok_or(USAGE)?;
            let out = out.unwrap_or_else(|| file.with_extension("exe"));
            let r = uc::compile(&file, &out, brands, gen)?;
            if explain {
                for line in &r.explain {
                    println!("  {line}");
                }
            }
            println!(
                "strike: {} read as {}; {} functions, {} struck, {} compiled, {} calls inlined; {:.3} s (read {:.3}, code {:.3}, exe {:.3}); {} ({} bytes)",
                file.display(),
                r.lang,
                r.funcs,
                r.struck,
                r.funcs - r.struck,
                r.inlined,
                r.seconds,
                r.read_seconds,
                r.code_seconds,
                r.exe_seconds,
                out.display(),
                r.exe_bytes
            );
            Ok(ExitCode::SUCCESS)
        }
        _ => Err(USAGE.into()),
    }
}

/// ArcStrike: `devices`, `receive`, `send`.
fn arc_command(args: &[String]) -> Result<ExitCode, String> {
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    match args.first().map(String::as_str) {
        Some("devices") => {
            let found = arc::devices(std::time::Duration::from_secs(3), None)?;
            for (name, (os, addr)) in &found {
                println!("  {name:<24} {os:<8} {addr}");
            }
            println!("  {} devices", found.len());
            Ok(ExitCode::SUCCESS)
        }
        Some("receive") => {
            let into = flag("--into").map(PathBuf::from).unwrap_or_else(|| {
                let home = std::env::var_os("USERPROFILE").or(std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
                home.join("Downloads")
            });
            let port = flag("--port").map(|p| p.parse().map_err(|_| "--port needs a number")).transpose()?.unwrap_or(arc::DEFAULT_PORT);
            arc::receive(&into, port, args.iter().any(|a| a == "--once"))?;
            Ok(ExitCode::SUCCESS)
        }
        Some("send") => {
            let path = args.get(1).filter(|a| !a.starts_with("--")).ok_or("arc send <path> --to <device>")?;
            let to = flag("--to").ok_or("arc send <path> --to <device>")?;
            let streams = flag("--streams").map(|s| s.parse().map_err(|_| "--streams needs a number")).transpose()?.unwrap_or(4);
            let stop = flag("--stop-after").map(|s| s.parse().map_err(|_| "--stop-after needs bytes")).transpose()?;
            let r = arc::send(std::path::Path::new(path), &to, streams, stop)?;
            println!(
                "  sent {} files ({:.1} MB) to {}: {:.1} MB over the wire in {} chunks, {:.2} s (scan {:.2} s), {:.1} MB/s",
                r.files,
                r.bytes as f64 / 1e6,
                r.peer,
                r.sent_bytes as f64 / 1e6,
                r.chunks_sent,
                r.seconds,
                r.scan_seconds,
                r.bytes as f64 / 1e6 / r.seconds
            );
            Ok(ExitCode::SUCCESS)
        }
        _ => Err(USAGE.into()),
    }
}

fn how_word(how: fstrike::How) -> &'static str {
    match how {
        fstrike::How::Struck => "struck ",
        fstrike::How::NotModified => "same   ",
        fstrike::How::Fetched(_) => "fetched",
    }
}
