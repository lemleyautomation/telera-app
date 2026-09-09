# TML (Telera Markdown Language) Specification

For the Rust side - the `App` trait, `run`, the macros, and the `API` methods -
see [`telera_api.md`](telera_api.md).

This document describes TML as implemented today by
`src/ui_renderer/layout_runner.rs` (the parser: `process_layout` /
`process_element` / `process_configs` / `process_variable`, and the runner:
`Binder::set_page` / `set_layout` / `execute_config` / `ResolveValue`). It is
a reference for the language surface that actually parses and runs, not an
aspirational design - where the code has a keyword or type that never gets
wired up to anything, that's called out explicitly in
[Known incomplete paths](#known-incomplete-paths) rather than presented as
working.

A `#### TML <version>` heading may appear (conventionally first) to open the
optional preamble block - see below. It is otherwise not required, and the
version string is not checked today; every `.md` file under an app's
configured layout directory is parsed the same way, by the same fixed grammar
described here.

## 1. Document shape

A TML file is a Markdown document read by the `markdown` crate into an AST
(`mdast`) and walked top to bottom. Four kinds of top-level heading matter:

- **A level-4 heading whose text starts with `TML`** (`#### TML 1.0`) opens
  the **preamble block**: the list under it is a flat sequence of directives
  that run once, when the file is parsed or its page replaced, before any
  layout. Four directives exist. Three are file-loading and share the
  `` `keyword` [text](path) `` Markdown-link shape: `` - `load` [atlas](path) ``
  (an image, see [Images](#images)); `` - `font` [id](path) `` - loads a
  `.ttf`/`.otf`, selected by `` `font-id` `` `id` and also added to the
  fallback chain (so a loaded emoji/symbol font is used automatically); and
  `` - `shader` [name](path.wgsl) `` - loads a custom UI effect shader,
  applied with the `` `shader` `` config keyword (see [Effects](#effects)).
  All paths are relative to the process's working directory. The fourth,
  `` - `calc` `name(a, b) = a * b` ``, defines a pure compile-time function -
  its argument is a backtick code span, not a link (see
  [Compile-time expressions](#compile-time-expressions-calc)). Unlike the
  file-loading directives, `` `calc` `` is collected regardless of where the
  header sits, so a function may be defined after the page that uses it.
- **A level-1 heading** (`# anything`) marks the start of the page body. The
  heading text itself is ignored - `# root` is the convention, but the page
  is actually named by whoever loads the file (`Binder::load_layout`/`API`
  use the file's own name, minus `.md`). Only the Markdown *list* that
  immediately follows becomes the page body; everything before the `#`
  heading (or between it and the list) is ignored by the parser.
- **A level-2 heading** (`## name`) opens a **reusable config snippet**: the
  list under it is parsed as a flat sequence of `Config`/hover-block
  commands (via `process_configs`) and stored under `name`. It holds config
  commands only - it can't open its own elements.
- **A level-3 heading** (`### name`) opens a **reusable element snippet**:
  the list under it is parsed as full element trees (via `process_element`)
  and stored under `name`.

Any other heading (a level-4+ heading not starting with `TML`, or a heading
with no plain-text first child) resets parsing to "ignore everything until
the next heading". Only one `List` node is consumed per heading - if a
heading is followed by something other than a list, nothing is parsed for it.

```markdown
## layout expand
- `width-grow`
- `height-grow`

### header button
- `element`
    - `config`
        - `color` rgb(140,140,140)
    - `text`
        - `config`
            - `font-size` 16
        - *label*

# root
- `element` outer container
    - `config`
        - `use` layout expand
    - `use` header button
        - `set-text` *label* Edit
```

## 2. Lexical building blocks

Every meaningful line is a Markdown list item whose paragraph starts with
**inline code** (the `` `keyword` `` backtick span) naming an element,
config, or declaration kind. What follows the inline code on the same line
supplies its argument(s); a nested (indented) list under the item supplies
its body (children, config block, or event-block configs).

### Backtick-free shorthand

Before the Markdown is parsed, a pre-pass (`add_missing_keyword_backticks`)
walks the source line by line: for any list item whose **first word** is a
recognised keyword and isn't already backticked, it wraps that word in
`` ` ``. So

```markdown
# root
- element
    - config
        - width-fixed 40
        - color grey
    - text
        - hello
```

parses identically to the fully-backticked form. The pass is a no-op on a
line that already starts with `` ` ``, so the two styles mix freely in one
file.

Caveats, both a consequence of the pass running with no grammar context:

- Only the **leading** keyword is touched. Every config keyword takes at most a
  single plain-text (or `*dynamic*`) value now, so the rest of the line is left
  alone; a UV rect like `` `image` *atlas* `[0,0,1,1]` `` still needs its own
  backticks around the bracketed list, and a `` `set-numeric` `` /
  `` `calc` `` expression that contains a `*` or `_` needs its own backticks
  around the expression (see
  [Compile-time expressions](#compile-time-expressions-calc)).
- A plain-text line that *happens* to start with a keyword word (a `text`
  element whose content begins with `color`, `image`, `if`, `line`, ...) will
  be turned into a config or element. Write that line's backticks yourself,
  or reword it, to opt out.

Lines inside a `` ``` `` / `~~~` fenced code block are left alone.

An argument after the keyword is one of two Markdown shapes, and the parser
expects a *specific* one per keyword (this isn't user-selectable per call
site):

- **Plain text** (`Node::Text`) - parsed straight off the paragraph's text
  run. What the parser *does* with it splits further by keyword:
  - For a config's value (a color, a number, an alignment word like `left`),
    it's a literal used as-is.
  - For a keyword whose argument is a *name* rather than a value - a `list`/
    `item`'s list name, an `if`/`if-not` condition, a `use` snippet name, a
    static event-block handler name - it's still
    resolved dynamically at runtime (against `bool`/numeric/etc.
    declarations or the field of the same name), same as the emphasis form
    below; it's just conventionally written bare, without `*...*`, because
    it's naming *which* field/snippet/list to use rather than substituting
    a value inline.
- **Emphasis** (`*name*`, i.e. Markdown italics) - the parser reads the name
  out of the emphasis span and stores it as an interned `GlobalSymbol` to
  resolve later, at *runtime*, against the current declarations/app data.
  This is the required form for a config's dynamic value (`*accent*` where
  a literal color could otherwise go), and for a declaration's own name
  (`` `set-text` *label* ... ``) - but, as above, some name-only keywords
  (`fn`) use it for a bare identifier too. There's no single rule that
  predicts emphasis-vs-plain from a keyword's meaning alone; it's called
  out per keyword below.

Which keywords take which shape is fixed by the parser; using the wrong one
silently drops the command (the `if let ... = ...` chain simply fails to
match and nothing is pushed).

### Name normalization

Whenever a dynamic name is resolved against the application (`get_bool`,
`get_numeric`, `get_text`, `get_color`, `get_image`, `get_event`, and the
per-item `field_*` equivalents), both the name written in the layout and the
Rust struct field name are normalized the same way before comparing
(`normalize_field_symbol`): lowercased, with spaces and hyphens folded to
underscores. So `*content background color*` matches a Rust field named
`content_background_color`, and an event name `file-menu-opened` matches a
handler/field named `file_menu_open`. This means spaces, hyphens, and
underscores are interchangeable in a dynamic name anywhere it's written in
TML.

### Colors

Wherever a config takes a color (`color`, `border-color`, `font-color`), the
static (plain-text) form is parsed by `Color::from_str` (from the
`telera-layout` crate) - named colors (`black`, `white`, `grey`, ...) and
`rgb(r,g,b)`/`rgba(r,g,b,a)` literals are the two forms it accepts.

## 3. Data binding: declarations

A `declarations` block is a list item whose own nested list holds named
bindings, each written as `` `keyword` *name* value ``:

```markdown
- `declarations`
    - `get-text` *title* document_title
    - `set-color` *accent* rgb(90,90,90)
```

The name (`*name*`) is always written with emphasis, regardless of whether
the binding itself is a `get-*` (dynamic) or `set-*` (static) kind - it's a
name being *defined* for this scope, not a value being resolved. The value
after it, and the naming convention, splits the eleven recognized keywords
into two families:

| Keyword | Value shape | Meaning |
|---|---|---|
| `get-bool` | plain text | binds `name` to another field, resolved dynamically via `LayoutRunnerReflection::get_bool` at render time using the *value* text as the field name |
| `get-numeric` | plain text | same, via `get_numeric` |
| `get-text` | plain text | same, via `get_text` |
| `get-color` | plain text | same, via `get_color` |
| `get-image` | plain text | same, via `get_image` |
| `get-event` | plain text | same, via `get_event` |
| `set-bool` | plain text, parsed as `bool` | binds `name` to a literal `true`/`false` fixed in the layout itself |
| `set-numeric` | plain text parsed as `f32`, **or** a backtick expression (`` `scale(w, 2) + 4` ``) | binds `name` to a number - a literal, or a [compile-time expression](#compile-time-expressions-calc) folded to a constant once, when the file is parsed |
| `set-text` | plain text | binds `name` to a literal string |
| `set-color` | plain text, parsed as a `Color` | binds `name` to a literal color |
| `set-event` | plain text | binds `name` to a literal handler name (looked up later via `LayoutReflector::dispatch_event`) |
| `set-image` | `*atlas*` then optional `[u1, v1, u2, v2]` | binds `name` to a `UIImageDescriptor` for atlas `atlas` (a name a `load` directive or the app staged), sampling the given UV sub-rectangle (default `[0, 0, 1, 1]`) - see [Images](#images) |

`set-image` is the one declaration whose value is itself written with
emphasis (`*atlas*`), since it names an atlas rather than carrying a plain
literal; the UV rect, if given, is ordinary trailing text.

(There is no `get-list`/`set-list` - a `Vec<T>` field is referenced directly
by name from `list`/`item`, not bound through `declarations` first.)

A binding created this way is then referenced elsewhere in the same scope
by writing `*name*` wherever that keyword's config/element normally expects
a dynamic value - e.g. a config's `color` config resolves `*accent*` above
by first checking whether `accent` is a page/list-scoped declaration (and if
so, following *its* `get-*`/`set-*` kind) before falling back to treating
`accent` itself as a field name straight on the application struct. So
`*name*` anywhere in TML is really "resolve `name`, checking local
declarations first, the app struct second" - a layout that skips
`declarations` entirely and writes `*some_field*` directly still works, as
long as `some_field` matches a real field.

Declarations placed directly in the page body (outside any `list`/`use`/
`item`) are visible to the *whole* page (`extract_page_locals`).
Declarations placed as the first item inside a `list`, or passed as the
body of a `use`/`item`, are scoped to that block only, though they still see
everything the enclosing scope declared (`merge_locals` - inner names win on
collision).

### The Rust side

An application struct normally gets `get_bool`/`get_numeric`/`get_text`/
`get_color`/`get_image`/`get_list_length` for free via
`#[derive(LayoutRunnerReflection)]` (`telera_macros`), which maps each
field's Rust type to the matching getter:

| Rust field type | Exposed as |
|---|---|
| `bool` | `get_bool` / (list item) `field_bool` |
| any integer or float primitive | `get_numeric` / `field_numeric` |
| `String` | `get_text` / `field_text` |
| `Color` | `get_color` / `field_color` |
| `UIImageDescriptor` | `get_image` / `field_image` |
| `Vec<T>` | `get_list_length` (and, if `T: FieldAccess`, per-item field lookups inside a `list`) |

A `Vec<T>` field's element type `T` derives `FieldAccess` instead (same
type-to-getter mapping, minus the list case) so a `list`'s body can resolve
each item's own fields. A `#[list_click_event(handler)]` attribute on the
`Vec<T>` field makes `left-clicked *Clicked*` inside that `list` resolve to
`handler`, with the current iteration index attached to the fired event's
`EventContext::list_index`.

Events and `fn *name*` custom elements are wired up by `#[telera_app]` on
the app's own `impl` block, which marks methods `#[layout_event]` (`fn
name(&mut self, context: Option<EventContext>, api: &mut API)`) or
`#[layout_element]` (`fn name(&mut self, api: &mut API, mt: &mut MT)`) and
generates the `LayoutReflector::dispatch_event`/`dispatch_custom_element`
match arms that route a name straight to that method.

### Compile-time expressions (`calc`)

A `` `set-numeric` `` whose value is a backtick code span is an **expression
evaluated once, while the file is parsed**, and folded to a constant. It has
no runtime cost and no runtime surface - the result lands in the declaration
as an ordinary static number, indistinguishable from a literal.

```markdown
#### TML 1.0
- `calc` `scale(a, b) = a * b`
- `calc` `golden(x) = x * 1618 / 1000`

# root
- `declarations`
    - `set-numeric` *base* 4
    - `set-numeric` *pad* `scale(base, base)`
    - `set-numeric` *panel_w* `golden(base) + 200`
- `element`
    - `config`
        - `padding-all` *pad*
        - `width-fixed` *panel_w*
```

**Why the backticks are required.** A bare `*` or `_` is Markdown emphasis, so
`2 * 45 / 34 * (56 + n)` would be mangled into `<em>` runs before the parser
ever sees it. Wrapping the expression in a `` `code span` `` (the same rule a
UV rect follows) keeps it verbatim. A value with no such characters
(`scale(a, 4)`, `base / 8`) also works un-backticked, but backticking every
expression is the convention.

**Grammar.** Arithmetic only - there are no built-in functions; `min`,
`clamp`, ... are whatever the file defines with `` `calc` ``.

```text
expr    := term (('+' | '-') term)*
term    := unary (('*' | '/' | '%') unary)*
unary   := '-' unary | primary
primary := number | ident | ident '(' (expr (',' expr)*)? ')' | '(' expr ')'
```

| Operators | Precedence | Associativity |
|---|---|---|
| unary `-` | highest | - |
| `*` `/` `%` | middle | left |
| `+` `-` | lowest | left |

`%` is float remainder. Division or remainder by zero is an error (not folded
to `inf`/`NaN`).

**Functions.** `` - `calc` `name(p1, p2) = <expr>` `` in the `#### TML` header
defines a pure function: its body sees only its own parameters (never the
caller's declarations) and may call other `` `calc` `` functions. Header
position does not matter - functions are collected before any page is walked.
A cyclic definition trips a recursion limit and the offending value is
dropped.

**What an expression may reference.** Numeric literals; `` `calc` ``
functions; and other `` `set-numeric` `` bindings **declared textually earlier
in the same or an enclosing scope**. It may **not** reference `get-*`
bindings, application fields, `list`/`item` fields, or `use` parameters -
those resolve at render time, and using one is an error. An identifier is
matched case-insensitively, but **`-` inside an expression is always
subtraction** - a declaration whose name has spaces or hyphens
(`*content background width*`) must be referenced by its normalized form with
underscores (`content_background_width`).

**Scope.** Same frames as declarations: page body, a `list`'s leading
`declarations`, and `set-numeric` `use` parameters each add a frame; forward
references within a frame fail. Inside a `##`/`###` reusable snippet only
functions and literals are visible (a snippet has no single compile-time
value for its call-site locals).

**Errors.** Any failure - syntax, unknown name, arity, divide-by-zero,
recursion, a runtime reference - prints a `TML calc:` line to stderr and
drops that one binding; the file still loads. A dropped `` `set-numeric` ``
leaves `*name*` resolving to `0.0` (the normal "missing numeric" fallback).

## 4. Elements

Every element keyword opens a nested Markdown list holding, in order: an
optional `config` block, then the element's body (child elements, or a text
value). All elements below live inside a body list (the page body, a `list`/
`item`/`use`/`if` body, ...).

### `element`

```markdown
- `element` optional-id
    - `config`
        - ...config commands...
    - `element` ...
    - `text`
        - ...
```

The generic container. `optional-id` (plain text, no dash needed before it)
sets the element's Clay id via `Config::Id` - used by `element` (and the
drawn shapes `circle`/`ring`/`line`/`arc`/`bezier`, below) whenever
it's useful to name an element, though nothing in the current config surface
can *reference* another element's id yet (see
[Known incomplete paths](#known-incomplete-paths)). Inside a `list`/`item`,
the `id-indexed` config keyword sets an id that folds in the current
iteration index, so repeated rows don't collide. Everything after the
optional `config` block is processed as child elements.

There is also a bare `grow` element shorthand - `` - `grow` `` with no body
at all - that opens and immediately closes an empty element configured with
`Config::GrowAll`, useful as a flexible spacer.

### `text`

```markdown
- `text`
    - `config`
        - `font-size` 16
        - `font-color` white
    - *title*
```

Its own `config` block is parsed by the same `process_configs` as an
element's, but only the text-related keywords have any effect here (`font-id`,
`font-size`, `font-color`, `line-height`, `letter-spacing`, `align`, `wrap`) -
see [Config reference](#5-config-reference); any non-text keyword resolves
against a throwaway `ElementConfiguration` and is discarded. The line right
after `config` is the text content itself: `*name*` resolves it dynamically
(via `String`'s `ResolveValue`, ultimately `get_text`/declarations), plain
text is used verbatim.

### `circle` / `ring` / `line` / `arc` / `bezier` (drawn shapes)

Each is shaped like `element` (own optional id, own `config` block, and -
like `element` - it may nest child elements after that block) and draws a
single primitive filling, or inscribed in, the element's bounding box,
stroked/filled with the element's `color`. Size the bounding box the usual
way (`width-fixed` / `height-fixed` / `grow` / ...); the shape follows it.

A shape's children are laid out inside it like any container and drawn on
top of it, so nesting shapes stacks them: an `arc` whose child is another
`arc` (each `grow`ing to fill the box) draws two concentric sweeps - a track
plus a value - which is how an RPM-style gauge is built. See
`examples/layouts/shapes.md`.

| element | draws |
|---|---|
| `circle` | filled disc inscribed in the box |
| `ring` | unfilled circle (outline) inscribed in the box |
| `line` | straight line between two points (default: vertical, down the centre) |
| `arc` | circular arc |
| `bezier` | cubic bezier curve |

**Points** (`line`/`bezier` endpoints and control points, `arc` centre) are
normalised `0..1` fractions of the bounding box - `0,0` is the top-left
corner, `1,1` the bottom-right - so a shape tracks its box as the layout
resizes. Give a point with the `-x` / `-y` keywords, or name one of the nine
`attatch-parent` anchor words with `from` / `to` / `center`:

```markdown
- `line`
    - `config`
        - `width-fixed` 80
        - `height-fixed` 80
        - `color` grey
        - `from` top-left
        - `to` bottom-right
        - `thickness` 3
```

Shape-config keywords (all take a single static-or-`*dynamic*` number, like
the old `line` `width`; they only apply to the shape they belong to and are
ignored elsewhere):

| Keyword | Shapes | Meaning |
|---|---|---|
| `thickness` (alias `width`) | `ring` `line` `arc` `bezier` | stroke width in logical px |
| `from` / `to` | `line` `bezier` | endpoint, as an anchor word (`top-left`, `center`, `bottom-right`, ...) |
| `center` | `arc` | arc centre, as an anchor word |
| `from-x` / `from-y` / `to-x` / `to-y` | `line` `bezier` | endpoint coordinate, `0..1` fraction of the box |
| `ctrl1-x` / `ctrl1-y` / `ctrl2-x` / `ctrl2-y` | `bezier` | the two cubic control points, `0..1` fractions |
| `center-x` / `center-y` | `arc` | arc centre coordinate, `0..1` fraction |
| `radius` | `arc` | `0..1` fraction of `min(width, height) / 2` (`1.0` = inscribed) |
| `start-angle` / `end-angle` | `arc` | degrees, clockwise from the 3 o'clock position |

Bare keyword defaults: `line` is a 1px vertical line down the centre
(unchanged from before shapes were expanded); `ring` a 1px inscribed
outline; `arc` a 1px inscribed half-circle opening downward; `bezier` a 1px
symmetric arch between the bottom corners.

### `render-window`

Shaped like the drawn shapes above (own `config` block, may nest children),
but instead of drawing anything it is a **hole in the UI** through which the
3D scene is rendered - into this element's bounding box, using its own
`Camera`. Size and place the box the usual way; the scene fills it.

The camera is named by the element's name (`render-window cockpit` uses
camera `cockpit`); an un-named `render-window` uses the camera `default`. A
name that has no camera yet is created (at a default framing). Cameras are
never auto-removed - a camera no `render-window` references this frame just
isn't drawn. App code reaches the same cameras via `api.camera("cockpit")` (returns
`Option<&mut Camera>`) - `Camera` has `pan` / `orbit` / `zoom` / `dolly` /
`frame` / `perspective` / `orthographic` / `clip_planes` convenience methods -
plus `api.add_camera` / `api.remove_camera`. With no `render-window` anywhere
in a frame, the `default` camera fills the whole window.

Camera keywords (each a single static-or-`*dynamic*` number; an omitted one
leaves that camera value as app code or a previous frame set it, so a
camera's parameters can be split between the layout and Rust):

| Keyword | Sets |
|---|---|
| `eye-x` / `eye-y` / `eye-z` | camera position in scene space |
| `target-x` / `target-y` / `target-z` | the point the camera looks at |
| `up-x` / `up-y` / `up-z` | the camera's up vector (default `0,1,0`) |
| `fov` | vertical field of view, degrees - also switches to a **perspective** projection |
| `ortho-height` | world units shown vertically - also switches to an **orthographic** projection (width follows the rect's aspect) |
| `near` / `far` | near / far clip plane distances |

Nothing opaque in the UI may sit *over* a `render-window`'s rect or it hides
the scene - the region must be transparent all the way down (no ancestor
`color`). `render-window` children, though, are drawn on top of its scene
view (a HUD). Because clipping is per-rect, keyboard-driven camera movement
was removed: drive cameras from `update()` or from these keywords.

```markdown
- `element` panes
    - `config`
        - `grow`
        - `horizontal`
    - `render-window` orbit
        - `config`
            - `grow`
    - `render-window` front
        - `config`
            - `grow`
            - `eye-z` *front_z*
            - `fov` 50
```

See `examples/scene.rs` + `examples/layouts/scene.md`.

### `list` / `item`

`list` iterates a `Vec<T>` field the whole way, once per element, laying out
its body once per item:

```markdown
- `list` Documents
    - `declarations`
        - `get-text` *heading* title
    - `element`
        - `text`
            - *title*
```

The list name (`Documents` above) is **plain text**, matched (after
normalization) against a `Vec<T>` field. An optional leading `declarations`
block scopes bindings to the list body (and can shadow outer ones); every
other child is the per-item body, replayed once per item with that item's
index attached, so `*field*` inside it resolves against the current item
(via `FieldAccess`) rather than the app itself.

`item` resolves a *single* index into a list once, instead of iterating:

```markdown
- `item` Documents selected_document
    - `text`
        - *title*
```

Its argument is `list-name index`, whitespace-separated in one plain-text
run. `index` is either a bare number (`item Documents 0`, a static index)
or an identifier (`selected_document`, a numeric field resolved
dynamically) - see `parse_index_arg`.

### `if` / `if-not` / `if-index` / `if-index-not`

```markdown
- `if` file-menu-opened
    - `element`
        - ...
- `if-not` file-menu-opened
    - `text`
        - Closed
```

`if`/`if-not` gate their body on a `bool` (a declaration or, directly, a
`bool` field) named in **plain text** (no emphasis). `if-index`/
`if-index-not` instead compare the current `list`/`item` iteration index
against a number or dynamic numeric field (same argument shape as `item`'s
index); outside any `list`/`item` they're always false (`if-index`) or
always true (`if-index-not`), since there's no current index to compare.

### `use`

Both a reusable *config* snippet (`## name`) and a reusable *element*
snippet (`### name`) are invoked the same way, from wherever that kind of
snippet is legal (a `config` block for the former, an element's body for
the latter):

```markdown
- `use` header button
    - `set-text` *label* Edit
```

The snippet name is plain text. Its body (if any) is a `declarations`-style
list of `set-*`/`get-*` bindings - not wrapped in a `declarations` list item
itself - that become the invocation's locals, letting one `### name`
snippet be reused with different text/colors/etc per call site. A `## name`
config snippet, invoked with `` `use` name `` from inside a `config` block,
takes no such parameters - it's just replayed inline (flat `Config`
commands only; it can't itself open a `hover`/`left-clicked`/... block from
that position).

### `fn`

```markdown
- `fn` *custom_element*
```

Calls back into the application's `LayoutReflector::dispatch_custom_element`
right at this point in the tree, so the handler can add whatever it wants
via `api.l` itself. Unlike every other identifier-only keyword, the name
here is written with emphasis despite being an identifier, not a value to
resolve against declarations - `dispatch_custom_element` always receives it
literally.

## 5. Config reference

Config commands live inside a `config` block (`element`, a drawn shape
`circle`/`ring`/`line`/`arc`/`bezier`, or a `render-window`) or a text
element's own `config` block
(text-only subset). Each row below
gives the keyword, its argument shape, and what it sets. "single" means one
value, static (plain text, parsed via `FromStr`) or dynamic (`*name*`). Every
keyword takes at most one value; the clamped-sizing keywords come as a family
(`width-grow`, `width-grow-min`, `width-grow-max`), and writing more than one
member folds them into a single sizing rule.

| Keyword | Shape | Effect |
|---|---|---|
| `grow` | none | element grows to fill both axes |
| `width-grow` / `width-grow-min` / `width-grow-max` | none / single (number) | grow horizontally; the `-min` / `-max` members clamp it (combine both for a range) |
| `height-grow` / `height-grow-min` / `height-grow-max` | none / single (number) | grow vertically, same family shape |
| `fit` | none | fit to content on both axes |
| `width-fit` / `width-fit-min` / `width-fit-max` | none / single (number) | fit to content horizontally, optionally clamped |
| `height-fit` / `height-fit-min` / `height-fit-max` | none / single (number) | fit to content vertically, optionally clamped |
| `width-fixed` | single (number) | fixed width |
| `height-fixed` | single (number) | fixed height |
| `fixed-square` | single (number) | fixed width *and* height, both set to the same value |
| `width-percent` | single (number) | width as a percentage of the parent |
| `height-percent` | single (number) | height as a percentage of the parent |
| `aspect-ratio` | single (number) | width\:height ratio to hold when only one axis is constrained (e.g. an image) |
| `padding-all` | single (number) | padding on all four sides |
| `padding-top`/`-bottom`/`-left`/`-right` | single (number) | padding on one side |
| `child-gap` | single (number) | gap between children |
| `vertical` | none | stack children top-to-bottom |
| `horizontal` | none | stack children left-to-right (the default; useful to override a `vertical` inherited from a reusable) |
| `align-children-x` | plain text: `left`/`right`/`center` | horizontal alignment of children |
| `align-children-y` | plain text: `top`/`bottom`/`center` | vertical alignment of children |
| `color` | single (color) | background color |
| `radius-all` | single (number) | corner radius, all corners |
| `radius-top-left`/`-top-right`/`-bottom-left`/`-bottom-right` | single (number) | corner radius, one corner |
| `border-color` | single (color) | border color |
| `border-all` | single (number) | border width, all sides |
| `border-top`/`-left`/`-bottom`/`-right` | single (number) | border width, one side |
| `border-in-between` | single (number) | border drawn between children |
| `scroll-horizontal` / `scroll-vertical` | none | enables clipping/scrolling on that axis; use both keywords for both axes. The child offset is driven automatically from the layout engine's scroll state each frame. |
| `id-indexed` | single (name) | like an `element` id, but folds the current `list`/`item` iteration index into the hash so each row gets a distinct id (no-op outside a list) |
| `image` *name* / `image` name | single (name), emphasised or bare | resolve `name` to a `UIImageDescriptor` - a `set-image` declaration first, then the app's `get_image` (see [Images](#images)) |
| `image` *atlas* `[u1, v1, u2, v2]` | name then a bracketed 4-float UV rect | an inline literal descriptor - no lookup; `atlas` is a name a `load` directive or the app staged |
| `floating` | none, with a nested config list | takes the element out of flow; the nested list's commands (`offset-x`/`offset-y`, `attatch-parent`, `attach-self`, the floating-only keywords below, plus ordinary configs) configure the floating placement |
| `offset-x` / `offset-y` (inside `floating`) | single (number) | floating offset from its attach point on that axis (the unwritten axis is `0`) |
| `floating-dimensions-width` / `floating-dimensions-height` | single (number) | fixed size for the floating box on that axis, independent of its content |
| `z-index` | single (number, may be negative) | stacking order of the floating element |
| `attatch-parent` | plain text: `top-left`/`center-left`/`bottom-left`/`top-center`/`center`/`bottom-center`/`top-right`/`center-right`/`bottom-right` | which point on the parent (or root) the floating element attaches to |
| `attach-self` | same nine values | which point on the floating element itself is the attach point |
| `attach-root` | none | attach to the layout root instead of the parent |
| `clip-to-parent` | none | clip the floating element to the same rectangle as the element it is attached to |
| `no-clip` | none | do not inherit clipping from the attached element (the default) |
| `pointer-capture` | none | the floating element captures pointer events over its bounds |
| `pointer-pass-through` | none | pointer events fall through the floating element to whatever is behind it |
| `use` name | plain text | inline a `## name` reusable config snippet here |
| `hover` / `unhovered`* / `hovered`* | none, or single (event name), with a nested config list | opens a block whose configs only apply while the condition holds; see [Events](#6-events) |
| `focus` | same shape as `hover` | gates its block (and fires its event) while this (named) element is the focused one |
| `focused`* / `unfocused`* | same shape as `hover` | focus-*edge* events - see [Known incomplete paths](#known-incomplete-paths) |
| `key-event` | same shape as `hover` | fires while this (named) element is focused **and** the window has unread key events this frame; the handler reads `api.key_events()`. Focus is checked by the runner - no need to nest it in `focus`. |
| `left-pressed`/`left-down`/`left-released`/`left-clicked`/`left-dbl-clicked` | same shape as `hover` | mouse-button event blocks |
| `left-tpl-clicked`* | same shape | parsed but never fires (no triple-click tracking) |
| `right-pressed`/`right-down`/`right-released`/`right-clicked` | same shape as `hover` | right mouse-button event blocks |
| `pointer` | plain text: `standard`/`resize-horizontal` | sets the cursor icon while this element (or an ancestor, since it's not gated) is being laid out |
| `font-id` | single (number) | text: font id |
| `font-size` | single (number) | text: font size |
| `font-color` | single (color) | text: font color |
| `line-height` | single (number) | text: line height |
| `letter-spacing` | single (number) | text: extra spacing between letters (carried through to `TextConfig::letter_spacing`; see the note in [Known incomplete paths](#known-incomplete-paths) about renderer support) |
| `align` | plain text: `left`/`center`/`right` | text: paragraph alignment |
| `wrap` | plain text: `words`/`lines`/`none` | text: wrap on whitespace (`words`), only on explicit newlines (`lines`), or never (`none`) |
| shape-config keywords (`thickness`/`width`, `from`/`to`/`center`, `from-x`, `radius`, `start-angle`, ...) | single (number) or anchor word | geometry of a drawn shape - see [`circle` / `ring` / `line` / `arc` / `bezier`](#circle--ring--line--arc--bezier-drawn-shapes); ignored outside the shape they belong to |
| camera keywords (`eye-x/-y/-z`, `target-x/-y/-z`, `up-x/-y/-z`, `fov`, `ortho-height`, `near`, `far`) | single (number) | position a `render-window`'s camera - see [`render-window`](#render-window); ignored elsewhere |
| `shader` | single (name), emphasised or bare | attach a fragment effect (`drop-shadow`/`raised-edge`/`inner-glow`, or a custom shader) - see [Effects](#effects) |
| effect keywords (`shadow-blur`, `shadow-color`, `shadow-offset-x/-y`, `shadow-spread`, `bevel-width`, `bevel-light-angle`, `bevel-highlight`, `bevel-shade`, `glow-blur`, `glow-spread`, `glow-color`, `blur-radius`, `blur-tint`, `shader-param-1`..`shader-param-8`) | single (number or color) | tune the open `shader` effect; ignored with no `shader` |

*Marked keywords parse into real `Element` commands but never actually
fire - see [Known incomplete paths](#known-incomplete-paths).

Any config keyword the parser doesn't recognize is silently ignored (the
outer `match` falls through to `_ => {}`), as is a recognized keyword given
an argument shape it doesn't expect (e.g. `color` written with plain text
where a valid `Color` literal wasn't parseable) - both fail silently rather
than erroring the whole file.

### Images

An image is a **`UIImageDescriptor`** - an atlas name plus a `[u1, v1, u2, v2]`
UV sub-rectangle (the whole image is `[0, 0, 1, 1]`). Putting one on an
element takes two steps: get the pixels into an atlas, then point an `image`
config at that atlas.

**Loading pixels.** Three ways, and a layout can mix them freely:

- **`` - `load` [atlas](path) `` in the `#### TML ...` preamble.** The parser
  records the request; when the file is parsed or its page replaced,
  `API::load_layout_file` reads `path` (relative to the process working
  directory, same base as the watch directory), decodes it, and stages it
  under `atlas` via `API::add_image`. Re-parsing the file re-stages (a
  changed image is picked up); an atlas already loaded from the same path by
  another file is skipped. No Rust code is involved.
- **`API::add_image(name, image)` from the app** (typically in
  `App::onload`) - registers a decoded `DynamicImage` under an atlas name.
  Calling it again with a name that already exists overwrites that atlas.
- Nothing stops both: `load` and `add_image` are the same staging path.

**Referencing an atlas.** In an element's `config` block:

- `` `image` *name* `` (or bare `` `image` name ``) resolves `name` to a
  descriptor: a `set-image` declaration in scope first, then the app's
  `get_image` (a `UIImageDescriptor` field, or a `get-image` declaration
  redirecting to one) - so an app that builds descriptors in Rust keeps
  working unchanged.
- `` `image` *atlas* [u1, v1, u2, v2] `` is an inline literal - atlas name
  plus UV rect, no lookup.
- `` `set-image` *name* *atlas* [u1, v1, u2, v2] `` in a `declarations` block
  binds `name` to such a literal so several elements can share it (and a
  `list`/`use` scope can shadow it).

```markdown
#### TML 1.0
- `load` [portrait](assets/portrait.png)

# root
- `declarations`
  - `set-image` *left half* *portrait* [0, 0, 0.5, 1]
- `element`
  - `config`
    - `image` *left half*
- `element`
  - `config`
    - `image` *portrait* [0.5, 0, 1, 1]
- `element`
  - `config`
    - `image` *app_provided_field*
```

### Effects

The `` `shader` `` config keyword attaches a fragment-level effect to the
element it sits on - any element with a background `color`, a border, an
`image`, or a `circle`/`ring` shape (not `text`, not `line`/`arc`/`bezier`).

Effects **stack**: repeat `` `shader` `` to add another; the tuning keywords
that follow bind to the most recently opened one. `drop-shadow` and `blur` paint
*behind* the element's fill, the rest *over* it (written order kept within each
group).

```markdown
- `element` card
  - `config`
    - `color` rgb(150,90,200)
    - `radius-all` 16
    - `shader` *drop-shadow*
    - `shadow-blur` *blur_amount*
    - `shadow-offset-y` 6
    - `shadow-color` rgba(0,0,0,0.55)
    - `shader` *raised-edge*
    - `bevel-width` 10
```

**Built-in effects.** `` `shader` `` takes the effect name (`*emphasised*` or
bare); the tuning keywords that follow are each a literal or a `*binding*`:

| `shader` | tuning keywords |
|---|---|
| `drop-shadow` (`shadow`) | `shadow-offset-x`, `shadow-offset-y`, `shadow-blur`, `shadow-spread`, `shadow-color` |
| `raised-edge` (`bevel`) | `bevel-width`, `bevel-light-angle` (degrees), `bevel-highlight`, `bevel-shade` |
| `inner-glow` (`glow`) | `glow-blur`, `glow-spread`, `glow-color` |
| `blur` | `blur-radius`, `blur-tint` - frosts the element by blurring the UI layer behind it (not the 3D scene); keep its content beside/below it, not inside |

Distances are logical px.

**Custom shaders.** A `` - `shader` [name](path.wgsl) `` preamble directive
loads a WGSL file that provides only `fs_main` (telera prepends the vertex
stage, bind groups, the `VertexPayload` struct and an `sd_rounded_box` helper).
Apply it the same way - `` `shader` *name* `` - and pass up to eight positional
values with `` `shader-param-1` `` .. `` `shader-param-8` `` (the shader reads
them as `in.params` / `in.params2`). A file that fails to load/validate is
logged and the element just renders without the effect. See
`examples/effects.rs` and `examples/shaders/glass.wgsl` / `neon.wgsl`.

## 6. Events

A `hover`/`left-clicked`/etc. keyword opens a **gated block**: everything in
its nested config list only takes effect while the named condition holds
*this frame* (`api.l.hovered()`, `api.left_mouse_clicked`, ...) - it's not a
one-shot toggle, it's re-evaluated every frame. Nesting is tracked so a
gated block can be skipped as a whole without hand-unwinding each command
inside it (`skip`/`nesting_level` in `set_layout`).

```markdown
- `left-clicked` document_clicked
    - `border-color` white
    - `border-all` 2
```

An event keyword's own argument (before its nested list) is optional and,
if given, is the handler to dispatch **the first frame the condition
becomes true** (an edge, not a level) - plain text for a literal handler
name, `*name*` for one resolved through declarations
(`get-event`/`set-event`). The nested configs, by contrast, apply for every
frame the condition holds, independent of whether a handler fired. Given no
argument, the block still gates its configs on the condition; it just never
dispatches anything.

Inside a `list`, a fired event's `EventContext::list_index` carries the
current iteration index, so a handler can tell which item was interacted
with (see `#[list_click_event(...)]` above).

The condition each keyword gates on:

| Keyword | Fires/gates on |
|---|---|
| `hover` | the element is hovered this frame |
| `left-pressed` | left mouse button pressed down this frame |
| `left-down` | left mouse button currently held |
| `left-released` | left mouse button released this frame |
| `left-clicked` | hovered *and* a left click completed this frame |
| `left-dbl-clicked` | hovered *and* a double-click completed this frame |
| `right-pressed`/`right-down`/`right-released`/`right-clicked` | same, right mouse button |
| `focus` | this (named) element is the focused element |
| `key-event` | this (named) element is focused *and* the window has key events queued this frame |

Focus follows the pointer: a left or right mouse-*down* focuses the topmost
**named** element under the cursor (only named elements are focusable), or
clears focus if that's empty space / an unnamed element. The runner tracks the
focused element by its name, reachable from Rust as `api.focus()`. A
capture-mode `floating` element blocks focus (and hover) from reaching whatever
it covers.

## Known incomplete paths

These are real keywords/types that parse and round-trip through `Layout`,
but the runner doesn't act on them - each is marked with a `// TODO` at its
match arm in the source. Documented here so they aren't mistaken for
working:

- `hovered`, `unhovered` - need hover-*edge* tracking (`API` only tracks the
  current hovered state, not the transition) - the block is always skipped.
- `focused`, `unfocused` - need focus-*edge* tracking (the runner tracks the
  current focused element, not the transition) - the block is always skipped.
  (`focus`, the "while focused" gate, does work.)
- `left-tpl-clicked` - `API` doesn't track triple-clicks - always skipped.
- `letter-spacing` - now parses into `Config::LetterSpacing` and is applied
  to `TextConfig::letter_spacing`, but the bundled renderer's text
  measurement / glyph layout (`UIRenderer`) ignores it, so it has no visible
  effect until a renderer that honors it is used.
- `Config::FloatingAttachElementToElement { other_element_id }` - always
  attaches to element id `0` regardless of `other_element_id`; the layout
  engine doesn't expose an id lookup yet. There is also no TML keyword that
  produces this variant today (no `attach-element` keyword is wired up).
- `CustomElement::RenderWindow` - a valid custom-element payload the
  renderer knows how to draw, but `process_element` never produces it (only
  `circle` and `line` map to a `CustomElement` from TML).
- `calc` expressions are only accepted as a `` `set-numeric` `` value. There
  is no `calc-color`, and no inline `calc(...)` directly on a config line -
  compute the value in a `` `set-numeric` `` and reference it by name.
- A `calc` expression sees only declarations written textually before it; a
  forward reference, or one across sibling `declarations` blocks, fails.
- A `calc` expression inside a `##`/`###` reusable snippet cannot see any
  declaration - only `` `calc` `` functions and literals.
