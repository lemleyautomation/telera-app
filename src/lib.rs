pub use image::DynamicImage;
use notify::Watcher as _;
pub use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
pub use rkyv;
use std::{
    collections::HashMap,
    fmt::Debug,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, mpsc},
    time::Instant,
};
pub use symbol_table;
pub use telera_macros::*;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopProxy},
};
pub use winit::{
    dpi::LogicalSize,
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

mod ui_toolkit;
pub use ui_toolkit::layout_runner::{
    Binder, Config, DataSrc, Declaration, Element, EventContext, EventHandler, FieldAccess, Layout,
    LayoutRunnerCustomElements, LayoutRunnerReflection, ParsedLayout, normalize_field_symbol,
    process_layout,
};
pub use ui_toolkit::telera_layout::{Color, ElementConfiguration, TextConfig};
pub use ui_toolkit::treeview::{TreeViewEvents, TreeViewItem};
pub use ui_toolkit::ui_renderer::{UIImageDescriptor, UIRenderer as MT};
pub use ui_toolkit::ui_shapes::*;
use ui_toolkit::{
    telera_layout::LayoutEngine, ui_renderer::CustomLayoutSettings, ui_renderer::UIRenderer,
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
    pub initial_window: WindowAttributes,
    pub window_name: String,
    pub watch_path: RunType,
}

#[allow(unused_variables)]
pub trait App: LayoutRunnerReflection<Self::Event>
where
    <Self::Event as FromStr>::Err: Debug,
{
    /// The event enum this app's markdown-driven layout(s), if any,
    /// dispatch through. `API` is generic over `(Event, UserApp)` (so it
    /// can own a concretely-typed `Binder` instead of a type-erased one),
    /// which means every `App` has to name one here, even an app that
    /// never touches the layout-file/`Binder` machinery at all: declare a
    /// trivial single-variant enum (`#[derive(EventHandler)]` still applies
    /// - see `examples/basic.rs`) and an empty `impl ParserDataAccess<...>
    /// for YourApp {}` to go with it.
    type Event: FromStr + Clone + PartialEq + Debug + Default + EventHandler<UserApplication = Self>;

    /// Called once, before the graphics context (and so `API` itself)
    /// exists, to create the application's first window: the returned
    /// [`Startup::window_name`] is that window's name, and - until
    /// something (e.g. `API::set_viewport_page`) says otherwise - its page
    /// too. [`Startup::watch_path`], if set, names a markdown layout file
    /// for `API` to load itself before the first frame - the app never
    /// reads or parses it.
    ///
    /// Required, not defaulted: there's no generic "right" window to hand
    /// back, so every `App` supplies its own attributes and name here.
    /// Anything else one-time setup needs `&mut API` for (staging an
    /// image, ...) - `API` doesn't exist yet at this point - belongs in
    /// `update`, guarded by a flag on the app so it only runs once.
    fn initialize(&mut self) -> Startup;

    /// All application update logic
    ///
    /// This will be called at the beginning of each render loop
    fn update(&mut self, api: &mut API<Self::Event, Self>)
    where
        Self: Sized,
    {
    }

    fn layout(&mut self, page: &str, api: &mut API<Self::Event, Self>, mt: &mut UIRenderer)
    where
        Self: Sized;
}

pub struct API<Event, UserApp>
where
    Event: FromStr + Clone + PartialEq + Debug + Default + EventHandler<UserApplication = UserApp>,
    <Event as FromStr>::Err: Debug,
    UserApp: LayoutRunnerReflection<Event>,
{
    staged_windows: Vec<(String, String, WindowAttributes)>,

    /// Backs `run_layout` - loaded, if `Startup::watch_path` names a
    /// directory, before the first frame (see `Application::resumed`).
    /// Using the markdown-driven layout pipeline at all is opt-in: an
    /// `App` that leaves `watch_path` as `RunType::None` and just drives
    /// `api.l` directly (as `basic.rs` does) never touches this.
    binder: Binder<Event, UserApp>,
    /// Consulted once, in `Application::resumed`, to load the initial
    /// directory of layout files and (for `RunType::Watch`) to know which
    /// directory the background watcher thread should watch.
    watch_path: RunType,

    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub scene_renderer: SceneRenderer,
    pub ui_renderer: Option<UIRenderer>,
    pub l: LayoutEngine<UIImageDescriptor, CustomElement, CustomLayoutSettings>,
    model_ids: HashMap<String, usize>,
    models: Vec<Model>,

    viewport_lookup: bimap::BiMap<String, WindowId>,
    viewports: HashMap<WindowId, Viewport>,

    pub event_string: String,

    pub keyboard_buffer: Vec<KeyEvent>,

    pub window_size: (f32, f32),
    pub left_mouse_pressed: bool,
    pub left_mouse_down: bool,
    pub left_mouse_released: bool,
    pub left_mouse_clicked: bool,
    #[allow(dead_code)]
    left_mouse_double_clicked: bool,
    left_mouse_clicked_timer: Option<Instant>,
    _left_mouse_dbl_clicked_timer: Option<Instant>,

    right_mouse_pressed: bool,
    right_mouse_down: bool,
    right_mouse_released: bool,
    right_mouse_clicked: bool,
    right_mouse_clicked_timer: Option<Instant>,

    pub x_at_click: f32,
    pub y_at_click: f32,
    pub focus: u32,

    pub dpi_scale: f32,
    pub mouse_poistion: (f32, f32),
    pub mouse_delta: (f32, f32),
    #[allow(dead_code)]
    scroll_delta_time: Instant,
    scroll_delta_distance: (f32, f32),
}

// private api functions
impl<Event, UserApp> API<Event, UserApp>
where
    Event: FromStr + Clone + PartialEq + Debug + Default + EventHandler<UserApplication = UserApp>,
    <Event as FromStr>::Err: Debug,
    UserApp: LayoutRunnerReflection<Event>,
{
    fn request_redraw_viewport(&mut self, window_id: WindowId) {
        if let Some(viewport) = self.viewports.get_mut(&window_id) {
            viewport.window.request_redraw();
        }
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
        self.window_size = (size.width as f32, size.height as f32);
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

            let viewport = Viewport {
                window,
                page,
                surface,
                surface_config,
                depth_texture,
                multi_sample_texture,
            };

            self.viewport_lookup.insert(name, window_id);
            self.viewports.insert(window_id, viewport);
        }
    }
    fn redraw_viewport(&mut self, window_id: WindowId, user_application: &mut UserApp)
    where
        UserApp: App<Event = Event>,
    {
        let page = self
            .viewports
            .get(&window_id)
            .map(|viewport| viewport.page.clone());
        let mut render_commands = Vec::new();
        let ui_renderer = if let Some(viewport) = self.viewports.get_mut(&window_id)
            && let Some(page) = page
        {
            //println!("UI renderer good");
            let size: (f32, f32) = viewport.window.inner_size().into();
            self.dpi_scale = viewport.window.scale_factor() as f32;

            let mut ui_renderer = self.ui_renderer.take().unwrap();
            ui_renderer.dpi_scale = self.dpi_scale;
            ui_renderer.resize((size.0 as i32, size.1 as i32), &self.queue);

            self.l.set_layout_dimensions(
                ui_renderer.viewport_size.0 / ui_renderer.dpi_scale,
                ui_renderer.viewport_size.1 / ui_renderer.dpi_scale,
            );
            self.l.pointer_state(
                self.mouse_poistion.0 / ui_renderer.dpi_scale,
                self.mouse_poistion.1 / ui_renderer.dpi_scale,
                self.left_mouse_down,
            );
            self.l.update_scroll_containers(
                false,
                (self.scroll_delta_distance.0 / ui_renderer.dpi_scale) * 3.0,
                (self.scroll_delta_distance.1 / ui_renderer.dpi_scale) * 3.0,
                0.016,
            );
            self.l.begin_layout();
            user_application.layout(&page, self, &mut ui_renderer);
            render_commands = self.l.end_layout(&mut ui_renderer);
            //            println!("{:#?}", render_commands);
            self.scroll_delta_distance = (0.0, 0.0);
            self.scroll_delta_time = Instant::now();

            Some(ui_renderer)
        } else {
            None
        };

        if let Some(mut ui_renderer) = ui_renderer {
            if let Some(viewport) = self.viewports.get_mut(&window_id) {
                let drawable = viewport.get_current_texture();
                //println!("render surface acquired");
                let mut command_encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Render Encoder"),
                        });

                if MULTI_SAMPLE_COUNT == 1 {
                    //println!("beginning render pass");
                    let mut render_pass: wgpu::RenderPass =
                        command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("RenderPass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &drawable
                                    .texture
                                    .create_view(&wgpu::TextureViewDescriptor::default()), //&view_port.multi_sample_texture.view,
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

                    self.scene_renderer
                        .render(&mut self.models, &mut render_pass, &self.queue);

                    ui_renderer.render_layout(
                        render_commands,
                        &mut render_pass,
                        &self.device,
                        &self.queue,
                        &viewport.surface_config,
                    );

                    //println!("frame rendreed");
                } else {
                    let mut render_pass: wgpu::RenderPass =
                        command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("RenderPass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &viewport.multi_sample_texture.view,
                                resolve_target: Some(
                                    &drawable
                                        .texture
                                        .create_view(&wgpu::TextureViewDescriptor::default()),
                                ),
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

                    self.scene_renderer
                        .render(&mut self.models, &mut render_pass, &self.queue);
                }

                self.queue.submit(std::iter::once(command_encoder.finish()));
                drawable.present();
            }

            self.ui_renderer = Some(ui_renderer);

            self.left_mouse_pressed = false;
            self.left_mouse_released = false;
            self.left_mouse_clicked = false;
            self.left_mouse_double_clicked = false;
            if let Some(timer) = self.left_mouse_clicked_timer
                && timer.elapsed().as_millis() > 400
            {
                self.left_mouse_clicked_timer = None;
            }
            // if let Some(timer) = self.core.left_mouse_dbl_clicked_timer
            // && timer.elapsed().as_millis() > 300 {
            //     self.core.left_mouse_dbl_clicked_timer = None;
            // }
            self.right_mouse_pressed = false;
            self.right_mouse_released = false;
            self.right_mouse_clicked = false;
            if let Some(timer) = self.right_mouse_clicked_timer
                && timer.elapsed().as_millis() > 300
            {
                self.right_mouse_clicked_timer = None;
            }
        }
    }
}

