// iTerm2-aware focus detection. Bypasses macOS frontmost so iTerm2 hotkey
// windows register correctly. State (focused + transition tracking) lives in
// the per-session TOML.

use crate::cache::now_epoch;
use crate::config;
use crate::input::Session;
use crate::state::State;
use std::process::{Command, Stdio};

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
                active_uuid = osascript(ITERM_QUERY).unwrap_or_default();
                focused = active_uuid == our_uuid;
            }
        }
        "vscode" => {
            frontmost = osascript(
                r#"tell application "System Events" to return name of first application process whose frontmost is true"#,
            )
            .unwrap_or_default();
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

fn osascript(script: &str) -> Option<String> {
    let out = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok().map(|s| s.trim().into())
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
    if let Ok(out) = Command::new("date")
        .args(["-r", &epoch.to_string(), "+%Y-%m-%dT%H:%M:%S"])
        .output()
    {
        if out.status.success() {
            return String::from_utf8(out.stdout)
                .unwrap_or_default()
                .trim()
                .into();
        }
    }
    epoch.to_string()
}
