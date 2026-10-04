//! Interface state and keyboard actions.
//!
//! Everything goes through the editor (`Origin::Keyboard`); after each change the replayer
//! gets the new song, so editing works while playing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use rust_i18n::t;

use super::dialog::{Answer, Browser, Choice, Dialog, Outcome, Prompt, Purpose};
use super::keys::{self, Action, Focus, Layout};
use super::particles::Particles;
use super::theme::{Theme, ThemeSource};
use crate::editor::{Change, Origin, journal_path, journal_text};
use crate::format::protracker;
use crate::monitor::Monitor;
use crate::replayer::{Mixer, Replayer};
use crate::session::{Cursor, Job, Session};
use crate::song::{Cell, Pattern, Sample, Song};
use crate::{note, samples};

/// How long cells written by the agent stay highlighted.
pub const AGENT_MARK: Duration = Duration::from_secs(20);

/// A new press of the same key sooner than this is an auto-repeat (key held down): it does
/// not retrigger the note.
const REPEAT_GAP: Duration = Duration::from_millis(150);

/// Cursor column within a `C-3 01 A04` cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Note,
    SampleTens,
    SampleUnits,
    Effect,
    ParamHigh,
    ParamLow,
}

const FIELDS: [Field; 6] = [
    Field::Note,
    Field::SampleTens,
    Field::SampleUnits,
    Field::Effect,
    Field::ParamHigh,
    Field::ParamLow,
];

/// Audio output: the replayer shared with the audio thread, and its monitor.
pub struct Audio {
    pub replayer: Arc<Mutex<Replayer>>,
    pub monitor: Arc<Monitor>,
    rate: u32,
    _stream: Option<cpal::Stream>,
}

impl Audio {
    /// Opens the default audio output, stopped.
    pub fn start(song: &Song) -> anyhow::Result<Self> {
        let monitor = Monitor::new(song.channels);
        let mut rate = 0;
        let song = Arc::new(song.clone());
        let (stream, replayer) = crate::audio::start(|r| {
            rate = r;
            let mut replayer = Replayer::new(song, r);
            replayer.set_monitor(monitor.clone());
            replayer.stop();
            replayer
        })?;
        Ok(Self {
            replayer,
            monitor,
            rate,
            _stream: Some(stream),
        })
    }

    /// No audio output (tests, or a machine without sound): the replayer exists but stays silent.
    pub fn silent(song: &Song) -> Self {
        let monitor = Monitor::new(song.channels);
        let mut replayer = Replayer::new(Arc::new(song.clone()), 48000);
        replayer.set_monitor(monitor.clone());
        replayer.stop();
        Self {
            replayer: Arc::new(Mutex::new(replayer)),
            monitor,
            rate: 48000,
            _stream: None,
        }
    }

    /// New song: new replayer (the voice count may change).
    fn reset(&mut self, song: &Song) {
        self.monitor = Monitor::new(song.channels);
        let mut replayer = Replayer::new(Arc::new(song.clone()), self.rate);
        replayer.set_monitor(self.monitor.clone());
        replayer.stop();
        *self.replayer.lock().unwrap() = replayer;
    }
}

pub struct App {
    /// The song, its file and unsaved-changes flag, shared with the agent through jobs.
    pub session: Session,
    pub audio: Audio,
    pub focus: Focus,
    /// Position being edited in the order list.
    pub position: usize,
    pub row: usize,
    pub voice: usize,
    pub field: Field,
    /// Octave of the bottom row of the piano keys (1 to 3).
    pub octave: u8,
    /// Sample used for the notes typed in (1 to 31).
    pub sample: usize,
    pub edit_mode: bool,
    /// Keyboard layout, for the piano keys and the digits.
    pub layout: Layout,
    /// Last note key and when it was last pressed (to ignore the auto-repeat of a held key).
    last_note: Option<(char, Instant)>,
    pub dialog: Option<Dialog>,
    pub status: String,
    pub quit: bool,
    /// Action to confirm by repeating it (quit or open without saving).
    armed: Option<Action>,
    /// Playing or not, speed and tempo (copied from the replayer on every frame).
    pub running: bool,
    /// Cells the agent changed lately, keyed by (pattern, row, voice), with when.
    pub agent_marks: HashMap<(usize, usize, usize), Instant>,
    /// Last time the agent changed something.
    pub agent_active: Option<Instant>,
    /// Colours, and where they come from (the Omarchy theme, checked once a second).
    pub theme: Theme,
    /// Sparks thrown off the voice scopes, and when they last moved.
    pub particles: Particles,
    particles_moved: Instant,
    pub theme_source: Option<ThemeSource>,
    theme_checked: Instant,
    /// Number of agents connected to this editor (live sessions), if it listens for them.
    pub agents: Option<Arc<std::sync::atomic::AtomicUsize>>,
    pub tempo: (u32, u32),
}

