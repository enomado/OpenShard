# Client map-block LOD

Map-block LOD is selected from the block's projected physical-pixel footprint,
not from a camera zoom rung. A UO map block is 8×8 tiles; with the client's
44-pixel isometric tile width, its zero-height ground diamond is 352×352
virtual pixels. The physical size is therefore `352 * zoom_numerator /
zoom_denominator`. This is deliberately the viewport scale, not a minified
offscreen render-target extent.

`openshard_client_render::lod` starts every block at LOD 0 (the existing
per-tile ground/static renderer). The cache uses LOD 1 and LOD 2 only for
immutable map ground and map statics. Server items, mobiles, effects,
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

## Recovery plan: an independent block producer

The camera-frame capture route is currently intentionally not shown. It may
still be useful as a diagnostic, but it cannot be the source of a visual LOD:
its rectangle is derived from a viewport that contains only part of the world,
and its ownership holes have no detailed fallback after a block replacement.

The replacement design has five ordered stages. A later stage must not be
enabled until the preceding stage has its test and acceptance condition.

**Current status.** Stages 1–4 are implemented: `CompositeProducerJob` derives
a fixed 864×864 local camera and source extent solely from a composite key,
`Screen` owns matching private attachments, and a map-only command buffer
renders ground/map statics/mesh faces there before filling all cached planes.
The queue prepares one job's full clipped producer-camera input range
append-only, marks it ready only on success, and never dispatches an
unprepared job. `CompositeProducerJob::rect_in` is the one transform used both
for producer source and visible restoration; its tests cover adjacent blocks
and zoom modes. The real-install/GPU oracle
`real_map_block_producer_keeps_every_owned_map_tile_after_restore` has passed
against the development client: it reads the producer and restored colour, IDs
and positions for a dense central-Britain block and proves all 64 map tiles
(land or their covering map static) and every owned map-static tile survive
LOD1 capture/restore. The direct
`lod-sweep` scenario at `1/2x` now reads the completed cache IDs directly,
before restore, while the reusable producer target is repeatedly redrawn. Each
entry retains its owned land/static pixels; the blank padded texels are
`Kind::Nothing` and the restore shader discards them. The GPU oracle also
overwrites every producer plane for a second job and proves that the first
entry's colour, IDs, position, normal and depth bytes do not change. LOD1 is
therefore enabled; LOD2 remains disabled pending its own minification run.
The Frames HUD shows queue state, retained cache memory and its budget
alongside the independently timed producer GPU pass.
Any block with an animated map static stays on LOD0: its source is not
immutable, so caching one clock tick would be incorrect.
The GPU profile reports `map composite producer` as its own pass, and the
Frames panel reports the cache's retained GPU-byte total and budget.

1. **Define the producer contract.** One job owns one padded 8×8
   [`MapBlock`](../crates/client/render/src/composite.rs): fixed `352 + 2 ×
   256` virtual-pixel extent, fixed local camera, and map-only inputs
   (ground, map statics, their G-buffer planes, and depth). It receives no
   viewport rect and no dynamic rows. The block's source extent is identical
   whether the player is panning, at another zoom, or an atlas grew.
2. **Allocate reusable offscreen attachments.** `Screen` owns a producer world
   texture, depth texture and G-buffer at that fixed extent. A job clears all
   of them, renders its complete block, then writes a `CompositeTexture` from
   those attachments. The producer has a separate command path and never
   samples the main frame's textures.
3. **Prepare immutable inputs ahead of time.** The queue's prefetch window
   prepares map graphics and atlas pages for the requested block before it
   dispatches the offscreen draw. Atlas growth is append-only; it must not
   invalidate completed composites or make a visible camera frame rebuild its
   atlas. A job that is not prepared remains pending and the visible block
   stays LOD0.
4. **Make replacement atomic.** A cache entry is `Ready` only after colour,
   IDs, positions, normals and depth have all been produced. Then, and only
   then, the frame assembler excludes that block's LOD0 ground/map-statics and
   restores its composite. Any miss, cancellation, cutaway, content mutation,
   or invalid plane keeps/reverts the block to LOD0 for that frame; it never
   leaves a cleared rectangle behind.
5. **Turn on tiers and budgets.** Enable LOD1 first, validate it while panning
   and zooming, then add LOD2 downsampling from the same canonical producer
   image. Keep one bounded job per frame initially; raise the budget only from
   measured GPU frame time and cache-memory telemetry.

The completion gate is visual as well as mechanical: at all zoom levels, a
continuous pan across an unprepared/ready boundary must show the same map
pixels with no black region, sprite substitution, or frame-to-frame atlas
rebuild. GPU tests cover private-source capture, atomic Ready-or-LOD0
replacement, ownership at shared boundaries, and the restored depth against a
later dynamic draw. The deterministic
`steady_far_zoom_pan_benchmark_keeps_producer_work_bounded` fixture simulates
256 far-pan frames through the real preparation gate and proves that each frame
hands the producer at most its configured one job for newly entered blocks.

## Composite scheduling

