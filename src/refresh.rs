// Background refresh entry points. The foreground render spawns a detached
// copy of ourselves with `--refresh-pr <id>` or `--refresh-other <id>` when
// the corresponding cache is stale and not already locked.
//
// Locking debounces concurrent refreshes: the first spawned process gets the
// lock, subsequent spawns either skip or block. We use try_acquire so a stuck
// fetch never blocks the next foreground render.

use crate::cache::now_epoch;
use crate::config;
use crate::state::{self, StateLock};
use std::process::{Command, Stdio};

const ENV_CWD: &str = "CC_STATUSLINE_REFRESH_CWD";
const ENV_TRANSCRIPT: &str = "CC_STATUSLINE_REFRESH_TRANSCRIPT";
const ENV_STACK_CWD: &str = "CC_STATUSLINE_STACK_CWD";

/// How long a MERGED current-branch PR stays fresh. A merged PR is terminal,
/// so re-asking at `pr_cache_ttl` (60s by default) spends the bulk of this
/// tool's GraphQL budget re-confirming a state that cannot change — measured
/// at roughly 780 points/hr across 13 live sessions, against ~25/hr for the
/// shared chip cache.
///
/// Not `never`: the same branch can get a new PR after the old one merges, so
/// this is a long backstop rather than a permanent stop.
const MERGED_PR_TTL: i64 = 1800;

/// True when the cached PR JSON says MERGED.
fn cached_pr_merged(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| {
            v.get("state")
                .and_then(|s| s.as_str())
                .map(|s| s == "MERGED")
        })
        .unwrap_or(false)
}

pub fn maybe_spawn_pr(session_id: &str, cwd: &str, st: &state::State) {
    let mut ttl = config::config().pr_cache_ttl();
    if cached_pr_merged(&st.pr.json) {
        ttl = ttl.max(MERGED_PR_TTL);
    }
    if state::fresh(st.pr.fetched_at, ttl) && !st.pr.json.is_empty() {
        return;
    }
    if state::fresh(st.pr.locked_at, ttl.max(10)) {
        return;
    }
    spawn_self(&["--refresh-pr", session_id], &[(ENV_CWD, cwd)]);
}

pub fn maybe_spawn_other(session_id: &str, transcript: &str, st: &state::State) {
    if transcript.is_empty() {
        return;
    }
    let ttl = config::config().other_cache_ttl();
    if state::fresh(st.other_prs.fetched_at, ttl)
        && state::fresh(st.other_prs.locked_at, ttl)
        && !st.other_prs.urls.is_empty()
        && !st.other_prs.states_json.is_empty()
    {
        return;
    }
    if state::fresh(st.other_prs.locked_at, ttl.max(30)) {
        return;
    }
    spawn_self(
        &["--refresh-other", session_id],
        &[(ENV_TRANSCRIPT, transcript)],
    );
}

/// Spawn an async refresh of the Graphite stack snapshot for `cwd`. Uses the
/// same detached-respawn pattern as PR/other refreshes; the stack TTL is
/// configurable (`[chips].stack_refresh_ttl`, default 60s). Also debounced by
/// a `locked_at` field to prevent thundering-herd `gt` invocations.
pub fn maybe_spawn_stack(session_id: &str, cwd: &str, st: &state::State) {
    if cwd.is_empty() {
        return;
    }
    let ttl = config::config().stack_refresh_ttl();
    if state::fresh(st.stack.fetched_at, ttl) {
        return;
    }
    if state::fresh(st.stack.locked_at, ttl.max(30)) {
        return;
    }
    spawn_self(&["--refresh-stack", session_id], &[(ENV_STACK_CWD, cwd)]);
}

