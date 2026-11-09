# Strings as Python makes them: literals, + and *, indexes and slices,
# comparisons and `in`, conversions, the methods, strings in lists.

def greet(name):
    return "hello, " + name + "!"

def count(s, c):
    n = 0
    for x in s:
        if x == c:
            n += 1
    return n

def reverse(s):
    out = ""
    i = len(s) - 1
    while i >= 0:
        out += s[i]
        i -= 1
    return out

def caesar(s, k):
    out = ""
    for c in s:
        if "a" <= c and c <= "z":
            out += chr((ord(c) - ord("a") + k) % 26 + ord("a"))
        else:
            out += c
    return out

def is_palindrome(s):
    t = "".join(s.lower().split())
    return t == reverse(t)

def digits(n):
    return [int(d) for d in str(n)]

s = "strings"
t = 'and more'
print(s, t)
print(greet("world"), len(greet("")))
print(s[0], s[-1], s[2:5], s[:3], s[4:], s[-3:], s[:-2], s[10:], s[5:2], s[-100:2])
print(s * 2, 3 * "ab", "x" * 0, "y" * -1, len("abc" * 1000))
print(s == "strings", s != t, s < t, "abc" < "abd", "ab" < "abc", "b" > "abc", "" < "a", "Z" < "a")
print("rin" in s, "z" in s, "" in s, s in s, "gss" in s)
print(str(42), str(-7), str(0), str(True), str(False), str(-9223372036854775807 - 1), str(s))
print(int("12"), int("  -3_000 "), int("+5"), int("\t7\n"), int("0_0"), int("-0"), int("007"))
print(ord("A"), ord(" "), chr(66), chr(ord("a") + 1), chr(0) == "\0", len(chr(10)))
print(count("banana", "a"), count("", "a"), reverse("drawer"), reverse(""))
print(caesar("hello, world", 3), caesar(caesar("abc xyz", 5), 21))
print(is_palindrome("Never odd or even"), is_palindrome("strings"))
print(digits(9075), digits(0))
print(s.upper(), "MiXeD 123 _".lower(), "  pad \n\t".strip() + "|", (chr(28) + " x " + chr(31)).strip())
print("  one two   three ".split(), "".split(), "solo".split(), ("a\tb\nc" + chr(11) + "d" + chr(12) + "e\rf").split())
print("-".join(["a", "b", "c"]), "".join([]), ", ".join("x y z".split()), "+".join(["only"]))
print(s.startswith("str"), s.startswith("ings"), s.startswith(""), "".startswith("a"))
print(s.find("r"), s.find("ing"), s.find("q"), s.find(""), "aaa".find("aa"))
names = ["ann", "bo"]
names.append("it's")
names.append('say "hi"')
names.append("both ' and \"")
names.append("tab\tnew\nline\\")
names.append(chr(1) + chr(127))
print(names, len(names))
print(names.pop(), names[-1], [[s], []], [c for c in "abc"])
grid = [["x", "o"], ["o", "x"]]
for row in grid:
    print("".join(row))
acc = ""
i = 0
while i < 5:
    acc += str(i * i)
    i += 1
print(acc, len(acc))
word = "abc"
word *= 2
print(word)
if "":
    print("the empty string is true")
if s:
    print("a string with characters is true")
if not "":
    print("not the empty string")
line = ""
for c in "hello":
    line = c + line
print(line)
n = 0
for w in "a bb ccc dddd".split():
    n += len(w)
print(n, len("a bb ccc dddd".split()))
pieces = []
for k in range(3):
    pieces.append(str(k) * (k + 1))
print(pieces, "/".join(pieces))
print("a" + str(1 + 2) + "b", str(1000000000000000000), str(len(s)) + s)
