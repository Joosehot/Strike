//! Loop vectorization (src/uc/vectorize.rs): random C programs whose loops
//! have the shapes it vectorizes and the shapes it must not (a dependency
//! from one time round to the next, pointer parameters that alias, float
//! reductions), with lengths from 0 to 37 so the scalar loop that finishes
//! the leftover elements runs every way, and offsets. Every mode prints
//! what the plain code prints; and the loops that can be vectorized are.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root(name: &str) -> PathBuf {
    let r = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("vector-{name}"));
    let _ = std::fs::remove_dir_all(&r);
    std::fs::create_dir_all(&r).unwrap();
    r
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr)
}

fn compile(home: &Path, src: &Path, exe: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_strike"))
        .args(["compile", src.to_str().unwrap(), "-o", exe.to_str().unwrap()])
        .args(extra)
        .env("STRIKE_HOME", home.join("home"))
        .output()
        .unwrap()
}

/// How many loops the release build vectorized, from its report.
fn vectorized(o: &Output) -> usize {
    let t = text(o);
    let Some(at) = t.find(" loops vectorized") else { return 0 };
    t[..at].rsplit(' ').next().unwrap().parse().unwrap()
}

/// `place op= e`, as C forms Strike reads (`a = a ^ (e)`).
fn assign(place: &str, op: &str, e: &str) -> String {
    match op {
        "=" | "+=" | "-=" => format!("{place} {op} {e};"),
        _ => format!("{place} = {place} {op} ({e});"),
    }
}

struct Gen(u64);

