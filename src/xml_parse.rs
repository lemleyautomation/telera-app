use std::{collections::HashMap, fmt::Debug, str::FromStr};

#[cfg(feature="parse_logger")]
use std::fs::OpenOptions;
#[cfg(feature="parse_logger")]
use std::io::Write;

use csscolorparser;

use quick_xml::events::Event as XMLEvent;
use quick_xml::reader::Reader;
use quick_xml::events::BytesStart;
use quick_xml::Decoder;

pub use strum;
pub use strum_macros::Display;
pub use strum_macros::EnumString;

use crate::toolkit;
use crate::TreeViewItem;

use crate::EventHandler;
use crate::API;

use telera_layout::{Color, ElementConfiguration, TextConfig};

#[derive(Debug, Display)]
pub enum ParserError{
    RequiredAttributeValueMissing,
    DynamicAndStaticValues,
    UnNamedPage,
    UnSpecifiedIdTag,
    ListWithoutSource,
    UnNamedReusable,
    UnnamedUseTag,
    FileNotAccessable,
    ReaderError,
    UnparsableEvent,
    UnknownTag(String),
}

#[derive(Clone, Debug, Display, PartialEq)]
pub enum LayoutCommandType<Event>
where
    Event: Clone+Debug+PartialEq
{
    FlowControl(FlowControlCommand),
    PageData(PageDataCommand<Event>),
    ElementConfig(ConfigCommand),
    TextConfig(TextConfigCommand),
}

#[derive(Clone, Debug, Display, PartialEq)]
pub enum FlowControlCommand{
    ElementOpened{id: Option<String>},
    ElementClosed,

    TextElementOpened,
    TextElementClosed,

    ConfigOpened,
    ConfigClosed,

    TextConfigOpened,
    TextConfigClosed,
    
    ListOpened{src: String},
    ListClosed,

    UseOpened{name: String},
    UseClosed,

    TKOpened{name: String, version: u32},
    TKClosed,

    // if not
    IfOpened{condition: String},
    IfNotOpened{condition: String},
    IfClosed,

    HoveredOpened,
    HoveredClosed,

    // use clay_onhover and retreive the pointerdata from it
    ClickedOpened{event: Option<String>},
    ClickedClosed,

    RightClickOpened{event: Option<String>},
    RightClickClosed,
}

#[derive(Clone, Debug, Display, PartialEq)]
pub enum PageDataCommand<Event>
where
    Event: Clone+Debug+PartialEq
{
    SetBool{local: String, to:bool},
    SetNumeric{local: String, to:f32},
    SetText{local: String, to:String},
    SetColor{local: String, to:Color},
    SetEvent{local: String, to:Event},

    GetBool{local: String, from:String},
    GetNumeric{local: String, from:String},
    GetText{local: String, from:String},
    GetImage{local: String, from:String},
    GetColor{local: String, from:String},
    GetEvent{local: String, from:String},
}

impl<Event: Clone+Debug+PartialEq> PageDataCommand<Event>{
    fn get_local(&self) -> String {
        match self {
            Self::GetBool { local, from:_ } => local.to_string(),
            Self::GetNumeric { local, from:_ } => local.to_string(),
            Self::GetText { local, from:_ } => local.to_string(),
            Self::GetImage { local, from:_ } => local.to_string(),
            Self::GetColor { local, from:_ } => local.to_string(),
            Self::GetEvent { local, from:_ } => local.to_string(),
            
            Self::SetBool { local, to:_ } => local.to_string(),
            Self::SetNumeric { local, to:_ } => local.to_string(),
            Self::SetText { local, to:_ } => local.to_string(),
            Self::SetColor { local, to:_ } => local.to_string(),
            Self::SetEvent { local, to:_ } => local.to_string(),
        }
    }
}

#[derive(Clone, Debug, Display, PartialEq)]
pub enum ConfigCommand{
    Id(String),
    DynamicId(String),
    StaticId(String),

    GrowAll,
    GrowX,
    GrowXmin{min: f32},
    GrowXminmax{min: f32, max:f32},
    GrowY,
    GrowYmin{min: f32},
    GrowYminmax{min: f32, max:f32},
    FitX,
    FitXmin{min: f32},
    FitXminmax{min: f32, max:f32},
    FitY,
    FitYmin{min: f32},
    FitYminmax{min: f32, max:f32},
    FixedX(f32),
    FixedXFrom(String),
    FixedY(f32),
    FixedYFrom(String),
    PercentX(f32),
    PercentY(f32),

    PaddingAll(u16),
    PaddingTop(u16),
    PaddingBottom(u16),
    PaddingLeft(u16),
    PaddingRight(u16),

    ChildGap(u16),

    DirectionTTB,
    DirectionLTR,

    ChildAlignmentXLeft,
    ChildAlignmentXRight,
    ChildAlignmentXCenter,
    ChildAlignmentYTop,
    ChildAlignmentYCenter,
    ChildAlignmentYBottom,

    Color(Color),
    DynamicColor(String),

    RadiusAll(f32),
    RadiusTopLeft(f32),
    RadiusTopRight(f32),
    RadiusBottomRight(f32),
    RadiusBottomLeft(f32),

    BorderColor(Color),
    BorderDynamicColor(String),
    BorderAll(f32),
    BorderTop(f32),
    BorderLeft(f32),
    BorderBottom(f32),
    BorderRight(f32),
    BorderBetweenChildren(f32),

    Clip{vertical: bool, horizontal: bool},

    Image{name: String},

    Floating,
    FloatingOffset{x:f32,y:f32,x_from:Option<String>,y_from:Option<String>},
    FloatingDimensions{width:f32,height:f32},
    FloatingZIndex{z:i16},
    FloatingAttatchToParentAtTopLeft,
    FloatingAttatchToParentAtCenterLeft,
    FloatingAttatchToParentAtBottomLeft,
    FloatingAttatchToParentAtTopCenter,
    FloatingAttatchToParentAtCenter,
    FloatingAttatchToParentAtBottomCenter,
    FloatingAttatchToParentAtTopRight,
    FloatingAttatchToParentAtCenterRight,
    FloatingAttatchToParentAtBottomRight,
    FloatingAttatchElementAtTopLeft,
    FloatingAttatchElementAtCenterLeft,
    FloatingAttatchElementAtBottomLeft,
    FloatingAttatchElementAtTopCenter,
    FloatingAttatchElementAtCenter,
    FloatingAttatchElementAtBottomCenter,
    FloatingAttatchElementAtTopRight,
    FloatingAttatchElementAtCenterRight,
    FloatingAttatchElementAtBottomRight,
    FloatingPointerPassThrough,
    FloatingAttachElementToElement{other_element_id:String},
    FloatingAttachElementToRoot,

    // todo:
    // floating elements
    // custom elements
    // custom layouts
}

pub struct ListData<'list_iteration>{
    pub src: &'list_iteration str,
    pub index: i32,
}

#[derive(Clone, Debug, Display, PartialEq)]
pub enum TextConfigCommand{
    DefaultText(String),
    FontId(u16),
    AlignRight,
    AlignLeft,
    AlignCenter,
    LineHeight(u16),
    FontSize(u16),
    Editable(bool),
    Content(String),
    DynamicContent(String),
    Color(Color),
}

#[derive(Default, Debug, Clone)]
enum ParsingMode{
    #[default]
    Normal,
    Reusable
}

#[allow(non_camel_case_types)]
#[derive(EnumString, Debug)]
enum SizeType{
    grow,
    fit,
    percent,
    fixed
}

#[allow(non_camel_case_types)]
#[derive(EnumString, PartialEq)]
enum AlignmentDirection {
    left,
    right,
    center,
    top,
    bottom,
}

trait Cdata {
    fn cdata(&self, value: &str) -> Option<String>;
}

impl Cdata for BytesStart<'_>{
    fn cdata(&self, value: &str) -> Option<String> {
        let optional_attr = match self.try_get_attribute(value) {
            Err(_) => return None,
            Ok(attr) => attr
        };
        let attr = match optional_attr {
            None => return None,
            Some(attribute) => attribute
        };
        let maybe_string = match attr.decode_and_unescape_value(Decoder {}) {
            Ok(a) => a,
            Err(_) => return None
        };
        Some(maybe_string.to_string())
    }
}

