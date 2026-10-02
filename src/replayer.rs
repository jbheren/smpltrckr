//! Replayer ProTracker : séquenceur, effets, rééchantillonnage et mixage.
//!
//! Objectif : une lecture propre et fidèle au timing de ProTracker, sans émuler le matériel
//! Amiga (pas de filtre Paula). Le même moteur sert à la lecture temps réel et au rendu WAV.
//! Dans le chemin audio (`process`), rien n'est alloué.

use std::sync::Arc;

use crate::monitor::Monitor;
use crate::note::PAULA_CLOCK_PAL;
use crate::song::{Cell, ModKind, Song};

/// Table de vibrato et de trémolo de ProTracker (demi-sinus, 32 valeurs).
const SINE: [u8; 32] = [
    0, 24, 49, 74, 97, 120, 141, 161, 180, 197, 212, 224, 235, 244, 250, 253, 255, 253, 250, 244,
    235, 224, 212, 197, 180, 161, 141, 120, 97, 74, 49, 24,
];

/// Limites de période de ProTracker (B-3 à C-1) pour les portamentos.
const PERIOD_MIN: i32 = 113;
const PERIOD_MAX: i32 = 856;

/// Réglages de mixage par voie, valables pour la session (non enregistrés dans le `.mod`).
#[derive(Debug, Clone)]
pub struct Mixer {
    pub mute: Vec<bool>,
    pub solo: Vec<bool>,
    /// Volume de chaque voie, de 0.0 à 1.0.
    pub volume: Vec<f32>,
    /// Séparation stéréo, de 0.0 (mono) à 1.0 (Amiga : voies totalement à gauche ou à droite).
    pub separation: f32,
}

impl Mixer {
    pub fn new(channels: usize) -> Self {
        Self {
            mute: vec![false; channels],
            solo: vec![false; channels],
            volume: vec![1.0; channels],
            separation: 0.5,
        }
    }

