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
        // If still too wide, drop lowest-priority non-required items.
        while total_width(&left_items, &right_items, gap) > cols {
            if !drop_one(&mut left_items, &mut right_items) {
                break;
            }
        }
    }

    // Persist shrink decisions for next render's hysteresis.
    state.layout = build_layout_state(&left_items, &right_items, cols);

    let left = join_items(&left_items);
    let right = join_items(&right_items);
    let llen = vlen(&left);
    let rlen = vlen(&right);

    // Two-line fallback: if expanded chips on the left pushed us over cols,
    // try compacting them and putting expanded chips on line 2. Best-effort.
    let mut line2 = String::new();
    let total = llen + rlen + gap;
    if total > cols {
        // If chips is in left at Xl, demote to S and stash expanded for line 2.
        if let Some(idx) = left_items
            .iter()
            .position(|i| i.name == "chips" && i.size == Size::Xl)
        {
            let chips_xl_text = left_items[idx].rendered.text.clone();
            // Demote to small.
            left_items[idx].size = Size::S;
            let r = components::render_named("chips", Size::S, ctx).unwrap_or_default();
            left_items[idx].rendered = r;
            line2 = chips_xl_text.trim_start().to_string();
        }
    }

    let left = join_items(&left_items);
    let llen = vlen(&left);
    let total = llen + rlen + gap;

    let mut out = assemble(&left, &right, llen, rlen, total, cols, gap);
    if !line2.is_empty() {
        out.push('\n');
        let trimmed = line2.strip_prefix(' ').unwrap_or(&line2);
        out.push_str(trimmed);
    }
    out
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
    dropped: bool,
    rendered: crate::component::Rendered,
}

impl Item {
    fn new(name: &str, cfg: &ComponentConfig) -> Option<Self> {
        let all = components::sizes_for(name)?;
        // Filter to allowed sizes per cfg.
        let mut allowed: Vec<Size> = if cfg.sizes.is_empty() {
            all.to_vec()
        } else {
            all.iter()
                .copied()
                .filter(|s| cfg.sizes.contains(s))
                .collect()
        };
        // Apply min.
        if let Some(m) = cfg.min {
            allowed.retain(|s| *s >= m);
        }
        if allowed.is_empty() {
            return None;
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
            dropped: false,
            rendered: Default::default(),
        })
    }

    fn min_size(&self) -> Size {
        *self.sizes.first().unwrap()
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
                    if it.sizes.contains(&s) {
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
}
