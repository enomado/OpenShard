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

import package::place_format::{
    KIND_MASK, SUB_TILE, STANCE_FLAT, STANCE_FACE_NORTH, STANCE_FACE_EAST,
    STANCE_FACE_SOUTH, STANCE_FACE_WEST, STANCE_CORNER, pack_ids, outward,
};

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
    // is. Their difference is how far the pixel is from the column its tile
    // stands in, and what that *means* is the stance's business: for a floor it
    // is half of the inverse projection, and for a wall it is how far along the
    // one edge of the diamond the wall stands on. See `crate::place::Stance`.
    @location(5) pixel_x: f32,
    @location(6) @interpolate(flat) middle_x: f32,
    // This instance's own index — what the attachment now carries in place of
    // `x`/`y`, and what `blit.wgsl` looks up in its own copy of this pass's
    // instance buffer, bound a second time as storage. See
    // `docs/gbuffer.md` decision 2 and step 3.
    @location(7) @interpolate(flat) id: u32,
    // The row of this instance's other face, for a corner — `0` and unread
    // for anything else. See `crate::sprite::SpriteQuad::twin` and
    // `docs/gbuffer.md` step 4.
    @location(8) @interpolate(flat) twin: u32,
};

// Virtual pixels one unit of height lifts a sprite up the screen —
// `camera::Z_STEP`, and the same number `ground.wgsl` is handed in its uniform
// block. A constant here because this pass is not given the world's grid, only
// rectangles somebody else placed in it.
const Z_STEP: f32 = 4.0;

// The largest fraction that still names *this* tile.
//
// `blit.wgsl` finds a fragment's cell with `floor(tile + fraction)`, so a
// fraction of exactly one lands in the *neighbour* — and the shadow walk exempts
// the fragment's own cell from occluding it, because a wall's own face must be
// the brightest thing beside a torch. A wall's face lies **on** the tile
// boundary, so writing a clean 1 there hands the walk the wrong cell: the wall's
// own tile stops being exempt and every faced wall is shadowed by the very wall
// it is the face of. It comes out at ambient, which is what a measurement of the
// first version of this said — a run of walls at 249 dropping to 65.
//
// One step of the seven-bit fraction is a hundred-and-twenty-seventh of a tile,
// 0.35 pixels of world. The seam does not notice and the cell is right.
const INSIDE: f32 = (SUB_TILE - 1.0) / SUB_TILE;

// A tile's width in virtual pixels, which in this projection is also the number
// of pixels a whole tile of `x - y` covers: one step of world `x` moves the
// picture 22 pixels right and 22 down, one step of `y` moves it 22 left and 22
// down, so the two axes each cover half of a 44-pixel cell in both directions.
const TILE_WIDTH: f32 = 44.0;

// Half of it, which is how far the diamond reaches either side of the column its
// sprite is centred on — and therefore how wide *one edge* of the diamond is on
// the screen. A wall stands on one edge, so this is the whole horizontal extent
// of a wall's face. Numerically the same 22 as `HALF_TILE_HEIGHT` and a different
// question: one is how far across, the other how far down.
const HALF_TILE_WIDTH: f32 = TILE_WIDTH * 0.5;

// Half a tile's height: how far above the sprite's bottom edge the tile's own
// centre is, since that edge stands on the diamond's bottom vertex.
const HALF_TILE_HEIGHT: f32 = 22.0;

// `crate::place::Stance`, in the four bits `Place::packed` writes it to — the
// *instance* word's own bits, a different word from the G-buffer's own
// `IDS_STANCE_SHIFT`. The numbers here are pinned on the Rust side; nothing
// but a person reading both files can check that these agree with them.
const STANCE_SHIFT: u32 = 16u;
const STANCE_MASK: u32 = 15u;

