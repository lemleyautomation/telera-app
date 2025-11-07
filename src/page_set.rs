use std::marker::PhantomData;
use std::{collections::HashMap, fmt::Debug, str::FromStr};

use symbol_table::GlobalSymbol;
use winit::window::Cursor;

use crate::{ui_toolkit, UIImageDescriptor};
use crate::EventContext;

use crate::EventHandler;
use crate::API;

use crate::layout_types::*;

use telera_layout::{Color, ElementConfiguration, TextConfig};

const DEFAULT_TEXT: &str = ":(";

pub struct Binder<Event,UserApp>
where
    Event: FromStr+Clone+PartialEq+Debug+Default+EventHandler<UserApplication = UserApp>, 
    <Event as FromStr>::Err: Debug,
    UserApp: ParserDataAccess<Event>,
{
    pages: HashMap<String, Vec<Layout<Event>>>,
    pub reusable: HashMap<GlobalSymbol, Vec<Layout<Event>>>,
    _x: PhantomData<UserApp>,
}

impl<Event,UserApp> Binder<Event,UserApp>
where 
    Event: FromStr+Clone+PartialEq+Debug+Default+EventHandler<UserApplication = UserApp>, 
    <Event as FromStr>::Err: Debug,
    UserApp: ParserDataAccess<Event>,
{
    pub fn new() -> Self {
        Self {
            pages: HashMap::new(),
            reusable: HashMap::new(),
            _x: PhantomData::default(),
        }
    }

    pub fn add_page(&mut self, name: &str, page: Vec<Layout<Event>>) {
        if self.pages.get(name).is_none() {
            self.pages.insert(name.to_string(), page);
        }
    }

    pub fn add_reusable(&mut self, name: &str, page: Vec<Layout<Event>>) {
        let name = GlobalSymbol::new(name);
        if self.reusable.get(&name).is_none() {
            self.reusable.insert(name, page);
        }
    }

    pub fn replace_page(&mut self, name: &str, page: Vec<Layout<Event>>) -> Result<(), ()> {
        if self.pages.get(name).is_some() {
            self.pages.remove(name);
            self.pages.insert(name.to_string(), page);
        }

        Err(())
    }

    pub fn replace_reusable(&mut self, name: &str, reusable: Vec<Layout<Event>>) -> Result<(), ()> {
        let name = GlobalSymbol::new(name);
        if self.reusable.get(&name).is_some() {
            self.reusable.remove(&name);
            self.reusable.insert(name, reusable);
        }

        Err(())
    }

    pub fn set_page<'render_pass>(
        &mut self,
        window_id: winit::window::WindowId,
        api: &mut API,
        user_app: &mut UserApp,
    ) {
        let page = api.viewports.get_mut(&window_id).as_mut().unwrap().page.clone();
        let mut events = Vec::<(Event, Option<EventContext>)>::new();
        let mut pointer = winit::window::CursorIcon::Default;

        if let Some(page_commands) = self.pages.get(&page) {
            let mut command_references = Vec::<&Layout<Event>>::new();
            for command in page_commands.iter() {
                command_references.push(command);
            }

            let mut page_call_stack = HashMap::<GlobalSymbol, &DataSrc<Declaration<Event>>>::new();
            for command in &command_references {
                if let Layout::Declaration { name, value } = command {
                    page_call_stack.insert(*name, value);
                }
                else if let Layout::Element(e) = command
                && let Element::Pointer(_) = e {}
                else {break}
            }

            (events, pointer) = set_layout(
                api,
                &command_references,
                &self.reusable,
                Some(&page_call_stack),
                None,
                None,
                None,
                user_app,
                events,
                pointer
            );

            //api.viewports.get_mut(&window_id).as_mut().unwrap().window.set_cursor(Cursor::Icon(pointer));
            
        }

        for (event, context) in events {
            event.dispatch(user_app, context, api);
        }
    }
}

