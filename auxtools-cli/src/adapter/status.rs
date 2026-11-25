use dmasm::reload::{Refusal, Until};

use super::*;

#[derive(serde::Serialize)]
struct ReloadEvent {
	state: &'static str,
	summary: Option<String>,
	problems: Vec<Problem>
}

impl Event for ReloadEvent {
	const EVENT: &'static str = "auxtoolsReload";
}

#[derive(serde::Serialize)]
struct Problem {
	file: Option<String>,
	line: Option<u32>,
	message: String,
	until: &'static str
}

#[derive(serde::Serialize)]
struct OutdatedEvent {
	files: Vec<String>
}

impl Event for OutdatedEvent {
	const EVENT: &'static str = "auxtoolsOutdated";
}

#[derive(Default)]
pub(super) struct ReloadStatus {
	reloading: bool,
	settled: Option<(&'static str, Option<String>)>,
	refusals: Vec<Refusal>
}

impl Adapter {
	pub(super) fn reload_state(&mut self, state: &'static str, summary: Option<String>) {
		self.reload_status.reloading = state == "reloading";
		if state != "reloading" {
			self.reload_status.settled = Some((state, summary.clone()));
		}
		let outstanding = !self.reload_status.refusals.is_empty();
		let shown = if outstanding && matches!(state, "active" | "reloaded") {
			"refused"
		} else {
			state
		};
		let dme = self.files.lock().unwrap().dme().map(|dme| dme.display().to_string());
		let problems = self
			.reload_status
			.refusals
			.iter()
			.map(|refusal| Problem {
				file: refusal.file.clone().or_else(|| dme.clone()),
				line: refusal.line,
				message: format!("{:#}", refusal),
				until: match refusal.until {
					Until::Reboot => "reboot",
					Until::Rebuild => "rebuild"
				}
			})
			.collect();
		self.out.event(ReloadEvent {
			state: shown,
			summary,
			problems
		});
	}

	pub(super) fn outstanding(&mut self, refusals: Vec<Refusal>) {
		self.reload_status.refusals = refusals;
		if !self.reload_status.reloading {
			if let Some((state, summary)) = self.reload_status.settled.clone() {
				self.reload_state(state, summary);
			}
		}
	}

	pub(super) fn forget_refusals(&mut self) {
		self.reload_status.refusals.clear();
	}

	pub(super) fn reloading(&mut self) {
		self.progress_start(RELOADING, "Reloading changes".to_owned());
		self.reload_state("reloading", None);
	}

	pub(super) fn looked(&mut self) {
		self.progress_end(RELOADING);
		if self.reload_status.reloading {
			if let Some((state, summary)) = self.reload_status.settled.clone() {
				self.reload_state(state, summary);
			}
		}
	}

	pub(super) fn refused(&mut self, refused: Vec<Refusal>, warnings: Vec<String>) {
		let reboot = refused.iter().all(|refusal| refusal.until == Until::Reboot);
		self.out.console(if reboot {
			"Hot reload requires a reboot:"
		} else {
			"Hot reload requires a rebuild:"
		});
		for group in grouped(&refused) {
			let text = format!("    {:#}", group);
			match &group.file {
				Some(file) => self.out.console_at(text, Path::new(file), group.line),
				None => self.out.console(text)
			}
		}
		self.journal_reload(
			false,
			if reboot { "reboot required" } else { "rebuild required" }.to_owned(),
			refused.iter().map(ToString::to_string).collect(),
			warnings
		);
	}

	pub(super) fn outdated(&mut self, files: Vec<String>) {
		let mut named = files.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
		if files.len() > 5 {
			named += &format!(" and {} more", files.len() - 5);
		}
		self.out
			.console(format!("Hot reload: files newer than the .dmb may have unapplied changes: {}", named));
		self.out.event(OutdatedEvent { files });
	}
}

fn grouped(refused: &[Refusal]) -> Vec<Refusal> {
	let mut groups: Vec<Refusal> = vec![];
	for refusal in refused {
		let same = groups.iter_mut().find(|group| {
			group.path == refusal.path && group.why == refusal.why && group.until == refusal.until && group.name.is_some() && refusal.name.is_some()
		});
		match (same, &refusal.name) {
			(Some(group), Some(name)) => {
				let names = group.name.as_mut().expect("grouped by name");
				if !names.split(", ").any(|known| known == name) {
					names.push_str(", ");
					names.push_str(name);
				}
			}
			_ => groups.push(refusal.clone())
		}
	}
	groups
}

#[cfg(test)]
mod tests {
	use dmasm::reload::Refusal;

	use super::grouped;

	#[test]
	fn refusals_of_one_kind_are_one() {
		let why = "cannot restore this built-in type's original default";
		let refused = vec![
			Refusal::new("/area", why).named("layer"),
			Refusal::new("/area", why).named("luminosity"),
			Refusal::new("/mob", why).named("layer"),
			Refusal::new("/area", why).named("icon"),
			Refusal::new("/area", "another reason").named("icon"),
			Refusal::new("/proc/f", "doesn't compile: x"),
		];
		let shown: Vec<String> = grouped(&refused).iter().map(|r| format!("{:#}", r)).collect();
		assert_eq!(shown, [
			format!("/area: layer, luminosity, icon: {}", why),
			format!("/mob: layer: {}", why),
			"/area: icon: another reason".to_owned(),
			"/proc/f: doesn't compile: x".to_owned()
		]);
	}
}
