#include <stdio.h>
// A polynomial of a number that changes every time round: Horner's form
// takes two multiplications where it is written with six.
int main() {
    long long s = 0;
    for (long long i = 0; i < 200000000; i++) {
        long long x = i ^ (i >> 3);
        s = s + 3 * x * x * x + 5 * x * x + 7 * x + 11;
    }
    printf("%lld\n", s);
    return 0;
}
