use telera_app::*;

#[derive(Clone, Debug, Default, FieldAccess)]
pub struct Document {
    pub title: String,
    pub contents: String,
}

#[derive(LayoutRunnerReflection)]
struct LayoutApp {
    #[list_click_event(DocumentClicked)]
    documents: Vec<Document>,
    selected_document: usize,
    file_menu_opened: bool,
    #[allow(dead_code)]
    search_bar: String,
}

#[telera_app]
impl LayoutApp {
    #[layout_event]
    fn file_button_clicked(&mut self, _context: Option<EventContext>, _api: &mut API) {
        self.file_menu_opened = !self.file_menu_opened;
    }

    #[layout_event]
    fn document_clicked(&mut self, context: Option<EventContext>, _api: &mut API) {
        if let Some(event_context) = context
            && let Some(index) = event_context.list_index
        {
            self.selected_document = index;
        }
    }
}

impl App for LayoutApp {
    fn initialize(&mut self) -> Startup {
        Startup {
            window_attributes: Window::default_attributes()
                .with_inner_size(LogicalSize::new(900, 600)),
            window_name: "Main".to_string(),
            page: None,
            watch_path: RunType::Watch("src/layouts".to_string()),
        }
    }
}

fn main() {
    let documents = vec![
        Document {
            title: "Squirrels".to_string(),
            contents: "Squirrels are small, quick-witted acrobats: powerful hind legs, a counterbalancing tail, and a scatter-hoarding memory for thousands of hidden caches.".to_string(),
        },
        Document {
            title: "Lorem Ipsum".to_string(),
            contents: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.".to_string(),
        },
    ];

    let app = LayoutApp {
        documents,
        selected_document: 0,
        file_menu_opened: false,
        search_bar: String::new(),
    };

    run::<LayoutApp>(app);
}
