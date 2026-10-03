//! Editor: every change to a song goes through here, whether it comes from the keyboard or
//! from the agent. Each change can be undone and redone, and leaves a line in the journal
//! (the ship's log, if you will).
//!
//! A change is a list of replacements (`Change`). Applying one returns its inverse, so undo
//! never needs to copy the whole song.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::ensure;
use rust_i18n::t;

use crate::replayer::Mixer;
use crate::song::{Pattern, Sample, Song};

/// Who asked for the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Keyboard,
    Agent,
}

/// One elementary replacement in the song.
#[derive(Debug, Clone)]
pub enum Change {
    Title([u8; 20]),
    /// Replaces a pattern, appends one (at the end) or removes the last one (`None`).
    Pattern(usize, Option<Pattern>),
    Orders {
        length: u8,
        orders: [u8; 128],
    },
    Sample(usize, Box<Sample>),
}

impl Change {
    /// Applies the replacement and returns the one that undoes it.
    fn apply(self, song: &mut Song) -> Change {
        match self {
            Change::Title(title) => Change::Title(std::mem::replace(&mut song.title, title)),
            Change::Pattern(i, Some(pattern)) if i == song.patterns.len() => {
                song.patterns.push(pattern);
                Change::Pattern(i, None)
            }
            Change::Pattern(i, Some(pattern)) => {
                Change::Pattern(i, Some(std::mem::replace(&mut song.patterns[i], pattern)))
            }
            Change::Pattern(i, None) => Change::Pattern(i, song.patterns.pop()),
            Change::Orders { length, orders } => {
                let old = Change::Orders {
                    length: song.song_length,
                    orders: song.orders,
                };
                song.song_length = length;
                song.orders = orders;
                old
            }
            Change::Sample(i, sample) => Change::Sample(
                i,
                Box::new(std::mem::replace(&mut song.samples[i], *sample)),
            ),
        }
    }
}

/// A whole change, as it shows up in the history.
#[derive(Debug, Clone)]
struct Edit {
    description: String,
    changes: Vec<Change>,
}

/// One line of the journal.
#[derive(Debug, Clone)]
pub struct JournalEntry {
    pub time: SystemTime,
    pub origin: Origin,
    /// "edit", "undo" or "redo" (localized), followed by the description.
    pub text: String,
}

pub struct Editor {
    song: Song,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    journal: Vec<JournalEntry>,
    /// Session mix (not saved in the `.mod`).
    pub mixer: Mixer,
}

const UNDO_DEPTH: usize = 500;

impl Editor {
    pub fn new(song: Song) -> Self {
        let mixer = Mixer::new(song.channels);
        Self {
            song,
            undo: Vec::new(),
            redo: Vec::new(),
            journal: Vec::new(),
            mixer,
        }
    }

    pub fn song(&self) -> &Song {
        &self.song
    }

    pub fn journal(&self) -> &[JournalEntry] {
        &self.journal
    }

