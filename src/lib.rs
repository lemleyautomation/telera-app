pub use image::{self, DynamicImage, load_from_memory};
use notify::Watcher as _;
pub use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
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
    keyboard::{self, Key, KeyCode, NamedKey, PhysicalKey},
    window::{Window, WindowAttributes, WindowId},
};

mod graphics;
pub use graphics::model::{
    BaseMesh, Euler, Model, Quaternion, Transform, TransformMatrix, load_model_gltf,
};
use graphics::{
    depth_texture::DepthTexture, multi_sample_texture::MultiSampleTexture,
    scene_renderer::SceneRenderer, texture, viewport::Viewport,
};
const MULTI_SAMPLE_COUNT: u32 = 1;

mod ui_renderer;
pub use ui_renderer::layout_runner::{
    Binder, Config, DataSrc, Declaration, Element, EventContext, FieldAccess, ImageLoad, Layout,
    LayoutReflector, LayoutRunnerReflection, ParsedLayout, normalize_field_symbol, process_layout,
};
pub use ui_renderer::telera_layout::{Color, ElementConfiguration, TextConfig};
pub use ui_renderer::ui_renderer::{CustomElement, LineConfig, UIImageDescriptor};
use ui_renderer::{
    telera_layout::LayoutEngine, ui_renderer::CustomLayoutSettings,
    ui_renderer::commands_fingerprint, ui_renderer::UIRenderer,
};

pub enum APIError {
    ModelNotFound,
}
/// Sent through the winit event loop's user-event channel by the
/// background thread `spawn_layout_watcher` starts for `RunType::Watch` -
/// `notify`'s watcher callback runs off the event-loop thread and can't
/// touch `API`/`Binder` directly, so it just names the file that changed
/// and lets `Application::user_event` do the actual reload.
#[derive(Debug)]
enum InternalEvents {
    RebuildLayout(PathBuf),
}

