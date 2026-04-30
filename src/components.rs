// Concrete `Component` implementations and per-component config.
//
// Each section: a unit struct, optional Config, and a `Component` impl with
// 3-5 size variants. Registry-style `render_named` dispatch lives at the
// bottom — keeps the layout engine ignorant of component types.

use crate::component::{Component, RenderCtx, Rendered, Size};
use crate::git::CiState;
use crate::glyphs::*;
use crate::pct::{self, PctConfig, PctMode};
use crate::quota::{self, WIN_5H, WIN_7D};
use schemars::JsonSchema;
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
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ChipsConfig {
    pub stack_separator: String,
    pub stack_glyph: String,
    pub force_stack: bool,
    pub stack_refresh_ttl: i64,
    /// Drop merged PRs older than this many hours from the chips chain.
    /// `0` disables the filter (every merged PR is rendered regardless of
    /// age). The current branch's own PR is never filtered. Stack mode
    /// (gt) bypasses the filter entirely — stacked PRs are by definition
    /// relevant.
    pub collapse_merged_after_hours: u32,
    /// When the merge-age filter drops ≥1 merged PRs from the chain, prepend
    /// a `<merged_glyph>×N` summary chip indicating how many were collapsed.
    /// Set to `false` to suppress. Stack mode bypasses the filter, so the
    /// summary chip never renders there.
    pub merged_summary: bool,
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
            collapse_merged_after_hours: 36,
            merged_summary: true,
            common: crate::component::ComponentConfig::default(),
        }
    }
}

pub struct Chips;

/// Result of `filter_collapsed_merged`: the kept URLs in original order plus
/// the count of merged PRs that were dropped (collapsed out).
pub(crate) struct CollapsedMergedFilter {
    pub kept: Vec<String>,
    pub dropped: usize,
}

