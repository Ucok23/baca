use gpui::{rgb, App, Rgba, SharedString};
use std::sync::OnceLock;

/// The typefaces baca prefers, most wanted first. gpui's fallback for a family
/// it cannot find silently drops weight and slant — bold and italic stop
/// rendering — so the first family that is actually installed is chosen at
/// startup instead of naming one and hoping.
const DISPLAY_STACK: [&str; 5] = [
    "Fraunces",
    "Newsreader",
    "Nimbus Roman",
    "Liberation Serif",
    "DejaVu Serif",
];
const BODY_STACK: [&str; 4] = [
    "Newsreader",
    "Nimbus Roman",
    "Liberation Serif",
    "DejaVu Serif",
];
const MONO_STACK: [&str; 5] = [
    "IBM Plex Mono",
    "JetBrainsMono Nerd Font",
    "Adwaita Mono",
    "Liberation Mono",
    "DejaVu Sans Mono",
];

struct Fonts {
    display: SharedString,
    body: SharedString,
    mono: SharedString,
}

static FONTS: OnceLock<Fonts> = OnceLock::new();

fn pick(stack: &[&str], installed: &[String]) -> SharedString {
    stack
        .iter()
        .find(|want| {
            installed
                .iter()
                .any(|have| have.eq_ignore_ascii_case(want))
        })
        .map(|name| SharedString::from(name.to_string()))
        // Nothing matched: let the platform pick, rather than naming a family
        // that is certainly absent.
        .unwrap_or_else(|| SharedString::from(stack[stack.len() - 1].to_string()))
}

/// Resolve the three typefaces once, against what this machine actually has.
pub fn load_fonts(cx: &App) {
    let installed = cx.text_system().all_font_names();
    let _ = FONTS.set(Fonts {
        display: pick(&DISPLAY_STACK, &installed),
        body: pick(&BODY_STACK, &installed),
        mono: pick(&MONO_STACK, &installed),
    });
}

fn fonts() -> &'static Fonts {
    FONTS.get_or_init(|| Fonts {
        display: SharedString::from(DISPLAY_STACK[0]),
        body: SharedString::from(BODY_STACK[0]),
        mono: SharedString::from(MONO_STACK[0]),
    })
}

pub fn display() -> SharedString {
    fonts().display.clone()
}

pub fn body() -> SharedString {
    fonts().body.clone()
}

pub fn mono() -> SharedString {
    fonts().mono.clone()
}

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
    pub selection: Rgba,
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

    /// The syntect theme whose colours sit well on this palette.
    pub fn syntax(self) -> &'static str {
        match self {
            Self::Paper => "InspiredGitHub",
            Self::Kraft => "base16-eighties.dark",
            Self::Malleable => "base16-ocean.dark",
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
                selection: rgb(0xd8ddea),
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
                selection: rgb(0x3a3e52),
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
                selection: rgb(0x23383d),
            },
        }
    }
}
