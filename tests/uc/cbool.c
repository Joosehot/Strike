#include <stdio.h>
#include <stdbool.h>
struct S { bool on; int n; };
bool flip(bool b) { return !b; }
bool big(int x) { return x; }
int main() {
    bool b = 5;
    bool z = 0;
    bool f = 0.25;
    bool arr[3] = {7, 0, -1};
    struct S s;
    s.on = 300;
    s.n = s.on + 1;
    b += 1;
    z = z + 2;
    arr[1] = 256;
    printf("%d %d %d %d %d %d\n", b, z, f, arr[0], arr[1], arr[2]);
    printf("%d %d %d %d\n", s.on, s.n, flip(b), big(512));
    int sum = b + b + z;
    printf("%d %d\n", sum, (int)sizeof(bool));
    return 0;
}
