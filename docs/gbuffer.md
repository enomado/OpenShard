# The G-buffer: an id and a depth, not a pixel's whole history

A living plan, in the shape the other plans here have: the decisions numbered so
one can be argued with alone, the steps, and a backlog of what was found on the
way and left undone.

Born from a dead end, not from a survey: [`lighting.md`](lighting.md) decision
40 needed a tread's own tilted normal to reach `blit.wgsl`, and `place::Stance`
had four bits and ten values already in them — no room, and no clean way to
make room, because the four bits were never a normal in the first place, they
were an index into ten hand-picked ones. Pushing on *why* the attachment is
shaped that way at all is what this plan is; decision 40's steps 4 and 5 are
now tracked here instead of there.

**Not a novel idea — an overdue one.** `lighting.md` decisions 36 and 38.3
already argued this, for the *occlusion* grid rather than the render
attachment: "the same box gives the shading half its answer for free: a
pixel's normal is the normal of the face it landed on, which is what
`place::Stance`'s nine values are a hand-rolled enumeration of" (36), and
`place::Stance`'s values are "a derived answer instead of a taxonomy to
extend" once a pixel's face comes from the same slab test occlusion already
runs (38.3). Its own backlog said outright that decision 22's one-sidedness
and `place::Stance` "should be *removed*... rather than carried alongside the
thing that replaces them" (backlog, "found while giving the world a spill",
lines 3676–3682) — and left it for whenever something forced the question.
Decision 40 is that something. This plan is that backlog item, executed for
the render side.

**Checked against `occlusion::Solid` before assuming it, not after:** the
struct exists (`crates/client/render/src/occlusion.rs:490`) but carries no
`normal_of_face` or equivalent today — 36/38.3's "for free" is the argument
for building one, not a description of something already there. See decision 3
and "Not settled" below for what that means for this plan.

Written against `crates/client/render/src/place.rs`, `select.rs`, `light.rs`,
`statics.wgsl`, `ground.wgsl`, `mobiles.wgsl`, `blit.wgsl`, `renderer.rs`.

## Where it stands today, and why it is tight

[`lighting.md`](lighting.md) decisions 1–2 put a second colour target behind
every world pass: `place`, `Rgba16Uint`, `(x, y, z + 128, kind)` with the
stance riding in four spare bits of the third channel — the tile a pixel
belongs to, the height it was drawn at, what kind of thing wrote it, and which
way it faces. `blit.wgsl` reads it to light in world coordinates instead of
screen ones; `select.rs` reads it a second time, for the ground a selected
thing stands on.

Every bit of that channel word is now spoken for except the four decision 40
wanted. Widening the format to make room for a real vector is not a fix, it is
the same design one texture format wider — the questions below are about
whether the payload should be shaped this way at all, not about finding four
more bits.

**Two things this payload cannot do, independently of decision 40:**

- **It cannot name what it came from.** `select.rs`'s own doc says so: two
  statics on one tile write the same tile, the same stance, the same height —
  the attachment cannot tell them apart, which is why picking *which* static is
  never asked of it (see below) and why the selected-thing wash needs a second,
  separate silhouette pass (`SpriteRenderer::render_mask`) just to know which
  one to shade.
- **It repeats per pixel what a whole sprite shares.** A wall's `(x, y)` is one
  tile — the same two numbers, written unchanged into every one of the maybe
  few thousand pixels its picture covers. That repetition is not a rounding
  error; it is most of the channel's width spent on a fact that was already
  known once, at the instance, before a single pixel of it was drawn.

## Picking does not use this buffer, and that is the control group

Worth stating because it looks like it should and does not: `statics::pick`,
`items::pick`, `mobiles::pick` (`crates/client/render/src/{statics,items,mobiles}.rs`)
answer "what is under the cursor" entirely on the CPU, by replaying each
candidate's own placement and testing the atlas for an opaque texel — no
readback of `place`, no GPU round-trip. `docs/client.md`'s M5 backlog now notes
the four near-duplicate walkers this makes (`statics`/`items`/`mobiles`/`gump`)
as worth a common shape later; that is a separate, smaller cleanup and not
blocking on anything here.

