//! Byte-exact round trip over the corpus of real modules (`corpus/`, not versioned).
//! Rebuild the corpus with `scripts/fetch-corpus.py --from-list`.

use std::path::Path;

use smpltrckr::format::protracker;

#[test]
fn corpus_roundtrip_is_byte_exact() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("no corpus ({}), test skipped", dir.display());
        return;
    };

    let (mut checked, mut skipped, mut failures) = (0, 0, Vec::new());
    for path in entries.map(|e| e.unwrap().path()) {
        let original = std::fs::read(&path).unwrap();
        match protracker::read(&original) {
            Ok(song) if protracker::write(&song) == original => checked += 1,
            Ok(_) => failures.push(format!("{}: rewritten file differs", path.display())),
            // Known packers, refused on purpose.
            Err(e) if format!("{e}").contains("PowerPacker") => skipped += 1,
            Err(e) => failures.push(format!("{} : {e:#}", path.display())),
        }
    }
    eprintln!("corpus: {checked} identical, {skipped} skipped (packed)");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
