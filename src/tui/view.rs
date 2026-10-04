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
use super::particles::Particle;
use super::theme::Theme;
use crate::editor::Origin;
use crate::format::text::cell_to_text;

/// Smallest voice column: "│ C-3 01 A04" plus a margin.
const MIN_VOICE_WIDTH: u16 = 13;
/// Scope height, in text rows (4 Braille dots per row).
const SCOPE_HEIGHT: u16 = 4;
/// Waveform zoom: a voice rarely goes past half scale, and the master is scaled down by the
/// mix (2 / voice count).
pub const VOICE_SCOPE_GAIN: f32 = 2.0;
const MASTER_SCOPE_GAIN: f32 = 3.0;
/// Effects shown on the second help page, one translation id each (`effect.help.<id>`).
const EFFECT_HELP: &[&str] = &[
    "0", "1", "2", "3", "4", "5", "6", "7", "9", "A", "B", "C", "D", "E1", "E2", "E4", "E5", "E6",
    "E7", "E9", "EA", "EB", "EC", "ED", "EE", "F",
];

/// `left`, then `right` pushed against the right edge of a `width`-column line.
fn spread(left: &str, right: &str, width: usize) -> String {
    let gap = width.saturating_sub(left.width() + right.width()).max(1);
    format!("{left}{}{right}", " ".repeat(gap))
}

/// Pads `text` with spaces up to `width` terminal columns (a Japanese character takes two).
fn pad_to(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}

pub fn draw(f: &mut Frame, app: &App) {
    if app.splash.is_some() {
        return super::show::draw_splash(f, app);
    }
    let [header, _gap, body, hints, status] = Layout::vertical([
        Constraint::Length(1),
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
    // Status on the left, the author in the bottom-right corner.
    let author = super::show::AUTHOR;
    let [status, corner] = Layout::horizontal([
        Constraint::Min(10),
        Constraint::Length(author.width() as u16 + 1),
    ])
    .areas(status);
    f.render_widget(Line::from(app.status.as_str()).fg(app.theme.text), status);
    f.render_widget(
        Line::from(author)
            .fg(app.theme.dim)
            .alignment(ratatui::layout::Alignment::Right),
        corner,
    );

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
                text.fg(app.theme.effect),
                format!("   {}", t!("hint.effect_help")).fg(app.theme.dim),
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
            .map(|(digit, name)| (format!("{prefix}{digit}"), name, app.theme.effect));
        let lead = Some(format!("{} ", t!("hint.effects")));
        return wrap_groups(&app.theme, lead, groups, width);
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
        (key.to_string(), what.to_string(), app.theme.accent)
    });
    wrap_groups(&app.theme, None, groups, width)
}

/// Lays out "key action" groups over as many lines as needed, never splitting a group.
fn wrap_groups(
    theme: &Theme,
    lead: Option<String>,
    groups: impl Iterator<Item = (String, String, Color)>,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    if let Some(lead) = lead {
        used = lead.width();
        spans.push(lead.fg(theme.dim));
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
        spans.push(format!(" {what}").fg(theme.dim));
        used += group_width;
    }
    if !spans.is_empty() {
        lines.push(Line::from(spans));
    }
    lines
}

fn panel(title: String, focused: bool, theme: &Theme) -> Block<'static> {
    let style = Style::new().fg(if focused { theme.accent } else { theme.dim });
    Block::bordered().title(title).border_style(style)
}

/// The program name in capitals, coloured like the splash logo.
fn logo_spans(theme: &Theme) -> Vec<Span<'static>> {
    const NAME: &str = "SMPLTRCKR";
    let last = (NAME.len() - 1) as f32;
    NAME.chars()
        .enumerate()
        .map(|(i, c)| {
            let color = super::show::gradient(theme.accent, theme.particle, i as f32 / last);
            Span::from(c.to_string()).bold().fg(color)
        })
        .collect()
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
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
            .fg(th.strong)
            .bg(th.edit)
    } else {
        format!(" {} ", t!("view.listen")).fg(th.dim)
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
            .fg(th.on_accent)
            .bg(th.agent),
        (_, false) => format!(" {} ", t!("view.agent")).fg(th.agent),
    };
    let play = if app.running {
        " ▶ ".fg(th.on_accent).bg(th.playing)
    } else {
        " ■ ".fg(th.dim)
    };
    let sample = &song.samples[app.sample - 1];
    let mut spans = vec![Span::raw(" ")];
    spans.extend(logo_spans(th));
    spans.extend([
        format!("  {} ", song.display_title()).bold(),
        format!("{file}{} ", if app.session.dirty { " *" } else { "" }).fg(th.dim),
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
        format!("{}  ", app.layout.name).fg(th.dim),
        format!(
            "{}  ",
            t!("view.tempo", speed = app.tempo.0, bpm = app.tempo.1)
        )
        .fg(th.sample),
    ]);
    f.render_widget(Line::from(spans), area);
}

