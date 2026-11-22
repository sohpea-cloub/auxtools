use std::{
	ffi::{CStr, CString},
	process::ExitCode
};

use auxtools::{
	byond_scan::{resolve_recipe_with_trace, AddressRange, Module, Recipe},
	registry::{self, Platform}
};

fn main() -> ExitCode {
	let arguments: Vec<String> = std::env::args().skip(1).collect();
	match run(&arguments) {
		Ok(code) => code,
		Err(why) => {
			eprintln!("check_recipes: {why}");
			ExitCode::from(64)
		}
	}
}

fn run(arguments: &[String]) -> Result<ExitCode, String> {
	const USAGE: &str = "usage: check_recipes linux|windows VERSION FILE [NAME...]\n       check_recipes linux|windows mask FILE 'AA ?? BB' [call]";
	let [platform, version, file, rest @ ..] = arguments else {
		return Err(USAGE.to_owned());
	};
	let platform = match platform.as_str() {
		"linux" => Platform::Linux,
		"windows" => Platform::Windows,
		_ => return Err(format!("the platform is linux or windows, not {platform}\n{USAGE}"))
	};
	let bytes: &'static [u8] = std::fs::read(file).map_err(|err| format!("can't read {file}: {err}"))?.leak();
	let module = match platform {
		Platform::Linux => map_elf(bytes),
		Platform::Windows => map_pe(bytes)
	}
	.ok_or_else(|| {
		format!(
			"{file} isn't a 32-bit {}",
			if platform == Platform::Linux { "ELF library" } else { "PE library" }
		)
	})?;

	if version == "mask" {
		let [mask, rest @ ..] = rest else {
			return Err(USAGE.to_owned());
		};
		return matches(&module, mask, rest.first().is_some_and(|word| word == "call"));
	}
	let numbers = version
		.split_once('.')
		.and_then(|(major, build)| Some((major.parse().ok()?, build.parse().ok()?)));
	let Some((major, build)) = numbers else {
		return Err(format!("the version is like 516.1689, not {version}\n{USAGE}"));
	};
	Ok(check(&module, platform, (major, build), rest))
}

fn range(recipe: &Recipe) -> String {
	if recipe.versions.max == u32::MAX {
		format!("{}..", recipe.versions.min)
	} else {
		format!("{}..={}", recipe.versions.min, recipe.versions.max)
	}
}

fn check(module: &Module, platform: Platform, (major, build): (u32, u32), only: &[String]) -> ExitCode {
	let wanted = |name: &str| only.is_empty() || only.iter().any(|one| one == name);
	if let Some(unknown) = only.iter().find(|one| !registry::names(platform).contains(&one.as_str())) {
		eprintln!("check_recipes: the registry has no recipe called {unknown} for this platform");
		return ExitCode::from(64);
	}
	let label = format!("{major}.{build} {}", if platform == Platform::Linux { "linux" } else { "windows" });

	let mut matched = 0;
	for recipe in platform.recipes().iter().filter(|recipe| wanted(recipe.name)) {
		let within = recipe.versions.contains(build);
		let result = resolve_recipe_with_trace(module, recipe, |_| None);
		let (status, detail) = match (&result, within) {
			(Ok(found), true) => ("found", format!("0x{:08X} (at 0x{:08X})", found.value, found.extraction_address)),
			(Ok(found), false) => ("found outside its range", format!("0x{:08X}", found.value)),
			(Err(why), true) => ("NOT FOUND", why.to_string()),
			(Err(_), false) => ("-", String::new())
		};
		matched += usize::from(result.is_ok());
		println!(
			"{}",
			format!("{:<28} {:<12} {status:<24} {detail}", recipe.name, range(recipe)).trim_end()
		);
	}

	if !registry::accepts(major, build) {
		println!("{label}: auxtools refuses this build; {matched} of its recipes match it all the same");
		return ExitCode::from(2);
	}
	let (mut found, mut unchecked, mut failed) = (0, 0, 0);
	let names = registry::names(platform).into_iter().filter(|name| wanted(name));
	for name in names.filter(|name| registry::needed(platform, name, build)) {
		let recipe = registry::recipe_for(platform, name, build).expect("the name is one of the list's");
		match registry::resolve(module, platform, (major, build), name) {
			Ok(_) if recipe.versions.contains(build) => found += 1,
			Ok(_) => {
				unchecked += 1;
				println!("UNCHECKED {name}: no recipe's range has {build}; the one for {} matches", range(recipe));
			}
			Err(why) if recipe.versions.contains(build) => {
				failed += 1;
				println!("FAIL {name}: {why}");
			}
			Err(why) => {
				failed += 1;
				println!(
					"FAIL {name}: no recipe's range has {build}; the one for {} is tried: {why}",
					range(recipe)
				);
			}
		}
	}
	println!(
		"{label}: {} names needed, {found} found, {unchecked} found by a recipe for other builds, {failed} not found",
		found + unchecked + failed
	);
	if build > registry::builds::NEWEST_CHECKED {
		println!(
			"{label}: newer than the newest checked build, {}: resource reload and the text2path cache refuse it until \
			 `registry::builds::NEWEST_CHECKED` says it was checked",
			registry::builds::NEWEST_CHECKED
		);
	}
	ExitCode::from(u8::from(failed > 0))
}

