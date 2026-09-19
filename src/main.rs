mod markdown;
mod theme;
use gpui::{
    actions, div, prelude::*, px, size, App, Bounds, KeyBinding, PathPromptOptions, Render,
    ScrollHandle, StatefulInteractiveElement, WindowBounds, WindowOptions,
};
use markdown::Block;
use std::path::{Path, PathBuf};
use theme::Theme;
actions!(baca, [ChooseFolder, Reload, CycleTheme, Library]);
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
            } else if matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("md" | "markdown" | "mdx")
            ) {
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
struct Baca {
    root: Option<PathBuf>,
    entries: Vec<Entry>,
    selected: Option<Entry>,
    doc: markdown::Document,
    theme: Theme,
    focus: gpui::FocusHandle,
    scroll: ScrollHandle,
}
impl Baca {
    fn new(root: Option<PathBuf>, cx: &mut gpui::Context<Self>) -> Self {
        let entries = root.as_deref().map(scan).unwrap_or_default();
        Self {
            root,
            entries,
            selected: None,
            doc: Default::default(),
            theme: Theme::Paper,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        }
    }
    fn reload(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(r) = &self.root {
            self.entries = scan(r)
        }
        cx.notify()
    }
    fn open(&mut self, e: Entry, cx: &mut gpui::Context<Self>) {
        self.doc = markdown::parse(&std::fs::read_to_string(&e.path).unwrap_or_default());
        self.selected = Some(e);
        self.scroll.scroll_to_item(0);
        cx.notify()
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
                    s.selected = None;
                    s.doc = Default::default();
                    s.reload(cx)
                }
            });
        })
        .detach()
    }
    fn block(&self, b: &Block, p: &theme::Palette) -> gpui::AnyElement {
        match b {
            Block::Heading { text, .. } => div()
                .font_family(theme::FONT_DISPLAY)
                .text_2xl()
                .text_color(p.text)
                .mt_7()
                .mb_2()
                .child(text.clone())
                .into_any_element(),
            Block::Paragraph(t) => div()
                .font_family(theme::FONT_BODY)
                .text_lg()
                .line_height(px(29.))
                .text_color(p.text)
                .mb_4()
                .child(t.clone())
                .into_any_element(),
            Block::Quote(t) => div()
                .font_family(theme::FONT_BODY)
                .text_lg()
                .text_color(p.text_muted)
                .pl_5()
                .my_5()
                .border_l_2()
                .border_color(p.accent)
                .child(t.clone())
                .into_any_element(),
            Block::Code(t) => div()
                .font_family(theme::FONT_MONO)
                .text_sm()
                .text_color(p.text)
                .p_4()
                .my_5()
                .bg(p.code)
                .child(t.clone())
                .into_any_element(),
            Block::Item { text, .. } => div()
                .font_family(theme::FONT_BODY)
                .text_lg()
                .text_color(p.text)
                .mb_2()
                .child(format!("• {text}"))
                .into_any_element(),
            Block::Rule => div().h(px(1.)).my_7().bg(p.border_mid).into_any_element(),
        }
    }
}
impl Render for Baca {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let es = self.entries.clone();
        let sel = self.selected.clone();
        let bs = self.doc.blocks.clone();
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
                s.selected = None;
                cx.notify()
            }))
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
                                    .font_family(theme::FONT_DISPLAY)
                                    .text_xl()
                                    .text_color(p.text)
                                    .child("baca"),
                            )
                            .child(
                                div()
                                    .font_family(theme::FONT_MONO)
                                    .text_xs()
                                    .text_color(p.text_faint)
                                    .mt_1()
                                    .child("MARKDOWN LIBRARY"),
                            )
                            .child(
                                div()
                                    .font_family(theme::FONT_BODY)
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
                                                    .font_family(theme::FONT_MONO)
                                                    .text_xs()
                                                    .text_color(p.accent)
                                                    .child("Choose folder  Ctrl+O"),
                                            )
                                            .child(
                                                div()
                                                    .font_family(theme::FONT_MONO)
                                                    .text_xs()
                                                    .text_color(p.accent)
                                                    .child(format!(
                                                        "{}  Ctrl+T",
                                                        self.theme.label()
                                                    )),
                                            ),
                                    )
                                    .child(if let Some(e) = sel {
                                        div()
                                            .pt_10()
                                            .child(
                                                div()
                                                    .font_family(theme::FONT_DISPLAY)
                                                    .text_3xl()
                                                    .text_color(p.text)
                                                    .child(e.title),
                                            )
                                            .children(bs.iter().map(|b| self.block(b, &p)))
                                            .into_any_element()
                                    } else {
                                        div()
                                            .pt_10()
                                            .child(
                                                div()
                                                    .font_family(theme::FONT_DISPLAY)
                                                    .text_3xl()
                                                    .text_color(p.text)
                                                    .child("Your reading shelf"),
                                            )
                                            .child(
                                                div()
                                                    .font_family(theme::FONT_BODY)
                                                    .text_lg()
                                                    .text_color(p.text_muted)
                                                    .mb_8()
                                                    .child(format!("{} Markdown files", es.len())),
                                            )
                                            .children(es.into_iter().enumerate().map(|(i, e)| {
                                                let x = e.clone();
                                                div()
                                                    .id(format!("e{i}"))
                                                    .cursor_pointer()
                                                    .py_5()
                                                    .border_t_1()
                                                    .border_color(p.border)
                                                    .on_click(cx.listener(move |s, _, _, cx| {
                                                        s.open(x.clone(), cx)
                                                    }))
                                                    .child(
                                                        div()
                                                            .font_family(theme::FONT_DISPLAY)
                                                            .text_xl()
                                                            .text_color(p.text)
                                                            .child(e.title),
                                                    )
                                                    .child(
                                                        div()
                                                            .font_family(theme::FONT_BODY)
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
    let a: Vec<String> = std::env::args().collect();
    let r = a.get(1).map(PathBuf::from).filter(|x| x.is_dir());
    gpui_platform::application().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("ctrl-o", ChooseFolder, Some("Baca")),
            KeyBinding::new("ctrl-r", Reload, Some("Baca")),
            KeyBinding::new("ctrl-t", CycleTheme, Some("Baca")),
            KeyBinding::new("alt-left", Library, Some("Baca")),
        ]);
        let b = Bounds::centered(None, size(px(1100.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(b)),
                ..Default::default()
            },
            move |w, cx| {
                let x = cx.new(|cx| Baca::new(r, cx));
                let f = x.read(cx).focus.clone();
                w.focus(&f, cx);
                x
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
