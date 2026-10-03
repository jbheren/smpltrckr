//! Serveur MCP sur stdio : l'agent compose en travaillant directement sur des fichiers.
//!
//! Chaque outil traduit sa demande en modifications de l'éditeur (`Origin::Agent`) : tout
//! s'annule avec `undo` et laisse une trace dans le journal, enregistré à côté du `.mod`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::editor::{Change, Editor, Origin, journal_path, journal_text};
use crate::format::{protracker, text};
use crate::replayer::Replayer;
use crate::song::{Pattern, Song};
use crate::{note, reference, render, samples, wav};

type ToolResult = Result<String, String>;

struct State {
    editor: Editor,
    /// Fichier ouvert ou enregistré en dernier.
    path: Option<PathBuf>,
}

#[derive(Clone)]
struct Tracker {
    state: Arc<Mutex<State>>,
    tool_router: ToolRouter<Self>,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

// --- Paramètres des outils -------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
struct NewSong {
    /// Titre du morceau (20 caractères au plus).
    #[serde(default)]
    title: String,
}

#[derive(Deserialize, JsonSchema)]
struct FilePath {
    /// Chemin du fichier.
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct SavePath {
    /// Chemin du .mod. Par défaut : le dernier fichier ouvert ou enregistré.
    path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct Title {
    /// Nouveau titre (20 caractères au plus).
    title: String,
}

#[derive(Deserialize, JsonSchema)]
struct Tempo {
    /// Tempo en BPM (32 à 255).
    bpm: Option<u8>,
    /// Vitesse en ticks par ligne (1 à 31 ; 6 par défaut).
    speed: Option<u8>,
}

#[derive(Deserialize, JsonSchema)]
struct Orders {
    /// Numéros des patterns à jouer, dans l'ordre (1 à 128 positions), ex. [0, 0, 1, 2].
    orders: Vec<u8>,
}

#[derive(Deserialize, JsonSchema)]
struct PatternIndex {
    /// Numéro du pattern (0 = le premier).
    pattern: usize,
}

#[derive(Deserialize, JsonSchema)]
struct PatternWrite {
    /// Numéro du pattern. Le numéro qui suit le dernier pattern crée un nouveau pattern vide.
    pattern: usize,
    /// Lignes en notation texte, une par ligne : `NN | C-3 01 ... | ... .. ... | …`.
    /// Seules les lignes données sont modifiées ; une ligne peut donner moins de cellules que
    /// de voies (elles s'appliquent à partir de `first_voice`).
    rows: String,
    /// Première voie concernée (1 par défaut).
    first_voice: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct PatternCopy {
    from: usize,
    /// Pattern de destination : existant (remplacé) ou le numéro suivant le dernier (créé).
    to: usize,
}

#[derive(Deserialize, JsonSchema)]
struct Transpose {
    pattern: usize,
    /// Nombre de demi-tons, positif ou négatif.
    semitones: i32,
    /// Voies concernées (toutes par défaut), numérotées à partir de 1.
    voices: Option<Vec<usize>>,
}

#[derive(Deserialize, JsonSchema)]
struct Generate {
    /// Numéro du sample (1 à 31).
    sample: usize,
    /// sine, square, pulse, saw, triangle (cycle en boucle), noise, kick, snare, hihat.
    waveform: String,
    /// Longueur du cycle en octets pour les formes tonales (32 par défaut ; 64 = une octave
    /// plus bas, 16 = une octave plus haut).
    cycle: Option<usize>,
    /// Volume du sample, 0 à 64 (64 par défaut).
    volume: Option<u8>,
    /// Nom du sample (22 caractères au plus).
    name: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct LoadSample {
    /// Numéro du sample (1 à 31).
    sample: usize,
    /// Fichier WAV ou AIFF.
    path: String,
    /// Divise la fréquence d'échantillonnage par deux (pour les sons longs ou trop aigus).
    halve: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct SampleSet {
    sample: usize,
    name: Option<String>,
    /// 0 à 64.
    volume: Option<u8>,
    /// -8 à +7 (huitièmes de demi-ton).
    finetune: Option<i8>,
    /// Début de boucle en octets (pair).
    loop_start: Option<usize>,
    /// Longueur de boucle en octets (paire) ; 0 = pas de boucle.
    loop_length: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct MixSet {
    /// Voie, à partir de 1.
    voice: usize,
    mute: Option<bool>,
    solo: Option<bool>,
    /// 0.0 à 1.0.
    volume: Option<f32>,
}

#[derive(Deserialize, JsonSchema)]
struct RenderWav {
    /// Fichier WAV de sortie (avec stems : préfixe, complété par « -voie1.wav »…).
    path: String,
    /// Une piste mono par voie au lieu du mixage stéréo.
    stems: Option<bool>,
    /// Durée maximale en secondes (600 par défaut).
    max_seconds: Option<f64>,
}

#[derive(Deserialize, JsonSchema)]
struct History {
    /// Nombre de lignes du journal à afficher (20 par défaut).
    last: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct Topic {
    /// guide, effects, notes, scales ou chords.
    topic: String,
}

// --- Outils ----------------------------------------------------------------------------------

#[tool_router(router = tool_router)]
impl Tracker {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                editor: Editor::new(Song::new("")),
                path: None,
            })),
            tool_router: Self::tool_router(),
        }
    }

    /// Exécute `f` sur l'état partagé.
    fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        f(&mut self.state.lock().unwrap())
    }

    /// Applique une modification de l'agent, décrite par `description`.
    fn edit(
        &self,
        description: String,
        build: impl FnOnce(&Editor) -> anyhow::Result<Vec<Change>>,
    ) -> ToolResult {
        self.with(|s| {
            let changes = build(&s.editor).map_err(err)?;
            s.editor.apply(Origin::Agent, description.clone(), changes);
            Ok(format!("ok : {description}"))
        })
    }

    #[tool(description = "Vérifie que le serveur smpltrckr répond et donne sa version.")]
    async fn ping(&self) -> String {
        format!("smpltrckr {}", env!("CARGO_PKG_VERSION"))
    }

    #[tool(
        description = "Aide-mémoire : guide (comment composer ici, à lire en premier), effects (effets ProTracker), notes, scales (gammes), chords (accords en arpège)."
    )]
    async fn reference(&self, Parameters(p): Parameters<Topic>) -> ToolResult {
        reference::get(&p.topic).map(str::to_string).ok_or_else(|| {
            format!(
                "sujet inconnu {:?} : {}",
                p.topic,
                reference::TOPICS.join(", ")
            )
        })
    }

