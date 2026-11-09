#include <stdio.h>
#include <math.h>

// floats as C has them (clang on Windows, the UCRT's printf): double and
// float, the usual arithmetic conversions, casts, and printf's formats

double total = 0.0;
float gain = 1.5f;
double table[4] = {0.5, 1, 2.25};

double hyp(double a, double b) {
    return sqrt(a * a + b * b);
}

double mean(double xs[], int n) {
    double s = 0;
    for (int i = 0; i < n; i++) {
        s += xs[i];
    }
    return s / n;
}

float halve(float x) {
    return x / 2;
}

int round_down(double x) {
    return x;
}

void add(double x) {
    total = total + x;
}

int main() {
    double d = 1.0 / 3;
    float f = 0.1f;
    float g = 0.1;
    int i = 7;
    long long big = 9007199254740993;
    printf("%f %.17g %e %g\n", d, d, d, d);
    printf("%f %.10f %.20f %g %g\n", f, f, g, f * 3, f * 3.0);
    printf("%.17g %.17g %.17g\n", (double)big, (float)big, big + 0.5);
    printf("%d %d %d %d %d\n", (int)2.9, (int)-2.9, round_down(1e9 + 0.5), (int)(d * 100), i / 2);
    printf("%.2f %.2f %.3f %f\n", (double)i / 2, i / 2.0, i * 0.1, i / 2 * 1.0);
    printf("%f %f %f\n", halve(3), halve(f), gain * 2);
    printf("%.0f %.0f %.0f %.0f %.1f %.2f %.3f\n", 0.5, 1.5, 2.5, -0.5, 0.25, 1.005, 2.0005);
    printf("%e %.0e %.3E %g %G %g %g\n", 12345.678, 12345.678, 0.000123456, 1e-5, 1e-5, 123456789.0, 100000.0);
    printf("%g %g %.3g %.10g %g %lf %le %lg\n", 0.0001, 1e6, 1.0, d, -0.0, 2.5, 2.5, 2.5);
    printf("%f %e %.1f\n", 1e300, 1e-300, 4.35);
    printf("%f %g %e %f\n", 1e308 * 10, -1e308 * 10, 1e308 * 10, -0.0);

    printf("%f %f %f %f\n", hyp(3, 4), hyp(1, 1), sqrt(2), fabs(-2.5));
    double xs[5] = {1, 2.5, -0.75};
    xs[3] = 1e30;
    xs[4] = xs[0] * 2;
    xs[1] += 1;
    xs[2] *= i;
    printf("%g %g %g %g %g mean %g\n", xs[0], xs[1], xs[2], xs[3], xs[4], mean(xs, 5));
    table[3] = table[1] + table[2];
    printf("%g %g\n", table[3], mean(table, 4));
    float fs[3] = {0.1f, 1, 0.3};
    fs[2] += 0.1;
    printf("%.10f %.10f %.10f\n", fs[0], fs[1], fs[2]);

    for (int k = 0; k < 10; k++) {
        add(0.1);
    }
    printf("%.17g %d %d\n", total, total == 1.0, total < 1);

    double acc = 0;
    for (int k = 1; k <= 1000; k++) {
        acc += 1.0 / (k * k);
    }
    printf("%.15f %.15f %d\n", acc, acc * 6, acc >= 1.6449);
    double n = 2;
    for (int k = 0; k < 30; k++) {
        n = (n + 2 / n) / 2;
    }
    printf("%.17g %.17g %d\n", n, n * n, n * n == 2);
    double t = 1;
    t /= 3;
    t *= 3;
    t -= 1;
    t++;
    char c = 'A';
    c += 1.75;
    printf("%g %d %d\n", t, c, !0.0 + !0.5);
    if (0.1) {
        puts("0.1 is true");
    }
    return 0;
}
