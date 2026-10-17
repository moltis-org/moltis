//! Colours and fonts, following the system light/dark appearance.

use gpui::{App, Global, Hsla, SharedString, WindowAppearance, rgb};

use crate::markdown::highlight::TokenKind;

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub dark: bool,
    pub background: Hsla,
    pub sidebar: Hsla,
    pub surface: Hsla,
    pub surface_hover: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub accent: Hsla,
    pub accent_text: Hsla,
    pub user_bubble: Hsla,
    pub code_background: Hsla,
    pub danger: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub selection: Hsla,
    pub link: Hsla,
    pub ui_font: SharedString,
    pub mono_font: SharedString,
    syntax: [Hsla; 6],
}

impl Global for Theme {}

fn hsla(hex: u32) -> Hsla {
    rgb(hex).into()
}

impl Theme {
    #[must_use]
    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
        }
    }

    #[must_use]
    pub fn light() -> Self {
        Self {
            dark: false,
            background: hsla(0xffffff),
            sidebar: hsla(0xf4f4f5),
            surface: hsla(0xf4f4f5),
            surface_hover: hsla(0xe4e4e7),
            border: hsla(0xe4e4e7),
            text: hsla(0x18181b),
            text_muted: hsla(0x71717a),
            accent: hsla(0x4f46e5),
            accent_text: hsla(0xffffff),
            user_bubble: hsla(0xeef2ff),
            code_background: hsla(0xf6f8fa),
            danger: hsla(0xdc2626),
            success: hsla(0x16a34a),
            warning: hsla(0xd97706),
            selection: hsla(0xc7d2fe),
            link: hsla(0x4f46e5),
            ui_font: default_ui_font(),
            mono_font: default_mono_font(),
            syntax: [
                hsla(0x6a737d),
                hsla(0x0a7d32),
                hsla(0x005cc5),
                hsla(0xb3186f),
                hsla(0x6f42c1),
                hsla(0x9a5b00),
            ],
        }
    }

    #[must_use]
    pub fn dark() -> Self {
        Self {
            dark: true,
            background: hsla(0x18181b),
            sidebar: hsla(0x111113),
            surface: hsla(0x27272a),
            surface_hover: hsla(0x3f3f46),
            border: hsla(0x2e2e33),
            text: hsla(0xf4f4f5),
            text_muted: hsla(0xa1a1aa),
            accent: hsla(0x818cf8),
            accent_text: hsla(0x111113),
            user_bubble: hsla(0x272a3d),
            code_background: hsla(0x111113),
            danger: hsla(0xf87171),
            success: hsla(0x4ade80),
            warning: hsla(0xfbbf24),
            selection: hsla(0x3730a3),
            link: hsla(0xa5b4fc),
            ui_font: default_ui_font(),
            mono_font: default_mono_font(),
            syntax: [
                hsla(0x8b949e),
                hsla(0x7ee787),
                hsla(0x79c0ff),
                hsla(0xff7b72),
                hsla(0xd2a8ff),
                hsla(0xffa657),
            ],
        }
    }

    #[must_use]
    pub fn syntax(&self, kind: TokenKind) -> Hsla {
        let index = match kind {
            TokenKind::Comment => 0,
            TokenKind::String => 1,
            TokenKind::Number => 2,
            TokenKind::Keyword => 3,
            TokenKind::Function => 4,
            TokenKind::Type => 5,
        };
        self.syntax[index]
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }
}

fn default_ui_font() -> SharedString {
    if cfg!(target_os = "macos") {
        ".SystemUIFont".into()
    } else if cfg!(target_os = "windows") {
        "Segoe UI".into()
    } else {
        "Inter".into()
    }
}

fn default_mono_font() -> SharedString {
    if cfg!(target_os = "macos") {
        "Menlo".into()
    } else if cfg!(target_os = "windows") {
        "Consolas".into()
    } else {
        "DejaVu Sans Mono".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_selects_palette() {
        assert!(Theme::for_appearance(WindowAppearance::VibrantDark).dark);
        assert!(!Theme::for_appearance(WindowAppearance::Light).dark);
        let theme = Theme::light();
        assert_ne!(
            theme.syntax(TokenKind::Comment),
            theme.syntax(TokenKind::Keyword)
        );
        assert_ne!(Theme::light().text, Theme::dark().text);
    }
}
