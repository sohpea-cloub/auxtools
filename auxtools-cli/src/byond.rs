use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
pub enum Server {
	#[serde(rename = "dreamseeker")]
	DreamSeeker,
	#[serde(rename = "dreamdaemon", alias = "dd")]
	DreamDaemon,
	#[serde(rename = "dreamdaemon-gui")]
	DreamDaemonGui
}

impl Server {
	pub const fn program(self) -> &'static str {
		if cfg!(windows) {
			match self {
				Self::DreamSeeker => "dreamseeker.exe",
				Self::DreamDaemon => "dd.exe",
				Self::DreamDaemonGui => "dreamdaemon.exe"
			}
		} else {
			"DreamDaemon"
		}
	}

	pub const fn default_for_platform() -> Self {
		Self::DreamDaemon
	}
}

const fn compiler() -> &'static str {
	if cfg!(windows) {
		"dm.exe"
	} else {
		"DreamMaker"
	}
}

pub fn bin_dir(byond_path: Option<&Path>) -> Result<PathBuf, String> {
	let candidates: Vec<PathBuf> = match byond_path {
		Some(path) => vec![path.to_owned()],
		None => usual_install_dirs()
	};

	for dir in &candidates {
		for bin in [dir.join("bin"), dir.to_owned()] {
			if bin.join(compiler()).is_file() || bin.join(Server::default_for_platform().program()).is_file() {
				return Ok(bin);
			}
		}
	}

	match byond_path {
		Some(path) => Err(format!("byondPath {} has no BYOND installation", path.display())),
		None => Err("BYOND not found. Set byondPath to its installation directory".to_owned())
	}
}

fn usual_install_dirs() -> Vec<PathBuf> {
	let mut dirs = vec![];

	if let Some(system) = std::env::var_os("BYOND_SYSTEM") {
		dirs.push(PathBuf::from(system));
	}

	#[cfg(windows)]
	{
		dirs.extend(registry::install_path());
		for var in ["ProgramFiles(x86)", "ProgramFiles"] {
			if let Some(dir) = std::env::var_os(var) {
				dirs.push(PathBuf::from(dir).join("BYOND"));
			}
		}
	}

	#[cfg(unix)]
	{
		if let Some(home) = std::env::var_os("HOME") {
			dirs.push(PathBuf::from(home).join("byond"));
		}
		dirs.push(PathBuf::from("/usr/local/byond"));
		dirs.push(PathBuf::from("/usr/share/byond"));
	}

	dirs
}

#[cfg(windows)]
mod registry {
	use std::{os::windows::ffi::OsStringExt, path::PathBuf, ptr::null_mut};

	use winapi::{
		shared::{minwindef::HKEY, winerror::ERROR_SUCCESS},
		um::winreg::{RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ}
	};

	pub fn install_path() -> Vec<PathBuf> {
		[HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER]
			.into_iter()
			.filter_map(|root| read(root, "SOFTWARE\\Dantom\\BYOND", "installpath"))
			.collect()
	}

	fn read(root: HKEY, key: &str, value: &str) -> Option<PathBuf> {
		let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
		let (key, value) = (wide(key), wide(value));
		let mut buf = vec![0u16; 1024];
		let mut size = (buf.len() * 2) as u32;
		let status = unsafe {
			RegGetValueW(
				root,
				key.as_ptr(),
				value.as_ptr(),
				RRF_RT_REG_SZ,
				null_mut(),
				buf.as_mut_ptr() as _,
				&mut size
			)
		};
		if status != ERROR_SUCCESS as i32 {
			return None;
		}
		let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
		Some(PathBuf::from(std::ffi::OsString::from_wide(&buf[..len])))
	}
}
