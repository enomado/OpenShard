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

**Then it worked, and this is what a press leaves.** Two dumps from one session
at Britain, `/tmp/openshard-frame/frame-0/` and `frame-1/`: thirteen pictures of
`1919x2077` — the viewport at the magnifying zoom the client was on — and an
`inputs.txt` each. Their diff is four lines, and every one of them is something a
person did between the two presses:

```
camera     eye tile (1503, 1654) → (1505, 1657)      the body walked
sky        None → Some(Ambient { … })                F10, night on
flame_time 3.443795s → 13.605293s                    ten seconds passed
view       lit → normal                              F11
```

That is the whole of what the summary is for. Nothing else moved — the same
facet, the same 2,906,871 statics, the same cutaway (`max_z: 47`,
`no_draw_roofs: true`: the player is indoors), the same tuning, `bake = kept
across frames`, `impostor = Met` — so a difference between these two frames is a
difference in those four lines and in nothing that had to be reconstructed by
reading `App::draw`.

One line of `inputs.txt` reads oddly beside the directory and is deliberately
left alone: `view` is what the *window* was showing, while each picture beside it
is named for the plane it is. A note explaining that would be a line the tool's
summary does not have, and the two exist to be diffed.

### The shard's own furniture ✅ 2026-08-10 — the tool stops answering about half a street

**`isolated_scene` reads the shard's database, so what the server placed is in
its frame.** This was the first backlog item, and what it cost is written there:
four tools agreed there was no cabinet at Britain's `(1504, 1655)` because all
four read `statics.mul`, and the cabinet is two `decorations` rows.

`examples/shard/mod.rs` is the reader, a shared example module in
`examples/oracle/mod.rs`'s own shape and for its own reason — the alternative is
a second copy of two queries the day `tests/` wants them. **It cannot be a
library, and the rule this time is the workspace's**: `openshard-persistence` is
a *server* crate and `crates/client/*` may not depend on one, so the two tables
are read by SQL rather than through its `Store`. That duplicates seven column
names and six JSON keys, bounded on purpose: a rename in `sqlite.rs`'s `SCHEMA`
fails here loudly — SQLite has no such column — rather than quietly returning
nothing, which is the failure mode the whole entry is about.

- **`items` where `loc_kind = 0`** (the ground; `1` is inside a container and `2`
  is worn) and **`decorations`**, whose record is one JSON blob and so is
  windowed by `json_extract`. Both over the same `_AT ± _RADIUS` rectangle and
  the same facet the map came off — named once now, as `FACET`, because three
  readers agreeing about it by writing `0` out three times is three places to
  stop agreeing.
- **On by default, and every way it can fail to read is a panic naming
  `OPENSHARD_SCENE_SHARD=0`.** An in-memory `database`, a `postgres://` one, a
  file that is not there: each would otherwise draw a frame missing everything
  the server placed, which is the original defect with a new cause. The knob is
  the honest answer ("the map's art alone") and it has to be *asked for*.
- **Read-only** (`SQLITE_OPEN_READ_ONLY`), so a mistyped path cannot create the
  empty database that would report "the server placed nothing here", and a live
  shard's own file is readable while it holds it open.
- `OPENSHARD_SCENE_CONFIG` says whose shard (default `openshard.toml` in the
  working directory) and `OPENSHARD_SCENE_SHARD_DB` names a database directly. A
  relative `database` resolves against the **config's** directory, not the
  process's: a shard is run from where its config sits, and resolving against
  the tool's own working directory would make the answer depend on where
  somebody typed `cargo run`.
- `OPENSHARD_SCENE_EXTRA` survives, with its job changed: it was how a live
  decoration got in, by hand, and it is now how a *hypothetical* one does — a
  torch put where no torch is, to find out what it would light.

**`tests/shard.rs` gates it, and the rows that must *not* come back are the
gate.** A reader with no `WHERE` clause at all passes a test that only checks
the cabinet arrives, so the fixture holds a contained item, a worn one, the same
graphic on the next facet, and one tile past each edge — none of which may
appear — beside the two the entry is about and one on the window's own corner,
which must. No GPU and no client files: the database is written row by row. All
three controls were witnessed by mutation, each turning the gate red: the
`loc_kind` filter dropped, the decorations' facet condition dropped, and the
east bound made exclusive.

