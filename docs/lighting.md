# Lighting: a flame that a wall can stop

A living plan, in the shape the other plans here have: the decisions numbered so
one can be argued with alone, the steps, and a backlog of what was found on the
way and left undone. It supersedes the lighting half of
[`client.md`](client.md)'s "Backlog, found while giving the client firelight";
what is still true there is linked from the backlog at the bottom rather than
copied.

## Where the next session starts

**23.2 has landed and 23.3 is next: the table carries a solid.** The spill and
the ring exist, and the number decision 38.2 asked for — the widest reach in the
table — is **zero today, honestly rather than by omission**: `ring_radius` reads
the atlas it is handed and every `Shape` this build can produce still puts its
box on the one tile the static stands on, so there is nothing wider to find until
23.3 gives the table a fourth verdict that can. Reading it off the atlas rather
than writing `0` in `collect` is what makes that a fact this build could get
wrong rather than a promise; see `bake::ring_radius`'s own doc for the argument.

What is real regardless of that number: `Baked::spill` and `Builder::paste`
place a solid on every tile its box touches, not only the tile it is anchored
on, whether that tile is in the same block or a neighbour's — decision 38.1's
"reference, not clip", finished for whatever geometry a solid turns out to have
rather than only the cross-block case decision 38.2 was written about. Nothing
built today produces such a box (`Solid::box_of` is still one tile, as 23.1 left
it), so the two tests that hold this — `a_solid_anchored_outside_the_frame_still_occludes_through_the_ring`
and `a_wider_reach_needs_a_wider_ring` — author one directly rather than through
the map walk, which is `Baked::synthetic`, a `#[cfg(test)]` seam that skips
straight to a solid list. The second test is the one decision 38.2's own text
asked for by name: a ring hardcoded to one block passes the first case and
fails it, because the same solid authored to reach two blocks needs a ring of
two to be found.

`bake::collect_ring` is where the widening happens — `collect`'s own body,
with the radius taken as an argument instead of read off the atlas, so the
tests above can hold it fixed while they vary what the solid reaches. `collect`
is `collect_ring` called with `ring_radius(atlas)`, and nothing else changed
shape: a block in the ring is baked and pasted exactly as a block the frame
asked for is, and `Builder::index`'s own clamp is what tells the two apart —
a ring block's own tiles land outside the frame's rectangle and are dropped,
its spill is the one thing that does something. No second code path for
"a block that is only here for its spill."

The log line decision 38.2 asked for is `tracing::debug!` in `collect_ring`,
one line, every frame — cheap enough not to matter at `radius: 0`, and `render`
gained a dependency on `tracing` for it, which no crate below the client's `net`
and `app` had needed before. `docs/architecture.md`'s layering is unaffected:
`tracing` emits nothing without a subscriber, so the crate's own claim of never
touching I/O is still true.

Everything below is the session before it.

**23.1 landed the ownership.** The ownership moved and no picture did — which is
the property the step existed to buy, and it is worth saying what "no picture"
is held by: `tests/lighting.rs`'s 31 scenes, `tests/frame.rs`'s 37 including the
parity pass that walks both implementations over one scene, and the whole-grid
equality the bake rests on.

Two things it decided that the step as written had left open, and both are
argued where the code is:

- **A solid's box is what the walk crosses, not a slab.** A panel's box is its
  plane. Storing a nominal thickness would put geometry no ray is tested against
  in the field a reader takes for geometry, and the walk in this step is
  unchanged by design. The thickness a person needs in order to see a plane
  edge-on moved into the view — `solid::drawn`, and the `DRAWN_` fence with it.
- **The kind is carried, not derived.** A static of `tiledata` height zero is a
  body with a degenerate span and is flat in `z` exactly as a floor is, so
  "flat in `z` is a lid" would silently re-kind it. `Solid::edges` goes in 23.5,
  with the rules that ask it.

The numbers are under step 23.1, including a bound rather than a reading for what
the extra fetch costs, and the reason it is only a bound (the bench's own control
case is noisier than the difference) is in the backlog as work.

Everything below is the session before it.

**The instrument was made honest, and 23.1 is next with nothing in front of it.**
Three of the four items under "Found while drawing the solid" are closed, and
they were the three 23.1 will be judged by:

- **`solid::Cut`, the second datum** — decision 39.8. Both views drew what stands
  above the player's feet, so a hole in a floor and a floor below the cut were
  the same picture. F4 flips between "above your feet" and the whole grid, and it
  governs the wireframe and the solids pass together, because they are read
  against each other. "This storey" is *not* a third value and the decision says
  why.
- **A panel's drawn box is tied to the plane the shader tests** —
  `a_panel_is_drawn_on_the_plane_its_face_pixels_lie_on`, deriving the plane from
  `Face::place_at` rather than restating it, plus the lid-and-body companion.
  Until this, a panel drawn on the wrong edge would have read as a defect in the
  *map*.
- **The nominal thicknesses are named `DRAWN_`** — they are a picture, nothing in
  the walk reads them, and 23.1's thickness is a different number reached by a
  different argument. The fence is in the doc comment so the collision cannot be
  made silently.

The fourth — the solids pass rebuilding its whole vertex buffer every frame — is
left, on purpose: it is a cost, not a lie, the view is off by default, and the
fix is named rather than hunted for.

**So: step 23.1, the ownership, with the geometry held still.** Read decision
38.5 first — the migration is two steps and not one for a reason this file has
given twice for smaller changes.

Everything below is the session that drew the solid.

**Step 23.0 has landed: the solid is drawn, in the world, where it stands.** F5
in the client, or `--solids` on the offline viewer, and `--at X,Y` to open on a
place this plan names. The geometry is `render/src/solid.rs` and it knows nothing
about who paints it — decision 39.6 is the finding that split it that way, and
39.7 is the half-tile trap that was found on the way.

It is a **real pass** (`render/src/solids.rs`), not an overlay, and the reason is
worth carrying into the next session: `render` takes its pictures headless, so
what the client's UI toolkit draws can be neither captured nor timed. The picture
and the number both come out of `tests/cost.rs` now —

```sh
OPENSHARD_CLIENT=… OPENSHARD_FRAME_DUMP=/tmp/britain.ppm OPENSHARD_FRAME_SOLIDS=1 \
    cargo test --release -p openshard-client-render --test cost -- --ignored --nocapture
```

