// Component model — every statusline element implements `Component`.
//
// Each component declares the size variants it supports and renders into a
// `Rendered { text, width }`. The layout engine in `layout.rs` is responsible
// for picking sizes and placing components; nothing in here knows about the
// terminal width or other components.
//
// New components: implement `Component`, register in `layout::registry`.
// No ad-hoc width gating, no bespoke shrink helpers — declare more sizes.

use crate::git::GitData;
use crate::input::Session;
use crate::transcript::{AgentCount, BurnInfo, OtherPrs};
use schemars::JsonSchema;
use serde::Deserialize;
use std::str::FromStr;

/// Discrete size buckets a component can render at, smallest → largest.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, JsonSchema)]
#[schemars(rename_all = "lowercase")]
pub enum Size {
    Xs,
    S,
    #[default]
    M,
    L,
    Xl,
}

impl Size {
    pub fn as_str(self) -> &'static str {
        match self {
            Size::Xs => "xs",
            Size::S => "s",
            Size::M => "m",
            Size::L => "l",
            Size::Xl => "xl",
        }
    }
    /// Step one size smaller, or `None` if already at `Xs`.
    pub fn smaller(self) -> Option<Size> {
        match self {
            Size::Xl => Some(Size::L),
            Size::L => Some(Size::M),
            Size::M => Some(Size::S),
            Size::S => Some(Size::Xs),
            Size::Xs => None,
        }
    }
}

impl FromStr for Size {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "xs" => Ok(Size::Xs),
            "s" => Ok(Size::S),
            "m" => Ok(Size::M),
            "l" => Ok(Size::L),
            "xl" => Ok(Size::Xl),
            other => Err(format!("invalid size {other:?}")),
        }
    }
}

impl<'de> Deserialize<'de> for Size {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s: String = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Output of a single component render.
#[derive(Debug, Default, Clone)]
pub struct Rendered {
    pub text: String,
    pub width: u32,
}

impl Rendered {
    pub fn empty() -> Self {
        Self::default()
    }
    pub fn from_text(text: String) -> Self {
        let width = crate::vlen::vlen(&text);
        Self { text, width }
    }
}

/// All data a component might need, populated once per render.
#[derive(Debug)]
pub struct RenderCtx<'a> {
    pub session: &'a Session,
    pub git: &'a GitData,
    pub other: &'a OtherPrs,
    pub burn: &'a BurnInfo,
    pub agents: &'a AgentCount,
    pub tick: u64,
}

/// Common per-component config. Lives at the top of every `[name]` block.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ComponentConfig {
    /// Sizes the user has whitelisted. Empty = use the component's full set.
    pub sizes: Vec<Size>,
    /// Don't shrink past this size. None = component's smallest.
    pub min: Option<Size>,
    /// Higher = shrunk LAST when autoresizing. Default 5.
    pub priority: u32,
    /// If true, never drop entirely.
    pub required: bool,
    /// Override of the default size. None = component's `default_size()`.
    pub default: Option<Size>,
}

impl Default for ComponentConfig {
    fn default() -> Self {
        Self {
            sizes: Vec::new(),
            min: None,
            priority: 5,
            required: false,
            default: None,
        }
    }
}

/// The trait every renderable element implements.
pub trait Component {
    /// Per-component extension config. Use `()` if there are none.
    type Config: Default;

    fn name() -> &'static str
    where
        Self: Sized;
    fn sizes() -> &'static [Size]
    where
        Self: Sized;
    fn default_size() -> Size
    where
        Self: Sized;
    fn render(&self, size: Size, cfg: &Self::Config, ctx: &RenderCtx) -> Rendered;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_parses() {
        assert_eq!("xs".parse::<Size>().unwrap(), Size::Xs);
        assert_eq!("XL".parse::<Size>().unwrap(), Size::Xl);
        assert!("zz".parse::<Size>().is_err());
    }

    #[test]
    fn size_smaller_steps_down() {
        assert_eq!(Size::Xl.smaller(), Some(Size::L));
        assert_eq!(Size::S.smaller(), Some(Size::Xs));
        assert_eq!(Size::Xs.smaller(), None);
    }
}
