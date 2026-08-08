// What a fragment *is* — the vocabulary `crate::place`'s `Kind` and `Stance`
// state in Rust, and the packing of the G-buffer's id plane that carries them.
// Every world pass (`ground.wgsl`, `statics.wgsl`, `mesh_face.wgsl`) writes it
// and every reader (`blit.wgsl`, `select.wgsl`) takes it apart.
//
// One `pub const` block instead of five hand-copied ones. Before this file,
// each of those five shader modules carried its own copy of these numbers —
// unavoidable under plain WGSL, since each pass's shader module is its own
// compilation unit and WGSL has no `#include`. A producer that forgot to
// stamp a bit this module's readers branch on decoded as a meaningful-but-
// wrong default instead of failing to compile: `ground.wgsl` never stamping
// `STANCE_FLAT` (session 23, `docs/lighting_raymarch.md`) is what that gap
// looks like when it fires, not a one-off. `crate::gbuffer::pack_ids` and its
// neighbouring constants state the same numbers once more on the Rust side;
// nothing but a person reading both can check the two still agree — see their
// own doc comments.
//
// **This used to be the `place` attachment's format**, four `u16` channels
// holding an id, a height in whole units and sixteenths, a stance and a
// hundred-and-twenty-seventh-of-a-tile fraction on each axis.
// `docs/lighting_rebuild.md` phase 2 took the height and the fraction out of
// it — a position plane carries the point itself — and what was left of eight
// bytes a fragment was six bits and an id. So it is one `u32` now, and the
// packing below is the whole of it.

// The id word's low two bits: `crate::place::Kind`.
const KIND_MASK: u32 = 3u;
const KIND_NOTHING: u32 = 0u;
const KIND_LAND: u32 = 1u;
const KIND_STATIC: u32 = 2u;
const KIND_MOBILE: u32 = 3u;

// Steps of the tile-local grid a fragment's position is held inside its own
// tile on: a hundred-and-twenty-seventh of a tile, on each axis.
//
// **No longer a packing.** Nothing quantises a fraction to it any more; what
// is left is the *clamp* the world passes apply to keep a fragment's position
// inside the tile its id names — `statics.wesl`'s `INSIDE`, and `mesh_face`'s
// own copy of the same number. `docs/lighting_rebuild.md` phase 4 is where the
// cell stops being a fact separate from the position and this goes with it.
const SUB_TILE: f32 = 127.0;

// Where the stance and the id ride above the kind in that word.
//
// Two bits of kind, four of stance — ten values, one flat, one unknown, four
// wall faces, four corners, and the mesh-face sentinel above them — and the
// twenty-six left over are the id. `crate::gbuffer::IDS_STANCE_SHIFT` and
// `IDS_ID_SHIFT` are the same two numbers in Rust.
//
// Twenty-six bits is not a budget anyone is near: an id is a row in one of
// four instance buffers and the widest frame this client has drawn is in the
// low thousands. What the layout is actually chosen for is the *bottom* —
// `KIND_NOTHING` is zero and so is the plane's clear value, so a pixel nothing
// drew and a pixel a pass stamped as nothing are the same word, which is the
// one invariant every reader's first branch rests on.
const IDS_STANCE_SHIFT: u32 = 2u;
const IDS_STANCE_MASK: u32 = 15u;
const IDS_ID_SHIFT: u32 = 6u;

// `crate::place::Stance`, in the four bits above.
const STANCE_UPRIGHT: u32 = 0u;
const STANCE_FLAT: u32 = 1u;
const STANCE_FACE_NORTH: u32 = 2u;
const STANCE_FACE_EAST: u32 = 3u;
const STANCE_FACE_SOUTH: u32 = 4u;
const STANCE_FACE_WEST: u32 = 5u;
// The first of the four corner stances — see `crate::place::Stance::CornerNorthSouth`
// and `STANCE_CORNER` in Rust for the arithmetic that resolves one to a face.
const STANCE_CORNER: u32 = 6u;
// `crate::place::Stance::MeshFace` — not a real stance, a routing sentinel
// the mesh-face pass writes instead of one of the five above.
const STANCE_MESH_FACE: u32 = 10u;

