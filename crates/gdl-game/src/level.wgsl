// Level geometry, combined exactly like the game's two TEV stages
// (docs/rendering.md), in gamma space — written out in gamma space too, so
// blending works on gamma values like the game's frame buffer (gamma.rs
// turns the finished picture linear):
//   stage 0: clamp(texture x rasterized colour x 2)
//   stage 1 (lightmap): stage 0 x lightmap alpha
//   or stage 1 (a texture over the object, the game's override −4: a death
//   texture, a hit flash): clamp(rasterized colour x that texture x 2),
//   alpha that texture's where stage 0's is above 2/255, else 0; then x
//   rasterized alpha (docs/rendering.md, "Texture overrides")

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_functions
#import bevy_pbr::mesh_view_bindings::view

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var diffuse_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var diffuse_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var lightmap_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var lightmap_sampler: sampler;
// x: lightmap enabled (2: the second texture is a dying monster's death
// texture instead; 3: the texture is a chrome power-up's, its coordinates
// from the normals), y: alpha cutoff, z: stage 0 scale, w: lit by the
// level light instead of prelit vertex colours
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

// The mesh's tag (flash.rs): bits 0-23 a colour (0xRRGGBB, opaque); bit 24
// draws it as the texture over the object (override −4), bit 25 in place of
// the object's texture (override −2); bit 26 skips the lightmap stage
// (render flag 0x4000).
const TAG_OVER: u32 = 0x1000000u;
const TAG_REPLACE: u32 = 0x2000000u;
const TAG_NO_LIGHTMAP: u32 = 0x4000000u;

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

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    let tag = mesh_functions::get_tag(in.instance_index);
    let tag_color = vec4(f32((tag >> 16u) & 0xFFu), f32((tag >> 8u) & 0xFFu), f32(tag & 0xFFu), 255.0) / 255.0;
#ifdef VERTEX_UVS_A
    var uv = in.uv + uv_offset.xy;
    if (params.x > 2.5) {
        // The chrome (override −3, draw flag 0x80000): the texture's
        // coordinates are the normal along the camera's right and up.
        let n = normalize(in.world_normal);
        uv = vec2(dot(n, view.world_from_view[0].xyz), dot(n, view.world_from_view[1].xyz));
    }
    var color = textureSample(diffuse_texture, diffuse_sampler, uv);
#else
    var color = vec4(1.0);
#endif
    if ((tag & TAG_REPLACE) != 0u) {
        color = tag_color;
    }
    // The rasterized colour.
    var ras = vec4(1.0);
#ifdef VERTEX_COLORS
    if (params.w > 0.5) {
        // The game's software lighting for unlit vertices: object colour x
        // (ambient + N.L x light colour), clamped to a byte.
        // Normal-less geometry (blob shadows) gets ambient only.
        let n = in.world_normal;
        let len = length(n);
        let d = select(0.0, max(dot(n / max(len, 1e-6), light_dir.xyz), 0.0), len > 1e-4);
        ras = vec4(clamp(light_color.a * (vec3(light_dir.w) + d * light_color.rgb), vec3(0.0), vec3(1.0)), in.color.a);
    } else {
        ras = in.color;
    }
#endif
    color = color * ras;
    color = vec4(clamp(color.rgb * params.z, vec3(0.0), vec3(1.0)), color.a);
#ifdef VERTEX_UVS_B
    let light = textureSample(lightmap_texture, lightmap_sampler, in.uv_b).a;
    if (params.x > 0.5 && params.x < 1.5 && (tag & TAG_NO_LIGHTMAP) == 0u) {
        color = vec4(color.rgb * light, color.a);
    }
#endif
    // A texture over the object: a dying monster's death texture
    // (docs/monsters.md), sampled with the object's own coordinates, or a
    // hit flash's one colour. The game has no such stage on lightmapped
    // objects.
    var over = vec4(0.0);
    var has_over = false;
#ifdef VERTEX_UVS_A
    if (params.x > 1.5 && params.x < 2.5) {
        over = textureSample(lightmap_texture, lightmap_sampler, in.uv);
        has_over = true;
    }
#endif
    if (params.x < 0.5 && (tag & TAG_OVER) != 0u) {
        over = tag_color;
        has_over = true;
    }
    if (has_over) {
        let a = select(0.0, over.a, color.a > 2.0 / 255.0);
        color = vec4(clamp(ras.rgb * over.rgb * 2.0, vec3(0.0), vec3(1.0)), ras.a * a);
    }
    if (color.a < params.y) {
        discard;
    }
    let rgb = clamp(color.rgb, vec3(0.0), vec3(1.0));
    color.a = color.a * (1.0 - uv_offset.w);
    if (uv_offset.z > 0.5) {
        // The game's additive blend: source x source alpha + destination.
        // Bevy draws `Add` as premultiplied alpha, so alpha 0 keeps all of
        // the destination.
        return output(in, vec4(rgb * color.a, 0.0));
    }
    return output(in, vec4(rgb, color.a));
}
