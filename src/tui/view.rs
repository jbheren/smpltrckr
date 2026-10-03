//! Affichage : en-tête, pattern avec un VU-mètre sous chaque voie, liste d'ordre, samples,
//! master, ligne d'état et dialogues.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use super::app::{App, Field};
use super::dialog::Dialog;
use super::keys::{Focus, HELP};
use crate::format::text::cell_to_text;

const DIM: Color = Color::DarkGray;
/// Largeur d'une colonne de voie : « C-3 01 A04 » et ses marges.
const VOICE_WIDTH: u16 = 13;

pub fn draw(f: &mut Frame, app: &App) {
    let [header, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(10),
        Constraint::Length(1),
    ])
    .areas(f.area());
    let [pattern, side] =
        Layout::horizontal([Constraint::Min(20), Constraint::Length(32)]).areas(body);
    let [orders, samples, master] = Layout::vertical([
        Constraint::Length(10),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .areas(side);

    draw_header(f, app, header);
    draw_pattern(f, app, pattern);
    draw_orders(f, app, orders);
    draw_samples(f, app, samples);
    draw_master(f, app, master);
    f.render_widget(Line::from(app.status.as_str()).fg(Color::Gray), status);

    if let Some(dialog) = &app.dialog {
        draw_dialog(f, dialog);
    }
}

fn panel(title: String, focused: bool) -> Block<'static> {
    let style = if focused {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new().fg(DIM)
    };
    Block::bordered().title(title).border_style(style)
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let song = app.song();
    let file = app
        .path
        .as_ref()
        .and_then(|p| p.file_name())
        .map_or("(sans fichier)".into(), |n| {
            n.to_string_lossy().into_owned()
        });
    let mode = if app.edit_mode {
        " ÉDITION ".bold().fg(Color::White).bg(Color::Red)
    } else {
        " écoute ".fg(DIM)
    };
    let play = if app.running {
        " ▶ ".fg(Color::Black).bg(Color::Green)
    } else {
        " ■ ".fg(DIM)
    };
    let sample = &song.samples[app.sample - 1];
    let line = Line::from(vec![
        " smpltrckr ".bold().fg(Color::Black).bg(Color::Cyan),
        format!(" {} ", song.display_title()).bold(),
        format!("{file}{} ", if app.dirty { " *" } else { "" }).fg(DIM),
        play,
        mode,
        format!(
            "  oct {}  sample {:02} {}  ",
            app.octave,
            app.sample,
            sample.display_name()
        )
        .into(),
        format!("vit {} · {} BPM  ", app.tempo.0, app.tempo.1).fg(Color::Yellow),
        format!("pos {:02}/{:02}", app.position, song.order_list().len() - 1).fg(DIM),
    ]);
    f.render_widget(line, area);
}

fn draw_pattern(f: &mut Frame, app: &App, area: Rect) {
    let p = app.pattern_index();
    let border = if app.edit_mode {
        Color::Red
    } else if app.focus == Focus::Pattern {
        Color::Cyan
    } else {
        DIM
    };
    let block = Block::bordered()
        .title(format!(" pattern {p:02} "))
        .border_style(Style::new().fg(border));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [voices_header, grid, meters] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(2),
    ])
    .areas(inner);
    let channels = app.channels();
    let mixer = &app.editor.mixer;
    let audible = |v: usize| mixer.audible(v);

    // En-tête des voies.
    let mut head = vec![Span::raw("   ")];
    for v in 0..channels {
        let style = if !audible(v) {
            Style::new().fg(DIM)
        } else {
            Style::new().fg(Color::Cyan)
        };
        head.push(Span::styled(format!("│ voie {:<6}", v + 1), style));
    }
    f.render_widget(Line::from(head), voices_header);

    // Grille : la ligne du curseur reste au milieu, comme dans ProTracker.
    let pattern = &app.song().patterns[p];
    let height = grid.height as usize;
    let lines: Vec<Line> = (0..height)
        .map(|y| {
            let offset = y as isize - height as isize / 2;
            let row = app.row as isize + offset;
            if !(0..64).contains(&row) {
                return Line::default();
            }
            let row = row as usize;
            let number_style = if row.is_multiple_of(4) {
                Style::new().fg(Color::Gray)
            } else {
                Style::new().fg(DIM)
            };
            let mut spans = vec![Span::styled(format!("{row:02} "), number_style)];
            for v in 0..channels {
                spans.push("│ ".fg(DIM));
                let text = cell_to_text(&pattern.rows[row][v]);
                let cursor_field = (offset == 0 && v == app.voice && app.focus == Focus::Pattern)
                    .then_some(app.field);
                spans.extend(cell_spans(&text, cursor_field, audible(v)));
                spans.push(" ".into());
            }
            let line = Line::from(spans);
            match (offset, app.running) {
                (0, true) => line.style(Style::new().bg(Color::Blue)),
                (0, false) => line.style(Style::new().bg(Color::Rgb(40, 40, 60))),
                _ => line,
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), grid);

    // VU-mètre et état sous chaque voie.
    let mut bars = vec![Span::raw("   ")];
    let mut states = vec![Span::raw("   ")];
    for v in 0..channels {
        let level = app.audio.monitor.level(Some(v));
        bars.push("│ ".fg(DIM));
        bars.extend(meter(level, (VOICE_WIDTH - 3) as usize, audible(v)));
        bars.push(" ".into());
        let state = match (mixer.mute[v], mixer.solo[v]) {
            (_, true) => " SOLO ".fg(Color::Black).bg(Color::Yellow),
            (true, _) => " coupée ".fg(Color::White).bg(Color::Red),
            _ => "".into(),
        };
        states.push("│ ".fg(DIM));
        let volume = format!("{:>3} % ", (mixer.volume[v] * 100.0).round() as u32);
        let used = volume.chars().count() + state.content.chars().count();
        states.push(volume.fg(if audible(v) { Color::Gray } else { DIM }));
        states.push(state);
        states.push(
            " ".repeat((VOICE_WIDTH as usize - 2).saturating_sub(used))
                .into(),
        );
    }
    f.render_widget(
        Paragraph::new(vec![Line::from(bars), Line::from(states)]),
        meters,
    );
}

