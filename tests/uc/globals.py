# globals: the module's names, read by functions, changed through global

counter = 10
total = 0

def bump(by):
    global counter
    counter = counter + by

def next_id():
    global counter
    counter += 1
    return counter

def peek():
    return counter * 2

bump(5)
print(counter)
a = next_id()
b = next_id()
print(a, b, peek())
for i in range(7):
    total += i * counter
print(total)