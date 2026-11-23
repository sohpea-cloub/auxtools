//! A miss is an error naming the lookup, never a null pointer: the C++ shim
//! and `debug.rs` use the results unchecked.

pub mod builds;
mod linux;
mod windows;

use byond_scan::{resolve_recipe, Module, Recipe};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
	Linux,
	Windows
}

impl Platform {
	pub const RUNNING: Platform = if cfg!(windows) { Platform::Windows } else { Platform::Linux };

	pub const fn recipes(self) -> &'static [Recipe] {
		match self {
			Platform::Linux => linux::RECIPES,
			Platform::Windows => windows::RECIPES
		}
	}
}

pub fn accepts(version: u32, build: u32) -> bool {
	builds::SUPPORTED.iter().any(|&(major, first)| major == version && build >= first)
}

/// Failing an exact range, the newest recipe before `build`, else the first.
pub fn recipe_for(platform: Platform, name: &str, build: u32) -> Option<&'static Recipe> {
	let named = || platform.recipes().iter().filter(move |recipe| recipe.name == name);
	named()
		.find(|recipe| recipe.versions.contains(build))
		.or_else(|| {
			named()
				.filter(|recipe| recipe.versions.max < build)
				.max_by_key(|recipe| recipe.versions.max)
		})
		.or_else(|| named().next())
}

pub fn names(platform: Platform) -> Vec<&'static str> {
	let mut names = Vec::new();
	for recipe in platform.recipes() {
		if !names.contains(&recipe.name) {
			names.push(recipe.name);
		}
	}
	names
}

/// From the name's first recipe until its last ends.
pub fn needed(platform: Platform, name: &str, build: u32) -> bool {
	let named = || platform.recipes().iter().filter(move |recipe| recipe.name == name);
	named().any(|recipe| recipe.versions.min <= build) && named().any(|recipe| build <= recipe.versions.max)
}

pub fn resolve(module: &Module, platform: Platform, (version, build): (u32, u32), name: &str) -> Result<usize, String> {
	let recipe = recipe_for(platform, name, build).ok_or_else(|| format!("Couldn't find {name} (no such pattern)"))?;
	resolve_checked(module, (version, build), recipe)
}

fn resolve_checked(module: &Module, (version, build): (u32, u32), recipe: &Recipe) -> Result<usize, String> {
	let name = recipe.name;

	if !accepts(version, build) || build < recipe.versions.min {
		let last = recipe.versions.max.min(builds::NEWEST_CHECKED);
		return Err(format!(
			"Couldn't find {name} (BYOND {version}.{build} is outside the checked range {}..={last})",
			recipe.versions.min
		));
	}

	resolve_recipe(module, recipe, |_| None).map_err(|err| format!("Couldn't find {name} ({err})"))
}

fn module_and_build() -> Result<(Module, (u32, u32)), String> {
	let module = Module::current().map_err(|err| format!("Couldn't locate BYOND core ({err})"))?;
	let build = byond_scan::version().map_err(|err| format!("Couldn't read BYOND version ({err})"))?;
	Ok((module, build))
}

/// [`find!`](crate::registry::find) also checks the name at compile time.
pub fn address(name: &str) -> Result<usize, String> {
	let (module, build) = module_and_build()?;
	resolve(&module, Platform::RUNNING, build, name)
}

pub const fn defines(name: &str) -> bool {
	byond_scan::defines(Platform::RUNNING.recipes(), name)
}

/// A name this platform's list lacks doesn't compile.
#[doc(hidden)]
#[macro_export]
macro_rules! __registry_find {
	($name:literal) => {{
		const _: () = assert!($crate::registry::defines($name), concat!("the registry has no recipe called ", $name));
		$crate::registry::address($name)
	}};
}
#[doc(inline)]
pub use __registry_find as find;

/// Builds older than the recipe's range are refused, newer ones tried.
pub fn find_recipe(recipe: &Recipe) -> Result<usize, String> {
	let (module, build) = module_and_build()?;
	resolve_checked(&module, build, recipe)
}

pub fn byond_build() -> Result<u32, String> {
	Ok(module_and_build()?.1 .1)
}

#[cfg(test)]
mod tests {
	use super::*;