fn set_layout<'render_pass, Event, UserApp>(
    api: &mut API,
    commands: &Vec<&Layout<Event>>,
    reusables: &HashMap<GlobalSymbol, Vec<Layout<Event>>>,
    locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration<Event>>>>,
    list_data: Option<(GlobalSymbol, usize)>,
    config: Option<&mut ElementConfiguration>,
    text_config: Option<&mut TextConfig>,
    user_app: &UserApp,
    mut events: Vec::<(Event, Option<EventContext>)>,
    mut pointer: winit::window::CursorIcon
) -> (Vec::<(Event, Option<EventContext>)>, winit::window::CursorIcon)
where
    Event: FromStr+Clone+PartialEq+Default+Debug+EventHandler<UserApplication = UserApp>,
    <Event as FromStr>::Err: Debug,
    UserApp: ParserDataAccess<Event>
{
    let mut nesting_level: u32 = 0;
    let mut skip: Option<u32> = None;

    let mut recursive_commands = Vec::<&Layout<Event>>::new();
    let mut recursive_call_stack = HashMap::<GlobalSymbol, &DataSrc<Declaration<Event>>>::new();
    let mut collect_declarations = false;

    let mut collect_list_commands = false;
    
    let mut config = match config {
        None => &mut ElementConfiguration::default(),
        Some(config) => config
    };

    let mut text_config = match text_config {
        None => &mut TextConfig::default(),
        Some(text_config) => text_config
    };

    //println!("{:?}", commands);

    #[allow(unused_variables)]
    for (index, command) in commands.iter().enumerate() {
        if collect_list_commands {
            match command {
                Layout::Element(flow_command) => {
                    if let Element::ListClosed(_) = flow_command {
                        collect_list_commands = false;
                    }
                }
                Layout::Declaration{name:_,value:_} => {}
                other => {
                    collect_declarations = false;
                    recursive_commands.push(other);
                    continue;
                }
            }
        }

        match command {
            Layout::Element(element_type) => {
                match element_type {
                    Element::IfOpened { condition } => {
                        if skip.is_none()
                        && !bool::resolve_name(condition, locals, user_app, &list_data) {
                            skip = Some(nesting_level)
                        }
                        nesting_level += 1;
                    }
                    Element::IfNotOpened { condition } => {
                        if skip.is_none()
                        && bool::resolve_name(condition, locals, user_app, &list_data) {
                            skip = Some(nesting_level)
                        }
                        nesting_level += 1;
                    }
                    Element::IfClosed => {
                        nesting_level -= 1;
                        if let Some(skip_level) = skip {
                            if skip_level >= nesting_level{
                                skip = None;
                            }
                        }
                    }
                    Element::HoverOpened { event } => {
                        if skip.is_none() {
                            skip = Some(nesting_level);

                            if api.ui_layout.hovered() {
                                skip = None;

                                if let Some(event) = event {
                                    events.push((Event::resolve_src(event, locals, user_app, &list_data),None));
                                }
                            }
                        }
                        nesting_level += 1;
                    }
                    Element::HoverClosed => {
                        nesting_level -= 1;

                        if let Some(skip_level) = skip {
                            if skip_level == nesting_level{
                                skip = None;
                            }
                        }
                    }
                    Element::LeftClickedOpened { event } => {
                        //println!("event at click opened: {:?}", event);
                        if skip.is_none() {
                            skip = Some(nesting_level);

                            if api.ui_layout.hovered() && api.left_mouse_clicked {
                                skip = None;

                                if let Some(event) = event {
                                    events.push((Event::resolve_src(event, locals, user_app, &list_data),None));
                                }
                            }
                        }
                        nesting_level += 1;
                    }
                    Element::LeftClickedClosed => {
                        nesting_level -= 1;

                        if let Some(skip_level) = skip {
                            if skip_level == nesting_level{
                                skip = None;
                            }
                        }
                    }
                    Element::RightClickedOpened { event } => {
                        if skip.is_none() {
                            skip = Some(nesting_level);

                            if api.ui_layout.hovered() && api.right_mouse_clicked {
                                skip = None;

                                if let Some(event) = event {
                                    events.push((Event::resolve_src(event, locals, user_app, &list_data),None));
                                }
                            }
                        }
                        nesting_level += 1;
                    }
                    Element::RightClickedClosed => {
                        nesting_level -= 1;

                        if let Some(skip_level) = skip {
                            if skip_level == nesting_level{
                                skip = None;
                            }
                        }
                    }
                    Element::Pointer(new_pointer) => {
                        if skip.is_none() {
                            pointer = new_pointer.clone();
                        }
                    }
                    Element::ListOpened => {
                        nesting_level += 1;

                        if skip.is_none() {
                            recursive_commands.clear();
                            recursive_call_stack.clear();
                            collect_list_commands = true;
                            collect_declarations = true;
                        }
                        
                    }
                    Element::ListClosed(src) => {
                        nesting_level -= 1;

                        if skip.is_none(){

                            if let Some(length) = user_app.get_list_length(src, &None) {
                                for index in 0..length {
                                    (events, pointer) = set_layout(
                                        api,
                                        &recursive_commands, 
                                        reusables,
                                        Some(&recursive_call_stack), 
                                        Some((*src, index)), 
                                        None, 
                                        None, 
                                        user_app,
                                        events,
                                        pointer
                                    );
                                }
                            }
                        }
                    }
                    Element::ElementOpened { id:_ } => {
                        nesting_level += 1;

                        if skip.is_none() {
                            api.ui_layout.open_element();
                            if api.ui_layout.hovered() {
                                let x = api.ui_layout.get_element_id("hi");
                            }
                        }
                    }
                    Element::ElementClosed => {
                        nesting_level -= 1;

                        if skip.is_none() {
                            api.ui_layout.close_element();
                        }
                    }
                    Element::ConfigOpened => {
                        nesting_level += 1;
        
                        if skip.is_none() {
                            *config = ElementConfiguration::default();
                        }
                    }
                    Element::ConfigClosed => {
                        nesting_level -= 1;
        
                        if skip.is_none() {
                            
                            let id = api.ui_layout.configure_element(&config);
                            //config = Some(ElementConfiguration::default());
                            if api.ui_layout.hovered() && api.left_mouse_clicked {
                                api.focus = id;
                                //println!("focus: {:?}", api.focus);
                            }
                        }
                    }
                    Element::TextElementOpened => nesting_level += 1,
                    Element::TextElementClosed(content) => {
                        nesting_level -= 1;
                        if skip.is_none() {
                            let text_content = String::resolve_src(content, locals, user_app, &list_data);
                            api.ui_layout.add_text_element(text_content, &text_config, false);
                        }
                    }
                    Element::TextConfigOpened => {
                        nesting_level += 1;

                        if skip.is_none() {
                            *text_config = TextConfig::default();
                        }
                    }
                    Element::TextConfigClosed => {
                        nesting_level -= 1;
                    },
                    Element::UseOpened => {
                        nesting_level += 1;

                        if skip.is_none() {
                            recursive_commands.clear();
                            recursive_call_stack.clear();
                            collect_declarations = true;
                        }
                        
                    }
                    Element::UseClosed(src) => {
                        nesting_level -= 1;

                        if skip.is_none() {
                            collect_declarations = false;
                            //println!("try to use: {:?}", recursive_source);
                            if let Some(reusable) = reusables.get(src){
                                //println!("use: {:?}", recursive_source);
                                for command in reusable.iter() {
                                    recursive_commands.push(command);
                                }
                                if recursive_call_stack.len() > 0 {
                                    (events, pointer) = set_layout(
                                        api,
                                        &recursive_commands,
                                        reusables,
                                        Some(&recursive_call_stack), 
                                        None,
                                        Some(&mut config),
                                        Some(&mut text_config),
                                        user_app,
                                        events,
                                        pointer
                                    );
                                }
                                else {
                                    (events, pointer) = set_layout(
                                        api,
                                        &recursive_commands,
                                        reusables,
                                        None,
                                        None,
                                        Some(&mut config),
                                        Some(&mut text_config),
                                        user_app,
                                        events,
                                        pointer
                                    );
                                }
                            }
                            
                        }
                    }
                    Element::TreeViewOpened => {
                        nesting_level += 1;

                        if skip.is_none() {
                            recursive_commands.clear();
                            recursive_call_stack.clear();
                            collect_declarations = true;
                        }
                    }
                    Element::TreeViewClosed(src) => {
                        nesting_level -= 1;

                        if skip.is_none() {
                            collect_declarations = false;
                            events = ui_toolkit::treeview::treeview(src, &list_data, api, user_app, events);
                        }
                    }
                    Element::TextBoxOpened => {
                        nesting_level += 1;

                        if skip.is_none() {
                            recursive_commands.clear();
                            recursive_call_stack.clear();
                            collect_declarations = true;
                            // text_box_source = String::resolve_src(name, locals, user_app, &list_data);
                            // api.ui_layout.open_element();
                            if api.ui_layout.hovered() {
                                pointer = winit::window::CursorIcon::Text;
                            }
                            api.ui_layout.configure_element(&ElementConfiguration::default());
                        }
                    }
                    Element::TextBoxClosed(_src) => {
                        nesting_level -= 1;

                        if skip.is_none() {
                            collect_declarations = false;
                            // events = ui_toolkit::textbox::text_box(
                            //     text_box_source, 
                            //     &list_data,
                            //     api, 
                            //     user_app, 
                            //     events);
                            api.ui_layout.close_element();
                        }
                    }
                    _ => {todo!("")}
                }
            }
            Layout::Declaration { name, value } => {
                if collect_declarations {
                    recursive_call_stack.insert(*name, value);
                }
            }
            Layout::Config(config_command) => {
                if skip.is_none() {
                    execute_config(
                        config_command,
                        Some(&mut config),
                        Some(&mut text_config),
                        reusables,
                        locals,
                        &list_data,
                        api,
                        user_app
                    );
                }
            }
        }
    }

    (events, pointer)

}

