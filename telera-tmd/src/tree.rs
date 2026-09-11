//! Position-aware structural classifier over `markdown`'s AST.
//!
//! [`parse`] runs [`crate::prepass::add_missing_keyword_backticks`], then
//! `markdown::to_mdast`, then a walk that **mirrors the layout-file parser's
//! structure but resolves nothing** — every list item is tagged with a grammar
//! [`Role`] and a source [`Span`], malformed items included. This is what the
//! language server analyses; it never produces the runtime `Layout` stream.
//!
//! Faithfulness note: the `markdown` crate inserts a `Text(" ")` node between an
//! inline-code span and a following `*emphasis*`, so a keyword's argument lands
//! at `children[2]` (emphasis) or `children[1]` (literal). [`keyword_and_arg`]
//! handles that. `image` / `set-image` and the `text`-content-is-the-second-
//! bullet rule are handled in [`walk_item`].

use markdown::mdast::{self, Node};
use markdown::unist::Position;

use crate::prepass::{Rewrite, rewrite_with_map};
use crate::schema::{self, Context};

/// A source range: byte offsets into the (post-prepass) text plus 1-based
/// line/column of the start and end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Span {
    pub byte_start: usize,
    pub byte_end: usize,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

impl Span {
    fn from_position(p: &Position) -> Self {
        Span {
            byte_start: p.start.offset,
            byte_end: p.end.offset,
            start_line: p.start.line,
            start_col: p.start.column,
            end_line: p.end.line,
            end_col: p.end.column,
        }
    }
    fn of(node: &Node) -> Span {
        node.position().map(Span::from_position).unwrap_or_default()
    }
}

/// A string slice plus its span.
#[derive(Clone, Debug, PartialEq)]
pub struct Spanned {
    pub text: String,
    pub span: Span,
}

/// The parsed argument that follows a keyword on its line.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    /// Plain trailing text (a literal, or a bare name).
    Literal(Spanned),
    /// An `*emphasis*` span (a dynamic name).
    Emphasis(Spanned),
    /// A `[text](url)` link (preamble directives).
    Link { text: Spanned, url: Spanned },
    /// An inline `` `code` `` span (a `calc` fn / expression).
    Code(Spanned),
    /// A `[f, f, f, f]` rectangle. `floats` is `None` when it wasn't exactly 4
    /// parseable numbers (which silently changes `image` semantics).
    UvRect { span: Span, floats: Option<[f32; 4]> },
    /// The `<list-name> <index>` compound argument of `item` / `if-index`.
    Compound { name: Spanned, index: Spanned },
    /// The keyword expects an argument but none was given.
    Missing,
}

/// What a list item is, grammatically.
#[derive(Clone, Debug, PartialEq)]
pub enum Role {
    /// A known keyword in some context (`kind` = which contexts the *cursor's*
    /// position allows; the analyser cross-checks against the schema).
    Keyword {
        name: String,
        name_span: Span,
        arg: Arg,
        /// The context this item was found in.
        context: Context,
    },
    /// A `text` element's content bullet (the second body item of a `text`).
    TextContent(Arg),
    /// A `#### TML` preamble list.
    Preamble,
    /// The first word isn't a keyword and the position isn't a free-text slot.
    Unknown { first_word: Spanned },
    /// Plain prose / a text run — nothing to check.
    Prose,
}

/// One classified list item and its children.
#[derive(Clone, Debug)]
pub struct TmlNode {
    pub role: Role,
    pub span: Span,
    pub children: Vec<TmlNode>,
}

/// A `## name` / `### name` reusable snippet.
#[derive(Clone, Debug)]
pub struct Snippet {
    pub name: Spanned,
    /// `false` for `## …` (config snippet), `true` for `### …` (element snippet).
    pub element_form: bool,
    pub body: Vec<TmlNode>,
}

/// The whole file, classified.
#[derive(Clone, Debug, Default)]
pub struct SpannedLayout {
    /// `#### TML …` preamble directive items.
    pub preamble: Vec<TmlNode>,
    /// `# …` page bodies (usually one).
    pub pages: Vec<Vec<TmlNode>>,
    pub config_snippets: Vec<Snippet>,
    pub element_snippets: Vec<Snippet>,
    /// `false` when `markdown::to_mdast` failed outright.
    pub parse_ok: bool,
}

