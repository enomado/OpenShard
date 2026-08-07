# The reference path tracer

A second renderer of the same scene, written to have none of the first one's
ideas. Where `light.rs` and `blit.wesl` decide a shadow by walking a grid of
tiles — carrying a cell, stepping to a boundary, asking an occluder whether it
is the fragment's own — this one has a ray, a box, and where they meet. No
tile appears in it anywhere.

That is the whole claim: **a defect that can only be stated in the walk's own
vocabulary cannot be reproduced here by construction**, rather than by
coverage. It is also a *third party*. When `light::sample` and `blit.wesl`
disagree, both are implementations of one formula and neither can say which is
right; this one is not in that argument.

Written against `crates/client/pathtrace/` (the whole crate) and
`crates/client/render/examples/boxes.rs`'s own `pathtrace_comparison`.

Satellite of [`lighting.md`](lighting.md), beside
[`lighting_raymarch.md`](lighting_raymarch.md) — that file is about keeping the
CPU and GPU copies of one walk numerically identical, this one is about having
something that is not either of them.

## Where it lives, and why it is a crate

`crates/client/pathtrace`, **with no dependencies at all** — not on
`openshard-client-render`, not on a shared geometry helper, not on a shared
constant. The arrow only points the other way: the render crate takes it as a
dev-dependency for `examples/boxes.rs`.

That direction is the design. A reference that can reach the thing it checks
will eventually share an answer with it, and the sharing is invisible in the
result — two pictures that agree because they computed the same thing twice
look exactly like two pictures that agree because both are right.

Being a crate rather than another module under `examples/` also buys the one
thing a reference cannot do without: `cargo test --workspace` runs its own
tests. There are 46 of them, and they are what says the reference is not
itself the defect — worked crossings by hand, two proptest laws over the slab
arithmetic, the camera recovery against a projection with `f32` noise in it,
and the estimator on scenes whose answer is known on paper.

## What it takes from the renderer, in full

Two things, and both arrive as **values rather than as formulas**.

**The camera.** `camera::Parallel::measure` takes the world-to-pixel map as a
black box — a closure `boxes.rs` builds out of the renderer's own
`project_exact` and `Camera` — probes it along three axes, recovers the affine
map, and then *checks that assumption* on four probe points it did not measure
from, failing loudly if the map is not affine to within a hundredth of a pixel.
The view direction falls out as the null space of the recovered 2×3 matrix, one
cross product, no linear solve.

Two consequences worth having written down:

- Nothing in the tracer knows a tile is 44 pixels across or that a `z` unit
  lifts a sprite four. A change to the projection reaches the reference camera
  automatically, because the reference camera is *measured* from it every run.
- The recovered kernel of OpenShard's own projection is `(1, 1, 11)`, which is
  `Z_PER_TILE` — arrived at from the picture rather than from the constant.
  The probe measures through an `f32` path (`Camera::to_view_exact` narrows
  there) so the central difference is taken over a 32-tile baseline, which
  divides that noise by 64.

**The metric.** `light::Z_PER_TILE`, read from the engine rather than written
down again, is what converts the world into the isotropic units the tracer's
scene is in. Visibility would survive getting this wrong — it is invariant
under any affine change of coordinates — which is exactly why it has to be
right anyway: a cosine, a solid angle and a penumbra's width are not, so a
scale error would show up in the soft modes as a plausible-looking picture
rather than as a failure.

Everything else — the boxes, the flame, the frame size — the caller states.

## The two modes

**Degenerate** is the gate. A point emitter, one path per pixel, no bounces:
every random draw is either never made or has one possible outcome, so the
Monte Carlo estimator collapses to a single deterministic visibility test and
the picture is exact. `Image::is_exact()` says so by asking the emitters and
the settings, not by trusting the constructor, and `boxes.rs` asserts on it —
a soft-shadow render disagreeing with a hard-shadow one is not a finding, and a
comparison that cannot tell the two apart would report it as one.

