//! Bettet das Tutorial (TUTORIAL.md) ins Programm ein.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=TUTORIAL.md");
    let text = std::fs::read_to_string("TUTORIAL.md").expect("TUTORIAL.md fehlt");
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("tutorial.md");
    std::fs::write(dest, text).expect("tutorial.md schreiben");
}
