//! The translations in `locales/` are embedded at compile time: rebuild when they change.

fn main() {
    println!("cargo:rerun-if-changed=locales");
    if let Ok(entries) = std::fs::read_dir("locales") {
        for entry in entries.flatten() {
            println!("cargo:rerun-if-changed={}", entry.path().display());
        }
    }
}
