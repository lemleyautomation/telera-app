use std::{collections::HashMap, fmt::Debug, str::FromStr};

use markdown::mdast::{List, Node};
use crate::{ConfigCommand, TextConfigCommand, FlowControlCommand, LayoutCommandType, PageDataCommand};
use telera_layout::Color;
use csscolorparser;

#[derive(Debug)]
enum ParsingMode {
    None,
    Body,
    ReusableElements,
    ReusableConfig,
    Variables,
}

pub fn process_layout<Event: Clone+Debug+PartialEq+FromStr>(file: String) -> Result<(String, Vec<LayoutCommandType<Event>>, HashMap::<String, Vec<LayoutCommandType<Event>>>), String> 
where <Event as FromStr>::Err: Debug
{
    let mut parsing_mode = ParsingMode::None;
    let mut page_name = "".to_string();
    let mut body = Vec::<LayoutCommandType<Event>>::new();
    let mut open_reuseable_name = "".to_string();
    let mut open_variable_name = "".to_string();
    let mut reusables = HashMap::<String, Vec<LayoutCommandType<Event>>>::new();
    let mut local_call_stack = HashMap::<String, PageDataCommand<Event>>::new();

    if let Ok(m) = markdown::to_mdast(&file, &markdown::ParseOptions::default())
    && let Some(nodes) = m.children() {

        for node in nodes {
            match node {
                Node::Heading(h) => {
                    if let Node::Text(t) = &h.children[0] {
                        match h.depth {
                            1 => {
                                parsing_mode = ParsingMode::Body;
                                page_name = t.value.trim().to_string();
                            }
                            2 => {
                                parsing_mode = ParsingMode::Variables;
                                open_variable_name = t.value.trim().to_string();
                            }
                            3 => {
                                parsing_mode = ParsingMode::ReusableElements;
                                open_reuseable_name = t.value.trim().to_string();
                            }
                            4 => {
                                parsing_mode = ParsingMode::ReusableConfig;
                                open_reuseable_name = t.value.trim().to_string();
                            },
                            _ => parsing_mode = ParsingMode::None,
                        }
                    }
                }
                Node::List(list) => {
                    match parsing_mode {
                        ParsingMode::ReusableConfig => {
                            let mut reusable_items = process_configs(list);
                            let mut formatted_reusable_items = Vec::<LayoutCommandType<Event>>::new();
                            //formatted_reusable_items.push(LayoutCommandType::FlowControl(FlowControlCommand::ConfigOpened));
                            formatted_reusable_items.append(&mut reusable_items);
                            //formatted_reusable_items.push(LayoutCommandType::FlowControl(FlowControlCommand::ConfigClosed));
                            reusables.insert(open_reuseable_name.clone(), formatted_reusable_items);
                        }
                        ParsingMode::ReusableElements => {
                            for node in &list.children{
                                let element = process_element(node);
                                reusables.insert(open_reuseable_name.clone(), element);
                            }
                            
                        }
                        ParsingMode::Variables => {
                            local_call_stack.insert(
                                open_variable_name.clone(), 
                                process_variable(open_variable_name.clone(), &list.children)
                            );
                        }
                        ParsingMode::Body => {
                            for node in &list.children {
                                let mut element = process_element(node);
                                body.append(&mut element);
                            }
                        }
                        _ => return Err("Invalid File".to_string())
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

fn process_element<Event: Clone+Debug+PartialEq+FromStr>(node: &Node) -> Vec<LayoutCommandType<Event>>
where <Event as FromStr>::Err: Debug
{
    let mut layout_commands: Vec<LayoutCommandType<Event>> = Vec::new();

    if let Node::ListItem(element) = node 
    && let Node::Paragraph(p) = &element.children[0]
    && let Node::InlineCode(element_type) = &p.children[0] {
        match element_type.value.as_str() {
            "element" => {
                layout_commands.push(LayoutCommandType::FlowControl(FlowControlCommand::ElementOpened { id: None }));
                layout_commands.push(LayoutCommandType::FlowControl(FlowControlCommand::ConfigOpened));
                if let Some(config) = element.children.get(1)
                && let Node::List(configs) = config
                && let Some(configs) = configs.children.get(0)
                && let Node::ListItem(configs) = configs
                && let Some(configs) = configs.children.get(1)
                && let Node::List(config_commands) = configs {
                    let mut layout_config_commands = process_configs(&config_commands);
                    layout_commands.append(&mut layout_config_commands);
                }
                layout_commands.push(LayoutCommandType::FlowControl(FlowControlCommand::ConfigClosed));

                if let Node::List(l) = &element.children[1] {
                    for child_element in l.children.iter().skip(1) {
                        let mut child_element = process_element(child_element);
                        layout_commands.append(&mut child_element);
                    }
                }

                layout_commands.push(LayoutCommandType::FlowControl(FlowControlCommand::ElementClosed));
            }
            "text" => {
                layout_commands.push(LayoutCommandType::FlowControl(FlowControlCommand::TextElementOpened));
                layout_commands.push(LayoutCommandType::FlowControl(FlowControlCommand::TextConfigOpened));
                if let Node::List(l) = &element.children[1]
                && let Node::ListItem(text_config_tree) = &l.children[0]
                && let Node::List(text_config_commands) = &text_config_tree.children[1] {
                    let mut text_config = process_text_configs(text_config_commands);
                    layout_commands.append(&mut text_config);
                }
                layout_commands.push(LayoutCommandType::FlowControl(FlowControlCommand::TextConfigClosed));
                if let Node::List(l) = &element.children[1]
                && let Node::ListItem(text_content_tree) = &l.children[1]
                && let Node::Paragraph(text_node) = &text_content_tree.children[0] {
                    match &text_node.children[0] {
                        Node::Emphasis(e) => {
                            if let Node::Text(var_name) = &e.children[0] {
                                layout_commands.push(LayoutCommandType::TextConfig(TextConfigCommand::DynamicContent(var_name.value.trim().to_string())));
                            }
                        }
                        Node::Text(content) => {
                            layout_commands.push(LayoutCommandType::TextConfig(TextConfigCommand::Content(content.value.trim().to_string())));
                        }
                        _ => {}
                    }
                }
                layout_commands.push(LayoutCommandType::FlowControl(FlowControlCommand::TextElementClosed));
            }
            "use" => {
                //println!("{:#?}", element);
                if let Node::Text(reusable_name) = &p.children[1] 
                && let Node::List(input_variables) = &element.children[1] {
                    layout_commands.push(LayoutCommandType::FlowControl(FlowControlCommand::UseOpened { name: reusable_name.value.trim().to_string() }));
                    for input_variable in &input_variables.children {
                        if let Node::ListItem(input_variable) = input_variable
                        && let Node::Paragraph(input_variable) = &input_variable.children[0]
                        && let Node::InlineCode(variable_type) = &input_variable.children[0]
                        && let Node::Emphasis(variable_name) = &input_variable.children[2]
                        && let Node::Text(variable_name) = &variable_name.children[0]
                        && let Node::Text(variable_value) = &input_variable.children[3] {
                            match variable_type.value.as_str() {
                                "get-bool" => {}
                                "get-numeric" => {}
                                "get-text" => {}
                                "get-image" => {}
                                "get-event" => {}
                                "set-bool" => {}
                                "set-numeric" => {}
                                "set-text" => {
                                    layout_commands.push(LayoutCommandType::PageData(PageDataCommand::SetText { 
                                        local: variable_name.value.trim().to_string(), 
                                        to: variable_value.value.trim().to_string() 
                                    }))
                                }
                                "set-image" => {}
                                "set-event" => {
                                    layout_commands.push(LayoutCommandType::PageData(PageDataCommand::SetEvent { 
                                        local: variable_name.value.trim().to_string(), 
                                        to: Event::from_str(variable_value.value.trim()).unwrap()
                                    }))
                                }
                                _ => {}
                            }
                        }
                    }
                    layout_commands.push(LayoutCommandType::FlowControl(FlowControlCommand::UseClosed));
                }
                
            }
            "list" => {
                if let Node::Text(list_src) = &p.children[1] 
                && let Node::List(list_content) = &element.children[1] {

                    let mut formatted_list = Vec::<LayoutCommandType<Event>>::new();
                    formatted_list.push(LayoutCommandType::FlowControl(FlowControlCommand::ListOpened { src: list_src.value.trim().to_string() }));

                    if let Some(declarations) = list_content.children.get(0)
                    && let Node::ListItem(declarations) = declarations
                    && let Some(declarations) = declarations.children.get(1)
                    && let Node::List(declarations) = declarations {
                        for declaration in &declarations.children {
                            if let Node::ListItem(declaration) = declaration
                            && let Some(declaration) = declaration.children.get(0)
                            && let Node::Paragraph(declaration) = declaration
                            && let Some(declaration_type) = declaration.children.get(0)
                            && let Node::InlineCode(declaration_type) = declaration_type
                            && let Some(declaration_name) = declaration.children.get(2)
                            && let Node::Emphasis(declaration_name) = declaration_name
                            && let Some(declaration_name) = declaration_name.children.get(0)
                            && let Node::Text(declaration_name) = declaration_name
                            && let Some(declaration_value) = declaration.children.get(3)
                            && let Node::Text(declaration_value) = declaration_value {
                                match declaration_type.value.trim() {
                                    "get-bool" => {
                                        formatted_list.push(LayoutCommandType::PageData(PageDataCommand::GetBool { 
                                            local: declaration_name.value.clone(),
                                            from: declaration_value.value.trim().to_string() 
                                        }));
                                    }
                                    "get-text" => {
                                        formatted_list.push(LayoutCommandType::PageData(PageDataCommand::GetText { 
                                            local: declaration_name.value.clone(),
                                            from: declaration_value.value.trim().to_string() 
                                        }));
                                    }
                                    "get-event" => {
                                        formatted_list.push(LayoutCommandType::PageData(PageDataCommand::GetEvent { 
                                            local: declaration_name.value.clone(),
                                            from: declaration_value.value.trim().to_string() 
                                        }));
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    for li in list_content.children.iter().skip(1) {
                        let mut list_item = process_element::<Event>(&li);
                        formatted_list.append(&mut list_item);
                    }

                    formatted_list.push(LayoutCommandType::FlowControl(FlowControlCommand::ListClosed));

                    layout_commands.append(&mut formatted_list);
                }
            }
            "if" => {
                if let Node::Text(conditional) = &p.children[1] 
                && let Node::List(conditional_elements) = &element.children[1] {

                    let mut formatted_element = Vec::<LayoutCommandType<Event>>::new();
                    formatted_element.push(LayoutCommandType::FlowControl(FlowControlCommand::IfOpened { condition: conditional.value.trim().to_string() }));

                    for conditional_element in &conditional_elements.children {
                        let mut conditional_element = process_element::<Event>(&conditional_element);
                        formatted_element.append(&mut conditional_element);
                    }

                    formatted_element.push(LayoutCommandType::FlowControl(FlowControlCommand::IfClosed));

                    layout_commands.append(&mut formatted_element);
                }
            }
            "if-not" => {
                if let Node::Text(conditional) = &p.children[1] 
                && let Node::List(conditional_elements) = &element.children[1] {

                    let mut formatted_element = Vec::<LayoutCommandType<Event>>::new();
                    formatted_element.push(LayoutCommandType::FlowControl(FlowControlCommand::IfNotOpened { condition: conditional.value.trim().to_string() }));

                    for conditional_element in &conditional_elements.children {
                        let mut conditional_element = process_element::<Event>(&conditional_element);
                        formatted_element.append(&mut conditional_element);
                    }

                    formatted_element.push(LayoutCommandType::FlowControl(FlowControlCommand::IfClosed));

                    layout_commands.append(&mut formatted_element);
                }
            }
            _ => {}
        }
    }

    layout_commands
}

fn process_variable<Event: Clone+Debug+PartialEq>(local: String, nodes: &Vec<Node>) -> PageDataCommand<Event>{
    if nodes.len() as u32 == 1
    && let Node::ListItem(l) = &nodes[0] 
    && l.children.len() as u32 == 1
    && let Node::Paragraph(p) = &l.children[0] 
    && p.children.len() as u8 == 2
    && let Node::InlineCode(var_type) = &p.children[0]
    && let Node::Text(var_value) = &p.children[1] {
        match var_type.value.as_str() {
            "set-bool" => return PageDataCommand::<Event>::SetBool { local, 
                to: match bool::from_str(&var_value.value.trim()) {
                    Ok(v) => v,
                    Err(_) => false
                }
            },
            "set-text" => return PageDataCommand::<Event>::SetText { local, 
                to: var_value.value.trim().to_string()
            },
            "set-color" => return PageDataCommand::<Event>::SetColor { local, 
                to: match csscolorparser::parse(&var_value.value) {
                    Err(_) => Color::default(),
                    Ok(color) => color.to_rgba8().into(),
                }
            },
            _ => {}
        }
    }

    PageDataCommand::SetBool { local: "".to_string(), to: false }
}

fn process_configs<Event: Clone+Debug+PartialEq>(configuration_set: &List) -> Vec<LayoutCommandType<Event>> {
    let mut configs = Vec::new();

    for configuration_item in &configuration_set.children {
        if let Some(node_with_keys) = configuration_item.children()
        && let Node::Paragraph(keys) = &node_with_keys[0]
        && let Node::InlineCode(config_type) = &keys.children[0] {
            match config_type.value.as_str() {
                "grow" => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::GrowAll)),
                // TODO: optimize both grow cases
                "width-grow" => {
                    match &keys.children.len() {
                        1 => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::GrowX)),
                        4 => {
                            if let Node::InlineCode(i) = &keys.children[2] 
                            && i.value.as_str() == "min" 
                            && let Node::Text(min) = &keys.children[3] {
                                configs.push(LayoutCommandType::ElementConfig(ConfigCommand::GrowXmin { 
                                    min: match f32::from_str(&min.value.trim()) {
                                        Ok(f) => f,
                                        Err(_) => 0.0
                                    }
                                }));
                            }
                        }
                        6 => {
                            if let Node::InlineCode(key) = &keys.children[2] 
                            && key.value.as_str() == "min" 
                            && let Node::Text(min) = &keys.children[3]
                            && let Node::InlineCode(key2) = &keys.children[4]
                            && key2.value.as_str() == "max"
                            && let Node::Text(max) = &keys.children[5] {
                                configs.push(LayoutCommandType::ElementConfig(ConfigCommand::GrowXminmax {
                                    min: match f32::from_str(&min.value.trim()) {
                                        Ok(f) => f,
                                        Err(_) => 0.0
                                    },
                                    max: match f32::from_str(&max.value.trim()) {
                                        Ok(f) => f,
                                        Err(_) => 0.0
                                    }
                                }));
                            }
                        }
                        _ => {}
                    }
                }
                "height-grow" => {
                    match &keys.children.len() {
                        1 => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::GrowY)),
                        4 => {
                            if let Node::InlineCode(i) = &keys.children[2] 
                            && i.value.as_str() == "min" 
                            && let Node::Text(min) = &keys.children[3] {
                                configs.push(LayoutCommandType::ElementConfig(ConfigCommand::GrowYmin { 
                                    min: match f32::from_str(&min.value.trim()) {
                                        Ok(f) => f,
                                        Err(_) => 0.0
                                    }
                                }));
                            }
                        }
                        6 => {
                            if let Node::InlineCode(key) = &keys.children[2] 
                            && key.value.as_str() == "min" 
                            && let Node::Text(min) = &keys.children[3]
                            && let Node::InlineCode(key2) = &keys.children[4]
                            && key2.value.as_str() == "max"
                            && let Node::Text(max) = &keys.children[5] {
                                configs.push(LayoutCommandType::ElementConfig(ConfigCommand::GrowYminmax {
                                    min: match f32::from_str(&min.value.trim()) {
                                        Ok(f) => f,
                                        Err(_) => 0.0
                                    },
                                    max: match f32::from_str(&max.value.trim()) {
                                        Ok(f) => f,
                                        Err(_) => 0.0
                                    }
                                }));
                            }
                        }
                        _ => {}
                    }
                }
                "fit-width" => {
                    if let Some(fit_a) = keys.children.get(2)
                    && let Node::InlineCode(fit_a) = fit_a
                    && (fit_a.value.trim() == "min" || fit_a.value.trim() == "max")
                    && let Some(value_a) = keys.children.get(3)
                    && let Node::Text(value_a) = value_a
                    && let Ok(value_a) = f32::from_str(value_a.value.trim())

                    && let Some(fit_b) = keys.children.get(4)
                    && let Node::InlineCode(fit_b) = fit_b
                    && (fit_b.value.trim() == "min" || fit_b.value.trim() == "max")
                    && let Some(value_b) = keys.children.get(5)
                    && let Node::Text(value_b) = value_b
                    && let Ok(value_b) = f32::from_str(value_b.value.trim()) {
                        if fit_a.value.trim() == "min" {
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FitXminmax {
                                min: value_a,
                                max: value_b
                            }));
                        }
                        else {
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FitXminmax {
                                min: value_b,
                                max: value_a
                            }));
                        }
                        
                    }
                    else if let Some(fit) = keys.children.get(2)
                    && let Node::InlineCode(fit) = fit
                    && fit.value.as_str() == "min" 
                    && let Some(min) = keys.children.get(3)
                    && let Node::Text(min) = min
                    && let Ok(min) = f32::from_str(min.value.trim()) {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FitXmin { 
                            min
                        }));
                    }
                    else {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FitX));
                    }
                }
                "fit-height" => {
                    if let Some(fit_a) = keys.children.get(2)
                    && let Node::InlineCode(fit_a) = fit_a
                    && (fit_a.value.trim() == "min" || fit_a.value.trim() == "max")
                    && let Some(value_a) = keys.children.get(3)
                    && let Node::Text(value_a) = value_a
                    && let Ok(value_a) = f32::from_str(value_a.value.trim())

                    && let Some(fit_b) = keys.children.get(4)
                    && let Node::InlineCode(fit_b) = fit_b
                    && (fit_b.value.trim() == "min" || fit_b.value.trim() == "max")
                    && let Some(value_b) = keys.children.get(5)
                    && let Node::Text(value_b) = value_b
                    && let Ok(value_b) = f32::from_str(value_b.value.trim()) {
                        if fit_a.value.trim() == "min" {
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FitYminmax {
                                min: value_a,
                                max: value_b
                            }));
                        }
                        else {
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FitYminmax {
                                min: value_b,
                                max: value_a
                            }));
                        }
                        
                    }
                    else if let Some(fit) = keys.children.get(2)
                    && let Node::InlineCode(fit) = fit
                    && fit.value.as_str() == "min" 
                    && let Some(min) = keys.children.get(3)
                    && let Node::Text(min) = min
                    && let Ok(min) = f32::from_str(min.value.trim()) {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FitYmin { 
                            min
                        }));
                    }
                    else {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FitY));
                    }
                }
                "width-fixed" => {
                    if let Some(dynamic_value) = keys.children.get(2)
                    && let Node::Emphasis(dynamic_value) = dynamic_value
                    && let Some(dynamic_value) = dynamic_value.children.get(0)
                    && let Node::Text(dynamic_value) = dynamic_value {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FixedXFrom(
                            dynamic_value.value.trim().to_string()
                        )));
                    }
                    else if let Some(static_value) = keys.children.get(1)
                    && let Node::Text(static_value) = static_value
                    && let Ok(static_value) = f32::from_str(static_value.value.trim()) {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FixedX(static_value)));
                    }
                }
                "height-fixed" => {
                    if let Some(dynamic_value) = keys.children.get(2)
                    && let Node::Emphasis(dynamic_value) = dynamic_value
                    && let Some(dynamic_value) = dynamic_value.children.get(0)
                    && let Node::Text(dynamic_value) = dynamic_value {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FixedYFrom(
                            dynamic_value.value.trim().to_string()
                        )));
                    }
                    else if let Some(static_value) = keys.children.get(1)
                    && let Node::Text(static_value) = static_value
                    && let Ok(static_value) = f32::from_str(static_value.value.trim()) {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FixedY(static_value)));
                    }
                }
                "width-percent" => {
                    if let Some(dynamic_percent) = keys.children.get(2)
                    && let Node::Emphasis(dynamic_percent) = dynamic_percent
                    && let Some(dynamic_percent) = dynamic_percent.children.get(0)
                    && let Node::Text(_dynamic_percent) = dynamic_percent {
                        // configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PercentXFrom(
                        //     dynamic_percent.value.trim().to_string()
                        // )));
                    }
                    else if let Some(static_value) = keys.children.get(1)
                    && let Node::Text(static_value) = static_value
                    && let Ok(static_value) = f32::from_str(static_value.value.trim()) {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PercentX(static_value)));
                    }
                }
                "height-percent" => {
                    if let Some(dynamic_percent) = keys.children.get(2)
                    && let Node::Emphasis(dynamic_percent) = dynamic_percent
                    && let Some(dynamic_percent) = dynamic_percent.children.get(0)
                    && let Node::Text(_dynamic_percent) = dynamic_percent {
                        // configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PercentYFrom(
                        //     dynamic_percent.value.trim().to_string()
                        // )));
                    }
                    else if let Some(static_percent) = keys.children.get(1)
                    && let Node::Text(static_percent) = static_percent
                    && let Ok(static_percent) = f32::from_str(static_percent.value.trim()) {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PercentY(static_percent)));
                    }
                }
                "padding-all" => {
                    if let Some(value) = keys.children.get(1)
                    && let Node::Text(value) = value
                    && let Ok(value) = u16::from_str(&value.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PaddingAll(value)));
                    }
                }
                "padding-top" => {
                    if let Some(value) = keys.children.get(1)
                    && let Node::Text(value) = value
                    && let Ok(value) = u16::from_str(&value.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PaddingTop(value)));
                    }
                }
                "padding-right" => {
                    if let Some(value) = keys.children.get(1)
                    && let Node::Text(value) = value
                    && let Ok(value) = u16::from_str(&value.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PaddingRight(value)));
                    }
                }
                "padding-bottom" => {
                    if let Some(value) = keys.children.get(1)
                    && let Node::Text(value) = value
                    && let Ok(value) = u16::from_str(&value.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PaddingBottom(value)));
                    }
                }
                "padding-left" => {
                    if let Some(value) = keys.children.get(1)
                    && let Node::Text(value) = value
                    && let Ok(value) = u16::from_str(&value.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PaddingLeft(value)));
                    }
                }
                "child-gap" => {
                    if let Some(value) = keys.children.get(1)
                    && let Node::Text(value) = value
                    && let Ok(value) = u16::from_str(&value.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::ChildGap(value)));
                    }
                }
                "direction" => {
                    if let Some(direction) = keys.children.get(1)
                    && let Node::Text(direction) = direction
                    && direction.value.trim() == "ttb" {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::DirectionTTB));
                    }
                }
                "align-children-x" => {
                    if let Some(alignment) = keys.children.get(1)
                    && let Node::Text(alignment) = alignment {
                        match alignment.value.trim() {
                            "left" => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::ChildAlignmentXLeft)),
                            "right" => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::ChildAlignmentXRight)),
                            "center" => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::ChildAlignmentXCenter)),
                            _ => {}
                        }
                    }
                }
                "align-children-y" => {
                    if let Some(alignment) = keys.children.get(1)
                    && let Node::Text(alignment) = alignment {
                        match alignment.value.trim() {
                            "top" => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::ChildAlignmentYTop)),
                            "bottom" => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::ChildAlignmentYBottom)),
                            "center" => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::ChildAlignmentYCenter)),
                            _ => {}
                        }
                    }
                }
                "color" => {
                    if let Some(dynamic_color) = keys.children.get(2)
                    && let Node::Emphasis(dynamic_color) = dynamic_color
                    && let Some(dynamic_color) = dynamic_color.children.get(0)
                    && let Node::Text(dynamic_color) = dynamic_color {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::DynamicColor(
                            dynamic_color.value.to_string()
                        )));
                    }
                    else if let Some(static_color) = keys.children.get(1)
                    && let Node::Text(static_color) = static_color 
                    && let Ok(static_color) = csscolorparser::parse(&static_color.value.trim()) {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::Color(
                            static_color.to_rgba8().into()
                        )));
                    }
                    else {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::Color(
                            Color::default()
                        )));
                    }
                }
                "radius-all" => {
                    if let Some(value) = keys.children.get(1)
                    && let Node::Text(value) = value
                    && let Ok(value) = f32::from_str(&value.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::RadiusAll(value)));
                    }
                }
                "radius-top-left" => {
                    if let Some(value) = keys.children.get(1)
                    && let Node::Text(value) = value
                    && let Ok(value) = f32::from_str(&value.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::RadiusTopLeft(value)));
                    }
                }
                "radius-top-right" => {
                    if let Some(value) = keys.children.get(1)
                    && let Node::Text(value) = value
                    && let Ok(value) = f32::from_str(&value.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::RadiusTopRight(value)));
                    }
                }
                "radius-bottom-left" => {
                    if let Some(value) = keys.children.get(1)
                    && let Node::Text(value) = value
                    && let Ok(value) = f32::from_str(&value.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::RadiusBottomLeft(value)));
                    }
                }
                "radius-bottom-right" => {
                    if let Some(value) = keys.children.get(1)
                    && let Node::Text(value) = value
                    && let Ok(value) = f32::from_str(&value.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::RadiusBottomRight(value)));
                    }
                }
                "border-color" => {
                    if let Some(dynamic_color) = keys.children.get(2)
                    && let Node::Emphasis(dynamic_color) = dynamic_color
                    && let Some(dynamic_color) = dynamic_color.children.get(0)
                    && let Node::Text(dynamic_color) = dynamic_color {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderDynamicColor(
                            dynamic_color.value.to_string()
                        )));
                    }
                    else if let Some(static_color) = keys.children.get(1)
                    && let Node::Text(static_color) = static_color 
                    && let Ok(static_color) = csscolorparser::parse(&static_color.value.trim()) {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderColor(
                            static_color.to_rgba8().into()
                        )));
                    }
                    else {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderColor(
                            Color::default()
                        )));
                    }
                }
                "border-all" => {
                    if let Some(width) = keys.children.get(1)
                    && let Node::Text(width) = width
                    && let Ok(width) = f32::from_str(&width.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderAll(width)));
                    }
                }
                "border-top" => {
                    if let Some(width) = keys.children.get(1)
                    && let Node::Text(width) = width
                    && let Ok(width) = f32::from_str(&width.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderTop(width)));
                    }
                }
                "border-left" => {
                    if let Some(width) = keys.children.get(1)
                    && let Node::Text(width) = width
                    && let Ok(width) = f32::from_str(&width.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderLeft(width)));
                    }
                }
                "border-bottom" => {
                    if let Some(width) = keys.children.get(1)
                    && let Node::Text(width) = width
                    && let Ok(width) = f32::from_str(&width.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderBottom(width)));
                    }
                }
                "border-right" => {
                    if let Some(width) = keys.children.get(1)
                    && let Node::Text(width) = width
                    && let Ok(width) = f32::from_str(&width.value.trim()) {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderRight(width)));
                    }
                }
                "border-in-between" => {
                    if let Some(width) = keys.children.get(1)
                    && let Node::Text(width) = width
                    && let Ok(width) = f32::from_str(&width.value.trim()) {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderBetweenChildren(width)));
                    }
                }
                "scroll" => {
                    match &keys.children.len() {
                        3 => {
                            if let Some(scroll_direction) = keys.children.get(2)
                            && let Node::InlineCode(scroll_direction) = scroll_direction {
                                if scroll_direction.value.as_str() == "x" {
                                    configs.push(LayoutCommandType::ElementConfig(ConfigCommand::Clip { vertical: false, horizontal: true }));
                                }
                                else if scroll_direction.value.as_str() == "y" {
                                    configs.push(LayoutCommandType::ElementConfig(ConfigCommand::Clip { vertical: true, horizontal: false }));
                                }
                            }
                        }
                        5 => {
                            if let Some(scroll_direction_a) = keys.children.get(2)
                            && let Some(scroll_direction_b) = keys.children.get(4)
                            && let Node::InlineCode(scroll_direction_a) = scroll_direction_a
                            && let Node::InlineCode(scroll_direction_b) = scroll_direction_b
                            && (
                                (scroll_direction_a.value == "x" && scroll_direction_b.value == "y") || 
                                (scroll_direction_a.value == "y" && scroll_direction_b.value == "x")
                            ) {
                                configs.push(LayoutCommandType::ElementConfig(ConfigCommand::Clip { vertical: true, horizontal: true }));
                            }
                        }
                        _ => {}
                    }
                }
                // TODO: optimize
                "image" => {
                    let emphasees = keys.children.iter().filter(|node|{
                        if let Node::Emphasis(_)= node{
                            return true
                        } return false
                    }).collect::<Vec<&Node>>();

                    if emphasees.len() > 0 {
                        if let Node::Emphasis(e) = &keys.children[2] 
                        && let Node::Text(t) = &e.children[0] {
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::Image { name: t.value.trim().to_string() }));
                        }
                    }
                    else {
                        if let Node::Text(t) = &keys.children[1] {
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::Image { name: t.value.trim().to_string() }));
                        }
                    }
                }
                "floating" => {
                    configs.push(LayoutCommandType::ElementConfig(ConfigCommand::Floating));
                    if let Some(floating_commands) = node_with_keys.get(1)
                    && let Node::List(floating_commands) = floating_commands {
                        let mut floating = process_floating::<Event>(floating_commands);
                        configs.append(&mut floating);
                    }
                }
                "use" => {
                    if let Some(reusable_name) = keys.children.get(1)
                    && let Node::Text(reusable_name) = reusable_name {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::Use { name: reusable_name.value.trim().to_string() }));
                    }
                }
                "clicked" => {
                    if let Some(dynamic_event) = keys.children.get(2)
                    && let Node::Emphasis(dynamic_event) = dynamic_event
                    && let Some(dynamic_event) = dynamic_event.children.get(0)
                    && let Node::Text(dynamic_event) = dynamic_event {
                        configs.push(LayoutCommandType::FlowControl(FlowControlCommand::ClickedOpened { 
                            event: Some(dynamic_event.value.trim().to_string()) 
                        }));
                    }
                    else if let Some(static_event) = keys.children.get(1)
                    && let Node::Text(static_event) = static_event {
                        configs.push(LayoutCommandType::FlowControl(FlowControlCommand::ClickedOpened { 
                            event: Some(static_event.value.trim().to_string()) 
                        }));
                    }
                    else {
                        configs.push(LayoutCommandType::FlowControl(FlowControlCommand::ClickedOpened { 
                            event: None
                        }));
                    }
                    if let Some(config_on_click) = node_with_keys.get(1)
                    && let Node::List(config_on_click) = config_on_click {
                        configs.append(&mut process_configs(config_on_click));
                    }
                    configs.push(LayoutCommandType::FlowControl(FlowControlCommand::ClickedClosed));
                }
                "hovered" => {
                    configs.push(LayoutCommandType::FlowControl(FlowControlCommand::HoveredOpened));
                    if let Some(config_on_hover) = node_with_keys.get(1)
                    && let Node::List(config_on_hover) = config_on_hover {
                        configs.append(&mut process_configs(config_on_hover));
                    }
                    configs.push(LayoutCommandType::FlowControl(FlowControlCommand::HoveredClosed));
                }
                _ => {}
            }
        }
    }

    configs
}

