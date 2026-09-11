use telera_app::*;

#[derive(LayoutRunnerReflection)]
struct MyApp {
    click_counter: u32,
}

#[telera_app]
impl MyApp {
    #[layout_element]
    fn custom_element(&mut self, api: &mut API) {
        api.l.open_element();
        api.l.configure_element(
            &ElementConfiguration::new()
                .width_fixed(20.0)
                .height_fixed(20.0)
                .color(Color::rgb(255, 0, 0))
                .end(),
        );
        api.l.close_element();
    }

    #[layout_event]
    fn click_handler(&mut self, _context: Option<EventContext>, _api: &mut API) {
        self.click_counter += 1;
        println!("Sidebar clicked {:} times", self.click_counter);
    }
}

impl App for MyApp {
    fn initialize(&mut self) -> Startup {
        Startup {
            window_attributes: Window::default_attributes()
                .with_inner_size(LogicalSize::new(900, 600)),
            window_name: "Custom".to_string(),
            page: None,
            watch_path: RunType::Watch("examples/layouts".to_string()),
            agent_access_port: None,
        }
    }
}

fn main() {
    run::<MyApp>(MyApp { click_counter: 0 });
}
