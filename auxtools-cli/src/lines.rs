//! Where a file's lines went when it changed: a breakpoint stays on its
//! statement when the code around it moves, whoever edited the file.

/// The lines of an old text, each with the line of the new text it is now
pub struct LineMap {
	/// By old line, from 0: the new line, from 0
	to: Vec<u32>,
	new_len: u32
}

/// The most lines the changed part of a file may have, old times new, for
/// its lines to be matched up one by one. Past it they're taken to have all
/// changed.
const MOST_CELLS: usize = 4_000_000;

impl LineMap {
	pub fn new(old: &[String], new: &[String]) -> Self {
		// Edits are local: what's the same at the start and the end is
		// where it was, give or take the difference in length
		let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
		let suffix = old[prefix..]
			.iter()
			.rev()
			.zip(new[prefix..].iter().rev())
			.take_while(|(a, b)| a == b)
			.count();
		let (old_mid, new_mid) = (&old[prefix..old.len() - suffix], &new[prefix..new.len() - suffix]);

		// Between the lines the two have in common are the runs that changed:
		// some old lines, and the new ones in their place. An old line of a
		// run is as far into the new ones as it was into the old, or on the
		// last of them; with none in its place it's on the next line left.
		let matches = matching_lines(old_mid, new_mid);
		let mut mid = vec![0u32; old_mid.len()];
		let mut i = 0;
		// Where the new lines of the run being looked at start
		let mut run_new = 0;
		while i < old_mid.len() {
			if let Some(j) = matches[i] {
				mid[i] = (prefix + j) as u32;
				run_new = j + 1;
				i += 1;
				continue;
			}
			let run_old = i;
			while i < old_mid.len() && matches[i].is_none() {
				i += 1;
			}
			let run_new_end = if i < old_mid.len() { matches[i].unwrap() } else { new_mid.len() };
			let in_place = run_new_end - run_new;
			for (n, line) in mid[run_old..i].iter_mut().enumerate() {
				let within = if in_place == 0 { 0 } else { n.min(in_place - 1) };
				*line = (prefix + run_new + within) as u32;
			}
		}

		let after = (new.len() - suffix) as u32;
		let mut to: Vec<u32> = (0..prefix as u32).collect();
		to.extend(mid);
		to.extend((0..suffix as u32).map(|i| after + i));

		Self {
			to,
			new_len: new.len() as u32
		}
	}

	/// The line (from 1) that an old line (from 1) is now. One that was
	/// changed is on what's written in its place, one that was taken away on
	/// the next line that's still there.
	pub fn line(&self, old: u32) -> u32 {
		let last = self.new_len.max(1);
		// An empty file has nothing to go by: the line is where it was, if
		// the new text has it
		if self.to.is_empty() {
			return old.clamp(1, last);
		}
		match self.to.get(old.saturating_sub(1) as usize) {
			Some(new) => (new + 1).min(last),
			// Past the end of the old text: as far past the end of the new
			None => old.saturating_sub(self.to.len() as u32) + self.new_len
		}
	}
}

/// For each old line, the new line that's the same line, in a longest run of
/// lines the two have in common, in order
fn matching_lines(old: &[String], new: &[String]) -> Vec<Option<usize>> {
	let mut matches = vec![None; old.len()];
	if old.is_empty() || new.is_empty() || old.len() * new.len() > MOST_CELLS {
		return matches;
	}

	// common[i][j]: how many lines old[i..] and new[j..] have in common
	let width = new.len() + 1;
	let mut common = vec![0u32; (old.len() + 1) * width];
	for i in (0..old.len()).rev() {
		for j in (0..new.len()).rev() {
			common[i * width + j] = if old[i] == new[j] {
				common[(i + 1) * width + j + 1] + 1
			} else {
				common[(i + 1) * width + j].max(common[i * width + j + 1])
			};
		}
	}

	let (mut i, mut j) = (0, 0);
	while i < old.len() && j < new.len() {
		if old[i] == new[j] {
			matches[i] = Some(j);
			i += 1;
			j += 1;
		} else if common[(i + 1) * width + j] >= common[i * width + j + 1] {
			i += 1;
		} else {
			j += 1;
		}
	}
	matches
}

/// A file's lines, if it can be read
pub fn read(path: &str) -> Option<Vec<String>> {
	let text = std::fs::read_to_string(path).ok()?;
	Some(text.lines().map(str::to_owned).collect())
}

#[cfg(test)]
mod tests {
	use super::LineMap;

