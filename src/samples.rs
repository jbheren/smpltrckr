//! Samples : génération de formes d'onde et de percussions, import de WAV et d'AIFF.
//!
//! Tout finit en 8 bits signés mono, le format du `.mod`.

use std::path::Path;

use anyhow::{Context, bail, ensure};

use crate::note;
use crate::song::{MAX_SAMPLE_BYTES, Sample};

/// Fréquence de lecture d'un sample joué en C-3 (≈ 16 574 Hz) : les percussions sont
/// calculées pour cette note.
fn c3_rate() -> f64 {
    note::period_to_hz(note::PERIODS[24])
}

/// Formes d'onde disponibles pour `generate`.
pub const WAVEFORMS: [&str; 9] = [
    "sine", "square", "pulse", "saw", "triangle", "noise", "kick", "snare", "hihat",
];

/// Génère un sample.
///
/// - Formes tonales (`sine`, `square`, `pulse` à 25 %, `saw`, `triangle`) : un cycle de
///   `cycle` octets, en boucle. Avec 32 octets, la note C-2 sonne à peu près comme un do 4
///   (259 Hz), C-3 un octave au-dessus.
/// - `noise` : 4 096 octets de bruit blanc en boucle.
/// - Percussions (`kick`, `snare`, `hihat`) : sans boucle, à jouer en C-3.
pub fn generate(waveform: &str, cycle: usize) -> anyhow::Result<Sample> {
    let cycle = cycle.clamp(2, 1024) & !1;
    let mut noise = Noise(0x2545_f491);
    let mut sample = Sample {
        volume: 64,
        ..Sample::default()
    };
    let tonal = |f: &dyn Fn(f64) -> f64| {
        (0..cycle)
            .map(|i| f(i as f64 / cycle as f64))
            .collect::<Vec<f64>>()
    };
    let (data, looped) = match waveform {
        "sine" => (tonal(&|t| (t * std::f64::consts::TAU).sin()), true),
        "square" => (tonal(&|t| if t < 0.5 { 1.0 } else { -1.0 }), true),
        "pulse" => (tonal(&|t| if t < 0.25 { 1.0 } else { -1.0 }), true),
        "saw" => (tonal(&|t| 1.0 - 2.0 * t), true),
        "triangle" => (tonal(&|t| 1.0 - 4.0 * (t - 0.5).abs()), true),
        "noise" => ((0..4096).map(|_| noise.next()).collect(), true),
        "kick" => (kick(), false),
        "snare" => (
            drum(0.25, |t, n| {
                0.5 * (t * 180.0 * std::f64::consts::TAU).sin() * (-t / 0.04).exp()
                    + 0.7 * n * (-t / 0.09).exp()
            }),
            false,
        ),
        "hihat" => {
            let mut last = 0.0;
            (
                drum(0.09, |t, n| {
                    // Bruit passé en passe-haut (différence de deux tirages) : plus métallique.
                    let high = n - last;
                    last = n;
                    0.6 * high * (-t / 0.025).exp()
                }),
                false,
            )
        }
        other => bail!(
            "forme d'onde inconnue {other:?} (au choix : {})",
            WAVEFORMS.join(", ")
        ),
    };
    sample.set_name(waveform);
    sample.set_data(data.iter().map(|&x| to_i8(x * 0.95)).collect());
    if looped {
        sample.loop_start = 0;
        sample.loop_length = sample.length_words;
    }
    Ok(sample)
}

fn kick() -> Vec<f64> {
    // Sinus dont la hauteur tombe de 150 à 45 Hz, avec une décroissance rapide.
    let rate = c3_rate();
    let mut phase = 0.0;
    (0..(0.45 * rate) as usize)
        .map(|i| {
            let t = i as f64 / rate;
            phase += (45.0 + 105.0 * (-t / 0.03).exp()) / rate;
            (phase * std::f64::consts::TAU).sin() * (-t / 0.12).exp()
        })
        .collect()
}

/// Percussion de `seconds` secondes : `f(t, bruit)` donne chaque échantillon.
fn drum(seconds: f64, mut f: impl FnMut(f64, f64) -> f64) -> Vec<f64> {
    let rate = c3_rate();
    let mut noise = Noise(0x9e37_79b9);
    (0..(seconds * rate) as usize)
        .map(|i| f(i as f64 / rate, noise.next()))
        .collect()
}

