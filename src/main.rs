mod highlight;
mod library;
mod markdown;
mod settings;
mod theme;
mod watch;
use gpui::{
    actions, div, img, point, prelude::*, px, relative, size, App, AppContext, Bounds,
    ClipboardItem, Entity, FontStyle, FontWeight, HighlightStyle, InteractiveText, KeyBinding,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathPromptOptions,
    Pixels, Point, Render, ScrollHandle, SharedString, SharedUri, StatefulInteractiveElement,
    StrikethroughStyle, StyledText, Subscription, TextLayout, UnderlineStyle, WindowBounds,
    WindowOptions,
};
use markdown::{Block, Marker};
use pulldown_cmark::{Alignment, HeadingLevel};
use settings::Settings;
use std::cell::RefCell;
use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
use theme::{Palette, Theme};

actions!(
    baca,
    [
        ChooseFolder,
        Reload,
        CycleTheme,
        Library,
        CopySelection,
        SelectAll,
        StartSearch,
        CancelSearch,
        AcceptSearch,
        FindNext,
        FindPrev,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        LineDown,
        LineUp,
        PageDown,
        PageUp,
        GoTop,
        GoBottom,
        ToggleOutline,
        NextEntry,
        PrevEntry,
        OpenEntry,
    ]
);

/// The measure: how wide a column of prose is allowed to get, however wide
/// the window is.
const COLUMN: Pixels = px(900.);

/// How often the watcher's channel is drained.
const WATCH_INTERVAL: Duration = Duration::from_millis(400);

/// How often the reading position is written out while reading.
const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(3);

/// Base type sizes, in pixels before the reader's zoom is applied.
mod size_of {
    pub const TITLE: f32 = 30.;
    pub const H1: f32 = 30.;
    pub const H2: f32 = 24.;
    pub const H3: f32 = 20.;
    pub const H4: f32 = 18.;
    pub const H5: f32 = 16.;
    pub const BODY: f32 = 18.;
    pub const BODY_LINE: f32 = 29.;
    pub const SMALL: f32 = 14.;
    pub const CODE: f32 = 14.;
    pub const CODE_LINE: f32 = 21.;
    pub const LABEL: f32 = 12.;
}

const MARKDOWN_EXTENSIONS: [&str; 3] = ["md", "markdown", "mdx"];

/// One entry of a document's outline: its level, its text, and its anchor.
type Heading = (HeadingLevel, String, String);

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
    /// Title, path and body folded to lower case, so filtering the library is
    /// a plain substring test rather than a fresh read of every file.
    haystack: String,
}

impl Entry {
    fn matches(&self, needle: &str) -> bool {
        needle.is_empty() || self.haystack.contains(needle)
    }
}

/// Reduce inline markup to the words it wraps, so a shelf preview reads as
/// prose rather than as source. Notes often open with a banner image.
fn strip_markup(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find(['[', '!', '*', '`', '_']) {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        // `![alt](url)` and `[text](url)` both keep only the part in brackets.
        let link = rest.strip_prefix("![").or_else(|| rest.strip_prefix('['));
        if let Some(inner) = link {
            if let Some((text, tail)) = inner.split_once(']') {
                let tail = match tail.strip_prefix('(') {
                    Some(after) => after.split_once(')').map_or(tail, |(_, t)| t),
                    None => tail,
                };
                out.push_str(text);
                rest = tail;
                continue;
            }
        }
        let mut chars = rest.chars();
        let first = chars.next();
        if !matches!(first, Some('*' | '`' | '_')) {
            if let Some(c) = first {
                out.push(c);
            }
        }
        rest = chars.as_str();
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// Split leading front matter off a document, as (metadata, body). A file
/// that merely opens with a horizontal rule is left alone.
fn split_front_matter(source: &str) -> (&str, &str) {
    let Some(rest) = source
        .strip_prefix("---\n")
        .or_else(|| source.strip_prefix("---\r\n"))
    else {
        return ("", source);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if matches!(line.trim_end(), "---" | "...") {
            return (&rest[..offset], &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    // No closing fence: treat the whole file as body rather than swallow it.
    ("", source)
}

/// A shelf entry's title and one line of preview, taken cheaply from the raw
/// text. Front matter is metadata, not the opening of the note.
fn summarize(body: &str, filename: &str) -> (String, String) {
    let (meta, rest) = split_front_matter(body);
    let tidy = |line: &str| strip_markup(line.trim_start_matches('#').trim());
    let mut lines = rest.lines().filter(|l| !l.trim().is_empty()).map(tidy);
    let titled = meta
        .lines()
        .find_map(|l| l.strip_prefix("title:"))
        .map(|v| v.trim().trim_matches(['"', '\'']).to_string())
        .filter(|t| !t.is_empty());
    let (title, preview) = match titled {
        // Front matter named the note, so its first body line is still preview.
        Some(title) => (title, lines.next()),
        None => (
            lines.next().unwrap_or_else(|| filename.to_string()),
            lines.next(),
        ),
    };
    (
        title,
        preview.unwrap_or_else(|| "No preview available.".to_string()),
    )
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
                let body = std::fs::read_to_string(&p).unwrap_or_default();
                let (title, preview) = summarize(&body, &n);
                let haystack = format!("{title}\n{}\n{body}", p.to_string_lossy()).to_lowercase();
                o.push(Entry {
                    path: p,
                    title,
                    preview,
                    haystack,
                })
            }
        }
    }
    let mut o = vec![];
    walk(root, &mut o);
    o.sort_by(|a, b| a.title.cmp(&b.title));
    o
}

/// Which screen baca is showing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    /// The gallery: what you were reading, what you read lately, what you have.
    Home,
    /// One collection's documents.
    Shelf,
    /// A document.
    Reading,
}

/// Something on the home screen the keyboard can land on.
#[derive(Clone)]
enum HomeItem {
    Document(PathBuf),
    Collection(PathBuf),
}

/// The outline entries on show, once folded groups are taken out. A folded
/// entry hides everything nested under it, down to the next heading at its own
/// level or shallower.
fn visible_outline(outline: &[Heading], collapsed: &HashSet<usize>) -> Vec<usize> {
    let mut rows = Vec::new();
    let mut hidden_below: Option<usize> = None;
    for (ix, (level, _, _)) in outline.iter().enumerate() {
        let level = *level as usize;
        if let Some(limit) = hidden_below {
            if level > limit {
                continue;
            }
            hidden_below = None;
        }
        rows.push(ix);
        if collapsed.contains(&ix) {
            hidden_below = Some(level);
        }
    }
    rows
}

/// Whether an outline entry has anything nested under it.
fn has_children(outline: &[Heading], ix: usize) -> bool {
    let Some((level, _, _)) = outline.get(ix) else {
        return false;
    };
    outline
        .get(ix + 1)
        .is_some_and(|(next, _, _)| *next as usize > *level as usize)
}

/// Column widths for a table, as fractions of its width.
///
/// A column of "yes"/"no" does not deserve as much room as a column of prose,
/// but one long cell should not swallow the table either. Weights are the
/// square root of the longest cell, so a column ten times wordier gets more
/// room but not ten times more, and every column keeps a readable minimum.
fn column_widths(head: &[markdown::Text], rows: &[Vec<markdown::Text>]) -> Vec<f32> {
    let columns = head.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return Vec::new();
    }
    let length = |cell: &markdown::Text| -> f32 {
        cell.spans
            .iter()
            .map(|s| s.text.chars().count())
            .sum::<usize>() as f32
    };
    let mut weights = vec![0f32; columns];
    for cells in std::iter::once(head).chain(rows.iter().map(Vec::as_slice)) {
        for (ix, cell) in cells.iter().enumerate().take(columns) {
            weights[ix] = weights[ix].max(length(cell));
        }
    }
    // The offset before the square root compresses the short end: two labels
    // of three and five characters should not differ by a third, while a
    // paragraph should still be visibly wider than a label.
    for weight in &mut weights {
        *weight = (*weight + 8.).sqrt();
    }
    let total: f32 = weights.iter().sum();
    if total <= 0. {
        return vec![1. / columns as f32; columns];
    }
    // No column narrower than roughly half an even share, so a terse column
    // still has room for its heading.
    let floor = 0.5 / columns as f32;
    let mut fractions: Vec<f32> = weights.iter().map(|w| (w / total).max(floor)).collect();
    let sum: f32 = fractions.iter().sum();
    for fraction in &mut fractions {
        *fraction /= sum;
    }
    fractions
}

