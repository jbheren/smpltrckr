//! smpltrckr command line. « À l'abordage ! »

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use rust_i18n::t;
use smpltrckr::replayer::{Mixer, Replayer};
use smpltrckr::{audio, format, lang, mcp, monitor, render, song, tone, tui, wav};

rust_i18n::i18n!("locales", fallback = "en");

#[derive(Parser)]
#[command(
    version,
    about = "A ProTracker-style text-mode tracker, playable by an agent (MCP)"
)]
struct Cli {
    /// Interface language: en, fr or ja (default: SMPLTRCKR_LANG, then the system locale).
    #[arg(long, global = true)]
    lang: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Opens the text-mode editor ("?" for help). The file is created on first save.
    #[command(alias = "ui")]
    Edit {
        file: Option<PathBuf>,
        /// Keyboard layout: qwerty, azerty or qwertz (detected by default).
        #[arg(long, alias = "clavier")]
        keyboard: Option<String>,
        /// Colours: omarchy (the active Omarchy theme, the default when there is one) or classic.
        #[arg(long)]
        theme: Option<String>,
    },
    /// Plays a test sound and measures audio latency and dropouts.
    Tone {
        /// Playback duration in seconds.
        #[arg(short, long, default_value_t = 5)]
        seconds: u64,
    },
    /// Runs the MCP server on stdio (to plug into Claude Code). Speaks English.
    Mcp,
    /// Prints a module as text: header, order list, samples and patterns.
    Dump {
        file: PathBuf,
        /// Patterns to print (all by default).
        #[arg(short, long)]
        pattern: Vec<usize>,
    },
    /// Loads then saves modules in memory and checks they are identical down to the byte.
    Roundtrip {
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Plays a module on the audio output, until its end.
    Play {
        file: PathBuf,
        #[command(flatten)]
        mix: MixArgs,
    },
    /// Renders a module to a 16-bit WAV (stereo, or one mono track per voice with --stems).
    Render {
        file: PathBuf,
        /// Output file (with --stems: a prefix, completed by "-voice1.wav"…).
        #[arg(short, long)]
        output: PathBuf,
        /// Sample rate.
        #[arg(long, default_value_t = 48000)]
        rate: u32,
        /// One mono track per voice, unmixed.
        #[arg(long)]
        stems: bool,
        /// Maximum duration, for songs that never end.
        #[arg(long, default_value_t = 1200.0)]
        max_seconds: f64,
        #[command(flatten)]
        mix: MixArgs,
    },
}

/// Per-voice mix settings (voices numbered from 1).
#[derive(clap::Args)]
struct MixArgs {
    /// Muted voices, e.g. --mute 2,4.
    #[arg(long, value_delimiter = ',')]
    mute: Vec<usize>,
    /// Solo voices, e.g. --solo 1.
    #[arg(long, value_delimiter = ',')]
    solo: Vec<usize>,
    /// Volume of a voice, 0 to 1, e.g. --volume 3=0.5.
    #[arg(long, value_parser = parse_volume)]
    volume: Vec<(usize, f32)>,
    /// Stereo separation, from 0 (mono) to 1 (Amiga).
    #[arg(long, default_value_t = 0.5)]
    separation: f32,
}

fn parse_volume(text: &str) -> Result<(usize, f32), String> {
    let (voice, volume) = text
        .split_once('=')
        .ok_or_else(|| t!("cli.volume_syntax").into_owned())?;
    let voice = voice
        .parse()
        .map_err(|_| t!("cli.bad_voice", voice = voice).into_owned())?;
    let volume: f32 = volume
        .parse()
        .map_err(|_| t!("cli.bad_volume", volume = volume).into_owned())?;
    Ok((voice, volume.clamp(0.0, 1.0)))
}

impl MixArgs {
    fn apply(&self, mixer: &mut Mixer) -> anyhow::Result<()> {
        let channels = mixer.mute.len();
        let check = |v: usize| {
            anyhow::ensure!(
                (1..=channels).contains(&v),
                t!("cli.no_voice", voice = v, count = channels)
            );
            Ok(v - 1)
        };
        for &v in &self.mute {
            mixer.mute[check(v)?] = true;
        }
        for &v in &self.solo {
            mixer.solo[check(v)?] = true;
        }
        for &(v, volume) in &self.volume {
            mixer.volume[check(v)?] = volume;
        }
        mixer.separation = self.separation.clamp(0.0, 1.0);
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let language = match &cli.lang {
        Some(l) => lang::supported(l)
            .ok_or_else(|| anyhow::anyhow!("unknown language {l:?}: en, fr or ja"))?,
        None => lang::detect(),
    };
    lang::set(language);
    match cli.command {
        Command::Edit {
            file,
            keyboard,
            theme,
        } => tui::run(file, keyboard, theme),
        Command::Tone { seconds } => tone::run(seconds),
        Command::Mcp => mcp::run(),
        Command::Dump { file, pattern } => dump(&file, pattern),
        Command::Roundtrip { files } => roundtrip(&files),
        Command::Play { file, mix } => play(&file, &mix),
        Command::Render {
            file,
            output,
            rate,
            stems,
            max_seconds,
            mix,
        } => render_file(&file, &output, rate, stems, max_seconds, &mix),
    }
}

fn load(file: &Path) -> anyhow::Result<(Vec<u8>, song::Song)> {
    let data = std::fs::read(file).with_context(|| t!("cli.read_failed", path = file.display()))?;
    let song = format::protracker::read(&data).with_context(|| format!("{}", file.display()))?;
    Ok((data, song))
}

fn dump(file: &Path, patterns: Vec<usize>) -> anyhow::Result<()> {
    let (_, song) = load(file)?;
    let patterns = if patterns.is_empty() {
        (0..song.patterns.len()).collect()
    } else {
        patterns
    };
    if let Some(p) = patterns.iter().find(|&&p| p >= song.patterns.len()) {
        anyhow::bail!(t!(
            "editor.no_such_pattern",
            pattern = p,
            count = song.patterns.len()
        ));
    }
    print!("{}", format::text::song_to_text(&song, patterns));
    Ok(())
}

fn roundtrip(files: &[PathBuf]) -> anyhow::Result<()> {
    let mut failures = 0;
    for file in files {
        let result = load(file).and_then(|(original, song)| {
            let written = format::protracker::write(&song);
            match original.iter().zip(&written).position(|(a, b)| a != b) {
                None if original.len() == written.len() => Ok(()),
                None => anyhow::bail!(t!(
                    "cli.size_differs",
                    written = written.len(),
                    original = original.len()
                )),
                Some(offset) => anyhow::bail!(t!("cli.first_difference", offset = offset)),
            }
        });
        match result {
            Ok(()) => println!("{}", t!("cli.roundtrip_ok", path = file.display())),
            Err(e) => {
                failures += 1;
                println!(
                    "{}",
                    t!(
                        "cli.roundtrip_failed",
                        path = file.display(),
                        error = format!("{e:#}")
                    )
                );
            }
        }
    }
    println!(
        "\n{}",
        t!(
            "cli.roundtrip_summary",
            files = files.len(),
            failures = failures
        )
    );
    anyhow::ensure!(
        failures == 0,
        t!("cli.roundtrip_failures", failures = failures)
    );
    Ok(())
}

fn play(file: &Path, mix: &MixArgs) -> anyhow::Result<()> {
    let (_, song) = load(file)?;
    let title = song.display_title();
    let song = Arc::new(song);
    let monitor = monitor::Monitor::new(song.channels);
    let mut replayer = Replayer::new(song.clone(), 48000);
    mix.apply(&mut replayer.mixer)?;
    let mixer = replayer.mixer.clone();
    let (_stream, replayer) = audio::start(|rate| {
        let mut r = Replayer::new(song.clone(), rate);
        r.mixer = mixer;
        r.set_monitor(monitor.clone());
        r
    })?;

    println!(
        "{}",
        t!("cli.playing", title = title, voices = song.channels)
    );
    while !replayer.lock().unwrap().ended() {
        let (position, row) = monitor.position();
        let meters: String = (0..song.channels)
            .map(|v| {
                let bars = (monitor.level(Some(v)) * 8.0).round() as usize;
                format!("{:<8}", "█".repeat(bars.min(8)))
            })
            .collect::<Vec<_>>()
            .join("│");
        let where_ = t!(
            "cli.position",
            position = format!("{position:03}"),
            row = format!("{row:02}")
        );
        print!("\r{where_}  │{meters}│");
        std::io::Write::flush(&mut std::io::stdout())?;
        std::thread::sleep(Duration::from_millis(50));
    }
    println!();
    Ok(())
}

fn render_file(
    file: &Path,
    output: &Path,
    rate: u32,
    stems: bool,
    max_seconds: f64,
    mix: &MixArgs,
) -> anyhow::Result<()> {
    let (_, song) = load(file)?;
    let channels = song.channels;
    let mut replayer = Replayer::new(Arc::new(song), rate);
    mix.apply(&mut replayer.mixer)?;

    let frames = if stems {
        let tracks = render::voices(&mut replayer, channels, rate, max_seconds);
        let stem = output.with_extension("");
        for (v, track) in tracks.iter().enumerate() {
            let path = PathBuf::from(format!("{}-voice{}.wav", stem.display(), v + 1));
            wav::write(&path, track, 1, rate)?;
            println!("{}", path.display());
        }
        tracks[0].len()
    } else {
        let out = render::stereo(&mut replayer, rate, max_seconds);
        wav::write(output, &out, 2, rate)?;
        println!("{}", output.display());
        out.len() / 2
    };
    let seconds = frames as f64 / rate as f64;
    let duration = format!("{}:{:05.2}", (seconds / 60.0) as u32, seconds % 60.0);
    if replayer.ended() {
        println!("{}", t!("cli.duration", duration = duration));
    } else {
        println!("{}", t!("cli.duration_capped", duration = duration));
    }
    Ok(())
}