/// Générateur de bruit déterministe (xorshift), pour des samples reproductibles.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f64 / u32::MAX as f64 * 2.0 - 1.0
    }
}

fn to_i8(x: f64) -> i8 {
    (x * 127.0).round().clamp(-128.0, 127.0) as i8
}

// --- Import ---------------------------------------------------------------------------------

/// Son décodé : mono, en flottants, avec sa fréquence et sa boucle éventuelle (en trames).
pub struct Decoded {
    pub samples: Vec<f32>,
    pub rate: u32,
    pub loop_frames: Option<(usize, usize)>,
}

/// Bilan d'un import, à montrer à l'utilisateur ou à l'agent.
pub struct ImportReport {
    pub sample: Sample,
    pub rate: u32,
    pub truncated: bool,
    /// Note à laquelle le sample sonne à sa hauteur d'origine, si elle existe dans C-1 … B-3.
    pub natural_note: Option<String>,
}

/// Importe un WAV ou un AIFF : mixage mono, 8 bits, sans rééchantillonnage.
/// `halve` divise la fréquence par deux (une trame sur deux, moyennée) pour gagner de la place.
pub fn import(path: &Path, halve: bool) -> anyhow::Result<ImportReport> {
    let data = std::fs::read(path).with_context(|| format!("lecture de {}", path.display()))?;
    let mut decoded = match &data.get(..4) {
        Some(b"RIFF") => decode_wav(&data)?,
        Some(b"FORM") => decode_aiff(&data)?,
        _ => bail!("{} : ni WAV ni AIFF", path.display()),
    };
    ensure!(
        !decoded.samples.is_empty(),
        "{} : aucun son",
        path.display()
    );
    if halve {
        decoded.samples = decoded
            .samples
            .chunks(2)
            .map(|p| p.iter().sum::<f32>() / p.len() as f32)
            .collect();
        decoded.rate /= 2;
        decoded.loop_frames = decoded.loop_frames.map(|(s, e)| (s / 2, e / 2));
    }

    let truncated = decoded.samples.len() > MAX_SAMPLE_BYTES;
    let mut sample = Sample {
        volume: 64,
        ..Sample::default()
    };
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    sample.set_name(&name);
    sample.set_data(decoded.samples.iter().map(|&x| to_i8(x as f64)).collect());
    if let Some((start, end)) = decoded.loop_frames {
        let (start, end) = (start / 2, end.min(sample.data.len()) / 2);
        if end > start + 1 {
            sample.loop_start = start as u16;
            sample.loop_length = (end - start) as u16;
        }
    }
    let natural_note = note::PERIODS
        .iter()
        .enumerate()
        .min_by_key(|(_, p)| (note::period_to_hz(**p) - decoded.rate as f64).abs() as u64)
        .filter(|(_, p)| (note::period_to_hz(**p) / decoded.rate as f64 - 1.0).abs() < 0.03)
        .map(|(i, _)| note::name(i));
    Ok(ImportReport {
        sample,
        rate: decoded.rate,
        truncated,
        natural_note,
    })
}

