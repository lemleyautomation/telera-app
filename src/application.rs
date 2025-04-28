use std::{collections::HashMap, hash::Hash, fmt::Debug, fs::read_to_string, path::PathBuf, str::FromStr, time::Instant};
use winit::{application::ApplicationHandler, event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent}, event_loop::EventLoopProxy};
pub use winit::window::WindowId;

use crate::graphics_context::GraphicsContext;

use crate::viewport::Viewport;
use crate::viewport::BuildViewport;

use crate::ui_renderer::UIRenderer;

use telera_layout::{LayoutEngine, Parser};
pub use telera_layout::ParserDataAccess;

use crate::scene_renderer::SceneRenderer;

use crate::Core;
use crate::App;

#[allow(dead_code)]
pub enum InternalEvents{
    Hi,
    RebuildLayout(PathBuf),
}


#[allow(dead_code)]
pub struct Application<UserApp, UserEvents, UserPages, Watcher>
where 
    UserEvents: FromStr+Clone+PartialEq+Default+Debug,
    <UserEvents as FromStr>::Err: Debug,
    UserPages: FromStr+Clone+Hash+Eq+Default,
    <UserPages as FromStr>::Err: Debug,
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
    parser: Parser<UserEvents,UserPages>,
    user_events: Vec<UserEvents>,

    viewport_lookup: HashMap<String, WindowId>,
    viewports: HashMap<WindowId, Viewport<UserPages>>,

    core: Core,
    app_events: EventLoopProxy<InternalEvents>,
    user_application: UserApp,
    watcher: Option<Watcher>,
}

impl<UserEvents, UserApp, UserPages, Watcher> Application<UserApp, UserEvents, UserPages, Watcher>
where 
    UserEvents: FromStr+Clone+PartialEq+Default+Debug,
    <UserEvents as FromStr>::Err: Debug,
    UserPages: FromStr+Clone+Hash+Eq+Default,
    <UserPages as FromStr>::Err: Debug,
    UserApp: App<UserEvents, (),(),()> + ParserDataAccess<(), UserEvents>,
{
    pub fn new(app_events: EventLoopProxy<InternalEvents>, user_application: UserApp, watcher: Option<Watcher>) -> Self {
        let (ctx, _) = GraphicsContext::new::<UserPages>();
        let scene_renderer = SceneRenderer::new(&ctx.device);

        let ui_renderer = UIRenderer::new(&ctx.device, &ctx.queue);

        let mut core =  Core { 
            staged_windows: Vec::new() 
        };

        user_application.initialize(&mut core);

        let ui_layout = LayoutEngine::<UIRenderer, (), (), ()>::new((1.0, 1.0));
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

            let window_id = viewport.window.as_ref().unwrap().id();

            let ui_renderer = self.ui_renderer.as_mut().unwrap();
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

impl<UserEvents, UserApp, UserPages, Watcher> ApplicationHandler<InternalEvents> for Application<UserApp,UserEvents, UserPages, Watcher>
where 
    UserEvents: FromStr+Clone+PartialEq+Default+Debug,
    <UserEvents as FromStr>::Err: Debug,
    UserPages: FromStr+Clone+Hash+Eq+Default,
    <UserPages as FromStr>::Err: Debug,
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
                self.viewport_lookup.remove(&viewport.window.as_ref().unwrap().title());
                self.viewports.remove(&window_id);
                return;
            },
            WindowEvent::Resized(size) => {
                viewport.resize(&self.ctx.device, size);
            }
            WindowEvent::RedrawRequested => {
                let window_size: (f32, f32) = viewport.window.as_ref().unwrap().inner_size().into();
                let dpi_scale  = viewport.window.as_ref().unwrap().scale_factor() as f32;
                
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
                );

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
                viewport.window.as_ref().unwrap().request_redraw();
            }
            WindowEvent::MouseWheel { device_id:_, delta, phase:_ } => {
                self.scroll_delta_distance = match delta {
                    MouseScrollDelta::LineDelta(x,y ) => (x,y),
                    MouseScrollDelta::PixelDelta(position) => position.into()
                };
                viewport.window.as_ref().unwrap().request_redraw();
            }
            WindowEvent::CursorMoved { device_id:_, position } => {
                self.mouse_poistion = position.into();
                viewport.window.as_ref().unwrap().request_redraw();
            }
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, event: InternalEvents) {
        if let InternalEvents::RebuildLayout(path) = event {
            let xml_string = read_to_string(path).unwrap();
            self.parser.update_page(&xml_string);
            for (_window_id,viewport) in self.viewports.iter_mut() {
                viewport.window.as_ref().unwrap().request_redraw();
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
