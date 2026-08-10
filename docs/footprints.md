# The footprint — a static's box is the box the art drew

A living plan. The backlog at the end is where the next session starts.

`docs/occluders.md` names this and puts it outside its own scope, in as many
words: *"the lateral fit … it changes **what one primitive's shape is**, where
this plan changes **how many there are**."* This is that change, and it is the
other half of `docs/lighting_rebuild.md`'s census line — the class counted there
as **"a whole tile, because the art would not say"**.

## What we are fixing

**A static whose art states a shape narrower than its tile is given the whole
tile.** Not as a fallback of last resort for a handful of graphics: over Britain
it is **31.6% of every static in the world** (`examples/geometry_census.rs`,
121×121 tiles around `(1501, 1659)`, 11,184 statics), and 19.2% on the smaller
window around `(1504, 1655)`.

**What that costs, in a picture a person reported.** Two bookcases
(`0x0A97`/`0x0A98`, server decorations at `(1505, 1656, 27)` and
`(1506, 1656, 27)`) draw as flat slabs against a wall. Their occluder is a cube
`1 × 1 × 12`, so `impostor::meets` answers the top 45% of each sprite with the
cube's **lid** — a `+z` normal over the pixels where the artist drew shelves —
and splits the rest down the sprite's middle column into `+y` and `+x` at the
cube's ridge. In `View::Normal` that reads as a large flat plane floating through
the furniture, the same colour as the floor, which is what was reported.

**Done when:** a static of this class stands as the box its own base edge
describes, and the frames of every other class do not move by a byte.

## Why it is a whole tile — the root

**The vocabulary, not the storage.** `occluders.md`'s D1 already made a
primitive's coordinates absolute `f64` end to end: `Solid` is two
`camera::WorldSpot`s, the wire carries `min`/`max`, `Builder::add_raw` stores an
arbitrary AABB in a tile bucket, and the BVH walks arbitrary boxes. **A
sub-tile parallelepiped is representable through the whole pipeline today.**

What is missing is at the two ends:

1. **Nothing measures one.** `facing::Facing` is `One(Face)` or
   `Corner { right, left }` — four tile edges and a pair of them. It has no term
   for an offset from an edge and none for a depth. So `facing_of` answers `None`
   for any box that does not *stand on* an edge, and `edges_of(None)` is
   `Edges::ANY`, the whole tile.
2. **Nothing consumes one.** `facing::Block` — `x`, `y`, `z` each a `(u8, u8)`
   span in eighths of a tile — is exactly the type, with a parser in
   `arttable.rs`, a `MAX_BLOCKS` cap, and `facing::blocks_silhouette` drawing one
   the way the projection draws it. `Shape::blocks` says outright: **"No detector
   writes one."** And `occlusion::boxes_of` has no branch that reads it: an
   authored block reaches the grid through nothing at all.

**The art does say it, and here are the numbers off one picture.** The base edge
of `0x0A97` (bottom drawn row per column, `examples/shape.rs`):

```
cols  0..10   11 .............. 35   36 ........ 43
        .     62 63 64 … 85 86      86 85 … 80 79
              └ 25 cols descending ┘└ 8 ascending ┘
```

Two 45° runs meeting at a near corner, which is what the projection makes of a
world-axis-aligned rectangle: the run descending right is the footprint's `+x`
edge, the run ascending right is its `+y` edge. Compare the same reading of
`0x0063` "stone wall", which *does* read as `One(South)`:

```
cols  0..21  36 37 … 56 57   22..30  57 56 … 50 49   31..43  .
```

The same V. The difference is not the shape, it is **where it sits**: the wall's
long run starts at column 0, so it stands on a tile edge and `Half::read` accepts
it; the bookcase's starts at column 11, so it stands in the middle of the tile
and both halves refuse it — the left holds 11 filled columns against
`MIN_FILLED` 18, and the right's base is a chevron with rise 6 over run 21
against `SQUARE` 3.

## The decisions

Made, with the alternative recorded where there was one.

**D1 — the art states the footprint and the tiledata states the height, and
neither is asked for the other's answer.** The measurement below reads two
horizontal spans off the base edge and nothing vertical. The top stays
`occlusion::calc_height(tile)`, unchanged and untouched. *Rejected:* measuring
the height off the art in the same step — it is a second change to the same box
in the same census, and a run that moved would not say which half moved it.
The vertical is a carried item, below.

