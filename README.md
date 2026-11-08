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
   if the brand came from another language. The same function in C, Go and
   Python is the same IR.
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
(0.058 s) and Go (0.059 s). clang -O2 takes 0.033 s; it also leaves the
saving of registers out of the base case. A release build of big.c takes 0.08 s, with all 2000 calls in
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
