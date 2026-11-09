#include <stdio.h>

/* how many numbers, then the numbers: their sum */

int main() {
    int n = 0;
    long long sum = 0;
    scanf("%d", &n);
    for (int i = 0; i < n; i++) {
        int x = 0;
        scanf("%d", &x);
        sum += x;
    }
    printf("%lld\n", sum);
    return 0;
}