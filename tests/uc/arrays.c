#include <stdio.h>

int squares[10];
int primes[] = {2, 3, 5, 7, 11};
long long big[3] = {1, 2};

int sum(int a[], int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        s += a[i];
    }
    return s;
}

void sort(int *a, int n) {
    for (int i = 0; i < n; i++) {
        for (int j = 0; j + 1 < n - i; j++) {
            if (a[j] > a[j + 1]) {
                int t = a[j];
                a[j] = a[j + 1];
                a[j + 1] = t;
            }
        }
    }
}

int trace(int g[][4], int n) {
    int t = 0;
    for (int i = 0; i < n; i++) {
        t += g[i][i];
    }
    return t;
}

int depth(int n) {
    int local[3] = {n, n * 2};
    if (n == 0) {
        return local[0] + local[1] + local[2];
    }
    int below = depth(n - 1);
    return below + local[0] + local[1];
}

int main() {
    for (int i = 0; i < 10; i++) {
        squares[i] = i * i;
    }
    printf("%d %d\n", squares[9], sum(squares, 10));
    printf("%d\n", sum(primes, 5));
    big[2] = 3000000000;
    printf("%lld\n", big[0] + big[1] + big[2]);

    int a[8] = {5, 3, 8, 1, 9, 2, 7, 4};
    sort(a, 8);
    for (int i = 0; i < 8; i++) {
        printf("%d ", a[i]);
    }
    printf("\n");

    int g[4][4];
    for (int i = 0; i < 4; i++) {
        for (int j = 0; j < 4; j++) {
            g[i][j] = i * 10 + j;
        }
    }
    g[2][2] += 100;
    g[3][3]++;
    printf("%d %d\n", trace(g, 4), g[1][3]);

    char c[3];
    c[0] = 300;
    c[1] = 'A';
    c[2] = c[0] + c[1];
    printf("%d %d %d\n", c[0], c[1], c[2]);

    int w[2] = {2147483647, 1};
    w[0] += w[1];
    printf("%d\n", w[0]);

    int counts[5] = {0};
    for (int k = 0; k < 100; k++) {
        counts[k % 5] += k;
    }
    for (int k = 0; k < 5; k++) {
        printf("%d ", counts[k]);
    }
    printf("\n");

    int total = 0;
    for (int r = 0; r < 3; r++) {
        int fresh[4] = {r, r + 1};
        fresh[3] = 7;
        total += fresh[0] + fresh[1] + fresh[2] + fresh[3];
    }
    printf("%d %d\n", total, depth(5));

    int sieve[1000] = {0};
    int found = 0;
    for (int p = 2; p < 1000; p++) {
        if (!sieve[p]) {
            found++;
            for (int m = p * p; m < 1000; m += p) {
                sieve[m] = 1;
            }
        }
    }
    printf("%d\n", found);
    return 0;
}
