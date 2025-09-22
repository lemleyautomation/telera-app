use std::{fmt::Debug, str::FromStr};
use strum_macros::Display;
use telera_layout::Color;

use crate::{EventHandler, TreeViewItem, UIImageDescriptor};

#[derive(Clone, Debug, Display, PartialEq)]
pub enum Layout<Event>
where
    Event: Clone+Debug+PartialEq
{
    Element(Element<Event>),
    Declaration{name:String, value:DataSrc<Declaration<Event>>},
    Config(Config),
}

#[derive(Clone, Debug, Display, PartialEq)]
pub enum Element<Event>
where
    Event: Clone+Debug+PartialEq
{
    ElementOpened{id: Option<DataSrc<String>>},
    ElementClosed,

    TextElementOpened,
    TextElementClosed(DataSrc<String>),

    ConfigOpened,
    ConfigClosed,

    TextConfigOpened,
    TextConfigClosed,
    
    ListOpened{src: String},
    ListClosed,

    UseOpened{name: String},
    UseClosed,

    TreeViewOpened{name: String},
    TreeViewClosed,

    TextBoxOpened{name: String},
    TextBoxClosed,

    // if not
    IfOpened{condition: String},
    IfNotOpened{condition: String},
    IfClosed,

    Pointer(winit::window::CursorIcon),

    HoverOpened{event: Option<DataSrc<Event>>},
    HoverClosed,

    HoveredOpened{event: Option<DataSrc<Event>>},
    HoveredClosed,

    UnHoveredOpened{event: Option<DataSrc<Event>>},
    UnHoveredClosed,

    FocusOpened{event: Option<DataSrc<Event>>},
    FocusClosed,

    FocusedOpened{event: Option<DataSrc<Event>>},
    FocusedClosed,

    UnFocusedOpened{event: Option<DataSrc<Event>>},
    UnFocusedClosed,

    LeftPressedOpened{event: Option<DataSrc<Event>>},
    LeftPressedClosed,

    LeftDownOpened{event: Option<DataSrc<Event>>},
    LeftDownClosed,

    LeftReleasedOpened{event: Option<DataSrc<Event>>},
    LeftReleasedClosed,

    LeftClickedOpened{event: Option<DataSrc<Event>>},
    LeftClickedClosed,

    LeftDoubleClickedOpened{event: Option<DataSrc<Event>>},
    LeftDoubleClickedClosed,

    LeftTripleClickedOpened{event: Option<DataSrc<Event>>},
    LeftTripleClickedClosed,

    RightPressedOpened{event: Option<DataSrc<Event>>},
    RightPressedClosed,

    RightDownOpened{event: Option<DataSrc<Event>>},
    RightDownClosed,

    RightReleasedOpened{event: Option<DataSrc<Event>>},
    RightReleasedClosed,

    RightClickedOpened{event: Option<DataSrc<Event>>},
    RightClickedClosed,
}

#[derive(Clone, Debug, Display, PartialEq)]
pub enum Config{
    Id(DataSrc<String>),

    GrowAll,
    GrowX,
    GrowXmin(DataSrc<f32>),
    GrowXmax(DataSrc<f32>),
    GrowXminmax{min:DataSrc<f32>,max:DataSrc<f32>},
    GrowY,
    GrowYmin(DataSrc<f32>),
    GrowYmax(DataSrc<f32>),
    GrowYminmax{min:DataSrc<f32>,max:DataSrc<f32>},
    FitX,
    FitXmin(DataSrc<f32>),
    FitXmax(DataSrc<f32>),
    FitXminmax{min:DataSrc<f32>,max:DataSrc<f32>},
    FitY,
    FitYmin(DataSrc<f32>),
    FitYmax(DataSrc<f32>),
    FitYminmax{min:DataSrc<f32>,max:DataSrc<f32>},
    FixedX(DataSrc<f32>),
    FixedY(DataSrc<f32>),
    PercentX(DataSrc<f32>),
    PercentY(DataSrc<f32>),

    PaddingAll(DataSrc<u16>),
    PaddingTop(DataSrc<u16>),
    PaddingBottom(DataSrc<u16>),
    PaddingLeft(DataSrc<u16>),
    PaddingRight(DataSrc<u16>),

    ChildGap(DataSrc<u16>),

    Vertical,

    ChildAlignmentXLeft,
    ChildAlignmentXRight,
    ChildAlignmentXCenter,
    ChildAlignmentYTop,
    ChildAlignmentYCenter,
    ChildAlignmentYBottom,

    Color(DataSrc<Color>),

    RadiusAll(DataSrc<f32>),
    RadiusTopLeft(DataSrc<f32>),
    RadiusTopRight(DataSrc<f32>),
    RadiusBottomRight(DataSrc<f32>),
    RadiusBottomLeft(DataSrc<f32>),

    BorderColor(DataSrc<Color>),
    BorderAll(DataSrc<u16>),
    BorderTop(DataSrc<u16>),
    BorderLeft(DataSrc<u16>),
    BorderBottom(DataSrc<u16>),
    BorderRight(DataSrc<u16>),
    BorderBetweenChildren(DataSrc<u16>),

    Clip{vertical: DataSrc<bool>, horizontal: DataSrc<bool>},

    Image{name: String},

    Floating,
    FloatingOffset{x:DataSrc<f32>,y:DataSrc<f32>},
    FloatingDimensions{width:DataSrc<f32>,height:DataSrc<f32>},
    FloatingZIndex{z:DataSrc<i16>},
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

    Use{name: String},

    FontId(DataSrc<u16>),
    AlignRight,
    AlignLeft,
    AlignCenter,
    LineHeight(DataSrc<u16>),
    FontSize(DataSrc<u16>),
    FontColor(DataSrc<Color>),
    Editable(bool),
}

#[derive(Debug)]
pub struct ListData<'list_iteration>{
    pub src: &'list_iteration str,
    pub index: i32,
}

#[derive(Clone, Debug, Display, PartialEq)]
pub enum Declaration<Event>
where
    Event: Clone+Debug+PartialEq
{
    Bool(bool),
    Numeric(f32),
    Text(String),
    Color(Color),
    Event(Event),
    Image(String)
}

#[derive(Clone, Debug, Display, PartialEq)]
pub enum DataSrc<T> {
    Static(T),
    Dynamic(String)
}

#[allow(unused_variables)]
pub trait ParserDataAccess<Event: FromStr+Clone+PartialEq+Debug+EventHandler>{
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
    fn get_image<'render_pass, 'application>(&'application self, name: &str, list: &Option<ListData> ) -> Option<&'render_pass UIImageDescriptor> where 'application: 'render_pass{
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
