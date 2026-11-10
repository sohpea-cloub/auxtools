//! The world's record of what happened in it: runtime errors, hot reloads,
//! worlds loading, what the world printed. Kept here rather than in the
//! `Server`, which a reboot puts away and makes again: the record lasts as
//! long as the process, and its numbers go on from one world to the next.

use std::{
	collections::VecDeque,
	sync::Mutex,
	time::{Instant, SystemTime, UNIX_EPOCH}
};

use auxtools::{byond_string, Value};

use crate::server_types::{JournalEntry, JournalEvent, JournalKind};

/// How many entries are kept. A world that runtimes on every tick fills it
/// in a minute or two, which is still the last minute or two.
pub const CAPACITY: usize = 4000;

/// How many lines of output an entry has at most, and how long each is: a
/// launcher sends them in batches, and anything bigger is cut
pub const MAX_LINES: usize = 500;
pub const MAX_LINE_LEN: usize = 4096;

struct Journal {
	entries: VecDeque<JournalEntry>,
	/// The number the next entry gets
	next: u64,
	last_reload: Option<JournalEntry>,
	/// When the world now running loaded
	loaded: Option<(u64, Instant)>,
	loads: u32
}

static JOURNAL: Mutex<Journal> = Mutex::new(Journal {
	entries: VecDeque::new(),
	next: 1,
	last_reload: None,
	loaded: None,
	loads: 0
});

fn now_ms() -> u64 {
	SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis() as u64)
}

/// `world.time`, which only the world's own thread may read
pub fn world_time() -> f64 {
	Value::WORLD.get_number(byond_string!("time")).map_or(0.0, f64::from)
}

/// Adds an entry, and hands it back. On the world's thread.
pub fn add(event: JournalEvent) -> JournalEntry {
	let event = trimmed(event);
	let world_time = world_time();
	let mut journal = JOURNAL.lock().unwrap();
	let entry = JournalEntry {
		number: journal.next,
		time: now_ms(),
		world_time,
		event
	};
	journal.next += 1;
	if entry.event.kind() == JournalKind::Reload {
		journal.last_reload = Some(entry.clone());
	}
	if journal.entries.len() == CAPACITY {
		journal.entries.pop_front();
	}
	journal.entries.push_back(entry.clone());
	entry
}

/// What a client sent, cut down to size
fn trimmed(event: JournalEvent) -> JournalEvent {
	match event {
		JournalEvent::Output { mut lines } => {
			lines.truncate(MAX_LINES);
			for line in &mut lines {
				if line.len() > MAX_LINE_LEN {
					let mut end = MAX_LINE_LEN;
					while !line.is_char_boundary(end) {
						end -= 1;
					}
					line.truncate(end);
				}
			}
			JournalEvent::Output { lines }
		}
		event => event
	}
}

/// A world has loaded: the record says so, and its uptime starts
pub fn world_loaded() {
	let load = {
		let mut journal = JOURNAL.lock().unwrap();
		journal.loads += 1;
		journal.loaded = Some((now_ms(), Instant::now()));
		journal.loads
	};
	add(JournalEvent::Loaded { load });
}

/// Whether an entry is of one of these kinds (of any, if there are none)
pub fn wanted(entry: &JournalEntry, kinds: &[JournalKind]) -> bool {
	kinds.is_empty() || kinds.contains(&entry.event.kind())
}

/// The entries after `after` of these kinds, the newest `limit` of them; and
/// the numbers of the oldest entry kept and of the newest
pub fn read(after: u64, kinds: &[JournalKind], limit: Option<u32>) -> (Vec<JournalEntry>, u64, u64) {
	let journal = JOURNAL.lock().unwrap();
	let mut entries: Vec<JournalEntry> = journal
		.entries
		.iter()
		.filter(|entry| entry.number > after && wanted(entry, kinds))
		.cloned()
		.collect();
	if let Some(limit) = limit {
		let extra = entries.len().saturating_sub(limit as usize);
		entries.drain(..extra);
	}
	let first = journal.entries.front().map_or(0, |entry| entry.number);
	(entries, first, journal.next - 1)
}

/// When the world loaded (milliseconds since 1970), how many seconds ago,
/// how many worlds the process has loaded, and the last hot reload
pub fn uptime() -> (u64, f64, u32, Option<JournalEntry>) {
	let journal = JOURNAL.lock().unwrap();
	let (loaded, up) = journal.loaded.map_or((0, 0.0), |(at, since)| (at, since.elapsed().as_secs_f64()));
	(loaded, up, journal.loads, journal.last_reload.clone())
}
