//! Interface clavier en mode texte.

mod app;
mod dialog;
mod keys;
mod view;

use std::path::PathBuf;
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyEventKind};

use crate::format::protracker;
use crate::song::Song;
use app::{App, Audio};

/// Ouvre l'éditeur sur un fichier (créé à l'enregistrement s'il n'existe pas encore).
pub fn run(file: Option<PathBuf>) -> anyhow::Result<()> {
    let song = match &file {
        Some(path) if path.exists() => protracker::read(&std::fs::read(path)?)?,
        _ => Song::new(""),
    };
    let (audio, warning) = match Audio::start(&song) {
        Ok(audio) => (audio, None),
        Err(e) => (
            Audio::silent(&song),
            Some(format!("pas de sortie audio : {e:#}")),
        ),
    };
    let mut app = App::new(song, file, audio);
    if let Some(warning) = warning {
        app.status = warning;
    }

    let mut terminal = ratatui::init();
    let result = (|| -> anyhow::Result<()> {
        while !app.quit {
            app.tick();
            terminal.draw(|f| view::draw(f, &app))?;
            if event::poll(Duration::from_millis(16))?
                && let Event::Key(key) = event::read()?
                && key.kind != KeyEventKind::Release
            {
                app.handle_key(key);
            }
        }
        Ok(())
    })();
    ratatui::restore();
    result
}
