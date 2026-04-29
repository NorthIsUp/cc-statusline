// Concrete `Component` implementations and per-component config.
//
// Each section: a unit struct, optional Config, and a `Component` impl with
// 3-5 size variants. Registry-style `render_named` dispatch lives at the
// bottom — keeps the layout engine ignorant of component types.

use crate::component::{Component, RenderCtx, Rendered, Size};
use crate::git::CiState;
use crate::glyphs::*;
use crate::quota::{self, WIN_5H, WIN_7D};
use serde::Deserialize;

// ─── helpers ────────────────────────────────────────────────────────────

fn render_pretty_repo(origin_url: &str) -> String {
    let s = origin_url.trim_end_matches(".git");
    let parts: Vec<&str> = s.split(['/', ':']).filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 {
        format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
    } else {
        s.into()
    }
}

fn pretty_repo_full(origin_url: &str) -> String {
    let s = origin_url.trim_end_matches(".git");
    // Take everything after host: e.g. github.com/org/repo or just org/repo.
    if let Some(rest) = s.strip_prefix("https://") {
        rest.into()
    } else if let Some(rest) = s.strip_prefix("http://") {
        rest.into()
    } else if let Some(rest) = s.strip_prefix("git@") {
        rest.replacen(':', "/", 1)
    } else {
        s.into()
    }
}

fn cwd_disp(ctx: &RenderCtx) -> String {
    let cwd = if !ctx.session.cwd.is_empty() {
        ctx.session.cwd.clone()
    } else {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() && cwd == home {
        "~".into()
    } else if !home.is_empty() && cwd.starts_with(&format!("{home}/")) {
        format!("~{}", &cwd[home.len()..])
    } else {
        cwd
    }
}

fn worktree_suffix(ctx: &RenderCtx) -> String {
    let g = ctx.git;
    if let (Some(gd), Some(cd), Some(top)) = (&g.git_dir, &g.common_dir, &g.toplevel) {
        if let (Ok(g_abs), Ok(c_abs)) = (gd.canonicalize(), cd.canonicalize()) {
            if g_abs != c_abs {
                let wt_name = top
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if !wt_name.is_empty() && wt_name != g.branch {
                    return format!(" {FG_CYAN}{WORKTREE}{RESET}{DIM} {wt_name}{RESET}");
                }
            }
        }
    }
    String::new()
}

fn pr_state_color(state: &str, is_draft: bool) -> &'static str {
    match state {
        "MERGED" => FG_GH_MERGED,
        "CLOSED" => FG_GH_CLOSED,
        "OPEN" if is_draft => FG_GH_DRAFT,
        "OPEN" => FG_GH_OPEN,
        _ => DIM,
    }
}

fn pr_state_glyph(state: &str, is_draft: bool) -> &'static str {
    match state {
        "MERGED" => MERGED,
        "CLOSED" => PR_CLOSED,
        "OPEN" if is_draft => PR_DRAFT,
        "OPEN" => PR_OPEN,
        _ => BRANCH,
    }
}

// ─── repo (location prefix) ─────────────────────────────────────────────

pub struct Repo;
impl Component for Repo {
    type Config = ();
    fn name() -> &'static str {
        "repo"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::S, Size::M, Size::Xl]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        if size == Size::Xs {
            return Rendered::empty();
        }
        let mut loc = String::new();
        if !ctx.git.origin_url.is_empty() {
            loc = match size {
                Size::Xl => pretty_repo_full(&ctx.git.origin_url),
                _ => render_pretty_repo(&ctx.git.origin_url),
            };
            if size != Size::S {
                loc.push_str(&worktree_suffix(ctx));
            }
        }
        if loc.is_empty() {
            loc = cwd_disp(ctx);
            if size == Size::S {
                // Just the basename for tightest non-empty form.
                if let Some(b) = std::path::Path::new(&loc)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                {
                    loc = b;
                }
            }
        }
        if loc.is_empty() {
            return Rendered::empty();
        }
        Rendered::from_text(format!("{DIM}{loc}{RESET}"))
    }
}

// ─── pr_icon ────────────────────────────────────────────────────────────

pub struct PrIcon;
impl Component for PrIcon {
    type Config = ();
    fn name() -> &'static str {
        "pr_icon"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, _size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        if ctx.git.branch.is_empty() {
            return Rendered::empty();
        }
        let g = pr_state_glyph(&ctx.git.pr.state, ctx.git.pr.is_draft);
        let c = pr_state_color(&ctx.git.pr.state, ctx.git.pr.is_draft);
        Rendered::from_text(format!("{c}{g}{RESET}"))
    }
}

