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
pr_expand_min_cols  = 160           # cols to expand "×N" → "#N1 #N2 …"
pr_cache_ttl        = 60            # seconds, current-branch PR
other_cache_ttl     = 600           # seconds, "other PRs" URL list
recent_prs_ttl      = 20            # seconds, global gh api graphql cache
debug_focus_log     = true
spinner             = "compact"     # or "epoch-N" (last N digits of epoch)
```

State per session lives at `$XDG_CACHE_HOME/cc-statusbar/<session_id>.toml`.
A shared `recent_prs.toml` holds one GraphQL response that hydrates chip
colors across every session.

## Releasing

1. Add a `## [X.Y.Z] - YYYY-MM-DD` section to `CHANGELOG.md`.
2. Bump `version` in `Cargo.toml`.
3. Commit and push to `main`.

The `release.yml` workflow watches Cargo.toml on main, reads the new version,
builds binaries for 5 targets, drops a `vX.Y.Z` tag, and publishes a GitHub
release with the matching CHANGELOG section as the body and SHA256 checksums.

## License

MIT