fn draw_pattern(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let p = app.pattern_index();
    let border = if app.edit_mode {
        th.edit
    } else if app.focus == Focus::Pattern {
        th.accent
    } else {
        th.dim
    };
    let title = format!(" {} ", t!("view.pattern", pattern = format!("{p:02}")));
    // The position in the order list sits in the top-right corner of the frame.
    let position = t!(
        "view.position",
        position = format!("{:02}", app.position),
        last = format!("{:02}", app.song().order_list().len() - 1)
    );
    let block = Block::bordered()
        .title(title)
        .title(
            Line::from(format!(" {position} "))
                .right_aligned()
                .fg(th.dim),
        )
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
        let style = Style::new().fg(if audible(v) { th.accent } else { th.dim });
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
                th.text
            } else {
                th.dim
            });
            let mut spans = vec![Span::styled(format!("{row:02} "), number_style)];
            for v in 0..channels {
                spans.push("│ ".fg(th.dim));
                let text = cell_to_text(&pattern.rows[row][v]);
                let cursor_field = (offset == 0 && v == app.voice && app.focus == Focus::Pattern)
                    .then_some(app.field);
                let marked = app
                    .agent_marks
                    .get(&(p, row, v))
                    .is_some_and(|t| t.elapsed() < AGENT_MARK);
                let mut cell = cell_spans(&text, cursor_field, audible(v), th);
                if marked {
                    // Written by the agent lately: a green tint, like fresh paint on the hull.
                    cell = cell
                        .into_iter()
                        .map(|span| span.bg(th.agent_mark))
                        .collect();
                }
                spans.extend(cell);
                spans.push(" ".repeat(width.saturating_sub(12)).into());
            }
            let line = Line::from(spans);
            match (offset, app.running) {
                (0, true) => line.style(Style::new().bg(th.play_row)),
                (0, false) => line.style(Style::new().bg(th.cursor_row)),
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
            Paragraph::new(vec![Line::from("│").fg(th.dim); SCOPE_HEIGHT as usize]),
            separator,
        );
        let scope_area = Rect {
            x: x + 1,
            width: (width as u16 - 2).min(scopes.right() - x - 1),
            ..scopes
        };
        let color = if audible(v) { th.scope } else { th.dim };
        let wave = app.audio.monitor.triggered_scope(Some(v));
        let sparks = app.particles.voices.get(v).map_or(&[][..], |p| &p[..]);
        let size = (scope_area.width as usize, SCOPE_HEIGHT as usize);
        let lines = scope_with_particles(&wave, sparks, size, VOICE_SCOPE_GAIN, color, th);
        f.render_widget(Paragraph::new(lines), scope_area);

        let state = match (mixer.mute[v], mixer.solo[v]) {
            (_, true) => format!(" {} ", t!("view.solo"))
                .fg(th.on_accent)
                .bg(th.solo),
            (true, _) => format!(" {} ", t!("view.muted")).fg(th.strong).bg(th.edit),
            _ => "".into(),
        };
        let volume = format!("{:>3} % ", (mixer.volume[v] * 100.0).round() as u32);
        let used = 2 + volume.width() + state.content.width();
        state_line.push("│ ".fg(th.dim));
        state_line.push(volume.fg(if audible(v) { th.text } else { th.dim }));
        state_line.push(state);
        state_line.push(" ".repeat(width.saturating_sub(used)).into());
    }
    f.render_widget(Line::from(state_line), states);
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
    let cells = wave_cells(samples, width, height, gain);
    braille_lines(&cells, width, |b, _| (b, color))
}

/// Braille dot bits: `BITS[x % 2][y % 4]`.
const BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// Waveform as Braille dot bits, one byte per character cell. Each dot column covers several
/// samples: draw the segment from their minimum to their maximum.
fn wave_cells(samples: &[f32], width: usize, height: usize, gain: f32) -> Vec<u8> {
    let (dots_w, dots_h) = (width * 2, height * 4);
    let mut cells = vec![0u8; width * height];
    if dots_w == 0 || samples.is_empty() {
        return cells;
    }
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
}

