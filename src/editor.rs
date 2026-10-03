//! Éditeur : toutes les modifications d'un morceau passent par ici, qu'elles viennent du
//! clavier ou de l'agent. Chaque modification s'annule, se rétablit et laisse une trace
//! dans le journal.
//!
//! Une modification est une liste de remplacements (`Change`). Appliquer un remplacement
//! renvoie le remplacement inverse : l'annulation n'a pas besoin de copier tout le morceau.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::ensure;

use crate::replayer::Mixer;
use crate::song::{Pattern, Sample, Song};

/// Qui a demandé la modification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Keyboard,
    Agent,
}

/// Un remplacement élémentaire dans le morceau.
#[derive(Debug, Clone)]
pub enum Change {
    Title([u8; 20]),
    /// Remplace un pattern, en ajoute un (à la fin) ou retire le dernier (`None`).
    Pattern(usize, Option<Pattern>),
    Orders {
        length: u8,
        orders: [u8; 128],
    },
    Sample(usize, Box<Sample>),
}

impl Change {
    /// Applique le remplacement et renvoie celui qui l'annule.
    fn apply(self, song: &mut Song) -> Change {
        match self {
            Change::Title(title) => Change::Title(std::mem::replace(&mut song.title, title)),
            Change::Pattern(i, Some(pattern)) if i == song.patterns.len() => {
                song.patterns.push(pattern);
                Change::Pattern(i, None)
            }
            Change::Pattern(i, Some(pattern)) => {
                Change::Pattern(i, Some(std::mem::replace(&mut song.patterns[i], pattern)))
            }
            Change::Pattern(i, None) => Change::Pattern(i, song.patterns.pop()),
            Change::Orders { length, orders } => {
                let old = Change::Orders {
                    length: song.song_length,
                    orders: song.orders,
                };
                song.song_length = length;
                song.orders = orders;
                old
            }
            Change::Sample(i, sample) => Change::Sample(
                i,
                Box::new(std::mem::replace(&mut song.samples[i], *sample)),
            ),
        }
    }
}

/// Une modification complète, telle qu'elle apparaît dans l'historique.
#[derive(Debug, Clone)]
struct Edit {
    description: String,
    changes: Vec<Change>,
}

/// Une ligne du journal.
#[derive(Debug, Clone)]
pub struct JournalEntry {
    pub time: SystemTime,
    pub origin: Origin,
    /// « modif », « annulation » ou « rétablissement », suivi de la description.
    pub text: String,
}

pub struct Editor {
    song: Song,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    journal: Vec<JournalEntry>,
    /// Mixage de la session (non enregistré dans le `.mod`).
    pub mixer: Mixer,
}

const UNDO_DEPTH: usize = 500;

impl Editor {
    pub fn new(song: Song) -> Self {
        let mixer = Mixer::new(song.channels);
        Self {
            song,
            undo: Vec::new(),
            redo: Vec::new(),
            journal: Vec::new(),
            mixer,
        }
    }

    pub fn song(&self) -> &Song {
        &self.song
    }

    pub fn journal(&self) -> &[JournalEntry] {
        &self.journal
    }

    /// Applique une modification. Une liste vide ne fait rien et ne laisse pas de trace.
    pub fn apply(&mut self, origin: Origin, description: impl Into<String>, changes: Vec<Change>) {
        if changes.is_empty() {
            return;
        }
        let description = description.into();
        let inverse = self.apply_all(changes);
        self.log(origin, format!("modif : {description}"));
        self.undo.push(Edit {
            description,
            changes: inverse,
        });
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Annule la dernière modification et renvoie sa description.
    pub fn undo(&mut self, origin: Origin) -> Option<String> {
        let edit = self.undo.pop()?;
        let inverse = self.apply_all(edit.changes);
        self.log(origin, format!("annulation : {}", edit.description));
        let description = edit.description.clone();
        self.redo.push(Edit {
            changes: inverse,
            ..edit
        });
        Some(description)
    }

    /// Rétablit la dernière modification annulée et renvoie sa description.
    pub fn redo(&mut self, origin: Origin) -> Option<String> {
        let edit = self.redo.pop()?;
        let inverse = self.apply_all(edit.changes);
        self.log(origin, format!("rétablissement : {}", edit.description));
        let description = edit.description.clone();
        self.undo.push(Edit {
            changes: inverse,
            ..edit
        });
        Some(description)
    }

    /// Remplace tout le morceau (nouveau, ouverture de fichier) : l'historique repart de zéro.
    pub fn replace_song(&mut self, origin: Origin, song: Song, description: impl Into<String>) {
        self.mixer = Mixer::new(song.channels);
        self.song = song;
        self.undo.clear();
        self.redo.clear();
        self.log(origin, description.into());
    }

    pub fn log(&mut self, origin: Origin, text: String) {
        self.journal.push(JournalEntry {
            time: SystemTime::now(),
            origin,
            text,
        });
    }

    /// Applique les remplacements dans l'ordre et renvoie leurs inverses, dans l'ordre inverse.
    fn apply_all(&mut self, changes: Vec<Change>) -> Vec<Change> {
        let mut inverse: Vec<Change> = changes
            .into_iter()
            .map(|c| c.apply(&mut self.song))
            .collect();
        inverse.reverse();
        inverse
    }

    // --- Constructeurs de modifications, avec vérifications ----------------------------------

    /// Remplacement d'un pattern existant, ou ajout d'un pattern juste après le dernier.
    pub fn set_pattern(&self, index: usize, pattern: Pattern) -> anyhow::Result<Change> {
        ensure!(
            index <= self.song.patterns.len(),
            "pattern {index} : on ne peut créer que le pattern suivant ({})",
            self.song.patterns.len()
        );
        ensure!(index < 128, "128 patterns au plus");
        ensure!(
            pattern.rows.len() == 64 && pattern.rows.iter().all(|r| r.len() == self.song.channels),
            "un pattern de .mod fait 64 lignes de {} voies",
            self.song.channels
        );
        Ok(Change::Pattern(index, Some(pattern)))
    }

    pub fn set_orders(&self, orders: &[u8]) -> anyhow::Result<Change> {
        ensure!(
            (1..=128).contains(&orders.len()),
            "la liste d'ordre compte de 1 à 128 positions"
        );
        if let Some(&p) = orders
            .iter()
            .find(|&&p| p as usize >= self.song.patterns.len())
        {
            anyhow::bail!(
                "pattern {p} inexistant (le morceau en a {})",
                self.song.patterns.len()
            );
        }
        let mut all = [0u8; 128];
        all[..orders.len()].copy_from_slice(orders);
        Ok(Change::Orders {
            length: orders.len() as u8,
            orders: all,
        })
    }

    pub fn set_sample(&self, number: usize, sample: Sample) -> anyhow::Result<Change> {
        ensure!(
            (1..=self.song.samples.len()).contains(&number),
            "sample {number} inexistant (1 à {})",
            self.song.samples.len()
        );
        Ok(Change::Sample(number - 1, Box::new(sample)))
    }
}

/// Fichier du journal, à côté du morceau : `morceau.mod.journal.txt`.
pub fn journal_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".journal.txt");
    PathBuf::from(name)
}

