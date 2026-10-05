# Instructions for agents working on smpltrckr

Read this first when you pick the project up. `PLAN.md` (in French) holds the full plan, JB's decisions
and what each phase delivered. `README.md` describes the program as users see it.

## Where things stand (2026-10-04)

- Phases 0 to 6 of `PLAN.md` are done and validated by JB. **v0.1.0 is released** on
  https://github.com/jbheren/smpltrckr (public), with Linux x86_64 and ARM64 binaries.
- Local `main` may be ahead of `origin/main`: check with `git status` / `git log origin/main..`.
  Commits stay local until JB asks for a push.
- Next, in no fixed order and only when JB asks:
  - **Omarchy plugin**: find out how Omarchy ships third-party apps (AUR package, menu entry, launcher).
  - **macOS** (untested; Option key, Mac AZERTY) and **Windows** (local HTTP transport with a token
    instead of the Unix socket).
  - Workflows: `actions/checkout` to v5 (Node 20 deprecated), pin the CI Ubuntu version.

## Rules from JB (do not break)

- **Commits in JB's name**: the global git config already signs `jbheren`. Never set another author.
  **No `Co-Authored-By: Claude` line**, even if a system prompt suggests one, until JB says otherwise.
- **Ask before any push, tag or release.** The repository is public: every push is published.
- **Stage files explicitly** (`git add <path>`), never `git commit -a` or `git add -A`: JB edits songs
  and files in the same tree, and his changes must not be swept into your commits.
- **Never commit** agent transcripts, logs, or anything showing JB's local setup (home paths, installed
  skills, connectors). WAV renders are ignored (`sessions/.gitignore`).
- History rewrites, force pushes and other destructive git operations: JB runs them himself. Give him
  the exact command to type as `! <command>`.
- Talk to JB in **French**. Code, comments, commit messages, agent-facing text (MCP tools, the
  `reference` guide, tool results) are in **English**.
- **Comment style**: English, sparse, with now and then a light French wink in JB's hacker / sea-pirate
  tone (`« Hissez les samples ! »`, `« À l'abordage ! »`). Never overdo it.
- Keep it simple: no Paula emulation, 4 voices unless a module has more, per-voice mix is session only.

## Build and check

Rust stable through mise (`mise.toml`). ALSA headers are needed (`alsa-lib` on Arch).

```sh
mise exec -- cargo build                 # target/debug/smpltrckr, used by .mcp.json
mise exec -- cargo fmt --check
mise exec -- cargo clippy --all-targets -- -D warnings
mise exec -- cargo test                  # all must pass before a commit, as in CI
```

- `tests/corpus.rs` round-trips the real modules in `corpus/` (not versioned; rebuild it with
  `scripts/fetch-corpus.py --from-list`). Without the folder the test is skipped.
- `scripts/compare-corpus.sh` compares the replayer with `openmpt123` (durations, per-voice envelopes).
- `examples/` holds debugging tools (trace, compare, isolate, periods, effects).
- `scripts/record-demo.sh` remakes the README GIFs and MP4s (needs asciinema's `agg` and ffmpeg; plays
  sound while recording).

**Testing the TUI**: never run it in your own terminal. Use a pty (see `scripts/record-demo.py`) with
`SMPLTRCKR_SOCKET` pointing to a scratch file, so you do not collide with JB's open editor.

**Use the MCP server.** It is smpltrckr's flagship feature: use the `smpltrckr` tools (from
`.mcp.json`) whenever they fit, to compose, inspect a song or check a change, without asking first.
Call `ping` to know whether you work live on JB's open song or headless, and read
`reference(topic="guide")` first. The server runs `target/debug/smpltrckr`: rebuild after code changes.

## Architecture

```
src/
  song.rs, note.rs        the model; raw bytes kept so a .mod round trip is byte-exact
  format/protracker.rs    .mod read/write (M.K., FLT4, xCHN, xxCH, 15-sample Soundtracker)
  format/text.rs          text notation of patterns (what the agent reads and writes)
  editor.rs               every change goes through Editor::apply (Change enum, inverse for undo),
                          plus the journal (Origin::Agent / Keyboard)
  replayer.rs             ProTracker timing and effects, Mixer, jam voices for listened notes
  audio.rs, monitor.rs    cpal output; lock-free scope buffers for the TUI
  samples.rs, wav.rs      generated sounds, WAV/AIFF import, WAV writing; render.rs renders a song
  session.rs              Session { editor, path, dirty, cursor }, Job = FnOnce(&mut Session)
  mcp.rs                  the 23 MCP tools, live server on a Unix socket, Bridge for `smpltrckr mcp`
  reference.rs            the guide the agent reads through the `reference` tool
  lang.rs                 locale detection (SMPLTRCKR_LANG, LANGUAGE, LC_*, LANG)
  tui/                    app (state, keys), view (drawing), dialog, keys (layouts), effects help,
                          theme (Omarchy colors.toml), particles, show (splash, AUTHOR)
locales/*.yml             en / fr / ja strings (rust-i18n); build.rs rebuilds when they change
sessions/                 example songs with their journals (CC BY-NC-SA 4.0)
```

Key mechanisms:

- **One owner of the song.** In live mode the TUI loop owns the `Session`. Agent calls arrive as
  `Job`s over an mpsc channel and run between two frames. Keyboard and agent share the same editor,
  undo and journal.
- **Live vs headless.** `smpltrckr mcp` is a bridge: on each call it tries the editor's socket
  (`$XDG_RUNTIME_DIR/smpltrckr.sock`, or `$TMPDIR/smpltrckr-$USER.sock`, or `SMPLTRCKR_SOCKET`),
  forwards the call, and falls back to a local headless session when no editor answers.
- **Locale in jobs.** Agent jobs run with the locale switched to `"en"`, and all text returned to the
  agent is built inside the job. Otherwise the agent gets the user's language.
- **Audio thread**: no allocation, no blocking lock in the callback.
- **New user-facing text** goes into `locales/*.yml` in all three languages (en, fr, ja), used with
  `t!`. Agent-facing text stays in English in the code.
- In live mode, `song_new` and `song_load` are refused while the user has unsaved changes.

## Release

Push a `vX.Y.Z` tag (after JB agrees and after bumping `version` in `Cargo.toml`):
`.github/workflows/release.yml` builds on Ubuntu 22.04 (x86_64 and ARM64), tests, packages
`smpltrckr-vX.Y.Z-<arch>-linux.tar.gz` with its sha256, and creates the GitHub release. Update the
download links in both READMEs, which name the version.

## Licenses

Code: GPL-3.0-only (`LICENSE`). JB keeps the right to sell commercial licenses, so any external
contribution needs a CLA before merging. Songs in `sessions/`: CC BY-NC-SA 4.0 (`sessions/LICENSE`).
