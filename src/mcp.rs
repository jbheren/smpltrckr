//! MCP server: the agent composes through tools that turn requests into editor changes
//! (`Origin::Agent`). Everything can be undone with `undo` and leaves a line in the journal.
//!
//! Two modes:
//! - live: the keyboard interface is open; it listens on a local socket (`socket_path`) and
//!   runs the server itself. Each tool sends a job to the interface loop, which owns the song,
//!   so the agent and the user edit the same song at the same time;
//! - headless: no interface; `smpltrckr mcp` keeps its own session and works on files.
//!
//! `smpltrckr mcp` (`run`) is what MCP clients launch: it relays stdio to the open interface
//! when there is one, and serves headless otherwise. The agent side speaks English.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::editor::{Change, Editor, Origin, journal_path, journal_text};
use crate::format::{protracker, text};
use crate::replayer::Replayer;
use crate::session::{Job, Session};
use crate::song::{Pattern, Song};
use crate::{note, reference, render, samples, wav};

type ToolResult = Result<String, String>;

/// Where the session lives.
#[derive(Clone)]
enum Host {
    /// Headless: our own session.
    Local(Arc<Mutex<Session>>),
    /// Live: the keyboard interface owns the session; send it jobs.
    Live(mpsc::Sender<Job>),
}

#[derive(Clone)]
struct Tracker {
    host: Host,
    tool_router: ToolRouter<Self>,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

fn minutes(seconds: f64) -> String {
    format!("{}:{:04.1}", (seconds / 60.0) as u32, seconds % 60.0)
}

const EDITOR_CLOSED: &str = "the editor has been closed";

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
    fn new(host: Host) -> Self {
        Self {
            host,
            tool_router: Self::tool_router(),
        }
    }

    fn live(&self) -> bool {
        matches!(self.host, Host::Live(_))
    }