**Full** is a picture, not a check. A spherical emitter, hundreds of paths, a
cosine term, diffuse bounces and a sky. It is *not* compared against anything:
none of what it adds exists in the renderer, so every pixel would "disagree".
It is there to be looked at — 512 samples and 3 bounces over a 512×512 frame is
about 13 seconds, single-threaded.

Both are one body of code. A reference with a separate "fast exact path" is two
implementations again, and the one that gets compared against the renderer
would be the one nobody looks at.

## How a pixel is compared

Only where the two agree about **what surface is there**. The renderer's answer
comes from the `place` attachment (which instance row drew this pixel); the
tracer's is its own nearest hit. Where they differ, the pixel is counted under
`disagree about which surface is there` and no further — an isometric painter's
order and a ray's own nearest hit are two different answers to "what is in
front", and filing that under lighting would name the wrong defect.

Three further splits, each of which exists because folding it in would have
produced a large, confident, wrong number:

- **Out of reach is not shadowed.** A pixel outside a torch's radius is dark
  because of the *radius*. `Visibility::within_reach` carries it separately, as
  the renderer's own debug view already spends a colour doing.
- **Facing away is not shadowed.** A surface whose normal points away from the
  flame is dark because of where it *points*; no shadow ray was ever a
  possibility. `Visibility::faces_light` carries it, and those pixels are held
  out of the comparison and reported on their own line.
- **An edge is not an interior.** The two renderers light *different points* —
  the tracer lights the world point under a pixel's centre, the shader lights
  the fragment the rasteriser wrote, quantised to a hundred-and-twenty-eighth
  of a tile. Half a pixel decides the answer exactly at a shadow's edge and
  nowhere else, so a disagreement is only reported when neither picture has an
  edge running through the pixel's own eight-neighbourhood.

And it counts what it checked. `compared` is asserted non-trivial: a detector
that silently compares nothing reads exactly like a detector that found
nothing.

## What it says today

Run over the three scenes `boxes.rs` builds, at their own defaults:

| scene | compared | interior | edge | different surface |
|---|---|---|---|---|
| `tree` | 256,826 | **0** | 168 | 59 |
| `line` | 250,377 | **0** | 90 | 833 |
| `pair` | 256,792 | **0** | 190 | 0 |

**Zero interior disagreements on all three.** Every pixel where the two
renderers agree about which surface is there, and where neither has a shadow
edge running through its neighbourhood, they agree about whether the flame
reaches it. Against a renderer that shares no arithmetic and has no notion of a
tile.

That is a much stronger statement than the existing oracles could make. It also
bounds the open residuals in `lighting_raymarch.md` and `lighting_height.md`:
whatever is left of them lives inside one pixel of a shadow's own edge, or in
the two categories below, and not in the interior of any lit or shadowed
region.

### The one real difference the tracer found

**The renderer lights surfaces that face away from the flame.** There is no
cosine term and no back-face test in the shipped model, so a box's south face
with the torch to the north is drawn lit:

| scene | back-facing pixels | of which the frame draws lit |
|---|---|---|
| `tree` | 5,259 | 4,878 |
| `line` | 10,934 | 6,432 |
| `pair` | 5,352 | 2,700 |

This is a difference between the two **light models**, not a bug report: UO's
own art has no normals, and a face's brightness in the client comes from the
sprite. Whether the mesh-face path — which *does* have a normal, and states it
in `Stance` — should use it is a design question this file raises and does not
answer. It is recorded here because the number is large, because it was
invisible to every oracle before this one, and because
`docs/lighting_height.md`'s own recent "the oracle had no half-space test, and
most residuals were that" is the same fact arrived at from the other side.

### And one the tracer found in itself

Worth recording because it is a trap anyone building a reference will meet.
The first version used the textbook area-light estimator — sample the emitter's
surface, weight by its own cosine and `1/d²`, divide by the sampling density.
That estimator is exactly right, and it is exactly right *only with a physical
falloff*: its near-field behaviour is a cancellation between an emitter cosine
going to zero and a `1/d²` growing without bound.

