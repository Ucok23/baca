use gpui::{rgb, Rgba};

pub const FONT_DISPLAY: &str = "Fraunces";
pub const FONT_BODY: &str = "Newsreader";
pub const FONT_MONO: &str = "IBM Plex Mono";

#[derive(Clone)]
pub struct Palette {
    pub bg: Rgba,
    pub bg_subtle: Rgba,
    pub bg_raised: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub text_faint: Rgba,
    pub accent: Rgba,
    pub accent_text: Rgba,
    pub border: Rgba,
    pub border_mid: Rgba,
    pub code: Rgba,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Paper,
    Kraft,
    Malleable,
}

impl Theme {
    pub fn label(self) -> &'static str {
        match self {
            Self::Paper => "Paper",
            Self::Kraft => "Kraft",
            Self::Malleable => "Malleable",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Paper => Self::Kraft,
            Self::Kraft => Self::Malleable,
            Self::Malleable => Self::Paper,
        }
    }

    pub fn palette(self) -> Palette {
        match self {
            Self::Paper => Palette {
                bg: rgb(0xf6f2e9),
                bg_subtle: rgb(0xefe9db),
                bg_raised: rgb(0xfdfbf5),
                text: rgb(0x23201a),
                text_muted: rgb(0x5e574b),
                text_faint: rgb(0x9a9183),
                accent: rgb(0x2e4a78),
                accent_text: rgb(0xfbf8f1),
                border: rgb(0xe4ddce),
                border_mid: rgb(0xd6ccb8),
                code: rgb(0xeee8dc),
            },
            Self::Kraft => Palette {
                bg: rgb(0x1c1a15),
                bg_subtle: rgb(0x242019),
                bg_raised: rgb(0x27231a),
                text: rgb(0xece5d3),
                text_muted: rgb(0xa39b85),
                text_faint: rgb(0x6f6858),
                accent: rgb(0x7fa0d9),
                accent_text: rgb(0x12182a),
                border: rgb(0x332e24),
                border_mid: rgb(0x423b2e),
                code: rgb(0x211e17),
            },
            Self::Malleable => Palette {
                bg: rgb(0x0a0c0f),
                bg_subtle: rgb(0x050506),
                bg_raised: rgb(0x1c1f24),
                text: rgb(0xd8dee5),
                text_muted: rgb(0x9aa1a8),
                text_faint: rgb(0x676b6f),
                accent: rgb(0x7fd4e0),
                accent_text: rgb(0x0a0c0f),
                border: rgb(0x1c1f24),
                border_mid: rgb(0x2a2f36),
                code: rgb(0x111419),
            },
        }
    }
}
