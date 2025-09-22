use std::{
    collections::HashMap, fmt::Debug, fs::read_to_string, path::{Path, PathBuf}, str::FromStr, time::Instant
};

pub use rkyv;
//pub use rkyv::{deserialize, rancor::Error, Archive, Deserialize, Serialize};

use image::DynamicImage;

use notify::{
    ReadDirectoryChangesWatcher,
    RecursiveMode,
    Result,
    Watcher
};

pub use rfd::FileDialog as FileDialog;
pub use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopProxy},
    window::{WindowAttributes}
};
pub use winit::{
    window::{
        Window,
        WindowId
    },
    dpi::LogicalSize
};

mod graphics_context;
use graphics_context::GraphicsContext;

const MULTI_SAMPLE_COUNT: u32 = 1;

mod depth_texture;
mod multi_sample_texture;

mod viewport;
use viewport::Viewport;
use viewport::BuildViewport;

mod ui_renderer;
use ui_renderer::UIRenderer;
pub use ui_renderer::UIImageDescriptor;
mod ui_shapes;
use ui_shapes::Shapes;

use telera_layout::LayoutEngine;

pub use strum;
mod layout_types;
pub use layout_types::*;
mod page_set;
pub use page_set::*;
mod markdown;
pub use markdown::*;
mod ui_toolkit;
pub use ui_toolkit::treeview::TreeViewItem;
pub use ui_toolkit::treeview::TreeViewEvents;

mod scene_renderer;
use scene_renderer::SceneRenderer;

use crate::model::{load_model_gltf, Model};
pub use crate::model::Transform;
pub use crate::model::TransformMatrix;
pub use crate::model::BaseMesh;

mod camera_controller;

mod model;
pub use model::Quaternion;
pub use model::Euler;

mod texture;

#[allow(dead_code)]
fn watch_file(file: &str, sender: EventLoopProxy<InternalEvents>) -> ReadDirectoryChangesWatcher{
    let expensive_closure = move |event: Result<notify::Event>| {
        if  let Ok(event) = event &&
            let Some(path) = event.paths.first()  {
            if event.kind == notify::EventKind::Modify(notify::event::ModifyKind::Any) {
                let _ = sender.send_event(InternalEvents::RebuildLayout(path.to_owned()));
            }
        }
    };

    let mut watcher = notify::recommended_watcher(expensive_closure).unwrap();

    watcher.watch(Path::new(file), RecursiveMode::NonRecursive).unwrap();

    watcher
}

#[allow(dead_code)]
enum InternalEvents{
    Hi,
    RebuildLayout(PathBuf),
}

#[allow(unused_variables)]
pub trait App{
    /// called once before start
    fn initialize(&mut self, api: &mut API);
    /// All application update logic
    /// 
    /// This will be called at the beginning of each render loop
    fn update(&mut self, api: &mut API){}
}

pub struct API{
    staged_windows: Vec<(String, String, WindowAttributes)>,

    ctx: GraphicsContext,
    pub scene_renderer: SceneRenderer,
    ui_renderer: Option<UIRenderer>,
    pub ui_layout: LayoutEngine<UIRenderer, UIImageDescriptor, Shapes, ()>,
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
    left_mouse_dbl_clicked_timer: Option<Instant>,

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
    pub mouse_delta: (f32,f32),
    scroll_delta_time: Instant,
    scroll_delta_distance: (f32, f32),
}

