use notify::{RecursiveMode, Watcher};
pub use rkyv;
use std::{
    collections::HashMap,
    fmt::Debug,
    fs::read_to_string,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Instant,
};

#[cfg(target_os = "windows")]
type DirectoryWatcher = notify::ReadDirectoryChangesWatcher;

#[cfg(target_os = "linux")]
type DirectoryWatcher = notify::INotifyWatcher;

pub use image::DynamicImage;
pub use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
pub use symbol_table;
pub use telera_macros::*;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
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
pub use ui_toolkit::{
    layout_types::*, markdown::*, page_set::*, treeview::TreeViewEvents, treeview::TreeViewItem,
    ui_renderer::UIImageDescriptor,
};
use ui_toolkit::{
    telera_layout::LayoutEngine, ui_renderer::CustomLayoutSettings, ui_renderer::UIRenderer,
    ui_shapes::CustomElement,
};

pub enum APIError {
    ModelNotFound,
}

#[allow(dead_code)]
#[derive(Debug)]
enum InternalEvents {
    Hi,
    RebuildLayout(String),
}

#[derive(Clone, Default)]
pub struct EventContext {
    pub text: Option<String>,
    pub code: Option<u32>,
    pub code2: Option<u32>,
}

impl EventContext {
    pub fn new() -> Self {
        EventContext {
            text: None,
            code: None,
            code2: None,
        }
    }
    pub fn from_code(code: u32) -> Self {
        EventContext {
            text: None,
            code: Some(code),
            code2: None,
        }
    }
    pub fn from_code2(code2: u32) -> Self {
        EventContext {
            text: None,
            code: None,
            code2: Some(code2),
        }
    }
    pub fn code(mut self, code: u32) -> Self {
        self.code = Some(code);
        self
    }
    pub fn code2(mut self, code2: u32) -> Self {
        self.code2 = Some(code2);
        self
    }
}

pub trait EventHandler {
    type UserApplication;
    #[allow(unused_variables)]
    fn dispatch(
        &self,
        app: &mut Self::UserApplication,
        context: Option<EventContext>,
        api: &mut API,
    ) {
    }
}

#[allow(unused_variables)]
pub trait App {
    /// called before Graphics card is initiallized.
    /// if the user doesn't implement this function,
    /// a default window of 800x600 will be created
    fn initial_window(&self) -> (WindowAttributes, String) {
        (
            winit::window::Window::default_attributes().with_inner_size(LogicalSize::new(800, 600)),
            "testing".to_string(),
        )
    }
    /// called once before start
    fn initialize(&mut self, api: &mut API) {
        api.create_default_viewport();
    }

    /// All application update logic
    ///
    /// This will be called at the beginning of each render loop
    fn update(&mut self, api: &mut API) {}
}

pub struct API {
    staged_windows: Vec<(String, String, WindowAttributes)>,

    #[warn(dead_code)]
    instance: wgpu::Instance,
    #[warn(dead_code)]
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub scene_renderer: SceneRenderer,
    ui_renderer: Option<UIRenderer>,
    pub ui_layout: LayoutEngine<UIRenderer, UIImageDescriptor, CustomElement, CustomLayoutSettings>,
    model_ids: HashMap<String, usize>,
    models: Vec<Model>,

    viewport_lookup: bimap::BiMap<String, WindowId>,
    viewports: HashMap<WindowId, Viewport>,

    pub event_string: String,

    left_mouse_pressed: bool,
    left_mouse_down: bool,
    left_mouse_released: bool,
    left_mouse_clicked: bool,
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
    scroll_delta_time: Instant,
    scroll_delta_distance: (f32, f32),
}

