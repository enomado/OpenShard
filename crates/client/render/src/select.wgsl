// The wash over what is selected: the picked sprite's own pixels, and the floor
// of the tile it stands on.
//
// Geometry is `blit.wgsl`'s and `outline.wgsl`'s: four corners from the vertex
// index, the render pass's viewport is the rectangle the UI left free, so `uv`
// means the same thing in all three and a world texel is found from a surface
// pixel with one multiplication.
//
// Two questions are asked per fragment, and they are asked of two different
// records because they are two different facts:
//
//   1. **Is this pixel the selected thing?** That is the mask: the silhouette
//      pass drew the picked static's quad into it, depth-tested against the
//      world, so what is in there is what is *visible* of it. Nothing in the
//      place attachment can answer this — two walls on one tile write the same
//      tile, the same height range and the same stance, and a screen-space pass
//      keyed on the attachment alone would shade both.
//   2. **Is this pixel the ground that thing stands on?** That is the place
//      attachment, which carries the tile of whatever was drawn at each pixel —
//      so the answer is a comparison and needs no second mask.
//
// The floor is *land or a flat static* of that tile, not land alone. Indoors the
// land is under a wooden floor and never drawn, so "the tile the wall stands on"
// would come out unshaded in exactly the building the player is pointing at. A
// flat static is a floor, a rug or a road — `Stance::Flat`, which is the client's
// own `Background` bit — and it is what is visibly *the ground here*.

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    let corner = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u));
    var out: VertexOut;
    out.position = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
    out.uv = corner;
    return out;
}

// Which selected object each world pixel belongs to, or zero for none. Its own
// mask and not the outline's: the ring pass draws an edge round every id it
// finds, so a selection sharing that texture would be ringed as well as shaded.
@group(0) @binding(0) var mask: texture_2d<u32>;
// Which tile each world pixel came from — `crate::place`, the same attachment
// the lighting reads.
@group(0) @binding(1) var place_of: texture_2d<u32>;

struct Selection {
    // The mask's size in texels, which is the world image's; the rest is
    // padding.
    shape: vec4<f32>,
    // The selected tile as (x, y), then whether there is one at all. Zero is
    // "no tile", which is the frame where something is selected that is not
    // standing anywhere — there is none today, and it is one comparison rather
    // than a second pipeline.
    tile: vec4<u32>,
    // The wash over the selected thing's own pixels: colour, and `a` is how much
    // of it replaces what is under it.
    sprite: vec4<f32>,
    // And over the ground it stands on.
    ground: vec4<f32>,
};

@group(0) @binding(2) var<uniform> selection: Selection;

// What the mask holds where nothing was drawn.
const EMPTY: u32 = 0u;

// `crate::place::Kind`, in the low two bits of the attachment's fourth channel.
// The same numbers `blit.wgsl` has, and pinned on the Rust side in
// `place::tests::the_kinds_are_the_numbers_the_shader_has`.
const KIND_MASK: u32 = 3u;
const KIND_LAND: u32 = 1u;
const KIND_STATIC: u32 = 2u;

// `crate::place::Stance`, above the height in the third channel — the same shift
// `statics.wgsl` writes it at and `blit.wgsl` reads it at.
const STANCE_SHIFT: u32 = 8u;
const STANCE_MASK: u32 = 15u;
const STANCE_FLAT: u32 = 1u;

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let size = vec2<i32>(selection.shape.xy);
    // Floor rather than round, which is `outline.wgsl`'s reason exactly: `uv`
    // runs 0..1 across the whole image, so this is the nearest-texel mapping the
    // magnifying blit does, and the wash is blocky in step with the art rather
    // than half a texel off it.
    let at = vec2<i32>(in.uv * selection.shape.xy);
    if at.x < 0 || at.y < 0 || at.x >= size.x || at.y >= size.y {
        discard;
    }

    var wash = vec4<f32>(0.0);
    if textureLoad(mask, at, 0).r != EMPTY {
        wash = selection.sprite;
    } else if selection.tile.z != 0u {
        let place = textureLoad(place_of, at, 0);
        let kind = place.w & KIND_MASK;
        let stance = (place.z >> STANCE_SHIFT) & STANCE_MASK;
        let ground = kind == KIND_LAND || (kind == KIND_STATIC && stance == STANCE_FLAT);
        if ground && place.x == selection.tile.x && place.y == selection.tile.y {
            wash = selection.ground;
        }
    }

    if wash.a <= 0.0 {
        discard;
    }
    // Premultiplied, like the ring pass and for the same reason: the target's
    // blend is `dst = src.rgb + dst * (1 - src.a)`, so a colour scaled by its own
    // alpha is the ordinary blend and the picture shows through by whatever is
    // left. A wash and not a replacement — the art has to stay readable under it,
    // or the selection has hidden the thing it is pointing at.
    return vec4<f32>(wash.rgb * wash.a, wash.a);
}
