use std::time::Instant;
use telera_app::*;

#[derive(LayoutRunnerReflection, Default)]
struct MyApp {
    dt: Option<Instant>,
    dt_samples: [u16; 20],
    dt_samples_index: u8,
    fps: String,
}

#[telera_app]
impl MyApp {}

impl App for MyApp {
    fn initialize(&mut self) -> Startup {
        Startup {
            window_attributes: Window::default_attributes()
                .with_inner_size(LogicalSize::new(900, 600)),
            window_name: "scene".to_string(),
            page: None,
            watch_path: RunType::Watch("examples/layouts".to_string()),
        }
    }

    fn onload(&mut self, api: &mut API) {
        // The model spins every frame, so ask the framework to keep drawing
        // this window (paced to `frame_interval`, 30 fps by default).
        api.set_viewport_continuous("scene", true);

        // rfd's `set_directory` needs an *absolute* path on Linux - the XDG
        // portal backend silently ignores a relative one (like `"./"`) and
        // falls back to the last-used folder, usually $HOME. `current_dir()`
        // is the process's working directory (the crate root under `cargo run`).
        let start_dir = std::env::current_dir().unwrap_or_else(|_| ".".into());

        let files = telera_app::FileDialog::new()
            .add_filter("gltf", &["gltf"])
            .set_directory(&start_dir)
            .pick_file();

        if let Some(files) = files {
            let _model = api.load_gltf_model("container", files, None);
            let mut transform = Transform::new();
            transform.position.x += 100.0;
            api.add_instance("container", "other", Some(transform));
        }
    }

    fn update(&mut self, api: &mut API) {
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
                    let mut averaged = 0.0;
                    for sample in self.dt_samples {
                        averaged += sample as f32;
                    }
                    averaged /= self.dt_samples.len() as f32;

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
