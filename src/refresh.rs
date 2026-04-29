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

pub fn maybe_spawn_pr(session_id: &str, cwd: &str, st: &state::State) {
    let ttl = config::config().pr_cache_ttl();
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

    // Mark the in-flight lock so concurrent foregrounds know not to re-spawn.
    handle.state.pr.locked_at = now_epoch();
    let _ = handle.save();

    let body = match Command::new("gh")
        .args([
            "pr",
            "view",
            "--json",
            "state,isDraft,reviewDecision,comments,statusCheckRollup,url,number",
        ])
        .current_dir(&cwd)
        // Force gh's keychain credential, not whatever stale GITHUB_TOKEN
        // might be in the environment. The user's shell often exports a
        // narrow-scope token from another tool; unset it so gh uses the
        // properly-authed keychain identity.
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_TOKEN")
        .stderr(Stdio::null())
        .output()
    {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => {
            String::from_utf8(o.stdout).unwrap_or_default()
        }
        _ => "{}".into(),
    };

    handle.state.pr.json = body;
    handle.state.pr.fetched_at = now_epoch();
    handle.state.pr.locked_at = 0;
    let _ = handle.save();
}

pub fn run_refresh_other(session_id: &str) {
    let transcript = std::env::var(ENV_TRANSCRIPT).unwrap_or_default();
    let mut handle = match StateLock::acquire_blocking(session_id) {
        Ok(h) => h,
        Err(_) => return,
    };

    let helper = format!(
        "{}/my/bin/cc-thread-prs",
        std::env::var("HOME").unwrap_or_default()
    );
    if !std::path::Path::new(&helper).exists() {
        return;
    }
    handle.state.other_prs.locked_at = now_epoch();
    let _ = handle.save();

    // PRs *created* by this session only. Cross-repo bleed is impossible at
    // this layer because cc-thread-prs only emits a URL when a tool_use in
    // the transcript actually created a PR.
    if let Ok(out) = Command::new(&helper)
        .args(["--urls-only", "--transcript", &transcript])
        .stderr(Stdio::null())
        .output()
    {
        let new_urls: Vec<String> = String::from_utf8(out.stdout)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect();

        // Detect newly-created PRs in this session and force-refresh the
        // global recent_prs cache so the chip lights up with state color
        // immediately, instead of waiting up to `recent_prs_ttl` seconds.
        let prev: std::collections::HashSet<&String> = handle.state.other_prs.urls.iter().collect();
        let has_new = new_urls.iter().any(|u| !prev.contains(u));

        handle.state.other_prs.urls = new_urls;
        handle.state.other_prs.fetched_at = now_epoch();

        if has_new {
            invalidate_recent_prs();
        }
    }

    // States are now hydrated from the global recent_prs cache, which is
    // refreshed by `--refresh-recent-prs` in one GraphQL call shared across
    // sessions. We just record the URL list and exit.
    handle.state.other_prs.locked_at = 0;
    let _ = handle.save();
}

/// Fetch states for many PRs in one GraphQL call instead of N `gh pr view`s.
/// Each PR view is a single GraphQL query; batching them into aliased fields
/// of one query brings N requests → 1, which is the difference between
/// surviving and exhausting the 5000/hr GitHub API budget when many sessions
/// each track several chips.
fn fetch_other_states(urls: &[String]) -> String {
    if urls.is_empty() {
        return "{}".into();
    }
    // Build aliased query: pr0: repository(owner:"o", name:"r") { pullRequest(number: N) {...} }
    let mut query = String::from("query {");
    let mut url_by_alias: Vec<(String, String)> = Vec::new();
    for (i, u) in urls.iter().enumerate() {
        let (owner, name, num) = match parse_pr_url(u) {
            Some(t) => t,
            None => continue,
        };
        let alias = format!("pr{i}");
        query.push_str(&format!(
            r#"{alias}: repository(owner: "{owner}", name: "{name}") {{ pullRequest(number: {num}) {{ url state isDraft }} }} "#
        ));
        url_by_alias.push((alias, u.clone()));
    }
    query.push('}');

    let out = Command::new("gh")
        .args(["api", "graphql", "-f", &format!("query={query}")])
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_TOKEN")
        .stderr(Stdio::null())
        .output();
    let body = match out {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => o.stdout,
        _ => return "{}".into(),
    };
    let v: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return "{}".into(),
    };
    let data = match v.get("data") {
        Some(d) => d,
        None => return "{}".into(),
    };

    let mut acc = serde_json::Map::new();
    for (alias, url) in url_by_alias {
        let pr = match data.get(&alias).and_then(|r| r.get("pullRequest")) {
            Some(p) if !p.is_null() => p,
            _ => continue,
        };
        let mut entry = serde_json::Map::new();
        if let Some(s) = pr.get("state") {
            entry.insert("state".into(), s.clone());
        }
        if let Some(d) = pr.get("isDraft") {
            entry.insert("isDraft".into(), d.clone());
        }
        acc.insert(url, serde_json::Value::Object(entry));
    }
    serde_json::to_string(&serde_json::Value::Object(acc)).unwrap_or_default()
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

fn parse_pr_url(u: &str) -> Option<(String, String, u64)> {
    let rest = u.strip_prefix("https://github.com/")?;
    let (repo, num_part) = rest.split_once("/pull/")?;
    let (owner, name) = repo.split_once('/')?;
    let num: u64 = num_part.split(['/', '?', '#']).next()?.parse().ok()?;
    Some((owner.to_string(), name.to_string(), num))
}
