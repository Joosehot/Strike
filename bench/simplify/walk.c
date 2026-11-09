#include <stdio.h>
// A walk through a table, each step's place read from the step before:
// the time is the address arithmetic and the load, row by row.
long long m[1024 * 64];
int main() {
    long long n = 1024;
    for (long long k = 0; k < 1024 * 64; k++) {
        m[k] = (k * 2654435761) % 1021;
    }
    long long j = 0;
    long long s = 0;
    for (long long r = 0; r < 3000; r++) {
        for (long long i = 0; i < 64; i++) {
            for (long long t = 0; t < 128; t++) {
                j = (m[i * n + j] + t) & 1023;
                s = s + j;
            }
        }
    }
    printf("%lld %lld\n", j, s);
    return 0;
}
