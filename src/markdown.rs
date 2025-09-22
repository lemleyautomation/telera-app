use std::{collections::HashMap, fmt::Debug, str::FromStr};

use markdown::mdast::{List, Node, Paragraph};
use crate::{Config, DataSrc, Declaration, Element, Layout};
use telera_layout::Color;

#[derive(Debug)]
enum ParsingMode {
    None,
    Body,
    ReusableElements,
    ReusableConfig,
}

pub fn process_layout<Event: Clone+Debug+PartialEq+FromStr>(file: String) -> Result<(String, Vec<Layout<Event>>, HashMap::<String, Vec<Layout<Event>>>), String> 
where <Event as FromStr>::Err: Debug
{
    let mut parsing_mode = ParsingMode::None;
    let mut page_name = "".to_string();
    let mut body = Vec::<Layout<Event>>::new();
    let mut open_reuseable_name = "".to_string();
    let mut _open_variable_name = "".to_string();
    let mut reusables = HashMap::<String, Vec<Layout<Event>>>::new();

    if let Ok(m) = markdown::to_mdast(&file, &markdown::ParseOptions::default())
    && let Some(nodes) = m.children() {

        for node in nodes {
            match node {
                Node::Heading(h) => {
                    if let Some(declaration) = h.children.get(0)
                    && let Node::Text(declaration) = declaration {
                        match h.depth {
                            1 => {
                                parsing_mode = ParsingMode::Body;
                                page_name = declaration.value.trim().to_string();
                            }
                            2 => {
                                parsing_mode = ParsingMode::ReusableConfig;
                                open_reuseable_name = declaration.value.trim().to_string();
                            },
                            3 => {
                                parsing_mode = ParsingMode::ReusableElements;
                                open_reuseable_name = declaration.value.trim().to_string();
                            }
                            _ => parsing_mode = ParsingMode::None,
                        }
                    }
                }
                Node::List(list) => {
                    match parsing_mode {
                        ParsingMode::ReusableConfig => {
                            let mut reusable_items = process_configs(list);
                            let mut formatted_reusable_items = Vec::<Layout<Event>>::new();
                            formatted_reusable_items.append(&mut reusable_items);
                            reusables.insert(open_reuseable_name.clone(), formatted_reusable_items);
                        }
                        ParsingMode::ReusableElements => {
                            for node in &list.children{
                                let element = process_element(node);
                                reusables.insert(open_reuseable_name.clone(), element);
                            }
                            
                        }
                        ParsingMode::Body => {
                            body.push(Layout::Element(Element::Pointer(winit::window::CursorIcon::Default)));
                            for node in &list.children {
                                let mut element = process_element(node);
                                body.append(&mut element);
                            }
                        }
                        ParsingMode::None => {}
                    }
                }
                _ => {}
            }
        }
        Ok((page_name, body, reusables))
    }
    else {
        Err(":(".to_string())
    }
}

