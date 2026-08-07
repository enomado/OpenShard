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
  (`OPENSHARD_BOXES_GROUND_ORACLE`) still finds disagreeing points in the
  `tree` scene — in both scenes sitting right at a box's own silhouette
  *corner*, where `light::sample` still predicts occlusion the rendered
  picture misses. Session 22's own hard-shadow decision removed all corner
  softening, so this reads like the CPU/GPU near-tangent divergence this
  doc's point 1 above already tracks (an exact tile-corner tie, two
  independent `1/abs(delta)` divisions rounding differently) — plausible,
  **not verified**: nobody has checked whether the `tree` and `line` residuals
  are the *same* shape yet. First move: rerun the ground oracle scoped tight
  to one box's own corner in each scene and compare; if they match, this is
  point 1 wearing a different scene, not a new item.

  **Count correction, found by the WESL migration below, unrelated to it:**
  this entry's own session 23 measurement (`## Current status`) says 159
  (`tree`)/692 (`line`); the `tree` scene now reads 527 (368 "too dark" + 159
  "too light") — confirmed identical on the pre-migration code by `git
  stash`ing the WESL change and rerunning, so the migration did not cause
  it. Something between session 23's own measurement and now moved the
  count; not diagnosed here — the 159 "too light" half matches session 23's
  own figure exactly, so whatever changed only added the 368 "too dark"
  half, and that is a second thread, not this one.

  Reproduce: `OPENSHARD_BOXES_SCENE=tree cargo run --release -p
  openshard-client-render --example boxes` (or `OPENSHARD_BOXES_SCENE=line`)
  — the ground oracle runs by default and prints both counts plus example
  points on stderr.

- **Half closed — the `place` attachment's packing was a hand-maintained
  contract, session 23's bug was an instance of a class, and the
  *value-drift* half of that class has landed a fix across all five of its
  files. The *omission* half — session 23's own failure mode — has not, and
  is not scoped as a step below.** `kind`/`stance`/`z` are packed into
  `place` by three independent
  WGSL producers (`statics.wgsl`, `ground.wgsl`, `mesh_face.wgsl`), each
  with its own copy of the shift/mask constants (WGSL modules cannot share
  Rust `const`s), read back by two more (`blit.wgsl`, `select.wgsl`). Only
  one pinning test exists (`place.rs::a_place_packs_into_two_words`), and it
  only covers the Rust `Place` struct that backs `statics.wgsl`'s path —
  `ground.wgsl` and `mesh_face.wgsl` packed their bits directly, untested.
  Session 23's bug (a producer that forgot to stamp `stance`, decoding as a
  meaningful-but-wrong default, `Stance::Upright`) is what that gap looks
  like when it fires; the same shape was live for any future producer or any
  new `Stance`/`Kind` value.

  **Language survey, decided:** `rust-gpu` (real Rust compiled to SPIR-V)
  was considered and rejected — this crate targets `wasm32`/WebGL2 as well
  as native (`Cargo.toml`'s own header, and `docs/client.md`'s "the browser
  is a target, so it constrains the design now"), and no browser shader
  input is SPIR-V: WebGL2 never was, and WebGPU's own spec settled on WGSL
  as its one input language, full stop, not a transitional one. `rust-gpu`
  would have meant two parallel shader implementations, worse than the
  duplication it was meant to fix. **[WESL](https://wesl-lang.dev/)**
  (`wesl-rs`, the `wesl` crate) was chosen instead: an `import`-carrying
  superset of WGSL that still compiles down to plain WGSL, so both targets
  are untouched.

  **Landed, all five:** `ground.wgsl`, `statics.wgsl`, `mesh_face.wgsl`,
  `select.wgsl` and `blit.wgsl` (~1500 lines, the biggest, done last) each
  moved to `src/shaders/*.wesl`, importing the format's constants — `KIND_*`,
  `SUB_TILE`/`SUB_TILE_MASK`, `PLACE_STANCE_SHIFT`/`PLACE_STANCE_MASK`/
  `PLACE_Z_MASK`, `STANCE_FLAT`/`STANCE_FACE_*`/`STANCE_CORNER`/
  `STANCE_MESH_FACE` — from one shared `src/shaders/place_format.wesl`
  instead of each declaring its own copy. What each file still declares
  locally is only what was never duplicated in the first place —
  `statics.wgsl`'s own `STANCE_SHIFT`/`STANCE_MASK` (a different word: the
  *instance* input's stance bits, shift 16, not the attachment's own shift
  8) stayed put on purpose, see its own comment. `crates/client/render/
  build.rs` compiles all five at build time (the crate's first
  build-dependency — `wesl = "0.4"`, MSRV 1.87, no nightly toolchain needed,
  see the crate's own `Cargo.toml` comment for why the trade was worth it
  here and not for `data/doors.json`); each of `renderer.rs`/`blit.rs`/
  `select.rs` loads its compiled output via
  `include_str!(concat!(env!("OUT_DIR"), "/<name>.wgsl"))` in place of the
  old `include_str!("<name>.wgsl")`. `blit.wesl` and `select.wesl` were
  copied byte-for-byte and edited only at the two const blocks, verified by
  diffing the migrated file against the original — the 1500-line body of
  `blit.wgsl`'s raymarch was never retyped by hand.

  One thing the pilot surfaced, true for all five: `wesl-rs`'s parser is
  stricter than naga's about mixing `<<` and `|` without parens (WGSL's own
  grammar requires them) — `ground.wgsl`'s `sub` line needed them added;
  naga had been accepting it unparenthesized. The other four already
  parenthesized every mixed expression and needed no such fix.

  Verified, after all five: `cargo test --workspace`/clippy/fmt clean, and
  both the `tree` and `line` scenes' box-top and ground oracles (below) read
  identically before and after the full migration — confirmed by stashing
  it and rerunning against the original `.wgsl` files. The migration changed
  nothing about what gets drawn, only where the constants live: what it
  closes is the *value* drifting between files — five copies of
  `PLACE_STANCE_SHIFT` silently disagreeing, or a new `Stance` value added
  to one file's copy and not another's. It does **not** close session 23's
  own failure mode: a producer that never reads a shared constant at all —
  never stamps the bit — compiles clean either way, WESL or plain WGSL,
  because that is an omission in the logic, not a wrong value. Closing that
  half would need the test-time or compile-time check the earlier draft of
  this entry proposed and did not build; still open, not scoped as a step.

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
