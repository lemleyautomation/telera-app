//! The keyword backtick pre-pass.
//!
//! A `.tmd` file may be written without the `` ` `` inline-code spans around
//! keywords - `- element` instead of `` - `element` ``. Before the Markdown is
//! parsed, [`add_missing_keyword_backticks`] walks the source line by line and,
//! for any list item whose **first word** is one of [`LEADING_KEYWORDS`] and
//! isn't already backticked, wraps that word in backticks. Fenced code blocks
//! pass through untouched. Both the runtime parser and the language server run
//! this pass so the two styles behave identically.

/// Every keyword the grammar looks for as the *first token* of a list item -
/// element, config, declaration and header directives. **Sorted** (binary
/// searched by [`add_missing_keyword_backticks`]).
///
/// Kept in sync by hand with the `match` arms of the layout-file parser
/// (`process_element` / `process_configs` / `process_variable` /
/// `process_header_directive`) and with [`crate::schema::KEYWORDS`] (a test in
/// `telera-app` asserts the three agree). `config` is included even though the
/// parser only recognises it positionally, so a backtick-free file still
/// round-trips to the canonical `` `config` `` form. Argument words that are
/// written as inline code but never *lead* an item (`min`, `max`, `x`, `y`,
/// `height`, and the alignment / attach-point value words) are deliberately
/// *not* in this list.
pub const LEADING_KEYWORDS: &[&str] = &[
    "align", "align-children-x", "align-children-y", "arc", "aspect-ratio", "attach-root",
    "attach-self", "attatch-parent",
    "bevel-highlight", "bevel-light-angle", "bevel-shade", "bevel-width",
    "bezier", "blur-radius", "blur-tint", "border-all", "border-bottom", "border-color",
    "border-in-between", "border-left", "border-right", "border-top", "calc", "canvas", "center",
    "center-x",
    "center-y", "child-gap", "circle", "clip-to-parent", "color", "config", "ctrl1-x", "ctrl1-y",
    "ctrl2-x", "ctrl2-y", "declarations", "element", "end-angle", "eye-x", "eye-y", "eye-z", "far",
    "fit", "fixed-square", "floating",
    "floating-dimensions-height", "floating-dimensions-width",
    "fn", "focus", "focused", "font", "font-color", "font-id", "font-size", "fov", "from", "from-x",
    "from-y", "get-bool", "get-color",
    "get-event", "get-image", "get-numeric", "get-text",
    "glow-blur", "glow-color", "glow-spread",
    "grow", "height-fit", "height-fit-max",
    "height-fit-min", "height-fixed", "height-grow", "height-grow-max", "height-grow-min",
    "height-percent", "horizontal", "hover", "hovered", "id-indexed", "if",
    "if-index", "if-index-not", "if-not", "image", "item", "key-event", "left-clicked",
    "left-dbl-clicked",
    "left-down", "left-pressed", "left-released", "left-tpl-clicked", "letter-spacing", "line",
    "line-height", "list", "load", "max-zoom",
    "middle-clicked", "middle-down", "middle-pressed", "middle-released",
    "min-zoom", "near", "no-clip", "offset-x", "offset-y",
    "ortho-height",
    "padding-all",
    "padding-bottom", "padding-left", "padding-right", "padding-top", "pan-x", "pan-y", "pointer",
    "pointer-capture",
    "pointer-pass-through", "radius", "radius-all", "radius-bottom-left", "radius-bottom-right",
    "radius-top-left", "radius-top-right", "render-window", "right-clicked", "right-down",
    "right-pressed",
    "right-released", "ring", "scroll-horizontal", "scroll-vertical", "set-bool", "set-color",
    "set-event", "set-image", "set-numeric",
    "set-text",
    "shader", "shader-color-1", "shader-color-2",
    "shader-param-1", "shader-param-2", "shader-param-3", "shader-param-4",
    "shader-param-5", "shader-param-6", "shader-param-7", "shader-param-8",
    "shadow-blur", "shadow-color", "shadow-offset-x", "shadow-offset-y", "shadow-spread",
    "start-angle", "target-x", "target-y", "target-z", "text", "thickness", "to", "to-x",
    "to-y", "unfocused", "unhovered", "up-x", "up-y", "up-z",
    "use", "vertical", "wheel", "width",
    "width-fit", "width-fit-max", "width-fit-min", "width-fixed", "width-grow", "width-grow-max",
    "width-grow-min", "width-percent", "world-height", "world-width", "wrap", "z-index", "zoom",
];

/// Pre-parsing pass: lets a layout file be written without the `` ` `` inline-code
/// spans around keywords. Walks the source line by line and, for any Markdown
/// list item whose **first word** is one of [`LEADING_KEYWORDS`], wraps that word
/// in backticks before the text is handed to the Markdown parser.
///
/// It is a no-op on a file that already uses backticks - an item whose content
/// starts with `` ` `` is left untouched - so it's safe to run unconditionally
/// and to mix both styles in one file.
///
/// Only the leading keyword is touched. Every config keyword now takes at most
/// one value, so nothing after the keyword needs backticks except a UV rect
/// (`` `image` *atlas* `[0,0,1,1]` ``). And because this runs with no grammar
/// context, a plain-text line - a `text` element's content, say - that happens
/// to start with a keyword word (`color`, `image`, `if`, ...) *will* be turned
/// into a config; write that line's backticks yourself (or reword it) to opt
/// out.
pub fn add_missing_keyword_backticks(source: &str) -> String {
    rewrite_with_map(source).text
}

