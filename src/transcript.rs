// Transcript-derived metrics: token burn rate, agent counter, "other PRs"
// chips. Caches live in the per-session state TOML; PR-related data is
// refreshed asynchronously by `refresh::run_refresh_other`. The local
// transcript scans (burn, agents) run inline since they only touch a local
// file and use mtime-keyed caching.

use crate::input::Session;
use crate::state::{State, TranscriptScan};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::OnceLock;

/// Every PR URL Claude *wrote about* in the transcript at `path`, with
/// Graphite PR links (`app.graphite.com/github/pr/O/R/N`) normalised to the
/// canonical `github.com/O/R/pull/N` form, deduped in first-appearance order.
///
/// Only assistant prose and what the human typed count. A URL that appears
/// solely inside a `tool_result` is data the agent scrolled past, not a PR the
/// conversation is about: one `gh pr list --json ...url` drops ~40 URLs into
/// the transcript, and every one of them used to become a chip. Measured on
/// the session that prompted this, the split was 1 relevant URL in assistant
/// text against 49 in tool output.
///
/// ponytail: a PR created this turn lands in `gh pr create`'s tool_result
/// first, so its chip waits until the assistant mentions the URL (usually the
/// same turn) or the branch lookup finds it. Whitelisting creation commands is
/// the upgrade if that lag ever shows.
///
/// Returns the URLs in first-appearance order plus the *last* one mentioned
/// anywhere in the file — the PR the conversation is currently on, which the
/// chips component renders bold.
pub fn pr_urls_in_transcript(path: &str, offset: u64) -> (Vec<String>, Option<String>, u64) {
    let (text, _, end) = match read_tail(path, offset) {
        Some(t) => t,
        None => return (Vec::new(), None, offset),
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut latest = None;
    for line in text.lines() {
        for url in scan_pr_urls(&relevant_text(line)) {
            latest = Some(url.clone());
            if seen.insert(url.clone()) {
                out.push(url);
            }
        }
    }
    (out, latest, end)
}

/// The complete lines appended to `path` since byte `offset`, plus where they
/// start and end. A file shorter than `offset` was rewritten, so it is read
/// from 0 and `start != offset` tells the caller to drop its totals. A partial
/// last line is left for the next call.
///
/// shortcut: a rewrite that leaves the file longer than `offset` reads as an
/// append. Claude Code only appends today; store a hash of the first line to
/// catch rewrites if that changes.
fn read_tail(path: &str, offset: u64) -> Option<(String, u64, u64)> {
    let mut f = File::open(path).ok()?;
    let start = if f.metadata().ok()?.len() < offset {
        0
    } else {
        offset
    };
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let len = buf.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    buf.truncate(len);
    Some((String::from_utf8(buf).ok()?, start, start + len as u64))
}

/// The parts of one transcript JSONL line a PR URL may be harvested from:
/// assistant/user `text` blocks only. System-reminders are stripped — they
/// carry injected CLAUDE.md prose, whose example URLs are not the user's.
fn relevant_text(line: &str) -> String {
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    if !matches!(
        v.get("type").and_then(Value::as_str),
        Some("assistant" | "user")
    ) {
        return String::new();
    }
    let content = &v["message"]["content"];
    let mut out = String::new();
    match content {
        Value::String(s) => out.push_str(s),
        Value::Array(blocks) => {
            for b in blocks {
                if b.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(t) = b.get("text").and_then(Value::as_str) {
                        out.push_str(t);
                        out.push('\n');
                    }
                }
            }
        }
        _ => {}
    }
    strip_system_reminders(&out)
}

