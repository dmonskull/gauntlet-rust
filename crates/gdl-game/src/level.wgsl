// Level geometry, combined exactly like the game's two TEV stages
// (docs/rendering.md), in gamma space, then converted to linear:
//   stage 0: clamp(texture x rasterized colour x 2)
//   stage 1 (lightmap): stage 0 x lightmap alpha

#import bevy_pbr::forward_io::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var diffuse_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var diffuse_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var lightmap_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var lightmap_sampler: sampler;
// x: lightmap enabled (2: the second texture is a dying monster's death
// texture instead), y: alpha cutoff, z: stage 0 scale, w: lit by the level
// light instead of prelit vertex colours
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var<uniform> params: vec4<f32>;
// xyz: unit vector toward the light, w: ambient level
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var<uniform> light_dir: vec4<f32>;
// rgb: light colour x intensity, a: object colour (0x80/0xFF)
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var<uniform> light_color: vec4<f32>;
// xy: diffuse texture scroll (texture modifiers); z: additive; w: how far
// faded out (vanishing bridges)
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var<uniform> uv_offset: vec4<f32>;
// x: depth-test this much nearer the camera (an effect's depth bias)
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var<uniform> depth_offset: vec4<f32>;

struct FragmentOutput {
    @location(0) color: vec4<f32>,
#ifdef DEPTH_BIAS
    // Reverse Z: nearer is larger.
    @builtin(frag_depth) depth: f32,
#endif
}

fn output(in: VertexOutput, color: vec4<f32>) -> FragmentOutput {
    var out: FragmentOutput;
    out.color = color;
#ifdef DEPTH_BIAS
    out.depth = clamp(in.position.z + depth_offset.x, 0.0, 1.0);
#endif
    return out;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(hi, lo, c <= vec3(0.04045));
}

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
#ifdef VERTEX_UVS_A
    var color = textureSample(diffuse_texture, diffuse_sampler, in.uv + uv_offset.xy);
#else
    var color = vec4(1.0);
#endif
#ifdef VERTEX_COLORS
    if (params.w > 0.5) {
        // The game's software lighting for unlit vertices: object colour x
        // (ambient + N.L x light colour), clamped to a byte.
        // Normal-less geometry (blob shadows) gets ambient only.
        let n = in.world_normal;
        let len = length(n);
        let d = select(0.0, max(dot(n / max(len, 1e-6), light_dir.xyz), 0.0), len > 1e-4);
        let ras = clamp(light_color.a * (vec3(light_dir.w) + d * light_color.rgb), vec3(0.0), vec3(1.0));
        color = color * vec4(ras, in.color.a);
    } else {
        color = color * in.color;
    }
#endif
    color = vec4(clamp(color.rgb * params.z, vec3(0.0), vec3(1.0)), color.a);
#ifdef VERTEX_UVS_B
    let light = textureSample(lightmap_texture, lightmap_sampler, in.uv_b).a;
    if (params.x > 0.5 && params.x < 1.5) {
        color = vec4(color.rgb * light, color.a);
    }
#endif
#ifdef VERTEX_UVS_A
    if (params.x > 1.5) {
        // A dying monster (docs/monsters.md): its death texture over the
        // body, colour x 2 and alpha, the way the game's other stages scale.
        let death = textureSample(lightmap_texture, lightmap_sampler, in.uv);
        color = vec4(clamp(color.rgb * death.rgb * 2.0, vec3(0.0), vec3(1.0)), color.a * death.a);
    }
#endif
    if (color.a < params.y) {
        discard;
    }
    let rgb = srgb_to_linear(clamp(color.rgb, vec3(0.0), vec3(1.0)));
    color.a = color.a * (1.0 - uv_offset.w);
    if (uv_offset.z > 0.5) {
        // The game's additive blend: source x source alpha + destination.
        // Bevy draws `Add` as premultiplied alpha, so alpha 0 keeps all of
        // the destination.
        return output(in, vec4(rgb * color.a, 0.0));
    }
    return output(in, vec4(rgb, color.a));
}
