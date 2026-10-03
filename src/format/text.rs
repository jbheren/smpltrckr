//! ProTracker-style text notation for patterns: the language spoken with the agent.
//!
//! ```text
//! 00 | C-3 01 ... | ... .. ... | E-3 02 C20 | ... .. ...
//! ```
//! A cell: note (`C-3`, `C#2`, `...`), sample in decimal (`01` to `31`, `..`), effect in hex
//! (`C20`, `...`). Octaves 1 to 3 (ProTracker's) can be written. Extended octaves 0 and 4
//! (FT2, OpenMPT…) are shown but cannot be written; an unknown period shows as `???`.

use std::fmt::Write;

use anyhow::{Context, bail, ensure};
use rust_i18n::t;

use crate::note;
use crate::song::{Cell, Pattern, Song};

pub fn cell_to_text(c: &Cell) -> String {
    let note = match c.period {
        0 => "...".to_string(),
        p => note::name_for_period(p).unwrap_or_else(|| "???".to_string()),
    };
    let sample = if c.sample == 0 {
        "..".to_string()
    } else {
        format!("{:02}", c.sample)
    };
    let effect = if c.effect == 0 && c.param == 0 {
        "...".to_string()
    } else {
        format!("{:X}{:02X}", c.effect, c.param)
    };
    format!("{note} {sample} {effect}")
}

pub fn parse_cell(text: &str) -> anyhow::Result<Cell> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    let [note, sample, effect] = parts[..] else {
        bail!(t!("text.bad_cell", text = format!("{text:?}")));
    };

    let period = match note {
        "..." => 0,
        "???" => bail!(t!("text.unknown_note_marker")),
        // An effect typed in the note column (say "F00 .. ..."): show the right form.
        n if note::parse(n).is_none()
            && n.len() == 3
            && n.chars().all(|c| c.is_ascii_hexdigit()) =>
        {
            bail!(t!("text.effect_in_note_column", effect = n))
        }
        n if note::parse(n).is_none() && n.ends_with(['0', '4']) => {
            bail!(t!("text.note_out_of_range", note = n))
        }
        n => note::PERIODS[note::parse(n).with_context(|| t!("text.unknown_note", note = n))?],
    };
    let sample = match sample {
        ".." => 0,
        s => {
            let n: u8 = s
                .parse()
                .with_context(|| t!("text.bad_sample", sample = s))?;
            ensure!(
                (1..=31).contains(&n),
                t!("text.sample_out_of_range", sample = n)
            );
            n
        }
    };
    let (effect, param) = match effect {
        "..." => (0, 0),
        e if e.len() == 3 => {
            let value =
                u16::from_str_radix(e, 16).with_context(|| t!("text.bad_effect", effect = e))?;
            ((value >> 8) as u8, value as u8)
        }
        e => bail!(t!("text.bad_effect", effect = e)),
    };
    Ok(Cell {
        period,
        sample,
        effect,
        param,
    })
}

pub fn row_to_text(index: usize, row: &[Cell]) -> String {
    let mut line = format!("{index:02}");
    for cell in row {
        write!(line, " | {}", cell_to_text(cell)).unwrap();
    }
    line
}

/// Reads a `NN | cell | cell …` line and returns (row number, cells).
pub fn parse_row(line: &str) -> anyhow::Result<(usize, Vec<Cell>)> {
    let mut fields = line.split('|');
    let index = fields.next().unwrap_or_default().trim();
    let index: usize = index
        .parse()
        .with_context(|| t!("text.bad_row_number", row = index))?;
    let cells = fields
        .map(parse_cell)
        .collect::<anyhow::Result<Vec<_>>>()
        .with_context(|| t!("text.row", row = format!("{index:02}")))?;
    Ok((index, cells))
}

pub fn pattern_to_text(index: usize, pattern: &Pattern) -> String {
    let mut out = format!("# pattern {index:02}\n");
    for (i, row) in pattern.rows.iter().enumerate() {
        out.push_str(&row_to_text(i, row));
        out.push('\n');
    }
    out
}

