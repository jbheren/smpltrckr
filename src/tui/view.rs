//! Affichage : en-tête, pattern avec un VU-mètre sous chaque voie, liste d'ordre, samples,
//! master, ligne d'état et dialogues.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use super::app::{App, Field};
use super::dialog::Dialog;
use super::keys::{FOCUS_HINTS, Focus, HELP};
use crate::format::text::cell_to_text;
use crate::monitor::{Monitor, SCOPE_LEN};

const DIM: Color = Color::DarkGray;
/// Largeur minimale d'une colonne de voie : « │ C-3 01 A04 » et une marge.
const MIN_VOICE_WIDTH: u16 = 13;

pub fn draw(f: &mut Frame, app: &App) {
    let [header, body, hints, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(10),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(f.area());
    let [pattern, side] =
        Layout::horizontal([Constraint::Min(20), Constraint::Length(32)]).areas(body);
    let [orders, samples, master] = Layout::vertical([
        Constraint::Length(10),
        Constraint::Min(5),
        Constraint::Length(SCOPE_HEIGHT + 3),
    ])
    .areas(side);

    draw_header(f, app, header);
    draw_pattern(f, app, pattern);
    draw_orders(f, app, orders);
    draw_samples(f, app, samples);
    draw_master(f, app, master);
    f.render_widget(hint_line(app), hints);
    f.render_widget(Line::from(app.status.as_str()).fg(Color::Gray), status);

    if let Some(dialog) = &app.dialog {
        draw_dialog(f, dialog);
    }
}

/// Touches utiles dans la zone active, toujours visibles en bas de l'écran.
fn hint_line(app: &App) -> Line<'static> {
    let hints = match app.focus {
        Focus::Pattern if app.edit_mode => FOCUS_HINTS[0].1,
        Focus::Pattern => FOCUS_HINTS[1].1,
        Focus::Orders => FOCUS_HINTS[2].1,
        Focus::Samples => FOCUS_HINTS[3].1,
    };
    let mut spans = Vec::new();
    for (i, part) in hints.split(" · ").enumerate() {
        if i > 0 {
            spans.push("  ".into());
        }
        let (key, what) = part.split_once(' ').unwrap_or((part, ""));
        spans.push(key.to_string().fg(Color::Cyan));
        spans.push(format!(" {what}").fg(DIM));
    }
    Line::from(spans)
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
        format!("{}  ", app.layout.name).fg(DIM),
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

    let [voices_header, grid, scopes, states] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(SCOPE_HEIGHT),
        Constraint::Length(1),
    ])
    .areas(inner);
    let channels = app.channels();
    let mixer = &app.editor.mixer;
    let audible = |v: usize| mixer.audible(v);
    // Les colonnes des voies se partagent la largeur disponible.
    let width = ((inner.width.saturating_sub(3)) / channels as u16).max(MIN_VOICE_WIDTH) as usize;
    let pad = |used: usize| " ".repeat(width.saturating_sub(used));

    // En-tête des voies.
    let mut head = vec![Span::raw("   ")];
    for v in 0..channels {
        let style = Style::new().fg(if audible(v) { Color::Cyan } else { DIM });
        let label = format!("│ voie {}", v + 1);
        let used = label.chars().count();
        head.push(Span::styled(label + &pad(used), style));
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
            let number_style = Style::new().fg(if row.is_multiple_of(4) {
                Color::Gray
            } else {
                DIM
            });
            let mut spans = vec![Span::styled(format!("{row:02} "), number_style)];
            for v in 0..channels {
                spans.push("│ ".fg(DIM));
                let text = cell_to_text(&pattern.rows[row][v]);
                let cursor_field = (offset == 0 && v == app.voice && app.focus == Focus::Pattern)
                    .then_some(app.field);
                spans.extend(cell_spans(&text, cursor_field, audible(v)));
                spans.push(pad(12).into());
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

    // Oscilloscope et état sous chaque voie.
    let mut state_line = vec![Span::raw("   ")];
    for v in 0..channels {
        let x = scopes.x + 3 + (v * width) as u16;
        if x >= scopes.right() {
            break;
        }
        let separator = Rect {
            x,
            width: 1,
            ..scopes
        };
        f.render_widget(
            Paragraph::new(vec![Line::from("│").fg(DIM); SCOPE_HEIGHT as usize]),
            separator,
        );
        let scope_area = Rect {
            x: x + 1,
            width: (width as u16 - 2).min(scopes.right() - x - 1),
            ..scopes
        };
        let color = if audible(v) { Color::Green } else { DIM };
        let wave = triggered_scope(&app.audio.monitor, Some(v));
        f.render_widget(
            Paragraph::new(scope(
                &wave,
                scope_area.width as usize,
                SCOPE_HEIGHT as usize,
                VOICE_SCOPE_GAIN,
                color,
            )),
            scope_area,
        );

        let state = match (mixer.mute[v], mixer.solo[v]) {
            (_, true) => " SOLO ".fg(Color::Black).bg(Color::Yellow),
            (true, _) => " coupée ".fg(Color::White).bg(Color::Red),
            _ => "".into(),
        };
        let volume = format!("{:>3} % ", (mixer.volume[v] * 100.0).round() as u32);
        let used = 2 + volume.chars().count() + state.content.chars().count();
        state_line.push("│ ".fg(DIM));
        state_line.push(volume.fg(if audible(v) { Color::Gray } else { DIM }));
        state_line.push(state);
        state_line.push(pad(used).into());
    }
    f.render_widget(Line::from(state_line), states);
}

/// Hauteur des oscilloscopes, en lignes de texte (4 points Braille par ligne).
const SCOPE_HEIGHT: u16 = 4;
/// Agrandissement des formes d'onde : une voie dépasse rarement la moitié de l'échelle, et le
/// master est atténué par le mixage (2 / nombre de voies).
const VOICE_SCOPE_GAIN: f32 = 2.0;
const MASTER_SCOPE_GAIN: f32 = 3.0;

/// Échantillons récents d'une voie (`None` = master), calés sur un passage par zéro montant
/// pour que la forme d'onde reste immobile d'une image à l'autre.
fn triggered_scope(monitor: &Monitor, voice: Option<usize>) -> Vec<f32> {
    let mut all = vec![0.0f32; SCOPE_LEN];
    monitor.scope(voice, &mut all);
    let shown = SCOPE_LEN / 2;
    let start = (1..SCOPE_LEN - shown)
        .find(|&i| all[i - 1] <= 0.0 && all[i] > 0.0)
        .unwrap_or(SCOPE_LEN - shown);
    all[start..start + shown].to_vec()
}

/// Forme d'onde en caractères Braille (2 × 4 points par caractère). Chaque colonne de points
/// couvre plusieurs échantillons : on trace le segment de leur minimum à leur maximum.
fn scope(
    samples: &[f32],
    width: usize,
    height: usize,
    gain: f32,
    color: Color,
) -> Vec<Line<'static>> {
    const BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
    let (dots_w, dots_h) = (width * 2, height * 4);
    if dots_w == 0 || samples.is_empty() {
        return Vec::new();
    }
    let mut cells = vec![0u8; width * height];
    let to_y = |s: f32| {
        ((1.0 - ((s * gain).clamp(-1.0, 1.0) + 1.0) / 2.0) * (dots_h - 1) as f32).round() as usize
    };
    let mut previous: Option<usize> = None;
    for x in 0..dots_w {
        let start = x * samples.len() / dots_w;
        let end = ((x + 1) * samples.len() / dots_w).max(start + 1);
        let bucket = &samples[start..end];
        let (lo, hi) = bucket
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &s| (lo.min(s), hi.max(s)));
        let (mut top, mut bottom) = (to_y(hi), to_y(lo));
        // Relie au dernier point de la colonne précédente : la courbe reste continue.
        if let Some(y) = previous {
            (top, bottom) = (top.min(y), bottom.max(y));
        }
        for y in top..=bottom {
            cells[(y / 4) * width + x / 2] |= BITS[x % 2][y % 4];
        }
        previous = Some(to_y(bucket[bucket.len() - 1]));
    }
    cells
        .chunks(width)
        .map(|row| {
            let text: String = row
                .iter()
                .map(|&b| {
                    if b == 0 {
                        ' '
                    } else {
                        char::from_u32(0x2800 + b as u32).unwrap()
                    }
                })
                .collect();
            Line::from(text).fg(color)
        })
        .collect()
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
    let [wave, bar] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    let samples = triggered_scope(&app.audio.monitor, None);
    f.render_widget(
        Paragraph::new(scope(
            &samples,
            wave.width as usize,
            wave.height as usize,
            MASTER_SCOPE_GAIN,
            Color::Cyan,
        )),
        wave,
    );
    let level = app.audio.monitor.level(None);
    f.render_widget(
        Line::from(meter(level * 2.0, bar.width as usize, true)),
        bar,
    );
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
            "F7 samples",
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
    fn scope_draws_a_continuous_wave() {
        // Un cycle de sinus sur 8 caractères × 2 lignes : chaque colonne de points est allumée.
        let wave: Vec<f32> = (0..256)
            .map(|i| (i as f32 / 256.0 * std::f32::consts::TAU).sin())
            .collect();
        let lines = scope(&wave, 8, 2, 1.0, Color::Green);
        assert_eq!(lines.len(), 2);
        let mut columns = [false; 16];
        for (r, line) in lines.iter().enumerate() {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            for (c, ch) in text.chars().enumerate() {
                let bits = (ch as u32).saturating_sub(0x2800);
                columns[c * 2] |= bits & 0x47 != 0;
                columns[c * 2 + 1] |= bits & 0xB8 != 0;
            }
            assert!(r < 2);
        }
        assert!(columns.iter().all(|&c| c), "{columns:?}");
        // Le haut de l'onde (début du cycle, sinus positif) est sur la première ligne.
        let first: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(first.chars().take(4).any(|c| c != ' '));
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
        {
            // Deux secondes de lecture pour remplir les oscilloscopes.
            let mut r = app.audio.replayer.lock().unwrap();
            r.play(0, false);
            let mut buf = vec![0.0f32; 2 * 48000 * 2 + 2 * 3000];
            r.process(&mut buf);
        }
        app.tick();
        println!("{}", render(&app));
    }
}