/// Le journal en texte, une ligne par entrée.
pub fn journal_text(editor: &Editor) -> String {
    editor
        .journal()
        .iter()
        .map(|e| {
            let secs = e
                .time
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let who = match e.origin {
                Origin::Agent => "agent",
                Origin::Keyboard => "clavier",
            };
            format!("{} {who:7} {}\n", format_time(secs), e.text)
        })
        .collect()
}

/// Heure UTC `AAAA-MM-JJ hh:mm:ss UTC`, sans dépendance de date.
fn format_time(secs: u64) -> String {
    let (days, rest) = (secs / 86400, secs % 86400);
    // Jours depuis 1970 → date civile (algorithme de Howard Hinnant).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::text::parse_cell;

    fn editor() -> Editor {
        Editor::new(Song::new("test"))
    }

    #[test]
    fn undo_and_redo_restore_the_song() {
        let mut ed = editor();
        let original = ed.song().clone();

        let mut p = ed.song().patterns[0].clone();
        p.rows[0][0] = parse_cell("C-2 01 ...").unwrap();
        let changes = vec![
            ed.set_pattern(0, p).unwrap(),
            Change::Title(*b"nouveau titre\0\0\0\0\0\0\0"),
        ];
        ed.apply(Origin::Agent, "première ligne", changes);
        let edited = ed.song().clone();
        assert_ne!(edited, original);

        assert_eq!(ed.undo(Origin::Keyboard).as_deref(), Some("première ligne"));
        assert_eq!(ed.song(), &original);
        assert_eq!(ed.redo(Origin::Keyboard).as_deref(), Some("première ligne"));
        assert_eq!(ed.song(), &edited);
        assert_eq!(ed.journal().len(), 3);
    }

    #[test]
    fn adding_a_pattern_and_orders_can_be_undone() {
        let mut ed = editor();
        let changes = vec![ed.set_pattern(1, Pattern::new(64, 4)).unwrap()];
        ed.apply(Origin::Agent, "pattern 01", changes);
        let changes = vec![ed.set_orders(&[0, 1, 1]).unwrap()];
        ed.apply(Origin::Agent, "ordre", changes);
        assert_eq!(ed.song().order_list(), &[0, 1, 1]);

        ed.undo(Origin::Agent);
        ed.undo(Origin::Agent);
        assert_eq!(ed.song().patterns.len(), 1);
        assert_eq!(ed.song().order_list(), &[0]);
    }

    #[test]
    fn rejects_invalid_changes() {
        let ed = editor();
        assert!(ed.set_pattern(5, Pattern::new(64, 4)).is_err());
        assert!(ed.set_pattern(0, Pattern::new(32, 4)).is_err());
        assert!(ed.set_orders(&[0, 3]).is_err());
        assert!(ed.set_orders(&[]).is_err());
        assert!(ed.set_sample(32, Sample::default()).is_err());
    }

    #[test]
    fn a_new_edit_clears_redo() {
        let mut ed = editor();
        ed.apply(Origin::Agent, "a", vec![Change::Title([1; 20])]);
        ed.undo(Origin::Agent);
        ed.apply(Origin::Agent, "b", vec![Change::Title([2; 20])]);
        assert!(ed.redo(Origin::Agent).is_none());
    }

    #[test]
    fn formats_dates() {
        assert_eq!(format_time(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_time(1_790_000_000), "2026-09-21 14:13:20 UTC");
    }

    #[test]
    fn journal_sits_next_to_the_song() {
        assert_eq!(
            journal_path(Path::new("/tmp/a.mod")),
            PathBuf::from("/tmp/a.mod.journal.txt")
        );
    }
}