    /// Applies a change. An empty list does nothing and leaves no trace.
    pub fn apply(&mut self, origin: Origin, description: impl Into<String>, changes: Vec<Change>) {
        if changes.is_empty() {
            return;
        }
        let description = description.into();
        let inverse = self.apply_all(changes);
        self.log(origin, t!("journal.edit", what = description).into_owned());
        self.undo.push(Edit {
            description,
            changes: inverse,
        });
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Undoes the last change and returns its description.
    pub fn undo(&mut self, origin: Origin) -> Option<String> {
        let edit = self.undo.pop()?;
        let inverse = self.apply_all(edit.changes);
        self.log(
            origin,
            t!("journal.undo", what = edit.description).into_owned(),
        );
        let description = edit.description.clone();
        self.redo.push(Edit {
            changes: inverse,
            ..edit
        });
        Some(description)
    }

    /// Redoes the last undone change and returns its description.
    pub fn redo(&mut self, origin: Origin) -> Option<String> {
        let edit = self.redo.pop()?;
        let inverse = self.apply_all(edit.changes);
        self.log(
            origin,
            t!("journal.redo", what = edit.description).into_owned(),
        );
        let description = edit.description.clone();
        self.undo.push(Edit {
            changes: inverse,
            ..edit
        });
        Some(description)
    }

    /// Replaces the whole song (new song, file opened): the history starts over.
    pub fn replace_song(&mut self, origin: Origin, song: Song, description: impl Into<String>) {
        self.mixer = Mixer::new(song.channels);
        self.song = song;
        self.undo.clear();
        self.redo.clear();
        self.log(origin, description.into());
    }

    pub fn log(&mut self, origin: Origin, text: String) {
        self.journal.push(JournalEntry {
            time: SystemTime::now(),
            origin,
            text,
        });
    }

    /// Applies the replacements in order and returns their inverses, in reverse order.
    fn apply_all(&mut self, changes: Vec<Change>) -> Vec<Change> {
        let mut inverse: Vec<Change> = changes
            .into_iter()
            .map(|c| c.apply(&mut self.song))
            .collect();
        inverse.reverse();
        inverse
    }

    // --- Change builders, with sanity checks ------------------------------------------------

    /// Replaces an existing pattern, or appends one right after the last.
    pub fn set_pattern(&self, index: usize, pattern: Pattern) -> anyhow::Result<Change> {
        let count = self.song.patterns.len();
        ensure!(
            index <= count,
            t!("editor.next_pattern_only", pattern = index, next = count)
        );
        ensure!(index < 128, t!("editor.too_many_patterns"));
        let channels = self.song.channels;
        ensure!(
            pattern.rows.len() == 64 && pattern.rows.iter().all(|r| r.len() == channels),
            t!("editor.pattern_shape", voices = channels)
        );
        Ok(Change::Pattern(index, Some(pattern)))
    }

    pub fn set_orders(&self, orders: &[u8]) -> anyhow::Result<Change> {
        ensure!((1..=128).contains(&orders.len()), t!("editor.order_length"));
        if let Some(&p) = orders
            .iter()
            .find(|&&p| p as usize >= self.song.patterns.len())
        {
            anyhow::bail!(t!(
                "editor.no_such_pattern",
                pattern = p,
                count = self.song.patterns.len()
            ));
        }
        let mut all = [0u8; 128];
        all[..orders.len()].copy_from_slice(orders);
        Ok(Change::Orders {
            length: orders.len() as u8,
            orders: all,
        })
    }

    /// Tempo (BPM) and speed at the start of the song: the Fxx of row 00 of the first pattern
    /// played, or 125 BPM and speed 6 by default.
    pub fn start_tempo(&self) -> (u8, u8) {
        let song = &self.song;
        let (mut bpm, mut speed) = (125, 6);
        if let Some(row) = song
            .patterns
            .get(song.orders[0] as usize)
            .and_then(|p| p.rows.first())
        {
            for cell in row.iter().filter(|c| c.effect == 0xF) {
                match cell.param {
                    1..=0x1F => speed = cell.param,
                    0x20.. => bpm = cell.param,
                    0 => {}
                }
            }
        }
        (bpm, speed)
    }

    /// Sets the tempo and/or speed at the start of the song: updates the Fxx of row 00 of the
    /// first pattern played, or writes them in the effect column of a free voice.
    pub fn set_start_tempo(&self, bpm: Option<u8>, speed: Option<u8>) -> anyhow::Result<Change> {
        if let Some(b) = bpm {
            ensure!(b >= 0x20, t!("editor.tempo_range"));
        }
        if let Some(s) = speed {
            ensure!((1..=0x1F).contains(&s), t!("editor.speed_range"));
        }
        let index = self.song.orders[0] as usize;
        let mut pattern = self.song.patterns[index].clone();
        let row = &mut pattern.rows[0];
        for (value, is_kind) in [
            (bpm, (|p: u8| p >= 0x20) as fn(u8) -> bool),
            (speed, (|p: u8| (1..=0x1F).contains(&p)) as fn(u8) -> bool),
        ] {
            let Some(value) = value else { continue };
            let slot = row
                .iter()
                .position(|c| c.effect == 0xF && is_kind(c.param))
                .or_else(|| row.iter().position(|c| c.effect == 0 && c.param == 0));
            let Some(v) = slot else {
                anyhow::bail!(t!("editor.no_free_effect", pattern = format!("{index:02}")));
            };
            (row[v].effect, row[v].param) = (0xF, value);
        }
        Ok(Change::Pattern(index, Some(pattern)))
    }

    pub fn set_sample(&self, number: usize, sample: Sample) -> anyhow::Result<Change> {
        let count = self.song.samples.len();
        ensure!(
            (1..=count).contains(&number),
            t!("editor.no_such_sample", sample = number, count = count)
        );
        Ok(Change::Sample(number - 1, Box::new(sample)))
    }
}

/// Journal file, next to the song: `song.mod.journal.txt`.
pub fn journal_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".journal.txt");
    PathBuf::from(name)
}

