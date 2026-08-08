// The world image, onto the viewport — and the lighting, on the way past.
//
// No vertex buffer: four corners derived from the vertex index, which is a
// triangle strip covering clip space. The render pass sets the viewport rect, so
// "clip space" is whatever rectangle the UI left free for the world.
//
// The lighting here is in *world* coordinates and not in this image's pixels:
// every fragment reads which tile its picture came from out of the place
// attachment (see `crate::place`) and is lit as that tile is lit. The screen
// folds height into `y` and a wall's sprite stands above the tile it occludes
// from, so nothing decided from a screen position can tell the lit face of a
// wall from the shadow behind it. `docs/lighting.md` is the argument.

import package::place_format::{
    KIND_NOTHING, KIND_LAND, KIND_STATIC,
    ids_kind, ids_stance, ids_id,
    STANCE_MESH_FACE, STANCE_FLAT, STANCE_FACE_NORTH,
    STANCE_FACE_EAST, STANCE_FACE_SOUTH, STANCE_FACE_WEST,
};

import package::normal_format::unpack_normal;

// The world's one metric, declared where the view ray needs it and imported
// here rather than restated — `docs/lighting_rebuild.md`'s *One metric*.
import package::impostor::Z_PER_TILE;

import package::tonemap::shade;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    // (0,0) (1,0) (0,1) (1,1), the same unit quad the world passes use.
    let corner = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u));
    var out: VertexOut;
    // Clip space is y-up and the image is y-down, so the vertical axis flips
    // here rather than in the sampling — one negation, in the place where the
    // convention actually changes.
    out.position = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
    out.uv = corner;
    return out;
}

@group(0) @binding(0) var world: texture_2d<f32>;
@group(0) @binding(1) var world_sampler: sampler;
// What drew each pixel of `world`: the kind, the stance and an id into one of
// the four instance buffers below — `crate::gbuffer::IDS_FORMAT`, one `u32`,
// and `place_format.wesl` is the layout. An id and not a literal tile since
// `docs/gbuffer.md` steps 3 and 7 moved it there for every kind. Never sampled
// — an integer texture has no filtering, and two ids averaged name a third row
// nothing was drawn from.
@group(0) @binding(3) var ids_of: texture_2d<u32>;
// And where that pixel's fragment actually is: `(x, y, z, 1)` in world
// coordinates, `crate::gbuffer::POSITION_FORMAT`. Never sampled either, and
// this one for a reason of its own — a position filtered against its
// neighbour's is a point on neither surface, floating in the air between a
// wall and the ground behind it.
@group(0) @binding(13) var position_of: texture_2d<f32>;
// And which way its surface looks: one octahedral word, or a word with no
// facing bit for a surface with no known one — `crate::gbuffer::NORMAL_FORMAT`,
// and `normal_format.wesl` is the layout. Never sampled — an integer texture has
// no filtering, and it would be wrong here anyway: the average of two unit
// vectors is not a unit vector, and the average of a wall's normal and the
// ground's behind it points into the seam between them.
//
// **This used to be `outward(stance)`, computed here.** A fragment's facing is
// something the pass that drew it knows — `statics.wesl` resolves a corner's
// two faces per fragment, and the mesh pass has a measured normal per face —
// and the lighting was recovering a rounded copy of it from four bits at the
// far end of a `u16`. `docs/lighting_rebuild.md` phase 2, the same move the
// position plane is.
@group(0) @binding(14) var normal_of: texture_2d<u32>;
// Where each tile's references are, one texel a tile over the rectangle in
// `lighting.grid`: (offset & 255, offset >> 8, offset >> 16, count). The index
// of `docs/lighting.md`'s decision 30 — a count of zero is open ground, and
// nothing else says a tile stands nothing. What the offset points at is the id
// list at binding 8, which is step 23.1. See `crate::occlusion`.
@group(0) @binding(4) var occluders: texture_2d<u32>;
// What each of those tiles *is*, over the same rectangle: (sky, unused, unused,
// unused). Only the sky channel is written today. See `crate::occlusion`, and
// `docs/lighting_world.md` for why this is a second plane rather than four more
// channels of the cell above.
@group(0) @binding(5) var field: texture_2d<u32>;
// And the primitives themselves — one struct each, indexed by a `SolidId`
// outright. `crate::occlusion::Occlusion::primitive_bytes`.
//
// **A storage buffer and not three textures**, `docs/occluders.md`'s D8. What
// stood here was `solids` (opacity and flags), `footprints` (a fraction of a
// tile, a byte an axis) and `solid_z` (sixteen bits an end), three planes
// indexed by one number and each a different encoding of one box — because
// decision 30.5's ceiling was WebGL2, where a list a shader can index has to be
// a texture. Phase 6a settled the ceiling as WebGPU, this file already reads
// eleven storage buffers, and a primitive is a struct.
//
// **Its two corners are absolute world coordinates**, D1: no cell is their base
// and there is no fraction of one anywhere in this pass. A box wider than a
// tile, one standing between two, and one a thousandth of a tile deep are the
// same six numbers.
struct Primitive {
    lo: vec3<f32>,
    // PRESENT | HOLED | edges, in the padding a `vec3<f32>` leaves behind.
    flags: u32,
    hi: vec3<f32>,
    opacity: u32,
};

@group(0) @binding(6) var<storage, read> primitives: array<Primitive>;
// The hole in each of those solids, indexed by the same number:
// (near, far, z_bottom + 128, z_top + 128) — `occlusion::Aperture`. A plane
// beside the list rather than four more channels of it, because the list is what
// the walk reads cell after cell in a loop and a hole is what almost nothing
// has: the `HOLED` bit is what makes this read at all, so an ordinary wall pays
// one bit test and no fetch.
@group(0) @binding(7) var apertures: texture_2d<u32>;
// And the references between the cells and the solids, one texel each and folded
// the same way: (id & 255, id >> 8, id >> 16, unused). A cell counts through a
// run of these and each one names a solid — decision 38's ownership, which is
// what lets one box be referenced by every cell it stands over.
@group(0) @binding(8) var solid_ids: texture_2d<u32>;

// One flame. Three `vec4`s rather than nine fields, because a uniform array's
// stride is rounded up to 16 bytes either way and this is what the CPU writes:
//
//   place = (tile x, tile y, z, radius in tiles)
//   color = (r, g, b, intensity)
//   beam  = (axis x, axis y, axis z, cosine of the half-angle)
//
// A fire standing in the world lights every direction and writes a `beam` of
// `(0, 0, 0, -1)`: no cosine is below -1, so the test below is one comparison
// and the dot product is never reached. `crate::light::Beam`.
struct Light {
    place: vec4<f32>,
    color: vec4<f32>,
    beam: vec4<f32>,
};

// Sized to `Lighting::MAX`, and the two have to agree — this is a uniform
// array, so its length is fixed at compile time and a mismatch is a buffer
// wgpu rejects rather than a wrong picture.
const MAX_LIGHTS: u32 = 64u;

struct Lighting {
    // What a tile with an open column above it is multiplied by away from every
    // flame, then how many of `lights` are real. Scaled per fragment by the sky
    // channel of `field`, which is what makes the inside of a house darker than
    // the street with nothing in either — `crate::light::Ambient`.
    sky: vec4<f32>,
    // And what every tile gets, roof or no roof: the floor under the darkness,
    // so that an unlit room is deep rather than pure black. `(1,1,1,·)` sky with
    // a `(0,0,0,·)` ground over an empty grid is no lighting at all, and the
    // identity below is exact for it.
    ground: vec4<f32>,
    // The occlusion grid's rectangle: its lowest tile `x` and `y`, then its
    // width and height in tiles. A tile outside it occludes nothing, which is
    // right by construction — the grid is grown by the widest pool's reach, so
    // nothing outside it could shadow anything this frame draws.
    grid: vec4<i32>,
    // Which picture to draw: `crate::debug::View` in the first component, the
    // rest padding. Zero is the lit frame and every other value is a diagnostic
    // drawn from the values this pass lit it with — see `docs/lighting.md`,
    // decision 8. The numbers are pinned in a test on the Rust side.
    view: vec4<u32>,
    // Which way the sun is — `x` and `y` in tiles, `z` in tiles as well — and in
    // the fourth component the height above which nothing in this frame's grid
    // can stop it. A ray that has climbed past that is in the sky and stops
    // walking, which is what makes a daylit frame affordable.
    sun: vec4<f32>,
    // Its colour, then how much it adds. An intensity of zero is "no sun", and
    // it is the only thing tested: a night frame never enters the walk.
    sun_color: vec4<f32>,
    lights: array<Light, MAX_LIGHTS>,
};

// `crate::debug::View`, and the same numbers.
const VIEW_LIT: u32 = 0u;
const VIEW_PLACE: u32 = 1u;
const VIEW_KIND: u32 = 2u;
const VIEW_HEIGHT: u32 = 3u;
const VIEW_OCCLUDERS: u32 = 4u;
const VIEW_LIGHT: u32 = 5u;
const VIEW_SHADOW: u32 = 6u;
const VIEW_REACH: u32 = 7u;
const VIEW_SUN: u32 = 8u;
const VIEW_SKY: u32 = 9u;
const VIEW_FLAMES: u32 = 10u;
const VIEW_NORMAL: u32 = 11u;

@group(0) @binding(2) var<uniform> lighting: Lighting;

// The statics and mobiles passes' own `SpriteQuad`, laid out exactly as
// `sprite.rs`'s `SpriteQuad::write` puts it on the GPU — this is that same
// buffer, bound a second time as storage rather than uploaded twice. Only
// `place` is read here today; the rest exists so the struct's size matches
// `SpriteQuad::STRIDE` and a future reader (decision 3's real per-face
// normal, step 4) has the rest already in reach. `docs/gbuffer.md` step 3.
// `twin` (step 4) is what `statics.wgsl` reads to give a corner's two faces
// two different ids — it is never read here, `face_instances[id]` is already
// addressed by whichever of the two ids a fragment's `place` carries, but it
// still has to be declared: this struct's size is what WGSL uses to stride
// through the buffer, and a struct narrower than `SpriteQuad::STRIDE` reads
// every row past the first at the wrong offset.
// `owner` *is* read here, unlike `twin`: which occluder of its own tile the
// static drawn on a pixel is. **It is a key and no longer the comparison** —
// `docs/lighting_rebuild.md` phase 4 compares a `SolidId`, and a sprite instance
// covers a corner's two panels, so `own_solid` narrows this owner by the
// fragment's own stance to name one of them. `crate::sprite::SpriteQuad`.
struct FaceInstance {
    rect: vec4<f32>,
    region: vec4<f32>,
    depth: f32,
    hue: u32,
    place: vec2<u32>,
    twin: u32,
    owner: u32,
};

@group(0) @binding(9) var<storage, read> face_instances: array<FaceInstance>;
@group(0) @binding(10) var<storage, read> mobile_instances: array<FaceInstance>;

// `docs/gbuffer.md` step 4c's mesh-face pass's own row: the tile a face's
// static stands on, which way that face really faces, and **which solid of the
// grid it is the drawn form of** — a mesh face has no picture, so it shares no
// field with `FaceInstance` and lives in its own buffer instead.
// `crate::mesh_face::MeshFaceRow`, laid out identically.
//
// `solid` was an owner until `docs/lighting_rebuild.md` phase 4. A mesh face is
// one primitive by construction, so it can name it outright, which is what lets
// a flight's treads stop shadowing each other without a height being read
// anywhere — see `own_solid` for the sprite side, which cannot.
struct MeshFaceInstance {
    tile: u32,
    stance: u32,
    solid: u32,
};

@group(0) @binding(11) var<storage, read> mesh_instances: array<MeshFaceInstance>;