fn strip_system_reminders(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("<system-reminder>") {
        out.push_str(&rest[..i]);
        rest = match rest[i..].find("</system-reminder>") {
            Some(j) => &rest[i + j + "</system-reminder>".len()..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

fn scan_pr_urls(text: &str) -> Vec<String> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(
            r"https://(?:github\.com/[A-Za-z0-9._-]+/[A-Za-z0-9._-]+/pull/[0-9]+|app\.graphite\.com/github/pr/[A-Za-z0-9._-]+/[A-Za-z0-9._-]+/[0-9]+)",
        )
        .expect("static PR-URL regex is valid")
    });
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for m in re.find_iter(text) {
        let url = normalize_graphite_pr_url(m.as_str());
        if seen.insert(url.clone()) {
            out.push(url);
        }
    }
    out
}

/// `app.graphite.com/github/pr/OWNER/REPO/N` → `github.com/OWNER/REPO/pull/N`;
/// already-canonical github URLs pass through unchanged.
fn normalize_graphite_pr_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("https://app.graphite.com/github/pr/") {
        let p: Vec<&str> = rest.split('/').collect();
        if p.len() == 3 {
            return format!("https://github.com/{}/{}/pull/{}", p[0], p[1], p[2]);
        }
    }
    url.to_string()
}

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
    /// The most recently mentioned PR in the transcript; rendered bold.
    pub latest: String,
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
    // A stack is shown whole: every branch in it with a PR gets a chip, even
    // one the conversation never mentioned. Half a stack is worse than none —
    // the chain is the unit you reason about, and since chips are now
    // harvested from chat text only, the siblings would otherwise vanish.
    if !own_repo.is_empty() {
        let seen: HashSet<&str> = out.urls.iter().map(String::as_str).collect();
        let mut add: Vec<String> = Vec::new();
        for e in &st.stack.entries {
            if let Some(n) = e.pr {
                let url = format!("https://github.com/{own_repo}/pull/{n}");
                if !seen.contains(url.as_str()) && !add.contains(&url) {
                    add.push(url);
                }
            }
        }
        out.urls.extend(add);
    }
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
    out.latest = st.other_prs.latest.clone();
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

/// Burn and agent totals, advanced by parsing only the transcript lines added
/// since the last render. Reparsing the whole file on every change cost ~80ms
/// of foreground CPU per turn on a 37MB transcript, times every session.
pub fn scan(session: &Session, st: &mut State) -> (BurnInfo, AgentCount) {
    let mut burn = BurnInfo::default();
    let mut agents = AgentCount::default();
    if session.transcript.is_empty() {
        return (burn, agents);
    }

    if let Some((text, start, end)) = read_tail(&session.transcript, st.transcript.offset) {
        if start != st.transcript.offset {
            st.transcript = TranscriptScan::default();
        }
        accumulate(&mut st.transcript, &text);
        st.transcript.offset = end;
    }

    agents.active = st.transcript.open_agents.len() as u32;
    agents.total = st.transcript.agents_total;

    // Burn needs a minute of wall time before a rate means anything.
    if session.duration_ms > 60_000 && st.transcript.tokens > 0 {
        burn.tokens_total = st.transcript.tokens;
        burn.tokens_per_hour = st
            .transcript
            .tokens
            .saturating_mul(3_600_000)
            .checked_div(session.duration_ms.max(1) as u64)
            .unwrap_or(0);
    }
    (burn, agents)
}

