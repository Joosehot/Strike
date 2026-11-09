# Lines read as strings: input() with and without a prompt, int() of a
# string, words split and joined, a line ended by \r\n, the last line
# without its newline.
name = input()
print("hello, " + name + "!")
n = int(input("how many? "))
words = input().split()
print(len(words), words)
total = 0
for w in words:
    total += int(w)
print(total * n)
blank = input()
print(len(blank), blank == "", "[" + blank + "]")
crlf = input()
print(crlf.upper(), len(crlf))
shout = []
line = input()
while line != "end":
    shout.append(line.strip().upper())
    line = input()
print(" ".join(shout), shout)
last = input()
print(last[-4:], len(last))
