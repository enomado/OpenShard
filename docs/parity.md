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

(Eleven, counted properly while P1 was being done: `tests/onsite.rs`,
`examples/boxes.rs`, `client/render/src/scene.rs` and two more in a **different
crate**, `client/artscan`'s `examples/probe.rs` and `examples/grid.rs`. That the
first count was low by four is itself the point — nobody holds the list either.
P2 has all of them.)

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

### P1 — the assembly, extracted ✅ 2026-08-10

`client/render/src/frame.rs`: [`frame::assemble`] takes one [`frame::Inputs`] and
returns one [`frame::Frame`] — the lighting, the ground quads and one
[`StaticGeometry`] with the server's items already absorbed into the map's. The
app's own order is kept and stated there (the grid before the pictures, since a
drawn row carries the number the grid gave the static it draws). The app and
`isolated_scene` are its first two callers, and neither calls a collector any
more.

**The open design question is settled** (it was the third backlog item): the
assembly takes *borrows in a struct built at the call site*, `bake: Option<&mut
Bake>` included. The app hands over `&self.map`, `&self.items` and `&mut
self.occlusion_bake` in one literal — they are disjoint fields, so nothing had to
move out of `App` and nothing is cloned.

Eighteen fields, no `Default`, and three of them are new as fields rather than as
constants somebody passed:

- **`sky: Option<Ambient>`** — `None` is the client's daylight, where no grid is
  built at all. It is what F10 actually switches, which is worth stating because
  it is *not* the impostor field below.
- **`impostor: Impostor`** — `Met` or `Billboards`, D3's field. The app passes
  `Met` always; `isolated_scene`'s `OPENSHARD_SCENE_IMPOSTOR=0` is the only
  caller that asks for the other.
- **`sun` / `carried` / `view`** — the three things the app used to do to
  `Lighting` *after* collecting it, three hundred lines further down the
  function. They are inputs now, and the comment where they used to be says
  nothing may touch the lighting between the assembly and the blit. A frame the
  client draws and a frame a tool dumps are the same frame only for as long as
  neither has an adjustment of its own afterwards.

*Done:* `examples/isolated_scene.rs` dumps seven planes of Britain's
`(1501, 1659)` — Lit, Place, Kind, Height, Normal, Solid, Occluders — **byte for
byte identical** before and after the extraction. `cargo test -p
openshard-client-render -p openshard-client-app` is green (454 + 142 and the GPU
suites), clippy silent.

*Not done, and it is the first backlog item below:* **the app's own half was
verified by reading rather than by a gate.** The client has no frame dump, so
there is nothing to compare; what was checked is that every argument arrives
where it used to, that `Lighting::NONE.sun` is `None` so the unconditional
`lighting.sun = sun` is the old conditional, and that nothing between the old
collect site and the old adjustment site reads anything of `lighting` but
`occlusion`.

### P2 — the remaining callers

`tests/cost.rs`, `tests/frame.rs`, `tests/traced.rs`, `tests/attachment.rs`,
`tests/onsite.rs`, `examples/two_cubes.rs`, `examples/boxes.rs`, and
`client/artscan`'s `examples/probe.rs` and `examples/grid.rs`. **Nine, not the
five this plan first named** — the two in `artscan` are a different crate and
were missed entirely, and `onsite.rs` and `boxes.rs` were counted as scene
fixtures rather than as assemblies. Some of these build synthetic scenes with no
map at all and will pass fields the app never does; that is what the struct is
for.

`client/render/src/scene.rs`'s `Scene::lighting` is a tenth, and a different
kind: it collects lighting and nothing else, for scenes that have no art to draw.
Whether it becomes an `Inputs` with empty atlases or stays what it is, is a
decision for P2 and not an oversight.

*Done when:* the four collectors have no caller outside the assembly. D3 lands
here: `cost.rs` prices a real grid, and its number changes — record both.

### The dump ✅ 2026-08-10 — P3's prerequisite

**Both ends can now be asked for a frame, and both answer in the same bytes.**
This was the first backlog item and it stood in front of P3: the gate needs two
frames of one place and only the tools' existed.

`client/render/src/dump.rs` is the one readback. [`dump::planes`] draws one
assembled frame once per [`debug::View`] — the same blit, the same lighting, the
same world image and G-buffer, nothing collected in between — and hands back a
PNG a plane. [`dump::read_rect`] is the copy underneath it, and it pads its own
rows and honours its own origin, which is what lets a dump come off an
arbitrarily-sized window and off a viewport a docked panel has pushed away from
the corner.

