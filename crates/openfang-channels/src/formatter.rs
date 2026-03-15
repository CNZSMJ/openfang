//! Channel-specific message formatting.
//!
//! Parses Markdown into a small document AST, then renders it into
//! channel-specific output formats.

use html_escape::{encode_double_quoted_attribute, encode_text};
use openfang_types::config::OutputFormat;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Format a message for a specific channel output format.
pub fn format_for_channel(text: &str, format: OutputFormat) -> String {
    match format {
        OutputFormat::Markdown => text.to_string(),
        OutputFormat::TelegramHtml => render_telegram_html(&parse_document(text)),
        OutputFormat::SlackMrkdwn => render_slack_mrkdwn(&parse_document(text)),
        OutputFormat::PlainText => render_plain_text(&parse_document(text)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Document {
    blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Block {
    Paragraph(Vec<InlineNode>),
    Heading {
        level: u8,
        content: Vec<InlineNode>,
    },
    BulletList(Vec<ListItem>),
    OrderedList {
        start: u64,
        items: Vec<ListItem>,
    },
    Quote(Vec<Block>),
    CodeFence {
        language: Option<String>,
        code: String,
    },
    Table {
        aligns: Vec<TableAlign>,
        headers: Vec<Vec<InlineNode>>,
        rows: Vec<Vec<Vec<InlineNode>>>,
    },
    ThematicBreak,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ListItem {
    blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InlineNode {
    Text(String),
    Bold(Vec<InlineNode>),
    Italic(Vec<InlineNode>),
    Code(String),
    Link {
        label: Vec<InlineNode>,
        url: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableAlign {
    Left,
    Center,
    Right,
    None,
}

fn parse_document(text: &str) -> Document {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);

    let events: Vec<Event<'_>> = Parser::new_ext(text, options).collect();
    let mut cursor = EventCursor::new(events);
    Document {
        blocks: cursor.parse_blocks(None),
    }
}

struct EventCursor<'a> {
    events: Vec<Event<'a>>,
    pos: usize,
}

impl<'a> EventCursor<'a> {
    fn new(events: Vec<Event<'a>>) -> Self {
        Self { events, pos: 0 }
    }

    fn parse_blocks(&mut self, until: Option<TagEnd>) -> Vec<Block> {
        let mut blocks = Vec::new();

        while let Some(event) = self.peek().cloned() {
            if matches_end(&event, until.as_ref()) {
                self.pos += 1;
                break;
            }

            match event {
                Event::Start(Tag::Paragraph) => {
                    self.pos += 1;
                    blocks.push(Block::Paragraph(self.parse_inlines(TagEnd::Paragraph)));
                }
                Event::Start(Tag::Heading { level, .. }) => {
                    self.pos += 1;
                    blocks.push(Block::Heading {
                        level: heading_level_to_u8(level),
                        content: self.parse_inlines(TagEnd::Heading(level)),
                    });
                }
                Event::Start(Tag::BlockQuote(kind)) => {
                    self.pos += 1;
                    blocks.push(Block::Quote(
                        self.parse_blocks(Some(TagEnd::BlockQuote(kind))),
                    ));
                }
                Event::Start(Tag::List(start)) => {
                    self.pos += 1;
                    let items = self.parse_list_items();
                    match start {
                        Some(start) => blocks.push(Block::OrderedList { start, items }),
                        None => blocks.push(Block::BulletList(items)),
                    }
                }
                Event::Start(Tag::CodeBlock(kind)) => {
                    self.pos += 1;
                    blocks.push(self.parse_code_block(kind));
                }
                Event::Start(Tag::Table(aligns)) => {
                    self.pos += 1;
                    blocks.push(self.parse_table(aligns));
                }
                Event::Rule => {
                    self.pos += 1;
                    blocks.push(Block::ThematicBreak);
                }
                Event::SoftBreak | Event::HardBreak => {
                    self.pos += 1;
                }
                _ if is_inline_event(&event) => {
                    let inlines = self.parse_inline_paragraph(until.as_ref());
                    if !inlines.is_empty() {
                        blocks.push(Block::Paragraph(inlines));
                    }
                }
                Event::End(_) => {
                    self.pos += 1;
                }
                _ => {
                    self.pos += 1;
                }
            }
        }

        blocks
    }

    fn parse_list_items(&mut self) -> Vec<ListItem> {
        let mut items = Vec::new();

        while let Some(event) = self.peek().cloned() {
            match event {
                Event::Start(Tag::Item) => {
                    self.pos += 1;
                    items.push(ListItem {
                        blocks: self.parse_blocks(Some(TagEnd::Item)),
                    });
                }
                Event::End(TagEnd::List(_)) => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    self.pos += 1;
                }
            }
        }

        items
    }

    fn parse_code_block(&mut self, kind: CodeBlockKind<'a>) -> Block {
        let language = match kind {
            CodeBlockKind::Indented => None,
            CodeBlockKind::Fenced(lang) => {
                let lang = lang.trim();
                if lang.is_empty() {
                    None
                } else {
                    Some(lang.to_string())
                }
            }
        };

        let mut code = String::new();
        while let Some(event) = self.next() {
            match event {
                Event::End(TagEnd::CodeBlock) => break,
                Event::Text(text) | Event::Code(text) | Event::Html(text) | Event::InlineHtml(text) => {
                    code.push_str(&text);
                }
                Event::SoftBreak | Event::HardBreak => code.push('\n'),
                _ => {}
            }
        }

        Block::CodeFence { language, code }
    }

    fn parse_table(&mut self, aligns: Vec<Alignment>) -> Block {
        let aligns = aligns.into_iter().map(TableAlign::from).collect();
        let mut headers = Vec::new();
        let mut rows = Vec::new();

        while let Some(event) = self.peek().cloned() {
            match event {
                Event::Start(Tag::TableHead) => {
                    self.pos += 1;
                    headers = self.parse_table_header();
                }
                Event::Start(Tag::TableRow) => {
                    self.pos += 1;
                    let row = self.parse_table_row();
                    rows.push(row);
                }
                Event::End(TagEnd::Table) => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    self.pos += 1;
                }
            }
        }

        Block::Table {
            aligns,
            headers,
            rows,
        }
    }

    fn parse_table_header(&mut self) -> Vec<Vec<InlineNode>> {
        let mut headers = Vec::new();
        while let Some(event) = self.peek().cloned() {
            if matches_end(&event, Some(&TagEnd::TableHead)) {
                self.pos += 1;
                break;
            }

            match event {
                Event::Start(Tag::TableCell) => {
                    self.pos += 1;
                    headers.push(self.parse_inlines(TagEnd::TableCell));
                }
                _ => {
                    self.pos += 1;
                }
            }
        }
        headers
    }

    fn parse_table_row(&mut self) -> Vec<Vec<InlineNode>> {
        let mut cells = Vec::new();

        while let Some(event) = self.peek().cloned() {
            match event {
                Event::Start(Tag::TableCell) => {
                    self.pos += 1;
                    cells.push(self.parse_inlines(TagEnd::TableCell));
                }
                Event::End(TagEnd::TableRow) => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    self.pos += 1;
                }
            }
        }

        cells
    }

    fn parse_inlines(&mut self, until: TagEnd) -> Vec<InlineNode> {
        let mut nodes = Vec::new();

        while let Some(event) = self.next() {
            match event {
                Event::End(end) if end == until => break,
                Event::Text(text) => push_text(&mut nodes, &text),
                Event::Code(code) => nodes.push(InlineNode::Code(code.into_string())),
                Event::SoftBreak | Event::HardBreak => push_text(&mut nodes, "\n"),
                Event::Start(Tag::Strong) => {
                    nodes.push(InlineNode::Bold(self.parse_inlines(TagEnd::Strong)));
                }
                Event::Start(Tag::Emphasis) => {
                    nodes.push(InlineNode::Italic(self.parse_inlines(TagEnd::Emphasis)));
                }
                Event::Start(Tag::Link { dest_url, .. }) => {
                    nodes.push(InlineNode::Link {
                        label: self.parse_inlines(TagEnd::Link),
                        url: dest_url.into_string(),
                    });
                }
                Event::Html(text) | Event::InlineHtml(text) => push_text(&mut nodes, &text),
                Event::FootnoteReference(text) => {
                    push_text(&mut nodes, &format!("[^{text}]"));
                }
                Event::TaskListMarker(checked) => {
                    push_text(&mut nodes, if checked { "[x] " } else { "[ ] " });
                }
                Event::Rule => push_text(&mut nodes, "---"),
                Event::Start(_) => {
                    // Ignore unsupported nested tags while preserving outer flow.
                }
                Event::End(_) => {}
                Event::InlineMath(text) | Event::DisplayMath(text) => push_text(&mut nodes, &text),
            }
        }

        nodes
    }

    fn parse_inline_paragraph(&mut self, parent_end: Option<&TagEnd>) -> Vec<InlineNode> {
        let mut nodes = Vec::new();

        while let Some(event) = self.peek().cloned() {
            if matches_end(&event, parent_end) || starts_block(&event) {
                break;
            }

            let Some(next_event) = self.next() else {
                break;
            };

            match next_event {
                Event::Text(text) => push_text(&mut nodes, &text),
                Event::Code(code) => nodes.push(InlineNode::Code(code.into_string())),
                Event::SoftBreak | Event::HardBreak => push_text(&mut nodes, "\n"),
                Event::Start(Tag::Strong) => {
                    nodes.push(InlineNode::Bold(self.parse_inlines(TagEnd::Strong)));
                }
                Event::Start(Tag::Emphasis) => {
                    nodes.push(InlineNode::Italic(self.parse_inlines(TagEnd::Emphasis)));
                }
                Event::Start(Tag::Link { dest_url, .. }) => {
                    nodes.push(InlineNode::Link {
                        label: self.parse_inlines(TagEnd::Link),
                        url: dest_url.into_string(),
                    });
                }
                Event::Html(text) | Event::InlineHtml(text) => push_text(&mut nodes, &text),
                Event::FootnoteReference(text) => push_text(&mut nodes, &format!("[^{text}]")),
                Event::TaskListMarker(checked) => {
                    push_text(&mut nodes, if checked { "[x] " } else { "[ ] " });
                }
                Event::Rule => break,
                Event::End(_) => break,
                Event::Start(_) => {}
                Event::InlineMath(text) | Event::DisplayMath(text) => push_text(&mut nodes, &text),
            }
        }

        nodes
    }

    fn peek(&self) -> Option<&Event<'a>> {
        self.events.get(self.pos)
    }

    fn next(&mut self) -> Option<Event<'a>> {
        let event = self.events.get(self.pos).cloned();
        self.pos += usize::from(event.is_some());
        event
    }
}

impl From<Alignment> for TableAlign {
    fn from(value: Alignment) -> Self {
        match value {
            Alignment::Left => TableAlign::Left,
            Alignment::Center => TableAlign::Center,
            Alignment::Right => TableAlign::Right,
            Alignment::None => TableAlign::None,
        }
    }
}

fn matches_end(event: &Event<'_>, expected: Option<&TagEnd>) -> bool {
    matches!((event, expected), (Event::End(actual), Some(expected)) if actual == expected)
}

fn starts_block(event: &Event<'_>) -> bool {
    matches!(
        event,
        Event::Start(
            Tag::Paragraph
                | Tag::Heading { .. }
                | Tag::BlockQuote(_)
                | Tag::CodeBlock(_)
                | Tag::List(_)
                | Tag::Item
                | Tag::Table(_)
                | Tag::TableHead
                | Tag::TableRow
                | Tag::TableCell
        ) | Event::Rule
    )
}

fn is_inline_event(event: &Event<'_>) -> bool {
    matches!(
        event,
        Event::Text(_)
            | Event::Code(_)
            | Event::Html(_)
            | Event::InlineHtml(_)
            | Event::SoftBreak
            | Event::HardBreak
            | Event::FootnoteReference(_)
            | Event::TaskListMarker(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::Start(Tag::Strong | Tag::Emphasis | Tag::Link { .. })
    )
}

fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn render_telegram_html(document: &Document) -> String {
    render_blocks_telegram(&document.blocks, 0)
}

fn render_blocks_telegram(blocks: &[Block], indent: usize) -> String {
    let mut parts = Vec::new();

    for block in blocks {
        let rendered = match block {
            Block::Paragraph(content) => format!("{}{}", " ".repeat(indent), render_inlines_telegram(content)),
            Block::Heading { level: _, content } => {
                format!("{}<b>{}</b>", " ".repeat(indent), render_inlines_telegram(content))
            }
            Block::BulletList(items) => render_list_telegram(items, indent, None),
            Block::OrderedList { start, items } => render_list_telegram(items, indent, Some(*start)),
            Block::Quote(children) => {
                let inner = render_blocks_telegram(children, 0);
                format!("{}<blockquote>{}</blockquote>", " ".repeat(indent), inner)
            }
            Block::CodeFence { code, .. } => format!(
                "{}<pre><code>{}</code></pre>",
                " ".repeat(indent),
                encode_text(code)
            ),
            Block::Table { aligns, headers, rows } => format!(
                "{}<pre><code>{}</code></pre>",
                " ".repeat(indent),
                encode_text(&render_ascii_table(aligns, headers, rows))
            ),
            Block::ThematicBreak => format!("{}────────", " ".repeat(indent)),
        };
        parts.push(rendered);
    }

    parts.join("\n\n")
}

fn render_list_telegram(items: &[ListItem], indent: usize, ordered_start: Option<u64>) -> String {
    let mut lines = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let prefix = match ordered_start {
            Some(start) => format!("{}{}. ", " ".repeat(indent), start + index as u64),
            None => format!("{}• ", " ".repeat(indent)),
        };
        lines.push(render_list_item_telegram(item, &prefix, indent + 2));
    }
    lines.join("\n")
}

fn render_list_item_telegram(item: &ListItem, prefix: &str, nested_indent: usize) -> String {
    render_list_item_blocks_telegram(&item.blocks, prefix, nested_indent)
}

fn render_inlines_telegram(nodes: &[InlineNode]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            InlineNode::Text(text) => out.push_str(&encode_text(text)),
            InlineNode::Bold(children) => {
                out.push_str("<b>");
                out.push_str(&render_inlines_telegram(children));
                out.push_str("</b>");
            }
            InlineNode::Italic(children) => {
                out.push_str("<i>");
                out.push_str(&render_inlines_telegram(children));
                out.push_str("</i>");
            }
            InlineNode::Code(code) => {
                out.push_str("<code>");
                out.push_str(&encode_text(code));
                out.push_str("</code>");
            }
            InlineNode::Link { label, url } => {
                out.push_str("<a href=\"");
                out.push_str(&encode_double_quoted_attribute(url));
                out.push_str("\">");
                out.push_str(&render_inlines_telegram(label));
                out.push_str("</a>");
            }
        }
    }
    out
}