// The ground pass's own `GroundQuad`, laid out exactly as `ground.rs`'s
// `GroundQuad::write` puts it on the GPU — this is that same buffer, bound a
// second time as storage. `docs/gbuffer.md` step 7, the ground half of what
// step 3 did for a static's tile.
//
// Scalars rather than the `vec4`s `GroundQuad`'s own doc groups them into,
// unlike `FaceInstance` above: `GroundQuad::write`'s first field is a bare
// `(x, y)`, only eight bytes wide, and a `vec4` declared right after it would
// force WGSL to align that field to sixteen bytes — opening a gap the real
// bytes do not have, and reading every field after it four bytes short of
// where the Rust side actually put it. A scalar's alignment is never wider
// than four bytes, so a struct built only from them can never drift from an
// offset `GroundQuad::write` chose. Only `place0` is read; the rest is
// declared so the struct's size matches `GroundQuad::STRIDE`, the same reason
// `FaceInstance`'s unread fields are — a struct narrower than the real stride
// reads every row past the first at the wrong offset. `place1` — the second
// word `crate::place::Place::packed` writes, `z`/`kind`/`stance` — is never
// read: the ground pass's `z` and fraction are this fragment's own, computed
// by `ground.wgsl` and carried in the position plane already, not a fact the
// row holds.
struct GroundInstance {
    origin_x: f32,
    origin_y: f32,
    corner_a: f32,
    corner_b: f32,
    corner_c: f32,
    corner_d: f32,
    region_u: f32,
    region_v: f32,
    region_du: f32,
    region_dv: f32,
    texmap_u: f32,
    texmap_v: f32,
    texmap_du: f32,
    texmap_dv: f32,
    depth: f32,
    // `Place::packed`'s two words: `place0` is `x | y << 16`, and `place1` —
    // never read here — is `(z + 128) | kind << 8 | stance << 16`.
    place0: u32,
    place1: u32,
};

@group(0) @binding(12) var<storage, read> ground_instances: array<GroundInstance>;

// No occluder at all — `occlusion::OwnerId::NONE`, and what the ground, a mobile
// and anything the grid refused stamp. Zero and matching nothing, which is why
// the numbering `Builder::finish` hands out starts at one: two fragments that are
// each a point of nothing are not a point of the same thing.
//
// Still read, by `own_solid` alone: a sprite instance covers a corner's two
// panels, so what a *fragment* of one is a point of is its static's owner
// narrowed by its own stance. Nothing else compares an owner any more.
const OWNER_NONE: u32 = 0u;

// And no *solid* at all: what a fragment carries when it is a point of none —
// the ground, a mobile, a gump, a face whose static this frame's grid refused.
// `occlusion::SolidId::NOBODY`, and the two are one number.
//
// **It cannot collide with a real id.** A reference carries its id in three
// bytes (`id_at`), so every id this pass can read back is at most `0xFF_FFFF`
// and this is strictly greater than all of them. `docs/lighting_rebuild.md`
// phase 4.
const SOLID_NOBODY: u32 = 0xFFFFFFFFu;

// The stance rides in the id word above the kind — `ids_stance`, and
// `statics.wgsl` writes it. **It is no longer read here for a normal**:
// `normal_of` carries the vector itself, and what is left of the stance for
// this pass is two things. Which instance buffer an id addresses, since a mesh
// face's row lives in its own (`STANCE_MESH_FACE`); and `own_side`, the shadow
// walk's own-run test, which asks which *edge of its tile* a fragment's surface
// stands on rather than which way it looks. `docs/lighting_rebuild.md` phase 4
// retires the second. `STANCE_FLAT` is no longer imported at all, and that is
// the measure of what the normal plane took away: the one flat value this pass
// used to read was for a facing.

// How much of a flame at `toward` reaches a surface facing `normal`:
// `max(N · L, 0)`, and nothing else.
//
// Textbook Lambert, with no wrap, no band and no width knob — `light::lit_from`'s
// own doc is the argument, and `docs/lighting_rebuild.md` is the decision.
//
// **The one thing a fraction cannot say.** A wall's two faces are one tile and
// one plane — the same tile, the same fraction, the same height — so without a
// facing a torch inside a room lights the outside of the house exactly as
// brightly as the inside. That was reported from the client as a wall behaving
// like glass, and it is what this fixes.
//
// `toward` is the fragment-to-flame offset in tiles and **it is normalised
// here**: fed unnormalised, the same expression answered about a distance along
// the normal rather than about an angle. `docs/lighting_rebuild.md` phase 3.
//
// `light::lit_from`, and the two are one formula.
fn lit_from(normal: vec3<f32>, toward: vec3<f32>) -> f32 {
    let span = length(toward);
    // A fragment standing exactly on the flame has no direction to be lit from,
    // and full is the answer that does not put a black dot in a pool's middle.
    if span <= 0.0 {
        return 1.0;
    }
    return clamp(dot(normal, toward / span), 0.0, 1.0);
}

// How many cells of the grid one ray may look at.
//
// One number for both rays, because they are one walk. A pool reaches nine tiles
// at the widest (`light::CAMPFIRE`) and a sunbeam's segment runs `MAX_SUN_TILES`,
// and `walk` visits every cell the segment crosses rather than a fixed number of
// samples — which on a diagonal is both axes' worth, so this is twice the longer
// of the two and a little. It is not meant to be reached: a fragment outside a
// pool never walks at all, and a sunbeam ends where it leaves the grid's ceiling.
// It exists so that a loop over data cannot be made unbounded by a radius
// somebody widens later. `light::MAX_WALK_STEPS`, and the two are one number.
const MAX_WALK_STEPS: i32 = 72;

// **`FLAME_SPREAD`, `SOFT_CROSSING_MIN`, `SOFT_CROSSING_MAX` and `FLAME_DEPTH`
// stood here** and went at `docs/lighting_rebuild.md` phase 5, with
// `light.rs`'s own four. They were the analytic penumbra: the flame's size in
// tiles, times `t / (1 - t)` for how far along the ray the occluder stands,
// bounded at both ends, converted into `z` by a flame's height. One ray
// reporting what eight would have found — the similar-triangles answer, which
// costs one division rather than a second ray, and which is a *picture of* a
// penumbra rather than a flame with a size.
//
// Eight rays are cast now and each of them answers about itself. See
// `light::FLAME_RADIUS`.

// How big a flame is: the radius of the sphere `shadow` samples, in tiles.
//
// **On the wire and no longer a constant** — `lighting.ground.w`, which is
// `light::Lighting::flame_radius`. A frame draws `light::FLAME_RADIUS`, an
// eighth of a tile, and the art is what says so: a torch's drawn flame is eight
// or ten screen pixels tall, four pixels to a `z`, so two and a half `z` of a
// tile's eleven. What the field buys is a *comparison* that can ask for zero —
// eight rays at a sphere are an estimate, and a gate laid against a path tracer
// otherwise reports that estimate's noise as a disagreement. See that field.
//
// A channel of the header that held a stated zero until a reader existed for it,
// which is this file's own rule for claiming one.
fn flame_radius() -> f32 {
    return lighting.ground.w;
}

// How many rays a fragment casts at each flame. `light::SHADOW_RAYS`.
const SHADOW_RAYS: u32 = 8u;

// Turns of the sample spiral between one point of a flame and the next: the
// golden angle, `π(3 − √5)`, which is what makes a Vogel spiral of any length
// evenly spread rather than spoked. `light::GOLDEN_ANGLE`, written out on both
// sides so the two are the same bits.
const GOLDEN_ANGLE: f32 = 2.3999632;
// And a whole turn, for the per-fragment rotation. `std::f32::consts::TAU`.
const TAU: f32 = 6.2831855;

// The four sides of a tile, in the low bits of a cell's fourth channel, and the
// `PRESENT` bit above them. `crate::occlusion` writes them; the numbers are
// pinned there and nothing but a person reading both files can check they agree.
//
// A mask of zero is a **lid** — a floor, a roof — whose occlusion is entirely its
// `z` span and which no vertical side describes. All four is "it stands up and
// the art would not say which way", which is the whole-tile occluder decision 3
// started with and the answer everything falls back to.
const EDGE_NORTH: u32 = 1u;
const EDGE_EAST: u32 = 2u;
const EDGE_SOUTH: u32 = 4u;
const EDGE_WEST: u32 = 8u;
const EDGE_MASK: u32 = 15u;
// And the bit that says this surface has a hole in it, so that its texel in the
// aperture plane means something. `occlusion::HOLED`.
const HOLED: u32 = 64u;
// How many parts of a tile one step of a hole's run coordinate is worth.
// `occlusion::RUN_STEPS`, and the two are one number: both sides read the same
// byte and divide it by this, so the two implementations agree exactly rather
// than to a tolerance.
const RUN_STEPS: f32 = 255.0;

// How far past an exact tangent `ray_vs_solid`'s own `entered > leaves`
// rejection still counts as a hit — a cross-implementation tolerance, and
// it lives only on this side. `light::ray_vs_solid`'s own doc comment has
// the story: `docs/lighting_raymarch.md`'s point 4 cutover found a ray
// grazing a box's corner where this shader's own `f32` division rounds to
// `entered > leaves` (a miss) on inputs `light::sample` — with the exact,
// untolerant formula — rounds to `entered <= leaves` (a hit) for. Widening
// the CPU copy to match broke a different equivalence instead of fixing
// this one; widening only here, to agree with what the CPU side already
// computes unprompted, is what `light::ray_vs_solid`'s own doc comment
// argues is the narrower, correct fix.
//
// **It is a rounding-scale number and it used to be `1.0e-2`**, which is not
// a rounding scale at all: `entered` and `leaves` are fractions of the whole
// segment, so a hundredth of one is a hundredth of the way from a fragment to
// its flame — around a screen pixel of world at this crate's own top zoom, and
// every box in the frame silently a pixel fatter wherever a ray grazed it. It
// cost 299 of the 302 disagreements `examples/boxes.rs`'s ground oracle
// reported on the `tree` scene and 27 of its face oracle's 43, all of them the
// shader alone against a `light::sample` that agreed with the independent
// geometry. And it never bought what it was introduced for:
// `the_shader_and_light_sample_agree_about_a_carried_beam` and
// `the_shader_lights_a_frame_as_light_sample_does` fail on the same pixel with
// the same two numbers at `1.0e-2`, at `1.0e-6` and at `0.0` — measured, not
// argued. What is left here is the guard the comment above describes and
// nothing more: two `f32` implementations of one formula, one rounding apart.
const RAY_TANGENT_TOLERANCE: f32 = 1.0e-6;

// **`pierces` stood here** and went at `docs/lighting_rebuild.md` phase 5, with
// `light::pierces`. It was a panel's own span softened over a band `tall` `z`
// units wide, centred on the top edge and hanging below the bottom one — the one
// asymmetry it had, and not a cosmetic one: a wall is based at the height of the
// ground it stands on, and the ray a person actually looks at (a torch and a
// floor, both at `z = 0`) runs exactly along that bottom edge, so a band centred
// there too would have let every wall in the frame pass half its light along the
// ground.
//
// Its *hard* half was already answered before it was ever called: a caller
// reaches it only with a `ray_vs_solid` hit against the very box whose `z` bounds
// it was handed, at that crossing's own midpoint. So what it added was the band,
// and the band is eight rays now.

// Whether a **lid** is in the way of a ray that runs from `from` to `to` in `z`
// across one cell: 1 where the ray went through the plane and out the other side,
// 0 where it stayed on one side of it.
//
// **A lid is a plane and not a slab**, which is why this is not the length rule
// beside it. A floor is `height 0` in `tiledata.mul` — 4,534 of the 4,647 lids
// over the block of Britain `artscan`'s `column` example reads — so its span is
// zero deep, and scaling what it stops by how far the ray ran *inside* that span
// gets zero out of every floor in the world. That is what lit the storey above a
// torch through its own floorboards; see `scene::storey_over_a_torch`.
//
// The crossing test is **strict**: a ray running exactly along the top of a lid —
// a candle standing on the floor it lights — has not gone through anything, and
// counting a touch would lay half a floor's shadow across every room lit from
// inside it.
//
// **It answers one ray, and one ray has no penumbra** — phase 5. `source` and
// `spread` were the flame's own `z` and size, and what they bought was the share
// of the source left on the lit side of the plane: an analytic soft shadow, cast
// by a single ray on behalf of the eight that are cast now.
//
// `light::crosses`, and the two are one formula.
fn crosses(entering: f32, leaving: f32, low: f32, high: f32) -> f32 {
    let under = min(entering, leaving);
    let over = max(entering, leaving);
    return select(1.0, 0.0, under >= high || over <= low);
}

