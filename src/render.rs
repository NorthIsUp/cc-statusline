// Layout assembly: left segment (repo + branch + PR + chips), right segment
// (burn / agents / quotas / ctx-bar / model), then right-align with overflow
// truncation. Mirrors the bash render section.

use crate::git::{CiState, GitData};
use crate::glyphs::*;
use crate::input::Session;
use crate::quota::{self, WIN_5H, WIN_7D};
use crate::transcript::{AgentCount, BurnInfo, OtherPrs};
use crate::vlen;

pub fn build(
    session: &Session,
    git: &GitData,
    other: &OtherPrs,
    burn: &BurnInfo,
    agents: &AgentCount,
    tick: u64,
) -> String {
    let cols = effective_cols(session.cols);
    let mut left = build_left(session, git, other);
    let right = build_right(session, burn, agents, tick);
    let chip_compact = build_chip_compact(other, &git.pr.url);
    let chip_expanded = build_chip_expanded(other, &git.pr.url);

    let rlen = vlen::vlen(&right);
    let gap: u32 = 2;
    let mut chip_line2 = String::new();

    if !chip_expanded.is_empty() {
        let min_cols: u32 = std::env::var("CC_STATUSLINE_PR_EXPAND_MIN_COLS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(160);
        if cols < min_cols {
            if !chip_compact.is_empty() {
                left = if left.is_empty() {
                    chip_compact
                } else {
                    format!("{left} {chip_compact}")
                };
            }
        } else {
            let candidate = if left.is_empty() {
                chip_expanded.clone()
            } else {
                format!("{left} {chip_expanded}")
            };
            if vlen::vlen(&candidate) + rlen + gap <= cols {
                left = candidate;
            } else {
                chip_line2 = chip_expanded.clone();
            }
        }
    }

    let llen = vlen::vlen(&left);
    let total = llen + rlen + gap;

    let mut out = String::new();
    if left.is_empty() {
        // Right-only single line — leading marker keeps the harness from
        // collapsing trailing whitespace.
        let pad_n = (cols.saturating_sub(rlen + 1)).max(1) as usize;
        out.push_str(&format!("{DIM}·{RESET}{}{right}", " ".repeat(pad_n)));
    } else if total <= cols {
        let pad_n = cols.saturating_sub(llen + rlen).max(gap) as usize;
        out.push_str(&format!("{left}{}{right}", " ".repeat(pad_n)));
    } else {
        // Overflow: truncate left, keep right.
        let budget = cols.saturating_sub(rlen + gap + 1) as u32; // -1 for ellipsis
        let stripped = vlen::strip(&left);
        let truncated = vlen::truncate_to_width(&stripped, budget);
        let left_out = format!("{truncated}{DIM}…{RESET}");
        let llen2 = vlen::vlen(&left_out);
        let pad_n = cols.saturating_sub(llen2 + rlen).max(gap) as usize;
        out.push_str(&format!("{left_out}{}{right}", " ".repeat(pad_n)));
    }

    if !chip_line2.is_empty() {
        out.push('\n');
        let trimmed = chip_line2.strip_prefix(' ').unwrap_or(&chip_line2);
        out.push_str(trimmed);
    }
    out
}

fn effective_cols(input: u32) -> u32 {
    let mut cols = if input > 0 { input } else { tty_cols() };
    // The bash original used a 6-cell safety margin to absorb Nerd Font glyph
    // width variance in its perl-based vlen. Our vlen counts cells exactly, so
    // default to 0; users can opt back in via the env var or config.
    let margin: u32 = std::env::var("CC_STATUSLINE_SAFETY_MARGIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| crate::config::config().safety_margin_or(0));
    cols = cols.saturating_sub(margin);
    cols.max(20)
}

fn tty_cols() -> u32 {
    use std::process::{Command, Stdio};

    // Claude Code runs us without a controlling tty, so plain `stty size`
    // fails. Open /dev/tty and hand it to stty as stdin — same trick as the
    // bash original (`stty size < /dev/tty`).
    let open_tty = || std::fs::OpenOptions::new().read(true).open("/dev/tty").ok();

    if let Some(tty) = open_tty() {
        if let Ok(out) = Command::new("stty")
            .arg("size")
            .stdin(Stdio::from(tty))
            .stderr(Stdio::null())
            .output()
        {
            if let Ok(s) = String::from_utf8(out.stdout) {
                if let Some(c) = s.split_whitespace().nth(1) {
                    if let Ok(n) = c.parse::<u32>() {
                        return n;
                    }
                }
            }
        }
    }
    if let Some(tty) = open_tty() {
        let term = std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".into());
        if let Ok(out) = Command::new("tput")
            .args(["-T", &term, "cols"])
            .stdin(Stdio::from(tty))
            .stderr(Stdio::null())
            .output()
        {
            if let Ok(s) = String::from_utf8(out.stdout) {
                if let Ok(n) = s.trim().parse::<u32>() {
                    return n;
                }
            }
        }
    }
    if let Ok(c) = std::env::var("COLUMNS") {
        if let Ok(n) = c.parse::<u32>() {
            return n;
        }
    }
    120
}

fn build_left(session: &Session, git: &GitData, _other: &OtherPrs) -> String {
    let mut left = String::new();
    if !git.branch.is_empty() {
        let (icon_glyph, state_color) = match git.pr.state.as_str() {
            "MERGED" => (MERGED, FG_GH_MERGED),
            "CLOSED" => (PR_CLOSED, FG_GH_CLOSED),
            "OPEN" if git.pr.is_draft => (PR_DRAFT, FG_GH_DRAFT),
            "OPEN" => (PR_OPEN, FG_GH_OPEN),
            _ => (BRANCH, DIM),
        };
        let icon = format!("{state_color}{icon_glyph}{RESET}");
        // Branch name keeps the default fg; the "#N" gets the state color so
        // the PR pill reads as one colored unit (icon + number).
        let pr_num = git
            .pr
            .number
            .map(|n| format!(" {state_color}#{n}{RESET}"))
            .unwrap_or_default();
        let label_plain = format!("{branch}{pr_num}", branch = git.branch);
        left = if !git.pr.url.is_empty() {
            format!("{icon} {}", link(&git.pr.url, &label_plain))
        } else {
            format!("{icon} {label_plain}")
        };

        // CI
        let ci_glyph = match git.ci_state() {
            CiState::Pass => Some(format!("{FG_GREEN}{CI_PASS}{RESET}")),
            CiState::Fail => Some(format!("{FG_RED}{CI_FAIL}{RESET}")),
            CiState::Pend => Some(format!("{FG_YELLOW}{CI_PEND}{RESET}")),
            CiState::None => None,
        };
        if let Some(mut g) = ci_glyph {
            if !git.pr.url.is_empty() {
                let checks = format!("{}/checks", git.pr.url);
                g = link(&checks, &g);
            }
            left.push(' ');
            left.push_str(&g);
        }

        // Review
        match git.pr.review_decision.as_str() {
            "APPROVED" => left.push_str(&format!(" {FG_GREEN}{APPROVED}{RESET}")),
            "CHANGES_REQUESTED" => left.push_str(&format!(" {FG_RED}{CHANGES}{RESET}")),
            _ => {}
        }

        // Comments
        let comment_n = git.pr.comments.len();
        if comment_n > 0 {
            left.push_str(&format!(" {COMMENT}{comment_n}"));
        }

        // Dirty / ahead / behind
        if git.dirty > 0 {
            left.push_str(&format!(" {FG_YELLOW}{DIRTY}{}{RESET}", git.dirty));
        }
        if git.ahead > 0 {
            left.push_str(&format!(" {DIM}{AHEAD}{}{RESET}", git.ahead));
        }
        if git.behind > 0 {
            left.push_str(&format!(" {FG_YELLOW}{BEHIND}{}{RESET}", git.behind));
        }

        // Linear ticket
        if let Some(t) = crate::git::extract_ticket(&git.branch) {
            let url = crate::git::linear_url(&t);
            let label = format!("[{t}]");
            left.push_str(&format!(" {FG_CYAN}{}{RESET}", link(&url, &label)));
        }
    }

    // Location prefix: org/repo (with optional ⎇ worktree-name) or ~/path.
    let mut loc_disp = String::new();
    if !git.origin_url.is_empty() {
        loc_disp = pretty_repo(&git.origin_url);
        if let (Some(gd), Some(cd), Some(top)) = (&git.git_dir, &git.common_dir, &git.toplevel) {
            if let (Ok(g_abs), Ok(c_abs)) = (gd.canonicalize(), cd.canonicalize()) {
                if g_abs != c_abs {
                    let wt_name = top
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if !wt_name.is_empty() && wt_name != git.branch {
                        loc_disp.push_str(&format!(
                            " {FG_CYAN}{WORKTREE}{RESET}{DIM} {wt_name}{RESET}"
                        ));
                    }
                }
            }
        }
    }
    if loc_disp.is_empty() {
        let cwd = if !session.cwd.is_empty() {
            session.cwd.clone()
        } else {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let home = std::env::var("HOME").unwrap_or_default();
        loc_disp = if !home.is_empty() && cwd == home {
            "~".into()
        } else if !home.is_empty() && cwd.starts_with(&format!("{home}/")) {
            format!("~{}", &cwd[home.len()..])
        } else {
            cwd
        };
    }
    if !loc_disp.is_empty() {
        let sep = if !left.is_empty() { " " } else { "" };
        left = format!("{DIM}{loc_disp}{RESET}{sep}{left}");
    }
    left
}

fn pretty_repo(origin_url: &str) -> String {
    // Strip trailing .git, then keep just "org/repo" — last two path segments.
    let s = origin_url.trim_end_matches(".git");
    let parts: Vec<&str> = s.split(['/', ':']).filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 {
        format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
    } else {
        s.into()
    }
}

fn pr_color_for(other: &OtherPrs, url: &str) -> &'static str {
    match other.states.get(url) {
        Some(s) => match s.state.as_str() {
            "MERGED" => FG_GH_MERGED,
            "CLOSED" => FG_GH_CLOSED,
            "OPEN" if s.is_draft => FG_GH_DRAFT,
            "OPEN" => FG_GH_OPEN,
            _ => DIM,
        },
        None => DIM,
    }
}

fn build_chip_compact(other: &OtherPrs, current_url: &str) -> String {
    let n = other.urls.len();
    if !chip_should_render(other, current_url) {
        return String::new();
    }
    format!("{DIM}{PR_OPEN}×{n}{RESET}")
}

fn build_chip_expanded(other: &OtherPrs, current_url: &str) -> String {
    if !chip_should_render(other, current_url) {
        return String::new();
    }
    let mut parts = String::new();
    for u in &other.urls {
        let n = u.rsplit('/').next().unwrap_or("");
        let c = pr_color_for(other, u);
        parts.push(' ');
        parts.push_str(&link(u, &format!("{c}#{n}{RESET}")));
    }
    format!("{DIM}{PR_OPEN}{RESET}{parts}")
}

fn chip_should_render(other: &OtherPrs, current_url: &str) -> bool {
    let n = other.urls.len();
    if n > 1 {
        return true;
    }
    if n == 1 && other.urls[0] != current_url {
        return true;
    }
    false
}

fn build_right(session: &Session, burn: &BurnInfo, agents: &AgentCount, tick: u64) -> String {
    let cols = effective_cols(session.cols);
    let bar = build_bar(session.ctx_pct);

    // Burn rate (≥130 cols)
    let mut right = String::new();
    let burn_str = if burn.tokens_per_hour > 0 {
        let n = burn.tokens_per_hour;
        let human = if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1_000_000.0)
        } else if n >= 1_000 {
            format!("{}k", (n as f64 / 1_000.0).round() as u64)
        } else {
            format!("{n}")
        };
        format!("{DIM}Σ{RESET} {human}{DIM}/hr{RESET}")
    } else {
        String::new()
    };
    if cols >= 130 && !burn_str.is_empty() {
        right.push_str(&burn_str);
        right.push_str(&format!(" {DIM}·{RESET} "));
    }

    // Agents (≥110 cols)
    let agent_str = if agents.total > 0 {
        if agents.active > 0 {
            format!(
                "{FG_YELLOW}{AGENT}{RESET} {}{DIM}/{}{RESET}",
                agents.active, agents.total
            )
        } else {
            format!("{DIM}{AGENT} {}{RESET}", agents.total)
        }
    } else {
        String::new()
    };
    if cols >= 110 && !agent_str.is_empty() {
        right.push_str(&agent_str);
        right.push_str(&format!(" {DIM}·{RESET} "));
    }

    // Quotas (≥90 cols)
    let q5h = quota::fmt_quota(session.r5h, session.r5h_reset, WIN_5H, CLOCK_5H);
    let q7d = quota::fmt_quota(session.r7d, session.r7d_reset, WIN_7D, CALENDAR_7D);
    let mut quota_str = String::new();
    if !q5h.is_empty() {
        quota_str.push_str(&q5h);
    }
    if !q7d.is_empty() {
        if !quota_str.is_empty() {
            quota_str.push_str(&format!(" {DIM}·{RESET} "));
        }
        quota_str.push_str(&q7d);
    }
    if cols >= 90 && !quota_str.is_empty() {
        right.push_str(&quota_str);
        right.push_str(&format!(" {DIM}·{RESET} "));
    }

    // Context bar + location icon + model + extras
    right.push_str(&format!(
        "{DIM}{CTX}{RESET} {bar} {pct}% {loc} {DIM}[{RESET}{BOLD}{model}{RESET}",
        pct = session.ctx_pct,
        loc = location_icon(),
        model = session.model,
    ));
    if session.fast_mode {
        right.push_str(&format!(" {FG_YELLOW}{EFFORT}{RESET}"));
    }
    if !session.output_style.is_empty() && session.output_style != "default" {
        right.push_str(&format!(
            " {DIM}·{RESET} {FG_CYAN}{}{RESET}",
            session.output_style
        ));
    }
    if !session.effort.is_empty() {
        let (ec, label) = match session.effort.as_str() {
            "high" => (FG_RED, "L".to_string()),
            "medium" => (FG_YELLOW, "M".to_string()),
            "low" => (FG_GREEN, "S".to_string()),
            "minimal" => (FG_GREEN, "XS".to_string()),
            other => (DIM, other.to_string()),
        };
        right.push_str(&format!(" {DIM}·{RESET} {ec}{EFFORT} {label}{RESET}"));
    }
    right.push_str(&format!("{DIM}]{RESET}"));

    let spin = spinner_text(tick);
    right.push_str(&format!(" {DIM}{spin}{RESET}"));
    right
}

