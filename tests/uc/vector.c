#include <stdio.h>
#include <math.h>
// Loops that vectorize.rs makes work on two or four elements an
// instruction, and loops it must leave scalar; lengths 0 to 37 so the
// loops that finish the leftover elements run every way. No signed
// overflow, and no two NaNs meet (clang -O2, the reference, may take either).

long long ga[64];
long long gb[64];
long long gc[64];
int ia[64];
int ib[64];
int ic[64];
double da[64];
double db[64];
double dc[64];
double m[40][40];

void fill() {
    for (int i = 0; i < 64; i++) {
        ga[i] = i * 37 % 101 - 50;
        gb[i] = i * 11 % 23;
        gc[i] = 1000 - i;
        ia[i] = i * 13 % 31 - 15;
        ib[i] = i % 7;
        ic[i] = 3 * i;
        da[i] = (i * 17 % 29) / 8.0 - 1.5;
        db[i] = (i % 5) * 0.25;
        dc[i] = 0.0;
    }
    da[13] = -1.0 / 0.0;
    db[12] = 1.0 / 0.0;
    da[10] = 1.0 / 0.0;
    db[11] = -0.0;
}

long long sum_ll(int n) {
    long long s = 7;
    for (int i = 0; i < n; i++) {
        s += ga[i] ^ (i + 3);
    }
    return s;
}

int sum_int(int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        s += ia[i] * ib[i] + (ia[i] >> 1);
    }
    return s;
}

int bits(int n) {
    int x = 0;
    int o = 0;
    int a = -1;
    for (int i = 0; i < n; i++) {
        x = x ^ (ia[i] + i);
        o = o | ib[i];
        a = a & (ic[i] + 1000);
    }
    return x + 3 * o + 7 * a;
}

long long minus(int n) {
    long long s = 100000;
    for (long long i = 0; i < n; i++) {
        s -= gb[i] * 9;
    }
    return s;
}

void lanes(int n, long long k, double y) {
    for (int i = 0; i < n; i++) {
        gc[i] = (ga[i] + gb[i]) - (ga[i] << 2) + (gb[i] >> 1) + k;
    }
    for (int i = 0; i < n; i++) {
        ic[i] = ia[i] * 5 - (ib[i] & 3) + ia[i] * ib[i];
    }
    for (int i = 0; i < n; i++) {
        dc[i] = (da[i] + db[i]) * y - sqrt(fabs(da[i])) / (db[i] + 2.0);
    }
    for (int i = 0; i < n; i++) {
        dc[i] += -da[i] * y;
    }
}

// a dependency from one time round to the next: never vectorized
void shift(int n) {
    for (int i = 0; i < n; i++) {
        ga[i + 1] = ga[i] + 1;
    }
}

// a float reduction: never reordered
double dot(int n) {
    double s = 0.0;
    for (int i = 0; i < n; i++) {
        s += da[i] * db[i];
    }
    return s;
}

// pointer parameters that may be the same array
void through(long long *p, long long *q, int n) {
    for (int i = 0; i < n; i++) {
        p[i] = q[i + 1] + 1;
    }
}

void matrix(int n) {
    for (int i = 0; i < n; i++) {
        for (int j = 0; j < n; j++) {
            m[i][j] = 0.0;
        }
        for (int k = 0; k < n; k++) {
            double x = da[(i + k) % 64];
            for (int j = 0; j < n; j++) {
                m[i][j] += x * db[(j + k) % 8];
            }
        }
    }
}

void show() {
    long long h = 0;
    for (int i = 0; i < 64; i++) {
        h = h * 31 % 1000000007 + ga[i] + gc[i] + ic[i];
    }
    printf("%lld", h);
    for (int i = 0; i < 40; i++) {
        printf(" %.17g", dc[i]);
    }
    printf("\n");
}

int main() {
    for (int n = 0; n <= 37; n += 1) {
        fill();
        printf("%d: %lld %d %d %lld %.17g\n", n, sum_ll(n), sum_int(n), bits(n), minus(n), dot(n));
        lanes(n, n * 3, 1.5);
        shift(n % 9);
        through(ga, ga, n);
        through(gc, gb, n);
        show();
    }
    matrix(40);
    double t = 0.0;
    for (int i = 0; i < 40; i++) {
        for (int j = 0; j < 40; j++) {
            t = t + m[i][j] * (i - j);
        }
    }
    printf("%.17g\n", t);
    return 0;
}
