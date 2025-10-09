use std::str::FromStr;
use std::fmt::Debug;

use crate::{EventContext, EventHandler};

use symbol_table::GlobalSymbol;
use telera_layout::{Color, TextConfig};
use telera_layout::ElementConfiguration;
use crate::API;

use crate::ui_shapes::Shapes;
use crate::ParserDataAccess;

#[derive(Clone)]
pub struct TreeViewEvents<UserEvent: FromStr+Clone+PartialEq+Debug+EventHandler> {
    pub bubble_left_clicked: Option<UserEvent>, 
    pub bubble_right_clicked: Option<UserEvent>,
    pub label_left_clicked: Option<UserEvent>, 
    pub label_right_clicked: Option<UserEvent>,
    pub icon_left_clicked: Option<UserEvent>, 
    pub icon_right_clicked: Option<UserEvent>,
    pub user_context: Option<EventContext>
}

impl <UserEvent: FromStr+Clone+PartialEq+Debug+EventHandler> TreeViewEvents<UserEvent> {
    pub fn new() -> Self {
        TreeViewEvents { 
            bubble_left_clicked: None,
            bubble_right_clicked: None, 
            label_left_clicked: None,
            label_right_clicked: None,
            icon_left_clicked: None,
            icon_right_clicked: None,
            user_context: None
        }
    }
    pub fn from_left_bubble(event: UserEvent) -> Self {
        TreeViewEvents { 
            bubble_left_clicked: Some(event),
            bubble_right_clicked: None, 
            label_left_clicked: None,
            label_right_clicked: None,
            icon_left_clicked: None,
            icon_right_clicked: None,
            user_context: None
        }
    }
    pub fn add_right_label(mut self, event:UserEvent) -> Self {
        self.label_right_clicked = Some(event);
        self
    }
    pub fn add_context(mut self, context: EventContext) -> Self{
        self.user_context = Some(context);
        self
    }
}

