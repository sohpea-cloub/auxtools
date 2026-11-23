use clap::error::{ContextKind, ContextValue, ErrorKind};

use super::{
	args::{self, Scope, Unparsed},
	Failure, Result
};

pub struct Page {
	pub path: &'static [&'static str],
	pub brief: &'static str,
	pub summary: &'static str,
	pub usage: &'static str,
	pub arguments: &'static str,
	pub sections: &'static str
}

const STEP_SECTIONS: &str = "Behavior:
  Requires a paused world. The command resumes execution and waits until the
  world pauses again or the step completes. The world stays paused after the
  command exits. A step can complete with the world running when no frame is
  left to pause in.
  On timeout, the command stops waiting and exits with code 2. A timeout does
  not mean that the step completed. The timeout does not pause or resume the
  world.

Output:
  The execution fields of status: pid, paused, reason, message, caught,
  changes, stack, and instruction.
  On timeout, the output shows the last state the world reported, if it
  answered. A missing pause does not mean that the world is running.

Exit codes:
  0  The world paused, or the step completed.
  1  The step failed.
  2  The step timed out, or the world did not respond in time.
  3  The world exited or rebooted before the step completed.";

pub const PAGES: &[Page] = &[
	Page {
		path: &["launch"],
		brief: "Start a world with debugging and hot reload enabled",
		summary: "Start a compiled world with the debug library loaded.",
		usage: "auxtools launch DMB [OPTIONS] [-- ARGS...]",
		arguments: "DMB   Path to a compiled BYOND world.
ARGS  Arguments for the BYOND program. They replace the default
      arguments: -trusted, and also -close for dreamdaemon-gui on Windows.",
		sections: "Behavior:
  In the foreground, the command prints launcher and world output until the
  world exits. Press Ctrl-C once to shut the world down gracefully. Press it
  again to terminate the world.
  The launcher does not build the DMB, and it does not pause execution. Other
  auxtools commands and editors can attach to the world. Use reboot to load a
  rebuilt DMB.
  If the debug library cannot start, the launcher reports the reason. An
  unsupported BYOND build is one cause. The world then runs without debugging
  until you stop it.

Output:
  In detached mode, the command prints the world's process ID, the launcher's
  process ID, and the launcher's log file. JSON contains pid, launcher, log,
  and unavailable. unavailable is the reason the debug library could not
  start, or null.

Exit codes:
  0  The world started in detached mode, or the foreground world exited with 0.
  1  The launch failed.
  2  Detached startup did not complete within 120 seconds. The world was stopped.
  4  The foreground world exited with a nonzero code or was terminated."
	},
	Page {
		path: &["list"],
		brief: "List worlds with the debug library loaded",
		summary: "List local worlds with the debug library loaded.",
		usage: "auxtools list [--json]",
		arguments: "",
		sections: "Output:
  Text output is a table of worlds. JSON output is an array of world records.
  Each record contains pid, program, dmb, args, cwd, protocol, compatible,
  responsive, and unavailable. An empty array means no worlds were found.
  compatible is false when the world uses a different debug protocol. To attach
  to such a world, use an auxtools CLI with the same protocol.
  responsive is false when the world did not answer within 5 seconds. Its dmb,
  args, and cwd are then unknown. The command does not ask incompatible worlds,
  so their responsive is null.
  unavailable is the reason the debug library could not start, or null. A world
  with a reason can only be stopped."
	},
	Page {
		path: &["status"],
		brief: "Show execution state, uptime, and hot reload status",
		summary: "Show execution state, uptime, and the latest hot reload result.",
		usage: "auxtools [--pid PID] status [--json]",
		arguments: "",
		sections: "Output:
  The output shows the execution state, the pause details when paused, the
  uptime, world.time, and the latest hot reload result.
  JSON contains pid, paused, reason, message, caught, changes, stack,
  instruction, uptime_ms, world_time, loads, and last_reload.
  uptime_ms is the time since the latest world load. world_time is in
  deciseconds. loads is the number of worlds this process has loaded: 1, plus
  one for each reboot. last_reload is a reload journal entry, or null.
  When paused, reason is \"breakpoint\", \"step\", \"pause\", \"runtime\", or \"change\".
  A runtime pause sets message and caught. A watch pause sets changes, and each
  change has name, old, and new. instruction is set only by the step commands.
  A field that does not apply is null, or an empty array for changes and stack.
  A running world has paused=false and no pause details."
	},
	Page {
		path: &["stop"],
		brief: "Request shutdown and wait for the process to exit",
		summary: "Request graceful shutdown and wait for the world process to exit.",
		usage: "auxtools [--pid PID] stop [--timeout SECONDS] [--kill [--kill-timeout SECONDS]] [--json]",
		arguments: "",
		sections: "Behavior:
  Calls world.Del(). A world started by launch also shuts down its launcher.
  If world.Del() returns without ending the world, the debugger still shuts
  down. In a launched world, a runtime error in world.Del() leaves the debugger
  running, and stop reports the failure.
  Pass --kill to allow forced termination after the timeout. You can retry
  with --kill while the debugger is reachable.
  With --kill, the command terminates the process when --timeout passes, or at
  once when world.Del() fails. If the debug library could not start,
  world.Del() cannot run, and the command terminates the process at once.
  After termination, the process must exit within --kill-timeout.

Output:
  JSON contains pid, stopped, and killed. killed is true when the process was
  terminated.

Exit codes:
  0  Process exited.
  1  Shutdown or forced termination failed.
  2  Graceful shutdown timed out without --kill, or forced termination timed out."
	},
	Page {
		path: &["reboot"],
		brief: "Reboot a world and wait for its debugger to be ready",
		summary: "Reboot the world and wait for it to start running.",
		usage: "auxtools [--pid PID] reboot [--timeout SECONDS] [--json]",
		arguments: "",
		sections: "Behavior:
  Calls world.Reboot() to load the DMB again. Debugger connections close.
  In a world launched with hot reload, the launcher applies the hot reload
  changes again before the world runs any code. The command returns when the
  new world starts running. To wait for a reboot without starting one, use
  wait reboot.

Output:
  JSON contains pid and rebooted.

Exit codes:
  0  World rebooted and started running.
  1  Reboot request failed.
  2  Reboot timed out.
  3  World process exited instead of rebooting."
	},
	Page {
		path: &["reconnect"],
		brief: "Reconnect clients to load a changed skin or client script",
		summary: "Reconnect clients to load the current skin and client script.",
		usage: "auxtools [--pid PID] reconnect [--timeout SECONDS] [--json]",
		arguments: "",
		sections: "Behavior:
  Hot reload sends a changed .dmf skin or client script to clients when they
  log in. Connected clients keep the old one. This command asks each connected
  client to reconnect, like DreamSeeker's .reconnect command. The world sees
  each client log out and log in again.
  If DreamSeeker hosts the world, its window cannot reconnect without
  restarting the world. The command then reboots the world instead, and
  returns when the new world starts running.

Output:
  JSON contains pid and either reconnected or rebooted. reconnected is the
  number of clients asked to reconnect.

Exit codes:
  0  Clients asked to reconnect, or the world rebooted and started running.
  1  Request failed.
  2  Reboot timed out.
  3  World process exited instead of rebooting."
	},
	Page {
		path: &["log"],
		brief: "Read or follow world events",
		summary: "Read or follow the world's event journal.",
		usage: "auxtools [--pid PID] log [OPTIONS]",
		arguments: "",
		sections: "Behavior:
  Without category options, the command shows every category. With several,
  it shows the entries of each. Entries are listed oldest first. Their numbers
  continue across reboots. The journal keeps only the latest few thousand
  entries.
  --follow reconnects after a reboot and ends when the world exits.
  Runtime errors are logged without pausing the world.
  World output is captured only from dreamdaemon, which is dd.exe on Windows.
  dreamseeker and dreamdaemon-gui do not pass their output to the launcher.

Output:
  Each JSON entry contains number, time, world_time, kind, and the fields of
  its kind. time is in Unix milliseconds. world_time is in deciseconds.
  kind is \"runtime\", \"reload\", \"output\", \"world\", or \"spawned\".

Exit codes:
  0  Entries were printed, or with --follow, the world exited.
  1  Command failed.
  2  The world did not respond in time."
	},
	Page {
		path: &["stack"],
		brief: "Show stack frames",
		summary: "Show stack frames, starting with the innermost frame.",
		usage: "auxtools [--pid PID] stack [--all] [--json]",
		arguments: "",
		sections: "Behavior:
  Requires a paused world.

Output:
  JSON is an array of frames. Each frame contains proc, file, line, offset,
  and old_code. With --all, JSON is an array of stacks. Each stack contains
  id, name, sleeping, and frames."
	},
	Page {
		path: &["vars"],
		brief: "Show frame arguments, local variables, and optional globals",
		summary: "Show arguments and local variables for a paused stack frame.",
		usage: "auxtools [--pid PID] vars [--frame N] [--globals] [--json [--depth N]]",
		arguments: "",
		sections: "Behavior:
  Requires a paused world.

Output:
  Text output cuts a long value to its first 200 characters and shows its
  length. Use --json or eval to see the whole value.
  JSON contains arguments, locals, and with --globals, globals. Values are
  encoded as in eval. Each variable uses one level of --depth. At the default
  depth, lists are expanded and object variables are not."
	},
	Page {
		path: &["eval"],
		brief: "Evaluate a DM expression",
		summary: "Evaluate a DM expression in the world or a paused stack frame.",
		usage: "auxtools [--pid PID] eval EXPRESSION [OPTIONS]
auxtools [--pid PID] eval --file FILE [OPTIONS]
auxtools [--pid PID] eval - [OPTIONS]",
		arguments: "",
		sections: "Behavior:
  Give exactly one input: EXPRESSION, --file, or '-' to read stdin.
  Put '--' before an expression that starts with '-', unless it is a number.
  In a paused world, expressions can use the frame's variables and src.
  In a running world, expressions can use globals and call procs.
  Several statements form a proc body. Separate them with newlines or ';'.
  A proc body returns its return value or the value of its last expression.
  If the last statement is an assignment, an increment or another statement,
  the result is the value of `.`.
  If launch started the world with hot reload, expressions can use the
  project's #define macros.
  Evaluation can change the world through assignments and proc calls.
  An evaluation error exits with code 1. A runtime error shows its location
  and the call stack when they are known.
  A runtime error in a called proc ends only that proc, which returns null to
  its caller, as in DM. The error is shown on stderr, and the evaluation
  continues.
  Evaluated code cannot sleep. Use --spawn to run code that sleeps. Spawned
  code runs outside any frame. Use --src to set its src. Spawned code pauses at
  breakpoints. Its result goes to the journal. Read it with log --spawned.
  In a paused world, spawned code runs after execution resumes.
  --spawn requires a world started by launch.
  In a launched world, auxtools_kept() keeps values between evaluations until
  the world reboots. For example: auxtools_kept()[\"m\"] = m

Output:
  JSON shows numbers, text, and null as they are. A list without associated
  values is an array. A list whose keys are all different text is an object.
  Any other list is an array of {key, value} pairs.
  An object is {ref, type, name}, with vars when expanded. A type path is
  {path}. Any other value is {kind, text}. An infinite or NaN number is
  {number: \"inf\"}, {number: \"-inf\"}, or {number: \"nan\"}.
  --depth sets how deep values are expanded. Lists are expanded at depth 1 or
  more. Object variables are expanded at depth 2 or more. Each level of
  nesting uses one level of depth. A list that is not expanded is
  {list: true, len}. An object that is not expanded has no vars.
  With --spawn and no --wait, the output is the spawn number. JSON output is
  {spawn}.

Exit codes:
  0  Evaluation completed, or the code was spawned.
  1  Evaluation failed, or with --wait, the spawned code ended in a runtime error.
  2  --wait timed out, or the world did not respond in time. Spawned code keeps
     running after a timeout.
  3  With --wait, the world exited or rebooted before the spawned code finished."
	},
	Page {
		path: &["proc"],
		brief: "Show a proc's source location, definitions, and reload status",
		summary: "Show a proc's source location, definition count, and hot reload status.",
		usage: "auxtools [--pid PID] proc /PATH[#N] [--json]",
		arguments: "/PATH  Proc path, with or without /proc/, such as /mob/proc/attack or
       /mob/attack. For a proc without a path, use the name that stacks
       show: (global init), (in list), or (var initializer).
#N     Zero-based definition index. [default: 0]",
		sections: "Output:
  JSON contains proc, file, line, definitions, and reloaded.
  reloaded is null, \"lines\", \"code\", or \"added\"."
	},
	Page {
		path: &["disassemble"],
		brief: "Show a proc's current bytecode",
		summary: "Disassemble a proc's current bytecode.",
		usage: "auxtools [--pid PID] disassemble /PATH[#N] [--json]",
		arguments: "/PATH  Proc path, with or without /proc/. For a proc without a path, use
       the name that stacks show, such as (global init).
#N     Zero-based definition index. [default: 0]",
		sections: "Output:
  The output lists each instruction with its offset, its text, and its source
  line. After a hot reload, it shows the new bytecode.
  JSON contains proc, file, and instructions. Each instruction contains
  offset, text, and line."
	},
	Page {
		path: &["pause"],
		brief: "Pause execution and report the location",
		summary: "Pause execution and report the location.",
		usage: "auxtools [--pid PID] pause [--hold-max SECONDS] [--json]",
		arguments: "",
		sections: "Behavior:
  The world stays paused after the command exits. Use continue to resume it.
  Use --hold-max to resume it after a time with no debugger connected.
  If the world is already paused, the command keeps that pause and reports it.

Output:
  The execution fields of status: pid, paused, reason, message, caught,
  changes, stack, and instruction."
	},
	Page {
		path: &["continue"],
		brief: "Resume execution",
		summary: "Resume execution of a paused world.",
		usage: "auxtools [--pid PID] continue [--json]",
		arguments: "",
		sections: "Behavior:
  Requires a paused world.
  The command returns when the world has resumed. It does not wait for
  another pause.

Output:
  JSON contains pid and resumed."
	},
	Page {
		path: &["step-over"],
		brief: "Advance to the next line, stepping over proc calls",
		summary: "Advance to the next source line, stepping over proc calls.",
		usage: "auxtools [--pid PID] step-over [--instruction] [--timeout SECONDS] [--hold-max SECONDS] [--json]",
		arguments: "",
		sections: STEP_SECTIONS
	},
	Page {
		path: &["step-in"],
		brief: "Advance to the next line, stepping into proc calls",
		summary: "Advance to the next source line, stepping into proc calls.",
		usage: "auxtools [--pid PID] step-in [--instruction] [--timeout SECONDS] [--hold-max SECONDS] [--json]",
		arguments: "",
		sections: STEP_SECTIONS
	},
	Page {
		path: &["step-out"],
		brief: "Run until the current proc returns",
		summary: "Run until the current proc returns to its caller.",
		usage: "auxtools [--pid PID] step-out [--timeout SECONDS] [--hold-max SECONDS] [--json]",
		arguments: "",
		sections: STEP_SECTIONS
	},
	Page {
		path: &["wait"],
		brief: "Wait for a pause, hot reload, or reboot",
		summary: "Wait for a pause, a hot reload, or a reboot.",
		usage: "auxtools [--pid PID] wait pause [OPTIONS]
auxtools [--pid PID] wait reload [OPTIONS]
auxtools [--pid PID] wait reboot [OPTIONS]",
		arguments: "",
		sections: "Commands:
  pause   Wait for a pause, with optional breakpoints or watches
  reload  Wait for a hot reload attempt and report its result
  reboot  Wait for a reboot and for the debugger to be ready

Name pause, reload, or reboot. `auxtools wait` alone shows this help.
Run `auxtools wait COMMAND --help` for options."
	},
	Page {
		path: &["wait", "pause"],
		brief: "Wait for a pause, with optional breakpoints or watches",
		summary: "Wait for execution to pause and report the location and reason.",
		usage: "auxtools [--pid PID] wait pause [OPTIONS]",
		arguments: "",
		sections: "Behavior:
  Without --break, --change, or --runtimes, the command waits for any pause.
  With them, it waits for a pause they cause and ignores other pauses.
  If the world is already paused, the command returns at once, unless
  --change is given. With --change in a paused world, the command finds the
  watched values in the selected frame, resumes execution, and waits.
  Watches work on object variables, list entries, globals, and proc variables.
  The command removes its breakpoints and watches when it exits.
  An invalid breakpoint location fails before the wait starts. The command
  shows each breakpoint's resolved location on stderr and in JSON output.
  By default, the world stays paused after the command exits. With --no-hold,
  the command does not keep it paused. An existing hold or another debugger
  can still keep the world paused.
  --if does not filter runtime errors or pauses caused by other clients.
  --runtime-filter selects errors by message. 'bounds, null' matches either
  substring. '!libdreamluau' excludes messages that contain libdreamluau.

Input:
  For EXPR and CONDITION, @FILE reads the text from FILE.
  Write @@ for text that starts with @.

Output:
  The execution fields of status, plus already, breakpoints, and watching.
  already is true when the world was paused before the wait.
  breakpoints lists the requested and resolved locations.
  watching lists the watched names and their first values.

Exit codes:
  0  World paused, or was already paused.
  1  Setup or command failed.
  2  Wait timed out.
  3  World exited or rebooted before a pause was reported.

Examples:
  auxtools --pid 1234 wait pause --break code/combat.dm:42 --timeout 30 --json
  auxtools --pid 1234 wait pause --change src.health --if \"value < old\" --json
  auxtools --pid 1234 wait pause --runtimes --hold-max 60 --json
  auxtools --pid 1234 wait pause --runtime-filter '!libdreamluau' --json"
	},
	Page {
		path: &["wait", "reload"],
		brief: "Wait for a hot reload attempt and report its result",
		summary: "Wait for a hot reload attempt and report whether it was applied.",
		usage: "auxtools [--pid PID] wait reload [--since N] [--timeout SECONDS] [--json]",
		arguments: "",
		sections: "Behavior:
  The command only waits. It does not start a reload or pause the world.
  Start it before you save the source. To include a reload that already
  finished, pass --since with a journal entry number from before that reload.
  --since finds an earlier reload only while the journal still has its entry.

Output:
  One reload journal entry, with the same fields as log --reloads.

Exit codes:
  0  Reload was applied.
  1  Reload was rejected or partly applied, or the command failed.
  2  Wait timed out.
  3  The world exited before the reload result arrived."
	},
	Page {
		path: &["wait", "reboot"],
		brief: "Wait for a reboot and for the debugger to be ready",
		summary: "Wait for the world to reboot and for its debugger to be ready.",
		usage: "auxtools [--pid PID] wait reboot [--since N] [--timeout SECONDS] [--json]",
		arguments: "",
		sections: "Behavior:
  The command only waits. It does not start a reboot or pause the world.
  After the reboot, it reconnects and returns when the new world starts
  running. By then, the launcher has applied the hot reload changes again.
  With --since, a world load that is already in the journal ends the wait at
  once.

Output:
  One world-load journal entry, with the same fields as log --world.

Exit codes:
  0  The world loaded, and its debugger responded.
  1  Command failed.
  2  Wait timed out.
  3  The world exited before the reboot completed."
	},
	Page {
		path: &["trace"],
		brief: "Log or count breakpoint hits without pausing execution",
		summary: "Log or count breakpoint hits without pausing execution.",
		usage: "auxtools [--pid PID] trace LOCATION... [OPTIONS]",
		arguments: "LOCATION  FILE:LINE or /proc/path[#N]. Specify one or more locations.
          #N selects a zero-based definition. [default: 0]",
		sections: "Behavior:
  By default, the command prints the location and the caller of each hit.
  With --by, it prints no line for each hit. It prints the counts at exit,
  highest first. --by and --log cannot be used together.
  With both --for and --count, the command stops at the first limit reached.
  If the condition fails with an error, the command skips the hit and counts
  it as skipped. It shows the error once.
  If the --by expression fails with an error, the hit is counted under the
  error text.
  The command removes its breakpoints when it exits.

Input:
  For MESSAGE, CONDITION, and EXPR, @FILE reads the text from FILE.
  Write @@ for text that starts with @.

Exit codes:
  0  Duration or hit limit reached, or interrupted with Ctrl-C.
  1  Setup or command failed.
  2  The world did not respond in time.
  3  World exited or rebooted."
	},
	Page {
		path: &["profile"],
		brief: "Profile instruction execution",
		summary: "Measure execution time by proc, source line and instruction.",
		usage: "auxtools [--pid PID] profile [/PATH[#N]...] [OPTIONS]",
		arguments: "/PATH  Proc to measure, with or without /proc/. Specify any number.
       [default: every proc]
#N     Zero-based definition index. [default: 0]",
		sections: "Behavior:
  The command profiles until --for ends or you press Ctrl-C. It then shows the
  report.
  Profiling slows execution. Only one profile can run in a world at a time.
  Code run by eval is included. Sleeping is not included. BYOND's own work
  between runs of DM code is not included.
  An instruction's self time lasts until the next instruction starts.
  Time in called procs is counted separately. It does not include calls that
  sleep.
  The command estimates its overhead as the average time of the cheapest
  instruction, and subtracts it.

Output:
  Without proc paths, the report lists the slowest procs and source lines.
  With proc paths, it lists their instructions by source line. Each line shows
  the hits, the share of the proc's self time, nanoseconds per hit, and time
  in calls per hit.
  --html also writes an interactive report. --json prints the report as JSON.
  JSON contains seconds, ended, overhead_ns, instructions, ns, and procs.
  ended is \"time\" or \"interrupted\". Each proc contains proc, override,
  old_code, file, line, calls, instructions, ns, callee_ns, and listing.
  listing is a list of entries with offset, text, line, source, hits, ns, and
  callee_ns. Without proc paths, listing is null for a proc outside --top.

Exit codes:
  0  Duration reached, or interrupted with Ctrl-C.
  1  Setup or command failed.
  2  The world did not respond in time.
  3  World exited or rebooted."
	},
	Page {
		path: &["check"],
		brief: "Check project source for known BYOND bugs",
		summary: "Report known BYOND bugs detected in project source.",
		usage: "auxtools check [DME] [OPTIONS]",
		arguments: "DME  Path to the source project.
     [default: the only .dme in the current directory]",
		sections: "Behavior:
  The command compiles every proc and reports the known bugs of the selected
  BYOND build. It writes no DMB. BYOND does not need to be installed.
  Procs that cannot be compiled and parse errors are reported as not checked.
  They do not change the exit code. Text output lists them on stderr. JSON
  output includes them.

Output:
  One FILE:LINE: PROC: TEXT line for each issue, then the number of issues and
  of checked procs. Paths are relative to the project directory.
  JSON contains byond, procs, found, not_checked, and parse_errors.
  procs is the number of procs checked.
  Each entry of found has id, file, line, proc, override, and text. The id
  names the bug. It stays the same when the text changes.
  Each entry of not_checked has file, proc, override, and error.
  Each entry of parse_errors is a FILE:LINE: TEXT string.

Exit codes:
  0  No known bugs found.
  1  The project could not be read.
  5  Known BYOND bugs found."
	},
	Page {
		path: &["dap"],
		brief: "Serve the Debug Adapter Protocol over stdin and stdout",
		summary: "Serve the Debug Adapter Protocol over stdin and stdout.",
		usage: "auxtools dap",
		arguments: "",
		sections: "Behavior:
  The command takes requests from an editor to launch worlds, attach to them,
  debug them, and hot reload source changes. It writes only DAP messages to
  stdout."
	},
	Page {
		path: &["help"],
		brief: "Show command help",
		summary: "Show top-level or command-specific help.",
		usage: "auxtools help [COMMAND [SUBCOMMAND]]",
		arguments: "",
		sections: "Equivalent forms:
  auxtools --help
  auxtools COMMAND --help
  auxtools COMMAND SUBCOMMAND --help"
	},
	Page {
		path: &["version"],
		brief: "Show CLI and debug protocol versions",
		summary: "Show CLI and debug protocol versions.",
		usage: "auxtools version [--json]",
		arguments: "",
		sections: "Output:
  JSON contains version and protocol.

Aliases: --version, -V"
	}
];

const MORE_ROWS: &[(&str, &str, &str, &str)] = &[("trace", "by", "--by caller", "Count hits by the calling proc and source line.")];

pub fn overview() -> String {
	let mut text = String::from(
		"auxtools - Launch, inspect, and debug BYOND worlds

Usage:
  auxtools [--pid PID] COMMAND [ARGS] [OPTIONS]
  auxtools help [COMMAND [SUBCOMMAND]]
"
	);
	let width = args::GROUPS
		.iter()
		.flat_map(|(_, commands)| commands.iter())
		.map(|(name, _)| name.len())
		.max()
		.unwrap_or(0);
	for (group, commands) in args::GROUPS {
		text += &format!("\n{}:\n", group);
		for (name, _) in *commands {
			let brief = page(&[name]).map_or("", |page| page.brief);
			text += &format!("  {:width$}  {}\n", name, brief);
		}
	}
	text += &format!(
		"
World selection:
  --pid PID    Select a world process. Without --pid, the command uses the
               only compatible world. Required when there are several.
               Run `auxtools list` to find process IDs.

Responses:
  --response-timeout SECONDS
               {response_timeout}
               The command exits with code 2 if the world does not respond
               in time. This does not limit waits for events. Use --timeout
               or --for to limit those. log --follow waits without a limit.

Output:
  --json       Print JSON instead of text, where a command supports it.
               log and trace print JSON Lines. Other commands print one JSON
               value. See each command's help for its fields.

Pauses:
  pause, step-over, step-in, step-out, and wait pause keep the world paused
  after they exit. Use continue to resume it. --hold-max SECONDS resumes the
  world after that many seconds with no debugger connected.
  [default: AUXTOOLS_HOLD_MAX, or unlimited if unset]

Help:
  auxtools COMMAND --help
  auxtools help COMMAND

Exit codes:
  0  Command completed successfully
  1  Command failed
  2  Operation timed out
  3  World connection ended before the operation completed
  4  launch in the foreground: the world failed or was terminated
  5  check: known BYOND bugs found

log --follow, wait reload, and wait reboot reconnect after a reboot.
",
		response_timeout = args::response_timeout_help()
	);
	text
}

pub fn page(path: &[&str]) -> Option<&'static Page> {
	PAGES.iter().find(|page| page.path == path)
}

fn indented(lines: &str) -> String {
	lines.lines().map(|line| format!("  {}\n", line)).collect()
}

fn spec(arg: &clap::Arg) -> String {
	let mut names: Vec<String> = arg.get_short().iter().map(|short| format!("-{}", short)).collect();
	names.extend(arg.get_long().iter().map(|long| format!("--{}", long)));
	let mut text = names.join(", ");
	if arg.get_action().takes_values() {
		for name in arg.get_value_names().unwrap_or_default() {
			text += &format!(" {}", name);
		}
	}
	text
}

fn option_rows(command: &clap::Command, path: &[&str]) -> Vec<(String, String)> {
	let row = |arg: &clap::Arg| (spec(arg), arg.get_help().map(ToString::to_string).unwrap_or_default());
	let shared = |id: &str| command.get_arguments().find(|arg| arg.get_id() == id).map(row);
	let mut rows = vec![];
	let mut common_json = false;
	for arg in command.get_arguments() {
		let id = arg.get_id().as_str();
		if arg.is_positional() || arg.is_hide_set() || matches!(id, "help" | "pid" | "response_timeout") {
			continue;
		}
		if id == "json" && arg.get_help().is_some_and(|help| help.to_string() == args::JSON) {
			common_json = true;
			continue;
		}
		rows.push(row(arg));
		for (_, _, more, help) in MORE_ROWS.iter().filter(|(name, after, ..)| path == [*name] && *after == id) {
			rows.push(((*more).to_owned(), (*help).to_owned()));
		}
	}
	let scope = path.first().and_then(|name| args::scope_of(name));
	if matches!(scope, Some(Scope::World | Scope::WorldExit)) {
		rows.extend(shared("pid"));
	}
	if scope == Some(Scope::World) {
		rows.extend(shared("response_timeout"));
	}
	if common_json {
		rows.extend(shared("json"));
	}
	rows.push(("-h, --help".to_owned(), "Show help for this command.".to_owned()));
	rows
}

pub fn render(page: &Page, command: &clap::Command) -> String {
	let mut text = format!("{}\n\nUsage:\n{}", page.summary, indented(page.usage));
	if !page.arguments.is_empty() {
		text += &format!("\nArguments:\n{}", indented(page.arguments));
	}
	if !command.has_subcommands() {
		let rows = option_rows(command, page.path);
		let width = rows.iter().map(|(spec, _)| spec.len()).max().unwrap_or(0);
		text += "\nOptions:\n";
		for (spec, help) in rows {
			let mut lines = help.lines();
			text += &format!("  {:width$}  {}\n", spec, lines.next().unwrap_or_default());
			for line in lines {
				text += &format!("  {:width$}  {}\n", "", line);
			}
		}
	}
	if !page.sections.is_empty() {
		text += &format!("\n{}\n", page.sections);
	}
	text
}

pub fn attach(command: clap::Command) -> clap::Command {
	fn attached(mut command: clap::Command, path: &[&str]) -> clap::Command {
		let names: Vec<String> = command.get_subcommands().map(|sub| sub.get_name().to_owned()).collect();
		for name in &names {
			let mut below = path.to_vec();
			below.push(name);
			command = command.mut_subcommand(name, |sub| attached(sub, &below));
		}
		match page(path) {
			Some(page) => {
				let text = render(page, &command);
				command.about(page.brief).override_help(text)
			}
			None if path.is_empty() => command.override_help(overview()),
			None => command
		}
	}
	let mut command = command;
	command.build();
	attached(command, &[])
}

pub fn show(path: &[String]) -> Result<()> {
	if path.is_empty() {
		say_part!("{}", overview());
		return Ok(());
	}
	let root = args::command();
	let mut command = &root;
	for (i, name) in path.iter().enumerate() {
		command = command
			.find_subcommand(name)
			.ok_or_else(|| Failure(1, unknown_command(&path[..i].iter().map(String::as_str).collect::<Vec<_>>(), name)))?;
	}
	let names: Vec<&str> = path.iter().map(String::as_str).collect();
	match page(&names) {
		Some(page) => say_part!("{}", render(page, command)),
		None => return Err(Failure(1, format!("no help page for '{}'.", path.join(" "))))
	}
	Ok(())
}

fn unknown_command(parents: &[&str], name: &str) -> String {
	match parents {
		[] => format!("unknown command '{}'. Run 'auxtools --help' for available commands.", name),
		_ => format!(
			"unknown {} command '{}'. Run 'auxtools {} --help' for available commands.",
			parents.join(" "),
			name,
			parents.join(" ")
		)
	}
}

fn command_path<'a>(root: &'a clap::Command, args: &[String]) -> Vec<&'a str> {
	let mut path = vec![];
	let mut command = root;
	let mut args = args.iter();
	while let Some(arg) = args.next() {
		if arg == "--pid" {
			args.next();
			continue;
		}
		if arg.starts_with('-') {
			continue;
		}
		match command.find_subcommand(arg) {
			Some(sub) => command = sub,
			None => break
		}
		path.push(command.get_name());
		if !command.has_subcommands() {
			break;
		}
	}
	path
}

