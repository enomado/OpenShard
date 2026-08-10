# Parity: one frame, however it was asked for

A living plan. The backlog at the end is where the next session starts.

## The root

**Nobody holds parity, so it is not a property — it is a coincidence.** A frame is
assembled by calling `light::collect`, `ground::collect`, `statics::collect` and
`items::collect` in the right order with the right arguments, and that sequence
is written out by hand in at least seven places: `client/app/src/lib.rs`,
`examples/isolated_scene.rs`, `examples/two_cubes.rs`, `tests/cost.rs`,
`tests/frame.rs`, `tests/traced.rs`, `tests/attachment.rs`. Every one of them is
free to pass a different cutaway, a different grid, a different clock — and each
of them does.

What that costs is not theoretical. In one session (2026-08-10), chasing a
one-pixel artefact a person could see in the live client:

- `tests/cost.rs` collects statics against `Occlusion::EMPTY` **on purpose**, so
  every fragment in its frame takes the billboard fallback. Its `View::Normal`
  is not the client's, its `View::Solid` is uniformly black, and nothing said so
  — the frame it dumps looks like a frame.
- `examples/isolated_scene.rs` passed `None` for the occlusion bake where the
  client passes one, `Cutaway::OPEN` where the client passes the player's own,
  and translated every scene onto a synthetic anchor near the origin where the
  client works at Britain's coordinates. Each was found by reading, not by a
  gate, and each changed the picture: the anchor alone moves 760 pixels, 746 of
  them one-pixel runs.
- The artefact itself reproduced in the tool the whole time. It went unseen for a
  session because the tool's frame was being *searched* rather than *compared* —
  there was nothing to compare it to.

**The client is the thing that is broken, and the tool is the only thing that can
be inspected.** So long as the two are different assemblies, a defect visible in
one and absent in the other says nothing about either.

## The target

**One assembly, one set of inputs, and a gate that a frame is the same frame
whoever asked for it.**

Parity by *construction* first and by *comparison* second. A comparison gate over
two hand-written assemblies only tells you they diverged; a shared assembly makes
most of the divergences unexpressible, and then the gate is about the handful of
inputs that are genuinely allowed to differ.

## The decisions, made

**D1. One function assembles a frame, and every caller goes through it.** It
lives in `client/render` — the app is a caller like any other, not the reference
implementation. Nothing outside it may call `light::collect` /
`statics::collect` / `items::collect` / `ground::collect` in sequence.

**D2. Its inputs are one struct, and a caller that wants to differ says so by a
field.** Not by omitting a call, not by passing a different constant at one of
four call sites. Every field this session found divergent is a field: the
cutaway, the bake, the atlas, the tuning, the flame and animation clocks, the
ground items, the camera. A field that has no honest default has no default.

**D3. `Occlusion::EMPTY` stops being a way to assemble a frame.** A caller that
wants to price the pass without the impostor asks for that by a field on the
inputs, and the field is named for what it does to the picture, not for what it
saves. `tests/cost.rs`'s frame becomes a real one.

**D4. The tool builds at real coordinates by default.** `OPENSHARD_SCENE_ANCHOR_REAL`
exists and is off because the synthetic map near the origin is cheaper. Once the
cost of a Britain-sized `Map::from_blocks` is measured and acceptable, the
default flips and the knob keeps the old behaviour for whoever wants it. An ulp
at 1660 is sixteen times an ulp at 100, and every tie at a shared plane is
decided in `f32`.

**D5. The gate compares planes, not pictures.** The lit frame folds every
disagreement into one colour; the G-buffer's planes — position, normal, solid,
place — each answer a different question, and a parity failure names which. The
gate reports the count of differing pixels per plane, and zero is the target.

**D6. A deliberate difference is a listed one.** Where a caller sets a field that
must change the picture (no roofs, no impostor, a different zoom), the gate is
run with that field *equal* on both sides. There is no allowance, no tolerance
and no ignore-list: an input that differs is set the same, or the case is not
gated.

## Phases

### P1 — the assembly, extracted

