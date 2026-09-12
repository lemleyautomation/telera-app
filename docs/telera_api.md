# Telera Rust API

The Rust side of the framework: the `App` trait you implement, the `run` entry
point, the macros that erase the boilerplate, and every public method on `API`.

For the markdown layout language (`.tmd` layout files, the `` `element` `` /
`` `text` `` / `` `list` `` / `` `render-window` `` vocabulary, data binding,
events) see [`tml-spec.md`](tml-spec.md). This document is only the Rust
surface.

---

## 1. Trivial app

```rust
use telera_app::*;

#[derive(LayoutRunnerReflection, Default)]
struct MyApp {
    clicks: u32,
    label: String,        // shown in the layout as *label*
}

#[telera_app]
impl MyApp {
    #[layout_event]
    fn bump(&mut self, _ctx: Option<EventContext>, _api: &mut API) {
        self.clicks += 1;
    }
}

impl App for MyApp {
    fn initialize(&mut self) -> Startup {
        Startup {
            window_attributes: Window::default_attributes()
                .with_inner_size(LogicalSize::new(800, 600)),
            window_name: "Main".to_string(),
            page: None,
            watch_path: RunType::Watch("layouts".to_string()),
        }
    }

    fn update(&mut self, _api: &mut API) {
        self.label = format!("clicked {} times", self.clicks);
    }
}

fn main() {
    run::<MyApp>(MyApp::default());
}
```

with `layouts/Main.tmd`:

```markdown
# root
- element
  - config
    - grow
    - color rgb(30,30,38)
    - align-children-x center
    - align-children-y center
  - element button
    - config
      - color rgb(80,110,200)
      - padding-all 16
      - radius-all 8
      - left-clicked bump
    - text
      - config
        - font-size 20
        - font-color white
      - *label*
```

What each piece does:

| piece | role |
|---|---|
| `#[derive(LayoutRunnerReflection)]` | lets the layout read `self`'s fields by name (`*label*` → `self.label`) |
| `#[telera_app]` on the `impl` | generates the event/element dispatch glue; `#[layout_event] fn bump` becomes reachable as `left-clicked bump` |
| `impl App` + `initialize` | the one required method: hands back the first window and where the layout files live |
| `App::update` | your per-frame logic - here it re-formats `label` from the click count |
| `Startup::watch_path` | `RunType::Watch("layouts")` loads every `.tmd` under `layouts/` and hot-reloads on change; the page shown is `Main.tmd` (matches `window_name`) |
| the `.tmd` file | one page. A leading `#### TML 1.0` header block is optional and only needed for `` `load` `` (image) / `` `font` `` directives (`tml-spec.md` §Images) |
| `run::<MyApp>(app)` | takes over the thread, opens the window, runs the event loop |

That is the whole program - no manual render loop, no GPU setup, no event
plumbing. `cargo run` and edit `Main.tmd` live.

---

## 2. `run` and the `App` trait

```rust
pub fn run<UserApp: App>(app: UserApp)   // takes over the thread and drives the winit event loop until the last window closes
```

```rust
pub trait App: LayoutRunnerReflection + LayoutReflector + Sized {
    fn initialize(&mut self) -> Startup;          // required - runs once, before API exists
    fn onload(&mut self, api: &mut API) {}        // once, right after the GPU context is up
    fn update(&mut self, api: &mut API) {}        // once per event-loop wake (≈ per frame while animating)
    fn layout(&mut self, page: &str, api: &mut API) {}  // imperative UI only (see #4)
}
```

- **`initialize`** builds the bootstrap window. `API` does not exist yet, so
  anything needing `&mut API` (loading a model, staging an image, adding a
  camera) goes in `onload`.
- **`onload`** is the place for one-time GPU-touching setup.
- **`update`** is your per-frame logic: mutate `self`'s fields, drive cameras
  and model transforms, read input off `api`. It runs every time the event
  loop wakes - which is every `frame_interval` while a window is animating
  (`set_viewport_continuous`), and otherwise only on input.
- **`layout`** is only called for windows whose `watch_path` is
  `RunType::None` - the hand-coded UI path. Markdown-driven windows ignore it.

`LayoutRunnerReflection` and `LayoutReflector` are supertraits; you almost
never write them by hand - the derives in #3 do it.

### `Startup`

```rust
pub struct Startup {
    pub window_attributes: WindowAttributes,  // winit; Window::default_attributes()....
    pub window_name: String,                  // the window title AND its lookup key for `set_viewport_*`
    pub page: Option<String>,                 // which page to show; None => same as window_name
    pub watch_path: RunType,
    pub agent_access_port: Option<u16>,       // Some(port) => remote-control HTTP server; see §4.11
}
```

