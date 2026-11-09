"""A stress test of the simplifier's compile time: N functions of dense random
polynomials, printed as one C program.

    python bench/simplify/genbig.py 2000 > big.c
"""
import random, sys
random.seed(7)


def ring(d, names=("a", "b", "c", "i")):
    if d == 0 or random.random() < 0.2:
        return random.choice(list(names) + [str(random.randint(-9, 99))])
    k = random.randrange(8)
    l, r = ring(d - 1, names), ring(d - 1, names)
    return ["(%s + %s)", "(%s - %s)", "(%s * %s)", "(%s * %s)", "(%s * 3 + %s * 3)", "(%s / 7 + %s)", "((%s << 2) + %s)", "(%s * %s * 5)"][k] % (l, r)


out = ["#include <stdio.h>"]
n = int(sys.argv[1])
for f in range(n):
    out.append("long long f%d(long long a, long long b, long long c) {\n    long long s = 0;\n    for (long long i = 0; i < 10; i++) {\n        s = s + %s;\n    }\n    return s + %s;\n}" % (f, ring(4), ring(4, ("a", "b", "c"))))
out.append("int main() {\n    long long t = 0;")
for f in range(n):
    out.append("    t = t + f%d(%d, %d, %d);" % (f, f, f + 1, f + 2))
out.append('    printf("%lld\\n", t);')
out.append("    return 0;\n}")
print("\n".join(out))
