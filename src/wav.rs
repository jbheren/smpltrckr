//! Écriture de fichiers WAV PCM 16 bits.

use std::io::Write;

/// Écrit des échantillons flottants (entrelacés si `channels` > 1) en WAV PCM 16 bits.
pub fn write(
    path: &std::path::Path,
    samples: &[f32],
    channels: u16,
    rate: u32,
) -> std::io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &x in samples {
        out.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    std::fs::File::create(path)?.write_all(&out)
}

/// Lit un WAV PCM 16 bits ou flottant 32 bits et le renvoie en mono, avec sa fréquence.
pub fn read_mono(path: &std::path::Path) -> anyhow::Result<(Vec<f32>, u32)> {
    let data = std::fs::read(path)?;
    anyhow::ensure!(
        data.len() > 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WAVE",
        "pas un WAV"
    );
    let (mut format, mut channels, mut rate, mut bits) = (0u16, 0u16, 0u32, 0u16);
    let mut pos = 12;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let len = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = &data[pos + 8..(pos + 8 + len).min(data.len())];
        if id == b"fmt " {
            format = u16::from_le_bytes([body[0], body[1]]);
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            bits = u16::from_le_bytes([body[14], body[15]]);
            if format == 0xFFFE {
                format = u16::from_le_bytes([body[24], body[25]]); // WAVE_FORMAT_EXTENSIBLE
            }
        } else if id == b"data" {
            let frames: Vec<f32> = match (format, bits) {
                (1, 16) => body
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| i16::from_le_bytes(*b) as f32 / 32768.0)
                    .collect(),
                (3, 32) => body
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect(),
                _ => anyhow::bail!("WAV non pris en charge (format {format}, {bits} bits)"),
            };
            let ch = channels.max(1) as usize;
            let mono = frames
                .chunks_exact(ch)
                .map(|f| f.iter().sum::<f32>() / ch as f32)
                .collect();
            return Ok((mono, rate));
        }
        pos += 8 + len + (len & 1);
    }
    anyhow::bail!("WAV sans données")
}

#[cfg(test)]
mod tests {
    #[test]
    fn write_then_read_back() {
        let path = std::env::temp_dir().join(format!("smpltrckr-test-{}.wav", std::process::id()));
        super::write(&path, &[0.5, -0.5, 0.25, -0.25], 2, 44100).unwrap();
        let (mono, rate) = super::read_mono(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(rate, 44100);
        assert_eq!(mono.len(), 2);
        assert!(mono.iter().all(|x| x.abs() < 1e-3));
    }
}
