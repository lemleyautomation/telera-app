use std::{collections::HashMap, str::FromStr};

use markdown::mdast::{List, Node};
use telera_app::{ConfigCommand, LayoutCommandType, FlowControlCommand};
use telera_layout::Color;
use csscolorparser;

fn main(){
    if let Ok(file) = std::fs::read_to_string("src/layouts/main.md")
    && let Ok(m) = markdown::to_mdast(&file, &markdown::ParseOptions::default())
    && let Some(children) = m.children() {
        println!("{:#?}", children[0]);
        //let _ = process_layout(children);
    }
}

#[derive(Debug)]
enum ParsingMode {
    None,
    Body,
    ReusableElements,
    ReusableConfig,
}

fn process_layout(nodes: &Vec<Node>) -> Result<(), String> {
    let mut parsing_mode = ParsingMode::None;
    //let mut page_name = "".to_string();
    let mut open_reuseable_name = "".to_string();
    let mut reusables = HashMap::<String, Vec<LayoutCommandType<()>>>::new();
    for node in nodes {
        match node {
            Node::Heading(h) => {
                if let Node::Text(t) = &h.children[0] {
                    if h.depth == 1 {
                        parsing_mode = ParsingMode::Body;
                        //page_name = t.value.clone();
                    }
                    else if h.depth == 3 {
                        parsing_mode = ParsingMode::ReusableElements;
                        open_reuseable_name = t.value.clone();
                    }
                    else if h.depth == 4 {
                        parsing_mode = ParsingMode::ReusableConfig;
                        open_reuseable_name = t.value.clone();
                    }
                    else {
                        parsing_mode = ParsingMode::None;
                    }
                }
            }
            Node::List(list) => {
                match parsing_mode {
                    ParsingMode::ReusableConfig => {
                        let reusable_items = process_element_config(list);
                        println!("{:#?}", reusable_items);
                        reusables.insert(open_reuseable_name.clone(), reusable_items);
                    }
                    ParsingMode::ReusableElements => {
                        // for child in &list.children {
                        //     process_element(child);
                        // }
                    }
                    _ => return Err("Invalid File".to_string())
                }
            }
            _ => {}
        }
        println!("mode: {:?}", parsing_mode);
    }
    Ok(())
}

fn process_element_config(list: &List) -> Vec<LayoutCommandType<()>> {
    let mut configs = Vec::new();

    for item in &list.children {
        if let Some(children) = item.children()
        && let Node::Paragraph(p) = &children[0]
        && let Node::InlineCode(i) = &p.children[0] {
            match i.value.as_str() {
                "width-grow" => configs.push(LayoutCommandType::ElementConfig(ConfigCommand::GrowX)),
                "height-fit-min" => {
                    if let Node::Text(t) = &p.children[1] {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::FitYmin { 
                            min: match f32::from_str(&t.value.trim()) {
                                Ok(f) => f,
                                Err(_) => 0.0
                            }
                        }));
                    }
                }
                "color" => {
                    if let Node::Text(t) = &p.children[1] {
                        configs.push(LayoutCommandType::ElementConfig(ConfigCommand::Color(
                            match csscolorparser::parse(&t.value) {
                                Err(_) => Color::default(),
                                Ok(color) => color.to_rgba8().into(),
                            }
                        )));
                    }
                }
                "clicked" => {
                    if let Node::Text(t) = &p.children[1] {
                        configs.push(LayoutCommandType::FlowControl(FlowControlCommand::ClickedOpened { 
                            event: Some(t.value.trim().to_string()) 
                        }));
                    }
                    else {
                        configs.push(LayoutCommandType::FlowControl(FlowControlCommand::ClickedOpened { 
                            event: None
                        }));
                    }
                    if let Node::List(configs_clicked) = &children[1]{
                        configs.append(&mut process_element_config(configs_clicked));
                    }
                    configs.push(LayoutCommandType::FlowControl(FlowControlCommand::ClickedClosed));
                }
                _ => {}
            }
        }
    }

    configs
}