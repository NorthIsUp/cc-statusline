// Rate-limit quota formatting. Pace-aware coloring: compares usage % to elapsed
// % of the window. On pace → dim, over pace → yellow, way over → red.

use crate::cache::now_epoch;
use crate::glyphs::{DIM, FG_RED, FG_YELLOW, RESET};
use crate::pct::{self, PctConfig};

pub const WIN_5H: i64 = 18_000;
pub const WIN_7D: i64 = 604_800;

pub fn pct_color(p: u32, reset: Option<i64>, window: i64) -> &'static str {
    if p >= 100 {
        return FG_RED;
    }
    let reset = match reset {
        Some(r) => r,
        None => {
            return if p >= 80 {
                FG_RED
            } else if p >= 50 {
                FG_YELLOW
            } else {
                DIM
            };
        }
    };
    let now = now_epoch();
    let mut remaining = (reset - now).max(0);
    if remaining > window {
        remaining = window;
    }
    let elapsed = window - remaining;
    let elapsed_pct = (elapsed * 100 / window.max(1)) as i64;

    if elapsed_pct < 10 {
        return if p >= 50 {
            FG_RED
        } else if p >= 20 {
            FG_YELLOW
        } else {
            DIM
        };
    }

    let p100 = (p as i64) * 100;
    if p100 > elapsed_pct * 130 {
        FG_RED
    } else if p100 > elapsed_pct * 100 {
        FG_YELLOW
    } else {
        DIM
    }
}

pub fn reset_str(reset: i64) -> String {
    let now = now_epoch();
    let mut remaining = reset - now;
    if remaining <= 0 {
        return "soon".into();
    }
    let days = remaining / 86_400;
    if days >= 1 {
        let hrs = (remaining % 86_400) / 3_600;
        return if hrs > 0 {
            format!("{days}d{hrs}h")
        } else {
            format!("{days}d")
        };
    }
    // <24h: show clock time + duration remaining.
    let clock = std::process::Command::new("date")
        .args(["-r", &reset.to_string(), "+%l:%M%p"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().replace("AM", "a").replace("PM", "p"))
        .unwrap_or_default();
    remaining = (remaining + 59) / 60 * 60;
    let hrs = remaining / 3_600;
    let mins = (remaining % 3_600) / 60;
    let dur = if hrs > 0 && mins > 0 {
        format!("{hrs}h{mins}m")
    } else if hrs > 0 {
        format!("{hrs}h")
    } else {
        format!("{mins}m")
    };
    format!("{} {}", clock.trim_start(), dur)
}

/// Render a single quota window. The percent visual is delegated to
/// `pct::render_plain` using `pct_cfg`, so callers can swap between text
/// (`percent`/`float`) and bar visuals (`dots`/`shaded`/`hbar`/`vbar`). The
/// reset-time suffix is unconditional (when present) and lives outside the
/// mode-rendered glyph: `<glyph> <pct-rendered> (<reset>)`.
pub fn fmt_quota(
    p: Option<u32>,
    reset: Option<i64>,
    window: i64,
    label: &str,
    pct_cfg: &PctConfig,
) -> String {
    let p = match p {
        Some(p) => p,
        None => return String::new(),
    };
    let c = pct_color(p, reset, window);
    let body = pct::render_plain(p, pct_cfg);
    let mut out = format!("{c}{label} {body}");
    if p >= 80 {
        if let Some(r) = reset {
            out.push_str(&format!(" ({})", reset_str(r)));
        }
    }
    out.push_str(RESET);
    out
}
