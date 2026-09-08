use std::{path::PathBuf, str::FromStr, time::Instant};

use telera_app::*;

/// Two `render-window` panes into the one spinning-container scene:
/// - `orbit` is driven from `update()` with the `Camera` convenience methods
///   (`orbit` / `zoom`) off `api.camera("orbit")`;
/// - `front` is an orthographic view positioned entirely by `render-window`
///   config keywords in `scene.md` (`ortho-height`, `eye-z *front_z*`).
#[derive(LayoutRunnerReflection, Default)]
struct MyApp {
    dt: Option<Instant>,
    dt_samples: [u16; 20],
    dt_samples_index: u8,
    fps: String,
    /// Bound into `scene.md` as the `front` render-window's `eye-z`.
    front_z: f32,
    start: Option<Instant>,
    prev_t: f32,
}

#[telera_app]
impl MyApp {}

impl App for MyApp {
    fn initialize(&mut self) -> Startup {
        Startup {
            window_attributes: Window::default_attributes()
                .with_inner_size(LogicalSize::new(1100, 600)),
            window_name: "scene".to_string(),
            page: None,
            watch_path: RunType::Watch("examples/layouts".to_string()),
        }
    }

    fn onload(&mut self, api: &mut API) {
        api.set_viewport_continuous("scene", true);

        // `front` is created implicitly by its render-window; `orbit` we drive
        // ourselves, so declare it and frame the scene once here.
        api.add_camera("orbit");
        if let Some(camera) = api.camera("orbit") {
            camera
                .clip_planes(1.0, 6000.0)
                .perspective(45.0)
                .frame(0.0, 120.0, 0.0, 900.0);
        }

        let path = PathBuf::from_str("examples/models/Cargo Container.gltf").unwrap();
        let _ = api.load_gltf_model("container", path, None);
        let mut transform = Transform::new();
        transform.position.x += 100.0;
        api.add_instance("container", "other", Some(transform));
    }

    fn update(&mut self, api: &mut API) {
        let t = self.start.get_or_insert_with(Instant::now).elapsed().as_secs_f32();

        // `api.dt()` is 0 outside a redraw, so measure our own step.
        let dt = (t - self.prev_t).clamp(0.0, 0.1);
        self.prev_t = t;

        // `front` pane: the orthographic camera pulls back and pushes in.
        self.front_z = 1400.0 + 700.0 * (t * 0.6).sin();

        // `orbit` pane: spin around the scene and breathe in/out - all via the
        // `Camera` convenience methods.
        if let Some(camera) = api.camera("orbit") {
            camera
                .orbit(dt * 0.5, (t * 0.7).sin() * dt * 0.4)
                .zoom((t * 1.3).sin() * dt);
        }

        match self.dt {
            None => self.dt = Some(Instant::now()),
            Some(instant) => {
                if instant.elapsed().as_millis() >= 15 {
                    let anim_dt = instant.elapsed().as_millis() as f32;

                    if let Ok(transform) = api.transform_model("container") {
                        transform.rotate_y_axis(0.02 * anim_dt);
                    }

                    let dt_sample = (1.0 / instant.elapsed().as_secs_f64()) as u16;
                    if let Some(sample) = self.dt_samples.get_mut(self.dt_samples_index as usize) {
                        *sample = dt_sample;
                    }
                    self.dt_samples_index =
                        (self.dt_samples_index + 1) % self.dt_samples.len() as u8;
                    let averaged: f32 = self.dt_samples.iter().map(|&s| s as f32).sum::<f32>()
                        / self.dt_samples.len() as f32;

                    self.dt = Some(Instant::now());
                    self.fps = format!("frames /s: {:}", averaged as u16);
                }
            }
        }
    }
}

fn main() {
    run::<MyApp>(MyApp {
        ..Default::default()
    });
}
