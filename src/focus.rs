// iTerm2-aware focus detection. Bypasses macOS frontmost so iTerm2 hotkey
// windows register correctly. State (focused + transition tracking) lives in
// the per-session TOML.

use crate::cache::now_epoch;
use crate::config;
use crate::input::Session;
use crate::state::State;

const ITERM_QUERY: &str = r#"
tell application "iTerm2"
  try
    set w to current window
    if w is missing value then return ""
    if not (visible of w) then return ""
    return unique ID of current session of w
  on error
    return ""
  end try
end tell
"#;

#[derive(Debug)]
pub enum Transition {
    None,
    BgToFg,
    FgToBg,
}

pub fn detect(session: &Session, st: &mut State) -> bool {
    let term = std::env::var("TERM_PROGRAM").unwrap_or_default();
    let iterm_id = std::env::var("ITERM_SESSION_ID").unwrap_or_default();
    let our_uuid = iterm_id
        .rsplit_once(':')
        .map(|(_, u)| u.to_string())
        .unwrap_or_default();

    let mut focused = false;
    let mut active_uuid = String::new();
    let mut frontmost = String::new();

    match term.as_str() {
        "iTerm.app" => {
            if !our_uuid.is_empty() {
                active_uuid = run_applescript(ITERM_QUERY).unwrap_or_default();
                focused = active_uuid == our_uuid;
            }
        }
        "vscode" => {
            frontmost = frontmost_app_name().unwrap_or_default();
            focused = matches!(
                frontmost.as_str(),
                "Code" | "Code - Insiders" | "Cursor" | "Windsurf"
            );
        }
        _ => {}
    }

    if focused && !iterm_id.is_empty() {
        let key = iterm_id.replace('/', "_");
        let tmp = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let marker = std::path::PathBuf::from(format!("{tmp}/iterm-tab-blue.{key}"));
        if marker.exists() {
            print!("\x1b]6;1;bg;*;default\x1b\\");
            let _ = std::fs::remove_file(&marker);
        }
    }

    let prev = st.focus.focused;
    st.focus.focused = focused;
    st.focus.updated_at = now_epoch();

    let transition = match (focused, prev) {
        (true, false) => Transition::BgToFg,
        (false, true) => Transition::FgToBg,
        _ => Transition::None,
    };

    if config::config().debug_focus_log() {
        log_line(
            session,
            focused,
            prev,
            &transition,
            &term,
            &frontmost,
            &our_uuid,
            &active_uuid,
        );
    }
    focused
}

/// Frontmost application's name via `NSWorkspace` — no AppleScript, no TCC
/// automation permission, no subprocess. Replaces the System Events
/// `osascript` query in the vscode branch.
fn frontmost_app_name() -> Option<String> {
    use objc2_app_kit::NSWorkspace;
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    app.localizedName().map(|n| n.to_string())
}

/// Run an AppleScript in-process via `NSAppleScript`, returning its string
/// result. Replaces `osascript -e <script>` — same Apple-event IPC, but no
/// fork/exec/Gatekeeper of an `osascript` subprocess.
fn run_applescript(script: &str) -> Option<String> {
    use objc2::AnyThread;
    use objc2_foundation::{NSAppleScript, NSString};
    let source = NSString::from_str(script);
    let script = NSAppleScript::initWithSource(NSAppleScript::alloc(), &source)?;
    let mut err = None;
    // SAFETY: `err` is the documented `Option<Retained<NSDictionary>>`
    // out-param type. Our scripts wrap their body in `try`, so this returns a
    // valid (possibly empty-string) descriptor rather than nil.
    let desc = unsafe { script.executeAndReturnError(Some(&mut err)) };
    if err.is_some() {
        return None;
    }
    desc.stringValue().map(|s| s.to_string())
}

#[allow(clippy::too_many_arguments)]
fn log_line(
    session: &Session,
    focused: bool,
    prev: bool,
    t: &Transition,
    term: &str,
    frontmost: &str,
    our: &str,
    active: &str,
) {
    let ts = format_local_time(now_epoch());
    let t_str = match t {
        Transition::BgToFg => "bg->fg",
        Transition::FgToBg => "fg->bg",
        Transition::None => "-",
    };
    let line = format!(
        "{ts} focused={f} prev={p} t={t_str} term={term} frontmost={fm} our_uuid={our} active_uuid={active} session={sid}\n",
        f = focused as u8,
        p = prev as u8,
        fm = if frontmost.is_empty() { "-" } else { frontmost },
        our = if our.is_empty() { "-" } else { our },
        active = if active.is_empty() { "-" } else { active },
        sid = session.session_id,
    );
    let path = format!("/tmp/cc-focus-{}.log", session.session_id);
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

fn format_local_time(epoch: i64) -> String {
    match crate::cache::local_tm(epoch) {
        Some(tm) => format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
        ),
        None => epoch.to_string(),
    }
}
