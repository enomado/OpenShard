# Shadow-raymarch boundary correctness: fixes, oracle, tooling

A living plan, in the shape the other plans here have: the decisions
numbered so one can be argued with alone, a short checklist, and a backlog
of what is still open. **The history lives in
[`lighting_raymarch_archive.md`](lighting_raymarch_archive.md)** — every
step's full worked narrative, the complete backlog (including everything
already fixed), and the session-by-session handoff log (sessions 1-22).
This file only carries what is still true and still actionable; split out
2026-08-07 because the two had been growing as one 3300-line document and
it was getting hard to tell current state from history.

Born from a thread inside [`lighting.md`](lighting.md) — see that doc's
"Fixed: the shadow-raymarch anomaly" entry for why this became its own
track: a cell index re-derived from a float that can legitimately sit on
the cell's own boundary, found twice (GPU and CPU sides), plus one still-
being-chased shape found along the way.

Written against `crates/client/render/src/light.rs`, `mesh_face.rs`,
`mesh_face.wgsl`, `blit.wgsl`, `statics.rs`, `debug.rs`, `tests/lighting.rs`,
`examples/synthetic_stair.rs`, `examples/isolated_scene.rs`, `examples/boxes.rs`.

## Current status

**Session 23 — the session-22 ground-shadow gap root-caused and fixed:
`ground.wgsl` never stamped a stance, so land read as `Stance::Upright` (not
`Flat`) to `blit.wgsl`'s exemption logic and wrongly earned the wall-mount
exemption meant only for a pixel standing on the surface it is bolted to.**
Invisible for every whole-tile body ever built (same tile = same footprint,
so the exemption and genuine occlusion always covered the same ground); the
`tree` scene's *sub-tile* box beside open ground on its own tile is what
finally told the two apart. Fixed in `ground.wgsl` (stamps `STANCE_FLAT`
alongside the height, the way `statics.wgsl` always has) plus one follow-on
seam it opened (`fs_main` now zeroes the face normal back out for
`KIND_LAND`, so land does not also pick up the half-space light-gate a wall's
flat cap correctly has — `kind`, not `stance`, tells the two apart for that
one consumer). Two tests updated to match what `light::sample` already
predicted (archive, Session 23 entry, has which and why — one was passing
for the same wrong reason this fix removes). Verified by reverting the one
fix alone and confirming the count regressed exactly back: a new ground-plane
oracle in `examples/boxes.rs`
(`OPENSHARD_BOXES_GROUND_ORACLE`) went from 1127 disagreeing points to 159
(an 86% cut), and the `tree` scene's own rendered picture shows the notch
that used to be bitten out of the shadow's own silhouette is gone. Full
`cargo test --workspace`/`clippy`/`fmt` clean. The remaining 159 (692 in the
`line` scene) are a **different, not-yet-confirmed** residual — read Session
23's own "what is left" paragraph before assuming it is the same bug.