fn execute_config<'render_pass, Event, UserApp>(
    config_command: &Config,
    config: Option<&mut ElementConfiguration>,
    text_config: Option<&mut TextConfig>,
    reusables: &HashMap<GlobalSymbol, Vec<Layout<Event>>>,
    locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration<Event>>>>,
    list_data: &Option<(GlobalSymbol, usize)>,
    api: &mut API,
    user_app: &UserApp,
)
where
    Event: FromStr+Clone+PartialEq+Debug+Default+EventHandler<UserApplication = UserApp>,
    <Event as FromStr>::Err: Debug,
    UserApp: ParserDataAccess<Event>
{


    let mut config = match config {
        None => &mut ElementConfiguration::default(),
        Some(config) => config
    };

    let mut text_config = match text_config {
        None => &mut TextConfig::default(),
        Some(c) => c
    };

    match config_command {
        Config::Id(id) => {
            if let DataSrc::Static(id) = id {
                config.id(id.as_str());
            }
        }//config.id(DEFAULT_TEXT).parse(),
        Config::FitX  => config.x_fit().parse(),
        Config::FitXmin(min)  => config.x_fit_min(f32::resolve_src(min, locals, user_app, list_data)).parse(),
        Config::FitXmax(max)  => config.x_fit_min_max(0.0, f32::resolve_src(max, locals, user_app, list_data)).parse(),
        Config::FitXminmax{min, max}  => config.x_fit_min_max(
            f32::resolve_src(min, locals, user_app, list_data),
            f32::resolve_src(max, locals, user_app, list_data)
        ).parse(),
        Config::FitY  => config.y_fit().parse(),
        Config::FitYmin(min)  => config.y_fit_min(f32::resolve_src(min, locals, user_app, list_data)).parse(),
        Config::FitYmax(max)  => config.y_fit_min_max(0.0, f32::resolve_src(max, locals, user_app, list_data)).parse(),
        Config::FitYminmax{min, max}  => config.y_fit_min_max(
            f32::resolve_src(min, locals, user_app, list_data),
            f32::resolve_src(max, locals, user_app, list_data)
        ).parse(),
        Config::GrowX  => config.x_grow().parse(),
        Config::GrowXmin(min) => config.x_grow_min(f32::resolve_src(min, locals, user_app, list_data)).parse(),
        Config::GrowXmax(max) => config.x_grow_min_max(0.0, f32::resolve_src(max, locals, user_app, list_data)).parse(),
        Config::GrowXminmax{min, max}  => config.x_grow_min_max(
            f32::resolve_src(min, locals, user_app, list_data),
            f32::resolve_src(max, locals, user_app, list_data)
        ).parse(),
        Config::GrowY  => config.y_grow().parse(),
        Config::GrowYmin(min) => config.y_grow_min(f32::resolve_src(min, locals, user_app, list_data)).parse(),
        Config::GrowYmax(max) => config.y_grow_min_max(0.0, f32::resolve_src(max, locals, user_app, list_data)).parse(),
        Config::GrowYminmax{min, max}  => config.y_grow_min_max(
            f32::resolve_src(min, locals, user_app, list_data),
            f32::resolve_src(max, locals, user_app, list_data)
        ).parse(),
        Config::FixedX(size) => config.x_fixed(f32::resolve_src(size, locals, user_app, list_data)).parse(),
        Config::FixedY(size) => config.y_fixed(f32::resolve_src(size, locals, user_app, list_data)).parse(),
        Config::PercentX(size) => config.x_percent(f32::resolve_src(size, locals, user_app, list_data)).parse(),
        Config::PercentY(size) => config.y_percent(f32::resolve_src(size, locals, user_app, list_data)).parse(),
        Config::GrowAll  => config.grow_all().parse(),
        Config::PaddingAll(padding)  => config.padding_all(u16::resolve_src(padding, locals, user_app, list_data)).parse(),
        Config::PaddingTop(padding)  => config.padding_top(u16::resolve_src(padding, locals, user_app, list_data)).parse(),
        Config::PaddingBottom(padding)  => config.padding_bottom(u16::resolve_src(padding, locals, user_app, list_data)).parse(),
        Config::PaddingLeft(padding)  => config.padding_left(u16::resolve_src(padding, locals, user_app, list_data)).parse(),
        Config::PaddingRight(padding)  => config.padding_right(u16::resolve_src(padding, locals, user_app, list_data)).parse(),
        Config::Vertical  => config.direction(true).parse(),
        Config::ChildGap(gap)  => config.child_gap(u16::resolve_src(gap, locals, user_app, list_data)).parse(),
        Config::ChildAlignmentXLeft  => config.align_children_x_left().parse(),
        Config::ChildAlignmentXRight  => config.align_children_x_right().parse(),
        Config::ChildAlignmentXCenter  => config.align_children_x_center().parse(),
        Config::ChildAlignmentYTop  => config.align_children_y_top().parse(),
        Config::ChildAlignmentYCenter  => config.align_children_y_center().parse(),
        Config::ChildAlignmentYBottom  => config.align_children_y_bottom().parse(),
        Config::Color(color)  => {
            let color = Color::resolve_src(color, locals, user_app, list_data);
            //println!("{:?}", color);
            config.color(color).parse();
        }
        Config::RadiusAll(radius)  => config.radius_all(f32::resolve_src(radius, locals, user_app, list_data)).parse(),
        Config::RadiusTopLeft(radius)  => config.radius_top_left(f32::resolve_src(radius, locals, user_app, list_data)).parse(),
        Config::RadiusTopRight(radius)  => config.radius_top_right(f32::resolve_src(radius, locals, user_app, list_data)).parse(),
        Config::RadiusBottomRight(radius)  => config.radius_bottom_right(f32::resolve_src(radius, locals, user_app, list_data)).parse(),
        Config::RadiusBottomLeft(radius)  => config.radius_bottom_left(f32::resolve_src(radius, locals, user_app, list_data)).parse(),
        Config::BorderColor(color) => config.border_color(Color::resolve_src(color, locals, user_app, list_data)).parse(),
        Config::BorderAll(border)  => config.border_all(u16::resolve_src(border, locals, user_app, list_data)).parse(),
        Config::BorderTop(border)  => config.border_top(u16::resolve_src(border, locals, user_app, list_data)).parse(),
        Config::BorderBottom(border)  => config.border_bottom(u16::resolve_src(border, locals, user_app, list_data)).parse(),
        Config::BorderLeft(border)  => config.border_left(u16::resolve_src(border, locals, user_app, list_data)).parse(),
        Config::BorderRight(border)  => config.border_right(u16::resolve_src(border, locals, user_app, list_data)).parse(),
        Config::BorderBetweenChildren(border)  => config.border_between_children(u16::resolve_src(border, locals, user_app, list_data)).parse(),
        Config::Clip { vertical, horizontal } => config.scroll(
            bool::resolve_src(vertical, locals, user_app, list_data), 
            bool::resolve_src(horizontal, locals, user_app, list_data), 
            api.ui_layout.get_scroll_offset()
        ).parse(),
        Config::Image { name } => {
            if let Some(image) = UIImageDescriptor::resolve_name(name, locals, user_app, list_data){
                config.image(image).parse();
            }
        }
        Config::Floating => config.floating().parse(),
        Config::FloatingOffset { x, y } => config.floating_offset(
            f32::resolve_src(x, locals, user_app, list_data), 
            f32::resolve_src(y, locals, user_app, list_data)
        ).parse(),
        Config::FloatingDimensions { width, height } => config.floating_dimensions(
            f32::resolve_src(width, locals, user_app, list_data),  
            f32::resolve_src(height, locals, user_app, list_data), 
        ).parse(),
        Config::FloatingZIndex { z } => config.floating_z_index(i16::resolve_src(z, locals, user_app, list_data)).parse(),
        Config::FloatingAttatchToParentAtTopLeft => config.floating_attach_to_parent_at_top_left().parse(),
        Config::FloatingAttatchToParentAtCenterLeft => config.floating_attach_to_parent_at_center_left().parse(),
        Config::FloatingAttatchToParentAtBottomLeft => config.floating_attach_to_parent_at_bottom_left().parse(),
        Config::FloatingAttatchToParentAtTopCenter => config.floating_attach_to_parent_at_top_center().parse(),
        Config::FloatingAttatchToParentAtCenter => config.floating_attach_to_parent_at_center().parse(),
        Config::FloatingAttatchToParentAtBottomCenter => config.floating_attach_to_parent_at_bottom_center().parse(),
        Config::FloatingAttatchToParentAtTopRight => config.floating_attach_to_parent_at_top_right().parse(),
        Config::FloatingAttatchToParentAtCenterRight => config.floating_attach_to_parent_at_center_right().parse(),
        Config::FloatingAttatchToParentAtBottomRight => config.floating_attach_to_parent_at_bottom_right().parse(),
        Config::FloatingAttatchElementAtTopLeft => config.floating_attach_element_at_top_left().parse(),
        Config::FloatingAttatchElementAtCenterLeft => config.floating_attach_element_at_center_left().parse(),
        Config::FloatingAttatchElementAtBottomLeft => config.floating_attach_element_at_bottom_left().parse(),
        Config::FloatingAttatchElementAtTopCenter => config.floating_attach_element_at_top_center().parse(),
        Config::FloatingAttatchElementAtCenter => config.floating_attach_element_at_center().parse(),
        Config::FloatingAttatchElementAtBottomCenter => config.floating_attach_element_at_bottom_center().parse(),
        Config::FloatingAttatchElementAtTopRight => config.floating_attach_element_at_top_right().parse(),
        Config::FloatingAttatchElementAtCenterRight => config.floating_attach_element_at_center_right().parse(),
        Config::FloatingAttatchElementAtBottomRight => config.floating_attach_element_at_bottom_right().parse(),
        Config::FloatingPointerPassThrough => config.floating_pointer_pass_through().parse(),
        Config::FloatingAttachElementToElement { other_element_id:_ } => {
            //let id = layout_engine.get_id(other_element_id);
            config.floating_attach_to_element(0).parse()
        }
        Config::FloatingAttachElementToRoot => config.floating_attach_to_root().parse(),
        Config::Use { name } => {
            if let Some(reusable) = reusables.get(name) {
                for config_command in reusable {
                    if let Layout::Config(config_command) = config_command {
                        execute_config(
                            config_command, 
                            Some(&mut config), 
                            Some(&mut text_config),
                            reusables, 
                            locals, 
                            list_data, 
                            api, 
                            user_app
                        );
                    }
                }
            }
        }

        Config::AlignCenter => text_config.alignment_center().parse(),
        Config::AlignLeft => text_config.alignment_left().parse(),
        Config::AlignRight => text_config.alignment_right().parse(),
        Config::Editable(_state) => (),
        Config::FontId(id) => text_config.font_id(u16::resolve_src(id, locals, user_app, list_data)).parse(),
        Config::FontColor(color)  => text_config.color(Color::resolve_src(color, locals, user_app, list_data)).parse(),
        Config::FontSize(size) => text_config.font_size(u16::resolve_src(size, locals, user_app, list_data)).parse(),
        Config::LineHeight(height) => text_config.line_height(u16::resolve_src(height, locals, user_app, list_data)).parse(),
    }
}