    #[tool(
        description = "Crée un nouveau morceau vide (4 voies, un pattern vide). L'historique repart de zéro."
    )]
    async fn song_new(&self, Parameters(p): Parameters<NewSong>) -> ToolResult {
        self.with(|s| {
            let description = format!("nouveau morceau « {} »", p.title);
            s.editor
                .replace_song(Origin::Agent, Song::new(&p.title), description);
            s.path = None;
        });
        Ok(format!(
            "morceau « {} » créé : 4 voies, pattern 00 vide, ordre [0]",
            p.title
        ))
    }

    #[tool(description = "Ouvre un fichier .mod.")]
    async fn song_load(&self, Parameters(p): Parameters<FilePath>) -> ToolResult {
        let path = PathBuf::from(&p.path);
        let data =
            std::fs::read(&path).map_err(|e| format!("lecture de {} : {e}", path.display()))?;
        let song = protracker::read(&data).map_err(err)?;
        let summary = text::song_to_text(&song, []);
        self.with(|s| {
            s.editor.replace_song(
                Origin::Agent,
                song,
                format!("ouverture de {}", path.display()),
            );
            s.path = Some(path);
        });
        Ok(summary)
    }

    #[tool(
        description = "Enregistre le morceau en .mod, et le journal des modifications à côté (.journal.txt)."
    )]
    async fn song_save(&self, Parameters(p): Parameters<SavePath>) -> ToolResult {
        self.with(|s| {
            let path = p
                .path
                .map(PathBuf::from)
                .or_else(|| s.path.clone())
                .ok_or("aucun chemin : préciser path")?;
            std::fs::write(&path, protracker::write(s.editor.song()))
                .map_err(|e| format!("écriture de {} : {e}", path.display()))?;
            s.editor.log(
                Origin::Agent,
                format!("enregistrement dans {}", path.display()),
            );
            let journal = journal_path(&path);
            std::fs::write(&journal, journal_text(&s.editor))
                .map_err(|e| format!("écriture du journal : {e}"))?;
            s.path = Some(path.clone());
            Ok(format!(
                "enregistré : {} (journal : {})",
                path.display(),
                journal.display()
            ))
        })
    }

    #[tool(
        description = "Résumé du morceau : titre, format, liste d'ordre, samples, nombre de patterns et durée."
    )]
    async fn song_info(&self) -> ToolResult {
        let song = self.with(|s| s.editor.song().clone());
        let seconds = render::duration(Arc::new(song.clone()), 1200.0);
        Ok(format!(
            "{}durée    : {}:{:04.1}\n",
            text::song_to_text(&song, []),
            (seconds / 60.0) as u32,
            seconds % 60.0
        ))
    }

    #[tool(description = "Change le titre du morceau.")]
    async fn song_set_title(&self, Parameters(p): Parameters<Title>) -> ToolResult {
        let title = Song::new(&p.title).title;
        self.edit(format!("titre « {} »", p.title), |_| {
            Ok(vec![Change::Title(title)])
        })
    }

    #[tool(
        description = "Fixe le tempo (BPM) et/ou la vitesse au début du morceau : écrit ou met à jour les Fxx de la ligne 00 du premier pattern joué."
    )]
    async fn song_set_tempo(&self, Parameters(p): Parameters<Tempo>) -> ToolResult {
        let parts: Vec<String> = [
            p.bpm.map(|b| format!("{b} BPM")),
            p.speed.map(|v| format!("vitesse {v}")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let description = format!("tempo : {}", parts.join(", "));
        self.edit(description, |ed| {
            Ok(vec![ed.set_start_tempo(p.bpm, p.speed)?])
        })
    }

    #[tool(description = "Définit la liste d'ordre : les numéros de patterns joués, dans l'ordre.")]
    async fn order_set(&self, Parameters(p): Parameters<Orders>) -> ToolResult {
        let list: Vec<String> = p.orders.iter().map(|o| format!("{o:02}")).collect();
        self.edit(format!("ordre {}", list.join(" ")), |ed| {
            Ok(vec![ed.set_orders(&p.orders)?])
        })
    }

    #[tool(description = "Lit un pattern en notation texte (64 lignes).")]
    async fn pattern_get(&self, Parameters(p): Parameters<PatternIndex>) -> ToolResult {
        self.with(|s| {
            let song = s.editor.song();
            let pattern = song.patterns.get(p.pattern).ok_or_else(|| {
                format!(
                    "pattern {} inexistant (0 à {})",
                    p.pattern,
                    song.patterns.len() - 1
                )
            })?;
            Ok(text::pattern_to_text(p.pattern, pattern))
        })
    }

    #[tool(
        description = "Écrit des lignes dans un pattern (notation texte). Seules les lignes données changent. Le numéro qui suit le dernier pattern crée un pattern."
    )]
    async fn pattern_write(&self, Parameters(p): Parameters<PatternWrite>) -> ToolResult {
        let first = p.first_voice.unwrap_or(1);
        let mut count = 0;
        let result = self.edit(format!("pattern {:02} : lignes écrites", p.pattern), |ed| {
            let song = ed.song();
            anyhow::ensure!(
                (1..=song.channels).contains(&first),
                "voie {first} inexistante (1 à {})",
                song.channels
            );
            let mut pattern = song
                .patterns
                .get(p.pattern)
                .cloned()
                .unwrap_or_else(|| Pattern::new(64, song.channels));
            for line in p
                .rows
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
            {
                let (row, cells) = text::parse_row(line)?;
                anyhow::ensure!(row < 64, "ligne {row} hors du pattern (0 à 63)");
                anyhow::ensure!(
                    first - 1 + cells.len() <= song.channels,
                    "ligne {row:02} : {} cellules à partir de la voie {first}, pour {} voies",
                    cells.len(),
                    song.channels
                );
                for (k, cell) in cells.into_iter().enumerate() {
                    pattern.rows[row][first - 1 + k] = cell;
                }
                count += 1;
            }
            Ok(vec![ed.set_pattern(p.pattern, pattern)?])
        })?;
        Ok(format!("{result} ({count} lignes)"))
    }

    #[tool(description = "Vide un pattern.")]
    async fn pattern_clear(&self, Parameters(p): Parameters<PatternIndex>) -> ToolResult {
        self.edit(format!("pattern {:02} vidé", p.pattern), |ed| {
            anyhow::ensure!(
                p.pattern < ed.song().patterns.len(),
                "pattern {} inexistant",
                p.pattern
            );
            Ok(vec![ed.set_pattern(
                p.pattern,
                Pattern::new(64, ed.song().channels),
            )?])
        })
    }

    #[tool(
        description = "Copie un pattern vers un autre (existant, ou nouveau s'il suit le dernier)."
    )]
    async fn pattern_copy(&self, Parameters(p): Parameters<PatternCopy>) -> ToolResult {
        self.edit(
            format!("pattern {:02} copié vers {:02}", p.from, p.to),
            |ed| {
                let source = ed
                    .song()
                    .patterns
                    .get(p.from)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("pattern {} inexistant", p.from))?;
                Ok(vec![ed.set_pattern(p.to, source)?])
            },
        )
    }

    #[tool(
        description = "Transpose les notes d'un pattern de n demi-tons (toutes les voies, ou certaines)."
    )]
    async fn pattern_transpose(&self, Parameters(p): Parameters<Transpose>) -> ToolResult {
        let description = format!(
            "pattern {:02} transposé de {:+} demi-tons",
            p.pattern, p.semitones
        );
        self.edit(description, |ed| {
            let song = ed.song();
            let mut pattern = song
                .patterns
                .get(p.pattern)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("pattern {} inexistant", p.pattern))?;
            let voices = p
                .voices
                .clone()
                .unwrap_or_else(|| (1..=song.channels).collect());
            for (row, cells) in pattern.rows.iter_mut().enumerate() {
                for &v in &voices {
                    let cell = cells
                        .get_mut(v.wrapping_sub(1))
                        .ok_or_else(|| anyhow::anyhow!("voie {v} inexistante"))?;
                    if cell.period == 0 {
                        continue;
                    }
                    let index = note::PERIODS
                        .iter()
                        .position(|&x| x == cell.period)
                        .ok_or_else(|| {
                            anyhow::anyhow!("ligne {row:02} voie {v} : note hors des octaves 1 à 3")
                        })?;
                    let target = index as i32 + p.semitones;
                    anyhow::ensure!(
                        (0..36).contains(&target),
                        "ligne {row:02} voie {v} : {} transposé sortirait des octaves 1 à 3",
                        note::name(index)
                    );
                    cell.period = note::PERIODS[target as usize];
                }
            }
            Ok(vec![ed.set_pattern(p.pattern, pattern)?])
        })
    }

    #[tool(
        description = "Liste les samples non vides : numéro, nom, taille, volume, finetune, boucle."
    )]
    async fn sample_list(&self) -> String {
        self.with(|s| {
            let lines: Vec<String> = s
                .editor
                .song()
                .samples
                .iter()
                .enumerate()
                .filter(|(_, x)| !x.data.is_empty())
                .map(|(i, x)| {
                    let looped = if x.loop_length > 1 {
                        format!(
                            ", boucle {}+{}",
                            x.loop_start as u32 * 2,
                            x.loop_length as u32 * 2
                        )
                    } else {
                        String::new()
                    };
                    format!(
                        "{:02} {:<22} {:>6} octets, vol {:>2}, finetune {:+}{looped}",
                        i + 1,
                        x.display_name(),
                        x.data.len(),
                        x.volume,
                        x.finetune()
                    )
                })
                .collect();
            if lines.is_empty() {
                "aucun sample".to_string()
            } else {
                lines.join("\n")
            }
        })
    }

    #[tool(
        description = "Génère un sample : forme d'onde en boucle (sine, square, pulse, saw, triangle, noise) ou percussion (kick, snare, hihat, à jouer en C-3)."
    )]
    async fn sample_generate(&self, Parameters(p): Parameters<Generate>) -> ToolResult {
        let mut sample = samples::generate(&p.waveform, p.cycle.unwrap_or(32)).map_err(err)?;
        sample.volume = p.volume.unwrap_or(64).min(64);
        if let Some(name) = &p.name {
            sample.set_name(name);
        }
        let size = sample.data.len();
        let description = format!("sample {:02} : {} généré", p.sample, p.waveform);
        let result = self.edit(description, |ed| Ok(vec![ed.set_sample(p.sample, sample)?]))?;
        Ok(format!("{result} ({size} octets)"))
    }

    #[tool(
        description = "Charge un fichier WAV ou AIFF dans un sample (mono, 8 bits, sans rééchantillonnage ; 128 Ko au plus)."
    )]
    async fn sample_load(&self, Parameters(p): Parameters<LoadSample>) -> ToolResult {
        let report = samples::import(Path::new(&p.path), p.halve.unwrap_or(false)).map_err(err)?;
        let size = report.sample.data.len();
        let description = format!("sample {:02} : {} chargé", p.sample, p.path);
        let result = self.edit(description, |ed| {
            Ok(vec![ed.set_sample(p.sample, report.sample)?])
        })?;
        let mut notes = vec![format!("{size} octets, {} Hz", report.rate)];
        if report.truncated {
            notes.push("TRONQUÉ à 128 Ko (essayer halve)".to_string());
        }
        notes.push(match report.natural_note {
            Some(n) => format!("hauteur d'origine en {n}"),
            None if report.rate as f64 > note::period_to_hz(note::PERIODS[35]) => {
                "hauteur d'origine au-dessus de B-3 : il sonnera plus grave (essayer halve)"
                    .to_string()
            }
            None => "hauteur d'origine sous C-1 : il sonnera plus aigu".to_string(),
        });
        Ok(format!("{result} ({})", notes.join(", ")))
    }

    #[tool(description = "Règle un sample : nom, volume, finetune, boucle (en octets).")]
    async fn sample_set(&self, Parameters(p): Parameters<SampleSet>) -> ToolResult {
        self.edit(format!("sample {:02} réglé", p.sample), |ed| {
            let mut sample = ed
                .song()
                .samples
                .get(p.sample.wrapping_sub(1))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("sample {} inexistant (1 à 31)", p.sample))?;
            if let Some(name) = &p.name {
                sample.set_name(name);
            }
            if let Some(v) = p.volume {
                anyhow::ensure!(v <= 64, "volume de 0 à 64");
                sample.volume = v;
            }
            if let Some(f) = p.finetune {
                anyhow::ensure!((-8..=7).contains(&f), "finetune de -8 à +7");
                sample.finetune = (f as u8) & 0x0F;
            }
            if p.loop_start.is_some() || p.loop_length.is_some() {
                let start = p.loop_start.unwrap_or(sample.loop_start as usize * 2);
                let length = p.loop_length.unwrap_or(sample.loop_length as usize * 2);
                if length < 4 {
                    (sample.loop_start, sample.loop_length) = (0, 1);
                } else {
                    anyhow::ensure!(
                        start + length <= sample.data.len(),
                        "boucle {start}+{length} au-delà du sample ({} octets)",
                        sample.data.len()
                    );
                    (sample.loop_start, sample.loop_length) =
                        ((start / 2) as u16, (length / 2) as u16);
                }
            }
            Ok(vec![ed.set_sample(p.sample, sample)?])
        })
    }

    #[tool(
        description = "Mixage de la session (non enregistré dans le .mod, appliqué au rendu WAV) : coupe, solo ou volume d'une voie."
    )]
    async fn mix_set(&self, Parameters(p): Parameters<MixSet>) -> ToolResult {
        self.with(|s| {
            let mixer = &mut s.editor.mixer;
            let v = p.voice.wrapping_sub(1);
            if v >= mixer.mute.len() {
                return Err(format!(
                    "voie {} inexistante (1 à {})",
                    p.voice,
                    mixer.mute.len()
                ));
            }
            if let Some(m) = p.mute {
                mixer.mute[v] = m;
            }
            if let Some(solo) = p.solo {
                mixer.solo[v] = solo;
            }
            if let Some(volume) = p.volume {
                mixer.volume[v] = volume.clamp(0.0, 1.0);
            }
            let state = format!(
                "voie {} : coupée {}, solo {}, volume {:.2}",
                p.voice, mixer.mute[v], mixer.solo[v], mixer.volume[v]
            );
            s.editor.log(Origin::Agent, format!("mixage : {state}"));
            Ok(state)
        })
    }

    #[tool(
        description = "Rend le morceau en WAV 16 bits 48 kHz (stéréo, ou une piste par voie), avec le mixage de la session."
    )]
    async fn render_wav(&self, Parameters(p): Parameters<RenderWav>) -> ToolResult {
        let (song, mixer) =
            self.with(|s| (Arc::new(s.editor.song().clone()), s.editor.mixer.clone()));
        let (rate, max) = (48000, p.max_seconds.unwrap_or(600.0));
        let mut replayer = Replayer::new(song.clone(), rate);
        replayer.mixer = mixer;
        let path = PathBuf::from(&p.path);
        let (frames, files) = if p.stems.unwrap_or(false) {
            let tracks = render::voices(&mut replayer, song.channels, rate, max);
            let stem = path.with_extension("");
            let mut files = Vec::new();
            for (v, track) in tracks.iter().enumerate() {
                let file = PathBuf::from(format!("{}-voie{}.wav", stem.display(), v + 1));
                wav::write(&file, track, 1, rate).map_err(err)?;
                files.push(file.display().to_string());
            }
            (tracks[0].len(), files)
        } else {
            let out = render::stereo(&mut replayer, rate, max);
            wav::write(&path, &out, 2, rate).map_err(err)?;
            (out.len() / 2, vec![path.display().to_string()])
        };
        let seconds = frames as f64 / rate as f64;
        let limit = if replayer.ended() {
            ""
        } else {
            " (durée maximale atteinte)"
        };
        self.with(|s| {
            s.editor
                .log(Origin::Agent, format!("rendu WAV : {}", files.join(", ")))
        });
        Ok(format!(
            "{} — {}:{:04.1}{limit}",
            files.join(", "),
            (seconds / 60.0) as u32,
            seconds % 60.0
        ))
    }

    #[tool(description = "Annule la dernière modification.")]
    async fn undo(&self) -> ToolResult {
        self.with(|s| {
            s.editor
                .undo(Origin::Agent)
                .map(|d| format!("annulé : {d}"))
                .ok_or_else(|| "rien à annuler".into())
        })
    }

    #[tool(description = "Rétablit la dernière modification annulée.")]
    async fn redo(&self) -> ToolResult {
        self.with(|s| {
            s.editor
                .redo(Origin::Agent)
                .map(|d| format!("rétabli : {d}"))
                .ok_or_else(|| "rien à rétablir".into())
        })
    }

    #[tool(description = "Dernières lignes du journal des modifications.")]
    async fn history(&self, Parameters(p): Parameters<History>) -> String {
        self.with(|s| {
            let text = journal_text(&s.editor);
            let lines: Vec<&str> = text.lines().collect();
            lines[lines.len().saturating_sub(p.last.unwrap_or(20))..].join("\n")
        })
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Tracker {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("smpltrckr", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "smpltrckr : tracker façon ProTracker (.mod, 4 voies, 31 samples, patterns de 64 lignes). \
                 Lire d'abord reference(topic=\"guide\"). Les patterns s'écrivent en notation texte : \
                 `NN | C-3 01 A04 | ... .. ... | …` (note, sample en décimal, effet en hexadécimal). \
                 Toute modification s'annule avec undo.",
            )
    }
}

pub fn run() -> anyhow::Result<()> {
    tokio::runtime::Runtime::new()?.block_on(async {
        let service = Tracker::new().serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        Ok(())
    })
}