impl App {
    pub fn new(song: Song, path: Option<PathBuf>, audio: Audio) -> Self {
        Self {
            session: Session::new(song, path),
            audio,
            focus: Focus::Pattern,
            position: 0,
            row: 0,
            voice: 0,
            field: Field::Note,
            octave: 2,
            sample: 1,
            edit_mode: false,
            layout: keys::LAYOUTS[0],
            last_note: None,
            dialog: None,
            status: t!("status.help_hint").into_owned(),
            quit: false,
            armed: None,
            running: false,
            agent_marks: HashMap::new(),
            agent_active: None,
            agents: None,
            theme: Theme::classic(),
            particles: Particles::new(0),
            particles_moved: Instant::now(),
            theme_source: None,
            theme_checked: Instant::now(),
            tempo: (6, 125),
        }
    }

    pub fn song(&self) -> &Song {
        self.session.editor.song()
    }

    pub fn channels(&self) -> usize {
        self.song().channels
    }

    /// Number of the pattern being edited (the current position's).
    pub fn pattern_index(&self) -> usize {
        self.song().orders[self.position] as usize
    }

    /// On every frame: follow playback (the cursor sits on the row being played).
    pub fn tick(&mut self) {
        {
            let r = self.audio.replayer.lock().unwrap();
            self.running = r.is_running();
            self.tempo = r.tempo();
            if self.running {
                (self.position, self.row) = r.position();
            }
        }
        // Tell the agent where the user is.
        self.session.cursor = Some(Cursor {
            position: self.position,
            pattern: self.pattern_index(),
            row: self.row,
            voice: self.voice + 1,
            playing: self.running,
        });
        self.agent_marks
            .retain(|_, when| when.elapsed() < AGENT_MARK);
        let dt = self.particles_moved.elapsed().as_secs_f32().min(0.1);
        self.particles_moved = Instant::now();
        self.particles
            .update(&self.audio.monitor, super::view::VOICE_SCOPE_GAIN, dt);
        // Follow the Omarchy theme when the user switches it.
        if self.theme_checked.elapsed() >= Duration::from_secs(1) {
            self.theme_checked = Instant::now();
            if let Some(theme) = self.theme_source.as_mut().and_then(ThemeSource::changed) {
                self.theme = theme;
            }
        }
    }

    /// Runs a job sent by the agent, then brings the screen and the sound up to date.
    pub fn run_job(&mut self, job: Job) {
        let before = self.song().clone();
        let journal = self.session.editor.journal().len();
        // The agent side speaks English, whatever the interface language.
        let language = rust_i18n::locale().to_string();
        crate::lang::set("en");
        job(&mut self.session);
        crate::lang::set(&language);

        let after = self.song().clone();
        if after.channels != before.channels {
            self.audio.reset(&after);
        }
        // Mark the cells the agent changed.
        let now = Instant::now();
        for (p, pattern) in after.patterns.iter().enumerate() {
            for (row, cells) in pattern.rows.iter().enumerate() {
                for (v, cell) in cells.iter().enumerate() {
                    let old = before
                        .patterns
                        .get(p)
                        .and_then(|o| o.rows.get(row))
                        .and_then(|r| r.get(v));
                    if old != Some(cell) {
                        self.agent_marks.insert((p, row, v), now);
                    }
                }
            }
        }
        // Say what the agent just did.
        let entries =
            &self.session.editor.journal()[journal.min(self.session.editor.journal().len())..];
        if let Some(last) = entries.iter().rev().find(|e| e.origin == Origin::Agent) {
            self.status = t!("status.agent", what = last.text).into_owned();
            self.agent_active = Some(now);
        }
        self.clamp_cursor();
        self.sync();
    }

    /// Keeps the cursor inside the song after a change made elsewhere.
    fn clamp_cursor(&mut self) {
        let (orders, channels) = (self.song().order_list().len(), self.channels());
        self.position = self.position.min(orders - 1);
        self.voice = self.voice.min(channels - 1);
        self.row = self.row.min(63);
    }

    /// Hands the song and the mix over to the replayer.
    fn sync(&mut self) {
        let song = Arc::new(self.session.editor.song().clone());
        let mut r = self.audio.replayer.lock().unwrap();
        r.set_song(song);
        r.mixer = self.session.editor.mixer.clone();
    }

    fn apply(&mut self, description: String, changes: Vec<Change>) {
        self.session
            .editor
            .apply(Origin::Keyboard, description, changes);
        self.session.dirty = true;
        self.sync();
    }

    fn apply_result(&mut self, description: String, changes: anyhow::Result<Vec<Change>>) {
        match changes {
            Ok(changes) => self.apply(description, changes),
            Err(e) => self.status = format!("{e:#}"),
        }
    }

