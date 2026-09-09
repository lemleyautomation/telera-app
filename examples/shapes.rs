use std::time::Instant;
use telera_app::*;

/// Draws one of every `ui_renderer` custom shape from a markdown layout, plus a
/// small RPM-style gauge built from an `arc` nested inside an `arc` (a custom
/// element with children). `gauge_end` sweeps every frame and is bound into the
/// layout as the inner arc's `end-angle`, exercising the `CustomElementSpec` ->
/// `CustomElement` dynamic-resolution path through a nested shape.
#[derive(LayoutRunnerReflection)]
struct ShapesApp {
    /// Inner arc's `end-angle`, degrees: sweeps between the gauge's endpoints
    /// (`start-angle` 130 .. `end-angle` 410).
    gauge_end: f32,
    start: Instant,
}

impl Default for ShapesApp {
    fn default() -> Self {
        Self {
            gauge_end: 130.0,
            start: Instant::now(),
        }
    }
}

#[telera_app]
impl ShapesApp {}

impl App for ShapesApp {
    fn initialize(&mut self) -> Startup {
        Startup {
            window_attributes: Window::default_attributes()
                .with_inner_size(LogicalSize::new(900, 600)),
            window_name: "shapes".to_string(),
            page: None,
            watch_path: RunType::Watch("examples/layouts".to_string()),
        }
    }

    fn onload(&mut self, api: &mut API) {
        api.set_viewport_continuous("shapes", true);
    }

    fn update(&mut self, _viewport: Option<&str>, _api: &mut API) {
        // `api.dt()` isn't meaningful outside a redraw, so drive the sweep
        // straight off a wall clock: a 0..1 triangle wave over the 280-degree
        // gauge span.
        let t = self.start.elapsed().as_secs_f32();
        let frac = ((t * 0.5).fract() * 2.0 - 1.0).abs();
        self.gauge_end = 130.0 + frac * 280.0;
    }
}

fn main() {
    run::<ShapesApp>(ShapesApp::default());
}
