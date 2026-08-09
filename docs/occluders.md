# The occluders — one shape per surface, and no tile in the answer

A multi-session refactor with its decisions already made. **Nothing below is an
open question**; where a choice had alternatives, the choice is written down with
the reason, and the alternatives are recorded so they are not re-opened rather
than re-argued. A session that starts here starts at the first step whose gate is
not yet green.

`docs/lighting_rebuild.md` phase 6e is the one-paragraph version and points here.
This continues `docs/lighting_geometry.md`'s question — box occluders becoming
real geometry — with the part that document never had: a reason, a measurement
and an order.

## What we are fixing

**The ragged boundary between solids on neighbouring tiles.** Holes, fringe, and
stair-stepping at a tile edge, in everything that reads the occlusion geometry:
the shadow walk, the impostor's positions and normals, and every debug view that
draws either.

**Done when: on neighbouring tiles there are no holes, no fringe and no
stair-stepping between solids.** That sentence is the acceptance criterion, and
§ *The detector* turns it into a number a run can print and a gate can fail on.

**Which step delivers it: S3**, and it is the only one that moves this number.
S1 lifted the ceiling that made the shape unstateable, S2 is the ruler, S4/S5 and
S3b are deletions and optimisations that must move nothing. **S3 has since landed
and moved nothing either** — its exemption needs a ray in the surface's own plane
and the renderer has none, which is measured at S3's own acceptance. The seam a
person reports is a *shading* defect and it belongs to
[`lighting_rebuild.md`](lighting_rebuild.md)'s **phase 5b**, which is where this
track's next session starts; nothing here is blocked on it, and everything here is
easier to judge after it, because the below-horizon rays it removes are the ones
that make a wall's own run look like an occluder question. So acceptance for the
sentence above is § *Acceptance for S3* — six things to run, each with a figure
to read, none of them resting on anybody's description of a picture.

## Why it is ragged — the root, measured

**A primitive is a tile's.** Not by any argument about geometry; by the shape of
the storage. Three consequences, each already seen in a frame:

1. **A primitive's coordinates on the wire are `tile + byte/255`.**
   `occlusion::Solid::box_from_footprint` rebuilds a box from a cell and four
   bytes of sub-tile fraction, so a primitive **cannot express a shape wider than
   one tile**, and its corners are quantised to a two-hundred-and-fifty-fifth of
   one. The *record* — `occlusion::Solid`, two `camera::WorldSpot`s of `f64` — is
   already absolute and exact. It is the upload that folds it back onto a cell,
   and `light::walk_cells_streaming` mirrors that quantisation on purpose, which
   is why the two CPU walks read different heights for one solid by design.
2. **One physical surface is N primitives with N−1 internal seams.** A run of
   wall is one wall to the artist and N statics on N tiles to us. A storey's
   floor is one slab and one box a tile. Every internal seam is a place where two
   boxes meet, where a fragment can stand exactly on the join, and where the
   silhouette steps at tile granularity.
