//! Interface en ligne de commande de smpltrckr.

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use smpltrckr::{format, mcp, song, tone, ui};

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
    /// Affiche une maquette d'interface Ratatui (q pour quitter).
    Ui,
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
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Ui => ui::run(),
        Command::Tone { seconds } => tone::run(seconds),
        Command::Mcp => mcp::run(),
        Command::Dump { file, pattern } => dump(&file, pattern),
        Command::Roundtrip { files } => roundtrip(&files),
    }
}

fn load(file: &PathBuf) -> anyhow::Result<(Vec<u8>, song::Song)> {
    let data = std::fs::read(file).with_context(|| format!("lecture de {}", file.display()))?;
    let song = format::protracker::read(&data).with_context(|| format!("{}", file.display()))?;
    Ok((data, song))
}

fn dump(file: &PathBuf, patterns: Vec<usize>) -> anyhow::Result<()> {
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
