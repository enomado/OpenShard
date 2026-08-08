// The statics pass: one sprite-sized quad per static, drawn at its own scale.
//
// Simpler than the ground in every way that matters. A static has one picture,
// one position and one height, so there is no shape to choose and nothing to
// stretch: the quad is the sprite's own rectangle in viewport pixels, and a
// texel is a pixel. What it does have, and ground does not, is transparency —
// a sprite is a picture with a shape rather than a diamond with a known one, so
// the fragment shader discards on alpha.
//
// **The picture is the artist's and the geometry is ours.**
// `docs/lighting_rebuild.md` phase 6: where a fragment *is* and which way its
// surface looks are not guessed back from the stance the art was read as — they
// are found by meeting this pixel's own view ray with the boxes the occlusion
// grid holds for **this instance**. See `package::impostor` for the meeting and
// `statics::push_volumes` for the association. So there is one silhouette, the
// sprite's; a pixel is already one instance's, because that instance's own quad
// drew it; and a static whose art overhangs its own volume produces a *number*
// (`Meeting::outside`) rather than a hole for a border constant to cover.
//
// Vertex-buffer instancing, one uniform block, one sampled texture and one
// storage buffer — which the header once said was kept out for WebGL2's ceiling,
// a stale objection: the crate's ceiling is **WebGPU** (see `crate` docs, and
// `docs/lighting.md` decision 30.5) and `blit.wgsl` beside this reads eleven of
// them.
//
// The depth comes in per instance and is written whole. It is not derived here
// because it is one ordering shared with the ground pass, and a second copy of
// the formula is a second chance to disagree with it — see `crate::depth`.

import package::place_format::{
    KIND_MASK, STANCE_FACE_NORTH, STANCE_FACE_SOUTH, STANCE_CORNER, pack_ids,
};
import package::normal_format::pack_normal;
import package::impostor::{hit, meets, ray_from};

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

// One box a static stands as — `crate::impostor::Volume`, and the frame's whole
// list of them: every drawn instance names its own run of this by the two words
// at the tail of its row. A fragment is met against **its own** run and no
// other, which is what makes the silhouettes agree by construction rather than
// by a border.
//
// `pad` is the vector's own alignment rather than a spare field, and `solid` is
// there because the row it fills is paid for either way — nothing in this pass
// reads it yet. See `crate::impostor::Volume::write`, which is the one place
// this layout is spelled on the other side.
struct Volume {
    lo: vec3<f32>,
    pad: f32,
    hi: vec3<f32>,
    solid: u32,
};

@group(0) @binding(5) var<storage, read> volumes: array<Volume>;

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
    // Which run of `volumes` is this instance's own: `(offset, count)`, and
    // `(_, 0)` for a picture the grid holds no shape for. See
    // `crate::impostor::Range`.
    @location(9) @interpolate(flat) volumes: vec2<u32>,
};

// Virtual pixels one unit of height lifts a sprite up the screen —
// `camera::Z_STEP`, and the same number `ground.wgsl` is handed in its uniform
// block. A constant here because this pass is not given the world's grid, only
// rectangles somebody else placed in it.
//
// Read by the *no volume* case alone now: a picture nothing measured a shape for
// is a billboard, and what runs down a billboard is height. Everything with a
// box gets its height from the box.
const Z_STEP: f32 = 4.0;