// ─── branch ─────────────────────────────────────────────────────────────

pub struct Branch;
impl Component for Branch {
    type Config = ();
    fn name() -> &'static str {
        "branch"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::S, Size::M, Size::Xl]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        if ctx.git.branch.is_empty() || size == Size::Xs {
            return Rendered::empty();
        }
        let label = match size {
            Size::S => {
                // Truncate long branch names.
                let max = 12;
                let chars: Vec<char> = ctx.git.branch.chars().collect();
                if chars.len() > max {
                    let s: String = chars.iter().take(max).collect();
                    format!("{s}…")
                } else {
                    ctx.git.branch.clone()
                }
            }
            _ => ctx.git.branch.clone(),
        };
        let text = if !ctx.git.pr.url.is_empty() {
            link(&ctx.git.pr.url, &label)
        } else {
            label
        };
        Rendered::from_text(text)
    }
}

// ─── pr_num ─────────────────────────────────────────────────────────────

pub struct PrNum;
impl Component for PrNum {
    type Config = ();
    fn name() -> &'static str {
        "pr_num"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        if size == Size::Xs {
            return Rendered::empty();
        }
        let n = match ctx.git.pr.number {
            Some(n) => n,
            None => return Rendered::empty(),
        };
        let c = pr_state_color(&ctx.git.pr.state, ctx.git.pr.is_draft);
        Rendered::from_text(format!("{c}#{n}{RESET}"))
    }
}

// ─── ci ─────────────────────────────────────────────────────────────────

pub struct Ci;
impl Component for Ci {
    type Config = ();
    fn name() -> &'static str {
        "ci"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, _size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        let raw = match ctx.git.ci_state() {
            CiState::Pass => format!("{FG_GREEN}{CI_PASS}{RESET}"),
            CiState::Fail => format!("{FG_RED}{CI_FAIL}{RESET}"),
            CiState::Pend => format!("{FG_YELLOW}{CI_PEND}{RESET}"),
            CiState::None => return Rendered::empty(),
        };
        let out = if !ctx.git.pr.url.is_empty() {
            link(&format!("{}/checks", ctx.git.pr.url), &raw)
        } else {
            raw
        };
        Rendered::from_text(out)
    }
}

// ─── review ─────────────────────────────────────────────────────────────

pub struct Review;
impl Component for Review {
    type Config = ();
    fn name() -> &'static str {
        "review"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, _size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        match ctx.git.pr.review_decision.as_str() {
            "APPROVED" => Rendered::from_text(format!("{FG_GREEN}{APPROVED}{RESET}")),
            "CHANGES_REQUESTED" => Rendered::from_text(format!("{FG_RED}{CHANGES}{RESET}")),
            _ => Rendered::empty(),
        }
    }
}

// ─── comments ───────────────────────────────────────────────────────────

pub struct Comments;
impl Component for Comments {
    type Config = ();
    fn name() -> &'static str {
        "comments"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, _size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        let n = ctx.git.pr.comments.len();
        if n == 0 {
            return Rendered::empty();
        }
        Rendered::from_text(format!("{COMMENT}{n}"))
    }
}

// ─── dirty / ahead / behind ─────────────────────────────────────────────

pub struct Dirty;
impl Component for Dirty {
    type Config = ();
    fn name() -> &'static str {
        "dirty"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, _size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        if ctx.git.dirty == 0 {
            return Rendered::empty();
        }
        Rendered::from_text(format!("{FG_YELLOW}{DIRTY}{}{RESET}", ctx.git.dirty))
    }
}

pub struct Ahead;
impl Component for Ahead {
    type Config = ();
    fn name() -> &'static str {
        "ahead"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, _size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        if ctx.git.ahead == 0 {
            return Rendered::empty();
        }
        Rendered::from_text(format!("{DIM}{AHEAD}{}{RESET}", ctx.git.ahead))
    }
}

pub struct Behind;
impl Component for Behind {
    type Config = ();
    fn name() -> &'static str {
        "behind"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, _size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        if ctx.git.behind == 0 {
            return Rendered::empty();
        }
        Rendered::from_text(format!("{FG_YELLOW}{BEHIND}{}{RESET}", ctx.git.behind))
    }
}

// ─── ticket ─────────────────────────────────────────────────────────────

