// `docs/gbuffer.md` step 4c's render pass: one raw, non-instanced vertex per
// corner of a `crate::mesh::Face`, fan-triangulated on the CPU
// (`Face::fan`). No colour target — the enclosing static's own billboard
// sprite already drew the picture (`statics.wgsl`), and this pass exists
// only to give that same static's pixels a more honest per-face normal and
// position than one blended stance (a former `facing::Prism::tread_normal`,
// retired by step 5 once this pass made it unnecessary) could.
//
// Depth is not derived here either, for the same reason `statics.wgsl` says
// so of its own: it is the enclosing static's own `SpriteQuad::depth`,
// carried through unchanged, so this pass's depth test can only tie or
// improve on what the billboard sprite already wrote — never disagree with
// it (`docs/gbuffer.md` decision 4).

import package::place_format::{KIND_STATIC, STANCE_MESH_FACE, pack_ids};
import package::normal_format::pack_normal;

struct Viewport {
    // Real pixels per virtual pixel — see `ground.wgsl`, which documents the
    // same three fields at length; every world pass reads them identically.
    size: vec2<f32>,
    scale: f32,
    _padding: f32,
    // The virtual-pixel point that lands in the middle of the target.
    origin: vec2<f32>,
    // Uniform blocks are sized in multiples of 16 bytes.
    _tail: vec2<f32>,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // This vertex's true world position, carried rather than recomputed:
    // linear interpolation across the triangle gives every fragment its own
    // exact world position for free, because the projection is affine and
    // every face is planar — see `crate::mesh_face::MeshFaceVertex::world`.
    @location(0) world: vec3<f32>,
    // This face's row in `blit.wgsl`'s `mesh_instances` — flat, identical for
    // all six vertices one face's fan produces.
    @location(1) @interpolate(flat) id: u32,
    // The tile this face stands on — `crate::mesh_face::MeshFaceRow::tile`
    // again, flat like `id`: every corner of one face agrees on it, and the
    // fragment stage needs it to place a fragment inside its own tile without
    // flooring a position that can legitimately sit on the edge shared with
    // the next one. See `fs_main`'s own comment.
    @location(2) @interpolate(flat) tile: vec2<f32>,
    // And this face's own normal — `crate::mesh::Face::normal`, flat like the
    // two above because a face is planar and every corner of it agrees.
    //
    // The one producer in the crate whose normal is *measured geometry* rather
    // than a stance: a tread's top and its riser are two `Face`s with two real
    // normals, and `docs/gbuffer.md` decision 3 is the argument for keeping them
    // apart. The row's `stance` is the same fact rounded to one of five names —
    // `Stance::of_normal`, on the CPU — and it is still what the shadow walk's
    // own-run test reads until phase 4 replaces that with an id.
    @location(3) @interpolate(flat) normal: vec3<f32>,
};

@vertex
fn vs_main(
    // This corner in `Camera::to_view_exact`'s space — the same
    // pre-viewport-zoom space `statics.wgsl`'s own `origin` is in.
    @location(0) screen: vec2<f32>,
    @location(1) world: vec3<f32>,
    // Where this face sorts: the enclosing static's own depth, reused whole.
    @location(2) depth: f32,
    @location(3) id: u32,
    @location(4) tile: vec2<f32>,
    @location(5) normal: vec3<f32>,
) -> VertexOut {
    // Virtual pixels to real ones, the same single line `statics.wgsl`'s own
    // vertex stage ends on.
    let real = (screen - viewport.origin) * viewport.scale + viewport.size * 0.5;
    let ndc = vec2<f32>(
        real.x / viewport.size.x * 2.0 - 1.0,
        1.0 - real.y / viewport.size.y * 2.0,
    );

    var out: VertexOut;
    out.clip = vec4<f32>(ndc, depth, 1.0);
    out.world = world;
    out.id = id;
    out.tile = tile;
    out.normal = normal;
    return out;
}

// **`INSIDE` lived here**, and `docs/lighting_rebuild.md` phase 4 deleted it. It
// was a hundred-and-twenty-seventh of a tile — the grain the retired `place`
// attachment held a fragment inside its tile on — and this pass clamped its own
// exact position into it so that a reader flooring the position would be handed
// the tile the row names rather than the neighbour's. Nothing floors it: the walk
// takes its starting cell from the row, which is the whole of what
// `MeshFaceVertex::tile` is for. What the clamp cost was eight thousandths of a
// tile of error in the ray's origin on exactly the fragments a flight's outer
// corner is made of, and with the bias at zero that is the largest thing left
// between a fragment and where it really is.

struct FragmentOut {
    // What drew this fragment and which row it came from —
    // `crate::gbuffer::IDS_FORMAT`.
    @location(0) ids: u32,
    // And where that fragment is, unpacked and unquantised — `crate::gbuffer`.
    // This pass is the one producer that had it exactly all along: `in.world`
    // is the rasteriser's own interpolation of three real corners, which is
    // why the phase's test picks a mesh face to compare against.
    @location(1) position: vec4<f32>,
    // And which way it looks, carried whole from the vertex rather than named
    // by a stance and read back out of one — packed by `normal_format.wesl`.
    @location(2) normal: u32,
};

@fragment
fn fs_main(in: VertexOut) -> FragmentOut {
    var out: FragmentOut;
    out.ids = pack_ids(in.id, STANCE_MESH_FACE, KIND_STATIC);
    // **`in.world`, exactly**, which this pass has had all along: the
    // rasteriser's own interpolation of three real corners, affine across a
    // planar face. It went through `in.tile + clamp(in.world.xy - in.tile, …)`
    // until phase 4, which changed nothing but the outermost eight thousandths of
    // a tile of a face — and those are exactly a flight's own outer corner, the
    // fragments every stair defect is found on. See the note where `INSIDE` stood.
    //
    // Which *tile* this fragment's walk starts from is a separate fact and stays
    // one: `MeshFaceVertex::tile`, carried on the vertex. A face's own edge can
    // sit exactly on a whole number — this tread's outer corner does — and
    // `floor`ing a position there picks whichever side rounds down rather than the
    // side the geometry stands on. `docs/lighting.md`'s shadow-raymarch anomaly is
    // that bug: one fragment on an otherwise evenly-shadowed tread reading fully
    // lit, because the walk it fed started from the wrong tile's occluders.
    out.position = vec4<f32>(in.world, 1.0);
    out.normal = pack_normal(in.normal);
    return out;
}
