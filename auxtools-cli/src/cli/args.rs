use std::{path::PathBuf, time::Duration};

use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};

use super::{help, hold, json, launch};

const JSON: &str = "Emit one JSON value on stdout.";
const HOLD_MAX: &str = "Automatically resume a retained pause after this many\nseconds with no debugger connected.";

pub const DEFAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_STOP_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_KILL_TIMEOUT: Duration = Duration::from_secs(5);
pub const DEFAULT_REBOOT_TIMEOUT: Duration = Duration::from_secs(60);
pub const DEFAULT_TOP: u64 = 40;

fn default_seconds(default: Duration) -> String {
	format!("[default: {}]", default.as_secs_f64())
}

pub fn response_timeout_help() -> String {
	format!(
		"Set the wait for each response from the world. {}",
		default_seconds(DEFAULT_RESPONSE_TIMEOUT)
	)
}

fn hold_max_help(help: &str) -> String {
	format!("{}\n[default: {}, or unlimited if unset]", help, hold::HOLD_MAX_VAR)
}

fn depth_help() -> String {
	format!("Set JSON value expansion depth. Requires --json. [default: {}]", json::DEFAULT_DEPTH)
}

#[derive(Parser)]
#[command(name = "auxtools", disable_help_subcommand = true, disable_version_flag = true)]
pub struct Cli {
	#[arg(long, global = true, value_name = "PID", value_parser = pid, help = "Select the world process.")]
	pub pid: Option<u32>,
	#[arg(long, global = true, value_name = "SECONDS", value_parser = seconds, help = response_timeout_help())]
	pub response_timeout: Option<Duration>,
	#[arg(long, short = 'V', hide = true)]
	pub version: bool,
	#[command(subcommand)]
	pub command: Option<Command>
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
	Local,
	World,
	WorldExit
}

macro_rules! commands {
	($($group:tt { $($name:tt => $variant:ident $(($options:ident))?, $scope:ident;)* })*) => {
		#[derive(Subcommand)]
		pub enum Command {
			$($(
				#[command(name = $name)]
				$variant $(($options))?,
			)*)*
		}

		impl Command {
			pub const fn name(&self) -> &'static str {
				match self {
					$($(Self::$variant { .. } => $name,)*)*
				}
			}

			pub const fn scope(&self) -> Scope {
				match self {
					$($(Self::$variant { .. } => Scope::$scope,)*)*
				}
			}
		}

		pub const GROUPS: &[(&str, &[(&str, Scope)])] = &[
			$(($group, &[$(($name, Scope::$scope),)*]),)*
		];
	};
}

commands! {
	"World management" {
		"launch" => Launch(Launch), Local;
		"list" => List(List), Local;
		"status" => Status(Status), World;
		"stop" => Stop(Stop), WorldExit;
		"reboot" => Reboot(Reboot), World;
		"reconnect" => Reconnect(Reconnect), World;
		"log" => Log(Log), World;
	}
	"Inspection" {
		"stack" => Stack(Stack), World;
		"vars" => Vars(Vars), World;
		"eval" => Eval(Eval), World;
		"proc" => Proc(ProcInfo), World;
		"disassemble" => Disassemble(Disassemble), World;
	}
	"Execution" {
		"pause" => Pause(Pause), World;
		"continue" => Continue(Continue), World;
		"step-over" => StepOver(StepOver), World;
		"step-in" => StepIn(StepIn), World;
		"step-out" => StepOut(StepOut), World;
		"wait" => Wait(Wait), World;
		"trace" => Trace(Trace), World;
		"profile" => Profile(Profile), World;
	}
	"Source code" {
		"check" => Check(Check), Local;
	}
	"Integration" {
		"dap" => Dap, Local;
		"help" => Help(Help), Local;
		"version" => Version(Version), Local;
	}
}

impl Command {
	pub const fn selects_world(&self) -> bool {
		!matches!(self.scope(), Scope::Local)
	}

	pub const fn awaits_responses(&self) -> bool {
		matches!(self.scope(), Scope::World)
	}
}

pub fn scope_of(name: &str) -> Option<Scope> {
	GROUPS
		.iter()
		.flat_map(|(_, commands)| commands.iter())
		.find(|(command, _)| *command == name)
		.map(|(_, scope)| *scope)
}

