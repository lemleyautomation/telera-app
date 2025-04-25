use std::{cell::RefCell, collections::HashMap, fmt::Debug, fs::read_to_string, path::Path, rc::Rc, str::FromStr, time::Instant};
use notify::ReadDirectoryChangesWatcher;
use winit::{application::ApplicationHandler, event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent}, event_loop::{ControlFlow, EventLoop, EventLoopProxy}, window::WindowAttributes};
pub use winit::window::Window;
pub use winit::dpi::LogicalSize;
pub use winit::window::WindowId;

mod graphics_context;
use graphics_context::GraphicsContext;

mod depth_texture;

mod viewport;
use viewport::Viewport;
use viewport::BuildViewport;

mod ui_renderer;
use ui_renderer::UIRenderer;
pub use ui_renderer::UIImageDescriptor;

use telera_layout::{Color, ElementConfiguration, LayoutEngine, Parser, ParserDataAccess, TextConfig, Vec2};
use telera_layout::RenderCommand;

mod scene_renderer;
use scene_renderer::SceneRenderer;

mod camera_controller;

mod model;

mod texture;

use notify::{RecursiveMode, Result, Watcher};
fn watch_file(file: &str, sender: EventLoopProxy<InternalEvents>) -> ReadDirectoryChangesWatcher{
    let expensive_closure = move |event: Result<notify::Event>| {
        match event {
            Err(e) => {println!("{:?}", e)}
            Ok(event) => {
                if event.kind == notify::EventKind::Modify(notify::event::ModifyKind::Any) {
                    let _ = sender.send_event(InternalEvents::RebuildLayout);
                }
            }
        }
    };

    let mut watcher = notify::recommended_watcher(expensive_closure).unwrap();

    watcher.watch(Path::new(file), RecursiveMode::NonRecursive).unwrap();

    watcher
}

#[allow(dead_code)]
enum InternalEvents {
    Hi,
    RebuildLayout,
}

pub struct Core{
    staged_windows: Vec<WindowAttributes>,
}

impl Core{
    pub fn create_window(&mut self, attributes: WindowAttributes){
        self.staged_windows.push(attributes);
    }
}

pub trait App<UserEvents, ImageElementData: Debug, CustomElementData: Debug, CustomLayoutSettings>{
    /// called once before start
    fn initialize(&self, core: &mut Core);
    /// All application update logic
    /// 
    /// This will be called at the beginning of each render loop
    //fn update(&self, layout: &mut LayoutEngine<UIRenderer, ImageElementData, CustomElementData, CustomLayoutSettings>) -> Vec<RenderCommand::<ImageElementData, CustomElementData, CustomLayoutSettings>>;
    /// handling of user events
    fn event_handler(&mut self, event: UserEvents, core: &mut Core);
}

#[allow(dead_code)]
struct Application<UserApp, UserEvents, UserPages>
where 
    UserEvents: FromStr+Clone+PartialEq+Default+Debug,
    <UserEvents as FromStr>::Err: Debug,
    UserPages: Default,
    UserApp: App<UserEvents, (),(),()> + ParserDataAccess<(), UserEvents>,
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

    pub ui_layout: LayoutEngine<UIRenderer, (), (), ()>,
    parser: Parser<UserEvents>,
    user_events: Vec<UserEvents>,

    viewport_lookup: HashMap<String, WindowId>,
    viewports: HashMap<WindowId, Viewport<UserPages>>,

    core: Core,
    app_events: EventLoopProxy<InternalEvents>,
    user_application: UserApp,
    watcher: ReadDirectoryChangesWatcher,
}