trait ResolveValue<'frame,'application, Event,UserApp> 
where
    'application: 'frame,
    Event: FromStr+Clone+PartialEq+Default+Debug+EventHandler<UserApplication = UserApp>,
    <Event as FromStr>::Err: Debug,
    UserApp: ParserDataAccess<Event>

{
    type DeclarationType;
    type ReturnType;
    fn resolve_src (
        var: &'frame DataSrc<Self::DeclarationType>,
        locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration<Event>>>>, 
        user_app: &'application UserApp, 
        list_data: &Option<(GlobalSymbol, usize)>
    ) -> Self::ReturnType;
    fn resolve_name (
        var: &GlobalSymbol,
        locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration<Event>>>>, 
        user_app: &'application UserApp, 
        list_data: &Option<(GlobalSymbol, usize)>
    ) -> Self::ReturnType;
}

impl<'frame, 'application, Event,UserApp> ResolveValue<'frame,'application, Event,UserApp> for UIImageDescriptor
where
    'application: 'frame,
    Event: FromStr+Clone+PartialEq+Default+Debug+EventHandler<UserApplication = UserApp>,
    <Event as FromStr>::Err: Debug,
    UserApp: ParserDataAccess<Event>
{
    type DeclarationType = Option<&'frame UIImageDescriptor>;
    type ReturnType = Option<&'frame UIImageDescriptor>;
    fn resolve_name (
            name: &GlobalSymbol,
            locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration<Event>>>>, 
            user_app: &'application UserApp, 
            list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        if let Some(locals) = locals
        && let Some(local) = locals.get(name)
        && let DataSrc::Dynamic(local) = local
        && let Some(value) = user_app.get_image(local, &list_data) {
            Some(value)
        }
        else if let Some(value) = user_app.get_image(name, &list_data) {
            Some(value)
        }
        else {
            None
        }
    }
    fn resolve_src (
            _var: &'frame DataSrc<Self::DeclarationType>,
            _locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration<Event>>>>, 
            _user_app: &'application UserApp, 
            _list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        None
    }
}