#[derive(Args)]
pub struct Launch {
	#[arg(value_name = "DMB")]
	pub dmb: PathBuf,
	#[arg(long, help = "Run in the background; return when startup completes.")]
	pub detach: bool,
	#[arg(long, help = "Emit {pid, launcher, log}. Requires --detach.")]
	pub json: bool,
	#[arg(
		long,
		value_name = "FILE",
		help = format!("Write launcher output to FILE.\nWith --detach, default: NAME.{} beside NAME.dmb.", launch::LOG_EXTENSION)
	)]
	pub log: Option<PathBuf>,
	#[arg(
		long,
		value_name = "FILE",
		help = "Set the source project used for hot reload.\n[default: NAME.dme beside NAME.dmb]"
	)]
	pub dme: Option<PathBuf>,
	#[arg(
		short = 'D',
		value_name = "NAME[=VALUE]",
		help = "Set a preprocessor define used by hot reload. Repeatable.\nUse the same defines used to compile the DMB."
	)]
	pub defines: Vec<String>,
	#[arg(long, help = "Disable source watching and hot reload.")]
	pub no_hot_reload: bool,
	#[arg(
		long,
		value_name = "PROGRAM",
		help = "Select dreamdaemon, dreamdaemon-gui, or dreamseeker.\n[default: dreamdaemon]\nOn Windows, dreamdaemon runs dd.exe and \
		        dreamdaemon-gui runs DreamDaemon.exe\nwith its window. Linux supports DreamDaemon only."
	)]
	pub server: Option<String>,
	#[arg(long, value_name = "N", value_parser = port, help = "Set the listening port for DreamDaemon.")]
	pub port: Option<u16>,
	#[arg(
		long,
		value_name = "PATH",
		help = "Set the BYOND installation path.\n[default: BYOND_SYSTEM, then the installed location]"
	)]
	pub byond: Option<PathBuf>,
	#[arg(long, hide = true)]
	pub detached: bool,
	#[arg(last = true, value_name = "ARGS")]
	pub args: Vec<String>
}

#[derive(Args)]
pub struct List {
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct Status {
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct Stop {
	#[arg(
		long,
		value_name = "SECONDS",
		value_parser = seconds,
		help = format!("Set the graceful shutdown deadline. {}", default_seconds(DEFAULT_STOP_TIMEOUT))
	)]
	pub timeout: Option<Duration>,
	#[arg(long, help = "Terminate the process if graceful shutdown times out.")]
	pub kill: bool,
	#[arg(
		long,
		value_name = "SECONDS",
		value_parser = seconds,
		help = format!("Set the wait after forced termination. {}\nRequires --kill.", default_seconds(DEFAULT_KILL_TIMEOUT))
	)]
	pub kill_timeout: Option<Duration>,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct Reboot {
	#[arg(
		long,
		value_name = "SECONDS",
		value_parser = seconds,
		help = format!("Set the reboot deadline. {}", default_seconds(DEFAULT_REBOOT_TIMEOUT))
	)]
	pub timeout: Option<Duration>,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct Reconnect {
	#[arg(
		long,
		value_name = "SECONDS",
		value_parser = seconds,
		help = format!("Set the reboot deadline for DreamSeeker-hosted worlds. {}", default_seconds(DEFAULT_REBOOT_TIMEOUT))
	)]
	pub timeout: Option<Duration>,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct Log {
	#[arg(long, help = "Stream new entries after printing matching retained entries.")]
	pub follow: bool,
	#[arg(long, help = "Include runtime errors and thrown exceptions, with stacks.")]
	pub runtimes: bool,
	#[arg(long, help = "Include hot reload attempts, results, and diagnostics.")]
	pub reloads: bool,
	#[arg(long, help = "Include world output captured by the launcher.")]
	pub output: bool,
	#[arg(long, help = "Include world loads, reboots, and automatic debugger actions.")]
	pub world: bool,
	#[arg(long, help = "Include results of code run by eval --spawn.")]
	pub spawned: bool,
	#[arg(
		long,
		value_name = "FILTER",
		help = "Show only runtime errors whose messages match FILTER:\ncomma-separated substrings, any of which matches;\n! excludes one. \
		        Case-insensitive. Implies --runtimes."
	)]
	pub runtime_filter: Option<String>,
	#[arg(long, value_name = "N", value_parser = whole::<u64>, help = "Read entries with sequence numbers greater than N.")]
	pub since: Option<u64>,
	#[arg(long, value_name = "N", value_parser = whole::<u32>, help = "Limit initial output to the last N matching entries.")]
	pub last: Option<u32>,
	#[arg(long, help = "Emit JSON Lines, one journal entry per line.")]
	pub json: bool
}

