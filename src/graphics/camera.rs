/// Remaps a `cgmath` (OpenGL, clip-z in `-1..1`) projection to wgpu's `0..1`
/// clip-z: `z' = 0.5 z + 0.5 w`, `w' = w`.
///
/// The `0.5` for the `+0.5 w` term belongs in **column 3** (it multiplies `w`)
/// and lands in **row 2** (the `z` output). It used to sit in column 2 / row 3,
/// which quietly rewrote `w` instead - so `z/w` never reached `1` at the far
/// plane and geometry well past `zfar` was never clipped.
#[rustfmt::skip]
pub const OPENGL_TO_WGPU_MATRIX: cgmath::Matrix4<f32> = cgmath::Matrix4::new(
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 0.5, 0.0,
    0.0, 0.0, 0.5, 1.0,
);

/// A view into the shared 3D scene. `SceneRenderer` keeps a map of these by
/// name; a `render-window` layout element (or app code via `API::camera`)
/// positions one, and the scene is drawn through it into that element's rect.
///
/// `eye`/`target`/`up` are the look-at frame; the projection is perspective
/// (vertical FOV `fovy`, degrees) unless `orthographic` is set, in which case
/// `ortho_height` world units are shown vertically. The `pan` / `orbit` /
/// `zoom` / … methods are the convenient way to move it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub eye: cgmath::Point3<f32>,
    pub target: cgmath::Point3<f32>,
    pub up: cgmath::Vector3<f32>,
    pub aspect: f32,
    pub fovy: f32,
    pub znear: f32,
    pub zfar: f32,
    /// `false` = perspective (`fovy`); `true` = orthographic (`ortho_height`).
    pub orthographic: bool,
    /// World-space height the orthographic projection shows; width follows
    /// `aspect`. Ignored while `orthographic` is `false`.
    pub ortho_height: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self::new()
    }
}

impl Camera {
    /// The default framing every camera slot starts at (roughly the values the
    /// single hard-coded camera used before `render-window` existed).
    ///
    /// `znear`/`zfar` span the scene's scale generously now that far-plane
    /// clipping actually works - set them per camera (`clip_planes`, or the
    /// `near` / `far` `render-window` keywords) for anything tighter.
    pub fn new() -> Self {
        Self {
            eye: (1000.0, 500.0, 1000.0).into(),
            target: (0.0, 120.0, 0.0).into(),
            up: cgmath::Vector3::unit_y(),
            aspect: 1.0,
            fovy: 45.0,
            znear: 1.0,
            zfar: 20_000.0,
            orthographic: false,
            ortho_height: 1200.0,
        }
    }

    /// Moves the camera to `(x, y, z)` in scene space.
    pub fn set_eye(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        self.eye = (x, y, z).into();
        self
    }
    /// Points the camera at `(x, y, z)` in scene space.
    pub fn set_target(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        self.target = (x, y, z).into();
        self
    }

    // --- readbacks -----------------------------------------------------------

    /// Distance from the eye to the target.
    pub fn distance(&self) -> f32 {
        use cgmath::InnerSpace;
        (self.target - self.eye).magnitude()
    }
    pub fn is_orthographic(&self) -> bool {
        self.orthographic
    }

    /// Unit vector the camera looks along (eye → target).
    fn forward(&self) -> cgmath::Vector3<f32> {
        use cgmath::InnerSpace;
        let f = self.target - self.eye;
        if f.magnitude2() > 1e-12 {
            f.normalize()
        } else {
            cgmath::Vector3::unit_z()
        }
    }
    /// Unit "screen right" vector.
    fn right(&self) -> cgmath::Vector3<f32> {
        use cgmath::InnerSpace;
        let r = self.forward().cross(self.up);
        if r.magnitude2() > 1e-12 {
            r.normalize()
        } else {
            cgmath::Vector3::unit_x()
        }
    }

    // --- movement ----------------------------------------------------------

    /// Slides the eye *and* target across the view plane: `+x` is screen-right,
    /// `+y` is screen-up. Units are world-space.
    pub fn pan(&mut self, x: f32, y: f32) -> &mut Self {
        use cgmath::InnerSpace;
        let right = self.right();
        let up = right.cross(self.forward()).normalize();
        let delta = right * x + up * y;
        self.eye += delta;
        self.target += delta;
        self
    }

    /// Moves the eye and target together by a world-space delta (free fly).
    pub fn translate(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        let d = cgmath::Vector3::new(x, y, z);
        self.eye += d;
        self.target += d;
        self
    }

    /// Steps the eye toward (`fraction > 0`) or away from (`fraction < 0`) the
    /// target, as a fraction of the current distance - `zoom(0.1)` moves 10%
    /// closer. Never crosses the target. An orthographic camera scales
    /// `ortho_height` instead (same visual effect).
    pub fn zoom(&mut self, fraction: f32) -> &mut Self {
        use cgmath::InnerSpace;
        if self.orthographic {
            self.ortho_height = (self.ortho_height * (1.0 - fraction)).max(0.001);
            return self;
        }
        let offset = self.eye - self.target;
        let len = (offset.magnitude() * (1.0 - fraction)).max(0.001);
        self.eye = self.target + offset.normalize() * len;
        self
    }