fn build_bar(pct: u32) -> String {
    let bw: u32 = 10;
    let mut filled = pct * bw / 100;
    if filled > bw {
        filled = bw;
    }
    let empty = bw - filled;
    let mut bar = String::new();
    for _ in 0..filled {
        bar.push('▓');
    }
    for _ in 0..empty {
        bar.push('░');
    }
    let color = if pct >= 80 {
        FG_RED
    } else if pct >= 50 {
        FG_YELLOW
    } else {
        DIM
    };
    format!("{color}{bar}{RESET}")
}

fn location_icon() -> String {
    let env = std::env::vars().collect::<std::collections::HashMap<_, _>>();
    let has = |k: &str| env.get(k).filter(|v| !v.is_empty()).is_some();
    if has("SSH_CONNECTION") || has("SSH_CLIENT") || has("SSH_TTY") {
        return format!("{FG_YELLOW}{SSH_TERM}{RESET}");
    }
    if has("CODESPACES")
        || has("GITPOD_WORKSPACE_ID")
        || has("CODER_AGENT_TOKEN")
        || has("CLAUDE_CODE_REMOTE")
        || has("CLAUDE_REMOTE")
    {
        return format!("{FG_CYAN}{CLOUD}{RESET}");
    }
    if has("REMOTE_CONTAINERS")
        || has("DEVCONTAINER")
        || has("IN_CONTAINER")
        || std::path::Path::new("/.dockerenv").exists()
    {
        return format!("{FG_CYAN}{DOCKER}{RESET}");
    }
    if std::env::consts::OS == "macos" {
        return format!("{DIM}{APPLE}{RESET}");
    }
    format!("{DIM}{SSH_TERM}{RESET}")
}