- **The client dumps on F12.** `App::frame_dump` is armed by the key and spent in
  `App::draw` after the frame's own submit: the ordinary frame, drawn by the
  ordinary passes, blitted again into a texture of its own once per plane. Not
  the surface — what is presented has the HUD and the solids overlay on it, and a
  tool's frame has neither. One directory a press, `<root>/frame-<n>/<plane>.png`,
  under `OPENSHARD_FRAME_DUMP_DIR` or the system temp. **Not**
  `OPENSHARD_FRAME_DUMP`, which the tools already read as the *file* their one
  picture goes to; one name meaning two things is the divergence this plan is
  about, in miniature.
- **`frame::Inputs::summary` is the other half of a dump.** A picture nobody can
  reproduce is what every dump before this one was: two frames that differ said
  nothing about *which* input differed, and the client's arguments were readable
  only by reading `App::draw`. Every field gets a line — including the four that
  cannot be stated, which say so — so two dumps diff. `isolated_scene` writes it
  beside its picture as `<dump>.inputs.txt` and prints it; the client writes it as
  `inputs.txt` in the dump's own directory.
- **`tests/dump.rs` assembles Britain's `(1501, 1659)` headlessly** — the map's
  own statics, the player's own cutaway, night with a flame in hand — draws it,
  and dumps every plane. It gates one picture per view at the size asked for, the
  view *reaching the shader* (Lit, Place and Normal cannot agree on a real
  street), and a readback off the corner at an unaligned width. Both controls
  were witnessed by mutation: made to ignore the view, and made to ignore the
  origin — each turns the gate red.
- Two hand-rolled readbacks died with it (`plan.rs`'s and `isolated_scene`'s),
  and with them `OPENSHARD_SCENE_VIEWPORT`'s 256-byte alignment rule, which was
  only ever that copy showing through.

**The first press killed the client, and that is the entry worth keeping.** The
dump drew into a texture of the *surface's* format, on the reasoning that the
blit's pipeline is built for it — and a surface is whatever the compositor
offered. Here it is `Rgba16Float`: eight bytes a texel, against a readback
measuring a row as `width * 4`, which is not a shorter row but a copy `wgpu`
refuses outright. Every test passed, because every test drew into
[`blit::WORLD_FORMAT`] — the tools' own format, four bytes, the one place this
could not go wrong.

Two things came out of it, and the second is the point:

- `dump::read_rect` takes the texel's size from `texture.format()`, and
  `tests/dump.rs` reads a rect out of `Rgba8Unorm`, `Bgra8Unorm` and
  `Rgba16Float` — a test that needs neither client files nor a drawn frame, and
  that fails on the old arithmetic.
- **The dump draws into `WORLD_FORMAT` and builds its own blit pipeline for it.**
  Even a four-byte surface would have been the wrong four: `Bgra8Unorm` reads
  back with red and blue swapped and nothing says so, and a dump exists to be
  compared against `isolated_scene`'s picture, which has always been RGBA8. The
  surface's format is a fact about the compositor, and a comparison cannot depend
  on one.

*Not done, and it is the first backlog item below:* **no F12 press has yet
written a directory.** The failure was witnessed; the success has not been.

### P3 — the gate

A test that assembles one real place twice, once with the client's inputs and
once with the tool's, and compares the G-buffer plane by plane.

