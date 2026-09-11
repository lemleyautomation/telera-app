pub use cgmath;
pub use image::{self, DynamicImage, load_from_memory};
use notify::Watcher as _;
pub use rkyv;
use std::{
    collections::HashMap, fmt::Debug, path::{Path, PathBuf}, sync::{Arc, mpsc}, time::{Duration, Instant},
};
pub use symbol_table;
pub use telera_macros::*;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopProxy},
};
pub use winit::{
    dpi::LogicalSize,
    event::{ElementState, KeyEvent},
    keyboard::{self, Key, KeyCode, KeyLocation, NamedKey, PhysicalKey},
    window::{Window, WindowAttributes, WindowId},
};

mod agent_port;

mod graphics;
pub use graphics::model::{
    BaseMesh, Euler, Model, Quaternion, Transform, TransformMatrix, load_model_gltf,
};
pub use graphics::camera::Camera;
pub use graphics::viewport::SyntheticKeyEvent;
pub use ui::canvas::Canvas;
use graphics::{
    scene_renderer::SceneRenderer,
    textures::{DepthTexture, MultiSampleTexture},
    viewport::Viewport,
};
const MULTI_SAMPLE_COUNT: u32 = 1;

mod ui;
pub use ui::layout_runner::{
    Binder, Config, CustomElementSpec, DataSrc, Declaration, Element, EventContext, FieldAccess,
    FontLoad, ImageLoad, Layout, LayoutReflector, LayoutRunnerReflection, LayoutResources,
    ParsedLayout, ShaderLoad, ShaderSpec, process_layout,
};
pub use ui::telera_layout::{Color, ElementConfiguration, TextConfig};
pub use ui::ui_renderer::{
    CustomElement, CustomLayoutSettings, EffectKind, ResolvedShader, UIImageDescriptor,
};
use ui::{
    telera_layout::LayoutEngine, ui_renderer::commands_fingerprint, ui_renderer::effect_source,
    ui_renderer::UIRenderer,
};

pub enum APIError {
    ModelNotFound,
}
/// Sent through the winit event loop's user-event channel by a background
/// thread that can't safely touch `API`/`Binder` itself, for
/// `Application::user_event` to act on back on the event-loop thread.
/// `RebuildLayout` is `spawn_layout_watcher`'s (`RunType::Watch`); the
/// `Agent*` variants are the agent access port's (`Startup::agent_access_port`,
/// `agent_port::spawn_agent_listener`) - each carries an `mpsc::Sender` so
/// the HTTP connection thread that sent it can block for a reply.
#[derive(Debug)]
enum InternalEvents {
    RebuildLayout(PathBuf),
    AgentInput {
        window: String,
        events: Vec<agent_port::SyntheticEvent>,
        reply: mpsc::Sender<agent_port::AgentReply>,
    },
    AgentScreenshot {
        window: String,
        path: PathBuf,
        reply: mpsc::Sender<agent_port::AgentReply>,
    },
    AgentDump {
        window: String,
        kind: agent_port::DumpKind,
        path: PathBuf,
        reply: mpsc::Sender<agent_port::AgentReply>,
    },
}

/// How an `App` wants its markdown layout file(s) loaded, per
/// [`Startup::watch_path`]. In every non-`None` case the string names a
/// *directory* - `API` recursively loads every `.tmd` file it finds there
/// into the `Binder`, not a single file.
#[derive(Clone)]
pub enum RunType {
    /// Load every layout file under this directory once, at startup, and
    /// never look at it again.
    Once(String),
    /// Load every layout file under this directory at startup, then keep
    /// watching it (recursively) in a background thread, reloading
    /// whichever file changed whenever it does.
    Watch(String),
    /// Don't touch the `Binder` at all - this app builds its layout by
    /// hand with `api.l` (see `examples/basic.rs`).
    None,
}

pub struct Startup {
    pub window_attributes: WindowAttributes,
    /// The bootstrap window's title and its viewport-lookup key.
    pub window_name: String,
    /// The page the bootstrap window shows. `None` means "use `window_name`",
    /// i.e. the layout file named `<window_name>.tmd` (or a page registered
    /// under that name by hand). Change it later with [`API::set_viewport_page`].
    pub page: Option<String>,
    pub watch_path: RunType,
    /// If `Some(port)`, spawns a background HTTP listener on
    /// `127.0.0.1:port` that lets an external agent (a script, a test
    /// harness, an LLM tool) drive this app: inject synthetic mouse/
    /// keyboard input, request a screenshot, or dump the current layout/
    /// render commands to a file - see `agent_port` for the wire format.
    /// While active, a ~3px border is force-drawn around every window so
    /// it's visually obvious the app is remotely controllable. `None` (the
    /// default) disables the feature entirely - no socket is opened.
    pub agent_access_port: Option<u16>,
}

#[allow(unused_variables)]
pub trait App: LayoutRunnerReflection + LayoutReflector + Sized {
    /// Called once, before the graphics context (and so `API` itself)
    /// exists, to create the application's first window: the returned
    /// [`Startup::window_name`] is that window's name, and - unless
    /// [`Startup::page`] overrides it, or something like
    /// `API::set_viewport_page` does later - its page too (the layout file
    /// of the same name). [`Startup::watch_path`], if set, names a directory
    /// of markdown layout files for `API` to load itself before the first
    /// frame - the app never reads or parses them.
    ///
    /// Required, not defaulted: there's no generic "right" window to hand
    /// back, so every `App` supplies its own attributes and name here.
    /// Anything else one-time setup needs `&mut API` for (staging an
    /// image, ...) - `API` doesn't exist yet at this point - belongs in
    /// `update`, guarded by a flag on the app so it only runs once.
    fn initialize(&mut self) -> Startup;

    fn onload(&mut self, api: &mut API) {}

    /// All application update logic.
    ///
    /// Called twice per frame cycle:
    /// - once with `viewport == None`, from the global loop-wake scheduler
    ///   (`about_to_wait`), before any window is redrawn. The per-window input
    ///   accessors (`left_mouse_down`, `mouse_position`, ...) have no window to
    ///   read here and return their neutral defaults.
    /// - once with `viewport == Some(name)` for each window that is about to be
    ///   redrawn, right before its `layout` pass. The input accessors read that
    ///   window's state, so mouse/keyboard input belongs in this call - `match`
    ///   or `if let` on `viewport` to act per window.
    fn update(&mut self, viewport: Option<&str>, api: &mut API) {}

    fn layout(&mut self, page: &str, api: &mut API) {}
}

pub struct API {
    staged_windows: Vec<(String, Option<String>, WindowAttributes)>,

    /// Backs `run_layout` - loaded, if `Startup::watch_path` names a
    /// directory, before the first frame (see `Application::resumed`).
    /// Using the markdown-driven layout pipeline at all is opt-in: an
    /// `App` that leaves `watch_path` as `RunType::None` and just drives
    /// `api.l` directly (as `basic.rs` does) never touches this.
    binder: Binder,
    /// Consulted once, in `Application::resumed`, to load the initial
    /// directory of layout files and (for `RunType::Watch`) to know which
    /// directory the background watcher thread should watch.
    watch_path: RunType,

    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub scene_renderer: SceneRenderer,
    ui_renderer: Option<UIRenderer>,
    pub l: LayoutEngine<UIRenderer, UIImageDescriptor, CustomElement, CustomLayoutSettings>,
    /// Per-frame storage for `UIImageDescriptor`s the markdown layout resolves
    /// (from a `set-image` declaration or an `image` literal). The layout engine
    /// keeps a raw pointer to each image descriptor a config sets until the
    /// render pass consumes it, so the value has to outlive `run_layout` - the
    /// declarations/reusables it might otherwise be borrowed from don't. Each
    /// entry is boxed so pushing more never moves the ones already handed out;
    /// cleared at the start of every layout pass.
    #[allow(clippy::vec_box)] // stable element addresses are the whole point
    image_frame_arena: Vec<Box<UIImageDescriptor>>,
    /// Per-frame storage for the `CustomElement` shapes the markdown layout
    /// resolves from a `` `circle` ``/`` `line` ``/`` `arc` ``/... element's
    /// `CustomElementSpec`. Same rationale (and boxing) as `image_frame_arena`:
    /// the layout engine holds a raw pointer to each until the render pass reads
    /// it. Cleared at the start of every layout pass.
    #[allow(clippy::vec_box)] // stable element addresses are the whole point
    custom_shape_arena: Vec<Box<CustomElement>>,
    /// Per-frame storage for the text of every `` `text` `` element, resolved
    /// once here and handed to `add_text_element`. clay stores only a
    /// `(ptr, len)` into the string and re-reads it (unchanged) all the way
    /// through `end_layout`, so a resolved `&str` that borrows a per-list /
    /// per-reusable *clone* of the layout commands (which is dropped when
    /// `run_layout` returns) would dangle - hence a copy that lives as long as
    /// the other frame arenas. Boxed for stable addresses; cleared each pass.
    #[allow(clippy::vec_box)]
    layout_text_arena: Vec<Box<str>>,
    /// Per-frame storage for the `CustomLayoutSettings` a `` `shader` `` config
    /// resolves to. Same rationale (and boxing) as the other frame arenas: the
    /// layout engine holds a raw pointer to each until the render pass reads it.
    #[allow(clippy::vec_box)]
    custom_layout_settings_arena: Vec<Box<CustomLayoutSettings>>,
    /// Atlases loaded from `` `load` `` directives in layout files, keyed by
    /// atlas name -> source path, so re-parsing a file (hot reload) can tell an
    /// already-loaded image from a new one.
    loaded_layout_images: HashMap<String, String>,
    /// `` `font` `` directive dedup: `font-id` -> source path already registered.
    loaded_layout_fonts: HashMap<u16, String>,
    /// `` `shader` `` directive dedup: shader name -> source path already
    /// compiled.
    loaded_layout_shaders: HashMap<String, String>,
    model_ids: HashMap<String, usize>,
    models: Vec<Model>,

