//! Raccourcis clavier : la table unique qui relie les touches aux actions.
//!
//! Le clavier « piano » suit la position physique des touches, comme dans ProTracker et FT2 :
//! la même place sur un clavier QWERTY, AZERTY ou QWERTZ. Un terminal transmet des caractères,
//! pas des positions : chaque disposition dit donc quel caractère produit chaque touche.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Disposition du clavier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub name: &'static str,
    /// Rangée du bas puis rangée du milieu, aux places de « z s x d c v g b h n j m , l . ; / »
    /// sur un QWERTY : do, do#, ré… de l'octave choisie.
    piano_low: &'static str,
    /// Rangée des lettres et des chiffres, aux places de « q 2 w 3 e r 5 t 6 y 7 u i 9 o 0 p » :
    /// l'octave au-dessus.
    piano_high: &'static str,
    /// Caractères de la rangée des chiffres sans majuscule, de 1 à 9 puis 0.
    digit_row: [char; 10],
}

pub const LAYOUTS: [Layout; 3] = [
    Layout {
        name: "QWERTY",
        piano_low: "zsxdcvgbhnjm,l.;/",
        piano_high: "q2w3er5t6y7ui9o0p",
        digit_row: ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0'],
    },
    Layout {
        name: "AZERTY",
        piano_low: "wsxdcvgbhnj,;l:m!",
        piano_high: "aéz\"er(t-yèuiçoàp",
        digit_row: ['&', 'é', '"', '\'', '(', '-', 'è', '_', 'ç', 'à'],
    },
    Layout {
        name: "QWERTZ",
        piano_low: "ysxdcvgbhnjm,l.ö-",
        piano_high: "q2w3er5t6z7ui9o0p",
        digit_row: ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0'],
    },
];

impl Layout {
    pub fn by_name(name: &str) -> Option<Layout> {
        let name = name.to_ascii_lowercase();
        let wanted = match name.as_str() {
            "fr" | "be" | "azerty" => "AZERTY",
            "de" | "ch" | "at" | "qwertz" => "QWERTZ",
            "us" | "gb" | "uk" | "qwerty" => "QWERTY",
            _ => return None,
        };
        LAYOUTS.iter().copied().find(|l| l.name == wanted)
    }

    /// Disposition suivante (F3).
    pub fn next(self) -> Layout {
        let i = LAYOUTS.iter().position(|l| *l == self).unwrap_or(0);
        LAYOUTS[(i + 1) % LAYOUTS.len()]
    }

    /// Demi-tons au-dessus du do de l'octave choisie.
    pub fn piano(&self, c: char) -> Option<i32> {
        let find = |row: &str| row.chars().position(|x| x == c).map(|i| i as i32);
        find(self.piano_low).or_else(|| find(self.piano_high).map(|i| i + 12))
    }

    /// Chiffre tapé, avec ou sans majuscule (sur AZERTY, « é » vaut 2).
    pub fn digit(&self, c: char) -> Option<u32> {
        c.to_digit(10).or_else(|| {
            self.digit_row
                .iter()
                .position(|&d| d == c)
                .map(|i| (i as u32 + 1) % 10)
        })
    }

    /// Chiffre hexadécimal tapé (0-9, A-F).
    pub fn hex_digit(&self, c: char) -> Option<u32> {
        self.digit(c).or_else(|| c.to_digit(16))
    }
}

/// Disposition au démarrage : `SMPLTRCKR_CLAVIER`, sinon celle du clavier principal de
/// Hyprland, sinon celle de localectl, sinon QWERTY.
pub fn detect_layout() -> Layout {
    let from_env = std::env::var("SMPLTRCKR_CLAVIER")
        .ok()
        .and_then(|n| Layout::by_name(&n));
    from_env
        .or_else(hyprland_layout)
        .or_else(localectl_layout)
        .unwrap_or(LAYOUTS[0])
}

fn hyprland_layout() -> Option<Layout> {
    let out = std::process::Command::new("hyprctl")
        .args(["-j", "devices"])
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    // Le clavier principal : « "layout": "fr" » dans le bloc qui contient « "main": true ».
    let block = text.split('{').find(|b| b.contains("\"main\": true"))?;
    let layout = block
        .split("\"layout\": \"")
        .nth(1)?
        .split(['"', ','])
        .next()?;
    Layout::by_name(layout)
}

fn localectl_layout() -> Option<Layout> {
    let out = std::process::Command::new("localectl")
        .arg("status")
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let line = text.lines().find(|l| l.contains("X11 Layout:"))?;
    Layout::by_name(line.split(':').nth(1)?.trim().split(',').next()?)
}

