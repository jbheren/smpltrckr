//! Brique « interface » : maquette Ratatui d'un écran de tracker, sans son ni données réelles.
//!
//! Vérifie le rendu (couleurs, caractères de blocs) et la boucle d'événements clavier.

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::{DefaultTerminal, Frame};

const ROWS: usize = 64;
const VOICES: usize = 4;
const ROW_TIME: Duration = Duration::from_millis(120);

struct App {
    row: usize,
    playing: bool,
    last_step: Instant,
    /// Niveau de chaque voie (0.0 à 1.0), pour les VU-mètres.
    levels: [f32; VOICES],
}

pub fn run() -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
    let mut app = App {
        row: 0,
        playing: true,
        last_step: Instant::now(),
        levels: [0.0; VOICES],
    };
    loop {
        terminal.draw(|f| draw(f, &app))?;
        if event::poll(Duration::from_millis(16))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char(' ') => app.playing = !app.playing,
                KeyCode::Up => app.row = (app.row + ROWS - 1) % ROWS,
                KeyCode::Down => app.row = (app.row + 1) % ROWS,
                _ => {}
            }
        }
        for level in &mut app.levels {
            *level *= 0.9;
        }
        if app.playing && app.last_step.elapsed() >= ROW_TIME {
            app.last_step = Instant::now();
            app.row = (app.row + 1) % ROWS;
            for v in 0..VOICES {
                if cell(app.row, v).is_some() {
                    app.levels[v] = 1.0;
                }
            }
        }
    }
}

/// Contenu factice d'une cellule : (note, sample, effet).
fn cell(row: usize, voice: usize) -> Option<(&'static str, u8, &'static str)> {
    match voice {
        0 if row.is_multiple_of(4) => Some((["A-1", "A-1", "F-1", "G-1"][row / 16 % 4], 1, "...")),
        1 if row % 8 == 4 => Some(("C-2", 2, "C30")),
        2 if row.is_multiple_of(2) => Some((["A-2", "C-3", "E-3", "A-3"][row / 2 % 4], 3, "A02")),
        3 if row.is_multiple_of(16) => Some(("E-3", 4, "...")),
        _ => None,
    }
}

fn draw(f: &mut Frame, app: &App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .areas(f.area());
    let [pattern, side] =
        Layout::horizontal([Constraint::Min(60), Constraint::Length(24)]).areas(body);

    let title = Line::from(vec![
        " smpltrckr ".bold().fg(Color::Black).bg(Color::Cyan),
        "  phase 0 — maquette d'interface".fg(Color::DarkGray),
    ]);
    f.render_widget(Paragraph::new(title).block(Block::bordered()), header);

    draw_pattern(f, app, pattern);
    draw_vu(f, app, side);

    let help = if app.playing {
        "espace pause · ↑↓ ligne · q quitter"
    } else {
        "espace lecture · ↑↓ ligne · q quitter"
    };
    f.render_widget(Line::from(help).fg(Color::DarkGray), footer);
}

fn draw_pattern(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered().title(" pattern 00 ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    // La ligne jouée reste au milieu de l'écran, comme dans ProTracker.
    let height = inner.height as usize;
    let lines: Vec<Line> = (0..height)
        .map(|y| {
            let offset = y as isize - height as isize / 2;
            let row = app.row as isize + offset;
            if !(0..ROWS as isize).contains(&row) {
                return Line::default();
            }
            let row = row as usize;
            let mut spans = vec![Span::styled(
                format!("{row:02} "),
                Style::new().fg(Color::DarkGray),
            )];
            for v in 0..VOICES {
                spans.push("│ ".fg(Color::DarkGray));
                match cell(row, v) {
                    Some((note, sample, fx)) => {
                        spans.push(note.fg(Color::White));
                        spans.push(format!(" {sample:02} ").fg(Color::Yellow));
                        spans.push(format!("{fx} ").fg(Color::Magenta));
                    }
                    None => spans.push("... .. ... ".fg(Color::DarkGray)),
                }
            }
            let line = Line::from(spans);
            if offset == 0 {
                line.style(Style::new().bg(Color::Blue).add_modifier(Modifier::BOLD))
            } else {
                line
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_vu(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered().title(" voies ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let width = inner.width.saturating_sub(4) as usize;
    let lines: Vec<Line> = app
        .levels
        .iter()
        .enumerate()
        .flat_map(|(v, &level)| {
            let filled = (level * width as f32).round() as usize;
            let bar: Vec<Span> = (0..width)
                .map(|i| {
                    let color = match i * 3 / width.max(1) {
                        0 => Color::Green,
                        1 => Color::Yellow,
                        _ => Color::Red,
                    };
                    if i < filled {
                        Span::styled("█", Style::new().fg(color))
                    } else {
                        "░".fg(Color::DarkGray)
                    }
                })
                .collect();
            [
                Line::from([vec![format!("{} ", v + 1).fg(Color::Cyan)], bar].concat()),
                Line::default(),
            ]
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn draws_pattern_and_meters() {
        let app = App {
            row: 0,
            playing: false,
            last_step: Instant::now(),
            levels: [1.0, 0.0, 0.5, 0.0],
        };
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        for expected in [
            "smpltrckr",
            "pattern 00",
            "voies",
            "00 │ A-1 01 ...",
            "espace lecture",
            "█",
        ] {
            assert!(screen.contains(expected), "absent de l'écran : {expected}");
        }
    }
}
