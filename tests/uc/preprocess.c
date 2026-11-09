#include <stdio.h>
#include <stdint.h>
#include <limits.h>
#include "preprocess.h"
#include "preprocess.h"

GETTER(one, 1)
GETTER(two, SQUARE(2) - 2)

#if defined(_WIN32) && !defined(__linux__)
#define PLATFORM 1
#elif defined(__linux__)
#define PLATFORM 2
#else
#define PLATFORM 3
#endif

#ifdef NOT_DEFINED
#error this group is skipped
#endif

#define SUM_TO(n) sum_to(n)
#define TWICE(e) ((e) + (e))
#define EMPTY
#define LAST_LINE __LINE__

long long sum_to(long long n) {
    long long s = 0;
    for (long long i = 1; i <= n; i++) {
        s += i;
    }
    return s;
}

int main() {
    int items[MAX_ITEMS] = {1, 2, 3, 4, 5};
    long long total = 0;
    for (int i = 0; i < MAX_ITEMS; i++) {
        total += CUBE(items[i]);
    }
    printf("%lld\n", total);
    printf("%d %d\n", get_one(), get_two());
    printf("%lld\n", SUM_TO(TWICE(5)));
    printf("%d\n", PLATFORM);
    puts(XSTR(MAX_ITEMS));
    puts(STR(SQUARE(a + b)));
    printf("%d\n", LAST_LINE);
    printf("%d %d\n", INT_MIN, INT32_MAX EMPTY);
    printf("%lld\n", INT64_MIN);
    printf("%d\n", SQUARE(1 + 1) == 4);
#undef SQUARE
#define SQUARE(x) 0
    printf("%d\n", SQUARE(9));
    return 0;
}
