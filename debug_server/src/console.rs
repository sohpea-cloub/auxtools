#[derive(Debug, PartialEq, Eq)]
pub enum Command<'a> {
	Disassemble { proc: Option<&'a str>, id: u32 },
	Skin,
	Script,
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

const SKIN_HELP: &str = "\
Show login skin resource ID and text

Usage: skin
";

const SCRIPT_HELP: &str = "\
Show login script string ID, resource IDs and text

Usage: script
";

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
		"skin" => at_most(&args, 0, "skin").map(|()| Command::Skin),
		"script" => at_most(&args, 0, "script").map(|()| Command::Script),
		"help" => {
			at_most(&args, 1, "help [COMMAND]")?;
			match args.first().copied() {
				None => Ok(Command::Help(HELP)),
				Some("disassemble" | "dis") => Ok(Command::Help(DISASSEMBLE_HELP)),
				Some("skin") => Ok(Command::Help(SKIN_HELP)),
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
		assert_eq!(parse("skin"), Ok(Command::Skin));
		assert_eq!(parse("script"), Ok(Command::Script));
	}

	#[test]
	fn help_lists_what_completion_offers() {
		let Ok(Command::Help(help)) = parse("help") else {
			panic!("help said {:?}", parse("help"));
		};
		assert!(help.contains("disassemble") && help.contains("help"), "{}", help);
		assert!(!help.contains("skin") && !help.contains("script"), "{}", help);
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
		assert_eq!(parse("skin x").unwrap_err(), "error: unexpected argument 'x' found\n\nUsage: skin\n");
	}
}
