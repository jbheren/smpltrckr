//! Sound check: plays a chiptune arpeggio and measures how the audio stream behaves.
//!
//! The sound is made the replayer way: a tiny signed 8-bit square sample read at the rate
//! given by an Amiga period.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

use anyhow::Context;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, OutputCallbackInfo, SampleFormat, SizedSample, StreamConfig};
use rust_i18n::t;

use crate::note;

/// A-minor arpeggio, one note per row (speed 6, 125 BPM: 120 ms per row).
const TUNE: [&str; 8] = ["A-2", "C-3", "E-3", "A-3", "E-3", "C-3", "A-2", "E-2"];
const ROW: Duration = Duration::from_millis(120);

/// Counters shared by the audio callback and the main thread (lock-free).
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
        .with_context(|| t!("audio.no_output"))?;
    let supported = device.default_output_config()?;
    println!("{}", t!("tone.host", host = format!("{:?}", host.id())));
    println!("{}", t!("tone.output", device = device.id()?));
    println!("{}", t!("tone.config", config = format!("{supported:?}")));

    let stats = Arc::new(Stats {
        frames_min: AtomicU64::new(u64::MAX),
        ..Default::default()
    });
    let config: StreamConfig = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, stats.clone()),
        SampleFormat::I16 => build::<i16>(&device, &config, stats.clone()),
        SampleFormat::I32 => build::<i32>(&device, &config, stats.clone()),
        other => anyhow::bail!(t!("audio.unsupported_format", format = other)),
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
    println!(
        "{}",
        t!("tone.callbacks", count = stats.callbacks.load(Relaxed))
    );
    let (ms_min, ms_max) = (format!("{:.1}", ms(fmin)), format!("{:.1}", ms(fmax)));
    println!(
        "{}",
        t!(
            "tone.buffer",
            min = fmin,
            max = fmax,
            ms_min = ms_min,
            ms_max = ms_max
        )
    );
    let latency = format!("{:.1}", stats.latency_max_us.load(Relaxed) as f64 / 1000.0);
    println!("{}", t!("tone.latency", ms = latency));
    println!("{}", t!("tone.busy", us = stats.busy_max_us.load(Relaxed)));
    println!("{}", t!("tone.errors", count = stats.errors.load(Relaxed)));
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

    // "Chip" sample: 32 bytes of looped signed 8-bit square wave.
    let sample: Vec<i8> = (0..32).map(|i| if i < 16 { 64 } else { -64 }).collect();
    // Periods computed up front: the audio callback must not allocate.
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
                // Position in the sample, in Paula "bytes per second".
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
            eprintln!("{}", t!("audio.error", error = err));
        },
        None,
    )?;
    Ok(stream)
}
