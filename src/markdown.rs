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

pub fn process_layout<Event: Clone+Debug+PartialEq>(file: String) -> Result<(String, Vec<LayoutCommandType<Event>>, HashMap::<String, Vec<LayoutCommandType<Event>>>), String> {
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
                            formatted_reusable_items.push(LayoutCommandType::FlowControl(FlowControlCommand::ConfigOpened));
                            formatted_reusable_items.append(&mut reusable_items);
                            formatted_reusable_items.push(LayoutCommandType::FlowControl(FlowControlCommand::ConfigClosed));
                            reusables.insert(open_reuseable_name.clone(), reusable_items);
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

fn process_element<Event: Clone+Debug+PartialEq>(node: &Node) -> Vec<LayoutCommandType<Event>>{
    let mut layout_commands = Vec::new();

    if let Node::ListItem(element) = node 
    && let Node::Paragraph(p) = &element.children[0]
    && let Node::InlineCode(element_type) = &p.children[0] {
        match element_type.value.as_str() {
            "element" => {
                layout_commands.push(LayoutCommandType::FlowControl(FlowControlCommand::ElementOpened { id: None }));
                layout_commands.push(LayoutCommandType::FlowControl(FlowControlCommand::ConfigOpened));
                if let Node::List(l) = &element.children[1]
                && let Node::ListItem(config_tree) = &l.children[0]
                && let Node::List(config_commands) = &config_tree.children[1] {
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
                println!("{:#?}", element);
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
                                "set-event" => {}
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
                    layout_commands.push(LayoutCommandType::FlowControl(FlowControlCommand::ListOpened { src: list_src.value.trim().to_string() }));
                    for li in &list_content.children {
                        let mut list_element = process_element::<Event>(&li);
                        layout_commands.append(&mut list_element);
                    }
                    layout_commands.push(LayoutCommandType::FlowControl(FlowControlCommand::ListClosed));
                }
            }
            "if" => {

            }
            "if-not" => {

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
                "width-fit" => {
                    match &keys.children.len() {
                        1 => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FitX)),
                        4 => {
                            if let Node::InlineCode(i) = &keys.children[2] 
                            && i.value.as_str() == "min" 
                            && let Node::Text(min) = &keys.children[3] {
                                configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FitXmin { 
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
                                configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FitXminmax {
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
                "height-fit" => {
                    match &keys.children.len() {
                        1 => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FitY)),
                        4 => {
                            if let Node::InlineCode(i) = &keys.children[2] 
                            && i.value.as_str() == "min" 
                            && let Node::Text(min) = &keys.children[3] {
                                configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FitYmin { 
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
                                configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FitYminmax {
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
                "width-fixed" => {
                    let emphasees = keys.children.iter().filter(|node|{
                        if let Node::Emphasis(_)= node{
                            return true
                        } return false
                    }).collect::<Vec<&Node>>();

                    if emphasees.len() > 0 {
                        if let Node::Emphasis(e) = &keys.children[2] 
                        && let Node::Text(var_name) = &e.children[0] {
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FixedXFrom(var_name.value.clone())));
                        }
                    }
                    else {
                        if let Node::Text(t) = &keys.children[1]
                        && let Ok(value) = f32::from_str(&t.value.trim()){
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FixedX(value)));
                        }
                    }
                }
                "height-fixed" => {
                    let emphasees = keys.children.iter().filter(|node|{
                        if let Node::Emphasis(_)= node{
                            return true
                        } return false
                    }).collect::<Vec<&Node>>();

                    if emphasees.len() > 0 {
                        if let Node::Emphasis(e) = &keys.children[2] 
                        && let Node::Text(var_name) = &e.children[0] {
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FixedYFrom(var_name.value.clone())));
                        }
                    }
                    else {
                        if let Node::Text(t) = &keys.children[1]
                        && let Ok(value) = f32::from_str(&t.value.trim()){
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FixedY(value)));
                        }
                    }
                }
                "width-percent" => {
                    let emphasees = keys.children.iter().filter(|node|{
                        if let Node::Emphasis(_)= node{
                            return true
                        } return false
                    }).collect::<Vec<&Node>>();

                    if emphasees.len() > 0 {
                        // if let Node::Emphasis(e) = &keys.children[2] 
                        // && let Node::Text(var_name) = &e.children[0] {
                        //     configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PercentXFrom(var_name.value.clone())));
                        // }
                    }
                    else {
                        if let Node::Text(t) = &keys.children[1]
                        && let Ok(value) = f32::from_str(&t.value.trim()){
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PercentX(value)));
                        }
                    }
                }
                "height-percent" => {
                    let emphasees = keys.children.iter().filter(|node|{
                        if let Node::Emphasis(_)= node{
                            return true
                        } return false
                    }).collect::<Vec<&Node>>();

                    if emphasees.len() > 0 {
                        // if let Node::Emphasis(e) = &keys.children[2] 
                        // && let Node::Text(var_name) = &e.children[0] {
                        //     configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PercentXFrom(var_name.value.clone())));
                        // }
                    }
                    else {
                        if let Node::Text(t) = &keys.children[1]
                        && let Ok(value) = f32::from_str(&t.value.trim()){
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PercentY(value)));
                        }
                    }
                }
                "padding-all" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = u16::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PaddingAll(value)));
                    }
                }
                "padding-top" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = u16::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PaddingTop(value)));
                    }
                }
                "padding-right" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = u16::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PaddingRight(value)));
                    }
                }
                "padding-bottom" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = u16::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PaddingBottom(value)));
                    }
                }
                "padding-left" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = u16::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::PaddingLeft(value)));
                    }
                }
                "child-gap" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = u16::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::ChildGap(value)));
                    }
                }
                "direction" => {
                    if let Node::Text(t) = &keys.children[1]
                    && t.value.trim() == "ttb" {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::DirectionTTB));
                    }
                    else if let Node::Text(t) = &keys.children[1]
                    && t.value.trim() == "ltr" {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::DirectionLTR));
                    }
                }
                "align-children-x" => {
                    if let Node::Text(t) = &keys.children[1] {
                        match t.value.trim() {
                            "left" => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::ChildAlignmentXLeft)),
                            "right" => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::ChildAlignmentXRight)),
                            "center" => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::ChildAlignmentXCenter)),
                            _ => {}
                        }
                    }
                }
                "align-children-y" => {
                    if let Node::Text(t) = &keys.children[1] {
                        match t.value.trim() {
                            "top" => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::ChildAlignmentYTop)),
                            "bottom" => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::ChildAlignmentYBottom)),
                            "center" => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::ChildAlignmentYCenter)),
                            _ => {}
                        }
                    }
                }
                "color" => {
                    let emphasees = keys.children.iter().filter(|node|{
                        if let Node::Emphasis(_)= node{
                            return true
                        } return false
                    }).collect::<Vec<&Node>>();

                    if emphasees.len() > 0 {
                        if let Node::Emphasis(e) = &keys.children[2] 
                        && let Node::Text(var_name) = &e.children[0] {
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::DynamicColor(var_name.value.to_string())));
                        }
                    }
                    else {
                        if let Node::Text(t) = &keys.children[1] {
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::Color(
                                match csscolorparser::parse(&t.value) {
                                    Err(_) => Color::default(),
                                    Ok(color) => color.to_rgba8().into(),
                                }
                            )));
                        }
                    }
                }
                "radius-all" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = f32::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::RadiusAll(value)));
                    }
                }
                "radius-top-left" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = f32::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::RadiusTopLeft(value)));
                    }
                }
                "radius-top-right" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = f32::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::RadiusTopRight(value)));
                    }
                }
                "radius-bottom-left" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = f32::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::RadiusBottomLeft(value)));
                    }
                }
                "radius-bottom-right" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = f32::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::RadiusBottomRight(value)));
                    }
                }
                "border-color" => {
                    let emphasees = keys.children.iter().filter(|node|{
                        if let Node::Emphasis(_)= node{
                            return true
                        } return false
                    }).collect::<Vec<&Node>>();

                    if emphasees.len() > 0 {
                        if let Node::Emphasis(e) = &keys.children[2] 
                        && let Node::Text(var_name) = &e.children[0] {
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderDynamicColor(var_name.value.to_string())));
                        }
                    }
                    else {
                        if let Node::Text(t) = &keys.children[1] {
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderColor(
                                match csscolorparser::parse(&t.value) {
                                    Err(_) => Color::default(),
                                    Ok(color) => color.to_rgba8().into(),
                                }
                            )));
                        }
                    }
                }
                "border-all" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = f32::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderAll(value)));
                    }
                }
                "border-top" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = f32::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderTop(value)));
                    }
                }
                "border-left" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = f32::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderLeft(value)));
                    }
                }
                "border-bottom" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = f32::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderBottom(value)));
                    }
                }
                "border-right" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = f32::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderRight(value)));
                    }
                }
                "border-in-between" => {
                    if let Node::Text(t) = &keys.children[1]
                    && let Ok(value) = f32::from_str(&t.value.trim()){
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::BorderBetweenChildren(value)));
                    }
                }
                "scroll" => {
                    match &keys.children.len() {
                        3 => {
                            if let Node::InlineCode(i) = &keys.children[2] {
                                if i.value.as_str() == "x" {
                                    configs.push(LayoutCommandType::ElementConfig(ConfigCommand::Clip { vertical: false, horizontal: true }));
                                }
                                else if i.value.as_str() == "y" {
                                    configs.push(LayoutCommandType::ElementConfig(ConfigCommand::Clip { vertical: true, horizontal: false }));
                                }
                            }
                        }
                        5 => {
                            if let Node::InlineCode(x) = &keys.children[2]
                            && let Node::InlineCode(y) = &keys.children[4]
                            && x.value == "x" && y.value == "y" {
                                configs.push(LayoutCommandType::ElementConfig(ConfigCommand::Clip { vertical: true, horizontal: true }));
                            }
                        }
                        _ => {}
                    }
                }
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
                // TODO: floating
                // TODO: collapse this
                "use" => {
                    let emphasees = keys.children.iter().filter(|node|{
                        if let Node::Emphasis(_)= node{
                            return true
                        } return false
                    }).collect::<Vec<&Node>>();

                    if emphasees.len() > 0 {
                        if let Node::Emphasis(e) = &keys.children[2] 
                        && let Node::Text(t) = &e.children[0] {
                            configs.push(LayoutCommandType::ElementConfig(ConfigCommand::Use { name: t.value.trim().to_string() }));
                        }
                    }
                }
                "clicked" => {
                    if let Node::Text(t) = &keys.children[1] {
                        configs.push(LayoutCommandType::FlowControl(FlowControlCommand::ClickedOpened { 
                            event: Some(t.value.trim().to_string()) 
                        }));
                    }
                    else {
                        configs.push(LayoutCommandType::FlowControl(FlowControlCommand::ClickedOpened { 
                            event: None
                        }));
                    }
                    if let Node::List(configs_clicked) = &node_with_keys[1]{
                        configs.append(&mut process_configs(configs_clicked));
                    }
                    configs.push(LayoutCommandType::FlowControl(FlowControlCommand::ClickedClosed));
                }
                "hovered" => {
                    configs.push(LayoutCommandType::FlowControl(FlowControlCommand::HoveredOpened));
                    if let Node::List(configs_clicked) = &node_with_keys[1]{
                        configs.append(&mut process_configs(configs_clicked));
                    }
                    configs.push(LayoutCommandType::FlowControl(FlowControlCommand::HoveredClosed));
                }
                _ => {}
            }
        }
    }

    configs
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