#[derive(Debug, PartialEq, Eq)]
pub enum Command<'a> {
	Disassemble {
		proc: Option<&'a str>,
		id: u32
	},
	#[cfg(feature = "test-commands")]
	Skin,
	#[cfg(feature = "test-commands")]
	Script,
	#[cfg(feature = "test-commands")]
	FailReload(crate::hot_reload::FailAt),
	#[cfg(feature = "test-commands")]
	StringReferences(&'a str),
	#[cfg(feature = "test-commands")]
	LocalsReferences(&'a str),
	#[cfg(feature = "test-commands")]
	DeclaredReferences(&'a str),
	#[cfg(feature = "test-commands")]
	NextSpawn(u64),
	#[cfg(feature = "test-commands")]
	InList {
		proc: &'a str,
		param: &'a str
	},
	Help(&'static str)
}

const HELP: &str = "\
Usage: #<SUBCOMMAND>

Commands:
  disassemble  Disassembles a proc and displays its bytecode in an assembly-like format
  help         Print this message or the help of the given subcommand(s)
";

const DISASSEMBLE_USAGE: &str = "disassemble [proc] [id]";

const DISASSEMBLE_HELP: &str = "\
Disassembles a proc and displays its bytecode in an assembly-like format

Usage: disassemble [proc] [id]

Arguments:
  [proc]  Path of the proc to disassemble (e.g. /proc/do_stuff)
  [id]    Id of the proc to disassemble (for when multiple procs are defined with the same path)

If no parameters are provided, the proc executing in the currently debugged stack frame will be disassembled
";

#[cfg(feature = "test-commands")]
const SKIN_HELP: &str = "\
Show login skin resource ID and text

Usage: skin
";

#[cfg(feature = "test-commands")]
const SCRIPT_HELP: &str = "\
Show login script string ID, resource IDs and text

Usage: script
";

#[cfg(feature = "test-commands")]
const STRING_REFS_USAGE: &str = "string-refs <text>";

#[cfg(feature = "test-commands")]
const LOCALS_REFS_USAGE: &str = "locals-refs <proc>";

#[cfg(feature = "test-commands")]
const DECLARED_REFS_USAGE: &str = "declared-refs <type>";

#[cfg(feature = "test-commands")]
const FAIL_RELOAD_USAGE: &str = "fail-reload <append|change|copy>";

#[cfg(feature = "test-commands")]
const NEXT_SPAWN_USAGE: &str = "next-spawn <number>";

#[cfg(feature = "test-commands")]
const IN_LIST_USAGE: &str = "in-list <proc> <parameter>";

const HELP_HELP: &str = "\
Print this message or the help of the given subcommand(s)

Usage: help [COMMAND]
";

fn unknown(name: &str) -> String {
	format!(
		"error: unrecognized subcommand '{}'\n\nUsage: #<SUBCOMMAND>\n\nFor more information, try '#help'.\n",
		name
	)
}

fn at_most(args: &[&str], most: usize, usage: &str) -> Result<(), String> {
	match args.get(most) {
		Some(extra) => Err(format!("error: unexpected argument '{}' found\n\nUsage: {}\n", extra, usage)),
		None => Ok(())
	}
}

pub fn parse(command: &str) -> Result<Command<'_>, String> {
	let mut words = command.split_ascii_whitespace();
	let Some(name) = words.next() else {
		return Err("error: a subcommand is required\n\nUsage: #<SUBCOMMAND>\n\nFor more information, try '#help'.\n".to_owned());
	};
	let args: Vec<&str> = words.collect();
	match name {
		"disassemble" | "dis" => {
			at_most(&args, 2, DISASSEMBLE_USAGE)?;
			Ok(Command::Disassemble {
				proc: args.first().copied(),
				id: args.get(1).and_then(|id| id.parse().ok()).unwrap_or(0)
			})
		}
		#[cfg(feature = "test-commands")]
		"skin" => at_most(&args, 0, "skin").map(|()| Command::Skin),
		#[cfg(feature = "test-commands")]
		"script" => at_most(&args, 0, "script").map(|()| Command::Script),
		#[cfg(feature = "test-commands")]
		"fail-reload" => {
			at_most(&args, 1, FAIL_RELOAD_USAGE)?;
			match args.first().copied() {
				Some("append") => Ok(Command::FailReload(crate::hot_reload::FailAt::Append)),
				Some("change") => Ok(Command::FailReload(crate::hot_reload::FailAt::Change)),
				Some("copy") => Ok(Command::FailReload(crate::hot_reload::FailAt::Copy)),
				_ => Err(format!(
					"error: a step to fail is required: append, change or copy\n\nUsage: {}\n",
					FAIL_RELOAD_USAGE
				))
			}
		}
		#[cfg(feature = "test-commands")]
		"string-refs" => {
			at_most(&args, 1, STRING_REFS_USAGE)?;
			match args.first() {
				Some(text) => Ok(Command::StringReferences(text)),
				None => Err(format!("error: a string is required\n\nUsage: {}\n", STRING_REFS_USAGE))
			}
		}
		#[cfg(feature = "test-commands")]
		"locals-refs" => {
			at_most(&args, 1, LOCALS_REFS_USAGE)?;
			match args.first() {
				Some(proc) => Ok(Command::LocalsReferences(proc)),
				None => Err(format!("error: a proc is required\n\nUsage: {}\n", LOCALS_REFS_USAGE))
			}
		}
		#[cfg(feature = "test-commands")]
		"declared-refs" => {
			at_most(&args, 1, DECLARED_REFS_USAGE)?;
			match args.first() {
				Some(path) => Ok(Command::DeclaredReferences(path)),
				None => Err(format!("error: a type is required\n\nUsage: {}\n", DECLARED_REFS_USAGE))
			}
		}
		#[cfg(feature = "test-commands")]
		"next-spawn" => {
			at_most(&args, 1, NEXT_SPAWN_USAGE)?;
			match args.first().map(|number| number.parse()) {
				Some(Ok(number)) => Ok(Command::NextSpawn(number)),
				_ => Err(format!("error: a spawn number is required\n\nUsage: {}\n", NEXT_SPAWN_USAGE))
			}
		}
		#[cfg(feature = "test-commands")]
		"in-list" => {
			at_most(&args, 2, IN_LIST_USAGE)?;
			match args[..] {
				[proc, param] => Ok(Command::InList { proc, param }),
				_ => Err(format!("error: a proc and a parameter are required\n\nUsage: {}\n", IN_LIST_USAGE))
			}
		}
		"help" => {
			at_most(&args, 1, "help [COMMAND]")?;
			match args.first().copied() {
				None => Ok(Command::Help(HELP)),
				Some("disassemble" | "dis") => Ok(Command::Help(DISASSEMBLE_HELP)),
				#[cfg(feature = "test-commands")]
				Some("skin") => Ok(Command::Help(SKIN_HELP)),
				#[cfg(feature = "test-commands")]
				Some("script") => Ok(Command::Help(SCRIPT_HELP)),
				Some("help") => Ok(Command::Help(HELP_HELP)),
				Some(other) => Err(unknown(other))
			}
		}
		other => Err(unknown(other))
	}
}

#[cfg(test)]
mod tests {
	use super::{parse, Command};

	#[test]
	fn commands_and_their_arguments() {
		assert_eq!(parse("dis"), Ok(Command::Disassemble { proc: None, id: 0 }));
		assert_eq!(
			parse("disassemble /proc/leaf"),
			Ok(Command::Disassemble {
				proc: Some("/proc/leaf"),
				id: 0
			})
		);
		assert_eq!(
			parse("  dis\t/proc/leaf  2 "),
			Ok(Command::Disassemble {
				proc: Some("/proc/leaf"),
				id: 2
			})
		);
		assert_eq!(
			parse("dis /proc/leaf two"),
			Ok(Command::Disassemble {
				proc: Some("/proc/leaf"),
				id: 0
			})
		);
		#[cfg(feature = "test-commands")]
		assert_eq!((parse("skin"), parse("script")), (Ok(Command::Skin), Ok(Command::Script)));
		#[cfg(feature = "test-commands")]
		assert_eq!(parse("fail-reload append"), Ok(Command::FailReload(crate::hot_reload::FailAt::Append)));
		#[cfg(feature = "test-commands")]
		assert_eq!(parse("string-refs key"), Ok(Command::StringReferences("key")));
		#[cfg(feature = "test-commands")]
		assert_eq!(parse("locals-refs /proc/leaf"), Ok(Command::LocalsReferences("/proc/leaf")));
		#[cfg(feature = "test-commands")]
		assert_eq!(parse("declared-refs /datum/a"), Ok(Command::DeclaredReferences("/datum/a")));
		#[cfg(feature = "test-commands")]
		assert_eq!(parse("next-spawn 16777217"), Ok(Command::NextSpawn(16777217)));
		#[cfg(feature = "test-commands")]
		assert_eq!(
			parse("in-list /mob/verb/choose state"),
			Ok(Command::InList {
				proc: "/mob/verb/choose",
				param: "state"
			})
		);
		#[cfg(not(feature = "test-commands"))]
		assert!(parse("skin").is_err() && parse("script").is_err() && parse("help skin").is_err() && parse("fail-reload append").is_err());
	}

	#[test]
	fn help_lists_what_completion_offers() {
		let Ok(Command::Help(help)) = parse("help") else {
			panic!("help said {:?}", parse("help"));
		};
		assert!(help.contains("disassemble") && help.contains("help"), "{}", help);
		assert!(!help.contains("skin") && !help.contains("script") && !help.contains("fail"), "{}", help);
		assert_eq!(parse("help dis"), parse("help disassemble"));
		assert!(matches!(parse("help dis"), Ok(Command::Help(text)) if text.contains("[proc]")));
	}

	#[test]
	fn errors_name_what_was_wrong() {
		assert_eq!(
			parse("bogus").unwrap_err(),
			"error: unrecognized subcommand 'bogus'\n\nUsage: #<SUBCOMMAND>\n\nFor more information, try '#help'.\n"
		);
		assert_eq!(parse("help bogus"), parse("bogus"));
		assert!(parse("").unwrap_err().starts_with("error: a subcommand is required\n"));
		assert_eq!(
			parse("dis a 1 c").unwrap_err(),
			"error: unexpected argument 'c' found\n\nUsage: disassemble [proc] [id]\n"
		);
		assert_eq!(
			parse("help x y").unwrap_err(),
			"error: unexpected argument 'y' found\n\nUsage: help [COMMAND]\n"
		);
		#[cfg(feature = "test-commands")]
		assert_eq!(parse("skin x").unwrap_err(), "error: unexpected argument 'x' found\n\nUsage: skin\n");
		#[cfg(feature = "test-commands")]
		assert_eq!(
			parse("fail-reload").unwrap_err(),
			"error: a step to fail is required: append, change or copy\n\nUsage: fail-reload <append|change|copy>\n"
		);
	}
}
