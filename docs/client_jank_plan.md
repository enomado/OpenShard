# Client jank: plan of work

## Current handoff — 2026-08-13

### What the current evidence says

The latest playground log showed two separate jank sources:

- The diagnostic Dev HUD can add about 118–123 ms to `ui_hud`. `ui_layout`
  stays around 1.5–3.7 ms and `ui_paint` around 0.36–0.50 ms, so egui layout
  and painting are not the source of that spike.
- The detailed world path costs about 28–34 ms in the same scenario. Geometry
  is about 14–17 ms, `static_walk` about 8.6–10.7 ms, ground about 4.7–5.0
  ms, and instance encoding about 10–14 ms. These CPU phases overlap in the
  assembly/encoding instrumentation and must not be summed as a frame total.
- GPU timestamps are about 1.8–7 ms and did not exceed the 16 ms budget in the
  observed run.
- Atlas growth caused additional spikes, including roughly 114 ms while
  uploading about 10 MB in an early frame and a later 5.12 MB upload. No
  observed frame had `repacked=true`.

### Completed since the baseline

- Jank records now split UI preparation into `ui_hud`, `ui_layout` and
  `ui_paint` in addition to the existing world phases.
- `ui_hud` now records `ui_terrain`, `ui_route`, `ui_occluders`, `ui_picking`
  and `ui_perf` sub-phases, so diagnostic overlay preparation can be measured
  without attributing the whole HUD cost to terrain.
- The terrain diagnostic overlay cache is keyed by visible tile bounds and
  player position, rather than the full pixel-precise camera. Camera motion
  within the same tile bounds therefore reuses the existing world-space
  overlay. World item changes already invalidate this cache.
- `cargo check -p openshard-client-app`, formatter checks, and the app unit
  tests pass.

### Remaining implementation order

1. Re-run the playground with the terrain overlay enabled while moving and
   compare `ui_hud` before/after the bounds-keyed cache. Keep the overlay
   enabled for the measurement; disabling it is only a control run.
2. Use the new HUD sub-phase records to identify which diagnostic path remains
   dominant before changing the terrain implementation.
3. Add a reusable static terrain surface index. For each map tile, retain the
   land/platform surface candidates and static fit data needed by
   `spawn_z`/`can_fit`; avoid allocating and rescanning map statics for every
   overlay tile. Build it lazily or in bounded visible blocks rather than
   eagerly materialising the whole facet.
4. Make the overlay incremental. Keep cells keyed by tile and a terrain
   revision; when the viewport moves, compute only entered rows/columns.
   Invalidate affected cells when the player height, map/static revision or
   dynamic blocker revision changes. Preserve world-space points so camera
   subpixel motion never invalidates them.
5. Collapse the remaining duplicate surface/fit work into one query over the
   cached candidate list. The dynamic `Clutter` lookup should remain separate
   and O(1)-by-tile, so moving server items invalidates only affected cells.
6. Add a benchmark fixture for a fixed camera, a one-tile pan, and a moving
   camera with the overlay on. Record overlay-cache hits/misses, cells rebuilt,
   `ui_hud`, `static_walk`, `encode`, and total build time.
7. Continue with the existing world bottlenecks: incremental static geometry,
   persistent instance buffers/uploads, far-zoom composites, and bounded
   atlas-page uploads. The current goals are below 2 ms for steady-state
   `static_walk`, below 3 ms for `encode`, and no atlas-repack hitch above
   50 ms.

### Handoff notes

- `target/openshard-playground-jank.log` is diagnostic output, not a checked-in
  benchmark artifact. Start a fresh playground run before comparing numbers.
- `ui_hud` includes `App::hud`, so it includes diagnostic data preparation as
  well as the overlay objects later consumed by egui. The new sub-timers in
  step 2 are required before attributing all of that cost to terrain.
- The working tree contains pre-existing changes unrelated to this handoff;
  preserve them when implementing the remaining items.

### Backlog — findings to verify

- The bounds-keyed terrain cache has not yet been measured in a fresh
  playground run. Capture a control run with the overlay off, then compare
  overlay-on movement before and after the cache; do not compare against the
  stale `target/openshard-playground-jank.log`.
- The new HUD records are sequential sub-phases inside `ui_hud`; they explain
  where HUD preparation spends time, but must not be added to `ui_hud` or to
  the frame total a second time.
- `ui_picking` currently includes the remaining HUD snapshot work after the
  named terrain, route and occluder queries, including selection resolution,
  health bars and the goal tile. If it dominates, split those readers before
  changing terrain caching.
