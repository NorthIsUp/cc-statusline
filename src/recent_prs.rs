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
use serde_json::Value;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentPrs {
    /// Set to `crate::state::STATE_VERSION` on save; mismatched on load
    /// triggers a full reset (same migration story as per-session state).
    pub version: String,
    pub fetched_at: i64,
    pub locked_at: i64,
    /// Last-observed GitHub GraphQL `rateLimit.remaining` (of 5000/hr). Drives
    /// adaptive refresh spacing: the closer to exhaustion, the longer every
    /// session waits before refreshing. `0` means "never observed" (a fresh
    /// cache or a failed fetch) and is treated as healthy — the failure path's
    /// `locked_at` backoff covers a hard rate-limit separately.
    pub rate_remaining: i64,
    /// Map url -> {state, isDraft}. Stored as serde_json::Value-ish so we
    /// don't need a fixed schema in TOML.
    pub prs: HashMap<String, PrEntry>,
}

/// Adaptive refresh interval from the last-seen rate-limit headroom: the fewer
/// GraphQL points left, the longer we space out refreshes (across every
/// session, since they share this cache). `remaining == 0` = unknown/healthy.
fn adaptive_ttl(remaining: i64, base: i64) -> i64 {
    match remaining {
        r if r > 0 && r < 500 => 300, // <10% of 5000 left: 5m
        r if r > 0 && r < 2000 => 60, // <40% left: 1m
        _ => base,                    // healthy (or unknown): base (20s)
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrEntry {
    pub state: String,
    pub is_draft: bool,
    pub number: u64,
    /// Unix epoch seconds when the PR was merged. `None` if not merged or
    /// the timestamp could not be parsed (defensive: treat unknown as
    /// "do not collapse" rather than dropping).
    pub merged_at: Option<i64>,
    /// True iff GitHub's `autoMergeRequest` is non-null — i.e. the PR is
    /// queued to auto-merge when checks pass.
    #[serde(default)]
    pub auto_merge: bool,
}

impl RecentPrs {
    pub fn path() -> PathBuf {
        config::recent_prs_path()
    }
    pub fn load() -> Self {
        let parsed: Self = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default();
        if parsed.version != crate::state::STATE_VERSION {
            return Self::default();
        }
        parsed
    }
    pub fn save(&mut self) -> io::Result<()> {
        self.version = crate::state::STATE_VERSION.into();
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
    let ttl = adaptive_ttl(cur.rate_remaining, config::config().recent_prs_ttl());
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
    let ttl = adaptive_ttl(cur.rate_remaining, config::config().recent_prs_ttl());
    if fresh(cur.fetched_at, ttl) && !cur.prs.is_empty() {
        return;
    }
    cur.locked_at = now_epoch();
    let _ = cur.save();

    if let Some((mut prs, remaining)) = fetch() {
        // Second pass: hydrate any URLs referenced by other_prs.urls in any
        // session state file that aren't already present in the freshly
        // fetched viewer.pullRequests result. This catches PRs older than
        // the 100 most-recently-updated `viewer.pullRequests` window.
        let missing = collect_missing_urls(&prs);
        if !missing.is_empty() {
            for (url, entry) in fetch_by_urls(&missing) {
                prs.entry(url).or_insert(entry);
            }
        }
        let mut new = RecentPrs {
            version: crate::state::STATE_VERSION.into(),
            fetched_at: now_epoch(),
            locked_at: 0,
            // Carry the last-seen headroom forward (0 = unobserved → healthy),
            // so `adaptive_ttl` can space out the next round of refreshes.
            rate_remaining: remaining.unwrap_or(cur.rate_remaining),
            prs,
        };
        let _ = new.save();
    } else {
        // Fetch failed (commonly a rate-limit). Stamp `locked_at = now` so the
        // debounce holds for `ttl.max(60)`s instead of re-spawning a worker on
        // the very next render — otherwise, exactly when we're rate-limited,
        // every live session hammers the API in a tight loop and never
        // recovers. Keep the previously-cached prs intact.
        cur.locked_at = now_epoch();
        let _ = cur.save();
    }
}

/// A session file not rewritten within this window is *inactive*. A displayed
/// session rewrites its state file on every render (~1s cadence), so 1h is a
/// wide safety margin. Inactive sessions are ignored when hydrating chips —
/// nobody's viewing them, so refreshing their PR state just burns API budget —
/// even though the file is left on disk until it ages out.
const ACTIVE_SESSION_SECS: i64 = 60 * 60;

/// A session file older than this is a *dead* session and is deleted outright.
/// Kept longer than `ACTIVE_SESSION_SECS` so a session resumed within the day
/// still finds its chip-URL history, without us paying to refresh it while idle.
/// Without this, dead sessions accumulate indefinitely (hundreds observed).
const STALE_SESSION_SECS: i64 = 24 * 60 * 60;

enum SessionAge {
    Active, // touched within the hour: a live viewer — refresh its chips
    Idle,   // 1h–24h: keep the file, but don't spend API refreshing it
    Dead,   // >24h: delete
}

fn classify_age(age_secs: i64) -> SessionAge {
    if age_secs > STALE_SESSION_SECS {
        SessionAge::Dead
    } else if age_secs > ACTIVE_SESSION_SECS {
        SessionAge::Idle
    } else {
        SessionAge::Active
    }
}

/// Walks every *active* session state TOML in `cache_dir()` (excluding
/// `recent_prs.toml` itself) and returns the union of `other_prs.urls` entries
/// that are missing from `have`. Sessions idle >1h are ignored (no live viewer
/// to serve); dead sessions >24h are additionally deleted. Either way their
/// chips don't drive a fetch — that fan-out is what exhausts the 5000/hr budget.
fn collect_missing_urls(have: &HashMap<String, PrEntry>) -> Vec<String> {
    let dir = config::cache_dir();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("toml") {
            continue;
        }
        if path.file_name().and_then(|s| s.to_str()) == Some("recent_prs.toml") {
            continue;
        }
        // Ignore inactive sessions (no live viewer); delete truly dead ones.
        let mtime = match crate::cache::mtime(&path) {
            Some(m) => m,
            None => continue,
        };
        match classify_age(now_epoch() - mtime) {
            SessionAge::Dead => {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            SessionAge::Idle => continue,
            SessionAge::Active => {}
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let st: crate::state::State = match toml::from_str(&text) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for url in st.other_prs.urls {
            if !have.contains_key(&url) && parse_pr_url(&url).is_some() {
                seen.insert(url);
            }
        }
    }
    seen.into_iter().collect()
}

/// Parse `https://github.com/OWNER/REPO/pull/N` (with optional trailing
/// path/query/fragment) into `(owner, repo, number)`. Returns `None` for any
/// input that doesn't look like a PR URL.
pub(crate) fn parse_pr_url(url: &str) -> Option<(String, String, u64)> {
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let mut parts = rest.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next()? != "pull" {
        return None;
    }
    let num_part = parts.next()?;
    let num_str: String = num_part
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if num_str.is_empty() || owner.is_empty() || repo.is_empty() {
        return None;
    }
    let number: u64 = num_str.parse().ok()?;
    Some((owner.to_string(), repo.to_string(), number))
}

/// Batched PR-by-(owner,repo,number) lookup using aliased `repository.pullRequest`
/// fields. Each chunk is one GraphQL query of single-object lookups (no
/// `first`/`last` connection), so it costs ~1 rate-limit point regardless of
/// how many aliases it packs — fewer, larger chunks means fewer points. 100
/// stays well within GraphQL's node/complexity caps while halving the query
/// count vs the old 50. A failed alias inside an otherwise-successful batch is
/// skipped silently.
fn fetch_by_urls(urls: &[String]) -> HashMap<String, PrEntry> {
    let mut out = HashMap::new();
    let parsed: Vec<(String, (String, String, u64))> = urls
        .iter()
        .filter_map(|u| parse_pr_url(u).map(|p| (u.clone(), p)))
        .collect();
    for chunk in parsed.chunks(100) {
        if let Some(map) = fetch_chunk(chunk) {
            out.extend(map);
        }
    }
    out
}

fn fetch_chunk(chunk: &[(String, (String, String, u64))]) -> Option<HashMap<String, PrEntry>> {
    let mut q = String::from("query {\n");
    for (i, (_url, (owner, repo, number))) in chunk.iter().enumerate() {
        // Escape: owner/repo are GitHub identifiers (alnum, dash, underscore,
        // dot) — safe to inline. Defensive: skip any that contain quotes.
        if owner.contains('"') || repo.contains('"') {
            continue;
        }
        q.push_str(&format!(
            "  a{i}: repository(owner: \"{owner}\", name: \"{repo}\") {{ pullRequest(number: {number}) {{ url state isDraft number mergedAt autoMergeRequest {{ __typename }} }} }}\n"
        ));
    }
    q.push_str("}\n");

    let v = crate::github::graphql(&q)?;
    let data = v.get("data")?.as_object()?;
    let mut map = HashMap::new();
    for (_alias, val) in data {
        let pr = match val.get("pullRequest") {
            Some(p) if !p.is_null() => p,
            _ => continue,
        };
        let url = match pr.get("url").and_then(|x| x.as_str()) {
            Some(u) => u.to_string(),
            None => continue,
        };
        let state = pr
            .get("state")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let is_draft = pr.get("isDraft").and_then(|x| x.as_bool()).unwrap_or(false);
        let number = pr.get("number").and_then(|x| x.as_u64()).unwrap_or(0);
        let merged_at = pr
            .get("mergedAt")
            .and_then(|x| x.as_str())
            .and_then(crate::input::ts_to_epoch);
        let auto_merge = pr
            .get("autoMergeRequest")
            .map(|v| !v.is_null())
            .unwrap_or(false);
        map.insert(
            url,
            PrEntry {
                state,
                is_draft,
                number,
                merged_at,
                auto_merge,
            },
        );
    }
    Some(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pr_url_basic() {
        assert_eq!(
            parse_pr_url("https://github.com/foo/bar/pull/42"),
            Some(("foo".into(), "bar".into(), 42))
        );
    }

    #[test]
    fn parse_pr_url_trailing_path() {
        assert_eq!(
            parse_pr_url("https://github.com/foo/bar/pull/42/files"),
            Some(("foo".into(), "bar".into(), 42))
        );
        assert_eq!(
            parse_pr_url("https://github.com/foo/bar/pull/42#issuecomment-1"),
            Some(("foo".into(), "bar".into(), 42))
        );
    }

    #[test]
    fn adaptive_ttl_tiers() {
        // healthy or unobserved (0) → base; low → 1m; very low → 5m.
        assert_eq!(adaptive_ttl(0, 20), 20);
        assert_eq!(adaptive_ttl(5000, 20), 20);
        assert_eq!(adaptive_ttl(2000, 20), 20);
        assert_eq!(adaptive_ttl(1999, 20), 60);
        assert_eq!(adaptive_ttl(500, 20), 60);
        assert_eq!(adaptive_ttl(499, 20), 300);
        assert_eq!(adaptive_ttl(1, 20), 300);
    }

    #[test]
    fn classify_age_windows() {
        assert!(matches!(classify_age(0), SessionAge::Active));
        assert!(matches!(
            classify_age(ACTIVE_SESSION_SECS),
            SessionAge::Active
        ));
        assert!(matches!(
            classify_age(ACTIVE_SESSION_SECS + 1),
            SessionAge::Idle
        ));
        assert!(matches!(classify_age(STALE_SESSION_SECS), SessionAge::Idle));
        assert!(matches!(
            classify_age(STALE_SESSION_SECS + 1),
            SessionAge::Dead
        ));
    }

    #[test]
    fn parse_pr_url_with_dashes_dots() {
        assert_eq!(
            parse_pr_url("https://github.com/some-org/my.repo/pull/7"),
            Some(("some-org".into(), "my.repo".into(), 7))
        );
    }

    #[test]
    fn parse_pr_url_rejects_non_pr() {
        assert!(parse_pr_url("https://github.com/foo/bar/issues/1").is_none());
        assert!(parse_pr_url("https://example.com/foo/bar/pull/1").is_none());
        assert!(parse_pr_url("https://github.com/foo/bar/pull/").is_none());
        assert!(parse_pr_url("https://github.com/foo/bar/pull/abc").is_none());
        assert!(parse_pr_url("not a url").is_none());
    }
}

const QUERY: &str = r#"query {
  rateLimit { remaining }
  viewer {
    pullRequests(first: 100, orderBy: {field: UPDATED_AT, direction: DESC}, states: [OPEN, MERGED, CLOSED]) {
      nodes { url state isDraft number mergedAt autoMergeRequest { __typename } }
    }
  }
}"#;

/// Returns `(prs, rateLimit.remaining)`. `remaining` is `None` when the field
/// is absent (older API / unexpected shape); `rateLimit` itself costs 0 points.
fn fetch() -> Option<(HashMap<String, PrEntry>, Option<i64>)> {
    let v = crate::github::graphql(QUERY)?;
    let remaining = v
        .pointer("/data/rateLimit/remaining")
        .and_then(Value::as_i64);
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
        let merged_at = n
            .get("mergedAt")
            .and_then(|x| x.as_str())
            .and_then(crate::input::ts_to_epoch);
        let auto_merge = n
            .get("autoMergeRequest")
            .map(|v| !v.is_null())
            .unwrap_or(false);
        map.insert(
            url,
            PrEntry {
                state,
                is_draft,
                number,
                merged_at,
                auto_merge,
            },
        );
    }
    Some((map, remaining))
}
