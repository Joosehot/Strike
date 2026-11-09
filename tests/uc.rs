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

/// Builds a suite file in every mode and runs it with the given input:
/// what it prints must be what the real toolchain printed for that input.
fn runs_with_input_as_the_real_one(file: &str, input: &str) {
    let want = std::fs::read(suite(&format!("{file}.out"))).unwrap();
    let stdin = std::fs::read(suite(input)).unwrap();
    let r = root(&format!("in-{file}"));
    for mode in [vec!["--plain"], vec![], vec!["--release"], vec!["--release", "--no-evaluate"]] {
        let exe = r.join("a.exe");
        let o = compile_with(&r, &suite(file), &exe, &mode, None);
        assert!(o.status.success(), "{file}: {}", text(&o));
        let got = run_with(&exe, &stdin);
        assert_eq!(String::from_utf8_lossy(&got.stdout), String::from_utf8_lossy(&want), "{file} {mode:?}");
    }
}

fn run_with(exe: &Path, stdin: &[u8]) -> Output {
    use std::io::Write;
    let mut c = Command::new(exe).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(stdin).unwrap();
    c.wait_with_output().unwrap()
}

#[test]
fn globals_are_what_clang_go_and_python_make_of_them() {
    for (file, lang) in [("globals.c", "C"), ("globals.go", "Go"), ("globals.py", "Python")] {
        runs_as_the_real_one(file, lang);
    }
}

/// Builds a suite file in every mode: what it prints must be what the real
/// toolchain printed.
fn runs_in_every_mode_as_the_real_one(file: &str) {
    let want = std::fs::read(suite(&format!("{file}.out"))).unwrap();
    let r = root(&format!("modes-{file}"));
    for mode in [vec!["--plain"], vec![], vec!["--release"], vec!["--release", "--no-evaluate"]] {
        let exe = r.join("a.exe");
        let o = compile_with(&r, &suite(file), &exe, &mode, None);
        assert!(o.status.success(), "{file} {mode:?}: {}", text(&o));
        let got = Command::new(&exe).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&got.stdout), String::from_utf8_lossy(&want), "{file} {mode:?}");
        assert!(got.status.success(), "{file} {mode:?}: {}", String::from_utf8_lossy(&got.stderr));
    }
}

/// Programs heavy in what the release simplifier rewrites (polynomials,
/// shared factors, constant multiples, index arithmetic, C int and char
/// cuts) print in every mode what clang and Go print.
/// Work across statements (an unrolled counter's known low bits, value
/// numbering, shared expressions) and the cuts a range analysis leaves
/// out: in every mode what clang and Go print.
#[test]
fn work_across_statements_is_what_clang_and_go_make_of_it() {
    for file in ["values.c", "values.go"] {
        runs_in_every_mode_as_the_real_one(file);
    }
}

/// A comparison waiting as flags under a call of no arguments: in every
/// mode what clang prints (a bug of the optimizing generator, fixed).
#[test]
fn a_comparison_before_a_call_of_no_arguments_keeps_its_value() {
    runs_in_every_mode_as_the_real_one("flags_call.c");
}

#[test]
fn simplified_expressions_are_what_clang_and_go_make_of_them() {
    for file in ["algebra.c", "algebra.go"] {
        runs_in_every_mode_as_the_real_one(file);
    }
}

/// C's bool is 0 or 1: a value going there becomes 1 where it is not
/// zero, as clang converts it (in a variable, an array, a field, a
/// parameter, from a float).
/// <<=, >>=, &=, |= and ^= on variables and elements, in the three languages.
#[test]
fn bitwise_assignments_are_what_clang_go_and_python_make_of_them() {
    for file in ["opassign.c", "opassign.go", "opassign.py"] {
        runs_in_every_mode_as_the_real_one(file);
    }
}

#[test]
fn c_bool_is_what_clang_makes_of_it() {
    runs_in_every_mode_as_the_real_one("cbool.c");
}

#[test]
fn arrays_lists_and_slices_are_what_clang_go_and_python_make_of_them() {
    for file in ["arrays.c", "arrays.go", "arrays.py"] {
        runs_in_every_mode_as_the_real_one(file);
    }
}

#[test]
fn structs_and_classes_are_what_clang_go_and_python_make_of_them() {
    for file in ["structs.c", "structs.go", "classes.py"] {
        runs_in_every_mode_as_the_real_one(file);
    }
}

/// 160 structs of random fields (tools/struct_layouts.py): every sizeof
/// and offsetof Strike's executable prints is what clang's printed.
#[test]
fn c_structs_are_laid_out_as_clang_lays_them_out() {
    runs_in_every_mode_as_the_real_one("layouts.c");
}

/// Go stops where a nil pointer is followed (exit code 2), Python where an
/// attribute is read that the object has not been given (AttributeError,
/// exit code 1), with what was printed before and nothing after.
#[test]
fn a_nil_pointer_and_a_missing_attribute_stop_the_program_as_go_and_python_stop_it() {
    let r = root("struct-stops");
    for (name, src, code, says) in [
        ("nil.go", "package main\nimport \"fmt\"\ntype P struct {\n\tX int\n}\nfunc main() {\n\tvar p *P\n\tfmt.Println(1)\n\tfmt.Println(p.X)\n}\n", 2, "nil pointer dereference"),
        ("nil_method.go", "package main\nimport \"fmt\"\ntype P struct {\n\tX int\n}\nfunc (p P) Get() int {\n\treturn p.X\n}\nfunc main() {\n\tvar p *P\n\tfmt.Println(1)\n\tfmt.Println(p.Get())\n}\n", 2, "nil pointer dereference"),
        ("nil_set.go", "package main\nimport \"fmt\"\ntype N struct {\n\tV int\n\tNext *N\n}\nfunc main() {\n\tn := &N{V: 1}\n\tfmt.Println(n.V)\n\tn.Next.V = 2\n}\n", 2, "nil pointer dereference"),
        ("attr.py", "class P:\n    def __init__(self):\n        self.x = 1\n\n    def later(self):\n        self.y = 2\n\n\np = P()\nprint(p.x)\nprint(p.y)\n", 1, "AttributeError: 'P' object has no attribute 'y'"),
        ("attr_order.py", "class P:\n    def __init__(self):\n        print(1)\n        print(self.x)\n        self.x = 2\n\n\nP()\n", 1, "AttributeError"),
    ] {
        let p = r.join(name);
        std::fs::write(&p, src).unwrap();
        for mode in [vec!["--plain"], vec![], vec!["--release"]] {
            let exe = r.join("a.exe");
            let o = compile_with(&r, &p, &exe, &mode, None);
            assert!(o.status.success(), "{name}: {}", text(&o));
            let got = Command::new(&exe).output().unwrap();
            assert_eq!(got.status.code(), Some(code), "{name} {mode:?}");
            assert_eq!(String::from_utf8_lossy(&got.stdout), "1\n", "{name} {mode:?}");
            assert!(String::from_utf8_lossy(&got.stderr).contains(says), "{name} {mode:?}: {}", String::from_utf8_lossy(&got.stderr));
        }
    }
}

#[test]
fn floats_are_what_clang_go_and_python_make_of_them() {
    for file in ["floats.c", "floats.go", "floats.py"] {
        runs_in_every_mode_as_the_real_one(file);
    }
}

/// 400 doubles (edge cases, random bit patterns, random decimals) printed
/// in every format: C's printf ten ways, Go's Println and Printf (and the
/// float32 of each), Python's repr. tools/float_values.py wrote the
/// programs, and clang, Go and Python the .out files.
#[test]
fn floats_print_as_clang_go_and_python_print_them() {
    for file in ["floatprint.c", "floatprint.go", "floatprint.py"] {
        runs_in_every_mode_as_the_real_one(file);
    }
}

/// Python stops where it raises: a division by zero (ZeroDivisionError),
/// int() of NaN (ValueError) or of an infinity (OverflowError), math.sqrt
/// of a negative number; and Strike stops where Python would go on with a
/// number it does not have (int() past 64 bits, a division of whole numbers
/// past 2^53). Nothing is printed after it.
#[test]
fn python_stops_on_floats_where_python_raises() {
    let r = root("py-floats");
    for (expr, says) in [
        ("1 / 0.0", "ZeroDivisionError: division by zero"),
        ("1.0 // 0", "ZeroDivisionError"),
        ("5.5 % -0.0", "ZeroDivisionError"),
        ("7 // 0", "ZeroDivisionError"),
        ("5 % 0", "ZeroDivisionError"),
        ("1 / 0", "ZeroDivisionError"),
        ("int(z * 1e400 - z * 1e400)", "ValueError: cannot convert float NaN to integer"),
        ("int(-1e400 * z)", "OverflowError: cannot convert float infinity to integer"),
        ("int(1e30 * z)", "grew past 64 bits"),
        ("math.sqrt(-2.5 * z)", "ValueError: expected a nonnegative input, got -2.5"),
        ("1152921504606846976 / 3", "past 2^53"),
    ] {
        let p = r.join("stop.py");
        std::fs::write(&p, format!("import math\nz = 1.0\nprint(1)\nprint({expr})\nprint(2)\n")).unwrap();
        for mode in [vec!["--plain"], vec![], vec!["--release"], vec!["--release", "--no-evaluate"]] {
            let exe = r.join("a.exe");
            assert!(compile_with(&r, &p, &exe, &mode, None).status.success(), "{expr}");
            let got = Command::new(&exe).output().unwrap();
            assert_eq!(got.status.code(), Some(1), "{expr} {mode:?}");
            assert_eq!(String::from_utf8_lossy(&got.stdout), "1\n", "{expr} {mode:?}");
            assert!(String::from_utf8_lossy(&got.stderr).contains(says), "{expr} {mode:?}: {}", String::from_utf8_lossy(&got.stderr));
        }
    }
}

#[test]
fn strings_are_what_clang_go_and_python_make_of_them() {
    for file in ["strings.c", "strings.go", "strings.py"] {
        runs_in_every_mode_as_the_real_one(file);
    }
    runs_with_input_as_the_real_one("strings_input.py", "strings_input.in");
}

