#![cfg_attr(rustfmt, rustfmt_skip)]

use core::f32;
use cosmic_text::{
    self, Attrs, Buffer, CacheKey, Color, Fallback, Family, FontSystem, Metrics, PlatformFallback,
    Shaping, SwashCache, SwashContent, fontdb,
};
use etagere::{AllocId, AtlasAllocator, size2};
use unicode_script::Script;

use lyon::geom::Arc;
use lyon::geom::euclid::{Box2D, Point2D, Size2D, UnknownUnit};
use lyon::math::{Angle, vector};
use lyon::path::Path;
use lyon::path::builder::BorderRadii;
use lyon::tessellation::*;

use image::{DynamicImage, RgbImage};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::{Add, Div, Mul, Sub};
use wgpu::util::DeviceExt;

use telera_layout::{MeasureText, RenderCommand, Vec2};

use symbol_table::GlobalSymbol;

/// One `render-window` element resolved for the current frame: the swapchain
/// rectangle (physical pixels) to draw a scene [`Camera`](crate::Camera) into,
/// plus any camera parameters the layout set this frame. A `None` override
/// leaves that camera value untouched, so a camera's parameters can be split
/// between app code and the layout.
///
/// Produced by [`collect_render_windows`] from the frame's render commands and
/// handed to the scene renderer.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderWindow {
    pub camera: GlobalSymbol,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub eye_x: Option<f32>,
    pub eye_y: Option<f32>,
    pub eye_z: Option<f32>,
    pub target_x: Option<f32>,
    pub target_y: Option<f32>,
    pub target_z: Option<f32>,
    pub up_x: Option<f32>,
    pub up_y: Option<f32>,
    pub up_z: Option<f32>,
    pub fov: Option<f32>,
    pub near: Option<f32>,
    pub far: Option<f32>,
    /// When set, switches the camera to orthographic showing this many world
    /// units vertically. (`fov` switches it back to perspective.)
    pub ortho_height: Option<f32>,
}

impl RenderWindow {
    /// A full-target window for `camera` with no layout-set camera overrides -
    /// the implicit window used when a frame declares no `render-window` at all.
    pub fn fullscreen(camera: GlobalSymbol, w: f32, h: f32) -> Self {
        RenderWindow {
            camera,
            x: 0.0,
            y: 0.0,
            w,
            h,
            eye_x: None,
            eye_y: None,
            eye_z: None,
            target_x: None,
            target_y: None,
            target_z: None,
            up_x: None,
            up_y: None,
            up_z: None,
            fov: None,
            near: None,
            far: None,
            ortho_height: None,
        }
    }
}

/// Scans a frame's render commands for [`CustomElement::RenderWindow`] customs
/// and turns each into a [`RenderWindow`] (rect scaled from logical to physical
/// pixels by `dpi_scale`). Called every frame from `API::redraw_viewport` -
/// unlike [`UIRenderer::render_layout`] it must run even when the cached UI
/// texture is reused, because the scene is redrawn every frame.
pub fn collect_render_windows(
    commands: &[RenderCommand<'_, UIImageDescriptor, CustomElement, CustomLayoutSettings>],
    dpi_scale: f32,
) -> Vec<RenderWindow> {
    commands
        .iter()
        .filter_map(|command| {
            let RenderCommand::Custom(c) = command else {
                return None;
            };
            let CustomElement::RenderWindow {
                camera,
                eye_x,
                eye_y,
                eye_z,
                target_x,
                target_y,
                target_z,
                up_x,
                up_y,
                up_z,
                fov,
                near,
                far,
                ortho_height,
            } = *c.data
            else {
                return None;
            };
            Some(RenderWindow {
                camera,
                x: c.bounding_box.x * dpi_scale,
                y: c.bounding_box.y * dpi_scale,
                w: c.bounding_box.width * dpi_scale,
                h: c.bounding_box.height * dpi_scale,
                eye_x,
                eye_y,
                eye_z,
                target_x,
                target_y,
                target_z,
                up_x,
                up_y,
                up_z,
                fov,
                near,
                far,
                ortho_height,
            })
        })
        .collect()
}

/// The custom layout elements this renderer knows how to draw, threaded through
/// the layout engine as its custom-element payload.
///
/// Every positional field is a normalised `0.0..1.0` fraction of the element's
/// bounding box (`0,0` = top-left, `1,1` = bottom-right), so a shape tracks its
/// box as the layout resizes. `radius` is a fraction of `min(width, height) / 2`
/// (`1.0` = inscribed). Angles are degrees, clockwise from the 3 o'clock
/// position. `thickness` is a logical-pixel stroke width (scaled by `dpi_scale`
/// at draw time).
///
/// This is the fully-resolved payload the renderer reads; the parser builds a
/// [`CustomElementSpec`](crate::CustomElementSpec) of `DataSrc<f32>` and
/// resolves it into one of these each frame.
#[derive(Debug, Default, Clone, PartialEq)]
pub enum CustomElement {
    /// Filled disc inscribed in the bounding box.
    #[default]
    Circle,
    /// Unfilled circle (stroked outline), inscribed in the bounding box.
    Ring { thickness: f32 },
    /// Straight line from `(from_x, from_y)` to `(to_x, to_y)`.
    Line {
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        thickness: f32,
    },
    /// Circular arc centred at `(center_x, center_y)`, swept from `start_angle`
    /// to `end_angle`.
    Arc {
        center_x: f32,
        center_y: f32,
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        thickness: f32,
    },
    /// Cubic bezier from `(from_x, from_y)` to `(to_x, to_y)` with control
    /// points `(ctrl1_x, ctrl1_y)` and `(ctrl2_x, ctrl2_y)`.
    Bezier {
        from_x: f32,
        from_y: f32,
        ctrl1_x: f32,
        ctrl1_y: f32,
        ctrl2_x: f32,
        ctrl2_y: f32,
        to_x: f32,
        to_y: f32,
        thickness: f32,
    },
    /// A hole in the UI layer that the 3D scene is drawn into, through the
    /// [`Camera`](crate::Camera) named `camera`. Each `Some` field overrides
    /// that camera parameter for this frame; `None` leaves it as the camera
    /// (app code, or a previous frame) last set it. See [`RenderWindow`].
    RenderWindow {
        camera: GlobalSymbol,
        eye_x: Option<f32>,
        eye_y: Option<f32>,
        eye_z: Option<f32>,
        target_x: Option<f32>,
        target_y: Option<f32>,
        target_z: Option<f32>,
        up_x: Option<f32>,
        up_y: Option<f32>,
        up_z: Option<f32>,
        fov: Option<f32>,
        near: Option<f32>,
        far: Option<f32>,
        ortho_height: Option<f32>,
    },
}

/// Identifies a shaped run of text. Everything that changes cosmic-text's glyph
/// output (or the depth we bake into it) is folded in, so a hit on this key can
/// reuse an already-shaped [`glyphon::Buffer`] as-is. `content` is a hash of the
/// string bytes; `font_size` / `line_height` are `f32::to_bits` of the
/// dpi-scaled pixel values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextKey {
    content: u64,
    font_id: u16,
    letter_spacing: u16,
    font_size: u32,
    line_height: u32,
}

/// `(start_byte, end_byte, x_advance)` per glyph of a shaped text run.
type GlyphAdvances = Vec<(u32, u32, f32)>;

pub struct TextLine {
    key: TextKey,
    left: f32,
    top: f32,
    color: Color,
    bounds: Option<(UIPosition, UIPosition)>,
    /// Depth (the `z` of the clay TEXT command), applied to every glyph quad.
    z: f32,
}

/// A shaped line held in [`UIRenderer::line_cache`], plus the frame it was last
/// used on so `frame_tick` can drop stale entries.
struct CachedLine {
    buffer: Buffer,
    generation: u64,
}

/// One rasterised glyph packed into [`GlyphAtlas`].
#[derive(Clone, Copy)]
struct GlyphSlot {
    /// `None` for zero-area glyphs (whitespace) - recorded so we don't re-probe.
    alloc: Option<AllocId>,
    /// Pixel rect in the atlas texture.
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    /// swash placement, relative to the pen position / baseline.
    left: i32,
    top: i32,
    /// `true` = full-colour bitmap (emoji), sampled straight; `false` = coverage
    /// mask in the red channel, tinted by the text colour.
    color: bool,
    generation: u64,
}

/// Persistent atlas of rasterised glyphs. Replaces glyphon's `TextAtlas` +
/// `TextRenderer` + `Viewport` - glyphs are drawn as ordinary quads in the UI
/// pipeline (`texture` tag `2`/`3`).
struct GlyphAtlas {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    allocator: AtlasAllocator,
    size: u32,
    map: HashMap<CacheKey, GlyphSlot>,
}

/// A run of glyph-quad indices sharing a scissor rect, drawn (with the glyph
/// atlas bound) after the normal render batches in `end`.
struct TextBatch {
    begin: u32,
    end: u32,
    scissor: Option<(UIPosition, UIPosition)>,
}

/// cosmic-text font fallback with the app's [`register_font`](UIRenderer::register_font)
/// families prepended to the platform's common list, so a loaded emoji/symbol
/// font is preferred over whatever system text font happens to carry an outline
/// glyph for the same codepoint.
struct TeleraFallback {
    /// Preferred families, then the platform's `common_fallback`. Leaked to
    /// `'static` (the trait demands it); a font is loaded once at startup.
    common: Vec<&'static str>,
}

impl TeleraFallback {
    fn new(preferred: &[&'static str]) -> Self {
        let mut common = preferred.to_vec();
        common.extend_from_slice(PlatformFallback.common_fallback());
        Self { common }
    }
}

impl Fallback for TeleraFallback {
    fn common_fallback(&self) -> &[&'static str] {
        &self.common
    }
    fn forbidden_fallback(&self) -> &[&'static str] {
        PlatformFallback.forbidden_fallback()
    }
    fn script_fallback(&self, script: Script, locale: &str) -> &[&'static str] {
        PlatformFallback.script_fallback(script, locale)
    }
}

#[derive(Debug)]
pub enum CustomLayoutSettings {
    Radii {
        top_left: f32,
        top_right: f32,
        bottom_left: f32,
        bottom_right: f32,
    },
    Inverted,
    /// The `` `shader` `` effects stacked on one element (drop shadow, raised
    /// edge, ...), in the order they were written. Carried on clay's `userData`;
    /// `render_layout` splits them into a "behind the fill" group (shadow, blur)
    /// and an "over the fill" group (bevel, glow, custom) and emits each.
    Effects(Vec<ResolvedShader>),
}

/// Which fragment effect a [`ResolvedShader`] draws. The three SDF effects share
/// one pipeline (`ui_effects.wgsl`, switched on this discriminant);
/// [`EffectKind::Blur`] runs in its own pass; [`EffectKind::Custom`] selects a
/// user shader compiled into [`UIRenderer::effect_pipelines`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum EffectKind {
    DropShadow = 0,
    RaisedEdge = 1,
    InnerGlow = 2,
    Blur = 3,
    Custom = 255,
}

impl EffectKind {
    /// Whether this effect paints *behind* the element's fill (a cast shadow /
    /// backdrop blur) rather than over it (bevel, glow, custom).
    fn behind(self) -> bool {
        matches!(self, EffectKind::DropShadow | EffectKind::Blur)
    }
}

/// The fully-resolved (all `f32`) effect payload for one element this frame -
/// the renderer-side twin of the parser's `ShaderSpec`. `params` / `params2`
/// are packed per [`EffectKind`] (for `Custom`, they are `shader-param-1..8`
/// verbatim). `custom` is the interned custom-shader name, unused for built-ins.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ResolvedShader {
    pub kind: EffectKind,
    pub custom: Option<GlobalSymbol>,
    pub params: [f32; 4],
    pub params2: [f32; 4],
}

#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct UIColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl UIColor {
    /// Opaque colour from clay's 0-255 channels.
    fn from_clay(c: telera_layout::Color) -> Self {
        Self { r: c.r / 255.0, g: c.g / 255.0, b: c.b / 255.0, a: c.a / 255.0 }
    }
    const WHITE: Self = Self { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
}

#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct UIPosition {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl From<Point2D<f32, UnknownUnit>> for UIPosition {
    fn from(val: Point2D<f32, UnknownUnit>) -> Self {
        let p = val.to_tuple();
        UIPosition {
            x: p.0,
            y: p.1,
            z: 0.1,
        }
    }
}

impl UIPosition {
    pub fn new() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }

    pub fn xy(x: f32, y: f32) -> Self {
        Self { x, y, z: 0.0 }
    }

    pub fn rotate(&mut self, mut degrees: f32) -> UIPosition {
        degrees = -degrees;

        degrees *= std::f32::consts::PI / 180.0;

        let (sn, cs) = degrees.sin_cos();

        let new = UIPosition {
            x: self.x * cs - self.y * sn,
            y: self.x * sn + self.y * cs,
            z: self.z,
        };
        *self = new;

        *self
    }

    pub fn with_x(&mut self, x: f32) -> UIPosition {
        UIPosition {
            x: self.x + x,
            y: self.y,
            z: self.z,
        }
    }

    pub fn with_y(&mut self, y: f32) -> UIPosition {
        UIPosition {
            x: self.x,
            y: self.y + y,
            z: self.z,
        }
    }
}

