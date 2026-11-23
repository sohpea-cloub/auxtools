# auxtools

auxtools is a debugger for BYOND worlds, with experimental hot reload. You can use it from
[VS Code](editors/vscode) or from the command line.

The debugger uses the `auxtools` Rust crate. Other Rust libraries can use the crate to hook DM
procs and work with BYOND values: see [The auxtools library](#the-auxtools-library).

[CHANGELOG.md](CHANGELOG.md) lists what to change when you upgrade from auxtools 3.

## The debugger

Build your world with your project's usual tools. Then start it with the debugger:

- In VS Code, install the extension and press F5 in a workspace that contains a built world.
  The [extension README](editors/vscode/README.md) describes its configuration and features.
- On the command line, run `auxtools launch WORLD.dmb`.

The launcher runs the compiled `.dmb` with the debug library loaded. The world's source needs
no change. The world runs in DreamDaemon (`dd.exe` on Windows). On Windows, it can also run in
windowed DreamDaemon or in DreamSeeker. After `world.Reboot()`, the launcher reconnects and
restores your breakpoints and reloads.

Line breakpoints need the world's line numbers. DreamMaker writes them only when the world
defines `DEBUG`. Add `#define DEBUG` to the `.dme` and rebuild. Many codebases already define
it. Without it, line breakpoints stay unverified, but breakpoints on procs still work.

The debugger has these features:

- line, function, instruction and data breakpoints with conditions and hit counts
- logpoints on source lines
- pausing on runtime errors
- stepping by line or by instruction
- call stacks of sleeping procs
- viewing and setting variables
- evaluating expressions and proc bodies
- disassembly
- a journal of runtime errors, reloads, output and reboots

Several debuggers can connect to the same world. Each session has its own breakpoints and
runtime error settings. A pause stops the world for every session, and any session can resume
it.

### Command line

Run `auxtools help COMMAND` to see a command's options, behavior, output and exit codes.

`auxtools list` shows the running worlds. When several worlds run, select one with `--pid PID`.
The list also shows worlds that use another debugger protocol. You can't attach to them: use
the auxtools that started the world.

By default, each request to the world times out after 30 seconds, with exit code 2. Use
`--response-timeout SECONDS` to change the limit. Results go to stdout and diagnostics to
stderr. With `--json`, a command prints one JSON value, and `log` and `trace` print JSON Lines.

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

| Exit code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | The command failed |
| 2 | The command timed out |
| 3 | The world exited or rebooted before the command finished |
| 4 | A world launched in the foreground exited with a nonzero code or was terminated |
| 5 | `check` found known BYOND bugs |

Editors run `auxtools dap` as their debug adapter.

## Instruction profiling

`auxtools profile` measures execution time by proc, source line and instruction.

```sh
auxtools profile --for 10                       # slowest procs and lines
auxtools profile /mob/living/proc/Life --for 10  # instructions in one proc
auxtools profile --for 10 --html profile.html   # interactive report
```

Without `--for`, profiling runs until you press Ctrl-C. Name procs to measure only those. Use
`--json` for structured output. In VS Code, run **auxtools: Profile instructions** to start.
Run it again to stop and open the report. Calls from `auxtools eval` and the Debug Console are
included.

An instruction's self time lasts until the next instruction starts. It includes BYOND's work
for the instruction, such as copying a list or freeing locals. For a call, self time covers
only the entry into the proc. The profiler reports the time inside the called proc separately.
Sleep and engine work between runs of code are not measured. If a called proc sleeps, its
caller's **time in calls** leaves out that call.

The profiler estimates its own overhead from the instruction with the lowest average time. It
subtracts that estimate from each instruction's time, so times are relative to that baseline.
An instruction with few hits can show the time of one slow run, such as the first run after a
sleep.

Profiling slows execution. In tests, tight loops took 1.3–1.6 times as long as with the debug
library alone. Their reported totals were within 10% of the time without the library. Only one
profile can run in a world at a time. A profile ends when its client disconnects or the world
reboots. After a hot reload, the profiler measures the old and the new code separately.

## Hot reload (experimental)

The launcher watches the source files. When you save a change, the launcher reloads it into the
running world. Hot reload is on by default for launched worlds, also when they run without
debugging. Give the launcher the same defines that you built the `.dmb` with.

Hot reload supports changes to:

- proc bodies, and new procs and verbs
- removed procs
- new types and variables
- variable types and defaults
- `static`, `const` and `tmp` declarations
- parameters and `set` statements
- parent types

A change to a proc declaration needs a rebuild. Changing a proc into a verb is an example.

Some reloads are partly applied: some of their steps changed the world, and then a later step
failed. The world then runs a mix of old and new code. The launcher reports this and lists each
failed step, and `auxtools wait reload` exits with code 1. Rebuild the `.dmb`, then restart the
world to load all of the new code.

**Running code and objects.** Calls that are running when you save finish on the old code.
Existing objects keep their verbs and built-in values. New objects get the new computed
defaults.

**Path lookups.** On some builds, `text2path()` finds a path after a reload, even if an earlier
lookup of that path failed. These builds are 515.1641–516.1689 on Windows and 515.1644–516.1689
on Linux. On other builds, `text2path()` keeps returning null for the path until the world
reboots.

**Settings.** To apply changed world settings, or client settings stored in the world record
such as `client.control_freak`, rebuild and restart the world.

**After a reboot.** The launcher restores your reloads before the world runs any code. A reload
that adds a resource file missing from the build waits until `world/New`. Until then, the atoms
that the map creates run the code from the build.

**Globals and statics.** A running world keeps the values of its globals and statics when you
change their initializers. After a reboot, the changed initializers run once, before the map
loads. dmasm orders all initializers by their dependencies. The order matches DreamMaker's in
the tested codebases, but some source layouts still give a different order. A new static in a
reload that waits for `world/New` gets its value at `world/New`.

**Resources.** When you save an icon, sound or other resource that the code uses, the launcher
reloads it. This includes files found through `FILE_DIR`. Existing appearances redraw with the
new file, and recompiled code and defaults use it too. Values stored in variables and generated
assets keep the old file. Files with the same content share one resource: rebuild to change one
of them alone. After a reboot, resource edits are restored at `world/New`. A reloaded resource
from `FILE_DIR` has a path relative to the world directory, not the path written in the source.
The [extension README](editors/vscode/README.md#icons-sounds-and-other-files) gives details
and limits.

**Skins.** When you save the `.dmf` skin, clients get the new skin when they log in. Run
`auxtools reconnect` to make connected clients log in again. If DreamSeeker hosts the world, the
command reboots the world instead.

**Client scripts.** Changes to `client/script` text also apply when clients log in. Scripts in
resource files and included `.dms` files need a rebuild.

**Parsing.** When you change only proc bodies, the launcher parses only the saved file. Other
changes make it parse the whole project.

The [extension README](editors/vscode/README.md#hot-reload-experimental) lists the limits of
hot reload. The [hot reload tests](tests/hot_reload_test) show the supported cases.

## Checking for known BYOND bugs

`auxtools check` finds known BYOND bugs in a project's source. Examples are ignored return
values, wrong operator calls and loops that can crash the server. It compiles the procs without
writing a `.dmb`, and it doesn't need BYOND, so it can run in CI.

```sh
auxtools check tgstation.dme -DCIBUILDING --byond-version 516.1689
```

Pass the build's defines with `-D`, and its BYOND version with `--byond-version`. Each finding
is printed as `FILE:LINE: PROC: diagnostic`, or as `FILE: PROC: diagnostic` when the line is not
known. The exit code is 5 if the check found bugs, 0 if it found none and 1 if it could not read
the project. Procs that dmasm can't compile, and parse errors, are reported as unchecked. They
don't change the exit code. The check can't find bugs that show only at runtime, such as errors
lost inside operator overloads.

## The auxtools library

The `auxtools` crate lets a Rust library hook DM procs and work with BYOND values. It isn't on
crates.io. Depend on it through git and pin it to a release tag. Build your library as a
`cdylib` for a 32-bit target: `i686-unknown-linux-gnu` or `i686-pc-windows-msvc`.

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
auxtools = { git = "https://github.com/auxtools/auxtools", tag = "v4.0.0" }
```

A hook replaces a DM proc's code with a Rust function. auxtools installs the hooks when DM calls
`auxtools_init`:

```rust
use auxtools::*;

#[hook("/proc/add_one")]
fn add_one(x: Value) {
	Ok(Value::from(x.as_number()? + 1.0))
}
```

The world must declare the proc and call `auxtools_init` before it uses the hook. It must call
`auxtools_shutdown` before it reboots or ends. When a hook returns an error, auxtools calls
`/proc/auxtools_stack_trace`, which raises the error as a DM runtime error:

```dm
/proc/add_one(x)
	CRASH("mylib isn't loaded")

/proc/auxtools_stack_trace(msg)
	CRASH(msg)

/proc/mylib()
	return world.system_type == MS_WINDOWS ? "mylib.dll" : "./libmylib.so"

/world/New()
	var/result = call_ext(mylib(), "auxtools_init")()
	if(result != "SUCCESS")
		world.log << "mylib: [result]"
	. = ..()

/world/Reboot()
	call_ext(mylib(), "auxtools_shutdown")()
	. = ..()

/world/Del()
	call_ext(mylib(), "auxtools_shutdown")()
	. = ..()
```

`auxtools_init` returns `SUCCESS`, or `FAILED` with the reason. The library's API can change
between major versions. [tests/auxtest](tests/auxtest) and its world in
[tests/auxtest_host](tests/auxtest_host) use most of the API.

## Supported BYOND versions

auxtools supports these BYOND builds on Windows and Linux: 515.1637–515.1647 and
516.1648–516.1689.

auxtools finds BYOND's functions by byte patterns. The patterns were checked on every build in
these ranges. auxtools also tries newer builds, but it fails to start on a build where a pattern
no longer matches. It refuses older builds and other major versions with a version error. If the
debugger fails to start, the world keeps running. The launcher, VS Code and the command line
report the reason. `auxtools stop` ends the world.

CI tests the library, the debugger, hot reload and the CLI on 515.1637, 515.1647, 516.1659,
516.1685 and 516.1689. The tests cover the library's hooks and values, the debug server, source
reloads, the launcher's lifecycle, CLI output and the debug adapter. Linux tests run on Ubuntu
22.04. Windows tests run in `dd.exe` on Windows Server 2022 and under Wine.

A harness outside this repository tests the VS Code extension on Linux with 516.1688. No
automated test covers the extension on Windows. The automated tests don't cover DreamSeeker or
windowed DreamDaemon. Both were checked by hand with the first part of `launcher_test`, before
the CLI was added.

### Checking a new BYOND build

The patterns are in `auxtools/src/registry/` and use
[byond-scan](https://github.com/Absolucy/byond-scan). The `check_recipes` example checks them
against a BYOND build without running it. Give it the build's `libbyond.so` or
`byondcore.dll`. Its first argument, `linux` or `windows`, names the kind of file. You can check
either file on either system.

Build the checker for your system's 32-bit target. The `auxtools` crate contains 32-bit
assembly, so it doesn't build for a 64-bit target. On Linux:

```sh
cargo run -p auxtools --target i686-unknown-linux-gnu --example check_recipes -- linux 516.1689 byond/bin/libbyond.so
cargo run -p auxtools --target i686-unknown-linux-gnu --example check_recipes -- windows 516.1689 byond/bin/byondcore.dll
```

On Windows, use `--target i686-pc-windows-msvc` instead.

The checker reports whether it found each pattern. It exits with status 1 if the build lacks a
function that auxtools needs on that build. If a new BYOND build passes, the patterns need no
change. CI runs the checker on each build it tests, for Linux and for Windows.

## Dependencies

*These instructions come from tgstation's [rust-g](https://github.com/tgstation/rust-g)
documentation.*

The [Rust] compiler:

1. Install the Rust compiler's dependencies, mainly the system linker:

   * Ubuntu: `sudo apt-get install gcc-multilib`
   * Windows (MSVC): [Build Tools for Visual Studio 2017][msvc]

1. Install Rust with [the Rust installer](https://rustup.rs) or another method. You can also
   run these commands:

    ```sh
    curl https://sh.rustup.rs -sSfo rustup-init.sh
    chmod +x rustup-init.sh
    ./rustup-init.sh
    ```

1. Clone the repository and add the **32-bit** target:

    ```sh
    git clone https://github.com/auxtools/auxtools
    cd auxtools
    # Linux
    rustup target add i686-unknown-linux-gnu
    # Windows
    rustup target add i686-pc-windows-msvc
    ```

System libraries:

* On Ubuntu and Debian, run:

    ```sh
    sudo dpkg --add-architecture i386
    sudo apt-get update
    sudo apt-get install build-essential g++-multilib libc6-i386 libstdc++6:i386
    ```

* On other Linux distributions, install the **32-bit development** and **32-bit runtime**
  packages.

## Compiling

Cargo downloads and builds the Rust dependencies. The lockfile pins two git dependencies:
[auxtools/SpacemanDMM](https://github.com/auxtools/SpacemanDMM) for parser fixes, and
[byond-scan](https://github.com/Absolucy/byond-scan) for finding BYOND's functions.

Build the workspace in release mode.

On Linux:

```sh
cargo build --workspace --release --target i686-unknown-linux-gnu
```

The outputs are `libdebug_server.so` and `auxtools`, in `target/i686-unknown-linux-gnu/release`.

On Windows:

```sh
cargo build --workspace --release --target i686-pc-windows-msvc
```

The outputs are `debug_server.dll` and `auxtools.exe`, in `target/i686-pc-windows-msvc/release`.

To build for Windows on Linux, install clang 19 or newer, lld and LLVM. Then fetch the Windows
SDK, which is about 800 MB, and build:

```sh
ci/msvc/fetch-sdk.sh ~/.cache/msvc-sdk
export MSVC_SDK=~/.cache/msvc-sdk
. ci/msvc/env.sh
cargo build --workspace --release --target i686-pc-windows-msvc
```

## Continuous integration

`.github/workflows/ci.yml` builds, tests and packages both platforms. It runs on pull requests
and on pushes to `master` in the willox and auxtools organizations. Other forks can start it by
hand.

Linux binaries need glibc 2.35 or later. Windows binaries need Windows 10 or later and link the
C runtime statically. `ci/check-linux.sh` and `ci/check-windows.sh` check these requirements.

CI pins its Rust version and action versions. `rust-toolchain.toml` selects the same Rust
version and both targets for local builds. CI also builds the workspace with Rust 1.95, and
`auxtools`, `auxtools-impl` and `dmasm-bytecode` with Rust 1.88. These are the minimum versions
that the manifests name. Format code with
`cargo +nightly-2026-10-24 fmt --all`. The rustfmt configuration uses options that need nightly.

CI downloads BYOND from the [BYOND build mirror](https://byond-builds.dm-lang.org) and checks
the archives against `ci/byond.sha256`. To run the integration tests locally, build with
`--features debug_server/test-commands` and run `ci/run-tests.sh PROGRAMS BYOND`. The feature
adds the console commands that the tests use to inspect a world. Release builds leave them out.

| Artifact | Contents |
| --- | --- |
| `auxtools-linux.tar.gz` | Launcher and debug library |
| `auxtools-linux-symbols.tar.gz` | `auxtools.debug`, `libdebug_server.so.debug`; put them beside the binaries for gdb |
| `auxtools-windows.zip` | Launcher and debug library |
| `auxtools-windows-symbols.zip` | `auxtools.pdb`, `debug_server.pdb` |
| `auxtools-VERSION.vsix` | VS Code extension with Linux and Windows binaries |

## Releasing

Run **Actions > Release > Run workflow** on `master`, and enter a version such as `4.0.0`. The
workflow builds and tests that version. Then it publishes the GitHub release and the
`sohpea.auxtools` extension to the Marketplace and Open VSX.

The workflow creates a `vVERSION` tag and moves `master` to the next patch version, with a
`-dev` suffix. It publishes from the `release` environment, with trusted publishing on both
registries. [release.yml](.github/workflows/release.yml) describes the setup.

[Rust]: https://rust-lang.org
[msvc]: https://visualstudio.microsoft.com/thank-you-downloading-visual-studio/?sku=BuildTools

## License

Auxtools is licensed under the [MIT license](https://en.wikipedia.org/wiki/MIT_License).
See [LICENSE](./LICENSE) for details.

The debug server, its protocol, its instruction hook interface, the launcher, `auxtools-dm` and
dmasm are licensed under GPL-3.0. See [debug_server/LICENSE](./debug_server/LICENSE),
[auxtools-protocol/LICENSE](./auxtools-protocol/LICENSE),
[instruction_hooking/LICENSE](./instruction_hooking/LICENSE),
[auxtools-cli/LICENSE](./auxtools-cli/LICENSE), [auxtools-dm/LICENSE](./auxtools-dm/LICENSE)
and [dmasm/LICENSE](./dmasm/LICENSE).
