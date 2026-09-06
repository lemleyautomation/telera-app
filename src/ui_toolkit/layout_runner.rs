//! Markdown-based UI layout format.
//!
//! This module is the condensed replacement for the old `layout_types.rs` /
//! `markdown.rs` / `page_set.rs` trio. It is organized top to bottom as three
//! layers that used to live in separate files:
//!
//!   1. **Types** - [`Layout`], [`Element`], [`Config`], [`Declaration`],
//!      [`DataSrc`] and the [`ParserDataAccess`] trait an application
//!      implements to feed dynamic data into a layout.
//!   2. **Parser** - [`process_layout`] turns a markdown document (see
//!      `src/layouts/main.md` for an example) into a flat `Vec<Layout>`
//!      of layout commands plus a table of reusable snippets.
//!   3. **Runner** - [`Binder`] owns the parsed pages/reusables and
//!      [`Binder::set_page`] walks the flattened command list each frame,
//!      driving `api.l` (the Clay layout engine) and dispatching events
//!      straight back into the app as they fire.
//!
//! # Event dispatch
//!
//! Event handling used to fight the borrow checker because a mutable text
//! renderer (`MT`) and mutable element/text config scratch space were passed
//! around as `Option<&mut T>` with a "make a fresh default if None" fallback.
//! That pattern doesn't work: the fallback default is a temporary that can't
//! outlive the match arm that creates it, and reborrowing an already-`&mut`
//! local as `&mut local` (instead of just passing `local`) produces a
//! double reference. Both mistakes showed up throughout the old code.
//!
//! The fix used here is to never make those parameters optional: `set_layout`
//! always receives a live `&mut MT`, `&mut ElementConfiguration` and
//! `&mut TextConfig`, and callers that don't have an existing one to hand
//! down (list items, the top level call) simply own a fresh local and pass a
//! reborrow of it.
//!
//! `set_layout` also holds a live `&mut UserApp` (bounded `LayoutReflector`)
//! and a `&mut API` as separate parameters, so when an event fires it calls
//! [`LayoutReflector::dispatch_event`] right then, in tree order - the same
//! way `fn *name*` elements already go straight to `dispatch_custom_element`.
//! A handler therefore sees state changes made by handlers earlier in the
//! same frame's tree. The one exception is `treeview`: the tree it walks is
//! borrowed out of the app (`get_treeview` returns `TreeViewItem<'_>` with
//! `&str` labels), so it can't hold `&mut UserApp` at the same time - it
//! collects its handful of events into an owned `Vec` and `set_layout`
//! drains that immediately, once the borrow on the tree is released.
use std::{collections::HashMap, fmt::Debug, str::FromStr};

use markdown::mdast::{List, Node, Paragraph};
use strum_macros::Display;
use symbol_table::GlobalSymbol;
use telera_layout::{Color, ElementConfiguration, TextConfig};

use crate::{
    API, CustomElement, LineConfig, MT, UIImageDescriptor, ui_toolkit::treeview::treeview,
};

const DEFAULT_TEXT: &str = ":(";

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Extra, free-form data carried alongside a dispatched event.
///
/// `code`/`code2` are a couple of general-purpose numeric slots; the layout
/// runner fills `code` with the current list index when an event fires from
/// inside a `list`, so a handler can tell which item was interacted with.
/// `treeview` chains `code`/`code2` forward from the tree's own event
/// definitions and additionally fills in `text` with the item's label.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EventContext {
    pub text: Option<String>,
    pub code: Option<u32>,
    pub code2: Option<u32>,
    pub list_index: Option<usize>,
}

fn build_event_context(list_data: &Option<(GlobalSymbol, usize)>) -> Option<EventContext> {
    list_data.as_ref().map(|(_, index)| EventContext {
        text: None,
        code: None,
        code2: None,
        list_index: Some(*index),
    })
}

/// Implemented (by hand) on an application struct so the layout runner can
/// call back into it by *name*: `dispatch_event` for an event a config like
/// `left-clicked some_handler` fired, `dispatch_custom_element` for a `fn
/// *name*` element that stands in for a hand-built subtree. Both names come
/// straight from the markdown as interned [`GlobalSymbol`]s - there is no
/// event enum anymore. Every method is defaulted to a no-op, so an app whose
/// layouts use neither still just writes `impl LayoutReflector for Self {}`.
#[allow(unused_variables)]
pub trait LayoutReflector {
    fn dispatch_event(
        &mut self,
        name: &GlobalSymbol,
        context: Option<EventContext>,
        api: &mut API,
    ) {
    }

    fn dispatch_custom_element(&mut self, name: &GlobalSymbol, api: &mut API, mt: &mut MT) {}
}

// ---------------------------------------------------------------------------
// Layout command types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Display, PartialEq)]
pub enum Layout {
    Element(Element),
    Declaration {
        name: GlobalSymbol,
        value: DataSrc<Declaration>,
    },
    Config(Config),
}

#[derive(Clone, Debug, Display, PartialEq)]
pub enum Element {
    ElementOpened {
        id: Option<DataSrc<String>>,
    },
    ElementClosed,

    TextElementOpened,
    TextElementClosed(DataSrc<String>),

    ConfigOpened,
    ConfigClosed,

    TextConfigOpened,
    TextConfigClosed,

    ListOpened,
    ListClosed(GlobalSymbol),

    // `item list-name index` - like `list`, but resolves a single (static or
    // dynamic) index into `list-name` once instead of iterating over it, so
    // the body can reference that one item's fields directly - e.g.
    // `documents[selected_document]` without any per-app Rust wiring.
    ItemOpened,
    ItemClosed {
        list: GlobalSymbol,
        index: DataSrc<f32>,
    },

    UseOpened,
    UseClosed(GlobalSymbol),

    TreeViewOpened,
    TreeViewClosed(GlobalSymbol),

    TextBoxOpened,
    TextBoxClosed(DataSrc<String>),

    // `fn *name*` - calls back into the app's own
    // `LayoutReflector::dispatch_custom_element` with `name` right where it
    // sits in the tree, so the dispatched function can add whatever it wants
    // via `api.l` at that spot. A single leaf command, unlike most other
    // elements here, since there's no body of its own to open/close.
    FunctionCall(GlobalSymbol),

    CircleOpened {
        id: Option<DataSrc<String>>,
    },
    CircleClosed,

    LineOpened {
        id: Option<DataSrc<String>>,
    },
    LineClosed,

    // if / if-not
    IfOpened {
        condition: GlobalSymbol,
    },
    IfNotOpened {
        condition: GlobalSymbol,
    },
    IfClosed,

    // if-index / if-index-not - true when inside a `list` and the current
    // iteration index equals `index` (a static or dynamic numeric value);
    // always false (if-index-not: always true) outside any list. Both close
    // via the plain `IfClosed` above - the nesting/skip bookkeeping is
    // identical, only the condition itself differs.
    IfIndexOpened {
        index: DataSrc<f32>,
    },
    IfIndexNotOpened {
        index: DataSrc<f32>,
    },

    Pointer(winit::window::CursorIcon),

    HoverOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    HoverClosed,

    HoveredOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    HoveredClosed,

    UnHoveredOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    UnHoveredClosed,

    FocusOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    FocusClosed,

    FocusedOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    FocusedClosed,

    UnFocusedOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    UnFocusedClosed,

    LeftPressedOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    LeftPressedClosed,

    LeftDownOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    LeftDownClosed,

    LeftReleasedOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    LeftReleasedClosed,

    LeftClickedOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    LeftClickedClosed,

    LeftDoubleClickedOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    LeftDoubleClickedClosed,

    LeftTripleClickedOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    LeftTripleClickedClosed,

    RightPressedOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    RightPressedClosed,

    RightDownOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    RightDownClosed,

    RightReleasedOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    RightReleasedClosed,

    RightClickedOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    RightClickedClosed,
}

#[derive(Clone, Debug, Display, PartialEq)]
pub enum Config {
    Id(DataSrc<String>),

    GrowAll,
    GrowX,
    GrowXmin(DataSrc<f32>),
    GrowXmax(DataSrc<f32>),
    GrowXminmax {
        min: DataSrc<f32>,
        max: DataSrc<f32>,
    },
    GrowY,
    GrowYmin(DataSrc<f32>),
    GrowYmax(DataSrc<f32>),
    GrowYminmax {
        min: DataSrc<f32>,
        max: DataSrc<f32>,
    },
    FitX,
    FitXmin(DataSrc<f32>),
    FitXmax(DataSrc<f32>),
    FitXminmax {
        min: DataSrc<f32>,
        max: DataSrc<f32>,
    },
    FitY,
    FitYmin(DataSrc<f32>),
    FitYmax(DataSrc<f32>),
    FitYminmax {
        min: DataSrc<f32>,
        max: DataSrc<f32>,
    },
    FixedX(DataSrc<f32>),
    FixedY(DataSrc<f32>),
    PercentX(DataSrc<f32>),
    PercentY(DataSrc<f32>),

    PaddingAll(DataSrc<u16>),
    PaddingTop(DataSrc<u16>),
    PaddingBottom(DataSrc<u16>),
    PaddingLeft(DataSrc<u16>),
    PaddingRight(DataSrc<u16>),

    ChildGap(DataSrc<u16>),

    Vertical,

    ChildAlignmentXLeft,
    ChildAlignmentXRight,
    ChildAlignmentXCenter,
    ChildAlignmentYTop,
    ChildAlignmentYCenter,
    ChildAlignmentYBottom,

    Color(DataSrc<Color>),

    RadiusAll(DataSrc<f32>),
    RadiusTopLeft(DataSrc<f32>),
    RadiusTopRight(DataSrc<f32>),
    RadiusBottomRight(DataSrc<f32>),
    RadiusBottomLeft(DataSrc<f32>),

    BorderColor(DataSrc<Color>),
    BorderAll(DataSrc<u16>),
    BorderTop(DataSrc<u16>),
    BorderLeft(DataSrc<u16>),
    BorderBottom(DataSrc<u16>),
    BorderRight(DataSrc<u16>),
    BorderBetweenChildren(DataSrc<u16>),

    Clip {
        vertical: DataSrc<bool>,
        horizontal: DataSrc<bool>,
    },

    Image {
        name: GlobalSymbol,
    },

    Floating,
    FloatingOffset {
        x: DataSrc<f32>,
        y: DataSrc<f32>,
    },
    FloatingDimensions {
        width: DataSrc<f32>,
        height: DataSrc<f32>,
    },
    FloatingZIndex {
        z: DataSrc<i16>,
    },
    FloatingAttatchToParentAtTopLeft,
    FloatingAttatchToParentAtCenterLeft,
    FloatingAttatchToParentAtBottomLeft,
    FloatingAttatchToParentAtTopCenter,
    FloatingAttatchToParentAtCenter,
    FloatingAttatchToParentAtBottomCenter,
    FloatingAttatchToParentAtTopRight,
    FloatingAttatchToParentAtCenterRight,
    FloatingAttatchToParentAtBottomRight,
    FloatingAttatchElementAtTopLeft,
    FloatingAttatchElementAtCenterLeft,
    FloatingAttatchElementAtBottomLeft,
    FloatingAttatchElementAtTopCenter,
    FloatingAttatchElementAtCenter,
    FloatingAttatchElementAtBottomCenter,
    FloatingAttatchElementAtTopRight,
    FloatingAttatchElementAtCenterRight,
    FloatingAttatchElementAtBottomRight,
    FloatingPointerPassThrough,
    FloatingAttachElementToElement {
        other_element_id: String,
    },
    FloatingAttachElementToRoot,

    CustomElement(CustomElement),

    Use {
        name: GlobalSymbol,
    },

    FontId(DataSrc<u16>),
    AlignRight,
    AlignLeft,
    AlignCenter,
    LineHeight(DataSrc<u16>),
    FontSize(DataSrc<u16>),
    FontColor(DataSrc<Color>),
    Editable(bool),
}

#[derive(Clone, Debug, Display, PartialEq)]
pub enum Declaration {
    Bool(bool),
    Numeric(f32),
    Text(String),
    Color(Color),
    Event(GlobalSymbol),
    Image(GlobalSymbol),
}

impl Default for Declaration {
    fn default() -> Self {
        Declaration::Bool(false)
    }
}

#[derive(Clone, Debug, Display, PartialEq)]
pub enum DataSrc<T> {
    Static(T),
    Dynamic(GlobalSymbol),
}

impl<T: Default> Default for DataSrc<T> {
    fn default() -> Self {
        DataSrc::Static(T::default())
    }
}