// Which way a face looks, in tiles. Zero for everything that has no face.
//
// The direction is the *drawn* one, which is the same thing: the art only ever
// draws the two faces an isometric camera can see — a wall stands on its tile's
// `y1` or `x1` edge and its picture is the surface turned towards the viewer —
// so a south face looks towards `+y` and an east face towards `+x`. North and
// west are five graphics out of 1197 and are here because the geometry has four
// edges. `docs/lighting.md`, step 15.
//
// A **flat** surface looks up, and that is where this was once missing: a
// wall's top cap is a `Flat` static, so nothing tested which way it looked and
// a lamp beside a wall lit its top as fully as one standing over it — decision
// 27. Land carries the same stance and must *not* be gated, and what tells the
// two apart is the *kind*: `ground.wesl` writes a zero normal outright rather
// than calling this. `crate::place::Stance::normal` states the same pair once
// more on the Rust side.
//
// `STANCE_UPRIGHT` looks nowhere: a billboard has no side, so every flame that
// reaches it lights it. That is a statement about what is not known, and
// `light::Surface::normal` returns nothing for the same reason. A **corner**
// never arrives here — `statics.wesl` has already resolved it to the face of
// the half the fragment was drawn on — and `STANCE_MESH_FACE` never does
// either: the mesh pass carries `crate::mesh::Face`'s own normal on its
// vertices.
//
// **This lives here and not in `blit.wesl`, where it used to.** A normal is now
// something a world pass *writes* — `docs/lighting_rebuild.md` phase 2, the
// G-buffer's normal plane — and the pass that writes it is `statics.wesl`,
// which is the file that already resolved the corner. The lighting reads the
// plane and has no case for a stance at all.
fn outward(stance: u32) -> vec3<f32> {
    switch stance {
        case STANCE_FACE_NORTH: { return vec3<f32>(0.0, -1.0, 0.0); }
        case STANCE_FACE_EAST: { return vec3<f32>(1.0, 0.0, 0.0); }
        case STANCE_FACE_SOUTH: { return vec3<f32>(0.0, 1.0, 0.0); }
        case STANCE_FACE_WEST: { return vec3<f32>(-1.0, 0.0, 0.0); }
        case STANCE_FLAT: { return vec3<f32>(0.0, 0.0, 1.0); }
        default: { return vec3<f32>(0.0); }
    }
}

// What `ground.wgsl`/`statics.wgsl`/`mesh_face.wgsl` each built by hand, one
// literal per file, before this. Sharing it closes half of session 23's bug
// class rather than the whole of it — see `docs/lighting_raymarch.md`'s own
// note on the distinction — but it closes the more dangerous half: `stance`
// is a required parameter here, so a producer literally cannot build an id
// word without deciding on one. Getting the *value* wrong (passing
// `STANCE_UPRIGHT` where `STANCE_FLAT` was meant) is still possible and still
// a bug, but it is now a line a person reading the call site can see and
// question — not a bit that simply never got OR'd in, invisible until a
// scene happens to tell the two apart.
//
// `crate::gbuffer::pack_ids` is the Rust twin, for the two things on that side
// that build a G-buffer without drawing a frame.
fn pack_ids(id: u32, stance: u32, kind: u32) -> u32 {
    return (kind & KIND_MASK)
        | ((stance & IDS_STANCE_MASK) << IDS_STANCE_SHIFT)
        | (id << IDS_ID_SHIFT);
}

// And the three questions asked of one, so that no reader spells a shift.
fn ids_kind(word: u32) -> u32 {
    return word & KIND_MASK;
}

fn ids_stance(word: u32) -> u32 {
    return (word >> IDS_STANCE_SHIFT) & IDS_STANCE_MASK;
}

fn ids_id(word: u32) -> u32 {
    return word >> IDS_ID_SHIFT;
}
