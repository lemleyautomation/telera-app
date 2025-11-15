//#![windows_subsystem = "windows"]

use telera_app::*;

#[derive(Debug, Default, Clone, PartialEq)]
enum BasicEvents {
    #[default]
    None,
}

impl std::str::FromStr for BasicEvents{
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            _ => Err(())
        }
    }
}

impl EventHandler for BasicEvents {
    type UserApplication = BasicApp;
    fn dispatch(&self, _app: &mut Self::UserApplication, _context: Option<EventContext>, _api: &mut API) {
        
    }
}

#[derive(Default)]
struct BasicApp {}

impl App for BasicApp {
    fn initialize(&mut self, core: &mut API) {
        let new_window =
            winit::window::Window::default_attributes().with_inner_size(LogicalSize::new(800, 600));
        core.create_viewport("Main", "testing", new_window);

        // let pic = include_bytes!("../pic.jpg");
        // let pic = pic.as_slice();
        // let pic = image::load_from_memory(pic).unwrap();
        // core.add_image("pic", pic);
        // self.pic = UIImageDescriptor {
        //     atlas: "pic".to_string(),
        //     u1: 0.0, v1: 0.0, u2: 1.0, v2: 1.0
        // }
    }
}

impl ParserDataAccess<BasicEvents> for BasicApp {
    
}

fn main() {
    let app = BasicApp { };

    run::<BasicEvents, BasicApp>(app);
}
