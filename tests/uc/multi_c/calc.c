#include "calc.h"

int calls = 0;
long long table[8];

/* static: this file's own; counter.c has a helper and a count of its own */
static int count = 100;

static int helper(int x) {
    return x + 1;
}

long long add(long long a, long long b) {
    calls++;
    return a + b;
}

long long power(long long b, int e) {
    long long r = 1;
    for (int i = 0; i < e; i++) {
        r *= b;
    }
    calls = calls + helper(0) - 1;
    count++;
    return r;
}

int calc_count(void) {
    return count;
}
