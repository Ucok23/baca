mod highlight;
mod markdown;
mod theme;
use gpui::{
    actions, div, img, prelude::*, px, size, App, Bounds, ClipboardItem, Entity, FontStyle,
    FontWeight, HighlightStyle, InteractiveText, KeyBinding, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathPromptOptions, Pixels, Point, Render, ScrollHandle,
    SharedString, SharedUri, StatefulInteractiveElement, StrikethroughStyle, StyledText,
    TextLayout,
    UnderlineStyle, WindowBounds, WindowOptions,
};
use markdown::{Block, Marker};
use pulldown_cmark::{Alignment, HeadingLevel};
use std::cell::RefCell;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use theme::{Palette, Theme};

actions!(
    baca,
    [ChooseFolder, Reload, CycleTheme, Library, CopySelection, SelectAll]
);

const MARKDOWN_EXTENSIONS: [&str; 3] = ["md", "markdown", "mdx"];

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| MARKDOWN_EXTENSIONS.contains(&x))
}

#[derive(Clone)]
struct Entry {
    path: PathBuf,
    title: String,
    preview: String,
}

fn scan(root: &Path) -> Vec<Entry> {
    fn walk(d: &Path, o: &mut Vec<Entry>) {
        let Ok(xs) = std::fs::read_dir(d) else { return };
        for x in xs.flatten() {
            let p = x.path();
            let n = x.file_name().to_string_lossy().to_string();
            if n.starts_with('.') || matches!(n.as_str(), "target" | "node_modules" | "vendor") {
                continue;
            }
            if p.is_dir() {
                walk(&p, o)
            } else if is_markdown(&p) {
                let s = std::fs::read_to_string(&p).unwrap_or_default();
                let mut l = s.lines().filter(|x| !x.trim().is_empty());
                o.push(Entry {
                    path: p,
                    title: l.next().unwrap_or(&n).trim_start_matches('#').trim().into(),
                    preview: l
                        .next()
                        .unwrap_or("No preview available.")
                        .replace(['#', '*', '`'], ""),
                })
            }
        }
    }
    let mut o = vec![];
    walk(root, &mut o);
    o.sort_by(|a, b| a.title.cmp(&b.title));
    o
}

/// One laid-out run of document text. Pieces are registered in document order
/// as the page renders, which is what lets a selection run across blocks.
struct Piece {
    text: String,
    layout: TextLayout,
}

/// A position in the document: which piece, and how far into its text.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Cursor {
    piece: usize,
    offset: usize,
}

/// Fold a selection into a block's style runs. `with_highlights` requires
/// sorted, non-overlapping ranges, so overlaps are split rather than stacked.
fn apply_selection(
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    selection: Range<usize>,
    background: gpui::Hsla,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let bare = HighlightStyle {
        background_color: Some(background),
        ..Default::default()
    };
    let mut out = Vec::with_capacity(highlights.len() + 2);
    let mut covered = selection.start;
    for (range, style) in highlights {
        if range.end <= selection.start || range.start >= selection.end {
            out.push((range, style));
            continue;
        }
        if range.start < selection.start {
            out.push((range.start..selection.start, style));
        }
        let inside = range.start.max(selection.start)..range.end.min(selection.end);
        if covered < inside.start {
            out.push((covered..inside.start, bare));
        }
        out.push((
            inside.clone(),
            HighlightStyle {
                background_color: Some(background),
                ..style
            },
        ));
        covered = inside.end;
        if range.end > selection.end {
            out.push((selection.end..range.end, style));
        }
    }
    if covered < selection.end {
        out.push((covered..selection.end, bare));
    }
    out.sort_by_key(|(range, _)| range.start);
    out
}

struct Baca {
    root: Option<PathBuf>,
    entries: Vec<Entry>,
    open: Option<PathBuf>,
    title: String,
    doc: markdown::Document,
    theme: Theme,
    focus: gpui::FocusHandle,
    scroll: ScrollHandle,
    pieces: Rc<RefCell<Vec<Piece>>>,
    anchor: Option<Cursor>,
    head: Option<Cursor>,
    dragging: bool,
    shown_title: String,
}