	fn lines(text: &str) -> Vec<String> {
		text.lines().map(str::to_owned).collect()
	}

	fn moved(old: &str, new: &str, from: &[u32]) -> Vec<u32> {
		let map = LineMap::new(&lines(old), &lines(new));
		from.iter().map(|&line| map.line(line)).collect()
	}

	const OLD: &str = "/proc/a()\n\treturn 1\n\n/proc/b()\n\tvar/x = 1\n\treturn x\n";

	#[test]
	fn a_file_that_didnt_change_keeps_its_lines() {
		assert_eq!(moved(OLD, OLD, &[1, 2, 3, 4, 5, 6]), [1, 2, 3, 4, 5, 6]);
	}

	#[test]
	fn lines_added_above_push_the_rest_down() {
		let new = "/proc/a()\n\treturn 1\n\n/proc/new()\n\treturn 0\n\n/proc/b()\n\tvar/x = 1\n\treturn x\n";
		assert_eq!(moved(OLD, new, &[1, 2, 4, 5, 6]), [1, 2, 7, 8, 9]);
	}

	#[test]
	fn lines_taken_away_above_pull_the_rest_up() {
		let new = "/proc/b()\n\tvar/x = 1\n\treturn x\n";
		assert_eq!(moved(OLD, new, &[4, 5, 6]), [1, 2, 3]);
		// What was taken away is where the next line left is
		assert_eq!(moved(OLD, new, &[1, 2, 3]), [1, 1, 1]);
	}

	#[test]
	fn a_line_added_inside_a_proc_moves_what_follows() {
		let new = "/proc/a()\n\treturn 1\n\n/proc/b()\n\tvar/x = 1\n\tx += 1\n\treturn x\n";
		assert_eq!(moved(OLD, new, &[5, 6]), [5, 7]);
	}

	#[test]
	fn a_line_that_changed_is_where_it_was() {
		let new = "/proc/a()\n\treturn 1\n\n/proc/b()\n\tvar/x = 2\n\treturn x\n";
		assert_eq!(moved(OLD, new, &[5, 6]), [5, 6]);
		let new = "/proc/a()\n\treturn 1\n\n/proc/b()\n\tvar/x = 1\n\treturn x + 1\n";
		assert_eq!(moved(OLD, new, &[5, 6]), [5, 6]);
	}

	#[test]
	fn lines_written_in_place_of_others_take_them_in_order() {
		// Two lines for three: the third is on the second
		let new = "/proc/a()\n\treturn 1\n\n/proc/c()\n\treturn 2\n";
		assert_eq!(moved(OLD, new, &[3, 4, 5, 6]), [3, 4, 5, 5]);
		// Three lines for one: it's on the first
		let new = "/proc/a()\n\tvar/y = 1\n\ty += 1\n\treturn y\n\n/proc/b()\n\tvar/x = 1\n\treturn x\n";
		assert_eq!(moved(OLD, new, &[2, 3, 4]), [2, 5, 6]);
	}

	#[test]
	fn lines_swapped_around_are_matched_where_they_can_be() {
		let old = "a\nb\nc\nd\ne\n";
		let new = "a\nd\nb\nc\ne\n";
		// b and c kept their order, d moved past them
		assert_eq!(moved(old, new, &[1, 2, 3, 5]), [1, 3, 4, 5]);
	}

	#[test]
	fn lines_past_the_end_stay_as_far_past_it() {
		assert_eq!(moved(OLD, "one\ntwo\n", &[7, 9]), [3, 5]);
	}

	#[test]
	fn an_empty_file_has_one_place() {
		assert_eq!(moved(OLD, "", &[1, 6]), [1, 1]);
	}

	#[test]
	fn lines_of_an_empty_file_stay_where_they_are() {
		assert_eq!(moved("", OLD, &[1, 3, 6]), [1, 3, 6]);
		assert_eq!(moved("", "one\ntwo\n", &[1, 5]), [1, 2]);
		assert_eq!(moved("", "", &[1, 4]), [1, 1]);
	}

	#[test]
	fn a_big_change_in_the_middle_keeps_both_ends() {
		let old: String = (0..3000).map(|i| format!("old {}\n", i)).collect();
		let new: String = (0..3000).map(|i| format!("new {}\n", i)).collect();
		let old = format!("first\n{}last\n", old);
		let new = format!("first\n{}more\nlast\n", new);
		// Too much changed to match up: its lines keep their order
		assert_eq!(moved(&old, &new, &[1, 2, 100, 3002]), [1, 2, 100, 3003]);
	}
}
