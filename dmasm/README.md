# dmasm

dmasm assembles, disassembles and compiles BYOND's DM bytecode. auxtools uses it to evaluate
expressions in the debugger, to show a proc's instructions, and to hot reload source changes
into a running world.

dmasm has three crates:

- `dmasm-bytecode`, in `bytecode/`, converts a proc's bytecode to named instructions and
  assembles the instructions back into bytecode. It needs no parser.
  [DMASM Specification.md](DMASM%20Specification.md) lists every instruction.
- `dmasm-compiler`, in `compiler/`, compiles procs, proc bodies and expressions that
  SpacemanDMM's `dreammaker` crate has parsed. It emits the instructions that DreamMaker emits
  for a given BYOND build, and it warns about known BYOND bugs. [COMPILER.md](COMPILER.md)
  describes what it supports, how it was compared with DreamMaker, and where it differs.
- `dmasm-reload`, in `reload/`, compares two parses of a codebase. From the differences, it
  works out the procs, variables, types and resources to change in a running world. It also
  finds the changes that must wait for a reboot or a rebuild. It uses the compiler.

The `dmasm` crate re-exports the three crates at the paths they had when dmasm was one crate.
The bytecode types, `assembler` and `disassembler` are at its root, next to the `compiler`,
`reload` and `diagnostic` modules.

Each error, warning, reload refusal and reload note has an ID from `diagnostic::Id`. Check the
ID in code. The message text is for people, and it can change.

Each crate has a `serde` feature, which derives serde's traits for its types. The `serde`
feature of `dmasm` turns on the feature in all three crates.

dmasm supports BYOND 515.1637 to 516.1689. Select the build to compile for with `ByondVersion`.

```sh
cargo test -p dmasm-bytecode -p dmasm-compiler -p dmasm-reload --all-features
```

The tests don't need BYOND. dmasm is licensed under GPL-3.0: see [LICENSE](LICENSE).

## Comparing with DreamMaker

`compiler/tests/dreammaker/` has small worlds that dmasm must compile as DreamMaker does. They
come from reduced fuzzer findings and from the code that the tests compile. The
`compare_dreammaker` example builds each world with the DreamMaker that you give it. Then it
compiles every proc with dmasm for the same BYOND build, and compares the code.

A proc passes if its code is the same as DreamMaker's, is equivalent, or is a known difference.
Equivalent code differs only in its encoding. For example, it writes out a cache load, calls a
proc by name instead of by ID, or places labels elsewhere. Any other proc fails the run.

```sh
cargo run --release -p dmasm-compiler --example compare_dreammaker -- BYOND/bin/DreamMaker
```

CI runs the comparison on Linux with each BYOND build that it tests. A run fails if no case
applies to its build.

A case starts with a `// Guards:` line, which says what the case is for. The case runs to the
next `// Guards:` line. These comments can follow:

- `// Builds: A-B` gives the builds whose DreamMaker accepts the case.
- `// Known [A-B]: PATH  WHY` marks a proc that differs on purpose, and WHY gives the reason. It
  can also mark a bug that is not fixed yet. WHY then says so.
- `// Known [A-B] on glibc 2.41: PATH  WHY` marks a proc that differs only when DreamMaker rounds
  the radians of inverse trigonometric functions to f32. Linux DreamMaker does this with glibc
  2.41 or later. The comparison builds `arctan(2)` first to find out. dmasm must compile and
  assemble the proc and warn about each angle it folds. Only the pushed angles may differ, and
  each of DreamMaker's must be the value that the warning for it gives.

The comparison skips the debug line records and the global init proc.