fn process_element<Event: Clone+Debug+PartialEq+FromStr>(element: &Node) -> Vec<Layout<Event>>
where <Event as FromStr>::Err: Debug
{
    let mut layout_commands: Vec<Layout<Event>> = Vec::new();

    if let Node::ListItem(element) = element
    && let Some(element_declaration) = element.children.get(0)
    && let Node::Paragraph(element_declaration) = element_declaration
    && let Some(element_type) = element_declaration.children.get(0)
    && let Node::InlineCode(element_type) = element_type {
        match element_type.value.as_str() {
            "declarations" => {
                if let Some(declarations) = element.children.get(1)
                && let Node::List(declarations) = declarations {
                    for declaration in declarations.children.iter() {
                        if let Some((name, value)) = process_variable::<Event>(&declaration) {
                            layout_commands.push(Layout::Declaration { name, value });
                        }
                    }
                }
            }
            "element" => {
                layout_commands.push(Layout::Element(Element::ElementOpened { id: None }));
                layout_commands.push(Layout::Element(Element::ConfigOpened));
                if let Some(config) = element.children.get(1)
                && let Node::List(configs) = config
                && let Some(configs) = configs.children.get(0)
                && let Node::ListItem(configs) = configs
                && let Some(configs) = configs.children.get(1)
                && let Node::List(config_commands) = configs {
                    let mut layout_config_commands = process_configs(&config_commands);
                    layout_commands.append(&mut layout_config_commands);
                }
                layout_commands.push(Layout::Element(Element::ConfigClosed));

                if let Some(child_elements) = element.children.get(1)
                && let Node::List(child_elements) = child_elements {
                    for child_element in child_elements.children.iter().skip(1) {
                        let mut child_element = process_element(child_element);
                        layout_commands.append(&mut child_element);
                    }
                }

                layout_commands.push(Layout::Element(Element::ElementClosed));
            }
            "grow" => {
                layout_commands.push(Layout::Element(Element::ElementOpened { id: None }));
                layout_commands.push(Layout::Element(Element::ConfigOpened));
                layout_commands.push(Layout::Config(Config::GrowAll));
                layout_commands.push(Layout::Element(Element::ConfigClosed));
                layout_commands.push(Layout::Element(Element::ElementClosed));
            }
            "text" => {
                layout_commands.push(Layout::Element(Element::TextElementOpened));

                layout_commands.push(Layout::Element(Element::TextConfigOpened));
                if let Some(config) = element.children.get(1)
                && let Node::List(config) = config
                && let Some(config) = config.children.get(0)
                && let Node::ListItem(config) = config
                && let Some(configs) = config.children.get(1)
                && let Node::List(configs) = configs {
                    let mut configs = process_configs(configs);
                    layout_commands.append(&mut configs);
                }
                layout_commands.push(Layout::Element(Element::TextConfigClosed));

                if let Some(text) = element.children.get(1)
                && let Node::List(text) = text
                && let Some(text) = text.children.get(1)
                && let Node::ListItem(text) = text
                && let Some(text) = text.children.get(0)
                && let Node::Paragraph(text) = text
                && let Some(text) = text.children.get(0) {
                    match text {
                        Node::Emphasis(dynamic_text) => {
                            if let Some(dynamic_text) = dynamic_text.children.get(0)
                            && let Node::Text(dynamic_text) = dynamic_text {
                                layout_commands.push(Layout::Element(Element::TextElementClosed(
                                    DataSrc::Dynamic(dynamic_text.value.trim().to_string())
                                )));
                            }
                        }
                        Node::Text(static_text) => {
                            layout_commands.push(Layout::Element(Element::TextElementClosed(
                                DataSrc::Static(static_text.value.trim().to_string())
                            )));
                        }
                        _ => {}
                    }
                }
            }
            "use" => {
                //println!("{:#?}", element);
                if let Some(reusable_name) = element_declaration.children.get(1)
                && let Node::Text(reusable_name) = reusable_name
                && let Some(input_variables) = element.children.get(1)
                && let Node::List(input_variables) = input_variables {
                    layout_commands.push(Layout::Element(Element::UseOpened { 
                        name: reusable_name.value.trim().to_string()
                    }));
                    for input_variable in &input_variables.children {
                        if let Some((name, declaration)) = process_variable(input_variable) {
                            layout_commands.push(Layout::Declaration { name, value: declaration });
                        }
                    }
                    layout_commands.push(Layout::Element(Element::UseClosed));
                }
                
            }
            "list" => {
                if let Some(list_src) = element_declaration.children.get(1)
                && let Node::Text(list_src) = list_src
                && let Some(list_content) = element.children.get(1)
                && let Node::List(list_content) = list_content {

                    let mut formatted_list = Vec::<Layout<Event>>::new();
                    formatted_list.push(Layout::Element(Element::ListOpened { src: list_src.value.trim().to_string() }));

                    if let Some(declarations) = list_content.children.get(0)
                    && let Node::ListItem(declarations) = declarations
                    && let Some(declarations) = declarations.children.get(1)
                    && let Node::List(declarations) = declarations {
                        for declaration in &declarations.children {
                            if let Some((name, declaration)) = process_variable(declaration) {
                            layout_commands.push(Layout::Declaration { name, value: declaration });
                        }
                        }
                    }

                    for li in list_content.children.iter().skip(1) {
                        let mut list_item = process_element::<Event>(&li);
                        formatted_list.append(&mut list_item);
                    }

                    formatted_list.push(Layout::Element(Element::ListClosed));

                    layout_commands.append(&mut formatted_list);
                }
            }
            "if" => {
                if let Some(conditional) = element_declaration.children.get(1)
                && let Node::Text(conditional) = conditional
                && let Some(conditional_elements) = element.children.get(1)
                && let Node::List(conditional_elements) = conditional_elements {

                    let mut formatted_element = Vec::<Layout<Event>>::new();
                    formatted_element.push(Layout::Element(Element::IfOpened { 
                        condition: conditional.value.trim().to_string() 
                    }));

                    for conditional_element in &conditional_elements.children {
                        let mut conditional_element = process_element::<Event>(&conditional_element);
                        formatted_element.append(&mut conditional_element);
                    }

                    formatted_element.push(Layout::Element(Element::IfClosed));

                    layout_commands.append(&mut formatted_element);
                }
            }
            "if-not" => {
                if let Some(conditional) = element_declaration.children.get(1)
                && let Node::Text(conditional) = conditional
                && let Some(conditional_elements) = element.children.get(1)
                && let Node::List(conditional_elements) = conditional_elements {

                    let mut formatted_element = Vec::<Layout<Event>>::new();
                    formatted_element.push(Layout::Element(Element::IfNotOpened { 
                        condition: conditional.value.trim().to_string() 
                    }));

                    for conditional_element in &conditional_elements.children {
                        let mut conditional_element = process_element::<Event>(&conditional_element);
                        formatted_element.append(&mut conditional_element);
                    }

                    formatted_element.push(Layout::Element(Element::IfClosed));

                    layout_commands.append(&mut formatted_element);
                }
            }
            "treeview" => {
                if let Some(reusable_name) = element_declaration.children.get(1)
                && let Node::Text(reusable_name) = reusable_name {
                    layout_commands.push(Layout::Element(Element::TreeViewOpened { 
                        name: reusable_name.value.trim().to_string() 
                    }));
                    layout_commands.push(Layout::Element(Element::TreeViewClosed));
                }
            }
            _ => {}
        }
    }

    layout_commands
}

enum AvailableParameters<T>{
    None,
    AStatic(T),
    ADynamic(String),
    BStatic(T),
    BDynamic(String),
    TwoStatic(T,T),
    TwoDynamic(String,String),
    AStaticBDynamic(T,String),
    ADynamicBStatic(String,T),
    SingleStatic(T),
    SingleDynamic(String),
}