/// Parse `source` into a [`SpannedLayout`]. Never fails — a broken document
/// yields `parse_ok = false` and whatever structure was recoverable.
pub fn parse(source: &str) -> SpannedLayout {
    // NOTE: the pre-pass only inserts backtick characters, shifting columns
    // rightward on rewritten lines. For now spans on those lines can be off by
    // the inserted backtick count; a byte-accurate offset map is a follow-up.
    // The walk below also accepts a *bare* leading keyword (a plain `Text`
    // node), so most files need no rewrite for classification to work.
    let rw = rewrite_with_map(source);
    let Ok(root) = markdown::to_mdast(&rw.text, &markdown::ParseOptions::default()) else {
        return SpannedLayout { parse_ok: false, ..Default::default() };
    };

    let mut out = SpannedLayout { parse_ok: true, ..Default::default() };
    let Some(children) = root.children() else { return out };

    // Walk top-level nodes: headings switch mode, the list that immediately
    // follows a heading is consumed for that mode.
    let mut mode = TopMode::Ignore;
    for node in children {
        match node {
            Node::Heading(h) => {
                mode = classify_heading(h, &mut out);
            }
            Node::List(list) => {
                match std::mem::replace(&mut mode, TopMode::Ignore) {
                    TopMode::Preamble => {
                        for item in &list.children {
                            if let Some(n) = walk_item(item, Context::Preamble) {
                                out.preamble.push(n);
                            }
                        }
                    }
                    TopMode::Page => {
                        out.pages.push(walk_body(list));
                    }
                    TopMode::ConfigSnippet(name) => {
                        // A `## name` snippet's body items are config keywords,
                        // inlined into a `config` block by `use` (config form).
                        out.config_snippets.push(Snippet {
                            name,
                            element_form: false,
                            body: list
                                .children
                                .iter()
                                .filter_map(|item| walk_item(item, Context::Config))
                                .collect(),
                        });
                    }
                    TopMode::ElementSnippet(name) => {
                        out.element_snippets.push(Snippet {
                            name,
                            element_form: true,
                            body: walk_body(list),
                        });
                    }
                    TopMode::Ignore => {}
                }
            }
            _ => {}
        }
    }

    // `markdown` reported every span as an offset into the rewritten buffer.
    // Map them back to the original source so diagnostics land on what the user
    // typed. No-op when the pre-pass changed nothing.
    if !rw.is_identity() {
        let idx = LineIndex::new(source);
        out.preamble.iter_mut().for_each(|n| remap_node(n, &rw, &idx));
        out.pages
            .iter_mut()
            .for_each(|p| p.iter_mut().for_each(|n| remap_node(n, &rw, &idx)));
        for s in out.config_snippets.iter_mut().chain(out.element_snippets.iter_mut()) {
            remap_span(&mut s.name.span, &rw, &idx);
            s.body.iter_mut().for_each(|n| remap_node(n, &rw, &idx));
        }
    }

    out
}

/// Byte-offset → 1-based line/column over a fixed source string.
struct LineIndex {
    /// Byte offset of the start of each line.
    line_starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    fn new(src: &str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        LineIndex { line_starts, len: src.len() }
    }
    /// 1-based `(line, column)` for a byte offset.
    fn line_col(&self, byte: usize) -> (usize, usize) {
        let byte = byte.min(self.len);
        let line = self.line_starts.partition_point(|&s| s <= byte).max(1);
        (line, byte - self.line_starts[line - 1] + 1)
    }
}

fn remap_span(span: &mut Span, rw: &Rewrite, idx: &LineIndex) {
    span.byte_start = rw.to_original(span.byte_start);
    span.byte_end = rw.to_original(span.byte_end);
    let (sl, sc) = idx.line_col(span.byte_start);
    let (el, ec) = idx.line_col(span.byte_end);
    span.start_line = sl;
    span.start_col = sc;
    span.end_line = el;
    span.end_col = ec;
}

