struct SizeUniform{
    x: f32,
    y: f32,
};

struct Vertex {
    @location(0)position: vec3<f32>,
    @location(1)texture: u32,
    @location(2)color: vec3<f32>,
};

struct VertexPayload {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) texture: u32,
    @location(3) tex_coords: vec2<f32>
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
    out.tex_coords.x = vertex.color.x;
    out.tex_coords.y = vertex.color.y;
    return out;
}

@group(0) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(0) @binding(1)
var s_diffuse: sampler;

@fragment
fn fs_main(in:VertexPayload) -> @location(0) vec4<f32> {
    switch in.texture {
        case 0u { return vec4<f32>(in.color, 1.0); }
        case 1u { return textureSample(t_diffuse, s_diffuse, in.tex_coords); }
        case default { return textureSample(t_diffuse, s_diffuse, in.tex_coords); }
    }
}