So `place` has exactly two readers — `blit.wgsl` and `select.rs` — and both are
about *what is here*, never *what did this*. That is the seam this plan cuts
along.

## The decisions

None of them is implemented yet.

**1. The payload shrinks to a depth and an id, not explicit attributes.**

The algebra is the argument. The projection (`camera.rs`) is affine and fixed —
`screen_x = (x - y) * HALF_WIDTH`, `screen_y = (x + y - 1) * HALF_HEIGHT -
z * Z_STEP` — two equations in three unknowns. A screen pixel alone names a
*line* through world space, not a point: exactly one degree of freedom is
missing, no matter how simple or how constant the projection is, because that
is what an orthographic projection is — a camera direction collapsed away on
purpose. A single scalar closes the system; a full `(x, y, z)` is two numbers
more than the missing information actually requires, once that scalar exists.

Which fragment's scalar survives is already decided before this payload is
ever written: the depth test each world pass already runs (`depth_state()`,
`LessEqual`) picks the one point along that line that is visible, the same way
it always has. The buffer's job was never to resolve *that* question — it
resolves the one algebra cannot: which object is the surviving fragment part
of. Hence two channels, not one and not five: a **depth**, and an **id**.

**2. The id indexes a storage buffer.**

`lighting.md` decision 30.5 ruled storage buffers out for the occlusion grid
under the crate's old ceiling, WebGL2. That ceiling is no longer current —
decision 30.5 was reopened and answered: the crate's target is now **WebGPU**,
which has real compute shaders and storage buffers, and `crates/client/render/
src/lib.rs`'s own module doc is the record of the change. `Occlusion`'s
texture-folded lookup (`LIST_ROW`, decision 38.4) is not being undone — it
stays, running, as a still-valid technique — this plan simply does not have to
route around the constraint that produced it, because that constraint no
longer applies to new work.