### `RunType`

| variant | meaning |
|---|---|
| `RunType::Watch(dir)` | load every `.tmd` under `dir` at startup, then hot-reload changed files |
| `RunType::Once(dir)` | load every `.tmd` under `dir` once, never look again |
| `RunType::None` | don't load any layout files - this app builds its UI in Rust via `App::layout` (see #4) |

A "page" is one `.tmd` file (named by its filename minus its extension); reusable
snippets inside a file are `## headings`. See `tml-spec.md` §1.

---

## 3. Macros

All are re-exported from `telera_app` (`use telera_app::*;`).

### `#[derive(LayoutRunnerReflection)]` — on your app struct

Exposes the struct's fields to the markdown layout **by name**. A field's Rust
type decides how it is exposed:

| Rust type | resolved by the layout as |
|---|---|
| `bool` | `get-bool` / `*name*` in a bool position (`if`) |
| any integer or float primitive (`u8`…`i128`, `usize`, `isize`, `f32`, `f64`) | `get-numeric` / `*name*` in a numeric position |
| `String` | `get-text` / `*name*` in a text element |
| `Color` | `get-color` / `*name*` in a `color` config |
| `UIImageDescriptor` | `get-image` / `*name*` in an `image` config |
| `Vec<T>` | `list Name` / `item Name` iteration |
| anything else | ignored (fine to have) |

Names are matched **exactly** - `*content_background_color*` resolves the
`content_background_color` field; `*content background color*` or a different
case does not. Write the Rust identifier verbatim in the layout.

```rust
#[derive(LayoutRunnerReflection, Default)]
struct App {
    title: String,           // *title*
    zoom: f32,               // *zoom*
    dark_mode: bool,         // *dark_mode*  (in an `if`)
    documents: Vec<Doc>,     // `list` documents
    selected: usize,         // an index field for `item` / `if-index`
}
```

**`Vec<T>` field attributes:**

| attribute | effect |
|---|---|
| *(none)* | each row's own fields resolve through `T: FieldAccess` (so `T` must `#[derive(FieldAccess)]`) |
| `#[no_field_access]` | `list` length only, no per-item fields |
| `#[list_click_event(handler)]` | routes `left-clicked *clicked*` (arg spelled exactly `clicked`) inside this `list` to `#[layout_event] fn handler`. In practice most layouts just write `left-clicked handler` statically inside the list instead - either way the handler gets the row index in `EventContext::list_index` |

### `#[derive(FieldAccess)]` — on a list-item struct

Same type→lookup mapping as above, minus the `Vec` case (no nested lists).
Put it on the `T` in a `Vec<T>` list field.

```rust
#[derive(FieldAccess, Default)]
struct Doc {
    title: String,       // inside `list` documents: *title*
    word_count: u32,     // *word_count*
}
```

### `#[telera_app]` — on your app's inherent `impl` block

Writes the `LayoutReflector` impl and wires up the two kinds of callback the
layout can name:

| marker on a method | signature | reached from the layout as |
|---|---|---|
| `#[layout_event]` | `fn name(&mut self, ctx: Option<EventContext>, api: &mut API)` | any event config, e.g. `left-clicked name`, `hover name`, `key-event name` |
| `#[layout_element]` | `fn name(&mut self, api: &mut API)` | `` `fn` *name* `` - the method builds a subtree at that spot with `api.l` |

Methods without a marker are left untouched. The markdown always refers to a
method by its Rust name.

```rust
#[telera_app]
impl App {
    #[layout_event]
    fn document_clicked(&mut self, ctx: Option<EventContext>, _api: &mut API) {
        if let Some(i) = ctx.and_then(|c| c.list_index) {
            self.selected = i;
        }
    }

    #[layout_element]
    fn gizmo(&mut self, api: &mut API) {
        api.l.open_element();
        api.l.configure_element(&ElementConfiguration::new().fixed_square(16.0)
            .color(Color::rgb(255, 0, 0)).end());
        api.l.close_element();
    }
}
```

### `#[layout_fn]` — on a hand-written `App::layout` (imperative UI)

For `RunType::None` windows that build their UI in Rust instead of markdown. It
injects two helper macros into the method body:

- `e!(config $(, child_stmt)*)` — open an element, `configure_element(&config)`,
  run the child statements, close it.
- `t!(text_config, content)` — add a text run to the current element. A bare
  string literal (`t!(cfg, "hello")`) takes a zero-copy path automatically;
  anything else (`&self.title`, `&format!(...)`) is copied so it's safe
  regardless of how long it lives.

Both call a binding named `api`, so keep the parameter named `api`.