    viewport_lookup: bimap::BiMap<String, WindowId>,
    viewports: HashMap<WindowId, Viewport>,
    /// The window currently being updated / laid out / drawn. Set for the
    /// duration of [`API::redraw_viewport`]; the public input accessors
    /// (`left_mouse_clicked`, `mouse_position`, `key_events`, ...) read the
    /// matching [`Viewport`], so layout code and event handlers transparently
    /// see only the input for the window they're building.
    active_window: Option<WindowId>,

    /// Per-`canvas`-element pan/zoom state, keyed by the element's name (or by
    /// `api.canvas("name")`). Auto-created the first frame a `canvas` of that
    /// name is laid out; never auto-removed.
    canvases: HashMap<symbol_table::GlobalSymbol, Canvas>,

    /// Set once from `Startup::agent_access_port.is_some()`; never toggled
    /// back off at runtime. Read every frame in `redraw_viewport` to decide
    /// whether to force-draw the "remotely controllable" border.
    agent_port_active: bool,
    /// `POST /screenshot` requests queued in `Application::user_event`,
    /// fulfilled from inside `redraw_viewport` once `window_id` actually
    /// redraws (screenshot pixels are transient, only existing mid-frame).
    pending_screenshots: Vec<agent_port::PendingScreenshot>,
    /// Same idea for `POST /dump` requests whose `kind` needs
    /// `render_commands` (`"render"`/`"both"`) - a `"layout"`-only dump is
    /// answered synchronously in `user_event` instead.
    pending_dumps: Vec<agent_port::PendingDump>,
}

/// Per-window input accessors. Each reads the [`Viewport`] named by
/// `active_window` (set for the duration of [`API::redraw_viewport`]), so layout
/// code and event handlers see only the input for the window being built. Away
/// from a redraw - or if the active window has gone - they return a neutral
/// default rather than panicking.
impl API {
    fn active_viewport(&self) -> Option<&Viewport> {
        self.viewports.get(&self.active_window?)
    }
    fn active_viewport_mut(&mut self) -> Option<&mut Viewport> {
        self.viewports.get_mut(&self.active_window?)
    }

    pub fn dt(&self) -> f32 {
        self.active_viewport().map_or(0.0, |v| v.dt)
    }
    pub fn window_size(&self) -> (f32, f32) {
        self.active_viewport().map_or((0.0, 0.0), |v| v.size)
    }
    pub fn dpi_scale(&self) -> f32 {
        self.active_viewport().map_or(1.0, |v| v.dpi_scale)
    }
    /// Cursor position in physical px, window-relative.
    pub fn mouse_position(&self) -> (f32, f32) {
        self.active_viewport().map_or((0.0, 0.0), |v| v.mouse_position)
    }
    /// Total cursor movement (physical px) since this window's previous redraw.
    pub fn mouse_delta(&self) -> (f32, f32) {
        self.active_viewport().map_or((0.0, 0.0), |v| v.mouse_delta)
    }
    pub fn scroll_delta(&self) -> (f32, f32) {
        self.active_viewport().map_or((0.0, 0.0), |v| v.scroll_delta)
    }
    pub fn x_at_click(&self) -> f32 {
        self.active_viewport().map_or(0.0, |v| v.x_at_click)
    }
    pub fn y_at_click(&self) -> f32 {
        self.active_viewport().map_or(0.0, |v| v.y_at_click)
    }
    /// The name of the element focused in the active window, or `None`.
    pub fn focus(&self) -> Option<symbol_table::GlobalSymbol> {
        self.active_viewport().and_then(|v| v.focus)
    }
    /// Sets (or, with `None`, clears) the active window's focused element. Called
    /// by the layout runner as it walks the tree on a mouse-down frame.
    pub(crate) fn set_focus(&mut self, focus: Option<symbol_table::GlobalSymbol>) {
        if let Some(viewport) = self.active_viewport_mut() {
            viewport.focus = focus;
        }
    }

    pub fn left_mouse_pressed(&self) -> bool {
        self.active_viewport().is_some_and(|v| v.left_mouse_pressed)
    }
    pub fn left_mouse_down(&self) -> bool {
        self.active_viewport().is_some_and(|v| v.left_mouse_down)
    }
    pub fn left_mouse_released(&self) -> bool {
        self.active_viewport().is_some_and(|v| v.left_mouse_released)
    }
    pub fn left_mouse_clicked(&self) -> bool {
        self.active_viewport().is_some_and(|v| v.left_mouse_clicked)
    }
    pub fn left_mouse_double_clicked(&self) -> bool {
        self.active_viewport()
            .is_some_and(|v| v.left_mouse_double_clicked)
    }
    pub fn right_mouse_pressed(&self) -> bool {
        self.active_viewport().is_some_and(|v| v.right_mouse_pressed)
    }
    pub fn right_mouse_down(&self) -> bool {
        self.active_viewport().is_some_and(|v| v.right_mouse_down)
    }
    pub fn right_mouse_released(&self) -> bool {
        self.active_viewport().is_some_and(|v| v.right_mouse_released)
    }
    pub fn right_mouse_clicked(&self) -> bool {
        self.active_viewport().is_some_and(|v| v.right_mouse_clicked)
    }
    pub fn middle_mouse_pressed(&self) -> bool {
        self.active_viewport().is_some_and(|v| v.middle_mouse_pressed)
    }
    pub fn middle_mouse_down(&self) -> bool {
        self.active_viewport().is_some_and(|v| v.middle_mouse_down)
    }
    pub fn middle_mouse_released(&self) -> bool {
        self.active_viewport()
            .is_some_and(|v| v.middle_mouse_released)
    }
    pub fn middle_mouse_clicked(&self) -> bool {
        self.active_viewport()
            .is_some_and(|v| v.middle_mouse_clicked)
    }

    /// Key events that landed on the active window since its last redraw.
    /// Cleared by `Viewport::end_frame` once the frame has consumed them, so a
    /// layout or handler must read them the same frame they arrive.
    pub fn key_events(&self) -> &[KeyEvent] {
        self.active_viewport().map_or(&[], |v| v.key_events.as_slice())
    }
    pub fn event_string(&self) -> &str {
        self.active_viewport().map_or("", |v| v.event_string.as_str())
    }
    /// Keyboard events fabricated by the agent access port's `POST /input`
    /// (`key_down`/`key_up`) on the active window since its last redraw.
    /// Separate from `key_events` because a real `winit::event::KeyEvent`
    /// can't be constructed outside winit - see [`SyntheticKeyEvent`].
    pub fn synthetic_key_events(&self) -> &[SyntheticKeyEvent] {
        self.active_viewport()
            .map_or(&[], |v| v.synthetic_key_events.as_slice())
    }
    /// Whether `Startup::agent_access_port` was set - i.e. whether a
    /// remote-control HTTP listener is running for this app.
    pub fn agent_port_active(&self) -> bool {
        self.agent_port_active
    }
}

// private api functions
impl API {
    /// Runs `page` (as loaded from `Startup::watch_path`) for this frame
    /// and dispatches whatever events fired straight back into `user_app`
    /// via `LayoutReflector::dispatch_event`.
    fn run_layout<UserApp>(&mut self, page: &str, user_app: &mut UserApp)
    where
        UserApp: LayoutRunnerReflection + LayoutReflector,
    {
        // `set_page` needs `&mut self` (as `api`) and `&mut self.binder` at
        // once, so take the binder out for the call - it's a plain data
        // holder (pages + reusables), nothing reaches back into `API`
        // through it. `set_page` dispatches events itself as it walks.
        let mut binder = std::mem::take(&mut self.binder);
        binder.set_page(page, self, user_app);
        self.binder = binder;
    }

