# Cutaway: living plan

> The entry point for architectural transparency.  It records the rendering
> contract, the order of work, and the next concrete hand-off; it is not a
> promise that an object merely hidden by `Cutaway` has become transparent.

## Contract

When an architectural sprite would cover the player's visible picture, the
player remains visible through art at `TRANSLUCENT_ALPHA`.  The wall must still
be lit as the wall, not with the position and normal of the player or floor
behind it.  It must not replace the opaque world's depth, identity, or
G-buffer answer: picking, outlines, and ordinary deferred shading continue to
describe the opaque scene.

The frame therefore has two images:

```
opaque world + opaque G-buffer --deferred--> lit world
             |                              ^
             | depth test (read only)       |
cutaway albedo + cutaway G-buffer --deferred+alpha blend-->
```

The cutaway pass reads the settled opaque depth.  It writes only its private
albedo/G-buffer layer, so a wall behind a body is rejected while a wall in
front of it can be blended over the body.  Its private layer uses the same
lighting inputs and static-instance data as the opaque world; its final blit
uses premultiplied source-over blending.

## Phases

| Phase | State | Done when |
| --- | --- | --- |
| C0: candidate and late alpha pass | done | Overlapping architectural sprites are put in a separate, back-to-front list and blend over the mobile without changing main depth or G-buffer. |
| C1: independently lit layer | done | A cutaway sprite has its own position/normal/owner and is deferred-lit before compositing; the player behind it cannot supply its lighting data. |
| C2: one source of opacity | done | The product-facing alpha is supplied once to both the CPU/test contract and GPU; no Rust/WGSL magic-number pair remains. |
| C3: exact selection policy | done | Architecture needs an overlapping opaque body/static texel; foliage deliberately uses its own screen-rectangle canopy rule. |
| C4: dynamic-world policy | done | Dropped server items use the same exact opaque-pixel candidate rule and independently lit translucent layer as map architecture. |
| C5: transition parity | pending | The reference-style temporal per-object alpha ramp is persistent across frames, including its removal semantics. |

## C1 implementation record

Completed 2026-08-12:

1. Carry cutaway `SpriteQuad`s with their own `Volume` ranges and owner IDs;
   repair these ranges when map statics and server items are joined.
2. Allocate a private world texture and a private `Gbuffer` with the camera
   image.  Recreate both with the ordinary world target.
3. Render cutaway statics with the ordinary static fragment logic into that
   private layer.  Attach the settled opaque depth read-only so it is a
   visibility test rather than a writer; clear only the private colour planes.
4. Add an alpha-aware deferred-blit variant.  It must shade the private layer
   from its private G-buffer, premultiply the resulting radiance once, then
   source-over blend it onto the already-lit surface.
5. Keep masks, selection, text, and the main G-buffer on the opaque depth
   path.  A cutaway is a visual layer, never a new pick/outline target.
6. Prove it in GPU tests: source-over composition and no mutation of the main
   G-buffer.  The test is capability-gated because the current local adapter
   cannot render to the project's `Rgba32Float` position attachment. C3's
   candidate decision is covered on the CPU, so it does not depend on that
   adapter capability.

## C3 implementation record

Completed 2026-08-12:

1. The player's packed body frame is copied into a small CPU opacity mask at
   its actual (including fractional walking) screen placement. Equipment is
   intentionally not part of the mask: the policy protects the controllable
   body, not every paperdoll decoration that happens to extend from it.
2. An ordinary architectural static reaches the late layer only when a pixel
   centre lies inside both sprites and both source texels are opaque. The two
   rectangles remain the bounded scan, not the decision. This keeps empty
   corners in a diagonal wall or body silhouette from making an unrelated wall
   translucent.
3. A static hidden by the storey/roof cut still reaches the late layer without
   this overlap check: that is the architectural cutaway's existing rule, and
   C3 narrows only otherwise-opaque occluders.
4. Foliage remains a separate product rule: any overlap of the player's body
   rectangle and a foliage sprite's screen rectangle hard-hides that canopy.
   It is intentionally generous and remains a hard-cut approximation until C5
   adds the reference-style persistent union/fade state. It does not reuse the
   architectural body-mask policy by accident.
5. CPU tests cover both a real opaque overlap moving a wall to the cutaway
   layer and a rectangle overlap confined to transparent static texels leaving
   it opaque.

## Current hand-off

Start at C5: map architecture and dropped server items now share the bounded
opaque-pixel policy and late independently lit layer; foliage retains its
separate hard-cut canopy rule. The remaining work is the reference-style,
persistent per-object transition ramp.

## C4 implementation record

Completed 2026-08-12:

1. Server ground items are dynamic world geometry, not a visual exception: an
   item with at least one opaque texel over the player's body enters the private
   cutaway G-buffer. A cutaway-hidden item does so too, while the absolute draw
   ceiling remains an absolute reject.
2. Their volumes and owner IDs travel with the private rows; joining map and
   server rows repairs private volume offsets and restores one stable
   back-to-front order before alpha composition.
3. The ordinary item path, including its selection identity and highlight,
   remains unchanged. Only the visual row moves to the late layer.
4. A CPU regression test proves a dropped opaque item overlapping the body
   leaves the opaque list for the late layer, while the no-body case remains
   opaque.

## Risks kept explicit

- Transparency composition is order-dependent.  Cutaway rows stay in stable
  back-to-front `depth::Order`; they must not be re-sorted by atlas id.
- A private depth buffer alone is insufficient: it would reveal walls that the
  opaque player or nearer world object should hide.  C1 tests against the
  *main* depth and does not write it.
- Deferred lighting contains more than a colour multiply (normals, own-solid
  shadow rules, flames, sun).  A special flat cutaway shader would drift from
  it; C1 reuses the deferred blit's logic.
- Server items share the architecture candidate rule; any future producer must
  choose explicitly whether it joins that layer or remains a hard cut.