/// public api functions
impl<Event, UserApp> API<Event, UserApp>
where
    Event: FromStr + Clone + PartialEq + Debug + Default + EventHandler<UserApplication = UserApp>,
    <Event as FromStr>::Err: Debug,
    UserApp: LayoutRunnerReflection<Event>,
{
    pub fn create_viewport(&mut self, name: &str, page: &str, attributes: WindowAttributes) {
        self.staged_windows
            .push((name.to_string(), page.to_string(), attributes));
    }
    pub fn create_default_viewport(&mut self) {
        let new_window = Window::default_attributes().with_inner_size(LogicalSize::new(800, 600));
        self.staged_windows
            .push(("Main".to_string(), "Main".to_string(), new_window));
    }
    pub fn add_image(&mut self, name: &str, image: DynamicImage) {
        if let Some(ui_renderer) = &mut self.ui_renderer {
            ui_renderer.stage_atlas(name.to_string(), image);
        }
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
            window.window.request_redraw();
        }
    }

    /// Reads and parses a single markdown layout file (see
    /// `process_layout`) and registers the page and any reusable snippets
    /// it defines, returning the page's name. Called both for the initial
    /// directory scan (`load_layout_directory`) and, one file at a time,
    /// whenever `Application::user_event` reloads a file the watcher
    /// thread reported as changed - an `App` never calls this itself, let
    /// alone touches the `Binder` it feeds.
    fn load_layout_file(&mut self, path: &Path) -> Result<String, String> {
        let markdown = std::fs::read_to_string(path)
            .map_err(|error| format!("failed to read layout file {path:?}: {error}"))?;
        self.binder.load_layout(&markdown)
    }

    /// Recursively loads every `.md` file under `dir` into the `Binder`
    /// (see `load_layout_file`), skipping (and logging) any individual
    /// file that fails to read or parse rather than aborting the whole
    /// scan. Returns the page name of the first file (in sorted order)
    /// that loaded successfully, so `Application::resumed` has something
    /// to point the bootstrap viewport at; errors only if `dir` contains
    /// no `.md` files at all, or none of them parsed.
    fn load_layout_directory(&mut self, dir: &str) -> Result<String, String> {
        let files = find_layout_files(Path::new(dir));
        if files.is_empty() {
            return Err(format!("no .md layout files found under {dir:?}"));
        }

        let mut first_page = None;
        for file in files {
            match self.load_layout_file(&file) {
                Ok(page_name) => {
                    if first_page.is_none() {
                        first_page = Some(page_name);
                    }
                }
                Err(error) => eprintln!("failed to load layout file {file:?}: {error}"),
            }
        }

        first_page.ok_or_else(|| format!("no layout files under {dir:?} parsed successfully"))
    }

    /// Runs `page` (as loaded from `Startup::watch_path`) for this frame
    /// and dispatches whatever events fired straight back into `user_app`
    /// via `EventHandler::dispatch`. Typically the entire body of
    /// `App::layout` for an app driven by a markdown layout:
    ///
    /// ```ignore
    /// fn layout(&mut self, page: &str, api: &mut API<Self::Event, Self>, mt: &mut MT) {
    ///     api.run_layout(page, mt, self);
    /// }
    /// ```
    pub fn run_layout(&mut self, page: &str, mt: &mut MT, user_app: &mut UserApp)
    where
        UserApp: LayoutRunnerCustomElements<Event, UserApp>,
    {
        // `self.binder` and `user_app` are already two separate places (a
        // field of `API`, and the caller's own, unrelated `&mut UserApp`),
        // so unlike a `Binder` stored *on* `UserApp` itself, there's no
        // self-borrow to fight here - just take it out for the duration of
        // the call so `set_page` and `dispatch` can each reborrow
        // `user_app` in turn instead of fighting over one long-lived borrow.
        let mut binder = std::mem::take(&mut self.binder);
        if let Some(events) = binder.set_page(page, self, mt, user_app) {
            for (event, context) in events {
                event.dispatch(user_app, context, self);
            }
        }
        self.binder = binder;
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
    <UserApp::Event as FromStr>::Err: Debug,
{
    core: Option<API<UserApp::Event, UserApp>>,
    user_application: UserApp,

    /// Cloned into `spawn_layout_watcher` for `RunType::Watch` so its
    /// background thread can send `InternalEvents::RebuildLayout` back to
    /// `Application::user_event` on the winit event-loop thread.
    app_events: EventLoopProxy<InternalEvents>,
}

impl<UserApp> Application<UserApp>
where
    UserApp: App,
    <UserApp::Event as FromStr>::Err: Debug,
{
    pub fn new(app_events: EventLoopProxy<InternalEvents>, user_application: UserApp) -> Self {
        Application {
            core: None,
            app_events,
            user_application,
        }
    }
}

impl<UserApp> ApplicationHandler<InternalEvents> for Application<UserApp>
where
    UserApp: App,
    <UserApp::Event as FromStr>::Err: Debug,
{
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        //println!("building context");
        if self.core.is_none() {
            let startup = self.user_application.initialize();

            if let Ok(window) = event_loop.create_window(startup.initial_window) {
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

                                let initial_viewport = Viewport {
                                    window,
                                    page: startup.window_name.clone(),
                                    surface,
                                    surface_config,
                                    depth_texture,
                                    multi_sample_texture,
                                };

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
                                    model_ids: HashMap::new(),
                                    models: Vec::<Model>::new(),
                                    viewport_lookup,
                                    viewports,
                                    event_string: "".to_string(),
                                    keyboard_buffer: Vec::new(),
                                    window_size: (0.0, 0.0),
                                    left_mouse_pressed: false,
                                    left_mouse_down: false,
                                    left_mouse_released: false,
                                    left_mouse_clicked: false,
                                    left_mouse_double_clicked: false,
                                    left_mouse_clicked_timer: None,
                                    _left_mouse_dbl_clicked_timer: None,

                                    right_mouse_pressed: false,
                                    right_mouse_down: false,
                                    right_mouse_released: false,
                                    right_mouse_clicked: false,
                                    right_mouse_clicked_timer: None,
                                    x_at_click: 0.0,
                                    y_at_click: 0.0,
                                    focus: 0,
                                    dpi_scale: 0.0,
                                    mouse_poistion: (0.0, 0.0),
                                    mouse_delta: (0.0, 0.0),
                                    scroll_delta_time: Instant::now(),
                                    scroll_delta_distance: (0.0, 0.0),
                                };

                                // `API` reads and parses the layout file itself - the app
                                // never sees the path, the markdown, or the `Binder` it
                                // produces. Point the bootstrap window/page at whatever
                                // page that file actually defines, so a plain
                                // `watch_path` is enough on its own; no follow-up
                                // `set_viewport_page` call needed.
                                match api.watch_path.clone() {
                                    RunType::Once(dir) | RunType::Watch(dir) => {
                                        match api.load_layout_directory(&dir) {
                                            Ok(page_name) => {
                                                if let Some(viewport) =
                                                    api.viewports.get_mut(&window_id)
                                                {
                                                    viewport.page = page_name;
                                                }
                                            }
                                            Err(error) => {
                                                eprintln!(
                                                    "failed to load layout directory {dir:?}: {error}"
                                                );
                                            }
                                        }
                                    }
                                    RunType::None => {}
                                }
                                if let RunType::Watch(dir) = &api.watch_path {
                                    spawn_layout_watcher(dir.clone(), self.app_events.clone());
                                }
                                //api.redraw_viewport(window_id, &mut self.user_application);

                                self.core = Some(api);
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
        if let Some(api) = &mut self.core {
            api.create_staged_viewports(event_loop);
            self.user_application.update(api);
            api.scene_renderer.camera_controller.process_events(&event);

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
                    api.dpi_scale = scale_factor as f32;
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
                    match button {
                        MouseButton::Left => {
                            match state {
                                ElementState::Pressed => {
                                    api.left_mouse_pressed = true;
                                    api.left_mouse_down = true;
                                    if api.left_mouse_clicked_timer.is_none() {
                                        api.left_mouse_clicked_timer = Some(Instant::now());
                                    }
                                    // else {
                                    //     self.core.left_mouse_clicked_timer = None;
                                    //     self.core.left_mouse_dbl_clicked_timer = Some(Instant::now());
                                    // }
                                    api.x_at_click = api.mouse_poistion.0 / api.dpi_scale;
                                    api.y_at_click = api.mouse_poistion.1 / api.dpi_scale;
                                }
                                ElementState::Released => {
                                    if let Some(timer) = api.left_mouse_clicked_timer
                                        && timer.elapsed().as_millis() < 400
                                    {
                                        api.left_mouse_clicked = true;
                                        api.left_mouse_clicked_timer = None;
                                    }
                                    // if let Some(timer) = self.core.left_mouse_dbl_clicked_timer
                                    // && timer.elapsed().as_millis() < 300 {
                                    //     self.core.left_mouse_double_clicked = true;
                                    //     self.core.left_mouse_dbl_clicked_timer = None;
                                    // }
                                    api.left_mouse_down = false;
                                    api.left_mouse_released = true;
                                }
                            }
                        }
                        MouseButton::Right => match state {
                            ElementState::Pressed => {
                                api.right_mouse_pressed = true;
                                api.right_mouse_down = true;
                                if api.right_mouse_clicked_timer.is_none() {
                                    api.right_mouse_clicked_timer = Some(Instant::now());
                                }
                                api.x_at_click = api.mouse_poistion.0 / api.dpi_scale;
                                api.y_at_click = api.mouse_poistion.1 / api.dpi_scale;
                            }
                            ElementState::Released => {
                                if let Some(timer) = api.right_mouse_clicked_timer
                                    && timer.elapsed().as_millis() < 300
                                {
                                    api.right_mouse_clicked = true;
                                    api.right_mouse_clicked_timer = None;
                                }
                                api.right_mouse_down = false;
                                api.right_mouse_released = true;
                            }
                        },

                        _ => {}
                    }
                }
                WindowEvent::MouseWheel {
                    device_id: _,
                    delta,
                    phase: _,
                } => {
                    api.scroll_delta_distance = match delta {
                        MouseScrollDelta::LineDelta(x, y) => (x, y),
                        MouseScrollDelta::PixelDelta(position) => position.into(),
                    };
                    api.request_redraw_viewport(window_id);
                }
                WindowEvent::CursorMoved {
                    device_id: _,
                    position,
                } => {
                    api.request_redraw_viewport(window_id);
                    api.mouse_delta.0 = position.x as f32 - api.mouse_poistion.0;
                    api.mouse_delta.1 = position.y as f32 - api.mouse_poistion.1;
                    api.mouse_poistion = position.into();
                }
                WindowEvent::KeyboardInput {
                    device_id: _,
                    event,
                    is_synthetic: _,
                } => {
                    api.keyboard_buffer.push(event);
                }
                _ => {}
            }
            //api.request_redraw_viewport(window_id);
        }
    }

    fn user_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        event: InternalEvents,
    ) {
        let InternalEvents::RebuildLayout(path) = event;
        let Some(api) = &mut self.core else {
            return;
        };
        match api.load_layout_file(&path) {
            Ok(_) => {
                // The file may or may not be the page a viewport is
                // currently showing (`load_layout_file` just re-registers
                // it in the `Binder` under whatever page name it
                // defines), so redraw every viewport rather than trying
                // to work out which one(s) are affected.
                for viewport in api.viewports.values() {
                    viewport.window.request_redraw();
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
    <UserApp::Event as FromStr>::Err: Debug,
{
    if let Ok(event_loop) = EventLoop::<InternalEvents>::with_user_event().build() {
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