    fn remove_viewport(&mut self, window_id: WindowId) {
        let viewport_title = if let Some(viewport) = self.viewports.get(&window_id) {
            viewport.window.title().clone()
        } else {
            String::default()
        };

        self.viewport_lookup.remove_by_left(viewport_title.as_str());
        self.viewports.remove(&window_id);
    }
    fn resize_viewport(&mut self, window_id: WindowId, size: PhysicalSize<u32>) {
        if let Some(viewport) = self.viewports.get_mut(&window_id) {
            viewport.resize(&self.device, size, MULTI_SAMPLE_COUNT);
        }
    }
    /// Turns every `API::create_viewport`/`create_default_viewport` call
    /// since the last time this ran into a real window, reusing the
    /// `wgpu::Instance`/`Adapter`/`Device`/`Queue` the bootstrap window in
    /// `Application::resumed` already set up - a `Device` can back any
    /// number of surfaces, so there's no need (and, since only the very
    /// first window brings `API` itself into existence, no way) to request
    /// a fresh one per window.
    fn create_staged_viewports(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        for (name, page, attributes) in std::mem::take(&mut self.staged_windows) {
            if self.viewport_lookup.get_by_left(&name).is_some() {
                continue;
            }

            let Ok(window) = event_loop.create_window(attributes) else {
                continue;
            };
            window.set_title(&name);
            // No explicit page -> the window shows the page of the same name.
            let page = page.unwrap_or_else(|| name.clone());
            let window_id = window.id();
            let window = Arc::new(window);

            let Ok(surface) = self.instance.create_surface(window.clone()) else {
                continue;
            };

            let size = window.inner_size();
            let surface_capabilities = surface.get_capabilities(&self.adapter);
            let Some(surface_format) = surface_capabilities.formats.iter().find(|f| f.is_srgb())
            else {
                continue;
            };
            let surface_config = wgpu::SurfaceConfiguration {
                // `COPY_SRC` alongside the usual `RENDER_ATTACHMENT` so the
                // agent access port's screenshot readback (a
                // `copy_texture_to_buffer` straight off the swapchain
                // texture) is legal - without it, wgpu's validation layer
                // rejects that copy as a hard (process-crashing) error.
                usage: agent_port_usage(self.agent_port_active),
                format: *surface_format,
                color_space: wgpu::SurfaceColorSpace::Auto,
                width: size.width,
                height: size.height,
                present_mode: surface_capabilities.present_modes[0],
                desired_maximum_frame_latency: 2,
                alpha_mode: surface_capabilities.alpha_modes[0],
                view_formats: vec![],
            };
            surface.configure(&self.device, &surface_config);

            let depth_texture =
                DepthTexture::new(&self.device, &surface_config, MULTI_SAMPLE_COUNT);
            let multi_sample_texture =
                MultiSampleTexture::new(&self.device, &surface_config, MULTI_SAMPLE_COUNT);

            if let Some(ui_renderer) = self.ui_renderer.as_mut()
                && ui_renderer.render_pipeline.is_none()
            {
                ui_renderer.build_shaders(
                    &self.device,
                    &self.queue,
                    &surface_config,
                    MULTI_SAMPLE_COUNT,
                );
            }
            if self.scene_renderer.render_pipeline.is_none() {
                self.scene_renderer.build_shaders(
                    &self.device,
                    &surface_config,
                    MULTI_SAMPLE_COUNT,
                );
            }

            let viewport = Viewport::new(
                window,
                page,
                surface,
                surface_config,
                depth_texture,
                multi_sample_texture,
            );

            self.viewport_lookup.insert(name, window_id);
            self.viewports.insert(window_id, viewport);
        }
    }
    fn redraw_viewport<UserApp>(&mut self, window_id: WindowId, user_application: &mut UserApp)
    where
        UserApp: App,
    {
        // Everything from here to `end_frame` builds and draws *this* window;
        // the input accessors read the viewport named here.
        self.active_window = Some(window_id);
        let viewport_name = self.viewport_lookup.get_by_right(&window_id).cloned();

        let page = self
            .viewports
            .get(&window_id)
            .map(|viewport| viewport.page.clone());
        let mut render_commands = Vec::new();
        // `render-window` rects for the scene renderer. Collected from this
        // frame's commands (below) even when the cached UI texture is reused -
        // the scene redraws every frame.
        let mut render_windows: Vec<ui::ui_renderer::RenderWindow> = Vec::new();
        let ui_renderer = if let Some(viewport) = self.viewports.get_mut(&window_id)
            && let Some(page) = page
        {
            //println!("UI renderer good");
            viewport.begin_frame();
            let size: (f32, f32) = viewport.window.inner_size().into();
            // Copy the input this frame needs out while the viewport is borrowed;
            // `self.l` / `run_layout` below need `self` exclusively.
            let dpi_scale = viewport.dpi_scale;
            let mouse_position = viewport.mouse_position;
            let left_mouse_down = viewport.left_mouse_down;
            let scroll_delta = viewport.scroll_delta;

            // Per-window update pass: `active_window` is set, so the input
            // accessors now read this window. Runs before the layout pass and
            // before `ui_renderer` is taken below.
            if let Some(name) = &viewport_name {
                user_application.update(Some(name), self);
            }

            let mut ui_renderer = self.ui_renderer.take().unwrap();
            ui_renderer.dpi_scale = dpi_scale;
            ui_renderer.resize((size.0 as i32, size.1 as i32), &self.queue);
            // Advance the text caches and drop stale shaped runs before this
            // frame's layout pass fills them again.
            ui_renderer.frame_tick();

            self.l.set_layout_dimensions(
                ui_renderer.viewport_size.0 / ui_renderer.dpi_scale,
                ui_renderer.viewport_size.1 / ui_renderer.dpi_scale,
            );
            self.l.pointer_state(
                mouse_position.0 / ui_renderer.dpi_scale,
                mouse_position.1 / ui_renderer.dpi_scale,
                left_mouse_down,
            );
            self.l.update_scroll_containers(
                false,
                (scroll_delta.0 / ui_renderer.dpi_scale) * 3.0,
                (scroll_delta.1 / ui_renderer.dpi_scale) * 3.0,
                0.016,
            );
            // Descriptors from the last frame's layout have been rendered; the
            // engine holds no live pointers into the arena now, so it's safe to
            // drop them before this frame fills it again.
            self.image_frame_arena.clear();
            self.custom_shape_arena.clear();
            self.layout_text_arena.clear();
            self.custom_layout_settings_arena.clear();
            self.l.begin_layout(ui_renderer);
            match self.watch_path {
                RunType::None => user_application.layout(&page, self),
                RunType::Once(_) | RunType::Watch(_) => self.run_layout(&page, user_application),
            }
            let (commands, ui_renderer) = self.l.end_layout();
            render_commands = commands;

            // Agent access port: force-draw a ~3px border around the whole
            // window while the port is active, so it's never ambiguous on
            // screen that this app is remotely controllable. Appended last -
            // `ui_renderer::render_layout` draws commands in list order with
            // no re-sort, so this always paints over everything else in the
            // frame regardless of its `z_index`.
            if self.agent_port_active {
                let logical_w = ui_renderer.viewport_size.0 / ui_renderer.dpi_scale;
                let logical_h = ui_renderer.viewport_size.1 / ui_renderer.dpi_scale;
                render_commands.push(ui::telera_layout::RenderCommand::Border(
                    ui::telera_layout::Border {
                        bounding_box: ui::telera_layout::BoundingBox {
                            x: 0.0,
                            y: 0.0,
                            width: logical_w,
                            height: logical_h,
                        },
                        id: u32::MAX,
                        z_index: i16::MAX,
                        custom_layout_settings: None,
                        color: Color { r: 255.0, g: 45.0, b: 0.0, a: 255.0 },
                        corner_radii: ui::telera_layout::CornerRadii {
                            top_left: 0.0,
                            top_right: 0.0,
                            bottom_left: 0.0,
                            bottom_right: 0.0,
                        },
                        width: ui::telera_layout::BorderWidth {
                            left: 3,
                            right: 3,
                            top: 3,
                            bottom: 3,
                            between_children: 0,
                        },
                    },
                ));
            }

            // Agent access port: fulfil any `POST /dump` requests for this
            // window that need `render_commands` (`"render"`/`"both"`) - a
            // `"layout"`-only dump was already answered synchronously in
            // `user_event`, so only those two kinds ever land here.
            if !self.pending_dumps.is_empty() {
                let (due, remaining): (Vec<_>, Vec<_>) = self
                    .pending_dumps
                    .drain(..)
                    .partition(|pending| pending.window_id == window_id);
                self.pending_dumps = remaining;
                for pending in due {
                    let mut text = String::new();
                    if matches!(
                        pending.kind,
                        agent_port::DumpKind::Layout | agent_port::DumpKind::Both
                    ) {
                        text.push_str("=== LAYOUT COMMANDS ===\n");
                        match self.binder.page(&page) {
                            Some(layout) => text.push_str(&format!("{layout:#?}\n")),
                            None => text.push_str("(page not found)\n"),
                        }
                    }
                    if matches!(
                        pending.kind,
                        agent_port::DumpKind::Render | agent_port::DumpKind::Both
                    ) {
                        text.push_str("=== RENDER COMMANDS ===\n");
                        text.push_str(&format!("{render_commands:#?}\n"));
                    }
                    match std::fs::write(&pending.path, text) {
                        Ok(()) => {
                            let _ = pending.reply.send(agent_port::AgentReply::ok(Some(
                                pending.path.display().to_string(),
                            )));
                        }
                        Err(error) => {
                            let _ = pending
                                .reply
                                .send(agent_port::AgentReply::error(error.to_string()));
                        }
                    }
                }
            }

            // Canvas pan/zoom control and the world-size auto fallback want this
            // frame's on-screen rect for every `canvas` element the walk touched.
            self.refresh_canvas_rects();
            render_windows = ui::ui_renderer::collect_render_windows(
                &render_commands,
                ui_renderer.dpi_scale,
            );
            //            println!("{:#?}", render_commands);

            Some(ui_renderer)
        } else {
            None
        };

        if let Some(mut ui_renderer) = ui_renderer {
            if let Some(viewport) = self.viewports.get_mut(&window_id)
                && let Some(drawable) = viewport.get_current_texture()
            {
                let drawable_view = drawable
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                //println!("render surface acquired");
                let mut command_encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Render Encoder"),
                        });

                // The cached UI layer is only re-rendered when this frame's
                // render commands differ from the ones already baked into it;
                // otherwise the pass is skipped and step 3 just re-composites
                // last frame's pixels over the freshly drawn scene.
                viewport.ensure_ui_surface(
                    &self.device,
                    ui_renderer.composite_bind_group_layout(),
                    ui_renderer.composite_sampler(),
                );
                let fingerprint = commands_fingerprint(
                    &render_commands,
                    ui_renderer.dpi_scale,
                    ui_renderer.viewport_size,
                );
                let scene_target = (
                    viewport.surface_config.width as f32,
                    viewport.surface_config.height as f32,
                );
                let ui_surface = viewport.ui_surface.as_mut().unwrap();
                let ui_dirty = ui_surface.last_fingerprint != Some(fingerprint);

                if MULTI_SAMPLE_COUNT == 1 {
                    // 1. Re-render the UI into its offscreen texture, only when
                    //    the commands changed.
                    if ui_dirty {
                        let mut ui_pass =
                            command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("UI Pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &ui_surface.color_view,
                                    depth_slice: None,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color {
                                            r: 0.0,
                                            g: 0.0,
                                            b: 0.0,
                                            a: 0.0,
                                        }),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: Some(
                                    wgpu::RenderPassDepthStencilAttachment {
                                        view: &ui_surface.depth_view,
                                        depth_ops: Some(wgpu::Operations {
                                            load: wgpu::LoadOp::Clear(1.0),
                                            store: wgpu::StoreOp::Store,
                                        }),
                                        stencil_ops: None,
                                    },
                                ),
                                timestamp_writes: None,
                                occlusion_query_set: None,
                                multiview_mask: None,
                            });
                        ui_renderer.render_layout(
                            render_commands,
                            &mut ui_pass,
                            &self.device,
                            &self.queue,
                            &viewport.surface_config,
                        );
                        drop(ui_pass);

                        // 1b. Backdrop `blur`: snapshot the freshly-drawn UI
                        //     layer, then blur each `` `shader` *blur* `` region
                        //     back into it. Part of the cached layer - only runs
                        //     on a dirty frame.
                        if ui_renderer.has_blur() {
                            command_encoder.copy_texture_to_texture(
                                ui_surface.color.as_image_copy(),
                                ui_surface.blur_src.as_image_copy(),
                                wgpu::Extent3d {
                                    width: ui_surface.width,
                                    height: ui_surface.height,
                                    depth_or_array_layers: 1,
                                },
                            );
                            let mut blur_pass = command_encoder.begin_render_pass(
                                &wgpu::RenderPassDescriptor {
                                    label: Some("UI Blur Pass"),
                                    color_attachments: &[Some(
                                        wgpu::RenderPassColorAttachment {
                                            view: &ui_surface.color_view,
                                            depth_slice: None,
                                            resolve_target: None,
                                            ops: wgpu::Operations {
                                                load: wgpu::LoadOp::Load,
                                                store: wgpu::StoreOp::Store,
                                            },
                                        },
                                    )],
                                    depth_stencil_attachment: None,
                                    timestamp_writes: None,
                                    occlusion_query_set: None,
                                    multiview_mask: None,
                                },
                            );
                            ui_renderer.render_blur(
                                &mut blur_pass,
                                &ui_surface.blur_bind_group,
                                &self.device,
                                &self.queue,
                            );
                            drop(blur_pass);
                        }

                        ui_surface.last_fingerprint = Some(fingerprint);
                    }