impl Baca {
    fn new(root: Option<PathBuf>, cx: &mut gpui::Context<Self>) -> Self {
        let entries = root.as_deref().map(scan).unwrap_or_default();
        Self {
            root,
            entries,
            open: None,
            title: String::new(),
            doc: Default::default(),
            theme: Theme::Paper,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            pieces: Default::default(),
            anchor: None,
            head: None,
            dragging: false,
            shown_title: String::new(),
        }
    }

    fn reload(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(r) = &self.root {
            self.entries = scan(r)
        }
        // Re-read whatever is on screen too, so Ctrl+R does the obvious thing
        // while reading rather than only refreshing the shelf.
        if let Some(path) = self.open.clone() {
            self.read(path, cx);
            return;
        }
        cx.notify()
    }

    fn read(&mut self, path: PathBuf, cx: &mut gpui::Context<Self>) {
        let source = std::fs::read_to_string(&path).unwrap_or_default();
        self.doc = markdown::parse(&source);
        self.title = self
            .doc
            .outline
            .first()
            .map(|(_, text, _)| text.clone())
            .or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        self.open = Some(path);
        self.anchor = None;
        self.head = None;
        self.pieces.borrow_mut().clear();
        self.scroll.scroll_to_item(0);
        cx.notify()
    }

    /// Resolve a link target the reader clicked. External URLs go to the
    /// browser; anything that lands on a Markdown file is opened in place.
    fn follow(&mut self, target: &str, cx: &mut gpui::Context<Self>) {
        if target.starts_with(markdown::FOOTNOTE_SCHEME) {
            return;
        }
        if let Some(scheme) = target.split_once("://").map(|(s, _)| s) {
            if matches!(scheme, "http" | "https" | "file") {
                cx.open_url(target);
                return;
            }
        }
        if target.starts_with("mailto:") {
            cx.open_url(target);
            return;
        }
        // In-document anchors are not navigable yet; ignore rather than
        // opening a nonsense file.
        if target.starts_with('#') {
            return;
        }
        let target = target.split('#').next().unwrap_or(target);
        let decoded = target.replace("%20", " ");
        if let Some(path) = self.resolve(&decoded) {
            self.read(path, cx);
        }
    }

    /// Find the file a relative or wiki-style link refers to: first beside the
    /// open document, then anywhere in the library.
    fn resolve(&self, target: &str) -> Option<PathBuf> {
        let base = self.open.as_ref().and_then(|p| p.parent());
        let mut candidates = Vec::new();
        if let Some(base) = base {
            candidates.push(base.join(target));
            for ext in MARKDOWN_EXTENSIONS {
                candidates.push(base.join(format!("{target}.{ext}")));
            }
        }
        if let Some(root) = &self.root {
            candidates.push(root.join(target));
            for ext in MARKDOWN_EXTENSIONS {
                candidates.push(root.join(format!("{target}.{ext}")));
            }
        }
        if let Some(hit) = candidates
            .into_iter()
            .find(|p| p.is_file() && is_markdown(p))
        {
            return Some(hit);
        }
        // Wiki-style links name a note, not a path, so fall back to matching
        // the stem or title anywhere in the library.
        let needle = target.trim().to_lowercase();
        self.entries
            .iter()
            .find(|e| {
                e.path
                    .file_stem()
                    .is_some_and(|s| s.to_string_lossy().to_lowercase() == needle)
                    || e.title.to_lowercase() == needle
            })
            .map(|e| e.path.clone())
    }

    /// Record a run of laid-out text and hand back its position in the
    /// document, so the next frame can paint a selection over it.
    fn register(&self, text: &str, layout: &TextLayout) -> usize {
        let mut pieces = self.pieces.borrow_mut();
        pieces.push(Piece {
            text: text.to_string(),
            layout: layout.clone(),
        });
        pieces.len() - 1
    }

    fn ordered_selection(&self) -> Option<(Cursor, Cursor)> {
        let (a, b) = (self.anchor?, self.head?);
        (a != b).then(|| if a <= b { (a, b) } else { (b, a) })
    }

