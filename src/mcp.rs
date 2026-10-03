//! MCP server over stdio: the agent composes by working directly on files.
//!
//! Each tool turns its request into editor changes (`Origin::Agent`): everything can be undone
//! with `undo` and leaves a line in the journal, saved next to the `.mod`. The agent side
//! speaks English, whatever the interface language.

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
    /// File opened or saved last.
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

fn minutes(seconds: f64) -> String {
    format!("{}:{:04.1}", (seconds / 60.0) as u32, seconds % 60.0)
}

// --- Tool parameters -------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
struct NewSong {
    /// Song title (20 characters at most).
    #[serde(default)]
    title: String,
}

#[derive(Deserialize, JsonSchema)]
struct FilePath {
    /// File path.
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct SavePath {
    /// Path of the .mod. Defaults to the file opened or saved last.
    path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct Title {
    /// New title (20 characters at most).
    title: String,
}

#[derive(Deserialize, JsonSchema)]
struct Tempo {
    /// Tempo in BPM (32 to 255).
    bpm: Option<u8>,
    /// Speed in ticks per row (1 to 31; 6 by default).
    speed: Option<u8>,
}

#[derive(Deserialize, JsonSchema)]
struct Orders {
    /// Pattern numbers to play, in order (1 to 128 positions), e.g. [0, 0, 1, 2].
    orders: Vec<u8>,
}

#[derive(Deserialize, JsonSchema)]
struct PatternIndex {
    /// Pattern number (0 = the first one).
    pattern: usize,
}

#[derive(Deserialize, JsonSchema)]
struct PatternWrite {
    /// Pattern number. The number right after the last pattern creates a new empty pattern.
    pattern: usize,
    /// Rows in text notation, one per line: `NN | C-3 01 ... | ... .. ... | …`.
    /// Only the given rows change; a row may give fewer cells than there are voices (they
    /// apply from `first_voice` on).
    rows: String,
    /// First voice concerned (1 by default).
    first_voice: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct PatternCopy {
    from: usize,
    /// Target pattern: an existing one (replaced) or the number right after the last (created).
    to: usize,
}

#[derive(Deserialize, JsonSchema)]
struct Transpose {
    pattern: usize,
    /// Number of semitones, positive or negative.
    semitones: i32,
    /// Voices concerned (all by default), numbered from 1.
    voices: Option<Vec<usize>>,
}

#[derive(Deserialize, JsonSchema)]
struct Generate {
    /// Sample number (1 to 31).
    sample: usize,
    /// sine, square, pulse, saw, triangle (looped cycle), noise, kick, snare, hihat.
    waveform: String,
    /// Cycle length in bytes for tonal waveforms (32 by default; 64 = one octave lower,
    /// 16 = one octave higher).
    cycle: Option<usize>,
    /// Sample volume, 0 to 64 (64 by default).
    volume: Option<u8>,
    /// Sample name (22 characters at most).
    name: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct LoadSample {
    /// Sample number (1 to 31).
    sample: usize,
    /// WAV or AIFF file.
    path: String,
    /// Halves the sample rate (for long or very high-pitched sounds).
    halve: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct SampleSet {
    sample: usize,
    name: Option<String>,
    /// 0 to 64.
    volume: Option<u8>,
    /// -8 to +7 (eighths of a semitone).
    finetune: Option<i8>,
    /// Loop start in bytes (even).
    loop_start: Option<usize>,
    /// Loop length in bytes (even); 0 = no loop.
    loop_length: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct MixSet {
    /// Voice, from 1.
    voice: usize,
    mute: Option<bool>,
    solo: Option<bool>,
    /// 0.0 to 1.0.
    volume: Option<f32>,
}

#[derive(Deserialize, JsonSchema)]
struct RenderWav {
    /// Output WAV file (with stems: a prefix, completed by "-voice1.wav"…).
    path: String,
    /// One mono track per voice instead of the stereo mix.
    stems: Option<bool>,
    /// Maximum duration in seconds (600 by default).
    max_seconds: Option<f64>,
}

#[derive(Deserialize, JsonSchema)]
struct History {
    /// Number of journal lines to show (20 by default).
    last: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct Topic {
    /// guide, effects, notes, scales or chords.
    topic: String,
}

// --- Tools -----------------------------------------------------------------------------------

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

    /// Runs `f` on the shared state.
    fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        f(&mut self.state.lock().unwrap())
    }

    /// Applies a change from the agent, described by `description`.
    fn edit(
        &self,
        description: String,
        build: impl FnOnce(&Editor) -> anyhow::Result<Vec<Change>>,
    ) -> ToolResult {
        self.with(|s| {
            let changes = build(&s.editor).map_err(err)?;
            s.editor.apply(Origin::Agent, description.clone(), changes);
            Ok(format!("ok: {description}"))
        })
    }

    #[tool(description = "Checks that the smpltrckr server answers and gives its version.")]
    async fn ping(&self) -> String {
        format!("smpltrckr {}", env!("CARGO_PKG_VERSION"))
    }

    #[tool(
        description = "Cheat sheet: guide (how to compose here, read it first), effects (ProTracker effects), notes, scales, chords (arpeggio chords)."
    )]
    async fn reference(&self, Parameters(p): Parameters<Topic>) -> ToolResult {
        reference::get(&p.topic).map(str::to_string).ok_or_else(|| {
            format!(
                "unknown topic {:?}: {}",
                p.topic,
                reference::TOPICS.join(", ")
            )
        })
    }

    #[tool(
        description = "Creates a new empty song (4 voices, one empty pattern). The history starts over."
    )]
    async fn song_new(&self, Parameters(p): Parameters<NewSong>) -> ToolResult {
        self.with(|s| {
            let description = format!("new song \"{}\"", p.title);
            s.editor
                .replace_song(Origin::Agent, Song::new(&p.title), description);
            s.path = None;
        });
        Ok(format!(
            "song \"{}\" created: 4 voices, empty pattern 00, orders [0]",
            p.title
        ))
    }

    #[tool(description = "Opens a .mod file.")]
    async fn song_load(&self, Parameters(p): Parameters<FilePath>) -> ToolResult {
        let path = PathBuf::from(&p.path);
        let data =
            std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let song = protracker::read(&data).map_err(err)?;
        let summary = text::song_to_text(&song, []);
        self.with(|s| {
            s.editor
                .replace_song(Origin::Agent, song, format!("opened {}", path.display()));
            s.path = Some(path);
        });
        Ok(summary)
    }

    #[tool(
        description = "Saves the song as a .mod, with the change journal next to it (.journal.txt)."
    )]
    async fn song_save(&self, Parameters(p): Parameters<SavePath>) -> ToolResult {
        self.with(|s| {
            let path = p
                .path
                .map(PathBuf::from)
                .or_else(|| s.path.clone())
                .ok_or("no path: give one with path")?;
            std::fs::write(&path, protracker::write(s.editor.song()))
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            s.editor
                .log(Origin::Agent, format!("saved to {}", path.display()));
            let journal = journal_path(&path);
            std::fs::write(&journal, journal_text(&s.editor))
                .map_err(|e| format!("cannot write the journal: {e}"))?;
            s.path = Some(path.clone());
            Ok(format!(
                "saved: {} (journal: {})",
                path.display(),
                journal.display()
            ))
        })
    }

    #[tool(
        description = "Song summary: title, format, order list, samples, pattern count and duration."
    )]
    async fn song_info(&self) -> ToolResult {
        let song = self.with(|s| s.editor.song().clone());
        let seconds = render::duration(Arc::new(song.clone()), 1200.0);
        Ok(format!(
            "{}duration : {}\n",
            text::song_to_text(&song, []),
            minutes(seconds)
        ))
    }

    #[tool(description = "Changes the song title.")]
    async fn song_set_title(&self, Parameters(p): Parameters<Title>) -> ToolResult {
        let title = Song::new(&p.title).title;
        self.edit(format!("title \"{}\"", p.title), |_| {
            Ok(vec![Change::Title(title)])
        })
    }

    #[tool(
        description = "Sets the tempo (BPM) and/or speed at the start of the song: writes or updates the Fxx of row 00 of the first pattern played."
    )]
    async fn song_set_tempo(&self, Parameters(p): Parameters<Tempo>) -> ToolResult {
        let parts: Vec<String> = [
            p.bpm.map(|b| format!("{b} BPM")),
            p.speed.map(|v| format!("speed {v}")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let description = format!("tempo: {}", parts.join(", "));
        self.edit(description, |ed| {
            Ok(vec![ed.set_start_tempo(p.bpm, p.speed)?])
        })
    }

    #[tool(description = "Sets the order list: the pattern numbers played, in order.")]
    async fn order_set(&self, Parameters(p): Parameters<Orders>) -> ToolResult {
        let list: Vec<String> = p.orders.iter().map(|o| format!("{o:02}")).collect();
        self.edit(format!("orders {}", list.join(" ")), |ed| {
            Ok(vec![ed.set_orders(&p.orders)?])
        })
    }

    #[tool(description = "Reads a pattern in text notation (64 rows).")]
    async fn pattern_get(&self, Parameters(p): Parameters<PatternIndex>) -> ToolResult {
        self.with(|s| {
            let song = s.editor.song();
            let pattern = song.patterns.get(p.pattern).ok_or_else(|| {
                format!(
                    "no pattern {} (0 to {})",
                    p.pattern,
                    song.patterns.len() - 1
                )
            })?;
            Ok(text::pattern_to_text(p.pattern, pattern))
        })
    }

    #[tool(
        description = "Writes rows into a pattern (text notation). Only the given rows change. The number right after the last pattern creates a pattern."
    )]
    async fn pattern_write(&self, Parameters(p): Parameters<PatternWrite>) -> ToolResult {
        let first = p.first_voice.unwrap_or(1);
        let mut count = 0;
        let result = self.edit(format!("pattern {:02}: rows written", p.pattern), |ed| {
            let song = ed.song();
            anyhow::ensure!(
                (1..=song.channels).contains(&first),
                "no voice {first} (1 to {})",
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
                anyhow::ensure!(row < 64, "row {row} is outside the pattern (0 to 63)");
                anyhow::ensure!(
                    first - 1 + cells.len() <= song.channels,
                    "row {row:02}: {} cells from voice {first}, for {} voices",
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
        Ok(format!("{result} ({count} rows)"))
    }

    #[tool(description = "Clears a pattern.")]
    async fn pattern_clear(&self, Parameters(p): Parameters<PatternIndex>) -> ToolResult {
        self.edit(format!("pattern {:02} cleared", p.pattern), |ed| {
            anyhow::ensure!(
                p.pattern < ed.song().patterns.len(),
                "no pattern {}",
                p.pattern
            );
            Ok(vec![ed.set_pattern(
                p.pattern,
                Pattern::new(64, ed.song().channels),
            )?])
        })
    }

    #[tool(
        description = "Copies a pattern onto another one (existing, or new if it follows the last)."
    )]
    async fn pattern_copy(&self, Parameters(p): Parameters<PatternCopy>) -> ToolResult {
        self.edit(
            format!("pattern {:02} copied to {:02}", p.from, p.to),
            |ed| {
                let source = ed
                    .song()
                    .patterns
                    .get(p.from)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no pattern {}", p.from))?;
                Ok(vec![ed.set_pattern(p.to, source)?])
            },
        )
    }

    #[tool(
        description = "Transposes the notes of a pattern by n semitones (every voice, or some)."
    )]
    async fn pattern_transpose(&self, Parameters(p): Parameters<Transpose>) -> ToolResult {
        let description = format!(
            "pattern {:02} transposed by {:+} semitones",
            p.pattern, p.semitones
        );
        self.edit(description, |ed| {
            let song = ed.song();
            let mut pattern = song
                .patterns
                .get(p.pattern)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no pattern {}", p.pattern))?;
            let voices = p
                .voices
                .clone()
                .unwrap_or_else(|| (1..=song.channels).collect());
            for (row, cells) in pattern.rows.iter_mut().enumerate() {
                for &v in &voices {
                    let cell = cells
                        .get_mut(v.wrapping_sub(1))
                        .ok_or_else(|| anyhow::anyhow!("no voice {v}"))?;
                    if cell.period == 0 {
                        continue;
                    }
                    let index = note::PERIODS
                        .iter()
                        .position(|&x| x == cell.period)
                        .ok_or_else(|| {
                            anyhow::anyhow!("row {row:02} voice {v}: note outside octaves 1 to 3")
                        })?;
                    let target = index as i32 + p.semitones;
                    anyhow::ensure!(
                        (0..36).contains(&target),
                        "row {row:02} voice {v}: {} would leave octaves 1 to 3",
                        note::name(index)
                    );
                    cell.period = note::PERIODS[target as usize];
                }
            }
            Ok(vec![ed.set_pattern(p.pattern, pattern)?])
        })
    }

    #[tool(
        description = "Lists the non-empty samples: number, name, size, volume, finetune, loop."
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
                            ", loop {}+{}",
                            x.loop_start as u32 * 2,
                            x.loop_length as u32 * 2
                        )
                    } else {
                        String::new()
                    };
                    format!(
                        "{:02} {:<22} {:>6} bytes, vol {:>2}, finetune {:+}{looped}",
                        i + 1,
                        x.display_name(),
                        x.data.len(),
                        x.volume,
                        x.finetune()
                    )
                })
                .collect();
            if lines.is_empty() {
                "no samples".to_string()
            } else {
                lines.join("\n")
            }
        })
    }

    #[tool(
        description = "Generates a sample: looped waveform (sine, square, pulse, saw, triangle, noise) or drum (kick, snare, hihat, to be played at C-3)."
    )]
    async fn sample_generate(&self, Parameters(p): Parameters<Generate>) -> ToolResult {
        let mut sample = samples::generate(&p.waveform, p.cycle.unwrap_or(32)).map_err(err)?;
        sample.volume = p.volume.unwrap_or(64).min(64);
        if let Some(name) = &p.name {
            sample.set_name(name);
        }
        let size = sample.data.len();
        let description = format!("sample {:02}: {} generated", p.sample, p.waveform);
        let result = self.edit(description, |ed| Ok(vec![ed.set_sample(p.sample, sample)?]))?;
        Ok(format!("{result} ({size} bytes)"))
    }

    #[tool(
        description = "Loads a WAV or AIFF file into a sample (mono, 8 bits, no resampling; 128 KB at most)."
    )]
    async fn sample_load(&self, Parameters(p): Parameters<LoadSample>) -> ToolResult {
        let report = samples::import(Path::new(&p.path), p.halve.unwrap_or(false)).map_err(err)?;
        let size = report.sample.data.len();
        let description = format!("sample {:02}: {} loaded", p.sample, p.path);
        let result = self.edit(description, |ed| {
            Ok(vec![ed.set_sample(p.sample, report.sample)?])
        })?;
        let mut notes = vec![format!("{size} bytes, {} Hz", report.rate)];
        if report.truncated {
            notes.push("TRUNCATED to 128 KB (try halve)".to_string());
        }
        notes.push(match report.natural_note {
            Some(n) => format!("original pitch at {n}"),
            None if report.rate as f64 > note::period_to_hz(note::PERIODS[35]) => {
                "original pitch above B-3: it will sound lower (try halve)".to_string()
            }
            None => "original pitch below C-1: it will sound higher".to_string(),
        });
        Ok(format!("{result} ({})", notes.join(", ")))
    }

    #[tool(description = "Tunes a sample: name, volume, finetune, loop (in bytes).")]
    async fn sample_set(&self, Parameters(p): Parameters<SampleSet>) -> ToolResult {
        self.edit(format!("sample {:02} tuned", p.sample), |ed| {
            let mut sample = ed
                .song()
                .samples
                .get(p.sample.wrapping_sub(1))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no sample {} (1 to 31)", p.sample))?;
            if let Some(name) = &p.name {
                sample.set_name(name);
            }
            if let Some(v) = p.volume {
                anyhow::ensure!(v <= 64, "volume from 0 to 64");
                sample.volume = v;
            }
            if let Some(f) = p.finetune {
                anyhow::ensure!((-8..=7).contains(&f), "finetune from -8 to +7");
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
                        "loop {start}+{length} goes past the sample ({} bytes)",
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
        description = "Session mix (not saved in the .mod, applied to WAV renders): mute, solo or volume of a voice."
    )]
    async fn mix_set(&self, Parameters(p): Parameters<MixSet>) -> ToolResult {
        self.with(|s| {
            let mixer = &mut s.editor.mixer;
            let v = p.voice.wrapping_sub(1);
            if v >= mixer.mute.len() {
                return Err(format!("no voice {} (1 to {})", p.voice, mixer.mute.len()));
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
                "voice {}: muted {}, solo {}, volume {:.2}",
                p.voice, mixer.mute[v], mixer.solo[v], mixer.volume[v]
            );
            s.editor.log(Origin::Agent, format!("mix: {state}"));
            Ok(state)
        })
    }

    #[tool(
        description = "Renders the song to a 16-bit 48 kHz WAV (stereo, or one track per voice), with the session mix."
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
                let file = PathBuf::from(format!("{}-voice{}.wav", stem.display(), v + 1));
                wav::write(&file, track, 1, rate).map_err(err)?;
                files.push(file.display().to_string());
            }
            (tracks[0].len(), files)
        } else {
            let out = render::stereo(&mut replayer, rate, max);
            wav::write(&path, &out, 2, rate).map_err(err)?;
            (out.len() / 2, vec![path.display().to_string()])
        };
        let limit = if replayer.ended() {
            ""
        } else {
            " (maximum duration reached)"
        };
        self.with(|s| {
            s.editor
                .log(Origin::Agent, format!("WAV render: {}", files.join(", ")))
        });
        Ok(format!(
            "{} — {}{limit}",
            files.join(", "),
            minutes(frames as f64 / rate as f64)
        ))
    }

    #[tool(description = "Undoes the last change.")]
    async fn undo(&self) -> ToolResult {
        self.with(|s| {
            s.editor
                .undo(Origin::Agent)
                .map(|d| format!("undone: {d}"))
                .ok_or_else(|| "nothing to undo".into())
        })
    }

    #[tool(description = "Redoes the last undone change.")]
    async fn redo(&self) -> ToolResult {
        self.with(|s| {
            s.editor
                .redo(Origin::Agent)
                .map(|d| format!("redone: {d}"))
                .ok_or_else(|| "nothing to redo".into())
        })
    }

    #[tool(description = "Last lines of the change journal.")]
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
                "smpltrckr: a ProTracker-style tracker (.mod, 4 voices, 31 samples, 64-row patterns). \
                 Read reference(topic=\"guide\") first. Patterns are written in text notation: \
                 `NN | C-3 01 A04 | ... .. ... | …` (note, sample in decimal, effect in hex). \
                 Every change can be undone with undo.",
            )
    }
}

pub fn run() -> anyhow::Result<()> {
    // The agent side speaks English, whatever the user's locale.
    crate::lang::set("en");
    tokio::runtime::Runtime::new()?.block_on(async {
        let service = Tracker::new().serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        Ok(())
    })
}