                    // 2. Draw the 3D scene into the swapchain.
                    {
                        let mut scene_pass =
                            command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("Scene Pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &drawable_view,
                                    depth_slice: None,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color {
                                            r: 0.15,
                                            g: 0.15,
                                            b: 0.15,
                                            a: 1.0,
                                        }),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: Some(
                                    wgpu::RenderPassDepthStencilAttachment {
                                        view: &viewport.depth_texture.view,
                                        depth_ops: Some(wgpu::Operations {
                                            load: wgpu::LoadOp::Clear(1.0),
                                            store: wgpu::StoreOp::Store,
                                        }),
                                        stencil_ops: None,
                                    },
                                ),
                                timestamp_writes: None,
                                occlusion_query_set: None,
                                multiview_mask: None,
                            });
                        self.scene_renderer.render(
                            &mut self.models,
                            &mut scene_pass,
                            &self.device,
                            &self.queue,
                            &render_windows,
                            scene_target,
                        );
                    }

                    // 3. Composite the cached UI texture over the scene.
                    {
                        let mut composite_pass =
                            command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("UI Composite Pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &drawable_view,
                                    depth_slice: None,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                                multiview_mask: None,
                            });
                        ui_renderer
                            .composite(&mut composite_pass, &ui_surface.composite_bind_group);
                    }

                    //println!("frame rendreed");
                } else {
                    // MSAA path: scene only. UI compositing is not wired here
                    // (dead while MULTI_SAMPLE_COUNT == 1).
                    let mut render_pass: wgpu::RenderPass =
                        command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("RenderPass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &viewport.multi_sample_texture.view,
                                depth_slice: None,
                                resolve_target: Some(&drawable_view),
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color {
                                        r: 1.0,
                                        g: 1.0,
                                        b: 1.0,
                                        a: 1.0,
                                    }),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: Some(
                                wgpu::RenderPassDepthStencilAttachment {
                                    view: &viewport.depth_texture.view,
                                    depth_ops: Some(wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(1.0),
                                        store: wgpu::StoreOp::Store,
                                    }),
                                    stencil_ops: None,
                                },
                            ),
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });

                    self.scene_renderer.render(
                        &mut self.models,
                        &mut render_pass,
                        &self.device,
                        &self.queue,
                        &render_windows,
                        scene_target,
                    );
                }

                // Agent access port: if a screenshot was requested for this
                // window, record the readback copy into this same command
                // buffer now - `drawable`'s texture is about to be presented
                // and is the last point in the frame it can be read from.
                let screenshot_readback = if self
                    .pending_screenshots
                    .iter()
                    .any(|pending| pending.window_id == window_id)
                {
                    let (due, remaining): (Vec<_>, Vec<_>) = self
                        .pending_screenshots
                        .drain(..)
                        .partition(|pending| pending.window_id == window_id);
                    self.pending_screenshots = remaining;

                    let format = viewport.surface_config.format;
                    let width = viewport.surface_config.width;
                    let height = viewport.surface_config.height;
                    let unpadded_bpr = width * 4;
                    let padded_bpr = unpadded_bpr.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
                    let readback_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("agent screenshot readback"),
                        size: u64::from(padded_bpr) * u64::from(height),
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    });
                    command_encoder.copy_texture_to_buffer(
                        drawable.texture.as_image_copy(),
                        wgpu::TexelCopyBufferInfo {
                            buffer: &readback_buffer,
                            layout: wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(padded_bpr),
                                rows_per_image: Some(height),
                            },
                        },
                        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                    );
                    Some((readback_buffer, width, height, padded_bpr, unpadded_bpr, format, due))
                } else {
                    None
                };

                self.queue.submit(std::iter::once(command_encoder.finish()));
                self.queue.present(drawable);

                if let Some((readback_buffer, width, height, padded_bpr, unpadded_bpr, format, due)) =
                    screenshot_readback
                {
                    self.fulfil_screenshots(readback_buffer, width, height, padded_bpr, unpadded_bpr, format, due);
                }
            }

            self.ui_renderer = Some(ui_renderer);
        }

        // The frame that observed this window's one-shot input is done - clear
        // the edge-triggered flags, deltas and key events so they don't leak
        // into the next redraw.
        if let Some(viewport) = self.viewports.get_mut(&window_id) {
            viewport.end_frame();
        }
        self.active_window = None;

    }

    /// Maps `readback_buffer` (already holding a `copy_texture_to_buffer` of
    /// the just-presented frame, padded to `padded_bpr`-byte rows per wgpu's
    /// alignment requirement), strips the padding (and swaps BGRA -> RGBA if
    /// the surface uses a BGRA format), encodes it as PNG via the `image`
    /// crate, and replies to every queued `POST /screenshot` request that
    /// wanted this frame.
    fn fulfil_screenshots(
        &mut self,
        readback_buffer: wgpu::Buffer,
        width: u32,
        height: u32,
        padded_bpr: u32,
        unpadded_bpr: u32,
        format: wgpu::TextureFormat,
        due: Vec<agent_port::PendingScreenshot>,
    ) {
        let slice = readback_buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());

        if !rx.recv().is_ok_and(|result| result.is_ok()) {
            for pending in due {
                let _ = pending
                    .reply
                    .send(agent_port::AgentReply::error("failed to map screenshot buffer"));
            }
            return;
        }

        let data = match slice.get_mapped_range() {
            Ok(data) => data,
            Err(error) => {
                for pending in due {
                    let _ = pending
                        .reply
                        .send(agent_port::AgentReply::error(format!(
                            "failed to read mapped screenshot buffer: {error}"
                        )));
                }
                return;
            }
        };
        let bgra = matches!(
            format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let mut pixels = vec![0u8; unpadded_bpr as usize * height as usize];
        for row in 0..height as usize {
            let src_start = row * padded_bpr as usize;
            let src = &data[src_start..src_start + unpadded_bpr as usize];
            let dst_start = row * unpadded_bpr as usize;
            let dst = &mut pixels[dst_start..dst_start + unpadded_bpr as usize];
            dst.copy_from_slice(src);
            if bgra {
                for pixel in dst.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
            }
        }
        drop(data);
        readback_buffer.unmap();

        match image::RgbaImage::from_raw(width, height, pixels) {
            Some(image) => {
                for pending in due {
                    match image.save(&pending.path) {
                        Ok(()) => {
                            let _ = pending.reply.send(agent_port::AgentReply::ok(Some(
                                pending.path.display().to_string(),
                            )));
                        }
                        Err(error) => {
                            let _ = pending
                                .reply
                                .send(agent_port::AgentReply::error(error.to_string()));
                        }
                    }
                }
            }
            None => {
                for pending in due {
                    let _ = pending.reply.send(agent_port::AgentReply::error(
                        "failed to interpret screenshot pixel buffer",
                    ));
                }
            }
        }
    }

}

