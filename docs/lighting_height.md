# Height as a continuous quantity

A fragment's height, and an occluder's, were integers when this was written —
phases 1 and 2 below are what changed that, and `## Status` is where it
stands. Everything a
shadow decides — where a ray starts, which box it enters, whether a solid is
the fragment's own — is decided from those integers. On a floor or a lid
that is exact, because a lid *is* at an integer `z`. On anything standing
up it is a lie: height varies continuously down a wall's face, and rounding
it to the nearest unit turns one surface into a staircase of one-unit
treads, each lit as though it were a whole unit higher or lower than it is.

This plan makes height continuous end to end, and then removes the one
place that only ever needed integers to paper over: `exemption`'s guess at
which solid a fragment belongs to.

## The defect this comes from

`examples/boxes.rs`'s `tree` scene draws a closed dark patch **inside** the
lower box's own south face — below the joint, below the top edge, lit above
it and lit below it. It is not the upper box's shadow: a shadow cast by
something standing on top of a face must touch that face's top edge.

Three facts, each verified against the tree rather than argued:

1. `pack_place` (`shaders/place_format.wesl`) wrote
   `round(raw_z)` — eight bits, one unit a step; phase 1 below is what
   replaced it, and the three facts here are as they stood before it.
   `mesh_face.wesl:100`
   hands it `in.world.z`, which is interpolated down a face and genuinely
   continuous; `statics.wesl` does the same for a wall's sprite. So every
   fragment of a vertical face reports one of four heights on a box three
   units tall, and `View::Height` shows exactly that: bands one unit apart.
2. Where the rounded height lands on a neighbouring solid's own base,
   `on_surface` (`blit.wesl:467`, `light.rs`'s twin) reads that solid as
   the fragment's own surface, and `exemption` (`light.rs:1247`,
   `blit.wesl:898`) drops it from the walk entirely. That is the **lit band
   under the top edge** — the upper box exempted from shadowing the face it
   stands on, because the face's top row of fragments rounded to the upper
   box's own floor.
3. One unit lower the exemption no longer fires, and the ray now starts a
   half unit below where the fragment really is, which is enough to send it
   into the upper box's `z` span instead of under it. That is the **dark
   band**, and the lit band below it is where a ray starting that low
   genuinely does pass beneath.

The control: `OPENSHARD_TREE_H1=3.5` moves the joint off an integer, and
the whole face comes back clean.

**What this is not.** The GPU is not blind to a sub-tile footprint — that
was fixed already, `Occlusion::footprint_bytes` (`occlusion.rs:1421`) and
`box_of` (`blit.wesl:722`) carry the box's horizontal extent to a
hundred-and-twenty-eighth of a tile. Horizontal geometry is exact and
vertical geometry is not, which is the whole of the asymmetry.

## Phase 0 — a gate on the old layer, before anything moves

`examples/boxes.rs` already runs two independent oracles, and **neither can
see this class**: the box-top oracle samples tops only (a top is flat, so
its height is an integer and rounding is exact there), and the ground
oracle samples the ground (likewise). The bug lives precisely where nobody
looks — on the vertical faces.

- A **face oracle** in `boxes.rs`: a grid over each box's own four vertical
  faces, each point projected through the scene's camera to the pixel the
  renderer actually drew, compared against `segment_clear_of_box` — the
  same fresh slab test the other two oracles already trust, no arithmetic
  shared with `light.rs` or `blit.wesl`.
- It **counts what it checked**, not only what disagreed. A face point that
  projects to a pixel some other face owns is skipped, and a skip is not a
  pass: the printed line carries sampled/compared/disagreeing, and the
  comparison count is asserted non-trivial. A detector that silently
  compares nothing reads exactly like a detector that found nothing.

This must go in first and must be **red** on `tree` before any of the
phases below start. It is what says a phase worked, and it is the only
thing that will catch this class coming back.

## Phase 1 — the fragment's height

Four spare bits sit in the `place` attachment's third channel: it is a
`u16` holding `z + 128` in the low eight and a four-bit stance at
`PLACE_STANCE_SHIFT = 8`, leaving bits 12..15 unused.