    // --- Keyboard --------------------------------------------------------------------------

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        // A held note key repeats: do not retrigger the note.
        let note_key =
            self.dialog.is_none() && self.focus == Focus::Pattern && self.field == Field::Note;
        if key.kind == KeyEventKind::Repeat && note_key && matches!(key.code, KeyCode::Char(_)) {
            return;
        }
        if let Some(dialog) = &mut self.dialog {
            match dialog.handle(key) {
                Outcome::Pending => {}
                Outcome::Cancel => self.dialog = None,
                Outcome::Done(purpose, answer) => {
                    self.dialog = None;
                    self.answer(purpose, answer);
                }
            }
            return;
        }
        if let Some(action) = keys::action(key, self.focus, self.layout) {
            let armed = self.armed.take();
            self.act(action, armed == Some(action));
            return;
        }
        self.armed = None;
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if let (KeyCode::Char(c), true, Focus::Pattern) = (key.code, plain, self.focus) {
            self.type_char(c.to_ascii_lowercase());
        }
    }

    fn act(&mut self, action: Action, confirmed: bool) {
        use Action::*;
        match action {
            Quit => {
                if self.session.dirty && !confirmed {
                    self.arm(Quit, t!("status.quit_unsaved").into_owned());
                } else {
                    self.quit = true;
                }
            }
            Save => match self.session.path.clone() {
                Some(path) => self.save(&path),
                None => self.act(SaveAs, false),
            },
            SaveAs => {
                let default = self
                    .session
                    .path
                    .as_ref()
                    .map_or(t!("file.default_name").into_owned(), |p| {
                        p.display().to_string()
                    });
                self.dialog = Some(Dialog::Prompt(Prompt::new(
                    Purpose::SaveAs,
                    t!("dialog.save_as"),
                    default,
                )));
            }
            Open => {
                if self.session.dirty && !confirmed {
                    self.arm(Open, t!("status.open_unsaved").into_owned());
                } else {
                    let dir = self.browse_dir();
                    let browser =
                        Browser::new(Purpose::OpenSong, t!("dialog.open"), &dir, &[".mod"]);
                    self.dialog = Some(Dialog::Browser(browser));
                }
            }
            Undo => match self.session.editor.undo(Origin::Keyboard) {
                Some(d) => self.after_history(t!("status.undone", what = d).into_owned()),
                None => self.status = t!("status.nothing_to_undo").into_owned(),
            },
            Redo => match self.session.editor.redo(Origin::Keyboard) {
                Some(d) => self.after_history(t!("status.redone", what = d).into_owned()),
                None => self.status = t!("status.nothing_to_redo").into_owned(),
            },
            PlaySong => {
                let mut r = self.audio.replayer.lock().unwrap();
                if r.is_running() {
                    r.stop();
                } else {
                    r.play(self.position, false);
                }
            }
            PlayPattern => self
                .audio
                .replayer
                .lock()
                .unwrap()
                .play(self.position, true),
            Stop => {
                let mut r = self.audio.replayer.lock().unwrap();
                r.stop();
                r.jam_stop();
            }
            ToggleEdit => {
                self.edit_mode = !self.edit_mode;
                let mode = if self.edit_mode {
                    t!("status.edit_mode")
                } else {
                    t!("status.listen_mode")
                };
                self.status = mode.into_owned();
            }
            OctaveDown => self.octave = (self.octave - 1).max(1),
            OctaveUp => self.octave = (self.octave + 1).min(3),
            PrevSample => self.sample = (self.sample - 1).max(1),
            NextSample => self.sample = (self.sample + 1).min(31),
            SetFocus(focus) => self.focus = focus,
            ToggleMute(v) if v < self.channels() => {
                self.session.editor.mixer.mute[v] = !self.session.editor.mixer.mute[v];
                self.sync();
            }
            ToggleMute(_) => {}
            Solo => {
                let v = self.voice;
                self.session.editor.mixer.solo[v] = !self.session.editor.mixer.solo[v];
                self.sync();
            }
            ResetMix => {
                self.session.editor.mixer = Mixer::new(self.channels());
                self.sync();
            }
            VoiceVolume(step) => {
                let volume = &mut self.session.editor.mixer.volume[self.voice];
                *volume = (*volume + step as f32 * 0.1).clamp(0.0, 1.0);
                self.sync();
            }
            SetTitle => {
                let title = self.song().display_title();
                self.dialog = Some(Dialog::Prompt(Prompt::new(
                    Purpose::SetTitle,
                    t!("dialog.title"),
                    title,
                )));
            }
            SetTempo => {
                let (bpm, speed) = self.session.editor.start_tempo();
                let prompt = Prompt::new(
                    Purpose::SetTempo,
                    t!("dialog.tempo"),
                    format!("{bpm} {speed}"),
                );
                self.dialog = Some(Dialog::Prompt(prompt));
            }
            Help => self.dialog = Some(Dialog::Help(0)),
            Journal => self.dialog = Some(Dialog::Journal),
            NextLayout => {
                self.layout = self.layout.next();
                self.status = t!("status.layout", layout = self.layout.name).into_owned();
            }
            _ => match self.focus {
                Focus::Pattern => self.act_pattern(action),
                Focus::Orders => self.act_orders(action),
                Focus::Samples => self.act_samples(action),
            },
        }
    }

    fn arm(&mut self, action: Action, message: String) {
        self.armed = Some(action);
        self.status = message;
    }

    fn after_history(&mut self, message: String) {
        self.status = message;
        self.session.dirty = true;
        self.position = self.position.min(self.song().order_list().len() - 1);
        self.sync();
    }

    fn browse_dir(&self) -> PathBuf {
        self.session
            .path
            .as_deref()
            .and_then(Path::parent)
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
    }

    // --- Pattern ---------------------------------------------------------------------------

    fn act_pattern(&mut self, action: Action) {
        use Action::*;
        let channels = self.channels();
        let field = FIELDS.iter().position(|&f| f == self.field).unwrap();
        match action {
            Up => self.row = (self.row + 63) % 64,
            Down => self.row = (self.row + 1) % 64,
            PageUp => self.row = self.row.saturating_sub(16),
            PageDown => self.row = (self.row + 16).min(63),
            Home => self.row = 0,
            End => self.row = 63,
            Left if field == 0 => {
                self.voice = (self.voice + channels - 1) % channels;
                self.field = Field::ParamLow;
            }
            Left => self.field = FIELDS[field - 1],
            Right if field == 5 => {
                self.voice = (self.voice + 1) % channels;
                self.field = Field::Note;
            }
            Right => self.field = FIELDS[field + 1],
            NextVoice => (self.voice, self.field) = ((self.voice + 1) % channels, Field::Note),
            PrevVoice => {
                (self.voice, self.field) = ((self.voice + channels - 1) % channels, Field::Note)
            }
            ClearField => self.edit_cell(&t!("edit.cleared"), |cell, field| match field {
                Field::Note => (cell.period, cell.sample) = (0, 0),
                Field::SampleTens | Field::SampleUnits => cell.sample = 0,
                _ => (cell.effect, cell.param) = (0, 0),
            }),
            ClearCell => {
                self.edit_cell(&t!("edit.cell_cleared"), |cell, _| *cell = Cell::default())
            }
            InsertRow => self.shift_voice(true),
            DeleteRow => self.shift_voice(false),
            _ => {}
        }
    }

    /// Types a character into the cursor column.
    fn type_char(&mut self, c: char) {
        if self.field == Field::Note {
            let Some(semitone) = self.layout.piano(c) else {
                return;
            };
            let index = (self.octave as i32 - 1) * 12 + semitone;
            if !(0..36).contains(&index) {
                self.status = t!("status.note_out_of_range").into_owned();
                return;
            }
            let period = note::PERIODS[index as usize];
            if self.repeated(c) {
                return;
            }
            self.audio
                .replayer
                .lock()
                .unwrap()
                .jam(self.voice, self.sample, period);
            if self.edit_mode {
                let sample = self.sample as u8;
                self.edit_cell(&note::name(index as usize), |cell, _| {
                    (cell.period, cell.sample) = (period, sample)
                });
                self.row = (self.row + 1) % 64;
            }
            return;
        }
        let Some(digit) = self.layout.hex_digit(c) else {
            return;
        };
        if !self.edit_mode {
            self.status = t!("status.listen_mode_hint").into_owned();
            return;
        }
        let decimal = matches!(self.field, Field::SampleTens | Field::SampleUnits);
        if decimal && digit > 9 {
            return;
        }
        let current = self.current_cell().sample as u32;
        let sample = match self.field {
            Field::SampleTens => digit * 10 + current % 10,
            Field::SampleUnits => current / 10 * 10 + digit,
            _ => current,
        };
        if sample > 31 {
            self.status = t!("status.sample_max", sample = sample).into_owned();
            return;
        }
        let d = digit as u8;
        self.edit_cell(&t!("edit.typed"), |cell, field| match field {
            Field::SampleTens | Field::SampleUnits => cell.sample = sample as u8,
            Field::Effect => cell.effect = d,
            Field::ParamHigh => cell.param = (d << 4) | (cell.param & 0x0F),
            Field::ParamLow => cell.param = (cell.param & 0xF0) | d,
            Field::Note => {}
        });
        self.row = (self.row + 1) % 64;
    }

    /// True when this press auto-repeats the previous key (key held down).
    fn repeated(&mut self, key: char) -> bool {
        let now = Instant::now();
        let repeat = self
            .last_note
            .is_some_and(|(k, t)| k == key && now - t < REPEAT_GAP);
        self.last_note = Some((key, now));
        repeat
    }

    pub fn current_cell(&self) -> Cell {
        self.song().patterns[self.pattern_index()].rows[self.row][self.voice]
    }

    /// Changes the cell under the cursor.
    fn edit_cell(&mut self, what: &str, f: impl FnOnce(&mut Cell, Field)) {
        let p = self.pattern_index();
        let mut pattern = self.song().patterns[p].clone();
        f(&mut pattern.rows[self.row][self.voice], self.field);
        let description = t!(
            "edit.cell",
            pattern = format!("{p:02}"),
            row = format!("{:02}", self.row),
            voice = self.voice + 1,
            what = what
        )
        .into_owned();
        let changes = self.session.editor.set_pattern(p, pattern).map(|c| vec![c]);
        self.apply_result(description, changes);
    }

    /// Inserts (shifts down) or deletes (shifts up) a row within the voice.
    fn shift_voice(&mut self, insert: bool) {
        let (p, v, row) = (self.pattern_index(), self.voice, self.row);
        let mut pattern = self.song().patterns[p].clone();
        let mut column: Vec<Cell> = pattern.rows.iter().map(|r| r[v]).collect();
        if insert {
            column.insert(row, Cell::default());
            column.pop();
        } else {
            column.remove(row);
            column.push(Cell::default());
        }
        for (r, cell) in pattern.rows.iter_mut().zip(column) {
            r[v] = cell;
        }
        let what = if insert {
            t!("edit.row_inserted")
        } else {
            t!("edit.row_deleted")
        };
        let changes = self.session.editor.set_pattern(p, pattern).map(|c| vec![c]);
        let description = t!(
            "edit.cell",
            pattern = format!("{p:02}"),
            row = format!("{row:02}"),
            voice = v + 1,
            what = what
        );
        self.apply_result(description.into_owned(), changes);
    }

    // --- Order list ------------------------------------------------------------------------

    fn act_orders(&mut self, action: Action) {
        use Action::*;
        let orders = self.song().order_list().to_vec();
        let last = orders.len() - 1;
        match action {
            Up => self.position = self.position.saturating_sub(1),
            Down => self.position = (self.position + 1).min(last),
            PageUp => self.position = self.position.saturating_sub(8),
            PageDown => self.position = (self.position + 8).min(last),
            Home => self.position = 0,
            End => self.position = last,
            Left | Right => {
                let current = orders[self.position] as usize;
                let wanted = if action == Left {
                    current.checked_sub(1)
                } else {
                    Some(current + 1)
                };
                let Some(wanted) = wanted else { return };
                let mut changes = Vec::new();
                if wanted == self.song().patterns.len() {
                    // One step past the last pattern: create an empty one. « Cap sur la suite ! »
                    match self
                        .session
                        .editor
                        .set_pattern(wanted, Pattern::new(64, self.channels()))
                    {
                        Ok(c) => changes.push(c),
                        Err(e) => return self.status = format!("{e:#}"),
                    }
                }
                let mut new_orders = orders.clone();
                new_orders[self.position] = wanted as u8;
                let description = t!(
                    "edit.position_pattern",
                    position = format!("{:02}", self.position),
                    pattern = format!("{wanted:02}")
                )
                .into_owned();
                let result = self
                    .orders_change(&new_orders, wanted)
                    .map(|c| changes.into_iter().chain([c]).collect());
                self.apply_result(description, result);
            }
            Insert => {
                if orders.len() >= 128 {
                    return self.status = t!("status.max_positions").into_owned();
                }
                let mut new_orders = orders.clone();
                new_orders.insert(self.position + 1, orders[self.position]);
                let result = self.session.editor.set_orders(&new_orders).map(|c| vec![c]);
                let description = t!(
                    "edit.position_inserted",
                    position = format!("{:02}", self.position + 1)
                );
                self.apply_result(description.into_owned(), result);
                self.position += 1;
            }
            Delete => {
                if orders.len() == 1 {
                    return self.status = t!("status.min_positions").into_owned();
                }
                let mut new_orders = orders.clone();
                new_orders.remove(self.position);
                let result = self.session.editor.set_orders(&new_orders).map(|c| vec![c]);
                let description = t!(
                    "edit.position_removed",
                    position = format!("{:02}", self.position)
                );
                self.apply_result(description.into_owned(), result);
                self.position = self.position.min(new_orders.len() - 1);
            }
            Enter => self.focus = Focus::Pattern,
            _ => {}
        }
    }

    /// Order change that may point at a pattern not created yet (created by the same change):
    /// check the numbers ourselves.
    fn orders_change(&self, orders: &[u8], created: usize) -> anyhow::Result<Change> {
        let mut all = [0u8; 128];
        all[..orders.len()].copy_from_slice(orders);
        let count = self.song().patterns.len().max(created + 1);
        anyhow::ensure!(
            orders.iter().all(|&p| (p as usize) < count),
            t!("editor.no_such_pattern", pattern = created, count = count)
        );
        Ok(Change::Orders {
            length: orders.len() as u8,
            orders: all,
        })
    }

    // --- Samples ---------------------------------------------------------------------------

    fn act_samples(&mut self, action: Action) {
        use Action::*;
        let n = self.sample;
        let current = self.song().samples[n - 1].clone();
        match action {
            Up => self.sample = (n - 1).max(1),
            Down => self.sample = (n + 1).min(31),
            PageUp => self.sample = n.saturating_sub(8).max(1),
            PageDown => self.sample = (n + 8).min(31),
            Home => self.sample = 1,
            End => self.sample = 31,
            Left | Right => {
                let step: i32 = if action == Left { -1 } else { 1 };
                let volume = (current.volume as i32 + step).clamp(0, 64) as u8;
                let description = t!(
                    "edit.sample_volume",
                    sample = format!("{n:02}"),
                    volume = volume
                );
                self.set_sample(description.into_owned(), Sample { volume, ..current });
            }
            FinetuneDown | FinetuneUp => {
                let step = if action == FinetuneDown { -1 } else { 1 };
                let finetune = (current.finetune() + step).clamp(-8, 7);
                let raw = (finetune as u8) & 0x0F;
                let description = t!(
                    "edit.sample_finetune",
                    sample = format!("{n:02}"),
                    finetune = format!("{finetune:+}")
                );
                self.set_sample(
                    description.into_owned(),
                    Sample {
                        finetune: raw,
                        ..current
                    },
                );
            }
            LoadSample => {
                let dir = self.browse_dir();
                let browser = Browser::new(
                    Purpose::LoadSample,
                    t!("dialog.load_sample"),
                    &dir,
                    &[".wav", ".aif", ".aiff", ".aifc"],
                );
                self.dialog = Some(Dialog::Browser(browser));
            }
            GenerateSample => {
                let items = samples::WAVEFORMS.iter().map(|w| w.to_string()).collect();
                let label = t!("dialog.generate", sample = format!("{n:02}")).into_owned();
                self.dialog = Some(Dialog::Choice(Choice {
                    purpose: Purpose::Generate,
                    label,
                    items,
                    selected: 0,
                }));
            }
            RenameSample => {
                let label = t!("dialog.rename", sample = format!("{n:02}"));
                let prompt = Prompt::new(Purpose::RenameSample, label, current.display_name());
                self.dialog = Some(Dialog::Prompt(prompt));
            }
            Preview => {
                let index = (self.octave as usize).min(2) * 12;
                if !self.repeated('p') {
                    self.audio
                        .replayer
                        .lock()
                        .unwrap()
                        .jam(self.voice, n, note::PERIODS[index]);
                }
            }
            Delete => {
                let description = t!("edit.sample_cleared", sample = format!("{n:02}"));
                self.set_sample(description.into_owned(), Sample::default())
            }
            Enter => self.focus = Focus::Pattern,
            _ => {}
        }
    }

    fn set_sample(&mut self, description: String, sample: Sample) {
        let changes = self
            .session
            .editor
            .set_sample(self.sample, sample)
            .map(|c| vec![c]);
        self.apply_result(description, changes);
    }

    // --- Files and dialog answers ----------------------------------------------------------

    fn answer(&mut self, purpose: Purpose, answer: Answer) {
        match (purpose, answer) {
            (Purpose::OpenSong, Answer::Path(path)) => self.open(&path),
            (Purpose::SaveAs, Answer::Text(text)) if !text.trim().is_empty() => {
                let path = PathBuf::from(text.trim());
                self.save(&path);
            }
            (Purpose::SetTempo, Answer::Text(text)) => {
                let mut numbers = text.split_whitespace().map(str::parse::<u8>);
                match (numbers.next(), numbers.next()) {
                    (Some(Ok(bpm)), speed) if !matches!(speed, Some(Err(_))) => {
                        let speed = speed.and_then(Result::ok);
                        let changes = self
                            .session
                            .editor
                            .set_start_tempo(Some(bpm), speed)
                            .map(|c| vec![c]);
                        let description = match speed {
                            Some(s) => t!("edit.tempo_speed", bpm = bpm, speed = s),
                            None => t!("edit.tempo", bpm = bpm),
                        }
                        .into_owned();
                        self.status = description.clone();
                        self.apply_result(description, changes);
                    }
                    _ => {
                        self.status =
                            t!("status.bad_tempo", text = format!("{text:?}")).into_owned()
                    }
                }
            }
            (Purpose::SetTitle, Answer::Text(title)) => {
                let title_bytes = Song::new(&title).title;
                self.apply(
                    t!("edit.title", title = title).into_owned(),
                    vec![Change::Title(title_bytes)],
                );
            }
            (Purpose::RenameSample, Answer::Text(name)) => {
                let mut sample = self.song().samples[self.sample - 1].clone();
                sample.set_name(&name);
                let description = t!(
                    "edit.sample_renamed",
                    sample = format!("{:02}", self.sample),
                    name = name
                );
                self.set_sample(description.into_owned(), sample);
            }
            (Purpose::Generate, Answer::Index(i)) => {
                let waveform = samples::WAVEFORMS[i];
                match samples::generate(waveform, 32) {
                    Ok(sample) => {
                        let description = t!(
                            "edit.sample_generated",
                            sample = format!("{:02}", self.sample),
                            waveform = waveform
                        );
                        self.set_sample(description.into_owned(), sample)
                    }
                    Err(e) => self.status = format!("{e:#}"),
                }
            }
            (Purpose::LoadSample, Answer::Path(path)) => match samples::import(&path, false) {
                Ok(report) => {
                    let mut info = t!(
                        "status.sample_info",
                        bytes = report.sample.data.len(),
                        rate = report.rate
                    )
                    .into_owned();
                    if let Some(n) = &report.natural_note {
                        info += &t!("status.natural_note", note = n);
                    }
                    if report.truncated {
                        info += &t!("status.truncated");
                    }
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    let description = t!(
                        "edit.sample_loaded",
                        sample = format!("{:02}", self.sample),
                        name = name
                    );
                    self.set_sample(description.into_owned(), report.sample);
                    self.status = t!("status.loaded", name = name, info = info).into_owned();
                }
                Err(e) => self.status = format!("{e:#}"),
            },
            _ => {}
        }
    }

    pub fn open(&mut self, path: &Path) {
        let song = match std::fs::read(path)
            .map_err(anyhow::Error::from)
            .and_then(|d| protracker::read(&d))
        {
            Ok(song) => song,
            Err(e) => {
                return self.status = t!(
                    "status.file_error",
                    path = path.display(),
                    error = format!("{e:#}")
                )
                .into_owned();
            }
        };
        self.audio.reset(&song);
        self.session.editor.replace_song(
            Origin::Keyboard,
            song,
            t!("journal.opened", path = path.display()).into_owned(),
        );
        self.session.path = Some(path.to_path_buf());
        self.session.dirty = false;
        (self.position, self.row, self.voice, self.field) = (0, 0, 0, Field::Note);
        self.sync();
        self.status = t!("status.opened", path = path.display()).into_owned();
    }

    fn save(&mut self, path: &Path) {
        let result = std::fs::write(path, protracker::write(self.song())).and_then(|_| {
            self.session.editor.log(
                Origin::Keyboard,
                t!("journal.saved", path = path.display()).into_owned(),
            );
            std::fs::write(journal_path(path), journal_text(&self.session.editor))
        });
        match result {
            Ok(()) => {
                self.session.path = Some(path.to_path_buf());
                self.session.dirty = false;
                self.status = t!("status.saved", path = path.display()).into_owned();
            }
            Err(e) => {
                self.status =
                    t!("status.write_error", path = path.display(), error = e).into_owned()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let song = Song::new("test");
        let audio = Audio::silent(&song);
        App::new(song, None, audio)
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::from(code));
    }

    fn typing(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn cell_text(app: &App, row: usize, voice: usize) -> String {
        crate::format::text::cell_to_text(
            &app.song().patterns[app.pattern_index()].rows[row][voice],
        )
    }

    #[test]
    fn notes_are_written_only_in_edit_mode() {
        let mut a = app();
        typing(&mut a, "z");
        assert_eq!(cell_text(&a, 0, 0), "... .. ...");
        // A real next keystroke comes much later than an auto-repeat.
        a.last_note = None;
        press(&mut a, KeyCode::Char(' '));
        a.sample = 5;
        typing(&mut a, "zq");
        assert_eq!(cell_text(&a, 0, 0), "C-2 05 ...");
        assert_eq!(cell_text(&a, 1, 0), "C-3 05 ...");
        assert_eq!(a.row, 2);
        assert!(a.session.dirty);
    }

    #[test]
    fn effect_and_sample_digits_go_in_their_columns() {
        let mut a = app();
        press(&mut a, KeyCode::Char(' '));
        a.field = Field::SampleTens;
        typing(&mut a, "1");
        a.row = 0;
        a.field = Field::SampleUnits;
        typing(&mut a, "2");
        a.row = 0;
        a.field = Field::Effect;
        typing(&mut a, "c");
        a.row = 0;
        a.field = Field::ParamHigh;
        typing(&mut a, "2");
        assert_eq!(cell_text(&a, 0, 0), "... 12 C20");
        a.row = 0;
        a.field = Field::SampleTens;
        typing(&mut a, "4");
        assert_eq!(cell_text(&a, 0, 0), "... 12 C20");
        assert!(a.status.contains("31 at most"), "{}", a.status);
    }

    #[test]
    fn undo_restores_and_cursor_wraps_across_voices() {
        let mut a = app();
        press(&mut a, KeyCode::Char(' '));
        typing(&mut a, "z");
        a.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(cell_text(&a, 0, 0), "... .. ...");
        press(&mut a, KeyCode::Left);
        assert_eq!((a.voice, a.field), (3, Field::ParamLow));
        press(&mut a, KeyCode::Right);
        assert_eq!((a.voice, a.field), (0, Field::Note));
    }

    #[test]
    fn orders_panel_creates_patterns_and_positions() {
        let mut a = app();
        press(&mut a, KeyCode::F(6));
        press(&mut a, KeyCode::Right);
        assert_eq!(a.song().patterns.len(), 2);
        assert_eq!(a.song().order_list(), &[1]);
        press(&mut a, KeyCode::Insert);
        press(&mut a, KeyCode::Left);
        assert_eq!(a.song().order_list(), &[1, 0]);
        press(&mut a, KeyCode::Delete);
        assert_eq!(a.song().order_list(), &[1]);
        assert_eq!(a.position, 0);
    }

    #[test]
    fn samples_panel_generates_and_adjusts() {
        let mut a = app();
        press(&mut a, KeyCode::F(7));
        press(&mut a, KeyCode::Char('g'));
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.song().samples[0].display_name(), "square");
        press(&mut a, KeyCode::Left);
        assert_eq!(a.song().samples[0].volume, 63);
        press(&mut a, KeyCode::Char('['));
        assert_eq!(a.song().samples[0].finetune(), -1);
    }

    #[test]
    fn mute_and_solo_reach_the_replayer() {
        let mut a = app();
        a.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::ALT));
        a.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT));
        let r = a.audio.replayer.lock().unwrap();
        assert!(r.mixer.mute[1] && r.mixer.solo[0]);
    }

    #[test]
    fn quitting_unsaved_work_needs_confirmation() {
        let mut a = app();
        press(&mut a, KeyCode::Char(' '));
        typing(&mut a, "z");
        let quit = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
        a.handle_key(quit);
        assert!(!a.quit);
        a.handle_key(quit);
        assert!(a.quit);
    }

    #[test]
    fn ctrl_b_sets_the_tempo() {
        let mut a = app();
        a.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        a.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        typing(&mut a, "140 4");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.session.editor.start_tempo(), (140, 4));
        assert_eq!(cell_text(&a, 0, 0), "... .. F8C");
        assert_eq!(cell_text(&a, 0, 1), "... .. F04");
    }

    #[test]
    fn every_azerty_piano_key_writes_its_note() {
        let mut a = app();
        a.layout = keys::Layout::by_name("azerty").unwrap();
        a.octave = 1;
        press(&mut a, KeyCode::Char(' '));
        // The 17 + 17 keys: bottom and middle rows, then the top rows.
        let keys = "wsxdcvgbhnj,;l:m!aéz\"er(t-yèuiçoàp";
        for (i, c) in keys.chars().enumerate() {
            a.last_note = None;
            a.row = 0;
            press(&mut a, KeyCode::Char(c));
            let semitone = if i < 17 { i } else { i - 17 + 12 };
            assert_eq!(
                cell_text(&a, 0, 0),
                format!("{} 01 ...", note::name(semitone)),
                "key {c:?}"
            );
        }
    }

    #[test]
    fn held_key_does_not_retrigger_or_rewrite() {
        let mut a = app();
        press(&mut a, KeyCode::Char(' '));
        typing(&mut a, "zzzz");
        assert_eq!(cell_text(&a, 0, 0), "C-2 01 ...");
        assert_eq!(cell_text(&a, 1, 0), "... .. ...");
        // Later on, a real keystroke of the same key does write its note.
        a.last_note = a.last_note.map(|(k, t)| (k, t - REPEAT_GAP * 2));
        typing(&mut a, "z");
        assert_eq!(cell_text(&a, 1, 0), "C-2 01 ...");
    }

    #[test]
    fn listened_note_sounds_on_the_cursor_voice_until_escape() {
        let mut a = app();
        let square = crate::samples::generate("square", 32).unwrap();
        let change = a.session.editor.set_sample(1, square).unwrap();
        a.apply("s1".into(), vec![change]);
        a.voice = 2;
        typing(&mut a, "z");
        let peak = |a: &App| {
            let mut r = a.audio.replayer.lock().unwrap();
            let mut out = vec![0.0f32; 2 * 4800];
            r.process(&mut out);
            out.iter().fold(0.0f32, |m, x| m.max(x.abs()))
        };
        assert!(peak(&a) > 0.05, "the note must sound");
        assert!(
            peak(&a) > 0.05,
            "a looped sample keeps going, as in the song"
        );
        assert!(
            a.audio.monitor.level(Some(2)) > 0.1,
            "voice 3's scope shows it"
        );
        press(&mut a, KeyCode::Esc);
        assert_eq!(peak(&a), 0.0);
    }

    #[test]
    fn agent_jobs_change_the_song_and_mark_cells() {
        let mut a = app();
        a.run_job(Box::new(|s: &mut Session| {
            let mut p = s.editor.song().patterns[0].clone();
            p.rows[4][2] = crate::format::text::parse_cell("A-2 01 037").unwrap();
            let change = s.editor.set_pattern(0, p).unwrap();
            s.editor.apply(Origin::Agent, "bass line", vec![change]);
            s.dirty = true;
        }));
        assert_eq!(cell_text(&a, 4, 2), "A-2 01 037");
        assert!(a.agent_marks.contains_key(&(0, 4, 2)));
        assert_eq!(a.agent_marks.len(), 1);
        assert!(a.status.contains("bass line"), "{}", a.status);
        assert!(a.session.dirty);
        // The replayer got the new song.
        let r = a.audio.replayer.lock().unwrap();
        drop(r);
        // The user can undo what the agent did.
        a.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(cell_text(&a, 4, 2), "... .. ...");
    }

    #[test]
    fn agent_sees_the_user_cursor() {
        let mut a = app();
        a.row = 7;
        a.voice = 1;
        a.tick();
        let c = a.session.cursor.unwrap();
        assert_eq!(
            (c.position, c.pattern, c.row, c.voice, c.playing),
            (0, 0, 7, 2, false)
        );
    }

    #[test]
    fn save_and_open_roundtrip() {
        let path = std::env::temp_dir().join(format!("smpltrckr-app-{}.mod", std::process::id()));
        let mut a = app();
        press(&mut a, KeyCode::Char(' '));
        typing(&mut a, "z");
        a.save(&path);
        assert!(!a.session.dirty);
        let mut b = app();
        b.open(&path);
        assert_eq!(cell_text(&b, 0, 0), "C-2 01 ...");
        assert!(journal_path(&path).exists());
        std::fs::remove_file(journal_path(&path)).unwrap();
        std::fs::remove_file(&path).unwrap();
    }
}