/// Render a spinner string per the user's `spinner` config.
///
/// Supported styles:
///   - `compact` (default): single braille frame advanced by tick. Each render
///     advances exactly one frame so the rotation is visible regardless of
///     harness invocation cadence.
///   - `epoch-N`: show the last N digits of the current epoch (1's, 10's, …).
///     N=3 → "535". The least-significant digit changes every wall-clock
///     second, guaranteeing visible motion even if renders skip.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::PrJson;

    fn make_session(cols: u32) -> Session {
        Session {
            model: "Opus".into(),
            cwd: "/tmp".into(),
            session_id: "test".into(),
            ctx_pct: 50,
            cost: 0.0,
            wt_branch: "".into(),
            r5h: None,
            r7d: None,
            r5h_reset: None,
            r7d_reset: None,
            transcript: "".into(),
            effort: "medium".into(),
            fast_mode: false,
            output_style: "".into(),
            duration_ms: 0,
            cols,
        }
    }

    fn make_git() -> GitData {
        GitData {
            branch: "main".into(),
            dirty: 3,
            ahead: 1,
            behind: 0,
            pr: PrJson::default(),
            origin_url: "git@github.com:foo/bar.git".into(),
            git_dir: None,
            common_dir: None,
            toplevel: None,
        }
    }

    /// Renders MUST consume exactly `cols` visible cells AND the rightmost
    /// cell must be occupied by content (the spinner), not whitespace. The
    /// first guarantees no clipping; the second guarantees the right pane
    /// is anchored to the actual right edge — "no extra padding on the right".
    #[test]
    fn render_fills_cols_exactly() {
        // SAFETY: tests run single-threaded by default in this crate.
        unsafe {
            std::env::set_var("CC_STATUSLINE_NF_WIDTH", "1");
            std::env::set_var("CC_STATUSLINE_SAFETY_MARGIN", "0");
        }
        let git = make_git();
        let other = OtherPrs::default();
        let burn = BurnInfo::default();
        let agents = AgentCount::default();

        for &target_cols in &[80u32, 100, 120, 160, 200] {
            let session = make_session(target_cols);
            let line = build(&session, &git, &other, &burn, &agents, 0);
            let first_line = line.lines().next().unwrap_or(&line);
            let width = vlen::vlen(first_line);
            assert_eq!(
                width, target_cols,
                "render width mismatch at cols={target_cols}: got {width}, line={first_line:?}"
            );
            let stripped = vlen::strip(first_line);
            let last_char = stripped.chars().last().expect("non-empty render");
            assert!(
                !last_char.is_whitespace(),
                "rightmost cell at cols={target_cols} is whitespace ({last_char:?}); \
                 right pane is not anchored to the right edge: {stripped:?}"
            );
        }
    }
}

fn spinner_text(tick: u64) -> String {
    let style = crate::config::config().spinner();
    if let Some(rest) = style.strip_prefix("epoch-") {
        let n: u32 = rest.parse().unwrap_or(3).clamp(1, 10);
        let now = crate::cache::now_epoch().max(0) as u64;
        let modv = 10u64.pow(n);
        let val = now % modv;
        return format!("{val:0width$}", width = n as usize);
    }
    // compact (default)
    const FRAMES: [char; 9] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇'];
    FRAMES[(tick as usize) % FRAMES.len()].to_string()
}
