//! Drawing: header, pattern with a scope under each voice, order list, samples, master, key
//! hints, status line and dialogs.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use rust_i18n::t;
use unicode_width::UnicodeWidthStr;

use super::app::{AGENT_MARK, App, Field};
use super::dialog::Dialog;
use super::effects;
use super::keys::{FOCUS_HINTS, Focus, HELP};
use crate::editor::Origin;
use crate::format::text::cell_to_text;
use crate::monitor::{Monitor, SCOPE_LEN};

const DIM: Color = Color::DarkGray;
/// Smallest voice column: "│ C-3 01 A04" plus a margin.
const MIN_VOICE_WIDTH: u16 = 13;
/// Scope height, in text rows (4 Braille dots per row).
const SCOPE_HEIGHT: u16 = 4;
/// Waveform zoom: a voice rarely goes past half scale, and the master is scaled down by the
/// mix (2 / voice count).
const VOICE_SCOPE_GAIN: f32 = 2.0;
const MASTER_SCOPE_GAIN: f32 = 3.0;
/// Effects shown on the second help page, one translation id each (`effect.help.<id>`).
const EFFECT_HELP: &[&str] = &[
    "0", "1", "2", "3", "4", "5", "6", "7", "9", "A", "B", "C", "D", "E1", "E2", "E4", "E5", "E6",
    "E7", "E9", "EA", "EB", "EC", "ED", "EE", "F",
];

/// Pads `text` with spaces up to `width` terminal columns (a Japanese character takes two).
fn pad_to(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}

pub fn draw(f: &mut Frame, app: &App) {
    let [header, body, hints, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(10),
        Constraint::Length(2),
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
    f.render_widget(Paragraph::new(hint_lines(app, hints.width as usize)), hints);
    f.render_widget(Line::from(app.status.as_str()).fg(Color::Gray), status);

    if let Some(dialog) = &app.dialog {
        draw_dialog(f, app, dialog);
    }
}

/// Bottom hints (two rows): on an effect column, what the effect under the cursor does (or
/// the effects to pick from); elsewhere, the useful keys of the active zone.
fn hint_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let on_effect = matches!(
        app.field,
        Field::Effect | Field::ParamHigh | Field::ParamLow
    );
    if app.focus == Focus::Pattern && on_effect {
        let cell = app.current_cell();
        if let Some(text) = effects::describe(cell.effect, cell.param) {
            return vec![Line::from(vec![
                text.fg(Color::Magenta),
                format!("   {}", t!("hint.effect_help")).fg(DIM),
            ])];
        }
        let palette = if cell.effect == 0xE {
            effects::extended_palette()
        } else {
            effects::palette()
        };
        let prefix = if cell.effect == 0xE { "E" } else { "" };
        let groups = palette
            .into_iter()
            .map(|(digit, name)| (format!("{prefix}{digit}"), name, Color::Magenta));
        let lead = Some(format!("{} ", t!("hint.effects")));
        return wrap_groups(lead, groups, width);
    }
    let id = match app.focus {
        Focus::Pattern if app.edit_mode => FOCUS_HINTS[0],
        Focus::Pattern => FOCUS_HINTS[1],
        Focus::Orders => FOCUS_HINTS[2],
        Focus::Samples => FOCUS_HINTS[3],
    };
    let hints = t!(id);
    let groups = hints.split(" · ").map(|part| {
        let (key, what) = part.split_once(' ').unwrap_or((part, ""));
        (key.to_string(), what.to_string(), Color::Cyan)
    });
    wrap_groups(None, groups, width)
}

/// Lays out "key action" groups over as many lines as needed, never splitting a group.
fn wrap_groups(
    lead: Option<String>,
    groups: impl Iterator<Item = (String, String, Color)>,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    if let Some(lead) = lead {
        used = lead.width();
        spans.push(lead.fg(DIM));
    }
    for (key, what, color) in groups {
        let group_width = key.width() + 1 + what.width();
        let gap = if spans.is_empty() { 0 } else { 2 };
        if used + gap + group_width > width && !spans.is_empty() {
            lines.push(Line::from(std::mem::take(&mut spans)));
            used = 0;
        } else if gap > 0 {
            spans.push("  ".into());
            used += gap;
        }
        spans.push(key.fg(color));
        spans.push(format!(" {what}").fg(DIM));
        used += group_width;
    }
    if !spans.is_empty() {
        lines.push(Line::from(spans));
    }
    lines
}

