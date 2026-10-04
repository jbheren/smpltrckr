//! Colours of the interface, by role. Classic terminal colours by default, or the active
//! Omarchy theme (`~/.local/state/omarchy/current/theme/colors.toml`), reloaded when the
//! user switches theme. « Hissez les couleurs ! »

use std::path::PathBuf;
use std::time::SystemTime;

use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Ordinary text.
    pub text: Color,
    /// Secondary text, empty cells, idle borders.
    pub dim: Color,
    /// Notes and anything that must stand out.
    pub strong: Color,
    /// Focused borders, logo, voice headers, key names.
    pub accent: Color,
    /// Text drawn on an accent, play or edit background.
    pub on_accent: Color,
    pub sample: Color,
    pub effect: Color,
    /// Voice scopes.
    pub scope: Color,
    /// Master scope.
    pub master: Color,
    /// Edit mode border and badge, muted voices.
    pub edit: Color,
    pub solo: Color,
    /// Background of the cursor row (stopped) and of selected list items.
    pub cursor_row: Color,
    /// Background of the row being played and of selected dialog items.
    pub play_row: Color,
    /// Background of cells the agent wrote lately.
    pub agent_mark: Color,
    /// Agent badge.
    pub agent: Color,
    /// Play badge.
    pub playing: Color,
    pub meter_low: Color,
    pub meter_mid: Color,
    pub meter_high: Color,
    /// Fresh particles, then fading ones.
    pub particle: Color,
    pub particle_fade: Color,
}

impl Theme {
    /// The colours used before themes existed: plain terminal colours.
    pub fn classic() -> Self {
        Self {
            text: Color::Gray,
            dim: Color::DarkGray,
            strong: Color::White,
            accent: Color::Cyan,
            on_accent: Color::Black,
            sample: Color::Yellow,
            effect: Color::Magenta,
            scope: Color::Green,
            master: Color::Cyan,
            edit: Color::Red,
            solo: Color::Yellow,
            cursor_row: Color::Rgb(40, 40, 60),
            play_row: Color::Blue,
            agent_mark: Color::Rgb(20, 60, 30),
            agent: Color::Green,
            playing: Color::Green,
            meter_low: Color::Green,
            meter_mid: Color::Yellow,
            meter_high: Color::Red,
            particle: Color::LightCyan,
            particle_fade: Color::DarkGray,
        }
    }

    /// Theme built from an Omarchy `colors.toml` (`key = "#rrggbb"` lines). Missing keys fall
    /// back to the classic colours.
    pub fn from_omarchy(colors: &str) -> Self {
        let get = |key: &str| -> Option<Color> {
            colors.lines().find_map(|line| {
                let (k, v) = line.split_once('=')?;
                (k.trim() == key).then(|| parse_hex(v.trim().trim_matches('"')))?
            })
        };
        let classic = Self::classic();
        let pick =
            |keys: &[&str], fallback: Color| keys.iter().find_map(|k| get(k)).unwrap_or(fallback);
        let background = pick(&["background"], Color::Black);
        let green = pick(&["green"], classic.scope);
        Self {
            text: pick(&["foreground"], classic.text),
            dim: pick(&["dark_foreground", "muted"], classic.dim),
            strong: pick(&["bright_foreground", "foreground"], classic.strong),
            accent: pick(&["bright_blue", "accent", "cyan"], classic.accent),
            on_accent: background,
            sample: pick(&["yellow", "orange"], classic.sample),
            effect: pick(&["magenta", "bright_magenta"], classic.effect),
            scope: pick(&["bright_green", "green"], classic.scope),
            master: pick(&["bright_cyan", "cyan"], classic.master),
            edit: pick(&["red", "bright_red"], classic.edit),
            solo: pick(&["bright_yellow", "yellow"], classic.solo),
            cursor_row: pick(&["selection", "lighter_background"], classic.cursor_row),
            play_row: pick(&["accent", "blue"], classic.play_row),
            agent_mark: blend(green, background, 0.35),
            agent: pick(&["bright_green", "green"], classic.agent),
            playing: green,
            meter_low: green,
            meter_mid: pick(&["yellow"], classic.meter_mid),
            meter_high: pick(&["red"], classic.meter_high),
            particle: pick(&["bright_cyan", "cyan"], classic.particle),
            particle_fade: pick(&["muted", "dark_foreground"], classic.particle_fade),
        }
    }
}

/// `#rrggbb` → colour.
fn parse_hex(text: &str) -> Option<Color> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some(Color::Rgb(
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ))
}

/// Mix of two RGB colours (`amount` of `a`); other colours give `a`.
fn blend(a: Color, b: Color, amount: f32) -> Color {
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let mix = |x: u8, y: u8| (x as f32 * amount + y as f32 * (1.0 - amount)).round() as u8;
            Color::Rgb(mix(r1, r2), mix(g1, g2), mix(b1, b2))
        }
        _ => a,
    }
}

/// Where the theme comes from, to reload it when it changes.
pub struct ThemeSource {
    path: PathBuf,
    modified: Option<SystemTime>,
}

impl ThemeSource {
    /// The active Omarchy theme, if there is one.
    pub fn omarchy() -> Option<Self> {
        let home = std::env::var_os("HOME")?;
        let path = PathBuf::from(home).join(".local/state/omarchy/current/theme/colors.toml");
        path.exists().then_some(Self {
            path,
            modified: None,
        })
    }

    /// The theme, if it is new or changed since the last call.
    pub fn changed(&mut self) -> Option<Theme> {
        let modified = std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok();
        if modified.is_some() && modified == self.modified {
            return None;
        }
        self.modified = modified;
        std::fs::read_to_string(&self.path)
            .ok()
            .map(|text| Theme::from_omarchy(&text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JAPAN_NIGHT: &str = r##"mode = "dark"
accent = "#2b5e8f"
selection = "#1c3b5c"
background = "#0b1b2b"
foreground = "#b9b6a7"
green = "#708c8b"
"##;

    #[test]
    fn reads_an_omarchy_palette() {
        let theme = Theme::from_omarchy(JAPAN_NIGHT);
        assert_eq!(theme.text, Color::Rgb(0xb9, 0xb6, 0xa7));
        assert_eq!(theme.cursor_row, Color::Rgb(0x1c, 0x3b, 0x5c));
        assert_eq!(theme.on_accent, Color::Rgb(0x0b, 0x1b, 0x2b));
        // Missing keys keep the classic colours.
        assert_eq!(theme.effect, Theme::classic().effect);
        // The agent tint sits between green and the background.
        assert_eq!(theme.agent_mark, Color::Rgb(0x2e, 0x43, 0x4d));
    }

    #[test]
    fn ignores_garbage() {
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("red"), None);
        let theme = Theme::from_omarchy("not = a theme");
        assert_eq!(
            (theme.text, theme.accent),
            (Theme::classic().text, Theme::classic().accent)
        );
    }
}