#[allow(unused_variables)]
pub trait ParserDataAccess<Image, Event: FromStr+Clone+PartialEq+Debug+EventHandler>{
    fn get_bool(&self, name: &str, list: &Option<ListData>) -> Option<bool>{
        None
    }
    fn get_numeric(&self, name: &str, list: &Option<ListData>) -> Option<f32>{
        None
    }
    fn get_list_length(&self, name: &str, list: &Option<ListData>) -> Option<i32>{
        None
    }
    fn get_text<'render_pass, 'application>(&'application self, name: &str, list: &Option<ListData>) -> Option<&'render_pass String> where 'application: 'render_pass{
        None
    }
    fn get_image<'render_pass, 'application>(&'application self, name: &str, list: &Option<ListData> ) -> Option<&'render_pass Image> where 'application: 'render_pass{
        None
    }
    fn get_color<'render_pass, 'application>(&'application self, name: &str, list: &Option<ListData> ) -> Option<&'render_pass Color> where 'application: 'render_pass{
        None
    }
    fn get_event<'render_pass, 'application>(&'application self, name: &str, list: &Option<ListData> ) -> Option<Event> where 'application: 'render_pass{
        None
    }
    fn get_treeview<'render_pass, 'application>(&'application self, name: &'render_pass str) -> Option<TreeViewItem<'render_pass, Event>> where 'application: 'render_pass {None}
}

enum SsizeType {
    None,
    Min{min: f32},
    MinMax{min: f32, max:f32},
    At{at: f32}
}

fn parse<'a, T: FromStr+Default>(name: &str, bytes_start: &'a mut BytesStart) -> (T, bool) {
    if let Some(value) = bytes_start.cdata(name) {
        match T::from_str(&value) {
            Err(_) => (T::default(), true),
            Ok(value) => (value, true)
        }
    } else {(T::default(), false)}
}

fn try_parse<'a, T:FromStr>(name: &str, bytes_start: &'a mut BytesStart) -> Option<T> {
    if let Some(value) = bytes_start.cdata(name) {
        match T::from_str(&value) {
            Err(_) => None,
            Ok(value) => Some(value)
        }
    } else {None}
}

fn set_sizing_attributes<'a>(bytes_start: &'a mut BytesStart) -> SsizeType{
    let (min, min_exists) = parse::<f32>("min", bytes_start);
    let (max, max_exists) = parse::<f32>("max", bytes_start);
    let (at, at_exists) = parse::<f32>("at", bytes_start);

    if min_exists && max_exists {
        return SsizeType::MinMax { min, max }
    }
    else if min_exists {
        return SsizeType::Min { min }
    }
    else if at_exists {
        return SsizeType::At { at };
    }
    else {
        return SsizeType::None;
    }
}

enum ValueRef{
    Dynamic(String),
    Static(String)
}

