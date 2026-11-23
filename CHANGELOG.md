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
- The debugger compiles an expression as the proc body `return EXPR`, on the same path as a
  proc. The code still runs in the debugger's evaluation stub, so some constructs work
  differently there. For example, the debugger refuses `__PROC__`.
  [dmasm's COMPILER.md](dmasm/COMPILER.md#debugger-code) lists the differences.

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

- `Proc::set_bytecode` returns a `Result`. It gives the proc a code entry of its own first, unless
  the proc already has one, so other procs and tables that share the entry keep their code. It
  fails and changes nothing when the proc can't get an entry of its own, as for a proc of 65,535
  words.
- `own_code` gives a proc a code entry of its own and keeps its code at the same address, so its
  running frames see a patch to the code. It is `unsafe`: no other library may have replaced the
  proc's code. `copy_code` gives a proc an entry of its own with a copy of its code at a new
  address.
- `debug::set_local_names` and `debug::has_local_names` take a `&Proc` first. Pass the proc
  whose local names you record or check. Procs that share code can have different local names.
  `set_local_names` also records the proc's parameter names, and `debug::parameter_names_for`
  gives them back.
- A hook binds its parameters by position, and `_` skips an argument. A parameter declared as
  `Value` gets a `Value`, and one declared as `&Value` gets a reference. Other parameter types
  and patterns are a compile error. A hook can name a parameter `args`.
- Weak reference ids are now text, with a namespace for each library, so they stay exact past
  2^24 and two libraries never issue the same id. A datum that an older version tagged gets a
  new id, and the older version's handles to it no longer upgrade.
- `Value::call` no longer leaks references to its arguments when the proc name holds a NUL.
- A `NameMapPlan` that is dropped without being written frees all of its memory.
- `instruction_hooking::InstructionHookToAny` is gone. `InstructionHook` no longer requires it.

### For users of dmasm

- dmasm is now part of this repository, as the crates `dmasm-bytecode`, `dmasm-compiler` and
  `dmasm-reload`. The `dmasm` crate re-exports them at their old paths.
- Compile errors, warnings and reload refusals are now `diagnostic::Diagnostic` values instead
  of strings. In code, check their `diagnostic::Id`, because their text can change.