```rust
impl App for MyApp {
    #[layout_fn]
    fn layout(&mut self, page: &str, api: &mut API) {
        let row  = ElementConfiguration::default().grow().child_gap(8).end();
        let cell = ElementConfiguration::default().padding_all(12)
            .color(Color::rgb(60, 60, 70)).end();
        let text = TextConfig::new().font_size(16).font_color([255; 4].into()).end();

        e!(row,
            e!(cell, t!(text, "left")),
            e!(cell, t!(text, "right")),
        );
    }
}
```

See `examples/basic.rs` for a full imperative UI (hover state, floating menus,
scroll, lists) built this way.

### Writing the reflection traits by hand

If your app takes no input from the layout and fires no events, skip the
derives:

```rust
impl LayoutRunnerReflection for MyApp {}   // all getters default to None
impl LayoutReflector for MyApp {}          // dispatch is a no-op
```

The full trait surfaces (every method is defaulted):

```rust
trait LayoutRunnerReflection {
    fn get_bool(&self, name, list_data) -> Option<bool>;
    fn get_numeric(&self, name, list_data) -> Option<f32>;
    fn get_text(&self, name, list_data) -> Option<&String>;
    fn get_color(&self, name, list_data) -> Option<&Color>;
    fn get_image(&self, name, list_data) -> Option<&UIImageDescriptor>;
    fn get_list_length(&self, name, list_data) -> Option<usize>;
    // + get_event
}

trait LayoutReflector {
    fn dispatch_event(&mut self, name, ctx: Option<EventContext>, api: &mut API);
    fn dispatch_custom_element(&mut self, name, api: &mut API);
}
```

---

## 4. `API` reference

`API` is handed to you as `&mut API` in `onload`, `update`, `layout`, and every
`#[layout_event]` / `#[layout_element]` method. It is the only handle to the
framework at runtime.

### 4.1 Per-window input (read-only)

These read the **active window** - the one currently being updated or drawn -
so the same call in an event handler and in `update` sees the right window's
input. Away from any window they return a neutral default.

| method | returns | notes |
|---|---|---|
| `dt() -> f32` | seconds since this window's previous *redraw* | `0.0` when called from `update` (no redraw in progress) - track your own clock there |
| `window_size() -> (f32, f32)` | physical pixel size | |
| `dpi_scale() -> f32` | the window scale factor | |
| `mouse_position() -> (f32, f32)` | cursor, physical px | |
| `mouse_delta() -> (f32, f32)` | movement since last frame | cleared each frame |
| `scroll_delta() -> (f32, f32)` | wheel delta this frame | cleared each frame |
| `x_at_click() / y_at_click() -> f32` | cursor position (logical px) latched at the last mouse-down | |
| `left_mouse_pressed() / _down() / _released() / _clicked() / _double_clicked() -> bool` | edge (`pressed`/`released`/`clicked`) vs level (`down`) | `clicked` = quick press+release |
| `right_mouse_pressed() / _down() / _released() / _clicked() -> bool` | same for the right button | |
| `middle_mouse_pressed() / _down() / _released() / _clicked() -> bool` | same for the middle button | |
| `key_events() -> &[KeyEvent]` | winit key events since the last redraw | read them the frame they arrive |
| `event_string() -> &str` | accumulated text this frame | |
| `synthetic_key_events() -> &[SyntheticKeyEvent]` | key events fabricated by the agent access port (§4.11) | separate from `key_events` - see why there |
| `focus() -> Option<GlobalSymbol>` | the interned id of the focused element, or `None` | only named elements are focusable |
| `agent_port_active() -> bool` | whether `Startup::agent_access_port` was set | see §4.11 |

```rust
fn update(&mut self, api: &mut API) {
    if api.left_mouse_clicked() {
        self.count += 1;
    }
    for key in api.key_events() {
        if key.state == ElementState::Pressed
            && let Some(t) = &key.text {
            self.search.push_str(t);
        }
    }
}
```

### 4.2 Windows / viewports

A "viewport" is one OS window. The **name** you pass is its title and its
lookup key. Missing name → the call is a silent no-op.

| method | effect |
|---|---|
| `create_viewport(name, page: Option<&str>, attrs: WindowAttributes)` | stage a new window; opens on the next loop iteration. `page = None` → shows the page named `name` |
| `create_default_viewport()` | stage an 800×600 window named `"Main"` |
| `set_viewport_title(name, title)` | change the OS title (the lookup key stays `name`) |
| `set_viewport_page(name, page)` | point a window at a different page and request a redraw |

