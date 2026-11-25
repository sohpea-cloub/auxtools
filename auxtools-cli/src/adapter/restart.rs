use super::*;

const REBOOT_PATIENCE: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
pub(super) struct RestartArguments {
	arguments: Option<Value>
}

impl Adapter {
	pub(super) fn restart(&mut self, args: RestartArguments) -> Handled {
		if self.ending {
			return Err("the world is already stopping".to_owned());
		}
		let Some(launched) = self.launched.clone() else {
			return self.reboot();
		};
		let mut wanted = match args.arguments {
			Some(config) => arguments::<LaunchArguments>(Some(config))?,
			None => launched.clone()
		};
		if wanted.build_flags == self.configured_flags {
			wanted.build_flags = launched.build_flags.clone();
		} else {
			self.configured_flags = wanted.build_flags.clone();
		}
		if wanted == launched && self.debuggee.is_some() && !self.unavailable {
			return self.reboot();
		}
		let prepared = Prepared::of(&wanted)?;
		self.relaunching = Some((wanted, prepared));
		self.progress_start(RESTARTING, "Restarting the world".to_owned());
		self.terminate()
	}

	pub(super) fn reboot(&mut self) -> Handled {
		if self.ending {
			return Err("the world is already stopping".to_owned());
		}
		self.debuggee()?.eval_unanswered("world.Reboot()".to_owned())?;
		self.rebooting = true;
		self.out.console("Rebooting the world");
		self.progress_start(RESTARTING, "Rebooting the world".to_owned());
		let (inputs, generation) = (self.inputs.clone(), self.debuggee()?.generation);
		std::thread::spawn(move || {
			std::thread::sleep(REBOOT_PATIENCE);
			let _ = inputs.send(Input::RebootOverdue { generation });
		});
		Ok(Value::Null)
	}

	pub(super) fn reboot_overdue(&mut self, generation: u64) {
		if self.rebooting && self.debuggee.as_ref().is_some_and(|d| d.generation == generation) {
			self.rebooting = false;
			self.progress_end(RESTARTING);
			self.out.console(format!(
				"The world hasn't rebooted after {} seconds: its world/Reboot() may be waiting, or have declined",
				REBOOT_PATIENCE.as_secs()
			));
		}
	}

	pub(super) fn relaunched(&mut self, args: LaunchArguments, prepared: Prepared) {
		if let Some(mut debuggee) = self.debuggee.take() {
			debuggee.gone();
		}
		self.joined.world_ended();
		self.pid = None;
		self.ending = false;
		self.unavailable = false;
		self.shutting_down = false;
		self.waiting_for_code = false;
		self.set.lock().unwrap().clear();
		self.stddef = None;
		if let Err(e) = self.start(args, prepared) {
			self.out.console(format!("Could not restart the world: {}", e));
			self.ended(-1);
		}
	}
}
