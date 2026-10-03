//! État de l'interface et exécution des actions clavier.
//!
//! Tout passe par l'éditeur (`Origin::Keyboard`) ; après chaque modification, le replayer
//! reçoit le nouveau morceau, ce qui permet d'éditer pendant la lecture.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::dialog::{Answer, Browser, Choice, Dialog, Outcome, Prompt, Purpose};
use super::keys::{self, Action, Focus, Layout};
use crate::editor::{Change, Editor, Origin, journal_path, journal_text};
use crate::format::protracker;
use crate::monitor::Monitor;
use crate::replayer::{JAM_VOICES, Mixer, Replayer};
use crate::song::{Cell, Pattern, Sample, Song};
use crate::{note, samples};

/// Une note jouée au clavier pour l'écouter.
#[derive(Debug, Clone, Copy)]
struct JamNote {
    key: char,
    /// Premier appui (pas les répétitions automatiques).
    pressed: Instant,
    /// Dernier appui ou dernière répétition.
    last: Instant,
    /// Des répétitions automatiques sont arrivées : la touche est tenue.
    held: bool,
    /// Le sample boucle : il faut l'arrêter, sinon il sonne indéfiniment.
    looped: bool,
}

/// Colonne du curseur dans une cellule `C-3 01 A04`.
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

/// Sortie audio : le replayer partagé avec le thread audio et son moniteur.
pub struct Audio {
    pub replayer: Arc<Mutex<Replayer>>,
    pub monitor: Arc<Monitor>,
    rate: u32,
    _stream: Option<cpal::Stream>,
}

impl Audio {
    /// Ouvre la sortie audio par défaut, arrêtée.
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

    /// Sans sortie audio (tests, ou machine sans carte son) : le replayer existe mais ne sonne pas.
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

    /// Nouveau morceau : nouveau replayer (le nombre de voies peut changer).
    fn reset(&mut self, song: &Song) {
        self.monitor = Monitor::new(song.channels);
        let mut replayer = Replayer::new(Arc::new(song.clone()), self.rate);
        replayer.set_monitor(self.monitor.clone());
        replayer.stop();
        *self.replayer.lock().unwrap() = replayer;
    }
}

pub struct App {
    pub editor: Editor,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub audio: Audio,
    pub focus: Focus,
    /// Position éditée dans la liste d'ordre.
    pub position: usize,
    pub row: usize,
    pub voice: usize,
    pub field: Field,
    /// Octave de la rangée du bas du clavier piano (1 à 3).
    pub octave: u8,
    /// Sample utilisé pour les notes saisies (1 à 31).
    pub sample: usize,
    pub edit_mode: bool,
    /// Disposition du clavier, pour le clavier piano et les chiffres.
    pub layout: Layout,
    /// Vrai si le terminal annonce le relâchement des touches.
    pub key_release: bool,
    /// Vrai dès qu'un relâchement est réellement arrivé : certains terminaux (ou un
    /// multiplexeur entre les deux) annoncent le protocole sans transmettre les relâchements.
    /// Tant qu'on n'en a pas vu, on déduit le relâchement de la répétition automatique.
    releases_seen: bool,
    /// Réglages de répétition du clavier, pour reconnaître une touche tenue.
    pub key_repeat: keys::KeyRepeat,
    /// Notes en cours d'écoute, par voix d'écoute.
    jam: [Option<JamNote>; JAM_VOICES],
    pub dialog: Option<Dialog>,
    pub status: String,
    pub quit: bool,
    /// Action à confirmer en la répétant (quitter ou ouvrir sans enregistrer).
    armed: Option<Action>,
    /// Lecture en cours, vitesse et tempo (copiés du replayer à chaque image).
    pub running: bool,
    pub tempo: (u32, u32),
}

impl App {
    pub fn new(song: Song, path: Option<PathBuf>, audio: Audio) -> Self {
        Self {
            editor: Editor::new(song),
            path,
            dirty: false,
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
            key_release: false,
            releases_seen: false,
            key_repeat: keys::KeyRepeat::default(),
            jam: [None; JAM_VOICES],
            dialog: None,
            status: "? : aide".into(),
            quit: false,
            armed: None,
            running: false,
            tempo: (6, 125),
        }
    }