// What one fragment of a world pass writes: the picture, and where in the world
// it came from.
struct FragmentOut {
    @location(0) color: vec4<f32>,
    // What drew this fragment and which row it came from —
    // `crate::gbuffer::IDS_FORMAT`.
    @location(1) ids: u32,
    // And where in the world that fragment is — `crate::gbuffer`. The point
    // itself, and not the tile-local fraction and sixteenths of a `z` unit the
    // id word used to carry beside the id.
    @location(2) position: vec4<f32>,
    // And which way this fragment's surface looks — `crate::gbuffer::
    // NORMAL_FORMAT`. This pass is where a **corner** is taken apart, which is
    // the whole reason the normal is written here rather than derived by the
    // reader: a corner's two faces are two halves of one picture and one pixel
    // is on one of them, and by the time anything downstream sees this fragment
    // that decision has already been made and thrown away.
    @location(3) normal: vec4<f32>,
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
    // Per instance: a corner's paired shadow row, or `0`.
    @location(7) twin: u32,
    @builtin(instance_index) instance_index: u32,
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
    out.id = instance_index;
    out.twin = twin;
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

    // Where this pixel is on the screen relative to its tile: `across` from the
    // column the sprite is centred on, `down` from the tile's own centre row.
    // Both are what the projection is inverted through below.
    let across = in.pixel_x - in.middle_x;

    // A **corner** is two faces in one picture — the client draws every
    // building's corner that way — and which of them a pixel is a pixel *of* is
    // which half of the tile's column it was drawn on. So the corner is resolved
    // here, per fragment, and nothing downstream ever sees one: what goes into
    // the attachment is a single face with a single outward normal, which is what
    // `blit.wgsl` knows how to light. Before this a corner was `Upright` — a flat
    // 44-pixel band between two continuous runs of wall, lit identically on the
    // side turned away from the flame. See `crate::facing::Facing::on_half`.
    // A corner's two faces need two ids, not one shared between them — see
    // `docs/gbuffer.md` decision 3 and step 4. The right half (`across > 0.0`)
    // keeps this instance's own id; the left half takes `in.twin`, the row
    // `crate::sprite::split_corners` appended for exactly this face. Every
    // other stance never reaches the branch that changes it.
    var id = in.id;
    var stance = (in.place.y >> STANCE_SHIFT) & STANCE_MASK;
    if stance >= STANCE_CORNER {
        let offset = stance - STANCE_CORNER;
        if across > 0.0 {
            stance = STANCE_FACE_NORTH + (offset >> 1u);
        } else {
            stance = STANCE_FACE_SOUTH + (offset & 1u);
            id = in.twin;
        }
    }
    let down = in.pixel_y - (in.bottom_y - HALF_TILE_HEIGHT);
    // How far along a *face's* edge this pixel is, before the face says which
    // direction that edge runs in. One division, shared by the four cases.
    let run = across / HALF_TILE_WIDTH;

    // Where this pixel is in its tile.
    //
    // A picture that *lies* in the tile is the tile's diamond, so both fractions
    // come out of the pixel's offset from the tile's centre by inverting the
    // projection — `screen = ((x - y) * 22, (x + y) * 22)`, whose forward
    // direction is `camera::project`.
    //
    // A **wall** is a billboard rising from one edge of that diamond, so one of
    // its two fractions is fixed at the edge and the other runs along it. Which
    // edge is measured from the art and arrives here as the stance — see
    // `crate::facing`. The point of it is the seam: the next tile along the run
    // starts its fraction at 0 where this one ended at 1, so a row of wall tiles
    // is one continuous surface rather than a row of separately lit sprites.
    //
    // An **upright** picture with no face is the middle of its tile, and that is
    // a statement about what is *not* known rather than a shortcut: a tree, a
    // post, a wall the detector would not guess at. Reading the
    // horizontal offset as `x - y` for those, as this did for one commit, spreads
    // their pixels along the one direction no wall ever runs.
    //
    // Clamped rather than wrapped in every case: the attachment holds one tile
    // per pixel, and it is the one the thing stands on. For a face that matters
    // for a real reason — a wall sprite draws a sliver of its own thickness past
    // the edge it stands on, and those pixels belong to the near end of the edge.
    var sub = vec2<f32>(0.5);
    switch stance {
        case STANCE_FLAT: {
            let local = vec2<f32>(across + down, down - across) / TILE_WIDTH + vec2<f32>(0.5);
            sub = clamp(local, vec2<f32>(0.0), vec2<f32>(INSIDE));
        }
        case STANCE_FACE_NORTH: {
            sub = vec2<f32>(clamp(run, 0.0, INSIDE), 0.0);
        }
        case STANCE_FACE_EAST: {
            sub = vec2<f32>(INSIDE, clamp(1.0 - run, 0.0, INSIDE));
        }
        case STANCE_FACE_SOUTH: {
            sub = vec2<f32>(clamp(1.0 + run, 0.0, INSIDE), INSIDE);
        }
        case STANCE_FACE_WEST: {
            sub = vec2<f32>(0.0, clamp(-run, 0.0, INSIDE));
        }
        default: {}
    }

