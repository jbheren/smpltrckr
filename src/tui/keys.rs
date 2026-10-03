//! Raccourcis clavier : la table unique qui relie les touches aux actions.
//!
//! Le clavier « piano » suit la position physique des touches d'un clavier QWERTY, comme dans
//! ProTracker et FT2. Une disposition AZERTY pourra s'ajouter ici sans toucher au reste.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
pub fn action(key: KeyEvent, focus: Focus) -> Option<Action> {
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
        KeyCode::Char(c @ '1'..='8') if alt => ToggleMute(c as usize - '1' as usize),
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

/// Clavier piano : demi-tons au-dessus du do de l'octave choisie (rangée du bas), ou de
/// l'octave suivante (rangée du haut).
pub fn piano(c: char) -> Option<i32> {
    const LOW: &str = "zsxdcvgbhnjm,l.;/";
    const HIGH: &str = "q2w3er5t6y7ui9o0p";
    LOW.find(c)
        .map(|i| i as i32)
        .or_else(|| HIGH.find(c).map(|i| i as i32 + 12))
}

/// Rappel des touches de la zone active, affiché en bas de l'écran : (zone, « touche action · … »).
pub const FOCUS_HINTS: [(&str, &str); 4] = [
    (
        "pattern, édition",
        "Espace écoute · zxc… notes · 0-9 A-F sample/effet · Suppr effacer · Entrée lire · F6 ordre · F7 samples · ? aide",
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
        "↑↓ choisir · g générer · l charger WAV/AIFF · n renommer · ←→ volume · [ ] finetune · p écouter · Suppr vider · F5 pattern",
    ),
];

/// Aide affichée par « ? » : (touches, action).
pub const HELP: &[(&str, &str)] = &[
    ("Entrée", "lire / arrêter le morceau depuis la position"),
    ("Ctrl+P", "lire le pattern en boucle"),
    ("Échap", "arrêter"),
    (
        "Espace",
        "mode édition (sinon, les notes ne font que sonner)",
    ),
    ("z s x … / q 2 w …", "notes (clavier piano, deux octaves)"),
    ("F1 / F2", "octave du clavier piano"),
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
    fn piano_follows_protracker_layout() {
        assert_eq!(piano('z'), Some(0));
        assert_eq!(piano('m'), Some(11));
        assert_eq!(piano('q'), Some(12));
        assert_eq!(piano('2'), Some(13));
        assert_eq!(piano('p'), Some(28));
        assert_eq!(piano('a'), None);
    }

    #[test]
    fn same_key_depends_on_focus() {
        let del = KeyEvent::from(KeyCode::Delete);
        assert_eq!(action(del, Focus::Pattern), Some(Action::ClearField));
        assert_eq!(action(del, Focus::Orders), Some(Action::Delete));
        let g = KeyEvent::from(KeyCode::Char('g'));
        assert_eq!(action(g, Focus::Samples), Some(Action::GenerateSample));
        assert_eq!(action(g, Focus::Pattern), None);
    }
}