fn dyn_or_stat<'a>(e: &'a mut BytesStart) -> Result<ValueRef, ParserError>{
    let static_value = e.cdata("is");
    let dynamic_value = e.cdata("from");

    match ((static_value.is_some() as u8)*2) + (dynamic_value.is_some() as u8) {
        0 => return Err(ParserError::RequiredAttributeValueMissing),
        1 => return Ok(ValueRef::Dynamic(dynamic_value.unwrap())),
        2 => return Ok(ValueRef::Static(static_value.unwrap())),
        3 => return Err(ParserError::DynamicAndStaticValues),
        _ => return Err(ParserError::RequiredAttributeValueMissing),
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct XMLPage<Event>
where
    Event: Clone+Debug+PartialEq+FromStr,
{
    commands: Vec<LayoutCommandType<Event>>,
    reusables: HashMap<String, Vec<LayoutCommandType<Event>>>,
    editable_text: HashMap<String, u32>
}

#[derive(Default, Debug, Clone)]
pub struct Parser<Event>
where
    Event: Clone+Debug+PartialEq+FromStr,
{
    page: HashMap<String, XMLPage<Event>>,
    reusable: HashMap<String, Vec<LayoutCommandType<Event>>>,

    mode: ParsingMode,

    current_page: Vec<LayoutCommandType<Event>>,
    current_page_name: String,

    current_reusable: Vec<LayoutCommandType<Event>>,
    reusable_name: String,

    nesting_level: i32,
    xml_nesting_stack: Vec<i32>,

    text_opened: bool,
    text_content: Option<String>,
}

impl<Event> Parser<Event>
where
    Event: Clone+Debug+PartialEq+FromStr,
    <Event as FromStr>::Err: Debug,
{
    pub fn new() -> Self {
        Parser { page: HashMap::new(), reusable: HashMap::new(),  mode: ParsingMode::default(), current_page: Vec::new(), current_page_name: String::new(), current_reusable: Vec::new(), reusable_name: String::new(), nesting_level: 0, xml_nesting_stack: Vec::new(), text_opened: false, text_content: None }
    }

    pub fn add_page(xml_string: &str) -> Result<(String, Vec<LayoutCommandType<Event>>,HashMap<String, Vec<LayoutCommandType<Event>>>), ParserError>{
        let mut parser = Parser::new();

        let mut reader = Reader::from_str(xml_string);
        reader.config_mut().trim_text(true);
        let mut buf = Vec::<u8>::new();

        let mut text_opened = false;

        let mut num_elements_opened: u32 = 0;
        let mut num_elements_closed: u32 = 0;
        let mut num_configs_opened: u32 = 0;
        let mut num_configs_closed: u32 = 0;

        loop {
            match reader.read_event_into(&mut buf) {
                Err(e) => {
                    println!("Reader Error: {:?}", e);
                    return Err(ParserError::ReaderError)
                },
                Ok(XMLEvent::Eof) => break,
                Ok(XMLEvent::Start(mut e)) => {
                    parser.nest();
                    match e.name().as_ref() {
                        b"reusable" => match e.cdata("name") {
                            None => return Err(ParserError::UnNamedReusable),
                            Some(reusable_name) => parser.open_reusable(reusable_name),
                        }
                        b"page" => match e.cdata("name") {
                            None => return Err(ParserError::UnNamedPage),
                            Some(name) => {
                                //println!("page: {:?}", &name);
                                parser.current_page_name = name;
                            }
                        }
                        b"element" => {
                            num_elements_opened += 1;
                            if let Some(condition) = e.cdata("if") {
                                parser.push_nest(FlowControlCommand::IfOpened{condition});
                            }
                            if let Some(condition) = e.cdata("if-not") {
                                parser.push_nest(FlowControlCommand::IfNotOpened{condition});
                            }
                            parser.flow_control(FlowControlCommand::ElementOpened{id:e.cdata("id")});
                        }
                        b"text-element" =>      {
                            parser.flow_control(FlowControlCommand::TextElementOpened);
                            text_opened = true;
                        }
                        b"element-config" =>    {
                            num_configs_opened += 1;
                            parser.flow_control(FlowControlCommand::ConfigOpened)
                        }
                        b"text-config" =>       parser.flow_control(FlowControlCommand::TextConfigOpened),
                        b"content" =>           parser.text_content = None,
                        b"use" => match e.cdata("name") {
                            None => return Err(ParserError::UnnamedUseTag),
                            Some(fragment_name) => parser.flow_control(FlowControlCommand::UseOpened{name:fragment_name.clone()}),
                        }
                        b"tk" => match e.cdata("type") {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(name) => {
                                match try_parse::<u32>("version", &mut e) {
                                    None => parser.flow_control(FlowControlCommand::TKOpened { name: name, version: 0 }),
                                    Some(version) => parser.flow_control(FlowControlCommand::TKOpened { name: name, version: version }),
                                }
                            }
                        }
                        b"hovered" =>           parser.flow_control(FlowControlCommand::HoveredOpened),
                        b"clicked" =>           parser.flow_control(FlowControlCommand::ClickedOpened{event: e.cdata("emit")}),
                        b"right-clicked" =>     parser.flow_control(FlowControlCommand::RightClickOpened { event: e.cdata("emit") }),
                        b"list" => match e.cdata("src") {
                            None => return Err(ParserError::ListWithoutSource),
                            Some(src) => parser.flow_control(FlowControlCommand::ListOpened { src }),
                        }
                        other => return Err(ParserError::UnknownTag(unsafe{String::from_raw_parts(other.to_owned().as_mut_ptr(), other.len(), other.len()*2)}))
                    }
                }
                Ok(XMLEvent::End(e)) => {
                    parser.denest();
                    match e.name().as_ref() {
                        b"reusable" =>          parser.close_reusable(),
                        b"page" => (),
                        b"element" => {
                            num_elements_closed += 1;
                            parser.try_pop_nest()
                        }
                        b"element-config" => {
                            num_configs_closed += 1;
                            parser.flow_control(FlowControlCommand::ConfigClosed)
                        }
                        b"text-config" =>       parser.flow_control(FlowControlCommand::TextConfigClosed),
                        b"text-element" =>      {
                            parser.flow_control(FlowControlCommand::TextElementClosed);
                            text_opened = false;
                        }
                        b"content" =>           parser.close_text_content(),
                        b"use" =>               parser.flow_control(FlowControlCommand::UseClosed),
                        b"tk" =>                parser.flow_control(FlowControlCommand::TKClosed),
                        b"hovered" =>           parser.flow_control(FlowControlCommand::HoveredClosed),
                        b"clicked" =>           parser.flow_control(FlowControlCommand::ClickedClosed),
                        b"right-clicked" =>     parser.flow_control(FlowControlCommand::RightClickClosed),
                        b"list" =>              parser.flow_control(FlowControlCommand::ListClosed),
                        other => return Err(ParserError::UnknownTag(unsafe{String::from_raw_parts(other.to_owned().as_mut_ptr(), other.len(), other.len()*2)}))
                    }
                }
                Ok(XMLEvent::Empty(mut e)) => {
                    match e.name().as_ref() {
                        b"set-bool" => {
                            let local = match e.cdata("local") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            let to = match e.cdata("to") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => match bool::from_str(&value) {
                                    Err(_) => return Err(ParserError::RequiredAttributeValueMissing),
                                    Ok(value) => value
                                }
                            };
                            parser.page_data(PageDataCommand::SetBool{local, to});
                        }
                        b"set-numeric" => {
                            let local = match e.cdata("local") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            let to = match e.cdata("to") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => match f32::from_str(&value) {
                                    Err(_) => return Err(ParserError::RequiredAttributeValueMissing),
                                    Ok(value) => value
                                }
                            };
                            parser.page_data(PageDataCommand::SetNumeric{local, to});
                        }
                        b"set-text" => {
                            let local = match e.cdata("local") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            let to = match e.cdata("to") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            parser.page_data(PageDataCommand::SetText{local, to});
                        }
                        b"set-color" => {
                            let local = match e.cdata("local") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            let to = match e.cdata("to") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => match csscolorparser::parse(&value) {
                                    Err(_) => return Err(ParserError::RequiredAttributeValueMissing),
                                    Ok(value) => value.to_rgba8().into()
                                }
                            };
                            parser.page_data(PageDataCommand::SetColor{local, to});
                        }
                        b"set-event" => {
                            let local = match e.cdata("local") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            let to = match e.cdata("to") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => match Event::from_str(&value) {
                                    Err(_) => return Err(ParserError::RequiredAttributeValueMissing),
                                    Ok(value) => value
                                }
                            };
                            parser.page_data(PageDataCommand::SetEvent{local, to});
                        }
                        b"get-bool" => {
                            let local = match e.cdata("local") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            let from = match e.cdata("from") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            parser.page_data(PageDataCommand::GetBool { local, from });
                        }
                        b"get-numeric" => {
                            let local = match e.cdata("local") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            let from = match e.cdata("from") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            parser.page_data(PageDataCommand::GetNumeric { local, from });
                        }
                        b"get-text" => {
                            let local = match e.cdata("local") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            let from = match e.cdata("from") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            parser.page_data(PageDataCommand::GetText { local, from });
                        }
                        b"get-image" => {
                            let local = match e.cdata("local") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            let from = match e.cdata("from") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            parser.page_data(PageDataCommand::GetImage { local, from });
                        }
                        b"get-color" => {
                            let local = match e.cdata("local") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            let from = match e.cdata("from") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            parser.page_data(PageDataCommand::GetColor { local, from });
                        }
                        b"get-event" => {
                            let local = match e.cdata("local") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            let from = match e.cdata("from") {
                                None => return Err(ParserError::RequiredAttributeValueMissing),
                                Some(value) => value
                            };
                            parser.page_data(PageDataCommand::GetEvent { local, from });
                        }
                        b"id" => match dyn_or_stat(&mut e) {
                            Err(e) => return Err(e),
                            Ok(id) => match id {
                                ValueRef::Dynamic(dyn_id) => parser.config_command(ConfigCommand::DynamicId(dyn_id)),
                                ValueRef::Static(stat_id) => parser.config_command(ConfigCommand::StaticId(stat_id)),
                            }
                        }
                        b"grow" => parser.config_command(ConfigCommand::GrowAll),
                        b"width-fit" => match set_sizing_attributes(&mut e) {
                            SsizeType::MinMax { min, max } => parser.config_command(ConfigCommand::FitXminmax { min, max }),
                            SsizeType::Min { min } => parser.config_command(ConfigCommand::FitXmin { min }),
                            SsizeType::None => parser.config_command(ConfigCommand::FitX),
                            _ => ()
                        }
                        b"width-grow" => match set_sizing_attributes(&mut e) {
                            SsizeType::MinMax { min, max } => parser.config_command(ConfigCommand::GrowXminmax { min, max }),
                            SsizeType::Min { min } => parser.config_command(ConfigCommand::GrowXmin { min }),
                            SsizeType::None => parser.config_command(ConfigCommand::GrowX),
                            _ => ()
                        }
                        b"width-fixed" => {
                            if let SsizeType::At { at } = set_sizing_attributes(&mut e) {
                                parser.config_command(ConfigCommand::FixedX(at));
                            }
                            else if let Some(value) = e.cdata("from") {
                                parser.config_command(ConfigCommand::FixedXFrom(value));
                            }
                        }
                        b"width-percent" => if let SsizeType::At { at } = set_sizing_attributes(&mut e) {
                            parser.config_command(ConfigCommand::PercentX(at));
                        }
                        b"height-fit" => match set_sizing_attributes(&mut e) {
                            SsizeType::MinMax { min, max } => parser.config_command(ConfigCommand::FitYminmax { min, max }),
                            SsizeType::Min { min } => parser.config_command(ConfigCommand::FitYmin { min }),
                            SsizeType::None => parser.config_command(ConfigCommand::FitY),
                            _ => ()
                        }
                        b"height-grow" => match set_sizing_attributes(&mut e) {
                            SsizeType::MinMax { min, max } => parser.config_command(ConfigCommand::GrowYminmax { min, max }),
                            SsizeType::Min { min } => parser.config_command(ConfigCommand::GrowYmin { min }),
                            SsizeType::None => parser.config_command(ConfigCommand::GrowY),
                            _ => ()
                        }
                        b"height-fixed" => {
                            if let SsizeType::At { at } = set_sizing_attributes(&mut e) {
                                parser.config_command(ConfigCommand::FixedY(at));
                            }
                            else if let Some(value) = e.cdata("from") {
                                parser.config_command(ConfigCommand::FixedYFrom(value));
                            }
                        }
                        b"height-percent" => if let SsizeType::At { at } = set_sizing_attributes(&mut e) {
                            parser.config_command(ConfigCommand::PercentY(at));
                        }
                        b"padding-all" => match try_parse::<u16>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(value) => parser.config_command(ConfigCommand::PaddingAll(value)),
                        }
                        b"padding-left" => match try_parse::<u16>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(value) => parser.config_command(ConfigCommand::PaddingLeft(value)),
                        }
                        b"padding-bottom" => match try_parse::<u16>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(value) => parser.config_command(ConfigCommand::PaddingBottom(value)),
                        }
                        b"padding-right" => match try_parse::<u16>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(value) => parser.config_command(ConfigCommand::PaddingRight(value)),
                        }
                        b"padding-top" => match try_parse::<u16>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(value) => parser.config_command(ConfigCommand::PaddingTop(value)),
                        }
                        b"child-gap" => match try_parse::<u16>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(value) => parser.config_command(ConfigCommand::ChildGap(value)),
                        }
                        b"direction" =>  match e.cdata("is") {
                            Some(direction) => {
                                if &direction == "ttb" {
                                    parser.config_command(ConfigCommand::DirectionTTB);
                                }
                                else {
                                    parser.config_command(ConfigCommand::DirectionLTR);
                                }
                            }
                            None => return Err(ParserError::RequiredAttributeValueMissing)
                        }
                        b"align-children-x" => if let Some(alignment) = e.cdata("to") {
                            match AlignmentDirection::from_str(&alignment) {
                                Err(_) => return Err(ParserError::RequiredAttributeValueMissing),
                                Ok(direction) => {
                                    match direction {
                                        AlignmentDirection::left => parser.config_command(ConfigCommand::ChildAlignmentXLeft),
                                        AlignmentDirection::center => parser.config_command(ConfigCommand::ChildAlignmentXCenter),
                                        AlignmentDirection::right => parser.config_command(ConfigCommand::ChildAlignmentXRight),
                                        _ => {}
                                    }
                                }
                            }
                        }
                        b"align-children-y" => if let Some(alignment) = e.cdata("to") {
                            match AlignmentDirection::from_str(&alignment) {
                                Err(_) => return Err(ParserError::RequiredAttributeValueMissing),
                                Ok(direction) => {
                                    match direction {
                                        AlignmentDirection::top => parser.config_command(ConfigCommand::ChildAlignmentYTop),
                                        AlignmentDirection::center => parser.config_command(ConfigCommand::ChildAlignmentYCenter),
                                        AlignmentDirection::bottom => parser.config_command(ConfigCommand::ChildAlignmentYBottom),
                                        _ => {}
                                    }
                                }
                            }
                        }
                        b"color" => if let Some(color) = e.cdata("is") {
                            if text_opened {
                                match csscolorparser::parse(&color) {
                                    Err(_) => parser.text_config(TextConfigCommand::Color(Color::default())),
                                    Ok(color) => parser.text_config(TextConfigCommand::Color(color.to_rgba8().into())),
                                }
                            }
                            else {
                                match csscolorparser::parse(&color) {
                                    Err(_) => parser.config_command(ConfigCommand::Color(Color::default())),
                                    Ok(color) => parser.config_command(ConfigCommand::Color(color.to_rgba8().into())),
                                }
                            }
                        }
                        b"dyn-color" => if let Some(color) = e.cdata("from") {
                            parser.config_command(ConfigCommand::DynamicColor(color));
                        }
                        b"radius-all" => match try_parse::<f32>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(radius) => parser.config_command(ConfigCommand::RadiusAll(radius)),
                        }
                        b"radius-top-left" => match try_parse::<f32>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(radius) => parser.config_command(ConfigCommand::RadiusTopLeft(radius)),
                        }
                        b"radius-top-right" => match try_parse::<f32>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(radius) => parser.config_command(ConfigCommand::RadiusTopLeft(radius)),
                        }
                        b"radius-bottom-left" => match try_parse::<f32>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(radius) => parser.config_command(ConfigCommand::RadiusBottomLeft(radius)),
                        }
                        b"radius-bottom-right" => match try_parse::<f32>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(radius) => parser.config_command(ConfigCommand::RadiusBottomRight(radius)),
                        }
                        b"border-color" => if let Some(color) = e.cdata("is") {
                            match csscolorparser::parse(&color) {
                                Err(_) => parser.config_command(ConfigCommand::BorderColor(Color::default())),
                                Ok(color) => parser.config_command(ConfigCommand::BorderColor(color.to_rgba8().into())),
                            }
                        }
                        b"border-dynamic-color" => match e.cdata("from") {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(color) => parser.config_command(ConfigCommand::BorderDynamicColor(color)),
                        }
                        b"border-all" => match try_parse::<f32>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(radius) => parser.config_command(ConfigCommand::BorderAll(radius)),
                        }
                        b"border-top" => match try_parse::<f32>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(radius) => parser.config_command(ConfigCommand::BorderTop(radius)),
                        }
                        b"border-left" => match try_parse::<f32>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(radius) => parser.config_command(ConfigCommand::BorderLeft(radius)),
                        }
                        b"border-bottom" => match try_parse::<f32>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(radius) => parser.config_command(ConfigCommand::BorderBottom(radius)),
                        }
                        b"border-right" => match try_parse::<f32>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(radius) => parser.config_command(ConfigCommand::BorderRight(radius)),
                        }
                        b"border-between-children" => match try_parse::<f32>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(radius) => parser.config_command(ConfigCommand::BorderBetweenChildren(radius)),
                        }
                        b"scroll" => {
                            let vertical = match e.cdata("vertical") {
                                None => false,
                                Some(value) => bool::from_str(&value).unwrap()
                            };
                            let horizontal = match e.cdata("horizontal") {
                                None => false,
                                Some(value) => bool::from_str(&value).unwrap()
                            };
    
                            parser.config_command(ConfigCommand::Clip{vertical, horizontal});
                        }
                        b"image" => {
                            let name = e.cdata("src");

                            if name.is_some() {
                                let name = name.unwrap();

                                parser.config_command(ConfigCommand::Image { name });
                            }
                        }
                        // todo:
                        // - custom element
                        // - custom layout
                        b"floating" => {
                            parser.config_command(ConfigCommand::Floating);
                        }
                        b"floating-offset" => {
                            let (x, _) = parse::<f32>("x", &mut e);
                            let (y, _) = parse::<f32>("y", &mut e);
                            parser.config_command(ConfigCommand::FloatingOffset { x, y, x_from: e.cdata("x-from"), y_from: e.cdata("y-from") });
                        }
                        b"floating-size" => {
                            let (width, width_exists) = parse::<f32>("width", &mut e);
                            let (height, height_exists) = parse::<f32>("height", &mut e);
                            if width_exists && height_exists {
                                parser.config_command(ConfigCommand::FloatingDimensions { width, height });
                            }
                        }
                        b"floating-z-index" => {
                            let (z, z_exists) = parse::<i16>("z", &mut e);
                            if z_exists {
                                parser.config_command(ConfigCommand::FloatingZIndex { z });
                            }
                        }
                        b"floating-attach-to-parent" => {
                            if e.cdata("top-left").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchToParentAtTopLeft);
                                continue
                            }
                            if e.cdata("center-left").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchToParentAtCenterLeft);
                                continue
                            }
                            if e.cdata("bottom-left").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchToParentAtBottomLeft);
                                continue
                            }
                            if e.cdata("top-center").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchToParentAtTopCenter);
                                continue
                            }
                            if e.cdata("center").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchToParentAtCenter);
                                continue
                            }
                            if e.cdata("bottom-center").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchToParentAtBottomCenter);
                                continue
                            }
                            if e.cdata("top-right").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchToParentAtTopRight);
                                continue
                            }
                            if e.cdata("center-right").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchToParentAtCenterRight);
                                continue
                            }
                            if e.cdata("bottom-right").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchToParentAtBottomRight);
                                continue
                            }
                        }
                        b"floating-attach-element" => {
                            if e.cdata("top-left").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchElementAtTopLeft);
                                continue
                            }
                            if e.cdata("center-left").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchElementAtCenterLeft);
                                continue
                            }
                            if e.cdata("bottom-left").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchElementAtBottomLeft);
                                continue
                            }
                            if e.cdata("top-center").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchElementAtTopCenter);
                                continue
                            }
                            if e.cdata("center").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchElementAtCenter);
                                continue
                            }
                            if e.cdata("bottom-center").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchElementAtBottomCenter);
                                continue
                            }
                            if e.cdata("top-right").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchElementAtTopRight);
                                continue
                            }
                            if e.cdata("center-right").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchElementAtCenterRight);
                                continue
                            }
                            if e.cdata("bottom-right").is_some() {
                                parser.config_command(ConfigCommand::FloatingAttatchElementAtBottomRight);
                                continue
                            }
                        }
                        b"floating-capture-pointer" => {
                            let (state, state_exists) = parse::<bool>("state", &mut e);
                            if state_exists && !state {
                                parser.config_command(ConfigCommand::FloatingPointerPassThrough);
                            }
                        }
                        b"floating-attach-to-element" => {
                            let (other_element_id, exists) = parse::<String>("id", &mut e);
                            if exists {
                                parser.config_command(ConfigCommand::FloatingAttachElementToElement { other_element_id });
                            }
                        }
                        b"floating-attach-to-root" => {
                            parser.config_command(ConfigCommand::FloatingAttachElementToRoot);
                        }
                        b"font-id" => match try_parse::<u16>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(id) => parser.text_config(TextConfigCommand::FontId(id)),
                        }
                        b"text-align-left" => parser.text_config(TextConfigCommand::AlignLeft),
                        b"text-align-right" => parser.text_config(TextConfigCommand::AlignRight),
                        b"text-align-center" => parser.text_config(TextConfigCommand::AlignCenter),
                        b"font-size" => match try_parse::<u16>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(value) => parser.text_config(TextConfigCommand::FontSize(value)),
                        }
                        b"line-height" => match try_parse::<u16>("is", &mut e) {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(value) => parser.text_config(TextConfigCommand::LineHeight(value)),
                        }
                        b"editable" => parser.text_config(TextConfigCommand::Editable(true)),
                        b"dyn-content" => match e.cdata("from") {
                            None => return Err(ParserError::RequiredAttributeValueMissing),
                            Some(name) => parser.text_config(TextConfigCommand::DynamicContent(name)),
                        }
                        _ => {
                            return Err(ParserError::UnparsableEvent)
                        }
                    }
                }
                Ok(XMLEvent::Text(e)) => {
                    parser.receive_text_content(e.unescape().unwrap().to_string())
                },
                _ => {}
            }
        }

        if num_elements_opened != num_elements_closed{
            println!("1023 invalid");
            return Err(ParserError::ReaderError);
        }

        if num_configs_opened != num_configs_closed {
            println!("1028 invalid");
            return Err(ParserError::ReaderError);
        }
        if num_configs_opened < num_elements_opened {
            println!("invalid {:} < {:}", num_configs_opened, num_elements_opened);
            return Err(ParserError::ReaderError);
        }

        match parser.page.contains_key(&parser.current_page_name) {
            true => parser.page.remove(&parser.current_page_name),
            false => parser.page.insert(parser.current_page_name.clone(),
            XMLPage {
                    commands: parser.current_page.clone(),
                    reusables: parser.reusable.clone(),                     
                    editable_text: HashMap::new(),
                }
            )
        };
        
        Ok((parser.current_page_name, parser.current_page, parser.reusable))
    }

    fn flow_control(&mut self, command: FlowControlCommand){
        match command {
            FlowControlCommand::TextConfigOpened => self.text_opened = true,
            FlowControlCommand::TextConfigClosed => self.text_opened = false,
            _ => ()
        }
        match self.mode {
            ParsingMode::Reusable => self.current_reusable.push(LayoutCommandType::FlowControl(command)),
            ParsingMode::Normal => self.current_page.push(LayoutCommandType::FlowControl(command)),
        }
    }
    fn page_data(&mut self, command: PageDataCommand<Event>){
        match self.mode {
            ParsingMode::Reusable => self.current_reusable.push(LayoutCommandType::PageData(command)),
            ParsingMode::Normal => self.current_page.push(LayoutCommandType::PageData(command)),
        }
    }
    fn config_command(&mut self, command: ConfigCommand){
        match self.mode {
            ParsingMode::Reusable => self.current_reusable.push(LayoutCommandType::ElementConfig(command)),
            ParsingMode::Normal => self.current_page.push(LayoutCommandType::ElementConfig(command)),
        }
    }
    fn text_config(&mut self, command: TextConfigCommand){
        match self.mode {
            ParsingMode::Reusable => self.current_reusable.push(LayoutCommandType::TextConfig(command)),
            ParsingMode::Normal => self.current_page.push(LayoutCommandType::TextConfig(command)),
        }
    }
    fn push_nest(&mut self, tag: FlowControlCommand){
        self.flow_control(tag);
        self.xml_nesting_stack.push(self.nesting_level);
    }
    fn try_pop_nest(&mut self){
        self.flow_control(FlowControlCommand::ElementClosed);
        match self.xml_nesting_stack.last() {
            None => {}
            Some(saved_nesting_level) => {
                if self.nesting_level < *saved_nesting_level {
                    self.xml_nesting_stack.pop();
                    self.flow_control(FlowControlCommand::IfClosed);
                }
            }
        }
    }
    fn nest(&mut self){
        self.nesting_level += 1;
    }
    fn denest(&mut self){
        self.nesting_level -= 1;
    }
    fn receive_text_content(&mut self, content: String){
        self.text_content = Some(content);
    }
    fn close_text_content(&mut self){
        let content = self.text_content.take().unwrap();
        self.text_config(TextConfigCommand::Content(content));
    }
    fn open_reusable(&mut self, name: String){
        self.reusable_name = name;
        self.current_reusable.clear();
        self.mode = ParsingMode::Reusable;
    }
    fn close_reusable(&mut self){
        let new_fragment = self.current_reusable.clone();
        self.reusable.insert(self.reusable_name.clone(), new_fragment);
        self.mode = ParsingMode::Normal;
    }    
}