- The terrain query still constructs `Clutter` and repeats surface/fit work
  across visible tiles on a cache miss. This keeps the static surface index,
  incremental cells and one-query fit consolidation as the next implementation
  backlog rather than treating the bounds cache as the final fix.

## Context and baseline

The playground writes every frame over the 16 ms budget to
`target/openshard-playground-jank.log` when started with:

```sh
cargo run -p openshard-playground
```

The log separates UI, CPU scene construction, GPU passes, atlas work and the
major geometry stages. The measurements below are from a debug build at a far
zoom-out while scrolling Britain.

| Phase | Typical cost | Notes |
| --- | ---: | --- |
| CPU build | 28–30 ms | Stable far-zoom frame, excluding atlas growth. |
| GPU | 6–7 ms | Not the limiting side. |
| Ground collection | 4.5–4.8 ms | Was about 8 ms before diagonal ordering. |
| Static collection | about 7.7 ms | About 6 ms walk and 1.7 ms stable ordering. |
| Instance encoding/upload | about 12 ms | Ground and sprites are rebuilt and sent each frame. |
| Full atlas repack | 400–430 ms | The visible scroll hitch. |

Completed work: route facts are cached after a destination is set; static
picking is narrowed to the cursor's conservative tile bounds; ground ordering
does not globally sort the whole frame; ordinary renderer staging buffers are
reused; and the jank log records the phases above. A trial replacement of the
stable static sort was measured and rejected because it regressed the far-zoom
case.

## Session 1 — eliminate atlas-repack hitches

### Goal

Scrolling must not synchronously rebuild all atlas pixels after an atlas fills.

### Work

1. Extend the jank record with the atlas that overflowed, its packed graphic
   count, newly requested graphic count and uploaded byte count.
2. Add a reproducible playground scroll path that crosses enough map tiles to
   fill the current static atlas.
3. Design atlas pages: keep full pages immutable and allocate a new page for
   new graphics instead of evicting and re-reading every visible graphic.
4. Update the sprite renderer to select the relevant page per instance (texture
   array if device support and format limits permit it; otherwise a bounded set
   of page bind groups/batches).
5. Retain the existing one-page implementation as the baseline until image,
   selection and g-buffer tests cover the paged path.

### Atlas-page decision (Work 3)

`StaticAtlasPages` keeps the established 2048×2048 RGBA8 shelf format as an
immutable sequence. The active page accepts ordered new images until the next
one would not fit; that page is then sealed, and the remaining images start a
fresh page. Lookups return both a `StaticAtlasPage` and the page-local sprite,
while dirty uploads are reported as `(page, rows)`. Thus filling a page neither
evicts nor re-reads graphics still visible on older pages.

The first path retains at most eight pages: 8 × 2048 × 2048 × 4 = 128 MiB of
static texture pixels. Work 4 uses bounded per-page bind groups/batches rather
than a texture array, so it stays within WebGL2 texture-array limits. Instances
retain their source order and change binding only at page runs, preserving
equal-depth ordering; the legacy one-page `StaticAtlas` renderer remains
available as the baseline. Work 5 keeps that legacy renderer covered while a
two-page fixture now verifies page-one image sampling, G-buffer identity, and
CPU pick/selection rows.

### Done when

- a scroll produces no `repacked=true` frame above 50 ms;
- adding graphics uploads only the newly allocated page or changed rows;
- all atlas, static-render and screenshot tests remain green;
- memory use is explicitly recorded for the selected page limit.

## Session 2 — far-zoom LOD and chunk composites

### Goal

At scales where a map block covers only a small screen area, stop constructing
and drawing every ground and map-static quad in that block. The client can zoom
farther out than the classic client, so this is a rendering mode, not a
last-mile optimisation.

### Work

1. Define LOD from projected block size in pixels, rather than one magic zoom
   rung. Give each threshold hysteresis so small zoom or resize changes cannot
   alternate a block between the detailed and composite paths every frame.
2. Keep the present ground/static renderer as LOD 0. Add a cached composite
   texture for a map block at LOD 1/2: it contains only immutable map ground
   and map statics, at the required zoom tier. Draw it as one quad (or a small
   fixed mesh) per visible block.
3. Build and refresh composites through the same bounded, prioritised cell
   queue used for streaming: visible blocks first, then blocks ahead of camera
   movement. A newly visible block may temporarily use its next-more-detailed
   representation; it must never synchronously compose a large block in the
   camera frame.
4. Keep server items, mobiles, effects, selection, cursor picking and UI out
   of the composite. The source map remains the authority for picking and game
   logic, regardless of which visual LOD was drawn.
