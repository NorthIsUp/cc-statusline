# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.6] - 2026-04-29

### Added

- Unified percentage display modes (`PctMode`) shared by every percent-aware
  component. New `mode` key on `[ctx_bar]`, `[quotas]`, and `[burn]`:
  - `percent` (default) — `47%`
  - `float`             — `0.47`
  - `dots`              — single cell, 9 steps via braille `⠀⡀⡄⡆⡇⣇⣧⣷⣿`
  - `shaded`            — single cell, 5 steps ` ░▒▓█`
  - `hbar`              — multi-cell horizontal bar with sub-cell precision
                          via `▏▎▍▌▋▊▉█` (replaces the old fixed-glyph blocks
                          look — set `mode = "hbar"` to bring back the bar)
  - `vbar`              — single cell, 8 vertical levels `▁▂▃▄▅▆▇█`
  Color thresholds (red ≥80%, yellow ≥50%, dim otherwise) are unified in the
  shared `pct::bar_color` so every component uses the same breakpoints.
  (#10, closes #10)
- `[burn].max_tokens_per_hour` (default `5_000_000`) — the ceiling against
  which `tokens_per_hour` is mapped to a percent for the visual modes. The
  text modes (`percent`, `float`) keep the legacy `Σ <human>/hr` rendering.
- `[quotas]` is now a real config block (was unit). Quotas always renders
  `<glyph> <pct-via-mode> (<reset>)` — the reset-time suffix lives outside
  the mode-rendered glyph so users can switch the percent visual to
  `dots`/`hbar` without losing the reset clock.

### Changed

- `[ctx_bar]` no longer has dedicated `width`/`filled`/`empty` keys at the
  top level; they're now part of the shared `[pct]` knobs (and still parse
  identically — old configs work unchanged, treated as overrides for `hbar`
  mode glyphs). The default mode is `percent`, so existing renders that
  relied on `[ctx_bar]` defaults will now show `47%` instead of the bar.
  Set `mode = "hbar"` to restore the bar look.

## [0.1.5] - 2026-04-29

### Added

- Two-line overflow for the `chips` component: when the layout engine
  collapses chips to its compact `×N` form (because the expanded `#a #b
  #c …` chain wouldn't fit alongside the rest of the bar), the expanded
  chain is now rendered on a second row, left-aligned and right-padded
  to the terminal width. Line 1 is unchanged. Stack ordering is honored
  on line 2 when `is_gt`, and per-chip OSC-8 hyperlinks are preserved.
  Disabled with `overflow_chips_to_second_row = false` in the `[layout]`
  block. Suppressed when there are fewer than 2 chips. (#8, closes #8)

## [0.1.4] - 2026-04-29

### Added

- Render Graphite stacks in stack order in the chips component. When `gt` is
  on `$PATH` and `gt log --json` succeeds in `$CWD`, chips are reordered
  trunk → leaf, joined with a configurable separator (default `─•─`), and
  prefixed by a configurable leading glyph (default the Nerd Font branch
  glyph, dim cyan). PRs not reachable from the worktree's trunk are appended
  after the stacked chain, sorted by ascending PR number — identical to the
  legacy ordering — so non-stack workflows are unaffected. Detection runs
  on the same detached `--refresh-*` path used for PR/other refreshes; the
  foreground render never blocks on `gt`. Cached in the per-session state
  TOML with a configurable TTL (`[chips].stack_refresh_ttl`, default 60s)
  and a separate `locked_at` debounce to prevent thundering-herd `gt`
  invocations. New `[chips]` config keys: `stack_separator`, `stack_glyph`
  (set `""` to disable), `force_stack` (treat all sessions as gt-stacks),
  `stack_refresh_ttl`. (#2, closes #2)
- Composable component model. Every statusline element now implements a
  `Component` trait with declared size variants (`xs`/`s`/`m`/`l`/`xl`)
  and per-component config (`priority`, `min`, `sizes`, `required`). New
  `[layout]` block in `config.toml` declares the left/right component
  lists, gap, autoresize toggle, and hysteresis band. The layout engine
  renders at default sizes, then iteratively shrinks lowest-priority
  components and finally drops them until the line fits the terminal
  width. Hysteresis decisions persist across renders to prevent
  oscillation when nudge-resizing. (#5, closes #5)
- Per-component `[ctx_bar]` config: `width`, `filled`, `empty`, plus the
  shared `priority`/`min`/`sizes`/`required`/`default` knobs.

### Changed

- Default left/right layouts produce the same visual output as the
  previous hardcoded layout at width 200+. Existing screenshots unchanged.

### Removed

- The Starship-style template DSL (`${name:variant}` substitutions, `left`
  and `right` template strings, `soft_wrap_cols`) is replaced by the
  declarative `[layout]` config. The `template.rs` module is deleted.
- All ad-hoc width gates (`if cols >= 130/110/90`) and bespoke shrink
  helpers (`strip_repo_prefix`, `strip_right_optional`,
  `strip_chip_suffix`). The chip-attach try-loop is gone — chips is just
  another component with size variants.

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