/// Zone qui reçoit les touches de navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Pattern,
    Orders,
    Samples,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    // Général
    Quit,
    Save,
    SaveAs,
    Open,
    Undo,
    Redo,
    PlaySong,
    PlayPattern,
    Stop,
    ToggleEdit,
    OctaveDown,
    OctaveUp,
    PrevSample,
    NextSample,
    SetFocus(Focus),
    ToggleMute(usize),
    Solo,
    ResetMix,
    VoiceVolume(i8),
    SetTitle,
    SetTempo,
    Help,
    NextLayout,
    // Déplacements (selon la zone active)
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    NextVoice,
    PrevVoice,
    // Pattern
    ClearField,
    ClearCell,
    InsertRow,
    DeleteRow,
    // Liste d'ordre et samples
    Insert,
    Delete,
    Enter,
    LoadSample,
    GenerateSample,
    RenameSample,
    FinetuneDown,
    FinetuneUp,
    Preview,
}

/// Action d'une touche, hors saisie (notes, chiffres) qui dépend de la colonne du curseur.
pub fn action(key: KeyEvent, focus: Focus, layout: Layout) -> Option<Action> {
    use Action::*;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    Some(match key.code {
        KeyCode::Char('q') if ctrl => Quit,
        KeyCode::Char('s') if ctrl => Save,
        KeyCode::Char('w') if ctrl => SaveAs,
        KeyCode::Char('o') if ctrl => Open,
        KeyCode::Char('z') if ctrl => Undo,
        KeyCode::Char('y') if ctrl => Redo,
        KeyCode::Char('p') if ctrl => PlayPattern,
        KeyCode::Char('t') if ctrl => SetTitle,
        KeyCode::Char('b') if ctrl => SetTempo,
        KeyCode::Char('k') if ctrl => DeleteRow,
        KeyCode::Char(c) if alt && matches!(layout.digit(c), Some(1..=8)) => {
            ToggleMute(layout.digit(c).unwrap() as usize - 1)
        }
        KeyCode::Char('s' | 'S') if alt => Solo,
        KeyCode::Char('0') if alt => ResetMix,
        KeyCode::Up if alt => VoiceVolume(1),
        KeyCode::Down if alt => VoiceVolume(-1),
        KeyCode::Enter => {
            if focus == Focus::Pattern {
                PlaySong
            } else {
                Enter
            }
        }
        KeyCode::Esc => Stop,
        KeyCode::Char(' ') => ToggleEdit,
        KeyCode::F(1) => OctaveDown,
        KeyCode::F(2) => OctaveUp,
        KeyCode::F(3) => NextLayout,
        KeyCode::F(5) => SetFocus(Focus::Pattern),
        KeyCode::F(6) => SetFocus(Focus::Orders),
        KeyCode::F(7) => SetFocus(Focus::Samples),
        KeyCode::Char('[') => {
            if focus == Focus::Samples {
                FinetuneDown
            } else {
                PrevSample
            }
        }
        KeyCode::Char(']') => {
            if focus == Focus::Samples {
                FinetuneUp
            } else {
                NextSample
            }
        }
        KeyCode::Char('?') => Help,
        KeyCode::Up => Up,
        KeyCode::Down => Down,
        KeyCode::Left => Left,
        KeyCode::Right => Right,
        KeyCode::PageUp => PageUp,
        KeyCode::PageDown => PageDown,
        KeyCode::Home => Home,
        KeyCode::End => End,
        KeyCode::Tab => NextVoice,
        KeyCode::BackTab => PrevVoice,
        KeyCode::Insert => {
            if focus == Focus::Pattern {
                InsertRow
            } else {
                Insert
            }
        }
        KeyCode::Delete => {
            if focus == Focus::Pattern {
                ClearField
            } else {
                Delete
            }
        }
        KeyCode::Backspace if focus == Focus::Pattern => ClearCell,
        KeyCode::Char('l') if focus == Focus::Samples => LoadSample,
        KeyCode::Char('g') if focus == Focus::Samples => GenerateSample,
        KeyCode::Char('n') if focus == Focus::Samples => RenameSample,
        KeyCode::Char('p') if focus == Focus::Samples => Preview,
        _ => return None,
    })
}

/// Rappel des touches de la zone active, affiché en bas de l'écran : (zone, « touche action · … »).
pub const FOCUS_HINTS: [(&str, &str); 4] = [
    (
        "pattern, édition",
        "Espace écoute · clavier piano notes · 0-9 A-F sample/effet · Suppr effacer · Entrée lire · F6 ordre · F7 samples · ? aide",
    ),
    (
        "pattern",
        "Espace éditer · Entrée lire · Ctrl+P boucle · Ctrl+B tempo · Alt+1…8 couper · F6 ordre · F7 samples · Ctrl+S enregistrer · ? aide",
    ),
    (
        "ordre",
        "↑↓ position · ←→ pattern (crée le suivant) · Inser ajouter · Suppr retirer · Entrée éditer · F5 pattern",
    ),
    (
        "samples",
        "↑↓ choisir · g générer · l charger WAV/AIFF · n renommer · ←→ volume · [ ] finetune · p écouter (Échap : silence) · Suppr vider · F5 pattern",
    ),
];

