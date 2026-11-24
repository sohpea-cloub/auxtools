use std::collections::HashMap;

pub struct LineMap {
	to: Vec<u32>,
	new_len: u32
}

const MOST_CELLS: usize = 4_000_000;

impl LineMap {
	pub fn new(old: &[String], new: &[String]) -> Self {
		let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
		let suffix = old[prefix..]
			.iter()
			.rev()
			.zip(new[prefix..].iter().rev())
			.take_while(|(a, b)| a == b)
			.count();
		let (old_mid, new_mid) = (&old[prefix..old.len() - suffix], &new[prefix..new.len() - suffix]);

		let matches = matching_lines(old_mid, new_mid);
		let mut mid = vec![0u32; old_mid.len()];
		let mut i = 0;
		let mut run_new_start = 0;
		while i < old_mid.len() {
			if let Some(j) = matches[i] {
				mid[i] = (prefix + j) as u32;
				run_new_start = j + 1;
				i += 1;
				continue;
			}
			let run_old = i;
			while i < old_mid.len() && matches[i].is_none() {
				i += 1;
			}
			let run_new_end = if i < old_mid.len() { matches[i].unwrap() } else { new_mid.len() };
			let in_place = run_new_end - run_new_start;
			let reindented = reindented_lines(&old_mid[run_old..i], &new_mid[run_new_start..run_new_end]);
			for (n, line) in mid[run_old..i].iter_mut().enumerate() {
				let within = reindented[n].unwrap_or(if in_place == 0 { 0 } else { n.min(in_place - 1) });
				*line = (prefix + run_new_start + within) as u32;
			}
		}
		let (old_unique, new_unique) = (unique_lines(old.iter().map(String::as_str)), unique_lines(new.iter().map(String::as_str)));
		for (i, text) in old_mid.iter().enumerate() {
			let moved = matches[i].is_none() && old_unique.get(text.as_str()) == Some(&Some(prefix + i));
			if let Some(&Some(now)) = new_unique.get(text.as_str()).filter(|_| moved) {
				mid[i] = now as u32;
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

	pub fn line(&self, old: u32) -> u32 {
		let last = self.new_len.max(1);
		if self.to.is_empty() {
			return old.clamp(1, last);
		}
		match self.to.get(old.saturating_sub(1) as usize) {
			Some(new) => (new + 1).min(last),
			None => old.saturating_sub(self.to.len() as u32) + self.new_len
		}
	}
}

fn matching_lines(old: &[String], new: &[String]) -> Vec<Option<usize>> {
	let mut matches = vec![None; old.len()];
	if old.is_empty() || new.is_empty() || old.len() * new.len() > MOST_CELLS {
		return matches;
	}

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

fn unique_lines<'a>(lines: impl Iterator<Item = &'a str>) -> HashMap<&'a str, Option<usize>> {
	let mut found = HashMap::new();
	for (i, line) in lines.enumerate() {
		found.entry(line).and_modify(|at| *at = None).or_insert(Some(i));
	}
	found
}

fn reindented_lines(old: &[String], new: &[String]) -> Vec<Option<usize>> {
	let (old_unique, new_unique) = (
		unique_lines(old.iter().map(|line| line.trim())),
		unique_lines(new.iter().map(|line| line.trim()))
	);
	(old.iter().enumerate())
		.map(|(i, line)| {
			let text = line.trim();
			let unique = !text.is_empty() && old_unique.get(text) == Some(&Some(i));
			new_unique.get(text).copied().flatten().filter(|_| unique)
		})
		.collect()
}

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
		let new = "/proc/a()\n\treturn 1\n\n/proc/c()\n\treturn 2\n";
		assert_eq!(moved(OLD, new, &[3, 4, 5, 6]), [3, 4, 5, 5]);
		let new = "/proc/a()\n\tvar/y = 1\n\ty += 1\n\treturn y\n\n/proc/b()\n\tvar/x = 1\n\treturn x\n";
		assert_eq!(moved(OLD, new, &[2, 3, 4]), [2, 5, 6]);
	}

	#[test]
	fn lines_swapped_around_are_matched_where_they_can_be() {
		let old = "a\nb\nc\nd\ne\n";
		let new = "a\nd\nb\nc\ne\n";
		assert_eq!(moved(old, new, &[1, 2, 3, 5]), [1, 3, 4, 5]);
	}

	#[test]
	fn lines_moved_past_others_go_with_their_text() {
		assert_eq!(moved("a\nb\nc\nd\n", "a\nc\nb\nd\n", &[2, 3]), [3, 2]);
		assert_eq!(moved("a\nb\nc\nd\ne\n", "a\nd\nb\nc\ne\n", &[4]), [2]);
	}

	#[test]
	fn a_statement_put_in_a_block_goes_with_its_text() {
		let old = "/proc/f()\n\tdo_work()\n\treturn\n";
		let new = "/proc/f()\n\tif (enabled)\n\t\tdo_work()\n\treturn\n";
		assert_eq!(moved(old, new, &[2, 3]), [3, 4]);
		assert_eq!(moved(new, old, &[2, 3, 4]), [2, 2, 3]);
		let trimmed = "/proc/f()\n\tif (enabled)\n\t\tdo_work()\n\treturn\n";
		assert_eq!(moved("/proc/f()\n\tdo_work()  \n\treturn\n", trimmed, &[2, 3]), [3, 4]);
	}

	#[test]
	fn a_reindented_statement_with_a_twin_in_the_block_is_placed_in_order() {
		let old = "/proc/f()\n\tdo_work()\n\tdo_work()\n";
		let new = "/proc/f()\n\tif (enabled)\n\t\tdo_work()\n\t\tdo_work()\n";
		assert_eq!(moved(old, new, &[2, 3]), [2, 3]);
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
		assert_eq!(moved(&old, &new, &[1, 2, 100, 3002]), [1, 2, 100, 3003]);
	}
}