	const PLATFORMS: [Platform; 2] = [Platform::Linux, Platform::Windows];

	#[test]
	fn a_name_has_one_recipe_a_build() {
		for platform in PLATFORMS {
			let recipes = platform.recipes();
			for (index, recipe) in recipes.iter().enumerate() {
				assert!(
					recipe.versions.min <= recipe.versions.max,
					"{platform:?} {} has an empty range",
					recipe.name
				);
				for other in &recipes[index + 1..] {
					let apart = recipe.versions.max < other.versions.min || other.versions.max < recipe.versions.min;
					assert!(
						recipe.name != other.name || apart,
						"{platform:?} has two recipes for {} on some build: {:?} and {:?}",
						recipe.name,
						recipe.versions,
						other.versions
					);
				}
			}
		}
	}

	#[test]
	fn a_names_newest_recipe_has_no_last_build() {
		const ENDED: [(Platform, &str); 1] = [(Platform::Linux, "misc_intern")];
		for platform in PLATFORMS {
			for recipe in platform.recipes() {
				let newest = platform
					.recipes()
					.iter()
					.filter(|other| other.name == recipe.name)
					.map(|other| other.versions.max)
					.max();
				assert!(
					newest == Some(u32::MAX) || ENDED.contains(&(platform, recipe.name)),
					"{platform:?} {} ends at {newest:?}",
					recipe.name
				);
			}
		}
	}

	#[test]
	fn a_name_is_needed_from_its_first_build_to_its_last() {
		let needed = |name, build| needed(Platform::Linux, name, build);
		assert!(!needed("alist_table_count", builds::FIRST_516 - 1) && needed("alist_table_count", 5000));
		assert!(needed("misc_intern", 1643) && !needed("misc_intern", 1644));
		assert!(!needed("register_typepath", 1640) && needed("register_typepath", 1641));
		assert!(needed("appearance_send", 1645));
		assert!(!names(Platform::Windows).contains(&"misc_intern") && names(Platform::Linux).contains(&"misc_intern"));
	}

	#[test]
	fn a_lookup_takes_the_recipe_for_the_build() {
		let range = |name, build| {
			let versions = recipe_for(Platform::Linux, name, build).unwrap().versions;
			(versions.min, versions.max)
		};
		assert_eq!(range("get_variable", 1637), (1637, 1643));
		assert_eq!(range("get_variable", 1644), (1644, 1647));
		assert_eq!(range("get_variable", 1648), (1648, u32::MAX));
		assert_eq!(range("get_variable", 5000), (1648, u32::MAX));
		assert_eq!(
			range("get_variable", 1500),
			(1648, u32::MAX),
			"a build older than every range gets the first recipe, which refuses it"
		);
		assert_eq!(range("appearance_send", 1643), (1637, 1643));
		assert_eq!(range("appearance_send", 1645), (1644, u32::MAX));
		assert_eq!(
			range("misc_intern", 1700),
			(1637, 1643),
			"a build past a name's last range gets that last recipe"
		);
		assert!(recipe_for(Platform::Linux, "no_such_name", 1648).is_none());
	}

	#[test]
	fn an_unsupported_build_is_refused_by_name() {
		let module = Module::from_parts(0, 0x1000, &[0x90]);
		let refused = |platform, version| resolve(&module, platform, version, "current_execution_context").unwrap_err();
		assert_eq!(
			refused(Platform::Windows, (515, 1636)),
			format!(
				"Couldn't find current_execution_context (BYOND 515.1636 is outside the checked range 1637..={})",
				builds::NEWEST_CHECKED
			)
		);
		assert_eq!(
			resolve(&module, Platform::Linux, (515, 1636), "misc_intern").unwrap_err(),
			"Couldn't find misc_intern (BYOND 515.1636 is outside the checked range 1637..=1643)"
		);
		assert!(refused(Platform::Windows, (516, 1640)).contains("BYOND 516.1640 is outside the checked range"));
		assert!(refused(Platform::Linux, (517, 1700)).contains("BYOND 517.1700 is outside the checked range"));
		assert_eq!(
			resolve(&module, Platform::Linux, (516, 1689), "get_variable").unwrap_err(),
			"Couldn't find get_variable (no match for: get_variable)"
		);
	}
}
