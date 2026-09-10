// Built-in UI effects. Concatenated after `effect_prelude.wgsl` by
// `UIRenderer::effect_source`, so `VertexPayload`, `sd_rounded_box`, `size`,
// `t_diffuse`/`s_diffuse` and `vs_main` are already in scope.
//
// `in.effect` selects the effect; it matches `EffectKind`'s discriminant:
//   0 = drop shadow, 1 = raised edge, 2 = inner glow.
// Blur (`EffectKind::Blur`) is NOT here - it needs the destination texture and
// runs as a separate pass (`blur.wgsl`).

@fragment
fn fs_main(in: VertexPayload) -> @location(0) vec4<f32> {
    let local = in.frag_px - in.rect_center;
    // Distance to the element's own rounded box (shared by every effect).
    let d_elem = sd_rounded_box(local, in.rect_half, in.radii);

    switch in.effect {
        // Drop shadow. params = (offset_x, offset_y, blur, spread); params2 = shadow rgba.
        case 0u {
            let blur = max(in.params.z, 0.5);
            let d = sd_rounded_box(
                local - in.params.xy,
                in.rect_half + vec2<f32>(in.params.w, in.params.w),
                in.radii,
            );
            let shadow = 1.0 - smoothstep(-blur, blur, d);
            // Punch out the element's footprint so the shadow does not show
            // through a translucent fill.
            let covered = 1.0 - smoothstep(-1.0, 1.0, d_elem);
            let a = shadow * (1.0 - covered);
            return vec4<f32>(in.params2.rgb, in.params2.a * a);
        }
        // Raised / beveled edge. params = (width_px, light_angle_rad, 0, 0);
        // params2 = (highlight_strength, shade_strength, 0, 0). Emits a pure
        // white (lit) or black (shaded) sliver, alpha-blended over the fill, so
        // the element colour never needs to be known here.
        case 1u {
            let w = max(in.params.x, 1.0);
            // Only the inner band, and never outside the element.
            if (d_elem > 0.5 || d_elem < -w) {
                return vec4<f32>(0.0);
            }
            // Outward normal from the SDF gradient.
            let e = 1.5;
            let nx = sd_rounded_box(local + vec2<f32>(e, 0.0), in.rect_half, in.radii)
                   - sd_rounded_box(local - vec2<f32>(e, 0.0), in.rect_half, in.radii);
            let ny = sd_rounded_box(local + vec2<f32>(0.0, e), in.rect_half, in.radii)
                   - sd_rounded_box(local - vec2<f32>(0.0, e), in.rect_half, in.radii);
            let n = normalize(vec2<f32>(nx, ny));
            let light = vec2<f32>(cos(in.params.y), sin(in.params.y));
            let l = dot(n, light);
            // 1 at the edge, 0 at the inner end of the band (squared for a
            // tighter ridge).
            let fade = clamp(1.0 + d_elem / w, 0.0, 1.0);
            let f = fade * fade;
            let hi = max(l, 0.0) * in.params2.x * f;
            let lo = max(-l, 0.0) * in.params2.y * f;
            if (hi >= lo) {
                return vec4<f32>(1.0, 1.0, 1.0, clamp(hi, 0.0, 1.0));
            }
            return vec4<f32>(0.0, 0.0, 0.0, clamp(lo, 0.0, 1.0));
        }
        // Inner glow. params = (0, 0, blur, spread); params2 = glow rgba.
        case 2u {
            if (d_elem > 0.5) {
                return vec4<f32>(0.0);
            }
            let blur = max(in.params.z, 0.5);
            // 1 at the inner edge, 0 `blur` px in (offset inward by `spread`).
            let g = smoothstep(-blur, 0.0, d_elem + in.params.w);
            let a = clamp(g, 0.0, 1.0) * step(d_elem, 0.0);
            return vec4<f32>(in.params2.rgb, in.params2.a * a);
        }
        default {
            return in.color;
        }
    }
}
