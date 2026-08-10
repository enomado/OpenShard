# The lighting engine: where it stands

**A status document, not a plan.** Six tracks have been running against one
renderer — the rebuild itself and five plans that came out of it — and each of
them is written as a *living plan with a backlog*, which is the right shape for
doing the work and the wrong shape for answering "is the lighting finished".
This page answers that question, in one place, and says which document holds the
reasoning for each line.

Nothing here is new work or a new decision. Where this page and a track document
disagree, the track document is right and this page is stale.

## The one-line answer

**The model is built, calibrated against a path tracer, and shipping. What is
left is not the model — it is the geometry the model is fed, two terms that were
never written (the sun's BRDF, a mobile's normal), and the content layer
(a day curve, UO's own `light.mul` mode).**

Concretely: a fragment today carries an exact world position, a measured normal,
an albedo and the name of the primitive it is a point of; it is lit by
`albedo × max(N·L, 0) × colour × intensity × windowed-inverse-square × visibility`
summed over eight stratified samples of a spherical flame, in linear radiance,
tonemapped once by an ACES fit. Every constant that used to stand in for a
missing measurement is deleted, and every deletion was gated by injecting the
fault rather than by reading the code.

What still misreads on screen is, without exception, a **box that does not fit
its picture** — which is why the two newest tracks
([`footprints.md`](footprints.md), [`silhouettes.md`](silhouettes.md)) are about
boxes and art rather than about light.

## Readiness, by subsystem

| Subsystem | State | What is left | Held by |
|---|---|---|---|
| Colour: sRGB in, linear throughout, ACES out | ✅ shipping | — | [`lighting_rebuild.md`](lighting_rebuild.md) phase 1 |
| G-buffer: position, normal, ids, albedo — 32 B/sample, WebGPU's floor exactly | ✅ shipping | — | phase 2 |
| BRDF: `max(N·L, 0)`, no dial, no band | ✅ shipping | — | phase 3 |
| Attenuation: windowed inverse square | ✅ shipping | — | phase 3 |
| Shadows: self-hit by primitive id, bias `0` | ✅ shipping | — | phase 4 |
| Area light: a sphere, 8 stratified rays, world-space dither | ✅ shipping | — | phase 5 |
| Every term a function of the sample point (no flame centre in the loop) | ✅ shipping | — | phase 5b |
| Impostor: one silhouette, the box met per fragment | 🟡 shipping with two holes | a corner's two panels are still told apart by the **screen half** (the box carries no instance row); the **fringe** — a clamped position where art overhangs its box | phase 6, 6f–6i |
| Occluders: absolute coordinates, merged runs, a BVH, no tile in the answer | ✅ landed | — | [`occluders.md`](occluders.md) (a record) |
| Footprints: a sub-tile box measured off the art | ✅ landed, partial by design | the **height** is never measured — a roof's picture stands 76 px over a box 3 `z` tall; the remaining `Crooked` class is furniture standing on more than one thing | [`footprints.md`](footprints.md) |
| Frame assembly: one `frame::assemble`, one `Inputs`, gated plane by plane | ✅ landed | P4 items 2–4 (a `CLEAR` piece's name, the whole-tile stand-in, `PANEL_THICKNESS`) | [`parity.md`](parity.md) |
| Pixel spaces: the census, the commensurability statement, the newtypes, the gates | ✅ landed | the **art texel** is the one grid with no type | [`pixels.md`](pixels.md) |
| Silhouettes: attribution of the two edges, the seam, the clamp | 🟡 attributed | the **widths** at `4x`; the decision S2 (leave it / let the box bound more / estimate coverage) | [`silhouettes.md`](silhouettes.md) |
| Billboards (mobiles) | 🟡 half | the inflated-silhouette normal and the choice between it and the camera-facing plane — its *done when* is a person looking | phase 7 |
| The sun | ⬜ not started | it is added straight, with **no `N·L` anywhere**; no soft edge, no sky visibility as ambient occlusion | phase 8 |
| Ambient: the day curve, the sky field reaching a lit pixel | ⬜ carried | no default frame has an ambient split, so a house reads as bright as the street | [`lighting_world.md`](lighting_world.md) |
| UO's own light: `light.mul` / `lightidx.mul` as a picked mode | ⬜ scoped, not started | the tiledata light-id parse, both file readers, the composite point, the toggle | phase's *Wanted after the model works* |

## The pipeline, phase by phase

`geometry pass → G-buffer → lighting pass → tonemap → screen`, the ordinary
deferred arrangement, with the decision everything rests on stated once: **the
art is albedo and the light is ours.** No term anywhere argues with the artist.

| | Phase | State |
|---|---|---|
| 0 | the reference path tracer | ✅ — engine and tracer agree on open ground to one step of 255 over 262,144 pixels |
| 1 | linear and HDR | ✅ |
| 2 | the G-buffer | ✅ — `place`'s packing is gone entirely |
| 3 | the BRDF | ✅ — `FACE_EDGE` deleted |
| 4 | shadows by identity | ✅ — `STAND_OFF`, `ON_TOP`, `exemption` deleted; the light oracle reads zero at every flame height |
| 5 / 5b | area lights, then no centre | ✅ — shadows ~8× crisper; the join wedge gone, signed mean `-0.0044` → `-0.0002` |
| 6 | the impostor | 🟡 — 6a, 6c, 6d, 6f, 6g, 6h landed; 6i's item 1 (a fixture driving `statics::collect` over a fitted climbable) is what is left |
| 6e | the grid stops being a rule | ✅ — [`occluders.md`](occluders.md), all six steps |
| 7 | billboards | 🟡 — position and the camera-facing normal landed |
| 8 | the sun | ⬜ |

**The instrument is a picture beside the path tracer's, looked at by a person.**
Twelve tests whose subject was the agreement of two of our own implementations
were retired for that reason; what survives is the brute-force occlusion
oracles, the world claims, and the pictures — which are the acceptance
instrument, not a side channel.

## The pixel spaces — the spec

Normative. The derivation, the per-site census and the pair-by-pair
commensurability table are [`pixels.md`](pixels.md); this is the statement of
what a person writing code here may assume.

### The grids

| Grid | Unit | Type | Origin |
|---|---|---|---|
| Real (screen) pixel | one physical pixel | `camera::RealPixel` (whole), `camera::RealPoint` (fractional) | the window; `ViewportRect.x/y` is **window-absolute** |
| View (virtual) pixel | one world pixel at `1:1` | `camera::ViewPixel` (whole), `camera::ViewPoint` (fractional) | the rounded eye, centred in `render_width()` |
| World pixel | one world pixel at `1:1`, camera-free | `camera::WorldPixel` (`i32`), `camera::WorldPoint` (`f64`) | the map |
| Tile / world units | `TILE_WIDTH` = 44 view px; `z` in `Z_STEP` = 4 view px | `camera::Point`, `camera::WorldSpot`, `light::WorldVec` | the map |
| Tile space (metrics) | all three axes in tiles — `z` divided by `Z_PER_TILE` = 11 | `light::TileVec` | a vector space; no point type yet |
| Art texel | one texel of an art file | **none** — the one grid with no type | the sprite's own rectangle in an atlas |
| Clip space | −1..1 | `vec4<f32>` | the viewport |

### The rules

1. **`z` is divided by `Z_PER_TILE` exactly twice**, in
   `TileVec::between` and `TileVec::in_world_units`. A metric — a distance, a
   cosine, a normal, a beam axis — lives in tile space; a *position* lives in
   world units. The two are one multiplication apart and the compiler now knows
   which is which.
2. **A tile corner is always a whole world pixel**, and a whole world pixel is
   always a whole view pixel. Both conversions are exact integer arithmetic
   upstream of the zoom ladder; no rung, parity or eye fraction can move them.
3. **No primary sample lands on a whole virtual pixel** at any magnifying rung,
   at either viewport parity, at any eye fraction — the nearest a sample comes
   is `0.5 / scale`. This is what stops a view ray passing exactly through a
   box's corner, where `impostor::meets`'s tie has no right answer. One
   exception is listed and asserted to *reproduce*: at `2/3x` the eye's quantum
   is `1.5`, so half of all camera positions there reach the corner.
4. **The fragment grid and the impostor's tile space are never commensurate, and
   the tolerance between them is a quantum rather than an epsilon.**
   `impostor::FRAGMENT` is `SQRT_2 / TILE_WIDTH` — the distance to the next
   sample, in the space the comparison is made in. A rounding epsilon in that
   role measured a floor's own seam as "outside its own box" and drew a glowing
   grid across every room.
5. **Below `1:1` there is no primary sample to land anywhere**: the world is
   drawn at `1:1` into an oversized image and the blit's linear sampler shrinks
   it. The whole commensurability question is about magnification.
6. **A constant that crosses into a shader is pinned from the shader's own
   source.** `TILE_WIDTH`, `Z_PER_TILE`, `Z_STEP`, `HALF_TILE_HEIGHT` and
   `FRAGMENT` have no compiler on either side of the wire, and a disagreement
   there does not fail to build and does not fail to draw — it draws a different
   frame from the one every test asserts about.

### The gates that hold them

| Rule | Gate |
|---|---|
| 2 | `grids.rs`'s `a_tile_step_is_a_whole_number_of_world_pixels`, `a_height_unit_is_a_whole_number_of_tile_space_units` |
| 3 | `camera::tests::no_primary_sample_lands_on_a_whole_virtual_pixel` (all seven rungs × both parities × every eye fraction, ~121k samples, with the `2/3x` exception counted); `tests/parity.rs`'s odd-versus-even frame comparison |
| 4 | `impostor::tests::every_pixel_of_a_blocks_picture_meets_that_blocks_own_box`, with a floor under the constant so halving it goes red |
| 6 | `grids.rs`'s `the_shaders_restate_the_cameras_constants_and_not_their_own` — reads the numbers back out of the `.wesl` source |
| the tie itself | `impostor::a_ray_through_a_boxs_own_corner_is_answered_by_the_order_of_three_ifs` — a record of the rule, not an endorsement of it |

### What is not typed, and why

- **The art texel.** Its hazard is real but is *between two conventions of one
  grid* — `LandAtlas` and the statics atlas divide exactly, `TexmapAtlas` insets
  by half a texel — so a newtype over the texel does not stop it. What would is
  a type carrying the convention, and that belongs with
  [`silhouettes.md`](silhouettes.md).
- **`geometry::Rect`** is a sprite's rectangle, an atlas rectangle, a gump's
  place and a plan pixel's. A shape shared by four spaces is a different problem
  from two spaces sharing one number.
- **A tile-space *point*.** `TileVec` is the vector; `Vec2` still means a
  tile-space position in `light.rs` (`Light.at`, `Spot::at`). The confusion is
  reachable — the `ViewPoint` sweep annotated `debug.rs`'s tile-space `middle`
  as a view point and the compiler caught it.

## What is left, ranked

**1. The height nobody measures.** The largest single number on the whole
track: the whole-tile class discards **32.7%** of its own art and roofs inside
it 44–53% — `0x05A2` "slate roof" is 48×76 pixels of picture standing on a box
three `z` units tall. Every fringe artefact below is downstream of it.
[`footprints.md`](footprints.md) deliberately measured the *footprint* and left
the height as a carried item, with `blocks_silhouette` named as the instrument
that would score it. **Measured 2026-08-10** (`geometry_census`, same window):
of the 3,388 whole-tile stand-ins, **2,825 (83.4%)** carry `ROOF` — a sloped
plate, which height alone does not turn into an AABB. The other **563
(16.6%)** are the actual target of "grow the box to the silhouette"; roofs are
a separate primitive question, not a height question, and stay out of A-2's
scope.

**2. The fringe, and it is one decision with two candidates.** A pixel whose ray
misses its box is clamped to the nearest point on it. That is better than the
two alternatives already tried and measured (drawing nothing: 11.09% of every
panel's art and 32.4% of a whole-tile one; no facing: lit from every side,
measured as a worse artefact and reverted). What is unbounded is *how far* — the
worst clamp is 133 fragments, four tiles, and the shadow ray starts there. The
open candidates are "keep the clamp" and "give a miss the face the sprite's own
volume presents".

**3. Phase 7's second half.** A mobile's normal is one vector for the whole
sprite, so a torch on a figure's left reads no brighter than one on its right.
The inflated-silhouette candidate is unbuilt, and the choice between the two
wants a picture of a figure beside a lamp — which wants a mobile pass in
`examples/isolated_scene.rs`, which does not exist.

**4. Phase 8, the sun.** No cosine, no soft edge, no sky visibility. The sky
field is ambient occlusion by another name and phase 8 is where it is adopted.

**5. The content layer.** The day curve, lights carried by other mobiles, the
flame's own glow, the sunbeam through a window, land as an occluder, and UO's
own `light.mul` mode as a switch beside the deferred pipeline.

**6. The instruments.** The tracer is single-threaded at 13 s a frame and has
never been run over a real map; `tests/dump.rs` draws at even extents only; no
gate holds that a debug view is drawn from the same planes the lit frame is.

## Open defects a person can see

Each of these has been reported by somebody looking at a frame, and each is
measured rather than guessed at. None is a defect in the model.

| | What it looks like | What it is |
|---|---|---|
| 🚩 | **A flame's own sprite is black.** Every free-standing emitter taller than `FLAME_LIFT` | The flame burns at the tile's centre, *inside* the lamp post's own box; the impostor answers the sprite with the box's camera-facing face, whose normal points away from the flame, so `N·L ≤ 0` on every visible pixel |
| 🚩 | **A sprite's top edge is serrated** | The nearest face of a *miss* flips between two answers along a silhouette, so a smooth overhang reads as a comb |
| 🚩 | **A whole-tile body reads dark and striped** | A body — the box for a graphic whose art would not name a side — writes a camera-facing normal it has no right to. This and the black emitter are **one question**: what should a body write for a normal |
| 🟡 | **Specks and dashes on an indoor floor** | Furniture drawn wider than its own per-tile box; the pixel over the boundary belongs to a static whose box is a tile away, and its ray leaves through a side face. 32 of 66 are pieces the grid holds nothing for, so no identity can excuse them |
| 🟡 | **A corner's two panels disagree near the tile corner** | The id follows `split_corners`' twin row and a `Volume` carries a `SolidId`, not a row number, so the *identity* is still picked by which half of the sprite a pixel was drawn on while the *normal* is picked by the box |
| 🟡 | **A north or west wall's face is a fifth of a tile inside its room** | `PANEL_THICKNESS` fattens inward, so two walls of one run drawn on one plane get positions four fifths of a tile apart. The construction that removes it is one slab straddling the shared edge |

## The map: which document holds what

The rebuild consolidated seven plans; five more have come out of it since. All
of them stay — the reasoning is worth more than the code it justified — but only
three are *live*.

**Live plans, with backlogs a session can start from:**

- [`lighting_rebuild.md`](lighting_rebuild.md) — the model, phases 0–8, and the
  backlog every defect above is filed in. Still the entry point for anything
  about light itself.
- [`silhouettes.md`](silhouettes.md) — the two edges, the seam inside the
  picture, the clamp, and the undecided S2.
- [`footprints.md`](footprints.md) — a static's box is the box the art drew.
  Landed, with the height as its own next census.

**Records — read one for its reasoning, not to find work:**

- [`occluders.md`](occluders.md) — the grid stopped being a rule. All six steps
  green; the four findings that outlive it moved into the rebuild's backlog.
- [`parity.md`](parity.md) — one frame however it was asked for. P1–P3 and P5
  landed; P4's remaining three items are geometry, which is
  [`footprints.md`](footprints.md)'s and the rebuild's ground.
- [`pixels.md`](pixels.md) — the six grids, all four phases done. The spec above
  is its normative half.
- [`lighting.md`](lighting.md), [`lighting_world.md`](lighting_world.md),
  [`lighting_raymarch.md`](lighting_raymarch.md),
  [`lighting_geometry.md`](lighting_geometry.md),
  [`lighting_height.md`](lighting_height.md),
  [`lighting_reference.md`](lighting_reference.md),
  [`gbuffer.md`](gbuffer.md), [`world_coordinates.md`](world_coordinates.md) —
  how the system being replaced was built, and why.

## The numbers to re-take

They are the ones every "how much of this is a crutch" argument is made from,
and they are all produced by tools in the tree
(`examples/geometry_census.rs`, `examples/discard_census.rs`,
`examples/footprints.rs`). Britain, `121×121` around `(1501, 1659)`, 11,184
statics:

| | |
|---:|---|
| 3.2% | a fitted prism — the only box whose *shape* came out of the picture |
| 39.6% | a lid — measured, `LID_THICKNESS` deep since P4.1 |
| 25.4% | panels on the edges the silhouette named |
| ~1.5% | a measured footprint, narrower than its tile (164 placements, new) |
| ~29.6% | **a whole tile, because the art would not say** — was 31.6% |
| 32.7% | of statics are a point of no primitive at all |
| 15.1% | of the world is a `CLEAR` piece handed a box with real height |
| 13.55% | of drawn static art misses its own box (7.82% with the roof cut) |

Re-run all three after anything on a backlog lands, and record the arguments
beside the answer — a census whose radius nobody wrote down has already caused
one contradiction between two documents.
