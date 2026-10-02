//! Compare deux rendus WAV d'un même module : durées et corrélation des enveloppes.
//!
//! Usage : compare nôtre.wav référence.wav
//! La corrélation porte sur l'énergie par fenêtres de 5 ms, en mono : elle mesure si les
//! notes et les volumes tombent au même moment, sans dépendre de l'interpolation ni de la stéréo.
//! Elle est calculée par tranches de 2 s, chacune alignée au mieux (±300 ms) : libopenmpt
//! arrondit la durée d'un tick à un nombre entier d'échantillons, d'où une légère dérive
//! hors 125 BPM, qu'on mesure à part. Avec SEGMENTS=1, le score de chaque tranche s'affiche.

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
    anyhow::ensure!(args.len() == 2, "usage : compare nôtre.wav référence.wav");
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
        // Les tranches presque silencieuses ne disent rien.
        if reference.iter().any(|&x| x > 1e-3) {
            scores.push(r);
            drift = lag;
            if std::env::var_os("SEGMENTS").is_some() {
                eprintln!(
                    "{:6.1} s  r={r:.3}  décalage {:+} ms",
                    start as f64 / 200.0,
                    lag * 5
                );
            }
        }
    }
    if scores.is_empty() {
        // Morceau plus court qu'une tranche : corrélation globale, sans alignement.
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
        "durée {d_ours:7.2} s / {d_ref:7.2} s ({gap:+5.1} %)  enveloppe r={mean:.3} (min {min:.3})  dérive {:+} ms",
        drift * 5
    );
    Ok(())
}
