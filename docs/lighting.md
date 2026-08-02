# Lighting: a flame that a wall can stop

A living plan, in the shape the other plans here have: the decisions numbered so
one can be argued with alone, the steps, and a backlog of what was found on the
way and left undone. It supersedes the lighting half of
[`client.md`](client.md)'s "Backlog, found while giving the client firelight";
what is still true there is linked from the backlog at the bottom rather than
copied.

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

**3. An occluder is a whole tile, not a wall's edge.**
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

This is also *cheaper* than what is there now. Today every fragment of the
screen runs the loop over all 64 lights; here a fragment outside every radius
leaves the loop immediately, and at night most of the screen is outside every
radius.

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
occluder.** The client is *told* a door opened: the item's graphic changes, and
the open leaf's graphic is not `NO_SHOOT` (you can shoot through an open door,
which is the same question decision 4 asks). So the tile leaves the grid on the
frame the graphic changes, and light fans out through the doorway with nothing in
this pass knowing what a door is. What that buys is worth stating: the spill is a
*tile-wide* fan, not a thin blade, because decision 3's occluder is a whole tile
and the opening therefore is one too.

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
- [ ] **6. A picture, and a number.** A screenshot of a torch inside a house
      that no longer lights the street, and a frame time from the playground at
      the widest zoom on Britain — the arrangement is *cheaper* per pixel than
      the one it replaces (a fragment outside every radius leaves the loop at
      once, where the old one ran all 64 lights for every pixel of the screen),
      and that claim is worth a measurement rather than an argument.
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
      the sun is F8 in the app, off by default until step 6's measurement says
      what a ray on every ground pixel costs.
- [ ] **12. The shaft.** The screen-space pass of decision 12, over the mask the
      step above produces.

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
  test (`Occlusion::tallest`) makes it two or three steps over open ground, but
  the number is still unmeasured — which is why the app's F8 is off by default.

Found while writing this plan:

- **A sconce lights through its own wall.** Decision 3 exempts the light's own
  tile from occluding it, which is right for a torch standing in a doorway and
  wrong for a sconce mounted on a wall: both sides of that wall are lit. The
  fix wants the wall's *facing*, which is decision 3's whole problem.
