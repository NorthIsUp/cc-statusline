//! Direct GitHub GraphQL API — replaces shelling out to `gh api graphql` and
//! `gh pr view`. Auth comes from the `GH_TOKEN` / `GITHUB_TOKEN` env var (the
//! statusline inherits the user's shell environment).
//!
//! Every call degrades to `None`/`"{}"` on any failure (missing token,
//! network, HTTP error, timeout, malformed JSON) — callers fall back as they
//! did when the `gh` subprocess failed, so an absent token just means no PR
//! chip rather than an error.

use serde_json::{Map, Value};
use std::sync::OnceLock;
use std::time::Duration;

/// Hard ceiling on a whole request. Without it a stalled read blocks in
/// `recvfrom` forever: the worker never fails, so it never stamps `locked_at`,
/// so the backoff added in 18f97a4 never engages and every later render spawns
/// another worker behind the same stale cache. Observed orphans held a socket
/// for 20h. A statusline refresh is worthless long before 10s anyway.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// GitHub token from the environment. Prefers `GH_TOKEN` (gh's own override)
/// then `GITHUB_TOKEN`.
pub fn token() -> Option<String> {
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

/// Shared agent pinned to the native-tls provider (macOS Secure Transport).
/// ureq 3.x defaults its provider to Rustls at runtime, so it must be set
/// explicitly or the first request panics.
fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        use ureq::config::Config;
        use ureq::tls::{TlsConfig, TlsProvider};
        Config::builder()
            .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
            .timeout_global(Some(HTTP_TIMEOUT))
            .build()
            .new_agent()
    })
}

/// POST a GraphQL `query` to api.github.com and return the full parsed
/// response (the `{"data": ...}` envelope), matching what `gh api graphql`
/// wrote to stdout.
pub fn graphql(query: &str) -> Option<Value> {
    let token = token()?;
    let mut resp = agent()
        .post("https://api.github.com/graphql")
        .header("Authorization", &format!("bearer {token}"))
        .header("User-Agent", "cc-statusline")
        .send_json(serde_json::json!({ "query": query }))
        .ok()?;
    resp.body_mut().read_json::<Value>().ok()
}

/// GraphQL-escape a string for inlining inside a query literal.
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// String field as `""` when absent or null — matches how `gh --json`
/// serialises null enums (`reviewDecision`, a CheckRun's pending
/// `conclusion`, etc.) so the downstream `PrJson` `String` fields parse.
fn str_or_empty(node: &Value, key: &str) -> Value {
    Value::String(node.get(key).and_then(Value::as_str).unwrap_or("").into())
}

/// The current branch's PR, as a JSON string in the exact shape
/// `gh pr view --json state,isDraft,reviewDecision,comments,statusCheckRollup,url,number,autoMergeRequest`
/// produced (which `git::PrJson` then deserialises). `None` on token/network
/// failure; `Some("{}")` when the branch has no PR.
pub fn pr_view_json(owner: &str, name: &str, branch: &str) -> Option<String> {
    let query = format!(
        r#"query {{
  repository(owner: "{o}", name: "{n}") {{
    pullRequests(headRefName: "{b}", first: 1, orderBy: {{field: CREATED_AT, direction: DESC}}) {{
      nodes {{
        state isDraft reviewDecision url number
        comments {{ totalCount }}
        autoMergeRequest {{ __typename }}
        commits(last: 1) {{ nodes {{ commit {{ statusCheckRollup {{ contexts(first: 100) {{ nodes {{
          __typename
          ... on CheckRun {{ conclusion status }}
        }} }} }} }} }} }}
      }}
    }}
  }}
}}"#,
        o = esc(owner),
        n = esc(name),
        b = esc(branch),
    );

    parse_pr_view(&graphql(&query)?)
}

