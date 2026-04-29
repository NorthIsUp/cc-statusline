// Global config: $XDG_CONFIG_HOME/cc-statusbar/config.toml.
// Falls back to ~/.config/cc-statusbar/config.toml. Env vars override file.

use serde::Deserialize;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub linear_workspace: Option<String>,
    pub safety_margin: Option<u32>,
    pub nerd_font_width: Option<u32>,
    pub pr_expand_min_cols: Option<u32>,
    pub pr_cache_ttl: Option<i64>,
    pub other_cache_ttl: Option<i64>,
    pub debug_focus_log: Option<bool>,
    pub spinner: Option<String>,
    pub recent_prs_ttl: Option<i64>,
}

impl Config {
    pub fn linear_workspace(&self) -> String {
        std::env::var("LINEAR_WORKSPACE")
            .ok()
            .or_else(|| self.linear_workspace.clone())
            .unwrap_or_else(|| "teamclara".into())
    }
    pub fn safety_margin(&self) -> u32 {
        self.safety_margin_or(0)
    }
    pub fn safety_margin_or(&self, default: u32) -> u32 {
        self.safety_margin.unwrap_or(default)
    }
    pub fn nerd_font_width(&self) -> u32 {
        std::env::var("CC_STATUSLINE_NF_WIDTH")
            .ok()
            .and_then(|v| v.parse().ok())
            .or(self.nerd_font_width)
            .filter(|w| *w == 1 || *w == 2)
            .unwrap_or(2)
    }
    pub fn pr_expand_min_cols(&self) -> u32 {
        std::env::var("CC_STATUSLINE_PR_EXPAND_MIN_COLS")
            .ok()
            .and_then(|v| v.parse().ok())
            .or(self.pr_expand_min_cols)
            .unwrap_or(160)
    }
    pub fn pr_cache_ttl(&self) -> i64 {
        // Was 10s. With 1Hz statusline + multiple sessions, that pace burns
        // through the 5000/hr GitHub API budget fast. 60s is still snappy
        // enough to see PR/CI changes within a minute.
        self.pr_cache_ttl.unwrap_or(60)
    }
    pub fn other_cache_ttl(&self) -> i64 {
        // 10 min. Other-PR chips are background context; refreshing every 2
        // min when there are many sessions × many chips × ~1 gh call each was
        // a primary source of the rate-limit hit.
        self.other_cache_ttl.unwrap_or(600)
    }
    /// TTL for the global "recent PRs by viewer" cache. One GraphQL call
    /// every `recent_prs_ttl` seconds, shared across all sessions.
    pub fn recent_prs_ttl(&self) -> i64 {
        self.recent_prs_ttl.unwrap_or(20)
    }
    pub fn debug_focus_log(&self) -> bool {
        self.debug_focus_log.unwrap_or(true)
    }
    pub fn spinner(&self) -> String {
        self.spinner.clone().unwrap_or_else(|| "compact".into())
    }
}

pub fn config_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join("cc-statusbar");
        }
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/cc-statusbar")
}

pub fn cache_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join("cc-statusbar");
        }
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache/cc-statusbar")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn state_path(session_id: &str) -> PathBuf {
    cache_dir().join(format!("{session_id}.toml"))
}

/// Shared, cross-session cache of recent PRs authored by the viewer. One
/// `gh api graphql` call hydrates state for every chip across every session,
/// instead of N per-URL lookups per session.
pub fn recent_prs_path() -> PathBuf {
    cache_dir().join("recent_prs.toml")
}

pub fn config() -> &'static Config {
    static CFG: OnceLock<Config> = OnceLock::new();
    CFG.get_or_init(|| {
        std::fs::read_to_string(config_path())
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    })
}
