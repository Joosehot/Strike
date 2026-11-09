#include <stdio.h>
// Expressions the release simplifier rewrites: polynomials, shared
// factors, constant multiples, index arithmetic in loops, C int and char
// cuts, and atoms (division, remainder, comparisons, shifts right) inside.
// Every number stays small enough that no signed operation overflows.

long long g[256];

long long poly(long long x) {
    return 3 * x * x * x + 5 * x * x + 7 * x + 11;
}

long long factor(long long a, long long b, long long c, long long d) {
    return a * b + a * c - a * d + b * c * 3 - c * b;
}

long long consts(long long x, long long y) {
    return x * 9 + y * 5 + x * 3 - y * 7 + (x << 3) - (y << 1) + x * 2 + 17;
}

long long atoms(long long a, long long b, long long c) {
    long long q = (a * b + a * c) / 3 + (a * b - a * c) % 5;
    long long r = (a * 3 + b > c * 2) + (a * a - b * b == (a + b) * (a - b));
    return q * 7 + r * (a + 1) - (a * b >> 2) + ~a * 3 + -b * 4;
}

int narrow(int a, int b) {
    int x = a * b * 3 + a * 5 - b * 7 + (a ^ b) * 9;
    char c = a * 37 + b * 11;
    short s = a * 300 + b * b;
    return x + c * 2 + s - (x / 7) * 3;
}

long long rows(long long n, long long m) {
    long long s = 0;
    for (long long i = 0; i < n; i++) {
        for (long long j = 0; j < m; j++) {
            s = s + g[(i * m + j) & 255] * (j + 1) + (i * m + j) * 8 + i * 3;
        }
    }
    return s;
}

long long local_rows(long long k) {
    long long a[64];
    for (long long i = 0; i < 64; i++) {
        a[i] = i * i - k;
    }
    long long s = 0;
    for (long long i = 0; i < 8; i++) {
        for (long long j = 0; j < 8; j++) {
            s = (s * 3 + a[i * 8 + j] - a[j * 8 + i] * 2) % 1000003;
        }
    }
    return s;
}

int main() {
    for (long long k = 0; k < 256; k++) {
        g[k] = (k * 37 + 11) % 101 - 50;
    }
    long long s = 0;
    for (long long x = -20; x <= 20; x++) {
        s = s + poly(x);
        printf("%lld %lld %lld\n", poly(x), factor(x, x + 3, 7 - x, x * 2), consts(x, 3 - x));
        printf("%lld %d\n", atoms(x, x * 3 - 5, 11 - x), narrow(x * 101, x - 7));
    }
    printf("%lld %lld %lld\n", s, rows(9, 13), rows(16, 16));
    printf("%lld %lld\n", local_rows(5), local_rows(-3));
    return 0;
}
