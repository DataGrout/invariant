//! cargo run -p invariant-core --example lens_file -- <path> [display-path]
//! Prints the facts `invariant lens` would extract from one file.
use invariant_core::{Analyzer, Language};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = &args[1];
    let shown = args.get(2).unwrap_or(path);
    let ext = path.rsplit('.').next().unwrap_or("");
    let language = Language::from_extension(ext).expect("unsupported extension");
    let code = std::fs::read_to_string(path).expect("read");
    let result = Analyzer::new()
        .unwrap()
        .lens_code(&code, language, shown, "sha")
        .expect("lens");
    for fact in result.facts {
        println!("{fact}");
    }
}