**And the summary says which.** `Inputs::summary` counts the items a frame drew
and has no way to say where they came from, so a frame missing everything the
shard placed and a frame with nothing to place read identically there. Three
lines beside it — `scene.map`, `scene.shard`, `scene.extra` — name the three
sources this tool's list is assembled from, `scene.shard` carrying the database's
path and a count of each table. The client's dump has no such block and cannot:
its list *is* the server's, arriving on the wire as it is placed.

*Witnessed, and by the thing that started it.* `_AT=1504,1655,27 _RADIUS=4` at
Britain, the player's own cutaway (`max_z: 47`, `no_draw_roofs: true`), run
twice either side of the knob:

```
scene.map   = 114 statics pulled from the map          both runs
scene.shard = 0 ground items and 6 decorations         vs. off
                → 120 items, 1 flames                  vs. 114 items, 0 flames
```

The six are the two bookcases the person asked about (`0x0A97`/`0x0A98` at
`(1505, 1656)` and `(1506, 1656)`), two more at `(1501, 1656)` and
`(1502, 1656)`, a door, and the street lamp at `(1507, 1658)` — and the lamp is
where the flame came from. **The frame the tool drew with the reader off has no
light in it at all**, which is the entry above in one number: not a dimmer
picture, a different world.

*Not done, and it is a backlog item below:* `tile_probe`, `onsite.rs` and
`geometry_census.rs` still read no database. The reader is a module three lines
of code away from each of them, and each is its own decision about what its
answer is *about*.

### P3 — the gate ✅ 2026-08-10

