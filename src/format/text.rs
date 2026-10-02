//! Notation texte des patterns, façon ProTracker. C'est le format d'échange avec l'agent.
//!
//! ```text
//! 00 | C-3 01 ... | ... .. ... | E-3 02 C20 | ... .. ...
//! ```
//! Une cellule : note (`C-3`, `C#2`, `...`), sample en décimal (`01` à `31`, `..`),
//! effet en hexadécimal (`C20`, `...`). On écrit les octaves 1 à 3 (celles de ProTracker).
//! Les octaves étendues 0 et 4 (FT2, OpenMPT…) s'affichent mais ne s'écrivent pas ;
//! une période non reconnue s'affiche `???`.

use std::fmt::Write;

use anyhow::{Context, bail, ensure};

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
        bail!("cellule attendue sous la forme « C-3 01 A04 », reçu {text:?}");
    };

    let period = match note {
        "..." => 0,
        "???" => bail!("« ??? » ne peut pas être écrit : utiliser une note de C-1 à B-3"),
        n if note::parse(n).is_none() && n.ends_with(['0', '4']) => {
            bail!("note {n:?} hors de la plage ProTracker : utiliser une note de C-1 à B-3")
        }
        n => {
            note::PERIODS
                [note::parse(n).with_context(|| format!("note inconnue {n:?} (C-1 à B-3)"))?]
        }
    };
    let sample = match sample {
        ".." => 0,
        s => {
            let n: u8 = s
                .parse()
                .with_context(|| format!("sample invalide {s:?}"))?;
            ensure!((1..=31).contains(&n), "sample hors limites {n} (1 à 31)");
            n
        }
    };
    let (effect, param) = match effect {
        "..." => (0, 0),
        e if e.len() == 3 => {
            let value =
                u16::from_str_radix(e, 16).with_context(|| format!("effet invalide {e:?}"))?;
            ((value >> 8) as u8, value as u8)
        }
        e => bail!("effet invalide {e:?} (3 chiffres hexadécimaux, ex. A04)"),
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

/// Lit une ligne `NN | cellule | cellule …` et renvoie (numéro, cellules).
pub fn parse_row(line: &str) -> anyhow::Result<(usize, Vec<Cell>)> {
    let mut fields = line.split('|');
    let index = fields.next().unwrap_or_default().trim();
    let index: usize = index
        .parse()
        .with_context(|| format!("numéro de ligne invalide {index:?}"))?;
    let cells = fields
        .map(parse_cell)
        .collect::<anyhow::Result<Vec<_>>>()
        .with_context(|| format!("ligne {index:02}"))?;
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

/// Lit un pattern complet. Les lignes vides et les commentaires (`#`) sont ignorés ;
/// les lignes absentes restent vides.
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
            "ligne {index} hors du pattern (0 à {})",
            rows - 1
        );
        ensure!(
            cells.len() == channels,
            "ligne {index:02} : {} voies au lieu de {channels}",
            cells.len()
        );
        pattern.rows[index] = cells;
    }
    Ok(pattern)
}

/// Vue d'ensemble lisible d'un morceau : en-tête, ordre, samples, et patterns.
pub fn song_to_text(song: &Song, patterns: impl IntoIterator<Item = usize>) -> String {
    let mut out = String::new();
    let kind = match song.kind {
        crate::song::ModKind::Soundtracker15 => "Soundtracker 15 samples".to_string(),
        crate::song::ModKind::Tagged(tag) => String::from_utf8_lossy(&tag).into_owned(),
    };
    writeln!(out, "titre    : {}", song.display_title()).unwrap();
    writeln!(
        out,
        "format   : {kind}, {} voies, {} patterns",
        song.channels,
        song.patterns.len()
    )
    .unwrap();
    let orders: Vec<String> = song
        .order_list()
        .iter()
        .map(|o| format!("{o:02}"))
        .collect();
    writeln!(out, "ordre    : {}", orders.join(" ")).unwrap();
    writeln!(out, "samples  :").unwrap();
    for (i, s) in song.samples.iter().enumerate() {
        if s.length_words == 0 && s.display_name().trim().is_empty() {
            continue;
        }
        let looped = if s.loop_length > 1 {
            format!(
                ", boucle {}+{}",
                s.loop_start as u32 * 2,
                s.loop_length as u32 * 2
            )
        } else {
            String::new()
        };
        writeln!(
            out,
            "  {:02} {:<22} {:>6} octets, vol {:>2}, finetune {:+}{looped}",
            i + 1,
            s.display_name(),
            s.length_words as u32 * 2,
            s.volume,
            s.finetune()
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
        assert!(format!("{err:#}").contains("ligne 03"), "{err:#}");
    }
}