One function, the app's own order (the grid before the pictures, since a drawn
row carries the number the grid gave the static it draws), and its inputs a
struct. The app and `isolated_scene` are its first two callers.

*Done when:* neither `client/app/src/lib.rs` nor `examples/isolated_scene.rs`
calls any of the four collectors directly; `cargo test --workspace` is green; the
frames both produce at one place are pixel-identical to the ones they produced
before the extraction, plane by plane. That last one is the whole point — a
refactor that changed a picture is a refactor that found a bug, and it should be
reported as one rather than absorbed.

### P2 — the remaining callers

`tests/cost.rs`, `tests/frame.rs`, `tests/traced.rs`, `tests/attachment.rs`,
`examples/two_cubes.rs`. Some of these build synthetic scenes with no map at all
and will pass fields the app never does; that is what the struct is for.

*Done when:* the four collectors have no caller outside the assembly. D3 lands
here: `cost.rs` prices a real grid, and its number changes — record both.

### P3 — the gate

A test that assembles one real place twice, once with the client's inputs and
once with the tool's, and compares the G-buffer plane by plane.

*Done when:* it is green at three places with a house on them, and red when any
one input is deliberately changed. The second half is the positive control and
is not optional: a gate that cannot be made to fail is not gating.

### P4 — the geometry, in census order

`examples/geometry_census.rs` counts what each box claims. Over 11,184 statics
around Britain's `(1501, 1659)`: 3.2% a fitted prism, 39.6% a lid, 25.4% panels,
31.8% a whole tile standing in for a shape nobody has. Crossed with that, 32.7%
are a point of no primitive and 15.1% are a `CLEAR` piece handed a box with real
height anyway.

The order follows the counts and the blast radius:

1. **A floor is a body** — 39.6% of the world is a plane, and
   `docs/lighting_rebuild.md`'s floor entry already scopes what changes:
   `Solid::box_of` gives a lid `bottom - 1 .. top`, the walk's `Edges::NONE` arm
   becomes an opacity, `occlusion::merge` folds floors as bodies, and
   `impostor::meets`'s "a lid has no side face" guard dies. The thickness is one
   `Z_STEP`, which is the quantum the wire states a height in.
2. **A `CLEAR` piece with a box** — 15.1%, and the pair that ends both the
   cornice entry and the floor entry. Two honest answers and the plan picks one:
   it stands in the grid (and has a name), or it is given no box (and is a
   billboard). Phase 6c refused the second; this is where that gets revisited
   with the census in hand.
3. **The whole-tile stand-in** — 31.6%, the expensive one, because reducing it
   means measuring more art rather than writing a rule.
4. **`PANEL_THICKNESS`** — one slab straddling the tile boundary instead of two
   inset ones, which is `docs/lighting_rebuild.md`'s own backlog item.

Each of the four re-runs the census as its own done-when, and the numbers go in
`docs/lighting_rebuild.md`'s census section beside the ones above.

## Backlog

- 🚩 **P1 is not started.** The extraction is the whole of it; everything below
  waits on it.
- 🚩 **The cost of a Britain-sized synthetic map is unmeasured** (D4). It is a
  `Map::from_blocks` of roughly 200×210 blocks with a land lookup a cell —
  cheap in principle, unmeasured in fact.
- 🚩 **The app's own inputs are not all reachable from a test.** `App` owns the
  bake, the clocks and the item list; whether the assembly takes them by
  reference or the app hands over a built struct is P1's one open design
  question, and it should be settled by writing the signature rather than by
  arguing.
- 🚩 **`examples/two_cubes.rs`, `synthetic_stair.rs` and `boxes.rs` build meshes
  by hand** — four hand-built diagnostic scenes, each its own copy, called out in
  `statics.rs`'s own note at `push_mesh`'s grave. They are not frame assemblies
  and may not belong in P2; decide when P2 reaches them.
- 🚩 **No gate holds that a debug view is drawn from the same planes the lit
  frame is.** `View::Solid` came out black in `cost.rs` for a whole session and
  nothing said so. A view that draws nothing on a frame that drew something is a
  finding, and it is cheap to assert.
