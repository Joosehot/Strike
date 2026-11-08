# the same suite as suite.c and suite.go

def fib(n):
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)

def gcd(a, b):
    while b != 0:
        t = a % b
        a = b
        b = t
    return a

def is_prime(n):
    if n < 2:
        return False
    d = 2
    while d * d <= n:
        if n % d == 0:
            return False
        d += 1
    return True

def collatz(n):
    steps = 0
    while n != 1:
        if n % 2 == 0:
            n = n // 2
        else:
            n = 3 * n + 1
        steps += 1
    return steps

def sum6(a, b, c, d, e, f):
    return a + 2 * b + 3 * c + 4 * d + 5 * e + 6 * f

print("fib", fib(25))
print("gcd", gcd(1071, 462))
count = 0
for i in range(10000):
    if is_prime(i):
        count += 1
print("primes", count)
print("collatz", collatz(27))
print("sum6", sum6(1, 2, 3, 4, 5, 6))
print("div", -7 // 2, -7 % 2, 7 // -2, 7 % -2)
print("lt", 3 < 4, 4 < 3)
print("min", -9223372036854775807 - 1)
pairs = 0
for i in range(10):
    if i == 7:
        break
    for j in range(10):
        if j % 3 == 0:
            continue
        pairs += i * j
print("pairs", pairs)
print("or", 0 or 5, 3 and 4, not 0)
for k in range(10, 0, -3):
    print("down", k)
print("done")