**Session 22 — decision: hard shadows, not faked soft ones.** A flame is a
point source, and a point source's shadow is exact everywhere, corners
included. `CORNER_GRAZE`/`CORNER_GAP_SOFTEN`/`ray_vs_body` (session 20/21's
hand-tuned penumbra for a body's silhouette corner) are gone, both sides —
`ray_vs_solid`'s exact slab test is now the entire `EDGE_ANY` occlusion test
in both `light.rs` and `blit.wgsl`. User-requested reversal, walked through
explicitly (archive, Session 22 entry). `SOFT_CROSSING_MIN`/`MAX` and the
lid/panel `crosses`/`pierced` machinery are untouched — different thing (a
lid's own zero-thickness plane, a panel's aperture), not what either bug
report was about. Verified: full workspace tests, clippy, fmt clean;
`examples/boxes.rs`'s `tree` scene independent oracle reads `0/9216`
disagreements on both box tops.

**Track A (the tile-boundary bug, steps 1-5) is closed.** All five steps
done; step 5's white line was fixed session 19 as a side effect of Track
B's own footprint upload. Full mechanism per step in the archive.

**Track B (ray-vs-Solid) — points 1-4 are done, the DDA cutover has
landed.** `blit.wgsl`'s `walk` is `light::walk_cells_streaming`'s own
algorithm; `walk_cells`/`corner_tie`/`panel_stop`/`DdaTransition::Corner`
are deleted, both sides. What is still open from this track:

1. **One parity gap, triaged and accepted rather than chased — not a
   "start here."** At an exact tile-corner tie, `light::sample` and
   `blit.wgsl`'s copy of the identical formula can disagree about which
   axis is nearer (two independent `1.0 / abs(delta)` divisions rounding
   differently on CPU vs GPU). `light::sample` is never in the real render
   path — every caller is debug/test tooling — so this is "does the CPU
   debug oracle still match the shader," not a visible shadow bug. Two
   affected tests are `#[ignore]`d with the reasoning attached
   (`tests/frame.rs`). An epsilon tie-break was tried and made things
   *worse* (fixed this case, broke ordinary-geometry parity elsewhere) — see
   the archive for what was tried and the two shapes worth trying if this is
   ever picked back up.
2. **The GPU sub-tile footprint gap has landed** (session 19,
   `Occlusion::footprint_bytes` + `box_side` reading a body's own `lo`/`hi`)
   — `blit.wgsl`'s occlusion upload now carries a solid's real `x`/`y`
   span instead of reconstructing one from `(tile, edges)`. This closed the
   `tree` scene's box-top disagreement to `0/9216`. Landing this surfaced
   the still-open ground-shadow bug below.

Two real bugs were found and fixed along the way and do not need
re-fixing: `corner_tie`'s formula bug (session 6) and `exemption`'s vacuous
self-exemption for a `Flat` fragment (session 14, `light.rs:1301` /
`blit.wgsl:995`). Mechanism for both in the archive, under their sessions.

## Steps

- [x] 1. `blit.wgsl` — separate "blocked" from "empty" in `View::Shadow`.
- [x] 2. `Spot` carries its own tile; `walk_cells` stops re-deriving it
      from a nudged `from` via `floor()`.
- [x] 3. A boundary unit test, written against the tile-carrying `Spot`.
- [x] 4. A brute-force CPU oracle, independent of both DDA implementations.
- [x] 5. Diagnose the second, still-unexplained white line — found to be
      `blit.wgsl`'s `walk` missing step 2's fix (session 18-19), and after
      that fix, the white line itself remained untouched and unexplained by
      it (it turned out to be a `Flat` fragment at its own tile's far edge,
      not background — cause still not the same thing the fix addressed).

All five done; full per-step narrative, reproduction commands, and what
each one ruled out is in the archive under `## Steps`.

## Backlog

Only what is currently open. Full history — including everything below
that used to be here and is now fixed — is in the archive under
`## Backlog`.

- **Open, session 23 — a smaller residual left by the session-22 gap's fix,
  not yet confirmed to be one bug or a second one.** After `ground.wgsl`'s
  stance fix landed, `examples/boxes.rs`'s new ground oracle
  (`OPENSHARD_BOXES_GROUND_ORACLE`) still finds 159 disagreeing points in the
  `tree` scene and 692 in the `line` scene (whole-tile boxes, so not a
  sub-tile-footprint story) — in both scenes sitting right at a box's own
  silhouette *corner*, where `light::sample` still predicts occlusion the
  rendered picture misses. Session 22's own hard-shadow decision removed all
  corner softening, so this reads like the CPU/GPU near-tangent divergence
  this doc's point 1 above already tracks (an exact tile-corner tie, two
  independent `1/abs(delta)` divisions rounding differently) — plausible,
  **not verified**: nobody has checked whether the `tree` and `line` residuals
  are the *same* shape yet. First move: rerun the ground oracle scoped tight
  to one box's own corner in each scene and compare; if they match, this is
  point 1 wearing a different scene, not a new item.

  Reproduce: `OPENSHARD_BOXES_SCENE=tree cargo run --release -p
  openshard-client-render --example boxes` (or `OPENSHARD_BOXES_SCENE=line`)
  — the ground oracle runs by default and prints both counts plus example
  points on stderr.

- **Someday/maybe, not scoped as a step**: a full fixed-point world
  coordinate (tile + N bits of sub-tile resolution, no `f32`) would remove
  float-epsilon bugs at the boundary at the source. Raised while doing step
  2; buys nothing more for *this* bug class than step 2 already closed
  (once a tile is carried and never re-derived, a fraction sitting on an
  exact boundary is harmless). Would buy something broader — no float
  epsilon anywhere a world position is stored or compared — but that is a
  question about `geometry::Vec2`, the camera, movement and the protocol,
  not lighting, and not scoped to one crate. Own track if ever picked up.

## Handoff log

Moved to [`lighting_raymarch_archive.md`](lighting_raymarch_archive.md)
(sessions 1-22). Add the next entry there, and update `## Current status`
and `## Backlog` above to match.
