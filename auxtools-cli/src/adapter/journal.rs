use std::sync::mpsc::RecvTimeoutError;

use super::*;

const OUTPUT_BATCH: Duration = Duration::from_millis(250);
const OUTPUT_BATCH_LINES: usize = 200;
const OUTPUT_WAITING: usize = 2000;

pub(super) fn batch_output(inputs: mpsc::Sender<Input>) -> mpsc::Sender<String> {
	let (lines, receiver) = mpsc::channel::<String>();
	std::thread::Builder::new()
		.name("output batches".to_owned())
		.spawn(move || {
			let mut batch = vec![];
			let mut due = Instant::now();
			loop {
				let line = if batch.is_empty() {
					receiver.recv().map_err(|_| RecvTimeoutError::Disconnected)
				} else {
					receiver.recv_timeout(due.saturating_duration_since(Instant::now()))
				};
				match line {
					Ok(line) => {
						if batch.is_empty() {
							due = Instant::now() + OUTPUT_BATCH;
						}
						batch.push(line);
						if batch.len() < OUTPUT_BATCH_LINES {
							continue;
						}
					}
					Err(RecvTimeoutError::Timeout) => {}
					Err(RecvTimeoutError::Disconnected) => {
						if !batch.is_empty() {
							let _ = inputs.send(Input::WorldOutput(batch));
						}
						return;
					}
				}
				if inputs.send(Input::WorldOutput(std::mem::take(&mut batch))).is_err() {
					return;
				}
			}
		})
		.expect("couldn't start a thread");
	lines
}

impl Adapter {
	pub(super) fn journal_output(&mut self, lines: Vec<String>) {
		self.waiting_output.extend(lines);
		let extra = self.waiting_output.len().saturating_sub(OUTPUT_WAITING);
		self.waiting_output.drain(..extra);
		self.send_output();
	}

	pub(super) fn send_output(&mut self) {
		if self.waiting_output.is_empty() {
			return;
		}
		let Some(debuggee) = &mut self.debuggee else {
			return;
		};
		let lines = std::mem::take(&mut self.waiting_output);
		let _ = debuggee.send(&AuxRequest::JournalAdd(aux::JournalEvent::Output { lines }));
	}

	pub(super) fn journal_reload(&mut self, applied: bool, summary: String, errors: Vec<String>, warnings: Vec<String>) {
		let Some(debuggee) = &mut self.debuggee else {
			return;
		};
		let _ = debuggee.send(&AuxRequest::JournalAdd(aux::JournalEvent::Reload {
			applied,
			summary,
			errors,
			warnings
		}));
	}

	pub(super) fn follow_reloads(&mut self) {
		if self.world.game.is_some() {
			return;
		}
		let _ = self.request(&AuxRequest::Journal {
			after: u64::MAX,
			kinds: vec![aux::JournalKind::Reload],
			limit: Some(0),
			follow: true
		});
	}
}

pub fn reload_by_another(entry: &aux::JournalEntry) -> Option<String> {
	let aux::JournalEvent::Reload {
		summary, errors, warnings, ..
	} = &entry.event
	else {
		return None;
	};
	let mut text = format!("Hot reload (by another launcher): {}", summary);
	for e in errors {
		text += &format!("\n    {}", e);
	}
	for w in warnings {
		text += &format!("\n    warning: {}", w);
	}
	Some(text)
}