impl Add for UIPosition {
    type Output = UIPosition;

    fn add(self, other: UIPosition) -> UIPosition {
        UIPosition {
            x: self.x + other.x,
            y: self.y + other.y,
            z: self.z,
        }
    }
}

impl Add<f32> for UIPosition {
    type Output = UIPosition;

    fn add(self, rhs: f32) -> UIPosition {
        UIPosition {
            x: self.x + rhs,
            y: self.y + rhs,
            z: self.z,
        }
    }
}

impl Sub<f32> for UIPosition {
    type Output = UIPosition;

    fn sub(self, rhs: f32) -> UIPosition {
        UIPosition {
            x: self.x - rhs,
            y: self.y - rhs,
            z: self.z,
        }
    }
}

impl Mul<f32> for UIPosition {
    type Output = UIPosition;

    fn mul(self, rhs: f32) -> Self::Output {
        UIPosition {
            x: self.x * rhs,
            y: self.y * rhs,
            z: self.z,
        }
    }
}

impl Div<f32> for UIPosition {
    type Output = UIPosition;

    fn div(self, rhs: f32) -> Self::Output {
        UIPosition {
            x: self.x / rhs,
            y: self.y / rhs,
            z: self.z,
        }
    }
}

#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct UIVertex {
    pub position: UIPosition,
    /// Fragment mode: `0` solid (uses `color` incl. alpha), `1` image
    /// (`textureSample(uv)`), `2` glyph mask (`color.rgb`, alpha *= atlas `.r`),
    /// `3` glyph colour bitmap / emoji (`textureSample(uv)` straight).
    pub texture: u32,
    pub color: UIColor,
    /// Atlas texture coordinates for modes 1/2/3; ignored for mode 0.
    pub uv: [f32; 2],
}

impl UIVertex {
    pub fn new() -> Self {
        Self {
            position: UIPosition {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            texture: 0,
            color: UIColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            uv: [0.0, 0.0],
        }
    }

    pub fn get_layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTR: [wgpu::VertexAttribute; 4] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32, 2 => Float32x4, 3 => Float32x2];

        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<UIVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTR,
        }
    }
}

/// One vertex of an effect quad (see `effect_prelude.wgsl`). Only elements that
/// carry a `` `shader` `` config emit these, into a stream separate from
/// `UIVertex` so the common rect/border/glyph path stays lean. `rect_center` /
/// `rect_half` / `radii` describe the *element* box (not the expanded quad) so
/// the fragment shader can evaluate the rounded-box SDF.
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct EffectVertex {
    pub position: UIPosition,
    pub effect: u32,
    pub color: UIColor,
    pub uv: [f32; 2],
    pub rect_center: [f32; 2],
    pub rect_half: [f32; 2],
    pub radii: [f32; 4],
    pub params: [f32; 4],
    pub params2: [f32; 4],
}

impl EffectVertex {
    pub fn get_layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTR: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Uint32, 2 => Float32x4, 3 => Float32x2,
            4 => Float32x2, 5 => Float32x2, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4
        ];
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<EffectVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTR,
        }
    }
}

/// The geometry an element's `` `shader` `` effects are emitted against:
/// physical-px box `[x, y, w, h]` at depth `z`, corner `radii` (`[tl, tr, bl,
/// br]`), and the element's own fill `base` (what `raised-edge` / custom shaders
/// read as `in.color`).
#[derive(Copy, Clone, Debug)]
struct EffectBox {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    z: f32,
    radii: [f32; 4],
    base: UIColor,
}

/// One `` `shader` *blur* `` element, collected during `render_layout` and drawn
/// by the backdrop-blur sub-pass (`blur.wgsl`) after the main UI pass. All px.
#[derive(Copy, Clone, Debug)]
pub struct BlurRegion {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub radii: [f32; 4],
    /// Blur kernel radius, physical px.
    pub radius: f32,
    /// Normalised rgba tint mixed into the blurred result.
    pub tint: [f32; 4],
}

#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct BlurVertex {
    position: [f32; 2],
    rect_center: [f32; 2],
    rect_half: [f32; 2],
    radii: [f32; 4],
    params: [f32; 4],
    tint: [f32; 4],
}

impl BlurVertex {
    fn get_layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTR: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Float32x2,
            3 => Float32x4, 4 => Float32x4, 5 => Float32x4
        ];
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<BlurVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTR,
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SizeUniform {
    x: f32,
    y: f32,
}

pub enum RenderBatch {
    Basic {
        begin: u32,
        end: u32,
    },
    Scissor {
        begin: u32,
        end: u32,
        position: UIPosition,
        size: UIPosition,
    },
    Atlas {
        begin: u32,
        end: u32,
        atlas: String,
        /// Clip rect (physical px) when this image batch was emitted inside a
        /// `scroll` container; `None` when unclipped. Without this, images
        /// drew straight through their container's clip region.
        scissor: Option<(UIPosition, UIPosition)>,
    },
    /// A run of `effect_indices` drawn by an effect pipeline. `key` picks the
    /// pipeline: `BuiltIn` = the shared SDF pipeline, `Custom(sym)` = a
    /// user shader from `effect_pipelines`.
    Effect {
        begin: u32,
        end: u32,
        key: EffectKey,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum EffectKey {
    BuiltIn,
    Custom(GlobalSymbol),
}

#[repr(C)]
pub struct UIRenderer {
    pub vertices: Vec<UIVertex>,
    pub indices: Vec<u32>,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,

    pub batches: Vec<RenderBatch>,
    pub batch_index_begin: u32,
    pub batch_index_end: u32,

    pub scissor_active: bool,
    pub scissor_position: UIPosition,
    pub scissor_size: UIPosition,

    pub staged_images: Vec<(String, DynamicImage)>,
    pub atlas_map: HashMap<String, wgpu::BindGroup>,
    pub active_atlas: String,

    pub render_pipeline: Option<wgpu::RenderPipeline>,

    /// Effect quad stream (drop shadow / raised edge / inner glow / custom
    /// shaders) - separate from `vertices`/`indices` so the common path is
    /// untouched. Drawn by `effect_pipeline` (built-ins) or an entry of
    /// `effect_pipelines` (custom), dispatched from `RenderBatch::Effect`.
    pub effect_vertices: Vec<EffectVertex>,
    pub effect_indices: Vec<u32>,
    effect_vertex_buffer: wgpu::Buffer,
    effect_index_buffer: wgpu::Buffer,
    effect_batch_index_begin: u32,
    effect_batch_index_end: u32,
    /// Shared pipeline for the built-in SDF effects (`ui_effects.wgsl`).
    effect_pipeline: Option<wgpu::RenderPipeline>,
    /// One pipeline per custom shader, keyed by its interned directive name.
    effect_pipelines: HashMap<GlobalSymbol, wgpu::RenderPipeline>,
    /// Custom shaders whose source is validated but not yet compiled (the
    /// directory scan runs before `build_shaders`). Drained once the pipeline
    /// machinery exists. `(name, full concatenated wgsl)`.
    pub staged_shaders: Vec<(GlobalSymbol, String)>,
    /// Cached so `compile_effect_shader` can build a pipeline after
    /// `build_shaders` (hot reload of a `.wgsl` file).
    effect_surface_format: Option<wgpu::TextureFormat>,
    effect_msaa_count: u32,

    /// `` `shader` *blur* `` elements collected this frame, drawn by the
    /// backdrop-blur sub-pass (`render_blur`) after the main UI pass.
    pub blur_regions: Vec<BlurRegion>,
    blur_pipeline: Option<wgpu::RenderPipeline>,
    blur_vertex_buffer: wgpu::Buffer,
    blur_index_buffer: wgpu::Buffer,

    /// Blits the per-window cached UI texture over the 3D scene each frame.
    /// Built in `build_shaders` (it needs the surface format); the layout and
    /// sampler it uses are format-independent and made in `new`.
    composite_pipeline: Option<wgpu::RenderPipeline>,
    composite_bind_group_layout: wgpu::BindGroupLayout,
    composite_sampler: wgpu::Sampler,

    pub font_system: FontSystem,
    swash_cache: SwashCache,
    pub measurement_buffer: Buffer,
    /// Every font handed to [`Self::register_font`], kept so the whole
    /// `FontSystem` can be rebuilt (with the fallback chain updated) each time
    /// one is added.
    loaded_fonts: Vec<(u16, std::sync::Arc<Vec<u8>>)>,
    /// Text runs collected during the command loop; turned into glyph quads in
    /// `emit_text` (appended to `vertices`/`indices`), then cleared.
    pub lines: Vec<TextLine>,
    /// Persistent rasterised-glyph atlas (R in an Rgba8 texture for coverage
    /// masks, full RGBA for colour glyphs). Built in `build_shaders`.
    glyph_atlas: Option<GlyphAtlas>,
    /// Index ranges into `indices` for the glyph quads emitted this frame, with
    /// the per-line scissor to apply. Drawn after the normal batches in `end`.
    text_batches: Vec<TextBatch>,

    /// Bumped once per frame by [`Self::frame_tick`]; stamps cache entries so
    /// stale ones can be swept.
    text_generation: u64,
    /// `measure_text` results, keyed by the exact word + style clay asked about.
    /// Survives clay's own per-element cache misses (which fire every frame for
    /// any text whose bytes change), so a given word is only shaped once.
    measure_cache: HashMap<TextKey, (Vec2, u64)>,
    /// Per-glyph advances `(start_byte, end_byte, width)` for a whole text run,
    /// keyed by the element's full string + style. clay measures a run one word
    /// at a time; this lets all those word queries share a single shaping of the
    /// run instead of one shaping per word.
    run_cache: HashMap<TextKey, (GlyphAdvances, u64)>,
    /// Shaped buffers for `draw_text`, so an unchanged line is not re-shaped on
    /// every `ui_dirty` frame and `render_text` can borrow the buffer directly.
    line_cache: HashMap<TextKey, CachedLine>,
    /// `font-id` -> font family name, populated by [`Self::register_font`].
    /// Unmapped ids fall back to the system sans-serif.
    font_families: HashMap<u16, Box<str>>,

    pub viewport_size: (f32, f32),
    pub size_buffer: wgpu::Buffer,
    pub size_bind_group: wgpu::BindGroup,
    size_bind_group_layout: wgpu::BindGroupLayout,

    pub dpi_scale: f32,
}

impl MeasureText for UIRenderer {
    fn measure_text(
        &mut self,
        text: &str,
        base: &str,
        text_config: telera_layout::TextConfig,
    ) -> Vec2 {
        let font_size = text_config.font_size as f32 * self.dpi_scale;
        let line_height = match text_config.line_height {
            0 => font_size * 1.2,
            _ => text_config.line_height as f32 * self.dpi_scale,
        };
        let word_key = Self::text_key(
            text,
            text_config.font_id,
            text_config.letter_spacing,
            font_size,
            line_height,
        );

        if let Some((size, generation)) = self.measure_cache.get_mut(&word_key) {
            *generation = self.text_generation;
            return *size;
        }

        // clay measures a text element one whitespace-delimited word at a time,
        // but every one of those slices points into the same `base` string.
        // Shape `base` once (cache it), then answer this word by summing the
        // advances of the glyphs that fall inside its byte range.
        let offset = (text.as_ptr() as usize).wrapping_sub(base.as_ptr() as usize);
        let within_base = offset <= base.len()
            && offset
                .checked_add(text.len())
                .is_some_and(|end| end <= base.len());

        let width: f32 = if within_base {
            let run_key = Self::text_key(
                base,
                text_config.font_id,
                text_config.letter_spacing,
                font_size,
                line_height,
            );
            if !self.run_cache.contains_key(&run_key) {
                let advances = self.shape_run_advances(base, &text_config, font_size, line_height);
                self.run_cache
                    .insert(run_key, (advances, self.text_generation));
            }
            let (advances, generation) = self.run_cache.get_mut(&run_key).unwrap();
            *generation = self.text_generation;
            let end = offset + text.len();
            advances
                .iter()
                .filter(|(start, glyph_end, _)| {
                    *start as usize >= offset && *glyph_end as usize <= end
                })
                .map(|(_, _, w)| *w)
                .sum()
        } else {
            self.shape_run_advances(text, &text_config, font_size, line_height)
                .iter()
                .map(|(_, _, w)| *w)
                .sum()
        };

        let size = Vec2 {
            x: width / self.dpi_scale,
            y: line_height / self.dpi_scale,
        };
        self.measure_cache
            .insert(word_key, (size, self.text_generation));
        size
    }
}

impl UIRenderer {
    /// Hashes a string + its style into a [`TextKey`]. Depth is no longer part of
    /// the key - it is applied per-quad from the `TextLine` at emit time - so the
    /// same string at different z shares one shaped buffer.
    fn text_key(
        text: &str,
        font_id: u16,
        letter_spacing: u16,
        font_size: f32,
        line_height: f32,
    ) -> TextKey {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        TextKey {
            content: hasher.finish(),
            font_id,
            letter_spacing,
            font_size: font_size.to_bits(),
            line_height: line_height.to_bits(),
        }
    }

    /// Shared attribute set for both the measurement and draw paths - so a
    /// string is measured with the exact font it is later drawn with. `font_size`
    /// is the dpi-scaled pixel size, used to turn clay's pixel `letter_spacing`
    /// into cosmic-text's em value.
    fn text_attrs<'a>(
        font_families: &'a HashMap<u16, Box<str>>,
        font_id: u16,
        letter_spacing: u16,
        font_size: f32,
    ) -> Attrs<'a> {
        let family = match font_families.get(&font_id) {
            Some(name) => Family::Name(name),
            None => Family::SansSerif,
        };
        let mut attrs = Attrs::new().family(family);
        if letter_spacing != 0 {
            attrs = attrs.letter_spacing(letter_spacing as f32 / font_size.max(1.0));
        }
        attrs
    }