- Third channel becomes `z + 128` (8 bits) · **fraction (4 bits)** · stance
  (4 bits): `PLACE_STANCE_SHIFT` moves 8 → 12, and a new
  `PLACE_Z_FRAC_SHIFT = 8` / `PLACE_Z_FRAC_MASK = 15` names the middle.
  Sixteenths of a `z` unit — a `z` unit is four screen pixels at zoom 1, so
  a sixteenth is a quarter pixel, well under anything visible, and no wider
  format or second attachment is needed.
- `pack_place` splits instead of rounding: `floor` into the integer field,
  the remainder into the fraction. The existing clamp is unchanged.
- `blit.wesl:1454` reassembles `at.z` from both fields. `place.rs`'s
  mirror constants and its round-trip test move with them.
- The three producers (`ground.wesl`, `statics.wesl`, `mesh_face.wesl`) do
  not change: each already hands `pack_place` a continuous `f32`, and the
  rounding it applied was never theirs.

Done when: `View::Height` no longer bands on a vertical face, and the face
oracle's disagreement count drops to whatever the penumbra's soft edge
accounts for. *(Landed. The first half held; the second was the wrong
criterion — what is left is a hard disagreement at the foot of every face, not
a soft edge. See `## Status`.)*

**Why four bits and not eight.** Eight would need the stance moved out of
the channel entirely, into the id channels — real work, for precision
below a quarter of a pixel. If phase 3 lands, nothing compares heights for
*identity* any more, and a quarter pixel is comfortably enough for
geometry. Revisit only if phase 3 is abandoned.

## Phase 2 — the occluder's height

`Solid::bottom`/`top` (`occlusion.rs:601`) `round()` to `i32`, and
`solid_bytes` (`occlusion.rs:1341`) ships those two integers as bytes. For
a static off `tiledata` that is exact — its height is a `u8` and its `z` an
`i8`. For everything else it is not: `Builder::add_raw`'s arbitrary AABB,
a mesh face, a slope, a tread.

- A **`solid_z` plane** beside `footprints`: one `Rgba8Uint` texel a solid,
  the fractional parts of `bottom` and `top`, indexed and folded exactly as
  `footprint_bytes` already is. Same trick, same format, same WebGL2
  ceiling — the integer bytes stay where they are and keep meaning what
  they mean, so nothing that reads only them breaks.
- `box_of` (`blit.wesl:722`) takes the fraction into `lo.z`/`hi.z`, the way
  it already takes the footprint into `lo.x`/`hi.x`.
- On the CPU, `ray_vs_solid` already reads `solid.space` — exact `f64`
  corners. Audit the walk for every remaining `bottom()`/`top()` call and
  route each to the exact span; the integer accessors survive only for the
  upload and for the merged-column grid, and their doc comments say so.

Done when: `tree` at an integer joint and `tree` at `H1=3.5` agree with the
face oracle to the same tolerance, instead of one being clean by luck.
*(Landed — 278 and 235, the same shape on both. See `## Status`.)*

## Phase 3 — identity instead of coincidence

`exemption` (`light.rs:1247`) answers "is this solid the fragment's own"
with a **guess**: does the fragment's height fall inside the solid's span
(`on_surface`), and does the solid's edge mask miss the fragment's own side.
Both are proxies. The lower box's top and the upper box's base are the same
plane, in the same cell, and no amount of precision separates them by
height alone — the ambiguity is structural, and phases 1 and 2 shrink it
without removing it. The comment at `light.rs:1261` already says as much,
in the case where it fires for `Surface::Flat`.

A fragment knows exactly which solid it belongs to. It should say so.

### The fixture, first — `tree` cannot show this

`examples/boxes.rs`'s `tree` stacks its two boxes, so their `z` spans meet at a
single plane and a fragment of one is inside the other's span for exactly one
quantum of height. Once the oracles stopped lying (see `## Status`), `tree`
reads 18 of 7008 and every one of those is `STAND_OFF`'s nudge: **this phase
has no number to move there**. A phase measured against a scene that cannot
show its defect is a phase that will read green whatever it does.