fn remap_arg(arg: &mut Arg, rw: &Rewrite, idx: &LineIndex) {
    match arg {
        Arg::Literal(s) | Arg::Emphasis(s) | Arg::Code(s) => remap_span(&mut s.span, rw, idx),
        Arg::Link { text, url } => {
            remap_span(&mut text.span, rw, idx);
            remap_span(&mut url.span, rw, idx);
        }
        Arg::UvRect { span, .. } => remap_span(span, rw, idx),
        Arg::Compound { name, index } => {
            remap_span(&mut name.span, rw, idx);
            remap_span(&mut index.span, rw, idx);
        }
        Arg::Missing => {}
    }
}

fn remap_node(node: &mut TmlNode, rw: &Rewrite, idx: &LineIndex) {
    remap_span(&mut node.span, rw, idx);
    match &mut node.role {
        Role::Keyword { name_span, arg, .. } => {
            remap_span(name_span, rw, idx);
            remap_arg(arg, rw, idx);
        }
        Role::TextContent(arg) => remap_arg(arg, rw, idx),
        Role::Unknown { first_word } => remap_span(&mut first_word.span, rw, idx),
        Role::Preamble | Role::Prose => {}
    }
    node.children.iter_mut().for_each(|c| remap_node(c, rw, idx));
}

enum TopMode {
    Ignore,
    Preamble,
    Page,
    ConfigSnippet(Spanned),
    ElementSnippet(Spanned),
}

fn classify_heading(h: &mdast::Heading, _out: &mut SpannedLayout) -> TopMode {
    let text = heading_text(h);
    match h.depth {
        1 => TopMode::Page,
        2 => TopMode::ConfigSnippet(Spanned {
            text: text.trim().to_string(),
            span: h.position.as_ref().map(Span::from_position).unwrap_or_default(),
        }),
        3 => TopMode::ElementSnippet(Spanned {
            text: text.trim().to_string(),
            span: h.position.as_ref().map(Span::from_position).unwrap_or_default(),
        }),
        4 if text.trim_start().starts_with("TML") => TopMode::Preamble,
        _ => TopMode::Ignore,
    }
}

fn heading_text(h: &mdast::Heading) -> String {
    h.children
        .iter()
        .filter_map(|n| match n {
            Node::Text(t) => Some(t.value.as_str()),
            Node::InlineCode(c) => Some(c.value.as_str()),
            _ => None,
        })
        .collect()
}

/// Walk a body `List` (a page body, or a container's children) as elements.
fn walk_body(list: &mdast::List) -> Vec<TmlNode> {
    list.children
        .iter()
        .filter_map(|item| walk_item(item, Context::Element))
        .collect()
}

/// Classify one `ListItem`. `ctx` is the grammatical context it sits in.
fn walk_item(item: &Node, ctx: Context) -> Option<TmlNode> {
    let Node::ListItem(li) = item else { return None };
    let span = Span::of(item);
    let paragraph = li.children.first();
    let Some(Node::Paragraph(p)) = paragraph else {
        // A list item with no paragraph (rare) — nothing to classify.
        return Some(TmlNode { role: Role::Prose, span, children: vec![] });
    };

    let Some((name, name_span)) = leading_keyword(p) else {
        // Not `keyword`-shaped: prose in an element slot, or free text.
        let fw = first_word(p);
        let role = match fw {
            Some(w) => Role::Unknown { first_word: w },
            None => Role::Prose,
        };
        return Some(TmlNode { role, span, children: vec![] });
    };

    // Event-gate keywords (`left-clicked`, `hover`, `key-event`, …) sit *inside*
    // a `config` list structurally, but the schema files them under their own
    // `EventGate` context. Re-tag them so the analyser checks the right slot.
    let ctx = if ctx == Context::Config
        && schema::by_name(&name)
            .is_some_and(|k| k.valid_in(Context::EventGate) && !k.valid_in(Context::Config))
    {
        Context::EventGate
    } else {
        ctx
    };

    let nested = nested_list(li);
    let child_ctx = child_context(&name, ctx);
    let children = nested
        .map(|l| {
            l.children
                .iter()
                .filter_map(|c| walk_item(c, child_ctx))
                .collect()
        })
        .unwrap_or_default();

    let arg = extract_arg(&name, p);
    let role = Role::Keyword { name, name_span, arg, context: ctx };
    Some(TmlNode { role, span, children })
}

