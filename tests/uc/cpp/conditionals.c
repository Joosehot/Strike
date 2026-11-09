/* conditionals: defined, integer expressions as clang reads them, and
   the macros clang defines itself */
#if defined(_WIN32) && _WIN64
win64
#endif
#if __STDC_VERSION__ >= 201112L
c11
#endif
#ifdef __clang__
clang __clang_major__
#endif
#ifdef __GNUC__
gnu __GNUC__
#endif
#if -1 > 0u
unsigned_compare
#else
signed_compare
#endif
#if (2 || 1 / 0) && 'A' == 65 && '\n' == 10
short_circuit
#endif
#if 0x10 == 16 && 010 == 8 && 10ULL == 10 && 18446744073709551615u == -1
numbers
#endif
#define LEVEL 3
#if LEVEL > 5
big
#elif LEVEL > 2
medium
#elif LEVEL > 1
small
#else
none
#endif
#if 0
#error never
#if garbage (
#endif
#bogus directive in a skipped group
#elif 1
elif_taken
#endif
#ifndef UNDEFINED
#define UNDEFINED 1
#endif
UNDEFINED
#if ~0 == -1 && (1 << 62) > 0 && (-8 >> 1) == -4 && 7 % -3 == 1 && -7 / 2 == -3
arith
#endif
#if 1 ? 2 : (1 / 0)
ternary
#endif
#if __OPTIMIZE__ && __SIZEOF_LONG__ == 4 && _MSC_VER && __x86_64__
target
#endif
#if NOT_A_MACRO || defined NOT_A_MACRO
wrong
#endif
#define ZERO 0
#if ZERO
wrong
#elif !ZERO && (ZERO + 1) * 3 == 3
zero
#endif
#if 0
#else
#if 1
nested_else
#endif
#endif
line __LINE__
