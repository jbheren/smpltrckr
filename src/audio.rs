//! Real-time audio output (cpal), fed by the replayer.

use std::sync::{Arc, Mutex};

use anyhow::Context;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

use crate::replayer::Replayer;
use rust_i18n::t;

/// Opens the default output. `make` gets the device sample rate and builds the replayer.
/// The replayer stays reachable through the returned `Mutex`, locked briefly by the audio
/// callback; other threads must only hold it for short operations.
pub fn start(
    make: impl FnOnce(u32) -> Replayer,
) -> anyhow::Result<(cpal::Stream, Arc<Mutex<Replayer>>)> {
    let device = cpal::default_host()
        .default_output_device()
        .with_context(|| t!("audio.no_output"))?;
    let supported = device.default_output_config()?;
    let config = supported.config();
    let replayer = Arc::new(Mutex::new(make(config.sample_rate)));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, replayer.clone()),
        SampleFormat::I16 => build::<i16>(&device, &config, replayer.clone()),
        SampleFormat::I32 => build::<i32>(&device, &config, replayer.clone()),
        other => anyhow::bail!(t!("audio.unsupported_format", format = other)),
    }?;
    stream.play()?;
    Ok((stream, replayer))
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    replayer: Arc<Mutex<Replayer>>,
) -> anyhow::Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    // Preallocated stereo buffer, grown only if the device asks for more than expected.
    let mut stereo = vec![0.0f32; 2 * 8192];
    Ok(device.build_output_stream(
        *config,
        move |out: &mut [T], _| {
            let frames = out.len() / channels;
            if stereo.len() < frames * 2 {
                stereo.resize(frames * 2, 0.0);
            }
            let buf = &mut stereo[..frames * 2];
            match replayer.try_lock() {
                Ok(mut r) => r.process(buf),
                Err(_) => buf.fill(0.0),
            }
            for (frame, lr) in out.chunks_mut(channels).zip(buf.as_chunks::<2>().0) {
                for (c, sample) in frame.iter_mut().enumerate() {
                    let x = match (channels, c) {
                        (1, _) => (lr[0] + lr[1]) / 2.0,
                        (_, 0) => lr[0],
                        (_, 1) => lr[1],
                        _ => 0.0,
                    };
                    *sample = T::from_sample(x);
                }
            }
        },
        |err| eprintln!("{}", t!("audio.error", error = err)),
        None,
    )?)
}
