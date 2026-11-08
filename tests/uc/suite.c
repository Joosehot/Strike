#include <stdio.h>

/* the same suite as suite.go and suite.py */

int fib(int n) {
    if (n < 2) return n;
    return fib(n - 1) + fib(n - 2);
}

int gcd(int a, int b) {
    while (b != 0) {
        int t = a % b;
        a = b;
        b = t;
    }
    return a;
}

int is_prime(int n) {
    if (n < 2) return 0;
    for (int d = 2; d * d <= n; d++) {
        if (n % d == 0) return 0;
    }
    return 1;
}

int collatz(long n) {
    int steps = 0;
    while (n != 1) {
        if (n % 2 == 0) n = n / 2; else n = 3 * n + 1;
        steps++;
    }
    return steps;
}

int sum6(int a, int b, int c, int d, int e, int f) {
    return a + 2 * b + 3 * c + 4 * d + 5 * e + 6 * f;
}

int main() {
    printf("fib %d\n", fib(25));
    printf("gcd %d\n", gcd(1071, 462));
    int count = 0;
    for (int i = 0; i < 10000; i++) {
        if (is_prime(i)) count++;
    }
    printf("primes %d\n", count);
    printf("collatz %d\n", collatz(27));
    printf("sum6 %d\n", sum6(1, 2, 3, 4, 5, 6));
    printf("div %d %d %d %d\n", -7 / 2, -7 % 2, 7 / -2, 7 % -2);
    printf("lt %d %d\n", 3 < 4, 4 < 3);
    printf("min %lld\n", -9223372036854775807 - 1);
    int pairs = 0;
    for (int i = 0; i < 10; i++) {
        if (i == 7) break;
        for (int j = 0; j < 10; j++) {
            if (j % 3 == 0) continue;
            pairs += i * j;
        }
    }
    printf("pairs %d\n", pairs);
    puts("done");
    return 0;
}
