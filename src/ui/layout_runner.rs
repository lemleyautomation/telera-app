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
//! Event handling used to fight the borrow checker because mutable
//! element/text config scratch space was passed around as `Option<&mut T>`
//! with a "make a fresh default if None" fallback. That pattern doesn't
//! work: the fallback default is a temporary that can't outlive the match
//! arm that creates it, and reborrowing an already-`&mut` local as `&mut
//! local` (instead of just passing `local`) produces a double reference.
//! Both mistakes showed up throughout the old code.
//!
//! The fix used here is to never make those parameters optional: `set_layout`
//! always receives a live `&mut ElementConfiguration` and `&mut TextConfig`,
//! and callers that don't have an existing one to hand down (list items, the
//! top level call) simply own a fresh local and pass a reborrow of it.
//! (Text measurement now lives inside `api.l` itself - the layout engine
//! owns the renderer for the duration of a `begin_layout`/`end_layout` pass -
//! so it is no longer threaded through here.)
//!
//! `set_layout` also holds a live `&mut UserApp` (bounded `LayoutReflector`)
//! and a `&mut API` as separate parameters, so when an event fires it calls
//! [`LayoutReflector::dispatch_event`] right then, in tree order - the same
//! way `fn *name*` elements already go straight to `dispatch_custom_element`.
//! A handler therefore sees state changes made by handlers earlier in the
//! same frame's tree.
use std::{collections::HashMap, fmt::Debug, str::FromStr};

use markdown::mdast::{List, ListItem, Node, Paragraph};
use symbol_table::GlobalSymbol;
use telera_layout::{Color, ElementConfiguration, TextConfig};

use super::calc;
use crate::{API, CustomElement, EffectKind, ResolvedShader, UIImageDescriptor};

const DEFAULT_TEXT: &str = ":(";

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Extra, free-form data carried alongside a dispatched event.
///
/// `code`/`code2` are a couple of general-purpose numeric slots; the layout
/// runner fills `code` with the current list index when an event fires from
/// inside a `list`, so a handler can tell which item was interacted with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EventContext {
    pub text: Option<String>,
    pub code: Option<u32>,
    pub code2: Option<u32>,
    pub list_index: Option<usize>,
}

/// A deterministic id for an un-named `` `floating` `` element that uses an
/// event gate, stable frame to frame for a static tree. The `list`/`item`
/// iteration key (if any) is folded in so the same synthetic slot in two list
/// rows doesn't collide. See the `hovered!` macro in [`set_layout`].
fn synthetic_gate_id_name(list_data: &Option<(GlobalSymbol, usize)>, counter: u32) -> String {
    match list_data {
        Some((list, index)) => format!("__telera_gate::{}#{index}::{counter}", list.as_str()),
        None => format!("__telera_gate::{counter}"),
    }
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

    fn dispatch_custom_element(&mut self, name: &GlobalSymbol, api: &mut API) {}
}

// ---------------------------------------------------------------------------
// Layout command types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Layout {
    Element(Element),
    Declaration {
        name: GlobalSymbol,
        value: DataSrc<Declaration>,
    },
    Config(Config),
}

#[derive(Clone, Debug, PartialEq)]
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

    // `fn *name*` - calls back into the app's own
    // `LayoutReflector::dispatch_custom_element` with `name` right where it
    // sits in the tree, so the dispatched function can add whatever it wants
    // via `api.l` at that spot. A single leaf command, unlike most other
    // elements here, since there's no body of its own to open/close.
    FunctionCall(GlobalSymbol),

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

    /// Gates on "this window received keyboard input this frame" and fires its
    /// handler once when that becomes true; the handler reads the events via
    /// `API::key_events`.
    KeyEventOpened {
        event: Option<DataSrc<GlobalSymbol>>,
    },
    KeyEventClosed,

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

    /// Opens the zoom-bake scope for a `` `canvas` `` (`set_layout` pushes the
    /// current `canvas_scale` and replaces it with `api.canvas_zoom(name)`).
    /// Nesting-neutral (does not move `nesting_level`); always emitted paired
    /// with [`Element::CanvasWorldClosed`].
    CanvasWorldOpened {
        name: GlobalSymbol,
    },
    CanvasWorldClosed,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Config {
    Id(DataSrc<String>),
    /// `id_indexed` - like [`Config::Id`] but folds the current `list`/`item`
    /// iteration index into the hash, so each row of a list gets a distinct id.
    IdIndexed(DataSrc<String>),

    GrowAll,
    FitAll,
    AspectRatio(DataSrc<f32>),
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
    FixedSquare(DataSrc<f32>),
    PercentX(DataSrc<f32>),
    PercentY(DataSrc<f32>),

    PaddingAll(DataSrc<u16>),
    PaddingTop(DataSrc<u16>),
    PaddingBottom(DataSrc<u16>),
    PaddingLeft(DataSrc<u16>),
    PaddingRight(DataSrc<u16>),

    ChildGap(DataSrc<u16>),

    Vertical,
    Horizontal,

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

    /// `` `image` *name* `` - resolve `name` to a [`UIImageDescriptor`], checking
    /// `set-image` declarations first, then the application's `get_image`.
    Image {
        name: GlobalSymbol,
    },
    /// `` `image` *atlas* [u1, v1, u2, v2] `` - a descriptor written inline, no
    /// lookup. The atlas name is whatever a `load` directive (or the app) staged
    /// it under.
    ImageLiteral(UIImageDescriptor),

    Floating,
    FloatingClipToParent,
    FloatingNoClip,
    FloatingPointerCapture,
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

    CustomElement(CustomElementSpec),

    /// One or more `` `shader` *name* `` per-element visual effects (drop
    /// shadow, raised edge, inner glow, blur, custom loaded shaders), in written
    /// order. Resolved to `ResolvedShader`s and carried on the element's clay
    /// `userData`.
    Shaders(Vec<ShaderSpec>),

    Use {
        name: GlobalSymbol,
        /// `set-*` / `get-*` bindings from the `` `use` `` body, applied as
        /// locals while the `## name` snippet's configs replay - so a config
        /// snippet takes parameters the same way a `### name` element snippet
        /// does. Empty when the `use` was written without a body.
        params: Vec<(GlobalSymbol, DataSrc<Declaration>)>,
    },

    FontId(DataSrc<u16>),
    AlignRight,
    AlignLeft,
    AlignCenter,
    LineHeight(DataSrc<u16>),
    FontSize(DataSrc<u16>),
    FontColor(DataSrc<Color>),
    LetterSpacing(DataSrc<u16>),
    WrapWords,
    WrapNewLines,
    WrapNone,

    /// The `` `canvas` `` element's state config, emitted once on the outer clip
    /// container. Seeds / updates the per-name [`Canvas`](crate::Canvas):
    /// `world_*` and the zoom limits are static bounds re-applied every frame;
    /// `initial_*` apply only the first time this canvas is laid out. Also
    /// applies the clip and the pan `childOffset` every frame.
    Canvas {
        name: GlobalSymbol,
        world_width: Option<DataSrc<f32>>,
        world_height: Option<DataSrc<f32>>,
        min_zoom: Option<DataSrc<f32>>,
        max_zoom: Option<DataSrc<f32>>,
        initial_zoom: Option<DataSrc<f32>>,
        initial_pan_x: Option<DataSrc<f32>>,
        initial_pan_y: Option<DataSrc<f32>>,
    },
    /// Sizes the `` `canvas` `` world wrapper: `world_size * canvas_scale`, with
    /// `world_size` falling back to the canvas element's own laid-out size (else
    /// [`Canvas::AUTO_WORLD`](crate::Canvas::AUTO_WORLD)).
    CanvasWorldSize {
        name: GlobalSymbol,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Declaration {
    Bool(bool),
    Numeric(f32),
    Text(String),
    Color(Color),
    Event(GlobalSymbol),
    Image(UIImageDescriptor),
}

impl Default for Declaration {
    fn default() -> Self {
        Declaration::Bool(false)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DataSrc<T> {
    Static(T),
    Dynamic(GlobalSymbol),
}

impl<T: Default> Default for DataSrc<T> {
    fn default() -> Self {
        DataSrc::Static(T::default())
    }
}

/// The parser-side, unresolved twin of [`CustomElement`]: every geometry
/// parameter is a [`DataSrc<f32>`] so it can be a literal or a `get-numeric`
/// binding. [`execute_config`] resolves one of these into a plain
/// [`CustomElement`] each frame (see [`CustomElementSpec::resolve`]).
///
/// The `Default` of each variant is the shape a bare keyword (`` `line` ``,
/// `` `arc` ``, ...) draws with no config of its own.
#[derive(Clone, Debug, PartialEq)]
pub enum CustomElementSpec {
    Circle,
    Ring {
        thickness: DataSrc<f32>,
    },
    Line {
        from_x: DataSrc<f32>,
        from_y: DataSrc<f32>,
        to_x: DataSrc<f32>,
        to_y: DataSrc<f32>,
        thickness: DataSrc<f32>,
    },
    Arc {
        center_x: DataSrc<f32>,
        center_y: DataSrc<f32>,
        radius: DataSrc<f32>,
        start_angle: DataSrc<f32>,
        end_angle: DataSrc<f32>,
        thickness: DataSrc<f32>,
    },
    Bezier {
        from_x: DataSrc<f32>,
        from_y: DataSrc<f32>,
        ctrl1_x: DataSrc<f32>,
        ctrl1_y: DataSrc<f32>,
        ctrl2_x: DataSrc<f32>,
        ctrl2_y: DataSrc<f32>,
        to_x: DataSrc<f32>,
        to_y: DataSrc<f32>,
        thickness: DataSrc<f32>,
    },
    /// A `render-window` element: a hole for the 3D scene, drawn through the
    /// camera named `camera` (the element's own name; un-named -> `default`).
    /// Each `Some` override positions that camera from the layout this frame.
    RenderWindow {
        camera: GlobalSymbol,
        eye_x: Option<DataSrc<f32>>,
        eye_y: Option<DataSrc<f32>>,
        eye_z: Option<DataSrc<f32>>,
        target_x: Option<DataSrc<f32>>,
        target_y: Option<DataSrc<f32>>,
        target_z: Option<DataSrc<f32>>,
        up_x: Option<DataSrc<f32>>,
        up_y: Option<DataSrc<f32>>,
        up_z: Option<DataSrc<f32>>,
        fov: Option<DataSrc<f32>>,
        near: Option<DataSrc<f32>>,
        far: Option<DataSrc<f32>>,
        ortho_height: Option<DataSrc<f32>>,
    },
}

/// A numeric literal, for building a [`CustomElementSpec`] default.
const fn s(v: f32) -> DataSrc<f32> {
    DataSrc::Static(v)
}

impl CustomElementSpec {
    /// The bare-keyword `` `ring` `` shape: a 1px inscribed outline.
    pub fn ring() -> Self {
        CustomElementSpec::Ring { thickness: s(1.0) }
    }
    /// The bare-keyword `` `line` `` shape: a 1px vertical line down the centre
    /// of the bounding box (matches the pre-shape-expansion behaviour).
    pub fn line() -> Self {
        CustomElementSpec::Line {
            from_x: s(0.5),
            from_y: s(0.0),
            to_x: s(0.5),
            to_y: s(1.0),
            thickness: s(1.0),
        }
    }
    /// The bare-keyword `` `arc` `` shape: a 1px half-circle, inscribed, opening
    /// downward.
    pub fn arc() -> Self {
        CustomElementSpec::Arc {
            center_x: s(0.5),
            center_y: s(0.5),
            radius: s(1.0),
            start_angle: s(0.0),
            end_angle: s(180.0),
            thickness: s(1.0),
        }
    }
    /// The bare-keyword `` `render-window` `` element: a hole for the scene,
    /// drawn through the `default` camera with no layout-set overrides.
    /// `process_shape` swaps in the element's name as the camera.
    pub fn render_window() -> Self {
        CustomElementSpec::RenderWindow {
            camera: GlobalSymbol::new("default"),
            eye_x: None,
            eye_y: None,
            eye_z: None,
            target_x: None,
            target_y: None,
            target_z: None,
            up_x: None,
            up_y: None,
            up_z: None,
            fov: None,
            near: None,
            far: None,
            ortho_height: None,
        }
    }

    /// The bare-keyword `` `bezier` `` shape: a 1px symmetric arch from the
    /// bottom-left to the bottom-right corner.
    pub fn bezier() -> Self {
        CustomElementSpec::Bezier {
            from_x: s(0.0),
            from_y: s(1.0),
            ctrl1_x: s(0.33),
            ctrl1_y: s(0.0),
            ctrl2_x: s(0.66),
            ctrl2_y: s(0.0),
            to_x: s(1.0),
            to_y: s(1.0),
            thickness: s(1.0),
        }
    }

    /// Resolves every `DataSrc<f32>` parameter against the app / local
    /// declarations, producing the plain [`CustomElement`] the renderer draws.
    pub fn resolve<UserApp>(
        &self,
        locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
        user_app: &UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> CustomElement
    where
        UserApp: LayoutRunnerReflection,
    {
        let r = |src: &DataSrc<f32>| f32::resolve_src(src, locals, user_app, list_data);
        let ro = |src: &Option<DataSrc<f32>>| src.as_ref().map(&r);
        match self {
            CustomElementSpec::Circle => CustomElement::Circle,
            CustomElementSpec::RenderWindow {
                camera,
                eye_x,
                eye_y,
                eye_z,
                target_x,
                target_y,
                target_z,
                up_x,
                up_y,
                up_z,
                fov,
                near,
                far,
                ortho_height,
            } => CustomElement::RenderWindow {
                camera: *camera,
                eye_x: ro(eye_x),
                eye_y: ro(eye_y),
                eye_z: ro(eye_z),
                target_x: ro(target_x),
                target_y: ro(target_y),
                target_z: ro(target_z),
                up_x: ro(up_x),
                up_y: ro(up_y),
                up_z: ro(up_z),
                fov: ro(fov),
                near: ro(near),
                far: ro(far),
                ortho_height: ro(ortho_height),
            },
            CustomElementSpec::Ring { thickness } => CustomElement::Ring {
                thickness: r(thickness),
            },
            CustomElementSpec::Line {
                from_x,
                from_y,
                to_x,
                to_y,
                thickness,
            } => CustomElement::Line {
                from_x: r(from_x),
                from_y: r(from_y),
                to_x: r(to_x),
                to_y: r(to_y),
                thickness: r(thickness),
            },
            CustomElementSpec::Arc {
                center_x,
                center_y,
                radius,
                start_angle,
                end_angle,
                thickness,
            } => CustomElement::Arc {
                center_x: r(center_x),
                center_y: r(center_y),
                radius: r(radius),
                start_angle: r(start_angle),
                end_angle: r(end_angle),
                thickness: r(thickness),
            },
            CustomElementSpec::Bezier {
                from_x,
                from_y,
                ctrl1_x,
                ctrl1_y,
                ctrl2_x,
                ctrl2_y,
                to_x,
                to_y,
                thickness,
            } => CustomElement::Bezier {
                from_x: r(from_x),
                from_y: r(from_y),
                ctrl1_x: r(ctrl1_x),
                ctrl1_y: r(ctrl1_y),
                ctrl2_x: r(ctrl2_x),
                ctrl2_y: r(ctrl2_y),
                to_x: r(to_x),
                to_y: r(to_y),
                thickness: r(thickness),
            },
        }
    }

    /// Applies a single shape-config keyword (`` `from-x` ``, `` `radius` ``,
    /// `` `thickness` ``, the `` `from` ``/`` `to` ``/`` `center` `` anchor
    /// words, the `render-window` camera keywords, ...) to this spec. Keywords
    /// that don't apply to the current variant are ignored.
    fn apply_param(&mut self, key: &str, config: &Paragraph) {
        // `render-window` camera keywords each set one `Option<DataSrc<f32>>`
        // override.
        if let CustomElementSpec::RenderWindow {
            eye_x,
            eye_y,
            eye_z,
            target_x,
            target_y,
            target_z,
            up_x,
            up_y,
            up_z,
            fov,
            near,
            far,
            ortho_height,
            ..
        } = self
        {
            let slot: Option<&mut Option<DataSrc<f32>>> = match key {
                "eye-x" => Some(eye_x),
                "eye-y" => Some(eye_y),
                "eye-z" => Some(eye_z),
                "target-x" => Some(target_x),
                "target-y" => Some(target_y),
                "target-z" => Some(target_z),
                "up-x" => Some(up_x),
                "up-y" => Some(up_y),
                "up-z" => Some(up_z),
                "fov" => Some(fov),
                "near" => Some(near),
                "far" => Some(far),
                "ortho-height" => Some(ortho_height),
                _ => None,
            };
            if let Some(slot) = slot {
                *slot = optional_arg::<f32>(config);
            }
            return;
        }

        // `from`/`to`/`center` take an anchor word and set an (x, y) pair.
        if let Some(anchor) = anchor_fraction_arg(config) {
            let pair: Option<(&mut DataSrc<f32>, &mut DataSrc<f32>)> = match (key, &mut *self) {
                ("from", CustomElementSpec::Line { from_x, from_y, .. })
                | ("from", CustomElementSpec::Bezier { from_x, from_y, .. }) => {
                    Some((from_x, from_y))
                }
                ("to", CustomElementSpec::Line { to_x, to_y, .. })
                | ("to", CustomElementSpec::Bezier { to_x, to_y, .. }) => Some((to_x, to_y)),
                ("center", CustomElementSpec::Arc {
                    center_x, center_y, ..
                }) => Some((center_x, center_y)),
                _ => None,
            };
            if let Some((x, y)) = pair {
                *x = s(anchor.0);
                *y = s(anchor.1);
            }
            return;
        }

        let Some(value) = optional_arg::<f32>(config) else {
            return;
        };
        let field: Option<&mut DataSrc<f32>> = match (key, &mut *self) {
            // `thickness` (and its `width` back-compat alias) applies to every
            // stroked shape.
            ("thickness" | "width", CustomElementSpec::Line { thickness, .. })
            | ("thickness" | "width", CustomElementSpec::Ring { thickness, .. })
            | ("thickness" | "width", CustomElementSpec::Arc { thickness, .. })
            | ("thickness" | "width", CustomElementSpec::Bezier { thickness, .. }) => Some(thickness),

            ("from-x", CustomElementSpec::Line { from_x, .. })
            | ("from-x", CustomElementSpec::Bezier { from_x, .. }) => Some(from_x),
            ("from-y", CustomElementSpec::Line { from_y, .. })
            | ("from-y", CustomElementSpec::Bezier { from_y, .. }) => Some(from_y),
            ("to-x", CustomElementSpec::Line { to_x, .. })
            | ("to-x", CustomElementSpec::Bezier { to_x, .. }) => Some(to_x),
            ("to-y", CustomElementSpec::Line { to_y, .. })
            | ("to-y", CustomElementSpec::Bezier { to_y, .. }) => Some(to_y),

            ("ctrl1-x", CustomElementSpec::Bezier { ctrl1_x, .. }) => Some(ctrl1_x),
            ("ctrl1-y", CustomElementSpec::Bezier { ctrl1_y, .. }) => Some(ctrl1_y),
            ("ctrl2-x", CustomElementSpec::Bezier { ctrl2_x, .. }) => Some(ctrl2_x),
            ("ctrl2-y", CustomElementSpec::Bezier { ctrl2_y, .. }) => Some(ctrl2_y),

            ("center-x", CustomElementSpec::Arc { center_x, .. }) => Some(center_x),
            ("center-y", CustomElementSpec::Arc { center_y, .. }) => Some(center_y),
            ("radius", CustomElementSpec::Arc { radius, .. }) => Some(radius),
            ("start-angle", CustomElementSpec::Arc { start_angle, .. }) => Some(start_angle),
            ("end-angle", CustomElementSpec::Arc { end_angle, .. }) => Some(end_angle),
            _ => None,
        };
        if let Some(field) = field {
            *field = value;
        }
    }
}

/// Every config keyword handled by [`CustomElementSpec::apply_param`] - the
/// shape-only keywords `process_configs` forwards to the current custom element
/// instead of turning into a [`Config`].
const SHAPE_PARAM_KEYWORDS: &[&str] = &[
    "center", "center-x", "center-y", "ctrl1-x", "ctrl1-y", "ctrl2-x", "ctrl2-y", "end-angle",
    "eye-x", "eye-y", "eye-z", "far", "fov", "from", "from-x", "from-y", "near", "ortho-height",
    "radius", "start-angle", "target-x", "target-y", "target-z", "thickness", "to", "to-x", "to-y",
    "up-x", "up-y", "up-z", "width",
];

/// Every keyword that tunes a `` `shader` `` effect - the built-in named
/// parameters plus the positional `` `shader-param-1` `` .. `` `shader-param-8` ``
/// a custom shader reads. `process_configs` forwards these to the current
/// [`ShaderSpec`] instead of turning them into a [`Config`]. **Sorted** (a
/// prefix of [`LEADING_KEYWORDS`]).
const SHADER_PARAM_KEYWORDS: &[&str] = &[
    "bevel-highlight", "bevel-light-angle", "bevel-shade", "bevel-width", "blur-radius", "blur-tint",
    "glow-blur", "glow-color", "glow-spread", "shader-param-1", "shader-param-2", "shader-param-3",
    "shader-param-4", "shader-param-5", "shader-param-6", "shader-param-7", "shader-param-8",
    "shadow-blur", "shadow-color", "shadow-offset-x", "shadow-offset-y", "shadow-spread",
];

/// The parser-side, unresolved twin of [`ResolvedShader`]: an element's
/// `` `shader` `` config. `name` selects the effect (a built-in alias or a
/// custom-shader directive name); the rest are tunables, each a literal or a
/// `get-numeric` / `get-color` binding. [`execute_config`] resolves one of
/// these into a [`ResolvedShader`] each frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderSpec {
    pub name: GlobalSymbol,
    pub shadow_offset_x: DataSrc<f32>,
    pub shadow_offset_y: DataSrc<f32>,
    pub shadow_blur: DataSrc<f32>,
    pub shadow_spread: DataSrc<f32>,
    pub shadow_color: DataSrc<Color>,
    pub bevel_width: DataSrc<f32>,
    pub bevel_light_angle: DataSrc<f32>,
    pub bevel_highlight: DataSrc<f32>,
    pub bevel_shade: DataSrc<f32>,
    pub glow_blur: DataSrc<f32>,
    pub glow_spread: DataSrc<f32>,
    pub glow_color: DataSrc<Color>,
    pub blur_radius: DataSrc<f32>,
    pub blur_tint: DataSrc<Color>,
    pub custom: [DataSrc<f32>; 8],
}

impl ShaderSpec {
    /// The effect with sensible defaults, before any parameter keyword.
    pub fn with_name(name: GlobalSymbol) -> Self {
        let black = Color { r: 0.0, g: 0.0, b: 0.0, a: 102.0 };
        let white = Color { r: 255.0, g: 255.0, b: 255.0, a: 120.0 };
        ShaderSpec {
            name,
            shadow_offset_x: s(0.0),
            shadow_offset_y: s(2.0),
            shadow_blur: s(8.0),
            shadow_spread: s(0.0),
            shadow_color: DataSrc::Static(black),
            bevel_width: s(6.0),
            bevel_light_angle: s(225.0),
            bevel_highlight: s(0.9),
            bevel_shade: s(0.6),
            glow_blur: s(8.0),
            glow_spread: s(0.0),
            glow_color: DataSrc::Static(white),
            blur_radius: s(8.0),
            blur_tint: DataSrc::Static(Color { r: 255.0, g: 255.0, b: 255.0, a: 0.0 }),
            custom: [s(0.0), s(0.0), s(0.0), s(0.0), s(0.0), s(0.0), s(0.0), s(0.0)],
        }
    }

    /// The [`EffectKind`] this spec's `name` selects. Anything that isn't a
    /// built-in alias is a custom shader.
    fn kind(&self) -> EffectKind {
        match self.name.as_str() {
            "drop_shadow" | "shadow" => EffectKind::DropShadow,
            "raised_edge" | "bevel" => EffectKind::RaisedEdge,
            "inner_glow" | "glow" => EffectKind::InnerGlow,
            "blur" => EffectKind::Blur,
            _ => EffectKind::Custom,
        }
    }

    fn apply_param(&mut self, key: &str, config: &Paragraph) {
        if let Some(idx) = key
            .strip_prefix("shader-param-")
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|n| (1..=8).contains(n))
        {
            if let Some(v) = optional_arg::<f32>(config) {
                self.custom[idx - 1] = v;
            }
            return;
        }
        let num: Option<&mut DataSrc<f32>> = match key {
            "shadow-offset-x" => Some(&mut self.shadow_offset_x),
            "shadow-offset-y" => Some(&mut self.shadow_offset_y),
            "shadow-blur" => Some(&mut self.shadow_blur),
            "shadow-spread" => Some(&mut self.shadow_spread),
            "bevel-width" => Some(&mut self.bevel_width),
            "bevel-light-angle" => Some(&mut self.bevel_light_angle),
            "bevel-highlight" => Some(&mut self.bevel_highlight),
            "bevel-shade" => Some(&mut self.bevel_shade),
            "glow-blur" => Some(&mut self.glow_blur),
            "glow-spread" => Some(&mut self.glow_spread),
            "blur-radius" => Some(&mut self.blur_radius),
            _ => None,
        };
        if let Some(slot) = num {
            if let Some(v) = optional_arg::<f32>(config) {
                *slot = v;
            }
            return;
        }
        let col: Option<&mut DataSrc<Color>> = match key {
            "shadow-color" => Some(&mut self.shadow_color),
            "glow-color" => Some(&mut self.glow_color),
            "blur-tint" => Some(&mut self.blur_tint),
            _ => None,
        };
        if let Some(slot) = col
            && let Some(v) = optional_arg::<Color>(config)
        {
            *slot = v;
        }
    }

    /// Resolves every binding for this frame into the plain [`ResolvedShader`]
    /// the renderer reads. `params` / `params2` are packed per [`EffectKind`];
    /// for a custom shader they are `shader-param-1..8` verbatim.
    pub fn resolve<UserApp>(
        &self,
        locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
        user_app: &UserApp,
        list_data: &Option<(GlobalSymbol, usize)>,
    ) -> ResolvedShader
    where
        UserApp: LayoutRunnerReflection,
    {
        let r = |src: &DataSrc<f32>| f32::resolve_src(src, locals, user_app, list_data);
        let rc = |src: &DataSrc<Color>| {
            let c = Color::resolve_src(src, locals, user_app, list_data);
            [c.r / 255.0, c.g / 255.0, c.b / 255.0, c.a / 255.0]
        };
        let kind = self.kind();
        let (params, params2) = match kind {
            EffectKind::DropShadow => (
                [
                    r(&self.shadow_offset_x),
                    r(&self.shadow_offset_y),
                    r(&self.shadow_blur),
                    r(&self.shadow_spread),
                ],
                rc(&self.shadow_color),
            ),
            EffectKind::RaisedEdge => (
                [
                    r(&self.bevel_width),
                    r(&self.bevel_light_angle).to_radians(),
                    0.0,
                    0.0,
                ],
                [r(&self.bevel_highlight), r(&self.bevel_shade), 0.0, 0.0],
            ),
            EffectKind::InnerGlow => (
                [0.0, 0.0, r(&self.glow_blur), r(&self.glow_spread)],
                rc(&self.glow_color),
            ),
            EffectKind::Blur => ([r(&self.blur_radius), 0.0, 0.0, 0.0], rc(&self.blur_tint)),
            EffectKind::Custom => (
                [
                    r(&self.custom[0]),
                    r(&self.custom[1]),
                    r(&self.custom[2]),
                    r(&self.custom[3]),
                ],
                [
                    r(&self.custom[4]),
                    r(&self.custom[5]),
                    r(&self.custom[6]),
                    r(&self.custom[7]),
                ],
            ),
        };
        ResolvedShader {
            kind,
            custom: if kind == EffectKind::Custom {
                Some(self.name)
            } else {
                None
            },
            params,
            params2,
        }
    }
}

/// Reads a `` `from` ``/`` `to` ``/`` `center` `` anchor argument - one of the
/// nine words `attatch-parent` accepts - into a normalised `(x, y)` fraction of
/// the bounding box. `None` if the argument isn't one of those words.
fn anchor_fraction_arg(config: &Paragraph) -> Option<(f32, f32)> {
    let word = match config.children.get(1) {
        Some(Node::Text(text)) => text.value.trim(),
        _ => return None,
    };
    if word == "center" {
        return Some((0.5, 0.5));
    }
    let (vy, vx) = word.split_once('-')?;
    let x = match vx {
        "left" => 0.0,
        "center" => 0.5,
        "right" => 1.0,
        _ => return None,
    };
    let y = match vy {
        "top" => 0.0,
        "center" => 0.5,
        "bottom" => 1.0,
        _ => return None,
    };
    Some((x, y))
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
    fn field_image(&self, name: &GlobalSymbol) -> Option<&UIImageDescriptor> {
        None
    }
}

/// Interns a markdown lookup name verbatim (only surrounding whitespace is
/// trimmed). The name must match its target - a Rust struct field, a
/// `set-*` / `get-*` declaration, a `list` / `item` / `if` name - **exactly**,
/// character for character: no case folding, no space/hyphen/underscore
/// equivalence. Used for every symbol later resolved against the app's fields
/// or declarations: `*dynamic*` values, `list` / `item` / `if` names,
/// `get-*` / `set-*` targets. Element, reusable-snippet, atlas and
/// event-handler names go through `GlobalSymbol::new` directly, same rules.
fn field_symbol(raw: &str) -> GlobalSymbol {
    GlobalSymbol::new(raw.trim())
}

// ---------------------------------------------------------------------------
// Markdown parser: turns a document into a flat Vec<Layout>
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum ParsingMode {
    None,
    /// The `#### TML <version>` preamble block: a flat list of directives
    /// (`load` today) that run once when the file is parsed / a page replaced,
    /// before any layout.
    Header,
    Body,
    ReusableElements,
    ReusableConfig,
}

/// One image a layout file asked to be loaded, via a `` - `load` [atlas](path) ``
/// directive in its `#### TML ...` header block. The parser only records the
/// request; `API` reads the file and hands the pixels to the renderer (see
/// [`Binder::load_layout`] / `API::load_layout_file`), so the layout can use
/// `atlas` from an `image` config or a `set-image` declaration without the
/// application staging it in Rust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageLoad {
    /// Atlas name the image is registered under - what `image`/`set-image`
    /// reference.
    pub atlas: String,
    /// Path to the image file, resolved relative to the process's working
    /// directory (the same base the layout-watch directory is given relative
    /// to).
    pub path: String,
}

/// One font a layout file asked to be loaded, via a `` - `font` [id](path) ``
/// directive in its `#### TML ...` header. Same lifecycle as [`ImageLoad`]:
/// the parser only records the request; `API` reads the file and hands the
/// bytes to the renderer ([`API::load_font`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontLoad {
    /// The `` `font-id` `` this face is selected by. It is also added to the
    /// global fallback chain (ahead of the platform defaults), so a loaded
    /// emoji/symbol font is picked up automatically without setting `font-id`.
    pub id: u16,
    /// Path to the `.ttf` / `.otf` file, resolved like [`ImageLoad::path`].
    pub path: String,
}

/// One custom UI shader a layout file asked to be loaded, via a
/// `` - `shader` [name](path.wgsl) `` directive in its `#### TML ...` header.
/// Same lifecycle as [`ImageLoad`] / [`FontLoad`]: the parser records the
/// request; `API::load_layout_file` reads the file, naga-validates it and
/// compiles it into a pipeline (see `API::register_ui_shader`). The layout then
/// applies it with `` `shader` *name* ``.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderLoad {
    /// Name `` `shader` *name* `` references it by (matched verbatim).
    pub name: String,
    /// Path to the `.wgsl` file, resolved like [`ImageLoad::path`].
    pub path: String,
}

