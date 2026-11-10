# auxtools

auxtools is a Rust-based successor to the [extools](https://github.com/MCHSL/extools) project.

Currently, it implements some of the features such as the debug server, but is planned to supplant most functionality (maptick, etc.).

Code Documentation: https://auxtools.willox.dev

## Features
Code Coverage - A tool to generate cobertura code coverage XML reports for a codebase. Note that, due to a [BYOND issue](http://www.byond.com/forum/post/108025) it requires some additional work to have properly un-mangled file names in the report.

Debug Server - A debugger for DM, used from VS Code (`editors/vscode`) or from the command line (`auxtools`), with hot reloading of changed code. See [The debugger](#the-debugger).

*Hot reloading* (experimental) - Changed procs can be swapped into a running world without a restart. The launcher watches the world's source while it runs (`hotReload`, on unless set to `false`, with or without debugging), compiles the procs that change with [dmasm](https://github.com/willox/dmasm)'s proc compiler for the BYOND version the world runs on and the `defines` it was built with, and sends them to the debug server; a world that reboots gets them again. When one file is saved and only the code in its procs changed, the launcher parses just that file again, with the macros it had (about 0.1 s for a tgstation file, where parsing everything takes about 6 s); anything else has it parse all the code, and it says why. Proc code, new procs and verbs (more definitions of one too, anywhere among the others for a type's procs), definitions taken away, new types (mobs too), new vars and static locals, var defaults (built-in ones like `name` and `pixel_x` too; those that aren't constants reach new instances), parameters, `in` lists, prefabs (`/obj{a = 1}`) and `set` statements can change, on Windows and Linux. Things made before a reload keep the verbs and built-in values they had. Changed var declarations (`static`, `tmp`, `const`, `as`; a var's type can change) and parent types, a verb's or a global proc's definitions added or taken away anywhere but the end, and the few built-in vars a reload can't set (`ckey`, ...), need a rebuild, and the launcher says so. `tests/hot_reload_test` shows what it does.

## The debugger

Breakpoints (on lines and on procs by name, with conditions, hit counts and log messages), data breakpoints ("who changes this var?"), pausing on runtime errors, stepping by line or by instruction, stack traces with sleeping procs, variables, expressions and whole proc bodies evaluated in a frame, disassembly, and a record of what happened in the world (runtime errors, hot reloads, reboots).

**The world's code needs nothing added.** The launcher, `auxtools` (`auxtools-cli`), starts the world in DreamSeeker, DreamDaemon or `dd.exe` with the debug library loaded into it (`LD_PRELOAD` on Linux; on Windows a call put in front of the program's entry point). The library initialises when the world makes its first proc call and adds the few procs it needs. The launcher never builds a world: it runs the `.dmb` your own tools built, and after `world.Reboot()` (which loads the `.dmb` again) it connects again and gives the new world its breakpoints and reloads back.

**Any number of debuggers at once.** Every world the launcher starts, with debugging or without, listens on a local socket (a named pipe on Windows) named after its process and the protocol's version, `auxtools-debug-vN-PID`. Each client has its own breakpoints and its own say on pausing for runtime errors; a pause stops the world for all of them, and any can let it carry on. `auxtools list` shows the worlds, those started by another version of auxtools too (as not compatible; `"compatible": false` with `--json`: `PROTOCOL_VERSION` in `server_types.rs` is bumped by hand when the messages change).

**From VS Code:** install the extension and press F5 in a workspace with a built world. Its [README](editors/vscode/README.md) has the rest.

**From the command line**, for people, scripts and coding agents: `auxtools` alone lists the commands, and `auxtools help COMMAND` (or `COMMAND --help`) shows a command's page: its options, what it does to the world, what it prints and its exit codes. A command connects to the world, does one thing, prints what came of it and leaves. `--pid PID` says which world when there's more than one. `--json` gives the same result as data: one JSON value, or a line of JSON per entry for `log` and `trace`. What isn't the result goes to stderr. Exit codes are the same everywhere: 0 done, 1 failed, 2 a time limit ran out, 3 the world exited or rebooted first. For example:

```sh
auxtools launch tgstation.dmb --detach             # start it with the debugger and hot reloading
auxtools wait pause --break code/game/foo.dm:120   # wait for it to pause there, and say where
auxtools vars                                      # the frame's arguments and locals
auxtools eval 'src.health'                         # an expression there (or --file FILE, or a proc's body)
auxtools step-over                                 # on to the next line; it stays paused
auxtools continue
auxtools trace /mob/proc/Life --by caller --for 5  # count calls by caller, without pausing
auxtools log --runtimes --follow                   # runtime errors with their stacks, as they happen
auxtools wait reload                               # after saving a file: what the hot reload did
auxtools reboot                                    # after a rebuild: load the .dmb again
auxtools stop
```

`auxtools dap` is the debug adapter editors start.

## Supported BYOND versions

515.1637 through 515.1647, and 516.1648 through 516.1688, on Windows and Linux. Those are the builds the patterns that find BYOND's functions were checked against. A newer build is still tried, and fails at init with the name of whatever stopped matching. An older build, or another major version, is refused at init with an error that names the build.

The patterns are run by [byond-scan](https://github.com/Absolucy/byond-scan) and live in `auxtools/src/symbols/`. They are copies of the ones in the byond-re repo's `byond_catalog`, which checks each against every local build, so a change to one has to land in both.

The debugger, hot reloading and the command line are tested on five of those builds: 515.1637, 515.1647, 516.1659, 516.1685 and 516.1688. Four suites run on each, on Linux and on Windows (under Wine, in `dd.exe`), and all of them pass on all five on both:

- `tests/debug_test` talks to the debug server as a stand-in for the launcher: breakpoints, expressions, a world reboot.
- `tests/hot_reload_test` reloads a world's code a change at a time.
- `tests/launcher_test` drives the launcher the way an editor does, in a world with no auxtools procs: breakpoints, a runtime error, a reboot and the world's exit. Then a world run without debugging is found with `list`, attached to by two launchers at once and hot reloaded by the one that started it. Then the command line: every command against that world, as text and as JSON, with its errors and exit codes, and worlds started, rebooted and stopped with `auxtools launch`, `reboot` and `stop`, in the foreground and with `--detach`.
- `tests/dap_test` debugs a world of its own through the launcher, a group of checks at a time (`dap_test stepping`): breakpoints on lines and procs, conditions, stepping, pausing and sleeping procs, stack frames, variables of every kind, expressions, runtime errors, several sessions at once, reboots, and all of it while the code is hot reloaded.

The VS Code extension is run in a real VS Code on Linux (516.1688) by a harness that isn't in this repository: launching with and without debugging, breakpoints, logpoints, the disassembly view, data breakpoints, edits above a breakpoint, and Stop. It hasn't been run by a test on Windows. DreamSeeker and the windowed DreamDaemon aren't part of these runs: only the first part of `launcher_test` has been run in them, before the command line was added. The `auxtest` suite and the code coverage tool's report were last checked on 516.1687.

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
    git clone https://github.com/willox/auxtools
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
    sudo apt-get install build-essential g++-multilib libc6-i386 libstdc++6:i386 pkg-config libssl-dev libssl-dev:i386
    ```

* Other Linux distributions install the appropriate **32-bit development** and **32-bit runtime** packages.

## Compiling

The [Cargo] tool handles compilation, as well as automatically downloading and
compiling all Rust dependencies.

First, once: dmasm needs SpacemanDMM's parser with some fixes that aren't released yet.
This clones it next to this repository, at `../SpacemanDMM`, and applies them:

```sh
scripts/setup-spacemandmm.sh
```

To compile in release mode (recommended for speed):

Linux:
```sh
export PKG_CONFIG_ALLOW_CROSS=1
cargo build --workspace --release --target i686-unknown-linux-gnu
# output: target/i686-unknown-linux-gnu/release/libauxtools.so,libdebug_server.so,libauxcov.so,auxtools
```

Windows:

```sh
cargo build --workspace --release --target i686-pc-windows-msvc
# output: target/i686-pc-windows-msvc/release/auxtools.dll,debug_server.dll,auxcov.dll,auxtools.exe
```

[Rust]: https://rust-lang.org
[Cargo]: https://doc.rust-lang.org/cargo
[rustup]: https://rustup.rs
[msvc]: https://visualstudio.microsoft.com/thank-you-downloading-visual-studio/?sku=BuildTools

## License

Auxtools is licensed under the [MIT license](https://en.wikipedia.org/wiki/MIT_License).
See [LICENSE](./LICENSE) for more details.

The Auxtool Debug Server (located @ ./debug_server), its launcher (located @ ./auxtools-cli) and Auxcov Code Coverage tool (located @ ./auxcov) are licensed under the [GPL 3.0 license](https://www.gnu.org/licenses/gpl-3.0.en.html).
See [debug_server/LICENSE](./debug_server/LICENSE), [auxtools-cli/LICENSE](./auxtools-cli/LICENSE) or [auxcov/LICENSE](./auxcov/LICENSE) for more details.
