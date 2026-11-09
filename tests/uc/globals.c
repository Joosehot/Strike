#include <stdio.h>

/* globals: given their first values before main, changed by functions */

int counter = 10;
long long total;
int limit = 3 + 4;

void bump(int by) {
    counter = counter + by;
}

int next_id() {
    counter++;
    return counter;
}

int main() {
    bump(5);
    printf("%d\n", counter);
    int a = next_id();
    int b = next_id();
    printf("%d %d\n", a, b);
    for (int i = 0; i < limit; i++) {
        total += i * counter;
    }
    printf("%lld\n", total);
    int counter = 99;
    printf("%d\n", counter);
    return 0;
}