fn matches(module: &Module, mask: &str, call: bool) -> Result<ExitCode, String> {
	let mut pattern = Vec::new();
	for byte in mask.split_whitespace() {
		pattern.push(match byte {
			"??" => None,
			_ => Some(u8::from_str_radix(byte, 16).map_err(|_| format!("{byte} in the mask is neither a hex byte nor ??"))?)
		});
	}
	if pattern.is_empty() {
		return Err("the mask is empty".to_owned());
	}
	let image = module.range();
	let mut count = 0;
	for at in 0..image.len().saturating_sub(pattern.len() - 1) {
		let address = module.range_start() + at;
		let same = pattern.iter().zip(&image[at..]).all(|(want, have)| want.is_none_or(|want| want == *have));
		if !same || !module.is_executable(address) {
			continue;
		}
		count += 1;
		if count > 20 {
			continue;
		}
		match word(image, at + 1) {
			Some(distance) if call => println!("0x{address:08X} -> call 0x{:08X}", (address + 5).wrapping_add(distance) as u32),
			_ => println!("0x{address:08X}")
		}
	}
	if count > 20 {
		println!("...");
	}
	println!("{count} matches");
	Ok(ExitCode::SUCCESS)
}

fn half(bytes: &[u8], at: usize) -> Option<usize> {
	Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as usize)
}

fn word(bytes: &[u8], at: usize) -> Option<usize> {
	Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize)
}

fn name_at(bytes: &[u8], at: usize) -> Option<&'static CStr> {
	let name = CStr::from_bytes_until_nul(bytes.get(at..)?).ok()?;
	Some(Box::leak(CString::from(name).into_boxed_c_str()))
}

