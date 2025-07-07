
#[derive(Default)]
struct TreeViewIcons {
    root_empty: UIImageDescriptor,
    root_hidden: UIImageDescriptor,
    root_expanded: UIImageDescriptor,
    item_hidden_last: UIImageDescriptor,
    item_expanded_last: UIImageDescriptor,
    item_hidden: UIImageDescriptor,
    item_expanded: UIImageDescriptor,
    item_empty: UIImageDescriptor,
    item_empty_last: UIImageDescriptor,
    extension: UIImageDescriptor
}

#[derive(Default)]
enum TreeViewItemType{
    #[default]
    EmptyRoot,
    CollapsedRoot,
    ExpandedRoot,

    EmptyItem,
    CollapsedItem,
    ExpandedItem,

    EmptyLastItem,
    CollapsedLastItem,
    ExpandedLastItem,
}

#[derive(Default)]
struct TreeViewItem {
    item_type: TreeViewItemType,
    sub_items: Vec<TreeViewItem>,
}
