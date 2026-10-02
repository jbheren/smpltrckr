//! Trace l'enchaînement des positions jouées par le replayer : trace module.mod
use std::sync::Arc;

use smpltrckr::replayer::Replayer;

fn main() {
    let path = std::env::args().nth(1).expect("usage : trace module.mod");
    let song = smpltrckr::format::protracker::read(&std::fs::read(path).unwrap()).unwrap();
    let mut r = Replayer::new(Arc::new(song), 1000);
    let (mut last, mut frames, mut buf) = (usize::MAX, 0usize, [0.0f32; 2]);
    let mut line = Vec::new();
    while !r.ended() && frames < 1000 * 1200 {
        let (position, row) = r.position();
        if position != last {
            line.push(format!(
                "{position}@{:.1}s{}",
                frames as f64 / 1000.0,
                if row > 0 {
                    format!("(ligne {row})")
                } else {
                    String::new()
                }
            ));
            last = position;
        }
        r.process(&mut buf);
        frames += 1;
    }
    println!("{}", line.join(" "));
    println!(
        "fin à {:.2} s, position {:?}",
        frames as f64 / 1000.0,
        r.position()
    );
}