/// What a string cannot be stops the program as the language stops it
/// (Python's IndexError, ValueError, TypeError and EOFError with exit code
/// 1, Go's panic with 2), with what was printed before it and nothing
/// after; and what Strike's strings do not hold stops it with Strike's
/// reason, never a wrong string.
#[test]
fn a_string_that_cannot_be_stops_the_program_as_python_and_go_stop_it() {
    let r = root("string-errors");
    for (name, src, stdin, code, says) in [
        ("index.py", "s = \"abc\"\nprint(s[-3])\nprint(s[3])\nprint(9)\n", "", 1, "IndexError: string index out of range"),
        ("int.py", "print(int(\" 1_0 \"))\nprint(int(\"1__0\"))\n", "", 1, "ValueError"),
        ("ord.py", "print(ord(\"a\"))\nprint(ord(\"ab\"))\n", "", 1, "TypeError"),
        ("chr.py", "print(chr(97))\nprint(chr(1114112))\n", "", 1, "ValueError"),
        ("eof.py", "print(input())\nprint(input())\n", "one\n", 1, "EOFError"),
        ("byte.go", "package main\nimport \"fmt\"\nfunc main() {\n\ts := \"abc\"\n\ti := 3\n\tfmt.Println(s[2])\n\tfmt.Println(s[i])\n}\n", "", 2, "index out of range"),
        ("cut.go", "package main\nimport \"fmt\"\nfunc main() {\n\ts := \"abc\"\n\ti := 2\n\tfmt.Println(s[1:3])\n\tfmt.Println(s[i:1])\n}\n", "", 2, "slice bounds out of range"),
        ("past.go", "package main\nimport \"fmt\"\nfunc main() {\n\ts := \"abc\"\n\ti := 4\n\tfmt.Println(s[:3])\n\tfmt.Println(s[:i])\n}\n", "", 2, "slice bounds out of range"),
        // not exact yet: refused while running, with the reason
        ("chr_past.py", "print(chr(65))\nprint(chr(233))\n", "", 1, "past ASCII"),
        ("read_past.py", "print(input())\nprint(input())\n", "ok\ncaf\u{e9}\n", 1, "past ASCII"),
        ("lone_cr.py", "print(input())\nprint(input())\n", "ok\na\rb\n", 1, "carriage return"),
        ("quote_past.go", "package main\nimport \"fmt\"\nfunc main() {\n\tfmt.Printf(\"%q\\n\", \"ab\")\n\tfmt.Printf(\"%q\\n\", \"caf\u{e9}\")\n}\n", "", 1, "%q of a string past ASCII"),
    ] {
        let p = r.join(name);
        std::fs::write(&p, src).unwrap();
        for mode in [vec!["--plain"], vec![], vec!["--release"]] {
            let exe = r.join("a.exe");
            let o = compile_with(&r, &p, &exe, &mode, None);
            assert!(o.status.success(), "{name}: {}", text(&o));
            let got = run_with(&exe, stdin.as_bytes());
            assert_eq!(got.status.code(), Some(code), "{name} {mode:?}");
            assert_eq!(String::from_utf8_lossy(&got.stdout).lines().count(), 1, "{name} {mode:?}: {}", String::from_utf8_lossy(&got.stdout));
            assert!(String::from_utf8_lossy(&got.stderr).contains(says), "{name} {mode:?}: {}", String::from_utf8_lossy(&got.stderr));
        }
    }
}

/// An index out of range stops the program as the language stops it:
/// Python's IndexError with exit code 1, Go's panic with exit code 2, and
/// nothing printed after it.
#[test]
fn an_index_out_of_range_stops_the_program_as_python_and_go_stop_it() {
    let r = root("index");
    for (name, src, code, says) in [
        ("list.py", "a = [1, 2, 3]\nprint(a[2])\nprint(a[3])\nprint(9)\n", 1, "IndexError"),
        ("negative.py", "a = [1, 2, 3]\nprint(a[-3])\nprint(a[-4])\n", 1, "IndexError"),
        ("pop.py", "a = []\na.append(1)\nprint(a.pop())\nprint(a.pop())\n", 1, "pop from empty list"),
        ("slice.go", "package main\nimport \"fmt\"\nfunc main() {\n\ts := []int{1, 2}\n\tfmt.Println(s[1])\n\ti := 2\n\tfmt.Println(s[i])\n}\n", 2, "index out of range"),
        ("array.go", "package main\nimport \"fmt\"\nfunc main() {\n\tvar a [3]int\n\ti := -1\n\tfmt.Println(a[0])\n\ta[i] = 1\n}\n", 2, "index out of range"),
        ("bounds.go", "package main\nimport \"fmt\"\nfunc main() {\n\ts := make([]int, 2, 4)\n\tfmt.Println(len(s[1:4]))\n\tn := 5\n\tfmt.Println(s[1:n])\n}\n", 2, "slice bounds out of range"),
        ("make.go", "package main\nimport \"fmt\"\nfunc main() {\n\tn := -1\n\tfmt.Println(1)\n\ts := make([]int, n)\n\tfmt.Println(len(s))\n}\n", 2, "makeslice"),
    ] {
        let p = r.join(name);
        std::fs::write(&p, src).unwrap();
        for mode in [vec!["--plain"], vec![], vec!["--release"]] {
            let exe = r.join("a.exe");
            assert!(compile_with(&r, &p, &exe, &mode, None).status.success(), "{name}");
            let got = Command::new(&exe).output().unwrap();
            assert_eq!(got.status.code(), Some(code), "{name} {mode:?}");
            assert_eq!(String::from_utf8_lossy(&got.stdout).lines().count(), 1, "{name} {mode:?}: {}", String::from_utf8_lossy(&got.stdout));
            assert!(String::from_utf8_lossy(&got.stderr).contains(says), "{name} {mode:?}");
        }
    }
}

#[test]
fn input_is_read_as_clang_go_and_python_read_it() {
    runs_with_input_as_the_real_one("input.c", "input.in");
    runs_with_input_as_the_real_one("input.go", "input.in");
    runs_with_input_as_the_real_one("input.py", "input.py.in");
}

#[test]
fn a_program_that_reads_is_never_worked_out_while_compiling() {
    let r = root("reads");
    let exe = r.join("a.exe");
    let o = compile_with(&r, &suite("input.c"), &exe, &["--release"], None);
    assert!(o.status.success() && !text(&o).contains("reads nothing"), "{}", text(&o));
    assert_eq!(String::from_utf8_lossy(&run_with(&exe, b"2 40 2").stdout), "42\n");
    assert_eq!(String::from_utf8_lossy(&run_with(&exe, b"3 1 1 1").stdout), "3\n");
    assert_eq!(String::from_utf8_lossy(&run_with(&exe, b"").stdout), "0\n"); // scanf finds nothing: n stays 0
}

#[test]
fn python_stops_on_a_line_that_is_not_a_number_as_python_does() {
    let r = root("py-input");
    let p = r.join("read.py");
    std::fs::write(&p, "x = int(input())\nprint(x * 2)\n").unwrap();
    let exe = r.join("a.exe");
    assert!(compile_with(&r, &p, &exe, &[], None).status.success());
    for (input, out, code, says) in [
        (&b"21\n"[..], "42\n", 0, ""),
        (b"  -7  \r\n", "-14\n", 0, ""),
        (b"+5", "10\n", 0, ""),
        (b"4 2\n", "", 1, "not a whole number"),
        (b"abc\n", "", 1, "not a whole number"),
        (b"\n", "", 1, "not a whole number"),
        (b"", "", 1, "no line to read"),
        (b"99999999999999999999\n", "", 1, "grew past 64 bits"),
    ] {
        let got = run_with(&exe, input);
        assert_eq!(String::from_utf8_lossy(&got.stdout), out, "{input:?}");
        assert_eq!(got.status.code(), Some(code), "{input:?}");
        assert!(String::from_utf8_lossy(&got.stderr).contains(says), "{input:?}: {}", String::from_utf8_lossy(&got.stderr));
    }
}

#[test]
fn c_integers_have_clangs_widths_in_every_mode() {
    let want = std::fs::read(suite("widths.c.out")).unwrap();
    let r = root("widths-c");
    for mode in ["--plain", "--opt", "--release", "--release-run"] {
        let exe = r.join(format!("w{mode}.exe"));
        let extra: Vec<&str> = match mode {
            "--opt" => vec![],
            "--release-run" => vec!["--release", "--no-evaluate"],
            m => vec![m],
        };
        let o = compile_with(&r, &suite("widths.c"), &exe, &extra, None);
        assert!(o.status.success(), "{}", text(&o));
        let run = Command::new(&exe).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&want), "{mode}");
    }
}

#[test]
fn go_int32_wraps_as_go_does() {
    runs_as_the_real_one("widths.go", "Go");
}