impl<'frame, 'application, Event,UserApp> ResolveValue<'frame,'application, Event,UserApp> for Color
where
    'application: 'frame,
    Event: FromStr+Clone+PartialEq+Default+Debug+EventHandler<UserApplication = UserApp>,
    <Event as FromStr>::Err: Debug,
    UserApp: ParserDataAccess<Event>
{
    type DeclarationType = Color;
    type ReturnType = Color;
    fn resolve_name (
            name: &GlobalSymbol,
            locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration<Event>>>>, 
            user_app: &'application UserApp, 
            list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        if let Some(locals) = locals
        && let Some(local) = locals.get(name)
        && let DataSrc::Dynamic(local) = local
        && let Some(value) = user_app.get_color(&local, &list_data) {
            value.clone()
        }
        else if let Some(locals) = locals
        && let Some(local) = locals.get(name)
        && let DataSrc::Static(local) = local
        && let Declaration::Color(value) = local {
            value.clone()
        }
        else if let Some(value) = user_app.get_color(&name, &list_data) {
            value.clone()
        }
        else {
            Color::default()
        }
    }
    fn resolve_src (
            var: &'frame DataSrc<Self::DeclarationType>,
            locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration<Event>>>>, 
            user_app: &'application UserApp, 
            list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        match var {
            DataSrc::Dynamic(name) => {
                if let Some(locals) = locals
                && let Some(local) = locals.get(name)
                && let DataSrc::Dynamic(local) = local
                && let Some(value) = user_app.get_color(&local, &list_data) {
                    value.clone()
                }
                else if let Some(locals) = locals
                && let Some(local) = locals.get(name)
                && let DataSrc::Static(local) = local
                && let Declaration::Color(value) = local {
                    value.clone()
                }
                else if let Some(value) = user_app.get_color(&name, &list_data) {
                    value.clone()
                }
                else {
                    Color::default()
                }
            }
            DataSrc::Static(value) => {
                value.clone()
            }
        }
    }
}

