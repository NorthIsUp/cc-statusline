// Render entry point: gather data into a `RenderCtx`, hand to the layout
// engine. All ad-hoc width/shrink/format logic now lives in `components.rs`
// (per-component) and `layout.rs` (autoresize). Nothing here knows about
// individual segments any more.

use crate::component::RenderCtx;
use crate::git::GitData;
use crate::input::Session;
use crate::state::State;
use crate::transcript::{AgentCount, BurnInfo, OtherPrs};

pub fn build(
    session: &Session,
    git: &GitData,
    other: &OtherPrs,
    burn: &BurnInfo,
    agents: &AgentCount,
    tick: u64,
) -> String {
    build_with_state(
        session,
        git,
        other,
        burn,
        agents,
        tick,
        &mut State::default(),
    )
}

/// Variant that accepts a mutable State so the layout engine's hysteresis
/// state can be persisted. Called from main render path; the no-state form
/// is kept for the read-only fallback in main.
pub fn build_with_state(
    session: &Session,
    git: &GitData,
    other: &OtherPrs,
    burn: &BurnInfo,
    agents: &AgentCount,
    tick: u64,
    state: &mut State,
) -> String {
    let cols = effective_cols(session.cols);
    let ctx = RenderCtx {
        session,
        git,
        other,
        burn,
        agents,
        tick,
    };
    crate::layout::render(&ctx, state, cols)
}

pub fn effective_cols(input: u32) -> u32 {
    let mut cols = if input > 0 { input } else { tty_cols() };
    let margin: u32 = std::env::var("CC_STATUSLINE_SAFETY_MARGIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| crate::config::config().safety_margin_or(0));
    cols = cols.saturating_sub(margin);
    cols.max(20)
}

fn tty_cols() -> u32 {
    use std::process::{Command, Stdio};
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