#[test]
fn a_python_number_that_leaves_64_bits_stops_the_program_instead_of_wrapping() {
    let r = root("python-overflow");
    let p = r.join("fact.py");
    std::fs::write(&p, "def fact(n):\n    f = 1\n    for i in range(1, n + 1):\n        f = f * i\n    return f\nprint(fact(20))\nprint(fact(25))\nprint(\"never\")\nx = 1\nprint(x << 70)\n").unwrap();
    for mode in ["--plain", "--opt", "--release", "--release-run"] {
        let exe = r.join(format!("f{mode}.exe"));
        let extra: Vec<&str> = match mode {
            "--opt" => vec![],
            "--release-run" => vec!["--release", "--no-evaluate"],
            m => vec![m],
        };
        let o = compile_with(&r, &p, &exe, &extra, None);
        assert!(o.status.success(), "{}", text(&o));
        let run = Command::new(&exe).output().unwrap();
        // 20! fits; 25! does not, and Python would print it: Strike stops
        assert_eq!(String::from_utf8_lossy(&run.stdout), "2432902008176640000\n", "{mode}");
        assert_eq!(run.status.code(), Some(1), "{mode}");
        assert!(String::from_utf8_lossy(&run.stderr).contains("grew past 64 bits"), "{mode}");
    }
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
    /// A float expression of the given names, the whole numbers i and n,
    /// constants (NaN and the infinities come of them), and calls of the
    /// two float helpers in the middle of it.
    fn fexpr_names(&mut self, names: &[String], depth: u32) -> String {
        if depth == 0 || self.below(4) == 0 {
            return match self.below(10) {
                0..=5 => names[self.below(names.len() as u64) as usize].clone(),
                6 => ["i", "n"][self.below(2) as usize].to_string(),
                _ => ["0.5", "-2.25", "1e300", "1e-300", "0.0", "-0.0", "3", "0.1", "-7.0", "1e-310"][self.below(10) as usize].to_string(),
            };
        }
        let d = depth - 1;
        match self.below(12) {
            0..=5 => format!("({} {} {})", self.fexpr_names(names, d), ["+", "-", "*", "/"][self.below(4) as usize], self.fexpr_names(names, d)),
            6 => format!("-({})", self.fexpr_names(names, d)),
            7 => format!("sqrt(fabs({}))", self.fexpr_names(names, d)),
            8 => format!("(double)(long long)({})", self.fexpr_names(names, d)),
            9 => format!("h0({}, {}, i)", self.fexpr_names(names, d), self.fexpr_names(names, d)),
            10 => format!("h1({}, {}, {}, {}, {}, n)", self.fexpr_names(names, d.min(1)), self.fexpr_names(names, 0), self.fexpr_names(names, 0), self.fexpr_names(names, 0), self.fexpr_names(names, 0)),
            _ => format!("({} {} {})", self.fexpr_names(names, d), ["<", "<=", "==", "!=", ">", ">="][self.below(6) as usize], self.fexpr_names(names, d)),
        }
    }

    /// A C program of float functions with many locals (more than the xmm
    /// registers they may live in), parameters past the fourth (on the
    /// stack), calls and printf between float operations, locals updated
    /// from themselves and from each other, and every result's bits printed.
    fn float_program(&mut self) -> String {
        let mut out = String::from("#include <stdio.h>\n#include <math.h>\n");
        out += "double h0(double a, double b, long long i) {\n    return a * b - i;\n}\n";
        out += "double h1(double a, double b, double c, double d, double e, long long n) {\n    return (a - b) * c + d / (e + n);\n}\n";
        let mut funcs = Vec::new();
        for k in 0..3 {
            let np = 1 + self.below(6) as usize;
            let params: Vec<String> = (0..np).map(|p| format!("p{p}")).collect();
            let nl = 4 + self.below(12) as usize;
            // an i the initializers may use; the loop's own shadows it
            let mut body = String::from("    long long i = n * 3;\n");
            let mut names = params.clone();
            for v in 0..nl {
                body += &format!("    double v{v} = {};\n", self.fexpr_names(&names, 2));
                names.push(format!("v{v}"));
            }
            let locals: Vec<String> = (0..nl).map(|v| format!("v{v}")).collect();
            let mut inner = String::new();
            for _ in 0..2 + self.below(5) {
                let a = &locals[self.below(nl as u64) as usize];
                inner += &match self.below(6) {
                    0 => format!("        {a} = {a} {} {};\n", ["+", "-", "*", "/"][self.below(4) as usize], self.fexpr_names(&names, 2)),
                    1 => format!("        {a} {}= {};\n", ["+", "-", "*", "/"][self.below(4) as usize], self.fexpr_names(&names, 2)),
                    2 => format!("        if ({}) {a} = -{a}; else {a} = {};\n", self.fexpr_names(&names, 1), self.fexpr_names(&names, 2)),
                    3 => format!("        printf(\"%.17g\\n\", (double)({}));\n", self.fexpr_names(&names, 1)),
                    4 => format!("        {a} = {};\n", self.fexpr_names(&names, 3)),
                    _ => format!("        {a} = {} * {a};\n", self.fexpr_names(&names, 1)),
                };
            }
            body += &format!("    for (long long i = 0; i < {}; i++) {{\n{inner}    }}\n", self.below(6));
            let ret: Vec<String> = (0..3).map(|_| locals[self.below(nl as u64) as usize].clone()).collect();
            body += &format!("    return {} + {} * {};\n", ret[0], ret[1], ret[2]);
            let decl: Vec<String> = params.iter().map(|p| format!("double {p}")).collect();
            out += &format!("double f{k}({}, long long n) {{\n{body}}}\n", decl.join(", "));
            funcs.push((format!("f{k}"), np));
        }
        out += "int main() {\n";
        for (f, np) in &funcs {
            for _ in 0..2 {
                let args: Vec<String> = (0..*np).map(|_| ["1.5", "-0.0", "1e308", "3", "0.1", "-2.5e-300", "7", "-1"][self.below(8) as usize].to_string()).collect();
                out += &format!("    printf(\"%.17g %e\\n\", {f}({}, {}), {f}({}, 2));\n", args.join(", "), self.below(5), args.join(", "));
            }
        }
        out + "    return 0;\n}\n"
    }

    /// A C program of integer functions with many locals (more than the
    /// registers): leaf functions, whose locals HotMotor may put in the
    /// volatile registers, and functions with calls, divisions (which take
    /// rdx and r8) and shifts between their uses; parameters used in every
    /// order, so they are moved between argument registers.
    fn int_program(&mut self) -> String {
        let mut out = String::from("#include <stdio.h>\n");
        // h writes the volatile registers: its own locals, a division by a
        // constant and by a variable
        out += "long long h(long long a, long long b) {\n    long long q = a / 7 + b % 5;\n    long long r = (a ^ b) / ((b & 3) + 1);\n    long long t = q * 3 + r - (a >> 2);\n    return t * 3 - (b >> 2) + q;\n}\n";
        // a function of no arguments that stays a call: a comparison
        // waiting as flags before it must keep its value
        out += "long long gz = 3;\nlong long z() {\n    if (gz > 0) {\n        gz = gz - 1;\n        return z() * 2 - 1;\n    }\n    gz = 3;\n    return -783;\n}\n";
        let mut funcs = Vec::new();
        for k in 0..4 {
            let np = 1 + self.below(6) as usize;
            let params: Vec<String> = (0..np).map(|p| format!("p{p}")).collect();
            let nl = 6 + self.below(10) as usize;
            let calls = self.below(2) == 0;
            // C int locals: each store cut to 32 bits (cuts a loop may
            // leave to its way out)
            let ty = ["long long", "int"][self.below(2) as usize];
            let mut names = params.clone();
            names.reverse();
            let mut body = String::new();
            for v in 0..nl {
                body += &format!("    {ty} v{v} = {};\n", self.int_expr(&names, 2, false));
                names.push(format!("v{v}"));
            }
            let locals: Vec<String> = (0..nl).map(|v| format!("v{v}")).collect();
            let mut inner = String::new();
            for _ in 0..3 + self.below(6) {
                let a = &locals[self.below(nl as u64) as usize];
                let b = locals[self.below(nl as u64) as usize].clone();
                inner += &match self.below(8) {
                    // a chain round the loop through a multiplication
                    7 => format!("        {a} = {a} * ({b} + {}) + {} - i;\n", self.below(9), self.int_expr(&names, 1, false)),
                    0 => format!("        {a} = {a} {} {};\n", ["+", "-", "*", "^", "&", "|"][self.below(6) as usize], self.int_expr(&names, 2, calls)),
                    1 => format!("        {a} = ({}) / {};\n", self.int_expr(&names, 2, calls), [3, 7, -5, 1000003][self.below(4) as usize]),
                    2 => format!("        {a} = ({}) % ((({}) & 7) + 1);\n", self.int_expr(&names, 2, calls), self.int_expr(&names, 1, false)),
                    3 => format!("        {a} = ({}) {} {};\n", self.int_expr(&names, 2, calls), ["<<", ">>"][self.below(2) as usize], self.below(64)),
                    4 => format!("        if ({}) {a} = {};\n", self.int_expr(&names, 1, false), self.int_expr(&names, 2, calls)),
                    5 => format!("        {a} = {} * {};\n", self.int_expr(&names, 1, false), [3, 5, 9, 6, -7][self.below(5) as usize]),
                    6 if calls => format!("        printf(\"%lld\\n\", (long long)({}));\n", self.int_expr(&names, 2, false)),
                    _ => format!("        {a} = {};\n", self.int_expr(&names, 3, calls)),
                };
            }
            body += &format!("    for (long long i = 0; i < {}; i++) {{\n{inner}    }}\n", self.below(9));
            let ret: Vec<String> = (0..3).map(|_| names[self.below(names.len() as u64) as usize].clone()).collect();
            body += &if calls {
                // a comparison as the left side of z(), straight in the return
                format!("    return {} + (({} {} {}) {} z());\n", ret[0], ret[1], ["<", "<=", "==", "!="][self.below(4) as usize], ret[2], ["^", "+", "-"][self.below(3) as usize])
            } else {
                format!("    return {} + {} * 3 - {};\n", ret[0], ret[1], ret[2])
            };
            let decl: Vec<String> = params.iter().map(|p| format!("long long {p}")).collect();
            out += &format!("long long f{k}({}) {{\n{body}}}\n", decl.join(", "));
            funcs.push((format!("f{k}"), np));
        }
        out += "int main() {\n";
        for (f, np) in &funcs {
            for _ in 0..2 {
                let args: Vec<String> = (0..*np).map(|_| self.constant()).collect();
                out += &format!("    printf(\"%lld\\n\", {f}({}));\n", args.join(", "));
            }
        }
        out + "    return 0;\n}\n"
    }

    /// An integer expression of the names; with `calls`, h() in it too.
    fn int_expr(&mut self, names: &[String], depth: u32, calls: bool) -> String {
        if depth == 0 || self.below(3) == 0 {
            return if self.below(4) > 0 { names[self.below(names.len() as u64) as usize].clone() } else { self.constant() };
        }
        let d = depth - 1;
        match self.below(8) {
            0..=3 => format!("({} {} {})", self.int_expr(names, d, calls), ["+", "-", "*", "^", "&", "|", "<", "=="][self.below(8) as usize], self.int_expr(names, d, calls)),
            4 => format!("({} >> {})", self.int_expr(names, d, calls), self.below(8)),
            5 if calls => match self.below(2) {
                0 => format!("h({}, {})", self.int_expr(names, d, calls), self.int_expr(names, d, calls)),
                _ => format!("(({} {} {}) {} z())", self.int_expr(names, d, false), ["<", "<=", "==", "!="][self.below(4) as usize], self.int_expr(names, d, false), ["^", "+", "-", "*"][self.below(4) as usize]),
            },
            6 => format!("-({})", self.int_expr(names, d, calls)),
            _ => self.int_expr(names, d, calls),
        }
    }

    /// A C program whose loops read elements other statements of the loop
    /// may or may not write: adjacent global arrays (an index past the end
    /// of one reaches the next), a local array, and an array given to the
    /// function, which may be the very array the loop reads, also written
    /// through another function. Every value the loops leave is printed.
    fn alias_program(&mut self) -> String {
        let mut out = String::from("#include <stdio.h>\n");
        // pad first: an index a little below ga's start stays in the globals
        out += "long long pad[4];\nlong long ga[16];\nlong long gb[16];\nlong long gc[8];\nlong long tail[16];\n";
        out += "void put(long long v[], long long i, long long x) {\n    v[i & 15] = x;\n}\n";
        let mut funcs = Vec::new();
        for k in 0..4 {
            // an index in range, or (of a global array) one a little past
            // either end; the frame's array is kept in range, as what lies
            // next to it differs from mode to mode
            let idx = |g: &mut Gen, v: &str, a: &str| -> String {
                match if a == "la" { 0 } else { g.below(4) } {
                    0 => format!("({v} & 15)"),
                    1 => format!("(({v} & 7) + {})", g.below(12)),
                    2 => format!("({v} - {})", g.below(3)),
                    _ => format!("(({v} + {}) & 15)", g.below(16)),
                }
            };
            let arrays = ["ga", "gb", "la", "v"];
            let mut inner = String::new();
            for _ in 0..2 + self.below(4) {
                let (w, r) = (arrays[self.below(4) as usize], arrays[self.below(4) as usize]);
                let wv = ["j", "i"][self.below(2) as usize];
                let (wi, ri) = (idx(self, wv, w), idx(self, "i", r));
                inner += &match self.below(5) {
                    0 => format!("            {w}[{wi}] = {r}[{ri}] + j;\n"),
                    1 => format!("            {w}[{wi}] += {r}[{ri}] * 3;\n"),
                    2 => format!("            s = s + {r}[{ri}] - gc[{}];\n", self.below(8)),
                    3 => format!("            put({w}, {wi}, s + {r}[{ri}]);\n"),
                    _ => format!("            gc[(j + {}) & 7] = s ^ {r}[{ri}];\n", self.below(8)),
                };
            }
            out += &format!(
                "long long a{k}(long long v[], long long n) {{\n    long long la[16];\n    long long s = n;\n    for (long long q = 0; q < 16; q++) la[q] = q * {} + n;\n    for (long long i = {}; i < {}; i++) {{\n        for (long long j = 0; j < {}; j++) {{\n{inner}            s = s * 3 + la[i & 15];\n        }}\n    }}\n    for (long long q = 0; q < 16; q++) s = s * 7 + la[q];\n    return s;\n}}\n",
                self.below(9),
                self.below(3),
                1 + self.below(6),
                1 + self.below(5)
            );
            funcs.push(format!("a{k}"));
        }
        out += "int main() {\n    for (long long q = 0; q < 16; q++) {\n        ga[q] = q * 5 + 1;\n        gb[q] = 100 - q;\n    }\n";
        for f in &funcs {
            let arg = ["ga", "gb"][self.below(2) as usize];
            out += &format!("    printf(\"%lld\\n\", {f}({arg}, {}));\n", self.constant());
        }
        out += "    long long t = 0;\n    for (long long q = 0; q < 16; q++) t = t * 31 + ga[q] * 7 + gb[q];\n    for (long long q = 0; q < 8; q++) t = t * 31 + gc[q];\n";
        out + "    printf(\"%lld\\n\", t);\n    return 0;\n}\n"
    }

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
                let mut k = match self.below(4) {
                    0 => self.below(19) as i64 - 9,
                    1 => 1 << self.below(40),
                    2 => -(self.below(2_000_000) as i64),
                    _ => self.next() as i64 >> self.below(62),
                };
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
    /// A float expression of the doubles x, y, s and the whole numbers n,
    /// i: every float operator, conversions both ways, constants that
    /// overflow, vanish and are zero of either sign (so NaN and the
    /// infinities come out too, and their bits are printed).
    fn fexpr(&mut self, depth: u32) -> String {
        self.fexpr_of(depth, true)
    }
    /// The same, without s and i where `inner` is false (an initializer).
    fn fexpr_of(&mut self, depth: u32, inner: bool) -> String {
        if depth == 0 || self.below(4) == 0 {
            return match self.below(8) {
                0..=2 => ["x", "y", "s"][self.below(if inner { 3 } else { 2 }) as usize].to_string(),
                3 => ["n", "i"][self.below(if inner { 2 } else { 1 }) as usize].to_string(),
                _ => ["0.5", "-2.25", "1e300", "1e-300", "0.0", "-0.0", "3", "0.1", "-7.0", "1e-310"][self.below(10) as usize].to_string(),
            };
        }
        let d = depth - 1;
        match self.below(10) {
            0..=4 => format!("({} {} {})", self.fexpr_of(d, inner), ["+", "-", "*", "/"][self.below(4) as usize], self.fexpr_of(d, inner)),
            5 => format!("-({})", self.fexpr_of(d, inner)),
            6 => format!("sqrt(fabs({}))", self.fexpr_of(d, inner)),
            7 => format!("(double)(long long)({})", self.fexpr_of(d, inner)),
            8 => format!("(float)({})", self.fexpr_of(d, inner)),
            _ => format!("({} {} {})", self.fexpr_of(d, inner), ["<", "<=", "==", "!=", ">", ">="][self.below(6) as usize], self.fexpr_of(d, inner)),
        }
    }

    /// A ring expression with some atoms: what the simplifier rewrites.
    fn ring(&mut self, names: &[String], depth: u32) -> String {
        if depth == 0 || self.below(5) == 0 {
            return if self.below(4) > 0 { names[self.below(names.len() as u64) as usize].clone() } else { self.constant() };
        }
        let d = depth - 1;
        match self.below(12) {
            0 | 1 => format!("({} + {})", self.ring(names, d), self.ring(names, d)),
            2 => format!("({} - {})", self.ring(names, d), self.ring(names, d)),
            3 | 4 => format!("({} * {})", self.ring(names, d), self.ring(names, d)),
            // a shared factor: f*x + f*y
            5 => {
                let f = self.ring(names, d.min(1));
                format!("({f} * {} + {f} * {} - {} * {f})", self.ring(names, d), self.ring(names, d), self.ring(names, d))
            }
            6 => {
                let k = [3, 5, 9, 7, -3, 8, 1 << 20, -1, 0, 6, 10, 12][self.below(12) as usize];
                format!("({} * {k})", self.ring(names, d))
            }
            7 => format!("({} << {})", self.ring(names, d), self.below(66)),
            8 => format!("{}({})", ["-", "~"][self.below(2) as usize], self.ring(names, d)),
            // a polynomial in one name
            9 => {
                let x = names[self.below(names.len() as u64) as usize].clone();
                format!("({} * {x} * {x} + {} * {x} + {})", self.below(20) as i64 - 10, self.below(20) as i64 - 10, self.constant())
            }
            // atoms: division and remainder by constants, a comparison, a shift right
            10 => format!("({} {} {})", self.ring(names, d), ["/", "%"][self.below(2) as usize], [3, 7, -5, 1000003][self.below(4) as usize]),
            _ => format!("({} {} {})", self.ring(names, d), ["<", ">>", "^"][self.below(3) as usize], self.below(5) + 1),
        }
    }

    /// C structs: nested, with arrays of them and arrays in them, given
    /// and returned by value (copies), changed through a pointer, copied
    /// whole and element by element. Their code, and main's calls of them.
    fn structs(&mut self) -> (String, String) {
        let mut out = String::new();
        out += "struct G0 {\n    long long a;\n    int b;\n    char c;\n    short d[3];\n};\nstruct G1 {\n    struct G0 in;\n    long long e;\n    struct G0 arr[2];\n};\n";
        let n = ["n".to_string()];
        let fields = ["s.a", "s.b", "s.c", "s.d[0]", "s.d[2]", "n"].map(String::from);
        out += &format!(
            "struct G0 gmk(long long n) {{\n    struct G0 s = {{{}, {}, {}, {{{}, {}}}}};\n    s.d[2] = {};\n    return s;\n}}\n",
            self.expr(&n, &[], 2),
            self.expr(&n, &[], 2),
            self.expr(&n, &[], 1),
            self.expr(&n, &[], 1),
            self.expr(&n, &[], 1),
            self.expr(&fields, &[], 2)
        );
        out += &format!(
            "long long gsum(struct G0 s) {{\n    s.a += {};\n    s.d[1] = {};\n    return s.a + s.b * 3 + s.c + s.d[0] + s.d[1] + s.d[2];\n}}\n",
            self.expr(&fields[..5], &[], 2),
            self.expr(&fields[..5], &[], 2)
        );
        let bump: Vec<String> = ["k", "p->in.a", "p->arr[0].c", "p->e", "p->arr[1].d[1]"].iter().map(|s| s.to_string()).collect();
        out += &format!(
            "void gbump(struct G1 *p, long long k) {{\n    p->in.a += k;\n    p->arr[k & 1].b = {};\n    p->e = {};\n}}\n",
            self.expr(&bump[..3], &[], 2),
            self.expr(&bump, &[], 2)
        );
        let mut structs = Vec::new();
        for k in 0..3 {
            let outer: Vec<String> = ["n", "x.in.a", "x.e", "y.arr[1].b", "y.in.d[1]"].iter().map(|s| s.to_string()).collect();
            let inner: Vec<String> = ["n", "i", "list[i].a", "list[(i + 1) & 3].c", "x.arr[i & 1].d[2]"].iter().map(|s| s.to_string()).collect();
            out += &format!(
                "long long st{k}(long long n) {{\n    struct G1 x = {{gmk(n), {}, {{gmk(n + 1), gmk(n - 1)}}}};\n    struct G1 y = x;\n    y.arr[0].a = {};\n    gbump(&y, n & 7);\n    struct G0 list[4] = {{{{0}}}};\n    for (long long i = 0; i < 4; i++) {{\n        list[i] = gmk(i * n);\n        list[i].d[i % 3] += {};\n    }}\n    list[n & 3] = y.arr[1];\n    y.arr[0] = list[{}];\n    return gsum(x.in) + gsum(y.in) * 7 + x.arr[0].a - y.arr[0].a + gsum(list[(n + 1) & 3]) + y.e + x.e + {};\n}}\n",
                self.expr(&outer[..1], &[], 2),
                self.expr(&outer, &[], 2),
                self.expr(&inner, &[], 2),
                self.below(4),
                self.expr(&outer, &[], 3)
            );
            structs.push(format!("st{k}"));
        }
        let mut calls = String::new();
        for f in &structs {
            calls += &format!("    printf(\"%lld\\n\", {f}({}));\n", self.constant());
        }
        (out, calls)
    }

    fn program(&mut self) -> String {
        let mut out = String::from("#include <stdio.h>\n#include <string.h>\n");
        // two globals every function may read and change
        out += &format!("long long g0 = {};\nlong long g1;\n", self.constant());
        let mut funcs: Vec<(String, usize)> = Vec::new();
        for k in 0..6 {
            let n = self.below(7) as usize;
            let params: Vec<String> = (0..n).map(|i| format!("p{i}")).collect();
            let mut names = params.clone();
            names.extend(["g0".to_string(), "g1".to_string()]);
            let mut body = String::new();
            if self.below(2) == 0 {
                body += &format!("    g{} = g{} + {};\n", k % 2, (k + 1) % 2, self.expr(&names, &funcs, 2));
            }
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
        // recursive functions: every shape a return may call itself in
        let mut recs = Vec::new();
        for k in 0..4 {
            let names = vec!["n".to_string(), "a".to_string()];
            let base = self.expr(&names, &funcs, 2);
            let x = self.expr(&names, &funcs, 2);
            let arg = self.expr(&names, &funcs, 2);
            let (call, depth) = match self.below(6) {
                0 => (format!("{x} + r{k}(n - 1, {arg})"), 25),
                1 => (format!("{x} * r{k}(n - 1, {arg})"), 25),
                2 => (format!("r{k}(n - 1, {arg})"), 25),
                3 => (format!("r{k}(n - 1, {arg}) - {x}"), 25),
                4 => (format!("r{k}(n - 1, a) + r{k}(n - 2, {arg})"), 16),
                _ => (format!("{x} + r{k}(n - 1, {arg}) * 3"), 25),
            };
            out += &format!(
                "long r{k}(long n, long a) {{\n    if (n <= 0) return {base};\n    if ({}) return {x} + a;\n    return {call};\n}}\n",
                self.expr(&names, &funcs, 1)
            );
            recs.push((format!("r{k}"), depth));
        }
        // loops that only add up a polynomial of their counter, in every
        // shape the closed form takes, and some it must leave alone
        let mut sums = Vec::new();
        for k in 0..3 {
            let c = |g: &mut Gen| -> String {
                match g.below(4) {
                    0 => format!("{}", g.next() as i64 >> g.below(64)),
                    _ => format!("{}", g.below(50) as i64 - 25),
                }
            };
            let mut poly = c(self);
            for _ in 0..self.below(4) {
                let term = match self.below(5) {
                    0 => format!("i * {}", c(self)),
                    1 => format!("i * i * {}", c(self)),
                    2 => format!("i * i * i * {}", c(self)),
                    3 => format!("(i << {})", self.below(70)),
                    _ => format!("-(i * {})", c(self)),
                };
                poly = format!("({poly} {} {term})", ["+", "-"][self.below(2) as usize]);
            }
            let (bound, arg) = match self.below(3) {
                0 => (format!("< {}", self.below(3000)), String::new()),
                1 => (format!("<= {}", self.below(3000)), String::new()),
                _ => ("< n".to_string(), format!("{}", self.below(3000) as i64 - 50)),
            };
            let stmt = match self.below(5) {
                0 => format!("s = s + {poly};"),
                1 => format!("s += {poly};"),
                2 => format!("s = {poly} + s;"),
                3 => format!("s -= {poly};"),
                _ => format!("s = s * 3 + {poly};"), // not a sum: must stay a loop
            };
            out += &format!(
                "long z{k}(long n) {{\n    long s = {};\n    for (long i = {}; i {bound}; i++) {{\n        {stmt}\n    }}\n    return s + n;\n}}\n",
                c(self),
                self.below(40)
            );
            sums.push((format!("z{k}"), if arg.is_empty() { format!("{}", self.below(100)) } else { arg }));
        }
        // arrays: a local one in the function's own memory, a global one,
        // one given to another function; every index held in range by & 15
        out += "long long garr[16];\nlong long asum(long long v[], long long n) {\n    long long s = 0;\n    for (long long i = 0; i < n; i++) {\n        s += v[i] * (i + 1);\n    }\n    return s;\n}\n";
        let mut arrays = Vec::new();
        for k in 0..3 {
            let names: Vec<String> = ["n", "i", "arr[(i * 7) & 15]", "garr[(n + i) & 15]", "arr[n & 15]"].iter().map(|s| s.to_string()).collect();
            let outer = vec!["n".to_string(), "arr[3]".to_string(), "garr[n & 15]".to_string()];
            out += &format!(
                "long long a{k}(long long n) {{\n    long long arr[16] = {{{}, {}}};\n    for (long long i = 0; i < 16; i++) {{\n        arr[({}) & 15] += {};\n        garr[i] = arr[i] ^ {};\n    }}\n    arr[({}) & 15] = arr[({}) & 15] - {};\n    return asum(arr, 16) + {};\n}}\n",
                self.constant(),
                self.constant(),
                self.expr(&names, &[], 2),
                self.expr(&names, &[], 3),
                self.expr(&names, &[], 2),
                self.expr(&outer, &[], 2),
                self.expr(&outer, &[], 2),
                self.expr(&outer, &[], 2),
                self.expr(&outer, &[], 3)
            );
            arrays.push(format!("a{k}"));
        }
        // floats: a loop of float expressions into a double, and the bits
        // of every result printed (NaN's sign too)
        let mut floats = Vec::new();
        for k in 0..3 {
            out += &format!(
                "double d{k}(double x, double y, long long n) {{\n    double s = {};\n    for (long long i = 0; i < {}; i++) {{\n        if ({}) s = {}; else s += {};\n    }}\n    return s * {};\n}}\n",
                self.fexpr_of(3, false),
                self.below(12),
                self.fexpr(2),
                self.fexpr(3),
                self.fexpr(2),
                self.fexpr_of(2, false)
            );
            floats.push(format!("d{k}"));
        }
        // strings: a char array made from a literal and changed in place,
        // a pointer to a literal, strlen (compared unsigned) and strcmp
        let word = |g: &mut Gen| -> String { (0..1 + g.below(20)).map(|_| (b'a' + g.below(26) as u8) as char).collect() };
        let mut texts = Vec::new();
        for k in 0..3 {
            let names = vec!["n".to_string(), "i".to_string(), "buf[i]".to_string(), "p[i % 3]".to_string()];
            out += &format!(
                "long long t{k}(long long n) {{\n    char buf[24] = \"{}\";\n    const char *p = \"{}\";\n    for (long long i = 0; buf[i]; i++) {{\n        buf[i] = 'a' + (({}) & 15);\n    }}\n    printf(\"%s %c %zu\\n\", buf, buf[0], strlen(p));\n    return strlen(buf) * 3 + strcmp(buf, p) + (strlen(buf) > n) + (n <= strlen(p));\n}}\n",
                word(self),
                // at least three chars: p[i % 3] is always one of them or its zero
                word(self) + "abc",
                self.expr(&names, &[], 3)
            );
            texts.push(format!("t{k}"));
        }
        // work across statements: an unrolled counter's copies (known low
        // bits), locals made again of what they hold, shared expressions,
        // C int values whose ranges make their cuts needless
        let mut across = Vec::new();
        for k in 0..3 {
            let names: Vec<String> = ["i", "a", "b", "c", "n"].iter().map(|s| s.to_string()).collect();
            let (from, to) = (self.below(3), self.below(41));
            let (x, sh, m) = (self.below(5000), self.below(6), self.below(300) as i64 - 150);
            let shared = self.ring(&names, 2);
            let e1 = self.ring(&names, 2);
            let e2 = self.ring(&names, 2);
            out += &format!(
                "long long w{k}(long long n) {{\n    long long s = 0;\n    int t = 0;\n    for (int i = {from}; i < {to}; i++) {{\n        long long a = i ^ {x};\n        long long b = (i + 1) >> {sh};\n        int c = (i & 1023) * {m} + (i >> 2);\n        b = (i + 1) >> {sh};\n        s = s + {shared} * {e1} + a * b;\n        t = t + c * 3 + (i << 2);\n        s = s - {shared} + ({e2} & 255);\n    }}\n    return s + t;\n}}\n"
            );
            across.push(format!("w{k}"));
        }
        // ring expressions for the release simplifier: shared factors,
        // polynomials, constant multiples, C long (cut to 32 bits) and long
        // long, index arithmetic in loops, atoms inside
        let mut rings = Vec::new();
        out += "long long rarr[64];\n";
        for k in 0..4 {
            let ty = ["long", "long long"][self.below(2) as usize];
            let names: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
            let inner: Vec<String> = ["a", "b", "c", "i", "j", "s"].iter().map(|s| s.to_string()).collect();
            let (n, m) = (self.below(9), self.below(9));
            let first = self.ring(&names, 3);
            let body = self.ring(&inner, 3);
            let index = self.ring(&inner, 2);
            let last = self.ring(&names, 4);
            out += &format!(
                "{ty} q{k}({ty} a, {ty} b, {ty} c) {{\n    {ty} s = {first};\n    for ({ty} i = 0; i < {n}; i++) {{\n        for ({ty} j = 0; j < {m}; j++) {{\n            s = s + {body} + rarr[({index}) & 63];\n            rarr[(i * {m} + j) & 63] = s;\n        }}\n    }}\n    return s + {last};\n}}\n"
            );
            rings.push(format!("q{k}"));
        }
        // structs, from a generator of their own: the rest of each program
        // is what it was before them
        let (code, calls) = Gen(self.0 ^ 0xA076_1D64_78BD_642F).structs();
        out += &code;
        out += "int main() {\n";
        out += &calls;
        for f in &floats {
            let c = |g: &mut Gen| ["1.5", "-0.0", "1e308", "3", "0.1", "-2.5e-300"][g.below(6) as usize];
            out += &format!("    printf(\"%.17g %e %g\\n\", {f}({}, {}, {}), {f}(2, -1, 5), {f}({}, 0.5, -3));\n", c(self), c(self), self.constant(), c(self));
        }
        for f in &texts {
            out += &format!("    printf(\"%lld\\n\", {f}({}));\n", self.constant());
        }
        for f in &rings {
            out += &format!("    printf(\"%lld\\n\", {f}({}, {}, {}));\n", self.constant(), self.constant(), self.constant());
        }
        for f in &across {
            out += &format!("    printf(\"%lld\\n\", {f}({}));\n", self.constant());
        }
        for (f, a) in &sums {
            out += &format!("    printf(\"%lld\\n\", {f}({a}));\n");
        }
        for f in &arrays {
            out += &format!("    printf(\"%lld\\n\", {f}({}));\n", self.constant());
        }
        out += "    printf(\"%lld %lld\\n\", garr[0], garr[15]);\n";
        for (f, n) in &funcs {
            let args: Vec<String> = (0..*n).map(|_| self.constant()).collect();
            out += &format!("    printf(\"%lld\\n\", {f}({}));\n", args.join(", "));
        }
        for (f, depth) in &recs {
            out += &format!("    printf(\"%lld\\n\", {f}({}, {}));\n", self.below(*depth), self.constant());
        }
        out += "    printf(\"%lld %lld\\n\", g0, g1);\n";
        out + "    return 0;\n}\n"
    }
}

#[test]
fn the_optimized_code_prints_what_the_plain_code_prints() {
    let r = root("differ");
    let rules = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("codegen.toml")).unwrap();
    let magic = r.join("magic.toml");
    // division by multiplying, and nothing worked out while compiling, so
    // the machine code itself is what runs
    let magic_rules = rules
        .replace("magic_division = false", "magic_division = true")
        .replace("max_steps = 20000000", "max_steps = 0")
        .replace("max_seconds = 2.0", "max_seconds = 0.0");
    assert!(magic_rules.contains("max_steps = 0") && magic_rules.contains("max_seconds = 0.0"));
    std::fs::write(&magic, magic_rules).unwrap();
    let mut g = Gen(0x9E37_79B9_7F4A_7C15);
    for n in 0..100 {
        let src = r.join(format!("p{n}.c"));
        std::fs::write(&src, g.program()).unwrap();
        let mut outs = Vec::new();
        for mode in ["--plain", "--opt", "--release", "--release-magic", "--release-run"] {
            let exe = r.join(format!("p{n}{mode}.exe"));
            let mut c = Command::new(env!("CARGO_BIN_EXE_strike"));
            c.args(["compile", src.to_str().unwrap(), "-o", exe.to_str().unwrap()]).env("STRIKE_HOME", r.join("home"));
            match mode {
                "--opt" => {}
                "--release-magic" => {
                    c.arg("--release").env("STRIKE_CODEGEN", &magic);
                }
                // the release optimizations as machine code that runs
                "--release-run" => {
                    c.args(["--release", "--no-evaluate"]);
                }
                m => {
                    c.arg(m);
                }
            }
            let o = c.output().unwrap();
            assert!(o.status.success(), "p{n}.c: {}", text(&o));
            let run = Command::new(&exe).output().unwrap();
            outs.push((run.status.code(), run.stdout));
        }
        assert_eq!(outs[0], outs[1], "p{n}.c: the optimized and the plain code differ");
        assert_eq!(outs[0], outs[2], "p{n}.c: the release and the plain code differ");
        assert_eq!(outs[0], outs[3], "p{n}.c: division by multiplying and the plain code differ");
        assert_eq!(outs[0], outs[4], "p{n}.c: the release optimizations and the plain code differ");
    }
}