impl<'frame, 'application, Event,UserApp> ResolveValue<'frame,'application, Event,UserApp> for String
where
    'application: 'frame,
    Event: FromStr+Clone+PartialEq+Default+Debug+EventHandler<UserApplication = UserApp>,
    <Event as FromStr>::Err: Debug,
    UserApp: ParserDataAccess<Event>
{
    type DeclarationType = String;
    type ReturnType = &'frame str;
    fn resolve_name (
            name: &GlobalSymbol,
            locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration<Event>>>>, 
            user_app: &'application UserApp, 
            list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        if let Some(locals) = locals
        && let Some(local) = locals.get(name)
        && let DataSrc::Dynamic(local) = local
        && let Some(value) = user_app.get_text(&local, &list_data) {
            value
        }
        else if let Some(locals) = locals
        && let Some(local) = locals.get(name)
        && let DataSrc::Static(local) = local
        && let Declaration::Text(value) = local {
            value
        }
        else if let Some(value) = user_app.get_text(&name, &list_data) {
            value
        }
        else {
            DEFAULT_TEXT
        }
    }
    fn resolve_src (
            var: &'frame DataSrc<Self::DeclarationType>,
            locals: Option<&HashMap<GlobalSymbol, &'frame DataSrc<Declaration<Event>>>>, 
            user_app: &'application UserApp, 
            list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        match var {
            DataSrc::Dynamic(name) => {
                if let Some(locals) = locals
                && let Some(local) = locals.get(name)
                && let DataSrc::Dynamic(local) = local
                && let Some(value) = user_app.get_text(&local, &list_data) {
                    value
                }
                else if let Some(locals) = locals
                && let Some(local) = locals.get(name)
                && let DataSrc::Static(local) = local
                && let Declaration::Text(value) = local {
                    value
                }
                else if let Some(value) = user_app.get_text(&name, &list_data) {
                    value
                }
                else {
                    DEFAULT_TEXT
                }
            }
            DataSrc::Static(value) => {
                value
            }
        }
    }
}