pub struct Ticket;
impl Component for Ticket {
    type Config = ();
    fn name() -> &'static str {
        "ticket"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, _size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        let t = match crate::git::extract_ticket(&ctx.git.branch) {
            Some(t) => t,
            None => return Rendered::empty(),
        };
        let url = crate::git::linear_url(&t);
        let label = format!("[{t}]");
        Rendered::from_text(format!("{FG_CYAN}{}{RESET}", link(&url, &label)))
    }
}

// ─── chips (other PRs) ──────────────────────────────────────────────────

/// `[chips]` config block. Common ComponentConfig fields (priority/min/sizes/
/// required/default) live alongside chips-specific stack-mode knobs.
///
/// In stack mode (Graphite detected and ≥2 chip PRs covered by the stack),
/// chips are reordered by depth and joined with `stack_separator`, prefixed
/// by `stack_glyph` in dim cyan. Set `force_stack=true` to use the stack
/// layout even without `gt`; set `stack_glyph=""` to suppress the leading
/// glyph.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ChipsConfig {
    pub stack_separator: String,
    pub stack_glyph: String,
    pub force_stack: bool,
    pub stack_refresh_ttl: i64,
    #[serde(flatten)]
    pub common: crate::component::ComponentConfig,
}

impl Default for ChipsConfig {
    fn default() -> Self {
        Self {
            stack_separator: "─•─".into(),
            // Default to the Nerd Font branch glyph; users can clear via "".
            stack_glyph: BRANCH.into(),
            force_stack: false,
            stack_refresh_ttl: 60,
            common: crate::component::ComponentConfig::default(),
        }
    }
}

pub struct Chips;

fn pr_color_for(other: &crate::transcript::OtherPrs, url: &str) -> &'static str {
    match other.states.get(url) {
        Some(s) => pr_state_color(&s.state, s.is_draft),
        None => DIM,
    }
}

fn chip_should_render(other: &crate::transcript::OtherPrs, current_url: &str) -> bool {
    let n = other.urls.len();
    if n > 1 {
        return true;
    }
    if n == 1 && other.urls[0] != current_url {
        return true;
    }
    false
}

fn pr_num_from_url(url: &str) -> Option<u32> {
    let rest = url.strip_prefix("https://github.com/")?;
    let (_, num_part) = rest.split_once("/pull/")?;
    num_part.split(['/', '?', '#']).next()?.parse().ok()
}

/// Order URLs for stack-mode rendering. Stack-covered PRs come first in
/// trunk→leaf depth order; the rest are appended sorted by ascending PR
/// number. Returns `None` when stack mode does not apply (no gt, or fewer
/// than 2 chip URLs are covered by the stack and force_stack is false).
fn stack_ordered_urls(
    other: &crate::transcript::OtherPrs,
    force_stack: bool,
) -> Option<Vec<String>> {
    if !other.is_gt && !force_stack {
        return None;
    }
    // PR number → URL for the chip set. Multiple URLs with the same PR number
    // are unlikely (cross-repo filtering already happened upstream).
    let mut by_num: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    let mut leftover: Vec<String> = Vec::new();
    for u in &other.urls {
        match pr_num_from_url(u) {
            Some(n) => {
                by_num.insert(n, u.clone());
            }
            None => leftover.push(u.clone()),
        }
    }
    // Walk stack entries in depth order, picking up matching chips.
    let mut stacked_urls: Vec<String> = Vec::new();
    let mut sorted_entries: Vec<&crate::transcript::StackChipEntry> =
        other.stack_entries.iter().collect();
    sorted_entries.sort_by_key(|e| e.depth);
    let mut consumed: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for e in sorted_entries {
        if let Some(pr) = e.pr {
            if let Some(u) = by_num.get(&pr) {
                stacked_urls.push(u.clone());
                consumed.insert(pr);
            }
        }
    }
    // Require ≥2 covered chips for stack mode (unless forced) — a single
    // matched chip is no improvement over the legacy ordering.
    if !force_stack && stacked_urls.len() < 2 {
        return None;
    }
    // Append uncovered chips, sorted by PR number ascending (legacy fallback).
    let mut rest_pairs: Vec<(u32, String)> = by_num
        .into_iter()
        .filter(|(n, _)| !consumed.contains(n))
        .collect();
    rest_pairs.sort_by_key(|(n, _)| *n);
    let mut out = stacked_urls;
    for (_, u) in rest_pairs {
        out.push(u);
    }
    out.extend(leftover);
    Some(out)
}

