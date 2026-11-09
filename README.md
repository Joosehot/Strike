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
strike compile <file> [-o EXE] [--explain] [--no-brand] [--plain] [--release [--no-evaluate]]
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

Integers and truth values, functions and recursion, if/else, while, for,
Python's `range`, break and continue, printing (print, Println, puts,
and printf with `%d`, `%s` and `%v`), global variables, and whole numbers
read from standard input.

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

Whatever Strike cannot do yet in the language's own way is refused, with
the line and the reason, and never guessed. Refused today:

- strings as values, arrays, floats, structs, pointers, `unsigned` and
  `uint`;
- a chained comparison in Python;
- a function whose value is used but which can end without one;
- a Python variable that changes between a number and a truth value.

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
- 41 tests in all, in about 25 s.

The tests have found real bugs, all fixed:

- a loop whose top was dropped as unreachable, so it never ended;
- a panic in release builds, from jumps left in code that folding made
  unreachable;
- a crash on frames larger than a page;
- Python's 25! and C's `int` overflow coming out wrong with no warning.

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
  twice per jump back.
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

On these five, Strike's code is as fast as clang -O2's, and some are
faster. The calls benchmark is a C `long` loop. It cuts each result to 32
bits, because C's `long` is 32 bits on Windows. clang does not cut,
because signed overflow is undefined in C. Proving that no overflow can
happen, and so leaving the cut out, would win that back.

With evaluation on, `--release` builds the input-free benchmarks into
executables that run in 4-5 ms, which is the process start. The builds
take 0.2 s.

### What is missing

To compile ordinary programs, these are still missing, in the order they
are planned:

1. Arrays and strings, with a heap and an allocator under them (then
   command-line arguments, which are strings).
2. Floats.
3. Structs and classes.
4. Several files and modules, and C's preprocessor. Today a `#` line is
   skipped. It must become a preprocessor, or be refused.

After those come garbage collection (for Python's and Go's objects),
`unsigned`, maps, closures, exceptions, the rest of each standard library,
debug information, and other platforms (ELF, Mach-O, ARM). Next to clang,
Go and CPython, Strike covers a small part of each language. Within that
part it is checked against them and gives their answers.

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
