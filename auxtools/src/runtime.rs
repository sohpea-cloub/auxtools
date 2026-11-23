use std::result;

use crate::value::Value;

/// An error returned by the library's API and by hooks.
#[derive(Debug)]
pub struct Runtime {
	pub message: String
}

impl Runtime {
	pub fn new<S: Into<String>>(message: S) -> Self {
		Self { message: message.into() }
	}
}

/// Creates a [`Runtime`], formatting the message when given arguments.
#[macro_export]
macro_rules! runtime {
	($fmt:expr) => {
		$crate::Runtime::new($fmt)
	};
	($fmt: expr, $( $args:expr ),*) => {
		$crate::Runtime::new(format!( $fmt, $( $args, )* ))
	};
}

pub type DMResult<T = Value> = result::Result<T, Runtime>;
