#include <stdio.h>
// A 600x600 matrix product of long long: c[i][j] += a[i][k] * b[k][j] over
// j (a 64-bit product, which SSE2 has no instruction for).
long long a[600][600];
long long b[600][600];
long long c[600][600];
int main() {
    int n = 600;
    for (int i = 0; i < n; i++) {
        for (int j = 0; j < n; j++) {
            a[i][j] = (i * 7 + j * 3) % 11;
            b[i][j] = (i * 5 + j * 2) % 13;
        }
    }
    for (int i = 0; i < n; i++) {
        for (int k = 0; k < n; k++) {
            long long x = a[i][k];
            for (int j = 0; j < n; j++) {
                c[i][j] += x * b[k][j];
            }
        }
    }
    long long s = 0;
    for (int i = 0; i < n; i++) {
        for (int j = 0; j < n; j++) {
            s += c[i][j] * (i + j);
        }
    }
    printf("%lld\n", s);
    return 0;
}