/// public api functions
impl API {
    /// Stages a new window. `name` is its title and its viewport-lookup key.
    /// `page` is the page it shows; pass `None` to use `name` itself (the
    /// layout file of that name, or a hand-registered page). Repoint it later
    /// with [`set_viewport_page`](Self::set_viewport_page).
    pub fn create_viewport(
        &mut self,
        name: &str,
        page: Option<&str>,
        attributes: WindowAttributes,
    ) {
        self.staged_windows
            .push((name.to_string(), page.map(str::to_string), attributes));
    }
    pub fn create_default_viewport(&mut self) {
        let new_window = Window::default_attributes().with_inner_size(LogicalSize::new(800, 600));
        self.staged_windows
            .push(("Main".to_string(), None, new_window));
    }
    /// Registers `image` under the atlas name `name`, to be uploaded on the next
    /// frame. If `name` is already a known atlas (or already staged this frame),
    /// the pixels are replaced; otherwise a new atlas is added. Safe to call
    /// before the renderer exists - the call is dropped in that case.
    pub fn add_image(&mut self, name: &str, image: DynamicImage) {
        if let Some(ui_renderer) = &mut self.ui_renderer {
            ui_renderer.stage_atlas(name.to_string(), image);
        }
    }

    /// Registers a font (`data` is the raw bytes of a `.ttf` / `.otf` file)
    /// under `font_id`, so TML `` `font-id` `` / [`TextConfig::font_id`] selects
    /// it for both measurement and drawing. Unregistered ids use the system
    /// sans-serif. Call during [`App::initialize`]; a no-op if the renderer does
    /// not exist yet.
    pub fn load_font(&mut self, font_id: u16, data: Vec<u8>) {
        if let Some(ui_renderer) = &mut self.ui_renderer {
            ui_renderer.register_font(font_id, data);
        }
    }

    /// Boxes `descriptor` into the per-frame arena and hands back a reference
    /// that stays valid for the rest of the layout + render pass (see
    /// [`API::image_frame_arena`]).
    pub(crate) fn stage_frame_image(
        &mut self,
        descriptor: UIImageDescriptor,
    ) -> &UIImageDescriptor {
        self.image_frame_arena.push(Box::new(descriptor));
        self.image_frame_arena.last().unwrap()
    }

    /// Copies `text` into the per-frame arena and adds it as a text element on
    /// the currently open layout element. The copy is what keeps clay's
    /// `(ptr, len)` valid through `end_layout` (see [`API::layout_text_arena`]).
    pub(crate) fn add_layout_text(&mut self, text: &str, config: &TextConfig) {
        self.layout_text_arena.push(text.into());
        // `layout_text_arena` and `l` are disjoint fields, so this borrow is fine.
        let stored: &str = self.layout_text_arena.last().unwrap();
        self.l.add_text_element(stored, config, false);
    }

    /// Boxes `shape` into the per-frame arena and hands back a reference that
    /// stays valid for the rest of the layout + render pass (see
    /// [`API::custom_shape_arena`]).
    pub(crate) fn stage_frame_shape(&mut self, shape: CustomElement) -> &CustomElement {
        self.custom_shape_arena.push(Box::new(shape));
        self.custom_shape_arena.last().unwrap()
    }

    /// Boxes a resolved `` `shader` `` effect into the per-frame arena and hands
    /// back a reference that stays valid through the render pass (see
    /// [`API::custom_layout_settings_arena`]).
    pub(crate) fn stage_frame_layout_settings(
        &mut self,
        settings: CustomLayoutSettings,
    ) -> &CustomLayoutSettings {
        self.custom_layout_settings_arena.push(Box::new(settings));
        self.custom_layout_settings_arena.last().unwrap()
    }
    pub fn set_viewport_title(&mut self, viewport: &str, title: &str) {
        if let Some(window_id) = self.viewport_lookup.get_by_left(viewport)
            && let Some(viewport) = self.viewports.get_mut(window_id)
        {
            viewport.window.set_title(title);
        }
    }
    pub fn set_current_viewport_page(&mut self, page: &str) {
        // TODO !
        println!("{:?}", page);
    }
    pub fn set_viewport_page(&mut self, viewport: &str, page: &str) {
        if let Some(window_id) = self.viewport_lookup.get_by_left(viewport)
            && let Some(window) = self.viewports.get_mut(window_id)
        {
            window.page = page.to_string();
            window.redraw_requested = true;
        }
    }

    /// Asks the framework to produce **one** more frame for `viewport` (its
    /// title / lookup key). The frame is paced by that viewport's
    /// `frame_interval` and the flag clears once it renders. For a running
    /// animation, prefer [`API::set_viewport_continuous`].
    pub fn request_redraw(&mut self, viewport: &str) {
        if let Some(window_id) = self.viewport_lookup.get_by_left(viewport)
            && let Some(window) = self.viewports.get_mut(window_id)
        {
            window.redraw_requested = true;
        }
    }

    /// Turns a viewport's continuous-render loop on or off. While on, the
    /// framework produces a frame every `frame_interval` for that window
    /// (default 30 fps) without the app re-asking each frame. Turning it on also
    /// wakes the loop immediately.
    pub fn set_viewport_continuous(&mut self, viewport: &str, continuous: bool) {
        if let Some(window_id) = self.viewport_lookup.get_by_left(viewport)
            && let Some(window) = self.viewports.get_mut(window_id)
        {
            window.continuous_rendering = continuous;
            if continuous {
                window.redraw_requested = true;
            }
        }
    }

    /// Overrides a viewport's minimum gap between animation frames (default
    /// 30 fps). Larger = slower / less CPU.
    pub fn set_viewport_frame_interval(&mut self, viewport: &str, interval: Duration) {
        if let Some(window_id) = self.viewport_lookup.get_by_left(viewport)
            && let Some(window) = self.viewports.get_mut(window_id)
        {
            window.frame_interval = interval;
        }
    }

    /// Registers a scene camera under `name` (at the default framing) unless one
    /// already exists. A `` `render-window` `` element whose name is `name` will
    /// draw the scene through it; position it from the layout's camera keywords
    /// or from app code via [`API::camera`]. A camera named `"default"` always
    /// exists and is what the scene falls back to full-screen when a frame has
    /// no `render-window`.
    pub fn add_camera(&mut self, name: &str) {
        self.scene_renderer
            .add_camera(&self.device, symbol_table::GlobalSymbol::new(name));
    }

    /// Removes the scene camera `name` (a no-op for `"default"`, which cannot be
    /// removed).
    pub fn remove_camera(&mut self, name: &str) {
        self.scene_renderer
            .remove_camera(symbol_table::GlobalSymbol::new(name));
    }

    /// Mutable access to a scene camera's framing for app-driven control - its
    /// `eye` / `target` / `fovy` fields, or the `pan` / `orbit` / `zoom` /
    /// `perspective` / … convenience methods:
    ///
    /// ```ignore
    /// if let Some(camera) = api.camera("orbit") {
    ///     camera.orbit(dt * 0.4, 0.0).zoom(0.01);
    /// }
    /// ```
    ///
    /// `None` if no camera of that name exists yet - call [`API::add_camera`]
    /// first, or let a `render-window` create it.
    pub fn camera(&mut self, name: &str) -> Option<&mut Camera> {
        self.scene_renderer
            .camera_mut(symbol_table::GlobalSymbol::new(name))
    }

    /// Registers a [`Canvas`] (pan/zoom state) under `name` at zoom `1.0` /
    /// pan `0` unless one already exists. A `` `canvas` `` element whose name is
    /// `name` reads its pan/zoom from here every frame; a `canvas` also
    /// auto-creates its own on first layout, so calling this is only needed to
    /// set limits in `onload` before the first frame.
    pub fn add_canvas(&mut self, name: &str) {
        self.canvases
            .entry(symbol_table::GlobalSymbol::new(name))
            .or_default();
    }

    /// Removes the [`Canvas`] `name`. A `canvas` element still on a page will
    /// re-create it (back at the default framing) next frame.
    pub fn remove_canvas(&mut self, name: &str) {
        self.canvases.remove(&symbol_table::GlobalSymbol::new(name));
    }

    /// Mutable access to a canvas's pan/zoom for app-driven control - its
    /// `zoom` / `pan_x` / `pan_y` fields or the `zoom_by` / `zoom_at_screen` /
    /// `pan_by` / `reset` methods:
    ///
    /// ```ignore
    /// fn update(&mut self, _v: Option<&str>, api: &mut API) {
    ///     let (mx, my) = api.mouse_position();
    ///     let (_, wheel) = api.scroll_delta();
    ///     if let Some(c) = api.canvas("board") {
    ///         if wheel != 0.0 { c.zoom_at_screen(1.0 + wheel * 0.1, mx, my); }
    ///         if api.right_mouse_down() {
    ///             let (dx, dy) = api.mouse_delta();
    ///             c.pan_by(dx, dy);
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// `None` if no canvas of that name has been created or laid out yet.
    pub fn canvas(&mut self, name: &str) -> Option<&mut Canvas> {
        self.canvases.get_mut(&symbol_table::GlobalSymbol::new(name))
    }