    /// Pure-ASCII runs can skip rustybuzz entirely (`Shaping::Basic` does a
    /// direct charmap + advance lookup); anything with combining marks / complex
    /// scripts still needs the full shaper.
    fn shaping_for(text: &str) -> Shaping {
        if text.is_ascii() {
            Shaping::Basic
        } else {
            Shaping::Advanced
        }
    }

    /// Shapes `s` once through the shared measurement buffer and returns each
    /// glyph's `(start_byte, end_byte, advance)` (byte offsets into `s`). Used to
    /// measure a whole element's text with a single shaping call and slice out
    /// per-word widths.
    fn shape_run_advances(
        &mut self,
        s: &str,
        cfg: &telera_layout::TextConfig,
        font_size: f32,
        line_height: f32,
    ) -> GlyphAdvances {
        self.measurement_buffer.set_metrics_and_size(
            Metrics { font_size, line_height },
            None,
            None,
        );
        {
            let attrs = Self::text_attrs(
                &self.font_families,
                cfg.font_id,
                cfg.letter_spacing,
                font_size,
            );
            self.measurement_buffer
                .set_text(s, &attrs, Self::shaping_for(s), None);
        }
        self.measurement_buffer
            .shape_until_scroll(&mut self.font_system, false);

        let mut advances = Vec::new();
        for run in self.measurement_buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                advances.push((glyph.start as u32, glyph.end as u32, glyph.w));
            }
        }
        advances
    }

    /// Advances the frame counter and evicts text-cache entries (and atlas
    /// glyphs) that have not been touched recently. Call once per frame before
    /// the layout pass.
    pub fn frame_tick(&mut self) {
        self.text_generation = self.text_generation.wrapping_add(1);
        let now = self.text_generation;
        const KEEP_FRAMES: u64 = 10;
        // Glyphs churn less than lines; keep them a while longer.
        const KEEP_GLYPHS: u64 = 240;
        self.measure_cache
            .retain(|_, (_, generation)| now.wrapping_sub(*generation) <= KEEP_FRAMES);
        self.run_cache
            .retain(|_, (_, generation)| now.wrapping_sub(*generation) <= KEEP_FRAMES);
        self.line_cache
            .retain(|_, line| now.wrapping_sub(line.generation) <= KEEP_FRAMES);
        if let Some(atlas) = self.glyph_atlas.as_mut() {
            let allocator = &mut atlas.allocator;
            atlas.map.retain(|_, slot| {
                if now.wrapping_sub(slot.generation) <= KEEP_GLYPHS {
                    return true;
                }
                if let Some(id) = slot.alloc {
                    allocator.deallocate(id);
                }
                false
            });
        }
    }

    /// Registers `data` (the bytes of a `.ttf` / `.otf` file) under `font_id` so
    /// TML `font-id` / [`TextConfig::font_id`] selects it. The face is also added
    /// to the fallback chain ahead of the platform defaults, so a loaded emoji /
    /// symbol font is used automatically without setting `font-id`. Unregistered
    /// ids fall back to the system sans-serif.
    pub fn register_font(&mut self, font_id: u16, data: Vec<u8>) {
        self.loaded_fonts.push((font_id, std::sync::Arc::new(data)));
        self.rebuild_font_system();
    }

    /// Rebuilds `font_system` from the system fonts plus every
    /// [`register_font`](Self::register_font)ed face, with a [`TeleraFallback`]
    /// that prefers those faces. cosmic-text fixes the fallback list at
    /// construction, so a full rebuild is the way to update it - only ever done
    /// at startup, once per loaded font.
    fn rebuild_font_system(&mut self) {
        let locale = self.font_system.locale().to_string();

        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        // Match `FontSystem::new`'s generic-family defaults.
        db.set_monospace_family("Noto Sans Mono");
        db.set_sans_serif_family("Open Sans");
        db.set_serif_family("DejaVu Serif");

        self.font_families.clear();
        let mut preferred: Vec<&'static str> = Vec::new();
        for (font_id, data) in &self.loaded_fonts {
            let ids = db.load_font_source(fontdb::Source::Binary(data.clone()));
            if let Some(face_id) = ids.first()
                && let Some(face) = db.face(*face_id)
                && let Some((name, _)) = face.families.first()
            {
                let name = name.clone().into_boxed_str();
                let leaked: &'static str = Box::leak(name.clone());
                self.font_families.insert(*font_id, name);
                if !preferred.contains(&leaked) {
                    preferred.push(leaked);
                }
            }
        }

        self.font_system =
            FontSystem::new_with_locale_and_db_and_fallback(locale, db, TeleraFallback::new(&preferred));
        self.measurement_buffer = Buffer::new(&mut self.font_system, Metrics::new(30.0, 42.0));

        // Every cache keyed by the old font ids / shaping is now stale.
        self.measure_cache.clear();
        self.run_cache.clear();
        self.line_cache.clear();
        if let Some(atlas) = self.glyph_atlas.as_mut() {
            atlas.map.clear();
            atlas.allocator =
                AtlasAllocator::new(size2(atlas.size as i32, atlas.size as i32));
        }
    }
}