/// One parsed `#### TML ...` header directive.
enum HeaderDirective {
    Image(ImageLoad),
    Font(FontLoad),
    Shader(ShaderLoad),
}

/// The files a page's `#### TML ...` header asked to be loaded. Returned by
/// [`Binder::load_layout`]; `API::load_layout_file` fulfils each.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LayoutResources {
    pub image_loads: Vec<ImageLoad>,
    pub font_loads: Vec<FontLoad>,
    pub shader_loads: Vec<ShaderLoad>,
}

/// A parsed page: its flattened layout commands, its table of reusable snippets
/// (declared with `##`/`###` headings, referenced with `use`), and the images
/// its `#### TML ...` header asked to be loaded.
///
/// The page has no name of its own - the `# ...` heading only marks where the
/// body starts (its text is ignored, `# root` by convention). The caller names
/// the page when it registers it (see [`Binder::load_layout`]); `API` uses the
/// layout file's own name, minus the `.md`.
#[derive(Debug, Default)]
pub struct ParsedLayout {
    pub body: Vec<Layout>,
    pub reusables: HashMap<String, Vec<Layout>>,
    pub image_loads: Vec<ImageLoad>,
    pub font_loads: Vec<FontLoad>,
    pub shader_loads: Vec<ShaderLoad>,
}

/// Parse-time environment for compile-time `` `calc` `` expressions: the file's
/// `` `calc` `` functions plus a stack of in-scope static numeric declarations
/// (innermost scope last). A `` `set-numeric` `` whose value is an expression is
/// evaluated against this the moment its line is read, so it only ever sees
/// declarations written textually before it.
struct CalcCtx {
    fns: HashMap<String, calc::CalcFn>,
    scope: Vec<HashMap<String, f32>>,
}

impl CalcCtx {
    fn new(fns: HashMap<String, calc::CalcFn>) -> Self {
        // One frame for the page body; `list` / `use` push their own.
        CalcCtx {
            fns,
            scope: vec![HashMap::new()],
        }
    }

    fn eval_str(&self, src: &str) -> Result<f32, calc::CalcError> {
        calc::eval(&calc::parse_expr(src)?, &self.scope, &self.fns, 0)
    }

    /// Records a resolved static numeric so later expressions in the same (or a
    /// nested) scope can reference it by name.
    fn bind(&mut self, name: &str, value: f32) {
        if let Some(frame) = self.scope.last_mut() {
            frame.insert(name.to_string(), value);
        }
    }
}

