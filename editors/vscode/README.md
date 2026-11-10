# auxtools Debugger (Beta)

Debug BYOND worlds in VS Code with [auxtools](https://github.com/willox/auxtools):
breakpoints (conditional ones too, and on procs by name), pausing on runtime errors,
stepping, stack traces, variables, and evaluating DM expressions while paused.

Your code needs nothing added to it. The extension's launcher starts DreamSeeker or
DreamDaemon with the debugger loaded into it, and adds the few procs the debugger needs
when the world loads. It works on Windows and Linux (DreamDaemon only on Linux).

This is only a debugger. For syntax highlighting, completion and the rest, use a DM
language extension alongside it.

## Debugging a world

- Press F5 in a workspace with a built world (`.dmb`): it's run in DreamSeeker
  (Windows) or DreamDaemon (Linux) with the debugger attached. With more than one
  `.dmb`, you pick which.
- Or run **auxtools: Debug in DreamSeeker** or **auxtools: Debug in DreamDaemon**
  from the command palette, or the **Run without debugging** versions.
- For a configuration you can keep, add one to `launch.json` (**Add Configuration...**
  has snippets):

```json
{
	"type": "auxtools",
	"request": "launch",
	"name": "Debug in DreamDaemon",
	"dmb": "${workspaceFolder}/tgstation.dmb",
	"server": "dreamdaemon"
}
```

Launch options:

| Option | What it does |
|---|---|
| `dmb` | The world to run. |
| `dme` | The world's `.dme`, if it isn't the one next to the `.dmb`. Its directory is where the world's files are, and hot reloading reads it. |
| `server` | `dreamseeker`, `dreamdaemon`, or `dd` (Windows' DreamDaemon without a window, whose output shows in the debug console). |
| `build` | Build the `.dme` with DreamMaker first (off unless set). |
| `defines` | DreamMaker's `-D` defines (`NAME` or `NAME=VALUE`), for building and reading the code. |
| `hotReload` | Swap changed code into the running world as files are saved (on unless set to `false`). |
| `byondPath` | Where BYOND is installed, if it isn't found. |
| `args` | What follows the `.dmb` on the command line, in place of `-trusted` (and `-close` for Windows' DreamDaemon). |
| `env` | Environment variables for the world. |
| `debugLibrary` | A debug library to load instead of the extension's own. |

The world's output shows in the debug console (with DreamDaemon on Linux and `dd` on
Windows; DreamSeeker and Windows' DreamDaemon show it in their own windows). The
launcher closes when the world does, and after `world.Reboot()` the debugger picks the
new world up again, breakpoints and all.

Windows' DreamDaemon asks before it runs a world in trusted mode; tick "Don't ask me
this again" to stop it asking.

## Breakpoints

Besides a line, a breakpoint can be on a proc by name (**Add Function Breakpoint**:
`/mob/proc/Login`, or `/mob/Login#1` for its second definition), and can have:

- a **condition**, an expression that has to be true for it to stop;
- a **hit count**: `5` stops from the fifth hit on, `== 5` only then, `% 5` every fifth,
  and `>`, `>=`, `<`, `<=` work too. With a condition, only the hits where it's true are
  counted. The count starts again when the breakpoint is changed.
- a **log message** (**Add Logpoint**): instead of stopping, the breakpoint prints its
  message in the debug console at each hit, with the values of the expressions in
  `{braces}`: `health is {health} for {src}`. It can have a condition and a hit count too.

## Runtime errors

Under **Breakpoints** in the Run and Debug view:

- **Runtime errors** (on unless you untick it) pauses on a runtime error that nothing
  catches, and on a `throw` that nothing catches, on the line it happens on.
- **Caught runtime errors** pauses on the ones inside a `try` too.

Each can have a condition (the pencil next to it): text the error's message has to have,
or after a `!` mustn't have, with commas between several. `!libdreamluau, !rust_g` pauses
on every error but those; `bounds, null` only on errors with either word.

## Variables

The Variables view shows the selected frame's locals and arguments (with `src` and
`usr`), and the world's globals, which are listed when you open them. Objects open to
their vars and lists to their entries, a hundred at a time for long ones. Hovering a
variable shows what kind of value it is.

**Set Value** (double-click a value) takes an expression, evaluated in that frame: a
local, an argument, an object's var, a list's entry or a global. **Copy as Expression**
and **Add to Watch** give the expression that reaches it (`mob.contents[3].name`).
Numbers can be shown in hex (right-click, **Format > Hex**).

## Expressions

Watches, breakpoint conditions and the debug console take DM expressions, evaluated in
the frame you've selected: `health / maxHealth`, `contents.len`, `get_dist(src, usr) < 3`.
Names mean what they would in that proc's code: its locals and arguments, then the vars
of `src`, then globals; a call without `src.` is `src`'s proc if it has one of that name.
They can assign (`health = 100`, `GLOB.debug = TRUE`) and call procs, so a watch can
change the world: it runs at every stop. An expression that fails says why where its
value would be. While the world runs, expressions are evaluated with no frame: globals
and global procs only. Expressions can't declare vars or loop, and can't wait: one that
calls something that sleeps is given up on.

While the world is paused, resting the mouse on a name in the code shows its value in the
selected frame: `health`, or `target.loc.name` with the mouse on `name`. A hover only
reads: it doesn't call procs.

`#dis` in the debug console shows the bytecode of the frame's proc, `#dis /proc/name` any
proc's.

## Hot reloading (experimental)

While a world runs, saving a file swaps the procs that changed into it, with or without
debugging: **Run without debugging** keeps hot reloading. Proc code, new procs and verbs,
new types and vars, var defaults and `set` statements can change; the debug console says
what was reloaded, and what needs a rebuild instead (changed var declarations and parent
types, for one). A world that reboots gets the changes again.

## Attaching to a running world

Any world the launcher started, with debugging or without, can be debugged by any number
of VS Code windows at once: run **auxtools: Attach to a running world**, or use an attach
configuration, and pick the world from the list. Each window has its own breakpoints; a
pause stops the world for all of them, and any of them can let it carry on. Leaving
doesn't end a world that was attached to, and a paused world stays paused for the windows
still debugging it.

```json
{
	"type": "auxtools",
	"request": "attach",
	"name": "Attach to a world",
	"processId": "${command:auxtools.pickWorld}"
}
```

Attach options: `processId` (the world's process; unset, you pick), `dme` (if it isn't
next to the `.dmb` the world runs), `defines`, and `hotReload` (off unless set, since
whoever launched the world may be reloading it already).

## Building

**Tasks: Run Build Task** offers "build NAME.dme" for each `.dme`, with DreamMaker's
errors in the Problems panel (for a `.dme` at the workspace's root).

## Settings

- `auxtools.byondPath`: where BYOND is installed. Unset, the launcher looks at
  `BYOND_SYSTEM`, the registry, `Program Files`, `~/byond` and `/usr/local/byond`.
- `auxtools.launcherPath`, `auxtools.debugLibrary`: your own builds of the launcher and
  the debug library, in place of the extension's.

## Licence

GPL-3.0, like auxtools' debug server, which the extension ships with its launcher
(`LICENSE`).
