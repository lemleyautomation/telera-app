use symbol_table::GlobalSymbol;
use telera_layout::ElementConfiguration;
use telera_layout::{Color, TextConfig};

use crate::LayoutRunnerReflection;
use crate::{API, CustomElement, EventContext, MT, ui_toolkit::ui_shapes::LineConfig};

/// An event a tree view row can fire, as an interned handler name - the same
/// `GlobalSymbol` the layout runner hands to `LayoutReflector::dispatch_event`.
type UserEvent = GlobalSymbol;

#[derive(Clone)]
pub struct TreeViewEvents {
    pub bubble_left_clicked: Option<UserEvent>,
    pub bubble_right_clicked: Option<UserEvent>,
    pub label_left_clicked: Option<UserEvent>,
    pub label_right_clicked: Option<UserEvent>,
    pub icon_left_clicked: Option<UserEvent>,
    pub icon_right_clicked: Option<UserEvent>,
    pub user_context: Option<EventContext>,
}

impl Default for TreeViewEvents {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeViewEvents {
    pub fn new() -> Self {
        TreeViewEvents {
            bubble_left_clicked: None,
            bubble_right_clicked: None,
            label_left_clicked: None,
            label_right_clicked: None,
            icon_left_clicked: None,
            icon_right_clicked: None,
            user_context: None,
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
            user_context: None,
        }
    }
    pub fn add_right_label(mut self, event: UserEvent) -> Self {
        self.label_right_clicked = Some(event);
        self
    }
    pub fn add_context(mut self, context: EventContext) -> Self {
        self.user_context = Some(context);
        self
    }
}

#[derive(Clone)]
pub enum TreeViewItem<'frame> {
    EmptyRoot {
        label: &'frame str,
        event_definitions: Option<TreeViewEvents>,
    },
    Root {
        label: &'frame str,
        event_definitions: Option<TreeViewEvents>,
        items: Vec<TreeViewItem<'frame>>,
    },

    EmptyItem {
        label: &'frame str,
        event_definitions: Option<TreeViewEvents>,
    },
    CollapsedItem {
        label: &'frame str,
        event_definitions: Option<TreeViewEvents>,
    },
    ExpandedItem {
        label: &'frame str,
        event_definitions: Option<TreeViewEvents>,
        items: Vec<TreeViewItem<'frame>>,
    },
}

pub fn treeview<UserApp>(
    name: &GlobalSymbol,
    list_data: &Option<(GlobalSymbol, usize)>,
    api: &mut API<UserApp>,
    mt: &mut MT,
    user_app: &UserApp,
    mut events: Vec<(GlobalSymbol, Option<EventContext>)>,
) -> Vec<(GlobalSymbol, Option<EventContext>)>
where
    UserApp: LayoutRunnerReflection,
{
    if let Some(treeview) = user_app.get_treeview(name, list_data) {
        events = recursive_treeview_layout(api, mt, &treeview, events);
    }

    events
}

fn recursive_treeview_layout<UserApp>(
    api: &mut API<UserApp>,
    mt: &mut MT,
    treeview: &TreeViewItem,
    mut events: Vec<(GlobalSymbol, Option<EventContext>)>,
) -> Vec<(GlobalSymbol, Option<EventContext>)>
where
    UserApp: LayoutRunnerReflection,
{
    api.l.open_element();
    api.l
        .configure_element(ElementConfiguration::new().x_grow().direction(true));

    events = add_treeview_image_to_layout(treeview, api, mt, events);

    match treeview {
        TreeViewItem::Root {
            label: _,
            event_definitions: _,
            items,
        } => {
            for item in items {
                events = recursive_treeview_layout(api, mt, item, events);
            }
        }
        TreeViewItem::ExpandedItem {
            label: _,
            event_definitions: _,
            items,
        } => {
            api.l.open_element();
            api.l
                .configure_element(ElementConfiguration::new().x_grow());

            api.l.open_element();
            api.l.configure_element(
                ElementConfiguration::new()
                    .x_fixed(20.0)
                    .y_grow()
                    .color(Color {
                        r: 0.0,
                        g: 96.0,
                        b: 255.0,
                        a: 255.0,
                    })
                    .custom_element(&CustomElement::Line(LineConfig {
                        width_source: None,
                        width: 2.0,
                    })),
            );
            api.l.close_element();

            api.l.open_element();
            api.l
                .configure_element(ElementConfiguration::new().x_grow().direction(true));

            for item in items {
                events = recursive_treeview_layout(api, mt, item, events);
            }
            api.l.close_element();
            api.l.close_element();
        }
        _ => {}
    }
    api.l.close_element();

    events
}

