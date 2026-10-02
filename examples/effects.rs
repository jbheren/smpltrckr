//! Compte les effets utilisés (dans les positions jouées) de chaque module donné.
use std::collections::BTreeMap;

fn main() {
    for path in std::env::args().skip(1) {
        let Ok(song) = smpltrckr::format::protracker::read(&std::fs::read(&path).unwrap()) else {
            continue;
        };
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for &p in song.order_list() {
            let Some(pattern) = song.patterns.get(p as usize) else {
                continue;
            };
            for c in pattern.rows.iter().flatten() {
                if c.effect == 0 && c.param == 0 {
                    continue;
                }
                let key = if c.effect == 0xE {
                    format!("E{:X}", c.param >> 4)
                } else {
                    format!("{:X}", c.effect)
                };
                *counts.entry(key).or_default() += 1;
            }
        }
        let name = path.rsplit('/').next().unwrap();
        let list: Vec<String> = counts.iter().map(|(k, n)| format!("{k}:{n}")).collect();
        println!("{name:40} {}", list.join(" "));
    }
}