fn render_slack_mrkdwn(document: &Document) -> String {
    render_blocks_slack(&document.blocks, 0)
}

fn render_blocks_slack(blocks: &[Block], indent: usize) -> String {
    let mut parts = Vec::new();

    for block in blocks {
        let rendered = match block {
            Block::Paragraph(content) => format!("{}{}", " ".repeat(indent), render_inlines_slack(content)),
            Block::Heading { level: _, content } => {
                format!("{}*{}*", " ".repeat(indent), render_inlines_slack(content))
            }
            Block::BulletList(items) => render_list_slack(items, indent, None),
            Block::OrderedList { start, items } => render_list_slack(items, indent, Some(*start)),
            Block::Quote(children) => prefix_lines(&render_blocks_slack(children, 0), &format!("{}> ", " ".repeat(indent))),
            Block::CodeFence { language, code } => {
                let mut out = String::new();
                out.push_str("```");
                if let Some(language) = language {
                    out.push_str(language);
                }
                out.push('\n');
                out.push_str(code);
                if !code.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("```");
                out
            }
            Block::Table { aligns, headers, rows } => {
                let ascii = render_ascii_table(aligns, headers, rows);
                format!("```\n{}\n```", ascii)
            }
            Block::ThematicBreak => "----------".to_string(),
        };
        parts.push(rendered);
    }

    parts.join("\n\n")
}

