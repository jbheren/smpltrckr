//! Rendu hors ligne d'un morceau, jusqu'à sa fin.

use crate::replayer::Replayer;

/// Rend le morceau en stéréo entrelacée, jusqu'à la fin ou à `max_seconds`.
pub fn stereo(replayer: &mut Replayer, rate: u32, max_seconds: f64) -> Vec<f32> {
    let max_frames = (max_seconds * rate as f64) as usize;
    let mut out = Vec::new();
    let mut buf = vec![0.0f32; 2 * 1024];
    while !replayer.ended() && out.len() / 2 < max_frames {
        replayer.process(&mut buf);
        out.extend_from_slice(&buf);
    }
    out
}

/// Rend une piste mono par voie (sans tenir compte du mixeur), jusqu'à la fin ou à `max_seconds`.
pub fn voices(
    replayer: &mut Replayer,
    channels: usize,
    rate: u32,
    max_seconds: f64,
) -> Vec<Vec<f32>> {
    let max_frames = (max_seconds * rate as f64) as usize;
    let mut outs = vec![Vec::new(); channels];
    while !replayer.ended() && outs[0].len() < max_frames {
        replayer.process_voices(1024, &mut outs);
    }
    outs
}

/// Durée du morceau en secondes (jusqu'à la fin ou à `max_seconds`), calculée rapidement
/// avec un rendu à basse fréquence.
pub fn duration(song: std::sync::Arc<crate::song::Song>, max_seconds: f64) -> f64 {
    const RATE: u32 = 1000;
    let mut replayer = Replayer::new(song, RATE);
    let mut buf = [0.0f32; 2 * 100];
    let mut frames = 0usize;
    while !replayer.ended() && (frames as f64) < max_seconds * RATE as f64 {
        replayer.process(&mut buf);
        frames += 100;
    }
    frames as f64 / RATE as f64
}
