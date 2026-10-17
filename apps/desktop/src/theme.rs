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
    /// The selected sidebar row and the user's message.
    pub selected: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    /// Section labels, placeholders, secondary metadata.
    pub text_faint: Hsla,
    /// Primary buttons and the send button: near-black, near-white in dark.
    pub accent: Hsla,
    pub accent_text: Hsla,
    /// Moltis's own mark: a reply in progress, the brand glyph.
    pub brand: Hsla,
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
            background: hsla(0xfaf9f6),
            sidebar: hsla(0xf3f1ec),
            surface: hsla(0xffffff),
            surface_hover: hsla(0xebe8e1),
            selected: hsla(0xe6e2d9),
            border: hsla(0xe5e1d8),
            text: hsla(0x1f1e1b),
            text_muted: hsla(0x77736a),
            text_faint: hsla(0xa8a399),
            accent: hsla(0x1f1e1b),
            accent_text: hsla(0xffffff),
            brand: hsla(0xd5714f),
            user_bubble: hsla(0xe6e2d9),
            code_background: hsla(0xf3f1ec),
            danger: hsla(0xc2412d),
            success: hsla(0x3f9a4a),
            warning: hsla(0xb7791f),
            selection: hsla(0xf2d9cc),
            link: hsla(0xb5532f),
            ui_font: default_ui_font(),
            mono_font: default_mono_font(),
            syntax: [
                hsla(0x8a857b),
                hsla(0x4d7a2a),
                hsla(0x2f6f9f),
                hsla(0xb5532f),
                hsla(0x7a4fa0),
                hsla(0x9a6a10),
            ],
        }
    }

    #[must_use]
    pub fn dark() -> Self {
        Self {
            dark: true,
            background: hsla(0x1d1c1a),
            sidebar: hsla(0x171614),
            surface: hsla(0x262522),
            surface_hover: hsla(0x2c2a27),
            selected: hsla(0x33312d),
            border: hsla(0x34322e),
            text: hsla(0xeceae4),
            text_muted: hsla(0x9d988e),
            text_faint: hsla(0x6f6b63),
            accent: hsla(0xeceae4),
            accent_text: hsla(0x1d1c1a),
            brand: hsla(0xe08a6a),
            user_bubble: hsla(0x33312d),
            code_background: hsla(0x151412),
            danger: hsla(0xe5735f),
            success: hsla(0x7cc47f),
            warning: hsla(0xe0b052),
            selection: hsla(0x5a3a2c),
            link: hsla(0xe9a07f),
            ui_font: default_ui_font(),
            mono_font: default_mono_font(),
            syntax: [
                hsla(0x8b867c),
                hsla(0xa9c97a),
                hsla(0x86b7e0),
                hsla(0xe9a07f),
                hsla(0xc6a6e8),
                hsla(0xe0b052),
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
