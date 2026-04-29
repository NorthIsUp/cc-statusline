// Per-session state at $XDG_CACHE_HOME/cc-statusbar/<id>.toml.
//
// All caches that the bash version kept as separate files (pr.json, other.txt,
// states.json, focused, tokens.txt, agents.txt) live here as fields of one
// TOML file.
//
// Concurrency: foreground renders and detached background refreshers both
// read-modify-write the state. We serialize via an OS file lock acquired
// against a sibling .lock file. The lock is held for the duration of the
// render or refresh pass, so renders never see a half-written file.

use crate::cache::now_epoch;
use crate::config;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::PathBuf;

/// Binary version this state was last written by. If the on-disk version
/// doesn't match the running binary's `CARGO_PKG_VERSION`, the state is
/// discarded and rebuilt — gives us free schema migration on version bumps.
pub const STATE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// Set to `STATE_VERSION` on save; checked on load.
    pub version: String,
    pub focus: FocusState,
    pub pr: PrCache,
    pub other_prs: OtherPrCache,
    pub burn: BurnCache,
    pub agents: AgentCache,
    /// Monotonic render counter. Drives the spinner so it advances exactly one
    /// frame per render, immune to harness invocation gaps and intra-second
    /// re-renders that would skip / repeat frames if we used epoch seconds.
    #[serde(default)]
    pub tick: u64,
    /// Layout-engine hysteresis state — sticky shrink decisions across renders.
    #[serde(default)]
    pub layout: crate::layout::LayoutState,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FocusState {
    pub focused: bool,
    pub updated_at: i64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrCache {
    pub fetched_at: i64,
    pub locked_at: i64,
    /// Raw JSON returned by `gh pr view --json …`.
    pub json: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OtherPrCache {
    pub fetched_at: i64,
    pub locked_at: i64,
    pub urls: Vec<String>,
    /// Raw JSON object: {url -> {state, isDraft}}.
    pub states_json: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BurnCache {
    pub transcript_mtime: i64,
    pub total_tokens: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentCache {
    pub transcript_mtime: i64,
    pub active: u32,
    pub total: u32,
}

/// RAII handle holding an exclusive OS lock on the per-session state file.
/// Lock is released when this is dropped.
pub struct StateLock {
    session_id: String,
    path: PathBuf,
    _lock: File,
    pub state: State,
}

impl StateLock {
    pub fn acquire_blocking(session_id: &str) -> io::Result<Self> {
        let path = State::path(session_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path.with_extension("toml.lock"))?;
        lock.lock()?;

        let state = read_state(&path);
        Ok(Self {
            session_id: session_id.into(),
            path,
            _lock: lock,
            state,
        })
    }

    pub fn try_acquire(session_id: &str) -> io::Result<Option<Self>> {
        let path = State::path(session_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path.with_extension("toml.lock"))?;
        match lock.try_lock() {
            Ok(_) => {
                let state = read_state(&path);
                Ok(Some(Self {
                    session_id: session_id.into(),
                    path,
                    _lock: lock,
                    state,
                }))
            }
            Err(_) => Ok(None),
        }
    }

    pub fn save(&mut self) -> io::Result<()> {
        self.state.version = STATE_VERSION.into();
        let body = toml::to_string(&self.state).map_err(|e| io::Error::other(e.to_string()))?;
        let tmp = self.path.with_extension("toml.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, &self.path)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

fn read_state(path: &PathBuf) -> State {
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(_) => return State::default(),
    };
    let mut buf = String::new();
    if f.read_to_string(&mut buf).is_err() {
        return State::default();
    }
    let parsed: State = toml::from_str(&buf).unwrap_or_default();
    // Auto-clear cache if the binary version changed since we last wrote.
    // Free schema migration — bumping Cargo.toml invalidates everyone's
    // state without manual `rm` dance.
    if parsed.version != STATE_VERSION {
        return State::default();
    }
    parsed
}

impl State {
    pub fn path(session_id: &str) -> PathBuf {
        config::state_path(session_id)
    }
    pub fn load(session_id: &str) -> Self {
        read_state(&Self::path(session_id))
    }
    pub fn save_unlocked(&self, session_id: &str) -> io::Result<()> {
        let path = Self::path(session_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = toml::to_string(self).map_err(|e| io::Error::other(e.to_string()))?;
        let tmp = path.with_extension("toml.tmp");
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(body.as_bytes())?;
        std::fs::rename(&tmp, &path)
    }
}

pub fn fresh(at: i64, ttl: i64) -> bool {
    at > 0 && (now_epoch() - at) < ttl
}
