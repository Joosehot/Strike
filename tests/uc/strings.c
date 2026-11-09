// Strings as clang makes them: char arrays and their zero byte, literals
// read through pointers, printf's %s and %c, puts, strlen and strcmp.
#include <stdio.h>
#include <string.h>

char banner[] = "== strings ==";
char names[3][8] = {"ann", "bo", "cyrus"};
const char *motto = "keep it exact";

int count(const char *s, char c) {
    int n = 0;
    int i = 0;
    while (s[i]) {
        if (s[i] == c) {
            n++;
        }
        i++;
    }
    return n;
}

void upper(char s[]) {
    for (int i = 0; s[i] != 0; i++) {
        if (s[i] >= 'a' && s[i] <= 'z') {
            s[i] = s[i] - 32;
        }
    }
}

void reverse(char s[]) {
    int n = strlen(s);
    for (int i = 0; i < n / 2; i++) {
        char t = s[i];
        s[i] = s[n - 1 - i];
        s[n - 1 - i] = t;
    }
}

// the decimal digits of n into buf, as a string
void itoa(long long n, char buf[]) {
    char tmp[24];
    int k = 0;
    int neg = n < 0;
    if (n == 0) {
        tmp[k] = '0';
        k++;
    }
    while (n != 0) {
        long long d = n % 10;
        if (d < 0) {
            d = -d;
        }
        tmp[k] = '0' + d;
        k++;
        n = n / 10;
    }
    int j = 0;
    if (neg) {
        buf[j] = '-';
        j++;
    }
    while (k > 0) {
        k--;
        buf[j] = tmp[k];
        j++;
    }
    buf[j] = 0;
}

int main() {
    puts(banner);
    puts(motto);
    puts("a literal");
    char s[] = "hello";
    char t[10] = "ab";
    char exact[3] = "xyz";
    char blank[4];
    blank[0] = 0;
    const char *p = "pointer";
    char *q = s;
    printf("%s %s %s|%zu %d [%s]\n", s, t, p, strlen(t), 6, blank);
    printf("%c%c%c\n", s[0], p[1], exact[2]);
    printf("%d %d %d %d\n", count(s, 'l'), count("banana", 'a'), count(p, 'z'), count("", 'a'));
    upper(s);
    puts(s);
    printf("%s|%s\n", q, s);
    reverse(s);
    printf("%s\n", s);
    printf("%d %d %d %d %d\n", strcmp("abc", "abd"), strcmp("b", "a"), strcmp(s, "OLLEH"), strcmp("", "x"), strcmp("ab", "abc"));
    int n = strlen(motto);
    printf("%d %zu %zu\n", n, strlen(banner), strlen(""));
    for (int i = 0; i < strlen(p); i++) {
        printf("%c", p[i]);
    }
    printf("\n");
    int j = -1;
    if (j < strlen(p)) {
        puts("signed");
    } else {
        puts("unsigned: -1 is the largest size_t");
    }
    if (strlen(p) > j) {
        puts("signed");
    } else {
        puts("unsigned again");
    }
    long long d = strlen("ab") - strlen("abcd");
    printf("%lld\n", d);
    t[2] = 'c';
    t[3] = 0;
    printf("[%s] %d %d\n", t, t[5], strlen(t) == 3);
    printf("%c|%s|%c|%c\n", 'A' + 1, "lit", 256 + 'C', -191);
    for (int i = 0; i < 3; i++) {
        printf("%d:%s(%zu) ", i, names[i], strlen(names[i]));
    }
    printf("\n");
    names[1][1] = 'b';
    puts(names[1]);
    char buf[24];
    long long nums[5] = {0, 7, -42, 1234567890123, -9223372036854775807 - 1};
    for (int i = 0; i < 5; i++) {
        itoa(nums[i], buf);
        printf("%s/%zu ", buf, strlen(buf));
    }
    printf("\n");
    char word[16] = "level";
    char copy[16];
    int k = 0;
    while (word[k]) {
        copy[k] = word[k];
        k++;
    }
    copy[k] = 0;
    reverse(copy);
    if (strcmp(word, copy) == 0) {
        printf("%s is a palindrome\n", word);
    }
    return 0;
}
