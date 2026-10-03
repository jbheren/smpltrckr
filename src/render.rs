//! Offline rendering of a song, up to its end.

use crate::replayer::Replayer;

/// Renders the song as interleaved stereo, up to its end or `max_seconds`.
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

/// Renders one mono track per voice (ignoring the mixer), up to the end or `max_seconds`.
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

/// Song duration in seconds (up to its end or `max_seconds`), computed quickly with a
/// low sample rate render.
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
