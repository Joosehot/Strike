#include <stdio.h>
// Work across statements: an unrolled loop's copies (the counter even, so
// (i + 1) >> k is i >> k), locals made again of what they already hold,
// expressions shared by statements, and C int values whose ranges make
// their cuts needless. No signed operation overflows.

long long halves(long long n) {
    long long s = 0;
    for (long long i = 0; i < n; i++) {
        long long a = i ^ 1540483477;
        long long b = i >> 1;
        long long c = i >> 2;
        long long d = (i + 1) >> 3;
        s = s + a * b + a * c - a * d + ((i + 1) & 6) + ((i + 3) ^ 5);
    }
    return s;
}

long long shared(long long x, long long y, long long n) {
    long long s = 0;
    long long t = 0;
    for (long long i = 0; i < n; i++) {
        long long p = (x + i) * (y - i) + x * 3;
        long long q = (x + i) * (y - i) - y * 5;
        s = s + p * 2 + (x + i) * (y - i);
        t = t + q;
        p = (x + i) * (y - i) + x * 3;
        s = s - p;
    }
    return s * 7 + t;
}

int narrow(int n) {
    int s = 0;
    for (int i = 0; i < 1000; i++) {
        int a = (i & 255) * 1000 + i * 3;
        int b = i * i - 500 * i;
        short c = i * 31 + 7;
        s = (s + a + b + c) % 1000003;
    }
    for (int j = 0; j < n; j++) {
        s = s + j * 2 + (j >> 1);
    }
    return s;
}

int main() {
    printf("%lld %lld %lld\n", halves(1000), halves(1001), halves(7));
    printf("%lld %lld\n", shared(12, -7, 100), shared(-3, 40, 51));
    printf("%d %d\n", narrow(10), narrow(999));
    return 0;
}