/// Implemented by an application struct (normally via
/// `#[derive(ParserDataAccess)]`, see `telera_macros`) to answer the
/// `get-*`/`set-*` declarations a layout can reference dynamically.
#[allow(unused_variables)]
pub trait LayoutRunnerReflection {
    fn get_list_length(
        &self,
        name: &GlobalSymbol,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Option<usize> {
        None
    }
    fn get_bool(
        &self,
        name: &GlobalSymbol,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Option<bool> {
        None
    }
    fn get_numeric(
        &self,
        name: &GlobalSymbol,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Option<f32> {
        None
    }
    fn get_text<'render_pass, 'application>(
        &'application self,
        name: &GlobalSymbol,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Option<&'render_pass String>
    where
        'application: 'render_pass,
    {
        None
    }
    fn get_image<'render_pass, 'application>(
        &'application self,
        name: &GlobalSymbol,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Option<&'render_pass UIImageDescriptor>
    where
        'application: 'render_pass,
    {
        None
    }
    fn get_color<'render_pass, 'application>(
        &'application self,
        name: &GlobalSymbol,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Option<&'render_pass Color>
    where
        'application: 'render_pass,
    {
        None
    }
    fn get_event<'render_pass, 'application>(
        &'application self,
        name: &GlobalSymbol,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Option<GlobalSymbol>
    where
        'application: 'render_pass,
    {
        None
    }
    fn get_treeview<'render_pass, 'application>(
        &'application self,
        name: &GlobalSymbol,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Option<crate::TreeViewItem<'render_pass>>
    where
        'application: 'render_pass,
    {
        None
    }
}

/// Implemented by an item type that shows up as the element of a `Vec<T>`
/// field on a [`ParserDataAccess`] struct (normally via
/// `#[derive(FieldAccess)]`, see `telera_macros`), so a `list` in a layout
/// can look up that item's own fields by name - e.g. `Vec<Document>` where
/// `Document` derives `FieldAccess` lets `list Documents` resolve `*title*`
/// to each document's own `title` field.
#[allow(unused_variables)]
pub trait FieldAccess {
    fn field_bool(&self, name: &GlobalSymbol) -> Option<bool> {
        None
    }
    fn field_numeric(&self, name: &GlobalSymbol) -> Option<f32> {
        None
    }
    fn field_text(&self, name: &GlobalSymbol) -> Option<&String> {
        None
    }
    fn field_color(&self, name: &GlobalSymbol) -> Option<&Color> {
        None
    }
}

/// Normalizes a markdown-provided lookup name the same way
/// `#[derive(ParserDataAccess)]`/`#[derive(FieldAccess)]` normalize a Rust
/// field's own name before comparing the two, so e.g. `*content background
/// color*` in a layout matches a `content_background_color` field, and
/// `file-menu-opened` matches `file_menu_open`: lowercase, with spaces and
/// hyphens folded to underscores. Exposed publicly because the derived code
/// calls it; not normally something you need to call yourself.
pub fn normalize_field_symbol(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            ' ' | '-' => '_',
            c => c.to_ascii_lowercase(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Markdown parser: turns a document into a flat Vec<Layout>
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum ParsingMode {
    None,
    Body,
    ReusableElements,
    ReusableConfig,
}

/// A parsed page: its flattened layout commands, and its table of reusable
/// snippets (declared with `##`/`###` headings, referenced with `use`).
///
/// The page has no name of its own - the `# ...` heading only marks where the
/// body starts (its text is ignored, `# root` by convention). The caller names
/// the page when it registers it (see [`Binder::load_layout`]); `API` uses the
/// layout file's own name, minus the `.md`.
pub type ParsedLayout = (Vec<Layout>, HashMap<String, Vec<Layout>>);

/// Parses a markdown layout document (see `src/layouts/Main.md`) into a
/// [`ParsedLayout`].
pub fn process_layout(file: String) -> Result<ParsedLayout, String> {
    let mut parsing_mode = ParsingMode::None;
    let mut body = Vec::<Layout>::new();
    let mut open_reuseable_name = "".to_string();
    let mut reusables = HashMap::<String, Vec<Layout>>::new();

    if let Ok(m) = markdown::to_mdast(&file, &markdown::ParseOptions::default())
        && let Some(nodes) = m.children()
    {
        for node in nodes {
            match node {
                Node::Heading(h) => {
                    if let Some(declaration) = h.children.first()
                        && let Node::Text(declaration) = declaration
                    {
                        match h.depth {
                            1 => {
                                // `# ...` just marks the start of the page body -
                                // the heading text (`# root` by convention) is
                                // ignored; the page is named by its caller.
                                parsing_mode = ParsingMode::Body;
                            }
                            2 => {
                                parsing_mode = ParsingMode::ReusableConfig;
                                open_reuseable_name = declaration.value.trim().to_string();
                            }
                            3 => {
                                parsing_mode = ParsingMode::ReusableElements;
                                open_reuseable_name = declaration.value.trim().to_string();
                            }
                            _ => parsing_mode = ParsingMode::None,
                        }
                    }
                }
                Node::List(list) => match parsing_mode {
                    ParsingMode::ReusableConfig => {
                        let mut reusable_items = process_configs(list, &mut None);
                        let mut formatted_reusable_items = Vec::<Layout>::new();
                        formatted_reusable_items.append(&mut reusable_items);
                        reusables.insert(open_reuseable_name.clone(), formatted_reusable_items);
                    }
                    ParsingMode::ReusableElements => {
                        for node in &list.children {
                            let element = process_element(node);
                            reusables.insert(open_reuseable_name.clone(), element);
                        }
                    }
                    ParsingMode::Body => {
                        body.push(Layout::Element(Element::Pointer(
                            winit::window::CursorIcon::Default,
                        )));
                        for node in &list.children {
                            let mut element = process_element(node);
                            body.append(&mut element);
                        }
                    }
                    ParsingMode::None => {}
                },
                _ => {}
            }
        }
        Ok((body, reusables))
    } else {
        Err("failed to parse layout markdown".to_string())
    }
}

/// A single argument like `item`'s index or `if-index`'s comparand: a bare
/// number is a static value, anything else is a dynamic symbol to resolve
/// (typically a plain numeric field, e.g. `selected_document`).
fn parse_index_arg(arg: &str) -> DataSrc<f32> {
    match arg.parse::<f32>() {
        Ok(value) => DataSrc::Static(value),
        Err(_) => DataSrc::Dynamic(GlobalSymbol::new(arg)),
    }
}

/// True if `node` is a `- \`declarations\`` list item, the way a `list`'s
/// (optional) leading declarations block looks.
fn is_declarations_block(node: &Node) -> bool {
    if let Node::ListItem(item) = node
        && let Some(Node::Paragraph(paragraph)) = item.children.first()
        && let Some(Node::InlineCode(marker)) = paragraph.children.first()
    {
        marker.value == "declarations"
    } else {
        false
    }
}

fn process_element(element: &Node) -> Vec<Layout> {
    let mut layout_commands: Vec<Layout> = Vec::new();

    if let Node::ListItem(element) = element
        && let Some(element_declaration) = element.children.first()
        && let Node::Paragraph(element_declaration) = element_declaration
        && let Some(element_type) = element_declaration.children.first()
        && let Node::InlineCode(element_type) = element_type
    {
        match element_type.value.as_str() {
            "declarations" => {
                if let Some(declarations) = element.children.get(1)
                    && let Node::List(declarations) = declarations
                {
                    for declaration in declarations.children.iter() {
                        if let Some((name, value)) = process_variable(declaration) {
                            let name = GlobalSymbol::new(name);
                            layout_commands.push(Layout::Declaration { name, value });
                        }
                    }
                }
            }
            "element" => {
                layout_commands.push(Layout::Element(Element::ElementOpened { id: None }));
                layout_commands.push(Layout::Element(Element::ConfigOpened));
                if let Some(element_name) = element_declaration.children.get(1)
                    && let Node::Text(element_name) = element_name
                {
                    layout_commands.push(Layout::Config(Config::Id(DataSrc::Static(
                        element_name.value.trim().to_string(),
                    ))));
                }
                if let Some(config) = element.children.get(1)
                    && let Node::List(configs) = config
                    && let Some(configs) = configs.children.first()
                    && let Node::ListItem(configs) = configs
                    && let Some(configs) = configs.children.get(1)
                    && let Node::List(config_commands) = configs
                {
                    let mut layout_config_commands = process_configs(config_commands, &mut None);
                    layout_commands.append(&mut layout_config_commands);
                }
                layout_commands.push(Layout::Element(Element::ConfigClosed));

                if let Some(child_elements) = element.children.get(1)
                    && let Node::List(child_elements) = child_elements
                {
                    for child_element in child_elements.children.iter().skip(1) {
                        let mut child_element = process_element(child_element);
                        layout_commands.append(&mut child_element);
                    }
                }

                layout_commands.push(Layout::Element(Element::ElementClosed));
            }
            "circle" => {
                layout_commands.push(Layout::Element(Element::CircleOpened { id: None }));
                layout_commands.push(Layout::Element(Element::ConfigOpened));
                if let Some(element_name) = element_declaration.children.get(1)
                    && let Node::Text(element_name) = element_name
                {
                    layout_commands.push(Layout::Config(Config::Id(DataSrc::Static(
                        element_name.value.trim().to_string(),
                    ))));
                }
                if let Some(config) = element.children.get(1)
                    && let Node::List(configs) = config
                    && let Some(configs) = configs.children.first()
                    && let Node::ListItem(configs) = configs
                    && let Some(configs) = configs.children.get(1)
                    && let Node::List(config_commands) = configs
                {
                    let mut custom_element = CustomElement::Circle;
                    let mut layout_config_commands =
                        process_configs(config_commands, &mut Some(&mut custom_element));
                    layout_commands.append(&mut layout_config_commands);
                    layout_commands.push(Layout::Config(Config::CustomElement(custom_element)));
                }
                layout_commands.push(Layout::Element(Element::ConfigClosed));
                layout_commands.push(Layout::Element(Element::CircleClosed));
            }
            "line" => {
                layout_commands.push(Layout::Element(Element::LineOpened { id: None }));
                layout_commands.push(Layout::Element(Element::ConfigOpened));
                if let Some(element_name) = element_declaration.children.get(1)
                    && let Node::Text(element_name) = element_name
                {
                    layout_commands.push(Layout::Config(Config::Id(DataSrc::Static(
                        element_name.value.trim().to_string(),
                    ))));
                }
                if let Some(config) = element.children.get(1)
                    && let Node::List(configs) = config
                    && let Some(configs) = configs.children.first()
                    && let Node::ListItem(configs) = configs
                    && let Some(configs) = configs.children.get(1)
                    && let Node::List(config_commands) = configs
                {
                    let line_config = LineConfig::default();
                    let mut custom_element = CustomElement::Line(line_config);
                    let mut layout_config_commands =
                        process_configs(config_commands, &mut Some(&mut custom_element));
                    layout_commands.append(&mut layout_config_commands);
                    layout_commands.push(Layout::Config(Config::CustomElement(custom_element)));
                }
                layout_commands.push(Layout::Element(Element::ConfigClosed));
                layout_commands.push(Layout::Element(Element::LineClosed));
            }
            "grow" => {
                layout_commands.push(Layout::Element(Element::ElementOpened { id: None }));
                layout_commands.push(Layout::Element(Element::ConfigOpened));
                layout_commands.push(Layout::Config(Config::GrowAll));
                layout_commands.push(Layout::Element(Element::ConfigClosed));
                layout_commands.push(Layout::Element(Element::ElementClosed));
            }
            "text" => {
                layout_commands.push(Layout::Element(Element::TextElementOpened));

                layout_commands.push(Layout::Element(Element::TextConfigOpened));
                if let Some(config) = element.children.get(1)
                    && let Node::List(config) = config
                    && let Some(config) = config.children.first()
                    && let Node::ListItem(config) = config
                    && let Some(configs) = config.children.get(1)
                    && let Node::List(configs) = configs
                {
                    let mut configs = process_configs(configs, &mut None);
                    layout_commands.append(&mut configs);
                }
                layout_commands.push(Layout::Element(Element::TextConfigClosed));

                if let Some(text) = element.children.get(1)
                    && let Node::List(text) = text
                    && let Some(text) = text.children.get(1)
                    && let Node::ListItem(text) = text
                    && let Some(text) = text.children.first()
                    && let Node::Paragraph(text) = text
                    && let Some(text) = text.children.first()
                {
                    match text {
                        Node::Emphasis(dynamic_text) => {
                            if let Some(dynamic_text) = dynamic_text.children.first()
                                && let Node::Text(dynamic_text) = dynamic_text
                            {
                                let src = GlobalSymbol::new(dynamic_text.value.trim());
                                layout_commands.push(Layout::Element(Element::TextElementClosed(
                                    DataSrc::Dynamic(src),
                                )));
                            }
                        }
                        Node::Text(static_text) => {
                            layout_commands.push(Layout::Element(Element::TextElementClosed(
                                DataSrc::Static(static_text.value.trim().to_string()),
                            )));
                        }
                        _ => {}
                    }
                }
            }
            "use" => {
                if let Some(reusable_name) = element_declaration.children.get(1)
                    && let Node::Text(reusable_name) = reusable_name
                    && let Some(input_variables) = element.children.get(1)
                    && let Node::List(input_variables) = input_variables
                {
                    let src = GlobalSymbol::new(reusable_name.value.trim());
                    layout_commands.push(Layout::Element(Element::UseOpened));
                    for input_variable in &input_variables.children {
                        if let Some((name, declaration)) = process_variable(input_variable) {
                            let name = GlobalSymbol::new(name);
                            layout_commands.push(Layout::Declaration {
                                name,
                                value: declaration,
                            });
                        }
                    }
                    layout_commands.push(Layout::Element(Element::UseClosed(src)));
                }
            }
            "list" => {
                if let Some(list_src) = element_declaration.children.get(1)
                    && let Node::Text(list_src) = list_src
                    && let Some(list_content) = element.children.get(1)
                    && let Node::List(list_content) = list_content
                {
                    let mut formatted_list = Vec::<Layout>::new();
                    formatted_list.push(Layout::Element(Element::ListOpened));

                    // A leading `declarations` block is optional - only skip
                    // it as the first body item if it's actually there, or
                    // an `item`-less list's first real element would be
                    // silently dropped.
                    let has_declarations = list_content
                        .children
                        .first()
                        .is_some_and(is_declarations_block);

                    if has_declarations
                        && let Some(declarations) = list_content.children.first()
                        && let Node::ListItem(declarations) = declarations
                        && let Some(declarations) = declarations.children.get(1)
                        && let Node::List(declarations) = declarations
                    {
                        for declaration in &declarations.children {
                            if let Some((name, declaration)) = process_variable(declaration) {
                                let src = GlobalSymbol::new(name);
                                formatted_list.push(Layout::Declaration {
                                    name: src,
                                    value: declaration,
                                });
                            }
                        }
                    }

                    let skip_count = if has_declarations { 1 } else { 0 };
                    for li in list_content.children.iter().skip(skip_count) {
                        let mut list_item = process_element(li);
                        formatted_list.append(&mut list_item);
                    }

                    let src = GlobalSymbol::new(list_src.value.trim());
                    formatted_list.push(Layout::Element(Element::ListClosed(src)));

                    layout_commands.append(&mut formatted_list);
                }
            }
            "item" => {
                if let Some(args) = element_declaration.children.get(1)
                    && let Node::Text(args) = args
                {
                    let mut args = args.value.split_whitespace();
                    if let Some(list_name) = args.next()
                        && let Some(index_arg) = args.next()
                    {
                        let list_symbol = GlobalSymbol::new(list_name);
                        let index = parse_index_arg(index_arg);

                        layout_commands.push(Layout::Element(Element::ItemOpened));

                        if let Some(body) = element.children.get(1)
                            && let Node::List(body) = body
                        {
                            for item in &body.children {
                                let mut item = process_element(item);
                                layout_commands.append(&mut item);
                            }
                        }

                        layout_commands.push(Layout::Element(Element::ItemClosed {
                            list: list_symbol,
                            index,
                        }));
                    }
                }
            }
            "if" => {
                if let Some(conditional) = element_declaration.children.get(1)
                    && let Node::Text(conditional) = conditional
                    && let Some(conditional_elements) = element.children.get(1)
                    && let Node::List(conditional_elements) = conditional_elements
                {
                    let mut formatted_element = Vec::<Layout>::new();
                    let src = GlobalSymbol::new(conditional.value.trim());
                    formatted_element.push(Layout::Element(Element::IfOpened { condition: src }));

                    for conditional_element in &conditional_elements.children {
                        let mut conditional_element = process_element(conditional_element);
                        formatted_element.append(&mut conditional_element);
                    }

                    formatted_element.push(Layout::Element(Element::IfClosed));

                    layout_commands.append(&mut formatted_element);
                }
            }
            "if-not" => {
                if let Some(conditional) = element_declaration.children.get(1)
                    && let Node::Text(conditional) = conditional
                    && let Some(conditional_elements) = element.children.get(1)
                    && let Node::List(conditional_elements) = conditional_elements
                {
                    let mut formatted_element = Vec::<Layout>::new();
                    let src = GlobalSymbol::new(conditional.value.trim());
                    formatted_element
                        .push(Layout::Element(Element::IfNotOpened { condition: src }));

                    for conditional_element in &conditional_elements.children {
                        let mut conditional_element = process_element(conditional_element);
                        formatted_element.append(&mut conditional_element);
                    }

                    formatted_element.push(Layout::Element(Element::IfClosed));

                    layout_commands.append(&mut formatted_element);
                }
            }
            "if-index" => {
                if let Some(index_arg) = element_declaration.children.get(1)
                    && let Node::Text(index_arg) = index_arg
                    && let Some(conditional_elements) = element.children.get(1)
                    && let Node::List(conditional_elements) = conditional_elements
                {
                    let mut formatted_element = Vec::<Layout>::new();
                    let index = parse_index_arg(index_arg.value.trim());
                    formatted_element.push(Layout::Element(Element::IfIndexOpened { index }));

                    for conditional_element in &conditional_elements.children {
                        let mut conditional_element = process_element(conditional_element);
                        formatted_element.append(&mut conditional_element);
                    }

                    formatted_element.push(Layout::Element(Element::IfClosed));

                    layout_commands.append(&mut formatted_element);
                }
            }
            "if-index-not" => {
                if let Some(index_arg) = element_declaration.children.get(1)
                    && let Node::Text(index_arg) = index_arg
                    && let Some(conditional_elements) = element.children.get(1)
                    && let Node::List(conditional_elements) = conditional_elements
                {
                    let mut formatted_element = Vec::<Layout>::new();
                    let index = parse_index_arg(index_arg.value.trim());
                    formatted_element.push(Layout::Element(Element::IfIndexNotOpened { index }));

                    for conditional_element in &conditional_elements.children {
                        let mut conditional_element = process_element(conditional_element);
                        formatted_element.append(&mut conditional_element);
                    }

                    formatted_element.push(Layout::Element(Element::IfClosed));

                    layout_commands.append(&mut formatted_element);
                }
            }
            "fn" => {
                // `- \`fn\` *custom_element*` parses as a paragraph of
                // [InlineCode("fn"), Text(" "), Emphasis([Text(name)])] -
                // the name is always dynamic (an identifier the app looks
                // up by name at dispatch time), never a static/quoted
                // string, so unlike most other elements there's no
                // `Node::Text` fallback branch to match here.
                if let Some(function_name) = element_declaration.children.get(2)
                    && let Node::Emphasis(function_name) = function_name
                    && let Some(function_name) = function_name.children.first()
                    && let Node::Text(function_name) = function_name
                {
                    let src = GlobalSymbol::new(function_name.value.trim());
                    layout_commands.push(Layout::Element(Element::FunctionCall(src)));
                }
            }
            "treeview" => {
                if let Some(reusable_name) = element_declaration.children.get(1)
                    && let Node::Text(reusable_name) = reusable_name
                {
                    layout_commands.push(Layout::Element(Element::TreeViewOpened));
                    let src = GlobalSymbol::new(reusable_name.value.trim());
                    layout_commands.push(Layout::Element(Element::TreeViewClosed(src)));
                }
            }
            "textbox" => match parameter_check::<String>(element_declaration, "", "") {
                AvailableParameters::SingleDynamic(a) => {
                    layout_commands.push(Layout::Element(Element::TextBoxOpened));
                    layout_commands
                        .push(Layout::Element(Element::TextBoxClosed(DataSrc::Dynamic(a))));
                }
                AvailableParameters::SingleStatic(a) => {
                    layout_commands.push(Layout::Element(Element::TextBoxOpened));
                    layout_commands
                        .push(Layout::Element(Element::TextBoxClosed(DataSrc::Static(a))));
                }
                _ => {}
            },
            _ => {}
        }
    }

    layout_commands
}

#[derive(Debug)]
enum AvailableParameters<T> {
    None,
    AStatic(T),
    ADynamic(GlobalSymbol),
    BStatic(T),
    BDynamic(GlobalSymbol),
    TwoStatic(T, T),
    TwoDynamic(GlobalSymbol, GlobalSymbol),
    AStaticBDynamic(T, GlobalSymbol),
    ADynamicBStatic(GlobalSymbol, T),
    SingleStatic(T),
    SingleDynamic(GlobalSymbol),
}

fn parameter_check<T: FromStr>(
    parameters: &Paragraph,
    bound_a: &str,
    bound_b: &str,
) -> AvailableParameters<T> {
    if parameters.children.len() < 2 {
        return AvailableParameters::None;
    }
    //  CASE: 2 static parameters
    if let Some(bound_range_a) = parameters.children.get(2)
        && let Node::InlineCode(bound_range_a) = bound_range_a
        && (bound_range_a.value.as_str() == bound_a || bound_range_a.value.as_str() == bound_b)
        && let Some(bound_value_a) = parameters.children.get(3)
        && let Node::Text(bound_value_a) = bound_value_a
        && let Ok(bound_value_a) = T::from_str(bound_value_a.value.trim())
        && let Some(bound_range_b) = parameters.children.get(4)
        && let Node::InlineCode(bound_range_b) = bound_range_b
        && (bound_range_b.value.as_str() == bound_a || bound_range_b.value.as_str() == bound_b)
        && let Some(bound_value_b) = parameters.children.get(5)
        && let Node::Text(bound_value_b) = bound_value_b
        && let Ok(bound_value_b) = T::from_str(bound_value_b.value.trim())
    {
        if bound_range_a.value.as_str() == bound_a {
            AvailableParameters::TwoStatic(bound_value_a, bound_value_b)
        } else {
            AvailableParameters::TwoStatic(bound_value_b, bound_value_a)
        }
    }
    //  CASE: 2 dynamic parameters
    else if let Some(bound_range_a) = parameters.children.get(2)
        && let Node::InlineCode(bound_range_a) = bound_range_a
        && (bound_range_a.value.as_str() == bound_a || bound_range_a.value.as_str() == bound_b)
        && let Some(bound_value_a) = parameters.children.get(4)
        && let Node::Emphasis(bound_value_a) = bound_value_a
        && let Some(bound_value_a) = bound_value_a.children.first()
        && let Node::Text(bound_value_a) = bound_value_a
        && let Some(bound_range_b) = parameters.children.get(6)
        && let Node::InlineCode(bound_range_b) = bound_range_b
        && (bound_range_b.value.as_str() == bound_a || bound_range_b.value.as_str() == bound_b)
        && let Some(bound_value_b) = parameters.children.get(8)
        && let Node::Emphasis(bound_value_b) = bound_value_b
        && let Some(bound_value_b) = bound_value_b.children.first()
        && let Node::Text(bound_value_b) = bound_value_b
    {
        let bound_value_a = GlobalSymbol::new(bound_value_a.value.trim());
        let bound_value_b = GlobalSymbol::new(bound_value_b.value.trim());
        if bound_range_a.value.as_str() == bound_a {
            AvailableParameters::TwoDynamic(bound_value_a, bound_value_b)
        } else {
            AvailableParameters::TwoDynamic(bound_value_b, bound_value_a)
        }
    }
    //  CASE: parameter A dynamic, b static
    else if let Some(bound_range_a) = parameters.children.get(2)
        && let Node::InlineCode(bound_range_a) = bound_range_a
        && (bound_range_a.value.as_str() == bound_a || bound_range_a.value.as_str() == bound_b)
        && let Some(bound_value_a) = parameters.children.get(4)
        && let Node::Emphasis(bound_value_a) = bound_value_a
        && let Some(bound_value_a) = bound_value_a.children.first()
        && let Node::Text(bound_value_a) = bound_value_a
        && let Some(bound_range_b) = parameters.children.get(6)
        && let Node::InlineCode(bound_range_b) = bound_range_b
        && (bound_range_b.value.as_str() == bound_a || bound_range_b.value.as_str() == bound_b)
        && let Some(bound_value_b) = parameters.children.get(7)
        && let Node::Text(bound_value_b) = bound_value_b
        && let Ok(bound_value_b) = T::from_str(bound_value_b.value.trim())
    {
        let bound_value_a = GlobalSymbol::new(bound_value_a.value.trim());
        if bound_range_a.value.as_str() == bound_a {
            AvailableParameters::ADynamicBStatic(bound_value_a, bound_value_b)
        } else {
            AvailableParameters::AStaticBDynamic(bound_value_b, bound_value_a)
        }
    }
    //  CASE: parameter A static, b dynamic
    else if let Some(bound_range_a) = parameters.children.get(2)
        && let Node::InlineCode(bound_range_a) = bound_range_a
        && (bound_range_a.value.as_str() == bound_a || bound_range_a.value.as_str() == bound_b)
        && let Some(bound_value_a) = parameters.children.get(3)
        && let Node::Text(bound_value_a) = bound_value_a
        && let Ok(bound_value_a) = T::from_str(bound_value_a.value.trim())
        && let Some(bound_range_b) = parameters.children.get(4)
        && let Node::InlineCode(bound_range_b) = bound_range_b
        && (bound_range_b.value.as_str() == bound_a || bound_range_b.value.as_str() == bound_b)
        && let Some(bound_value_b) = parameters.children.get(6)
        && let Node::Emphasis(bound_value_b) = bound_value_b
        && let Some(bound_value_b) = bound_value_b.children.first()
        && let Node::Text(bound_value_b) = bound_value_b
    {
        let bound_value_b = GlobalSymbol::new(bound_value_b.value.trim());
        if bound_range_a.value.as_str() == bound_a {
            AvailableParameters::ADynamicBStatic(bound_value_b, bound_value_a)
        } else {
            AvailableParameters::AStaticBDynamic(bound_value_a, bound_value_b)
        }
    }
    //  CASE: 1 static parameter
    else if let Some(bound_range_a) = parameters.children.get(2)
        && let Node::InlineCode(bound_range_a) = bound_range_a
        && (bound_range_a.value.as_str() == bound_a || bound_range_a.value.as_str() == bound_b)
        && let Some(bound_value_a) = parameters.children.get(3)
        && let Node::Text(bound_value_a) = bound_value_a
        && let Ok(bound_value_a) = T::from_str(bound_value_a.value.trim())
    {
        if bound_range_a.value.as_str() == bound_a {
            AvailableParameters::AStatic(bound_value_a)
        } else {
            AvailableParameters::BStatic(bound_value_a)
        }
    }
    //  CASE: 1 dynamic parameter
    else if let Some(bound_range_a) = parameters.children.get(2)
        && let Node::InlineCode(bound_range_a) = bound_range_a
        && (bound_range_a.value.as_str() == bound_a || bound_range_a.value.as_str() == bound_b)
        && let Some(bound_value_a) = parameters.children.get(4)
        && let Node::Emphasis(bound_value_a) = bound_value_a
        && let Some(bound_value_a) = bound_value_a.children.first()
        && let Node::Text(bound_value_a) = bound_value_a
    {
        let bound_value_a = GlobalSymbol::new(bound_value_a.value.trim());
        if bound_range_a.value.as_str() == bound_a {
            AvailableParameters::ADynamic(bound_value_a)
        } else {
            AvailableParameters::BDynamic(bound_value_a)
        }
    } else if let Some(parameter) = parameters.children.get(2)
        && let Node::Emphasis(parameter) = parameter
        && let Some(parameter) = parameter.children.first()
        && let Node::Text(parameter) = parameter
    {
        let parameter = GlobalSymbol::new(parameter.value.trim());
        AvailableParameters::SingleDynamic(parameter)
    } else if let Some(parameter) = parameters.children.get(1)
        && let Node::Text(parameter) = parameter
        && let Ok(parameter) = T::from_str(parameter.value.trim())
    {
        AvailableParameters::SingleStatic(parameter)
    }
    //  CASE: no parameters
    else {
        AvailableParameters::None
    }
}

fn process_variable(declaration: &Node) -> Option<(String, DataSrc<Declaration>)> {
    if let Node::ListItem(declaration) = declaration
        && let Some(declaration) = declaration.children.first()
        && let Node::Paragraph(declaration) = declaration
        && let Some(declaration_type) = declaration.children.first()
        && let Node::InlineCode(variable_type) = declaration_type
        && let Some(declaration_name) = declaration.children.get(2)
        && let Node::Emphasis(declaration_name) = declaration_name
        && let Some(declaration_name) = declaration_name.children.first()
        && let Node::Text(variable_name) = declaration_name
        && let Some(declaration_value) = declaration.children.get(3)
        && let Node::Text(variable_value) = declaration_value
    {
        match variable_type.value.as_str() {
            "get-bool" | "get-numeric" | "get-text" | "get-event" | "get-image" | "get-color" => {
                let value = GlobalSymbol::new(variable_value.value.trim());
                Some((
                    variable_name.value.trim().to_string(),
                    DataSrc::<Declaration>::Dynamic(value),
                ))
            }
            "set-bool" => bool::from_str(variable_value.value.trim())
                .ok()
                .map(|value| {
                    (
                        variable_name.value.trim().to_string(),
                        DataSrc::<Declaration>::Static(Declaration::Bool(value)),
                    )
                }),
            "set-numeric" => f32::from_str(variable_value.value.trim())
                .ok()
                .map(|value| {
                    (
                        variable_name.value.trim().to_string(),
                        DataSrc::<Declaration>::Static(Declaration::Numeric(value)),
                    )
                }),
            "set-text" => Some((
                variable_name.value.trim().to_string(),
                DataSrc::<Declaration>::Static(Declaration::Text(
                    variable_value.value.trim().to_string(),
                )),
            )),
            "set-event" => Some((
                variable_name.value.trim().to_string(),
                DataSrc::<Declaration>::Static(Declaration::Event(GlobalSymbol::new(
                    variable_value.value.trim(),
                ))),
            )),
            "set-color" => Color::from_str(variable_value.value.trim())
                .ok()
                .map(|value| {
                    (
                        variable_name.value.trim().to_string(),
                        DataSrc::<Declaration>::Static(Declaration::Color(value)),
                    )
                }),
            _ => None,
        }
    } else {
        None
    }
}

fn process_configs(
    configuration_set: &List,
    custom_element: &mut Option<&mut CustomElement>,
) -> Vec<Layout> {
    let mut configs = Vec::new();

    for configuration_item in &configuration_set.children {
        if let Some(config_elements) = configuration_item.children()
            && let Some(config) = config_elements.first()
            && let Node::Paragraph(config) = config
            && let Some(config_type) = config.children.first()
            && let Node::InlineCode(config_type) = config_type
        {
            match config_type.value.as_str() {
                "grow" => configs.push(Layout::Config(Config::GrowAll)),
                "width-grow" => match parameter_check::<f32>(config, "min", "max") {
                    AvailableParameters::None => configs.push(Layout::Config(Config::GrowX)),
                    AvailableParameters::ADynamic(a) => {
                        configs.push(Layout::Config(Config::GrowXmin(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::AStatic(a) => {
                        configs.push(Layout::Config(Config::GrowXmin(DataSrc::Static(a))))
                    }
                    AvailableParameters::BDynamic(b) => {
                        configs.push(Layout::Config(Config::GrowXmax(DataSrc::Dynamic(b))))
                    }
                    AvailableParameters::BStatic(b) => {
                        configs.push(Layout::Config(Config::GrowXmax(DataSrc::Static(b))))
                    }
                    AvailableParameters::TwoStatic(min, max) => {
                        configs.push(Layout::Config(Config::GrowXminmax {
                            min: DataSrc::Static(min),
                            max: DataSrc::Static(max),
                        }))
                    }
                    AvailableParameters::TwoDynamic(min, max) => {
                        configs.push(Layout::Config(Config::GrowXminmax {
                            min: DataSrc::Dynamic(min),
                            max: DataSrc::Dynamic(max),
                        }))
                    }
                    AvailableParameters::ADynamicBStatic(min, max) => {
                        configs.push(Layout::Config(Config::GrowXminmax {
                            min: DataSrc::Dynamic(min),
                            max: DataSrc::Static(max),
                        }))
                    }
                    AvailableParameters::AStaticBDynamic(min, max) => {
                        configs.push(Layout::Config(Config::GrowXminmax {
                            min: DataSrc::Static(min),
                            max: DataSrc::Dynamic(max),
                        }))
                    }
                    _ => {}
                },
                "height-grow" => match parameter_check::<f32>(config, "min", "max") {
                    AvailableParameters::None => configs.push(Layout::Config(Config::GrowY)),
                    AvailableParameters::ADynamic(a) => {
                        configs.push(Layout::Config(Config::GrowYmin(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::AStatic(a) => {
                        configs.push(Layout::Config(Config::GrowYmin(DataSrc::Static(a))))
                    }
                    AvailableParameters::BDynamic(b) => {
                        configs.push(Layout::Config(Config::GrowYmax(DataSrc::Dynamic(b))))
                    }
                    AvailableParameters::BStatic(b) => {
                        configs.push(Layout::Config(Config::GrowYmax(DataSrc::Static(b))))
                    }
                    AvailableParameters::TwoStatic(min, max) => {
                        configs.push(Layout::Config(Config::GrowYminmax {
                            min: DataSrc::Static(min),
                            max: DataSrc::Static(max),
                        }))
                    }
                    AvailableParameters::TwoDynamic(min, max) => {
                        configs.push(Layout::Config(Config::GrowYminmax {
                            min: DataSrc::Dynamic(min),
                            max: DataSrc::Dynamic(max),
                        }))
                    }
                    AvailableParameters::ADynamicBStatic(min, max) => {
                        configs.push(Layout::Config(Config::GrowYminmax {
                            min: DataSrc::Dynamic(min),
                            max: DataSrc::Static(max),
                        }))
                    }
                    AvailableParameters::AStaticBDynamic(min, max) => {
                        configs.push(Layout::Config(Config::GrowYminmax {
                            min: DataSrc::Static(min),
                            max: DataSrc::Dynamic(max),
                        }))
                    }
                    _ => {}
                },
                "width-fit" => match parameter_check::<f32>(config, "min", "max") {
                    AvailableParameters::None => configs.push(Layout::Config(Config::FitX)),
                    AvailableParameters::ADynamic(a) => {
                        configs.push(Layout::Config(Config::FitXmin(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::AStatic(a) => {
                        configs.push(Layout::Config(Config::FitXmin(DataSrc::Static(a))))
                    }
                    AvailableParameters::BDynamic(b) => {
                        configs.push(Layout::Config(Config::FitXmax(DataSrc::Dynamic(b))))
                    }
                    AvailableParameters::BStatic(b) => {
                        configs.push(Layout::Config(Config::FitXmax(DataSrc::Static(b))))
                    }
                    AvailableParameters::TwoStatic(min, max) => {
                        configs.push(Layout::Config(Config::FitXminmax {
                            min: DataSrc::Static(min),
                            max: DataSrc::Static(max),
                        }))
                    }
                    AvailableParameters::TwoDynamic(min, max) => {
                        configs.push(Layout::Config(Config::FitXminmax {
                            min: DataSrc::Dynamic(min),
                            max: DataSrc::Dynamic(max),
                        }))
                    }
                    AvailableParameters::ADynamicBStatic(min, max) => {
                        configs.push(Layout::Config(Config::FitXminmax {
                            min: DataSrc::Dynamic(min),
                            max: DataSrc::Static(max),
                        }))
                    }
                    AvailableParameters::AStaticBDynamic(min, max) => {
                        configs.push(Layout::Config(Config::FitXminmax {
                            min: DataSrc::Static(min),
                            max: DataSrc::Dynamic(max),
                        }))
                    }
                    _ => {}
                },
                "height-fit" => match parameter_check::<f32>(config, "min", "max") {
                    AvailableParameters::None => configs.push(Layout::Config(Config::FitY)),
                    AvailableParameters::ADynamic(a) => {
                        configs.push(Layout::Config(Config::FitYmin(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::AStatic(a) => {
                        configs.push(Layout::Config(Config::FitYmin(DataSrc::Static(a))))
                    }
                    AvailableParameters::BDynamic(b) => {
                        configs.push(Layout::Config(Config::FitYmax(DataSrc::Dynamic(b))))
                    }
                    AvailableParameters::BStatic(b) => {
                        configs.push(Layout::Config(Config::FitYmax(DataSrc::Static(b))))
                    }
                    AvailableParameters::TwoStatic(min, max) => {
                        configs.push(Layout::Config(Config::FitYminmax {
                            min: DataSrc::Static(min),
                            max: DataSrc::Static(max),
                        }))
                    }
                    AvailableParameters::TwoDynamic(min, max) => {
                        configs.push(Layout::Config(Config::FitYminmax {
                            min: DataSrc::Dynamic(min),
                            max: DataSrc::Dynamic(max),
                        }))
                    }
                    AvailableParameters::ADynamicBStatic(min, max) => {
                        configs.push(Layout::Config(Config::FitYminmax {
                            min: DataSrc::Dynamic(min),
                            max: DataSrc::Static(max),
                        }))
                    }
                    AvailableParameters::AStaticBDynamic(min, max) => {
                        configs.push(Layout::Config(Config::FitYminmax {
                            min: DataSrc::Static(min),
                            max: DataSrc::Dynamic(max),
                        }))
                    }
                    _ => {}
                },
                "width-fixed" => match parameter_check::<f32>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::FixedX(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::FixedX(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "height-fixed" => match parameter_check::<f32>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::FixedY(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::FixedY(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "width-percent" => match parameter_check::<f32>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::PercentX(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::PercentX(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "height-percent" => match parameter_check::<f32>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::PercentY(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::PercentY(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "padding-all" => match parameter_check::<u16>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::PaddingAll(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::PaddingAll(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "padding-top" => match parameter_check::<u16>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::PaddingTop(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::PaddingTop(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "padding-right" => match parameter_check::<u16>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::PaddingRight(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::PaddingRight(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "padding-bottom" => match parameter_check::<u16>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::PaddingBottom(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::PaddingBottom(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "padding-left" => match parameter_check::<u16>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::PaddingLeft(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::PaddingLeft(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "child-gap" => match parameter_check::<u16>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::ChildGap(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::ChildGap(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "vertical" => configs.push(Layout::Config(Config::Vertical)),
                "align-children-x" => {
                    if let Some(alignment) = config.children.get(1)
                        && let Node::Text(alignment) = alignment
                    {
                        match alignment.value.trim() {
                            "left" => configs.push(Layout::Config(Config::ChildAlignmentXLeft)),
                            "right" => configs.push(Layout::Config(Config::ChildAlignmentXRight)),
                            "center" => configs.push(Layout::Config(Config::ChildAlignmentXCenter)),
                            _ => {}
                        }
                    }
                }
                "align-children-y" => {
                    if let Some(alignment) = config.children.get(1)
                        && let Node::Text(alignment) = alignment
                    {
                        match alignment.value.trim() {
                            "top" => configs.push(Layout::Config(Config::ChildAlignmentYTop)),
                            "bottom" => configs.push(Layout::Config(Config::ChildAlignmentYBottom)),
                            "center" => configs.push(Layout::Config(Config::ChildAlignmentYCenter)),
                            _ => {}
                        }
                    }
                }
                "color" => match parameter_check::<Color>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::Color(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::Color(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "width" => {
                    if let Some(custom_element) = custom_element
                        && let CustomElement::Line(line_config) = custom_element
                    {
                        match parameter_check::<f32>(config, "", "") {
                            AvailableParameters::SingleDynamic(a) => {
                                line_config.width_source = Some(a)
                            }
                            AvailableParameters::SingleStatic(a) => line_config.width = a,
                            _ => {}
                        }
                    }
                }
                "radius-all" => match parameter_check::<f32>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::RadiusAll(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::RadiusAll(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "radius-top-left" => match parameter_check::<f32>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::RadiusTopLeft(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::RadiusTopLeft(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "radius-top-right" => match parameter_check::<f32>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::RadiusTopRight(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::RadiusTopRight(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "radius-bottom-left" => match parameter_check::<f32>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(
                        Config::RadiusBottomLeft(DataSrc::Dynamic(a)),
                    )),
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::RadiusBottomLeft(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "radius-bottom-right" => match parameter_check::<f32>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(
                        Config::RadiusBottomRight(DataSrc::Dynamic(a)),
                    )),
                    AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(
                        Config::RadiusBottomRight(DataSrc::Static(a)),
                    )),
                    _ => {}
                },
                "border-color" => match parameter_check::<Color>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::BorderColor(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::BorderColor(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "border-all" => match parameter_check::<u16>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::BorderAll(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::BorderAll(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "border-top" => match parameter_check::<u16>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::BorderTop(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::BorderTop(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "border-left" => match parameter_check::<u16>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::BorderLeft(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::BorderLeft(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "border-bottom" => match parameter_check::<u16>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::BorderBottom(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::BorderBottom(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "border-right" => match parameter_check::<u16>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::BorderRight(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::BorderRight(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "border-in-between" => match parameter_check::<u16>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(
                        Config::BorderBetweenChildren(DataSrc::Dynamic(a)),
                    )),
                    AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(
                        Config::BorderBetweenChildren(DataSrc::Static(a)),
                    )),
                    _ => {}
                },
                "scroll" => {
                    if let Some(direction_a) = config.children.get(2)
                        && let Node::InlineCode(direction_a) = direction_a
                        && (direction_a.value.as_str() == "x" || direction_a.value.as_str() == "y")
                        && let Some(direction_b) = config.children.get(4)
                        && let Node::InlineCode(direction_b) = direction_b
                        && (direction_b.value.as_str() == "x" || direction_b.value.as_str() == "y")
                    {
                        configs.push(Layout::Config(Config::Clip {
                            vertical: DataSrc::Static(true),
                            horizontal: DataSrc::Static(true),
                        }));
                    } else if let Some(direction_a) = config.children.get(2)
                        && let Node::InlineCode(direction_a) = direction_a
                        && (direction_a.value.as_str() == "x" || direction_a.value.as_str() == "y")
                    {
                        if direction_a.value.as_str() == "x" {
                            configs.push(Layout::Config(Config::Clip {
                                vertical: DataSrc::Static(false),
                                horizontal: DataSrc::Static(true),
                            }));
                        } else {
                            configs.push(Layout::Config(Config::Clip {
                                vertical: DataSrc::Static(true),
                                horizontal: DataSrc::Static(false),
                            }));
                        }
                    }
                }
                "image" => {
                    if let Some(src) = config.children.get(1)
                        && let Node::Text(src) = src
                    {
                        let src = GlobalSymbol::new(src.value.trim());
                        configs.push(Layout::Config(Config::Image { name: src }));
                    }
                }
                "floating" => {
                    configs.push(Layout::Config(Config::Floating));
                    if let Some(floating_commands) = config_elements.get(1)
                        && let Node::List(floating_commands) = floating_commands
                    {
                        let mut floating = process_configs(floating_commands, &mut None);
                        configs.append(&mut floating);
                    }
                }
                "use" => {
                    if let Some(reusable_name) = config.children.get(1)
                        && let Node::Text(reusable_name) = reusable_name
                    {
                        let reusable_name = GlobalSymbol::new(reusable_name.value.trim());
                        configs.push(Layout::Config(Config::Use {
                            name: reusable_name,
                        }));
                    }
                }

                "hovered" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::HoveredOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::HoveredOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => {
                            configs.push(Layout::Element(Element::HoveredOpened { event: None }))
                        }
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None));
                    }
                    configs.push(Layout::Element(Element::HoveredClosed));
                }
                "unhovered" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::UnHoveredOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::UnHoveredOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => {
                            configs.push(Layout::Element(Element::UnHoveredOpened { event: None }))
                        }
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None));
                    }
                    configs.push(Layout::Element(Element::UnHoveredClosed));
                }
                "hover" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::HoverOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::HoverOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => {
                            configs.push(Layout::Element(Element::HoverOpened { event: None }))
                        }
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None));
                    }
                    configs.push(Layout::Element(Element::HoverClosed));
                }
                "focused" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::FocusedOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::FocusedOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => {
                            configs.push(Layout::Element(Element::FocusedOpened { event: None }))
                        }
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None));
                    }
                    configs.push(Layout::Element(Element::FocusedClosed));
                }
                "unfocused" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::UnFocusedOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::UnFocusedOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => {
                            configs.push(Layout::Element(Element::UnFocusedOpened { event: None }))
                        }
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None));
                    }
                    configs.push(Layout::Element(Element::UnFocusedClosed));
                }
                "focus" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::FocusOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::FocusOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => {
                            configs.push(Layout::Element(Element::FocusOpened { event: None }))
                        }
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None));
                    }
                    configs.push(Layout::Element(Element::FocusClosed));
                }
                "left-pressed" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::LeftPressedOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::LeftPressedOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => configs
                            .push(Layout::Element(Element::LeftPressedOpened { event: None })),
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None));
                    }
                    configs.push(Layout::Element(Element::LeftPressedClosed));
                }
                "left-down" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::LeftDownOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::LeftDownOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => {
                            configs.push(Layout::Element(Element::LeftDownOpened { event: None }))
                        }
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None));
                    }
                    configs.push(Layout::Element(Element::LeftDownClosed));
                }
                "left-released" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::LeftReleasedOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::LeftReleasedOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => configs
                            .push(Layout::Element(Element::LeftReleasedOpened { event: None })),
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None));
                    }
                    configs.push(Layout::Element(Element::LeftReleasedClosed));
                }
                "left-clicked" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::LeftClickedOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::LeftClickedOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => configs
                            .push(Layout::Element(Element::LeftClickedOpened { event: None })),
                        _ => {}
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                        && let Node::List(config_on_click) = config_on_click
                    {
                        configs.append(&mut process_configs(config_on_click, &mut None));
                    }
                    configs.push(Layout::Element(Element::LeftClickedClosed));
                }
                "left-dbl-clicked" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::LeftDoubleClickedOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::LeftDoubleClickedOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => {
                            configs.push(Layout::Element(Element::LeftDoubleClickedOpened {
                                event: None,
                            }))
                        }
                        _ => {}
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                        && let Node::List(config_on_click) = config_on_click
                    {
                        configs.append(&mut process_configs(config_on_click, &mut None));
                    }
                    configs.push(Layout::Element(Element::LeftDoubleClickedClosed));
                }
                "left-tpl-clicked" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::LeftTripleClickedOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::LeftTripleClickedOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => {
                            configs.push(Layout::Element(Element::LeftTripleClickedOpened {
                                event: None,
                            }))
                        }
                        _ => {}
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                        && let Node::List(config_on_click) = config_on_click
                    {
                        configs.append(&mut process_configs(config_on_click, &mut None));
                    }
                    configs.push(Layout::Element(Element::LeftTripleClickedClosed));
                }
                "right-pressed" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::RightPressedOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::RightPressedOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => configs
                            .push(Layout::Element(Element::RightPressedOpened { event: None })),
                        _ => {}
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                        && let Node::List(config_on_click) = config_on_click
                    {
                        configs.append(&mut process_configs(config_on_click, &mut None));
                    }
                    configs.push(Layout::Element(Element::RightPressedClosed));
                }
                "right-down" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::RightDownOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::RightDownOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => {
                            configs.push(Layout::Element(Element::RightDownOpened { event: None }))
                        }
                        _ => {}
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                        && let Node::List(config_on_click) = config_on_click
                    {
                        configs.append(&mut process_configs(config_on_click, &mut None));
                    }
                    configs.push(Layout::Element(Element::RightDownClosed));
                }
                "right-released" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::RightReleasedOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::RightReleasedOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => {
                            configs.push(Layout::Element(Element::RightReleasedOpened {
                                event: None,
                            }))
                        }
                        _ => {}
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                        && let Node::List(config_on_click) = config_on_click
                    {
                        configs.append(&mut process_configs(config_on_click, &mut None));
                    }
                    configs.push(Layout::Element(Element::RightReleasedClosed));
                }
                "right-clicked" => {
                    match parameter_check::<GlobalSymbol>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::RightClickedOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::RightClickedOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => configs
                            .push(Layout::Element(Element::RightClickedOpened { event: None })),
                        _ => {}
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                        && let Node::List(config_on_click) = config_on_click
                    {
                        configs.append(&mut process_configs(config_on_click, &mut None));
                    }
                    configs.push(Layout::Element(Element::RightClickedClosed));
                }
                "pointer" => {
                    if let Some(pointer) = config.children.get(1)
                        && let Node::Text(pointer) = pointer
                    {
                        match pointer.value.trim() {
                            "standard" => configs.push(Layout::Element(Element::Pointer(
                                winit::window::CursorIcon::Default,
                            ))),
                            "resize-horizontal" => configs.push(Layout::Element(Element::Pointer(
                                winit::window::CursorIcon::EwResize,
                            ))),
                            _ => {}
                        }
                    }
                }

                "font-id" => match parameter_check::<u16>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::FontId(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::FontId(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "font-size" => match parameter_check::<u16>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::FontSize(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::FontSize(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "align" => {
                    if let Some(alignment) = config.children.get(1)
                        && let Node::Text(alignment) = alignment
                    {
                        match alignment.value.trim() {
                            "left" => configs.push(Layout::Config(Config::AlignLeft)),
                            "center" => configs.push(Layout::Config(Config::AlignCenter)),
                            "right" => configs.push(Layout::Config(Config::AlignRight)),
                            _ => {}
                        }
                    }
                }
                "line-height" => match parameter_check::<u16>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::LineHeight(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::LineHeight(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                // TODO: letter-spacing isn't exposed by telera-layout's TextConfig yet.
                "letter-spacing" => {}
                "font-color" => match parameter_check::<Color>(config, "", "") {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::FontColor(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::FontColor(DataSrc::Static(a))))
                    }
                    _ => {}
                },

                "offset" => match parameter_check::<f32>(config, "x", "y") {
                    AvailableParameters::ADynamic(a) => {
                        configs.push(Layout::Config(Config::FloatingOffset {
                            x: DataSrc::Dynamic(a),
                            y: DataSrc::Static(0.0),
                        }))
                    }
                    AvailableParameters::AStatic(a) => {
                        configs.push(Layout::Config(Config::FloatingOffset {
                            x: DataSrc::Static(a),
                            y: DataSrc::Static(0.0),
                        }))
                    }
                    AvailableParameters::BDynamic(b) => {
                        configs.push(Layout::Config(Config::FloatingOffset {
                            x: DataSrc::Static(0.0),
                            y: DataSrc::Dynamic(b),
                        }))
                    }
                    AvailableParameters::BStatic(b) => {
                        configs.push(Layout::Config(Config::FloatingOffset {
                            x: DataSrc::Static(0.0),
                            y: DataSrc::Static(b),
                        }))
                    }
                    AvailableParameters::TwoStatic(a, b) => {
                        configs.push(Layout::Config(Config::FloatingOffset {
                            x: DataSrc::Static(a),
                            y: DataSrc::Static(b),
                        }))
                    }
                    AvailableParameters::TwoDynamic(x, y) => {
                        configs.push(Layout::Config(Config::FloatingOffset {
                            x: DataSrc::Dynamic(x),
                            y: DataSrc::Dynamic(y),
                        }))
                    }
                    AvailableParameters::ADynamicBStatic(x, y) => {
                        configs.push(Layout::Config(Config::FloatingOffset {
                            x: DataSrc::Dynamic(x),
                            y: DataSrc::Static(y),
                        }))
                    }
                    AvailableParameters::AStaticBDynamic(x, y) => {
                        configs.push(Layout::Config(Config::FloatingOffset {
                            x: DataSrc::Static(x),
                            y: DataSrc::Dynamic(y),
                        }))
                    }
                    _ => {}
                },
                "attatch-parent" => {
                    if let Some(attach_point) = config.children.get(1)
                        && let Node::Text(attach_point) = attach_point
                    {
                        match attach_point.value.trim() {
                            "top-left" => configs
                                .push(Layout::Config(Config::FloatingAttatchToParentAtTopLeft)),
                            "center-left" => configs
                                .push(Layout::Config(Config::FloatingAttatchToParentAtCenterLeft)),
                            "bottom-left" => configs
                                .push(Layout::Config(Config::FloatingAttatchToParentAtBottomLeft)),
                            "top-center" => configs
                                .push(Layout::Config(Config::FloatingAttatchToParentAtTopCenter)),
                            "center" => configs
                                .push(Layout::Config(Config::FloatingAttatchToParentAtCenter)),
                            "bottom-center" => configs.push(Layout::Config(
                                Config::FloatingAttatchToParentAtBottomCenter,
                            )),
                            "top-right" => configs
                                .push(Layout::Config(Config::FloatingAttatchToParentAtTopRight)),
                            "center-right" => configs
                                .push(Layout::Config(Config::FloatingAttatchToParentAtCenterRight)),
                            "bottom-right" => configs
                                .push(Layout::Config(Config::FloatingAttatchToParentAtBottomRight)),
                            _ => {}
                        }
                    }
                }
                "attach-self" => {
                    if let Some(attach_point) = config.children.get(1)
                        && let Node::Text(attach_point) = attach_point
                    {
                        match attach_point.value.trim() {
                            "top-left" => configs
                                .push(Layout::Config(Config::FloatingAttatchElementAtTopLeft)),
                            "center-left" => configs
                                .push(Layout::Config(Config::FloatingAttatchElementAtCenterLeft)),
                            "bottom-left" => configs
                                .push(Layout::Config(Config::FloatingAttatchElementAtBottomLeft)),
                            "top-center" => configs
                                .push(Layout::Config(Config::FloatingAttatchElementAtTopCenter)),
                            "center" => {
                                configs.push(Layout::Config(Config::FloatingAttatchElementAtCenter))
                            }
                            "bottom-center" => configs
                                .push(Layout::Config(Config::FloatingAttatchElementAtBottomCenter)),
                            "top-right" => configs
                                .push(Layout::Config(Config::FloatingAttatchElementAtTopRight)),
                            "center-right" => configs
                                .push(Layout::Config(Config::FloatingAttatchElementAtCenterRight)),
                            "bottom-right" => configs
                                .push(Layout::Config(Config::FloatingAttatchElementAtBottomRight)),
                            _ => {}
                        }
                    }
                }
                // TODO: z-index, pointer pass through
                _ => {}
            }
        }
    }

    configs
}

// ---------------------------------------------------------------------------
// Runner: walks the flattened commands each frame
// ---------------------------------------------------------------------------

/// Scans a page's top-level `declarations` - the ones that sit directly in
/// the page body, not inside a `list`/`use`/`item`/`treeview` (those scope
/// their own declarations to their own body, via `recursive_call_stack` in
/// `set_layout`, and shouldn't leak into the rest of the page). These are
/// the ones a whole page can reference anywhere, e.g. `*content background
/// color*` declared once at the top of `main.md` and used by several
/// unrelated elements further down.
///
/// Returns owned values (rather than references into `commands`) so the
/// caller can hold this independently of the `&mut` borrow `set_layout`
/// itself needs on `commands`.
fn extract_page_locals(commands: &[Layout]) -> HashMap<GlobalSymbol, DataSrc<Declaration>> {
    let mut locals = HashMap::new();
    let mut depth: u32 = 0;
    for command in commands {
        match command {
            Layout::Element(
                Element::ListOpened
                | Element::UseOpened
                | Element::ItemOpened
                | Element::TreeViewOpened,
            ) => {
                depth += 1;
            }
            Layout::Element(
                Element::ListClosed(_)
                | Element::UseClosed(_)
                | Element::ItemClosed { .. }
                | Element::TreeViewClosed(_),
            ) => {
                depth = depth.saturating_sub(1);
            }
            Layout::Declaration { name, value } if depth == 0 => {
                locals.insert(*name, value.clone());
            }
            _ => {}
        }
    }
    locals
}

/// Combines a scope's own declarations (`inner` - e.g. a `list`'s per-item
/// bindings) with whatever locals were already visible around it (`outer` -
/// e.g. the page-level ones from [`extract_page_locals`]), so a nested
/// scope doesn't lose access to them. `inner` wins on name collisions.
fn merge_locals<'a>(
    outer: Option<&HashMap<GlobalSymbol, &'a DataSrc<Declaration>>>,
    inner: &HashMap<GlobalSymbol, &'a DataSrc<Declaration>>,
) -> HashMap<GlobalSymbol, &'a DataSrc<Declaration>> {
    let mut merged = outer.cloned().unwrap_or_default();
    merged.extend(inner.iter().map(|(name, value)| (*name, *value)));
    merged
}

/// Owns every page and reusable snippet a `markdown` document was parsed
/// into, and drives them against the layout engine each frame.
pub struct Binder {
    pages: HashMap<String, Vec<Layout>>,
    pub reusable: HashMap<GlobalSymbol, Vec<Layout>>,
}

impl Default for Binder {
    fn default() -> Self {
        Self::new()
    }
}

impl Binder {
    pub fn new() -> Self {
        Self {
            pages: HashMap::new(),
            reusable: HashMap::new(),
        }
    }

    /// Adds a page, replacing any existing page of the same name.
    pub fn add_page(&mut self, name: &str, page: Vec<Layout>) {
        self.pages.insert(name.to_string(), page);
    }

    /// Adds a reusable snippet, replacing any existing one of the same name.
    pub fn add_reusable(&mut self, name: &str, reusable: Vec<Layout>) {
        self.reusable.insert(GlobalSymbol::new(name), reusable);
    }

    /// Parses a markdown layout document (see [`process_layout`]) and
    /// registers it as the page `name`, along with any reusable snippets it
    /// defines, replacing any existing ones of the same names. The document
    /// itself carries no page name (its `# ...` heading is ignored) - the
    /// caller chooses it; `API` passes the layout file's name minus `.md`.
    ///
    /// This only parses `markdown_source` - reading it from disk (or
    /// embedding it with `include_str!`) is left to the caller.
    pub fn load_layout(&mut self, name: &str, markdown_source: &str) -> Result<(), String> {
        let (body, reusables) = process_layout(markdown_source.to_string())?;
        for (reusable_name, reusable) in reusables {
            self.add_reusable(&reusable_name, reusable);
        }
        self.add_page(name, body);
        Ok(())
    }

    /// Replaces an existing page, returning `false` (and leaving it
    /// untouched) if `name` isn't a known page.
    pub fn replace_page(&mut self, name: &str, page: Vec<Layout>) -> bool {
        if !self.pages.contains_key(name) {
            return false;
        }
        self.pages.insert(name.to_string(), page);
        true
    }

    /// Replaces an existing reusable snippet, returning `false` (and
    /// leaving it untouched) if `name` isn't a known reusable.
    pub fn replace_reusable(&mut self, name: &str, reusable: Vec<Layout>) -> bool {
        let name = GlobalSymbol::new(name);
        if !self.reusable.contains_key(&name) {
            return false;
        }
        self.reusable.insert(name, reusable);
        true
    }

    /// Runs `page`'s layout commands for this frame, driving `api.l` and
    /// dispatching every event that fires straight into `user_app` via
    /// [`LayoutReflector::dispatch_event`]. Returns `None` (doing nothing)
    /// if `page` isn't a known page.
    pub fn set_page<UserApp>(
        &mut self,
        page: &str,
        api: &mut API,
        mt: &mut MT,
        user_app: &mut UserApp,
    ) -> Option<()>
    where
        UserApp: LayoutRunnerReflection + LayoutReflector,
    {
        let layout_commands = self.pages.get_mut(page)?;

        // Owned so it can outlive the `&mut layout_commands` borrow below -
        // see `extract_page_locals`.
        let page_locals_owned = extract_page_locals(layout_commands);
        let page_locals: HashMap<GlobalSymbol, &DataSrc<Declaration>> = page_locals_owned
            .iter()
            .map(|(name, value)| (*name, value))
            .collect();

        let mut config = ElementConfiguration::default();
        let mut text_config = TextConfig::default();

        let _pointer = set_layout(
            api,
            mt,
            layout_commands,
            &mut self.reusable,
            Some(&page_locals),
            None,
            &mut config,
            &mut text_config,
            user_app,
            winit::window::CursorIcon::Default,
        );

        Some(())
    }
}

/// `if-index`/`if-index-not`'s condition: true when inside a `list` and the
/// current iteration index equals `index`; always false outside any list
/// (there's no iteration index to compare against).
fn index_matches<UserApp>(
    index: &DataSrc<f32>,
    locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
    user_app: &UserApp,
    list_data: &Option<(GlobalSymbol, usize)>,
) -> bool
where
    UserApp: LayoutRunnerReflection,
{
    let Some((_, current_index)) = list_data else {
        return false;
    };
    *current_index as f32 == f32::resolve_src(index, locals, user_app, list_data)
}

#[allow(clippy::too_many_arguments)]
fn set_layout<UserApp>(
    api: &mut API,
    mt: &mut MT,
    commands: &mut [Layout],
    reusables: &mut HashMap<GlobalSymbol, Vec<Layout>>,
    locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
    list_data: Option<(GlobalSymbol, usize)>,
    config: &mut ElementConfiguration,
    text_config: &mut TextConfig,
    user_app: &mut UserApp,
    mut pointer: winit::window::CursorIcon,
) -> winit::window::CursorIcon
where
    UserApp: LayoutRunnerReflection + LayoutReflector,
{
    let mut nesting_level: u32 = 0;
    let mut skip: Option<u32> = None;

    let mut recursive_commands = Vec::<Layout>::new();
    let mut recursive_call_stack = HashMap::<GlobalSymbol, &DataSrc<Declaration>>::new();
    let mut collect_declarations = false;
    let mut collect_list_commands = false;

    // Opens/closes that just gate on a condition and (optionally) fire an
    // event the first frame the condition becomes true. Written as macros
    // (rather than a helper fn) so they can freely use the locals above -
    // this is the same "macro captures the enclosing scope" trick the `e!`/
    // `t!` layout macros elsewhere in this crate rely on.
    macro_rules! event_gate_open {
        ($condition:expr, $event:expr) => {{
            if skip.is_none() {
                skip = Some(nesting_level);
                if $condition {
                    skip = None;
                    if let Some(event) = $event {
                        // `resolve_src` only borrows `user_app` shared and
                        // returns an owned `GlobalSymbol`, so the `&mut` for
                        // `dispatch_event` is free by the time we need it.
                        let handler =
                            GlobalSymbol::resolve_src(event, locals, user_app, &list_data);
                        let context = build_event_context(&list_data);
                        user_app.dispatch_event(&handler, context, api);
                    }
                }
            }
            nesting_level += 1;
        }};
    }
    macro_rules! event_gate_close {
        () => {{
            nesting_level -= 1;
            if let Some(skip_level) = skip
                && skip_level == nesting_level
            {
                skip = None;
            }
        }};
    }
    // Not backed by any state `API` tracks yet, so the gated block never
    // runs. Kept explicit (rather than silently ignored) so it's clear this
    // is a known gap rather than a bug.
    macro_rules! event_gate_unsupported {
        () => {{
            if skip.is_none() {
                skip = Some(nesting_level);
            }
            nesting_level += 1;
        }};
    }

    for command in commands.iter_mut() {
        if collect_list_commands {
            match command {
                Layout::Element(Element::ListClosed(_) | Element::ItemClosed { .. }) => {
                    collect_list_commands = false;
                    // fall through: the second match below runs the replay.
                }
                Layout::Declaration { .. } => {
                    // fall through: the second match below fills recursive_call_stack.
                }
                other => {
                    collect_declarations = false;
                    recursive_commands.push(other.clone());
                    continue;
                }
            }
        }

        match command {
            Layout::Element(element) => {
                match element {
                    Element::IfOpened { condition } => {
                        if skip.is_none()
                            && !bool::resolve_name(condition, locals, user_app, &list_data)
                        {
                            skip = Some(nesting_level)
                        }
                        nesting_level += 1;
                    }
                    Element::IfNotOpened { condition } => {
                        if skip.is_none()
                            && bool::resolve_name(condition, locals, user_app, &list_data)
                        {
                            skip = Some(nesting_level)
                        }
                        nesting_level += 1;
                    }
                    Element::IfClosed => {
                        nesting_level -= 1;
                        if let Some(skip_level) = skip
                            && skip_level >= nesting_level
                        {
                            skip = None;
                        }
                    }

                    Element::IfIndexOpened { index } => {
                        if skip.is_none() && !index_matches(index, locals, user_app, &list_data) {
                            skip = Some(nesting_level)
                        }
                        nesting_level += 1;
                    }
                    Element::IfIndexNotOpened { index } => {
                        if skip.is_none() && index_matches(index, locals, user_app, &list_data) {
                            skip = Some(nesting_level)
                        }
                        nesting_level += 1;
                    }

                    Element::HoverOpened { event } => event_gate_open!(api.l.hovered(), event),
                    Element::HoverClosed => event_gate_close!(),
                    Element::HoveredOpened { .. } => event_gate_unsupported!(), // TODO: needs hover-edge tracking in `API`
                    Element::HoveredClosed => event_gate_close!(),
                    Element::UnHoveredOpened { .. } => event_gate_unsupported!(), // TODO: needs hover-edge tracking in `API`
                    Element::UnHoveredClosed => event_gate_close!(),

                    Element::FocusOpened { .. } => event_gate_unsupported!(), // TODO: needs the resolved element id before it's assigned
                    Element::FocusClosed => event_gate_close!(),
                    Element::FocusedOpened { .. } => event_gate_unsupported!(),
                    Element::FocusedClosed => event_gate_close!(),
                    Element::UnFocusedOpened { .. } => event_gate_unsupported!(),
                    Element::UnFocusedClosed => event_gate_close!(),

                    Element::LeftPressedOpened { event } => {
                        event_gate_open!(api.left_mouse_pressed, event)
                    }
                    Element::LeftPressedClosed => event_gate_close!(),
                    Element::LeftDownOpened { event } => {
                        event_gate_open!(api.left_mouse_down, event)
                    }
                    Element::LeftDownClosed => event_gate_close!(),
                    Element::LeftReleasedOpened { event } => {
                        event_gate_open!(api.left_mouse_released, event)
                    }
                    Element::LeftReleasedClosed => event_gate_close!(),
                    Element::LeftClickedOpened { event } => {
                        event_gate_open!(api.l.hovered() && api.left_mouse_clicked, event)
                    }
                    Element::LeftClickedClosed => event_gate_close!(),
                    Element::LeftDoubleClickedOpened { event } => {
                        event_gate_open!(api.l.hovered() && api.left_mouse_double_clicked, event)
                    }
                    Element::LeftDoubleClickedClosed => event_gate_close!(),
                    Element::LeftTripleClickedOpened { .. } => event_gate_unsupported!(), // TODO: `API` doesn't track triple-clicks yet
                    Element::LeftTripleClickedClosed => event_gate_close!(),

                    Element::RightPressedOpened { event } => {
                        event_gate_open!(api.right_mouse_pressed, event)
                    }
                    Element::RightPressedClosed => event_gate_close!(),
                    Element::RightDownOpened { event } => {
                        event_gate_open!(api.right_mouse_down, event)
                    }
                    Element::RightDownClosed => event_gate_close!(),
                    Element::RightReleasedOpened { event } => {
                        event_gate_open!(api.right_mouse_released, event)
                    }
                    Element::RightReleasedClosed => event_gate_close!(),
                    Element::RightClickedOpened { event } => {
                        event_gate_open!(api.l.hovered() && api.right_mouse_clicked, event)
                    }
                    Element::RightClickedClosed => event_gate_close!(),

                    Element::Pointer(new_pointer) => {
                        if skip.is_none() {
                            pointer = *new_pointer;
                        }
                    }

                    Element::ListOpened => {
                        nesting_level += 1;
                        if skip.is_none() {
                            recursive_commands.clear();
                            recursive_call_stack.clear();
                            collect_list_commands = true;
                            collect_declarations = true;
                        }
                    }
                    Element::ListClosed(src) => {
                        nesting_level -= 1;
                        if skip.is_none()
                            && let Some(length) = user_app.get_list_length(src, &None)
                        {
                            let merged_locals = merge_locals(locals, &recursive_call_stack);
                            for index in 0..length {
                                let mut item_config = ElementConfiguration::default();
                                let mut item_text_config = TextConfig::default();
                                pointer = set_layout(
                                    api,
                                    mt,
                                    &mut recursive_commands,
                                    reusables,
                                    Some(&merged_locals),
                                    Some((*src, index)),
                                    &mut item_config,
                                    &mut item_text_config,
                                    user_app,
                                    pointer,
                                );
                            }
                        }
                    }

                    Element::ItemOpened => {
                        nesting_level += 1;
                        if skip.is_none() {
                            recursive_commands.clear();
                            recursive_call_stack.clear();
                            collect_list_commands = true;
                        }
                    }
                    Element::ItemClosed { list, index } => {
                        nesting_level -= 1;
                        if skip.is_none() {
                            let resolved_index =
                                f32::resolve_src(index, locals, user_app, &list_data) as usize;
                            let merged_locals = merge_locals(locals, &recursive_call_stack);
                            let mut item_config = ElementConfiguration::default();
                            let mut item_text_config = TextConfig::default();
                            pointer = set_layout(
                                api,
                                mt,
                                &mut recursive_commands,
                                reusables,
                                Some(&merged_locals),
                                Some((*list, resolved_index)),
                                &mut item_config,
                                &mut item_text_config,
                                user_app,
                                pointer,
                            );
                        }
                    }

                    Element::ElementOpened { id: _ } => {
                        nesting_level += 1;
                        if skip.is_none() {
                            api.l.open_element();
                        }
                    }
                    Element::ElementClosed => {
                        nesting_level -= 1;
                        if skip.is_none() {
                            api.l.close_element();
                        }
                    }
                    Element::CircleOpened { id: _ } => {
                        nesting_level += 1;
                        if skip.is_none() {
                            api.l.open_element();
                        }
                    }
                    Element::CircleClosed => {
                        nesting_level -= 1;
                        if skip.is_none() {
                            api.l.close_element();
                        }
                    }
                    Element::LineOpened { id: _ } => {
                        nesting_level += 1;
                        if skip.is_none() {
                            api.l.open_element();
                        }
                    }
                    Element::LineClosed => {
                        nesting_level -= 1;
                        if skip.is_none() {
                            api.l.close_element();
                        }
                    }

                    Element::ConfigOpened => {
                        nesting_level += 1;
                        if skip.is_none() {
                            *config = ElementConfiguration::default();
                        }
                    }
                    Element::ConfigClosed => {
                        nesting_level -= 1;
                        if skip.is_none() {
                            let id = api.l.configure_element(config);
                            if api.l.hovered() && api.left_mouse_clicked {
                                api.focus = id;
                            }
                        }
                    }

                    Element::TextElementOpened => nesting_level += 1,
                    Element::TextElementClosed(content) => {
                        nesting_level -= 1;
                        if skip.is_none() {
                            let text_content =
                                String::resolve_src(content, locals, user_app, &list_data);
                            api.l.add_text_element(text_content, text_config, false, mt);
                        }
                    }
                    Element::TextConfigOpened => {
                        nesting_level += 1;
                        if skip.is_none() {
                            *text_config = TextConfig::default();
                        }
                    }
                    Element::TextConfigClosed => {
                        nesting_level -= 1;
                    }

                    Element::UseOpened => {
                        nesting_level += 1;
                        if skip.is_none() {
                            recursive_commands.clear();
                            recursive_call_stack.clear();
                            collect_declarations = true;
                        }
                    }
                    Element::UseClosed(src) => {
                        nesting_level -= 1;
                        if skip.is_none() {
                            collect_declarations = false;
                            if let Some(reusable) = reusables.get(src) {
                                for command in reusable.iter() {
                                    recursive_commands.push(command.clone());
                                }
                                let merged_locals = merge_locals(locals, &recursive_call_stack);
                                pointer = set_layout(
                                    api,
                                    mt,
                                    &mut recursive_commands,
                                    reusables,
                                    Some(&merged_locals),
                                    None,
                                    config,
                                    text_config,
                                    user_app,
                                    pointer,
                                );
                            }
                        }
                    }

                    Element::TreeViewOpened => {
                        nesting_level += 1;
                        if skip.is_none() {
                            recursive_commands.clear();
                            recursive_call_stack.clear();
                            collect_declarations = true;
                        }
                    }
                    Element::TreeViewClosed(src) => {
                        nesting_level -= 1;
                        if skip.is_none() {
                            collect_declarations = false;
                            // `treeview` walks a tree borrowed out of `user_app`
                            // (`get_treeview` -> `TreeViewItem<'_>`), so it can't
                            // hold `&mut user_app` to dispatch - it returns its
                            // events and we drain them here, borrow released.
                            for (handler, context) in treeview(src, &list_data, api, mt, user_app) {
                                user_app.dispatch_event(&handler, context, api);
                            }
                        }
                    }

                    Element::TextBoxOpened => {
                        nesting_level += 1;
                        if skip.is_none() {
                            recursive_commands.clear();
                            recursive_call_stack.clear();
                            collect_declarations = true;
                            // TODO: wire up ui_toolkit::textbox once it's finished; for now
                            // a text box is laid out as an empty, focusable element. It still
                            // has to be a real (opened) element, though - `TextBoxClosed`
                            // always calls `close_element`, and skipping `open_element` here
                            // would leave that close unmatched, corrupting Clay's internal
                            // element stack for the rest of the frame.
                            api.l.open_element();
                            if api.l.hovered() {
                                pointer = winit::window::CursorIcon::Text;
                            }
                            api.l.configure_element(&ElementConfiguration::default());
                        }
                    }
                    Element::TextBoxClosed(_src) => {
                        nesting_level -= 1;
                        if skip.is_none() {
                            collect_declarations = false;
                            api.l.close_element();
                        }
                    }

                    Element::FunctionCall(name) => {
                        if skip.is_none() {
                            user_app.dispatch_custom_element(name, api, mt);
                        }
                    }
                }
            }
            Layout::Declaration { name, value } => {
                if collect_declarations {
                    recursive_call_stack.insert(*name, value);
                }
            }
            Layout::Config(config_command) => {
                if skip.is_none() {
                    execute_config(
                        config_command,
                        config,
                        text_config,
                        reusables,
                        locals,
                        &list_data,
                        api,
                        user_app,
                    );
                }
            }
        }
    }

    pointer
}

#[allow(clippy::too_many_arguments)]
fn execute_config<UserApp>(
    config_command: &mut Config,
    config: &mut ElementConfiguration,
    text_config: &mut TextConfig,
    reusables: &HashMap<GlobalSymbol, Vec<Layout>>,
    locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
    list_data: &Option<(GlobalSymbol, usize)>,
    api: &mut API,
    user_app: &UserApp,
) where
    UserApp: LayoutRunnerReflection,
{
    match config_command {
        Config::Id(id) => {
            if let DataSrc::Static(id) = id {
                config.id(id.as_str());
            }
        }
        Config::FitX => {
            config.x_fit();
        }
        Config::FitXmin(min) => {
            config.x_fit_min(f32::resolve_src(min, locals, user_app, list_data));
        }
        Config::FitXmax(max) => {
            config.x_fit_min_max(0.0, f32::resolve_src(max, locals, user_app, list_data));
        }
        Config::FitXminmax { min, max } => {
            config.x_fit_min_max(
                f32::resolve_src(min, locals, user_app, list_data),
                f32::resolve_src(max, locals, user_app, list_data),
            );
        }
        Config::FitY => {
            config.y_fit();
        }
        Config::FitYmin(min) => {
            config.y_fit_min(f32::resolve_src(min, locals, user_app, list_data));
        }
        Config::FitYmax(max) => {
            config.y_fit_min_max(0.0, f32::resolve_src(max, locals, user_app, list_data));
        }
        Config::FitYminmax { min, max } => {
            config.y_fit_min_max(
                f32::resolve_src(min, locals, user_app, list_data),
                f32::resolve_src(max, locals, user_app, list_data),
            );
        }
        Config::GrowX => {
            config.x_grow();
        }
        Config::GrowXmin(min) => {
            config.x_grow_min(f32::resolve_src(min, locals, user_app, list_data));
        }
        Config::GrowXmax(max) => {
            config.x_grow_min_max(0.0, f32::resolve_src(max, locals, user_app, list_data));
        }
        Config::GrowXminmax { min, max } => {
            config.x_grow_min_max(
                f32::resolve_src(min, locals, user_app, list_data),
                f32::resolve_src(max, locals, user_app, list_data),
            );
        }
        Config::GrowY => {
            config.y_grow();
        }
        Config::GrowYmin(min) => {
            config.y_grow_min(f32::resolve_src(min, locals, user_app, list_data));
        }
        Config::GrowYmax(max) => {
            config.y_grow_min_max(0.0, f32::resolve_src(max, locals, user_app, list_data));
        }
        Config::GrowYminmax { min, max } => {
            config.y_grow_min_max(
                f32::resolve_src(min, locals, user_app, list_data),
                f32::resolve_src(max, locals, user_app, list_data),
            );
        }
        Config::FixedX(size) => {
            config.x_fixed(f32::resolve_src(size, locals, user_app, list_data));
        }
        Config::FixedY(size) => {
            config.y_fixed(f32::resolve_src(size, locals, user_app, list_data));
        }
        Config::PercentX(size) => {
            config.x_percent(f32::resolve_src(size, locals, user_app, list_data));
        }
        Config::PercentY(size) => {
            config.y_percent(f32::resolve_src(size, locals, user_app, list_data));
        }
        Config::GrowAll => {
            config.grow_all();
        }
        Config::PaddingAll(padding) => {
            config.padding_all(u16::resolve_src(padding, locals, user_app, list_data));
        }
        Config::PaddingTop(padding) => {
            config.padding_top(u16::resolve_src(padding, locals, user_app, list_data));
        }
        Config::PaddingBottom(padding) => {
            config.padding_bottom(u16::resolve_src(padding, locals, user_app, list_data));
        }
        Config::PaddingLeft(padding) => {
            config.padding_left(u16::resolve_src(padding, locals, user_app, list_data));
        }
        Config::PaddingRight(padding) => {
            config.padding_right(u16::resolve_src(padding, locals, user_app, list_data));
        }
        Config::Vertical => {
            config.direction(true);
        }
        Config::ChildGap(gap) => {
            config.child_gap(u16::resolve_src(gap, locals, user_app, list_data));
        }
        Config::ChildAlignmentXLeft => {
            config.align_children_x_left();
        }
        Config::ChildAlignmentXRight => {
            config.align_children_x_right();
        }
        Config::ChildAlignmentXCenter => {
            config.align_children_x_center();
        }
        Config::ChildAlignmentYTop => {
            config.align_children_y_top();
        }
        Config::ChildAlignmentYCenter => {
            config.align_children_y_center();
        }
        Config::ChildAlignmentYBottom => {
            config.align_children_y_bottom();
        }
        Config::Color(color) => {
            let color = Color::resolve_src(color, locals, user_app, list_data);
            config.color(color);
        }

        Config::CustomElement(custom_element) => {
            if let CustomElement::Line(line) = custom_element
                && let Some(source) = line.width_source
                && let Some(width) = user_app.get_numeric(&source, list_data)
            {
                line.width = width;
            }
            config.custom_element(custom_element);
        }
        Config::RadiusAll(radius) => {
            config.radius_all(f32::resolve_src(radius, locals, user_app, list_data));
        }
        Config::RadiusTopLeft(radius) => {
            config.radius_top_left(f32::resolve_src(radius, locals, user_app, list_data));
        }
        Config::RadiusTopRight(radius) => {
            config.radius_top_right(f32::resolve_src(radius, locals, user_app, list_data));
        }
        Config::RadiusBottomRight(radius) => {
            config.radius_bottom_right(f32::resolve_src(radius, locals, user_app, list_data));
        }
        Config::RadiusBottomLeft(radius) => {
            config.radius_bottom_left(f32::resolve_src(radius, locals, user_app, list_data));
        }
        Config::BorderColor(color) => {
            config.border_color(Color::resolve_src(color, locals, user_app, list_data));
        }
        Config::BorderAll(border) => {
            config.border_all(u16::resolve_src(border, locals, user_app, list_data));
        }
        Config::BorderTop(border) => {
            config.border_top(u16::resolve_src(border, locals, user_app, list_data));
        }
        Config::BorderBottom(border) => {
            config.border_bottom(u16::resolve_src(border, locals, user_app, list_data));
        }
        Config::BorderLeft(border) => {
            config.border_left(u16::resolve_src(border, locals, user_app, list_data));
        }
        Config::BorderRight(border) => {
            config.border_right(u16::resolve_src(border, locals, user_app, list_data));
        }
        Config::BorderBetweenChildren(border) => {
            config.border_between_children(u16::resolve_src(border, locals, user_app, list_data));
        }
        Config::Clip {
            vertical,
            horizontal,
        } => {
            config.scroll(
                bool::resolve_src(vertical, locals, user_app, list_data),
                bool::resolve_src(horizontal, locals, user_app, list_data),
                api.l.get_scroll_offset(),
            );
        }
        Config::Image { name } => {
            if let Some(image) = UIImageDescriptor::resolve_name(name, locals, user_app, list_data)
            {
                config.image(image);
            }
        }
        Config::Floating => {
            config.floating();
        }
        Config::FloatingOffset { x, y } => {
            config.floating_offset(
                f32::resolve_src(x, locals, user_app, list_data),
                f32::resolve_src(y, locals, user_app, list_data),
            );
        }
        Config::FloatingDimensions { width, height } => {
            config.floating_dimensions(
                f32::resolve_src(width, locals, user_app, list_data),
                f32::resolve_src(height, locals, user_app, list_data),
            );
        }
        Config::FloatingZIndex { z } => {
            config.floating_z_index(i16::resolve_src(z, locals, user_app, list_data));
        }
        Config::FloatingAttatchToParentAtTopLeft => {
            config.floating_attach_to_parent_at_top_left();
        }
        Config::FloatingAttatchToParentAtCenterLeft => {
            config.floating_attach_to_parent_at_center_left();
        }
        Config::FloatingAttatchToParentAtBottomLeft => {
            config.floating_attach_to_parent_at_bottom_left();
        }
        Config::FloatingAttatchToParentAtTopCenter => {
            config.floating_attach_to_parent_at_top_center();
        }
        Config::FloatingAttatchToParentAtCenter => {
            config.floating_attach_to_parent_at_center();
        }
        Config::FloatingAttatchToParentAtBottomCenter => {
            config.floating_attach_to_parent_at_bottom_center();
        }
        Config::FloatingAttatchToParentAtTopRight => {
            config.floating_attach_to_parent_at_top_right();
        }
        Config::FloatingAttatchToParentAtCenterRight => {
            config.floating_attach_to_parent_at_center_right();
        }
        Config::FloatingAttatchToParentAtBottomRight => {
            config.floating_attach_to_parent_at_bottom_right();
        }
        Config::FloatingAttatchElementAtTopLeft => {
            config.floating_attach_element_at_top_left();
        }
        Config::FloatingAttatchElementAtCenterLeft => {
            config.floating_attach_element_at_center_left();
        }
        Config::FloatingAttatchElementAtBottomLeft => {
            config.floating_attach_element_at_bottom_left();
        }
        Config::FloatingAttatchElementAtTopCenter => {
            config.floating_attach_element_at_top_center();
        }
        Config::FloatingAttatchElementAtCenter => {
            config.floating_attach_element_at_center();
        }
        Config::FloatingAttatchElementAtBottomCenter => {
            config.floating_attach_element_at_bottom_center();
        }
        Config::FloatingAttatchElementAtTopRight => {
            config.floating_attach_element_at_top_right();
        }
        Config::FloatingAttatchElementAtCenterRight => {
            config.floating_attach_element_at_center_right();
        }
        Config::FloatingAttatchElementAtBottomRight => {
            config.floating_attach_element_at_bottom_right();
        }
        Config::FloatingPointerPassThrough => {
            config.floating_pointer_pass_through();
        }
        Config::FloatingAttachElementToElement {
            other_element_id: _,
        } => {
            // TODO: resolve `other_element_id` through the layout engine once
            // it exposes an id lookup; for now floating elements can only
            // attach to their parent or the root.
            config.floating_attach_to_element(0);
        }
        Config::FloatingAttachElementToRoot => {
            config.floating_attach_to_root();
        }
        Config::Use { name } => {
            // A `## name` reusable (as opposed to a `### name` one, which
            // wraps a whole element and goes through `Element::UseOpened`/
            // `UseClosed` in `set_layout`) is just a named bundle of config
            // commands - inline and run each of them against the config
            // that's currently being built.
            //
            // This only handles flat `Config` entries. A reusable config
            // snippet that itself opens a `hover`/`left-clicked`/etc block
            // (an `Element` command, not a `Config` one) can't be replayed
            // from here - doing that needs the `skip`/`nesting_level`
            // bookkeeping `set_layout` owns, which this function doesn't
            // have access to.
            if let Some(reusable) = reusables.get(name) {
                for command in reusable {
                    if let Layout::Config(nested_command) = command {
                        let mut nested_command = nested_command.clone();
                        execute_config(
                            &mut nested_command,
                            config,
                            text_config,
                            reusables,
                            locals,
                            list_data,
                            api,
                            user_app,
                        );
                    }
                }
            }
        }

        Config::AlignCenter => {
            text_config.alignment_center();
        }
        Config::AlignLeft => {
            text_config.alignment_left();
        }
        Config::AlignRight => {
            text_config.alignment_right();
        }
        Config::Editable(_state) => {}
        Config::FontId(id) => {
            text_config.font_id(u16::resolve_src(id, locals, user_app, list_data));
        }
        Config::FontColor(color) => {
            text_config.color(Color::resolve_src(color, locals, user_app, list_data));
        }
        Config::FontSize(size) => {
            text_config.font_size(u16::resolve_src(size, locals, user_app, list_data));
        }
        Config::LineHeight(height) => {
            text_config.line_height(u16::resolve_src(height, locals, user_app, list_data));
        }
    }
}

// ---------------------------------------------------------------------------
// Resolving a DataSrc<T>/GlobalSymbol into a concrete value
// ---------------------------------------------------------------------------

trait ResolveValue<'frame, 'application, UserApp>
where
    'application: 'frame,
    UserApp: LayoutRunnerReflection,
{
    type DeclarationType;
    type ReturnType;
    fn resolve_src(
        var: &'frame DataSrc<Self::DeclarationType>,
        locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration>>>,
        user_app: &'application UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType;
    fn resolve_name(
        var: &GlobalSymbol,
        locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration>>>,
        user_app: &'application UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType;
}

impl<'frame, 'application, UserApp> ResolveValue<'frame, 'application, UserApp>
    for UIImageDescriptor
where
    'application: 'frame,
    UserApp: LayoutRunnerReflection,
{
    type DeclarationType = Option<&'frame UIImageDescriptor>;
    type ReturnType = Option<&'frame UIImageDescriptor>;
    fn resolve_name(
        name: &GlobalSymbol,
        locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration>>>,
        user_app: &'application UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        if let Some(locals) = locals
            && let Some(local) = locals.get(name)
            && let DataSrc::Dynamic(local) = local
            && let Some(value) = user_app.get_image(local, list_data)
        {
            Some(value)
        } else if let Some(value) = user_app.get_image(name, list_data) {
            Some(value)
        } else {
            None
        }
    }
    fn resolve_src(
        _var: &'frame DataSrc<Self::DeclarationType>,
        _locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration>>>,
        _user_app: &'application UserApp,
        _list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        None
    }
}

impl<'frame, 'application, UserApp> ResolveValue<'frame, 'application, UserApp> for Color
where
    'application: 'frame,
    UserApp: LayoutRunnerReflection,
{
    type DeclarationType = Color;
    type ReturnType = Color;
    fn resolve_name(
        name: &GlobalSymbol,
        locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration>>>,
        user_app: &'application UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        if let Some(locals) = locals
            && let Some(local) = locals.get(name)
            && let DataSrc::Dynamic(local) = local
            && let Some(value) = user_app.get_color(local, list_data)
        {
            *value
        } else if let Some(locals) = locals
            && let Some(local) = locals.get(name)
            && let DataSrc::Static(local) = local
            && let Declaration::Color(value) = local
        {
            *value
        } else if let Some(value) = user_app.get_color(name, list_data) {
            *value
        } else {
            Color::default()
        }
    }
    fn resolve_src(
        var: &'frame DataSrc<Self::DeclarationType>,
        locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration>>>,
        user_app: &'application UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        match var {
            DataSrc::Dynamic(name) => Self::resolve_name(name, locals, user_app, list_data),
            DataSrc::Static(value) => *value,
        }
    }
}

impl<'frame, 'application, UserApp> ResolveValue<'frame, 'application, UserApp> for String
where
    'application: 'frame,
    UserApp: LayoutRunnerReflection,
{
    type DeclarationType = String;
    type ReturnType = &'frame str;
    fn resolve_name(
        name: &GlobalSymbol,
        locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration>>>,
        user_app: &'application UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        if let Some(locals) = locals
            && let Some(local) = locals.get(name)
            && let DataSrc::Dynamic(local) = local
            && let Some(value) = user_app.get_text(local, list_data)
        {
            value
        } else if let Some(locals) = locals
            && let Some(local) = locals.get(name)
            && let DataSrc::Static(local) = local
            && let Declaration::Text(value) = local
        {
            value
        } else if let Some(value) = user_app.get_text(name, list_data) {
            value
        } else {
            DEFAULT_TEXT
        }
    }
    fn resolve_src(
        var: &'frame DataSrc<Self::DeclarationType>,
        locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration>>>,
        user_app: &'application UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        match var {
            DataSrc::Dynamic(name) => Self::resolve_name(name, locals, user_app, list_data),
            DataSrc::Static(value) => value,
        }
    }
}

impl<'frame, 'application, UserApp> ResolveValue<'frame, 'application, UserApp> for f32
where
    'application: 'frame,
    UserApp: LayoutRunnerReflection,
{
    type DeclarationType = f32;
    type ReturnType = f32;
    fn resolve_src(
        var: &DataSrc<Self::DeclarationType>,
        locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
        user_app: &UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        match var {
            DataSrc::Dynamic(name) => Self::resolve_name(name, locals, user_app, list_data),
            DataSrc::Static(value) => *value,
        }
    }
    fn resolve_name(
        name: &GlobalSymbol,
        locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
        user_app: &UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        if let Some(locals) = locals
            && let Some(local) = locals.get(name)
            && let DataSrc::Dynamic(local) = local
            && let Some(value) = user_app.get_numeric(local, list_data)
        {
            value
        } else if let Some(locals) = locals
            && let Some(local) = locals.get(name)
            && let DataSrc::Static(local) = local
            && let Declaration::Numeric(value) = local
        {
            *value
        } else {
            user_app.get_numeric(name, list_data).unwrap_or(0.0)
        }
    }
}

impl<'frame, 'application, UserApp> ResolveValue<'frame, 'application, UserApp> for u16
where
    'application: 'frame,
    UserApp: LayoutRunnerReflection,
{
    type DeclarationType = u16;
    type ReturnType = u16;
    fn resolve_src(
        var: &DataSrc<Self::DeclarationType>,
        locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
        user_app: &UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        match var {
            DataSrc::Dynamic(name) => Self::resolve_name(name, locals, user_app, list_data),
            DataSrc::Static(value) => *value,
        }
    }
    fn resolve_name(
        name: &GlobalSymbol,
        locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
        user_app: &UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        if let Some(locals) = locals
            && let Some(local) = locals.get(name)
            && let DataSrc::Dynamic(local) = local
            && let Some(value) = user_app.get_numeric(local, list_data)
        {
            value as u16
        } else if let Some(locals) = locals
            && let Some(local) = locals.get(name)
            && let DataSrc::Static(local) = local
            && let Declaration::Numeric(value) = local
        {
            *value as u16
        } else if let Some(value) = user_app.get_numeric(name, list_data) {
            value as u16
        } else {
            0
        }
    }
}

impl<'frame, 'application, UserApp> ResolveValue<'frame, 'application, UserApp> for i16
where
    'application: 'frame,
    UserApp: LayoutRunnerReflection,
{
    type DeclarationType = i16;
    type ReturnType = i16;
    fn resolve_src(
        var: &DataSrc<Self::DeclarationType>,
        locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
        user_app: &UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        match var {
            DataSrc::Dynamic(name) => Self::resolve_name(name, locals, user_app, list_data),
            DataSrc::Static(value) => *value,
        }
    }
    fn resolve_name(
        name: &GlobalSymbol,
        locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
        user_app: &UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        if let Some(locals) = locals
            && let Some(local) = locals.get(name)
            && let DataSrc::Dynamic(local) = local
            && let Some(value) = user_app.get_numeric(local, list_data)
        {
            value as i16
        } else if let Some(locals) = locals
            && let Some(local) = locals.get(name)
            && let DataSrc::Static(local) = local
            && let Declaration::Numeric(value) = local
        {
            *value as i16
        } else if let Some(value) = user_app.get_numeric(name, list_data) {
            value as i16
        } else {
            0
        }
    }
}

impl<'frame, 'application, UserApp> ResolveValue<'frame, 'application, UserApp> for bool
where
    'application: 'frame,
    UserApp: LayoutRunnerReflection,
{
    type DeclarationType = bool;
    type ReturnType = bool;
    fn resolve_src(
        var: &DataSrc<Self::DeclarationType>,
        locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
        user_app: &UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        match var {
            DataSrc::Dynamic(name) => Self::resolve_name(name, locals, user_app, list_data),
            DataSrc::Static(value) => *value,
        }
    }
    fn resolve_name(
        name: &GlobalSymbol,
        locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
        user_app: &UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        if let Some(locals) = locals
            && let Some(local) = locals.get(name)
            && let DataSrc::Dynamic(local) = local
            && let Some(value) = user_app.get_bool(local, list_data)
        {
            value
        } else if let Some(locals) = locals
            && let Some(local) = locals.get(name)
            && let DataSrc::Static(local) = local
            && let Declaration::Bool(value) = local
        {
            *value
        } else {
            user_app.get_bool(name, list_data).unwrap_or_default()
        }
    }
}

impl<'frame, 'application, UserApp> ResolveValue<'frame, 'application, UserApp> for GlobalSymbol
where
    'application: 'frame,
    UserApp: LayoutRunnerReflection,
{
    type DeclarationType = GlobalSymbol;
    type ReturnType = GlobalSymbol;
    fn resolve_src(
        var: &DataSrc<Self::DeclarationType>,
        locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
        user_app: &UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        match var {
            DataSrc::Dynamic(name) => Self::resolve_name(name, locals, user_app, list_data),
            DataSrc::Static(value) => value.clone(),
        }
    }
    fn resolve_name(
        name: &GlobalSymbol,
        locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
        user_app: &UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> Self::ReturnType {
        if let Some(locals) = locals
            && let Some(local) = locals.get(name)
            && let DataSrc::Dynamic(local) = local
            && let Some(value) = user_app.get_event(local, list_data)
        {
            value
        } else if let Some(locals) = locals
            && let Some(local) = locals.get(name)
            && let DataSrc::Static(local) = local
            && let Declaration::Event(value) = local
        {
            value.clone()
        } else {
            user_app
                .get_event(name, list_data)
                .unwrap_or_else(|| GlobalSymbol::new(""))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parser should turn `src/layouts/Main.md` into a non-empty,
    /// flattened command stream without panicking or erroring.
    #[test]
    fn parses_main_md() {
        let file =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/layouts/Main.md"))
                .unwrap();
        let (body, reusables) = process_layout(file).expect("Main.md should parse");

        assert!(!body.is_empty());
        assert!(reusables.contains_key("layout expand"));
        assert!(reusables.contains_key("header button"));
        assert!(reusables.contains_key("drop down menu item"));

        // The command stream must be well-nested: every *Opened has a matching
        // *Closed and the stream never dips below zero depth.
        let mut depth: i32 = 0;
        for command in &body {
            if let Layout::Element(element) = command {
                let name = format!("{element}");
                if name.ends_with("Opened") {
                    depth += 1;
                } else if name.ends_with("Closed") {
                    depth -= 1;
                }
            }
            assert!(depth >= 0, "command stream closed more than it opened");
        }
        assert_eq!(depth, 0, "command stream left something unclosed");
    }
}