/// The journal as text, one line per entry.
pub fn journal_text(editor: &Editor) -> String {
    editor
        .journal()
        .iter()
        .map(|e| {
            let secs = e
                .time
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let who = match e.origin {
                Origin::Agent => t!("journal.agent"),
                Origin::Keyboard => t!("journal.keyboard"),
            };
            format!("{} {who:7} {}\n", format_time(secs), e.text)
        })
        .collect()
}

/// UTC time `YYYY-MM-DD hh:mm:ss UTC`, without a date crate.
pub fn format_time(secs: u64) -> String {
    let (days, rest) = (secs / 86400, secs % 86400);
    // Days since 1970 → civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::text::parse_cell;

    fn editor() -> Editor {
        Editor::new(Song::new("test"))
    }

    #[test]
    fn undo_and_redo_restore_the_song() {
        let mut ed = editor();
        let original = ed.song().clone();

        let mut p = ed.song().patterns[0].clone();
        p.rows[0][0] = parse_cell("C-2 01 ...").unwrap();
        let changes = vec![
            ed.set_pattern(0, p).unwrap(),
            Change::Title(*b"new title\0\0\0\0\0\0\0\0\0\0\0"),
        ];
        ed.apply(Origin::Agent, "first row", changes);
        let edited = ed.song().clone();
        assert_ne!(edited, original);

        assert_eq!(ed.undo(Origin::Keyboard).as_deref(), Some("first row"));
        assert_eq!(ed.song(), &original);
        assert_eq!(ed.redo(Origin::Keyboard).as_deref(), Some("first row"));
        assert_eq!(ed.song(), &edited);
        assert_eq!(ed.journal().len(), 3);
    }

    #[test]
    fn adding_a_pattern_and_orders_can_be_undone() {
        let mut ed = editor();
        let changes = vec![ed.set_pattern(1, Pattern::new(64, 4)).unwrap()];
        ed.apply(Origin::Agent, "pattern 01", changes);
        let changes = vec![ed.set_orders(&[0, 1, 1]).unwrap()];
        ed.apply(Origin::Agent, "ordre", changes);
        assert_eq!(ed.song().order_list(), &[0, 1, 1]);

        ed.undo(Origin::Agent);
        ed.undo(Origin::Agent);
        assert_eq!(ed.song().patterns.len(), 1);
        assert_eq!(ed.song().order_list(), &[0]);
    }

    #[test]
    fn rejects_invalid_changes() {
        let ed = editor();
        assert!(ed.set_pattern(5, Pattern::new(64, 4)).is_err());
        assert!(ed.set_pattern(0, Pattern::new(32, 4)).is_err());
        assert!(ed.set_orders(&[0, 3]).is_err());
        assert!(ed.set_orders(&[]).is_err());
        assert!(ed.set_sample(32, Sample::default()).is_err());
    }

    #[test]
    fn start_tempo_updates_or_adds_f_effects() {
        let mut ed = editor();
        assert_eq!(ed.start_tempo(), (125, 6));
        let change = ed.set_start_tempo(Some(140), None).unwrap();
        ed.apply(Origin::Keyboard, "tempo", vec![change]);
        assert_eq!(ed.start_tempo(), (140, 6));
        let change = ed.set_start_tempo(Some(90), Some(3)).unwrap();
        ed.apply(Origin::Keyboard, "tempo", vec![change]);
        assert_eq!(ed.start_tempo(), (90, 3));
        // The BPM was updated in place: two effect columns in use, not three.
        let used = ed.song().patterns[0].rows[0]
            .iter()
            .filter(|c| c.effect == 0xF)
            .count();
        assert_eq!(used, 2);
        assert!(ed.set_start_tempo(Some(10), None).is_err());
    }

    #[test]
    fn a_new_edit_clears_redo() {
        let mut ed = editor();
        ed.apply(Origin::Agent, "a", vec![Change::Title([1; 20])]);
        ed.undo(Origin::Agent);
        ed.apply(Origin::Agent, "b", vec![Change::Title([2; 20])]);
        assert!(ed.redo(Origin::Agent).is_none());
    }

    #[test]
    fn formats_dates() {
        assert_eq!(format_time(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_time(1_790_000_000), "2026-09-21 14:13:20 UTC");
    }

    #[test]
    fn journal_sits_next_to_the_song() {
        assert_eq!(
            journal_path(Path::new("/tmp/a.mod")),
            PathBuf::from("/tmp/a.mod.journal.txt")
        );
    }
}