/// Filter out merged PRs older than `cutoff_hours` from `urls`. Never drops
/// `current_url` (the active branch's PR). When `cutoff_hours == 0` or
/// `bypass` is true (stack mode), returns `urls` unchanged with `dropped = 0`.
fn filter_collapsed_merged(
    urls: &[String],
    states: &std::collections::HashMap<String, crate::transcript::PrStateLite>,
    cutoff_hours: u32,
    current_url: &str,
    bypass: bool,
    now: i64,
) -> CollapsedMergedFilter {
    if bypass || cutoff_hours == 0 {
        return CollapsedMergedFilter {
            kept: urls.to_vec(),
            dropped: 0,
        };
    }
    let cutoff = now - (cutoff_hours as i64) * 3600;
    let mut kept = Vec::with_capacity(urls.len());
    let mut dropped = 0usize;
    for u in urls {
        let keep = if u.as_str() == current_url {
            true
        } else {
            match states.get(u.as_str()) {
                Some(s) if s.state == "MERGED" => match s.merged_at {
                    Some(ts) => ts >= cutoff,
                    None => true, // unknown timestamp → keep
                },
                _ => true,
            }
        };
        if keep {
            kept.push(u.clone());
        } else {
            dropped += 1;
        }
    }
    CollapsedMergedFilter { kept, dropped }
}

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
        // Stack mode (gt or force_stack) bypasses the merge-age filter;
        // stacked PRs are always relevant.
        let bypass_filter = stack.is_some();
        let now = crate::cache::now_epoch();
        let CollapsedMergedFilter {
            kept: filtered_urls,
            dropped: dropped_count,
        } = filter_collapsed_merged(
            &ctx.other.urls,
            &ctx.other.states,
            cfg.collapse_merged_after_hours,
            &ctx.git.pr.url,
            bypass_filter,
            now,
        );

        match size {
            Size::S | Size::M => {
                // Compact: still ×N count even in stack mode (Xs already
                // dropped earlier; Sm+ shows the chain, but only at L+ here
                // because the fixed sizes() list is Xs/S/M/Xl).
                let n = filtered_urls.len();
                Rendered::from_text(format!("{DIM}{PR_OPEN}×{n}{RESET}"))
            }
            _ => match stack {
                Some(urls) if !urls.is_empty() => {
                    // Stack mode bypasses the merge-age filter, so the
                    // summary chip never renders here (dropped_count is 0).
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
                    // in their original (transcript-discovered) order. When
                    // ≥1 merged PRs were collapsed by the filter, prepend a
                    // `<merged_glyph>×N` summary chip in the merged color.
                    // The summary chip is intentionally not OSC-8 hyperlinked
                    // (no single URL applies).
                    let mut parts = String::new();
                    if cfg.merged_summary && dropped_count > 0 {
                        parts.push(' ');
                        parts.push_str(&format!("{FG_GH_MERGED}{MERGED}×{dropped_count}{RESET}"));
                    }
                    for u in &filtered_urls {
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

/// Per-component config for `burn`. Inherits the shared `[pct]` knobs (`mode`,
/// `width`, `filled`, `empty`) plus the burn-specific `max_tokens_per_hour`
/// ceiling used to derive a percent. Default mode is `percent`, which
/// preserves the legacy `Σ <human>/hr` text rendering. The visual modes
/// (`dots`, `shaded`, `hbar`, `vbar`) replace the text with a bar; `float`
/// also keeps the legacy text format.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BurnConfig {
    /// Tokens-per-hour value treated as 100%. Picked to map typical burn
    /// rates (1M–3M/hr) to the 20–60% range so the visual modes are
    /// meaningful without saturating to red.
    pub max_tokens_per_hour: u64,
    #[serde(flatten)]
    pub pct: PctConfig,
    #[serde(flatten)]
    pub common: crate::component::ComponentConfig,
}

impl Default for BurnConfig {
    fn default() -> Self {
        Self {
            max_tokens_per_hour: 5_000_000,
            pct: PctConfig::default(),
            common: crate::component::ComponentConfig::default(),
        }
    }
}

pub struct Burn;
impl Component for Burn {
    type Config = BurnConfig;
    fn name() -> &'static str {
        "burn"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M, Size::Xl]
    }
    fn default_size() -> Size {
        Size::Xl
    }
    fn render(&self, size: Size, cfg: &BurnConfig, ctx: &RenderCtx) -> Rendered {
        if ctx.burn.tokens_per_hour == 0 || size == Size::Xs {
            return Rendered::empty();
        }
        // For text-y modes, keep the legacy `Σ <human>/hr` look.
        match cfg.pct.mode {
            PctMode::Percent | PctMode::Float => {
                let h = human_burn(ctx.burn.tokens_per_hour);
                let text = match size {
                    Size::Xl => format!("{DIM}Σ{RESET} {h}{DIM}/hr{RESET}"),
                    _ => format!("{DIM}Σ{RESET} {h}"),
                };
                Rendered::from_text(text)
            }
            _ => {
                // Visual modes: render the percent bar derived from the
                // configurable ceiling.
                let max = cfg.max_tokens_per_hour.max(1);
                let pct = ((ctx.burn.tokens_per_hour as u128 * 100) / max as u128).min(100) as u32;
                let body = pct::render(pct, &cfg.pct);
                let text = match size {
                    Size::Xl => format!("{DIM}Σ{RESET} {body}{DIM}/hr{RESET}"),
                    _ => format!("{DIM}Σ{RESET} {body}"),
                };
                Rendered::from_text(text)
            }
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

/// Per-component config for `quotas`. Top-level keys (`mode`, `width`,
/// `filled`, `empty`) act as defaults applied to every bucket; per-bucket
/// sub-sections (`[quotas.hourly]`, `[quotas.weekly]`, `[quotas.design]`,
/// `[quotas.sonnet]`) override those defaults for that bucket only.
///
/// Default mode is `percent` to preserve the legacy `<glyph> 47%` text.
/// The reset-time suffix is appended by `quota::fmt_quota` regardless of
/// mode, so users can switch the percent visual to `dots`/`hbar` without
/// losing the reset clock.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct QuotasConfig {
    #[serde(flatten)]
    pub pct: PctConfig,
    #[serde(flatten)]
    pub common: crate::component::ComponentConfig,
    /// Override for the 5-hour bucket (`rate_limits.five_hour`).
    pub hourly: Option<BucketConfig>,
    /// Override for the 7-day bucket (`rate_limits.seven_day`).
    pub weekly: Option<BucketConfig>,
    /// Override for the design-partner bucket.
    pub design: Option<BucketConfig>,
    /// Override for the sonnet model bucket.
    pub sonnet: Option<BucketConfig>,
}

/// Per-bucket override block. Carries both the percent-display fields
/// (`mode`/`width`/`filled`/`empty`) and the layout `ComponentConfig` knobs
/// (`priority`/`min`/`required`/...) — the latter let users elevate a single
/// bucket's autoresize priority via e.g. `[quotas.hourly] priority = 30`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BucketConfig {
    #[serde(flatten)]
    pub pct: PctConfig,
    #[serde(flatten)]
    pub common: crate::component::ComponentConfig,
}

/// Which quota bucket to render. Used for the dotted-layout-entry routing
/// (`quotas.hourly`, `quotas.weekly`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BucketKind {
    Hourly,
    Weekly,
    Design,
    Sonnet,
}

impl BucketKind {
    pub fn from_suffix(s: &str) -> Option<Self> {
        Some(match s {
            "hourly" => BucketKind::Hourly,
            "weekly" => BucketKind::Weekly,
            "design" => BucketKind::Design,
            "sonnet" => BucketKind::Sonnet,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            BucketKind::Hourly => "hourly",
            BucketKind::Weekly => "weekly",
            BucketKind::Design => "design",
            BucketKind::Sonnet => "sonnet",
        }
    }
}

impl QuotasConfig {
    /// Resolve a bucket override against the parent defaults. When the bucket
    /// has its own `PctConfig`, it wins wholesale; otherwise the parent
    /// `[quotas]` defaults are used. (We don't field-merge: `PctConfig` fields
    /// are tightly coupled — `mode = "hbar"` only makes sense alongside its
    /// own `width`/`filled`/`empty`.)
    fn effective(&self, bucket: &Option<BucketConfig>) -> PctConfig {
        bucket
            .as_ref()
            .map(|b| b.pct.clone())
            .unwrap_or_else(|| self.pct.clone())
    }

    fn bucket_for(&self, kind: BucketKind) -> &Option<BucketConfig> {
        match kind {
            BucketKind::Hourly => &self.hourly,
            BucketKind::Weekly => &self.weekly,
            BucketKind::Design => &self.design,
            BucketKind::Sonnet => &self.sonnet,
        }
    }
}

/// Render a single quota bucket, returning `None` if the bucket has no data.
pub fn render_one_bucket(cfg: &QuotasConfig, kind: BucketKind, ctx: &RenderCtx) -> Option<String> {
    let s = ctx.session;
    // Design/sonnet windows aren't documented upstream — use WIN_7D as a
    // sane default so pace coloring degrades to the static threshold when
    // `resets_at` is missing (which it currently always is).
    let (label, pct, reset, window) = match kind {
        BucketKind::Hourly => (CLOCK_5H, s.r5h, s.r5h_reset, WIN_5H),
        BucketKind::Weekly => (CALENDAR_7D, s.r7d, s.r7d_reset, WIN_7D),
        // design/sonnet: config plumbing exists, but the upstream session JSON
        // paths aren't documented yet — pct stays None so these always render
        // empty until follow-up work captures a real session payload and
        // wires `Session::design`/`sonnet`.
        BucketKind::Design => (DESIGN_Q, None, None, WIN_7D),
        BucketKind::Sonnet => (SONNET_Q, None, None, WIN_7D),
    };
    let pcfg = cfg.effective(cfg.bucket_for(kind));
    let q = quota::fmt_quota(pct, reset, window, label, &pcfg);
    if q.is_empty() { None } else { Some(q) }
}

pub struct Quotas;
impl Component for Quotas {
    type Config = QuotasConfig;
    fn name() -> &'static str {
        "quotas"
    }
    fn sizes() -> &'static [Size] {
        &[Size::Xs, Size::M]
    }
    fn default_size() -> Size {
        Size::M
    }
    fn render(&self, _size: Size, cfg: &QuotasConfig, ctx: &RenderCtx) -> Rendered {
        // Render order: hourly, weekly, design, sonnet. Skip empty buckets.
        let mut out = String::new();
        for kind in [
            BucketKind::Hourly,
            BucketKind::Weekly,
            BucketKind::Design,
            BucketKind::Sonnet,
        ] {
            let Some(q) = render_one_bucket(cfg, kind, ctx) else {
                continue;
            };
            if !out.is_empty() {
                out.push_str(&format!(" {DIM}·{RESET} "));
            }
            out.push_str(&q);
        }
        if out.is_empty() {
            return Rendered::empty();
        }
        Rendered::from_text(out)
    }
}