/// How an `App` wants its markdown layout file(s) loaded, per
/// [`Startup::watch_path`]. In every non-`None` case the string names a
/// *directory* - `API` recursively loads every `.md` file it finds there
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
    /// i.e. the layout file named `<window_name>.md` (or a page registered
    /// under that name by hand). Change it later with [`API::set_viewport_page`].
    pub page: Option<String>,
    pub watch_path: RunType,
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

    /// All application update logic
    ///
    /// This will be called at the beginning of each render loop
    fn update(&mut self, api: &mut API) {}

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
    /// Atlases loaded from `` `load` `` directives in layout files, keyed by
    /// atlas name -> source path, so re-parsing a file (hot reload) can tell an
    /// already-loaded image from a new one.
    loaded_layout_images: HashMap<String, String>,
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
    pub fn mouse_position(&self) -> (f32, f32) {
        self.active_viewport().map_or((0.0, 0.0), |v| v.mouse_position)
    }
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

    /// Key events that landed on the active window since its last redraw.
    /// Cleared by `Viewport::end_frame` once the frame has consumed them, so a
    /// layout or handler must read them the same frame they arrive.
    pub fn key_events(&self) -> &[KeyEvent] {
        self.active_viewport().map_or(&[], |v| v.key_events.as_slice())
    }
    pub fn event_string(&self) -> &str {
        self.active_viewport().map_or("", |v| v.event_string.as_str())
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
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: *surface_format,
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

        let page = self
            .viewports
            .get(&window_id)
            .map(|viewport| viewport.page.clone());
        let mut render_commands = Vec::new();
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

            let mut ui_renderer = self.ui_renderer.take().unwrap();
            ui_renderer.dpi_scale = dpi_scale;
            ui_renderer.resize((size.0 as i32, size.1 as i32), &self.queue);

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
            self.l.begin_layout(ui_renderer);
            match self.watch_path {
                RunType::None => user_application.layout(&page, self),
                RunType::Once(_) | RunType::Watch(_) => self.run_layout(&page, user_application),
            }
            let (commands, ui_renderer) = self.l.end_layout();
            render_commands = commands;
            //            println!("{:#?}", render_commands);

            Some(ui_renderer)
        } else {
            None
        };

        if let Some(mut ui_renderer) = ui_renderer {
            if let Some(viewport) = self.viewports.get_mut(&window_id) {
                let drawable = viewport.get_current_texture();
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
                let aspect = viewport.aspect();
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
                            });
                        ui_renderer.render_layout(
                            render_commands,
                            &mut ui_pass,
                            &self.device,
                            &self.queue,
                            &viewport.surface_config,
                        );
                        drop(ui_pass);
                        ui_surface.last_fingerprint = Some(fingerprint);
                    }

                    // 2. Draw the 3D scene into the swapchain.
                    {
                        let mut scene_pass =
                            command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("Scene Pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &drawable_view,
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
                            });
                        self.scene_renderer.render(
                            &mut self.models,
                            &mut scene_pass,
                            &self.queue,
                            aspect,
                        );
                    }

                    // 3. Composite the cached UI texture over the scene.
                    {
                        let mut composite_pass =
                            command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("UI Composite Pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &drawable_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
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
                        });

                    self.scene_renderer.render(
                        &mut self.models,
                        &mut render_pass,
                        &self.queue,
                        aspect,
                    );
                }

                self.queue.submit(std::iter::once(command_encoder.finish()));
                drawable.present();
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

    /// Reads and parses a single markdown layout file (see
    /// `process_layout`) and registers it - along with any reusable snippets
    /// it defines - as the page named after the file itself (its name with
    /// the `.md` extension stripped), returning that page name. Called both
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
        let image_loads = self.binder.load_layout(&page_name, &markdown)?;

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

        Ok(page_name)
    }

    /// Recursively loads every `.md` file under `dir` into the `Binder`
    /// (see `load_layout_file`), each registered as a page named after its
    /// file, skipping (and logging) any individual file that fails to read
    /// or parse rather than aborting the whole scan. Errors only if `dir`
    /// contains no `.md` files at all, or none of them parsed.
    fn load_layout_directory(&mut self, dir: &str) -> Result<(), String> {
        let files = find_layout_files(Path::new(dir));
        if files.is_empty() {
            return Err(format!("no .md layout files found under {dir:?}"));
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
        self.user_application.update(api);
        api.create_staged_viewports(event_loop);

        let now = Instant::now();
        let camera_moving = api.scene_renderer.camera_controller.is_moving();

        let mut next_wake: Option<Instant> = None;
        for viewport in api.viewports.values_mut() {
            let minimized =
                viewport.surface_config.width == 0 || viewport.surface_config.height == 0;
            let wants_frame = !minimized
                && (viewport.redraw_requested
                    || viewport.continuous_rendering
                    || camera_moving);
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
                    };
                    //println!("requesting adapater context");
                    if let Some(adapter) =
                        pollster::block_on(instance.request_adapter(&adapter_options))
                    {
                        //println!("adapter context established");
                        let device_descriptor = wgpu::DeviceDescriptor {
                            label: Some("main device"),
                            required_features: wgpu::Features::empty(),
                            required_limits: wgpu::Limits::default(),
                            memory_hints: wgpu::MemoryHints::default(),
                        };
                        if let Ok((device, queue)) =
                            pollster::block_on(adapter.request_device(&device_descriptor, None))
                        {
                            //println!("device connection established");
                            let size = window.inner_size();
                            let surface_capabilities = surface.get_capabilities(&adapter);
                            if let Some(surface_format) =
                                surface_capabilities.formats.iter().find(|f| f.is_srgb())
                            {
                                //println!("context established, opening window");
                                let surface_config = wgpu::SurfaceConfiguration {
                                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                                    format: *surface_format,
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
                                    loaded_layout_images: HashMap::new(),
                                    model_ids: HashMap::new(),
                                    models: Vec::<Model>::new(),
                                    viewport_lookup,
                                    viewports,
                                    active_window: None,
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
            let camera_handled = api.scene_renderer.camera_controller.process_events(&event);

            // Anything that can change what a frame would draw schedules one.
            // `about_to_wait` turns the flag into a paced redraw request.
            let touches_render = camera_handled
                || matches!(
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
                        viewport.mouse_delta.0 = position.0 - viewport.mouse_position.0;
                        viewport.mouse_delta.1 = position.1 - viewport.mouse_position.1;
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
        let InternalEvents::RebuildLayout(path) = event;
        let Some(api) = &mut self.api else {
            return;
        };
        match api.load_layout_file(&path) {
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
        }
    }
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

/// Recursively collects every `.md` file under `dir`, sorted for
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
        } else if path.extension().is_some_and(|extension| extension == "md") {
            files.push(path);
        }
    }
    files.sort();
    files
}

/// Spawns the background thread backing `RunType::Watch`: watches `dir`
/// (recursively) for filesystem changes and, for every `.md` file that's
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
                if path.extension().is_none_or(|extension| extension != "md") {
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
