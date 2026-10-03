//! Lists the periods missing from the ProTracker table in the given modules.
use std::collections::BTreeMap;

fn main() {
    let mut counts: BTreeMap<u16, (usize, usize)> = BTreeMap::new();
    for (i, path) in std::env::args().skip(1).enumerate() {
        let Ok(song) = smpltrckr::format::protracker::read(&std::fs::read(&path).unwrap()) else {
            continue;
        };
        for cell in song.patterns.iter().flat_map(|p| p.rows.iter().flatten()) {
            if cell.period != 0 && !smpltrckr::note::PERIODS.contains(&cell.period) {
                let e = counts.entry(cell.period).or_default();
                e.0 += 1;
                e.1 |= 1 << (i % 64);
            }
        }
    }
    for (p, (n, files)) in counts {
        println!("{p:5} ×{n:4}  ({} fichier(s))", (files as u64).count_ones());
    }
}
