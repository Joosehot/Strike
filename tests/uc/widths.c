#include <stdio.h>

/* integers as clang makes them on Windows: int and long 32 bits, long long 64 */
int twice(int x) { return x * 2; }
short sh(short a, short b) { return a + b; }
long mul(long a, long b) { return a * b; }
long long big(long long a) { return a * 3; }
int main() {
    int x = 2147483647;
    x = x + 1;
    printf("%d\n", x);
    int y = 100000;
    printf("%d\n", y * y);
    printf("%d\n", twice(1500000000));
    long z = 2000000000;
    z += z;
    printf("%d\n", z);
    printf("%d\n", mul(65536, 65536));
    printf("%d\n", sh(30000, 30000));
    char c = 100;
    c = c + 100;
    printf("%d\n", c);
    long long w = 3000000000;
    printf("%lld\n", big(w));
    int m = -2147483647 - 1;
    printf("%d\n", -m);
    int k = 7;
    k *= 1000000000;
    printf("%d\n", k);
    return 0;
}