#[derive(Args)]
pub struct Stack {
	#[arg(long, help = "Include sleeping proc stacks and their stable stack IDs.")]
	pub all: bool,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct Vars {
	#[arg(long, value_name = "N", value_parser = whole::<u32>, help = "Select a zero-based frame; 0 is innermost. [default: 0]")]
	pub frame: Option<u32>,
	#[arg(long, help = "Include global variables.")]
	pub globals: bool,
	#[arg(long, value_name = "N", value_parser = whole::<u32>, help = depth_help())]
	pub depth: Option<u32>,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct Eval {
	#[arg(value_name = "EXPRESSION", allow_negative_numbers = true)]
	pub expression: Vec<String>,
	#[arg(short = 'f', long, value_name = "FILE", help = "Read the expression from FILE.")]
	pub file: Option<PathBuf>,
	#[arg(long, value_name = "N", value_parser = whole::<u32>, help = "Select a zero-based paused frame. [default: 0 when paused]")]
	pub frame: Option<u32>,
	#[arg(
		long,
		value_name = "REF",
		help = "Evaluate outside any frame, with src set to the object whose\nref() is REF: [0x2000001] or 0x2000001."
	)]
	pub src: Option<String>,
	#[arg(
		long,
		help = "Run the code as the world's own on its next tick instead of\nevaluating it now. Report its spawn number and return."
	)]
	pub spawn: bool,
	#[arg(long, help = "Wait for spawned code to finish and report its value.\nRequires --spawn.")]
	pub wait: bool,
	#[arg(
		long,
		value_name = "SECONDS",
		value_parser = seconds,
		help = "Set the --wait deadline. Requires --wait. [default: unlimited]"
	)]
	pub timeout: Option<Duration>,
	#[arg(long, value_name = "N", value_parser = whole::<u32>, help = depth_help())]
	pub depth: Option<u32>,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct ProcInfo {
	#[arg(value_name = "/PATH[#N]")]
	pub proc: String,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct Disassemble {
	#[arg(value_name = "/PATH[#N]")]
	pub proc: String,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct Pause {
	#[arg(long, value_name = "SECONDS", value_parser = seconds, help = hold_max_help(HOLD_MAX))]
	pub hold_max: Option<Duration>,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct Continue {
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct StepOver {
	#[arg(
		long,
		help = "Step by one bytecode instruction, stepping over proc calls.\nInclude the resulting instruction offset and text."
	)]
	pub instruction: bool,
	#[arg(long, value_name = "SECONDS", value_parser = seconds, help = "Set the step deadline. [default: unlimited]")]
	pub timeout: Option<Duration>,
	#[arg(long, value_name = "SECONDS", value_parser = seconds, help = hold_max_help(HOLD_MAX))]
	pub hold_max: Option<Duration>,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct StepIn {
	#[arg(
		long,
		help = "Step by one bytecode instruction, stepping into proc calls.\nInclude the resulting instruction offset and text."
	)]
	pub instruction: bool,
	#[arg(long, value_name = "SECONDS", value_parser = seconds, help = "Set the step deadline. [default: unlimited]")]
	pub timeout: Option<Duration>,
	#[arg(long, value_name = "SECONDS", value_parser = seconds, help = hold_max_help(HOLD_MAX))]
	pub hold_max: Option<Duration>,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct StepOut {
	#[arg(long, value_name = "SECONDS", value_parser = seconds, help = "Set the step deadline. [default: unlimited]")]
	pub timeout: Option<Duration>,
	#[arg(long, value_name = "SECONDS", value_parser = seconds, help = hold_max_help(HOLD_MAX))]
	pub hold_max: Option<Duration>,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct Wait {
	#[command(subcommand)]
	pub event: Option<WaitFor>
}

#[derive(Subcommand)]
pub enum WaitFor {
	Pause(WaitPause),
	Reload(WaitReload),
	Reboot(WaitReboot)
}

#[derive(Args)]
pub struct WaitPause {
	#[arg(
		long = "break",
		value_name = "LOCATION",
		help = "Set a temporary breakpoint. Repeatable.\nLOCATION is FILE:LINE or /proc/path[#N].\n#N selects a zero-based definition. [default: 0]"
	)]
	pub breaks: Vec<String>,
	#[arg(long = "change", value_name = "EXPR", help = "Watch a variable or list entry for changes. Repeatable.")]
	pub changes: Vec<String>,
	#[arg(
		long,
		value_name = "N",
		value_parser = whole::<u32>,
		help = "Select the paused frame used to resolve watches.\n[default: 0, the innermost frame]"
	)]
	pub frame: Option<u32>,
	#[arg(
		long = "if",
		value_name = "CONDITION",
		help = "Apply a DM condition to watches, if present;\notherwise apply it to breakpoints.\nFor watches, old and value contain the previous \
		        and new values."
	)]
	pub condition: Option<String>,
	#[arg(long, help = "Pause on uncaught runtime errors and exceptions.")]
	pub runtimes: bool,
	#[arg(long, help = "Also pause on runtime errors inside try blocks.\nImplies --runtimes.")]
	pub caught: bool,
	#[arg(
		long,
		value_name = "FILTER",
		help = "Pause only on runtime errors whose messages match FILTER:\ncomma-separated substrings, any of which matches;\n! excludes one. \
		        Case-insensitive. Implies --runtimes."
	)]
	pub runtime_filter: Option<String>,
	#[arg(long, value_name = "SECONDS", value_parser = seconds, help = "Set the wait deadline. [default: unlimited]")]
	pub timeout: Option<Duration>,
	#[arg(long, help = "Do not retain the pause after this command disconnects.")]
	pub no_hold: bool,
	#[arg(
		long,
		value_name = "SECONDS",
		value_parser = seconds,
		help = hold_max_help("Limit the retained pause when no debugger is connected.")
	)]
	pub hold_max: Option<Duration>,
	#[arg(long, help = "Emit one JSON object with pause details.")]
	pub json: bool
}

