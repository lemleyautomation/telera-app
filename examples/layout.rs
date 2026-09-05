use telera_app::*;

#[derive(Clone, Debug, Default, FieldAccess)]
pub struct Document {
    pub title: String,
    pub contents: String,
}

#[derive(Clone, Debug, Default, PartialEq, strum_macros::EnumString, EventHandler)]
#[handler_for(LayoutApp)]
enum Event {
    #[default]
    None,
    FileButtonClicked,
    DocumentClicked,
}

fn file_button_clicked_handler(
    app: &mut LayoutApp,
    _context: Option<EventContext>,
    _api: &mut API<Event, LayoutApp>,
) {
    app.file_menu_opened = !app.file_menu_opened;
}

fn document_clicked_handler(
    app: &mut LayoutApp,
    context: Option<EventContext>,
    _api: &mut API<Event, LayoutApp>,
) {
    // `left-clicked *Clicked*` fires from inside `list Documents`, so
    // `Binder`/`set_layout` stamps the item's index into `EventContext::code`
    // for us - see `get_event` below for the other half of this.
    if let Some(EventContext {
        code: Some(index), ..
    }) = context
    {
        app.selected_document = index as usize;
    }
}

#[derive(LayoutRunnerReflection)]
#[event_handler(Event)]
struct LayoutApp {
    #[list_click_event(DocumentClicked)]
    documents: Vec<Document>,
    selected_document: usize,
    file_menu_opened: bool,
    #[allow(dead_code)]
    search_bar: String,
}

// `run_layout` requires this even though `src/layouts/main.md` has no `fn`
// elements of its own - the default `dispatch` is a no-op, so there's
// nothing to write here. See `examples/custom_element.rs` for an app that
// actually uses `fn`.
impl LayoutRunnerCustomElements<Event, LayoutApp> for LayoutApp {}

impl App for LayoutApp {
    type Event = Event;

    fn initialize(&mut self) -> Startup {
        Startup {
            initial_window: Window::default_attributes()
                .with_inner_size(LogicalSize::new(900, 600)),
            window_name: "Main".to_string(),
            watch_path: RunType::Watch("src/layouts".to_string()),
        }
    }

    fn layout(&mut self, page: &str, api: &mut API<Event, LayoutApp>, mt: &mut MT) {
        api.run_layout(page, mt, self);
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
