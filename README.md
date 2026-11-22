# auxtools

auxtools debugs DM worlds from [VS Code](editors/vscode) or the command line, with experimental hot reload.

The `auxtools` crate, which the debugger is built on, lets other Rust libraries hook DM procs and work with BYOND values.

## The debugger

The debugger supports line, function, instruction and data breakpoints; conditional breakpoints and logpoints; runtime error breakpoints; line and instruction stepping; sleeping proc stacks; variable inspection and assignment; expression and proc-body evaluation; and disassembly. A journal records runtime errors, reloads, output and reboots.

No changes to the world's source are required. `auxtools` launches a compiled `.dmb` in DreamDaemon (`dd.exe` on Windows), windowed DreamDaemon or DreamSeeker with the debug library loaded. Build the world with your project's usual tools. After `world.Reboot()`, the launcher reconnects and restores breakpoints and reloads.

Multiple debuggers can connect to the same world. Each session has independent breakpoints and runtime error settings. Pausing stops the world for every session; any session can resume it.

Use `auxtools list` to find running worlds. Worlds using an incompatible debugger protocol are listed but cannot be attached to.

For VS Code, install the extension and press F5 in a workspace containing a built world. See the [extension README](editors/vscode/README.md) for configuration and debugging features.

For command-line use, run `auxtools help COMMAND` for options, behavior, output and exit codes. `--pid PID` selects a world when several are running. Requests time out after 30 seconds by default, with exit code 2. Use `--response-timeout SECONDS` to change the limit. Results go to stdout and diagnostics to stderr. `--json` returns a JSON value, or JSON Lines for `log` and `trace`.

```sh
auxtools launch tgstation.dmb --detach
auxtools wait pause --break code/game/foo.dm:120
auxtools vars
auxtools eval 'src.health'
auxtools step-over
auxtools continue
auxtools trace /mob/proc/Life --by caller --for 5
auxtools profile --for 10
auxtools log --runtimes --follow
auxtools wait reload
auxtools reboot
auxtools stop
```

Exit codes are 0 for success, 1 for failure, 2 for timeout, 3 if the world exits or reboots before completion, 4 if a world launched in the foreground exits unsuccessfully, and 5 if `check` finds known BYOND bugs.

Editors use `auxtools dap` as the debug adapter.

## Instruction profiling

`auxtools profile` reports execution time by proc, source line and instruction.

```sh
auxtools profile --for 10                       # slowest procs and lines
auxtools profile /mob/living/proc/Life --for 10  # instructions in one proc
auxtools profile --for 10 --html profile.html   # interactive report
```

Without `--for`, profiling continues until Ctrl-C. Name procs to measure only those.
Use `--json` for structured output. In VS Code, run **auxtools: Profile instructions** to
start, then again to stop and open the report. Calls from `auxtools eval` and the
Debug Console are included.

Self time includes an instruction's work until the next instruction starts, including
BYOND operations such as copying lists or freeing locals. For calls, self time covers
entering the proc; execution inside it is reported separately. Sleep and engine work
between runs of code are excluded. A call that sleeps is left out of its caller's
time in calls.

The profiler subtracts the cheapest instruction's average time from each instruction
as an estimate of its own overhead. Times are relative to that baseline. With few
hits, an instruction's time may be dominated by one slow execution, such as the first
after a sleep.

Profiling slows execution. Tested tight loops took 1.3–1.6 times as long as with the
debug library alone; their reported totals were within 10% of execution without the
library. Only one profile can run per world. It ends when its client disconnects or
the world reboots. Hot reload keeps measurements of old and new code separate.

## Hot reload (experimental)

The launcher watches source files and reloads changes when they are saved. Hot reload is enabled by default for launched worlds, including **Run Without Debugging**. Supply the same `defines` used to build the `.dmb`.

Hot reload supports proc bodies, proc and verb additions, proc removals, new types and variables, variable types and defaults, `static`, `const` and `tmp` declarations, parameters, `set` statements and parent types. Changes to proc declarations, such as converting a proc to a verb, require a rebuild.

Existing calls finish on their original bytecode. Existing instances retain their verbs and built-in values; computed defaults apply to new instances. `text2path()` resolves previously missing paths after reload on verified builds: 515.1641–516.1689 on Windows and 515.1644–516.1689 on Linux. Other builds retain cached misses until reboot. Changed world settings and client settings stored in the world record, such as `client.control_freak`, take effect after restart.

