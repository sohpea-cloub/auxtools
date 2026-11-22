//! What auxtools finds in BYOND's binary, as data.
//!
//! BYOND exports almost none of the functions and globals auxtools uses, so
//! each is found by a recipe (see `byond-scan`): a byte pattern or an export,
//! the calls to follow from it and what to read there. A recipe holds for a
//! range of builds, and a name has one recipe for each range whose code
//! differs. `registry/linux.rs` and `registry/windows.rs` list them, and both
//! lists are compiled on either platform. The builds where something changed
//! are named in `registry/builds.rs`. `examples/check_recipes.rs` checks the
//! lists against a BYOND binary on disk.
//!
//! Every lookup goes through [`resolve`]. A miss is an error that names the
//! lookup. Nothing here hands back a null pointer, because the C++ shim calls
//! these functions without checking and `debug.rs` reads the globals the same
//! way.

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

/// The recipe a lookup of `name` uses on `build`: the one whose range has the
/// build; failing that the newest one from before it, which is tried in case
/// it still matches; failing that the first, which the build is too old for.
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

/// Whether auxtools looks `name` up on `build`: a recipe of the name has begun
/// by then and the name's last hasn't ended. The callers ask by the same
/// boundaries the ranges are made of.
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

/// Finds `name` in the running BYOND. [`find!`](crate::registry::find) does
/// the same and checks the name when it is compiled.
pub fn address(name: &str) -> Result<usize, String> {
	let (module, build) = module_and_build()?;
	resolve(&module, Platform::RUNNING, build, name)
}

pub const fn defines(name: &str) -> bool {
	byond_scan::defines(Platform::RUNNING.recipes(), name)
}

/// The address of a function or global in the running BYOND, by its name in
/// the registry: `registry::find!("get_variable")?`. A name this platform's
/// list lacks does not compile.
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

/// Resolves a recipe of your own with the same rules auxtools applies to its
/// own: builds older than the recipe's range are refused, newer ones are tried.
pub fn find_recipe(recipe: &Recipe) -> Result<usize, String> {
	let (module, build) = module_and_build()?;
	resolve_checked(&module, build, recipe)
}

pub fn find_recipe_of(recipes: &[Recipe]) -> Result<usize, String> {
	let (module, build) = module_and_build()?;
	let recipe = recipes
		.iter()
		.find(|recipe| recipe.versions.contains(build.1))
		.or_else(|| {
			recipes
				.iter()
				.filter(|recipe| recipe.versions.max < build.1)
				.max_by_key(|recipe| recipe.versions.max)
		})
		.or(recipes.first())
		.ok_or("Couldn't find a pattern (none given)")?;
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
		assert_eq!(range("get_variable", 1500), (1648, u32::MAX));
		assert_eq!(range("appearance_send", 1645), (1637, 1643));
		assert_eq!(range("misc_intern", 1700), (1637, 1643));
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
