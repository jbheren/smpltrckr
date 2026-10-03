//! Keyboard shortcuts: the one table that maps keys to actions.
//!
//! The 'piano' keys follow the physical position of the keys, as in ProTracker and FT2: the
//! same spot on a QWERTY, AZERTY or QWERTZ keyboard. A terminal sends characters, not
//! positions, so each layout tells which character each key produces.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Keyboard layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub name: &'static str,
    /// Bottom then middle row, at the spots of 'z s x d c v g b h n j m , l . ; /' on a QWERTY:
    /// C, C#, D… of the chosen octave.
    piano_low: &'static str,
    /// Letter and digit rows, at the spots of 'q 2 w 3 e r 5 t 6 y 7 u i 9 o 0 p': the octave
    /// above.
    piano_high: &'static str,
    /// Characters of the digit row without Shift, from 1 to 9 then 0.
    digit_row: [char; 10],
}

pub const LAYOUTS: [Layout; 3] = [
    Layout {
        name: "QWERTY",
        piano_low: "zsxdcvgbhnjm,l.;/",
        piano_high: "q2w3er5t6y7ui9o0p",
        digit_row: ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0'],
    },
    Layout {
        name: "AZERTY",
        piano_low: "wsxdcvgbhnj,;l:m!",
        piano_high: "aéz\"er(t-yèuiçoàp",
        digit_row: ['&', 'é', '"', '\'', '(', '-', 'è', '_', 'ç', 'à'],
    },
    Layout {
        name: "QWERTZ",
        piano_low: "ysxdcvgbhnjm,l.ö-",
        piano_high: "q2w3er5t6z7ui9o0p",
        digit_row: ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0'],
    },
];

impl Layout {
    pub fn by_name(name: &str) -> Option<Layout> {
        let name = name.to_ascii_lowercase();
        let wanted = match name.as_str() {
            "fr" | "be" | "azerty" => "AZERTY",
            "de" | "ch" | "at" | "qwertz" => "QWERTZ",
            "us" | "gb" | "uk" | "qwerty" => "QWERTY",
            _ => return None,
        };
        LAYOUTS.iter().copied().find(|l| l.name == wanted)
    }

    /// Next layout (F3).
    pub fn next(self) -> Layout {
        let i = LAYOUTS.iter().position(|l| *l == self).unwrap_or(0);
        LAYOUTS[(i + 1) % LAYOUTS.len()]
    }

    /// Semitones above the C of the chosen octave.
    pub fn piano(&self, c: char) -> Option<i32> {
        let find = |row: &str| row.chars().position(|x| x == c).map(|i| i as i32);
        find(self.piano_low).or_else(|| find(self.piano_high).map(|i| i + 12))
    }

    /// Digit typed, with or without Shift (on AZERTY, 'é' stands for 2).
    pub fn digit(&self, c: char) -> Option<u32> {
        c.to_digit(10).or_else(|| {
            self.digit_row
                .iter()
                .position(|&d| d == c)
                .map(|i| (i as u32 + 1) % 10)
        })
    }

    /// Hex digit typed (0-9, A-F).
    pub fn hex_digit(&self, c: char) -> Option<u32> {
        self.digit(c).or_else(|| c.to_digit(16))
    }
}

/// Layout at startup: `SMPLTRCKR_KEYBOARD` (or `SMPLTRCKR_CLAVIER`), then Hyprland's main
/// keyboard, then localectl, then QWERTY.
pub fn detect_layout() -> Layout {
    let from_env = ["SMPLTRCKR_KEYBOARD", "SMPLTRCKR_CLAVIER"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .find_map(|n| Layout::by_name(&n));
    from_env
        .or_else(hyprland_layout)
        .or_else(localectl_layout)
        .unwrap_or(LAYOUTS[0])
}

fn hyprland_layout() -> Option<Layout> {
    let out = std::process::Command::new("hyprctl")
        .args(["-j", "devices"])
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    // The main keyboard: '"layout": "fr"' in the block holding '"main": true'.
    let block = text.split('{').find(|b| b.contains("\"main\": true"))?;
    let layout = block
        .split("\"layout\": \"")
        .nth(1)?
        .split(['"', ','])
        .next()?;
    Layout::by_name(layout)
}

fn localectl_layout() -> Option<Layout> {
    let out = std::process::Command::new("localectl")
        .arg("status")
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let line = text.lines().find(|l| l.contains("X11 Layout:"))?;
    Layout::by_name(line.split(':').nth(1)?.trim().split(',').next()?)
}

/// Zone that gets the navigation keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Pattern,
    Orders,
    Samples,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    // General
    Quit,
    Save,
    SaveAs,
    Open,
    Undo,
    Redo,
    PlaySong,
    PlayPattern,
    Stop,
    ToggleEdit,
    OctaveDown,
    OctaveUp,
    PrevSample,
    NextSample,
    SetFocus(Focus),
    ToggleMute(usize),
    Solo,
    ResetMix,
    VoiceVolume(i8),
    SetTitle,
    SetTempo,
    Help,
    NextLayout,
    // Moves (depending on the active zone)
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    NextVoice,
    PrevVoice,
    // Pattern
    ClearField,
    ClearCell,
    InsertRow,
    DeleteRow,
    // Order list and samples
    Insert,
    Delete,
    Enter,
    LoadSample,
    GenerateSample,
    RenameSample,
    FinetuneDown,
    FinetuneUp,
    Preview,
}

