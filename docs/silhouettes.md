# Silhouettes: the two edges that meet along one line

A living plan, and its own session. The backlog at the end is where the next one
starts.

## The root

**A magnified frame draws its outlines at two different resolutions, and they
are adjacent.** Measured at Britain on the client's own `4x` dump
(`1919x2077`), as the number of rows a silhouette holds one column before it
steps:

```
an impostor box's edge        every 1–2 rows      decided per fragment
a billboard's own alpha edge  4, 8, 12, 16, 20…   decided by the art's texel
```

The second is not a defect on its own: the quad is scaled by
`Projection::scale` and sampled `nearest`, so one art texel is `scale` real
pixels square and its edge *cannot* be finer. Neither is the first. What a
person sees — and dislikes — is the two of them meeting along one line: a wall's
box face and the same wall's drawn outline, one side crisp and one side in
four-pixel stairs.

**The colloquial name for it is the zigzags, and this plan is what turns that
into a measurement.**

**Z1 has since amended this.** The second line is not an outline at all: a box
miss is no longer discarded, so the picture's silhouette is the art's and all of
it, and what the fragment grid draws is a *seam inside* the picture. The section
below is the question as it was asked; the phase records what it turned out to
be.

## What has *not* been established, and it is the first thing to fix

The 4-row number above was measured on a **mobile**, and a mobile is the one
case that is certainly art-bounded: `statics.wesl` takes the `is_mobile` branch
straight to `billboard_normal()`, meets no box and clips against nothing. A
static with a volume is a different story — the same shader ends its box meet
with `if !hit(best) { discard; }`, so its outline is *already* clipped to the
box wherever the art overhangs it.

So the honest statement of what is known today is:

- a mobile's outline is art-quantised — **certain, from the code**;
- a static the grid holds no volume for is art-quantised — certain for the same
  reason, and it is already its own layer (`View::NormalSprites`, `5e52279`);
- a static with a volume is bounded by **whichever of the two ends first, per
  fragment**, and *nobody has measured which one that is anywhere*.

The zigzags a person points at have therefore not been attributed. That is the
whole reason the view comes before the fix.

## The target

**Two debug views in which a fragment is in exactly one**, in the shape
`View::NormalGeometry` / `View::NormalSprites` already established: a picture
that answers "what bounded this outline" rather than "what colour is this
outline". Then, with the attribution in hand, a decision about the coarse one.

## The decisions to make, and the candidates

**S1. What the split is read off. — SETTLED, and both of the cheap candidates
were refuted before a line was written.** The precedent held: the normal split's
first rule was "the solid the fragment names" and a dump of it *refuted* that
rule. Here the refutation came off the code rather than off a picture.

1. ~~**`Meeting::outside`, carried into the G-buffer.**~~ **Vacuous.**
   `impostor::hit` *is* `outside <= TANGENT`, and `TANGENT` is `1e-4` of a tile
   — a numerical epsilon for a ray that reaches a box's own corner, not a rim.
   While the box-miss discard stood, every surviving fragment therefore measured
   at most that, and the plan's own sentence above — "a fragment at the box's rim
   sits at the tangent limit; one in the art's interior sits near zero" — is
   wrong in both halves: both sit at zero. The number carries no information
   about a neighbour, because it was never about one.
2. ~~**A neighbourhood test in the blit.**~~ **Cannot attribute.** The blit sees
   *that* a neighbour is not a static and never *why*: the G-buffer holds one
   answer per pixel and no record of the art rectangle a pixel came from, so it
   cannot re-ask either mask. It can find a silhouette; it cannot name what
   ended one.
3. **The texel grid, drawn.** Still unbuilt, and no longer needed for the
   attribution — see the backlog, where it survives as a picture of the quantum.

**4. Both masks tested in the producer, four screen neighbours each. — BUILT.**
`statics.wesl` is the one place both are alive. Two bits ride at the top of the
id word (`place_format.wesl`'s `IDS_EDGE_ART` / `IDS_EDGE_BOX`, which cost the
row field two of its twenty-six bits and left twenty-four), and two views draw
them: `View::SilhouetteArt` and `View::SilhouetteBox`.

- `art_edge` — a neighbouring **texel** fails the alpha test this fragment
  passed, or lies outside the sprite's own rectangle in the atlas.
