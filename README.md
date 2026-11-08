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
sent; 1 MB overwritten in one file 2.5 s, 1.5 MB sent. Between two devices:
see below when measured.

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

1. More drivers: C/C++ (clang `-MD`, MSVC `/showIncludes`), CUDA
   (`nvcc -MD`), Go, TypeScript; the motor and the lightning do not depend
   on the language.
2. fStrike for more lockfiles: npm, pip, Gradle and Maven, Go.
3. The missing Kotlin option, for all 4270 classes.
4. Function order for the linker (`/ORDER`) from a wave grid like Devil's
   Clockwork's: every order is correct, only speed changes.

MIT. © 2026 Joose Hotari.
