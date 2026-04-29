// Git status (local, fast) + PR view-only-from-state. Async fetch lives in
// `refresh` and is dispatched by main before this runs.

use crate::input::Session;
use crate::state::State;
use serde::Deserialize;
use std::path::PathBuf;
use std::process::{Command, Stdio};

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

fn git_in(cwd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok().map(|s| s.trim().into())
}

pub fn view(session: &Session, st: &State) -> GitData {
    let mut g = GitData::default();
    if session.cwd.is_empty() {
        return g;
    }
    if git_in(&session.cwd, &["rev-parse", "--git-dir"]).is_none() {
        return g;
    }
    g.branch = if !session.wt_branch.is_empty() {
        session.wt_branch.clone()
    } else {
        git_in(&session.cwd, &["branch", "--show-current"]).unwrap_or_default()
    };
    g.origin_url =
        git_in(&session.cwd, &["config", "--get", "remote.origin.url"]).unwrap_or_default();
    if let Some(s) = git_in(&session.cwd, &["rev-parse", "--git-dir"]) {
        g.git_dir = Some(absolutize(&session.cwd, &s));
    }
    if let Some(s) = git_in(&session.cwd, &["rev-parse", "--git-common-dir"]) {
        g.common_dir = Some(absolutize(&session.cwd, &s));
    }
    if let Some(s) = git_in(&session.cwd, &["rev-parse", "--show-toplevel"]) {
        g.toplevel = Some(PathBuf::from(s));
    }

    if let Ok(out) = Command::new("git")
        .args(["status", "--porcelain=v2", "-b"])
        .current_dir(&session.cwd)
        .stderr(Stdio::null())
        .output()
    {
        if let Ok(text) = String::from_utf8(out.stdout) {
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("# branch.ab ") {
                    let mut it = rest.split_whitespace();
                    if let (Some(a), Some(b)) = (it.next(), it.next()) {
                        g.ahead = a.trim_start_matches('+').parse().unwrap_or(0);
                        g.behind = b.trim_start_matches('-').parse().unwrap_or(0);
                    }
                } else if line.starts_with('#') || line.is_empty() {
                    // skip
                } else {
                    g.dirty += 1;
                }
            }
        }
    }

    if !st.pr.json.is_empty() {
        if let Ok(parsed) = serde_json::from_str::<PrJson>(&st.pr.json) {
            g.pr = parsed;
        }
    }
    g
}

fn absolutize(cwd: &str, p: &str) -> PathBuf {
    let path = std::path::Path::new(p);
    if path.is_absolute() {
        path.into()
    } else {
        std::path::Path::new(cwd).join(path)
    }
}

pub fn extract_ticket(branch: &str) -> Option<String> {
    let re = regex::Regex::new(r"([A-Za-z][A-Za-z]+)-([0-9]+)").ok()?;
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