    /// Moves the eye a fixed world-space `amount` along the view direction
    /// (positive = toward the target), leaving the target put. Never crosses
    /// the target.
    pub fn dolly(&mut self, amount: f32) -> &mut Self {
        use cgmath::InnerSpace;
        let max = (self.target - self.eye).magnitude() - 0.001;
        self.eye += self.forward() * amount.min(max);
        self
    }

    /// Rotates the eye around the target: `yaw` radians about `up`, then `pitch`
    /// radians about the camera's right axis. Distance is preserved; pitch that
    /// would tip the view past vertical is dropped.
    pub fn orbit(&mut self, yaw: f32, pitch: f32) -> &mut Self {
        use cgmath::{InnerSpace, Matrix3, Rad};
        let up = self.up.normalize();
        let mut offset = self.eye - self.target;

        offset = Matrix3::from_axis_angle(up, Rad(yaw)) * offset;

        let axis = offset.cross(up);
        if axis.magnitude2() > 1e-9 {
            let pitched = Matrix3::from_axis_angle(axis.normalize(), Rad(pitch)) * offset;
            if pitched.normalize().dot(up).abs() < 0.999 {
                offset = pitched;
            }
        }
        self.eye = self.target + offset;
        self
    }

    /// Points the camera at `(x, y, z)` and backs the eye off - keeping the
    /// current view direction - so a sphere of `radius` there fills the view.
    pub fn frame(&mut self, x: f32, y: f32, z: f32, radius: f32) -> &mut Self {
        use cgmath::InnerSpace;
        let target = cgmath::Point3::new(x, y, z);
        let view_dir = {
            let d = self.eye - self.target;
            if d.magnitude2() > 1e-9 {
                d.normalize()
            } else {
                -self.forward()
            }
        };
        let radius = radius.max(1e-3);
        if self.orthographic {
            self.ortho_height = radius * 2.2;
        }
        let dist = radius / (0.5 * self.fovy.to_radians()).tan().max(1e-4) * 1.1;
        self.target = target;
        self.eye = target + view_dir * dist;
        self
    }

    // --- projection ------------------------------------------------------------

    /// Switches to a perspective projection with vertical field of view
    /// `fovy_degrees`.
    pub fn perspective(&mut self, fovy_degrees: f32) -> &mut Self {
        self.orthographic = false;
        self.fovy = fovy_degrees;
        self
    }

    /// Switches to an orthographic projection showing `height` world units
    /// vertically (width follows `aspect`).
    pub fn orthographic(&mut self, height: f32) -> &mut Self {
        self.orthographic = true;
        self.ortho_height = height.max(0.001);
        self
    }

    /// Sets the near / far clip-plane distances.
    pub fn clip_planes(&mut self, near: f32, far: f32) -> &mut Self {
        self.znear = near;
        self.zfar = far;
        self
    }

    pub fn build_view_projection_matrix(&self) -> cgmath::Matrix4<f32> {
        let view = cgmath::Matrix4::look_at_rh(self.eye, self.target, self.up);
        if self.orthographic {
            // `OPENGL_TO_WGPU_MATRIX` is a *perspective* depth fix-up (it assumes
            // the w-divide), so an orthographic projection has to target wgpu's
            // `0..1` clip-z range directly: RH, looking down -Z, no w-divide.
            let h = self.ortho_height.max(0.001) * 0.5;
            let w = h * self.aspect;
            let (n, f) = (self.znear, self.zfar);
            #[rustfmt::skip]
            let proj = cgmath::Matrix4::new(
                1.0 / w, 0.0,     0.0,           0.0,
                0.0,     1.0 / h, 0.0,           0.0,
                0.0,     0.0,     1.0 / (n - f), 0.0,
                0.0,     0.0,     n / (n - f),   1.0,
            );
            proj * view
        } else {
            let proj =
                cgmath::perspective(cgmath::Deg(self.fovy), self.aspect, self.znear, self.zfar);
            OPENGL_TO_WGPU_MATRIX * proj * view
        }
    }
    pub fn bindgroup_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
            label: Some("camera_bind_group_layout"),
        })
    }
}

#[repr(C)]
// This is so we can store this in a buffer
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniform {
    // We can't use cgmath with bytemuck directly, so we'll have
    // to convert the Matrix4 into a 4x4 f32 array
    view_proj: [[f32; 4]; 4],
}

impl CameraUniform {
    pub fn new() -> Self {
        use cgmath::SquareMatrix;
        Self {
            view_proj: cgmath::Matrix4::identity().into(),
        }
    }