```rust
fn onload(&mut self, api: &mut API) {
    api.create_viewport(
        "Inspector",
        Some("inspector"),                        // shows inspector.tmd
        Window::default_attributes().with_inner_size(LogicalSize::new(400, 700)),
    );
}
```

### 4.3 Redraw control

The event loop is demand-driven: it sleeps until something asks for a frame.

| method | effect |
|---|---|
| `request_redraw(name)` | produce **one** more frame for that window (paced by `frame_interval`) |
| `set_viewport_continuous(name, bool)` | keep producing frames every `frame_interval` (default 30 fps); turning it on wakes the loop now |
| `set_viewport_frame_interval(name, Duration)` | change the minimum gap between animation frames for that window |

Hot-reload, input, and resize schedule redraws automatically. You only need
these when *you* change something (an animation, a model transform) with no
matching input event - e.g. a spinning model wants `set_viewport_continuous`.

```rust
fn onload(&mut self, api: &mut API) {
    api.set_viewport_continuous("Main", true);            // animate at 30 fps
    api.set_viewport_frame_interval("Main", Duration::from_millis(16)); // ...make it 60
}
```

### 4.4 Images

| method | effect |
|---|---|
| `add_image(name, image: DynamicImage)` | register/replace an atlas the layout can reference by `name` in an `image` config or `set-image` declaration |

`DynamicImage` and `load_from_memory` are re-exported from the `image` crate.
A layout can also load its own images with a `` `load` `` header directive (no
Rust needed) - see `tml-spec.md` §Images.

```rust
fn onload(&mut self, api: &mut API) {
    let logo = load_from_memory(include_bytes!("logo.png")).unwrap();
    api.add_image("logo", logo);      // layout: `image` logo
}
```

### 4.5 Fonts

| method | effect |
|---|---|
| `load_font(font_id: u16, data: Vec<u8>)` | register a `.ttf`/`.otf`; `` `font-id` `` `font_id` selects it, and it also joins the fallback chain ahead of the platform defaults (so a loaded emoji/symbol font is used automatically) |

Unregistered ids use the system sans-serif. A layout can load its own fonts from
its `#### TML` header, no Rust needed - the link text is the numeric `font-id`:

```
#### TML 1.0
- `font` [1](assets/Inter.ttf)
- `font` [2](assets/TwemojiMozilla.ttf)
```

```rust
fn onload(&mut self, api: &mut API) {
    api.load_font(1, include_bytes!("Inter.ttf").to_vec());   // layout: `font-id` 1
}
```

`examples/fonts/TwemojiMozilla.ttf` (bundled) is a ~1.5 MB COLRv0 colour-emoji
font; loading it makes emoji render in colour everywhere.

### 4.6 Effects & custom shaders

Any element with a background `color` (or a border, an `image`, or a
`circle`/`ring` shape) can carry one or more fragment-level effects via the
`` `shader` `` config keyword (repeat it to stack). Four effects are built in;
more come from your own WGSL. `drop_shadow`/`blur` render behind the fill, the
rest over it.

| method | effect |
|---|---|
| `register_ui_shader(name: &str, wgsl_body: &str) -> Result<(), String>` | validate (naga) + compile a custom fragment shader; `` `shader` *name* `` then applies it. Also reachable from a `#### TML` header directive. |

```
#### TML 1.0
- `shader` [glass](shaders/glass.wgsl)

# root
- `element` card
  - `config`
    - `color` rgb(210,90,70)
    - `radius-all` 16
    - `shader` *drop_shadow*
    - `shadow-blur` *blur_amount*
    - `shadow-color` rgba(0,0,0,0.55)
```

**Built-in effects** (name plus alias, and their named parameter keywords -
each a literal or a `*get-numeric*` / `*get-color*` binding):

| `shader` | parameters |
|---|---|
| `drop_shadow` (`shadow`) | `shadow-offset-x`, `shadow-offset-y`, `shadow-blur`, `shadow-spread`, `shadow-color` |
| `raised_edge` (`bevel`) | `bevel-width`, `bevel-light-angle` (deg), `bevel-highlight`, `bevel-shade` |
| `inner_glow` (`glow`) | `glow-blur`, `glow-spread`, `glow-color` |
| `blur` | `blur-radius`, `blur-tint` |

Distances are logical px (dpi-scaled for you). Anything a value is bound to and
animated repaints correctly (it is part of the UI-layer cache fingerprint).

`blur` is a backdrop blur: it runs a small sub-pass after the UI is drawn that
blurs the **UI layer** behind the element (not the 3D scene) and writes it back,
tinted by `blur-tint`. Put a blur panel's label/content beside or below it, not
inside - anything drawn into the UI before the sub-pass sits under the frost.