// ─── ctx_bar ────────────────────────────────────────────────────────────

/// Per-component config for `ctx_bar`. Inherits the shared `[pct]` knobs
/// (`mode`, `width`, `filled`, `empty`) via `serde(flatten)` — meaning the
/// existing `[ctx_bar]` blocks with `width = 10`, `filled = "▓"`, `empty =
/// "░"` continue to parse unchanged. They're treated as overrides for the
/// `hbar` mode glyphs.
///
/// Default mode is `percent` (a plain `47%`); set `mode = "hbar"` to bring
/// back the bar look (now with sub-cell precision via the eighths glyphs).
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CtxBarConfig {
    #[serde(flatten)]
    pub pct: PctConfig,
    #[serde(flatten)]
    pub common: crate::component::ComponentConfig,
}

pub struct CtxBar;

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
        // Per-size visual:
        //   Xs   one-cell `dots` glyph, regardless of configured mode (a
        //        single percent doesn't read at one cell anyway).
        //   S    just the percent text via the configured mode.
        //   M    mode-rendered visual, plus a trailing `47%` text for legibility.
        //   L    M + leading CTX glyph.
        //   Xl   L + trailing `/ 200k tokens` annotation.
        match size {
            Size::Xs => {
                let dots_cfg = PctConfig {
                    mode: PctMode::Dots,
                    ..cfg.pct.clone()
                };
                Rendered::from_text(pct::render(pct, &dots_cfg))
            }
            Size::S => Rendered::from_text(pct::render(pct, &cfg.pct)),
            Size::M => {
                let body = pct::render(pct, &cfg.pct);
                // For the textual modes don't double-print the percent.
                if matches!(cfg.pct.mode, PctMode::Percent | PctMode::Float) {
                    Rendered::from_text(body)
                } else {
                    Rendered::from_text(format!("{body} {pct}%"))
                }
            }
            Size::L => {
                let body = pct::render(pct, &cfg.pct);
                if matches!(cfg.pct.mode, PctMode::Percent | PctMode::Float) {
                    Rendered::from_text(format!("{DIM}{CTX}{RESET} {body}"))
                } else {
                    Rendered::from_text(format!("{DIM}{CTX}{RESET} {body} {pct}%"))
                }
            }
            Size::Xl => {
                let body = pct::render(pct, &cfg.pct);
                if matches!(cfg.pct.mode, PctMode::Percent | PctMode::Float) {
                    Rendered::from_text(format!("{DIM}Σ{RESET} {body} {DIM}/ 200k tokens{RESET}"))
                } else {
                    Rendered::from_text(format!(
                        "{DIM}Σ{RESET} {body} {pct}% {DIM}/ 200k tokens{RESET}"
                    ))
                }
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

/// Parse `quotas.<bucket>` layout entry names into a `BucketKind`. Returns
/// `None` for the bare `"quotas"` name and for unknown sub-buckets like
/// `"quotas.foo"`.
pub fn quotas_bucket_kind(name: &str) -> Option<BucketKind> {
    name.strip_prefix("quotas.")
        .and_then(BucketKind::from_suffix)
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
        "burn" => Burn.render(size, &cfg.burn, ctx),
        "agents" => Agents.render(size, &(), ctx),
        "quotas" => Quotas.render(size, &cfg.quotas, ctx),
        n if quotas_bucket_kind(n).is_some() => {
            let kind = quotas_bucket_kind(n).unwrap();
            match render_one_bucket(&cfg.quotas, kind, ctx) {
                Some(text) => Rendered::from_text(text),
                None => Rendered::empty(),
            }
        }
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
        n if quotas_bucket_kind(n).is_some() => Quotas::sizes(),
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
        n if quotas_bucket_kind(n).is_some() => Quotas::default_size(),
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
        "quotas.hourly" => 25,
        "quotas.weekly" => 20,
        "quotas.design" => 18,
        "quotas.sonnet" => 17,
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

    #[test]
    fn ctx_bar_hbar_mode_renders_bar_glyphs() {
        let (mut session, git, other, burn, agents) = mkctx();
        session.ctx_pct = 50;
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let cfg = CtxBarConfig {
            pct: PctConfig {
                mode: PctMode::Hbar,
                width: 10,
                filled: "█".into(),
                empty: " ".into(),
            },
            ..CtxBarConfig::default()
        };
        let r = CtxBar.render(Size::M, &cfg, &ctx);
        assert!(
            r.text.contains("█"),
            "hbar should render block glyph: {}",
            r.text
        );
        assert!(
            r.text.contains("50%"),
            "M size should still show percent suffix"
        );
    }

    #[test]
    fn ctx_bar_dots_mode_one_cell() {
        let (mut session, git, other, burn, agents) = mkctx();
        session.ctx_pct = 50;
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let cfg = CtxBarConfig {
            pct: PctConfig {
                mode: PctMode::Dots,
                ..PctConfig::default()
            },
            ..CtxBarConfig::default()
        };
        let r = CtxBar.render(Size::S, &cfg, &ctx);
        assert!(r.text.contains("⡇"), "dots@50%% → ⡇: {}", r.text);
    }

    #[test]
    fn quotas_dots_mode_renders_braille_glyph() {
        let (mut session, git, other, burn, agents) = mkctx();
        session.r5h = Some(50);
        session.r5h_reset = None;
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let cfg = QuotasConfig {
            pct: PctConfig {
                mode: PctMode::Dots,
                ..PctConfig::default()
            },
            ..QuotasConfig::default()
        };
        let r = Quotas.render(Size::M, &cfg, &ctx);
        assert!(
            r.text.contains("⡇"),
            "quotas dots@50%% should contain ⡇: {}",
            r.text
        );
    }

    #[test]
    fn quotas_renders_only_hourly_weekly_when_design_sonnet_none() {
        // Back-compat: with only the legacy r5h/r7d populated, the rendered
        // string contains exactly the existing two glyphs and no design/sonnet
        // glyphs, regardless of the new sub-section plumbing.
        let (mut session, git, other, burn, agents) = mkctx();
        session.r5h = Some(40);
        session.r7d = Some(60);
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let cfg = QuotasConfig::default();
        let r = Quotas.render(Size::M, &cfg, &ctx);
        assert!(
            r.text.contains(CLOCK_5H),
            "hourly glyph present: {}",
            r.text
        );
        assert!(
            r.text.contains(CALENDAR_7D),
            "weekly glyph present: {}",
            r.text
        );
        assert!(
            !r.text.contains(DESIGN_Q),
            "design glyph absent when None: {}",
            r.text
        );
        assert!(
            !r.text.contains(SONNET_Q),
            "sonnet glyph absent when None: {}",
            r.text
        );
    }

    #[test]
    fn quotas_design_sonnet_inert_until_session_input_lands() {
        // design/sonnet config plumbing exists, but Session has no input
        // fields for them yet — so even with overrides set, those buckets
        // never render. Hourly/weekly still work normally.
        let (mut session, git, other, burn, agents) = mkctx();
        session.r5h = Some(10);
        session.r7d = Some(20);
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let cfg = QuotasConfig {
            design: Some(BucketConfig::default()),
            sonnet: Some(BucketConfig::default()),
            ..QuotasConfig::default()
        };
        let r = Quotas.render(Size::M, &cfg, &ctx);
        assert!(r.text.contains(CLOCK_5H));
        assert!(r.text.contains(CALENDAR_7D));
        assert!(!r.text.contains(DESIGN_Q));
        assert!(!r.text.contains(SONNET_Q));
    }

    #[test]
    fn quotas_per_bucket_mode_overrides_parent() {
        // Parent mode = percent; hourly overrides to vbar. Verify hourly
        // bucket renders the vbar glyph (50% → ▄) and weekly still renders
        // the percent text "50%".
        let (mut session, git, other, burn, agents) = mkctx();
        session.r5h = Some(50);
        session.r7d = Some(50);
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let cfg = QuotasConfig {
            pct: PctConfig {
                mode: PctMode::Percent,
                ..PctConfig::default()
            },
            hourly: Some(BucketConfig {
                pct: PctConfig {
                    mode: PctMode::Vbar,
                    ..PctConfig::default()
                },
                ..BucketConfig::default()
            }),
            ..QuotasConfig::default()
        };
        let r = Quotas.render(Size::M, &cfg, &ctx);
        // Split on the bucket separator's `·` to isolate each section.
        let parts: Vec<&str> = r.text.split('·').collect();
        assert_eq!(parts.len(), 2, "two buckets joined: {}", r.text);
        assert!(
            parts[0].contains('▄'),
            "hourly should render vbar (50%% → ▄): {}",
            parts[0]
        );
        assert!(
            !parts[0].contains("50%"),
            "hourly should NOT contain percent text: {}",
            parts[0]
        );
        assert!(
            parts[1].contains("50%"),
            "weekly should still render percent text: {}",
            parts[1]
        );
    }

    #[test]
    fn quotas_missing_resets_at_falls_back_to_threshold_color() {
        // With reset = None and pct >= 80, pct_color falls back to FG_RED
        // via the static threshold band.
        let (mut session, git, other, burn, agents) = mkctx();
        session.r5h = Some(85);
        session.r5h_reset = None;
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let cfg = QuotasConfig::default();
        let r = Quotas.render(Size::M, &cfg, &ctx);
        // FG_RED escape — hard-coded to "\x1b[31m" in glyphs.rs.
        assert!(
            r.text.contains("\x1b[31m"),
            "fallback red color present: {:?}",
            r.text
        );
    }

    #[test]
    fn quotas_hourly_only_renders_hourly_glyph() {
        // `quotas.hourly` dispatcher path renders ONLY the hourly bucket;
        // weekly is absent even when r7d is populated.
        let (mut session, git, other, burn, agents) = mkctx();
        session.r5h = Some(40);
        session.r7d = Some(60);
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let r = render_named("quotas.hourly", Size::M, &ctx).expect("dispatched");
        assert!(
            r.text.contains(CLOCK_5H),
            "hourly glyph present: {}",
            r.text
        );
        assert!(
            !r.text.contains(CALENDAR_7D),
            "weekly glyph absent: {}",
            r.text
        );
    }

    #[test]
    fn quotas_weekly_only_renders_weekly_glyph() {
        let (mut session, git, other, burn, agents) = mkctx();
        session.r5h = Some(40);
        session.r7d = Some(60);
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let r = render_named("quotas.weekly", Size::M, &ctx).expect("dispatched");
        assert!(r.text.contains(CALENDAR_7D), "weekly glyph: {}", r.text);
        assert!(!r.text.contains(CLOCK_5H), "no hourly: {}", r.text);
    }

    #[test]
    fn quotas_unknown_bucket_drops_silently() {
        // `quotas.foo` is not a known bucket — sizes_for/default_size_for must
        // return None so the layout engine drops it, and render_named must
        // also return None (not crash).
        let (session, git, other, burn, agents) = mkctx();
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        assert!(sizes_for("quotas.foo").is_none());
        assert!(default_size_for("quotas.foo").is_none());
        assert!(render_named("quotas.foo", Size::M, &ctx).is_none());
    }

    #[test]
    fn quotas_bare_entry_still_renders_all_buckets() {
        // Back-compat: bare `quotas` renders both populated buckets joined.
        let (mut session, git, other, burn, agents) = mkctx();
        session.r5h = Some(40);
        session.r7d = Some(60);
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let cfg = QuotasConfig::default();
        let r = Quotas.render(Size::M, &cfg, &ctx);
        assert!(r.text.contains(CLOCK_5H));
        assert!(r.text.contains(CALENDAR_7D));
    }

    #[test]
    fn quotas_hourly_and_weekly_at_separate_positions() {
        // Simulate a layout where the two dotted entries sit at non-adjacent
        // positions: ["burn", "quotas.hourly", "ctx_bar", "quotas.weekly"].
        // Verify both render at their layout positions and ordering follows
        // the layout (hourly comes before weekly).
        let (mut session, git, other, _burn_default, agents) = mkctx();
        session.r5h = Some(40);
        session.r7d = Some(60);
        let burn = crate::transcript::BurnInfo {
            tokens_per_hour: 1_000_000,
            tokens_total: 0,
        };
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let names = ["burn", "quotas.hourly", "ctx_bar", "quotas.weekly"];
        let mut parts: Vec<String> = Vec::new();
        for n in names {
            let s = default_size_for(n).expect("known");
            let r = render_named(n, s, &ctx).expect("dispatched");
            if !r.text.is_empty() {
                parts.push(r.text);
            }
        }
        let joined = parts.join(" ");
        let p_hourly = joined.find(CLOCK_5H).expect("hourly rendered");
        let p_weekly = joined.find(CALENDAR_7D).expect("weekly rendered");
        assert!(
            p_hourly < p_weekly,
            "hourly position precedes weekly: {joined}"
        );
        // Sanity: the bare quotas separator (" · ") does NOT appear between
        // them — the two dotted entries are independent items, so the layout
        // separator (a plain space) sits between them rather than the
        // intra-bucket "·".
        let between = &joined[p_hourly..p_weekly];
        assert!(
            !between.contains("·"),
            "no intra-quotas separator between split buckets: {between}"
        );
    }

    #[test]
    fn quotas_dotted_default_priorities() {
        assert_eq!(default_priority("quotas"), 15);
        assert_eq!(default_priority("quotas.hourly"), 25);
        assert_eq!(default_priority("quotas.weekly"), 20);
        assert_eq!(default_priority("quotas.design"), 18);
        assert_eq!(default_priority("quotas.sonnet"), 17);
    }

    #[test]
    fn quotas_per_bucket_priority_override_via_bucket_common() {
        // [quotas.weekly] priority = 99 — the BucketConfig common.priority
        // flows through Config::component_config("quotas.weekly").
        let cfg = QuotasConfig {
            weekly: Some(BucketConfig {
                common: crate::component::ComponentConfig {
                    priority: 99,
                    ..crate::component::ComponentConfig::default()
                },
                ..BucketConfig::default()
            }),
            ..QuotasConfig::default()
        };
        // Simulate the lookup path used by layout::resolve_cfg.
        let bucket_common = cfg
            .weekly
            .as_ref()
            .map(|b| b.common.clone())
            .unwrap_or_default();
        assert_eq!(bucket_common.priority, 99);
    }

    #[test]
    fn quotas_hbar_mode_renders_bar() {
        let (mut session, git, other, burn, agents) = mkctx();
        session.r7d = Some(80);
        session.r7d_reset = None;
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let cfg = QuotasConfig {
            pct: PctConfig {
                mode: PctMode::Hbar,
                width: 5,
                filled: "█".into(),
                empty: " ".into(),
            },
            ..QuotasConfig::default()
        };
        let r = Quotas.render(Size::M, &cfg, &ctx);
        assert!(
            r.text.contains("█"),
            "quotas hbar should contain █: {}",
            r.text
        );
    }

    #[test]
    fn burn_dots_mode_renders_braille_glyph() {
        let (session, git, other, _burn_default, agents) = mkctx();
        let burn = crate::transcript::BurnInfo {
            tokens_per_hour: 2_500_000, // 50% of default 5M ceiling
            tokens_total: 0,
        };
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let cfg = BurnConfig {
            pct: PctConfig {
                mode: PctMode::Dots,
                ..PctConfig::default()
            },
            ..BurnConfig::default()
        };
        let r = Burn.render(Size::Xl, &cfg, &ctx);
        assert!(
            r.text.contains("⡇"),
            "burn dots@50%% should contain ⡇: {}",
            r.text
        );
    }

    #[test]
    fn burn_hbar_mode_renders_bar() {
        let (session, git, other, _burn_default, agents) = mkctx();
        let burn = crate::transcript::BurnInfo {
            tokens_per_hour: 5_000_000, // 100% at default ceiling
            tokens_total: 0,
        };
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let cfg = BurnConfig {
            pct: PctConfig {
                mode: PctMode::Hbar,
                width: 4,
                filled: "█".into(),
                empty: " ".into(),
            },
            ..BurnConfig::default()
        };
        let r = Burn.render(Size::Xl, &cfg, &ctx);
        assert!(
            r.text.contains("████"),
            "burn hbar@100%% should fill: {}",
            r.text
        );
    }

    #[test]
    fn burn_percent_mode_keeps_legacy_text() {
        let (session, git, other, _b, agents) = mkctx();
        let burn = crate::transcript::BurnInfo {
            tokens_per_hour: 1_500_000,
            tokens_total: 0,
        };
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let cfg = BurnConfig::default(); // mode = percent
        let r = Burn.render(Size::Xl, &cfg, &ctx);
        assert!(
            r.text.contains("1.5M"),
            "percent mode keeps human-burn text: {}",
            r.text
        );
        assert!(r.text.contains("/hr"));
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

    // ─── collapse_merged_after_hours filter tests ────────────────────────

    fn lite_merged(ts: Option<i64>) -> crate::transcript::PrStateLite {
        crate::transcript::PrStateLite {
            state: "MERGED".into(),
            is_draft: false,
            merged_at: ts,
        }
    }

    fn lite_state(state: &str) -> crate::transcript::PrStateLite {
        crate::transcript::PrStateLite {
            state: state.into(),
            is_draft: false,
            merged_at: None,
        }
    }

    fn other_with_states(
        urls: Vec<u32>,
        states: Vec<(u32, crate::transcript::PrStateLite)>,
    ) -> OtherPrs {
        let url_map: std::collections::HashMap<String, crate::transcript::PrStateLite> =
            states.into_iter().map(|(n, s)| (url(n), s)).collect();
        OtherPrs {
            urls: urls.into_iter().map(url).collect(),
            states: url_map,
            is_gt: false,
            stack_entries: vec![],
        }
    }

    fn render_chips(other: &OtherPrs, cfg: &ChipsConfig, current_url: &str) -> String {
        let (session, mut git, _o, burn, agents) = mkctx();
        git.pr.url = current_url.into();
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        Chips.render(Size::Xl, cfg, &ctx).text
    }

    #[test]
    fn chips_filters_old_merged_pr() {
        let now = crate::cache::now_epoch();
        let merged_48h = now - 48 * 3600;
        let other = other_with_states(vec![101, 102], vec![(102, lite_merged(Some(merged_48h)))]);
        let cfg = ChipsConfig::default(); // 36h cutoff
        let txt = render_chips(&other, &cfg, "https://github.com/foo/bar/pull/999");
        assert!(txt.contains("#101"), "kept: {txt}");
        assert!(!txt.contains("#102"), "old merged dropped: {txt}");
    }

    #[test]
    fn chips_keeps_recent_merged_pr() {
        let now = crate::cache::now_epoch();
        let merged_12h = now - 12 * 3600;
        let other = other_with_states(vec![101, 102], vec![(102, lite_merged(Some(merged_12h)))]);
        let cfg = ChipsConfig::default();
        let txt = render_chips(&other, &cfg, "https://github.com/foo/bar/pull/999");
        assert!(txt.contains("#101"));
        assert!(txt.contains("#102"), "recent merged kept: {txt}");
    }

    #[test]
    fn chips_keeps_merged_with_unknown_timestamp() {
        let other = other_with_states(vec![101, 102], vec![(102, lite_merged(None))]);
        let cfg = ChipsConfig::default();
        let txt = render_chips(&other, &cfg, "https://github.com/foo/bar/pull/999");
        assert!(txt.contains("#102"), "merged with no ts kept: {txt}");
    }

    #[test]
    fn chips_collapse_zero_disables_filter() {
        let now = crate::cache::now_epoch();
        let merged_old = now - 30 * 24 * 3600;
        let other = other_with_states(vec![101, 102], vec![(102, lite_merged(Some(merged_old)))]);
        let cfg = ChipsConfig {
            collapse_merged_after_hours: 0,
            ..ChipsConfig::default()
        };
        let txt = render_chips(&other, &cfg, "https://github.com/foo/bar/pull/999");
        assert!(txt.contains("#102"), "filter disabled: {txt}");
    }

    #[test]
    fn chips_keeps_old_closed_pr() {
        let other = other_with_states(vec![101, 102], vec![(102, lite_state("CLOSED"))]);
        let cfg = ChipsConfig::default();
        let txt = render_chips(&other, &cfg, "https://github.com/foo/bar/pull/999");
        assert!(txt.contains("#102"), "closed not affected: {txt}");
    }

    #[test]
    fn chips_never_filters_current_branch_pr() {
        let now = crate::cache::now_epoch();
        let merged_48h = now - 48 * 3600;
        // #102 is the current branch PR AND merged 48h ago.
        let other = other_with_states(vec![101, 102], vec![(102, lite_merged(Some(merged_48h)))]);
        let cfg = ChipsConfig::default();
        let txt = render_chips(&other, &cfg, &url(102));
        assert!(
            txt.contains("#102"),
            "current branch PR never filtered: {txt}"
        );
    }

    #[test]
    fn chips_renders_merged_summary_when_collapsed() {
        let now = crate::cache::now_epoch();
        let old = now - 100 * 3600;
        // 3 collapsed merged PRs + 1 visible open PR.
        let other = other_with_states(
            vec![201, 202, 203, 301],
            vec![
                (201, lite_merged(Some(old))),
                (202, lite_merged(Some(old))),
                (203, lite_merged(Some(old))),
                (301, lite_state("OPEN")),
            ],
        );
        let cfg = ChipsConfig::default();
        let txt = render_chips(&other, &cfg, "https://github.com/foo/bar/pull/999");
        assert!(
            txt.contains(&format!("{MERGED}×3")),
            "summary chip with ×3 expected: {txt}"
        );
        // Summary chip must appear before the remaining chip(s).
        let summary_pos = txt
            .find(&format!("{MERGED}×3"))
            .expect("summary chip present");
        let chip_pos = txt.find("#301").expect("open chip present");
        assert!(summary_pos < chip_pos, "summary before chips: {txt}");
        // Collapsed chips themselves should not appear.
        assert!(!txt.contains("#201"), "collapsed dropped: {txt}");
        assert!(!txt.contains("#202"), "collapsed dropped: {txt}");
        assert!(!txt.contains("#203"), "collapsed dropped: {txt}");
        // Not OSC-8 hyperlinked: the merged-summary substring must not be
        // wrapped in a `\x1b]8;;<url>` escape pair around the glyph.
        // (Individual chips elsewhere may still carry OSC-8.) The summary
        // segment itself is plain ANSI.
        let idx = summary_pos;
        // Check there is no OSC-8 opener immediately before the summary
        // colour escape. The summary is rendered as
        // `<FG_GH_MERGED><MERGED>×3<RESET>` with no `\x1b]8;;` prefix.
        let before = &txt[..idx];
        assert!(
            !before.ends_with("\x1b\\"),
            "no OSC-8 opener immediately before summary: {txt}"
        );
    }

    #[test]
    fn chips_no_summary_when_zero_collapsed() {
        let now = crate::cache::now_epoch();
        let recent = now - 3600;
        let other = other_with_states(
            vec![401, 402],
            vec![(401, lite_state("OPEN")), (402, lite_merged(Some(recent)))],
        );
        let cfg = ChipsConfig::default();
        let txt = render_chips(&other, &cfg, "https://github.com/foo/bar/pull/999");
        assert!(
            !txt.contains(&format!("{MERGED}×")),
            "no summary when nothing collapsed: {txt}"
        );
    }

    #[test]
    fn chips_summary_disabled_via_config() {
        let now = crate::cache::now_epoch();
        let old = now - 100 * 3600;
        let other = other_with_states(
            vec![501, 502, 503],
            vec![
                (501, lite_merged(Some(old))),
                (502, lite_merged(Some(old))),
                (503, lite_state("OPEN")),
            ],
        );
        let cfg = ChipsConfig {
            merged_summary: false,
            ..ChipsConfig::default()
        };
        let txt = render_chips(&other, &cfg, "https://github.com/foo/bar/pull/999");
        assert!(
            !txt.contains(&format!("{MERGED}×")),
            "summary suppressed via config: {txt}"
        );
        assert!(txt.contains("#503"), "remaining chip rendered: {txt}");
    }

    #[test]
    fn chips_no_summary_in_stack_mode() {
        let now = crate::cache::now_epoch();
        let old = now - 100 * 3600;
        let mut other = other_with_states(
            vec![601, 602, 603],
            vec![
                (601, lite_merged(Some(old))),
                (602, lite_merged(Some(old))),
                (603, lite_state("OPEN")),
            ],
        );
        other.is_gt = true;
        other.stack_entries = vec![
            entry("feat/a", Some(601), 1),
            entry("feat/b", Some(602), 2),
            entry("feat/c", Some(603), 3),
        ];
        let cfg = ChipsConfig::default();
        let txt = render_chips(&other, &cfg, "https://github.com/foo/bar/pull/999");
        assert!(
            !txt.contains(&format!("{MERGED}×")),
            "no summary in stack mode: {txt}"
        );
    }

    #[test]
    fn chips_stack_mode_bypasses_filter() {
        let now = crate::cache::now_epoch();
        let merged_old = now - 100 * 3600;
        let mut other = other_with_states(
            vec![101, 102],
            vec![
                (101, lite_merged(Some(merged_old))),
                (102, lite_merged(Some(merged_old))),
            ],
        );
        other.is_gt = true;
        other.stack_entries = vec![entry("feat/a", Some(101), 1), entry("feat/b", Some(102), 2)];
        let cfg = ChipsConfig::default();
        let txt = render_chips(&other, &cfg, "https://github.com/foo/bar/pull/999");
        assert!(txt.contains("#101"), "stack bypass: {txt}");
        assert!(txt.contains("#102"), "stack bypass: {txt}");
    }
}
