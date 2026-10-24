//! Bettet das Tutorial (TUTORIAL.md / TUTORIAL.en.md) ins Programm ein.

use std::path::Path;

fn main() {
    let dir = std::env::var("OUT_DIR").unwrap();
    for (src, dest) in [("TUTORIAL.md", "tutorial.md"), ("TUTORIAL.en.md", "tutorial.en.md")] {
        println!("cargo:rerun-if-changed={src}");
        let text = std::fs::read_to_string(src).unwrap_or_else(|_| panic!("{src} fehlt"));
        std::fs::write(Path::new(&dir).join(dest), text).expect("Tutorial schreiben");
    }
}
