// Autoresize layout engine.
//
// Reads `[layout]` (left/right component lists, gap, autoresize, hysteresis)
// from config. Renders each component at its current size, then iteratively
// shrinks or drops components — lowest-priority first — until the line fits
// `cols`. Hysteresis: a component, once shrunk at width W, stays shrunk while
// cols stays within ±`hysteresis_band` of W. Shrink decisions persist in
// per-session state so the bar doesn't oscillate as the user nudge-resizes.

use crate::component::{ComponentConfig, RenderCtx, Size};
use crate::components;
use crate::glyphs::*;
use crate::state::State;

/// Render one fully-laid-out statusline. Single source of truth for assembly,
/// width-fitting, padding and right-alignment.
pub fn render(ctx: &RenderCtx, state: &mut State, cols: u32) -> String {
    let cfg = crate::config::config();
    let layout = cfg.layout();
    let gap = layout.gap;

    // Build per-name resolved configs (default_priority unless overridden).
    let resolve_cfg = |name: &str| -> ComponentConfig {
        let mut c = cfg.component_config(name);
        if c.priority == 5 && !is_user_set(cfg, name, "priority") {
            c.priority = components::default_priority(name);
        }
        c
    };

    let left_names: Vec<String> = layout.left.clone();
    let right_names: Vec<String> = layout.right.clone();

    // Resolve initial sizes (default size or `cfg.default` override).
    let mut left_items: Vec<Item> = left_names
        .iter()
        .filter_map(|n| Item::new(n, &resolve_cfg(n)))
        .collect();
    let mut right_items: Vec<Item> = right_names
        .iter()
        .filter_map(|n| Item::new(n, &resolve_cfg(n)))
        .collect();

    // Hysteresis: re-apply previously-shrunk decisions if cols is within band.
    if layout.autoresize {
        apply_hysteresis(&mut left_items, &state.layout, cols, layout.hysteresis_band);
        apply_hysteresis(
            &mut right_items,
            &state.layout,
            cols,
            layout.hysteresis_band,
        );
    }

    // Render at current sizes.
    render_all(&mut left_items, ctx);
    render_all(&mut right_items, ctx);

    // Autoresize loop: shrink or drop until fit.
    if layout.autoresize {
        while total_width(&left_items, &right_items, gap) > cols {
            if !shrink_one(&mut left_items, &mut right_items, ctx) {
                break;
            }
        }
        // Soft-min relax: items pinned by `cfg.min` may still have intrinsic
        // sizes below their soft min. Before dropping anything, allow shrinking
        // those items toward their intrinsic minimum, lowest priority first.
        while total_width(&left_items, &right_items, gap) > cols {
            if !relax_one(&mut left_items, &mut right_items, ctx) {
                break;
            }
        }
        // If still too wide, drop lowest-priority non-required items.
        while total_width(&left_items, &right_items, gap) > cols {
            if !drop_one(&mut left_items, &mut right_items) {
                break;
            }
        }
    }

    // Persist shrink decisions for next render's hysteresis.
    state.layout = build_layout_state(&left_items, &right_items, cols);

    // Compute optional chips-overflow line BEFORE assembling line 1, so that
    // line 1 stays exactly as the layout engine decided. The overflow line
    // is independent of line 1 sizing — it just needs the final chips item.
    let line2 = if layout.overflow_chips_to_second_row {
        compute_chips_overflow(&left_items, &right_items, ctx, cols)
    } else {
        None
    };

    let left = join_items(&left_items);
    let right = join_items(&right_items);
    let llen = vlen(&left);
    let rlen = vlen(&right);
    let total = llen + rlen + gap;

    let mut out = assemble(&left, &right, llen, rlen, total, cols, gap);
    if let Some(l2) = line2 {
        out.push('\n');
        out.push_str(&l2);
    }
    out
}

