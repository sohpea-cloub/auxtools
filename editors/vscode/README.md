# auxtools Debugger

Debug BYOND worlds in VS Code with [auxtools](https://github.com/auxtools/auxtools). You can
set breakpoints, step through code, pause on runtime errors, see the stacks of sleeping procs,
view variables and evaluate expressions. The extension also shows disassembly, profiles
instructions and has experimental hot reload.

The launcher loads the debugger without changes to your world's source. Worlds run in
DreamDaemon (`dd.exe` on Windows). For syntax highlighting and language features, also install
a DM language extension.

## Start debugging

Build your world. Then press F5, or run **auxtools: Debug in DreamDaemon**. The extension runs
the `.dmb` in DreamDaemon. If the workspace has more than one `.dmb`, it asks you to select one.

Line breakpoints need the world's line numbers. DreamMaker writes them only when the world
defines `DEBUG`. Add `#define DEBUG` to the `.dme` before you build. Many codebases already
define it. Without it, line breakpoints stay unverified, but breakpoints on procs still work.

If you use the [DM language client](https://github.com/SpaceManiac/vscode-dm-langclient), F5
and **Debug in DreamDaemon** run the environment that you selected in it. A launch
configuration without `dmb` and `dme` also uses that environment.

On Windows, the extension opens DreamSeeker on the world when it launches it. To turn this off,
set `auxtools.autoJoin` to `false`. Run **auxtools: Join with DreamSeeker** to open a client at
any time. On Linux, this command offers to copy the world's address instead.

DreamSeeker clients that the extension opens close when the world ends. Set
`auxtools.closeDreamSeeker` to `false` to keep them. They stay open through a reboot and
reconnect.

**Run Without Debugging** (Ctrl+F5) turns off breakpoints and stepping. Hot reload stays on.
You can also set `"noDebug": true` in a launch configuration.

To save a configuration, add it to `.vscode/launch.json`. **Add Configuration...** offers
templates:

```json
{
    "type": "auxtools",
    "request": "launch",
    "name": "Debug in DreamDaemon",
    "dmb": "${workspaceFolder}/tgstation.dmb",
    "server": "dreamdaemon"
}
```

With the DM language client, **Add Configuration...** fills in the world paths as
`${workspaceFolder}/${command:dreammaker.getFilenameDmb}` and
`${workspaceFolder}/${command:dreammaker.getFilenameDme}`. It also sets the language client's
build task as `preLaunchTask`.

| Option | Description |
| --- | --- |
| `dmb` | The compiled world to run. |
| `dme` | The project file. Default: the `.dme` beside the `.dmb`. Its directory is the source root. |
| `server` | The program that runs the world. Default: `dreamdaemon` (`dd.exe` on Windows). On Windows, `dreamdaemon-gui` runs windowed DreamDaemon and `dreamseeker` hosts the world in DreamSeeker. `dd` is another name for `dreamdaemon`. |
| `defines` | The defines that the world was built with: `NAME`, `NAME=VALUE`, or DreamMaker arguments such as `-DA -DB=2`. The extension adds flags selected in the [SS13 Build Flags extension](https://github.com/spacestation13/SS13-BuildFlags) for matching build tasks (see below). |
| `noDebug` | Turn off debugging. Default: `false`. |
| `hotReload` | Reload source changes when you save. Default: `true`. |
| `sleepingThreads` | The most sleeping procs to show in the call stack. Default: `100`. |
| `byondPath` | The BYOND installation to use. Optional. |
| `port` | The port the world listens on. Without it, DreamDaemon chooses a free port when it starts. |
| `args` | Arguments to put after the `.dmb`. They replace the default arguments. The default is `-trusted`, and on Windows `dreamdaemon-gui` also gets `-close`. |
| `env` | Environment variables for the world. |
| `debugLibrary` | The debug library for this launch, instead of `auxtools.debugLibrary`. If neither is set, the extension uses its own library. |

### SS13 Build Flags

Newer versions of SS13 Build Flags provide an API. With such a version, the extension adds the
selected flags when `preLaunchTask` matches `ss13BuildFlags.baseTask`. The build and the
debugger use these flags on launch and on restart. When you select other flags:

- If the code doesn't change, the new flags apply at once.
- If the code changes, the launcher waits for a reboot before it reloads that change or later
  code edits.
- If maps or world settings change, rebuild and restart.

Select the previous flags again to reload the waiting edits.

Older versions have no API. With them, launches and attaches without `defines` use the
selected flags. Set `"defines": []` to turn this off. To use the selected flags with other
defines, write `"defines": ["${command:ss13BuildFlags.current}", "MORE"]`.

### Output, stop and restart

DreamDaemon's output goes to the Debug Console. DreamSeeker and `dreamdaemon-gui` show output in
their own windows. After `world.Reboot()`, the debugger reconnects and restores your breakpoints
and reloads.

**Stop** asks the world to shut down through `world/Del()`, so it can clean up, for example to
save. After five seconds, the launcher ends the process. To end a launched world at once, press
**Stop** again or use **Disconnect**. For attached worlds, see
[Attach to a running world](#attach-to-a-running-world).

**Restart** runs the configuration's `preLaunchTask`, then reboots the world with
`world.Reboot()`. The process and port stay the same, and players reconnect. The world loads
the new `.dmb` and the launcher restores your reloads. So a build task can rebuild the world
before it reboots. If the launch settings changed or the debugger disconnected, **Restart**
stops the old process and launches a new one.

To reboot without the build task, run **auxtools: Reboot world** or choose **Reboot** in a hot
reload notification. This reboots the world in place, also in attached sessions.

Windowed DreamDaemon (`dreamdaemon-gui`) may ask before it runs a world in trusted mode. Select
"Don't ask me this again" to stop the question.

## Breakpoints

Click the gutter to set a line breakpoint. Use **Add Function Breakpoint** to break on a proc:
write its path, such as `/mob/proc/Login`. For the proc's second definition, write
`/mob/Login#1`.

- **Conditions** are DM expressions. When the result is false, the debugger skips the hit. A
  runtime error in a condition also skips the hit, and the Debug Console reports the error
  once. If a condition doesn't compile, the breakpoint isn't set. Hover over it to see why.
- **Hit counts** decide which hits pause. `5` pauses on the fifth hit and every hit after it.
  `== 5` pauses on the fifth hit only, and `% 5` on every fifth hit. You can also use `>`,
  `>=`, `<` and `<=`. Only hits that pass the condition count. The count starts again when you
  change the breakpoint, when its file is reloaded and when the world restarts. A function
  breakpoint keeps its count through a hot reload.
- **Logpoints** print a message instead of pausing. Put expressions in braces, as in
  `health is {health} for {src}`. If an expression fails, its error appears in the message.
  Write `{{` and `}}` for literal braces. Logpoints can also have conditions and hit counts.

With hot reload on, a breakpoint that you add to unsaved text stays unverified. It is verified
when you save the file and the launcher reloads it. Existing breakpoints stay on the running
code. After a reload, they move with their statements when lines move.

With hot reload off, breakpoints refer to the source that the world was built from. If the file
changed after the build, a breakpoint can show on another line than the one it pauses on. The
Debug Console warns about this.

**Loaded Scripts** is in the **...** menu of the Run and Debug view. It lists the files named in
the world's debug information, and BYOND's `stddef.dm`. A file that has only types, variables
or defines is not listed, because it has no procs.

## Sleeping procs

While the world is paused, the Call Stack view lists the running proc and then the sleeping
procs. You can view their frames and variables, and evaluate expressions in them, as usual. When
you step in a sleeping proc, the step finishes when the proc wakes. The view lists up to 100
sleeping procs. Change `sleepingThreads` to change the limit.

## Data breakpoints

While the world is paused, right-click a variable and choose **Break on Value Change**. The
world pauses just after the value changes, usually on the next line. The Debug Console shows the
old and the new value.

You can watch object variables, list entries, globals, locals and arguments. A watch on a local
or an argument lasts only for that call. A watch on a variable that holds a list detects when
the variable gets another list. To detect changes inside the list, watch an entry. You can't
watch `src`, `usr`, or an expression that is not a variable or list entry, such as `a + b`.
Assigning the same value again doesn't trigger the breakpoint.

A data breakpoint's condition can use `value` for the new value and `old` for the previous one,
as in `value < old`. Hit counts and condition errors work as for other breakpoints. A watch
ends when its object is deleted or its proc returns. After a reboot, only watches on globals
come back. The Breakpoints view shows which watches ended.

The debugger checks watched values between instructions. So it also detects changes that the
engine makes, such as atom movement. Watches slow execution while they are on: remove them when
you no longer need them.

## Runtime errors

In the Breakpoints view, **Runtime errors** pauses on runtime errors and thrown values that
nothing catches. It is on by default. **Caught runtime errors** also pauses on errors inside
`try` blocks.

To pause only on some errors, click the filter's pencil icon and enter parts of the messages.
Separate alternatives with commas. Put `!` before a part to exclude messages that contain it.
For example, `bounds, null` matches messages that contain either word, and
`!libdreamluau, !rust_g` excludes messages that contain either name.

When no breakpoint pauses on an error, the Debug Console still prints it. The source locations
in the Debug Console are links to the file and line.

## Variables and evaluation

The Variables view shows the locals and arguments of the selected frame, with `src` and `usr`.
Expand **Globals** to load the global variables. Objects expand to their variables. Lists
expand to their entries, 100 at a time. Hover over a variable to see its type.

**Set Value** takes an expression and evaluates it in the selected frame. **Copy as
Expression** and **Add to Watch** use a path to the value, such as `mob.contents[3].name`. Use
**Format > Hex** to show numbers in hexadecimal.

Watches, conditions and the Debug Console take DM expressions such as `health / maxHealth` or
`get_dist(src, usr) < 3`. While the world is paused, the debugger looks up a name in the selected
frame. It tries locals and arguments first, then variables of `src`, then globals. A call such
as `f()` calls `src.f()` if `src` has that proc. While the world runs, only globals are
available.

`/obj/item::force` reads the initial value of a type's variable. In launched worlds,
`auxtools_kept()` keeps values between evaluations until the world reboots. For example,
`auxtools_kept()["m"] = m` saves a value, and a later `auxtools_kept()["m"]` reads it.

Expressions can assign variables and call procs. Watch expressions run at every pause, so their
side effects can change the world.

The Debug Console also takes proc bodies, with declarations, loops and `return`:

```dm
var/n = 0
for (var/mob/M in world)
    n++
n
```

The result is the returned value or the last expression. A body that ends with a statement,
such as an assignment or an increment, returns `.`, as a proc does. A body can change the
frame's variables. The Debug Console shows compiler warnings with the result. If the code
sleeps or calls a proc that sleeps, the debugger stops it.

Your source's `#define`s work in evaluations and logpoints once the launcher has parsed the
project for hot reload. A reload updates the defines and removes the ones you deleted. Without a
parsed project, the defines are not available. This includes sessions with
`"hotReload": false`.

Evaluation folds constants as the proc compiler does. This includes DreamMaker's quirks, such
as `~-4` and `new /obj` in conditions. Some differences remain:

- The debugger reads const variables from the running world. So an expression such as
  `~-CONST` or `-(0 * CONST)` can give another result than DreamMaker's folded value.
- `x::name` uses the type of the value that `x` holds, not its declared type. It returns null
  when `x` is null.
- `..()`, `__FILE__`, `__LINE__`, `__PROC__` and `__TYPE__` are not available.

When you hover over source code, the debugger evaluates one expression in the selected frame. It
doesn't call procs or assign values for a hover. The Debug Console completes the names of
visible variables and procs, also after a `.`.

`#dis` shows the bytecode of the selected proc. `#dis /proc/name` shows another proc.

## Disassembly

While the world is paused, right-click a stack frame and select **Open Disassembly View**. The
view shows the dmasm instructions, their words in hexadecimal, the source lines and the current
instruction. A frame can still run the bytecode from before a reload. The view then shows that
bytecode without source lines, because the source has changed.

Click beside an instruction address to set a breakpoint. The address must be the start of an
instruction in the proc's current bytecode. Instruction breakpoints can have conditions and hit
counts. They stay set through world reboots.

An address has the form `0xPPPPPPPPCCCCOOOO`: the proc ID, the code ID and the word offset. The
code ID is 0 for the proc's current bytecode. Bytecode from before a reload has another code ID,
until the world reboots. For example, `0x0000002A00000013` is offset `0x13` in the current
bytecode of proc `0x2A`.

When the Disassembly view has focus, the step commands move by instruction:

- **Step Over** runs to the next instruction in the frame. It runs calls to the end.
- **Step Into** runs to the next instruction that executes, also in a called proc.
- **Step Out** runs until the frame returns to its caller.

## Instruction profiling

To start, run **auxtools: Profile instructions**, or choose **Profile instructions** in the
world actions. To stop, click **Profiling** in the status bar. The report then opens. The world
keeps running while you profile, but more slowly.

Select a proc in the report to see its instructions by source line. The report has listings for
the 40 procs with the most self time. Self time leaves out the time in called procs. **Include
calls** adds that time to the bars and shows it in its own column. Click a source line to open
it.

The profiler doesn't measure sleep, or engine work between runs of code. Calls from the Debug
Console are included. The [auxtools README](https://github.com/auxtools/auxtools#instruction-profiling)
explains the limits of the timings.

## Hot reload (experimental)

When you save a source file, the launcher reloads the supported changes into the world. This
also works with **Run Without Debugging**. The status bar shows the progress and the result of
each reload. Click it for actions: join, reconnect, restart, reboot and stop. The Debug Console
reports each reload.

Some changes need a reboot or a rebuild. Your file is still saved, but the launcher doesn't
reload that change or later code edits. The Problems panel shows where the change is, and a
notification offers the action you need:

- **Reboot required**: you changed the initializer of an existing global or static. **Reboot**
  applies the change without running `preLaunchTask`.
- **Rebuild required**: the change is not supported, hot reload can't compile the code, or you
  changed world or client settings that are stored in the `.dmb`. **Rebuild and Restart** runs
  `preLaunchTask` and then restarts the world. Configure a build task or rebuild by hand: the
  launcher doesn't build the world.

Fix or revert the change to reload the other edits.

Some reloads are partly applied: some of their steps changed the world, and then a later step
failed. The world then runs a mix of old and new code. The Debug Console reports this and lists
each failed step, and the status bar shows that the reload failed. Use **Restart** to rebuild
and restart the world.

At startup, the launcher warns about source files that are newer than the `.dmb`. The world may
be missing their edits.

After a reboot, the launcher restores your reloads before the world runs any code. A reload that
adds a resource missing from the build waits until `world/New`. Until then, the atoms that the
map creates run the code from the build.

Hot reload supports these changes:

- proc bodies
- new procs and verbs, and removed ones
- new types, including mobs
- new variables and static locals
- variable types and defaults
- `static`, `const` and `tmp` in variable declarations
- parameters, including their `as` input types and `in` lists
- prefabs
- `set` statements
- parent types (`parent_type`)

You can also add or remove a proc, verb, world proc or global proc definition between existing
overrides.

These changes need a rebuild:

- changes to a proc's declaration, such as making a proc a verb or adding `final`
- built-in defaults that hot reload doesn't support, such as `see_infrared`
- removing an override of a built-in variable on a built-in type itself, such as `/mob/layer`.
  The rebuild is needed because the running world no longer has BYOND's original default.

Reloaded and evaluated code caches fields as DreamMaker does. After `a.b.f()`, a later `a.b.x`
in the same flow can read the old object, even if `f()` put another object in `a.b`. The
debugger gives no warning for this, because the pattern is common. If a call can replace a
field, read the field into a local. Other differences from DreamMaker, and DreamMaker quirks
that hot reload copies, are reported as warnings.

Calls that are running when you save finish on the old code. Existing objects keep their verbs
and built-in values. New objects get the new computed defaults. A new computed variable is null
on existing objects until the world restarts, and so is its `initial()` value. A changed
initializer of a global or static waits for a reboot. Changed world settings, and client
settings stored in the world record such as `client.control_freak`, wait for a rebuild.

Breakpoints move with their statements when you edit in VS Code, in another editor or with a
checkout. The VS Code extension tells the launcher which files have unsaved changes. Other DAP
clients don't, so the launcher places their new breakpoints by the line numbers on disk.

### Icons, sounds and other files

When you save an icon, sound or other resource that the code uses, the launcher reloads it. It
looks for files in the `.dme`'s directory first, then in the `FILE_DIR` directories in order, as
DreamMaker does. Existing appearances, overlays, images and mouse pointers redraw with the new
file. Recompiled code and defaults use the new resource.

Some values keep the old resource: variables, lists, `/icon` and `/sound` datums, mutable
appearances, and objects made later from those values. Files with the same content share one
resource ID, so the launcher can't replace one of them alone. A warning asks you to rebuild.

After a reboot, resource edits and the code that needs them wait until `world/New`. Until then,
map atoms use the resources from the build. If you edit a resource before the launch, the
launcher reports it, but it reloads the file only when you save it again.

A reloaded resource from `FILE_DIR` has a path relative to the world directory. For example,
`"[icon]"` and `browse_rsc()` give `icons/lamp.dmi`, where a rebuilt world gives `lamp.dmi` as
written in the source.

Resource reload changes BYOND's appearance tree, so it needs a BYOND build that auxtools has
checked. On builds newer than 516.1689, auxtools turns resource reload off. The rest of the
debugger still tries to start.

For generated assets and caches, the codebase must make them again, or you must rebuild:

- Icons made at runtime with `icon()` and its procs have their own copy of the data. Examples
  are tgstation's greyscale items and tgui spritesheets.
- Caches built at startup keep the old resources even after a reboot, because global
  initialization runs before the launcher restores resource edits. Examples are tgstation's HUD
  styles and sound groups.
- Files sent with `browse_rsc()` or through an asset cache reach clients when the codebase sends
  them again.

The launcher also watches and reloads files that the world writes while it runs, such as
tgstation's map preview icons.

### Skins

When you save the last `.dmf` that the `.dme` includes, the launcher updates the skin that
clients get when they log in. The new skin stays after a reboot. Connected players must
reconnect to get it: use the reload notification or **auxtools: Reconnect players**. If
DreamSeeker hosts the world, the world reboots instead, because the host can't reconnect on its
own. A skin that uses new images missing from the build needs a rebuild.

### Client scripts

Changes to `client/script` text apply when clients log in, and they stay after a reboot.
Connected players must reconnect to get them: use the notification or **auxtools: Reconnect
players**. Scripts in resource files and included `.dms` files need a rebuild.

## Attach to a running world

Run **auxtools: Attach to a running world** and select a world that the launcher started.
Several VS Code windows can attach to one world at the same time, each with its own
breakpoints. Any session can resume a paused world.

When you disconnect, an attached world keeps running. If it is paused and other debugging
sessions remain, it stays paused. The Debug Console also reports reloads that other launchers
make.

```json
{
    "type": "auxtools",
    "request": "attach",
    "name": "Attach to a world",
    "processId": "${command:auxtools.pickWorld}"
}
```

The attach options are `processId`, `dme`, `defines`, `hotReload` and `sleepingThreads`.
Without `processId`, you select a running world. `hotReload` is `false` by default when you
attach, so that your reloads don't compete with those of the session that launched the world.

## Settings

- `auxtools.byondPath`: the BYOND installation directory. Without it, the launcher looks in
  `BYOND_SYSTEM`, the registry, `Program Files`, `~/byond`, `/usr/local/byond` and
  `/usr/share/byond`.
- `auxtools.launcherPath`: a launcher to use instead of the bundled one. Leave it empty to use
  the bundled launcher.
- `auxtools.debugLibrary`: a debug library to use instead of the bundled one, `debug_server.dll`
  on Windows or `libdebug_server.so` on Linux. Leave it empty to use the bundled library.
- `auxtools.addToTerminalPath`: add the launcher's directory to `PATH` in VS Code terminals, so
  commands such as `auxtools log` work there. Open new terminals to apply a change. Default:
  `true`.
- `auxtools.autoJoin`: open DreamSeeker on a world when it starts, on Windows. Default: `true`.
- `auxtools.closeDreamSeeker`: close the clients that the extension opened when the world
  process ends. Default: `true`.
- `auxtools.notifications`: set a category to `false` to hide its notifications:
  - `reconnect`: offers to reconnect players after a skin or client script reload.
  - `join`: offers to join a world that the extension didn't join automatically, or to copy its
    address when DreamSeeker is not available.
  - `rebuild`: warnings about changes that need a reboot or a rebuild.
  - `outdated`: warnings about edits that may be missing from the build.

## License

GPL-3.0. See [LICENSE](LICENSE). The extension contains the launcher and the debug server.
