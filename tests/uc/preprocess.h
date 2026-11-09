/* preprocess.h: macros and a guard, read through #include "..." */
#ifndef PREPROCESS_H
#define PREPROCESS_H

#define MAX_ITEMS 5
#define SQUARE(x) ((x) * (x))
#define CUBE(x) (SQUARE(x) * (x))
#define STR(x) #x
#define XSTR(x) STR(x)
/* a function named by pasting: get_one, get_two */
#define GETTER(name, value) \
    int get_##name(void) { return value; }

#endif
