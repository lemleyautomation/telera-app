//#![windows_subsystem = "windows"]

#![cfg_attr(rustfmt, rustfmt_skip)]

use telera_app::*;
use telera_layout::ElementConfiguration;


#[derive(Default)]
struct BasicApp {
    radii: f32
}

impl LayoutRunnerReflection for BasicApp {}
impl LayoutReflector<BasicApp> for BasicApp {}

impl App for BasicApp {
    fn initialize(&mut self) -> Startup {
        Startup {
            initial_window: Window::default_attributes().with_inner_size(LogicalSize::new(800, 600)),
            window_name: "Main".to_string(),
            watch_path: RunType::None
        }
    }

    fn layout(&mut  self, _page: &str, api: &mut API<BasicApp>, _mt: &mut MT) {
        macro_rules! e {
            ($v:expr $(, $c:stmt)* $(,)? ) => {
                api.l.open_element();
                api.l.configure_element(&$v);
                $(
                    $c
                )*
                api.l.close_element();
            };
        }

        // macro_rules! t {
        //     ($v:expr, $c:expr) => {
        //         api.l.add_text_element($c, &$v, true, mt);
        //     };
        // }


        let root = ElementConfiguration::new()
            .direction(true)
            .color(Color::rgb(0,0,0))
            .padding_all(20)
            .child_gap(20)
            .end();

        let square1 = ElementConfiguration::new()
            .color(Color::rgb(100,100, 100))
            .x_fixed(200.0)
            .y_fixed(200.0)
            .border_top(10)
            .radius_all(self.radii+10.0)
            .border_color(Color::rgb(0,208,0))
            .end();

        e!(root, e!(square1), e!(square1), e!(square1), e!(square1));
        self.radii += 0.01;
    }
}

fn main() {

    let app = BasicApp {
        radii: 0.0
    };

    run::<BasicApp>(app);
}