impl API{
    pub fn create_viewport(&mut self, name: &str, page: &str, attributes: WindowAttributes){
        self.staged_windows.push((name.to_string(), page.to_string(), attributes));
    }
    pub fn add_image(&mut self, name: &str, image: DynamicImage) {
        if let Some(ui_renderer) = &mut self.ui_renderer {
            ui_renderer.stage_atlas(name.to_string(), image);
        }
    }
    pub fn set_viewport_title(&mut self, viewport: &str, title: &str) {
        if  let Some(window_id) = self.viewport_lookup.get_by_left(viewport) && 
            let Some (viewport) = self.viewports.get_mut(window_id) {
            viewport.window.set_title(title);
        }
    }
    pub fn set_current_viewport_page(&mut self, page: &str) {
        // TODO !
        println!("{:?}", page);
    }
    pub fn set_viewport_page(&mut self, viewport: &str, page: &str){
        if  let Some(window_id) = self.viewport_lookup.get_by_left(viewport) &&
            let Some(window) = self.viewports.get_mut(window_id) {
            window.page = page.to_string();
            window.window.request_redraw();
        }
    }
    pub fn load_gltf_model(&mut self, model_name: &str, filename: PathBuf, transfrom: Option<Transform>) -> BaseMesh{
        self.model_ids.insert(model_name.to_string(), self.models.len());
        let model = load_model_gltf(filename, &self.ctx.device, &self.ctx.queue, transfrom).unwrap();
        let base = model.mesh.base.clone();
        self.models.push(model);

        base
    }
    pub fn transform_model(&mut self, model_name: &str) -> Result<&mut Transform> {
        if let Some(model_index) = self.model_ids.get(model_name) {
            if let Some(model_reference) = self.models.get_mut(*model_index) {
                model_reference.transform_dirty = true;
                return Ok(&mut model_reference.transform)
            }
        }

        Err(notify::Error::path_not_found())
    }
    pub fn add_instance(&mut self, model_name: &str, instance_name: &str, transfrom: Option<Transform>){
        if let Some(model_index) = self.model_ids.get(model_name) {
            if let Some(model) = self.models.get_mut(*model_index) {
                model.mesh.add_instance(instance_name.to_string(), &self.ctx.device, transfrom);
                //println!("hi {:?}", model.mesh.instances);
            }
        }
    }
    pub fn transform_instance(&mut self, model_name: &str, instance_name: &str) -> Result<&mut Transform> {
        if  let Some(model_index) = self.model_ids.get(model_name) &&
            let Some(model_reference) = self.models.get_mut(*model_index) &&
            let Some(instance) = model_reference.mesh.instance_lookup.get(instance_name) &&
            let Some(instance_reference) = model_reference.mesh.instances.get_mut(*instance)
            {
            model_reference.mesh.instances_dirty = true;
            return Ok(instance_reference)
        }
        Err(notify::Error::path_not_found())
    }
}

#[derive(Clone)]
pub struct EventContext{
    pub text: Option<String>,
    pub code: Option<u32>,
    pub code2: Option<u32>
}

