// Custom UI shader: a frosted-glass panel face - rounded-rect clipped, with a
// rim highlight near the edge and an animated diagonal sheen.
//
// telera prepends `effect_prelude.wgsl`, so this file only provides `fs_main`.
// Available: in.color (element color), in.uv (0..1 over the quad), in.frag_px,
// in.rect_center / in.rect_half / in.radii (px), in.params = shader-param-1..4,
// in.params2 = shader-param-5..8, plus `sd_rounded_box` and `size`.
//
//   shader-param-1  sheen strength   (0..1)
//   shader-param-2  sheen frequency
//   shader-param-3  sheen phase      (animate this)

@fragment
fn fs_main(in: VertexPayload) -> @location(0) vec4<f32> {
    let local = in.frag_px - in.rect_center;
    let d = sd_rounded_box(local, in.rect_half, in.radii);
    if (d > 0.5) {
        return vec4<f32>(0.0);
    }
    let rim = 1.0 - smoothstep(-7.0, 0.0, d);
    let sheen = clamp(
        0.5 + 0.5 * sin((in.uv.x + in.uv.y) * 6.2831 * max(in.params.y, 0.1) + in.params.z),
        0.0, 1.0,
    );
    let col = mix(in.color.rgb, vec3<f32>(1.0), rim * 0.45 + sheen * 0.14 * in.params.x);
    return vec4<f32>(col, in.color.a);
}
