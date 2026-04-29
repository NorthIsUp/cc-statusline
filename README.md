# cc-statusline

Rust statusline for [Claude Code](https://claude.com/claude-code). Renders
branch, PR, CI, comments, dirty/ahead/behind, Linear ticket, "other PRs"
chips, token burn rate, subagent counter, rate-limit quotas, context bar,
location icon, model, output style, effort label.

## Install

Download a release binary from the
[releases page](https://github.com/NorthIsUp/cc-statusline/releases):

```sh
mkdir -p ~/.local/bin
curl -L -o ~/.local/bin/cc-statusline \
  https://github.com/NorthIsUp/cc-statusline/releases/latest/download/cc-statusline-aarch64-darwin
chmod +x ~/.local/bin/cc-statusline
```

Or build from source:

```sh
cargo install --path .
```

Wire into Claude Code by setting `statusLine.command` in `~/.claude/settings.json`:

```json
"statusLine": {
  "type": "command",
  "command": "~/.local/bin/cc-statusline",
  "padding": 0,
  "refreshInterval": 1
}
```

## Configure

Drop a config at `$XDG_CONFIG_HOME/cc-statusbar/config.toml` (or
`~/.config/cc-statusbar/config.toml`). Every field is optional.

```toml
linear_workspace    = "teamclara"   # used for [TICKET] hyperlinks
safety_margin       = 0             # cells subtracted from terminal width
nerd_font_width     = 2             # cell width of Nerd Font PUA glyphs
pr_cache_ttl        = 60            # seconds, current-branch PR
other_cache_ttl     = 600           # seconds, "other PRs" URL list
recent_prs_ttl      = 20            # seconds, global gh api graphql cache
debug_focus_log     = true
spinner             = "compact"     # or "epoch-N" (last N digits of epoch)
```

State per session lives at `$XDG_CACHE_HOME/cc-statusbar/<session_id>.toml`.
A shared `recent_prs.toml` holds one GraphQL response that hydrates chip
colors across every session.

## Layout

Every statusline element is a `Component` with declared size variants
(`xs`/`s`/`m`/`l`/`xl`) and a per-component config block. The layout
engine renders at default sizes, then iteratively shrinks
lowest-priority components — and finally drops them — until the line
fits the terminal width.

The `[layout]` block declares which components appear and in what order:

```toml
[layout]
left  = ["repo", "pr_icon", "branch", "pr_num", "ci", "review",
         "comments", "dirty", "ahead", "behind", "ticket", "chips"]
right = ["burn", "agents", "quotas", "ctx_bar", "loc", "model",
         "effort", "spinner"]
gap             = 2
autoresize      = true
hysteresis_band = 2
```

Each component accepts a `[name]` block with the common config:

```toml
[ctx_bar]
priority = 90        # higher = shrunk LAST
default  = "m"       # size before autoresize kicks in
min      = "s"       # never shrink past this size
sizes    = ["s", "m", "l"]   # whitelist allowed sizes
required = true              # never drop entirely
# component-specific knobs:
width  = 10
filled = "▓"
empty  = "░"
```

Sizes a component may render at, smallest to largest:

| Component | xs | s | m | l | xl |
|---|---|---|---|---|---|
| `ctx_bar` | one cell | `71%` | `▓▓▓░░░ 71%` | `⛁ ▓▓▓░░░ 71%` | `Σ ▓▓▓░░░ 71% / 200k tokens` |
| `repo` | _empty_ | basename | `org/repo` | — | `host/org/repo` |
| `branch` | _empty_ | truncated | full | — | full |
| `model` | initial | first word | first word | — | full name |
| `effort` | `⚡` | — | `⚡ M` | — | `⚡ Medium` |
| `chips` | _empty_ | `×N` | `×N` | — | `#1 #2 …` |
| `burn` | _empty_ | — | `Σ 12.3M` | — | `Σ 12.3M/hr` |

The other components (`pr_icon`, `pr_num`, `ci`, `review`, `comments`,
`dirty`, `ahead`, `behind`, `ticket`, `agents`, `quotas`, `loc`,
`spinner`) just render at `m` or omit themselves at `xs`.

Hysteresis: once a component is shrunk at terminal width W, it stays
shrunk while cols stays within ±`hysteresis_band` of W. Decisions
persist in `state.toml` so the line doesn't oscillate as you nudge-resize.

## Editor support

A JSON schema for `config.toml` lives at the repo root as
`config.schema.json`. Point editors at it for autocomplete and validation.

With the [Even Better TOML](https://marketplace.visualstudio.com/items?itemName=tamasfe.even-better-toml)
VS Code extension, add a `#:schema` directive to the top of your
`config.toml`:

```toml
#:schema https://raw.githubusercontent.com/NorthIsUp/cc-statusline/main/config.schema.json
```

To regenerate the schema after changing config structs:

```sh
cargo run --bin gen_schema > config.schema.json
```

CI diffs the committed schema against `gen_schema` output, so any
struct change must come with a regenerated `config.schema.json`.

## Releasing

1. Add a `## [X.Y.Z] - YYYY-MM-DD` section to `CHANGELOG.md`.
2. Bump `version` in `Cargo.toml`.
3. Commit and push to `main`.

The `release.yml` workflow watches Cargo.toml on main, reads the new version,
builds binaries for 5 targets, drops a `vX.Y.Z` tag, and publishes a GitHub
release with the matching CHANGELOG section as the body and SHA256 checksums.

## License

MIT
