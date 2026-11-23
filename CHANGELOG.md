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
  and `auxtools wait reload` exits with code 1. Rebuild the `.dmb`, then restart the world to
  load all of the new code.
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

### For users of the auxtools library

- `Value::raw` is now a method that returns a copy. A `Value` releases its reference when it
  drops, so replacing its raw value wasn't safe.
- The functions in `runtime_tables`, `appearance_tables` and `raw_types::misc`,
  `Proc::set_bytecode`, `remember_bytecode` and `bytecode_at` are now `unsafe`. Call them on
  BYOND's thread while a world is live. Their `# Safety` sections say what else they need.
  `NameMapPlan::write` returns a `Result`.
- `Proc::set_bytecode` returns a `Result`. It gives the proc a code entry of its own first, unless
  the proc already has one, so other procs and tables that share the entry keep their code. It
  fails and changes nothing when the proc can't get an entry of its own, as for a proc of 65,535
  words.
- `WeakValue::upgrade` returns `None` for a handle made before a shutdown.
- Weak reference ids now include a namespace for each library. A datum that an older version
  tagged gets a new id, and the older version's handles to it no longer upgrade.
- A hook can name a parameter `args`.

### For users of dmasm

- dmasm is now part of this repository, as the crates `dmasm-bytecode`, `dmasm-compiler` and
  `dmasm-reload`. The `dmasm` crate re-exports them at their old paths.
- Compile errors, warnings and reload refusals are now `diagnostic::Diagnostic` values instead
  of strings. In code, check their `diagnostic::Id`, because their text can change.