    /// The part of `piece` that falls inside the selection, if any.
    fn selected_range(&self, piece: usize, len: usize) -> Option<Range<usize>> {
        let (start, end) = self.ordered_selection()?;
        if piece < start.piece || piece > end.piece {
            return None;
        }
        let from = if piece == start.piece {
            start.offset.min(len)
        } else {
            0
        };
        let to = if piece == end.piece {
            end.offset.min(len)
        } else {
            len
        };
        (from < to).then_some(from..to)
    }

    /// Map a window position onto a document position, snapping to the nearest
    /// piece when the pointer is in a margin rather than on text.
    fn cursor_at(&self, position: Point<Pixels>) -> Option<Cursor> {
        let pieces = self.pieces.borrow();
        let mut nearest: Option<(Pixels, Cursor)> = None;
        for (ix, piece) in pieces.iter().enumerate() {
            let bounds = piece.layout.bounds();
            if bounds.contains(&position) {
                if let Ok(offset) = piece.layout.index_for_position(position) {
                    return Some(Cursor { piece: ix, offset });
                }
            }
            let distance = if position.y < bounds.top() {
                bounds.top() - position.y
            } else if position.y > bounds.bottom() {
                position.y - bounds.bottom()
            } else {
                px(0.)
            };
            let offset = match piece.layout.index_for_position(position) {
                Ok(offset) | Err(offset) => offset,
            };
            if nearest.as_ref().is_none_or(|(best, _)| distance < *best) {
                nearest = Some((distance, Cursor { piece: ix, offset }));
            }
        }
        nearest.map(|(_, cursor)| cursor)
    }

    fn selected_text(&self) -> String {
        let Some((start, end)) = self.ordered_selection() else {
            return String::new();
        };
        let pieces = self.pieces.borrow();
        let mut out = Vec::new();
        for ix in start.piece..=end.piece {
            let Some(piece) = pieces.get(ix) else { continue };
            let len = piece.text.len();
            let from = if ix == start.piece {
                start.offset.min(len)
            } else {
                0
            };
            let to = if ix == end.piece { end.offset.min(len) } else { len };
            if from < to {
                out.push(piece.text[from..to].to_string());
            }
        }
        out.join("\n")
    }

