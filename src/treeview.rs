use std::str::FromStr;
use std::fmt::Debug;

use crate::EventHandler;


#[derive(Clone)]
pub enum TreeViewItem<'frame, UserEvent: FromStr+Clone+PartialEq+Debug+EventHandler>{
    EmptyRoot{label: &'frame str, left_clicked: Option<UserEvent>, right_clicked: Option<UserEvent>},
    Root{label: &'frame str, items: Vec<TreeViewItem<'frame, UserEvent>>},

    EmptyItem{label: &'frame str},
    CollapsedItem{label: &'frame str, items: Vec<TreeViewItem<'frame, UserEvent>>},
    ExpandedItem{label: &'frame str, items: Vec<TreeViewItem<'frame, UserEvent>>},

    EmptyLastItem{label: &'frame str},
    CollapsedLastItem{label: &'frame str, items: Vec<TreeViewItem<'frame, UserEvent>>},
    ExpandedLastItem{label: &'frame str, items: Vec<TreeViewItem<'frame, UserEvent>>},
}
