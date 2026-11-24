mod standin {
	pub mod commands;
	pub mod ending;
	pub mod flags;
	pub mod hold;
	pub mod partial;
	pub mod world;

	pub const SHORTLY: std::time::Duration = std::time::Duration::from_millis(300);
}

#[test]
fn world_process() {
	test_support::standin::world_process();
}