    /// The canvas element's on-screen rect `(x, y, w, h)` in **physical px**
    /// (same space as `mouse_position`), as of the last frame it was laid out
    /// (one frame old, like reading a camera before a redraw). `None` if the
    /// canvas is unknown or not yet drawn.
    pub fn canvas_rect(&self, name: &str) -> Option<(f32, f32, f32, f32)> {
        self.canvases
            .get(&symbol_table::GlobalSymbol::new(name))
            .map(|c| c.screen_rect)
    }

    // --- canvas: layout-runner-facing helpers -------------------------------

    /// The canvas `name`, creating it at the default framing if absent. Used by
    /// the layout runner while walking a `` `canvas` `` element.
    pub(crate) fn canvas_entry(&mut self, name: symbol_table::GlobalSymbol) -> &mut Canvas {
        self.canvases.entry(name).or_default()
    }

    /// Current zoom of canvas `name` (`1.0` if unknown).
    pub(crate) fn canvas_zoom(&self, name: symbol_table::GlobalSymbol) -> f32 {
        self.canvases.get(&name).map_or(1.0, |c| c.zoom)
    }

    /// Pan of canvas `name` in the layout engine's logical px (its
    /// `clip.childOffset`), `(0, 0)` if unknown.
    pub(crate) fn canvas_pan_logical(&self, name: symbol_table::GlobalSymbol) -> (f32, f32) {
        self.canvases.get(&name).map_or((0.0, 0.0), |c| c.pan_logical())
    }

    /// World size for canvas `name` in **logical** px (what `width_fixed` wants):
    /// its configured `world-width`/`world-height` if set, else the canvas
    /// element's own laid-out size, else [`Canvas::AUTO_WORLD`].
    pub(crate) fn canvas_world_or_default(&self, name: symbol_table::GlobalSymbol) -> (f32, f32) {
        let c = match self.canvases.get(&name) {
            Some(c) => c,
            None => return (Canvas::AUTO_WORLD, Canvas::AUTO_WORLD),
        };
        let dpi = if c.dpi > 1e-4 { c.dpi } else { 1.0 };
        // `screen_rect` is physical; the world wrapper is sized in logical px.
        let fallback = |v: f32| if v > dpi { v / dpi } else { Canvas::AUTO_WORLD };
        (
            c.world_width.unwrap_or_else(|| fallback(c.screen_rect.2)),
            c.world_height.unwrap_or_else(|| fallback(c.screen_rect.3)),
        )
    }

    /// After a layout pass, refresh every known canvas's `dpi` + `screen_rect`
    /// (physical px) from the finished Clay tree so cursor-anchored zoom and the
    /// world-size auto fallback have this frame's geometry.
    fn refresh_canvas_rects(&mut self) {
        let dpi = self.dpi_scale();
        let names: Vec<symbol_table::GlobalSymbol> = self.canvases.keys().copied().collect();
        for name in names {
            let id = self.l.get_element_id(name.as_str());
            let bb = self.l.bounding_box(id);
            if let Some(canvas) = self.canvases.get_mut(&name) {
                canvas.dpi = dpi;
                if let Some(bb) = bb {
                    canvas.screen_rect =
                        (bb.x * dpi, bb.y * dpi, bb.width * dpi, bb.height * dpi);
                }
            }
        }
    }

    /// Reads and parses a single markdown layout file (see
    /// `process_layout`) and registers it - along with any reusable snippets
    /// it defines - as the page named after the file itself (its name with
    /// its extension stripped), returning that page name. Called both
    /// for the initial directory scan (`load_layout_directory`) and, one
    /// file at a time, whenever `Application::user_event` reloads a file the
    /// watcher thread reported as changed - an `App` never calls this
    /// itself, let alone touches the `Binder` it feeds.
    fn load_layout_file(&mut self, path: &Path) -> Result<String, String> {
        let page_name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| format!("layout file {path:?} has no usable name"))?
            .to_string();
        let markdown = std::fs::read_to_string(path)
            .map_err(|error| format!("failed to read layout file {path:?}: {error}"))?;
        let resources = self.binder.load_layout(&page_name, &markdown)?;
        let LayoutResources {
            image_loads,
            font_loads,
            shader_loads,
        } = resources;

        // Fulfil the file's `` `load` `` directives: decode each image once and
        // stage it under its atlas name. Re-parsing the same file (hot reload)
        // re-stages, so a changed image picks up; an unrelated file naming an
        // atlas that's already loaded from the same path is skipped.
        for load in image_loads {
            if self.loaded_layout_images.get(&load.atlas) == Some(&load.path) {
                continue;
            }
            match image::open(&load.path) {
                Ok(decoded) => {
                    self.add_image(&load.atlas, decoded);
                    self.loaded_layout_images
                        .insert(load.atlas.clone(), load.path.clone());
                }
                Err(error) => eprintln!(
                    "layout {page_name:?}: failed to load image {:?} ({}): {error}",
                    load.path, load.atlas
                ),
            }
        }

        // Same for `` `font` `` directives: read each `.ttf`/`.otf` once and
        // register it (also skipped if the same path is already loaded for that
        // id).
        for load in font_loads {
            if self.loaded_layout_fonts.get(&load.id) == Some(&load.path) {
                continue;
            }
            match std::fs::read(&load.path) {
                Ok(bytes) => {
                    self.load_font(load.id, bytes);
                    self.loaded_layout_fonts.insert(load.id, load.path.clone());
                }
                Err(error) => eprintln!(
                    "layout {page_name:?}: failed to load font {:?} (id {}): {error}",
                    load.path, load.id
                ),
            }
        }

        // `` `shader` `` directives: read each `.wgsl`, validate + compile it
        // into an effect pipeline (skipped if the same path is already loaded
        // for that name). A bad shader is logged and ignored - the element it
        // would have styled just renders without the effect.
        for load in shader_loads {
            if self.loaded_layout_shaders.get(&load.name) == Some(&load.path) {
                continue;
            }
            match std::fs::read_to_string(&load.path) {
                Ok(source) => match self.register_ui_shader(&load.name, &source) {
                    Ok(()) => {
                        self.loaded_layout_shaders
                            .insert(load.name.clone(), load.path.clone());
                    }
                    Err(error) => eprintln!(
                        "layout {page_name:?}: shader {:?} ({}) failed: {error}",
                        load.path, load.name
                    ),
                },
                Err(error) => eprintln!(
                    "layout {page_name:?}: failed to read shader {:?} ({}): {error}",
                    load.path, load.name
                ),
            }
        }

        Ok(page_name)
    }

    /// Validates a custom UI-shader body (naga parse + validate, after the
    /// shared prelude is prepended) and compiles it into an effect pipeline
    /// keyed by `name`. Returns an error string (never panics) on any WGSL
    /// problem. Also reachable via a `` `shader` `` header directive.
    pub fn register_ui_shader(&mut self, name: &str, wgsl_body: &str) -> Result<(), String> {
        let full = effect_source(wgsl_body);

        let module = wgpu::naga::front::wgsl::parse_str(&full)
            .map_err(|e| e.emit_to_string(&full))?;
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .map_err(|e| e.emit_to_string(&full))?;

        let sym = symbol_table::GlobalSymbol::new(name.trim());
        if let Some(ui_renderer) = &mut self.ui_renderer {
            ui_renderer.compile_effect_shader(&self.device, sym, &full);
        } else {
            return Err("renderer not initialised".to_string());
        }
        Ok(())
    }

    /// Recursively loads every `.tmd` file under `dir` into the `Binder`
    /// (see `load_layout_file`), each registered as a page named after its
    /// file, skipping (and logging) any individual file that fails to read
    /// or parse rather than aborting the whole scan. Errors only if `dir`
    /// contains no `.tmd` files at all, or none of them parsed.
    fn load_layout_directory(&mut self, dir: &str) -> Result<(), String> {
        let files = find_layout_files(Path::new(dir));
        if files.is_empty() {
            return Err(format!("no .tmd layout files found under {dir:?}"));
        }

        let mut any_loaded = false;
        for file in files {
            match self.load_layout_file(&file) {
                Ok(_) => any_loaded = true,
                Err(error) => eprintln!("failed to load layout file {file:?}: {error}"),
            }
        }

        if any_loaded {
            Ok(())
        } else {
            Err(format!("no layout files under {dir:?} parsed successfully"))
        }
    }

    pub fn load_gltf_model(
        &mut self,
        model_name: &str,
        filename: PathBuf,
        transfrom: Option<Transform>,
    ) -> BaseMesh {
        self.model_ids
            .insert(model_name.to_string(), self.models.len());
        let model = load_model_gltf(filename, &self.device, &self.queue, transfrom).unwrap();
        let base = model.mesh.base.clone();
        self.models.push(model);

        base
    }
    pub fn transform_model(&mut self, model_name: &str) -> Result<&mut Transform, APIError> {
        if let Some(model_index) = self.model_ids.get(model_name)
            && let Some(model_reference) = self.models.get_mut(*model_index)
        {
            model_reference.transform_dirty = true;
            return Ok(&mut model_reference.transform);
        }

        Err(APIError::ModelNotFound)
    }
    pub fn add_instance(
        &mut self,
        model_name: &str,
        instance_name: &str,
        transfrom: Option<Transform>,
    ) {
        if let Some(model_index) = self.model_ids.get(model_name)
            && let Some(model) = self.models.get_mut(*model_index)
        {
            model
                .mesh
                .add_instance(instance_name.to_string(), &self.device, transfrom);
            //println!("hi {:?}", model.mesh.instances);
        }
    }
    pub fn transform_instance(
        &mut self,
        model_name: &str,
        instance_name: &str,
    ) -> Result<&mut Transform, APIError> {
        if let Some(model_index) = self.model_ids.get(model_name)
            && let Some(model_reference) = self.models.get_mut(*model_index)
            && let Some(instance) = model_reference.mesh.instance_lookup.get(instance_name)
            && let Some(instance_reference) = model_reference.mesh.instances.get_mut(*instance)
        {
            model_reference.mesh.instances_dirty = true;
            return Ok(instance_reference);
        }
        Err(APIError::ModelNotFound)
    }
}

