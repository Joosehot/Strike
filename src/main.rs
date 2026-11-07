//! strike: a Rust build system that calls rustc itself.
//!
//!   strike build <dir> [--for build|run] [--rules FILE] [--explain]
//!   strike test  <dir> [--for build|run] [--rules FILE] [--explain]
//!   strike brand          every branded build

mod builder;
mod graph;
mod lightning;
mod manifest;
mod motor;
mod times;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage:
  strike build <dir> [--for build|run] [--rules FILE] [--explain]
  strike test  <dir> [--for build|run] [--rules FILE] [--explain]
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
        _ => Err(USAGE.into()),
    }
}
