fn main() {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let Some((expr, params)) = args.split_first() else {
		eprintln!("usage: dmasm_compile EXPR [PARAM...]");
		std::process::exit(2);
	};
	let params: Vec<&str> = params.iter().map(String::as_str).collect();

	match dmasm::compiler::compile_expr(expr, &params) {
		Ok(nodes) => {
			for node in nodes {
				println!("{:?}", node);
			}
		}
		Err(e) => {
			eprintln!("{}", e);
			std::process::exit(1);
		}
	}
}
