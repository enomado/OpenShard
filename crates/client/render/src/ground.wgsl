// The ground pass: one textured quad per land tile.
//
// Deliberately inside WebGL2's ceiling — vertex-buffer instancing, one uniform
// block, one sampled texture, no storage buffers and no compute. The unit quad
// arrives as a vertex buffer rather than being derived from `vertex_index`,
// because a pipeline with no vertex attributes is a place WebGL2 backends have
// historically differed.
//
// A tile has two shapes, and which one it gets is decided here rather than on
// the CPU, so the decision cannot disagree with the heights it is made from:
//
//   * Four equal corners — a *flat* tile. Drawn as the art's own 44x44 square,
//     texel for texel, with the diamond cut out of it by alpha. This is the
//     common case and the one a frame test can compare against the file.
//   * Anything else — a slope. Drawn as the diamond itself: four vertices at
//     the tile's four corners, each lifted by its own height. Neighbouring
//     tiles then share vertices instead of merely abutting, so a seam between
//     them is not expressible.
//
// On level ground the two agree exactly: the diamond's vertices land on the
// square's edge midpoints and the texture map is the identity.
//
// A slope is textured from `texmaps.mul` and not from the art. The art is a
// 44x44 diamond drawn for level ground, and pulling it over a steep quad smears
// it; the texture maps are square pictures of the same terrain, made to be
// mapped corner to corner onto exactly this shape. Which corner is which is
// ClassicUO's `DrawStretchedLand`, and it is the identity here: the unit quad's
// (0,0) is the tile's top vertex and the texture's top-left, (1,0) the right,
// (0,1) the left, (1,1) the bottom.
//
// A tile whose graphic has no texture — most of them do not — falls back to the
// stretched art. The client instead refuses to stretch such a tile at all and
// draws it flat, which leaves gaps against its stretched neighbours; ours stays
// watertight and is textured approximately. See docs/client.md.

struct Viewport {
    // The target's size in *real* pixels — the display's, which at a
    // magnification above 1 is not the world's own. Only the last two lines of
    // the vertex shader use it.
    size: vec2<f32>,
    // The size of one land sprite in virtual pixels: 44x44, from the file
    // format.
    tile: vec2<f32>,
    // Virtual pixels one unit of height lifts a corner up the screen.
    z_step: f32,
    // Real pixels per virtual pixel. One when the world is being drawn into the
    // offscreen image, which is the minifying path — see `camera::Projection`.
    scale: f32,
    // The point of the world's own grid that lands in the middle of the target,
    // in virtual pixels. Fractional: the eye is quantised to a real pixel, so at
    // 3x it carries thirds of a virtual one.
    origin: vec2<f32>,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;
@group(0) @binding(3) var texmap_texture: texture_2d<f32>;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // Constant across the instance, so ordinary interpolation reproduces it
    // exactly and no interpolation attribute is needed to keep it intact.
    @location(1) is_flat: f32,
    // Where to sample the texture atlas, and whether there is anything there.
    @location(2) texmap_uv: vec2<f32>,
    @location(3) has_texmap: f32,
    // The tile these pixels are, flat across the instance — see `crate::place`.
    @location(4) @interpolate(flat) place: vec2<u32>,
    // And the height, which is *not* flat: a hillside's pixels each carry the
    // one their corner interpolation gave them, which is the height the light
    // has to reach. Taking the tile's base instead would light a hilltop as if
    // it were the valley floor.
    @location(5) place_z: f32,
};

// What a pixel of the ground is, in `crate::place::Kind`'s numbering. The Rust
// side pins these values in a test; nothing else can compare the two.
const KIND_LAND: u32 = 1u;

// What one fragment of a world pass writes: the picture, and where in the world
// it came from.
struct FragmentOut {
    @location(0) color: vec4<f32>,
    @location(1) place: vec4<u32>,
};

