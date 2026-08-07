// The `place` attachment's own bit format — what `crate::place::Place::packed`
// writes on the CPU side and what every world pass (`ground.wgsl`,
// `statics.wgsl`, `mesh_face.wgsl`) and every reader (`blit.wgsl`,
// `select.wgsl`) must agree on to draw one shadow correctly.
//
// One `pub const` block instead of five hand-copied ones. Before this file,
// each of those five shader modules carried its own copy of these numbers —
// unavoidable under plain WGSL, since each pass's shader module is its own
// compilation unit and WGSL has no `#include`. A producer that forgot to
// stamp a bit this module's readers branch on decoded as a meaningful-but-
// wrong default instead of failing to compile: `ground.wgsl` never stamping
// `STANCE_FLAT` (session 23, `docs/lighting_raymarch.md`) is what that gap
// looks like when it fires, not a one-off. `crate::place::STANCE_SHIFT`
// states the same numbers once more on the Rust side; nothing but a person
// reading both can check the two still agree — see its own doc comment.
//
// Migrated one pass at a time — `ground.wgsl` first (this module exists
// because of its bug), the others tracked as backlog in
// `docs/lighting_raymarch.md`. A shader not yet migrated still carries its
// own copy of whatever subset of these it needs, unchanged.

// The fourth channel's low two bits: `crate::place::Kind`.
const KIND_MASK: u32 = 3u;
const KIND_NOTHING: u32 = 0u;
const KIND_LAND: u32 = 1u;
const KIND_STATIC: u32 = 2u;
const KIND_MOBILE: u32 = 3u;

// The fourth channel's upper bits, above the kind: a hundred-and-twenty-
// seventh of a tile in each of the two tile-local axes.
const SUB_TILE_MASK: u32 = 127u;
const SUB_TILE: f32 = 127.0;

// Where the third channel's stance bits sit — `z + 128` in the low eight
// bits of a `u16`, stance in four of the eight above.
// `crate::place::STANCE_SHIFT` is the same number, stated once more in Rust.
const PLACE_STANCE_SHIFT: u32 = 8u;
const PLACE_STANCE_MASK: u32 = 15u;
const PLACE_Z_MASK: u32 = 255u;

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

// What `ground.wgsl`/`statics.wgsl`/`mesh_face.wgsl` each built by hand, one
// `vec4<u32>` literal per file, before this: `id` split across the first two
// channels, `z` and `stance` folded into the third, `kind` and the tile-local
// fraction into the fourth. Sharing this closes half of session 23's bug
// class rather than the whole of it — see `docs/lighting_raymarch.md`'s own
// note on the distinction — but it closes the more dangerous half: `stance`
// is a required parameter here, so a producer literally cannot build a
// `place` value without deciding on one. Getting the *value* wrong (passing
// `STANCE_UPRIGHT` where `STANCE_FLAT` was meant) is still possible and still
// a bug, but it is now a line a person reading the call site can see and
// question — not a bit that simply never got OR'd in, invisible until a
// scene happens to tell the two apart.
//
// `raw_z` and `sub` are each pre-clamp: every call site before this clamped
// `z` to `-128.0..127.0` and `sub` to its own tile (statics/mesh_face to
// `0.0..INSIDE`, ground to the full `0.0..1.0`) before folding it in, and
// this repeats both clamps rather than trusting the caller already did —
// harmless where a caller's own range is already inside `0.0..1.0`, and a
// second gate on the one thing every one of the three used to get separately
// right.
fn pack_place(id: u32, raw_z: f32, stance: u32, kind: u32, sub: vec2<f32>) -> vec4<u32> {
    let z = u32(clamp(round(raw_z), -128.0, 127.0) + 128.0) | (stance << PLACE_STANCE_SHIFT);
    let clamped = clamp(sub, vec2<f32>(0.0), vec2<f32>(1.0));
    let packed_sub = (u32(round(clamped.x * SUB_TILE)) << 2u) | (u32(round(clamped.y * SUB_TILE)) << 9u);
    return vec4<u32>(id & 0xFFFFu, id >> 16u, z, kind | packed_sub);
}
