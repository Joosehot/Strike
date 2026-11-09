# a module runs where it is first imported, and only then
import helpers
from counter import bump, total
import shapes as sh

print(helpers.double(21))
print(helpers.LIMIT)
bump()
bump()
# total was copied when it was imported
print(total)
import counter
print(counter.total)
counter.total = 10
bump()
print(counter.total)
print(sh.area(3, 4))
print(sh.calls, sh.area(1, 1), sh.calls)
if __name__ == "__main__":
    print("run as the main module")
