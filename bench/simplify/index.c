#include <stdio.h>
// Index arithmetic in loops: m[(i * n + j)] read row by row, the row's
// address worked out once per row.
long long m[1000000];
int main() {
    long long n = 1000;
    for (long long k = 0; k < 1000000; k++) {
        m[k] = k * 7 + 3;
    }
    long long s = 0;
    for (long long r = 0; r < 40; r++) {
        for (long long i = 0; i < n; i++) {
            for (long long j = 0; j < n; j++) {
                s = s + (m[i * n + j] ^ (j + r));
            }
        }
    }
    printf("%lld\n", s);
    return 0;
}