    pub fn song(&self) -> &Song {
        self.editor.song()
    }

    pub fn channels(&self) -> usize {
        self.song().channels
    }

    /// Numéro du pattern édité (celui de la position courante).
    pub fn pattern_index(&self) -> usize {
        self.song().orders[self.position] as usize
    }

    /// À chaque image : suit la lecture (le curseur se place sur la ligne jouée).
    pub fn tick(&mut self) {
        let mut r = self.audio.replayer.lock().unwrap();
        if !(self.key_release && self.releases_seen) {
            // Sans relâchement : une touche tenue se répète ; quand les répétitions cessent (ou
            // n'arrivent jamais, pour une simple frappe), la touche a été relâchée.
            let repeat = self.key_repeat;
            for (slot, jam) in self.jam.iter_mut().enumerate() {
                let Some(note) = jam else { continue };
                let released = if note.held {
                    note.last.elapsed() > (repeat.interval * 4).max(Duration::from_millis(100))
                } else {
                    note.pressed.elapsed() > repeat.delay + Duration::from_millis(120)
                };
                if released {
                    // Un sample sans boucle va jusqu'au bout, comme une percussion.
                    if note.looped {
                        r.jam_stop(slot);
                    }
                    *jam = None;
                }
            }
        }
        self.running = r.is_running();
        self.tempo = r.tempo();
        if self.running {
            (self.position, self.row) = r.position();
        }
    }

    /// Transmet le morceau et le mixage au replayer.
    fn sync(&mut self) {
        let song = Arc::new(self.editor.song().clone());
        let mut r = self.audio.replayer.lock().unwrap();
        r.set_song(song);
        r.mixer = self.editor.mixer.clone();
    }

    fn apply(&mut self, description: String, changes: Vec<Change>) {
        self.editor.apply(Origin::Keyboard, description, changes);
        self.dirty = true;
        self.sync();
    }

    fn apply_result(&mut self, description: String, changes: anyhow::Result<Vec<Change>>) {
        match changes {
            Ok(changes) => self.apply(description, changes),
            Err(e) => self.status = format!("{e:#}"),
        }
    }