fn add_treeview_image_to_layout<UserApp>(
    treeview_type: &TreeViewItem,
    api: &mut API<UserApp>,
    mt: &mut MT,
    mut events: Vec<(GlobalSymbol, Option<EventContext>)>,
) -> Vec<(GlobalSymbol, Option<EventContext>)>
where
    UserApp: LayoutRunnerReflection,
{
    let green = Color {
        r: 0.0,
        g: 255.0,
        b: 0.0,
        a: 255.0,
    };
    let blue = Color {
        r: 0.0,
        g: 0.0,
        b: 255.0,
        a: 255.0,
    };
    let yellow = Color {
        r: 255.0,
        g: 255.0,
        b: 0.0,
        a: 255.0,
    };
    let red = Color {
        r: 255.0,
        g: 0.0,
        b: 0.0,
        a: 255.0,
    };
    let orange = Color {
        r: 255.0,
        g: 120.0,
        b: 0.0,
        a: 255.0,
    };
    let black = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 255.0,
    };
    let white = Color {
        r: 255.0,
        g: 255.0,
        b: 255.0,
        a: 255.0,
    };

    let mut icon_config = ElementConfiguration::new()
        .x_fixed(20.0)
        .y_fixed(20.0)
        .padding_all(0)
        .padding_right(10)
        .custom_element(&CustomElement::Circle)
        .end();
    let mut label_config = TextConfig::new().color(black).font_size(12).end();

    api.l.open_element();
    let mut container_config = ElementConfiguration::new()
        .align_children_y_center()
        .child_gap(3)
        .x_grow()
        .end();
    if api.l.hovered() {
        container_config = container_config.color(blue).end();
        label_config = label_config.color(white).end();
    }

    api.l.configure_element(&container_config);
    match treeview_type {
        TreeViewItem::EmptyRoot {
            label,
            event_definitions,
        } => {
            api.l.open_element();
            api.l.configure_element(
                ElementConfiguration::new()
                    .x_fixed(20.0)
                    .y_fixed(20.0)
                    .padding_all(5),
            );
            api.l.open_element();

            if api.l.hovered()
                && let Some(eventsd) = event_definitions
            {
                if api.left_mouse_clicked
                    && let Some(left_click_event) = eventsd.bubble_left_clicked.clone()
                {
                    let eee = {
                        match &eventsd.user_context {
                            Some(cc) => Some(EventContext {
                                text: Some(label.to_string()),
                                code: cc.code,
                                code2: cc.code2,
                            }),
                            None => Some(EventContext {
                                text: Some(label.to_string()),
                                code: None,
                                code2: None,
                            }),
                        }
                    };
                    events.push((left_click_event.clone(), eee));
                }
                if api.right_mouse_clicked
                    && let Some(right_click_event) = eventsd.bubble_right_clicked.clone()
                {
                    let eee = {
                        match &eventsd.user_context {
                            Some(cc) => Some(EventContext {
                                text: Some(label.to_string()),
                                code: cc.code,
                                code2: cc.code2,
                            }),
                            None => Some(EventContext {
                                text: Some(label.to_string()),
                                code: None,
                                code2: None,
                            }),
                        }
                    };
                    events.push((right_click_event.clone(), eee));
                }
            }

            api.l
                .configure_element(icon_config.color(red).x_fixed(10.0).y_fixed(10.0));
            api.l.close_element();
            api.l.close_element();

            api.l.add_text_element(label, &label_config, false, mt);
        }
        TreeViewItem::Root {
            label,
            event_definitions: _,
            items: _,
        } => {
            api.l.open_element();

            api.l.configure_element(
                ElementConfiguration::new()
                    .x_fixed(20.0)
                    .y_fixed(20.0)
                    .padding_all(5),
            );
            api.l.open_element();
            api.l
                .configure_element(icon_config.color(green).x_fixed(10.0).y_fixed(10.0));
            api.l.close_element();
            api.l.close_element();

            api.l.add_text_element(label, &label_config, false, mt);
        }
        TreeViewItem::EmptyItem {
            label,
            event_definitions,
        } => {
            if api.right_mouse_clicked
                && let Some(eventsd) = event_definitions
                && let Some(right_click_event) = eventsd.label_right_clicked.clone()
            {
                let eee = {
                    match &eventsd.user_context {
                        Some(cc) => Some(EventContext {
                            text: Some(label.to_string()),
                            code: cc.code,
                            code2: cc.code2,
                        }),
                        None => Some(EventContext {
                            text: Some(label.to_string()),
                            code: None,
                            code2: None,
                        }),
                    }
                };
                events.push((right_click_event.clone(), eee));
            }

            api.l.open_element();

            if api.l.hovered()
                && let Some(eventsd) = event_definitions
            {
                if api.left_mouse_clicked
                    && let Some(left_click_event) = eventsd.bubble_left_clicked.clone()
                {
                    let eee = {
                        match &eventsd.user_context {
                            Some(cc) => Some(EventContext {
                                text: Some(label.to_string()),
                                code: cc.code,
                                code2: cc.code2,
                            }),
                            None => Some(EventContext {
                                text: Some(label.to_string()),
                                code: None,
                                code2: None,
                            }),
                        }
                    };
                    events.push((left_click_event.clone(), eee));
                }
                if api.right_mouse_clicked
                    && let Some(right_click_event) = eventsd.bubble_right_clicked.clone()
                {
                    let eee = {
                        match &eventsd.user_context {
                            Some(cc) => Some(EventContext {
                                text: Some(label.to_string()),
                                code: cc.code,
                                code2: cc.code2,
                            }),
                            None => Some(EventContext {
                                text: Some(label.to_string()),
                                code: None,
                                code2: None,
                            }),
                        }
                    };
                    events.push((right_click_event.clone(), eee));
                }
            }

            api.l.configure_element(
                ElementConfiguration::new()
                    .x_fixed(20.0)
                    .y_fixed(20.0)
                    .padding_all(5),
            );
            api.l.open_element();
            api.l
                .configure_element(icon_config.color(yellow).x_fixed(10.0).y_fixed(10.0));
            api.l.close_element();
            api.l.close_element();

            api.l.add_text_element(label, &label_config, false, mt);
        }
        TreeViewItem::CollapsedItem {
            label,
            event_definitions,
        } => {
            if api.right_mouse_clicked
                && let Some(eventsd) = event_definitions
                && let Some(right_click_event) = eventsd.label_right_clicked.clone()
            {
                let eee = {
                    match &eventsd.user_context {
                        Some(cc) => Some(EventContext {
                            text: Some(label.to_string()),
                            code: cc.code,
                            code2: cc.code2,
                        }),
                        None => Some(EventContext {
                            text: Some(label.to_string()),
                            code: None,
                            code2: None,
                        }),
                    }
                };
                events.push((right_click_event.clone(), eee));
            }

            api.l.open_element();

            if api.l.hovered()
                && let Some(eventsd) = event_definitions
            {
                if api.left_mouse_clicked
                    && let Some(left_click_event) = eventsd.bubble_left_clicked.clone()
                {
                    let eee = {
                        match &eventsd.user_context {
                            Some(cc) => Some(EventContext {
                                text: Some(label.to_string()),
                                code: cc.code,
                                code2: cc.code2,
                            }),
                            None => Some(EventContext {
                                text: Some(label.to_string()),
                                code: None,
                                code2: None,
                            }),
                        }
                    };
                    events.push((left_click_event.clone(), eee));
                }
                if api.right_mouse_clicked
                    && let Some(right_click_event) = eventsd.bubble_right_clicked.clone()
                {
                    let eee = {
                        match &eventsd.user_context {
                            Some(cc) => Some(EventContext {
                                text: Some(label.to_string()),
                                code: cc.code,
                                code2: cc.code2,
                            }),
                            None => Some(EventContext {
                                text: Some(label.to_string()),
                                code: None,
                                code2: None,
                            }),
                        }
                    };
                    events.push((right_click_event.clone(), eee));
                }
            }

            api.l.configure_element(icon_config.color(orange));
            api.l.close_element();

            api.l.add_text_element(label, &label_config, false, mt);
        }
        TreeViewItem::ExpandedItem {
            label,
            event_definitions,
            items: _,
        } => {
            if api.right_mouse_clicked
                && let Some(eventsd) = event_definitions
                && let Some(right_click_event) = eventsd.label_right_clicked.clone()
            {
                let eee = {
                    match &eventsd.user_context {
                        Some(cc) => Some(EventContext {
                            text: Some(label.to_string()),
                            code: cc.code,
                            code2: cc.code2,
                        }),
                        None => Some(EventContext {
                            text: Some(label.to_string()),
                            code: None,
                            code2: None,
                        }),
                    }
                };
                events.push((right_click_event.clone(), eee));
            }

            api.l.open_element();

            if api.l.hovered()
                && let Some(eventsd) = event_definitions
            {
                if api.left_mouse_clicked
                    && let Some(left_click_event) = eventsd.bubble_left_clicked.clone()
                {
                    let eee = {
                        match &eventsd.user_context {
                            Some(cc) => Some(EventContext {
                                text: Some(label.to_string()),
                                code: cc.code,
                                code2: cc.code2,
                            }),
                            None => Some(EventContext {
                                text: Some(label.to_string()),
                                code: None,
                                code2: None,
                            }),
                        }
                    };
                    events.push((left_click_event.clone(), eee));
                }
                if api.right_mouse_clicked
                    && let Some(right_click_event) = eventsd.bubble_right_clicked.clone()
                {
                    let eee = {
                        match &eventsd.user_context {
                            Some(cc) => Some(EventContext {
                                text: Some(label.to_string()),
                                code: cc.code,
                                code2: cc.code2,
                            }),
                            None => Some(EventContext {
                                text: Some(label.to_string()),
                                code: None,
                                code2: None,
                            }),
                        }
                    };
                    events.push((right_click_event.clone(), eee));
                }
            }

            api.l.configure_element(icon_config.color(red));
            api.l.close_element();

            api.l.add_text_element(label, &label_config, false, mt);
        }
    }
    api.l.close_element();
    events
}