- `box_edge` — a neighbouring **fragment**, one real pixel away
  (`1 / viewport.scale` virtual pixels), meets none of this instance's boxes.

The cost is four `textureLoad`s and four extra runs of the selection loop per
static fragment, always on. It was gated on the view for one draft and the gate
was taken out: a G-buffer whose content depends on which picture is being asked
for is exactly the coincidence `docs/parity.md` is about, and it would have made
the flag travel from a diagnostic into a world pass.

**S2. What to do about the coarse edge, once it is attributed.** Not decided,
and the three are not variations of one answer:

- **Leave it and say so.** The art is pixel art and the client it is compatible
  with drew it this way. Then the work is one paragraph in `docs/style.md` and
  the views above, so the next person who notices reads the answer instead of
  re-deriving it.
- **Let the box bound more of it.** `docs/footprints.md` and
  `docs/occluders.md` are both making boxes fit the art better. A tighter box
  clips more of the outline, which moves fragments from the art's grid onto the
  fragment grid — *for free, as a side effect of work already planned*. This is
  why S3 below insists the ratio is measured before and after those land.
- **Estimate coverage instead of sampling `nearest`.** D11's argument for whole
  rungs is about *position* — a texel landing on a whole number of real pixels,
  so a whole pixel of camera movement translates the picture. It is not
  obviously an argument about *alpha*. An outline that resolves its own coverage
  at the magnified resolution is a different-looking engine and needs stating as
  such before anyone writes it.

**S3. The ratio is the measurement, and it has to be taken twice.** "How many
silhouette fragments are art-bounded" is one number per frame, and it will move
on its own as the footprint work lands. Take it now, take it after, and record
both — otherwise a change in the picture gets attributed to whatever was being
worked on at the time.

## Phases

### Z1 — the attribution — **done, and it answered a different question**

S1.4 wired through the G-buffer, both views in `debug::View::ALL`, and the
counts at Britain's `(1501, 1659)`. `tests/dump.rs`'s
`the_two_silhouette_layers_are_two_lines_and_a_frame_agrees_about_both` is the
gate: the six colours the branch spells and nothing else, the two views agreeing
pixel for pixel, land and background in neither layer, and — the rule made to
answer wrongly — **every fragment in the box layer carries a measured normal**,
which a `box_edge` that had quietly been reading the art's alpha would break on
the unmeasured remainder of every sprite.

**The finding, and it is not the one the plan expected.** While Z1 was being
built, a parallel change took the box-miss discard out of `statics.wesl` (its own
census: the discard threw away 11.09% of every panel's art and 32.44% of every
whole-tile one — a display case lost its whole top). So the two edges are no
longer two candidate bounds on one outline:

- the **art's** edge is the picture's silhouette, all of it, and it is
  texel-quantised — `scale` real pixels a step, and it cannot be finer;
- the **box's** edge is a seam *inside* the picture, one real pixel a step, where
  the measured region of a sprite ends and the unmeasured remainder carries on at
  the tile's centre with no facing.

That answers the backlog's first 🚩 outright: the zigzag a person points at is
the art's outline, and the fragment-fine line beside it is not a silhouette at
all. It is still a visible line — the two sides of it are lit by different rules
— which is why both layers are kept.

**Britain `(1501, 1659)`, 900×700, night, roof cut, `1:1`:**

```
art only    155        the picture's outline where no box ran out under it
box only     96        the measured region's seam, strictly inside the art
both        473        the seam reaching the outline
            ---
            724 edge fragments of 8075 static ones, 0 mobile
```

Two thirds of the edge is *both*, which is the box fitting the art's outline
well; the 96 are where it does not, and they are the pixels `docs/footprints.md`
is about.

**What Z1 did not get: the widths.** The two edges are two *rules* at every
magnification and two different *widths* only above `1:1`, so the plan's root
claim — one steps by `scale` pixels, the other by one — needs a magnified frame.
A magnified frame is now assemblable and the measurement is the next step; what
stood in the way was not a defect. See "The `4x` frame, and the blocker that was
not one" below.

### The `4x` frame, and the blocker that was not one

Z1 recorded a 🚩🚩 blocker: `frame::assemble` at `4x` over Britain's
`(1501, 1659)` returns 595 quads of land and **zero** static quads, where
`statics::visible_graphics` over the same camera offers 140 graphics for the
atlas. The suspects named were `statics::place` returning `None` and `on_screen`
against `render_width`.

