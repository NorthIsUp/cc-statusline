//! Direct GitHub GraphQL API — replaces shelling out to `gh api graphql` and
//! `gh pr view`. Auth comes from the `GH_TOKEN` / `GITHUB_TOKEN` env var (the
//! statusline inherits the user's shell environment).
//!
//! Every call degrades to `None`/`"{}"` on any failure (missing token,
//! network, HTTP error, malformed JSON) — callers fall back exactly as they
//! did when the `gh` subprocess failed, so an absent token just means no PR
//! chip rather than an error.

use serde_json::{Map, Value};
use std::sync::OnceLock;

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

    let v = graphql(&query)?;
    let nodes = v
        .pointer("/data/repository/pullRequests/nodes")
        .and_then(Value::as_array);
    let node = match nodes.and_then(|n| n.first()) {
        Some(n) => n,
        None => return Some("{}".into()), // no PR for this branch
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
