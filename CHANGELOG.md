# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.13] - 2026-04-30

### Fixed

- Layout autoresize treats `cfg.min` as a SOFT preference: when the bar
  still overflows after the normal shrink loop, a new relax pass shrinks
  pinned items below their `min` toward the component's intrinsic
  smallest size (lowest priority first) before any items are dropped.
  Previously, a config like `[chips] min = "xl"` with 50+ PRs at a narrow
  terminal width could drop chips entirely, leaving the bar blank. Chips
  now degrade to their compact `×N` form instead. (#22, closes #22)

## [0.1.12] - 2026-04-30

### Added

- `[chips] merged_summary` (default `true`) prepends a `<merged_glyph>×N`
  summary chip to the chips chain when the merge-age filter
  (`collapse_merged_after_hours`) collapses ≥1 merged PRs. Renders in the
  same merged-state color as individual merged chips and is intentionally
  not OSC-8 hyperlinked. Stack mode bypasses the filter, so the summary
  chip never appears there. Set to `false` to suppress. (Closes #20)

## [0.1.11] - 2026-04-29

### Added

- `[chips] collapse_merged_after_hours` (default `36`; `0` disables) drops
  merged PRs older than the cutoff from the chips chain. Reduces visual
  noise when the chain accumulates days-old merged PRs. The current
  branch's own PR is never filtered, and stack mode (gt) bypasses the
  filter entirely. (Closes #18)
- `PrEntry.merged_at` (Unix epoch seconds) added to the recent-PRs cache;
  the GraphQL query now requests `mergedAt` and parses ISO8601 inline.
  Cache is keyed to the binary version so the schema change triggers a
  one-shot refresh on first run after upgrade.

## [0.1.10] - 2026-04-29

### Added

- Dotted layout entries `quotas.hourly`, `quotas.weekly`, `quotas.design`,
  `quotas.sonnet`. Each addresses a single quota bucket so users can place
  buckets at distinct layout positions, e.g.
  `right = ["burn", "quotas.hourly", "ctx_bar", "quotas.weekly", "model"]`.
  The bare `quotas` entry continues to render every configured bucket
  joined with the existing `·` separator. Default priorities for the
  dotted entries are higher than the bare parent (25 / 20 / 18 / 17 vs 15)
  since explicitly-placed buckets are user-elevated.
- Per-bucket layout knobs: `[quotas.hourly] priority = 30` (and `min`,
  `required`, `default`, `sizes`) now flow through. The bucket sub-section
  is a `BucketConfig` carrying both the percent fields (`mode`/`width`/...)
  and the common `ComponentConfig` fields. (#16, closes #16)

### Changed

- `QuotasConfig.{hourly,weekly,design,sonnet}` field type changed from
  `Option<PctConfig>` to `Option<BucketConfig>`. The on-disk TOML format
  is unchanged — both `PctConfig` and `ComponentConfig` deserialize via
  `serde(flatten)` so existing `[quotas.hourly] mode = "hbar"` configs
  parse unchanged.

## [0.1.9] - 2026-04-29

### Added

- Per-bucket `[quotas]` configuration. Sub-sections under `[quotas]`,
  each a full `PctConfig` that overrides the parent defaults for that
  bucket only:
  - `[quotas.hourly]` — 5-hour bucket (`rate_limits.five_hour`)
  - `[quotas.weekly]` — 7-day bucket (`rate_limits.seven_day`)
  - `[quotas.design]` — placeholder for future design-partner quota
  - `[quotas.sonnet]` — placeholder for future sonnet model quota
  Top-level `[quotas]` keys (`mode`/`width`/`filled`/`empty`) act as defaults
  applied to every bucket; sub-sections override wholesale. Existing flat
  `[quotas] mode = "..."` configs continue to work unchanged.
  (#12, closes #12)
- New Nerd Font glyphs `DESIGN_Q` (`nf-md-palette`) and `SONNET_Q`
  (`nf-md-music_note`) reserved for the design and sonnet quota labels
  once the upstream session-JSON schema exposes those buckets.

### Notes

- The `design` and `sonnet` sub-sections are config plumbing only — the
  upstream session-JSON schema for those buckets is not yet documented,
  so the corresponding `Session` input fields are deliberately omitted.
  Follow-up work will capture a real session JSON to discover the paths.

## [0.1.8] - 2026-04-29

### Added

- JSON schema for `config.toml`, generated from the config structs via
  `schemars`. New `gen_schema` binary emits `config.schema.json` to stdout;
  the committed schema lives at the repo root and is checked in CI for
  drift. `config.example.toml` ships with a `#:schema` directive so editors
  with the Even Better TOML extension light up autocomplete and validation
  out of the box. (Closes #13)

### Changed

- The crate now exposes a library target (`cc_statusline`) alongside the
  existing `cc-statusline` binary so external binaries (currently
  `gen_schema`) can reach the config types. Installs are unchanged —
  `cargo install --path .` still produces `cc-statusline`.

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
