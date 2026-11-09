# Strike

**The King Killer.**

A build system that calls the compiler itself, and a compiler of its own
for every language it has a description of (below). Two parts decide
every build:

- **HotMotor** chooses how to build. One rule per decision (opt-level,
  codegen units, incremental, debuginfo, linker), each with variants scored
  on three axes (build time, program speed, disk). A beam search keeps the
  best partial builds, and judges score the choices together. Every number
  is in [rules.toml](rules.toml).
- **Lightning** remembers what was built. Every build is branded once, under
  the hash of everything that decides its output: the compiler, the flags,
  the *content* of every file the compiler read (from its own dep-info), the
  environment variables it read, and the brands it links. Before a compiler
  call, a strike looks for that brand. A file's new time is not a change, but
  new content is. Brands are hard links, so a big library is never copied.
  The lightning is also one of the motor's judges: a build already branded
  costs no build time, so it scores as much as the profile asks for build
  time.

## Rust

Strike builds Rust packages with their dependencies. The versions come from
Cargo.lock and the sources from the registry folder Cargo already
downloaded, or from fStrike (below); Strike does not resolve. Features are unified over
the graph. Build scripts are built and run, proc macros are built as
libraries the compiler loads, and every job of the graph runs as soon as
what it uses is done, on every core; a library built on another starts as
soon as the other's interface (rmeta) is written. Each flag set keeps its
own incremental folder, since rustc throws that work away when the flags
change. Brands are shared by every project: a dependency with the same
version and features is built once for all of them.

Measured on Nuome (53 locked packages, 66 jobs, 48 500 lines in one crate):