fn parameter_check<T: FromStr>(parameters: &Paragraph, bound_a: &str, bound_b: &str) -> AvailableParameters<T> {
    if parameters.children.len() < 2{
        return AvailableParameters::None
    }
    //  CASE: 2 static parameters
    if let Some(bound_range_a) = parameters.children.get(2)
    && let Node::InlineCode(bound_range_a) = bound_range_a
    && (bound_range_a.value.as_str() == bound_a || bound_range_a.value.as_str() == bound_b)

    && let Some(bound_value_a) = parameters.children.get(3)
    && let Node::Text(bound_value_a) = bound_value_a
    && let Ok(bound_value_a) = T::from_str(bound_value_a.value.trim())
    
    && let Some(bound_range_b) = parameters.children.get(4)
    && let Node::InlineCode(bound_range_b) = bound_range_b
    && (bound_range_b.value.as_str() == bound_a || bound_range_b.value.as_str() == bound_b)

    && let Some(bound_value_b) = parameters.children.get(5)
    && let Node::Text(bound_value_b) = bound_value_b
    && let Ok(bound_value_b) = T::from_str(bound_value_b.value.trim())
    {
        if bound_range_a.value.as_str() == bound_a {
            AvailableParameters::TwoStatic(bound_value_a, bound_value_b)
        }
        else {
            AvailableParameters::TwoStatic(bound_value_b, bound_value_a)
        }
    }
    //  CASE: 2 dynamic parameters
    else
    if let Some(bound_range_a) = parameters.children.get(2)
    && let Node::InlineCode(bound_range_a) = bound_range_a
    && (bound_range_a.value.as_str() == bound_a || bound_range_a.value.as_str() == bound_b)

    && let Some(bound_value_a) = parameters.children.get(4)
    && let Node::Emphasis(bound_value_a) = bound_value_a
    && let Some(bound_value_a) = bound_value_a.children.get(0)
    && let Node::Text(bound_value_a) = bound_value_a
    
    && let Some(bound_range_b) = parameters.children.get(6)
    && let Node::InlineCode(bound_range_b) = bound_range_b
    && (bound_range_b.value.as_str() == bound_a || bound_range_b.value.as_str() == bound_b)

    && let Some(bound_value_b) = parameters.children.get(8)
    && let Node::Emphasis(bound_value_b) = bound_value_b
    && let Some(bound_value_b) = bound_value_b.children.get(0)
    && let Node::Text(bound_value_b) = bound_value_b
    {
        if bound_range_a.value.as_str() == bound_a {
            AvailableParameters::TwoDynamic(bound_value_a.value.trim().to_string(), bound_value_b.value.trim().to_string())
        }
        else {
            AvailableParameters::TwoDynamic(bound_value_b.value.trim().to_string(), bound_value_a.value.trim().to_string())
        }
    }
    //  CASE: parameter A dynamic, b static
    else
    if let Some(bound_range_a) = parameters.children.get(2)
    && let Node::InlineCode(bound_range_a) = bound_range_a
    && (bound_range_a.value.as_str() == bound_a || bound_range_a.value.as_str() == bound_b)

    && let Some(bound_value_a) = parameters.children.get(4)
    && let Node::Emphasis(bound_value_a) = bound_value_a
    && let Some(bound_value_a) = bound_value_a.children.get(0)
    && let Node::Text(bound_value_a) = bound_value_a
    
    && let Some(bound_range_b) = parameters.children.get(6)
    && let Node::InlineCode(bound_range_b) = bound_range_b
    && (bound_range_b.value.as_str() == bound_a || bound_range_b.value.as_str() == bound_b)

    && let Some(bound_value_b) = parameters.children.get(7)
    && let Node::Text(bound_value_b) = bound_value_b
    && let Ok(bound_value_b) = T::from_str(bound_value_b.value.trim()) {
        if bound_range_a.value.as_str() == bound_a {
            AvailableParameters::ADynamicBStatic(bound_value_a.value.trim().to_string(), bound_value_b)
        }
        else {
            AvailableParameters::AStaticBDynamic(bound_value_b, bound_value_a.value.trim().to_string())
        }
    }
    //  CASE: parameter A static, b dynamic
    else
    if let Some(bound_range_a) = parameters.children.get(2)
    && let Node::InlineCode(bound_range_a) = bound_range_a
    && (bound_range_a.value.as_str() == bound_a || bound_range_a.value.as_str() == bound_b)

    && let Some(bound_value_a) = parameters.children.get(3)
    && let Node::Text(bound_value_a) = bound_value_a
    && let Ok(bound_value_a) = T::from_str(bound_value_a.value.trim())
    
    && let Some(bound_range_b) = parameters.children.get(4)
    && let Node::InlineCode(bound_range_b) = bound_range_b
    && (bound_range_b.value.as_str() == bound_a || bound_range_b.value.as_str() == bound_b)

    && let Some(bound_value_b) = parameters.children.get(6)
    && let Node::Emphasis(bound_value_b) = bound_value_b
    && let Some(bound_value_b) = bound_value_b.children.get(0)
    && let Node::Text(bound_value_b) = bound_value_b {
        if bound_range_a.value.as_str() == bound_a {
            AvailableParameters::ADynamicBStatic(bound_value_b.value.trim().to_string(), bound_value_a)
        }
        else {
            AvailableParameters::AStaticBDynamic(bound_value_a, bound_value_b.value.trim().to_string())
        }
    }
    //  CASE: 1 static parameter
    else
    if let Some(bound_range_a) = parameters.children.get(2)
    && let Node::InlineCode(bound_range_a) = bound_range_a
    && (bound_range_a.value.as_str() == bound_a || bound_range_a.value.as_str() == bound_b)

    && let Some(bound_value_a) = parameters.children.get(3)
    && let Node::Text(bound_value_a) = bound_value_a
    && let Ok(bound_value_a) = T::from_str(bound_value_a.value.trim()) {
        if bound_range_a.value.as_str() == bound_a {
            AvailableParameters::AStatic(bound_value_a)
        }
        else {
            AvailableParameters::BStatic(bound_value_a)
        }
    }
    //  CASE: 1 dynamic parameter
    else
    if let Some(bound_range_a) = parameters.children.get(2)
    && let Node::InlineCode(bound_range_a) = bound_range_a
    && (bound_range_a.value.as_str() == bound_a || bound_range_a.value.as_str() == bound_b)

    && let Some(bound_value_a) = parameters.children.get(4)
    && let Node::Emphasis(bound_value_a) = bound_value_a
    && let Some(bound_value_a) = bound_value_a.children.get(0)
    && let Node::Text(bound_value_a) = bound_value_a {
        if bound_range_a.value.as_str() == bound_a {
            AvailableParameters::ADynamic(bound_value_a.value.trim().to_string())
        }
        else {
            AvailableParameters::BDynamic(bound_value_a.value.trim().to_string())
        }
    }
    else
    if let Some(parameter) = parameters.children.get(2)
    && let Node::Emphasis(parameter) = parameter
    && let Some(parameter) = parameter.children.get(0)
    && let Node::Text(parameter) = parameter {
        AvailableParameters::SingleDynamic(parameter.value.trim().to_string())
    }
    else
    if let Some(parameter) = parameters.children.get(1)
    && let Node::Text(parameter) = parameter
    && let Ok(parameter) = T::from_str(parameter.value.trim()) {
        AvailableParameters::SingleStatic(parameter)
    }
    //  CASE: no parameters
    else {
        AvailableParameters::None
    }
}

