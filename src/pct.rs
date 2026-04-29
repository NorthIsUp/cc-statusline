// Unified percentage display modes shared by all percent-aware components
// (ctx_bar, quotas, …).
//
// `PctMode` selects the rendering family; `PctConfig` carries the per-mode
// knobs (bar width and the hbar-mode filled/empty glyph overrides). `render`
// turns a 0..=100 integer percent into a colored string via `bar_color`
// (the same red/yellow/dim breakpoints used by the original `ctx_bar`).
//
// Modes:
//   percent   "47%"
//   float     "0.47"      (always 2 decimals, no '%')
//   dots      1 cell, 9 steps:   ⠀⡀⡄⡆⡇⣇⣧⣷⣿
//   shaded    1 cell, 5 steps:   ' ', ░, ▒, ▓, █
//   hbar      `width` cells, full █ + partial ▏▎▍▌▋▊▉ for the leading sub-cell
//             (default — replaces the old `blocks` look with sub-cell precision)
//   vbar      1 cell, 8 vertical levels:   ▁▂▃▄▅▆▇█

use crate::glyphs::{DIM, FG_RED, FG_YELLOW, RESET};
use serde::Deserialize;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PctMode {
    #[default]
    Percent,
    Float,
    Dots,
    Shaded,
    Hbar,
    Vbar,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PctConfig {
    pub mode: PctMode,
    pub width: u32,
    /// Override for the full-cell glyph in `hbar` mode. Defaults to `█`.
    pub filled: String,
    /// Override for the empty-cell glyph in `hbar` mode. Defaults to a single
    /// space so the bar reads cleanly as "filled to here, empty after".
    pub empty: String,
}

impl Default for PctConfig {
    fn default() -> Self {
        Self {
            mode: PctMode::Percent,
            width: 10,
            filled: "█".into(),
            empty: " ".into(),
        }
    }
}

/// Red ≥80%, yellow ≥50%, dim otherwise. Single source of truth for every
/// percent-aware component's colour band.
pub fn bar_color(pct: u32) -> &'static str {
    if pct >= 80 {
        FG_RED
    } else if pct >= 50 {
        FG_YELLOW
    } else {
        DIM
    }
}

/// 9-step braille ramp: 0/8…8/8.
const DOTS_STEPS: [&str; 9] = ["⠀", "⡀", "⡄", "⡆", "⡇", "⣇", "⣧", "⣷", "⣿"];

/// 5-step shade ramp: 0/4…4/4. The first step is a literal space cell so the
/// glyph still occupies one column.
const SHADED_STEPS: [&str; 5] = [" ", "░", "▒", "▓", "█"];

/// 8-step vertical bar ramp: 1/8…8/8. Index 0 is "empty" — a literal space so
/// the cell still occupies one column.
const VBAR_STEPS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// 7 partial-eighth glyphs (1/8…7/8).
const HBAR_PARTIAL: [&str; 7] = ["▏", "▎", "▍", "▌", "▋", "▊", "▉"];

fn clamp_pct(pct: u32) -> u32 {
    pct.min(100)
}

fn render_percent(pct: u32) -> String {
    format!("{pct}%")
}

fn render_float(pct: u32) -> String {
    let whole = pct / 100;
    let frac = pct % 100;
    format!("{whole}.{frac:02}")
}

fn render_dots(pct: u32) -> String {
    // 9 buckets, evenly distributed across 0..=100. Use rounding so that
    // exact step boundaries (k/8 of 100) land on bucket k.
    let idx = ((pct as u64 * 8 + 50) / 100) as usize;
    let idx = idx.min(8);
    DOTS_STEPS[idx].to_string()
}

fn render_shaded(pct: u32) -> String {
    // 5 buckets across 0..=100 with rounding.
    let idx = ((pct as u64 * 4 + 50) / 100) as usize;
    let idx = idx.min(4);
    SHADED_STEPS[idx].to_string()
}

fn render_vbar(pct: u32) -> String {
    // 9 buckets (0..=8), rounded.
    let idx = ((pct as u64 * 8 + 50) / 100) as usize;
    let idx = idx.min(8);
    VBAR_STEPS[idx].to_string()
}

fn render_hbar(pct: u32, width: u32, filled: &str, empty: &str) -> String {
    if width == 0 {
        return String::new();
    }
    // Total eighth-units across the bar.
    let total_eighths = (width as u64) * 8;
    let filled_eighths = (pct as u64 * total_eighths) / 100;
    let full_cells = (filled_eighths / 8) as u32;
    let partial = (filled_eighths % 8) as usize;

    let mut s = String::new();
    for _ in 0..full_cells {
        s.push_str(filled);
    }
    let mut emitted = full_cells;
    if partial > 0 && emitted < width {
        s.push_str(HBAR_PARTIAL[partial - 1]);
        emitted += 1;
    }
    while emitted < width {
        s.push_str(empty);
        emitted += 1;
    }
    s
}

