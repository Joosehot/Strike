#include <stdio.h>
// Midpoint integration of 4 / (1 + x^2) over [0, 1], 50 million steps (a
// float reduction: never reordered).
int main() {
    long long n = 50000000;
    double h = 1.0 / n;
    double s = 0.0;
    for (long long i = 0; i < n; i++) {
        double x = (i + 0.5) * h;
        s += 4.0 / (1.0 + x * x);
    }
    printf("%.15f\n", s * h);
    return 0;
}
