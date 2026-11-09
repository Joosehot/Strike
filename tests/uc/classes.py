# Python's classes: objects on the heap, shared by every name that holds
# them; attributes learned as a list's elements are; class attributes,
# bases, overriding and super(); printed by __str__ and __repr__.

limit = 10
scale = 3


class Point:
    count = 0

    def __init__(self, x, y):
        self.x = x
        self.y = y
        Point.count += 1

    def move(self, dx, dy):
        self.x += dx
        self.y = self.y + dy
        return self

    def dist2(self, other):
        return (self.x - other.x) * (self.x - other.x) + (self.y - other.y) * (self.y - other.y)

    def __repr__(self):
        return "Point(" + str(self.x) + ", " + str(self.y) + ")"


class Bag:
    def __init__(self):
        self.items = []
        self.name = "bag"

    def add(self, x):
        self.items.append(x)
        return len(self.items) < limit

    def __str__(self):
        return self.name + ":" + str(len(self.items))


class Animal:
    kind = "animal"
    legs = 4

    def __init__(self, name, age):
        self.name = name
        self.age = age
        self.tricks = []

    def speak(self):
        return "..."

    def describe(self):
        return self.name + " (" + str(self.age) + ") says " + self.speak()

    def learn(self, t):
        self.tricks.append(t)
        return len(self.tricks)

    def __str__(self):
        return self.kind + ":" + self.name


class Dog(Animal):
    kind = "dog"

    def __init__(self, name, age, good):
        super().__init__(name, age)
        self.good = good

    def speak(self):
        return "woof"


class Puppy(Dog):
    def speak(self):
        return "yip " + super().speak()


class Bird(Animal):
    legs = 2

    def __init__(self, name):
        super().__init__(name, 1)
        self.wing = 0.5


class Counter:
    total = 0

    def __init__(self):
        self.n = 0

    def tick(self):
        self.n += 1
        Counter.total += 1
        return self

    def __repr__(self):
        return "C" + str(self.n)


class Pair:
    def __init__(self, a, b):
        self.first = a
        self.second = b

    def swap(self):
        return Pair(self.second, self.first)

    def __repr__(self):
        return "(" + str(self.first) + ", " + str(self.second) + ")"


class Flags:
    def __init__(self, a, b):
        self.on = a
        self.off = b
        self.ratio = 0.5

    def __repr__(self):
        return str(self.on) + "/" + str(self.off)


class Line:
    def __init__(self, a, b):
        self.a = a
        self.b = b

    def length2(self):
        return self.a.dist2(self.b)


class Consts:
    a = 1
    c = a + scale
    scale = a * 100
    d = [a, scale]


def make(n):
    c = Counter()
    for i in range(n):
        c.tick()
    return c


def shift(p, d):
    p.x += d


p = Point(1, 2)
q = p
q.move(3, 4)
print(p.x, p.y, q)
r = Point(0, 0)
print(p.dist2(r), Point.count, p.count)
pts = [Point(i, i * i) for i in range(3)]
print(pts)
for t in pts:
    t.move(1, 1)
shift(pts[0], 100)
print(pts, Point.count)
b = Bag()
print(b.add(5), b.add(7))
print(b, b.items, str(b))

a = Animal("generic", 3)
d = Dog("rex", 5, True)
pup = Puppy("bit", 1, False)
bird = Bird("tweety")
print(a.describe())
print(d.describe(), d.good)
print(pup.describe(), pup.good, pup.legs, pup.kind)
print(bird.describe(), bird.legs, bird.kind, bird.wing)
print(a, d, pup, bird)
print(str(d), d.learn("sit"), d.learn("roll"), d.tricks, a.tricks)
d.kind = "mine"
print(d.kind, Dog.kind, pup.kind, Animal.kind)
Animal.legs = 3
print(a.legs, d.legs, bird.legs)

cs = [Counter() for i in range(3)]
cs[0].tick().tick()
cs[2].tick()
alias = cs[2]
alias.tick()
print(cs, Counter.total, alias.n)
grid = [[Counter(), Counter()], [Counter()]]
grid[1][0].tick()
print(grid)
m = make(4)
print(m, m.n, Counter.total)

print(Pair(1, 2).swap(), [Pair(3, 4), Pair(-1, 0)])
flags = Flags(True, False)
flags.ratio *= 3
print(flags, flags.on and flags.off, flags.ratio, [flags])

ln = Line(Point(0, 0), Point(3, 4))
print(ln.length2(), ln.a, ln.b)
ln.b.move(1, 1)
ln.a = ln.b
print(ln.length2(), ln.a)
print(Consts.a, Consts.c, Consts.scale, Consts.d, scale)

p.label = "named"
print(p.label, p)