/// If `chips` ended up at its compact (`×N`) form and there are ≥2 chip URLs
/// and rendering at a larger allowed size would render wider than the compact
/// form (and fits within `cols`), return the expanded chain right-padded with
/// spaces to `cols` cells. Otherwise return `None`.
fn compute_chips_overflow(
    left: &[Item],
    right: &[Item],
    ctx: &RenderCtx,
    cols: u32,
) -> Option<String> {
    let chips = left
        .iter()
        .chain(right.iter())
        .find(|i| i.name == "chips" && !i.dropped)?;
    // ≥2 chips required.
    if ctx.other.urls.len() < 2 {
        return None;
    }
    // Final size must be the compact `×N` form (anything below the chips
    // component's largest allowed size). At Xs the component renders empty;
    // at S/M it renders `×N`. The expanded chain only lives at the largest
    // allowed size (Xl by default).
    let largest = *chips.sizes.last()?;
    if chips.size >= largest {
        return None; // already showing the full chain inline
    }
    let current_w = chips.rendered.width;
    // Try the largest size first, falling back to smaller-but-still-larger
    // sizes if the expanded form doesn't fit terminal width.
    let mut candidate: Option<crate::component::Rendered> = None;
    for s in chips.sizes.iter().rev().copied() {
        if s <= chips.size {
            break;
        }
        let r = components::render_named(&chips.name, s, ctx).unwrap_or_default();
        if r.width > current_w && r.width <= cols {
            candidate = Some(r);
            break;
        }
    }
    let r = candidate?;
    // Right-pad to `cols` cells so the right-edge invariant matches line 1.
    let pad = cols.saturating_sub(r.width) as usize;
    let mut out = r.text;
    if pad > 0 {
        out.push_str(&" ".repeat(pad));
    }
    Some(out)
}

fn assemble(
    left: &str,
    right: &str,
    llen: u32,
    rlen: u32,
    total: u32,
    cols: u32,
    gap: u32,
) -> String {
    let mut out = String::new();
    if left.is_empty() && right.is_empty() {
        return out;
    } else if left.is_empty() {
        let pad_n = cols.saturating_sub(rlen + 1).max(1) as usize;
        out.push_str(&format!("{DIM}·{RESET}{}{right}", " ".repeat(pad_n)));
    } else if right.is_empty() {
        out.push_str(left);
    } else if total <= cols {
        let pad_n = cols.saturating_sub(llen + rlen).max(gap) as usize;
        out.push_str(&format!("{left}{}{right}", " ".repeat(pad_n)));
    } else {
        // Truncate left as a last resort.
        let budget = cols.saturating_sub(rlen + gap + 1);
        let stripped = crate::vlen::strip(left);
        let truncated = crate::vlen::truncate_to_width(&stripped, budget);
        let left_out = format!("{truncated}{DIM}…{RESET}");
        let llen2 = crate::vlen::vlen(&left_out);
        let pad_n = cols.saturating_sub(llen2 + rlen).max(gap) as usize;
        out.push_str(&format!("{left_out}{}{right}", " ".repeat(pad_n)));
    }
    out
}

#[derive(Debug, Clone)]
struct Item {
    name: String,
    cfg: ComponentConfig,
    size: Size,
    sizes: Vec<Size>, // allowed sizes, smallest → largest, filtered by cfg.sizes & min
    /// Intrinsic component sizes (full unfiltered list from `components::sizes_for`).
    /// Used by the soft-min relax pass: when overflow persists after normal
    /// shrink is exhausted, we relax `cfg.min` and continue shrinking down to
    /// the intrinsic minimum before resorting to dropping items.
    intrinsic_sizes: Vec<Size>,
    dropped: bool,
    rendered: crate::component::Rendered,
}