impl UIRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let mut atlas_dictionary = HashMap::<String, wgpu::BindGroup>::new();
        atlas_dictionary.insert(
            "default_atlas".to_string(),
            wgpu::BindGroup::create_atlas(
                DynamicImage::ImageRgb8(RgbImage::new(10, 10)),
                device,
                queue,
            ),
        );
        let active_atlas = "defualt_atlas".to_string();

        let size_bind_group_layout =
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
                label: Some("ui_renderer_size_bind_group_layout"),
            });

        let size_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui_renderer_size_buffer"),
            contents: bytemuck::cast_slice(&[SizeUniform { x: 1.0, y: 1.0 }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let size_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &size_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: size_buffer.as_entire_binding(),
            }],
            label: Some("ui_renderer_size_bind_group"),
        });

        let vertices = [UIVertex::new(); 3].to_vec();
        let indices = [u32::MIN; 3].to_vec();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui_vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui_indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        });

        let effect_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui_effect_vertices"),
            contents: bytemuck::cast_slice(&[<EffectVertex as bytemuck::Zeroable>::zeroed(); 3]),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let effect_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui_effect_indices"),
            contents: bytemuck::cast_slice(&[0u32; 3]),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        });
        let blur_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui_blur_vertices"),
            contents: bytemuck::cast_slice(&[<BlurVertex as bytemuck::Zeroable>::zeroed(); 4]),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let blur_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui_blur_indices"),
            contents: bytemuck::cast_slice(&[0u32; 6]),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        });

        let composite_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ui_composite_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let composite_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui_composite_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let measurement_buffer = Buffer::new(&mut font_system, Metrics::new(30.0, 42.0));

        Self {
            batches: Vec::<RenderBatch>::new(),
            batch_index_begin: 0,
            batch_index_end: 0,
            scissor_active: false,
            scissor_position: UIPosition::new(),
            scissor_size: UIPosition::new(),

            vertex_buffer,
            vertices,
            indices,
            index_buffer,

            staged_images: Vec::<(String, DynamicImage)>::new(),
            atlas_map: atlas_dictionary,
            active_atlas,

            render_pipeline: None,
            effect_vertices: Vec::new(),
            effect_indices: Vec::new(),
            effect_vertex_buffer,
            effect_index_buffer,
            effect_batch_index_begin: 0,
            effect_batch_index_end: 0,
            effect_pipeline: None,
            effect_pipelines: HashMap::new(),
            staged_shaders: Vec::new(),
            effect_surface_format: None,
            effect_msaa_count: 1,
            blur_regions: Vec::new(),
            blur_pipeline: None,
            blur_vertex_buffer,
            blur_index_buffer,
            composite_pipeline: None,
            composite_bind_group_layout,
            composite_sampler,

            font_system,
            swash_cache,
            measurement_buffer,
            loaded_fonts: Vec::new(),
            lines: Vec::<TextLine>::new(),
            glyph_atlas: None,
            text_batches: Vec::new(),
            text_generation: 0,
            measure_cache: HashMap::new(),
            run_cache: HashMap::new(),
            line_cache: HashMap::new(),
            font_families: HashMap::new(),
            dpi_scale: 1.0,
            viewport_size: (1.0, 1.0),
            size_buffer,
            size_bind_group,
            size_bind_group_layout,
        }
    }

    fn update_buffers(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let slice = bytemuck::cast_slice(self.vertices.as_slice());
        if slice.len() > self.vertex_buffer.size() as usize {
            let vertex_buffer_desctriptor = wgpu::util::BufferInitDescriptor {
                label: Some("ui_vertices"),
                contents: bytemuck::cast_slice(&self.vertices),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            };
            self.vertex_buffer = device.create_buffer_init(&vertex_buffer_desctriptor);
        } else {
            queue.write_buffer(&self.vertex_buffer, 0, slice);
        }

        let slice = bytemuck::cast_slice(self.indices.as_slice());
        if slice.len() > self.index_buffer.size() as usize {
            let index_buffer_descriptor = wgpu::util::BufferInitDescriptor {
                label: Some("ui_indices"),
                contents: bytemuck::cast_slice(&self.indices),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            };
            self.index_buffer = device.create_buffer_init(&index_buffer_descriptor);
        } else {
            queue.write_buffer(&self.index_buffer, 0, slice);
        }

        if !self.effect_indices.is_empty() {
            let slice = bytemuck::cast_slice(self.effect_vertices.as_slice());
            if slice.len() > self.effect_vertex_buffer.size() as usize {
                self.effect_vertex_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("ui_effect_vertices"),
                        contents: slice,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    });
            } else {
                queue.write_buffer(&self.effect_vertex_buffer, 0, slice);
            }

            let slice = bytemuck::cast_slice(self.effect_indices.as_slice());
            if slice.len() > self.effect_index_buffer.size() as usize {
                self.effect_index_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("ui_effect_indices"),
                        contents: slice,
                        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    });
            } else {
                queue.write_buffer(&self.effect_index_buffer, 0, slice);
            }
        }
    }

    pub fn build_shaders(
        &mut self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        config: &wgpu::SurfaceConfiguration,
        multi_sample_count: u32,
    ) {
        let mut ui_pipeline_builder = UIPipeline::new(config.format);

        ui_pipeline_builder.add_buffer_layout(UIVertex::get_layout());

        self.render_pipeline = Some(ui_pipeline_builder.build_pipeline(
            device,
            &self.size_bind_group_layout,
            wgpu::MultisampleState {
                count: multi_sample_count,
                mask: 1,
                alpha_to_coverage_enabled: false,
            },
        ));

        self.composite_pipeline = Some(build_composite_pipeline(
            device,
            config.format,
            &self.composite_bind_group_layout,
        ));

        self.effect_surface_format = Some(config.format);
        self.effect_msaa_count = multi_sample_count;
        self.effect_pipeline = Some(build_effect_pipeline(
            device,
            config.format,
            multi_sample_count,
            &self.size_bind_group_layout,
            &effect_source(include_str!("ui_effects.wgsl")),
            "UI Effect Pipeline",
        ));
        self.drain_staged_shaders(device);

        self.blur_pipeline = Some(build_blur_pipeline(
            device,
            config.format,
            &self.size_bind_group_layout,
            &self.composite_bind_group_layout,
        ));

        self.glyph_atlas = Some(GlyphAtlas::new(device));
    }

    /// Compiles any custom shaders queued before the pipeline machinery existed
    /// (the layout-directory scan runs before `build_shaders`). Their source is
    /// already prelude-concatenated and naga-validated (see
    /// `API::register_ui_shader`).
    fn drain_staged_shaders(&mut self, device: &wgpu::Device) {
        if self.effect_surface_format.is_none() {
            return;
        }
        for (sym, wgsl) in std::mem::take(&mut self.staged_shaders) {
            self.compile_effect_shader(device, sym, &wgsl);
        }
    }

    /// Builds (or replaces) the pipeline for one custom shader. `wgsl` must
    /// already be `effect_source(body)` and have passed naga validation.
    pub fn compile_effect_shader(
        &mut self,
        device: &wgpu::Device,
        name: GlobalSymbol,
        wgsl: &str,
    ) {
        let Some(format) = self.effect_surface_format else {
            self.staged_shaders.push((name, wgsl.to_string()));
            return;
        };
        let pipeline = build_effect_pipeline(
            device,
            format,
            self.effect_msaa_count,
            &self.size_bind_group_layout,
            wgsl,
            "UI Custom Effect Pipeline",
        );
        self.effect_pipelines.insert(name, pipeline);
    }

    /// True once `build_shaders` has run and custom shaders can be compiled
    /// immediately rather than staged.
    pub fn effects_ready(&self) -> bool {
        self.effect_pipeline.is_some()
    }

    /// Layout + sampler a [`UiSurface`](crate::graphics::textures::UiSurface)
    /// binds its colour texture into for compositing. Format-independent, so
    /// they exist from `new` (before `build_shaders`).
    pub fn composite_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.composite_bind_group_layout
    }
    pub fn composite_sampler(&self) -> &wgpu::Sampler {
        &self.composite_sampler
    }

    /// Draws the cached UI texture (via `bind_group`) over whatever is already in
    /// the render pass's colour target. The pass must have no depth attachment.
    pub fn composite(&self, render_pass: &mut wgpu::RenderPass, bind_group: &wgpu::BindGroup) {
        let Some(pipeline) = &self.composite_pipeline else {
            return;
        };
        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    /// Whether `render_blur` has anything to draw this frame.
    pub fn has_blur(&self) -> bool {
        self.blur_pipeline.is_some() && !self.blur_regions.is_empty()
    }

    /// The backdrop-`blur` sub-pass: for each `` `shader` *blur* `` element
    /// collected this frame, draws a rounded-rect-clipped gaussian of
    /// `src_bind_group` (a copy of the UI colour texture) back into the pass's
    /// colour target. Must run in its own `LoadOp::Load` pass on the UI layer,
    /// after the main UI pass, with no depth attachment.
    pub fn render_blur(
        &mut self,
        render_pass: &mut wgpu::RenderPass,
        src_bind_group: &wgpu::BindGroup,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let Some(pipeline) = &self.blur_pipeline else {
            return;
        };
        if self.blur_regions.is_empty() {
            return;
        }

        let mut verts: Vec<BlurVertex> = Vec::with_capacity(self.blur_regions.len() * 4);
        let mut idx: Vec<u32> = Vec::with_capacity(self.blur_regions.len() * 6);
        for r in &self.blur_regions {
            let m = 1.0;
            let (x0, y0, x1, y1) = (r.x - m, r.y - m, r.x + r.w + m, r.y + r.h + m);
            let center = [r.x + r.w * 0.5, r.y + r.h * 0.5];
            let half = [r.w * 0.5, r.h * 0.5];
            let params = [r.radius, 0.0, 0.0, 0.0];
            let base = verts.len() as u32;
            let v = |px: f32, py: f32| BlurVertex {
                position: [px, py],
                rect_center: center,
                rect_half: half,
                radii: r.radii,
                params,
                tint: r.tint,
            };
            verts.extend_from_slice(&[v(x0, y0), v(x0, y1), v(x1, y1), v(x1, y0)]);
            idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }

        let vslice = bytemuck::cast_slice(&verts);
        if vslice.len() > self.blur_vertex_buffer.size() as usize {
            self.blur_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ui_blur_vertices"),
                contents: vslice,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
        } else {
            queue.write_buffer(&self.blur_vertex_buffer, 0, vslice);
        }
        let islice = bytemuck::cast_slice(&idx);
        if islice.len() > self.blur_index_buffer.size() as usize {
            self.blur_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ui_blur_indices"),
                contents: islice,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            });
        } else {
            queue.write_buffer(&self.blur_index_buffer, 0, islice);
        }

        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, src_bind_group, &[]);
        render_pass.set_bind_group(1, &self.size_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.blur_vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.blur_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..idx.len() as u32, 0, 0..1);
    }

    pub fn resize(&mut self, size: (i32, i32), queue: &wgpu::Queue) {
        self.viewport_size = (size.0 as f32, size.1 as f32);

        queue.write_buffer(
            &self.size_buffer,
            0,
            bytemuck::cast_slice(&[SizeUniform {
                x: size.0 as f32,
                y: size.1 as f32,
            }]),
        );
    }

    pub fn begin(
        &mut self,
        render_pass: &mut wgpu::RenderPass,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        self.add_atlas(device, queue);
        self.drain_staged_shaders(device);
        self.vertices.clear();
        self.indices.clear();
        self.effect_vertices.clear();
        self.effect_indices.clear();
        self.blur_regions.clear();

        self.batches.clear();
        self.text_batches.clear();
        self.lines.clear();
        self.batch_index_begin = 0;
        self.batch_index_end = 0;
        self.effect_batch_index_begin = 0;
        self.effect_batch_index_end = 0;

        match self.render_pipeline.as_mut() {
            None => (),
            Some(render_pipeline) => {
                render_pass.set_pipeline(render_pipeline);
                match self.atlas_map.get(&self.active_atlas) {
                    None => {
                        render_pass.set_bind_group(
                            0,
                            self.atlas_map.get("default_atlas").unwrap(),
                            &[],
                        );
                    }
                    Some(atlas) => {
                        render_pass.set_bind_group(0, atlas, &[]);
                    }
                }
                render_pass.set_bind_group(1, &self.size_bind_group, &[]);
            }
        }
    }

    pub fn batch(&mut self) {
        if self.batch_index_end > self.batch_index_begin {
            self.batches.push(RenderBatch::Basic {
                begin: self.batch_index_begin,
                end: self.batch_index_end,
            });
            self.batch_index_begin = self.batch_index_end;
        }
    }

    pub fn begin_scissor(&mut self, position: UIPosition, mut size: UIPosition) {
        match self.scissor_active {
            true => {
                self.end_scissor();
            }
            false => {
                self.batch();
            }
        }

        let scissor_space = position + size;

        if scissor_space.x > self.viewport_size.0 {
            size.x += self.viewport_size.0 - scissor_space.x;
        }

        if scissor_space.y > self.viewport_size.1 {
            size.y += self.viewport_size.1 - scissor_space.y;
        }

        self.scissor_active = true;
        self.scissor_position = position;
        self.scissor_size = size;
    }

    pub fn end_scissor(&mut self) {
        match self.scissor_active {
            false => (),
            true => {
                self.scissor_active = false;
                if self.batch_index_end > self.batch_index_begin {
                    self.batches.push(RenderBatch::Scissor {
                        begin: self.batch_index_begin,
                        end: self.batch_index_end,
                        position: self.scissor_position,
                        size: self.scissor_size,
                    });
                    self.batch_index_begin = self.batch_index_end;
                }
            }
        }
    }

    /// Closes whatever geometry is pending (as a `Scissor` batch inside a
    /// clip region, else a `Basic` one) so the image about to be tessellated
    /// starts a fresh range, and records which atlas its texture batch will
    /// bind. Pair every call with [`Self::end_atlas`].
    pub fn bind_atlas(&mut self, atlas: &str) {
        if self.batch_index_end > self.batch_index_begin {
            if self.scissor_active {
                self.batches.push(RenderBatch::Scissor {
                    begin: self.batch_index_begin,
                    end: self.batch_index_end,
                    position: self.scissor_position,
                    size: self.scissor_size,
                });
                self.batch_index_begin = self.batch_index_end;
            } else {
                self.batch();
            }
        }

        self.active_atlas = atlas.to_string();
    }

    /// Flushes the geometry pushed since [`Self::bind_atlas`] as an `Atlas`
    /// batch - carrying the current clip rect, if any, so an image inside a
    /// `scroll` container is scissored like everything else in it.
    pub fn end_atlas(&mut self) {
        if self.batch_index_end > self.batch_index_begin {
            self.batches.push(RenderBatch::Atlas {
                begin: self.batch_index_begin,
                end: self.batch_index_end,
                atlas: self.active_atlas.clone(),
                scissor: if self.scissor_active {
                    Some((self.scissor_position, self.scissor_size))
                } else {
                    None
                },
            });
            self.batch_index_begin = self.batch_index_end;
        }
    }

    pub fn end(
        &mut self,
        render_pass: &mut wgpu::RenderPass,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _surface_config: &wgpu::SurfaceConfiguration,
    ) {
        match self.scissor_active {
            false => self.batch(),
            true => self.end_scissor(),
        }

        // Glyph quads join `vertices`/`indices` before the buffers are uploaded.
        self.emit_text(device, queue);

        match self.render_pipeline {
            None => (),
            Some(_) => {
                self.update_buffers(device, queue);

                let main_pipeline = self.render_pipeline.as_ref().unwrap();
                let bind_main = |render_pass: &mut wgpu::RenderPass| {
                    render_pass.set_pipeline(main_pipeline);
                    render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    render_pass
                        .set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                };
                bind_main(render_pass);
                // `true` while an effect pipeline + the effect buffers are bound;
                // the next `Basic`/`Scissor`/`Atlas` batch (or the text pass)
                // switches back to the main pipeline first.
                let mut on_effect = false;

                for render_batch in self.batches.iter() {
                    if on_effect && !matches!(render_batch, RenderBatch::Effect { .. }) {
                        bind_main(render_pass);
                        on_effect = false;
                    }
                    match render_batch {
                        RenderBatch::Basic { begin, end } => {
                            render_pass.draw_indexed(*begin..*end, 0, 0..1);
                        }
                        RenderBatch::Scissor {
                            begin,
                            end,
                            position,
                            size,
                        } => {
                            render_pass.set_scissor_rect(
                                position.x as u32,
                                position.y as u32,
                                size.x as u32,
                                size.y as u32,
                            );
                            render_pass.draw_indexed(*begin..*end, 0, 0..1);
                            render_pass.set_scissor_rect(
                                0,
                                0,
                                self.viewport_size.0 as u32,
                                self.viewport_size.1 as u32,
                            );
                        }
                        RenderBatch::Atlas {
                            begin,
                            end,
                            atlas,
                            scissor,
                        } => {
                            match self.atlas_map.get(atlas) {
                                None => continue,
                                Some(atlas) => {
                                    render_pass.set_bind_group(0, atlas, &[]);
                                    if let Some((position, size)) = scissor {
                                        render_pass.set_scissor_rect(
                                            position.x as u32,
                                            position.y as u32,
                                            size.x as u32,
                                            size.y as u32,
                                        );
                                        render_pass.draw_indexed(*begin..*end, 0, 0..1);
                                        render_pass.set_scissor_rect(
                                            0,
                                            0,
                                            self.viewport_size.0 as u32,
                                            self.viewport_size.1 as u32,
                                        );
                                    } else {
                                        render_pass.draw_indexed(*begin..*end, 0, 0..1);
                                    }
                                }
                            }
                        }
                        RenderBatch::Effect { begin, end, key } => {
                            let pipeline = match key {
                                EffectKey::BuiltIn => self.effect_pipeline.as_ref(),
                                EffectKey::Custom(sym) => self.effect_pipelines.get(sym),
                            };
                            let Some(pipeline) = pipeline else {
                                continue;
                            };
                            render_pass.set_pipeline(pipeline);
                            render_pass
                                .set_vertex_buffer(0, self.effect_vertex_buffer.slice(..));
                            render_pass.set_index_buffer(
                                self.effect_index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            render_pass.draw_indexed(*begin..*end, 0, 0..1);
                            on_effect = true;
                        }
                    }
                }

                if on_effect {
                    bind_main(render_pass);
                }

                // Text: bind the glyph atlas once, then draw each line's quad
                // range with its scissor. Emitted last but depth-tested, so a
                // later panel still occludes earlier text.
                if let Some(glyph_atlas) = &self.glyph_atlas
                    && !self.text_batches.is_empty()
                {
                    render_pass.set_bind_group(0, &glyph_atlas.bind_group, &[]);
                    let full = (self.viewport_size.0 as u32, self.viewport_size.1 as u32);
                    for tb in self.text_batches.iter() {
                        if let Some((position, size)) = tb.scissor {
                            let x = position.x.max(0.0) as u32;
                            let y = position.y.max(0.0) as u32;
                            let w = (size.x as u32).min(full.0.saturating_sub(x));
                            let h = (size.y as u32).min(full.1.saturating_sub(y));
                            if w == 0 || h == 0 {
                                continue;
                            }
                            render_pass.set_scissor_rect(x, y, w, h);
                            render_pass.draw_indexed(tb.begin..tb.end, 0, 0..1);
                            render_pass.set_scissor_rect(0, 0, full.0, full.1);
                        } else {
                            render_pass.draw_indexed(tb.begin..tb.end, 0, 0..1);
                        }
                    }
                }
            }
        }
    }

    pub fn render_layout<'render_pass>(
        &mut self,
        render_commands: Vec<
            RenderCommand<'render_pass, UIImageDescriptor, CustomElement, CustomLayoutSettings>,
        >,
        render_pass: &mut wgpu::RenderPass,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_config: &wgpu::SurfaceConfiguration,
    ) {
        let mut z: f32 = 0.1;

        self.begin(render_pass, device, queue);

        //println!("{:#?}", &render_commands);

        for command in render_commands {
            match command {
                RenderCommand::Rectangle(r) => {
                    let x = r.bounding_box.x * self.dpi_scale;
                    let y = r.bounding_box.y * self.dpi_scale;
                    let w = r.bounding_box.width * self.dpi_scale;
                    let h = r.bounding_box.height * self.dpi_scale;
                    let radii = [
                        r.corner_radii.top_left * self.dpi_scale,
                        r.corner_radii.top_right * self.dpi_scale,
                        r.corner_radii.bottom_left * self.dpi_scale,
                        r.corner_radii.bottom_right * self.dpi_scale,
                    ];
                    let color = UIColor::from_clay(r.color);
                    if w > 0.0 && h > 0.0 {
                        let fx = r.custom_layout_settings;
                        let ebox = EffectBox { x, y, w, h, z, radii, base: color };
                        self.emit_effects(ebox, fx, true);
                        if radii.iter().all(|&v| v < 0.05) {
                            self.push_quad(x, y, x + w, y + h, z, color);
                        } else {
                            let outline = Self::rounded_outline(x, y, w, h, radii);
                            self.push_fan(&outline, z, color);
                        }
                        self.emit_effects(ebox, fx, false);
                    }
                }
                RenderCommand::Border(b) => {
                    if b.width.top == b.width.bottom
                        && b.width.bottom == b.width.left
                        && b.width.left == b.width.right
                    {
                        // Equal-width border: a ring centred on the bbox edges
                        // (matches lyon's centred stroke). Direct geometry, no
                        // tessellator.
                        let bw = b.width.top as f32 * self.dpi_scale;
                        if bw > 0.0 {
                            let x = b.bounding_box.x * self.dpi_scale;
                            let y = b.bounding_box.y * self.dpi_scale;
                            let w = b.bounding_box.width * self.dpi_scale;
                            let h = b.bounding_box.height * self.dpi_scale;
                            let color = UIColor::from_clay(b.color);
                            let half = bw * 0.5;
                            let radii = [
                                b.corner_radii.top_left * self.dpi_scale,
                                b.corner_radii.top_right * self.dpi_scale,
                                b.corner_radii.bottom_left * self.dpi_scale,
                                b.corner_radii.bottom_right * self.dpi_scale,
                            ];
                            let fx = b.custom_layout_settings;
                            let ebox = EffectBox { x, y, w, h, z, radii, base: color };
                            self.emit_effects(ebox, fx, true);
                            if radii.iter().all(|&v| v < 0.05) {
                                // 4 edge strips.
                                self.push_quad(x - half, y - half, x + w + half, y + half, z, color);
                                self.push_quad(x - half, y + h - half, x + w + half, y + h + half, z, color);
                                self.push_quad(x - half, y + half, x + half, y + h - half, z, color);
                                self.push_quad(x + w - half, y + half, x + w + half, y + h - half, z, color);
                            } else {
                                let outer = Self::rounded_outline(
                                    x - half, y - half, w + bw, h + bw,
                                    radii.map(|r| r + half),
                                );
                                let inner = Self::rounded_outline(
                                    x + half, y + half, (w - bw).max(0.0), (h - bw).max(0.0),
                                    radii.map(|r| (r - half).max(0.0)),
                                );
                                self.push_ring(&outer, &inner, z, color);
                            }
                            self.emit_effects(ebox, fx, false);
                        }
                    } else {
                        // Per-side widths differ: draw each side as its own
                        // rect strip, centred on the edge (half in / half out)
                        // like the equal-width branch above. The old lyon path
                        // here closed every 2-point edge (`end(true)`), which
                        // tessellates to nothing - so single-side borders never
                        // drew. Corner radii aren't applied in the uneven case
                        // (per-side rounded corners aren't well defined); the
                        // equal-width branch handles the rounded case.
                        let x = b.bounding_box.x * self.dpi_scale;
                        let y = b.bounding_box.y * self.dpi_scale;
                        let w = b.bounding_box.width * self.dpi_scale;
                        let h = b.bounding_box.height * self.dpi_scale;
                        let color = UIColor::from_clay(b.color);
                        let wl = b.width.left as f32 * self.dpi_scale;
                        let wr = b.width.right as f32 * self.dpi_scale;
                        let wt = b.width.top as f32 * self.dpi_scale;
                        let wb = b.width.bottom as f32 * self.dpi_scale;

                        let fx = b.custom_layout_settings;
                        let ebox = EffectBox {
                            x: x - wl * 0.5,
                            y: y - wt * 0.5,
                            w: w + (wl + wr) * 0.5,
                            h: h + (wt + wb) * 0.5,
                            z,
                            radii: [0.0; 4],
                            base: color,
                        };
                        self.emit_effects(ebox, fx, true);
                        // Verticals run the full height (incl. the corners) so
                        // a left+top pair meets cleanly; horizontals then fill
                        // the top/bottom strips between them.
                        if wl > 0.0 {
                            self.push_quad(
                                x - wl * 0.5, y - wt * 0.5,
                                x + wl * 0.5, y + h + wb * 0.5,
                                z, color,
                            );
                        }
                        if wr > 0.0 {
                            self.push_quad(
                                x + w - wr * 0.5, y - wt * 0.5,
                                x + w + wr * 0.5, y + h + wb * 0.5,
                                z, color,
                            );
                        }
                        if wt > 0.0 {
                            self.push_quad(
                                x - wl * 0.5, y - wt * 0.5,
                                x + w + wr * 0.5, y + wt * 0.5,
                                z, color,
                            );
                        }
                        if wb > 0.0 {
                            self.push_quad(
                                x - wl * 0.5, y + h - wb * 0.5,
                                x + w + wr * 0.5, y + h + wb * 0.5,
                                z, color,
                            );
                        }
                        self.emit_effects(ebox, fx, false);
                    }
                }
                RenderCommand::Text(t) => self.draw_text(
                    t.text,
                    (t.font_size as f32) * self.dpi_scale,
                    match t.line_height {
                        0 => (t.font_size as f32) * 1.2 * self.dpi_scale,
                        _ => (t.line_height as f32) * self.dpi_scale,
                    },
                    t.font_id,
                    t.letter_spacing,
                    UIPosition {
                        x: t.bounding_box.x * self.dpi_scale,
                        y: t.bounding_box.y * self.dpi_scale,
                        z,
                    },
                    match self.scissor_active {
                        true => Some((self.scissor_position, self.scissor_size)),
                        false => None,
                    },
                    Color::rgb(t.color.r as u8, t.color.g as u8, t.color.b as u8),
                    z,
                ),
                RenderCommand::ScissorStart(b) => self.begin_scissor(
                    UIPosition::xy(b.x, b.y) * self.dpi_scale,
                    UIPosition::xy(b.width, b.height) * self.dpi_scale,
                ),
                RenderCommand::ScissorEnd => self.end_scissor(),
                RenderCommand::Image(image) => {
                    let uv = image.data;
                    let ipx = image.bounding_box.x * self.dpi_scale;
                    let ipy = image.bounding_box.y * self.dpi_scale;
                    let isx = image.bounding_box.width * self.dpi_scale;
                    let isy = image.bounding_box.height * self.dpi_scale;
                    let radii = if let Some(settings) = image.custom_layout_settings
                        && let CustomLayoutSettings::Radii {
                            top_left,
                            top_right,
                            bottom_left,
                            bottom_right,
                        } = settings
                    {
                        BorderRadii {
                            top_left: top_left * self.dpi_scale,
                            top_right: top_right * self.dpi_scale,
                            bottom_left: bottom_left * self.dpi_scale,
                            bottom_right: bottom_right * self.dpi_scale,
                        }
                    } else {
                        BorderRadii {
                            top_left: 0.0 * self.dpi_scale,
                            top_right: 0.0 * self.dpi_scale,
                            bottom_left: 0.0 * self.dpi_scale,
                            bottom_right: 0.0 * self.dpi_scale,
                        }
                    };

                    let er = [
                        radii.top_left,
                        radii.top_right,
                        radii.bottom_left,
                        radii.bottom_right,
                    ];
                    let ibase = UIColor::from_clay(image.background_color);
                    let ifx = image.custom_layout_settings;
                    let ebox = EffectBox {
                        x: ipx,
                        y: ipy,
                        w: isx,
                        h: isy,
                        z,
                        radii: er,
                        base: ibase,
                    };
                    self.emit_effects(ebox, ifx, true);

                    let mut builder = Path::builder();
                    builder.add_rounded_rectangle(
                        &Box2D::from_origin_and_size(Point2D::new(ipx, ipy), Size2D::new(isx, isy)),
                        &radii,
                        path::Winding::Negative,
                    );
                    let path = builder.build();

                    let mut geometry: VertexBuffers<UIVertex, u32> = VertexBuffers::new();
                    let mut tessellator = FillTessellator::new();
                    if tessellator
                        .tessellate_path(
                            &path,
                            &FillOptions::default()
                                .with_tolerance(0.1)
                                .with_fill_rule(lyon::tessellation::FillRule::EvenOdd),
                            &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| {
                                let x = vertex.position().x;
                                let y = vertex.position().y;
                                let r = (x - ipx) / isx;
                                let r = (r*(uv.u2-uv.u1))+uv.u1;
                                let g = (y - ipy) / isy;
                                let g = (g*(uv.v2-uv.v1))+uv.v1;
                                UIVertex {
                                    position: UIPosition { x, y, z },
                                    texture: 1,
                                    color: UIColor::WHITE,
                                    uv: [r, g],
                                }
                            }),
                        )
                        .is_ok()
                    {
                        self.bind_atlas(image.data.atlas);
                        let mut offset_indices = geometry
                            .indices
                            .iter()
                            .map(|index| index + self.vertices.len() as u32)
                            .collect::<Vec<u32>>();
                        self.vertices.append(&mut geometry.vertices);
                        self.indices.append(&mut offset_indices);
                        self.batch_index_end = self.indices.len() as u32;
                        self.end_atlas();
                    }
                    self.emit_effects(ebox, ifx, false);
                }
                RenderCommand::Custom(shape) => {
                    let dpi = self.dpi_scale;
                    let bb = shape.bounding_box;
                    // Maps a normalised (0..1, 0..1) point in the bounding box to
                    // a physical-pixel path coordinate.
                    let p = |fx: f32, fy: f32| {
                        Point2D::new((bb.x + fx * bb.width) * dpi, (bb.y + fy * bb.height) * dpi)
                    };
                    // Half the smaller box dimension, physical pixels - the unit
                    // `radius` fractions and the inscribed circle/ring use.
                    let unit_radius = bb.width.min(bb.height) / 2.0 * dpi;
                    let color = UIColor::from_clay(shape.background_color);

                    // Effects on a `circle`/`ring`: feed the SDF a circular
                    // radius so the rounded box degenerates to a disc. Stroked
                    // paths (`line`/`arc`/`bezier`) and `render-window` get none.
                    let (ex, ey, ew, eh) =
                        (bb.x * dpi, bb.y * dpi, bb.width * dpi, bb.height * dpi);
                    let er = ew.min(eh) * 0.5;
                    let efx = if matches!(
                        *shape.data,
                        CustomElement::Circle | CustomElement::Ring { .. }
                    ) {
                        shape.custom_layout_settings
                    } else {
                        None
                    };
                    let ebox = EffectBox {
                        x: ex,
                        y: ey,
                        w: ew,
                        h: eh,
                        z,
                        radii: [er; 4],
                        base: color,
                    };
                    self.emit_effects(ebox, efx, true);

                    match *shape.data {
                        CustomElement::Circle => {
                            let mut builder = Path::builder();
                            builder.add_circle(p(0.5, 0.5), unit_radius, path::Winding::Negative);
                            self.fill_path(&builder.build(), color, z);
                        }
                        CustomElement::Ring { thickness } => {
                            let mut builder = Path::builder();
                            builder.add_circle(p(0.5, 0.5), unit_radius, path::Winding::Negative);
                            self.stroke_path(&builder.build(), thickness * dpi, color, z);
                        }
                        CustomElement::Line {
                            from_x,
                            from_y,
                            to_x,
                            to_y,
                            thickness,
                        } => {
                            let mut builder = Path::builder();
                            builder.begin(p(from_x, from_y));
                            builder.line_to(p(to_x, to_y));
                            builder.end(false);
                            self.stroke_path(&builder.build(), thickness * dpi, color, z);
                        }
                        CustomElement::Arc {
                            center_x,
                            center_y,
                            radius,
                            start_angle,
                            end_angle,
                            thickness,
                        } => {
                            let r = radius * unit_radius;
                            let arc = Arc {
                                center: p(center_x, center_y),
                                radii: vector(r, r),
                                start_angle: Angle::degrees(start_angle),
                                sweep_angle: Angle::degrees(end_angle - start_angle),
                                x_rotation: Angle::zero(),
                            };
                            let mut builder = Path::builder();
                            builder.begin(arc.from());
                            arc.for_each_quadratic_bezier(&mut |curve| {
                                builder.quadratic_bezier_to(curve.ctrl, curve.to);
                            });
                            builder.end(false);
                            self.stroke_path(&builder.build(), thickness * dpi, color, z);
                        }
                        CustomElement::Bezier {
                            from_x,
                            from_y,
                            ctrl1_x,
                            ctrl1_y,
                            ctrl2_x,
                            ctrl2_y,
                            to_x,
                            to_y,
                            thickness,
                        } => {
                            let mut builder = Path::builder();
                            builder.begin(p(from_x, from_y));
                            builder.cubic_bezier_to(
                                p(ctrl1_x, ctrl1_y),
                                p(ctrl2_x, ctrl2_y),
                                p(to_x, to_y),
                            );
                            builder.end(false);
                            self.stroke_path(&builder.build(), thickness * dpi, color, z);
                        }
                        // Draws nothing into the UI layer: the rect is left
                        // transparent so the 3D scene composited underneath
                        // shows through. The scene renderer picks it up from
                        // `collect_render_windows`.
                        CustomElement::RenderWindow { .. } => {}
                    }
                    self.emit_effects(ebox, efx, false);
                }
                RenderCommand::None => {}
            }
            // Depth decreases per command so later draws land on top. Past ~1000
            // commands it would go negative and the primitive would be
            // depth-clipped, so pin it at the near plane - those elements are
            // drawn last anyway, so painter's order keeps them correct.
            z = (z - 0.0001).max(0.0);
        }

        self.end(render_pass, device, queue, surface_config);
    }

    /// Walks the text runs collected this frame, rasterises any missing glyphs
    /// into [`GlyphAtlas`], and appends a textured quad per glyph to
    /// `vertices`/`indices` (recording the index ranges in `text_batches`).
    /// Must run before `update_buffers` in `end`.
    fn emit_text(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.lines.is_empty() || self.glyph_atlas.is_none() {
            return;
        }

        // Pass 1: pen positions + colours, borrowing the shaped buffers.
        struct Placement {
            key: CacheKey,
            pen_x: i32,
            pen_y: i32,
            color: UIColor,
            z: f32,
            line: usize,
        }
        let mut placements: Vec<Placement> = Vec::new();
        let mut line_scissor: Vec<Option<(UIPosition, UIPosition)>> = Vec::new();

        for (li, text_line) in self.lines.iter().enumerate() {
            line_scissor.push(text_line.bounds);
            let Some(cached) = self.line_cache.get(&text_line.key) else {
                continue;
            };
            let left = text_line.left.round() as i32;
            let top = text_line.top.round() as i32;
            let default_color = UIColor {
                r: text_line.color.r() as f32 / 255.0,
                g: text_line.color.g() as f32 / 255.0,
                b: text_line.color.b() as f32 / 255.0,
                a: text_line.color.a() as f32 / 255.0,
            };
            for run in cached.buffer.layout_runs() {
                for glyph in run.glyphs.iter() {
                    let physical = glyph.physical((0.0, run.line_y), 1.0);
                    let color = match glyph.color_opt {
                        Some(c) => UIColor {
                            r: c.r() as f32 / 255.0,
                            g: c.g() as f32 / 255.0,
                            b: c.b() as f32 / 255.0,
                            a: c.a() as f32 / 255.0,
                        },
                        None => default_color,
                    };
                    placements.push(Placement {
                        key: physical.cache_key,
                        pen_x: left + physical.x,
                        pen_y: top + physical.y,
                        color,
                        z: text_line.z,
                        line: li,
                    });
                }
            }
        }

        // Pass 2: resolve atlas slots and emit quads, one batch per line.
        let atlas_size = self.glyph_atlas.as_ref().unwrap().size as f32;
        let mut batch_line: Option<usize> = None;
        let mut batch_begin = self.indices.len() as u32;

        let flush = |batches: &mut Vec<TextBatch>,
                     line: usize,
                     begin: u32,
                     end: u32,
                     scissor: &[Option<(UIPosition, UIPosition)>]| {
            if end > begin {
                batches.push(TextBatch {
                    begin,
                    end,
                    scissor: scissor[line],
                });
            }
        };

        for p in placements {
            if batch_line != Some(p.line) {
                if let Some(prev) = batch_line {
                    flush(
                        &mut self.text_batches,
                        prev,
                        batch_begin,
                        self.indices.len() as u32,
                        &line_scissor,
                    );
                }
                batch_line = Some(p.line);
                batch_begin = self.indices.len() as u32;
            }

            let Some(slot) = self.atlas_glyph(device, queue, p.key) else {
                continue;
            };
            if slot.w == 0 || slot.h == 0 {
                continue;
            }

            let x0 = (p.pen_x + slot.left) as f32;
            let y0 = (p.pen_y - slot.top) as f32;
            let x1 = x0 + slot.w as f32;
            let y1 = y0 + slot.h as f32;
            let u0 = slot.x as f32 / atlas_size;
            let v0 = slot.y as f32 / atlas_size;
            let u1 = (slot.x + slot.w) as f32 / atlas_size;
            let v1 = (slot.y + slot.h) as f32 / atlas_size;
            let (tag, color): (u32, UIColor) = if slot.color {
                (3, UIColor::WHITE)
            } else {
                (2, p.color)
            };

            let base = self.vertices.len() as u32;
            self.vertices.extend_from_slice(&[
                UIVertex { position: UIPosition { x: x0, y: y0, z: p.z }, texture: tag, color, uv: [u0, v0] },
                UIVertex { position: UIPosition { x: x1, y: y0, z: p.z }, texture: tag, color, uv: [u1, v0] },
                UIVertex { position: UIPosition { x: x1, y: y1, z: p.z }, texture: tag, color, uv: [u1, v1] },
                UIVertex { position: UIPosition { x: x0, y: y1, z: p.z }, texture: tag, color, uv: [u0, v1] },
            ]);
            // Winding to match lyon's output through the Y-flipping vertex
            // shader (`FrontFace::Ccw` + back-face culling).
            self.indices.extend_from_slice(&[
                base, base + 2, base + 1, base, base + 3, base + 2,
            ]);
        }
        if let Some(prev) = batch_line {
            flush(
                &mut self.text_batches,
                prev,
                batch_begin,
                self.indices.len() as u32,
                &line_scissor,
            );
        }
    }

    /// Returns the atlas slot for `key`, rasterising the glyph and packing it on
    /// a cache miss. `None` if the glyph can't be rendered (rare) or the atlas
    /// is exhausted even after a reset.
    fn atlas_glyph(
        &mut self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: CacheKey,
    ) -> Option<GlyphSlot> {
        let generation = self.text_generation;
        if let Some(atlas) = self.glyph_atlas.as_mut()
            && let Some(slot) = atlas.map.get_mut(&key)
        {
            slot.generation = generation;
            return Some(*slot);
        }

        let image = self
            .swash_cache
            .get_image_uncached(&mut self.font_system, key)?;
        let w = image.placement.width;
        let h = image.placement.height;
        let color = matches!(image.content, SwashContent::Color);

        let atlas = self.glyph_atlas.as_mut()?;

        if w == 0 || h == 0 {
            let slot = GlyphSlot {
                alloc: None,
                x: 0,
                y: 0,
                w: 0,
                h: 0,
                left: image.placement.left,
                top: image.placement.top,
                color,
                generation,
            };
            atlas.map.insert(key, slot);
            return Some(slot);
        }

        // 1px gutter so nearest-neighbour sampling never reads a neighbour.
        let alloc = match atlas.allocator.allocate(size2(w as i32 + 1, h as i32 + 1)) {
            Some(a) => a,
            None => {
                // Full: drop everything and start over (glyphs re-rasterise as
                // they're next seen).
                atlas.allocator = AtlasAllocator::new(size2(atlas.size as i32, atlas.size as i32));
                atlas.map.clear();
                atlas
                    .allocator
                    .allocate(size2(w as i32 + 1, h as i32 + 1))?
            }
        };
        let ox = alloc.rectangle.min.x as u32;
        let oy = alloc.rectangle.min.y as u32;

        let rgba: Vec<u8> = if color {
            image.data
        } else {
            let mut buf = vec![0u8; (w * h * 4) as usize];
            for (i, &coverage) in image.data.iter().enumerate() {
                buf[i * 4] = coverage;
            }
            buf
        };

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: ox, y: oy, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );

        let slot = GlyphSlot {
            alloc: Some(alloc.id),
            x: ox,
            y: oy,
            w,
            h,
            left: image.placement.left,
            top: image.placement.top,
            color,
            generation,
        };
        atlas.map.insert(key, slot);
        Some(slot)
    }

    pub fn draw_text(
        &mut self,
        text: &str,
        font_size: f32,
        line_height: f32,
        font_id: u16,
        letter_spacing: u16,
        position: UIPosition,
        bounds: Option<(UIPosition, UIPosition)>,
        color: cosmic_text::Color,
        draw_order: f32,
    ) {
        let key = Self::text_key(text, font_id, letter_spacing, font_size, line_height);
        let generation = self.text_generation;

        match self.line_cache.get_mut(&key) {
            Some(cached) => cached.generation = generation,
            None => {
                let mut buffer =
                    Buffer::new(&mut self.font_system, Metrics::new(font_size, line_height));
                {
                    let attrs =
                        Self::text_attrs(&self.font_families, font_id, letter_spacing, font_size);
                    buffer.set_text(text, &attrs, Self::shaping_for(text), None);
                }
                buffer.shape_until_scroll(&mut self.font_system, false);
                self.line_cache.insert(key, CachedLine { buffer, generation });
            }
        }

        self.lines.push(TextLine {
            key,
            left: position.x,
            top: position.y,
            color,
            bounds,
            z: draw_order,
        });
    }

    /// Queues `atlas_data` to become the atlas `name` on the next frame. If an
    /// atlas of that name is already pending upload this frame, its pixels are
    /// replaced rather than a second copy queued; an already-uploaded atlas of
    /// the same name is overwritten when the queue is drained (`add_atlas`).
    pub fn stage_atlas(&mut self, name: String, atlas_data: DynamicImage) {
        if let Some((_, pending)) = self.staged_images.iter_mut().find(|(n, _)| *n == name) {
            *pending = atlas_data;
        } else {
            self.staged_images.push((name, atlas_data));
        }
    }

    fn add_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        for (name, staged_image) in self.staged_images.drain(..) {
            let new_atlas = wgpu::BindGroup::create_atlas(staged_image, device, queue);
            self.atlas_map.insert(name.clone(), new_atlas);
            self.active_atlas = name;
        }
    }

    /// Fills `path` with a solid `color` at depth `z` and appends the result to
    /// the current geometry buffers.
    fn fill_path(&mut self, path: &Path, color: UIColor, z: f32) {
        let mut geometry: VertexBuffers<UIVertex, u32> = VertexBuffers::new();
        let mut tessellator = FillTessellator::new();
        if tessellator
            .tessellate_path(
                path,
                &FillOptions::default()
                    .with_tolerance(0.1)
                    .with_fill_rule(lyon::tessellation::FillRule::EvenOdd),
                &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| UIVertex {
                    position: UIPosition {
                        x: vertex.position().x,
                        y: vertex.position().y,
                        z,
                    },
                    texture: 0,
                    color,
                    uv: [0.0, 0.0],
                }),
            )
            .is_ok()
        {
            self.append_geometry(geometry);
        }
    }

    /// Strokes `path` with a `width`-pixel line in `color` at depth `z` and
    /// appends the result to the current geometry buffers.
    fn stroke_path(&mut self, path: &Path, width: f32, color: UIColor, z: f32) {
        let mut geometry: VertexBuffers<UIVertex, u32> = VertexBuffers::new();
        let mut tessellator = StrokeTessellator::new();
        if tessellator
            .tessellate_path(
                path,
                &StrokeOptions::default().with_line_width(width),
                &mut BuffersBuilder::new(&mut geometry, |vertex: StrokeVertex| UIVertex {
                    position: UIPosition {
                        x: vertex.position().x,
                        y: vertex.position().y,
                        z,
                    },
                    texture: 0,
                    color,
                    uv: [0.0, 0.0],
                }),
            )
            .is_ok()
        {
            self.append_geometry(geometry);
        }
    }

    /// Appends a tessellated `geometry` buffer to `self.vertices` / `self.indices`,
    /// rebasing its indices onto the current vertex count and advancing the batch.
    fn append_geometry(&mut self, mut geometry: VertexBuffers<UIVertex, u32>) {
        let mut offset_indices = geometry
            .indices
            .iter()
            .map(|index| index + self.vertices.len() as u32)
            .collect::<Vec<u32>>();
        self.vertices.append(&mut geometry.vertices);
        self.indices.append(&mut offset_indices);
        self.batch_index_end = self.indices.len() as u32;
    }

    /// A solid axis-aligned quad `[x0,y0]..[x1,y1]` at depth `z` - 4 vertices +
    /// 2 triangles straight into the batch, no tessellator. Winding matches the
    /// glyph quads (see `emit_text`): CCW-in-screen perimeter, fanned so the
    /// `FrontFace::Ccw` + back-face-culling pipeline keeps it.
    fn push_quad(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, z: f32, color: UIColor) {
        let base = self.vertices.len() as u32;
        let v = |x: f32, y: f32| UIVertex {
            position: UIPosition { x, y, z },
            texture: 0,
            color,
            uv: [0.0, 0.0],
        };
        // perimeter CCW in screen space: TL, BL, BR, TR
        self.vertices
            .extend_from_slice(&[v(x0, y0), v(x0, y1), v(x1, y1), v(x1, y0)]);
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        self.batch_index_end = self.indices.len() as u32;
    }

    /// Emits the stacked `` `shader` `` effects on one element that belong on the
    /// given side of its fill - `behind == true` for cast shadows / backdrop
    /// blur (call before drawing the fill), `false` for bevel / glow / custom
    /// shaders (call after). A no-op when the element has no effects.
    fn emit_effects(
        &mut self,
        rect: EffectBox,
        settings: Option<&CustomLayoutSettings>,
        behind: bool,
    ) {
        let Some(CustomLayoutSettings::Effects(list)) = settings else {
            return;
        };
        for rs in list {
            if rs.kind.behind() == behind {
                self.emit_effect(rect, rs);
            }
        }
    }

    /// Routes a resolved `` `shader` `` effect for element box `rect`: a `blur`
    /// records a [`BlurRegion`] for the post-pass, everything else emits an
    /// effect quad now.
    fn emit_effect(&mut self, rect: EffectBox, rs: &ResolvedShader) {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        if rs.kind == EffectKind::Blur {
            self.blur_regions.push(BlurRegion {
                x: rect.x,
                y: rect.y,
                w: rect.w,
                h: rect.h,
                radii: rect.radii,
                radius: rs.params[0].max(0.0) * self.dpi_scale,
                tint: rs.params2,
            });
        } else {
            self.push_effect_quad(rect, rs);
        }
    }

    /// Emits one effect quad for element box `rect`. The quad is expanded past
    /// the box by whatever margin the effect needs to paint (a drop shadow
    /// reaches out; a bevel/glow stays inside). Flushes any pending `Basic`
    /// range first, then records a `RenderBatch::Effect`.
    fn push_effect_quad(&mut self, rect: EffectBox, rs: &ResolvedShader) {
        let EffectBox { x, y, w, h, z, radii, base } = rect;
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let dpi = self.dpi_scale;
        let p = rs.params;
        // Built-in effects author their distances in logical px; scale the
        // spatial ones to physical px (the space the shader works in). Custom
        // shader params pass through verbatim.
        let params = match rs.kind {
            EffectKind::DropShadow => [p[0] * dpi, p[1] * dpi, p[2] * dpi, p[3] * dpi],
            EffectKind::RaisedEdge => [p[0] * dpi, p[1], p[2], p[3]],
            EffectKind::InnerGlow => [p[0], p[1], p[2] * dpi, p[3] * dpi],
            EffectKind::Blur | EffectKind::Custom => p,
        };
        let margin = match rs.kind {
            EffectKind::DropShadow => {
                params[0].abs() + params[1].abs() + params[2] + params[3] + 2.0
            }
            EffectKind::RaisedEdge | EffectKind::InnerGlow => 1.0,
            EffectKind::Blur => 0.0,
            // A custom shader can spill well past the box (a neon glow, say);
            // give it a wide margin so it is not clipped by its own quad.
            EffectKind::Custom => {
                (p[0].abs() + p[1].abs() + p[2].abs() + p[3].abs()) * dpi * 4.0 + 24.0
            }
        };
        let (x0, y0, x1, y1) = (x - margin, y - margin, x + w + margin, y + h + margin);
        let center = [x + w * 0.5, y + h * 0.5];
        let half = [w * 0.5, h * 0.5];
        let effect = rs.kind as u32;
        let vbase = self.effect_vertices.len() as u32;
        // Vertex `color` is the element's own fill (what `raised-edge` and
        // custom shaders read as `in.color`); effect-specific colours ride in
        // `params2`.
        let v = |px: f32, py: f32, u: f32, vv: f32| EffectVertex {
            position: UIPosition { x: px, y: py, z },
            effect,
            color: base,
            uv: [u, vv],
            rect_center: center,
            rect_half: half,
            radii,
            params,
            params2: rs.params2,
        };
        self.effect_vertices.extend_from_slice(&[
            v(x0, y0, 0.0, 0.0),
            v(x0, y1, 0.0, 1.0),
            v(x1, y1, 1.0, 1.0),
            v(x1, y0, 1.0, 0.0),
        ]);
        self.effect_indices.extend_from_slice(&[
            vbase,
            vbase + 1,
            vbase + 2,
            vbase,
            vbase + 2,
            vbase + 3,
        ]);
        self.effect_batch_index_end = self.effect_indices.len() as u32;

        let key = match (rs.kind, rs.custom) {
            (EffectKind::Custom, Some(sym)) => EffectKey::Custom(sym),
            _ => EffectKey::BuiltIn,
        };
        self.batch_effect(key);
    }

    /// Closes the current effect run as a `RenderBatch::Effect`, flushing the
    /// pending main `Basic` range first so draw order (and depth) is preserved.
    fn batch_effect(&mut self, key: EffectKey) {
        self.batch();
        if self.effect_batch_index_end > self.effect_batch_index_begin {
            self.batches.push(RenderBatch::Effect {
                begin: self.effect_batch_index_begin,
                end: self.effect_batch_index_end,
                key,
            });
            self.effect_batch_index_begin = self.effect_batch_index_end;
        }
    }

    /// A convex polygon (given as a CCW-in-screen perimeter) filled by a triangle
    /// fan from its centroid, at depth `z`.
    fn push_fan(&mut self, perimeter: &[[f32; 2]], z: f32, color: UIColor) {
        let n = perimeter.len();
        if n < 3 {
            return;
        }
        let (mut cx, mut cy) = (0.0f32, 0.0f32);
        for p in perimeter {
            cx += p[0];
            cy += p[1];
        }
        cx /= n as f32;
        cy /= n as f32;

        let base = self.vertices.len() as u32;
        let v = |x: f32, y: f32| UIVertex {
            position: UIPosition { x, y, z },
            texture: 0,
            color,
            uv: [0.0, 0.0],
        };
        self.vertices.push(v(cx, cy));
        self.vertices
            .extend(perimeter.iter().map(|p| v(p[0], p[1])));
        for i in 0..n as u32 {
            let next = if i + 1 == n as u32 { 0 } else { i + 1 };
            self.indices
                .extend_from_slice(&[base, base + 1 + i, base + 1 + next]);
        }
        self.batch_index_end = self.indices.len() as u32;
    }

    /// A closed strip between an `outer` and `inner` perimeter (same length,
    /// same winding) - the fill for a border ring.
    fn push_ring(&mut self, outer: &[[f32; 2]], inner: &[[f32; 2]], z: f32, color: UIColor) {
        let n = outer.len();
        if n < 3 || inner.len() != n {
            return;
        }
        let base = self.vertices.len() as u32;
        let v = |x: f32, y: f32| UIVertex {
            position: UIPosition { x, y, z },
            texture: 0,
            color,
            uv: [0.0, 0.0],
        };
        for i in 0..n {
            self.vertices.push(v(outer[i][0], outer[i][1]));
            self.vertices.push(v(inner[i][0], inner[i][1]));
        }
        for i in 0..n as u32 {
            let ni = if i + 1 == n as u32 { 0 } else { i + 1 };
            let (o0, i0, o1, i1) = (base + 2 * i, base + 2 * i + 1, base + 2 * ni, base + 2 * ni + 1);
            // Same winding as `push_quad` / `push_fan` (see `emit_text`).
            self.indices
                .extend_from_slice(&[o0, o1, i0, o1, i1, i0]);
        }
        self.batch_index_end = self.indices.len() as u32;
    }

    /// The perimeter of a rounded rectangle, CCW in screen space (down the left
    /// edge first), as exactly `4·(ARC_STEPS+1)` points - a fixed count so an
    /// outer/inner pair lines up for [`push_ring`](Self::push_ring). Each radius
    /// is clamped to `min(w,h)/2` and floored at `0`. `radii` is `[top_left,
    /// top_right, bottom_left, bottom_right]` (clay's `CornerRadii` order).
    fn rounded_outline(x: f32, y: f32, w: f32, h: f32, radii: [f32; 4]) -> Vec<[f32; 2]> {
        let maxr = (w.min(h) * 0.5).max(0.0);
        let tl = radii[0].clamp(0.0, maxr);
        let tr = radii[1].clamp(0.0, maxr);
        let bl = radii[2].clamp(0.0, maxr);
        let br = radii[3].clamp(0.0, maxr);
        let arc = unit_arc();

        let mut pts: Vec<[f32; 2]> = Vec::with_capacity(4 * (ARC_STEPS + 1));
        // corner: centre `(cx,cy)`, radius `r`, start/end unit vectors `s`/`e`
        // (perpendicular); the arc lerps `s*cos t + e*sin t`. `r == 0` gives the
        // sharp corner repeated `ARC_STEPS+1` times.
        let mut corner = |cx: f32, cy: f32, r: f32, s: [f32; 2], e: [f32; 2]| {
            for a in arc.iter() {
                let (c, sn) = (a[0], a[1]);
                let dx = s[0] * c + e[0] * sn;
                let dy = s[1] * c + e[1] * sn;
                pts.push([cx + r * dx, cy + r * dy]);
            }
        };
        corner(x + bl, y + h - bl, bl, [-1.0, 0.0], [0.0, 1.0]); // left edge -> bottom-left
        corner(x + w - br, y + h - br, br, [0.0, 1.0], [1.0, 0.0]); // bottom -> bottom-right
        corner(x + w - tr, y + tr, tr, [1.0, 0.0], [0.0, -1.0]); // right -> top-right
        corner(x + tl, y + tl, tl, [0.0, -1.0], [-1.0, 0.0]); // top -> top-left
        pts
    }
}