// private api functions
impl API {
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
        if let Some(viewport) = self.viewports.get_mut(&window_id) {
            viewport.resize(&self.device, size, MULTI_SAMPLE_COUNT);
        }
    }
    fn create_staged_viewports(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        for _ in 0..self.staged_windows.len() {
            //let (name, page, attr) = self.staged_windows.pop().unwrap();

            //if self.viewport_lookup.get_by_left(&name).is_some() {
            //    continue;
            //}

            //let viewport = attr.build_viewport(event_loop, page, &self.ctx, MULTI_SAMPLE_COUNT);

            //viewport.window.set_title(&name);
            //let window_id = viewport.window.id();

            //let ui_renderer = self.ui_renderer.as_mut().unwrap();
            //match ui_renderer.render_pipeline {
            //    Some(_) => {}
            //    None => ui_renderer.build_shaders(
            //        &self.ctx.device,
            //        &self.ctx.queue,
            //        &viewport.config,
            //        MULTI_SAMPLE_COUNT,
            //    ),
            //}

            //match self.scene_renderer.render_pipeline {
            //    Some(_) => {}
            //    None => self.scene_renderer.build_shaders(
            //        &self.device,
            //        &viewport.config,
            //        MULTI_SAMPLE_COUNT,
            //    ),
            //}

            //self.viewport_lookup.insert(name.clone(), window_id);
            //self.viewports.insert(window_id, viewport);
        }
        self.staged_windows.clear();
    }
    fn redraw_viewport<UserEvents, UserApp>(
        &mut self,
        window_id: WindowId,
        layout_binder: &mut Binder<UserEvents, UserApp>,
        user_application: &mut UserApp,
    ) where
        UserApp: ParserDataAccess<UserEvents>,
        UserEvents:
            FromStr + Debug + Default + Clone + PartialEq + EventHandler<UserApplication = UserApp>,
        <UserEvents as FromStr>::Err: Debug + Default,
    {
        //println!("attempting viewport redraw");
        //let x = self.viewports.keys();
        //println!(
        //    "available viewports: {:?}, requested viewport: {:?}",
        //    x, window_id
        //);
        let ui_renderer = if let Some(viewport) = self.viewports.get_mut(&window_id) {
            //println!("UI renderer good");
            let size: (f32, f32) = viewport.window.inner_size().into();
            self.dpi_scale = viewport.window.scale_factor() as f32;

            let mut ui_renderer = self.ui_renderer.take().unwrap();
            ui_renderer.dpi_scale = self.dpi_scale;
            ui_renderer.resize((size.0 as i32, size.1 as i32), &self.queue);

            self.ui_layout
                .set_layout_dimensions(size.0 / self.dpi_scale, size.1 / self.dpi_scale);

            self.ui_layout.pointer_state(
                self.mouse_poistion.0 / self.dpi_scale,
                self.mouse_poistion.1 / self.dpi_scale,
                self.left_mouse_down,
            );
            self.ui_layout.update_scroll_containers(
                false,
                self.scroll_delta_distance.0,
                self.scroll_delta_distance.1,
                self.scroll_delta_time.elapsed().as_secs_f32(),
            );
            self.scroll_delta_distance = (0.0, 0.0);
            self.scroll_delta_time = Instant::now();

            Some(ui_renderer)
        } else {
            None
        };

        if let Some(ui_renderer) = ui_renderer {
            self.ui_layout.begin_layout(ui_renderer);
            //println!("beginning UI layout");
            if let Ok(events) = layout_binder.set_page(window_id, self, user_application) {
                for (event, event_context) in events.iter() {
                    event.dispatch(user_application, event_context.clone(), self);
                }
                //println!("UI layout successful");
            }

            let (render_commands, mut ui_renderer) = self.ui_layout.end_layout();

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

                    ui_renderer.render_layout(
                        render_commands,
                        &mut render_pass,
                        &self.device,
                        &self.queue,
                        &viewport.surface_config,
                    );
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
impl API {
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

struct Application<UserApp, UserEvents>
where
    UserEvents:
        FromStr + Clone + PartialEq + Default + Debug + EventHandler<UserApplication = UserApp>,
    <UserEvents as FromStr>::Err: Debug,
    UserApp: App + ParserDataAccess<UserEvents>,
{
    layout_binder: Binder<UserEvents, UserApp>,
    core: Option<API>,
    user_application: UserApp,

    #[allow(dead_code)]
    app_events: EventLoopProxy<InternalEvents>,
    #[allow(dead_code)]
    watcher: Option<DirectoryWatcher>,
}

impl<UserEvents, UserApp> Application<UserApp, UserEvents>
where
    UserEvents:
        FromStr + Clone + PartialEq + Debug + Default + EventHandler<UserApplication = UserApp>,
    <UserEvents as FromStr>::Err: Debug + Default,
    UserApp: App + ParserDataAccess<UserEvents>,
{
    pub fn new(
        app_events: EventLoopProxy<InternalEvents>,
        user_application: UserApp,
        watcher: Option<DirectoryWatcher>,
    ) -> Self {
        let layout_binder = Binder::new();

        let mut application = Application {
            layout_binder,
            core: None,
            app_events,
            user_application,
            watcher,
        };

        application.load_layouts("src/layouts".to_string());

        application
    }

    fn load_layouts(&mut self, dir: String) {
        let entries = std::fs::read_dir(dir).unwrap_or_else(|e| {
            eprintln!("Error reading directory: {}", e);
            std::process::exit(1);
        });

        for dir in entries {
            #[allow(for_loops_over_fallibles)]
            for dir in dir {
                let entry = dir.path();
                if entry.is_file()
                    && let Ok(file) = read_to_string(entry)
                    && let Ok((page_name, page_layout, reusables)) =
                        process_layout::<UserEvents>(file)
                {
                    self.layout_binder.add_page(&page_name, page_layout);
                    for (name, reusable) in reusables {
                        self.layout_binder.add_reusable(&name, reusable);
                    }
                }
            }
        }
    }
}

impl<UserEvents, UserApp> ApplicationHandler<InternalEvents> for Application<UserApp, UserEvents>
where
    UserEvents:
        FromStr + Clone + PartialEq + Debug + Default + EventHandler<UserApplication = UserApp>,
    UserEvents: EventHandler<UserApplication = UserApp>,
    <UserEvents as FromStr>::Err: Debug + Default,
    UserApp: App + ParserDataAccess<UserEvents>,
{
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        //println!("building context");
        if self.core.is_none() {
            let (new_window, window_name) = self.user_application.initial_window();

            if let Ok(window) = event_loop.create_window(new_window) {
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
                                viewport_lookup.insert(window_name.clone(), window_id);

                                let initial_viewport = Viewport {
                                    window,
                                    page: window_name.clone(),
                                    surface,
                                    surface_config,
                                    depth_texture,
                                    multi_sample_texture,
                                };

                                let mut viewports = HashMap::new();
                                viewports.insert(window_id, initial_viewport);
                                //println!("viewport configured");
                                let ui_layout = LayoutEngine::<
                                    UIRenderer,
                                    UIImageDescriptor,
                                    CustomElement,
                                    CustomLayoutSettings,
                                >::new(size.into());
                                println!("API initialized");
                                let mut api = API {
                                    staged_windows: Vec::new(),
                                    instance,
                                    adapter,
                                    device,
                                    queue,
                                    scene_renderer,
                                    ui_renderer,
                                    ui_layout,
                                    model_ids: HashMap::new(),
                                    models: Vec::<Model>::new(),
                                    viewport_lookup,
                                    viewports,
                                    event_string: "".to_string(),
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

                                api.redraw_viewport(
                                    window_id,
                                    &mut self.layout_binder,
                                    &mut self.user_application,
                                );

                                self.user_application.initialize(&mut api);
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
                    return;
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
                    api.redraw_viewport(
                        window_id,
                        &mut self.layout_binder,
                        &mut self.user_application,
                    );
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
                    //viewport.window.request_redraw();
                }
                WindowEvent::CursorMoved {
                    device_id: _,
                    position,
                } => {
                    api.mouse_delta.0 = position.x as f32 - api.mouse_poistion.0;
                    api.mouse_delta.1 = position.y as f32 - api.mouse_poistion.1;
                    api.mouse_poistion = position.into();
                }
                _ => {}
            }
            api.request_redraw_viewport(window_id);
        }
    }

    fn user_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        event: InternalEvents,
    ) {
        //println!("file changed");
        if let InternalEvents::RebuildLayout(path) = event {
            //println!("{:?}", &path);
            self.load_layouts(path);
        }
    }
}

fn watch_file(file: &str, sender: EventLoopProxy<InternalEvents>) -> Result<DirectoryWatcher, ()> {
    if let Ok(mut watcher) =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            //println!("file event result: {:?}", &event);
            if let Ok(event) = event {
                let path = event.paths.first();
                //println!("file path: {:?}", path);
                if let Some(path) = path {
                    let event_kind = event.kind;
                    //println!("file event kind: {:?}", event_kind);
                    if let notify::EventKind::Modify(_kind) = event_kind {
                        let path = path
                            .parent()
                            .unwrap()
                            .to_owned()
                            .to_str()
                            .unwrap()
                            .to_owned();
                        //println!(
                        //    "sending {:?}, to main thread {:?}",
                        //    kind,
                        let _ = sender.send_event(InternalEvents::RebuildLayout(path));
                        //);
                    }
                }
            }
        })
        && let Ok(()) = watcher.watch(Path::new(file), RecursiveMode::NonRecursive)
    {
        return Ok(watcher);
    }

    Err(())
}

pub fn run<UserEvents, UserApp>(user_application: UserApp)
where
    UserEvents:
        FromStr + Clone + PartialEq + Default + Debug + EventHandler<UserApplication = UserApp>,
    <UserEvents as FromStr>::Err: Debug + Default,
    UserApp: App + ParserDataAccess<UserEvents>,
{
    if let Ok(event_loop) = EventLoop::<InternalEvents>::with_user_event().build() {
        event_loop.set_control_flow(ControlFlow::Wait);
        let file_watcher_proxy = event_loop.create_proxy();

        if let Ok(watcher) = watch_file("src/layouts", file_watcher_proxy) {
            let mut app =
                Application::new(event_loop.create_proxy(), user_application, Some(watcher));
            event_loop.run_app(&mut app).unwrap();
        } else {
            panic!("Can't find layout files.");
        }
    } else {
        panic!("Event loop creation failed.");
    }
}