fn render_list_slack(items: &[ListItem], indent: usize, ordered_start: Option<u64>) -> String {
    let mut lines = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let prefix = match ordered_start {
            Some(start) => format!("{}{}. ", " ".repeat(indent), start + index as u64),
            None => format!("{}- ", " ".repeat(indent)),
        };
        lines.push(render_list_item_slack(item, &prefix, indent + 2));
    }
    lines.join("\n")
}

fn render_list_item_slack(item: &ListItem, prefix: &str, nested_indent: usize) -> String {
    render_list_item_blocks_slack(&item.blocks, prefix, nested_indent)
}

fn render_inlines_slack(nodes: &[InlineNode]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            InlineNode::Text(text) => out.push_str(text),
            InlineNode::Bold(children) => {
                out.push('*');
                out.push_str(&render_inlines_slack(children));
                out.push('*');
            }
            InlineNode::Italic(children) => {
                out.push('_');
                out.push_str(&render_inlines_slack(children));
                out.push('_');
            }
            InlineNode::Code(code) => {
                out.push('`');
                out.push_str(code);
                out.push('`');
            }
            InlineNode::Link { label, url } => {
                out.push('<');
                out.push_str(url);
                out.push('|');
                out.push_str(&render_plain_inlines(label));
                out.push('>');
            }
        }
    }
    out
}