    fn copy_selection(&mut self, cx: &mut gpui::Context<Self>) {
        let text = self.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn select_all(&mut self, cx: &mut gpui::Context<Self>) {
        let pieces = self.pieces.borrow();
        let Some(last) = pieces.len().checked_sub(1) else {
            return;
        };
        let end = pieces[last].text.len();
        drop(pieces);
        self.anchor = Some(Cursor {
            piece: 0,
            offset: 0,
        });
        self.head = Some(Cursor {
            piece: last,
            offset: end,
        });
        cx.notify()
    }

    /// Whether the document leads with a heading of its own, at any level.
    fn opens_with_heading(&self) -> bool {
        matches!(self.doc.blocks.first(), Some(Block::Heading { .. }))
    }

    fn directory(&self) -> Option<&Path> {
        self.open.as_ref().and_then(|p| p.parent())
    }

    /// Turn styled inline runs into one text element, with a click target for
    /// every link range.
    fn inline(
        &self,
        text: &markdown::Text,
        p: &Palette,
        id: usize,
        this: &Entity<Self>,
    ) -> gpui::AnyElement {
        let mut source = String::new();
        let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
        let mut families: Vec<(Range<usize>, SharedString)> = Vec::new();
        let mut ranges: Vec<Range<usize>> = Vec::new();
        let mut targets: Vec<String> = Vec::new();

        for span in &text.spans {
            let start = source.len();
            source.push_str(&span.text);
            let range = start..source.len();
            if range.is_empty() {
                continue;
            }
            let mut style = HighlightStyle::default();
            if span.style.strong {
                style.font_weight = Some(FontWeight::BOLD);
            }
            if span.style.emphasis {
                style.font_style = Some(FontStyle::Italic);
            }
            if span.style.strikethrough {
                style.strikethrough = Some(StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(p.text_muted.into()),
                });
            }
            if span.style.code {
                style.background_color = Some(p.code.into());
                families.push((range.clone(), theme::mono()));
            }
            if let Some(url) = &span.link {
                style.color = Some(p.accent.into());
                style.underline = Some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(p.border_mid.into()),
                    wavy: false,
                });
                ranges.push(range.clone());
                targets.push(url.clone());
            }
            if style != HighlightStyle::default() {
                highlights.push((range, style));
            }
        }

        let styled = StyledText::new(source.clone());
        let piece = self.register(&source, styled.layout());
        if let Some(range) = self.selected_range(piece, source.len()) {
            highlights = apply_selection(highlights, range, p.selection.into());
        }
        let styled = styled
            .with_highlights(highlights)
            .with_font_family_overrides(families);
        if ranges.is_empty() {
            return styled.into_any_element();
        }
        let this = this.clone();
        InteractiveText::new(("inline", id), styled)
            .on_click(ranges, move |ix, _, cx| {
                let Some(target) = targets.get(ix).cloned() else {
                    return;
                };
                this.update(cx, |state, cx| state.follow(&target, cx));
            })
            .into_any_element()
    }

    fn block(
        &self,
        b: &Block,
        p: &Palette,
        id: usize,
        this: &Entity<Self>,
    ) -> gpui::AnyElement {
        match b {
            Block::Heading { level, text, .. } => {
                let base = div()
                    .font_family(theme::display())
                    .text_color(p.text)
                    .mt_7()
                    .mb_2();
                let sized = match level {
                    HeadingLevel::H1 => base.text_3xl(),
                    HeadingLevel::H2 => base.text_2xl(),
                    HeadingLevel::H3 => base.text_xl(),
                    HeadingLevel::H4 => base.text_lg(),
                    _ => base.text_base(),
                };
                sized
                    .child(self.inline(text, p, id, this))
                    .into_any_element()
            }
            Block::Paragraph(t) => div()
                .font_family(theme::body())
                .text_lg()
                .line_height(px(29.))
                .text_color(p.text)
                .mb_4()
                .child(self.inline(t, p, id, this))
                .into_any_element(),
            Block::Quote(t) => div()
                .font_family(theme::body())
                .text_lg()
                .line_height(px(29.))
                .text_color(p.text_muted)
                .pl_5()
                .my_5()
                .border_l_2()
                .border_color(p.accent)
                .child(self.inline(t, p, id, this))
                .into_any_element(),
            Block::Code { lang, text } => {
                let mut colors: Vec<(Range<usize>, HighlightStyle)> =
                    highlight::spans(lang.as_deref(), text, self.theme.syntax())
                        .into_iter()
                        .map(|(range, color)| {
                            (
                                range,
                                HighlightStyle {
                                    color: Some(color.into()),
                                    ..Default::default()
                                },
                            )
                        })
                        .collect();
                let painted = StyledText::new(text.clone());
                let piece = self.register(text, painted.layout());
                if let Some(range) = self.selected_range(piece, text.len()) {
                    colors = apply_selection(colors, range, p.selection.into());
                }
                let painted = painted.with_highlights(colors);
                let body = div()
                    .font_family(theme::mono())
                    .text_sm()
                    .line_height(px(21.))
                    .text_color(p.text)
                    .child(painted);
                div()
                    .my_5()
                    .bg(p.code)
                    .border_1()
                    .border_color(p.border)
                    .children(lang.as_ref().map(|lang| {
                        div()
                            .font_family(theme::mono())
                            .text_xs()
                            .text_color(p.text_faint)
                            .px_4()
                            .pt_2()
                            .child(lang.to_uppercase())
                    }))
                    .child(div().p_4().child(body))
                    .into_any_element()
            }
            Block::Item {
                marker,
                depth,
                text,
            } => {
                let glyph = match marker {
                    Some(Marker::Bullet) => "•".to_string(),
                    Some(Marker::Ordinal(n)) => format!("{n}."),
                    Some(Marker::Task(true)) => "☑".to_string(),
                    Some(Marker::Task(false)) => "☐".to_string(),
                    None => String::new(),
                };
                let muted = matches!(marker, Some(Marker::Task(true)));
                div()
                    .flex()
                    .items_start()
                    .mb_2()
                    .ml(px(*depth as f32 * 22.))
                    .child(
                        div()
                            .w(px(26.))
                            .flex_none()
                            .font_family(theme::body())
                            .text_lg()
                            .line_height(px(29.))
                            .text_color(p.text_faint)
                            .child(glyph),
                    )
                    .child(
                        div()
                            .flex_1()
                            .font_family(theme::body())
                            .text_lg()
                            .line_height(px(29.))
                            .text_color(if muted { p.text_muted } else { p.text })
                            .child(self.inline(text, p, id, this)),
                    )
                    .into_any_element()
            }
            Block::Table { aligns, head, rows } => {
                let cell = |text: &markdown::Text,
                            ix: usize,
                            cell_id: usize,
                            strong: bool|
                 -> gpui::AnyElement {
                    let mut c = div()
                        .flex_1()
                        .px_3()
                        .py_2()
                        .font_family(theme::body())
                        .text_base()
                        .text_color(if strong { p.text } else { p.text_muted })
                        .child(self.inline(text, p, cell_id, this));
                    c = match aligns.get(ix) {
                        Some(Alignment::Center) => c.text_center(),
                        Some(Alignment::Right) => c.text_right(),
                        _ => c,
                    };
                    c.into_any_element()
                };
                let mut table = div()
                    .my_5()
                    .border_1()
                    .border_color(p.border)
                    .child(
                        div()
                            .flex()
                            .bg(p.bg_subtle)
                            .border_b_1()
                            .border_color(p.border_mid)
                            .children(head.iter().enumerate().map(|(i, t)| {
                                cell(t, i, id * 1000 + i, true)
                            })),
                    );
                for (r, row) in rows.iter().enumerate() {
                    table = table.child(
                        div()
                            .flex()
                            .border_t_1()
                            .border_color(p.border)
                            .children(row.iter().enumerate().map(|(i, t)| {
                                cell(t, i, id * 1000 + (r + 1) * 16 + i, false)
                            })),
                    );
                }
                table.into_any_element()
            }
            Block::Image { url, alt } => {
                // A `SharedString` source means an asset compiled into the
                // binary, so a file on disk has to be passed as a path.
                let source = if url.starts_with("http://") || url.starts_with("https://") {
                    Some(img(SharedUri::from(url.clone())))
                } else {
                    self.directory()
                        .map(|d| d.join(url))
                        .filter(|p| p.is_file())
                        .map(img)
                };
                let figure = div().my_6().flex().flex_col().items_center();
                match source {
                    Some(source) => figure
                        .child(source.max_w_full().max_h(px(520.)))
                        .children((!alt.is_empty()).then(|| {
                            div()
                                .font_family(theme::body())
                                .text_sm()
                                .text_color(p.text_faint)
                                .mt_2()
                                .child(alt.clone())
                        }))
                        .into_any_element(),
                    None => figure
                        .child(
                            div()
                                .font_family(theme::mono())
                                .text_xs()
                                .text_color(p.text_faint)
                                .px_4()
                                .py_6()
                                .bg(p.bg_subtle)
                                .child(format!("missing image — {url}")),
                        )
                        .into_any_element(),
                }
            }
            Block::Footnote { label, text } => div()
                .flex()
                .items_start()
                .mt_2()
                .child(
                    // Sized by its content: a long label must not wrap into a
                    // narrow column.
                    div()
                        .flex_none()
                        .pr_3()
                        .font_family(theme::mono())
                        .text_xs()
                        .text_color(p.accent)
                        .child(format!("[{label}]")),
                )
                .child(
                    div()
                        .flex_1()
                        .font_family(theme::body())
                        .text_sm()
                        .text_color(p.text_muted)
                        .child(self.inline(text, p, id, this)),
                )
                .into_any_element(),
            Block::Rule => div().h(px(1.)).my_7().bg(p.border_mid).into_any_element(),
        }
    }

    fn choose(&mut self, cx: &mut gpui::Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose a Markdown library folder".into()),
        });
        cx.spawn(async move |this, cx| {
            let p = rx
                .await
                .ok()
                .and_then(Result::ok)
                .and_then(|x| x.and_then(|mut x| x.pop()));
            let _ = this.update(cx, |s, cx| {
                if let Some(p) = p {
                    s.root = Some(p);
                    s.open = None;
                    s.doc = Default::default();
                    s.reload(cx)
                }
            });
        })
        .detach()
    }
}

