# auxtools Debugger

Debug BYOND worlds in VS Code with [auxtools](https://github.com/willox/auxtools):
breakpoints (conditional ones too, and on procs by name), pausing on runtime errors,
stepping, stack traces, variables, and evaluating DM expressions while paused.

Your code needs nothing added to it. The extension's launcher starts DreamSeeker or
DreamDaemon with the debugger loaded into it, and adds the few procs the debugger needs
when the world loads. It works on Windows and Linux (DreamDaemon only on Linux).

This is only a debugger. For syntax highlighting, completion and the rest, use a DM
language extension alongside it.

## Debugging a world

- Press F5 in a workspace with a `.dme`: it's built with DreamMaker, then run in
  DreamSeeker (Windows) or DreamDaemon (Linux) with the debugger attached.
- Or run **auxtools: Debug in DreamSeeker** or **auxtools: Debug in DreamDaemon**
  from the command palette, or the **Run without debugging** versions.
- For a configuration you can keep, add one to `launch.json` (**Add Configuration...**
  has snippets):

```json
{
	"type": "auxtools",
	"request": "launch",
	"name": "Debug in DreamDaemon",
	"dme": "${workspaceFolder}/tgstation.dme",
	"server": "dreamdaemon",
	"build": true
}
```

Launch options:

| Option | What it does |
|---|---|
| `dme` | The world's `.dme`. Breakpoints, stack traces and hot reloading need it. |
| `dmb` | The world to run, if it isn't the `.dme`'s `.dmb`. |
| `server` | `dreamseeker`, `dreamdaemon`, or `dd` (Windows' DreamDaemon without a window, whose output shows in the debug console). |
| `build` | Build the `.dme` first. |
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
doesn't end a world that was attached to.

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
