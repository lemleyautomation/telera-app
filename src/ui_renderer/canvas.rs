//! Pan/zoom state for a `` `canvas` `` element.
//!
//! A `canvas` is a plain clip container in the layout: it clips its content and
//! offsets it by the pan, and every spatial config inside it (`width-fixed`,
//! `padding-*`, `font-size`, ...) is multiplied by [`Canvas::zoom`] as the
//! layout is walked, so text and shapes re-rasterize crisply at any zoom
//! instead of being sampled from a scaled texture.
//!
//! One `Canvas` per `canvas` element name, held in `API::canvases`, addressed
//! from the layout by the element's name and from app code via
//! [`API::canvas`](crate::API::canvas). The framework never drives pan/zoom
//! itself - an app reads the mouse in `update` and calls the methods below.
//! This mirrors [`Camera`](crate::Camera) for the 3D scene.
//!
//! **Units.** Everything an app touches here is in **physical pixels** - the
//! same space as [`API::mouse_position`](crate::API::mouse_position) /
//! [`API::mouse_delta`](crate::API::mouse_delta) - so `pan_by(dx, dy)` takes a
//! raw mouse delta and `zoom_at_screen(f, mx, my)` takes a raw cursor position.
//! `world` coordinates are logical units (`1.0` world unit = `1` logical px at
//! `zoom == 1.0`), matching the numbers you write in TML (`offset-x 300`, ...).
//! The runner keeps [`Canvas::dpi`] and [`Canvas::screen_rect`] up to date.

/// Pan/zoom state for one `` `canvas` `` element.
///
/// Convention: a point at world `(wx, wy)` is drawn at physical screen
/// `(screen_rect.0 + wx*zoom*dpi + pan_x, screen_rect.1 + wy*zoom*dpi + pan_y)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Canvas {
    /// Content scale. `1.0` = 1 world unit per logical pixel. Clamped to
    /// `min_zoom..=max_zoom` after every mutation.
    pub zoom: f32,
    /// Pan offset applied to the content, in **physical px** (unclamped - pan is
    /// infinite). Set it with `pan_by` / `set_pan` from a mouse delta.
    pub pan_x: f32,
    pub pan_y: f32,
    pub min_zoom: f32,
    pub max_zoom: f32,
    /// World size the inner wrapper is fixed to, so `grow`/`fit`/`width-percent`
    /// children have a definite parent size (logical units). `None` = fall back
    /// to the canvas element's own laid-out size (else [`Canvas::AUTO_WORLD`]).
    pub world_width: Option<f32>,
    pub world_height: Option<f32>,
    /// The canvas element's on-screen rect `(x, y, w, h)` in **physical px**,
    /// from the previous frame. Filled by the runner each frame; lets app code
    /// do cursor-anchored zoom against a raw `api.mouse_position()`.
    pub screen_rect: (f32, f32, f32, f32),
    /// Framework-managed: the active window's DPI scale, kept in sync by the
    /// runner so the pan / zoom-anchor math can convert between the physical px
    /// an app passes in and the logical px the layout engine works in.
    #[doc(hidden)]
    pub dpi: f32,
    /// Framework-managed: whether the `` `canvas` `` element's one-time
    /// `zoom` / `pan-x` / `pan-y` seed has been applied yet. Not for app use.
    #[doc(hidden)]
    pub seeded: bool,
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

impl Canvas {
    /// World size used when neither `world-width`/`world-height` nor a known
    /// on-screen rect is available.
    pub const AUTO_WORLD: f32 = 1024.0;

    pub fn new() -> Self {
        Self {
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            min_zoom: 0.1,
            max_zoom: 10.0,
            world_width: None,
            world_height: None,
            screen_rect: (0.0, 0.0, 0.0, 0.0),
            dpi: 1.0,
            seeded: false,
        }
    }

    fn dpi(&self) -> f32 {
        if self.dpi > 1e-4 { self.dpi } else { 1.0 }
    }

    fn clamp_zoom(&mut self) {
        // Guard against an inverted/zero range from bad config.
        let lo = self.min_zoom.max(1e-4);
        let hi = self.max_zoom.max(lo);
        self.zoom = self.zoom.clamp(lo, hi);
    }

    /// Sets `min_zoom`/`max_zoom` and re-clamps the current zoom.
    pub fn set_zoom_limits(&mut self, min: f32, max: f32) -> &mut Self {
        self.min_zoom = min;
        self.max_zoom = max;
        self.clamp_zoom();
        self
    }

    pub fn set_zoom(&mut self, zoom: f32) -> &mut Self {
        self.zoom = zoom;
        self.clamp_zoom();
        self
    }

    /// Multiplies the current zoom by `factor` (`> 1` zooms in).
    pub fn zoom_by(&mut self, factor: f32) -> &mut Self {
        self.zoom *= factor;
        self.clamp_zoom();
        self
    }

    /// Zooms by `factor` while keeping the **world** point `(wx, wy)` under the
    /// same screen pixel. Respects the zoom limits (the pan compensation uses
    /// the actual, clamped zoom change).
    pub fn zoom_around(&mut self, factor: f32, wx: f32, wy: f32) -> &mut Self {
        let dpi = self.dpi();
        let old = self.zoom;
        self.set_zoom(old * factor);
        let delta = (self.zoom - old) * dpi;
        self.pan_x -= wx * delta;
        self.pan_y -= wy * delta;
        self
    }

