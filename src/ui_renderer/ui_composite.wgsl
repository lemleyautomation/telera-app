// Fullscreen-triangle blit of the cached UI texture over the already-rendered
// 3D scene. No vertex buffer: the three corners come from `vertex_index`.

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var out: VertexOut;
    // (0,0) (2,0) (0,2) in UV space -> a triangle that covers the whole screen.
    let uv = vec2<f32>(
        f32((vertex_index << 1u) & 2u),
        f32(vertex_index & 2u),
    );
    out.uv = uv;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return out;
}

@group(0) @binding(0) var t_ui: texture_2d<f32>;
@group(0) @binding(1) var s_ui: sampler;

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return textureSample(t_ui, s_ui, in.uv);
}