The renderer's falloff is `(1 - d/reach)²`, which does not grow. Put the two
together and only the collapse survives — a wide emitter near the floor drew a
**dark patch directly beneath itself**, exactly where it should be brightest.

The fix is to separate the emitter's two roles: brightness is point-source
photometry from the emitter's centre through whichever curve the caller chose,
and the emitter's *extent* is used for one thing only — where to aim the shadow
ray, which is what a penumbra is made of. `Light::sample`'s own doc carries the
argument. The picture is what found it; no test would have, because no test
knew to ask about a spot on the floor under the torch.

## Running it

```sh
# The gate. On by default, every run of the tool.
OPENSHARD_FRAME_DUMP=/tmp/tree OPENSHARD_BOXES_SCENE=tree \
    cargo run --release -p openshard-client-render --example boxes
```

Writes `<path>_pathtrace.ppm` — the frame's own shadow decision and the
tracer's, side by side, grey where a pixel was not compared. `OPENSHARD_BOXES_PATHTRACE=0`
skips it.

```sh
# The picture. Off by default; the sample count is the switch.
OPENSHARD_BOXES_PATHTRACE_SAMPLES=512 OPENSHARD_BOXES_PATHTRACE_BOUNCES=3 \
OPENSHARD_BOXES_PATHTRACE_EMITTER=0.5 OPENSHARD_BOXES_PATHTRACE_EXPOSURE=3.0 \
    cargo run --release -p openshard-client-render --example boxes
```

Writes `<path>_pathtrace_full.ppm`. `_EMITTER` is the emitter's radius in tiles
(`0` is not a point — use the degenerate mode for that), `_EXPOSURE` is a plain
linear multiplier before the sRGB curve.

## Status

Built and current:

- `crates/client/pathtrace`, zero dependencies, 46 tests under
  `cargo test --workspace`.
- The camera is measured from the renderer's own projection and asserts its own
  affine assumption; nothing restates the projection's formula.
- Degenerate mode is a gate, runs by default in `examples/boxes.rs`, and reads
  zero interior disagreements on `tree`, `line` and `pair`.
- Full mode renders soft shadows, a cosine term, indirect light and ambient
  occlusion over the same scene and camera.

## Backlog

- **The gate is not in CI.** It runs inside `examples/boxes.rs`, which needs a
  GPU adapter, so `cargo test --workspace` never reaches it — the same
  limitation every other oracle in that file has. The tracer itself needs no
  GPU; what needs one is the frame it is compared against. A test that renders
  the frame offscreen under `tests/` would close this, and would be the first
  thing to break if the walk regressed.
- **Nothing runs it over a real map.** All three scenes are hand-built boxes.
  A static off `tiledata` is a sprite with a `Solid` approximation behind it,
  and the tracer would be checking the approximation rather than what a player
  sees — worth doing anyway, but the limit has to be stated in the same breath
  or a green tracer will be read as a green shard.
- **`line`'s 833 surface disagreements.** Where the painter's order and the
  ray's nearest hit differ. Almost certainly the two whole-tile boxes' shared
  silhouette edge, but nobody has looked; `tree` reads 59 and `pair` reads 0,
  and a scene-dependent number that nobody has explained is a number that could
  be hiding something.
- **The cosine question above** wants an answer, not just a count. A mesh face
  carries its own normal already.
- **Single-threaded.** 13 seconds for 512 samples over a 512×512 frame is fine
  for looking at a picture and too slow for a sweep. The pixel loop is
  embarrassingly parallel and each pixel's stream is addressed by its own
  index, so it is already deterministic under any evaluation order — but that
  is a dependency (`rayon`) on a crate whose whole design is having none.
- **The edge exclusion is a neighbourhood test, not a bound.** It correctly
  refuses to report sub-pixel disagreements, and it would also hide a genuine
  one-pixel-wide defect that happened to sit along a shadow edge. Rendering the
  tracer at a higher resolution and downsampling would replace the heuristic
  with a real sub-pixel answer.
- **No sun.** `Lighting::sun` is a directional light the renderer supports and
  the tracer has no counterpart for, so a sunlit scene cannot be compared at
  all today.
