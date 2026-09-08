use std::sync::Arc;
use std::time::Instant;

use symbol_table::GlobalSymbol;
use winit::window::Window;
use winit::{dpi::PhysicalSize, event::KeyEvent};

use crate::graphics::{
    depth_texture::DepthTexture, multi_sample_texture::MultiSampleTexture, ui_surface::UiSurface,
};

/// One window and everything that is specific to it: its GPU surface, the page
/// it currently shows, and - the point of this struct - all the mouse / keyboard
/// / os input that has landed on *this* window since it was last drawn. The
/// layout runner only ever looks at the active viewport's input, so building the
/// UI for one window never sees events meant for another.
#[derive(Debug)]
pub struct Viewport {
    pub window: Arc<Window>,
    pub page: String,
    pub surface: wgpu::Surface<'static>,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub depth_texture: DepthTexture,
    pub multi_sample_texture: MultiSampleTexture,
    /// The cached UI layer for this window: the UI is drawn into it only when
    /// the render commands change, and composited over the 3D scene every frame.
    /// `None` until the first redraw (and after a resize) - rebuilt lazily by
    /// [`Viewport::ensure_ui_surface`].
    pub ui_surface: Option<UiSurface>,

    /// Physical pixel size of the surface, refreshed on every resize.
    pub size: (f32, f32),
    /// The window's scale factor, refreshed at the top of each redraw and on
    /// `ScaleFactorChanged`.
    pub dpi_scale: f32,

    // --- timing ---
    /// When this window was last drawn; only [`Viewport::begin_frame`] reads it.
    last_render: Instant,
    /// Seconds between the last two redraws of this window.
    pub dt: f32,

    // --- pointer ---
    pub mouse_position: (f32, f32),
    pub mouse_delta: (f32, f32),
    pub scroll_delta: (f32, f32),

    // --- left button ---
    pub left_mouse_pressed: bool,
    pub left_mouse_down: bool,
    pub left_mouse_released: bool,
    pub left_mouse_clicked: bool,
    pub left_mouse_double_clicked: bool,
    left_mouse_clicked_timer: Option<Instant>,

    // --- right button ---
    pub right_mouse_pressed: bool,
    pub right_mouse_down: bool,
    pub right_mouse_released: bool,
    pub right_mouse_clicked: bool,
    right_mouse_clicked_timer: Option<Instant>,

    pub x_at_click: f32,
    pub y_at_click: f32,
    /// Name (interned) of the currently focused element in this window's layout,
    /// or `None` if nothing is focused. Only named elements are focusable; the
    /// layout runner rewrites this on a left/right mouse-down (see
    /// `API::set_focus`).
    pub focus: Option<GlobalSymbol>,

    // --- os / keyboard one-shots, cleared every redraw ---
    pub key_events: Vec<KeyEvent>,
    pub event_string: String,
}

impl Viewport {
    /// Builds a viewport around an already-configured surface, with every input
    /// field zeroed. The two call sites in `lib.rs` (bootstrap window and staged
    /// windows) share this so the ~20 input fields aren't spelled out twice.
    pub fn new(
        window: Arc<Window>,
        page: String,
        surface: wgpu::Surface<'static>,
        surface_config: wgpu::SurfaceConfiguration,
        depth_texture: DepthTexture,
        multi_sample_texture: MultiSampleTexture,
    ) -> Self {
        let size = (
            surface_config.width as f32,
            surface_config.height as f32,
        );
        let dpi_scale = window.scale_factor() as f32;
        Viewport {
            window,
            page,
            surface,
            surface_config,
            depth_texture,
            multi_sample_texture,
            ui_surface: None,
            size,
            dpi_scale,
            last_render: Instant::now(),
            dt: 0.0,
            mouse_position: (0.0, 0.0),
            mouse_delta: (0.0, 0.0),
            scroll_delta: (0.0, 0.0),
            left_mouse_pressed: false,
            left_mouse_down: false,
            left_mouse_released: false,
            left_mouse_clicked: false,
            left_mouse_double_clicked: false,
            left_mouse_clicked_timer: None,
            right_mouse_pressed: false,
            right_mouse_down: false,
            right_mouse_released: false,
            right_mouse_clicked: false,
            right_mouse_clicked_timer: None,
            x_at_click: 0.0,
            y_at_click: 0.0,
            focus: None,
            key_events: Vec::new(),
            event_string: String::new(),
        }
    }

    /// Refreshes `dt` (time since this window's previous redraw) and the window
    /// scale factor. Called at the top of `API::redraw_viewport`.
    pub fn begin_frame(&mut self) {
        self.dt = self.last_render.elapsed().as_secs_f32();
        self.last_render = Instant::now();
        self.dpi_scale = self.window.scale_factor() as f32;
    }