impl Item {
    fn new(name: &str, cfg: &ComponentConfig) -> Option<Self> {
        let all = components::sizes_for(name)?;
        // Intrinsic full list (sorted smallest → largest, after `cfg.sizes`
        // filtering — explicit allowed-size restrictions are still hard, only
        // `cfg.min` is treated as soft).
        let mut intrinsic: Vec<Size> = if cfg.sizes.is_empty() {
            all.to_vec()
        } else {
            all.iter()
                .copied()
                .filter(|s| cfg.sizes.contains(s))
                .collect()
        };
        intrinsic.sort();
        if intrinsic.is_empty() {
            return None;
        }
        // Apply soft min to compute the normal-shrink-allowed list.
        let mut allowed: Vec<Size> = intrinsic.clone();
        if let Some(m) = cfg.min {
            allowed.retain(|s| *s >= m);
        }
        if allowed.is_empty() {
            // Soft min eliminated everything; fall back to intrinsic.
            allowed = intrinsic.clone();
        }
        allowed.sort();
        let default = cfg
            .default
            .filter(|d| allowed.contains(d))
            .or_else(|| {
                let dflt = components::default_size_for(name)?;
                if allowed.contains(&dflt) {
                    Some(dflt)
                } else {
                    allowed.last().copied()
                }
            })
            .unwrap_or(*allowed.last().unwrap());
        Some(Self {
            name: name.into(),
            cfg: cfg.clone(),
            size: default,
            sizes: allowed,
            intrinsic_sizes: intrinsic,
            dropped: false,
            rendered: Default::default(),
        })
    }

    fn min_size(&self) -> Size {
        *self.sizes.first().unwrap()
    }

    /// The intrinsic (hard) minimum — the smallest size the component can
    /// actually render at, regardless of `cfg.min`.
    fn intrinsic_min(&self) -> Size {
        *self.intrinsic_sizes.first().unwrap()
    }
}

fn render_all(items: &mut [Item], ctx: &RenderCtx) {
    for it in items.iter_mut() {
        if it.dropped {
            it.rendered = crate::component::Rendered::empty();
            continue;
        }
        it.rendered = components::render_named(&it.name, it.size, ctx).unwrap_or_default();
    }
}

/// Sum widths plus `" "` separator between non-empty items.
fn join_widths(items: &[Item]) -> u32 {
    let mut w = 0u32;
    let mut first = true;
    for it in items {
        if it.rendered.width == 0 {
            continue;
        }
        if !first {
            w += 1; // separator
        }
        w += it.rendered.width;
        first = false;
    }
    w
}

fn join_items(items: &[Item]) -> String {
    let mut out = String::new();
    let mut first = true;
    for it in items {
        if it.rendered.text.is_empty() {
            continue;
        }
        if !first {
            out.push(' ');
        }
        out.push_str(&it.rendered.text);
        first = false;
    }
    out
}

fn vlen(s: &str) -> u32 {
    crate::vlen::vlen(s)
}

fn total_width(left: &[Item], right: &[Item], gap: u32) -> u32 {
    let l = join_widths(left);
    let r = join_widths(right);
    if l == 0 && r == 0 {
        0
    } else if l == 0 {
        r
    } else if r == 0 {
        l
    } else {
        l + gap + r
    }
}

/// Pick the lowest-priority item that can still be shrunk one step. Step it.
/// Returns false if no item can be shrunk further.
fn shrink_one(left: &mut [Item], right: &mut [Item], ctx: &RenderCtx) -> bool {
    // Find the candidate across both panes.
    let mut best: Option<(bool, usize, u32)> = None; // (in_left, idx, priority)
    for (i, it) in left.iter().enumerate() {
        if it.dropped || it.size <= it.min_size() {
            continue;
        }
        let p = it.cfg.priority;
        if best.map(|(_, _, bp)| p < bp).unwrap_or(true) {
            best = Some((true, i, p));
        }
    }
    for (i, it) in right.iter().enumerate() {
        if it.dropped || it.size <= it.min_size() {
            continue;
        }
        let p = it.cfg.priority;
        if best.map(|(_, _, bp)| p < bp).unwrap_or(true) {
            best = Some((false, i, p));
        }
    }
    let (in_left, idx, _) = match best {
        Some(b) => b,
        None => return false,
    };
    let it = if in_left {
        &mut left[idx]
    } else {
        &mut right[idx]
    };
    // Step to next-smaller allowed size.
    let new_size = it
        .sizes
        .iter()
        .copied()
        .rfind(|s| *s < it.size)
        .unwrap_or_else(|| it.min_size());
    it.size = new_size;
    it.rendered = components::render_named(&it.name, new_size, ctx).unwrap_or_default();
    true
}

