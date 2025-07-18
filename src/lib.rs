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
    window::WindowAttributes
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

use telera_layout::LayoutEngine;

mod xml_parse;
pub use xml_parse::*;

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
    ui_renderer: Option<UIRenderer>,
    model_ids: HashMap<String, usize>,
    models: Vec<Model>,
    
    viewport_lookup: bimap::BiMap<String, WindowId>,
    viewports: HashMap<WindowId, Viewport>,

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

pub use event_handler_derive;
pub trait EventHandler {
    type UserApplication;
    fn dispatch(&self, app: &mut Self::UserApplication, api: &mut API) {}
}

#[allow(dead_code)]
struct Application<UserApp, UserEvents>
where 
    UserEvents: FromStr+Clone+PartialEq+Debug+EventHandler,
    <UserEvents as FromStr>::Err: Debug,
    UserApp: App + ParserDataAccess<UIImageDescriptor, UserEvents>,
{
    pub scene_renderer: SceneRenderer,

    pointer_state: bool,
    dimensions: (f32, f32),
    dpi_scale: f32,
    clicked: bool,
    mouse_poistion: (f32, f32),
    scroll_delta_time: Instant,
    scroll_delta_distance: (f32, f32),

    pub ui_layout: LayoutEngine<UIRenderer, UIImageDescriptor, (), ()>,
    parser: Parser<UserEvents>,
    user_events: Vec<UserEvents>,

    core: API,
    app_events: EventLoopProxy<InternalEvents>,
    user_application: UserApp,
    watcher: Option<ReadDirectoryChangesWatcher>,
}

impl<UserEvents, UserApp> Application<UserApp, UserEvents>
where 
    UserEvents: FromStr+Clone+PartialEq+Debug+EventHandler,
    <UserEvents as FromStr>::Err: Debug,
    UserApp: App + ParserDataAccess<UIImageDescriptor, UserEvents>,
{
    pub fn new(app_events: EventLoopProxy<InternalEvents>, user_application: UserApp, watcher: Option<ReadDirectoryChangesWatcher>) -> Self {
        let mut core =  API { 
            staged_windows: Vec::new(), 
            ctx: GraphicsContext::new(),
            ui_renderer: None,
            model_ids: HashMap::new(),
            models: Vec::<Model>::new(),
            viewport_lookup: bimap::BiMap::new(),
            viewports: HashMap::new(),
        };
        core.ui_renderer = Some(UIRenderer::new(&core.ctx.device, &core.ctx.queue));
        
        let mut parser = Parser::new();

        #[cfg(debug_assertions)]
        {
            for dir in std::fs::read_dir("src/layouts").unwrap() {
                #[allow(for_loops_over_fallibles)]
                for dir in dir {
                    let entry = dir.path();
                    if entry.is_file() {
                        let file = read_to_string(entry).unwrap();
                        parser.add_page(&file).unwrap();
                    }
                }
            }
        }

        #[cfg(not(debug_assertions))]
        {
            use include_dir::{include_dir, Dir};
            const LAYOUTS: Dir = include_dir!("src/layouts");
            for layout in LAYOUTS.files(){
                let file = layout.contents_utf8().unwrap();
                parser.add_page(file).unwrap();
            }
        }
        

        Application {
            scene_renderer: SceneRenderer::new(&core.ctx.device),
            ui_layout: LayoutEngine::<UIRenderer, UIImageDescriptor, (), ()>::new((1.0, 1.0)),
            parser,
            pointer_state: false,
            dimensions: (1.0, 1.0),
            dpi_scale: 1.0,
            clicked: false,
            mouse_poistion: (0.0,0.0),
            scroll_delta_time: Instant::now(),
            scroll_delta_distance: (0.0, 0.0),
            core,
            app_events,
            user_events: Vec::new(),
            user_application,
            watcher
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

            match self.scene_renderer.render_pipeline {
                Some(_) => {}
                None => self.scene_renderer.build_shaders(&self.core.ctx.device, &viewport.config, MULTI_SAMPLE_COUNT)
            }

            self.core.viewport_lookup.insert(name, window_id);
            self.core.viewports.insert(window_id, viewport);
        }
        self.core.staged_windows.clear();
    }
}

