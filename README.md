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

Stage 1 builds Rust packages without dependencies: the library, every
binary, and with `test` the library's tests and every file of `tests/`.
Dependencies, build scripts and proc macros are refused with the reason
(stage 2).

```
strike build <dir> [--for build|run] [--rules FILE] [--explain]
strike test  <dir> [--for build|run] [--rules FILE] [--explain]
strike brand
```

The output goes to `<dir>/strike-out/<profile>/`, the brands to
`~/.strike/brand/` (or `STRIKE_HOME`).

## Next

1. Dependencies from Cargo.lock and the sources already downloaded, with
   features, build scripts and proc macros; checked on Nuome.
2. Drivers for other languages: the motor and the lightning do not depend on
   the language. A driver says how to read the project, what to build in
   which order, the compiler's flags as rules, and where the compiler says
   which files it read (C/C++ through MSVC `/showIncludes`, CUDA through
   `nvcc -MD`).
3. Early cutoff: when a rebuilt library's interface is unchanged, what links
   it is not rebuilt.
4. Function order for the linker (`/ORDER`) from a wave grid like Devil's
   Clockwork's: every order is correct, only speed changes.

MIT. © 2026 Joose Hotari.
