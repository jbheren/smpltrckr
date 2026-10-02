//! Modèle en mémoire d'un morceau.
//!
//! Le modèle garde les valeurs brutes du fichier (noms en octets, périodes, octets de fin),
//! pour qu'un `.mod` chargé puis réenregistré reste identique à l'octet près.
//! Le nombre de voies et de lignes est variable, pour laisser la porte ouverte au XM.

/// Une cellule de pattern : note (période Amiga), sample, effet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cell {
    /// Période Amiga (12 bits), 0 = pas de note.
    pub period: u16,
    /// Numéro de sample (1 à 31), 0 = pas de sample.
    pub sample: u8,
    /// Effet (0 à F).
    pub effect: u8,
    /// Paramètre de l'effet.
    pub param: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    /// `rows[ligne][voie]`.
    pub rows: Vec<Vec<Cell>>,
}

impl Pattern {
    pub fn new(rows: usize, channels: usize) -> Self {
        Self {
            rows: vec![vec![Cell::default(); channels]; rows],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    /// Nom brut (22 octets, pas forcément terminé par un zéro).
    pub name: [u8; 22],
    /// Longueur annoncée par l'en-tête, en mots de 16 bits.
    /// Peut dépasser `data` quand le fichier est tronqué.
    pub length_words: u16,
    /// Octet de finetune brut (seuls les 4 bits bas comptent : -8 à +7).
    pub finetune: u8,
    /// Volume (0 à 64).
    pub volume: u8,
    /// Début de boucle, en mots.
    pub loop_start: u16,
    /// Longueur de boucle, en mots (1 = pas de boucle).
    pub loop_length: u16,
    /// Données PCM 8 bits signées, telles que présentes dans le fichier.
    pub data: Vec<i8>,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            name: [0; 22],
            length_words: 0,
            finetune: 0,
            volume: 0,
            loop_start: 0,
            loop_length: 1,
            data: Vec::new(),
        }
    }
}

impl Sample {
    pub fn display_name(&self) -> String {
        text_from_bytes(&self.name)
    }

    /// Finetune signé, de -8 à +7.
    pub fn finetune(&self) -> i8 {
        ((self.finetune & 0x0F) as i8) << 4 >> 4
    }
}

/// Variante du format `.mod`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModKind {
    /// Soundtracker d'origine : 15 samples, pas de signature, 4 voies.
    Soundtracker15,
    /// ProTracker et dérivés : 31 samples et signature de 4 octets (`M.K.`, `8CHN`…).
    Tagged([u8; 4]),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Song {
    /// Titre brut (20 octets).
    pub title: [u8; 20],
    pub kind: ModKind,
    pub channels: usize,
    pub samples: Vec<Sample>,
    /// Nombre de positions jouées dans `orders`.
    pub song_length: u8,
    /// Octet de reprise (127 chez ProTracker, position de reprise ailleurs).
    pub restart: u8,
    /// Liste d'ordre complète, positions inutilisées comprises.
    pub orders: [u8; 128],
    pub patterns: Vec<Pattern>,
    /// Octets présents après les données des samples, conservés tels quels.
    pub trailing: Vec<u8>,
}

impl Song {
    pub fn display_title(&self) -> String {
        text_from_bytes(&self.title)
    }

    /// Les positions effectivement jouées de la liste d'ordre.
    pub fn order_list(&self) -> &[u8] {
        &self.orders[..(self.song_length as usize).min(128)]
    }
}

/// Texte lisible depuis un champ de nom : coupé au premier zéro, octets lus en Latin-1.
fn text_from_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect()
}
