# Height as a continuous quantity

A fragment's height, and an occluder's, are integers today. Everything a
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

1. `pack_place` (`shaders/place_format.wesl:75`) writes
   `round(raw_z)` — eight bits, one unit a step. `mesh_face.wesl:100`
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
accounts for.

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

- `Builder::add`/`add_raw` return the `SolidId` they pushed. One static can
  push several (a body plus panels on named edges), so the caller keeps a
  set, not a scalar — this is the part that has to be designed rather than
  typed, and the run bookkeeping (`own_run`) is where it will show.
- The instance rows carry it: `MeshFaceRow` and the statics pass's own row
  gain the id, alongside the tile they already carry.
- `exemption` becomes a comparison of ids. `on_surface` loses its identity
  role and keeps only its geometric one, if any remains; `own_run`'s
  heuristic and `ON_TOP` as an *identity* tolerance go away entirely.
  `STAND_OFF` stays — it is about where a ray starts, not about who owns
  what.

Done when: the face oracle reads zero, `tests/lighting.rs` and
`tests/frame.rs`'s parity suite are green, and no exemption decision
anywhere reads a height.

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
on the GPU side, not the CPU walk. `OPENSHARD_TREE_H1=3.5` brings it down
(691/16384) but not to zero — the residual matches the same soft-edge
baseline the box-top oracle already reports against `walk_cells_exact`
(~5%), so it is not read as a second bug; phase 1's own "done when" already
expects a non-zero floor from the penumbra. Phase 1 next.

Open questions, deliberately not pre-decided:

- **One static, several solids.** A wall with two named-edge panels is
  several solids for one sprite. Whether a fragment names a solid or a
  *run* of them is phase 3's real design question; `own_run` exists today
  precisely because runs, not solids, are what must not shadow themselves.
- **Mobiles.** They are billboards with no solid at all, so phase 3's id
  has nothing to point at for them. Today they fall out of `exemption`
  through the same height guess as everything else; what replaces that for
  a billboard is unanswered.
- Whether `lighting_geometry.md`'s mesh occluder changes any of this. It
  should not — a mesh is a different *shape* test, not a different height
  representation — but the two tracks touch `ray_vs_solid` and should be
  read together before phase 2 moves.
