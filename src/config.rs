// Global config: $XDG_CONFIG_HOME/cc-statusbar/config.toml.
// Falls back to ~/.config/cc-statusbar/config.toml. Env vars override file.

use crate::component::ComponentConfig;
use crate::components::{BurnConfig, ChipsConfig, CtxBarConfig, QuotasConfig};
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LayoutConfig {
    pub left: Vec<String>,
    pub right: Vec<String>,
    pub gap: u32,
    pub autoresize: bool,
    pub hysteresis_band: u32,
    /// When `chips` collapses to its compact `×N` form, render the expanded
    /// `#a #b #c …` chain on a second row instead of dropping it.
    pub overflow_chips_to_second_row: bool,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            left: default_left(),
            right: default_right(),
            gap: 2,
            autoresize: true,
            hysteresis_band: 2,
            overflow_chips_to_second_row: true,
        }
    }
}

/// Default left pane order — reproduces the legacy hardcoded layout.
pub fn default_left() -> Vec<String> {
    [
        "repo", "pr_icon", "branch", "pr_num", "ci", "review", "comments", "dirty", "ahead",
        "behind", "ticket", "chips",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Default right pane order.
pub fn default_right() -> Vec<String> {
    [
        "burn", "agents", "quotas", "ctx_bar", "loc", "model", "effort", "spinner",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[derive(Debug, Default, Clone, Deserialize, JsonSchema)]
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

    /// `[layout]` block — left/right component lists, gap, autoresize, etc.
    #[serde(default)]
    pub layout: Option<LayoutConfig>,

    /// Per-component common config blocks. Each component's `[name]` block
    /// is parsed into a `ComponentConfig` (priority/min/sizes/required) plus
    /// component-specific extension fields handled separately.
    #[serde(default)]
    pub repo: Option<ComponentConfig>,
    #[serde(default)]
    pub branch: Option<ComponentConfig>,
    #[serde(default)]
    pub pr_icon: Option<ComponentConfig>,
    #[serde(default)]
    pub pr_num: Option<ComponentConfig>,
    #[serde(default)]
    pub ci: Option<ComponentConfig>,
    #[serde(default)]
    pub review: Option<ComponentConfig>,
    #[serde(default)]
    pub comments: Option<ComponentConfig>,
    #[serde(default)]
    pub dirty: Option<ComponentConfig>,
    #[serde(default)]
    pub ahead: Option<ComponentConfig>,
    #[serde(default)]
    pub behind: Option<ComponentConfig>,
    #[serde(default)]
    pub ticket: Option<ComponentConfig>,
    #[serde(default)]
    pub chips: ChipsConfig,
    #[serde(default)]
    pub burn: BurnConfig,
    #[serde(default)]
    pub agents: Option<ComponentConfig>,
    #[serde(default)]
    pub quotas: QuotasConfig,
    #[serde(default)]
    pub ctx_bar: CtxBarConfig,
    #[serde(default)]
    pub loc: Option<ComponentConfig>,
    #[serde(default)]
    pub model: Option<ComponentConfig>,
    #[serde(default)]
    pub effort: Option<ComponentConfig>,
    #[serde(default)]
    pub spinner_cfg: Option<ComponentConfig>,
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
        self.pr_cache_ttl.unwrap_or(60)
    }
    pub fn other_cache_ttl(&self) -> i64 {
        self.other_cache_ttl.unwrap_or(600)
    }
    pub fn stack_refresh_ttl(&self) -> i64 {
        self.chips.stack_refresh_ttl
    }
    pub fn recent_prs_ttl(&self) -> i64 {
        self.recent_prs_ttl.unwrap_or(20)
    }
    pub fn debug_focus_log(&self) -> bool {
        self.debug_focus_log.unwrap_or(true)
    }
    pub fn spinner(&self) -> String {
        self.spinner.clone().unwrap_or_else(|| "compact".into())
    }
    pub fn layout(&self) -> LayoutConfig {
        self.layout.clone().unwrap_or_default()
    }

    /// Per-component common config. Returns the user-configured block or a
    /// default if absent.
    pub fn component_config(&self, name: &str) -> ComponentConfig {
        let pick = |o: &Option<ComponentConfig>| o.clone().unwrap_or_default();
        match name {
            "repo" => pick(&self.repo),
            "branch" => pick(&self.branch),
            "pr_icon" => pick(&self.pr_icon),
            "pr_num" => pick(&self.pr_num),
            "ci" => pick(&self.ci),
            "review" => pick(&self.review),
            "comments" => pick(&self.comments),
            "dirty" => pick(&self.dirty),
            "ahead" => pick(&self.ahead),
            "behind" => pick(&self.behind),
            "ticket" => pick(&self.ticket),
            "chips" => self.chips.common.clone(),
            "burn" => self.burn.common.clone(),
            "agents" => pick(&self.agents),
            "quotas" => self.quotas.common.clone(),
            n if crate::components::quotas_bucket_kind(n).is_some() => {
                // Per-bucket layout knobs (priority/min/required/etc.) come
                // from the `[quotas.<bucket>] common = ...` flattened fields
                // when set; otherwise inherit the parent `[quotas]` common.
                let bucket = crate::components::quotas_bucket_kind(n).unwrap();
                let q = &self.quotas;
                let bcfg = match bucket {
                    crate::components::BucketKind::Hourly => &q.hourly,
                    crate::components::BucketKind::Weekly => &q.weekly,
                    crate::components::BucketKind::Design => &q.design,
                    crate::components::BucketKind::Sonnet => &q.sonnet,
                };
                bcfg.as_ref()
                    .map(|b| b.common.clone())
                    .unwrap_or_else(|| q.common.clone())
            }
            "ctx_bar" => self.ctx_bar.common.clone(),
            "loc" => pick(&self.loc),
            "model" => pick(&self.model),
            "effort" => pick(&self.effort),
            "spinner" => pick(&self.spinner_cfg),
            _ => ComponentConfig::default(),
        }
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
