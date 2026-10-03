//! Fenêtres de dialogue : navigateur de fichiers, saisie de texte, choix dans une liste, aide.

use std::path::{Path, PathBuf};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// À quoi sert la réponse du dialogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    OpenSong,
    LoadSample,
    SaveAs,
    RenameSample,
    SetTitle,
    SetTempo,
    Generate,
}

pub enum Dialog {
    Browser(Browser),
    Prompt(Prompt),
    Choice(Choice),
    Help,
}

/// Réponse d'un dialogue terminé.
pub enum Answer {
    Path(PathBuf),
    Text(String),
    Index(usize),
}

pub enum Outcome {
    /// Le dialogue reste ouvert.
    Pending,
    Cancel,
    Done(Purpose, Answer),
}

impl Dialog {
    pub fn handle(&mut self, key: KeyEvent) -> Outcome {
        if key.code == KeyCode::Esc {
            return Outcome::Cancel;
        }
        match self {
            Dialog::Help => Outcome::Cancel,
            Dialog::Browser(b) => b.handle(key),
            Dialog::Prompt(p) => p.handle(key),
            Dialog::Choice(c) => c.handle(key),
        }
    }
}

pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

pub struct Browser {
    pub purpose: Purpose,
    pub title: &'static str,
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub selected: usize,
    extensions: &'static [&'static str],
}

impl Browser {
    pub fn new(
        purpose: Purpose,
        title: &'static str,
        dir: &Path,
        extensions: &'static [&'static str],
    ) -> Self {
        let mut browser = Self {
            purpose,
            title,
            dir: dir.to_path_buf(),
            entries: Vec::new(),
            selected: 0,
            extensions,
        };
        browser.refresh();
        browser
    }

    /// Relit le dossier : « .. », puis les sous-dossiers, puis les fichiers aux bonnes extensions.
    fn refresh(&mut self) {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        if let Ok(read) = std::fs::read_dir(&self.dir) {
            for entry in read.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let is_dir = entry.path().is_dir();
                let lower = name.to_lowercase();
                if is_dir {
                    dirs.push(name);
                } else if self
                    .extensions
                    .iter()
                    .any(|e| lower.ends_with(e) || lower.starts_with(&format!("{}.", &e[1..])))
                {
                    files.push(name);
                }
            }
        }
        dirs.sort_by_key(|n| n.to_lowercase());
        files.sort_by_key(|n| n.to_lowercase());
        self.entries = std::iter::once(Entry {
            name: "..".into(),
            is_dir: true,
        })
        .chain(dirs.into_iter().map(|name| Entry { name, is_dir: true }))
        .chain(files.into_iter().map(|name| Entry {
            name,
            is_dir: false,
        }))
        .collect();
        self.selected = self.selected.min(self.entries.len() - 1);
    }

    fn handle(&mut self, key: KeyEvent) -> Outcome {
        let last = self.entries.len() - 1;
        match key.code {
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => self.selected = (self.selected + 1).min(last),
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(10),
            KeyCode::PageDown => self.selected = (self.selected + 10).min(last),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = last,
            KeyCode::Backspace | KeyCode::Left => self.enter_dir(".."),
            KeyCode::Enter | KeyCode::Right => {
                let entry = &self.entries[self.selected];
                if entry.is_dir {
                    let name = entry.name.clone();
                    self.enter_dir(&name);
                } else if key.code == KeyCode::Enter {
                    return Outcome::Done(self.purpose, Answer::Path(self.dir.join(&entry.name)));
                }
            }
            _ => {}
        }
        Outcome::Pending
    }

    fn enter_dir(&mut self, name: &str) {
        let previous = self
            .dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        self.dir = if name == ".." {
            self.dir
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.dir.clone())
        } else {
            self.dir.join(name)
        };
        self.selected = 0;
        self.refresh();
        // En remontant, on se replace sur le dossier d'où l'on vient.
        if name == ".."
            && let Some(prev) = previous
        {
            self.selected = self
                .entries
                .iter()
                .position(|e| e.name == prev)
                .unwrap_or(0);
        }
    }
}

pub struct Prompt {
    pub purpose: Purpose,
    pub label: String,
    pub text: String,
}

impl Prompt {
    pub fn new(purpose: Purpose, label: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            purpose,
            label: label.into(),
            text: text.into(),
        }
    }

    fn handle(&mut self, key: KeyEvent) -> Outcome {
        match key.code {
            KeyCode::Enter => return Outcome::Done(self.purpose, Answer::Text(self.text.clone())),
            KeyCode::Backspace => {
                self.text.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.text.clear()
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.text.push(c)
            }
            _ => {}
        }
        Outcome::Pending
    }
}

pub struct Choice {
    pub purpose: Purpose,
    pub label: String,
    pub items: Vec<String>,
    pub selected: usize,
}

impl Choice {
    fn handle(&mut self, key: KeyEvent) -> Outcome {
        match key.code {
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => self.selected = (self.selected + 1).min(self.items.len() - 1),
            KeyCode::Enter => return Outcome::Done(self.purpose, Answer::Index(self.selected)),
            _ => {}
        }
        Outcome::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_lists_dirs_then_matching_files_and_navigates() {
        let root = std::env::temp_dir().join(format!("smpltrckr-browser-{}", std::process::id()));
        std::fs::create_dir_all(root.join("sous")).unwrap();
        for f in ["b.mod", "A.MOD", "notes.txt", "mod.vieux"] {
            std::fs::write(root.join(f), b"").unwrap();
        }
        let mut b = Browser::new(Purpose::OpenSong, "ouvrir", &root, &[".mod"]);
        let names: Vec<&str> = b.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["..", "sous", "A.MOD", "b.mod", "mod.vieux"]);

        b.selected = 1;
        assert!(matches!(
            b.handle(KeyEvent::from(KeyCode::Enter)),
            Outcome::Pending
        ));
        assert!(b.dir.ends_with("sous"));
        b.handle(KeyEvent::from(KeyCode::Backspace));
        assert_eq!(b.entries[b.selected].name, "sous");

        b.selected = 3;
        let Outcome::Done(Purpose::OpenSong, Answer::Path(path)) =
            b.handle(KeyEvent::from(KeyCode::Enter))
        else {
            panic!("fichier non choisi");
        };
        assert!(path.ends_with("b.mod"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn prompt_ignores_control_keys_and_ctrl_u_clears() {
        let mut p = Prompt::new(Purpose::SaveAs, "test", "abc");
        p.handle(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(p.text, "abc");
        p.handle(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        p.handle(KeyEvent::from(KeyCode::Char('x')));
        assert_eq!(p.text, "x");
    }
}
