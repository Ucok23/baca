use pulldown_cmark::{
    Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};

/// The typographic variations a run of text can carry. These compose, so a span
/// can be bold and italic and struck through at once.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub strong: bool,
    pub emphasis: bool,
    pub strikethrough: bool,
    pub code: bool,
}

/// A run of text sharing one style and one link target.
#[derive(Clone)]
pub struct Span {
    pub text: String,
    pub style: Style,
    pub link: Option<String>,
}

/// Inline content: the styled runs that make up a paragraph, heading or cell.
#[derive(Clone, Default)]
pub struct Text {
    pub spans: Vec<Span>,
}

impl Text {
    pub fn is_empty(&self) -> bool {
        self.spans.iter().all(|s| s.text.is_empty())
    }

    pub fn plain(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
}

/// What introduces a list item. `None` on a block means the item continues one
/// already marked, so the bullet is not repeated.
#[derive(Clone, Copy, Debug)]
pub enum Marker {
    Bullet,
    Ordinal(u64),
    Task(bool),
}

#[derive(Clone)]
pub enum Block {
    Heading {
        level: HeadingLevel,
        text: Text,
    },
    Paragraph(Text),
    Quote(Text),
    Code {
        lang: Option<String>,
        text: String,
    },
    Item {
        marker: Option<Marker>,
        depth: usize,
        text: Text,
    },
    Table {
        aligns: Vec<Alignment>,
        head: Vec<Text>,
        rows: Vec<Vec<Text>>,
    },
    Image {
        url: String,
        alt: String,
    },
    Footnote {
        label: String,
        text: Text,
    },
    Rule,
}

#[derive(Clone, Default)]
pub struct Document {
    pub blocks: Vec<Block>,
    pub outline: Vec<(HeadingLevel, String, String)>,
}

/// The link scheme a footnote reference carries, so the renderer can tell one
/// apart from an ordinary link without a second field on `Span`.
pub const FOOTNOTE_SCHEME: &str = "baca-footnote:";

#[derive(Default)]
struct TableState {
    aligns: Vec<Alignment>,
    head: Vec<Text>,
    rows: Vec<Vec<Text>>,
    row: Vec<Text>,
    in_head: bool,
}

#[derive(Default)]
struct Builder {
    doc: Document,
    spans: Vec<Span>,
    strong: u32,
    emphasis: u32,
    strikethrough: u32,
    link: Option<String>,
    heading: Option<(HeadingLevel, String)>,
    quote: u32,
    code: Option<Option<String>>,
    lists: Vec<Option<u64>>,
    marker: Option<Marker>,
    task: Option<bool>,
    image: Option<(String, usize)>,
    table: Option<TableState>,
    footnote: Option<String>,
}

impl Builder {
    fn style(&self) -> Style {
        Style {
            strong: self.strong > 0,
            emphasis: self.emphasis > 0,
            strikethrough: self.strikethrough > 0,
            code: false,
        }
    }

    fn push(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        match self.spans.last_mut() {
            Some(last) if last.style == style && last.link == self.link => {
                last.text.push_str(text)
            }
            _ => self.spans.push(Span {
                text: text.into(),
                style,
                link: self.link.clone(),
            }),
        }
    }

    /// Take the pending inline runs, trimmed of the whitespace Markdown leaves
    /// at the edges of a block.
    fn take(&mut self) -> Text {
        let mut spans = std::mem::take(&mut self.spans);
        while let Some(first) = spans.first_mut() {
            first.text = first.text.trim_start().into();
            if first.text.is_empty() {
                spans.remove(0);
            } else {
                break;
            }
        }
        while let Some(last) = spans.last_mut() {
            last.text = last.text.trim_end().into();
            if last.text.is_empty() {
                spans.pop();
            } else {
                break;
            }
        }
        Text { spans }
    }