`CompositeWorkQueue` uses map blocks as its cell unit.  At most 128 requests
wait at once and a producer may take one per frame; it is a queue of jobs, not
a place that rasterises pixels.  The app refreshes it from the fixed camera
snapshot after UI layout, so exposing a far-zoom block only changes bounded
queue state and never synchronously composes that block in the camera frame.
The former capture path sampled the already-drawn map-only attachments before
server items and mobiles. It is retained only as non-visible diagnostic code;
the displayed renderer is held at LOD0 while producer source coverage is
validated. A background producer may return `CompositePixels` through
`finish_into_cache`. A result that was not dispatched by the queue is rejected.

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
cache contain immutable map ground and statics only. Static-atlas page growth
does not change a composite key: pages are append-only and an entry holds final
pixels rather than atlas UVs. Its padded 8×8 block rectangle is a fixed
cache-format extent (256 source pixels of static overhang), and capture filters
by G-buffer tile ownership so overlapping padded rectangles cannot restore a
neighbour's pixels. Deferred restore samples colour nearest-neighbour as well
as ownership IDs: filtering a valid edge texel against a transparent texel
discarded for its neighbour would otherwise create a moving dark seam. Each
distinct source-depth base in one command encoder has a separate deferred
viewport/instance binding slot; queue writes to a later group must never alter
an earlier group's draw. More importantly, ownership starts in the producer:
its padded camera provides occlusion context, while its ground and map-static
lists are collected only from the entry's own 8×8 tiles. The capture filter is
therefore a defensive assertion rather than the operation that assigns pixels
to a block. During cutaway, cache restore and capture are both bypassed
because the normal map attachments omit the cut-away rows; dispatched jobs are
released to be scheduled again on the next ordinary frame.

## Handoff: LOD1 rollout, LOD2 held back

**Current safety state (2026-08-13).** The independent producer, cache, queue,
LOD selection and telemetry code are live. `App::draw_from` enables LOD1 and
caps selected LOD2 at LOD1. The injected `--scenario lod-sweep` diagnostic
reaches `1/2x`, pans through block boundaries without desktop input, and can
be run with `OPENSHARD_COMPOSITE_AUDIT=1` to read each completed cache ID plane
before restore. LOD2 is disabled.

The prior failure was an invalid readiness contract: a texture became drawable
once deferred planes were recorded, while the renderer assumed those planes
covered every detail row it removed. The real-map oracle now exercises that
exact producer/capture/restore path against dense statics. A cache miss,
unprepared job, cutaway or animated block still remains LOD0 for that frame.

### Relevant code

- `crates/client/app/src/presentation.rs`: LOD1-only rollout cap, producer
  invocation and its fixed local camera.
- `crates/client/app/src/window.rs`: `prepare_composite_job`, including the
  clipped full producer-camera atlas-input bound.
- `crates/client/render/src/composite.rs`: fixed producer contract, capture,
  ownership filter, deferred restore and synthetic GPU oracle.
- `crates/client/app/src/render_passes.rs`: the point at which a ready block
  excludes LOD0 ground/statics and draws its deferred composite.

### Required investigation and fix order

1. **Done: real-map producer oracle.** The gated
   `frame::real_map_block_producer_keeps_every_owned_map_tile_after_restore`
   test uses real map/land/static atlases and the actual producer camera. It
   reads source and restored colour, IDs and positions and asserts both own
   every land and map-static tile in a dense block after LOD1 capture/restore.
   It passed with the development client. Re-run it with
   `OPENSHARD_CLIENT=/path/to/client cargo test -p openshard-client-render --test frame real_map_block_producer_keeps_every_owned_map_tile_after_restore -- --exact`
   after changing the producer contract or its capture shaders.
2. **Done: prepare every producer source input.** The target is padded by 256
   pixels on each side, so `prepare_composite_job` now grows atlases over
   `CompositeProducerJob::source_tiles()` after clipping it to the map. The
   animated-static eligibility check remains limited to the owned 8×8 block:
   padding tiles are discarded by ownership filtering and must still be able to
   remain dynamic in their own detailed block.
3. **Done: share and test the transform.**
   `CompositeProducerJob::rect_in(camera)` replaces the separate
   `render_passes::block_rect()` calculation. The renderer test proves that
   producer/source and visible rectangles agree, and that east/south adjacent
   blocks retain their projected offsets through zoom and minification.
4. **Done: conservative replacement.** Only a ready deferred composite
   excludes LOD0 geometry. Any cache miss, input-preparation failure, cutaway,
   mutation or animated source keeps the detailed path; no cache allocation by
   itself can suppress it.
5. **In progress: LOD1 field run; LOD2 held back.** The direct `lod-sweep`
   now verifies the texture state rather than inferring it from a screenshot:
   cache entries retain their owner IDs while the next producer job redraws all
   source planes. LOD1 is enabled. Continue the scenario over ordinary terrain,
   map edges, dense/tall statics and animated statics; the Frames HUD must show
   bounded queue/cache values, no atlas rebuild every frame, and no black,
   holes or shifted map pixels. Enable LOD2 only after that run is clean.

Existing tests cover synthetic ownership boundaries, depth interaction,
producer sizing and bounded queue work; the gated real-map oracle covers
end-to-end LOD1 map-ground and map-static ownership. It is deliberately not a
substitute for the sustained interactive field run required before LOD2.