/// Les cinq morceaux d'une cellule, avec le champ du curseur en inverse.
fn cell_spans(text: &str, cursor: Option<Field>, audible: bool) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let part = |range: std::ops::Range<usize>| chars[range].iter().collect::<String>();
    let parts = [
        (part(0..3), Color::White, Field::Note),
        (part(4..5), Color::Yellow, Field::SampleTens),
        (part(5..6), Color::Yellow, Field::SampleUnits),
        (part(7..8), Color::Magenta, Field::Effect),
        (part(8..9), Color::Magenta, Field::ParamHigh),
        (part(9..10), Color::Magenta, Field::ParamLow),
    ];
    let mut spans = Vec::new();
    for (i, (s, color, field)) in parts.into_iter().enumerate() {
        if i == 1 || i == 3 {
            spans.push(" ".into());
        }
        let empty = s.chars().all(|c| c == '.');
        let mut style = Style::new().fg(if empty || !audible { DIM } else { color });
        if cursor == Some(field) {
            style = style.add_modifier(Modifier::REVERSED);
        }
        spans.push(Span::styled(s, style));
    }
    spans
}

/// Barre de niveau horizontale, au huitième de caractère près.
fn meter(level: f32, width: usize, active: bool) -> Vec<Span<'static>> {
    const EIGHTHS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
    let filled = (level.clamp(0.0, 1.0) * width as f32 * 8.0).round() as usize;
    (0..width)
        .map(|i| {
            let color = match (active, i * 4 / width.max(1)) {
                (false, _) => DIM,
                (_, 0 | 1) => Color::Green,
                (_, 2) => Color::Yellow,
                _ => Color::Red,
            };
            let c = match filled.saturating_sub(i * 8) {
                0 => '·',
                n if n >= 8 => '█',
                n => EIGHTHS[n],
            };
            Span::styled(
                c.to_string(),
                Style::new().fg(if c == '·' { DIM } else { color }),
            )
        })
        .collect()
}