/// Fold JSONL `text` into the running totals: one JSON parse per line, both
/// metrics from the same pass.
fn accumulate(scan: &mut TranscriptScan, text: &str) {
    let mut open: HashSet<String> = scan.open_agents.drain(..).collect();
    for line in text.lines() {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let msg = match v.get("message") {
            Some(m) => m,
            None => continue,
        };
        if let Some(usage) = msg.get("usage").and_then(|u| u.as_object()) {
            let pick = |k| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
            scan.tokens += pick("input_tokens")
                + pick("output_tokens")
                + pick("cache_creation_input_tokens")
                + pick("cache_read_input_tokens");
        }
        let arr = match msg.get("content").and_then(|c| c.as_array()) {
            Some(a) => a,
            None => continue,
        };
        for item in arr {
            match item.get("type").and_then(|x| x.as_str()).unwrap_or("") {
                "tool_use" => {
                    if item.get("name").and_then(|x| x.as_str()) == Some("Agent") {
                        if let Some(id) = item.get("id").and_then(|x| x.as_str()) {
                            open.insert(id.to_string());
                            scan.agents_total += 1;
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
    scan.open_agents = open.into_iter().collect();
    scan.open_agents.sort();
}

#[derive(Debug, Default)]
pub struct AgentCount {
    pub active: u32,
    pub total: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_pr_urls_dedups_normalizes_and_ignores_non_prs() {
        let text = r#"
            opened https://github.com/o/r/pull/12 then mentioned
            https://github.com/o/r/pull/12 again; a graphite link
            https://app.graphite.com/github/pr/team/repo/345 ; and ignore
            https://github.com/o/r/issues/9 plus https://github.com/o/r (no pull).
        "#;
        assert_eq!(
            scan_pr_urls(text),
            vec![
                "https://github.com/o/r/pull/12".to_string(),
                "https://github.com/team/repo/pull/345".to_string(),
            ]
        );
    }

    #[test]
    fn pr_urls_only_from_chat_text_not_tool_output() {
        let lines = [
            r#"{"type":"user","message":{"content":[{"type":"text","text":"look at https://github.com/o/r/pull/1"}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"{\"url\":\"https://github.com/o/r/pull/99\"}"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"PR https://github.com/o/r/pull/2 is the one"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","input":{"command":"gh pr view https://github.com/o/r/pull/98"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"text","text":"<system-reminder>example https://github.com/org/repo/pull/44</system-reminder>ok"}]}}"#,
            r#"{"type":"attachment","content":"https://github.com/org/repo/pull/97"}"#,
            "not json",
        ];
        let dir = std::env::temp_dir().join(format!("cc-prurl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.jsonl");
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        assert_eq!(
            pr_urls_in_transcript(p.to_str().unwrap(), 0).0,
            vec![
                "https://github.com/o/r/pull/1".to_string(),
                "https://github.com/o/r/pull/2".to_string(),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stack_prs_join_chips_and_latest_is_last_mentioned() {
        let mut st = State::default();
        st.other_prs.urls = vec!["https://github.com/o/r/pull/1".into()];
        st.other_prs.latest = "https://github.com/o/r/pull/1".into();
        st.stack.is_gt = true;
        st.stack.entries = vec![
            crate::state::StackEntry {
                branch: "a".into(),
                pr: Some(7),
                depth: 1,
            },
            crate::state::StackEntry {
                branch: "b".into(),
                pr: None,
                depth: 2,
            },
        ];
        let v = other_prs_view(&st, "git@github.com:o/r.git");
        assert_eq!(
            v.urls,
            vec![
                "https://github.com/o/r/pull/1".to_string(),
                "https://github.com/o/r/pull/7".to_string(),
            ]
        );
        assert_eq!(v.latest, "https://github.com/o/r/pull/1");
    }

    #[test]
    fn latest_is_the_last_mention_not_the_first() {
        let lines = [
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"https://github.com/o/r/pull/1"}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"text","text":"now https://github.com/o/r/pull/2"}]}}"#,
        ];
        let dir = std::env::temp_dir().join(format!("cc-latest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.jsonl");
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        let (urls, latest, _) = pr_urls_in_transcript(p.to_str().unwrap(), 0);
        assert_eq!(urls.len(), 2);
        assert_eq!(latest.unwrap(), "https://github.com/o/r/pull/2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_pr_urls_empty_when_none() {
        assert!(scan_pr_urls("no links here").is_empty());
    }

    fn scanned(text: &str) -> TranscriptScan {
        let mut scan = TranscriptScan::default();
        accumulate(&mut scan, text);
        scan
    }

    /// The merged pass must still total usage across every shape of line and
    /// leave the agent counter agreeing with the tool_use/tool_result pairing:
    /// two Agents dispatched, one returned, so one stays open.
    #[test]
    fn accumulate_totals_tokens_and_pairs_agents() {
        let got = scanned(
            &[
                r#"{"message":{"usage":{"input_tokens":10,"output_tokens":5,"cache_creation_input_tokens":1,"cache_read_input_tokens":2}}}"#,
                r#"{"message":{"content":[{"type":"tool_use","name":"Agent","id":"a1"},{"type":"tool_use","name":"Bash","id":"b1"}]}}"#,
                r#"{"message":{"content":[{"type":"tool_use","name":"Agent","id":"a2"}],"usage":{"input_tokens":7}}}"#,
                r#"{"message":{"content":[{"type":"tool_result","tool_use_id":"a1"}]}}"#,
                "not json at all",
                r#"{"no_message":true}"#,
            ]
            .join("\n"),
        );
        assert_eq!(got.tokens, 25); // 10+5+1+2 then +7
        assert_eq!(got.open_agents, vec!["a2".to_string()]); // a1 returned
        assert_eq!(got.agents_total, 2); // Bash is not an Agent
    }

    /// A line carrying both usage and content must be counted by both halves —
    /// the failure mode of merging two loops into one is an early `continue`
    /// that skips the other metric.
    #[test]
    fn accumulate_counts_both_metrics_on_one_line() {
        let got = scanned(
            r#"{"message":{"usage":{"input_tokens":3},"content":[{"type":"tool_use","name":"Agent","id":"x"}]}}"#,
        );
        assert_eq!(got.tokens, 3, "usage on an Agent line must still count");
        assert_eq!(
            got.agents_total, 1,
            "Agent on a usage line must still count"
        );
    }

    /// Renders between appends must land on the same totals as one full pass:
    /// an Agent opened in one chunk and returned in the next closes, a partial
    /// last line waits for its newline, and a file that shrank (rewritten) is
    /// recounted from scratch rather than added on top.
    #[test]
    fn scan_tail_matches_a_full_pass_and_restarts_on_rewrite() {
        let dir = std::env::temp_dir().join(format!("cc-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.jsonl");
        let session = Session {
            transcript: p.to_str().unwrap().into(),
            ..Default::default()
        };
        let open = r#"{"message":{"usage":{"input_tokens":4},"content":[{"type":"tool_use","name":"Agent","id":"a"}]}}"#;
        let close = r#"{"message":{"usage":{"input_tokens":6},"content":[{"type":"tool_result","tool_use_id":"a"}]}}"#;
        let mut st = State::default();

        std::fs::write(&p, format!("{open}\n{}", &close[..10])).unwrap();
        scan(&session, &mut st);
        assert_eq!(
            (st.transcript.tokens, st.transcript.open_agents.len()),
            (4, 1)
        );

        std::fs::write(&p, format!("{open}\n{close}\n")).unwrap();
        scan(&session, &mut st);
        let full = TranscriptScan {
            offset: std::fs::metadata(&p).unwrap().len(),
            ..scanned(&format!("{open}\n{close}\n"))
        };
        assert_eq!(st.transcript, full);
        assert_eq!(
            (st.transcript.tokens, st.transcript.open_agents.len()),
            (10, 0)
        );

        std::fs::write(&p, format!("{close}\n")).unwrap();
        scan(&session, &mut st);
        assert_eq!((st.transcript.tokens, st.transcript.agents_total), (6, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The URL scan resumes where it stopped: a second call sees only the new
    /// mention, and its offset covers the whole file.
    #[test]
    fn pr_urls_resume_from_offset() {
        let dir = std::env::temp_dir().join(format!("cc-prtail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.jsonl");
        let say = |n: u32| {
            format!(
                r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"https://github.com/o/r/pull/{n}"}}]}}}}"#
            )
        };
        std::fs::write(&p, say(1) + "\n").unwrap();
        let (_, _, off) = pr_urls_in_transcript(p.to_str().unwrap(), 0);
        std::fs::write(&p, say(1) + "\n" + &say(2) + "\n").unwrap();
        let (urls, latest, end) = pr_urls_in_transcript(p.to_str().unwrap(), off);
        assert_eq!(urls, vec!["https://github.com/o/r/pull/2".to_string()]);
        assert_eq!(latest.unwrap(), "https://github.com/o/r/pull/2");
        assert_eq!(end, std::fs::metadata(&p).unwrap().len());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