    /// Close whatever inline run is open and file it under the container it
    /// belongs to.
    fn flush(&mut self) {
        if let Some(table) = &mut self.table {
            return_cell(table, &mut self.spans);
            return;
        }
        let text = self.take();
        if text.is_empty() {
            return;
        }
        if let Some((level, anchor)) = self.heading.take() {
            self.doc.outline.push((level, text.plain(), anchor));
            self.doc.blocks.push(Block::Heading { level, text });
        } else if let Some(label) = &self.footnote {
            let label = label.clone();
            self.doc.blocks.push(Block::Footnote { label, text });
        } else if !self.lists.is_empty() {
            self.doc.blocks.push(Block::Item {
                marker: self.marker.take(),
                depth: self.lists.len().saturating_sub(1),
                text,
            });
        } else if self.quote > 0 {
            self.doc.blocks.push(Block::Quote(text));
        } else {
            self.doc.blocks.push(Block::Paragraph(text));
        }
    }
}

/// Inside a table the inline buffer belongs to the current cell, not to a block.
fn return_cell(table: &mut TableState, spans: &mut Vec<Span>) {
    let text = Text {
        spans: std::mem::take(spans),
    };
    if table.in_head {
        table.head.push(text);
    } else {
        table.row.push(text);
    }
}

/// Rewrite `[[Note]]` and `[[Note|label]]` into ordinary Markdown links so the
/// parser sees them, leaving anything inside code spans and fenced blocks
/// exactly as written.
fn expand_wikilinks(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut fenced = false;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            out.push_str(line);
            continue;
        }
        if fenced {
            out.push_str(line);
            continue;
        }
        let mut rest = line;
        let mut ticks = false;
        while let Some(ix) = rest.find(['`', '[']) {
            let (head, tail) = rest.split_at(ix);
            out.push_str(head);
            if let Some(after) = tail.strip_prefix('`') {
                ticks = !ticks;
                out.push('`');
                rest = after;
                continue;
            }
            match tail.strip_prefix("[[").and_then(|t| t.split_once("]]")) {
                Some((inner, tail)) if !ticks && !inner.contains('[') => {
                    let (target, label) = match inner.split_once('|') {
                        Some((t, l)) => (t.trim(), l.trim()),
                        None => (inner.trim(), inner.trim()),
                    };
                    // A destination containing spaces is only valid inside
                    // angle brackets.
                    if target.contains(|c: char| c.is_whitespace()) && !target.contains('>') {
                        out.push_str(&format!("[{label}](<{target}>)"));
                    } else {
                        out.push_str(&format!("[{label}]({target})"));
                    }
                    rest = tail;
                }
                _ => {
                    out.push('[');
                    rest = &tail[1..];
                }
            }
        }
        out.push_str(rest);
    }
    out
}

