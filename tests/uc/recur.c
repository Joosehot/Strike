#include <stdio.h>
// C int recurrences whose cuts move to the loop's exits, chained through
// each other (x's cut and y's), with a break as a second way out, and a
// long long chain whose terms come late. The ints overflow: the .out is
// clang -O2 -fwrapv's (C leaves the overflow undefined; Strike wraps, and
// clang -O2 gives the same numbers here).
int main() {
    int x = 1;
    int y = 7;
    for (int round = 0; round < 4; round++) {
        for (int i = 0; i < 1000; i++) {
            x = x * 3 + y * 5 + x * y - i;
            y = y * 7 - x * 2 + 11;
            if (x == 123456789) {
                break;
            }
        }
        printf("%d %d\n", x, y);
    }
    long long a = 1;
    long long b = 2;
    for (long long i = 0; i < 5000; i++) {
        a = 3 * a * a * b + 5 * a * b + 7 * b + i;
        b = b * 9 + a * 5 - b * 3 + 1;
    }
    printf("%lld %lld\n", a, b);
    return 0;
}