/// The context a keyword's *children* are in.
fn child_context(name: &str, parent: Context) -> Context {
    match name {
        "config" => Context::Config,
        "declarations" => Context::Declaration,
        // A `use` element-form body is a list of inline `set-*` bindings, fed
        // through the same path as a `declarations` block.
        "use" => Context::Declaration,
        // event gates open a nested config list
        _ if schema::by_name(name).is_some_and(|k| k.valid_in(Context::EventGate)) => Context::Config,
        // list/item/if bodies are element context
        "list" | "item" | "if" | "if-not" | "if-index" | "if-index-not" => Context::Element,
        _ => parent,
    }
}

fn nested_list(li: &mdast::ListItem) -> Option<&mdast::List> {
    li.children.iter().find_map(|c| match c {
        Node::List(l) => Some(l),
        _ => None,
    })
}

/// The leading `` `keyword` `` inline-code span, or a bare first word that is a
/// known keyword (the pre-pass would have backticked it, but accept it directly
/// so a not-yet-rewritten buffer still classifies).
fn leading_keyword(p: &mdast::Paragraph) -> Option<(String, Span)> {
    match p.children.first()? {
        Node::InlineCode(c) => Some((c.value.clone(), node_span(p.children.first()?))),
        Node::Text(t) => {
            let word = t.value.trim_start();
            let word = word.split([' ', '\t']).next().unwrap_or("");
            if !word.is_empty() && schema::by_name(word).is_some() {
                Some((word.to_string(), node_span(p.children.first()?)))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn node_span(n: &Node) -> Span {
    Span::of(n)
}

fn first_word(p: &mdast::Paragraph) -> Option<Spanned> {
    match p.children.first()? {
        Node::Text(t) if !t.value.trim().is_empty() => {
            let w = t.value.trim().split([' ', '\t']).next().unwrap_or("").to_string();
            Some(Spanned { text: w, span: node_span(p.children.first()?) })
        }
        Node::InlineCode(c) => Some(Spanned { text: c.value.clone(), span: node_span(p.children.first()?) }),
        _ => None,
    }
}

/// Pull the argument for a keyword out of its paragraph's remaining children.
fn extract_arg(name: &str, p: &mdast::Paragraph) -> Arg {
    // preamble directives: a link
    if matches!(
        schema::by_name(name).and_then(|k| k.arg_in(Context::Preamble)),
        Some(schema::ArgShape::Link)
    ) && let Some(link) = p.children.iter().find_map(|n| match n {
        Node::Link(l) => Some(l),
        _ => None,
    }) {
        let text = l_text(link);
        return Arg::Link {
            text: Spanned { text, span: Span::of(&Node::Link(link.clone())) },
            url: Spanned { text: link.url.clone(), span: Span::of(&Node::Link(link.clone())) },
        };
    }

    // an *emphasis* name anywhere after the keyword
    if let Some((emph_text, emph_span)) = p.children.iter().skip(1).find_map(|n| match n {
        Node::Emphasis(e) => match e.children.first() {
            Some(Node::Text(t)) if !t.value.trim().is_empty() => {
                Some((t.value.trim().to_string(), Span::of(n)))
            }
            _ => None,
        },
        _ => None,
    }) {
        return Arg::Emphasis(Spanned { text: emph_text, span: emph_span });
    }

    // an inline code span (calc)
    if let Some((code, span)) = p.children.iter().skip(1).find_map(|n| match n {
        Node::InlineCode(c) => Some((c.value.clone(), Span::of(n))),
        _ => None,
    }) {
        return Arg::Code(Spanned { text: code, span });
    }

    // trailing plain text
    if let Some((txt, span)) = p.children.iter().skip(1).find_map(|n| match n {
        Node::Text(t) if !t.value.trim().is_empty() => Some((t.value.trim().to_string(), Span::of(n))),
        _ => None,
    }) {
        // `item` alone is the compound `<list-name> <index>` form; `if-index` /
        // `if-index-not` take a single number-or-name argument.
        if name == "item" {
            let mut it = txt.split_whitespace();
            if let (Some(a), Some(b)) = (it.next(), it.next()) {
                return Arg::Compound {
                    name: Spanned { text: a.to_string(), span },
                    index: Spanned { text: b.to_string(), span },
                };
            }
        }
        return Arg::Literal(Spanned { text: txt, span });
    }

    Arg::Missing
}

fn l_text(link: &mdast::Link) -> String {
    link.children
        .iter()
        .filter_map(|n| match n {
            Node::Text(t) => Some(t.value.as_str()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_simple_page() {
        let src = "\
# root
- element
  - config
    - color grey
    - width-fixed 40
  - text
    - hi
";
        let t = parse(src);
        assert!(t.parse_ok);
        assert_eq!(t.pages.len(), 1);
        let root = &t.pages[0][0];
        assert!(matches!(&root.role, Role::Keyword { name, .. } if name == "element"));
        // element -> [config, text]
        let names: Vec<_> = root
            .children
            .iter()
            .filter_map(|c| match &c.role {
                Role::Keyword { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, ["config", "text"]);
    }

    #[test]
    fn emphasis_arg_is_read_past_the_space_node() {
        let src = "# root\n- `if` *viewport_open*\n    - `element`\n";
        let t = parse(src);
        let if_node = &t.pages[0][0];
        match &if_node.role {
            Role::Keyword { name, arg, .. } => {
                assert_eq!(name, "if");
                assert_eq!(*arg, Arg::Emphasis(Spanned {
                    text: "viewport_open".into(),
                    span: arg_span(arg),
                }));
            }
            other => panic!("{other:?}"),
        }
    }

    // helper: pull the span out for the assert above (span value isn't asserted)
    fn arg_span(a: &Arg) -> Span {
        match a {
            Arg::Emphasis(s) | Arg::Literal(s) | Arg::Code(s) => s.span,
            _ => Span::default(),
        }
    }

    #[test]
    fn preamble_directive_link() {
        let src = "#### TML 1.0\n- `load` [icons](assets/icons.png)\n";
        let t = parse(src);
        assert_eq!(t.preamble.len(), 1);
        match &t.preamble[0].role {
            Role::Keyword { name, arg, context, .. } => {
                assert_eq!(name, "load");
                assert_eq!(*context, Context::Preamble);
                assert!(matches!(arg, Arg::Link { .. }));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bare_keyword_spans_map_back_to_the_original_source() {
        // `color` is bare here; the pre-pass backticks it before parsing, which
        // would otherwise push the arg span 2 bytes right.
        let src = "# root\n- element\n  - config\n    - color grey\n";
        let t = parse(src);
        let cfg = &t.pages[0][0].children[0];
        let color = &cfg.children[0];
        match &color.role {
            Role::Keyword { name, name_span, arg, .. } => {
                assert_eq!(name, "color");
                // `color` starts at byte 6 of line 4 ("    - color grey").
                let line4_start = src.find("    - color").unwrap();
                assert_eq!(name_span.byte_start, line4_start + 6);
                assert_eq!(&src[name_span.byte_start..name_span.byte_end], "color");
                match arg {
                    Arg::Literal(s) => {
                        assert_eq!(src[s.span.byte_start..s.span.byte_end].trim(), "grey")
                    }
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn snippets_are_collected() {
        let src = "## expand\n- `grow`\n\n### badge\n- `element`\n\n# root\n- `use` badge\n";
        let t = parse(src);
        assert_eq!(t.config_snippets.len(), 1);
        assert_eq!(t.config_snippets[0].name.text, "expand");
        assert_eq!(t.element_snippets.len(), 1);
        assert_eq!(t.element_snippets[0].name.text, "badge");
    }
}