/// A voice scope with its particles on top: the wave keeps its colour, cells holding only
/// particles take the particle colour, bright when fresh and faded past half their life.
fn scope_with_particles(
    samples: &[f32],
    particles: &[Particle],
    (width, height): (usize, usize),
    gain: f32,
    color: Color,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let wave = wave_cells(samples, width, height, gain);
    let (dots_w, dots_h) = (width * 2, height * 4);
    let mut sparks = vec![0u8; width * height];
    let mut freshest = vec![1.0f32; width * height];
    for p in particles {
        let (x, y) = (
            (p.x * (dots_w as f32 - 1.0)).round(),
            (p.y * (dots_h as f32 - 1.0)).round(),
        );
        if !(0.0..dots_w as f32).contains(&x) || !(0.0..dots_h as f32).contains(&y) {
            continue;
        }
        let (x, y) = (x as usize, y as usize);
        let cell = (y / 4) * width + x / 2;
        sparks[cell] |= BITS[x % 2][y % 4];
        freshest[cell] = freshest[cell].min(p.fade());
    }
    braille_lines(&wave, width, |bits, i| match (bits, sparks[i]) {
        (0, 0) => (0, color),
        (0, s) => (
            s,
            if freshest[i] < 0.5 {
                theme.particle
            } else {
                theme.particle_fade
            },
        ),
        (w, s) => (w | s, color),
    })
}

/// Turns dot bits into lines of Braille characters; `paint(bits, index)` gives each cell its
/// final bits and colour. Neighbouring cells of the same colour share a span.
fn braille_lines(
    cells: &[u8],
    width: usize,
    paint: impl Fn(u8, usize) -> (u8, Color),
) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    cells
        .chunks(width)
        .enumerate()
        .map(|(row, chunk)| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut run = String::new();
            let mut run_color = None;
            for (col, &b) in chunk.iter().enumerate() {
                let (bits, color) = paint(b, row * width + col);
                let c = if bits == 0 {
                    ' '
                } else {
                    char::from_u32(0x2800 + bits as u32).unwrap()
                };
                if run_color.is_some_and(|rc| rc != color) {
                    spans.push(std::mem::take(&mut run).fg(run_color.unwrap()));
                }
                run_color = Some(color);
                run.push(c);
            }
            if let Some(color) = run_color {
                spans.push(run.fg(color));
            }
            Line::from(spans)
        })
        .collect()
}

/// The pieces of a cell, with the cursor field in reverse video.
fn cell_spans(
    text: &str,
    cursor: Option<Field>,
    audible: bool,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let part = |range: std::ops::Range<usize>| chars[range].iter().collect::<String>();
    let parts = [
        (part(0..3), theme.strong, Field::Note),
        (part(4..5), theme.sample, Field::SampleTens),
        (part(5..6), theme.sample, Field::SampleUnits),
        (part(7..8), theme.effect, Field::Effect),
        (part(8..9), theme.effect, Field::ParamHigh),
        (part(9..10), theme.effect, Field::ParamLow),
    ];
    let mut spans = Vec::new();
    for (i, (s, color, field)) in parts.into_iter().enumerate() {
        if i == 1 || i == 3 {
            spans.push(" ".into());
        }
        let empty = s.chars().all(|c| c == '.');
        let mut style = Style::new().fg(if empty || !audible { theme.dim } else { color });
        if cursor == Some(field) {
            style = style.add_modifier(Modifier::REVERSED);
        }
        spans.push(Span::styled(s, style));
    }
    spans
}

/// Horizontal level bar, to an eighth of a character.
fn meter(level: f32, width: usize, active: bool, theme: &Theme) -> Vec<Span<'static>> {
    const EIGHTHS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
    let filled = (level.clamp(0.0, 1.0) * width as f32 * 8.0).round() as usize;
    (0..width)
        .map(|i| {
            let color = match (active, i * 4 / width.max(1)) {
                (false, _) => theme.dim,
                (_, 0 | 1) => theme.meter_low,
                (_, 2) => theme.meter_mid,
                _ => theme.meter_high,
            };
            let c = match filled.saturating_sub(i * 8) {
                0 => '·',
                n if n >= 8 => '█',
                n => EIGHTHS[n],
            };
            Span::styled(
                c.to_string(),
                Style::new().fg(if c == '·' { theme.dim } else { color }),
            )
        })
        .collect()
}

