//! Reading and writing `.mod` files (15-sample Soundtracker, ProTracker and N-voice flavours).
//!
//! File layout:
//! title (20) · samples (15 or 31 × 30) · length (1) · restart (1) · orders (128)
//! · tag (4, missing with 15 samples) · patterns (64 rows × N voices × 4 bytes)
//! · sample data · trailing bytes, if any.

use anyhow::{Context, bail, ensure};
use rust_i18n::t;

use crate::song::{Cell, ModKind, Pattern, Sample, Song};

const ROWS: usize = 64;

/// Voice count announced by a tag, when the tag is known.
fn channels_for_tag(tag: &[u8; 4]) -> Option<usize> {
    let digit = |b: u8| b.is_ascii_digit().then(|| (b - b'0') as usize);
    match tag {
        b"M.K." | b"M!K!" | b"M&K!" | b"N.T." | b"FLT4" => Some(4),
        b"CD81" | b"OKTA" | b"OCTA" => Some(8),
        [n, b'C', b'H', b'N'] => digit(*n),
        [a, b, b'C', b'H'] => Some(digit(*a)? * 10 + digit(*b)?),
        [b'T', b'D', b'Z', n] => digit(*n),
        _ => None,
    }
    .filter(|&n| (1..=32).contains(&n))
}

/// Read cursor that clearly reports a file that is too short.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> anyhow::Result<&'a [u8]> {
        let end = self.pos + n;
        ensure!(
            end <= self.data.len(),
            t!("mod.truncated", offset = self.data.len())
        );
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self) -> anyhow::Result<[u8; N]> {
        Ok(self.take(N)?.try_into().unwrap())
    }

    fn u8(&mut self) -> anyhow::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16_be(&mut self) -> anyhow::Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
}

pub fn read(data: &[u8]) -> anyhow::Result<Song> {
    ensure!(!data.starts_with(b"PP20"), t!("mod.powerpacker"),);
    let tag: Option<[u8; 4]> = data.get(1080..1084).map(|t| t.try_into().unwrap());
    let (kind, channels, sample_count) = match tag {
        Some(tag) if tag == *b"FLT8" => bail!(t!("mod.flt8")),
        Some(tag) => match channels_for_tag(&tag) {
            Some(channels) => (ModKind::Tagged(tag), channels, 31),
            None => (ModKind::Soundtracker15, 4, 15),
        },
        None => (ModKind::Soundtracker15, 4, 15),
    };

    let mut r = Reader { data, pos: 0 };
    let title = r.array()?;
    let mut samples = Vec::with_capacity(sample_count);
    for _ in 0..sample_count {
        samples.push(Sample {
            name: r.array()?,
            length_words: r.u16_be()?,
            finetune: r.u8()?,
            volume: r.u8()?,
            loop_start: r.u16_be()?,
            loop_length: r.u16_be()?,
            data: Vec::new(),
        });
    }
    let song_length = r.u8()?;
    let restart = r.u8()?;
    let orders: [u8; 128] = r.array()?;
    if let ModKind::Tagged(_) = kind {
        r.take(4)?;
    }

    let pattern_size = ROWS * channels * 4;
    let pattern_count = pattern_count(&orders, song_length, r.remaining(), pattern_size);
    if kind == ModKind::Soundtracker15 {
        // No tag: check that the header looks sane before trusting it.
        ensure!(
            (1..=128).contains(&song_length)
                && pattern_count <= 64
                && samples.iter().all(|s| s.volume <= 64),
            t!("mod.unknown_format"),
        );
    }

    let mut patterns = Vec::with_capacity(pattern_count);
    for p in 0..pattern_count {
        let bytes = r
            .take(pattern_size)
            .with_context(|| format!("pattern {p}"))?;
        let rows = bytes
            .chunks(channels * 4)
            .map(|row| row.chunks(4).map(decode_cell).collect())
            .collect();
        patterns.push(Pattern { rows });
    }

    // Sample data. A truncated file keeps whatever is there.
    for sample in &mut samples {
        let len = (sample.length_words as usize * 2).min(r.remaining());
        sample.data = r.take(len)?.iter().map(|&b| b as i8).collect();
    }
    let trailing = r.take(r.remaining())?.to_vec();

    Ok(Song {
        title,
        kind,
        channels,
        samples,
        song_length,
        restart,
        orders,
        patterns,
        trailing,
    })
}

/// Number of stored patterns. ProTracker takes the highest number of the whole order list
/// (128 positions). Some files carry junk past the song length: in that case, if it does
/// not fit in the file, only the played positions count.
fn pattern_count(
    orders: &[u8; 128],
    song_length: u8,
    remaining: usize,
    pattern_size: usize,
) -> usize {
    let all = orders.iter().max().map_or(0, |&m| m as usize + 1);
    if all * pattern_size <= remaining {
        return all;
    }
    let played = &orders[..(song_length as usize).clamp(1, 128)];
    played.iter().max().map_or(0, |&m| m as usize + 1)
}

fn decode_cell(b: &[u8]) -> Cell {
    Cell {
        period: (((b[0] & 0x0F) as u16) << 8) | b[1] as u16,
        sample: (b[0] & 0xF0) | (b[2] >> 4),
        effect: b[2] & 0x0F,
        param: b[3],
    }
}

fn encode_cell(c: &Cell) -> [u8; 4] {
    [
        (c.sample & 0xF0) | ((c.period >> 8) as u8 & 0x0F),
        c.period as u8,
        (c.sample << 4) | (c.effect & 0x0F),
        c.param,
    ]
}