/// A [`add_missing_keyword_backticks`] rewrite together with a map from byte
/// offsets in the rewritten text back to the original source. The language
/// server needs this because every span `markdown` reports is an offset into the
/// rewritten buffer, but diagnostics must point into what the user actually
/// typed.
#[derive(Clone, Debug, Default)]
pub struct Rewrite {
    /// The rewritten text (identical to [`add_missing_keyword_backticks`]).
    pub text: String,
    /// Backtick runs the pre-pass inserted, as `(offset_in_text, len)`, sorted
    /// by offset and non-overlapping.
    inserts: Vec<(usize, usize)>,
}

impl Rewrite {
    /// Map a byte offset in [`Rewrite::text`] back to a byte offset in the
    /// original source. An offset that lands *inside* an inserted backtick run
    /// maps to the point in the original where that run was inserted.
    pub fn to_original(&self, offset: usize) -> usize {
        let mut shift = 0usize;
        for &(at, len) in &self.inserts {
            if at + len <= offset {
                shift += len;
            } else if at <= offset {
                return at - shift;
            } else {
                break;
            }
        }
        offset - shift
    }

    /// `true` when the pre-pass changed nothing (spans need no mapping).
    pub fn is_identity(&self) -> bool {
        self.inserts.is_empty()
    }
}

/// [`add_missing_keyword_backticks`], but also returning the offset map.
pub fn rewrite_with_map(source: &str) -> Rewrite {
    debug_assert!(
        LEADING_KEYWORDS.windows(2).all(|w| w[0] < w[1]),
        "LEADING_KEYWORDS must stay sorted"
    );

    let mut out = String::with_capacity(source.len() + 64);
    let mut inserts = Vec::new();
    let mut in_code_fence = false;

    for line in source.split_inclusive('\n') {
        let (text, newline) = match line.strip_suffix('\n') {
            Some(rest) => (rest.strip_suffix('\r').unwrap_or(rest), &line[rest.len()..]),
            None => (line, ""),
        };

        // ``` / ~~~ fenced code blocks pass straight through.
        let fence = text.trim_start();
        if fence.starts_with("```") || fence.starts_with("~~~") {
            in_code_fence = !in_code_fence;
            out.push_str(line);
            continue;
        }
        if in_code_fence {
            out.push_str(line);
            continue;
        }

        match backtick_leading_keyword_at(text) {
            Some((prefix_len, word_len)) => {
                let base = out.len();
                out.push_str(&text[..prefix_len]);
                out.push('`');
                inserts.push((base + prefix_len, 1));
                out.push_str(&text[prefix_len..prefix_len + word_len]);
                out.push('`');
                inserts.push((base + prefix_len + 1 + word_len, 1));
                out.push_str(&text[prefix_len + word_len..]);
                out.push_str(newline);
            }
            None => out.push_str(line),
        }
    }

    Rewrite { text: out, inserts }
}

/// The per-line worker for [`add_missing_keyword_backticks`]. Returns the
/// rewritten line if `line` is a list item whose first word is a keyword and
/// isn't already backticked, otherwise `None` (leave the line as-is).
pub fn backtick_leading_keyword(line: &str) -> Option<String> {
    let (prefix_len, word_len) = backtick_leading_keyword_at(line)?;
    Some(format!(
        "{}`{}`{}",
        &line[..prefix_len],
        &line[prefix_len..prefix_len + word_len],
        &line[prefix_len + word_len..],
    ))
}

/// The location of a bare leading keyword in `line`: `(byte offset of the first
/// letter, keyword length)`, or `None` if `line` isn't a list item leading with
/// an un-backticked keyword.
fn backtick_leading_keyword_at(line: &str) -> Option<(usize, usize)> {
    let indent_len = line.len() - line.trim_start().len();
    let after_indent = &line[indent_len..];
    let bytes = after_indent.as_bytes();

    // Bullet marker: `-`/`*`/`+`, or an ordered `12.` / `12)`.
    let marker_len = match bytes.first()? {
        b'-' | b'*' | b'+' => 1,
        b'0'..=b'9' => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            match bytes.get(digits) {
                Some(b'.') | Some(b')') => digits + 1,
                _ => return None,
            }
        }
        _ => return None,
    };

    // At least one space/tab between the marker and the content.
    let after_marker = &after_indent[marker_len..];
    if !after_marker.starts_with([' ', '\t']) {
        return None;
    }
    let content = after_marker.trim_start_matches([' ', '\t']);
    if content.is_empty() || content.starts_with('`') {
        return None;
    }

    let word_end = content.find([' ', '\t']).unwrap_or(content.len());
    let word = &content[..word_end];
    if LEADING_KEYWORDS.binary_search(&word).is_err() {
        return None;
    }

    Some((line.len() - content.len(), word_end))
}