fn process_floating<Event: Clone+Debug+PartialEq>(floating_config: &List) -> Vec<LayoutCommandType<Event>> {
    let mut floating_commands = Vec::new();

    for config in &floating_config.children {
        if let Node::ListItem(config) = config
        && let Some(config) = config.children.get(0)
        && let Node::Paragraph(config) = config
        && let Some(config_type) = config.children.get(0)
        && let Node::InlineCode(config_type) = config_type {
            match config_type.value.as_str() {
                "offset" => {
                    if let Some(offset_type_a) = config.children.get(2)
                    && let Node::InlineCode(offset_type_a) = offset_type_a
                    && (offset_type_a.value == "x" || offset_type_a.value == "y")
                    && let Some(offset_value_a) = config.children.get(3)
                    && let Node::Text(offset_value_a) = offset_value_a
                    && let Ok(offset_value_a) = f32::from_str(&offset_value_a.value.trim())
                    && let Some(offset_type_b) = config.children.get(4)
                    && let Node::InlineCode(offset_type_b) = offset_type_b
                    && (offset_type_b.value == "x" || offset_type_b.value == "y")
                    && let Some(offset_value_b) = config.children.get(5)
                    && let Node::Text(offset_value_b) = offset_value_b
                    && let Ok(offset_value_b) = f32::from_str(&offset_value_b.value.trim()) {
                        if offset_type_a.value == "x" {
                            floating_commands.push(LayoutCommandType::ElementConfig(ConfigCommand::FloatingOffset { 
                                x: offset_value_a, 
                                y: offset_value_b, 
                                x_from: None, y_from: None 
                            }));
                        }
                        else {
                            floating_commands.push(LayoutCommandType::ElementConfig(ConfigCommand::FloatingOffset { 
                                x: offset_value_b, 
                                y: offset_value_a, 
                                x_from: None, y_from: None 
                            }));
                        }
                        //
                    }
                    else if let Some(offset_type) = config.children.get(2)
                    && let Node::InlineCode(offset_type) = offset_type
                    && (offset_type.value == "x" || offset_type.value == "y")
                    && let Some(offset_value) = config.children.get(3)
                    && let Node::Text(offset_value) = offset_value
                    && let Ok(offset_value) = f32::from_str(&offset_value.value.trim()) {
                        if offset_type.value == "x" {
                            floating_commands.push(LayoutCommandType::ElementConfig(ConfigCommand::FloatingOffset { 
                                x: offset_value, 
                                y: 0.0, 
                                x_from: None, y_from: None 
                            }));
                        }
                        else {
                            floating_commands.push(LayoutCommandType::ElementConfig(ConfigCommand::FloatingOffset { 
                                x: 0.0, 
                                y: offset_value, 
                                x_from: None, y_from: None 
                            }));
                        }
                    }
                }
                "attatch-parent" => {
                    if let Some(attach_point) = config.children.get(1)
                    && let Node::Text(attach_point) = attach_point {
                        match attach_point.value.trim() {
                            "top-left" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchToParentAtTopLeft
                                )
                            ),
                            "center-left" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchToParentAtCenterLeft
                                )
                            ),
                            "bottom-left" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchToParentAtBottomLeft
                                )
                            ),
                            "top-center" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchToParentAtTopCenter
                                )
                            ),
                            "center" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchToParentAtCenter
                                )
                            ),
                            "bottom-center" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchToParentAtBottomCenter
                                )
                            ),
                            "top-right" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchToParentAtTopRight
                                )
                            ),
                            "center-right" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchToParentAtCenterRight
                                )
                            ),
                            "bottom-right" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchToParentAtBottomRight
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
                            "top-left" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchElementAtTopLeft
                                )
                            ),
                            "center-left" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchElementAtCenterLeft
                                )
                            ),
                            "bottom-left" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchElementAtBottomLeft
                                )
                            ),
                            "top-center" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchElementAtTopCenter
                                )
                            ),
                            "center" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchElementAtCenter
                                )
                            ),
                            "bottom-center" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchElementAtBottomCenter
                                )
                            ),
                            "top-right" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchElementAtTopRight
                                )
                            ),
                            "center-right" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchElementAtCenterRight
                                )
                            ),
                            "bottom-right" => floating_commands.push(
                                LayoutCommandType::ElementConfig(
                                    ConfigCommand::FloatingAttatchElementAtBottomRight
                                )
                            ),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    floating_commands
}

