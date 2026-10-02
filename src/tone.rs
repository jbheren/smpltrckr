//! Brique « son » : joue un arpège façon chiptune et mesure le comportement du flux audio.
//!
//! Le son est produit comme le fera le replayer : un petit sample 8 bits signé (onde carrée)
//! lu à la fréquence donnée par une période Amiga.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

use anyhow::Context;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, OutputCallbackInfo, SampleFormat, SizedSample, StreamConfig};

use crate::note;

/// Arpège en la mineur, une note par ligne (vitesse 6, 125 BPM : 120 ms par ligne).
const TUNE: [&str; 8] = ["A-2", "C-3", "E-3", "A-3", "E-3", "C-3", "A-2", "E-2"];
const ROW: Duration = Duration::from_millis(120);

/// Compteurs partagés entre le callback audio et le thread principal (sans verrou).
#[derive(Default)]
struct Stats {
    callbacks: AtomicU64,
    frames_min: AtomicU64,
    frames_max: AtomicU64,
    busy_max_us: AtomicU64,
    latency_max_us: AtomicU64,
    errors: AtomicU64,
}

pub fn run(seconds: u64) -> anyhow::Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .context("aucune sortie audio par défaut")?;
    let supported = device.default_output_config()?;
    println!("Hôte audio  : {:?}", host.id());
    println!("Sortie      : {}", device.id()?);
    println!("Config      : {supported:?}");

    let stats = Arc::new(Stats {
        frames_min: AtomicU64::new(u64::MAX),
        ..Default::default()
    });
    let config: StreamConfig = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, stats.clone()),
        SampleFormat::I16 => build::<i16>(&device, &config, stats.clone()),
        SampleFormat::I32 => build::<i32>(&device, &config, stats.clone()),
        other => anyhow::bail!("format d'échantillon non géré : {other}"),
    }?;

    stream.play()?;
    std::thread::sleep(Duration::from_secs(seconds));
    drop(stream);

    let rate = config.sample_rate as f64;
    let ms = |frames: u64| frames as f64 * 1000.0 / rate;
    let (fmin, fmax) = (
        stats.frames_min.load(Relaxed),
        stats.frames_max.load(Relaxed),
    );
    println!();
    println!("Callbacks   : {}", stats.callbacks.load(Relaxed));
    println!(
        "Tampon      : {fmin} à {fmax} trames ({:.1} à {:.1} ms)",
        ms(fmin),
        ms(fmax)
    );
    println!(
        "Latence max : {:.1} ms (callback → sortie, estimée par cpal)",
        stats.latency_max_us.load(Relaxed) as f64 / 1000.0
    );
    println!(
        "Calcul max  : {} µs par callback",
        stats.busy_max_us.load(Relaxed)
    );
    println!(
        "Erreurs     : {} (décrochages, changements de périphérique…)",
        stats.errors.load(Relaxed)
    );
    Ok(())
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    stats: Arc<Stats>,
) -> anyhow::Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let rate = config.sample_rate as f64;
    let channels = config.channels as usize;

    // Sample « chip » : 32 octets d'onde carrée 8 bits signée, en boucle.
    let sample: Vec<i8> = (0..32).map(|i| if i < 16 { 64 } else { -64 }).collect();
    // Périodes calculées d'avance : le callback audio ne doit rien allouer.
    let periods = TUNE.map(|n| note::PERIODS[note::parse(n).unwrap()]);
    let frames_per_row = (ROW.as_secs_f64() * rate) as u64;
    let (mut frame, mut pos) = (0u64, 0f64);

    let errors = stats.clone();
    let stream = device.build_output_stream(
        *config,
        move |out: &mut [T], info: &OutputCallbackInfo| {
            let start = Instant::now();
            for chunk in out.chunks_mut(channels) {
                let period = periods[(frame / frames_per_row) as usize % periods.len()];
                // Position dans le sample en « octets par seconde » de Paula.
                pos = (pos + note::period_to_hz(period) / rate) % sample.len() as f64;
                let value = T::from_sample(sample[pos as usize] as f32 / 128.0 * 0.3);
                chunk.fill(value);
                frame += 1;
            }

            let frames = (out.len() / channels) as u64;
            let ts = info.timestamp();
            stats.callbacks.fetch_add(1, Relaxed);
            stats.frames_min.fetch_min(frames, Relaxed);
            stats.frames_max.fetch_max(frames, Relaxed);
            stats.latency_max_us.fetch_max(
                ts.playback.duration_since(ts.callback).as_micros() as u64,
                Relaxed,
            );
            stats
                .busy_max_us
                .fetch_max(start.elapsed().as_micros() as u64, Relaxed);
        },
        move |err| {
            errors.errors.fetch_add(1, Relaxed);
            eprintln!("erreur audio : {err}");
        },
        None,
    )?;
    Ok(stream)
}