fn panel(title: String, focused: bool) -> Block<'static> {
    let style = Style::new().fg(if focused { Color::Cyan } else { DIM });
    Block::bordered().title(title).border_style(style)
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let song = app.song();
    let file = app
        .session
        .path
        .as_ref()
        .and_then(|p| p.file_name())
        .map_or(t!("view.no_file").into_owned(), |n| {
            n.to_string_lossy().into_owned()
        });
    let mode = if app.edit_mode {
        format!(" {} ", t!("view.edit"))
            .bold()
            .fg(Color::White)
            .bg(Color::Red)
    } else {
        format!(" {} ", t!("view.listen")).fg(DIM)
    };
    let agents = app
        .agents
        .as_ref()
        .map_or(0, |a| a.load(std::sync::atomic::Ordering::Relaxed));
    let fresh = app
        .agent_active
        .is_some_and(|t| t.elapsed().as_secs_f32() < 2.0);
    let agent = match (agents, fresh) {
        (0, _) => "".into(),
        (_, true) => format!(" {} ", t!("view.agent"))
            .bold()
            .fg(Color::Black)
            .bg(Color::Green),
        (_, false) => format!(" {} ", t!("view.agent")).fg(Color::Green),
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
        format!("{file}{} ", if app.session.dirty { " *" } else { "" }).fg(DIM),
        play,
        mode,
        agent,
        format!(
            "  {}  ",
            t!(
                "view.octave_sample",
                octave = app.octave,
                sample = format!("{:02}", app.sample),
                name = sample.display_name()
            )
        )
        .into(),
        format!("{}  ", app.layout.name).fg(DIM),
        format!(
            "{}  ",
            t!("view.tempo", speed = app.tempo.0, bpm = app.tempo.1)
        )
        .fg(Color::Yellow),
        t!(
            "view.position",
            position = format!("{:02}", app.position),
            last = format!("{:02}", song.order_list().len() - 1)
        )
        .into_owned()
        .fg(DIM),
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
    let title = format!(" {} ", t!("view.pattern", pattern = format!("{p:02}")));
    let block = Block::bordered()
        .title(title)
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
    let mixer = &app.session.editor.mixer;
    let audible = |v: usize| mixer.audible(v);
    // Voice columns share the available width.
    let width = ((inner.width.saturating_sub(3)) / channels as u16).max(MIN_VOICE_WIDTH) as usize;

    // Voice headers.
    let mut head = vec![Span::raw("   ")];
    for v in 0..channels {
        let style = Style::new().fg(if audible(v) { Color::Cyan } else { DIM });
        let label = format!("│ {}", t!("view.voice", voice = v + 1));
        head.push(Span::styled(pad_to(&label, width), style));
    }
    f.render_widget(Line::from(head), voices_header);

    // Grid: the cursor row stays in the middle, as in ProTracker.
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
                let marked = app
                    .agent_marks
                    .get(&(p, row, v))
                    .is_some_and(|t| t.elapsed() < AGENT_MARK);
                let mut cell = cell_spans(&text, cursor_field, audible(v));
                if marked {
                    // Written by the agent lately: a green tint, like fresh paint on the hull.
                    cell = cell
                        .into_iter()
                        .map(|span| span.bg(Color::Rgb(20, 60, 30)))
                        .collect();
                }
                spans.extend(cell);
                spans.push(" ".repeat(width.saturating_sub(12)).into());
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

    // Scope and state under each voice.
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
        let lines = scope(
            &wave,
            scope_area.width as usize,
            SCOPE_HEIGHT as usize,
            VOICE_SCOPE_GAIN,
            color,
        );
        f.render_widget(Paragraph::new(lines), scope_area);

        let state = match (mixer.mute[v], mixer.solo[v]) {
            (_, true) => format!(" {} ", t!("view.solo"))
                .fg(Color::Black)
                .bg(Color::Yellow),
            (true, _) => format!(" {} ", t!("view.muted"))
                .fg(Color::White)
                .bg(Color::Red),
            _ => "".into(),
        };
        let volume = format!("{:>3} % ", (mixer.volume[v] * 100.0).round() as u32);
        let used = 2 + volume.width() + state.content.width();
        state_line.push("│ ".fg(DIM));
        state_line.push(volume.fg(if audible(v) { Color::Gray } else { DIM }));
        state_line.push(state);
        state_line.push(" ".repeat(width.saturating_sub(used)).into());
    }
    f.render_widget(Line::from(state_line), states);
}

