#include <stdio.h>
// Element by element: doubles (y = a x + y), long long and C int arrays,
// 200 times over arrays of 10000.
double x[10000];
double y[10000];
long long p[10000];
long long q[10000];
int u[10000];
int v[10000];
int main() {
    int n = 10000;
    for (int i = 0; i < n; i++) {
        x[i] = i * 0.5;
        y[i] = 1.0;
        p[i] = i;
        q[i] = i * 3;
        u[i] = i;
        v[i] = 7 - i;
    }
    double a = 1.0000001;
    for (int r = 0; r < 200; r++) {
        for (int i = 0; i < n; i++) {
            y[i] = a * x[i] + y[i];
        }
        for (int i = 0; i < n; i++) {
            p[i] = (p[i] ^ q[i]) + (q[i] >> 3);
        }
        for (int i = 0; i < n; i++) {
            u[i] = u[i] * 3 + (v[i] & 255) - r;
        }
    }
    long long s = 0;
    for (int i = 0; i < n; i++) {
        s += p[i] + u[i];
    }
    printf("%.17g %lld\n", y[n - 1], s);
    return 0;
}
