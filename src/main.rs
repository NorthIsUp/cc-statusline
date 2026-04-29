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

mod cache;
mod component;
mod components;
mod config;
mod focus;
mod git;
mod glyphs;
mod input;
mod layout;
mod quota;
mod recent_prs;
mod refresh;
mod render;
mod state;
mod transcript;
mod vlen;

use std::io::{self, Read};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && args[1] == "--refresh-recent-prs" {
        recent_prs::run_refresh();
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
            _ => {}
        }
    }
    render_once();
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
