//! Affiche, tick par tick, la période et le volume d'une voie : periods module.mod voie début_s fin_s
use std::sync::Arc;

use smpltrckr::replayer::Replayer;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let song = smpltrckr::format::protracker::read(&std::fs::read(&a[0]).unwrap()).unwrap();
    let (voice, from, to): (usize, f64, f64) = (
        a[1].parse::<usize>().unwrap() - 1,
        a[2].parse().unwrap(),
        a[3].parse().unwrap(),
    );
    let rate = 48000;
    let mut r = Replayer::new(Arc::new(song), rate);
    let mut buf = [0.0f32; 2];
    let mut last = (usize::MAX, 0, 0);
    for frame in 0..(to * rate as f64) as usize {
        r.process(&mut buf);
        let state = r.voice_state(voice);
        if frame as f64 >= from * rate as f64 && state != last {
            let (pos, row) = r.position();
            println!(
                "{:7.3} s  pos {pos:3} ligne {row:2}  sample {:2} période {:4} volume {:2}",
                frame as f64 / rate as f64,
                state.0,
                state.1,
                state.2
            );
        }
        last = state;
    }
}
