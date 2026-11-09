import helpers

calls = 0


def area(w, h):
    global calls
    calls += 1
    return helpers.double(w * h) // 2


print("shapes loads", helpers.LIMIT)
