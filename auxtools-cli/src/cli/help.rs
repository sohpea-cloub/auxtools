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
  Requires a paused world. Resume execution and wait for a pause or confirmed
  step completion. Report the resulting state and retain any resulting pause.
  A step can complete with the world running if no frame remains to pause in.
  An expired deadline stops waiting and returns exit code 2; it does not imply
  step completion or automatically pause or resume the world.

Output:
  Execution state and pause details, using the shared execution fields from
  status: pid, paused, reason, message, caught, changes, stack, and instruction.
  On timeout, include the last observed state when available; do not infer
  execution state from the absence of a pause event.

Exit codes:
  0  Execution paused or the step completed.
  1  Step request failed.
  2  Step deadline expired, or the world did not respond in time.
  3  World exited or rebooted before the operation completed.";

pub const PAGES: &[Page] = &[
	Page {
		path: &["launch"],
		brief: "Start a world with debugging and hot reload enabled",
		summary: "Start a compiled world with the debug library loaded.",
		usage: "auxtools launch DMB [OPTIONS] [-- ARGS...]",
		arguments: "DMB   Path to a compiled BYOND world.
ARGS  Arguments passed to the BYOND program. Replace the
      default -trusted and, for Windows DreamDaemon, -close.",
		sections: "Behavior:
  Foreground mode streams launcher and world output until the world exits.
  Interrupt once to request graceful shutdown; interrupt again to force exit.
  The launcher does not compile the DMB or pause execution. Other CLI commands
  and editors can attach to the world. Use reboot to load a rebuilt DMB.
  If the debug library cannot start in the world (an unsupported BYOND
  build), the launcher says why and the world runs without it until stopped.

Output:
  Detached mode reports the world's process ID, the launcher's process ID, and
  the launcher's log file. JSON contains pid, launcher, log, and unavailable:
  why the debug library could not start, or null.

Exit codes:
  0  Startup completed in detached mode, or the foreground world exited with 0.
  1  Launch failed.
  2  Detached startup did not complete within 120 seconds; the world was ended.
  4  Foreground world exited with a nonzero code or was terminated."
	},
	Page {
		path: &["list"],
		brief: "List worlds with the debug library loaded",
		summary: "List local worlds with the debug library loaded.",
		usage: "auxtools list [--json]",
		arguments: "",
		sections: "Output:
  Text displays a table of worlds. JSON returns an array of world records.
  Both include process IDs, program and world information, protocol versions,
  protocol compatibility, and responsiveness. An empty JSON array means no
  worlds were found. Worlds with compatible=false require a matching CLI version.
  A world record contains pid, program, dmb, args, cwd, protocol, compatible,
  responsive, and unavailable. responsive is false if the world did not answer
  within 5 seconds; its dmb, args, and cwd are then unknown. Incompatible
  worlds are not queried and have responsive=null. unavailable says why the
  debug library could not start in the world, which can then only be stopped,
  and is null otherwise."
	},
	Page {
		path: &["status"],
		brief: "Show execution state, uptime, and hot reload status",
		summary: "Show execution state, uptime, and the latest hot reload result.",
		usage: "auxtools [--pid PID] status [--json]",
		arguments: "",
		sections: "Output:
  Text and JSON include execution state, pause details when available, uptime,
  world.time, and the latest hot reload result.
  JSON contains pid, paused, reason, message, caught, changes, stack,
  instruction, uptime_ms, world_time, loads, and last_reload.
  uptime_ms measures time since the latest world load; world_time is in
  deciseconds. loads counts the worlds this process has loaded: 1, plus one
  for each reboot. last_reload is a reload journal entry, or null if none exists.
  reason is \"breakpoint\", \"step\", \"pause\", \"runtime\", or \"change\" when paused.
  Runtime pauses populate message and caught; watch pauses populate changes
  with name, old, and new. Instruction steps include offset and text.
  Inapplicable scalar fields are null; changes and stack are empty arrays
  when unavailable. A running world has paused=false and no pause details."
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
  down. In launched worlds, a runtime error in world.Del() instead leaves
  the debugger available, and stop reports the failure.
  Include --kill in the initial request for a forced fallback.
  With --kill, forced termination starts when the graceful deadline passes,
  or at once when world.Del() fails. A world the debug library could not
  start in cannot call world.Del(), and is terminated at once.
  Success depends on the process exiting within the additional kill timeout.

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
		brief: "Reboot a world and wait for debugger readiness",
		summary: "Reboot the world and wait for it to start running.",
		usage: "auxtools [--pid PID] reboot [--timeout SECONDS] [--json]",
		arguments: "",
		sections: "Behavior:
  Calls world.Reboot() to reload the DMB. Existing debugger connections close.
  Worlds launched with hot reload wait for reloads to be restored before
  running any code. This command returns when the new world starts running.
  This command initiates a reboot; wait reboot only observes one.

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
  Hot reload gives a changed .dmf skin or client script to clients when they
  log in; connected clients keep what they have. This command asks each one
  to reconnect, as DreamSeeker's .reconnect does. The world sees each client log
  out and in again. A world DreamSeeker hosts is rebooted instead, since its
  own window can't reconnect without restarting the world; the command then
  returns when the new world starts running.

Output:
  JSON contains pid and either reconnected, the number of clients asked, or
  rebooted.

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
  With no category options, include all categories. Multiple categories are
  combined. Entries are ordered oldest first; sequence numbers survive reboots.
  The journal retains only the most recent few thousand entries.
  --follow reconnects across reboots and ends when the world exits.
  Runtime logging does not require pausing the world.
  Captured output is available from DreamDaemon (dd.exe on Windows);
  DreamSeeker and windowed DreamDaemon do not expose it to the launcher.

Output:
  JSON entries contain number, time (Unix milliseconds), world_time
  (deciseconds), kind, and event-specific fields.
  kind is \"runtime\", \"reload\", \"output\", \"world\", or \"spawned\".

Exit codes:
  0  Entries were printed; with --follow, the world exited.
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
  JSON is an array of frames: proc, file, line, offset, and old_code.
  With --all, JSON is an array of stacks: id, name, sleeping, and frames."
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
  Text shows the first 200 characters of a long value and its length; use
  --json or eval for the whole value.
  JSON contains arguments, locals, and optionally globals, using the same
  value encoding as eval. Each variable consumes one expansion level, so the
  default expands lists but does not expand object variables."
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
  Use exactly one input source. '-' reads stdin.
  Put '--' before an expression that starts with '-', unless it is a number.
  In a paused world, expressions can access frame variables and src.
  In a running world, expressions can access globals and call procs.
  Several statements form a proc body: separate them with newlines or ';'.
  A proc body yields its return value or final expression. A final statement,
  including an assignment or increment, leaves `.` as the result.
  Expressions can use the project's #define macros when the world was started
  by launch with hot reload enabled.
  Evaluation can change world state through assignments and proc calls.
  Evaluation errors exit with code 1; runtime diagnostics include locations
  and the call stack when available. A callee runtime error ends only that
  proc and returns null to the caller, as in DM. It is reported on stderr
  without failing the evaluation.
  Evaluated code cannot sleep unless --spawn is set. Spawned code runs outside
  any frame (--src can set src), pauses at breakpoints, and records its result
  in the journal (log --spawned). In a paused world it runs after execution
  resumes. --spawn requires a world started by launch.
  In launched worlds, auxtools_kept() stores values between evaluations
  until reboot: auxtools_kept()[\"m\"] = m.

Output:
  JSON represents numbers, text, and null directly. Lists use arrays, objects
  for text-keyed associations, or arrays of {key, value} pairs. Objects use
  {ref, type, name} with vars when expanded. Type paths use {path}; remaining
  values use {kind, text}. Non-finite numbers use {number: \"inf\"},
  {number: \"-inf\"}, or {number: \"nan\"}.
  Lists expand at depth >= 1; object variables expand at depth >= 2.
  Each nested value reduces the depth by one. Unexpanded lists use
  {list: true, len}; unexpanded objects omit vars.
  With --spawn and without --wait, the output is the spawn number: {spawn}.

Exit codes:
  0  Evaluation completed, or the code was spawned.
  1  Evaluation failed; with --wait, the spawned code ended in a runtime error.
  2  --wait deadline expired (the spawned code keeps running), or the world
     did not respond in time.
  3  World exited or rebooted before the spawned code finished (--wait)."
	},
	Page {
		path: &["proc"],
		brief: "Show a proc's source location, definitions, and reload status",
		summary: "Show a proc's source location, definition count, and hot reload status.",
		usage: "auxtools [--pid PID] proc /PATH[#N] [--json]",
		arguments: "/PATH  Proc path, with or without /proc/: /mob/proc/attack or /mob/attack.
       Procs without a path go by the names stacks show: (global init),
       (in list) and (var initializer).
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
		arguments: "/PATH  Proc path, with or without /proc/, or a name stacks show for
       a proc without a path, such as (global init).
#N     Zero-based definition index. [default: 0]",
		sections: "Output:
  Instructions include bytecode offsets, instruction text, and source lines.
  After a hot reload, this shows the updated bytecode.
  JSON contains proc, file, and instructions; each instruction contains
  offset, text, and line."
	},
	Page {
		path: &["pause"],
		brief: "Pause execution and report the location",
		summary: "Pause execution and report the location.",
		usage: "auxtools [--pid PID] pause [--hold-max SECONDS] [--json]",
		arguments: "",
		sections: "Behavior:
  The world remains paused after the command exits. Use continue to resume,
  or set --hold-max to limit the pause when no debugger is connected.
  If the world is already paused, retain that pause and report it.

Output:
  Execution state and pause details, using the shared execution fields from
  status: pid, paused, reason, message, caught, changes, stack, and instruction."
	},
	Page {
		path: &["continue"],
		brief: "Resume execution",
		summary: "Resume execution of a paused world.",
		usage: "auxtools [--pid PID] continue [--json]",
		arguments: "",
		sections: "Behavior:
  Requires a paused world.
  Return after the resume request completes; do not wait for another pause.

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
		summary: "Wait for a debugger or world lifecycle event.",
		usage: "auxtools [--pid PID] wait pause [OPTIONS]
auxtools [--pid PID] wait reload [OPTIONS]
auxtools [--pid PID] wait reboot [OPTIONS]",
		arguments: "",
		sections: "Commands:
  pause   Wait for a pause, optionally installing breakpoints or watches.
  reload  Wait for a hot reload attempt and report its result.
  reboot  Wait for a world load and debugger readiness.

Specify pause, reload, or reboot. Bare `auxtools wait` shows this help.
Use `auxtools wait COMMAND --help` for options."
	},
	Page {
		path: &["wait", "pause"],
		brief: "Wait for a pause, optionally installing breakpoints or watches.",
		summary: "Wait for execution to pause and report the location and reason.",
		usage: "auxtools [--pid PID] wait pause [OPTIONS]",
		arguments: "",
		sections: "Behavior:
  With no triggers, observe any pause. With breakpoints, watches or
  --runtimes, wait for a matching trigger and ignore other pauses.
  If already paused, return immediately unless --change is specified.
  With --change on a paused world, resolve watches in the selected frame,
  resume execution, and wait for a pause.
  Watches support object variables, list entries, globals, and proc variables.
  Breakpoints and watches belong to this invocation and are removed on exit.
  Invalid breakpoint locations fail before waiting. Resolved breakpoint
  locations are reported on stderr and included in JSON output.
  By default, the resulting pause persists after exit. --no-hold does not
  request a retained pause; an existing hold or another debugger can still
  keep the world paused.
  --if does not filter runtime errors or pauses caused by other clients;
  --runtime-filter selects errors by message: 'bounds, null' matches either
  substring; '!libdreamluau' excludes messages containing libdreamluau.

Input:
  For EXPR and CONDITION, @FILE reads text from a file.
  Use @@ to represent a literal leading @.

Output:
  The shared execution fields from status plus already, breakpoints, and watching.
  already indicates that the world was paused before waiting.
  breakpoints lists requested and resolved locations.
  watching lists watch names and their initial values.

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
		brief: "Wait for a hot reload attempt and report its result.",
		summary: "Wait for a hot reload attempt and report whether it was applied.",
		usage: "auxtools [--pid PID] wait reload [--since N] [--timeout SECONDS] [--json]",
		arguments: "",
		sections: "Behavior:
  Observe hot reload; do not trigger it or pause execution.
  Start waiting before saving source changes, or use --since with a previously
  recorded journal sequence number to include an attempt that already finished.
  Historical events are available only while retained in the journal.

Output:
  One reload journal entry, with the same fields as log --reloads.

Exit codes:
  0  Reload was applied.
  1  Reload was rejected or partly applied, or the command failed.
  2  Wait timed out.
  3  World exited before a reload result was received."
	},
	Page {
		path: &["wait", "reboot"],
		brief: "Wait for a world load and debugger readiness.",
		summary: "Wait for a world load and debugger readiness.",
		usage: "auxtools [--pid PID] wait reboot [--since N] [--timeout SECONDS] [--json]",
		arguments: "",
		sections: "Behavior:
  Observe a reboot; do not initiate it or pause execution.
  Reconnect after the reboot and return when the new world starts running,
  after its launcher has reapplied hot reload changes.
  With --since, a retained world-load event can satisfy the wait immediately.

Output:
  One world-load journal entry, with the same fields as log --world.

Exit codes:
  0  World-load event received from the responding debugger.
  1  Command failed.
  2  Wait timed out.
  3  World exited before the operation completed."
	},
	Page {
		path: &["trace"],
		brief: "Log or count breakpoint hits without pausing execution",
		summary: "Log or count breakpoint hits without pausing execution.",
		usage: "auxtools [--pid PID] trace LOCATION... [OPTIONS]",
		arguments: "LOCATION  FILE:LINE or /proc/path[#N]. Specify one or more locations.
          #N selects a zero-based definition. [default: 0]",
		sections: "Behavior:
  Default output identifies each hit location and caller.
  --by suppresses per-hit output and sorts counts from highest to lowest.
  --by and --log are mutually exclusive.
  If both --for and --count are set, stop when either limit is reached.
  Condition errors skip the hit, report a diagnostic once, and increment the
  skipped count. Grouping expression errors are counted under the error text.
  Trace breakpoints are removed when this command exits.

Input:
  For MESSAGE, CONDITION, and EXPR, @FILE reads text from a file.
  Use @@ to represent a literal leading @.

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
  Profile until --for expires or Ctrl-C, then show the report.
  Profiling slows execution. Only one profile can run per world.
  Calls from eval are included; sleep and engine work between runs are excluded.
  Self time covers an instruction's work until the next instruction starts.
  Time inside called procs is separate; calls that sleep are excluded from it.
  The cheapest instruction's average time is subtracted as estimated overhead.

Output:
  Without proc paths, list the slowest procs and source lines.
  With proc paths, show their instructions by source line: hits, share of the
  proc's self time, nanoseconds per hit, and time in calls per hit.
  --html adds an interactive report; --json prints structured data.
  JSON contains seconds, ended (\"time\" or \"interrupted\"), overhead_ns,
  instructions, ns, and procs. Each proc contains proc, override, old_code,
  file, line, calls, instructions, ns, callee_ns, and listing: null outside
  --top, else entries with offset, text, line, source, hits, ns, and callee_ns.
  Instruction listings are limited by --top unless proc paths are given.

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
  Compile every proc and report known bugs affecting the selected BYOND build.
  No DMB is written, and BYOND need not be installed or running.
  Unsupported procs and parse errors are reported as unchecked and do not
  change the exit code. Text output lists them on stderr; JSON includes them.

Output:
  Print FILE:LINE: PROC: diagnostic, then a count of issues and checked procs.
  Paths are relative to the project directory.
  JSON contains byond, procs, found, not_checked, and parse_errors.
  Each entry of found has file, line, proc, override, and text;
  each entry of not_checked has file, proc, override, and error; parse_errors
  are FILE:LINE: message strings. procs counts the procs checked.

Exit codes:
  0  No known bugs found.
  1  Check failed: the project could not be read.
  5  Known BYOND bugs found."
	},
	Page {
		path: &["dap"],
		brief: "Serve the Debug Adapter Protocol over stdin and stdout",
		summary: "Serve the Debug Adapter Protocol over stdin and stdout.",
		usage: "auxtools dap",
		arguments: "",
		sections: "Behavior:
  Accept editor requests to launch or attach to worlds, debug execution,
  and hot reload source changes. Stdout is reserved for DAP messages."
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

const MORE_ROWS: &[(&str, &str, &str, &str)] = &[("trace", "by", "--by caller", "Group hits by the calling proc and source line.")];

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
  --pid PID    Select a world process. Required when selection is ambiguous.
               Without --pid, select the only compatible world.
               Use `auxtools list` to find process IDs.

Responses:
  --response-timeout SECONDS
               {response_timeout}
               Exit with code 2 if the world does not respond in time.
               Event waits are separate: use --timeout or --for where
               available. log --follow has no event deadline.

Output:
  --json       Request structured output where supported.
               log and trace emit JSON Lines; other commands emit one JSON value.
               Default: text. See command help for details.

Pause lifetime:
  pause, step-over, step-in, step-out, and wait pause retain the resulting pause.
  Use continue to resume. --hold-max SECONDS limits a retained pause when no
  debugger is connected. Default: AUXTOOLS_HOLD_MAX, or no limit if unset.

Help:
  auxtools COMMAND --help
  auxtools help COMMAND

Exit codes:
  0  Command completed successfully
  1  Command failed
  2  Operation timed out
  3  World connection ended before the operation completed
  4  Foreground launch: world exited unsuccessfully or was terminated
  5  check: known BYOND bugs found

Commands that follow lifecycle events can reconnect across reboots.
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
					"unexpected argument '{}' before a wait command: specify pause, reload, or reboot.",
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
		assert_eq!(defaults_stated(&first).len(), 1, "{}", first);
		assert!(
			first.contains(&format!("Default: {}, or no limit if unset.", hold::HOLD_MAX_VAR)),
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
  --timeout SECONDS       Set the graceful shutdown deadline. [default: 10]
  --kill                  Terminate the process if graceful shutdown times out.
  --kill-timeout SECONDS  Set the wait after forced termination. [default: 5]
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
			"\n  --by EXPR                   Group hits by expression value and print counts at exit.\n  --by caller                 Group hits by \
			 the calling proc and source line.\n"
		));
	}
}