/// Scans a parsed markdown document for `` - `calc` `name(args) = expr` ``
/// items under the `#### TML ...` header and builds the function table. Runs
/// before the main walk so a `` `set-numeric` `` expression anywhere in the
/// file can call a function regardless of where the header sits.
fn collect_calc_fns(root: &Node) -> HashMap<String, calc::CalcFn> {
    let mut fns = HashMap::new();
    let Some(nodes) = root.children() else {
        return fns;
    };

    let mut in_header = false;
    for node in nodes {
        match node {
            Node::Heading(h) => {
                in_header = h.depth == 4
                    && matches!(h.children.first(), Some(Node::Text(t)) if t.value.trim().starts_with("TML"));
            }
            Node::List(list) if in_header => {
                for item in &list.children {
                    if let Node::ListItem(item) = item
                        && let Some(Node::Paragraph(p)) = item.children.first()
                        && let Some(Node::InlineCode(keyword)) = p.children.first()
                        && keyword.value == "calc"
                        && let Some(Node::InlineCode(spec)) =
                            p.children.iter().skip(1).find(|n| matches!(n, Node::InlineCode(_)))
                    {
                        match calc::parse_fn(spec.value.trim()) {
                            Ok((name, func)) => {
                                fns.insert(name, func);
                            }
                            Err(error) => {
                                eprintln!("TML calc: `calc` `{}`: {error}", spec.value.trim());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    fns
}

/// Every keyword the grammar looks for as the *first token* of a list item -
/// element, config, declaration and header directives. **Sorted** (binary
/// searched by [`add_missing_keyword_backticks`]).
///
/// Kept in sync by hand with the `match` arms of [`process_element`],
/// [`process_configs`], [`process_variable`] and [`process_header_directive`];
/// adding a keyword there means adding it here too. `config` is included even
/// though the parser only recognises it positionally, so a backtick-free file
/// still round-trips to the canonical `` `config` `` form. Argument words that
/// are written as inline code but never lead an item (`min`, `max`, `x`, `y`,
/// `height`, and the alignment/attach-point value words) are deliberately *not*
/// in this list.
const LEADING_KEYWORDS: &[&str] = &[
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
    "line-height", "list", "load", "max-zoom", "min-zoom", "near", "no-clip", "offset-x", "offset-y",
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
    "shader", "shader-param-1", "shader-param-2", "shader-param-3", "shader-param-4",
    "shader-param-5", "shader-param-6", "shader-param-7", "shader-param-8",
    "shadow-blur", "shadow-color", "shadow-offset-x", "shadow-offset-y", "shadow-spread",
    "start-angle", "target-x", "target-y", "target-z", "text", "thickness", "to", "to-x",
    "to-y", "unfocused", "unhovered", "up-x", "up-y", "up-z",
    "use", "vertical", "width",
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
/// (`` `image` *atlas* `[0,0,1,1]` ``). And because
/// this runs with no grammar context, a plain-text line - a `text` element's
/// content, say - that happens to start with a keyword word (`color`, `image`,
/// `if`, ...) *will* be turned into a config; write that line's backticks
/// yourself (or reword it) to opt out.
fn add_missing_keyword_backticks(source: &str) -> String {
    debug_assert!(
        LEADING_KEYWORDS.windows(2).all(|w| w[0] < w[1]),
        "LEADING_KEYWORDS must stay sorted"
    );

    let mut out = String::with_capacity(source.len() + 64);
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

        match backtick_leading_keyword(text) {
            Some(rewritten) => {
                out.push_str(&rewritten);
                out.push_str(newline);
            }
            None => out.push_str(line),
        }
    }

    out
}

/// The per-line worker for [`add_missing_keyword_backticks`]. Returns the
/// rewritten line if `line` is a list item whose first word is a keyword and
/// isn't already backticked, otherwise `None` (leave the line as-is).
fn backtick_leading_keyword(line: &str) -> Option<String> {
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

    let word_end = content
        .find([' ', '\t'])
        .unwrap_or(content.len());
    let word = &content[..word_end];
    if LEADING_KEYWORDS.binary_search(&word).is_err() {
        return None;
    }

    // Everything up to the first content character, verbatim, then `` `word` ``.
    let prefix = &line[..line.len() - content.len()];
    Some(format!("{prefix}`{word}`{}", &content[word_end..]))
}

/// Parses a markdown layout document (see `examples/layouts/Main.md`) into a
/// [`ParsedLayout`].
pub fn process_layout(file: String) -> Result<ParsedLayout, String> {
    let file = add_missing_keyword_backticks(&file);
    let mut parsing_mode = ParsingMode::None;
    let mut body = Vec::<Layout>::new();
    let mut open_reuseable_name = "".to_string();
    let mut reusables = HashMap::<String, Vec<Layout>>::new();
    let mut image_loads = Vec::<ImageLoad>::new();
    let mut font_loads = Vec::<FontLoad>::new();
    let mut shader_loads = Vec::<ShaderLoad>::new();

    if let Ok(m) = markdown::to_mdast(&file, &markdown::ParseOptions::default())
        && let Some(nodes) = m.children()
    {
        let mut calc_ctx = CalcCtx::new(collect_calc_fns(&m));
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
                            4 if declaration.value.trim().starts_with("TML") => {
                                // `#### TML <version>` opens the preamble block.
                                parsing_mode = ParsingMode::Header;
                            }
                            _ => parsing_mode = ParsingMode::None,
                        }
                    }
                }
                Node::List(list) => match parsing_mode {
                    ParsingMode::Header => {
                        for item in &list.children {
                            match process_header_directive(item) {
                                Some(HeaderDirective::Image(load)) => image_loads.push(load),
                                Some(HeaderDirective::Font(load)) => font_loads.push(load),
                                Some(HeaderDirective::Shader(load)) => shader_loads.push(load),
                                None => {}
                            }
                        }
                    }
                    ParsingMode::ReusableConfig => {
                        let mut reusable_items = process_configs(list, &mut None, &mut calc_ctx);
                        let mut formatted_reusable_items = Vec::<Layout>::new();
                        formatted_reusable_items.append(&mut reusable_items);
                        reusables.insert(open_reuseable_name.clone(), formatted_reusable_items);
                    }
                    ParsingMode::ReusableElements => {
                        for node in &list.children {
                            let element = process_element(node, &mut calc_ctx);
                            reusables.insert(open_reuseable_name.clone(), element);
                        }
                    }
                    ParsingMode::Body => {
                        body.push(Layout::Element(Element::Pointer(
                            winit::window::CursorIcon::Default,
                        )));
                        for node in &list.children {
                            let mut element = process_element(node, &mut calc_ctx);
                            body.append(&mut element);
                        }
                    }
                    ParsingMode::None => {}
                },
                _ => {}
            }
        }
        Ok(ParsedLayout {
            body,
            reusables,
            image_loads,
            font_loads,
            shader_loads,
        })
    } else {
        Err("failed to parse layout markdown".to_string())
    }
}

/// Parses one item of a `#### TML ...` header list:
/// - `` - `load` [atlas](path) `` - an image, registered under the atlas name.
/// - `` - `font` [id](path) `` - a `.ttf`/`.otf` selected by `` `font-id` `` `id`
///   (also added to the fallback chain).
/// - `` - `shader` [name](path.wgsl) `` - a custom UI effect shader, applied
///   with `` `shader` *name* ``.
///
/// In every case the link text is the name/id and the link url is the file path.
fn process_header_directive(item: &Node) -> Option<HeaderDirective> {
    let Node::ListItem(item) = item else {
        return None;
    };
    let Some(Node::Paragraph(paragraph)) = item.children.first() else {
        return None;
    };
    let Some(Node::InlineCode(keyword)) = paragraph.children.first() else {
        return None;
    };

    let link = paragraph.children.iter().find_map(|node| match node {
        Node::Link(link) => Some(link),
        _ => None,
    })?;
    let name = link.children.iter().find_map(|node| match node {
        Node::Text(text) => Some(text.value.trim().to_string()),
        _ => None,
    })?;
    let path = link.url.trim().to_string();
    if name.is_empty() || path.is_empty() {
        return None;
    }

    match keyword.value.as_str() {
        "load" => Some(HeaderDirective::Image(ImageLoad { atlas: name, path })),
        "font" => Some(HeaderDirective::Font(FontLoad {
            id: name.parse().ok()?,
            path,
        })),
        "shader" => Some(HeaderDirective::Shader(ShaderLoad { name, path })),
        _ => None,
    }
}

/// Parses a `[u1, v1, u2, v2]` bracketed list of four floats (the UV
/// sub-rectangle for an `image` literal / `set-image` declaration). Whitespace
/// around and inside the brackets is ignored. Returns `None` unless exactly four
/// numbers parse.
fn parse_uv_rect(raw: &str) -> Option<[f32; 4]> {
    let inner = raw.trim().strip_prefix('[')?.strip_suffix(']')?;
    let mut values = [0.0f32; 4];
    let mut count = 0;
    for part in inner.split(',') {
        let value = part.trim().parse::<f32>().ok()?;
        if count == 4 {
            return None;
        }
        values[count] = value;
        count += 1;
    }
    if count == 4 { Some(values) } else { None }
}

/// A single argument like `item`'s index or `if-index`'s comparand: a bare
/// number is a static value, anything else is a dynamic symbol to resolve
/// (typically a plain numeric field, e.g. `selected_document`).
fn parse_index_arg(arg: &str) -> DataSrc<f32> {
    match arg.parse::<f32>() {
        Ok(value) => DataSrc::Static(value),
        Err(_) => DataSrc::Dynamic(field_symbol(arg)),
    }
}

/// The `` `keyword` `` a list item leads with (`config`, `use`, `element`,
/// `image`, ...), or `None` if the item isn't `` `keyword` ``-shaped.
fn list_item_keyword(node: &Node) -> Option<&str> {
    if let Node::ListItem(item) = node
        && let Some(Node::Paragraph(paragraph)) = item.children.first()
        && let Some(Node::InlineCode(marker)) = paragraph.children.first()
    {
        Some(marker.value.as_str())
    } else {
        None
    }
}

/// True if `node` is a `- \`declarations\`` list item, the way a `list`'s
/// (optional) leading declarations block looks.
fn is_declarations_block(node: &Node) -> bool {
    list_item_keyword(node) == Some("declarations")
}

/// True if `node` is a `- \`config\`` list item - an element's (optional)
/// leading config block. An element body without one starts straight into
/// child elements.
fn is_config_block(node: &Node) -> bool {
    list_item_keyword(node) == Some("config")
}

/// Emits the layout commands for a drawn-shape element (`` `circle` ``,
/// `` `ring` ``, `` `line` ``, `` `arc` ``, `` `bezier` ``). Same structure as
/// the `` `element` `` arm - open, optional id, run the `config` block (with
/// `spec` threaded through so the shape-only keywords land on it), then any
/// child elements, close. `spec` starts as the bare-keyword default and
/// `process_configs` mutates it in place.
///
/// A shape can nest children (each their own element with its own bounding box
/// and, if it wants, its own custom shape) - the layout engine treats a custom
/// element as an ordinary container. Stacking arcs this way draws an RPM-gauge
/// style dial: several concentric sweeps sharing one box.
fn process_shape(
    element: &ListItem,
    element_declaration: &Paragraph,
    mut spec: CustomElementSpec,
    ctx: &mut CalcCtx,
) -> Vec<Layout> {
    let mut layout_commands: Vec<Layout> = Vec::new();
    layout_commands.push(Layout::Element(Element::ElementOpened { id: None }));
    layout_commands.push(Layout::Element(Element::ConfigOpened));
    if let Some(element_name) = element_declaration.children.get(1)
        && let Node::Text(element_name) = element_name
    {
        let name = element_name.value.trim();
        layout_commands.push(Layout::Config(Config::Id(DataSrc::Static(name.to_string()))));
        // A `render-window`'s camera is its element name (un-named -> `default`).
        if let CustomElementSpec::RenderWindow { camera, .. } = &mut spec {
            *camera = GlobalSymbol::new(name);
        }
    }
    let body = element.children.get(1).and_then(|node| match node {
        Node::List(list) => Some(list),
        _ => None,
    });
    let has_config = body
        .and_then(|list| list.children.first())
        .is_some_and(is_config_block);

    if has_config
        && let Some(body) = body
        && let Some(Node::ListItem(configs)) = body.children.first()
        && let Some(Node::List(config_commands)) = configs.children.get(1)
    {
        let mut layout_config_commands =
            process_configs(config_commands, &mut Some(&mut spec), ctx);
        layout_commands.append(&mut layout_config_commands);
    }
    layout_commands.push(Layout::Config(Config::CustomElement(spec)));
    layout_commands.push(Layout::Element(Element::ConfigClosed));

    // Child elements sit after the `config` block (if any), same as `element`.
    if let Some(body) = body {
        for child_element in body.children.iter().skip(usize::from(has_config)) {
            let mut child_element = process_element(child_element, ctx);
            layout_commands.append(&mut child_element);
        }
    }

    layout_commands.push(Layout::Element(Element::ElementClosed));
    layout_commands
}

/// The `` `canvas` `` element: an infinitely pannable, zoomable surface.
///
/// Emits an **outer clip container** (carrying the user's own `config` plus the
/// clip + pan `childOffset` from the named [`Canvas`](crate::Canvas)) wrapping a
/// fixed-size **world wrapper** (sized `world-width`/`world-height`), inside
/// which the body children are laid out. The `CanvasWorldOpened` /
/// `CanvasWorldClosed` markers around the wrapper tell `set_layout` to multiply
/// every spatial config in between by the canvas's `zoom`.
///
/// `canvas`-only config keywords (`world-width`, `world-height`, `min-zoom`,
/// `max-zoom`, `zoom`, `pan-x`, `pan-y`) are pulled out here; everything else in
/// the `config` block goes through `process_configs` onto the outer container.
fn process_canvas(
    element: &ListItem,
    element_declaration: &Paragraph,
    ctx: &mut CalcCtx,
) -> Vec<Layout> {
    let mut out: Vec<Layout> = Vec::new();

    let name = element_declaration
        .children
        .get(1)
        .and_then(|node| match node {
            Node::Text(text) if !text.value.trim().is_empty() => Some(text.value.trim().to_string()),
            _ => None,
        });
    let name_sym = GlobalSymbol::new(name.as_deref().unwrap_or("canvas"));

    let body = element.children.get(1).and_then(|node| match node {
        Node::List(list) => Some(list),
        _ => None,
    });
    let config_list = body
        .and_then(|list| list.children.first())
        .filter(|first| is_config_block(first))
        .and_then(|first| match first {
            Node::ListItem(item) => item.children.get(1),
            _ => None,
        })
        .and_then(|node| match node {
            Node::List(list) => Some(list),
            _ => None,
        });
    let has_config = config_list.is_some();

    // Pull the canvas-only keywords out of the config block.
    let mut world_width = None;
    let mut world_height = None;
    let mut min_zoom = None;
    let mut max_zoom = None;
    let mut initial_zoom = None;
    let mut initial_pan_x = None;
    let mut initial_pan_y = None;
    if let Some(config_list) = config_list {
        for item in &config_list.children {
            let Some(Node::Paragraph(paragraph)) = item.children().and_then(|c| c.first()) else {
                continue;
            };
            let Some(Node::InlineCode(keyword)) = paragraph.children.first() else {
                continue;
            };
            let slot = match keyword.value.as_str() {
                "world-width" => &mut world_width,
                "world-height" => &mut world_height,
                "min-zoom" => &mut min_zoom,
                "max-zoom" => &mut max_zoom,
                "zoom" => &mut initial_zoom,
                "pan-x" => &mut initial_pan_x,
                "pan-y" => &mut initial_pan_y,
                _ => continue,
            };
            *slot = optional_arg::<f32>(paragraph);
        }
    }

    // ---- outer clip container ---------------------------------------------
    out.push(Layout::Element(Element::ElementOpened { id: None }));
    out.push(Layout::Element(Element::ConfigOpened));
    if let Some(name) = &name {
        out.push(Layout::Config(Config::Id(DataSrc::Static(name.clone()))));
    }
    if let Some(config_list) = config_list {
        // Non-canvas keywords (`color`, `padding`, `border`, `grow`, ...) land on
        // the outer container. The canvas keywords fall through the `_ => {}`.
        out.append(&mut process_configs(config_list, &mut None, ctx));
    }
    out.push(Layout::Config(Config::Canvas {
        name: name_sym,
        world_width,
        world_height,
        min_zoom,
        max_zoom,
        initial_zoom,
        initial_pan_x,
        initial_pan_y,
    }));
    out.push(Layout::Element(Element::ConfigClosed));

    // ---- zoom-bake scope + world wrapper ---------------------------------
    out.push(Layout::Element(Element::CanvasWorldOpened { name: name_sym }));
    out.push(Layout::Element(Element::ElementOpened { id: None }));
    out.push(Layout::Element(Element::ConfigOpened));
    out.push(Layout::Config(Config::CanvasWorldSize { name: name_sym }));
    out.push(Layout::Element(Element::ConfigClosed));

    if let Some(body) = body {
        for child in body.children.iter().skip(usize::from(has_config)) {
            out.append(&mut process_element(child, ctx));
        }
    }

    out.push(Layout::Element(Element::ElementClosed)); // world wrapper
    out.push(Layout::Element(Element::CanvasWorldClosed));
    out.push(Layout::Element(Element::ElementClosed)); // outer clip container
    out
}

fn process_element(element: &Node, ctx: &mut CalcCtx) -> Vec<Layout> {
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
                        if let Some((name, value)) = process_variable(declaration, ctx) {
                            if let DataSrc::Static(Declaration::Numeric(number)) = value {
                                ctx.bind(&name, number);
                            }
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
                // The body list's first item is the `config` block *only* if it
                // actually is one - an element can skip `config` and start
                // straight into child elements (or a lone `use`), and mistaking
                // that first child for the config block would drop it.
                let body = element.children.get(1).and_then(|node| match node {
                    Node::List(list) => Some(list),
                    _ => None,
                });
                let has_config = body
                    .and_then(|list| list.children.first())
                    .is_some_and(is_config_block);

                if has_config
                    && let Some(body) = body
                    && let Some(Node::ListItem(configs)) = body.children.first()
                    && let Some(Node::List(config_commands)) = configs.children.get(1)
                {
                    let mut layout_config_commands =
                        process_configs(config_commands, &mut None, ctx);
                    layout_commands.append(&mut layout_config_commands);
                }
                layout_commands.push(Layout::Element(Element::ConfigClosed));

                if let Some(body) = body {
                    let skip = usize::from(has_config);
                    for child_element in body.children.iter().skip(skip) {
                        let mut child_element = process_element(child_element, ctx);
                        layout_commands.append(&mut child_element);
                    }
                }

                layout_commands.push(Layout::Element(Element::ElementClosed));
            }
            "circle" => layout_commands.append(&mut process_shape(
                element,
                element_declaration,
                CustomElementSpec::Circle,
                ctx,
            )),
            "ring" => layout_commands.append(&mut process_shape(
                element,
                element_declaration,
                CustomElementSpec::ring(),
                ctx,
            )),
            "line" => layout_commands.append(&mut process_shape(
                element,
                element_declaration,
                CustomElementSpec::line(),
                ctx,
            )),
            "arc" => layout_commands.append(&mut process_shape(
                element,
                element_declaration,
                CustomElementSpec::arc(),
                ctx,
            )),
            "bezier" => layout_commands.append(&mut process_shape(
                element,
                element_declaration,
                CustomElementSpec::bezier(),
                ctx,
            )),
            "render-window" => layout_commands.append(&mut process_shape(
                element,
                element_declaration,
                CustomElementSpec::render_window(),
                ctx,
            )),
            "canvas" => {
                layout_commands.append(&mut process_canvas(element, element_declaration, ctx))
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
                    let mut configs = process_configs(configs, &mut None, ctx);
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
                                let src = field_symbol(dynamic_text.value.trim());
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
                    ctx.scope.push(HashMap::new());
                    for input_variable in &input_variables.children {
                        if let Some((name, declaration)) = process_variable(input_variable, ctx) {
                            if let DataSrc::Static(Declaration::Numeric(number)) = declaration {
                                ctx.bind(&name, number);
                            }
                            let name = GlobalSymbol::new(name);
                            layout_commands.push(Layout::Declaration {
                                name,
                                value: declaration,
                            });
                        }
                    }
                    ctx.scope.pop();
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
                    ctx.scope.push(HashMap::new());

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
                            if let Some((name, declaration)) = process_variable(declaration, ctx) {
                                if let DataSrc::Static(Declaration::Numeric(number)) = declaration {
                                    ctx.bind(&name, number);
                                }
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
                        let mut list_item = process_element(li, ctx);
                        formatted_list.append(&mut list_item);
                    }

                    ctx.scope.pop();

                    let src = field_symbol(list_src.value.trim());
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
                        let list_symbol = field_symbol(list_name);
                        let index = parse_index_arg(index_arg);

                        layout_commands.push(Layout::Element(Element::ItemOpened));

                        if let Some(body) = element.children.get(1)
                            && let Node::List(body) = body
                        {
                            for item in &body.children {
                                let mut item = process_element(item, ctx);
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
                    let src = field_symbol(conditional.value.trim());
                    formatted_element.push(Layout::Element(Element::IfOpened { condition: src }));

                    for conditional_element in &conditional_elements.children {
                        let mut conditional_element = process_element(conditional_element, ctx);
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
                    let src = field_symbol(conditional.value.trim());
                    formatted_element
                        .push(Layout::Element(Element::IfNotOpened { condition: src }));

                    for conditional_element in &conditional_elements.children {
                        let mut conditional_element = process_element(conditional_element, ctx);
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
                        let mut conditional_element = process_element(conditional_element, ctx);
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
                        let mut conditional_element = process_element(conditional_element, ctx);
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
            _ => {}
        }
    }

    layout_commands
}

#[derive(Debug)]
enum AvailableParameters<T> {
    None,
    SingleStatic(T),
    SingleDynamic(GlobalSymbol),
}

/// Reads the single argument of a config keyword: an `*emphasised*` name is a
/// dynamic (`get-*`) reference, a plain-text token is a literal parsed as `T`,
/// and anything else (or nothing) is [`AvailableParameters::None`]. Keywords
/// that used to take a `` `min` ``/`` `max` `` (etc.) pair are now split into
/// one keyword per value, so this only ever handles a lone argument.
fn parameter_check<T: FromStr>(parameters: &Paragraph) -> AvailableParameters<T> {
    if let Some(parameter) = parameters.children.get(2)
        && let Node::Emphasis(parameter) = parameter
        && let Some(parameter) = parameter.children.first()
        && let Node::Text(parameter) = parameter
    {
        AvailableParameters::SingleDynamic(field_symbol(parameter.value.trim()))
    } else if let Some(parameter) = parameters.children.get(1)
        && let Node::Text(parameter) = parameter
        && let Ok(parameter) = T::from_str(parameter.value.trim())
    {
        AvailableParameters::SingleStatic(parameter)
    } else {
        AvailableParameters::None
    }
}

/// Resolves a config keyword's lone numeric argument to a `DataSrc`, or `None`
/// when the keyword was written bare. Used by the split sizing keywords
/// (`width-grow-min`, `offset-x`, ...).
fn optional_arg<T: FromStr>(config: &Paragraph) -> Option<DataSrc<T>> {
    match parameter_check::<T>(config) {
        AvailableParameters::SingleDynamic(name) => Some(DataSrc::Dynamic(name)),
        AvailableParameters::SingleStatic(value) => Some(DataSrc::Static(value)),
        AvailableParameters::None => None,
    }
}

/// The value of a `declarations` binding - the run after the `*name*`. Usually
/// plain text (`` `set-numeric` *a* 5 ``); for the backtick form
/// (`` `set-numeric` *a* `scale(w, 2)` ``, needed whenever the value contains a
/// `*` or `_` that Markdown would eat as emphasis) it is the code span. `None`
/// when the item carries no value at all.
fn declaration_value_str(paragraph: &Paragraph) -> Option<&str> {
    paragraph.children.iter().skip(3).find_map(|node| match node {
        Node::InlineCode(code) => Some(code.value.as_str()),
        Node::Text(text) if !text.value.trim().is_empty() => Some(text.value.as_str()),
        _ => None,
    })
}

fn process_variable(
    declaration: &Node,
    ctx: &CalcCtx,
) -> Option<(String, DataSrc<Declaration>)> {
    if let Node::ListItem(declaration) = declaration
        && let Some(declaration) = declaration.children.first()
        && let Node::Paragraph(declaration) = declaration
        && let Some(declaration_type) = declaration.children.first()
        && let Node::InlineCode(variable_type) = declaration_type
        && let Some(declaration_name) = declaration.children.get(2)
        && let Node::Emphasis(declaration_name) = declaration_name
        && let Some(declaration_name) = declaration_name.children.first()
        && let Node::Text(variable_name) = declaration_name
    {
        let raw_value = declaration_value_str(declaration).map(str::trim);
        let name = || variable_name.value.trim().to_string();
        match variable_type.value.as_str() {
            "get-bool" | "get-numeric" | "get-text" | "get-event" | "get-image" | "get-color" => {
                let value = field_symbol(raw_value?);
                Some((name(), DataSrc::<Declaration>::Dynamic(value)))
            }
            "set-bool" => bool::from_str(raw_value?)
                .ok()
                .map(|value| (name(), DataSrc::<Declaration>::Static(Declaration::Bool(value)))),
            // A literal number, or - failing that - a compile-time `calc`
            // expression over `calc` functions and earlier static declarations.
            "set-numeric" => {
                let raw = raw_value?;
                let value = match f32::from_str(raw) {
                    Ok(value) => Some(value),
                    Err(_) => match ctx.eval_str(raw) {
                        Ok(value) => Some(value),
                        Err(error) => {
                            eprintln!("TML calc: `set-numeric` *{}*: {error}", name());
                            None
                        }
                    },
                };
                value.map(|value| {
                    (name(), DataSrc::<Declaration>::Static(Declaration::Numeric(value)))
                })
            }
            "set-text" => Some((
                name(),
                DataSrc::<Declaration>::Static(Declaration::Text(raw_value?.to_string())),
            )),
            "set-event" => Some((
                name(),
                DataSrc::<Declaration>::Static(Declaration::Event(GlobalSymbol::new(raw_value?))),
            )),
            "set-color" => Color::from_str(raw_value?)
                .ok()
                .map(|value| (name(), DataSrc::<Declaration>::Static(Declaration::Color(value)))),
            // `` `set-image` *name* *source* [u1, v1, u2, v2] `` - `source` is a
            // second emphasis span (children[4]); the UV rect is optional
            // trailing text (children[5]).
            //
            // - **With** a UV rect: a literal descriptor - `source` is an atlas
            //   name (a `load` directive or `API::add_image`), sampled at that
            //   sub-rectangle.
            // - **Without** a UV rect: an *alias* - `name` resolves to whatever
            //   image `source` resolves to (another `set-image` / `get-image`
            //   binding in scope, or an app `get_image` field). This is how you
            //   forward an image into a `use` (`set-image` *icon* *play*).
            //   Runtime shape is identical to `get-image`.
            "set-image" => {
                let source = declaration
                    .children
                    .get(4)
                    .and_then(|node| match node {
                        Node::Emphasis(emphasis) => emphasis.children.first(),
                        _ => None,
                    })
                    .and_then(|node| match node {
                        Node::Text(text) => Some(text.value.trim()),
                        _ => None,
                    })?;
                let uv = declaration.children.get(5).and_then(|node| match node {
                    Node::Text(text) => parse_uv_rect(text.value.trim()),
                    _ => None,
                });
                let value = match uv {
                    Some([u1, v1, u2, v2]) => {
                        DataSrc::<Declaration>::Static(Declaration::Image(UIImageDescriptor {
                            atlas: GlobalSymbol::new(source).as_str(),
                            u1,
                            v1,
                            u2,
                            v2,
                        }))
                    }
                    None => DataSrc::<Declaration>::Dynamic(GlobalSymbol::new(source)),
                };
                Some((variable_name.value.trim().to_string(), value))
            }
            _ => None,
        }
    } else {
        None
    }
}

/// One grow/fit axis while a config block is being parsed. The bare keyword
/// (`width-grow`) and its `-min` / `-max` companions each land in a separate
/// list item, so they're collected here and folded into a single `Config` by
/// [`AxisSizing::emit`] once the whole block has been read.
#[derive(Default)]
struct AxisSizing {
    bare: bool,
    min: Option<DataSrc<f32>>,
    max: Option<DataSrc<f32>>,
}

impl AxisSizing {
    fn emit(
        self,
        configs: &mut Vec<Layout>,
        bare: Config,
        min_only: impl FnOnce(DataSrc<f32>) -> Config,
        max_only: impl FnOnce(DataSrc<f32>) -> Config,
        min_max: impl FnOnce(DataSrc<f32>, DataSrc<f32>) -> Config,
    ) {
        let config = match (self.min, self.max) {
            (Some(min), Some(max)) => Some(min_max(min, max)),
            (Some(min), None) => Some(min_only(min)),
            (None, Some(max)) => Some(max_only(max)),
            (None, None) if self.bare => Some(bare),
            (None, None) => None,
        };
        if let Some(config) = config {
            configs.push(Layout::Config(config));
        }
    }
}

/// The multi-input config keywords, each now split into one keyword per value,
/// collected across a whole config block and emitted once at the end.
#[derive(Default)]
struct SizingAccumulator {
    width_grow: AxisSizing,
    height_grow: AxisSizing,
    width_fit: AxisSizing,
    height_fit: AxisSizing,
    offset_x: Option<DataSrc<f32>>,
    offset_y: Option<DataSrc<f32>>,
    floating_width: Option<DataSrc<f32>>,
    floating_height: Option<DataSrc<f32>>,
}

fn process_configs(
    configuration_set: &List,
    custom_element: &mut Option<&mut CustomElementSpec>,
    ctx: &mut CalcCtx,
) -> Vec<Layout> {
    let mut configs = Vec::new();
    let mut sizing = SizingAccumulator::default();
    // `scroll-horizontal` / `scroll-vertical` both feed the one `Config::Clip`
    // (clip state is set per-axis-pair by the layout engine), so collect them
    // across the whole config list and emit a single command at the end.
    let mut scroll_horizontal = false;
    let mut scroll_vertical = false;
    // Each `shader` keyword opens a new effect on this element; the `shader-*` /
    // `shadow-*` / `bevel-*` / `glow-*` / `blur-*` keywords tune the most
    // recent one. Emitted as one `Config::Shaders` at the end, like `sizing`.
    let mut shader_specs: Vec<ShaderSpec> = Vec::new();

    for configuration_item in &configuration_set.children {
        if let Some(config_elements) = configuration_item.children()
            && let Some(config) = config_elements.first()
            && let Node::Paragraph(config) = config
            && let Some(config_type) = config.children.first()
            && let Node::InlineCode(config_type) = config_type
        {
            match config_type.value.as_str() {
                "grow" => configs.push(Layout::Config(Config::GrowAll)),
                "fit" => configs.push(Layout::Config(Config::FitAll)),
                "id-indexed" => match parameter_check::<String>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::IdIndexed(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::IdIndexed(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "aspect-ratio" => match parameter_check::<f32>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::AspectRatio(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::AspectRatio(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                // Sizing keywords come in three flat pieces - the bare keyword
                // plus a `-min` / `-max` variant - which are folded back into one
                // `Config` at the end of this function (see `sizing` below).
                "width-grow" => sizing.width_grow.bare = true,
                "width-grow-min" => sizing.width_grow.min = optional_arg(config),
                "width-grow-max" => sizing.width_grow.max = optional_arg(config),
                "height-grow" => sizing.height_grow.bare = true,
                "height-grow-min" => sizing.height_grow.min = optional_arg(config),
                "height-grow-max" => sizing.height_grow.max = optional_arg(config),
                "width-fit" => sizing.width_fit.bare = true,
                "width-fit-min" => sizing.width_fit.min = optional_arg(config),
                "width-fit-max" => sizing.width_fit.max = optional_arg(config),
                "height-fit" => sizing.height_fit.bare = true,
                "height-fit-min" => sizing.height_fit.min = optional_arg(config),
                "height-fit-max" => sizing.height_fit.max = optional_arg(config),
                "width-fixed" => match parameter_check::<f32>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::FixedX(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::FixedX(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "height-fixed" => match parameter_check::<f32>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::FixedY(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::FixedY(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "fixed-square" => match parameter_check::<f32>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::FixedSquare(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::FixedSquare(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "width-percent" => match parameter_check::<f32>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::PercentX(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::PercentX(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "height-percent" => match parameter_check::<f32>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::PercentY(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::PercentY(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "padding-all" => match parameter_check::<u16>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::PaddingAll(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::PaddingAll(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "padding-top" => match parameter_check::<u16>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::PaddingTop(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::PaddingTop(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "padding-right" => match parameter_check::<u16>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::PaddingRight(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::PaddingRight(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "padding-bottom" => match parameter_check::<u16>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::PaddingBottom(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::PaddingBottom(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "padding-left" => match parameter_check::<u16>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::PaddingLeft(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::PaddingLeft(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "child-gap" => match parameter_check::<u16>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::ChildGap(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::ChildGap(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "vertical" => configs.push(Layout::Config(Config::Vertical)),
                "horizontal" => configs.push(Layout::Config(Config::Horizontal)),
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
                "color" => match parameter_check::<Color>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::Color(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::Color(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                // Each `shader` opens a new effect on this element; the
                // `shader-*` / `shadow-*` / `bevel-*` / `glow-*` / `blur-*`
                // keywords tune the most recent one. Folded into one
                // `Config::Shaders` at the end.
                "shader" => {
                    // The name is the first `*emphasis*` or non-blank text after
                    // the `` `shader` `` keyword (`*drop_shadow*`, `bevel`,
                    // `neon`, ...). Matched verbatim against the built-in aliases
                    // in `ShaderSpec::kind` and against `` `shader` `` directive
                    // names, so it must be spelled exactly.
                    let name = config.children.iter().skip(1).find_map(|node| match node {
                        Node::Emphasis(e) => e.children.iter().find_map(|c| match c {
                            Node::Text(t) => Some(t.value.trim().to_string()),
                            _ => None,
                        }),
                        Node::Text(t) if !t.value.trim().is_empty() => {
                            Some(t.value.trim().to_string())
                        }
                        _ => None,
                    });
                    if let Some(name) = name.filter(|n| !n.is_empty()) {
                        shader_specs.push(ShaderSpec::with_name(field_symbol(&name)));
                    }
                }
                key if SHADER_PARAM_KEYWORDS.contains(&key) => {
                    if let Some(spec) = shader_specs.last_mut() {
                        spec.apply_param(key, config);
                    }
                }
                // Shape-only geometry keywords (`from`, `to`, `from-x`, `radius`,
                // `thickness`, the `width` alias, ...) land on the current custom
                // element instead of becoming a `Config`.
                key if SHAPE_PARAM_KEYWORDS.contains(&key) => {
                    if let Some(spec) = custom_element {
                        spec.apply_param(key, config);
                    }
                }
                "radius-all" => match parameter_check::<f32>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::RadiusAll(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::RadiusAll(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "radius-top-left" => match parameter_check::<f32>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::RadiusTopLeft(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::RadiusTopLeft(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "radius-top-right" => match parameter_check::<f32>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::RadiusTopRight(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::RadiusTopRight(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "radius-bottom-left" => match parameter_check::<f32>(config) {
                    AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(
                        Config::RadiusBottomLeft(DataSrc::Dynamic(a)),
                    )),
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::RadiusBottomLeft(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "radius-bottom-right" => match parameter_check::<f32>(config) {
                    AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(
                        Config::RadiusBottomRight(DataSrc::Dynamic(a)),
                    )),
                    AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(
                        Config::RadiusBottomRight(DataSrc::Static(a)),
                    )),
                    _ => {}
                },
                "border-color" => match parameter_check::<Color>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::BorderColor(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::BorderColor(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "border-all" => match parameter_check::<u16>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::BorderAll(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::BorderAll(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "border-top" => match parameter_check::<u16>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::BorderTop(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::BorderTop(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "border-left" => match parameter_check::<u16>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::BorderLeft(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::BorderLeft(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "border-bottom" => match parameter_check::<u16>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::BorderBottom(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::BorderBottom(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "border-right" => match parameter_check::<u16>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::BorderRight(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::BorderRight(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "border-in-between" => match parameter_check::<u16>(config) {
                    AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(
                        Config::BorderBetweenChildren(DataSrc::Dynamic(a)),
                    )),
                    AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(
                        Config::BorderBetweenChildren(DataSrc::Static(a)),
                    )),
                    _ => {}
                },
                "scroll-horizontal" => scroll_horizontal = true,
                "scroll-vertical" => scroll_vertical = true,
                "image" => {
                    // Three shapes:
                    //   `image` *name*              -> resolve `name` (set-image / get_image)
                    //   `image` name                -> same, name written bare
                    //   `image` *atlas* [u,v,u,v]   -> literal descriptor, no lookup
                    // The name sits in an `Emphasis` node (children[2]) or, when
                    // written bare, in the trailing `Text` after the keyword;
                    // an optional `[...]` UV rect follows in the next `Text`.
                    let emphasis_name = config.children.get(2).and_then(|node| match node {
                        Node::Emphasis(emphasis) => match emphasis.children.first() {
                            Some(Node::Text(text)) => Some(text.value.trim().to_string()),
                            _ => None,
                        },
                        _ => None,
                    });
                    let bare_name = config.children.get(1).and_then(|node| match node {
                        Node::Text(text) if !text.value.trim().is_empty() => {
                            Some(text.value.trim().to_string())
                        }
                        _ => None,
                    });
                    // A `[...]` rect can only follow an emphasis name (it lands
                    // in the Text node right after the Emphasis).
                    let uv = config
                        .children
                        .get(3)
                        .and_then(|node| match node {
                            Node::Text(text) => parse_uv_rect(text.value.trim()),
                            _ => None,
                        });

                    if let Some(name) = emphasis_name.or(bare_name) {
                        match uv {
                            Some([u1, v1, u2, v2]) => {
                                configs.push(Layout::Config(Config::ImageLiteral(
                                    UIImageDescriptor {
                                        atlas: GlobalSymbol::new(&name).as_str(),
                                        u1,
                                        v1,
                                        u2,
                                        v2,
                                    },
                                )));
                            }
                            None => {
                                // Interned verbatim, like every other dynamic
                                // name: `` `image` *portrait* `` resolves a
                                // `` `set-image` *portrait* `` binding or a
                                // `get_image` field of exactly that name.
                                configs.push(Layout::Config(Config::Image {
                                    name: field_symbol(&name),
                                }));
                            }
                        }
                    }
                }
                "floating" => {
                    configs.push(Layout::Config(Config::Floating));
                    if let Some(floating_commands) = config_elements.get(1)
                        && let Node::List(floating_commands) = floating_commands
                    {
                        let mut floating = process_configs(floating_commands, &mut None, ctx);
                        configs.append(&mut floating);
                    }
                }
                "clip-to-parent" => {
                    configs.push(Layout::Config(Config::FloatingClipToParent))
                }
                "no-clip" => configs.push(Layout::Config(Config::FloatingNoClip)),
                "pointer-capture" => {
                    configs.push(Layout::Config(Config::FloatingPointerCapture))
                }
                "pointer-pass-through" => {
                    configs.push(Layout::Config(Config::FloatingPointerPassThrough))
                }
                "attach-root" => {
                    configs.push(Layout::Config(Config::FloatingAttachElementToRoot))
                }
                "z-index" => match parameter_check::<i16>(config) {
                    AvailableParameters::SingleDynamic(z) => {
                        configs.push(Layout::Config(Config::FloatingZIndex {
                            z: DataSrc::Dynamic(z),
                        }))
                    }
                    AvailableParameters::SingleStatic(z) => {
                        configs.push(Layout::Config(Config::FloatingZIndex {
                            z: DataSrc::Static(z),
                        }))
                    }
                    _ => {}
                },
                "floating-dimensions-width" => {
                    sizing.floating_width = optional_arg(config)
                }
                "floating-dimensions-height" => {
                    sizing.floating_height = optional_arg(config)
                }
                "use" => {
                    if let Some(reusable_name) = config.children.get(1)
                        && let Node::Text(reusable_name) = reusable_name
                    {
                        let name = GlobalSymbol::new(reusable_name.value.trim());
                        // A nested list under the `use` is its parameter body -
                        // `set-*` / `get-*` bindings, same shape as a `### `
                        // element snippet's `use` body.
                        let mut params = Vec::new();
                        if let Some(Node::List(body)) = config_elements.get(1) {
                            for item in &body.children {
                                if let Some((n, v)) = process_variable(item, ctx) {
                                    if let DataSrc::Static(Declaration::Numeric(number)) = v {
                                        ctx.bind(&n, number);
                                    }
                                    params.push((GlobalSymbol::new(n), v));
                                }
                            }
                        }
                        configs.push(Layout::Config(Config::Use { name, params }));
                    }
                }

                "hovered" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::HoveredClosed));
                }
                "unhovered" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::UnHoveredClosed));
                }
                "hover" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::HoverClosed));
                }
                "focused" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::FocusedClosed));
                }
                "unfocused" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::UnFocusedClosed));
                }
                "focus" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::FocusClosed));
                }
                "key-event" => {
                    match parameter_check::<GlobalSymbol>(config) {
                        AvailableParameters::SingleDynamic(a) => {
                            configs.push(Layout::Element(Element::KeyEventOpened {
                                event: Some(DataSrc::Dynamic(a)),
                            }))
                        }
                        AvailableParameters::SingleStatic(a) => {
                            configs.push(Layout::Element(Element::KeyEventOpened {
                                event: Some(DataSrc::Static(a)),
                            }))
                        }
                        AvailableParameters::None => configs
                            .push(Layout::Element(Element::KeyEventOpened { event: None })),
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::KeyEventClosed));
                }
                "left-pressed" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::LeftPressedClosed));
                }
                "left-down" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::LeftDownClosed));
                }
                "left-released" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                        && let Node::List(onconfig_on) = onconfig_on
                    {
                        configs.append(&mut process_configs(onconfig_on, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::LeftReleasedClosed));
                }
                "left-clicked" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                        && let Node::List(config_on_click) = config_on_click
                    {
                        configs.append(&mut process_configs(config_on_click, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::LeftClickedClosed));
                }
                "left-dbl-clicked" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                        && let Node::List(config_on_click) = config_on_click
                    {
                        configs.append(&mut process_configs(config_on_click, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::LeftDoubleClickedClosed));
                }
                "left-tpl-clicked" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                        && let Node::List(config_on_click) = config_on_click
                    {
                        configs.append(&mut process_configs(config_on_click, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::LeftTripleClickedClosed));
                }
                "right-pressed" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                        && let Node::List(config_on_click) = config_on_click
                    {
                        configs.append(&mut process_configs(config_on_click, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::RightPressedClosed));
                }
                "right-down" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                        && let Node::List(config_on_click) = config_on_click
                    {
                        configs.append(&mut process_configs(config_on_click, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::RightDownClosed));
                }
                "right-released" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                        && let Node::List(config_on_click) = config_on_click
                    {
                        configs.append(&mut process_configs(config_on_click, &mut None, ctx));
                    }
                    configs.push(Layout::Element(Element::RightReleasedClosed));
                }
                "right-clicked" => {
                    match parameter_check::<GlobalSymbol>(config) {
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
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                        && let Node::List(config_on_click) = config_on_click
                    {
                        configs.append(&mut process_configs(config_on_click, &mut None, ctx));
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

                "font-id" => match parameter_check::<u16>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::FontId(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::FontId(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "font-size" => match parameter_check::<u16>(config) {
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
                "line-height" => match parameter_check::<u16>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::LineHeight(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::LineHeight(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "letter-spacing" => match parameter_check::<u16>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::LetterSpacing(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::LetterSpacing(DataSrc::Static(a))))
                    }
                    _ => {}
                },
                "wrap" => {
                    if let Some(mode) = config.children.get(1)
                        && let Node::Text(mode) = mode
                    {
                        match mode.value.trim() {
                            "words" => configs.push(Layout::Config(Config::WrapWords)),
                            "lines" => configs.push(Layout::Config(Config::WrapNewLines)),
                            "none" => configs.push(Layout::Config(Config::WrapNone)),
                            _ => {}
                        }
                    }
                }
                "font-color" => match parameter_check::<Color>(config) {
                    AvailableParameters::SingleDynamic(a) => {
                        configs.push(Layout::Config(Config::FontColor(DataSrc::Dynamic(a))))
                    }
                    AvailableParameters::SingleStatic(a) => {
                        configs.push(Layout::Config(Config::FontColor(DataSrc::Static(a))))
                    }
                    _ => {}
                },

                "offset-x" => sizing.offset_x = optional_arg(config),
                "offset-y" => sizing.offset_y = optional_arg(config),
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
                _ => {}
            }
        }
    }

    if scroll_horizontal || scroll_vertical {
        configs.push(Layout::Config(Config::Clip {
            vertical: DataSrc::Static(scroll_vertical),
            horizontal: DataSrc::Static(scroll_horizontal),
        }));
    }

    let SizingAccumulator {
        width_grow,
        height_grow,
        width_fit,
        height_fit,
        offset_x,
        offset_y,
        floating_width,
        floating_height,
    } = sizing;
    width_grow.emit(
        &mut configs,
        Config::GrowX,
        Config::GrowXmin,
        Config::GrowXmax,
        |min, max| Config::GrowXminmax { min, max },
    );
    height_grow.emit(
        &mut configs,
        Config::GrowY,
        Config::GrowYmin,
        Config::GrowYmax,
        |min, max| Config::GrowYminmax { min, max },
    );
    width_fit.emit(
        &mut configs,
        Config::FitX,
        Config::FitXmin,
        Config::FitXmax,
        |min, max| Config::FitXminmax { min, max },
    );
    height_fit.emit(
        &mut configs,
        Config::FitY,
        Config::FitYmin,
        Config::FitYmax,
        |min, max| Config::FitYminmax { min, max },
    );
    if offset_x.is_some() || offset_y.is_some() {
        configs.push(Layout::Config(Config::FloatingOffset {
            x: offset_x.unwrap_or(DataSrc::Static(0.0)),
            y: offset_y.unwrap_or(DataSrc::Static(0.0)),
        }));
    }
    if floating_width.is_some() || floating_height.is_some() {
        configs.push(Layout::Config(Config::FloatingDimensions {
            width: floating_width.unwrap_or(DataSrc::Static(0.0)),
            height: floating_height.unwrap_or(DataSrc::Static(0.0)),
        }));
    }
    if !shader_specs.is_empty() {
        configs.push(Layout::Config(Config::Shaders(shader_specs)));
    }

    // Hoist `` `floating` `` ahead of everything else in the block. `Config::Floating`
    // is just a flag (`config.floating()`), order-independent among the other
    // configs - but `set_layout` needs to know the element is floating *before*
    // it evaluates any event gate (`hover` / `left-clicked` / ...) in the same
    // block, so the gate hit-tests by id instead of calling `Clay_Hovered()` on
    // the unconfigured element (see the `hovered!` macro). Without this, writing
    // `left-clicked` above `floating` would reintroduce the DUPLICATE_ID storm.
    if let Some(pos) = configs
        .iter()
        .position(|c| matches!(c, Layout::Config(Config::Floating)))
        && pos != 0
    {
        let floating = configs.remove(pos);
        configs.insert(0, floating);
    }

    configs
}

// ---------------------------------------------------------------------------
// Runner: walks the flattened commands each frame
// ---------------------------------------------------------------------------

/// Scans a page's top-level `declarations` - the ones that sit directly in
/// the page body, not inside a `list`/`use`/`item` (those scope
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
                Element::ListOpened | Element::UseOpened | Element::ItemOpened,
            ) => {
                depth += 1;
            }
            Layout::Element(
                Element::ListClosed(_) | Element::UseClosed(_) | Element::ItemClosed { .. },
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
    ///
    /// Returns the [`LayoutResources`] the document's `#### TML ...` header
    /// asked to be loaded (`load` / `font` / `shader`); the caller
    /// (`API::load_layout_file`) reads those files and hands them to the
    /// renderer. `Binder` itself never touches the filesystem or the renderer.
    pub fn load_layout(
        &mut self,
        name: &str,
        markdown_source: &str,
    ) -> Result<LayoutResources, String> {
        let parsed = process_layout(markdown_source.to_string())?;
        for (reusable_name, reusable) in parsed.reusables {
            self.add_reusable(&reusable_name, reusable);
        }
        self.add_page(name, parsed.body);
        Ok(LayoutResources {
            image_loads: parsed.image_loads,
            font_loads: parsed.font_loads,
            shader_loads: parsed.shader_loads,
        })
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

        // Focus only moves on a mouse-down frame. `set_layout` fills this with
        // the topmost hovered named element as it walks; whatever's left (incl.
        // `None`, i.e. a click on empty space / an unnamed element) is committed
        // below, so clicking away clears focus.
        let focusing = api.left_mouse_pressed() || api.right_mouse_pressed();
        let mut focus_target: Option<GlobalSymbol> = None;

        let _pointer = set_layout(
            api,
            layout_commands,
            &mut self.reusable,
            Some(&page_locals),
            None,
            &mut config,
            &mut text_config,
            user_app,
            winit::window::CursorIcon::Default,
            &mut focus_target,
            1.0,
        );

        if focusing {
            api.set_focus(focus_target);
        }

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
    commands: &mut [Layout],
    reusables: &mut HashMap<GlobalSymbol, Vec<Layout>>,
    locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration>>>,
    list_data: Option<(GlobalSymbol, usize)>,
    config: &mut ElementConfiguration,
    text_config: &mut TextConfig,
    user_app: &mut UserApp,
    mut pointer: winit::window::CursorIcon,
    // Accumulates the focus target for this frame: on a mouse-down frame every
    // hovered *named* element writes its own name here as the tree is walked
    // (outermost first), so the last writer - the innermost / topmost hovered
    // named element - wins. `set_page` commits it to `api.set_focus` afterwards.
    focus_target: &mut Option<GlobalSymbol>,
    // Multiplier applied to every spatial config (sizing, spacing, borders,
    // radii, font size, ...) while walking the body of a `` `canvas` ``. `1.0`
    // everywhere else. Pushed/popped by the `CanvasWorld*` markers.
    canvas_scale: f32,
) -> winit::window::CursorIcon
where
    UserApp: LayoutRunnerReflection + LayoutReflector,
{
    let mut nesting_level: u32 = 0;
    let mut skip: Option<u32> = None;
    let mut canvas_scale = canvas_scale;
    let mut canvas_scale_stack: Vec<f32> = Vec::new();
    // Name of the element whose `config` block is currently open (its `` `element`
    // *name* `` -> `Config::Id`), or `None` for an unnamed element. Reset at each
    // `ConfigOpened`, read at `ConfigClosed` and by the `focus` gate.
    let mut current_element_name: Option<GlobalSymbol> = None;
    // Does the currently-open `config` block carry `` `floating` ``? Set when
    // `Config::Floating` is seen, reset at `ConfigOpened`. An event gate on a
    // floating element must not fall through to `Clay_Hovered()` (see the
    // `hovered!` macro below).
    let mut current_element_floating = false;
    // An id synthesised for an *un-named* floating element that hits an event
    // gate, so the gate has something to hit-test by and `ConfigClosed` can
    // attach the same id. Reset at `ConfigOpened`.
    let mut gate_synthetic_id: Option<GlobalSymbol> = None;
    let mut gate_synth_counter: u32 = 0;
    // Was this a left/right mouse-down frame? Then focus is up for grabs.
    let focusing = api.left_mouse_pressed() || api.right_mouse_pressed();

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

    // Is the pointer over the element whose `config` block is currently open?
    //
    // This runs from an event gate (`hover`, `left-clicked`, ...) *inside* the
    // config block, i.e. before `configure_element`. `api.l.hovered()` there
    // makes clay generate + register an anonymous id for the still-unconfigured
    // element (`Clay_Hovered` -> `Clay__GenerateIdForAnonymousElement`), and for
    // consecutive **floating** siblings those ids collide - clay doesn't advance
    // the parent's child count for a floating child - which spams
    // `CLAY_ERROR_TYPE_DUPLICATE_ID` every frame. So hit-test by the element's
    // real id instead: it hashes identically to what `configure_element` will
    // attach, and reading last frame's `pointerOverIds` has no side effect. An
    // un-named floating element gets a synthesised, position-stable id (also
    // attached at `ConfigClosed`) so it works too.
    macro_rules! hovered {
        () => {{
            let id = match current_element_name.or(gate_synthetic_id) {
                Some(sym) => Some(sym),
                None if current_element_floating => {
                    let sym = GlobalSymbol::new(synthetic_gate_id_name(
                        &list_data,
                        gate_synth_counter,
                    ));
                    gate_synth_counter += 1;
                    gate_synthetic_id = Some(sym);
                    Some(sym)
                }
                None => None,
            };
            match id {
                Some(sym) => api.l.pointer_over(api.l.get_element_id(sym.as_str())),
                None => api.l.hovered(),
            }
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

                    Element::HoverOpened { event } => event_gate_open!(hovered!(), event),
                    Element::HoverClosed => event_gate_close!(),
                    Element::HoveredOpened { .. } => event_gate_unsupported!(), // TODO: needs hover-edge tracking in `API`
                    Element::HoveredClosed => event_gate_close!(),
                    Element::UnHoveredOpened { .. } => event_gate_unsupported!(), // TODO: needs hover-edge tracking in `API`
                    Element::UnHoveredClosed => event_gate_close!(),

                    // `focus`: gates its block while this (named) element is the
                    // focused one - the analogue of `hover`. `focused` /
                    // `unfocused` still need focus-*edge* tracking, so they stay
                    // unsupported like `hovered` / `unhovered`.
                    Element::FocusOpened { event } => {
                        let is_focused = current_element_name
                            .is_some_and(|name| api.focus() == Some(name));
                        event_gate_open!(is_focused, event)
                    }
                    Element::FocusClosed => event_gate_close!(),
                    Element::FocusedOpened { .. } => event_gate_unsupported!(), // TODO: needs focus-edge tracking
                    Element::FocusedClosed => event_gate_close!(),
                    Element::UnFocusedOpened { .. } => event_gate_unsupported!(), // TODO: needs focus-edge tracking
                    Element::UnFocusedClosed => event_gate_close!(),

                    Element::LeftPressedOpened { event } => {
                        event_gate_open!(hovered!() && api.left_mouse_pressed(), event)
                    }
                    Element::LeftPressedClosed => event_gate_close!(),
                    Element::LeftDownOpened { event } => {
                        event_gate_open!(hovered!() && api.left_mouse_down(), event)
                    }
                    Element::LeftDownClosed => event_gate_close!(),
                    Element::LeftReleasedOpened { event } => {
                        event_gate_open!(hovered!() && api.left_mouse_released(), event)
                    }
                    Element::LeftReleasedClosed => event_gate_close!(),
                    Element::LeftClickedOpened { event } => {
                        event_gate_open!(hovered!() && api.left_mouse_clicked(), event)
                    }
                    Element::LeftClickedClosed => event_gate_close!(),
                    Element::LeftDoubleClickedOpened { event } => {
                        event_gate_open!(hovered!() && api.left_mouse_double_clicked(), event)
                    }
                    Element::LeftDoubleClickedClosed => event_gate_close!(),
                    Element::LeftTripleClickedOpened { .. } => event_gate_unsupported!(), // TODO: `API` doesn't track triple-clicks yet
                    Element::LeftTripleClickedClosed => event_gate_close!(),

                    // `key-event` only fires while its element holds focus - the
                    // runner checks that here, so a layout never has to nest it
                    // inside a `focus` block. (Only named elements are focusable,
                    // so an unnamed `key-event` element is simply never active.)
                    Element::KeyEventOpened { event } => {
                        let focused = current_element_name
                            .is_some_and(|name| api.focus() == Some(name));
                        event_gate_open!(focused && !api.key_events().is_empty(), event)
                    }
                    Element::KeyEventClosed => event_gate_close!(),

                    Element::RightPressedOpened { event } => {
                        event_gate_open!(hovered!() && api.right_mouse_pressed(), event)
                    }
                    Element::RightPressedClosed => event_gate_close!(),
                    Element::RightDownOpened { event } => {
                        event_gate_open!(hovered!() && api.right_mouse_down(), event)
                    }
                    Element::RightDownClosed => event_gate_close!(),
                    Element::RightReleasedOpened { event } => {
                        event_gate_open!(hovered!() && api.right_mouse_released(), event)
                    }
                    Element::RightReleasedClosed => event_gate_close!(),
                    Element::RightClickedOpened { event } => {
                        event_gate_open!(hovered!() && api.right_mouse_clicked(), event)
                    }
                    Element::RightClickedClosed => event_gate_close!(),

                    // Enter / leave a `canvas` world: bake the canvas's zoom into
                    // every spatial config in between. Runs even under `skip` so
                    // the stack always balances; `nesting_level` is untouched.
                    Element::CanvasWorldOpened { name } => {
                        canvas_scale_stack.push(canvas_scale);
                        if skip.is_none() {
                            canvas_scale = api.canvas_zoom(*name);
                        }
                    }
                    Element::CanvasWorldClosed => {
                        canvas_scale = canvas_scale_stack.pop().unwrap_or(1.0);
                    }

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
                                    &mut recursive_commands,
                                    reusables,
                                    Some(&merged_locals),
                                    Some((*src, index)),
                                    &mut item_config,
                                    &mut item_text_config,
                                    user_app,
                                    pointer,
                                    focus_target,
                                    canvas_scale,
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
                                &mut recursive_commands,
                                reusables,
                                Some(&merged_locals),
                                Some((*list, resolved_index)),
                                &mut item_config,
                                &mut item_text_config,
                                user_app,
                                pointer,
                                focus_target,
                                canvas_scale,
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
                    Element::ConfigOpened => {
                        nesting_level += 1;
                        if skip.is_none() {
                            *config = ElementConfiguration::default();
                            current_element_name = None;
                            current_element_floating = false;
                            gate_synthetic_id = None;
                        }
                    }
                    Element::ConfigClosed => {
                        nesting_level -= 1;
                        if skip.is_none() {
                            // An un-named floating element whose event gate had to
                            // synthesise an id: attach that same id here so the
                            // hit-test in `hovered!` lines up next frame.
                            if current_element_name.is_none()
                                && let Some(sym) = gate_synthetic_id
                            {
                                config.id(sym.as_str());
                            }
                            api.l.configure_element(config);
                            // Rule: only a *named* element can take focus, and
                            // only on the frame a mouse button went down while it
                            // was hovered. `hovered()` is false for anything a
                            // capture-mode floating element covers, and the
                            // outermost-first walk means the innermost (topmost)
                            // hovered named element writes last and wins.
                            if focusing
                                && let Some(name) = current_element_name
                                && api.l.hovered()
                            {
                                *focus_target = Some(name);
                            }
                        }
                    }

                    Element::TextElementOpened => nesting_level += 1,
                    Element::TextElementClosed(content) => {
                        nesting_level -= 1;
                        if skip.is_none() {
                            let text_content =
                                String::resolve_src(content, locals, user_app, &list_data);
                            // Inside a `canvas`, scale the whole text config by the
                            // canvas zoom - font size, line height *and* letter
                            // spacing together - so the (usually never-set,
                            // defaults-to-14) line height tracks the font instead
                            // of the lines scrunching / spreading as you zoom.
                            if canvas_scale != 1.0 {
                                let sc = |v: u16| {
                                    ((v as f32) * canvas_scale)
                                        .round()
                                        .clamp(0.0, u16::MAX as f32) as u16
                                };
                                text_config.font_size = sc(text_config.font_size);
                                text_config.line_height = sc(text_config.line_height);
                                text_config.letter_spacing = sc(text_config.letter_spacing);
                            }
                            // Copied into a frame arena: the resolved slice may
                            // borrow a per-list / per-reusable clone of the
                            // commands that is dropped before `end_layout` reads
                            // clay's stored pointer.
                            api.add_layout_text(text_content, text_config);
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
                                    &mut recursive_commands,
                                    reusables,
                                    Some(&merged_locals),
                                    None,
                                    config,
                                    text_config,
                                    user_app,
                                    pointer,
                                    focus_target,
                                    canvas_scale,
                                );
                            }
                        }
                    }

                    Element::FunctionCall(name) => {
                        if skip.is_none() {
                            user_app.dispatch_custom_element(name, api);
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
                    // The element's `` `element` *name* `` lands here as a static
                    // `Config::Id`; remember it (interned) so `ConfigClosed` and
                    // the `focus` gate can key focus off the name.
                    if let Config::Id(DataSrc::Static(name)) = &*config_command {
                        current_element_name = Some(GlobalSymbol::new(name.as_str()));
                    }
                    if matches!(&*config_command, Config::Floating) {
                        current_element_floating = true;
                    }
                    execute_config(
                        config_command,
                        config,
                        text_config,
                        reusables,
                        locals,
                        &list_data,
                        api,
                        user_app,
                        canvas_scale,
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
    // Spatial-config multiplier from the enclosing `` `canvas` `` (`1.0`
    // otherwise). Applied to lengths only - never to ratios, percentages,
    // z-index, colors or booleans.
    canvas_scale: f32,
) where
    UserApp: LayoutRunnerReflection,
{
    // `s` scales an `f32` length; `su16` scales a `u16` length (padding, gap,
    // border, font metrics) and rounds back.
    let s = canvas_scale;
    let su16 = |v: u16| ((v as f32) * s).round().clamp(0.0, u16::MAX as f32) as u16;
    match config_command {
        Config::Id(id) => {
            if let DataSrc::Static(id) = id {
                config.id(id.as_str());
            }
        }
        Config::IdIndexed(id) => {
            if let DataSrc::Static(id) = id
                && let Some((_, index)) = list_data
            {
                config.id_indexed(id.as_str(), *index as u32);
            }
        }
        Config::FitX => {
            config.width_fit();
        }
        Config::FitXmin(min) => {
            config.width_fit_min(f32::resolve_src(min, locals, user_app, list_data) * s);
        }
        Config::FitXmax(max) => {
            config.width_fit_min_max(0.0, f32::resolve_src(max, locals, user_app, list_data) * s);
        }
        Config::FitXminmax { min, max } => {
            config.width_fit_min_max(
                f32::resolve_src(min, locals, user_app, list_data) * s,
                f32::resolve_src(max, locals, user_app, list_data) * s,
            );
        }
        Config::FitY => {
            config.height_fit();
        }
        Config::FitYmin(min) => {
            config.height_fit_min(f32::resolve_src(min, locals, user_app, list_data) * s);
        }
        Config::FitYmax(max) => {
            config.height_fit_min_max(0.0, f32::resolve_src(max, locals, user_app, list_data) * s);
        }
        Config::FitYminmax { min, max } => {
            config.height_fit_min_max(
                f32::resolve_src(min, locals, user_app, list_data) * s,
                f32::resolve_src(max, locals, user_app, list_data) * s,
            );
        }
        Config::GrowX => {
            config.width_grow();
        }
        Config::GrowXmin(min) => {
            config.width_grow_min(f32::resolve_src(min, locals, user_app, list_data) * s);
        }
        Config::GrowXmax(max) => {
            config.width_grow_min_max(0.0, f32::resolve_src(max, locals, user_app, list_data) * s);
        }
        Config::GrowXminmax { min, max } => {
            config.width_grow_min_max(
                f32::resolve_src(min, locals, user_app, list_data) * s,
                f32::resolve_src(max, locals, user_app, list_data) * s,
            );
        }
        Config::GrowY => {
            config.height_grow();
        }
        Config::GrowYmin(min) => {
            config.height_grow_min(f32::resolve_src(min, locals, user_app, list_data) * s);
        }
        Config::GrowYmax(max) => {
            config.height_grow_min_max(0.0, f32::resolve_src(max, locals, user_app, list_data) * s);
        }
        Config::GrowYminmax { min, max } => {
            config.height_grow_min_max(
                f32::resolve_src(min, locals, user_app, list_data) * s,
                f32::resolve_src(max, locals, user_app, list_data) * s,
            );
        }
        Config::FixedX(size) => {
            config.width_fixed(f32::resolve_src(size, locals, user_app, list_data) * s);
        }
        Config::FixedY(size) => {
            config.height_fixed(f32::resolve_src(size, locals, user_app, list_data) * s);
        }
        Config::PercentX(size) => {
            // not scaled: a fraction of the (already-scaled) parent.
            config.width_percent(f32::resolve_src(size, locals, user_app, list_data));
        }
        Config::PercentY(size) => {
            // not scaled: a fraction of the (already-scaled) parent.
            config.height_percent(f32::resolve_src(size, locals, user_app, list_data));
        }
        Config::GrowAll => {
            config.grow();
        }
        Config::FitAll => {
            config.fit();
        }
        Config::AspectRatio(ratio) => {
            // not scaled: a ratio.
            config.aspect_ratio(f32::resolve_src(ratio, locals, user_app, list_data));
        }
        Config::FixedSquare(size) => {
            config.fixed_square(f32::resolve_src(size, locals, user_app, list_data) * s);
        }
        Config::Horizontal => {
            config.horizontal();
        }
        Config::PaddingAll(padding) => {
            config.padding_all(su16(u16::resolve_src(padding, locals, user_app, list_data)));
        }
        Config::PaddingTop(padding) => {
            config.padding_top(su16(u16::resolve_src(padding, locals, user_app, list_data)));
        }
        Config::PaddingBottom(padding) => {
            config.padding_bottom(su16(u16::resolve_src(padding, locals, user_app, list_data)));
        }
        Config::PaddingLeft(padding) => {
            config.padding_left(su16(u16::resolve_src(padding, locals, user_app, list_data)));
        }
        Config::PaddingRight(padding) => {
            config.padding_right(su16(u16::resolve_src(padding, locals, user_app, list_data)));
        }
        Config::Vertical => {
            config.vertical();
        }
        Config::ChildGap(gap) => {
            config.child_gap(su16(u16::resolve_src(gap, locals, user_app, list_data)));
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

        Config::CustomElement(spec) => {
            // Resolve every `DataSrc<f32>` parameter for this frame and stash the
            // plain `CustomElement` in `api`'s per-frame arena so its address
            // stays put until the render pass reads it (same as `Config::Image`).
            let mut resolved = spec.resolve(locals, user_app, list_data);
            if s != 1.0 {
                // Stroke widths are logical px; the 0..1 positional fields track
                // the (already-scaled) bounding box, so only `thickness` scales.
                match &mut resolved {
                    CustomElement::Ring { thickness }
                    | CustomElement::Line { thickness, .. }
                    | CustomElement::Arc { thickness, .. }
                    | CustomElement::Bezier { thickness, .. } => *thickness *= s,
                    _ => {}
                }
            }
            config.custom_element(api.stage_frame_shape(resolved));
        }
        Config::Shaders(specs) => {
            // Resolve this frame's effect params and stash the `ResolvedShader`
            // list in `api`'s per-frame arena, carried on the element's clay
            // `userData` (a different slot from `custom_element` above).
            let resolved: Vec<ResolvedShader> = specs
                .iter()
                .map(|spec| spec.resolve(locals, user_app, list_data))
                .collect();
            config.custom_layout_settings(
                api.stage_frame_layout_settings(crate::CustomLayoutSettings::Effects(resolved)),
            );
        }
        Config::RadiusAll(radius) => {
            config.radius_all(f32::resolve_src(radius, locals, user_app, list_data) * s);
        }
        Config::RadiusTopLeft(radius) => {
            config.radius_top_left(f32::resolve_src(radius, locals, user_app, list_data) * s);
        }
        Config::RadiusTopRight(radius) => {
            config.radius_top_right(f32::resolve_src(radius, locals, user_app, list_data) * s);
        }
        Config::RadiusBottomRight(radius) => {
            config.radius_bottom_right(f32::resolve_src(radius, locals, user_app, list_data) * s);
        }
        Config::RadiusBottomLeft(radius) => {
            config.radius_bottom_left(f32::resolve_src(radius, locals, user_app, list_data) * s);
        }
        Config::BorderColor(color) => {
            config.border_color(Color::resolve_src(color, locals, user_app, list_data));
        }
        Config::BorderAll(border) => {
            config.border_all(su16(u16::resolve_src(border, locals, user_app, list_data)));
        }
        Config::BorderTop(border) => {
            config.border_top(su16(u16::resolve_src(border, locals, user_app, list_data)));
        }
        Config::BorderBottom(border) => {
            config.border_bottom(su16(u16::resolve_src(border, locals, user_app, list_data)));
        }
        Config::BorderLeft(border) => {
            config.border_left(su16(u16::resolve_src(border, locals, user_app, list_data)));
        }
        Config::BorderRight(border) => {
            config.border_right(su16(u16::resolve_src(border, locals, user_app, list_data)));
        }
        Config::BorderBetweenChildren(border) => {
            config.border_in_between(su16(u16::resolve_src(border, locals, user_app, list_data)));
        }
        Config::Clip {
            vertical,
            horizontal,
        } => {
            let offset = api.l.get_scroll_offset();
            config
                .scroll(
                    bool::resolve_src(vertical, locals, user_app, list_data),
                    bool::resolve_src(horizontal, locals, user_app, list_data),
                )
                .scroll_child_offset(offset.x, offset.y);
        }
        Config::Canvas {
            name,
            world_width,
            world_height,
            min_zoom,
            max_zoom,
            initial_zoom,
            initial_pan_x,
            initial_pan_y,
        } => {
            let resolve_opt = |src: &Option<DataSrc<f32>>| {
                src.as_ref()
                    .map(|v| f32::resolve_src(v, locals, user_app, list_data))
            };
            let ww = resolve_opt(world_width);
            let wh = resolve_opt(world_height);
            let lo = resolve_opt(min_zoom);
            let hi = resolve_opt(max_zoom);
            let iz = resolve_opt(initial_zoom);
            let ipx = resolve_opt(initial_pan_x);
            let ipy = resolve_opt(initial_pan_y);

            let canvas = api.canvas_entry(*name);
            // Static bounds - safe to re-apply every frame.
            if let Some(w) = ww {
                canvas.world_width = Some(w);
            }
            if let Some(h) = wh {
                canvas.world_height = Some(h);
            }
            if lo.is_some() || hi.is_some() {
                canvas.set_zoom_limits(
                    lo.unwrap_or(canvas.min_zoom),
                    hi.unwrap_or(canvas.max_zoom),
                );
            }
            // `zoom` / `pan-*` are a one-time seed: applied the first time this
            // `canvas` element is laid out (even if app code pre-created the
            // `Canvas` in `onload` to set limits), never again - after that the
            // app owns pan/zoom.
            if !canvas.seeded {
                canvas.seeded = true;
                if let Some(z) = iz {
                    canvas.set_zoom(z);
                }
                if let Some(px) = ipx {
                    canvas.pan_x = px;
                }
                if let Some(py) = ipy {
                    canvas.pan_y = py;
                }
            }

            let (px, py) = api.canvas_pan_logical(*name);
            config.scroll(true, true).scroll_child_offset(px, py);
        }
        Config::CanvasWorldSize { name } => {
            let (ww, wh) = api.canvas_world_or_default(*name);
            config.width_fixed(ww * s).height_fixed(wh * s);
        }
        Config::Image { name } => {
            if let Some(image) = UIImageDescriptor::resolve_name(name, locals, user_app, list_data)
            {
                // `image` is borrowed from either the app struct or a
                // `set-image` local; copy it into `api`'s per-frame image arena
                // so it stays put until the render pass reads it, no matter
                // which of those (or a `use`-cloned reusable) it came from.
                let image = image.clone();
                config.image(api.stage_frame_image(image));
            }
        }
        Config::ImageLiteral(descriptor) => {
            let descriptor = descriptor.clone();
            config.image(api.stage_frame_image(descriptor));
        }
        Config::Floating => {
            config.floating();
        }
        Config::FloatingClipToParent => {
            config.floating_clip_to_attached_parent();
        }
        Config::FloatingNoClip => {
            config.floating_no_clip();
        }
        Config::FloatingPointerCapture => {
            config.floating_pointer_capture();
        }
        Config::FloatingOffset { x, y } => {
            config.floating_offset(
                f32::resolve_src(x, locals, user_app, list_data) * s,
                f32::resolve_src(y, locals, user_app, list_data) * s,
            );
        }
        Config::FloatingDimensions { width, height } => {
            config.floating_dimensions(
                f32::resolve_src(width, locals, user_app, list_data) * s,
                f32::resolve_src(height, locals, user_app, list_data) * s,
            );
        }
        Config::FloatingZIndex { z } => {
            // not scaled: stacking order.
            config.floating_z_index(i16::resolve_src(z, locals, user_app, list_data));
        }
        Config::FloatingAttatchToParentAtTopLeft => {
            config.floating_attach_parent_top_left();
        }
        Config::FloatingAttatchToParentAtCenterLeft => {
            config.floating_attach_parent_center_left();
        }
        Config::FloatingAttatchToParentAtBottomLeft => {
            config.floating_attach_parent_bottom_left();
        }
        Config::FloatingAttatchToParentAtTopCenter => {
            config.floating_attach_parent_top_center();
        }
        Config::FloatingAttatchToParentAtCenter => {
            config.floating_attach_parent_center();
        }
        Config::FloatingAttatchToParentAtBottomCenter => {
            config.floating_attach_parent_bottom_center();
        }
        Config::FloatingAttatchToParentAtTopRight => {
            config.floating_attach_parent_top_right();
        }
        Config::FloatingAttatchToParentAtCenterRight => {
            config.floating_attach_parent_center_right();
        }
        Config::FloatingAttatchToParentAtBottomRight => {
            config.floating_attach_parent_bottom_right();
        }
        Config::FloatingAttatchElementAtTopLeft => {
            config.floating_attach_self_top_left();
        }
        Config::FloatingAttatchElementAtCenterLeft => {
            config.floating_attach_self_center_left();
        }
        Config::FloatingAttatchElementAtBottomLeft => {
            config.floating_attach_self_bottom_left();
        }
        Config::FloatingAttatchElementAtTopCenter => {
            config.floating_attach_self_top_center();
        }
        Config::FloatingAttatchElementAtCenter => {
            config.floating_attach_self_center();
        }
        Config::FloatingAttatchElementAtBottomCenter => {
            config.floating_attach_self_bottom_center();
        }
        Config::FloatingAttatchElementAtTopRight => {
            config.floating_attach_self_top_right();
        }
        Config::FloatingAttatchElementAtCenterRight => {
            config.floating_attach_self_center_right();
        }
        Config::FloatingAttatchElementAtBottomRight => {
            config.floating_attach_self_bottom_right();
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
        Config::Use { name, params } => {
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
                // `use` body bindings shadow the caller's locals for the
                // duration of the replay, so `` `image` *icon* `` in the
                // snippet resolves a `` `set-image` *icon* `` passed here.
                let param_refs: HashMap<GlobalSymbol, &DataSrc<Declaration>> =
                    params.iter().map(|(n, v)| (*n, v)).collect();
                let merged;
                let use_locals = if params.is_empty() {
                    locals
                } else {
                    merged = merge_locals(locals, &param_refs);
                    Some(&merged)
                };
                for command in reusable {
                    if let Layout::Config(nested_command) = command {
                        let mut nested_command = nested_command.clone();
                        execute_config(
                            &mut nested_command,
                            config,
                            text_config,
                            reusables,
                            use_locals,
                            list_data,
                            api,
                            user_app,
                            s,
                        );
                    }
                }
            }
        }

        Config::AlignCenter => {
            text_config.align_center();
        }
        Config::AlignLeft => {
            text_config.align_left();
        }
        Config::AlignRight => {
            text_config.align_right();
        }
        Config::FontId(id) => {
            text_config.font_id(u16::resolve_src(id, locals, user_app, list_data));
        }
        Config::FontColor(color) => {
            text_config.font_color(Color::resolve_src(color, locals, user_app, list_data));
        }
        Config::FontSize(size) => {
            // Canvas zoom is applied to the whole TextConfig at `TextElementClosed`
            // (so the never-explicitly-set `line_height` default scales too), not
            // here.
            text_config.font_size(u16::resolve_src(size, locals, user_app, list_data));
        }
        Config::LineHeight(height) => {
            text_config.line_height(u16::resolve_src(height, locals, user_app, list_data));
        }
        Config::LetterSpacing(spacing) => {
            text_config.letter_spacing(u16::resolve_src(spacing, locals, user_app, list_data));
        }
        Config::WrapWords => {
            text_config.wrap_mode_words();
        }
        Config::WrapNewLines => {
            text_config.wrap_mode_new_lines();
        }
        Config::WrapNone => {
            text_config.wrap_mode_none();
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
        // Local declarations win over the app struct (same order as every other
        // resolver). A `set-image` local with a UV rect carries the descriptor
        // itself; a `set-image` alias / `get-image` local redirects to another
        // name, which may itself be another image binding - so follow the chain
        // (`icon` -> `connect_plc` -> descriptor), capped so a cycle can't hang
        // the layout. Whatever the chain ends on is looked up on the app struct.
        let mut current = *name;
        for _ in 0..16 {
            let Some(local) = locals.and_then(|locals| locals.get(&current)) else {
                break;
            };
            match local {
                DataSrc::Static(Declaration::Image(descriptor)) => return Some(descriptor),
                DataSrc::Dynamic(next) => current = *next,
                DataSrc::Static(_) => break,
            }
        }
        user_app.get_image(&current, list_data)
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

    /// `canvas` lowers to: an outer clip container carrying the user's own
    /// config + one `Config::Canvas`, then a `CanvasWorldOpened` ..
    /// `CanvasWorldClosed` pair wrapping a fixed-size world element
    /// (`Config::CanvasWorldSize`) that holds the body children. Canvas-only
    /// keywords are pulled onto `Config::Canvas`; every other keyword stays on
    /// the outer container.
    #[test]
    fn canvas_parses() {
        let src = "\
# root
- `canvas` board
    - `config`
        - `world-width` 4000
        - `world-height` 3000
        - `min-zoom` 0.25
        - `max-zoom` 8
        - `zoom` 2
        - `pan-x` 15
        - `color` grey
    - `element` node
        - `config`
            - `width-fixed` 120
"
        .to_string();
        let ParsedLayout { body, .. } = process_layout(src).expect("canvas should parse");

        // exactly one Config::Canvas, fully populated
        let canvas = body
            .iter()
            .find_map(|c| match c {
                Layout::Config(cfg @ Config::Canvas { .. }) => Some(cfg.clone()),
                _ => None,
            })
            .expect("a Config::Canvas");
        let Config::Canvas {
            name,
            world_width,
            world_height,
            min_zoom,
            max_zoom,
            initial_zoom,
            initial_pan_x,
            initial_pan_y,
        } = canvas
        else {
            unreachable!()
        };
        assert_eq!(name, GlobalSymbol::new("board"));
        assert_eq!(world_width, Some(DataSrc::Static(4000.0)));
        assert_eq!(world_height, Some(DataSrc::Static(3000.0)));
        assert_eq!(min_zoom, Some(DataSrc::Static(0.25)));
        assert_eq!(max_zoom, Some(DataSrc::Static(8.0)));
        assert_eq!(initial_zoom, Some(DataSrc::Static(2.0)));
        assert_eq!(initial_pan_x, Some(DataSrc::Static(15.0)));
        assert_eq!(initial_pan_y, None);

        // the plain `color` keyword still reached the outer container
        assert!(body.iter().any(|c| matches!(c, Layout::Config(Config::Color(_)))));

        // world-wrapper scaffolding, correctly paired and nested
        let world_open = body
            .iter()
            .position(|c| matches!(c, Layout::Element(Element::CanvasWorldOpened { .. })))
            .unwrap();
        let world_close = body
            .iter()
            .position(|c| matches!(c, Layout::Element(Element::CanvasWorldClosed)))
            .unwrap();
        assert!(world_open < world_close);
        assert!(body[world_open..world_close]
            .iter()
            .any(|c| matches!(c, Layout::Config(Config::CanvasWorldSize { .. }))));
        // the `node` child's config sits inside the world scope
        let node_fixed = body
            .iter()
            .position(|c| matches!(c, Layout::Config(Config::FixedX(_))))
            .unwrap();
        assert!(world_open < node_fixed && node_fixed < world_close);

        // well-nested overall
        let mut depth = 0i32;
        for c in &body {
            if let Layout::Element(e) = c {
                let n = format!("{e:?}");
                if n.contains("Opened") {
                    depth += 1;
                } else if n.contains("Closed") {
                    depth -= 1;
                }
            }
            assert!(depth >= 0);
        }
        assert_eq!(depth, 0);
    }

    /// A bare `` `canvas` `` (no config block) still emits the full scaffolding
    /// with an all-`None` `Config::Canvas`.
    #[test]
    fn canvas_bare_parses() {
        let src = "\
# root
- `canvas` board
    - `element` node
"
        .to_string();
        let ParsedLayout { body, .. } = process_layout(src).unwrap();
        assert!(body.iter().any(|c| matches!(
            c,
            Layout::Config(Config::Canvas {
                world_width: None,
                initial_zoom: None,
                ..
            })
        )));
        assert!(body
            .iter()
            .any(|c| matches!(c, Layout::Element(Element::CanvasWorldOpened { .. }))));
    }

    /// The parser should turn `examples/layouts/Main.md` into a non-empty,
    /// flattened command stream without panicking or erroring.
    #[test]
    fn parses_main_md() {
        let file = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/layouts/Main.md"
        ))
        .unwrap();
        let ParsedLayout {
            body, reusables, ..
        } = process_layout(file).expect("Main.md should parse");

        assert!(!body.is_empty());
        assert!(reusables.contains_key("layout expand"));
        assert!(reusables.contains_key("header button"));
        assert!(reusables.contains_key("drop down menu item"));

        // The command stream must be well-nested: every *Opened has a matching
        // *Closed and the stream never dips below zero depth.
        let mut depth: i32 = 0;
        for command in &body {
            if let Layout::Element(element) = command {
                let name = format!("{element:?}");
                if name.contains("Opened") {
                    depth += 1;
                } else if name.contains("Closed") {
                    depth -= 1;
                }
            }
            assert!(depth >= 0, "command stream closed more than it opened");
        }
        assert_eq!(depth, 0, "command stream left something unclosed");

        // `*pad*` in Main.md is `scale(base, base)` with `base = 4` and the
        // header `calc` `scale(a, b) = a * b`, so it must fold to 16.
        let pad = GlobalSymbol::new("pad");
        assert!(body.iter().any(|command| matches!(
            command,
            Layout::Declaration { name, value: DataSrc::Static(Declaration::Numeric(v)) }
                if *name == pad && *v == 16.0
        )));
    }

    /// Pulls the value bound to a `set-numeric` declaration named `name` out of
    /// a parsed body, or `None` if the declaration was dropped.
    fn declared_number(body: &[Layout], name: &str) -> Option<f32> {
        let want = GlobalSymbol::new(name);
        body.iter().find_map(|command| match command {
            Layout::Declaration {
                name,
                value: DataSrc::Static(Declaration::Numeric(v)),
            } if *name == want => Some(*v),
            _ => None,
        })
    }

    #[test]
    fn set_numeric_expression_folds_to_constant() {
        let src = "\
# root
- `declarations`
    - `set-numeric` *a* 5
    - `set-numeric` *b* 2
    - `set-numeric` *c* `(a + b) * 3`
";
        let body = process_layout(src.to_string()).expect("should parse").body;
        assert_eq!(declared_number(&body, "c"), Some(21.0));
        // a plain literal still works, backticked or not
        assert_eq!(declared_number(&body, "a"), Some(5.0));
    }

    #[test]
    fn calc_fn_from_header_is_callable() {
        let src = "\
#### TML 1.0
- `calc` `scale(a, b) = a * b`

# root
- `declarations`
    - `set-numeric` *x* `scale(3, 4)`
    - `set-numeric` *y* `scale(scale(1, 1), 3)`
";
        let body = process_layout(src.to_string()).expect("should parse").body;
        assert_eq!(declared_number(&body, "x"), Some(12.0));
        assert_eq!(declared_number(&body, "y"), Some(3.0));
    }

    #[test]
    fn header_after_body_still_provides_calc_fns() {
        // `collect_calc_fns` pre-scans, so header position doesn't matter.
        let src = "\
# root
- `declarations`
    - `set-numeric` *x* `dbl(21)`

#### TML 1.0
- `calc` `dbl(n) = n * 2`
";
        let body = process_layout(src.to_string()).expect("should parse").body;
        assert_eq!(declared_number(&body, "x"), Some(42.0));
    }

    #[test]
    fn forward_and_runtime_references_are_dropped() {
        let src = "\
# root
- `declarations`
    - `set-numeric` *forward* `later + 1`
    - `set-numeric` *later* 2
    - `get-numeric` *runtime* some_field
    - `set-numeric` *bad* `runtime * 2`
";
        let body = process_layout(src.to_string()).expect("should parse").body;
        assert_eq!(declared_number(&body, "forward"), None);
        assert_eq!(declared_number(&body, "later"), Some(2.0));
        assert_eq!(declared_number(&body, "bad"), None);
    }

    #[test]
    fn recursive_calc_fn_is_dropped_not_hung() {
        let src = "\
#### TML 1.0
- `calc` `r(n) = r(n)`

# root
- `declarations`
    - `set-numeric` *x* `r(1)`
";
        let body = process_layout(src.to_string()).expect("should parse").body;
        assert_eq!(declared_number(&body, "x"), None);
    }

    #[test]
    fn calc_scope_is_per_list() {
        // A `list` gets its own frame; its leading declarations don't leak out,
        // but they do see the enclosing page scope.
        let src = "\
# root
- `declarations`
    - `set-numeric` *page_base* 10
- `list` rows
    - `declarations`
        - `set-numeric` *row_pad* `page_base + 2`
    - `element`
";
        let body = process_layout(src.to_string()).expect("should parse").body;
        assert_eq!(declared_number(&body, "row_pad"), Some(12.0));
    }

    /// Every config keyword added to expose an `ElementConfiguration` /
    /// `TextConfig` builder should parse into its `Config` variant.
    #[test]
    fn newly_exposed_config_keywords_parse() {
        let src = "\
# root
- `element`
    - `config`
        - `fit`
        - `horizontal`
        - `aspect-ratio` 1.5
        - `fixed-square` 24
        - `width-fixed` 10
        - `height-fixed` 20
        - `id-indexed` row
        - `scroll-horizontal`
        - `scroll-vertical`
        - `width-grow-min` 30
        - `width-grow-max` 90
        - `height-grow-min` 15
        - `floating`
            - `clip-to-parent`
            - `no-clip`
            - `pointer-capture`
            - `pointer-pass-through`
            - `attach-root`
            - `z-index` 5
            - `offset-x` 4
            - `offset-y` 8
            - `floating-dimensions-width` 100
            - `floating-dimensions-height` 50
    - `text`
        - `config`
            - `letter-spacing` 2
            - `wrap` none
        - hello
";
        let body = process_layout(src.to_string())
            .expect("should parse")
            .body;

        let configs: Vec<&Config> = body
            .iter()
            .filter_map(|c| match c {
                Layout::Config(c) => Some(c),
                _ => None,
            })
            .collect();

        let has = |pred: &dyn Fn(&Config) -> bool| configs.iter().any(|c| pred(c));

        assert!(has(&|c| matches!(c, Config::FitAll)));
        assert!(has(&|c| matches!(c, Config::Horizontal)));
        assert!(has(&|c| matches!(c, Config::AspectRatio(DataSrc::Static(r)) if *r == 1.5)));
        assert!(has(&|c| matches!(c, Config::FixedSquare(DataSrc::Static(s)) if *s == 24.0)));
        assert!(has(&|c| matches!(c, Config::FixedX(DataSrc::Static(w)) if *w == 10.0)));
        assert!(has(&|c| matches!(c, Config::FixedY(DataSrc::Static(h)) if *h == 20.0)));
        assert!(has(&|c| matches!(c, Config::IdIndexed(_))));
        // split sizing keywords fold back into one config per axis
        assert!(has(&|c| matches!(
            c,
            Config::GrowXminmax {
                min: DataSrc::Static(min),
                max: DataSrc::Static(max),
            } if *min == 30.0 && *max == 90.0
        )));
        assert!(has(&|c| matches!(c, Config::GrowYmin(DataSrc::Static(m)) if *m == 15.0)));
        assert!(has(&|c| matches!(
            c,
            Config::FloatingOffset {
                x: DataSrc::Static(x),
                y: DataSrc::Static(y),
            } if *x == 4.0 && *y == 8.0
        )));
        assert!(has(&|c| matches!(
            c,
            Config::Clip {
                vertical: DataSrc::Static(true),
                horizontal: DataSrc::Static(true),
            }
        )));
        assert!(has(&|c| matches!(c, Config::FloatingClipToParent)));
        assert!(has(&|c| matches!(c, Config::FloatingNoClip)));
        assert!(has(&|c| matches!(c, Config::FloatingPointerCapture)));
        assert!(has(&|c| matches!(c, Config::FloatingPointerPassThrough)));
        assert!(has(&|c| matches!(c, Config::FloatingAttachElementToRoot)));
        assert!(has(&|c| matches!(c, Config::FloatingZIndex { z: DataSrc::Static(5) })));
        assert!(has(&|c| matches!(
            c,
            Config::FloatingDimensions {
                width: DataSrc::Static(w),
                height: DataSrc::Static(h),
            } if *w == 100.0 && *h == 50.0
        )));
        assert!(has(&|c| matches!(c, Config::LetterSpacing(DataSrc::Static(2)))));
        assert!(has(&|c| matches!(c, Config::WrapNone)));
    }

    /// The drawn-shape elements (`line`/`arc`/`ring`/`bezier`) parse their
    /// geometry keywords onto a `CustomElementSpec`, including the `from`/`to`
    /// anchor words and a dynamic `*binding*` parameter.
    #[test]
    fn shape_elements_parse() {
        let src = "\
# root
- `element`
    - `config`
        - `grow`
    - `line`
        - `config`
            - `from` top-left
            - `to` bottom-right
            - `thickness` 3
    - `arc`
        - `config`
            - `start-angle` 45
            - `end-angle` 270
            - `radius` 0.8
            - `thickness` *stroke*
    - `ring`
    - `bezier`
        - `config`
            - `ctrl1-x` 0.1
            - `to-y` 0.25
";
        let body = process_layout(src.to_string()).expect("should parse").body;
        let specs: Vec<&CustomElementSpec> = body
            .iter()
            .filter_map(|c| match c {
                Layout::Config(Config::CustomElement(s)) => Some(s),
                _ => None,
            })
            .collect();
        assert_eq!(specs.len(), 4, "one spec per shape element");

        assert!(specs.iter().any(|s| matches!(
            s,
            CustomElementSpec::Line {
                from_x: DataSrc::Static(fx),
                from_y: DataSrc::Static(fy),
                to_x: DataSrc::Static(tx),
                to_y: DataSrc::Static(ty),
                thickness: DataSrc::Static(t),
            } if *fx == 0.0 && *fy == 0.0 && *tx == 1.0 && *ty == 1.0 && *t == 3.0
        )));
        assert!(specs.iter().any(|s| matches!(
            s,
            CustomElementSpec::Arc {
                start_angle: DataSrc::Static(sa),
                end_angle: DataSrc::Static(ea),
                radius: DataSrc::Static(r),
                thickness: DataSrc::Dynamic(_),
                ..
            } if *sa == 45.0 && *ea == 270.0 && *r == 0.8
        )));
        assert!(specs.iter().any(|s| matches!(
            s,
            CustomElementSpec::Ring {
                thickness: DataSrc::Static(t)
            } if *t == 1.0
        )));
        assert!(specs.iter().any(|s| matches!(
            s,
            CustomElementSpec::Bezier {
                ctrl1_x: DataSrc::Static(c),
                to_y: DataSrc::Static(ty),
                ..
            } if *c == 0.1 && *ty == 0.25
        )));
    }

    /// A drawn shape can nest children - another shape, or a plain `element` -
    /// each opened/closed like a normal container (an `arc` inside an `arc` is
    /// how an RPM gauge is drawn).
    #[test]
    fn shape_elements_nest_children() {
        let src = "\
# root
- `arc` outer
    - `config`
        - `grow`
        - `end-angle` 300
    - `arc` inner
        - `config`
            - `grow`
            - `end-angle` 220
        - `circle` hub
            - `config`
                - `fixed-square` 10
";
        let body = process_layout(src.to_string()).expect("should parse").body;

        // three shapes: outer arc, inner arc, hub circle
        let specs: Vec<&CustomElementSpec> = body
            .iter()
            .filter_map(|c| match c {
                Layout::Config(Config::CustomElement(s)) => Some(s),
                _ => None,
            })
            .collect();
        assert_eq!(specs.len(), 3);
        assert_eq!(
            specs
                .iter()
                .filter(|s| matches!(s, CustomElementSpec::Arc { .. }))
                .count(),
            2
        );
        assert!(specs.iter().any(|s| matches!(s, CustomElementSpec::Circle)));

        // balanced open/close, and the outer shape's `CustomElement` config is
        // emitted before its children's opens (parent drawn first).
        let kinds: Vec<&Layout> = body.iter().collect();
        let outer_custom = kinds
            .iter()
            .position(|c| matches!(c, Layout::Config(Config::CustomElement(_))))
            .unwrap();
        let inner_open = kinds
            .iter()
            .enumerate()
            .filter(|(_, c)| matches!(c, Layout::Element(Element::ElementOpened { .. })))
            .nth(1)
            .map(|(i, _)| i)
            .unwrap();
        assert!(outer_custom < inner_open);
        assert_eq!(
            body.iter()
                .filter(|c| matches!(c, Layout::Element(Element::ElementOpened { .. })))
                .count(),
            body.iter()
                .filter(|c| matches!(c, Layout::Element(Element::ElementClosed)))
                .count()
        );
    }

    /// `render-window` parses like a shape: camera name = element name (un-named
    /// -> `default`), and each camera keyword becomes an `Option<DataSrc<f32>>`
    /// override (dynamic when written as `*field*`).
    #[test]
    fn render_window_parses() {
        let src = "\
# root
- `render-window` cockpit
    - `config`
        - `grow`
        - `eye-z` *cam_z*
        - `fov` 60
- `render-window`
    - `config`
        - `grow`
";
        let body = process_layout(src.to_string()).expect("should parse").body;
        let specs: Vec<&CustomElementSpec> = body
            .iter()
            .filter_map(|c| match c {
                Layout::Config(Config::CustomElement(s)) => Some(s),
                _ => None,
            })
            .collect();
        assert_eq!(specs.len(), 2);

        assert!(specs.iter().any(|s| matches!(
            s,
            CustomElementSpec::RenderWindow {
                camera,
                eye_z: Some(DataSrc::Dynamic(_)),
                fov: Some(DataSrc::Static(f)),
                eye_x: None,
                ..
            } if camera.as_str() == "cockpit" && *f == 60.0
        )));
        // un-named render-window -> the shared `default` camera, no overrides
        assert!(specs.iter().any(|s| matches!(
            s,
            CustomElementSpec::RenderWindow { camera, fov: None, .. }
                if camera.as_str() == "default"
        )));
    }

    /// The `shapes` example layout parses; its RPM-gauge cell nests an `arc`
    /// (and a `circle` hub) inside an `arc`, so shape elements carry children.
    #[test]
    fn shapes_example_parses() {
        let file = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/layouts/shapes.md"
        ))
        .unwrap();
        // fully backticked already - the pre-pass must not touch it
        assert_eq!(add_missing_keyword_backticks(&file), file);

        let parsed = process_layout(file).expect("shapes.md should parse");
        let specs: Vec<&CustomElementSpec> = parsed
            .body
            .iter()
            .filter_map(|c| match c {
                Layout::Config(Config::CustomElement(s)) => Some(s),
                _ => None,
            })
            .collect();
        let count = |pred: &dyn Fn(&CustomElementSpec) -> bool| {
            specs.iter().filter(|s| pred(s)).count()
        };
        assert_eq!(count(&|s| matches!(s, CustomElementSpec::Line { .. })), 1);
        assert_eq!(count(&|s| matches!(s, CustomElementSpec::Arc { .. })), 2, "gauge = arc-in-arc");
        assert_eq!(count(&|s| matches!(s, CustomElementSpec::Ring { .. })), 1);
        assert_eq!(count(&|s| matches!(s, CustomElementSpec::Bezier { .. })), 1);
        assert_eq!(count(&|s| matches!(s, CustomElementSpec::Circle)), 2, "shape circle + gauge hub");
        // the inner gauge arc's end-angle is bound to `*gauge_end*`
        assert!(specs.iter().any(|s| matches!(
            s,
            CustomElementSpec::Arc { end_angle: DataSrc::Dynamic(_), .. }
        )));

        // The gauge arc's child render commands (inner arc, hub) sit between its
        // own open and close - i.e. it is a container, not a leaf.
        let opens = parsed
            .body
            .iter()
            .filter(|c| matches!(c, Layout::Element(Element::ElementOpened { .. })))
            .count();
        let closes = parsed
            .body
            .iter()
            .filter(|c| matches!(c, Layout::Element(Element::ElementClosed)))
            .count();
        assert_eq!(opens, closes);
    }

    /// The `scene` example layout (bare, non-backticked style) parses to two
    /// `render-window` panes - `orbit` (app-driven) and `front` (layout keywords,
    /// `eye-z` bound dynamically).
    #[test]
    fn scene_example_parses() {
        let file = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/layouts/scene.md"
        ))
        .unwrap();
        let parsed = process_layout(file).expect("scene.md should parse");
        let windows: Vec<&CustomElementSpec> = parsed
            .body
            .iter()
            .filter_map(|c| match c {
                Layout::Config(Config::CustomElement(s @ CustomElementSpec::RenderWindow { .. })) => {
                    Some(s)
                }
                _ => None,
            })
            .collect();
        assert_eq!(windows.len(), 2);
        assert!(windows.iter().any(|s| matches!(
            s,
            CustomElementSpec::RenderWindow { camera, .. } if camera.as_str() == "orbit"
        )));
        assert!(windows.iter().any(|s| matches!(
            s,
            CustomElementSpec::RenderWindow {
                camera,
                eye_z: Some(DataSrc::Dynamic(_)),
                ortho_height: Some(DataSrc::Static(_)),
                ..
            } if camera.as_str() == "front"
        )));
    }

    /// `parse_uv_rect` accepts a bracketed 4-float list with arbitrary spacing
    /// and rejects anything else.
    #[test]
    fn uv_rect_parsing() {
        assert_eq!(parse_uv_rect("[0, 0, 1, 1]"), Some([0.0, 0.0, 1.0, 1.0]));
        assert_eq!(parse_uv_rect("[0,0,1,1]"), Some([0.0, 0.0, 1.0, 1.0]));
        assert_eq!(
            parse_uv_rect("  [0.25, 0.5 , 0.75,1.0]  "),
            Some([0.25, 0.5, 0.75, 1.0])
        );
        assert_eq!(parse_uv_rect("[0, 0, 1]"), None);
        assert_eq!(parse_uv_rect("[0, 0, 1, 1, 1]"), None);
        assert_eq!(parse_uv_rect("0, 0, 1, 1"), None);
        assert_eq!(parse_uv_rect("[a, b, c, d]"), None);
    }

    /// The header `` `load` `` directive, the `set-image` declaration, and both
    /// `image` config shapes should all parse the way `Image Viewer.md` uses
    /// them.
    #[test]
    fn image_loading_syntax_parses() {
        let src = "\
#### TML 1.0
- `load` [pic](examples/pic.jpg)

# root
- `declarations`
  - `set-image` *family* *pic* [0, 0, 0.5, 1]
- `element`
  - `config`
    - `image` *family*
  - `element`
    - `config`
      - `image` *pic* [0, 0, 1, 1]
  - `element`
    - `config`
      - `image` from_the_app
";
        let parsed = process_layout(src.to_string()).expect("should parse");

        assert_eq!(
            parsed.image_loads,
            vec![ImageLoad {
                atlas: "pic".to_string(),
                path: "examples/pic.jpg".to_string(),
            }]
        );

        // `set-image` -> a page-level declaration carrying the descriptor.
        let declared = parsed.body.iter().find_map(|c| match c {
            Layout::Declaration {
                name,
                value: DataSrc::Static(Declaration::Image(descriptor)),
            } if name.as_str() == "family" => Some(descriptor),
            _ => None,
        });
        assert_eq!(
            declared,
            Some(&UIImageDescriptor {
                atlas: "pic",
                u1: 0.0,
                v1: 0.0,
                u2: 0.5,
                v2: 1.0,
            })
        );

        let configs: Vec<&Config> = parsed
            .body
            .iter()
            .filter_map(|c| match c {
                Layout::Config(c) => Some(c),
                _ => None,
            })
            .collect();

        // `image` *family* -> a name lookup.
        assert!(
            configs
                .iter()
                .any(|c| matches!(c, Config::Image { name } if name.as_str() == "family"))
        );
        // `image` from_the_app -> a bare name lookup (still hits get_image).
        assert!(
            configs
                .iter()
                .any(|c| matches!(c, Config::Image { name } if name.as_str() == "from_the_app"))
        );
        // `image` *pic* [..] -> an inline literal.
        assert!(configs.iter().any(|c| matches!(
            c,
            Config::ImageLiteral(UIImageDescriptor { atlas, u2, .. })
            if *atlas == "pic" && *u2 == 1.0
        )));
    }

    /// Dynamic names are interned verbatim at parse time - no case folding, no
    /// space/hyphen/underscore equivalence. A `set-color` target, its `color`
    /// reference and a `list` name all keep exactly the spelling written, and
    /// only an exact match resolves.
    #[test]
    fn dynamic_names_are_interned_verbatim() {
        let want = GlobalSymbol::new("content background color");
        let src = "\
# root
- `declarations`
    - `set-color` *content background color* rgb(1,2,3)
- `element`
    - `config`
        - `color` *content background color*
- `list` content background color
    - `element`
";
        let body = process_layout(src.to_string()).expect("should parse").body;

        // the `set-color` declaration's own name - kept verbatim
        assert!(body.iter().any(|c| matches!(
            c,
            Layout::Declaration { name, .. } if *name == want
        )));
        // the `color` config that references it - same spelling, resolves
        assert!(body.iter().any(|c| matches!(
            c,
            Layout::Config(Config::Color(DataSrc::Dynamic(s))) if *s == want
        )));
        // the `list` name
        assert!(body.iter().any(|c| matches!(
            c,
            Layout::Element(Element::ListClosed(s)) if *s == want
        )));

        // A differently-cased / punctuated spelling is a *different* symbol.
        let hyphenated = GlobalSymbol::new("content-background-color");
        assert!(!body.iter().any(|c| matches!(
            c,
            Layout::Config(Config::Color(DataSrc::Dynamic(s))) if *s == hyphenated
        )));

        // `parse_index_arg` (used by `if-index` / `item`) keeps names verbatim too.
        assert!(matches!(
            parse_index_arg("My Index"),
            DataSrc::Dynamic(s) if s == GlobalSymbol::new("My Index")
        ));
    }

    /// `` `image` *name* `` interns its lookup name verbatim, the same as the
    /// `` `set-image` `` declaration and app `get_image` fields, so an exact
    /// spelling resolves and only an exact spelling.
    #[test]
    fn image_config_name_is_verbatim() {
        let want = GlobalSymbol::new("left half");
        let src = "\
#### TML 1.0
- `load` [pic](examples/pic.jpg)

# root
- `declarations`
    - `set-image` *left half* *pic* [0, 0, 0.5, 1]
- `element`
    - `config`
        - `image` *left half*
";
        let body = process_layout(src.to_string()).expect("should parse").body;

        assert!(body.iter().any(|c| matches!(
            c,
            Layout::Declaration { name, .. } if *name == want
        )));
        assert!(body.iter().any(|c| matches!(
            c,
            Layout::Config(Config::Image { name }) if *name == want
        )));
    }

    /// `` `set-image` *name* *source* `` with **no** UV rect is an alias: it
    /// parses to a `DataSrc::Dynamic` (like `get-image`), not a literal
    /// descriptor with `source` as the atlas.
    #[test]
    fn set_image_without_uv_is_an_alias() {
        let src = "\
# root
- `declarations`
    - `set-image` *base* *icon_atlas* [0.04, 0, 0.08, 1]
    - `set-image` *icon* *base*
";
        let body = process_layout(src.to_string()).expect("should parse").body;

        // literal (has a UV rect)
        assert!(body.iter().any(|c| matches!(
            c,
            Layout::Declaration {
                name,
                value: DataSrc::Static(Declaration::Image(d)),
            } if name.as_str() == "base" && d.atlas == "icon_atlas"
        )));
        // alias (no UV rect) - points at `base`, does not become atlas "base"
        assert!(body.iter().any(|c| matches!(
            c,
            Layout::Declaration { name, value: DataSrc::Dynamic(src) }
                if name.as_str() == "icon" && src.as_str() == "base"
        )));
    }

    /// `UIImageDescriptor::resolve_name` follows an alias chain through locals:
    /// `icon` -> (alias) `connect_plc` -> descriptor. This is the runtime half
    /// of `` `set-image` *icon* *connect_plc* `` passed into a `use`.
    #[test]
    fn image_alias_chain_resolves() {
        struct NoApp;
        impl LayoutRunnerReflection for NoApp {}

        let desc = UIImageDescriptor { atlas: "icon_atlas", u1: 0.04, v1: 0.0, u2: 0.08, v2: 1.0 };
        let connect: DataSrc<Declaration> = DataSrc::Static(Declaration::Image(desc.clone()));
        let icon: DataSrc<Declaration> = DataSrc::Dynamic(GlobalSymbol::new("connect_plc"));
        let indirect: DataSrc<Declaration> = DataSrc::Dynamic(GlobalSymbol::new("icon"));

        let mut locals: HashMap<GlobalSymbol, &DataSrc<Declaration>> = HashMap::new();
        locals.insert(GlobalSymbol::new("connect_plc"), &connect);
        locals.insert(GlobalSymbol::new("icon"), &icon);
        locals.insert(GlobalSymbol::new("indirect"), &indirect);

        let app = NoApp;
        let got = <UIImageDescriptor as ResolveValue<'_, '_, NoApp>>::resolve_name(
            &GlobalSymbol::new("indirect"),
            Some(&locals),
            &app,
            &None,
        );
        assert_eq!(got, Some(&desc));

        // an alias that dead-ends (no binding, no app field) resolves to nothing
        let dangling: DataSrc<Declaration> = DataSrc::Dynamic(GlobalSymbol::new("missing"));
        locals.insert(GlobalSymbol::new("dangling"), &dangling);
        let got = <UIImageDescriptor as ResolveValue<'_, '_, NoApp>>::resolve_name(
            &GlobalSymbol::new("dangling"),
            Some(&locals),
            &app,
            &None,
        );
        assert_eq!(got, None);
    }

    /// A `set-image` passed as a `use` parameter to a `### element` reusable
    /// lands in the command stream as a `Layout::Declaration` between
    /// `UseOpened` and `UseClosed`, so the runner can merge it into the
    /// snippet's locals and `` `image` *icon* `` inside the snippet resolves it.
    #[test]
    fn set_image_use_parameter_reaches_reusable() {
        let src = "\
#### TML 1.0
- `load` [pic](examples/pic.jpg)

### model-control
- `element`
    - `config`
        - `image` *icon*

# root
- `element`
    - `config`
        - `vertical`
    - `use` model-control
        - `set-image` *icon* *pic* [0, 0, 1, 1]
";
        let parsed = process_layout(src.to_string()).expect("should parse");

        // the reusable stores an `image *icon*` lookup
        let reusable = parsed
            .reusables
            .get("model-control")
            .expect("reusable registered");
        assert!(reusable.iter().any(|c| matches!(
            c,
            Layout::Config(Config::Image { name }) if name.as_str() == "icon"
        )));

        // the call site emits the parameter as a declaration inside the use block
        let icon = GlobalSymbol::new("icon");
        let mut depth = 0i32;
        let mut decl_in_use = false;
        for c in &parsed.body {
            match c {
                Layout::Element(Element::UseOpened) => depth += 1,
                Layout::Element(Element::UseClosed(_)) => depth -= 1,
                Layout::Declaration { name, value: DataSrc::Static(Declaration::Image(d)) }
                    if depth > 0 && *name == icon =>
                {
                    assert_eq!(d.atlas, "pic");
                    decl_in_use = true;
                }
                _ => {}
            }
        }
        assert!(decl_in_use, "set-image parameter should sit inside the use block");
    }

    /// A `## config` snippet invoked with `` `use` `` can take a parameter
    /// body (`set-*` / `get-*` bindings), captured on `Config::Use` so the
    /// runner can apply them as locals while the snippet replays.
    #[test]
    fn config_snippet_use_captures_parameters() {
        let src = "\
#### TML 1.0
- `load` [pic](examples/pic.jpg)

## model-control
- `image` *icon*

# root
- `element`
    - `config`
        - `use` model-control
            - `set-image` *icon* *pic* [0, 0, 1, 1]
";
        let body = process_layout(src.to_string()).expect("should parse").body;

        let params = body.iter().find_map(|c| match c {
            Layout::Config(Config::Use { name, params }) if name.as_str() == "model-control" => {
                Some(params)
            }
            _ => None,
        });
        let params = params.expect("Config::Use emitted");
        assert!(params.iter().any(|(n, v)| n.as_str() == "icon"
            && matches!(v, DataSrc::Static(Declaration::Image(d)) if d.atlas == "pic")));
    }

    /// An `element` can skip its `config` block and start straight into
    /// children - the first body item is not blindly treated as (and skipped
    /// as) the config block. A lone `use` under such an element still runs.
    #[test]
    fn element_without_config_block_keeps_its_first_child() {
        let src = "\
### model-control
- `element`
    - `config`
        - `grow`

# root
- `element`
    - `use` model-control
        - `set-text` *label* hi
- `element`
    - `element` inner
";
        let body = process_layout(src.to_string()).expect("should parse").body;

        // the `use` under the config-less first element is emitted
        assert!(
            body.iter()
                .any(|c| matches!(c, Layout::Element(Element::UseClosed(s)) if s.as_str() == "model-control")),
            "lone `use` child must not be swallowed as a config block"
        );
        // the config-less nested `element inner` is emitted too
        assert!(body.iter().any(|c| matches!(
            c,
            Layout::Config(Config::Id(DataSrc::Static(id))) if id == "inner"
        )));
    }

    /// A `#### TML` header can mix `` `load` `` (image) and `` `font` ``
    /// directives; the font directive's link text is the numeric `font-id`.
    #[test]
    fn font_and_image_header_directives_parse() {
        let src = "\
#### TML 1.0
- `load` [pic](examples/pic.jpg)
- `font` [1](examples/fonts/TwemojiMozilla.ttf)
- `font` [42](fonts/Inter.otf)
- `font` [not-a-number](ignored.ttf)

# root
- `element`
";
        let parsed = process_layout(src.to_string()).expect("should parse");
        assert_eq!(
            parsed.image_loads,
            vec![ImageLoad {
                atlas: "pic".to_string(),
                path: "examples/pic.jpg".to_string(),
            }]
        );
        assert_eq!(
            parsed.font_loads,
            vec![
                FontLoad { id: 1, path: "examples/fonts/TwemojiMozilla.ttf".to_string() },
                FontLoad { id: 42, path: "fonts/Inter.otf".to_string() },
            ]
        );
    }

    /// The shipped `Image Viewer.md` example parses and exercises every image path.
    #[test]
    fn image_viewer_example_parses() {
        let file = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/layouts/Image Viewer.md"
        ))
        .unwrap();
        let parsed = process_layout(file).expect("Image Viewer.md should parse");

        assert_eq!(
            parsed.image_loads,
            vec![ImageLoad {
                atlas: "pic".to_string(),
                path: "examples/pic.jpg".to_string(),
            }]
        );
        assert!(parsed.body.iter().any(|c| matches!(
            c,
            Layout::Declaration { value: DataSrc::Static(Declaration::Image(_)), .. }
        )));
        assert!(
            parsed
                .body
                .iter()
                .any(|c| matches!(c, Layout::Config(Config::ImageLiteral(_))))
        );
        assert!(
            parsed
                .body
                .iter()
                .any(|c| matches!(c, Layout::Config(Config::Image { .. })))
        );
    }

    #[test]
    fn leading_keyword_list_stays_sorted() {
        assert!(LEADING_KEYWORDS.windows(2).all(|w| w[0] < w[1]));
        assert!(SHADER_PARAM_KEYWORDS.windows(2).all(|w| w[0] < w[1]));
        // Every shader-param keyword must also be a leading keyword (so the
        // backtick pre-pass recognises it).
        for k in SHADER_PARAM_KEYWORDS {
            assert!(
                LEADING_KEYWORDS.binary_search(k).is_ok(),
                "{k} missing from LEADING_KEYWORDS"
            );
        }
        assert!(LEADING_KEYWORDS.binary_search(&"shader").is_ok());
    }

    /// A `#### TML` header `` `shader` `` directive records a [`ShaderLoad`].
    #[test]
    fn shader_header_directive_parses() {
        let src = "\
#### TML 1.0
- `shader` [glass](examples/shaders/glass.wgsl)
- `shader` [neon](examples/shaders/neon.wgsl)

# root
- `element`
";
        let parsed = process_layout(src.to_string()).expect("should parse");
        assert_eq!(
            parsed.shader_loads,
            vec![
                ShaderLoad { name: "glass".to_string(), path: "examples/shaders/glass.wgsl".to_string() },
                ShaderLoad { name: "neon".to_string(), path: "examples/shaders/neon.wgsl".to_string() },
            ]
        );
    }

    /// `` `shader` `` in a `config` block opens an effect; the `shadow-*` /
    /// `shader-param-*` keywords tune it (static or `*bound*`). Stacking two
    /// `` `shader` `` keywords gives two specs, each keeping its own params.
    #[test]
    fn shader_config_keyword_parses() {
        let src = "\
# root
- `element`
    - `config`
        - `color` white
        - `shader` *drop_shadow*
        - `shadow-blur` 6
        - `shadow-color` *tint*
        - `shader` *raised_edge*
        - `bevel-width` 4
";
        let parsed = process_layout(src.to_string()).expect("should parse");
        let specs = parsed.body.iter().find_map(|c| match c {
            Layout::Config(Config::Shaders(s)) => Some(s),
            _ => None,
        });
        let specs = specs.expect("a Config::Shaders should be emitted");
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, GlobalSymbol::new("drop_shadow"));
        assert_eq!(specs[0].shadow_blur, DataSrc::Static(6.0));
        assert!(matches!(specs[0].shadow_color, DataSrc::Dynamic(s) if s == GlobalSymbol::new("tint")));
        assert_eq!(specs[1].name, GlobalSymbol::new("raised_edge"));
        assert_eq!(specs[1].bevel_width, DataSrc::Static(4.0));
        // `bevel-width` after the second `shader` must not have touched the first.
        assert_eq!(specs[0].bevel_width, DataSrc::Static(6.0));
    }

    /// The keyword-backtick pre-pass leaves an already-backticked line alone,
    /// wraps a bare leading keyword, and ignores a keyword that isn't the first
    /// word (or isn't a list item at all).
    #[test]
    fn keyword_backtick_prepass() {
        let src = "\
#### TML 1.0
- load [pic](examples/pic.jpg)

# root
- element outer
    - config
        - grow
        - color rgb(1,2,3)
        - `padding-all` 8
    - text
        - color me impressed
- if ready
    - grow
";
        let out = add_missing_keyword_backticks(src);

        // leading keywords picked up, arguments left as text
        assert!(out.contains("- `load` [pic](examples/pic.jpg)"));
        assert!(out.contains("- `element` outer"));
        assert!(out.contains("- `config`"));
        assert!(out.contains("- `grow`"));
        assert!(out.contains("- `color` rgb(1,2,3)"));
        assert!(out.contains("- `if` ready"));
        // already-backticked line untouched (no double backticks)
        assert!(out.contains("- `padding-all` 8"));
        assert!(!out.contains("``"));
        // headings are not list items
        assert!(out.contains("#### TML 1.0"));
        assert!(out.contains("# root"));
        // the documented footgun: a text line starting with a keyword *is*
        // rewritten - `- color me impressed` becomes a config
        assert!(out.contains("- `color` me impressed"));

        // a bare-keyword document parses into the same command stream as the
        // hand-backticked equivalent
        let bare = "\
# root
- element
    - config
        - width-fixed 40
        - color grey
    - text
        - hello
";
        let backticked = "\
# root
- `element`
    - `config`
        - `width-fixed` 40
        - `color` grey
    - `text`
        - hello
";
        assert_eq!(
            process_layout(bare.to_string()).unwrap().body,
            process_layout(backticked.to_string()).unwrap().body,
        );
    }

    /// Running the pre-pass over a fully-backticked file is a no-op.
    #[test]
    fn keyword_backtick_prepass_is_idempotent_on_real_files() {
        for name in ["Main.md", "Custom.md", "Image Viewer.md"] {
            let path = format!("{}/examples/layouts/{name}", env!("CARGO_MANIFEST_DIR"));
            let file = std::fs::read_to_string(&path).unwrap();
            assert_eq!(
                add_missing_keyword_backticks(&file),
                file,
                "{name} should be unchanged by the pre-pass"
            );
        }
    }
}
