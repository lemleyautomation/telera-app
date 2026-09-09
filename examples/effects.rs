use std::time::Instant;
use telera_app::*;

/// Acceptance harness for the UI effect-shader system (`examples/layouts/Effects.md`).
///
/// Shows every built-in effect - `drop-shadow`, `raised-edge`, `inner-glow` -
/// on rectangles and on a `circle`, plus two custom shaders loaded with a
/// `#### TML` `` `shader` `` header directive (`examples/shaders/glass.wgsl`,
/// `examples/shaders/neon.wgsl`). Four values are animated off a wall clock and
/// bound into the layout (`*shadow_blur*`, `*bevel_angle*`, `*sheen_phase*`,
/// `*neon_glow*`), so the `commands_fingerprint` repaint path is exercised too.
#[derive(LayoutRunnerReflection)]
struct EffectsApp {
    /// `drop-shadow` blur radius, px - pulses 4..18.
    shadow_blur: f32,
    /// `raised-edge` light angle, degrees - rotates.
    bevel_angle: f32,
    /// `glass` custom shader sheen phase - advances.
    sheen_phase: f32,
    /// `neon` custom shader glow intensity - pulses 0.3..1.4.
    neon_glow: f32,
    start: Instant,
}

impl Default for EffectsApp {
    fn default() -> Self {
        Self {
            shadow_blur: 10.0,
            bevel_angle: 225.0,
            sheen_phase: 0.0,
            neon_glow: 0.8,
            start: Instant::now(),
        }
    }
}

#[telera_app]
impl EffectsApp {}

impl App for EffectsApp {
    fn initialize(&mut self) -> Startup {
        Startup {
            window_attributes: Window::default_attributes()
                .with_inner_size(LogicalSize::new(1000, 700)),
            window_name: "Effects".to_string(),
            page: None,
            watch_path: RunType::Watch("examples/layouts".to_string()),
        }
    }

    fn onload(&mut self, api: &mut API) {
        api.set_viewport_continuous("Effects", true);
    }

    fn update(&mut self, _api: &mut API) {
        let t = self.start.elapsed().as_secs_f32();
        let tri = |speed: f32| ((t * speed).fract() * 2.0 - 1.0).abs();
        self.shadow_blur = 4.0 + tri(0.35) * 14.0;
        self.bevel_angle = (t * 60.0) % 360.0;
        self.sheen_phase = t * 1.5;
        self.neon_glow = 0.3 + tri(0.6) * 1.1;
    }
}

fn main() {
    run::<EffectsApp>(EffectsApp::default());
}
