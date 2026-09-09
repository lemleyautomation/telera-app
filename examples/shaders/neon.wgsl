// Custom UI shader: a glowing neon outline around the element box. The glow
// spills OUTSIDE the box, so the effect quad is expanded - keep the params
// modest or raise the expansion margin in `push_effect_quad`.
//
//   shader-param-1  glow width      (px)
//   shader-param-2  glow intensity  (animate this)

@fragment
fn fs_main(in: VertexPayload) -> @location(0) vec4<f32> {
    let local = in.frag_px - in.rect_center;
    let d = sd_rounded_box(local, in.rect_half, in.radii);
    let width = max(in.params.x, 1.0);
    let glow = exp(-abs(d) / width) * max(in.params.y, 0.0);
    let core = 1.0 - smoothstep(0.0, 2.0, abs(d));
    let rgb = in.color.rgb + vec3<f32>(core) * 0.6;
    let a = clamp(glow + core, 0.0, 1.0) * in.color.a;
    return vec4<f32>(rgb, a);
}
