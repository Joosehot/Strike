/* macros: the standard's own examples (C17 6.10.3.5), and more */
#define x 3
#define f(a) f(x * (a))
#undef x
#define x 2
#define g f
#define z z[0]
#define h g(~
#define m(a) a(w)
#define w 0,1
#define t(a) a
#define p() int
#define q(x) x
#define r(x,y) x ## y
#define str(x) # x
f(y+1) + f(f(z)) % t(t(g)(0) + t)(1);
g(x+(3,4)-w) | h 5) & m
    (f)^m(m);
p() i[q()] = { q(1), r(2,3), r(4,), r(,5), r(,) };
char c[2][6] = { str(hello), str() };

#undef x
#undef f
#undef g
#undef z
#undef h
#undef m
#undef w
#undef t
#undef p
#undef q
#undef r
#undef str
#define str(s) # s
#define xstr(s) str(s)
#define debug(s, t) printf("x" # s "= %d, x" # t "= %s", \
    x ## s, x ## t)
#define INCFILE(n) vers ## n
#define glue(a, b) a ## b
#define xglue(a, b) glue(a, b)
#define HIGHLOW "hello"
#define LOW LOW ", world"
debug(1, 2);
fputs(str(strncmp("abc\0d", "abc", '\4') // this goes away
    == 0) str(: @\n), s);
xstr(INCFILE(2).h)
glue(HIGH, LOW);
xglue(HIGH, LOW)

#define hash_hash # ## #
#define mkstr(a) # a
#define in_between(a) mkstr(a)
#define join(c, d) in_between(c hash_hash d)
char p[] = join(x, y);

#undef debug
#define debug(...) fprintf(stderr, __VA_ARGS__)
#define showlist(...) puts(#__VA_ARGS__)
#define report(test, ...) ((test)?puts(#test): printf(__VA_ARGS__))
debug("Flag");
debug("X = %d\n", x);
showlist(The first, second, and third items.);
report(x>y, "x is %d but y is %d", x, y);

/* a definition over lines, a comment inside one, and a call over lines */
#define LONG_MACRO(a, b) \
    ((a) * \
     (b))
#define CMT 1 /* a comment
   over two lines */ + 2
int long_one = LONG_MACRO(CMT,
                          3);
#define EMPTY
#define CALL(f) f
int e = CALL(EMPTY) 5 EMPTY;
#define SELF SELF + 1
int s = SELF;
#define TWICE(a) a a
TWICE(TWICE(k))
#define NAME_THEN_PAREN f
NAME_THEN_PAREN
(1)
#define SQUARE(v) ((v) * (v))
#define APPLY(fn, v) fn(v)
int sq = APPLY(SQUARE, 1 + 2);
#define CAT3(a, b, c) a ## b ## c
int CAT3(var, _, 1) = CAT3(1, 2, 3);
#define STR_ESCAPE(v) #v
const char *esc = STR_ESCAPE("a\"b" '\'' x   +   y);
#define AGAIN 1
#define AGAIN 1
#undef NEVER_DEFINED
