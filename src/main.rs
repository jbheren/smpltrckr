//! smpltrckr — tracker texte façon ProTracker, pilotable au clavier ou par un agent (MCP).
//!
//! Phase 0 : trois briques de validation de la pile, indépendantes les unes des autres.

mod mcp;
mod note;
mod tone;
mod ui;

use clap::{Parser, Subcommand};

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
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Ui => ui::run(),
        Command::Tone { seconds } => tone::run(seconds),
        Command::Mcp => mcp::run(),
    }
}
