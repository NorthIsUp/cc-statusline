// Claude Code statusline — Rust port.
//
// Layout:
//   Wide  (1 line):  [branch #PR]  CI  review  💬N  [TICKET] (N prs)                bar 47% [Opus]
//   Narrow (2 lines): [branch #PR]  CI  review  💬N  [TICKET] (N prs)
//                                                         bar 47% [Opus]
//
// Reads session JSON on stdin. Per-session state at
// $XDG_CACHE_HOME/cc-statusbar/<id>.toml; user config at
// $XDG_CONFIG_HOME/cc-statusbar/config.toml.
//
// Subcommands (used by the foreground render to fan out async refreshes):
//   cc-statusline --refresh-pr    <session_id>
//   cc-statusline --refresh-other <session_id>

// Allow some clippy lints that fight with serde-default fields and the
// stylistic preferences of this codebase. Real bugs (unwraps, dead branches,
// shadowed bindings) still surface; we just don't want CI failing on
// "field is never read" for serde-driven structs whose fields are the public
// schema, or on `if A { if B { ... } }` patterns that read more naturally
// than `if A && B`.
#![allow(
    dead_code,
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::unnecessary_cast,
    clippy::manual_range_contains
)]

use cc_statusline::{components, focus, git, input, recent_prs, refresh, render, state, transcript};
use std::io::{self, Read};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && args[1] == "--refresh-recent-prs" {
        recent_prs::run_refresh();
        return;
    }
    if args.len() >= 2 && args[1] == "--preview" {
        preview_all();
        return;
    }
    if args.len() >= 3 {
        match args[1].as_str() {
            "--refresh-pr" => {
                refresh::run_refresh_pr(&args[2]);
                return;
            }
            "--refresh-other" => {
                refresh::run_refresh_other(&args[2]);
                return;
            }
            "--refresh-stack" => {
                refresh::run_refresh_stack(&args[2]);
                return;
            }
            _ => {}
        }
    }
    render_once();
}

/// Print every registered component at every size it offers, using live
/// data gathered from the current working directory (and whatever session
/// JSON the caller pipes in on stdin, if any). Useful for visually
/// auditing how each component shrinks across `xs → xl`.
fn preview_all() {
    use cc_statusline::component::RenderCtx;
    use std::io::IsTerminal;

    // If stdin has JSON, parse it; otherwise synthesize a minimal session
    // rooted at the cwd. We intentionally keep both paths going through the
    // same `Session::parse` so output reflects production behavior.
    let mut buf = String::new();
    if !io::stdin().is_terminal() {
        let _ = io::stdin().read_to_string(&mut buf);
    }
    if buf.trim().is_empty() {
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        buf = format!(
            r#"{{"session_id":"preview","model":{{"display_name":"Opus 4.7"}},"workspace":{{"current_dir":"{cwd}"}},"transcript_path":""}}"#
        );
    }
    let session = input::Session::parse(&buf);
    let st = state::State::load(&session.session_id);
    let git = git::view(&session, &st);
    let other = transcript::other_prs_view(&st, &git.origin_url);
    let burn = transcript::BurnInfo::default();
    let agents = transcript::AgentCount::default();
    let ctx = RenderCtx {
        session: &session,
        git: &git,
        other: &other,
        burn: &burn,
        agents: &agents,
        tick: 0,
    };

    let name_w = components::ALL_NAMES
        .iter()
        .map(|n| n.len())
        .max()
        .unwrap_or(10);

    println!("\x1b[1mcc-statusline component preview\x1b[0m  ({} components)", components::ALL_NAMES.len());
    println!();
    for &name in components::ALL_NAMES {
        let sizes = components::sizes_for(name).unwrap_or(&[]);
        let default = components::default_size_for(name);
        for &size in sizes {
            let r = components::render_named(name, size, &ctx).unwrap_or_default();
            let marker = if Some(size) == default { "*" } else { " " };
            let body = if r.text.is_empty() {
                "\x1b[2m(empty)\x1b[0m".to_string()
            } else {
                r.text.clone()
            };
            println!(
                "{name:<nw$}  {marker}{sz:<2}  {w:>3}w  {body}",
                name = name,
                marker = marker,
                sz = size.as_str(),
                w = r.width,
                body = body,
                nw = name_w,
            );
        }
        println!();
    }
    println!("\x1b[2m* = default size · widths are visible-column counts\x1b[0m");
}

fn render_once() {
    let mut buf = String::new();
    if io::stdin().read_to_string(&mut buf).is_err() {
        std::process::exit(0);
    }
    let session = input::Session::parse(&buf);

    // Hold an OS lock on the state file for the duration of the render so we
    // don't see a half-written file mid-update from a background refresher.
    let mut handle = match state::StateLock::acquire_blocking(&session.session_id) {
        Ok(h) => h,
        Err(_) => {
            // Lock acquisition failed (very rare). Fall back to lockless
            // read-only render so the user still gets *something*.
            let st = state::State::load(&session.session_id);
            return render_with_state(&session, st);
        }
    };

    // Spawn async refreshes for stale caches BEFORE rendering, so by the next
    // tick the freshest data is in place.
    refresh::maybe_spawn_pr(&session.session_id, &session.cwd, &handle.state);
    refresh::maybe_spawn_other(&session.session_id, &session.transcript, &handle.state);
    refresh::maybe_spawn_stack(&session.session_id, &session.cwd, &handle.state);
    recent_prs::maybe_spawn_refresh();

    handle.state.tick = handle.state.tick.wrapping_add(1);
    let _focused = focus::detect(&session, &mut handle.state);
    let git = git::view(&session, &handle.state);
    let other = transcript::other_prs_view(&handle.state, &git.origin_url);
    let burn = transcript::burn_rate(&session, &mut handle.state);
    let agents = transcript::agent_counter(&session, &mut handle.state);

    let line = render::build_with_state(
        &session,
        &git,
        &other,
        &burn,
        &agents,
        handle.state.tick,
        &mut handle.state,
    );
    println!("{line}");

    let _ = handle.save();
}

fn render_with_state(session: &input::Session, state: state::State) {
    let git = git::view(session, &state);
    let other = transcript::other_prs_view(&state, &git.origin_url);
    let burn = transcript::BurnInfo::default();
    let agents = transcript::AgentCount::default();
    let line = render::build(session, &git, &other, &burn, &agents, state.tick);
    println!("{line}");
}
