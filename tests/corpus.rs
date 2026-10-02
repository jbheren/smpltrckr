//! Aller-retour à l'octet près sur le corpus de modules réels (`corpus/`, non versionné).
//! Le corpus se reconstitue avec `scripts/fetch-corpus.py --from-list`.

use std::path::Path;

use smpltrckr::format::protracker;

#[test]
fn corpus_roundtrip_is_byte_exact() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("corpus absent ({}), test ignoré", dir.display());
        return;
    };

    let (mut checked, mut skipped, mut failures) = (0, 0, Vec::new());
    for path in entries.map(|e| e.unwrap().path()) {
        let original = std::fs::read(&path).unwrap();
        match protracker::read(&original) {
            Ok(song) if protracker::write(&song) == original => checked += 1,
            Ok(_) => failures.push(format!("{} : réécriture différente", path.display())),
            // Formats d'emballage connus et volontairement refusés.
            Err(e) if format!("{e}").contains("PowerPacker") => skipped += 1,
            Err(e) => failures.push(format!("{} : {e:#}", path.display())),
        }
    }
    eprintln!("corpus : {checked} identiques, {skipped} ignorés (compressés)");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