**Custom shaders.** A `` - `shader` [name](path.wgsl) `` directive names a file
that provides only `fs_main` - telera prepends a prelude with the vertex stage,
bind groups, a `VertexPayload` (`in.color`, `in.uv`, `in.frag_px`,
`in.rect_center`/`in.rect_half`/`in.radii` in px, `in.params` =
`shader-param-1..4`, `in.params2` = `shader-param-5..8`) and an
`sd_rounded_box(p, b, r4)` helper. Apply with `` `shader` *name* `` and tune
with `` `shader-param-1` `` .. `` `shader-param-8` `` (positional). A shader
that fails to read/parse/validate is logged and skipped - the element renders
without the effect. See `examples/effects.rs` + `examples/shaders/*.wgsl`.

### 4.7 3D scene: models

The scene is global - one set of models shared by every window and every
camera. `model_name` is your lookup key.

| method | returns | effect |
|---|---|---|
| `load_gltf_model(model_name, path: PathBuf, transform: Option<Transform>)` | `BaseMesh` (mesh metadata, usually ignored) | register a glTF mesh. Registered ≠ visible |
| `add_instance(model_name, instance_name, transform: Option<Transform>)` | | add one visible copy of the model at `transform` |
| `transform_model(model_name) -> Result<&mut Transform, APIError>` | | the model-level transform (applied to *every* instance) |
| `transform_instance(model_name, instance_name) -> Result<&mut Transform, APIError>` | | one instance's own transform |

`APIError` is `ModelNotFound`.

```rust
fn onload(&mut self, api: &mut API) {
    api.load_gltf_model("crate", PathBuf::from("models/crate.gltf"), None);
    api.add_instance("crate", "a", None);
    let mut t = Transform::new();
    t.move_x_axis(200.0);
    api.add_instance("crate", "b", Some(t));
}

fn update(&mut self, api: &mut API) {
    if let Ok(t) = api.transform_model("crate") {
        t.rotate_y_axis(1.0);          // spins both copies
    }
}
```

### 4.8 3D scene: cameras

A `render-window` element (see `tml-spec.md` §`render-window`) draws the scene
into a layout rectangle through a named `Camera`. Cameras also drive the
full-screen fallback when a frame has no `render-window`.

