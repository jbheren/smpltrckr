//! A session: the song being edited, its file and whether it has unsaved changes. The
//! keyboard interface owns it; the agent reaches it through jobs run on the interface loop,
//! so there is only ever one hand on the helm.

use std::path::PathBuf;

use crate::editor::Editor;
use crate::song::Song;

pub struct Session {
    pub editor: Editor,
    /// File opened or saved last.
    pub path: Option<PathBuf>,
    /// Changes not saved yet.
    pub dirty: bool,
    /// Where the user is, kept up to date by the keyboard interface (live sessions only).
    pub cursor: Option<Cursor>,
}

/// What the user is looking at, so the agent can work alongside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub position: usize,
    pub pattern: usize,
    pub row: usize,
    /// Voice, from 1.
    pub voice: usize,
    pub playing: bool,
}

impl Session {
    pub fn new(song: Song, path: Option<PathBuf>) -> Self {
        Self {
            editor: Editor::new(song),
            path,
            dirty: false,
            cursor: None,
        }
    }
}

/// Work sent by the agent to the session owner, run there between two frames.
pub type Job = Box<dyn FnOnce(&mut Session) + Send>;