// **`STAND_OFF` and `ON_TOP` lived here**, and `docs/lighting_rebuild.md` phase 4
// deleted them: **the bias is zero.** They were two steps of the seven-bit
// fraction the retired `place` attachment carried and well under the one `z` unit
// it quantised a height to — numbers off a byte layout, standing in for a ray
// that did not start inside the surface it was drawn on. Identity does that
// exactly now, and phase 2's exact position plus a per-solid `ray_vs_solid`
// removed the second thing they bought. `light.rs`'s own note where the pair
// stood has the whole argument.

// Whether a lit point lies **on** a surface: its `z` is inside the span that
// surface occupies, its two edges included, and **exactly** — the `ON_TOP`
// tolerance was the nudge `walk` gave the point, handed back here, and both went
// at phase 4. What the exemptions are asked, one surface at a time — see `walk`,
// and `light::on_surface`.
//
// `low`/`high` are the primitive's own `lo.z`/`hi.z`: this is a question about
// whether a fragment is a point *of* the solid, and half a unit of rounding is
// the difference between a face's bottom row belonging to the box it is drawn on
// and standing under it.
fn on_surface(z: f32, low: f32, high: f32) -> bool {
    return z >= low && z <= high;
}

// **`drawn_on` lived here**, and `docs/lighting_rebuild.md` phase 4 retired it
// along with its twin — see `light.rs`'s own note where the Rust one stood. It
// asked whether a lid was a plane the fragment was drawn at the height of, which
// is how one static's several lids were told apart when identity was per static.
// A fragment names its own solid now, and `walk`'s `raw_start.z` is no longer
// carried through two functions to be compared against a span.

// **`inside` stood here** and went at phase 5 with `light::inside`. It was
// `pierces` with the asymmetry taken out — a soft interval, a band wide at each
// edge — and its one caller was `hole`, because a window's edges are in the
// middle of a surface and no ray runs along them by construction, so they
// softened both ways where a wall's own top softened one way. Both bands are
// eight rays now.

// Where along a panel's own run a point of it lies, 0 to 1 across the tile.
//
// A panel on a north or south side lies in a plane of constant `y`, so what runs
// along it is `x`; an east or west one is the other way round. That is the whole
// of a surface's own coordinate system, and it is why only a *named* panel may
// have a hole: a lid and a body have no run to measure one along. `light::run_v`.
fn run_v(edges: u32, px: f32, py: f32) -> f32 {
    let along = select(py, px, (edges & (EDGE_NORTH | EDGE_SOUTH)) != 0u);
    return along - floor(along);
}

// Whether a surface is **missing** where a ray goes through it: 1 inside the
// hole, 0 outside it.
//
// Both spans have to hold, which is what an opening in a wall is. They met with
// `min` of two soft intervals until phase 5 — deliberately `min` and not a
// product, so that a point halfway into the hole across *and* halfway up read as
// one corner's own softening rather than as a quarter of a hole. With no band on
// either span the `min` is a conjunction and says so. `light::hole`.
fn hole(texel: vec4<u32>, v: f32, z: f32) -> f32 {
    let across = v >= f32(texel.x) / RUN_STEPS && v <= f32(texel.y) / RUN_STEPS;
    let up = z >= f32(texel.z) - 128.0 && z <= f32(texel.w) - 128.0;
    return select(0.0, 1.0, across && up);
}

// How much of a surface stands in the way at the point a ray goes through its
// plane: all of it, less the hole in it.
//
// The whole of `docs/lighting.md`'s step 21.3, and the reason it is this small is
// decision 30.7 — a panel was already *pierced at a point* rather than travelled
// through, so the point was already being computed and a window is what that
// point is asked about. `id` is the solid's own place in the list, which is
// where its hole is; `cross` is where the ray goes through the plane, in all
// three, because the height alone cannot say whether the ray went through the
// window or through the wall beside it.
//
// **The span itself is not asked about any more** — phase 5, and the `pierces`
// call that stood here. A caller reaches this only with a `ray_vs_solid` hit
// against the box `low`/`high` were the `z` bounds of, at that crossing's own
// midpoint, so `cross.z` was inside the span by construction and the band was the
// whole of what the call added. `light::pierced`.
fn pierced(stands: Primitive, id: u32, cross: vec3<f32>) -> f32 {
    if (stands.flags & HOLED) == 0u {
        return 1.0;
    }
    let v = run_v(stands.flags & EDGE_MASK, cross.x, cross.y);
    return 1.0 - hole(aperture_at(id), v, cross.z);
}

// Which of a cell's sides are **the same wall the lit end is part of**, and
// therefore must not shadow it.
//
// A wall's face lies *on* the panel it is the face of — that is what decision 16
// is about — so a pixel of a wall is a point in the plane of its own tile's
// panel, and the panels of the tiles either side of it are in that same plane.
// Every ray leaving such a pixel along the wall therefore *grazes* the
// neighbours' panels, and whether it counts as crossing one is decided by the
// last bits of a float. It showed as a thin dark stroke down the wall at every
// tile seam, one per corner, on any wall lit by a lamp standing near it —
// the same corner ambiguity that used to draw bright spokes, arriving with the
// sign reversed.
//
// A run of wall is one surface, and no part of a surface shadows another part of
// it. So a panel on the same side of its tile as the lit end's own, on the same
// *line* — the same row for a north or south face, the same column for an east
// or west one — is not an occluder for this ray. Anything else about that cell
// still is: a wall tile that also carries the perpendicular face of a corner
// stops the ray on that face as it always did.
//
// Which side of its own tile a stance is the face of, as an edge bit. Nothing
// for a flat or faceless pixel: neither is part of a run of wall.
//
// `light::Surface::face`, put through `occlusion::edges_of`.
fn own_side(stance: u32) -> u32 {
    switch stance {
        case STANCE_FACE_NORTH: { return EDGE_NORTH; }
        case STANCE_FACE_EAST: { return EDGE_EAST; }
        case STANCE_FACE_SOUTH: { return EDGE_SOUTH; }
        case STANCE_FACE_WEST: { return EDGE_WEST; }
        default: { return 0u; }
    }
}

// Which axis the plane a stance lies in runs across (0 `x`, 1 `y`, 2 `z`), and
// which end of a box that plane is — `.y` of `1.0` for the box's `hi` corner,
// `-1.0` for its `lo`. `.x` is `-1.0` for a stance that has no plane at all,
// which is `STANCE_UPRIGHT`: a tree, a billboard, a static whose art named no
// side. `light::lit_plane`, and the same absence.
fn lit_plane(stance: u32) -> vec2<f32> {
    switch stance {
        // A lid looks up, and its box is flat in `z`: both corners are the plane.
        case STANCE_FLAT: { return vec2<f32>(2.0, 1.0); }
        case STANCE_FACE_NORTH: { return vec2<f32>(1.0, -1.0); }
        case STANCE_FACE_SOUTH: { return vec2<f32>(1.0, 1.0); }
        case STANCE_FACE_WEST: { return vec2<f32>(0.0, -1.0); }
        case STANCE_FACE_EAST: { return vec2<f32>(0.0, 1.0); }
        default: { return vec2<f32>(-1.0, 0.0); }
    }
}

// `on_the_lit_surface` is the other half of this and lives below `solid_at`,
// which it reads.

// The cell a walk actually starts in: the tile its caller carries, unless the
// start point is **strictly outside** that tile. `light::starting_cell`, and the
// two are one rule — see it for what the carried tile is for, why a point past
// the edge is not the tie it settles, and the 324 leaked pixels that named this.
// No tolerance: the comparison is against the tile's own two planes and a point
// exactly on one is inside.
fn starting_cell(at: vec2<f32>, tile: vec2<f32>) -> vec2<i32> {
    let x = select(tile.x, floor(at.x), at.x < tile.x || at.x > tile.x + 1.0);
    let y = select(tile.y, floor(at.y), at.y < tile.y || at.y > tile.y + 1.0);
    return vec2<i32>(i32(x), i32(y));
}

// `light::same_run`, and see it for why `docs/lighting_rebuild.md` phase 4 does
// *not* delete this: a run of wall is N different statics cut on tile boundaries,
// so the panel next along the run is a different solid however exactly a fragment
// names its own. The height gate rides inside it now, where the Rust twin has it.
fn same_run(own: u32, cell: vec2<i32>, first: vec2<i32>, spot_z: f32, low: f32, high: f32) -> u32 {
    if !on_surface(spot_z, low, high) {
        return 0u;
    }
    var line = 0u;
    if cell.y == first.y {
        line = line | EDGE_NORTH | EDGE_SOUTH;
    }
    if cell.x == first.x {
        line = line | EDGE_EAST | EDGE_WEST;
    }
    return own & line;
}

// How wide the lists are as textures, in texels. `occlusion::LIST_ROW`, and the
// two are one number — a list is one dimensional and a texture is not.
const LIST_ROW: u32 = 1024u;

// Where one tile's references are: `(offset, count)`, and `(0, 0)` for open
// ground and for anything outside the grid.
//
// The three-channel offset is `crate::occlusion::Occlusion::bytes`: one byte
// holds 255 references and a city block holds thousands, so it is spread over
// the three channels the count does not need.
fn solids_at(x: i32, y: i32) -> vec2<u32> {
    let cell = vec2<i32>(x - lighting.grid.x, y - lighting.grid.y);
    if cell.x < 0 || cell.y < 0 || cell.x >= lighting.grid.z || cell.y >= lighting.grid.w {
        return vec2<u32>(0u, 0u);
    }
    let span = textureLoad(occluders, cell, 0);
    return vec2<u32>(span.x | (span.y << 8u) | (span.z << 16u), span.w);
}

// Which solid a cell's `n`th reference names, and **which occluder of that cell
// it is** — `occlusion::SolidId` spread over three channels exactly as the
// index's offset is, and `occlusion::OwnerId` in the fourth.
//
// The one fetch step 23.1 added to the hot loop, and it is a fetch rather than
// arithmetic because a reference is a fact about the world's geometry: the run a
// cell holds is contiguous, what it points at need not be. The owner rides in the
// same texel for the same reason — it is a fact about the *reference*, and the
// loop is already reading this word.
fn id_at(reference: u32) -> vec2<u32> {
    let at = vec2<i32>(i32(reference % LIST_ROW), i32(reference / LIST_ROW));
    let id = textureLoad(solid_ids, at, 0);
    return vec2<u32>(id.x | (id.y << 8u) | (id.z << 16u), id.w);
}

// One primitive of the list: its own two corners, its flags and its opacity.
//
// An array index and no folding at all, where this used to be a `textureLoad`
// into a row `LIST_ROW` wide — the whole of what `docs/occluders.md`'s D8 buys
// on the read side. **The box is here and nowhere else**: there is no second
// plane to fetch for a height and none for a footprint, so a solid's geometry
// arrives in one read and cannot be half-read.
fn solid_at(id: u32) -> Primitive {
    return primitives[id];
}

// And its hole: (near, far, z_bottom + 128, z_top + 128). The same id and the
// same folding, which is what makes the two planes one list — read only where
// the solid's `HOLED` bit says there is one.
fn aperture_at(id: u32) -> vec4<u32> {
    let at = vec2<i32>(i32(id % LIST_ROW), i32(id / LIST_ROW));
    return textureLoad(apertures, at, 0);
}