/// `(owner, name, branch)` for the repo at `cwd` — the inputs `gh pr view`
/// derived implicitly from the origin remote and the checked-out branch.
/// `None` on a detached HEAD, a missing/non-GitHub origin, or no repo.
fn repo_identity(cwd: &str) -> Option<(String, String, String)> {
    let repo = git2::Repository::discover(cwd).ok()?;
    let head = repo.head().ok()?;
    if !head.is_branch() {
        return None;
    }
    let branch = head.shorthand().ok()?.to_string();
    let remote = repo.find_remote("origin").ok()?;
    let (owner, name) = parse_remote(remote.url().ok()?)?;
    Some((owner, name, branch))
}

/// Extract `(owner, name)` from a github.com remote URL in any of the common
/// forms (scp-style, https, ssh). Non-github.com hosts return `None`.
fn parse_remote(url: &str) -> Option<(String, String)> {
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("git://github.com/"))?;
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, name) = rest.split_once('/')?;
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some((owner.to_string(), name.to_string()))
}

fn spawn_self(args: &[&str], envs: &[(&str, &str)]) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let _ = cmd.spawn();
}

pub fn run_refresh_pr(session_id: &str) {
    let cwd = std::env::var(ENV_CWD).unwrap_or_default();
    {
        let mut handle = match StateLock::acquire_blocking(session_id) {
            Ok(h) => h,
            Err(_) => return,
        };

        // Re-check freshness inside the lock — another worker may have already
        // refreshed between our spawn and our acquire.
        let ttl = config::config().pr_cache_ttl();
        if state::fresh(handle.state.pr.fetched_at, ttl) && !handle.state.pr.json.is_empty() {
            return;
        }

        // Mark the in-flight lock so concurrent foregrounds know not to
        // re-spawn, then release before the fetch: a foreground render
        // blocks on this same lock, so holding it across a network
        // round-trip stalls the statusline for the length of the request.
        handle.state.pr.locked_at = now_epoch();
        let _ = handle.save();
    }

    // The PR for the current branch, fetched directly from GitHub's GraphQL
    // API (was `gh pr view --json …`). `repo_identity` recovers the
    // owner/name/branch that gh inferred implicitly from cwd; auth is the
    // `GH_TOKEN`/`GITHUB_TOKEN` env var (see `github::token`).
    //
    // Two outcomes are "success" and one is "failure", and they must not be
    // conflated:
    //   - no repo / detached HEAD / non-GitHub origin → no PR context exists,
    //     so cache an empty `"{}"` as fresh (the chip correctly shows nothing).
    //   - `Some(json)` → a real PR (or a definitive "branch has no PR" `"{}"`);
    //     cache it as fresh.
    //   - `None` → the fetch itself failed (rate limit, network, missing
    //     token). DON'T overwrite the last-known-good json and DON'T advance
    //     `fetched_at` — otherwise a transient failure blanks the chip and
    //     freezes that blank as "fresh" for a whole TTL, so it never recovers.
    //     Instead keep the previous state and stamp `locked_at` as a short
    //     retry backoff (`ttl.max(10)`s) so the next render re-attempts soon
    //     without hammering the API every tick.
    let fetched = match repo_identity(&cwd) {
        Some((owner, name, branch)) => crate::github::pr_view_json(&owner, &name, &branch),
        None => Some("{}".into()), // no PR context → definitive empty
    };

    // Re-acquire to commit the result. The state is re-read from disk here, so
    // anything another worker wrote during the fetch is preserved.
    let mut handle = match StateLock::acquire_blocking(session_id) {
        Ok(h) => h,
        Err(_) => return,
    };

    match fetched {
        Some(body) => {
            handle.state.pr.json = body;
            handle.state.pr.fetched_at = now_epoch();
            handle.state.pr.locked_at = 0;
        }
        None => {
            // Preserve last-known-good json; leave fetched_at stale so we retry.
            handle.state.pr.locked_at = now_epoch();
        }
    }
    let _ = handle.save();
}