So `OPENSHARD_BOXES_SCENE=pair` was built to show it, and does — two boxes of
one height side by side on one tile, on the tile's own diagonal so neither
covers the other on screen, the flame on the line through both centres and
beyond the near one. Every fragment of either box is inside *both* spans, which
is precisely what `exemption` reads as ownership, so the near box is exempted
from shadowing the far box's face while standing squarely in front of it.
Three oracles are red at once and all of them "both walks together" — no
precision or parity work can reach any of it:

| oracle, `pair` | before phase 3 |
|---|---|
| box 0's `east` face | 1296 / 1296 pixels |
| box 0's `south` face | 1248 / 1248 |
| box 0's own top | 9216 / 9216 — the `caps_this` arm, same guess |
| box 1 (the near one) | 0, correctly |
| ground | 147 / 254248 — the same nudge/tangent floor `tree` has |

### The design, decided

The three questions this section used to leave open are answered here, because
each of them decides a format and none of them can be deferred to the typing.

**1. Identity is the *thing that was added*, not the solid.** One
`Builder::add` — one static — is one owner, and every solid it pushes (a
corner's two panels, a stair's tread tops and risers, a body) carries that one
owner. That is what makes "one static, several solids" a non-question: the run
*is* the owner, and `own_run`'s bookkeeping has nothing left to approximate
within a tile.

**2. The key is the world thing, not a walk order.** An owner is
`(tile, the static's own z, its graphic)`. Not a counter the builder hands out:
`occlusion::bake` builds a *block's* solids once and pastes them into frames
for as long as the atlas revision holds, so any number that depended on the
order a frame's walk found things in would be a number from another frame. Not
a "the n-th static of this tile" index either, tempting as it is at 8 bits —
the two walks that would have to agree on it refuse different statics (the
occlusion side drops `opacity == CLEAR` and anything above the draw ceiling,
the draw side drops what the atlas has no art for), so the indices diverge
exactly where a tile holds something invisible.

**3. What rides on the wire is *which occluder of this cell*, one byte.** The
comparison is only ever made between a fragment and a solid **on the fragment's
own cell** — `lit_end` and `caps_this` are both `own_cell`-gated — so the id
does not have to be unique in the frame, only in the tile, and a tile holds at
most `MAX_SOLIDS_PER_CELL` = 255 of anything. `Occlusion::id_bytes` already
uploads a `SolidId` as three bytes of four, so the fourth byte of a *reference*
is where this goes: no new plane, no format wider than it is, and the value is
read in the loop that is already reading that texel.

Which leaves the join, and it is the one real cost: the pass that draws a
static has to learn the number the grid gave it. `Occlusion::owner_at(tile, z,
graphic) -> Option<u8>` answers it by scanning the cell (four solids, not four
hundred), and `statics::collect`/`items::collect` stamp the answer into the
instance row beside the tile. That means **the frame's occlusion has to be
built before its statics are collected**, which is a reordering in
`app::render` and not a change to either pass's logic — today the statics go
first for no reason anyone recorded.

- `Solid` carries its owner key on the CPU (three bytes, never uploaded) and
  its per-cell number for the upload. `Builder::add_raw` takes the key from
  its caller — a hand-built scene has no `tiledata` to derive one from, and
  inventing one inside the builder would be a second identity.
- `MeshFaceRow` and the statics pass's row gain the byte. A fragment with no
  solid at all — the ground, a mobile — stamps `OWNER_NONE`, which matches
  nothing.
- `exemption` becomes `stands.owner == fragment.owner`. `on_surface` keeps
  only its geometric role (does this ray's `z` lie in this solid's span, for
  `pierces` and the lid rules); `own_run`'s heuristic and `ON_TOP`-as-identity
  go away. `STAND_OFF` stays — it is about where a ray starts, not who owns
  what.

**Two things the design does not answer, deliberately.**

- **`flame_end`.** The other end of the ray is a flame, not a fragment, so it
  has no owner to compare: the arm that exempts the solid the flame is mounted
  on stays a height test for now. It is `mounted_at`'s question rather than
  this phase's, and worth its own entry once the fragment side is identity.
