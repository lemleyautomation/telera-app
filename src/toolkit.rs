use std::str::FromStr;
use std::fmt::Debug;

use telera_layout::{Color, TextConfig};
use telera_layout::{LayoutEngine, MeasureText, ElementConfiguration};
use crate::treeview::*;

use crate::ui_shapes::Shapes;
use crate::ParserDataAccess;

pub fn treeview<Renderer, Image, Custom, CustomLayout, UserApp, Event>(
    _clicked: bool,
    name: &str,
    layout_engine: &mut LayoutEngine<Renderer, Image, Custom, CustomLayout>,
    user_app: &UserApp,
) -> Vec<Event>
where 
    Renderer: MeasureText,
    Image: Clone+Debug+Default+PartialEq, 
    Event: FromStr+Clone+PartialEq+Debug, 
    Custom: Debug+Default,
    UserApp: ParserDataAccess<Image, Event>,
    Event: Clone+Debug+PartialEq+FromStr,
    <Event as FromStr>::Err: Debug,

{
    if let Some(treeview) = user_app.get_treeview(name) {
        let mut offset: u16 = 0;
        recursive_treeview_layout(layout_engine, &treeview, &mut offset);
    }

    Vec::<Event>::new()
}

fn recursive_treeview_layout<Renderer, Image, Custom, CustomLayout>(
    layout_engine: &mut LayoutEngine<Renderer, Image, Custom, CustomLayout>,
    treeview: &TreeViewItem,
    mut offset: &mut u16,
)
where 
    Renderer: MeasureText,
    Image: Clone+Debug+Default+PartialEq, 
    Custom: Debug+Default,
{
    layout_engine.open_element();
    layout_engine.configure_element(&ElementConfiguration::new()
        .x_grow()
        .direction(true)
    );
    match treeview {
        TreeViewItem::EmptyRoot{label: _} => {
            add_treeview_image_to_layout(
                &mut offset,
                treeview,
                layout_engine
            );
        }
        TreeViewItem::Root{label: _, items} => {
            add_treeview_image_to_layout(
                &mut offset,
                treeview,
                layout_engine
            );
            for item in items {
                recursive_treeview_layout(
                    layout_engine,
                    item,
                    offset
                );
            }
        }
        TreeViewItem::EmptyItem{label:_} => {
            add_treeview_image_to_layout(
                &mut offset,
                treeview, 
                layout_engine
            );
        }
        TreeViewItem::EmptyLastItem{label:_} => {
            add_treeview_image_to_layout(
                &mut offset,
                treeview, 
                layout_engine
            );
        }
        TreeViewItem::ExpandedItem{label: _, items} => {
            add_treeview_image_to_layout(
                &mut offset,
                treeview,
                layout_engine
            );

            layout_engine.open_element();
            layout_engine.configure_element(&ElementConfiguration::new().x_grow());

            layout_engine.open_element();
            layout_engine.configure_element(&ElementConfiguration::new()
                .x_fixed(20.0)
                .y_grow()
                .color(Color{r:0.0,g:96.0,b:255.0,a:255.0})
                .custom_element(&Shapes::Line{width:2.0})
            );
            layout_engine.close_element();

            layout_engine.open_element();
            layout_engine.configure_element(&ElementConfiguration::new()
                .x_grow()
                .direction(true)
            );
            
            for item in items {
                recursive_treeview_layout(
                    layout_engine,
                    item,
                    &mut offset
                );
            }
            layout_engine.close_element();
            layout_engine.close_element();
        }
        _ => {}
    }
    layout_engine.close_element();
}

fn add_treeview_image_to_layout<Renderer, Image, Custom, CustomLayout>(
    _offset: &mut u16,
    treeview_type: &TreeViewItem,
    layout_engine: &mut LayoutEngine<Renderer, Image, Custom, CustomLayout>
)
where 
    Renderer: MeasureText,
    Image: Clone+Debug+Default+PartialEq, 
    Custom: Debug+Default,
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

    layout_engine.open_element();
    let mut container_config = ElementConfiguration::new()
        .align_children_y_center()
        .child_gap(3)
        .x_grow()
        .end();
    if layout_engine.hovered() {
        container_config = container_config.color(blue).end();
        label_config = label_config.color(white).end();
    }
    layout_engine.configure_element(&container_config);
    match treeview_type {
        TreeViewItem::EmptyRoot{label} => {
            layout_engine.open_element();
            layout_engine.configure_element(&ElementConfiguration::new()
                .x_fixed(20.0)
                .y_fixed(20.0)
                .padding_all(5)
            );
                layout_engine.open_element();
                layout_engine.configure_element(
                    &icon_config
                        .color(red)
                        .x_fixed(10.0)
                        .y_fixed(10.0)
                );
                layout_engine.close_element();
            layout_engine.close_element();

            layout_engine.add_text_element(
                label, 
                &label_config,
                false,
            );
        }
        TreeViewItem::Root{label, items:_} => {
            layout_engine.open_element();
            layout_engine.configure_element(&ElementConfiguration::new()
                .x_fixed(20.0)
                .y_fixed(20.0)
                .padding_all(5)
            );
                layout_engine.open_element();
                layout_engine.configure_element(
                    &icon_config
                        .color(green)
                        .x_fixed(10.0)
                        .y_fixed(10.0)
                );
                layout_engine.close_element();
            layout_engine.close_element();

            layout_engine.add_text_element(
                label, 
                &label_config,
                false,
            );
        }
        TreeViewItem::EmptyItem{label} => {
            layout_engine.open_element();
            layout_engine.configure_element(&ElementConfiguration::new()
                .x_fixed(20.0)
                .y_fixed(20.0)
                .padding_all(5)
            );
                layout_engine.open_element();
                layout_engine.configure_element(
                    &icon_config
                        .color(yellow)
                        .x_fixed(10.0)
                        .y_fixed(10.0)
                );
                layout_engine.close_element();
            layout_engine.close_element();
            
            layout_engine.add_text_element(
                label, 
                &label_config,
                false,
            );
        }
        TreeViewItem::EmptyLastItem{label} => {
            layout_engine.open_element();
            layout_engine.configure_element(&ElementConfiguration::new()
                .x_fixed(20.0)
                .y_fixed(20.0)
                .padding_all(5)
            );
                layout_engine.open_element();
                layout_engine.configure_element(
                    &icon_config
                        .color(orange)
                        .x_fixed(10.0)
                        .y_fixed(10.0)
                );
                layout_engine.close_element();
            layout_engine.close_element();
            
            layout_engine.add_text_element(
                label, 
                &label_config,
                false,
            );
        }
        TreeViewItem::ExpandedItem { label, items: _ } => {
            layout_engine.open_element();
            layout_engine.configure_element(
                &icon_config.color(red)
            );
            layout_engine.close_element();

            layout_engine.add_text_element(
                label, 
                &label_config,
                false,
            );
        }
        _ => {}
    }
    layout_engine.close_element();
}