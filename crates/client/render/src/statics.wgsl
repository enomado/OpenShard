// The statics pass: one sprite-sized quad per static, drawn at its own scale.
//
// Simpler than the ground in every way that matters. A static has one picture,
// one position and one height, so there is no shape to choose and nothing to
// stretch: the quad is the sprite's own rectangle in viewport pixels, and a
// texel is a pixel. What it does have, and ground does not, is transparency —
// a sprite is a picture with a shape rather than a diamond with a known one, so
// the fragment shader discards on alpha.
//
// Inside WebGL2's ceiling, like its neighbour: vertex-buffer instancing, one
// uniform block, one sampled texture.
//
// The depth comes in per instance and is written whole. It is not derived here
// because it is one ordering shared with the ground pass, and a second copy of
// the formula is a second chance to disagree with it — see `crate::depth`.

struct Viewport {
    // The target's size in *real* pixels — see `ground.wgsl`, which documents
    // the same three fields at length; this pass reads them identically and
    // deliberately, because two passes that scaled differently would draw two
    // pictures rather than one wrong one.
    size: vec2<f32>,
    // Real pixels per virtual pixel.
    scale: f32,
    _padding: f32,
    // The virtual-pixel point that lands in the middle of the target.
    origin: vec2<f32>,
    // Uniform blocks are sized in multiples of 16 bytes.
    _tail: vec2<f32>,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;
// `hues.mul`'s ramps, `crate::hue::HueRamp::width()` wide by one row a hue —
// see that module for why the index rides in the atlas texture's own red
// channel instead of a second one.
@group(0) @binding(3) var hue_ramp: texture_2d<f32>;
@group(0) @binding(4) var hue_sampler: sampler;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // Zero is "no hue"; otherwise the wire value `Hue` carries, index and
    // partial-flag both, untouched until the fragment shader needs them.
    @location(1) @interpolate(flat) hue: u32,
    // Where in the world this sprite stands, flat across the instance — see
    // `crate::place`. A wall's picture is 44 pixels above the tile it stands on,
    // and this is what says so: the lighting pass reads the tile rather than
    // guessing it back from the pixel's position on the screen.
    @location(2) @interpolate(flat) place: vec2<u32>,
    // Where this fragment is up the screen, and where the sprite's bottom edge
    // is, both in viewport pixels. Their difference is the *height* the pixel
    // stands at: a sprite is a picture of something vertical, four pixels of it
    // are one unit of `z`, and its bottom edge sits on the diamond's bottom
    // vertex. Without this a wall is lit as one flat thing from its base tile,
    // which reads as a lantern shining on a signboard.
    @location(3) pixel_y: f32,
    @location(4) @interpolate(flat) bottom_y: f32,
    // Where this fragment is across the screen, and where the sprite's middle
    // is: their difference is how far along the world's `x - y` axis the pixel
    // is from the tile's own column, because that axis is the horizontal one in
    // this projection. See `crate::place::Stance`.
    @location(5) pixel_x: f32,
    @location(6) @interpolate(flat) middle_x: f32,
};

// Virtual pixels one unit of height lifts a sprite up the screen —
// `camera::Z_STEP`, and the same number `ground.wgsl` is handed in its uniform
// block. A constant here because this pass is not given the world's grid, only
// rectangles somebody else placed in it.
const Z_STEP: f32 = 4.0;

// How far below a tile's centre the diamond's bottom vertex is, in `z` units:
// half a tile of 44 pixels over `Z_STEP`. A sprite's bottom edge stands there,
// so a pixel on it is that much *below* the height the static is based at.
const BOTTOM_LIFT: f32 = 5.5;

// The fourth channel of the place attachment: the kind in the low two bits, then
// seven bits of tile-local `x` and seven of tile-local `y`. See `crate::place`
// and `blit.wgsl`, which take the same word apart. The channel is full, so the
// kind is masked out of the instance's word before the fraction is laid over it
// — the stance rides in that word too and has done its work by then.
const KIND_MASK: u32 = 3u;
const SUB_TILE: f32 = 127.0;

// A tile's width in virtual pixels, which in this projection is also the number
// of pixels a whole tile of `x - y` covers: one step of world `x` moves the
// picture 22 pixels right and 22 down, one step of `y` moves it 22 left and 22
// down, so the two axes each cover half of a 44-pixel cell in both directions.
const TILE_WIDTH: f32 = 44.0;

// Half a tile's height: how far above the sprite's bottom edge the tile's own
// centre is, since that edge stands on the diamond's bottom vertex.
const HALF_TILE_HEIGHT: f32 = 22.0;

// `crate::place::Stance::Flat`, in the bit `Place::packed` writes it to.
const STANCE_FLAT: u32 = 1u << 16u;

// What one fragment of a world pass writes: the picture, and where in the world
// it came from.
struct FragmentOut {
    @location(0) color: vec4<f32>,
    @location(1) place: vec4<u32>,
};

