# Client map-block LOD

Map-block LOD is selected from the block's projected physical-pixel footprint,
not from a camera zoom rung. A UO map block is 8×8 tiles; with the client's
44-pixel isometric tile width, its zero-height ground diamond is 352×352
virtual pixels. The physical size is therefore `352 * zoom_numerator /
zoom_denominator`. This is deliberately the viewport scale, not a minified
offscreen render-target extent.

`openshard_client_render::lod` starts every block at LOD 0 (the existing
per-tile ground/static renderer). The cache uses LOD 1 and LOD 2 only for
immutable flat map ground. Sloped land and every map static remain in the
live LOD 0 layer: slopes depend on the adjoining height field, while a roof
can rise outside its base block's finite source image. Server items, mobiles, effects,
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

**Current status.** Stages 1–4 are implemented with one owner per source:
`CompositeProducerJob` derives a fixed 352×352 local camera and source extent
solely from a composite key. `Screen` owns matching private attachments and
the map-only command buffer renders only a wholly flat 8×8 ground block there
before filling all cached planes. A block containing any slope remains LOD0:
the slope and its neighbouring flat diamonds are one depth-comparable region,
so splitting their owners between a cached pass and a live pass is not safe.
For a fully flat elevated plateau, the producer and restore rectangle use that
plateau's shared `z`, rather than treating the 352-pixel source as sea level;
otherwise the fixed source would clip the top or bottom of its diamonds.
That eligibility result is carried as one immutable `FlatGroundBlock` from
atlas preparation through producer capture into the cache entry: the queue
will not rediscover a possibly different height or transform after dispatch.
The queue prepares only land/texmap inputs,
marks a job ready only on success, and never dispatches an unprepared job.
`CompositeProducerJob::rect_in` is the one transform used both for producer
source and visible restoration; its tests cover adjacent blocks and zoom
modes. Map statics, including animated statics and high roofs, have exactly
one live renderer owner and depth-test after cached ground. LOD1 is enabled
again after repairing the deferred-id route; LOD2 remains held at LOD1 while
the live oracle validates the lossless tier. A cache mismatch falls back rather
than leaving a hole or borrowing a frame-local row. The Frames HUD shows
queue state, retained cache memory and its budget alongside the independently
timed producer GPU pass.

The producer has its own `GroundRenderer` stream: its pipeline bind group,
uniform buffer, unit quad and instance buffer are distinct from the camera
frame's. Both streams retain handles to the same GPU land/texmap textures, so
an atlas row upload is still a single update seen by both. This split is
intentional: queue submissions for a background block and a camera frame are
independent, while the input instance rows for those two draws must never be
overwritten by one another.

The full-frame field oracle renders the same immutable scene again at LOD0
after the real frame has submitted, then compares every immutable-map pixel's
raw colour, semantic ID, world position and lit result. An empty actual pixel
where LOD0 contains map geometry is an error too; this is what makes a black
cache seam visible to the oracle rather than merely absent from its selection.
`OPENSHARD_LOD_FRAME_ORACLE=1` enables it per diagnostic frame. For the real
network case, `--scenario live-oracle` starts normally connected to the shard,
waits at the default zoom, zooms out, leaves all server/NPC animation traffic
enabled, and writes a direct-LOD0 comparison every two seconds to
`/tmp/openshard-frame/live-oracle/`. It is an observation of the live client,
not a synthetic replacement for that traffic.
When this oracle sees an actual `Nothing` pixel where direct LOD0 has land,
it records the responsible source block and quarantines only that block from
the composite cache for the rest of the session. The following frame therefore
uses LOD0 for that block instead of cycling a known-bad texture back onto the
road; unaffected blocks remain cached.
It found the remaining sparse churn in sloped land and tall roofs. Slopes had
neighbour-dependent rasters, and a roof at `z=93` exceeded the former 256-px
static margin, leaving an underlying neighbouring ground pixel after its block
was marked cached. Keeping slopes and all map statics live makes the composite
owner a bounded flat-ground source and preserves roof depth ordering.
The direct cache/producer byte audit and atlas CPU/GPU audit remain separate
checks, so this conclusion does not infer texture state from a screenshot.

1. **Define the producer contract.** One job owns one 8×8
   [`MapBlock`](../crates/client/render/src/composite.rs): fixed 352-pixel
   ground extent, fixed local camera, and flat-ground G-buffer planes/depth.
   If any land tile in the block is sloped, the complete block is permanently
   kept on the LOD0 path.
   It receives no static rows, no dynamic rows and no
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
   then, the frame assembler excludes that block's cacheable flat ground and
   restores its composite. Map statics remain live. Any miss, cancellation, cutaway, content mutation,
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
cache contain immutable flat map ground only. Static-atlas growth does not
participate in composite preparation or keys. Its 8×8 block rectangle is a
fixed 352-pixel ground extent. Ownership has one authority: the producer
collects exactly the entry's flat ground rows, while the static pass owns all
map-static rows. Capture preserves those rendered pixels rather than
re-deriving tile bounds from interpolated G-buffer positions. LOD1 retains one cache texel per source
pixel, so colour, ID, position, normal and depth remain an atomic fact. The
still-disabled LOD2 uses conservative representative selection within its
minified footprint. Deferred restore samples colour nearest-neighbour as well
as ownership IDs: filtering a valid edge texel against a transparent texel
discarded for its neighbour would otherwise create a moving dark seam. Each
distinct source-depth base in one command encoder has a separate deferred
viewport/instance binding slot; queue writes to a later group must never alter
an earlier group's draw. More importantly, ownership starts in the producer:
its camera provides the exact ground transform while its flat-ground list is
collected only from the entry's own 8×8 tiles. Sloped ground and map statics
are deliberately retained in the frame's live detail list. Capture does
not add a second per-pixel ownership rule. During cutaway, cache restore and capture are both bypassed
because the normal map attachments omit the cut-away rows; dispatched jobs are
released to be scheduled again on the next ordinary frame.

