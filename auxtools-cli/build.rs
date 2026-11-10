//! On Windows the launcher is `auxtools.exe`, and the `auxtools` crate's own
//! library is `auxtools.dll`: the linker would keep the debug information of
//! both in `auxtools.pdb`, in the same directory, one over the other. The
//! launcher's goes in `auxtools-cli.pdb`, next to where the program is built.

use std::path::PathBuf;

fn main() {
	println!("cargo:rerun-if-changed=build.rs");
	if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
		return;
	}
	// OUT_DIR is somewhere under target/[TRIPLE/]PROFILE/build, and the
	// program is built in target/[TRIPLE/]PROFILE
	let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
	let profile = out.ancestors().find(|dir| dir.file_name().is_some_and(|name| name == "build")).and_then(|build| build.parent());
	if let Some(profile) = profile {
		println!("cargo:rustc-link-arg-bin=auxtools=/PDB:{}", profile.join("auxtools-cli.pdb").display());
	}
}
