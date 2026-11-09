#include <stdio.h>
#include <math.h>

double x[5] = {0, 4.84143144246472090, 8.34336671824457987, 12.8943695621391310, 15.3796971148509165};
double y[5] = {0, -1.16032004402742839, 4.12479856412430479, -15.1111514016986312, -25.9193146099879641};
double z[5] = {0, -0.103622044471123109, -0.403523417114321381, -0.223307578892655734, 0.179258772950371181};
double vx[5] = {0, 0.606326392995832, -1.01077434617430, 1.08279100644154, 0.979090732243898};
double vy[5] = {0, 2.81198684491626, 1.82566237123041, 0.868713018169608, 0.594698998647676};
double vz[5] = {0, -0.0252183616598876, 0.00841576137658415, -0.0108326374013636, -0.0347559555040781};
double m[5] = {39.4784176043574, 0.0376936748703895, 0.0112863261319688, 0.00172372405705189, 0.00203368686992463};

double energy() {
    double e = 0;
    for (int i = 0; i < 5; i++) {
        e += 0.5 * m[i] * (vx[i] * vx[i] + vy[i] * vy[i] + vz[i] * vz[i]);
        for (int j = i + 1; j < 5; j++) {
            double dx = x[i] - x[j];
            double dy = y[i] - y[j];
            double dz = z[i] - z[j];
            e -= m[i] * m[j] / sqrt(dx * dx + dy * dy + dz * dz);
        }
    }
    return e;
}

void advance(double dt) {
    for (int i = 0; i < 5; i++) {
        for (int j = i + 1; j < 5; j++) {
            double dx = x[i] - x[j];
            double dy = y[i] - y[j];
            double dz = z[i] - z[j];
            double d2 = dx * dx + dy * dy + dz * dz;
            double mag = dt / (d2 * sqrt(d2));
            vx[i] -= dx * m[j] * mag;
            vy[i] -= dy * m[j] * mag;
            vz[i] -= dz * m[j] * mag;
            vx[j] += dx * m[i] * mag;
            vy[j] += dy * m[i] * mag;
            vz[j] += dz * m[i] * mag;
        }
    }
    for (int i = 0; i < 5; i++) {
        x[i] += dt * vx[i];
        y[i] += dt * vy[i];
        z[i] += dt * vz[i];
    }
}

int main() {
    printf("%.9f\n", energy());
    for (int k = 0; k < 1000000; k++) {
        advance(0.01);
    }
    printf("%.9f\n", energy());
    return 0;
}