fn module(start: usize, image: Vec<u8>, code: Vec<AddressRange>, data: Vec<AddressRange>, exports: Vec<(&'static CStr, usize)>) -> Module {
	Module::from_parts_with_ranges(0, start, image.leak(), code.leak(), data.leak()).with_exports(exports.leak())
}

fn map_elf(file: &[u8]) -> Option<Module> {
	const LOAD: usize = 1;
	const REL: usize = 9;
	const DYNSYM: usize = 11;
	if file.get(..6)? != b"\x7FELF\x01\x01" {
		return None;
	}
	let (programs, program_count) = (word(file, 0x1C)?, half(file, 0x2C)?);
	let (sections, section_count) = (word(file, 0x20)?, half(file, 0x30)?);
	let (mut code, mut data) = (Vec::new(), Vec::new());
	let mut image = Vec::new();
	for index in 0..program_count {
		let header = programs + index * 32;
		if word(file, header)? != LOAD {
			continue;
		}
		let (offset, address) = (word(file, header + 4)?, word(file, header + 8)?);
		let (file_size, size, flags) = (word(file, header + 16)?, word(file, header + 20)?, word(file, header + 24)?);
		if flags & 1 != 0 {
			code.push(AddressRange::new(address, file_size));
		}
		if flags & 2 != 0 {
			data.push(AddressRange::new(address, size));
		}
		if image.len() < address + size {
			image.resize(address + size, 0);
		}
		image[address..address + file_size].copy_from_slice(file.get(offset..offset + file_size)?);
	}
	let section = |index: usize| -> Option<(usize, usize, usize, usize)> {
		let header = sections + index * 40;
		Some((
			word(file, header + 4)?,
			word(file, header + 16)?,
			word(file, header + 20)?,
			word(file, header + 24)?
		))
	};
	for index in 0..section_count {
		let (kind, offset, size, link) = section(index)?;
		if kind != REL {
			continue;
		}
		let symbols = section(link)?.1;
		for entry in (offset..offset + size).step_by(8) {
			let (at, info) = (word(file, entry)?, word(file, entry + 4)?);
			let value = word(file, symbols + (info >> 8) * 16 + 4)? as u32;
			let Some(addend) = word(&image, at) else {
				continue;
			};
			let patched = match info & 0xFF {
				1 if value != 0 => value.wrapping_add(addend as u32),
				2 if value != 0 => value.wrapping_add(addend as u32).wrapping_sub(at as u32),
				_ => continue
			};
			image[at..at + 4].copy_from_slice(&patched.to_le_bytes());
		}
	}
	let mut exports = Vec::new();
	for index in 0..section_count {
		let (kind, offset, size, link) = section(index)?;
		if kind != DYNSYM {
			continue;
		}
		let strings = section(link)?.1;
		for entry in (offset..offset + size).step_by(16) {
			let (name, value) = (word(file, entry)?, word(file, entry + 4)?);
			if value != 0 {
				exports.push((name_at(file, strings + name)?, value));
			}
		}
	}
	Some(module(0, image, code, data, exports))
}

fn map_pe(file: &[u8]) -> Option<Module> {
	const EXECUTE: usize = 0x2000_0000;
	const WRITE: usize = 0x8000_0000;
	if file.get(..2)? != b"MZ" {
		return None;
	}
	let pe = word(file, 0x3C)?;
	let optional = pe + 24;
	if file.get(pe..pe + 4)? != b"PE\0\0" || half(file, optional)? != 0x10B {
		return None;
	}
	let (section_count, optional_size) = (half(file, pe + 6)?, half(file, pe + 20)?);
	let (base, size, headers) = (word(file, optional + 28)?, word(file, optional + 56)?, word(file, optional + 60)?);
	let mut image = vec![0u8; size];
	image.get_mut(..headers)?.copy_from_slice(file.get(..headers)?);
	let (mut code, mut data) = (Vec::new(), Vec::new());
	for index in 0..section_count {
		let header = optional + optional_size + index * 40;
		let (size, address) = (word(file, header + 8)?, word(file, header + 12)?);
		let (file_size, offset, flags) = (word(file, header + 16)?, word(file, header + 20)?, word(file, header + 36)?);
		let copied = file_size.min(image.len().checked_sub(address)?);
		image[address..address + copied].copy_from_slice(file.get(offset..offset + copied)?);
		let range = AddressRange::new(base + address, size.max(file_size));
		if flags & EXECUTE != 0 {
			code.push(range);
		}
		if flags & WRITE != 0 {
			data.push(range);
		}
	}
	let mut exports = Vec::new();
	let directory = word(file, optional + 96)?;
	if directory != 0 {
		let (count, functions) = (word(&image, directory + 24)?, word(&image, directory + 28)?);
		let (names, ordinals) = (word(&image, directory + 32)?, word(&image, directory + 36)?);
		for index in 0..count {
			let name = name_at(&image, word(&image, names + index * 4)?)?;
			let function = word(&image, functions + half(&image, ordinals + index * 2)? * 4)?;
			exports.push((name, base + function));
		}
	}
	Some(module(base, image, code, data, exports))
}
