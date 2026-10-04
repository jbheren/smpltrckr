//! The showy screens: the splash with the block-letter logo, and the full-screen demo mode
//! where each voice gets a big scope with its sparks (a nod to VLC's text-mode video output).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use rust_i18n::t;

use super::app::App;
use super::theme::Theme;
use super::view::{VOICE_SCOPE_GAIN, scope_with_particles};

/// "SMPLTRCKR" in block letters, five rows high.
const LOGO: [&str; 5] = [
    "████ █   █ ████ █     █████ ████ ████ █  █ ████",
    "█    ██ ██ █  █ █       █   █  █ █    █ █  █  █",
    "████ █ █ █ ████ █       █   ████ █    ██   ████",
    "   █ █   █ █    █       █   █ █  █    █ █  █ █ ",
    "████ █   █ █    ████    █   █  █ ████ █  █ █  █",
];

/// Colour at `t` (0 to 1) along a gradient between two theme colours.
fn gradient(from: Color, to: Color, t: f32) -> Color {
    match (from, to) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
            Color::Rgb(mix(r1, r2), mix(g1, g2), mix(b1, b2))
        }
        _ if t < 0.5 => from,
        _ => to,
    }
}

/// The logo, coloured column by column from the accent to the particle colour.
fn logo_lines(theme: &Theme) -> Vec<Line<'static>> {
    let width = LOGO[0].chars().count().max(1) as f32;
    LOGO.iter()
        .map(|row| {
            Line::from(
                row.chars()
                    .enumerate()
                    .map(|(i, c)| {
                        Span::from(c.to_string()).fg(gradient(
                            theme.accent,
                            theme.particle,
                            i as f32 / width,
                        ))
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

pub fn draw_splash(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let area = f.area();
    f.render_widget(Clear, area);
    let mut lines = logo_lines(th);
    lines.push(Line::default());
    lines.push(Line::from(t!("splash.tagline").into_owned()).fg(th.text));
    lines.push(Line::from(format!("v{}", env!("CARGO_PKG_VERSION"))).fg(th.dim));
    lines.push(Line::default());
    lines.push(Line::from(t!("splash.hint").into_owned()).fg(th.dim));
    let height = lines.len() as u16;
    let rect = Rect {
        y: area.y + area.height.saturating_sub(height) / 2,
        height: height.min(area.height),
        ..area
    };
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), rect);
}

pub fn draw_demo(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let area = f.area();
    f.render_widget(Clear, area);
    let channels = app.channels();
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .areas(area);

    let song = app.song();
    let title = Line::from(vec![
        " smpltrckr ".bold().fg(th.on_accent).bg(th.accent),
        format!("  {}  ", song.display_title()).bold().fg(th.strong),
        t!(
            "view.position",
            position = format!("{:02}", app.position),
            last = format!("{:02}", song.order_list().len() - 1)
        )
        .into_owned()
        .fg(th.dim),
        format!("  {}  ", t!("demo.row", row = format!("{:02}", app.row))).fg(th.dim),
        t!("view.tempo", speed = app.tempo.0, bpm = app.tempo.1)
            .into_owned()
            .fg(th.sample),
    ]);
    f.render_widget(title, header);

    // One band per voice, then the master.
    let constraints = vec![Constraint::Ratio(1, channels as u32 + 1); channels + 1];
    let bands = Layout::vertical(constraints).split(body);
    for (v, band) in bands.iter().enumerate() {
        let master = v == channels;
        let label = if master {
            " master ".to_string()
        } else {
            format!(" {} ", t!("view.voice", voice = v + 1))
        };
        let audible = master || app.session.editor.mixer.audible(v);
        let color = match (master, audible) {
            (true, _) => th.master,
            (false, true) => th.scope,
            (false, false) => th.dim,
        };
        let block = Block::default().title(label.fg(if audible { th.accent } else { th.dim }));
        let inner = block.inner(*band);
        f.render_widget(block, *band);
        let wave = app
            .audio
            .monitor
            .triggered_scope(if master { None } else { Some(v) });
        let sparks = if master {
            &[][..]
        } else {
            app.particles.voices.get(v).map_or(&[][..], |p| &p[..])
        };
        let gain = if master { 3.0 } else { VOICE_SCOPE_GAIN };
        let size = (inner.width as usize, inner.height as usize);
        f.render_widget(
            Paragraph::new(scope_with_particles(&wave, sparks, size, gain, color, th)),
            inner,
        );
    }
    f.render_widget(
        Line::from(t!("demo.hint").into_owned())
            .fg(th.dim)
            .alignment(Alignment::Center),
        footer,
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn logo_rows_line_up() {
        let width = super::LOGO[0].chars().count();
        assert!(super::LOGO.iter().all(|row| row.chars().count() == width));
    }
}
