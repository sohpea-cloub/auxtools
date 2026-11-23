# Changelog

## 4.0.0 (not released yet)

### Upgrading from 3.x

- The launcher and the debug library use protocol 15. They must both be 4.0. A 3.x launcher
  can't attach to a world that runs the 4.0 library, and a 4.0 launcher can't attach to a
  world that runs a 3.x library. A 4.0 `auxtools list` shows 3.x worlds as incompatible. The VS
  Code extension doesn't check the launcher's version. If you set `auxtools.launcherPath` or
  `auxtools.debugLibrary`, point them at 4.0 builds.
- BYOND 515.1637 is the oldest supported build. The debugger refuses older builds with a
  version error.
- A hot reload can now be partly applied. Some of its steps changed the world and then a
  later step failed, so the world runs a mix of old and new code. The launcher lists the
  failed steps and asks you to restart the world. `auxtools wait reload` exits with code 1.
- The debugger compiles an expression as the proc body `return EXPR`. Its result now matches
  what the same code computes in the world.

### Removed

- auxcov, the code coverage library.
- The debug console commands `#mem_profiler` and `#guest_override`. The memory profiler
  hooked nothing on supported BYOND builds. The guest ckey override worked only on Windows.
- Support for BYOND builds before 515.1637.
- The `auxtools_check_signatures` export of the `auxtools` library. The `check_recipes`
  example checks a BYOND build instead: see the README.
- The `cdylib` build of the `auxtools` library. A library that uses it builds its own.
- dmasm's reader of assembly text.

### For users of dmasm

- dmasm is now part of this repository, as the crates `dmasm-bytecode`, `dmasm-compiler` and
  `dmasm-reload`. The `dmasm` crate re-exports them at their old paths.
- Compile errors, warnings and reload refusals are `diagnostic::Diagnostic` values instead of
  strings. Decide by their `diagnostic::Id`. Their text can change.
