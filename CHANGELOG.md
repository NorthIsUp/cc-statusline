# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Configurable layout templates (Starship-style). Set `left` / `right` in
  `config.toml` with `${name}` / `${name:variant}` placeholders to customise
  segment order and content. Empty variables collapse adjacent literal
  whitespace so optional segments don't leave stray spaces. New
  `soft_wrap_cols` config (default `160`) pushes the right pane onto a
  second line, right-aligned, when the rendered width would exceed the
  threshold. Default behaviour unchanged when `left`/`right` are unset.
  (#1)

## [0.1.3] - 2026-04-28

### Added

- Auto-clear per-session and shared recent_prs caches when the binary version
  changes. Free schema migration on every Cargo.toml bump.

## [0.1.2] - 2026-04-28

### Changed

- Consolidate tag + release into one workflow (the GITHUB_TOKEN-pushed-tag
  trigger problem).

## [0.1.0] - 2026-04-28

### Added

- Initial release. Rust port of the bash `cc-statusline` for Claude Code.
- Single-TOML per-session state at `$XDG_CACHE_HOME/cc-statusbar/<id>.toml`.
- User config at `$XDG_CONFIG_HOME/cc-statusbar/config.toml` (linear workspace,
  Nerd Font width, spinner style, cache TTLs, focus log toggle).
- Async refresh subcommands (`--refresh-pr`, `--refresh-other`,
  `--refresh-recent-prs`) so renders never block on `gh`.
- Shared `recent_prs.toml` cache fed by a single `gh api graphql` query —
  hydrates state for every chip across every session in one call.
- iTerm2-aware focus detection that handles hotkey-window overlays.
- Same-origin filter for "other PRs" chips so cross-repo URLs (e.g. upstream
  PRs from WebSearch results) don't bleed in.
- GitHub-tuned PR-state palette and one-unit colored PR pill.
- `cargo test` covering ticket extraction and right-edge layout invariant.