pub struct Binder<Event>
where
    Event: Clone+Debug+PartialEq+FromStr
{
    pages: HashMap<String, Vec<LayoutCommandType<Event>>>,
    reusable: HashMap<String, Vec<LayoutCommandType<Event>>>,
}

impl<Event> Binder<Event>
where 
    Event: Clone+Debug+PartialEq+FromStr,
    <Event as FromStr>::Err: Debug,
{
    pub fn new() -> Self {
        Self {
            pages: HashMap::new(),
            reusable: HashMap::new()
        }
    }

    pub fn add_page(&mut self, name: &str, page: Vec<LayoutCommandType<Event>>) {
        if self.pages.get(name).is_none() {
            self.pages.insert(name.to_string(), page);
        }
    }

    pub fn add_reusables(&mut self, name: &str, page: Vec<LayoutCommandType<Event>>) {
        if self.reusable.get(name).is_none() {
            self.reusable.insert(name.to_string(), page);
        }
    }

    pub fn replace_page(&mut self, name: &str, page: Vec<LayoutCommandType<Event>>) -> Result<(), ()> {
        if self.pages.get(name).is_some() {
            self.pages.remove(name);
            self.pages.insert(name.to_string(), page);
        }

        Err(())
    }

    pub fn replace_reusable(&mut self, name: &str, reusable: Vec<LayoutCommandType<Event>>) -> Result<(), ()> {
        if self.reusable.get(name).is_some() {
            self.reusable.remove(name);
            self.reusable.insert(name.to_string(), reusable);
        }

        Err(())
    }

    pub fn set_page<'render_pass, Image, UserApp>(
        &mut self,
        window_id: winit::window::WindowId,
        api: &mut API,
        user_app: &mut UserApp,
    )
    where 
        Image: Clone+Debug+Default+PartialEq, 
        Event: FromStr+Clone+PartialEq+Debug+EventHandler<UserApplication = UserApp>, 
        UserApp: ParserDataAccess<Image, Event>,
    {
        let page = api.viewports.get_mut(&window_id).as_mut().unwrap().page.clone();
        let mut events = Vec::<Event>::new();

        if let Some(page_commands) = self.pages.get(&page) {
            let mut command_references = Vec::<&LayoutCommandType<Event>>::new();
            for command in page_commands.iter() {
                command_references.push(command);
            }
            events = set_layout(
                api,
                &command_references,
                &self.reusable,
                None,
                None,
                &mut None,
                &mut None,
                user_app,
                events
            );
            #[cfg(feature="parse_logger")]
            println!("Page set");
        }

        for event in events {
            event.dispatch(user_app, api);
        }
    }
}

