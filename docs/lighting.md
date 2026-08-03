# Lighting: a flame that a wall can stop

A living plan, in the shape the other plans here have: the decisions numbered so
one can be argued with alone, the steps, and a backlog of what was found on the
way and left undone. It supersedes the lighting half of
[`client.md`](client.md)'s "Backlog, found while giving the client firelight";
what is still true there is linked from the backlog at the bottom rather than
copied.

## Where the next session starts

Steps 1–15 and 18 are done; **16 and 17 are not** — the *steps*; decision 17 is
a decision and landed. Step 15 has just landed and
it is half of the measurement step 16 needs: a wall's face is now read out of its
own art, and the window's hole is the same silhouette measured again — a span of
`v` along the face and a span of `z` — so whoever picks 16 up starts from
`render/src/facing.rs` rather than from nothing.

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
crossing.** Not a fixed number of samples along the segment: at two tiles apart
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

**And the flame's own tile stops being exempt.** That exemption was decision 3's
— a sconce must not be shadowed by the wall it hangs on, and with a whole-tile
occluder there was nothing else to do. With a panel there is, because the flame
sits at its tile's *centre*, which is inside the panel: a ray crossing it is
leaving the wall. Keeping the exemption is not merely generous, it is *visibly*
wrong — it lets a bright wedge straight out through the wall while the
neighbouring tiles' panels cut everything either side of it, so a street lamp
reads as a starburst and its own street is blown out. Measured on Britain
1439,1693: the tiles east of the lamp came out at 0.72 with the exemption and
0.41 without.

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
      cost but the tile-stepped ray the backlog names.
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

      Where it went: `render/src/facing.rs` holds `face_of(&Image) -> Option<Face>`
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
- **The ray is Chebyshev-sampled, one cell a step.** A ray running almost exactly
  along a diagonal can pass between two wall tiles that touch only at their
  corners. Real walls are rows, so it has not been seen; a supercover walk that
  visits both cells of every crossing would close it at about twice the samples.
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
  The cost was never the reason to leave F8 off; the tile-stepped ray below is.

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

Found while asking why a house's windows burn:

- **Eighty window graphics are flagged `LIGHT_SOURCE`, and every one of them is
  given a torch.** Scanned over the client's `tiledata.mul`: 615 statics carry
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

- **The sun's ray still steps a whole tile at a time.** `sunlight` and
  `light::walk_sun` sample one point per tile along the direction, which is the
  arrangement decision 14 replaced for flames: at a low elevation the ray skips
  cells, and what it does hit is tested all-or-nothing, so a sunlit frame has the
  tile-edged shadows a torchlit one no longer does. The traversal is written and
  wants lifting into a shape both walks can use — the only difference is that the
  sun's has no endpoint.

  **And it is not a softness question, it is a hole.** Measured in the sun view
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
- **The sun's walk does not test the panel a wall stands on.** Decision 17 gave
  the flame's walk the edge test — a cell stops a ray only where the ray crosses
  the side the thing is actually built on — and `sunlight` never got it: it tests
  a cell's `z` span alone, so a wall shadows the sun from every direction
  including along its own line. It cannot be lifted across on its own, though,
  because a point sample has no crossing to name; the entry above is what makes it
  possible, and the two are one piece of work.
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
- **The remaining quarter of Britain's walls has a shape.** The most-built unread
  graphics are `0x00DE`/`0x00DD` (roof slabs carrying `WALL`), `0x0081`/`0x0082`
  (pillars filling a whole tile) and `0x00C8`/`0x00C9`. None of those is a wall
  standing on one edge, so the honest next move is not a looser gate but a second
  *kind* of answer — a corner is two faces and could carry both, which is the
  same shape the occlusion grid would need to stop exempting a whole tile.
- **A corner could be answered rather than refused.** `0x0104` is an east face
  and a south face in one picture, and the detector has already measured both
  halves by the time it gives up. Two faces in the stance would need four more
  values and a rule for which half of the sprite a pixel is on — which the
  fragment shader has in `across` already.
- **Nothing measures how far a decided face is from the edge except a gate.**
  The check that caught `0x0171` is a pass/fail inside `face_of`; the *median*
  and the outlier list that made it obvious were a throwaway script. A graphic
  drifting from zero to two pixels across a client version is invisible until it
  crosses three and vanishes. The sweep prints two shares and could print this
  distribution for a few lines more.
- **The sweep reads the whole art file to answer a question about 3,212
  graphics.** It takes a couple of seconds, which is fine for an `#[ignore]`d
  test and would not be if it ever moved into CI.
- **`face_of` is a second walk of pixels the atlas has just copied.** One pass
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

- **Thin spokes still fan out of a lamp standing against a wall.** Most of the
  starburst was the flame's own-tile exemption and is gone with it, but not all:
  a run of wall with a doorway in it passes light where the gap is and stops it
  either side, and a panel is a line rather than a slab, so the boundary between
  the two is exact and reads as a blade. It is *correct* geometry drawn without
  a penumbra worth the name — decision 14's softening is scaled by how far along
  the ray the occluder is, and says nothing about how narrow the opening was.
  The honest fix is the several jittered rays the backlog already asks for.
  Judge it moving, in the client, rather than in a still.
- **A ray through the corner between two panels passes between them.** The
  diagonal gap the backlog already carries, arriving with more room to happen in:
  two walls meeting at a tile corner used to be two solid tiles and are now two
  panels that touch at a point. A supercover walk closes both at about twice the
  samples.
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
  ones `facing::face_of` reads sit on a tile edge as squarely as a plain wall —
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

Found while writing this plan:

- **A sconce lights through its own wall.** Decision 3 exempts the light's own
  tile from occluding it, which is right for a torch standing in a doorway and
  wrong for a sconce mounted on a wall: both sides of that wall are lit. The
  fix wants the wall's *facing*, which is decision 3's whole problem.