fn process_text_configs<Event: Clone+Debug+PartialEq>(configuration_set: &List) -> Vec<LayoutCommandType<Event>> {
    let mut layout_commands = Vec::new();

    for config_item in &configuration_set.children {
        if let Node::ListItem(l) = &config_item
        && let Node::Paragraph(keys) = &l.children[0]
        && let Node::InlineCode(config_type) = &keys.children[0] {
            match config_type.value.as_str() {
                "font-id" => {
                    if let Node::Text(value) = &keys.children[1] 
                    && let Ok(value) = u16::from_str(&value.value.trim()) {
                        layout_commands.push(LayoutCommandType::TextConfig(TextConfigCommand::FontId(value)));
                    }
                }
                "font-size" => {
                    if let Node::Text(value) = &keys.children[1] 
                    && let Ok(value) = u16::from_str(&value.value.trim()) {
                        layout_commands.push(LayoutCommandType::TextConfig(TextConfigCommand::FontSize(value)));
                    }
                }
                "line-height" => {
                    if let Node::Text(value) = &keys.children[1]
                    && let Ok(value) = u16::from_str(&value.value.trim()) {
                        layout_commands.push(LayoutCommandType::TextConfig(TextConfigCommand::LineHeight(value)));
                    }
                }
                "letter-spacing" => {
                    if let Node::Text(value) = &keys.children[1]
                    && let Ok(_value) = u16::from_str(&value.value.trim()) {
                        //layout_commands.push(LayoutCommandType::TextConfig(telera_app::TextConfigCommand::LetterSpacing(value)));
                    }
                }
                "color" => {
                    if let Node::Text(t) = &keys.children[1] {
                        layout_commands.push(LayoutCommandType::TextConfig(TextConfigCommand::Color(
                            match csscolorparser::parse(&t.value) {
                                Err(_) => Color::default(),
                                Ok(color) => color.to_rgba8().into(),
                            }
                        )));
                    }
                }
                _ => {}
            }
        }
    }

    layout_commands
}

#[allow(dead_code)]
fn pvec<T: Debug>(vec: &Vec<T>){
    println!("*******************************************************************");
    vec.iter().for_each(|element| println!("{:?}", element));
    println!("*******************************************************************");
}