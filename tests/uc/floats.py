# Floats as Python has them: repr, true division, floor division and
# remainder, whole numbers made floats, exact comparisons, conversions.
import math

print(0.1, 0.2, 0.1 + 0.2, 1.0, -0.0, 2.5e-7, 1e16, 1e15, 123456789.0)
print(1e22, 1e-5, 0.0001, 5e-324, 1.7976931348623157e308, 1 / 3, 2 / 3)
print(7 / 2, -7 / 2, 1 / 7, 10 / 5)
print(.5, 5., 1_000.25, 3e0, 1E3)

x = 7.5
print(x // 2, x % 2, -x // 2, -x % 2, x // -2, x % -2, -x // -2, -x % -2)
print(0.0 % -1.0, -0.0 // 1.0, 5 // 0.5, 1e300 * 1e10, -1e300 * 1e10)
print(5.5 % 1, 1e18 % 7.0, -1e-300 % 3.0, 3.0 % 1e-300)

# whole numbers meeting floats
i = 3
f = 0.5
print(i + f, i * f, i - f, f - i, i / f, i // f, i % f, 2 * f)
print(i < f, i > f, i == 3.0, 3.0 == i, i != 3.0, f <= 0.5, f >= 0.6)
big = 9007199254740993
print(big == 9007199254740992.0, big > 9007199254740992.0, 9007199254740992.0 < big)
print(big + 0.0, float(big), big == float(big))
huge = 9223372036854775807
print(huge < 9.3e18, huge > 9.2e18, huge == 9223372036854775808.0, -huge - 1 == -9.223372036854775808e18)
nan = 1e308 * 10 - 1e308 * 10
print(nan == nan, nan != nan, nan < 1, nan > 1, nan == 1, nan != 1)

# conversions
print(int(3.9), int(-3.9), int(2.5e18), float(7), float(True), int(-0.5))
print(float(-5) / 2, int(7 / 2), abs(-2.5), abs(-3), abs(0.0), abs(-0.0))

# truth values
print(not 0.0, not -0.0, not 0.5, 0.0 or 1.5, 2.5 and 0.0, -0.0 or 3.0)
if 0.1:
    print("0.1 is true")

def mean(xs):
    s = 0.0
    for v in xs:
        s += v
    return s / len(xs)

def norm(a, b):
    return math.sqrt(a * a + b * b)

vals = [1.5, 2.25, -0.75, 10.0]
print(mean(vals), norm(3.0, 4.0), norm(1.0, 1e-3), math.sqrt(2), math.sqrt(0.0))
vals.append(0.125)
vals[0] = vals[0] * 2
vals[1] += 1
vals[2] /= 4
print(vals, len(vals), vals[-1])
grid = [[0.0] * 3 for _ in range(2)]
grid[1][2] = 1e-9
print(grid, [v * 2 for v in vals if v > 0])
halves = [k / 2 for k in range(5)]
print(halves)

# floats in a loop: Newton's square root and a running sum
def newton(a):
    g = a
    for _ in range(30):
        g = (g + a / g) / 2
    return g

print(newton(2.0), newton(1e10), newton(0.25))
acc = 0.0
k = 1
while k <= 1000:
    acc += 1.0 / (k * k)
    k += 1
print(acc, acc * 6)
t = 1.0
t /= 3
t *= 3
t -= 1
print(t)

r = 0.0
def bump():
    global r
    r = r + 0.1

for _ in range(10):
    bump()
print(r, r == 1.0, 1.0 - r)