| method | effect |
|---|---|
| `add_camera(name)` | create a camera at the default framing if it doesn't exist. A camera named `"default"` always exists |
| `remove_camera(name)` | remove it (`"default"` can't be removed) |
| `camera(name) -> Option<&mut Camera>` | app-side control |

`Camera` (re-exported) — fields `eye`, `target`, `up` (`cgmath::Point3`/`Vector3`),
`aspect`, `fovy`, `znear`, `zfar`, `orthographic: bool`, `ortho_height: f32`.
The `cgmath` crate is re-exported for the vector types; the convenience methods
below keep most code cgmath-free (all take/return `f32`, all chain):

| method | does |
|---|---|
| `set_eye(x,y,z)` / `set_target(x,y,z)` | place / aim the camera |
| `pan(x, y)` | slide eye + target across the view plane (`+x` right, `+y` up) |
| `translate(x, y, z)` | move eye + target by a world delta |
| `orbit(yaw, pitch)` | rotate the eye around the target (radians); distance-preserving, clamps at the poles |
| `zoom(fraction)` | step a fraction of the eye→target distance closer (`+`) / away (`-`); scales `ortho_height` when orthographic |
| `dolly(amount)` | move the eye an absolute distance along the view direction |
| `frame(x, y, z, radius)` | aim at a point and back off so a sphere of `radius` fills the view |
| `perspective(fovy_deg)` / `orthographic(height)` | switch projection |
| `clip_planes(near, far)` | set the clip-plane distances |
| `distance()` / `is_orthographic()` | readbacks |

```rust
fn onload(&mut self, api: &mut API) {
    api.add_camera("orbit");
    if let Some(cam) = api.camera("orbit") {
        cam.clip_planes(1.0, 6000.0).frame(0.0, 120.0, 0.0, 900.0);
    }
}

fn update(&mut self, api: &mut API) {
    if let Some(cam) = api.camera("orbit") {
        cam.orbit(0.01, 0.0);              // needs set_viewport_continuous to run every frame
    }
}
```

### 4.9 `api.canvas` — pan/zoom surfaces

A `` `canvas` `` element (see `tml-spec.md` §`canvas`) is a clipped container
that pans and zooms a named [`Canvas`](#canvas-re-exported). The framework only
*stores* that state - the app drives it, the same way `api.camera` drives a
scene camera.

| method | effect |
|---|---|
| `add_canvas(name)` | create a `Canvas` (zoom `1.0`, pan `0`) if absent; use it in `onload` to set limits before the first frame |
| `remove_canvas(name)` | drop it (a `canvas` element on a live page re-creates it next frame) |
| `canvas(name) -> Option<&mut Canvas>` | app-side pan/zoom control |
| `canvas_rect(name) -> Option<(f32,f32,f32,f32)>` | the canvas element's on-screen rect `(x,y,w,h)` in **physical px** (same space as `mouse_position`), as of the last frame it was drawn (one frame old) |

#### `Canvas` (re-exported)

**Units:** everything you touch here is in **physical px** - the space of
`api.mouse_position` / `api.mouse_delta` - so `pan_by` takes a raw mouse delta
and `zoom_at_screen` a raw cursor position, at any DPI. `world` coords are
logical units (the numbers you write in TML: `offset-x 300`, ...).

Fields `zoom`, `pan_x`, `pan_y`, `min_zoom`, `max_zoom`,
`world_width: Option<f32>`, `world_height: Option<f32>`, `screen_rect` (physical
px). The runner keeps `screen_rect` and the DPI up to date. Methods (all `f32`,
all chain):

| method | does |
|---|---|
| `set_zoom(z)` / `zoom_by(factor)` | set / multiply zoom; clamped to `min_zoom..=max_zoom` |
| `zoom_around(factor, world_x, world_y)` | zoom keeping that **world** point under the same screen pixel |
| `zoom_at_screen(factor, screen_x, screen_y)` | same, anchored on a **physical** screen pixel - pass `api.mouse_position()` (needs `screen_rect`, so accurate from frame 2) |
| `pan_by(dx, dy)` / `set_pan(x, y)` | move / set the pan - pass `api.mouse_delta()`; unclamped |
| `reset()` | zoom `1.0`, pan `0` |
| `set_zoom_limits(min, max)` / `set_world_size(w, h)` | set the clamp / world size |
| `world_to_screen(wx, wy)` / `screen_to_world(sx, sy)` | convert between world and physical screen px |

```rust
fn onload(&mut self, api: &mut API) {
    api.add_canvas("board");
    if let Some(c) = api.canvas("board") { c.set_zoom_limits(0.35, 4.0); }
}

fn update(&mut self, viewport: Option<&str>, api: &mut API) {
    if viewport.is_none() { return; }                 // 2nd update carries input
    let (mx, my) = api.mouse_position();
    let (_, wheel) = api.scroll_delta();
    let panning = api.right_mouse_down();
    let (dx, dy) = api.mouse_delta();
    if let Some(c) = api.canvas("board") {
        if wheel != 0.0 { c.zoom_at_screen(1.15_f32.powf(wheel), mx, my); }
        if panning { c.pan_by(dx, dy); }
    }
}
```

See `examples/canvas.rs`.

### 4.10 `api.l` — the layout engine

`api.l` (`pub l: LayoutEngine<...>`) is the low-level layout builder. Markdown
apps rarely touch it; you need it for `#[layout_element]` methods,
`#[layout_fn]` UI, and querying the laid-out result.

**Building (inside `#[layout_element]` / `#[layout_fn]`):**

| method | effect |
|---|---|
| `open_element()` | begin an element |
| `configure_element(&ElementConfiguration) -> u32` | configure the open element (returns its clay id) |
| `add_text_element(content: &str, &TextConfig)` | add a text run to the open element; copies `content`, so any `&str` works regardless of how long it lives |
| `add_static_text_element(content: &'static str, &TextConfig)` | like `add_text_element` but zero-copy for genuinely `'static` content (string literals) |
| `close_element()` | end it. Every `open_element` needs a `configure_element` and a `close_element` |

`configure_element` clones any `.image()` / `.custom_element()` / `.custom_layout_settings()`
referent on `ElementConfiguration` immediately, so those can point at a plain local
too - no `const`/`'static` needed, and no lifetime bookkeeping of your own. Only the
*moment `configure_element` is called* matters: the referent just needs to still be
in scope then, which it always is for values built earlier in the same function.

**Querying (any time after a layout pass):**

| method | returns |
|---|---|
| `hovered() -> bool` | is the pointer over the currently open element (call between configure and close) |
| `get_element_id(id: &str)` | a clay element id for a named element (pass to `bounding_box` / `element_found`) |
| `bounding_box(id) -> Option<BoundingBox>` | its final on-screen rectangle from the last frame (`.x/.y/.width/.height`) |
| `element_found(id) -> bool` | was an element with that id laid out last frame |
| `get_scroll_offset()` | the open scroll container's current offset (`.x` / `.y`) |

```rust
#[layout_element]
fn badge(&mut self, api: &mut API) {
    api.l.open_element();
    api.l.configure_element(&ElementConfiguration::new()
        .padding_all(6).radius_all(4.0).color(Color::rgb(200, 60, 60)).end());
    api.l.add_text_element(&self.badge_text, &self.badge_style);
    api.l.close_element();
}
```

### 4.11 Agent access port

Setting `Startup::agent_access_port` to `Some(port)` spawns a background HTTP
server on `127.0.0.1:port` that lets an external process - a script, a test
harness, an LLM coding agent - drive the app directly: inject synthetic mouse
and keyboard input (including multi-step "combos"), pull a screenshot, or
dump the current frame's layout/render commands to a file. `None` (the
default) opens no socket and adds no overhead - the feature is entirely
opt-in.

```rust
fn initialize(&mut self) -> Startup {
    Startup {
        // ...
        agent_access_port: Some(4545),
    }
}
```

**While the port is active, every window force-draws a ~3px border** so it's
never ambiguous, on screen, that the app is remotely controllable:

![A window with the agent access port active - note the orange border around every edge](images/agent-port-border.png)

The border is drawn last in that frame's render commands (so it always paints
over everything else, regardless of any element's own `z-index`) and is not
configurable - a fixed, unmissable color by design.

Plain JSON over hand-rolled HTTP/1.1 (no keep-alive, no chunked encoding) -
`curl` is enough to drive it. Every response is `{"ok": bool, "path"?:
string, "message"?: string}`; a screenshot/dump's `path` is the file it wrote
(request one explicitly or a timestamped default under the OS temp dir is
used). Every request except `GET /health` accepts an optional `"window"`
field (defaults to the bootstrap window's name).

| endpoint | body | effect |
|---|---|---|
| `GET /health` | - | liveness check, `{"ok":true}` |
| `POST /input` | `{"window"?, "events": [...]}` | apply a sequence of synthetic input events, in order, then request a redraw |
| `POST /screenshot` | `{"window"?, "path"?}` | wait for the next frame, PNG-encode the swapchain, write it to `path` |
| `POST /dump` | `{"window"?, "kind": "layout"\|"render"\|"both", "path"?}` | `Debug`-format the requested command list(s) to a text file |

`POST /input`'s `events` array is a tagged union - `mouse_move {x, y}`
(physical px, absolute), `mouse_down`/`mouse_up`/`mouse_click {button}`
(`"left"|"right"|"middle"`), `scroll {dx, dy}`, `key_down`/`key_up {key}`
(a name like `"KeyA"`, `"Enter"`, `"ControlLeft"` - see `key_from_name` in
`src/agent_port.rs` for the full table), and `type_text {text}` (appended
straight to `event_string()`). Events apply in order within one request, so a
"combo" - hold a modifier, click, release it - is one request, not several:

```bash
curl -X POST http://127.0.0.1:4545/input -d '{"events":[
  {"type":"mouse_move","x":700.0,"y":240.0},
  {"type":"key_down","key":"ControlLeft"},
  {"type":"mouse_click","button":"left"},
  {"type":"key_up","key":"ControlLeft"}
]}'

curl -X POST http://127.0.0.1:4545/screenshot -d '{"path":"/tmp/shot.png"}'
curl -X POST http://127.0.0.1:4545/dump -d '{"kind":"both","path":"/tmp/dump.txt"}'
```

A few things worth knowing before you rely on this:

- **`key_down`/`key_up` land on `synthetic_key_events()`, not `key_events()`.**
  A real `winit::event::KeyEvent` can't be constructed outside winit (it has
  a `platform_specific` field private to winit itself), so a fabricated key
  press is a different, framework-owned type
  (`SyntheticKeyEvent { physical_key, logical_key, location, state, repeat }`,
  built from winit's public `PhysicalKey`/`Key`/`KeyLocation`/`ElementState`).
  Code that only reads real keyboard input via `key_events()` won't see
  injected key presses unless it also checks `synthetic_key_events()`.
- **`POST /screenshot`/`POST /dump` (with `kind: "render"` or `"both"`) wait
  for an actual frame** - screenshot pixels and render commands are
  transient, only existing mid-redraw - so they can take a little longer
  than `/input`, and time out (`504`) after 5s if the target window never
  redraws (e.g. it's minimized). `kind: "layout"` alone answers immediately,
  since a page's authored command list persists across frames.
- **The connection is one request per socket** - no keep-alive. Each request
  spawns its own short-lived thread, so a slow/hung client only blocks
  itself, never other requests.

See `src/agent_port.rs` for the full wire format and implementation.

---

## 5. Building blocks

### `ElementConfiguration`

A `const`-friendly builder; every method returns `&mut Self`, finish with
`.end()`. Grouped:

- **sizing** — `grow()`, `fit()`, `width_grow()` / `height_grow()` (+ `_min`,
  `_min_max`), `width_fit()` / `height_fit()` (+ `_min`, `_min_max`),
  `width_fixed(f32)` / `height_fixed(f32)`, `fixed(w, h)`, `fixed_square(s)`,
  `width_percent(p)` / `height_percent(p)`, `aspect_ratio(r)`
- **layout** — `vertical()` / `horizontal()`, `padding_all(u16)` (+
  `_top/_bottom/_left/_right`), `child_gap(u16)`,
  `align_children_x_left/_center/_right()`,
  `align_children_y_top/_center/_bottom()`
- **paint** — `color(Color)`, `radius_all(f32)` (+ per-corner),
  `border_color(Color)`, `border_all(u16)` (+ per-side, `border_in_between`)
- **scroll** — `scroll(vertical: bool, horizontal: bool)`,
  `scroll_child_offset(x, y)`
- **floating** — `floating()`, `floating_offset(x, y)`,
  `floating_dimensions(w, h)`, `floating_z_index(i16)`,
  `floating_attach_parent_*()` / `floating_attach_self_*()` (nine anchor
  points each), `floating_attach_to_root()`, `floating_clip_to_attached_parent()`,
  `floating_pointer_capture()` / `floating_pointer_pass_through()`
- **identity / special** — `id(&str)`, `id_indexed(&str, index)`,
  `custom_element(&data)`, `custom_layout_settings(&data)`, `image(&data)` — all
  three accept a plain local; `configure_element` clones `data` in immediately,
  so it only needs to be valid at that call, not `const`/`'static`

```rust
let panel = ElementConfiguration::new()
    .vertical()
    .padding_all(16)
    .child_gap(8)
    .color(Color::rgb(43, 41, 51))
    .radius_all(8.0)
    .grow()
    .end();
```

### `TextConfig`

`TextConfig::new()....end()`: `font_id(u16)`, `font_size(u16)`,
`font_color(Color)`, `line_height(u16)`, `letter_spacing(u16)`,
`align_left/_center/_right()`, `wrap_mode_words/_new_lines/_none()`.

### `Color`

`Color::rgb(u8, u8, u8)`, or `[r, g, b, a].into()` from a `[u8; 4]`.

```rust
let accent: Color = Color::rgb(90, 110, 200);
let white:  Color = [255, 255, 255, 255].into();
```

### `Transform` (models)

`Transform::new()` (identity); public fields `position` / `rotation` / `scale`
(`cgmath` types), plus helpers: `move_x_axis(m)` / `_y_axis` / `_z_axis`,
`rotate_x_axis(deg)` / `_y_axis` / `_z_axis`, `scale_x_axis(s)` / `_y_axis` /
`_z_axis`.

### `EventContext`

Passed to every `#[layout_event]` handler (`Option<EventContext>`):

```rust
pub struct EventContext {
    pub text: Option<String>,
    pub code: Option<u32>,
    pub code2: Option<u32>,
    pub list_index: Option<usize>,   // set when the event fired from inside a `list`
}
```

---

## 6. Re-exports

`use telera_app::*;` brings in, besides the framework's own items:

| from | items |
|---|---|
| `winit` | `Window`, `WindowAttributes`, `WindowId`, `LogicalSize`, `KeyEvent`, `ElementState`, `Key`, `NamedKey`, `KeyCode`, `KeyLocation`, `PhysicalKey`, `keyboard` |
| `image` | `image` (the crate), `DynamicImage`, `load_from_memory` |
| `cgmath` | `cgmath` (the crate) - for `Camera` / `Transform` vector math |
| framework | `Color`, `ElementConfiguration`, `TextConfig`, `Camera`, `Canvas`, `Transform`, `Quaternion`, `Euler`, `Model`, `BaseMesh`, `CustomElement`, `UIImageDescriptor`, `EventContext`, `SyntheticKeyEvent`, `symbol_table`, `rkyv` |

---

## 7. See also

- [`tml-spec.md`](tml-spec.md) — the markdown layout language in full.
- `examples/` — `basic.rs` (imperative UI; also wires `agent_access_port` on
  for manual testing, see §4.11), `layout.rs` (markdown + events + list),
  `images.rs` (all three image paths), `scene.rs` (3D + cameras +
  `render-window`), `shapes.rs` (drawn shapes), `custom_element.rs`
  (`#[layout_element]`), `canvas.rs` (a pannable / zoomable `canvas`),
  `stress.rs` (a large markdown dashboard).