    // --- Clavier ---------------------------------------------------------------------------

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            self.releases_seen = true;
            if let KeyCode::Char(c) = key.code {
                self.release_jam(c.to_ascii_lowercase());
            }
            return;
        }
        // Une touche de note tenue se répète : on ne relance pas la note.
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
                if self.dirty && !confirmed {
                    self.arm(Quit, "morceau non enregistré : Ctrl+Q encore pour quitter");
                } else {
                    self.quit = true;
                }
            }
            Save => match self.path.clone() {
                Some(path) => self.save(&path),
                None => self.act(SaveAs, false),
            },
            SaveAs => {
                let default = self
                    .path
                    .as_ref()
                    .map_or("morceau.mod".into(), |p| p.display().to_string());
                self.dialog = Some(Dialog::Prompt(Prompt::new(
                    Purpose::SaveAs,
                    "Enregistrer sous",
                    default,
                )));
            }
            Open => {
                if self.dirty && !confirmed {
                    self.arm(
                        Open,
                        "morceau non enregistré : Ctrl+O encore pour ouvrir quand même",
                    );
                } else {
                    let dir = self.browse_dir();
                    self.dialog = Some(Dialog::Browser(Browser::new(
                        Purpose::OpenSong,
                        "Ouvrir un module",
                        &dir,
                        &[".mod"],
                    )));
                }
            }
            Undo => match self.editor.undo(Origin::Keyboard) {
                Some(d) => self.after_history(format!("annulé : {d}")),
                None => self.status = "rien à annuler".into(),
            },
            Redo => match self.editor.redo(Origin::Keyboard) {
                Some(d) => self.after_history(format!("rétabli : {d}")),
                None => self.status = "rien à rétablir".into(),
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
                for slot in 0..JAM_VOICES {
                    r.jam_stop(slot);
                }
                self.jam = [None; JAM_VOICES];
            }
            ToggleEdit => {
                self.edit_mode = !self.edit_mode;
                self.status = if self.edit_mode {
                    "mode édition".into()
                } else {
                    "mode écoute".into()
                };
            }
            OctaveDown => self.octave = (self.octave - 1).max(1),
            OctaveUp => self.octave = (self.octave + 1).min(3),
            PrevSample => self.sample = (self.sample - 1).max(1),
            NextSample => self.sample = (self.sample + 1).min(31),
            SetFocus(focus) => self.focus = focus,
            ToggleMute(v) if v < self.channels() => {
                self.editor.mixer.mute[v] = !self.editor.mixer.mute[v];
                self.sync();
            }
            ToggleMute(_) => {}
            Solo => {
                let v = self.voice;
                self.editor.mixer.solo[v] = !self.editor.mixer.solo[v];
                self.sync();
            }
            ResetMix => {
                self.editor.mixer = Mixer::new(self.channels());
                self.sync();
            }
            VoiceVolume(step) => {
                let volume = &mut self.editor.mixer.volume[self.voice];
                *volume = (*volume + step as f32 * 0.1).clamp(0.0, 1.0);
                self.sync();
            }
            SetTitle => {
                let title = self.song().display_title();
                self.dialog = Some(Dialog::Prompt(Prompt::new(
                    Purpose::SetTitle,
                    "Titre du morceau",
                    title,
                )));
            }
            SetTempo => {
                let (bpm, speed) = self.editor.start_tempo();
                let prompt = Prompt::new(
                    Purpose::SetTempo,
                    "Tempo en BPM, puis vitesse (ex. 140 6)",
                    format!("{bpm} {speed}"),
                );
                self.dialog = Some(Dialog::Prompt(prompt));
            }
            Help => self.dialog = Some(Dialog::Help),
            NextLayout => {
                self.layout = self.layout.next();
                self.status = format!("clavier {}", self.layout.name);
            }
            _ => match self.focus {
                Focus::Pattern => self.act_pattern(action),
                Focus::Orders => self.act_orders(action),
                Focus::Samples => self.act_samples(action),
            },
        }
    }

    fn arm(&mut self, action: Action, message: &str) {
        self.armed = Some(action);
        self.status = message.into();
    }

    fn after_history(&mut self, message: String) {
        self.status = message;
        self.dirty = true;
        self.position = self.position.min(self.song().order_list().len() - 1);
        self.sync();
    }

    fn browse_dir(&self) -> PathBuf {
        self.path
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
            ClearField => self.edit_cell("effacé", |cell, field| match field {
                Field::Note => (cell.period, cell.sample) = (0, 0),
                Field::SampleTens | Field::SampleUnits => cell.sample = 0,
                _ => (cell.effect, cell.param) = (0, 0),
            }),
            ClearCell => self.edit_cell("cellule effacée", |cell, _| *cell = Cell::default()),
            InsertRow => self.shift_voice(true),
            DeleteRow => self.shift_voice(false),
            _ => {}
        }
    }

    /// Saisie d'un caractère dans la colonne du curseur.
    fn type_char(&mut self, c: char) {
        if self.field == Field::Note {
            let Some(semitone) = self.layout.piano(c) else {
                return;
            };
            let index = (self.octave as i32 - 1) * 12 + semitone;
            if !(0..36).contains(&index) {
                self.status = "note hors des octaves 1 à 3".into();
                return;
            }
            let period = note::PERIODS[index as usize];
            if self.repeated_jam(c) {
                return;
            }
            self.play_jam(c, self.sample, period);
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
            self.status = "mode écoute : Espace pour éditer".into();
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
            self.status = format!("sample {sample} : 31 au plus");
            return;
        }
        let d = digit as u8;
        self.edit_cell("saisie", |cell, field| match field {
            Field::SampleTens | Field::SampleUnits => cell.sample = sample as u8,
            Field::Effect => cell.effect = d,
            Field::ParamHigh => cell.param = (d << 4) | (cell.param & 0x0F),
            Field::ParamLow => cell.param = (cell.param & 0xF0) | d,
            Field::Note => {}
        });
        self.row = (self.row + 1) % 64;
    }

    /// Vrai si cet appui est une répétition automatique d'une touche tenue (à ignorer : la
    /// note continue). Avec le protocole de relâchement, les répétitions sont déjà marquées.
    fn repeated_jam(&mut self, key: char) -> bool {
        let repeat = self.key_repeat;
        let edit_mode = self.edit_mode;
        let Some(note) = self.jam.iter_mut().flatten().find(|n| n.key == key) else {
            return false;
        };
        let now = Instant::now();
        let since_last = now - note.last;
        let since_press = now - note.pressed;
        // Personne ne frappe deux fois la même touche en moins de deux intervalles de répétition.
        let fast = since_last < (repeat.interval * 2).max(Duration::from_millis(60));
        // La première répétition arrive après le délai de répétition. En mode écoute seulement :
        // en édition, une vraie frappe à ce rythme doit écrire sa note.
        let first_repeat = !edit_mode
            && !note.held
            && since_press + Duration::from_millis(40) >= repeat.delay
            && since_press <= repeat.delay + Duration::from_millis(80);
        if fast || first_repeat {
            note.last = now;
            note.held = true;
            return true;
        }
        false
    }

    /// Fait entendre une note : sur la voix d'écoute de la même touche si elle sonne déjà
    /// (sinon deux copies du même son s'additionneraient), ou sur une voix libre, ou sur la
    /// plus ancienne.
    fn play_jam(&mut self, key: char, sample: usize, period: u16) {
        let slot = self
            .jam
            .iter()
            .position(|j| j.is_some_and(|n| n.key == key))
            .or_else(|| self.jam.iter().position(Option::is_none))
            .unwrap_or_else(|| {
                (0..JAM_VOICES)
                    .min_by_key(|&i| self.jam[i].map(|n| n.pressed))
                    .unwrap()
            });
        self.audio
            .replayer
            .lock()
            .unwrap()
            .jam(slot, sample, period);
        let looped = self
            .song()
            .samples
            .get(sample.wrapping_sub(1))
            .is_some_and(|s| s.loop_length > 1);
        let now = Instant::now();
        self.jam[slot] = Some(JamNote {
            key,
            pressed: now,
            last: now,
            held: false,
            looped,
        });
    }

    fn release_jam(&mut self, key: char) {
        for (slot, jam) in self.jam.iter_mut().enumerate() {
            if jam.is_some_and(|n| n.key == key) {
                self.audio.replayer.lock().unwrap().jam_stop(slot);
                *jam = None;
            }
        }
    }

    pub fn current_cell(&self) -> Cell {
        self.song().patterns[self.pattern_index()].rows[self.row][self.voice]
    }

    /// Modifie la cellule sous le curseur.
    fn edit_cell(&mut self, what: &str, f: impl FnOnce(&mut Cell, Field)) {
        let p = self.pattern_index();
        let mut pattern = self.song().patterns[p].clone();
        f(&mut pattern.rows[self.row][self.voice], self.field);
        let description = format!(
            "pattern {p:02} ligne {:02} voie {} : {what}",
            self.row,
            self.voice + 1
        );
        let changes = self.editor.set_pattern(p, pattern).map(|c| vec![c]);
        self.apply_result(description, changes);
    }

    /// Insère (décale vers le bas) ou supprime (décale vers le haut) une ligne dans la voie.
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
            "ligne insérée"
        } else {
            "ligne supprimée"
        };
        let changes = self.editor.set_pattern(p, pattern).map(|c| vec![c]);
        self.apply_result(
            format!("pattern {p:02} voie {} ligne {row:02} : {what}", v + 1),
            changes,
        );
    }

    // --- Liste d'ordre ---------------------------------------------------------------------

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
                    // Un cran après le dernier pattern : on en crée un vide.
                    match self
                        .editor
                        .set_pattern(wanted, Pattern::new(64, self.channels()))
                    {
                        Ok(c) => changes.push(c),
                        Err(e) => return self.status = format!("{e:#}"),
                    }
                }
                let mut new_orders = orders.clone();
                new_orders[self.position] = wanted as u8;
                let description = format!("position {:02} : pattern {wanted:02}", self.position);
                let result = self
                    .orders_change(&new_orders, wanted)
                    .map(|c| changes.into_iter().chain([c]).collect());
                self.apply_result(description, result);
            }
            Insert => {
                if orders.len() >= 128 {
                    return self.status = "128 positions au plus".into();
                }
                let mut new_orders = orders.clone();
                new_orders.insert(self.position + 1, orders[self.position]);
                let result = self.editor.set_orders(&new_orders).map(|c| vec![c]);
                self.apply_result(format!("position {:02} insérée", self.position + 1), result);
                self.position += 1;
            }
            Delete => {
                if orders.len() == 1 {
                    return self.status = "la liste d'ordre garde au moins une position".into();
                }
                let mut new_orders = orders.clone();
                new_orders.remove(self.position);
                let result = self.editor.set_orders(&new_orders).map(|c| vec![c]);
                self.apply_result(format!("position {:02} retirée", self.position), result);
                self.position = self.position.min(new_orders.len() - 1);
            }
            Enter => self.focus = Focus::Pattern,
            _ => {}
        }
    }

    /// Modification de l'ordre qui peut viser un pattern pas encore créé (créé dans la même
    /// modification) : on vérifie les numéros nous-mêmes.
    fn orders_change(&self, orders: &[u8], created: usize) -> anyhow::Result<Change> {
        let mut all = [0u8; 128];
        all[..orders.len()].copy_from_slice(orders);
        let count = self.song().patterns.len().max(created + 1);
        anyhow::ensure!(
            orders.iter().all(|&p| (p as usize) < count),
            "pattern inexistant"
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
                self.set_sample(
                    format!("sample {n:02} : volume {volume}"),
                    Sample { volume, ..current },
                );
            }
            FinetuneDown | FinetuneUp => {
                let step = if action == FinetuneDown { -1 } else { 1 };
                let finetune = (current.finetune() + step).clamp(-8, 7);
                let raw = (finetune as u8) & 0x0F;
                self.set_sample(
                    format!("sample {n:02} : finetune {finetune:+}"),
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
                    "Charger un sample",
                    &dir,
                    &[".wav", ".aif", ".aiff", ".aifc"],
                );
                self.dialog = Some(Dialog::Browser(browser));
            }
            GenerateSample => {
                let items = samples::WAVEFORMS.iter().map(|w| w.to_string()).collect();
                let label = format!("Générer le sample {n:02}");
                self.dialog = Some(Dialog::Choice(Choice {
                    purpose: Purpose::Generate,
                    label,
                    items,
                    selected: 0,
                }));
            }
            RenameSample => {
                let prompt = Prompt::new(
                    Purpose::RenameSample,
                    format!("Nom du sample {n:02}"),
                    current.display_name(),
                );
                self.dialog = Some(Dialog::Prompt(prompt));
            }
            Preview => {
                let index = (self.octave as usize).min(2) * 12;
                if !self.repeated_jam('p') {
                    self.play_jam('p', n, note::PERIODS[index]);
                }
            }
            Delete => self.set_sample(format!("sample {n:02} vidé"), Sample::default()),
            Enter => self.focus = Focus::Pattern,
            _ => {}
        }
    }

    fn set_sample(&mut self, description: String, sample: Sample) {
        let changes = self.editor.set_sample(self.sample, sample).map(|c| vec![c]);
        self.apply_result(description, changes);
    }

    // --- Fichiers et réponses des dialogues ------------------------------------------------

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
                            .editor
                            .set_start_tempo(Some(bpm), speed)
                            .map(|c| vec![c]);
                        let description = match speed {
                            Some(s) => format!("tempo {bpm} BPM, vitesse {s}"),
                            None => format!("tempo {bpm} BPM"),
                        };
                        self.status = description.clone();
                        self.apply_result(description, changes);
                    }
                    _ => {
                        self.status = format!("tempo illisible {text:?} : ex. « 140 » ou « 140 6 »")
                    }
                }
            }
            (Purpose::SetTitle, Answer::Text(title)) => {
                let title_bytes = Song::new(&title).title;
                self.apply(
                    format!("titre « {title} »"),
                    vec![Change::Title(title_bytes)],
                );
            }
            (Purpose::RenameSample, Answer::Text(name)) => {
                let mut sample = self.song().samples[self.sample - 1].clone();
                sample.set_name(&name);
                self.set_sample(
                    format!("sample {:02} renommé « {name} »", self.sample),
                    sample,
                );
            }
            (Purpose::Generate, Answer::Index(i)) => {
                let waveform = samples::WAVEFORMS[i];
                match samples::generate(waveform, 32) {
                    Ok(sample) => self.set_sample(
                        format!("sample {:02} : {waveform} généré", self.sample),
                        sample,
                    ),
                    Err(e) => self.status = format!("{e:#}"),
                }
            }
            (Purpose::LoadSample, Answer::Path(path)) => match samples::import(&path, false) {
                Ok(report) => {
                    let mut info =
                        format!("{} octets, {} Hz", report.sample.data.len(), report.rate);
                    if let Some(n) = &report.natural_note {
                        info += &format!(", hauteur d'origine en {n}");
                    }
                    if report.truncated {
                        info += ", TRONQUÉ à 128 Ko";
                    }
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    self.set_sample(
                        format!("sample {:02} : {name} chargé", self.sample),
                        report.sample,
                    );
                    self.status = format!("{name} : {info}");
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
            Err(e) => return self.status = format!("{} : {e:#}", path.display()),
        };
        self.audio.reset(&song);
        self.editor.replace_song(
            Origin::Keyboard,
            song,
            format!("ouverture de {}", path.display()),
        );
        self.path = Some(path.to_path_buf());
        self.dirty = false;
        (self.position, self.row, self.voice, self.field) = (0, 0, 0, Field::Note);
        self.sync();
        self.status = format!("{} ouvert", path.display());
    }

    fn save(&mut self, path: &Path) {
        let result = std::fs::write(path, protracker::write(self.song())).and_then(|_| {
            self.editor.log(
                Origin::Keyboard,
                format!("enregistrement dans {}", path.display()),
            );
            std::fs::write(journal_path(path), journal_text(&self.editor))
        });
        match result {
            Ok(()) => {
                self.path = Some(path.to_path_buf());
                self.dirty = false;
                self.status = format!("enregistré : {}", path.display());
            }
            Err(e) => self.status = format!("écriture de {} : {e}", path.display()),
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
        // Une vraie frappe suivante arrive bien plus tard qu'une répétition automatique.
        a.jam = [None; JAM_VOICES];
        press(&mut a, KeyCode::Char(' '));
        a.sample = 5;
        typing(&mut a, "zq");
        assert_eq!(cell_text(&a, 0, 0), "C-2 05 ...");
        assert_eq!(cell_text(&a, 1, 0), "C-3 05 ...");
        assert_eq!(a.row, 2);
        assert!(a.dirty);
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
        assert!(a.status.contains("31 au plus"));
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
        assert_eq!(a.editor.start_tempo(), (140, 4));
        assert_eq!(cell_text(&a, 0, 0), "... .. F8C");
        assert_eq!(cell_text(&a, 0, 1), "... .. F04");
    }

    #[test]
    fn same_key_reuses_its_voice_and_escape_silences_everything() {
        let mut a = app();
        press(&mut a, KeyCode::F(7));
        typing(&mut a, "p");
        a.jam = a.jam.map(|j| {
            j.map(|n| JamNote {
                last: n.last - Duration::from_secs(1),
                pressed: n.pressed - Duration::from_secs(1),
                ..n
            })
        });
        typing(&mut a, "p");
        assert_eq!(a.jam.iter().flatten().count(), 1);
        press(&mut a, KeyCode::F(5));
        typing(&mut a, "zc");
        assert_eq!(a.jam.iter().flatten().count(), 3);
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.jam.iter().flatten().count(), 0);
    }

    /// Simule un clavier qui se répète au bout de 250 ms, 40 fois par seconde.
    fn with_repeat(mut a: App) -> App {
        a.key_repeat = keys::KeyRepeat {
            delay: Duration::from_millis(250),
            interval: Duration::from_millis(25),
        };
        a
    }

    fn age(a: &mut App, slot: usize, ms: u64) {
        let d = Duration::from_millis(ms);
        a.jam[slot] = a.jam[slot].map(|n| JamNote {
            pressed: n.pressed - d,
            last: n.last - d,
            ..n
        });
    }

    #[test]
    fn held_key_repeats_do_not_retrigger_the_note() {
        let mut a = with_repeat(app());
        typing(&mut a, "z");
        let first = a.jam[0].unwrap().pressed;
        // Première répétition (250 ms) puis répétitions rapides : la note n'est pas relancée.
        age(&mut a, 0, 250);
        typing(&mut a, "z");
        typing(&mut a, "zzz");
        let note = a.jam[0].unwrap();
        assert!(note.held);
        assert!(note.pressed < first, "la note a été relancée");
        assert_eq!(a.jam.iter().flatten().count(), 1);
    }

    #[test]
    fn inferred_release_stops_looped_notes_only() {
        let mut a = with_repeat(app());
        let mut looped = crate::samples::generate("square", 32).unwrap();
        looped.volume = 64;
        a.editor.apply(
            Origin::Keyboard,
            "s1",
            vec![a.editor.set_sample(1, looped).unwrap()],
        );
        a.editor.apply(
            Origin::Keyboard,
            "s2",
            vec![
                a.editor
                    .set_sample(2, crate::samples::generate("kick", 32).unwrap())
                    .unwrap(),
            ],
        );
        a.sync();
        typing(&mut a, "z");
        a.sample = 2;
        typing(&mut a, "x");
        // Simple frappe : pas de répétition après le délai → relâchée.
        age(&mut a, 0, 400);
        age(&mut a, 1, 400);
        a.tick();
        assert!(a.jam.iter().all(Option::is_none));
    }

    #[test]
    fn in_edit_mode_a_held_key_writes_one_note() {
        let mut a = with_repeat(app());
        press(&mut a, KeyCode::Char(' '));
        typing(&mut a, "zzzz");
        assert_eq!(cell_text(&a, 0, 0), "C-2 01 ...");
        assert_eq!(cell_text(&a, 1, 0), "... .. ...");
    }

    #[test]
    fn listened_notes_use_jam_voices_and_stop_on_release() {
        let mut a = app();
        a.key_release = true;
        a.releases_seen = true;
        typing(&mut a, "zc");
        assert_eq!(a.jam.iter().flatten().count(), 2);
        assert_eq!(cell_text(&a, 0, 0), "... .. ...");
        a.handle_key(KeyEvent::new_with_kind(
            KeyCode::Char('z'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        ));
        assert_eq!(
            a.jam.iter().flatten().map(|n| n.key).collect::<Vec<_>>(),
            ['c']
        );
        // Une touche tenue qui se répète ne relance rien.
        a.handle_key(KeyEvent::new_with_kind(
            KeyCode::Char('c'),
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        ));
        assert_eq!(a.jam.iter().flatten().count(), 1);
    }

    #[test]
    fn save_and_open_roundtrip() {
        let path = std::env::temp_dir().join(format!("smpltrckr-app-{}.mod", std::process::id()));
        let mut a = app();
        press(&mut a, KeyCode::Char(' '));
        typing(&mut a, "z");
        a.save(&path);
        assert!(!a.dirty);
        let mut b = app();
        b.open(&path);
        assert_eq!(cell_text(&b, 0, 0), "C-2 01 ...");
        assert!(journal_path(&path).exists());
        std::fs::remove_file(journal_path(&path)).unwrap();
        std::fs::remove_file(&path).unwrap();
    }
}
