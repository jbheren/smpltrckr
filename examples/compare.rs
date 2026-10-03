//! Compares two WAV renders of the same module: durations and envelope correlation.
//!
//! Usage: compare ours.wav reference.wav
//! The correlation runs on the energy over 5 ms windows, in mono: it tells whether notes and
//! volumes land at the same moments, whatever the interpolation or the stereo.
//! It is computed over 2 s slices, each aligned as well as possible (±300 ms): libopenmpt
//! rounds a tick to a whole number of samples, hence a slight drift away from 125 BPM,
//! measured separately. With SEGMENTS=1, each slice's score is printed.

use std::path::Path;

use smpltrckr::wav;

fn envelope(samples: &[f32], rate: u32) -> Vec<f64> {
    let window = (rate / 200) as usize;
    samples
        .chunks(window)
        .map(|w| (w.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / w.len() as f64).sqrt())
        .collect()
}

fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n < 2 {
        return 0.0;
    }
    let (a, b) = (&a[..n], &b[..n]);
    let mean = |x: &[f64]| x.iter().sum::<f64>() / n as f64;
    let (ma, mb) = (mean(a), mean(b));
    let cov: f64 = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum();
    let var = |x: &[f64], m: f64| x.iter().map(|v| (v - m).powi(2)).sum::<f64>();
    let denom = (var(a, ma) * var(b, mb)).sqrt();
    if denom == 0.0 { 1.0 } else { cov / denom }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(args.len() == 2, "usage: compare ours.wav reference.wav");
    let (ours, rate_ours) = wav::read_mono(Path::new(&args[0]))?;
    let (reference, rate_ref) = wav::read_mono(Path::new(&args[1]))?;
    let (d_ours, d_ref) = (
        ours.len() as f64 / rate_ours as f64,
        reference.len() as f64 / rate_ref as f64,
    );
    let (a, b) = (envelope(&ours, rate_ours), envelope(&reference, rate_ref));
    let (segment, max_lag) = (400, 60);
    let (mut scores, mut drift) = (Vec::new(), 0isize);
    for start in (0..a.len().min(b.len()).saturating_sub(segment)).step_by(segment) {
        let reference = &b[start..start + segment];
        let (r, lag) = (-max_lag..=max_lag)
            .filter_map(|lag| {
                let from = start.checked_add_signed(lag)?;
                Some((correlation(a.get(from..from + segment)?, reference), lag))
            })
            .fold((f64::MIN, 0), |best, x| if x.0 > best.0 { x } else { best });
        // Near-silent slices tell nothing.
        if reference.iter().any(|&x| x > 1e-3) {
            scores.push(r);
            drift = lag;
            if std::env::var_os("SEGMENTS").is_some() {
                eprintln!(
                    "{:6.1} s  r={r:.3}  offset {:+} ms",
                    start as f64 / 200.0,
                    lag * 5
                );
            }
        }
    }
    if scores.is_empty() {
        // Song shorter than a slice: global correlation, no alignment.
        scores.push(correlation(&a, &b));
    }
    let mean = scores.iter().sum::<f64>() / scores.len() as f64;
    let min = scores.iter().copied().fold(1.0, f64::min);
    let gap = if d_ref > 0.0 {
        (d_ours - d_ref) / d_ref * 100.0
    } else {
        0.0
    };
    println!(
        "duration {d_ours:7.2} s / {d_ref:7.2} s ({gap:+5.1} %)  envelope r={mean:.3} (min {min:.3})  drift {:+} ms",
        drift * 5
    );
    Ok(())
}
