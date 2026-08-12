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
| C3: exact selection policy | pending | The coarse screen-rectangle shortlist is replaced or bounded by a documented body mask/radius policy; foliage has its own policy, not an accidental reuse. |
| C4: dynamic-world policy | pending | Dropped server items are deliberately either included in the translucent layer or declared hard-cut, with a test. |
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
   cannot render to the project's `Rgba32Float` position attachment.  The
   night/light-fixture differential remains a C3 follow-up test once exact
   candidate selection has its policy.

## Current hand-off

Start at C3: the implementation deliberately retains the existing
screen-rectangle shortlist.  Choose and document the product policy for a
body mask/radius and foliage; only then narrow the candidate generator and
add the associated light-fixture regression case.  Dynamic dropped items stay
hard-cut until C4 chooses their policy.

## Risks kept explicit

- Transparency composition is order-dependent.  Cutaway rows stay in stable
  back-to-front `depth::Order`; they must not be re-sorted by atlas id.
- A private depth buffer alone is insufficient: it would reveal walls that the
  opaque player or nearer world object should hide.  C1 tests against the
  *main* depth and does not write it.
- Deferred lighting contains more than a colour multiply (normals, own-solid
  shadow rules, flames, sun).  A special flat cutaway shader would drift from
  it; C1 reuses the deferred blit's logic.
- Dynamic items have no chosen product policy yet.  They remain excluded until
  C4 makes that decision explicit.