## Handoff: LOD1 rollout, LOD2 held back

**Current safety state (2026-08-13).** The independent producer, cache, queue,
LOD selection and telemetry code are live. `App::draw_from` enables LOD1 and
caps selected LOD2 at LOD1. The injected `--scenario lod-sweep` diagnostic
reaches `1/2x`, pans through block boundaries without desktop input, and can
be run with `OPENSHARD_COMPOSITE_AUDIT=1` to read each completed cache ID plane
before restore. LOD1 is spatially lossless: it preserves one cache texel per
producer texel, because a deferred G-buffer's colour, ID, position, normal and
depth must remain one atomic fact. LOD2 is disabled.

The prior failure was an invalid ownership contract: the renderer removed
static rows when the fixed-height source image could not contain a tall roof.
The real-frame oracle now exercises the producer/capture/restore path against
the full LOD0 scene. A cache miss, unprepared job or cutaway keeps ground at
LOD0 for that frame; map statics stay live in every case.

### Relevant code

- `crates/client/app/src/presentation.rs`: LOD1-only rollout cap, producer
  invocation and its fixed local camera.
- `crates/client/app/src/window.rs`: `prepare_composite_job`, which prepares
  only the ground/texmap inputs its producer owns.
- `crates/client/render/src/composite.rs`: fixed producer contract,
  conservative capture, deferred restore and synthetic GPU oracle.
- `crates/client/app/src/render_passes.rs`: the point at which a ready block
  excludes cacheable LOD0 ground, draws its deferred composite, then draws all
  live map statics.

### Required investigation and fix order

1. **Done: real-map producer oracle.** The gated
   `frame::real_map_block_producer_keeps_every_owned_map_tile_after_restore`
   test uses real map/land/static atlases and the actual producer camera. It
   reads source and restored colour, IDs and positions and asserts the cached
   ground pixels survive LOD1 capture/restore.
   It passed with the development client. Re-run it with
   `OPENSHARD_CLIENT=/path/to/client cargo test -p openshard-client-render --test frame real_map_block_producer_keeps_every_owned_map_tile_after_restore -- --exact`
   after changing the producer contract or its capture shaders.
2. **Done: prepare every producer source input.** `prepare_composite_job`
   grows only the owned 8×8 block's land/texmap graphics. Static atlas state is
   not an input to this cache, so packet-driven static-atlas growth cannot
   mutate or invalidate a completed ground composite.
3. **Done: share and test the transform.**
   `CompositeProducerJob::rect_in(camera)` replaces the separate
   `render_passes::block_rect()` calculation. The renderer test proves that
   producer/source and visible rectangles agree, and that east/south adjacent
   blocks retain their projected offsets through zoom and minification.
4. **Done: conservative replacement.** Only a ready deferred composite
   excludes LOD0 geometry. Any cache miss, input-preparation failure, cutaway,
   mutation or animated source keeps the detailed path; no cache allocation by
   itself can suppress it.

For the delayed, no-motion report use `--scenario zoom-soak`: it waits for the
real window viewport to settle, renders for three seconds at the opening zoom,
injects up to three zoom-out notches (subject to the GPU texture limit), and
then performs no pan. It logs an error if the camera changes after that point.
Run it with `OPENSHARD_ATLAS_AUDIT=1`,
`OPENSHARD_COMPOSITE_AUDIT=1`, `OPENSHARD_LOD_SCREEN_AUDIT=1` and
`OPENSHARD_LOD_FRAME_ORACLE=1`; those four independent checks cover atlas
bytes, cached producer bytes, visible ground coverage and the final displayed
pixels respectively.
On a live shard, its post-zoom report also lists every authoritative update by
packet kind. `--scenario zoom-soak-freeze-server` runs the same transition but
counts and deliberately does not fold those server updates after zoom; it is a
diagnostic A/B mode only, for separating packet-driven scene changes from a
cache or texture mutation.

When an intermittent roof artifact is visible in an ordinary session, press
the status-strip **capture GPU dump** button (or `F12`). It arms the next
ordinary world frame and writes its rendered world/G-buffer planes, instance
buffers, `inputs.txt` and `lod-oracle.txt` into
`OPENSHARD_FRAME_DUMP_DIR/frame-N` (default:
the system temporary directory's `openshard-frame/frame-N`). The log reports
the exact directory; archive that complete directory rather than taking a
screen shot. The same one-shot action also performs atlas CPU/GPU readbacks,
visible-tile coverage and a complete displayed-pixel comparison with fresh
LOD0; its log names any mismatched pixels and source blocks.
5. **In progress: LOD1 field run; LOD2 held back.** The direct `lod-sweep`
   now verifies the texture state rather than inferring it from a screenshot:
   cache entries retain their owner IDs while the next producer job redraws all
   source planes. LOD1 is enabled and lossless; only LOD2 may minify source
   texels. Continue the scenario over ordinary terrain,
   map edges, dense/tall statics and animated statics; the Frames HUD must show
   bounded queue/cache values, no atlas rebuild every frame, and no black,
   holes or shifted map pixels. `OPENSHARD_LOD_FRAME_ORACLE=1` additionally
   compares every displayed cached pixel to a fresh full-LOD0 render (including
   the lighting branch). The slow max-zoom pan now matches after slopes remain
   live. Enable LOD2 only after that run is clean.

Existing tests cover synthetic ownership boundaries, depth interaction,
producer sizing and bounded queue work; the gated real-map oracle covers
end-to-end LOD1 map-ground ownership. It is deliberately not a substitute for
the sustained interactive field run required before LOD2.
