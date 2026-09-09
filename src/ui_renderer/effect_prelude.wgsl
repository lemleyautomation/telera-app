// Shared contract for every UI effect shader - the built-in effects
// (`ui_effects.wgsl`) and every custom shader loaded via a `#### TML`
// `` - `shader` [name](path.wgsl) `` directive. `UIRenderer::effect_source`
// concatenates this file in front of the effect body, so a custom shader only
// provides `fs_main`.
//
// Vertex input is `EffectVertex` (see `ui_renderer.rs`); bind group 0 is the
// currently bound texture atlas, bind group 1 is the viewport size uniform -
// exactly the layout the main `ui_shader.wgsl` uses, so an effect batch can be
// drawn in the same render pass without rebinding groups.

struct SizeUniform {
    x: f32,
    y: f32,
};

struct EffectVertex {
    @location(0) position: vec3<f32>,
    @location(1) effect: u32,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) rect_center: vec2<f32>,
    @location(5) rect_half: vec2<f32>,
    @location(6) radii: vec4<f32>,
    @location(7) params: vec4<f32>,
    @location(8) params2: vec4<f32>,
};

struct VertexPayload {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) @interpolate(flat) effect: u32,
    @location(2) uv: vec2<f32>,
    // Fragment position in physical pixels (same space as `rect_center`).
    @location(3) frag_px: vec2<f32>,
    @location(4) rect_center: vec2<f32>,
    @location(5) rect_half: vec2<f32>,
    // Corner radii in px: (top_left, top_right, bottom_left, bottom_right).
    @location(6) radii: vec4<f32>,
    // `shader-param-1..4` for a custom shader; effect-specific for built-ins.
    @location(7) params: vec4<f32>,
    // `shader-param-5..8` for a custom shader; effect-specific for built-ins.
    @location(8) params2: vec4<f32>,
};

@group(1) @binding(0)
var<uniform> size: SizeUniform;

@group(0) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(0) @binding(1)
var s_diffuse: sampler;

@vertex
fn vs_main(vertex: EffectVertex) -> VertexPayload {
    var out: VertexPayload;
    out.position = vec4<f32>(
        (vertex.position.x / (size.x / 2.0)) - 1.0,
        -((vertex.position.y / (size.y / 2.0)) - 1.0),
        vertex.position.z,
        1.0
    );
    out.color = vertex.color;
    out.effect = vertex.effect;
    out.uv = vertex.uv;
    out.frag_px = vertex.position.xy;
    out.rect_center = vertex.rect_center;
    out.rect_half = vertex.rect_half;
    out.radii = vertex.radii;
    out.params = vertex.params;
    out.params2 = vertex.params2;
    return out;
}

// Signed distance to a rounded box centred at the origin, half-extents `b`,
// per-corner radii `r4` = (top_left, top_right, bottom_left, bottom_right).
// Negative inside, zero on the edge, positive outside. `p` is in the same
// pixel space as `rect_center` (screen y points down, so p.y < 0 is the top).
fn sd_rounded_box(p: vec2<f32>, b: vec2<f32>, r4: vec4<f32>) -> f32 {
    var r: f32;
    if (p.x > 0.0) {
        r = select(r4.w, r4.y, p.y < 0.0);
    } else {
        r = select(r4.z, r4.x, p.y < 0.0);
    }
    let q = abs(p) - b + vec2<f32>(r, r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0, 0.0))) - r;
}