/// Random C programs of float functions with many locals, where HotMotor
/// puts floats in xmm registers (some, as there are only ten of them), and
/// calls and printf go between their operations: every mode prints the
/// same bits as the plain code, NaN's sign included.
#[test]
fn float_locals_in_registers_print_what_the_plain_code_prints() {
    let r = root("float-locals");
    let mut g = Gen(0x2545_F491_4F6C_DD1D);
    for n in 0..40 {
        let src = r.join(format!("f{n}.c"));
        std::fs::write(&src, g.float_program()).unwrap();
        let mut outs = Vec::new();
        for mode in [vec!["--plain"], vec![], vec!["--release"], vec!["--release", "--no-evaluate"]] {
            let exe = r.join(format!("f{n}.exe"));
            let o = compile_with(&r, &src, &exe, &mode, None);
            assert!(o.status.success(), "f{n}.c: {}", text(&o));
            let run = Command::new(&exe).output().unwrap();
            outs.push((run.status.code(), run.stdout));
        }
        for (i, o) in outs.iter().enumerate().skip(1) {
            assert_eq!(&outs[0], o, "f{n}.c: mode {i} and the plain code differ");
        }
    }
}

/// Builds `n` random programs of `gen` in the plain mode, the default and
/// --release --no-evaluate: each must print, and return, what the plain code
/// does.
fn every_mode_as_plain(dir: &str, seed: u64, n: usize, gen: fn(&mut Gen) -> String) {
    let r = root(dir);
    let mut g = Gen(seed);
    for k in 0..n {
        let src = r.join(format!("p{k}.c"));
        std::fs::write(&src, gen(&mut g)).unwrap();
        let mut outs = Vec::new();
        for mode in [vec!["--plain"], vec![], vec!["--release", "--no-evaluate"]] {
            let exe = r.join(format!("p{k}.exe"));
            let o = compile_with(&r, &src, &exe, &mode, None);
            assert!(o.status.success(), "{dir}/p{k}.c: {}", text(&o));
            let run = Command::new(&exe).output().unwrap();
            outs.push((run.status.code(), run.stdout));
        }
        for (i, o) in outs.iter().enumerate().skip(1) {
            assert_eq!(&outs[0], o, "{dir}/p{k}.c: mode {i} and the plain code differ");
        }
    }
}