fn render_plain_text(document: &Document) -> String {
    render_blocks_plain(&document.blocks, 0)
}

fn render_blocks_plain(blocks: &[Block], indent: usize) -> String {
    let mut parts = Vec::new();

    for block in blocks {
        let rendered = match block {
            Block::Paragraph(content) => format!("{}{}", " ".repeat(indent), render_plain_inlines(content)),
            Block::Heading { level, content } => {
                let text = render_plain_inlines(content);
                let underline = match level {
                    1 => "=".repeat(text.chars().count()),
                    2 => "-".repeat(text.chars().count()),
                    _ => String::new(),
                };
                if underline.is_empty() {
                    format!("{}{}", " ".repeat(indent), text)
                } else {
                    format!("{}{}\n{}{}", " ".repeat(indent), text, " ".repeat(indent), underline)
                }
            }
            Block::BulletList(items) => render_list_plain(items, indent, None),
            Block::OrderedList { start, items } => render_list_plain(items, indent, Some(*start)),
            Block::Quote(children) => prefix_lines(&render_blocks_plain(children, 0), &format!("{}> ", " ".repeat(indent))),
            Block::CodeFence { language, code } => {
                let lang = language.as_deref().unwrap_or("");
                format!("{}```{}\n{}\n{}```", " ".repeat(indent), lang, code, " ".repeat(indent))
            }
            Block::Table { aligns, headers, rows } => render_ascii_table(aligns, headers, rows),
            Block::ThematicBreak => "----------".to_string(),
        };
        parts.push(rendered);
    }

    parts.join("\n\n")
}

