#include <stdio.h>
#include "calc.h"

static int count;

static int helper(int x) {
    return x * 10;
}

/* a static variable in a function keeps its value from call to call */
int next(void) {
    static int n = 5;
    n++;
    count += helper(1);
    return n;
}

int counter_count(void) {
    return count;
}

void fill(void) {
    for (int i = 0; i < 8; i++) {
        table[i] = SQUARE(i + 1);
    }
}