#[derive(Args)]
pub struct WaitReload {
	#[arg(
		long,
		value_name = "N",
		value_parser = whole::<u64>,
		help = "Use the first reload event after journal entry N.\n[default: wait for a new event]"
	)]
	pub since: Option<u64>,
	#[arg(long, value_name = "SECONDS", value_parser = seconds, help = "Set the wait deadline. [default: unlimited]")]
	pub timeout: Option<Duration>,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct WaitReboot {
	#[arg(
		long,
		value_name = "N",
		value_parser = whole::<u64>,
		help = "Use the first world-load event after journal entry N.\n[default: wait for a new event]"
	)]
	pub since: Option<u64>,
	#[arg(long, value_name = "SECONDS", value_parser = seconds, help = "Set the wait deadline. [default: unlimited]")]
	pub timeout: Option<Duration>,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct Trace {
	#[arg(value_name = "LOCATION", required = true)]
	pub locations: Vec<String>,
	#[arg(
		long,
		value_name = "MESSAGE",
		help = "Format each hit using MESSAGE with {DM expression} values.\n{{ and }} are literal braces."
	)]
	pub log: Option<String>,
	#[arg(long = "if", value_name = "CONDITION", help = "Record only hits where the DM condition is true.")]
	pub condition: Option<String>,
	#[arg(long, value_name = "EXPR", help = "Group hits by expression value and print counts at exit.")]
	pub by: Option<String>,
	#[arg(long = "for", value_name = "SECONDS", value_parser = seconds, help = "Stop after the specified duration. [default: unlimited]")]
	pub duration: Option<Duration>,
	#[arg(long, value_name = "N", value_parser = positive, help = "Stop after N recorded hits. N must be positive.")]
	pub count: Option<u64>,
	#[arg(long, help = "Emit JSON Lines: bound, hit, and summary events.")]
	pub json: bool
}

#[derive(Args)]
pub struct Profile {
	#[arg(value_name = "/PATH[#N]")]
	pub procs: Vec<String>,
	#[arg(long = "for", value_name = "SECONDS", value_parser = seconds, help = "Stop after the specified duration. [default: unlimited]")]
	pub duration: Option<Duration>,
	#[arg(
		long,
		value_name = "N",
		value_parser = positive,
		default_value_t = DEFAULT_TOP,
		hide_default_value = true,
		help = format!("List the N slowest procs and lines. [default: {}]", DEFAULT_TOP)
	)]
	pub top: u64,
	#[arg(long, value_name = "FILE", help = "Write an HTML report to FILE.")]
	pub html: Option<PathBuf>,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct Check {
	#[arg(value_name = "DME")]
	pub dme: Option<PathBuf>,
	#[arg(short = 'D', value_name = "NAME[=VALUE]", help = "Set a build define (repeatable). Match the DMB build.")]
	pub defines: Vec<String>,
	#[arg(long, value_name = "VERSION", value_parser = byond_version, help = byond_version_help())]
	pub byond_version: Option<(u32, u32)>,
	#[arg(long, help = JSON)]
	pub json: bool
}

