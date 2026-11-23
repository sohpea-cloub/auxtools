# Changelog

## 4.0.0 (not released yet)

### Upgrading from 3.x

- The launcher and the debug library now use protocol 15, so both must be 4.0. A 3.x launcher
  can't attach to a world that runs the 4.0 library. A 4.0 launcher can't attach to a world that
  runs a 3.x library, and `auxtools list` shows such worlds as incompatible. The VS Code
  extension doesn't check the launcher's version. If you set `auxtools.launcherPath` or
  `auxtools.debugLibrary`, point them at 4.0 builds.
- The oldest supported BYOND build is 515.1637. The debugger refuses older builds with a version
  error.
- A hot reload can now be partly applied: some of its steps change the world, and then a later
  step fails. The world then runs a mix of old and new code. The launcher lists the failed steps
  and asks you to restart the world, and `auxtools wait reload` exits with code 1.
- The debugger compiles an expression as the proc body `return EXPR`. Its result now matches
  what the same code computes in the world.

### Removed

- auxcov, the code coverage library.
- The debug console commands `#mem_profiler` and `#guest_override`. The memory profiler hooked
  nothing on supported BYOND builds. The guest ckey override worked only on Windows.
- Support for BYOND builds before 515.1637.
- The `auxtools_check_signatures` export of the `auxtools` library. To check a BYOND build, use
  the `check_recipes` example instead, as the
  [README](README.md#checking-a-new-byond-build) describes.
- The `cdylib` build of the `auxtools` library. A library that uses `auxtools` builds its own
  `cdylib`.
- dmasm's reader of assembly text.

### For users of dmasm

- dmasm is now part of this repository, as the crates `dmasm-bytecode`, `dmasm-compiler` and
  `dmasm-reload`. The `dmasm` crate re-exports them at their old paths.
- Compile errors, warnings and reload refusals are now `diagnostic::Diagnostic` values instead
  of strings. In code, check their `diagnostic::Id`, because their text can change.
