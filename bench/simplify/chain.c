#include <stdio.h>
// A recurrence: each value is a polynomial of the one before, so nothing
// overlaps and nothing can be vectorized; the polynomial's own work is
// the time.
int main() {
    long long x = 1;
    long long y = 2;
    for (long long i = 0; i < 100000000; i++) {
        x = 3 * x * x * y + 5 * x * y + 7 * y + i;
        y = y * 9 + x * 5 - y * 3 + 1;
    }
    printf("%lld %lld\n", x, y);
    return 0;
}
