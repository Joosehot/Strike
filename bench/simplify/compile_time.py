"""Strike's own compile time (what it reports, --release --no-evaluate
--no-brand) with the simplifier and before it (STRIKE_BASE), interleaved:
the minimum and the median of ROUNDS rounds, each the sum over the files.

    python bench/simplify/compile_time.py ROUNDS FILE...
"""
import os, re, statistics, subprocess, sys

here = os.path.dirname(os.path.abspath(__file__))
new = os.path.join(os.path.dirname(os.path.dirname(here)), "target", "release", "strike.exe")
base = os.environ["STRIKE_BASE"]
rounds = int(sys.argv[1])
files = sys.argv[2:]
out = os.path.join(here, "out", "ct.exe")
os.makedirs(os.path.dirname(out), exist_ok=True)
times = {"strike": [], "before": []}
for _ in range(rounds):
    for tag, exe in (("strike", new), ("before", base)):
        total = 0.0
        for f in files:
            r = subprocess.run([exe, "compile", f, "-o", out, "--release", "--no-evaluate", "--no-brand"], capture_output=True, text=True, check=True)
            total += float(re.search(r"; ([0-9.]+) s \(read", r.stderr + r.stdout).group(1))
        times[tag].append(total)
print("  ".join(f"{k}: min {min(v):.3f} s med {statistics.median(v):.3f} s" for k, v in times.items()))
