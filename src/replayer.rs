//! ProTracker replayer: sequencer, effects, resampling and mixing.
//!
//! Goal: clean playback with ProTracker's exact timing, without emulating the Amiga hardware
//! (no Paula filter). The same engine drives real-time playback and WAV rendering.
//! Nothing is allocated on the audio path (`process`). « Pas de cale qui fuit à bord. »

use std::sync::Arc;

use crate::monitor::Monitor;
use crate::note::PAULA_CLOCK_PAL;
use crate::song::{Cell, ModKind, Song};

/// ProTracker's vibrato and tremolo table (half sine, 32 values).
const SINE: [u8; 32] = [
    0, 24, 49, 74, 97, 120, 141, 161, 180, 197, 212, 224, 235, 244, 250, 253, 255, 253, 250, 244,
    235, 224, 212, 197, 180, 161, 141, 120, 97, 74, 49, 24,
];

/// ProTracker's period limits (B-3 to C-1) for portamentos.
const PERIOD_MIN: i32 = 113;
const PERIOD_MAX: i32 = 856;

/// Per-voice mix settings, for the session only (not saved in the `.mod`).
#[derive(Debug, Clone)]
pub struct Mixer {
    pub mute: Vec<bool>,
    pub solo: Vec<bool>,
    /// Volume of each voice, from 0.0 to 1.0.
    pub volume: Vec<f32>,
    /// Stereo separation, from 0.0 (mono) to 1.0 (Amiga: voices hard left or hard right).
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
    /// Current sample number (1 to 31), 0 = none.
    sample: usize,
    playing: bool,
    /// Position in the sample, in bytes (fractional part included).
    pos: f64,
    /// Base period (before vibrato and arpeggio).
    period: i32,
    /// Period actually played during this tick.
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
    /// Cell of the current row (drives the effects of the following ticks).
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
    /// Frames left before the next tick.
    tick_frames_left: f64,
    position: usize,
    row: usize,
    /// Jump requested by the current row (Bxx, Dxx, E6x), applied at the end of the row.
    jump_position: Option<usize>,
    jump_row: Option<usize>,
    /// Repeats left for the current row (EEx).
    pattern_delay: u32,
    in_pattern_delay: bool,
    voices: Vec<Voice>,
    /// Output of each voice for the current frame (before mixing).
    voice_out: Vec<f32>,
    /// Rows already played (position × 64 + row), to detect the end of the song.
    visited: Vec<bool>,
    ended: bool,
    /// False when playback is stopped: the sequencer stands still, but notes played by hand
    /// (`jam`) still sound.
    running: bool,
    /// Loops on the same position (pattern playback).
    loop_pattern: bool,
    /// Notes played on the keyboard to listen to them: one per voice, rendered exactly as the
    /// voice would play it (same volume, same place in the stereo field), but on the side, so
    /// that song playback does not cut it.
    jam: Vec<Voice>,
    /// Output of the listened notes for the current frame, per voice.
    jam_out: Vec<f32>,
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
            running: true,
            loop_pattern: false,
            jam: vec![Voice::default(); channels],
            jam_out: vec![0.0; channels],
            song,
        };
        replayer.skip_invalid_positions();
        replayer
    }

    /// Plugs in a monitor that gets the output of each voice (scopes, meters).
    pub fn set_monitor(&mut self, monitor: Arc<Monitor>) {
        self.monitor = Some(monitor);
    }

    /// True once the song is over (end of the order list, loop detected, or F00).
    /// Playback goes on anyway, from the restart point.
    pub fn ended(&self) -> bool {
        self.ended
    }

    pub fn position(&self) -> (usize, usize) {
        (self.position, self.row)
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Current speed (ticks per row) and tempo (BPM).
    pub fn tempo(&self) -> (u32, u32) {
        (self.speed, self.bpm)
    }

    /// Swaps the song being played (after an edit) without cutting the sound. The voice
    /// count must not change.
    pub fn set_song(&mut self, song: Arc<Song>) {
        debug_assert_eq!(song.channels, self.voices.len());
        self.song = song;
        self.skip_invalid_positions();
    }

    /// Starts playback at a position of the order list, or loops a single pattern.
    /// Speed and tempo are the ones set by the Fxx of the previous positions.
    pub fn play(&mut self, position: usize, loop_pattern: bool) {
        let (mut speed, mut bpm) = (6, 125);
        for &p in &self.song.orders[..position.min(self.song.order_list().len())] {
            for cell in self
                .song
                .patterns
                .get(p as usize)
                .iter()
                .flat_map(|p| p.rows.iter().flatten())
            {
                match (cell.effect, cell.param) {
                    (0xF, 1..=0x1F) => speed = cell.param as u32,
                    (0xF, 0x20..) => bpm = cell.param as u32,
                    _ => {}
                }
            }
        }
        (self.speed, self.bpm) = (speed, bpm);
        (self.position, self.row, self.tick, self.tick_frames_left) = (position, 0, 0, 0.0);
        (self.jump_position, self.jump_row) = (None, None);
        (self.pattern_delay, self.in_pattern_delay) = (0, false);
        self.visited.fill(false);
        self.ended = false;
        self.loop_pattern = loop_pattern;
        self.voices.iter_mut().for_each(|v| *v = Voice::default());
        self.skip_invalid_positions();
        self.running = true;
    }

    /// Stops playback and silences every voice.
    pub fn stop(&mut self) {
        self.running = false;
        self.voices.iter_mut().for_each(|v| v.playing = false);
    }

    /// Plays a note on a voice to listen to it, whether playback runs or not. It sounds as if
    /// it were written in the pattern: until the next note, the end of the sample, or
    /// `jam_stop`.
    pub fn jam(&mut self, voice: usize, sample: usize, period: u16) {
        let song = self.song.clone();
        let Some(s) = song.samples.get(sample.wrapping_sub(1)) else {
            return;
        };
        let Some(v) = self.jam.get_mut(voice) else {
            return;
        };
        *v = Voice {
            sample,
            volume: s.volume.min(64) as i32,
            finetune: s.finetune(),
            ..Voice::default()
        };
        v.out_volume = v.volume;
        v.period = finetuned(period, v.finetune);
        v.out_period = v.period;
        trigger(v, &song);
    }

    /// Stops every listened note.
    pub fn jam_stop(&mut self) {
        self.jam.iter_mut().for_each(|v| v.playing = false);
    }

    /// State of a voice: sample, played period and played volume (for display and debugging).
    pub fn voice_state(&self, voice: usize) -> (usize, i32, i32) {
        let v = &self.voices[voice];
        (v.sample, v.out_period, v.out_volume)
    }

    /// Fills `out` (interleaved left/right stereo) with the mix of every voice.
    pub fn process(&mut self, out: &mut [f32]) {
        let channels = self.voices.len();
        let master = 2.0 / channels.max(2) as f32;
        for frame in out.as_chunks_mut::<2>().0 {
            self.next_frame();
            for (voice, out) in self.jam.iter_mut().zip(&mut self.jam_out) {
                *out = render_voice(voice, &self.song, self.rate);
            }
            let (mut left, mut right) = (0.0, 0.0);
            for (v, (&x, &jam)) in self.voice_out.iter().zip(&self.jam_out).enumerate() {
                // A listened note is heard even on a muted voice: we asked for it.
                let x = if self.mixer.audible(v) { x + jam } else { jam };
                let x = x * self.mixer.volume[v];
                // Amiga panning: voices 1 and 4 left, 2 and 3 right, and so on.
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
                // A voice's scope also shows the listened note.
                for (out, &jam) in self.voice_out.iter_mut().zip(&self.jam_out) {
                    *out += jam;
                }
                monitor.push(&self.voice_out, (frame[0] + frame[1]) / 2.0);
            }
        }
        if let Some(monitor) = &self.monitor {
            monitor.set_position(self.position, self.row);
        }
    }

    /// Fills one mono track per voice (`outs[voice]`), ignoring the mixer:
    /// used for the "one track per voice" render.
    pub fn process_voices(&mut self, frames: usize, outs: &mut [Vec<f32>]) {
        for _ in 0..frames {
            self.next_frame();
            for (out, &x) in outs.iter_mut().zip(&self.voice_out) {
                out.push(x);
            }
        }
    }

    fn next_frame(&mut self) {
        if self.running && self.tick_frames_left <= 0.0 {
            self.do_tick();
            // A ProTracker tick lasts 2.5 / BPM seconds.
            self.tick_frames_left += self.rate * 2.5 / self.bpm as f64;
        }
        self.tick_frames_left -= 1.0;

        let song = &*self.song;
        for (voice, out) in self.voices.iter_mut().zip(&mut self.voice_out) {
            *out = render_voice(voice, song, self.rate);
        }
    }

    // --- Sequencer -----------------------------------------------------------------------

    fn do_tick(&mut self) {
        if self.tick == 0 {
            if self.in_pattern_delay {
                // Row repeated by EEx: notes are not played again.
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
        let current = self.position;

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
        if self.loop_pattern {
            self.position = current;
            if self.row == 0 {
                self.visited.fill(false);
            }
        }
        self.skip_invalid_positions();
        if self.row >= 64 {
            self.row = 0;
        }
    }

    /// Brings the position back into the order list when it falls out, flagging the song end.
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

    // --- Effects -------------------------------------------------------------------------

    /// Tick 0: read the cell, trigger the note, one-shot effects.
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
                // Past the end, ProTracker shrinks the sample to one word, then moves on to the
                // loop: a looped sample plays its loop, a one-shot sample goes quiet.
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
                // Without a note, ProTracker also retriggers on tick 0 (with one, it already did).
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

    /// Following ticks: continuous effects.
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

// --- Voice helpers ---------------------------------------------------------------------------

/// Period of a note for a given finetune (-8 to +7, in eighths of a semitone).
fn finetuned(period: u16, finetune: i8) -> i32 {
    (period as f64 * 2f64.powf(-(finetune as f64) / 96.0)).round() as i32
}

/// Portamento clamped to ProTracker's limits (except for notes already out of range,
/// written by extended trackers).
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

/// Wave amplitude (0 to 255) and sign, for a position 0..63.
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
    /// Where playback ends (loop end for a looped sample, otherwise end of data).
    end: usize,
    loop_start: Option<usize>,
}

fn sample_bounds(voice: &Voice, song: &Song) -> Option<Bounds> {
    let sample = song.samples.get(voice.sample.checked_sub(1)?)?;
    let len = sample.data.len();
    if len == 0 {
        return None;
    }
    // 15-sample Soundtracker: the loop start is in bytes, not words.
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

/// One frame of a voice: linear interpolation, loop, volume.
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

    /// Test song: a looped 32-byte square sample, and a pattern written as text.
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
        // Speed 3 (twice as fast) and a break at the end of row 15: 16 rows of 60 ms.
        let (frames, _) = render_until_end(song_with(
            "00 | C-2 01 F03 | ... .. ... | ... .. ... | ... .. ...\n15 | ... .. D00 | ... .. ... | ... .. ... | ... .. ...",
        ));
        let seconds = frames as f64 / 44100.0;
        assert!((seconds - 0.96).abs() < 0.02, "{seconds}");
    }

    #[test]
    fn note_plays_at_amiga_pitch() {
        // C-2: 8287 bytes/s, 32-byte sample → 259 Hz. Count the zero crossings.
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

    /// Output peak between two instants (in seconds).
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
        // The test sample loops over 32 bytes: 9FF (65,280 bytes) lands past its end.
        let song = song_with("00 | C-2 01 9FF | ... .. ... | ... .. ... | ... .. ...");
        assert!(peak_between(song, 0.0, 0.1) > 0.1);
    }

    #[test]
    fn retrigger_without_note_restarts_at_tick_0() {
        // One-shot 8-byte sample: silent within 1 ms. E93 retriggers it on ticks 0 and 3.
        let mut song = (*song_with(
            "00 | C-2 01 ... | ... .. ... | ... .. ... | ... .. ...\n01 | ... .. E93 | ... .. ... | ... .. ... | ... .. ...",
        ))
        .clone();
        song.samples[0].loop_length = 1;
        song.samples[0].data.truncate(8);
        // Tick 0 of row 1: from 120 to 140 ms.
        assert!(peak_between(Arc::new(song), 0.1195, 0.1225) > 0.1);
    }

    #[test]
    fn stopped_replayer_only_plays_jammed_notes() {
        let mut r = Replayer::new(
            song_with("00 | C-2 01 ... | ... .. ... | ... .. ... | ... .. ..."),
            44100,
        );
        r.stop();
        let mut out = vec![0.0f32; 2 * 4410];
        r.process(&mut out);
        assert!(out.iter().all(|&x| x == 0.0));
        assert_eq!(r.position(), (0, 0));
        r.jam(1, 1, 428);
        r.process(&mut out);
        assert!(out.iter().any(|&x| x != 0.0));
        assert_eq!(r.position(), (0, 0));
        r.jam_stop();
        r.process(&mut out);
        assert!(out.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn pattern_loop_mode_stays_on_one_position() {
        let mut song =
            (*song_with("00 | C-2 01 ... | ... .. ... | ... .. ... | ... .. ...")).clone();
        song.patterns.push(Pattern::new(64, 4));
        song.song_length = 2;
        song.orders[1] = 1;
        let mut r = Replayer::new(Arc::new(song), 1000);
        r.play(1, true);
        let mut out = vec![0.0f32; 2 * 1000 * 10];
        r.process(&mut out);
        assert_eq!(r.position().0, 1);
        r.play(0, false);
        r.process(&mut out);
        assert_eq!(r.position().0, 1);
    }

    #[test]
    fn play_from_a_position_picks_up_earlier_tempo() {
        let mut song =
            (*song_with("00 | ... .. F03 | ... .. F90 | ... .. ... | ... .. ...")).clone();
        song.patterns.push(Pattern::new(64, 4));
        song.song_length = 2;
        song.orders[1] = 1;
        let mut r = Replayer::new(Arc::new(song), 1000);
        r.play(1, false);
        assert_eq!(r.tempo(), (3, 0x90));
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
