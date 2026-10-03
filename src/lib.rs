//! smpltrckr — a ProTracker-style text-mode tracker, played from the keyboard or by an agent
//! through MCP. « Hissez les samples ! »

rust_i18n::i18n!("locales", fallback = "en");

pub mod audio;
pub mod editor;
pub mod format;
pub mod lang;
pub mod mcp;
pub mod monitor;
pub mod note;
pub mod reference;
pub mod render;
pub mod replayer;
pub mod samples;
pub mod session;
pub mod song;
pub mod tone;
pub mod tui;
pub mod wav;