fn render_list_plain(items: &[ListItem], indent: usize, ordered_start: Option<u64>) -> String {
    let mut lines = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let prefix = match ordered_start {
            Some(start) => format!("{}{}. ", " ".repeat(indent), start + index as u64),
            None => format!("{}- ", " ".repeat(indent)),
        };
        lines.push(render_list_item_blocks_plain(&item.blocks, &prefix, indent + 2));
    }
    lines.join("\n")
}

fn render_list_item_blocks_telegram(blocks: &[Block], prefix: &str, nested_indent: usize) -> String {
    render_list_item_blocks_generic(
        blocks,
        prefix,
        nested_indent,
        render_block_telegram_single,
    )
}

fn render_list_item_blocks_slack(blocks: &[Block], prefix: &str, nested_indent: usize) -> String {
    render_list_item_blocks_generic(
        blocks,
        prefix,
        nested_indent,
        render_block_slack_single,
    )
}

fn render_list_item_blocks_plain(blocks: &[Block], prefix: &str, nested_indent: usize) -> String {
    render_list_item_blocks_generic(
        blocks,
        prefix,
        nested_indent,
        render_block_plain_single,
    )
}

fn render_list_item_blocks_generic<F>(
    blocks: &[Block],
    prefix: &str,
    nested_indent: usize,
    mut render_block: F,
) -> String
where
    F: FnMut(&Block, usize) -> String,
{
    let Some((first, rest)) = blocks.split_first() else {
        return prefix.trim_end().to_string();
    };

    let first_rendered = render_block(first, 0);
    let mut parts = vec![format!("{prefix}{first_rendered}")];
    parts.extend(rest.iter().map(|block| render_block(block, nested_indent)));
    parts.join("\n\n")
}