const N: usize = 48;

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
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len() as u64) as usize]
    }

    /// An index: the counter plus a small offset, or plus the parameter o.
    fn index(&mut self) -> String {
        match self.below(5) {
            0 => "i + o".to_string(),
            1 | 2 => format!("i + {}", self.below(4)),
            _ => "i".to_string(),
        }
    }

    /// An integer expression of elements of `arrays`, i, x and constants:
    /// the operations vectorize takes and some it does not.
    fn iexpr(&mut self, arrays: &[&str], depth: u32, wide: bool) -> String {
        if depth == 0 || self.below(4) == 0 {
            return match self.below(7) {
                0..=2 => {
                    let a = self.pick(arrays);
                    format!("{a}[{}]", self.index())
                }
                3 => "i".to_string(),
                4 => "x".to_string(),
                _ => format!("{}", self.below(200) as i64 - 100),
            };
        }
        let d = depth - 1;
        let bits = if wide { 64 } else { 32 };
        match self.below(10) {
            0..=4 => {
                let op = self.pick(&["+", "-", "&", "|", "^", "+", "-", "*"]);
                format!("({} {op} {})", self.iexpr(arrays, d, wide), self.iexpr(arrays, d, wide))
            }
            5 | 6 => format!("({} {} {})", self.iexpr(arrays, d, wide), self.pick(&["<<", ">>"]), self.below(bits)),
            7 => format!("{}({})", self.pick(&["-", "~"]), self.iexpr(arrays, d, wide)),
            8 => format!("({} * {})", self.iexpr(arrays, d, wide), self.pick(&["3", "8", "-5", "9"])),
            _ => format!("({} < {})", self.iexpr(arrays, d, wide), self.iexpr(arrays, d, wide)),
        }
    }

    /// A float expression of elements, y and constants (NaN, the
    /// infinities and both zeros are among the elements).
    fn fexpr(&mut self, depth: u32) -> String {
        if depth == 0 || self.below(4) == 0 {
            return match self.below(6) {
                0..=2 => {
                    let a = self.pick(&["da", "db", "dc"]);
                    format!("{a}[{}]", self.index())
                }
                3 => "y".to_string(),
                _ => self.pick(&["0.5", "-2.25", "3.0", "-0.0", "1e300", "0.1"]).to_string(),
            };
        }
        let d = depth - 1;
        match self.below(8) {
            0..=4 => format!("({} {} {})", self.fexpr(d), self.pick(&["+", "-", "*", "/"]), self.fexpr(d)),
            5 => format!("-({})", self.fexpr(d)),
            6 => format!("{}({})", self.pick(&["sqrt", "fabs"]), self.fexpr(d)),
            _ => format!("sqrt(fabs({}))", self.fexpr(d)),
        }
    }

    /// A loop's head: the counter's type, its start and its bound (a
    /// constant, the parameter n, or a local).
    fn head(&mut self, int_counter: bool) -> (String, String) {
        let t = if int_counter { "int" } else { "long long" };
        let start = match self.below(4) {
            0 => format!("{}", self.below(4)),
            1 => "s".to_string(),
            _ => "0".to_string(),
        };
        let (pre, bound) = match self.below(3) {
            0 => (String::new(), format!("{}", self.below(38))),
            1 => (format!("    {t} m = n;\n"), "m".to_string()),
            _ => (String::new(), "n".to_string()),
        };
        (pre, format!("for ({t} i = {start}; i < {bound}; i++)"))
    }

    /// One function with one loop of a random shape.
    fn case(&mut self, k: usize) -> String {
        let sig = format!("void t{k}(long long n, long long o, long long s, long long x, double y)");
        let mut body = String::new();
        match self.below(10) {
            // element by element, long long
            0 | 1 => {
                let c = self.below(2) == 0;
                let (pre, head) = self.head(c);
                let dst = self.pick(&["ga", "gb", "gc"]);
                // a source of other arrays, or (now and then) the one stored
                let srcs: Vec<&str> = if self.below(3) == 0 { vec!["ga", "gb", "gc"] } else { ["ga", "gb", "gc"].into_iter().filter(|a| *a != dst).collect() };
                let e = self.iexpr(&srcs, 3, true);
                let idx = self.index();
                let op = self.pick(&["=", "=", "+=", "^", "-="]);
                let line = assign(&format!("{dst}[{idx}]"), op, &e);
                body += &format!("{pre}    {head} {{\n        {line}\n    }}\n");
            }
            // element by element, C int
            2 | 3 => {
                let c = self.below(3) > 0;
                let (pre, head) = self.head(c);
                let dst = self.pick(&["ia", "ib", "ic"]);
                let srcs: Vec<&str> = if self.below(3) == 0 { vec!["ia", "ib", "ic"] } else { ["ia", "ib", "ic"].into_iter().filter(|a| *a != dst).collect() };
                let e = self.iexpr(&srcs, 3, false);
                let idx = self.index();
                let op = self.pick(&["=", "=", "+=", "|", "&"]);
                // a local made in the body, now and then
                if self.below(2) == 0 {
                    let f = self.iexpr(&srcs, 2, false);
                    let line = assign(&format!("{dst}[{idx}]"), op, &format!("{e} + t"));
                    body += &format!("{pre}    {head} {{\n        int t = {f};\n        {line}\n    }}\n");
                } else {
                    let line = assign(&format!("{dst}[{idx}]"), op, &e);
                    body += &format!("{pre}    {head} {{\n        {line}\n    }}\n");
                }
            }
            // a reduction, long long or C int
            4 | 5 => {
                let wide = self.below(2) == 0;
                let c = !wide && self.below(2) == 0;
                let (pre, head) = self.head(c);
                let arrays: &[&str] = if wide { &["ga", "gb", "gc"] } else { &["ia", "ib", "ic"] };
                let e = self.iexpr(arrays, 3, wide);
                let op = self.pick(&["+=", "+=", "-=", "^", "|", "&"]);
                let line = assign("r", op, &e);
                let (t, f) = if wide { ("long long", "%lld") } else { ("int", "%d") };
                // a second statement that stores, now and then
                let extra = if self.below(3) == 0 { format!("        {}[i] = {};\n", arrays[2], self.iexpr(&arrays[..2], 2, wide)) } else { String::new() };
                body += &format!("    {t} r = x;\n{pre}    {head} {{\n{extra}        {line}\n    }}\n    printf(\"{f}\\n\", r);\n");
            }
            // doubles element by element
            6 | 7 => {
                let c = self.below(2) == 0;
                let (pre, head) = self.head(c);
                let dst = self.pick(&["da", "db", "dc"]);
                let e = self.fexpr(3);
                let idx = self.index();
                let line = match self.below(3) {
                    0 => format!("{dst}[{idx}] += y * {};", self.fexpr(1)),
                    _ => format!("{dst}[{idx}] = {e};"),
                };
                body += &format!("{pre}    {head} {{\n        {line}\n    }}\n");
            }
            // a float reduction: never vectorized (it would reorder the sum)
            8 => {
                let (pre, head) = self.head(false);
                let e = self.fexpr(2);
                body += &format!("    double r = y;\n{pre}    {head} {{\n        r += {e};\n    }}\n    printf(\"%.17g\\n\", r);\n");
            }
            // a row of a 2D array, and a local array of the frame
            _ => {
                let (pre, head) = self.head(false);
                let e = self.iexpr(&["gb", "la"], 2, true);
                let r = self.below(4);
                body += &format!(
                    "    long long la[{N}];\n    for (long long j = 0; j < {N}; j++) {{\n        la[j] = ga[j] * 3 + j;\n    }}\n{pre}    {head} {{\n        mm[{r}][i] = {e};\n        la[i] = la[i] + mm[{r}][i + 1];\n    }}\n    long long h = 0;\n    for (long long j = 0; j < {N}; j++) {{\n        h = h * 31 + la[j] + mm[{r}][j];\n    }}\n    printf(\"%lld\\n\", h);\n"
                );
            }
        }
        format!("{sig} {{\n{body}}}\n")
    }

    /// Pointer parameters: the same arrays may come in as both, so a
    /// store through one may be what the other reads next time round.
    fn pointers(&mut self) -> String {
        let a = self.below(3) + 1;
        let e = format!("q[i + {a}] {} p[i]", self.pick(&["+", "^", "-"]));
        format!("void pp(long long *p, long long *q, long long n) {{\n    for (long long i = 0; i < n; i++) {{\n        p[i] = {e};\n    }}\n}}\nint ps(int *p, int *q, int n) {{\n    int r = 0;\n    for (int i = 0; i < n; i++) {{\n        r += p[i] * q[i] + (p[i] >> 1);\n    }}\n    return r;\n}}\n")
    }

    fn program(&mut self) -> String {
        let mut out = String::from("#include <stdio.h>\n#include <math.h>\n");
        for (t, names) in [("long long", ["ga", "gb", "gc"]), ("int", ["ia", "ib", "ic"]), ("double", ["da", "db", "dc"])] {
            for a in names {
                out += &format!("{t} {a}[{N}];\n");
            }
        }
        out += &format!("long long mm[4][{N}];\n");
        out += &format!("long long seed = {};\n", self.next() as i64 >> 1);
        out += "long long rnd() {\n    seed = seed * 6364136223846793005 + 1442695040888963407;\n    return seed >> 13;\n}\n";
        out += &format!(
            "void fill() {{\n    for (int i = 0; i < {N}; i++) {{\n        ga[i] = rnd();\n        gb[i] = rnd() % 1000;\n        gc[i] = rnd() >> 20;\n        ia[i] = rnd();\n        ib[i] = rnd() % 100;\n        ic[i] = rnd() >> 30;\n        da[i] = (rnd() % 20001 - 10000) / 64.0;\n        db[i] = (rnd() % 2001) / 7.0;\n        dc[i] = rnd() % 5;\n        for (int r = 0; r < 4; r++) {{\n            mm[r][i] = rnd();\n        }}\n    }}\n    for (int k = 4; k < 40; k += 3) {{\n        da[k] = 0.0 / 0.0;\n        db[k] = -(0.0 / 0.0);\n        dc[k] = -(0.0 / 0.0);\n        da[k + 1] = -(0.0 / 0.0);\n        db[k + 1] = 0.0 / 0.0;\n    }}\n    da[11] = 1.0 / 0.0;\n    db[12] = -1.0 / 0.0;\n    dc[7] = -0.0;\n    db[20] = 0.0;\n}}\n"
        );
        out += &format!(
            "void show() {{\n    long long h = 0;\n    for (int i = 0; i < {N}; i++) {{\n        h = h * 31 + ga[i];\n        h = h * 31 + gb[i];\n        h = h * 31 + gc[i];\n        h = h * 31 + ia[i];\n        h = h * 31 + ib[i];\n        h = h * 31 + ic[i];\n    }}\n    printf(\"%lld\\n\", h);\n    for (int i = 0; i < {N}; i++) {{\n        printf(\"%.17g %.17g %.17g\\n\", da[i], db[i], dc[i]);\n    }}\n}}\n"
        );
        out += &self.pointers();
        let cases = 3 + self.below(3) as usize;
        for k in 0..cases {
            out += &self.case(k);
        }
        out += "int main() {\n    fill();\n";
        for k in 0..cases {
            for _ in 0..2 {
                let n = self.below(38);
                let o = self.below(5);
                let s = self.below(4);
                let x = self.next() as i64 >> self.below(60);
                let y = self.pick(&["1.5", "-0.25", "3.0", "-0.0", "1e300", "0.0 / 0.0", "-(0.0 / 0.0)"]);
                out += &format!("    t{k}({n}, {o}, {s}, {x}, {y});\n    show();\n");
            }
        }
        for _ in 0..2 {
            let n = self.below(38);
            let (p, q) = [("ga", "gb"), ("ga", "ga"), ("gc", "ga"), ("gb", "gb")][self.below(4) as usize];
            out += &format!("    pp({p}, {q}, {n});\n    show();\n");
            let (p, q) = [("ia", "ib"), ("ia", "ia"), ("ic", "ib")][self.below(3) as usize];
            out += &format!("    printf(\"%d\\n\", ps({p}, {q}, {}));\n", self.below(38));
        }
        out + "    return 0;\n}\n"
    }
}

/// 60 random programs of loops in every mode: each prints what the plain
/// code prints, and the release build vectorizes many of their loops.
#[test]
fn vectorized_loops_print_what_the_plain_code_prints() {
    let r = root("random");
    let mut g = Gen(0x5DEE_CE66_D1A4_F00D);
    let mut total = 0;
    for n in 0..60 {
        let src = r.join(format!("v{n}.c"));
        std::fs::write(&src, g.program()).unwrap();
        let mut outs = Vec::new();
        for mode in [vec!["--plain"], vec![], vec!["--release"], vec!["--release", "--no-evaluate"]] {
            let exe = r.join(format!("v{n}.exe"));
            let o = compile(&r, &src, &exe, &mode);
            assert!(o.status.success(), "v{n}.c: {}", text(&o));
            if mode.len() == 2 {
                total += vectorized(&o);
            }
            let run = Command::new(&exe).output().unwrap();
            outs.push((run.status.code(), run.stdout));
        }
        for (i, o) in outs.iter().enumerate().skip(1) {
            assert!(&outs[0] == o, "v{n}.c: mode {i} and the plain code differ");
        }
    }
    eprintln!("{total} loops vectorized");
    assert!(total >= 100, "only {total} loops vectorized");
}
