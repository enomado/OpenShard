// The silhouette pass: highlighted sprites, as shapes rather than pictures.
//
// The vertex half is `statics.wgsl`'s, line for line, and deliberately so — a
// silhouette that landed one pixel from the sprite it belongs to would ring the
// wrong outline, and the only defence against that is the same arithmetic. What
// differs is the fragment half: instead of the art's colour it writes *which
// object* the texel belongs to, into an `R8Uint` mask.
//
// The id is `instance_index + 1`, so zero stays free for "nothing here". That
// is why the caller hands this pass a list of only the sprites to be outlined
// rather than the whole frame's: the list's own order is the numbering, and no
// field on `SpriteQuad` has to carry it. 255 outlined objects at once, against
// the one the cursor is over today.
//
// The depth buffer is the world's, loaded and tested but not written: the mask
// must hold the id of whoever is *visible*, or a barrel behind a wall would be
// ringed through the wall.

struct Viewport {
    size: vec2<f32>,
    scale: f32,
    _padding: f32,
    origin: vec2<f32>,
    _tail: vec2<f32>,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) id: u32,
};

@vertex
fn vs_main(
    @builtin(instance_index) instance: u32,
    @location(0) corner: vec2<f32>,
    @location(1) origin: vec2<f32>,
    @location(2) size: vec2<f32>,
    @location(3) region: vec4<f32>,
    @location(4) depth: f32,
) -> VertexOut {
    let pixel = origin + corner * size;
    let real = (pixel - viewport.origin) * viewport.scale + viewport.size * 0.5;
    let ndc = vec2<f32>(
        real.x / viewport.size.x * 2.0 - 1.0,
        1.0 - real.y / viewport.size.y * 2.0,
    );

    var out: VertexOut;
    out.clip = vec4<f32>(ndc, depth, 1.0);
    out.uv = region.xy + corner * region.zw;
    // Zero is "nothing here", so the first instance is 1.
    out.id = instance + 1u;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) u32 {
    // The same alpha cut the picture pass makes, and it has to be the same
    // number: the mask is the shape of what was drawn, so a silhouette that
    // kept a texel the sprite discards would put a ring round a fringe nobody
    // can see.
    let color = textureSample(atlas_texture, atlas_sampler, in.uv);
    if color.a < 0.5 {
        discard;
    }
    return in.id;
}
