# Lighting: a flame that a wall can stop

A living plan, in the shape the other plans here have: the decisions numbered so
one can be argued with alone, the steps, and a backlog of what was found on the
way and left undone. It supersedes the lighting half of
[`client.md`](client.md)'s "Backlog, found while giving the client firelight";
what is still true there is linked from the backlog at the bottom rather than
copied.

## Where it stands

`client/render/src/light.rs` collects the flames a frame can see, `blit.wgsl`
multiplies them over the finished world image, and F10 toggles night. Every
light is a **circle in the pixels of the drawn image**: `light::place` projects
the flame's tile to a screen point, and the blit compares each fragment's screen
position with it.

That arrangement cannot be given walls. Two facts about it are the whole reason
this plan exists:

- **The screen folds height into `y`.** A brazier in a cellar and a lantern on
  the street above it are a few pixels apart in the image, so the pool of one
  covers the other. This is `client.md`'s "a flame lights through a floor".
- **A wall's sprite stands above the tile it occludes from.** A wall is 44
  pixels of picture rising from a diamond that is at the floor. Whatever
  screen-space mask darkens the ground behind the wall also covers the wall's
  own face — including the face *turned towards the flame*, which is the one
  surface that must obviously be lit. There is no shadow polygon that fixes
  this, because in the image the lit face and the shadowed ground are the same
  pixels of the same sprite; only a per-pixel answer to "which tile is this?"
  separates them.

So the pass moves from the screen into the world, and the shadow comes with it.

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

The grid carries an *opacity* byte rather than a flag, so that a hedge or a pane
can dim rather than stop later; today the rule above fills it with 0 or 255 and
the shader multiplies either way.

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

## Steps

- [ ] **1. `render/src/occlusion.rs`.** The tile grid of decision 4/5, built
      from the map, the tiledata and the cutaway over the bounds `light.rs`
      already computes. Pure CPU, no GPU types, tested without client files: the
      builder takes occluders one at a time and the map walk is the caller.
- [ ] **2. The `(x, y, z)` attachment.** `ground.wgsl` and `statics.wgsl` gain a
      second output; the quad structs gain the tile they are for; the renderer
      gains the texture and a second colour target. The frame tests read it back
      and assert that a wall's pixel names the wall's tile.
- [ ] **3. `light.rs` in world coordinates.** A `Light` becomes a tile, a `z`, a
      radius in tiles. `place` and `FLAME_LIFT` go; the lift becomes `z` units.
- [ ] **4. `blit.wgsl`.** Reads the attachment and the occlusion texture,
      computes the world distance and the ray's product of opacities.
- [ ] **5. Wiring and a picture.** `app/src/lib.rs` passes the occlusion grid;
      a screenshot of a torch inside a house that no longer lights the street.

## Backlog

Carried from `client.md`'s firelight backlog and still true:

- `light.mul` / `lightidx.mul` are not read; `light::flame` is the stand-in.
- Nothing a mobile carries burns — a player holding a torch makes no light.
- The ambient is a key (F10), not a clock.
- A light is placed by its tile, not by its sprite.

Found while writing this plan:

- **A sconce lights through its own wall.** Decision 3 exempts the light's own
  tile from occluding it, which is right for a torch standing in a doorway and
  wrong for a sconce mounted on a wall: both sides of that wall are lit. The
  fix wants the wall's *facing*, which is decision 3's whole problem.
