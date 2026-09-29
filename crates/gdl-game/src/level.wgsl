// Level geometry: diffuse texture x vertex colour x lightmap, computed in
// gamma space like the GameCube's fixed-function TEV, then converted to
// linear for Bevy's output.

#import bevy_pbr::forward_io::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var diffuse_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var diffuse_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var lightmap_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var lightmap_sampler: sampler;
// x: lightmap enabled, y: alpha cutoff, z: lightmap scale
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var<uniform> params: vec4<f32>;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(hi, lo, c <= vec3(0.04045));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
#ifdef VERTEX_UVS_A
    var color = textureSample(diffuse_texture, diffuse_sampler, in.uv);
#else
    var color = vec4(1.0);
#endif
#ifdef VERTEX_COLORS
    color = color * in.color;
#endif
#ifdef VERTEX_UVS_B
    let light = textureSample(lightmap_texture, lightmap_sampler, in.uv_b).a;
    if (params.x > 0.5) {
        color = vec4(color.rgb * light * params.z, color.a);
    }
#endif
    if (color.a < params.y) {
        discard;
    }
    return vec4(srgb_to_linear(clamp(color.rgb, vec3(0.0), vec3(1.0))), color.a);
}