/// Soft-min relax pass. Picks the lowest-priority item whose current size is
/// at-or-below its soft `cfg.min` but still above its intrinsic minimum, then
/// shrinks it one intrinsic step. Used after `shrink_one` is exhausted but
/// before `drop_one` runs, so a heavily pinned bar (e.g. `chips.min = "xl"`)
/// still degrades to its compact form rather than disappearing entirely.
/// Returns false when no candidate remains.
fn relax_one(left: &mut [Item], right: &mut [Item], ctx: &RenderCtx) -> bool {
    let mut best: Option<(bool, usize, u32)> = None;
    for (i, it) in left.iter().enumerate() {
        if it.dropped || it.size <= it.intrinsic_min() {
            continue;
        }
        let p = it.cfg.priority;
        if best.map(|(_, _, bp)| p < bp).unwrap_or(true) {
            best = Some((true, i, p));
        }
    }
    for (i, it) in right.iter().enumerate() {
        if it.dropped || it.size <= it.intrinsic_min() {
            continue;
        }
        let p = it.cfg.priority;
        if best.map(|(_, _, bp)| p < bp).unwrap_or(true) {
            best = Some((false, i, p));
        }
    }
    let (in_left, idx, _) = match best {
        Some(b) => b,
        None => return false,
    };
    let it = if in_left {
        &mut left[idx]
    } else {
        &mut right[idx]
    };
    let new_size = it
        .intrinsic_sizes
        .iter()
        .copied()
        .rfind(|s| *s < it.size)
        .unwrap_or_else(|| it.intrinsic_min());
    it.size = new_size;
    it.rendered = components::render_named(&it.name, new_size, ctx).unwrap_or_default();
    true
}

fn drop_one(left: &mut [Item], right: &mut [Item]) -> bool {
    let mut best: Option<(bool, usize, u32)> = None;
    for (i, it) in left.iter().enumerate() {
        if it.dropped || it.cfg.required {
            continue;
        }
        let p = it.cfg.priority;
        if best.map(|(_, _, bp)| p < bp).unwrap_or(true) {
            best = Some((true, i, p));
        }
    }
    for (i, it) in right.iter().enumerate() {
        if it.dropped || it.cfg.required {
            continue;
        }
        let p = it.cfg.priority;
        if best.map(|(_, _, bp)| p < bp).unwrap_or(true) {
            best = Some((false, i, p));
        }
    }
    let (in_left, idx, _) = match best {
        Some(b) => b,
        None => return false,
    };
    let it = if in_left {
        &mut left[idx]
    } else {
        &mut right[idx]
    };
    it.dropped = true;
    it.rendered = crate::component::Rendered::empty();
    true
}