fn render_chip(other: &crate::transcript::OtherPrs, url: &str) -> String {
    let n = url.rsplit('/').next().unwrap_or("");
    let c = pr_color_for(other, url);
    link(url, &format!("{c}#{n}{RESET}"))
}

impl Component for Chips {
    type Config = ChipsConfig;
    fn name() -> &'static str {
        "chips"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::S, Size::M, Size::Xl]
    }
    fn default_size() -> Size {
        Size::Xl
    }
    fn render(&self, size: Size, cfg: &ChipsConfig, ctx: &RenderCtx) -> Rendered {
        if size == Size::Xs {
            return Rendered::empty();
        }
        if !chip_should_render(ctx.other, &ctx.git.pr.url) {
            return Rendered::empty();
        }

        let stack = stack_ordered_urls(ctx.other, cfg.force_stack);

        match size {
            Size::S | Size::M => {
                // Compact: still ×N count even in stack mode (Xs already
                // dropped earlier; Sm+ shows the chain, but only at L+ here
                // because the fixed sizes() list is Xs/S/M/Xl).
                let n = ctx.other.urls.len();
                Rendered::from_text(format!("{DIM}{PR_OPEN}×{n}{RESET}"))
            }
            _ => match stack {
                Some(urls) if !urls.is_empty() => {
                    let sep = format!("{DIM}{}{RESET}", cfg.stack_separator);
                    let glyph = if cfg.stack_glyph.is_empty() {
                        String::new()
                    } else {
                        format!("{DIM}{FG_CYAN}{}{RESET} ", cfg.stack_glyph)
                    };
                    let chips: Vec<String> =
                        urls.iter().map(|u| render_chip(ctx.other, u)).collect();
                    Rendered::from_text(format!("{glyph}{}", chips.join(&sep)))
                }
                _ => {
                    // Legacy path: leading PR_OPEN icon, space-separated chips
                    // in their original (transcript-discovered) order.
                    let mut parts = String::new();
                    for u in &ctx.other.urls {
                        parts.push(' ');
                        parts.push_str(&render_chip(ctx.other, u));
                    }
                    Rendered::from_text(format!("{DIM}{PR_OPEN}{RESET}{parts}"))
                }
            },
        }
    }
}

// ─── burn ───────────────────────────────────────────────────────────────

fn human_burn(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}k", (n as f64 / 1_000.0).round() as u64)
    } else {
        format!("{n}")
    }
}

pub struct Burn;
impl Component for Burn {
    type Config = ();
    fn name() -> &'static str {
        "burn"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M, Size::Xl]
    }
    fn default_size() -> Size {
        Size::Xl
    }
    fn render(&self, size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        if ctx.burn.tokens_per_hour == 0 || size == Size::Xs {
            return Rendered::empty();
        }
        let h = human_burn(ctx.burn.tokens_per_hour);
        match size {
            Size::Xl => Rendered::from_text(format!("{DIM}Σ{RESET} {h}{DIM}/hr{RESET}")),
            _ => Rendered::from_text(format!("{DIM}Σ{RESET} {h}")),
        }
    }
}

// ─── agents ─────────────────────────────────────────────────────────────

pub struct Agents;
impl Component for Agents {
    type Config = ();
    fn name() -> &'static str {
        "agents"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, _size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        let a = ctx.agents;
        if a.total == 0 {
            return Rendered::empty();
        }
        let text = if a.active > 0 {
            format!(
                "{FG_YELLOW}{AGENT}{RESET} {}{DIM}/{}{RESET}",
                a.active, a.total
            )
        } else {
            format!("{DIM}{AGENT} {}{RESET}", a.total)
        };
        Rendered::from_text(text)
    }
}

// ─── quotas ─────────────────────────────────────────────────────────────

pub struct Quotas;
impl Component for Quotas {
    type Config = ();
    fn name() -> &'static str {
        "quotas"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, _size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        let s = ctx.session;
        let q5 = quota::fmt_quota(s.r5h, s.r5h_reset, WIN_5H, CLOCK_5H);
        let q7 = quota::fmt_quota(s.r7d, s.r7d_reset, WIN_7D, CALENDAR_7D);
        let mut out = String::new();
        if !q5.is_empty() {
            out.push_str(&q5);
        }
        if !q7.is_empty() {
            if !out.is_empty() {
                out.push_str(&format!(" {DIM}·{RESET} "));
            }
            out.push_str(&q7);
        }
        if out.is_empty() {
            return Rendered::empty();
        }
        Rendered::from_text(out)
    }
}

