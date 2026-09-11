//! The keyword schema — the single source of truth for what every TML keyword
//! means, where it is valid, and what argument it takes.
//!
//! [`KEYWORDS`] has one entry per keyword (sorted by name, matching
//! [`crate::prepass::LEADING_KEYWORDS`]). A `telera-app` test asserts the two
//! agree and that each entry's `Context` set matches the parser's actual match
//! arms; the language server drives completion, hover and diagnostics off this
//! table.

/// Where in a `.tmd` file a keyword may lead a list item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// The list under a `#### TML …` header.
    Preamble,
    /// A body list item (a page, a `list`/`item`/`use`/`if` body, a shape's
    /// children, ...).
    Element,
    /// Inside a `config` block, or an event-gate's nested config list.
    Config,
    /// Inside a `declarations` block, or a `use` body's inline bindings.
    Declaration,
    /// An event-gate keyword: opens a nested config list gated on the condition.
    EventGate,
}

/// The shape of a keyword's argument (what follows it on the same line).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgShape {
    /// No argument (`grow`, `vertical`, `clip-to-parent`, ...).
    None,
    /// One number - a literal (`FromStr`) or an `*emphasis*` numeric name.
    Number,
    /// A color - `Color::from_str` (a named color or `rgb()`/`rgba()`) or an
    /// `*emphasis*` color name.
    Color,
    /// Exactly one bare word from [`KeywordDef::enum_words`].
    Enum,
    /// One of the nine anchor words (`top-left` … `bottom-right`, `center`).
    Anchor,
    /// A runtime-resolved name, written bare or as `*emphasis*` (both resolve
    /// the same way: local declaration first, then an app field/method).
    Name,
    /// A name that must be written with `*emphasis*` (`fn *name*`).
    NameEmphasis,
    /// An optional trailing bare id (an `element`/shape name). Not resolved.
    OptionalId,
    /// Plain text used verbatim (`set-text` value).
    Text,
    /// A Markdown link `[text](url)` (preamble `load` / `font` / `shader`).
    Link,
    /// An inline code span holding a `calc` function definition.
    CalcFnDef,
    /// The `image` / `set-image` positional form: `name` | `*name*` |
    /// `*atlas* [u,v,u,v]`.
    Image,
    /// A `Vec<T>` field name (`list <name>`, `item <name> …`).
    ListName,
    /// The compound `<list-name> <index>` argument of `item`.
    ItemArg,
    /// A number, or an inline `` `calc` `` code span (`set-numeric` value).
    SetNumericValue,
    /// An optional event-handler name (bare or `*emphasis*`); absent = gate
    /// only, dispatch nothing.
    EventHandler,
}

/// Whether a keyword actually does anything at runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Stable,
    /// Parses and round-trips, but the runtime never acts on it. The `&str` is
    /// the reason (shown as a diagnostic).
    Incomplete(&'static str),
}

/// One keyword.
#[derive(Clone, Copy, Debug)]
pub struct KeywordDef {
    pub name: &'static str,
    /// Each `(context, arg-shape-in-that-context)` the keyword is valid in.
    /// Most keywords have exactly one; a few (`shader`, `grow`, `use`) differ
    /// by context.
    pub slots: &'static [(Context, ArgShape)],
    pub doc: &'static str,
    pub status: Status,
    /// The allowed words for [`ArgShape::Enum`] (empty otherwise). For
    /// [`ArgShape::Anchor`] the words are always [`ANCHORS`].
    pub enum_words: &'static [&'static str],
}

impl KeywordDef {
    pub fn valid_in(&self, ctx: Context) -> bool {
        self.slots.iter().any(|(c, _)| *c == ctx)
    }
    pub fn arg_in(&self, ctx: Context) -> Option<ArgShape> {
        self.slots.iter().find(|(c, _)| *c == ctx).map(|(_, a)| *a)
    }
}

/// Look up a keyword by exact name (binary search).
pub fn by_name(name: &str) -> Option<&'static KeywordDef> {
    KEYWORDS
        .binary_search_by(|k| k.name.cmp(name))
        .ok()
        .map(|i| &KEYWORDS[i])
}

/// Every keyword valid in `ctx`.
pub fn in_context(ctx: Context) -> impl Iterator<Item = &'static KeywordDef> {
    KEYWORDS.iter().filter(move |k| k.valid_in(ctx))
}

use ArgShape::*;
use Context::*;
use Status::Stable;

