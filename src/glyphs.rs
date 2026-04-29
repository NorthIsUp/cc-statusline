// Nerd Font glyphs (UTF-8 codepoints) + ANSI color escapes.
// Codepoints decoded from the bash original's raw byte escapes.

// PR / git
pub const PR_OPEN: &str = "\u{f407}"; // nf-oct-git_pull_request
pub const PR_DRAFT: &str = "\u{eba9}"; // nf-cod-git_pull_request_draft
pub const PR_CLOSED: &str = "\u{eba8}"; // nf-cod-git_pull_request_closed
pub const MERGED: &str = "\u{f419}"; // nf-oct-git_merge
pub const BRANCH: &str = "\u{f418}"; // nf-oct-git_branch
pub const COMMENT: &str = "\u{f41f}"; // nf-oct-comment
pub const APPROVED: &str = "\u{f42e}"; // nf-oct-check
pub const CHANGES: &str = "\u{f421}"; // nf-oct-x
pub const CI_PASS: &str = APPROVED;
pub const CI_FAIL: &str = CHANGES;
pub const CI_PEND: &str = "\u{f46a}"; // nf-oct-clock
pub const WORKTREE: &str = "\u{e57e}"; // nf-pl-branch

// Location/status
pub const APPLE: &str = "\u{f179}";
pub const CLOUD: &str = "\u{f0c2}";
pub const SSH_TERM: &str = "\u{f489}";
pub const DOCKER: &str = "\u{f308}";
pub const CTX: &str = "\u{f0a0}";
pub const CLOCK_5H: &str = "\u{f017}";
pub const CALENDAR_7D: &str = "\u{f073}";
// Quota bucket glyphs for design-partner and sonnet model quotas.
pub const DESIGN_Q: &str = "\u{f0aa6}"; // nf-md-palette
pub const SONNET_Q: &str = "\u{f035c}"; // nf-md-music_note
pub const EFFORT: &str = "\u{f0e7}";
pub const AGENT: &str = "\u{f0c0}";
pub const AHEAD: &str = "↑";
pub const BEHIND: &str = "↓";
pub const DIRTY: &str = "±";

// ANSI styles
pub const DIM: &str = "\x1b[2m";
pub const BOLD: &str = "\x1b[1m";
pub const RESET: &str = "\x1b[0m";
pub const FG_GREEN: &str = "\x1b[32m";
pub const FG_RED: &str = "\x1b[31m";
pub const FG_YELLOW: &str = "\x1b[33m";
pub const FG_CYAN: &str = "\x1b[36m";
#[allow(dead_code)]
pub const FG_MAGENTA: &str = "\x1b[35m";

// GitHub PR-state palette
pub const FG_GH_OPEN: &str = "\x1b[38;5;34m";
pub const FG_GH_MERGED: &str = "\x1b[38;5;99m";
pub const FG_GH_CLOSED: &str = "\x1b[38;5;167m";
pub const FG_GH_DRAFT: &str = "\x1b[38;5;102m";

/// OSC 8 hyperlink wrapper: (url, text) -> escape-wrapped text.
pub fn link(url: &str, text: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
}