pub fn parse(source: &str) -> Document {
    let source = expand_wikilinks(source);
    let source = source.as_str();
    let options = Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_HEADING_ATTRIBUTES;
    let mut b = Builder::default();
    let mut code = String::new();

    for event in Parser::new_ext(source, options) {
        match event {
            Event::Start(Tag::Heading { level, id, .. }) => {
                b.flush();
                let anchor = id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| b.doc.outline.len().to_string());
                b.heading = Some((level, anchor));
            }
            Event::End(TagEnd::Heading(_)) => b.flush(),

            Event::Start(Tag::Paragraph) => b.flush(),
            Event::End(TagEnd::Paragraph) => b.flush(),

            Event::Start(Tag::BlockQuote(_)) => {
                b.flush();
                b.quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                b.flush();
                b.quote = b.quote.saturating_sub(1);
            }

            Event::Start(Tag::CodeBlock(kind)) => {
                b.flush();
                code.clear();
                b.code = Some(match kind {
                    CodeBlockKind::Fenced(lang) if !lang.is_empty() => Some(lang.to_string()),
                    _ => None,
                });
            }
            Event::End(TagEnd::CodeBlock) => {
                let lang = b.code.take().flatten();
                let text = std::mem::take(&mut code).trim_end().to_string();
                if !text.is_empty() {
                    b.doc.blocks.push(Block::Code { lang, text });
                }
            }

            Event::Start(Tag::List(start)) => {
                b.flush();
                b.lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                b.flush();
                b.lists.pop();
            }
            Event::Start(Tag::Item) => {
                b.flush();
                let ordinal = b.lists.last().copied().flatten();
                if let Some(n) = ordinal {
                    if let Some(slot) = b.lists.last_mut() {
                        *slot = Some(n + 1);
                    }
                }
                b.marker = Some(match (b.task.take(), ordinal) {
                    (Some(done), _) => Marker::Task(done),
                    (None, Some(n)) => Marker::Ordinal(n),
                    (None, None) => Marker::Bullet,
                });
            }
            Event::End(TagEnd::Item) => b.flush(),
            // The marker arrives after `Start(Item)`, so upgrade the pending one.
            Event::TaskListMarker(done) => b.marker = Some(Marker::Task(done)),

            Event::Start(Tag::Table(aligns)) => {
                b.flush();
                b.table = Some(TableState {
                    aligns,
                    ..Default::default()
                });
            }
            Event::End(TagEnd::Table) => {
                if let Some(t) = b.table.take() {
                    b.doc.blocks.push(Block::Table {
                        aligns: t.aligns,
                        head: t.head,
                        rows: t.rows,
                    });
                }
            }
            Event::Start(Tag::TableHead) => {
                if let Some(t) = &mut b.table {
                    t.in_head = true
                }
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(t) = &mut b.table {
                    t.in_head = false
                }
            }
            Event::End(TagEnd::TableRow) => {
                if let Some(t) = &mut b.table {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(t) = &mut b.table {
                    return_cell(t, &mut b.spans)
                }
            }

            Event::Start(Tag::Link { dest_url, .. }) => b.link = Some(dest_url.to_string()),
            Event::End(TagEnd::Link) => b.link = None,

            Event::Start(Tag::Image { dest_url, .. }) => {
                b.image = Some((dest_url.to_string(), b.spans.len()))
            }
            Event::End(TagEnd::Image) => {
                if let Some((url, start)) = b.image.take() {
                    let alt: String = b.spans.drain(start..).map(|s| s.text).collect();
                    // An image ends whatever paragraph wrapped it and stands alone.
                    b.flush();
                    b.doc.blocks.push(Block::Image { url, alt });
                }
            }

            Event::Start(Tag::FootnoteDefinition(label)) => {
                b.flush();
                b.footnote = Some(label.to_string());
            }
            Event::End(TagEnd::FootnoteDefinition) => {
                b.flush();
                b.footnote = None;
            }
            Event::FootnoteReference(label) => {
                let style = b.style();
                let restore = b.link.take();
                b.link = Some(format!("{FOOTNOTE_SCHEME}{label}"));
                b.push(&format!("[{label}]"), style);
                b.link = restore;
            }

            Event::Start(Tag::Strong) => b.strong += 1,
            Event::End(TagEnd::Strong) => b.strong = b.strong.saturating_sub(1),
            Event::Start(Tag::Emphasis) => b.emphasis += 1,
            Event::End(TagEnd::Emphasis) => b.emphasis = b.emphasis.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => b.strikethrough += 1,
            Event::End(TagEnd::Strikethrough) => {
                b.strikethrough = b.strikethrough.saturating_sub(1)
            }

            Event::Text(value) => {
                if b.code.is_some() {
                    code.push_str(&value);
                } else {
                    let style = b.style();
                    b.push(&value, style);
                }
            }
            Event::Code(value) => {
                let style = Style {
                    code: true,
                    ..b.style()
                };
                b.push(&value, style);
            }
            Event::SoftBreak => {
                let style = b.style();
                b.push(" ", style);
            }
            Event::HardBreak => {
                let style = b.style();
                b.push("\n", style);
            }
            Event::Rule => {
                b.flush();
                b.doc.blocks.push(Block::Rule);
            }
            // Raw HTML is not rendered, but a lone `<br>` is a line break
            // people actually rely on in Markdown notes.
            Event::Html(value) | Event::InlineHtml(value)
                if value.trim().starts_with("<br") =>
            {
                let style = b.style();
                b.push("\n", style);
            }
            _ => {}
        }
    }
    b.flush();
    b.doc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(source: &str) -> Vec<Block> {
        parse(source).blocks
    }

    /// The spans of the first block, as `(text, bold, italic, code, link)`.
    fn spans(source: &str) -> Vec<(String, bool, bool, bool, Option<String>)> {
        match &blocks(source)[0] {
            Block::Paragraph(t) | Block::Heading { text: t, .. } => t
                .spans
                .iter()
                .map(|s| {
                    (
                        s.text.clone(),
                        s.style.strong,
                        s.style.emphasis,
                        s.style.code,
                        s.link.clone(),
                    )
                })
                .collect(),
            _ => panic!("expected inline content"),
        }
    }

    #[test]
    fn inline_styles_are_kept_apart() {
        let got = spans("plain **bold** *italic* `code`");
        assert_eq!(got[0].0, "plain ");
        assert_eq!((got[1].0.as_str(), got[1].1), ("bold", true));
        assert_eq!((got[3].0.as_str(), got[3].2), ("italic", true));
        assert_eq!((got[5].0.as_str(), got[5].3), ("code", true));
    }

    #[test]
    fn nested_emphasis_composes() {
        let got = spans("***both***");
        assert_eq!(got.len(), 1);
        assert!(got[0].1 && got[0].2, "expected bold and italic at once");
    }

    #[test]
    fn links_carry_their_target() {
        let got = spans("see [the docs](https://example.com/a) now");
        assert_eq!(got[1].0, "the docs");
        assert_eq!(got[1].4.as_deref(), Some("https://example.com/a"));
        assert_eq!(got[2].4, None);
    }

    #[test]
    fn wikilinks_become_links() {
        let got = spans("go to [[Some Note]] and [[other|that one]]");
        assert_eq!(got[1].0, "Some Note");
        assert_eq!(got[1].4.as_deref(), Some("Some Note"));
        assert_eq!(got[3].0, "that one");
        assert_eq!(got[3].4.as_deref(), Some("other"));
    }

    #[test]
    fn wikilinks_inside_code_are_left_alone() {
        let got = spans("literal `[[not a link]]` here");
        assert_eq!(got[1].0, "[[not a link]]");
        assert!(got[1].3 && got[1].4.is_none());
        let fenced = blocks("```\n[[keep me]]\n```");
        assert!(matches!(&fenced[0], Block::Code { text, .. } if text == "[[keep me]]"));
    }

    #[test]
    fn code_blocks_keep_their_language() {
        let got = blocks("```rust\nfn main() {}\n```");
        match &got[0] {
            Block::Code { lang, text } => {
                assert_eq!(lang.as_deref(), Some("rust"));
                assert_eq!(text, "fn main() {}");
            }
            _ => panic!("expected a code block"),
        }
    }

    #[test]
    fn ordered_and_nested_lists_keep_markers_and_depth() {
        let got = blocks("1. first\n2. second\n   - inner\n");
        let markers: Vec<_> = got
            .iter()
            .filter_map(|b| match b {
                Block::Item {
                    marker,
                    depth,
                    text,
                } => Some((text.plain(), *depth, marker.map(|m| format!("{m:?}")))),
                _ => None,
            })
            .collect();
        assert_eq!(markers[0].0, "first");
        assert_eq!(markers[0].1, 0);
        assert!(markers[0].2.as_deref().unwrap().contains("Ordinal(1)"));
        assert!(markers[1].2.as_deref().unwrap().contains("Ordinal(2)"));
        assert_eq!(markers[2].0, "inner");
        assert_eq!(markers[2].1, 1, "nested item should sit one level deeper");
    }

    #[test]
    fn task_lists_record_their_state() {
        let got = blocks("- [ ] todo\n- [x] done\n");
        let states: Vec<_> = got
            .iter()
            .filter_map(|b| match b {
                Block::Item { marker, .. } => marker.map(|m| format!("{m:?}")),
                _ => None,
            })
            .collect();
        assert!(states[0].contains("Task(false)"));
        assert!(states[1].contains("Task(true)"));
    }

    #[test]
    fn tables_split_head_from_body() {
        let got = blocks("| a | b |\n|---|--:|\n| 1 | 2 |\n| 3 | 4 |\n");
        match &got[0] {
            Block::Table { aligns, head, rows } => {
                assert_eq!(head.len(), 2);
                assert_eq!(head[0].plain(), "a");
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[1][1].plain(), "4");
                assert!(matches!(aligns[1], Alignment::Right));
            }
            other => panic!("expected a table, got {}", name(other)),
        }
    }

    #[test]
    fn footnotes_are_collected() {
        let doc = blocks("text[^a]\n\n[^a]: the note\n");
        let reference = match &doc[0] {
            Block::Paragraph(t) => t.spans.last().unwrap().link.clone(),
            _ => panic!("expected a paragraph"),
        };
        assert_eq!(reference.as_deref(), Some("baca-footnote:a"));
        assert!(doc
            .iter()
            .any(|b| matches!(b, Block::Footnote { label, text } if label == "a" && text.plain() == "the note")));
    }

    #[test]
    fn images_become_their_own_block() {
        let got = blocks("![a cat](cat.png)");
        assert!(matches!(&got[0], Block::Image { url, alt } if url == "cat.png" && alt == "a cat"));
    }

    #[test]
    fn soft_breaks_do_not_glue_words_together() {
        let got = spans("one\ntwo");
        assert_eq!(got[0].0, "one two");
    }

    fn name(b: &Block) -> &'static str {
        match b {
            Block::Heading { .. } => "heading",
            Block::Paragraph(_) => "paragraph",
            Block::Quote(_) => "quote",
            Block::Code { .. } => "code",
            Block::Item { .. } => "item",
            Block::Table { .. } => "table",
            Block::Image { .. } => "image",
            Block::Footnote { .. } => "footnote",
            Block::Rule => "rule",
        }
    }
}