impl<'frame, 'application, Event,UserApp> ResolveValue<'frame, 'application, Event,UserApp> for f32
where
    'application: 'frame,
    Event: FromStr+Clone+PartialEq+Default+Debug+EventHandler<UserApplication = UserApp>,
    <Event as FromStr>::Err: Debug,
    UserApp: ParserDataAccess<Event>
{
    type DeclarationType = f32;
    type ReturnType = f32;
    fn resolve_src (
            var: &DataSrc<Self::DeclarationType>,
            locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration<Event>>>>, 
            user_app: &UserApp, 
            list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        match var {
            DataSrc::Dynamic(name) => {
                if let Some(locals) = locals
                && let Some(local) = locals.get(name)
                && let DataSrc::Dynamic(local) = local
                && let Some(value) = user_app.get_numeric(&local, &list_data) {
                    value
                }
                else if let Some(locals) = locals
                && let Some(local) = locals.get(name)
                && let DataSrc::Static(local) = local
                && let Declaration::Numeric(value) = local {
                    *value
                }
                else if let Some(value) = user_app.get_numeric(&name, &list_data) {
                    value
                }
                else {
                    0.0
                }
            }
            DataSrc::Static(value) => {
                *value
            }
        }
    }
    fn resolve_name (
            name: &GlobalSymbol,
            locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration<Event>>>>, 
            user_app: &UserApp, 
            list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        if let Some(locals) = locals
        && let Some(local) = locals.get(name)
        && let DataSrc::Dynamic(local) = local
        && let Some(value) = user_app.get_numeric(&local, &list_data) {
            value
        }
        else if let Some(locals) = locals
        && let Some(local) = locals.get(name)
        && let DataSrc::Static(local) = local
        && let Declaration::Numeric(value) = local {
            *value
        }
        else if let Some(value) = user_app.get_numeric(&name, &list_data) {
            value
        }
        else {
            0.0
        }
    }
}

impl<'frame, 'application, Event,UserApp> ResolveValue<'frame, 'application, Event,UserApp> for u16
where
    'application: 'frame,
    Event: FromStr+Clone+PartialEq+Default+Debug+EventHandler<UserApplication = UserApp>,
    <Event as FromStr>::Err: Debug,
    UserApp: ParserDataAccess<Event>
{
    type DeclarationType = u16;
    type ReturnType = u16;
    fn resolve_src (
            var: &DataSrc<Self::DeclarationType>,
            locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration<Event>>>>, 
            user_app: &UserApp, 
            list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        match var {
            DataSrc::Dynamic(name) => {
                if let Some(locals) = locals
                && let Some(local) = locals.get(name)
                && let DataSrc::Dynamic(local) = local
                && let Some(value) = user_app.get_numeric(&local, &list_data) {
                    value as u16
                }
                else if let Some(locals) = locals
                && let Some(local) = locals.get(name)
                && let DataSrc::Static(local) = local
                && let Declaration::Numeric(value) = local {
                    *value as u16
                }
                else if let Some(value) = user_app.get_numeric(&name, &list_data) {
                    value as u16
                }
                else {
                    0
                }
            }
            DataSrc::Static(value) => {
                *value as u16
            }
        }
    }
    fn resolve_name (
            name: &GlobalSymbol,
            locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration<Event>>>>, 
            user_app: &UserApp, 
            list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        if let Some(locals) = locals
        && let Some(local) = locals.get(name)
        && let DataSrc::Dynamic(local) = local
        && let Some(value) = user_app.get_numeric(&local, &list_data) {
            value as u16
        }
        else if let Some(locals) = locals
        && let Some(local) = locals.get(name)
        && let DataSrc::Static(local) = local
        && let Declaration::Numeric(value) = local {
            *value as u16
        }
        else if let Some(value) = user_app.get_numeric(&name, &list_data) {
            value as u16
        }
        else {
            0
        }
    }
}