    pub fn audible(&self, voice: usize) -> bool {
        if self.solo.iter().any(|&s| s) {
            self.solo[voice]
        } else {
            !self.mute[voice]
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Voice {
    /// Numéro du sample courant (1 à 31), 0 = aucun.
    sample: usize,
    playing: bool,
    /// Position dans le sample, en octets (avec la partie fractionnaire).
    pos: f64,
    /// Période de base (avant vibrato et arpège).
    period: i32,
    /// Période réellement jouée pendant ce tick.
    out_period: i32,
    volume: i32,
    out_volume: i32,
    finetune: i8,
    porta_target: i32,
    porta_speed: u8,
    vib_pos: u8,
    vib_speed: u8,
    vib_depth: u8,
    vib_wave: u8,
    trem_pos: u8,
    trem_speed: u8,
    trem_depth: u8,
    trem_wave: u8,
    offset_memory: u8,
    loop_row: usize,
    loop_count: u8,
    /// Cellule de la ligne en cours (sert aux effets des ticks suivants).
    cell: Cell,
}

pub struct Replayer {
    song: Arc<Song>,
    rate: f64,
    pub mixer: Mixer,
    monitor: Option<Arc<Monitor>>,

    speed: u32,
    bpm: u32,
    tick: u32,
    /// Trames restantes avant le prochain tick.
    tick_frames_left: f64,
    position: usize,
    row: usize,
    /// Saut demandé par la ligne en cours (Bxx, Dxx, E6x), appliqué à la fin de la ligne.
    jump_position: Option<usize>,
    jump_row: Option<usize>,
    /// Nombre de répétitions restantes de la ligne (EEx).
    pattern_delay: u32,
    in_pattern_delay: bool,
    voices: Vec<Voice>,
    /// Sortie de chaque voie pour la trame courante (avant mixage).
    voice_out: Vec<f32>,
    /// Lignes déjà jouées (position × 64 + ligne), pour détecter la fin du morceau.
    visited: Vec<bool>,
    ended: bool,
}

impl Replayer {
    pub fn new(song: Arc<Song>, rate: u32) -> Self {
        let channels = song.channels;
        let mut replayer = Self {
            rate: rate as f64,
            mixer: Mixer::new(channels),
            monitor: None,
            speed: 6,
            bpm: 125,
            tick: 0,
            tick_frames_left: 0.0,
            position: 0,
            row: 0,
            jump_position: None,
            jump_row: None,
            pattern_delay: 0,
            in_pattern_delay: false,
            voices: vec![Voice::default(); channels],
            voice_out: vec![0.0; channels],
            visited: vec![false; 128 * 64],
            ended: false,
            song,
        };
        replayer.skip_invalid_positions();
        replayer
    }

    /// Branche un moniteur qui recevra la sortie de chaque voie (oscilloscopes, VU-mètres).
    pub fn set_monitor(&mut self, monitor: Arc<Monitor>) {
        self.monitor = Some(monitor);
    }

    /// Vrai quand le morceau a fini (fin de la liste d'ordre, boucle détectée ou F00).
    /// La lecture continue tout de même, en reprenant au point de reprise.
    pub fn ended(&self) -> bool {
        self.ended
    }

    pub fn position(&self) -> (usize, usize) {
        (self.position, self.row)
    }

    /// État d'une voie : sample, période jouée et volume joué (pour l'affichage et le débogage).
    pub fn voice_state(&self, voice: usize) -> (usize, i32, i32) {
        let v = &self.voices[voice];
        (v.sample, v.out_period, v.out_volume)
    }

    /// Remplit `out` (stéréo entrelacée gauche/droite) avec le mixage de toutes les voies.
    pub fn process(&mut self, out: &mut [f32]) {
        let channels = self.voices.len();
        let master = 2.0 / channels.max(2) as f32;
        for frame in out.as_chunks_mut::<2>().0 {
            self.next_frame();
            let (mut left, mut right) = (0.0, 0.0);
            for (v, &x) in self.voice_out.iter().enumerate() {
                if !self.mixer.audible(v) {
                    continue;
                }
                let x = x * self.mixer.volume[v];
                // Panoramique Amiga : voies 1 et 4 à gauche, 2 et 3 à droite, et ainsi de suite.
                let towards_left = matches!(v % 4, 0 | 3);
                let near = (1.0 + self.mixer.separation) / 2.0;
                let (l, r) = if towards_left {
                    (near, 1.0 - near)
                } else {
                    (1.0 - near, near)
                };
                left += x * l;
                right += x * r;
            }
            frame[0] = (left * master).clamp(-1.0, 1.0);
            frame[1] = (right * master).clamp(-1.0, 1.0);
            if let Some(monitor) = &self.monitor {
                monitor.push(&self.voice_out, (frame[0] + frame[1]) / 2.0);
            }
        }
        if let Some(monitor) = &self.monitor {
            monitor.set_position(self.position, self.row);
        }
    }

    /// Remplit une piste mono par voie (`outs[voie]`), sans tenir compte du mixeur :
    /// sert au rendu « une piste par voie ».
    pub fn process_voices(&mut self, frames: usize, outs: &mut [Vec<f32>]) {
        for _ in 0..frames {
            self.next_frame();
            for (out, &x) in outs.iter_mut().zip(&self.voice_out) {
                out.push(x);
            }
        }
    }

    fn next_frame(&mut self) {
        if self.tick_frames_left <= 0.0 {
            self.do_tick();
            // Durée d'un tick ProTracker : 2,5 / BPM secondes.
            self.tick_frames_left += self.rate * 2.5 / self.bpm as f64;
        }
        self.tick_frames_left -= 1.0;

        let song = &*self.song;
        for (voice, out) in self.voices.iter_mut().zip(&mut self.voice_out) {
            *out = render_voice(voice, song, self.rate);
        }
    }

    // --- Séquenceur ---------------------------------------------------------------------

    fn do_tick(&mut self) {
        if self.tick == 0 {
            if self.in_pattern_delay {
                // Ligne répétée par EEx : les notes ne sont pas rejouées.
            } else {
                self.play_row();
            }
        } else {
            for v in 0..self.voices.len() {
                self.tick_effects(v);
            }
        }
        for voice in &mut self.voices {
            if voice.out_period == 0 {
                voice.out_period = voice.period;
            }
        }

        self.tick += 1;
        if self.tick >= self.speed {
            self.tick = 0;
            self.next_row();
        }
    }

    fn play_row(&mut self) {
        let key = self.position * 64 + self.row.min(63);
        let looping = self.voices.iter().any(|v| v.loop_count > 0);
        if self.visited[key] && !looping {
            self.ended = true;
            self.visited.fill(false);
        }
        self.visited[key] = true;

        let pattern = self.song.orders[self.position] as usize;
        for v in 0..self.voices.len() {
            let cell = self
                .song
                .patterns
                .get(pattern)
                .and_then(|p| p.rows.get(self.row))
                .and_then(|r| r.get(v))
                .copied()
                .unwrap_or_default();
            self.row_effects(v, cell);
        }
    }

    fn next_row(&mut self) {
        if self.pattern_delay > 0 {
            self.pattern_delay -= 1;
            self.in_pattern_delay = true;
            return;
        }
        self.in_pattern_delay = false;

        let rows = self
            .song
            .patterns
            .get(self.song.orders[self.position] as usize)
            .map_or(64, |p| p.rows.len());
        match (self.jump_position.take(), self.jump_row.take()) {
            (None, None) => {
                self.row += 1;
                if self.row >= rows {
                    self.row = 0;
                    self.position += 1;
                }
            }
            (pos, row) => {
                self.position = pos.unwrap_or(self.position + 1);
                self.row = row.unwrap_or(0);
            }
        }
        self.skip_invalid_positions();
        if self.row >= 64 {
            self.row = 0;
        }
    }

    /// Ramène la position dans la liste d'ordre quand on en sort, en marquant la fin du morceau.
    fn skip_invalid_positions(&mut self) {
        let length = (self.song.song_length as usize).clamp(1, 128);
        if self.position >= length {
            self.ended = true;
            self.visited.fill(false);
            let restart = self.song.restart as usize;
            let soundtracker = self.song.kind == ModKind::Soundtracker15;
            self.position = if restart < length && !soundtracker {
                restart
            } else {
                0
            };
            for voice in &mut self.voices {
                voice.loop_count = 0;
            }
        }
    }

    // --- Effets -------------------------------------------------------------------------

    /// Tick 0 : lecture de la cellule, déclenchement de la note, effets ponctuels.
    fn row_effects(&mut self, v: usize, cell: Cell) {
        let song = self.song.clone();
        let voice = &mut self.voices[v];
        voice.cell = cell;
        voice.out_period = 0;
        let (fx, param) = (cell.effect, cell.param);
        let (x, y) = (param >> 4, param & 0x0F);

        if cell.sample != 0 && (cell.sample as usize) <= song.samples.len() {
            let sample = &song.samples[cell.sample as usize - 1];
            voice.sample = cell.sample as usize;
            voice.volume = sample.volume.min(64) as i32;
            voice.finetune = sample.finetune();
        }

        if cell.period != 0 {
            if fx == 0xE && x == 5 {
                voice.finetune = ((y as i8) << 4) >> 4;
            }
            let period = finetuned(cell.period, voice.finetune);
            if fx == 0x3 || fx == 0x5 {
                voice.porta_target = period;
            } else if !(fx == 0xE && x == 0xD && y > 0) {
                voice.period = period;
                trigger(voice, &song);
            }
        }

        match fx {
            0x3 if param != 0 => voice.porta_speed = param,
            0x4 => {
                if x != 0 {
                    voice.vib_speed = x;
                }
                if y != 0 {
                    voice.vib_depth = y;
                }
            }
            0x7 => {
                if x != 0 {
                    voice.trem_speed = x;
                }
                if y != 0 {
                    voice.trem_depth = y;
                }
            }
            0x9 if cell.period != 0 => {
                if param != 0 {
                    voice.offset_memory = param;
                }
                // Au-delà de la fin, ProTracker réduit le sample à un mot puis enchaîne sur
                // la boucle : un sample bouclé joue sa boucle, un sample sans boucle se tait.
                let offset = voice.offset_memory as f64 * 256.0;
                match sample_bounds(voice, &song) {
                    Some(b) if offset < b.end as f64 => voice.pos = offset,
                    Some(Bounds {
                        loop_start: Some(start),
                        ..
                    }) => voice.pos = start as f64,
                    _ => voice.playing = false,
                }
            }
            0xB => {
                self.jump_position = Some(param as usize);
                self.jump_row.get_or_insert(0);
            }
            0xC => voice.volume = param.min(64) as i32,
            0xD => {
                let row = (x * 10 + y) as usize;
                self.jump_row = Some(if row < 64 { row } else { 0 });
                self.jump_position.get_or_insert(self.position + 1);
            }
            0xE => match x {
                0x1 => voice.period = slide(voice.period, -(y as i32)),
                0x2 => voice.period = slide(voice.period, y as i32),
                0x4 => voice.vib_wave = y,
                0x6 => {
                    if y == 0 {
                        voice.loop_row = self.row;
                    } else {
                        if voice.loop_count == 0 {
                            voice.loop_count = y;
                        } else {
                            voice.loop_count -= 1;
                        }
                        if voice.loop_count > 0 {
                            self.jump_position = Some(self.position);
                            self.jump_row = Some(voice.loop_row);
                        }
                    }
                }
                0x7 => voice.trem_wave = y,
                0xA => voice.volume = (voice.volume + y as i32).min(64),
                0xB => voice.volume = (voice.volume - y as i32).max(0),
                // Sans note, ProTracker re-déclenche aussi au tick 0 (avec une note, c'est déjà fait).
                0x9 if y > 0 && cell.period == 0 => trigger(voice, &song),
                0xC if y == 0 => voice.volume = 0,
                0xE if !self.in_pattern_delay => self.pattern_delay = y as u32,
                _ => {}
            },
            0xF => match param {
                0 => self.ended = true,
                1..=0x1F => self.speed = param as u32,
                _ => self.bpm = param as u32,
            },
            _ => {}
        }
        voice.out_volume = voice.volume;
    }

    /// Ticks suivants : effets continus.
    fn tick_effects(&mut self, v: usize) {
        let song = self.song.clone();
        let tick = self.tick;
        let voice = &mut self.voices[v];
        let (fx, param) = (voice.cell.effect, voice.cell.param);
        let (x, y) = (param >> 4, param & 0x0F);
        voice.out_period = voice.period;
        voice.out_volume = voice.volume;

        match fx {
            0x0 if param != 0 => {
                let semitones = [0, x, y][(tick % 3) as usize];
                voice.out_period =
                    (voice.period as f64 * 2f64.powf(-(semitones as f64) / 12.0)).round() as i32;
            }
            0x1 => voice.period = slide(voice.period, -(param as i32)),
            0x2 => voice.period = slide(voice.period, param as i32),
            0x3 => tone_portamento(voice),
            0x4 => vibrato(voice),
            0x5 => {
                tone_portamento(voice);
                volume_slide(voice, x, y);
            }
            0x6 => {
                vibrato(voice);
                volume_slide(voice, x, y);
            }
            0x7 => tremolo(voice),
            0xA => volume_slide(voice, x, y),
            0xE => match x {
                0x9 if y > 0 && tick.is_multiple_of(y as u32) => trigger(voice, &song),
                0xC if tick == y as u32 => voice.volume = 0,
                0xD if tick == y as u32 && voice.cell.period != 0 => {
                    voice.period = finetuned(voice.cell.period, voice.finetune);
                    trigger(voice, &song);
                }
                _ => {}
            },
            _ => {}
        }
        if !matches!(fx, 0x0 | 0x4 | 0x6) || (fx == 0 && param == 0) {
            voice.out_period = voice.period;
        }
        if fx != 0x7 {
            voice.out_volume = voice.volume;
        }
    }
}

// --- Fonctions des voies --------------------------------------------------------------------

/// Période d'une note pour un finetune donné (-8 à +7, en huitièmes de demi-ton).
fn finetuned(period: u16, finetune: i8) -> i32 {
    (period as f64 * 2f64.powf(-(finetune as f64) / 96.0)).round() as i32
}

/// Portamento borné aux limites de ProTracker (sauf pour les notes déjà hors limites,
/// écrites par des trackers étendus).
fn slide(period: i32, delta: i32) -> i32 {
    let p = period + delta;
    if (PERIOD_MIN..=PERIOD_MAX).contains(&period) {
        p.clamp(PERIOD_MIN, PERIOD_MAX)
    } else {
        p.clamp(28, 3424)
    }
}

fn trigger(voice: &mut Voice, song: &Song) {
    voice.pos = 0.0;
    voice.playing = voice.sample != 0 && sample_bounds(voice, song).is_some();
    if voice.vib_wave & 4 == 0 {
        voice.vib_pos = 0;
    }
    if voice.trem_wave & 4 == 0 {
        voice.trem_pos = 0;
    }
}

fn tone_portamento(voice: &mut Voice) {
    let (target, speed) = (voice.porta_target, voice.porta_speed as i32);
    if target == 0 {
        return;
    }
    voice.period = if voice.period < target {
        (voice.period + speed).min(target)
    } else {
        (voice.period - speed).max(target)
    };
}

/// Amplitude de l'onde (0 à 255) et signe, pour une position 0..63.
fn waveform(wave: u8, pos: u8) -> i32 {
    let pos = pos & 63;
    let amplitude = match wave & 3 {
        1 => {
            let ramp = (pos & 31) as i32 * 8;
            if pos >= 32 { 255 - ramp } else { ramp }
        }
        2 => 255,
        _ => SINE[(pos & 31) as usize] as i32,
    };
    if pos >= 32 { -amplitude } else { amplitude }
}

fn vibrato(voice: &mut Voice) {
    let delta = waveform(voice.vib_wave, voice.vib_pos) * voice.vib_depth as i32 / 128;
    voice.out_period = voice.period + delta;
    voice.vib_pos = voice.vib_pos.wrapping_add(voice.vib_speed) & 63;
}

fn tremolo(voice: &mut Voice) {
    let delta = waveform(voice.trem_wave, voice.trem_pos) * voice.trem_depth as i32 / 64;
    voice.out_volume = (voice.volume + delta).clamp(0, 64);
    voice.trem_pos = voice.trem_pos.wrapping_add(voice.trem_speed) & 63;
}

fn volume_slide(voice: &mut Voice, up: u8, down: u8) {
    voice.volume = if up > 0 {
        (voice.volume + up as i32).min(64)
    } else {
        (voice.volume - down as i32).max(0)
    };
}

struct Bounds {
    /// Fin de lecture (fin de boucle si le sample boucle, sinon fin des données).
    end: usize,
    loop_start: Option<usize>,
}

fn sample_bounds(voice: &Voice, song: &Song) -> Option<Bounds> {
    let sample = song.samples.get(voice.sample.checked_sub(1)?)?;
    let len = sample.data.len();
    if len == 0 {
        return None;
    }
    // Soundtracker 15 samples : le début de boucle est en octets, pas en mots.
    let unit = if song.kind == ModKind::Soundtracker15 {
        1
    } else {
        2
    };
    let loop_start = sample.loop_start as usize * unit;
    let loop_end = loop_start + sample.loop_length as usize * 2;
    if sample.loop_length > 1 && loop_start < len {
        Some(Bounds {
            end: loop_end.min(len),
            loop_start: Some(loop_start),
        })
    } else {
        Some(Bounds {
            end: len,
            loop_start: None,
        })
    }
}

/// Une trame de la voie : interpolation linéaire, boucle, volume.
fn render_voice(voice: &mut Voice, song: &Song, rate: f64) -> f32 {
    if !voice.playing || voice.out_period <= 0 {
        return 0.0;
    }
    let Some(bounds) = sample_bounds(voice, song) else {
        voice.playing = false;
        return 0.0;
    };
    let data = &song.samples[voice.sample - 1].data;

    if voice.pos >= bounds.end as f64 {
        match bounds.loop_start {
            Some(start) if bounds.end > start => {
                let length = (bounds.end - start) as f64;
                voice.pos = start as f64 + (voice.pos - start as f64) % length;
            }
            _ => {
                voice.playing = false;
                return 0.0;
            }
        }
    }

    let i = voice.pos as usize;
    let frac = (voice.pos - i as f64) as f32;
    let next = if i + 1 < bounds.end {
        data[i + 1]
    } else {
        bounds.loop_start.map_or(0, |start| data[start])
    };
    let value = data[i] as f32 + (next as f32 - data[i] as f32) * frac;

    voice.pos += PAULA_CLOCK_PAL / (2.0 * voice.out_period as f64) / rate;
    value / 128.0 * voice.out_volume as f32 / 64.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::text::parse_pattern;
    use crate::song::{Pattern, Sample};

    /// Morceau de test : un sample carré de 32 octets en boucle, et un pattern écrit en texte.
    fn song_with(pattern: &str) -> Arc<Song> {
        let square: Vec<i8> = (0..32).map(|i| if i < 16 { 100 } else { -100 }).collect();
        let mut samples = vec![Sample::default(); 31];
        samples[0] = Sample {
            length_words: 16,
            volume: 64,
            loop_start: 0,
            loop_length: 16,
            data: square,
            ..Sample::default()
        };
        let mut orders = [0u8; 128];
        orders[1] = 0;
        Arc::new(Song {
            title: [0; 20],
            kind: ModKind::Tagged(*b"M.K."),
            channels: 4,
            samples,
            song_length: 1,
            restart: 127,
            orders,
            patterns: vec![parse_pattern(pattern, 64, 4).unwrap()],
            trailing: Vec::new(),
        })
    }

    fn render_until_end(song: Arc<Song>) -> (usize, Vec<f32>) {
        let mut r = Replayer::new(song, 44100);
        let mut out = Vec::new();
        let mut buf = [0.0f32; 2 * 441];
        while !r.ended() && out.len() < 44100 * 2 * 120 {
            r.process(&mut buf);
            out.extend_from_slice(&buf);
        }
        (out.len() / 2, out)
    }

    #[test]
    fn default_tempo_lasts_64_rows_of_120_ms() {
        let (frames, _) = render_until_end(song_with(
            "00 | C-2 01 ... | ... .. ... | ... .. ... | ... .. ...",
        ));
        let seconds = frames as f64 / 44100.0;
        assert!((seconds - 7.68).abs() < 0.02, "{seconds}");
    }

    #[test]
    fn speed_and_pattern_break_change_duration() {
        // Vitesse 3 (deux fois plus rapide) et saut en fin de ligne 15 : 16 lignes à 60 ms.
        let (frames, _) = render_until_end(song_with(
            "00 | C-2 01 F03 | ... .. ... | ... .. ... | ... .. ...\n15 | ... .. D00 | ... .. ... | ... .. ... | ... .. ...",
        ));
        let seconds = frames as f64 / 44100.0;
        assert!((seconds - 0.96).abs() < 0.02, "{seconds}");
    }

    #[test]
    fn note_plays_at_amiga_pitch() {
        // C-2 : 8287 octets/s, sample de 32 octets → 259 Hz. On compte les passages par zéro.
        let mut r = Replayer::new(
            song_with("00 | C-2 01 ... | ... .. ... | ... .. ... | ... .. ..."),
            44100,
        );
        r.mixer.separation = 0.0;
        let mut out = vec![0.0f32; 2 * 44100];
        r.process(&mut out);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let crossings = left
            .windows(2)
            .filter(|w| w[0] > 0.0 && w[1] <= 0.0)
            .count();
        assert!((crossings as i32 - 259).abs() <= 2, "{crossings}");
    }

    #[test]
    fn mute_and_solo_silence_voices() {
        let song = song_with("00 | C-2 01 ... | ... .. ... | ... .. ... | ... .. ...");
        let peak = |configure: &dyn Fn(&mut Mixer)| {
            let mut r = Replayer::new(song.clone(), 44100);
            configure(&mut r.mixer);
            let mut out = vec![0.0f32; 2 * 4410];
            r.process(&mut out);
            out.iter().fold(0.0f32, |m, x| m.max(x.abs()))
        };
        assert!(peak(&|_| {}) > 0.1);
        assert_eq!(peak(&|m| m.mute[0] = true), 0.0);
        assert_eq!(peak(&|m| m.solo[1] = true), 0.0);
        assert!(peak(&|m| m.solo[0] = true) > 0.1);
        assert!(peak(&|m| m.volume[0] = 0.5) < peak(&|_| {}));
    }

    #[test]
    fn volume_slide_fades_out() {
        let mut text = String::from("00 | C-2 01 A0F | ... .. ... | ... .. ... | ... .. ...\n");
        for row in 1..8 {
            text += &format!("{row:02} | ... .. A0F | ... .. ... | ... .. ... | ... .. ...\n");
        }
        let mut r = Replayer::new(song_with(&text), 44100);
        let mut out = vec![0.0f32; 2 * 44100];
        r.process(&mut out);
        let tail = &out[out.len() - 2000..];
        assert_eq!(tail.iter().fold(0.0f32, |m, x| m.max(x.abs())), 0.0);
    }

    /// Crête de la sortie entre deux instants (en secondes).
    fn peak_between(song: Arc<Song>, from: f64, to: f64) -> f32 {
        let mut r = Replayer::new(song, 44100);
        let mut out = vec![0.0f32; 2 * (to * 44100.0) as usize];
        r.process(&mut out);
        out[2 * (from * 44100.0) as usize..]
            .iter()
            .fold(0.0f32, |m, x| m.max(x.abs()))
    }

    #[test]
    fn sample_offset_past_the_end_plays_the_loop() {
        // Le sample de test boucle sur 32 octets : 9FF (65 280 octets) tombe au-delà de la fin.
        let song = song_with("00 | C-2 01 9FF | ... .. ... | ... .. ... | ... .. ...");
        assert!(peak_between(song, 0.0, 0.1) > 0.1);
    }

    #[test]
    fn retrigger_without_note_restarts_at_tick_0() {
        // Sample sans boucle de 8 octets : il se tait en 1 ms. E93 le relance aux ticks 0 et 3.
        let mut song = (*song_with(
            "00 | C-2 01 ... | ... .. ... | ... .. ... | ... .. ...\n01 | ... .. E93 | ... .. ... | ... .. ... | ... .. ...",
        ))
        .clone();
        song.samples[0].loop_length = 1;
        song.samples[0].data.truncate(8);
        // Tick 0 de la ligne 1 : de 120 à 140 ms.
        assert!(peak_between(Arc::new(song), 0.1195, 0.1225) > 0.1);
    }

    #[test]
    fn empty_pattern_is_silent() {
        let song = Arc::new(Song {
            patterns: vec![Pattern::new(64, 4)],
            ..(*song_with("")).clone()
        });
        let (_, out) = render_until_end(song);
        assert!(out.iter().all(|&x| x == 0.0));
    }
}
