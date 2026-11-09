# The C poly.c in Python: its numbers do not wrap, so every operation is a
# checked one, and the simplifier leaves them where they are (the checks
# must stop exactly where they would).
s = 0
i = 0
while i < 20000000:
    x = i ^ (i >> 3)
    s = (s + 3 * x * x + 5 * x + 7) % 1000000007
    i = i + 1
print(s)
