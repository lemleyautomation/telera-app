// Backdrop-blur sub-pass. Runs once, after the main UI pass, into the UI layer's
// own colour texture with `LoadOp::Load` - so it reads a *copy* of that texture
// (`UiSurface::blur_src`, bind group 0) and writes a rounded-rect-clipped
// gaussian back over each `` `shader` *blur* `` element's box.
//
// This blurs the UI layer behind the panel (not the 3D scene - copying the
// swapchain is not portable). Put a blur panel's label beside or below it, not
// inside: content drawn into the UI layer before this pass sits under the frost.

struct SizeUniform { x: f32, y: f32, };

@group(1) @binding(0) var<uniform> size: SizeUniform;
@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;

struct VIn {
    @location(0) position: vec2<f32>,     // px
    @location(1) rect_center: vec2<f32>,
    @location(2) rect_half: vec2<f32>,
    @location(3) radii: vec4<f32>,        // tl, tr, bl, br
    @location(4) params: vec4<f32>,       // (blur_radius_px, 0, 0, 0)
    @location(5) tint: vec4<f32>,         // normalised rgba
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) frag_px: vec2<f32>,
    @location(1) rect_center: vec2<f32>,
    @location(2) rect_half: vec2<f32>,
    @location(3) radii: vec4<f32>,
    @location(4) params: vec4<f32>,
    @location(5) tint: vec4<f32>,
    // 1 / viewport size, so the fragment stage never touches `size` (its bind
    // group layout is vertex-visibility only).
    @location(6) texel: vec2<f32>,
};

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

@vertex
fn vs_main(v: VIn) -> VOut {
    var o: VOut;
    o.clip = vec4<f32>(
        (v.position.x / (size.x / 2.0)) - 1.0,
        -((v.position.y / (size.y / 2.0)) - 1.0),
        0.0, 1.0,
    );
    o.frag_px = v.position;
    o.rect_center = v.rect_center;
    o.rect_half = v.rect_half;
    o.radii = v.radii;
    o.params = v.params;
    o.tint = v.tint;
    o.texel = vec2<f32>(1.0 / size.x, 1.0 / size.y);
    return o;
}

@fragment
fn fs_main(i: VOut) -> @location(0) vec4<f32> {
    let local = i.frag_px - i.rect_center;
    let d = sd_rounded_box(local, i.rect_half, i.radii);
    let cov = 1.0 - smoothstep(-1.5, 0.5, d);
    if (cov <= 0.001) {
        return vec4<f32>(0.0);
    }
    let radius = max(i.params.x, 0.5);
    let texel = i.texel;
    var acc = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var oy: i32 = -3; oy <= 3; oy = oy + 1) {
        for (var ox: i32 = -3; ox <= 3; ox = ox + 1) {
            let fo = vec2<f32>(f32(ox), f32(oy));
            let w = exp(-dot(fo, fo) * 0.28);
            let uv = clamp(
                (i.frag_px + fo * radius * 0.5) * texel,
                vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0),
            );
            acc = acc + textureSampleLevel(src_tex, src_samp, uv, 0.0).rgb * w;
            wsum = wsum + w;
        }
    }
    let col = mix(acc / wsum, i.tint.rgb, i.tint.a);
    return vec4<f32>(col, cov);
}