#[derive(Clone)]
pub enum TreeViewItem<'frame, UserEvent: FromStr+Clone+PartialEq+Debug+EventHandler>{
    EmptyRoot{label: &'frame str, event_definitions: Option<TreeViewEvents<UserEvent>>},
    Root{label: &'frame str, event_definitions: Option<TreeViewEvents<UserEvent>>, items: Vec<TreeViewItem<'frame, UserEvent>>},

    EmptyItem{label: &'frame str, event_definitions: Option<TreeViewEvents<UserEvent>>},
    CollapsedItem{label: &'frame str, event_definitions: Option<TreeViewEvents<UserEvent>>},
    ExpandedItem{label: &'frame str, event_definitions: Option<TreeViewEvents<UserEvent>>, items: Vec<TreeViewItem<'frame, UserEvent>>},
}

pub fn treeview<UserApp, Event>(
    name: &GlobalSymbol,
    list_data: &Option<(GlobalSymbol, usize)>,
    api: &mut API,
    user_app: &UserApp,
    mut events: Vec::<(Event, Option<EventContext>)>
) -> Vec::<(Event, Option<EventContext>)>
where
    Event: FromStr+Clone+PartialEq+Debug+EventHandler<UserApplication = UserApp>, 
    UserApp: ParserDataAccess<Event>,
{
    if let Some(treeview) = user_app.get_treeview(name, list_data) {
        events = recursive_treeview_layout(api, &treeview, events);
    }

    events
}

fn recursive_treeview_layout<Event: FromStr+Clone+PartialEq+Debug+EventHandler>(
    api: &mut API,
    treeview: &TreeViewItem<Event>,
    mut events: Vec::<(Event, Option<EventContext>)>
) -> Vec::<(Event, Option<EventContext>)>
{
    api.ui_layout.open_element();
    api.ui_layout.configure_element(&ElementConfiguration::new()
        .x_grow()
        .direction(true)
    );

    events = add_treeview_image_to_layout(
        treeview,
        api,
        events,
    );

    match treeview {
        TreeViewItem::Root{label:_, event_definitions:_, items} => {
            for item in items {
                events = recursive_treeview_layout(api, item, events);
            }
        }
        TreeViewItem::ExpandedItem{label:_, event_definitions:_, items} => {
            api.ui_layout.open_element();
            api.ui_layout.configure_element(&ElementConfiguration::new().x_grow());

                api.ui_layout.open_element();
                api.ui_layout.configure_element(&ElementConfiguration::new()
                    .x_fixed(20.0)
                    .y_grow()
                    .color(Color{r:0.0,g:96.0,b:255.0,a:255.0})
                    .custom_element(&Shapes::Line{width:2.0})
                );
                api.ui_layout.close_element();

                api.ui_layout.open_element();
                api.ui_layout.configure_element(&ElementConfiguration::new()
                    .x_grow()
                    .direction(true)
                );
                
                for item in items {
                    events = recursive_treeview_layout(api, item, events);
                }
                api.ui_layout.close_element();
            api.ui_layout.close_element();
        }
        _ => {}
    }
    api.ui_layout.close_element();

    events
}

fn add_treeview_image_to_layout<Event: FromStr+Clone+PartialEq+Debug+EventHandler>(
    treeview_type: &TreeViewItem<Event>,
    api: &mut API,
    mut events: Vec::<(Event, Option<EventContext>)>,
) -> Vec::<(Event, Option<EventContext>)>
{
    let green = Color{r:0.0,g:255.0,b:0.0,a:255.0};
    let blue = Color{r:0.0,g:0.0,b:255.0,a:255.0};
    let yellow = Color{r:255.0,g:255.0,b:0.0,a:255.0};
    let red = Color{r:255.0,g:0.0,b:0.0,a:255.0};
    let orange = Color{r:255.0,g:120.0,b:0.0,a:255.0};
    let black = Color{r:0.0,g:0.0,b:0.0,a:255.0};
    let white = Color { r: 255.0, g: 255.0, b: 255.0, a: 255.0 };

    let mut icon_config = ElementConfiguration::new()
        .x_fixed(20.0)
        .y_fixed(20.0)
        .padding_all(0)
        .padding_right(10)
        .custom_element(&Shapes::Circle)
        .end();
    let mut label_config = TextConfig::new()
        .color(black)
        .font_size(12)
        .end();

    api.ui_layout.open_element();
    let mut container_config = ElementConfiguration::new()
        .align_children_y_center()
        .child_gap(3)
        .x_grow()
        .end();
    if api.ui_layout.hovered() {
        container_config = container_config.color(blue).end();
        label_config = label_config.color(white).end();
    }

    api.ui_layout.configure_element(&container_config);
    match treeview_type {
        TreeViewItem::EmptyRoot{label, event_definitions} => {
            api.ui_layout.open_element();
            api.ui_layout.configure_element(&ElementConfiguration::new()
                .x_fixed(20.0)
                .y_fixed(20.0)
                .padding_all(5)
            );
                api.ui_layout.open_element();

                if api.ui_layout.hovered() && let Some (eventsd) = event_definitions {
                    if api.left_mouse_clicked && let Some(left_click_event) = eventsd.bubble_left_clicked.clone()
                    {
                        let eee = {
                            match &eventsd.user_context {
                                Some(cc) => Some(EventContext{text:Some(label.to_string()),code:cc.code,code2:cc.code2}),
                                None => Some(EventContext { text: Some(label.to_string()), code: None, code2: None })
                            }
                        };
                        events.push((left_click_event.clone(), eee));
                    }
                    if api.right_mouse_clicked && let Some(right_click_event) = eventsd.bubble_right_clicked.clone()
                    {
                        let eee = {
                            match &eventsd.user_context {
                                Some(cc) => Some(EventContext{text:Some(label.to_string()),code:cc.code,code2:cc.code2}),
                                None => Some(EventContext { text: Some(label.to_string()), code: None, code2: None })
                            }
                        };
                        events.push((right_click_event.clone(), eee));
                    }
                }

                api.ui_layout.configure_element(
                    &icon_config
                        .color(red)
                        .x_fixed(10.0)
                        .y_fixed(10.0)
                );
                api.ui_layout.close_element();
            api.ui_layout.close_element();

            api.ui_layout.add_text_element(
                label, 
                &label_config,
                false,
            );
        }
        TreeViewItem::Root{label, event_definitions:_, items:_} => {
            api.ui_layout.open_element();

            api.ui_layout.configure_element(&ElementConfiguration::new()
                .x_fixed(20.0)
                .y_fixed(20.0)
                .padding_all(5)
            );
                api.ui_layout.open_element();
                api.ui_layout.configure_element(
                    &icon_config
                        .color(green)
                        .x_fixed(10.0)
                        .y_fixed(10.0)
                );
                api.ui_layout.close_element();
            api.ui_layout.close_element();

            api.ui_layout.add_text_element(
                label, 
                &label_config,
                false,
            );
        }
        TreeViewItem::EmptyItem{label, event_definitions} => {
            if api.right_mouse_clicked
            && let Some (eventsd) = event_definitions
            && let Some(right_click_event) = eventsd.label_right_clicked.clone() {
                    let eee = {
                    match &eventsd.user_context {
                        Some(cc) => Some(EventContext{text:Some(label.to_string()),code:cc.code,code2:cc.code2}),
                        None => Some(EventContext { text: Some(label.to_string()), code: None, code2: None })
                    }
                };
                events.push((right_click_event.clone(), eee));
            }

            api.ui_layout.open_element();

            if api.ui_layout.hovered() && let Some (eventsd) = event_definitions {
                if api.left_mouse_clicked && let Some(left_click_event) = eventsd.bubble_left_clicked.clone()
                {
                    let eee = {
                        match &eventsd.user_context {
                            Some(cc) => Some(EventContext{text:Some(label.to_string()),code:cc.code,code2:cc.code2}),
                            None => Some(EventContext { text: Some(label.to_string()), code: None, code2: None })
                        }
                    };
                    events.push((left_click_event.clone(), eee));
                }
                if api.right_mouse_clicked
                && let Some(right_click_event) = eventsd.bubble_right_clicked.clone() {
                        let eee = {
                        match &eventsd.user_context {
                            Some(cc) => Some(EventContext{text:Some(label.to_string()),code:cc.code,code2:cc.code2}),
                            None => Some(EventContext { text: Some(label.to_string()), code: None, code2: None })
                        }
                    };
                    events.push((right_click_event.clone(), eee));
                }
            }

            api.ui_layout.configure_element(&ElementConfiguration::new()
                .x_fixed(20.0)
                .y_fixed(20.0)
                .padding_all(5)
            );
                api.ui_layout.open_element();
                api.ui_layout.configure_element(
                    &icon_config
                        .color(yellow)
                        .x_fixed(10.0)
                        .y_fixed(10.0)
                );
                api.ui_layout.close_element();
            api.ui_layout.close_element();
            
            api.ui_layout.add_text_element(
                label, 
                &label_config,
                false,
            );
        }
        TreeViewItem::CollapsedItem { label, event_definitions } => {

            if api.right_mouse_clicked
            && let Some (eventsd) = event_definitions
            && let Some(right_click_event) = eventsd.label_right_clicked.clone() {
                    let eee = {
                    match &eventsd.user_context {
                        Some(cc) => Some(EventContext{text:Some(label.to_string()),code:cc.code,code2:cc.code2}),
                        None => Some(EventContext { text: Some(label.to_string()), code: None, code2: None })
                    }
                };
                events.push((right_click_event.clone(), eee));
            }

            api.ui_layout.open_element();

            if api.ui_layout.hovered() && let Some (eventsd) = event_definitions {
                if api.left_mouse_clicked && let Some(left_click_event) = eventsd.bubble_left_clicked.clone()
                {
                    let eee = {
                        match &eventsd.user_context {
                            Some(cc) => Some(EventContext{text:Some(label.to_string()),code:cc.code,code2:cc.code2}),
                            None => Some(EventContext { text: Some(label.to_string()), code: None, code2: None })
                        }
                    };
                    events.push((left_click_event.clone(), eee));
                }
                if api.right_mouse_clicked
                && let Some(right_click_event) = eventsd.bubble_right_clicked.clone() {
                        let eee = {
                        match &eventsd.user_context {
                            Some(cc) => Some(EventContext{text:Some(label.to_string()),code:cc.code,code2:cc.code2}),
                            None => Some(EventContext { text: Some(label.to_string()), code: None, code2: None })
                        }
                    };
                    events.push((right_click_event.clone(), eee));
                }
            }

            api.ui_layout.configure_element(
                &icon_config.color(orange)
            );
            api.ui_layout.close_element();

            api.ui_layout.add_text_element(
                label, 
                &label_config,
                false,
            );
        }
        TreeViewItem::ExpandedItem { label, event_definitions, items: _ } => {
            if api.right_mouse_clicked
            && let Some (eventsd) = event_definitions
            && let Some(right_click_event) = eventsd.label_right_clicked.clone() {
                    let eee = {
                    match &eventsd.user_context {
                        Some(cc) => Some(EventContext{text:Some(label.to_string()),code:cc.code,code2:cc.code2}),
                        None => Some(EventContext { text: Some(label.to_string()), code: None, code2: None })
                    }
                };
                events.push((right_click_event.clone(), eee));
            }

            api.ui_layout.open_element();

            if api.ui_layout.hovered() && let Some (eventsd) = event_definitions {
                if api.left_mouse_clicked && let Some(left_click_event) = eventsd.bubble_left_clicked.clone()
                {
                    let eee = {
                        match &eventsd.user_context {
                            Some(cc) => Some(EventContext{text:Some(label.to_string()),code:cc.code,code2:cc.code2}),
                            None => Some(EventContext { text: Some(label.to_string()), code: None, code2: None })
                        }
                    };
                    events.push((left_click_event.clone(), eee));
                }
                if api.right_mouse_clicked
                && let Some(right_click_event) = eventsd.bubble_right_clicked.clone() {
                        let eee = {
                        match &eventsd.user_context {
                            Some(cc) => Some(EventContext{text:Some(label.to_string()),code:cc.code,code2:cc.code2}),
                            None => Some(EventContext { text: Some(label.to_string()), code: None, code2: None })
                        }
                    };
                    events.push((right_click_event.clone(), eee));
                }
            }

            api.ui_layout.configure_element(
                &icon_config.color(red)
            );
            api.ui_layout.close_element();

            api.ui_layout.add_text_element(
                label, 
                &label_config,
                false,
            );
        }
    }
    api.ui_layout.close_element();
    events
}