impl EventContext {
    pub fn new() -> Self {
        EventContext { text: None, code: None, code2: None }
    }
    pub fn from_code(code: u32) -> Self {
        EventContext { text: None, code: Some(code), code2: None }
    }
    pub fn from_code2(code2: u32) -> Self {
        EventContext { text: None, code: None, code2: Some(code2) }
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

pub use event_handler_derive;
pub trait EventHandler {
    type UserApplication;
    #[allow(unused_variables)]
    fn dispatch(&self, app: &mut Self::UserApplication, context: Option<EventContext>, api: &mut API) {}
}

struct Application<UserApp, UserEvents>
where 
    UserEvents: FromStr+Clone+PartialEq+Default+Debug+EventHandler<UserApplication = UserApp>,
    <UserEvents as FromStr>::Err: Debug,
    UserApp: App + ParserDataAccess<UserEvents>,
{
    layout_binder: Binder<UserEvents,UserApp>,
    core: API,
    user_application: UserApp,

    #[allow(dead_code)]
    app_events: EventLoopProxy<InternalEvents>,
    #[allow(dead_code)]
    watcher: Option<ReadDirectoryChangesWatcher>,
}

impl<UserEvents, UserApp> Application<UserApp, UserEvents>
where 
    UserEvents: FromStr+Clone+PartialEq+Debug+Default+EventHandler<UserApplication = UserApp>,
    <UserEvents as FromStr>::Err: Debug,
    UserApp: App + ParserDataAccess<UserEvents>,
{
    pub fn new(app_events: EventLoopProxy<InternalEvents>, user_application: UserApp, watcher: Option<ReadDirectoryChangesWatcher>) -> Self {

        let ctx = GraphicsContext::new();
        let scene_renderer = SceneRenderer::new(&ctx.device);
        let ui_renderer = Some(UIRenderer::new(&ctx.device, &ctx.queue));

        let core =  API { 
            staged_windows: Vec::new(), 
            ctx,
            scene_renderer,
            ui_renderer,
            ui_layout: LayoutEngine::<UIRenderer, UIImageDescriptor, Shapes, ()>::new((1.0, 1.0)),
            model_ids: HashMap::new(),
            models: Vec::<Model>::new(),
            viewport_lookup: bimap::BiMap::new(),
            viewports: HashMap::new(),

            event_string: "".to_string(),

            left_mouse_pressed: false,
            left_mouse_down: false,
            left_mouse_released: false,
            left_mouse_clicked: false,
            left_mouse_double_clicked: false,
            left_mouse_clicked_timer: None,
            left_mouse_dbl_clicked_timer: None,

            right_mouse_pressed: false,
            right_mouse_down: false,
            right_mouse_released: false,
            right_mouse_clicked: false,
            right_mouse_clicked_timer: None,

            x_at_click: 0.0,
            y_at_click: 0.0,
            focus: 0,
            
            dpi_scale: 0.0,
            mouse_poistion: (0.0,0.0),
            mouse_delta: (0.0,0.0),
            scroll_delta_time: Instant::now(),
            scroll_delta_distance: (0.0, 0.0),
        };
        
        let mut layout_binder = Binder::new();

        let entries = std::fs::read_dir("src/layouts").unwrap_or_else(|e| {
            eprintln!("Error reading directory: {}", e);
            std::process::exit(1);
        });

        for dir in entries {
            #[allow(for_loops_over_fallibles)]
            for dir in dir {
                let entry = dir.path();
                if entry.is_file() {
                    if let Ok(file) = read_to_string(entry) {
                        if let Ok((page_name, page_layout, reusables)) = process_layout::<UserEvents>(file) {
                            layout_binder.add_page(&page_name, page_layout);
                            for (name, reusable) in reusables {
                                layout_binder.add_reusable(&name, reusable);
                            }
                        }
                    }
                }
            }
        }

        Application {
            layout_binder,
            core,
            app_events,
            user_application,
            watcher,
        }
    }

    fn open_staged_windows(&mut self, event_loop: &winit::event_loop::ActiveEventLoop){
        for _ in 0..self.core.staged_windows.len() {
                    
            let (name, page, attr) = self.core.staged_windows.pop().unwrap();
            
            if self.core.viewport_lookup.get_by_left(&name).is_some() { continue; }
            
            let viewport = attr.build_viewport(event_loop, page, &self.core.ctx, MULTI_SAMPLE_COUNT);
            
            viewport.window.set_title(&name);
            let window_id = viewport.window.id();
            
            let ui_renderer = self.core.ui_renderer.as_mut().unwrap();
            match ui_renderer.render_pipeline {
                Some(_) => {}
                None => ui_renderer.build_shaders(&self.core.ctx.device, &self.core.ctx.queue, &viewport.config, MULTI_SAMPLE_COUNT)
            }
            
            match self.core.scene_renderer.render_pipeline {
                Some(_) => {}
                None => self.core.scene_renderer.build_shaders(&self.core.ctx.device, &viewport.config, MULTI_SAMPLE_COUNT)
            }
            
            self.core.viewport_lookup.insert(name.clone(), window_id);
            self.core.viewports.insert(window_id, viewport);
            
        }
        self.core.staged_windows.clear();
    }
}

impl<UserEvents, UserApp> ApplicationHandler<InternalEvents> for Application<UserApp, UserEvents>
where 
    UserEvents: FromStr+Clone+PartialEq+Debug+Default+EventHandler<UserApplication = UserApp>,
    UserEvents: EventHandler<UserApplication = UserApp>, 
    <UserEvents as FromStr>::Err: Debug,
    UserApp: App + ParserDataAccess<UserEvents>,
{
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.user_application.initialize(&mut self.core);
        self.open_staged_windows(event_loop);
    }

    fn window_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, window_id: WindowId, event: winit::event::WindowEvent) {
        
        if self.core.staged_windows.len() > 0 {
            self.open_staged_windows(event_loop);
        }

        self.user_application.update(&mut self.core);

        let num_viewports = self.core.viewports.len();

        self.core.scene_renderer.camera_controller.process_events(&event);

        match event {
            WindowEvent::CloseRequested => {
                if num_viewports < 2 {
                    event_loop.exit();
                }
                self.core.viewport_lookup.remove_by_left(&self.core.viewports.get_mut(&window_id).as_mut().unwrap().window.title());
                self.core.viewports.remove(&window_id);
                return;
            },
            WindowEvent::Resized(size) => {
                self.core.viewports.get_mut(&window_id).as_mut().unwrap().resize(&self.core.ctx.device, size, MULTI_SAMPLE_COUNT);
            }
            WindowEvent::RedrawRequested => {


                let window_size: (f32, f32) = self.core.viewports.get_mut(&window_id).as_mut().unwrap().window.inner_size().into();
                let dpi_scale  = self.core.viewports.get_mut(&window_id).as_mut().unwrap().window.scale_factor() as f32;

                self.core.dpi_scale = dpi_scale;
                
                let mut ui_renderer = self.core.ui_renderer.take().unwrap();
                ui_renderer.dpi_scale = dpi_scale;
                ui_renderer.resize((window_size.0 as i32, window_size.1 as i32), &self.core.ctx.queue);

                self.core.ui_layout.set_layout_dimensions(window_size.0/dpi_scale, window_size.1/dpi_scale);
                self.core.ui_layout.pointer_state(
                    self.core.mouse_poistion.0/dpi_scale, 
                    self.core.mouse_poistion.1/dpi_scale, 
                    self.core.left_mouse_down
                );
                self.core.ui_layout.update_scroll_containers(
                    false, 
                    self.core.scroll_delta_distance.0, 
                    self.core.scroll_delta_distance.1, 
                    self.core.scroll_delta_time.elapsed().as_secs_f32()
                );
                self.core.scroll_delta_distance = (0.0,0.0);
                self.core.scroll_delta_time = Instant::now();

                self.core.ui_layout.begin_layout(ui_renderer);
        
                self.layout_binder.set_page(
                    window_id,
                    &mut self.core, 
                    &mut self.user_application
                );
                
                let (render_commands, mut ui_renderer) = self.core.ui_layout.end_layout();

                self.core.ctx.render(
                    self.core.viewports.get_mut(&window_id).as_mut().unwrap(),
                    MULTI_SAMPLE_COUNT,
                    |render_pass, device, queue, config| {
                        
                        self.core.scene_renderer.render(&mut self.core.models, render_pass, &queue);
                        
                        ui_renderer.render_layout(render_commands, render_pass, &device, &queue, &config);
                      
                    }
                ).unwrap();


                self.core.ui_renderer = Some(ui_renderer);

                
                self.core.left_mouse_pressed = false;
                self.core.left_mouse_released = false;
                self.core.left_mouse_clicked = false;
                self.core.left_mouse_double_clicked = false;
                if let Some(timer) = self.core.left_mouse_clicked_timer
                && timer.elapsed().as_millis() > 400 {
                    self.core.left_mouse_clicked_timer = None;
                }
                // if let Some(timer) = self.core.left_mouse_dbl_clicked_timer
                // && timer.elapsed().as_millis() > 300 {
                //     self.core.left_mouse_dbl_clicked_timer = None;
                // }
                self.core.right_mouse_pressed = false;
                self.core.right_mouse_released = false;
                self.core.right_mouse_clicked = false;
                if let Some(timer) = self.core.right_mouse_clicked_timer
                && timer.elapsed().as_millis() > 300 {
                    self.core.right_mouse_clicked_timer = None;
                }
            }
            WindowEvent::MouseInput { device_id:_, state, button } => {
                let dpi = self.core.ui_renderer.as_ref().unwrap().dpi_scale;
                match button {
                    MouseButton::Left => {
                        match state {
                            ElementState::Pressed => {
                                self.core.left_mouse_pressed = true;
                                self.core.left_mouse_down = true;
                                if self.core.left_mouse_clicked_timer.is_none() {
                                    self.core.left_mouse_clicked_timer = Some(Instant::now());
                                }
                                // else {
                                //     self.core.left_mouse_clicked_timer = None;
                                //     self.core.left_mouse_dbl_clicked_timer = Some(Instant::now());
                                // }
                                self.core.x_at_click = self.core.mouse_poistion.0/dpi;
                                self.core.y_at_click = self.core.mouse_poistion.1/dpi;
                            }
                            ElementState::Released => {
                                if let Some(timer) = self.core.left_mouse_clicked_timer
                                && timer.elapsed().as_millis() < 400 {
                                    self.core.left_mouse_clicked = true;
                                    self.core.left_mouse_clicked_timer = None;
                                }
                                // if let Some(timer) = self.core.left_mouse_dbl_clicked_timer
                                // && timer.elapsed().as_millis() < 300 {
                                //     self.core.left_mouse_double_clicked = true;
                                //     self.core.left_mouse_dbl_clicked_timer = None;
                                // }
                                self.core.left_mouse_down = false;
                                self.core.left_mouse_released = true;
                            }
                        }
                    }
                    MouseButton::Right => {
                        match state {
                            ElementState::Pressed => {
                                self.core.right_mouse_pressed = true;
                                self.core.right_mouse_down = true;
                                if self.core.right_mouse_clicked_timer.is_none() {
                                    self.core.right_mouse_clicked_timer = Some(Instant::now());
                                }
                                self.core.x_at_click = self.core.mouse_poistion.0/dpi;
                                self.core.y_at_click = self.core.mouse_poistion.1/dpi;
                            }
                            ElementState::Released => {
                                if let Some(timer) = self.core.right_mouse_clicked_timer
                                && timer.elapsed().as_millis() < 300 {
                                    self.core.right_mouse_clicked = true;
                                    self.core.right_mouse_clicked_timer = None;
                                }
                                self.core.right_mouse_down = false;
                                self.core.right_mouse_released = true;
                            }
                        }
                    }
                    
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { device_id:_, delta, phase:_ } => {
                self.core.scroll_delta_distance = match delta {
                    MouseScrollDelta::LineDelta(x,y ) => (x,y),
                    MouseScrollDelta::PixelDelta(position) => position.into()
                };
                //viewport.window.request_redraw();
            }
            WindowEvent::CursorMoved { device_id:_, position } => {
                self.core.mouse_delta.0 = position.x as f32 - self.core.mouse_poistion.0;
                self.core.mouse_delta.1 = position.y as f32 - self.core.mouse_poistion.1;
                self.core.mouse_poistion = position.into();
            }
            _ => {}
        }
        
        self.core.viewports.get_mut(&window_id).as_mut().unwrap().window.request_redraw();
        
    }

    fn user_event(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, event: InternalEvents) {
        if let InternalEvents::RebuildLayout(path) = event {
            let file = read_to_string(path).unwrap();
            if let Ok((page_name, page_layout, reusables)) = process_layout::<UserEvents>(file) {
                let _ = self.layout_binder.replace_page(&page_name, page_layout);
                self.layout_binder.reusable.clear();
                for (name, reusable) in reusables {
                    self.layout_binder.add_reusable(&name, reusable);
                }
            }
        }
    }
}

pub fn run<UserEvents, UserApp>(user_application: UserApp)
where 
    UserEvents: FromStr+Clone+PartialEq+Default+Debug+EventHandler<UserApplication = UserApp>,
    <UserEvents as FromStr>::Err: Debug,
    UserApp: App + ParserDataAccess<UserEvents>,
{
    let event_loop = match EventLoop::<InternalEvents>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(_) => return
    };
    event_loop.set_control_flow(ControlFlow::Wait);

    let file_watcher = event_loop.create_proxy();
    let watcher = watch_file("src/layouts", file_watcher);
    let mut app: Application<UserApp, UserEvents> = Application::new(event_loop.create_proxy(), user_application, Some(watcher));
    
    event_loop.run_app(&mut app).unwrap();
}