@vertex
fn vs_main(
    // The unit quad: (0,0) to (1,1). (0,0) is the tile's own corner, and the
    // two axes are the map's: `x` runs down-right on screen, `y` down-left.
    @location(0) corner: vec2<f32>,
    // Per instance: the diamond's centre at height zero, in viewport pixels.
    @location(1) origin: vec2<f32>,
    // Per instance: the four corner heights, indexed `a + 2*b`.
    @location(2) heights: vec4<f32>,
    // Per instance: its place in the atlas, as (u, v, du, dv).
    @location(3) region: vec4<f32>,
    // Per instance: its place in the texture atlas, the same way. A size of
    // zero means the tile has no texture map.
    @location(4) texmap: vec4<f32>,
    // Per instance: where this tile sorts against everything else in the frame,
    // statics included. Computed on the CPU — see `crate::depth` — because it
    // is one ordering shared by two passes and neither may derive its own.
    @location(5) depth: f32,
    // Per instance: the tile, packed as `crate::place::Place::packed` writes it.
    @location(6) place: vec2<u32>,
) -> VertexOut {
    let half = viewport.tile * 0.5;
    let is_flat = f32(all(heights == vec4<f32>(heights.x)));

    // Bilinear over the unit square, which is exact at the four corners — the
    // only places it is evaluated.
    let height = mix(
        mix(heights.x, heights.y, corner.x),
        mix(heights.z, heights.w, corner.x),
        corner.y,
    );

    // The square: the sprite's own bounding box, centred on the tile.
    let flat_offset = (corner - vec2<f32>(0.5)) * viewport.tile;
    let flat_uv = corner;

    // The diamond: (0,0) is the top vertex, (1,0) the right, (0,1) the left and
    // (1,1) the bottom — one half-tile out from the centre along each screen
    // axis, which is the same projection the CPU used to place the centre.
    let slope_offset = vec2<f32>(
        (corner.x - corner.y) * half.x,
        (corner.x + corner.y - 1.0) * half.y,
    );
    let slope_uv = vec2<f32>(
        0.5 + (corner.x - corner.y) * 0.5,
        (corner.x + corner.y) * 0.5,
    );

    let offset = mix(slope_offset, flat_offset, is_flat);
    let local_uv = mix(slope_uv, flat_uv, is_flat);
    let pixel = origin + offset - vec2<f32>(0.0, height * viewport.z_step);

    // Virtual pixels to real ones, and the only line in this pass that knows a
    // magnification exists. Everything above is the art's own grid, which is
    // what makes it comparable against the files texel for texel.
    let real = (pixel - viewport.origin) * viewport.scale + viewport.size * 0.5;

    // Pixels to clip space. `y` flips: the viewport counts down from the top and
    // clip space counts up from the middle.
    let ndc = vec2<f32>(
        real.x / viewport.size.x * 2.0 - 1.0,
        1.0 - real.y / viewport.size.y * 2.0,
    );

    var out: VertexOut;
    // The whole tile shares one depth: a quad is one thing standing at one
    // place in the order, and a per-vertex depth would let a slope's own
    // corners sort against each other.
    out.clip = vec4<f32>(ndc, depth, 1.0);
    out.uv = region.xy + local_uv * region.zw;
    out.is_flat = is_flat;
    // Corner to corner: the unit quad's own coordinates *are* the texture's.
    out.texmap_uv = texmap.xy + corner * texmap.zw;
    out.has_texmap = f32(texmap.z > 0.0);
    out.place = place;
    out.place_z = height;
    return out;
}

// Where in the world this fragment's pixel is: the instance's tile, the
// interpolated corner height, and "this is ground".
fn place_of(in: VertexOut) -> vec4<u32> {
    let z = u32(clamp(round(in.place_z), -128.0, 127.0) + 128.0);
    return vec4<u32>(in.place.x & 0xFFFFu, in.place.x >> 16u, z, KIND_LAND);
}

@fragment
fn fs_main(in: VertexOut) -> FragmentOut {
    var out: FragmentOut;
    out.place = place_of(in);

    // A slope with a texture of its own takes it, and nothing else about the
    // tile changes: the shape is still the diamond its four corners make.
    if in.is_flat < 0.5 && in.has_texmap > 0.5 {
        let textured = textureSample(texmap_texture, atlas_sampler, in.texmap_uv);
        out.color = vec4<f32>(textured.rgb, 1.0);
        return out;
    }

    let color = textureSample(atlas_texture, atlas_sampler, in.uv);

    // A flat tile's quad is the sprite's square, so a quarter of it is outside
    // the diamond and must not be drawn. Discarding rather than blending keeps
    // the pass independent of draw order, which is what lets a frame be
    // compared byte for byte.
    //
    // A slope's quad *is* the diamond, so every sample is inside it by
    // construction — the corner texture coordinates are the diamond's own
    // vertices and interpolation cannot leave their convex hull. Discarding
    // there would only let a texel's worth of rounding punch a hole through the
    // ground, so it does not.
    if in.is_flat > 0.5 && color.a < 0.5 {
        discard;
    }
    out.color = vec4<f32>(color.rgb, 1.0);
    return out;
}