3. **Rules are stated in cells to paper over 1 and 2.** `same_run` exists because
   a run is N solids. `starting_cell` exists because a fragment's position and
   its instance's tile can disagree. The vertical shortcut reads one cell. The
   per-cell `max` exists because two panels of one corner are two boxes of one
   wall. Four rules, all standing in for a shape that is not stated.

   **And for a *body* not even that** — a correction to this list, measured after
   it was written. `same_run` is a run of **panels** along a row or column; the
   walk's `edges == EDGE_MASK` branch never asks it. A climbable's treads are
   declared as bodies (`occlusion.rs`'s own test: *"a tread is a body: a stair is
   solid"*), so a flight of steps has **no surface exemption of any kind** —
   only `mine == reference.x`, one primitive. So point 2 is not merely papered
   over here, it is bare, and § *The flight seams* is what it looks like on a
   frame. It also weakens D5's own order: `same_run` cannot be load-bearing for a
   shape that never reaches it.

Measured, on one real place at 4:1, before any of this: 474 fragments stood
strictly outside their own carried tile and **324 of them leaked a fully lit
pixel into a shadow**; the narrow leaks over one building's floors numbered 303.
`starting_cell` took that to zero, and `docs/lighting_rebuild.md`'s backlog
records that it is a repair rather than a construction — it arbitrates between
two spellings of one fact instead of removing the second spelling. This plan
removes it.

## The decisions

Made, not to be re-opened. Each carries the reason and, where there was one, the
alternative it beat.

**D1 — geometry is absolute world coordinates, everywhere.** The wire carries a
primitive's own `min`/`max`, not a cell and a fraction of it.
`box_from_footprint`, `footprint_bytes` and `wire_span` go. No tile is the base
of any coordinate. *Rejected:* widening the fraction to sixteen bits — it keeps
the ceiling that a primitive is a tile's, which is the whole defect.

**D2 — a fragment is exempt from its own *surface*, not from its own primitive.**
*Rejected:* leaving the seam and widening the rules that hide it, which is
`WIDTH_OVERLAP`'s own family and what `docs/style.md`'s *No fudge constants* was
written from. Also rejected: an `ε` along the normal — the classic shadow bias,
which this renderer already owned as `STAND_OFF`/`ON_TOP` and already deleted.

**The defect this names.** A shadow ray starts *on* the surface, so it meets that
surface at `t = 0`. The textbook has two cures — offset the origin, or exclude
the primitive the ray came from — and this renderer took the second, correctly.
But the exclusion is spelled `mine == reference.x`: **one** primitive, where one
physical surface is N of them. The ray leaves its own box and enters the
neighbour's a thousandth of a tile later. It is the mesh tracer's own
self-intersection bug, one level up: excluding the source *triangle* does not
save a ray that grazes into the next triangle of the same polygon.

**The rule, and it is a theorem rather than a heuristic.** A primitive is
axis-aligned, so each of its faces lies at its own extremum — the whole box is
therefore in the closed half-space **behind** the plane of that face. So for a
fragment on that plane with outward normal `N`, and any ray with `d·N > 0`: the
ray leaves the half-space at `t = 0+` and never returns, and **no primitive whose
face lies in that plane facing the same way can ever occlude it.**

> Skip a candidate exactly when its extent along the fragment's own normal axis
> **ends at the fragment's own plane, on the fragment's back side** —
> `candidate.hi[axis] == plane` for an outward `+`, `candidate.lo[axis] == plane`
> for a `−`.

That set is provably empty of true occlusions, which is the whole difference from
a bias: `ε` trades acne against peter-panning, this discards nothing. The other
two cases close themselves — `d·N < 0` is a light behind the surface and `N·L` is
already zero there, and `d·N = 0` is measure zero and is precisely the graze the
exemption exists for.

⚠ **That middle clause was false when it was written and phase 5b is what makes
it true.** "`N·L` is already zero there" is a statement about the *shaded* frame;
the shadow term is compared without a cosine on either side, so a ray behind the
plane was traced and its crossing was real — which is why S3 had to take the ray's
direction as a parameter (`d·N >= 0`) rather than wave the case away, and why
`same_run` is broader than the theorem. Once every sample carries its own cosine
the clause is true by construction: a sample behind the plane is not traced at
all.

**What it subsumes**, so the step is a deletion rather than an addition:
`mine == reference.x` (a fragment's own box ends at its own face — the special
case), `same_run` **with** its row/column cell test and its `on_surface` height
gate (a run of wall is coplanar same-facing panels), and — to be measured —
`ray_vs_solid`'s zero-length graze rule, which exists today for exactly this
reason and says so.

**Two halves, and the order between them is the decision.**

- **D2a — the identity.** The rule above. It moves no geometry and needs no
  merge. ~~**This is what cures the seam.**~~ **It is not — measured at S3, and
  neither is D2b.** The exemption is reachable only by a ray lying in the
  surface's own plane, and the shipped renderer has no such ray: S3 moves **0
  pixels** on the flights, 0 of 29,696 on the wall run, 0 of 262,144 on the stair
  under a front light. What cures the seam is `docs/lighting_rebuild.md`'s phase
  5b — see this plan's backlog, where all three arguments are written out. D2a is
  the rule that says *why* a surface may not shadow itself, and that is worth
  having stated whether or not a frame today can reach it.
- **D2b — the merge.** Contiguous same-surface neighbours become one box at build
  time. A **pure optimisation** once D2a holds: fewer primitives, no pixel moved.
  Last, not first.

*Reversed from this plan's first draft*, where the merge was the premise and the
identity fell out of it. The reason is measured: the merge is what forces a
primitive wider than a tile, which is what forces the hierarchy (D3) and breaks
the grid's superset property — so making it the premise buys a seam fix at the
price of three other steps. And a *derived* identity cannot work for a lid at
all: `edges == 0` gives `own == 0`, so `same_run` is unconditionally zero and a
floor gets no exemption in principle.

⚠ **Honest status of the theorem.** It was derived in the session that measured
the run of flights, not when this plan was written, and its one soft spot is
float equality: the fragment's plane arrives interpolated from the rasteriser and
the candidate's box from the storage buffer. If those are not bit-identical the
temptation is a tolerance, and the answer is **not** a tolerance but removing the
second number — carry the plane from the instance row, which already carries
`solid`. That has to be measured, and S3's gate is where.

**D3 — the broad phase is a bounding volume hierarchy.** A tree of axis-aligned
boxes over the primitives; a ray that misses a node skips its whole
subtree. The uniform tile grid goes. *The reason is D2b and not speed:* a uniform
grid must list a primitive in **every cell it spans** or it stops being a
superset, so the more a surface merges — which is the point — the worse a grid
fits it. A grid likes many small primitives of one size; a hierarchy likes few
large ones of different sizes, which is what a merged world is.

**D4 — the broad phase may not change the answer.** It returns a *superset* of
the primitives a segment might meet; the answer is `ray_vs_solid` over that
superset and nothing else. Every tuning knob in the hierarchy — leaf size, split
rule, node budget — is therefore a cost knob that **cannot** move a pixel, and
that is a property to be gated rather than asserted: see § *The oracle*.

**D5 — the cell disappears from every rule.** `same_run`, `starting_cell`, the
vertical shortcut and the per-cell `max` are deleted. The first two because D2
removes what they stand in for; the last two because they are statements about a
cell in a pass that no longer has one. Each goes only after its own measurement
says nothing depends on it — see § *Steps*, S4.

**And `same_run` is licensed by D2a rather than by the merge**, which is what the
reversal above buys: the identity replaces it outright, so S4 no longer waits on
a build-time transformation of the geometry. It is also less load-bearing than
this plan first assumed — the walk's body branch never consults it at all, so for
a climbable it was never holding anything up. See § *Why it is ragged*, point 3.

**And so does every *scan* of a cell**, which is the same defect wearing a
different coat: `blit.wesl`'s `own_solid` walks a cell's list to name the solid a
sprite fragment is a point of, and `occlusion::owner_at` is a linear scan of one
too — `docs/lighting_rebuild.md`'s backlog has both, and counts **thirteen scans
of one cell for a four-tread flight**. Under D6 the answer is carried: the
primitive a fragment met is the primitive it is a point of. They are in scope
here and land in S4 with the rest.

**D6 — the impostor meets the merged primitive its instance is part of.** Phase
6c made a fragment's shape a property of its own instance, and `occlusion::Part`
is the join from an instance to the solids it pushed. After merging that join
points at the merged solid, so two neighbouring wall sprites are met against
**one continuous box**. The sprite is still drawn per instance and the silhouette
is still the art's; only the volume behind it becomes continuous. **This is the
reason the acceptance criterion is reachable at all**: position and normal stop
being able to jump at a tile edge, because there is no edge in the volume.

**D7 — the map's tile keeps its own job, which is placement.** A static arrives
at `(x, y, z)` on a tile; that is the wire and this plan does not touch it. The
tile-to-world mapping — the arithmetic that decides which world coordinates a
tile's corners are, and how to state it so that no reader needs a `floor` that
can land on the wrong side — is a **separate task** and explicitly out of scope
here. What this plan does is remove every reader that needed such a `floor` in
the first place.

**D8 — the wire is storage buffers, not textures.** The grid's texture encoding
(`occluders`, `footprints`, `solid_z`, the reference lists) predates the
allowance; `blit.wesl` already reads eleven storage buffers and phase 6a settled
that the crate's ceiling is WebGPU. A primitive is a struct and a node is a
struct, and neither should be spelled as channels of a texel.

**D9 — `z` stays in `z` units on the wire.** Phase 2 decided this deliberately:
the occlusion set, every span and every walk are stated in them, and a wire that
alone counted in tiles would be a second metric. Not re-opened.

**D10 — `f32` on the wire, `f64` in the record.** The record is authored and
merged on the CPU where exactness is free; the wire is what a shader can read.
The gap between them is a thing to *measure* (the oracle below runs on the wire's
own numbers), not to hide.

**D11 — tests are the deliverable.** Every step lands with a gate that has been
**fault-injected to red** before it is trusted. A step whose suite is green with
its own change reverted has not landed; it has been written. This is the
discipline phases 4, 5 and 6 already used and the reason each of them has a
number in it.

## How this sits in `lighting_rebuild.md`

Checked against it line by line rather than assumed, because that document is the
entry point and a plan that quietly contradicts it is worse than no plan.

**What it fulfils.** Two of its own promissory notes are this plan: `own_run`'s
row says the rule is retired "when a run becomes one solid", and
`lighting_geometry.md`'s row says the generic form of box-into-real-geometry
"continues" at `facing::Blocks` — an authored list of up to four boxes, written
and wired to nothing. **D1 is what makes `Blocks` wireable**: a shape of up to
four boxes cannot be uploaded at all while a primitive's coordinates are a
tile's, so the carried-over item "`Builder::add` consuming an authored `Blocks`
list" becomes available here rather than needing its own fight. It is not in
this plan's steps — it is content, and it stops being blocked.

**What it supersedes, and those documents now say so.** `MAX_WALK_STEPS`, which
"survives" in the *What goes* table, bounds cells stepped and is replaced by a
node budget in the same role. `lighting_raymarch.md`'s row read "survives as the
walk"; the walk it means is the DDA, and S5 retires it — what carries over is
`ray_vs_solid`, which was never about cells. And the corner-tie CPU/GPU parity
gap was listed under *Known gaps that outlive the rebuild*: it does not, because
a corner tie is two backends disagreeing about which **cell** a ray crosses
first, and there will be no cells.

**What governs this plan and is not restated in it.** *How this is judged* still
holds: **the instrument is a picture beside the path tracer's, looked at by a
person.** The census and the brute-force oracle below are detectors, and a
detector is what catches a defect between two lookings — neither replaces the
frame, and no step here is finished on a number alone.

**What it must not disturb.** Phase 4's self-shadow rule is identity between
primitives, and merging changes which primitive a fragment is a point of. The
three tests that go red when the identity compare is neutralised must stay red
under that injection through every step — a merge that quietly made a fragment
exempt from a genuine occluder would be trading this plan's defect for a worse
one. S4 states it as a gate.

**Where it sits in the order.** Phase 6d — the mesh pass coming off real statics
and gaining a colour target, which is phase 2's albedo — is still open, and the
two do not collide: 6d is about *drawing*, this is about the occlusion geometry
behind it. One place they touch, settled here so no session has to work it out
again: **a flight's treads do not merge.** S3b's rule requires an equal span, and
three treads are three heights by construction. A flight stays three primitives,
which is what its shape is.

## The detector

The acceptance criterion has to be a number a run prints and a gate can fail on,
or it is a hope. Two instruments, and they answer different halves.

**The seam census — the DoD itself.** Over a rendered frame: for every pair of
horizontally or vertically adjacent fragments that lie on **opposite sides of a
tile boundary** and belong to the **same primitive**, their shadow answer must
agree to within one shadow ray (`1 / SHADOW_RAYS`), and their normals must be
equal. Count the pairs that do not. **The DoD is zero**, and the count is printed
every run of `examples/isolated_scene.rs` beside the box census phase 6c added.

Two things it must report and not just use: how many pairs it *examined* — a
census that examined nothing passes — and the breakdown by what disagreed
(visibility, normal, or the fragments naming different primitives where the
geometry says one).

**Its own before-number is taken at S2, before anything is merged.** A detector
built after the fix has never been seen to fire; this one is built while the
defect is still there, and its first reading is the thing S3 is measured against.

### Reading the dump, in numbers rather than by eye

`tests/traced.rs` and `examples/boxes.rs` both write
[`Verdict::strips`](../crates/client/render/examples/oracle/pathtrace.rs) when
their dump variable names a directory: the frame's own shadow decision, the
tracer's, their difference, **why an uncompared pixel was not compared**, and
**which solid the frame drew**, one colour a body. `tools/mask_probe.py` reads
that composite back — an overlay of the shadow onto the body map, the
neighbourhood of one pixel as text, and a seam census across the joins.

**It exists because every wrong reading on this track came from looking instead
of measuring.** A dump older than the fix, read as a live lighting fault. A mask
laid over a picture from the *other* tool and so placed one tile east — the tool
centres on the scene's own tile bounds, the gate on a named tile, and for a run
of three flights those differ by one. A composite sliced by `width // 3` and read
as a three-pixel camera offset, when the slice was off by the ruler. Each of
those is a question with a numeric answer.

### The flight seams: **a continuous wall shadows itself, once per primitive**

The run of flights shows a hard shadow step landing exactly on the join between
two primitives of one continuous riser wall, which reads as "each new primitive
starts its shadow at its own corner". **It is that, and the mechanism is D2's own
argument, now with a fixture and numbers under it.** What it is *not* is anything
about draw order or precision, and ruling those out is what the measurement did.

The probe, in world coordinates, at the join across `x = 101`:

```
(299, 225) frame: box 0's FaceSouth at (100.992, 100.333, 4.333) shadowed
(300, 225) frame: box 3's FaceSouth at (101.008, 100.333, 4.417) lit
```

One plane, `y = 100.333`. One flat wall, `x` from 100 to 103, `z` 0 to 5, in
three primitives. The fragment at `x = 100.992` sends its segment toward a flame
that sits a sixth of a tile **behind** that plane, so the segment goes into the
wall: out through its own solid, which is exempt, and straight into box 3, which
is not — box 3's west face is nine thousandths of a tile away. The fragment nine
thousandths further east has box 3 as its *own* solid, and its next neighbour is
a whole tile off, by which point the segment has climbed past `z 5` and clears.
One tooth per primitive, and the tooth is exactly a tile wide.

Three things this pins down, each of which had to be measured rather than argued:

- **Not draw order.** Shadow is a deferred pass over the G-buffer; the order faces
  were drawn in decides which surface owns a pixel and nothing else. Reversing the
  flights changes nothing at all — they are three different tiles, so they are
  three different depths and there is no tie to break. Measured: identical
  verdict, `261682 / 0 / 11 / 462`, to the pixel. Reversing the *treads* within a
  flight does change 7,016 pixels, and that is a tie, but it is a tie about
  visibility and not about light — and it is a fixture artefact besides, since
  `box_mesh` gives every box a full-height riser where `Prism::mesh` builds each
  riser exactly between two treads.
- **Not precision.** The path tracer, which has no cells, no tiles and no walk,
  draws the identical picture: 261,682 compared, 0 in the interior, 11 on an edge.
  It agrees because it is handed the same nine boxes — which is the point. The
  model is what is wrong, not the arithmetic over it.
- **And it is live, not a fixture's invention.** `Builder::add` declares a
  climbable's treads as **bodies** (`edges == EDGE_ANY`, asserted in
  `occlusion.rs`'s own test: *"a tread is a body: a stair is solid"*), exactly as
  `add_raw` does here. The walk's body branch never consults `same_run` at all —
  that exemption is a run of *panels* along a row or column — so for bodies there
  is no surface exemption of any kind. Only `mine == reference.x`, one primitive.

**What the cosine hides, and what it will not.** In the shaded frame this costs at
most 43 steps of 255, mean under 9, over some 120 pixels: the wall here is
back-facing, and `N·L` darkens it whatever the visibility says. That is luck of
the arrangement. Turn the flame to sit just *in front* of the plane and the same
per-primitive exemption produces acne on a lit surface, where nothing downstream
saves it.

**The fixture for that is a run of *bodies*, and a run of wall cannot be it** —
recorded here because S3 spent an hour finding out. `scene::wall_run_lit_from_along_it`
*is* drawn, by `pictures.rs`'s `a_wall_lit_from_one_end_has_no_dark_stroke_at_its_seam`,
and it is green both before and after S3: a wall is panels, so its seam lands on
`same_run`, which has covered it since phase 4. A body had no exemption at all, which
is why the defect is a body's — so the fixture is nine treads and the gate is
`lighting.rs`'s `a_landing_cut_into_three_primitives_is_not_shadowed_by_its_own_pieces`.
See S3's acceptance, point 2.

## The oracle

**Brute force over every primitive in the scene.** No hierarchy, no cells, no
early exit: `ray_vs_solid` against the whole list, in the wire's own numbers.
That is the one non-circular check available — it shares no traversal with the
walk it judges — and D4 is exactly the claim it makes machine-checkable: the
walk's answer equals the brute-force answer for every ray, whatever the tree
looks like.

It is also what makes every knob in D3 safe to turn: a leaf size or a split rule
that changes a pixel turns this red.

`tests/lighting.rs`'s `brute_force_blocked` and `frame.rs`'s
`ground_truth_blocked` are the existing shape of this and stay — they are dumb
fixed-step point samplers, which is a *different* dumbness and worth keeping
beside an exact one.

### **"No cells" is the load-bearing word, and it cost a day to learn why**

Both samplers looked their boxes up by `solids_at(floor(x), floor(y))`. Everything
else about them was brute force — fixed steps, no DDA, a point-in-box test in
`f64` — and that one line made them not brute force at all: it is the walk's own
indexing with a slower loop inside it, and it inherits the one thing indexing can
get wrong.

**A point on a box's own `max` face floors into the next cell, which does not list
that box.** So a sampler standing *inside* a solid is handed an empty cell and
reports open ground. That is the whole of the corner graze pinned on 2026-08-09
and resolved on the same day (§ *Backlog*), and the damage it does is worse than
being wrong: an oracle arbitrates, so a wrong oracle convicts whichever walk was
right. Both walks were called defective for a day over a wall they had read
correctly.

So the rule, and it is not a preference:

> An oracle iterates **every primitive in the frame** — `Occlusion::solids()`,
> which exists for this and says so — and states its tile exemptions as
> **volumes**, closed on both sides, so a point on a boundary is exempt from both
> columns rather than assigned to one by `floor()`. After the repair there is no
> `floor()` left in either sampler.

**Nor was the step size ever the culprit here, and that matters** because it is the
thing this oracle *has* been patched for twice: the clip in question is `0.000225`
of a tile deep, larger than `BRUTE_STEP`, and the march really did land a point in
it. `the_pinned_corner_graze_is_blocked_and_all_three_oracles_say_so` asserts that
depth against `BRUTE_STEP` for exactly this reason — if a later fixture makes the
sliver thinner than the step, that test says so instead of quietly becoming a
resolution story again.

**And when a sampler and a walk disagree, neither of them is the arbiter.** A
fixed-step sampler can be defeated by a thin enough sliver at any resolution, so
the tie-break is an exact segment-versus-box test: `segment_inside_box` in
`tests/lighting.rs`, the textbook slab test in `f64`, written out in the test's own
arithmetic rather than calling `solid::ray_vs_solid` — being held to the crate's
own slab test would be the crate agreeing with itself. It answered the pinned case
in one run. Reach for it first the next time this shape appears.

## Steps

Each is landable alone and leaves the tree working. A session starts at the first
one whose gate is not green.

**S1 — absolute coordinates on the wire.** D1. ✅ **Landed.** The reconstruction
and the quantisation are gone; a primitive carries its own six numbers.
`walk_cells_streaming` no longer previews a quantisation that does not exist,
which collapses the documented difference between the two CPU walks to an `f32`
rounding. **The ceiling that a primitive is a tile's is lifted here**, and
nothing can merge before it is.

What it took, so a reader does not have to diff for it:

- `Solid::fraction`, `Solid::z_bytes`, `Solid::span_from_bytes`,
  `Solid::box_from_footprint` and `Solid::Z_STEPS` are deleted, and
  `Solid::wire_box` — the record's six corners through `f32` — replaces the lot.
  It is the **only** place the wire's rounding is stated, so the upload and the
  walk that previews it cannot disagree.
- `Occlusion::solid_bytes`, `footprint_bytes` and `solid_z_bytes` — three
  planes, three encodings of one box — become `Occlusion::primitive_bytes`: one
  32-byte struct a primitive, `(lo.xyz, flags, hi.xyz, opacity)`, in a **storage
  buffer**. That is D8 arriving with D1 rather than after it: an absolute
  coordinate does not fit in a channel of an `Rgba8Uint` texel, so the two were
  one change.
- `blit.wesl` loses `box_of`, `footprint_at`, `span_of`, `SolidBox` and the
  `SOLID_Z_STEPS`/`SOLID_Z_FLOOR` pair. `solid_at(id)` is an array index and
  returns the whole primitive; a box is two fields rather than a reconstruction.
  Bindings 13 and 14 are freed and the G-buffer's two planes move down into them.
- `Z_FLOOR`/`Z_CEILING` no longer bound a *span* — a spire through the top of the
  world reaches its own height on the wire now. They are the `Aperture`'s alone,
  which makes a hole's two whole-unit ends the last quantised number in the pass.
- `solid::drawn` stops clamping a drawn box's `z` into an `i8`. It did that to
  draw where the *shader* believed a box was rather than where the map said, and
  with the pin gone from the wire the clamp had become the one thing an
  instrument may not be — a picture of somewhere the renderer is not. **Nothing
  in the suite went red when it was removed**, which is the honest state of that
  view: the rule was never gated.

*Gate, as built:* `light::a_primitive_at_no_fraction_a_byte_could_name_reads_the_
same_three_ways` — a box whose every face sits **half a step** off the byte grid
the old wire measured on, which is the point that grid is maximally wrong about,
with twelve rays aimed parallel to its faces a half-thousandth of a tile to
either side. Both CPU walks and a brute-force oracle over every primitive (no
cells, no traversal shared with either walk) must give one answer to each.
`frame.rs`'s `the_shader_reads_a_primitive_at_no_fraction_a_byte_could_name` is
the shader's third of it, on **the sun** rather than a flame: eight rays at a
sphere spread by `FLAME_RADIUS * t` at the crossing, forty times the half step
being aimed inside, so a flame cannot resolve this fixture at all and a single
directional ray can.

*Fault injection, run:* the `/255` rounding put back in `Solid::wire_box` turns
the CPU gate red (the exact walk against the oracle, on the first ray);
put back in `Occlusion::primitive_bytes` alone — the wire and nowhere else — it
turns the shader gate red, both frames sunlit where one must be shadowed.

**S2 — the detector, before the fix.** § *The detector*, built and read. Its
first number on a real place is recorded here, in this document, as the thing S3
moves.

*Gate:* the census fires on today's tree (it must — the seams are there), reports
how many pairs it examined, and its synthetic twin runs under `cargo test` on a
scene with a known seam.

**S3 — the surface exemption. ✅ Landed 2026-08-09.** D2a, and nothing else: no
geometry is built, moved or merged. `light::on_the_lit_surface` and its twin in
`blit.wesl` are the half-space predicate — a candidate is skipped when its extent
along the fragment's own normal axis ends at the fragment's own plane, from behind
it. Both CPU walks and the shader, one rule, stated once the way `Solid::wire_box`
is, at four call sites and two.

**Three things the step learned, each by a gate going red rather than by argument.**

*The theorem's precondition is load-bearing, and D2 as written left it out.* The
proof says `d·N > 0` — the ray must be *leaving* the plane — and dismisses `d·N < 0`
with "the flame is behind the surface, `N·L` is already zero". That is true of the
shaded frame and **false of the shadow term**, which the reference path tracer
compares directly with no cosine on either side. It said so immediately: 4,017
interior pixels of `line_scene`, every one a `y = 101` face of the west box with the
flame at `y = 98.5` behind it, drawn lit where the tracer had them shadowed by the
east box the exemption had just discarded. So the ray's direction is a parameter and
the precondition is a comparison of two signs. `d·N = 0` stays exempt: a ray lying
in the plane is the graze the whole rule exists for.

*The plane comes from the fragment's own **solid**, not from its position.* Both are
the same number — measured below — but reading it off the box puts both sides of the
comparison in one list and one precision, so the equality is exact by construction
rather than by a rasteriser's good behaviour. It costs nothing: the row a fragment
carries already names its solid.

*It does not eat `same_run`, and S4 may not delete that on this step's licence.*
D2's list of what it subsumes was right about `mine == reference.x` wherever a
stance names a side, and right about the cell arithmetic — but `same_run` is
*broader* than the theorem: it exempts a neighbouring panel of the run whatever the
ray's direction, including rays that dip **behind** the surface's plane, which the
theorem cannot license and the tracer will not allow. A flame is a sphere, so a lamp
standing level with a wall puts half its rays either side of that wall's plane; the
half going behind genuinely crosses the neighbouring panel. What removes those is the
*merge*, S3b — one primitive per surface leaves nothing to cross — not this step. See
§ *Backlog*.

Identity also survives for `Surface::Upright`, which has no plane at all: a tree's
sprite is excused from its own box by name and by nothing else.

What the step must settle, and it was the only open question in it: **where the
fragment's plane comes from.** Reading it off the interpolated position plane and
comparing to a stored box is a float equality across two sources; if it is not
bit-identical, the fix is to carry the plane from the instance row — which
already carries `solid` — and *not* to introduce a tolerance. Measure first, and
record which of the two it was.

✅ **Measured, and it is the interpolated position: they are identical, bit for
bit.** `traced.rs`'s `a_face_fragments_own_plane_is_the_primitives_own_number`
renders the run of flights and compares every face fragment's coordinate on its
own face's axis against that face's own bound: **39,930 fragments, zero off**, on
a scene whose faces sit on the thirds of a tile — the coordinates with no exact
`f32` at all.

The reason it holds is not luck. Every vertex of an axis-aligned face carries the
*same* coordinate on that face's axis, and interpolating a value equal at all
three corners returns it exactly under the `v0 + b·(v1−v0) + c·(v2−v0)` form a
rasteriser uses. So **the exemption is an equality, nothing is added to the
instance row, and S3 adds no number anywhere** — which is acceptance point 6
satisfied by construction rather than by inspection.

The measurement stays as a gate, because what it pins is the *pipeline*: a
projection that went perspective, a vertex format that lost a bit, or a driver
interpolating as `a·v0 + b·v1 + c·v2` would each break the equality while leaving
every picture looking right, and would turn the exemption into a rule that fires
on some pixels of a seam and not others.

### Acceptance for S3, as things to run and numbers to read

Each is a command, an artefact and a figure, so acceptance does not rest on
anybody's description of a picture.

Each is a command, an artefact and a figure, so acceptance does not rest on
anybody's description of a picture. **All six are run below, and two of them turned
out to be asking for the wrong thing** — recorded as found rather than quietly
restated.

1. 🔁 **The seam census. Asked for zero; zero was never the right target, and the
   census cannot be the gate.**
   ```sh
   OPENSHARD_TRACED_DUMP=target/traced/s3 cargo test --release -p openshard-client-render \
     --test traced -- the_frame_and_the_path_tracer_agree_about_a_run_of_flights --nocapture
   ./tools/mask_probe.py seams crates/client/render/target/traced/s3/run_of_flights_pathtrace.png
   ```
   🔴 **And the before/after this first read is not one — the correction is the
   point.** The census reports **87** (12 + 14 + 6 + 24 + 25 + 6) against a figure
   of 123 recorded in a previous session, and that difference was written up here
   as S3's own doing. It is not: with the exemption **neutralised in the shader**
   the census is **87 as well**, and the dumped mask is identical to the last
   pixel — 0 of 2568 × 512. The 123 came from a dump made in some other state, and
   attributing a difference to a change without injecting that change is exactly
   the trap this track has already paid for once (§ *Backlog*, "a dumped picture
   carries no mark of the code that made it"). One number, two sessions, no
   provenance.

   **What S3 moves on screen, measured: nothing.** Run of flights, 0 pixels; wall
   run elevation, 0 of 29,696; the stair fixture under a low front light, 0 of
   262,144. Its exemption is reachable only when a ray runs *in* the surface's own
   plane, which needs a point flame — the gate below uses one, and the shipped
   renderer never does, because a sphere of `FLAME_RADIUS` centred in the plane
   puts half its rays below it. So S3 is a rule made right and a picture unchanged,
   and the seam a person sees belongs to `docs/lighting_rebuild.md`'s backlog
   entry on the flame's own extent — the cosine is taken from the flame's centre
   while visibility is sampled over its whole sphere.

   The census pairs pixels by **which body drew them**, because that is all a dumped
   mask carries; it does not know which *face*. Probed, the first survivor is
   `(299, 218)`: box 0's `FaceSouth` at `(100.992, 100.333, 4.917)` shadowed, beside
   box 3's `Flat` at `(101.008, 100.333, 5.000)` lit. A **riser** beside a **landing
   top** — two surfaces, two normals, a real geometric edge, and the decision is
   supposed to flip there. So a flip across a join has three causes a picture cannot
   separate: a piece of a surface shadowing that surface (the defect), another
   surface's shadow boundary crossing the seam column (legitimate — four of them
   here, and the tracer draws each in the same place), and the walk inventing an edge
   (which `interior == 0` already rules out). The reference tracer cannot arbitrate
   the first against the second either: it holds the same nine boxes, so it
   reproduces a self-shadow as faithfully as a real shadow.

   **What tells them apart is which solid stopped the ray**, and only the walk can
   say. So the gate is `lighting.rs`'s
   `a_landing_cut_into_three_primitives_is_not_shadowed_by_its_own_pieces`: nine
   treads, both walks, forty fragments across each tread's own lid, and no solid of a
   fragment's own landing may be the one `Stopper` names. The Python census stays an
   instrument and now prints a pixel of each run, so a reading can be followed up
   with `OPENSHARD_TRACED_PROBE`.

2. 🔁 **The wall run lit along itself. Already drawn, already green — and it is a
   run of *panels*, so it could not have shown this.**
   `scene::wall_run_lit_from_along_it` *is* drawn by a tool:
   `pictures.rs`'s `a_wall_lit_from_one_end_has_no_dark_stroke_at_its_seam` renders
   its elevation, marks the seams and asserts monotonicity along the run. It passes
   today and passed before this step, because a panel run's seam is what `same_run`
   already covers. The uncovered defect was on a **body**, where there was no
   exemption at all — so the fixture that shows it is a run of bodies, which is what
   the gate in point 1 builds. A "before" picture of the panel scene would have shown
   nothing and proved nothing.

3. ✅ **The brute-force oracle stays equal.** The exemption's whole claim is that it
   discards no true occlusion, and the oracle is the non-circular check of exactly
   that. Green, both fuzz tests and both grid sweeps.

   The pinned corner graze that blocked this point was the *oracle's* own defect,
   resolved by deciding which side was right rather than by widening the fuzzer's
   carve-out; the seed stays pinned and passes. See § *The oracle*'s "no cells" rule.

   ⚠ **And the oracle cannot see this step at all**, which is worth knowing before
   leaning on it: its fixtures light a `Spot::flat` that is a point of *no* solid, so
   `own_box` is `None` and the exemption never fires. It is a check that S3 broke
   nothing, not evidence that S3 works.

4. ✅ **The path tracer stays at `interior == 0`.** Both gates, both scenes —
   261,682 pixels compared on the run of flights, 0 interior, 11 on an edge. It is
   also what caught the missing precondition, at 4,017 pixels.

5. ✅ **Fault injection, both directions, both run.**
   - *Neutralised* (`on_the_lit_surface` returns `false` outright): the landing gate
     reports **480 of 720** fragments shadowed by a piece of their own landing, on
     both walks. The three tests phase 4 found stay red under *their* injection —
     identity is untouched here.
   - *Widened past the theorem* (a candidate skipped when it merely **touches** the
     plane, from either side): `a_room_lights_its_own_wall_and_not_the_storey_over_it`
     goes red. A real occlusion discarded, which is what that injection is for.

   And one injection that came free: while the fixture in point 1 was written with
   the flame *above* a landing it passed **with the exemption neutralised** — a
   vacuous gate, because a ray leaving a lid upward touches the neighbouring piece
   only at `t = 0` and the zero-length touch rule already answers it. The flame had
   to go **into** the surface's own plane for the exemption to be the only thing that
   can excuse the crossing. A gate that passes under injection is not a gate, and
   this one nearly shipped as one.

6. ✅ **No new constant.** No tolerance, no epsilon, no widened box: the diff adds a
   plane comparison, a sign comparison and a table of five stances. The plane did not
   have to come from the instance row, so nothing was added there either.

**S3b — the merge, and it is last.** D2b. Contiguous neighbours that are the same
surface become one primitive at build time, after `occlusion::boxes_of` and inside
`Builder::finish`. Two primitives merge exactly when they share a whole face, have
equal opacity, equal `edges` classification and equal span — all exact
comparisons, since the coordinates come from integers and authored fractions, and
**no tolerance is introduced anywhere**. `occlusion::Part` keeps pointing every
instance at the solid it is now a part of (D6).

**`PANEL_THICKNESS`'s inward fattening is answered here** and not separately: two
walls on a shared tile edge are one surface, so they merge into one slab lying on
the plane the art draws, which is what the `docs/lighting_rebuild.md` backlog
entry asks for. The constant survives as *how thick a wall is* and stops being
*which side of its tile a wall sits on*.

*Gate:* **not one pixel moves.** That is the whole of what a pure optimisation
means, and it is checkable: the shadow masks before and after are identical, and
the cost harness says the primitive count fell. It runs **after** S5, since a
merged primitive is wider than a tile and the grid stops being a superset the
moment one exists — see the backlog's first entry, which is this step's own
precondition and not a nuisance.

**S4 — delete the cell rules.** D5, in this order and each behind its own
measurement: `same_run` (🔴 **not licensed by S3, and licensed by phase 5b rather than by the
merge** — S3 landed and measured that the exemption is *narrower* than this
function, which excuses a neighbouring panel of the run for rays that dip behind the
surface's plane as well as for rays leaving it. The theorem cannot license those and
the path tracer will not allow them. What was written here was that the merge
retires it; what actually does is that **those rays stop being traced**: a sample
behind the fragment's own plane has a zero cosine and contributes nothing, so there
is no crossing left for `same_run` to excuse. That is
`docs/lighting_rebuild.md`'s phase 5b, it is measured rather than argued, and this
deletion waits on it — not on S3b. See S3's own list of what it learned), the
per-cell `max` (there is no cell to group
by; the corner double-count it existed for outlives the merge's departure to S3b,
so this one carries its own measurement rather than inheriting one), the vertical
shortcut (a hierarchy has no reason for a special case, and
this removes a branch that has twice had to grow a footprint gate to stop being a
different answer), and `starting_cell` with `first` (nothing left reads a cell).

*Gate:* each deletion is preceded by neutralising the rule and finding the suite
green, and followed by the brute-force oracle staying equal. The three tests that
phase 4 found go red when identity is neutralised must stay red under that
injection — the self-shadow rule is **not** part of this and must not be
weakened by the merge.

**S5 — the hierarchy.** D3. A CPU build over the primitives and a
stackless traversal on both sides.

Pinned so the step has no decisions left in it:

- **Median split on the longest axis**, recursively, to start. Deterministic and
  free of a tuning constant. A surface-area-heuristic split is an optimisation,
  allowed later, gated on the cost harness and forbidden from changing a pixel by
  D4.
- **The build is a pure function of the primitive list and its order.** The tick
  is deterministic and the two backends must agree; a build that depended on
  anything else would make them two trees.
- **Stackless traversal, with an escape index per node.** WGSL has no dynamic
  stack, and a fixed-size array is a cap that would silently truncate — the shape
  `MAX_WALK_STEPS` has today, and the reason it is a *bound* rather than a
  budget. A node's escape index is where a miss continues, so traversal is a
  walk over an array with no stack at all.
- **Leaves hold up to four primitives.** A cost knob under D4, which is why it is
  a number here rather than a question.
- **A node budget replaces `MAX_WALK_STEPS`**, in the same role and for the same
  reason: a loop over data must not become unbounded because somebody widened a
  radius.

*Gate:* the brute-force oracle over the whole sweep, on the real place and on
every hand-built scene; a CPU-against-GPU test in the shape of
`a_sprite_pixel_meets_the_same_box_on_both_sides`, since the traversal is a
second spelling with no compiler between the two; and a cost measurement — which
needs `tests/cost.rs` to be able to price a frame **with real occluders**, since
it builds against `Occlusion::EMPTY` today and therefore cannot see this at all.
That harness fix is part of this step, not a follow-up — and it is also
`docs/lighting_rebuild.md`'s own backlog entry asking for "a cost harness that
prices the pass the client actually runs", inherited here rather than left in two
places.

## Not in scope, deliberately

Named so that a later session does not adopt them by accident:

- **A flame's own sprite reading black.** A real defect, found in the same frame,
  and it is about where a light *is* rather than about the shape of an occluder.
  `docs/lighting_rebuild.md`'s backlog owns it.
- **How far a real static's art overhangs its own volume.** Phase 6's own second
  number, still untaken. It is art against volume, not solid against solid.
- **The lateral fit.** `facing::Prism` is `up`, `heights` and `count` — it has no
  term for a cross-axis extent at all, so a fitted climbable is sub-tile along
  its climb and a whole tile across. Worth doing and not this: it changes what one
  primitive's shape is, where this plan changes how many there are.
- **The tile-to-world mapping.** D7.
- **Phases 7 and 8** of `docs/lighting_rebuild.md`, which are billboards and the
  sun and touch none of this.
- **Land as an occluder** — a hill casts no shadow today. A hierarchy over
  arbitrary boxes is the structure that would make terrain an occluder cheap,
  and that is a *reason to expect it later*, not a step here. It stays a carried
  item of `lighting_rebuild.md`.
- **A corner's two panels told apart by the screen half.** They are
  perpendicular, so no merge joins them, and what closes it is a volume carrying
  its instance row — D6's join one level finer. Named because it looks like this
  plan's business and is not.

## Backlog

Findings from this track that do not block a step. Kept here so the plan can be
read as work.

🚩 **The merge inherits the seam, and what it inherits is a sphere's own half.**
S3 cures a surface shadowing itself for every ray *leaving* that surface, which is
what its theorem licenses. What it cannot touch: a flame is a sphere, so a lamp
standing level with a wall — or in a landing's own plane — puts half of its eight
rays on the far side of that plane, and those rays genuinely cross the neighbouring
primitive of the same surface. The reference tracer, handed the same primitives,
agrees that they do. `same_run` papers over exactly this for a panel run by exempting
the neighbour whatever the ray's direction, and there is no equivalent for a body.

⚠ **And the merge may not be what answers it.** Those below-plane rays are only
traced at all because the shading takes its cosine from the flame's *centre* while
visibility is sampled over the flame's whole sphere: a sample point below the
fragment's horizon should contribute zero by `N·L` and never be asked about
occlusion. Fix that, and the set of rays a join can block is empty — no merge
required, and `same_run` loses its reason too. Prototyped and rendered on
2026-08-09; it lives in `docs/lighting_rebuild.md`'s backlog, since it is a shading
question rather than a geometry one. **Measure that before spending S3b on this**,
because the merge's own argument then falls back to what it always was — one
primitive per surface is cheaper and simpler, not a cure.

**Decided 2026-08-09, and it reverses the paragraph that used to stand here.** That
paragraph read "only the merge answers it, and it answers it completely", and it was
written before the shading side was measured. Three things say otherwise, and each is
enough on its own:

- **The merge does not reach the fixture the wedge was measured on.** S3b merges
  primitives that share a whole face *and have an equal span*, which is why this plan
  already writes down that a flight's treads do not merge. The wedge was measured at
  the joins of three flights that are geometrically one landing. They stay separate
  primitives, so the join stays, so the wedge stays.
- **Even where it merges, it cures a neighbour and not a set of rays.** A ray below
  the horizon can end on anything — a wall's base, the step below, a body. The merge
  removes the neighbour *of the same surface*; only `max(N · L, 0)` removes the set,
  because the set is "everything behind the plane".
- **The defect is not only seams.** The prototype moved 21,177 pixels and darkened
  20,308 of them: the centre cosine over-pays every grazed surface, join or no join.
  No step of this plan may touch that — D4 is "not one pixel moves". **A step
  forbidden to move a pixel cannot fix a defect whose symptom is moved pixels.**

So: `docs/lighting_rebuild.md`'s **phase 5b is the cure**, S3b is an optimisation —
one primitive per surface is cheaper and simpler — and it keeps its place last,
after S5, with its own gate unchanged: not one pixel moves.

✅ **The pinned corner graze: the walks were right and the oracle was wrong —
closed 2026-08-09.** `lighting.proptest-regressions`' newest line, found by a
fresh seed, red for a day. Nothing in the session that found it touched
`crates/*/src`; the case was always there and no seed had reached it.

```
spot  (104.6041, 100.9463,  2.00) tile (104, 100)
light ( 93.1834, 101.0253, 13.69) tile ( 93, 101)
walk_cells says blocked, the brute-force oracle says open
```

One whole-tile body at `(100, 100)`, `z` 0..20. The segment crosses the wall's
column while its `y` runs 100.971 → 100.978 — **three hundredths of a tile from
the corner at `(101, 101)`**, which is the region the sibling grid test excludes
by construction and this fuzzer aims at on purpose.

**Settled by the exact test rather than by either disputant**, which is what the
open question asked for: `segment_inside_box` over the eight flame points says all
eight enter the wall's box, so **blocked is the truth** and both walks had it.

What the sampler got wrong, in one line of numbers:

```
ray 5: enters at t 0.315466, leaves at 0.315485 — 0.000225 tiles of wall,
       and over that whole clip y runs 100.999997 → 101.000000
step 18023: point (100.9999084, 101.0000000, 5.52059) → tile (100, 101)
            inside the box on every axis, and that cell lists 0 solids
```

The clip's entire `y` extent is three millionths of a tile below `y = 101`, and no
`f32` exists in that gap — the ulp at 101 is `7.6e-6` — so the sampled point's `y`
is *exactly* `101.0`. `floor()` sends it to cell `(100, 101)`, which is empty, and
the oracle reported open ground from inside a wall.

So the step was never the culprit: `0.000225` is **deeper** than `BRUTE_STEP`, and
the march did land a point in there. Both walks failing identically was the clue
read backwards — they agreed because they were *right*, and the thing the two of
them share is not a DDA bug but a correct answer. The fix is § *The oracle*'s "no
cells" rule: both samplers iterate `Occlusion::solids()` and state their exemptions
as closed volumes, so neither has a `floor()` in it any more. The seed stays pinned
and passes, `the_pinned_corner_graze_is_blocked_and_all_three_oracles_say_so` pins
the verdict itself, and putting the cell lookup back turns exactly that assertion
red while the walks' two stay green.

**This also disarms half of the merge hazard below**: `brute_force_blocked` was
named there as "cell-based too, and would agree with the defect". It is not
cell-based any more, so a merged primitive wider than its registration cell is
now caught by the oracle rather than blessed by it. The two *walks* are still
cell-based, which is the rest of that entry and S3b's own problem.

**`frame.rs`'s `ground_truth_blocked` took the same repair with no coverage to
prove it.** It is only ever called on a `walk_cells`/`walk_cells_exact`
disagreement, and the sweep over all seven scenes reports `0 explained, 0
unexplained, 0 grazed` — the two walks no longer disagree anywhere, so the
arbiter is a standby that never runs. Its correctness rests on its twin in
`lighting.rs`, which the fuzzers do exercise. Worth knowing before trusting it as
a gate: it is not one today.

**A gate whose fixture puts the flame in the wrong place passes under injection.**
The landing gate S3 built passed *with the exemption neutralised* while its flame
stood above the landing rather than in its plane: a ray leaving a lid upward touches
the neighbouring piece only at `t = 0`, which the zero-length touch rule already
answers, so the fixture never reached the rule it was written for. It was caught by
running the injection, which is the only thing that can catch it — a green gate and a
vacuous gate are the same output. Worth stating as a habit rather than as an
incident: **every new gate on this track gets its injection run in the same
session**, and the flame's position relative to a surface's own plane is the
parameter that decides which rule a fixture is even asking about.

**A cell lists a primitive once, and D1 has just made that a hole S3b will fall
into.** `Builder::push` puts a solid in exactly the cell it was added on;
`Solid::footprint` — which answers *which tiles a box touches* and whose own doc
says "the day a box is wider, this is where the extra tiles come from" — has one
caller, and it is `bake`'s. Nothing before S1 could build a box reaching past its
own tile, so nothing noticed. **S3b's merge is exactly the thing that builds
one**, and the moment it does, the grid stops being a superset: a ray that
crosses the overhang without ever entering the registration cell is answered
"open" by both walks. `tests/lighting.rs`'s `brute_force_blocked` was cell-based
too and would have agreed with the defect; since the corner graze above it iterates
every primitive, so the oracle now **catches** this instead of blessing it — the
difference between a step that fails loudly and one that merges wrong geometry
quietly. This is D3's own argument
arriving early, and S3b has to answer it before it merges anything — either by
listing a merged primitive in every cell it spans, or by taking the hierarchy
first. It is why S1's own gate keeps its fixture inside one tile: the wire is
what that step is about, and a straddling box would have failed it for a reason
S1 does not own.

**And listing one primitive from two cells double-counts it.** The walk
multiplies `1 - stopped` cell after cell, and the per-cell `max` groups only
*within* a cell — so a solid a ray meets on two of its cells is applied twice.
Opaque either way, wrong for anything translucent (a pane, a `PANE` opacity of
51). Whichever way the item above is answered, this is the second half of it, and
D5's deletion of the per-cell `max` is where it lands.

**The apertures are the last texture indexed by a `SolidId`.** The primitives are
a storage buffer now and the holes beside them are still `Rgba8Uint` folded into
`LIST_ROW` rows, with `Occlusion::list_rows` existing for that plane alone. One
list in two shapes; D8's argument covers it and S1 left it deliberately, because
a hole is read behind a bit test and moving it buys nothing until something else
touches it. It should go with the reference list in S5.

**Grey in a dumped mask meant three different things, and that cost a session —
fixed.** The two mask strips drew `None` as one grey level, and `None` is
"nothing drawn here", "the two disagree which surface is there, on a silhouette"
and "…and not on one" — the last of which is the only one that is a defect. A
field of grey slabs across the run of flights was carried into a second session
as evidence of a lighting fault, while the counts printed beside it already said
`0 with nothing drawn` and `0 not on a silhouette`. What settled it in one pass
was **laying the grey over the lit render**: the slabs fell exactly on the risers
below the tread in front — the paint-order defect this scene's own doc records,
not a walk defect.

So the dump has a **fourth strip** now, `Verdict::strips`'s own: black compared,
grey nothing drawn, teal a silhouette, **red the one that is a defect**. Built
where the judging is, so the tool and the gate draw one rule rather than two —
they had a copy of the three-strip code each, identical to the line. Checked by
injection rather than believed: putting the flights back in climb order paints
7,016 red pixels in exactly the shape that was argued about, with 2,179 teal
around them, and the histogram matches the printed counts term for term.

**A dumped picture carries no mark of the code that made it.** The same grey
slabs survived as evidence because the file predated the fix and nothing about it
said so — the name even carried `_fixed`, meaning a corrected *crop*, not a
corrected scene. A dump is the instrument this track is steered by, so it should
stamp what it is: the verdict's own counts, at least, written beside the pixels
they describe.

**A composite of strips cannot be cut by dividing its width.**
`png::write_strips` puts a `RULE_WIDTH` ruler between strips, so a three-strip
image of 512-pixel panels is 1540 wide and `w / 3` is 513. Cutting that way
shifts every strip after the first by a pixel per ruler, and a one-pixel shift
between two renders reads exactly like a camera that is off. It was reported as
"a systematic 3 px offset" once; there is no offset. Slice at
`k * (SIDE + RULE)`.

**A hole's own `z` is still quantised**, to whole units offset by 128
(`occlusion::z_byte`), and it is now the only quantised number left in the pass.
No defect: a hole is measured off the art as whole units, so there is nothing
below the step to lose. Written down because "everything here is exact except
this" is the sort of fact a later reader should find stated rather than discover.