**Next is 23.1, the migration**, and it now has the instrument it was waiting
for: the picture below to compare against, and a person able to stand in a place
and see a solid rather than infer one from twelve strokes. Read the backlog under
"Found while drawing the solid" first — two of its four items (the `stands`
filter, and no test tying a solid's face to the plane the shader tests) are about
the instrument's own honesty, and 23.1 is the step that will be judged with it.

The DoD is closed, cost reading included: **3.61 ms at the widest zoom** for
3,768 boxes, against 0.34 ms for the whole lighting pass on the same frame. It
stays a debug view, and the backlog says where the time goes if that ever has to
change.

Everything below is the session before it.

**Nothing was built that session. The plan was re-cut, and what re-cut it is
decision 38: the tile grid stops being a container and becomes an index.**

The question that started it was about stairs — a tread is a shape the surface
record cannot state — and the answer that came out is larger than a tread. Three
things were settled and each one removes an argument this file had been making
for several sessions:

- **A solid is not clipped to its tile, and nothing bounds how far it reaches; a
  cell *references* it.** The anchor is the whole invariant — it names the block
  that owns the solid — and the only number in the neighbourhood is the width of
  the ring a frame pastes, which is *measured off the table* rather than decreed.
  An earlier draft of 38.2 invented a one-tile limit and it lasted a day. Every seam this
  file has fought was manufactured by cutting geometry on a tile boundary: the
  spokes of decision 18 were a ray slipping through the corner between two
  panels, and a corner is only a corner because the wall was cut there. A whole
  solid referenced from four cells answers the same ray identically from all
  four, and the class dies by construction rather than by a rule.
- **The "two primitives" half of decision 36 is withdrawn**, and it was mine. It
  argued that a wall must stay a plane because a box of zero thickness is a
  numerical coin toss — which is an argument against *zero*, not against a box.
  With authoring, a nominal thickness is a number a person states, and the
  vocabulary collapses to one solid plus the hole, which stays a subtraction.
- **"The art cannot measure it" stopped being a bound on the model.** Decision 3
  is still right about a *detector*; it says nothing about a record a person
  writes by hand. The measured and the authored differ in provenance, and
  provenance is a column of the table rather than a second shape in the shader.

**Start at step 23.0 — the solid, drawn.** Decision 39 is the finding that put it
first: the renderer is not a sprite blitter that would have to acquire a third
dimension, it is a three-dimensional scene whose primitives are billboards.
World space, a per-pixel world position, an orthographic projection written as
integer arithmetic, and a hardware depth buffer are all already there, so a box
drawn in world coordinates lands in its pixels with nothing fitted, and the pass
that draws it is an instanced quad pass of the same shape as `statics` — three
faces, always the same three, no mesh pipeline anywhere. It comes before the
migration because 23.1's whole DoD is *"the picture did not move"* and the
instrument that would judge that is currently a wireframe.

Then **23.1**, the migration: the frame's arena stops owning surfaces and starts
owning solids that cells point at, with the geometry exactly as it is today and
every scene green. Steps 23.2 onwards are where anything is allowed to look
different. **Step 22 is folded into it** — a body's footprint is a solid narrower
than its tile, and building it first would mean writing a flag into the aperture
plane in order to delete it two steps later.

The WebGL2 ceiling was questioned and **kept**, with a finding: it costs this
feature nothing, because the bake is on the CPU and a storage buffer would only
be a tidier spelling of `textureLoad`. Decision 30.5 carries the finding and the
one question it leaves a person.

Everything below this line is the session before it.

**Step 21.5 has landed, and step 21 with it: the grid is baked per block.**
`occlusion::bake` keeps one `Baked` per map block — the surfaces its statics
stand and the sky they take — and a frame is those blocks pasted, plus the three
things that are genuinely per frame: the server's ground items, the blur, and the
pack with the frame's `Cutaway`. **1.22ms to 0.37ms**, and the reading that
decides it is the *panning* one: a camera moving a tile a frame builds about
three and a half blocks and costs the same 0.36ms as a still one.

What comes out is the grid the walk builds **to the byte**, asserted on a built
town and on every batch of Britain in `tests/cost.rs`. The one thing the plan did
not name is **decision 37**: a surface is derived from the map *through the
atlas*, an atlas grows, and a block baked before a graphic was packed would hold
the whole-tile fallback for ever — so `StaticAtlas::revision` counts the three
answers a `Shape` is made of and the bake lets go when it moves.

**Start at step 22, decision 34's five changes — a body's footprint.** It is the
next thing in the grid that a real tile in Britain is visibly wrong about
(`1509,1635` is drawn and behaved as a full square), it is written up step by step
with a DoD each, and none of it waits on anything. Step 17, the shaft, is the
other open one. And the backlog has three new entries from this session, the first
of which is the one to read if a frame indoors ever looks expensive: **the paste
is now the largest thing left in the build.**

Everything below this line is the session before it.

**Step 16 has landed: a window is a hole the art was measured for.**
`facing::aperture_of` reads the rectangle a window graphic leaves out of its own
silhouette — **58 pictures of 39,189, 56 of them the client's own `WINDOW`
flag**, and **85 wall statics standing in Britain**. It is the largest rectangle
*inscribed* in the transparent region, because the client's windows are arches
and a bounding box would let light through drawn stone. A measurement is a height
above the picture's own base (`facing::Hole`); `Aperture::above` places it on the
static that is standing at a `z`, and that conversion happens once, in
`Builder::add`.

Nothing else moved, which was the step's own claim: the walk already knew what to
do with a hole (step 21.3), so what a real install gained is that four graphics
stopped being solid stone. `Shape` is now both verdicts about one picture, and
`Shape::of` is the one function the tool and a table-less client both measure
through.

**Start at step 21, decision 30's remaining four changes** — 21.1, 21.2 and 21.3
have landed, and what is left is the *bake*: 30.4's per-block, per-storey-band
list, which is where the 2.0ms of a 3.3ms CPU budget is. Step 17, the shaft, is
the other open one and it now has real windows to be a shaft through. And the
backlog's newest entry is the one to read first if windows look wrong in a house:
**the leaded window is refused**, 46 pictures with a lattice drawn across them,
four of which stand in Britain.

Everything below this line is the session before it.

**Step 20b: the measurement has left the frame.**
`openshard-client-artscan` reads an install, offers every picture in it to
`facing::facing_of`, and writes one table beside the client; the atlas looks a
graphic up in that table instead of walking its pixels on the frame it is first
seen. Measured on a 2D install: **39,189 pictures, 6,150 read, four seconds** in
an ordinary `cargo run`. The client with no table behaves exactly as it did
before and says so in a log line — decision 31.6 — and the two lines it can print
are the two states a person can do something about:

```
art table: 6150 of 39189 pictures read, 0 written by hand
art table: measuring as we pack — …/openshard-art.table: No such file or directory
```

**Start at step 16**, and start it in the tool. The budget for a measurement is
now a minute and today's measurement spends four seconds of it, which is what
decision 31 was for: an aperture is a hole to be *found*, and a runtime pass
would have had to be a scanline trick. The seams are `facing.rs` (a pure function
of an `Image`, which is where an `aperture_of` goes beside `facing_of`),
`arttable.rs` (a row grammar with room for a second verdict, and `FORMAT` to bump
when it grows one) and `StaticAtlas::state_aperture`, still one method with one
caller.

Everything below this line is the session before it.

**Step 21.3 landed before it: a surface can have a hole in it.** A panel carries a
rectangle in its own coordinates — a span along the run and a span of `z` — and
the crossing test asks whether the ray went through it. A wall with a hole in
one tile throws a **fan** onto the ground behind it: narrow at the wall, wider
further out, with the tiles either side of it at the ambient exactly. That is
the picture step 16 was always for, arriving before step 16, because the
mechanism and the measurement are independent and this half needs no art.

Nothing in a real frame has moved and that is deliberate: no graphic in any
install has an aperture yet, so `Occlusion::any_aperture` is false, the hole
plane is not laid out or uploaded, and the `HOLED` bit is never set — which is
what makes the walk's extra fetch cost nothing until there is something to
fetch. **Decision 30.8** is that storage choice and the argument for it.

~~**Start at step 20b, then step 16**~~ — 20b is the session above. The two turn
"a hole a scene states" into "a hole the art has": 20b is decision 31, the
silhouette work leaving the frame for a tool and a table, which is what lets 16's
measurement be as expensive as it needs to be, and 16 is the measurement.
`StaticAtlas::state_aperture` is the seam they arrive through, and it is one
method with one caller today.

Everything below this line is the session before it.

**Step 21.2 landed before it: the union is gone.** A tile's occluders are surfaces
standing beside each other rather than one merged span — a wall from `z 0` to
`z 10` and another from `z 30` to `z 40` no longer close the thirty `z` of air
between them, and a lid over a wall tile keeps its own span, its own opacity and
its own rule instead of contributing its `z` to the wall and losing its lid-ness.
`push_surfaces` is gone with it; the shape a static *is* is decided in
`Builder::add`, which is the only place that ever knew.

**And decision 30.6 has its distribution**, off Britain at the widest zoom:
**10,212 standing cells hold 18,071 surfaces**, against 10,653 under the union.
58.2% of them hold one, 26.5% two, 7.4% three, and the tail runs to 21 on ten
tiles — a shop with a stack of floors, walls and a roof on one square. Nothing
was dropped: the only cap is the format's own byte and the worst tile in a city
is an eighth of it. That is the number the truncation question wanted and it
answers it by not arising.

The three sessions below are what that rests on, and the short version is that
the lighting stopped answering with a *tile* anywhere it was asked about a
*surface*. That is what makes the representation change a change of storage
rather than a reopening of the rules — decision 30.7.

**Read decisions 26, 27 and 28 — they are three answers to one objection**, and
the objection is worth stating in the words it arrived in: *deciding who is lit
should be by polygon, not by tile.* Every one of the three was a rule that
answered with a **tile** where the question was about a **surface**.

- 26: a flame in a wall's row or column was "part of" that wall — a whole street
  long. Now a mounted flame is *placed* outside the plane its tile names, and the
  facing test has no exception in it.
- 27: a flat surface had no normal at all, so a wall's top cap took the whole of
  any pool that reached it. Now it looks up.
- 28: the exemption from self-shadowing was asked of the tile. Now it is asked of
  the surface — a face and an upright exempt their own cell, a floor pixel does
  not.

**Decisions 29 and 30 are written and not built, and 30 is the one to read.** The
world's occluders become a *baked* list of surfaces — quads with `z` spans and
holes — indexed by the tile grid, derived from the art and overridable by hand.
It is worth 2.0ms of a 3.3ms CPU budget on its own, it is what a real window with
a real shaft needs, and its seven micro-decisions are settled in the plan so that
nothing there has to be re-argued. **Two things come before it**, in this order: decision 31 moves the silhouette
measurement out of the frame into a tool and a table — which is what lets the next
measurement be as expensive as it needs to be — and step 16 is that measurement,
the aperture, a pure function with a test against the client. No storage can hold
a window before something has read one off the art.

**Read decision 26 first, then 25.** They are one session and the second half is
the one a player pointed at: a lamp post standing in the street lit the far side
of a house's east wall, because the facing test excused any flame in a wall's own
row or column and a column is a whole street long. That exemption existed to
protect a *mounted* lamp, which sits at its tile's centre behind the plane of the
face it is bolted to — so the answer was to move the flame outside that plane
rather than to excuse it, which also closed the oldest entry in this file's
backlog: a sconce no longer lights the room behind its own wall.

The instrument grew the line that report needed. `tests/onsite.rs` now prints a
tile's four faces with **`through` and `facing` apart** — a face behind the flame
reads `through 1.000, facing 1.000` and a shadowed one reads `through 0.000`, and
those are two different defects that no picture of the ground can tell apart.

**Read decision 25 and the backlog's "found while giving a corner its two
faces"**. The missing fact the session before named — a corner is two faces and
the art would not name either — is measured now: `facing::facing_of` answers
`Facing::Corner`, the stance carries the four of them, `statics.wgsl` resolves
one per fragment by which half of the picture a pixel is on, and the grid gets
two panels where it used to get a whole tile. Two of the three open entries at
that corner are closed by it. **The third is not**, and it is the one to pick up:
the floor under a wall tile is lit from outside the house, because the exemption
that keeps a wall's own tile from shadowing it is asked of the *tile* where it
should be asked of the *pixel* — the attachment already carries what is needed to
tell a floor pixel from a face.

What the change is worth, measured rather than argued: the detector reads
**91.9% of the wall statics standing in Britain** where it read 75.7%, and 45.5%
of the install's wall art where it read 36.3%. The 744 corner statics in that
difference are what a city has wherever a run of wall turns.

Everything below this line is the session before it.

**Read decision 24 and the backlog's "found at a house corner in Britain"
first**, and then decision 18, which decision 24 finishes. Four things were
reported from one picture of a lamp against a house corner; one is fixed and the
other three are one missing fact — a corner is two faces, `facing` refuses to
name either, and everything that falls back from that refusal falls back twice:
to a whole-tile occluder in the grid and to `Stance::Upright` in the attachment.

The method that session used is worth copying, because the leak was a *stripe*
thinner than a tile and nothing that samples one point per tile could have seen
it:

- **The instrument came before the reproduction.**
  `crates/client/render/tests/onsite.rs` takes a coordinate and prints what
  stands there, what the atlas read off each picture, what the grid ended up
  with, and a sweep of rays at a third of a tile. It is the thing that turns a
  report into the handful of facts a built scene has to reproduce, and it is
  where to start with the next one.
- **The built scene was then checked against the number.** `scene::house_corner`
  is Britain at `(1441, 1692)` with the graphics replaced by three synthetic
  ones, and it leaked **0.845** of the flame where the map leaked 0.847. A
  reproduction that agrees to three digits is a reproduction; one that merely
  looks similar is a second scene.

Everything below this line is the session before it.

**Read decision 18 and step 19 first.** The pass was reported from the client as
having gone badly wrong — a lamp with no falloff, thin spokes fanning out of
every wall, light coming through the seams between tiles — and three of those
four complaints were one defect and are fixed. What that session did, in order,
is worth knowing because the order is the method:

- It built the instrument first (step 19, `render/src/plan.rs`): the real blit
  over a synthetic flat ground, one tile to a square of pixels, with the
  occluders and the flames' own rims stroked on top. A plan view answers "is
  this a circle" in one glance where the isometric frame cannot.
- The first picture of the dumbest scene there is — one torch, one straight
  wall — showed the spokes immediately, and named them: a panel scaled by the
  *length* of a crossing lets a ray through the corner between two panels.
  Decision 18.
- The second picture was Britain, and it showed the other half: 64 flames in the
  frame and not one pool on any street, because the flames were *windows*, each
  standing inside the very wall that then cut it up. Decision 19.

Steps 1–15 and 18–19 are done; **16 and 17 are not** — the *steps*; decisions
17 and 18 are decisions and landed. Step 15's measurement is half of what step 16
needs: a wall's face is read out of its own art, and the window's hole is the
same silhouette measured again — a span of `v` along the face and a span of `z` —
so whoever picks 16 up starts from `render/src/facing.rs` rather than from
nothing.

What step 15 changed, and what it is worth knowing about it: a wall's pixels no
longer all claim the middle of their tile, so a row of walls is one lit surface
instead of a row of flat 44-pixel bands with a cliff at each seam. It reads
**76% of the walls standing in Britain** and leaves the rest exactly as they
were. It also produced decision 16, which is the kind of thing that only turns up
when the picture is looked at: a fraction of exactly one names the wrong tile and
made every faced wall shadow itself.

Step 6's measurement has been taken and it moved two things: the sun is no longer
gated on what it costs (it is 9% of the pass), and decision 6's claim that this
arrangement is cheaper per pixel than the screen-space circles it replaced **did
not survive** — the loop is entered for every light on every fragment, and that
is where nearly all of the pass's GPU time goes. The numbers are under step 6 and
what they raise is in the backlog under "found while measuring it". The headline
is that the GPU half was never the expensive one: a frame's lighting costs 0.22ms
on the GPU and 2.8ms on the CPU.

Step 18 is out of that order on purpose — a light in the player's hand needed
nothing measured out of the art, because a *mobile's* facing is on the wire while
a *wall's* is not. What it leaves for whoever picks this up is in the backlog
under "found while putting a light in the player's hand", and the first line of
it is the one worth reading: the beam is the local player's alone.

Nothing outside this file is half-finished — `main` builds, the three commands
are silent, and the pictures are `tests/cost.rs`'s frame dump, headless, with
`OPENSHARD_FRAME_VIEW=5` for the ones the art had to be thrown away from.

## Where it stands

Done, and the decisions below are what was built rather than what was proposed.
`client/render/src/light.rs` collects the flames a frame can see *and* the
occluders in their way; the three world passes write which tile each pixel came
from; `blit.wgsl` lights the frame in world coordinates and walks the grid
between a fragment and each flame. F10 still toggles night. A torch inside a
house no longer lights the street, and the wall's own face is the brightest
thing next to it.

What it replaced, because the argument is the point:

Every light used to be a **circle in the pixels of the drawn image** — the
flame's tile projected to a screen point, compared against the fragment's screen
position. That arrangement cannot be given walls, for two reasons:

- **The screen folds height into `y`.** A brazier in a cellar and a lantern on
  the street above it are a few pixels apart in the image, so the pool of one
  covers the other. This was `client.md`'s "a flame lights through a floor".
- **A wall's sprite stands above the tile it occludes from.** A wall is 44
  pixels of picture rising from a diamond that is at the floor. Whatever
  screen-space mask darkens the ground behind the wall also covers the wall's
  own face — including the face *turned towards the flame*, which is the one
  surface that must obviously be lit. There is no shadow polygon that fixes
  this, because in the image the lit face and the shadowed ground are the same
  pixels of the same sprite; only a per-pixel answer to "which tile is this?"
  separates them.

So the pass moved from the screen into the world, and the shadow came with it.

## Decisions

**1. Lighting is computed in world coordinates, not in screen pixels.**
A fragment is lit according to the tile and height of *the thing drawn there*,
not according to where that thing landed in the image. This is what makes a
wall's face lit as the wall's own tile is lit, and it is what lets a storey
below stay dark while the street is not.

**2. The world passes write a second attachment: `(x, y, z)` per pixel.**
`Rgba16Uint`, as `(x, y, z + 128, kind)` — the tile the pixel belongs to, the
height it was drawn at, and what kind of thing wrote it. Ground, statics and
mobiles all know these numbers per instance already; none of them has to compute
anything new. A fragment a sprite discarded writes nothing, so the channel says
what is *visible*, which is exactly the question lighting asks. `kind == 0` is
"no world here" — the cleared background — and takes ambient and no flame.

Why an integer format and not a float one: these are tile indices and a `z`, and
a `u16` holds a coordinate on the largest facet a client ships (7,168) exactly.
`Rgba16Uint` is colour-renderable in WebGL2, which is the ceiling this crate
draws under.

**3. An occluder is a whole tile, not a wall's edge.** ~~Superseded by decision
17~~ — what follows is still why it was right at the time, and its last paragraph
is exactly what changed.

**3 (as written).** An occluder is a whole tile, not a wall's edge.
`client.md` proposed projecting each wall static to the segment its base covers.
The map cannot say which segment that is: **nothing in `tiledata.mul` records
which edge of its tile a wall stands on** — that is only in the shape of the
sprite. Guessing it from the art's silhouette is a subsystem, and a wrong guess
opens a corner of a room to the street.

The tile is the honest unit, and it is better than the segment in one way that
matters: a room's wall tiles form a *closed* ring by construction, so no light
leaks out of a corner. It is worse in one way that does not: a pool stops up to
half a tile early. The tile a light stands on never occludes it — a sconce is a
static on the wall's own tile, and a light that shadowed itself would be dark.

**4. What stops light is what stops an arrow: `WINDOW | NO_SHOOT`.**
Not `BLOCK`. The two are different questions and the reference answers them
separately: ServUO's `Map.LineOfSight` (`Server/Map.cs:3040`) tests statics with

```cs
if (t.Z <= pointTop && t.Z + height >= point.Z && (flags & (TileFlag.Window | TileFlag.NoShoot)) != 0)
```

— impassability never enters it. That is the right rule and it is better than
anything invented here: a barrel and a fence are `BLOCK` and you can see over
both, a wall is `NO_SHOOT` and you cannot see through it, and a shard's custom
wall gets it right for free. Reading `BLOCK` instead would put a shadow behind
every crate.

The grid carries an *opacity* byte rather than a flag, and it now carries three
answers rather than two: `NO_SHOOT` stops everything, `WINDOW` stops a fifth
(`occlusion::PANE`), and everything else stops nothing. That is where this parts
company with the reference on purpose — line of sight is a yes or a no, so a
window is a wall in it, and light is a fraction. A window that stopped light
makes a lit room read as a bunker and hides the one thing a candle is for after
dark. The fifth is a guess; there is no number for it in any client file.

**A static the cutaway has taken away occludes nothing.** The same
`cutaway::shows` test the lights already run: a shadow cast by a wall that was
not drawn is a dark band with nothing making it, which is `client.md`'s second
unsettled question and this is the answer to it.

**5. An occluder carries the span of heights it occupies.**
`z` from the static and `z + height` from its tiledata entry — `height` being
ServUO's `CalcHeight` (`Server/TileData.cs:112`), which halves a climbable
(`Bridge`) tile the way `movement`'s `platform_surface` already does here. A ray is stopped
by a tile only where it passes *through* that span, so an upper storey's wall
does not shadow the ground floor and a cellar's wall does not shadow the street.
Where a tile holds more than one opaque static, the span is their union — after
the cutaway has already removed the storeys the player is not on, that is one
wall in nearly every real case, and the union is conservative in the direction
that darkens rather than leaks.

**6. The shadow test is a walk along the ray through a tile grid, in the shader.**
One `Rgba8Uint` texture per frame covering the tiles a flame could reach —
`(z_bottom, z_top, opacity, present)` per tile — and a fragment multiplies the
opacities of the cells between it and each flame. Not a mask rendered per light:
one texture is uploaded once for the frame instead of sixty-four, the shadow
edge is exact rather than the resolution of a mask, and the cost is paid only by
fragments that are inside a pool at all.

This was also claimed to be *cheaper* than what it replaced — every fragment of
the screen ran the old loop over all 64 lights, and here a fragment outside every
radius was said to leave the loop immediately. **Step 6 measured it and the claim
is wrong.** A fragment outside a light's radius `continue`s to the *next* light;
there is no way out of the loop, so every fragment still runs 64 iterations
whatever is on screen, and those misses are 63% of what the whole pass costs. The
saving is real but it is a smaller one: what a miss skips is the ray walk, not
the iteration. See step 6 for the numbers and the backlog for the shape of the
fix.

**7. Distance is three-dimensional, with `z` in tiles.**
`Z_PER_TILE = TILE_WIDTH / Z_STEP = 11`: eleven `z` units is one tile's width,
which is the ratio the projection itself uses. A flame reaches as far up and
down as it reaches sideways, which is what stops a cellar from lighting the
street even where nothing occludes.

**8. The debug views are branches of the blit, not a second pipeline.**
Everything an observer of this pass could want to see is already bound to it:
which tile a pixel claims and what drew it (the place attachment), what stands
on that tile (the occlusion grid), how far every flame is and how much of it
survived the walk (the loop itself). A separate visualiser would be a second copy
of that unpacking, kept in step with this one by hand, and it would answer about
*its* copy of the frame rather than about the frame on the screen. So the mode is
one number in the lighting uniform and a `switch` at the end of `fs_main`, and
what it shows is the very values the lit picture was made of.

**9. The reasons are computed on the CPU, and the shader is checked against
them.** "Why is this tile lit" is a list — this flame, that far, inside its
radius, and the ray died on that cell — and a picture cannot be a list.
`light::sample` is that list, in Rust, from the same `Lighting` the GPU is given.
Which makes one formula exist twice, and the failure mode of that is specific and
nasty: the debugger diverges from the renderer and then lies exactly when it is
believed. So the shader is not the canon and the CPU is not a sketch — a GPU test
uploads a synthetic place attachment, runs the real blit over it and asserts the
two agree per pixel. The parity test is the reason the second implementation is
allowed to exist at all.

**10. The scenes are built, not loaded.** A room with a torch in it is a `Map`
of flat ground, a `TileData` where two graphics have flags, and a list of items —
every one of which this workspace can construct from nothing. That is not a
concession to the no-client-files rule, it is better than a real house: the wall
is at a stated tile with a stated height, so a test can say *which* cell should
have stopped the ray, and a failure prints the room rather than a coordinate.
`render/src/scene.rs` holds them, and they are ordinary `pub` items rather than
`#[cfg(test)]` ones because the GPU tests, the playground and a future benchmark
are all outside the crate.

**11. An open door is not a special case — it is a static that stopped being an
occluder.** ~~The client is *told* a door opened: the item's graphic changes, and
the open leaf's graphic is not `NO_SHOOT`.~~ **The second half of that is false,
and it was checked against the client only after somebody looked at a lit
doorway.** `tiledata.mul` does not distinguish an open door from a shut one at
all. Measured over ServUO's own thirteen door families
(`Scripts/Items/Functional/Doors.cs`, where every `BaseDoor` is
`base(closed + 2 * facing, closed + 1 + 2 * facing, …)`, so within a family the
even offsets are shut and the odd ones are open), the flags of the two are
**identical in every one of the 104 pairs**: the wooden and metal doors are
`NO_SHOOT` open and shut alike, the gates are clear open and shut alike, the
barred doors are `WINDOW` both ways. 55 of 104 open leaves stop everything.

So today an open door lays a **whole tile of wall across its own doorway** — a
band of shadow with nothing visible casting it, which is exactly what it looks
like on screen, and the more visible for the leaf beside it being brightly lit.

The intent of this decision stands and the mechanism does not: an open door must
occlude nothing, because decision 3's occluder is a whole tile and a tile-wide
wall in an opening is far more wrong than no occluder at all. What is missing is
the *fact*, and it is not in the client:

- not in `tiledata.mul`, per the measurement above;
- not in how the graphics are laid out — the `DOOR` flag comes in runs of 1, 2,
  4, 6, 7, 8, 11, 12, 13, 16, 20, 24, 29, 32, 80 and 98, so there is no parity to
  read an odd offset off;
- not in the art — "an open leaf's picture is wider than a tile" holds for 46 of
  the 104 pairs and no better, because a door swung to four of the eight facings
  is still 44 across.

Which leaves the door table itself, and it is ServUO's. `render/src/doors.rs`
and `data/doors.json`: the thirteen family bases, sixteen graphics each, even
shut and odd open — and `occlusion::opacity` asks it **before** it looks at a
flag, so an open leaf stops nothing and takes none of the tile's sky.

Two things follow from where the question is asked. `opacity` takes the
**graphic** now and not only the tiledata entry, which is the general shape
rather than a door-shaped patch: a flag is a fact about a *picture*, and
anything that opens, lifts or breaks — a shutter, a portcullis, a drawbridge —
is a fact about the *thing*. And **a graphic the table does not know keeps
today's behaviour exactly**, so a shard's own door goes on occluding rather than
a wrong guess opening a room to the street; the same refusal decision 15's
detector makes.

Held to by three tests, two of which need a real install. The client is the
oracle a ported table needs: every one of the 208 graphics the table claims is
flagged `DOOR` in `tiledata.mul` bar four, which is the client's own gap and not
a mistyped base. The second asserts the module's *premise* — that the stopping
flags of an open leaf and its shut twin are identical, 103 of the 104 pairs, the
one exception (`0x0683`/`0x0684`) named rather than tolerated by a percentage.
The day that stops being true, the right move is to delete the table and read the
flags, and the test says so. The third is in the grid: the same `StaticTile`
twice, two graphics, and only the open one leaves no cell.

`server/world/src/doorgen.rs` ports the same `+ 2 * facing` rule for the doors a
shard generates. Two copies, because `client/*` and `server/*` never depend on
each other and a table of art indices is not something both ends of the wire
agree on. If a third reader appears, the table moves down; the boundary does
not.

**12. The sun is a direction; a sunbeam on the floor is the same walk without an
endpoint.** A flame is a point and the walk between a fragment and it is bounded
by the radius. Sunlight has no position: every fragment walks the *same*
direction — an azimuth and an elevation, in tile units — until the ray leaves the
grid or is stopped, and what it produces is a wall's shadow lying across the
street and a bright patch on the floor behind a window. That patch is the honest
form of "sun through a window" in a tile world, and it is where this starts: a
beam in the air with no lit floor under it looks like a decal.

The beam itself — the visible shaft between the window and the floor — is not
geometry at all here, because nothing in this renderer draws the air. It is a
screen-space pass: the sunlit fragments are a mask, blurred along the sun's
direction *on the screen* and added. That is a second pass and a separate step,
and it only makes sense once the patch it grows out of is right.

Two things sunlight needs that firelight did not, and both are why it is not
simply a 65th light:

- **A window has to pass some light.** `occlusion::opacity` is binary today and
  `WINDOW` is opaque, which is right for line of sight and wrong for a pane at
  noon. The byte and the shader's multiply are already there; what is missing is
  the rule, and the rule wants a scene to be tuned against.
- **The ray is long.** A flame's walk is bounded by nine tiles; the sun's is
  bounded by how far a wall can throw a shadow, which at a low elevation is the
  width of the grid. That is a real per-fragment cost on every ground pixel of a
  daylit frame, where firelight's cost was paid only inside a pool — so the walk
  needs a cheaper bound (stop as soon as the ray is above the tallest occluder
  the grid holds) before it is on by default.

**13. A sprite says which way its picture faces, and that is where its pixels
are.** The attachment carries where in its tile a pixel is, and a sprite used to
write the middle of the tile for every one of its pixels — which is right for
nothing and wrong for two different reasons. A *floor* static is a picture of the
tile's diamond, so its pixels are spread across the tile and its height is the
tile's; a room's floor written as one place came out as flat 44-pixel diamonds
with a step at every seam, which is most of what a pool of light was accused of
looking like. A *wall* is a billboard: what runs down its picture is height.

**Across a wall, this decision first said `1/44` of a tile along the screen's
`x - y` axis, and that was wrong.** It survived one commit. That axis is the
horizontal, and no wall runs along it: a wall runs along one *world* axis, which
in this projection is a screen diagonal. Spreading a wall's pixels sideways puts
them along the one direction the wall does not go, and it looks like it. What
replaced it was the tile's middle everywhere — an honest statement of what was
not known — until step 15 measured the axis out of the art. The `1/44` is not
half-right; it is the wrong direction, and it is written down here because the
plausible-looking version is the one somebody re-derives.

Which stance a static has comes from the client first: `TileFlags::FLOOR` —
`UFLAG1_FLOOR` in Sphere, `Background` in ClassicUO — is set on floors, rugs and
roads and on nothing that stands up. Not `PLATFORM`: a table is `BLOCK |
PLATFORM` and is a picture of a table, not of the ground. Then the art, for which
edge a wall stands on. `place::Stance` is the six of them — flat, four faces, and
"standing but unknown" — and it rides in three bits above the kind in the
instance's place word, never in the attachment, whose fourth channel is two bits
of kind and fourteen of fraction with nothing spare.

**14. The shadow ray walks the cells it crosses, and spends the length of each
crossing.** ~~Half superseded by decision 18~~ — the length is what a *body*
spends and the walk still spends it there; what a **panel** does is decided where
the ray pierces it, and the paragraph below about how wide the gradient is now
describes only the vertical half of it. The rest stands, and the first paragraph
is why the walk visits cells at all.

**14 (as written).** Not a fixed number of samples along the segment: at two tiles apart
that was one interior point, so whether a fragment was in shadow was decided at
the resolution of a tile and every shadow in the frame had a tile's straight
side. A grid traversal visits exactly the cells the ray passes through, and it
knows how long each crossing is and what share of it falls inside the span the
tile occupies.

Having the length is what makes the edge a gradient: a ray clipping a wall tile's
corner keeps most of its light, and one grazing the top of a wall is dimmed
rather than switched. How wide that gradient is is not one number — a flame is a
body, not a point, so an occluder against the thing it shadows draws a sharp edge
and a distant one draws a penumbra. Its width is `FLAME_SPREAD * t / (1 - t)`,
`t` being how far along the ray the occluder is from the lit end: the ordinary
similar-triangles answer, for one division rather than a second ray. It is capped
below a tile, because a wall crossed squarely must stop *all* of the light or
rooms leak — which is the same conservative direction decision 5's union takes.

**15. A flame in a hand is a cone, and the hand is not a shutter.** Everything
on the map lights every direction, and a light carried by a character must not:
an omnidirectional pool centred on a body lights the wall behind it exactly as
brightly as the one it is walking towards, and the eye reads that as the
character *glowing* rather than as the character *carrying* something. So a
`Light` may have a `Beam` — an axis and the cosine of a half-angle — and the pool
is multiplied by how far inside that cone the lit spot is.

A cone and not a second radius, and the ordering is what makes it cheap: a
fragment outside the radius never asks about the angle, and the whole of a beam
is one dot product against a direction the CPU normalised once. `(0, 0, 0, -1)`
is "lights every way" — no cosine is below `-1`, so a fire standing in the world
pays a comparison and never the arithmetic.

Two numbers keep it from reading as a stencil rather than as light. The rim
softens over `BEAM_EDGE` of the way in from it, because a hard edge is found by
the eye instantly — the same complaint the tile-edged shadows drew. And
`BEAM_SPILL` of the flame escapes it in every other direction, because a hand is
not a shutter: the arm holds the torch out in front and the body is behind it, and
neither of those stops a flame from being a flame. Without the spill the one
thing the player is looking at — their own body, whose pixels are on the flame's
own tile and directly above and below it — is the only black shape in the frame.
A quarter, so that what is in front is four times what is beside and the
direction is legible at a glance.

Where a beam is aimed is the *facing*, which is the one direction this pass can
have for nothing: it is on the wire for every mobile, and the client already
holds it to pick which way a sprite is drawn. That is the whole of why this
arrives before the wall facings of step 15 — the light knows which way it is
pointed even though a wall does not.

**16. A fraction of exactly one names the next tile, and the walk believes it.**
A wall's face lies *on* the tile boundary, so the honest fraction for a south
face is `y = 1` — and `blit.wgsl` finds a fragment's cell with
`floor(tile + fraction)`, which for that number is the tile beyond the wall. The
walk exempts the fragment's own cell from shadowing it, precisely so a wall's
face is the brightest thing beside a torch; hand it the neighbour and the wall's
own tile stops being exempt, so **every faced wall is shadowed by the wall it is
the face of** and comes out at ambient. Measured on Britain the first time this
was drawn: a run of lit wall at 249 dropping to the 65 of an unlit night.

So what the attachment carries is the fraction held one step of its own
seven-bit grid inside the tile — a hundred-and-twenty-seventh, 0.35 pixels of
world. `statics.wgsl`'s `INSIDE`. The geometry is still the boundary and
`facing::Face::place_at` still says so; it is the *encoding* that has to name the
tile the wall belongs to, and the two are different questions. The same clamp
covers a floor's outermost pixel, which had the same latent bug from step 12 and
never showed it, because a floor's tile is not an occluder.

**17. An occluder is a panel on a named edge, now that the art will name one.**
Decision 3 made an occluder the whole tile and gave the reason: nothing in
`tiledata.mul` says which edge a wall stands on, and reading it off the
silhouette "is a subsystem", where a wrong guess opens a corner of a room to the
street. **Step 15 built that subsystem**, and it refuses rather than guesses — so
the reason has expired and the cost has not.

What the whole tile costs is light that travels *alongside* a wall. A lamp
mounted on a house is shadowed by the next tile of its own wall, so the street it
hangs over comes out with a band of darkness that nothing visible is casting.
That is how this was found: a player pointed at Britain 1439,1692 and
`light::sample` answered `stopped at (1440, 1692)` — a tile that does hold a
wall, and whose wall the ray never goes through.

So a cell carries **which sides of its tile are occupied**, four bits, and a ray
is stopped only where it *crosses* one of them. Three things make that cheap:

- The walk already knows. `boundary.x < boundary.y` is which boundary is being
  crossed and `toward` is the direction, so the side a ray leaves by is two
  comparisons, and the side it enters the next cell by is the opposite of it.
- The cell already has room. `Rgba8Uint` is `(z_bottom, z_top, opacity, present)`
  and `present` was a byte holding a bare yes; it is `PRESENT | mask` now.
- The face is already measured: `Sprite::face`, once, when the picture is packed.

~~**And the flame's own tile stops being exempt.**~~ **Reverted, and the picture
is what reverted it.** The argument was sound and the frame it produced was not:
the flame sits at its tile's centre, which is inside the panel, so a ray leaving
it does cross the wall — and since every lamp in a city is mounted on a building,
the whole of Britain came out with its walls lit from the inside and **not one
pool of light on any street**. The starburst that motivated the change was mostly
decision 18's spokes and went with them; what is left is a lamp that lights both
sides of the wall it hangs on, which is the defect this file has carried since
its first backlog and is much the smaller of the two.

So decision 3's rule stands: **neither end of a ray is shadowed by the tile it is
on.** The lit end because a wall's two faces are one tile and there is no telling
which of them a pixel is on; the flame's end because a sconce is mounted *on* a
wall. What would answer it properly is knowing which *side* of its tile a mounted
light hangs on — the panel's own side is in the grid already, and a lamp pushed
just outside that plane would light the street and be stopped by its own wall
going the other way. That is the shape of the fix and it is in the backlog, not
in the code.

The tile *being lit* stays exempt whatever it holds, and the asymmetry is the
point. A wall's two faces are one tile — the backlog has carried that since the
sun arrived — so a pixel's fraction is clamped inside its tile whichever face it
is on, and testing its own panel would darken whichever face the flame is not
behind. There is no telling which that is. The flame's position is known; the
fragment's side is not.

Three answers and not two, and the third is the one to be careful about. A mask
of **all four** is "it stands up and the art would not say" — a corner, a post, a
tree — which is the whole-tile occluder decision 3 started with, so an unread
graphic behaves exactly as it did. A mask of **zero** is a *lid*: something
horizontal, whose occlusion is entirely its `z` span and which no vertical side
describes. The client's own `FLOOR` bit decides that and not the detector,
because a floor whose silhouette happened to read as a wall would otherwise stop
three quarters less light than it does today.

**And it deletes the door problem rather than solving it.** Decision 11 needed a
table of which graphics are open leaves, because an open door has the flags of a
shut one. It does not need one now: where the detector reads both, an open leaf
is on the *perpendicular* edge of its tile — 28 pairs out of 28, never once the
same axis — so a shut door blocks the doorway and an open one blocks the side of
the tile it swung against, out of the geometry, with nothing knowing what a door
is. Which is what decision 11 always claimed to be doing.

**18. A panel is *pierced*; a body is *travelled through*. The length of a
crossing is the wrong question for a surface.**

Decision 14 gave every occluding cell one rule: what it stops is its opacity
scaled by how far the ray ran inside it, over a softening width. That is right
for a solid and wrong for a plane, and the wrongness is not subtle — it is what
drew the **spokes**. A thin bright ray fanned out of every lamp standing near a
wall, one per tile corner, straight through walls with no hole in them.

The mechanism, because it is worth being able to recognise again: a ray that
clips the corner between two panels leaves the first cell *sideways*, so that
cell's own face is never among the sides it crosses; and it enters the second
cell *across the corner*, where the crossing is a hair long, so `length / soft`
rounds to nothing. Two cells, both holding a wall, and the ray passes both.

A panel is a surface. What it does to a ray is decided where the ray goes
through it: at a point, at a height, once. So the walk asks, for each side of the
cell the ray actually crosses, what height the ray is at *there* — and the tile's
`z` span answers yes or no. There is no length in it.

Three consequences, all of them measured rather than reasoned:

- **All four sides is a body, not four panels.** A mask of `EDGE_ANY` means "it
  stands up and the art would not say", which is the whole-tile occluder — and a
  roof is a lid five `z` deep. Pierce-testing a slab is the "stepped over the top
  of a wall" failure this file already carries, arriving from the other side: a
  45° ray that enters a roof's cell at 19 and leaves it at 22 pierces neither
  side inside the span while passing straight through the middle of it. It lit
  the floor of a sealed house. ~~Lids and whole tiles keep decision 14's length;
  only a cell whose art named one, two or three sides is pierced.~~ **Half
  superseded by decision 24**: a whole tile now keeps the length *and* is pierced
  on the sides it is crossed by, the larger answer winning, because "it stands
  up" is what a house corner falls back to and the sliver was leaking. A lid is
  unchanged and keeps the length alone.
- **The penumbra that survives is vertical.** A ray grazing the top of a wall is
  dimmed rather than switched, over a band of the same similar-triangles width
  decision 14 derived. Sideways there is no longer a gradient for a named panel:
  the shadow's edge is where the geometry says it is, which is a straight line at
  any angle rather than a staircase on tile boundaries, and that was the actual
  complaint. The band is centred on the *top* edge and hangs below the bottom
  one, because a wall is based on the ground it stands on and the ray a person
  looks at — a torch and a floor, both at `z = 0` — runs exactly along that base.
  Centred there too, every wall in the frame passes half its light along the
  ground: measured at `0.378` against an ambient of `0.356` before the line said
  so.
- **A corner is answered rather than left open.** Where the two boundaries land
  together within `CORNER_TIE`, the ray is crossing the point where four tiles
  meet, and the walk asks *both* of the cells that share it — at the height the
  ray passes through the corner — before stepping diagonally past them. That is
  the supercover walk the backlog has asked for since the first version of this
  pass, at two extra samples on the rays that hit a corner exactly rather than at
  twice the samples everywhere. It closes the diagonal gap
  (`a_ray_slips_between_two_walls_that_touch_at_a_corner` used to pin the leak
  and now pins its absence) and it is also what makes the two implementations of
  decision 9 agree: a ray through a corner is a knife edge, and the parity test
  found a pixel where the CPU stopped and the GPU did not.

**19. A window is not an emitter.**

615 of the install's statics carry `LIGHT_SOURCE`, and 80 of the 163 named
"window" are among them — `0x0103`, `0x2BBF`, the shutters at `0x2501`, the
windowed walls at `0x2B7D`. `light::flame` answers `TORCH` for any graphic it has
no name for, so a street of houses was a street of six-tile warm pools with
nothing burning in them: **64 flames in a Britain frame, of which seven were
fires.** And every one of the other 57 stood *inside a wall*, which is where the
whole complaint came from — a light in a panel lights the panel, and what escapes
it is whatever the geometry lets through. The spokes and the missing pools are
the same 57 lights seen from two directions.

The backlog offered three answers and this is the third of them: a window is not
a light. It is a hole with glass in it, it is already in the grid as
`occlusion::PANE`, and what should make it glow is a candle behind it — which is
the one thing this pass can already do. The flag is the client's way of saying
"draw a glow here" and this renderer answers that with geometry.

Stated as **"a light source that stops light is not a flame"** rather than as a
list of window graphics: the property is the one that matters, it is already
computed for the grid, and a shard's own lantern goes on burning for free. The
conservative direction is the right one here too — a missing pool is easier to
see than sixty invented ones.

**20. While the point lights are the subject, the ambient holds still.**

`docs/lighting_world.md`'s sky field — a room under a roof darker than the road
outside it, before anything burns — is **off by default** (F6), and the ambient
is one colour per frame again. Not because it is wrong: because it changes the
ambient of *every tile in the frame*, and a pool that looks wrong indoors is then
two questions at once. It is also the larger thing in a picture — in the light
view a city reads as a field of dark building-shaped blobs with the pools
somewhere inside them — so it hides exactly what a person judging a falloff needs
to see. `light::Ambient::flattened` sums the two terms back into the one they
were split from, so the flat picture is not a lesser version of the field: it is
the frame this pass had before the field existed, which is what a difference is
measured against.

**21. The screen-space glow is a *second layer*, not a thing that was replaced.**

This pass began as a circle in the pixels of the drawn image — the flame's tile
projected to a screen point, compared against the fragment's screen position —
and "Where it stands" above is written as though moving into the world had
*replaced* it. That framing is wrong, and it is worth correcting where it stands
rather than at the bottom: the two are different things and a lit frame wants
both.

- **The world layer** — everything decisions 1 to 20 are about — answers *which
  surfaces are lit*. It is a multiplier on the art, it knows about walls and
  heights and storeys, and it is what makes a torch inside a house not light the
  street. It cannot draw the flame itself: nothing in this renderer draws air,
  and a fire's own brightness is not a property of the ground under it.
- **The screen layer** is the *glare*: a soft radial falloff centred on the
  flame's own sprite, added over the finished picture. It is what the reference
  client draws (`light.mul` sprites blended over the scene) and it is the thing a
  person actually recognises as "a lamp" — the halo around the source, which is
  in the eye and in the air rather than on any surface. It was working, and it
  was the circle the complaint above remembers.

The two failures they have are opposite, which is the whole argument for keeping
both. A screen circle alone lights through walls and folds a cellar into the
street — the two reasons this pass moved into the world. A world multiplier alone
has no source in it: the brightest thing in the frame is a patch of floor, and
the flame is a sprite the same brightness as it was in daylight.

Composed, and in this order: the world layer multiplies the art, and the glow is
added on top of the result. Multiplying by the glow would tint whatever happens
to be drawn there and a black pixel would stay black, which is exactly what a
halo must not do — a lamp glares over the dark doorway behind it.

What the glow needs that the world layer already has: the flame's *screen*
position, which is the sprite's, not the tile's — `light.rs` places a light by its
tile and the backlog has carried that since the beginning; here it is the whole
point, because a halo half a tile from the burning sprite reads as a mistake
rather than as light.

**22. A wall's face is one-sided, and the stance is what says so.**

Reported from the client: a wall lit from inside a house glows on the street as
though it were made of glass. It is the one fact the attachment did not carry.
A wall's two faces are **one tile, one plane, one fraction and one height** —
everything decisions 2, 13 and 16 write — so nothing in the frame could tell the
street side of a house from the room side, and a torch in a room lit both
equally. `docs/lighting_world.md`'s backlog has been carrying it as "the sun has
no facing either" since the sun arrived.

Step 15 already measured the answer and threw it away: the *stance* — which edge
of its tile a wall stands on — was used to place a pixel's fraction and then
dropped, because the attachment's fourth channel is two bits of kind and fourteen
of fraction with nothing spare. It is not the only channel. The **third** is a
`z + 128` in the low eight bits of a `u16`, and the eight above it were empty; the
stance rides there now (`place::STANCE_SHIFT`), and `blit.wgsl` turns it into an
outward normal.

Which way is *outward* is not a guess. The art only ever draws the two faces an
isometric camera can see — step 15 measured that too, north and west being five
graphics out of 1197 — so a south face's picture is the surface turned towards
`+y` and an east face's towards `+x`. A flame behind that plane lights nothing,
over a band `FACE_EDGE` wide so that a lamp walking past the end of a wall does
not switch its face off between two frames.

~~**Except a flame standing in the wall's own line, which is part of that
wall.**~~ **Superseded by decision 26, and it was the wrong half of the pair.** A
lamp mounted on a house sits at its tile's *centre* — behind the plane of the very
face it is bolted to — so testing it blacked out the wall it hangs on, and the
exemption was what kept that from happening. What the exemption could not tell is
a mounted lamp from a **lamp post standing in the street**: a line is a whole
street long, so a post south of a house was "part of" the east wall three tiles
north of it and lit every face of that run at full strength. Reported from the
client, with coordinates. The answer was the one this paragraph already named and
put in the backlog — place the mounted light outside the plane its tile names —
and once it is placed there, the facing test needs no exemption at all.

**23. A wall does not shadow the rest of the wall it is part of.**

The second thing reported from the same picture: a thin dark stroke down every
tile seam of a wall, appearing only when the lamp is *beside* the wall rather
than in front of it.

Decision 16 is why. A wall's face lies **on** the panel it is the face of, so a
pixel of the face is a point in the plane of its own tile's panel — and the panels
of the tiles either side of it are in that same plane. A ray from that pixel to a
lamp half a tile out from the wall crosses the plane almost at once, and *where*
it crosses is a little further along the run than the pixel is: for the pixels
near the far end of each tile the crossing lands in the **next** tile, whose
panel is a wall. The ray is stopped by the wall it is standing on the face of.
The perpendicular case is clean because the crossing then lands in the pixel's
own tile, which is exempt — which is exactly the difference the report described.

A run of wall is one surface and no part of a surface shadows another part of it.
So a panel on the same side of its tile as the lit end's own, on the same *line*
— the same row for a north or south face, the same column for an east or west one
— is not an occluder for that ray. Anything else about that cell still is: a wall
tile that also carries the perpendicular face of a corner stops the ray on that
face as it always did.

The elevation view is what this was found in and it is worth keeping the order:
the artefact is invisible in a plan, because a plan's pixels are on the *ground*
and this is a defect of pixels on a *wall*.

**24. A thing that stands up is a surface on every side of its tile, and not
only a solid inside it.**

Decision 18 split a cell in two: a *panel* is pierced where the ray goes through
it, a *body* is travelled through and what it stops is scaled by the length of
the crossing. It put `EDGE_ANY` — "it stands up and the art would not say which
way" — with the bodies, and gave the reason: a roof slab five `z` deep is pierced
by neither of its sides at 45° while the ray passes straight through the middle
of it, and pierce-testing it lit the floor of a sealed house.

That reason is sound and it covers only half of `EDGE_ANY`. The other half is
**every corner of every building in the world**, and it brought the spoke back.

The picture came in as a lamp in a Britain street throwing a bright seam at 45°
out of a house corner, and the coordinates were `(1441, 1692)`. What is actually
there — `crates/client/render/tests/onsite.rs` prints it, and this is what that
file is for:

| tile | graphic | what the art says | in the grid |
|---|---|---|---|
| `(1440, 1692)` | `0x0037` | a south face | a panel on `S`, `z 0..=25` |
| `(1441, 1691)` | `0x0035` | an east face | a panel on `E`, `z 0..=25` |
| `(1441, 1692)` | `0x0033` | **nothing — a corner** | `EDGE_ANY`, `z 0..=25` |

A ray from inside the house to the lamp arrived at **85% strength**, and the path
is two cells long:

- It enters the last tile of the south run through that tile's **north** side and
  leaves it **eastwards**. It never crosses the panel that tile stands on — which
  is correct, and is decision 17's whole point: it is what lets a lamp light the
  street it hangs over.
- It then clips the corner tile. That tile is faceless, so it is a body, so what
  it stops is `length / soft` — and the sliver is 0.107 of a tile against a
  softening width of 0.7. It stops 15%.

Two cells, both holding wall, and the ray passes both. That is decision 18's own
sentence, word for word, arriving in the one branch decision 18 did not change.

So a cell whose mask is `EDGE_ANY` is asked **both** questions and the larger
answer wins: the length it was travelled through, and the sides it was crossed
by, pierced at the height the ray is at there. The length has to stay, because it
is what answers the roof slab; the pierce is what closes the sliver. A **lid** —
mask zero, a floor, a rug, a road — is not asked, and that is not an oversight: a
horizontal surface has no vertical side for a ray to pierce, and it is the one
case where the `z` span really is the whole of what the cell is.

`max` and not a sum, and the direction matters: nothing that was dark before
becomes lit. The change can only darken, which is the direction this file has
taken at every one of these forks — a missing pool is easier to see than a room
leaking into a street.

**What it costs is the last of the sideways penumbra**, and one test had to be
re-aimed to say so. `the_edge_of_a_shadow_passes_through_the_values_in_between`
swept across the fan out of an open doorway and read a gradient; a built scene
has no art, so every occluder in it was `EDGE_ANY`, and the gradient it was
reading was the length rule — the same softening that let the ray through the
house corner. It is now
`the_edge_of_a_shadow_lands_where_the_geometry_puts_it`, which asserts what was
worth having underneath it: the fan is wider than the doorway by a *fraction* of
a tile, so the edge is neither a tile boundary nor nothing. Decision 18 already
argued why a sideways gradient cannot be right here — it is measured from the
**cell's** boundary and not from the **surface's** silhouette, so wherever a wall
carries on into the next tile it is wrong in both directions at once.

The penumbra that survives is vertical, it was never measured, and it is now:
`a_ray_grazing_the_top_of_a_wall_is_dimmed_rather_than_switched`.

**Two of the three defects in that report are not this one**, and both are the
same missing fact — that a corner is two faces and the art will not name them.
They are in the backlog under "found at a house corner in Britain", with what
each needs. **Decision 25 is that fact.**

**25. A corner is two faces, and the art says so as plainly as it says one.**

Decision 3 refused to read an edge off a silhouette, step 15 read one, and
everything since has been written as though the answer were "one face or
nothing". A corner was the *nothing*: `Stance::Upright` in the attachment,
`EDGE_ANY` in the grid, and three separate artefacts following from those two
fallbacks — a flat 44-pixel band between two continuous runs of wall, both of its
faces lit whichever side the flame was on, and a whole-tile occluder where two
panels stand.

The measurement was already being made and then thrown away. `face_of` proposed
each half of the tile's column in turn and refused the graphic when the *other*
half held more than a wall's own thickness — so by the time it gave up it had
measured both halves and found both to be faces. That refusal was the only thing
between here and an answer.

So the halves are read **twice**, and the order is what makes the change safe:

- **Strictly first**, each half having to be the only face in the picture. That
  is exactly what the module did before, so every graphic it read reads the same
  today — 76% of Britain's walls did not move.
- **Then together**, each half offered the picture on its own. The only way
  through is that both are faces and each was refused for the other, which is
  what a corner is. A face beside a *blob* still fails, because the blob is not a
  face — two failures are not a corner, and that is the property the second pass
  rests on.

**Measured**: 91.9% of the wall statics standing in Britain, against 75.7%; 45.5%
of the install's wall art, against 36.3%. 297 corner graphics, 296 of them the
east-and-south pair a camera can see and one north-and-west. `tests/facing.rs`
prints all of it and asserts a floor under each, corners included as their own
count rather than as a share — a tail hidden in a percentage is a tail that can
go to zero unnoticed.

**A pixel is resolved to one of the two, in `statics.wgsl`, per fragment.** Which
half of the tile's column the pixel is drawn on is which surface it is a pixel
of; there is nothing else to ask, and nothing else needs asking. So the
attachment carries a single face with a single normal, `blit.wgsl` is not
touched, and `light::sample` has no case for a corner either. Ten stances need
four bits where six needed three, and **no format changed**: both words had eight
or more spare above the stance. The four corner values are laid out so the two
faces come out by arithmetic rather than by a table — `right = FaceNorth +
(offset >> 1)`, `left = FaceSouth + (offset & 1)` — because the shader does it
per fragment, and `place.rs` pins those two lines in a test.

**In the grid it is two bits, and two bits is the panel path.** Decision 18's
`edges` arm already handles a mask with more than one side in it; what changes is
that a corner stops being `EDGE_ANY` and therefore stops being a *body*. A ray
running alongside a corner — down the street it stands on — crosses neither of
its panels and passes, exactly as it does beside the runs of wall either side of
it. A ray from inside the house to a lamp outside still crosses one of the two
and is stopped, which is decision 24's leak staying shut.

**What it costs is a free-standing solid's other two sides.** A pillar filling
its whole tile reads as a corner, because it *is* one — the same two faces drawn
on the same two edges, and nothing in a silhouette tells a pillar from the corner
of a building. Shading it as two faces is right. Occluding it as two panels is
not quite: a building's corner has its north and west sides inside the house,
where a pillar's are in the open, so a ray clipping a pillar's far corner now
passes where it used to be stopped by the length rule. It is in the backlog with
what it would take; it is not a whole-tile answer coming back, because that would
take the street-lighting back with it.

**26. A mounted flame burns outside the plane its tile names, and the facing test
is geometry with no exceptions in it.**

Reported from the client with the coordinates, which is the shape of report this
pass has learned to want: the lamp at `(1441, 1693)`, the corner tile at
`(1441, 1692)`, and *the face leaning towards `(1442, 1692)`* — the corner's east
one — lit when it should not be. The lamp stands at `x = 1441.5` and that face
lies in the plane `x = 1442`, so the flame is half a tile **behind** the surface
it was lighting.

What lit it was decision 22's exemption: a flame standing in a wall's own row or
column is part of that wall. That is true of a sconce and false of everything
else standing in a street, and a column is as long as the street — so one lamp
post lit the far side of every wall in its column. `tests/onsite.rs` says it in
one line now, because that is the instrument this report needed: it prints each
of a tile's four faces with `through` and `facing` apart, and a face behind the
flame reads `through 1.000, facing 1.000` where a shadowed one reads
`through 0.000`. Two numbers, two different defects, and one of them invisible in
any picture of the ground.

The exemption existed for a real reason: a lamp bolted to a wall sits at its
tile's *centre*, which is behind the plane of the face it lights, so the geometry
blacks out the very wall it hangs on. But that is a fact about **where the flame
is**, not about which surfaces it may light — the map says "this tile" because a
tile is all the map has, and the lamp is really on the outside of the panel. So
the flame is moved rather than excused: `light::mounted_at` puts a flame whose own
cell carries a panel half a tile plus `FACE_EDGE` outside that plane, on the side
the wall's picture is drawn from, componentwise so that a corner's two panels are
both cleared.

Three things fall out of the move, and the second is the one that has been in
this file's backlog since its first version:

- **The wall it hangs on is lit at full strength**, because the flame is now in
  front of the plane by more than the band the facing test softens over.
- **A sconce stops lighting the room behind its wall.** The flame lands on the
  *next* tile, so the wall stops being the flame's own cell — which decisions 3
  and 17 exempt from shadowing it — and becomes an ordinary occluder. The oldest
  known-wrong entry in this file, and the test that pinned it
  (`a_sconce_lights_through_its_own_wall`) is now
  `a_sconce_lights_the_street_and_not_the_room_behind_it`.
- **The facing test loses its only exception.** A surface is lit if the flame is
  in front of its own plane, and that is the whole rule. Two comparisons less per
  light per fragment, in both implementations.

A flame on a tile with **no** panel is not moved, and that is what covers the
ordinary cases by construction: a torch on the ground, a brazier in a room, and
the lamp post the report was about. Neither is one whose sides cancel — a lid, or
the whole-tile `EDGE_ANY` of a graphic the art would not name — because there is
no direction in those to move along, and a guess would be a wrong one.

**27. A horizontal surface is a surface: it looks up.**

`Stance::Flat` carried no direction, so `blit.wgsl`'s facing test was skipped for
it and a flat pixel took the whole of every pool that reached it, from any side.
Reported from the client as two walls "adding up" at a corner — a bright diamond
wedged between a lit face and a dark one. Nothing adds; a fragment is lit once.
The diamond is the corner's **top cap**, a `Flat` static at the top of the wall,
and it was lit by a lamp standing two tiles *below* it as fully as one standing
over it. Measured at the reported corner before the change:

```
lid at z 25: through 1.000  facing 1.000
East:        through 1.000  facing 0.000
```

So a lid's normal is `(0, 0, 1)` and the facing test takes the third component of
an offset it already had — `blit.wgsl` computes the flame's offset with `z`
divided into tiles, which is the space the normal is stated in, so what comes out
for a lid is *how far above its plane the flame is, in tiles*, through the same
formula and the same [`FACE_EDGE`] band. One `select` in the shader, one arm in
`light::Surface::normal`.

**Still a half-space test and deliberately not a cosine.** UO's art is
pre-shaded: every wall's picture already has a light painted into it, so a
Lambert term would be a second light fighting the first. What this answers is
only which side of the surface the flame is on. The backlog has carried that
argument since decision 22 and it is what keeps this a rule rather than a
lighting model.

`Spot` stopped carrying `Option<Face>` and started carrying a `Surface` — flat,
one of the four faces, or upright — which is exactly what the attachment holds
per pixel after `statics.wgsl` has resolved a corner. `Upright` is still "nothing
is known, so every flame lights it", and it is still what a tree, a body and an
unread wall get.

**28. A surface does not shadow itself — which is not the same as a tile.**

*"Neither end of the ray is shadowed by the tile it is on"* (decisions 3 and 17)
was always reaching for this, and the tile was the only handle available at the
time. The reason it gave is a reason about a *face*: a wall's face lies **on** the
panel it is the face of, so that panel cannot be between it and anything, and a
pixel of a wall claims a fraction clamped inside its tile whichever face it is
on. An **upright** billboard's pixels are inside their tile too.

A **floor** pixel on the same tile is not ambiguous at all. It is the ground, it
is inside the room, and the ray from it to a lamp in the street crosses the panel
its own tile stands on — so a wall tile's own square of floor came out fully lit
against a dark room, which is the seam on the ground the corner report ended
with. It is visible in the plan view of `scene::sconce_on_wall` as a lit band
along the wall's own row, with the wall's shadow starting only beyond it.

So the exemption is asked of the **surface**: a face and an upright exempt their
own cell, a flat pixel does not. Two things bound it, and both are the direction
this file always takes:

- **Only a named panel.** A mask of all four is "it stands up and the art would
  not say", which is every tree, post and barrel — testing those would put a dark
  square under each of them out of a *fallback* rather than out of a measurement.
  A lid is not asked either: it has no vertical side, and the ground standing on a
  pier's plank is the open question the backlog already carries.
- **The flame's end stays a whole tile**, because decision 26 moved a mounted
  flame outside the plane its tile names, so what is left on that tile is not
  between it and anything.

`own_run` narrowed with it. It took the lit end's whole tile mask and it now takes
the side the pixel **is the face of** — which is what decision 23 says in words:
a corner's perpendicular panel is a different surface and stops the ray as it
always did. A pixel that is not a face is part of no run and gets nothing.

**29. What a cell should hold: panels, not one merged span.** ~~*(the shape of the
next format change)*~~ — **superseded in its storage half by decision 30**, which
puts the panels in a list the texel points at rather than inline in the texel.
What it says about *why* a cell needs more than one surface is why 30 exists, and
it is kept for that.

A cell is `(z_bottom, z_top, opacity, PRESENT | edges)` — one `Rgba8Uint` texel a
tile — which is an axis-aligned box with four bits saying which of its sides are
real surfaces. That is already the "polygonal wall" this pass needs and it is why
a corner is a proper object in it: `(1441, 1692), z 0..=25, sides E|S`. What it
cannot say is anything a tile holds **twice**:

- a lid and a wall on one tile merge into one span — conservative in the direction
  that darkens for the `z` and in the direction that leaks for the sides, which is
  not one direction;
- a window is a hole *in* a panel, so an aperture is a rectangle in that panel's
  own `(v, z)` — step 16, and the thing that makes a real shaft of light;
- two walls at different heights on one tile close the gap between them.

So a cell wants a small list of **panels**: a side, a `z` span, an opacity, an
aperture. The grid stays exactly as it is — a uniform grid over a world whose
every surface is tile-aligned *is* the acceleration structure, and the walk, the
per-frame build and the upload do not change shape. What changes is the texel.

Two things to decide when it is picked up, and neither is decided here: how many
panels a cell may hold before it truncates (two covers a corner, three covers a
corner with a lid, and the tail wants measuring on Britain rather than guessing),
and whether the second plane `Occlusion::field_bytes` already uploads is where
they go. What is **not** on the table is a list of boxes with an index of its own:
the CPU is already thirteen times the GPU on this pass and the grid build is most
of it.

**30. The occluding world is a baked list of surfaces, indexed by the tile grid
— derived from the art, and overridable by hand.** *(decided, not built)*

The grid is rebuilt **every frame**: `occlusion::collect` is 2.0ms of the pass's
3.3ms CPU against 0.31ms on the GPU. A house does not change between frames; the
camera moves. That is the strongest argument for baking and it is not about
freedom or effects — it is the largest single number in this pass.

What baking with real geometry buys on top of that, and what it does not:

- **Sub-tile holes.** A window is a hole *in* a surface, so a real one needs a
  rectangle in the plane of a panel. Today a pane dims the whole tile, which is a
  dimmer tile and not a beam.
- **Baked light.** A sky field, an ambient occlusion, a lightmap for the static
  world — computed once per region rather than blurred per frame, which is what
  `docs/lighting_world.md` does today.
- **A shaft with a shape** (step 17): the mask can come from the opening's own
  geometry rather than from a tile-sized approximation.
- **It does not remove the G-buffer.** What is drawn is a sprite, and a sprite's
  pixels do not lie where a box's faces do — the art has thickness, ornament and
  overhang, 44 pixels of picture on a 22-pixel edge. The place attachment stays
  the bridge from a drawn pixel to a world surface, and the stance stays its
  normal. **Geometry replaces the occluder, not the source of normals.**
- **It does not remove the measurement, which is the hard half.** A box has a
  window only if something read the hole off the art. Step 16 is that, it is the
  same machinery as `facing::facing_of`, and it comes first whatever the storage
  is.

The micro-decisions, numbered so one can be argued with alone:

**30.1 Derived first, authored as an override — and derived *offline*.** The
geometry is measured from the client's own art, so a stock install gets windows
with no assets at all; a shard that ships models overrides by graphic. The engine
must not require content the world does not come with — and a hand-made mesh per
building is thousands of assets, which is a Community Pack's business. **When**
that measurement runs is decision 31, and the answer is not "in a frame".

**30.2 A surface is a quad with holes.** A plane (one of the tile's four sides, or
a horizontal lid), a `z` span, a span along the run, an opacity, and up to `K`
apertures as `(v, z)` rectangles in the surface's own coordinates. That is the
whole vocabulary the art can be measured into, and it is what decision 25's
corner, decision 27's lid and step 16's window all are.

**30.3 The index stays the tile grid.** *(and decision 38 keeps it while changing
what it means: a cell's entries become references to solids that need not lie
inside it, so the grid stops being where geometry is stored and becomes only
where it is found)*

A texel becomes `(offset, count)` into the
surface list; the DDA walk is unchanged in shape and iterates a cell's two or
three surfaces instead of reading one merged span. A uniform grid over a world
whose every surface is tile-aligned **is** the acceleration structure — a BVH
would put its build on the CPU, which is the side that is already thirteen times
the GPU.

**30.4 Baked per block, and ~~per storey band~~ by block alone.** *(the band is
gone — decision 33 is why, and it landed before the bake did; the bake itself is
step 21.5 and `occlusion::bake`, and what it turned out to also need is decision
37)*

The band was here because the cutaway removed the storeys the player is not on
*at the map walk*, which made a built grid one frame's: a cache keyed by block
alone would have been invalidated by walking through a door, and keyed by band
the cutaway could *select* rather than rebuild. Decision 33 moved the cut to the
end, so what a block holds is the same for every frame and the key is the block.
What the server changes — a door's graphic, a ground item — stays in the
per-frame path, which is small and already exists.

**30.5 No storage buffers.** The ceiling is WebGL2 (`crates/client/render/src/lib.rs`):
no compute, no storage buffers. So the list is a **texture** read with
`textureLoad`, and the bake is CPU-side. This is the constraint that decides the
format, and it is written here because it is the one a session would otherwise
design around for an hour before finding it.

**The ceiling was questioned when decision 38 needed a second indirection, and it
was kept.** What it actually costs, item by item, is close to nothing here:

- **Compute shaders** — unused. The bake is on the CPU, per block, and the
  largest thing in it is the paste rather than the build.
- **Atomics and writes from a shader** — unused, for the same reason.
- **Storage buffers** — the only real loss, and in this pass a storage buffer is
  precisely "an array read at a computed index from a fragment shader", which a
  texture already is. `textureLoad` from an integer texture plus the address
  arithmetic is the difference; it is a dozen lines, not a millisecond.

So the indirection decision 38 needs is a cost of the *model*, not of the floor,
and it would be paid on WebGPU too. Two things are worth writing down beside
that. It is **not a WASM limit** — WASM has no opinion about GPUs; this is a
backend choice, and `wgpu` targets WebGPU as readily. And the place the floor
*would* bite is a GPU-side bake, GPU light culling, or per-frame variable-length
lists — none of which is in this plan.

What is left for a person rather than for this file: the sentence in
`crates/client/render/src/lib.rs` reads as a principle and is a dated assumption
— WebGPU was behind a flag when it was written and is broadly shipped now. The
question under it is not a graphics question: **is the web still a target?** If
it is, this floor is right and the texture indirection is the price. If it is
"one day, perhaps", saying so plainly is cheaper than carrying the constraint
through every decision and discovering later that it defended nothing. Keeping
*both* backends is the worst of the three: WGSL has no preprocessor, so a second
fetch path means a generated shader or `naga-oil`, which is a real cost paid for
tidiness.

**30.6 The truncation is measured, not chosen.** How many surfaces a cell may hold
comes from a distribution printed over Britain, and whatever is dropped is
*logged* rather than silently capped — a grid that quietly truncates reads as
"covered everything" when it did not.

**30.7 The walk's rules carry over untouched.** Decisions 17, 18, 23, 24, 25, 26,
27 and 28 are already stated about *surfaces* — a panel is pierced, a body is
travelled through, a surface does not shadow itself, a face is one-sided, a lid
looks up. That is what the last three sessions bought and it is why this is a
change of representation rather than a rewrite: the rules do not get relitigated,
and the parity test keeps holding both implementations to them.

**30.8 A hole is a plane beside the list, not four more channels of it.**
*(decided in step 21.3, and the format is what it decides)*

A [`Surface`] texel is four `Rgba8Uint` channels and all four are spoken for —
`(z_bottom, z_top, opacity, PRESENT | edges)`. A rectangle needs four more, so
the question is where they live, and there were three answers:

- **Interleave**: two texels a surface, the hole in the second. No new binding
  and no new upload, and it doubles the footprint of *the one texture the walk
  reads in a loop* in order to carry zeros — because a hole is what almost
  nothing has.
- **A third kind of list element**, a texel the count includes and the walk skips.
  Costs nothing when there are no holes, and it makes a cell's `count` mean
  texels rather than surfaces, so `histogram`, the truncation cap and decision
  30.6's distribution all quietly start counting a different thing.
- **A parallel plane** over the same indices, read only where a bit on the
  surface says there is something to read. One more binding, one more upload,
  and the hot loop is untouched.

The third, and the deciding argument is the one this pass makes everywhere else:
**a miss must be cheap.** `HOLED` is a spare bit of a byte that already had
three; the plane is written only when `Occlusion::any_aperture` is true, so a
frame of a map with no measured window neither lays it out nor sends it; and a
surface with no hole costs one bit test in the shader. The two planes are grown
together and never apart, because they are one list indexed by one number.


**31. The art is measured once, off the clock, and the engine reads a table.**

`facing::facing_of` runs while the atlas packs a sprite — on the frame a graphic
is first seen, on the player's machine. That was right when the measurement was
one pass over 44×80 pixels and there was one of them. It stops being right twice
over: a scroll that introduces four hundred graphics pays for four hundred of
them at once (this file's backlog has carried it as *"a second walk of pixels the
atlas has just copied"*), and every future measurement is a bigger one — an
aperture is a hole to be found, a corner is two fits, a mesh would be a solve.

So the measurement moves **out of the frame entirely**: a tool reads an install
and writes a table, and the client loads it. The budget goes from a frame to a
minute, and what that buys is not speed but *ambition* — a runtime pass has to be
a scanline trick, and an offline one can do connected components, fit and
cross-check, and print an outlier list for a person to look at.

**31.1 A tool, not a pass.** The same shape `docs/roadmap.md` already settled for
the Sphere scriptpack: a build tool, not an engine feature. It runs against an
install and writes one table for every graphic it could read.

**31.2 One table, and a hand-authored entry wins.** Decision 30.1's "derive first,
author later" becomes one artifact rather than two code paths: an override is a
row in the same table, and the tool leaves it alone. So a shard fixing one wall
edits a file rather than patching a detector.

**31.3 The generated table is not checked in.** It is derived from copyrighted
art, and this repository ships no client files, ever. What is checked in is the
**tool** and the **overrides**; the table is generated beside the install, into a
cache, on the machine that has the files. A pack that ships its own art may of
course check in the table for *its* art — that is its own content.

**31.4 Staleness is detected, not assumed.** The table records what it was
measured from, and a mismatch re-derives rather than trusting: art changes between
client versions, and a table silently describing a different install would move
every wall's face by a rule nobody could see. `docs/client_versions.md` is why
this is not paranoia.

**31.5 What moves is everything measured from a picture.** Today that is
`facing::facing_of`; tomorrow it is step 16's aperture, and after that whatever a
mesh needs. The runtime keeps a *reader*, and the detector's own tests keep
running against the client exactly as `tests/facing.rs` does now — the sweep is
already the tool, minus the file it writes.

**31.6 The client still works with no table at all.** It derives what it needs the
way it does today and says so in a log line. A missing cache is a slow first
frame, not a shard that will not start — the same refusal-to-guess this pass takes
everywhere else, arriving as a refusal to *require*.

**32. A lid is a plane, and a plane is crossed rather than travelled through.**

Decision 24 gave the walk two rules — a panel is *pierced at a point*, a body is
*travelled through* and scaled by the length of the run inside its span — and a
lid was put with the bodies. It reads right and it is wrong by a number: **a
floor is `height 0`**. Over the block of Britain `artscan`'s `column` example
reads, 4,534 of the 4,647 lids are zero deep. A span of no depth has no length
inside it, so `share` came out `0.0` for every floor in the world and a lid
stopped exactly nothing. What a player sees is a house whose upper storey is lit
from under its own floorboards, the upper wall brightest of all, because a wall's
face takes the ray head on. Reported from the client, reproduced in
`scene::storey_over_a_torch`.

So a lid gets the third rule, and it is the one its geometry asks for: **did the
ray get from one side of the plane to the other inside this cell**. Not a pierce
either — a pierce is a point on a *vertical* plane at a height, and a lid has no
height to be pierced at.

Two things about it are decisions rather than arithmetic:

- **The crossing is strict.** A ray that runs exactly along the top of a lid — a
  candle standing on the floor it lights, both at one `z` — has gone through
  nothing. This is `pierces`'s own asymmetry (its band hangs *below* the bottom
  edge because a wall stands on the ground a ray runs along) arriving at the
  surface that has no thickness for a band to hang under. Counting a touch would
  lay half a floor's shadow across every room lit from inside it.
- **The softness is the flame's, and it is measured at the flame.** The plane
  cuts the source, so what gets through is the share of the source left on the lit
  side: a flame standing in the plane of a lid is half cut by it, one a storey
  below it is wholly under it. A sunbeam is a point source and gets the hard edge
  a point source casts, which is the same `spread` parameter that already tells
  the two ends apart.

What a floor pixel of an upper storey gets from a torch *below* it is not this
rule's business and never was: decision 27 already refuses it, because a flat
surface looks up and the flame is not on the side it looks at.

`light::crosses` and `blit.wgsl`'s are one formula, held by the parity test.
Nothing else in the walk's *rules* moved — a body keeps decision 24's length and
its second pierce, a panel keeps decision 18's.

**What did move is three exemptions, and the rule stopping light was worth
nothing until all three did.** Each was invisible from the others, and a scene
that read one spot four tiles from the flame would have passed with any two of
them fixed:

- **Neither end's own cell may exempt a lid.** Both exemptions are statements
  about things that *stand up* — a pixel lies on the panel it is the face of, a
  billboard's pixels are inside their own tile, a mounted flame burns outside the
  plane its tile names (decision 26). None of that is true of a horizontal plane.
  A sconce at `z 36` and the storey's wall at `z 45` are the **same tile** with
  the floor at `z 40` between them, which is how a real house is built, and both
  ends were exempting it. It costs nothing where the exemptions earn their keep:
  a ray only crosses a plane inside its own cell when the other end is nearly
  straight above or below it.
- **A vertical ray still has one cell to ask.** "Straight up or down: the only
  cells on the line are the exempt ones" was true when the exempt cells were
  exempt in full, and it is the shortcut a torch directly under a plank falls
  through. The walk now asks that one cell's lids before returning.
- **Both ends of the ray stand where they are drawn, not inside what they are
  drawn on** (`stand_clear`). A face pixel is walked from a hair in *front* of the
  plane it is the face of, and every point of the world from a hair *above*
  whatever it lies on. The first is geometry the attachment cannot carry:
  `statics.wgsl` keeps a face pixel a hundred-and-twenty-seventh short of its own
  plane, because a fraction of exactly one names the next tile and the
  attachment's tile is what a click selects — right for the attachment, wrong for
  the walk, because the floor whose edge meets that plane belongs to the tile in
  front and the ray was crossing it in the wall's own column, which has no plank
  over it. The second is what the strict crossing test costs: a point whose `z`
  is exactly a floor's lies *in* the floor, so the ray runs along the plane
  rather than through it. Strict the test must stay — inclusive, it lays half a
  floor's shadow across every room lit from inside it — so the point moves onto
  the boards instead, and so does the flame, because a candle stands on a floor
  rather than in it. **Neither nudge closes the line alone**, which is why they
  are one change: with only the height the ray still crosses the plane a column
  too early, and with only the offset it starts in the plane and runs along it.
  Only the walk moves; picking, the wireframe and the debug views still read the
  wall's own tile.
- **An exemption reaches only as high as the surface it is about**
  (`on_surface`). A tile of a two-storey house carries a wall per storey —
  `0..20` and `20..40`, two surfaces since step 21.2 — and a pixel at `z 25` lies
  on the upper one. The lower one is under its feet and occludes it exactly as
  anybody else's wall would. Exempting it let every ray out of the room below
  climb the column of its own wall tile, which is the one tile a house's floor
  never covers. This is decision 28 said with the `z` it never had, and it
  narrows `own_run` by the same argument.

**33. What a ray may cross and what the frame draws are two sets, and the cut
between them is at the end.**

Decision 4 said nothing occludes that was not drawn, and it is still true of the
picture. What was wrong was *where* it was decided: `collect` asked
`cutaway::shows` at the map walk, so what came out of a `Builder` was one frame's
grid — and a per-block cache of one frame's grid is not a cache. That is the
whole of what 30.4's storey band was working around, and the band would have had
to be re-argued the moment a ray was allowed to cross a storey the frame did not
draw.

So the walk builds **what a ray may cross** — every surface standing on the map
inside the rectangle — and `Builder::finish` applies the frame's `Cutaway` as it
packs. Everything above that line is a fact about the map and can be built once
and kept; everything below it is a fact about the tile the player is standing on
and costs one predicate per surface, on a copy that was already happening.

Three things this decides rather than assumes:

- **The cut needs two facts and a surface now carries both.** `Surface::bottom`
  is the `z` the static stood at, and `Surface::roof` is the flag a roof is
  cut by at any height. Nothing in the walk's rules asks either — `roof` exists
  for this and says so.
- **The rule has one spelling.** `Cutaway::shows_at` is `shows_static` with the
  tiledata row already read, and `shows_static` calls it. A second copy of "at or
  above `max_z` it goes, and a roof goes once the player is under one" in the
  occlusion module would be a second policy, not a second caller.
- **The draw ceiling does not move.** The other half of `cutaway::shows` — a
  static past `DRAW_CEILING`, or one the client marks internal — is a fact about
  the static and not about the player, so it stays at the map walk as
  `cutaway::drawn_in_any_frame`. A mountain top a hundred and fifty `z` up is
  drawn in no frame from any tile, and no cache wants it.

What this does **not** decide is whether a ray *should* cross a storey the frame
took away. It stays exactly as it was: the frame's grid is the drawn set, the sky
field is not (`lighting_world.md`'s decision 3), and the two are as far apart as
they have always been. What changed is that the question is now asked in one
place, on one line, over a list that already exists — so the day light is made to
reach the storey above a torch, that is a change to which set `finish` keeps and
nothing else.

**34. A body has a footprint, and the art can only measure one axis of it.**

A surface is a plane on an edge, a lid, or **the whole tile**, and the third is a
fallback: `facing_of` refuses a picture it cannot read an edge in, and what the
grid then does is stop light across the entire square. Measured on Britain's
`1509,1635` — the tile a person pointed at because it was the one lit thing in a
dark house — the graphic is `0x00CC`, whose silhouette occupies **columns 12 to
31 of 44**. Twenty columns of art became an occluder across the whole tile,
standing among neighbours that are panels on one edge. It over-blocks in every
direction at once, and the view shows it as the odd shape it is.

So a body gets a **footprint**, and the whole of the decision is what can honestly
be measured for one. The projection is what says: world `+x` moves the screen by
`(+22, +22)` and `+y` by `(-22, +22)`, so a sprite's **column** is
`(fx - fy)` and nothing else. The other diagonal, `(fx + fy)`, is depth — a single
picture cannot say how far back a thing goes, and inventing it would be decision
3's mistake made again.

A footprint is therefore a **band across the tile in the `(fx - fy)` axis,
unbounded along the other** — which is exactly the shape a panel's run already
is, one axis measured and one refused. It is `(near, far)` in
[`RUN_STEPS`](crate::occlusion::RUN_STEPS)ths, the same units and the same byte
pair a `Hole` carries.

What it costs, and why this is the cheap one of the two:

- **The measurement is a pass that already happens.** `facing_of` scans the
  silhouette by column; the band is the first and last column with a pixel in it.
- **The format already fits.** The surface texel is full, but the *aperture*
  plane beside it is `(near, far, bottom, top)` per surface and is allocated only
  when something has a hole. A body's footprint is two of those four numbers, so
  it rides in the same plane under a flag of its own, and no texture grows.
- **The walk gains one clip.** A body is travelled through, so what changes is the
  length: the segment inside the cell is clipped to the strip. Closed form, exact,
  a few ALU. The side pierce of decision 24 moves with it — the sides that stop a
  ray are the strip's own boundaries rather than the tile's.
- **A full-width picture gets no footprint at all.** The band is only written down
  when it is narrower than the tile, so every body in the world behaves exactly as
  it does today unless the art says otherwise. That is the direction this file
  takes at every fork.

**35. A sloped surface is deferred, and the reason is that its consumer does not
exist yet.** Four corner heights instead of two, which is a second texel per
surface; and the lid's crossing test stops being a plane test and becomes a
bilinear patch — two triangles, a ray-plane test each and a containment test,
in both implementations. Worse than the arithmetic is what it reopens: the
strictness of the seam, `on_surface`, and the direction `stand_clear` nudges a
point are all *stated about an axis-aligned plane*, and each of the three was a
defect found the hard way.

And it would not buy the thing it looks like it buys. A roof in this client is a
slab five `z` deep and decision 24 deliberately keeps the travelled-through rule
for it, so a ray at 45° cannot step over it. What is genuinely sloped in this
world is the **land** — four corner heights per tile — and the land is not in the
occlusion grid at all (the backlog's "a hill between a campfire and a valley
stops nothing"). So the order is: land in the grid first, and slopes with it or
not at all. Written down here so that the next person to want it finds the price
rather than the idea.

**36. An occluder is a box in the tile's own coordinates, ~~and a plane where the
art cannot say how deep it is~~.** *(the first half stands and is the reason this
decision exists; the second half is withdrawn — decision 38 is why, and it also
takes the "in the tile's own coordinates" out of it)*

The rules of this grid have grown one shape at a time and each one arrived the
same way: not as a new rule about light, but as a **form the surface record could
not state**, faked with a flag. A corner became two panels (decision 25). A wall
got a hole (21.3). A body got a footprint (34). A stair is a solid, and its treads
are a shape there is still no way to write down. Five special cases, one after
another, and none of them was ever about how a ray behaves.

So the record becomes a shape: a **box**, `(u0..u1, v0..v1, z0..z1)` in the
tile's own unit square, with an opacity. Everything the grid holds today is that
box with two of its six numbers pinned:

| today | as a box |
|---|---|
| a lid | zero height |
| a body | the whole tile |
| a tread | part of one axis |
| a footprint (decision 34) | part of the other |

And the walk gets **simpler**, which is the argument that matters more than the
tidiness: `blit.wgsl` has three rules today — pierce a plane, travel a span, clip
to a strip — and a ray against a box is one slab test, three pairs of
comparisons, closed form. The same box gives the *shading* half its answer for
free: a pixel's normal is the normal of the face it landed on, which is what
`place::Stance`'s nine values are a hand-rolled enumeration of.

~~**A panel stays a plane, and that is the one thing not folded in.**~~
*(withdrawn — the argument is left standing so that the next person to reach for
it finds it already answered)*

~~A wall's thickness is not in the art — decision 3 is that argument and it has
not changed — so a box for a wall would need a depth somebody invented. Worse, a
zero-thickness box is not the same test as a plane: "the segment overlaps a slab
of width zero" is a numerical coin toss where "the segment crosses this plane" is
exact, and the seam rules (decision 16, `on_surface`, `stand_clear`) are all
stated about a plane and each was a defect found the hard way. Two primitives,
then, and the pair is honest about which one we can measure: a plane where only
one axis is known, a box where the shape was fitted whole.~~

Two things are wrong with it and the second is the interesting one.

**The coin toss is an argument against zero, not against a box.** Give a wall a
thickness of two forty-fourths of a tile and the slab test is exactly as
well-conditioned as the plane test. What the paragraph did was insist the box be
degenerate and then object to the degeneracy.

**And "the art cannot measure it" is a bound on a *detector*, not on a record.**
Decision 3 is right and unchanged: no single sprite says how deep a wall is, and
a detector that invented a depth would be making decision 3's mistake. But this
whole track is the authoring one — decision 31.2's `authored` row already exists
and already wins — and the moment a person can write six numbers, "unmeasurable"
stops being a property of the model and becomes a property of the *fallback*.
Provenance is then a column of the table, not a second primitive in the shader.

So the vocabulary is **one solid**, and a derived one is a solid with one
measured axis and a nominal other. What is still not folded in is the **hole**:
it is a subtraction rather than a body, a box with a bite out of it is two
primitives or one exception, and the exception already works.

What it costs:

- **The surface texel doubles.** Four bytes today (`bottom`, `top`, `opacity`,
  `edges`); a box needs six or seven. The aperture plane already exists beside it
  and decision 34 already plans to put `near`/`far` there, so this is a second
  texel in an existing plane rather than a new texture: ~140KB to ~280KB at the
  widest zoom.
- **A hole is still not a box.** It is a *subtraction*, and it keeps its own
  field. A box with a bite out of it is two primitives or one exception, and the
  exception is the one that already works.
- **Nothing may move in the picture on the way.** The migration is: express the
  four existing kinds as boxes, keep every current test green — they are the
  specification of what must not change — and only then let a tread be a box that
  is a part of a tile. A step that changes both the representation and the
  picture is a step where a difference cannot be attributed.

Where this parts company with decision 35: that one deferred *slopes*, and it is
still deferred and still right — a bilinear patch reopens three rules that each
cost a day. A box does not. A flight of steps is horizontals and verticals, which
is exactly what this world is made of, and the shape that was missing was never a
slope.

**37. What invalidates the bake is the *art*, and the art has a revision.**

Decision 33 made a `Builder` a fact about the map, and decision 30.4 read that as
"so a block can be built once". Both are true and neither is the whole of it: a
surface is derived from the map **through the atlas** — which edge a wall stands
on, the hole in it, the solid a stair is — and `occlusion::shape_of` falls back to
the whole-tile answer for a graphic the atlas does not hold.

**An atlas grows.** A graphic the camera has not reached yet is not in it, so a
block baked a second before that graphic was packed holds `EDGE_ANY` where the
atlas can now name a face — and nothing about the baked block would ever say so.
The wall would stay a body for as long as the player stood still. That is the
whole class of bug a cache has that a rebuild does not, and it is the quiet kind:
the picture is a *little* wrong, in a way that looks like the detector failing
rather than like a cache being stale.

So the fact the bake depends on is given a name and a counter.
`StaticAtlas::revision` counts changes to exactly the three answers
`occlusion::Shape` is made of — a facing, a hole, a prism — and a `Bake` keeps the
revision it was built under and drops **everything** when it moves. Three things
about that shape are deliberate:

- **A counter and not a comparison of contents.** "Has the atlas changed" asked of
  the maps themselves is a scan of a few thousand entries every frame, to answer
  no.
- **Bumped where something is actually packed, not per call.** The app offers the
  atlas every visible graphic on every frame, and a bump per *call* would tell the
  bake its shapes had changed sixty times a second — a cache that is cleared every
  frame is not a cache, and it costs exactly what having none costs while looking
  like it works.
- **Pixels are not in it.** A dirty row is a texture upload and changes no
  geometry.

The map itself is the other input, and it is not versioned: a `Bake` is one map's,
the caller owns that, and this client has one map. That is stated rather than
enforced because the alternative — a map that could tell you it had changed —
would be a facet-wide dirty bit for a case the client does not have.

**38. The tile grid is a broadphase index, and a solid is a body of the world
that no cell owns.**

Decision 36 made the record a shape and left it *in the tile's own unit square*.
That last clause is the one carrying the damage, and it is worth naming what it
has cost, because the bill is already in this file: **every seam here was
manufactured by cutting geometry on a tile boundary.** The spokes of decision 18
were a ray slipping between two panels that meet at a corner — and there is a
corner there only because the wall was cut where the map's storage happens to be
cut. Decision 16's fraction of exactly one, `on_surface`, the direction
`stand_clear` nudges a point: three rules, three days, all of them about what
happens *at a cut*.

So the solid stops being cut. A solid is a box in **world** coordinates with its
own six numbers, and a cell holds **references** to every solid whose extent
touches it.

**38.1 Reference, not clip — and that is the whole of the argument.** A ray
crossing the join tests the same one solid from both cells and gets the same
answer twice; there is no hairline left to slip through, and the fix is a
property of the representation rather than a fourth rule about seams. A solid
overlapping four cells is referenced four times, which costs four `u16`s. It was
cut into four pieces before, which cost four records *and* the seams between
them.

The walk is unchanged in shape: the DDA of decision 14 still steps cells, a cell
still yields a list, and the test is still one slab test. A solid spanning two
cells may be tested twice on one ray; a visited-set that avoided that would cost
more, on a ray of a dozen cells, than the redundant test it saves. So it is not
deduplicated, and the test being exact is what makes that safe.

**38.2 A solid is anchored, and its reach is *measured* rather than limited.**
The anchor — the tile the static stands on — is the whole of the invariant a
solid needs, because it names the block that owns it. How far the solid extends
past it is nobody's business but the geometry's.

~~A solid may not extend further than one tile beyond its anchor.~~ *(an invented
constant, withdrawn the day it was written. This file's own decision 30.6 says
the shape of the answer: measured, not chosen.)*

What genuinely needs a number is not the model but the **bake**. Blocks are baked
independently (30.4) and a frame pastes the ones it needs, so a solid anchored in
block `A` and reaching into block `B` puts references into `B`'s cells that only
`A` can supply. The frame therefore pastes a **ring** around the blocks it wants,
and the question is how wide the ring is.

It is measured, and the measurement is free: a solid belongs to a *graphic*, so
the widest reach in the whole world is `max` over the table's solids and is known
before the first block is baked. Zero on a stock install; one after somebody
authors an arch; three if somebody authors a bridge. Nothing is refused and no
graphic is special-cased — a large solid simply costs what it costs, every frame,
and the ring is exactly as wide as the content made it. This rides on the fact
decision 37 already tracks: the table changes, the radius is recomputed and the
bake is dropped, one path.

The bookkeeping is the owner's. A `Baked` block carries, beside its cells, a
small **spill** list of the references reaching outside its own bounds; the frame
pastes the ring's spill and nothing else of it, which for every block in a stock
install is empty.

The one thing that must not happen quietly is a person paying for a reach they
did not intend, so the radius is **logged**: one line saying how many blocks wide
this table makes the ring. A cost that is visible is a cost somebody can decide
about; a silent one is how a frame gets slower for a reason nobody can name.

**38.3 The pixel's face is the same slab test.** The projection is orthographic,
so "which face of the solid is this drawn pixel on" is a ray from the camera
through the pixel against the same box — the same arithmetic, the same code, a
different origin. That is what gives the stepped lid of `0x0736` three
horizontal treads instead of two vertical half-walls, and it is why
`place::Stance`'s nine hand-enumerated values become a derived answer instead of
a taxonomy to extend. One-sidedness (decision 22) stops being a rule at the same
time: a box's back face is real, and the artist simply drew no pixels that land
on it.

Note what does **not** change: the drawn frame. `statics.wgsl` puts the sprite
where it put it, the G-buffer is still the bridge from a pixel to a world
surface (30.'s fourth bullet), and the camera has no opinion about any of this.
That is exactly why the freedom is affordable — a solid is consulted by the
light and by the normal, and never by the rasteriser.

**38.4 The format grows one indirection, and it is the model's cost, not the
floor's.** A cell becomes `(offset, count)` into an **index** plane of solid ids,
and the ids address a **solid** plane. Two textures where there is one, one more
`textureLoad` in the walk. Decision 30.5 carries the measurement of what WebGL2
costs here, and the answer is that a storage buffer would be a tidier spelling of
the same fetch.

The count of solids goes **down**, not up: a flight of five tiles of stair is one
solid, not five; a run of wall is one per graphic instance rather than one per
cell it crosses. Today's 18,071 surfaces over 10,212 cells are largely an
artefact of tile-shaped storage, and 30.6's distribution is measured again after
the migration rather than assumed to carry over.

**38.5 Nothing may move in the picture on the way, and the migration is
therefore two steps and not one.** First the ownership changes with the geometry
held still — cells reference solids, every solid is exactly the box its surface
was, every scene and the parity test green. Only then may a solid be a shape no
surface could have been. A step that changes both where geometry lives and what
it is, is a step where a difference cannot be attributed, and this file has said
that twice already for smaller changes.

**39. The scene is already three-dimensional. What is missing is a primitive.**

This was written down because the wrong mental model cost an hour of argument in
the session that produced it: asked whether a wall could be drawn as a solid, the
answer given was "that is a mesh pass, a depth buffer that agrees with a
painter's sort, multi-session work" — and every clause of it was false. The
renderer is not a sprite blitter that would have to *acquire* a third dimension.
It is a three-dimensional scene whose primitives happen to be billboards.

What is already there, item by item, because each one is a thing that would
otherwise be built:

- **World space is the space the lighting reasons in.** Decision 1 moved it
  there and the whole file since is about the consequences. A fragment is lit by
  the tile and height of the thing drawn at it.
- **A per-pixel world position** is written by all three world passes (step 2).
  That is a G-buffer position plane; it was never called one.
- **[`Camera::project`](../crates/client/render/src/camera.rs) is a
  view-projection matrix** written as integer arithmetic. There is no
  trigonometry in the file, because there is no rotation: an orthographic camera
  at one fixed angle.
- **The depth buffer is hardware**, written and tested by both passes, and
  `crates/client/render/src/depth.rs` is the ordering ported from ClassicUO.
  Draw order between passes was deliberately made not to matter.

**39.1 The projection is exact, and the world is anisotropic.** A tile is 44
pixels wide and 22 high on the screen, and one `z` is
[`Z_STEP`](../crates/client/render/src/camera.rs) = 4. So a unit of height is
about 0.18 of a unit of ground, and geometry placed in world coordinates lands
in the same pixels the sprite for that tile lands in — **to the pixel, with
nothing fitted**, because the sprite is placed by the same map.

The trap is on the way in: a solid written with equal axes, a "real cube", comes
out five and a half times the wrong height. The non-uniform scale is part of the
projection and is carried, not corrected.

**39.2 The depth is the client's ordering, not the distance to a camera — and
that is the one place a solid does not simply fit.** `depth::Order` is
`(x + y, priority_z)`, discrete and **per instance**; `statics.wgsl` says in as
many words that deriving it in the shader would be a second chance to disagree
with `depth::Order`. For a sprite that is right. For a **box spanning several
tiles** it is not: one instance depth, several tile depths underneath it.

Two honest answers, and the first is enough for a long time:

- **Translucent, over the frame, writing no depth.** For looking at geometry this
  is not a compromise but the thing wanted: the wall's sprite is visible *inside*
  the box that claims to contain it, and the top face is what makes its thickness
  legible.
- **Per-fragment depth through the same `Order`.** The fragment knows its own
  world point, so the key is computed from the point rather than from a new
  formula — the rule this file uses everywhere: cite the function it came from.
  This is what a solid occluded by the sprites in front of it needs.

**39.3 Three faces, always the same three.** With no rotation, an axis-aligned
box shows exactly `+x`, `+y` and the top; its outline is a hexagon. So a solids
pass is not a mesh pipeline — no index buffers, no back-face culling, no asset:
it is an instanced quad pass of the same shape as `statics`, six numbers and a
colour per instance, the corners emitted in the vertex shader. Three constant
normals shade it for free, which is what makes the top face read as thickness.
And instancing through vertex buffers is how statics already draw, so decision
30.5's floor is untouched.

**39.4 Drawing a slope is nearly free, and it is not decision 35.** Under an
affine projection an inclined face is still a parallelogram, and a stair's prism
is a few of them — one more parameter in the same shader. Decision 35 priced
something else entirely: a *sloped surface in the ray walk*, which is a bilinear
patch and reopens the three seam rules. The two must not be confused, and the
good news in the distinction is the order it allows: a shape can be **looked at**
long before the walk can integrate it, which is the right way round.

**39.5 The billboards stay billboards, and that is the design.** The art is drawn
for this projection and for no other, and the depth must stay the client's
ordering or the picture stops matching the client the engine exists to serve. So
this is not a renderer on its way to being general — it is a three-dimensional
scene with a fixed camera, sprite primitives, and now a solid one beside them.

**39.6 The pass is not what makes it a solid; the projection is. But a picture
nothing can capture is not an instrument.** Both halves of this were learned in
one sitting, in that order.

The first: `Camera::project` already had a float core waiting to be named, so
`project_exact` takes a place between the tiles, `project` is it at a whole one,
and one test pins the two together over the whole map. Twelve points through
that, three polygons out. The geometry is the durable half —
`render/src/solid.rs` knows nothing about who paints it — and the first version
of 23.0 painted it through the *egui painter*, beside the wireframe, and looked
right.

The second is why that version did not survive the day. **`render` takes its
pictures headless**: `tests/cost.rs` builds Britain at the widest zoom on a real
adapter, times the passes and writes the frame out; `tests/pictures.rs` does the
same for the plan views. A diagnostic drawn by the client's UI toolkit appears in
none of them, and cannot be timed beside the passes it runs with — so the one
number 23.0's DoD asked for was the one number that arrangement could not
produce. A view whose whole job is to be looked at has to be capturable by the
thing that takes the pictures.

So the solids are `render/src/solids.rs`: two pipelines over one shader — a
triangle list for the faces, a line list for the silhouette — drawn after the
blit, on the surface, translucent, writing no depth. And the split above is what
made that a small change rather than a rewrite: **the projection stayed in
Rust.** The corners arrive already in viewport pixels from `Solid::faces`, and
`solids.wgsl` does the one thing no CPU can do for it — a pixel into clip space,
and the blend. A vertex shader deriving a box from its two corners would be a
second implementation of the arithmetic every sprite in the frame is placed by,
which is exactly what `statics.wgsl` refuses to do about depth and for the same
reason. The cost of keeping it in Rust is one buffer write a frame, and it is
measured rather than argued.

What the pass still cannot do, stated so it is not rediscovered: a solid is not
**occluded by the sprites in front of it**, because it draws over the finished
picture. That is 39.2's first answer and it is the wanted one for an instrument;
the second answer — per-fragment depth through `depth::Order` — is what a solid
that has stopped being a diagnostic will need.

**39.7 The lattice is the corners, and the tiles are the centres.** The one trap
the projection had left in it. `project` takes a `Point` and returns where that
tile's diamond is *centred*, so a solid whose extent is stated in the same
numbers — "tile `(x, y)`, from `x` to `x+1`" — is half a tile off, in both ground
axes at once, which on screen is one clean step down. `WorldSpot` is therefore
the corner lattice and `WorldSpot::centre` is the only place the half lives.
Written down because it is invisible in a wireframe of single tiles and obvious
the moment a box has to contain a sprite.

**39.8 A view of the grid has a second datum, and "this storey" is not one of
its values.** Both views drew what stands above the player's feet, which is the
right answer for a picture of *what shadows you* and the wrong one for a picture
whose subject is geometry: standing in a room at `z = 0`, the room's own floor
and every lid under it are simply not drawn — and **a hole in a floor and a floor
below the cut are the same picture.** Counting what was hidden does not close
that, and the distinction is worth keeping: a count says *how much* is missing,
never *where*. An instrument that can be wrong in a way indistinguishable from
the defect it is pointed at is the one failure a diagnostic may not have.

So `solid::Cut` is a switch, with the two values that can be *stated*:
`BelowFeet(z)` — what could shadow somebody standing here, and why a pier is not
2,011 boxes — and `Nothing`, the whole grid, unreadable in a town by design. F4
in the client, a pair beside the two checkboxes, and it governs **both** views,
because they are read against each other and two grids cut differently cannot be
compared.

*This storey* is the value a person would reach for most and it is deliberately
absent: it needs a ceiling, and therefore a rule for which of the four lids over
your head is *yours*. Inventing that rule to fill out an enum would put a third
answer into the instrument that no test could hold. It arrives when a room is a
thing the world can name.

The cut is resolved once per frame (`App::solid_cut`) and never stored: what a
person picks is one of two questions and holds across frames, while the `z` in
`BelowFeet` is a fact about the frame it is drawn in. One join, so no stale
height can be kept anywhere and the two views cannot drift apart.

**40. A surface's normal is a value it carries, not a tag naming which of three
axes it must be — and it stays a box on the tile grid regardless.** Written down
because the alternative was argued for at length in the same session that found
the tread cutoff (`FACE_EDGE`, the backlog under "Found while building the
treads"), and the argument deserves the answer on the record rather than a
re-litigation the next time a curved roof or a mountain comes up.

The case *for* a general triangle mesh — arbitrary vertices, a BVH, `Solid`
becoming index buffers — was that this renderer already builds real geometry
(decision 39's boxes have eight real corners) and will build more of it, so why
cap the shape at three constant normals. The answer is that every shape this
world actually has is already a **box in the tile's own coordinates** — decision
36 settled that for lids, bodies, treads and footprints, and there is no
graphic in a stock install whose art implies a wall or a roof that is not one.
What decision 36 left as a fixed constant is the box's *normal*, taken from
"which axis-aligned face did the ray land on" rather than computed from the
box's own geometry — and that is the one thing a box does not have to be
degenerate about. A **land tile already carries four corner heights**
(`Map::land_corners`, `crates/common/uofiles/src/map.rs:670`) and
[`crate::light::Spot::flat`]'s ground case flattens them to one
(`average_corner_z`) for *position* and has never asked them for a *normal* at
all — the slope a mountain's art draws is real data this pass already reads and
already throws away before lighting sees it.

So the box's shading normal generalises from a fixed three-way tag to a value
computed from whatever vertices the box actually has — a tread's top tilted by
its own rise and run, a land tile's plane fitted to its four corners — while
the box stays exactly what decision 36 made it: **anchored to one tile's cell,
found through the same grid, baked once per block rather than raycast against
per frame.** That is the whole of what a triangle mesh was being asked to buy
and the whole of what it would have cost twice over for it: a BVH replacing a
grid that is free precisely because every box in it is tile-aligned, and a
ray-plane test becoming a ray-mesh test that `blit.wgsl` and `light.rs` would
each have to get identically right for decision 9's parity to hold, for
content — arbitrary curvature — that the stock art never draws. Nothing here
is drawn as a triangle soup anywhere the client's own files do not already
imply one, and nothing not tile-aligned is ever asked to occlude.

**Decision 35's ordering still holds and is the gate on the land half of this**:
land is not in the occlusion grid at all yet, so a land tile's normal has
nowhere to be read from until it is. The tread half has no such gate — a stair
is a solid already, decision 36's table already lists it — which is why the
stairs are where this is proven first (`docs/lighting.md`'s backlog, "Found
while building the treads", and the handoff that starts the next session).

## Steps

- [x] **1. `render/src/occlusion.rs`.** The tile grid of decision 4/5, built
      from the map, the tiledata and the cutaway over the bounds `light.rs`
      already computes. Pure CPU, no GPU types, tested without client files: the
      builder takes occluders one at a time and the map walk is the caller.
- [x] **2. The `(x, y, z)` attachment.** `ground.wgsl` and `statics.wgsl` gain a
      second output; the quad structs gain the tile they are for; the renderer
      gains the texture and a second colour target. The frame tests read it back
      and assert that a wall's pixel names the wall's tile.
- [x] **3. `light.rs` in world coordinates.** A `Light` becomes a tile, a `z`, a
      radius in tiles. `place` and `FLAME_LIFT` go; the lift becomes `z` units.
- [x] **4. `blit.wgsl`.** Reads the attachment and the occlusion texture,
      computes the world distance and the ray's product of opacities.
- [x] **5. Wiring.** `app/src/lib.rs` carries the place attachment through the
      three passes and into the blit; `light::collect` builds the grid itself, so
      no call site grew an argument.
- [x] **6. A picture, and a number.** `render/tests/cost.rs`, ignored and gated
      on `OPENSHARD_CLIENT` and an adapter: Britain at the widest zoom, 1920×1080
      on screen over a 3840×2160 world image, drawn once and then lit five ways.
      64 flames, 10,212 standing cells in a 187×187 grid, and 256,101 of the
      2,073,600 fragments — an eighth of the screen — changed by a flame.

      ```
        case   ms/frame    ns/pixel     over dark
        copy      0.173       0.084      -23.9%    Lighting::NONE, the pass as a blit
        dark      0.228       0.110         0      the grid and the ambient, no flames
         far      0.363       0.175      +59.6%    the same 64 flames, 1000 tiles away
       night      0.388       0.187      +70.3%    the frame as played
         sun      0.249       0.120       +9.3%    no flames, a midday sun
      ```

      Read from the *differences*, which is why the cases hold the frame still
      and change one thing. Lighting a night frame costs **0.215ms** of GPU over
      a plain copy — 1.3% of a 60Hz budget. Of that, 0.135ms is `far`: sixty-four
      flames that reach nothing, on every fragment. The pools and their ray walks
      — the part with all the arithmetic in it — are the remaining 0.025ms,
      because only an eighth of the screen is inside any pool. **The misses cost
      five times what the light does**, and that is decision 6's claim inverted:
      see the backlog.

      The sun is 0.021ms, which `Occlusion::tallest`'s ceiling test is what buys
      — two or three steps over open ground rather than 32. It is no longer
      gated on cost; what still keeps F8 off by default is that its ray steps a
      whole tile, which the backlog has carried since step 11.

      And the CPU, on the same frame: **`light::collect` is 2.83ms**, of which
      `occlusion::collect` is 1.56ms and laying both planes out as bytes for the
      queue is 0.04ms. That is thirteen times the whole GPU pass, and it is paid
      on every frame the camera moves.

      The picture is the same test's, written where `OPENSHARD_FRAME_DUMP` says:
      Britain at night with its windows lit and its streets dark — the pools stay
      inside the houses, which is the claim the whole pass was built for, on real
      art rather than on a built room.

      ```sh
      OPENSHARD_CLIENT=… OPENSHARD_FRAME_DUMP=/tmp/britain_night.ppm \
          cargo test --release -p openshard-client-render --test cost -- --ignored --nocapture
      ```

      Two things the numbers do not cover, stated so nobody reads them as more
      than they are: the frame is `Cutaway::OPEN` with no ground items on it, and
      each case's batch pays a grid upload per pass where a real frame pays one —
      identical in every case, so it cancels in a difference and inflates every
      absolute equally.
- [x] **7. `light::sample`, the reasons in Rust.** The shader's loop and its ray
      walk, on the CPU, returning per flame: the distance in tiles, whether the
      fragment is inside the radius, what survived the walk, and *which cell*
      stopped it. Unit-tested on its own before anything draws with it.
- [x] **8. `render/src/scene.rs`.** The rooms of decision 10 — a closed room, a
      doorway, a window, a sconce on a wall, a cellar under a street, and the
      diagonal gap the backlog names — each a `Map`, a `TileData`, an item list
      and a camera, plus an ASCII diagram of a scene's lighting for the message a
      failing test prints.
- [x] **9. The debug views.** `render/src/debug.rs`'s `View`, one field in the
      lighting uniform, a `switch` at the end of `blit.wgsl`, and F11 in the app
      to cycle them: the place, the kind, the height, the occluders, the light
      alone, the shadow term alone, and how many flames reached a fragment.
- [x] **10. The parity test.** A synthetic place attachment uploaded to the GPU,
      the real blit run over it, and every sampled pixel compared with
      `light::sample`. No client files and no art — this is about two
      implementations of one formula, and decision 9 is what it protects.
- [x] **11. Sunlight on the floor.** Decision 12's directional term: a sun
      direction in the uniform, the same grid walk without an endpoint, a wall's
      shadow on the street and a lit patch behind a window. `WINDOW` no longer
      borrows `NO_SHOOT`'s answer — `occlusion::PANE` passes four fifths — and
      the sun is F8 in the app, off by default. Step 6 has since measured it —
      0.021ms a frame, 9% of the pass — so what keeps it off is no longer the
      cost but the tile-stepped ray the backlog named — and that has since been
      fixed too, so what keeps the sun off by default is now only that nothing
      has asked for it to be on.
- [x] **12. A floor is not a wall.** Decision 13's `place::Stance`: a flat
      static's fraction is the inverse of `camera::project` over the pixel's
      offset from its tile's centre, an upright one's is the tile's middle, and
      the bit comes from `TileFlags::FLOOR`. A room's floor stops being flat
      44-pixel diamonds with a step at each seam.
- [x] **13. The ray walks its cells.** Decision 14: a grid traversal with the
      length of each crossing and the share of it inside the tile's span, and a
      penumbra whose width is `FLAME_SPREAD * t / (1 - t)`. `light::walk` learns
      the same walk; the parity test is what says they agree.
- [x] **14. The occluder boxes, drawn.** The instrument the next two steps are
      judged with, and the answer to "why is there a shadow where nothing
      stands". `Occlusion::boxes` is the iterator over the cells that hold
      something — open tiles are most of a grid and are skipped, so a caller
      spends nothing on them; `Hud::occluders` carries the grid beside the
      terrain overlay under its own checkbox; `shell::draw_occluders` takes each
      cell's two diamonds through `Camera::tile_diamond` at the span's clamped
      ends and strokes the twelve edges, coloured from glass to bone by the
      cell's opacity so a `PANE` and a wall are told apart. No GPU pass and no
      new texture: this is arithmetic the camera already does.

      Three decisions inside it, each the same one the shader made and therefore
      not a second policy: the grid is rebuilt over `light::lit_tiles` (now
      public for exactly this) rather than over the drawn tiles, so the wireframe
      covers what the walk covers; the `z` span is clamped into an `i8` the way
      `Occlusion::bytes` clamps it, so a box is drawn where the shader believes
      it is rather than where the map says; and it is built from the frame's own
      `Cutaway`, which is now handed to `App::hud`.

      **The cost**: one more walk of the map's statics over the lit bounds per
      frame *while the box is ticked*, and twelve strokes per standing cell. Off
      by default, like the terrain overlay, and the boxes whose eight corners all
      fall outside the clip rect are dropped before a shape is built — at the
      widest zoom most of the grid is offscreen.

      What it is expected to show first: **a door's shadow is a tile wide**,
      because decision 3's occluder is the whole tile and not the leaf — which is
      the report that started this step.
- [x] **15. A wall's facing, measured from its art.** Decision 3 is right that
      `tiledata.mul` does not say which edge a wall stands on. The *art* does,
      and what says it is the **base edge** — the lowest drawn pixel of each
      column, which is where the wall meets the ground and the one part of a
      wall's silhouette with no ornament on it. Two independent bits come out of
      it and together they are the four faces:

      | face | runs along | occupies | base descends |
      |---|---|---|---|
      | N | `+x` | right half | to the right |
      | E | `+y` | right half | to the left |
      | S | `+x` | left half | to the right |
      | W | `+y` | left half | to the left |

      Verified against the client before a line was written: `0x0100` "marble
      wall" has its mass in columns 18..=43 with the base descending left — the
      east face — and its base lands on the predicted `dy = 22 - across` **to the
      pixel** over the whole 22-column span. `0x0007` is the south face of the
      same shape and lands the same way. That is the not-circular check the whole
      step rests on.

      Then a pixel maps onto that face instead of onto the tile's middle. With
      `v` along the edge and `(dx, dy)` the offset from the tile's centre:

      | face | place | `v` |
      |---|---|---|
      | N | `(v, 0)` | `dx/22` |
      | E | `(1, v)` | `1 - dx/22` |
      | S | `(v, 1)` | `1 + dx/22` |
      | W | `(0, v)` | `-dx/22` |

      And the height is **one line for all six stances**, which is more than the
      plan hoped for: the point of the tile a pixel's picture rises from is
      `(place.x + place.y - 1) * 22` pixels below the tile's centre row, so
      `z = z0 + ((sub.x + sub.y - 1) * 22 - dy) / 4` covers the four faces, the
      faceless upright case (where the term is zero and this is exactly the old
      `z0 - dy/4`) and — read with an *unclamped* fraction — the flat case too.
      The formula generalises rather than replaces, and `BOTTOM_LIFT` is gone.

      **The point is the seam**, and it is what the frame test asserts: the next
      tile along the run starts its `v` at 0 where this one ended at 1, so a row
      of wall tiles is one continuous surface. Held to by two mutations — the
      run reversed, and the run replaced by a constant — each of which fails it.

      Where it went: `render/src/facing.rs` holds `facing_of(&Image) ->
      Option<Facing>` (`face_of` at the time; decision 25 made the answer a
      face *or a corner*)
      and `silhouette`, the fixture both the unit tests and the GPU test are
      drawn against (`pub` for the reason `scene.rs`'s rooms are); `StaticAtlas`
      calls it once while packing and keeps the answer on `Sprite`;
      `place::Stance` grew from two values to six; `statics.wgsl` gained the
      switch. `light.rs` and `blit.wgsl` were not touched — the attachment is
      their input, exactly as planned.

      **What it reads, measured rather than hoped.** 36% of the install's 3,212
      `WALL` graphics, and **76% of the 4,596 wall statics standing in Britain**
      — the second is the number that decides how a frame looks, and the two are
      reported apart because the table is mostly things nobody built with. The
      unread remainder is corners, posts, roof slabs flagged `WALL`, and
      multi-tile buildings shipped as one graphic; every one of them keeps
      today's behaviour exactly. `tests/facing.rs` prints both, pins seven named
      graphics to their verdicts, and asserts a floor under each share — a
      detector with no coverage count is a green light for having checked
      nothing.

      Four things the detector had to be taught by being caught getting them
      wrong, all four by measurement rather than by reasoning:

      - **It only looked at the half it had proposed.** A 106-pixel statue read
        as a north face because its mass, far off to the left of any tile edge,
        was never tested. Every one of the 15 "north" graphics was that bug.
      - **`SPILL` at six pixels refused most of a city.** A wall is a solid and
        the picture shows its thickness; where it is low enough to look down on,
        its whole top surface is drawn — 8.5 pixels on `0x0063`, the garden wall
        Britain is fenced with. Twelve reads 76% of the map where six read 40%,
        and a corner still covers the whole other half, 21.5 pixels of it.
      - **A slab is not a wall.** A roof piece has the right 45° base and no
        height above it, so the detector asks that the thing stand up.
      - **A 45° line has the same slope wherever it sits.** Everything above
        measures the base line's *direction*; nothing pinned down its
        *position*, and nothing had to — `statics::stand_on` puts a sprite's
        bottom row on the diamond's bottom vertex, so the edge's screen position
        is fully determined by the face, with no freedom at all. `0x0171` is what
        that bought: a flat diamond drawn eighty pixels above its own tile, an
        awning, whose lower-right side is a clean run in the empty right half. It
        passed every other gate and was shaded as a vertical face. The gate is
        that a base pixel lands within three pixels of the edge it claims — and
        the client agrees to the pixel, median zero over 908 graphics. It removed
        45 graphics from the table and **not one instance from Britain**, which
        is what a false-positive gate should do.

      **Doors are not a special case, and it shows.** 558 graphics carry `DOOR`
      and are offered like anything else. The ones read sit on a tile edge as
      squarely as a plain wall — median distance zero, none over two pixels — so
      an open leaf, where the art puts it on an edge, is shaded along the axis it
      actually swung to. The wide open leaves, 56 to 106 pixels across, stick
      past their own tile and die on `OVERHANG`. Decision 11 said an open door is
      a static that stopped being an occluder and nothing here knows what a door
      is; this arrives at the same place from the shading side.

      **And the art only ever draws two of the four.** Every wall the install
      ships stands on its tile's `y1` or `x1` edge — the two an isometric camera
      can see the face of; a `y0` or `x0` face is a surface turned away from the
      viewer and there is no picture of one. North and west are five graphics and
      one, out of 1,197. The enum keeps all four because the *geometry* has four
      edges and a detector that could not name one could not be caught naming it
      wrongly.

      **Not for the occlusion grid**, and that has not changed: there a wrong
      guess is a room leaking onto the street, which is what decision 3 refuses;
      here it is shading that looks odd. 76% is the number that conversation now
      starts from — and it is the same key that unlocks the sconce lighting
      through its own wall and the sun lighting both faces of one.
- [x] **16. The window's aperture, measured off the art.** A pane passed four
      fifths of the light *across the whole tile*, which is a dimmer tile and not
      a beam. The hole is in the art — a window graphic's silhouette has a
      transparent gap inside an opaque wall — and `facing::aperture_of` reads it:
      a span of `v` along the face and a span of `z`, in the surface's own
      coordinates.

      **58 pictures out of 39,189, and 56 of them carry the client's own `WINDOW`
      flag** — which is the cross-check worth having, because nothing in the
      detector looks at a flag: the agreement is between a silhouette and a table
      neither half was reading. The other two are `0x21FF` and `0x2200`, a ruined
      wall with a hole knocked in it, and they are right too. Weighted by what
      stands in a city: **85 wall statics in Britain** have a window, out of four
      graphics — `0x003C` and `0x003B`, the arched windows of a plaster house, and
      `0x00B9`/`0x00BA`, the same in stone.

      **What is measured is a rectangle in the surface, and the art draws an
      arch.** `0x003C` is a doorway with a flat sill, straight sides and a rounded
      top — two pixels taller in the middle than at its ends. So the answer is the
      **largest rectangle inscribed** in what the art left transparent, searched
      over every sub-run of the columns (`O(n²)` over at most 22 of them, which is
      the sort of arithmetic decision 31's budget was bought for). A bounding box
      would let light through stone the artist drew.

      **A measurement is relative and a placement is absolute**, and that is the
      one structural thing this step added: `facing::Hole` is `z` above the
      *static's own base*, because one picture stands on a hundred tiles at a
      hundred heights, and `Aperture::above` is the single conversion, called in
      `Builder::add` where the instance's `z` is. `Shape` carries the measurement
      — one row, one lookup, both verdicts — and `Shape::of` is the one function
      the tool and the table-less client both measure through, so the two cannot
      drift into a window that exists only where somebody ran a tool.

      The gates, each of which a real client picture fails: a hole must have wall
      either side of it along the run (`HOLE_MARGIN`), a column with two gaps in
      it is a lattice and not a rectangle, gap columns that are not one run of
      them are two windows, a corner is refused outright (nothing in a silhouette
      says which of its two faces a hole is in), and anything under three columns
      by two `z` is a scratch in the art. The refusals are counted, not guessed
      at: of the 244 `WINDOW`-flagged pictures the detector reads a face on, 81
      have no hole drawn at all (the glass is painted opaque — `0x00CB` is one),
      46 are lattices, and 61 fail a gate.

      Held by nine tests in `facing.rs` — the round trip on all four faces, a
      solid wall with none, the corner's refusal, the margin, the scratch, two
      gaps in a column, two holes along the run, and **both directions of the
      inscribed rectangle** (an arch that keeps its width and a chimney that keeps
      its height) — plus the format's round trip, the atlas seam with and without
      a table, and the install sweep, which pins `0x003C`'s four numbers by hand.
- [x] **20b. The measuring tool, and the table it writes.** Decision 31: the
      silhouette work leaves the frame. `tests/facing.rs`'s sweep was most of it
      already — it walks an install, reads every `WALL` graphic and prints the
      shares — so what this added is the file, the loader, the staleness key and
      the override merge. Doing it *before* step 16 is what lets step 16's
      measurement be as expensive as it needs to be, and it closes the backlog
      entry about the atlas walking the same pixels twice.

      **Where it went**, and the split is the one this workspace already has:
      `client/render` never opens a file (its `Cargo.toml` says so and means it),
      so `render/src/arttable.rs` holds the *type* and its text and nothing else,
      and the new crate `crates/client/artscan` holds everything with a path in
      it — the sweep, the file, and the reader `client/app` loads through. The
      reader lives with the tool rather than in the app on purpose: they are the
      two ends of one file, and a client that looked for it somewhere the tool
      does not write is a bug that reads as "the table does nothing".

      **The format is one file and hand-editable**, which is decision 31.2 rather
      than a taste: a row is `0x0104 corner E S`, a comment is `#`, and an
      override is the same row with `authored` on the end — the tool re-derives
      everything else and leaves those alone. `data/overrides.table` is what this
      repository ships (decision 31.3: the tool and the overrides are checked in,
      the generated table never is, because it is derived from copyrighted art),
      and it holds **no rows today** — the mechanism is held by tests, because a
      row invented to exercise it would be a wrong answer shipped to every shard.

      **An absent row means measured and refused**, and the header's `examined`
      count is what makes that legible: a table that had swept the `WALL` graphics
      alone would be claiming a verdict about fifty thousand pictures it never
      opened. So the sweep offers *every* graphic the install ships, which is also
      exactly what the atlas does — the table has to answer the questions the
      atlas asks, not the ones a wall would.

      **Staleness is detected** (31.4): the stamp is the art container's name and
      byte length plus `facing::DETECTOR`, a version to bump when a gate in
      `facing.rs` changes. Two independent halves because they fail
      independently — a different install, and the same install read by different
      rules, the second of which nothing else in the file could ever say.

      Measured on a 2D install, `cargo run` with no `--release`:

      ```
      pictures with art: 39189
      read:              6150  (15.7%)
      corners:           4362
        East         824
        East+South   4359
        North        46
        North+West   3
        South        872
        West         46
      ```

      **Four seconds**, which is the number step 16 gets to spend against: the
      budget decision 31 bought is a minute and this is the first thing in it.
      The corner count is the one to look at twice — see the backlog.

      Held by nine tests. Five are the format (a round trip that keeps every
      verdict, a derived refusal that is an absent row, a re-derivation that
      leaves an authored row alone *in both directions*, a sheet of overrides
      handing its rows to a measured table, and a stamp that is stale if either
      half differs); two are the seam (`a_packed_sprite_takes_its_surface_from_the_table`
      and the same with no table, which is the fallback decision 31.6 promises);
      and two need a real install — every graphic's row against a live
      `facing_of`, and a stale table refused through the real reader over the real
      art file.
- [x] **21. The surface list.** Decision 30, **and it was five changes rather than
      one**. They are listed in the order that kept every one of them testable on
      its own, and nothing here waited on anything else:

      1. ✅ **The list and the walk over it, with the union kept.** A cell stopped
         being one merged span and became `(offset, count)` into a list of
         surfaces; both walks iterate a cell's one or two or three. The picture
         did not move, which is the whole point of doing it first: a cell maps
         one-to-one onto surfaces — a lid is one horizontal, named sides are a
         quad each with the same span, and `EDGE_ANY` is one **body** rather than
         four quads, which is why the list has two kinds of element exactly as
         the walk has two rules. Every existing test stayed green, the parity
         tests included, so the break it could have made would have landed in the
         plumbing and nowhere else.

         What it is made of: `occlusion::Surface` and `occlusion::Builder` — the
         merge now lives in the builder and only there, which is what makes 21.2
         a change to one function. `Occlusion::at` survives as the **merged
         view**, folded on demand for the readers whose question is genuinely
         about a tile: the wireframe overlay, the plan view, and which way a
         mounted flame steps out of its own cell. Three textures instead of two —
         the grid is the index `(offset & 255, offset >> 8, offset >> 16, count)`,
         and the list is a texture `SURFACE_ROW` wide read with `textureLoad`,
         which is decision 30.5 arriving. A cell's surfaces are combined with
         **`max` and not a product**: two panels on one tile are two faces of one
         corner, and a ray crossing both has gone through one thing once.

         And the first number for decision 30.6, off Britain at the widest zoom:
         **10,212 standing cells hold 10,653 surfaces**. Four hundred and forty
         one cells in a city block carry more than one, which says what the union
         has been merging away — and what 21.2 is about to multiply.

         The cost, measured on the same frame, is the new baseline rather than a
         comparison: `light::collect` 3.37ms of CPU with `occlusion::collect`
         2.06ms of it, 0.05ms to lay all three planes out as bytes, and on the
         GPU `copy` 0.181ms, `dark` 0.254ms, `night` 0.368ms, `sun` 0.514ms. No
         like-for-like before-and-after was taken — the scene has changed since
         step 6's numbers, so the two are not comparable, and what would want
         watching is that the walk now reads two texels a cell where it read one.
         Step 21.5 is where that is bought back several times over.
      2. ✅ **Split the union.** Two statics on one tile stopped merging into one
         span with one mask. This is the one place the picture *had* to change —
         it is the backlog's "a cell merges a lid and a panel into one mask and
         one span" — so it is its own change with its own test, and not smuggled
         in under a refactor that claimed to change nothing.

         The union was wrong in two directions at once and the change closes
         both. For the **span** it was conservative: two walls with air between
         them closed the gap, so a frame carried a band of shadow with nothing in
         the picture casting it. For the **mask** it leaked: a floor over a wall
         tile handed its `z` to the wall's span and lost its own lid-ness, so the
         walk pierced a horizontal surface as though it were a vertical panel and
         travelled through nothing — and a pane beside a wall came out opaque
         across the whole tile, because the opacity was a `max` too.

         `occlusion::Builder::add` now decides what a static *is* — a lid, a body,
         or a panel per side its art named — and pushes it. Nothing merges. What
         is left of the fold is `Occlusion::at`, the **merged view**, which is
         unchanged and is what the wireframe, the plan view and `light::mounted_at`
         go on reading: their question is genuinely about a tile. A tile's
         surfaces live in a linked list in one arena rather than in a `Vec` a
         tile, and that is a cost decision — 35,000 tiles at the widest zoom
         would otherwise be 35,000 allocations a frame on the side of this pass
         that is already thirteen times the GPU.

         Three tests pin it and each fails on the union: two walls keep the air
         between them (`two_occluders_on_one_tile_stop_closing_the_gap_between_them`),
         a lid and a panel keep their spans and their two rules
         (`a_lid_and_a_panel_on_one_tile_are_not_one_surface`), and the walk
         itself passes a ray through the gap
         (`a_ray_through_the_gap_between_two_walls_on_one_tile_passes` — built by
         hand rather than out of a scene, because two statics on one tile is the
         thing a `Map` makes fiddly and a `Builder` makes one line). The union was
         put back for a run to check they were red, and they were.

         **The distribution decision 30.6 asked for**, Britain at the widest zoom
         — `tests/cost.rs` prints it now, and `Occlusion::histogram` is what it
         asks:

         ```
           surfaces   cells      share
                  1    5942      58.2%
                  2    2702      26.5%
                  3     759       7.4%
                  4     428       4.2%
                  5     164       1.6%
                6–10     186       1.8%
               11–21      31       0.3%
         ```

         10,212 standing cells hold **18,071** surfaces, against 10,653 under the
         union. Nothing was dropped, and the cap is the format's own byte rather
         than a number anybody chose: the worst tile in a city is 21, an eighth of
         what an `(offset, count)` can name. `Occlusion::dropped` counts what does
         not fit and `cost.rs` prints it — a grid that quietly truncates reads as
         "covered everything" when it did not.

         **The cost, and it is not free.** On the same frame and the same machine
         as 21.1's numbers: `light::collect` 3.43ms against 3.37, the grid 2.19ms
         against 2.06 — the walk that builds it is unchanged and what grew is the
         list. On the GPU `night` is **0.497ms against 0.368**, which is the
         backlog's "a cell's fetch count went from one to `1 + count`" arriving
         with a count that is now 1.77 rather than 1.04. It is still 3% of a 60Hz
         budget, and step 21.5's bake is where the CPU half is bought back.
      3. ✅ **The aperture in the walk, tested on a built scene.** A surface got a
         rectangular hole — `occlusion::Aperture`, a span along the run and a span
         of `z` in the surface's own coordinates — and the crossing test asks
         whether the ray went through it. No art was needed, exactly as planned:
         `StaticAtlas::state_aperture` is the seam step 16 will fill from a
         silhouette, and a scene states one directly.

         **The change is small because decision 30.7 said it would be.** A panel
         was already *pierced at a point* rather than travelled through, so the
         point was already being computed; what step 21.3 adds is that the point
         has two coordinates instead of one and is asked about a rectangle.
         `light::pierced` and `blit.wgsl`'s are the whole of it, and everything
         above them — `own_run`, the corner case, the body's second answer, the
         sun — reaches them unchanged.

         Four things were decided along the way, and each is a refusal rather than
         a mechanism:

         - **Only a named panel may have a hole.** A lid is horizontal and a body
           is "it stands up and the art would not say which way", so neither has a
           plane for a rectangle to be stated in. `Builder::add` drops one offered
           to either — decision 3's refusal arriving one level down.
         - **A corner carries it on both of its panels.** They are the two faces of
           one picture, so a hole measured off that picture is the same window seen
           from either side, and nothing in a silhouette says which half it was in.
         - **The run coordinate is a byte**, `occlusion::RUN_STEPS`, a
           two-hundred-and-fifty-fifth of a tile — finer than the seven bits the
           place attachment carries a *pixel's* fraction in. Quantised once, in
           `Aperture::new`, so that both walks read the same byte and divide it by
           the same number: the parity test is exact rather than to a tolerance.
         - **A hole's edges soften symmetrically**, which is why `inside` is a
           second function beside `pierces` rather than a call of it. `pierces`
           hangs its band below the bottom edge because a wall is based on the
           ground and the ray a person looks at runs along that base; a hole's
           edges are in the middle of a surface and no ray runs along them, so a
           band centred there would move the hole half a penumbra downwards.

         Held by five tests and each was run against the mutation that should
         break it. Two aim a ray by hand — `a_ray_through_a_hole_in_a_wall_passes_and_one_beside_it_does_not`
         for the run and `a_ray_over_a_hole_in_a_wall_is_stopped_by_the_wall_above_it`
         for the height, which is the axis no picture of a floor can ask about,
         because a floor pixel and a flame are both near `z = 0` and every ray in
         that picture crosses at one height. One is the scene:
         `scene::wall_with_a_hole_in_it` is `torch_before_a_wall` with the middle
         tile's graphic swapped for one that carries a hole, so the wall either
         side is the same graphic at the same height and a fan that appeared
         without the hole would be some other defect. It asserts the fan is there,
         that the tiles either side are at the ambient exactly, and — measured as
         the width at half the sweep's own peak, because a hole this size is seen
         through a penumbra of about its own width — that it is **wider three and
         a half tiles out than one and a half**. Two are the format:
         `only_a_named_panel_carries_a_hole` and
         `a_hole_is_uploaded_at_its_own_surface_s_index`, the second because a
         shader reading the hole plane at the wrong index would draw something
         everywhere and be wrong only where a window is. And the GPU parity test
         has a sixth fixture, `the_shader_and_light_sample_agree_about_a_hole_in_a_wall`,
         which goes red when the shader is made to ignore the hole.

         **The cost is nothing measurable, and the reason is decision 30.8.** No
         graphic in any install has an aperture until step 16 lands, so
         `any_aperture` is false, the plane is neither laid out nor uploaded, and
         the `HOLED` bit is never set — what a real frame pays is one bit test per
         pierce. No like-for-like number was taken, because there is nothing yet
         to measure: the frame that will want measuring is the first one with
         windows in it, and that is step 21.4's.
      4. ✅ **The tool, the table and the measured aperture** — steps 20b and 16,
         both landed and both written up at the top of this file. The tool reads
         an install once and writes a table beside the client; the measurement is
         `facing::aperture_of`, 58 pictures of 39,189 and 85 wall statics standing
         in Britain.
      5. ✅ **Bake it.** Decision 30.4's block cache, and it is **1.22ms to
         0.37ms** on the frame the breakdown below was taken on.

         `crates/client/render/src/occlusion/bake.rs`. A `Bake` holds one
         `Baked` per map block — the surfaces its statics stand and the sky they
         take, in cell coordinates so the same bytes serve a frame at any offset
         — and `bake::collect` assembles a frame by pasting the blocks its
         rectangle overlaps, then does the three things that are genuinely per
         frame: the server's ground items, the blur, and the pack with the
         frame's `Cutaway`. Everything a block holds goes through the same
         `occlusion::place` the uncached walk uses, which is now one function
         rather than two copies of a pair of lines.

         **The property it rests on is equality and not similarity**, and it is
         asserted twice. `a_baked_grid_is_the_one_the_walk_builds` compares the
         packed `Occlusion` of a baked frame against a walked one on a built
         town — four blocks, a run of wall crossing a block boundary, two
         statics on one tile, a ground item, and both of the cutaway's two cuts
         — and `tests/cost.rs` makes the same comparison on **Britain, every
         batch**: 25,702 statics over 187×187 tiles, which is where a read-out
         that dropped a rim tile or reordered a tile's run would show. Both were
         run against the two mutations that should break them (drop the per-tile
         reverse; do not paste the sky) and both go red.

         Equality to the byte is available because nothing about the assembly is
         approximate. A tile's statics all live in its own block, so a block's
         surfaces and its sky are entirely its own; within a block the map's
         order is `(y, x)`, which is the row walk's order restricted to that
         block, so a tile's surfaces arrive in the same order either way; and the
         sky is *assigned* rather than multiplied in, because no two blocks share
         a tile and the ground items come after — so the integer rounding of
         `sky * passes / 255` happens in the same sequence in both.

         **What it cost to get right that the plan did not name: decision 37.**
         A surface is derived from the map *through the atlas*, and an atlas
         grows — so a block baked before a graphic was packed holds the
         whole-tile fallback for ever. `StaticAtlas::revision` is the counter and
         the `Bake` drops everything when it moves.

         **The numbers**, `what_the_grid_costs_to_build` and `tests/cost.rs`,
         release, Britain at the widest zoom — 187×187 tiles, 25,702 statics,
         17,201 surfaces on 10,212 standing cells:

         ```
         phase                       ms     cumulative
         allocate the builder     0.001      0.001
         walk the map             0.073      0.073
         + shade the sky          0.125      0.199
         + add the surfaces       0.668      0.867
         + blur and pack          0.352      1.220   (`collect` itself)

         camera                      ms     served   built   blocks held
         still                    0.366       9000     600           600
         one tile a frame         0.363       9050     650
         ```

         **The companion is the "served" column and it is asserted, not just
         printed**: a bake that rebuilt every block would cost what the walk
         costs and read identically in a millisecond. A still camera serves 600
         of 600 after the first frame; a camera moving a tile a frame builds
         about three and a half blocks a frame and costs *the same* 0.36ms,
         which is the reading that decides the thing — a widest-zoom frame is
         550 blocks and a tile of pan buys at most one new column of them.

         What is left in the 0.37ms is the paste (~0.15ms), the blur (0.14ms)
         and the pack (0.08ms). The last two are over the frame's rectangle and
         are per frame whatever is cached, exactly as the handoff said; the paste
         is a copy through `Builder::push`, whose per-tile scan is the only thing
         in it that is not linear. In `tests/cost.rs`'s whole-frame reading the
         grid falls from 1.26ms to 0.42ms, against a GPU side of 0.35ms for a
         night frame — so **the CPU half of this pass has stopped being the
         larger one**, which is what decision 30 was written to do.

         Two things a cache has that a rebuild does not, both bounded rather than
         argued about: it lets go of the coldest blocks past `KEEP_BLOCKS`
         (4,096, about seven frames of walking, and never a block this frame
         touched — a cache that thrashes is worse than none), and it is one map's,
         which decision 37 states because nothing here can check it.

      Read decision 30's micro-decisions before starting: 30.5 decides the format
      (WebGL2 — a texture read with `textureLoad`, not a storage buffer), 30.6
      decides how many surfaces a cell may hold (a distribution printed over
      Britain, not a guess), and 30.7 is why none of the walk's rules are
      reopened.

      What comes out on the street at the end is a fan: narrow at the wall,
      widening with distance, with the soft edge decision 14's penumbra already
      gives it.
- [ ] **22. A body's footprint.** *(absorbed into step 23 — decision 38 makes a
      footprint a solid narrower than its tile, so building this first would mean
      writing a flag into the aperture plane in order to delete it two steps
      later. What survives unchanged is **22.1, the measurement**: a derived
      solid still needs the band, and `facing::footprint_of` is where it comes
      from. 22.2's table row is subsumed by the solid verdict of step 23.3.
      22.3–22.5 are gone: the grid, the walk and the view all learn the general
      shape instead.)*

      Decision 34, and it is five changes in the
      order that keeps each one testable alone. Nothing here waits on anything
      outside this list, and every step but the last leaves the picture exactly as
      it is — which is the property that makes the last one readable.

      1. **The measurement.** `facing::footprint_of(image) -> Option<Footprint>`,
         beside `facing_of` and off the same one pass over the pixels: the first
         and last column with a pixel in it, mapped across the tile's diamond and
         quantised to `RUN_STEPS`ths. `None` for a picture that reaches both
         corners, which is every full-width graphic in the install — so the
         measurement can only narrow the grid and never widen it.

         **Two things to get right and both are cheap to state.** The units are
         counted from the **west** corner (`fx - fy = -1`) to the east, because
         that is left to right across the sprite. And the sprite is centred on its
         tile's column, so the tile's own diamond is the middle 44 columns
         whatever the picture's width — a graphic that overhangs is clamped rather
         than refused, since what it covers *of its own tile* is still everything
         on that side.

         **DoD:** a unit test that builds a synthetic silhouette of a stated width
         and reads the band back; a test that a full-width picture measures
         `None`; and a sweep over the install printing how many bodies get a
         footprint at all, which is the number that says whether the rest of this
         step is worth doing. `0x00CC` — columns 12 to 31 of 44 — is the fixture
         the numbers are checked against.
      2. **The table.** `arttable::Shape` gains the footprint, which is a format
         bump (to 3) and a `facing::DETECTOR` bump, for the reason the last one
         was: a table written under the old rules describes yesterday's detector
         exactly and looks perfectly fresh. Authoring comes free with it — a
         person may write a band for a graphic the measurement got wrong, and
         `adopt_authored` already carries it over a re-derivation.

         **DoD:** a round trip through the file, and a stale table refused rather
         than half-read.
      3. **The grid.** `occlusion::Surface` gains it, and it rides in the
         **aperture plane** — `(near, far, ., .)` under a flag of its own beside
         `HOLED`, because that plane already exists per surface and is allocated
         only when something is in it. No texture grows and no texel widens.
         `Builder::add` writes one only for a body, exactly as it drops a hole
         offered to a lid.

         **DoD:** `only_a_body_carries_a_footprint`, and the upload test that a
         footprint lands at its own surface's index — the same failure mode a hole
         had, where a shader reading the wrong index draws something everywhere
         and is wrong only where the thing is.
      4. **The walk.** A body is travelled through, so what changes is the
         *length*: the segment inside the cell is clipped to the strip
         `near <= (fx - fy + 1) / 2 <= far`, which is closed form and exact. The
         side pierce of decision 24 moves with it — the sides that stop a ray
         become the strip's own two boundaries rather than the tile's four edges.
         Both implementations, held by the parity test.

         **DoD:** a scene — a narrow body in the open with a torch beside it —
         where the ground either side of the strip is lit and the strip's own
         shadow is narrower than a tile; the parity test green; and every existing
         scene unmoved, because none of them has a narrow graphic in it.
      5. **The view.** The occluder overlay draws the strip rather than the
         square, which is the step that makes the whole thing visible: a body is
         currently the one kind whose drawn shape and whose behaviour are the same
         wrong answer.

         **DoD:** the tile a person pointed at — Britain's `1509,1635` — reads as
         a narrow violet slab among red panels rather than as a full square.

      **What this does not do**, stated so the next session does not go looking:
      it says nothing about depth (`fx + fy`), which no single picture can
      measure; a footprint is one band, not a polygon; and it is per *graphic*,
      so the same picture is the same band on every tile it stands on.
- [ ] **23. A solid the world owns.** Decision 38, in six changes. 23.0 comes
      first and is not bookkeeping: it is the oracle the rest is judged with.
      23.1 is deliberately invisible, and that is the property being bought.

      0. **[x] The solid, drawn — in the world, not against a sprite.** Decision 39:
         a pass that draws a box as a box, in the frame, where it stands.
         Translucent and over the world, so the static's own sprite is visible
         *inside* the solid that claims to contain it and the top face makes its
         thickness legible.

         **This comes before the migration and not after it, for the reason this
         file gave itself at decision 24: the instrument comes before the
         reproduction.** 23.1's whole DoD is "the picture did not move", and what
         we have to judge that with today is twelve strokes per cell through the
         egui painter (`shell::draw_occluders`) — a wireframe that cannot show a
         face, a normal, or a solid standing inside another. A migration judged
         by an instrument that cannot see the thing being migrated is a migration
         whose defects arrive later, attributed to something else.

         It is also the answer to a question no measurement against a sprite can
         reach. `tests/prism.rs` scores a shape against the picture it was drawn
         from, which is the right check for *is this the shape the artist drew*;
         it says nothing about **how the shapes work together** — a wall meeting a
         wall, a stair meeting a landing, an arch over a street. That is a fact
         about a place and it can only be looked at in one.

         Built against **today's** surfaces (`Occlusion::boxes` already yields
         what stands), so it needs nothing from 23.1 and survives it unchanged:
         after the migration the same six numbers arrive from a solid. The
         translucent-over-the-frame choice is decision 39.2's first answer, and
         per-fragment depth is left until there is a reason.

         **DoD:** the toggle beside the wireframe rather than replacing it — a
         wireframe shows what a solid hides; the staircase at `(1493, 1639)` and
         the house corner at `(1441, 1692)` looked at and *reported on*, which is
         a person's step and not a test's; and a cost reading with the view on at
         the widest zoom, because a translucent overlay over a town is overdraw
         and the number decides whether it stays a debug view or gets a bound.

         **What landed.** Decision 39.6 has the two findings behind the shape of
         it: the geometry is the durable half and stayed in Rust, and the pass is
         a real one because `render`'s pictures are taken headless and an overlay
         drawn by the client's UI toolkit is in none of them.

         Built:
         - `camera::WorldSpot` and `project_exact` — a place between the tiles,
           on the corner lattice (39.7), with `project` delegating to it and a
           test pinning the two over the whole map;
         - `solid::Solid`, `Solid::faces` and `Solid::outline` — the three faces
           in the order `Camera::tile_facet` uses, with a test that a unit
           solid's top *is* its tile's diamond, and the nine lines of the
           silhouette and the star inside it;
         - `occlusion::Surface::solid` — the drawing-only nominal thickness
           (`PANEL_THICKNESS` a fifth of a tile, `LID_THICKNESS` two `z`), which
           step 23.1 must re-decide rather than inherit — and `Surface::stands`,
           which the wireframe used to keep to itself;
         - `solid::standing` and `solid::kind_colour` — the one list and the one
           palette both views draw, so that "what is on screen" has one answer;
         - `render/src/solids.rs` and `solids.wgsl` — the pass, over the lit
           frame, translucent, no depth. In the app it is fed the frame's *own*
           grid (`Lighting::occlusion`, the list the shader is walking) rather
           than a second walk of the map;
         - the toggle: F5, the checkbox beside the wireframe's, and the pass's
           own count of what it drew against what it was handed;
         - `--at X,Y` and `--solids` on the offline viewer, and
           `client_app::Opening` behind them: this plan names places, and until
           now the only way to reach one was to walk there with a shard running;
         - `tests/cost.rs` draws it over Britain at the widest zoom and times
           it — `OPENSHARD_FRAME_SOLIDS=1` beside `OPENSHARD_FRAME_DUMP`.

         **What was seen**, at 1:1 over Britain, in a debug build:
         - **The staircase at `(1493, 1639)` is a stepped mass of whole-tile
           violet bodies** — nine of them, each a full tile of solid from the
           ground to its own step's height, so the shape on screen is a ziggurat
           and not a stair. This is step 23.5's headline defect, and it is now a
           picture rather than an argument.
         - **A wall's thickness reads.** A run of panels is a ribbon with a
           visible top face, and where two runs meet the joint is legible — which
           is the thing twelve strokes could not show.
         - **Ends and corners of wall runs are whole-tile bodies** (violet posts
           standing in red ribbons), not panels. Worth knowing before 23.5
           argues about corners: some of them are already solid by accident.
         - **The tile the staircase descends through carries no solid at all**,
           and the art there is the black opening the client draws for a hole in
           a floor. Correct — nothing stands on it — and worth having seen: it
           is a tile a ray passes through vertically with nothing to stop it,
           which is what a cellar looks like from above.
         - **The house's windows and doors are panes** (cyan) standing in the
           same plane as the wall's panels, and they read as glass in a run of
           brick. Nothing here is a defect; it is the first picture of decision
           3's opacity actually being about a *place*.

         **The cost, at the widest zoom, from `tests/cost.rs`: 3.61 ms a frame**,
         drawing 3,768 of the grid's 16,729 boxes — the rest are off the edge of
         the picture and dropped before a vertex is written. Beside it on the
         same frame: the whole lighting pass is 0.34 ms and a plain blit is 0.18.

         So the number decides what the DoD said it would, and the answer is
         **it stays a debug view**: ten times the pass it is a picture of is fine
         for something switched on to answer a question and not fine for
         anything else. What it is *not* is the shader — the fill is a translucent
         quad over a fifth of the screen — and the honest reading is that most of
         it is the frame's own vertex buffer being rebuilt and uploaded, 3.3 MB
         of it, because the geometry is on the CPU by choice (39.6). If this ever
         has to be cheap, the fix is named by that sentence rather than hunted
         for: keep the buffer between frames and rebuild it when the camera moves.
      1. **[x] The ownership, with the geometry held still.** `occlusion::Surface`
         becomes `Solid` — a box in world coordinates plus the fields it already
         has (`opacity`, the hole flag) — and a cell holds `(offset, count)` into
         an index plane of solid ids rather than into the solids themselves.
         ~~Every solid built in this step is exactly the box its surface was: a
         panel is a slab of nominal thickness on its edge, a lid is a slab of
         nominal height, a body is its tile.~~ The nominal numbers are chosen so
         that **no test moves** — where a plane test and a slab test can differ,
         the slab is the one that must reproduce the plane's answer, and where it
         cannot, the scene that catches it is the finding.

         **DoD:** every scene in `tests/lighting.rs` and `tests/frame.rs`
         unchanged to the byte, the parity test green in both implementations,
         `tests/cost.rs`'s grid assertions re-derived and the new distribution of
         solids-per-cell printed beside 30.6's old one. And a bench reading:
         one indirection in the hot loop is the thing most likely to cost
         something, and it is measured rather than argued.

         **What landed, and the two things it decided.**

         **The slab is not stored, and that is the struck-out sentence above.**
         The step as written wanted a panel to become a slab of nominal thickness
         straight away. It cannot, without the record telling a lie the whole
         plan is built to avoid: what a ray is tested against in this step is
         still a *plane* — the walk's rules are unchanged, which is the entire
         DoD — so a thickness in the box would be geometry sitting in the field a
         reader takes for geometry with nothing testing it. So a solid's box is
         what the walk crosses: a panel's is its plane, flat on one horizontal
         axis; a lid's is the height it lies at; a body's is its tile, which is
         the one kind that was already a box. The thickness a person needs in
         order to *see* a plane edge-on is the view's, and it moved there with
         its fence intact — `solid::DRAWN_PANEL_THICKNESS`,
         `solid::DRAWN_LID_THICKNESS` and `solid::drawn`, whose only caller is a
         picture. Decision 38 withdrew "a wall must stay a plane because a box of
         zero thickness is a numerical coin toss" on the grounds that *with
         authoring* a thickness is a number a person states; nothing has authored
         one yet (that is step 23.3), so zero is the honest entry and 23.5 is
         where a stated one arrives with a ray to test it.

         **The kind is carried, not derived** — the backlog entry that named this
         as the step's real work. Deriving it from the box reads well and is
         wrong on a case the map is full of: a static whose `tiledata` height is
         zero is a **body** with a degenerate span, flat in `z` exactly as a floor
         is, so "flat in `z` is a lid" would silently re-kind it and a lid is
         travelled through by a different rule. `Solid::edges` stays, with the
         argument in its doc, and goes in 23.5 when the rules that ask it go.

         Built:
         - `occlusion::Solid` — the box (`solid::Solid`) plus `opacity`, `edges`,
           the hole and `roof`; `bottom()`/`top()` come off the box, and
           `Solid::box_of` is the one place a kind becomes geometry, so the four
           call sites in `Builder::add` cannot put a panel on the wrong edge one
           at a time;
         - `occlusion::SolidId`, `Occlusion::ids`, `ids_at`, `solid`, `solids_at`
           — the level between a cell and a solid. Today's ids are the identity,
           because nothing is shared yet, and building it anyway is 23.2's own
           argument: a missing reference is a hole in a shadow that looks exactly
           like a detector failing;
         - the upload is four planes now — `bytes` (the index, unchanged to the
           texel), `id_bytes` (new), `solid_bytes` (was `surface_bytes`,
           unchanged to the texel) and `aperture_bytes`. **The box's `x` and `y`
           are deliberately not uploaded**: the walk derives a panel's plane from
           the cell it is stepping through and `edges`, so the two horizontal axes
           have no reader in the shader, and four channels of geometry beside a
           walk that ignores them is how a format grows a field nobody dares
           change. They arrive in 23.5 with a reader;
         - `blit.wgsl`: `solids_at`, `id_at`, `solid_at`, binding 8, and the one
           extra `textureLoad` per solid per cell. `SURFACE_ROW` became `LIST_ROW`
           because three lists are folded by it now;
         - `light::walk_cells` and `panel_stop` read through the same level, so
           the two implementations still mirror each other line for line;
         - the views — `solid::standing`, `shell::draw_occluders`, the plan view
           and `artscan`'s `grid` example — read the owned solid, and
           `Surface::solid` is gone.

         **Green:** `cargo test --workspace`, `clippy --all-targets` and `fmt`
         silent; `tests/lighting.rs`'s 31 scenes and `tests/frame.rs`'s 37 —
         parity included, which is the one that walks both implementations over
         the same scene — unchanged.

         **The distribution, re-measured**, which the backlog asked for and which
         supersedes 30.6's: over Britain at the widest zoom, **10,212 standing
         cells hold 17,201 solids under 17,201 references**, nothing dropped, and
         the tail is short —

         ```
           solids a cell references     cells      share
                              1          6102      59.8%
                              2          2625      25.7%
                              3           773       7.6%
                              4           390       3.8%
                              5           164       1.6%
                              6            80       0.8%
                           7–11           158       1.5%
         ```

         The two totals being **equal** is the fact worth having, not a
         redundancy: it says nothing is shared yet, which is a statement about
         the map's geometry under today's builder rather than about this format,
         and it is the number 38.2's spill will move first. `tests/cost.rs`
         prints both and asserts that references never fall below solids — a
         solid nothing points at is a wall no ray can find.

         30.6's old figure was 18,071, and **the difference is not this step's**:
         `Builder::push` dedups on the same predicate over the same records (a
         cell's solids share a tile, so equal boxes are equal spans and kinds),
         so nothing here can change what is built. The old number was taken at
         step 21.2, before a climbable static stopped being two panels and became
         one body. It should not be quoted again either way.

         **The cost of the indirection: below what this bench can resolve**, and
         the instrument says so itself. Four runs each way at the widest zoom,
         with and without the id fetch — the ids are the identity today, so
         `solid_at(id_at(i))` against `solid_at(i)` is exactly the pass as it was
         before this step:

         ```
           night, ms a frame     0.639  0.793  0.805  0.830   with the fetch
                                 0.435  0.670  0.702  0.725   without it
           dark, the control     0.385 … 0.621                  neither walks a ray
         ```

         The medians differ by about 0.1ms in the direction one would expect, and
         the sets overlap. What settles it is the control: `dark` has no flames,
         so it walks no ray and reads no solid at all, and its own spread over the
         same eight runs is 0.24ms — **wider than the difference being looked
         for**. So the honest reading is a bound rather than a measurement: the
         fetch costs under about a fifth of the pass, on an adapter where the
         whole night pass is 0.8ms against a 16.7ms frame. A number small enough
         to need a better instrument is a number that does not decide anything
         here, and the backlog carries what a better one would take.
      2. **[x] The spill, and the ring's measured radius.** Decision 38.2: a
         `Baked` block gains a spill list, the frame pastes the ring's spill, and
         the ring's width comes from the widest reach in the table rather than
         from a constant. Still no geometry that spills — this is the plumbing
         arriving before its first user, on purpose, because a missing reference
         is a hole in a shadow that looks exactly like a detector failing.

         **DoD:** a synthetic solid, authored to overhang, that occludes
         correctly when its anchor's block is *outside* the frame's block set —
         which is the test that fails if the ring is not pasted, and it wants a
         second case at two blocks of reach, because a ring that is hardcoded to
         one passes the first and fails the second. A radius that follows the
         table rather than a constant; the log line; and a cost reading showing
         that a radius of zero costs a lookup.

         **What landed, and the one thing the DoD could not yet ask for
         honestly.** `Baked::spill` (`occlusion/bake.rs`) and `Solid::footprint`
         (`occlusion.rs`) are decision 38.1 finished for whatever box a solid
         turns out to have, not only the cross-block case 38.2 was written
         about: every tile a solid's box touches besides its own anchor is a
         spill entry, in absolute map coordinates, so `Builder::paste` places it
         with no translation and no case split between "this block was wanted"
         and "this block is only here for its spill" — `Builder::index`'s
         existing clamp is what tells the two apart. `bake::collect_ring` widens
         the block range by a radius and is what `collect` calls with
         `ring_radius(atlas)`; the tests hold the radius themselves and author
         the overhanging solid through `Baked::synthetic`, a `#[cfg(test)]` seam,
         because nothing `Solid::box_of` builds today is wider than one tile —
         23.1 left it that way on purpose and this step does not move it.

         **`ring_radius` is zero, and it earns that answer rather than stating
         it.** The table has nothing to carry a reach in yet — that is 23.3, the
         next step — so there is no per-graphic number to take a `max` over. What
         this step could honestly build is a function that reads the atlas it is
         handed and finds nothing wider than a tile in it, which is what
         `bake::ring_radius` is, and its doc says why in place of the number
         changing later without this comment moving. The alternative — a
         hardcoded `0` with no argument — was rejected as the same invented
         constant decision 38.2 already withdrew once for the ring's width
         itself; a function that ignores what it is handed is not "measured", it
         only reads like it.

         **Green:** `cargo test --workspace`, `clippy --all-targets` and `fmt`
         silent; the two DoD tests above, plus
         `the_measured_radius_is_zero_until_something_authors_a_reach`, which is
         the "cost reading" for `radius: 0` — the function is an atlas read and a
         comparison, not a scan, so the honest reading is that lookup rather
         than a bench number with nothing to measure against yet. `render`
         gained a dependency on `tracing` for the log line, which is inert
         without a subscriber and does not touch the crate's own claim of never
         doing I/O.
      3. **The table carries a solid.** `arttable` gains a third verdict and a
         `FORMAT` bump to 3, with `facing::DETECTOR` bumped for the reason the
         last bump had: a table written under the old rules describes yesterday's
         detector exactly and looks perfectly fresh. Derivation is the prism fit
         that already exists (`tests/prism.rs` scores 0.977 and 0.975 on the
         staircase, against 0.812 for a wall that is not a prism at all), gated
         on `CLIMBABLE` first and the score second. `adopt_authored` carries a
         hand-written solid over a re-derivation, which is already how it works.

         **DoD:** a round trip through the file including a multi-box solid, a
         stale table refused rather than half-read, and — the one that matters —
         a graphic whose solid was measured on a machine with no table reads the
         *same* solid on a machine with one. That is the defect the backlog
         already names: a prism measured by `Shape::of` is lost through the table
         today, and the client quietly goes back to reading a stair as a corner.
      4. **The instrument, which is what makes "by hand" a real mode.** Authoring
         six numbers per graphic is only tractable with a loop: draw the
         candidate solid's silhouette over the real sprite, score the
         intersection over union, show where they disagree, edit the row, look
         again. Half of it exists — `tests/artshot.rs` writes a graphic with the
         tile's diamond stroked over it, `tests/prism.rs` scores a fit — and what
         is missing is the two of them in one run that takes a graphic and a
         table and says: here is what you wrote, here is what the artist drew,
         here is the difference.

         **DoD:** the staircase's two graphics authored through it, and a joint
         and an arch — the two shapes a person reported as "something odd
         happens" — authored and scored. The number to record is how long one
         graphic takes a person, because that is what says whether the mode is
         hundreds of graphics or three.
      5. **And now the picture changes.** Treads as their own boxes, a wall with a
         stated thickness, an arch as more than one solid. Each of these is its
         own reading, taken one at a time against a scene that isolates it, and
         each may be reverted alone — which is the entire reason 23.1 through
         23.4 were built without moving a pixel.

         **DoD:** the staircase at Britain's `(1493, 1639)` lit as horizontal
         treads rather than as two vertical half-walls; `1509,1635` a narrow slab
         among red panels rather than a full square; and a corner where two walls
         meet with no light through the join, which is decision 18's spokes
         closed by geometry rather than by scaling a crossing's length.
- [ ] **17. The shaft.** The screen-space pass of decision 12, over the mask step
      11 produces — and, once step 16 exists, over the beam from a window too.
      Nothing in this renderer draws air, so a visible shaft is a blur of the lit
      mask along the light's direction *on the screen* and nothing else. It only
      makes sense after the patch it grows out of is right.
- [x] **18. The light in the player's own hand.** Decision 15: `light::Beam`,
      one more `vec4` per light in the uniform, `cone` in `blit.wgsl` and
      `Beam::lights` beside it in Rust, and `Lighting::hold` for a flame no walk
      of the map could have found — nothing on the wire says a hand is carrying
      anything, so `light::carried` builds it from the player's tile and facing
      and the app puts it into the frame after the sort. Never the flame dropped
      when a tavern's candles fill the array. F7 in the app, on by default, and
      it does nothing in plain daylight where the whole pass is a copy.

      `scene::lantern_in_a_room` is the fixture and it is a room with **no torch
      in it**: the only flame is the carried one, so every bright pixel is the
      beam's. Held to by three tests — the floor and the wall ahead against the
      floor and the wall behind, the rim's gradient and its width measured at
      four tiles out (`4 * tan(30°)` ≈ 2.3 tiles), and the GPU parity test over
      the same scene, which is the only parity fixture whose cone is not
      identically one.

- [x] **19. The plan view, and the two dumbest scenes there are.** The
      instrument decisions 18 and 19 were found with, and the one this pass did
      not have: `render/src/plan.rs` draws the **real blit** over a synthetic
      place attachment that says every pixel is flat ground on the tile above it,
      one tile to a square of `scale` pixels. The world image is white, so what
      comes out is the multiplier itself — a circle in the world is a circle in
      the picture, a tile is a square, and a wall is a line one can point at.

      It is the same seam `tests/frame.rs`'s parity fixture already used, lifted
      out of the tests so that a person looking at a bug can get a picture
      without writing one. Nothing here computes lighting: a plan view with its
      own arithmetic would be a third implementation of decision 9's one formula.

      `Picture::mark` strokes **the reasons** over it — every occluding cell's
      panel on the side it stands on, coloured from glass to bone by opacity, a
      lid as a dashed square, the tile grid, each flame and the dashed rim of its
      reach. That is the half without which a picture cannot be read: a pool that
      is the wrong shape and a pool that is the right shape behind a wall nobody
      drew are the same picture until the wall is drawn on it.

      And two scenes as dumb as they can be, because every scene this file had
      was a room: `scene::torch_on_open_ground` — one torch, nothing else, so a
      pool that is not a circle here is not a circle anywhere — and
      `scene::torch_before_a_wall`, one straight nine-tile wall two tiles from a
      torch, which is one shadow and nothing else. The spokes were visible in the
      second one at the first attempt.

      `tests/pictures.rs` writes every scene in five views under
      `target/lighting/` (or `OPENSHARD_LIGHT_PICTURES`) and asserts what a shape
      can state: the pool is the same brightness in every direction at every
      distance, it falls off at every step of its inner half, it never brightens
      outwards, and the wall darkens the ground behind it and not the ground
      beside it.

      `View::Flames` came out of the same session and is the sixth view: what the
      flames added, with the ambient subtracted, on black. `View::Light` cannot
      answer "does this pool have a shape" — it draws the ambient underneath and
      bends everything over `KNEE` towards white, so a torch's whole falloff is
      squeezed into the top third of the range and reads as a flat bright blob.
      Take the ambient out and the same pool is a gradient from white to black
      with nothing under it.

      ```sh
      cargo test -p openshard-client-render --test pictures -- --nocapture
      magick target/lighting/one-torch-on-open-ground.flames.plan.ppm /tmp/look.png
      ```

      **And the elevation, which is the other half of it.** A plan's pixels are on
      the ground; a wall's are not, and the two defects decisions 22 and 23 name
      are invisible in a plan for exactly that reason. `plan::elevation` unrolls
      one run of wall: across is how far along the run, down is height, and each
      pixel is written into the attachment as `statics.wgsl` would write it for
      that point of that face — stance included, or the picture would be lit from
      behind. A seam artefact is then a vertical stroke a person can point at, and
      `mark_seams` says where the joins are. The scene it was found in is
      `scene::wall_run_lit_from_along_it`, and the arrangement matters far more
      than the length: a lamp *along* the wall draws the strokes and a lamp in
      front of it draws none.

- [ ] **20. The glow, as its own layer.** Decision 21: the screen-space halo
      around a flame's own sprite, added over the lit frame rather than
      multiplied into it.

      It is a second term in `blit.wgsl` and not a second pass — the lights are
      already in the uniform, and what it needs beside each one is where that
      flame landed **on the screen**, which the CPU knows when it collects them
      and the shader cannot recover from a tile. So: one more `vec4` per light,
      the flame's viewport position and the halo's radius in pixels, and an
      `added` term after the multiply at the end of `fs_main`.

      Three things to decide when it is picked up, and none of them is decided
      here:

      - **Whether the halo is occluded at all.** Cheapest is not: glare is in the
        air and a wall between the eye and a lamp still glares round it. But a
        lamp in a sealed cellar would then glow through the floor above it, which
        is the failure the world layer exists to prevent — so the honest first cut
        is probably to gate the halo on the *world* term at the flame's own tile,
        which the pass has already computed.
      - **Its falloff, which is not the world layer's.** A halo is a glare and
        falls off much faster than a pool of light; reusing `(1 - d)²` would draw
        a second pool over the first and double every complaint about flatness.
      - **Where the sprite is.** The flame's screen position is the sprite's, and
        `light::place` gives a tile. The backlog's "a light is placed by its tile,
        not by its sprite" is a nuisance for the world layer and a blocker for
        this one.

      Off by a key while it is being tuned, like the sun and the sky field, and
      for the same reason: a picture with one thing changed in it is the only
      picture anything can be judged from.

## Backlog

Carried from `client.md`'s firelight backlog and still true:

- `light.mul` / `lightidx.mul` are not read; `light::flame` is the stand-in.
- Nothing a mobile carries burns — a player holding a torch makes no light.
- The ambient is a key (F10), not a clock.
- A light is placed by its tile, not by its sprite.

Found while migrating the ownership (step 23.1):

- **The cost of one fetch in the hot loop is under this bench's noise, and the
  bench says so with its own control.** `tests/cost.rs`'s `dark` case walks no
  ray at all, so it cannot be moved by anything in the walk, and its spread over
  eight runs is 0.24ms — wider than the difference between the pass with the id
  fetch and without it. That is fine for the answer 23.1 needed (a bound), and it
  is not fine for the next question of this shape, which 23.5 will certainly ask.
  What a better instrument wants is not more runs: it is the same frame timed
  with a GPU timestamp query around the blit alone rather than a wall clock round
  a submit, and a case whose *only* difference is the thing under test.
- **`solid::standing` lists a solid once per cell that references it.** 38.2's
  spill has landed and the mechanism is real, but still harmless: nothing built
  today produces a box wider than one tile, so nothing is referenced twice yet. A
  solid overhanging four cells, once 23.5 authors one, will be drawn four times,
  translucent, and read as four weights of colour on one box. The fix is a dedup
  on the id, and the reason it is not written yet is that a view of a *shared*
  solid also wants to say which cells found it, which is a question about what
  the instrument is for.
- **A lid with a span of its own is drawn two `z` deep, not as deep as it is.**
  `solid::drawn` replaces a lid's bottom rather than lowering it to reach, which
  is what step 23.0 drew and is kept to the pixel because 23.1's whole claim is
  that no picture moved. A `FLOOR` static with a height — a sloped roof section is
  one — therefore looks thinner in the view than the span the walk stops light
  over. Worth a picture before it is changed, and it belongs with 23.5's readings
  rather than on its own.
- ~~**The walk's rules are keyed on `Surface::edges`, and a box does not have one
  — which is the real shape of step 23.1's work.**~~ Decided in 23.1: the kind is
  **carried**. The case that settled it is not the abstract one — a static whose
  `tiledata` height is zero is a body with a degenerate span, flat in `z` exactly
  as a floor is, so deriving would re-kind it into a lid and a lid is travelled
  through by a different rule. Written out at `occlusion::Solid`, and it goes in
  23.5 with the rules that ask it.

Found while making the instrument honest:
- **Nothing renders a picture of either view and asserts anything about it.** The
  new tests hold the geometry a view is built from — the plane a panel is drawn
  on, and that the two cuts are a subset and its superset — and `tests/cost.rs`
  draws Britain with the pass on and times it. Between those two there is no test
  that the pass put a box on the screen at all, and `Cut::Nothing` in particular
  has never been drawn by anything but a person. The shape of the answer is the
  one `tests/pictures.rs` already has for the lighting: a small built scene, one
  frame, and a claim about a pixel that a wrong cut or a dropped face would move.

Found while re-cutting the plan around decision 38 (nothing was built):

- **A wall's thickness may be measurable after all, and the number is already in
  the tree.** `facing::OVERHANG`'s own doc says it: *a wall is a solid with a
  thickness, and the picture shows that thickness* — the far side of the face is
  a sliver past the tile's centre column, **3.5 pixels on `0x0100`, 2.5 on
  `0x0007`** — and the conversion is written beside it, `22t` pixels for `t`
  tiles. That is 0.16 of a tile, derived rather than invented, and it means
  decision 3's "the art cannot say how deep a wall is" is too strong: it cannot
  say from the *outline alone*, but this sliver is the depth, projected.
  The confounder is named in the same comment and is real: on a wall low enough
  to look down on, the sliver also contains the **top** surface (8.5 pixels on
  Britain's garden wall), so the measurement is two things added together
  wherever the top is visible. The way to settle it is the instrument, not an
  argument — score a box of thickness `t` against the sprite and take the best
  `t`, exactly as `facing::best_prism` already takes the best prism.
Found while building the treads (step 23.5, in progress and not yet committed):

- **A footprint bug from 23.2 that only the real map catches.** `Solid::footprint`
  floored an `EDGE_EAST`/`EDGE_SOUTH` panel's flat coordinate straight — correct
  for `EDGE_NORTH`/`EDGE_WEST`, whose plane sits at the tile's own low edge, wrong
  for the other two, whose plane sits at the *far* edge (`x + 1`, `y + 1`, an
  integer that floors to the neighbour). `tests/cost.rs`'s oracle
  (`cached == grid`) is the only thing in the tree that reads a wide-enough real
  map to hit it — no synthetic scene stood a panel exactly on a block boundary.
  Fixed by reading `self.edges` in `footprint`'s degenerate branch; see the
  function's own doc in `occlusion.rs` for the two cases. Found while chasing what
  turned out to be an unrelated question (below), which is worth remembering the
  next time a synthetic-scene suite is all green and a real map has not been run
  through the same oracle.
- **Reproducing one real place headlessly, for the next session — done.** No GUI
  is needed — `tests/cost.rs` already opens a headless `wgpu` adapter and can
  dump any of `debug::View`'s pictures with
  `OPENSHARD_FRAME_DUMP`/`OPENSHARD_FRAME_VIEW` (see the test's own doc). What was
  missing was a way to point its camera anywhere but the hardcoded `BRITAIN`
  constant — every look at the staircase run at `(1494..=1497, 1626..=1627)` the
  session before this one took a hand edit of `BRITAIN`'s literal and `widest()`
  → `Zoom::ONE` at every call site, run, then `git checkout -- tests/cost.rs` to
  undo it. `OPENSHARD_FRAME_AT=x,y,z` now does that: `frame_point_and_zoom`
  returns `BRITAIN` at `widest()` when unset, and the named point at `Zoom::ONE`
  — close, since naming a place is for looking closely at it — when it is. The
  one assertion that only holds at the widest rung (`camera.minifies()`) is
  skipped when a place is named; the rest of the test's assertions (a lit frame,
  a standing cell, a changed pixel) still run and still may panic if the named
  place has nothing lit nearby, which is the honest outcome and not a bug in the
  env var.

  ```sh
  OPENSHARD_CLIENT=… OPENSHARD_FRAME_AT=1495,1627,10 \
      OPENSHARD_FRAME_DUMP=/tmp/lit.ppm OPENSHARD_FRAME_VIEW=0 \
      cargo test --release -p openshard-client-render --test cost -- --ignored --nocapture
  ```
  `OPENSHARD_FRAME_VIEW` is the index into `debug::View::ALL` — `0` is `Lit`,
  `4` is `Occluders`, `5` is `Light`.
- **The flame the user means is usually not a map static.** `Solid::footprint`'s
  own staircase (`1849`/`0x0739`) carries no `LIGHT_SOURCE` flag — it is only
  steps. The wall sconces standing right next to it (`0x013A`/`0x013B`) do carry
  the flag but never burn: `light::burns` also requires
  `occlusion::opacity == CLEAR`, and a bracket mounted flush against a wall has
  `NO_SHOOT`, so it reads as wall rather than flame — see `burns`'s own doc for
  why that is the conservative direction and not a bug. What actually lights a
  place like this is usually a **decoration the running shard placed**, which
  lives in `openshard.db`'s `decorations` table (a static-like fixture the
  Community Pack's scripts put down) or `items` (`loc_kind = 0`, something
  dropped), never in the client's own `.mul`/`.uop` — so `map.statics_at` cannot
  see it and neither can a raw-file-only reproduction. Pull it straight from the
  live DB rather than guessing:
  ```sh
  sqlite3 openshard.db "select data from decorations" | python3 -c '
  import sys, json
  for line in sys.stdin:
      d = json.loads(line)
      if d["facet"] == 0 and abs(d["x"] - 1498) <= 2 and abs(d["y"] - 1626) <= 2:
          print(d)'
  ```
  and feed the one result in as a [`crate::items::GroundItem`] — `at`, `graphic`,
  `hue`, nothing else — passed as `extra_items` everywhere `tests/cost.rs` passes
  `&[]` today (three call sites: `light::collect`, `occlusion::collect`,
  `occlusion::bake::collect`). Keep the list to the one lamp the question is
  about; the DB holds hundreds of decorations in the same block and every one
  not in reach of the tile in question is noise in the picture and nothing
  more — pulling the *whole* nearby set once (all 217 within 45 tiles, this
  session) is worth doing exactly once, to confirm nothing closer was missed,
  and then thrown away in favour of the one that mattered.
- **Two debug views that looked like the right instrument and were not.**
  `View::Height` draws the *drawn sprite's* own per-pixel world height (the
  `place` attachment `statics.wgsl` writes) — a different mechanism entirely from
  `occlusion::Solid`, so a stair's art reading as one smooth ramp there says
  nothing about whether its occlusion is one box or three. `View::Occluders`
  (`blit.wgsl`'s `merged_at`) reads the tile's *merged* span — the union of every
  solid on it — which by construction cannot distinguish one whole-tile body from
  three tread-strips whose union is the same envelope. Neither view answered "did
  the tread split actually happen"; only `Occlusion::solids_at(x, y)`, read
  directly in Rust, did — see the recipe above, minus the `OPENSHARD_FRAME_*`
  vars, plus a loop over `grid.solids_at(tx, ty)`.
- **What that direct read confirmed, and what is still open.** `tread_box_of`
  does what it was built to: tile `(1495, 1627)`'s three solids are three `y`
  strips (`10..=11`, `10..=13`, `10..=15` in `z`, each a third of the tile along
  the climb), the low one nearest south and the high one nearest the `up: North`
  the table measured. The user's own screenshot of `View::Light` over this run
  shows a fine sawtooth along the whole flight where a coarser one stood before —
  eight tiles × up to three treads is more edges than eight tiles × one box, and
  that is the geometry working as intended rather than a defect. **What is not
  settled**: whether that finer edge wants a blur radius wider than a third of a
  tile so it reads as a staircase and not static, which is a rendering-quality
  question for the next session and not a correctness one — `tests/cost.rs`'s
  oracle is green on the real map with the footprint fix in, and the geometry
  itself is confirmed by direct read rather than by eye.
- **The user's actual complaint, tracked down — and a wrong first read of it
  worth leaving in, since it names a trap.** Not the sun (tried first, and
  wrong — see above), and not the distant flames already in the tree. The real
  lamp is a decoration at exactly `(1498, 1626, 10)` — see the DB-lookup bullet
  above — sitting almost on top of the *corner* of the nearest tread box.
  **First attempt, and wrong:** a hand-rolled ray march (180 rays, stepped
  `0.02` tile through `Occlusion::solids_at`, `opacity > 128` treated as a flat
  wall) showed a razor-sharp red/green boundary right at that corner and
  concluded the pass has no penumbra at all — a boolean test, one sample, done.
  That conclusion is **wrong**, and the tell was in the tool: that ray march is
  a diagnostic stand-in, not the pass. The real one is `light::walk_cells`
  (`light.rs`), and it is deliberately *not* boolean — it spends the length of
  the ray inside each occluding cell, softened by [`SOFT_CROSSING_MIN`/`_MAX`]
  and the flame's own size (`FLAME_SPREAD`), exactly so a corner clip is dimmed
  rather than switched. Sampling the real formula
  (`light::sample(Spot::at(...), &lighting)`, `Surface::Upright` so the facing
  term below does not confound it) across the same corner, a hundredth of a
  tile at a time, gives a genuinely smooth ramp — `through` climbs `0.0 → 1.0`
  over about a third of a tile, and `brightness` with it, `0.36` to `0.85`
  continuously, no step anywhere in the trace. **The lesson: a hand-rolled
  stand-in for a shadow test answers a question about the stand-in, not the
  shader — sample the production function (`light::sample`) directly, the way
  `docs/lighting.md` decision 9's parity test already insists on for the GPU
  side, rather than re-deriving the walk by hand.**
  What *does* still cut sharply, on the very same tile: [`Surface::Flat`]'s
  `faces()` term, which multiplies in whether the surface looks toward the
  flame at all. A tread's flat top looks straight up; a flame sitting well
  below it (this lamp lifts `FLAME_LIFT` above its base, half a tile — a modest
  climb) reads as behind the plane, and `faces()` clamps that off over
  [`FACE_EDGE`] — `0.2` tiles, about `2.2` `z` units [`Z_PER_TILE`]-scaled — a
  window far narrower than the several `z` units of climb between one tread and
  the next. So a flat tread-top several steps above a low lamp goes dark not
  because a box stood in the ray's way but because the surface itself is
  turned away from the light, and that cutoff is steep enough across a single
  tread's height to read as a hard line even though `faces()`, like `through`,
  is a clamped ramp and not a step. **Not settled this session**: which of the
  two — the occlusion ramp (confirmed soft, a third of a tile wide) or the
  facing cutoff (confirmed steep, not yet measured against the actual riser
  geometry the art draws) — is the one a person's eye is catching in the
  screenshot. Answering that needs `Spot::face(...)` sampled along the tread's
  actual visible riser, not `Spot::at(...)` on an assumed flat top; left for
  the next session that has the same lamp handy. The sampling code was
  disposable, in a temporary `tests/cost.rs` edit, thrown away with
  `git checkout` same as the recipe above.
- **A reusable tool for exactly this, so the next session's sampling code does
  not have to be disposable either.** `examples/isolated_scene.rs` draws a
  **synthetic** map (`Map::from_blocks`, which never carries statics) and puts
  back only what is asked for, all through environment variables: the real
  map's statics within a stated radius of a stated point (optionally filtered
  to a list of tile IDs), the real ground under them or none at all, and any
  hand-named extra item — a live-shard decoration such as this lamp, in the
  shape the DB-lookup recipe above already produces. Turn every knob down and
  what is left is one tile:
  ```sh
  OPENSHARD_CLIENT=… \
      OPENSHARD_SCENE_AT=1497,1626,10 OPENSHARD_SCENE_TILES=0x0739,0x0738 \
      OPENSHARD_SCENE_GROUND=0 OPENSHARD_SCENE_EXTRA=1498,1626,10,2852 \
      OPENSHARD_FRAME_DUMP=/tmp/corner.ppm OPENSHARD_FRAME_VIEW=5 \
      cargo run --release -p openshard-client-render --example isolated_scene
  ```
  See the file's own doc for every knob. It does not answer the open question
  above by itself — `Spot::face(...)` sampled along the riser still has to be
  written — but the scene it draws is now one command instead of a hand edit,
  and the tread and the lamp it draws are exactly the ones this question is
  about, with nothing else in the picture to confound a reading.
- **The open question, answered — `faces()`, not the occlusion walk.**
  `examples/isolated_scene.rs` now has a profile mode
  (`OPENSHARD_SCENE_PROFILE_FACE=north|east|south|west|flat|upright` plus
  `_FROM`/`_TO`/`_STEPS`/`_LIGHT`): instead of drawing a frame it walks
  `light::sample` along a segment and prints each [`light::Reach`]'s `through`
  and `cone` — the same two numbers `docs/lighting.md`'s corner investigation
  already leaned on, read straight off the production function rather than
  re-derived. It also prints `Occlusion::solids_at` for `_AT`'s own tile, so a
  segment does not have to be guessed at from a picture — this session's stair
  (`1497,1626,10`, filtered to `0x0739`/`0x0738`) came out as one lid
  (`z 10..15`, whatever sits under the flight) and three tread strips split
  along `y` with heights `1, 3, 5` (`z 15..16`, `15..18`, `15..20`), the low
  one nearest south — `up: North`, confirming the reading `tread_box_of`'s own
  test already carries.

  Sampling `Surface::Face(South)` and `Surface::Face(North)` on the two
  riser planes between those strips came back `cone: 0.000` everywhere along
  their full height. Not a bug: `place()` puts a flame at its tile's *centre*
  (`+0.5`, easy to forget doing this by hand — the first attempt at this did),
  and this lamp's centre (`1498.5, 1626.5`) sits almost exactly on this stair
  tile's own east edge — a full tile east of the risers' `x` and inside the
  middle tread's own `y` span, so both risers' normals (`[0, ±1]`) are close
  to perpendicular to the lamp and `faces()` clamps to zero everywhere on
  them. The risers cannot be the hard edge here; there is no ramp on them to
  be hard, they are simply always dark, which is correct — a step's riser
  facing away from the only light nearby *should* be unlit.

  Sampling `Surface::Flat` instead — walked across the three tread **tops**
  from the low, south one to the high, north one (`(1497.5, 1626.83, 16)` to
  `(1497.5, 1626.17, 20)`, 20 steps) — is where the cutoff actually is:
  `cone` falls from `0.273` to `0.000` in the first three samples, a climb of
  about `0.6` `z` units, and stays at `0.000` for the remaining seventeen —
  while `through` is still climbing smoothly past it, reaching `1.000` around
  the fifth sample and staying there until the next tread's own occlusion
  box interrupts it. So the two hypotheses this session set out to tell apart
  are told apart: **the facing cutoff is the hard edge, not the occlusion
  ramp** — `through` never stops being the smooth, roughly-a-third-of-a-tile
  ramp the corner investigation already measured, but `faces()` gates the
  lamp off within the first tenth of a tread's climb and holds it at zero for
  the rest of the flight, which reads as one lit step and then a flat, matte
  run of tread tops above it — the report's "hard line", now with a name and
  a place in the code (`FACE_EDGE`, `light.rs`).

  **Settled by decision 40, not by widening `FACE_EDGE`.** Widening the
  constant for `Surface::Flat` specifically was considered and dropped: it is a
  tuning fix for one shape and would make an ordinary floor a full storey below
  a lamp bleed light through the widened band it does not have today.
  `faces()` itself was never the problem — `along = normal · toward` is
  already a genuine physical distance in tiles from the surface's own plane to
  the flame (`toward` arrives unnormalised, `light.rs`'s `offset`, built in
  `sample`), so `FACE_EDGE` stays the one number it is, "how far off the plane
  before you count as behind it," for *any* unit normal. What was narrow is
  `Surface`: it could only *hold* three normals — none, straight up, or one of
  the four cardinal horizontals — because every panel and lid built before this
  step really did look one of those three ways. A tread top is the first that
  does not: it is the top face of a box, honestly horizontal, but the shape it
  is one step of is a ramp, and reads to a light standing beside the flight as
  something other than a floor.

  Decision 40 is the fix this argues for, generalised past this one stair:
  `Surface` gains a normal computed from a shape's own geometry instead of a
  fixed tag, the box stays a box on the tile grid (decision 36), and `faces()`
  does not change at all. This is where it is proven first — see the handoff
  that starts the next session on exactly this tread.

Found while building the spill (step 23.2):

- **`bake::collect_ring`'s widened range still bakes and caches an empty block
  for every ring tile past the facet's own edge.** `Baked::of` answers correctly
  — `Map::statics_in_block` is empty out of range — but a frame in a facet's
  corner, once a real reach exists, pays a `Bake` cache entry for blocks that can
  never hold anything. Not a bug at `radius: 0`, where the widened range is the
  core range; worth a clamp against `map.width()`/`map.height()` in the same
  change that gives `ring_radius` a real number to return.
- **`ring_radius` has nothing to read yet, and it shows in the test:**
  `the_measured_radius_is_zero_until_something_authors_a_reach` asserts a
  constant against an atlas that cannot hold anything else. That test earns its
  place once 23.3 gives a graphic a reach to author — until then it is a
  regression guard against the function being *replaced* by a literal, not a
  measurement of anything. Said so in the step's own "What landed" note rather
  than left implicit.

Found while drawing the solid (step 23.0):

- ~~**Both views filter by `Surface::stands`, so a house's floor is invisible
  from its own floor.**~~ Closed by decision 39.8: `solid::Cut` is the second
  datum, F4 flips it, and it governs both views at once. Two values and not
  three — "this storey" needs a ceiling and therefore a rule for which lid is
  yours, and that rule is not inventable here.
- **The solids pass rebuilds its whole vertex buffer every frame.** 3.3 MB at the
  widest zoom, and the 3.61 ms reading above is mostly that rather than the
  fragments. The geometry is on the CPU deliberately (39.6) and the fix, if one
  is ever wanted, is the same one the occlusion grid already took in step 21.5:
  keep the buffer and rebuild it when the camera moves. Not worth doing for a
  view that is off by default; worth knowing before anybody concludes the
  translucent fill is expensive.
- ~~**Nothing tests that the solids view and the walk agree.**~~ Closed:
  `a_panel_is_drawn_on_the_plane_its_face_pixels_lie_on` derives the plane from
  `Face::place_at` — what `statics.wgsl` places a face fragment with — and
  asserts the box has a face on it, lies *inside* its tile rather than
  straddling the edge, spans the whole run, and carries the span the walk tests.
  Its companion does the lid (top face on its plane, hanging under it) and the
  body (its whole tile). What is still untested is a *thickness*, and that is
  correct: it is a drawing number until step 23.1 makes it geometry.
- ~~**The nominal thicknesses are drawing numbers with no owner.**~~ Closed by
  renaming them `DRAWN_PANEL_THICKNESS` and `DRAWN_LID_THICKNESS`, with the
  fence stated in the doc comment: no ray is tested against either, the only
  reader is `Surface::solid`, and 23.1's thickness is a different number reached
  by a different argument. The collision the entry warned about — two constants
  with one name, one drawn and one tested — is now impossible to make silently.
- **`Camera::project` is a matrix, and writing it as one would change no pixel.**
  It is an orthographic view-projection with a fixed rotation, spelled as integer
  arithmetic. `docs/camera.md` is the plan that wants this seam — "one pipeline
  every camera is a parameter set of" — and decision 39 is the same fact arriving
  from the other side. Nothing here needs it; it is written down because the two
  plans should not discover it separately.

- **The renderer's own doc line about WebGL2 reads as a principle and is a dated
  assumption.** `crates/client/render/src/lib.rs:17` says the ceiling is WebGL2
  "because the web is a target", written when WebGPU was behind a flag. The
  ceiling was re-examined and kept — decision 30.5 has the measurement — but the
  sentence should say what it is: a floor chosen for a target, with a date on the
  reasoning. The question underneath it is a product question and not a graphics
  one, and it wants a person: *is the web still a target?*
- **Decision 22's one-sidedness and `place::Stance`'s nine values are both
  taxonomies that a solid derives.** Once a pixel's face comes from a slab test
  (38.3), a stance is an answer rather than an enum to extend, and "a face is
  one-sided" is a consequence of where the artist put pixels. Neither is worth
  touching before step 23.5, but both should be *removed* there rather than
  carried alongside the thing that replaces them — two ways to answer the same
  question is how a rule and its replacement drift apart.
- ~~**30.6's distribution does not survive the migration and must be
  re-measured.**~~ Re-measured under step 23.1: **10,212 cells hold 17,201 solids
  under 17,201 references**, nothing dropped, 59.8% of standing cells holding one.
  The old 18,071 is not comparable and is not to be quoted; the table and what
  moved it are with the step. The *per-cell cap* is still the format's own 255 and
  still nowhere near reached — the worst cell in Britain references eleven.
- **`Shape::of`'s prism is still lost through the table.** Carried up from the
  staircase entry below because step 23.3 is now where it is fixed: a client with
  a table reads a stair as a corner where a client without one measures a solid,
  which is a table making things *worse* and the exact failure mode decision
  31.6 was written to avoid.

Found while baking it (step 21.5):

- **The paste is now the largest single thing in the build, and it is a linear
  scan.** Of the 0.37ms a cached frame spends, roughly 0.15ms is
  `Builder::paste` — pushing 17,201 surfaces into the frame's arena through
  `Builder::push`, which walks the tile's existing list on every one of them to
  drop an exact repeat. Pasting a *baked* block cannot produce a repeat: the block
  was deduplicated when it was built and no two blocks share a tile. So the scan
  is provably dead work on this path, and the shape of the fix is a `push` that
  does not dedup, used only by `paste`. It was left undone because "provably" is
  an argument and not a test, and the test that would hold it — a paste that
  silently doubled a tile's surfaces — wants naming before the code is written.
- **A frame indoors is not measured.** Every number in this file is off
  `Cutaway::OPEN`. Decision 33 moved the cut to `finish`, so a frame inside a
  house now *builds* the surfaces it is about to drop — which was the cost the
  bake was meant to pay back, and the bake does pay it back, but nobody has put
  the two side by side. It is one more call in `what_the_grid_costs_to_build`
  with a cutaway that cuts, and it is worth having because it is the case where a
  cached frame and an uncached one differ most.
- **`Occlusion::dropped` is double-counted at a block boundary, harmlessly.**
  `paste` adds a block's whole `dropped` count whether or not the tile that
  overflowed is inside the frame's rectangle. The number is a diagnostic about
  the map and the worst tile in Britain holds 21 of a cap of 255, so it is not
  reachable today; it is written down because the two implementations of the grid
  are otherwise equal *to the byte*, and this is the one field where "equal" rests
  on the cap never being hit rather than on the arithmetic.

Found while building it:

- **The per-tile cap and `dropped` now count the map, not the frame.** Decision
  33 puts every solid into the builder, so `MAX_SOLIDS_PER_CELL` (named
  `MAX_SURFACES_PER_TILE` when this was written) is reached by solids a frame was
  about to cut away — a tile could in principle drop a
  *drawn* one because undrawn ones filled it, and `Occlusion::dropped` counts
  what the map has rather than what the picture lost. Nothing is at risk today:
  the worst tile in Britain is 11 of 255 (step 23.1's distribution), and the
  distribution was measured under `Cutaway::OPEN`, which is the same set the
  builder now holds. It becomes a real question only if the cap is ever lowered
  to fit a format, and the honest fix then is to cut before the cap rather than
  after — which the builder cannot do, because it does not know the frame.
- **The land itself does not occlude.** A hill between a campfire and a valley
  stops nothing: only statics are in the grid. The map has the four corner
  heights for every tile and the grid already carries a span, so the shape of the
  fix is "add the land's own height as an occluder with `opacity` scaled by how
  far the ray is under it" — the reason it is not done is that a hillside that
  cast hard shadows would look worse than one that casts none until the falloff
  and the span are tuned against a real scene.
- **Nothing a mobile is standing in casts a shadow.** A body between a torch and
  a wall lights the wall as if it were not there. The reference does not shadow
  mobiles either, so this is a note rather than a defect.
- ~~**The ray is Chebyshev-sampled, one cell a step.**~~ Closed by decision 18:
  where the two boundaries land together the walk asks both cells that share the
  corner and then steps diagonally past them, which is the supercover answer paid
  for only on the rays that hit a corner exactly.
- **`Occlusion` is rebuilt and reallocated every frame.** 140KB at the widest
  zoom, and the texture upload beside it. Both want the buffer kept between
  frames — the rectangle only changes size on a zoom step or a resize.

Found while building the observability and the sun:

- **A room with no roof is a courtyard, and the sun is right to flood it.** The
  first sunlit scene had four walls and open sky, and at 45° the sun clears a
  two-tile wall in two tiles — so the floor was fully lit and the window proved
  nothing. `scene::sunlit_room_with_window` has a roof for exactly this reason,
  and it is worth remembering when a real house looks wrong: ask what the
  cutaway did to its roof before asking what the sun did.
- **The sun has no facing either.** A wall's two faces are one tile, so both are
  lit when either is — the same hole decision 3 leaves for a sconce, arriving
  from the other direction. It is more visible with a sun than with a torch,
  because every wall in the frame has a shaded side that is not shaded.
- **`occlusion::PANE` is a guess.** A fifth stopped, from nothing: the client has
  no number for how much light glass passes. It is the one value in the pass
  invented rather than read, along with `light::flame` and `light::midday`.
- **The diagram does not draw the sun.** `debug::diagram` marks flames and
  occluders and samples brightness, so a sunlit scene reads as a field of `+`
  with darker tiles in the shadows — legible, but there is no `☀` and no arrow
  saying which way the light comes from.
- **The sun's ray is walked for every ground pixel.** Firelight's cost is paid
  only inside a pool; this one is paid everywhere the sky is visible. The ceiling
  test (`Occlusion::tallest`) makes it two or three steps over open ground —
  ~~and the number is still unmeasured~~, and step 6 has now measured it at
  0.021ms a frame, which is a tenth of what firelight costs on the same frame.
  ~~The cost was never the reason to leave F8 off; the tile-stepped ray below
  is.~~ Both are answered: the walk is a walk now and costs 0.057ms of the
  0.287ms pass.

Found while drawing the boxes:

- **A pier is two thousand occluders, and they are floors.** The first frame of
  the wireframe was Britain's docks, and the grid held **2011 cells** — one thin
  slab on every plank, because a floor is exactly what you cannot shoot *through*
  to the storey above and the membership test is the shooting flags. It is not
  wrong; it is what makes the picture unreadable, and it is why the view draws
  only what stands above the floor the player is on. What it raises and does not
  answer: a fragment standing on that deck is *inside* one of those cells, and
  the walk exempts the light's own tile (decision 3) but not the fragment's — so
  whether a floor dims the light falling on the thing standing on it is an open
  question with a scene-shaped answer. Nothing in the frame looks wrong today,
  which is exactly why it is written down rather than assumed.

- **The overlay walks the map a second time in the same frame.** The HUD is built
  before the world passes and the frame's `Lighting` after them, so the wireframe
  cannot read the grid the shader is about to be given — it builds its own from
  the same bounds, the same cutaway and the same map, which is the same answer
  and twice the walk. Sharing them means either building the lighting before the
  HUD or keeping the last frame's grid, and the second is what draws a wireframe
  a frame behind the picture it is a claim about. Worth doing when the frame
  keeps its `Lighting` for another reason; not worth it for this alone.
- **Nothing tests the projection of a box.** `Occlusion::boxes` is pinned — the
  row-major arithmetic is the half that fails silently and it has a test — but
  the eight corners and the twelve edges are held by looking at them. A frame
  test would need an egui context and a painter offscreen, which nothing in this
  workspace does yet; the shape of it, if it is ever wanted, is that a box's lid
  is exactly `(top - bottom) * Z_STEP` viewport pixels above its floor.
- **The wireframe shows what stands and not what is missing.** The same point
  `lighting_world.md`'s backlog makes about the sky field, arriving here: a roof
  that is one tile over from where it should be draws a box, correctly, one tile
  over — and the tile it *should* have covered draws nothing, which looks exactly
  like open ground. `View::Sky` on the ground is the instrument for that half,
  and the two want reading together rather than one replacing the other.

Found while giving a wall a side to be lit from:

- **The sun still has no facing.** Decision 22 gives a flame one and the sunbeam
  does not ask: `sunlight` walks the grid and never looks at the normal, so every
  wall in a daylit frame is still lit on the side turned away from the sun. It is
  the same two lines and the same `outward`, and it is left because the sun is off
  by default and every scene that would judge it is a firelit one.
- **The facing is binary where a real surface is a cosine.** What a wall gets is
  `1` in front and `0` behind with a fifth of a tile of gradient between, and a
  real surface lit obliquely gets less than one lit head-on. Lambert would be one
  `dot` more — but UO's art is pre-shaded, so a wall's picture *already* has a
  light in it, and multiplying by a second one is a decision that wants a scene
  rather than a formula.
- **A mobile has no facing either, and it has one on the wire.** A body is drawn
  as a billboard and lit as `Stance::Upright`, so a character walking through a
  pool is lit identically front and back. The direction is already parsed —
  `light::carried` uses it for the beam — so what is missing is the will to decide
  whether a paper-doll sprite should be shaded at all.
- **Three bits of the height channel are spent and five are left.** The stance
  took the first three of the eight a `z + 128` leaves free in a `u16`. Worth
  remembering before the next thing wants a channel: step 16's aperture is asking
  for one.

Found while starting again, from the picture rather than from the argument:

- ~~**A lamp mounted on a wall wants pushing off it, not exempting from it.**~~
  Done, decision 26, and the entry's own guess is what was built: outside the
  panel, on the side the picture is drawn from. What it did *not* predict is that
  the same move would let the facing test drop its line exemption, which is what
  the reported defect actually was. The **shadow** walk still exempts a flame's
  own cell (decision 17's amendment) — it is just that a mounted flame's own cell
  is now the street rather than the wall, so the exemption stopped mattering
  where it did harm.
- **The flame is still placed by its tile and not by its sprite.** With the pools
  legible again this is the next visible thing: a torch in a wall sconce burns at
  the tile's centre and half a tile up, and the flame in the picture is neither.
- **`View::Flames` is the view a shape is judged in, and nothing says so in the
  client.** F11 cycles eleven views now and their names go to a log line. A person
  who does not already know which one answers their question has to walk all
  eleven.
- **The plan view is not in the client.** `render/src/plan.rs` needs a device and
  a queue and draws into its own texture, so a test can call it and the app does
  not. What the app would want is a key that dumps the current frame's `Lighting`
  as a plan beside the screenshot — the same instrument pointed at the world the
  player is standing in rather than at a built room.
- **A scene has no art, so almost every scene tests the whole-tile occluder.**
  Two scenes hand `facing::silhouette` to the grid and the rest get `EDGE_ANY`,
  which after decision 18 is a *different code path* — the body, not the panel.
  The suite is therefore thin exactly where the change was: `torch_before_a_wall`
  is the only picture of a named panel's shadow, and the doorway and room scenes
  all measure the body. Giving every wall scene a silhouette would double the
  coverage for one line each, and it would also change what those tests assert,
  which is why it is written down rather than done.

Found while asking why a house's windows burn:

- ~~**Eighty window graphics are flagged `LIGHT_SOURCE`, and every one of them is
  given a torch.**~~ Answered by decision 19, with the third of the three answers
  this entry offered: a light source that stops light is not a flame. Britain's
  64 flames at the widest zoom are 7. The two the entry names and this does not
  do are still there to be done — `light.mul`'s shape by the static's `layer`
  byte, and the reference's rule about something opaque standing over `(x+1,
  y+1)`.

- **Eighty window graphics are flagged `LIGHT_SOURCE` (as written).** Kept
  because the scan is the evidence and the two unfixed answers are in it.
  Scanned over the client's `tiledata.mul`: 615 statics carry
  the flag, and 80 of the 163 named "window" are among them — `0x0103`,
  `0x2BBF`, the shutters at `0x2501`, the windowed walls at `0x2B7D`. `light::flame`
  answers `TORCH` for every graphic it has no name for, so a street of houses is a
  street of six-tile warm pools with nothing burning in them. Two things the
  reference does that this does not, and either one alone fixes it: the light's
  *shape* comes from `light.mul` indexed by the static's `layer` byte
  (ClassicUO `GameScene.AddLight`, `Game/Scenes/GameScene.cs:508` — `light.ID =
  data.Layer`, and `StaticTile::layer` is already parsed here), and a light is
  dropped entirely when something opaque stands over the tile at `(x+1, y+1)`
  above `z + 5` (`GameScene.cs:415`). The third answer, and the one worth
  arguing for: **a window is not an emitter at all.** It is a pane, it is already
  in the occlusion grid, and it should glow because a candle behind it does —
  which is what decision 4's fraction was for.
- **A windowed wall passes four fifths of the light.** The same scan: those
  graphics are `WALL | BLOCK | WINDOW`, and `occlusion::opacity` reads `WINDOW`
  before anything else, so a whole wall tile whose art has a window in it stops
  `PANE` rather than `OPAQUE`. The older ones do not carry `NO_SHOOT` either, so
  nothing rescues them. The pane is the hole in the wall, not the wall.

Found while asking why the light steps from tile to tile — decisions 13 and 14
are what came of the first three, and these are what is left:

- ~~**The sun's ray still steps a whole tile at a time.**~~ Done: there is one
  walk now, and both rays take it. `blit.wgsl`'s `walk` and `light::walk_cells`
  take the two ends as parameters — `skip_last` is the flame's own tile, `spread`
  is how big the source is — and a sunbeam passes `false` and `0.0`. What the sun
  has instead of a position is a direction, so the far end is *computed*: the
  point at which the ray leaves the grid's ceiling, past which it is looking at
  sky. It costs 0.057ms a frame at the widest zoom over Britain, against the
  0.021ms the point sampling cost, and the pass as a whole is 0.287ms — a quarter
  over an unlit frame, for a walk that runs on every ground pixel there is.

  The sun's ray also gains the panel test it never had (below), because a
  crossing is something only a walk can name.

  **And it was not a softness question, it was a hole.** Measured in the sun view
  over `scene::roofed_room`, which is a shut house with a roof on: every tile of
  the interior reads `0`, *except* the column one tile in from the sunward wall,
  which reads a full `255`. At noon the ray climbs 11 `z` a tile, so from that
  column it is at `z = 16.5` where it crosses the wall's plane — inside a span of
  `0..=20`, and stopped — and at `z = 22` at the next tile's centre, which is the
  only place the walk looks. It steps straight over the top of the wall. In
  `scene::sunlit_room_with_window` the same column is `255` along the whole wall
  while the floor tile actually at the window is `204`, the pane's four fifths: the
  bright band inside a windowed room is one tile off the window, brighter than the
  window's own patch, and runs the length of the wall rather than the width of the
  opening. Reported from the client as "the light from the windows looks
  inverted", which is exactly what that reads as.
- ~~**The sun's walk does not test the panel a wall stands on.**~~ Done, with the
  entry above and by it: a point sample has no crossing to name, so the edge test
  could not be lifted across until the sun's ray was a walk. It is one now, and
  decision 17 applies to it unchanged.
- ~~**A wall is still lit as one point per tile.**~~ Done, in step 15, for the
  three quarters of a city whose art names an edge. What is left of it is the
  other quarter — corners, posts and slabs — which still light as a row of tiles
  each at its own brightness, with only the vertical gradient the height gives.
- **The penumbra is a width, not an area light.** Decision 14's `t / (1 - t)` is
  the right *shape* off one ray, but it softens by how far the ray ran inside the
  cell rather than by how much of the flame the cell hides. Where an opening is a
  tile wide and the ground is right behind it — a doorway — the honest answer is
  still nearly a hard edge, which is what a point light through a one-tile
  aperture is. Several jittered rays to points on a sphere of the flame's size
  would be the real thing, at that many times the walk.
- **`FLAME_SPREAD` and its two bounds are invented**, like `occlusion::PANE` and
  `light::flame`. What holds them is a scene, not a file.
- **An upright sprite's fraction is clamped at its tile's edge.** A tree is a
  hundred pixels across and the attachment holds one tile per pixel, so the
  outermost columns of a wide sprite all claim the edge of the tile the thing
  stands on. It is the honest answer available — a billboard's pixels are not
  anywhere in particular — but it means a very wide sprite's lighting flattens
  towards its edges.

Found while putting a light in the player's hand:

- **Only the player carries one.** `light::carried` is built in the app from
  `self.player`, so a second character walking past with a torch makes no light
  at all — and the crowd's mobiles have a facing and a tile, which is everything
  the constructor needs. What is missing is the *reason to believe it*: nothing
  says a given body is holding anything, and giving every mobile on screen a beam
  would light a market square from sixty invented torches.
- **Nothing on the wire says a hand is holding a torch.** The equipment layers
  are parsed here already (`0x2E` and the paperdoll's items), and a torch in
  `Layer::OneHanded` is exactly the fact this pass is guessing at. Until it is
  read, `App::lantern` is a key that defaults to on — which is a client that
  lights the dark rather than a client that is right.
- **`HELD_BEAM_DEGREES`, `BEAM_EDGE` and `BEAM_SPILL` are invented**, joining
  `occlusion::PANE`, `FLAME_SPREAD` and `light::flame`. That is now six numbers in
  this pass that no client file has, and the honest way to hold them is one scene
  each rather than an argument each — which is what `scene::lantern_in_a_room`
  does for the last three.
- **The beam does not move with the sprite's own arm.** A carried flame is at the
  middle of the player's tile, half a tile up, whatever the body's animation is
  doing — so at the instant a step lands, the pool jumps a whole tile while the
  drawn body slides. It is the same "a light is placed by its tile, not by its
  sprite" the backlog above already carries, arriving where it is most visible,
  because this is the one light that moves every frame.
- **A dark tile now has three causes and the diagram shows one.**
  `light::Reach` grew a `cone`, and `Sample`'s report prints it — but
  `debug::diagram` still draws brightness alone, so "behind the character" and
  "behind a wall" are the same blank cell in the picture. The shadow view
  (`View::Shadow`) has the same hole from the other end: it draws what the walk
  lost and knows nothing about where the light was pointed.

Found while measuring it — step 6's numbers are above, and these are what they
raise:

- **The loop has no way out, and the misses are the pass.** Decision 6 said a
  fragment outside every radius leaves the loop at once. It does not: `blit.wgsl`
  `continue`s to the next light, so every fragment of the screen runs 64
  iterations at night whatever is on it, and those iterations are 0.135ms of the
  0.215ms lighting costs. What a miss skips is `reaches` — the ray walk — which
  is why the *lit* eighth of the screen adds only 0.025ms on top. The shape of a
  fix is a bound the whole loop can be skipped against: the lights are already
  sorted by distance from the eye, so a per-frame screen rectangle for the union
  of the pools, or a coarse per-tile light list, would let most fragments do one
  test instead of sixty-four. Worth doing when a frame is short of time and not
  before — the whole pass is 1.3% of a 60Hz budget.
- **The expensive half is the CPU, by thirteen times.** 2.83ms in `light::collect`
  against 0.215ms in the shader, on the same frame. Everything argued about this
  pass so far has been about what a fragment does, and a fragment is not where the
  time is. Three separate things want fixing and they have three different fixes —
  which is why `cost.rs` reports them apart rather than as one number.
- **The map's statics over the lit bounds are walked twice a frame.**
  `light::collect` walks them for flames (`for_each_static_in`, 1.27ms of the
  2.83) and then hands the same bounds, the same map and the same cutaway to
  `occlusion::collect`, which walks them again for the grid (1.56ms). Every
  static is read twice, its tiledata entry looked up twice, and its `z` tested
  twice. One walk with two visitors is the same answer for about half the price,
  and the two are already in one function — this is not a design change, it is a
  loop that was written twice.
- **A widest-zoom frame's grid is 187×187 cells and 10,212 of them stand.** The
  backlog above says `Occlusion` is rebuilt and reallocated every frame at 140KB;
  the number under it is 1.56ms, which is what makes that item worth doing rather
  than merely worth writing down.
- **The pass is measured on `Cutaway::OPEN` and no ground items.** A player
  standing inside a house is drawn with storeys removed, which is a *smaller*
  grid and fewer flames, so these numbers are the outdoor worst case rather than
  the average. Nothing here says what a cutaway costs, and the cutaway is rebuilt
  every frame too.

Found while measuring a wall's facing out of its art:

- **The detector's own coverage is a moving target and only the sweep knows it.**
  37% of the graphic table, 76% of what Britain is built from. Both are printed
  and both have a floor asserted under them, but the floors are *measurements*
  and not targets — the thing they catch is a gate tightened until the feature
  stops applying, which is what the six-pixel `SPILL` did before it was measured.
- **The remaining quarter of Britain's walls has a shape** — and decision 25 took
  two thirds of it. The most-built unread graphics were `0x00DE`/`0x00DD` (roof
  slabs carrying `WALL`), `0x0081`/`0x0082` (pillars filling a whole tile) and
  `0x00C8`/`0x00C9`; the pillars read as corners now, and what is left is **8.1%
  of the statics standing in Britain**, headed by `0x02D8`, `0x02D3`, `0x02D6`
  and `0x02D0`. The entry's own prediction was right about the shape of the
  answer: not a looser gate but a second *kind* of it. Whatever is left will want
  a third, and it is worth printing the new worst list before guessing at one —
  `tests/facing.rs` does.
- ~~**A corner could be answered rather than refused.**~~ Done, decision 25, and
  the estimate in it was exact: four more stances and the rule that a pixel
  belongs to the face on its own half of the picture, which the fragment shader
  had in `across` already.
- **Nothing measures how far a decided face is from the edge except a gate.**
  The check that caught `0x0171` is a pass/fail inside `facing_of`; the *median*
  and the outlier list that made it obvious were a throwaway script. A graphic
  drifting from zero to two pixels across a client version is invisible until it
  crosses three and vanishes. The sweep prints two shares and could print this
  distribution for a few lines more.
- **The sweep reads the whole art file to answer a question about 3,212
  graphics.** It takes a couple of seconds, which is fine for an `#[ignore]`d
  test and would not be if it ever moved into CI.
- ~~**`facing_of` is a second walk of pixels the atlas has just copied.**~~ Closed
  by step 20b: with a table beside the install the atlas does a lookup, and the
  walk happens once in a tool that is allowed to take four seconds. The entry's
  own reading of the cost was right and incomplete — it is measurable on a scroll
  that introduces four hundred graphics, and the reason it had to go was not the
  cost but the *ceiling*: a measurement that has to fit in a frame can never be a
  search.
- **A wall's *top* surface is shaded as if it were the face.** The pixels past
  the tile's centre column — the thickness `SPILL` allows — clamp to the near end
  of the edge, so the top of a low garden wall is lit as though it were the
  vertical face at that point. Better than one flat tile and not right; the top
  is a horizontal surface and would want the flat mapping, which the silhouette
  can separate (it is the part above the base line's own 45°) but nothing does.
- **The frame dump can now be pointed at a debug view.** `OPENSHARD_FRAME_VIEW`
  in `tests/cost.rs`, and it is what made this step's measurement possible: a
  brightness profile across a *drawn* wall measures the timbers and the windows,
  not the lighting. `View::Light` throws the art away. Anything about the shape
  of a pool should be judged there.

Found while asking why a lamp on a house does not light the street:

- ~~**Thin spokes still fan out of a lamp standing against a wall.**~~ ~~**A ray
  through the corner between two panels passes between them.**~~ Both closed by
  decision 18, and they were one defect: a panel scaled by the length of a
  crossing passes a ray that clips the corner between two of them, through the
  first sideways and through the second over nothing. The entry above blamed the
  penumbra and it was not the penumbra. What is still true of it is that a named
  panel's shadow edge is now exact sideways — a straight line at the angle the
  geometry says, rather than a staircase on tile boundaries — and whether that
  wants softening is a question to ask of a moving picture, not of a still.
- ~~**A cell merges a lid and a panel into one mask and one span.**~~ Closed by
  step 21.2, and the entry's own reading of it was right in both directions: the
  span darkened air the map had nothing standing in, and the mask leaked a
  horizontal surface into the panel path. What it did not predict is the third —
  the *opacity* was a `max` too, so a pane beside a wall was opaque across the
  whole tile. "Two slots a cell" turned out to want 21 on the worst tile in
  Britain, which is why the answer was a list and not a second slot.
- **`crate::doors` is now deletable.** Decision 17 answers an open door out of the
  geometry, so the ported table earns nothing the edge mask does not. It is left
  in for one reason: 40 of the 104 open leaves are graphics `facing` refuses —
  the wide ones that stick past their own tile — and for those the table is still
  the only thing that knows. When that number is measured against the picture
  rather than against the art table, the answer is probably to delete it.
- **The atlas is now an input to the occlusion grid.** `light::collect` and
  `occlusion::collect` take `Option<&StaticAtlas>`, because a facing is a property
  of a picture and only the atlas has pictures. `None` is every occluder as a
  whole tile, which is what a built scene gets and what the tests that predate
  this still assert on. It is also the eighth argument of `light::collect`, which
  is one over what clippy likes and is allowed with a note.

Found while asking what an open door does:

- ~~**An open door is a tile-wide wall across its own doorway.**~~ Fixed:
  `render/src/doors.rs` and decision 11. What is left of it is the shape of the
  fix rather than the fix — see the two items below.
- **The shading half of a door already works.** 558 graphics carry `DOOR`; the
  ones `facing::facing_of` reads sit on a tile edge as squarely as a plain wall —
  median distance zero, none over two pixels — so an open leaf is shaded along
  the axis it swung to. Only the occlusion half is wrong.
- **The door table is thirteen families and a shard's own door is not in it.**
  `doors::is_open` answers `false` for anything it does not know, so a custom
  door occludes shut or open alike. The right home for that fact is the shard —
  it is what changes the graphic — and the wire does not carry it. A pack that
  ships doors would want to ship the table beside them, which is the shape
  `data/doors.json` is already in.
- **Four of the 208 graphics the table claims are not flagged `DOOR`.**
  `0x0692`, `0x0844`, `0x0846`, `0x0873`. Measured and left alone: they sit
  inside otherwise solid families, so it is the client's gap rather than a
  mistyped base, and nothing reads the `DOOR` flag anyway. Worth a look if one of
  them ever turns up drawn.
- **A pane and a door are the same question asked twice.** `occlusion::opacity`
  now takes the graphic because a flag describes a *picture* and an open door is
  a fact about a *thing*. A shutter, a portcullis and a drawbridge are the same
  shape and none of them is handled; what they all want is the client knowing an
  item's state, which is a seam this half of the workspace does not have.

Found at a house corner in Britain:

Four things were reported from one picture — a lamp in the street at
`(1441, 1693)`, against the corner of the house whose corner tile is
`(1441, 1692)`. One of them is closed by decision 24 and two more by decision 25;
what is left is the last entry below. **Three of the four were the same missing
fact**: `facing` refused a corner graphic, so `0x0033` was `EDGE_ANY` in the grid
and `Stance::Upright` in the attachment, and every consequence below follows from
one of those two. It is measured now — `facing_of` answers `Facing::Corner` — and
the entries it closes are struck through with what the fix turned out to be.

- ~~**A ray at 45° goes through a house corner into the room behind it.**~~
  Closed by decision 24. What is worth keeping of it is the shape of the report:
  the leak is a *stripe*, thinner than a tile and running the diagonal, so a
  per-tile diagram walks straight over it. `tests/onsite.rs` samples at a third
  of a tile for that reason, and that is what made it visible on the map rather
  than only in a built scene.
- ~~**A corner's two faces are lit as one.**~~ Closed by decision 25, and the
  estimate under it was right: widening the stance to four bits was three
  constants and no format change. What is worth keeping is the shape of the fix,
  because it is the shape any *pair* of surfaces in one picture will want — the
  corner exists in the instance word and nowhere else, and `statics.wgsl`
  resolves it per fragment, so every reader downstream of the world passes still
  sees one surface with one normal.
- ~~**A corner's pixels all claim the middle of their tile.**~~ Closed by the
  same. A corner's halves now map onto their own edges, so a run of wall, its
  corner and the run going the other way are one continuous surface — which is
  step 15's seam property arriving at the place it is most visible.
- **A corner's two faces are lit as one (as written).** `Stance::Upright` has no outward
  normal, so `blit.wgsl`'s `faces` is skipped entirely and both of the faces the
  art draws are as bright as each other — including the one turned away from the
  flame, which the corner itself occludes. Decision 22 fixed exactly this for a
  wall and cannot reach a corner, because there is nothing in the attachment to
  fix it with. What it needs is the **corner in the stance** — and the bits are
  not the obstacle, which is worth stating because it looks as though they are.
  Ten values need four bits where six needed three, and the stance rides at bit
  16 of the instance's second word and at bit 8 of the attachment's third
  channel: both have eight or more spare above it, so widening the mask is three
  constants in `place.rs`, `statics.wgsl` and `blit.wgsl` and no format change at
  all. What the work actually is: `facing::face_of` answering a corner instead of
  refusing one — it has already measured both halves by the time it gives up —
  `Face::place_at` mapping a fraction per half, and `outward` choosing between
  the two by which half of the sprite a pixel is on, which the shader has as
  `across`. `occlusion::edges_of` then returns two bits and the pierce path
  handles a two-edge mask already.
- **A corner's pixels all claim the middle of their tile (as written).** The same
  `Stance::Upright`, and the other half of what step 15 gave a wall: a faced wall
  spreads its pixels along the edge it stands on and reads as one continuous
  surface with its neighbours, and a corner between two such runs is a flat
  44-pixel band with a step at each of its two seams. It is the artefact step 15
  removed from 76% of Britain's walls, still standing in the 24% — and a corner
  is the place it is most visible, because it always has a faced run on both
  sides of it to be compared against.
- ~~**The floor under a wall tile is lit from outside the house.**~~ Decision 28,
  and the entry's own shape is what was built — exempt a face and an upright, test
  a flat — with one bound it did not name: only a *named* panel, so the tree, the
  post and the barrel keep the answer they had. The pier question below is still
  open and is now the only part of it left.
- **The floor under a wall tile is lit from outside the house (as written).**
  Neither end of a
  ray is shadowed by the tile it is on (decisions 3 and 17), and the reason is
  about a *wall's* pixels: its two faces are one tile and there is no telling
  which of them a pixel is on. A **ground** pixel on that same tile is not
  ambiguous at all — it is the floor, it is inside, and the ray from it to a lamp
  in the street crosses the panel its own tile stands on. So the corner tile's
  own square of floor comes out fully lit against a dark room, which is the
  small seam on the ground the report ends with. The fix has a shape and it is
  cheap: the attachment already carries the stance, `light::Spot` already carries
  a face, so the exemption can be asked of the *pixel* rather than of the tile —
  exempt a face and an upright, test a flat. What makes it worth measuring rather
  than assuming is that a real floor is often a static in the grid itself, and a
  floor that shadowed the thing standing on it would be a worse artefact than the
  one being removed. The pier entry above is the same question from the other
  side.

Found while deciding what a cell should hold:

- **A cell's fetch count goes from one to `1 + K`.** The walk reads one texel a
  cell today; with decision 30 it reads the index and then each of that cell's
  surfaces, inside the same loop. The GPU has the headroom — the whole pass is
  0.31ms against a 16ms frame — but it is a real change in the shape of the inner
  loop and it belongs in the measurement of step 21 rather than in its surprise.

Found while turning the cell into a list:

- **A cell's fetch count went from one to `1 + count`, and the measurement it was
  promised is not a comparison.** The entry above asked for it to land in step
  21's measurement rather than in its surprise, and here it is with a caveat: the
  cost instrument was run *after* the change and there is no before beside it,
  because the scene it walks has changed since step 6's numbers and the two are
  not like for like. The new baseline is in step 21.1. What a comparison would
  have to hold still is the flame count — this run found 7 where step 6 found 64.
- **The union's own cost is now countable, and it is 441 cells.** 10,212 standing
  cells hold 10,653 surfaces over Britain, so all but a few hundred tiles are one
  surface and the list costs almost nothing over the merged cell. That is the
  cheerful reading; the other one is that 441 cells is what step 21.2 will
  multiply, and a distribution — decision 30.6 — still wants printing rather than
  a total.
- **The surface texture is padded to a whole row, and the row is 1024.** A frame
  with 12 surfaces uploads 4KB. It does not matter at Britain's scale, where the
  list is ten rows, but it is a floor rather than a cost that scales, and a narrow
  scene pays it every frame. The fix, if one is ever wanted, is a row width that
  is a function of the count rather than a constant — which is a second number
  the shader would have to be told.
- **`Occlusion::at` folds on every call, and `boxes()` calls it per tile.** The
  merged view is derived rather than stored, so the wireframe overlay now costs a
  fold per cell where it used to cost a read. Nothing in a frame's hot path asks
  it, and the overlay is a debug view — but `shell.rs` draws it per frame when it
  is on, and that is the place it would show.
- **A tile's surfaces are contiguous, and that is an invariant nothing states.**
  `(offset, count)` names a run, and it only names one because `Builder::finish`
  packs in the index's own order. Nothing outside that function could break it
  today, and nothing outside that function is stopped from breaking it either —
  the list and the index are two private `Vec`s that agree by construction and
  not by type.

Found while splitting the union:

- **A change that has to move the picture moved no test.** Every test in the crate
  stayed green through step 21.2, which is not reassurance — it is the coverage
  report. A built scene is a `Map` with a handful of items on it and almost none
  of them puts *two* statics on one tile, so the whole suite had no opinion about
  the one thing this step is. The three tests that pin it were written for it, and
  the one that goes through the walk builds its grid with a `Builder` rather than
  with a scene, because a `Map` makes "two statics on one tile" fiddly to say. The
  same shape as the backlog's "a scene has no art, so almost every scene tests the
  whole-tile occluder": the scenes are thin exactly where the format is.
- **The union was put back to check the tests were red**, by hand, for one run.
  Worth writing down because it is the only thing that distinguishes a test that
  pins the new behaviour from a test that pins the arithmetic that happens to be
  there — and two of the three would have passed a weaker mutation (merging only
  surfaces with the same mask) that leaves the lid-and-panel case alone.
- **A cell's fetch count is 1.77 now, and the GPU noticed.** `night` went from
  0.368ms to 0.497ms on the same frame and the same machine, which is the first
  time in this file that a representation change has cost something legible. It
  is still 3% of a 60Hz budget and the CPU is still the expensive half by four
  times — but the backlog entry that asked for this to land "in the measurement
  rather than in the surprise" now has a real number in both halves.
- **The tail of the distribution is a shop, and nothing says which one.**
  Ten tiles in a Britain frame hold 21 surfaces. That is a stack of floors, walls
  and roof pieces on one square and it is almost certainly right — but the
  histogram is a count with no coordinate in it, and `tests/onsite.rs` is the
  instrument that would name the tile. Worth doing the first time a cap has to be
  chosen, which is not today.
- **`Occlusion::dropped` is counted and nothing asserts on it.** `cost.rs` prints
  it and a frame that dropped a wall would say so to a person reading the output.
  Nothing fails. The right home for an assertion is the bake of step 21.5, where a
  region is measured once and a truncation is permanent rather than one frame's.
- **A surface list makes duplicate suppression a linear scan.** `Builder::push`
  walks the tile's list looking for an exact repeat, which is one to three
  comparisons on 99.9% of tiles and twenty-one on ten of them. It is nothing today
  at 18,000 surfaces a frame; it is the shape that stops being nothing when the
  bake covers a block rather than a camera.
- **A tile's surfaces are contiguous, and that is still an invariant nothing
  states.** The entry below from step 21.1 is unchanged by this step and is now
  one function further from being checkable: the arena and the heads agree by
  construction, `finish` is what packs them, and nothing has a type saying so.

Found while giving a corner its two faces:

- ~~**A lid has no normal, so it takes the whole pool from any side.**~~ Decision
  27, and the estimate held: the stance already told a lid from a face and the dot
  product already had a `z` in it.
- **The plan view said `Upright` while its own comment said "flat ground".**
  Found by the change above, which is the point worth keeping: a fixture that
  writes a different attachment from the world pass answers about *itself*, and
  it cost nothing at all until the day a stance meant something for a floor. It
  writes `Stance::Flat` now. Worth a look at every other synthetic attachment in
  the tests for the same reason.

- **A pillar in the open loses two of its four sides.** A solid filling its whole
  tile reads as a corner, which is right about the *picture* and half right about
  the *tile*: a building's corner has its north and west sides inside the house
  and a free-standing pillar's are in the street, so a ray clipping a pillar's far
  corner now passes where the length rule used to stop it. The two are one
  silhouette and no gate can tell them apart. What can is the **map**: a corner
  has a wall on the tile beyond each of its two panels and a pillar has open
  ground on all four sides, which `occlusion::collect` is already walking. Until
  then it is a sliver of light past a pillar against a room leaking into a
  street, and this file has taken the second every time.
- ~~**Decision 22's exemption is a whole row or column, and a street lamp can
  stand in one.**~~ Written down as a finding one session and reported from the
  client the next, with the coordinates: decision 26. Worth keeping as a note on
  how it was found, because the entry was written from *reading the rule* and the
  report came from *looking at the frame*, and the two arrived at the same line of
  code from opposite ends. `scene::house_corner_named_by_its_art` now stands where
  Britain does — the lamp due south — instead of standing clear of the exemption.
- **A corner is not in the elevation view.** `plan::elevation` unrolls one run of
  one face — `wall.face` is a single `Face` — so the instrument that made
  decisions 22 and 23 visible cannot draw the join a corner makes between two
  runs, which is exactly where a seam artefact would now show. It is the same
  shape as `mark_seams` and wants the run to be a list of faces rather than one.
- **A built scene still gets `EDGE_ANY` unless it is handed art.** Three scenes
  now carry silhouettes and the rest do not, so the backlog entry above about
  thin coverage of the *panel* path is one scene better and otherwise unchanged.
  `scene::corner_art` is the place a fourth would be added.

Found while writing this plan:

- ~~**A sconce lights through its own wall.**~~ The oldest entry in this backlog,
  closed by decision 26. It wanted the wall's facing and it got it — by way of
  step 15 measuring one and decision 26 using it to place the flame rather than to
  excuse it.

Found while moving the measurement out of the frame:

- **The detector is offered 39,189 pictures and calls 4,362 of them corners**,
  where the `WALL` graphics alone hold 297. That is not new and not the table's
  doing — `StaticAtlas::insert` has always asked `facing_of` about every graphic
  it packs, `WALL` or not, and `place::Stance::of` reads the client's `FLOOR` bit
  and then takes whatever the art said. What is new is that somebody finally
  *counted* it. A solid filling its own tile reads as a corner because it is one
  shape (see the pillar entry above), so a crate, a rock and a tree stump are
  shaded as two vertical faces with a normal each. It costs nothing in the
  occlusion grid — a crate is not `NO_SHOOT`, so it is not a cell — and it is
  pure shading, which is why nobody has reported it. Whether it is *wrong* is a
  question for a picture: a barrel lit only on the two sides a camera can see is
  arguably better than one lit flat. The measurement to make is the same one
  `tests/facing.rs` makes for walls — the share of what actually stands in a city
  — for the graphics no `WALL` flag vouches for.
- **A tool that measures pictures has to build a renderer.** `artscan` depends on
  `client/render` for `facing` and `arttable`, and `client/render` depends on
  `wgpu`, so a build of the tool is a build of the graphics stack. Nothing about
  a silhouette needs a GPU. The shape of the fix is a crate under both of them —
  the measurement is a pure function of an `Image` and the table is text — and it
  is not worth doing for one tool; it is worth doing the moment a second reader
  appears, which is the same rule `doors.rs` states about its own table.
- **The staleness key cannot see a patch that keeps the file's length.** The
  stamp is `artLegacyMUL.uop`'s name and byte count, which tells two *installs*
  apart and would not notice an art patch that replaced a sprite in place. The
  honest alternatives both cost: a hash of a 150MB file every start, or a
  modification time, which changes on a copy and would re-derive for nothing. The
  install-gated test — every row against a live measurement — is what catches it
  on the machine that has the files, and it is the only thing that does.
- **`DETECTOR` is a number somebody has to remember to bump.** Nothing enforces
  it and nothing can: it is a claim about a diff. What makes it survivable is the
  same install-gated test, which compares the table against today's rules rather
  than against the version it says it was written by. Worth remembering when
  step 16 changes `facing.rs`.
- **The table's row grammar has one verdict and step 16 brings a second.** A row
  is a graphic and a facing today. An aperture is a rectangle in a surface's own
  coordinates, which is four more numbers and a question about which surface of a
  corner they belong to — and the version gate (`FORMAT`, refused rather than
  half-read) is what keeps a client from answering confidently out of a table
  written before the field existed.
- **Nothing runs the tool for the player.** A first run with no table is a client
  that measures as it packs, which is what it always did, and the log line says
  so — but somebody has to notice the line and run a command. The obvious next
  step is for the client to *write* the table when it finds none, which is a
  four-second stall at startup on one run in the lifetime of an install, and it is
  deliberately not done yet: it would put file writing into `client/app`'s startup
  path on the strength of one measurement of one install's size.

Found while cutting a hole in a wall:

- **A surface holds one hole, and decision 30.2 said "up to `K`".** A wall with
  two windows in it is two rectangles in one plane, and `Aperture` is a field
  rather than a list. One covers every window graphic the client ships as far as
  anybody has looked, and the cheap way out if it does not is a second surface on
  the same side with the same span — the walk takes the largest of a cell's
  surfaces, so two panels with two holes are not the union of the holes. That is
  the shape of the wrongness, and it wants a measurement (step 16) before it
  wants a fix.
- **A hole is a fact about a graphic and not about a thing.** Two windows of the
  same graphic in one wall have the same hole, which is right; a wall a siege
  engine knocked a gap in has nowhere to say so. The same boundary decision 11
  drew for doors — a flag is about a picture, a state is about a thing — and the
  same answer would apply: a per-item override, keyed the way `GroundItem` is.
- **The sky field does not know about holes.** `Builder::shade` multiplies a
  tile's sky by what each static leaves, and a static with a window in it leaves
  exactly what a solid one does — so a room under a glazed roof lantern is as
  dark as one under slate. It is the crude half of decision 14 meeting the fine
  half of step 21.3 and losing; the shape of the fix is that `shade` scales the
  opacity by the share of the tile the hole covers, which is arithmetic the
  aperture already carries.
- **Nothing draws the hole.** `Occlusion::at`'s merged view has no aperture in
  it, so the wireframe overlay and `plan::Picture::mark` both stroke a holed
  panel as a solid one — and a fan of light with no hole drawn on it is exactly
  the picture step 19 argued against: "a pool that is the wrong shape and a pool
  that is the right shape behind a wall nobody drew are the same picture until
  the wall is drawn on it". The instrument should gap the panel's stroke where
  the hole is.
- **The `field` plane's second channel is free again.** Its doc predicted
  "the sky today, an aperture and a body's opacity" — and an aperture turned out
  to be a fact about a *surface* rather than about a tile, so it went beside the
  surface list instead (decision 30.8). What that plane is for is unchanged and
  it has one more channel to spend than it thought.

Found while measuring a window off its own art:

- **The leaded window is refused, and it is the biggest thing left here.** 46 of
  the install's pictures draw a lattice — mullions across the glass, so a column
  of the sprite has two, three or four transparent runs in it rather than one —
  and the detector refuses the whole picture rather than pick one of them or
  merge them. Four of those 46 stand in Britain and one of them, `0x000E`, is on
  twenty walls in the tiles the sweep reads. It is the conservative direction (a
  refused window is a solid wall, which is what every wall was until this step)
  but it is the wrong answer for the most ordinary window in the game. The shape
  of a fix, and why it was not done here: a lattice is *mostly* hole, so the
  honest measurement is the largest rectangle over a region defined by how much
  of it is transparent rather than by a single run per column — which needs a
  threshold nobody has measured yet, and a threshold invented on the way past is
  how a detector starts reading light through stone.
- **A second gap anywhere in the picture refuses the first.** The gate is per
  *picture*, not per region: `0x24F6` is a porthole with a small second shape
  beside it, and both are lost. Refusing the picture is right when the two are
  windows; when one is a scratch it throws away a real one. What it wants is the
  same thing the entry above wants — a notion of *which region* is the hole —
  and the two are one piece of work.
- **A corner may not have a window.** `aperture_of` refuses a corner outright,
  because a hole would go to both of its panels and there is nothing in a
  silhouette that says which face it was cut into. No corner graphic in the
  install has a hole to lose, so this costs nothing today; what would change it
  is measuring the hole's *columns* against the halves, which is the same
  information the corner's two faces are already read from.
- **81 `WINDOW`-flagged pictures have no hole drawn at all.** `0x00CB` is one: a
  solid wall with the glass painted opaque. That is the flag and the art
  disagreeing, and the art wins here on purpose — decision 3's refusal, arriving
  for a second kind of measurement. What it means in a frame is that a flagged
  window with painted glass keeps `occlusion::PANE`'s fifth stopped across the
  whole tile, which is exactly the behaviour this step was supposed to replace.
  It is not a defect; it is where the art stops saying anything.
- **The measured `z` is quantised to whole units.** A hole's edge lands on a
  pixel and one unit of `z` is four of them, so a sill measured at 41 pixels
  becomes ten and a quarter and is written down as ten. The rounding is to
  nearest and the rectangle it rounds is already the inscribed one, so the error
  is under half a `z` in each direction and always at the *edge* of a penumbra
  the walk softens anyway. Worth knowing before anybody reads `Hole`'s numbers as
  exact.
- **A hole's `near` and `far` are the run of the whole tile, and a window is
  drawn on 22 pixels of it.** So the quantisation the other way is a 255th of a
  tile, which is finer than the art can say — `RUN_STEPS` was chosen for the
  walk's own agreement between shader and Rust, and it is comfortably finer than
  this measurement needs. Nothing to do; the asymmetry between the two axes is
  worth not being surprised by.

Found while making a floor stop light (decision 32):

- **A lid is a plane per *tile*, and it has no sub-tile hole.** A gap in a floor
  is a tile with no plank on it — `scene::hole_in_a_floor` — and that is what a
  house's floors are made of, so it is enough for what a house does. An
  `occlusion::Aperture` is still refused to a lid on purpose (step 21.3): a hole
  is a rectangle in a plane, and the run coordinate a rectangle would be stated
  along is a *vertical* panel's. A trapdoor would want one, and reading it would
  want a silhouette measured from above, which no art in the install is.
- **The edge of a floor is a hard step at the tile boundary.** The crossing test
  is per cell and a lid fills its cell, so nothing softens where the planks stop.
  It is the same shape decision 18 left the walls in — the surviving penumbra is
  vertical, and the lateral one was removed because a cell-local softening is
  measured from the *cell's* boundary rather than from the surface's silhouette.
  Consistent, and worth remembering the first time a shaft through a floor is
  looked at closely.
- ~~**Directly beside a flame, a storey up, the floor still passes light.**~~
  Closed by the three exemptions decision 32 had to narrow — the entry as first
  written blamed the own-cell rule alone and proposed the wrong fix. What it
  actually took: neither end's cell may exempt a **lid**, a vertical ray must
  still ask the one cell it stands in, and an exemption reaches only as high as
  the surface it is about (`on_surface`). The whole row of
  `scene::storey_over_a_torch` is now the ambient to six decimal places, the
  torch's own tile included.
- ~~**The line at the floor is the strict crossing test, seen.**~~ Closed, and by
  moving the *point* rather than loosening the test — see decision 32's fourth
  paragraph and `light::stand_clear`. Reported from a frame as a bright stroke
  along a house's floorboards; `scene::storey_over_a_lit_room` is the house it
  was argued in, and on the real one at `1509,1637` the wall's face at the floor's
  own `z` went from `through 1.00` to `0.09`, brightness `0.62` to `0.24` against
  an ambient of `0.20`.
- **A flame's height is not its width, and for a day it was.** `crosses` cuts a
  source by the plane it straddles, and the band it does that over was
  `FLAME_SPREAD * Z_PER_TILE` — a flame a whole tile tall. A house's sconce burns
  three to five `z` under the floor above it (Britain's at `1491,1636` is `z 31`
  under boards at `40`), so a tenth of every one of them was above the plane and
  the storey over it read `through 0.09` — a faint wash on the wall, reported
  from a frame right after the line at the floor was closed. `FLAME_DEPTH` is its
  own constant now, half a tile, which is `FLAME_LIFT`'s number and the only one
  in the file that is about a flame's *height* rather than the lateral softness
  of what it casts. Both houses now read `through 0.00` with the blocking cell
  named. `scene::storey_over_a_lit_room` burns its flame at sconce height for
  this reason: on the ground it would be fourteen `z` under the boards, far
  enough that any band at all would pass.
- ~~**What is left above a wall is the flame's assumed size, not a gap.**~~
  Mostly closed, and by the same category error one function over: a penumbra is
  the size of the source **across the edge it spills over**, and every edge this
  pass softens vertically — a wall's top, a hole's sill, a lid's plane — is
  horizontal, so what blurs it is how tall the flame is. `pierces` was given
  `soft * Z_PER_TILE`, a flame as tall as it is wide, and a ray passing three
  quarters of a `z` under the top of a wall kept two fifths of its light.
  `FLAME_DEPTH` now does that conversion everywhere. On the corner of Britain's
  house at `1509,1635`, over the wall beside it: `through 0.21 -> 0.00` at the
  wall's own top and `0.40 -> 0.11` three `z` above it. The lateral softness is
  untouched and still `FLAME_SPREAD`'s.
  What is left is `0.11` at that one height — `0.267` against an ambient of
  `0.251`, six percent — and it is a real penumbra rather than a leak: the flame
  burns four and a half `z` under the top of a twenty-tall wall, so the top of it
  genuinely clears the edge. Shrinking `FLAME_DEPTH` to an eighth of a tile would
  take it to nothing, and that is choosing a constant to make one pixel dark: at
  a quarter it is what the pictures show, four screen pixels to a `z` and a
  torch's drawn flame eight or ten of them.
- ~~**A strip of wall just above the floor line is still lit from the room
  below.**~~ Superseded by the two entries above: measured at the middle of a
  tile, which is not where a face pixel is. The band it reported at `z 40..42`
  is the seam at `z 40` and nothing at `41` and `42`.
  What is left, measured on a real house — the tile at `1490,1635` in Britain,
  a sconce at `z 36.5` on the tile southeast of it: the face reads `through 1.00`
  at `z 40` and `z 42`, and `0.18` from `z 45` up. The cause is geometry the map
  really has: **a house's floor covers the room and stops at the wall tile**, so
  the wall's own square is a column with no plank over it, and a ray from a flame
  near the wall crosses `z 40` inside that square rather than over a lid. It is
  a band a few pixels tall against a wall a storey high.
  What would close it is deciding that a wall tile is floored by its neighbours —
  a lid grown one tile into any wall tile that touches one at the same `z`. That
  is a *model* decision and not a bug fix: it invents a plank the map does not
  have, and the same invention would darken the street under an overhang. Worth
  putting to a person with the picture in front of them rather than settling
  here.

Found on a staircase in Britain:

- **A stair is read as a corner of two walls, and there is no stance for a
  slope.** Reported from `(1496, 1641)` and `(1493, 1639)`: a flight of stairs
  is drawn with hard triangles of shadow across it, as if the lit surface had
  been turned inside out. `tests/onsite.rs` at either tile names it in one line —
  the stair graphics `0x071E` (`1822`, height 10) and `0x0736` (`1846`, height 5)
  read `facing Some(Corner { right: East, left: South })`, `stance
  CornerEastSouth`, `opacity 255`, `climbable true`. So:
  - **The shading.** `blit.wgsl`'s `outward` gives the right half of every step
    the normal `(1, 0, 0)` and the left half `(0, 1, 0)` — two *vertical* walls
    meeting on the sprite's centre column. A stair's surface is neither: it
    climbs at roughly 45°, and its normal has a `z` in it. Every step is
    therefore lit as a pair of half-tiles turned away from whatever the sun and
    the flames are, and the seam between the halves is what the picture shows as
    a triangle. Nothing in [`Stance`](../crates/client/render/src/place.rs) can
    say "a slope": the enum is flat, upright, four faces and four corners.
  - **The occlusion.** The same verdict puts opaque panels on the tile's East and
    South edges for the stair's whole height, so a staircase shadows like a run
    of wall — including onto its own steps.
  - **The detector cannot see it from the silhouette alone.** A stair's base *is*
    a clean 45° run on both halves, which is exactly what `facing_of` asks for,
    and it stands 40 pixels tall, well over `MIN_STANDING`. What tells it apart
    is not the picture but the client's own bit: `TileFlags::CLIMBABLE`
    (`Bridge`, `0x0400`) is set on both graphics and on nothing that is a wall —
    the same order-of-policy argument `Stance::of` already makes for
    `is_background`, one flag over.
  What it needs, in the order it would be built: a `scene::staircase` with one
  flight and nothing else (the plan view is what says whether the shading follows
  the climb, the way `one-torch-on-open-ground` says whether a pool is a circle),
  then a stance for the shape, and the occlusion side, where a climbable tile
  should stop being a wall. Sphere already halves a climbable tile's height,
  which is a hint that the reference treats this shape as a special case too.
- **And the shape is a box, not a slope.** The first guess above was that a stair
  is an inclined plane and that what the art would have to be measured for is
  which way it climbs. Then the pictures were looked at — `tests/artshot.rs`
  writes any graphic out scaled with the tile's diamond stroked over it, and
  prints the lowest and highest drawn pixel of every column. `0x071E` is a **cube
  ten `z` tall**: its base is the whole diamond (21 pixels down to 0 at the
  centre column and back up, a 1:1 run, which is the diamond and not a wall's
  single 45° edge), and its top contour is the same diamond raised 42 pixels.
  `0x0736` is the same box with a **stepped lid** — three treads falling away to
  the west, which the column profile shows as three flats in the top contour.
  Against a real wall for contrast, `0x00C8`: base `21…2` across the left half
  and *nothing at all* past column 32. So:
  - The surface that dominates either sprite is the **lid**, and the lid is
    horizontal. It is currently lit as a vertical wall, which is the whole of
    what the report saw.
  - `facing_of` says `Corner { East, South }` about a box for a reason that is
    not a bug in it: a box's base *is* two 45° runs meeting at the south corner,
    which is exactly the silhouette two walls meeting at a corner leave. The
    detector answers about the two vertical faces it can see and there is no
    third answer in `Facing` for the lid on top of them.
  - **The geometry is a profile, extruded.** A height field over the tile that
    varies along one axis and is constant across the other: `facing::Prism`, with
    `up` naming the high side and `treads` the profile. A box is the one-tread
    case. `facing::prism_silhouette` is its forward projection, drawn the way
    `facing::silhouette` draws a wall, and every column of it is a vertical run
    the solid really contains rather than a rasterised polygon.
  - **And the fit against the client's own art says the model is right.**
    `tests/prism.rs` scores every candidate prism against a real sprite by
    intersection over union of the two silhouettes, aligned by the bottom row and
    the centre column — no free placement parameter. Measured on the staircase
    this came from:

    | graphic | best prism | agreement | tiledata height | drawn height |
    |---|---|---|---|---|
    | `0x071E` the landing | box, 5 `z` | **0.977** | 10 | 5 |
    | `0x0736` the flight | 3 treads climbing west, to 5 `z` | **0.975** | 5 | 5 |
    | `0x00C8` a plain wall | (control) | 0.812 | 20 | — |

    Two things fall out of that table and neither was expected. **The height
    cannot come from `tiledata`**: the landing states ten and the artist drew
    five, the flight states five and the artist drew five — the same field means
    the full height on one and the drawn height on the other, which is the same
    ambiguity `movement::scene::stair`'s "stand half way up it" lives with. The
    art is the measurement. And **the fit alone is not a gate**: a wall that is
    not a prism at all still scores 0.81, so what admits a prism is `CLIMBABLE`
    first and the score second — the order-of-policy `Stance::of` already uses
    for a floor.
  - **The grid believes it now, and the picture is one body per stair.**
    `Builder::add` asks `CLIMBABLE` first and, where the art fitted a prism,
    stands one `EDGE_ANY` surface at the *measured* height instead of two opaque
    panels on the tile's east and south edges. Measured at `(1493, 1639)` in
    Britain: the stair tiles read `edges NESW` where they read `-ES-`. A
    staircase no longer shadows a street like a run of wall.
  - **What is left is the treads themselves, and they are decision 36.** A tread
    is a body over *part* of a tile, and `Surface` has no way to say "part of":
    its three kinds are a panel on one edge, a lid, and a body over the whole
    tile. That missing form is the fifth of its kind in this file, which is what
    turned it from a fix into a decision — an occluder becomes a box, and a tread
    is one. Until that lands, a flight of steps occludes as a single box the
    height of its top tread. *(and the box turned out not to live in the tile's
    own coordinates either — decision 38, step 23)*
  - ~~**`ArtTable` does not carry a prism.**~~ **Done.** A row is `facing`,
    `hole` and now `prism U h…` — the face the climb ends at and one height per
    tread — at `FORMAT` 3, so a format-2 table is refused rather than half-read
    into a world where every staircase is a corner of two walls. It rides
    *beside* the verdict rather than replacing it, because the corner is what the
    wall detector really says about the picture and `Builder::add` is the one
    that picks between them on `CLIMBABLE`. A `face` may not carry one, which is
    the mirror of the hole's rule and comes from the same place: `Shape::of`
    scores prisms only against a picture it read as a corner, so a row saying
    otherwise would state what no detector will. `artscan` reports `solids:`
    beside `corners:` and `windows:` — the number that says the seconds a scan
    spends searching bought something, and the one a tightened gate would show up
    in and nowhere else.

Found while chasing a client that took half a minute to open a window:

- **The prism search redrew its candidates once per picture, and that cost was
  paid on the render thread.** `best_prism` scored a graphic against 261
  candidates and *drew* each one as it went — 129×129 samples a silhouette — so
  every corner the face detector found paid a quarter of a million tile samples.
  The candidate set does not depend on the picture, which is the whole of the
  defect: `artscan` went from **more than ten minutes** (it never finished) to
  **eleven seconds**, and the table-less client, which measures as the atlas
  packs, went from **27 seconds of black screen before the first frame** to no
  measurable stall. The candidates are drawn once (`facing::candidates`) and a
  candidate whose drawn-pixel count cannot beat the best score already found is
  never walked — an exact bound, `min / max` over the two counts, so the answers
  are identical: `tests/prism.rs` still scores the same stairs at 0.977 and
  0.975 and the same wall at 0.812.
  Two things worth carrying. **The cost was invisible because it was in the
  fallback**: decision 31.6 says a missing table is a log line and a slow first
  frame, and "slow" silently became "the window does not appear" when step 22
  added a search to `Shape::of`. A fallback nobody times is a fallback that can
  cost anything. And **the same shape is waiting for the thickness search** the
  entry above proposes — scoring a box of thickness `t` per picture is another
  candidate set that does not depend on the picture, and it should be built the
  same way rather than measured, found slow, and fixed again.
- ~~**A table makes the client read stairs as corners, and nothing says so.**~~
  **Fixed by the same change, and it is the half of it that mattered.** The
  entry recorded that `ArtTable` carried no prism; what was not written down is
  that this was a *behavioural* difference a person could turn on by running a
  tool — run `artscan`, and the graphics the atlas would have measured a prism
  for came back from the table with `prism: None`, so the staircase quietly went
  back to occluding like a run of wall while the log line said only how many
  pictures were read. The two honest states named there were "no table" and "a
  table with prisms in it"; the format bump is what removes the one in between,
  since a table written before the third verdict is now refused by version
  rather than read as a set of silent `None`s.
  What is worth carrying out of it: **the measurement was being paid twice or
  not at all, and never once.** A machine with no table paid the whole prism
  search while packing the atlas — the 27 seconds of black screen the entry
  above is about — and a machine with a table paid nothing and got the wrong
  answer. That is the shape decision 31 exists to prevent, and it came back the
  moment a *new* verdict was measured without a place in the file to put it. The
  next detector to land wants its row in the grammar in the same commit, not the
  one after.
  Left open: `tests/install.rs` has floors for faces, corners and windows and
  none for solids, because a floor is a number measured off a real install and
  nobody has run the sweep since format 3. It prints `solids:`; the floor goes in
  the day that print has a number in it.

- **An architectural alternative to decision 30/38's block cache, raised while
  building the spill (step 23.2), and worth arguing rather than losing.** The
  spill and the ring exist to patch a leak that is an artefact of one specific
  choice — caching the occlusion grid by baking it in the map file's own 8×8
  blocks (`bake.rs`) — rather than being inherent to "what stands between a
  flame and the ground" as a question. If solids were held in a structure
  queried directly by a frame's rectangle instead — an R-tree or a BVH over
  every solid in the facet, built once — there would be no block boundary for a
  solid to leak across, and no spill/ring to build at all.

  Why it was not built that way, as best this can be reconstructed without
  measuring: the shader still needs a **flat per-tile texture**
  (`Occlusion::bytes`/`id_bytes`/`solid_bytes`) to walk in `blit.wgsl`, and
  WGSL has no tree traversal — so a tree only ever helps the CPU side, "which
  solids does this rectangle see", and the rasterisation into a flat grid
  (this step's `Solid::footprint`, in different clothes) is still a separate
  step afterwards either way. Block-based baking also happens to align with
  the file's own I/O chunking (`Map::statics_in_block` is a contiguous slice
  per block for free), which a tree built from the same statics would not
  give up for nothing, but would not obviously need either.

  What would settle it rather than argue it: whether a persistent tree,
  queried per frame, actually beats "bake per block, cache blocks, paste a
  ring" on the numbers `tests/cost.rs` already reads — build cost once at
  load, per-frame query cost, and memory, over Britain at the widest zoom. If
  it wins, the honest scope is large: decision 38.1's grid-of-references, the
  cache in `bake::Bake`, and the whole shape of the spill this step just
  built would be replaced rather than extended, which is why this stays a
  backlog entry and not a step under decision 38 — it is a challenge to
  decision 30/38 itself, not a thing decision 38 asks for.
