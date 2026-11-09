#include <stdio.h>
#include <stdbool.h>
#include <limits.h>
#include "calc.h"

int next(void);
int counter_count(void);
void fill(void);

#define REPEAT 3
#if REPEAT > 2 && defined(CALC_H)
#define LABEL 1
#else
#define LABEL 0
#endif

int main() {
    printf("%lld\n", add(2, 3));
    printf("%lld\n", power(2, 10));
    for (int i = 0; i < REPEAT; i++) {
        printf("%d\n", next());
    }
    fill();
    long long s = 0;
    for (int i = 0; i < 8; i++) {
        s += table[i];
    }
    printf("%lld\n", s);
    printf("%d %d %d\n", calls, calc_count(), counter_count());
    printf("%d %d\n", INT_MAX, LABEL);
    bool b = true;
    printf("%d\n", b);
    return 0;
}