/// Recent samples of a voice (`None` = master), locked onto a rising zero crossing so the
/// waveform stands still from one frame to the next.
fn triggered_scope(monitor: &Monitor, voice: Option<usize>) -> Vec<f32> {
    let mut all = vec![0.0f32; SCOPE_LEN];
    monitor.scope(voice, &mut all);
    let shown = SCOPE_LEN / 2;
    let start = (1..SCOPE_LEN - shown)
        .find(|&i| all[i - 1] <= 0.0 && all[i] > 0.0)
        .unwrap_or(SCOPE_LEN - shown);
    all[start..start + shown].to_vec()
}

/// Waveform in Braille characters (2 × 4 dots each). Each dot column covers several samples:
/// draw the segment from their minimum to their maximum.
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
        // Join the last dot of the previous column: the curve stays continuous.
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

/// The pieces of a cell, with the cursor field in reverse video.
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

/// Horizontal level bar, to an eighth of a character.
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
    let block = panel(
        format!(" {} ", t!("view.orders")),
        app.focus == Focus::Orders,
    );
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
            let line = Line::from(format!(
                " {i:02}  {}",
                t!("view.pattern", pattern = format!("{p:02}"))
            ));
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
    let block = panel(
        format!(" {} ", t!("view.samples")),
        app.focus == Focus::Samples,
    );
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
    let lines = scope(
        &samples,
        wave.width as usize,
        wave.height as usize,
        MASTER_SCOPE_GAIN,
        Color::Cyan,
    );
    f.render_widget(Paragraph::new(lines), wave);
    let level = app.audio.monitor.level(None);
    f.render_widget(
        Line::from(meter(level * 2.0, bar.width as usize, true)),
        bar,
    );
}

/// First item shown so that `selected` stays visible in a list of `len` items.
fn scroll(selected: usize, len: usize, height: usize) -> usize {
    if len <= height {
        0
    } else {
        selected.saturating_sub(height / 2).min(len - height)
    }
}

/// Lines of a help page: keys, or effects.
fn help_lines(page: usize) -> Vec<Line<'static>> {
    let rows: Vec<(String, String)> = if page == 0 {
        HELP.iter()
            .map(|id| {
                (
                    t!(format!("help.{id}.keys")).into_owned(),
                    t!(format!("help.{id}.what")).into_owned(),
                )
            })
            .collect()
    } else {
        EFFECT_HELP
            .iter()
            .map(|id| {
                let text = t!(format!("effect.help.{id}")).into_owned();
                let (code, what) = text.split_once("  ").unwrap_or((&text, ""));
                (code.to_string(), what.trim().to_string())
            })
            .collect()
    };
    let key_width = rows.iter().map(|(k, _)| k.width()).max().unwrap_or(0) + 2;
    let color = if page == 0 {
        Color::Cyan
    } else {
        Color::Magenta
    };
    rows.into_iter()
        .map(|(k, what)| Line::from(vec![pad_to(&k, key_width).fg(color), what.into()]))
        .collect()
}

