use image::load_from_memory;
use telera_app::*;

/// `Image Viewer.md` shows all three ways a layout gets an image:
///
/// * `` `load` [pic](examples/pic.jpg) `` in the header - the parser asks the
///   API to stage the file, no Rust involved.
/// * `` `set-image` *family* *pic* [..] `` / `` `image` *pic* [..] `` - a
///   `UIImageDescriptor` built in the layout from a loaded atlas.
/// * `` `image` *from_the_app* `` - still resolved against this struct's
///   `get_image`, exactly as before.
#[derive(LayoutRunnerReflection)]
struct MyApp {
    from_the_app: UIImageDescriptor,
}

#[telera_app]
impl MyApp {}

impl App for MyApp {
    fn initialize(&mut self) -> Startup {
        Startup {
            window_attributes: Window::default_attributes()
                .with_inner_size(LogicalSize::new(900, 600)),
            window_name: "Image Viewer".to_string(),
            page: None,
            watch_path: RunType::Watch("examples/layouts".to_string()),
        }
    }

    fn onload(&mut self, api: &mut API) {
        let picture = load_from_memory(include_bytes!("pic.jpg")).unwrap();
        api.add_image("from_the_app", picture);
    }
}

fn main() {
    run::<MyApp>(MyApp {
        from_the_app: UIImageDescriptor::from_atlas("from_the_app"),
    });
}
