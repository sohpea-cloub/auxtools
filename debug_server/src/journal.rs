use std::{
	collections::VecDeque,
	sync::Mutex,
	time::{Instant, SystemTime, UNIX_EPOCH}
};

use auxtools::{byond_string, Value};

use auxtools_protocol::{JournalEntry, JournalEvent, JournalKind};

pub const CAPACITY: usize = 4000;

pub const MAX_LINES: usize = 500;
pub const MAX_LINE_LEN: usize = 4096;

struct Journal {
	entries: VecDeque<JournalEntry>,
	next_number: u64,
	last_reload: Option<JournalEntry>,
	loaded: Option<(u64, Instant)>,
	loads: u32
}

static JOURNAL: Mutex<Journal> = Mutex::new(Journal {
	entries: VecDeque::new(),
	next_number: 1,
	last_reload: None,
	loaded: None,
	loads: 0
});

fn now_ms() -> u64 {
	SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis() as u64)
}

pub fn world_time() -> f64 {
	Value::world().get_number(byond_string!("time")).map_or(0.0, f64::from)
}

pub fn add(event: JournalEvent) -> JournalEntry {
	let event = trimmed(event);
	let world_time = world_time();
	let mut journal = JOURNAL.lock().unwrap();
	let entry = JournalEntry {
		number: journal.next_number,
		time: now_ms(),
		world_time,
		event
	};
	journal.next_number += 1;
	if entry.event.kind() == JournalKind::Reload {
		journal.last_reload = Some(entry.clone());
	}
	if journal.entries.len() == CAPACITY {
		journal.entries.pop_front();
	}
	journal.entries.push_back(entry.clone());
	entry
}

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

pub fn world_loaded() {
	let load = {
		let mut journal = JOURNAL.lock().unwrap();
		journal.loads += 1;
		journal.loaded = Some((now_ms(), Instant::now()));
		journal.loads
	};
	add(JournalEvent::Loaded { load });
}

pub fn wanted(entry: &JournalEntry, kinds: &[JournalKind]) -> bool {
	kinds.is_empty() || kinds.contains(&entry.event.kind())
}

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
	(entries, first, journal.next_number - 1)
}

pub fn uptime() -> (u64, f64, u32, Option<JournalEntry>) {
	let journal = JOURNAL.lock().unwrap();
	let (loaded, up) = journal.loaded.map_or((0, 0.0), |(at, since)| (at, since.elapsed().as_secs_f64()));
	(loaded, up, journal.loads, journal.last_reload.clone())
}
