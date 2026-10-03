//! Text-mode keyboard interface. « Larguez les amarres ! »

mod app;
mod dialog;
mod effects;
mod keys;
mod view;

use std::path::PathBuf;
use std::time::Duration;

use ratatui::crossterm::event::{self, Event};
use rust_i18n::t;

use crate::format::protracker;
use crate::session::Job;
use crate::song::Song;
use app::{App, Audio};

/// Opens the editor on a file (created on first save if it does not exist yet).
pub fn run(file: Option<PathBuf>, layout: Option<String>) -> anyhow::Result<()> {
    let song = match &file {
        Some(path) if path.exists() => protracker::read(&std::fs::read(path)?)?,
        _ => Song::new(""),
    };
    let (audio, warning) = match Audio::start(&song) {
        Ok(audio) => (audio, None),
        Err(e) => (
            Audio::silent(&song),
            Some(t!("status.no_audio", error = format!("{e:#}")).into_owned()),
        ),
    };
    let mut app = App::new(song, file, audio);
    app.layout = match layout {
        Some(name) => keys::Layout::by_name(&name).ok_or_else(|| {
            anyhow::anyhow!(t!("status.unknown_layout", layout = format!("{name:?}")))
        })?,
        None => keys::detect_layout(),
    };
    app.status = t!("status.welcome", layout = app.layout.name).into_owned();
    if let Some(warning) = warning {
        app.status = warning;
    }

    // Live session: agents connecting through `smpltrckr mcp` send jobs to this loop.
    let (jobs, inbox) = std::sync::mpsc::channel::<Job>();
    let live = match crate::mcp::serve_live(jobs) {
        Ok(server) => {
            app.agents = Some(server.agents.clone());
            Some(server)
        }
        Err(e) => {
            app.status = t!("status.no_live", error = format!("{e:#}")).into_owned();
            None
        }
    };

    let mut terminal = ratatui::init();
    let result = (|| -> anyhow::Result<()> {
        while !app.quit {
            while let Ok(job) = inbox.try_recv() {
                app.run_job(job);
            }
            app.tick();
            terminal.draw(|f| view::draw(f, &app))?;
            if event::poll(Duration::from_millis(16))?
                && let Event::Key(key) = event::read()?
            {
                app.handle_key(key);
            }
        }
        Ok(())
    })();
    ratatui::restore();
    drop(live);
    result
}
