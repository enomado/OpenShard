// The ground pass: one textured quad per land tile.
//
// Deliberately inside WebGL2's ceiling — vertex-buffer instancing, one uniform
// block, one sampled texture, no storage buffers and no compute. The unit quad
// arrives as a vertex buffer rather than being derived from `vertex_index`,
// because a pipeline with no vertex attributes is a place WebGL2 backends have
// historically differed.

struct Viewport {
    // Pixels across, and down.
    size: vec2<f32>,
    // The size of one land sprite in pixels: 44x44, from the file format.
    tile: vec2<f32>,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(
    // The unit quad: (0,0) to (1,1).
    @location(0) corner: vec2<f32>,
    // Per instance: the sprite's top-left corner, in viewport pixels.
    @location(1) origin: vec2<f32>,
    // Per instance: its place in the atlas, as (u, v, du, dv).
    @location(2) region: vec4<f32>,
) -> VertexOut {
    let pixel = origin + corner * viewport.tile;

    // Pixels to clip space. `y` flips: the viewport counts down from the top and
    // clip space counts up from the middle.
    let ndc = vec2<f32>(
        pixel.x / viewport.size.x * 2.0 - 1.0,
        1.0 - pixel.y / viewport.size.y * 2.0,
    );

    var out: VertexOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = region.xy + corner * region.zw;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let color = textureSample(atlas_texture, atlas_sampler, in.uv);

    // A land sprite is a diamond in a square, so a quarter of every quad is
    // transparent. Discarding rather than blending keeps the pass independent of
    // draw order, which is what lets a frame be compared byte for byte.
    if color.a < 0.5 {
        discard;
    }
    return vec4<f32>(color.rgb, 1.0);
}
