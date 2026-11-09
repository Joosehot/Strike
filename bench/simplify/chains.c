#include <stdio.h>
// chain.c with clang's substitution written by hand: P = x(3x + 5) + 7,
// x' = Py + i and y' = y(5P + 6) + 5i + 1 from the same P, so y' does not
// wait for x'.
int main() {
    long long x = 1;
    long long y = 2;
    for (long long i = 0; i < 100000000; i++) {
        long long p = x * (3 * x + 5) + 7;
        long long nx = p * y + i;
        y = y * (5 * p + 6) + 5 * i + 1;
        x = nx;
    }
    printf("%lld %lld\n", x, y);
    return 0;
}
