# auxtools

auxtools is a Rust-based successor to the [extools](https://github.com/MCHSL/extools) project.

Currently, it implements some of the features such as the debug server, but is planned to supplant most functionality (maptick, etc.).

Code Documentation: https://auxtools.willox.dev

## Features
Code Coverage - A tool to generate cobertura code coverage XML reports for a codebase. Note that, due to a [BYOND issue](http://www.byond.com/forum/post/108025) it requires some additional work to have properly un-mangled file names in the report.

Debug Server - A debugger for DM: breakpoints (conditional ones, and on procs by name), pausing on runtime errors, stepping, stack traces, variables and expression evaluation. It's used through the VS Code extension (`editors/vscode`): press F5 in a workspace with a `.dme`. Its launcher (`tools/auxtools-cli`, a debug adapter that talks DAP on stdin and stdout) builds the world, starts it in DreamSeeker, DreamDaemon or `dd.exe` (DreamDaemon on Linux) with the debug library loaded into it, shows the world's output in the debug console, and exits when the world does. The world's code needs nothing added: the library is loaded before the world (`LD_PRELOAD` on Linux; on Windows a call put in front of the program's entry point), initialises when the world makes its first proc call, and adds the procs it needs, including one that sleeps in a loop so requests are answered in a world that runs no code of its own. After `world.Reboot()` it connects again and gets the breakpoints back. Every world it starts, with debugging or without, listens on a local socket (a named pipe on Windows) named after its process and the protocol's version, `auxtools-debug-vN-PID` (`PROTOCOL_VERSION` in `server_types.rs`, bumped by hand when the messages change, so launchers only find worlds they can talk to), for any number of debuggers at once: VS Code's attach configuration lists them (`auxtools --list`) and attaches another launcher. Each has its own breakpoints and its own say on pausing for runtime errors; a pause stops the world for all of them, and any can let it carry on. The launcher also sends a running world one command from the command line, for scripts and agents: `auxtools [--pid PID] COMMAND` connects, does it, prints what came of it and leaves. The commands are `status`, `stack`, `vars`, `eval`, `pause`, `continue`, `next`, `step`, `out` and `wait`, which waits for the world to pause (at its own `--break FILE:LINE` or `--break /proc/path` breakpoints, on a runtime error with `--runtimes`, or for another debugger) and prints why and where: run in the background, it says when the code stops. A world a command paused stays paused after it leaves, until a `continue`. `python3 editors/vscode/package.py` packages the extension with the release builds of the launcher and library for each platform built; `tests/launcher_test` drives the launcher like VS Code would. The launcher is the only way to debug: the debug server doesn't listen on a network port, and worlds don't load the library themselves (an `enable_debugging()` call in a world's code does nothing). The launcher is also a library (`auxtools_cli::game`) for what talks to the debug server directly: `tests/debug_test`, `tests/hot_reload_test` and `tools/debug_client`, a stand-in debugger that runs a real codebase to one breakpoint.

*Dissassembly*  - When currently stopped at a breakpoint, enter the `#dis` command into the Debug Console in VSC to see the DM bytecode for the current proc. You can also provide it any arbitrary proc path like `#dis /mob/proc/Life`.

*Hot reloading* (experimental) - Changed procs can be swapped into a running world without a restart. The launcher watches the world's source while it runs (`hotReload`, on unless set to `false`, with or without debugging), compiles the procs that change with [dmasm](https://github.com/willox/dmasm)'s proc compiler for the BYOND version the world runs on and the `defines` it was built with, and sends them to the debug server; a world that reboots gets them again. When one file is saved and only the code in its procs changed, the launcher parses just that file again, with the macros it had (about 0.1 s for a tgstation file, where parsing everything takes about 6 s); anything else has it parse all the code, and it says why. Proc code, new procs and verbs (more definitions of one too, anywhere among the others for a type's procs), definitions taken away, new types (mobs too), new vars and static locals, var defaults (built-in ones like `name` and `pixel_x` too; those that aren't constants reach new instances), parameters, `in` lists, prefabs (`/obj{a = 1}`) and `set` statements can change, on Windows and Linux. Things made before a reload keep the verbs and built-in values they had. Changed var declarations (`static`, `tmp`, `const`, `as`; a var's type can change) and parent types, a verb's or a global proc's definitions added or taken away anywhere but the end, and the few built-in vars a reload can't set (`ckey`, ...), need a rebuild, and the launcher says so. `tests/hot_reload_test` shows what it does.

## Supported BYOND versions

516.1659 through 516.1688 on Windows, and 516.1664 through 516.1688 on Linux. Those are the builds the patterns that find BYOND's functions were checked against. A newer build is still tried, and fails at init with the name of whatever stopped matching. An older build, or another major version, is refused at init with an error that names the build.

The patterns are run by [byond-scan](https://github.com/Absolucy/byond-scan) and live in `auxtools/src/symbols/`. They are copies of the ones in the byond-re repo's `byond_catalog`, which checks each against every local build, so a change to one has to land in both.

What was run on Linux (516.1687): the `auxtest` suite, and `debug_test`, which sets breakpoints, evaluates expressions and follows a world reboot through a stand-in for the launcher. The code coverage tool's report was checked on Windows and Linux 1687 against a world with a known set of lines. Stepping and breakpoints with a real debugger attached were not tested on either platform.

The launcher's test (`tests/launcher_test`: breakpoints, a runtime error, a reboot and the world's exit, in a world with no auxtools procs; then a world run without debugging, found with `--list`, attached to by two launchers at once and hot reloaded by the one that started it, then paused and looked at with the command line) was run on Linux 516.1688, and under Wine on Windows 516.1688 with `dd.exe` (its first part also with `dreamseeker.exe` and `dreamdaemon.exe`). The VS Code extension was not run in VS Code.

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

The Auxtool Debug Server (located @ ./debug_server), its launcher (located @ ./tools/auxtools-cli) and Auxcov Code Coverage tool (located @ ./auxcov) are licensed under the [GPL 3.0 license](https://www.gnu.org/licenses/gpl-3.0.en.html).
See [debug_server/LICENSE](./debug_server/LICENSE), [tools/auxtools-cli/LICENSE](./tools/auxtools-cli/LICENSE) or [auxcov/LICENSE](./auxcov/LICENSE) for more details.