    /// Zooms by `factor` while keeping the **screen** point `(sx, sy)` anchored -
    /// pass a raw `api.mouse_position()` (physical px, window-relative). Needs
    /// `screen_rect`, which the runner fills each frame, so it is accurate from
    /// the second frame onward.
    pub fn zoom_at_screen(&mut self, factor: f32, sx: f32, sy: f32) -> &mut Self {
        let (wx, wy) = self.screen_to_world(sx, sy);
        self.zoom_around(factor, wx, wy)
    }

    /// Adds a **physical-px** delta (a raw `api.mouse_delta()`) to the pan. Not
    /// clamped.
    pub fn pan_by(&mut self, dx: f32, dy: f32) -> &mut Self {
        self.pan_x += dx;
        self.pan_y += dy;
        self
    }

    /// Sets the pan to a physical-px offset.
    pub fn set_pan(&mut self, x: f32, y: f32) -> &mut Self {
        self.pan_x = x;
        self.pan_y = y;
        self
    }

    /// Back to zoom `1.0`, pan `0, 0`.
    pub fn reset(&mut self) -> &mut Self {
        self.zoom = 1.0;
        self.pan_x = 0.0;
        self.pan_y = 0.0;
        self.clamp_zoom();
        self
    }

    pub fn set_world_size(&mut self, width: f32, height: f32) -> &mut Self {
        self.world_width = Some(width);
        self.world_height = Some(height);
        self
    }

    /// World `(wx, wy)` -> window-relative **physical** screen px.
    pub fn world_to_screen(&self, wx: f32, wy: f32) -> (f32, f32) {
        let k = self.zoom * self.dpi();
        (
            self.screen_rect.0 + wx * k + self.pan_x,
            self.screen_rect.1 + wy * k + self.pan_y,
        )
    }

    /// Window-relative **physical** screen px (e.g. `api.mouse_position()`) ->
    /// world `(wx, wy)`.
    pub fn screen_to_world(&self, sx: f32, sy: f32) -> (f32, f32) {
        let k = self.zoom * self.dpi();
        (
            (sx - self.screen_rect.0 - self.pan_x) / k,
            (sy - self.screen_rect.1 - self.pan_y) / k,
        )
    }

    /// Pan expressed in the layout engine's logical px (its `clip.childOffset`).
    pub(crate) fn pan_logical(&self) -> (f32, f32) {
        let d = self.dpi();
        (self.pan_x / d, self.pan_y / d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> Canvas {
        let mut c = Canvas::new();
        c.set_zoom_limits(0.25, 8.0);
        c
    }

    #[test]
    fn zoom_clamps_at_limits() {
        let mut c = canvas();
        c.set_zoom(100.0);
        assert_eq!(c.zoom, 8.0);
        c.set_zoom(0.001);
        assert_eq!(c.zoom, 0.25);
        c.zoom_by(0.5); // 0.125 -> clamped
        assert_eq!(c.zoom, 0.25);
    }

    #[test]
    fn pan_is_unclamped() {
        let mut c = canvas();
        c.pan_by(1.0e9, -1.0e9);
        assert_eq!((c.pan_x, c.pan_y), (1.0e9, -1.0e9));
    }

    #[test]
    fn zoom_around_keeps_world_focus_fixed_at_various_dpi() {
        for dpi in [1.0_f32, 1.5, 2.0] {
            for (start, factor) in [(1.0_f32, 2.0_f32), (2.5, 0.5), (0.8, 3.0)] {
                let mut c = canvas();
                c.dpi = dpi;
                c.set_zoom(start).set_pan(37.0, -12.0);
                c.screen_rect = (10.0, 20.0, 400.0, 300.0);
                let (wx, wy) = (140.0, 90.0);
                let before = c.world_to_screen(wx, wy);
                c.zoom_around(factor, wx, wy);
                let after = c.world_to_screen(wx, wy);
                assert!((before.0 - after.0).abs() < 1e-2, "dpi {dpi} x {before:?} {after:?}");
                assert!((before.1 - after.1).abs() < 1e-2, "dpi {dpi} y {before:?} {after:?}");
            }
        }
    }

    #[test]
    fn zoom_at_screen_anchors_the_cursor_at_dpi_2() {
        let mut c = canvas();
        c.dpi = 2.0;
        c.set_zoom(1.0).set_pan(20.0, 20.0);
        c.screen_rect = (100.0, 60.0, 780.0, 520.0);
        let (mx, my) = (500.0, 300.0); // physical cursor
        let world = c.screen_to_world(mx, my);
        c.zoom_at_screen(2.5, mx, my);
        let back = c.world_to_screen(world.0, world.1);
        assert!((back.0 - mx).abs() < 1e-2, "{back:?}");
        assert!((back.1 - my).abs() < 1e-2, "{back:?}");
    }

    #[test]
    fn pan_logical_divides_by_dpi() {
        let mut c = canvas();
        c.dpi = 2.0;
        c.pan_by(200.0, -80.0);
        assert_eq!(c.pan_logical(), (100.0, -40.0));
    }

    #[test]
    fn screen_world_round_trip() {
        let mut c = canvas();
        c.set_zoom(1.75).set_pan(-40.0, 15.0);
        c.screen_rect = (8.0, 8.0, 100.0, 100.0);
        let (wx, wy) = c.screen_to_world(123.0, 456.0);
        let (sx, sy) = c.world_to_screen(wx, wy);
        assert!((sx - 123.0).abs() < 1e-3);
        assert!((sy - 456.0).abs() < 1e-3);
    }
}
