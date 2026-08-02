# The outline: a sprite with an edge drawn round it

The client already says "the cursor is on this" by hue: whatever `items::pick`
answers is drawn in `items::HIGHLIGHT_HUE`, ClassicUO's own
`HIGHLIGHT_CURRENT_OBJECT_HUE`, replacing the art's colour the way a `hues.mul`
ramp does. That is the reference's whole vocabulary for it and it works.

This plan is the **second** way of saying it, wanted alongside the first rather
than instead of it: an **outline** — a hard one-pixel edge round the sprite's
silhouette first, and later the same edge blurred into a glow. The two must
compose: an item can be hued *and* outlined at once, which is the first thing
this plan decides.

Written against `crates/client/render/`: `items.rs`, `sprite.rs`, `atlas.rs`,
`statics.wgsl`, `renderer.rs`, `blit.rs`, and `crates/client/app/src/lib.rs`'s
`draw`, where the frame is staged.

## The reference is not UO

ClassicUO does not draw outlines. Its whole per-sprite vocabulary is
`ClassicUO.Renderer/ShaderHueTranslator.cs` — `SHADER_NONE`, `SHADER_HUED`,
`SHADER_PARTIAL_HUED`, `SHADER_SPECTRAL`, `SHADER_SHADOW`, `SHADER_LIGHTS` — a
`byte` picked per draw and carried in the third component of the hue vector.

So the reference is worth exactly two things and nothing else:

- **The mode travels beside the hue, not inside it.** A `byte` of its own, not a
  reserved value of the hue. Copy that.
- **`SHADER_SHADOW`** is the silhouette draw — a sprite rendered as a flat
  shape rather than its own colours. Whatever draws the silhouette here is doing
  what that mode does.

There is a reference for the *effect*, just not in UO: **Fallout 1 and 2 outline
objects**, and they do it by per-pixel edge detection at blit time. The sprites
are 8-bit palettized with index 0 transparent, so the shape is already a bitmask;
the engine walks the object's own buffer and, where a transparent pixel is next
to an opaque one, writes the outline colour's palette index. No polygon, no
second asset, no preprocessing — one extra pass over a rectangle grown by a
pixel, with the colour a per-object property (hostile, party member, the
highlight-items hotkey).

That is D5 step 2 with the GPU taken away. The pipeline below is not a different
idea from theirs; what differs is *where* it runs, and that is decided by two
things Fallout did not have — a zoom, and a packed atlas. See D3 and D4.

