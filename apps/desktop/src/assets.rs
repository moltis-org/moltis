//! Icons compiled into the binary, served to gpui's `svg()` element.
//!
//! The SVGs come from the web UI's icon masks (`crates/web/src/assets/icons`)
//! so both clients share one icon language. They are drawn as alpha masks,
//! tinted with the element's text colour.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString, Styled, Svg, px, svg};

/// Every icon the app draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    ArrowUp,
    ChevronDown,
    ChevronRight,
    Clock,
    Globe,
    Monitor,
    Plus,
    Settings,
    Sparkles,
    Stop,
}

impl Icon {
    const ALL: [Self; 10] = [
        Self::ArrowUp,
        Self::ChevronDown,
        Self::ChevronRight,
        Self::Clock,
        Self::Globe,
        Self::Monitor,
        Self::Plus,
        Self::Settings,
        Self::Sparkles,
        Self::Stop,
    ];

    #[must_use]
    pub fn path(self) -> &'static str {
        match self {
            Self::ArrowUp => "icons/arrow-up.svg",
            Self::ChevronDown => "icons/chevron-down.svg",
            Self::ChevronRight => "icons/chevron-right.svg",
            Self::Clock => "icons/clock.svg",
            Self::Globe => "icons/globe.svg",
            Self::Monitor => "icons/monitor.svg",
            Self::Plus => "icons/plus.svg",
            Self::Settings => "icons/settings.svg",
            Self::Sparkles => "icons/sparkles.svg",
            Self::Stop => "icons/stop.svg",
        }
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Self::ArrowUp => include_bytes!("../assets/icons/arrow-up.svg"),
            Self::ChevronDown => include_bytes!("../assets/icons/chevron-down.svg"),
            Self::ChevronRight => include_bytes!("../assets/icons/chevron-right.svg"),
            Self::Clock => include_bytes!("../assets/icons/clock.svg"),
            Self::Globe => include_bytes!("../assets/icons/globe.svg"),
            Self::Monitor => include_bytes!("../assets/icons/monitor.svg"),
            Self::Plus => include_bytes!("../assets/icons/plus.svg"),
            Self::Settings => include_bytes!("../assets/icons/settings.svg"),
            Self::Sparkles => include_bytes!("../assets/icons/sparkles.svg"),
            Self::Stop => include_bytes!("../assets/icons/stop.svg"),
        }
    }
}

/// `icon` at `size` pixels. Give it a `.text_color(...)`: an `svg()` does not
/// inherit its parent's colour.
#[must_use]
pub fn icon(icon: Icon, size: f32) -> Svg {
    svg().path(icon.path()).size(px(size)).flex_none()
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(Icon::ALL
            .into_iter()
            .find(|icon| icon.path() == path)
            .map(|icon| Cow::Borrowed(icon.bytes())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Icon::ALL
            .into_iter()
            .map(Icon::path)
            .filter(|candidate| candidate.starts_with(path))
            .map(SharedString::from)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn every_icon_loads_as_svg() {
        for icon in Icon::ALL {
            let bytes = Assets.load(icon.path()).unwrap().unwrap();
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(text.trim_start().starts_with("<svg"), "{icon:?}");
        }
        assert!(Assets.load("icons/missing.svg").unwrap().is_none());
        assert_eq!(Assets.list("icons/").unwrap().len(), Icon::ALL.len());
    }
}