/// Random C programs of integer functions with more locals than registers,
/// leaf functions and functions with calls and divisions between the uses
/// of their locals (so locals live in volatile registers only where nothing
/// writes them): every mode prints what the plain code prints.
#[test]
fn integer_locals_in_every_register_print_what_the_plain_code_prints() {
    every_mode_as_plain("int-locals", 0x1234_5678_9ABC_DEF1, 40, Gen::int_program);
}

/// Random C programs whose loops read elements that writes of the same
/// loop may reach: adjacent globals (an index past an array's end), a local
/// array, and an array given as a parameter that is the one the loop reads.
/// An element moved out of a loop must never be one a write reaches.
#[test]
fn elements_moved_out_of_loops_are_never_ones_the_loop_writes() {
    every_mode_as_plain("alias", 0x0DDB_A11C_AFE5_7EA5, 40, Gen::alias_program);
}

fn compile_with(home: &Path, src: &Path, exe: &Path, extra: &[&str], rules: Option<&Path>) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_strike"));
    c.args(["compile", src.to_str().unwrap(), "-o", exe.to_str().unwrap()]).args(extra).env("STRIKE_HOME", home.join("home"));
    if let Some(r) = rules {
        c.env("STRIKE_CODEGEN", r);
    }
    c.output().unwrap()
}