**D2 — a new measured type, and `Blocks` is left alone.**
`facing::Footprint { x: (u8, u8), y: (u8, u8) }`, in eighths like `Block`, on
`Shape::footprint`. *Rejected:* having the detector write `Shape::blocks` —
`Blocks` is a full parallelepiped list an *author* writes for a shape no
measurement can reach (an arch's posts and lintel), and a derived value in the
same field would make "who wrote this" unanswerable, which is the one thing
`ArtTable`'s authored-row precedence exists to keep answerable.

**D3 — eighths, not a finer grid.** An eighth of a tile is 5.5 screen pixels and
the art's own antialiased border is worth ±2. *Rejected:* a wider fraction — it
buys precision the picture does not have, and `Block` already fixed the unit for
the authored half of the same question.

**D4 — this replaces the whole-tile fallback and nothing else.** `boxes_of` reads
a footprint only where it would otherwise reach `edges_of(None) == Edges::ANY`:
not for a climbable (the prism wins, and `occluders.md` calls the prism's own
lateral fit a separate item), not for a `BACKGROUND` lid, not for anything
`facing_of` already named a face or a corner. So every wall, floor, roof and
stair in the world draws exactly as it does today, and that is a gate rather
than an intention.

**D5 — the fit is a residual, and a picture that does not fit keeps the tile.**
The gates are height-free, because the height is D1's other half and scoring
against it would refuse a correct footprint for a wrong top: a contiguous base,
two runs at 45° within `STRAIGHT` of their own fitted lines, a near corner
flatter than `PLATEAU` columns, and the picture drawn inside its own tile's
column — the last being the gate `Half::read` already opens with, and the one
that separates this from a picture of a whole building.

*Rejected:* scoring by `blocks_silhouette` IoU the way `best_prism` scores a
prism. It is the right instrument for a shape whose height is also measured, and
it is the instrument the carried vertical item should use.

**D5a — the footprint is clipped to its tile, not refused for leaving it.**
Measured, and it is why the first cut of this read nothing: `0x0A97`'s own base
states a box **1.11 tiles** across, since the artist drew twenty-five columns of
descent where a tile's edge is twenty-two. Real furniture is drawn a little wider
than its cell. Clipping gives back exactly what the whole-tile fallback already
gave on that axis and keeps the *other* one — which is the whole gain, and
refusing instead throws a measurement away over an axis that was never wrong.

**D5b — the near corner is not read from a pixel, and the flat one is skipped.**
Each run states one far coordinate as the *intercept the projection makes
constant along it*, taken as a median over the whole run, so no single
antialiased pixel moves the answer. The columns at the corner belong to neither
run. The first cut capped that at two columns and refused **47.3%** of the class
on it — the largest refusal by four times, every one of them measurable from the
two runs either side. `PLATEAU` is where a corner stops being a corner.

**D6 — the projection is not re-derived.** The measurement inverts
`impostor::ray_from`, which is the one spelling of this projection on the CPU:
`across = (u − v)·22`, `down = (u + v − 1)·22`, a sprite's bottom row on the
tile's own `(1, 1)` vertex per `statics::stand_on`. A unit test round-trips every
measured footprint through `facing::blocks_silhouette` — the existing reference,
written for the authored half — so the arithmetic here is checked against a
drawing rather than against itself. `docs/style.md`'s own rule about a formula
written twice.

**D7 — `DETECTOR` bumps and the table format bumps with it.** A footprint is a
new column, so a table written before this is a table that cannot say a bookcase
is a slab. `arttable`'s own version note covers the trap: a reader that half-read
one would answer `footprint: None` for every graphic and look like a detector
that found nothing.

## Steps

**S1 — the measurement.** ✅ 2026-08-10. `facing::Footprint`,
`facing::footprint_of` and `facing::measure_footprint`, which names its refusal;
`artscan`'s `examples/footprints.rs` is the census, in two passes.

*What it reads*, on Britain's `121×121` around `(1501, 1659)`, 11,184 statics:

| | |
|---:|---|
| **3534** | the class: whole tile, the art would not say — 31.6% of every static |
| **2825** | of it, `ROOF` — a **sloped slab**, which is not a box and is not meant to be |
| **709** | the rest, which a box could be |
| **213** | a footprint measured — **30.0%** of that rest, and every one narrower than its tile |

**The `ROOF` split is the finding, not a filter.** The first run of this census
read 6.2% and reported 76.6% of its refusals as "crooked"; naming the pictures
turned that into "roof, roof, shingles, roof" — eight of the twelve most-refused
graphics, 2,825 placements. A sloped plane's base edge is not two 45° runs and
never will be. `docs/lighting_rebuild.md`'s phase 6i is the open question about
those and it is untouched here.

*What it still refuses*, named because a share hides a tail: `Crooked` 356 —
counters, display cases, benches, rocks, a stone arch. Furniture whose base is
not a clean V, and the next place to look.

**And two gates were wrong before they were measured**, both recorded above as
D5a and D5b rather than quietly fixed: the tile-span refusal (it refused the
bookcase this plan was written for) and the two-column plateau cap (47.3% of the
class).

**S2 — the table.** ✅ 2026-08-10. `Shape::footprint`, on `facing: None` only
(D4, one level up from `boxes_of`'s own gate); `arttable`'s `footprint x0 x1 y0
y1` column, restricted to a `none` verdict in the grammar itself; `FORMAT` 4→5
and `DETECTOR` 2→3, both with the same trap the prism and the hole each closed
for themselves recorded in their own doc comments; and `derive`'s "was
anything read" test grown to three terms so a measured footprint with no face
and no prism is not mistaken for nothing having been read and dropped as a
refusal. The shipped `data/overrides.table` carries the bump too — an
`include_str!` parsed with `.expect`, so a forgotten sheet would have failed
at test time rather than at review.

**S3 — the consumption.** ✅ 2026-08-10. `boxes_of`'s `Edges::ANY` branch reads
`shape.footprint` where it is `Some` — the one branch `edges_of(None)` reaches,
so D4's gate falls out of the existing structure rather than needing a
separate check: a face or a corner already routes through `named`, a lid
through `Edges::NONE`, a climbable returns before either. `StaticAtlas` grew a
`footprints` map beside `holes` and `prisms` — the same lookup, the same
per-graphic seam — and `occlusion::shape_of` reads it instead of the S2
placeholder `None`.

*Gate:* `examples/geometry_census.rs` grew the row — "a measured footprint,
narrower than the whole tile" — and on Britain's `121×121` around
`(1501, 1659)` the "whole tile, the art would not say" line drops from 3534
(31.6%) to 3315 (29.6%), a fall of **219**. Six more than S1's own placement
count of 213: that census skips `ROOF` tiles before ever calling
`measure_footprint`, a reporting choice for its own narrative ("a roof is not
a box"), while `boxes_of` and `Shape::of` were never gated on the `ROOF` flag
— a handful of roof pieces whose base happens to read as a clean two-run V get
measured too, and now narrowed, exactly like any other picture. Not a defect:
`examples/footprints.rs at 1501 1659 60`'s own split confirms it — 2825 roof
placements minus 6 measured leaves 2819, plus 496 of the boxy 709 still
refused, is 3315.

**S4 — what it does to the picture, measured before it is believed.** Two
numbers, both of which can go the wrong way:
- **the impostor's discard.** A tighter box is a box the sprite overhangs more,
  and `statics.wesl` discards a fragment whose ray meets none. Phase 6 measured
  the whole-tile version at 4,460 of 187,086 static pixels (2.38%); this must be
  re-measured, and a footprint that eats a tabletop's overhang is a finding, not
  a cost to accept quietly.
- **the shadow.** A narrower occluder casts a narrower shadow. Every occluder in
  this class is `CLEAR` today (`opacity == CLEAR`, `Builder::add` returns) —
  furniture stops no light — so the expected move is **zero**, and a run that
  moves it names something that is not furniture.

**S5 — the frame gate.** The bookcase pair, `View::Normal`, asserting the lid
shrinks to the measured slab, with the whole-tile footprint as the injection that
must go red. Depends on the parity item below.

## Not in scope, deliberately

- **The height.** D1. A carried item: the art states it, `blocks_silhouette` is
  the instrument that would score it, and it is a second census.
- **`facing::Prism`'s lateral fit.** `occluders.md` names it; a climbable takes
  the prism branch and never reaches this one.
- **Authored `Blocks` reaching the grid.** `boxes_of` ignores them today and will
  still ignore them after S3 — a separate, smaller item, and one this plan's
  branch makes obvious rather than closes.
- **Whether furniture should stop light.** Every graphic this plan touches is
  `CLEAR`. Making it an occluder is a gameplay-visible change with its own
  argument, and S4's "expected move is zero" is a gate that depends on not
  making it here.

## Backlog

- 🚩 **S5 needs the parity item first.** The reported picture is two *server*
  decorations, and `examples/isolated_scene.rs` reads no database — see
  `docs/parity.md`'s own first backlog entry. Until it does, the frame gate's
  fixture is two hand-transcribed `OPENSHARD_SCENE_EXTRA` rows, which is a
  hand-written input standing in for the client's.
- 🚩 **`Crooked` is 356 placements and they have names.** `0x0B3F` and `0x0B40`
  "counter", `0x0AA0`/`0x0AFE`/`0x0B01` "display case", `0x0B5F`/`0x0B60`
  "bench", `0x1365`/`0x1366` "rock", `0x00CF`/`0x00D1` "stone arch". Two of them
  are the sharpest case: **`0x0B3E` measures `x (0,4) y (0,4)` and `0x0B40`,
  the same counter drawn the other way round, is refused** — so the refusal is
  not "a counter is not a box", it is something about one of the two pictures.
  That pair is a two-graphic fixture, which is the cheapest kind.
- 🚩 **The outer ends are half a pixel wide and the rounding pays for it.** The
  two far coordinates are exact (D5b); the two near ones are read from the end
  column's centre, and the sweep paints the column a corner falls *inside*, so
  each is out by up to half a pixel — one eighth after outward rounding, which is
  what `a_footprint_measures_the_block_that_drew_it` asserts as slack rather than
  hides. Reading the column's inner boundary instead would halve it; worth doing
  only if S4 shows the impostor's discard cares.
