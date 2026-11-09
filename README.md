# Strike

**The King Killer.**

A build system that calls the compiler itself. Two parts decide every build:

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
strike compile <file> [-o EXE] [--explain] [--no-brand] [--plain] [--release]
```

Strike's own compiler. It does not know any language itself. Every
language is a description in [languages/](languages/):

- how its text is cut into tokens (comments, strings, a block by braces or
  by indentation, where a line ends a statement);
- its operators, with their precedence;
- its forms, written as patterns. For example, `if ( $expr ) $body [ else
  $body ]` is C's if, and `for $name in range ( $args ) : $block` is
  Python's.

Each form says which universal construct it is. A new language is a new
file, not a new compiler. There are three today: C, Go and Python.

How a file becomes an executable:

1. **HotMotor picks the language.** Every description is a candidate, and
   the judges in [languages/judges.toml](languages/judges.toml) score each
   one:
   - it reads the whole file and knows every name in it;
   - the file's extension is one the description names;
   - how far into the file it read.

   A description that could not beat the best score so far is not read at
   all. A file with the wrong extension is still read in its own language.
2. **Strike IR.** The file is lowered into one form shared by every
   language: functions of stack operations on 64-bit integers.
   - Truth values are followed through variables, parameters and results,
     because Python prints `True` and Go prints `true` where C prints `1`.
   - Each division keeps its language's meaning. C and Go round toward
     zero, Python rounds down.
3. **Brands per function.** Each function is branded by its IR. A
   function whose brand already exists is struck instead of compiled, even
   if the brand came from another language. The same work in two languages
   is the same IR: a C `long long` function and a Go `int` one share their
   brand. A C `int` one does not, since it wraps at 32 bits, and neither
   does a Python one, which does not wrap at all.
4. **Machine code by Strike.** x86-64 is written byte by byte in one pass,
   with no LLVM and no assembler.
5. **The executable by Strike.** The PE file, its import table (KERNEL32)
   and the runtime (printing and the entry point) are written directly,
   with no linker.

Whatever Strike cannot do yet in the language's own way is refused with
the line and the reason, never guessed. Examples are a chained comparison
in Python, `/` on floats, and a function whose value is used but which can
end without one. Stage one has integers and truth values, functions,
if/else, while, for, Python's `range`, break and continue, and printing
(including printf formats). It has no floats, strings as values, arrays,
structs or libraries yet.

### Integers as each language has them

Two places gave a wrong number with no warning. Both are fixed:

- **Python's integers have no upper bound; Strike's are 64 bits.** 25!
  came out as 7034535277573963776. Now `+`, `-`, `*`, `<<` and unary `-`
  in Python are checked operations (`overflow = "error"` in
  python.toml). Where a number would leave 64 bits, the program stops with
  exit code 1 and says so on standard error. It never goes on with a wrong
  number. Folding, working out while compiling, LICM and the closed form
  of sums all leave a checked operation alone where it could stop the
  program. Before this, the closed form would have wrapped a Python sum
  without a word.
- **C's `int` was 64 bits in Strike; clang's is 32.** On Windows clang's
  `long` is 32 bits too. Now each type has its width (`[int_widths]` in
  c.toml and go.toml). Arithmetic is done at least as wide as `int`, and a
  result is cut back to the width of where it goes (`Sext` in the IR),
  as C converts it. Go's `int32` wraps at 32 bits as Go defines. A Go
  literal takes the width of what it meets, and one that does not fit is
  an error, as in Go. `unsigned`, Go's `uint` and pointers are refused,
  not guessed.

[tests/uc/widths.c](tests/uc/widths.c) prints what clang -O0 prints in
every mode: int, long, short and char wrapping, a parameter's conversion,
and `-INT_MIN`. [tests/uc/widths.go](tests/uc/widths.go) prints what Go
prints. A Python test checks that 20! prints and 25! stops the program,
in every mode.

The cost: a C `long` loop now cuts its results to 32 bits, as it must.
Clang -O2 does not cut them, because signed overflow is undefined in C
and it works in 32-bit registers. On 30 million `(a * 31 + b) % 1000003`
Strike went from 0.085 s to 0.095 s, still 3.5% faster than clang -O2.
Proving that no overflow can happen, and so leaving the cut out, is next.

[tests/uc/](tests/uc/) holds the same suite in all three languages.
Strike's executables print byte for byte what clang, Go and Python print
for them (the `.out` files were written by those toolchains).

Measured on one program of 2000 functions (each a loop with a branch),
written in each language, 3 runs, mean:

| compile to an executable | time |
|---|---|
| **Strike, C** | **0.12 s** (0.13 s all struck, 0.23 s with empty brands) |
| **Strike, Go** | **0.12 s** |
| **Strike, Python** | **0.17 s** |
| clang -O0 | 0.27 s |
| go build (standard library already built) | 0.86 s |
| clang -O2 | 17.4 s |

Strike compiles 2.3 times as fast as clang -O0 and 7 times as fast as
`go build`. Every executable gives the same answer as clang's, Go's or
Python's own run.

### The optimizing generator

[src/uc/x64opt.rs](src/uc/x64opt.rs) is the default. `--plain` selects the
first generator, which is kept as the reference.

- **Values stay where they are.** The IR's stack is followed at compile
  time: a constant, a local or a result in rax stays put until an operation
  needs it. A comparison followed by a jump becomes one `cmp` and one
  `jcc`. Only what must cross a call, a jump or a label is pushed.
- **HotMotor chooses which locals live in registers.** The rule is in
  [codegen.toml](codegen.toml). Every local offers two variants: one of the
  seven callee-saved registers, or memory. The judge scores the memory
  operations a register saves, with a use inside a loop weighted 8 times
  per level, against the cost of saving the register. The beam keeps the
  best choices.
- **Brands live in one pack**, `~/.strike/uc/pack`. It is read whole once
  per compile, and new brands are added at its end. The brand covers the
  IR and the generator with its rules, so other rules give other code.

- **Calls take their arguments straight in registers.** Each argument goes
  to rcx, rdx, r8, r9 or its stack slot from wherever it is. Nothing is
  pushed except a result in rax that must outlive the call.
- **HotMotor chooses which calls are inlined** ([src/uc/inline.rs](src/uc/inline.rs)).
  The rule is the `[inline]` part of codegen.toml, and it applies to every
  call of a function that calls nothing itself. There are two variants:
  the call, or the callee's IR copied in its place. The judge scores the
  cost of the call saved, weighted by the loops around the call, against
  the operations copied. This is done before branding, so a caller's brand
  covers what was copied into it.

Every optimized program is checked against the plain one, which is not
inlined either. One test writes 100 random C programs, with up to six
parameters, loops, every operator and calls between functions; HotMotor
inlines calls in most of them. Both must give the same output and the same
exit code. The suites also still print what clang, Go and Python print.

Running, the second and third runs:

| | fib(36) | 30 million calls in a loop | primes below 300 000 |
|---|---|---|---|
| **Strike, optimized** | **0.079 s** | **0.086 s** | **0.022 s** |
| Strike, plain | 0.125 s | 0.133 s | 0.043 s |
| clang -O0 | 0.058 s | 0.126 s | 0.021 s |
| go | 0.058 s | 0.091 s | 0.028 s |
| clang -O2 | 0.031 s | 0.100 s | 0.021 s |
| python | 1.3 s | | |

- **A small function called in a loop:** Strike's code is the fastest of
  all, because HotMotor inlined the call.
- **Loops:** it is as fast as clang -O0 and faster than Go.
- **Recursion (fib, which cannot be inlined):** it is 1.35 times as slow
  as clang -O0, against 1.6 times before direct calls. The first call's
  result still crosses the second call on the stack.

### Release builds

`--release` uses the same generator, plus the work a release build may
spend more time on. The rules are in the `[release]` part of codegen.toml:

- constant expressions are worked out while compiling
  ([src/uc/fold.rs](src/uc/fold.rs)), by the machine's own rules
  (wrapping, a shift's count modulo 64; never a division by zero);
- calls are copied in more readily (callees of up to 200 operations).

Every function also reserves its call area once, so a call with nothing
pushed needs no stack adjustment. A frame larger than a page is touched
page by page as Windows requires: before that, a function with very many
locals crashed.

Division by a constant as a multiplication by its magic number is written
and tested. A unit test checks the sequence against real division, and
the random-program test runs it on 100 programs. It is still off in
codegen.toml, because on this machine it was slower: 0.095 s against
0.085 s with `idiv`, on 30 million dependent `% 1000003`. Clang -O2's own
multiplication took 0.100 s on the same loop.

Measured, release builds are as fast as the default here and no faster:

| | fib(36) | 30 million calls | primes |
|---|---|---|---|
| Strike --release | 0.082 s | 0.085 s | 0.023 s |
| Strike | 0.083 s | 0.084 s | 0.022 s |
| clang -O2 | 0.034 s | 0.101 s | 0.020 s |

### Calls of a function by itself

[src/uc/tail.rs](src/uc/tail.rs) turns a function's call of itself at a
return into a jump to its start, in every mode but `--plain`:

- `return f(args)`: the arguments go into the parameters, then back to the
  start.
- `return x + f(args)` and `return x * f(args)`: x goes into an
  accumulator, then the same. Every other return gives the accumulator
  combined with its own value.

Wrapping addition and multiplication are associative and commutative, so
the result is the same number. x and the arguments are worked out in the
same order as before. `fib(n-1) + fib(n-2)` makes half as many calls; the
function becomes a loop around one call.

The random-program test now also writes four recursive functions per
program, in every shape a return can call itself in, including shapes that
must not be changed (`f(n-1) - x`, `x + f(n-1) * 3`). In the 100 programs,
266 calls became jumps, and all printed what the plain code prints.

fib(36): 0.082 s became **0.043 s**. That is faster than clang -O0
(0.058 s) and Go (0.059 s).

Two more changes went in for calls:

- **A frameless fast path.** A function that begins by testing its
  parameters and returning at once, like fib's `if (n < 2) return n`, gets
  that test and that return first, worked out from the argument registers.
  No frame is made and no register is saved: fib's base case is four
  instructions. Only operations that cannot touch an argument register are
  taken: no call, no division, constants that fit 32 bits. In the 100
  random programs, 208 functions in 91 programs got a fast path, and all
  printed what the plain code prints.
- **Results stored straight into a register.** An operation whose result
  goes at once into a local held in a register is worked out in that
  register: `n = n - 2` is one `sub`, and `acc = x + acc` is one `add`. The
  call's result can then stay in rax.

These took fib(36) only from 0.043 s to 0.041 s.

- **Recursion unrolled.** HotMotor's `[unroll]` rule in codegen.toml
  copies a function's own body in place of each call of itself, `levels`
  times, while the function is no longer than `max_ops` operations. The
  calls inside the copy stay calls, so each call that is left does the
  work of two levels. This is done before branding, with the same copying
  as inlining. In the 100 random programs, 213 recursive calls were
  unrolled, and all printed what the plain code prints.

fib(36), the last four of five runs:

| unrolled levels | time | exe |
|---|---|---|
| 0 | 0.040-0.042 s | 1536 bytes |
| **1 (the rule)** | **0.034-0.037 s** | 2048 bytes |
| 2 | 0.031-0.039 s | 2048 bytes |
| 3 | 0.035-0.039 s | 3072 bytes |
| clang -O2 | 0.031-0.033 s | |

One level takes fib from 0.041 s to 0.035 s, within 10% of clang -O2.
More levels gave no gain that stands out from the noise, so the rule
stays at one.

What was still slow in that code, and is now fixed:

1. **The frame.** Saving four registers to memory under an rbp frame cost
   about 13 instructions per call. The frame now has no rbp: the saved
   registers are pushed and popped. A local in memory is addressed from
   rsp, because the generator knows at every operation how much has been
   pushed since.
2. **An unrolled copy's value went through the stack** (`push rax; jmp;
   pop rax`). A value that crosses a jump now stays in rax. Every label
   records what it expects (how much is pushed, and whether the top is in
   rax), and every jump to it delivers that. A jump to the very next
   instruction is left out.
3. **The test ran twice.** After the fast path's test has failed, the full
   function stores its constants and jumps straight to the rest of the
   function.
4. **Zeros were stored twice.** A store of zero into a slot already known
   to be zero is left out.
5. **`n - 1` took three instructions.** A local in a register plus or
   minus a constant now waits as a lazy value and becomes one `lea` into
   wherever it goes (`lea rcx, [rdi-1]`).

Two bugs turned up while fixing these, and both are fixed:

- **A loop that never ended.** Code that nothing falls into is not
  emitted. A loop's top looked like such code where only later jumps came
  back to it, so it was dropped.
- **A release build stopped with a panic.** Folding constants had left
  jumps in code that is never reached. Those jumps were counted, and a
  label was given the wrong depth.

The random-program test found the second one. Both are now settled by one
pass: it follows every way through the IR from the start, and records
which operations are reached and the depth of the stack at each.

The last four of five runs:

| | fib(36) | 30 million calls | primes |
|---|---|---|---|
| **Strike** | **0.029-0.030 s** | **0.083-0.086 s** | 0.022-0.023 s |
| clang -O2 | 0.031-0.033 s | 0.101-0.102 s | 0.020 s |
| clang -O0 | 0.057-0.058 s | 0.125-0.126 s | 0.021-0.022 s |
| go | 0.057-0.058 s | 0.091-0.092 s | 0.026-0.027 s |

On calls, Strike's code is now as fast as clang -O2 or faster.

### Division of small numbers

The primes loop divides by a variable, so the magic multiplication does not
apply. clang -O2 was 11% faster on it, because C's `long` is 32 bits on
Windows and clang uses the 32-bit division. Strike's numbers are 64 bits,
and the 64-bit division is the slower instruction.

Each division now first checks its two numbers. If both are at least zero
and below 2^32, the 32-bit unsigned division gives the same quotient and
remainder, so Strike uses it. Otherwise it uses the 64-bit one, so no
64-bit number loses anything. LLVM does the same. The rule is `[division]
bypass_32` in codegen.toml. A comparison with zero is also a `test` now,
which sets the same flags as `cmp`.

Primes below 300 000, mean of 7 runs:

| | time | Strike against it |
|---|---|---|
| **Strike** | **0.0208 s** (0.0227 s before: 8% faster) | |
| clang -O2 | 0.0205 s | 1.5% slower (min 0.0195 s against 0.0197 s) |
| clang -O0 | 0.0208 s | the same |
| go | 0.0283 s | 26% faster |

## What clang -O2 does, in --release

On top of the default generator, `--release` makes these optimizations,
each of which clang -O2 also makes:

- **Algebra** (clang's instcombine; [src/uc/opt.rs](src/uc/opt.rs)).
  `x + 0`, `x * 1` and `x | 0` become `x`, and `x * 2^k` becomes `x << k`.
  Python's `x // 2^k` becomes `x >> k`, and `x % 2^k` becomes
  `x & (2^k - 1)`.
