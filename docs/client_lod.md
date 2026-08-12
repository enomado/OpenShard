# Client map-block LOD

Map-block LOD is selected from the block's projected physical-pixel footprint,
not from a camera zoom rung. A UO map block is 8×8 tiles; with the client's
44-pixel isometric tile width, its zero-height ground diamond is 352×352
virtual pixels. The physical size is therefore `352 * zoom_numerator /
zoom_denominator`. This is deliberately the viewport scale, not a minified
offscreen render-target extent.

`openshard_client_render::lod` starts every block at LOD 0 (the existing
per-tile ground/static renderer). The planned cache will use LOD 1 and LOD 2
only for immutable map ground and map statics. Server items, mobiles, effects,
selection, cursor picking, and UI stay on their existing paths; the source map
remains authoritative for all game logic.

| Transition | Zooming out: enter at or below | Zooming in: leave at or above |
| --- | ---: | ---: |
| LOD 0 ↔ LOD 1 | 192 px | 224 px |
| LOD 1 ↔ LOD 2 | 96 px | 112 px |

The gaps are hysteresis bands. While projected size remains inside a band, a
block keeps its prior level, preventing small resize, scale, or zoom changes
from swapping composite and detailed paths each frame. A large change may skip
directly to its settled tier. Thresholds are validated as strictly ordered, so
an inverted policy cannot make per-frame alternation expressible.

## Composite scheduling

`CompositeWorkQueue` uses map blocks as its cell unit.  At most 128 requests
wait at once and a producer may take one per frame; it is a queue of jobs, not
a place that rasterises pixels.  The app refreshes it from the fixed camera
snapshot after UI layout, so exposing a far-zoom block only changes bounded
queue state and never synchronously composes that block in the camera frame.
An idle producer returns its finished `CompositePixels` with the same key to
`finish_into_cache`, which atomically uploads it to the Work 2 cache and
releases the in-flight queue slot.  A result that was not dispatched by the
queue is rejected.

Jobs are stable-ordered by category, distance and key: visible blocks first,
then one viewport-sized rectangle ahead of block-level camera movement.  A
reversal drops unstarted work from the old direction; a completed or in-flight
exact `(block, tier, immutable revision)` is not requested again.  The queue
does not prescribe cancellation of in-flight jobs, because a producer may
already have touched its source data.

When the selected LOD 2 texture is not ready, the draw policy may use a ready
LOD 1 texture for the same block and immutable revision.  When LOD 1 is not
ready either, it continues through LOD 0.  Thus a newly visible block becomes
more detailed temporarily rather than forcing the large composite to be built
in the camera frame.  The source map remains authoritative; the queue and
cache contain immutable map ground and statics only.
