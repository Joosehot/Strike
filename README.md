# Strike

A build system that calls the compiler itself. Two parts decide every build:

- **HotMotor** chooses how to build. One rule per decision (opt-level,
  codegen units, incremental, debuginfo), each with variants scored on three
  axes (build time, program speed, disk). A beam search keeps the best
  partial builds, and judges score the choices together. Every number is in
  [rules.toml](rules.toml).
- **Lightning** remembers what was built. Every build is branded once, under
  the hash of everything that decides its output: the compiler, the flags,
  the *content* of every file the compiler read (from its own dep-info), the
  environment variables it read, and the brands it links. Before a compiler
  call, a strike looks for that brand. A file's new time is not a change, but
  new content is. The lightning is also one of the motor's judges: a build
  already branded costs no build time, so it scores as much as the profile
  asks for build time.

Strike builds Rust packages with their dependencies. The versions come from
Cargo.lock and the sources from the registry folder Cargo already
downloaded (Strike does not resolve or fetch). Features are unified over
the graph. Build scripts are built and run, proc macros are built as
libraries the compiler loads, and every job of the graph runs as soon as
what it uses is done, on every core. Brands are shared by every project: a
dependency with the same version and features is built once for all of them.

Measured on Nuome (53 locked packages, 66 jobs):

| | time |
|---|---|
| first build, empty brand store | 42 s |
| nothing changed (or only a file's time) | 0.7 s |
| one line changed in the library | 8-12 s (cargo: 9-12 s) |
| `strike test`: 341 tests in 3 harnesses, all pass | 101 s |

Strike built itself next: 9 of its 44 jobs were struck from Nuome's brands.

```
strike build <dir> [--for build|run] [--rules FILE] [--explain]
strike test  <dir> [--for build|run] [--rules FILE] [--explain]
strike brand
```

The output goes to `<dir>/strike-out/<profile>/`, the brands to
`~/.strike/brand/` (or `STRIKE_HOME`).

## Next

1. Drivers for other languages: the motor and the lightning do not depend on
   the language. A driver says how to read the project, what to build in
   which order, the compiler's flags as rules, and where the compiler says
   which files it read (C/C++ through MSVC `/showIncludes`, CUDA through
   `nvcc -MD`).
3. Early cutoff: when a rebuilt library's interface is unchanged, what links
   it is not rebuilt.
4. Function order for the linker (`/ORDER`) from a wave grid like Devil's
   Clockwork's: every order is correct, only speed changes.

MIT. © 2026 Joose Hotari.