#[derive(Args)]
pub struct Help {
	#[arg(value_name = "COMMAND")]
	pub path: Vec<String>
}

#[derive(Args)]
pub struct Version {
	#[arg(long, help = JSON)]
	pub json: bool
}

pub const MANY_SECONDS: f64 = 1e9;

fn seconds(text: &str) -> Result<Duration, String> {
	match text.trim().parse::<f64>() {
		Ok(seconds) if seconds.is_finite() && seconds >= 0.0 => Ok(Duration::from_secs_f64(seconds.min(MANY_SECONDS))),
		_ => Err("expected a non-negative number of seconds".to_owned())
	}
}

fn whole<T: std::str::FromStr>(text: &str) -> Result<T, String> {
	text.parse().map_err(|_| "expected a non-negative integer".to_owned())
}

fn positive(text: &str) -> Result<u64, String> {
	match text.parse() {
		Ok(n) if n > 0 => Ok(n),
		_ => Err("expected a positive integer".to_owned())
	}
}

fn pid(text: &str) -> Result<u32, String> {
	text.parse().map_err(|_| "expected a process ID".to_owned())
}

fn port(text: &str) -> Result<u16, String> {
	text.parse().map_err(|_| "expected a port number from 0 to 65535".to_owned())
}

fn byond_version(text: &str) -> Result<(u32, u32), String> {
	text.split_once('.')
		.and_then(|(major, build)| Some((major.parse().ok()?, build.parse().ok()?)))
		.ok_or_else(|| "expected a BYOND build such as 516.1689".to_owned())
}

fn byond_version_help() -> String {
	let latest = dmasm::ByondVersion::LATEST;
	format!("Check for bugs affecting this BYOND build. [default: {}.{}]", latest.major, latest.build)
}

pub enum Unparsed {
	Help(String),
	Usage(String)
}

pub fn command() -> clap::Command {
	help::attach(Cli::command())
}

pub fn parse(args: &[String]) -> Result<Cli, Unparsed> {
	let given = std::iter::once("auxtools".to_owned()).chain(args.iter().cloned());
	let mut command = command();
	let matches = command
		.try_get_matches_from_mut(given)
		.map_err(|error| help::unparsed(&error, args, &command))?;
	Cli::from_arg_matches(&matches).map_err(|error| help::unparsed(&error, args, &command))
}

