//! Syntax colouring for fenced code blocks.
//!
//! Syntect's grammars are large and loading them is slow, so the syntax and
//! theme sets are built once, on first use, and shared from then on.

use gpui::Rgba;
use std::ops::Range;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn themes() -> &'static ThemeSet {
    static THEMES: OnceLock<ThemeSet> = OnceLock::new();
    THEMES.get_or_init(ThemeSet::load_defaults)
}

/// Colour one code block, as byte ranges into `text`. An unknown language, or
/// a missing theme, yields no ranges and the block renders in plain body ink.
pub fn spans(lang: Option<&str>, text: &str, theme: &str) -> Vec<(Range<usize>, Rgba)> {
    let syntaxes = syntaxes();
    let Some(syntax) = lang.and_then(|lang| {
        syntaxes
            .find_syntax_by_token(lang)
            .or_else(|| syntaxes.find_syntax_by_extension(lang))
    }) else {
        return Vec::new();
    };
    let Some(theme) = themes().themes.get(theme) else {
        return Vec::new();
    };

    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut out: Vec<(Range<usize>, Rgba)> = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let Ok(regions) = highlighter.highlight_line(line, syntaxes) else {
            break;
        };
        for (style, piece) in regions {
            let range = offset..offset + piece.len();
            offset = range.end;
            if piece.trim().is_empty() {
                continue;
            }
            let color = Rgba {
                r: style.foreground.r as f32 / 255.,
                g: style.foreground.g as f32 / 255.,
                b: style.foreground.b as f32 / 255.,
                a: 1.,
            };
            // Merge with the previous run when the colour has not changed, so
            // the text system gets as few runs as possible.
            match out.last_mut() {
                Some((prev, prev_color)) if *prev_color == color && prev.end == range.start => {
                    prev.end = range.end
                }
                _ => out.push((range, color)),
            }
        }
    }
    out
}