#[test]
fn a_release_build_of_a_program_that_reads_nothing_gives_its_output_as_it_is() {
    let r = root("worked-out");
    let rules = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("codegen.toml")).unwrap();
    let run_once = r.join("run-once.toml");
    std::fs::write(&run_once, rules.replace("max_steps = 20000000", "max_steps = 0")).unwrap();
    for file in ["suite.c", "suite.go", "suite.py"] {
        let want = std::fs::read(suite(&format!("{file}.out"))).unwrap();
        for (rules, says) in [(None, "IR runner"), (Some(run_once.as_path()), "run once while compiling")] {
            let exe = r.join(format!("{file}.exe"));
            let o = compile_with(&r, &suite(file), &exe, &["--release"], rules);
            assert!(o.status.success() && text(&o).contains(says), "{file}: {}", text(&o));
            let run = Command::new(&exe).output().unwrap();
            assert_eq!(String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&want), "{file} ({says})");
        }
    }
    // the number main returns is the exit code, worked out or not
    let p = r.join("seven.c");
    std::fs::write(&p, "#include <stdio.h>\nint main() {\n    puts(\"hi\");\n    return 7;\n}\n").unwrap();
    for rules in [None, Some(run_once.as_path())] {
        let exe = r.join("seven.exe");
        assert!(compile_with(&r, &p, &exe, &["--release"], rules).status.success());
        let run = Command::new(&exe).output().unwrap();
        assert_eq!((run.status.code(), String::from_utf8_lossy(&run.stdout).to_string()), (Some(7), "hi\n".to_string()));
    }
}

#[test]
fn a_program_that_fails_is_not_worked_out_and_fails_as_it_would() {
    let r = root("fails");
    let p = r.join("zero.c");
    std::fs::write(&p, "#include <stdio.h>\nint main() {\n    long z = 0;\n    puts(\"before\");\n    printf(\"%lld\\n\", 1 / z);\n    return 0;\n}\n").unwrap();
    let rules = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("codegen.toml")).unwrap();
    let run_once = r.join("run-once.toml");
    std::fs::write(&run_once, rules.replace("max_steps = 20000000", "max_steps = 0")).unwrap();
    let plain = r.join("plain.exe");
    assert!(compile_with(&r, &p, &plain, &["--plain"], None).status.success());
    let want = Command::new(&plain).output().unwrap();
    assert!(!want.status.success());
    for rules in [None, Some(run_once.as_path())] {
        let exe = r.join("release.exe");
        let o = compile_with(&r, &p, &exe, &["--release"], rules);
        assert!(o.status.success() && !text(&o).contains("reads nothing"), "{}", text(&o));
        let got = Command::new(&exe).output().unwrap();
        assert_eq!((got.status.code(), got.stdout), (want.status.code(), want.stdout.clone()));
    }
}