`tests/parity.rs`: one real place, assembled twice — the map's own statics
through `statics::collect` (the client's route) and the same statics pulled
into `GroundItem`s and drawn through `items::collect` (the tool's route) — and
every one of `View::ALL`'s G-buffer planes compared, pixel for pixel. The list is
read off `View::ALL` rather than counted here, because it grows: it was thirteen
planes when this was written and fifteen by the time the gate first ran green,
`normal` having been split into `normal-geometry` and `normal-sprites` by work
landing beside it (`5e52279`).
`dump::plane_bytes` is the prerequisite's other half: `dump::planes` with the
PNG step split off, so a comparison zips raw bytes instead of decoding files.

**The anchor is real, not translated.** `isolated_scene`'s ordinary anchor
(`SYN_ANCHOR_NEAR_THE_ORIGIN`) moves a place next to the synthetic map's
origin, which is irrelevant to a byte comparison and worse than irrelevant: a
G-buffer's position and place planes carry absolute world coordinates, so two
frames anchored at different numbers differ everywhere before a single input
is allowed to. The gate's own synthetic map is `Map::from_blocks` filled from
the real map's own land, one for one, wide enough to hold every place it
looks at — D4's own knob (`OPENSHARD_SCENE_ANCHOR_REAL`) with the cost it
asked to have measured: 32ms in a debug build for a block covering all three
places below. D4's backlog item is closed for the size this gate needs; the
live tool's own default is untouched.

**Two real divergences turned up before it was green, and both were the
gate's inputs rather than the assembly it was gating:**

- **The occlusion grid is wider than what is drawn.** `light::collect` builds
  its grid over `light::lit_tiles` — `Camera::visible_tiles` grown by the
  widest flame's own reach — and the first version of this gate pulled the
  tool's statics over the narrower `visible_tiles` alone. A wall standing in
  that margin (off screen, still occluding) was missing from the tool's grid
  and present in the client's, and every plane downstream of the grid's shape
  disagreed by about 1.2% at all three places: `place`/`kind`/`height`/
  `normal`/`solid` from a fragment meeting a differently-shaped neighbour,
  `occluders` and `sky` from the grid itself. Pulling over `lit_tiles` instead
  closed it everywhere except the second finding below.
- **An atlas grown for what is drawn is not always an atlas grown for what
  occludes.** `occlusion::shape_of` reads a graphic's facing off the same
  static atlas the sprite pass uses, and falls back to the whole tile when the
  atlas does not hold it. The tool's item list already spanned `lit_tiles`
  (the first fix), but its atlas was still built from the *narrow* bound
  (`statics::visible_graphics`, `Camera::visible_tiles`) — the same bound the
  live client's own atlas grows from (`App::wanted_now`). A margin-band wall
  therefore had a real shape on the side whose atlas happened to hold its
  graphic for some other reason and the whole-tile fallback on the other, for
  the same occluder: forty-six percent of `solid` at the stair corner
  (`1497,1626,10`) before this was found, nothing at the corner this file
  already dumps (`1501,1659`), where no such graphic exists only in the
  margin. Both routes now grow their atlas over `lit_tiles` too, which is
  correct for what this gate is asking and is not a claim about whether the
  *live* client ever meets the same gap — see the backlog entry below.

*Done:* green at `(1501,1659)`, `(1497,1626,10)` and `(1504,1655,27)` —
5,507, 6,744 and 6,127 items respectively, every plane byte-identical at each. `the_gate_is_red_when_the_tool_forgets_the_maps_statics` is the
positive control: the map's statics dropped from the tool's route on purpose,
and every plane downstream of a drawn fragment goes red. `cargo test -p
openshard-client-render` is green apart from one failure in `tests/frame.rs`
already present in the tree from work landing concurrently with this
session's own (`843fec1`, unrelated to `dump.rs`/`frame.rs`'s assembly).

### The window's own parity — found 2026-08-10, and it is why no tool ever drew the client's artefact

**A vertical green line runs down every wall in the client's `View::Normal`, one
per tile, and not one tool has ever drawn a single one of them.** The plan's
opening entry says the artefact "reproduced in the tool the whole time"; that was
about a different artefact, and this one is the opposite case — the tool cannot
reproduce it at all, on any place, any route, any anchor. What decides it is the
**parity of the viewport**, which no tool and no gate has ever varied.

*The measurement, at Britain's `(1503, 1657)`, radius 20, `View::Normal`, the
same scene each time — a run of `+Y` pixels one column wide standing inside a
`+X` wall face:*

```
tool  1920x2080  4x       0        tool  480x520  1x    14
tool  1919x2077  4x    3484        tool  481x521  1x   835
client 1919x2077 4x    3285
```

One pixel of window width turns it on. The tool's own frames match the client's
in every other way the run distribution can be read: the genuine `+Y` regions
come out `29/44/264` real pixels wide at an even extent and `30/45/265` at an
odd one, and the client's are `30/45/265` — the same geometry, drawn one pixel
wider, plus a sliver everywhere a `+Y` face was zero pixels wide to begin with.

**Every one of the 3,484 slivers stands at `x ≡ 3 (mod 4)`, on eleven columns
88 real pixels apart** — half a tile, which is one wall static — and the
client's 3,285 stand on *the same eleven columns*. That residue is the whole
mechanism:

- `statics.wesl` and `mesh_face.wesl` place a vertex at
  `(screen - origin) * scale + viewport.size * 0.5`. At an **odd** extent
  `size * 0.5` is a half real pixel, so the world is centred half a pixel off
  the pixel grid; at an even one it is not.
- A fragment samples at `i + 0.5`. Even: the world coordinate behind it is
  `(i + 0.5 - 960)/4` — always `.125`, `.375`, `.625`, `.875` of a virtual
  pixel, **never a whole one**. Odd: `(i + 0.5 - 959.5)/4 = (i - 959)/4`, a
  whole virtual pixel exactly when `i ≡ 959 ≡ 3 (mod 4)`. At `1x` the same
  arithmetic makes *every* column whole, which is the 835.
- A box's own corner is at a whole virtual pixel by construction, so an odd
  viewport is the only way a ray ever passes exactly through one — and there,
  `impostor::meets`'s `far.x` and `far.y` are equal and its documented tie rule
  ("ties go to `z`, then `y`, then `x`") answers `+Y`. The line down the wall is
  each box's own **vertical corner edge**, drawn as a face. `place` says so: the
  sliver names the same tile as the pixel to its *right*, not the one to its
  left, so it is the near box's leading edge and not a crack between two.

The shader already states the principle this breaks, three lines above the tie,
for the lid: **"A face with no area is not a face."** A lid's `x` and `y` faces
are lines and are retired by `hi.z > lo.z`. A box's corner is a line too, and
nothing retires it — because it is not a *face* that is degenerate there but the
place where two faces meet, which no extent test can see.

*Why it is invisible in half the world, which is why it reads as a wall-only
defect:* the tie always answers `+Y`. In a wall whose visible face is `+Y` the
sliver is the wall's own colour and nobody can see it. Only a `+X` wall shows it
— and the client's frame has **no** `+X` slivers inside a `+Y` wall at all,
which is the asymmetry a symmetric geometry should not have.

*What this says about the gate:* P3 compares two routes at `900x700`, `tests/dump.rs`
draws at its own even size, `isolated_scene` defaults to one, and every screenshot
anybody has taken of a tool has been even. The client's window is whatever the
compositor hands it. **A gate that never varies a viewport's parity is green
about a defect that a person can see from across the room**, and it would not
say so.

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

- 🚩 **A box's vertical corner edge is drawn as a `+Y` face, and the fix is a
  decision.** The section above has the whole diagnosis; what is not decided is
  where it is repaired, and the three candidates are not equivalent:
  1. **In the tie** — `impostor::meets`, both the shader's and the Rust twin's.
     Flipping `x` and `y` in the ordering only moves the artefact into `+Y`
     walls, where it is invisible today, so it is not a fix but a change of
     which half of the world lies. What the edge honestly wants is the face the
     *neighbouring* box continues, which no per-box meet can see.
  2. **In the selection loop** — at the corner the near box and its neighbour
     exit at the same `t`, so the loop's "largest `t` is in front" is a tie too,
     and it is won by the box whose leading edge the sliver is. Had the other
     one won, the pixel would be `+X` and continuous with its neighbours. This
     is the candidate that makes the picture right *by construction*, and it is
     also the one whose blast radius is every fragment in the frame.
  3. **In the centring** — `viewport.size * 0.5` floored, so no sample ever
     lands on a whole virtual pixel at any zoom. It would take the artefact off
     the screen tomorrow and is a fudge: the tie stays wrong and waits for the
     next thing that samples a boundary exactly. `docs/style.md`'s own rule
     about repairing geometry rather than nudging a constant applies.
- 🚩 **Nothing gates a viewport whose extent is odd.** P3 runs at `900x700`,
  `tests/dump.rs` at its own even sizes, `isolated_scene` defaults to one. Every
  tool picture ever compared has been drawn on a grid the client's own window is
  under no obligation to share. The cheap half is one more case in P3 at
  `901x701`; the honest half is stating, somewhere a person will read it, that a
  frame's parity is an input like any other and belongs in `Inputs::summary`
  beside the camera.
- ✅ **The live client's own atlas may be narrower than the grid it is read
  for** — fixed 2026-08-10. Found while building P3's gate, and not something
  the gate itself needed to fix: `App::wanted_now`/`wanted_since` grew the
  client's static atlas over `camera.visible_tiles()`, but `light::collect`
  builds the occlusion grid over the wider `light::lit_tiles` — the same
  bound, grown by the widest flame's own reach — and reads *that same atlas*
  for an occluder's facing (`occlusion::shape_of`). A wall standing only in
  the margin between the two bounds — off screen, still occluding — fell back
  to the whole-tile shape there whenever no other reason had already put its
  graphic in the atlas, on the *live* client and not only in a tool. P3's own
  gate had already closed this gap for the tool by growing both routes' atlas
  over `lit_tiles`; the live client's three atlas-growing call sites
  (`wanted_now`, `wanted_since`, and the eviction-rebuild fallback in
  `draw()`) now do the same — `light::lit_tiles(camera, tuning)` in place of
  `camera.visible_tiles()`, with `tuning` threaded into `wanted_since` for it
  and `self.covered` set from the same widened rectangle at every write, so
  the band-difference walk stays in one convention throughout. `cargo test -p
  openshard-client-app` (143 tests), clippy and fmt are silent.
  **Still unverified: nobody has walked a build past a margin-band wall
  before/after this to see the picture actually change** — no test in
  `client/app` asserts what rectangle an atlas is grown over, and this fix
  closes the gap by reading rather than by a gate. That absence of coverage,
  not the wiring, is what is left to look at next.
- 🚩 **P3's positive control moves nine planes and leaves five of the lighting's
  own untouched.** Dropping every one of the map's statics from the tool's route
  at `(1501, 1659)` reddens `lit`, `place`, `kind`, `height`, `normal`,
  `normal-geometry`, `solid`, `occluders` and `sky` — and `light`, `flames`,
  `shadow`, `reach` and `sun` come back **0 of 630,000**, alongside
  `normal-sprites`, which has no geometry in it by construction. `occluders`
  differing by 1,513 pixels while `shadow` does not is the one that wants
  reading: a frame whose occluder grid demonstrably changed casts, so far as
  that plane can say, exactly the same shadows. Either those planes are
  genuinely blind to a difference this large (in which case the gate is green
  about them for a reason nobody has written down) or they are not being drawn
  from the frame under test at all — which is the black `View::Solid` of
  `tests/cost.rs` again, and the last backlog item below is the same suspicion
  from the other end. Nothing here is diagnosed; it is what the control printed
  on 2026-08-10, recorded before it is explained.
- 🚩 **The other three tools still read no shard database.** `isolated_scene` now
  does (see the section above), and `tile_probe`, `onsite.rs` and
  `geometry_census.rs` do not — so each of them still answers "there is no
  cabinet at Britain's `(1504, 1655)`" about a cabinet a player can see. The
  reader is `examples/shard/mod.rs` and reaching it is a `mod shard;`, so the
  work is not the plumbing: it is deciding, per tool, what its answer is *about*.
  A census of *the art's* geometry is honest to exclude what no art file holds
  and dishonest to be read as a census of the world; a probe of a tile is
  dishonest either way, because a person points it at a place and not at a file.
  Say which in each tool's own doc, whichever way it goes.
- 🚩 **The shard reader windows by the tool's radius; the client windows by what
  the server sent.** `_AT ± _RADIUS` is a rectangle chosen to keep a house from
  standing beside the thing under test, and it now decides which *lights* are in
  the frame too — the street lamp four tiles outside it lights the pavement in
  the client and nothing here. The same shape as the map-statics radius has
  always had, and newly consequential: geometry outside the radius is missing
  scenery, a flame outside it is a different picture everywhere. A gate at a
  place with a lamp needs a radius chosen from the *lighting's* reach, or the
  reader wants a second, wider window for the tables that carry light.
- 🚩 **A stacked ground item's graphic may not be the column's.** The reader
  takes `items.graphic` as it stands, and an `items` row also carries `amount` —
  which `crate::items`'s own doc says is deliberately not a `GroundItem` field,
  because "a pile of 500 gold is one sprite, and which sprite is the caller's
  question". Whether the *server* resolves that before the wire or the app does
  it on the way in was not established this session. If it is the app, a pile the
  tool draws is the wrong sprite and nothing says so. One reading of
  `crates/server/items` settles it; the reader either inherits that rule or
  states that it does not.
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
- 🚩 **A dump is 156 MB and the pictures are uncompressed.** Thirteen planes of a
  `1919x2077` viewport is twelve megabytes apiece, because `png.rs` writes stored
  deflate blocks on the argument that a debug dump is a file nobody keeps — which
  was true when a dump was one picture from a tool. A press of F12 is now
  thirteen, and a session's worth of presses fills a temp directory faster than
  anyone will think to empty it. `png.rs`'s own doc names the answer (the `png`
  crate as a dev-dependency) and rules it out for the library; the client is not
  the library, and a dump is exactly the caller that changes the arithmetic.
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
  <br>
  **The shard reader changes where to look for one.** A real place's flames are
  mostly the pack's: the street lamp at `(1507, 1658)` is a `decorations` row,
  and with the reader off the frame above has *no* light in it at all. So the
  three places are chosen out of the database — a lamp the shard placed is a lit
  pixel both ends can be asked about, while an `OPENSHARD_SCENE_EXTRA` torch is
  one only the tool has.
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
