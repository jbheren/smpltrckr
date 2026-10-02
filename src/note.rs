//! Notes ProTracker et périodes Amiga.

/// Table des périodes ProTracker (finetune 0), de C-1 à B-3.
pub const PERIODS: [u16; 36] = [
    856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453, // octave 1
    428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226, // octave 2
    214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113, // octave 3
];

const NAMES: [&str; 12] = [
    "C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-",
];

/// Horloge Paula PAL, en Hz.
pub const PAULA_CLOCK_PAL: f64 = 7_093_789.2;

/// Index (0..36) d'une note écrite façon ProTracker (`C-1` … `B-3`, `C#2`).
pub fn parse(text: &str) -> Option<usize> {
    let text = text.trim().to_ascii_uppercase();
    if text.len() != 3 {
        return None;
    }
    let semitone = NAMES.iter().position(|n| text.starts_with(n))?;
    let octave = text[2..].parse::<usize>().ok()?;
    (1..=3)
        .contains(&octave)
        .then(|| (octave - 1) * 12 + semitone)
}

/// Nom ProTracker d'un index de note (0..36).
pub fn name(index: usize) -> String {
    format!("{}{}", NAMES[index % 12], index / 12 + 1)
}

/// Fréquence de lecture d'un sample (Hz) pour une période Amiga donnée.
pub fn period_to_hz(period: u16) -> f64 {
    PAULA_CLOCK_PAL / (2.0 * period as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_name_roundtrip() {
        for i in 0..PERIODS.len() {
            assert_eq!(parse(&name(i)), Some(i));
        }
        assert_eq!(parse("c-2"), Some(12));
        assert_eq!(parse("C-4"), None);
        assert_eq!(parse("H-1"), None);
    }

    #[test]
    fn c2_plays_near_8287_hz() {
        let hz = period_to_hz(PERIODS[parse("C-2").unwrap()]);
        assert!((hz - 8287.1).abs() < 0.5, "{hz}");
    }
}