#[test]
fn a_frame_larger_than_a_page_runs_in_every_mode() {
    let r = root("frame");
    let mut src = String::from("#include <stdio.h>\nlong f(long a) {\n");
    for i in 0..1500 {
        src += &format!("    long v{i} = a + {i};\n");
    }
    src += "    return v0 + v1499;\n}\nint main() {\n    printf(\"%lld\\n\", f(1));\n    return 0;\n}\n";
    let p = r.join("frame.c");
    std::fs::write(&p, src).unwrap();
    for mode in ["--plain", "--release"] {
        let exe = r.join(format!("f{mode}.exe"));
        let o = Command::new(env!("CARGO_BIN_EXE_strike"))
            .args(["compile", p.to_str().unwrap(), "-o", exe.to_str().unwrap(), mode])
            .env("STRIKE_HOME", r.join("home"))
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", text(&o));
        let run = Command::new(&exe).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&run.stdout), "1501\n", "{mode}: exit {:?}", run.status.code());
    }
}

#[test]
fn what_strike_cannot_do_yet_is_refused_not_guessed() {
    let r = root("refused");
    for (name, src, says) in [
        ("chain.py", "x = 1\nprint(0 < x < 2)\n", "chained comparison"),
        // floats: what Strike cannot do the language's way
        ("exact.go", "package main\nimport \"fmt\"\nfunc main() {\n\tfmt.Println(0.1 + 0.2)\n}\n", "constant expressions exactly"),
        ("thirds.go", "package main\nimport \"fmt\"\nfunc main() {\n\tfmt.Println(1.0 / 3.0 * 3.0)\n}\n", "constant expressions exactly"),
        ("mix.go", "package main\nimport \"fmt\"\nfunc main() {\n\ti := 1\n\tx := 1.5\n\tfmt.Println(x + i)\n}\n", "Go does not mix whole numbers and floats"),
        ("widths.go", "package main\nimport \"fmt\"\nfunc main() {\n\tvar f float32 = 1\n\tx := 1.5\n\tfmt.Println(x * f)\n}\n", "a float64 and a float32"),
        ("truncated.go", "package main\nimport \"fmt\"\nfunc main() {\n\tfmt.Println(int(-2.9))\n}\n", "not a whole number"),
        ("over32.go", "package main\nimport \"fmt\"\nfunc main() {\n\tvar f float32 = 1e40\n\tfmt.Println(f)\n}\n", "overflows float32"),
        ("kind.py", "x = 0\nx = x + 0.5\nprint(x)\n", "x holds a number in one place and a float in another"),
        ("param.py", "def f(a):\n    return a\nprint(f(1), f(1.5))\n", "a float in one place and a whole number in another"),
        ("floats.py", "print([1, 2.5])\n", "one kind per list"),
        ("floor.py", "import math\nprint(math.floor(2.5))\n", "math.floor is not in Strike yet"),
        ("mod.c", "#include <stdio.h>\nint main() { double d = 5.5; printf(\"%f\", d % 2); return 0; }\n", "not one C has for floats"),
        ("long.c", "int main() { long double d = 1; return 0; }\n", "long double is not in Strike yet"),
        ("scan.c", "#include <stdio.h>\nint main() { double d; scanf(\"%d\", &d); return 0; }\n", "only a whole number is read"),
        ("percentd.c", "#include <stdio.h>\nint main() { printf(\"%d\", 1.5); return 0; }\n", "%d is given a float"),
        ("percentf.c", "#include <stdio.h>\nint main() { printf(\"%f\", 1); return 0; }\n", "%f is given a number, not a float"),
        ("precision.c", "#include <stdio.h>\nint main() { printf(\"%.200f\", 1.5); return 0; }\n", "a precision above 100"),
        ("width.c", "#include <stdio.h>\nint main() { printf(\"%8.3f\", 1.5); return 0; }\n", "the format %8 is not in Strike yet"),
        ("undeclared.c", "int main() { return y; }\n", "y is not declared"),
        ("none.py", "def f(x):\n    if x:\n        return 1\nprint(f(0))\n", "can end without returning"),
        ("nolang.txt", "SELECT * FROM t;\n", "no description reads the whole file"),
        ("unsigned.c", "int main() { unsigned int x = 1; return x; }\n", "unsigned numbers are not in Strike yet"),
        ("pointer.c", "int main() { int *p; return 0; }\n", "pointer variables are not in Strike yet"),
        // structs and classes: what is not exact yet
        ("float_field.c", "struct P { float f; };\nint main() { return 0; }\n", "a float field is not in Strike yet"),
        ("holds_itself.c", "struct P { int a; struct P b; };\nint main() { return 0; }\n", "holds itself"),
        ("struct_math.c", "struct P { int a; };\nint main() { struct P s; struct P *p = &s; p = p + 1; return 0; }\n", "this operator on a pointer"),
        ("addr_int.c", "int main() { int x = 1; int *p = &x; return 0; }\n", "& of a number is not in Strike yet"),
        ("struct_d.c", "#include <stdio.h>\nstruct P { int a; };\nint main() { struct P s = {1}; printf(\"%d\", s); return 0; }\n", "%d is given a struct P"),
        ("tag.c", "struct P { int a; };\nint main() { P s; return 0; }\n", "P is a struct's tag: C writes it struct P"),
        ("excess.c", "struct P { int a; };\nint main() { struct P s = {1, 2}; return s.a; }\n", "excess elements"),
        ("arrow.c", "struct P { int a; };\nint main() { struct P s; struct P *p = &s; return p.a; }\n", "C reaches its fields with ->"),
        ("sizeof_expr.c", "int main() { int a = 1; return sizeof(a + 1); }\n", "sizeof of this expression is not in Strike yet"),
        ("ptr_print.go", "package main\nimport \"fmt\"\ntype N struct {\n\tV int\n\tNext *N\n}\nfunc main() {\n\tfmt.Println(N{})\n}\n", "which Go prints as its address"),
        ("ptr_method.go", "package main\nimport \"fmt\"\ntype C struct {\n\tn int\n}\nfunc (c *C) Inc() {\n\tc.n++\n}\nfunc mk() C {\n\treturn C{}\n}\nfunc main() {\n\tmk().Inc()\n\tfmt.Println(1)\n}\n", "the value has no address"),
        ("few.go", "package main\nimport \"fmt\"\ntype C struct {\n\tA, B int\n}\nfunc main() {\n\tfmt.Println(C{1})\n}\n", "too few values in struct literal"),
        ("struct_eq.go", "package main\nimport \"fmt\"\ntype C struct {\n\tA int\n}\nfunc main() {\n\tfmt.Println(C{1} == C{1})\n}\n", "this operator on a struct C and a struct C"),
        ("error_method.go", "package main\nimport \"fmt\"\ntype E struct {\n\tA int\n}\nfunc (e E) Error() string {\n\treturn \"e\"\n}\nfunc main() {\n\tfmt.Println(E{1})\n}\n", "has a Error method"),
        ("nested_string.go", "package main\nimport \"fmt\"\ntype M struct {\n\tA int\n}\nfunc (m M) String() string {\n\treturn \"m\"\n}\nfunc main() {\n\tfmt.Println([]M{{1}})\n}\n", "has a String method"),
        ("byte_field.go", "package main\nimport \"fmt\"\ntype B struct {\n\tX byte\n}\nfunc main() {\n\tfmt.Println(B{1})\n}\n", "a byte field is not in Strike yet"),
        ("no_repr.py", "class P:\n    pass\n\n\nprint(P())\n", "an address Strike cannot give as Python does"),
        ("str_list.py", "class P:\n    def __str__(self):\n        return \"p\"\n\n\nprint([P()])\n", "by its __repr__, and P has none"),
        ("static.py", "class P:\n    def f():\n        return 1\n\n\nprint(1)\n", "a static method is not in Strike yet"),
        ("isinstance.py", "class P:\n    pass\n\n\nprint(isinstance(P(), P))\n", "there is no function isinstance"),
        ("obj_eq.py", "class P:\n    pass\n\n\nprint(P() == P())\n", "this operator on an object of class P"),
        ("obj_kinds.py", "class P:\n    def __init__(self, x):\n        self.x = x\n\n\na = P(1)\nb = P(1.5)\nprint(a.x, b.x)\n", "a float in one place and a whole number in another"),
        ("class_call.py", "class P:\n    def f(self):\n        return 1\n\n\nprint(P.f(P()))\n", "a method called on its class is not in Strike yet"),
        ("obj_cond.py", "class P:\n    pass\n\n\nif P():\n    print(1)\n", "is not a condition"),
        ("vla.c", "int main() { int n = 3; int a[n]; return 0; }\n", "no variable-length arrays"),
        ("mixed.py", "a = [1, True]\nprint(a)\n", "one kind per list"),
        ("uint.go", "package main\nfunc main() {\n\tvar x uint = 1\n\tx = x + 1\n}\n", "unsigned numbers are not in Strike yet"),
        ("unbound.py", "n = 1\ndef f():\n    print(n)\n    n = 2\nf()\n", "read before this function assigns it"),
        // the preprocessor: what it does not do, and a header it does not know
        ("pragma.c", "#pragma pack(1)\nint main() { return 0; }\n", "#pragma pack(1) is not in Strike yet"),
        ("windows.c", "#include <windows.h>\nint main() { return 0; }\n", "the header <windows.h> is not one Strike knows"),
        ("error.c", "#if 1\n#error stop here\n#endif\nint main() { return 0; }\n", "error.c:2: #error stop here"),
        ("file.c", "#include <stdio.h>\nint main() { puts(__FILE__); return 0; }\n", "__FILE__ depends on how and when clang is run"),
        ("unterminated.c", "#ifdef X\nint main() { return 0; }\n", "unterminated conditional directive"),
        // packages and modules
        ("init.go", "package main\nfunc init() {\n}\nfunc main() {\n}\n", "a package's init function is not in Strike yet"),
        ("lib.go", "package lib\nfunc main() {\n}\n", "package lib is not a main package"),
        ("os.go", "package main\nimport \"os\"\nfunc main() {\n}\n", "the package os is not in Strike yet"),
        ("sprintf.go", "package main\nimport \"fmt\"\nfunc main() {\n\tx := fmt.Sprint(1)\n\tfmt.Println(x)\n}\n", "fmt.Sprint is not in Strike yet"),
        ("os.py", "import os\nprint(1)\n", "os is one of Python's own modules"),
        ("mathfrom.py", "from math import sqrt\nprint(sqrt(2.0))\n", "from math import ... is not in Strike yet"),
        ("mathas.py", "import math as m\nprint(m.sqrt(2.0))\n", "import math as ... is not in Strike yet"),
        ("missing.py", "import nowhere\nprint(1)\n", "there is no nowhere.py next to missing.py"),
        ("nested.py", "def f():\n    import nowhere\n    return 1\nprint(f())\n", "an import inside a function or a block is not in Strike yet"),
        // strings: what is not exact yet
        ("unicode.py", "s = \"caf\u{e9}\"\nprint(len(s))\n", "past ASCII"),
        ("unicode_print.py", "print(\"caf\u{e9}\")\n", "past ASCII"),
        ("change.py", "s = \"abc\"\ns[0] = \"x\"\n", "a string is not changed in place"),
        ("str_int.py", "print(\"a\" + 1)\n", "this operator on a string and a number"),
        ("list_in.py", "print(1 in [1, 2])\n", "this operator on a number and a list"),
        ("split_sep.py", "print(\"a,b\".split(\",\"))\n", ".split() with these values"),
        ("bytes.go", "package main\nimport \"fmt\"\nfunc main() {\n\tb := []byte{1}\n\tfmt.Println(b)\n}\n", "a slice or an array of bytes is not in Strike yet"),
        ("atoi.go", "package main\nimport \"strconv\"\nfunc main() {\n\tn := strconv.Atoi(\"1\")\n\tn = n + 1\n}\n", "strconv.Atoi is not in Strike yet"),
        ("quote_int.go", "package main\nimport \"fmt\"\nfunc main() {\n\tfmt.Printf(\"%q\\n\", 65)\n}\n", "%q is given a number"),
        ("string_cond.go", "package main\nimport \"fmt\"\nfunc main() {\n\ts := \"a\"\n\tif s {\n\t\tfmt.Println(s)\n\t}\n}\n", "a string is not a condition"),
        ("pointer_math.c", "#include <stdio.h>\nint main() { const char *p = \"abc\"; puts(p + 1); return 0; }\n", "this operator on a pointer"),
        ("strlen_div.c", "#include <string.h>\nint main() { return strlen(\"abcd\") / 2; }\n", "unsigned division"),
        ("char_array.c", "int main() { char s[2] = \"abc\"; return s[0]; }\n", "a string of 3 chars is given to an array of 2"),
        ("int_string.c", "int main() { int s[4] = \"abc\"; return s[0]; }\n", "only a char array is given a string"),
    ] {
        let p = r.join(name);
        std::fs::write(&p, src).unwrap();
        let o = compile(&r, &p, &r.join("a.exe"));
        assert!(!o.status.success(), "{name} compiled");
        assert!(text(&o).contains(says), "{name}: {}", text(&o));
    }
}

