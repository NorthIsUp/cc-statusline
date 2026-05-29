// Library crate — exposes the statusline's modules so external binaries (e.g.
// `gen_schema`) can reach the config types. The main `cc-statusline` binary in
// `src/main.rs` continues to live as a binary target with its own module tree;
// this lib mirrors the same module set so both compile from the same sources.

#![allow(
    dead_code,
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::unnecessary_cast,
    clippy::manual_range_contains
)]

pub mod cache;
pub mod component;
pub mod components;
pub mod config;
pub mod focus;
pub mod git;
pub mod github;
pub mod glyphs;
pub mod input;
pub mod layout;
pub mod pct;
pub mod quota;
pub mod recent_prs;
pub mod refresh;
pub mod render;
pub mod state;
pub mod transcript;
pub mod vlen;