    /// Runs `f` on the session, wherever it lives.
    async fn with<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Session) -> T + Send + 'static,
    ) -> Result<T, String> {
        match &self.host {
            Host::Local(session) => Ok(f(&mut session.lock().unwrap())),
            Host::Live(jobs) => {
                let (reply, answer) = tokio::sync::oneshot::channel();
                let job: Job = Box::new(move |s| {
                    let _ = reply.send(f(s));
                });
                jobs.send(job).map_err(|_| EDITOR_CLOSED.to_string())?;
                answer.await.map_err(|_| EDITOR_CLOSED.to_string())
            }
        }
    }

    /// Applies a change from the agent, described by `description`.
    async fn edit(
        &self,
        description: String,
        build: impl FnOnce(&Editor) -> anyhow::Result<Vec<Change>> + Send + 'static,
    ) -> ToolResult {
        self.with(move |s| {
            let changes = build(&s.editor).map_err(err)?;
            s.editor.apply(Origin::Agent, description.clone(), changes);
            s.dirty = true;
            Ok(format!("ok: {description}"))
        })
        .await?
    }

    #[tool(
        description = "Checks that the smpltrckr server answers. Tells whether you are live (working on the song open in the user's editor, at the same time as them) or headless (your own song, on files)."
    )]
    async fn ping(&self) -> String {
        let mode = if self.live() {
            "live: connected to the user's open editor; you both edit the same song, changes show up on their screen"
        } else {
            "headless: no editor open, you work on your own song and files"
        };
        format!("smpltrckr {} — {mode}", env!("CARGO_PKG_VERSION"))
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
        description = "Creates a new empty song (4 voices, one empty pattern). The history starts over. Live: refused while the user has unsaved changes."
    )]
    async fn song_new(&self, Parameters(p): Parameters<NewSong>) -> ToolResult {
        let live = self.live();
        self.with(move |s| {
            if live && s.dirty {
                return Err("the user has unsaved changes: ask them to save first".to_string());
            }
            let description = format!("new song \"{}\"", p.title);
            s.editor
                .replace_song(Origin::Agent, Song::new(&p.title), description);
            (s.path, s.dirty) = (None, false);
            Ok(format!(
                "song \"{}\" created: 4 voices, empty pattern 00, orders [0]",
                p.title
            ))
        })
        .await?
    }

    #[tool(description = "Opens a .mod file. Live: refused while the user has unsaved changes.")]
    async fn song_load(&self, Parameters(p): Parameters<FilePath>) -> ToolResult {
        let path = PathBuf::from(&p.path);
        let live = self.live();
        // Texts are built inside jobs, where the language is the agent's (English).
        self.with(move |s| {
            if live && s.dirty {
                return Err("the user has unsaved changes: ask them to save first".to_string());
            }
            let data =
                std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let song = protracker::read(&data).map_err(err)?;
            let summary = text::song_to_text(&song, []);
            s.editor
                .replace_song(Origin::Agent, song, format!("opened {}", path.display()));
            (s.path, s.dirty) = (Some(path), false);
            Ok(summary)
        })
        .await?
    }

    #[tool(
        description = "Saves the song as a .mod, with the change journal next to it (.journal.txt)."
    )]
    async fn song_save(&self, Parameters(p): Parameters<SavePath>) -> ToolResult {
        self.with(move |s| {
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
            (s.path, s.dirty) = (Some(path.clone()), false);
            Ok(format!(
                "saved: {} (journal: {})",
                path.display(),
                journal.display()
            ))
        })
        .await?
    }

    #[tool(
        description = "Song summary: title, format, order list, samples, pattern count and duration. Live: also where the user is (position, pattern, row, voice, playing)."
    )]
    async fn song_info(&self) -> ToolResult {
        let (song, cursor, summary) = self
            .with(|s| {
                (
                    s.editor.song().clone(),
                    s.cursor,
                    text::song_to_text(s.editor.song(), []),
                )
            })
            .await?;
        let seconds = render::duration(Arc::new(song), 1200.0);
        let mut out = format!("{summary}duration : {}\n", minutes(seconds));
        if let Some(c) = cursor {
            let playing = if c.playing { "playing" } else { "stopped" };
            out += &format!(
                "user     : position {:02} (pattern {:02}), row {:02}, voice {}, {playing}\n",
                c.position, c.pattern, c.row, c.voice
            );
        }
        Ok(out)
    }

    #[tool(description = "Changes the song title.")]
    async fn song_set_title(&self, Parameters(p): Parameters<Title>) -> ToolResult {
        let title = Song::new(&p.title).title;
        self.edit(format!("title \"{}\"", p.title), move |_| {
            Ok(vec![Change::Title(title)])
        })
        .await
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
        self.edit(description, move |ed| {
            Ok(vec![ed.set_start_tempo(p.bpm, p.speed)?])
        })
        .await
    }

    #[tool(description = "Sets the order list: the pattern numbers played, in order.")]
    async fn order_set(&self, Parameters(p): Parameters<Orders>) -> ToolResult {
        let list: Vec<String> = p.orders.iter().map(|o| format!("{o:02}")).collect();
        self.edit(format!("orders {}", list.join(" ")), move |ed| {
            Ok(vec![ed.set_orders(&p.orders)?])
        })
        .await
    }

    #[tool(description = "Reads a pattern in text notation (64 rows).")]
    async fn pattern_get(&self, Parameters(p): Parameters<PatternIndex>) -> ToolResult {
        self.with(move |s| {
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
        .await?
    }

    #[tool(
        description = "Writes rows into a pattern (text notation). Only the given rows change. The number right after the last pattern creates a pattern."
    )]
    async fn pattern_write(&self, Parameters(p): Parameters<PatternWrite>) -> ToolResult {
        let first = p.first_voice.unwrap_or(1);
        let count = p
            .rows
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .count();
        let result = self
            .edit(
                format!("pattern {:02}: rows written", p.pattern),
                move |ed| {
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
                    let lines = p
                        .rows
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty() && !l.starts_with('#'));
                    for line in lines {
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
                    }
                    Ok(vec![ed.set_pattern(p.pattern, pattern)?])
                },
            )
            .await?;
        Ok(format!("{result} ({count} rows)"))
    }

    #[tool(description = "Clears a pattern.")]
    async fn pattern_clear(&self, Parameters(p): Parameters<PatternIndex>) -> ToolResult {
        self.edit(format!("pattern {:02} cleared", p.pattern), move |ed| {
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
        .await
    }

    #[tool(
        description = "Copies a pattern onto another one (existing, or new if it follows the last)."
    )]
    async fn pattern_copy(&self, Parameters(p): Parameters<PatternCopy>) -> ToolResult {
        self.edit(
            format!("pattern {:02} copied to {:02}", p.from, p.to),
            move |ed| {
                let source = ed
                    .song()
                    .patterns
                    .get(p.from)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no pattern {}", p.from))?;
                Ok(vec![ed.set_pattern(p.to, source)?])
            },
        )
        .await
    }

    #[tool(
        description = "Transposes the notes of a pattern by n semitones (every voice, or some)."
    )]
    async fn pattern_transpose(&self, Parameters(p): Parameters<Transpose>) -> ToolResult {
        let description = format!(
            "pattern {:02} transposed by {:+} semitones",
            p.pattern, p.semitones
        );
        self.edit(description, move |ed| {
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
        .await
    }

    #[tool(
        description = "Lists the non-empty samples: number, name, size, volume, finetune, loop."
    )]
    async fn sample_list(&self) -> ToolResult {
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
        .await
    }

    #[tool(
        description = "Generates a sample: looped waveform (sine, square, pulse, saw, triangle, noise) or drum (kick, snare, hihat, to be played at C-3)."
    )]
    async fn sample_generate(&self, Parameters(p): Parameters<Generate>) -> ToolResult {
        let description = format!("sample {:02}: {} generated", p.sample, p.waveform);
        let size = Arc::new(AtomicUsize::new(0));
        let measured = size.clone();
        let result = self
            .edit(description, move |ed| {
                let mut sample = samples::generate(&p.waveform, p.cycle.unwrap_or(32))?;
                sample.volume = p.volume.unwrap_or(64).min(64);
                if let Some(name) = &p.name {
                    sample.set_name(name);
                }
                measured.store(sample.data.len(), Ordering::Relaxed);
                Ok(vec![ed.set_sample(p.sample, sample)?])
            })
            .await?;
        Ok(format!("{result} ({} bytes)", size.load(Ordering::Relaxed)))
    }

    #[tool(
        description = "Loads a WAV or AIFF file into a sample (mono, 8 bits, no resampling; 128 KB at most)."
    )]
    async fn sample_load(&self, Parameters(p): Parameters<LoadSample>) -> ToolResult {
        let (path, halve) = (p.path.clone(), p.halve.unwrap_or(false));
        let report = self
            .with(move |_| samples::import(Path::new(&path), halve).map_err(err))
            .await??;
        let size = report.sample.data.len();
        let description = format!("sample {:02}: {} loaded", p.sample, p.path);
        let sample = report.sample;
        let result = self
            .edit(description, move |ed| {
                Ok(vec![ed.set_sample(p.sample, sample)?])
            })
            .await?;
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
        self.edit(format!("sample {:02} tuned", p.sample), move |ed| {
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
        .await
    }

    #[tool(
        description = "Session mix (not saved in the .mod, applied to WAV renders): mute, solo or volume of a voice. Live: changes what the user hears."
    )]
    async fn mix_set(&self, Parameters(p): Parameters<MixSet>) -> ToolResult {
        self.with(move |s| {
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
        .await?
    }

    #[tool(
        description = "Renders the song to a 16-bit 48 kHz WAV (stereo, or one track per voice), with the session mix."
    )]
    async fn render_wav(&self, Parameters(p): Parameters<RenderWav>) -> ToolResult {
        let (song, mixer) = self
            .with(|s| (Arc::new(s.editor.song().clone()), s.editor.mixer.clone()))
            .await?;
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
        let list = files.join(", ");
        let logged = list.clone();
        self.with(move |s| s.editor.log(Origin::Agent, format!("WAV render: {logged}")))
            .await?;
        Ok(format!(
            "{list} — {}{limit}",
            minutes(frames as f64 / rate as f64)
        ))
    }

    #[tool(
        description = "Undoes the last change (live: the last change of anyone, the user included)."
    )]
    async fn undo(&self) -> ToolResult {
        self.with(|s| {
            let done = s.editor.undo(Origin::Agent).map(|d| format!("undone: {d}"));
            s.dirty |= done.is_some();
            done.ok_or_else(|| "nothing to undo".to_string())
        })
        .await?
    }

    #[tool(description = "Redoes the last undone change.")]
    async fn redo(&self) -> ToolResult {
        self.with(|s| {
            let done = s.editor.redo(Origin::Agent).map(|d| format!("redone: {d}"));
            s.dirty |= done.is_some();
            done.ok_or_else(|| "nothing to redo".to_string())
        })
        .await?
    }

    #[tool(description = "Last lines of the change journal (who did what: agent or keyboard).")]
    async fn history(&self, Parameters(p): Parameters<History>) -> ToolResult {
        self.with(move |s| {
            let text = journal_text(&s.editor);
            let lines: Vec<&str> = text.lines().collect();
            lines[lines.len().saturating_sub(p.last.unwrap_or(20))..].join("\n")
        })
        .await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Tracker {
    fn get_info(&self) -> ServerConfig {
        let live = if self.live() {
            " You are LIVE: the user has this song open in their editor and edits it at the same \
             time; your changes show up on their screen. Call song_info to see where they are, \
             and avoid rewriting what they are working on unless asked."
        } else {
            ""
        };
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("smpltrckr", env!("CARGO_PKG_VERSION")))
            .with_instructions(format!(
                "smpltrckr: a ProTracker-style tracker (.mod, 4 voices, 31 samples, 64-row patterns). \
                 Read reference(topic=\"guide\") first. Patterns are written in text notation: \
                 `NN | C-3 01 A04 | ... .. ... | …` (note, sample in decimal, effect in hex). \
                 Every change can be undone with undo.{live}"
            ))
    }
}

