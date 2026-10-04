//! The splash screen, with the block-letter logo.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use rust_i18n::t;

use super::app::App;
use super::theme::Theme;

/// Who made it: shown on the splash and in a corner of the interface.
pub const AUTHOR: &str = "@jbheren";

/// "SMPLTRCKR" in block letters, five rows high.
const LOGO: [&str; 5] = [
    "████ █   █ ████ █     █████ ████ ████ █  █ ████",
    "█    ██ ██ █  █ █       █   █  █ █    █ █  █  █",
    "████ █ █ █ ████ █       █   ████ █    ██   ████",
    "   █ █   █ █    █       █   █ █  █    █ █  █ █ ",
    "████ █   █ █    ████    █   █  █ ████ █  █ █  █",
];

/// Colour at `t` (0 to 1) along a gradient between two theme colours.
pub(super) fn gradient(from: Color, to: Color, t: f32) -> Color {
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
    lines.push(Line::from(t!("splash.author", author = AUTHOR).into_owned()).fg(th.accent));
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

#[cfg(test)]
mod tests {
    #[test]
    fn logo_rows_line_up() {
        let width = super::LOGO[0].chars().count();
        assert!(super::LOGO.iter().all(|row| row.chars().count() == width));
    }
}
