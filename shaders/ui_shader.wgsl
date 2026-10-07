struct SizeUniform{
    x: f32,
    y: f32,
};

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) texture: u32,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
};

struct VertexPayload {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) @interpolate(flat) texture: u32,
    @location(2) uv: vec2<f32>,
};

@group(1) @binding(0)
var<uniform> size: SizeUniform;

@vertex
fn vs_main(vertex: Vertex) -> VertexPayload {
    var out: VertexPayload;
    out.position = vec4<f32>(
        (vertex.position.x/(size.x/2.0))-1,
        -((vertex.position.y/(size.y/2.0))-1),
        vertex.position.z,
        1.0
    );
    out.color = vertex.color;
    out.texture = vertex.texture;
    out.uv = vertex.uv;
    return out;
}

@group(0) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(0) @binding(1)
var s_diffuse: sampler;

@fragment
fn fs_main(in: VertexPayload) -> @location(0) vec4<f32> {
    // Sampled up front: WGSL only allows `textureSample` in uniform control
    // flow, and `in.texture` varies per fragment. naga doesn't enforce that
    // rule, but browsers' WebGPU compilers reject the whole pipeline.
    let texel = textureSample(t_diffuse, s_diffuse, in.uv);
    switch in.texture {
        // solid fill
        case 0u { return in.color; }
        // glyph coverage mask: tint by `color`, modulate alpha by atlas red
        case 2u { return vec4<f32>(in.color.rgb, in.color.a * texel.r); }
        // colour glyph bitmap (emoji): straight RGBA from the atlas
        case 3u { return texel; }
        // image atlas
        default { return texel; }
    }
}