fn process_variable<Event: Clone+Debug+PartialEq+FromStr>(declaration: &Node) -> Option<(String, DataSrc<Declaration<Event>>)>{
    if let Node::ListItem(declaration) = declaration
    && let Some(declaration) = declaration.children.get(0)
    && let Node::Paragraph(declaration) = declaration
    && let Some(declaration_type) = declaration.children.get(0)
    && let Node::InlineCode(variable_type) = declaration_type
    && let Some(declaration_name) = declaration.children.get(2)
    && let Node::Emphasis(declaration_name) = declaration_name
    && let Some(declaration_name) = declaration_name.children.get(0)
    && let Node::Text(variable_name) = declaration_name
    && let Some(declaration_value) = declaration.children.get(3)
    && let Node::Text(variable_value) = declaration_value {
        match variable_type.value.as_str() {
            "get-bool" |
            "get-numeric" |
            "get-text" |
            "get-event" |
            "get-image" |
            "get-color" => {
                Some((
                    variable_name.value.trim().to_string(),
                    DataSrc::<Declaration<Event>>::Dynamic(variable_value.value.trim().to_string())
                ))
            }
            "set-bool" => {
                if let Ok(variable_value) = bool::from_str(&variable_value.value.trim()) {
                    Some((
                        variable_name.value.trim().to_string(),
                        DataSrc::<Declaration<Event>>::Static(
                            Declaration::Bool(variable_value)
                        )
                    ))
                }
                else {
                    None
                }
            }
            "set-numeric" => {
                if let Ok(variable_value) = f32::from_str(&variable_value.value.trim()) {
                    Some((
                        variable_name.value.trim().to_string(),
                        DataSrc::<Declaration<Event>>::Static(
                            Declaration::Numeric(variable_value)
                        )
                    ))
                }
                else {
                    None
                }
            }
            "set-text" => {
                Some((
                    variable_name.value.trim().to_string(),
                    DataSrc::<Declaration<Event>>::Static(
                        Declaration::Text(variable_value.value.trim().to_string())
                    )
                ))
            }
            "set-event" => {
                if let Ok(variable_value) = Event::from_str(variable_value.value.trim()) {
                    Some((
                        variable_name.value.trim().to_string(),
                        DataSrc::<Declaration<Event>>::Static(
                            Declaration::Event(variable_value)
                        )
                    ))
                }
                else {
                    None
                }
            }
            "set-color" => {
                if let Ok(variable_value) = Color::from_str(&variable_value.value.trim()) {
                    Some((
                        variable_name.value.trim().to_string(),
                        DataSrc::<Declaration<Event>>::Static(
                            Declaration::Color(variable_value)
                        )
                    ))
                }
                else {
                    None
                }
            }
            _ => None
        }
    }
    else {
        None
    }
}