fn draw_orders(f: &mut Frame, app: &App, area: Rect) {
    let block = panel(
        format!(" {} ", t!("view.orders")),
        app.focus == Focus::Orders,
        &app.theme,
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
            let left = format!(" {i:02}  {}", t!("view.pattern_label"));
            let line = Line::from(spread(&left, &format!("{p:02} "), inner.width as usize));
            if i == app.position {
                line.style(Style::new().bg(app.theme.cursor_row).bold())
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
        &app.theme,
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
            let name: String = s.display_name().chars().take(18).collect();
            let text = spread(
                &format!(" {:02}  {name}", i + 1),
                &format!("{:>2} ", s.volume),
                inner.width as usize,
            );
            let line = Line::from(text).fg(if empty { app.theme.dim } else { app.theme.text });
            if i + 1 == app.sample {
                line.style(Style::new().bg(app.theme.cursor_row).bold())
            } else {
                line
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_master(f: &mut Frame, app: &App, area: Rect) {
    let block = panel(" master ".into(), false, &app.theme);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [wave, bar] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    let samples = app.audio.monitor.triggered_scope(None);
    let lines = scope(
        &samples,
        wave.width as usize,
        wave.height as usize,
        MASTER_SCOPE_GAIN,
        app.theme.master,
    );
    f.render_widget(Paragraph::new(lines), wave);
    let level = app.audio.monitor.level(None);
    f.render_widget(
        Line::from(meter(level * 2.0, bar.width as usize, true, &app.theme)),
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
fn help_lines(page: usize, theme: &Theme) -> Vec<Line<'static>> {
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
        theme.accent
    } else {
        theme.effect
    };
    rows.into_iter()
        .map(|(k, what)| Line::from(vec![pad_to(&k, key_width).fg(color), what.into()]))
        .collect()
}

fn draw_dialog(f: &mut Frame, app: &App, dialog: &Dialog) {
    let th = &app.theme;
    let area = f.area();
    let (title, lines, width): (String, Vec<Line>, u16) =
        match dialog {
            Dialog::Help(page) => {
                let title = if *page == 0 {
                    t!("dialog.help_keys")
                } else {
                    t!("dialog.help_effects")
                };
                (format!(" {title} "), help_lines(*page, th), 100)
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
                            Origin::Agent => (t!("journal.agent"), th.agent),
                            Origin::Keyboard => (t!("journal.keyboard"), th.text),
                        };
                        Line::from(vec![
                            format!("{} ", &time[11..19]).fg(th.dim),
                            pad_to(&who, 12).fg(color),
                            e.text.clone().fg(color),
                        ])
                    })
                    .collect();
                (format!(" {} ", t!("dialog.journal")), lines, 100)
            }
            Dialog::Prompt(p) => {
                let lines = vec![Line::from(vec![p.text.clone().into(), "█".fg(th.accent)])];
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
                            line.style(Style::new().bg(th.play_row))
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
                let mut lines = vec![Line::from(b.dir.display().to_string()).fg(th.dim)];
                lines.extend(b.entries.iter().enumerate().skip(first).take(visible).map(
                    |(i, e)| {
                        let text = if e.is_dir {
                            format!(" {}/", e.name)
                        } else {
                            format!(" {}", e.name)
                        };
                        let line = Line::from(text).fg(if e.is_dir { th.accent } else { th.text });
                        if i == b.selected {
                            line.style(Style::new().bg(th.play_row))
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
                .border_style(Style::new().fg(th.accent)),
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
            "SMPLTRCKR",
            "@jbheren",
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
    fn spread_pushes_the_last_column_right() {
        assert_eq!(spread(" 00  pattern", "01 ", 20), " 00  pattern     01 ");
        assert_eq!(spread("toolong", "x", 4), "toolong x");
    }

    #[test]
    fn japanese_text_keeps_columns_aligned() {
        assert_eq!(pad_to("音量", 6).width(), 6);
        assert_eq!(pad_to("vol", 6), "vol   ");
    }

    /// Text capture of the splash: `cargo test snapshot_show -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn snapshot_show() {
        let path = std::env::var("SNAPSHOT_MOD")
            .unwrap_or("sessions/2026-10-03-kaze-no-uta/kaze-no-uta.mod".into());
        let song = crate::format::protracker::read(&std::fs::read(&path).unwrap()).unwrap();
        let mut app = App::new(song.clone(), Some(path.into()), Audio::silent(&song));
        app.splash = Some(std::time::Instant::now());
        println!("{}", render(&app));
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
        // A few frames for the sparks to fly.
        for _ in 0..4 {
            app.particles
                .update(&app.audio.monitor, VOICE_SCOPE_GAIN, 0.05);
        }
        println!("{}", render(&app));
    }
}