pub fn unparsed(error: &clap::Error, args: &[String], root: &clap::Command) -> Unparsed {
	if matches!(error.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand) {
		return Unparsed::Help(error.render().to_string());
	}
	let path = command_path(root, args);
	let text = |kind: ContextKind| match error.get(kind) {
		Some(ContextValue::String(text)) => Some(text.clone()),
		Some(ContextValue::Strings(texts)) => Some(texts.join(", ")),
		Some(other) => Some(other.to_string()),
		None => None
	};
	let option = |spec: String| spec.split([' ', '=']).next().unwrap_or_default().to_owned();
	let value_name = |spec: &str| spec.split_once(' ').map(|(_, value)| value.trim_matches(['<', '>', '.']).to_owned());
	let usage = match path.as_slice() {
		[] => "Run 'auxtools --help' for usage.".to_owned(),
		_ => format!("Run 'auxtools {} --help' for usage.", path.join(" "))
	};
	let message = match error.kind() {
		ErrorKind::InvalidSubcommand => return Unparsed::Usage(unknown_command(&path, &text(ContextKind::InvalidSubcommand).unwrap_or_default())),
		ErrorKind::UnknownArgument => {
			let argument = text(ContextKind::InvalidArg).unwrap_or_default();
			match path.as_slice() {
				["wait"] => format!(
					"unexpected argument '{}' before a wait command. Specify pause, reload, or reboot first.",
					argument
				),
				_ => format!("unexpected argument '{}'.", argument)
			}
		}
		ErrorKind::InvalidValue | ErrorKind::ValueValidation | ErrorKind::NoEquals | ErrorKind::TooFewValues | ErrorKind::WrongNumberOfValues => {
			let spec = text(ContextKind::InvalidArg).unwrap_or_default();
			let value = text(ContextKind::InvalidValue).unwrap_or_default();
			let why = std::error::Error::source(error).map(ToString::to_string);
			match (value.is_empty(), why) {
				(true, _) => match value_name(&spec) {
					Some(name) => format!("{} requires a value: {}.", option(spec), name),
					None => format!("{} requires a value.", option(spec))
				},
				(false, Some(why)) => format!("invalid value '{}' for {}: {}.", value, option(spec), why),
				(false, None) => format!("invalid value '{}' for {}.", value, option(spec))
			}
		}
		ErrorKind::MissingRequiredArgument => {
			let missing = text(ContextKind::InvalidArg).unwrap_or_default();
			format!("missing required argument: {}.", missing.replace(['<', '>'], "").replace("...", ""))
		}
		ErrorKind::ArgumentConflict => {
			let first = option(text(ContextKind::InvalidArg).unwrap_or_default());
			match text(ContextKind::PriorArg).map(option) {
				Some(second) if second != first => format!("{} and {} cannot be used together.", second, first),
				_ => format!("{} cannot be used more than once.", first)
			}
		}
		_ => {
			let rendered = error.render().to_string();
			let line = rendered.lines().next().unwrap_or_default();
			format!("{}.", line.trim_start_matches("error: ").trim_end_matches('.'))
		}
	};
	Unparsed::Usage(format!("{} {}", message, usage))
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use auxtools_cli::{byond::Server, local::PROBE_PATIENCE};

	use super::{command_path, overview, page, render, spec, PAGES};
	use crate::cli::{
		args::{self, Scope},
		check, hold, json, launch, HUNG_UP, TIMED_OUT
	};

	fn commands() -> Vec<(Vec<String>, clap::Command)> {
		fn under(command: &clap::Command, path: &[String], found: &mut Vec<(Vec<String>, clap::Command)>) {
			for sub in command.get_subcommands() {
				let mut path = path.to_vec();
				path.push(sub.get_name().to_owned());
				found.push((path.clone(), sub.clone()));
				under(sub, &path, found);
			}
		}
		let mut found = vec![];
		under(&args::command(), &[], &mut found);
		found
	}

	fn names(path: &[String]) -> Vec<&str> {
		path.iter().map(String::as_str).collect()
	}

	fn options_named(text: &str) -> Vec<(&str, &str)> {
		let mut found = vec![];
		let mut at = 0;
		while let Some(offset) = text[at..].find('-') {
			let start = at + offset;
			let rest = &text[start..];
			let hyphens = if rest.starts_with("--") { 2 } else { 1 };
			let name = rest[hyphens..]
				.split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
				.next()
				.unwrap_or_default()
				.trim_end_matches('-');
			at = start + hyphens + name.len();
			if text[..start].ends_with(|c: char| c.is_alphanumeric() || c == '-') {
				continue;
			}
			let long = hyphens == 2 && name.starts_with(|c: char| c.is_ascii_lowercase());
			let short = hyphens == 1 && name.len() == 1 && name.starts_with(|c: char| c.is_ascii_alphabetic());
			if long || short {
				let before = text[..start].split_whitespace().next_back().unwrap_or_default();
				found.push((before.trim_matches(|c: char| !c.is_ascii_alphanumeric()), &rest[..hyphens + name.len()]));
			}
		}
		found
	}

	fn takes(command: &clap::Command, option: &str) -> bool {
		command.get_arguments().any(|arg| {
			arg.get_long().is_some_and(|long| format!("--{}", long) == option) || arg.get_short().is_some_and(|short| format!("-{}", short) == option)
		})
	}

	fn defaults_stated(text: &str) -> Vec<&str> {
		text.split("[default: ")
			.skip(1)
			.map(|rest| rest.split(']').next().unwrap_or_default())
			.collect()
	}

	fn default_in_code(path: &[&str], id: &str) -> Option<String> {
		let seconds = |default: Duration| default.as_secs_f64().to_string();
		Some(match (path, id) {
			(_, "response_timeout") => seconds(args::DEFAULT_RESPONSE_TIMEOUT),
			(["stop"], "timeout") => seconds(args::DEFAULT_STOP_TIMEOUT),
			(["stop"], "kill_timeout") => seconds(args::DEFAULT_KILL_TIMEOUT),
			(["reboot" | "reconnect"], "timeout") => seconds(args::DEFAULT_REBOOT_TIMEOUT),
			(["vars" | "eval"], "depth") => json::DEFAULT_DEPTH.to_string(),
			(["profile"], "top") => args::DEFAULT_TOP.to_string(),
			(["check"], "byond_version") => {
				let latest = dmasm::ByondVersion::LATEST;
				format!("{}.{}", latest.major, latest.build)
			}
			(_, "hold_max") => hold::HOLD_MAX_VAR.to_owned(),
			(["vars" | "eval"] | ["wait", "pause"], "frame") | (["wait", "pause"], "breaks") | (_, "#N") => "0".to_owned(),
			_ => return None
		})
	}

	#[test]
	fn every_command_has_a_page_and_every_page_a_command() {
		let commands = commands();
		for (path, _) in &commands {
			let names: Vec<&str> = path.iter().map(String::as_str).collect();
			assert!(page(&names).is_some(), "{} has no page", path.join(" "));
		}
		assert_eq!(commands.len(), PAGES.len(), "a page is of no command");
		for gone in ["next", "step", "out", "dis", "--list", "run"] {
			assert!(page(&[gone]).is_none(), "{}", gone);
		}
	}

	#[test]
	fn the_first_page_lists_every_command_once() {
		let help = overview();
		let root = args::command();
		let width = root.get_subcommands().map(|command| command.get_name().len()).max().unwrap();
		for command in root.get_subcommands() {
			let name = command.get_name();
			let page = page(&[name]).unwrap();
			assert_eq!(
				help.matches(&format!("\n  {:width$}  {}\n", name, page.brief)).count(),
				1,
				"{} isn't in the help once",
				name
			);
			assert!(!page.brief.ends_with('.'), "{}", page.brief);
		}
	}

	#[test]
	fn a_command_with_commands_lists_each_as_its_page_does() {
		for (path, command) in commands() {
			let mut names = names(&path);
			let text = render(page(&names).unwrap(), &command);
			let width = command.get_subcommands().map(|sub| sub.get_name().len()).max().unwrap_or(0);
			for sub in command.get_subcommands() {
				names.push(sub.get_name());
				let brief = page(&names).unwrap().brief;
				names.pop();
				assert!(
					text.contains(&format!("\n  {:width$}  {}\n", sub.get_name(), brief)),
					"{} doesn't list {}: {}",
					path.join(" "),
					sub.get_name(),
					text
				);
			}
		}
	}

	#[test]
	fn a_page_starts_with_what_the_command_does_and_its_usage() {
		for (path, command) in commands() {
			let names: Vec<&str> = path.iter().map(String::as_str).collect();
			let page = page(&names).unwrap();
			let text = render(page, &command);
			assert!(text.starts_with(&format!("{}\n\nUsage:\n  auxtools ", page.summary)), "{}", text);
			assert!(page.summary.ends_with('.'), "{}", page.summary);
			for usage in page.usage.lines() {
				assert!(usage.contains(&format!(" {}", path.join(" "))), "{}", usage);
				let selects = args::scope_of(names[0]) != Some(Scope::Local);
				assert_eq!(usage.starts_with("auxtools [--pid PID] "), selects, "{}", usage);
			}
			if command.has_subcommands() {
				assert!(!text.contains("\nOptions:\n"), "{}", text);
				continue;
			}
			assert!(text.contains("  -h, --help"), "{}", text);
			assert_eq!(text.contains("  --pid PID "), args::scope_of(names[0]) != Some(Scope::Local), "{}", text);
			assert!(!text.contains("--detached"), "{}", text);
		}
	}

	#[test]
	fn every_option_is_in_its_commands_page_and_says_what_it_does() {
		for (path, command) in commands() {
			if command.has_subcommands() {
				continue;
			}
			let names = names(&path);
			let text = render(page(&names).unwrap(), &command);
			let scope = args::scope_of(names[0]);
			for arg in command.get_arguments().filter(|arg| !arg.is_positional()) {
				let id = arg.get_id().as_str();
				let spec = spec(arg);
				let listed = match id {
					"pid" => scope != Some(Scope::Local),
					"response_timeout" => scope == Some(Scope::World),
					_ => !arg.is_hide_set()
				};
				let row = format!("  {}  ", spec);
				assert_eq!(
					text.lines().filter(|line| line.starts_with(&row)).count(),
					usize::from(listed),
					"{} {}: {}",
					path.join(" "),
					spec,
					text
				);
				if !listed || id == "help" {
					continue;
				}
				let help = arg.get_help().map(ToString::to_string).unwrap_or_default();
				for line in help.lines() {
					assert!(text.contains(&format!("  {}\n", line)), "{} {}: {:?}", path.join(" "), spec, line);
				}
				assert!(
					help.chars().next().is_some_and(char::is_uppercase),
					"{} {}: {:?}",
					path.join(" "),
					spec,
					help
				);
				assert!(help.ends_with('.') || help.ends_with(']'), "{} {}: {:?}", path.join(" "), spec, help);
			}
		}
	}

	#[test]
	fn a_usage_line_names_every_option_or_says_options() {
		for (path, command) in commands() {
			let page = page(&names(&path)).unwrap();
			if command.has_subcommands() || page.usage.contains("[OPTIONS]") {
				continue;
			}
			let own = command
				.get_arguments()
				.filter(|arg| !arg.is_positional() && !arg.is_hide_set() && !matches!(arg.get_id().as_str(), "help" | "pid" | "response_timeout"));
			for arg in own {
				let spec = spec(arg);
				assert!(
					page.usage.contains(&format!("[{}]", spec)) || page.usage.contains(&format!("[{} [", spec)),
					"the usage of {} leaves out {}: {}",
					path.join(" "),
					spec,
					page.usage
				);
			}
		}
	}

	#[test]
	fn a_page_names_no_option_that_doesnt_exist() {
		let root = args::command();
		let commands = commands();
		for (path, command) in &commands {
			let text = render(page(&names(path)).unwrap(), command);
			for (before, option) in options_named(&text) {
				let others = root.find_subcommand(before).is_some_and(|other| takes(other, option));
				assert!(
					takes(command, option) || takes(&root, option) || others,
					"the page of {} names {}, which isn't an option of it: {}",
					path.join(" "),
					option,
					text
				);
			}
		}
		let first = overview();
		for (_, option) in options_named(&first) {
			assert!(
				takes(&root, option) || commands.iter().any(|(_, command)| takes(command, option)),
				"the first page names {}, which isn't an option",
				option
			);
		}
		assert_eq!(options_named("Put '--' before -x (--wait), log --follow; step-over -trusted Ctrl-C"), [
			("before", "-x"),
			("", "--wait"),
			("log", "--follow")
		]);
	}

	#[test]
	fn a_default_the_help_states_is_the_one_the_code_uses() {
		let check = |path: &[&str], id: &str, stated: &str| match default_in_code(path, id) {
			Some(value) => assert!(
				stated
					.strip_prefix(value.as_str())
					.is_some_and(|rest| rest.is_empty() || rest.starts_with([',', ' '])),
				"{} {} says [default: {}], and the code uses {}",
				path.join(" "),
				id,
				stated,
				value
			),
			None => assert!(
				!stated.starts_with(|c: char| c.is_ascii_digit()),
				"{} {} says [default: {}]: name the value the code uses in `default_in_code`",
				path.join(" "),
				id,
				stated
			)
		};
		for (path, command) in commands() {
			let names = names(&path);
			let page = page(&names).unwrap();
			for arg in command.get_arguments() {
				let id = arg.get_id().as_str();
				let help = arg.get_help().map(ToString::to_string).unwrap_or_default();
				let stated = defaults_stated(&help);
				if arg.get_action().takes_values() && !arg.get_default_values().is_empty() {
					let given: Vec<_> = arg.get_default_values().iter().map(|value| value.to_string_lossy()).collect();
					assert_eq!(stated, given, "{} {}", path.join(" "), id);
				}
				for stated in stated {
					if names == ["launch"] && id == "server" {
						let server: Server = serde_json::from_value(serde_json::json!(stated)).expect("a server by its name");
						assert_eq!(server, Server::default_for_platform());
						continue;
					}
					check(&names, id, stated);
				}
			}
			for line in page.arguments.lines().chain(page.sections.lines()) {
				let id = if line.trim_start().starts_with("#N ") { "#N" } else { "" };
				for stated in defaults_stated(line) {
					check(&names, id, stated);
				}
			}
		}
		let first = overview();
		assert!(
			first.contains(&format!("\n               {}\n", args::response_timeout_help())),
			"{}",
			first
		);
		assert_eq!(defaults_stated(&first).len(), 2, "{}", first);
		assert!(
			first.contains(&format!("[default: {}, or unlimited if unset]", hold::HOLD_MAX_VAR)),
			"{}",
			first
		);
	}

	#[test]
	fn the_times_and_exit_codes_in_the_prose_are_the_commands() {
		let sections = |name: &str| page(&[name]).unwrap().sections;
		let within = |time: Duration| format!("within {} seconds", time.as_secs());
		assert!(sections("launch").contains(&within(launch::COMING_UP)), "{}", sections("launch"));
		assert!(sections("list").contains(&within(PROBE_PATIENCE)), "{}", sections("list"));

		fn codes(text: &str) -> Vec<i32> {
			let listed = text.split("Exit codes:\n").nth(1).unwrap_or_default();
			listed
				.lines()
				.take_while(|line| !line.is_empty())
				.filter_map(|line| line.strip_prefix("  ")?.split("  ").next()?.parse().ok())
				.collect()
		}
		let first = codes(&overview());
		assert_eq!(first, [0, 1, TIMED_OUT, HUNG_UP, launch::WORLD_FAILED, check::FOUND]);
		for page in PAGES {
			for code in codes(page.sections) {
				assert!(first.contains(&code), "{}: {}", page.path.join(" "), code);
				assert!(
					code != launch::WORLD_FAILED || page.path == ["launch"],
					"{}: {}",
					page.path.join(" "),
					code
				);
				assert!(code != check::FOUND || page.path == ["check"], "{}: {}", page.path.join(" "), code);
			}
		}
	}

	#[test]
	fn no_line_of_the_help_is_wider_than_a_terminal() {
		let pages = commands().into_iter().map(|(path, command)| {
			let names: Vec<&str> = path.iter().map(String::as_str).collect();
			render(page(&names).unwrap(), &command)
		});
		for text in std::iter::once(overview()).chain(pages) {
			for line in text.lines() {
				assert!(line.chars().count() <= 100, "{} characters: {}", line.chars().count(), line);
				assert_eq!(line, line.trim_end(), "a line ends in spaces: {:?}", line);
			}
			assert!(
				text.ends_with('\n') && !text.ends_with("\n\n"),
				"{:?}",
				&text[text.len().saturating_sub(40)..]
			);
		}
	}

	#[test]
	fn the_command_an_error_is_about_is_found_among_the_arguments() {
		let root = args::command();
		let path = |args: &[&str]| command_path(&root, &args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<String>>());
		assert_eq!(path(&["--pid", "12", "wait", "pause", "--break", "x"]), ["wait", "pause"]);
		assert_eq!(path(&["wait", "--break", "x"]), ["wait"]);
		assert_eq!(path(&["eval", "wait", "pause"]), ["eval"]);
		assert_eq!(path(&["frobnicate", "stop"]), Vec::<&str>::new());
		assert_eq!(path(&["--json"]), Vec::<&str>::new());
	}

	#[test]
	fn options_are_listed_in_a_column() {
		let commands = commands();
		let (_, stop) = commands.iter().find(|(path, _)| path == &["stop".to_owned()]).unwrap();
		let text = render(page(&["stop"]).unwrap(), stop);
		assert!(
			text.contains(
				"\nOptions:
  --timeout SECONDS       Set how long to wait for graceful shutdown. [default: 10]
  --kill                  Terminate the process if graceful shutdown fails or times out.
  --kill-timeout SECONDS  Set how long to wait for the process to exit after termination.
                          [default: 5]
                          Requires --kill.
  --pid PID               Select the world process.
  --json                  Emit one JSON value on stdout.
  -h, --help              Show help for this command.
"
			),
			"{}",
			text
		);
		let (_, eval) = commands.iter().find(|(path, _)| path == &["eval".to_owned()]).unwrap();
		assert!(render(page(&["eval"]).unwrap(), eval).contains("\n  -f, --file FILE             Read the expression from FILE.\n"));
		let (_, launch) = commands.iter().find(|(path, _)| path == &["launch".to_owned()]).unwrap();
		assert!(
			render(page(&["launch"]).unwrap(), launch).contains("\n  -D NAME[=VALUE]   Set a preprocessor define used by hot reload. Repeatable.\n")
		);
		let (_, trace) = commands.iter().find(|(path, _)| path == &["trace".to_owned()]).unwrap();
		assert!(render(page(&["trace"]).unwrap(), trace).contains(
			"\n  --by EXPR                   Count hits by the value of EXPR and print the counts at exit.\n  --by caller                 Count \
			 hits by the calling proc and source line.\n"
		));
	}
}
