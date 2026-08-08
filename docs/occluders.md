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

**D2 — one physical surface is one primitive.** Contiguous neighbours that are
the same surface merge into one box at build time. This is what makes the seam
stop existing rather than get chosen a side. *Rejected:* leaving the seam and
widening the rules that hide it, which is `WIDTH_OVERLAP`'s own family and what
`docs/style.md`'s *No fudge constants* was written from.

**D3 — the broad phase is a bounding volume hierarchy.** A tree of axis-aligned
boxes over the merged primitives; a ray that misses a node skips its whole
subtree. The uniform tile grid goes. *The reason is D2 and not speed:* a uniform
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

## Steps

Each is landable alone and leaves the tree working. A session starts at the first
one whose gate is not green.

**S1 — absolute coordinates on the wire.** D1. The reconstruction and the
quantisation go; a primitive carries its own six numbers. `walk_cells_streaming`
stops previewing a quantisation that no longer exists, which collapses the
documented difference between the two CPU walks. **The ceiling that a primitive
is a tile's is lifted here**, and nothing can merge before it is.

*Gate:* a fixture whose primitive is deliberately **not** tile-aligned — no
scene in the tree has one today, and that is the blindness this step is also
fixing — and the two walks and the shader all equal the brute-force oracle on it.
Fault injection: re-introduce the `/255` rounding on one side alone.

**S2 — the detector, before the fix.** § *The detector*, built and read. Its
first number on a real place is recorded here, in this document, as the thing S3
moves.

*Gate:* the census fires on today's tree (it must — the seams are there), reports
how many pairs it examined, and its synthetic twin runs under `cargo test` on a
scene with a known seam.

**S3 — merge.** D2. Contiguous neighbours that are the same surface become one
primitive at build time, after `occlusion::boxes_of` and inside `Builder::finish`.
Two primitives merge exactly when they share a whole face, have equal opacity,
equal `edges` classification and equal span — all exact comparisons, since the
coordinates come from integers and authored fractions, and **no tolerance is
introduced anywhere**. `occlusion::Part` keeps pointing every instance at the
solid it is now a part of (D6).

**`PANEL_THICKNESS`'s inward fattening is answered here** and not separately: two
walls on a shared tile edge are one surface, so they merge into one slab lying on
the plane the art draws, which is what the `docs/lighting_rebuild.md` backlog
entry asks for. The constant survives as *how thick a wall is* and stops being
*which side of its tile a wall sits on*.

*Gate:* the census goes to zero. A run of N coplanar walls is one primitive, and
a storey's floor is one. `same_run` neutralised now leaves the whole suite green
— which is not a claim, it is the **measurement that licenses S4** to delete it.

**S4 — delete the cell rules.** D5, in this order and each behind its own
measurement: `same_run` (licensed by S3's reading), the per-cell `max` (there is
no cell to group by, and the corner double-count it existed for is gone with the
merge), the vertical shortcut (a hierarchy has no reason for a special case, and
this removes a branch that has twice had to grow a footprint gate to stop being a
different answer), and `starting_cell` with `first` (nothing left reads a cell).

*Gate:* each deletion is preceded by neutralising the rule and finding the suite
green, and followed by the brute-force oracle staying equal. The three tests that
phase 4 found go red when identity is neutralised must stay red under that
injection — the self-shadow rule is **not** part of this and must not be
weakened by the merge.

**S5 — the hierarchy.** D3. A CPU build over the merged primitives and a
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
That harness fix is part of this step, not a follow-up.

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

## Backlog

Findings from this track that do not block a step. Kept here so the plan can be
read as work.

*(Empty at the time of writing — S1 has not started.)*