- **A run of wall across tiles.** `own_run` also answers a *second* question —
  a ray leaving a wall pixel along the wall grazes the neighbouring tiles'
  panels of the same wall, which are different statics and therefore different
  owners. That is not identity, it is a surface being cut on a tile boundary,
  and it stays until something measures it. The `pair` fixture cannot see it
  (one tile), so a scene that can has to come with the change that touches it.

Done when: `pair` reads zero on all three of its red oracles, `tree` still
reads 18/7008 and 226/252105, `tests/lighting.rs` and `tests/frame.rs`'s parity
suite are green, and no exemption decision about a *fragment* reads a height.

## Order, and what gates what

0 gates everything. 1 and 2 are independent of each other and both precede
3 — not because 3 needs their precision, but because 3 removes the code
that hides whether they worked. Each phase lands with the face oracle's
count in its commit message.

## Status

Phase 0 done: the face oracle lives in `examples/boxes.rs`
(`OPENSHARD_BOXES_FACE_ORACLE=0` to skip it), grids each box's own rendered
`east`/`south` face, and is red on `tree` as expected — 956/16384 compared
points disagree at the default `H1=3`, `light::sample` agreeing with the
independent oracle at every disagreement checked by hand (`through=1.000`,
"lit", against a rendered pixel reading "shadowed"), which places the fault
on the GPU side, not the CPU walk. It now also reports **where** up each face
its disagreements sat, as runs of grid rows — added in phase 1, because a
count alone cannot tell a defect made smaller from a different defect the
first one was hiding, and phase 1's residual turned out to be exactly that
distinction.

Phase 1 done: **956 → 278 of 16384** on `tree` at `H1=3`, and the shape says
what moved. Rounding was restored for one run to take the before-picture with
the same instrument, so these are one measurement apart and nothing else:

| face | before | after |
|---|---|---|
| box 0 east | 128, all in rows 0..3 (`z` 0.02..0.12) | 128, rows 0..3 — untouched |
| box 0 south | 746: 2 at the foot, **744 in rows 31..64** (`z` 1.48..2.98) | 68: 2 at the foot, 66 in rows 41..63 |
| box 1 east | 81, all in rows 0..4 (`z` 3.02..3.16) | 81, rows 0..4 — untouched |
| box 1 south | 1, at the foot | 1, at the foot |

So phase 1 removed 678 of the 744 points of the dark patch inside the lower
box's south face — the defect this plan opens with — and moved nothing at all
outside it. `View::Height` down one column of that face went from **4 distinct
values in runs of 15-17 pixels to 49, one step per pixel**; down box 0's east
face, 5 to 60. The banding is gone.

**The residual is not the penumbra**, and phase 1's own "done when" above
guessed wrong about that: `light::sample` reads `through=1.000` at these
points, a hard disagreement rather than a soft edge. 210 of the 278 are the
bottom one-to-five grid rows of a face — a fragment at the very foot of a face
being shadowed by the thing it stands on. That is `exemption`'s guess, phase 3,
and no amount of height precision reaches it; the ~5% soft-edge baseline the
box-top oracle reports against `walk_cells_exact` is a different measurement
against a different reference and should not have been borrowed as this
phase's floor.

`OPENSHARD_TREE_H1=3.5` **went the other way: 691 → 1103**, and that is
expected rather than a regression. With the fragment's height continuous and
the occluder's still rounded (`Solid::bottom`/`top`, phase 2), a box whose own
base is at 3.5 is uploaded as a solid spanning 4..7, so the bottom half-unit of
its own faces now sits *below* its own solid and stops being exempt from it.
Before phase 1 the two roundings cancelled and hid that. This is precisely what
phase 2's "done when" asks for — the two configurations agreeing rather than
one being clean by luck — and it now has a number to close.

Not from phase 1 and worth knowing before phase 2: at `H1=3.5` the box-top
oracle reads 3027/9216 and 9216/9216 against `light::sample`, a **CPU-side**
disagreement that no part of phase 1 touches (the `place` attachment is not on
that path). Same cause, one layer over: a solid whose `z` span is fractional.

