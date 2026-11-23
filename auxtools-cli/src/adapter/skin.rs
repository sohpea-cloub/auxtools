use super::*;

#[derive(serde::Serialize)]
struct SkinEvent {
	file: String,
	clients: u32,
	hosted: bool
}

impl Event for SkinEvent {
	const EVENT: &'static str = "auxtoolsSkin";
}

impl Adapter {
	pub(super) fn reload_skin(&mut self, request: aux::SkinRequest, file: &Path, images: &[String], restored: bool) -> bool {
		let file = file.display().to_string();
		let result = match self.request(&AuxRequest::Skin(request)) {
			Ok(AuxResponse::Skin(result)) => result,
			Ok(other) => Err(unexpected(other)),
			Err(e) => Err(e)
		};
		let reloaded = match result {
			Ok(reloaded) => reloaded,
			Err(e) => {
				self.out.console(format!("Hot reload failed: skin {}: {}", file, e));
				self.journal_reload(false, format!("skin {} not reloaded", file), vec![e], vec![]);
				return false;
			}
		};

		if restored {
			self.journal_reload(true, format!("skin {} restored", file), vec![], vec![]);
			return true;
		}
		let mut notes: Vec<String> = self.waiting_clients(reloaded.clients, reloaded.hosted, "skin").into_iter().collect();
		if !images.is_empty() {
			notes.push(format!(
				"{} {} new to the skin and need{} a rebuild.",
				images.iter().map(|name| format!("'{}'", name)).collect::<Vec<_>>().join(", "),
				if images.len() == 1 { "is" } else { "are" },
				if images.len() == 1 { "s" } else { "" }
			));
		}
		self.out.console(format!(
			"Hot reload: skin {} updated.{}",
			file,
			notes.iter().map(|n| format!(" {}", n)).collect::<String>()
		));
		self.journal_reload(true, format!("skin {}", file), vec![], notes);
		self.out.event(SkinEvent {
			file,
			clients: reloaded.clients,
			hosted: reloaded.hosted
		});
		true
	}

	pub(super) fn waiting_clients(&self, clients: u32, hosted: bool, what: &str) -> Option<String> {
		let how = if self.out.to_client() { "" } else { " Run auxtools reconnect." };
		match (clients, hosted) {
			(0, _) => None,
			(_, true) => Some(format!("DreamSeeker hosts the world. Reboot it to load the new {}.{}", what, how)),
			(1, false) => Some(format!("1 connected client needs to reconnect to load the new {}.{}", what, how)),
			(_, false) => Some(format!(
				"{} connected clients need to reconnect to load the new {}.{}",
				clients, what, how
			))
		}
	}

	pub(super) fn reconnect_clients(&mut self) -> Handled {
		match self.request(&AuxRequest::Reconnect)? {
			AuxResponse::Reconnect(Ok(aux::Reconnected::Clients(clients))) => Ok(serde_json::json!({ "reconnected": clients })),
			AuxResponse::Reconnect(Ok(aux::Reconnected::Hosted)) => {
				self.reboot()?;
				Ok(serde_json::json!({ "rebooted": true }))
			}
			AuxResponse::Reconnect(Err(e)) => Err(e),
			other => Err(unexpected(other))
		}
	}
}
