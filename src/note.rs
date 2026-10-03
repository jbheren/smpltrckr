//! ProTracker notes and Amiga periods.

/// ProTracker period table (finetune 0), from C-1 to B-3.
pub const PERIODS: [u16; 36] = [
    856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453, // octave 1
    428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226, // octave 2
    214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113, // octave 3
];

const NAMES: [&str; 12] = [
    "C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-",
];

/// PAL Paula clock, in Hz.
pub const PAULA_CLOCK_PAL: f64 = 7_093_789.2;

/// Index (0..36) of a note written ProTracker style (`C-1` … `B-3`, `C#2`).
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

/// ProTracker name of a note index (0..36).
pub fn name(index: usize) -> String {
    format!("{}{}", NAMES[index % 12], index / 12 + 1)
}

/// Extended octaves 0 and 4: not in ProTracker, but written by FT2, OpenMPT…
/// The values differ by one or two units from one tracker to the next.
const OCTAVE_0: [u16; 12] = [
    1712, 1616, 1525, 1440, 1357, 1281, 1209, 1141, 1077, 1017, 961, 907,
];
const OCTAVE_4: [u16; 12] = [107, 101, 95, 90, 85, 80, 76, 71, 67, 64, 60, 57];

/// Name of the note played by a period found in a pattern, extended octaves included.
/// Octaves 0 and 4 are matched within one or two units (read only).
pub fn name_for_period(period: u16) -> Option<String> {
    if let Some(i) = PERIODS.iter().position(|&p| p == period) {
        return Some(name(i));
    }
    [(0, &OCTAVE_0, 2), (4, &OCTAVE_4, 1)]
        .into_iter()
        .find_map(|(octave, table, tolerance)| {
            let i = table
                .iter()
                .position(|&p| p.abs_diff(period) <= tolerance)?;
            Some(format!("{}{octave}", NAMES[i]))
        })
}

/// Sample playback rate (Hz) for a given Amiga period.
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
    fn names_extended_octaves() {
        assert_eq!(name_for_period(428).as_deref(), Some("C-2"));
        assert_eq!(name_for_period(107).as_deref(), Some("C-4"));
        assert_eq!(name_for_period(56).as_deref(), Some("B-4"));
        assert_eq!(name_for_period(1211).as_deref(), Some("F#0"));
        assert_eq!(name_for_period(1000), None);
    }

    #[test]
    fn c2_plays_near_8287_hz() {
        let hz = period_to_hz(PERIODS[parse("C-2").unwrap()]);
        assert!((hz - 8287.1).abs() < 0.5, "{hz}");
    }
}