/// One laid-out run of document text. Pieces are registered in document order
/// as the page renders, which is what lets a selection run across blocks.
struct Piece {
    text: String,
    layout: TextLayout,
}

/// One occurrence of the search query, as found while rendering. Only the
/// piece is needed to scroll to it; the range is painted as it is found.
#[derive(Clone, Copy)]
struct Hit {
    piece: usize,
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
    /// Where each outline entry's heading was laid out, refreshed every frame.
    heading_pieces: Rc<RefCell<Vec<usize>>>,
    /// Every occurrence of the query in the open document, in reading order.
    hits: Rc<RefCell<Vec<Hit>>>,
    query: String,
    /// Whether keystrokes are going into the search field.
    typing: bool,
    hit: usize,
    scale: f32,
    outline: bool,
    /// Which library entry the keyboard is on.
    cursor: usize,
    view: View,
    /// The outline entry whose section is on screen.
    active: usize,
    /// Outline entries whose children are folded away.
    collapsed: HashSet<usize>,
    outline_scroll: ScrollHandle,
    /// Set when the reader crosses into a new section, so the outline can
    /// scroll that entry into view once.
    follow_outline: bool,
    collections: Vec<library::Collection>,
    settings: Settings,
    watch: Option<watch::Watch>,
    _subscriptions: Vec<Subscription>,
}

impl Baca {
    fn new(settings: Settings, cx: &mut gpui::Context<Self>) -> Self {
        let theme = Theme::from_label(&settings.theme).unwrap_or(Theme::Paper);
        let mut baca = Self {
            root: settings.root.clone(),
            entries: Vec::new(),
            open: None,
            title: String::new(),
            doc: Default::default(),
            theme,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            pieces: Default::default(),
            anchor: None,
            head: None,
            dragging: false,
            shown_title: String::new(),
            heading_pieces: Default::default(),
            hits: Default::default(),
            query: String::new(),
            typing: false,
            hit: 0,
            scale: settings.scale,
            outline: settings.outline,
            cursor: 0,
            view: View::Home,
            active: 0,
            collapsed: HashSet::new(),
            outline_scroll: ScrollHandle::new(),
            follow_outline: false,
            collections: Vec::new(),
            watch: watch::Watch::new(),
            settings,
            _subscriptions: Vec::new(),
        };
        baca.rescan(cx);
        baca.discover(cx);
        baca.observe_files(cx);
        baca.autosave(cx);
        baca
    }

    /// Persist the current shape of things, including where the reader is in
    /// the open document.
    fn remember(&mut self) {
        self.settings.root = self.root.clone();
        self.settings.open = self.open.clone();
        self.settings.theme = self.theme.label().to_string();
        self.settings.scale = self.scale;
        self.settings.outline = self.outline;
        if let Some(path) = &self.open {
            let path = path.clone();
            self.settings
                .remember_position(&path, f32::from(self.scroll.offset().y));
            let progress = self.progress();
            self.settings.update_progress(&path, progress);
        }
        self.settings.save();
    }

