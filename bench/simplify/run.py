"""Interleaved run times of the simplifier's benchmarks: clang -O2 (or go
build), Strike before the simplifier (STRIKE_BASE: a build of master) and
Strike with it, all built first and checked to print the same.

    python bench/simplify/run.py [ROUNDS] [NAMES...]
"""
import os, shutil, statistics, subprocess, sys, time

here = os.path.dirname(os.path.abspath(__file__))
root = os.path.dirname(os.path.dirname(here))
new = os.path.join(root, "target", "release", "strike.exe")
base = os.environ.get("STRIKE_BASE")
clang = r"C:\Program Files\LLVM\bin\clang.exe"
out = os.path.join(here, "out")
os.makedirs(out, exist_ok=True)
rounds = int(sys.argv[1]) if len(sys.argv) > 1 else 11
names = sys.argv[2:] or sorted(f for f in os.listdir(here) if f.endswith((".c", ".go", ".py")) and f not in ("run.py", "compile_time.py"))


def build(src):
    stem = os.path.join(out, src.replace(".", "_"))
    exes = {}
    home = os.path.join(out, "home")
    for tag, strike in (("strike", new), ("before", base)):
        if strike:
            exe = f"{stem}-{tag}.exe"
            subprocess.run([strike, "compile", os.path.join(here, src), "-o", exe, "--release", "--no-evaluate", "--no-brand"], check=True, capture_output=True, env={**os.environ, "STRIKE_HOME": home})
            exes[tag] = [exe]
    exe = f"{stem}-native.exe"
    if src.endswith(".c"):
        subprocess.run([clang, "-O2", os.path.join(here, src), "-o", exe], check=True, capture_output=True)
        exes["clang -O2"] = [exe]
    elif src.endswith(".go"):
        subprocess.run([shutil.which("go") or r"C:\Program Files\Go\bin\go.exe", "build", "-o", exe, os.path.join(here, src)], check=True, capture_output=True)
        exes["go"] = [exe]
    return exes


for src in names:
    exes = build(src)
    outs = {k: subprocess.run(e, capture_output=True).stdout.replace(b"\r", b"") for k, e in exes.items()}
    assert len(set(outs.values())) == 1, (src, outs)
    times = {k: [] for k in exes}
    for _ in range(rounds):
        for k, e in exes.items():
            t = time.perf_counter()
            subprocess.run(e, capture_output=True)
            times[k].append(time.perf_counter() - t)
    row = "  ".join(f"{k}: min {min(v):.4f} med {statistics.median(v):.4f}" for k, v in times.items())
    print(f"{src:12} {row}", flush=True)
