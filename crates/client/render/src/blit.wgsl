// The world image, onto the viewport.
//
// No vertex buffer: four corners derived from the vertex index, which is a
// triangle strip covering clip space. The render pass sets the viewport rect, so
// "clip space" is whatever rectangle the UI left free for the world.

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    // (0,0) (1,0) (0,1) (1,1), the same unit quad the world passes use.
    let corner = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u));
    var out: VertexOut;
    // Clip space is y-up and the image is y-down, so the vertical axis flips
    // here rather than in the sampling — one negation, in the place where the
    // convention actually changes.
    out.position = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
    out.uv = corner;
    return out;
}

@group(0) @binding(0) var world: texture_2d<f32>;
@group(0) @binding(1) var world_sampler: sampler;

// One flame. Two `vec4`s rather than four fields, because a uniform array's
// stride is rounded up to 16 bytes either way and this is what the CPU writes:
//
//   place = (x, y, radius, intensity)   in the drawn image's own pixels
//   color = (r, g, b, unused)
struct Light {
    place: vec4<f32>,
    color: vec4<f32>,
};

// Sized to `Lighting::MAX`, and the two have to agree — this is a uniform
// array, so its length is fixed at compile time and a mismatch is a buffer
// wgpu rejects rather than a wrong picture.
const MAX_LIGHTS: u32 = 64u;

struct Lighting {
    // The world image's size in pixels, then how many of `lights` are real.
    // Packed together because a `vec4` is the smallest a uniform field gets.
    image: vec4<f32>,
    // What the whole frame is multiplied by away from every flame. `(1,1,1,_)`
    // is no lighting at all, and the identity below is exact for it.
    ambient: vec4<f32>,
    lights: array<Light, MAX_LIGHTS>,
};

@group(0) @binding(2) var<uniform> lighting: Lighting;

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Straight through: both sides are `Rgba8Unorm` and this crate never
    // converts colour. Whatever filtering happens is the sampler's, chosen by
    // the caller from which way the zoom went.
    let color = textureSample(world, world_sampler, in.uv);

    // Where this fragment is in the image being lit. The lights arrived in the
    // same space — `light::place` applies the projection on the CPU — so a
    // flame lands on the pixel the sprite making it landed on, at every zoom.
    let at = in.uv * lighting.image.xy;

    var lit = lighting.ambient.rgb;
    let count = u32(lighting.image.z);
    for (var i = 0u; i < count; i = i + 1u) {
        let light = lighting.lights[i];
        // Linear distance, squared falloff: `1 - d` alone gives a cone with a
        // visible straight edge in the gradient, and an inverse-square law with
        // no cutoff has no radius at all and tints the whole frame. This is the
        // soft pool with a hard end — the shape the reference isometrics draw.
        let d = distance(at, light.place.xy) / max(light.place.z, 1.0);
        let fall = clamp(1.0 - d, 0.0, 1.0);
        lit = lit + light.color.rgb * (light.place.w * fall * fall);
    }

    // Multiplicative, so a flame brightens whatever is under it — ground, wall,
    // the body standing in it — rather than laying a coloured film over it. A
    // pixel the art left black stays black, which is what makes the light look
    // like it is falling on the scene instead of floating above it.
    //
    // With no lights and a white ambient every channel is multiplied by exactly
    // `1.0` and this is the copy it has always been. `blit::Lighting::NONE` is
    // that case, and a frame test asserts the whole surface texel for texel.
    // Alpha carried through untouched, not written as `1.0`: this pass copies
    // an image and lighting is about how bright a pixel is, not about whether it
    // is there. The world passes already write an opaque frame; a surface whose
    // alpha this decided would differ from the world image in a channel nobody
    // lit, which is exactly what the copy test reads.
    return vec4<f32>(min(color.rgb * lit, vec3<f32>(1.0)), color.a);
}
