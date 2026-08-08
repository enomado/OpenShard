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
again: **a flight's treads do not merge.** S3's rule requires an equal span, and
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

### What the flight seams turned out to be — measured, and not a defect

The run of flights shows a hard shadow step landing **exactly on the join
between two primitives** of one continuous riser, which reads as "each new
primitive starts its shadow at its own corner". It is not that. Three
measurements, in the order they settle it:

- **The path tracer draws the identical picture** — 261,682 pixels compared, 0
  disagreeing in the interior, 11 on an edge. That renderer has no cells, no
  tiles and no walk, so nothing about traversal order or the grid can be what
  puts the step there.
- **`probe` at the join** shows the step is not near the seam, it *is* the seam:
  every pixel of the west primitive dark, every pixel of the east one lit, with
  no transition between. That is what a real blocker looks like when the blocker
  is the neighbour's own body — the flame sits a sixth of a tile *behind* the
  riser plane, so the segment from a fragment leaves through its own solid
  (exempt) and enters the next flight's, whose west face is the join.
- **`seams` against the shaded frame** says how much of it a player sees: at most
  43 steps of 255 and a mean under 9, over some 120 pixels. The cosine has
  already darkened a face the flame is behind, so the visibility mask shows a
  cliff where the lit frame shows almost nothing.

**The seam that is a defect is the opposite configuration** and this scene cannot
pose it: a flame just *in front* of the plane, where the segment runs along the
surface and grazes the neighbour it never enters. That is acne on coincident
planes, it is what `same_run` currently papers over with a cell rule, and it is
what D2's declared surface identity is for. Its fixture is a **run of wall**, not
a flight of steps — `scene::wall_run_lit_from_along_it` exists and no tool draws
it yet.

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

**A cell lists a primitive once, and D1 has just made that a hole S3 will fall
into.** `Builder::push` puts a solid in exactly the cell it was added on;
`Solid::footprint` — which answers *which tiles a box touches* and whose own doc
says "the day a box is wider, this is where the extra tiles come from" — has one
caller, and it is `bake`'s. Nothing before S1 could build a box reaching past its
own tile, so nothing noticed. **S3's merge is exactly the thing that builds
one**, and the moment it does, the grid stops being a superset: a ray that
crosses the overhang without ever entering the registration cell is answered
"open" by both walks *and* by `tests/lighting.rs`'s `brute_force_blocked`, which
is cell-based too and would agree with the defect. This is D3's own argument
arriving early, and S3 has to answer it before it merges anything — either by
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
