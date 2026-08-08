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

// **A stance was once a facing too, and is not one any more.**
//
// `outward(stance)` lived here — a table turning each of the five named stances
// into the direction its surface looks, read by `statics.wesl` to write the
// G-buffer's normal plane. `docs/lighting_rebuild.md` phase 6c deleted it: a
// fragment's normal is the face of the box its own view ray met
// (`package::impostor`), which is a measurement rather than a table, and the
// table was *wrong* for two of its five rows — it answered `(0, -1, 0)` for a
// north face, the side turned away from the viewer, where what a camera sees of
// a north wall is its `+y` surface. Five graphics out of 1197 stand on a north
// or west edge, which is why nothing caught it; the impostor cannot spell it,
// because a ray from the camera only ever leaves a box through `+x`, `+y` or
// `+z`.
//
// What is left of a stance is the thing no geometry states: **which side of its
// own tile a panel stands on**. `blit.wesl` reads it for the shadow walk's
// own-run test and to narrow a corner instance's owner to one of its two panels,
// and for the sentinel that routes a mesh face's id to its own instance buffer.
// `crate::place::Stance` is the Rust side of the same word.
//
// `crate::place::Stance::normal` still states the old table, and deliberately:
// its readers are the hand-built G-buffers of `plan.rs` and the test fixtures,
// which state their own scenes rather than reproduce this pass. See its own doc.

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