impl<UserEvents, UserApp> ApplicationHandler<InternalEvents> for Application<UserApp,UserEvents>
where 
    UserEvents: FromStr+Clone+PartialEq+Debug+EventHandler,
    UserEvents: EventHandler<UserApplication = UserApp>, 
    <UserEvents as FromStr>::Err: Debug,
    UserApp: App + ParserDataAccess<UIImageDescriptor, UserEvents>,
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

        let mut events = Vec::<UserEvents>::new();

        let viewport = match self.core.viewports.get_mut(&window_id) {
            Some(window) => window,
            None => return
        };

        self.scene_renderer.camera_controller.process_events(&event);
        //println!("{:?}", self.scene_renderer.camera_controller.process_events(&event));

        match event {
            WindowEvent::CloseRequested => {
                if num_viewports < 2 {
                    event_loop.exit();
                }
                self.core.viewport_lookup.remove_by_left(&viewport.window.title());
                self.core.viewports.remove(&window_id);
                return;
            },
            WindowEvent::Resized(size) => {
                viewport.resize(&self.core.ctx.device, size, MULTI_SAMPLE_COUNT);
            }
            WindowEvent::RedrawRequested => {

                let window_size: (f32, f32) = viewport.window.inner_size().into();
                let dpi_scale  = viewport.window.scale_factor() as f32;
                
                let mut ui_renderer = self.core.ui_renderer.take().unwrap();
                ui_renderer.dpi_scale = dpi_scale;
                ui_renderer.resize((window_size.0 as i32, window_size.1 as i32), &self.core.ctx.queue);

                self.ui_layout.set_layout_dimensions(window_size.0/dpi_scale, window_size.1/dpi_scale);
                self.ui_layout.pointer_state(self.mouse_poistion.0/dpi_scale, self.mouse_poistion.1/dpi_scale, self.pointer_state);
                self.ui_layout.update_scroll_containers(false, self.scroll_delta_distance.0, self.scroll_delta_distance.1, self.scroll_delta_time.elapsed().as_secs_f32());
                self.scroll_delta_distance = (0.0,0.0);
                self.scroll_delta_time = Instant::now();

                self.ui_layout.begin_layout(ui_renderer);
                events = self.parser.set_page(&viewport.page, self.clicked, &mut self.ui_layout, &self.user_application, None);
                self.clicked = false;
                let (render_commands, mut ui_renderer) = self.ui_layout.end_layout();

                self.core.ctx.render(
                    viewport,
                    MULTI_SAMPLE_COUNT,
                    |render_pass, device, queue, config| {
                        
                        self.scene_renderer.render(&mut self.core.models, render_pass, &queue);
                        ui_renderer.render_layout::<UIImageDescriptor, (), ()>(render_commands, render_pass, &device, &queue, &config);
                    }
                ).unwrap();

                self.core.ui_renderer = Some(ui_renderer);
                viewport.window.request_redraw();
            }
            WindowEvent::MouseInput { device_id:_, state, button } => {
                match button {
                    MouseButton::Left => {
                        match state {
                            ElementState::Pressed => {
                                self.pointer_state = true;
                                self.clicked = true;
                            }
                            ElementState::Released => self.pointer_state = false,
                        }
                    }
                    _ => {}
                }
                viewport.window.request_redraw();
            }
            WindowEvent::MouseWheel { device_id:_, delta, phase:_ } => {
                self.scroll_delta_distance = match delta {
                    MouseScrollDelta::LineDelta(x,y ) => (x,y),
                    MouseScrollDelta::PixelDelta(position) => position.into()
                };
                viewport.window.request_redraw();
            }
            WindowEvent::CursorMoved { device_id:_, position } => {
                self.mouse_poistion = position.into();
                viewport.window.request_redraw();
            }
            _ => {}
        }

        for event in events.iter() {
            event.dispatch(&mut self.user_application, &mut self.core);
        }
    }

    fn user_event(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, event: InternalEvents) {
        if let InternalEvents::RebuildLayout(path) = event {
            let xml_string = read_to_string(path).unwrap();
            self.parser.update_page(&xml_string);
            for (_window_id,viewport) in self.core.viewports.iter_mut() {
                viewport.window.request_redraw();
            }
        }
    }
}

pub fn run<UserEvents, UserApp>(user_application: UserApp)
where 
    UserEvents: FromStr+Clone+PartialEq+Debug+EventHandler,
    UserEvents: EventHandler<UserApplication = UserApp>, 
    <UserEvents as FromStr>::Err: Debug,
    UserApp: App + ParserDataAccess<UIImageDescriptor, UserEvents>,
{
    let event_loop = match EventLoop::<InternalEvents>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(_) => return
    };
    event_loop.set_control_flow(ControlFlow::Poll);

    #[cfg(debug_assertions)]
    {
        let file_watcher = event_loop.create_proxy();
        let watcher = watch_file("src/layouts", file_watcher);
        let mut app: Application<UserApp, UserEvents> = Application::new(event_loop.create_proxy(), user_application, Some(watcher));
        event_loop.run_app(&mut app).unwrap();
    }
    
    #[cfg(not(debug_assertions))]
    {
        let mut app: Application<UserApp, UserEvents> = Application::new(event_loop.create_proxy(), user_application, None);
        event_loop.run_app(&mut app).unwrap();
    }
}

/*
CLAY({ .id = CLAY_ID("FileMenu"),
        .floating = {
            .attachTo = CLAY_ATTACH_TO_PARENT,
            .attachPoints = {
                .parent = CLAY_ATTACH_POINT_LEFT_BOTTOM
            },
        },
        .layout = {
            .padding = {0, 0, 8, 8 }
        }
    })) {
        CLAY({
            .layout = {
                .layoutDirection = CLAY_TOP_TO_BOTTOM,
                .sizing = {
                        .width = CLAY_SIZING_FIXED(200)
                },
            },
            .backgroundColor = {40, 40, 40, 255 },
            .cornerRadius = CLAY_CORNER_RADIUS(8)
        }) {
            // Render dropdown items here
            RenderDropdownMenuItem(CLAY_STRING("New"));
            RenderDropdownMenuItem(CLAY_STRING("Open"));
            RenderDropdownMenuItem(CLAY_STRING("Close"));
        }
    }
}
              .sizing = {
                        .width = CLAY_SIZING_FIXED(200)
                },
            },
            .backgroundColor = {40, 40, 40, 255 },
            .cornerRadius = CLAY_CORNER_RADIUS(8)
        }) {
            // Render dropdown items here
            RenderDropdownMenuItem(CLAY_STRING("New"));
            RenderDropdownMenuItem(CLAY_STRING("Open"));
            RenderDropdownMenuItem(CLAY_STRING("Close"));
        }
    }
}
*/