- **Loop-invariant code motion** (LICM). An expression in a loop whose
  locals the loop never stores is worked out once, before the loop, into a
  new slot. Only expressions that cannot fail are moved (no division by a
  variable), since a loop may not run at all. Only loops entered by falling
  into their top are touched.
- **Division by a constant as a multiplication by its magic number**,
  where HotMotor's judge takes it (`magic_where` in x64opt.rs). A division
  whose value comes back round its loop into what it divides waits on
  itself, and there the divider is faster. Any other division gets the
  multiplication, whose instructions overlap from one time round to the
  next.
- **Constant folding, readier inlining, and working out a program that
  reads nothing** (above). Clang does the same. On pow2 its executable is
  one constant and one printf.

The random-program test now has a fifth mode, `--release --no-evaluate`:
the release optimizations as machine code that runs. In the 100 programs,
128 operations were simplified and 1053 expressions were moved out of
loops. All printed what the plain code prints.

Measured with `--no-evaluate`, so the generated code runs. Every C file
uses 64-bit `long long`, since on Windows clang's `long` is 32 bits and
would compute something else. Mean of 5 runs:

| | Strike --release | clang -O2 | Strike against clang -O2 |
|---|---|---|---|
| fib(36) | 0.0300 s | 0.0335 s | 10% faster |
| 30 million calls | 0.0857 s | 0.0995 s | 14% faster |
| primes below 300 000 | 0.0204 s | 0.0208 s | 2% faster |
| invariant loops (`s += (a*b + i*7 ^ j) % 1000`) | 0.0310 s (0.040 s before) | 0.0275 s | 13% slower |
| pow2 (`s += i*8 + (i*4+1)*1 + 0`) | 0.0340 s | 0.0050 s | clang worked the loop out while compiling |

