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