fn render_block_telegram_single(block: &Block, indent: usize) -> String {
    match block {
        Block::Paragraph(content) => format!("{}{}", " ".repeat(indent), render_inlines_telegram(content)),
        Block::Heading { level: _, content } => {
            format!("{}<b>{}</b>", " ".repeat(indent), render_inlines_telegram(content))
        }
        Block::BulletList(items) => render_list_telegram(items, indent, None),
        Block::OrderedList { start, items } => render_list_telegram(items, indent, Some(*start)),
        Block::Quote(children) => {
            let inner = render_blocks_telegram(children, 0);
            format!("{}<blockquote>{}</blockquote>", " ".repeat(indent), inner)
        }
        Block::CodeFence { code, .. } => format!(
            "{}<pre><code>{}</code></pre>",
            " ".repeat(indent),
            encode_text(code)
        ),
        Block::Table { aligns, headers, rows } => format!(
            "{}<pre><code>{}</code></pre>",
            " ".repeat(indent),
            encode_text(&render_ascii_table(aligns, headers, rows))
        ),
        Block::ThematicBreak => format!("{}────────", " ".repeat(indent)),
    }
}

fn render_block_slack_single(block: &Block, indent: usize) -> String {
    match block {
        Block::Paragraph(content) => format!("{}{}", " ".repeat(indent), render_inlines_slack(content)),
        Block::Heading { level: _, content } => {
            format!("{}*{}*", " ".repeat(indent), render_inlines_slack(content))
        }
        Block::BulletList(items) => render_list_slack(items, indent, None),
        Block::OrderedList { start, items } => render_list_slack(items, indent, Some(*start)),
        Block::Quote(children) => prefix_lines(&render_blocks_slack(children, 0), &format!("{}> ", " ".repeat(indent))),
        Block::CodeFence { language, code } => {
            let mut out = String::new();
            out.push_str("```");
            if let Some(language) = language {
                out.push_str(language);
            }
            out.push('\n');
            out.push_str(code);
            if !code.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("```");
            out
        }
        Block::Table { aligns, headers, rows } => {
            let ascii = render_ascii_table(aligns, headers, rows);
            format!("```\n{}\n```", ascii)
        }
        Block::ThematicBreak => "----------".to_string(),
    }
}

fn render_block_plain_single(block: &Block, indent: usize) -> String {
    match block {
        Block::Paragraph(content) => format!("{}{}", " ".repeat(indent), render_plain_inlines(content)),
        Block::Heading { level, content } => {
            let text = render_plain_inlines(content);
            let underline = match level {
                1 => "=".repeat(text.chars().count()),
                2 => "-".repeat(text.chars().count()),
                _ => String::new(),
            };
            if underline.is_empty() {
                format!("{}{}", " ".repeat(indent), text)
            } else {
                format!("{}{}\n{}{}", " ".repeat(indent), text, " ".repeat(indent), underline)
            }
        }
        Block::BulletList(items) => render_list_plain(items, indent, None),
        Block::OrderedList { start, items } => render_list_plain(items, indent, Some(*start)),
        Block::Quote(children) => prefix_lines(&render_blocks_plain(children, 0), &format!("{}> ", " ".repeat(indent))),
        Block::CodeFence { language, code } => {
            let lang = language.as_deref().unwrap_or("");
            format!("{}```{}\n{}\n{}```", " ".repeat(indent), lang, code, " ".repeat(indent))
        }
        Block::Table { aligns, headers, rows } => render_ascii_table(aligns, headers, rows),
        Block::ThematicBreak => "----------".to_string(),
    }
}

