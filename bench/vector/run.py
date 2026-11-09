"""Interleaved run times for vectorization: Strike before it (STRIKE_BASE: a
build of master), Strike with it, clang -O2 and clang -O2 without its
vectorizers, all built first and checked to print the same.

    python bench/vector/run.py [ROUNDS] [FILES...]

FILES are C files (default: every .c here and in bench/simplify).
"""
import os, statistics, subprocess, sys, time

here = os.path.dirname(os.path.abspath(__file__))
root = os.path.dirname(os.path.dirname(here))
new = os.path.join(root, "target", "release", "strike.exe")
base = os.environ.get("STRIKE_BASE")
clang = r"C:\Program Files\LLVM\bin\clang.exe"
out = os.path.join(here, "out")
os.makedirs(out, exist_ok=True)
rounds = int(sys.argv[1]) if len(sys.argv) > 1 else 11
files = sys.argv[2:] or sorted(
    [os.path.join(here, f) for f in os.listdir(here) if f.endswith(".c")]
    + [os.path.join(root, "bench", "simplify", f) for f in os.listdir(os.path.join(root, "bench", "simplify")) if f.endswith(".c")]
)


def build(src):
    stem = os.path.join(out, os.path.basename(src).replace(".", "_"))
    exes = {}
    home = os.path.join(out, "home")
    for tag, strike in (("before", base), ("strike", new)):
        if strike:
            exe = f"{stem}-{tag}.exe"
            subprocess.run([strike, "compile", src, "-o", exe, "--release", "--no-evaluate", "--no-brand"], check=True, capture_output=True, env={**os.environ, "STRIKE_HOME": home})
            exes[tag] = exe
    for tag, flags in (("clang -O2", []), ("clang scalar", ["-fno-vectorize", "-fno-slp-vectorize"])):
        exe = f"{stem}-{tag.replace(' ', '_')}.exe"
        subprocess.run([clang, "-O2", *flags, src, "-o", exe], check=True, capture_output=True)
        exes[tag] = exe
    return exes


for src in files:
    exes = build(src)
    outs = {k: subprocess.run(e, capture_output=True).stdout.replace(b"\r", b"") for k, e in exes.items()}
    assert len(set(outs.values())) == 1, (src, outs)
    times = {k: [] for k in exes}
    for _ in range(rounds):
        for k, e in exes.items():
            t = time.perf_counter()
            subprocess.run(e, capture_output=True)
            times[k].append(time.perf_counter() - t)
    row = "  ".join(f"{k}: {min(v):.4f}/{statistics.median(v):.4f}" for k, v in times.items())
    print(f"{os.path.basename(src):12} {row}", flush=True)
