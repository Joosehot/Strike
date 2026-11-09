#include <stdio.h>
// Repeated terms: a*b + a*c + a*d - a*e is a*(b + c + d - e).
int main() {
    long long s = 0;
    for (long long i = 0; i < 200000000; i++) {
        long long a = i ^ 0x5bd1e995;
        long long b = i >> 1;
        long long c = i >> 2;
        long long d = i >> 3;
        long long e = i >> 4;
        s = s + a * b + a * c + a * d - a * e;
    }
    printf("%lld\n", s);
    return 0;
}
