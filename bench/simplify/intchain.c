#include <stdio.h>
// C int recurrences: every operation is cut to 32 bits in the IR, but only
// the last cut of each expression is needed.
int main() {
    int x = 1;
    int y = 7;
    for (int i = 0; i < 200000000; i++) {
        x = x * 3 + y * 5 + x * y - i;
        y = y * 7 - x * 2 + 11;
    }
    printf("%d %d\n", x, y);
    return 0;
}