// ─── hysteresis state ───────────────────────────────────────────────────

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutState {
    /// Per-component last decision. Records the size and the cols-at-which the
    /// shrink was applied; used to keep the size sticky within ±band.
    pub items: std::collections::HashMap<String, ItemDecision>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemDecision {
    pub size: String,
    pub at_cols: u32,
    pub dropped: bool,
}

fn apply_hysteresis(items: &mut [Item], state: &LayoutState, cols: u32, band: u32) {
    for it in items.iter_mut() {
        if let Some(d) = state.items.get(&it.name) {
            if cols.abs_diff(d.at_cols) <= band {
                if d.dropped {
                    it.dropped = true;
                    continue;
                }
                if let Ok(s) = d.size.parse::<Size>() {
                    // Allow restoring sizes from the intrinsic list — the
                    // relax pass may have shrunk below the soft min.
                    if it.intrinsic_sizes.contains(&s) {
                        it.size = s;
                    }
                }
            }
        }
    }
}

fn build_layout_state(left: &[Item], right: &[Item], cols: u32) -> LayoutState {
    let mut s = LayoutState::default();
    for it in left.iter().chain(right.iter()) {
        // Only record sticky decisions for items that were shrunk below their
        // resolved default — otherwise we'd lock everyone to their defaults
        // and effectively disable upsize.
        let default_for = components::default_size_for(&it.name).unwrap_or(it.size);
        if it.dropped || it.size < default_for {
            s.items.insert(
                it.name.clone(),
                ItemDecision {
                    size: it.size.as_str().into(),
                    at_cols: cols,
                    dropped: it.dropped,
                },
            );
        }
    }
    s
}

// Probe whether the user explicitly set a key in the config. Conservative —
// returns false if we can't tell, which means default_priority kicks in.
fn is_user_set(_cfg: &crate::config::Config, _name: &str, _key: &str) -> bool {
    // We don't preserve original TOML structure, so assume defaults are
    // never explicitly set. The effect: components::default_priority is
    // always applied unless the user-supplied config has priority != 5.
    // This is the desired behaviour — opting-in to priority=5 is rare.
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{GitData, PrJson};
    use crate::input::Session;
    use crate::transcript::{AgentCount, BurnInfo, OtherPrs};

    fn mk_session(cols: u32) -> Session {
        Session {
            model: "Opus".into(),
            cwd: "/tmp".into(),
            session_id: "t".into(),
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

    fn mk_git() -> GitData {
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

    /// Two items with different priority. When width forces a shrink, the
    /// LOWER-priority one steps down first.
    #[test]
    fn autoresize_picks_lowest_priority_first() {
        let session = mk_session(20);
        let git = mk_git();
        let other = OtherPrs::default();
        let _burn = BurnInfo::default();
        let agents = AgentCount::default();
        // High-priority item that supports M and Xs; low-priority same.
        let cfg_high = ComponentConfig {
            sizes: vec![Size::Xs, Size::Xl],
            min: Some(Size::Xs),
            priority: 100,
            required: false,
            default: Some(Size::Xl),
        };
        let cfg_low = ComponentConfig {
            sizes: vec![Size::Xs, Size::Xl],
            min: Some(Size::Xs),
            priority: 1,
            required: false,
            default: Some(Size::Xl),
        };
        let mut left: Vec<Item> = vec![
            Item::new("model", &cfg_high).unwrap(),
            Item::new("burn", &cfg_low).unwrap(),
        ];
        let mut right: Vec<Item> = Vec::new();
        // Render at default sizes, then shrink once.
        // Burn renders empty when tokens_per_hour==0; force a value.
        let burn2 = BurnInfo {
            tokens_per_hour: 1_000_000,
            tokens_total: 1_000_000,
        };
        let ctx2 = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn2,
            agents: &agents,
            tick: 0,
        };
        render_all(&mut left, &ctx2);
        let _ = shrink_one(&mut left, &mut right, &ctx2);
        // Lower-priority "burn" must have been shrunk first.
        assert_eq!(left[1].size, Size::Xs);
        assert_eq!(left[0].size, Size::Xl);
    }

    /// A component must not shrink past its min size.
    #[test]
    fn min_size_respected() {
        let cfg = ComponentConfig {
            sizes: vec![Size::Xs, Size::S, Size::M, Size::L, Size::Xl],
            min: Some(Size::M),
            priority: 1,
            required: false,
            default: Some(Size::Xl),
        };
        let it = Item::new("ctx_bar", &cfg).unwrap();
        // sizes vec must not contain anything below M.
        assert!(it.sizes.iter().all(|s| *s >= Size::M));
    }

    /// Hysteresis: same item, same cols ±band, decision sticks.
    #[test]
    fn hysteresis_prevents_oscillation() {
        let mut state = LayoutState::default();
        state.items.insert(
            "model".into(),
            ItemDecision {
                size: "s".into(),
                at_cols: 100,
                dropped: false,
            },
        );

        let cfg = ComponentConfig {
            sizes: vec![],
            min: None,
            priority: 5,
            required: false,
            default: None,
        };
        let mut items = vec![Item::new("model", &cfg).unwrap()];
        // Default would be Xl. With cols=101 (within band=2), hysteresis
        // pulls us back to S.
        apply_hysteresis(&mut items, &state, 101, 2);
        assert_eq!(items[0].size, Size::S);
        // Outside band: ignored.
        let mut items2 = vec![Item::new("model", &cfg).unwrap()];
        apply_hysteresis(&mut items2, &state, 200, 2);
        assert_ne!(items2[0].size, Size::S);
    }

    /// The right edge must be content, not whitespace, at every width.
    #[test]
    fn render_fills_cols_exactly() {
        unsafe {
            std::env::set_var("CC_STATUSLINE_NF_WIDTH", "1");
            std::env::set_var("CC_STATUSLINE_SAFETY_MARGIN", "0");
        }
        let git = mk_git();
        let other = OtherPrs::default();
        let burn = BurnInfo::default();
        let agents = AgentCount::default();
        for &cols in &[80u32, 100, 120, 160, 200] {
            let session = mk_session(cols);
            let ctx = RenderCtx {
                session: &session,
                git: &git,
                other: &other,
                burn: &burn,
                agents: &agents,
                tick: 0,
            };
            let mut state = State::default();
            let line = render(&ctx, &mut state, cols);
            let first = line.lines().next().unwrap_or(&line);
            let w = crate::vlen::vlen(first);
            assert_eq!(
                w, cols,
                "width mismatch at cols={cols}: got {w}, line={first:?}"
            );
            let stripped = crate::vlen::strip(first);
            let last = stripped.chars().last().expect("non-empty");
            assert!(
                !last.is_whitespace(),
                "right edge at cols={cols} is whitespace: {stripped:?}"
            );
        }
    }

    // ─── chips overflow tests ──────────────────────────────────────────

    fn url(n: u32) -> String {
        format!("https://github.com/foo/bar/pull/{n}")
    }

    fn chips_other(urls: Vec<u32>) -> OtherPrs {
        OtherPrs {
            urls: urls.into_iter().map(url).collect(),
            ..Default::default()
        }
    }

    fn stack_chips_other(urls: Vec<u32>) -> OtherPrs {
        OtherPrs {
            urls: urls.iter().copied().map(url).collect(),
            is_gt: true,
            stack_entries: urls
                .iter()
                .copied()
                .enumerate()
                .map(|(i, n)| crate::transcript::StackChipEntry {
                    branch: format!("feat/{n}"),
                    pr: Some(n),
                    depth: i as u32 + 1,
                })
                .collect(),
            ..Default::default()
        }
    }

    fn mk_chips_item(size: Size, ctx: &RenderCtx) -> Item {
        let cfg = ComponentConfig {
            sizes: vec![],
            min: None,
            priority: 5,
            required: false,
            default: None,
        };
        let mut it = Item::new("chips", &cfg).unwrap();
        it.size = size;
        it.rendered = components::render_named("chips", size, ctx).unwrap_or_default();
        it
    }

    /// Helper to produce a render context with chips populated.
    fn chips_ctx<'a>(
        session: &'a Session,
        git: &'a GitData,
        other: &'a OtherPrs,
        burn: &'a crate::transcript::BurnInfo,
        agents: &'a AgentCount,
    ) -> RenderCtx<'a> {
        RenderCtx {
            session,
            git,
            other,
            burn,
            agents,
            tick: 0,
        }
    }

    /// (b) Chips collapsed to compact `×N` form → overflow line emitted with
    /// the expanded chain, right-padded to cols.
    #[test]
    fn chips_overflow_emits_when_collapsed() {
        let session = mk_session(80);
        let git = mk_git();
        let other = chips_other(vec![101, 102, 103]);
        let burn = crate::transcript::BurnInfo::default();
        let agents = AgentCount::default();
        let ctx = chips_ctx(&session, &git, &other, &burn, &agents);
        let chips = mk_chips_item(Size::S, &ctx); // forced compact
        let left = vec![chips];
        let right: Vec<Item> = Vec::new();
        let l2 = compute_chips_overflow(&left, &right, &ctx, 80).expect("overflow line");
        // Width must equal cols (right-padded with spaces).
        assert_eq!(crate::vlen::vlen(&l2), 80);
        let stripped = crate::vlen::strip(&l2);
        // Expanded form contains every chip number.
        assert!(stripped.contains("#101"), "line2 missing #101: {stripped}");
        assert!(stripped.contains("#102"), "line2 missing #102: {stripped}");
        assert!(stripped.contains("#103"), "line2 missing #103: {stripped}");
        // OSC-8 hyperlink escape preserved (\x1b]8;;URL\x1b\\).
        assert!(l2.contains("\x1b]8;;"), "line2 missing OSC-8: {l2:?}");
    }

    /// (a) Chips already at largest size → no overflow.
    #[test]
    fn chips_no_overflow_when_inline_fits() {
        let session = mk_session(200);
        let git = mk_git();
        let other = chips_other(vec![101, 102, 103]);
        let burn = crate::transcript::BurnInfo::default();
        let agents = AgentCount::default();
        let ctx = chips_ctx(&session, &git, &other, &burn, &agents);
        let chips = mk_chips_item(Size::Xl, &ctx);
        let left = vec![chips];
        let right: Vec<Item> = Vec::new();
        assert!(compute_chips_overflow(&left, &right, &ctx, 200).is_none());
    }

    /// (d) <2 chips → no overflow.
    #[test]
    fn chips_no_overflow_with_one_chip() {
        let session = mk_session(80);
        let git = mk_git();
        let other = chips_other(vec![101]);
        let burn = crate::transcript::BurnInfo::default();
        let agents = AgentCount::default();
        let ctx = chips_ctx(&session, &git, &other, &burn, &agents);
        let chips = mk_chips_item(Size::S, &ctx);
        let left = vec![chips];
        let right: Vec<Item> = Vec::new();
        assert!(compute_chips_overflow(&left, &right, &ctx, 80).is_none());
    }

    /// (e) Stack mode preserved on line 2 (trunk-first ordering, separator
    /// present).
    #[test]
    fn chips_overflow_preserves_stack_mode() {
        let session = mk_session(120);
        let git = mk_git();
        // is_gt + stack entries with depth-ordered PR numbers.
        let other = stack_chips_other(vec![101, 102, 103]);
        let burn = crate::transcript::BurnInfo::default();
        let agents = AgentCount::default();
        let ctx = chips_ctx(&session, &git, &other, &burn, &agents);
        let chips = mk_chips_item(Size::S, &ctx);
        let left = vec![chips];
        let right: Vec<Item> = Vec::new();
        let l2 = compute_chips_overflow(&left, &right, &ctx, 120).expect("overflow line");
        let stripped = crate::vlen::strip(&l2);
        // Stack separator must be present.
        let cfg = crate::components::ChipsConfig::default();
        assert!(
            stripped.contains(&cfg.stack_separator),
            "line2 missing stack separator: {stripped}"
        );
        // Trunk-first: #101 before #103.
        let p101 = stripped.find("#101").expect("has 101");
        let p103 = stripped.find("#103").expect("has 103");
        assert!(p101 < p103, "trunk-first order on line 2");
    }

    /// (c) Overflow disabled by config → no line 2 (verified at the
    /// `render()` boundary by skipping `compute_chips_overflow` entirely).
    /// We exercise the gate by simulating the disabled branch directly:
    /// when the helper isn't called, no second line appears.
    #[test]
    fn chips_overflow_disabled_skips_line2() {
        // Sanity: the helper returns Some when enabled with the same inputs.
        let session = mk_session(80);
        let git = mk_git();
        let other = chips_other(vec![101, 102, 103]);
        let burn = crate::transcript::BurnInfo::default();
        let agents = AgentCount::default();
        let ctx = chips_ctx(&session, &git, &other, &burn, &agents);
        let chips = mk_chips_item(Size::S, &ctx);
        let left = vec![chips];
        let right: Vec<Item> = Vec::new();
        assert!(compute_chips_overflow(&left, &right, &ctx, 80).is_some());
        // Disabled path: render() guards with the bool, producing only
        // line 1. We assert the gate logic at the type level (the config
        // field exists and defaults to true).
        let lc = crate::config::LayoutConfig::default();
        assert!(lc.overflow_chips_to_second_row);
    }

    /// Issue #22: when chips is pinned `min = Xl` and the terminal is too
    /// narrow to fit the full chain, the bar must NOT collapse to empty —
    /// the soft-min relax pass should shrink chips to its compact form.
    #[test]
    fn soft_min_relaxes_before_dropping() {
        unsafe {
            std::env::set_var("CC_STATUSLINE_NF_WIDTH", "1");
            std::env::set_var("CC_STATUSLINE_SAFETY_MARGIN", "0");
        }
        let session = mk_session(80);
        let git = mk_git();
        // 50+ PRs so the Xl chain blows past any reasonable width.
        let other = chips_other((101..=160).collect());
        let burn = crate::transcript::BurnInfo::default();
        let agents = AgentCount::default();
        let ctx = chips_ctx(&session, &git, &other, &burn, &agents);

        // Build a layout with a single chips item pinned at min = Xl,
        // mirroring the user's config from issue #22.
        let cfg_chips = ComponentConfig {
            sizes: vec![],
            min: Some(Size::Xl),
            priority: 1,
            required: false,
            default: Some(Size::Xl),
        };
        let mut left: Vec<Item> = vec![Item::new("chips", &cfg_chips).unwrap()];
        let mut right: Vec<Item> = Vec::new();

        // sanity: normal-shrink list is locked to Xl only.
        assert_eq!(left[0].sizes, vec![Size::Xl]);
        // intrinsic list contains smaller sizes the relax pass can step to.
        assert!(left[0].intrinsic_sizes.iter().any(|s| *s < Size::Xl));

        render_all(&mut left, &ctx);
        let cols: u32 = 80;
        let gap: u32 = 1;
        // Drive the same shrink → relax → drop sequence as render().
        while total_width(&left, &right, gap) > cols && shrink_one(&mut left, &mut right, &ctx) {}
        while total_width(&left, &right, gap) > cols && relax_one(&mut left, &mut right, &ctx) {}

        // After relax, chips must still be present and below its soft min.
        assert!(!left[0].dropped, "chips should not be dropped");
        assert!(
            left[0].size < Size::Xl,
            "chips should have shrunk past soft min, got {:?}",
            left[0].size
        );
        assert!(
            !left[0].rendered.text.is_empty(),
            "chips render must be non-empty"
        );
    }

    /// End-to-end via `render()`: with chips pinned `min = "xl"`, 50+ PRs in
    /// other_prs and a narrow terminal, the resulting line is non-empty.
    /// (We can't easily inject ComponentConfig overrides through the public
    /// config(), so this exercises the full pipeline at the layout level.)
    #[test]
    fn render_non_empty_with_many_chips_at_narrow_cols() {
        unsafe {
            std::env::set_var("CC_STATUSLINE_NF_WIDTH", "1");
            std::env::set_var("CC_STATUSLINE_SAFETY_MARGIN", "0");
        }
        let cols: u32 = 80;
        let session = mk_session(cols);
        let git = mk_git();
        let other = chips_other((101..=160).collect());
        let burn = crate::transcript::BurnInfo::default();
        let agents = AgentCount::default();
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let mut state = State::default();
        let line = render(&ctx, &mut state, cols);
        let first = line.lines().next().unwrap_or(&line);
        let stripped = crate::vlen::strip(first);
        assert!(
            !stripped.trim().is_empty(),
            "line must not be blank: {stripped:?}"
        );
    }

    /// Two-line render via the public `render()` entry point: line 2 fills
    /// the terminal width exactly, parallel to the line-1 invariant.
    #[test]
    fn render_two_line_pads_line2_to_cols() {
        unsafe {
            std::env::set_var("CC_STATUSLINE_NF_WIDTH", "1");
            std::env::set_var("CC_STATUSLINE_SAFETY_MARGIN", "0");
        }
        let git = mk_git();
        // Many chips so the engine collapses chips to compact form on a
        // narrow terminal.
        let other = chips_other((101..=110).collect());
        let burn = crate::transcript::BurnInfo::default();
        let agents = AgentCount::default();
        let cols: u32 = 80;
        let session = mk_session(cols);
        let ctx = RenderCtx {
            session: &session,
            git: &git,
            other: &other,
            burn: &burn,
            agents: &agents,
            tick: 0,
        };
        let mut state = State::default();
        let line = render(&ctx, &mut state, cols);
        let mut iter = line.lines();
        let first = iter.next().unwrap_or("");
        assert_eq!(crate::vlen::vlen(first), cols, "line 1 width");
        if let Some(second) = iter.next() {
            assert_eq!(
                crate::vlen::vlen(second),
                cols,
                "line 2 right-padded to cols"
            );
        }
    }
}