After reboot, the launcher restores reloads before the world runs its code. Reloads introducing resource files absent from the build wait until `world/New`, so map atoms run the built code.

Running globals and statics keep their values when their initializers change. After reboot, updated initializers run once before map creation. dmasm orders all initializers by their dependencies. This matches the tested codebases, but some source arrangements still differ from DreamMaker. New statics in deferred reloads initialize at `world/New`.

Saving an icon, sound or other referenced resource reloads it, including files found through `FILE_DIR`. Existing appearances redraw with the new file; recompiled code and defaults use it too. Stored values and generated assets keep their old resources. Shared resources need a rebuild to separate them. After reboot, resource edits are restored at `world/New`. Reloaded `FILE_DIR` resources use paths relative to the world directory rather than their source spelling. See the [extension documentation](editors/vscode/README.md#icons-sounds-and-other-files) for details and limits.

Saving the `.dmf` skin updates the interface sent at login. Connected clients need to reconnect; `auxtools reconnect` requests this, or reboots DreamSeeker-hosted worlds.

Changes to `client/script` text also apply at login. Resource-file scripts and included `.dms` files require a rebuild.

Proc-body edits can use a single-file parse. Other edits trigger a full project parse. See the [extension documentation](editors/vscode/README.md#hot-reload-experimental) for reload limits and the [hot reload tests](tests/hot_reload_test) for supported cases.

## Checking for known BYOND bugs

`auxtools check` scans project source for known BYOND bugs, including ignored return values, incorrect operator dispatch and loops that can crash the server. It compiles procs without writing a `.dmb` and runs without BYOND, including in CI.

```sh
auxtools check tgstation.dme -DCIBUILDING --byond-version 516.1689
```

Findings use `FILE:LINE: PROC: diagnostic`. Pass the build's defines with `-D` and select its version with `--byond-version`. Exit code 5 means bugs were found; 0 means none were found. Unsupported procs and parse errors are reported as unchecked and do not affect the exit code. Runtime-only bugs, such as errors swallowed inside operator overloads, are not detected.

## Supported BYOND versions

Windows and Linux: 515.1637–515.1647 and 516.1648–516.1689.

Symbol patterns were checked throughout these ranges. Newer builds are attempted but may fail initialization if a pattern no longer matches. Older builds and unsupported major versions are rejected with a version error. If debugger initialization fails, the world keeps running. The launcher, VS Code and CLI report the reason; `auxtools stop` can terminate the world.

The patterns use [byond-scan](https://github.com/Absolucy/byond-scan) and are stored in `auxtools/src/registry/`. To check them against a build without running it, give its `libbyond.so` or `byondcore.dll` to the registry's checker, on either platform:

```sh
cargo run -p auxtools --example check_recipes -- linux 516.1689 byond/bin/libbyond.so
cargo run -p auxtools --example check_recipes -- windows 516.1689 byond/bin/byondcore.dll
```

It lists each pattern as found or not and exits with status 1 if the build lacks something auxtools looks up on it. A new BYOND build that passes needs no change to the patterns.

CI tests the library, debugger, hot reload and CLI on 515.1637, 515.1647, 516.1659, 516.1685 and 516.1689. The integration suites cover the library's hooks and values, the debug server, source reloads, launcher lifecycle, CLI output and DAP behavior. Linux tests run on Ubuntu 22.04; Windows tests run in `dd.exe` on Windows Server 2022 and under Wine.

The VS Code extension has separate integration coverage on Linux 516.1688 using a harness outside this repository. It has no automated Windows coverage. DreamSeeker and windowed DreamDaemon were checked manually with the first part
of `launcher_test` before the CLI was added; they are outside these automated runs.

## Dependencies

*These instructions were taken directly from tgstation's [rust-g](https://github.com/tgstation/rust-g) documentation.*

The [Rust] compiler:

1. Install the Rust compiler's dependencies (primarily the system linker):

   * Ubuntu: `sudo apt-get install gcc-multilib`
   * Windows (MSVC): [Build Tools for Visual Studio 2017][msvc]

1. Use [the Rust installer](https://rustup.rs), or another Rust installation method,
   or run the following:

    ```sh
    curl https://sh.rustup.rs -sSfo rustup-init.sh
    chmod +x rustup-init.sh
    ./rustup-init.sh
    ```

1. Add the **32-bit** compilation target:

    ```sh
    # Clone the `auxtools` repository to a directory of your choice
    git clone https://github.com/auxtools/auxtools
    # in the `auxtools` directory...
    cd auxtools
    # Linux
    rustup target add i686-unknown-linux-gnu
    # Windows
    rustup target add i686-pc-windows-msvc
    ```

System libraries:

* Ubuntu and Debian users run:

    ```sh
    sudo dpkg --add-architecture i386
    sudo apt-get update
    sudo apt-get install build-essential g++-multilib libc6-i386 libstdc++6:i386
    ```

* Other Linux distributions install the appropriate **32-bit development** and **32-bit runtime** packages.

## Compiling

[dmasm](dmasm), the DM compiler and assembler behind expression evaluation and hot reload, is
a crate of this workspace. Cargo downloads and builds the other Rust dependencies. The
lockfile pins two git dependencies:
[auxtools/SpacemanDMM](https://github.com/auxtools/SpacemanDMM) for parser fixes, and
[byond-scan](https://github.com/Absolucy/byond-scan) for finding BYOND's functions.

Build the workspace in release mode:

Linux:

```sh
cargo build --workspace --release --target i686-unknown-linux-gnu
```

Outputs in `target/i686-unknown-linux-gnu/release`: `libdebug_server.so`, `auxtools`.

Windows:

```sh
cargo build --workspace --release --target i686-pc-windows-msvc
```

Outputs in `target/i686-pc-windows-msvc/release`: `debug_server.dll`, `auxtools.exe`.

To cross-compile Windows builds on Linux, install clang 19 or newer, lld and LLVM, then fetch the SDK (about 800 MB):

```sh
ci/msvc/fetch-sdk.sh ~/.cache/msvc-sdk
export MSVC_SDK=~/.cache/msvc-sdk
. ci/msvc/env.sh
cargo build --workspace --release --target i686-pc-windows-msvc
```

## Continuous integration

`.github/workflows/ci.yml` builds, tests and packages both platforms. It runs on pull requests and pushes to `master` in the willox and auxtools organizations; other forks can run it manually.

Linux binaries target glibc 2.35 and later. Windows binaries target Windows 10 and later and link the C runtime statically. `ci/check-linux.sh` and `ci/check-windows.sh` check these requirements.

CI uses pinned Rust and action versions. `rust-toolchain.toml` selects the same Rust version and both targets for local builds. Format code with `cargo +nightly-2026-10-24 fmt --all`; the rustfmt configuration requires nightly options.

BYOND test archives come from the [BYOND build mirror](https://byond-builds.dm-lang.org), verified against `ci/byond.sha256`. To run the integration suites locally, use `ci/run-tests.sh PROGRAMS BYOND`.

| Artifact | Contents |
| --- | --- |
| `auxtools-linux.tar.gz` | Launcher and debug library |
| `auxtools-linux-symbols.tar.gz` | `auxtools.debug`, `libdebug_server.so.debug`; place beside the binaries for gdb |
| `auxtools-windows.zip` | Launcher and debug library |
| `auxtools-windows-symbols.zip` | `auxtools.pdb`, `debug_server.pdb` |
| `auxtools-VERSION.vsix` | VS Code extension with Linux and Windows binaries |

## Releasing

Run **Actions > Release > Run workflow** on `master` and enter a version such
as `3.0.0`. The workflow builds and tests that version before publishing the
GitHub release and `sohpea.auxtools` extension to the Marketplace and Open VSX.

It creates a `vVERSION` tag and advances `master` to the next patch's `-dev`
version. Publishing uses the `release` environment and trusted publishing on
both registries. See [release.yml](.github/workflows/release.yml) for setup.

[Rust]: https://rust-lang.org
[msvc]: https://visualstudio.microsoft.com/thank-you-downloading-visual-studio/?sku=BuildTools

## License

Auxtools is licensed under the [MIT license](https://en.wikipedia.org/wiki/MIT_License).
See [LICENSE](./LICENSE) for more details.

The debug server, its instruction hook interface, the launcher and dmasm are licensed under GPL-3.0.
See [debug_server/LICENSE](./debug_server/LICENSE), [instruction_hooking/LICENSE](./instruction_hooking/LICENSE), [auxtools-cli/LICENSE](./auxtools-cli/LICENSE) and [dmasm/LICENSE](./dmasm/LICENSE).
