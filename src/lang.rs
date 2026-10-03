//! Language selection. The interface speaks French, English and Japanese; the texts live in
//! `locales/*.yml` and are embedded at compile time.

/// Supported languages, as locale codes.
pub const LANGUAGES: [&str; 3] = ["en", "fr", "ja"];

/// Language to use: `SMPLTRCKR_LANG`, then the usual locale variables (`LANGUAGE`, `LC_ALL`,
/// `LC_MESSAGES`, `LANG`), then English.
pub fn detect() -> &'static str {
    [
        "SMPLTRCKR_LANG",
        "LANGUAGE",
        "LC_ALL",
        "LC_MESSAGES",
        "LANG",
    ]
    .iter()
    .filter_map(|var| std::env::var(var).ok())
    .find_map(|value| supported(&value))
    .unwrap_or("en")
}

/// Supported language matching a locale value such as `fr_FR.UTF-8`, `ja` or `en:fr`.
pub fn supported(value: &str) -> Option<&'static str> {
    let first = value.split(':').next()?.to_ascii_lowercase();
    LANGUAGES.iter().copied().find(|l| first.starts_with(l))
}

/// Sets the language for the whole program.
pub fn set(language: &str) {
    rust_i18n::set_locale(language);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_locale_values() {
        assert_eq!(supported("fr_FR.UTF-8"), Some("fr"));
        assert_eq!(supported("ja_JP.UTF-8"), Some("ja"));
        assert_eq!(supported("en_US"), Some("en"));
        assert_eq!(supported("de_DE"), None);
        assert_eq!(supported("ja:en"), Some("ja"));
    }
}
