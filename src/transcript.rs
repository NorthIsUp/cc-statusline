// Transcript-derived metrics: token burn rate, agent counter, "other PRs"
// chips. Caches live in the per-session state TOML; PR-related data is
// refreshed asynchronously by `refresh::run_refresh_other`. The local
// transcript scans (burn, agents) run inline since they only touch a local
// file and use mtime-keyed caching.

use crate::cache::mtime;
use crate::input::Session;
use crate::state::State;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

fn url_repo(url: &str) -> Option<String> {
    url.strip_prefix("https://github.com/")
        .and_then(|s| s.split_once("/pull/"))
        .map(|(repo, _)| repo.to_string())
}

fn origin_to_repo(origin_url: &str) -> String {
    // git@github.com:org/repo.git or https://github.com/org/repo(.git)
    let s = origin_url.trim_end_matches(".git");
    let parts: Vec<&str> = s.split(['/', ':']).filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 {
        format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
    } else {
        String::new()
    }
}

#[derive(Debug, Default)]
pub struct OtherPrs {
    pub urls: Vec<String>,
    pub states: HashMap<String, PrStateLite>,
    /// Whether the current worktree is a Graphite stack (gt log --json
    /// succeeded). When true, `stack_entries` is non-empty and trunk-first.
    pub is_gt: bool,
    /// Trunk-first branches with optional PR numbers and depth-from-trunk.
    pub stack_entries: Vec<StackChipEntry>,
}

#[derive(Debug, Default, Clone)]
pub struct StackChipEntry {
    pub branch: String,
    pub pr: Option<u32>,
    pub depth: u32,
}

#[derive(Debug, Default)]
pub struct PrStateLite {
    pub state: String,
    pub is_draft: bool,
    /// Unix epoch seconds the PR was merged, or `None` if open/closed/unknown.
    pub merged_at: Option<i64>,
    /// True iff GitHub's automerge is queued for this PR.
    pub auto_merge: bool,
}

pub fn other_prs_view(st: &State, origin_url: &str) -> OtherPrs {
    let mut out = OtherPrs::default();
    let own_repo = origin_to_repo(origin_url);
    out.urls = st
        .other_prs
        .urls
        .iter()
        .filter(|u| own_repo.is_empty() || url_repo(u).map(|r| r == own_repo).unwrap_or(false))
        .cloned()
        .collect();
    // Hydrate state from the global recent_prs cache (one GraphQL call shared
    // across all sessions). PRs older than the recent 100 won't be in the
    // cache; their chips render dim, which is acceptable.
    let recent = crate::recent_prs::RecentPrs::load();
    for url in &out.urls {
        if let Some(entry) = recent.prs.get(url) {
            out.states.insert(
                url.clone(),
                PrStateLite {
                    state: entry.state.clone(),
                    is_draft: entry.is_draft,
                    merged_at: entry.merged_at,
                    auto_merge: entry.auto_merge,
                },
            );
        }
    }
    out.is_gt = st.stack.is_gt;
    out.stack_entries = st
        .stack
        .entries
        .iter()
        .map(|e| StackChipEntry {
            branch: e.branch.clone(),
            pr: e.pr,
            depth: e.depth,
        })
        .collect();
    out
}

#[derive(Debug, Default)]
pub struct BurnInfo {
    pub tokens_per_hour: u64,
    pub tokens_total: u64,
}

pub fn burn_rate(session: &Session, st: &mut State) -> BurnInfo {
    let mut out = BurnInfo::default();
    if session.transcript.is_empty()
        || !Path::new(&session.transcript).exists()
        || session.duration_ms <= 60_000
    {
        return out;
    }
    let tm = mtime(Path::new(&session.transcript)).unwrap_or(0);
    if st.burn.transcript_mtime != tm {
        st.burn.transcript_mtime = tm;
        st.burn.total_tokens = sum_transcript_tokens(&session.transcript);
    }
    if st.burn.total_tokens == 0 {
        return out;
    }
    out.tokens_total = st.burn.total_tokens;
    out.tokens_per_hour = st
        .burn
        .total_tokens
        .saturating_mul(3_600_000)
        .checked_div(session.duration_ms.max(1) as u64)
        .unwrap_or(0);
    out
}

fn sum_transcript_tokens(path: &str) -> u64 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return 0,
    };
    let mut total: u64 = 0;
    for line in text.lines() {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let usage = match v
            .get("message")
            .and_then(|m| m.get("usage"))
            .and_then(|u| u.as_object())
        {
            Some(u) => u,
            None => continue,
        };
        let pick = |k| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        total += pick("input_tokens")
            + pick("output_tokens")
            + pick("cache_creation_input_tokens")
            + pick("cache_read_input_tokens");
    }
    total
}

#[derive(Debug, Default)]
pub struct AgentCount {
    pub active: u32,
    pub total: u32,
}

pub fn agent_counter(session: &Session, st: &mut State) -> AgentCount {
    let mut out = AgentCount::default();
    if session.transcript.is_empty() || !Path::new(&session.transcript).exists() {
        return out;
    }
    let tm = mtime(Path::new(&session.transcript)).unwrap_or(0);
    if st.agents.transcript_mtime != tm {
        let (a, t) = compute_agents(&session.transcript);
        st.agents.transcript_mtime = tm;
        st.agents.active = a;
        st.agents.total = t;
    }
    out.active = st.agents.active;
    out.total = st.agents.total;
    out
}

fn compute_agents(path: &str) -> (u32, u32) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return (0, 0),
    };
    let mut open: HashSet<String> = HashSet::new();
    let mut total: u32 = 0;
    for line in text.lines() {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let arr = v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array());
        let arr = match arr {
            Some(a) => a,
            None => continue,
        };
        for item in arr {
            let typ = item.get("type").and_then(|x| x.as_str()).unwrap_or("");
            match typ {
                "tool_use" => {
                    if item.get("name").and_then(|x| x.as_str()) == Some("Agent") {
                        if let Some(id) = item.get("id").and_then(|x| x.as_str()) {
                            open.insert(id.to_string());
                            total += 1;
                        }
                    }
                }
                "tool_result" => {
                    if let Some(id) = item.get("tool_use_id").and_then(|x| x.as_str()) {
                        open.remove(id);
                    }
                }
                _ => {}
            }
        }
    }
    (open.len() as u32, total)
}