/// Number of segments per 90° corner arc for rounded rectangles / borders.
const ARC_STEPS: usize = 8;

/// Unit quarter-circle sample vectors `(cos t, sin t)` for `t` in `[0, π/2]`,
/// `ARC_STEPS + 1` of them. Built once; used to lerp rounded-rect corners.
fn unit_arc() -> &'static [[f32; 2]; ARC_STEPS + 1] {
    static ARC: std::sync::OnceLock<[[f32; 2]; ARC_STEPS + 1]> = std::sync::OnceLock::new();
    ARC.get_or_init(|| {
        let mut a = [[0.0f32; 2]; ARC_STEPS + 1];
        for (i, p) in a.iter_mut().enumerate() {
            let t = (i as f32 / ARC_STEPS as f32) * core::f32::consts::FRAC_PI_2;
            *p = [t.cos(), t.sin()];
        }
        a
    })
}

pub struct UIPipeline {
    pixel_format: wgpu::TextureFormat,
    vertex_buffer_layouts: Vec<wgpu::VertexBufferLayout<'static>>,
}

impl UIPipeline {
    pub fn new(pixel_format: wgpu::TextureFormat) -> Self {
        Self {
            pixel_format,
            vertex_buffer_layouts: Vec::new(),
        }
    }

    pub fn add_buffer_layout(&mut self, layout: wgpu::VertexBufferLayout<'static>) {
        self.vertex_buffer_layouts.push(layout);
    }

