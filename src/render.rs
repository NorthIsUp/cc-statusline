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
    // Source priority — first that produces a usable number wins:
    //   1. /dev/tty winsize (cheap; rarely usable in Claude's hook
    //      subprocess but free to try).
    //   2. Ancestor pty: walk the parent chain via `proc_pidinfo` and ioctl
    //      the first ancestor with a real controlling tty — works in
    //      Claude's hook subprocess and is live on resize.
    //   3. JSON `terminal.width` from Claude Code (not currently sent;
    //      future-proofing).
    //   4. $COLUMNS / 120 fallback.
    //
    // All width probing is now pure syscall (ioctl TIOCGWINSZ +
    // proc_pidinfo) — no `ps`/`stty`/`tput` subprocesses on the render path.
    let mut cols = winsize_cols("/dev/tty")
        .or_else(ancestor_tty_cols)
        .or_else(|| (input > 0).then_some(input))
        .or_else(env_cols)
        .unwrap_or(120);
    let margin: u32 = std::env::var("CC_STATUSLINE_SAFETY_MARGIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| crate::config::config().safety_margin_or(0));
    cols = cols.saturating_sub(margin);
    cols.max(20)
}

/// `(ppid, controlling-tty dev_t)` for `pid` via `proc_pidinfo`'s
/// `PROC_PIDTBSDINFO` flavor. `e_tdev` is `NODEV` (-1) when the process has
/// no controlling terminal.
fn proc_info(pid: u32) -> Option<(u32, libc::dev_t)> {
    let mut bi: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let sz = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let n = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut bi as *mut _ as *mut libc::c_void,
            sz,
        )
    };
    if n != sz {
        return None;
    }
    Some((bi.pbi_ppid, bi.e_tdev as libc::dev_t))
}

/// Walk the parent-PID chain (us → claude → shell → terminal app) looking
/// for an ancestor with a real controlling tty, then read its winsize.
fn ancestor_tty_cols() -> Option<u32> {
    let mut pid = std::process::id();
    for _ in 0..16 {
        let (ppid, tdev) = proc_info(pid)?;
        if tdev != -1 {
            if let Some(c) = winsize_for_dev(tdev) {
                return Some(c);
            }
        }
        if ppid <= 1 {
            break;
        }
        pid = ppid;
    }
    None
}

/// Map a controlling-tty `dev_t` to `/dev/<name>` (via `devname`) and read
/// its winsize.
fn winsize_for_dev(dev: libc::dev_t) -> Option<u32> {
    let name = unsafe { libc::devname(dev, libc::S_IFCHR as libc::mode_t) };
    if name.is_null() {
        return None;
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(name) };
    let path = format!("/dev/{}", cstr.to_str().ok()?);
    winsize_cols(&path)
}

/// `ioctl(TIOCGWINSZ)` on `path` → column count. Replaces `stty size`:
/// opening the specific tty path queries that tty's geometry directly, even
/// when `/dev/tty` is detached in a hook subprocess.
fn winsize_cols(path: &str) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let f = std::fs::OpenOptions::new().read(true).open(path).ok()?;
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::ioctl(f.as_raw_fd(), libc::TIOCGWINSZ, &mut ws) };
    if rc != 0 || ws.ws_col == 0 {
        return None;
    }
    Some(ws.ws_col as u32)
}

fn env_cols() -> Option<u32> {
    std::env::var("COLUMNS").ok()?.parse().ok()
}
