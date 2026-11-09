#include <stdio.h>
int main() {
    int x = 5;
    long long y = 1;
    int a[2] = {12, 10};
    x <<= 3; x >>= 1; x &= 0x1E; x |= 64; x ^= 5;
    y <<= 40; y ^= 12345; y >>= 3;
    a[0] ^= 7; a[1] <<= 2; a[1] |= 1;
    printf("%d %lld %d %d\n", x, y, a[0], a[1]);
    return 0;
}

