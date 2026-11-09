# how many numbers, then the numbers, one a line: their sum

n = int(input())
s = 0
for i in range(n):
    s += int(input())
print(s)