fn draw_orders(f: &mut Frame, app: &App, area: Rect) {
    let block = panel(" ordre (F6) ".into(), app.focus == Focus::Orders);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let orders = app.song().order_list();
    let first = scroll(app.position, orders.len(), inner.height as usize);
    let lines: Vec<Line> = orders
        .iter()
        .enumerate()
        .skip(first)
        .take(inner.height as usize)
        .map(|(i, &p)| {
            let line = Line::from(format!(" {i:02}  pattern {p:02}"));
            if i == app.position {
                line.style(Style::new().bg(Color::Rgb(40, 40, 60)).bold())
            } else {
                line
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_samples(f: &mut Frame, app: &App, area: Rect) {
    let block = panel(" samples (F7) ".into(), app.focus == Focus::Samples);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let samples = &app.song().samples;
    let first = scroll(app.sample - 1, samples.len(), inner.height as usize);
    let lines: Vec<Line> = samples
        .iter()
        .enumerate()
        .skip(first)
        .take(inner.height as usize)
        .map(|(i, s)| {
            let empty = s.data.is_empty();
            let text = format!("{:02} {:<16.16} {:>2}", i + 1, s.display_name(), s.volume);
            let line = Line::from(text).fg(if empty { DIM } else { Color::Gray });
            if i + 1 == app.sample {
                line.style(Style::new().bg(Color::Rgb(40, 40, 60)).bold())
            } else {
                line
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_master(f: &mut Frame, app: &App, area: Rect) {
    let block = panel(" master ".into(), false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let level = app.audio.monitor.level(None);
    f.render_widget(Line::from(meter(level, inner.width as usize, true)), inner);
}

/// Premier élément affiché pour garder `selected` visible dans une liste de `len` éléments.
fn scroll(selected: usize, len: usize, height: usize) -> usize {
    if len <= height {
        0
    } else {
        selected.saturating_sub(height / 2).min(len - height)
    }
}

fn draw_dialog(f: &mut Frame, dialog: &Dialog) {
    let area = f.area();
    let (title, lines, width, height): (String, Vec<Line>, u16, u16) =
        match dialog {
            Dialog::Help => {
                let lines = HELP
                    .iter()
                    .map(|(k, a)| Line::from(vec![format!("{k:<32}").fg(Color::Cyan), (*a).into()]))
                    .collect();
                (
                    " aide — Échap pour fermer ".into(),
                    lines,
                    96,
                    HELP.len() as u16 + 2,
                )
            }
            Dialog::Prompt(p) => {
                let lines = vec![Line::from(vec![p.text.clone().into(), "█".fg(Color::Cyan)])];
                (format!(" {} — Entrée / Échap ", p.label), lines, 70, 3)
            }
            Dialog::Choice(c) => {
                let lines = c
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        let line = Line::from(format!(" {item}"));
                        if i == c.selected {
                            line.style(Style::new().bg(Color::Blue))
                        } else {
                            line
                        }
                    })
                    .collect();
                (
                    format!(" {} ", c.label),
                    lines,
                    40,
                    c.items.len() as u16 + 2,
                )
            }
            Dialog::Browser(b) => {
                let visible = (area.height.saturating_sub(8)) as usize;
                let first = scroll(b.selected, b.entries.len(), visible);
                let mut lines = vec![Line::from(b.dir.display().to_string()).fg(DIM)];
                lines.extend(b.entries.iter().enumerate().skip(first).take(visible).map(
                    |(i, e)| {
                        let text = if e.is_dir {
                            format!(" {}/", e.name)
                        } else {
                            format!(" {}", e.name)
                        };
                        let line =
                            Line::from(text).fg(if e.is_dir { Color::Cyan } else { Color::Gray });
                        if i == b.selected {
                            line.style(Style::new().bg(Color::Blue))
                        } else {
                            line
                        }
                    },
                ));
                let height = lines.len() as u16 + 2;
                (
                    format!(" {} — Entrée, ← dossier parent, Échap ", b.title),
                    lines,
                    70,
                    height,
                )
            }
        };
    let width = width.min(area.width);
    let height = height.min(area.height);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(title)
                .border_style(Style::new().fg(Color::Cyan)),
        ),
        rect,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use ratatui::{Terminal, buffer::Buffer};

    use super::*;
    use crate::song::Song;
    use crate::tui::app::Audio;

    fn screen(buffer: &Buffer) -> String {
        let width = buffer.area.width as usize;
        let symbols: Vec<&str> = buffer.content().iter().map(|c| c.symbol()).collect();
        symbols
            .chunks(width)
            .map(|l| l.concat())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn render(app: &App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(110, 36)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        screen(terminal.backend().buffer())
    }

    #[test]
    fn shows_pattern_meters_and_panels() {
        let song = Song::new("écran");
        let mut app = App::new(song.clone(), None, Audio::silent(&song));
        app.handle_key(KeyEvent::from(KeyCode::Char(' ')));
        app.handle_key(KeyEvent::from(KeyCode::Char('z')));
        app.editor.mixer.mute[2] = true;
        let s = render(&app);
        for expected in [
            "smpltrckr",
            "écran",
            "ÉDITION",
            "pattern 00",
            "voie 4",
            "C-2 01 ...",
            "coupée",
            "ordre (F6)",
            "samples (F7)",
            "master",
        ] {
            assert!(s.contains(expected), "absent : {expected}\n{s}");
        }
    }

    #[test]
    fn shows_help_dialog() {
        let song = Song::new("");
        let mut app = App::new(song.clone(), None, Audio::silent(&song));
        app.handle_key(KeyEvent::from(KeyCode::Char('?')));
        let s = render(&app);
        assert!(s.contains("aide") && s.contains("Ctrl+Q"), "{s}");
    }

    /// Capture texte de l'écran sur un vrai morceau : `cargo test snapshot -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn snapshot() {
        let path = std::env::var("SNAPSHOT_MOD")
            .unwrap_or("sessions/2026-10-03-chiptune-la-mineur/morceau.mod".into());
        let song = crate::format::protracker::read(&std::fs::read(&path).unwrap()).unwrap();
        let mut app = App::new(song.clone(), Some(path.into()), Audio::silent(&song));
        app.row = 12;
        app.handle_key(KeyEvent::from(KeyCode::Char(' ')));
        println!("{}", render(&app));
    }
}