fn set_layout<'render_pass, Image, Event, UserApp>(
    api: &mut API,
    commands: &Vec<&LayoutCommandType<Event>>,
    reusables: &HashMap<String, Vec<LayoutCommandType<Event>>>,
    locals: Option<&HashMap<String, &PageDataCommand<Event>>>,
    list_data: Option<ListData>,
    append_config: &mut Option<ElementConfiguration>,
    append_text_config: &mut Option<TextConfig>,
    user_app: &UserApp,
    mut events: Vec::<Event>,
) -> Vec::<Event>
where 
    Image: Clone+Debug+Default+PartialEq, 
    Event: FromStr+Clone+PartialEq+Debug+EventHandler<UserApplication = UserApp>,
    <Event as FromStr>::Err: Debug,
    UserApp: ParserDataAccess<Image, Event>
{
    #[cfg(feature="parse_logger")]
    if let Some(list_data) = &list_data {
        println!("list src:{:?}, list index:{:?}", &list_data.src, &list_data.index);
        if let Some(locals) = locals {
            for key in locals.keys() {
                println!("{:}", key);
            }
        }
    }

    let mut nesting_level: u32 = 0;
    let mut skip: Option<u32> = None;

    let mut recursive_commands = Vec::<&LayoutCommandType<Event>>::new();
    let mut recursive_source = String::new();
    //let mut recursive_version: u32 = 0;
    let mut recursive_call_stack = HashMap::<String, &PageDataCommand<Event>>::new();
    let mut collect_recursive_declarations = false;

    let mut collect_list_commands = false;
    
    let mut config = None::<ElementConfiguration>;
    
    let mut text_config = match append_text_config.is_some() {
        false => None::<TextConfig>,
        true => *append_text_config
    };

    let mut text_content = None::<&String>;
    let mut dynamic_text_content = None::<&String>;

    #[cfg(feature="parse_logger")]
    let mut file = OpenOptions::new()
        .create(true) // Create the file if it doesn't exist
        .append(true) // Open in append mode
        .open("logs.csv").unwrap();
    #[cfg(feature="parse_logger")]
    let _ = file.write_all(format!("layout command number,layout command,skip active,nesting level,comments\n").as_bytes());

    #[allow(unused_variables)]
    for (index, command) in commands.iter().enumerate() {
        #[cfg(feature="parse_logger")]
        let _ = file.write_all(format!("{:?}/{:?},{:?},{:?},{:?}\n", index, commands.len(), command, &skip, nesting_level).as_bytes());

        if collect_list_commands {
            match command {
                LayoutCommandType::FlowControl(flow_command) if *flow_command == FlowControlCommand::ListClosed => collect_list_commands = false,
                LayoutCommandType::PageData(_) => {}
                other => {
                    collect_recursive_declarations = false;
                    recursive_commands.push(other);
                    continue;
                }
            }
        }

        match command {
            LayoutCommandType::FlowControl(control_command) => {
                match control_command {
                    FlowControlCommand::IfOpened { condition } => {
                        if skip.is_none() &&
                            !try_get_bool(condition, false, locals, |v,l|user_app.get_bool(v, l), &list_data) {
                            skip = Some(nesting_level)
                        }
                        nesting_level += 1;
                    }
                    FlowControlCommand::IfNotOpened { condition } => {
                        if skip.is_none() &&
                            try_get_bool(condition, false, locals, |v,l|user_app.get_bool(v, l), &list_data) {
                            skip = Some(nesting_level)
                        }
                        nesting_level += 1;
                    }
                    FlowControlCommand::IfClosed => {
                        nesting_level -= 1;
                        if let Some(skip_level) = skip {
                            if skip_level >= nesting_level{
                                skip = None;
                            }
                        }
                    }
                    FlowControlCommand::HoveredOpened => {
                        if skip.is_none() && !api.ui_layout.hovered() {
                            skip = Some(nesting_level);
                        }
        
                        nesting_level += 1;
                    }
                    FlowControlCommand::HoveredClosed => {
                        nesting_level -= 1;

                        if let Some(skip_level) = skip {
                            if skip_level == nesting_level{
                                skip = None;
                            }
                        }
                    }
                    FlowControlCommand::ClickedOpened { event } => {
                        //println!("event at click opened: {:?}", event);
                        if skip.is_none() {
                            skip = Some(nesting_level);

                            if api.ui_layout.hovered() && api.left_mouse_clicked {
                                skip = None;

                                // is there an emitted event
                                if let Some(event) = event {
                                    // is there a local call stack containing the event
                                    if  let Some(locals) = locals &&
                                        let Some(event) = locals.get(event)
                                    {
                                        if  let PageDataCommand::GetEvent { local, from } = event &&
                                            let Some(event) = user_app.get_event(&from, &list_data)
                                        {
                                            events.push(event);
                                        }
                                        else if let PageDataCommand::SetEvent { local, to } = event {
                                            //println!("{:?}",to);
                                            events.push(to.clone());
                                        }
                                    }
                                    else {
                                        // attempt to process it as a global event
                                        if let Ok(event) = Event::from_str(event) {
                                            events.push(event);
                                        }
                                    }
                                }
                            }
                        }
                        nesting_level += 1;
                    }
                    FlowControlCommand::ClickedClosed => {
                        nesting_level -= 1;

                        if let Some(skip_level) = skip {
                            if skip_level == nesting_level{
                                skip = None;
                            }
                        }
                    }
                    FlowControlCommand::RightClickOpened { event } => {
                        if skip.is_none() {
                            skip = Some(nesting_level);

                            if api.ui_layout.hovered() && api.right_mouse_clicked {
                                skip = None;

                                // is there an emitted event
                                if let Some(event) = event {
                                    // is there a local call stack containing the event
                                    if  let Some(locals) = locals &&
                                        let Some(event) = locals.get(event)
                                    {
                                        if  let PageDataCommand::GetEvent { local, from } = event &&
                                            let Some(event) = user_app.get_event(&from, &list_data)
                                        {
                                            events.push(event);
                                        }
                                        else if let PageDataCommand::SetEvent { local, to } = event {
                                            //println!("{:?}",to);
                                            events.push(to.clone());
                                        }
                                    }
                                    else {
                                        // attempt to process it as a global event
                                        if let Ok(event) = Event::from_str(event) {
                                            events.push(event);
                                        }
                                    }
                                }
                            }
                        }
                        nesting_level += 1;
                    }
                    FlowControlCommand::RightClickClosed => {
                        nesting_level -= 1;

                        if let Some(skip_level) = skip {
                            if skip_level == nesting_level{
                                skip = None;
                            }
                        }
                    }
                    FlowControlCommand::ListOpened { src } => {
                        nesting_level += 1;

                        if skip.is_none() {
                            recursive_source = src.to_string();
                            recursive_commands.clear();
                            recursive_call_stack.clear();
                            collect_list_commands = true;
                            collect_recursive_declarations = true;
                        }
                        
                    }
                    FlowControlCommand::ListClosed => {
                        nesting_level -= 1;

                        if skip.is_none(){
                            let list_length = user_app.get_list_length(&recursive_source, &None);
                            
                            if let Some(source) = list_length {
                                for i in 0..source {
                                    events = set_layout(
                                        api,
                                        &recursive_commands, 
                                        reusables,
                                        Some(&recursive_call_stack), 
                                        Some(ListData { src: &recursive_source, index: i }), 
                                        &mut None, 
                                        &mut None, 
                                        user_app,
                                        events
                                    );
                                }
                            }
                        }
                    }
                    FlowControlCommand::ElementOpened { id:_ } => {
                        nesting_level += 1;

                        if skip.is_none() {
                            api.ui_layout.open_element();
                        }
                    }
                    FlowControlCommand::ElementClosed => {
                        nesting_level -= 1;

                        if skip.is_none() {
                            api.ui_layout.close_element();
                        }
                    }
                    FlowControlCommand::ConfigOpened => {
                        nesting_level += 1;
        
                        if skip.is_none() {
                            config = Some(ElementConfiguration::default());
                        }
                    }
                    FlowControlCommand::ConfigClosed => {
                        nesting_level -= 1;
        
                        if skip.is_none() && append_config.is_none(){
                            let final_config = config.take().unwrap();
                            api.ui_layout.configure_element(&final_config);
                        }
                        else {
                            //println!("config actually not closed");
                        }
                    }
                    FlowControlCommand::UseOpened { name } => {
                        nesting_level += 1;

                        if skip.is_none() {
                            recursive_commands.clear();
                            recursive_call_stack.clear();
                            collect_recursive_declarations = true;
                            recursive_source = name.to_string();
                        }
                        
                    }
                    FlowControlCommand::UseClosed => {
                        nesting_level -= 1;

                        if skip.is_none() {
                            collect_recursive_declarations = false;
                            if let Some(reusable) = reusables.get(&recursive_source){
                                for command in reusable.iter() {
                                    recursive_commands.push(command);
                                }
                                if recursive_call_stack.len() > 0 {
                                    events = set_layout(
                                        api,
                                        &recursive_commands,
                                        reusables,
                                        Some(&recursive_call_stack), 
                                        None,
                                        &mut config,
                                        &mut text_config,
                                        user_app,
                                        events
                                    );
                                }
                                else {
                                    events = set_layout(
                                        api,
                                        &recursive_commands,
                                        reusables,
                                        None,
                                        None,
                                        &mut config,
                                        &mut text_config,
                                        user_app,
                                        events
                                    );
                                }
                            }
                            
                        }
                    }
                    FlowControlCommand::TKOpened { name, version } => {
                        nesting_level += 1;

                        if skip.is_none() {
                            recursive_commands.clear();
                            recursive_call_stack.clear();
                            collect_recursive_declarations = true;
                            recursive_source = name.to_string();
                            //recursive_version = *version;
                        }
                    }
                    FlowControlCommand::TKClosed => {
                        nesting_level -= 1;

                        if skip.is_none() {
                            collect_recursive_declarations = false;
                            
                            match recursive_source.as_str() {
                                "treeview" => {
                                    events = toolkit::treeview("treeview", api, user_app, events);
                                }
                                _ => {}
                            }
                        }
                    }
                    FlowControlCommand::TextElementOpened => {
                        nesting_level += 1;

                        if skip.is_none() {
                            text_config = Some(TextConfig::default());
                        }
                    }
                    FlowControlCommand::TextElementClosed => {
                        if skip.is_none() {
                            match text_config.is_some() {
                                false => panic!("invalid xml stack"),
                                true => {
                                    let final_text_config = text_config.take().unwrap();
                                    match text_content.is_some() {
                                        false => {
                                            match dynamic_text_content.is_some() {
                                                false => {
                                                    api.ui_layout.add_text_element("", &final_text_config.end(), false);
                                                }
                                                true => {
                                                    let final_dyn_content = dynamic_text_content.take().unwrap();
                                                    api.ui_layout.add_text_element(&final_dyn_content, &final_text_config.end(), false);
                                                }
                                            }
                                        }
                                        true => {
                                            api.ui_layout.add_text_element(text_content.take().unwrap(), &final_text_config.end(), false);
                                        }
                                    }
                                },
                            }
                        }
        
                        nesting_level -= 1;
                    }
                    FlowControlCommand::TextConfigOpened => {
                        nesting_level += 1;
                    }
                    FlowControlCommand::TextConfigClosed => nesting_level -= 1,
                }
            }
            LayoutCommandType::PageData(data_command) => {
                if collect_recursive_declarations {
                    let name = data_command.get_local();
                    recursive_call_stack.insert(name, data_command);
                }
            }
            LayoutCommandType::ElementConfig(config_command) => {
                if skip.is_none() {
                    let open_config = match append_config.is_some() {
                        true => append_config.as_mut().unwrap(),
                        false => config.as_mut().unwrap()
                    };
                    match config_command {
                        ConfigCommand::FitX  => open_config.x_fit().parse(),
                        ConfigCommand::FitXmin{min}  => open_config.x_fit_min(*min).parse(),
                        ConfigCommand::FitXminmax{min, max}  => open_config.x_fit_min_max(*min, *max).parse(),
                        ConfigCommand::FitY  => open_config.y_fit().parse(),
                        ConfigCommand::FitYmin{min}  => open_config.y_fit_min(*min).parse(),
                        ConfigCommand::FitYminmax{min, max}  => open_config.y_fit_min_max(*min, *max).parse(),
                        ConfigCommand::GrowX  => open_config.x_grow().parse(),
                        ConfigCommand::GrowXmin{min} => open_config.x_grow_min(*min).parse(),
                        ConfigCommand::GrowXminmax{min, max}  => open_config.x_grow_min_max(*min, *max).parse(),
                        ConfigCommand::GrowY  => open_config.y_grow().parse(),
                        ConfigCommand::GrowYmin{min} => open_config.y_grow_min(*min).parse(),
                        ConfigCommand::GrowYminmax{min, max}  => open_config.y_grow_min_max(*min, *max).parse(),
                        ConfigCommand::FixedX(x)  => open_config.x_fixed(*x).parse(),
                        ConfigCommand::FixedXFrom(value) => open_config.x_fixed(
                                try_get_numeric(
                                    value, 
                                    100.0, 
                                    locals, 
                                    |v,l | user_app.get_numeric(v,l), 
                                    &list_data
                                )
                            ).parse(),
                        ConfigCommand::FixedYFrom(value) =>open_config.y_fixed(
                                try_get_numeric(
                                    value, 
                                    100.0, 
                                    locals, 
                                    |v,l | user_app.get_numeric(v,l), 
                                    &list_data
                                )
                            ).parse(),
                        ConfigCommand::FixedY(y)  => open_config.y_fixed(*y).parse(),
                        ConfigCommand::PercentX(size)  => open_config.x_percent(*size).parse(),
                        ConfigCommand::PercentY(size)  => open_config.y_percent(*size).parse(),
                        ConfigCommand::GrowAll  => open_config.grow_all().parse(),
                        ConfigCommand::PaddingAll(padding)  => open_config.padding_all(*padding).parse(),
                        ConfigCommand::PaddingTop(padding)  => open_config.padding_top(*padding).parse(),
                        ConfigCommand::PaddingBottom(padding)  => open_config.padding_bottom(*padding).parse(),
                        ConfigCommand::PaddingLeft(padding)  => open_config.padding_left(*padding).parse(),
                        ConfigCommand::PaddingRight(padding)  => open_config.padding_right(*padding).parse(),
                        ConfigCommand::DirectionTTB  => open_config.direction(true).parse(),
                        ConfigCommand::DirectionLTR  => open_config.direction(false).parse(),
                        ConfigCommand::DynamicId(name) => {
                            if let Some(locals) = locals {
                                if let Some(data_command) = locals.get(name) {
                                    if let PageDataCommand::SetText { local:_, to } = data_command {
                                        open_config.id(&to);
                                    }
                                }
                            }
                        }
                        ConfigCommand::StaticId(label)|
                        ConfigCommand::Id(label)  => open_config.id(&label).parse(),
                        ConfigCommand::ChildGap(gap)  => open_config.child_gap(*gap).parse(),
                        ConfigCommand::ChildAlignmentXLeft  => open_config.align_children_x_left().parse(),
                        ConfigCommand::ChildAlignmentXRight  => open_config.align_children_x_right().parse(),
                        ConfigCommand::ChildAlignmentXCenter  => open_config.align_children_x_center().parse(),
                        ConfigCommand::ChildAlignmentYTop  => open_config.align_children_y_top().parse(),
                        ConfigCommand::ChildAlignmentYCenter  => open_config.align_children_y_center().parse(),
                        ConfigCommand::ChildAlignmentYBottom  => open_config.align_children_y_bottom().parse(),
                        ConfigCommand::Color(color)  => open_config.color(*color).parse(),
                        ConfigCommand::DynamicColor(color) => match locals {
                            None => match user_app.get_color(color, &list_data) {
                                None => open_config.color(Color::default()).parse(),
                                Some(color) => open_config.color(*color).parse(),
                            }
                            Some(locals) =>  match locals.get(color) {
                                None => match user_app.get_color(color, &list_data) {
                                    None => open_config.color(Color::default()).parse(),
                                    Some(color) => open_config.color(*color).parse(),
                                }
                                Some(data_command) => {
                                    match data_command {
                                        PageDataCommand::GetColor { local:_, from } =>  match user_app.get_color(from, &list_data) {
                                            None => open_config.color(Color::default()).parse(),
                                            Some(color) => open_config.color(*color).parse(),
                                        }
                                        PageDataCommand::SetColor { local:_, to } => open_config.color(*to).parse(),
                                        _ => open_config.color(Color::default()).parse(),
                                    }
                                }
                            }
                        }
                        ConfigCommand::RadiusAll(radius)  => open_config.radius_all(*radius).parse(),
                        ConfigCommand::RadiusTopLeft(radius)  => open_config.radius_top_left(*radius).parse(),
                        ConfigCommand::RadiusTopRight(radius)  => open_config.radius_top_right(*radius).parse(),
                        ConfigCommand::RadiusBottomRight(radius)  => open_config.radius_bottom_right(*radius).parse(),
                        ConfigCommand::RadiusBottomLeft(radius)  => open_config.radius_bottom_left(*radius).parse(),
                        ConfigCommand::BorderColor(color) => open_config.border_color(*color).parse(),
                        ConfigCommand::BorderDynamicColor(color) => match locals {
                            None => match user_app.get_color(color, &list_data) {
                                None => open_config.border_color(Color::default()).parse(),
                                Some(color) => open_config.border_color(*color).parse(),
                            }
                            Some(locals) =>  match locals.get(color) {
                                None => match user_app.get_color(color, &list_data) {
                                    None => open_config.border_color(Color::default()).parse(),
                                    Some(color) => open_config.border_color(*color).parse(),
                                }
                                Some(data_command) => {
                                    match data_command {
                                        PageDataCommand::GetColor { local:_, from } =>  match user_app.get_color(from, &list_data) {
                                            None => open_config.border_color(Color::default()).parse(),
                                            Some(color) => open_config.border_color(*color).parse(),
                                        }
                                        PageDataCommand::SetColor { local:_, to } => open_config.border_color(*to).parse(),
                                        _ => open_config.border_color(Color::default()).parse(),
                                    }
                                }
                            }
                        }
                        ConfigCommand::BorderAll(border)  => open_config.border_all(*border as u16).parse(),
                        ConfigCommand::BorderTop(border)  => open_config.border_top(*border as u16).parse(),
                        ConfigCommand::BorderBottom(border)  => open_config.border_bottom(*border as u16).parse(),
                        ConfigCommand::BorderLeft(border)  => open_config.border_left(*border as u16).parse(),
                        ConfigCommand::BorderRight(border)  => open_config.border_right(*border as u16).parse(),
                        ConfigCommand::BorderBetweenChildren(border)  => open_config.border_between_children(*border as u16).parse(),
                        ConfigCommand::Clip { vertical, horizontal } => {
                            let child_offset = api.ui_layout.get_scroll_offset();
                            open_config.scroll(*vertical, *horizontal, child_offset).parse()
                        }
                        ConfigCommand::Image { name } => {
                            match locals {
                                None => match user_app.get_image(name, &list_data) {
                                    None => {},
                                    Some(image) => open_config.image(image).parse(),
                                }
                                Some(locals) =>  match locals.get(name) {
                                    None => match user_app.get_image(name, &list_data) {
                                        None => {},
                                        Some(image) => open_config.image(image).parse(),
                                    }
                                    Some(data_command) => {
                                        match data_command {
                                            PageDataCommand::GetImage { local:_, from } =>  match user_app.get_image(from, &list_data) {
                                                None => {},
                                                Some(image) => open_config.image(image).parse(),
                                            }
                                            _ => {},
                                        }
                                    }
                                }
                            }
                        }
                        ConfigCommand::Floating => open_config.floating().parse(),
                        ConfigCommand::FloatingOffset { x, y, x_from, y_from } => {
                            if let Some(x_from) = x_from && let Some(y_from) = y_from {
                                open_config.floating_offset(
                                    try_get_numeric(
                                        x_from,
                                        *x,
                                        locals,
                                        |v,l|user_app.get_numeric(v, l), 
                                        &list_data
                                    ),
                                    try_get_numeric(
                                        y_from,
                                        *y,
                                        locals,
                                        |v,l|user_app.get_numeric(v, l), 
                                        &list_data
                                    )
                                ).parse()
                            }
                            else if let Some(x_from) = x_from {
                                open_config.floating_offset(
                                    try_get_numeric(
                                        x_from,
                                        *x,
                                        locals,
                                        |v,l|user_app.get_numeric(v, l),  
                                        &list_data
                                    ),
                                    *y
                                ).parse()
                            }
                            else if let Some(y_from) = y_from {
                                open_config.floating_offset(
                                    *x,
                                    try_get_numeric(
                                        y_from,
                                        *y,
                                        locals,
                                        |v,l|user_app.get_numeric(v, l),  
                                        &list_data
                                    )
                                ).parse()
                            }
                            else {
                                open_config.floating_offset(*x, *y).parse()
                            }
                        }
                        ConfigCommand::FloatingDimensions { width, height } => open_config.floating_dimensions(*width, *height).parse(),
                        ConfigCommand::FloatingZIndex { z } => open_config.floating_z_index(*z).parse(),
                        ConfigCommand::FloatingAttatchToParentAtTopLeft => open_config.floating_attach_to_parent_at_top_left().parse(),
                        ConfigCommand::FloatingAttatchToParentAtCenterLeft => open_config.floating_attach_to_parent_at_center_left().parse(),
                        ConfigCommand::FloatingAttatchToParentAtBottomLeft => open_config.floating_attach_to_parent_at_bottom_left().parse(),
                        ConfigCommand::FloatingAttatchToParentAtTopCenter => open_config.floating_attach_to_parent_at_top_center().parse(),
                        ConfigCommand::FloatingAttatchToParentAtCenter => open_config.floating_attach_to_parent_at_center().parse(),
                        ConfigCommand::FloatingAttatchToParentAtBottomCenter => open_config.floating_attach_to_parent_at_bottom_center().parse(),
                        ConfigCommand::FloatingAttatchToParentAtTopRight => open_config.floating_attach_to_parent_at_top_right().parse(),
                        ConfigCommand::FloatingAttatchToParentAtCenterRight => open_config.floating_attach_to_parent_at_center_right().parse(),
                        ConfigCommand::FloatingAttatchToParentAtBottomRight => open_config.floating_attach_to_parent_at_bottom_right().parse(),
                        ConfigCommand::FloatingAttatchElementAtTopLeft => open_config.floating_attach_element_at_top_left().parse(),
                        ConfigCommand::FloatingAttatchElementAtCenterLeft => open_config.floating_attach_element_at_center_left().parse(),
                        ConfigCommand::FloatingAttatchElementAtBottomLeft => open_config.floating_attach_element_at_bottom_left().parse(),
                        ConfigCommand::FloatingAttatchElementAtTopCenter => open_config.floating_attach_element_at_top_center().parse(),
                        ConfigCommand::FloatingAttatchElementAtCenter => open_config.floating_attach_element_at_center().parse(),
                        ConfigCommand::FloatingAttatchElementAtBottomCenter => open_config.floating_attach_element_at_bottom_center().parse(),
                        ConfigCommand::FloatingAttatchElementAtTopRight => open_config.floating_attach_element_at_top_right().parse(),
                        ConfigCommand::FloatingAttatchElementAtCenterRight => open_config.floating_attach_element_at_center_right().parse(),
                        ConfigCommand::FloatingAttatchElementAtBottomRight => open_config.floating_attach_element_at_bottom_right().parse(),
                        ConfigCommand::FloatingPointerPassThrough => open_config.floating_pointer_pass_through().parse(),
                        ConfigCommand::FloatingAttachElementToElement { other_element_id:_ } => {
                            //let id = layout_engine.get_id(other_element_id);
                            open_config.floating_attach_to_element(0).parse()
                        }
                        ConfigCommand::FloatingAttachElementToRoot => open_config.floating_attach_to_root().parse(),
                    }
                }
            }
            LayoutCommandType::TextConfig(config_command) => {
                if skip.is_none() {
                    let text_config = text_config.as_mut().unwrap();
                    match config_command {
                        TextConfigCommand::AlignCenter => text_config.alignment_center().parse(),
                        TextConfigCommand::AlignLeft => text_config.alignment_left().parse(),
                        TextConfigCommand::AlignRight => text_config.alignment_right().parse(),
                        TextConfigCommand::Color(color) => text_config.color(*color).parse(),
                        TextConfigCommand::Editable(_state) => (),
                        TextConfigCommand::Content(content) => text_content = Some(content),
                        TextConfigCommand::DefaultText(_default) => {}
                        TextConfigCommand::DynamicContent(name) => {
                            // #[cfg(feature="parse_logger")]
                            // println!("-------------------Command: Dynamic Text Content. Name: {:?}", name);
                            match locals {
                                None => match user_app.get_text(name, &list_data) {
                                    None => {
                                        text_content = None;
                                        dynamic_text_content = None;
                                    }
                                    Some(text) => dynamic_text_content = Some(text),
                                }
                                Some(locals) =>  match locals.get(name) {
                                    None => match user_app.get_text(name, &list_data) {
                                        None => {
                                            text_content = None;
                                            dynamic_text_content = None;
                                        }
                                        Some(text) => dynamic_text_content = Some(text),
                                    }
                                    Some(data_command) => {
                                        // #[cfg(feature="parse_logger")]
                                        // println!("trying to get dynamic text: {:?}", name);
                                        match data_command {
                                            PageDataCommand::SetText { local:_, to } => {
                                                dynamic_text_content = Some(to);
                                            }
                                            PageDataCommand::GetText { local:_, from } => {
                                                match user_app.get_text(from, &list_data) {
                                                    None => {
                                                        text_content = None;
                                                        dynamic_text_content = None;
                                                    }
                                                    Some(text) => dynamic_text_content = Some(text),
                                                }
                                            }
                                            _ => {
                                                text_content = None;
                                                dynamic_text_content = None;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        TextConfigCommand::FontId(id) => text_config.font_id(*id).parse(),
                        TextConfigCommand::FontSize(size) => text_config.font_size(*size).parse(),
                        TextConfigCommand::LineHeight(height) => text_config.line_height(*height).parse(),
                    }
                }
                
            }
        }
    }

    events

    // #[cfg(feature="parse_logger")]
    
}

fn try_get_numeric<Event, F>(from: &String, defualt: f32, locals: Option<&HashMap<String, &PageDataCommand<Event>>>, ua: F, list_data: &Option<ListData>) -> f32
where 
    F: Fn(&str, &Option<ListData>) -> Option<f32>,
    Event: FromStr+Clone+PartialEq+Debug,
    <Event as FromStr>::Err: Debug,
{
    if let Some(locals) = locals &&
        let Some(local) = locals.get(from) {
            
        if let PageDataCommand::GetNumeric { local:_, from } = local &&
            let Some(value) = ua(&from, &list_data) {
            value
        }
        else if let PageDataCommand::SetNumeric { local:_, to } = local {
            *to
        }
        else {
            defualt
        }
    }
    else if let Some(value) = ua(&from, &list_data) {
        value
    }
    else {
        defualt
    }
}

fn try_get_bool<Event, F>(from: &String, defualt: bool, locals: Option<&HashMap<String, &PageDataCommand<Event>>>, ua: F, list_data: &Option<ListData>) -> bool
where 
    F: Fn(&str, &Option<ListData>) -> Option<bool>,
    Event: FromStr+Clone+PartialEq+Debug,
    <Event as FromStr>::Err: Debug,
{
    if let Some(locals) = locals &&
        let Some(local) = locals.get(from) {
            
        if let PageDataCommand::GetBool { local:_, from } = local &&
            let Some(value) = ua(&from, &list_data) {
            value
        }
        else if let PageDataCommand::SetBool { local:_, to } = local {
            *to
        }
        else {
            defualt
        }
    }
    else if let Some(value) = ua(&from, &list_data) {
        value
    }
    else {
        defualt
    }
}