// **Which solid of the grid a *sprite* fragment is a point of**, or
// `SOLID_NOBODY` where the grid holds none.
//
// `docs/lighting_rebuild.md` phase 4, and the half of it a mesh face does not
// need: `mesh_instances[id].solid` names one primitive outright, because the pass
// that draws a tread draws exactly one. A sprite instance is not one primitive —
// a corner static is two panels and one picture, and which of the two a fragment
// belongs to is decided *per fragment*, by the stance `statics.wgsl` resolved the
// corner into. So the id has to be found here rather than carried.
//
// It is exact rather than a guess, and the reason is a property of
// `Builder::add`: a static that is not a fitted climbable pushes **either** one
// lid, **or** one body, **or** one panel per named side — never two of a kind.
// So `(owner, shape)` names exactly one solid of the cell, and the fragment's own
// stance is what says which shape:
//
//   - a fragment with a **side** is a point of the panel standing on that side;
//   - a fragment with none is a point of the lid (`edges == 0`) or of the body
//     (`edges == EDGE_MASK`), and a static has only one of those two.
//
// Once a fragment, not once a candidate solid, which is the other half of why it
// is a function: the walk's own rule is now a single comparison of two ids, and
// everything that used to be asked per solid per light — a height inside a span,
// which plane the fragment was drawn at, `edges & own` — is asked here once.
//
// **A fitted climbable is the one case this cannot answer**, and it is the case
// that does not ask: its six solids are lids and panels of one owner, so `own`
// alone is ambiguous — and every pixel of it is drawn by the mesh pass over the
// sprite, which carries its id. Where the mesh does not quite cover the sprite
// (`WIDTH_OVERLAP`'s border, phase 6's), this answers with some tread of the
// right shape rather than the right one. That is a hairline on a static whose own
// art overhangs its volume, and phase 6 removes the border rather than this the
// ambiguity.
fn own_solid(tile: vec2<i32>, owner: u32, own: u32) -> u32 {
    if owner == OWNER_NONE {
        return SOLID_NOBODY;
    }
    let span = solids_at(tile.x, tile.y);
    for (var k = 0u; k < span.y; k = k + 1u) {
        let reference = id_at(span.x + k);
        if reference.y != owner {
            continue;
        }
        let edges = solid_at(reference.x).flags & EDGE_MASK;
        if own != 0u {
            if (edges & own) != 0u {
                return reference.x;
            }
            continue;
        }
        if edges == 0u || edges == EDGE_MASK {
            return reference.x;
        }
    }
    return SOLID_NOBODY;
}

// **`docs/occluders.md`'s D2**: whether a candidate box is part of the very
// surface the lit fragment is a point of, and so must not shadow it.
//
// `light::on_the_lit_surface` states the theorem at length and this is its twin,
// one rule in two spellings with no compiler between them. In short: a candidate
// whose extent along the fragment's own normal axis *ends* on the fragment's own
// plane lies wholly in the closed half-space behind that plane, so a ray leaving
// the surface has nothing there to cross — the discarded set is empty of real
// crossings, which is what makes this a theorem and not a bias.
//
// The plane comes from the box the fragment is a point of (`mine`) and not from
// its interpolated position: the two are the same number — measured, 39,930
// fragments, `traced.rs`'s `a_face_fragments_own_plane_is_the_primitives_own_
// number` — and reading it from the list puts both sides of the comparison in one
// precision, so the equality is exact by construction.
//
// `delta` is the ray, and it is here because the theorem's precondition is
// `d·N >= 0`: the ray has to be *leaving* the plane. A flame behind the surface
// sends the ray into the half-space the candidate really occupies, and the
// reference tracer reads this walk's answer with no cosine to hide it — 4,017
// interior pixels of `line_scene`, the hour this rule went in without the sign
// test. `light::on_the_lit_surface` has the measurement.
fn on_the_lit_surface(stance: u32, mine: u32, lo: vec3<f32>, hi: vec3<f32>, delta: vec3<f32>) -> bool {
    if mine == SOLID_NOBODY {
        return false;
    }
    let plane = lit_plane(stance);
    if plane.x < 0.0 {
        return false;
    }
    let axis = u32(plane.x);
    let own = solid_at(mine);
    // `>= 0` and not `> 0`: a ray lying in the plane is the graze this exists for.
    if delta[axis] * plane.y < 0.0 {
        return false;
    }
    if plane.y > 0.0 {
        return hi[axis] == own.hi[axis];
    }
    return lo[axis] == own.lo[axis];
}

// One tile's surfaces as one box: the union of the spans, the largest opacity
// and the union of the sides, with `edges` of zero for a tile that stands
// nothing.
//
// `Occlusion::at`'s merged view, and for the same readers — a picture of the
// grid rather than a step of the walk. The walk stopped asking it when the list
// arrived, so nothing in the hot loop pays for this.
//
// The span is the primitive's own `lo.z`/`hi.z` and not a rounded copy of it.
// Its one reader is a *diagnostic* picture, which is exactly where phase 1 was
// bitten: `plan.rs`'s elevation view kept its own copy of a format, rounded the
// height, and drew the staircase the plan was about into the instrument meant to
// show it.
struct MergedCell {
    low: f32,
    high: f32,
    opacity: u32,
    edges: u32,
};

fn merged_at(x: i32, y: i32) -> MergedCell {
    let run = solids_at(x, y);
    if run.y == 0u {
        return MergedCell(0.0, 0.0, 0u, 0u);
    }
    let first = solid_at(id_at(run.x).x);
    var merged = MergedCell(first.lo.z, first.hi.z, first.opacity, first.flags);
    for (var i = 1u; i < run.y; i = i + 1u) {
        let stands = solid_at(id_at(run.x + i).x);
        merged = MergedCell(
            min(merged.low, stands.lo.z),
            max(merged.high, stands.hi.z),
            max(merged.opacity, stands.opacity),
            merged.edges | stands.flags,
        );
    }
    return merged;
}

// How much of the sky one tile can see, `0..=1`.
//
// Open sky outside the rectangle, which is `Occlusion::sky_at`'s own answer and
// the honest one in the direction that matters: the grid is grown by the widest
// pool's reach, so a tile outside it is one the frame does not draw, and
// answering "dark" there would put a band of night around every frame.
fn sky_at(x: i32, y: i32) -> f32 {
    let cell = vec2<i32>(x - lighting.grid.x, y - lighting.grid.y);
    if cell.x < 0 || cell.y < 0 || cell.x >= lighting.grid.z || cell.y >= lighting.grid.w {
        return 1.0;
    }
    return f32(textureLoad(field, cell, 0).x) / 255.0;
}

// How much of a ray survives a segment of the world: 1 for nothing in the way,
// 0 for a wall.
//
// **One walk for both the flame and the sun, and now there is nothing left to
// tell them apart but their ends.** The `spread` that stood here was how big the
// source is, and it drew that source's penumbra analytically; a sunbeam passed
// `0` and got the hard edge a point casts. Phase 5 casts eight rays at a flame's
// own sphere instead, so a flame's rays are a sun's rays and the size is in where
// they end. The `skip_last` beside it — the flame's own tile, excused because a
// sconce stands *on* a wall — went at phase 4, which moves such a flame clear of
// that wall (`light::mounted_at`) rather than excusing it.
//
// They were two functions until the sun's was measured. The sun's sampled one
// point per tile — the arrangement this walk replaced for flames — and a sealed
// roofed room therefore had a full-strength column of sunlight down the inside of
// its sunward wall: at noon a ray climbs 11 `z` a tile, so it crossed the wall's
// plane at 16 and was looked at, one tile later, at 22. It stepped over the top of
// a wall it had gone through. Two implementations of one idea, one of them a
// generation behind, is what that failure is made of; hence one.
//
// A grid traversal of the cells the segment crosses — every one, in order, with
// the length of each crossing. Not a fixed number of samples along the ray: at
// two tiles apart that was one interior point, so whether a fragment was in
// shadow was decided at the resolution of a tile and every shadow's edge was a
// staircase.
//
// The starting cell is always left out: the tile being lit must not shadow
// itself, which is what keeps a wall's own face the brightest thing near a torch.
//
// **A cell in between stops the ray only where the ray crosses the side the
// thing actually stands on.** That is decision 3 revised: an occluder used to be
// a whole tile, because nothing said which edge a wall was built on, and a lamp
// mounted on a house was therefore shadowed by the next tile of its own wall —
// the light could not run *along* the street it hung over. The walk already
// knows which boundary it crossed to enter a cell and which it will cross to
// leave, so the test costs two bit operations and no extra sampling.
//
// The height is interpolated along the ray, so a cell only stops the light where
// the ray actually passes through the span it occupies — and it is the *share*
// of the crossing that is inside that span which counts, so a ray grazing the
// top of a wall is dimmed rather than switched. That is what keeps a cellar's
// wall out of the street above it, without a step where the two meet.

