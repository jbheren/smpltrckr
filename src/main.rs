//! Interface en ligne de commande de smpltrckr.

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};
use std::sync::Arc;
use std::time::Duration;

use smpltrckr::replayer::{Mixer, Replayer};
use smpltrckr::{audio, format, mcp, monitor, render, song, tone, tui, wav};

#[derive(Parser)]
#[command(
    version,
    about = "Tracker texte façon ProTracker, pilotable par un agent (MCP)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Ouvre l'éditeur en mode texte (« ? » pour l'aide). Le fichier est créé à
    /// l'enregistrement s'il n'existe pas.
    #[command(alias = "ui")]
    Edit {
        file: Option<PathBuf>,
        /// Disposition du clavier : qwerty, azerty ou qwertz (détectée par défaut).
        #[arg(long)]
        clavier: Option<String>,
    },
    /// Joue un son de test et mesure latence et décrochages audio.
    Tone {
        /// Durée de lecture en secondes.
        #[arg(short, long, default_value_t = 5)]
        seconds: u64,
    },
    /// Lance le serveur MCP sur stdio (à brancher dans Claude Code).
    Mcp,
    /// Affiche un module en texte : en-tête, ordre, samples et patterns.
    Dump {
        file: PathBuf,
        /// Patterns à afficher (tous par défaut).
        #[arg(short, long)]
        pattern: Vec<usize>,
    },
    /// Charge puis réenregistre des modules en mémoire et vérifie qu'ils sont identiques à l'octet près.
    Roundtrip {
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Joue un module sur la sortie audio, jusqu'à sa fin.
    Play {
        file: PathBuf,
        #[command(flatten)]
        mix: MixArgs,
    },
    /// Rend un module en WAV 16 bits (stéréo, ou une piste mono par voie avec --stems).
    Render {
        file: PathBuf,
        /// Fichier de sortie (avec --stems : préfixe, complété par « -voie1.wav »…).
        #[arg(short, long)]
        output: PathBuf,
        /// Fréquence d'échantillonnage.
        #[arg(long, default_value_t = 48000)]
        rate: u32,
        /// Une piste mono par voie, sans mixage.
        #[arg(long)]
        stems: bool,
        /// Durée maximale, pour les morceaux qui ne finissent jamais.
        #[arg(long, default_value_t = 1200.0)]
        max_seconds: f64,
        #[command(flatten)]
        mix: MixArgs,
    },
}

/// Réglages de mixage par voie (voies numérotées à partir de 1).
#[derive(clap::Args)]
struct MixArgs {
    /// Voies coupées, ex. --mute 2,4.
    #[arg(long, value_delimiter = ',')]
    mute: Vec<usize>,
    /// Voies en solo, ex. --solo 1.
    #[arg(long, value_delimiter = ',')]
    solo: Vec<usize>,
    /// Volume d'une voie, de 0 à 1, ex. --volume 3=0.5.
    #[arg(long, value_parser = parse_volume)]
    volume: Vec<(usize, f32)>,
    /// Séparation stéréo, de 0 (mono) à 1 (Amiga).
    #[arg(long, default_value_t = 0.5)]
    separation: f32,
}

fn parse_volume(text: &str) -> Result<(usize, f32), String> {
    let (voice, volume) = text
        .split_once('=')
        .ok_or("attendu voie=volume, ex. 3=0.5")?;
    let voice = voice
        .parse()
        .map_err(|_| format!("voie invalide {voice:?}"))?;
    let volume: f32 = volume
        .parse()
        .map_err(|_| format!("volume invalide {volume:?}"))?;
    Ok((voice, volume.clamp(0.0, 1.0)))
}

impl MixArgs {
    fn apply(&self, mixer: &mut Mixer) -> anyhow::Result<()> {
        let channels = mixer.mute.len();
        let check = |v: usize| {
            anyhow::ensure!(
                (1..=channels).contains(&v),
                "voie {v} inexistante (1 à {channels})"
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
    match Cli::parse().command {
        Command::Edit { file, clavier } => tui::run(file, clavier),
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
    let data = std::fs::read(file).with_context(|| format!("lecture de {}", file.display()))?;
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
        anyhow::bail!(
            "pattern {p} inexistant (le module en a {})",
            song.patterns.len()
        );
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
                None => anyhow::bail!("taille {} au lieu de {}", written.len(), original.len()),
                Some(offset) => anyhow::bail!("premier écart à l'octet {offset}"),
            }
        });
        match result {
            Ok(()) => println!("ok      {}", file.display()),
            Err(e) => {
                failures += 1;
                println!("ÉCHEC   {} : {e:#}", file.display());
            }
        }
    }
    println!("\n{} fichier(s), {} échec(s)", files.len(), failures);
    anyhow::ensure!(failures == 0, "{failures} fichier(s) en échec");
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
        "lecture de « {title} » ({} voies) — Ctrl-C pour arrêter",
        song.channels
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
        print!("\rposition {position:03} ligne {row:02}  │{meters}│");
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
            let path = PathBuf::from(format!("{}-voie{}.wav", stem.display(), v + 1));
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
    let note = if replayer.ended() {
        ""
    } else {
        " (durée maximale atteinte)"
    };
    println!(
        "durée : {}:{:05.2}{note}",
        (seconds / 60.0) as u32,
        seconds % 60.0
    );
    Ok(())
}
