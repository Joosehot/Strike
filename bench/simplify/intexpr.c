#include <stdio.h>
// C int arithmetic: every operation is cut to 32 bits, but only the last
// cut of a ring expression is needed.
int main() {
    int s = 0;
    for (int i = 0; i < 200000000; i++) {
        int a = i ^ 12345;
        int b = i >> 2;
        s = s + a * b * 3 + a * 5 - b * 7 + (a ^ b) * 9;
    }
    printf("%d\n", s);
    return 0;
}
