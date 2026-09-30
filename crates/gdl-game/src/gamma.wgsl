// The 3D picture was drawn and blended in gamma space, as the game's frame
// buffer is (gamma.rs); turn it into linear light for Bevy's sRGB output.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(hi, lo, c <= vec3(0.04045));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(picture, picture_sampler, in.uv);
    // The game's frame buffer holds 8 bits a channel: blends saturate.
    return vec4(srgb_to_linear(clamp(c.rgb, vec3(0.0), vec3(1.0))), c.a);
}