pub fn run_refresh_other(session_id: &str) {
    let transcript = std::env::var(ENV_TRANSCRIPT).unwrap_or_default();
    {
        // Stamp the debounce, then release before the scan — the transcript
        // runs to tens of MB and a foreground render waits on this lock.
        let mut handle = match StateLock::acquire_blocking(session_id) {
            Ok(h) => h,
            Err(_) => return,
        };
        handle.state.other_prs.locked_at = now_epoch();
        let _ = handle.save();
    }

    // All PR URLs referenced in the transcript (created + linked), scanned
    // in-process — the equivalent of `cc-thread-prs --urls-only --all`. This
    // captures PRs created out-of-band (e.g. via Graphite `gt`) and ones the
    // conversation merely touched. The chips component collapses a large set
    // to a `×N` summary.
    let new_urls = crate::transcript::pr_urls_in_transcript(&transcript);

    let mut handle = match StateLock::acquire_blocking(session_id) {
        Ok(h) => h,
        Err(_) => return,
    };

    // Detect newly-created PRs in this session and force-refresh the global
    // recent_prs cache so the chip lights up with state color immediately,
    // instead of waiting up to `recent_prs_ttl` seconds.
    let prev: std::collections::HashSet<String> =
        handle.state.other_prs.urls.iter().cloned().collect();
    let has_new = new_urls.iter().any(|u| !prev.contains(u));

    // Union: keep all previously-seen URLs (so /compact rewriting the
    // transcript doesn't drop chip history) and append any new ones in
    // discovery order. Chips never age out — the chips component collapses
    // to a `×N` summary when there are too many to render.
    for u in new_urls {
        if !prev.contains(&u) {
            handle.state.other_prs.urls.push(u);
        }
    }
    handle.state.other_prs.fetched_at = now_epoch();

    if has_new {
        invalidate_recent_prs();
    }

    // States are now hydrated from the global recent_prs cache, which is
    // refreshed by `--refresh-recent-prs` in one GraphQL call shared across
    // sessions. We just record the URL list and exit.
    handle.state.other_prs.locked_at = 0;
    let _ = handle.save();
}

/// Async Graphite stack refresh. Runs `gt log --json` in $CWD; on success,
/// flattens the JSON into a trunk-first list of `StackEntry`. On any failure
/// (gt missing, non-zero exit, malformed JSON), `is_gt` stays `false` so the
/// chips component falls back to its legacy ascending-PR-number rendering —
/// no error surface to the user.
pub fn run_refresh_stack(session_id: &str) {
    let cwd = std::env::var(ENV_STACK_CWD).unwrap_or_default();
    if cwd.is_empty() {
        return;
    }
    {
        // Stamp the debounce, then release before shelling out to `gt` — a
        // foreground render waits on this same lock.
        let mut handle = match StateLock::acquire_blocking(session_id) {
            Ok(h) => h,
            Err(_) => return,
        };
        let ttl = config::config().stack_refresh_ttl();
        if state::fresh(handle.state.stack.fetched_at, ttl) {
            return;
        }
        handle.state.stack.locked_at = now_epoch();
        let _ = handle.save();
    }

    let (is_gt, entries) = fetch_stack(&cwd);

    let mut handle = match StateLock::acquire_blocking(session_id) {
        Ok(h) => h,
        Err(_) => return,
    };
    handle.state.stack.is_gt = is_gt;
    handle.state.stack.entries = entries;
    handle.state.stack.fetched_at = now_epoch();
    handle.state.stack.locked_at = 0;
    let _ = handle.save();
}

