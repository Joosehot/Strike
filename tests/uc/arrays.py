squares = []
for i in range(10):
    squares.append(i * i)
print(squares, len(squares), squares[-1], squares[-10])

def total(xs):
    t = 0
    for x in xs:
        t += x
    return t

def fill(xs, n):
    for i in range(n):
        xs.append(i % 3 == 0)

flags = []
fill(flags, 5)
print(flags)
print(total(squares))

grid = [[0] * 3 for _ in range(3)]
grid[1][2] = 5
grid[0][0] += 7
print(grid)
alias = [[0] * 2] * 2
alias[0][1] = 9
print(alias)

evens = [x * 2 for x in range(10) if x % 3 != 0]
print(evens)
doubled = [x + x for x in evens]
print(doubled[1:4], doubled[:2], doubled[3:], doubled[-2:], doubled[5:2])

a = [1, 2, 3]
b = a
b.append(4)
c = a + [5, 6]
a += [7]
print(a, b, c)
print(a.pop(), a, len(a))

stack = []
for k in range(5):
    stack.append(k)
out = []
while stack:
    out.append(stack.pop())
print(out, stack)

log = []
def note(x):
    log.append(x)

for k in range(4):
    note(k * 100)
print(log)

sieve = [True] * 50
sieve[0] = False
sieve[1] = False
for p in range(2, 50):
    if sieve[p]:
        for m in range(p * p, 50, p):
            sieve[m] = False
print(len([p for p in range(50) if sieve[p]]))

nested = [[1, 2], [], [3]]
nested[1].append(8)
print(nested, len(nested[0]))

empty = []
print(empty, len(empty))