    /// Find the folders worth reading, off the main thread.
    fn discover(&mut self, cx: &mut gpui::Context<Self>) {
        // Only the standard note folders and the ones the reader added.
        // `root` is merely the collection currently open — folding it in here
        // would make the set of collections shift as you browse.
        let mut roots = library::default_roots();
        roots.extend(self.settings.folders.iter().cloned());
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move { library::discover(&roots) })
                .await;
            this.update(cx, |state, cx| {
                state.collections = found;
                cx.notify()
            })
        })
        .detach();
    }

    /// How far through the open document the reader has come.
    fn progress(&self) -> f32 {
        let max = f32::from(self.scroll.max_offset().y);
        if max <= 1. {
            // Nothing to scroll: the whole thing is on screen, so it is read.
            return 1.;
        }
        (f32::from(self.scroll.offset().y).abs() / max).clamp(0., 1.)
    }

    /// Everything the keyboard can land on at home, in the order shown.
    fn home_items(&self) -> Vec<HomeItem> {
        let needle = self.query.to_lowercase();
        let mut items: Vec<HomeItem> = self
            .settings
            .recent_reads()
            .into_iter()
            .filter(|r| {
                needle.is_empty()
                    || r.title.to_lowercase().contains(&needle)
                    || r.collection.to_lowercase().contains(&needle)
            })
            .map(|r| HomeItem::Document(r.path.clone()))
            .collect();
        items.extend(
            self.collections
                .iter()
                .filter(|c| needle.is_empty() || c.name.to_lowercase().contains(&needle))
                .map(|c| HomeItem::Collection(c.path.clone())),
        );
        items
    }

    fn open_collection(&mut self, path: PathBuf, cx: &mut gpui::Context<Self>) {
        self.root = Some(path);
        self.view = View::Shelf;
        // Each screen starts at its own top; only a document resumes where it
        // was left, and that is restored when it is read.
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.query.clear();
        self.typing = false;
        self.cursor = 0;
        self.entries.clear();
        self.rescan(cx);
        self.retarget_watch();
        self.remember();
        cx.notify()
    }

    /// Straight back to the gallery, from wherever you are.
    fn go_home(&mut self, cx: &mut gpui::Context<Self>) {
        self.remember();
        self.view = View::Home;
        self.open = None;
        self.doc = Default::default();
        self.query.clear();
        self.typing = false;
        self.cursor = 0;
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify()
    }

    /// Step back one screen: a document returns to its collection, a
    /// collection returns to the gallery.
    fn go_back(&mut self, cx: &mut gpui::Context<Self>) {
        self.query.clear();
        self.typing = false;
        self.cursor = 0;
        self.view = match self.view {
            View::Reading if self.root.is_some() => View::Shelf,
            _ => View::Home,
        };
        if self.view == View::Home {
            self.open = None;
            self.doc = Default::default();
        }
        self.remember();
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify()
    }

    /// Walk the library off the main thread: a large vault reads thousands of
    /// files, which must not stall the frame.
    fn rescan(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(root) = self.root.clone() else {
            self.entries.clear();
            cx.notify();
            return;
        };
        cx.spawn(async move |this, cx| {
            let entries = cx.background_spawn(async move { scan(&root) }).await;
            this.update(cx, |state, cx| {
                state.entries = entries;
                state.cursor = state.cursor.min(state.visible().len().saturating_sub(1));
                cx.notify()
            })
        })
        .detach();
    }

    /// Save the reading position now and then. Quitting fires `on_app_quit`,
    /// but a process that is killed outright never gets there, and losing your
    /// place in a long document is exactly what this is meant to prevent.
    fn autosave(&mut self, cx: &mut gpui::Context<Self>) {
        cx.spawn(async move |this, cx| {
            // `None`, not a NaN sentinel: every comparison against NaN is
            // false, so the first save would never fire.
            let mut saved: Option<f32> = None;
            loop {
                cx.background_executor().timer(AUTOSAVE_INTERVAL).await;
                let carry_on = this.update(cx, |state, _| {
                    let at = f32::from(state.scroll.offset().y);
                    let moved = saved.is_none_or(|last| (at - last).abs() > 1.0);
                    if state.open.is_some() && moved {
                        saved = Some(at);
                        state.remember();
                    }
                    true
                });
                if carry_on.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Poll the file watcher, reloading whatever actually changed.
    fn observe_files(&mut self, cx: &mut gpui::Context<Self>) {
        if self.watch.is_none() {
            return;
        }
        self.retarget_watch();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(WATCH_INTERVAL).await;
            let carry_on = this.update(cx, |state, cx| {
                let Some(watcher) = &state.watch else {
                    return false;
                };
                let changed = watcher.drain();
                if changed.is_empty() {
                    return true;
                }
                if let Some(open) = state.open.clone() {
                    // Only while it is actually on screen: an edit should not
                    // yank the reader out of the shelf and into the document.
                    if state.view == View::Reading && watch::touches(&changed, &open) {
                        // Keep the reading position: an edit elsewhere in the
                        // file should not throw the reader back to the top.
                        let offset = state.scroll.offset();
                        state.read(open, cx);
                        state.scroll.set_offset(offset);
                    }
                }
                if changed.iter().any(|p| is_markdown(p) || p.is_dir()) {
                    state.rescan(cx);
                }
                true
            });
            match carry_on {
                Ok(true) => continue,
                _ => break,
            }
        })
        .detach();
    }

    fn retarget_watch(&mut self) {
        let mut roots: Vec<PathBuf> = self.root.iter().cloned().collect();
        // A document opened by path may sit outside the library entirely.
        if let Some(open) = &self.open {
            if !roots.iter().any(|r| open.starts_with(r)) {
                roots.push(open.clone());
            }
        }
        if let Some(watcher) = &mut self.watch {
            watcher.observe(roots);
        }
    }

    fn reload(&mut self, cx: &mut gpui::Context<Self>) {
        self.rescan(cx);
        self.discover(cx);
        // Re-read whatever is on screen too, so Ctrl+R does the obvious thing
        // while reading rather than only refreshing the shelf.
        if self.view != View::Reading {
            cx.notify();
            return;
        }
        if let Some(path) = self.open.clone() {
            let offset = self.scroll.offset();
            self.read(path, cx);
            self.scroll.set_offset(offset);
            return;
        }
        cx.notify()
    }

    fn read(&mut self, path: PathBuf, cx: &mut gpui::Context<Self>) {
        let source = std::fs::read_to_string(&path).unwrap_or_default();
        self.doc = markdown::parse(&source);
        self.title = self
            .doc
            .meta
            .title()
            .map(str::to_string)
            .or_else(|| self.doc.outline.first().map(|(_, text, _)| text.clone()))
            .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let resumed = self.settings.position(&path);
        let collection = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Carry any progress forward: re-reading a document, or reloading it
        // after an edit on disk, must not reset how far through it you were.
        let progress = self
            .settings
            .recent
            .iter()
            .find(|r| r.path == path)
            .map_or(0., |r| r.progress);
        self.settings
            .remember_read(&path, &self.title, &collection, progress);
        self.view = View::Reading;
        self.open = Some(path);
        self.anchor = None;
        self.head = None;
        self.hit = 0;
        self.active = 0;
        self.collapsed.clear();
        self.pieces.borrow_mut().clear();
        self.hits.borrow_mut().clear();
        match resumed {
            Some(y) => self.scroll.set_offset(point(px(0.), px(y))),
            None => self.scroll.scroll_to_item(0),
        }
        self.retarget_watch();
        self.remember();
        cx.notify()
    }

    /// Indices of the library entries passing the current filter. Indices,
    /// not clones: an entry carries the whole file for searching, and cloning
    /// the list every frame would copy the library on each repaint.
    fn visible(&self) -> Vec<usize> {
        let needle = self.query.to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.matches(&needle))
            .map(|(ix, _)| ix)
            .collect()
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

    /// Follow the desktop's light/dark preference, unless the reader has
    /// already picked a theme by hand.
    fn apply_system_theme(&mut self, dark: bool) {
        if !self.settings.follow_system_theme {
            return;
        }
        self.theme = Theme::for_system(dark);
        self.settings.theme = self.theme.label().to_string();
    }

    /// Choosing a theme by hand means the desktop preference stops applying.
    fn cycle_theme(&mut self, cx: &mut gpui::Context<Self>) {
        self.theme = self.theme.next();
        self.settings.follow_system_theme = false;
        self.remember();
        cx.notify()
    }

    /// A text size, scaled by the reader's zoom.
    fn at(&self, base: f32) -> Pixels {
        px(base * self.scale)
    }

    fn zoom(&mut self, to: f32, cx: &mut gpui::Context<Self>) {
        // Rounded, so repeated steps do not drift to 1.3000001.
        let clamped = (to * 10.).round() / 10.;
        let clamped = clamped.clamp(0.6, 2.5);
        if (clamped - self.scale).abs() < f32::EPSILON {
            return;
        }
        self.scale = clamped;
        self.remember();
        cx.notify()
    }

    fn scroll_by(&self, dy: Pixels, cx: &mut gpui::Context<Self>) {
        let at = self.scroll.offset();
        self.scroll.set_offset(point(at.x, at.y - dy));
        cx.notify()
    }

    fn page(&self) -> Pixels {
        // A page turn leaves a couple of lines of overlap to read across.
        let height = self.scroll.bounds().size.height;
        (height - self.at(size_of::BODY_LINE) * 2.).max(px(80.))
    }

    fn go_top(&self, cx: &mut gpui::Context<Self>) {
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify()
    }

    fn go_bottom(&self, cx: &mut gpui::Context<Self>) {
        let max = self.scroll.max_offset();
        self.scroll.set_offset(point(px(0.), -max.y));
        cx.notify()
    }

    /// Bring a laid-out piece of text to just under the top of the page.
    fn scroll_to_piece(&self, piece: usize, cx: &mut gpui::Context<Self>) {
        let pieces = self.pieces.borrow();
        let Some(target) = pieces.get(piece) else {
            return;
        };
        let bounds = target.layout.bounds();
        let viewport = self.scroll.bounds();
        let at = self.scroll.offset();
        let delta = bounds.top() - viewport.top() - self.at(size_of::BODY_LINE);
        drop(pieces);
        self.scroll.set_offset(point(at.x, at.y - delta));
        cx.notify()
    }

    /// Work out which section the reader is in, from the previous frame's
    /// layout — this frame's has not been measured yet. Silence rather than a
    /// guess when there is nothing laid out to go on.
    fn measure_active_heading(&mut self) {
        let found = {
            let pieces = self.pieces.borrow();
            let headings = self.heading_pieces.borrow();
            if pieces.is_empty() || headings.is_empty() {
                None
            } else {
                // A heading counts as reached once it passes a line or two
                // below the top edge, which is where the eye actually is.
                let top = self.scroll.bounds().top() + self.at(size_of::BODY_LINE) * 2.;
                let mut active = 0;
                for (ix, &piece) in headings.iter().enumerate() {
                    let Some(piece) = pieces.get(piece) else {
                        continue;
                    };
                    if piece.layout.bounds().top() <= top {
                        active = ix;
                    } else {
                        break;
                    }
                }
                Some(active)
            }
        };
        if let Some(active) = found {
            if active != self.active {
                self.active = active;
                self.follow_outline = true;
            }
        }
    }

    fn outline_rows(&self) -> Vec<usize> {
        visible_outline(&self.doc.outline, &self.collapsed)
    }

    fn has_children(&self, ix: usize) -> bool {
        has_children(&self.doc.outline, ix)
    }

    fn toggle_collapsed(&mut self, ix: usize, cx: &mut gpui::Context<Self>) {
        if !self.collapsed.remove(&ix) {
            self.collapsed.insert(ix);
        }
        cx.notify()
    }

    fn jump_to_heading(&self, ix: usize, cx: &mut gpui::Context<Self>) {
        let piece = self.heading_pieces.borrow().get(ix).copied();
        if let Some(piece) = piece {
            self.scroll_to_piece(piece, cx);
        }
    }

    fn start_search(&mut self, cx: &mut gpui::Context<Self>) {
        self.typing = true;
        self.hit = 0;
        cx.notify()
    }

    fn cancel_search(&mut self, cx: &mut gpui::Context<Self>) {
        self.typing = false;
        self.query.clear();
        self.hits.borrow_mut().clear();
        self.cursor = 0;
        cx.notify()
    }

    /// Step through the matches in the open document, wrapping at the ends.
    fn step_hit(&mut self, forward: bool, cx: &mut gpui::Context<Self>) {
        let count = self.hits.borrow().len();
        if count == 0 {
            return;
        }
        self.hit = if forward {
            (self.hit + 1) % count
        } else {
            (self.hit + count - 1) % count
        };
        let piece = self.hits.borrow().get(self.hit).map(|h| h.piece);
        if let Some(piece) = piece {
            self.scroll_to_piece(piece, cx);
        }
        cx.notify()
    }

    /// Where the query occurs in one run of text, and which of those is the
    /// match the reader is currently on.
    fn matches_in(&self, text: &str, piece: usize) -> Vec<(Range<usize>, bool)> {
        if self.query.is_empty() || self.open.is_none() {
            return Vec::new();
        }
        let needle = self.query.to_lowercase();
        let haystack = text.to_lowercase();
        let mut found = Vec::new();
        let mut at = 0;
        while let Some(offset) = haystack[at..].find(&needle) {
            let start = at + offset;
            let end = start + needle.len();
            // Byte offsets from a lowercased copy only line up with the
            // original when the case folding did not change its length.
            if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                at = start + 1;
                continue;
            }
            let mut hits = self.hits.borrow_mut();
            let is_current = hits.len() == self.hit;
            hits.push(Hit { piece });
            drop(hits);
            found.push((start..end, is_current));
            at = end.max(start + 1);
        }
        found
    }

    /// Move the keyboard through whichever list is showing.
    fn step_entry(&mut self, forward: bool, cx: &mut gpui::Context<Self>) {
        let count = match self.view {
            View::Home => self.home_items().len(),
            _ => self.visible().len(),
        };
        if count == 0 {
            return;
        }
        self.cursor = match forward {
            true => (self.cursor + 1).min(count - 1),
            false => self.cursor.saturating_sub(1),
        };
        cx.notify()
    }

    /// Activate whatever the keyboard is on.
    fn open_focused(&mut self, cx: &mut gpui::Context<Self>) {
        if self.view == View::Home {
            match self.home_items().get(self.cursor).cloned() {
                Some(HomeItem::Document(path)) => {
                    self.typing = false;
                    self.query.clear();
                    self.read(path, cx)
                }
                Some(HomeItem::Collection(path)) => self.open_collection(path, cx),
                None => {}
            }
            return;
        }
        let Some(&ix) = self.visible().get(self.cursor) else {
            return;
        };
        let Some(path) = self.entries.get(ix).map(|e| e.path.clone()) else {
            return;
        };
        self.typing = false;
        self.query.clear();
        self.read(path, cx);
    }

    /// Feed a keystroke to the search field. Returns whether it was consumed.
    fn typed(&mut self, event: &KeyDownEvent, cx: &mut gpui::Context<Self>) -> bool {
        if !self.typing {
            return false;
        }
        let stroke = &event.keystroke;
        if stroke.modifiers.control || stroke.modifiers.alt || stroke.modifiers.platform {
            return false;
        }
        if stroke.key == "backspace" {
            self.query.pop();
            self.hit = 0;
            self.cursor = 0;
            cx.notify();
            return true;
        }
        let Some(text) = stroke
            .key_char
            .as_ref()
            .filter(|t| !t.is_empty() && !t.chars().any(|c| c.is_control()))
        else {
            return false;
        };
        self.query.push_str(text);
        self.hit = 0;
        self.cursor = 0;
        cx.notify();
        true
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
            let Some(piece) = pieces.get(ix) else {
                continue;
            };
            let len = piece.text.len();
            let from = if ix == start.piece {
                start.offset.min(len)
            } else {
                0
            };
            let to = if ix == end.piece {
                end.offset.min(len)
            } else {
                len
            };
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
        for (range, current) in self.matches_in(&source, piece) {
            let tint = if current { p.match_now } else { p.match_any };
            highlights = apply_selection(highlights, range, tint.into());
        }
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

    fn block(&self, b: &Block, p: &Palette, id: usize, this: &Entity<Self>) -> gpui::AnyElement {
        match b {
            Block::Heading { level, text, .. } => {
                // The outline needs to know which laid-out run each heading
                // became, so clicking an entry can scroll to it.
                self.heading_pieces
                    .borrow_mut()
                    .push(self.pieces.borrow().len());
                let base = div()
                    .font_family(theme::display())
                    .text_color(p.text)
                    .mt_7()
                    .mb_2();
                let sized = match level {
                    HeadingLevel::H1 => base.text_size(self.at(size_of::H1)),
                    HeadingLevel::H2 => base.text_size(self.at(size_of::H2)),
                    HeadingLevel::H3 => base.text_size(self.at(size_of::H3)),
                    HeadingLevel::H4 => base.text_size(self.at(size_of::H4)),
                    _ => base.text_size(self.at(size_of::H5)),
                };
                sized
                    .child(self.inline(text, p, id, this))
                    .into_any_element()
            }
            Block::Paragraph(t) => div()
                .font_family(theme::body())
                .text_size(self.at(size_of::BODY))
                .line_height(self.at(size_of::BODY_LINE))
                .text_color(p.text)
                .mb_4()
                .child(self.inline(t, p, id, this))
                .into_any_element(),
            Block::Quote(t) => div()
                .font_family(theme::body())
                .text_size(self.at(size_of::BODY))
                .line_height(self.at(size_of::BODY_LINE))
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
                for (range, current) in self.matches_in(text, piece) {
                    let tint = if current { p.match_now } else { p.match_any };
                    colors = apply_selection(colors, range, tint.into());
                }
                if let Some(range) = self.selected_range(piece, text.len()) {
                    colors = apply_selection(colors, range, p.selection.into());
                }
                let painted = painted.with_highlights(colors);
                let body = div()
                    .font_family(theme::mono())
                    .text_size(self.at(size_of::CODE))
                    .line_height(self.at(size_of::CODE_LINE))
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
                            .text_size(self.at(size_of::LABEL))
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
                            .text_size(self.at(size_of::BODY))
                            .line_height(self.at(size_of::BODY_LINE))
                            .text_color(p.text_faint)
                            .child(glyph),
                    )
                    .child(
                        div()
                            .flex_1()
                            // A flex item's automatic minimum width is its
                            // min-content size, which for a run of text is the
                            // whole line: without this the item refuses to
                            // shrink and the text never wraps.
                            .min_w_0()
                            .font_family(theme::body())
                            .text_size(self.at(size_of::BODY))
                            .line_height(self.at(size_of::BODY_LINE))
                            .text_color(if muted { p.text_muted } else { p.text })
                            .child(self.inline(text, p, id, this)),
                    )
                    .into_any_element()
            }
            Block::Table { aligns, head, rows } => {
                let widths = column_widths(head, rows);
                let even = 1. / widths.len().max(1) as f32;
                let cell = |text: &markdown::Text,
                            ix: usize,
                            cell_id: usize,
                            strong: bool|
                 -> gpui::AnyElement {
                    let mut c = div()
                        .w(relative(widths.get(ix).copied().unwrap_or(even)))
                        .flex_none()
                        .min_w_0()
                        .px_3()
                        .py_2()
                        .font_family(theme::body())
                        .text_size(self.at(size_of::H5))
                        .text_color(if strong { p.text } else { p.text_muted })
                        .child(self.inline(text, p, cell_id, this));
                    c = match aligns.get(ix) {
                        Some(Alignment::Center) => c.text_center(),
                        Some(Alignment::Right) => c.text_right(),
                        _ => c,
                    };
                    c.into_any_element()
                };
                let mut table = div().my_5().border_1().border_color(p.border).child(
                    div()
                        .flex()
                        .bg(p.bg_subtle)
                        .border_b_1()
                        .border_color(p.border_mid)
                        .children(
                            head.iter()
                                .enumerate()
                                .map(|(i, t)| cell(t, i, id * 1000 + i, true)),
                        ),
                );
                for (r, row) in rows.iter().enumerate() {
                    table = table.child(
                        div().flex().border_t_1().border_color(p.border).children(
                            row.iter()
                                .enumerate()
                                .map(|(i, t)| cell(t, i, id * 1000 + (r + 1) * 16 + i, false)),
                        ),
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
                                .text_size(self.at(size_of::SMALL))
                                .text_color(p.text_faint)
                                .mt_2()
                                .child(alt.clone())
                        }))
                        .into_any_element(),
                    None => figure
                        .child(
                            div()
                                .font_family(theme::mono())
                                .text_size(self.at(size_of::LABEL))
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
                        .text_size(self.at(size_of::LABEL))
                        .text_color(p.accent)
                        .child(format!("[{label}]")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(theme::body())
                        .text_size(self.at(size_of::SMALL))
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
                    if !s.settings.folders.contains(&p) {
                        s.settings.folders.push(p.clone());
                    }
                    s.open = None;
                    s.doc = Default::default();
                    s.discover(cx);
                    s.open_collection(p, cx);
                }
            });
        })
        .detach()
    }
}

