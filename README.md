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

## Layout templates

Customise segment order and content with Starship-style placeholders. Set
`left` and `right` in `config.toml`; when either is set, the built-in
hardcoded layout is replaced by your template.

```toml
left  = "${repo} ${branch}${pr_num} ${ci}${review}${comments} ${dirty}${ahead}${behind} ${ticket}"
right = "${burn} · ${agents} · ${quotas} ${ctx} ${loc} [${model:long} · ${effort:icon}${effort:short}] ${spinner}"

# When the rendered single line would exceed this width, push the right
# pane to line 2, right-aligned to the terminal width.
soft_wrap_cols = 160
```

Syntax:

- `${name}` resolves to the variable's default form.
- `${name:variant}` resolves to a named variant (falls back to default if
  the variant isn't defined).
- Empty variables collapse adjacent literal whitespace so optional segments
  don't leave stray spaces.

Variables: `repo`, `branch`, `pr_num`, `pr_icon`, `ci`, `review`,
`comments`, `dirty`, `ahead`, `behind`, `ticket`, `burn`, `agents`,
`quotas`, `ctx`, `loc`, `model` (`:long`, `:short`), `effort` (`:icon`,
`:short`), `spinner`, `chips` (`:compact`, `:expanded`).

## Releasing

1. Add a `## [X.Y.Z] - YYYY-MM-DD` section to `CHANGELOG.md`.
2. Bump `version` in `Cargo.toml`.
3. Commit and push to `main`.

The `release.yml` workflow watches Cargo.toml on main, reads the new version,
builds binaries for 5 targets, drops a `vX.Y.Z` tag, and publishes a GitHub
release with the matching CHANGELOG section as the body and SHA256 checksums.

## License

MIT
