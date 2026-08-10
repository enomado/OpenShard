# Pixels: the inventory, and which grids share a divisor

A living plan, and its own session. The backlog at the end is where the next one
starts.

## The root

**Six grids meet in this renderer and no document lists them.** `docs/camera.md`
D11 names two — the real pixel and the virtual one — and that was the whole
argument it needed at the time. A frame has more than two, they meet inside
single expressions, and the conversions between them are written where they are
used rather than anywhere a person can read them in one sitting.

What that costs is on the record. `docs/parity.md`'s window-parity entry is one
defect, and its whole cause is that **two grids turned out to share a divisor
and nobody knew**: an odd viewport puts the world's centre on a pixel *centre*,
which makes the fragment grid commensurate with the world's integer grid, which
puts a sample exactly on a box's corner, which reaches a tie in
`impostor::meets` that has no right answer. Every step of that is documented in
its own file. The composition is documented nowhere, and the composition is the
bug.

A glossary would not have caught it. **A statement of which pairs are
commensurate would have.**

## The target

One page that answers, without opening a shader:

1. What grids exist, in what units, with what origin.
2. Every conversion between them: which are exact, which round, and which way.
3. **Which pairs share a divisor**, and under what parameters (zoom rung,
   viewport parity, eye fraction) — because a sample landing exactly on a
   discontinuity is the failure this whole plan exists to make predictable.

## What is already known

Collected while chasing the parity defect, and to be checked rather than
trusted — this is the starting list, not the answer.

| Grid | Unit | Type today | Notes |
|---|---|---|---|
| Real (screen) pixel | one physical pixel | none — bare `u32` in `ViewportRect`, `f32` in the shaders' `viewport.size` | what the compositor hands us; the quantum D11 chose |
| Virtual (world) pixel | one pixel of the world at `1:1` | [`WorldPixel`] `i32`, [`ViewPixel`] `i32`, [`WorldPoint`] `f64` | what the world is measured in; `Camera::render_width` counts these |
| Tile | `TILE_WIDTH` = `TILE_HEIGHT` = **44** virtual px | `Point` (`x`, `y`, `z`) | a step in `x` moves *half* a tile on each axis — 22 and 22 |
| `z` step | **4** virtual px (`Z_STEP`) | `i8` inside `Point` | the quantum the wire states a height in |
| Impostor tile space | `z` in **11ths of a tile** (`Z_PER_TILE` = `TILE_WIDTH / Z_STEP`) | bare `f32`/`vec3<f32>` | `impostor::meets`'s `lo`/`hi`; a second `z` unit, related to the first by a constant nobody carries in a type |
| Art texel | one texel of the art file | none | one virtual pixel at `1:1`, `Projection::scale` real pixels magnified |
| Clip space | −1..1 | `vec4<f32>` | the only grid nothing else is measured against |

And the parameters that decide commensurability:

- `Zoom::LADDER` = `[(1,2), (2,3), (3,4), (1,1), (2,1), (3,1), (4,1)]`, `1:1` at
  index 3. Magnifying rungs are whole on purpose (D11); minifying ones are not.
- The viewport's **parity**, per axis, which until `docs/parity.md`'s fix was
  the difference between "no sample is ever on a whole virtual pixel" and
  "every `scale`-th one is".
- The eye's own fraction (`Camera::projection`'s `self.eye.x - rounded.x`),
  which is a multiple of `1/scale` and therefore cannot itself make a
  half-integer whole — worth *stating*, since it is the kind of thing a reader
  assumes rather than checks.

## Phases

### P1 — the census

Every conversion site, found rather than remembered: `project`/`unproject`,
`to_view`/`to_view_exact`/`to_world`/`to_screen`, `Projection`, the three
vertex stages' last line, `ray_from`, `light::Z_PER_TILE`'s readers,
`plan.rs`'s `scale`, the atlases' texel arithmetic. One row each: from, to,
exact or rounding, and which rounding.

*Done when:* the table above is filled from the code and not from this page,
and any row this page got wrong is corrected in place with the site that
proves it.

### P2 — the commensurability statement

For each pair of grids, under which `(rung, parity, fraction)` a point of one
can land exactly on a boundary of the other. This is the deliverable — the rest
is context for it.

*Done when:* the window-parity defect is **derivable** from the table, and the
table says out loud which other pairs are in the same position today.

### P3 — the types that are missing

The real pixel has no type; the art texel has no type; impostor tile space has
no type and carries a `z` in different units from every other `z` in the
engine. `docs/style.md`'s own newtype rule applies, and the reason to do it
here rather than as a sweep is that P2 will have just named which confusions are
*expressible* — a newtype is worth its cost exactly where two domains meet.

*Done when:* each grid P2 shows can collide has a type that stops the collision
at compile time, or a written reason it does not.

### P4 — the gates

An invariant of the form "no primary sample lands on a whole virtual pixel at
any rung, at either parity, at any eye fraction" is a unit test with no GPU in
it: a loop over the ladder and a divisibility assertion. `docs/parity.md`'s fix
is currently held by an argument in a comment; this is where it becomes a gate
that a mutation turns red.

## Backlog

- 🚩 **The art texel is the one grid with no representation anywhere.** It is
  implicit in every atlas rectangle and in `Projection::scale`, and it is the
  grid `docs/silhouettes.md` is entirely about.
- 🚩 **`Z_STEP` and `Z_PER_TILE` are one relationship written twice.**
  `Z_PER_TILE` is *defined* as `TILE_WIDTH / Z_STEP`, so they cannot disagree —
  but a reader meeting `lo.z`/`hi.z` in the impostor has no way to know which of
  the two `z` units they are in without following the definition.
- 🚩 **Nothing states what a `ViewportRect` is measured in when a docked panel
  has moved it.** `dump::read_rect` honours an origin, the blit sets a viewport
  rect, and `Camera` knows an image size — three numbers about the same
  rectangle, in the same unit, owned by three places.
