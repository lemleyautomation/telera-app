use std::{collections::HashMap, hash::Hash, fmt::Debug, fs::read_to_string, path::{Path, PathBuf}, str::FromStr, time::Instant};
use image::DynamicImage;
use notify::ReadDirectoryChangesWatcher;
use winit::{application::ApplicationHandler, event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent}, event_loop::{ControlFlow, EventLoop, EventLoopProxy}, window::WindowAttributes};
pub use winit::window::Window;
pub use winit::dpi::LogicalSize;
pub use winit::window::WindowId;

mod graphics_context;
use graphics_context::GraphicsContext;

const MULTI_SAMPLE_COUNT: u32 = 4;

mod depth_texture;
mod multi_sample_texture;

mod viewport;
use viewport::Viewport;
use viewport::BuildViewport;

mod ui_renderer;
use ui_renderer::UIRenderer;
pub use ui_renderer::UIImageDescriptor;

use telera_layout::{LayoutEngine, Parser};
pub use telera_layout::ParserDataAccess;
pub use telera_layout::EnumString;
pub use telera_layout::strum;
pub use telera_layout::ListData;

mod scene_renderer;
use scene_renderer::SceneRenderer;

mod camera_controller;

mod model;

mod texture;

use notify::{RecursiveMode, Result, Watcher};
#[allow(dead_code)]
fn watch_file(file: &str, sender: EventLoopProxy<InternalEvents>) -> ReadDirectoryChangesWatcher{
    let expensive_closure = move |event: Result<notify::Event>| {
        match event {
            Err(e) => {println!("{:?}", e)}
            Ok(event) => {
                if let Some(path) = event.paths.first(){
                    if event.kind == notify::EventKind::Modify(notify::event::ModifyKind::Any) {
                        let _ = sender.send_event(InternalEvents::RebuildLayout(path.to_owned()));
                    }
                }
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

pub struct API<UserPages>{
    staged_windows: Vec<(String, UserPages, WindowAttributes)>,
    staged_images: Vec<(String,DynamicImage)>,
    page_changes: Vec<(String,UserPages)>,
}

impl<UserPages> API<UserPages>{
    pub fn create_window(&mut self, name: &str, page: UserPages, attributes: WindowAttributes){
        self.staged_windows.push((name.to_string(), page, attributes));
    }
    pub fn add_image(&mut self, name: &str, image: DynamicImage) {
        self.staged_images.push((name.to_string(), image));
    }
    pub fn set_viewport_page(&mut self, viewport: &str, page: UserPages){
        self.page_changes.push((viewport.to_string(), page));
    }
}

#[allow(unused_variables)]
pub trait App<UserEvents, UserPages>{
    /// called once before start
    fn initialize(&mut self, api: &mut API<UserPages>);
    /// All application update logic
    /// 
    /// This will be called at the beginning of each render loop
    fn update(&mut self, api: &mut API<UserPages>){}
    /// handling of user events
    fn event_handler(&mut self, event: UserEvents, viewport: &str, api: &mut API<UserPages>){}
}

#[allow(dead_code)]
struct Application<UserApp, UserEvents, UserPages>
where 
    UserEvents: FromStr+Clone+PartialEq+Default+Debug,
    <UserEvents as FromStr>::Err: Debug,
    UserPages: FromStr+Clone+Hash+Eq+Default,
    <UserPages as FromStr>::Err: Debug,
    UserApp: App<UserEvents, UserPages> + ParserDataAccess<UIImageDescriptor, UserEvents>,
{
    ctx: GraphicsContext,
    pub scene_renderer: SceneRenderer,
    pub ui_renderer: Option<UIRenderer>,

    pointer_state: bool,
    dimensions: (f32, f32),
    dpi_scale: f32,
    clicked: bool,
    mouse_poistion: (f32, f32),
    scroll_delta_time: Instant,
    scroll_delta_distance: (f32, f32),

    pub ui_layout: LayoutEngine<UIRenderer, UIImageDescriptor, (), ()>,
    parser: Parser<UserEvents,UserPages>,
    user_events: Vec<UserEvents>,

    viewport_lookup: bimap::BiMap<String, WindowId>,
    viewports: HashMap<WindowId, Viewport<UserPages>>,

    core: API<UserPages>,
    app_events: EventLoopProxy<InternalEvents>,
    user_application: UserApp,
    watcher: Option<ReadDirectoryChangesWatcher>,
}

impl<UserEvents, UserApp, UserPages> Application<UserApp, UserEvents, UserPages>
where 
    UserEvents: FromStr+Clone+PartialEq+Default+Debug,
    <UserEvents as FromStr>::Err: Debug,
    UserPages: FromStr+Clone+Hash+Eq+Default,
    <UserPages as FromStr>::Err: Debug,
    UserApp: App<UserEvents, UserPages> + ParserDataAccess<UIImageDescriptor, UserEvents>,
{
    pub fn new(app_events: EventLoopProxy<InternalEvents>, mut user_application: UserApp, watcher: Option<ReadDirectoryChangesWatcher>) -> Self {
        let ctx = GraphicsContext::new();
        let scene_renderer = SceneRenderer::new(&ctx.device);

        let mut ui_renderer = UIRenderer::new(&ctx.device, &ctx.queue);

        let mut core =  API { 
            staged_windows: Vec::new(), 
            staged_images: Vec::new(),
            page_changes: Vec::new()
        };

        user_application.initialize(&mut core);

        for _ in 0..core.staged_images.len() {
            let (name, image) = core.staged_images.pop().unwrap();
            ui_renderer.stage_atlas(name, image);
        }

        let ui_layout = LayoutEngine::<UIRenderer, UIImageDescriptor, (), ()>::new((1.0, 1.0));
        ui_layout.set_debug_mode(false);

        let mut parser = Parser::default();

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
        

        let app = Application {
            ctx,
            scene_renderer,
            ui_renderer: Some(ui_renderer),
            ui_layout,
            parser,
            pointer_state: false,
            dimensions: (1.0, 1.0),
            dpi_scale: 1.0,
            clicked: false,
            mouse_poistion: (0.0,0.0),
            scroll_delta_time: Instant::now(),
            scroll_delta_distance: (0.0, 0.0),
            viewport_lookup: bimap::BiMap::new(),
            viewports: HashMap::new(),
            core,
            app_events,
            user_events: Vec::new(),
            user_application,
            watcher
        };

        app
    }

    fn open_staged_windows(&mut self, event_loop: &winit::event_loop::ActiveEventLoop){
        for _ in 0..self.core.staged_windows.len() {

            let (name, page, attr) = self.core.staged_windows.pop().unwrap();
            
            if self.viewport_lookup.get_by_left(&name).is_some() { continue; }

            let viewport = attr.build_viewport(event_loop, page, &self.ctx, MULTI_SAMPLE_COUNT);

            viewport.window.set_title(&name);
            let window_id = viewport.window.id();

            let ui_renderer = self.ui_renderer.as_mut().unwrap();
            match ui_renderer.render_pipeline {
                Some(_) => {}
                None => ui_renderer.build_shaders(&self.ctx.device, &self.ctx.queue, &viewport.config, MULTI_SAMPLE_COUNT)
            }

            match self.scene_renderer.render_pipeline {
                Some(_) => {}
                None => self.scene_renderer.build_shaders(&self.ctx.device, &viewport.config, MULTI_SAMPLE_COUNT)
            }

            self.viewport_lookup.insert(name, window_id);
            self.viewports.insert(window_id, viewport);
        }
        self.core.staged_windows.clear();
    }
}

impl<UserEvents, UserApp, UserPages> ApplicationHandler<InternalEvents> for Application<UserApp,UserEvents, UserPages>
where 
    UserEvents: FromStr+Clone+PartialEq+Default+Debug,
    <UserEvents as FromStr>::Err: Debug,
    UserPages: FromStr+Clone+Hash+Eq+Default,
    <UserPages as FromStr>::Err: Debug,
    UserApp: App<UserEvents, UserPages> + ParserDataAccess<UIImageDescriptor, UserEvents>,
{
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.open_staged_windows(event_loop);
    }

    fn window_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, window_id: WindowId, event: winit::event::WindowEvent) {
        if self.core.staged_windows.len() > 0 {
            self.open_staged_windows(event_loop);
        }

        self.user_application.update(&mut self.core);

        for _ in 0..self.core.page_changes.len() {
            let (name, page)  = self.core.page_changes.pop().unwrap();
            if let Some(window_id) = self.viewport_lookup.get_by_left(&name) {
                if let Some(window) = self.viewports.get_mut(window_id){
                    window.page = page;
                    window.window.request_redraw();
                }
            }
        }

        let num_viewports = self.viewports.len();

        let viewport_name = self.viewport_lookup.get_by_right(&window_id).unwrap();

        let viewport = match self.viewports.get_mut(&window_id) {
            Some(window) => window,
            None => return
        };

        match event {
            WindowEvent::CloseRequested => {
                if num_viewports < 2 {
                    event_loop.exit();
                }
                self.viewport_lookup.remove_by_left(&viewport.window.title());
                self.viewports.remove(&window_id);
                return;
            },
            WindowEvent::Resized(size) => {
                viewport.resize(&self.ctx.device, size, MULTI_SAMPLE_COUNT);
            }
            WindowEvent::RedrawRequested => {

                let window_size: (f32, f32) = viewport.window.inner_size().into();
                let dpi_scale  = viewport.window.scale_factor() as f32;
                
                let mut ui_renderer = self.ui_renderer.take().unwrap();
                ui_renderer.dpi_scale = dpi_scale;
                ui_renderer.resize((window_size.0 as i32, window_size.1 as i32), &self.ctx.queue);

                self.ui_layout.set_layout_dimensions(window_size.0/dpi_scale, window_size.1/dpi_scale);
                self.ui_layout.pointer_state(self.mouse_poistion.0/dpi_scale, self.mouse_poistion.1/dpi_scale, self.pointer_state);
                self.ui_layout.update_scroll_containers(false, self.scroll_delta_distance.0, self.scroll_delta_distance.1, self.scroll_delta_time.elapsed().as_secs_f32());
                self.scroll_delta_distance = (0.0,0.0);
                self.scroll_delta_time = Instant::now();

                self.ui_layout.begin_layout(ui_renderer);

                let events = self.parser.set_page(&viewport.page, self.clicked, &mut self.ui_layout, &self.user_application);
                for event in events.iter() {
                    self.user_application.event_handler(event.clone(), &viewport_name, &mut self.core);
                }
                self.clicked = false;
                // self.ui_layout.open_element();
                // let mut c = ElementConfiguration::default();
                // c.x_grow();
                // c.y_grow();
                // c.color(Color { r: 43.0, g: 41.0, b: 51.0, a: 255.0 });
                // self.ui_layout.configure_element(&c);
                // self.ui_layout.close_element();
                
                let (render_commands, mut ui_renderer) = self.ui_layout.end_layout();

                // for command in render_commands.iter() {
                //     println!("{:?}",command);
                // }

                self.ctx.render(
                    viewport,
                    MULTI_SAMPLE_COUNT,
                    |render_pass, device, queue, config| {

                        self.scene_renderer.render(render_pass, &queue);
                        ui_renderer.render_layout::<UIImageDescriptor, (), ()>(render_commands, render_pass, &device, &queue, &config);
                    }
                ).unwrap();

                self.ui_renderer = Some(ui_renderer);
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
    }

    fn user_event(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, event: InternalEvents) {
        if let InternalEvents::RebuildLayout(path) = event {
            let xml_string = read_to_string(path).unwrap();
            self.parser.update_page(&xml_string);
            for (_window_id,viewport) in self.viewports.iter_mut() {
                viewport.window.request_redraw();
            }
        }
        // let file = "examples/layout.xml";
        // let file = read_to_string(file).unwrap();
        // println!("updating layout");
        // self.parser.update_page(&file);
        // for (_window_id,viewport) in self.viewports.iter_mut() {
        //     viewport.window.request_redraw();
        // }
    }
}


pub fn run<UserEvents, UserApp, UserPages>(user_application: UserApp)
where 
    UserEvents: FromStr+Clone+PartialEq+Default+Debug,
    <UserEvents as FromStr>::Err: Debug,
    UserPages: FromStr+Clone+Hash+Eq+Default,
    <UserPages as FromStr>::Err: Debug,
    UserApp: App<UserEvents, UserPages> + ParserDataAccess<UIImageDescriptor, UserEvents>,
{

    let event_loop = match EventLoop::<InternalEvents>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(_) => return
    };
    event_loop.set_control_flow(ControlFlow::Wait);

    #[cfg(debug_assertions)]
    {
        let file_watcher = event_loop.create_proxy();
        let watcher = watch_file("src/layouts", file_watcher);
        let mut app: Application<UserApp, UserEvents, UserPages> = Application::new(event_loop.create_proxy(), user_application, Some(watcher));
        event_loop.run_app(&mut app).unwrap();
    }
    
    #[cfg(not(debug_assertions))]
    {
        let mut app: Application<UserApp, UserEvents, UserPages> = Application::new(event_loop.create_proxy(), user_application, None);
        event_loop.run_app(&mut app).unwrap();
    }
}