struct Application<UserApp>
where
    UserApp: App,
{
    api: Option<API>,
    user_application: UserApp,

    /// Cloned into `spawn_layout_watcher` for `RunType::Watch` so its
    /// background thread can send `InternalEvents::RebuildLayout` back to
    /// `Application::user_event` on the winit event-loop thread.
    app_events: EventLoopProxy<InternalEvents>,
}

impl<UserApp> Application<UserApp>
where
    UserApp: App,
{
    pub fn new(app_events: EventLoopProxy<InternalEvents>, user_application: UserApp) -> Self {
        Application {
            api: None,
            app_events,
            user_application,
        }
    }
}

impl<UserApp> ApplicationHandler<InternalEvents> for Application<UserApp>
where
    UserApp: App,
{
    /// The frame-loop scheduler. Runs once per event-loop iteration, right
    /// before the loop blocks. It decides, per window, whether a frame is owed
    /// (a pending `redraw_requested`, a `continuous_rendering` loop, or camera
    /// movement) and, if so, whether enough time has passed since that window's
    /// last frame to ask for another. When nothing is owed anywhere the loop is
    /// parked on `ControlFlow::Wait` and sleeps until the next OS event -
    /// that's what keeps idle CPU at ~0.
    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let Some(api) = &mut self.api else {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        };

        // App logic + window staging: moved off `window_event` so they run once
        // per loop wake (i.e. per frame while animating), not once per OS event.
        self.user_application.update(None, api);
        api.create_staged_viewports(event_loop);

        let now = Instant::now();

        let mut next_wake: Option<Instant> = None;
        for viewport in api.viewports.values_mut() {
            let minimized =
                viewport.surface_config.width == 0 || viewport.surface_config.height == 0;
            let wants_frame =
                !minimized && (viewport.redraw_requested || viewport.continuous_rendering);
            if !wants_frame {
                continue;
            }

            let due = viewport.last_render + viewport.frame_interval;
            if now >= due {
                viewport.window.request_redraw();
            } else {
                next_wake = Some(next_wake.map_or(due, |w| w.min(due)));
            }
        }

        event_loop.set_control_flow(match next_wake {
            Some(deadline) => ControlFlow::WaitUntil(deadline),
            None => ControlFlow::Wait,
        });
    }
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        //println!("building context");
        if self.api.is_none() {
            let startup = self.user_application.initialize();

            if let Ok(window) = event_loop.create_window(startup.window_attributes) {
                //println!("window created");
                let window_id = window.id();
                let window = Arc::new(window);
                let instance = wgpu::Instance::default();
                if let Ok(surface) = instance.create_surface(window.clone()) {
                    //println!("render surface created");
                    let adapter_options = wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::default(),
                        compatible_surface: Some(&surface),
                        force_fallback_adapter: false,
                        apply_limit_buckets: false,
                    };
                    //println!("requesting adapater context");
                    if let Ok(adapter) =
                        pollster::block_on(instance.request_adapter(&adapter_options))
                    {
                        //println!("adapter context established");
                        let device_descriptor = wgpu::DeviceDescriptor {
                            label: Some("main device"),
                            required_features: wgpu::Features::empty(),
                            required_limits: wgpu::Limits::default(),
                            experimental_features: wgpu::ExperimentalFeatures::disabled(),
                            memory_hints: wgpu::MemoryHints::default(),
                            trace: wgpu::Trace::Off,
                        };
                        if let Ok((device, queue)) =
                            pollster::block_on(adapter.request_device(&device_descriptor))
                        {
                            //println!("device connection established");
                            let size = window.inner_size();
                            let surface_capabilities = surface.get_capabilities(&adapter);
                            if let Some(surface_format) =
                                surface_capabilities.formats.iter().find(|f| f.is_srgb())
                            {
                                //println!("context established, opening window");
                                let surface_config = wgpu::SurfaceConfiguration {
                                    // See the comment on the matching field in
                                    // `create_staged_viewports` - the agent
                                    // port's screenshot readback needs `COPY_SRC`
                                    // on the surface texture's usage flags.
                                    usage: agent_port_usage(startup.agent_access_port.is_some()),
                                    format: *surface_format,
                                    color_space: wgpu::SurfaceColorSpace::Auto,
                                    width: size.width,
                                    height: size.height,
                                    present_mode: surface_capabilities.present_modes[0],
                                    desired_maximum_frame_latency: 2,
                                    alpha_mode: surface_capabilities.alpha_modes[0],
                                    view_formats: vec![],
                                };
                                surface.configure(&device, &surface_config);
                                //println!("surface configured");
                                let depth_texture =
                                    DepthTexture::new(&device, &surface_config, MULTI_SAMPLE_COUNT);
                                let multi_sample_texture = MultiSampleTexture::new(
                                    &device,
                                    &surface_config,
                                    MULTI_SAMPLE_COUNT,
                                );
                                //println!("depth texture created");
                                let mut scene_renderer = SceneRenderer::new(&device);
                                scene_renderer.build_shaders(
                                    &device,
                                    &surface_config,
                                    MULTI_SAMPLE_COUNT,
                                );
                                //println!("3d Shader compiled");
                                let mut ui_renderer = UIRenderer::new(&device, &queue);
                                ui_renderer.build_shaders(
                                    &device,
                                    &queue,
                                    &surface_config,
                                    MULTI_SAMPLE_COUNT,
                                );
                                let ui_renderer = Some(ui_renderer);
                                //println!("2d shader compiled");
                                let mut viewport_lookup = bimap::BiMap::new();
                                viewport_lookup.insert(startup.window_name.clone(), window_id);

                                // No explicit `Startup::page` -> the bootstrap
                                // window shows the page of the same name.
                                let initial_page = startup
                                    .page
                                    .clone()
                                    .unwrap_or_else(|| startup.window_name.clone());

                                let initial_viewport = Viewport::new(
                                    window,
                                    initial_page,
                                    surface,
                                    surface_config,
                                    depth_texture,
                                    multi_sample_texture,
                                );

                                let l = LayoutEngine::new((0.0, 0.0));

                                let mut viewports = HashMap::new();
                                viewports.insert(window_id, initial_viewport);
                                //println!("API initialized");
                                let mut api = API {
                                    staged_windows: Vec::new(),
                                    binder: Binder::new(),
                                    watch_path: startup.watch_path,
                                    instance,
                                    adapter,
                                    device,
                                    queue,
                                    scene_renderer,
                                    ui_renderer,
                                    l,
                                    image_frame_arena: Vec::new(),
                                    custom_shape_arena: Vec::new(),
                                    layout_text_arena: Vec::new(),
                                    custom_layout_settings_arena: Vec::new(),
                                    loaded_layout_images: HashMap::new(),
                                    loaded_layout_fonts: HashMap::new(),
                                    loaded_layout_shaders: HashMap::new(),
                                    model_ids: HashMap::new(),
                                    models: Vec::<Model>::new(),
                                    viewport_lookup,
                                    viewports,
                                    active_window: None,
                                    canvases: HashMap::new(),
                                    agent_port_active: startup.agent_access_port.is_some(),
                                    pending_screenshots: Vec::new(),
                                    pending_dumps: Vec::new(),
                                };

                                // `API` reads and parses the layout files itself - the
                                // app never sees the paths, the markdown, or the
                                // `Binder` they produce. Each file registers a page
                                // named after itself; the bootstrap viewport already
                                // points at its page (`Startup::page`, or
                                // `window_name`), so no follow-up `set_viewport_page`
                                // call is needed.
                                match api.watch_path.clone() {
                                    RunType::Once(dir) | RunType::Watch(dir) => {
                                        if let Err(error) = api.load_layout_directory(&dir) {
                                            eprintln!(
                                                "failed to load layout directory {dir:?}: {error}"
                                            );
                                        }
                                    }
                                    RunType::None => {}
                                }
                                if let RunType::Watch(dir) = &api.watch_path {
                                    spawn_layout_watcher(dir.clone(), self.app_events.clone());
                                }
                                if let Some(port) = startup.agent_access_port {
                                    agent_port::spawn_agent_listener(
                                        port,
                                        startup.window_name.clone(),
                                        self.app_events.clone(),
                                    );
                                }
                                api.redraw_viewport(window_id, &mut self.user_application);

                                self.user_application.onload(&mut api);

                                self.api = Some(api);
                            }
                        }
                    }
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: WindowId,
        event: winit::event::WindowEvent,
    ) {
        if let Some(api) = &mut self.api {
            // Anything that can change what a frame would draw schedules one.
            // `about_to_wait` turns the flag into a paced redraw request.
            let touches_render = matches!(
                &event,
                WindowEvent::Resized(_)
                    | WindowEvent::ScaleFactorChanged { .. }
                    | WindowEvent::MouseInput { .. }
                    | WindowEvent::MouseWheel { .. }
                    | WindowEvent::CursorMoved { .. }
                    | WindowEvent::KeyboardInput { .. }
            );

            match event {
                WindowEvent::CloseRequested => {
                    if api.viewports.len() < 2 {
                        event_loop.exit();
                    }
                    api.remove_viewport(window_id);
                }
                WindowEvent::Resized(size) => {
                    api.resize_viewport(window_id, size);
                }
                WindowEvent::ScaleFactorChanged {
                    scale_factor,
                    inner_size_writer: _,
                } => {
                    if let Some(viewport) = api.viewports.get_mut(&window_id) {
                        viewport.dpi_scale = scale_factor as f32;
                    }
                }
                WindowEvent::RedrawRequested => {
                    //println!("redraw requested");
                    api.redraw_viewport(window_id, &mut self.user_application);
                }
                WindowEvent::MouseInput {
                    device_id: _,
                    state,
                    button,
                } => {
                    if let Some(viewport) = api.viewports.get_mut(&window_id) {
                        match (button, state) {
                            (MouseButton::Left, ElementState::Pressed) => {
                                viewport.left_mouse_press()
                            }
                            (MouseButton::Left, ElementState::Released) => {
                                viewport.left_mouse_release()
                            }
                            (MouseButton::Right, ElementState::Pressed) => {
                                viewport.right_mouse_press()
                            }
                            (MouseButton::Right, ElementState::Released) => {
                                viewport.right_mouse_release()
                            }
                            (MouseButton::Middle, ElementState::Pressed) => {
                                viewport.middle_mouse_press()
                            }
                            (MouseButton::Middle, ElementState::Released) => {
                                viewport.middle_mouse_release()
                            }
                            _ => {}
                        }
                    }
                }
                WindowEvent::MouseWheel {
                    device_id: _,
                    delta,
                    phase: _,
                } => {
                    if let Some(viewport) = api.viewports.get_mut(&window_id) {
                        viewport.scroll_delta = match delta {
                            MouseScrollDelta::LineDelta(x, y) => (x, y),
                            MouseScrollDelta::PixelDelta(position) => position.into(),
                        };
                    }
                }
                WindowEvent::CursorMoved {
                    device_id: _,
                    position,
                } => {
                    if let Some(viewport) = api.viewports.get_mut(&window_id) {
                        let position: (f32, f32) = position.into();
                        // Accumulate over every move since the last frame (cleared
                        // in `end_frame`), not just the last segment - several
                        // `CursorMoved` events routinely arrive between two
                        // (frame-paced) redraws, and a drag handler that read only
                        // the final hop would lag the pointer.
                        viewport.mouse_delta.0 += position.0 - viewport.mouse_position.0;
                        viewport.mouse_delta.1 += position.1 - viewport.mouse_position.1;
                        viewport.mouse_position = position;
                    }
                }
                WindowEvent::KeyboardInput {
                    device_id: _,
                    event,
                    is_synthetic: _,
                } => {
                    if let Some(viewport) = api.viewports.get_mut(&window_id) {
                        viewport.key_events.push(event);
                    }
                }
                _ => {}
            }

            if touches_render
                && let Some(viewport) = api.viewports.get_mut(&window_id)
            {
                viewport.redraw_requested = true;
            }
        }
    }

    fn user_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        event: InternalEvents,
    ) {
        let Some(api) = &mut self.api else {
            return;
        };
        match event {
            InternalEvents::RebuildLayout(path) => match api.load_layout_file(&path) {
                Ok(_) => {
                    // The file may or may not be the page a viewport is
                    // currently showing (`load_layout_file` just re-registers
                    // it in the `Binder` under whatever page name it
                    // defines), so redraw every viewport rather than trying
                    // to work out which one(s) are affected.
                    for viewport in api.viewports.values_mut() {
                        viewport.redraw_requested = true;
                    }
                }
                Err(error) => {
                    eprintln!("failed to reload layout file {path:?}: {error}");
                }
            },
            InternalEvents::AgentInput { window, events, reply } => {
                let Some(window_id) = api.viewport_lookup.get_by_left(&window).copied() else {
                    let _ = reply.send(agent_port::AgentReply::error(format!(
                        "no such window: {window}"
                    )));
                    return;
                };
                if let Some(viewport) = api.viewports.get_mut(&window_id) {
                    agent_port::apply_events(viewport, &events);
                }
                let _ = reply.send(agent_port::AgentReply::ok(None));
            }
            InternalEvents::AgentScreenshot { window, path, reply } => {
                let Some(window_id) = api.viewport_lookup.get_by_left(&window).copied() else {
                    let _ = reply.send(agent_port::AgentReply::error(format!(
                        "no such window: {window}"
                    )));
                    return;
                };
                api.pending_screenshots.push(agent_port::PendingScreenshot {
                    window_id,
                    path,
                    reply,
                });
                if let Some(viewport) = api.viewports.get_mut(&window_id) {
                    viewport.redraw_requested = true;
                }
            }
            InternalEvents::AgentDump { window, kind, path, reply } => {
                let Some(window_id) = api.viewport_lookup.get_by_left(&window).copied() else {
                    let _ = reply.send(agent_port::AgentReply::error(format!(
                        "no such window: {window}"
                    )));
                    return;
                };
                // A layout-only dump doesn't need a live frame: the authored
                // per-page command list (`Binder::page`) persists across
                // frames, unlike render commands.
                if kind == agent_port::DumpKind::Layout {
                    let page = api.viewports.get(&window_id).map(|v| v.page.clone());
                    let text = match page.as_deref().and_then(|p| api.binder.page(p)) {
                        Some(layout) => format!("=== LAYOUT COMMANDS ===\n{layout:#?}\n"),
                        None => "=== LAYOUT COMMANDS ===\n(page not found)\n".to_string(),
                    };
                    match std::fs::write(&path, text) {
                        Ok(()) => {
                            let _ = reply.send(agent_port::AgentReply::ok(Some(
                                path.display().to_string(),
                            )));
                        }
                        Err(error) => {
                            let _ = reply.send(agent_port::AgentReply::error(error.to_string()));
                        }
                    }
                    return;
                }
                api.pending_dumps.push(agent_port::PendingDump {
                    window_id,
                    kind,
                    path,
                    reply,
                });
                if let Some(viewport) = api.viewports.get_mut(&window_id) {
                    viewport.redraw_requested = true;
                }
            }
        }
    }
}

