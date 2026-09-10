use telera_app::*;

/// A `` `canvas` `` element (see `examples/layouts/Canvas.md`): a clipped,
/// pannable, zoomable surface in the middle of the window, with a sidebar
/// checklist and three floating cards laid out on it.
///
/// The framework only *stores* a canvas's pan/zoom - the app drives it. Here
/// `update` reads the mouse: the scroll wheel zooms toward the cursor and
/// right-drag pans. Everything on the canvas re-rasterizes crisply as it
/// scales, because the canvas lays its content out at the zoomed size rather
/// than stretching a texture.
#[derive(LayoutRunnerReflection, Default)]
struct CanvasApp {
    /// The status line under the canvas, shown as `*hint*`.
    hint: String,
    /// Bumped by clicking any card - proves hit-testing follows the pan/zoom.
    card_clicks: u32,
    clicks_label: String,
}

#[telera_app]
impl CanvasApp {
    #[layout_event]
    fn bump(&mut self, _ctx: Option<EventContext>, _api: &mut API) {
        self.card_clicks += 1;
    }
}

impl App for CanvasApp {
    fn initialize(&mut self) -> Startup {
        Startup {
            window_attributes: Window::default_attributes()
                .with_inner_size(LogicalSize::new(1100, 720)),
            window_name: "Canvas".to_string(),
            page: None,
            watch_path: RunType::Watch("examples/layouts".to_string()),
        }
    }

    fn onload(&mut self, api: &mut API) {
        // The `canvas` element creates this itself on the first frame; doing it
        // here lets us set the zoom limits before that frame runs.
        api.add_canvas("board");
        if let Some(c) = api.canvas("board") {
            c.set_zoom_limits(0.35, 4.0);
        }
    }

    fn update(&mut self, viewport: Option<&str>, api: &mut API) {
        // The second `update` of each frame is the one carrying window input.
        self.clicks_label = format!("clicked {} times", self.card_clicks);

        if viewport.is_none() {
            return;
        }

        let (mx, my) = api.mouse_position();
        let (_, wheel) = api.scroll_delta();
        let panning = api.right_mouse_down();
        let (dx, dy) = api.mouse_delta();

        if let Some(canvas) = api.canvas("board") {
            if wheel != 0.0 {
                // Exponential: each wheel notch is a fixed ratio, sign handled
                // for free, anchored on the cursor.
                canvas.zoom_at_screen(1.15_f32.powf(wheel), mx, my);
            }
            if panning {
                canvas.pan_by(dx, dy);
            }
            self.hint = format!(
                "zoom {:.2}x    pan ({:.0}, {:.0})    -    scroll wheel to zoom, right-drag to pan",
                canvas.zoom, canvas.pan_x, canvas.pan_y
            );
        }
    }
}

fn main() {
    run::<CanvasApp>(CanvasApp::default());
}