impl Render for Baca {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        // Name the window after what is being read, so a taskbar or window
        // switcher says something useful.
        let title = match (&self.open, self.title.is_empty()) {
            (Some(_), false) => format!("{} — baca", self.title),
            _ => "baca".to_string(),
        };
        if title != self.shown_title {
            window.set_window_title(&title);
            self.shown_title = title;
        }
        let p = self.theme.palette();
        let this = cx.entity();
        // Pieces are re-registered from scratch each frame, in document order,
        // so the indices a selection holds stay meaningful between frames.
        self.pieces.borrow_mut().clear();
        let es = self.entries.clone();
        div()
            .size_full()
            .bg(p.bg)
            .track_focus(&self.focus)
            .key_context("Baca")
            .on_action(cx.listener(|s, _: &ChooseFolder, _, cx| s.choose(cx)))
            .on_action(cx.listener(|s, _: &Reload, _, cx| s.reload(cx)))
            .on_action(cx.listener(|s, _: &CycleTheme, _, cx| {
                s.theme = s.theme.next();
                cx.notify()
            }))
            .on_action(cx.listener(|s, _: &Library, _, cx| {
                s.open = None;
                cx.notify()
            }))
            .on_action(cx.listener(|s, _: &CopySelection, _, cx| s.copy_selection(cx)))
            .on_action(cx.listener(|s, _: &SelectAll, _, cx| s.select_all(cx)))
            .child(
                div()
                    .h_full()
                    .flex()
                    .child(
                        div()
                            .w(px(250.))
                            .h_full()
                            .bg(p.bg_subtle)
                            .border_r_1()
                            .border_color(p.border)
                            .p_6()
                            .child(
                                div()
                                    .font_family(theme::display())
                                    .text_xl()
                                    .text_color(p.text)
                                    .child("baca"),
                            )
                            .child(
                                div()
                                    .font_family(theme::mono())
                                    .text_xs()
                                    .text_color(p.text_faint)
                                    .mt_1()
                                    .child("MARKDOWN LIBRARY"),
                            )
                            .child(
                                div()
                                    .font_family(theme::body())
                                    .text_sm()
                                    .text_color(p.text_muted)
                                    .mt_8()
                                    .child(
                                        self.root
                                            .as_ref()
                                            .map(|x| x.display().to_string())
                                            .unwrap_or_else(|| "Choose a folder".into()),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id("scroll")
                            .flex_1()
                            .h_full()
                            .overflow_scroll()
                            .track_scroll(&self.scroll)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|s, e: &MouseDownEvent, _, cx| {
                                    s.anchor = s.cursor_at(e.position);
                                    s.head = s.anchor;
                                    s.dragging = s.anchor.is_some();
                                    cx.notify()
                                }),
                            )
                            .on_mouse_move(cx.listener(|s, e: &MouseMoveEvent, _, cx| {
                                if s.dragging {
                                    s.head = s.cursor_at(e.position);
                                    cx.notify()
                                }
                            }))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|s, _: &MouseUpEvent, _, cx| {
                                    s.dragging = false;
                                    cx.notify()
                                }),
                            )
                            .child(
                                div()
                                    .max_w(px(900.))
                                    .mx_auto()
                                    .px_10()
                                    .pb_16()
                                    .child(
                                        div()
                                            .h(px(76.))
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .border_b_1()
                                            .border_color(p.border)
                                            .child(
                                                div()
                                                    .font_family(theme::mono())
                                                    .text_xs()
                                                    .text_color(p.accent)
                                                    .child("Choose folder  Ctrl+O"),
                                            )
                                            .child(
                                                div()
                                                    .font_family(theme::mono())
                                                    .text_xs()
                                                    .text_color(p.accent)
                                                    .child(format!(
                                                        "{}  Ctrl+T",
                                                        self.theme.label()
                                                    )),
                                            ),
                                    )
                                    .child(if self.open.is_some() {
                                        div()
                                            .pt_10()
                                            // A document that opens on its own
                                            // heading already states its title.
                                            .children((!self.opens_with_heading()).then(|| {
                                                div()
                                                    .font_family(theme::display())
                                                    .text_3xl()
                                                    .text_color(p.text)
                                                    .child(self.title.clone())
                                            }))
                                            .children(
                                                self.doc
                                                    .blocks
                                                    .iter()
                                                    .enumerate()
                                                    .map(|(i, b)| self.block(b, &p, i, &this)),
                                            )
                                            .into_any_element()
                                    } else {
                                        div()
                                            .pt_10()
                                            .child(
                                                div()
                                                    .font_family(theme::display())
                                                    .text_3xl()
                                                    .text_color(p.text)
                                                    .child("Your reading shelf"),
                                            )
                                            .child(
                                                div()
                                                    .font_family(theme::body())
                                                    .text_lg()
                                                    .text_color(p.text_muted)
                                                    .mb_8()
                                                    .child(format!("{} Markdown files", es.len())),
                                            )
                                            .children(es.into_iter().enumerate().map(|(i, e)| {
                                                let path = e.path.clone();
                                                div()
                                                    .id(("entry", i))
                                                    .cursor_pointer()
                                                    .py_5()
                                                    .border_t_1()
                                                    .border_color(p.border)
                                                    .on_click(cx.listener(move |s, _, _, cx| {
                                                        s.read(path.clone(), cx)
                                                    }))
                                                    .child(
                                                        div()
                                                            .font_family(theme::display())
                                                            .text_xl()
                                                            .text_color(p.text)
                                                            .child(e.title),
                                                    )
                                                    .child(
                                                        div()
                                                            .font_family(theme::body())
                                                            .text_base()
                                                            .text_color(p.text_muted)
                                                            .mt_1()
                                                            .child(e.preview),
                                                    )
                                            }))
                                            .into_any_element()
                                    }),
                            ),
                    ),
            )
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let target = args.get(1).map(PathBuf::from);
    // A file argument opens that document; a folder argument opens a library.
    let file = target.clone().filter(|p| p.is_file());
    let root = target
        .filter(|p| p.is_dir())
        .or_else(|| file.as_ref().and_then(|f| f.parent().map(PathBuf::from)));
    gpui_platform::application().run(move |cx: &mut App| {
        theme::load_fonts(cx);
        cx.bind_keys([
            KeyBinding::new("ctrl-o", ChooseFolder, Some("Baca")),
            KeyBinding::new("ctrl-r", Reload, Some("Baca")),
            KeyBinding::new("ctrl-t", CycleTheme, Some("Baca")),
            KeyBinding::new("alt-left", Library, Some("Baca")),
            KeyBinding::new("ctrl-c", CopySelection, Some("Baca")),
            KeyBinding::new("ctrl-a", SelectAll, Some("Baca")),
        ]);
        let b = Bounds::centered(None, size(px(1100.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(b)),
                // Without an app id a compositor cannot tell baca's windows
                // apart from anything else, so rules and taskbars miss it.
                app_id: Some("baca".into()),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("baca".into()),
                    ..Default::default()
                }),
                window_min_size: Some(size(px(480.), px(360.))),
                ..Default::default()
            },
            move |w, cx| {
                let x = cx.new(|cx| {
                    let mut state = Baca::new(root, cx);
                    if let Some(file) = file {
                        state.read(file, cx);
                    }
                    state
                });
                let f = x.read(cx).focus.clone();
                w.focus(&f, cx);
                x
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