fn draw_dialog(f: &mut Frame, app: &App, dialog: &Dialog) {
    let area = f.area();
    let (title, lines, width): (String, Vec<Line>, u16) =
        match dialog {
            Dialog::Help(page) => {
                let title = if *page == 0 {
                    t!("dialog.help_keys")
                } else {
                    t!("dialog.help_effects")
                };
                (format!(" {title} "), help_lines(*page), 100)
            }
            Dialog::Journal => {
                let journal = app.session.editor.journal();
                let shown = (area.height.saturating_sub(6)) as usize;
                let lines = journal[journal.len().saturating_sub(shown)..]
                    .iter()
                    .map(|e| {
                        let secs = e
                            .time
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_secs());
                        let time = crate::editor::format_time(secs);
                        let (who, color) = match e.origin {
                            Origin::Agent => (t!("journal.agent"), Color::Green),
                            Origin::Keyboard => (t!("journal.keyboard"), Color::Gray),
                        };
                        Line::from(vec![
                            format!("{} ", &time[11..19]).fg(DIM),
                            pad_to(&who, 12).fg(color),
                            e.text.clone().fg(color),
                        ])
                    })
                    .collect();
                (format!(" {} ", t!("dialog.journal")), lines, 100)
            }
            Dialog::Prompt(p) => {
                let lines = vec![Line::from(vec![p.text.clone().into(), "█".fg(Color::Cyan)])];
                (
                    format!(" {} — {} ", p.label, t!("dialog.prompt_keys")),
                    lines,
                    70,
                )
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
                (format!(" {} ", c.label), lines, 40)
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
                (
                    format!(" {} — {} ", b.title, t!("dialog.browser_keys")),
                    lines,
                    70,
                )
            }
        };
    let width = width.min(area.width);
    let height = (lines.len() as u16 + 2).min(area.height);
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
        let song = Song::new("screen");
        let mut app = App::new(song.clone(), None, Audio::silent(&song));
        app.handle_key(KeyEvent::from(KeyCode::Char(' ')));
        app.handle_key(KeyEvent::from(KeyCode::Char('z')));
        app.session.editor.mixer.mute[2] = true;
        let s = render(&app);
        for expected in [
            "Space listen",
            "Alt+S solo",
            "smpltrckr",
            "screen",
            "EDIT",
            "pattern 00",
            "voice 4",
            "C-2 01 ...",
            "muted",
            "orders (F6)",
            "samples (F7)",
            "master",
        ] {
            assert!(s.contains(expected), "missing: {expected}\n{s}");
        }
    }

    #[test]
    fn effect_column_explains_the_effect_under_the_cursor() {
        let song = Song::new("");
        let mut app = App::new(song.clone(), None, Audio::silent(&song));
        app.handle_key(KeyEvent::from(KeyCode::Char(' ')));
        app.field = Field::Effect;
        let s = render(&app);
        assert!(
            s.contains("arpeggio") && s.contains("tempo"),
            "palette expected\n{s}"
        );
        for c in ['a', '0', '4'] {
            app.handle_key(KeyEvent::from(KeyCode::Char(c)));
            app.row = 0;
            app.field = match app.field {
                Field::Effect => Field::ParamHigh,
                _ => Field::ParamLow,
            };
        }
        app.field = Field::Effect;
        let s = render(&app);
        assert!(s.contains("A04: volume slide: down by 4 per tick"), "{s}");
    }

    #[test]
    fn scope_draws_a_continuous_wave() {
        // One sine cycle over 8 characters × 2 rows: every dot column is lit.
        let wave: Vec<f32> = (0..256)
            .map(|i| (i as f32 / 256.0 * std::f32::consts::TAU).sin())
            .collect();
        let lines = scope(&wave, 8, 2, 1.0, Color::Green);
        assert_eq!(lines.len(), 2);
        let mut columns = [false; 16];
        for line in &lines {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            for (c, ch) in text.chars().enumerate() {
                let bits = (ch as u32).saturating_sub(0x2800);
                columns[c * 2] |= bits & 0x47 != 0;
                columns[c * 2 + 1] |= bits & 0xB8 != 0;
            }
        }
        assert!(columns.iter().all(|&c| c), "{columns:?}");
        // The top of the wave (start of the cycle, positive sine) sits on the first row.
        let first: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(first.chars().take(4).any(|c| c != ' '));
    }

    #[test]
    fn help_has_a_keys_page_and_an_effects_page() {
        let song = Song::new("");
        let mut app = App::new(song.clone(), None, Audio::silent(&song));
        app.handle_key(KeyEvent::from(KeyCode::Char('?')));
        let s = render(&app);
        assert!(s.contains("Ctrl+Q") && s.contains("Tab"), "{s}");
        app.handle_key(KeyEvent::from(KeyCode::Tab));
        let s = render(&app);
        assert!(s.contains("0xy") && s.contains("Fxx"), "{s}");
    }

    #[test]
    fn japanese_text_keeps_columns_aligned() {
        assert_eq!(pad_to("音量", 6).width(), 6);
        assert_eq!(pad_to("vol", 6), "vol   ");
    }

    /// Text capture of the screen on a real song: `cargo test snapshot -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn snapshot() {
        let path = std::env::var("SNAPSHOT_MOD")
            .unwrap_or("sessions/2026-10-03-chiptune-la-mineur/morceau.mod".into());
        if let Ok(language) = std::env::var("SNAPSHOT_LANG") {
            crate::lang::set(&language);
        }
        let song = crate::format::protracker::read(&std::fs::read(&path).unwrap()).unwrap();
        let mut app = App::new(song.clone(), Some(path.into()), Audio::silent(&song));
        {
            // Two seconds of playback to fill the scopes.
            let mut r = app.audio.replayer.lock().unwrap();
            r.play(0, false);
            let mut buf = vec![0.0f32; 2 * 48000 * 2 + 2 * 3000];
            r.process(&mut buf);
        }
        app.tick();
        println!("{}", render(&app));
    }
}