// Half a tile's height: how far above the sprite's bottom edge the tile's own
// centre is, since that edge stands on the diamond's bottom vertex. The one
// thing this pass still spells about the projection — `impostor::ray_from` is
// the rest of it.
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
    // NORMAL_FORMAT`, one octahedral word, `normal_format.wesl`. This pass is
    // where a **corner** is taken apart, which is the whole reason the normal is
    // written here rather than derived by the reader: a corner's two faces are
    // two halves of one picture and one pixel is on one of them, and by the time
    // anything downstream sees this fragment that decision has already been made
    // and thrown away.
    @location(3) normal: u32,
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
    // Per instance: which run of `volumes` is its own, as
    // `crate::impostor::Range` wrote it.
    @location(8) volumes: vec2<u32>,
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
    out.volumes = volumes;
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
    // the id word is a single face, which is the side of the tile `blit.wgsl`'s
    // own-run test and `own_solid` ask about. Before this a corner was `Upright`
    // — a flat 44-pixel band between two continuous runs of wall, lit
    // identically on the side turned towards the flame and the side turned away.
    // See `crate::facing::Facing::on_half`.
    //
    // **The screen half and not the box the ray met**, which is the one join
    // this phase left standing: the two panels of a corner are two boxes and the
    // impostor does pick between them for the *normal*, so a fragment could name
    // its own panel outright. What stops it is the id — the left half takes
    // `in.twin`, the row `split_corners` appended, and the box carries no row
    // number to choose one by. See `docs/lighting_rebuild.md` phase 6c.
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
    // How far this pixel is below the row its tile's own centre projects to.
    // With `across` beside it, the two numbers `impostor::ray_from` needs.
    let down = in.pixel_y - (in.bottom_y - HALF_TILE_HEIGHT);

    // The tile the picture stands on, off the instance's own first word — the
    // one `blit.wgsl` fetches back out of `face_instances[id]`.
    let tile = vec2<f32>(f32(in.place.x & 0xFFFFu), f32(in.place.x >> 16u));

    // **Where this fragment is, and which way its surface looks.**
    //
    // Both come from meeting this pixel's own view ray with the boxes *this
    // instance* stands as. What that replaced was an inverse projection guessed
    // from the stance the art was read as: a floor's two fractions taken from the
    // pixel's offset from the tile's centre, a wall's one fraction pinned to the
    // edge its face names and the other run along it, a height recovered from
    // `pixel_y` at four pixels a unit — five branches, each stating a plane the
    // geometry already knew, and every one of them clamped a
    // hundred-and-twenty-seventh of a tile inside the tile so that a reader
    // flooring the result would not land in the neighbour. Nothing floors it: the
    // walk starts from the tile the instance carries, which is a whole number a
    // pass knew exactly.
    //
    // The default is the **no volume** case, and it is the one reading that is
    // not geometry: a picture the grid holds no shape for — a mobile, a text
    // glyph, a static the grid refused — stands in the middle of its tile with
    // its height running down the picture at four pixels a unit, and has no
    // facing at all. That is what `Stance::Upright` has always meant, said once
    // here instead of five times below, and phase 7 is what gives a billboard a
    // normal.
    var at = vec3<f32>(tile + vec2<f32>(0.5), base - down / Z_STEP);
    var normal = vec3<f32>(0.0);
    if in.volumes.y != 0u {
        let start = ray_from(tile, base, across, down);
        // `impostor::nearest`, spelled here because the boxes are a binding: the
        // box in *front* wins among those the ray genuinely goes through — the
        // largest `t` along the view direction — and if it goes through none of
        // them, the one it passes closest to. That last case is the sprite
        // overhanging its own volume, and `met.outside` is how far.
        var best = meets(start, volumes[in.volumes.x].lo, volumes[in.volumes.x].hi);
        for (var k = 1u; k < in.volumes.y; k = k + 1u) {
            let volume = volumes[in.volumes.x + k];
            let met = meets(start, volume.lo, volume.hi);
            // A real hit always beats a miss, however near the miss came; two
            // hits are decided by which is in front, and two misses by which came
            // closer. Ties go to the box the list holds first, which for a flight
            // is the grid's own order — a tread's top before the tread above it.
            var better = false;
            if hit(best) == hit(met) {
                if hit(met) {
                    better = met.t > best.t;
                } else {
                    better = met.outside < best.outside;
                }
            } else {
                better = hit(met);
            }
            if better {
                best = met;
            }
        }
        at = best.at;
        normal = best.normal;
    }

    // The stance rides into the id word above the kind, and **it is not a
    // facing any more.** The normal is measured now — it is the face of the box
    // the view ray met — and what is left of the stance is the thing no geometry
    // states: *which side of its own tile* a panel stands on. `blit.wgsl` reads
    // it for exactly that, twice — `own_side` for the shadow walk's own-run test,
    // and `own_solid`, which narrows a corner instance's owner to one of its two
    // panels — plus the sentinel that routes a mesh face's id to its own buffer.
    // See `crate::place::Place`.
    //
    // Nothing else rides here. The **tile**'s `x`/`y` went to this pass's own
    // instance buffer at `docs/gbuffer.md` step 3, read back by `blit.wgsl` as
    // `face_instances[id]`, since the whole tile is the same two numbers on
    // every one of a sprite's pixels; the height and the fraction went to the
    // position plane at `docs/lighting_rebuild.md` phase 2, since both are this
    // fragment's own and both were being quantised on the way.
    out.ids = pack_ids(id, stance, (in.place.y >> 8u) & KIND_MASK);
    out.position = vec4<f32>(at, 1.0);
    // `pack_normal` carries the two bits that separate the three answers: a zero
    // vector is a real one here — a picture with no measured volume has no side
    // — so "nothing drew this pixel" needs a bit of its own to be
    // distinguishable from it.
    out.normal = pack_normal(normal);
    return out;
}
