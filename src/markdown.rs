use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Clone)]
pub enum Block {
    Heading {
        level: HeadingLevel,
        text: String,
    },
    Paragraph(String),
    Quote(String),
    Code(String),
    Item {
        ordinal: Option<u64>,
        depth: usize,
        text: String,
    },
    Rule,
}

#[derive(Clone, Default)]
pub struct Document {
    pub blocks: Vec<Block>,
    pub outline: Vec<(HeadingLevel, String)>,
}

pub fn parse(source: &str) -> Document {
    let mut doc = Document::default();
    let mut text = String::new();
    let mut heading: Option<HeadingLevel> = None;
    let mut quote = false;
    let mut code = false;
    let mut item = false;
    let mut lists: Vec<Option<u64>> = Vec::new();

    let flush = |doc: &mut Document,
                 text: &mut String,
                 heading: Option<HeadingLevel>,
                 quote: bool,
                 code: bool,
                 item: bool,
                 lists: &mut Vec<Option<u64>>| {
        let value = std::mem::take(text).trim_end().to_string();
        if value.is_empty() {
            return;
        }
        if let Some(level) = heading {
            doc.outline.push((level, value.clone()));
            doc.blocks.push(Block::Heading { level, text: value });
        } else if code {
            doc.blocks.push(Block::Code(value));
        } else if item {
            let ordinal = lists.last().copied().flatten();
            if let Some(n) = ordinal {
                if let Some(slot) = lists.last_mut() {
                    *slot = Some(n + 1);
                }
            }
            doc.blocks.push(Block::Item {
                ordinal,
                depth: lists.len().saturating_sub(1),
                text: value,
            });
        } else if quote {
            doc.blocks.push(Block::Quote(value));
        } else {
            doc.blocks.push(Block::Paragraph(value));
        }
    };

    for event in Parser::new_ext(source, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some(level);
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(
                    &mut doc,
                    &mut text,
                    heading.take(),
                    quote,
                    code,
                    false,
                    &mut lists,
                );
            }
            Event::Start(Tag::Paragraph) => text.clear(),
            Event::End(TagEnd::Paragraph) => {
                if !item {
                    flush(&mut doc, &mut text, None, quote, code, false, &mut lists);
                }
            }
            Event::Start(Tag::BlockQuote(_)) => quote = true,
            Event::End(TagEnd::BlockQuote(_)) => quote = false,
            Event::Start(Tag::CodeBlock(_)) => {
                code = true;
                text.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                flush(&mut doc, &mut text, None, false, true, false, &mut lists);
                code = false;
            }
            Event::Start(Tag::List(start)) => lists.push(start),
            Event::End(TagEnd::List(_)) => {
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                item = true;
                text.clear();
            }
            Event::End(TagEnd::Item) => {
                flush(&mut doc, &mut text, None, quote, false, true, &mut lists);
                item = false;
            }
            Event::Text(value) | Event::Code(value) => text.push_str(&value),
            Event::SoftBreak | Event::HardBreak => text.push('\n'),
            Event::Rule => doc.blocks.push(Block::Rule),
            _ => {}
        }
    }
    doc
}
