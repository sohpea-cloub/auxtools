//! What the launcher puts in the world's record of what happened in it, which
//! the world can't see for itself: what came of each hot reload, compile
//! errors included, and what the world printed, for a world the launcher
//! started and reads the output of. A launcher that attached to a world shows
//! the hot reloads that whoever reloads it reports.

use std::sync::mpsc::RecvTimeoutError;

use super::*;

/// How long the world's output is gathered before it's passed on: a world
/// that prints a lot sends a message every so often, not one a line
const OUTPUT_BATCH: Duration = Duration::from_millis(250);
/// The most lines a message has
const OUTPUT_BATCH_LINES: usize = 200;
/// How many lines are kept for the world while the launcher isn't connected
/// to it (it's loading, or rebooting)
const OUTPUT_WAITING: usize = 2000;

/// Gathers the lines sent to it and passes them on as `Input::WorldOutput`
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
	/// Lines the world printed, for its record: sent now, or once the world
	/// is connected
	pub(super) fn journal_output(&mut self, lines: Vec<String>) {
		self.waiting_output.extend(lines);
		let extra = self.waiting_output.len().saturating_sub(OUTPUT_WAITING);
		self.waiting_output.drain(..extra);
		self.send_output();
	}

	/// Sends the world's record the output that's waiting, if it's connected
	pub(super) fn send_output(&mut self) {
		if self.waiting_output.is_empty() {
			return;
		}
		let Some(debuggee) = &mut self.debuggee else {
			return;
		};
		let lines = std::mem::take(&mut self.waiting_output);
		// It has no answer
		let _ = debuggee.send(&AuxRequest::JournalAdd(aux::JournalEvent::Output { lines }));
	}

	/// What came of a hot reload, for the world's record
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

	/// Has the world tell this launcher of the hot reloads others report, for
	/// one attached to a world it didn't start: whoever started it reloads
	/// it, and only they would say so otherwise
	pub(super) fn follow_reloads(&mut self) {
		if self.game.is_some() {
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

/// A hot reload another client reported, as the console shows it: as the
/// launcher shows its own
pub fn reload_by_another(entry: &aux::JournalEntry) -> Option<String> {
	let aux::JournalEvent::Reload {
		applied,
		summary,
		errors,
		warnings
	} = &entry.event
	else {
		return None;
	};
	let mut text = if *applied {
		format!("Hot reload (by another launcher): {}", summary)
	} else {
		format!("Hot reload (by another launcher): not reloaded: {}", summary)
	};
	for e in errors {
		text += &format!("\n    {}", e);
	}
	for w in warnings {
		text += &format!("\n    warning: {}", w);
	}
	Some(text)
}
