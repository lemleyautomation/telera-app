use std::{hash::Hash, fmt::Debug, str::FromStr};
use winit::{event_loop::{ControlFlow, EventLoop}, window::WindowAttributes};
pub use winit::window::Window;
pub use winit::dpi::LogicalSize;
pub use winit::window::WindowId;

mod graphics_context;

mod depth_texture;

mod viewport;

mod ui_renderer;
pub use ui_renderer::UIImageDescriptor;

pub use telera_layout::ParserDataAccess;
pub use telera_layout::EnumString;
pub use telera_layout::strum;

mod scene_renderer;

mod camera_controller;

mod model;

mod texture;

mod application;
mod application_drm;


pub struct Core{
    staged_windows: Vec<WindowAttributes>,
}

impl Core{
    pub fn create_window(&mut self, attributes: WindowAttributes){
        self.staged_windows.push(attributes);
    }
}

#[allow(unused_variables)]
pub trait App<UserEvents, ImageElementData: Debug, CustomElementData: Debug, CustomLayoutSettings>{
    /// called once before start
    fn initialize(&self, core: &mut Core);
    /// All application update logic
    /// 
    /// This will be called at the beginning of each render loop
    //fn update(&self, layout: &mut LayoutEngine<UIRenderer, ImageElementData, CustomElementData, CustomLayoutSettings>) -> Vec<RenderCommand::<ImageElementData, CustomElementData, CustomLayoutSettings>>;
    /// handling of user events
    fn event_handler(&mut self, event: UserEvents, core: &mut Core){}
}



pub fn run<UserEvents, UserApp, UserPages>(user_application: UserApp)
where 
    UserEvents: FromStr+Clone+PartialEq+Default+Debug,
    <UserEvents as FromStr>::Err: Debug,
    UserPages: FromStr+Clone+Hash+Eq+Default,
    <UserPages as FromStr>::Err: Debug,
    UserApp: App<UserEvents, (),(),()> + ParserDataAccess<(), UserEvents>,
{

    // let event_loop = match EventLoop::<application::InternalEvents>::with_user_event().build() {
    //     Ok(event_loop) => event_loop,
    //     Err(_) => return
    // };
    // event_loop.set_control_flow(ControlFlow::Wait);

    #[cfg(debug_assertions)]
    {
        // let file_watcher = event_loop.create_proxy();
        // //let watcher = watch_file("src/layouts", file_watcher);

        // let expensive_closure = move |event: Result<notify::Event>| {
        //     match event {
        //         Err(e) => {println!("{:?}", e)}
        //         Ok(event) => {
        //             if let Some(path) = event.paths.first(){
        //                 if event.kind == notify::EventKind::Modify(notify::event::ModifyKind::Any) {
        //                     let _ = file_watcher.send_event(InternalEvents::RebuildLayout(path.to_owned()));
        //                 }
        //             }
        //         }
        //     }
        // };
    
        // let mut watcher = notify::recommended_watcher(expensive_closure).unwrap();
    
        // watcher.watch(Path::new("src/layouts"), RecursiveMode::NonRecursive).unwrap();

        //let mut app: Application<UserApp, UserEvents, UserPages> = Application::new(event_loop.create_proxy(), user_application, Some(watcher));
        // let mut app: application::Application<UserApp, UserEvents, UserPages, ()> = application::Application::new(event_loop.create_proxy(), user_application, None);
        // event_loop.run_app(&mut app).unwrap();
        let mut app = application_drm::Application::<UserApp, UserEvents, UserPages, ()>::new(user_application, None);

    }
    
    #[cfg(not(debug_assertions))]
    {
        // let mut app: application::Application<UserApp, UserEvents, UserPages,()> = application::Application::new(event_loop.create_proxy(), user_application, None);
        // event_loop.run_app(&mut app).unwrap();
        application_drm::drm::<UserEvents, UserApp, UserPages>(user_application);
    }
}