/// Action of a key, apart from typing (notes, digits), which depends on the cursor column.
pub fn action(key: KeyEvent, focus: Focus, layout: Layout) -> Option<Action> {
    use Action::*;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    Some(match key.code {
        KeyCode::Char('q') if ctrl => Quit,
        KeyCode::Char('s') if ctrl => Save,
        KeyCode::Char('w') if ctrl => SaveAs,
        KeyCode::Char('o') if ctrl => Open,
        KeyCode::Char('z') if ctrl => Undo,
        KeyCode::Char('y') if ctrl => Redo,
        KeyCode::Char('p') if ctrl => PlayPattern,
        KeyCode::Char('t') if ctrl => SetTitle,
        KeyCode::Char('b') if ctrl => SetTempo,
        KeyCode::Char('k') if ctrl => DeleteRow,
        KeyCode::Char(c) if alt && matches!(layout.digit(c), Some(1..=8)) => {
            ToggleMute(layout.digit(c).unwrap() as usize - 1)
        }
        KeyCode::Char('s' | 'S') if alt => Solo,
        KeyCode::Char('0') if alt => ResetMix,
        KeyCode::Up if alt => VoiceVolume(1),
        KeyCode::Down if alt => VoiceVolume(-1),
        KeyCode::Enter => {
            if focus == Focus::Pattern {
                PlaySong
            } else {
                Enter
            }
        }
        KeyCode::Esc => Stop,
        KeyCode::Char(' ') => ToggleEdit,
        KeyCode::F(1) => OctaveDown,
        KeyCode::F(2) => OctaveUp,
        KeyCode::F(3) => NextLayout,
        KeyCode::F(5) => SetFocus(Focus::Pattern),
        KeyCode::F(6) => SetFocus(Focus::Orders),
        KeyCode::F(7) => SetFocus(Focus::Samples),
        KeyCode::Char('[') => {
            if focus == Focus::Samples {
                FinetuneDown
            } else {
                PrevSample
            }
        }
        KeyCode::Char(']') => {
            if focus == Focus::Samples {
                FinetuneUp
            } else {
                NextSample
            }
        }
        KeyCode::Char('?') => Help,
        KeyCode::Up => Up,
        KeyCode::Down => Down,
        KeyCode::Left => Left,
        KeyCode::Right => Right,
        KeyCode::PageUp => PageUp,
        KeyCode::PageDown => PageDown,
        KeyCode::Home => Home,
        KeyCode::End => End,
        KeyCode::Tab => NextVoice,
        KeyCode::BackTab => PrevVoice,
        KeyCode::Insert => {
            if focus == Focus::Pattern {
                InsertRow
            } else {
                Insert
            }
        }
        KeyCode::Delete => {
            if focus == Focus::Pattern {
                ClearField
            } else {
                Delete
            }
        }
        KeyCode::Backspace if focus == Focus::Pattern => ClearCell,
        KeyCode::Char('l') if focus == Focus::Samples => LoadSample,
        KeyCode::Char('g') if focus == Focus::Samples => GenerateSample,
        KeyCode::Char('n') if focus == Focus::Samples => RenameSample,
        KeyCode::Char('p') if focus == Focus::Samples => Preview,
        _ => return None,
    })
}

/// Key reminder for the active zone, shown at the bottom of the screen: translation ids
/// (pattern in edit mode, pattern, order list, samples).
pub const FOCUS_HINTS: [&str; 4] = ["hint.edit", "hint.pattern", "hint.orders", "hint.samples"];

/// Help page shown by "?": one translation id per line, `help.<id>.keys` and `help.<id>.what`.
pub const HELP: &[&str] = &[
    "play", "loop", "stop", "edit", "notes", "octave", "layout", "sample", "move", "digits",
    "clear", "rows", "mute", "solo", "volume", "zones", "orders", "samples", "files", "undo",
    "title", "tempo", "quit",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piano_keys_sit_at_the_same_places_on_every_layout() {
        let [qwerty, azerty, qwertz] = LAYOUTS;
        // Bottom-left key: C; the QWERTY 'A' key (AZERTY 'Q'): nothing.
        assert_eq!(qwerty.piano('z'), Some(0));
        assert_eq!(azerty.piano('w'), Some(0));
        assert_eq!(qwertz.piano('y'), Some(0));
        assert_eq!(azerty.piano('q'), None);
        // Top row: C of the next octave, then C# on the digit row.
        assert_eq!(azerty.piano('a'), Some(12));
        assert_eq!(azerty.piano('é'), Some(13));
        assert_eq!(azerty.piano(','), Some(11));
        assert_eq!(azerty.piano('p'), Some(28));
        for layout in LAYOUTS {
            assert_eq!(layout.piano_low.chars().count(), 17, "{}", layout.name);
            assert_eq!(layout.piano_high.chars().count(), 17, "{}", layout.name);
        }
    }

    #[test]
    fn azerty_digit_row_counts_as_digits() {
        let azerty = Layout::by_name("fr").unwrap();
        assert_eq!(azerty.digit('&'), Some(1));
        assert_eq!(azerty.digit('à'), Some(0));
        assert_eq!(azerty.hex_digit('c'), Some(12));
        let alt_e = KeyEvent::new(KeyCode::Char('é'), KeyModifiers::ALT);
        assert_eq!(
            action(alt_e, Focus::Pattern, azerty),
            Some(Action::ToggleMute(1))
        );
    }

    #[test]
    fn same_key_depends_on_focus() {
        let del = KeyEvent::from(KeyCode::Delete);
        assert_eq!(
            action(del, Focus::Pattern, LAYOUTS[0]),
            Some(Action::ClearField)
        );
        assert_eq!(action(del, Focus::Orders, LAYOUTS[0]), Some(Action::Delete));
        let g = KeyEvent::from(KeyCode::Char('g'));
        assert_eq!(
            action(g, Focus::Samples, LAYOUTS[0]),
            Some(Action::GenerateSample)
        );
        assert_eq!(action(g, Focus::Pattern, LAYOUTS[0]), None);
    }
}