Phase 2 done: the occluder's height is continuous end to end, and the two
configurations now agree instead of one being clean by luck.

| oracle, `tree` | `H1=3` before | `H1=3` after | `H1=3.5` before | `H1=3.5` after |
|---|---|---|---|---|
| face oracle | 278/16384 | **278** — identical, face by face and row-run by row-run | 1103/16384 | **235/16384** |
| box 0's own top | 0/9216 | 0 | 3027/9216 | **0** |
| box 1's own top | 0/9216 | 0 | 9216/9216 | **0** |
| ground oracle | 509/57600 | 509 | 1325/57600 | 574 |

*(Every face- and ground-oracle number in this table is from the old
instrument, and most of each is the instrument rather than the engine — see
the end of this section. The two box-top columns are unaffected: that oracle
never projected anything.)*

The integer column is the control and it does not move *at all*: at a whole `z`
every fraction this phase adds is zero, so the run is bit-for-bit the one before
it — which is what says the 868 points the fractional column lost were the
rounding and not a second change riding along with it. The two CPU-side box-top
numbers the last session flagged as phase 2's entry (3027 and 9216, a solid
whose span was rounded under a *flat* sample) are gone outright.

What is left, 235 and 278, is now **one shape in both**: about 200 points in the
bottom one-to-five grid rows of every face, and a band of 44–66 just under the
lower box's top. Both are `exemption`'s guess — a fragment at the foot of a face
shadowed by the thing it stands on, and the lower box's top being the same plane
as the upper box's base — and neither is reachable by precision at all. That is
phase 3, and the plan's own opening paragraph said so.

How it is carried, since the answer differs from what this section sketched:

- **The whole span, sixteen bits an end, and nothing left behind.** The plane is
  `Solid::z_bytes` — each end a `u16` in steps of a two-hundred-and-fifty-sixth
  of a `z` unit from `Z_FLOOR`, which is exactly the `-128 ..= 127` a map's own
  `z` lives in. `Occlusion::solid_bytes`' first two channels, which held the
  rounded span, are **zero**.
- **`span_of` is the only place in `blit.wesl` that turns the wire into a
  height**, and `light::wire_span` its CPU twin. A reader that decodes a height
  itself still compiles and still looks like a height, which is the failure
  phase 1 hit in `plan.rs`.
- `on_surface`, `pierced`/`pierces`, `crosses` and `box_of` take the span as a
  parameter on both sides now, so each walk supplies the one it is entitled to:
  `walk_cells_exact` the record's own `f64` corners, `walk_cells_streaming` the
  quantised one off the wire — the vertical half of the discipline
  `Solid::fraction` already stated for the horizontal one.
- The audit the phase asked for is done: no `bottom()`/`top()` call is left on
  either walk. What survives is the cutaway and `Occlusion::at`'s merged view,
  and each says so in its doc comment. `solid::standing`'s painter-order key was
  a third and is now the exact span — two boxes half a unit apart used to tie.

**And then the instrument turned out to be wrong, which retired most of the
numbers above.** The next session pointed the face oracle at what the renderer
had actually drawn instead of at a reconstruction of it, and the residual both
phases had been reporting mostly stopped existing. In full, because the shape
of the mistake matters more than the arithmetic:

- The face oracle gridded world points over each face and projected them to
  pixels. Whether the pixel belonged to the face it was asking about was
  answered by re-deriving every face's screen quad on the CPU, with a
  point-in-quad test and a painter's-order tie-break — a reconstruction that
  knew nothing about the ground pass. Half a pixel below a face's own base is
  the ground, correctly shadowed by the box, and that read as the face being
  wrongly shadowed: **212 of the 278**. Those are the "~200 points in the
  bottom one-to-five rows of every face" this section attributes to
  `exemption`'s guess above, twice, in two sessions' handoffs. They were the
  instrument.
- The oracle also asked about points the shader never lights: a pixel's
  fragment sits at the pixel's centre, and the attachment quantises what it
  carries. The ground oracle had known this since it was written and
  quantised by hand; the face oracle never did.