// ─── ctx_bar ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CtxBarConfig {
    pub width: u32,
    pub filled: String,
    pub empty: String,
    /// Common ComponentConfig fields lifted into the same TOML block. Lets
    /// users write:
    ///   [ctx_bar]
    ///   priority = 9
    ///   width = 10
    ///   filled = "▓"
    #[serde(flatten)]
    pub common: crate::component::ComponentConfig,
}
impl Default for CtxBarConfig {
    fn default() -> Self {
        Self {
            width: 10,
            filled: "▓".into(),
            empty: "░".into(),
            common: crate::component::ComponentConfig::default(),
        }
    }
}

pub struct CtxBar;

fn bar_color(pct: u32) -> &'static str {
    if pct >= 80 {
        FG_RED
    } else if pct >= 50 {
        FG_YELLOW
    } else {
        DIM
    }
}

fn build_bar(pct: u32, bw: u32, filled: &str, empty: &str) -> String {
    let mut f = pct * bw / 100;
    if f > bw {
        f = bw;
    }
    let e = bw - f;
    let mut s = String::new();
    for _ in 0..f {
        s.push_str(filled);
    }
    for _ in 0..e {
        s.push_str(empty);
    }
    let c = bar_color(pct);
    format!("{c}{s}{RESET}")
}

impl Component for CtxBar {
    type Config = CtxBarConfig;
    fn name() -> &'static str {
        "ctx_bar"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::S, Size::M, Size::L, Size::Xl]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, size: Size, cfg: &CtxBarConfig, ctx: &RenderCtx) -> Rendered {
        let pct = ctx.session.ctx_pct;
        match size {
            Size::Xs => {
                // One cell, no number. Use the filled glyph if any progress,
                // otherwise the empty glyph.
                let g = if pct > 0 { &cfg.filled } else { &cfg.empty };
                let c = bar_color(pct);
                Rendered::from_text(format!("{c}{g}{RESET}"))
            }
            Size::S => Rendered::from_text(format!("{}%", pct)),
            Size::M => {
                let bar = build_bar(pct, cfg.width, &cfg.filled, &cfg.empty);
                Rendered::from_text(format!("{bar} {pct}%"))
            }
            Size::L => {
                let bar = build_bar(pct, cfg.width, &cfg.filled, &cfg.empty);
                Rendered::from_text(format!("{DIM}{CTX}{RESET} {bar} {pct}%"))
            }
            Size::Xl => {
                let bar = build_bar(pct, cfg.width, &cfg.filled, &cfg.empty);
                Rendered::from_text(format!(
                    "{DIM}Σ{RESET} {bar} {pct}% {DIM}/ 200k tokens{RESET}"
                ))
            }
        }
    }
}

// ─── loc (host icon) ────────────────────────────────────────────────────

pub struct Loc;

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

impl Component for Loc {
    type Config = ();
    fn name() -> &'static str {
        "loc"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, _size: Size, _cfg: &(), _ctx: &RenderCtx) -> Rendered {
        Rendered::from_text(location_icon())
    }
}

// ─── model ──────────────────────────────────────────────────────────────

pub struct Model;
impl Component for Model {
    type Config = ();
    fn name() -> &'static str {
        "model"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::S, Size::M, Size::Xl]
    }
    fn default_size() -> Size {
        Size::Xl
    }
    fn render(&self, size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        let m = &ctx.session.model;
        if m.is_empty() {
            return Rendered::empty();
        }
        let label = match size {
            Size::Xs => m.chars().next().map(|c| c.to_string()).unwrap_or_default(),
            Size::S => m.split_whitespace().next().unwrap_or(m).to_string(),
            Size::M => m.split_whitespace().next().unwrap_or(m).to_string(),
            _ => m.clone(),
        };
        Rendered::from_text(format!("{DIM}[{RESET}{BOLD}{label}{RESET}{DIM}]{RESET}"))
    }
}

// ─── effort ─────────────────────────────────────────────────────────────

