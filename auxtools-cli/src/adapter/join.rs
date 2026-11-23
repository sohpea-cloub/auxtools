use std::process::{Child, Command, Stdio};

use super::*;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorldEvent {
	address: Option<String>,
	started: bool,
	can_join: bool,
	hosted: bool
}

impl Event for WorldEvent {
	const EVENT: &'static str = "auxtoolsWorld";
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JoinArguments {
	#[serde(default = "yes")]
	open: bool,
	#[serde(default)]
	close_with_world: bool
}

#[derive(Default)]
pub(super) struct Joined {
	clients: Vec<(Child, bool)>,
	announced: Option<u32>
}

impl Joined {
	pub(super) fn world_ended(&mut self) {
		for (mut client, close) in std::mem::take(&mut self.clients) {
			if close {
				let _ = client.kill();
			}
			let _ = client.try_wait();
		}
	}
}

impl Adapter {
	fn address(&mut self) -> Result<Option<String>, String> {
		let port = match self.request(&AuxRequest::WorldInfo)? {
			AuxResponse::WorldInfo { port, .. } => port,
			other => return Err(unexpected(other))
		};
		Ok((port != 0).then(|| format!("byond://127.0.0.1:{}", port)))
	}

	fn hosted(&self) -> bool {
		self.world
			.launched
			.as_ref()
			.is_some_and(|launched| launched.server == Some(Server::DreamSeeker))
	}

	pub(super) fn announce_world(&mut self) {
		let process = self.world_process();
		let started = process != self.joined.announced;
		self.joined.announced = process;
		let Ok(address) = self.address() else {
			return;
		};
		let hosted = self.hosted();
		self.out.event(WorldEvent {
			can_join: cfg!(windows) && address.is_some(),
			address,
			started,
			hosted
		});
	}

	pub(super) fn join(&mut self, args: JoinArguments) -> Handled {
		let address = self
			.address()?
			.ok_or("the world has no port to join. Set \"port\" in the launch configuration")?;
		if !args.open || !cfg!(windows) {
			return Ok(serde_json::json!({ "address": address, "opened": false }));
		}
		let byond = self.world.launched.as_ref().and_then(|launched| launched.byond_path.clone());
		let program = byond::bin_dir(byond.as_deref())?.join(Server::DreamSeeker.program());
		let client = Command::new(&program)
			.arg(&address)
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.map_err(|e| format!("couldn't start {}: {}", program.display(), e))?;
		self.joined.clients.retain_mut(|(client, _)| matches!(client.try_wait(), Ok(None)));
		self.joined.clients.push((client, args.close_with_world));
		Ok(serde_json::json!({ "address": address, "opened": true }))
	}
}
