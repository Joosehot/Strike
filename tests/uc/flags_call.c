#include <stdio.h>
// A comparison's value waits as the machine's flags; a call of a function
// of no arguments in the same expression changes the flags, so the value
// must be made a number before the call (found by the stage-4 tests:
// release gave -783 for -784).
long long g = 3;
long z() {
    if (g > 0) {
        g = g - 1;
        return z() * 2 - 1;
    }
    return -783;
}
long f(long p0, long p2, long p5) {
    return ((p0 <= (p5 == -860)) < ~(p2)) ^ z();
}
long k() {
    return z() + 1;
}
int main() {
    printf("%ld\n", f(924772656, -431, 18));
    printf("%ld\n", (g < 5) + z());
    printf("%ld\n", (g == 0) - k() * (g != 7));
    return 0;
}