pub struct Effort;
impl Component for Effort {
    type Config = ();
    fn name() -> &'static str {
        "effort"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M, Size::Xl]
    }
    fn default_size() -> Size {
        Size::Xl
    }
    fn render(&self, size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        let e = &ctx.session.effort;
        if e.is_empty() {
            return Rendered::empty();
        }
        let (color, short, long) = match e.as_str() {
            "high" => (FG_RED, "L", "High"),
            "medium" => (FG_YELLOW, "M", "Medium"),
            "low" => (FG_GREEN, "S", "Low"),
            "minimal" => (FG_GREEN, "XS", "Minimal"),
            other => (DIM, other, other),
        };
        match size {
            Size::Xs => Rendered::from_text(format!("{color}{EFFORT}{RESET}")),
            Size::M => Rendered::from_text(format!("{color}{EFFORT} {short}{RESET}")),
            _ => Rendered::from_text(format!("{color}{EFFORT} {long}{RESET}")),
        }
    }
}

// ─── spinner ────────────────────────────────────────────────────────────

pub struct Spinner;

fn spinner_text(tick: u64) -> String {
    let style = crate::config::config().spinner();
    if let Some(rest) = style.strip_prefix("epoch-") {
        let n: u32 = rest.parse().unwrap_or(3).clamp(1, 10);
        let now = crate::cache::now_epoch().max(0) as u64;
        let modv = 10u64.pow(n);
        let val = now % modv;
        return format!("{val:0width$}", width = n as usize);
    }
    const FRAMES: [char; 9] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇'];
    FRAMES[(tick as usize) % FRAMES.len()].to_string()
}

impl Component for Spinner {
    type Config = ();
    fn name() -> &'static str {
        "spinner"
    }
    fn sizes() -> &'static [Size] {
        &[Size::M]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, _size: Size, _cfg: &(), ctx: &RenderCtx) -> Rendered {
        Rendered::from_text(format!("{DIM}{}{RESET}", spinner_text(ctx.tick)))
    }
}

// ─── registry-style dispatch ────────────────────────────────────────────

/// Render a component by name at the given size. Returns `None` if the name
/// is not a registered component.
pub fn render_named(name: &str, size: Size, ctx: &RenderCtx) -> Option<Rendered> {
    let cfg = crate::config::config();
    Some(match name {
        "repo" => Repo.render(size, &(), ctx),
        "branch" => Branch.render(size, &(), ctx),
        "pr_icon" => PrIcon.render(size, &(), ctx),
        "pr_num" => PrNum.render(size, &(), ctx),
        "ci" => Ci.render(size, &(), ctx),
        "review" => Review.render(size, &(), ctx),
        "comments" => Comments.render(size, &(), ctx),
        "dirty" => Dirty.render(size, &(), ctx),
        "ahead" => Ahead.render(size, &(), ctx),
        "behind" => Behind.render(size, &(), ctx),
        "ticket" => Ticket.render(size, &(), ctx),
        "chips" => Chips.render(size, &cfg.chips, ctx),
        "burn" => Burn.render(size, &(), ctx),
        "agents" => Agents.render(size, &(), ctx),
        "quotas" => Quotas.render(size, &(), ctx),
        "ctx_bar" => CtxBar.render(size, &cfg.ctx_bar, ctx),
        "loc" => Loc.render(size, &(), ctx),
        "model" => Model.render(size, &(), ctx),
        "effort" => Effort.render(size, &(), ctx),
        "spinner" => Spinner.render(size, &(), ctx),
        _ => return None,
    })
}

/// All known component names — used for default layouts and config validation.
pub const ALL_NAMES: &[&str] = &[
    "repo", "branch", "pr_icon", "pr_num", "ci", "review", "comments", "dirty", "ahead", "behind",
    "ticket", "chips", "burn", "agents", "quotas", "ctx_bar", "loc", "model", "effort", "spinner",
];

/// Sizes a named component supports. Returns `None` if unknown.
pub fn sizes_for(name: &str) -> Option<&'static [Size]> {
    Some(match name {
        "repo" => Repo::sizes(),
        "branch" => Branch::sizes(),
        "pr_icon" => PrIcon::sizes(),
        "pr_num" => PrNum::sizes(),
        "ci" => Ci::sizes(),
        "review" => Review::sizes(),
        "comments" => Comments::sizes(),
        "dirty" => Dirty::sizes(),
        "ahead" => Ahead::sizes(),
        "behind" => Behind::sizes(),
        "ticket" => Ticket::sizes(),
        "chips" => Chips::sizes(),
        "burn" => Burn::sizes(),
        "agents" => Agents::sizes(),
        "quotas" => Quotas::sizes(),
        "ctx_bar" => CtxBar::sizes(),
        "loc" => Loc::sizes(),
        "model" => Model::sizes(),
        "effort" => Effort::sizes(),
        "spinner" => Spinner::sizes(),
        _ => return None,
    })
}

