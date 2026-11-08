//! The universal compiler: the same suite in C, Go and Python, compiled by
//! Strike to executables whose output must be byte for byte what clang, Go
//! and Python themselves print (tests/uc/*.out, written by them).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root(name: &str) -> PathBuf {
    let r = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("uc-{name}"));
    let _ = std::fs::remove_dir_all(&r);
    std::fs::create_dir_all(&r).unwrap();
    r
}

fn suite(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/uc").join(file)
}

fn compile(home: &Path, src: &Path, exe: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_strike"))
        .args(["compile", src.to_str().unwrap(), "-o", exe.to_str().unwrap(), "--explain"])
        .env("STRIKE_HOME", home.join("home"))
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr)
}

fn runs_as_the_real_one(file: &str, lang: &str) {
    let r = root(file);
    let exe = r.join("a.exe");
    let o = compile(&r, &suite(file), &exe);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains(&format!("read as {lang};")), "{}", text(&o));
    let run = Command::new(&exe).output().unwrap();
    assert!(run.status.success());
    let want = std::fs::read(suite(&format!("{file}.out"))).unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&want));
}

#[test]
fn c_prints_what_clang_prints() {
    runs_as_the_real_one("suite.c", "C");
}

#[test]
fn go_prints_what_go_prints() {
    runs_as_the_real_one("suite.go", "Go");
}

#[test]
fn python_prints_what_python_prints() {
    runs_as_the_real_one("suite.py", "Python");
}

#[test]
fn the_language_is_found_without_the_extension() {
    let r = root("noext");
    let src = r.join("program.txt");
    std::fs::copy(suite("suite.go"), &src).unwrap();
    let o = compile(&r, &src, &r.join("a.exe"));
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("read as Go;"), "{}", text(&o));
}

#[test]
fn a_second_compile_strikes_every_function_and_writes_the_same_exe() {
    let r = root("again");
    let (one, two) = (r.join("one.exe"), r.join("two.exe"));
    assert!(compile(&r, &suite("suite.c"), &one).status.success());
    let o = compile(&r, &suite("suite.c"), &two);
    assert!(text(&o).contains("6 functions, 6 struck, 0 compiled"), "{}", text(&o));
    assert_eq!(std::fs::read(one).unwrap(), std::fs::read(two).unwrap());
}

/// Random C programs (no recursion, loops with fixed bounds, division only
/// by constants that are not zero), to check the optimizing generator
/// against the plain one.
struct Gen(u64);

impl Gen {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn constant(&mut self) -> String {
        match self.below(6) {
            0 => format!("{}", self.next() as i64 / 3),
            1 => format!("-{}", self.below(1000)),
            _ => format!("{}", self.below(100)),
        }
    }
    fn expr(&mut self, names: &[String], funcs: &[(String, usize)], depth: u32) -> String {
        if depth == 0 || self.below(4) == 0 {
            return if !names.is_empty() && self.below(3) > 0 {
                names[self.below(names.len() as u64) as usize].clone()
            } else {
                self.constant()
            };
        }
        let d = depth - 1;
        match self.below(9) {
            0..=2 => {
                let op = ["+", "-", "*", "&", "|", "^", "<", "<=", "==", "!=", ">", ">=", "&&", "||"][self.below(14) as usize];
                format!("({} {op} {})", self.expr(names, funcs, d), self.expr(names, funcs, d))
            }
            3 => {
                let mut k = self.below(19) as i64 - 9;
                if k == 0 {
                    k = 7;
                }
                let op = ["/", "%"][self.below(2) as usize];
                format!("({} {op} {k})", self.expr(names, funcs, d))
            }
            4 => format!("({} {} {})", self.expr(names, funcs, d), ["<<", ">>"][self.below(2) as usize], self.below(64)),
            5 => format!("{}({})", ["-", "!", "~"][self.below(3) as usize], self.expr(names, funcs, d)),
            6 if !funcs.is_empty() => {
                let (f, n) = &funcs[self.below(funcs.len() as u64) as usize];
                let args: Vec<String> = (0..*n).map(|_| self.expr(names, funcs, d.min(1))).collect();
                format!("{f}({})", args.join(", "))
            }
            _ => self.expr(names, funcs, d),
        }
    }
    fn program(&mut self) -> String {
        let mut out = String::from("#include <stdio.h>\n");
        let mut funcs: Vec<(String, usize)> = Vec::new();
        for k in 0..6 {
            let n = self.below(7) as usize;
            let params: Vec<String> = (0..n).map(|i| format!("p{i}")).collect();
            let mut names = params.clone();
            let mut body = String::new();
            for v in 0..self.below(4) {
                body += &format!("    long x{v} = {};\n", self.expr(&names, &funcs, 3));
                names.push(format!("x{v}"));
            }
            let target = names.last().cloned();
            if let Some(t) = &target {
                body += &format!(
                    "    for (int i = 0; i < {}; i++) {{\n        if ({}) {t} = {}; else {t} += {};\n    }}\n",
                    self.below(20),
                    self.expr(&names, &funcs, 2),
                    self.expr(&names, &funcs, 3),
                    self.expr(&names, &funcs, 2)
                );
            }
            body += &format!("    return {};\n", self.expr(&names, &funcs, 4));
            let decl: Vec<String> = params.iter().map(|p| format!("long {p}")).collect();
            out += &format!("long f{k}({}) {{\n{body}}}\n", decl.join(", "));
            funcs.push((format!("f{k}"), n));
        }
        out += "int main() {\n";
        for (f, n) in &funcs {
            let args: Vec<String> = (0..*n).map(|_| self.constant()).collect();
            out += &format!("    printf(\"%lld\\n\", {f}({}));\n", args.join(", "));
        }
        out + "    return 0;\n}\n"
    }
}

#[test]
fn the_optimized_code_prints_what_the_plain_code_prints() {
    let r = root("differ");
    let mut g = Gen(0x9E37_79B9_7F4A_7C15);
    for n in 0..100 {
        let src = r.join(format!("p{n}.c"));
        std::fs::write(&src, g.program()).unwrap();
        let mut outs = Vec::new();
        for plain in [false, true] {
            let exe = r.join(format!("p{n}-{plain}.exe"));
            let mut c = Command::new(env!("CARGO_BIN_EXE_strike"));
            c.args(["compile", src.to_str().unwrap(), "-o", exe.to_str().unwrap()]).env("STRIKE_HOME", r.join("home"));
            if plain {
                c.arg("--plain");
            }
            let o = c.output().unwrap();
            assert!(o.status.success(), "p{n}.c: {}", text(&o));
            let run = Command::new(&exe).output().unwrap();
            outs.push((run.status.code(), run.stdout));
        }
        assert_eq!(outs[0], outs[1], "p{n}.c: the optimized and the plain code differ");
    }
}

#[test]
fn what_strike_cannot_do_yet_is_refused_not_guessed() {
    let r = root("refused");
    for (name, src, says) in [
        ("chain.py", "x = 1\nprint(0 < x < 2)\n", "chained comparison"),
        ("float.py", "print(7 / 2)\n", "/"),
        ("undeclared.c", "int main() { return y; }\n", "y is not declared"),
        ("none.py", "def f(x):\n    if x:\n        return 1\nprint(f(0))\n", "can end without returning"),
        ("nolang.txt", "SELECT * FROM t;\n", "no description reads the whole file"),
    ] {
        let p = r.join(name);
        std::fs::write(&p, src).unwrap();
        let o = compile(&r, &p, &r.join("a.exe"));
        assert!(!o.status.success(), "{name} compiled");
        assert!(text(&o).contains(says), "{name}: {}", text(&o));
    }
}