impl Baca {
    /// The sidebar: the outline of the open document, or the library's home.
    /// The sidebar, which always describes whatever the page is showing: a
    /// document's outline while reading, the collection while on its shelf.
    fn sidebar(&self, p: &Palette, cx: &mut gpui::Context<Self>) -> gpui::AnyElement {
        let reading = self.view == View::Reading;
        let outline = reading && self.outline && !self.doc.outline.is_empty();
        let mut panel = div()
            .w(px(260.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(p.bg_subtle)
            .border_r_1()
            .border_color(p.border)
            .child(
                div()
                    .px_6()
                    .pt_6()
                    .child(
                        div()
                            .id("wordmark")
                            .cursor_pointer()
                            .font_family(theme::display())
                            .text_size(self.at(size_of::H3))
                            .text_color(p.text)
                            .on_click(cx.listener(|state, _, _, cx| state.go_home(cx)))
                            .child("baca"),
                    )
                    .child(
                        div()
                            .font_family(theme::mono())
                            .text_size(self.at(size_of::LABEL))
                            .text_color(p.text_faint)
                            .mt_1()
                            .child(if outline { "CONTENTS" } else { "COLLECTION" }),
                    ),
            );

        if outline {
            let smallest = self
                .doc
                .outline
                .iter()
                .map(|(level, _, _)| *level as usize)
                .min()
                .unwrap_or(1);
            panel = panel.child(
                div()
                    .id("outline")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.outline_scroll)
                    .px_2()
                    .pt_4()
                    .pb_6()
                    .children(self.outline_rows().into_iter().map(|ix| {
                        let (level, text, _) = &self.doc.outline[ix];
                        let depth = (*level as usize).saturating_sub(smallest);
                        let here = ix == self.active;
                        let folded = self.collapsed.contains(&ix);
                        let parent = self.has_children(ix);
                        div()
                            .flex()
                            .items_start()
                            .pl(px(4. + depth as f32 * 10.))
                            .when(here, |this| this.bg(p.bg_raised))
                            // A rule down the left marks where you are far
                            // more quietly than colouring the whole row.
                            .border_l_2()
                            .border_color(if here { p.accent } else { p.bg_subtle })
                            .child(
                                div()
                                    .id(("fold", ix))
                                    .w(px(16.))
                                    .flex_none()
                                    .py_1()
                                    .font_family(theme::mono())
                                    .text_size(self.at(size_of::LABEL))
                                    .text_color(p.text_faint)
                                    .when(parent, |this| {
                                        this.cursor_pointer()
                                            .hover(|h| h.text_color(p.accent))
                                            .on_click(cx.listener(move |state, _, _, cx| {
                                                state.toggle_collapsed(ix, cx)
                                            }))
                                    })
                                    .child(match (parent, folded) {
                                        (true, true) => "▸",
                                        (true, false) => "▾",
                                        (false, _) => "",
                                    }),
                            )
                            .child(
                                div()
                                    .id(("outline", ix))
                                    .flex_1()
                                    .min_w_0()
                                    .cursor_pointer()
                                    .py_1()
                                    .pr_2()
                                    .font_family(theme::body())
                                    .text_size(self.at(size_of::SMALL))
                                    // Top-level entries, and wherever you
                                    // are, read at full strength.
                                    .text_color(if here || depth == 0 {
                                        p.text
                                    } else {
                                        p.text_muted
                                    })
                                    .hover(|this| this.text_color(p.accent))
                                    .on_click(cx.listener(move |state, _, _, cx| {
                                        state.jump_to_heading(ix, cx)
                                    }))
                                    .child(text.clone()),
                            )
                    })),
            );
        } else {
            // On a shelf, or in a document with no headings: say which
            // collection this is and how much is in it.
            let name = self
                .root
                .as_ref()
                .and_then(|r| r.file_name())
                .map(|n| n.to_string_lossy().into_owned());
            let note = if reading {
                "No headings — Ctrl+B".to_string()
            } else {
                match self.entries.len() {
                    1 => "1 document".to_string(),
                    n => format!("{n} documents"),
                }
            };
            panel = panel.child(
                div()
                    .flex_1()
                    .px_6()
                    .pt_8()
                    .child(
                        div()
                            .font_family(theme::display())
                            .text_size(self.at(size_of::H4))
                            .text_color(p.text)
                            .child(name.unwrap_or_else(|| "No folder".into())),
                    )
                    .child(
                        div()
                            .font_family(theme::mono())
                            .text_size(self.at(size_of::LABEL))
                            .text_color(p.text_faint)
                            .mt_1()
                            .child(note),
                    )
                    .child(
                        div()
                            .font_family(theme::body())
                            .text_size(self.at(size_of::SMALL))
                            .text_color(p.text_muted)
                            .mt_6()
                            .child(
                                self.root
                                    .as_ref()
                                    .map(|x| x.display().to_string())
                                    .unwrap_or_else(|| "Choose a folder — Ctrl+O".into()),
                            ),
                    ),
            );
        }

        // Front-matter tags belong to the open document, not to the shelf.
        let tags = self.doc.meta.tags();
        if reading && !tags.is_empty() {
            panel = panel.child(
                div()
                    .px_6()
                    .pb_6()
                    .pt_4()
                    .border_t_1()
                    .border_color(p.border)
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(tags.into_iter().map(|tag| {
                        div()
                            .px_2()
                            .py_1()
                            .bg(p.bg_raised)
                            .font_family(theme::mono())
                            .text_size(self.at(size_of::LABEL))
                            .text_color(p.text_muted)
                            .child(tag)
                    })),
            );
        }
        panel.into_any_element()
    }

    /// The strip above the page: what the keys do, and the search field.
    fn header(&self, p: &Palette, cx: &mut gpui::Context<Self>) -> gpui::AnyElement {
        let label = |text: String| {
            div()
                .font_family(theme::mono())
                .text_size(self.at(size_of::LABEL))
                .text_color(p.accent)
                .child(text)
        };
        if self.typing || !self.query.is_empty() {
            let hits = self.hits.borrow().len();
            let tally = if self.view == View::Home {
                format!("{} found", self.home_items().len())
            } else if self.view == View::Shelf {
                format!("{} of {} notes", self.visible().len(), self.entries.len())
            } else if hits == 0 {
                "no matches".to_string()
            } else {
                format!("{} of {hits}", self.hit + 1)
            };
            return div()
                .h(px(76.))
                .flex()
                .items_center()
                .gap_3()
                .border_b_1()
                .border_color(p.border)
                .child(label("FIND".into()))
                .child(
                    div()
                        .flex_1()
                        .font_family(theme::mono())
                        .text_size(self.at(size_of::SMALL))
                        .text_color(p.text)
                        .child(if self.query.is_empty() {
                            // A caret alone reads as "type here".
                            "|".to_string()
                        } else if self.typing {
                            format!("{}|", self.query)
                        } else {
                            self.query.clone()
                        }),
                )
                .child(
                    div()
                        .font_family(theme::mono())
                        .text_size(self.at(size_of::LABEL))
                        .text_color(p.text_faint)
                        .child(tally),
                )
                .into_any_element();
        }
        // Where you are, and the way back out of it.
        let back = match self.view {
            View::Home => None,
            View::Shelf => Some("←  All collections".to_string()),
            View::Reading => Some(match &self.root {
                Some(root) => format!(
                    "←  {}",
                    root.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Back".into())
                ),
                None => "←  Home".to_string(),
            }),
        };
        div()
            .h(px(76.))
            .flex()
            .items_center()
            .justify_between()
            .gap_6()
            .border_b_1()
            .border_color(p.border)
            .child(match back {
                Some(text) => div()
                    .id("back")
                    .cursor_pointer()
                    .on_click(cx.listener(|state, _, _, cx| state.go_back(cx)))
                    .child(label(text))
                    .into_any_element(),
                None => div()
                    .id("choose")
                    .cursor_pointer()
                    .on_click(cx.listener(|state, _, _, cx| state.choose(cx)))
                    .child(label("Choose folder  Ctrl+O".into()))
                    .into_any_element(),
            })
            .child(label("Find  Ctrl+F".into()))
            .child(
                div()
                    .id("theme")
                    .cursor_pointer()
                    .on_click(cx.listener(|state, _, _, cx| state.cycle_theme(cx)))
                    .child(label(format!("{}  Ctrl+T", self.theme.label()))),
            )
            .into_any_element()
    }

    /// A small caption line: collection, progress, and when it was read.
    fn caption(&self, p: &Palette, parts: Vec<String>) -> gpui::AnyElement {
        div()
            .font_family(theme::mono())
            .text_size(self.at(size_of::LABEL))
            .text_color(p.text_faint)
            .mt_1()
            .child(parts.join("  ·  "))
            .into_any_element()
    }

    /// A thin bar showing how far through a document the reader got.
    fn progress_bar(&self, p: &Palette, progress: f32) -> gpui::AnyElement {
        div()
            .mt_3()
            .h(px(3.))
            .w_full()
            .bg(p.border)
            .child(
                div()
                    .h_full()
                    .w(relative(progress.clamp(0.02, 1.)))
                    .bg(p.accent),
            )
            .into_any_element()
    }

    /// The gallery: where you left off, what you read lately, what you have.
    fn home(&self, p: &Palette, cx: &mut gpui::Context<Self>) -> gpui::AnyElement {
        let heading = |text: &str| {
            div()
                .font_family(theme::mono())
                .text_size(self.at(size_of::LABEL))
                .text_color(p.text_faint)
                .mt_10()
                .mb_4()
                .child(text.to_string())
        };
        let recent = self.home_items();
        let documents: Vec<PathBuf> = recent
            .iter()
            .filter_map(|i| match i {
                HomeItem::Document(path) => Some(path.clone()),
                _ => None,
            })
            .collect();
        let collections: Vec<PathBuf> = recent
            .iter()
            .filter_map(|i| match i {
                HomeItem::Collection(path) => Some(path.clone()),
                _ => None,
            })
            .collect();

        let mut page = div()
            .pt_10()
            .child(
                div()
                    .font_family(theme::display())
                    .text_size(self.at(size_of::TITLE * 1.4))
                    .text_color(p.text)
                    .child("baca"),
            )
            .child(
                div()
                    .font_family(theme::body())
                    .text_size(self.at(size_of::BODY))
                    .text_color(p.text_muted)
                    .mt_1()
                    .child("A quiet Markdown reader."),
            );

        // Nothing found and nothing added: say where baca looked, and how to
        // point it somewhere else.
        if documents.is_empty() && collections.is_empty() && self.query.is_empty() {
            return page
                .child(heading("NOTHING HERE YET"))
                .child(
                    div()
                        .font_family(theme::body())
                        .text_size(self.at(size_of::BODY))
                        .line_height(self.at(size_of::BODY_LINE))
                        .text_color(p.text_muted)
                        .max_w(px(520.))
                        .child(
                            "baca looks for Markdown in Documents, Notes, Obsidian, \
                             vault and wiki under your home folder. Point it at a \
                             folder of your own and it will remember.",
                        ),
                )
                .child(
                    div()
                        .id("onboard-choose")
                        .cursor_pointer()
                        .mt_6()
                        .px_4()
                        .py_3()
                        .bg(p.bg_subtle)
                        .border_1()
                        .border_color(p.border_mid)
                        .w(px(220.))
                        .font_family(theme::mono())
                        .text_size(self.at(size_of::SMALL))
                        .text_color(p.accent)
                        .hover(|this| this.bg(p.bg_raised))
                        .on_click(cx.listener(|state, _, _, cx| state.choose(cx)))
                        .child("Choose a folder  Ctrl+O"),
                )
                .into_any_element();
        }

        // The first recent document gets a card of its own: picking up where
        // you left off is the commonest reason to open a reader.
        let mut row = 0;
        if let Some(path) = documents.first().cloned() {
            if let Some(entry) = self.settings.recent.iter().find(|r| r.path == path) {
                let focused = self.cursor == row;
                let open = path.clone();
                page = page.child(heading("CONTINUE READING")).child(
                    div()
                        .id("continue")
                        .cursor_pointer()
                        .p_5()
                        .bg(if focused { p.bg_raised } else { p.bg_subtle })
                        .border_1()
                        .border_color(if focused { p.accent } else { p.border })
                        .hover(|this| this.bg(p.bg_raised))
                        .on_click(cx.listener(move |state, _, _, cx| state.read(open.clone(), cx)))
                        .child(
                            div()
                                .font_family(theme::display())
                                .text_size(self.at(size_of::H2))
                                .text_color(p.text)
                                .child(entry.title.clone()),
                        )
                        .child(self.caption(
                            p,
                            vec![
                                entry.collection.clone(),
                                format!("{}%", (entry.progress * 100.).round() as u32),
                                settings::ago(entry.at),
                            ],
                        ))
                        .child(self.progress_bar(p, entry.progress)),
                );
                row += 1;
            }
        }

        if documents.len() > 1 {
            page =
                page.child(heading("RECENTLY READ")).children(
                    documents
                        .iter()
                        .skip(1)
                        .filter_map(|path| {
                            let entry = self.settings.recent.iter().find(|r| &r.path == path)?;
                            let ix = row;
                            row += 1;
                            let focused = self.cursor == ix;
                            let open = path.clone();
                            Some(
                                div()
                                    .id(("recent", ix))
                                    .cursor_pointer()
                                    .flex()
                                    .items_baseline()
                                    .justify_between()
                                    .gap_4()
                                    .py_3()
                                    .px_3()
                                    .border_t_1()
                                    .border_color(p.border)
                                    .when(focused, |this| this.bg(p.bg_subtle))
                                    .hover(|this| this.bg(p.bg_subtle))
                                    .on_click(cx.listener(move |state, _, _, cx| {
                                        state.read(open.clone(), cx)
                                    }))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .font_family(theme::body())
                                            .text_size(self.at(size_of::H5))
                                            .text_color(p.text)
                                            .child(entry.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .font_family(theme::mono())
                                            .text_size(self.at(size_of::LABEL))
                                            .text_color(p.text_faint)
                                            .child(format!(
                                                "{}  ·  {}",
                                                entry.collection,
                                                settings::ago(entry.at)
                                            )),
                                    ),
                            )
                        })
                        .collect::<Vec<_>>(),
                );
        }

        page = page.child(heading("COLLECTIONS")).child(
            div()
                .flex()
                .flex_wrap()
                .gap_3()
                .children(collections.iter().filter_map(|path| {
                    let found = self.collections.iter().find(|c| &c.path == path)?;
                    let ix = row;
                    row += 1;
                    let focused = self.cursor == ix;
                    let open = path.clone();
                    Some(
                        div()
                            .id(("collection", ix))
                            .cursor_pointer()
                            .w(px(200.))
                            .p_4()
                            .bg(if focused { p.bg_raised } else { p.bg_subtle })
                            .border_1()
                            .border_color(if focused { p.accent } else { p.border })
                            .hover(|this| this.bg(p.bg_raised))
                            .on_click(cx.listener(move |state, _, _, cx| {
                                state.open_collection(open.clone(), cx)
                            }))
                            .child(
                                div()
                                    .font_family(theme::display())
                                    .text_size(self.at(size_of::H4))
                                    .text_color(p.text)
                                    .child(found.name.clone()),
                            )
                            .child(self.caption(
                                p,
                                vec![match found.count {
                                    1 => "1 document".to_string(),
                                    n => format!("{n} documents"),
                                }],
                            )),
                    )
                }))
                .child(
                    div()
                        .id("add-folder")
                        .cursor_pointer()
                        .w(px(200.))
                        .p_4()
                        .border_1()
                        .border_color(p.border_mid)
                        .font_family(theme::mono())
                        .text_size(self.at(size_of::SMALL))
                        .text_color(p.accent)
                        .hover(|this| this.bg(p.bg_subtle))
                        .on_click(cx.listener(|state, _, _, cx| state.choose(cx)))
                        .child("+  Add a folder"),
                ),
        );
        page.into_any_element()
    }

    /// The shelf of documents, filtered by the current query.
    fn shelf(&self, p: &Palette, cx: &mut gpui::Context<Self>) -> gpui::AnyElement {
        let entries = self.visible();
        let summary = if self.query.is_empty() {
            format!("{} Markdown files", entries.len())
        } else {
            format!(
                "{} of {} match “{}”",
                entries.len(),
                self.entries.len(),
                self.query
            )
        };
        div()
            .pt_10()
            .child(
                div()
                    .font_family(theme::display())
                    .text_size(self.at(size_of::TITLE))
                    .text_color(p.text)
                    .child("Your reading shelf"),
            )
            .child(
                div()
                    .font_family(theme::body())
                    .text_size(self.at(size_of::BODY))
                    .text_color(p.text_muted)
                    .mb_8()
                    .child(summary),
            )
            .children(entries.into_iter().enumerate().filter_map(|(row, ix)| {
                let entry = self.entries.get(ix)?;
                let path = entry.path.clone();
                let focused = row == self.cursor;
                Some(
                    div()
                        .id(("entry", row))
                        .cursor_pointer()
                        .py_5()
                        .px_3()
                        .border_t_1()
                        .border_color(p.border)
                        .when(focused, |this| this.bg(p.bg_subtle))
                        .hover(|this| this.bg(p.bg_subtle))
                        .on_click(cx.listener(move |state, _, _, cx| state.read(path.clone(), cx)))
                        .child(
                            div()
                                .font_family(theme::display())
                                .text_size(self.at(size_of::H3))
                                .text_color(p.text)
                                .child(entry.title.clone()),
                        )
                        .child(
                            div()
                                .font_family(theme::body())
                                .text_size(self.at(size_of::H5))
                                .text_color(p.text_muted)
                                .mt_1()
                                .child(entry.preview.clone()),
                        ),
                )
            }))
            .into_any_element()
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
        // Read the last frame's layout before discarding it: it is the only
        // record of where the headings ended up.
        self.measure_active_heading();
        if self.follow_outline {
            if let Some(row) = self.outline_rows().iter().position(|&ix| ix == self.active) {
                self.outline_scroll.scroll_to_item(row);
            }
            self.follow_outline = false;
        }
        // These are re-registered from scratch each frame, in document order,
        // so the indices a selection or a search holds stay meaningful.
        self.pieces.borrow_mut().clear();
        self.heading_pieces.borrow_mut().clear();
        self.hits.borrow_mut().clear();

        let reading = self.view == View::Reading;
        let body = if reading {
            div()
                .pt_10()
                // A document that opens on its own heading already states its
                // title.
                .children((!self.opens_with_heading()).then(|| {
                    div()
                        .font_family(theme::display())
                        .text_size(self.at(size_of::TITLE))
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
        } else if self.view == View::Shelf {
            self.shelf(&p, cx)
        } else {
            self.home(&p, cx)
        };
        let header = self.header(&p, cx);
        div()
            .size_full()
            .bg(p.bg)
            .track_focus(&self.focus)
            .key_context(if self.typing { "BacaSearch" } else { "Baca" })
            .on_key_down(cx.listener(|state, event: &KeyDownEvent, _, cx| {
                state.typed(event, cx);
            }))
            .on_action(cx.listener(|s, _: &ChooseFolder, _, cx| s.choose(cx)))
            .on_action(cx.listener(|s, _: &Reload, _, cx| s.reload(cx)))
            .on_action(cx.listener(|s, _: &CycleTheme, _, cx| s.cycle_theme(cx)))
            .on_action(cx.listener(|s, _: &Library, _, cx| s.go_back(cx)))
            .on_action(cx.listener(|s, _: &CopySelection, _, cx| s.copy_selection(cx)))
            .on_action(cx.listener(|s, _: &SelectAll, _, cx| s.select_all(cx)))
            .on_action(cx.listener(|s, _: &StartSearch, _, cx| s.start_search(cx)))
            .on_action(cx.listener(|s, _: &CancelSearch, _, cx| s.cancel_search(cx)))
            .on_action(cx.listener(|s, _: &AcceptSearch, _, cx| {
                s.typing = false;
                // In a list, committing a search opens what it found.
                if s.view != View::Reading {
                    s.open_focused(cx);
                }
                cx.notify()
            }))
            .on_action(cx.listener(|s, _: &FindNext, _, cx| {
                if s.view == View::Reading {
                    s.step_hit(true, cx)
                } else {
                    s.step_entry(true, cx)
                }
            }))
            .on_action(cx.listener(|s, _: &FindPrev, _, cx| {
                if s.view == View::Reading {
                    s.step_hit(false, cx)
                } else {
                    s.step_entry(false, cx)
                }
            }))
            .on_action(cx.listener(|s, _: &ZoomIn, _, cx| {
                let to = s.scale + 0.1;
                s.zoom(to, cx)
            }))
            .on_action(cx.listener(|s, _: &ZoomOut, _, cx| {
                let to = s.scale - 0.1;
                s.zoom(to, cx)
            }))
            .on_action(cx.listener(|s, _: &ZoomReset, _, cx| s.zoom(1.0, cx)))
            .on_action(cx.listener(|s, _: &LineDown, _, cx| {
                let step = s.at(size_of::BODY_LINE) * 3.;
                s.scroll_by(step, cx)
            }))
            .on_action(cx.listener(|s, _: &LineUp, _, cx| {
                let step = s.at(size_of::BODY_LINE) * 3.;
                s.scroll_by(-step, cx)
            }))
            .on_action(cx.listener(|s, _: &PageDown, _, cx| {
                let step = s.page();
                s.scroll_by(step, cx)
            }))
            .on_action(cx.listener(|s, _: &PageUp, _, cx| {
                let step = s.page();
                s.scroll_by(-step, cx)
            }))
            .on_action(cx.listener(|s, _: &GoTop, _, cx| s.go_top(cx)))
            .on_action(cx.listener(|s, _: &GoBottom, _, cx| s.go_bottom(cx)))
            .on_action(cx.listener(|s, _: &ToggleOutline, _, cx| {
                s.outline = !s.outline;
                s.remember();
                cx.notify()
            }))
            // In the shelf the arrows walk the list; in a document they scroll.
            .on_action(cx.listener(|s, _: &NextEntry, _, cx| {
                if s.view == View::Reading {
                    let step = s.at(size_of::BODY_LINE) * 3.;
                    s.scroll_by(step, cx)
                } else {
                    s.step_entry(true, cx)
                }
            }))
            .on_action(cx.listener(|s, _: &PrevEntry, _, cx| {
                if s.view == View::Reading {
                    let step = s.at(size_of::BODY_LINE) * 3.;
                    s.scroll_by(-step, cx)
                } else {
                    s.step_entry(false, cx)
                }
            }))
            .on_action(cx.listener(|s, _: &OpenEntry, _, cx| {
                if s.view != View::Reading {
                    s.open_focused(cx)
                }
            }))
            .child(
                div()
                    .h_full()
                    .flex()
                    // The gallery is its own full-width screen; a sidebar
                    // listing one document's headings has nothing to say there.
                    .children((self.view != View::Home).then(|| self.sidebar(&p, cx)))
                    .child(
                        div()
                            .flex_1()
                            // Without this the page's own width wins and squeezes
                            // the sidebar instead of scrolling.
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            // The header stays put: a search tally is no use
                            // if stepping through matches scrolls it away.
                            .child(
                                div()
                                    .w_full()
                                    .flex_none()
                                    .flex()
                                    .justify_center()
                                    .child(div().w_full().max_w(COLUMN).px_10().child(header)),
                            )
                            .child(
                                div()
                                    .id("scroll")
                                    .flex_1()
                                    .min_h_0()
                                    // Vertical only: prose that scrolls sideways
                                    // has simply failed to wrap.
                                    .overflow_y_scroll()
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
                                        div().w_full().flex().justify_center().child(
                                            div()
                                                .w_full()
                                                .max_w(COLUMN)
                                                .px_10()
                                                .pb_16()
                                                .pt_2()
                                                .child(body),
                                        ),
                                    ),
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
    let mut settings = Settings::load();
    // A folder named on the command line is as deliberate as choosing one in
    // the app, so it joins the gallery rather than applying just this once.
    if let Some(root) = root {
        if !settings.folders.contains(&root) {
            settings.folders.push(root.clone());
        }
        settings.root = Some(root);
    }
    gpui_platform::application().run(move |cx: &mut App| {
        theme::load_fonts(cx);
        cx.bind_keys([
            // Available whether or not the search field has the keyboard.
            KeyBinding::new("ctrl-o", ChooseFolder, None),
            KeyBinding::new("ctrl-r", Reload, None),
            KeyBinding::new("ctrl-t", CycleTheme, None),
            KeyBinding::new("ctrl-c", CopySelection, None),
            KeyBinding::new("ctrl-a", SelectAll, None),
            KeyBinding::new("ctrl-f", StartSearch, None),
            KeyBinding::new("ctrl-b", ToggleOutline, None),
            KeyBinding::new("ctrl-=", ZoomIn, None),
            KeyBinding::new("ctrl-+", ZoomIn, None),
            KeyBinding::new("ctrl--", ZoomOut, None),
            KeyBinding::new("ctrl-0", ZoomReset, None),
            // Typing into the search field must not also scroll the page, so
            // the bare keys live in the reading context only.
            KeyBinding::new("escape", Library, Some("Baca")),
            KeyBinding::new("alt-left", Library, Some("Baca")),
            KeyBinding::new("j", LineDown, Some("Baca")),
            KeyBinding::new("k", LineUp, Some("Baca")),
            KeyBinding::new("down", NextEntry, Some("Baca")),
            KeyBinding::new("up", PrevEntry, Some("Baca")),
            KeyBinding::new("enter", OpenEntry, Some("Baca")),
            KeyBinding::new("space", PageDown, Some("Baca")),
            KeyBinding::new("shift-space", PageUp, Some("Baca")),
            KeyBinding::new("pagedown", PageDown, Some("Baca")),
            KeyBinding::new("pageup", PageUp, Some("Baca")),
            KeyBinding::new("g", GoTop, Some("Baca")),
            KeyBinding::new("shift-g", GoBottom, Some("Baca")),
            KeyBinding::new("home", GoTop, Some("Baca")),
            KeyBinding::new("end", GoBottom, Some("Baca")),
            KeyBinding::new("n", FindNext, Some("Baca")),
            KeyBinding::new("shift-n", FindPrev, Some("Baca")),
            // While searching, the same keys steer the search instead.
            KeyBinding::new("escape", CancelSearch, Some("BacaSearch")),
            KeyBinding::new("enter", AcceptSearch, Some("BacaSearch")),
            KeyBinding::new("down", FindNext, Some("BacaSearch")),
            KeyBinding::new("up", FindPrev, Some("BacaSearch")),
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
            move |window, cx| {
                let dark = matches!(
                    window.appearance(),
                    gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
                );
                let baca = cx.new(|cx| {
                    let mut state = Baca::new(settings, cx);
                    state.apply_system_theme(dark);
                    // An explicit argument outranks whatever was last open.
                    if let Some(file) = file {
                        state.read(file, cx);
                    }
                    state
                });

                let watcher = baca.clone();
                let appearance = window.observe_window_appearance(move |window, cx| {
                    let dark = matches!(
                        window.appearance(),
                        gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
                    );
                    watcher.update(cx, |state, cx| {
                        state.apply_system_theme(dark);
                        cx.notify()
                    });
                });

                let quitting = baca.clone();
                let quit = cx.on_app_quit(move |cx| {
                    quitting.update(cx, |state, _| state.remember());
                    async {}
                });

                baca.update(cx, |state, _| {
                    state._subscriptions.push(appearance);
                    state._subscriptions.push(quit);
                });

                let focus = baca.read(cx).focus.clone();
                window.focus(&focus, cx);
                baca
            },
        )
        .unwrap();
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_is_kept_out_of_the_shelf() {
        let (title, preview) = summarize(
            "---\ntitle: A Note\ntags: x\n---\n\n# Ignored heading\n\nThe body.\n",
            "file.md",
        );
        assert_eq!(title, "A Note");
        assert_eq!(preview, "Ignored heading");
    }

    #[test]
    fn a_plain_document_uses_its_heading() {
        let (title, preview) = summarize("# Heading\n\nFirst **line**.\n", "file.md");
        assert_eq!(title, "Heading");
        assert_eq!(preview, "First line.");
    }

    #[test]
    fn front_matter_without_a_title_falls_back_to_the_heading() {
        let (title, preview) = summarize("---\ntags: x\n---\n\n# Heading\n\nBody.\n", "f.md");
        assert_eq!(title, "Heading");
        assert_eq!(preview, "Body.");
    }

    #[test]
    fn an_opening_rule_is_not_front_matter() {
        let (meta, body) = split_front_matter("---\n\nJust a rule above.\n");
        assert_eq!(meta, "");
        assert!(body.contains("Just a rule above."));
    }

    #[test]
    fn a_preview_reads_as_prose_not_source() {
        assert_eq!(strip_markup("![A figure](figure.png)"), "A figure");
        assert_eq!(
            strip_markup("See [the docs](http://x/y) now"),
            "See the docs now"
        );
        assert_eq!(strip_markup("**bold** and `code`"), "bold and code");
        assert_eq!(strip_markup("plain text"), "plain text");
        // An unpaired bracket is literal text in Markdown, so it stays.
        assert_eq!(strip_markup("an [unclosed link"), "an [unclosed link");
    }

    #[test]
    fn an_empty_document_falls_back_to_its_filename() {
        let (title, preview) = summarize("", "notes.md");
        assert_eq!(title, "notes.md");
        assert_eq!(preview, "No preview available.");
    }

    fn cells(texts: &[&str]) -> Vec<markdown::Text> {
        texts
            .iter()
            .map(|t| {
                markdown::parse(t).blocks.first().map_or_else(
                    markdown::Text::default,
                    |b| match b {
                        markdown::Block::Paragraph(text) => text.clone(),
                        _ => markdown::Text::default(),
                    },
                )
            })
            .collect()
    }

    fn outline_of(levels: &[u32]) -> Vec<Heading> {
        levels
            .iter()
            .enumerate()
            .map(|(ix, level)| {
                let level = match level {
                    1 => HeadingLevel::H1,
                    2 => HeadingLevel::H2,
                    3 => HeadingLevel::H3,
                    _ => HeadingLevel::H4,
                };
                (level, format!("h{ix}"), ix.to_string())
            })
            .collect()
    }

    #[test]
    fn folding_a_heading_hides_what_is_nested_under_it() {
        //  0 #   1 ##   2 ###   3 ###   4 ##   5 #
        let outline = outline_of(&[1, 2, 3, 3, 2, 1]);
        let collapsed = HashSet::from([1]);
        assert_eq!(
            visible_outline(&outline, &collapsed),
            vec![0, 1, 4, 5],
            "the two H3s under the folded H2 go, its sibling and the next H1 stay"
        );
    }

    #[test]
    fn folding_a_top_heading_hides_the_whole_branch() {
        let outline = outline_of(&[1, 2, 3, 1]);
        assert_eq!(visible_outline(&outline, &HashSet::from([0])), vec![0, 3]);
    }

    #[test]
    fn nothing_folded_shows_everything() {
        let outline = outline_of(&[1, 2, 3, 2]);
        assert_eq!(visible_outline(&outline, &HashSet::new()), vec![0, 1, 2, 3]);
    }

    #[test]
    fn folding_a_leaf_changes_nothing() {
        let outline = outline_of(&[1, 2, 2]);
        assert_eq!(
            visible_outline(&outline, &HashSet::from([2])),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn only_headings_with_something_below_can_fold() {
        let outline = outline_of(&[1, 2, 2, 1]);
        assert!(has_children(&outline, 0), "H1 with H2s under it");
        assert!(!has_children(&outline, 1), "H2 followed by a sibling");
        assert!(!has_children(&outline, 2), "H2 followed by an H1");
        assert!(!has_children(&outline, 3), "the last entry");
        assert!(!has_children(&outline, 99), "out of range");
    }

    #[test]
    fn a_terse_column_yields_room_to_a_wordy_one() {
        let head = cells(&["Feature", "Yes"]);
        let rows = vec![cells(&[
            "A long description of what this row is actually about",
            "no",
        ])];
        let widths = column_widths(&head, &rows);
        assert!(
            widths[0] > widths[1],
            "prose column should be wider: {widths:?}"
        );
        assert!((widths.iter().sum::<f32>() - 1.).abs() < 0.001);
    }

    #[test]
    fn a_narrow_column_keeps_a_readable_minimum() {
        let head = cells(&["x", "Description"]);
        let rows = vec![cells(&["1", &"word ".repeat(40)])];
        let widths = column_widths(&head, &rows);
        assert!(
            widths[0] >= 0.2,
            "one long column must not crush the other: {widths:?}"
        );
    }

    #[test]
    fn identical_columns_are_even() {
        let head = cells(&["One", "Two", "Six"]);
        let rows = vec![cells(&["aaa", "bbb", "ccc"])];
        let widths = column_widths(&head, &rows);
        for w in &widths {
            assert!((w - 1. / 3.).abs() < 0.001, "{widths:?}");
        }
    }

    #[test]
    fn a_couple_of_characters_barely_move_a_column() {
        let head = cells(&["One", "Two", "Three"]);
        let rows = vec![cells(&["aaa", "bbb", "ccc"])];
        let widths = column_widths(&head, &rows);
        for w in &widths {
            assert!((w - 1. / 3.).abs() < 0.02, "{widths:?}");
        }
    }

    #[test]
    fn a_table_with_no_columns_is_harmless() {
        assert!(column_widths(&[], &[]).is_empty());
    }

    #[test]
    fn filtering_matches_title_path_and_body() {
        let entry = Entry {
            path: PathBuf::from("/vault/deep/note.md"),
            title: "Rust".into(),
            preview: String::new(),
            haystack: "rust\n/vault/deep/note.md\nabout ownership".to_lowercase(),
        };
        assert!(entry.matches("rust"));
        assert!(entry.matches("ownership"), "body text is searchable");
        assert!(entry.matches("deep"), "the path is searchable");
        assert!(entry.matches(""), "an empty query matches everything");
        assert!(!entry.matches("python"));
    }
}
