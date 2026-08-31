// Git status (local, fast) + PR view-only-from-state. Async fetch lives in
// `refresh` and is dispatched by main before this runs.

use crate::input::Session;
use crate::state::State;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PrJson {
    pub state: String,
    #[serde(rename = "isDraft")]
    pub is_draft: bool,
    #[serde(rename = "reviewDecision")]
    pub review_decision: String,
    pub comments: Vec<serde_json::Value>,
    #[serde(rename = "statusCheckRollup")]
    pub status_check_rollup: Vec<CheckRow>,
    pub url: String,
    pub number: Option<u64>,
    /// RFC3339 merge timestamp, or empty when the PR is not merged.
    #[serde(rename = "mergedAt")]
    pub merged_at: String,
    /// Non-null iff automerge is enabled on the PR. We only care about
    /// presence/absence, so any JSON value is accepted.
    #[serde(rename = "autoMergeRequest")]
    pub auto_merge_request: Option<serde_json::Value>,
}

impl PrJson {
    pub fn auto_merge(&self) -> bool {
        self.auto_merge_request.is_some()
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct CheckRow {
    pub conclusion: String,
    pub status: String,
}

#[derive(Debug, Default)]
pub struct GitData {
    pub branch: String,
    pub dirty: u32,
    pub ahead: u32,
    pub behind: u32,
    pub pr: PrJson,
    pub origin_url: String,
    /// Repo/branch this session is on, published for the global worker.
    pub watch: crate::state::Watch,
    pub git_dir: Option<PathBuf>,
    pub common_dir: Option<PathBuf>,
    pub toplevel: Option<PathBuf>,
}

#[derive(Debug, PartialEq)]
pub enum CiState {
    Pass,
    Fail,
    Pend,
    None,
}

impl GitData {
    pub fn ci_state(&self) -> CiState {
        let rows = &self.pr.status_check_rollup;
        if rows.is_empty() {
            return CiState::None;
        }
        let mut any_pend = false;
        for r in rows {
            let v = if !r.conclusion.is_empty() {
                &r.conclusion
            } else {
                &r.status
            };
            match v.as_str() {
                "FAILURE" | "FAILED" | "TIMED_OUT" => return CiState::Fail,
                "IN_PROGRESS" | "PENDING" | "QUEUED" => any_pend = true,
                _ => {}
            }
        }
        if any_pend {
            CiState::Pend
        } else {
            CiState::Pass
        }
    }
}

pub fn view(session: &Session, st: &State) -> GitData {
    let mut g = GitData::default();
    if session.cwd.is_empty() {
        return g;
    }
    // Discover the repo from cwd upward (equivalent to `git rev-parse`'s
    // discovery). Not a repo → empty GitData, same as before.
    let repo = match git2::Repository::discover(&session.cwd) {
        Ok(r) => r,
        Err(_) => return g,
    };

    // Repo dirs. `path()` is the gitdir (`.git/worktrees/<n>` in a linked
    // worktree), `commondir()` the shared `.git`, `workdir()` the toplevel —
    // these feed worktree detection in `components::worktree_suffix`.
    g.git_dir = Some(repo.path().to_path_buf());
    g.common_dir = Some(repo.commondir().to_path_buf());
    g.toplevel = repo.workdir().map(PathBuf::from);

    // Current branch. `--show-current` is empty on a detached HEAD; mirror
    // that by only taking the shorthand when HEAD points at a branch.
    g.branch = if !session.wt_branch.is_empty() {
        session.wt_branch.clone()
    } else {
        repo.head()
            .ok()
            .filter(|h| h.is_branch())
            .and_then(|h| h.shorthand().ok().map(String::from))
            .unwrap_or_default()
    };

    g.origin_url = repo
        .find_remote("origin")
        .ok()
        .and_then(|r| r.url().ok().map(String::from))
        .unwrap_or_default();

    if let Some((ahead, behind)) = ahead_behind(&repo) {
        g.ahead = ahead;
        g.behind = behind;
    }

    g.dirty = dirty_count(&repo);

    // What this session wants watched. Published to the state file so the one
    // global worker can batch every live session's branch into a single query.
    g.watch = crate::state::Watch {
        owner: String::new(),
        repo: String::new(),
        branch: g.branch.clone(),
    };
    if let Some((owner, name)) = parse_github_remote(&g.origin_url) {
        g.watch.owner = owner;
        g.watch.repo = name;
    }

    // Prefer the shared watchlist result; fall back to this session's own blob
    // so an upgrade (or a not-yet-run worker) still renders the last chip.
    let from_global = if g.watch.is_empty() {
        None
    } else {
        crate::recent_prs::RecentPrs::load()
            .branches
            .get(&g.watch.key())
            .cloned()
    };
    let blob = from_global.unwrap_or_else(|| st.pr.json.clone());
    if !blob.is_empty() {
        if let Ok(parsed) = serde_json::from_str::<PrJson>(&blob) {
            g.pr = parsed;
        }
    }
    g
}

/// Commits ahead/behind the upstream tracking branch — the `branch.ab`
/// line of `git status -b`. `None` when detached or no upstream is set.
fn ahead_behind(repo: &git2::Repository) -> Option<(u32, u32)> {
    let head = repo.head().ok()?;
    if !head.is_branch() {
        return None;
    }
    let local_oid = head.target()?;
    let local = repo
        .find_branch(head.shorthand().ok()?, git2::BranchType::Local)
        .ok()?;
    let upstream_oid = local.upstream().ok()?.get().target()?;
    let (ahead, behind) = repo.graph_ahead_behind(local_oid, upstream_oid).ok()?;
    Some((ahead as u32, behind as u32))
}

/// Count of changed paths — the non-header lines of `git status --porcelain`.
/// Untracked files count (`recurse_untracked_dirs(false)` matches git's
/// default of collapsing a wholly-untracked dir to one entry); ignored
/// files do not.
fn dirty_count(repo: &git2::Repository) -> u32 {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
        .include_ignored(false);
    repo.statuses(Some(&mut opts))
        .map(|s| s.len() as u32)
        .unwrap_or(0)
}

/// `(owner, name)` from a github.com remote URL in any of the common forms
/// (scp-style, https, ssh). Non-github.com hosts return `None`.
pub fn parse_github_remote(url: &str) -> Option<(String, String)> {
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

pub fn extract_ticket(branch: &str) -> Option<String> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"([A-Za-z][A-Za-z]+)-([0-9]+)").expect("static ticket regex is valid")
    });
    re.captures(branch).map(|c| {
        let prefix = c.get(1).unwrap().as_str().to_uppercase();
        let num = c.get(2).unwrap().as_str();
        format!("{prefix}-{num}")
    })
}

pub fn linear_url(ticket: &str) -> String {
    format!(
        "https://linear.app/{}/issue/{ticket}",
        crate::config::config().linear_workspace()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_extracts_uppercase() {
        assert_eq!(
            extract_ticket("adam/cla-1057-x").as_deref(),
            Some("CLA-1057")
        );
    }
    #[test]
    fn ticket_none() {
        assert!(extract_ticket("just-a-branch").is_none());
    }
}
