use std::io::BufRead;

fn main() {
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        println!("== {line}");
        match dmasm_compiler::compiler::compile_expr(&line, &["a", "b"]) {
            Ok(nodes) => print!("{}", dmasm_bytecode::format(&nodes)),
            Err(e) => println!("error: {e}"),
        }
    }
}