    /// Clears the one-shot input state (edge-triggered clicks, deltas, key
    /// events) once the frame that observed it has been built. Down/position
    /// state persists; only the "happened this frame" signals reset.
    pub fn end_frame(&mut self) {
        self.left_mouse_pressed = false;
        self.left_mouse_released = false;
        self.left_mouse_clicked = false;
        self.left_mouse_double_clicked = false;
        if let Some(timer) = self.left_mouse_clicked_timer
            && timer.elapsed().as_millis() > 400
        {
            self.left_mouse_clicked_timer = None;
        }
        self.right_mouse_pressed = false;
        self.right_mouse_released = false;
        self.right_mouse_clicked = false;
        if let Some(timer) = self.right_mouse_clicked_timer
            && timer.elapsed().as_millis() > 300
        {
            self.right_mouse_clicked_timer = None;
        }
        self.scroll_delta = (0.0, 0.0);
        self.mouse_delta = (0.0, 0.0);
        self.key_events.clear();
        self.event_string.clear();
    }

    /// Records a left-button press for this window: sets the down/pressed flags,
    /// starts the click timer, and latches the cursor position at click time.
    pub fn left_mouse_press(&mut self) {
        self.left_mouse_pressed = true;
        self.left_mouse_down = true;
        if self.left_mouse_clicked_timer.is_none() {
            self.left_mouse_clicked_timer = Some(Instant::now());
        }
        self.x_at_click = self.mouse_position.0 / self.dpi_scale;
        self.y_at_click = self.mouse_position.1 / self.dpi_scale;
    }

    /// Records a left-button release: promotes a quick press+release into a
    /// `left_mouse_clicked` one-shot and clears the down flag.
    pub fn left_mouse_release(&mut self) {
        if let Some(timer) = self.left_mouse_clicked_timer
            && timer.elapsed().as_millis() < 400
        {
            self.left_mouse_clicked = true;
            self.left_mouse_clicked_timer = None;
        }
        self.left_mouse_down = false;
        self.left_mouse_released = true;
    }

    /// Right-button press: mirror of [`Viewport::left_mouse_press`].
    pub fn right_mouse_press(&mut self) {
        self.right_mouse_pressed = true;
        self.right_mouse_down = true;
        if self.right_mouse_clicked_timer.is_none() {
            self.right_mouse_clicked_timer = Some(Instant::now());
        }
        self.x_at_click = self.mouse_position.0 / self.dpi_scale;
        self.y_at_click = self.mouse_position.1 / self.dpi_scale;
    }

    /// Right-button release: mirror of [`Viewport::left_mouse_release`].
    pub fn right_mouse_release(&mut self) {
        if let Some(timer) = self.right_mouse_clicked_timer
            && timer.elapsed().as_millis() < 300
        {
            self.right_mouse_clicked = true;
            self.right_mouse_clicked_timer = None;
        }
        self.right_mouse_down = false;
        self.right_mouse_released = true;
    }

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        size: PhysicalSize<u32>,
        multi_sample_count: u32,
    ) {
        self.surface_config.width = size.width;
        self.surface_config.height = size.height;
        self.size = (size.width as f32, size.height as f32);
        self.surface.configure(device, &self.surface_config);
        // Dropped here, rebuilt at the new size on the next redraw (this method
        // has no handle on the compositor's bind-group layout / sampler).
        self.ui_surface = None;

        if size.width > 0 && size.height > 0 {
            self.depth_texture =
                DepthTexture::new(device, &self.surface_config, multi_sample_count);
            self.multi_sample_texture =
                MultiSampleTexture::new(device, &self.surface_config, multi_sample_count);
        }
    }

    /// Rebuilds [`Viewport::ui_surface`] if it is missing or no longer matches
    /// the surface size. Called at the top of `API::redraw_viewport`'s render
    /// step, with the compositor resources from the shared `UIRenderer`.
    pub fn ensure_ui_surface(
        &mut self,
        device: &wgpu::Device,
        composite_bind_group_layout: &wgpu::BindGroupLayout,
        composite_sampler: &wgpu::Sampler,
    ) {
        let (w, h) = (self.surface_config.width, self.surface_config.height);
        let stale = match &self.ui_surface {
            Some(surface) => surface.width != w.max(1) || surface.height != h.max(1),
            None => true,
        };
        if stale {
            self.ui_surface = Some(UiSurface::new(
                device,
                (w, h),
                self.surface_config.format,
                composite_bind_group_layout,
                composite_sampler,
            ));
        }
    }
    pub fn get_current_texture(&self) -> wgpu::SurfaceTexture {
        self.surface
            .get_current_texture()
            .expect("Failed to acquire next swap chain texture")
    }

    /// Width / height of the current surface, for the 3D camera's projection.
    /// Falls back to `1.0` when the window has zero height (e.g. minimized) so
    /// the perspective matrix never sees a NaN.
    pub fn aspect(&self) -> f32 {
        if self.surface_config.height == 0 {
            1.0
        } else {
            self.surface_config.width as f32 / self.surface_config.height as f32
        }
    }
}