// --- Live sessions ---------------------------------------------------------------------------

/// Socket where an open editor waits for agents: `SMPLTRCKR_SOCKET`, else
/// `$XDG_RUNTIME_DIR/smpltrckr.sock`, else a per-user file in the temp folder.
pub fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SMPLTRCKR_SOCKET") {
        return PathBuf::from(path);
    }
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("smpltrckr.sock"),
        None => std::env::temp_dir().join(format!("smpltrckr-{}.sock", std::process::id())),
    }
}

/// An editor listening for agents. Dropping it closes the socket.
pub struct LiveServer {
    path: PathBuf,
    /// Number of agents connected right now.
    pub agents: Arc<AtomicUsize>,
}

impl Drop for LiveServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Opens the live socket: each agent connecting to it gets an MCP server whose tools send
/// jobs to `jobs`. Fails if another editor already listens there.
pub fn serve_live(jobs: mpsc::Sender<Job>) -> anyhow::Result<LiveServer> {
    let path = socket_path();
    // A socket file nobody answers on is a leftover from a crash: clear it.
    if path.exists() {
        anyhow::ensure!(
            std::os::unix::net::UnixStream::connect(&path).is_err(),
            "another smpltrckr editor already listens on {}",
            path.display()
        );
        std::fs::remove_file(&path)?;
    }
    let listener = std::os::unix::net::UnixListener::bind(&path)?;
    listener.set_nonblocking(true)?;
    let agents = Arc::new(AtomicUsize::new(0));
    let counter = agents.clone();
    std::thread::Builder::new()
        .name("smpltrckr-live".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build();
            let Ok(runtime) = runtime else { return };
            runtime.block_on(async move {
                let Ok(listener) = tokio::net::UnixListener::from_std(listener) else {
                    return;
                };
                while let Ok((stream, _)) = listener.accept().await {
                    let (jobs, counter) = (jobs.clone(), counter.clone());
                    tokio::spawn(async move {
                        counter.fetch_add(1, Ordering::Relaxed);
                        if let Ok(service) = Tracker::new(Host::Live(jobs)).serve(stream).await {
                            let _ = service.waiting().await;
                        }
                        counter.fetch_sub(1, Ordering::Relaxed);
                    });
                }
            });
        })?;
    Ok(LiveServer { path, agents })
}

/// `smpltrckr mcp`: relays stdio to the open editor if there is one, else serves headless.
pub fn run() -> anyhow::Result<()> {
    // The agent side speaks English, whatever the user's locale.
    crate::lang::set("en");
    tokio::runtime::Runtime::new()?.block_on(async {
        if let Ok(stream) = tokio::net::UnixStream::connect(socket_path()).await {
            let (mut from_editor, mut to_editor) = stream.into_split();
            let (mut stdin, mut stdout) = (tokio::io::stdin(), tokio::io::stdout());
            // Whichever side closes first ends the relay.
            tokio::select! {
                _ = tokio::io::copy(&mut stdin, &mut to_editor) => {}
                _ = tokio::io::copy(&mut from_editor, &mut stdout) => {}
            }
            return Ok(());
        }
        let session = Session::new(Song::new(""), None);
        let service = Tracker::new(Host::Local(Arc::new(Mutex::new(session))))
            .serve(rmcp::transport::stdio())
            .await?;
        service.waiting().await?;
        Ok(())
    })
}