pub fn write(song: &Song) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&song.title);
    for s in &song.samples {
        out.extend_from_slice(&s.name);
        out.extend_from_slice(&s.length_words.to_be_bytes());
        out.push(s.finetune);
        out.push(s.volume);
        out.extend_from_slice(&s.loop_start.to_be_bytes());
        out.extend_from_slice(&s.loop_length.to_be_bytes());
    }
    out.push(song.song_length);
    out.push(song.restart);
    out.extend_from_slice(&song.orders);
    if let ModKind::Tagged(tag) = song.kind {
        out.extend_from_slice(&tag);
    }
    for pattern in &song.patterns {
        for cell in pattern.rows.iter().flatten() {
            out.extend_from_slice(&encode_cell(cell));
        }
    }
    for s in &song.samples {
        out.extend(s.data.iter().map(|&b| b as u8));
    }
    out.extend_from_slice(&song.trailing);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small hand-made M.K. module: 1 sample, 2 patterns.
    fn tiny_mod() -> Vec<u8> {
        tiny_mod_with(b"M.K.", 4)
    }

    fn tiny_mod_with(tag: &[u8; 4], channels: usize) -> Vec<u8> {
        let mut d = vec![0u8; 20];
        d[..4].copy_from_slice(b"test");
        for i in 0..31 {
            let mut h = [0u8; 30];
            if i == 0 {
                h[..5].copy_from_slice(b"chip!");
                h[22..24].copy_from_slice(&4u16.to_be_bytes()); // 8 bytes
                h[24] = 0x0F; // finetune -1
                h[25] = 64;
            }
            h[28..30].copy_from_slice(&1u16.to_be_bytes());
            d.extend_from_slice(&h);
        }
        d.push(2);
        d.push(127);
        let mut orders = [0u8; 128];
        orders[1] = 1;
        d.extend_from_slice(&orders);
        d.extend_from_slice(tag);
        let mut patterns = vec![0u8; 2 * 64 * channels * 4];
        patterns[..4].copy_from_slice(&encode_cell(&Cell {
            period: 428,
            sample: 1,
            effect: 0xC,
            param: 0x20,
        }));
        d.extend_from_slice(&patterns);
        d.extend_from_slice(&[0, 64, 127, 64, 0, 0x80, 0xC0, 0x80]);
        d
    }

    #[test]
    fn reads_header_patterns_and_samples() {
        let song = read(&tiny_mod()).unwrap();
        assert_eq!(song.display_title(), "test");
        assert_eq!(song.channels, 4);
        assert_eq!(song.patterns.len(), 2);
        assert_eq!(song.order_list(), &[0, 1]);
        assert_eq!(song.samples[0].display_name(), "chip!");
        assert_eq!(song.samples[0].finetune(), -1);
        assert_eq!(
            song.samples[0].data,
            vec![0, 64, 127, 64, 0, -128, -64, -128]
        );
        assert_eq!(
            song.patterns[0].rows[0][0],
            Cell {
                period: 428,
                sample: 1,
                effect: 0xC,
                param: 0x20
            }
        );
    }

    #[test]
    fn reads_and_writes_other_variants() {
        for (tag, channels) in [
            (b"M!K!", 4),
            (b"FLT4", 4),
            (b"6CHN", 6),
            (b"8CHN", 8),
            (b"12CH", 12),
        ] {
            let original = tiny_mod_with(tag, channels);
            let song = read(&original).unwrap();
            assert_eq!(song.channels, channels, "{}", String::from_utf8_lossy(tag));
            assert_eq!(song.patterns[0].rows[0].len(), channels);
            assert_eq!(song.samples[0].data.len(), 8);
            assert_eq!(write(&song), original);
        }
    }

    #[test]
    fn roundtrip_is_byte_exact() {
        let original = tiny_mod();
        assert_eq!(write(&read(&original).unwrap()), original);
    }

    #[test]
    fn keeps_truncated_samples_and_trailing_bytes() {
        let mut short = tiny_mod();
        short.truncate(short.len() - 3);
        assert_eq!(write(&read(&short).unwrap()), short);

        let mut long = tiny_mod();
        long.extend_from_slice(b"extra");
        let song = read(&long).unwrap();
        assert_eq!(song.trailing, b"extra");
        assert_eq!(write(&song), long);
    }

    #[test]
    fn cell_encoding_keeps_high_sample_bits() {
        for cell in [
            Cell {
                period: 0xFFF,
                sample: 31,
                effect: 0xF,
                param: 0xFF,
            },
            Cell {
                period: 113,
                sample: 0x10,
                effect: 0,
                param: 0,
            },
        ] {
            assert_eq!(decode_cell(&encode_cell(&cell)), cell);
        }
    }

    #[test]
    fn recognises_channel_tags() {
        assert_eq!(channels_for_tag(b"M.K."), Some(4));
        assert_eq!(channels_for_tag(b"6CHN"), Some(6));
        assert_eq!(channels_for_tag(b"16CH"), Some(16));
        assert_eq!(channels_for_tag(b"OCTA"), Some(8));
        assert_eq!(channels_for_tag(b"\0\0\0\0"), None);
    }

    #[test]
    fn rejects_garbage() {
        assert!(read(b"pas un module").is_err());
        assert!(read(&vec![0xAB; 3000]).is_err());
    }
}
