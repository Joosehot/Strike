#include <stdio.h>
// The sieve of Eratosthenes below 100 million, in chars.
char comp[100000000];
int main() {
    int n = 100000000;
    int count = 0;
    for (int p = 2; p < n; p++) {
        if (!comp[p]) {
            count++;
            for (long long q = (long long)p * p; q < n; q += p) {
                comp[q] = 1;
            }
        }
    }
    printf("%d\n", count);
    return 0;
}