    pub fn build_pipeline(
        &self,
        device: &wgpu::Device,
        size_bind_group_layout: &wgpu::BindGroupLayout,
        multisample: wgpu::MultisampleState,
    ) -> wgpu::RenderPipeline {
        let source_code = include_str!("ui_shader.wgsl");

        let shader_module_desc = wgpu::ShaderModuleDescriptor {
            label: Some("UI Shader Module"),
            source: wgpu::ShaderSource::Wgsl(source_code.into()),
        };
        let shader_module = device.create_shader_module(shader_module_desc);

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
                label: Some("texture_bind_group_layout"),
            });

        let piplaydesc = wgpu::PipelineLayoutDescriptor {
            label: Some("UI Render Pipeline Layout"),
            bind_group_layouts: &[Some(&texture_bind_group_layout), Some(size_bind_group_layout)],
            immediate_size: 0,
        };
        let pipeline_layout = device.create_pipeline_layout(&piplaydesc);

        let render_targets = [Some(wgpu::ColorTargetState {
            format: self.pixel_format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let vertex_buffers: Vec<Option<wgpu::VertexBufferLayout>> =
            self.vertex_buffer_layouts.iter().cloned().map(Some).collect();

        let render_pip_desc = wgpu::RenderPipelineDescriptor {
            label: Some("UI Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader_module,
                entry_point: Some("vs_main"),
                buffers: &vertex_buffers,
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader_module,
                entry_point: Some("fs_main"),
                targets: &render_targets,
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                // Command `z` decreases monotonically, so for the rect/border/
                // image stream this behaves like `Always`; text glyph quads
                // (emitted last, but carrying the `z` of their originating
                // command) rely on the test to sit correctly under later panels.
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample,
            multiview_mask: None,
            cache: None,
        };

        device.create_render_pipeline(&render_pip_desc)
    }
}

/// Builds the fullscreen-triangle pipeline that composites a cached UI texture
/// (see `ui_composite.wgsl`) over the 3D scene. Alpha-blended, no vertex
/// buffers, no depth attachment.
fn build_composite_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("UI Composite Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("ui_composite.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("UI Composite Pipeline Layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("UI Composite Pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// The shared contract prepended to every effect shader body (built-in and
/// custom), so a `.wgsl` file only has to provide `fs_main`. See
/// `effect_prelude.wgsl`.
pub const EFFECT_PRELUDE: &str = include_str!("effect_prelude.wgsl");

/// Concatenates [`EFFECT_PRELUDE`] in front of an effect shader body.
pub fn effect_source(body: &str) -> String {
    format!("{EFFECT_PRELUDE}\n{body}")
}

/// The bind-group layout the effect pipelines share with the main UI pipeline
/// (group 0 = atlas texture + sampler). Built identically to
/// `UIPipeline::build_pipeline`'s so an atlas bind group set for the main
/// pipeline stays valid when an effect pipeline is bound mid-pass.
fn effect_texture_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
        label: Some("texture_bind_group_layout"),
    })
}

/// Builds one effect pipeline (built-in or custom) from an already-concatenated
/// WGSL source (`effect_source(body)`). Same depth/blend/primitive/MSAA state
/// as the main UI pipeline so effect batches interleave with `Basic` batches
/// under one depth buffer.
fn build_effect_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    msaa_count: u32,
    size_bind_group_layout: &wgpu::BindGroupLayout,
    wgsl: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    });
    let texture_bind_group_layout = effect_texture_bind_group_layout(device);
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("UI Effect Pipeline Layout"),
        bind_group_layouts: &[Some(&texture_bind_group_layout), Some(size_bind_group_layout)],
        immediate_size: 0,
    });
    let vertex_layout = EffectVertex::get_layout();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[Some(vertex_layout)],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: msaa_count,
            mask: 1,
            alpha_to_coverage_enabled: false,
        },
        multiview_mask: None,
        cache: None,
    })
}

