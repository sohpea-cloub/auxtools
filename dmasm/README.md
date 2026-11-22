# dmasm

dmasm assembles, disassembles and compiles BYOND's DM bytecode. auxtools uses it to evaluate
expressions in the debugger, to show a proc's instructions, and to hot reload source changes
into a running world.

It has three parts:

- **Bytecode** (`assembler`, `disassembler`): a proc's bytecode as named instructions
  and back. [DMASM Specification.md](DMASM%20Specification.md) lists every instruction.
- **Compiler** (`compiler`): a proc, a proc body or an expression parsed by SpacemanDMM's
  `dreammaker` crate, compiled to the instructions DreamMaker would emit for a given BYOND
  build, with warnings for known BYOND bugs. [COMPILER.md](COMPILER.md) covers what it
  supports, how it was checked against DreamMaker, and where it differs.
- **Reload** (`reload`): two parses of a codebase compared into the procs, variables, types and
  resources to change in a running world, and what has to wait for a reboot or a rebuild.

BYOND 515.1637 to 516.1689 are supported; `ByondVersion` selects the build to compile for.

```sh
cargo test -p dmasm --all-features
```

The tests need no BYOND installation. dmasm is licensed under GPL-3.0; see [LICENSE](LICENSE).

## Comparing with DreamMaker

`tests/dreammaker/` holds small worlds that dmasm must compile as DreamMaker does: the
fuzzer's reduced findings, and the snippets the tests compile. The `compare_dreammaker`
example builds each with a DreamMaker, compiles every proc with dmasm for that build and
fails on a proc whose code is neither DreamMaker's, nor equivalent to it (a cache load written
out, a static call made by name, labels placed elsewhere), nor a known difference:

```sh
cargo run --release -p dmasm --example compare_dreammaker -- BYOND/bin/DreamMaker
```

CI runs it with 516.1689 on Linux; it passes on every build from 515.1637 to 516.1689 that CI
tests. A case starts with a `// Guards:` line saying what it is for, and runs to the next one.
`// Builds: A-B` gives the builds whose DreamMaker accepts it, `// Known [A-B]: PATH  WHY` a
proc that differs by design (or a finding not fixed yet, which says so). Debug line records
and the global init proc are not compared.
