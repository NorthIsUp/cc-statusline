// Cross-session cache of recent PRs authored by the viewer. One GraphQL call
// (~$XDG_CACHE_HOME/cc-statusbar/recent_prs.toml) hydrates state for every
// chip across every Claude Code session, replacing N per-URL `gh pr view`
// lookups with one batched query refreshed every `recent_prs_ttl` seconds.
//
// Concurrency: a sibling `recent_prs.lock` file is held exclusively by any
// in-flight refresh worker; foreground renders never block on it (they read
// whatever's currently on disk, even if stale).

use crate::cache::now_epoch;
use crate::config;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentPrs {
    pub fetched_at: i64,
    pub locked_at: i64,
    /// Map url -> {state, isDraft}. Stored as serde_json::Value-ish so we
    /// don't need a fixed schema in TOML.
    pub prs: HashMap<String, PrEntry>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrEntry {
    pub state: String,
    pub is_draft: bool,
    pub number: u64,
}

impl RecentPrs {
    pub fn path() -> PathBuf {
        config::recent_prs_path()
    }
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> io::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = toml::to_string(self).map_err(|e| io::Error::other(e.to_string()))?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, &path)
    }
}

fn fresh(at: i64, ttl: i64) -> bool {
    at > 0 && (now_epoch() - at) < ttl
}

/// Spawn a detached refresh worker if the cache is stale and not currently
/// locked. Returns immediately — the worker runs in background.
pub fn maybe_spawn_refresh() {
    let cur = RecentPrs::load();
    let ttl = config::config().recent_prs_ttl();
    if fresh(cur.fetched_at, ttl) && !cur.prs.is_empty() {
        return;
    }
    if fresh(cur.locked_at, ttl.max(60)) {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe)
            .arg("--refresh-recent-prs")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

/// Refresh worker entry point — invoked from main on `--refresh-recent-prs`.
/// Acquires an exclusive OS lock so only one worker runs at a time, then
/// fetches and writes the cache.
pub fn run_refresh() {
    let lock_path = RecentPrs::path().with_extension("toml.lock");
    if let Some(parent) = lock_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let lock = match OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(_) => return,
    };
    if lock.try_lock().is_err() {
        return; // another worker has it; that one will refresh.
    }

    // Re-check freshness inside the lock.
    let mut cur = RecentPrs::load();
    let ttl = config::config().recent_prs_ttl();
    if fresh(cur.fetched_at, ttl) && !cur.prs.is_empty() {
        return;
    }
    cur.locked_at = now_epoch();
    let _ = cur.save();

    if let Some(prs) = fetch() {
        let new = RecentPrs {
            fetched_at: now_epoch(),
            locked_at: 0,
            prs,
        };
        let _ = new.save();
    } else {
        // Fetch failed — clear the lock so the next render can retry, but
        // keep the previously-cached prs intact.
        cur.locked_at = 0;
        let _ = cur.save();
    }
}

const QUERY: &str = r#"query {
  viewer {
    pullRequests(first: 100, orderBy: {field: UPDATED_AT, direction: DESC}, states: [OPEN, MERGED, CLOSED]) {
      nodes { url state isDraft number }
    }
  }
}"#;

fn fetch() -> Option<HashMap<String, PrEntry>> {
    let out = Command::new("gh")
        .args(["api", "graphql", "-f", &format!("query={QUERY}")])
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_TOKEN")
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let nodes = v
        .get("data")?
        .get("viewer")?
        .get("pullRequests")?
        .get("nodes")?
        .as_array()?;
    let mut map = HashMap::with_capacity(nodes.len());
    for n in nodes {
        let url = n.get("url").and_then(|x| x.as_str())?.to_string();
        let state = n
            .get("state")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let is_draft = n.get("isDraft").and_then(|x| x.as_bool()).unwrap_or(false);
        let number = n.get("number").and_then(|x| x.as_u64()).unwrap_or(0);
        map.insert(
            url,
            PrEntry {
                state,
                is_draft,
                number,
            },
        );
    }
    Some(map)
}
