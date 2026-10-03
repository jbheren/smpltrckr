//! Monitor: what the replayer publishes for the display (scopes, meters, position).
//!
//! One writer (the audio thread) and any number of readers, lock-free: each sample is an
//! `AtomicU32`. A reader may see a half-updated buffer, which does no harm to a display.
//!

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};

/// Samples kept per voice (about 43 ms at 48 kHz).
pub const SCOPE_LEN: usize = 2048;

pub struct Monitor {
    voices: usize,
    /// `voices + 1` ring buffers (the last one is the master), back to back.
    samples: Vec<AtomicU32>,
    write: AtomicUsize,
    position: AtomicUsize,
    row: AtomicUsize,
}

impl Monitor {
    pub fn new(voices: usize) -> Arc<Self> {
        Arc::new(Self {
            voices,
            samples: (0..(voices + 1) * SCOPE_LEN)
                .map(|_| AtomicU32::new(0))
                .collect(),
            write: AtomicUsize::new(0),
            position: AtomicUsize::new(0),
            row: AtomicUsize::new(0),
        })
    }

    pub fn voices(&self) -> usize {
        self.voices
    }

    pub(crate) fn push(&self, voices: &[f32], master: f32) {
        let i = self.write.load(Relaxed) % SCOPE_LEN;
        for (v, &x) in voices.iter().enumerate().take(self.voices) {
            self.samples[v * SCOPE_LEN + i].store(x.to_bits(), Relaxed);
        }
        self.samples[self.voices * SCOPE_LEN + i].store(master.to_bits(), Relaxed);
        self.write.store(i + 1, Relaxed);
    }

    pub(crate) fn set_position(&self, position: usize, row: usize) {
        self.position.store(position, Relaxed);
        self.row.store(row, Relaxed);
    }

    /// Position (in the order list) and row being played.
    pub fn position(&self) -> (usize, usize) {
        (self.position.load(Relaxed), self.row.load(Relaxed))
    }

    /// Copies the last `out.len()` samples of a voice (`None` = master), oldest first.
    pub fn scope(&self, voice: Option<usize>, out: &mut [f32]) {
        let base = voice.unwrap_or(self.voices).min(self.voices) * SCOPE_LEN;
        let end = self.write.load(Relaxed);
        let n = out.len().min(SCOPE_LEN);
        for (k, slot) in out.iter_mut().take(n).enumerate() {
            let i = (end + SCOPE_LEN - n + k) % SCOPE_LEN;
            *slot = f32::from_bits(self.samples[base + i].load(Relaxed));
        }
    }

    /// Recent peak of a voice (`None` = master), from 0.0 to 1.0.
    pub fn level(&self, voice: Option<usize>) -> f32 {
        let mut recent = [0.0f32; 512];
        self.scope(voice, &mut recent);
        recent.iter().fold(0.0f32, |m, x| m.max(x.abs())).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_returns_latest_samples_in_order() {
        let m = Monitor::new(2);
        for i in 0..(SCOPE_LEN + 10) {
            m.push(&[i as f32, -(i as f32)], 0.5);
        }
        let mut out = [0.0; 3];
        m.scope(Some(0), &mut out);
        let last = (SCOPE_LEN + 9) as f32;
        assert_eq!(out, [last - 2.0, last - 1.0, last]);
        m.scope(Some(1), &mut out);
        assert_eq!(out[2], -last);
        assert_eq!(m.level(None), 0.5);
    }
}