/// Project a raw GraphQL response envelope into the `gh pr view --json …`
/// shape that `git::PrJson` deserialises. Split out from the network call so
/// the error-vs-empty distinction is unit-testable.
///
/// Returns:
///   - `None` — the response carries no usable `data` (a `{"data": null,
///     "errors": […]}` rate-limit/transient envelope, or an unresolvable
///     repo). Signals a *failed* fetch so the caller keeps last-known-good.
///   - `Some("{}")` — a valid response whose `nodes` array is empty: the
///     branch genuinely has no PR.
///   - `Some(json)` — the projected PR object.
fn parse_pr_view(v: &Value) -> Option<String> {
    // GitHub signals rate limits and transient server errors as HTTP 200 with
    // `{"data": null, "errors": [...]}`. Reading a missing `nodes` array as
    // "no PR" there would wipe a real chip; distinguish a valid-but-empty
    // response (branch genuinely has no PR → `Some("{}")`) from a failed one
    // (null `data` / errors / unresolvable repo → `None`, so the caller keeps
    // the last-known-good chip and retries).
    // A missing `nodes` array (null `data` / errors envelope / unresolvable
    // repo) propagates `None` → a *failed* fetch, distinct from the empty-array
    // "no PR" case handled just below.
    let nodes = v
        .pointer("/data/repository/pullRequests/nodes")
        .and_then(Value::as_array)?;
    let node = match nodes.first() {
        Some(n) => n,
        None => return Some("{}".into()), // valid response, no PR for this branch
    };

    let mut pr = Map::new();
    pr.insert("state".into(), str_or_empty(node, "state"));
    pr.insert(
        "isDraft".into(),
        Value::Bool(node.get("isDraft").and_then(Value::as_bool).unwrap_or(false)),
    );
    pr.insert("reviewDecision".into(), str_or_empty(node, "reviewDecision"));
    pr.insert("url".into(), str_or_empty(node, "url"));
    if let Some(num) = node.get("number").cloned() {
        pr.insert("number".into(), num);
    }
    // `comments` is only ever read via `.len()`; synthesise an array of the
    // right length rather than fetching every comment body.
    let total = node
        .pointer("/comments/totalCount")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    pr.insert("comments".into(), Value::Array(vec![Value::Null; total]));
    // Non-null iff automerge is enabled; PrJson only checks presence.
    pr.insert(
        "autoMergeRequest".into(),
        node.get("autoMergeRequest").cloned().unwrap_or(Value::Null),
    );
    // Flatten commit → statusCheckRollup → contexts into [{conclusion,status}].
    // StatusContext nodes have neither key → both become "" (ignored by
    // `ci_state`), preserving the prior `gh` behaviour.
    let rows: Vec<Value> = node
        .pointer("/commits/nodes/0/commit/statusCheckRollup/contexts/nodes")
        .and_then(Value::as_array)
        .map(|ns| {
            ns.iter()
                .map(|n| {
                    let mut row = Map::new();
                    row.insert("conclusion".into(), str_or_empty(n, "conclusion"));
                    row.insert("status".into(), str_or_empty(n, "status"));
                    Value::Object(row)
                })
                .collect()
        })
        .unwrap_or_default();
    pr.insert("statusCheckRollup".into(), Value::Array(rows));

    serde_json::to_string(&Value::Object(pr)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Guards the hang: a stalled request used to block in `recvfrom` forever,
    /// orphaning the worker and starving the cache. 203.0.113.1 is TEST-NET-3 —
    /// unroutable by definition, so it stalls the same way the real one did.
    #[test]
    fn agent_gives_up_instead_of_hanging() {
        let start = std::time::Instant::now();
        let r = agent()
            .post("https://203.0.113.1/graphql")
            .send_json(serde_json::json!({}));
        let elapsed = start.elapsed();
        assert!(r.is_err(), "unroutable address should not succeed");
        assert!(
            elapsed < HTTP_TIMEOUT + Duration::from_secs(5),
            "took {elapsed:?} — global timeout did not fire"
        );
    }

    /// A rate-limit / transient error envelope (HTTP 200, null data) must read
    /// as a *failure* (`None`), not as "branch has no PR" — otherwise the
    /// caller caches a blank chip as fresh and freezes it. This is the bug.
    #[test]
    fn error_envelope_is_failure_not_empty() {
        let v = json!({
            "data": null,
            "errors": [{ "type": "RATE_LIMITED", "message": "API rate limit exceeded" }],
        });
        assert_eq!(parse_pr_view(&v), None);
    }

    /// A valid response whose `nodes` array is empty means the branch genuinely
    /// has no PR — a definitive empty result, distinct from a failure.
    #[test]
    fn empty_nodes_is_no_pr() {
        let v = json!({
            "data": { "repository": { "pullRequests": { "nodes": [] } } },
        });
        assert_eq!(parse_pr_view(&v).as_deref(), Some("{}"));
    }

    /// `repository: null` (repo not found / no access) has no `nodes` array, so
    /// it is treated as a failure rather than silently blanking the chip.
    #[test]
    fn null_repository_is_failure() {
        let v = json!({ "data": { "repository": null } });
        assert_eq!(parse_pr_view(&v), None);
    }

    /// A real PR with automerge queued projects into a json blob whose
    /// `autoMergeRequest` is non-null, so `PrJson::auto_merge()` reports true.
    #[test]
    fn automerge_pr_projects_non_null_automerge() {
        let v = json!({
            "data": { "repository": { "pullRequests": { "nodes": [{
                "state": "OPEN",
                "isDraft": false,
                "reviewDecision": null,
                "url": "https://github.com/o/r/pull/1672",
                "number": 1672,
                "comments": { "totalCount": 0 },
                "autoMergeRequest": { "__typename": "AutoMergeRequest" },
                "commits": { "nodes": [] }
            }] } } },
        });
        let json = parse_pr_view(&v).expect("should project a PR");
        let pr: crate::git::PrJson = serde_json::from_str(&json).unwrap();
        assert_eq!(pr.state, "OPEN");
        assert_eq!(pr.number, Some(1672));
        assert!(pr.auto_merge(), "autoMergeRequest present → auto_merge() true");
    }

    /// An open PR without automerge projects a null `autoMergeRequest`, so
    /// `auto_merge()` is false (the plain-open color path).
    #[test]
    fn open_pr_without_automerge_is_false() {
        let v = json!({
            "data": { "repository": { "pullRequests": { "nodes": [{
                "state": "OPEN",
                "isDraft": false,
                "url": "https://github.com/o/r/pull/1",
                "number": 1,
                "comments": { "totalCount": 0 },
                "autoMergeRequest": null,
                "commits": { "nodes": [] }
            }] } } },
        });
        let json = parse_pr_view(&v).expect("should project a PR");
        let pr: crate::git::PrJson = serde_json::from_str(&json).unwrap();
        assert!(!pr.auto_merge());
    }
}
