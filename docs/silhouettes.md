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

**S1. What the split is read off.** The precedent is instructive: the normal
split's first rule was "the solid the fragment names" and a dump of it *refuted*
that rule, because a static can meet a real volume the grid holds no solid for.
The rule that survived was read off the vector itself. So the criterion here has
to be something the fragment already computes, not something inferred beside it.

1. **`Meeting::outside`, carried into the G-buffer.** It already exists in the
   shader, it is already the number `docs/lighting_rebuild.md` phase 6 asks its
   own done-when to carry, and it is exactly "how far this fragment is outside
   the box it names". A fragment at the box's rim sits at the tangent limit; one
   in the art's interior sits near zero. **Cheapest, and it needs no
   neighbourhood.** Start here.
2. **A neighbourhood test in the blit.** A fragment is on a silhouette if a
   4-neighbour holds no static; classify by which mask ended. Answers the
   question directly and costs a second pass over the G-buffer, which the blit
   is already doing.
3. **The texel grid, drawn.** Colour by the art texel index's parity, so the
   `scale × scale` blocks are visible where the art rules and invisible where
   the fragments do. Not an attribution — a *picture* of the quantum, which is
   what a person actually wants to look at, and it composes with either of the
   above.

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

### Z1 — the attribution

S1's cheapest candidate wired through the G-buffer, both views in
`debug::View::ALL`, and a picture of Britain's `(1501, 1659)` with the count of
fragments in each layer beside it.

*Done when:* every silhouette fragment in one real frame is in exactly one of
the two layers, and the count of each is written here — with the same mutation
witness the normal split used, a rule made to answer wrongly and the picture
that shows it.

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

- 🚩 **Whether the zigzag a person points at is even a silhouette.** It may also
  be an interior edge — two adjacent art texels of different colours, magnified.
  Nothing about that is a defect at all, and telling the two apart is a question
  Z1's picture answers on the way past. State which one the complaint was about
  before spending a phase on the other.
- 🚩 **The minifying rungs are the untested half.** Everything above is about
  magnification, where one texel is several pixels. Below `1:1` several texels
  land on one pixel and the blit's linear sampler is the filter — a different
  regime, with its own artefacts, and no measurement in this repository.
- 🚩 **`docs/pixels.md` owes this plan the art texel's own row.** It is the one
  grid with no type and no document, and it is the grid this whole file is
  about.
