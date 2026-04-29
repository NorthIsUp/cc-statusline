# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1] - 2026-04-28

### Changed

- Verify the tag-on-version-bump → release pipeline end-to-end.

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