/// The backdrop-`blur` sub-pass pipeline (`blur.wgsl`). Runs into the UI layer's
/// colour texture with `LoadOp::Load` and no depth attachment; group 0 is the
/// blur source (a copy of that texture), group 1 the viewport-size uniform.
fn build_blur_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size_bind_group_layout: &wgpu::BindGroupLayout,
    src_bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("UI Blur Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("blur.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("UI Blur Pipeline Layout"),
        bind_group_layouts: &[Some(src_bind_group_layout), Some(size_bind_group_layout)],
        immediate_size: 0,
    });
    let vbl = BlurVertex::get_layout();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("UI Blur Pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[Some(vbl)],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// A 64-bit hash of everything that affects the pixels `render_layout` would
/// draw for `commands`. When two consecutive frames hash to the same value the
/// UI render pass is skipped and the previous frame's texture is re-composited.
pub fn commands_fingerprint(
    commands: &[RenderCommand<'_, UIImageDescriptor, CustomElement, CustomLayoutSettings>],
    dpi_scale: f32,
    viewport_size: (f32, f32),
) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut h = DefaultHasher::new();

    fn f(h: &mut DefaultHasher, x: f32) {
        x.to_bits().hash(h);
    }
    fn bbox(h: &mut DefaultHasher, b: &telera_layout::BoundingBox) {
        f(h, b.x);
        f(h, b.y);
        f(h, b.width);
        f(h, b.height);
    }
    fn color(h: &mut DefaultHasher, c: &telera_layout::Color) {
        f(h, c.r);
        f(h, c.g);
        f(h, c.b);
        f(h, c.a);
    }
    fn radii(h: &mut DefaultHasher, r: &telera_layout::CornerRadii) {
        f(h, r.top_left);
        f(h, r.top_right);
        f(h, r.bottom_left);
        f(h, r.bottom_right);
    }
    fn effect(h: &mut DefaultHasher, s: Option<&CustomLayoutSettings>) {
        if let Some(CustomLayoutSettings::Effects(list)) = s {
            9u8.hash(h);
            list.len().hash(h);
            for rs in list {
                (rs.kind as u32).hash(h);
                rs.custom.hash(h);
                for v in rs.params.iter().chain(rs.params2.iter()) {
                    f(h, *v);
                }
            }
        }
    }

    f(&mut h, dpi_scale);
    f(&mut h, viewport_size.0);
    f(&mut h, viewport_size.1);

    for command in commands {
        match command {
            RenderCommand::None => 0u8.hash(&mut h),
            RenderCommand::Rectangle(r) => {
                1u8.hash(&mut h);
                r.id.hash(&mut h);
                r.z_index.hash(&mut h);
                bbox(&mut h, &r.bounding_box);
                color(&mut h, &r.color);
                radii(&mut h, &r.corner_radii);
                effect(&mut h, r.custom_layout_settings);
            }
            RenderCommand::Border(b) => {
                2u8.hash(&mut h);
                b.id.hash(&mut h);
                b.z_index.hash(&mut h);
                bbox(&mut h, &b.bounding_box);
                color(&mut h, &b.color);
                radii(&mut h, &b.corner_radii);
                b.width.left.hash(&mut h);
                b.width.right.hash(&mut h);
                b.width.top.hash(&mut h);
                b.width.bottom.hash(&mut h);
                b.width.between_children.hash(&mut h);
                effect(&mut h, b.custom_layout_settings);
            }
            RenderCommand::Text(t) => {
                3u8.hash(&mut h);
                t.id.hash(&mut h);
                t.z_index.hash(&mut h);
                t.text.as_bytes().hash(&mut h);
                bbox(&mut h, &t.bounding_box);
                color(&mut h, &t.color);
                t.font_id.hash(&mut h);
                t.font_size.hash(&mut h);
                t.letter_spacing.hash(&mut h);
                t.line_height.hash(&mut h);
            }
            RenderCommand::Image(i) => {
                4u8.hash(&mut h);
                i.id.hash(&mut h);
                i.z_index.hash(&mut h);
                bbox(&mut h, &i.bounding_box);
                color(&mut h, &i.background_color);
                i.data.atlas.as_bytes().hash(&mut h);
                f(&mut h, i.data.u1);
                f(&mut h, i.data.v1);
                f(&mut h, i.data.u2);
                f(&mut h, i.data.v2);
                effect(&mut h, i.custom_layout_settings);
            }
            RenderCommand::Custom(c) => {
                5u8.hash(&mut h);
                c.id.hash(&mut h);
                c.z_index.hash(&mut h);
                bbox(&mut h, &c.bounding_box);
                color(&mut h, &c.background_color);
                radii(&mut h, &c.corner_radii);
                match *c.data {
                    CustomElement::Circle => 0u8.hash(&mut h),
                    CustomElement::RenderWindow {
                        camera,
                        eye_x,
                        eye_y,
                        eye_z,
                        target_x,
                        target_y,
                        target_z,
                        up_x,
                        up_y,
                        up_z,
                        fov,
                        near,
                        far,
                        ortho_height,
                    } => {
                        1u8.hash(&mut h);
                        camera.hash(&mut h);
                        for v in [
                            eye_x, eye_y, eye_z, target_x, target_y, target_z, up_x, up_y, up_z,
                            fov, near, far, ortho_height,
                        ] {
                            v.map(f32::to_bits).hash(&mut h);
                        }
                    }
                    CustomElement::Line {
                        from_x,
                        from_y,
                        to_x,
                        to_y,
                        thickness,
                    } => {
                        2u8.hash(&mut h);
                        for v in [from_x, from_y, to_x, to_y, thickness] {
                            f(&mut h, v);
                        }
                    }
                    CustomElement::Ring { thickness } => {
                        3u8.hash(&mut h);
                        f(&mut h, thickness);
                    }
                    CustomElement::Arc {
                        center_x,
                        center_y,
                        radius,
                        start_angle,
                        end_angle,
                        thickness,
                    } => {
                        4u8.hash(&mut h);
                        for v in [center_x, center_y, radius, start_angle, end_angle, thickness] {
                            f(&mut h, v);
                        }
                    }
                    CustomElement::Bezier {
                        from_x,
                        from_y,
                        ctrl1_x,
                        ctrl1_y,
                        ctrl2_x,
                        ctrl2_y,
                        to_x,
                        to_y,
                        thickness,
                    } => {
                        5u8.hash(&mut h);
                        for v in [
                            from_x, from_y, ctrl1_x, ctrl1_y, ctrl2_x, ctrl2_y, to_x, to_y, thickness,
                        ] {
                            f(&mut h, v);
                        }
                    }
                }
                effect(&mut h, c.custom_layout_settings);
            }
            RenderCommand::ScissorStart(b) => {
                6u8.hash(&mut h);
                bbox(&mut h, b);
            }
            RenderCommand::ScissorEnd => 7u8.hash(&mut h),
        }
    }

    h.finish()
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct UIImageDescriptor {
    pub atlas: &'static str,
    pub u1: f32,
    pub v1: f32,
    pub u2: f32,
    pub v2: f32,
}

impl UIImageDescriptor {
    pub const fn new() -> Self {
        UIImageDescriptor { atlas: "", u1: 0.0, v1: 0.0, u2: 1.0, v2: 1.0 }
    }
    pub const fn from_v_slice(atlas: &'static str, u1: f32, u2: f32) -> Self {
        UIImageDescriptor { atlas, u1, v1: 0.0, u2, v2: 1.0 }
    }
    pub const fn from_atlas(atlas: &'static str) -> Self {
        UIImageDescriptor { atlas, u1: 0.0, v1: 0.0, u2: 1.0, v2: 1.0 }
    }
    pub const fn v_slice(&mut self, u1: f32, u2: f32) {
        self.u1 = u1;
        self.u2 = u2;
    }
}

/// Bind-group layout for `@group(0)` of `ui_shader.wgsl` (a filterable 2D
/// texture + sampler). Both [`GlyphAtlas`] and `create_atlas` build a
/// structurally identical one; wgpu matches them by structure.
fn atlas_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("texture_bind_group_layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

impl GlyphAtlas {
    const SIZE: u32 = 1024;

    fn new(device: &wgpu::Device) -> Self {
        let size = Self::SIZE;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph_atlas"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("glyph_atlas_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glyph_atlas_bind_group"),
            layout: &atlas_bind_group_layout(device),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        Self {
            texture,
            bind_group,
            allocator: AtlasAllocator::new(size2(size as i32, size as i32)),
            size,
            map: HashMap::new(),
        }
    }
}

pub trait UIAtlasCreation {
    fn create_atlas(atlas_data: DynamicImage, device: &wgpu::Device, queue: &wgpu::Queue) -> Self;
}

impl UIAtlasCreation for wgpu::BindGroup {
    fn create_atlas(atlas_data: DynamicImage, device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let diffuse_rgba = atlas_data.to_rgba8();

        use image::GenericImageView;
        let dimensions = atlas_data.dimensions();

        let texture_size = wgpu::Extent3d {
            width: dimensions.0,
            height: dimensions.1,
            depth_or_array_layers: 1,
        };

        let diffuse_texture = device.create_texture(&wgpu::TextureDescriptor {
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            label: Some("diffuse_texture"),
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &diffuse_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &diffuse_rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * dimensions.0),
                rows_per_image: Some(dimensions.1),
            },
            texture_size,
        );

        let diffuse_texture_view =
            diffuse_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let diffuse_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
                label: Some("texture_bind_group_layout"),
            });

        device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&diffuse_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&diffuse_sampler),
                },
            ],
            label: Some("diffuse_bind_group"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in effect shaders (prelude + body) parse and validate with
    /// naga - the same check `API::register_ui_shader` runs on custom shaders,
    /// but no GPU needed. Guards the shipped WGSL in CI.
    #[test]
    fn built_in_effect_wgsl_validates() {
        for (name, body) in [
            ("ui_effects.wgsl", include_str!("ui_effects.wgsl")),
        ] {
            let full = effect_source(body);
            let module = wgpu::naga::front::wgsl::parse_str(&full)
                .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(&full)));
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(&full)));
        }
    }

    /// The standalone backdrop-blur shader (`blur.wgsl`) parses and validates.
    #[test]
    fn blur_wgsl_validates() {
        let src = include_str!("blur.wgsl");
        let module = wgpu::naga::front::wgsl::parse_str(src)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
    }

    /// A trivial custom shader (just `fs_main`) validates against the prelude.
    #[test]
    fn minimal_custom_effect_wgsl_validates() {
        let body = "@fragment fn fs_main(in: VertexPayload) -> @location(0) vec4<f32> { \
            return vec4<f32>(in.color.rgb, in.color.a * in.params.x); }";
        let full = effect_source(body);
        let module = wgpu::naga::front::wgsl::parse_str(&full)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&full)));
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&full)));
    }
}
