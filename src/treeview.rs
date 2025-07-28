//use crate::UIImageDescriptor;


// #[derive(Default)]
// pub struct TreeViewIcons {
//     root_empty: UIImageDescriptor,
//     root_hidden: UIImageDescriptor,
//     root_expanded: UIImageDescriptor,
//     item_hidden_last: UIImageDescriptor,
//     item_expanded_last: UIImageDescriptor,
//     item_hidden: UIImageDescriptor,
//     item_expanded: UIImageDescriptor,
//     item_empty: UIImageDescriptor,
//     item_empty_last: UIImageDescriptor,
//     extension: UIImageDescriptor
// }

#[derive(Clone)]
pub enum TreeViewItem<'frame>{
    EmptyRoot{label: &'frame str},
    Root{label: &'frame str, items: Vec<TreeViewItem<'frame>>},

    EmptyItem{label: &'frame str},
    CollapsedItem{label: &'frame str, items: Vec<TreeViewItem<'frame>>},
    ExpandedItem{label: &'frame str, items: Vec<TreeViewItem<'frame>>},

    EmptyLastItem{label: &'frame str},
    CollapsedLastItem{label: &'frame str, items: Vec<TreeViewItem<'frame>>},
    ExpandedLastItem{label: &'frame str, items: Vec<TreeViewItem<'frame>>},
}