Everything else below is ours, and the standard techniques are named in
[Techniques](#techniques-what-to-search-for) rather than invented here.

## The decisions

Numbered so one can be argued with alone. None of them is implemented yet.

### D1 — the outline is a pass, not a flag on the static pass

The tempting version is a bit on `SpriteQuad` and a branch in `statics.wgsl`.
It does not work, for a reason that is geometry rather than taste: **an outline
lives outside the sprite's own quad.** The rectangle is the art's exact size
(`Sprite::width`/`height`, see `statics::stand_on`), so an edge drawn round the
silhouette has nowhere to land.

So: a **pass of its own**, drawing only the highlighted sprites. That is also
the only shape glow can be added to without being rewritten (D5).

**Built, and where it runs is now fixed:** the silhouette pass is
`SpriteRenderer::render_mask`, between the mobile pass and the text pass; the
ring is `outline::Outline::render`, after the blit.

### D1a — the ring does not shine through what is in front of it

This was written as a consequence of D1 ("with the depth test off") and it is
not one, it is a choice, so it is numbered separately.

The silhouette pass **tests the world's depth buffer and does not write it**. So
the mask holds the id of whoever is *visible*, and a barrel behind a shopfront is
ringed only where the barrel can be seen — the ring is occluded exactly as the
picture is. Fallout does the same thing for the same reason, and it falls out for
free rather than costing a pass: the ordering was settled by the passes that drew
the picture, and the mask is the record of that decision.

Depth *write* is off because the ordering must not be settled twice; the text
pass draws at the near plane afterwards and would otherwise punch the mask
through, which is why the mask pass runs before it.

### D2 — no mode field: the list is the numbering

**Withdrawn.** The plan wanted one more `u32` on `SpriteQuad` so a sprite could
say "outline me". It is not needed, and adding it would have touched every
construction site in the crate for a fact about one sprite in a frame.

The silhouette pass takes **only the sprites to be outlined**, as their own list
(`items::outlined`). A quad's identity is then its position in that list, read in
the shader as `instance_index + 1` — zero staying free for "nothing here". No
field, no bits stolen from the hue, and hue and outline compose because they are
two passes over different pixels rather than two flags on one instance.

The ceiling is `outline::MAX_OUTLINED` — 255, the mask being one byte. Past it
the tail is dropped rather than wrapped: an id that wrapped onto another
object's would ring the two as one, silently.

### D3 — the atlas has no padding, and it turns out not to matter

**Corrected.** The claim was that *every* outline technique samples neighbouring
texels, so a highlighted barrel would be outlined in whatever art `Shelf::take`
packed beside it. That is true of two techniques and false of the one this plan
chose.

In D5's pipeline the atlas is sampled with the picture pass's own UVs, strictly
inside the sprite's `Region`. The growing happens **in the mask**, where a
neighbour is a neighbouring pixel of the screen and not a neighbouring sprite in
the atlas. So the packing needs no border and no clamp, and the adjacent-sprite
test would be pinning a property nothing depends on.

It comes back the moment either of these is wanted, and both are listed under
[Techniques](#techniques-what-to-search-for):

- **the offset-draw silhouette** (the sprite drawn 4 or 8 times, shifted, as a
  flat colour), which samples the atlas *at an offset* by construction;
- **baking the edge into the art at load time**, which dilates in texel space.

If either is built, the fix is a one-texel transparent border in `Shelf::take`
(`region_at` and `Packed::origin` shift with it) or a clamp to the `Region` in
the shader — and then the adjacent-sprite test this section originally asked for.

### D4 — thickness is one *virtual* pixel, not one screen pixel

**Reversed, deliberately.** The plan argued that a "one pixel" outline drawn in
the world image is four screen pixels at 4x, "which is not what a one-pixel edge
means to anybody looking at it". On reflection the opposite is true for pixel
art: a one-*screen*-pixel hairline round a sprite drawn at 4x is finer than any
edge in the picture it is tracing, and reads as a rendering artefact rather than
as a highlight.

So the mask is the world image's size and the ring is grown in its texels, which
are virtual pixels; the blit's nearest sampler magnifies the ring together with
the art, blockily and in step with it. This is also what makes the pipeline
cheap: no second resolution, no rescaling of a radius, and the mask can share the
world's depth buffer — which it must, since a depth attachment has to match its
colour attachment's size.

The composite still runs **after** the blit, for a different reason than the plan
gave: not resolution but lighting. A ring drawn into the world image would be
multiplied by the night, and a highlight that dims exactly when the picture is
hardest to read is a highlight that stops working.

What this costs is at *minification*: below 1x a one-texel ring is thinner than a
screen pixel and starts to break up. `Ring::width` searches a denser
neighbourhood and is the knob; nothing sets it from the zoom yet — see the
backlog.

### D5 — one mask, two effects

The pipeline that makes the pixel outline and the glow the *same* work:

1. **Silhouette into a mask.** A target the size of the world image; every
   sprite to be outlined drawn into it in its own id (D2). This is
   `SHADER_SHADOW`'s draw.
2. **Grow it.** A neighbourhood test on the mask: a fragment is a ring texel of
   object *N* when some neighbour is *N* and the fragment itself is not.
3. **Composite.** That ring, in the ring's colour, alpha-blended over the
   surface.

Steps 2 and 3 are **one pass**, not two: the grow is nine texel loads in the
composite's own fragment shader, so there is no second target and no second
draw. A separate dilation target buys nothing until the blur arrives.

**The mask holds an id, not a coverage bit**, and that one choice is what keeps
two rings apart. With coverage the second half of the rule reads "and the
fragment is empty", so where two outlined sprites touch there is no ring between
them and the pair comes out ringed as one blob. With ids the boundary between
two outlined objects is a ring on both sides of itself. It costs the same byte
and the same nine taps.

The glow is step 2 with a blur in it and step 3 with additive blending. Nothing
in 1 or 3 changes, which is the whole reason for doing it in this order: **ship
the pixel outline, and the glow is one shader later.**

### D6 — what decides *which* sprites are outlined stays where it is

`items::pick` already answers "which item", `App::world_owns_pointer` already
answers "may the world read the cursor", and `draw` already asks both once per
frame. The outline consumes that answer and adds nothing: no second pick, no
per-item flag, no highlight state kept between frames.

`items::outlined` and `items::collect` build their quads through one
`items::quad_of`, so the silhouette lands on the picture rather than beside it.
Two copies of that arithmetic would be a ring half a pixel off its sprite, and
nothing in either copy would look wrong.

Ground items only, for now, for the reason the picking has: statics are not
entities and mobiles are the paperdoll arm of `0x06`. Both are listed in
`docs/client.md`'s M5 backlog and neither changes anything here — the pass takes
a list of quads and does not care where they came from.

## Techniques: what to search for

- **Pixel outline, one pass:** *sprite outline shader alpha dilation*, an 8-tap
  neighbourhood max over alpha. This is D5 step 2 at its simplest.
- **Silhouette by repeated offset draws** (draw the sprite 4 or 8 times, offset
  by a pixel, as a flat colour, then the sprite on top). The cheapest thing that
  works and the one that needs no mask target — but it samples the atlas at an
  offset, so D3 applies to it exactly as much.
- **Thick or uniform outlines:** *jump flood algorithm outline*, *signed
  distance field outline*. Worth it only once a thickness above two pixels or a
  smooth falloff is wanted; the JFA output is also a free distance field for the
  glow.
- **Glow:** *kawase blur*, *dual filter blur*, separable gaussian. Two passes
  over the mask, then additive composite.
- **Stencil-buffer outline** is the other classic answer and is listed for
  completeness: it draws the ring without a mask target, at the cost of a
  stencil attachment on the world pass and a second draw of every highlighted
  sprite.

## Steps

- [x] ~~D2: the mode field on `SpriteQuad`~~ — withdrawn, see D2.
- [x] ~~D3: pick padding or clamping~~ — not needed for this pipeline, see D3.
- [x] D5 step 1: the mask target (`outline::mask_texture`) and the silhouette
      draw (`SpriteRenderer::render_mask`, `silhouette.wgsl`), sized like
      `Screen.world` and resized with it.
- [x] D5 steps 2–3: the neighbourhood test and the composite, in one pass
      (`outline::Outline`, `outline.wgsl`).
- [x] Frame tests. `a_ring_is_drawn_around_a_silhouette_and_not_over_it` pins
      the ring's shape both ways — the border is ringed and the sprite is not —
      and `two_touching_silhouettes_are_ringed_separately` is what an id mask
      buys over a coverage one.
- [ ] The switch: hue highlight, outline, or both — a field on the HUD's request
      (`shell::Request`), so it can be looked at before it is a setting. Both are
      drawn today.
- [ ] The glow: `Ring` grows a blur radius, the composite gains a downsampled
      kawase pass and additive blending. Steps 1 and 3 do not change.

## Backlog, in advance

- **Nothing else on screen is outlined yet, and the moment a mobile is, the
  pass needs the mobile atlas too.** It is a second texture bound to the same
  pipeline, not a second pipeline; worth knowing before the mask draw is
  written against `StaticAtlas` alone.
- **The ring does not thin gracefully under minification.** `Ring::width` is
  fixed at one mask texel, which is one *virtual* pixel — below 1x that is less
  than a screen pixel and the ring starts to break up. The knob exists and
  searches a denser neighbourhood; what is missing is driving it from
  `Zoom::numerator`/`denominator`, and a test that says a minified ring is still
  continuous. See D4.
- **Both highlights are drawn at once.** A pointed-at item is redrawn in
  `HIGHLIGHT_HUE` *and* ringed. The two were designed to compose and they do,
  but nothing chooses between them yet — see the switch in Steps.
- **The mask is allocated for every frame, whether anything is outlined or
  not.** One byte per world pixel, cleared each frame by a pass that usually
  draws nothing. Trivial next to the world image, and worth remembering only if
  the mask ever grows a channel.
- **The click still picks against a camera it reads back from `self.control`**
  (`App::use_under_cursor`), while the highlight picks against the frame's own.
  See `docs/client.md`'s M5 backlog: the outline makes this more visible, since
  what is lit and what is used would then differ by a whole visible ring.
