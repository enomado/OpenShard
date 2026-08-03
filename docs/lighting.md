# Lighting: a flame that a wall can stop

A living plan, in the shape the other plans here have: the decisions numbered so
one can be argued with alone, the steps, and a backlog of what was found on the
way and left undone. It supersedes the lighting half of
[`client.md`](client.md)'s "Backlog, found while giving the client firelight";
what is still true there is linked from the backlog at the bottom rather than
copied.

## Where the next session starts

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

**Decision 29 is the next one and it is written but not built**: a cell should
hold a small list of panels rather than one merged span, which is what a window's
aperture (step 16) needs, what stops a lid and a wall on one tile from merging,
and what "polygons, honestly" means in a world where every surface is
tile-aligned. Read it before touching the grid's format.

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

**29. What a cell should hold: panels, not one merged span.** *(the shape of the
next format change, not built)*

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
- [ ] **16. The window's aperture, and the beam on the ground.** A pane passes
      four fifths of the light *across the whole tile* today, which is a dimmer
      tile and not a beam. The hole is in the art — a window graphic's silhouette
      has a transparent gap in an opaque wall — so the same measurement as step
      15 yields the aperture: a span of `v` along the face and a span of `z`.

      Two things change. The occlusion cell has to carry it, and it is full at
      four bytes (`Rgba8Uint`), so this wants a second texture or a wider format.
      And the walk, which already knows where the ray enters and leaves a cell
      and at what height, tests whether that crossing passes through the
      aperture's rectangle on the face — a few lines where the span test is.

      What comes out is a fan on the street: narrow at the wall, widening with
      distance, with the soft edge decision 14's penumbra already gives it.
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

Found while building it:

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
- **`facing_of` is a second walk of pixels the atlas has just copied.** One pass
  per graphic, on the frame it is first packed, and the packing pass is already
  touching every one of them. Measurable only on a scroll that introduces four
  hundred graphics at once, which is the frame `StaticAtlas::add` already owns as
  the expensive one.
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
- **A cell merges a lid and a panel into one mask and one span.** `Occlusion::add`
  unions everything on a tile, so a floor over a wall tile contributes its `z` to
  the span while the wall contributes the sides — and the floor's own lid-ness is
  lost. Conservative in the direction that darkens for the span and in the
  direction that leaks for the sides, which is not one direction. Two slots a
  cell would fix it and want the wider format step 16 is already asking for.
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