const NO: &[&str] = &[];

/// Helper: one config-context slot with the given arg shape.
macro_rules! cfg {
    ($arg:expr) => {
        &[(Config, $arg)]
    };
}
macro_rules! kw {
    ($name:literal, $slots:expr, $doc:literal) => {
        KeywordDef { name: $name, slots: $slots, doc: $doc, status: Stable, enum_words: NO }
    };
    ($name:literal, $slots:expr, $doc:literal, $status:expr) => {
        KeywordDef { name: $name, slots: $slots, doc: $doc, status: $status, enum_words: NO }
    };
    ($name:literal, $slots:expr, $doc:literal, words: $words:expr) => {
        KeywordDef { name: $name, slots: $slots, doc: $doc, status: Stable, enum_words: $words }
    };
}

pub const ANCHORS: &[&str] = &[
    "top-left", "center-left", "bottom-left", "top-center", "center", "bottom-center", "top-right",
    "center-right", "bottom-right",
];

/// The full keyword table. **Sorted by `name`** — keep it that way (there is a
/// test) and keep it in sync with [`crate::prepass::LEADING_KEYWORDS`].
pub const KEYWORDS: &[KeywordDef] = &[
    kw!("align", cfg!(Enum), "text: paragraph alignment", words: &["left", "center", "right"]),
    kw!("align-children-x", cfg!(Enum), "horizontal alignment of children", words: &["left", "right", "center"]),
    kw!("align-children-y", cfg!(Enum), "vertical alignment of children", words: &["top", "bottom", "center"]),
    kw!("arc", &[(Element, OptionalId)], "a circular-arc shape drawn in the element's box"),
    kw!("aspect-ratio", cfg!(Number), "width:height ratio to hold when only one axis is constrained"),
    kw!("attach-root", cfg!(ArgShape::None), "floating: attach to the layout root instead of the parent"),
    kw!("attach-self", cfg!(Anchor), "floating: which point on the floating element is its attach point"),
    kw!("attatch-parent", cfg!(Anchor), "floating: which point on the parent the floating element attaches to (note the spelling)"),

    kw!("bevel-highlight", cfg!(Number), "raised_edge effect: highlight strength"),
    kw!("bevel-light-angle", cfg!(Number), "raised_edge effect: light direction, degrees"),
    kw!("bevel-shade", cfg!(Number), "raised_edge effect: shade strength"),
    kw!("bevel-width", cfg!(Number), "raised_edge effect: bevel width, logical px"),
    kw!("bezier", &[(Element, OptionalId)], "a cubic-bezier curve shape"),
    kw!("blur-radius", cfg!(Number), "blur effect: backdrop blur radius"),
    kw!("blur-tint", cfg!(Color), "blur effect: tint applied to the frosted backdrop"),
    kw!("border-all", cfg!(Number), "border width on all four sides"),
    kw!("border-bottom", cfg!(Number), "border width, bottom"),
    kw!("border-color", cfg!(Color), "border color"),
    kw!("border-in-between", cfg!(Number), "border drawn between children"),
    kw!("border-left", cfg!(Number), "border width, left"),
    kw!("border-right", cfg!(Number), "border width, right"),
    kw!("border-top", cfg!(Number), "border width, top"),

    kw!("calc", &[(Preamble, CalcFnDef)], "define a pure compile-time function: `name(a, b) = expr`"),
    kw!("canvas", &[(Element, OptionalId)], "an infinitely pannable, zoomable, clipping surface (state name = the element name)"),
    kw!("center", cfg!(Anchor), "arc: the arc centre, as an anchor word"),
    kw!("center-x", cfg!(Number), "arc: centre x, 0..1 fraction of the box"),
    kw!("center-y", cfg!(Number), "arc: centre y, 0..1 fraction of the box"),
    kw!("child-gap", cfg!(Number), "gap between children"),
    kw!("circle", &[(Element, OptionalId)], "a filled-disc shape inscribed in the element's box"),
    kw!("clip-to-parent", cfg!(ArgShape::None), "floating: clip this element to its attached parent's clip rect"),
    kw!("color", cfg!(Color), "background color"),
    kw!("config", &[(Element, ArgShape::None)], "opens an element's config block (must be its first body item)"),
    kw!("ctrl1-x", cfg!(Number), "bezier: first control point x, 0..1"),
    kw!("ctrl1-y", cfg!(Number), "bezier: first control point y, 0..1"),
    kw!("ctrl2-x", cfg!(Number), "bezier: second control point x, 0..1"),
    kw!("ctrl2-y", cfg!(Number), "bezier: second control point y, 0..1"),

    kw!("declarations", &[(Element, ArgShape::None)], "opens a block of named `set-*` / `get-*` data bindings"),
    kw!("element", &[(Element, OptionalId)], "the generic container element"),
    kw!("end-angle", cfg!(Number), "arc: sweep end angle, degrees clockwise from 3 o'clock"),
    kw!("eye-x", cfg!(Number), "render-window camera: eye position x"),
    kw!("eye-y", cfg!(Number), "render-window camera: eye position y"),
    kw!("eye-z", cfg!(Number), "render-window camera: eye position z"),
    kw!("far", cfg!(Number), "render-window camera: far clip-plane distance"),
    kw!("fit", cfg!(ArgShape::None), "fit to content on both axes"),
    kw!("fixed-square", cfg!(Number), "fixed width AND height, both set to this value"),
    kw!("floating", cfg!(ArgShape::None), "take the element out of flow; its nested config list configures the floating placement"),
    kw!("floating-dimensions-height", cfg!(Number), "fixed height for the floating box, independent of content"),
    kw!("floating-dimensions-width", cfg!(Number), "fixed width for the floating box, independent of content"),
    kw!("fn", &[(Element, NameEmphasis)], "call the app's `#[layout_element]` method of this name to build a subtree here"),
    kw!("focus", &[(EventGate, EventHandler)], "gate on / fire while this (named) element is the focused one"),
    kw!("focused", &[(EventGate, EventHandler)], "focus-gained edge", Status::Incomplete("focus-edge tracking is not implemented - the block never runs")),
    kw!("font", &[(Preamble, Link)], "load a `.ttf`/`.otf`: `[<font-id>](path)`; joins the fallback chain"),
    kw!("font-color", cfg!(Color), "text: font color"),
    kw!("font-id", cfg!(Number), "text: which loaded font to use"),
    kw!("font-size", cfg!(Number), "text: font size"),
    kw!("fov", cfg!(Number), "render-window camera: vertical field of view (degrees); switches to perspective"),
    kw!("from", cfg!(Anchor), "line/bezier: start point, as an anchor word"),
    kw!("from-x", cfg!(Number), "line/bezier: start x, 0..1 fraction of the box"),
    kw!("from-y", cfg!(Number), "line/bezier: start y, 0..1 fraction of the box"),

    kw!("get-bool", &[(Declaration, Name)], "bind a name to a `bool` app field, resolved every frame"),
    kw!("get-color", &[(Declaration, Name)], "bind a name to a `Color` app field, resolved every frame"),
    kw!("get-event", &[(Declaration, Name)], "bind a name to an event-handler app field, resolved every frame"),
    kw!("get-image", &[(Declaration, Name)], "bind a name to a `UIImageDescriptor` app field (or another image binding)"),
    kw!("get-numeric", &[(Declaration, Name)], "bind a name to a numeric app field, resolved every frame"),
    kw!("get-text", &[(Declaration, Name)], "bind a name to a `String` app field, resolved every frame"),
    kw!("glow-blur", cfg!(Number), "inner_glow effect: blur amount"),
    kw!("glow-color", cfg!(Color), "inner_glow effect: glow color"),
    kw!("glow-spread", cfg!(Number), "inner_glow effect: spread"),
    kw!("grow", &[(Element, ArgShape::None), (Config, ArgShape::None)], "grow to fill both axes (as an element: a flexible spacer)"),

    kw!("height-fit", cfg!(ArgShape::None), "fit height to content"),
    kw!("height-fit-max", cfg!(Number), "fit height to content, clamped to at most this"),
    kw!("height-fit-min", cfg!(Number), "fit height to content, at least this"),
    kw!("height-fixed", cfg!(Number), "fixed height"),
    kw!("height-grow", cfg!(ArgShape::None), "grow vertically to fill the parent"),
    kw!("height-grow-max", cfg!(Number), "grow vertically, clamped to at most this"),
    kw!("height-grow-min", cfg!(Number), "grow vertically, at least this"),
    kw!("height-percent", cfg!(Number), "height as a fraction (0..1) of the parent"),
    kw!("horizontal", cfg!(ArgShape::None), "stack children left-to-right (the default)"),
    kw!("hover", &[(EventGate, EventHandler)], "gate on / fire while the element is hovered this frame"),
    kw!("hovered", &[(EventGate, EventHandler)], "hover-entered edge", Status::Incomplete("hover-edge tracking is not implemented - the block never runs")),

    kw!("id-indexed", cfg!(Name), "like an element id but folds the current `list`/`item` index into the hash"),
    kw!("if", &[(Element, Name)], "gate the body on a `bool` (bare or `*emphasis*`)"),
    kw!("if-index", &[(Element, Name)], "gate the body on the current `list`/`item` index equalling this (a literal number or a numeric name)"),
    kw!("if-index-not", &[(Element, Name)], "gate the body on the current index NOT equalling this (a literal number or a numeric name)"),
    kw!("if-not", &[(Element, Name)], "gate the body on a `bool` being false"),
    kw!("image", cfg!(Image), "an image: `*name*` (resolved), bare `name`, or `*atlas* [u,v,u,v]` literal"),
    kw!("item", &[(Element, ItemArg)], "resolve one index into a `Vec<T>` field: `<list-name> <index>`"),

    kw!("key-event", &[(EventGate, EventHandler)], "fires while this (named) element is focused AND the window has queued key events"),

    kw!("left-clicked", &[(EventGate, EventHandler)], "hovered AND a left click completed this frame"),
    kw!("left-dbl-clicked", &[(EventGate, EventHandler)], "hovered AND a left double-click completed this frame"),
    kw!("left-down", &[(EventGate, EventHandler)], "hovered AND left button currently held"),
    kw!("left-pressed", &[(EventGate, EventHandler)], "hovered AND left button pressed this frame"),
    kw!("left-released", &[(EventGate, EventHandler)], "hovered AND left button released this frame"),
    kw!("left-tpl-clicked", &[(EventGate, EventHandler)], "triple-click", Status::Incomplete("triple-click tracking is not implemented - the block never runs")),
    kw!("letter-spacing", cfg!(Number), "text: extra spacing between letters", Status::Incomplete("the bundled renderer ignores letter spacing")),
    kw!("line", &[(Element, OptionalId)], "a straight-line shape"),
    kw!("line-height", cfg!(Number), "text: line height"),
    kw!("list", &[(Element, ListName)], "iterate a `Vec<T>` field, laying out the body once per item"),
    kw!("load", &[(Preamble, Link)], "load an image into an atlas: `[<atlas-name>](path)`"),

    kw!("max-zoom", cfg!(Number), "canvas: maximum zoom (re-applied every frame)"),
    kw!("middle-clicked", &[(EventGate, EventHandler)], "hovered AND a middle click completed this frame"),
    kw!("middle-down", &[(EventGate, EventHandler)], "hovered AND middle button currently held"),
    kw!("middle-pressed", &[(EventGate, EventHandler)], "hovered AND middle button pressed this frame"),
    kw!("middle-released", &[(EventGate, EventHandler)], "hovered AND middle button released this frame"),
    kw!("min-zoom", cfg!(Number), "canvas: minimum zoom (re-applied every frame)"),

    kw!("near", cfg!(Number), "render-window camera: near clip-plane distance"),
    kw!("no-clip", cfg!(ArgShape::None), "floating: do not inherit clipping from the attached element (the default)"),

    kw!("offset-x", cfg!(Number), "floating: x offset from the attach point"),
    kw!("offset-y", cfg!(Number), "floating: y offset from the attach point"),
    kw!("ortho-height", cfg!(Number), "render-window camera: world units shown vertically; switches to orthographic"),

    kw!("padding-all", cfg!(Number), "padding on all four sides"),
    kw!("padding-bottom", cfg!(Number), "padding, bottom"),
    kw!("padding-left", cfg!(Number), "padding, left"),
    kw!("padding-right", cfg!(Number), "padding, right"),
    kw!("padding-top", cfg!(Number), "padding, top"),
    kw!("pan-x", cfg!(Number), "canvas: initial pan x (physical px, applied once on creation)"),
    kw!("pan-y", cfg!(Number), "canvas: initial pan y (physical px, applied once on creation)"),
    kw!("pointer", cfg!(Enum), "cursor icon while this element is laid out", words: &["standard", "resize-horizontal"]),
    kw!("pointer-capture", cfg!(ArgShape::None), "floating: capture pointer events over the element's bounds"),
    kw!("pointer-pass-through", cfg!(ArgShape::None), "floating: let pointer events fall through to what's behind"),

    kw!("radius", cfg!(Number), "arc: radius as a 0..1 fraction of min(width,height)/2"),
    kw!("radius-all", cfg!(Number), "corner radius, all corners"),
    kw!("radius-bottom-left", cfg!(Number), "corner radius, bottom-left"),
    kw!("radius-bottom-right", cfg!(Number), "corner radius, bottom-right"),
    kw!("radius-top-left", cfg!(Number), "corner radius, top-left"),
    kw!("radius-top-right", cfg!(Number), "corner radius, top-right"),
    kw!("render-window", &[(Element, OptionalId)], "a hole in the UI that the 3D scene is drawn into through the camera of this name"),
    kw!("right-clicked", &[(EventGate, EventHandler)], "hovered AND a right click completed this frame"),
    kw!("right-down", &[(EventGate, EventHandler)], "hovered AND right button currently held"),
    kw!("right-pressed", &[(EventGate, EventHandler)], "hovered AND right button pressed this frame"),
    kw!("right-released", &[(EventGate, EventHandler)], "hovered AND right button released this frame"),
    kw!("ring", &[(Element, OptionalId)], "an unfilled-circle (outline) shape"),

    kw!("scroll-horizontal", cfg!(ArgShape::None), "clip + scroll on the x axis"),
    kw!("scroll-vertical", cfg!(ArgShape::None), "clip + scroll on the y axis"),
    kw!("set-bool", &[(Declaration, Text)], "bind a name to a literal `true`/`false`"),
    kw!("set-color", &[(Declaration, Text)], "bind a name to a literal color"),
    kw!("set-event", &[(Declaration, Text)], "bind a name to a literal event-handler name"),
    kw!("set-image", &[(Declaration, Image)], "bind a name to `*atlas* [u,v,u,v]`, or (no rect) alias another image binding"),
    kw!("set-numeric", &[(Declaration, SetNumericValue)], "bind a name to a number, or a folded `calc` expression"),
    kw!("set-text", &[(Declaration, Text)], "bind a name to a literal string"),
    kw!("shader", &[(Preamble, Link), (Config, Name)], "preamble: load a custom `.wgsl` effect. config: attach an effect (`drop_shadow`/`raised_edge`/`inner_glow`/`blur`/custom)"),
    kw!("shader-color-1", cfg!(Color), "custom shader: color slot 1 - carried to the shader packed as a single flat `u32` (`in.colors.x`, unpack with `unpack4x8unorm`), not as four more `shader-param-*` floats"),
    kw!("shader-color-2", cfg!(Color), "custom shader: color slot 2 (`in.colors.y`)"),
    kw!("shader-param-1", cfg!(Number), "custom shader: positional parameter 1"),
    kw!("shader-param-2", cfg!(Number), "custom shader: positional parameter 2"),
    kw!("shader-param-3", cfg!(Number), "custom shader: positional parameter 3"),
    kw!("shader-param-4", cfg!(Number), "custom shader: positional parameter 4"),
    kw!("shader-param-5", cfg!(Number), "custom shader: positional parameter 5"),
    kw!("shader-param-6", cfg!(Number), "custom shader: positional parameter 6"),
    kw!("shader-param-7", cfg!(Number), "custom shader: positional parameter 7"),
    kw!("shader-param-8", cfg!(Number), "custom shader: positional parameter 8"),
    kw!("shadow-blur", cfg!(Number), "drop_shadow effect: blur radius"),
    kw!("shadow-color", cfg!(Color), "drop_shadow effect: shadow color"),
    kw!("shadow-offset-x", cfg!(Number), "drop_shadow effect: x offset"),
    kw!("shadow-offset-y", cfg!(Number), "drop_shadow effect: y offset"),
    kw!("shadow-spread", cfg!(Number), "drop_shadow effect: spread"),
    kw!("start-angle", cfg!(Number), "arc: sweep start angle, degrees clockwise from 3 o'clock"),

    kw!("target-x", cfg!(Number), "render-window camera: look-at target x"),
    kw!("target-y", cfg!(Number), "render-window camera: look-at target y"),
    kw!("target-z", cfg!(Number), "render-window camera: look-at target z"),
    kw!("text", &[(Element, ArgShape::None)], "a text run; the content is the SECOND body bullet (after an optional `config` block)"),
    kw!("thickness", cfg!(Number), "ring/line/arc/bezier: stroke width, logical px (alias: `width`)"),
    kw!("to", cfg!(Anchor), "line/bezier: end point, as an anchor word"),
    kw!("to-x", cfg!(Number), "line/bezier: end x, 0..1 fraction of the box"),
    kw!("to-y", cfg!(Number), "line/bezier: end y, 0..1 fraction of the box"),

    kw!("unfocused", &[(EventGate, EventHandler)], "focus-lost edge", Status::Incomplete("focus-edge tracking is not implemented - the block never runs")),
    kw!("unhovered", &[(EventGate, EventHandler)], "hover-left edge", Status::Incomplete("hover-edge tracking is not implemented - the block never runs")),
    kw!("up-x", cfg!(Number), "render-window camera: up vector x"),
    kw!("up-y", cfg!(Number), "render-window camera: up vector y"),
    kw!("up-z", cfg!(Number), "render-window camera: up vector z"),
    kw!("use", &[(Element, Name), (Config, Name)], "inline a reusable snippet: `### name` (element form) or `## name` (config form)"),

    kw!("vertical", cfg!(ArgShape::None), "stack children top-to-bottom"),
    kw!("wheel", &[(EventGate, EventHandler)], "hovered AND the scroll wheel moved this frame - read the amount via `API::scroll_delta`"),
    kw!("width", cfg!(Number), "alias of `thickness` for ring/line/arc/bezier"),
    kw!("width-fit", cfg!(ArgShape::None), "fit width to content"),
    kw!("width-fit-max", cfg!(Number), "fit width to content, clamped to at most this"),
    kw!("width-fit-min", cfg!(Number), "fit width to content, at least this"),
    kw!("width-fixed", cfg!(Number), "fixed width"),
    kw!("width-grow", cfg!(ArgShape::None), "grow horizontally to fill the parent"),
    kw!("width-grow-max", cfg!(Number), "grow horizontally, clamped to at most this"),
    kw!("width-grow-min", cfg!(Number), "grow horizontally, at least this"),
    kw!("width-percent", cfg!(Number), "width as a fraction (0..1) of the parent"),
    kw!("world-height", cfg!(Number), "canvas: world-wrapper height that grow/fit children resolve against"),
    kw!("world-width", cfg!(Number), "canvas: world-wrapper width that grow/fit children resolve against"),
    kw!("wrap", cfg!(Enum), "text: wrap mode", words: &["words", "lines", "none"]),

    kw!("z-index", cfg!(Number), "floating: stacking order (may be negative)"),
    kw!("zoom", cfg!(Number), "canvas: initial zoom (applied once on creation)"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prepass::LEADING_KEYWORDS;

    #[test]
    fn keywords_sorted_and_unique() {
        for w in KEYWORDS.windows(2) {
            assert!(w[0].name < w[1].name, "KEYWORDS not sorted at {:?}", w[0].name);
        }
    }

    #[test]
    fn schema_covers_exactly_the_leading_keywords() {
        let schema: std::collections::BTreeSet<_> = KEYWORDS.iter().map(|k| k.name).collect();
        let lexical: std::collections::BTreeSet<_> = LEADING_KEYWORDS.iter().copied().collect();
        let missing: Vec<_> = lexical.difference(&schema).collect();
        let extra: Vec<_> = schema.difference(&lexical).collect();
        assert!(missing.is_empty(), "in LEADING_KEYWORDS but not in schema: {missing:?}");
        assert!(extra.is_empty(), "in schema but not in LEADING_KEYWORDS: {extra:?}");
    }

    #[test]
    fn enum_words_present_iff_enum_shape() {
        for k in KEYWORDS {
            let has_enum_slot = k.slots.iter().any(|(_, a)| *a == ArgShape::Enum);
            assert_eq!(
                has_enum_slot,
                !k.enum_words.is_empty(),
                "{}: enum_words / Enum shape mismatch",
                k.name
            );
        }
    }

    #[test]
    fn by_name_and_context_lookups() {
        assert_eq!(by_name("width-fixed").unwrap().arg_in(Config), Some(ArgShape::Number));
        assert_eq!(by_name("shader").unwrap().arg_in(Preamble), Some(ArgShape::Link));
        assert_eq!(by_name("shader").unwrap().arg_in(Config), Some(ArgShape::Name));
        assert!(by_name("grow").unwrap().valid_in(Element));
        assert!(by_name("grow").unwrap().valid_in(Config));
        assert!(by_name("nonsense").is_none());
        assert!(matches!(by_name("letter-spacing").unwrap().status, Status::Incomplete(_)));
    }
}