5. Invalidate only a block and its affected LOD levels for map/static mutation,
   art/atlas revision, cutaway state or an output-format change. Establish a
   bounded GPU-memory policy with an LRU tail outside the viewport hysteresis
   margin.
6. Add fixed-camera screenshot tests around each LOD threshold and regression
    tests that compare map picking and dynamic-object placement with LOD 0.

### Work 5 limits

`CompositeCache` exposes block- and tier-scoped invalidation for map/static
mutation, plus matching cancellation for queued or in-flight work so a late
capture cannot restore stale pixels.  The app invalidates the affected visible
blocks when static atlas content or the cutaway changes; a world-output-format
change clears the cache because a texture cannot cross formats.  Atlas growth
also cancels the bounded prefetch queue, since that queue has no per-graphic
dependency index yet.

The shipped GPU cache tail is capped at 128 MiB.  A completed deferred
composite retains colour plus its deferred planes (eight RGBA-sized planes in
the current implementation).  On every frame the cache evicts least-recently
used entries outside a one-map-block viewport margin.  Visible and margin
entries are protected, so a slow, small pan can temporarily exceed the tail
budget rather than churn its just-left blocks; the maintenance result reports
the protected overage for future diagnostics.

### Done when

- far zoom scales with visible blocks rather than visible tiles/statics;
- a steady far-zoom scroll builds or uploads only entered block composites;
- crossing an LOD threshold produces no visible flashing or per-frame
  rebuilds;
- dynamic entities and picking agree with the detailed renderer.

## Session 3 — incremental static geometry

### Goal

Avoid rebuilding all visible static quads and impostor volumes when the camera
moves by one or a few tiles.

### Work

1. Split `static_walk` further in a benchmark-only profile into map walk,
   placement/culling, occlusion lookup and volume construction. Do not put an
   `Instant` around every static in production frames.
2. Cache static geometry by map block and atlas revision. A cache entry owns
   its quads, volumes and a conservative visibility bound.
3. On scroll, remove blocks outside the visible bounds and build only newly
   entered blocks; concatenate cached block output in the exact depth order.
4. Invalidate an entry for atlas revision, cutaway/fade state, static animation
   frame or world/item mutation as appropriate. Keep dynamic server items out
   of the map-static cache.
5. Establish byte-for-byte frame-output tests for a full rebuild versus the
   incremental path at fixed camera positions.

### Done when

- far-zoom `static_walk` is below 2 ms during a steady scroll;
- equal-depth map statics preserve their existing file order;
- no stale sprites, volume ranges or cutaway rows remain after invalidation.

## Session 4 — incremental ground and instance uploads

### Goal

Remove the per-frame CPU encoding and GPU upload of the full visible world.

### Work

1. Add upload counters to the jank record: bytes and instance count for ground,
   opaque statics, cutaway statics and volumes.
2. Store cacheable ground/static instances in persistent GPU buffers, indexed
   by visible map blocks or viewport strips.
3. On camera movement, upload only entered strips and update a small camera
   transform/uniform for the common movement case.
4. Keep a safe full-rebuild fallback for resize, zoom-rung change, atlas-page
   change and cache invalidation; test it against the incremental buffer state.
5. Re-evaluate the ground collector after this change. Its current 4.5–4.8 ms
   is no longer the first bottleneck once most instances are retained.

### Done when

- far-zoom `encode` is below 3 ms;
- normal scroll transfers proportional-to-edge data, not full viewport data;
- ground collection is below 1.5 ms on the same profile;
- GPU frame time does not regress above the current 6–7 ms baseline.

## Session 5 — performance guardrails

### Goal

Make the gains observable and prevent silent regressions.

### Work

1. Add a repeatable benchmark scenario: start, zoom out, scroll across an
   atlas boundary, stop and pan back.
2. Parse the jank log into p50/p95/p99 and maxima for CPU build, GPU, atlas,
   ground, static walk/sort and encode.
3. Run the scenario in debug and release profiles; store only aggregate output
   in CI artifacts, never a machine-specific performance assertion as a unit
   test.
4. Keep the log opt-in or bounded outside playground if it is enabled in other
   binaries, so diagnostics cannot become a new I/O bottleneck.

### Done when

- debug steady-state frames at far zoom are within the 16 ms CPU budget on the
  reference machine, or the remaining machine-dependent gap is measured and
  documented;
- p99 excludes atlas-repack pauses during the scroll scenario;
- each performance change cites before/after samples from the same scenario.