fn process_configs<Event: Clone+Debug+PartialEq+FromStr>(configuration_set: &List) -> Vec<Layout<Event>> {
    let mut configs = Vec::new();

    for configuration_item in &configuration_set.children {
        if let Some(config_elements) = configuration_item.children()
        && let Some(config) = config_elements.get(0)
        && let Node::Paragraph(config) = config
        && let Some(config_type) = config.children.get(0)
        && let Node::InlineCode(config_type) = config_type {
            match config_type.value.as_str() {
                "grow" => configs.push(Layout::Config(Config::GrowAll)),
                "width-grow" => {
                    match parameter_check::<f32>(config, "min", "max") {
                        AvailableParameters::None => configs.push(Layout::Config(Config::GrowX)),
                        AvailableParameters::ADynamic(a) => configs.push(Layout::Config(Config::GrowXmin(DataSrc::Dynamic(a)))),
                        AvailableParameters::AStatic(a) => configs.push(Layout::Config(Config::GrowXmin(DataSrc::Static(a)))),
                        AvailableParameters::BDynamic(b) => configs.push(Layout::Config(Config::GrowXmax(DataSrc::Dynamic(b)))),
                        AvailableParameters::BStatic(b) => configs.push(Layout::Config(Config::GrowXmax(DataSrc::Static(b)))),
                        AvailableParameters::TwoStatic(min, max) => configs.push(Layout::Config(Config::GrowXminmax { 
                            min: DataSrc::Static(min), max: DataSrc::Static(max)
                        })),
                        AvailableParameters::TwoDynamic(min, max) => configs.push(Layout::Config(Config::GrowXminmax { 
                            min: DataSrc::Dynamic(min), max: DataSrc::Dynamic(max)
                        })),
                        AvailableParameters::ADynamicBStatic(min, max) => configs.push(Layout::Config(Config::GrowXminmax { 
                            min: DataSrc::Dynamic(min), max: DataSrc::Static(max)
                        })),
                        AvailableParameters::AStaticBDynamic(min, max) => configs.push(Layout::Config(Config::GrowXminmax { 
                            min: DataSrc::Static(min), max: DataSrc::Dynamic(max)
                        })),
                        _ => {}
                    }
                }
                "height-grow" => {
                    match parameter_check::<f32>(config, "min", "max") {
                        AvailableParameters::None => configs.push(Layout::Config(Config::GrowY)),
                        AvailableParameters::ADynamic(a) => configs.push(Layout::Config(Config::GrowYmin(DataSrc::Dynamic(a)))),
                        AvailableParameters::AStatic(a) => configs.push(Layout::Config(Config::GrowYmin(DataSrc::Static(a)))),
                        AvailableParameters::BDynamic(b) => configs.push(Layout::Config(Config::GrowYmax(DataSrc::Dynamic(b)))),
                        AvailableParameters::BStatic(b) => configs.push(Layout::Config(Config::GrowYmax(DataSrc::Static(b)))),
                        AvailableParameters::TwoStatic(min, max) => configs.push(Layout::Config(Config::GrowYminmax { 
                            min: DataSrc::Static(min), max: DataSrc::Static(max)
                        })),
                        AvailableParameters::TwoDynamic(min, max) => configs.push(Layout::Config(Config::GrowYminmax { 
                            min: DataSrc::Dynamic(min), max: DataSrc::Dynamic(max)
                        })),
                        AvailableParameters::ADynamicBStatic(min, max) => configs.push(Layout::Config(Config::GrowYminmax { 
                            min: DataSrc::Dynamic(min), max: DataSrc::Static(max)
                        })),
                        AvailableParameters::AStaticBDynamic(min, max) => configs.push(Layout::Config(Config::GrowYminmax { 
                            min: DataSrc::Static(min), max: DataSrc::Dynamic(max)
                        })),
                        _ => {}
                    }
                }
                "width-fit" => {
                    match parameter_check::<f32>(config, "min", "max") {
                        AvailableParameters::None => configs.push(Layout::Config(Config::FitX)),
                        AvailableParameters::ADynamic(a) => configs.push(Layout::Config(Config::FitXmin(DataSrc::Dynamic(a)))),
                        AvailableParameters::AStatic(a) => configs.push(Layout::Config(Config::FitXmin(DataSrc::Static(a)))),
                        AvailableParameters::BDynamic(b) => configs.push(Layout::Config(Config::FitXmax(DataSrc::Dynamic(b)))),
                        AvailableParameters::BStatic(b) => configs.push(Layout::Config(Config::FitXmax(DataSrc::Static(b)))),
                        AvailableParameters::TwoStatic(min, max) => configs.push(Layout::Config(Config::FitXminmax { 
                            min: DataSrc::Static(min), max: DataSrc::Static(max)
                        })),
                        AvailableParameters::TwoDynamic(min, max) => configs.push(Layout::Config(Config::FitXminmax { 
                            min: DataSrc::Dynamic(min), max: DataSrc::Dynamic(max)
                        })),
                        AvailableParameters::ADynamicBStatic(min, max) => configs.push(Layout::Config(Config::FitXminmax { 
                            min: DataSrc::Dynamic(min), max: DataSrc::Static(max)
                        })),
                        AvailableParameters::AStaticBDynamic(min, max) => configs.push(Layout::Config(Config::FitXminmax { 
                            min: DataSrc::Static(min), max: DataSrc::Dynamic(max)
                        })),
                        _ => {}
                    }
                }
                "height-fit" => {
                    match parameter_check::<f32>(config, "min", "max") {
                        AvailableParameters::None => configs.push(Layout::Config(Config::FitY)),
                        AvailableParameters::ADynamic(a) => configs.push(Layout::Config(Config::FitYmin(DataSrc::Dynamic(a)))),
                        AvailableParameters::AStatic(a) => configs.push(Layout::Config(Config::FitYmin(DataSrc::Static(a)))),
                        AvailableParameters::BDynamic(b) => configs.push(Layout::Config(Config::FitYmax(DataSrc::Dynamic(b)))),
                        AvailableParameters::BStatic(b) => configs.push(Layout::Config(Config::FitYmax(DataSrc::Static(b)))),
                        AvailableParameters::TwoStatic(min, max) => configs.push(Layout::Config(Config::FitYminmax { 
                            min: DataSrc::Static(min), max: DataSrc::Static(max)
                        })),
                        AvailableParameters::TwoDynamic(min, max) => configs.push(Layout::Config(Config::FitYminmax { 
                            min: DataSrc::Dynamic(min), max: DataSrc::Dynamic(max)
                        })),
                        AvailableParameters::ADynamicBStatic(min, max) => configs.push(Layout::Config(Config::FitYminmax { 
                            min: DataSrc::Dynamic(min), max: DataSrc::Static(max)
                        })),
                        AvailableParameters::AStaticBDynamic(min, max) => configs.push(Layout::Config(Config::FitYminmax { 
                            min: DataSrc::Static(min), max: DataSrc::Dynamic(max)
                        })),
                        _ => {}
                    }
                }
                "width-fixed" => {
                    match parameter_check::<f32>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::FixedX(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::FixedX(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "height-fixed" => {
                    match parameter_check::<f32>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::FixedY(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::FixedY(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "width-percent" => {
                    match parameter_check::<f32>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::PercentX(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::PercentX(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "height-percent" => {
                    match parameter_check::<f32>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::PercentY(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::PercentY(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "padding-all" => {
                   match parameter_check::<u16>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::PaddingAll(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::PaddingAll(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "padding-top" => {
                    match parameter_check::<u16>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::PaddingTop(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::PaddingTop(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "padding-right" => {
                    match parameter_check::<u16>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::PaddingRight(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::PaddingRight(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "padding-bottom" => {
                    match parameter_check::<u16>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::PaddingBottom(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::PaddingBottom(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "padding-left" => {
                    match parameter_check::<u16>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::PaddingLeft(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::PaddingLeft(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "child-gap" => {
                    match parameter_check::<u16>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::ChildGap(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::ChildGap(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "vertical" => configs.push(Layout::Config(Config::Vertical)),
                "align-children-x" => {
                    if let Some(alignment) = config.children.get(1)
                    && let Node::Text(alignment) = alignment {
                        match alignment.value.trim() {
                            "left" => configs.push(Layout::Config(Config::ChildAlignmentXLeft)),
                            "right" => configs.push(Layout::Config(Config::ChildAlignmentXRight)),
                            "center" => configs.push(Layout::Config(Config::ChildAlignmentXCenter)),
                            _ => {}
                        }
                    }
                }
                "align-children-y" => {
                    if let Some(alignment) = config.children.get(1)
                    && let Node::Text(alignment) = alignment {
                        match alignment.value.trim() {
                            "top" => configs.push(Layout::Config(Config::ChildAlignmentYTop)),
                            "bottom" => configs.push(Layout::Config(Config::ChildAlignmentYBottom)),
                            "center" => configs.push(Layout::Config(Config::ChildAlignmentYCenter)),
                            _ => {}
                        }
                    }
                }
                "color" => {
                    match parameter_check::<Color>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::Color(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::Color(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "radius-all" => {
                    match parameter_check::<f32>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::RadiusAll(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::RadiusAll(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "radius-top-left" => {
                    match parameter_check::<f32>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::RadiusTopLeft(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::RadiusTopLeft(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "radius-top-right" => {
                    match parameter_check::<f32>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::RadiusTopRight(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::RadiusTopRight(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "radius-bottom-left" => {
                    match parameter_check::<f32>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::RadiusBottomLeft(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::RadiusBottomLeft(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "radius-bottom-right" => {
                    match parameter_check::<f32>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::RadiusBottomRight(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::RadiusBottomRight(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "border-color" => {
                    match parameter_check::<Color>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::BorderColor(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::BorderColor(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "border-all" => {
                    match parameter_check::<u16>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::BorderAll(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::BorderAll(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "border-top" => {
                    match parameter_check::<u16>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::BorderTop(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::BorderTop(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "border-left" => {
                    match parameter_check::<u16>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::BorderLeft(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::BorderLeft(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "border-bottom" => {
                    match parameter_check::<u16>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::BorderBottom(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::BorderBottom(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "border-right" => {
                    match parameter_check::<u16>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::BorderRight(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::BorderRight(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "border-in-between" => {
                    match parameter_check::<u16>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::BorderBetweenChildren(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::BorderBetweenChildren(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                "scroll" => {
                    if let Some(direction_a) = config.children.get(2)
                    && let Node::InlineCode(direction_a) = direction_a
                    && (direction_a.value.as_str() == "x" || direction_a.value.as_str() == "y")
                    && let Some(direction_b) = config.children.get(4)
                    && let Node::InlineCode(direction_b) = direction_b
                    && (direction_b.value.as_str() == "x" || direction_b.value.as_str() == "y"){
                        configs.push(Layout::Config(Config::Clip { vertical: DataSrc::Static(true), horizontal: DataSrc::Static(true) }));
                    }
                    else if let Some(direction_a) = config.children.get(2)
                    && let Node::InlineCode(direction_a) = direction_a
                    && (direction_a.value.as_str() == "x" || direction_a.value.as_str() == "y") {
                        if direction_a.value.as_str() == "x" {
                            configs.push(Layout::Config(Config::Clip { vertical: DataSrc::Static(false), horizontal: DataSrc::Static(true) }));
                        }
                        else {
                            configs.push(Layout::Config(Config::Clip { vertical: DataSrc::Static(true), horizontal: DataSrc::Static(false) }));
                        }
                    }
                }
                "image" => {
                    if let Some(src) = config.children.get(1)
                    && let Node::Text(src) = src {
                        configs.push(Layout::Config(Config::Image { name: src.value.trim().to_string() }));
                    }
                }
                "floating" => {
                    configs.push(Layout::Config(Config::Floating));
                    if let Some(floating_commands) = config_elements.get(1)
                    && let Node::List(floating_commands) = floating_commands {
                        let mut floating = process_configs(floating_commands);
                        configs.append(&mut floating);
                    }
                }
                "use" => {
                    if let Some(reusable_name) = config.children.get(1)
                    && let Node::Text(reusable_name) = reusable_name {
                        configs.push(Layout::Config(Config::Use { name: reusable_name.value.trim().to_string() }));
                    }
                }
                
                "hovered" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::HoveredOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::HoveredOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::HoveredOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                    && let Node::List(onconfig_on) = onconfig_on {
                        configs.append(&mut process_configs(onconfig_on));
                    }
                    configs.push(Layout::Element(Element::HoveredClosed));
                }
                "unhovered" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::UnHoveredOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::UnHoveredOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::UnHoveredOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                    && let Node::List(onconfig_on) = onconfig_on {
                        configs.append(&mut process_configs(onconfig_on));
                    }
                    configs.push(Layout::Element(Element::UnHoveredClosed));
                }
                "hover" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::HoverOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::HoverOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::HoverOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                    && let Node::List(onconfig_on) = onconfig_on {
                        configs.append(&mut process_configs(onconfig_on));
                    }
                    configs.push(Layout::Element(Element::HoverClosed));
                }
                "focused" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::FocusedOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::FocusedOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::FocusedOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                    && let Node::List(onconfig_on) = onconfig_on {
                        configs.append(&mut process_configs(onconfig_on));
                    }
                    configs.push(Layout::Element(Element::FocusedClosed));
                }
                "unfocused" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::UnFocusedOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::UnFocusedOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::UnFocusedOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                    && let Node::List(onconfig_on) = onconfig_on {
                        configs.append(&mut process_configs(onconfig_on));
                    }
                    configs.push(Layout::Element(Element::UnFocusedClosed));
                }
                "focus" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::FocusOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::FocusOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::FocusOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                    && let Node::List(onconfig_on) = onconfig_on {
                        configs.append(&mut process_configs(onconfig_on));
                    }
                    configs.push(Layout::Element(Element::FocusClosed));
                }
                "left-pressed" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::LeftPressedOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::LeftPressedOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::LeftPressedOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                    && let Node::List(onconfig_on) = onconfig_on {
                        configs.append(&mut process_configs(onconfig_on));
                    }
                    configs.push(Layout::Element(Element::LeftPressedClosed));
                }
                "left-down" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::LeftDownOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::LeftDownOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::LeftDownOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                    && let Node::List(onconfig_on) = onconfig_on {
                        configs.append(&mut process_configs(onconfig_on));
                    }
                    configs.push(Layout::Element(Element::LeftDownClosed));
                }
                "left-released" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::LeftReleasedOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::LeftReleasedOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::LeftReleasedOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(onconfig_on) = config_elements.get(1)
                    && let Node::List(onconfig_on) = onconfig_on {
                        configs.append(&mut process_configs(onconfig_on));
                    }
                    configs.push(Layout::Element(Element::LeftReleasedClosed));
                }
                "left-clicked" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::LeftClickedOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::LeftClickedOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::LeftClickedOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                    && let Node::List(config_on_click) = config_on_click {
                        configs.append(&mut process_configs(config_on_click));
                    }
                    configs.push(Layout::Element(Element::LeftClickedClosed));
                }
                "left-dbl-clicked" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::LeftDoubleClickedOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::LeftDoubleClickedOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::LeftDoubleClickedOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                    && let Node::List(config_on_click) = config_on_click {
                        configs.append(&mut process_configs(config_on_click));
                    }
                    configs.push(Layout::Element(Element::LeftDoubleClickedClosed));
                }
                "left-tpl-clicked" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::LeftTripleClickedOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::LeftTripleClickedOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::LeftTripleClickedOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                    && let Node::List(config_on_click) = config_on_click {
                        configs.append(&mut process_configs(config_on_click));
                    }
                    configs.push(Layout::Element(Element::LeftTripleClickedClosed));
                }
                "right-pressed" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::RightPressedOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::RightPressedOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::RightPressedOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                    && let Node::List(config_on_click) = config_on_click {
                        configs.append(&mut process_configs(config_on_click));
                    }
                    configs.push(Layout::Element(Element::RightPressedClosed));
                }
                "right-down" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::RightDownOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::RightDownOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::RightDownOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                    && let Node::List(config_on_click) = config_on_click {
                        configs.append(&mut process_configs(config_on_click));
                    }
                    configs.push(Layout::Element(Element::RightDownClosed));
                }
                "right-released" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::RightReleasedOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::RightReleasedOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::RightReleasedOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                    && let Node::List(config_on_click) = config_on_click {
                        configs.append(&mut process_configs(config_on_click));
                    }
                    configs.push(Layout::Element(Element::RightReleasedClosed));
                }
                "right-clicked" => {
                    match parameter_check::<Event>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Element(Element::RightClickedOpened { 
                            event: Some(DataSrc::Dynamic(a)) 
                        })),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Element(Element::RightClickedOpened { 
                            event: Some(DataSrc::Static(a)) 
                        })),
                        AvailableParameters::None => configs.push(Layout::Element(Element::RightClickedOpened { 
                            event: None 
                        })),
                        _ => {}
                    }
                    if let Some(config_on_click) = config_elements.get(1)
                    && let Node::List(config_on_click) = config_on_click {
                        configs.append(&mut process_configs(config_on_click));
                    }
                    configs.push(Layout::Element(Element::RightClickedClosed));
                }
                "pointer" => {
                    if let Some(pointer) = config.children.get(1)
                    && let Node::Text(pointer) = pointer {
                        match pointer.value.trim() {
                            "standard" => configs.push(Layout::Element(Element::Pointer(winit::window::CursorIcon::Default))),
                            "resize-horizontal" => configs.push(Layout::Element(Element::Pointer(winit::window::CursorIcon::EwResize))),
                            _ => {}
                        }
                    }
                }
                
                "font-id" => {
                    match parameter_check::<u16>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::FontId(
                            DataSrc::Dynamic(a)
                        ))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::FontId(
                            DataSrc::Static(a)
                        ))),
                        _ => {}
                    }
                }
                "font-size" => {
                    match parameter_check::<u16>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::FontSize(
                            DataSrc::Dynamic(a)
                        ))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::FontSize(
                            DataSrc::Static(a)
                        ))),
                        _ => {}
                    }
                }
                "align" => {
                    if let Some(alignment) = config.children.get(1)
                    && let Node::Text(alignment) = alignment {
                        match alignment.value.trim() {
                            "left" => configs.push(Layout::Config(Config::AlignLeft)),
                            "center" => configs.push(Layout::Config(Config::AlignCenter)),
                            "right" => configs.push(Layout::Config(Config::AlignRight)),
                            _ => {}
                        }
                    }
                }
                "line-height" => {
                    match parameter_check::<u16>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::LineHeight(
                            DataSrc::Dynamic(a)
                        ))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::LineHeight(
                            DataSrc::Static(a)
                        ))),
                        _ => {}
                    }
                }
                "letter-spacing" => {
                    // match parameter_check::<u16>(config, "", "") {
                    //     AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::LetterSpacing(
                    //         DataSrc::Dynamic(a)
                    //     ))),
                    //     AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::LetterSpacing(
                    //         DataSrc::Static(a)
                    //     ))),
                    //     _ => {}
                    // }
                }
                "font-color" => {
                    match parameter_check::<Color>(config, "", "") {
                        AvailableParameters::SingleDynamic(a) => configs.push(Layout::Config(Config::FontColor(DataSrc::Dynamic(a)))),
                        AvailableParameters::SingleStatic(a) => configs.push(Layout::Config(Config::FontColor(DataSrc::Static(a)))),
                        _ => {}
                    }
                }
                
                "offset" => {
                    match parameter_check::<f32>(config, "x", "y") {
                        AvailableParameters::ADynamic(a) => configs.push(Layout::Config(Config::FloatingOffset { 
                            x: DataSrc::Dynamic(a), y: DataSrc::Static(0.0) 
                        })),
                        AvailableParameters::AStatic(a) => configs.push(Layout::Config(Config::FloatingOffset { 
                            x: DataSrc::Static(a), y: DataSrc::Static(0.0) 
                        })),
                        AvailableParameters::BDynamic(b) => configs.push(Layout::Config(Config::FloatingOffset { 
                            x: DataSrc::Static(0.0), y: DataSrc::Dynamic(b) 
                        })),
                        AvailableParameters::BStatic(b) => configs.push(Layout::Config(Config::FloatingOffset { 
                            x: DataSrc::Static(0.0), y: DataSrc::Static(b) 
                        })),
                        AvailableParameters::TwoStatic(a, b) => configs.push(Layout::Config(Config::FloatingOffset { 
                            x: DataSrc::Static(a), y: DataSrc::Static(b) 
                        })),
                        AvailableParameters::TwoDynamic(x, y) => configs.push(Layout::Config(Config::FloatingOffset { 
                            x: DataSrc::Dynamic(x), y: DataSrc::Dynamic(y)
                        })),
                        AvailableParameters::ADynamicBStatic(x, y) => configs.push(Layout::Config(Config::FloatingOffset { 
                            x: DataSrc::Dynamic(x), y: DataSrc::Static(y)
                        })),
                        AvailableParameters::AStaticBDynamic(x, y) => configs.push(Layout::Config(Config::FloatingOffset { 
                            x: DataSrc::Static(x), y: DataSrc::Dynamic(y)
                        })),
                        _ => {}
                    }
                }
                "attatch-parent" => {
                    if let Some(attach_point) = config.children.get(1)
                    && let Node::Text(attach_point) = attach_point {
                        match attach_point.value.trim() {
                            "top-left" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchToParentAtTopLeft
                                )
                            ),
                            "center-left" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchToParentAtCenterLeft
                                )
                            ),
                            "bottom-left" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchToParentAtBottomLeft
                                )
                            ),
                            "top-center" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchToParentAtTopCenter
                                )
                            ),
                            "center" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchToParentAtCenter
                                )
                            ),
                            "bottom-center" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchToParentAtBottomCenter
                                )
                            ),
                            "top-right" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchToParentAtTopRight
                                )
                            ),
                            "center-right" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchToParentAtCenterRight
                                )
                            ),
                            "bottom-right" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchToParentAtBottomRight
                                )
                            ),
                            _ => {}
                        }
                    }
                }
                "attach-self" => {
                    if let Some(attach_point) = config.children.get(1)
                    && let Node::Text(attach_point) = attach_point {
                        match attach_point.value.trim() {
                            "top-left" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchElementAtTopLeft
                                )
                            ),
                            "center-left" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchElementAtCenterLeft
                                )
                            ),
                            "bottom-left" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchElementAtBottomLeft
                                )
                            ),
                            "top-center" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchElementAtTopCenter
                                )
                            ),
                            "center" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchElementAtCenter
                                )
                            ),
                            "bottom-center" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchElementAtBottomCenter
                                )
                            ),
                            "top-right" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchElementAtTopRight
                                )
                            ),
                            "center-right" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchElementAtCenterRight
                                )
                            ),
                            "bottom-right" => configs.push(
                                Layout::Config(
                                    Config::FloatingAttatchElementAtBottomRight
                                )
                            ),
                            _ => {}
                        }
                    }
                }
                // TODO: z-index, pointer pass through
                _ => {}
            }
        }
    }

    configs
}