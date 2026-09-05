use symbol_table::GlobalSymbol;
use telera_app::*;

// #[inject_code]
// mod CustomElements {
//     use crate::{Event, LayoutApp};
//     use telera_app::*;

//     pub fn yeet(app: &mut LayoutApp, api: &mut API<Event, LayoutApp>, _mt: &mut MT) {}
// }
// use CustomElements::yeet;

#[derive(Clone, Debug, Default, PartialEq, strum_macros::EnumString, EventHandler)]
#[handler_for(LayoutApp)]
enum Event {
    #[default]
    None,
}

#[derive(LayoutRunnerReflection)]
#[event_handler(Event)]
struct LayoutApp {}

#[inject_code]
impl LayoutApp {
    fn custom_element(&mut self, api: &mut API<Event, LayoutApp>, _mt: &mut MT) {
        api.l.open_element();
        api.l.configure_element(
            &ElementConfiguration::new()
                .x_fixed(20.0)
                .y_fixed(20.0)
                .color(Color::rgb(255, 0, 0))
                .end(),
        );
        api.l.close_element();
    }
}

impl LayoutRunnerCustomElements<Event, LayoutApp> for LayoutApp {
    fn dispatch(
        &mut self,
        name: &symbol_table::GlobalSymbol,
        api: &mut API<Event, LayoutApp>,
        mt: &mut MT,
    ) {
        if name.eq(&GlobalSymbol::new("custom_element")) {
            self.custom_element(api, mt);
        }
    }
}

impl App for LayoutApp {
    type Event = Event;

    fn initialize(&mut self) -> Startup {
        Startup {
            initial_window: Window::default_attributes()
                .with_inner_size(LogicalSize::new(900, 600)),
            window_name: "Custom".to_string(),
            watch_path: RunType::Watch("src/layouts".to_string()),
        }
    }

    fn layout(&mut self, page: &str, api: &mut API<Event, LayoutApp>, mt: &mut MT) {
        api.run_layout(page, mt, self);
        //yeet(self, api, mt);
    }
}

fn main() {
    let app = LayoutApp {};

    run::<LayoutApp>(app);
}