// **A box is no longer built here at all.** `box_of` stood in this place and
// reconstructed one from `(cell, bottom, top, footprint)` — a tile plus a
// fraction of it across and sixteen bits an end up — because that is all the
// wire carried. `docs/occluders.md`'s D1 took the reconstruction away with the
// quantisation: `solid_at(id).lo`/`.hi` are the primitive's own corners in
// absolute world coordinates, so what used to be a function is now two fields.
//
// Whether a point stands over a solid's own horizontal footprint —
// `light::over_footprint`, and `ray_vs_solid`'s parallel-axis branch on the two
// horizontal axes alone, both ends inclusive as they are there.
//
// Used by the no-horizontal-run shortcut in `walk`, which cannot call
// `ray_vs_solid` outright: the two answer the *height* half differently, and a
// vertical ray's height answer is `crosses`'s. `ray_vs_solid`'s slab is
// inclusive at both ends, so a ray that starts exactly on a lid's own plane has
// hit it; `crosses` is strict, and a candle standing on the floor it lights has
// gone through nothing. That was a softness until phase 5 and is a strictness
// now, and the split it forces is the same one. The Rust twin's doc comment
// states it too.
fn over_footprint(at: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> bool {
    return at.x >= lo.x && at.x <= hi.x && at.y >= lo.y && at.y <= hi.y;
}

// Where a straight segment enters and leaves an axis-aligned box, as a
// fraction of the whole segment (`0.0` is `from`, `1.0` is `to`) — the slab
// method, continuous throughout, no notion of "which tile" anywhere in it.
// `light::ray_vs_solid`, and the two must answer the same segment identically.
//
// `hit` is `false` when the segment misses the box on at least one axis
// outright, including a segment parallel to a flat axis that does not lie
// exactly in the box's own plane on it. `entered`/`leaves` are already
// clamped to `0.0..=1.0` — the box's own extent past either end of the
// segment does not count. Unrolled over `x`/`y`/`z` rather than a loop
// indexing a `vec3`, matching this file's own idiom everywhere else a walk
// has needed one axis at a time.
struct RayBox {
    hit: bool,
    entered: f32,
    leaves: f32,
};

// A flame is a point source, and a point source's shadow is exact — no
// widened corner test, no taper: an intersection here already means the
// segment genuinely passed through the box. See `docs/lighting_raymarch.md`'s
// "hard shadows" decision; `light::ray_vs_solid` is the whole test now, for
// every edge kind, and this is its mirror.
fn ray_vs_solid(ray_from: vec3<f32>, ray_to: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> RayBox {
    var entered = 0.0;
    var leaves = 1.0;
    let delta = ray_to - ray_from;
    if abs(delta.x) <= 1.0e-9 {
        if ray_from.x < lo.x || ray_from.x > hi.x {
            return RayBox(false, 0.0, 0.0);
        }
    } else {
        let t1 = (lo.x - ray_from.x) / delta.x;
        let t2 = (hi.x - ray_from.x) / delta.x;
        entered = max(entered, min(t1, t2));
        leaves = min(leaves, max(t1, t2));
        // `RAY_TANGENT_TOLERANCE`, not a bare `entered > leaves`: its own
        // comment has the reason — a ray grazing a box's corner can close one
        // axis's interval one rounding of `t` before the other opens, and CPU
        // and GPU are not guaranteed to agree which side of that razor-thin
        // gap is real for the identical segment.
        if entered > leaves + RAY_TANGENT_TOLERANCE {
            return RayBox(false, 0.0, 0.0);
        }
    }
    if abs(delta.y) <= 1.0e-9 {
        if ray_from.y < lo.y || ray_from.y > hi.y {
            return RayBox(false, 0.0, 0.0);
        }
    } else {
        let t1 = (lo.y - ray_from.y) / delta.y;
        let t2 = (hi.y - ray_from.y) / delta.y;
        entered = max(entered, min(t1, t2));
        leaves = min(leaves, max(t1, t2));
        if entered > leaves + RAY_TANGENT_TOLERANCE {
            return RayBox(false, 0.0, 0.0);
        }
    }
    if abs(delta.z) <= 1.0e-9 {
        if ray_from.z < lo.z || ray_from.z > hi.z {
            return RayBox(false, 0.0, 0.0);
        }
    } else {
        let t1 = (lo.z - ray_from.z) / delta.z;
        let t2 = (hi.z - ray_from.z) / delta.z;
        entered = max(entered, min(t1, t2));
        leaves = min(leaves, max(t1, t2));
        if entered > leaves + RAY_TANGENT_TOLERANCE {
            return RayBox(false, 0.0, 0.0);
        }
    }
    return RayBox(true, entered, max(entered, leaves));
}

// How much one candidate cell stops `walk`'s ray — every solid on it tested
// with its own exact `ray_vs_solid` interval, `light::walk_cells_streaming`'s
// own `apply` closure restated as a function, since WGSL has none.
//
// Split out of `walk`'s own loop so it can be called a second time, for the
// cell its own unconditional tie-break probe tests without moving the
// walk's own trajectory there — see `walk`'s own comment at the call sites.
//
// **The walk's own cell and its probe share one tolerance, and that took a
// measurement to be allowed.** This used to take the tolerance as a parameter
// — `RAY_TANGENT_TOLERANCE` here, a hard `0.0` at the probe — because sharing
// the *old* `1.0e-2` broke four previously-green parity tests
// (`_about_a_wall_that_faces_away`, `_about_which_side_a_wall_is_on`,
// `_about_a_hole_in_a_wall`, `_at_the_corner_of_a_house`) once the probe went
// unconditional: a probed cell is a diagonal neighbour by construction, which
// is exactly the geometry a ray is likeliest to graze rather than cross, so a
// tolerance a hundredth of a segment wide rescued spurious near-tangents on
// cells the ray never approached. At a rounding-scale tolerance there is
// nothing spurious left to rescue — the whole parity suite and all three of
// `examples/boxes.rs`'s oracles read the same with it shared as with the probe
// exact — so the parameter is gone and both calls are the one function.
// `light::walk_cells_streaming`'s own probe has always been exact both places.
fn cell_stopped(
    cell: vec2<i32>,
    first: vec2<i32>,
    own: u32,
    // Which solid of the grid the lit end is a point of, or `SOLID_NOBODY` for a
    // point of none. `light::LitEnd`'s `solid`, and the whole of the self-shadow
    // rule.
    mine: u32,
    stance: u32,
    start: vec3<f32>,
    finish: vec3<f32>,
    delta: vec3<f32>,
    ground: f32,
) -> f32 {
    let span = solids_at(cell.x, cell.y);
    let lit = start;
    // **The tile-wide union of sides used to be gathered here**, in a loop of
    // its own over every solid of the ray's own cell, and `light::Surface::
    // shadowed_by_own_tile` turned it into a mask the exemption below was anded
    // with. `docs/lighting_height.md` phase 3 retired both: that mask was zero
    // for every surface that reached the test (the arm also required the stance
    // *not* to be flat, and the mask was only ever non-zero for a flat one), and
    // what a fragment is a point of is now a number it carries rather than
    // anything derived from its tile. One fetch loop a cell less, besides.

    // What the *cell* stops is the **largest** of what its surfaces stop,
    // and not the product: two panels on one tile are the two faces of one
    // corner, and a ray crossing both has gone through one thing once.
    var stopped = 0.0;
    for (var k = 0u; k < span.y; k = k + 1u) {
        let reference = id_at(span.x + k);
        let id = reference.x;
        let stands = solid_at(id);
        let edges = stands.flags & EDGE_MASK;
        // The solid's own `z` span — the primitive's two corners, which is
        // where every height in this pass comes from. `light::wire_span`.
        let low = stands.lo.z;
        let high = stands.hi.z;
        // And the exact interval this ray crosses the primitive's own box in —
        // `light::ray_vs_solid`, replacing the DDA's own per-*cell*
        // `entered`/`leaves` with a per-*solid* one. The box is read, not
        // rebuilt: `docs/occluders.md`'s D1.
        let hit = ray_vs_solid(start, finish, stands.lo, stands.hi);
        if !hit.hit {
            continue;
        }
        // **A surface does not shadow itself, and that is one comparison of two
        // names** — `docs/lighting_rebuild.md` phase 4, `light.rs`'s own inline
        // copy of it in both walks. Everything that used to stand in for it here
        // is gone: the owner, the `own_cell` gate an owner needed to be
        // meaningful, the `match` on `edges` that asked each shape the one
        // question the wire could answer about it, and `flame_end` — the far end's
        // own height test, which `mounted_at` made unnecessary. `SOLID_NOBODY`
        // matches no reference, so a fragment that is a point of no occluder is
        // exempt from nothing.
        if mine == reference.x {
            continue;
        }
        // **And a surface does not shadow itself when it is cut into more than one
        // primitive** — `docs/occluders.md`'s D2, this step's whole subject. The
        // rule above answers for the fragment's own box and could never answer for
        // the box abutting it, which is a different primitive of the same wall,
        // floor or landing. `same_run` below stood in for it and only for a *panel*
        // on the same row or column; a **body** got no exemption at all, which is
        // the seam this cures.
        if on_the_lit_surface(stance, mine, stands.lo, stands.hi, delta) {
            continue;
        }
        // **A ray that only touches a solid at the point it starts from has not
        // gone through it** — `light::walk_cells_exact`'s own copy states the
        // argument at length, and it is `crosses`'s strictness said about a box
        // instead of a plane. No tolerance: the interval is exactly `0..0`.
        //
        // The case is a tread's outer corner. A riser is a plane on the climb
        // axis and a tread's lid stops exactly at it, so a fragment on that lip
        // stands in the riser's own plane at exactly the riser's top, and every
        // ray it sends touches that box at `t = 0` and nowhere else. Identity
        // cannot excuse it — the riser is genuinely a different primitive.
        if hit.entered == 0.0 && hit.leaves == 0.0 {
            continue;
        }
        let same_run = same_run(own, cell, first, lit.z, low, high);
        // Where along the ray this crossing happens, and therefore where in the
        // world — the exact crossing's own midpoint rather than the cell-shared
        // one the old walk used. It was also what the penumbra's own width was
        // computed from, until phase 5 took the width away; a panel still needs
        // the *point*, to ask whether the ray went through a window.
        let middle = (hit.entered + hit.leaves) * 0.5;
        let opacity = f32(stands.opacity) / 255.0;
        var by_surface = 0.0;
        if edges == 0u {
            // A **lid** — a floor, a rug, a plank. What crosses it is asked
            // over the ray's whole run inside the lid's own real horizontal
            // footprint (an infinite-`z` box, since a lid's own box above is
            // flat in `z` and would only ever catch the instant the ray is
            // exactly at its height), not over `hit`'s own thin interval.
            // `crosses` does the actual plane test on the `z` that gives it.
            // The primitive's own `x`/`y`, not the whole cell — a tread's
            // own top is a lid narrower than its tile, and asking a wider
            // box than the lid actually is would read a ray grazing the
            // tile's corner past the tread's real edge as "crossed through"
            // it. `docs/lighting_raymarch.md`'s "second bigger idea".
            let unbounded = ray_vs_solid(
                start,
                finish,
                vec3<f32>(stands.lo.x, stands.lo.y, -1.0e6),
                vec3<f32>(stands.hi.x, stands.hi.y, 1.0e6),
            );
            var tile_entered = hit.entered;
            var tile_leaves = hit.leaves;
            if unbounded.hit {
                tile_entered = unbounded.entered;
                tile_leaves = unbounded.leaves;
            }
            by_surface = opacity * crosses(
                lit.z + delta.z * tile_entered,
                lit.z + delta.z * tile_leaves,
                low,
                high,
            );
        } else if edges == EDGE_MASK {
            // A **body** — a whole tile that stands up and whose art would
            // not say which way. `ray_vs_solid` is an exact slab test, so a
            // hit already means the segment genuinely passed through the
            // box over `hit.entered..hit.leaves`: occlusion is the body's
            // own opacity outright. `light::walk_cells_streaming`'s own
            // mirror; see `docs/lighting_raymarch.md`'s "hard shadows"
            // decision for why the length-and-pierce grading this used to
            // do (and the widened-corner graze feeding it) are both gone.
            by_surface = opacity;
        } else {
            // A **panel** — a wall standing on one side of its tile. A
            // *surface*, and what a surface does to a ray is decided where
            // the ray pierces it: one sample, at the exact crossing's own
            // midpoint, less whatever of this cell is the same run of wall
            // the lit end stands in.
            let stops = edges & ~same_run;
            if stops != 0u {
                by_surface = opacity * pierced(stands, id, start + delta * middle);
            }
        }
        stopped = max(stopped, by_surface);
    }
    return stopped;
}

// **`ahead` is gone from this signature and from `sunlight`'s.** It was the lit
// end's own outward normal, and the only thing it was ever spent on was
// `STAND_OFF`; with the bias at zero there is nothing here that asks which way a
// surface looks. `docs/lighting_rebuild.md` phase 4.
fn walk(
    start: vec3<f32>,
    finish: vec3<f32>,
    tile: vec2<f32>,
    stance: u32,
    // Which solid of the grid the lit end is a point of — `own_solid` for a
    // sprite fragment, `mesh_instances[id].solid` for a mesh face, `SOLID_NOBODY`
    // for the ground and a mobile. Resolved once by the caller, per fragment.
    mine: u32,
) -> f32 {
    // **Both ends stand exactly where they are, and the bias is zero** —
    // `docs/lighting_rebuild.md` phase 4. A pair of nudges off the retired
    // attachment's byte layout stood here; identity is what keeps a ray from
    // being stopped by the surface it left, and it does not need the ray moved.
    let delta = finish - start;
    let ground = length(delta.xy);
    let lit = start;
    // The caller's own tile, not `floor(start.x/y)`: `start` can legitimately sit
    // exactly on this tile's own far edge — a mesh face's own outer corner does,
    // and nothing nudges it off — and flooring it back would pick whichever side
    // happens to round down rather than the side the geometry actually stands on.
    // `light::walk_cells`'s `first`, the same fix, and
    // `docs/lighting_raymarch.md` step 5: this is where it names the same class of
    // bug the CPU walk had, on the GPU walk decision 9 requires to match it.
    //
    // **And the carried tile breaks a tie rather than overruling the position** —
    // `light::starting_cell`, and the two are one rule. A point strictly outside
    // the tile it was handed is not on a boundary at all, and seeding this walk
    // from that tile hands it a negative distance to a boundary already crossed.
    let first = starting_cell(lit.xy, tile);
    if ground < 1.0e-6 {
        // Straight up or down. There is no direction to walk in and there never
        // was — but "the only cells on the line are the exempt ones", which is
        // what this used to say, stopped being true at decision 32: a **lid** on
        // that one cell is between the two ends of the ray, and a vertical ray is
        // the case a floor is most obviously in the way of. A torch on the ground
        // floor and the plank directly over it were the shortcut's own
        // counterexample.
        //
        // **A lid or a body, not a panel**, and `light::walk_cells`'s own copy of
        // this says why: a vertical ray is stopped by anything it is inside the
        // footprint of and whose span it crosses, and both of those shapes are
        // one. A panel is a plane, and a vertical ray lying in a wall's own plane
        // is the graze `same_run` exists for on the main path and has no answer
        // here. Lids alone until a fitted climbable's treads became bodies, which
        // is what exposed a gap that was already there for every body in the
        // world: a ray straight up out of a tree's box left it unstopped.
        let straight = solids_at(first.x, first.y);
        var stopped = 0.0;
        for (var i = 0u; i < straight.y; i = i + 1u) {
            let reference = id_at(straight.x + i);
            let id = reference.x;
            let stands = solid_at(id);
            let sides = stands.flags & EDGE_MASK;
            if sides != 0u && sides != EDGE_MASK {
                continue;
            }
            // And only the lids this ray is actually **under**. A tread's top is
            // a lid narrower than its tile; the walk below this shortcut has
            // asked it since sub-tile footprints landed and this shortcut did
            // not follow, so a flight's three treads shadowed one another from
            // straight above and below. `docs/lighting_height.md`'s backlog, and
            // `light::walk_cells_streaming`'s own copy of the same gate — the
            // primitive's own box on both axes, the same one the walk below
            // reads.
            if !over_footprint(start, stands.lo, stands.hi) {
                continue;
            }
            // Phase 4's rule, here too — `light::walk_cells_streaming`'s own copy
            // of it in this same shortcut, and here it is the whole rule: a ray
            // going straight up or down off a tread is exactly the ray whose only
            // contact with that tread's own top is the point it started at.
            if mine == id {
                continue;
            }
            // And D2's, for the same reason `light::walk_cells_streaming`'s copy of
            // this in its own shortcut gives: the neighbouring treads of one landing
            // top out on the very plane a fragment of one of them stands on, and a
            // ray going straight up crosses none of them.
            if on_the_lit_surface(stance, mine, stands.lo, stands.hi, delta) {
                continue;
            }
            let stops = f32(stands.opacity) / 255.0 * crosses(start.z, finish.z, stands.lo.z, stands.hi.z);
            stopped = max(stopped, stops);
        }
        return max(0.0, 1.0 - stopped);
    }
    // Which side of its own tile the lit end **is the face of**. What it is for
    // is `same_run` below: a run of wall is one surface and no part of a surface
    // shadows another part of it. The pixel's own side and not its tile's whole
    // mask, which is what decision 23 says in words — a corner's *perpendicular*
    // panel is a different surface and stops the ray as it always did. A pixel
    // that is not a face is part of no run and gets nothing. `light::walk_cells_streaming`.
    let own = own_side(stance);
    // Which way each axis steps and how much of the whole segment one tile of
    // it is worth. An axis the ray does not move along never reaches its
    // boundary, which is what the enormous `t` says.
    let toward = vec2<i32>(select(-1, 1, delta.x >= 0.0), select(-1, 1, delta.y >= 0.0));
    var per_tile = vec2<f32>(1.0e30, 1.0e30);
    var boundary = vec2<f32>(1.0e30, 1.0e30);
    // The known tile's own edge, not `floor(lit.x/y)` — the same reason `first`
    // above reads `tile`. A `lit` sitting exactly on this axis' boundary must
    // seed `boundary[axis]` near zero (the ray is already leaving `first`), and
    // `floor(lit.x/y)` there can just as well pick the far side and seed a whole
    // tile of slack that was never there. `light::walk_cells_streaming`'s `boundary[axis]`.
    if abs(delta.x) > 1.0e-6 {
        per_tile.x = 1.0 / abs(delta.x);
        let ahead = select(lit.x - tile.x, tile.x + 1.0 - lit.x, delta.x >= 0.0);
        boundary.x = ahead * per_tile.x;
    }
    if abs(delta.y) > 1.0e-6 {
        per_tile.y = 1.0 / abs(delta.y);
        let ahead = select(lit.y - tile.y, tile.y + 1.0 - lit.y, delta.y >= 0.0);
        boundary.y = ahead * per_tile.y;
    }

    // **Plain single-axis DDA, one cell per step, no diagonal probe on the
    // walk's own trajectory** — `light::walk_cells_streaming`'s own shape,
    // `docs/lighting_raymarch.md`'s point 4 cutover. A walk that never skips
    // a cell is complete by construction (the textbook reason grid-line
    // rasterisation steps one axis at a time), so there is nothing here for
    // `corner_tie`/`panel_stop` to add — see that function's own doc comment
    // for the fault-injection discipline that checked this rather than
    // assumed it. The unconditional probe below is a different, narrower
    // thing: see its own comment at the tie-break.
    var through = 1.0;
    var cell = first;
    for (var i = 0; i < MAX_WALK_STEPS; i = i + 1) {
        let stopped = cell_stopped(
            cell, first, own, mine, stance, start, finish, delta, ground,
        );
        through = through * (1.0 - stopped);
        if through <= 0.004 {
            // Under a byte's worth of light: nothing further can matter, and a
            // wall is the common case.
            return 0.0;
        }

        let next = min(boundary.x, boundary.y);
        if next >= 1.0 {
            break;
        }
        // A bare `<` decides which cell the walk's own trajectory visits
        // next, matching `light::walk_cells_streaming`'s own and
        // `light::dda_walk`'s bare comparison — biasing this one with a
        // tolerance was tried and reverted, see `light::walk_cells_streaming`'s
        // own doc comment for why: it changes which cell the walk visits, not
        // just how a near-tie candidate is read, and that broke agreement
        // with `light::dda_walk`'s own trajectory on ordinary geometry.
        let out_by_x = boundary.x < boundary.y;
        // Probe the side this step does not take, without moving the walk's
        // own trajectory there — unconditional, not gated behind how close
        // `boundary.x`/`boundary.y` are. `light::walk_cells_streaming`'s own
        // doc comment has the full argument: a *gated* probe was tried
        // first, and made parity worse rather than better, because CPU and
        // GPU do not compute a close-enough `boundary.x - boundary.y` to
        // agree on which rays even count as near a tie — widening the gate
        // only widened the set of rays where one backend probed and the
        // other did not. Unconditional costs nothing `light::walk_cells_
        // exact` could disagree with either: `light::candidate_tiles`
        // already names both single-axis neighbours at every transition
        // regardless of any tie.
        let probe = select(
            vec2<i32>(cell.x + toward.x, cell.y),
            vec2<i32>(cell.x, cell.y + toward.y),
            out_by_x,
        );
        let probe_stopped = cell_stopped(
            probe, first, own, mine, stance, start, finish, delta, ground,
        );
        through = through * (1.0 - probe_stopped);
        if through <= 0.004 {
            return 0.0;
        }
        if out_by_x {
            cell.x = cell.x + toward.x;
            boundary.x = boundary.x + per_tile.x;
        } else {
            cell.y = cell.y + toward.y;
            boundary.y = boundary.y + per_tile.y;
        }
    }
    return through;
}

// A number in `0.0..1.0` that belongs to a point of the world and to no other:
// the rotation `shadow` turns its sample pattern by.
//
// **World space and integers, so that both backends produce the same number.**
// The obvious thing — a hash of `texel` — cannot be spelled at all on the CPU
// side, which has no pixel; and the float hash a shader usually carries
// (`fract(sin(dot(…)))`) is exactly the arithmetic two backends are least likely
// to agree about. This quantises the position to a hundred-and-twenty-eighth of
// a tile, which is under a screen pixel at 1:1, and mixes the three integers with
// a plain bit-avalanche. Being stable *in the world* rather than on the screen is
// worth having on its own: a panning camera does not make a penumbra crawl.
//
// `bitcast` and not `u32(…)` for the sign: the two are the same by the spec's own
// modulo-2³² rule, and one of them says so. `light::dither`.
fn dither(at: vec3<f32>) -> f32 {
    let quantised = vec3<i32>(floor(at * 128.0));
    var hash = (bitcast<u32>(quantised.x) * 0x8DA6B343u)
        ^ (bitcast<u32>(quantised.y) * 0xD8163841u)
        ^ (bitcast<u32>(quantised.z) * 0xCB1AB31Fu);
    hash = hash ^ (hash >> 16u);
    hash = hash * 0x7FEB352Du;
    hash = hash ^ (hash >> 15u);
    hash = hash * 0x846CA68Bu;
    hash = hash ^ (hash >> 16u);
    // The top 24 bits over 2²⁴: a float's own mantissa, so every value this can
    // produce is representable and none of them is 1.
    return f32(hash >> 8u) / 16777216.0;
}

// How much of a flame the fragment at `at` can see: `SHADOW_RAYS` rays at its
// sphere, and the share of them that got there.
//
// **This is the whole of the soft shadow** — `docs/lighting_rebuild.md` phase 5.
// A shadow's edge is a gradient because a fragment in it can see part of the
// flame and not the rest, which is what a penumbra *is*; nothing here softens
// anything, and every one of the rays is the hard, exact walk the sun's ray is.
//
// The points are a Vogel spiral on the disc the sphere presents to this fragment
// — the silhouette, not the surface, which is the same choice
// `pathtrace::Emitter::Sphere` makes and for the same reason: what a receiver can
// be occluded from is the disc it sees. `sqrt` of the index spaces them by equal
// *area* rather than equal radius, so the middle of the flame is not sampled
// eight times over. The disc is laid out in **tile space**, `z` divided by
// `Z_PER_TILE` and multiplied back on the way out — the metric the sphere is
// round in, and the one the falloff and the cosine are already stated in.
//
// Only visibility is sampled: the falloff and the cosine are taken at the flame's
// own centre. At an eighth of a tile across, moving the sample point moves either
// of them by well under a byte, and it keeps a flame's brightness free of this
// function's noise. `light::shadow`.
fn shadow(at: vec3<f32>, to: vec3<f32>, tile: vec2<f32>, stance: u32, mine: u32) -> f32 {
    let toward = vec3<f32>(to.x - at.x, to.y - at.y, (to.z - at.z) / Z_PER_TILE);
    let span = length(toward);
    if span < 1.0e-6 {
        // The fragment is inside the flame. Every point of the sphere is as good
        // as any other and none of them has a direction; one ray to the centre is
        // the honest answer, and it is the ray with no length.
        return walk(at, to, tile, stance, mine);
    }
    let normal = toward / span;
    // Two directions across the ray, built the textbook branching way: away from
    // whichever axis the ray is most nearly along, so the cross product is never
    // near zero. Any consistent pair does — the pattern is rotated per fragment
    // anyway — and what matters is that `light::shadow` builds the same one.
    let aside = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(normal.x) > 0.9);
    let across = normalize(cross(aside, normal));
    let up = cross(normal, across);
    let phase = dither(at) * TAU;
    var reached = 0.0;
    for (var ray = 0u; ray < SHADOW_RAYS; ray = ray + 1u) {
        let angle = phase + GOLDEN_ANGLE * f32(ray);
        let radius = flame_radius() * sqrt((f32(ray) + 0.5) / f32(SHADOW_RAYS));
        let offset = (across * cos(angle) + up * sin(angle)) * radius;
        let point = vec3<f32>(to.x + offset.x, to.y + offset.y, to.z + offset.z * Z_PER_TILE);
        reached = reached + walk(at, point, tile, stance, mine);
    }
    return reached / f32(SHADOW_RAYS);
}

// How far in from the rim of a beam its edge finishes softening, as a share of
// the way from the rim to the axis. `light::BEAM_EDGE`, and the two are one
// number — a cone with a hard rim reads as a stencil laid over the scene rather
// than as light.
const BEAM_EDGE: f32 = 0.25;

// How much of a beamed flame escapes it in every other direction: a hand is not
// a shutter, and a cone with nothing outside it leaves the character carrying the
// light as the only black thing in the frame. `light::BEAM_SPILL`, and the two
// are one number.
const BEAM_SPILL: f32 = 0.25;

// How much of a flame's beam falls on a spot `offset` away from it — `x` and `y`
// in tiles and `z` in tiles as well, pointing *from* the flame *to* the spot.
//
// `light::Beam::lights`, arithmetic for arithmetic, and the parity test of
// `docs/lighting.md`'s decision 9 is what holds the two together. The smoothstep
// is written out rather than called for the same reason: two texts that mean to
// be one polynomial should be one polynomial.
fn cone(beam: vec4<f32>, offset: vec3<f32>) -> f32 {
    let distance = length(offset);
    if distance < 1.0e-6 {
        // A spot at the flame itself is inside every beam: there is no direction
        // from a point to itself, and a lantern's own tile is not the place to
        // start refusing light.
        return 1.0;
    }
    let along = dot(offset / distance, beam.xyz);
    let inner = beam.w + (1.0 - beam.w) * BEAM_EDGE;
    let t = clamp((along - beam.w) / max(inner - beam.w, 1.0e-6), 0.0, 1.0);
    return BEAM_SPILL + (1.0 - BEAM_SPILL) * t * t * (3.0 - 2.0 * t);
}

// How far along the ground one sunbeam may run, in tiles. `light::MAX_SUN_TILES`.
//
// The bound that matters is the ceiling below — a ray that has climbed above
// everything in the grid is looking at sky and stops there, which over open
// ground is two or three tiles. This is what is left for a sun so low that it
// never climbs out: a shadow thirty-two tiles long is already longer than any
// frame, and an unbounded segment would be a walk with no end at sunset.
const MAX_SUN_TILES: f32 = 32.0;

// How much of the sun reaches a fragment: 1 in the open, 0 in a wall's shadow.
//
// The same walk as a flame's, which is the whole of it now: what the sun has
// instead of a position is a *direction*, so the far end of the segment is
// computed rather than given. It is the point at which the ray leaves the
// grid's ceiling — `lighting.sun.w`, the height above which nothing this frame
// holds can stop anything — because from there on the ray is in the sky. A
// fragment already above the ceiling is in open air and the walk never starts.
//
// `false` and `0.0`: there is no far tile to exempt, the far end being a point in
// the sky, and the sun is half a degree wide, so its penumbra is the narrowest
// `walk` draws. The fragment's own tile is skipped there as it is for a flame,
// which is what lights a wall on the side the sun is on instead of shadowing it
// with itself.
fn sunlight(at: vec3<f32>, tile: vec2<f32>, stance: u32, mine: u32) -> f32 {
    let horizontal = length(lighting.sun.xy);
    if horizontal < 1.0e-6 {
        // Straight overhead: nothing but the fragment's own tile is in the way,
        // and that one is exempt.
        return 1.0;
    }
    // One tile of ground a unit, so `z` climbs by the sun's own slope.
    let step = vec3<f32>(
        lighting.sun.x / horizontal,
        lighting.sun.y / horizontal,
        lighting.sun.z / horizontal * Z_PER_TILE,
    );
    var tiles = MAX_SUN_TILES;
    if step.z > 1.0e-6 {
        tiles = min(tiles, (lighting.sun.w - at.z) / step.z);
    }
    if tiles <= 0.0 {
        // Above everything the grid holds — or the grid holds nothing at all,
        // which arrives here as a ceiling below every fragment there is.
        return 1.0;
    }
    return walk(at, at + step * tiles, tile, stance, mine);
}

// Where the light view stops being the multiplier and starts being a curve.
//
// The view's problem is that the interesting range is not `0..=1`. A night
// ambient is `0.36`, a torch adds `0.95` on top of it and a lit day is past
// `1.1`, so a clamp — which is what this was — painted the middle of every pool
// one flat white disc, and the middle is exactly the part of a flame's shape
// nothing else shows: the lit frame multiplies the art by it, and the art is
// dark, so the clipping never appears there.
//
// A tone map over the whole range is the other wrong answer, and it was tried:
// pulling `0.36` down to `0.26` and `1.27` to `0.56` leaves a picture where the
// lamp does not read as a lamp. So the curve is identity below `KNEE` and an
// exponential approach to `1.0` above it — everything a person reads as a level
// is untouched, and only what was being clipped is bent. Monotone and unbounded:
// nothing reaches white, so a step or a seam anywhere above the knee is still a
// step, and two flames overlapping still differ from one.
const KNEE: f32 = 0.6;

fn knee(value: f32) -> f32 {
    if value <= KNEE {
        return value;
    }
    let headroom = 1.0 - KNEE;
    return KNEE + headroom * (1.0 - exp(-(value - KNEE) / headroom));
}

// One of the pass's own values, as a colour.
//
// Every argument here is something the lit path already computed — none of these
// views measures anything of its own, which is what makes them evidence about
// *this* frame rather than about a second frame drawn like it. See
// `crate::debug::View` for what each one is for.
fn debug_color(
    view: u32,
    kind: u32,
    tile: vec2<f32>,
    at: vec3<f32>,
    normal: vec3<f32>,
    sub: vec2<f32>,
    lit: vec3<f32>,
    flames: vec3<f32>,
    reached: u32,
    nearest: f32,
    nearest_through: f32,
    sun_through: f32,
    share: f32,
) -> vec3<f32> {
    if view == VIEW_SKY {
        // The sky field on the ground it is a field of: white under open air,
        // black under a roof, and a gradient across a doorway. Drawn over the
        // picture rather than as a wireframe of the boxes, because the failure
        // this field actually has is a tile that is *wrongly open* — an eave that
        // did not cover the floor under it — and a box is drawn for what stands,
        // not for what does not. `docs/lighting_world.md`'s backlog says so.
        return vec3<f32>(share, share, 0.15 + share * 0.85);
    }
    if view == VIEW_SUN {
        // How much of the sun arrived: white in the open, black in a wall's
        // shadow, and the sun's own colour where a pane dimmed it rather than
        // stopping it — which is the picture a person looking for a lit patch on
        // a floor is actually after.
        if lighting.sun_color.w <= 0.0 {
            return vec3<f32>(0.0, 0.0, 0.35);
        }
        return vec3<f32>(sun_through);
    }
    if view == VIEW_NORMAL {
        // The G-buffer's normal plane, drawn. Each axis is a channel, mapped
        // from `-1..1` into `0..1` the way every normal map is read — so a
        // south-facing wall is green, an east-facing one red, and anything
        // lying flat is blue.
        //
        // A surface with **no** facing is the zero vector, which lands on the
        // grey in the middle of all three — and that is the shade to look for:
        // it is the honest one for a mobile and for a tree, and phases 6 and 7
        // are the work of leaving less of it in a frame. Its being a *neutral*
        // grey rather than a colour is the point: nothing about it reads as a
        // direction, because it is not one.
        return normal * 0.5 + vec3<f32>(0.5);
    }
    if view == VIEW_PLACE {
        // A checkerboard of tiles with the sub-tile fraction laid over it: the
        // squares say which tile a pixel claims, and the gradient inside each
        // one says the fraction is being written at all. A wall reads as one
        // flat square with its sprite standing out of it — which is exactly the
        // property the lighting depends on.
        //
        // The tile itself, off the row the fragment's id names — which is what
        // this view always claimed to draw and did not: the checker used to be
        // taken from the two halves of the *id*, so a frame's squares counted
        // instance rows rather than tiles. It went unnoticed because a
        // diagnostic is read for whether a gradient is *there*, and both
        // versions have one.
        let checker = f32((u32(tile.x) + u32(tile.y)) & 1u) * 0.35 + 0.15;
        return vec3<f32>(sub.x, sub.y, checker);
    }
    if view == VIEW_KIND {
        if kind == 1u {
            return vec3<f32>(0.20, 0.65, 0.30);  // land
        }
        if kind == 2u {
            return vec3<f32>(0.25, 0.45, 1.00);  // a static or an item
        }
        return vec3<f32>(1.00, 0.40, 0.15);      // a mobile
    }
    if view == VIEW_HEIGHT {
        // A ramp over the `z` a map actually uses, with a band every tile's
        // worth of height: the ramp is for reading a slope, the bands are for
        // counting storeys, and eleven units is one tile — the ratio that
        // decides whether a cellar's flame reaches the street.
        let ramp = clamp((at.z + 64.0) / 128.0, 0.0, 1.0);
        let band = fract(at.z / Z_PER_TILE);
        return vec3<f32>(ramp, band * 0.8, 1.0 - ramp);
    }
    if view == VIEW_OCCLUDERS {
        let cell = merged_at(i32(floor(at.x)), i32(floor(at.y)));
        if cell.edges == 0u {
            return vec3<f32>(0.06, 0.06, 0.10);
        }
        let opacity = f32(cell.opacity) / 255.0;
        // Red where this pixel's own height is inside the span the tile stops
        // light at, blue where the tile occludes but not here. The second colour
        // is the one worth having: a wall of an upper storey standing over an
        // open street is blue, and a shadow appearing under it would be the bug.
        if at.z >= cell.low && at.z <= cell.high {
            return vec3<f32>(0.25 + 0.75 * opacity, 0.05, 0.05);
        }
        return vec3<f32>(0.05, 0.10, 0.25 + 0.55 * opacity);
    }
    if view == VIEW_LIGHT {
        // The lighting with the art thrown away: the pools' own shapes, where a
        // seam or a step has nothing to hide behind. `KNEE` is why it is not a
        // clamp and not a tone map either.
        return vec3<f32>(knee(lit.r), knee(lit.g), knee(lit.b));
    }
    if view == VIEW_FLAMES {
        // And the same pools with the ambient taken out, on black: what the
        // *flames* added and nothing else. No curve here — a flame's whole range
        // is already `0..=1` and a little over, and bending it is what makes the
        // view above unable to answer whether a pool has a shape at all.
        return clamp(flames, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    if view == VIEW_SHADOW {
        // Blue for "no flame reaches here at all", which is a different fact
        // from "a wall is in the way" and the one people confuse.
        if nearest >= 1.0 {
            return vec3<f32>(0.0, 0.0, 0.35);
        }
        // Dark red for "a flame reaches, and is fully blocked" — the walk's
        // own answer of "none", not silence. Left as pure black this read
        // identically to the cleared background this fragment never reaches
        // (`KIND_NOTHING`, above, returns `color` before this view is ever
        // asked), and that has cost a wrong diagnosis twice: a shadowed pixel
        // beside empty background read as an orphaned shape, and a face's own
        // outline read as fewer sides than it has. `docs/lighting_raymarch.md`
        // step 1. `0.004` is `light.rs`'s own `RAY_CUTOFF`.
        if nearest_through <= 0.004 {
            return vec3<f32>(0.2, 0.0, 0.0);
        }
        return vec3<f32>(nearest_through);
    }
    // VIEW_REACH: how many flames got through, green through red, and the same
    // blue as above for none.
    if reached == 0u {
        return vec3<f32>(0.0, 0.0, 0.35);
    }
    let many = min(f32(reached) / 4.0, 1.0);
    return vec3<f32>(many, 1.0 - many * 0.7, 0.15);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Straight through: both sides are `Rgba8Unorm` and this crate never
    // converts colour. Whatever filtering happens is the sampler's, chosen by
    // the caller from which way the zoom went.
    let color = textureSample(world, world_sampler, in.uv);

    // What drew this pixel. `textureLoad` and not a sampler, and the texel is
    // the nearest one whatever the zoom: an id is an identity, and half of one
    // is nobody.
    let size = vec2<f32>(textureDimensions(ids_of));
    let texel = vec2<i32>(clamp(in.uv * size, vec2<f32>(0.0), size - vec2<f32>(1.0)));
    let ids = textureLoad(ids_of, texel, 0).x;

    let view = lighting.view.x;

    // Nothing was drawn here — the cleared background, or the letters over a
    // speaker's head, which are a message and not a thing standing in the
    // street. Neither is lit and neither is dimmed. See `crate::place::Kind`.
    //
    // The diagnostics leave it alone too, and deliberately: a view that painted
    // the background would make the world's own silhouette hard to find, and
    // every one of these is read by comparing a shape against the picture it
    // came from. The one exception is the kind view, whose whole subject is
    // which pixels are nothing.
    if ids_kind(ids) == KIND_NOTHING {
        if view == VIEW_KIND {
            return vec4<f32>(0.0, 0.0, 0.0, color.a);
        }
        return color;
    }

    // Which *tile* this pixel's picture came from is still an id into a
    // per-kind instance buffer — `docs/gbuffer.md` step 3 for a static or a
    // mobile, step 7 for the ground — because the tile is what the shadow walk
    // starts its cell stepping from, and a whole number a pass knew exactly is
    // not something to recover by flooring anything.
    let kind = ids_kind(ids);
    // Read before the id lookup below, not after: a mesh face's sentinel
    // (`STANCE_MESH_FACE`) decides *which* buffer the id addresses, and the
    // real stance a mesh face's normal names lives in that buffer's own row,
    // not in this word — see the branch below and
    // `crate::place::Stance::MeshFace`'s own doc.
    var stance = ids_stance(ids);
    let id = ids_id(ids);
    var tile = vec2<f32>(0.0);
    // **And which solid of the grid this fragment is a point of** — the one
    // number the shadow walk compares, `docs/lighting_rebuild.md` phase 4.
    // Resolved here, once a fragment, rather than asked per candidate solid per
    // light the way the owner-and-shape pair it replaced was.
    //
    // Land keeps `SOLID_NOBODY`: the ground is no occluder, so it is exempt from
    // nothing, which is what it always was in effect.
    var mine = SOLID_NOBODY;
    if kind == KIND_LAND {
        tile = vec2<f32>(
            f32(ground_instances[id].place0 & 0xFFFFu),
            f32(ground_instances[id].place0 >> 16u),
        );
    } else if kind == KIND_STATIC && stance == STANCE_MESH_FACE {
        // `docs/gbuffer.md` step 4c: a mesh face has no picture of its
        // own, so its row lives in `mesh_instances`, not
        // `face_instances` — and carries the *real* stance the
        // attachment's sentinel stood in for.
        //
        // And its solid outright: a mesh face is one primitive by construction,
        // so the pass that drew it already knew which. See `MeshFaceInstance`.
        let row = mesh_instances[id];
        tile = vec2<f32>(f32(row.tile & 0xFFFFu), f32(row.tile >> 16u));
        stance = row.stance;
        mine = row.solid;
    } else {
        var row_place: vec2<u32>;
        var owner = OWNER_NONE;
        if kind == KIND_STATIC {
            row_place = face_instances[id].place;
            owner = face_instances[id].owner;
        } else {
            row_place = mobile_instances[id].place;
            // A mobile's row stamps `OwnerId::NONE` on the CPU side
            // (`mobiles::collect`), so this reads it rather than assuming it —
            // one place decides what a billboard is a point of, and it is the
            // one that builds the row. `own_solid` turns that into
            // `SOLID_NOBODY` by the same rule it uses for everything else,
            // which is why a mobile needs no branch of its own here.
            owner = mobile_instances[id].owner;
        }
        tile = vec2<f32>(f32(row_place.x & 0xFFFFu), f32(row_place.x >> 16u));
        // A sprite instance is not one primitive: a corner static is two panels
        // and one picture, and the stance `statics.wgsl` resolved says which.
        mine = own_solid(vec2<i32>(i32(tile.x), i32(tile.y)), owner, own_side(stance));
    }
    // And where the fragment itself is — read, not rebuilt. This used to be
    // `tile + fraction` with the fraction taken out of the place attachment's
    // fourth channel in hundred-and-twenty-sevenths, and a height taken out of
    // its third as eight whole units plus four bits of sixteenths. Every one of
    // the three was exact in the pass that computed it; `docs/lighting_rebuild.md`
    // phase 2 is the decision to carry the number instead of the fields, and its
    // second root — "a fragment's own position is not exactly known, so a shadow
    // ray must start away from where it really is" — is the bill that packing ran
    // up.
    let at = textureLoad(position_of, texel, 0).xyz;
    // Where in its tile that leaves it. Nothing computes the fraction any more;
    // this is the difference between two things already known, and only the
    // diagnostics below read it.
    let sub = at.xy - tile;
    // And which way the surface drawn here looks — read, like the position, and
    // no longer recovered from four bits of the place attachment. Zero for the
    // ground, for a mobile and for anything standing up whose art would not name
    // an edge: none of those has a side to be lit from, and a zero vector is the
    // value that says so.
    //
    // The distinction between land and a wall's flat cap — both `STANCE_FLAT`,
    // and only one of them wanting the retired half-space gate — used to be made
    // here, by testing the kind. It is made by the *producer* now: `ground.wesl`
    // writes the bilinear patch's own normal, and `statics.wesl` writes the face
    // of the box this fragment's own view ray met (`package::impostor`,
    // `docs/lighting_rebuild.md` phase 6c). Neither is a table any more, which is
    // what retired `place_format.wesl`'s `outward` and the two rows of it that
    // were wrong.
    let normal = unpack_normal(textureLoad(normal_of, texel, 0).x);

    // The ambient this *tile* has: the sky's share of it scaled by how much of
    // the sky the column over the tile can see, plus the floor everything gets.
    // `docs/lighting_world.md`, decision 1, and it is the whole of why a room is
    // darker than the road outside it before anything burns.
    //
    // **The tile the pass carried, not `floor(at.x/y)`** — the same reason
    // `first` below is, and the same class of defect: a surface may lie exactly
    // on a tile boundary, and since `docs/lighting_rebuild.md` phase 6c meets a
    // fragment with its own volume rather than clamping it a
    // hundred-and-twenty-seventh of a tile inside, a south or east wall's face
    // does. Flooring one of those reads the *neighbour* tile's sky, so a street
    // wall would take the room's ambient along one of its two sides and not the
    // other.
    let share = sky_at(i32(tile.x), i32(tile.y));
    var lit = lighting.ground.rgb + lighting.sky.rgb * share;
    // What the diagnostics are made of, gathered while the frame is lit rather
    // than computed a second time: how many flames actually reached this
    // fragment, and what the *nearest* of them lost on the way. Nearest and not
    // an average, because a shadow view is read as "is this pixel behind
    // something", and the flame that answers that is the one lighting it most.
    var reached = 0u;
    var nearest = 1.0e9;
    var nearest_through = 0.0;
    // What the flames added, apart from the ambient they were added to. Only
    // `VIEW_FLAMES` reads it, and it is accumulated here rather than recomputed
    // there for the reason every one of these values is: a diagnostic that lit its
    // own copy of the frame would answer about that copy.
    var flames = vec3<f32>(0.0);
    let count = u32(lighting.sky.w);
    for (var i = 0u; i < count; i = i + 1u) {
        let light = lighting.lights[i];
        let to = light.place.xyz;
        // All three axes in one unit: `z` is divided into tiles, so a flame
        // reaches as far up and down as it does sideways. Without it the screen's
        // own folding comes back — a cellar lighting the street.
        let offset = vec3<f32>(to.x - at.x, to.y - at.y, (to.z - at.z) / Z_PER_TILE);
        let d = length(offset) / max(light.place.w, 0.001);
        if d >= 1.0 {
            // Outside the pool entirely, which most fragments are for most
            // flames: no ray is walked and nothing is sampled. This is why the
            // grid walk below is affordable at all.
            continue;
        }
        // Which way it is pointing, where it points one way at all. A dot
        // product and a branch: a fire standing in the world writes a rim below
        // every cosine there is and never reaches the arithmetic.
        //
        // `offset` runs from the fragment to the flame and with `z` already
        // divided into tiles, which is the space the axis is stated in; a beam's
        // axis runs the other way, so the sign flips here.
        var lit_by = 1.0;
        if light.beam.w > -1.0 {
            lit_by = cone(light.beam, -offset);
        }
        // And how squarely this surface looks at the flame. A wall is one-sided
        // and its two faces are one tile, so this is the only thing that keeps a
        // torch in a room from lighting the outside of the house.
        //
        // Geometry and nothing else. This used to exempt a flame standing in the
        // wall's own row or column, because a lamp mounted on a house sits at its
        // tile's centre — behind the plane of the face it is bolted to — and
        // testing it blacked out the very wall it hangs on. But a *line* is the
        // whole length of a street: a lamp post standing south of a house is in
        // the column of its east wall, and that wall's faces came out fully lit
        // from a flame half a tile behind their plane, for the length of the run.
        // Reported from the client at Britain's `(1441, 1692)`. What answers it is
        // moving the mounted flame instead of excusing it — `light::mounted_at`,
        // and `docs/lighting.md`'s decision 26.
        // `offset` is already the flame less the fragment with `z` divided into
        // tiles, which is the space the normal is stated in — so a lid and a wall
        // go through one formula, and since it is normalised the answer is the
        // same angle on both.
        //
        // The zero vector still means "no facing at all", and it is a value a
        // producer writes on purpose rather than an absence — see
        // `docs/lighting_rebuild.md` phase 2 on why the plane is not octahedral.
        if any(normal != vec3<f32>(0.0)) {
            lit_by = lit_by * lit_from(normal, offset);
        }
        // Eight rays at the flame's own sphere, and the share of them that
        // arrived — `docs/lighting_rebuild.md` phase 5. It is the only term in
        // this loop that is an estimate rather than a formula, which is why it is
        // the only one with a ray count behind it.
        let through = shadow(at, vec3<f32>(to.x, to.y, to.z), tile, stance, mine);
        // Recorded before the shadow is tested, so that a fragment inside a pool
        // and behind a wall is *nearest, stopped* rather than indistinguishable
        // from open ground nothing reaches. That difference is the one the whole
        // shadow view exists to show.
        if d < nearest {
            nearest = d;
            nearest_through = through;
        }
        if through <= 0.0 {
            continue;
        }
        reached = reached + 1u;
        // Linear distance, squared falloff: `1 - d` alone gives a cone with a
        // visible straight edge in the gradient, and an inverse-square law with
        // no cutoff has no radius at all and tints the whole frame. This is the
        // soft pool with a hard end — the shape the reference isometrics draw.
        let fall = 1.0 - d;
        let added = light.color.rgb * (light.color.w * fall * fall * through * lit_by);
        lit = lit + added;
        flames = flames + added;
    }

    // And the sun, which is one direction rather than a place: no distance and no
    // falloff, only whether anything stands between this pixel and the sky.
    var sun_through = 0.0;
    if lighting.sun_color.w > 0.0 {
        sun_through = sunlight(at, tile, stance, mine);
        lit = lit + lighting.sun_color.rgb * (lighting.sun_color.w * sun_through);
    }

    if view != VIEW_LIT {
        return vec4<f32>(
            debug_color(
                view, kind, tile, at, normal, sub, lit, flames, reached, nearest, nearest_through,
                sun_through, share,
            ),
            color.a,
        );
    }

    // Multiplicative, so a flame brightens whatever is under it — ground, wall,
    // the body standing in it — rather than laying a coloured film over it. A
    // pixel the art left black stays black, which is what makes the light look
    // like it is falling on the scene instead of floating above it.
    //
    // **Through `shade`, which is the whole of `docs/lighting_rebuild.md`'s
    // phase 1**: the art is decoded out of sRGB, the multiplication happens in
    // linear radiance, and the result is curved and encoded once. What this
    // replaced was `min(color.rgb * lit, 1.0)` — a multiplication of stored
    // bytes, in which two equal flames are not twice one flame and a surface at
    // half the distance is not four times as bright, so no reference renderer
    // could agree with this pass even in principle.
    //
    // `Lighting::NONE` is no longer a texel-for-texel copy of the world image
    // and cannot be: a light of exactly `1.0` still goes through the curve, and
    // a curve that left its input alone would not be one. The frame test that
    // asserted the copy now asserts `crate::tonemap::shade_u8` of it, which is a
    // stronger statement — it catches a blit that shifts by a texel *and* a
    // colour pipeline that drifts from its own twin.
    //
    // Alpha carried through untouched, not written as `1.0`: this pass shades an
    // image and lighting is about how bright a pixel is, not about whether it is
    // there.
    return vec4<f32>(shade(color.rgb, lit), color.a);
}