/// Builds a program of several files in every mode, given as its
/// directory and as its files one by one: what it prints must be what the
/// real toolchain printed for it (tests/uc/<dir>.out).
fn program_runs_in_every_mode_as_the_real_one(dir: &str, files: &[&str], lang: &str) {
    let want = std::fs::read(suite(&format!("{dir}.out"))).unwrap();
    let r = root(&format!("program-{dir}"));
    let named: Vec<String> = files.iter().map(|f| suite(dir).join(f).to_str().unwrap().to_string()).collect();
    for (round, given) in [vec![suite(dir).to_str().unwrap().to_string()], named].into_iter().enumerate() {
        for mode in [vec!["--plain"], vec![], vec!["--release"], vec!["--release", "--no-evaluate"]] {
            let exe = r.join("a.exe");
            let o = Command::new(env!("CARGO_BIN_EXE_strike"))
                .arg("compile")
                .args(&given)
                .args(["-o", exe.to_str().unwrap()])
                .args(&mode)
                .env("STRIKE_HOME", r.join("home"))
                .output()
                .unwrap();
            assert!(o.status.success(), "{given:?} {mode:?}: {}", text(&o));
            assert!(text(&o).contains(&format!("read as {lang};")), "{}", text(&o));
            // the same program given the other way is the same IR: every
            // function is struck from its brand
            if round == 1 && mode.is_empty() {
                assert!(text(&o).contains(" struck, 0 compiled"), "{}", text(&o));
            }
            let got = Command::new(&exe).output().unwrap();
            assert_eq!(String::from_utf8_lossy(&got.stdout), String::from_utf8_lossy(&want), "{given:?} {mode:?}");
            assert!(got.status.success(), "{given:?} {mode:?}");
        }
    }
}

/// C: three files linked by a header's functions and extern globals; the
/// static names of two files are the same, and each file has its own.
#[test]
fn several_c_files_are_one_program_as_clang_links_them() {
    program_runs_in_every_mode_as_the_real_one("multi_c", &["main.c", "calc.c", "counter.c"], "C");
}

/// Go: package main in two files, and two packages of the module, one
/// importing the other; their variables are given before main's.
#[test]
fn go_packages_of_a_module_are_what_go_makes_of_them() {
    program_runs_in_every_mode_as_the_real_one("multi_go", &["main.go", "helper.go"], "Go");
}

/// Python: modules next to the main one, each run once where it is first
/// imported; `from m import x` copies x as it is then.
#[test]
fn python_modules_run_as_python_imports_them() {
    program_runs_in_every_mode_as_the_real_one("multi_py", &["__main__.py"], "Python");
}

/// Macros, conditionals and a header, compiled and run as clang's.
#[test]
fn the_preprocessed_program_prints_what_clangs_prints() {
    runs_in_every_mode_as_the_real_one("preprocess.c");
}

/// A program of several files that Strike cannot make what the language
/// makes of it is refused, at the file and line it stops on.
#[test]
fn what_several_files_cannot_do_yet_is_refused_not_guessed() {
    let r = root("refused-programs");
    let go_mod = ("go.mod", "module example.com/m\n\ngo 1.22\n");
    let cases: Vec<(&str, Vec<(&str, &str)>, &str)> = vec![
        (
            "unexported",
            vec![go_mod, ("main.go", "package main\nimport \"example.com/m/calc\"\nfunc main() {\n\tcalc.add(1, 2)\n}\n"), ("calc/calc.go", "package calc\nfunc add(a, b int) int {\n\treturn a + b\n}\n")],
            "main.go:4: name add not exported by package calc",
        ),
        (
            "unused",
            vec![go_mod, ("main.go", "package main\nimport \"example.com/m/calc\"\nfunc main() {\n}\n"), ("calc/calc.go", "package calc\nfunc Add(a, b int) int {\n\treturn a + b\n}\n")],
            "\"example.com/m/calc\" imported and not used",
        ),
        (
            "cycle",
            vec![
                go_mod,
                ("main.go", "package main\nimport \"example.com/m/a\"\nfunc main() {\n\ta.F()\n}\n"),
                ("a/a.go", "package a\nimport \"example.com/m/b\"\nfunc F() {\n\tb.G()\n}\n"),
                ("b/b.go", "package b\nimport \"example.com/m/a\"\nfunc G() {\n\ta.F()\n}\n"),
            ],
            "import cycle not allowed",
        ),
        (
            "require",
            vec![("go.mod", "module example.com/m\n\ngo 1.22\n\nrequire example.com/other v1.0.0\n"), ("main.go", "package main\nfunc main() {\n}\n")],
            "go.mod:5: require in go.mod is not in Strike yet",
        ),
        (
            "twice",
            vec![("a.c", "int f(void) { return 1; }\nint main() { return f(); }\n"), ("b.c", "int f(void) { return 2; }\n")],
            "b.c:1: f is defined in",
        ),
        ("static", vec![("a.c", "int helper(void);\nint main() { return helper(); }\n"), ("b.c", "static int helper(void) { return 2; }\n")], "there is no function helper"),
        ("header_line", vec![("main.c", "#include \"bad.h\"\nint main() { return 0; }\n"), ("bad.h", "/* a header */\n\nint x = ;\n")], "bad.h:3:"),
        (
            "circular",
            vec![("__main__.py", "import a\nprint(a.x)\n"), ("a.py", "import b\nx = 1\n"), ("b.py", "import a\ny = 2\n")],
            "a circular import is not in Strike yet",
        ),
        ("noname", vec![("__main__.py", "from m import nothing\n"), ("m.py", "x = 1\n")], "__main__.py:1: cannot import name nothing from m"),
        ("star", vec![("__main__.py", "from m import *\n"), ("m.py", "x = 1\n")], "from m import * is not in Strike yet"),
        ("module_line", vec![("__main__.py", "import m\n"), ("m.py", "x = 1\nprint(x ** 2)\n")], "m.py:2:"),
        ("module_value", vec![("__main__.py", "import m\nprint(m)\n"), ("m.py", "x = 1\n")], "m is a module"),
        // structs and classes of other packages and modules, and one struct
        // name given two shapes by two C files
        (
            "package_type",
            vec![go_mod, ("main.go", "package main\nimport \"example.com/m/calc\"\nfunc main() {\n\tcalc.F()\n}\n"), ("calc/calc.go", "package calc\ntype T struct {\n\tA int\n}\nfunc F() {\n}\n")],
            "calc/calc.go:2: a type of a package other than main is not in Strike yet",
        ),
        ("module_class", vec![("__main__.py", "import m\nprint(m.x)\n"), ("m.py", "class C:\n    pass\n\n\nx = 1\n")], "m.py:1: a class of a module other than the main one is not in Strike yet"),
        ("two_shapes", vec![("a.c", "struct D { int z; };\nint main() { return 0; }\n"), ("b.c", "struct D { long long z; };\n")], "struct D is defined differently in two files"),
    ];
    for (name, files, says) in cases {
        let d = r.join(name);
        for (f, src) in files {
            let p = d.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, src).unwrap();
        }
        let o = compile(&r, &d, &r.join("a.exe"));
        assert!(!o.status.success(), "{name} compiled");
        assert!(text(&o).replace('\\', "/").contains(says), "{name}: {}", text(&o));
    }
    // Python runs one file: the others are imported, not given
    let d = r.join("noname");
    let o = Command::new(env!("CARGO_BIN_EXE_strike"))
        .args(["compile", d.join("__main__.py").to_str().unwrap(), d.join("m.py").to_str().unwrap(), "-o", r.join("a.exe").to_str().unwrap()])
        .env("STRIKE_HOME", r.join("home"))
        .output()
        .unwrap();
    assert!(!o.status.success() && text(&o).contains("Python runs one file"), "{}", text(&o));
}