pub fn rules(command: &Command) -> Result<(), String> {
	let requires = |given: bool, option: &str, needed: bool, other: &str| match given && !needed {
		true => Err(format!("{} requires {}.", option, other)),
		false => Ok(())
	};
	let conflict = |a: bool, first: &str, b: bool, second: &str| match a && b {
		true => Err(format!("{} and {} cannot be used together.", first, second)),
		false => Ok(())
	};
	match command {
		Command::Launch(launch) => requires(launch.json, "--json", launch.detach || launch.detached, "--detach"),
		Command::Stop(stop) => requires(stop.kill_timeout.is_some(), "--kill-timeout", stop.kill, "--kill"),
		Command::Vars(vars) => requires(vars.depth.is_some(), "--depth", vars.json, "--json"),
		Command::Eval(eval) => {
			requires(eval.depth.is_some(), "--depth", eval.json, "--json")?;
			requires(eval.wait, "--wait", eval.spawn, "--spawn")?;
			requires(eval.timeout.is_some(), "--timeout", eval.wait, "--wait")?;
			conflict(eval.frame.is_some(), "--frame", eval.src.is_some(), "--src")?;
			conflict(eval.frame.is_some(), "--frame", eval.spawn, "--spawn")?;
			match (eval.expression.as_slice(), &eval.file) {
				([], None) => Err("eval requires an expression: EXPRESSION, --file FILE, or - for stdin.".to_owned()),
				([_, ..], Some(_)) => Err("EXPRESSION and --file cannot be used together.".to_owned()),
				_ => Ok(())
			}
		}
		Command::Wait(Wait {
			event: Some(WaitFor::Pause(wait))
		}) => {
			conflict(wait.no_hold, "--no-hold", wait.hold_max.is_some(), "--hold-max")?;
			requires(wait.frame.is_some(), "--frame", !wait.changes.is_empty(), "--change")?;
			requires(
				wait.condition.is_some(),
				"--if",
				!wait.changes.is_empty() || !wait.breaks.is_empty(),
				"--break or --change"
			)
		}
		Command::Trace(trace) => conflict(trace.by.is_some(), "--by", trace.log.is_some(), "--log"),
		_ => Ok(())
	}
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use super::{parse, rules, Cli, Command, Unparsed, WaitFor, DEFAULT_TOP};

	fn args(args: &[&str]) -> Vec<String> {
		args.iter().map(|arg| (*arg).to_owned()).collect()
	}

	fn parsed(given: &[&str]) -> Cli {
		match parse(&args(given)) {
			Ok(cli) => cli,
			Err(Unparsed::Usage(why)) => panic!("{:?}: {}", given, why),
			Err(Unparsed::Help(_)) => panic!("{:?} showed help", given)
		}
	}

	fn refused(given: &[&str]) -> String {
		match parse(&args(given)) {
			Ok(cli) => match cli.command.as_ref().map(rules) {
				Some(Err(why)) => why,
				_ => panic!("{:?} was taken", given)
			},
			Err(Unparsed::Usage(why)) => why,
			Err(Unparsed::Help(_)) => panic!("{:?} showed help", given)
		}
	}

	#[test]
	fn the_process_goes_before_the_command_or_after_it() {
		for given in [&["--pid", "1234", "status"][..], &["status", "--pid", "1234"], &[
			"status",
			"--pid=1234",
			"--json"
		]] {
			let cli = parsed(given);
			assert_eq!(cli.pid, Some(1234), "{:?}", given);
			assert!(matches!(cli.command, Some(Command::Status(_))));
		}
		assert_eq!(parsed(&["wait", "pause", "--pid", "7"]).pid, Some(7));
		assert_eq!(parsed(&["wait", "--pid", "7", "pause"]).pid, Some(7));
		assert_eq!(parsed(&["status"]).pid, None);
		for given in [&["--response-timeout", "2.5", "status"][..], &["status", "--response-timeout=2.5"]] {
			assert_eq!(parsed(given).response_timeout, Some(Duration::from_millis(2500)), "{:?}", given);
		}
		assert_eq!(parsed(&["status"]).response_timeout, None);
		assert_eq!(
			refused(&["--pid", "soon", "status"]),
			"invalid value 'soon' for --pid: expected a process ID. Run 'auxtools status --help' for usage."
		);
	}

	#[test]
	fn the_old_names_are_gone() {
		for (old, said) in [
			("next", "unknown command 'next'. Run 'auxtools --help' for available commands."),
			("dis", "unknown command 'dis'. Run 'auxtools --help' for available commands.")
		] {
			assert_eq!(refused(&[old]), said);
		}
		assert_eq!(
			refused(&["wait", "soon"]),
			"unknown wait command 'soon'. Run 'auxtools wait --help' for available commands."
		);
		assert_eq!(
			refused(&["wait", "--break", "a.dm:3"]),
			"unexpected argument '--break' before a wait command: specify pause, reload, or reboot. Run 'auxtools wait --help' for usage."
		);
		assert_eq!(
			refused(&["status", "--sideways"]),
			"unexpected argument '--sideways'. Run 'auxtools status --help' for usage."
		);
	}

	#[test]
	fn times_are_numbers_of_seconds_that_arent_negative() {
		let Some(Command::Stop(stop)) = parsed(&["stop", "--timeout", "1.5", "--kill", "--kill-timeout=0"]).command else {
			panic!("not stop");
		};
		assert_eq!(
			(stop.timeout, stop.kill_timeout),
			(Some(Duration::from_millis(1500)), Some(Duration::ZERO))
		);
		let Some(Command::Reboot(reboot)) = parsed(&["reboot", "--timeout", "1e300"]).command else {
			panic!("not reboot");
		};
		assert!(std::time::Instant::now().checked_add(reboot.timeout.unwrap()).is_some());
		for wrong in ["-1", "NaN", "inf", "soon"] {
			assert_eq!(
				refused(&["reboot", &format!("--timeout={}", wrong)]),
				format!(
					"invalid value '{}' for --timeout: expected a non-negative number of seconds. Run 'auxtools reboot --help' for usage.",
					wrong
				)
			);
		}
		assert_eq!(
			refused(&["stop", "--timeout"]),
			"--timeout requires a value: SECONDS. Run 'auxtools stop --help' for usage."
		);
	}

	#[test]
	fn options_that_need_another_say_so() {
		for (given, said) in [
			(&["vars", "--depth", "3"][..], "--depth requires --json."),
			(&["eval", "1", "--depth", "3"], "--depth requires --json."),
			(&["eval", "1", "--wait"], "--wait requires --spawn."),
			(&["eval", "1", "--spawn", "--timeout", "2"], "--timeout requires --wait."),
			(
				&["eval", "1", "--frame", "1", "--src", "0x1"],
				"--frame and --src cannot be used together."
			),
			(&["eval", "1", "--frame", "1", "--spawn"], "--frame and --spawn cannot be used together."),
			(&["eval"], "eval requires an expression: EXPRESSION, --file FILE, or - for stdin."),
			(&["eval", "1", "--file", "x.dm"], "EXPRESSION and --file cannot be used together."),
			(&["stop", "--kill-timeout", "2"], "--kill-timeout requires --kill."),
			(&["launch", "a.dmb", "--json"], "--json requires --detach."),
			(
				&["trace", "/proc/x", "--by", "caller", "--log", "hit"],
				"--by and --log cannot be used together."
			),
			(
				&["wait", "pause", "--no-hold", "--hold-max", "5"],
				"--no-hold and --hold-max cannot be used together."
			),
			(&["wait", "pause", "--frame", "1"], "--frame requires --change."),
			(&["wait", "pause", "--runtimes", "--if", "x"], "--if requires --break or --change.")
		] {
			assert_eq!(refused(given), said, "{:?}", given);
		}
		assert_eq!(
			refused(&["trace", "/proc/x", "--count", "0"]),
			"invalid value '0' for --count: expected a positive integer. Run 'auxtools trace --help' for usage."
		);
		assert_eq!(
			refused(&["trace"]),
			"missing required argument: LOCATION. Run 'auxtools trace --help' for usage."
		);
	}

	#[test]
	fn a_profile_is_of_every_proc_or_of_those_named() {
		let profile = |given: &[&str]| match parsed(given).command {
			Some(Command::Profile(profile)) => profile,
			_ => panic!("not profile")
		};
		let all = profile(&["profile"]);
		assert!(all.procs.is_empty() && all.duration.is_none() && all.top == DEFAULT_TOP && all.html.is_none() && !all.json);
		let some = profile(&[
			"profile",
			"/mob/proc/life",
			"/proc/tick#1",
			"--for",
			"2.5",
			"--top",
			"3",
			"--html",
			"out.html",
			"--json"
		]);
		assert_eq!(some.procs, args(&["/mob/proc/life", "/proc/tick#1"]));
		assert_eq!(some.duration, Some(Duration::from_millis(2500)));
		assert!(some.top == 3 && some.html.as_deref() == Some(std::path::Path::new("out.html")) && some.json);
		assert_eq!(
			refused(&["profile", "--top", "0"]),
			"invalid value '0' for --top: expected a positive integer. Run 'auxtools profile --help' for usage."
		);
	}

	#[test]
	fn an_expression_is_its_words_whatever_they_start_with() {
		let eval = |given: &[&str]| match parsed(given).command {
			Some(Command::Eval(eval)) => eval,
			_ => panic!("not eval")
		};
		assert_eq!(eval(&["eval", "6", "*", "7"]).expression, args(&["6", "*", "7"]));
		assert_eq!(eval(&["eval", "-"]).expression, args(&["-"]));
		assert_eq!(eval(&["eval", "-1"]).expression, args(&["-1"]));
		let after = eval(&["eval", "x", "+", "1", "--json", "--frame", "2"]);
		assert_eq!(after.expression, args(&["x", "+", "1"]));
		assert!(after.json && after.frame == Some(2));
		let negated = eval(&["eval", "--json", "--", "-x", "--y"]);
		assert_eq!(negated.expression, args(&["-x", "--y"]));
		assert!(negated.json);
		assert_eq!(
			refused(&["eval", "-x"]),
			"unexpected argument '-x'. Run 'auxtools eval --help' for usage."
		);
		let file = eval(&["eval", "-f", "x.dm"]);
		assert!(file.expression.is_empty() && file.file.is_some());
		assert!(eval(&["eval", "--file=x.dm"]).file.is_some());
	}

	#[test]
	fn launch_takes_defines_and_the_programs_arguments() {
		let Some(Command::Launch(launch)) = parsed(&[
			"launch",
			"a.dmb",
			"-DTESTING",
			"-D",
			"MAP=box",
			"--port",
			"1337",
			"--",
			"-trusted",
			"-verbose"
		])
		.command
		else {
			panic!("not launch");
		};
		assert_eq!(launch.defines, args(&["TESTING", "MAP=box"]));
		assert_eq!(launch.port, Some(1337));
		assert_eq!(launch.args, args(&["-trusted", "-verbose"]));
	}

	#[test]
	fn check_takes_a_project_its_defines_and_a_build() {
		let check = |given: &[&str]| match parsed(given).command {
			Some(Command::Check(check)) => check,
			_ => panic!("not check")
		};
		let bare = check(&["check"]);
		assert!(bare.dme.is_none() && bare.defines.is_empty() && bare.byond_version.is_none() && !bare.json);
		let full = check(&["check", "a.dme", "-DTESTING", "-D", "MAP=box", "--byond-version", "516.1689", "--json"]);
		assert_eq!(full.dme.as_deref(), Some(std::path::Path::new("a.dme")));
		assert_eq!(full.defines, args(&["TESTING", "MAP=box"]));
		assert_eq!(full.byond_version, Some((516, 1689)));
		assert!(full.json);
		for wrong in ["516", "516.", "516.x", "latest"] {
			assert_eq!(
				refused(&["check", "--byond-version", wrong]),
				format!(
					"invalid value '{}' for --byond-version: expected a BYOND build such as 516.1689. Run 'auxtools check --help' for usage.",
					wrong
				)
			);
		}
		assert_eq!(
			refused(&["check", "a.dme", "b.dme"]),
			"unexpected argument 'b.dme'. Run 'auxtools check --help' for usage."
		);
	}

	#[test]
	fn wait_names_what_its_for() {
		let wait = |given: &[&str]| match parsed(given).command {
			Some(Command::Wait(wait)) => wait.event,
			_ => panic!("not wait")
		};
		assert!(wait(&["wait"]).is_none());
		let Some(WaitFor::Pause(pause)) = wait(&["wait", "pause", "--break", "a.dm:3", "--break", "/proc/x", "--caught", "--timeout", "2"]) else {
			panic!("not a pause");
		};
		assert_eq!(pause.breaks, args(&["a.dm:3", "/proc/x"]));
		assert!(pause.caught && pause.timeout == Some(Duration::from_secs(2)));
		let Some(WaitFor::Pause(pause)) = wait(&["wait", "pause", "--runtime-filter", "!libdreamluau"]) else {
			panic!("not a pause");
		};
		assert_eq!(pause.runtime_filter.as_deref(), Some("!libdreamluau"));
		assert!(matches!(wait(&["wait", "reload", "--since", "12"]), Some(WaitFor::Reload(reload)) if reload.since == Some(12)));
		assert!(matches!(wait(&["wait", "reboot", "--json"]), Some(WaitFor::Reboot(reboot)) if reboot.json));
	}

	#[test]
	fn help_is_asked_for_in_the_usual_ways() {
		let shown = |given: &[&str]| match parse(&args(given)) {
			Err(Unparsed::Help(text)) => text,
			_ => panic!("{:?} didn't show help", given)
		};
		assert!(shown(&["--help"]).starts_with("auxtools - Launch, inspect, and debug BYOND worlds\n"));
		assert_eq!(shown(&["-h"]), shown(&["--help"]));
		assert!(shown(&["stop", "--help"]).starts_with("Request graceful shutdown and wait for the world process to exit.\n"));
		assert_eq!(shown(&["stop", "-h"]), shown(&["stop", "--help"]));
		assert!(shown(&["wait", "pause", "--help"]).starts_with("Wait for execution to pause and report the location and reason.\n"));
		assert!(shown(&["wait", "--help"]).starts_with("Wait for a debugger or world lifecycle event.\n"));
	}
}