fn render_plain_inlines(nodes: &[InlineNode]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            InlineNode::Text(text) => out.push_str(text),
            InlineNode::Bold(children) | InlineNode::Italic(children) => {
                out.push_str(&render_plain_inlines(children));
            }
            InlineNode::Code(code) => out.push_str(code),
            InlineNode::Link { label, url } => {
                out.push_str(&render_plain_inlines(label));
                out.push_str(" (");
                out.push_str(url);
                out.push(')');
            }
        }
    }
    out
}

fn render_ascii_table(
    aligns: &[TableAlign],
    headers: &[Vec<InlineNode>],
    rows: &[Vec<Vec<InlineNode>>],
) -> String {
    let col_count = headers
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0))
        .max(1);

    let mut data = Vec::new();
    if !headers.is_empty() {
        data.push(headers.iter().map(|cell| render_plain_inlines(cell)).collect::<Vec<_>>());
    }
    for row in rows {
        data.push(row.iter().map(|cell| render_plain_inlines(cell)).collect::<Vec<_>>());
    }

    let mut widths = vec![0usize; col_count];
    for row in &data {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }

    let mut lines = Vec::new();
    if !headers.is_empty() {
        lines.push(render_ascii_table_row(
            &headers
                .iter()
                .map(|cell| render_plain_inlines(cell))
                .collect::<Vec<_>>(),
            &widths,
            aligns,
        ));
        lines.push(render_ascii_table_separator(&widths));
    }

    for row in rows {
        lines.push(render_ascii_table_row(
            &row.iter().map(|cell| render_plain_inlines(cell)).collect::<Vec<_>>(),
            &widths,
            aligns,
        ));
    }

    lines.join("\n")
}

fn render_ascii_table_row(row: &[String], widths: &[usize], aligns: &[TableAlign]) -> String {
    let mut cells = Vec::new();

    for (index, width) in widths.iter().enumerate() {
        let text = row.get(index).map(String::as_str).unwrap_or("");
        let align = aligns.get(index).copied().unwrap_or(TableAlign::None);
        cells.push(pad_cell(text, *width, align));
    }

    format!("| {} |", cells.join(" | "))
}

fn render_ascii_table_separator(widths: &[usize]) -> String {
    let cells = widths
        .iter()
        .map(|width| "-".repeat((*width).max(3)))
        .collect::<Vec<_>>();
    format!("| {} |", cells.join(" | "))
}

fn pad_cell(text: &str, width: usize, align: TableAlign) -> String {
    let len = text.chars().count();
    if len >= width {
        return text.to_string();
    }

    let pad = width - len;
    match align {
        TableAlign::Right => format!("{}{}", " ".repeat(pad), text),
        TableAlign::Center => {
            let left = pad / 2;
            let right = pad - left;
            format!("{}{}{}", " ".repeat(left), text, " ".repeat(right))
        }
        TableAlign::Left | TableAlign::None => format!("{}{}", text, " ".repeat(pad)),
    }
}

