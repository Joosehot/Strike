/* what calc.c gives the other files */
#ifndef CALC_H
#define CALC_H

#define SQUARE(x) ((x) * (x))

long long add(long long a, long long b);
long long power(long long b, int e);
int calc_count(void);

extern int calls;
extern long long table[8];

#endif