/// Render a percent (0..=100; clamped above) using `cfg`. Wraps the visible
/// glyph(s) in the appropriate colour code from `bar_color`.
pub fn render(pct: u32, cfg: &PctConfig) -> String {
    let body = render_plain(pct, cfg);
    let c = bar_color(clamp_pct(pct));
    format!("{c}{body}{RESET}")
}

/// Strip the colour wrapping for tests / inspection. Useful when callers want
/// to compose the rendered glyphs with their own styling (e.g. `quotas`
/// wrapping with pace-aware colour from `quota::pct_color`).
pub fn render_plain(pct: u32, cfg: &PctConfig) -> String {
    let pct = clamp_pct(pct);
    match cfg.mode {
        PctMode::Percent => render_percent(pct),
        PctMode::Float => render_float(pct),
        PctMode::Dots => render_dots(pct),
        PctMode::Shaded => render_shaded(pct),
        PctMode::Hbar => render_hbar(pct, cfg.width, &cfg.filled, &cfg.empty),
        PctMode::Vbar => render_vbar(pct),
    }
}

// ─── tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(mode: PctMode) -> PctConfig {
        PctConfig {
            mode,
            ..PctConfig::default()
        }
    }

    fn cfg_w(mode: PctMode, width: u32) -> PctConfig {
        PctConfig {
            mode,
            width,
            ..PctConfig::default()
        }
    }

    fn cfg_blocks(width: u32) -> PctConfig {
        // hbar with the legacy `▓`/`░` glyphs — exercises the override path.
        PctConfig {
            mode: PctMode::Hbar,
            width,
            filled: "▓".into(),
            empty: "░".into(),
        }
    }

    // ── percent ──

    #[test]
    fn percent_basic() {
        assert_eq!(render_plain(0, &cfg(PctMode::Percent)), "0%");
        assert_eq!(render_plain(50, &cfg(PctMode::Percent)), "50%");
        assert_eq!(render_plain(100, &cfg(PctMode::Percent)), "100%");
    }

    // ── float ──

    #[test]
    fn float_basic() {
        assert_eq!(render_plain(0, &cfg(PctMode::Float)), "0.00");
        assert_eq!(render_plain(50, &cfg(PctMode::Float)), "0.50");
        assert_eq!(render_plain(100, &cfg(PctMode::Float)), "1.00");
    }

    #[test]
    fn float_boundaries() {
        assert_eq!(render_plain(0, &cfg(PctMode::Float)), "0.00");
        assert_eq!(render_plain(1, &cfg(PctMode::Float)), "0.01");
        assert_eq!(render_plain(99, &cfg(PctMode::Float)), "0.99");
        assert_eq!(render_plain(100, &cfg(PctMode::Float)), "1.00");
    }

    // ── dots ──

    #[test]
    fn dots_basic() {
        assert_eq!(render_plain(0, &cfg(PctMode::Dots)), "⠀");
        assert_eq!(render_plain(50, &cfg(PctMode::Dots)), "⡇"); // 4/8
        assert_eq!(render_plain(100, &cfg(PctMode::Dots)), "⣿");
    }

    #[test]
    fn dots_nine_step_boundaries() {
        // pct ≈ round(k/8 * 100) for k in 0..=8
        // k:   0   1   2   3   4   5   6   7   8
        // pct: 0  12  25  38  50  62  75  88  100
        let cases = [
            (0u32, "⠀"),
            (12, "⡀"),
            (25, "⡄"),
            (38, "⡆"),
            (50, "⡇"),
            (62, "⣇"),
            (75, "⣧"),
            (88, "⣷"),
            (100, "⣿"),
        ];
        for (pct, want) in cases {
            assert_eq!(render_plain(pct, &cfg(PctMode::Dots)), want, "pct={pct}");
        }
    }

    // ── shaded ──

    #[test]
    fn shaded_basic() {
        assert_eq!(render_plain(0, &cfg(PctMode::Shaded)), " ");
        assert_eq!(render_plain(50, &cfg(PctMode::Shaded)), "▒"); // 2/4
        assert_eq!(render_plain(100, &cfg(PctMode::Shaded)), "█");
    }

    #[test]
    fn shaded_five_step_boundaries() {
        // k:   0   1   2   3   4
        // pct: 0  25  50  75  100
        let cases = [(0u32, " "), (25, "░"), (50, "▒"), (75, "▓"), (100, "█")];
        for (pct, want) in cases {
            assert_eq!(render_plain(pct, &cfg(PctMode::Shaded)), want, "pct={pct}");
        }
    }

    // ── vbar ──

    #[test]
    fn vbar_basic() {
        assert_eq!(render_plain(0, &cfg(PctMode::Vbar)), " ");
        assert_eq!(render_plain(50, &cfg(PctMode::Vbar)), "▄"); // 4/8
        assert_eq!(render_plain(100, &cfg(PctMode::Vbar)), "█");
    }

    #[test]
    fn vbar_eight_step_boundaries() {
        // k:   0   1   2   3   4   5   6   7   8
        // pct: 0  12  25  38  50  62  75  88  100
        let cases = [
            (0u32, " "),
            (12, "▁"),
            (25, "▂"),
            (38, "▃"),
            (50, "▄"),
            (62, "▅"),
            (75, "▆"),
            (88, "▇"),
            (100, "█"),
        ];
        for (pct, want) in cases {
            assert_eq!(render_plain(pct, &cfg(PctMode::Vbar)), want, "pct={pct}");
        }
    }

    // ── hbar (sub-cell + legacy override) ──

    #[test]
    fn hbar_basic_width_4() {
        let c = cfg_w(PctMode::Hbar, 4);
        assert_eq!(render_plain(0, &c), "    ");
        // 50% of 4 cells = 2 full cells, 2 empty (default empty = ' ').
        assert_eq!(render_plain(50, &c), "██  ");
        assert_eq!(render_plain(100, &c), "████");
    }

    #[test]
    fn hbar_partial_glyph_at_each_eighth_width_1() {
        // With width=1, partial = (pct*8/100). Pick percentages that floor
        // exactly to k/8 for k in 1..=7.
        // pct=13: 13*8/100=1 -> ▏
        // pct=25: 25*8/100=2 -> ▎
        // pct=38: 38*8/100=3 -> ▍
        // pct=50: 50*8/100=4 -> ▌
        // pct=63: 63*8/100=5 -> ▋
        // pct=75: 75*8/100=6 -> ▊
        // pct=88: 88*8/100=7 -> ▉
        let c = cfg_w(PctMode::Hbar, 1);
        assert_eq!(render_plain(13, &c), "▏");
        assert_eq!(render_plain(25, &c), "▎");
        assert_eq!(render_plain(38, &c), "▍");
        assert_eq!(render_plain(50, &c), "▌");
        assert_eq!(render_plain(63, &c), "▋");
        assert_eq!(render_plain(75, &c), "▊");
        assert_eq!(render_plain(88, &c), "▉");
        // 100% snaps to a single full block.
        assert_eq!(render_plain(100, &c), "█");
    }

    #[test]
    fn hbar_legacy_blocks_glyphs() {
        // Override `filled`/`empty` to the legacy `▓`/`░` look — exercises the
        // back-compat path for users with existing `[ctx_bar]` configs.
        let c = cfg_blocks(10);
        assert_eq!(render_plain(0, &c), "░░░░░░░░░░");
        // 50% of 10 = 5 full cells, no partial, 5 empty cells.
        assert_eq!(render_plain(50, &c), "▓▓▓▓▓░░░░░");
        assert_eq!(render_plain(100, &c), "▓▓▓▓▓▓▓▓▓▓");
    }

    // ── clamping & zero ──

    #[test]
    fn clamp_above_100() {
        let c = cfg_blocks(4);
        assert_eq!(render_plain(150, &c), "▓▓▓▓");
        assert_eq!(render_plain(101, &cfg(PctMode::Percent)), "100%");
        assert_eq!(render_plain(200, &cfg(PctMode::Float)), "1.00");
        assert_eq!(render_plain(999, &cfg(PctMode::Dots)), "⣿");
        assert_eq!(render_plain(101, &cfg(PctMode::Shaded)), "█");
        assert_eq!(render_plain(500, &cfg(PctMode::Vbar)), "█");
    }

    #[test]
    fn zero_renders_all_empty_blocks() {
        let c = cfg_blocks(5);
        assert_eq!(render_plain(0, &c), "░░░░░");
    }

    #[test]
    fn zero_renders_all_empty_hbar() {
        let c = cfg_w(PctMode::Hbar, 5);
        assert_eq!(render_plain(0, &c), "     ");
    }

    // ── color wrapping ──

    #[test]
    fn render_wraps_with_color_codes() {
        let s = render(50, &cfg(PctMode::Percent));
        assert!(s.contains("50%"));
        assert!(s.contains(RESET));
    }

    #[test]
    fn bar_color_thresholds() {
        assert_eq!(bar_color(0), DIM);
        assert_eq!(bar_color(49), DIM);
        assert_eq!(bar_color(50), FG_YELLOW);
        assert_eq!(bar_color(79), FG_YELLOW);
        assert_eq!(bar_color(80), FG_RED);
        assert_eq!(bar_color(100), FG_RED);
    }
}
