#include <stdio.h>
// A 300x300 matrix product of doubles, 4 times: c[i][j] += a[i][k] * b[k][j]
// over j, element by element (no reduction).
double a[300][300];
double b[300][300];
double c[300][300];
int main() {
    int n = 300;
    for (int i = 0; i < n; i++) {
        for (int j = 0; j < n; j++) {
            a[i][j] = (i * 7 + j * 3) % 11 * 0.5;
            b[i][j] = (i * 5 + j * 2) % 13 * 0.25;
        }
    }
    for (int r = 0; r < 4; r++) {
        for (int i = 0; i < n; i++) {
            for (int j = 0; j < n; j++) {
                c[i][j] = 0.0;
            }
            for (int k = 0; k < n; k++) {
                double x = a[i][k];
                for (int j = 0; j < n; j++) {
                    c[i][j] += x * b[k][j];
                }
            }
        }
    }
    double s = 0.0;
    for (int i = 0; i < n; i++) {
        for (int j = 0; j < n; j++) {
            s += c[i][j];
        }
    }
    printf("%f\n", s);
    return 0;
}