    // The height this pixel stands at.
    //
    // For anything standing up it is how far the pixel is above the point of the
    // tile its picture rises from, at four pixels a unit of `z`. That point is
    // `(sub.x + sub.y - 1) * 22` pixels below the tile's centre row — which is
    // the forward projection of the fraction just computed, so the four faces and
    // the faceless upright case are one line rather than five. With `sub` at the
    // tile's middle the term is zero and this is exactly what it was before faces
    // existed; with a face it is what makes a wall's base sit at the static's own
    // `z` all the way along the edge instead of only at the tile's centre.
    //
    // A floor is the tile's height everywhere, because what runs down a floor's
    // picture is the tile, which `down` has already spent. Its own branch rather
    // than the line above, which would read a *clamped* fraction back out through
    // the projection and give the corners of a 44-pixel floor sprite — which lie
    // outside the diamond — a height nobody drew them at.
    var z = base;
    if stance != STANCE_FLAT {
        z = base + ((sub.x + sub.y - 1.0) * HALF_TILE_HEIGHT - down) / Z_STEP;
    }

    // The stance rides into the id word above the kind, and it is not a
    // duplicate of what the position below already says: a position says
    // *where in its tile* a pixel is, and a face says **which way that surface
    // looks** — which the lighting cannot recover from a point, because a
    // wall's two faces are one tile and one plane. It is read for two things
    // now and neither is a normal: which instance buffer a mesh face's id
    // addresses, and the shadow walk's own-run test. See `crate::place::Place`.
    //
    // Nothing else rides here. The **tile**'s `x`/`y` went to this pass's own
    // instance buffer at `docs/gbuffer.md` step 3, read back by `blit.wgsl` as
    // `face_instances[id]`, since the whole tile is the same two numbers on
    // every one of a sprite's pixels; `z` and `sub` went to the position plane
    // at `docs/lighting_rebuild.md` phase 2, since both are this fragment's own
    // and both were being quantised on the way.
    out.ids = pack_ids(id, stance, (in.place.y >> 8u) & KIND_MASK);
    // And the same fragment as a point. The tile comes off the instance's own
    // first word — the one `blit.wgsl` fetches back out of `face_instances[id]`
    // — so this is what that pass reconstructs, computed where the two halves
    // are still both exact instead of after each has been through a field.
    let tile = vec2<f32>(f32(in.place.x & 0xFFFFu), f32(in.place.x >> 16u));
    out.position = vec4<f32>(tile + sub, z, 1.0);
    // And the normal, off the stance the corner branch above already resolved.
    // The fourth channel is coverage and not part of the vector: a zero normal
    // is a real answer here — an upright picture has no side — so "nothing drew
    // this pixel" needs a channel of its own to be distinguishable from it.
    out.normal = vec4<f32>(outward(stance), 1.0);
    return out;
}