**Neither. The cull is right and the scene was the wrong scene.** Counted through
the same public arithmetic the collector uses, at that eye tile:

| zoom | drawn image | walked | placed | on screen |
|---|---|---|---|---|
| `1x` | 900×700 world px | 3513 | 1300 | **9** |
| `2x` | 450×350 | 2019 | 502 | **0** |
| `4x` | 225×175 | 1381 | 185 | **0** |

The nine statics `(1501, 1659)` draws at `1:1` are one wall run at tiles
`(1484..1487, 1663..1666)`, and every one of them lands in the **top-left corner**
of the image — `x` from −34 to 32, `y` from −20 to 112, four hundred-odd pixels
from an eye that sits at the middle of a 900×700 frame. Magnifying shrinks the
drawn image around that eye (`render_width` is `world_pixels(900)` = 225 at `4x`),
so the cluster leaves the frame, and a cull that kept it would be the defect.
Nothing between `visible_graphics` and `collect` rejects anything it should not:
the same three zooms over `(1486, 1664)` — standing *on* the wall run — collect
109, 54 and 30 statics.

The lesson is the plan's own, not the renderer's: **`docs/parity.md`'s coordinate
is a `1:1` coordinate.** It is where a person stands to look at a lit house
corner, and a magnified frame taken from it is a frame of ground. Every question
this plan asks above `1:1` is asked from `ON_THE_WALLS` — `tests/dump.rs`'s
`(1486, 1664)` — and `draw_britain` takes the eye tile as a parameter so that the
two cannot be confused. `the_magnified_frame_over_a_wall_run_still_collects_its_statics`
is the gate: every rung collects statics over the wall run, and the same `4x`
that collects none over `AT` still collects its land, which is what attributes the
zero to the place rather than to the zoom.

### Z2 — the ratio, before

The count from Z1 taken at the three places `docs/parity.md`'s gate uses, so
that "before the footprints work" is a number and not a memory.

### Z3 — the decision

S2, argued with Z1's picture in hand rather than in the abstract.

### Z4 — the ratio, after

Re-run Z2 once `docs/footprints.md` has landed its fitted boxes. The prediction
this plan makes, and which Z4 either confirms or kills: **the zigzags recede on
their own**, because a box that fits the art clips more of the outline.

## Backlog

- ✅ ~~**A frame at `4x` collects no statics at all.**~~ Not a defect. The nine
  statics that eye tile draws all stand in the corner of its `1:1` image, and
  magnifying shrinks the image around the eye until they are outside it — the
  section above has the counts and the gate. What it leaves behind is a habit
  rather than a bug: **a scene chosen at `1:1` does not carry over to a magnified
  frame**, and a diagnostic that changes the zoom has to change the eye tile too.
- 🚩 **The widths, which is Z1's own unfinished half.** Now unblocked: a `4x`
  frame over `ON_THE_WALLS` assembles, and the claim to measure in it is that a
  run of `art_edge` pixels crossing the outline is `scale` real pixels wide while
  a run of `box_edge` pixels is one. The instrument is the two views Z1 already
  built; what is missing is the scan across the edge and a number.
- ✅ ~~**Whether the zigzag a person points at is even a silhouette.**~~ Answered
  by Z1: it is the art's outline. The finer line beside it is the measured
  region's own seam, not a silhouette. What is *not* ruled out is the third
  candidate the entry named — an interior edge between two art texels of
  different colours, which is not a defect at all and which neither layer marks.
  A person pointing at a magnified frame may still mean that one.
- 🚩 **S1.3, the texel grid drawn.** Colour by the art texel index's parity, so
  the `scale × scale` blocks are visible where the art rules and invisible where
  the fragments do. Not an attribution — a *picture* of the quantum, which is
  what a person actually wants to look at, and it is the one instrument that
  would show the interior-edge case the entry above cannot rule out.
- 🚩 **The minifying rungs are the untested half.** Everything above is about
  magnification, where one texel is several pixels. Below `1:1` several texels
  land on one pixel and the blit's linear sampler is the filter — a different
  regime, with its own artefacts, and no measurement in this repository.
- 🚩 **`docs/pixels.md` owes this plan the art texel's own row.** It is the one
  grid with no type and no document, and it is the grid this whole file is
  about.