On pow2, clang found the loop's sum while compiling, from the loop's
recurrences (SCEV). Two more release passes followed:

- **A sum's closed form** ([src/uc/scev.rs](src/uc/scev.rs)), as clang's
  SCEV does. Take a loop `for (c = a; c < N; c++) s = s + P(c)`, where P
  is a polynomial of degree up to 3. The stored expression is read as a
  polynomial in s and c, and must be s plus P(c). P(a + k) is written in
  the binomial basis, and sum_{k<T} C(k, m) = C(T, m+1), so the loop
  becomes sum b_m C(T, m+1).
  - The arithmetic is modulo 2^64, as the loop's own wrapping arithmetic
    would be. The binomials are exact, because m! is divided out of the
    factors before they are multiplied.
  - The loop may add to several locals.
  - A local copying the counter (Python's `for i in range(...)`) gets the
    counter's last value.
  - If N is known while compiling, any degree up to 3 works. If N is in a
    local, degree up to 1 works, and the result is worked out when the
    program runs.
  - Unit tests check the binomials against exact numbers, and the closed
    form against the loop for 300 random polynomials, with overflow. The
    random-program test also writes three sum loops per program, in every
    shape plus one that must stay a loop (`s = s * 3 + ...`). 438 loops
    were replaced, and all printed what the plain code prints.
- **Loop rotation and unrolling** ([src/uc/rotate.rs](src/uc/rotate.rs)).
  The test moves to the loop's bottom, so there is one jump less each time
  round. A short body with no jump in it goes round twice per jump back.
- **A remainder by a magic number is three instructions**
  (`imul; neg; add`), not eight.

Against clang -O2 again, the mean of 7 runs, with `--no-evaluate`:

| | Strike --release | clang -O2 | go | Strike against clang -O2 |
|---|---|---|---|---|
| pow2 | 0.0044 s | 0.0051 s | 0.0415 s | as fast (both worked the loop out) |
| invariant loops | 0.0271 s (0.031 s before) | 0.0275 s | 0.0283 s | 1% faster (13% slower before) |
| primes | 0.0206 s | 0.0220 s | 0.0283 s | 6% faster |
| 30 million calls | 0.0845 s | 0.0994 s | 0.0911 s | 15% faster |
| fib(36) | 0.0293 s | 0.0345 s | 0.0591 s | 15% faster |

On these five, Strike's release build is now as fast as clang -O2 or
faster. What clang -O2 still has and Strike does not is SIMD
vectorisation, and everything for the parts of the languages Strike does
not have yet: arrays, strings, floats, memory.

## A program that reads nothing is worked out while compiling (--release)

A program with no input has a fixed result: what it prints and what it
returns. A release build works the program out while compiling, and the
executable only prints that output and returns that number
([src/uc/eval.rs](src/uc/eval.rs); limits in `[release.evaluate]` of
codegen.toml):

1. Strike's own IR runner tries first, for up to 20 million steps. It
   follows the machine's rules: wrapping arithmetic, a shift's count modulo
   64, and each language's division.
2. If that is not enough, the program is compiled and run once, for up to
   2 s. This run prints a marker and then the number main returned, so a
   crash can never be taken for a result. It is safe, because these
   languages can only print.
3. If the program does not end in time, prints more than 1 MB, recurses
   too deep, or fails (a division by zero), it is compiled as it is, and it
   fails when it runs, as it would.

Nothing is recognised or guessed. It is the program's own work, done once
before it ships. Golden Answer would answer "how many primes are below
300 000" as fast, but it answers a question; this runs the code, whatever
it is.

| --release | compile | run |
|---|---|---|
| primes below 300 000 | 0.24 s | 0.005 s (0.021 s before) |
| 30 million calls | 0.23 s | 0.004 s (0.085 s before) |
| fib(36) | 0.21 s | 0.004 s (0.030 s before) |
| big.c, the suites | 0.03-0.07 s (IR runner) | 0.004 s |

The 4 ms left is starting the process. The run-time tables above measure
the code generator, so they are built without `--release`. A benchmark of
the generated code must take its numbers from outside, or a release build
works it out.

Tests check four things:

- a release build of each suite prints what clang, Go and Python print,
  by both ways;
- the number main returns stays the exit code;
- a program that divides by zero is not worked out and fails as the plain
  build fails;
- the random-program test runs division by multiplying with nothing
  worked out, so that machine code is still tested. A release build of big.c takes 0.08 s, with all 2000 calls in
main inlined.

Compiling big.c (2000 functions) with the optimizing generator takes
0.07 s, 3 runs. The code part takes 0.004 s when every function is struck
from the pack, against 0.007-0.010 s when compiled. With the pack, a brand
is now faster to strike than to compile. Most of the remaining time
(0.04 s) is reading the file.

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

1. The universal compiler: arguments in registers; floats, strings and
   arrays; more languages as descriptions.
2. More drivers: C/C++ (clang `-MD`, MSVC `/showIncludes`), CUDA
   (`nvcc -MD`), Go, TypeScript; the motor and the lightning do not depend
   on the language.
3. fStrike for more lockfiles: npm, pip, Gradle and Maven, Go.
4. The missing Kotlin option, for all 4270 classes.
5. Function order for the linker (`/ORDER`) from a wave grid like Devil's
   Clockwork's: every order is correct, only speed changes.

MIT. © 2026 Joose Hotari.
