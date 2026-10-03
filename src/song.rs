//! In-memory model of a song.
//!
//! The model keeps the file's raw values (names as bytes, periods, trailing bytes), so that a
//! `.mod` loaded then saved again stays identical down to the last byte.
//! Voice and row counts are variable, to keep the door open for XM.

/// A pattern cell: note (Amiga period), sample, effect.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cell {
    /// Amiga period (12 bits), 0 = no note.
    pub period: u16,
    /// Sample number (1 to 31), 0 = no sample.
    pub sample: u8,
    /// Effect (0 to F).
    pub effect: u8,
    /// Effect parameter.
    pub param: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    /// `rows[row][voice]`.
    pub rows: Vec<Vec<Cell>>,
}

impl Pattern {
    pub fn new(rows: usize, channels: usize) -> Self {
        Self {
            rows: vec![vec![Cell::default(); channels]; rows],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    /// Raw name (22 bytes, not necessarily zero-terminated).
    pub name: [u8; 22],
    /// Length announced by the header, in 16-bit words.
    /// May exceed `data` when the file is truncated.
    pub length_words: u16,
    /// Raw finetune byte (only the low 4 bits matter: -8 to +7).
    pub finetune: u8,
    /// Volume (0 to 64).
    pub volume: u8,
    /// Loop start, in words.
    pub loop_start: u16,
    /// Loop length, in words (1 = no loop).
    pub loop_length: u16,
    /// Signed 8-bit PCM data, as found in the file.
    pub data: Vec<i8>,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            name: [0; 22],
            length_words: 0,
            finetune: 0,
            volume: 0,
            loop_start: 0,
            loop_length: 1,
            data: Vec::new(),
        }
    }
}

impl Sample {
    pub fn display_name(&self) -> String {
        text_from_bytes(&self.name)
    }

    /// Signed finetune, from -8 to +7.
    pub fn finetune(&self) -> i8 {
        ((self.finetune & 0x0F) as i8) << 4 >> 4
    }

    pub fn set_name(&mut self, name: &str) {
        self.name = bytes_from_text(name);
    }

    /// Replaces the data and keeps the header length consistent
    /// (an even number of bytes, at most 65,535 words).
    pub fn set_data(&mut self, mut data: Vec<i8>) {
        data.truncate(MAX_SAMPLE_BYTES);
        if data.len() % 2 == 1 {
            data.push(0);
        }
        self.length_words = (data.len() / 2) as u16;
        self.data = data;
    }
}

/// Largest sample a `.mod` can hold: 65,535 16-bit words.
pub const MAX_SAMPLE_BYTES: usize = 65_535 * 2;

/// Flavour of the `.mod` format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModKind {
    /// The original Soundtracker: 15 samples, no tag, 4 voices.
    Soundtracker15,
    /// ProTracker and friends: 31 samples and a 4-byte tag (`M.K.`, `8CHN`…).
    Tagged([u8; 4]),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Song {
    /// Raw title (20 bytes).
    pub title: [u8; 20],
    pub kind: ModKind,
    pub channels: usize,
    pub samples: Vec<Sample>,
    /// Number of positions played in `orders`.
    pub song_length: u8,
    /// Restart byte (127 in ProTracker, a restart position elsewhere).
    pub restart: u8,
    /// Full order list, unused positions included.
    pub orders: [u8; 128],
    pub patterns: Vec<Pattern>,
    /// Bytes found after the sample data, kept as they are.
    pub trailing: Vec<u8>,
}

impl Song {
    /// Empty ProTracker `M.K.` song: 4 voices, 31 empty samples, one pattern.
    pub fn new(title: &str) -> Self {
        Self {
            title: bytes_from_text(title),
            kind: ModKind::Tagged(*b"M.K."),
            channels: 4,
            samples: vec![Sample::default(); 31],
            song_length: 1,
            restart: 127,
            orders: [0; 128],
            patterns: vec![Pattern::new(64, 4)],
            trailing: Vec::new(),
        }
    }

    pub fn set_title(&mut self, title: &str) {
        self.title = bytes_from_text(title);
    }

    pub fn display_title(&self) -> String {
        text_from_bytes(&self.title)
    }

    /// The positions of the order list that are actually played.
    pub fn order_list(&self) -> &[u8] {
        &self.orders[..(self.song_length as usize).min(128)]
    }
}

/// Name field from a text: Latin-1 characters (others become `?`), truncated to the field
/// size and padded with zeros.
fn bytes_from_text<const N: usize>(text: &str) -> [u8; N] {
    let mut out = [0u8; N];
    for (slot, c) in out.iter_mut().zip(text.chars()) {
        *slot = u8::try_from(c as u32).unwrap_or(b'?');
    }
    out
}

/// Readable text from a name field: cut at the first zero, bytes read as Latin-1.
fn text_from_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_song_is_a_valid_empty_mod() {
        let song = Song::new("Démo");
        assert_eq!(song.display_title(), "Démo");
        assert_eq!(song.order_list(), &[0]);
        let bytes = crate::format::protracker::write(&song);
        assert_eq!(bytes.len(), 1084 + 1024);
        assert_eq!(crate::format::protracker::read(&bytes).unwrap(), song);
    }

    #[test]
    fn names_are_truncated_and_padded() {
        let mut sample = Sample::default();
        sample.set_name("une basse très très très longue");
        assert_eq!(sample.display_name(), "une basse très très tr");
        sample.set_data(vec![1, 2, 3]);
        assert_eq!((sample.length_words, sample.data.len()), (2, 4));
    }
}