**Its prerequisite is built** — see the dump above, and `tests/dump.rs` is half
of it already: it assembles a place the client's way and reads every plane back.
What is left is the second frame in the same test, built the tool's way
(`isolated_scene`'s synthetic map, its anchor, its cutaway), and the comparison —
per plane, counting differing pixels, with the inputs that must differ set equal
first (D6). The two summaries diffed are what says they were.

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

- 🚩 **The tool reads no shard database, so half of what a player is looking at is
  not in its frame.** Found on 2026-08-10, by answering the wrong question at
  length: a person asked about a cabinet they could see at Britain's
  `(1504, 1655)`, and `tile_probe`, `onsite.rs`, `geometry_census.rs` and
  `isolated_scene.rs` agree there is no cabinet there — because all four read
  `map`/`statics.mul` and the cabinet is a **`decorations` row**: `0x0A97`/`0x0A98`
  "bookcase" at `(1505, 1656, 27)` and `(1506, 1656, 27)`. The session spent its
  first half explaining the nearest map static instead (`0x0B3E` "counter"), which
  is a different graphic with a different box.
  <br>
  The knob that closes it by hand exists — `OPENSHARD_SCENE_EXTRA` takes
  `x,y,z,graphic` and the defect reproduced at once once the two rows were
  transcribed into it — and that is exactly the problem: **a hand-transcribed
  input is not a parity input.** `Inputs::ground_items` is already a field, so
  what is missing is a reader that fills it: point the tool at `openshard.toml`'s
  own `database`, pull `items` (`loc_kind = 0`, the facet, the rect the radius
  covers) and `decorations` (a JSON blob, so `json_extract`) for the same window
  the statics come from, and let a knob turn it off rather than off by default.
  <br>
  Until then "it does not reproduce in the tool" is a **false negative for
  everything the server placed**, and nothing in the summary says so — the frame
  looks like a frame, which is this document's own root sentence.
- 🚩 **Map statics reach the tool's frame through `items::collect` and the
  client's through `statics::collect`.** `isolated_scene` builds a synthetic map
  that carries no statics at all (`Map::from_blocks`) and pushes the real map's
  statics into `Inputs::ground_items` as `GroundItem`s. Both paths call
  `statics::push_volumes` with the same `boxes_of`, so the *boxes* agree; what
  does not obviously agree is everything around them — the owner key, the sort
  (`items::collect` sorts by `depth::Order` and ties by the caller's order, which
  for the server is serial and here is a nested `x`/`y` loop), and `highlight`.
  D1 gave the two a shared assembly; it did not give them a shared *route
  through* it, and no gate compares the two routes on one place.
- 🚩 **No F12 press has written a directory yet.** The first one panicked on the
  surface's format (above, now fixed and gated); the fixed path has been read and
  not run. Press it once in the live client and look at
  `/tmp/openshard-frame/frame-0/` — thirteen pictures and an `inputs.txt`. Worth
  noting what the failure cost, since it is this plan's own argument in
  miniature: a client that dies on a diagnostic keypress takes the frame a person
  was looking at with it, which is exactly the instant the dump exists to keep.
  A dump that cannot be taken twice from one session is a tool with a rationed
  answer.
- 🚩 **The tool advances no animation clock at all.** The first thing
  `Inputs::summary` caught, on the first run: `isolated_scene` passes
  `StaticAnimations::default()` — nought cycles — where the client builds 1068
  out of `animdata.mul`. So every animated static in a tool's frame draws its
  base graphic while the client draws whatever the cycle is on, and a fire is the
  most likely thing anyone points either at. A field on `Inputs` already; what is
  missing is the tool building the table and a knob for the instant. P2's work,
  named here because it is the first *found* divergence rather than a suspected
  one.
- 🚩 **A summary cannot state the files it read.** `map` says the facet and the
  static count, the atlases say their sizes, and `tiledata` says only that it is
  the client's own table. Two frames off two different installs compare equal in
  every line. A digest of the loaded tables would close it, and until then the
  gate's answer is "equal given the same client files".
- 🚩 **Every GPU test binary keeps its own `gpu()` and `client_dir()`.**
  `tests/frame.rs`, `tests/dump.rs`, `tests/cost.rs` and the rest each carry the
  same adapter request and the same environment lookup, because integration test
  binaries share nothing without a `mod common`. Four copies of a device request
  that has to ask for `gbuffer::required_limits` is four places to forget it.
- 🚩 **A parity gate needs a place where the lighting is reachable.** At Britain's
  `(1501, 1659)`, a torch dropped in by `OPENSHARD_SCENE_EXTRA` at the client's
  own default brightness and reach changes the Lit plane **not by one byte** — it
  is shut inside a house and everything its pool would touch is under a roof. The
  seven-plane comparison P1 was checked with only became sensitive to the
  lighting at `_BRIGHTNESS=4 _REACH=3`. A gate laid on a frame with no flame that
  reaches anything is green about the geometry and blind about the light, and it
  would not say so. P3's three places have to be chosen for a lit pixel, not only
  for a house.
- 🚩 **The cost of a Britain-sized synthetic map is unmeasured** (D4). It is a
  `Map::from_blocks` of roughly 200×210 blocks with a land lookup a cell —
  cheap in principle, unmeasured in fact.
- 🚩 **`examples/two_cubes.rs`, `synthetic_stair.rs` and `boxes.rs` build meshes
  by hand** — four hand-built diagnostic scenes, each its own copy, called out in
  `statics.rs`'s own note at `push_mesh`'s grave. They are not frame assemblies
  and may not belong in P2; decide when P2 reaches them.
- 🚩 **No gate holds that a debug view is drawn from the same planes the lit
  frame is.** `View::Solid` came out black in `cost.rs` for a whole session and
  nothing said so. A view that draws nothing on a frame that drew something is a
  finding, and it is cheap to assert.