/// The swapchain-surface usage flags a window's `SurfaceConfiguration`
/// should request: always `RENDER_ATTACHMENT`, plus `COPY_SRC` whenever the
/// agent access port is active, since its screenshot readback does a
/// `copy_texture_to_buffer` straight off the surface texture - wgpu
/// validates that the source texture's usage includes `COPY_SRC` and treats
/// a missing flag as a fatal (process-crashing) error, so this can't be
/// requested lazily only once a screenshot is actually asked for.
fn agent_port_usage(agent_port_active: bool) -> wgpu::TextureUsages {
    let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
    if agent_port_active {
        usage |= wgpu::TextureUsages::COPY_SRC;
    }
    usage
}

pub fn run<UserApp>(user_application: UserApp)
where
    UserApp: App,
{
    if let Ok(event_loop) = EventLoop::<InternalEvents>::with_user_event().build() {
        // `about_to_wait` sets the real policy from the first iteration.
        event_loop.set_control_flow(ControlFlow::Wait);
        let mut app = Application::new(event_loop.create_proxy(), user_application);
        event_loop.run_app(&mut app).unwrap();
    } else {
        panic!("Event loop creation failed.");
    }
}

/// Recursively collects every `.tmd` file under `dir`, sorted for
/// deterministic load order. Unreadable directories (missing, permission
/// denied, ...) just yield no files rather than erroring - the caller
/// (`API::load_layout_directory`) turns "found nothing" into its own
/// error with more context.
fn find_layout_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(find_layout_files(&path));
        } else if path.extension().is_some_and(|extension| extension == "tmd") {
            files.push(path);
        }
    }
    files.sort();
    files
}

/// Spawns the background thread backing `RunType::Watch`: watches `dir`
/// (recursively) for filesystem changes and, for every `.tmd` file that's
/// modified or created, sends `InternalEvents::RebuildLayout(path)`
/// through `app_events` so `Application::user_event` can reload it on the
/// winit event-loop thread - `notify`'s callback runs on its own thread
/// and can't safely touch `API`/`Binder` itself.
///
/// Mirrors the `notify` crate's own basic usage example: a
/// `recommended_watcher` feeding an `mpsc::channel`, read in a blocking
/// loop. The thread (and the watcher it owns) runs until either the
/// channel's sender is dropped (the watcher failing internally) or
/// `app_events.send_event` starts failing (the event loop, and so the
/// whole application, has shut down).
fn spawn_layout_watcher(dir: String, app_events: EventLoopProxy<InternalEvents>) {
    std::thread::spawn(move || {
        let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();

        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(watcher) => watcher,
            Err(error) => {
                eprintln!("failed to create layout watcher for {dir:?}: {error}");
                return;
            }
        };
        if let Err(error) = watcher.watch(Path::new(&dir), notify::RecursiveMode::Recursive) {
            eprintln!("failed to watch layout directory {dir:?}: {error}");
            return;
        }

        for res in rx {
            let event = match res {
                Ok(event) => event,
                Err(error) => {
                    eprintln!("layout watch error: {error}");
                    continue;
                }
            };
            if !matches!(
                event.kind,
                notify::EventKind::Modify(_) | notify::EventKind::Create(_)
            ) {
                continue;
            }
            for path in event.paths {
                if path.extension().is_none_or(|extension| extension != "tmd") {
                    continue;
                }
                if app_events
                    .send_event(InternalEvents::RebuildLayout(path))
                    .is_err()
                {
                    // The event loop (and so the whole application) is
                    // gone - stop watching.
                    return;
                }
            }
        }
    });
}