fn u16_le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
fn u32_be(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Parcourt les blocs d'un fichier RIFF ou IFF : (identifiant, contenu).
fn chunks(data: &[u8], big_endian: bool) -> Vec<(&[u8], &[u8])> {
    let mut out = Vec::new();
    let mut pos = 12;
    while pos + 8 <= data.len() {
        let len = if big_endian {
            u32_be(&data[pos + 4..])
        } else {
            u32_le(&data[pos + 4..])
        } as usize;
        let body = &data[pos + 8..(pos + 8 + len).min(data.len())];
        out.push((&data[pos..pos + 4], body));
        pos += 8 + len + (len & 1);
    }
    out
}

/// Convertit des trames PCM entrelacées en mono flottant.
fn to_mono(
    bytes: &[u8],
    channels: usize,
    bits: u16,
    float: bool,
    big_endian: bool,
) -> anyhow::Result<Vec<f32>> {
    let width = (bits as usize).div_ceil(8);
    ensure!(
        channels > 0 && (1..=8).contains(&width),
        "format audio invalide ({bits} bits)"
    );
    let read = |b: &[u8]| -> f32 {
        let mut v = [0u8; 8];
        for (k, &byte) in b.iter().enumerate() {
            v[if big_endian { width - 1 - k } else { k }] = byte;
        }
        match (float, width) {
            (true, 4) => f32::from_le_bytes([v[0], v[1], v[2], v[3]]),
            (true, _) => f64::from_le_bytes(v) as f32,
            // Le WAV 8 bits est non signé ; l'AIFF 8 bits est signé.
            (false, 1) if !big_endian => (v[0] as f32 - 128.0) / 128.0,
            (false, w) => {
                let raw = i64::from_le_bytes(v) << (64 - 8 * w) >> (64 - 8 * w);
                raw as f32 / (1i64 << (8 * w - 1)) as f32
            }
        }
    };
    if float {
        ensure!(
            width == 4 || width == 8,
            "flottants de {bits} bits non pris en charge"
        );
    }
    Ok(bytes
        .chunks_exact(width * channels)
        .map(|frame| frame.chunks_exact(width).map(read).sum::<f32>() / channels as f32)
        .collect())
}

fn decode_wav(data: &[u8]) -> anyhow::Result<Decoded> {
    ensure!(data.len() > 12 && &data[8..12] == b"WAVE", "WAV invalide");
    let (mut format, mut channels, mut rate, mut bits) = (0u16, 0usize, 0u32, 0u16);
    let (mut samples, mut loop_frames) = (None, None);
    for (id, body) in chunks(data, false) {
        match id {
            b"fmt " if body.len() >= 16 => {
                format = u16_le(body);
                channels = u16_le(&body[2..]) as usize;
                rate = u32_le(&body[4..]);
                bits = u16_le(&body[14..]);
                if format == 0xFFFE && body.len() >= 26 {
                    format = u16_le(&body[24..]); // WAVE_FORMAT_EXTENSIBLE
                }
            }
            b"data" => {
                ensure!(
                    format == 1 || format == 3,
                    "WAV compressé (format {format}) non pris en charge"
                );
                samples = Some(to_mono(body, channels, bits, format == 3, false)?);
            }
            // Bloc « smpl » : la première boucle, en trames (fin incluse).
            b"smpl" if body.len() >= 36 + 24 && u32_le(&body[28..]) > 0 => {
                loop_frames = Some((
                    u32_le(&body[44..]) as usize,
                    u32_le(&body[48..]) as usize + 1,
                ));
            }
            _ => {}
        }
    }
    Ok(Decoded {
        samples: samples.context("WAV sans données")?,
        rate,
        loop_frames,
    })
}

fn decode_aiff(data: &[u8]) -> anyhow::Result<Decoded> {
    ensure!(data.len() > 12, "AIFF invalide");
    let aifc = match &data[8..12] {
        b"AIFF" => false,
        b"AIFC" => true,
        _ => bail!("AIFF invalide"),
    };
    let (mut channels, mut bits, mut rate) = (0usize, 0u16, 0u32);
    let (mut little_endian, mut float) = (false, false);
    let mut samples = None;
    for (id, body) in chunks(data, true) {
        match id {
            b"COMM" if body.len() >= 18 => {
                channels = u16::from_be_bytes([body[0], body[1]]) as usize;
                bits = u16::from_be_bytes([body[6], body[7]]);
                rate = extended_to_f64(&body[8..18]).round() as u32;
                if aifc && body.len() >= 22 {
                    match &body[18..22] {
                        b"NONE" | b"twos" => {}
                        b"sowt" => little_endian = true,
                        b"fl32" | b"FL32" => float = true,
                        other => bail!(
                            "AIFF-C compressé ({}) non pris en charge",
                            String::from_utf8_lossy(other)
                        ),
                    }
                }
            }
            b"SSND" if body.len() >= 8 => {
                let offset = u32_be(&body[0..]) as usize + 8;
                let pcm = body.get(offset..).unwrap_or_default();
                // Les flottants AIFF sont gros-boutistes : on les retourne comme des entiers.
                samples = Some(to_mono(pcm, channels, bits, float, !little_endian)?);
            }
            _ => {}
        }
    }
    Ok(Decoded {
        samples: samples.context("AIFF sans données")?,
        rate,
        loop_frames: None,
    })
}

/// Flottant étendu IEEE 754 sur 80 bits (fréquence d'échantillonnage des AIFF).
fn extended_to_f64(b: &[u8]) -> f64 {
    let exponent = (u16::from_be_bytes([b[0], b[1]]) & 0x7FFF) as i32;
    let mantissa = u64::from_be_bytes(b[2..10].try_into().unwrap());
    let sign = if b[0] & 0x80 != 0 { -1.0 } else { 1.0 };
    if exponent == 0 && mantissa == 0 {
        return 0.0;
    }
    sign * mantissa as f64 * 2f64.powi(exponent - 16383 - 63)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tonal_waveforms_loop_over_one_cycle() {
        for w in ["sine", "square", "pulse", "saw", "triangle"] {
            let s = generate(w, 32).unwrap();
            assert_eq!(s.data.len(), 32, "{w}");
            assert_eq!((s.loop_start, s.loop_length), (0, 16), "{w}");
            assert!(
                s.data.iter().any(|&x| x > 100) && s.data.iter().any(|&x| x < -100),
                "{w}"
            );
        }
    }

    #[test]
    fn drums_are_one_shots_that_fade_out() {
        for w in ["kick", "snare", "hihat"] {
            let s = generate(w, 32).unwrap();
            assert_eq!(s.loop_length, 1, "{w}");
            let tail = &s.data[s.data.len() - 50..];
            assert!(tail.iter().all(|x| x.abs() < 8), "{w} ne s'éteint pas");
        }
        assert!(generate("gong", 32).is_err());
    }

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("smpltrckr-{}-{name}", std::process::id()))
    }

    #[test]
    fn imports_a_stereo_wav() {
        let path = temp("stereo.wav");
        // Stéréo 16 bits : gauche à 0,5, droite à 0 → mono à 0,25.
        let frames: Vec<f32> = (0..1000).flat_map(|_| [0.5, 0.0]).collect();
        crate::wav::write(&path, &frames, 2, 16574).unwrap();
        let report = import(&path, false).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(report.sample.data.len(), 1000);
        assert!(
            report.sample.data.iter().all(|&x| x == 32),
            "{:?}",
            &report.sample.data[..4]
        );
        assert_eq!(report.natural_note.as_deref(), Some("C-3"));
        assert!(!report.truncated);
    }

    #[test]
    fn imports_an_aiff_and_halves_it() {
        // AIFF mono 16 bits à 44 100 Hz, 4 trames.
        let mut ssnd = vec![0u8; 8];
        for v in [16384i16, 16384, -16384, -16384] {
            ssnd.extend_from_slice(&v.to_be_bytes());
        }
        let mut comm = Vec::new();
        comm.extend_from_slice(&1u16.to_be_bytes());
        comm.extend_from_slice(&4u32.to_be_bytes());
        comm.extend_from_slice(&16u16.to_be_bytes());
        comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]); // 44 100 Hz
        let mut file = b"FORM\0\0\0\0AIFF".to_vec();
        for (id, body) in [(b"COMM", &comm), (b"SSND", &ssnd)] {
            file.extend_from_slice(id);
            file.extend_from_slice(&(body.len() as u32).to_be_bytes());
            file.extend_from_slice(body);
        }
        let path = temp("mono.aiff");
        std::fs::write(&path, &file).unwrap();
        let report = import(&path, true).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(report.rate, 22050);
        assert_eq!(report.sample.data, vec![64, -64]);
        assert_eq!(
            report.sample.display_name(),
            format!("smpltrckr-{}-mono", std::process::id())
        );
    }

    #[test]
    fn long_files_are_truncated() {
        let path = temp("long.wav");
        crate::wav::write(&path, &vec![0.1; MAX_SAMPLE_BYTES + 10], 1, 16574).unwrap();
        let report = import(&path, false).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(report.truncated);
        assert_eq!(report.sample.length_words, 65_535);
    }
}