- What was left after both fixes was 43, and 27 of those were the *shader*
  alone: `blit.wesl`'s `RAY_TANGENT_TOLERANCE`, a cross-implementation
  rounding guard, was set to `1.0e-2` of a whole ray — about a screen pixel of
  world — so every box was a pixel fatter than its geometry wherever a ray
  grazed it. At a rounding-scale `1.0e-6` the whole parity suite is still green
  and the two tests the tolerance was introduced for fail identically, which
  they also did at `1.0e-2`.

The reference scene's honest residual is **18 of 7008 drawn face pixels**, all
of them `STAND_OFF`/`ON_TOP`'s deliberate nudge at a grazing corner — zeroing
the two constants on both walks for one run reads `0/7008`. None of it is
`exemption`. See `docs/lighting.md`'s "One scene is the reference" for the
current table and `a4b698c`/`ccca681`/`f050c2d` for the work.

The lesson is not that phases 1 and 2 were wrong — they moved `View::Height`
from four values to forty-nine down a face, and closed two CPU-side box-top
oracles outright, and those are real. It is that **a residual is a claim about
a cause, and this plan twice let a plausible attribution stand as one**: the
count moved the way the phase predicted, so the remainder was assumed to be the
next phase's. Nothing checked which side of the comparison was out until
something did, and then it was the side nobody had instrumented.

**Two things this phase got wrong first, and what they cost.** Both were found
by being asked whether the work was a workaround, which is worth writing down as
plainly as the result:

- The span shipped as **a rounded unit plus a signed fraction**, on the argument
  that `solid_bytes`' channels had to keep meaning what they meant "for a reader
  not taught about the new plane". There was no such reader: after the phase
  `blit.wesl` reads a height through `span_of` and nowhere else. The
  compatibility had nothing to be compatible with, and it bought a second
  concept (a fraction *of* something), a second clamp, and a rounded copy of a
  number living better elsewhere — the exact shape of a format growing a field
  nobody dares change. Replaced by the whole span above; every oracle number in
  the table is identical either way, so this was cost without effect.
- The three `walk_cells_streaming_agrees_with_walk_cells_exact_*` tests were
  **blind to what this phase introduced**. They build every fixture through
  `Builder::add` off a `StaticTile`, so every span in them is a whole `z` — and
  the two walks now read *different* heights for one solid on purpose, which on
  a whole `z` are equal by construction. Mutating `wire_span` back to the
  rounded span leaves all three green; only the fractional-`z` body added here
  goes red. A fourth test, and the mutation is what says it earns its place.

Phase 3 next, and its own section now carries a decided design and a fixture
that is red for it. The three questions this section left open when phase 2
landed are answered there:

- **One static, several solids** — the owner is the static, so its solids share
  one, and there is no run to name.
- **Mobiles** — a billboard has no solid, so it stamps `OWNER_NONE` and is
  exempt from nothing. That is the honest answer and it is a *behaviour
  change*: today a mobile standing on a walled tile is exempted from that wall
  by the same height guess as everything else. Worth a look at a real frame
  when it lands, not a preemptive tolerance.
- **`lighting_geometry.md`'s mesh occluder** — read, and it changes nothing
  here: a mesh is a different *shape* test against the same `ray_vs_solid`,
  and identity is about which occluder a fragment came from, not what shape it
  is. The one line of that doc which does bear on this track is its warning
  that vertex data fits a fixed-size `Rgba8Uint` grid worse than a box's six
  numbers — which is why phase 3's own byte goes in a plane that already
  exists rather than in a fifth one.

## Backlog

Picked up while phases 1 and 2 landed and while the oracles were repaired; none
of it blocked any of them.

- **`STAND_OFF`/`ON_TOP` are the reference scene's whole residual, and nobody
  has priced them.** Zeroing both on both walks takes `tree`'s face oracle from
  18/7008 to 0 and its ground oracle from 226 to 137. They exist for a reason
  that is written down and measured (a wall wore a bright stroke along its
  floorboards without them), so this is not a proposal to remove them — it is
  that "how far off its own surface a ray starts" is a number chosen once, in
  units of the *attachment's* quantisation (`2/127` of a tile, `1/128` of a `z`
  unit), and what it costs at a grazing corner has never been looked at. A
  smaller nudge, or one scaled to the surface rather than to the format, might
  cost nothing.
