use auxtools::appearance_tables;
use dmasm::assembler::AssembleEnv as _;

use auxtools_protocol::{HotReloadResource, ReloadedResource};

use crate::assemble_env::{AssembleEnv, RESOURCE_DIRS};

pub struct Prepared {
	path: String,
	id: u32,
	replaces: Vec<u32>
}

pub fn use_dirs(dirs: &[String]) {
	if let Ok(mut current) = RESOURCE_DIRS.lock() {
		*current = dirs.to_vec();
	}
}

pub fn prepare(resources: &[HotReloadResource]) -> Result<Vec<Prepared>, Vec<String>> {
	if resources.is_empty() {
		return Ok(vec![]);
	}
	(unsafe { appearance_tables::check() }).map_err(|e| vec![e])?;
	let mut prepared = vec![];
	let mut errors = vec![];
	for resource in resources {
		match AssembleEnv.get_resource_index(&resource.path) {
			Some(id) => prepared.push(Prepared {
				path: resource.path.clone(),
				id,
				replaces: resource.ids.iter().copied().filter(|&old| old != id).collect()
			}),
			None => errors.push(format!("resource reload: couldn't read {}", resource.path))
		}
	}
	if errors.is_empty() {
		Ok(prepared)
	} else {
		Err(errors)
	}
}

pub struct Applied {
	pub reloaded: Vec<ReloadedResource>,
	pub appearances: Vec<u32>,
	pub warnings: Vec<String>
}

pub fn apply(prepared: &[Prepared]) -> Result<Applied, String> {
	if prepared.is_empty() {
		return Ok(Applied {
			reloaded: vec![],
			appearances: vec![],
			warnings: vec![]
		});
	}
	let mappings: Vec<(u32, u32)> = prepared
		.iter()
		.flat_map(|resource| resource.replaces.iter().map(|&old| (old, resource.id)))
		.collect();
	let repointed = unsafe { appearance_tables::repoint(&mappings) }?;
	let reloaded = prepared
		.iter()
		.map(|resource| ReloadedResource {
			path: resource.path.clone(),
			id: resource.id,
			appearances: repointed.counts.get(&resource.id).copied().unwrap_or(0)
		})
		.collect();
	let mut appearances = repointed.appearances;
	appearances.sort_unstable();
	appearances.dedup();
	let warnings: Vec<String> = repointed.warnings.into_iter().map(|w| format!("resource reload: {}", w)).collect();
	Ok(Applied {
		reloaded,
		appearances,
		warnings
	})
}

pub fn resend(appearances: &[u32]) -> Result<usize, String> {
	unsafe { appearance_tables::resend(appearances) }
}
