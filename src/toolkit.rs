use std::str::FromStr;
use std::fmt::Debug;

use telera_layout::{Color, TextConfig};
use telera_layout::ElementConfiguration;
use crate::{treeview::*, API, EventHandler};

use crate::ui_shapes::Shapes;
use crate::ParserDataAccess;

pub fn treeview<Image, UserApp, Event>(
    name: &str,
    api: &mut API,
    user_app: &UserApp,
    mut events: Vec<Event>
) -> Vec<Event>
where 
    Image: Clone+Debug+Default+PartialEq, 
    Event: FromStr+Clone+PartialEq+Debug+EventHandler, 
    UserApp: ParserDataAccess<Image, Event>,
{
    if let Some(treeview) = user_app.get_treeview(name) {
        events = recursive_treeview_layout(api, &treeview, events);
    }

    events
}

fn recursive_treeview_layout<Event: FromStr+Clone+PartialEq+Debug+EventHandler>(
    api: &mut API,
    treeview: &TreeViewItem<Event>,
    mut events: Vec<Event>
) -> Vec<Event>
{
    api.ui_layout.open_element();
    api.ui_layout.configure_element(&ElementConfiguration::new()
        .x_grow()
        .direction(true)
    );
    match treeview {
        TreeViewItem::EmptyRoot{label: _, left_clicked, right_clicked} => {
            if api.ui_layout.hovered() && api.left_mouse_clicked
                && let Some(left_click_event) = left_clicked
            {
                events.push(left_click_event.clone());
            }
            if api.ui_layout.hovered() && api.right_mouse_clicked
                && let Some(right_click_event) = right_clicked
            {
                events.push(right_click_event.clone());
            }
            add_treeview_image_to_layout(
                treeview,
                api
            );
        }
        TreeViewItem::Root{label: _, items} => {
            add_treeview_image_to_layout(
                treeview,
                api
            );
            for item in items {
                events = recursive_treeview_layout(
                    api,
                    item,
                    events
                );
            }
        }
        TreeViewItem::EmptyItem{label:_} => {
            add_treeview_image_to_layout(
                treeview, 
                api
            );
        }
        TreeViewItem::EmptyLastItem{label:_} => {
            add_treeview_image_to_layout(
                treeview, 
                api
            );
        }
        TreeViewItem::ExpandedItem{label: _, items} => {
            add_treeview_image_to_layout(
                treeview,
                api
            );

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
                events = recursive_treeview_layout(
                    api,
                    item,
                    events
                );
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
    api: &mut API
)
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
        TreeViewItem::EmptyRoot{label, left_clicked: _, right_clicked: _} => {
            api.ui_layout.open_element();
            api.ui_layout.configure_element(&ElementConfiguration::new()
                .x_fixed(20.0)
                .y_fixed(20.0)
                .padding_all(5)
            );
                api.ui_layout.open_element();
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
        TreeViewItem::Root{label, items:_} => {
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
        TreeViewItem::EmptyItem{label} => {
            api.ui_layout.open_element();
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
        TreeViewItem::EmptyLastItem{label} => {
            api.ui_layout.open_element();
            api.ui_layout.configure_element(&ElementConfiguration::new()
                .x_fixed(20.0)
                .y_fixed(20.0)
                .padding_all(5)
            );
                api.ui_layout.open_element();
                api.ui_layout.configure_element(
                    &icon_config
                        .color(orange)
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
        TreeViewItem::ExpandedItem { label, items: _ } => {
            api.ui_layout.open_element();
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
        _ => {}
    }
    api.ui_layout.close_element();
}