/// Aide affichée par « ? » : (touches, action).
pub const HELP: &[(&str, &str)] = &[
    ("Entrée", "lire / arrêter le morceau depuis la position"),
    ("Ctrl+P", "lire le pattern en boucle"),
    ("Échap", "arrêter la lecture et les notes écoutées"),
    (
        "Espace",
        "mode édition (sinon, les notes ne font que sonner)",
    ),
    (
        "rangées du bas et du haut",
        "notes (clavier piano, deux octaves, selon la disposition)",
    ),
    ("F1 / F2", "octave du clavier piano"),
    ("F3", "disposition du clavier : QWERTY, AZERTY, QWERTZ"),
    ("[ ]", "sample courant (dans les samples : finetune)"),
    (
        "flèches, PgPréc/PgSuiv",
        "se déplacer ; Tab / Maj+Tab : voie suivante / précédente",
    ),
    (
        "0-9 A-F",
        "sample (décimal) et effet (hexadécimal) selon la colonne",
    ),
    ("Suppr / Retour arrière", "effacer le champ / la cellule"),
    (
        "Inser / Ctrl+K",
        "insérer / supprimer une ligne dans la voie",
    ),
    ("Alt+1 … Alt+8", "couper / rétablir une voie"),
    (
        "Alt+S / Alt+0",
        "solo de la voie du curseur / mixage remis à zéro",
    ),
    ("Alt+↑ / Alt+↓", "volume de la voie du curseur"),
    ("F5 F6 F7", "zone active : pattern, liste d'ordre, samples"),
    (
        "ordre : ← → Inser Suppr Entrée",
        "changer le pattern, ajouter, retirer, éditer",
    ),
    (
        "samples : ← → l g n p Suppr",
        "volume, charger, générer, renommer, écouter, vider",
    ),
    (
        "Ctrl+S / Ctrl+W / Ctrl+O",
        "enregistrer / enregistrer sous / ouvrir",
    ),
    ("Ctrl+Z / Ctrl+Y", "annuler / rétablir"),
    ("Ctrl+T", "titre du morceau"),
    ("Ctrl+B", "tempo (BPM) et vitesse au début du morceau"),
    (
        "Ctrl+Q",
        "quitter (deux fois si le morceau n'est pas enregistré)",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piano_keys_sit_at_the_same_places_on_every_layout() {
        let [qwerty, azerty, qwertz] = LAYOUTS;
        // Touche en bas à gauche : do ; touche « A » du QWERTY (Q de l'AZERTY) : rien.
        assert_eq!(qwerty.piano('z'), Some(0));
        assert_eq!(azerty.piano('w'), Some(0));
        assert_eq!(qwertz.piano('y'), Some(0));
        assert_eq!(azerty.piano('q'), None);
        // Rangée du haut : do de l'octave suivante, puis do# sur la rangée des chiffres.
        assert_eq!(azerty.piano('a'), Some(12));
        assert_eq!(azerty.piano('é'), Some(13));
        assert_eq!(azerty.piano(','), Some(11));
        assert_eq!(azerty.piano('p'), Some(28));
        for layout in LAYOUTS {
            assert_eq!(layout.piano_low.chars().count(), 17, "{}", layout.name);
            assert_eq!(layout.piano_high.chars().count(), 17, "{}", layout.name);
        }
    }

    #[test]
    fn azerty_digit_row_counts_as_digits() {
        let azerty = Layout::by_name("fr").unwrap();
        assert_eq!(azerty.digit('&'), Some(1));
        assert_eq!(azerty.digit('à'), Some(0));
        assert_eq!(azerty.hex_digit('c'), Some(12));
        let alt_e = KeyEvent::new(KeyCode::Char('é'), KeyModifiers::ALT);
        assert_eq!(
            action(alt_e, Focus::Pattern, azerty),
            Some(Action::ToggleMute(1))
        );
    }

    #[test]
    fn same_key_depends_on_focus() {
        let del = KeyEvent::from(KeyCode::Delete);
        assert_eq!(
            action(del, Focus::Pattern, LAYOUTS[0]),
            Some(Action::ClearField)
        );
        assert_eq!(action(del, Focus::Orders, LAYOUTS[0]), Some(Action::Delete));
        let g = KeyEvent::from(KeyCode::Char('g'));
        assert_eq!(
            action(g, Focus::Samples, LAYOUTS[0]),
            Some(Action::GenerateSample)
        );
        assert_eq!(action(g, Focus::Pattern, LAYOUTS[0]), None);
    }
}
