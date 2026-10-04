//! Particles thrown off the voice scopes: each attack throws a spray of dots from the crest of
//! the wave; they rise, fall back and fade out. Coordinates are relative to the scope area:
//! x from 0 (left) to 1 (right), y from 0 (top) to 1 (bottom).

use crate::monitor::Monitor;

#[derive(Debug, Clone, Copy)]
pub struct Particle {
    pub x: f32,
    pub y: f32,
    vx: f32,
    vy: f32,
    /// Age and lifetime, in seconds.
    age: f32,
    life: f32,
}

impl Particle {
    /// How far along its life the particle is, from 0 (new) to 1 (gone).
    pub fn fade(&self) -> f32 {
        (self.age / self.life).min(1.0)
    }
}

/// Pull of gravity, in scope heights per second squared.
const GRAVITY: f32 = 3.0;
/// A level jump at least this big counts as an attack.
const ATTACK: f32 = 0.08;
/// Particles alive at most per voice, to keep the display light.
const MAX_PER_VOICE: usize = 60;

pub struct Particles {
    pub voices: Vec<Vec<Particle>>,
    levels: Vec<f32>,
    seed: u32,
}

impl Particles {
    pub fn new(voices: usize) -> Self {
        Self {
            voices: vec![Vec::new(); voices],
            levels: vec![0.0; voices],
            seed: 0x5eed_1234,
        }
    }

    /// Pseudo-random number in [0, 1) (xorshift: cheap and good enough for sparks).
    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Moves the particles by `dt` seconds and throws new ones where voices attack or ring.
    /// `gain` is the scope zoom, so particles leave from the crest as drawn.
    pub fn update(&mut self, monitor: &Monitor, gain: f32, dt: f32) {
        if self.voices.len() != monitor.voices() {
            *self = Self::new(monitor.voices());
        }
        for voice in &mut self.voices {
            for p in voice.iter_mut() {
                p.vy += GRAVITY * dt;
                p.x += p.vx * dt;
                p.y += p.vy * dt;
                p.age += dt;
            }
            voice.retain(|p| p.age < p.life && p.y < 1.2 && (0.0..=1.0).contains(&p.x));
        }
        for v in 0..self.voices.len() {
            let wave = monitor.triggered_scope(Some(v));
            let level = wave.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            let attack = level - self.levels[v] > ATTACK;
            self.levels[v] = level;
            // A burst on each attack, a few sparks while the note rings.
            let count = if attack {
                (6.0 + level * 14.0) as usize
            } else {
                usize::from(self.random() < level * 0.5)
            };
            let Some((i, crest)) = wave
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            else {
                continue;
            };
            let x0 = i as f32 / wave.len().max(1) as f32;
            let y0 = (1.0 - ((crest * gain).clamp(-1.0, 1.0) + 1.0) / 2.0).clamp(0.0, 1.0);
            for _ in 0..count {
                if self.voices[v].len() >= MAX_PER_VOICE {
                    break;
                }
                let particle = Particle {
                    x: (x0 + (self.random() - 0.5) * 0.2).clamp(0.0, 1.0),
                    y: y0,
                    vx: (self.random() - 0.5) * 0.9,
                    vy: -(0.5 + self.random() * 1.2),
                    age: 0.0,
                    life: 0.5 + self.random() * 0.5,
                };
                self.voices[v].push(particle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_attack_throws_particles_that_fall_and_fade() {
        let monitor = Monitor::new(2);
        let mut particles = Particles::new(2);
        particles.update(&monitor, 2.0, 0.016);
        assert!(
            particles.voices.iter().all(Vec::is_empty),
            "silence throws nothing"
        );

        // A loud square wave on voice 1: an attack.
        for i in 0..4096 {
            let x = if (i / 32) % 2 == 0 { 0.5 } else { -0.5 };
            monitor.push(&[x, 0.0], x / 2.0);
        }
        particles.update(&monitor, 2.0, 0.016);
        assert!(
            particles.voices[0].len() >= 6,
            "{}",
            particles.voices[0].len()
        );
        assert!(particles.voices[1].is_empty());

        // They rise first, then gravity wins and they all fade away.
        let start = particles.voices[0][0].y;
        particles.update(&monitor, 2.0, 0.05);
        assert!(particles.voices[0][0].y < start);
        for _ in 0..80 {
            particles.update(&Monitor::new(2), 2.0, 0.05);
        }
        assert!(particles.voices[0].is_empty());
    }
}