@vertex
fn vs_main(
    // The unit quad: (0,0) top-left to (1,1) bottom-right, in screen axes this
    // time rather than the map's.
    @location(0) corner: vec2<f32>,
    // Per instance: the sprite's top-left corner, in viewport pixels.
    @location(1) origin: vec2<f32>,
    // Per instance: its size in pixels.
    @location(2) size: vec2<f32>,
    // Per instance: where it sits in the atlas, as (u, v, du, dv).
    @location(3) region: vec4<f32>,
    // Per instance: where it sorts. Smaller is nearer.
    @location(4) depth: f32,
    // Per instance: the wire hue, or 0 for none.
    @location(5) hue: u32,
    // Per instance: the tile and height, packed as
    // `crate::place::Place::packed` writes it.
    @location(6) place: vec2<u32>,
) -> VertexOut {
    let pixel = origin + corner * size;
    // Virtual pixels to real ones, the same single line the ground pass ends on.
    let real = (pixel - viewport.origin) * viewport.scale + viewport.size * 0.5;

    // Pixels to clip space, the same way the ground does it: `y` flips because
    // the viewport counts down from the top and clip space counts up from the
    // middle.
    let ndc = vec2<f32>(
        real.x / viewport.size.x * 2.0 - 1.0,
        1.0 - real.y / viewport.size.y * 2.0,
    );

    var out: VertexOut;
    out.clip = vec4<f32>(ndc, depth, 1.0);
    // The quad is the sprite's own size in pixels, so the corner coordinates
    // *are* the region's edges and a fragment's centre lands on a texel's
    // centre. No half-texel inset: unlike a stretched land texture, nothing
    // here samples the far edge of the region at a vertex.
    out.uv = region.xy + corner * region.zw;
    out.hue = hue;
    out.place = place;
    out.pixel_y = pixel.y;
    out.bottom_y = origin.y + size.y;
    out.pixel_x = pixel.x;
    // The sprite is centred on the tile's column — `statics::stand_on`, which is
    // `View.DrawStatic`'s `x -= (width >> 1) - 22`. Half a pixel out on an
    // odd-width sprite, exactly as it is there.
    out.middle_x = origin.x + size.x * 0.5;
    return out;
}

// The bits of a wire hue that are the index into `hues.mul`, and the bit
// asking for a partial (grey-pixels-only) tint. `openshard_uofiles::hues`
// is the port these came from — see `Hues::get` and `is_partial`.
const HUE_INDEX_MASK: u32 = 0x3FFFu;
const HUE_PARTIAL_FLAG: u32 = 0x8000u;

@fragment
fn fs_main(in: VertexOut) -> FragmentOut {
    let color = textureSample(atlas_texture, atlas_sampler, in.uv);

    // The sprite's shape. Discarding rather than blending is what keeps the
    // pass independent of draw order — a discarded fragment writes no depth
    // either, so the ground behind a tree's gaps stays visible.
    if color.a < 0.5 {
        discard;
    }

    var rgb = color.rgb;
    if in.hue != 0u {
        let partial = (in.hue & HUE_PARTIAL_FLAG) != 0u;
        // A partial hue leaves anything already coloured alone — skin, metal,
        // whatever the art painted with its own hue rather than in grey — and
        // only a full hue always replaces the pixel. Equality rather than a
        // tolerance: nearest sampling reproduces the atlas's stored bytes
        // exactly, with no filtering to blur two channels apart.
        if !partial || (color.r == color.g && color.g == color.b) {
            // The art's red channel is not a colour here, it is the index
            // `get_rgb(color.r, hue)` reads — see `crate::hue`'s docs for why
            // `round(r * 31.0)` recovers the file's 5-bit value exactly.
            let index = i32(round(color.r * 31.0));
            let row = i32((in.hue & HUE_INDEX_MASK) - 1u);
            rgb = textureLoad(hue_ramp, vec2<i32>(index, row), 0).rgb;
        }
    }

    var out: FragmentOut;
    out.color = vec4<f32>(rgb, 1.0);

    // The tile, the height and the kind, taken apart the way
    // `crate::place::Place::packed` put them together. A discarded fragment
    // never reaches this line, so what the attachment holds is what is visible.
    let base = f32(in.place.y & 0xFFu) - 128.0;
    let flat = (in.place.y & STANCE_FLAT) != 0u;

    // Where this pixel is relative to the tile's own centre, in virtual pixels:
    // across, always, and down the screen only for a picture that lies in the
    // tile. For an upright sprite what runs down the picture is height, not
    // ground, and it is read as height below.
    let across = in.pixel_x - in.middle_x;
    var down = 0.0;
    if flat {
        down = in.pixel_y - (in.bottom_y - HALF_TILE_HEIGHT);
    }

    // And the same two numbers as world axes. The projection is
    // `screen = ((x - y) * 22, (x + y) * 22)`, so inverting it over the pair is
    // two sums — see `camera::project`, which is the forward direction of
    // exactly this. An upright sprite has no `down` to invert, so both fractions
    // come out of `across` alone and are mirror images: a pixel to the right of
    // the column is further along `x` and equally less along `y`.
    let local = vec2<f32>(across + down, down - across) / TILE_WIDTH + vec2<f32>(0.5);
    // Clamped rather than wrapped, and only a sprite wider than its tile ever
    // reaches it: a tree is 100 pixels across and the world has no room to say
    // its edges are on the neighbouring tile — the attachment holds one tile per
    // pixel, and it is the one the thing stands on.
    let sub = clamp(local, vec2<f32>(0.0), vec2<f32>(1.0));

    // The height this pixel stands at. For a wall that is the sprite's own
    // picture — the bottom edge is `BOTTOM_LIFT` below the base and every four
    // pixels up is one unit of `z` — and for a floor it is the tile's height
    // everywhere, because what runs down a floor's picture is the tile, which
    // `down` has already spent.
    var z = base;
    if !flat {
        z = base - BOTTOM_LIFT + (in.bottom_y - in.pixel_y) / Z_STEP;
    }

    out.place = vec4<u32>(
        in.place.x & 0xFFFFu,
        in.place.x >> 16u,
        u32(clamp(round(z), -128.0, 127.0) + 128.0),
        ((in.place.y >> 8u) & KIND_MASK)
            | (u32(round(sub.x * SUB_TILE)) << 2u)
            | (u32(round(sub.y * SUB_TILE)) << 9u),
    );
    return out;
}