- **The wire's span rounds to nearest, so a solid can be a hair *shorter* than
  it is.** `Solid::z_bytes` rounds each end to the closest step, so
  `walk_cells_streaming`'s box can be smaller than the record's on either end —
  and a smaller occluder is a shadow with a hole in it, which is the one
  direction of error the rest of this pass takes care to avoid (`z_bytes`'s own
  clamp says so in words: "it stops at least what it really stops"). Rounding
  *outward* instead — floor the base, ceil the top — costs one more step of
  span and buys a one-sided property: the wire box always contains the exact
  one, so `walk_cells_streaming` can never let through what `walk_cells_exact`
  stops. That is a stronger claim than the numeric agreement the parity tests
  assert today, and a cheaper one to hold at a tangent.
- **The exact-tangent case is a definition, and the two sides differ.** The
  other 137 ground pixels are rays that touch a box's corner at exactly one
  point. `light::ray_vs_solid`'s doc says a zero-length crossing is the
  caller's decision and then no caller decides it; `boxes.rs`'s independent
  oracle counts it as blocked. One of the two should move, and which is a
  question about what a hard shadow's corner should look like.
- **`examples/two_cubes.rs` still carries the old oracle idiom.** It projects
  world points and reads pixels without asking the `place` attachment whose
  pixel it got — the same blindness `boxes.rs` just shed, in a tool that is
  still used to answer the same kind of question.

- **`tests/cost.rs`'s "what the upload sends" measures three planes of five.**
  Its `black_box` sums `bytes` + `field_bytes` + `id_bytes` + `solid_bytes`, and
  has never included `footprint_bytes`; `solid_z_bytes` is now a second one
  missing. A cost line that names most of a thing reads as the whole of it.
- **`plan::Wall::top` is an `i32` the caller invents**, so an elevation of a wall
  standing to `z 3.5` is drawn in a frame four units tall with half a unit of
  nothing at the top. Only `tests/pictures.rs` builds one today, always at a
  whole `z`, so nothing is wrong on any picture that exists — but the field is
  the picture's own vertical extent and there is no reason for it to be whole.
  (An earlier draft of this entry said the value came from `Occlusion::at`'s
  rounded `Cell`. It does not; it is a parameter.)
- **`Occlusion::at`'s `Cell` is still whole units.** Its three readers are the
  wireframe, the plan view and `mounted_at` (which reads `edges` only), so
  nothing that decides a shadow reads it — deliberately left, and worth
  revisiting if a fourth reader ever wants a height rather than a picture.

- **Two hand-copies of the third channel are left**, and both are correct
  today only by accident: `tests/select.rs`'s `place_texel` and
  `tests/frame.rs`'s parity-fixture builder each fold `(z + 128) | stance <<
  STANCE_SHIFT` themselves, and each happens to pass an integer `z`, so the
  fraction they never write is zero. `place::packed_height` is what they
  should go through. A third copy — `plan.rs`'s elevation picture — was the
  one that *did* bite: an instrument with its own copy of the format rounded
  the height and drew, in the diagnostic meant to show a wall's face, the
  very treads this plan is about.
- **The face oracle's projection idiom is now stated five times** in
  `examples/boxes.rs` (box-top oracle, ground oracle, the main mesh dump, the
  face oracle, and its `ScreenFace` corners): `camera.to_view_exact(
  project_exact(..))` with `projection.origin`/`.scale` applied by hand. One
  named function, once.
- **`mesh::Face` and `facing::Face` collide by name** inside one crate, and
  `boxes.rs` aliases one of them (`as WallFace`) to say which it means. Not
  phase 1's business, but the next file that needs both will pay it again.
- **The `owned_by_someone_nearer` tie-break has never executed.** Its
  `f.depth == box_depth[i] && f.box_index > i` arm needs two faces at equal
  depth with overlapping silhouettes, and no scene here produces that. It has
  been read against `renderer.rs`'s `LessEqual` and nothing more.