/// Reads a whole pattern. Blank lines and comments (`#`) are skipped; missing rows stay
/// empty.
pub fn parse_pattern(text: &str, rows: usize, channels: usize) -> anyhow::Result<Pattern> {
    let mut pattern = Pattern::new(rows, channels);
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let (index, cells) = parse_row(line)?;
        ensure!(
            index < rows,
            t!("text.row_out_of_pattern", row = index, last = rows - 1)
        );
        ensure!(
            cells.len() == channels,
            t!(
                "text.wrong_voice_count",
                row = format!("{index:02}"),
                count = cells.len(),
                expected = channels
            )
        );
        pattern.rows[index] = cells;
    }
    Ok(pattern)
}

/// Readable overview of a song: header, order list, samples, then patterns.
pub fn song_to_text(song: &Song, patterns: impl IntoIterator<Item = usize>) -> String {
    let mut out = String::new();
    let kind = match song.kind {
        crate::song::ModKind::Soundtracker15 => "Soundtracker 15 samples".to_string(),
        crate::song::ModKind::Tagged(tag) => String::from_utf8_lossy(&tag).into_owned(),
    };
    writeln!(out, "{}", t!("dump.title", title = song.display_title())).unwrap();
    let format = t!(
        "dump.format",
        kind = kind,
        voices = song.channels,
        patterns = song.patterns.len()
    );
    writeln!(out, "{format}").unwrap();
    let orders: Vec<String> = song
        .order_list()
        .iter()
        .map(|o| format!("{o:02}"))
        .collect();
    writeln!(out, "{}", t!("dump.orders", orders = orders.join(" "))).unwrap();
    writeln!(out, "{}", t!("dump.samples")).unwrap();
    for (i, s) in song.samples.iter().enumerate() {
        if s.length_words == 0 && s.display_name().trim().is_empty() {
            continue;
        }
        let looped = if s.loop_length > 1 {
            t!(
                "dump.loop",
                start = s.loop_start as u32 * 2,
                length = s.loop_length as u32 * 2
            )
            .into_owned()
        } else {
            String::new()
        };
        let details = t!(
            "dump.sample",
            bytes = format!("{:>6}", s.length_words as u32 * 2),
            volume = format!("{:>2}", s.volume),
            finetune = format!("{:+}", s.finetune())
        );
        writeln!(
            out,
            "  {:02} {:<22} {details}{looped}",
            i + 1,
            s.display_name()
        )
        .unwrap();
    }
    for p in patterns {
        out.push('\n');
        out.push_str(&pattern_to_text(p, &song.patterns[p]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_text_roundtrip() {
        for text in [
            "C-3 01 A04",
            "... .. ...",
            "C#2 .. ...",
            "... 31 F06",
            "B-1 12 E91",
        ] {
            assert_eq!(cell_to_text(&parse_cell(text).unwrap()), text);
        }
    }

    #[test]
    fn odd_periods_are_shown_but_not_written() {
        let cell = |period| Cell {
            period,
            sample: 1,
            effect: 0,
            param: 0,
        };
        assert_eq!(cell_to_text(&cell(1712)), "C-0 01 ...");
        assert_eq!(cell_to_text(&cell(1000)), "??? 01 ...");
        assert!(parse_cell("C-0 01 ...").is_err());
        let misplaced = parse_cell("F00 .. ...").unwrap_err().to_string();
        assert!(misplaced.contains("... .. F00"), "{misplaced}");
    }

    #[test]
    fn rejects_bad_cells() {
        for bad in [
            "C-4 01 ...",
            "C-3 32 ...",
            "C-3 01 G00",
            "C-3 01",
            "??? 01 ...",
        ] {
            assert!(parse_cell(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn pattern_text_roundtrip() {
        let mut p = Pattern::new(64, 4);
        p.rows[0][0] = parse_cell("C-3 01 ...").unwrap();
        p.rows[2][3] = parse_cell("... .. F06").unwrap();
        let text = pattern_to_text(0, &p);
        assert_eq!(parse_pattern(&text, 64, 4).unwrap(), p);
    }

    #[test]
    fn pattern_errors_name_the_row() {
        let err = parse_pattern("03 | C-3 01 ... | X", 64, 2).unwrap_err();
        assert!(format!("{err:#}").contains("row 03"), "{err:#}");
    }
}