impl<'frame, 'application, Event,UserApp> ResolveValue<'frame, 'application, Event,UserApp> for i16
where
    'application: 'frame,
    Event: FromStr+Clone+PartialEq+Default+Debug+EventHandler<UserApplication = UserApp>,
    <Event as FromStr>::Err: Debug,
    UserApp: ParserDataAccess<Event>
{
    type DeclarationType = i16;
    type ReturnType = i16;
    fn resolve_src (
            var: &DataSrc<Self::DeclarationType>,
            locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration<Event>>>>, 
            user_app: &UserApp, 
            list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        match var {
            DataSrc::Dynamic(name) => {
                if let Some(locals) = locals
                && let Some(local) = locals.get(name)
                && let DataSrc::Dynamic(local) = local
                && let Some(value) = user_app.get_numeric(&local, &list_data) {
                    value as i16
                }
                else if let Some(locals) = locals
                && let Some(local) = locals.get(name)
                && let DataSrc::Static(local) = local
                && let Declaration::Numeric(value) = local {
                    *value as i16
                }
                else if let Some(value) = user_app.get_numeric(&name, &list_data) {
                    value as i16
                }
                else {
                    0
                }
            }
            DataSrc::Static(value) => {
                *value as i16
            }
        }
    }
    fn resolve_name (
            name: &GlobalSymbol,
            locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration<Event>>>>, 
            user_app: &UserApp, 
            list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        if let Some(locals) = locals
        && let Some(local) = locals.get(name)
        && let DataSrc::Dynamic(local) = local
        && let Some(value) = user_app.get_numeric(&local, &list_data) {
            value as i16
        }
        else if let Some(locals) = locals
        && let Some(local) = locals.get(name)
        && let DataSrc::Static(local) = local
        && let Declaration::Numeric(value) = local {
            *value as i16
        }
        else if let Some(value) = user_app.get_numeric(&name, &list_data) {
            value as i16
        }
        else {
            0
        }
    }
}

impl<'frame, 'application, Event,UserApp> ResolveValue<'frame, 'application, Event,UserApp> for bool
where
    'application: 'frame,
    Event: FromStr+Clone+PartialEq+Default+Debug+EventHandler<UserApplication = UserApp>,
    <Event as FromStr>::Err: Debug,
    UserApp: ParserDataAccess<Event>
{
    type DeclarationType = bool;
    type ReturnType = bool;
    fn resolve_src (
            var: &DataSrc<Self::DeclarationType>,
            locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration<Event>>>>, 
            user_app: &UserApp, 
            list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        match var {
            DataSrc::Dynamic(name) => {
                if let Some(locals) = locals
                && let Some(local) = locals.get(name)
                && let DataSrc::Dynamic(local) = local
                && let Some(value) = user_app.get_bool(&local, &list_data) {
                    value
                }
                else if let Some(locals) = locals
                && let Some(local) = locals.get(name)
                && let DataSrc::Static(local) = local
                && let Declaration::Bool(value) = local {
                    *value
                }
                else if let Some(value) = user_app.get_bool(&name, &list_data) {
                    value
                }
                else {
                    false
                }
            }
            DataSrc::Static(value) => {
                *value
            }
        }
    }
    fn resolve_name (
            name: &GlobalSymbol,
            locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration<Event>>>>, 
            user_app: &UserApp, 
            list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        if let Some(locals) = locals
        && let Some(local) = locals.get(name)
        && let DataSrc::Dynamic(local) = local
        && let Some(value) = user_app.get_bool(&local, &list_data) {
            value
        }
        else if let Some(locals) = locals
        && let Some(local) = locals.get(name)
        && let DataSrc::Static(local) = local
        && let Declaration::Bool(value) = local {
            *value
        }
        else if let Some(value) = user_app.get_bool(&name, &list_data) {
            value
        }
        else {
            false
        }
    }
}

impl<'frame, 'application, Event,UserApp> ResolveValue<'frame, 'application, Event,UserApp> for Event
where
    'application:'frame,
    Event: FromStr+Clone+PartialEq+Default+Debug+EventHandler<UserApplication = UserApp>,
    <Event as FromStr>::Err: Debug,
    UserApp: ParserDataAccess<Event>
{
    type DeclarationType = Event;
    type ReturnType = Event;
    fn resolve_src (
            var: &DataSrc<Self::DeclarationType>,
            locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration<Event>>>>, 
            user_app: &UserApp, 
            list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        match var {
            DataSrc::Dynamic(name) => {
                if let Some(locals) = locals
                && let Some(local) = locals.get(name)
                && let DataSrc::Dynamic(local) = local
                && let Some(value) = user_app.get_event(&local, &list_data) {
                    value
                }
                else if let Some(locals) = locals
                && let Some(local) = locals.get(name)
                && let DataSrc::Static(local) = local
                && let Declaration::Event(value) = local {
                    value.clone()
                }
                else if let Some(value) = user_app.get_event(&name, &list_data) {
                    value
                }
                else {
                    Event::default()
                }
            }
            DataSrc::Static(value) => {
                value.clone()
            }
        }
    }
    fn resolve_name (
            name: &GlobalSymbol,
            locals: Option<&HashMap<GlobalSymbol, &DataSrc<Declaration<Event>>>>, 
            user_app: &UserApp, 
            list_data: &Option<(GlobalSymbol, usize)>
        ) -> Self::ReturnType {
        if let Some(locals) = locals
        && let Some(local) = locals.get(name)
        && let DataSrc::Dynamic(local) = local
        && let Some(value) = user_app.get_event(&local, &list_data) {
            value
        }
        else if let Some(locals) = locals
        && let Some(local) = locals.get(name)
        && let DataSrc::Static(local) = local
        && let Declaration::Event(value) = local {
            value.clone()
        }
        else if let Some(value) = user_app.get_event(&name, &list_data) {
            value
        }
        else {
            Event::default()
        }
    }
}