| | Strike | cargo |
|---|---|---|
| cold, empty brand store | 28.0 s | 46 s |
| nothing changed (or only a file's time) | 0.5 s | |
| one line changed in the library, warm | 4.9-5.8 s | 8.8 s (5.9 s with debug=0) |
| `strike test`: 341 tests in 3 harnesses, all pass | 101 s | |

Most of Strike's lead over cargo's defaults comes from the settings
HotMotor chose (no debug info, rust-lld); against cargo with the same
settings it is about even.

```
strike build <dir> [--for build|run] [--rules FILE] [--explain] [--verbose]
strike test  <dir> [--for build|run] [--rules FILE] [--explain] [--verbose]
strike brand
```

The output goes to `<dir>/strike-out/<profile>/`, the brands to
`~/.strike/brand/` (or `STRIKE_HOME`). `STRIKE_TIMES=1` shows where Strike's
own time goes.

## The universal compiler

```
strike compile <file>... | <dir> [-o EXE] [-I DIR] [--explain] [--no-brand] [--plain] [--release [--no-evaluate]]
```

Strike's own compiler. It does not know any language itself. Every
language is a description in [languages/](languages/):

- how its text is cut into tokens (comments, strings, a block by braces or
  by indentation, where a line ends a statement);
- its operators, with their precedence;
- its integer types and their widths;
- its forms, written as patterns. For example, `if ( $expr ) $body [ else
  $body ]` is C's if, and `for $name in range ( $args ) : $block` is
  Python's.

Each form says which universal construct it is. A new language is a new
file, not a new compiler. There are three today: C, Go and Python.

### How a file becomes an executable

(A program of several files: [below](#several-files-modules-and-cs-preprocessor).)

1. **HotMotor picks the language.** Every description is a candidate, and
   the judges in [languages/judges.toml](languages/judges.toml) score each
   one:
   - it reads the whole file and knows every name in it;
   - the file's extension is one the description names;
   - how far into the file it read.

   A description that cannot beat the best score so far is not read at all.
   A file with the wrong extension is still read in its own language.
2. **Strike IR** ([src/uc/ir.rs](src/uc/ir.rs)). The file is lowered into
   one form shared by every language: functions of stack operations on
   64-bit integers. Each language's meaning is kept in it:
   - C and Go divide toward zero, and Python rounds down.
   - Python prints `True`, Go prints `true`, and C prints `1`.
   - C's `int` and `long` wrap at 32 bits, as clang makes them on Windows.
   - Go's `int32` wraps at 32 bits.
   - Python's integers do not wrap at all.
3. **Brands per function.** Each function is branded by its IR and by the
   generator's rules. A function whose brand exists is struck from the pack
   (`~/.strike/uc/pack`) instead of compiled, even if the brand came from
   another language. The same work in two languages is the same IR: a C
   `long long` function and a Go `int` one share a brand. A C `int` one
   does not share it, and neither does a Python one.
4. **Machine code by Strike** ([src/uc/x64opt.rs](src/uc/x64opt.rs)).
   x86-64 is written byte by byte, with no LLVM and no assembler.
5. **The executable by Strike** ([src/uc/pe.rs](src/uc/pe.rs)). The PE
   file, its import table and the runtime (printing, the entry point, the
   overflow stop) are written directly, with no linker.

### What it compiles today

Integers, floats and truth values, functions and recursion, if/else,
while, for, Python's `range`, break and continue, printing (print,
Println, puts, and printf with `%d`, `%s`, `%c`, `%q`, `%v`, `%f`, `%e` and
`%g`), global variables, arrays, lists and slices, strings, and whole
numbers and lines read from standard input.

- **Globals.** C's and Go's variables outside functions get their first
  values before main runs. Python's module names are read by functions, and
  `global x` lets a function change one. Python's own rule holds too: a name
  a function assigns anywhere is its own, so reading it before that is an
  error, as Python's UnboundLocalError is.
- **Input.** C's `scanf("%d", &x)` and Go's `fmt.Scan(&x)` skip blanks and
  read a number; with none, the variable keeps its value. Python's
  `int(input())` takes a whole line. A line that is not a whole number, no
  line at all, or a number past 64 bits stops the program, with Python's
  reason. Input is read 4096 bytes at a time. A program that reads its input
  is never worked out while compiling.
- **Arrays, lists and slices** ([src/uc/lib.rs](src/uc/lib.rs): what they
  need beyond single operations is written in Strike IR, and branded,
  inlined and optimized as the program's own functions are). The IR has
  memory: loads and stores of 1, 2, 4 or 8 bytes, a function's own memory,
  globals' addresses, and a stop with the language's exit code. New memory
  comes from Strike's own allocator over VirtualAlloc; nothing is given
  back yet. An element takes its own width: a C `char` 1 byte, an `int` 4,
  a Go `int32` 4.
  - **C:** arrays local (in the function's own memory) and global, 2D,
    `{...}` initializers (the rest zero), array parameters (`int a[]`,
    `int *a`, `int g[][10]`). An index is not checked, as C does not
    check it.
  - **Go:** slices (`make`, `[]T{...}`, `append` with Go 1.27's growth and
    the allocator's size classes, so `cap` is Go's for a slice on the heap,
    slicing, `len`, `cap`) and arrays as values (copied when assigned,
    given or returned). `range` with an index, a value or both. An index
    out of range panics with exit code 2. (Go 1.25 and later keep a small
    slice that never leaves its function on the stack, where its first
    capacity is 4; Strike does not follow Go's escape analysis.)
  - **Python:** lists (`[...]`, `[x] * n`, comprehensions with `if`,
    `append`, `pop`, `+`, `+=`, slices, negative indexes, `for x in`,
    `len`, a list as a condition), printed as Python prints them. An index
    out of range is IndexError, with exit code 1. The kind of an empty
    list's elements is learned from what is appended, in any function.
- **Strings** ([src/uc/strs.rs](src/uc/strs.rs), written in Strike IR as
  the rest of the library is). A Python or Go string is the address of its
  length and its bytes, never changed once made; a literal lives in the
  read-only data (`Op::Data`), and a copy of a string is a copy of its
  address.
  - **Python:** literals, `+`, `*`, `len`, `s[i]` and negative indexes,
    slices, the comparisons, `in`, `for c in s`, `str()`, `int()` of a
    string (blanks around it, a sign, underscores between digits, else
    ValueError), `ord`, `chr`, `input()` (with a prompt too), and
    `upper`, `lower`, `strip`, `split()`, `join`, `startswith`, `find`. A
    string in a list prints as Python's repr, quotes chosen as Python
    chooses them. Strike holds Python's strings to ASCII, so a code point is
    a byte: a literal past ASCII is refused, and `chr()` past it or a line
    read that holds one stops the program with the reason.
  - **Go:** bytes. `s[i]` is a `byte` (uint8, which wraps at 256),
    `range` decodes UTF-8 as Go does (U+FFFD and one byte where it is not
    UTF-8), `string(r)` encodes it, and slicing out of range panics with
    exit code 2. Printf's `%s`, `%v`, `%c` and `%q` print as Go prints;
    `%q` of a string past ASCII stops the program (which characters Go
    keeps as they are is Unicode's table). `strconv.Itoa` is there,
    `strconv.Atoi` (two results) is not.
  - **C:** `char s[] = "abc"` and `char s[10] = "ab"` (the zero byte where
    there is room), a pointer variable given a literal or an array to point
    at, `s[i]`, `while (s[i])`, printf's `%s` and `%c`, `puts`, `strlen` and
    `strcmp` (-1, 0 or 1, as Windows' C library gives it). strlen's
    `size_t` is unsigned: it is compared unsigned, as C converts the other
    side, and dividing or shifting it is refused.

Integers are exactly what each language says they are:

- **C:** `int` and `long` are 32 bits (as clang makes them on Windows),
  `long long` is 64, `short` is 16 and `char` is 8. Arithmetic is done at
  least as wide as `int`, and a result is cut to the width of where it
  goes.
- **Go:** `int` is 64 bits and `int32` wraps at 32. A literal takes the
  width of what it meets, and one that does not fit is an error, as in Go.
- **Python:** `+`, `-`, `*`, `<<` and unary `-` are checked. Where a
  number would leave 64 bits, the program stops with exit code 1 and a
  message on standard error. It never goes on with a wrong number.

### Floats

A float is carried in the IR as its 64 bits, so the IR is still a stack
of 64-bit values. The float operations are their own operations (`FAdd`,
`FSub`, `FMul`, `FDiv`, `FRem`, the comparisons, `FNeg`, `FAbs`, `FSqrt`,
and the conversions `FromInt`, `FromInt32`, `ToF32`, `ToInt`, `ToInt32`).
The generators do them with SSE2: `addsd`, `ucomisd`, `cvtsi2sd`,
`cvttsd2si`, `sqrtsd`. `FRem` (Python's `%` needs C's `fmod`) uses the
x87's `fprem`, which is exact. The optimizing generator keeps a float in
xmm2 to xmm5 until something else needs it, and loads an array element a
float operation will use straight into one.

Folding and the IR runner work a float out bit for bit as the machine
does, including which NaN comes out: the first NaN that comes in, quieted,
or the machine's own NaN (negative, `-nan(ind)` to the UCRT) for an
invalid operation. Floats are never reassociated, and no pass treats
`x + 0.0` as `x`. A float expression may still move out of a loop, which
changes nothing.

Each language's floats are its own:

- **C:** `double` is 64 bits and `float` is 32 (`1.5f`). A float32 is held
  as the double of the same value, and every operation on float32s is
  rounded back to float32. This is exact, because a double has enough bits
  that the double rounding cannot change the result. Whole numbers are
  made floats by the usual arithmetic conversions, and `(double)x` and
  `(int)d` are casts. `sqrt` and `fabs` are there.
- **Go:** `float64` and `float32` are never mixed with each other or with
  ints without a conversion. A constant takes the type it meets, and Go's
  constants are exact numbers. An operation on two constants is taken only
  where Strike can be exact: the operands are exact as doubles, so the one
  rounding is Go's (`1.0/3.0`). A further operation is taken only on a
  result that is exact too. So `0.1 + 0.2`, which Go makes 0.3, is
  refused. `math.Sqrt` and `math.Abs` are there.
- **Python:** `/` is true division. `//` and `%` of floats are CPython's
  own algorithm. A whole number meeting a float becomes one, but a
  comparison between them is exact, as Python's is
  (`9007199254740993 > 9007199254740992.0` is True). A division by zero
  stops with ZeroDivisionError. `int()` of NaN or of an infinity stops
  with Python's ValueError or OverflowError, and of a float past 64 bits
  with Strike's own stop. `math.sqrt` of a negative number gives Python's
  ValueError and its message. A division of two whole numbers past 2^53,
  which a double cannot hold exactly, stops instead of guessing.

Printing is exact, with no tables and no approximation. It is written in
C, in [src/uc/floats.c](src/uc/floats.c), in the C that Strike compiles,
and Strike's own C description lowers it into IR like the program's own
code. A double is m * 2^e, which is a whole number of decimal digits times
a power of ten. Those digits are worked out with big numbers in limbs of
10^9.

- **Shortest** (Python's repr, Go's %v, and a float32's own for Go): the
  fewest digits that lie between the halfway points to the two neighbours,
  the points themselves allowed when m is even. Of those, the nearest
  digits are taken, half to even.
- **%f, %e, %g** with any precision up to 100: the exact digits rounded
  half to even, as the UCRT's printf and Go's strconv do. NaN and the
  infinities print as each one writes them: `-nan(ind)`, `nan(snan)` and
  `INF` for the UCRT, `NaN` and `+Inf` for Go.

floats.c is real C too. clang compiled it first, and it was checked
against printf, Go and Python on 10,095 doubles (all the powers of 2 and
10, random bit patterns, random decimals, subnormals, halfway cases):
504,750 printf results, 353,325 Go Printf results, every repr, and every
%v of float64 and float32 were all the same.
### Several files, modules and C's preprocessor

`strike compile a.c b.c`, or a directory: `strike compile prog/`. Each
description says in its `[program]` table what a directory is in its
language: every `.c` file, every `.go` file but the tests, or Python's
`__main__.py`. HotMotor still picks the language, now for the whole
program. The files are read into the universal syntax one by one
([src/uc/modules.rs](src/uc/modules.rs)) and made one program before it
is lowered:

- **C** ([src/uc/cpp.rs](src/uc/cpp.rs)). Every file is preprocessed
  first. The preprocessor does `#include "..."` (next to the including
  file, then `-I`) and `<...>`, `#define` with arguments, `#`, `##` and
  `__VA_ARGS__`, `#undef`, `#if`/`#ifdef`/`#ifndef`/`#elif`/`#else`/
  `#endif` with `defined` and clang's integer arithmetic (unsigned where
  clang makes it unsigned, `&&`, `||` and `?:` evaluating only the side
  taken), `#error`, `#pragma once` and `__LINE__`. Expansion is the
  standard's, by hide sets. The macros clang defines before any file come
  first ([languages/c-predefined.h](languages/c-predefined.h), from `clang
  -O2 -dM -E`), so `#ifdef _WIN32` or `#if __STDC_VERSION__ >= 201112L`
  finds what clang has. A header Strike knows (`stdio.h`, `stdlib.h`,
  `stdbool.h`, `stdint.h`, `limits.h`) gives what [c.toml](languages/c.toml)
  says it gives: `printf`, `puts` and `scanf` are Strike's own, and
  `INT_MAX` or `true` are clang's values. Every token keeps its file and
  line, so an error in a header points at the header's line.
- **C files** are translation units. A function or a variable outside a
  function is shared by every file, and `extern int x;` names one that
  some file defines. A `static` one is its file's own: two files may each
  have a `static int count`. A `static` variable in a function keeps its
  value from call to call, given once before main, as C gives it.
- **Go** reads `go.mod`. The files of a directory are one package, and
  `import "example.com/m/calc"` is the package in `calc/` under the module.
  `calc.Add(1, 2)`, `calc.Total` and `calc.Total += 1` reach its names. A
  name that does not begin with a capital letter is not seen outside its
  package. The packages' variables are given in Go's order: a package
  after those it imports, and of those ready the first by its path.
- **Python** imports `m.py` next to the main file: `import m`, `import m as
  k`, `from m import f, x as y`. A module runs where it is first imported,
  and only then. `from m import x` copies x as it is at that point.
  `m.f()`, `m.x`, `m.x = 1` (in a function too) and `__name__ ==
  "__main__"` mean what they do in Python.

A name of a module other than the main one is given the module's prefix
(`m.f`, `example.com/m/calc.Add`), and a C static name its file's. So
names from different files never meet in the IR. A function's brand is
still its IR: the same program given as a directory or as its files
strikes every function from the brands made the first time.

What is refused, with its file and line: `#pragma` (but `once`), `#line`,
`#include_next`, `#embed`, a computed `#include`, `__FILE__`, `__DATE__`,
`_Pragma`, `__has_include`, `__VA_OPT__`, GNU's `, ## __VA_ARGS__`, digraphs
and any header Strike does not know. Also a function or variable defined
in two C files; Go's `init`, import cycles, unused imports, packages
outside the module and `require` in go.mod; Python's circular imports,
`from m import *`, an import inside a function or a block, a module named
like one of Python's own (which Python finds first), and a module used as
a value.

Whatever Strike cannot do yet in the language's own way is refused, with
the line and the reason, and never guessed. Refused today:

- structs, pointer arithmetic, `&` and `*`, and the unsigned types other
  than Go's `byte`;
- a Python string past ASCII, a string changed in place, `split` with a
  separator, Go's `[]byte`;
- a chained comparison in Python;
- a function whose value is used but which can end without one;
- a Python variable or parameter that changes between a number and a truth
  value, or between a whole number and a float (`x = 0` then
  `x += 0.5`), and a list that holds both;
- a Go constant expression that Strike cannot work out exactly
  (`0.1 + 0.2`), and a constant that overflows its float type;
- C's `long double`, `%` on a float, a printf width or flag (`%8.3f`), a
  precision above 100, and reading a float with scanf;
- Python's `math` functions other than `sqrt`, and `round`.

### How it is checked

- [tests/uc/](tests/uc/) holds the same suite in C, Go and Python, plus
  integer-width programs. Strike's executables print byte for byte what
  clang, Go and Python print; the `.out` files were written by those
  toolchains.
- `--plain` is the first, simple generator, with no optimization at all.
  It is kept as the reference.
- One test writes 100 random C programs and builds each in five modes:
  plain, default, `--release`, `--release` with every division by a
  constant as a multiplication, and `--release --no-evaluate`. Every mode
  must give the same output and the same exit code. The programs have up
  to six parameters, every operator, calls between functions, recursion in
  every shape a return can call itself in, and sum loops. Across the 100
  programs these are all exercised:
  - inlined calls;
  - tail calls made jumps;
  - unrolled recursion;
  - fast paths;
  - expressions moved out of loops;
  - loops replaced by their sums.
- Unit tests check the magic division against real division, the
  binomials against exact numbers, and the closed form of sums against the
  loop for 300 random polynomials, with overflow.
- Globals and input are checked against clang, Go and Python with the
  same input, in every mode, and Python's input errors one by one.
- Floats have a suite in each language (tests/uc/floats.*), in every mode.
  400 doubles are printed in every format each language has
  (tests/uc/floatprint.*, written by
  [tools/float_values.py](tools/float_values.py)): ten printf formats each
  in C, Println, Printf and float32 in Go, and repr in Python. Python's
  float stops are checked one by one. The 100 random C programs have float
  functions too, with every float operator, both conversions, NaN, the
  infinities and both zeros, and they print every result's bits.
- Programs of several files ([tests/uc/multi_c/](tests/uc/multi_c/),
  [multi_go/](tests/uc/multi_go/), [multi_py/](tests/uc/multi_py/)) and a
  preprocessed C program are built in every mode, given as a directory and
  as their files. They print what clang, `go run .` and Python print. The
  preprocessor alone gives, token for token, what `clang -E -P` gives for
  [tests/uc/cpp/](tests/uc/cpp/): the standard's own macro examples,
  conditionals, and includes through guards, `#pragma once` and `-I`.
- Arrays, lists and slices are checked the same way (tests/uc/arrays.*),
  with each index error and its exit code, in every mode; the random C
  programs have local and global arrays and arrays given to functions.
- Strings are checked the same way (tests/uc/strings.*, and lines read by
  Python), with each error a string can stop on (IndexError, ValueError,
  TypeError, EOFError, Go's panics) and each refusal. The random C
  programs make strings in char arrays too, and compare them with strlen
  and strcmp.
- 82 tests in all (every target of `cargo test --release`), in about 50 s.

The tests have found real bugs, all fixed:

- a loop whose top was dropped as unreachable, so it never ended;
- a panic in release builds, from jumps left in code that folding made
  unreachable;
- a crash on frames larger than a page;
- Python's 25! and C's `int` overflow coming out wrong with no warning;
- Go's `x := int32(y)` (and `c := s[0]`) taking int's width, not the
  value's, so x did not wrap at 32 bits.

### Code generation

[src/uc/x64opt.rs](src/uc/x64opt.rs) does the following by default. Its
numbers are in [codegen.toml](codegen.toml).

- **The IR's stack is followed while compiling.** A constant, a local, a
  local plus a constant (one `lea` when needed) or a result in rax stays
  where it is until an operation needs it. A comparison followed by a jump
  is one `cmp` and one `jcc`. A value that crosses a jump stays in rax.
- **HotMotor chooses which locals live in registers.** Every local offers
  one of the seven callee-saved registers, or memory. The judge scores the
  memory operations a register saves, where a use in a loop weighs 8 times
  more per level, against the cost of saving the register. A beam keeps
  the best choices.
- **The frame has no rbp.** Saved registers are pushed and popped, and
  locals are addressed from rsp, because the generator knows how much is
  pushed at every operation. Each frame reserves its call area once.
- **Calls take their arguments straight in registers.** A result is
  stored straight into the register of the local it goes to.
- **HotMotor chooses which calls are inlined** (`[inline]`;
  [src/uc/inline.rs](src/uc/inline.rs)). For each call of a function that
  calls nothing, the judge weighs the call saved, weighted by the loops
  around it, against the code copied.
- **A function's calls of itself at its returns become jumps**
  ([src/uc/tail.rs](src/uc/tail.rs)). `return x + f()` and `return x * f()`
  get an accumulator, also through a C `int`'s cut. `fib` makes half as
  many calls.
- **Recursion is unrolled one level** (`[unroll]`), so each call that is
  left does two levels' work.
- **A frameless fast path.** A function that begins by testing its
  parameters and returning, like `if (n < 2) return n`, does that from the
  argument registers before any frame is made. fib's base case is four
  instructions.
- **A division whose numbers both fit 32 bits takes the 32-bit
  instruction** (`[division]`), which gives the same quotient and is the
  faster one.

### --release

`--release` adds the work a release build may spend more time on. Each of
these is also something clang -O2 does:

- **Constant folding** ([src/uc/fold.rs](src/uc/fold.rs)), by the
  machine's own rules.
- **Algebra** ([src/uc/opt.rs](src/uc/opt.rs)). `x + 0` and `x * 1` become
  `x`, and `x * 2^k` becomes a shift. Python's `// 2^k` and `% 2^k` become
  a shift and a mask.
- **The algebraic simplifier** ([src/uc/simplify.rs](src/uc/simplify.rs)),
  in the way of Nuome's. See [below](#the-algebraic-simplifier).
- **Loop-invariant code motion.** An expression that cannot fail, made of
  locals the loop never stores, is worked out once before the loop.
- **Division by a constant as a magic multiplication**, where HotMotor's
  judge takes it. A division whose value comes back round its loop into
  what it divides keeps the divider, which is faster there. Any other
  division is multiplied.
- **A sum's closed form** ([src/uc/scev.rs](src/uc/scev.rs)), like clang's
  SCEV. Take `for (c = a; c < N; c++) s += P(c)`, with P of degree up to 3.
  The loop becomes sum b_m C(T, m+1), modulo 2^64 with exact binomials, so
  it is bit for bit the loop's own result. A local that copies the counter
  gets the counter's last value.
- **Loop rotation and unrolling** ([src/uc/rotate.rs](src/uc/rotate.rs)).
  The test moves to the bottom, and a short body with no jump goes round
  twice (a very short one four times) per jump back. A counted loop whose
  count four or two divides has its copies with no test between them, and
  copy m reads the counter plus m, so the copies differ only by their
  addresses' displacement.
- **Array loops.** A local every store of which gives one constant is read
  as it (so `int n = 300` makes a loop's count known). A global array's
  address and a row's leave the loop. `a[i] += x` is one instruction on
  the memory, an element only tested is one `cmp` with zero, and an
  element's address is `[base + index * size + d]`. A C `int` counter's
  cut back to 32 bits is left out where the loop has just found it below a
  bound that fits: clang leaves it out because signed overflow cannot
  happen, Strike shows that it cannot.
- **Inlining callees of up to 200 operations.**
- **A program that reads nothing is worked out while compiling**
  ([src/uc/eval.rs](src/uc/eval.rs)). Strike's IR runner tries first,
  for up to 20 million steps. Then the compiled program is run once, for up
  to 2 s; it prints a marker and then main's return, so a crash is never
  taken for a result. The executable then only prints the output and
  returns the number. A program that fails or does not finish in time is
  compiled as it is. `--no-evaluate` turns this off, which is how the run
  times below are measured.

A checked Python operation is never folded, moved, summed or worked out
where it could stop the program.

### The algebraic simplifier

The simplifier is built the way Nuome's is. Nuome is the owner's math
engine, and its simplifier does a beam search over rewrite paths: rules
offer moves, and judges score the states. Here the rules are Nuome's ring
rules, ported to the machine's integers, and the judge is the x86-64 code
that x64opt.rs would make.

- **What it reads.** Every pure integer expression of the IR is read as a
  polynomial over Z/2^64 in its atoms. An atom is a leaf, or any operation
  that is not a ring operation: division, remainder, comparisons, `& | ^`,
  shifts right, shifts by a variable, and Python's checked operations. An
  atom's own operands are simplified separately.
- **The rules.** Each one is an identity mod 2^64, so it holds for the
  machine's wrapping arithmetic:
  - associativity and commutativity, and `x - y = x + (-1)y`;
  - `~x = -x - 1`, and `x << k = 2^(k mod 64) x`;
  - multiplying out;
  - collecting like terms, and the full normal form;
  - taking out a factor that two or more terms share. Done again and again,
    this is Horner's form;
  - taking out a constant (2, 4, 8 or the gcd);
  - splitting `(2^k ± 1)x` into `2^k x ± x`;
  - `0x = 0`, `1x = x`, `x + 0 = x`.
- **Narrow widths.** The low n bits of a ring expression depend only on the
  low n bits of its inputs. So under a C int's `Sext(32)`, every inner
  `Sext(m >= 32)` is dropped and the coefficients are taken mod 2^32. A cut
  is never moved across an atom, and a narrower cut stays where it is.
- **The judge.** It follows x64opt's compile-time stack:
  - a local plus a constant, times 1, 2, 4 or 8, plus another local, is one
    `lea` or an address;
  - a value that waits under another goes to the machine's stack;
  - imul takes 3 cycles, and a scaled lea takes 2;
  - `t = t + rest` is worked out in t's own register.

  The judge counts instructions plus latency. In a loop, it knows what LICM
  will take out, so the simplifier gathers invariant terms and factors into
  one expression that hoist can move: `(i*m + j)*8 + base` becomes
  `i*(8m) + (8j + base)`. A local just worked out is added last, so the
  dependency chain through it stays short.
- **Kept only if it is cheaper, and checked.** A rewrite is kept only when
  it is strictly cheaper. It is also checked first: the old and the new
  code must give the same value on 200 sets of inputs (0, ±1, the extremes,
  powers of two and random numbers), evaluated with the machine's rules.
  This is a guard. The proof is that every rule is an identity.
- **Operations that can stop the program are left alone.** An expression
  with a division by a variable or a checked operation is not rewritten
  around it. It still stops exactly where it would. Python's numbers are all
  checked, so a Python program is not changed.

The simplifier is tested in several ways:

- [tests/uc/algebra.c](tests/uc/algebra.c) and `algebra.go` must print what
  clang and Go print, in every mode.
- The random differential test now also writes ring expressions, in C
  `long` and `long long`, with index arithmetic in loops.
- Unit tests check every move of the beam against the original expression
  on random expressions, and check that the guard catches wrong rewrites.

**Running times.** These are from [bench/simplify/](bench/simplify/)
(`python bench/simplify/run.py`, with STRIKE_BASE set to a build of master).
Each program was run 11 times, with the builds interleaved. Times are in
seconds, given as the minimum / the median. The machine is a Ryzen 5 7600X
that other builds were also using. The first column is Strike before the
simplifier. "clang scalar" is `-O2 -fno-vectorize -fno-slp-vectorize`.

| | Strike before | **Strike now** | clang -O2 | clang scalar | go |
|---|---|---|---|---|---|
| poly.c (Horner) | 0.294 / 0.297 | **0.150 / 0.153** | 0.137 / 0.141 | 0.136 / 0.142 | |
| poly.go | 0.296 / 0.297 | **0.151 / 0.154** | | | 0.222 / 0.224 |
| factor.c (`ab+ac+ad-ae`) | 0.272 / 0.276 | **0.191 / 0.193** | 0.107 / 0.113 | 0.093 / 0.095 | |
| factor.go | 0.272 / 0.274 | **0.191 / 0.193** | | | 0.311 / 0.314 |
| chain.c (a recurrence) | 0.246 / 0.248 | **0.206 / 0.208** | 0.195 / 0.197 | | |
| chain.go | 0.247 / 0.248 | **0.206 / 0.209** | | | 0.235 / 0.238 |
| intchain.c (C int) | 0.605 / 0.607 | **0.376 / 0.377** | 0.275 / 0.276 | | |
| intexpr.c (C int) | 0.360 / 0.362 | **0.204 / 0.207** | 0.072 / 0.073 | 0.127 / 0.130 | |
| index.c (rows) | 0.027 / 0.028 | **0.020 / 0.021** | 0.012 / 0.012 | 0.015 / 0.016 | |
| walk.c (dependent index) | 0.064 / 0.068 | **0.055 / 0.056** | 0.059 / 0.061 | | |
| poly.py (checked) | 0.062 / 0.063 | 0.063 / 0.063 | | | |

Strike is now 1.15 to 2 times as fast as before on these programs, and
faster than Go on every Go one. Against clang -O2:

- **walk.c: Strike is 6% faster.** The row's address is taken out of the
  loop and the step is one address mode.
- **chain.c: 6% slower; poly.c: 10% slower.**
- **The rest are slower, for reasons that are not algebra:**
  - clang vectorizes (intexpr, index);
  - clang works across the copies of an unrolled loop and through locals,
    for example `(i+1) >> 1` against `i >> 1`, and in intchain the next
    iteration's x put into y;
  - clang leaves out the C int cuts, because signed overflow is undefined
    in C. Strike keeps one cut per expression, which is the exact result.

**Compile time** (`python bench/simplify/compile_time.py`), what Strike
reports, minimum of the rounds:

| program | before | now |
|---|---|---|
| suite.c, algebra.c and arrays.c | 0.007 s | 0.013 s |
| the 100 random test programs | 0.523 s | 0.562 s (+7%) |
| 2000 functions of dense random polynomials ([genbig.py](bench/simplify/genbig.py)) | 0.86 s | 1.51 s |

The last row is a stress test: every function has two expressions of up
to 31 leaves over four names. The beam keeps 4 states for up to 8 rounds,
and judges at most 30 states for each expression, fewer for a large one.

### Work across statements, and the ranges of locals

Two more passes run in `--release`, after the simplifier.

[src/uc/values.rs](src/uc/values.rs) runs after loops are rotated and
unrolled, so the copies of an unrolled loop are one straight stretch of
code. It does four things:

- **Known low bits.** For every local, it finds how many of its lowest bits
  are known to be zero, from every value ever stored into it. A counter
  that starts at 0 and goes up by 2 is even. If x has t zero bits and
  0 <= c < 2^t, then x + c has no carry, so these hold exactly:
  - `(x + c) >> k = (x >> k) + (c >> k)`
  - `(x + c) ^ d = x ^ (c ^ d)`
  - `(x + c) & d = (x & d) + (c & d)`

  This is how a second copy's `(i + 1) >> 1` becomes the first copy's
  `i >> 1`, as clang finds it.
- **Value numbering.** Every store gives a local a new version, and a
  local's value is known as an expression of the versions it read. A store
  of what the local already holds is left out. A value that another local
  holds is read from that local.
- **Common subexpressions.** An expression that two statements of the same
  straight code work out from the same versions is worked out once, into a
  new local, before the first of them.
- **A waiting operand worked out first.** x64opt works an expression out
  in rax. When both operands of an operation must be worked out, the left
  one waits on the machine's stack, with every value under it. Instead, the
  right operand is worked out first into a new local, unless it reads the
  local the statement stores (that would lengthen a loop's chain).

Only expressions that cannot fail are moved or shared. Float operations
are left where they are. Each rewrite is also checked against the original
on 200 sets of inputs, which keep each local's known zero bits.

[src/uc/ranges.rs](src/uc/ranges.rs) finds every local's interval: the
union of what is ever stored into it. A loop entered only at its top, whose
test is `c < bound` and which stores c once, bounds c where the body reads
it. Bounds that keep moving are widened and then narrowed again. A
`Sext(n)` whose operand's interval fits n bits changes nothing, so it is
left out. Nothing is assumed from undefined behaviour: an operation that
could wrap gives "anything". This generalizes `opt::counters`.

Each pass has its tests:

- unit tests of the rules, checked on every even number, and of each pass;
- an interval test: in 2000 random expressions, every value evaluated
  lies in its interval;
- [tests/uc/values.c](tests/uc/values.c) and `values.go` print what clang
  and Go print, in every mode;
- the random differential test now also writes functions with unrolled
  counters, locals made again, shared expressions and bounded C ints.

**Running times**, against master at a9e5700, which has the simplifier.
Each program ran 11 times, interleaved, and the times are min / median in
seconds:

| | master | **now** | clang -O2 | clang scalar | go |
|---|---|---|---|---|---|
| poly.c | 0.151 / 0.153 | **0.125 / 0.131** | 0.138 / 0.140 | | |
| poly.go | 0.151 / 0.155 | **0.125 / 0.129** | | | 0.223 / 0.228 |
| factor.c | 0.194 / 0.196 | **0.179 / 0.182** | 0.126 / 0.129 | 0.093 / 0.095 | |
| factor.go | 0.191 / 0.196 | **0.178 / 0.181** | | | 0.316 / 0.320 |
| intexpr.c | 0.204 / 0.213 | **0.155 / 0.157** | 0.071 / 0.073 | 0.128 / 0.135 | |
| chain.c | 0.205 / 0.208 | 0.207 / 0.208 | 0.196 / 0.198 | | |
| intchain.c | 0.377 / 0.380 | 0.377 / 0.381 | 0.273 / 0.276 | | |
| index.c | 0.020 / 0.020 | 0.019 / 0.020 | 0.012 / 0.012 | 0.016 / 0.016 | |

poly.c is now **faster than clang -O2: by 10% at the minimum and 6% at
the median**. walk.c was
already faster. The other C benchmarks are still slower than clang:

- factor.c is still slower, even than clang's scalar code. What is left is
  in x64opt, not in the IR. A shift into a register takes `mov rax, x;
  sar rax, k; mov r, rax`, three instructions for two. With seven saved
  registers, the eighth local of factor.c's loop goes to memory.
- intexpr.c and index.c: clang vectorizes them.
- intchain.c and chain.c are dependency chains. clang works the next
  time round's x into y algebraically, and it leaves out intchain's cuts,
  because C's signed overflow is undefined. Those cuts are needed for the
  exact result, and the range analysis keeps them.

Compile time, minimum of the rounds, against master:

| programs | master | now |
|---|---|---|
| suite.c, algebra.c, arrays.c and values.c | 0.018 s | 0.018 s |
| the 100 random test programs | 1.48 s | 1.59 s |
| genbig.py's 2000 functions | 1.63 s | 1.71 s |

### Measured

A whole build of one program of 2000 functions, from source to
executable, 3 runs:

| build | time | Strike against it |
|---|---|---|
| **Strike (C or Go)** | **0.07 s** | |
| clang -O0 | 0.27 s | 3.7 times as fast |
| go build, file changed | 0.86 s | 12 times as fast |
| go build, nothing changed (Go's cache) | 0.12 s | 1.8 times as fast |
| clang -O2 | 17.5 s | about 250 times as fast |

Running, `--release --no-evaluate` against clang -O2, mean of 7 runs. The
C benchmarks use `long long` where they need 64 bits, so that clang
computes the same numbers.

| | Strike | clang -O2 | go | Strike against clang -O2 |
|---|---|---|---|---|
| fib(36) | 0.0310 s | 0.0313 s | 0.059 s | 1% faster |
| 30 million calls | 0.0951 s | 0.0986 s | 0.091 s | 3.5% faster |
| primes below 300 000 | 0.0198 s | 0.0220 s | 0.028 s | 10% faster |
| invariant loops | 0.0275 s | 0.0260 s | 0.028 s | 6% slower |
| a sum loop | 0.0047 s | 0.0053 s | 0.042 s | as fast (both closed it) |

Arrays, fastest and median of interleaved runs (the machine busy with other
builds, so a few percent either way is noise):

| | Strike | clang -O2 | Strike against clang -O2 |
|---|---|---|---|
| 600x600 matrix product (`long long`) | 0.053 s | 0.056 s | 6% faster |
| 300x300 matrix product | 0.0122 s | 0.0128 s | 5% faster |
| sieve of 100 million (`char`) | 0.648 / 0.678 s | 0.617 / 0.690 s | as fast |

clang vectorizes the matrix product with SSE2, but x86 has no 64-bit
vector multiplication, so its three `pmuludq` per two elements lose to
Strike's four copies of one `imul` and one `add` on the memory. The sieve
is bound by memory, and its loops are now the same as clang's.

On these five, Strike's code is as fast as clang -O2's, and some are
faster. The calls benchmark is a C `long` loop. It cuts each result to 32
bits, because C's `long` is 32 bits on Windows. clang does not cut,
because signed overflow is undefined in C. Proving that no overflow can
happen, and so leaving the cut out, would win that back.

Floats: a 5-body simulation, 1 million steps, mean of 7 runs. Both print
the same energies.

| | Strike | clang -O2 | Strike against clang -O2 |
|---|---|---|---|
| n-body | 0.147 s | 0.054 s | 2.7 times as slow |

A float local lives in a general register today, so each use is a move
to an xmm register and back; float locals in xmm registers are being
built.

With evaluation on, `--release` builds the input-free benchmarks into
executables that run in 4-5 ms, which is the process start. The builds
take 0.2 s.

### What is missing

To compile ordinary programs, these are still missing, in the order they
are planned:

1. Command-line arguments, which are strings now there are strings.
2. More of floats: reading floats from input, and more of each language's
   math library.
3. Structs and classes.
4. ~~Several files and modules, and C's preprocessor.~~ Done: see
   above.

After those come garbage collection (for Python's and Go's objects),
`unsigned`, maps, closures, exceptions, the rest of each standard library,
debug information, and other platforms (ELF, Mach-O, ARM). Next to clang,
Go and CPython, Strike covers a small part of each language. Within that
part it is checked against them and gives their answers.

## The daemon

```
strike daemon start [--idle SECONDS]   one per user and Strike home (idle exit: 30 min)
strike daemon stop | status
strike daemon watch <dir> | unwatch <dir>
```

A resident Strike that keeps work hot between commands. `strike compile`
and `strike build` use it when it runs, and work alone when it does not,
or when it does not answer. The executables are byte for byte the same
either way. `STRIKE_DAEMON=off` never uses it; `STRIKE_DAEMON=auto` also
starts one in the background when none runs.

What it keeps was chosen by measuring where a small command's time goes.
Parsing the language descriptions takes 0.4 ms and opening the brand pack
0.07 ms, so they are not kept. Three things are:

- **Readings.** A description's reading of a program (preprocess, lex,
  parse, the files made one, lower) is 65 of the 73 ms of compiling a
  program of 2000 functions. It is kept under the hash of the description
  and the files given, with their contents. A reading also pulls in other
  files (headers, imported modules and packages, go.mod) and looks for
  some that are not there. Each of those looks is kept with it, and the
  reading is used again only while every look finds the same.
- **Evaluations** (`--release`). What Strike's IR runner made of a program
  is kept under the hash of the whole IR and the runner's limits. What a
  baking executable printed is kept under the hash of its bytes. A run
  that did not end in time is not kept.
- **rustc's facts.** `rustc --print cfg` and `rustc -vV` are 85 of the 90 ms
  of a `strike build` where nothing changed. They are kept under the hash
  of everything that decides which rustc answers:
  - every RUST*, CARGO* and PATH variable;
  - every rustc the search could find, by path, size and time;
  - rustup's settings and each toolchain's rustc;
  - the rust-toolchain files from the working folder up.

  The build itself still runs in the client, with the client's own
  environment.

Everything kept is found by content, never by a file's name or time. An
entry is either exactly what the work would give, or it is not found.

`watch` reads a folder's files ahead: every file of a known language whose
size or time changes is read within half a second, so the next compile
finds its reading. Only the folders named are watched. Folders whose names
start with a dot, `target`, `strike-out` and `node_modules` are skipped.

### Safety

- **The channel.** It is a named pipe that refuses remote clients, so
  nothing listens on the network.
- **The token.** Both sides prove they know a token kept in
  `<home>/daemon/state.json`, which only the user can read. The proof is a
  keyed BLAKE3 over a fresh nonce. A pipe someone else made is never
  believed. A client without the token gets no answer and works alone.
- **No impersonation.** The client opens the pipe at identification level,
  so the daemon cannot act as its client.
- **Another binary.** A daemon from another Strike binary is never used.
  The binary is compared by content, and the language descriptions and
  generator are inside it. The CLI says so on stderr and works alone, and
  `start` replaces the old daemon.
- **Rebuilding Strike.** The daemon runs from a copy of the binary
  (`<home>/daemon/bin`), so Strike can be rebuilt while it runs. The
  rebuilt binary is then another binary.
- **The client's surroundings.** Each compile takes the client's working
  folder and STRIKE_ variables, including the generator's rules
  (`STRIKE_CODEGEN`, read on every compile). Compiles therefore run one at
  a time.
- **Ending.** The daemon goes back to its own folder after every compile.
  It ends on `stop`, or after the idle time with no requests. It waits for
  a compile that is running, and removes its state.
- **Crashes.** A crashed daemon, or one that does not answer, never breaks
  a build: the CLI works alone.

### Measured

Mean of 7 runs of the whole command, the brands warm both times:

| | alone | daemon |
|---|---|---|
| `compile` tests/uc/suite.c (6 functions) | 37.7 ms | 31.5 ms |
| `compile` tests/uc/suite.py | 37.5 ms | 30.7 ms |
| `compile` a program of 2000 functions | 126.7 ms | 48.0 ms |
| `compile --release` fib(27), worked out while compiling | 57.6 ms | 32.7 ms |
| the 2000 functions, one line changed, not watched | 105.7 ms | 109.7 ms |
| the 2000 functions, one line changed, folder watched | 105.7 ms | 43.4 ms |
| `build` of a small crate, nothing changed | 117.5 ms | 40.4 ms |

About 30 ms of every command is starting a process, and the daemon cannot
save that. On a small file the daemon gains next to nothing. The time
saved is work that depends on content: reading a big file, evaluating, and
asking rustc.

## fStrike: downloads

```
strike fetch <dir>                          the packages of <dir>/Cargo.lock
strike fetch <url> <file> [--sha256 HASH]   one file
```

Every downloaded file is kept once in **Nimbus** (`~/.strike/nimbus/`),
named by the hash of its content, read-only, and struck into place as a
hard link: the same file under a second name, so nothing is copied.

- A file whose hash is known before (Cargo.lock's checksum, or `--sha256`)
  is struck from Nimbus without the network, and a download with another
  hash is refused.
- A plain URL is remembered with the server's version tag (ETag,
  Last-Modified); the next fetch only asks whether it changed.
- A package is unpacked file by file into Nimbus, so two versions share
  every file they have in common, and struck into
  `~/.strike/registry/<name>-<version>/`, where the build looks first.

Measured on Nuome's 51 crates.io packages: empty Nimbus 3.0 s (6.6 MB);
again 0.00 s; the package folders deleted and struck back from Nimbus,
without the network, 0.5 s. All 2002 files are byte for byte Cargo's own,
and Nuome builds from them with no Cargo downloads at all.

## ArcStrike: big folders to a nearby device

```
strike arc receive [--into DIR] [--port N]     on the device that takes
strike arc devices                             who is on the network
strike arc send <path> --to <device|host:port> [--streams N]
```

AirDrop for big things: games, engine installs, big projects, sent over the
local network as fast as it allows, never sending what the receiver has.

How it works:

1. **Discovery.** A receiver announces its name, OS and port by UDP
   broadcast every second (port 47821); `devices` and `send --to <name>`
   listen for it.
2. **Pairing and encryption.** Every connection is a Noise XX handshake
   (X25519, ChaCha20-Poly1305, BLAKE2s); each device has its own key in
   `~/.strike/arc/identity.key`. A device seen for the first time is shown on
   both sides with the same six-digit code, taken from the handshake, and
   both must accept; the key is then remembered in `peers.json`. A device not
   accepted gets no file and no list of files.
3. **Chunks.** The sender cuts every file into content-defined chunks
   (FastCDC: 64 KiB to 1 MiB, about 256 KiB), each named by its BLAKE3 hash;
   the file is named by its SHA-256, as everything in Nimbus is. A scan is
   remembered by file size and time, so a resend reads only what changed.
4. **Dedup.** The receiver answers the offer with the chunks it lacks. A file
   already in its Nimbus costs nothing. A changed file takes its unchanged
   chunks from the old version on the receiver's own disk (Nimbus keeps a
   chunk index, `nimbus/chunks.idx`, of where each chunk lies), so an
   insertion in the middle of a file sends only the chunks around it.
5. **Streams.** The missing chunks go round several encrypted TCP streams
   (4 by default); the receiver writes each straight to its places in the
   files being made.
6. **Resume.** Every chunk that arrives is logged. A cut transfer keeps its
   files; the same folder sent again asks only for the rest.
7. **Check and strike.** Every chunk is checked by its BLAKE3 on arrival,
   every file by its SHA-256 before it enters Nimbus, then struck into the
   target folder as a hard link (a copy onto another drive) and checked
   once more.

Struck files are read-only, as in all of Nimbus: a program that writes into
its own files (a game patching itself) needs a copy, not a strike.

Measured on one machine over 127.0.0.1 (two Nimbus homes, 4 files of 512 MB
of random bytes): 2147.5 MB in 14.4 s with 4 streams (149 MB/s, more than
gigabit Ethernet carries), 19.9 s with 1 stream; nothing changed 1.3 s, 0 MB
sent; 1 MB overwritten in one file 2.5 s, 1.5 MB sent.

Between two devices (desktop on Ethernet, laptop on the same home network),
the same 2147.5 MB:

| | time | over the wire |
|---|---|---|
| fresh | about 250 s of transfer (8.7 MB/s, the network's rate) | 2147.5 MB |
| nothing changed | 1.15 s | 0 MB |
| 20 MB (about 1%) rewritten in one file | 9.1 s (4 s of it finding the device, since fixed) | 21.3 MB in 65 chunks |

## JVM (Java and Kotlin)

```
strike jvm <export-dir> <out-dir>
```

compiles the Java and Kotlin compile tasks of a JVM build with javac and
kotlinc under the same options, in parallel, each struck or branded. The
tasks come from the build's export, written once by
[tools/strike-export.gradle](tools/strike-export.gradle)
(`gradlew strikeExport -I tools/strike-export.gradle`), as a lockfile is:
sources, classpath, options and outputs. Generated sources are inputs.

Measured on JUnit 5.14.4, the classes of every module, really cold:

| | time |
|---|---|
| Gradle, with downloads (empty Gradle home) | 216.2 s |
| Gradle, without downloads (only Gradle and the dependency jars kept, no daemon, no build cache) | 122.1 s |
| **Strike, empty brand store** | **32.7 s** |
| Strike, second run | 1.7 s |

Gradle's 48 compile tasks hold a little more work: the code generator
(2.1 s) and one more task (1.9 s) that Strike does not run. With those
added Strike still takes about 70% less time. Most of Gradle's time is
Gradle itself: starting, compiling its Kotlin build logic and configuring
23 projects. 4261 of 4270 class files are byte for byte Gradle's; the 9
others are the lambda classes of one Kotlin file, where Gradle's Kotlin
plugin passes an option the export does not show yet.

## Next

1. The universal compiler: input and globals, arrays and strings, floats,
   structs, several files (see What is missing above); then more languages
   as descriptions.
2. More drivers: C/C++ (clang `-MD`, MSVC `/showIncludes`), CUDA
   (`nvcc -MD`), Go, TypeScript; the motor and the lightning do not depend
   on the language.
3. fStrike for more lockfiles: npm, pip, Gradle and Maven, Go.
4. The missing Kotlin option, for all 4270 classes.
5. Function order for the linker (`/ORDER`) from a wave grid like Devil's
   Clockwork's: every order is correct, only speed changes.

MIT. © 2026 Joose Hotari.