pub fn default_size_for(name: &str) -> Option<Size> {
    Some(match name {
        "repo" => Repo::default_size(),
        "branch" => Branch::default_size(),
        "pr_icon" => PrIcon::default_size(),
        "pr_num" => PrNum::default_size(),
        "ci" => Ci::default_size(),
        "review" => Review::default_size(),
        "comments" => Comments::default_size(),
        "dirty" => Dirty::default_size(),
        "ahead" => Ahead::default_size(),
        "behind" => Behind::default_size(),
        "ticket" => Ticket::default_size(),
        "chips" => Chips::default_size(),
        "burn" => Burn::default_size(),
        "agents" => Agents::default_size(),
        "quotas" => Quotas::default_size(),
        "ctx_bar" => CtxBar::default_size(),
        "loc" => Loc::default_size(),
        "model" => Model::default_size(),
        "effort" => Effort::default_size(),
        "spinner" => Spinner::default_size(),
        _ => return None,
    })
}

/// Default priority assignments. Higher = shrunk later. Keep important visual
/// cues (model, ctx_bar, spinner, branch) high; nice-to-haves (chips, burn,
/// agents) low. Required-to-show items can also set `required = true`.
pub fn default_priority(name: &str) -> u32 {
    match name {
        "spinner" => 100,
        "ctx_bar" => 90,
        "model" => 80,
        "branch" => 75,
        "pr_icon" => 70,
        "pr_num" => 65,
        "loc" => 60,
        "effort" => 55,
        "ci" => 50,
        "review" => 45,
        "ticket" => 40,
        "dirty" => 35,
        "ahead" | "behind" => 30,
        "comments" => 25,
        "repo" => 20,
        "quotas" => 15,
        "agents" => 10,
        "burn" => 8,
        "chips" => 5,
        _ => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{GitData, PrJson};
    use crate::input::Session;
    use crate::transcript::{AgentCount, BurnInfo, OtherPrs};

    fn mkctx() -> (Session, GitData, OtherPrs, BurnInfo, AgentCount) {
        let session = Session {
            model: "Opus".into(),
            cwd: "/tmp".into(),
            session_id: "t".into(),
            ctx_pct: 71,
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
            cols: 200,
        };
        let git = GitData {
            branch: "main".into(),
            dirty: 0,
            ahead: 0,
            behind: 0,
            pr: PrJson::default(),
            origin_url: "git@github.com:foo/bar.git".into(),
            git_dir: None,
            common_dir: None,
            toplevel: None,
        };
        (
            session,
            git,
            OtherPrs::default(),
            BurnInfo::default(),
            AgentCount::default(),
        )
    }

    #[test]
    fn ctx_bar_renders_all_sizes() {
        let (session, git, other, burn, agents) = mkctx();
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let cfg = CtxBarConfig::default();
        let bar = CtxBar;
        for s in [Size::Xs, Size::S, Size::M, Size::L, Size::Xl] {
            let r = bar.render(s, &cfg, &ctx);
            assert!(!r.text.is_empty(), "size {s:?} empty");
            assert!(r.width > 0, "size {s:?} zero width");
        }
        // Width ordering: bigger size → wider (mostly).
        let xs = bar.render(Size::Xs, &cfg, &ctx).width;
        let xl = bar.render(Size::Xl, &cfg, &ctx).width;
        assert!(xl > xs);
    }

    fn url(n: u32) -> String {
        format!("https://github.com/foo/bar/pull/{n}")
    }

    fn entry(branch: &str, pr: Option<u32>, depth: u32) -> crate::transcript::StackChipEntry {
        crate::transcript::StackChipEntry {
            branch: branch.into(),
            pr,
            depth,
        }
    }

    fn stack_other(urls: Vec<u32>, entries: Vec<crate::transcript::StackChipEntry>) -> OtherPrs {
        OtherPrs {
            urls: urls.into_iter().map(url).collect(),
            states: Default::default(),
            is_gt: true,
            stack_entries: entries,
        }
    }

    #[test]
    fn chips_stack_orders_by_depth() {
        // Stack: trunk(main, depth 0, no PR) → feat/a(#101, depth 1)
        //                                    → feat/b(#102, depth 2)
        //                                    → feat/c(#103, depth 3)
        // Discovered URLs are scrambled (#103, #101, #102) — output must be
        // trunk-first depth ordering.
        let other = stack_other(
            vec![103, 101, 102],
            vec![
                entry("main", None, 0),
                entry("feat/a", Some(101), 1),
                entry("feat/b", Some(102), 2),
                entry("feat/c", Some(103), 3),
            ],
        );
        let cfg = ChipsConfig::default();
        let ordered = stack_ordered_urls(&other, cfg.force_stack).expect("stack mode active");
        assert_eq!(
            ordered,
            vec![url(101), url(102), url(103)],
            "depth-ordered chip URLs"
        );
    }

    #[test]
    fn chips_uncovered_prs_appended_after_stack() {
        // #999 is not in the stack — must come after all stacked chips,
        // sorted ascending among the uncovered set.
        let other = stack_other(
            vec![102, 999, 101],
            vec![entry("feat/a", Some(101), 1), entry("feat/b", Some(102), 2)],
        );
        let cfg = ChipsConfig::default();
        let ordered = stack_ordered_urls(&other, cfg.force_stack).expect("stack active");
        assert_eq!(ordered, vec![url(101), url(102), url(999)]);
    }

    #[test]
    fn chips_fallback_when_not_gt() {
        // is_gt=false, force_stack=false ⇒ no stack ordering.
        let mut other = stack_other(
            vec![102, 101],
            vec![entry("feat/a", Some(101), 1), entry("feat/b", Some(102), 2)],
        );
        other.is_gt = false;
        let cfg = ChipsConfig::default();
        assert!(stack_ordered_urls(&other, cfg.force_stack).is_none());
    }

    #[test]
    fn chips_requires_two_covered_for_stack_mode() {
        // Only #101 maps into the stack; #999 doesn't. With one covered chip
        // the stack ordering is no improvement over legacy → fall back.
        let other = stack_other(vec![999, 101], vec![entry("feat/a", Some(101), 1)]);
        let cfg = ChipsConfig::default();
        assert!(stack_ordered_urls(&other, cfg.force_stack).is_none());
    }

    #[test]
    fn chips_force_stack_engages_with_one_covered() {
        // force_stack overrides the ≥2 covered rule.
        let other = stack_other(vec![999, 101], vec![entry("feat/a", Some(101), 1)]);
        let ordered = stack_ordered_urls(&other, true).expect("forced");
        assert_eq!(ordered, vec![url(101), url(999)]);
    }

    #[test]
    fn chips_xs_collapses_to_count() {
        // Xs path returns empty (component is dropped); the ×N collapse lives
        // at S/M.
        let (session, git, other_def, burn, agents) = mkctx();
        let other = stack_other(
            vec![101, 102, 103],
            vec![
                entry("feat/a", Some(101), 1),
                entry("feat/b", Some(102), 2),
                entry("feat/c", Some(103), 3),
            ],
        );
        let _ = other_def; // unused; replaced
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let cfg = ChipsConfig::default();
        let xs = Chips.render(Size::Xs, &cfg, &ctx);
        assert!(xs.text.is_empty(), "Xs drops chips entirely");

        let m = Chips.render(Size::M, &cfg, &ctx);
        assert!(m.text.contains("×3"), "M collapses to ×N count: {}", m.text);

        let xl = Chips.render(Size::Xl, &cfg, &ctx);
        assert!(xl.text.contains("#101"), "Xl shows full chain: {}", xl.text);
        assert!(xl.text.contains("#103"));
        // Stack separator present and trunk-first ordering: #101 appears
        // before #103 in the rendered string.
        let p101 = xl.text.find("#101").unwrap();
        let p103 = xl.text.find("#103").unwrap();
        assert!(p101 < p103, "trunk-first order");
        assert!(
            xl.text.contains(&cfg.stack_separator),
            "uses configured stack separator"
        );
    }

    #[test]
    fn chips_legacy_render_when_no_stack() {
        // Without is_gt and without force_stack, the legacy leading PR_OPEN
        // glyph is rendered and no stack separator appears.
        let (session, git, _other_def, burn, agents) = mkctx();
        let other = OtherPrs {
            urls: vec![url(101), url(102)],
            ..Default::default()
        };
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let cfg = ChipsConfig::default();
        let r = Chips.render(Size::Xl, &cfg, &ctx);
        assert!(r.text.contains("#101"));
        assert!(r.text.contains("#102"));
        assert!(
            !r.text.contains(&cfg.stack_separator),
            "no stack separator in legacy mode"
        );
    }
}
