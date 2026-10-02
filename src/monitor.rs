//! Moniteur : ce que le replayer publie pour l'affichage (oscilloscopes, VU-mètres, position).
//!
//! Un seul écrivain (le thread audio) et des lecteurs quelconques, sans verrou : chaque
//! échantillon est un `AtomicU32`. Un lecteur peut voir un tampon à moitié mis à jour, ce qui
//! est sans conséquence pour un affichage.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};

/// Nombre d'échantillons gardés par voie (environ 21 ms à 48 kHz).
pub const SCOPE_LEN: usize = 1024;

pub struct Monitor {
    voices: usize,
    /// `voices + 1` tampons circulaires (le dernier est le master), à la suite.
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

    /// Position (dans la liste d'ordre) et ligne en cours de lecture.
    pub fn position(&self) -> (usize, usize) {
        (self.position.load(Relaxed), self.row.load(Relaxed))
    }

    /// Copie les `out.len()` derniers échantillons d'une voie (`None` = master), du plus ancien
    /// au plus récent.
    pub fn scope(&self, voice: Option<usize>, out: &mut [f32]) {
        let base = voice.unwrap_or(self.voices).min(self.voices) * SCOPE_LEN;
        let end = self.write.load(Relaxed);
        let n = out.len().min(SCOPE_LEN);
        for (k, slot) in out.iter_mut().take(n).enumerate() {
            let i = (end + SCOPE_LEN - n + k) % SCOPE_LEN;
            *slot = f32::from_bits(self.samples[base + i].load(Relaxed));
        }
    }

    /// Crête récente d'une voie (`None` = master), de 0.0 à 1.0.
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