fn prefix_lines(text: &str, prefix: &str) -> String {
    text.lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn push_text(nodes: &mut Vec<InlineNode>, text: &str) {
    if let Some(InlineNode::Text(existing)) = nodes.last_mut() {
        existing.push_str(text);
    } else {
        nodes.push(InlineNode::Text(text.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_markdown_passthrough() {
        let text = "**bold** and *italic*";
        assert_eq!(format_for_channel(text, OutputFormat::Markdown), text);
    }

    #[test]
    fn test_telegram_html_bold() {
        let result = format_for_channel("Hello **world**!", OutputFormat::TelegramHtml);
        assert_eq!(result, "Hello <b>world</b>!");
    }

    #[test]
    fn test_telegram_html_italic() {
        let result = format_for_channel("Hello *world*!", OutputFormat::TelegramHtml);
        assert_eq!(result, "Hello <i>world</i>!");
    }

    #[test]
    fn test_telegram_html_preserves_bullet_lists() {
        let input = "### 1. 垃圾清理\n*   **清理对象：** `logs/`\n*   **动作：** 直接删！";
        let result = format_for_channel(input, OutputFormat::TelegramHtml);
        assert_eq!(
            result,
            "<b>1. 垃圾清理</b>\n\n• <b>清理对象：</b> <code>logs/</code>\n• <b>动作：</b> 直接删！"
        );
    }

    #[test]
    fn test_telegram_html_leaves_unbalanced_asterisks_literal() {
        let result =
            format_for_channel("* 列表项\n还有一个孤立的 * 星号", OutputFormat::TelegramHtml);
        assert_eq!(result, "• 列表项\n还有一个孤立的 * 星号");
    }

    #[test]
    fn test_telegram_html_code() {
        let result = format_for_channel("Use `println!`", OutputFormat::TelegramHtml);
        assert_eq!(result, "Use <code>println!</code>");
    }

    #[test]
    fn test_telegram_html_link() {
        let result =
            format_for_channel("[click here](https://example.com)", OutputFormat::TelegramHtml);
        assert_eq!(result, "<a href=\"https://example.com\">click here</a>");
    }

    #[test]
    fn test_telegram_html_escapes_text_and_url() {
        let result = format_for_channel(
            "[<click>](https://example.com/?a=\"b\") & <tag>",
            OutputFormat::TelegramHtml,
        );
        assert_eq!(
            result,
            "<a href=\"https://example.com/?a=&quot;b&quot;\">&lt;click&gt;</a> &amp; &lt;tag&gt;"
        );
    }

    #[test]
    fn test_telegram_html_nested_inline_nodes() {
        let result = format_for_channel("**bold and *italic***", OutputFormat::TelegramHtml);
        assert_eq!(result, "<b>bold and <i>italic</i></b>");
    }

    #[test]
    fn test_telegram_html_code_fence() {
        let result = format_for_channel("```rust\nfn main() {}\n```", OutputFormat::TelegramHtml);
        assert_eq!(result, "<pre><code>fn main() {}\n</code></pre>");
    }

    #[test]
    fn test_telegram_html_table_renders_as_preformatted_text() {
        let result = format_for_channel(
            "| Name | Cost |\n| ---- | ----: |\n| A | 12 |\n| B | 234 |",
            OutputFormat::TelegramHtml,
        );
        assert_eq!(
            result,
            "<pre><code>| Name | Cost |\n| ---- | ---- |\n| A    |   12 |\n| B    |  234 |</code></pre>"
        );
    }

    #[test]
    fn test_slack_mrkdwn_bold() {
        let result = format_for_channel("Hello **world**!", OutputFormat::SlackMrkdwn);
        assert_eq!(result, "Hello *world*!");
    }

    #[test]
    fn test_slack_mrkdwn_italic() {
        let result = format_for_channel("Hello *world*!", OutputFormat::SlackMrkdwn);
        assert_eq!(result, "Hello _world_!");
    }

    #[test]
    fn test_slack_mrkdwn_link() {
        let result = format_for_channel("[click](https://example.com)", OutputFormat::SlackMrkdwn);
        assert_eq!(result, "<https://example.com|click>");
    }

    #[test]
    fn test_slack_code_fence_and_table() {
        let result = format_for_channel(
            "```bash\necho hi\n```\n\n| A | B |\n| - | - |\n| 1 | 2 |",
            OutputFormat::SlackMrkdwn,
        );
        assert_eq!(result, "```bash\necho hi\n```\n\n```\n| A | B |\n| --- | --- |\n| 1 | 2 |\n```");
    }

    #[test]
    fn test_plain_text_strips_formatting() {
        let result = format_for_channel("**bold** and `code` and *italic*", OutputFormat::PlainText);
        assert_eq!(result, "bold and code and italic");
    }

    #[test]
    fn test_plain_text_converts_links() {
        let result = format_for_channel("[click](https://example.com)", OutputFormat::PlainText);
        assert_eq!(result, "click (https://example.com)");
    }

    #[test]
    fn test_plain_text_preserves_unmatched_markers() {
        let result = format_for_channel("`oops and **still literal", OutputFormat::PlainText);
        assert_eq!(result, "`oops and **still literal");
    }
}