So: build the plain mechanism, `instances[id]` — no row-width bookkeeping, no
address arithmetic to keep in sync between the bake and the shader. If a
future target ever needs the older ceiling back, decision 30.5 has the
compression already worked out (a texture and `textureLoad`, "a dozen lines,
not a millisecond") as a known, cheap fallback — not a reason to pay for it now.

Instance data is already uploaded to the GPU today — `statics.wgsl`'s
`@location`-tagged vertex attributes are a per-instance buffer, just one the
GPU can only walk in draw order, not address at random. A storage buffer is
the same memory bound a second way: `blit.wgsl`'s fragment stage reads the id
out of the attachment and does `instances[id]`, an arbitrary lookup a
vertex-attribute binding cannot do. No second draw call and no second upload —
one buffer created with both `VERTEX` and `STORAGE` usage, bound once more, to
a different stage.

Everything the four spare bits and the fixed `outward()` switch used to encode
— which tile, which way it faces, what kind of thing this is — becomes a field
of that one row, read once per fragment instead of decoded from a packed word.

Three buffers, not one — ground's row and a face's row hold different data
(ground has no normal to carry, a face has no corner heights), so `instances`
is really `ground_instances`, `face_instances` and `mobile_instances`, and
`Kind` (unchanged from today's two bits, `place.rs:58-74`) is what the id
channel carries to pick which — see "Not settled"'s id-width item for the bit
budget and the measured counts behind it.

**3. An id names a face, not a game object — and the normal is honest per-face
geometry, not a fitted formula.**

Decision 40's problem dissolves twice over, not once. The first way: a normal
is per-*instance* data once the id exists, and an instance budget is thousands
of entries, not a screen's worth of pixels, so there was never a reason to fit
it into a code. The second, sharper way: the *tread itself* stops being
approximated by one surface standing in for a shape it is not.
`Prism::tread_normal`'s blend — mixing `Surface::Flat`'s `[0, 0, 1]` toward
`Surface::Face(up)`'s horizontal normal by the tread's own rise over run —
exists to make one flat top read like the ramp a flight of them actually is.
That is a proxy for geometry the occlusion grid already has and does not need
approximating: a tread is a box with up to two faces a camera ever sees — its
top, and the riser between it and the tread before it — and a lid static
under the flight contributes its own top where no tread covers it. This
session's stair is 3 treads × 2 faces + the lid's own top: **seven honest
normals**, none of them blended.

So the id this plan's decision 1 and 2 describe names a **face**, not a game
object. A wall's sprite has one relevant face and collapses to exactly what it
is today — one id, no change. A tread has up to two; a corner has exactly
two. Each is rasterised separately, by the invisible geometry pass decision 4
below adds, and each carries its own trivial, axis-aligned normal — `[0, 0, 1]`
for a top, the climb's own outward direction for a riser — with no blending
arithmetic anywhere. A corner static gets this for free: its two faces become
two separate face-instances, rasterised and depth-tested independently, which
is what `statics.wgsl`'s screen-half resolution stands in for today — decision
25's exact arithmetic, `right = FaceNorth + (offset >> 1)`,
`left = FaceSouth + (offset & 1)`, `stance >= STANCE_CORNER` picking which
pixels ask the question at all.

**This also reopens a settled claim, not just a bit-packing choice — say so
plainly rather than let a reader trip over the contradiction.** Decision 30's
fourth bullet and decision 38.3 both state, as a closed question, that
occlusion geometry stays out of the render path: "the G-buffer is still the
bridge from a pixel to a world surface... a solid is consulted by the light
and by the normal, and never by the rasteriser" (38.3) — the separation was
called "exactly why the freedom is affordable" for occlusion's own shape.
This plan does not touch the rasteriser (decision 4 below keeps the visible
sprite draw untouched), but it does put honest per-face geometry behind the
G-buffer's *normal*, which 38.3 filed under "the light and... the normal"
already, so the two decisions are not actually in conflict — 38.3 drew the
line at the rasteriser, and this plan stays on the same side of it. Worth
tracing precisely rather than asserting, because the wording is close enough
to invite a wrong skim.

**Not settled, and worth checking before believing either answer:** decision
40 existed because a single flat top read wrong next to a nearby light — a
hard-edged cliff in `cone` across the first tenth of a tread's climb. Honest
per-face lighting might fix that on its own, if a correctly-lit riser gives the
eye the continuity the flat top's tilt was faking — or it might not, if the
top's own hard cutoff is still there regardless of what lights beside it.
Check with the same tool and the same reproduction that found the original bug
(`examples/isolated_scene.rs`'s profile mode, the `1497,1626,10` stair) before
assuming either way. If honest geometry fixes it alone, `Prism::tread_normal`
is **deleted rather than ported** and step 5 below shrinks to nothing.

**4. The invisible geometry pass stays depth-consistent with the visible one.**

A face-decomposed proxy is a second, parallel description of the same objects
the sprite passes already draw — not a new source of truth about what the
world contains. If a rug's sprite covers a floor's on screen, the proxy pass
has to agree the rug's face wins the same pixels, or lighting answers about a
surface nothing shows. Concretely: the proxy pass draws from the same instance
list and the same depth ordering the visible passes already use, decomposed to
faces, so a fragment's winning id is always the one the picture agrees is on
top.

**5. Geometry-agnostic on purpose.**

Nothing above assumes the surface is a flat billboard. A depth and an id are
exactly what a rasterised triangle of real geometry produces too — the
reconstruction does not care whether the id's row describes a sprite's tile or
a mesh's material, because "what was here" was never answered by the shape of
the thing, only by what its own data says. [`client.md`](client.md)'s later
milestones bring real geometry into this client; this plan is written so that
day does not reopen the G-buffer's shape a second time. The alternative this
plan does *not* take — deriving a billboard's exact world position from its
screen row and a closed-form offset, which is true today and stops being true
the day a sprite is a mesh — was considered and dropped for exactly that
reason.

## Not settled

- ~~Whether the existing depth texture (`Depth24Plus`, `renderer.rs:45`)
  already carries a reconstructable value, or only an ordering key.~~
  **Answered (step 1): only an ordering key.** Every world pass writes
  `order.to_depth(base)` (`statics.rs:269`, `ground.rs:194`, `mobiles.rs:434`)
  and nothing reads the depth texture back — `blit.wgsl` has no reference to
  it at all, only to `place`. The value itself is not `z`: `Order` folds two
  numbers into one key, `(tile - base) * DEPTH_PER_TILE + priority_z`, and
  `priority_z` is not a world height — it is `z` bent by object-type-specific
  adjustments (ground averages its corners and subtracts 2, a static shifts
  ±1 by two tiledata flags, a mobile adds 1) that only make sense once the
  object's kind is already known. Two different world heights on the same
  tile can fold to the same key — `depth::tests::
  priority_z_can_collide_for_two_different_world_heights` pins a concrete
  pair, a flat static at `z=5` and a wall at `z=4`, that produce the same
  `priority_z` and hence the identical depth value — which a linear depth
  worth reconstructing from could never do for two genuinely different
  points. So: a linear world depth has to be written somewhere it is not
  written now — decision 1 is not free. In exchange, decision 2 already pays
  for the thing decision 1's algebra was reaching for: the id's storage-buffer
  row can hold the instance's real `(x, y, z)` directly, read once per
  fragment, with no per-pixel inverse-projection needed for a billboard at
  all — the projection algebra only has work left to do for ground's
  per-pixel slope (the fourth "Not settled" item) and decision 16's fraction,
  not for recovering an object's own anchor.
- ~~The id's width and the buffer's capacity, i.e. how many faces one frame's
  storage buffer is sized to hold. Not measured against how many faces a real
  screen ever holds...~~ **Answered (step 2):** measured, at the same frame
  `docs/lighting.md`'s 25,702/17,201-statics numbers were taken from —
  Britain, widest zoom, `Cutaway::OPEN` (nothing hidden, the worst case) —
  by extending `tests/cost.rs` to print the two `collect()` calls it already
  builds but never counted (`cost.rs:319-347`):

  ```
  27889 ground quads, 6560 static quads (431 of them corners,
  so 6991 faces once decision 3 splits each in two)
  ```

  Ground is not face-decomposed by decision 3 — a land quad is one face today
  and stays one — so its count needs no correction. A static's does: 431 of
  6,560 carry a corner `Stance` (`place::STANCE_CORNER`, `place.rs:160`) and
  decision 3 splits each into two faces, so 6,560 objects become 6,991 faces.
  Treads are not in this number — nothing decomposes a tread into top+riser
  yet (`occlusion.rs:1358-1378` still pushes one body per tread, not two
  faces; see decision 3's own body above for why that gap is the render
  side's to close, in step 4) — but a flight of stairs is a small, bounded
  class of statics next to 6,560 of them, and even a generous doubling of
  every tread in view would not move this number by an order of magnitude.
  Mobiles are not in it either, for a different reason: they are population,
  not map data, so a snapshot of Britain's furniture cannot measure them —
  but every mobile-or-worn-layer quad is already exactly one face by
  construction (`Stance` never varies by corner or tread for `Kind::Mobile`),
  so no multiplier applies there at all, only a raw count bounded by however
  many characters a shard ever draws on one screen, which is server
  population and an order of magnitude below the map's own furniture in any
  arrangement this client has ever drawn.

  So: **one shared 32-bit id channel**, not per-kind widths to track. Two
  bits carry `Kind` — unchanged from today's values (`place.rs:58-74`,
  `Nothing=0, Land=1, Static=2, Mobile=3`) — and select which of three
  storage buffers (ground quads, static/item faces, mobile-and-layer quads)
  the remaining 30 bits index into. Thirty bits is not a number chosen to
  feel safe; it is chosen because a per-kind id, each counted separately,
  needs nowhere near it — the widest real frame measured needs 27,889 for
  one kind and 6,991 for another — and one width for all three kinds avoids
  the "row-width bookkeeping... to keep in sync" decision 2 already refused
  to pay for, extended from *one* buffer to three. If a future city or a
  crowded event needs more of one kind than Britain ever showed this client,
  the ceiling is 1,073,741,823 away, not a few thousand.

  The **buffer's capacity** — the initial allocation, not a cap; decision 2's
  storage buffers resize the same way today's vertex buffers already do
  (`renderer.rs:491-495`, `1029-1031`) and never drop a frame's worth of
  instances — is a different question, and this measurement answers it too.
  Today's `INITIAL_QUADS = 4096` (`renderer.rs:37`) is not sized against
  reality: a widest-zoom frame at Britain already asks for 6.8× that many
  ground quads and 1.6× that many static quads, so **the very first frame
  drawn at this location reallocates**, on every run, before a single pixel
  is on screen. Worth carrying into step 3 rather than repeating: seed each
  kind's storage buffer from the measured count with one power-of-two of
  headroom, the same rounding `renderer.rs`'s own grow path already uses —
  ground at 32,768, static/item faces at 8,192, mobiles left at today's 4,096
  for lack of a map-density argument to move it, since population is a
  server number this measurement cannot speak to.
- **Whether the face-instance table is its own texture or a field `occlusion`
  already has reason to grow.** The intro's `occlusion::Solid` check found no
  `normal_of_face` today, so decisions 36/38.3's "for free" has to be built
  either way — the open question is *where*: as a face table this plan owns,
  or as a method on `Solid` occlusion's own walk could also call once built,
  with this plan's table holding only the id-to-`Solid`-and-face mapping.
  The second keeps one shape of "a box's face" instead of two; not chosen yet
  because it means reading `occlusion.rs`'s box representation closely enough
  to know it fits a renderer's needs, not just a shadow walk's.
- **Decision 16's fraction is likely free, but say so only once checked.**
  `blit.wgsl` reads it to build `at`, the sub-tile world position the whole
  lighting distance calculation runs on (`sub.x`/`sub.y` folded into `at.x`/
  `at.y`, `blit.wgsl:1277-1281`) — it is not a separate ground-texturing fact,
  it is the fractional part of the same position decision 1 already argues is
  reconstructable. Solving decision 1's two equations for `x, y` given a depth
  does not round to a tile; the fraction should fall out of the same algebra
  for nothing, for any flat/billboard face. Worth confirming in step 1 rather
  than assumed, and it does not extend to ground's bilinear case (the item
  above) for the reason already given there.

  **Step 1 answered, and it complicates this item rather than closing it.**
  The depth that exists today is not the reconstructable position this
  paragraph assumes (see the first item above) — a linear depth still has to
  be written, and once it is, it names an object's own anchor, which decision
  2's `instances[id]` row already carries directly. Whether solving for the
  fraction from a *new* linear depth is still worth doing, versus reading it
  off the id's row the same way `(x, y, z)` would be, is a separate question
  this step did not answer and step 2 or 3 should settle before building
  either path.

  **Step 2 settles the face-instance half: it needs no fraction at all.** A
  fraction is sub-tile position — where inside its tile a fragment sits — and
  that question only has content for a surface a fragment's position varies
  across, which is what makes ground's bilinear quad different from
  everything decision 3 gives an id to. A static's face and a mobile's
  billboard occupy one anchor; every fragment of a wall's picture is the same
  `(x, y, z)` decision 2's row already carries, exactly as decision 1's own
  "Not settled" answer says ("no per-pixel inverse-projection needed for a
  billboard at all"). So the face-instance row below carries no fraction
  field, not because solving for one was tried and dropped, but because
  nothing on a billboard face varies with it — the question was never this
  row's to answer. It stays exactly what the item below already says it is:
  ground's own, unresolved, and step 7's.

  **The row itself, decided here rather than left implicit:** an anchor
  (`x: u16, y: u16, z: i8`, unchanged from today's `Place`, `place.rs:219-237`)
  and one **normal**, in place of today's ten-value `Stance`. Decision 3
  already removes the four corner values — a corner is two rows now, not one
  value naming two faces — so what is left to name is a top (`[0, 0, 1]`), one
  of the four cardinal directions (a wall's face, or a riser's "climb's own
  outward direction"), or *unknown* (`Stance::Upright`'s fallback, "a tree, a
  body, a post... across it nothing varies"): six values, not ten, three bits
  where `Stance` needed four. `kind` does **not** ride in this row — it is
  what selects *which* of the three per-kind buffers above the id indexes
  into in the first place, so a row stating it again would be exactly the
  repetition this plan's intro opens by objecting to ("it repeats per pixel
  what a whole sprite shares"). Nothing else `Place` carries today survives
  into the row: `Kind` is answered by the attachment channel, not the row,
  and the fraction is answered directly above.
- **Ground's own reconstruction is not the billboard's.** A sloped land quad's
  height is bilinearly interpolated across its four corners — solvable in
  closed form from screen position and the tile's own corner heights, but not
  the free "read the screen row" trick a flat billboard gets. Still per-tile
  data, not per-pixel, just not as trivial as decision 1's paragraph makes it
  look for statics.
- **Whether `SpriteRenderer::render_mask` shrinks.** The id closes the identity
  gap `select.rs` uses it for today, but [`outline.md`](outline.md) D1 already
  leans on that same silhouette pass for its own, independent reason — the
  outline's edge lives outside the sprite's own quad and needs a mask to draw
  against. Removing `select.rs`'s use of it does not mean the pass itself goes
  away; check `outline.md` before assuming it does.

## Steps

Ordered so geometry holds still while ownership moves, and only then may
change shape — decision 38.5's own discipline, cited by name because this file
has already paid for skipping it twice: "a step that changes both where
geometry lives and what it is, is a step where a difference cannot be
attributed."

- [x] 1. Confirm or replace what the depth channel carries — the first "Not
      settled" item above, because everything after it assumes an answer.
      Confirmed: only an ordering key, never read back. See the answered
      item above and `depth::tests::
      priority_z_can_collide_for_two_different_world_heights`.
- [x] 2. Design the face-instance row's layout and the id's width/buffer
      capacity against a real frame's face count, not object count
      (decision 3, "Not settled"). Measured at Britain, widest zoom:
      27,889 ground quads, 6,560 statics (6,991 faces once corners split).
      One 32-bit id channel, `Kind` in two bits selecting one of three
      per-kind storage buffers, 30 bits of id — see the answered item above
      for the row's fields and the buffer-capacity numbers for step 3.
- [ ] 3. Wire the storage buffer: dual-usage (`VERTEX` + `STORAGE`) buffer
      creation, the second bind group on `blit.wgsl`'s fragment stage,
      `instances[id]` (decision 2). No shape change yet — same faces
      `outward()` names today, just addressed by id instead of decoded from
      `Stance` bits.
- [ ] 4. Build the invisible geometry pass: decompose each occluder — a wall
      to its one face, a tread box to its top and riser, a corner to its two —
      and rasterise depth+id from the same instance list and ordering the
      visible passes already use (decision 4). Settle "Not settled"'s
      Solid-sharing question first; this step is where the answer is paid for.
- [ ] 5. Check whether honest per-face lighting alone fixes decision 40's
      original hard-edge report, against the same reproduction that found it.
      If it does, retire `Prism::tread_normal` and `outward()`'s switch rather
      than porting either; if it does not, land the formula's output as one
      face's own row value instead.
- [ ] 6. `select.rs`'s ground wash reads the id instead of tile/stance; measure
      whether `render_mask` still has a reason to run for selection specifically
      once it does, separately from outline's own use of it.
- [ ] 7. Ground's bilinear case (third "Not settled" item), once statics prove
      the shape works at all.

## Backlog

- **Decision 3 said a tread's face count two different ways.** "A tread is a
  box with up to two faces a camera ever sees — its top, and the riser" and
  the worked example built on it (3 treads × 2 faces + the lid = seven) said
  two; a later sentence in the same decision said "a tread has up to three."
  Found while sizing step 2's id against decision 3's own numbers — the two
  could not both be used for the same measurement. Fixed to "up to two,"
  matching the paragraph the worked example is built on; if a tread ever
  turns out to expose a third face (an end tread's open side, unhidden by the
  next one), that is step 4's discovery to make when it actually decomposes
  one, not a number to carry forward from a sentence nothing else in the
  decision agreed with.