fn fetch_stack(cwd: &str) -> (bool, Vec<state::StackEntry>) {
    let out = match Command::new("gt")
        .args(["log", "--json"])
        .current_dir(cwd)
        .stderr(Stdio::null())
        .output()
    {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => o.stdout,
        _ => return (false, Vec::new()),
    };
    let v: serde_json::Value = match serde_json::from_slice(&out) {
        Ok(v) => v,
        Err(_) => return (false, Vec::new()),
    };
    // `gt log --json` emits a flat array of entries. Each entry has at least
    // `branch`, `parentBranch` (null for trunk), and optionally `prNumber`.
    // We tolerate a top-level object containing the array under e.g.
    // "branches" or "log" — best-effort to insulate against minor schema
    // shifts in newer gt versions.
    let arr = if let Some(a) = v.as_array() {
        a.clone()
    } else if let Some(a) = v.get("branches").and_then(|x| x.as_array()) {
        a.clone()
    } else if let Some(a) = v.get("log").and_then(|x| x.as_array()) {
        a.clone()
    } else {
        return (false, Vec::new());
    };
    if arr.is_empty() {
        return (false, Vec::new());
    }
    // Build branch→parent map and a branch→pr map.
    use std::collections::HashMap;
    let mut parent: HashMap<String, Option<String>> = HashMap::new();
    let mut pr_of: HashMap<String, Option<u32>> = HashMap::new();
    for e in &arr {
        let branch = e
            .get("branch")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if branch.is_empty() {
            continue;
        }
        let parent_branch = e
            .get("parentBranch")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let pr = e.get("prNumber").and_then(|x| x.as_u64()).map(|n| n as u32);
        parent.insert(branch.clone(), parent_branch);
        pr_of.insert(branch, pr);
    }
    // Compute depth for each branch by walking up to a root (parent==None or
    // missing). Cap walk length to avoid pathological cycles.
    let mut entries: Vec<state::StackEntry> = parent
        .keys()
        .map(|b| state::StackEntry {
            branch: b.clone(),
            pr: pr_of.get(b).copied().flatten(),
            depth: depth_of(b, &parent),
        })
        .collect();
    entries.sort_by_key(|e| (e.depth, e.branch.clone()));
    (true, entries)
}

fn depth_of(start: &str, parent: &std::collections::HashMap<String, Option<String>>) -> u32 {
    let mut d: u32 = 0;
    let mut cur = start.to_string();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    loop {
        if !seen.insert(cur.clone()) {
            break; // cycle guard
        }
        match parent.get(&cur) {
            Some(Some(p)) if !p.is_empty() => {
                d += 1;
                cur = p.clone();
            }
            _ => break,
        }
        if d > 64 {
            break;
        }
    }
    d
}

/// Force the global recent-PRs cache to be considered stale on the next
/// render, AND eagerly spawn a refresh worker now. Called when this session
/// just created a PR, so the chip lights up with state color immediately.
fn invalidate_recent_prs() {
    let mut cur = crate::recent_prs::RecentPrs::load();
    cur.fetched_at = 0;
    cur.locked_at = 0;
    let _ = cur.save();
    crate::recent_prs::maybe_spawn_refresh();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A merged current-branch PR is terminal, so it must be recognised and
    /// held far longer than `pr_cache_ttl`. Re-polling it every 60s across
    /// every live session is where this tool's GraphQL budget actually went.
    #[test]
    fn merged_pr_json_is_detected() {
        assert!(cached_pr_merged(r#"{"state":"MERGED","number":35}"#));
    }

    /// Every non-terminal state keeps the normal short TTL — an open PR's
    /// checks, review decision and merge state all still move.
    #[test]
    fn non_merged_states_keep_short_ttl() {
        assert!(!cached_pr_merged(r#"{"state":"OPEN"}"#));
        assert!(!cached_pr_merged(r#"{"state":"CLOSED"}"#));
    }

    /// The empty-cache and malformed cases must not be mistaken for merged, or
    /// a branch with no PR yet would stop being polled for half an hour.
    #[test]
    fn empty_or_malformed_json_is_not_merged() {
        assert!(!cached_pr_merged("{}"));
        assert!(!cached_pr_merged(""));
        assert!(!cached_pr_merged("not json"));
        assert!(!cached_pr_merged(r#"{"state":null}"#));
    }
}