    pub fn update_view_proj(&mut self, camera: &Camera) {
        self.view_proj = camera.build_view_projection_matrix().into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::{InnerSpace, Transform};

    fn cam() -> Camera {
        let mut c = Camera::new();
        c.eye = (0.0, 0.0, 100.0).into();
        c.target = (0.0, 0.0, 0.0).into();
        c.aspect = 1.0;
        c
    }

    #[test]
    fn orbit_preserves_distance() {
        let mut c = cam();
        let d = c.distance();
        c.orbit(0.7, 0.3);
        assert!((c.distance() - d).abs() < 1e-2);
        // a yaw of ~90° should swing the eye from +Z toward +X
        let mut c = cam();
        c.orbit(std::f32::consts::FRAC_PI_2, 0.0);
        assert!(c.eye.x.abs() > 90.0 && c.eye.z.abs() < 10.0);
    }

    #[test]
    fn orbit_pitch_clamps_at_the_pole() {
        let mut c = cam();
        c.orbit(0.0, 10.0); // way past vertical
        // never ends up looking straight down the up axis
        let dir = (c.target - c.eye).normalize();
        assert!(dir.dot(c.up).abs() < 0.999);
    }

    #[test]
    fn pan_moves_eye_and_target_together() {
        let mut c = cam();
        c.pan(5.0, 3.0);
        // +x is screen-right (world +X here), +y screen-up (world +Y)
        assert!((c.eye.x - 5.0).abs() < 1e-3 && (c.target.x - 5.0).abs() < 1e-3);
        assert!((c.eye.y - 3.0).abs() < 1e-3 && (c.target.y - 3.0).abs() < 1e-3);
        assert!((c.distance() - 100.0).abs() < 1e-3);
    }

    #[test]
    fn zoom_steps_a_fraction_of_the_distance() {
        let mut c = cam();
        c.zoom(0.25);
        assert!((c.distance() - 75.0).abs() < 1e-3);
        c.zoom(-1.0); // back away 100%
        assert!((c.distance() - 150.0).abs() < 1e-2);
        // orthographic zoom scales ortho_height instead
        let mut c = cam();
        c.orthographic(400.0).zoom(0.5);
        assert!((c.ortho_height - 200.0).abs() < 1e-3);
        assert!((c.distance() - 100.0).abs() < 1e-3);
    }

    #[test]
    fn projection_toggle() {
        let mut c = cam();
        assert!(!c.is_orthographic());
        c.orthographic(300.0);
        assert!(c.is_orthographic() && (c.ortho_height - 300.0).abs() < 1e-3);
        c.perspective(60.0);
        assert!(!c.is_orthographic() && (c.fovy - 60.0).abs() < 1e-3);
    }

    /// `OPENGL_TO_WGPU_MATRIX` must give the perspective path a real `0..1`
    /// clip-z: `0` at the near plane, `1` at the far plane, and `> 1` (i.e.
    /// clipped) for geometry beyond `zfar`.
    #[test]
    fn perspective_clip_z_maps_and_clips() {
        let mut c = cam(); // eye at (0,0,100) looking at origin
        c.perspective(45.0).clip_planes(1.0, 200.0);
        let m = c.build_view_projection_matrix();

        let ndc_z = |world_z: f32| {
            let clip: [f32; 4] = (m
                * cgmath::Vector4::new(0.0, 0.0, world_z, 1.0))
            .into();
            clip[2] / clip[3]
        };

        assert!((ndc_z(99.0)).abs() < 0.02, "near ~ 0, got {}", ndc_z(99.0)); // 1 unit from eye
        assert!((ndc_z(-100.0) - 1.0).abs() < 0.02, "far ~ 1, got {}", ndc_z(-100.0)); // 200 units
        assert!(ndc_z(-400.0) > 1.0, "beyond far is clipped, got {}", ndc_z(-400.0)); // 500 units
    }

    /// The orthographic matrix must land wgpu clip-z in `0..1` (the perspective
    /// `OPENGL_TO_WGPU_MATRIX` fix-up does not, hence the hand-built matrix).
    #[test]
    fn orthographic_clip_z_is_zero_to_one() {
        let mut c = cam();
        c.orthographic(200.0).clip_planes(1.0, 500.0);
        let m = c.build_view_projection_matrix();
        let near = m.transform_point(cgmath::Point3::new(0.0, 0.0, 99.0)); // ~1 from eye
        let far = m.transform_point(cgmath::Point3::new(0.0, 0.0, -400.0)); // 500 from eye
        assert!(near.z >= -1e-3 && near.z <= 0.05, "near z = {}", near.z);
        assert!(far.z >= 0.95 && far.z <= 1.0 + 1e-3, "far z = {}", far.z);
        // a point at the centre projects to the middle of the view
        let mid = m.transform_point(cgmath::Point3::new(0.0, 0.0, 0.0));
        assert!(mid.x.abs() < 1e-3 && mid.y.abs() < 1e-3);
    }
}