impl<UserEvents, UserApp, UserPages> Application<UserApp, UserEvents, UserPages>
where 
    UserEvents: FromStr+Clone+PartialEq+Default+Debug,
    <UserEvents as FromStr>::Err: Debug,
    UserPages: Default,
    UserApp: App<UserEvents, (),(),()> + ParserDataAccess<(), UserEvents>,
{
    pub fn new(app_events: EventLoopProxy<InternalEvents>, user_application: UserApp, watcher: ReadDirectoryChangesWatcher) -> Self {
        let ctx = GraphicsContext::new();
        let scene_renderer = SceneRenderer::new(&ctx.device);

        let ui_renderer = UIRenderer::new(&ctx.device, &ctx.queue);

        let mut core =  Core { 
            staged_windows: Vec::new() 
        };

        user_application.initialize(&mut core);

        let mut ui_layout = LayoutEngine::<UIRenderer, (), (), ()>::new((1.0, 1.0));
        ui_layout.set_debug_mode(false);

        let mut parser = Parser::default();
        let file = "examples/layout.xml";
        let file = read_to_string(file).unwrap();
        parser.add_page(&file).unwrap();

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
            viewport_lookup: HashMap::new(),
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

            let attr = self.core.staged_windows.pop().unwrap();
            let window_title = attr.title.clone();
            
            if self.viewport_lookup.get(&window_title).is_some() { continue; }

            let viewport = attr.build_viewport(event_loop, UserPages::default(), &self.ctx);

            let window_id = viewport.window.id();

            let mut ui_renderer = self.ui_renderer.as_mut().unwrap();
            match ui_renderer.render_pipeline {
                Some(_) => {}
                None => ui_renderer.build_shaders(&self.ctx.device, &self.ctx.queue, &viewport.config)
            }

            match self.scene_renderer.render_pipeline {
                Some(_) => {}
                None => self.scene_renderer.build_shaders(&self.ctx.device, &viewport.config)
            }

            self.viewport_lookup.insert(window_title, window_id);
            self.viewports.insert(window_id, viewport);
        }
        self.core.staged_windows.clear();
    }
}

impl<UserEvents, UserApp, UserPages> ApplicationHandler<InternalEvents> for Application<UserApp,UserEvents, UserPages>
where 
    UserEvents: FromStr+Clone+PartialEq+Default+Debug,
    <UserEvents as FromStr>::Err: Debug,
    UserPages: Default,
    UserApp: App<UserEvents, (),(),()> + ParserDataAccess<(), UserEvents>,
{
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.open_staged_windows(event_loop);
    }

    fn window_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, window_id: WindowId, event: winit::event::WindowEvent) {
        if self.core.staged_windows.len() > 0 {
            self.open_staged_windows(event_loop);
        }

        let num_viewports = self.viewports.len();

        let viewport = match self.viewports.get_mut(&window_id) {
            Some(window) => window,
            None => return
        };

        match event {
            WindowEvent::CloseRequested => {
                if num_viewports < 2 {
                    event_loop.exit();
                }
                self.viewport_lookup.remove(&viewport.window.title());
                self.viewports.remove(&window_id);
                return;
            },
            WindowEvent::Resized(size) => {
                viewport.resize(&self.ctx.device, size);
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

                let events = self.parser.set_page("Main", self.clicked, &mut self.ui_layout, &self.user_application);
                for event in events.iter() {
                    self.user_application.event_handler(event.clone(), &mut self.core);
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

                self.ctx.render(
                    viewport,
                    |render_pass, device, queue, config| {

                        self.scene_renderer.render(render_pass, &queue);
                        ui_renderer.render_layout::<(), (), ()>(render_commands, render_pass, &device, &queue, &config);
                    }
                ).unwrap();

                self.ui_renderer = Some(ui_renderer);

                while let Some(event) = self.user_events.pop() {
                    self.user_application.event_handler(event, &mut self.core);
                }
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

    fn user_event(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, _event: InternalEvents) {
        let file = "examples/layout.xml";
        let file = read_to_string(file).unwrap();
        self.parser.update_page(&file);
        for (_window_id,viewport) in self.viewports.iter_mut() {
            viewport.window.request_redraw();
        }
    }
}


pub fn run<UserEvents, UserApp, UserPages>(user_application: UserApp)
where 
    UserEvents: FromStr+Clone+PartialEq+Default+Debug,
    <UserEvents as FromStr>::Err: Debug,
    UserPages: Default,
    UserApp: App<UserEvents, (),(),()> + ParserDataAccess<(), UserEvents>,
{

    let event_loop = match EventLoop::<InternalEvents>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(_) => return
    };
    event_loop.set_control_flow(ControlFlow::Wait);

    let file_watcher = event_loop.create_proxy();

    let watcher = watch_file("examples/layout2.xml", file_watcher);

    let mut app: Application<UserApp, UserEvents, UserPages> = Application::new(event_loop.create_proxy(), user_application, watcher);

    event_loop.run_app(&mut app).unwrap();
}