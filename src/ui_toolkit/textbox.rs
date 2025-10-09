use std::str::FromStr;
use std::fmt::Debug;

use crate::{EventContext, EventHandler};

use symbol_table::GlobalSymbol;
use telera_layout::{Color, TextConfig};
use telera_layout::ElementConfiguration;
use crate::API;

use crate::ParserDataAccess;

const DEFAULT_TEXT: &str = "";

#[allow(dead_code)]
pub fn text_box<UserApp, Event>(
    content: &GlobalSymbol,
    list_data: &Option<(GlobalSymbol, usize)>,
    api: &mut API,
    user_app: &UserApp,
    events: Vec::<(Event, Option<EventContext>)>
) -> Vec::<(Event, Option<EventContext>)>
where 
    Event: FromStr+Clone+PartialEq+Debug+Default+EventHandler<UserApplication = UserApp>, 
    UserApp: ParserDataAccess<Event>,
{
    //let mut line = Buffer::new(&mut self.font_system, Metrics::new(font_size, line_height));
    
    let clay = &mut api.ui_layout;

    let config = ElementConfiguration::new()
        .border_all(5)
        .x_fit_min(80.0)
        .y_fit_min(20.0)
        .color(Color { r: 255.0, g: 255.0, b: 255.0, a: 255.0 })
        .padding_all(5)
        .end();

    let label_config = TextConfig::new()
        .color(Color{r:0.0,g:0.0,b:0.0,a:255.0})
        .font_size(12)
        .end();

    clay.open_element();
    clay.configure_element(&config);
    
    clay.add_text_element(
        match user_app.get_text(content, list_data) {
            Some(content) => content,
            None => DEFAULT_TEXT
        },
        &label_config,
        false);
    clay.close_element();

    events
}