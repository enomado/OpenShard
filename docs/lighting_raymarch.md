# Shadow-raymarch boundary correctness: fixes, oracle, tooling

A living plan, in the shape the other plans here have: the decisions numbered
so one can be argued with alone, the steps, and a backlog of what was found on
the way and left undone.

Born from a thread inside [`lighting.md`](lighting.md), not from a survey: its
"Fixed: the shadow-raymarch anomaly" entry found the same class of bug twice —
once on the GPU side (`mesh_face.wgsl`'s `fract()`), once still open on the
CPU side (`light.rs`'s `walk_cells`/`sample`) — and left a second, unrelated
shape unexplained, with no instrument that could have told the two apart
without a screenshot. Split out because it is not one fix: it is the shape of
bug (a cell index re-derived from a float that can legitimately sit on the
cell's own boundary), the tooling that makes the next instance of it cheap to
find, and one still-open picture nobody has explained yet — enough sessions'
worth of work that it does not belong buried in `lighting.md`'s own "next
session" note.

Written against `crates/client/render/src/light.rs`, `mesh_face.rs`,
`mesh_face.wgsl`, `blit.wgsl`, `statics.rs`, `debug.rs`, `tests/lighting.rs`,
`examples/synthetic_stair.rs`, `examples/isolated_scene.rs`.

## Where the next session starts

Steps 1–4 are done and committed. **Step 5 is still open** — the
still-unexplained second shape, the white line over empty background in
`View::Shadow` — but a session with `OPENSHARD_CLIENT` finally reached it, and
found and fixed a real, adjacent bug on the way rather than the shape itself:
`blit.wgsl`'s own `walk` never got step 2's fix, and now does. Step 5's own
"Found and fixed on the way" entry has the full mechanism and a reproduction
command; its "The white line itself is untouched by this fix" entry has the
measurements (`View::Kind`, `View::Place`'s `sub.x`) that rule out the two
obvious suspects — background and the just-fixed boundary bug — and name the
one thing confirmed so far: it is a `Flat` mesh fragment, at its own tile's
far edge, immune to both fixes in this doc by construction. Start by reading
that entry before bisecting further; the backlog's `PARITY_TILE` entry is
the next piece of tooling worth building regardless of what step 5 turns out
to be, since it is the reason decision 9's suite missed both bugs in this
doc, not just the second one. Step 4's own "Done" entry still has a design
note worth reading first if step 5 (or anything after it) reaches for the
climbable stair as a fixture — it does not work as a brute-force oracle's
scene, and the reasoning is there rather than only in the handoff log.

Session 4 changed the approach rather than the diagnosis: instead of
bisecting the white line's own screenshot further, it started building the
family of small, real-geometry parity fixtures the backlog's `PARITY_TILE`
entry calls for — a primitives-first oracle, the way a test suite is built up
rather than one screenshot read harder. Two rungs are done, both in
`tests/frame.rs` through a shared `assert_single_face_parity` helper: one
hand-built flat face with no occluder (proven, by fault injection rather than
assumption, structurally blind to the boundary-walk bug class), then the same
face with one `Shape::UNREAD` occluder on its eastern neighbour, which the
same fault injection *does* catch. Full account, including the exact
fault-injected disagreement, in the backlog's two matching entries.

Session 4 ended on a geometric hypothesis for why neither rung could reach
the exact bug this doc is about, and named the fix: two faces sharing one
edge. **Session 5 built that third rung and it does not close the gap —
the hypothesis was wrong, not just unverified**, and the reason is worth
reading before touching this family again: see the backlog's "The third rung
is built, and the hypothesis behind it was wrong" entry. What actually
reaches a fragment exactly on a whole tile coordinate is not *how many faces*
share the seam, it is whether the camera happens to put that seam on a pixel
*centre* rather than a pixel boundary — nothing built so far controls that
deliberately. Either work out how to place a query point at a chosen fragment
centre on purpose (reading back which pixel a seam's own screen position
falls nearest to, rather than assuming a grid value lands there), or accept
that this family of fixtures has reached its ceiling and the white line needs
a different instrument — a debug view that reports a fragment's own
`(tile, sub)` pair directly, so a real client session can be read without
guessing which pixel matters first. The backlog entry has the reasoning for
both options; neither is started.

**The "A new `walk_cells` miss" lead is now fixed, session 6** — see the
backlog entry's own "Fixed, session 6" continuation for the mechanism.
`corner_tie` (`light.rs:1128`, `blit.wgsl:547`) now clamps at
`per_tile[near]` (one step of the axis actually being crossed) rather than
the segment-wide `1.0` this doc previously guessed at, which turned out not
to be enough on its own. Landed in both the CPU and GPU walks, verified with
the fault-injection discipline this doc uses throughout, and covered by a
permanent regression test
(`light::tests::a_wall_level_with_the_flame_is_not_skipped_by_a_shallow_ray`)
plus a new fuzz test over the same class of scene
(`tests/lighting.rs`'s `a_fuzzed_flame_near_a_row_edge_agrees_with_the_
brute_force_oracle`, `proptest` now a workspace dev-dependency). One thing
this session found and deliberately left alone, worth reading before
reaching for the fuzz harness again: widening the fuzz past the wall's own
row turns up a *second*, unrelated disagreement — a ray grazing a solid's
diagonal corner within `PANEL_THICKNESS` without entering its box — which
looks like a bug but is `corner_tie`'s own diagonal-neighbour check working
as designed (the same corner-grazing tolerance two adjoining wall panels
rely on to overlap at a shared corner). The backlog entry's last paragraph
has the confirmation that this is unrelated to the clamp fixed here (it
reproduces against the unclamped formula too) and what an oracle would need
to tell the two apart on purpose, if a future session wants to fuzz that
region rather than route around it.

**What is left in this doc is what step 5 already was: the still-open white
line.** Nothing in this session's fix touches it — the scenes are unrelated
(step 5 is about a `Flat` mesh fragment landing on a whole tile coordinate at
a pixel *centre*, this was a DDA corner-detection bug in the occlusion walk)
— so the next session's starting point is exactly where the paragraph above
step 5 left it: `PARITY_TILE`, or the debug view that reports a fragment's
own `(tile, sub)` pair directly.

## Steps

- [x] **1. `blit.wgsl` — separate "blocked" from "empty" in `View::Shadow`.**
      `through == 0.0` (a ray this pixel *has*, fully stopped) and
      `KIND_NOTHING` (no ray at all, empty background) both paint pure black
      today. Cost this track a wrong diagnosis twice already — an "orphaned
      fragment" that was just a shadowed pixel next to background, and "one
      face instead of six" that was the same confusion at a different corner.
      One line: give blocked-but-on-mesh a distinct, dark, non-black colour
      (a dark red reads as "answer: none" without competing with `Lit`'s
      palette). Diagnostics only, zero risk, no test depends on the exact
      colour — do this one first, in isolation.
- [x] **2. `Spot` carries its own tile, `walk_cells` stops re-deriving it.**
      The CPU twin of the already-shipped `MeshFaceVertex::tile`
      (`mesh_face.rs`) fix. Add `tile: (i32, i32)` to `Spot`
      (`light.rs:1189`); `Spot::at`/`::flat`/`::face` take it from the
      caller, who already knows it — `statics.rs`'s `push_mesh` has `at.x`/
      `at.y` right where it builds a vertex today, `debug.rs:219` iterates
      whole tiles, every test fixture in `tests/lighting.rs`/`onsite.rs`/
      `frame.rs` and `artscan/examples/probe.rs` already names a `tile`
      variable it currently throws away by building a bare `Vec2`.
      `walk_cells`'s `first` (`light.rs:1681`, `from[0].floor() as i32`)
      reads `spot.tile` instead of flooring `from` — `from` has already been
      nudged by `stand_clear`, so flooring it back is exactly the hazard
      `lighting.md`'s `INSIDE` constant and the `mesh_face.wgsl` fix both
      exist to name: a coordinate that legitimately sits on a whole number
      floors to the wrong side. This is the actual fix — 1, 3, 4 and 5 exist
      to make it safe to ship and to keep it from recurring elsewhere, not to
      replace it. Public API change on `Spot`, one commit, then the full
      CPU/GPU parity suite (`lighting.md` decision 9) — `cargo test -p
      openshard-client-render`.

      **Done, and it grew by one line the plan above did not name.** Fixing
      only `first` left `boundary[axis]`'s seed (the per-axis loop just below
      it, `let ahead = ...from.floor() + 1.0 - from...`) still flooring the
      same nudged `from` to find the first grid-line crossing — consistent
      with the old, wrong `first` and inconsistent with the new, right one.
      A `from` sitting on its tile's exit edge would have `first` correctly
      say "you are in this tile" while `boundary[axis]` said "you are a
      whole tile short of its edge", handing the walk a tile of slack that
      was never there. Reads `[tile.0, tile.1][axis]` instead, the same
      fix in the same shape. All 15 call sites now name a tile explicitly;
      most already had one in scope (a test's own `tile`/`x, y` fixture
      variable, `debug.rs`'s tile iteration, `frame.rs`'s parity fixture's
      `(x, y)`). Three did not — `isolated_scene.rs`'s `run_profile` (an
      arbitrary point along a swept segment) and two interior-sweep helpers
      in `tests/lighting.rs` — and got `.floor()` explicitly, which is
      **not** a regression: the profiler exists to show what a naive
      derivation does at a boundary, and the sweeps are genuinely interior
      points with nothing more authoritative to carry. Full crate builds
      (`cargo check --workspace --all-targets`); `cargo test -p
      openshard-client-render` is step 2's own remaining item below.
- [x] **3. A boundary unit test, written against the fixed `Spot`.** New test
      in `tests/lighting.rs`: `light::sample` at a handful of points
      straddling an exact integer tile edge (mirroring the real tread's
      `world.x = 1498.0`), a flame on one side, asserting `through` is
      continuous across the boundary rather than flipping. Must be added
      *after* step 2 lands, against the tile-carrying `Spot` — written
      against today's `Spot` it would just re-encode the bug it is meant to
      catch.

      **Done as `a_point_on_its_own_tiles_far_edge_reads_that_tile_not_the_next_one`**,
      reusing `light::tests::a_treads_top_is_not_shadowed_by_its_own_riser`'s
      own fixture (a climbable three-tread `Prism`) rather than a new one,
      read at the tallest tread's own far `y` edge instead of its middle.
      **Verified it actually catches the regression, not just its own
      geometry**: temporarily reverted both `first = tile` and the
      `boundary[axis]` edge fix back to `.floor()` and reran — `through`
      dropped from `1.000` at the tile's middle to `0.513` exactly on its far
      edge, confirming the earlier, weaker draft (light east of an edge the
      ray only ever moves *away* from) had picked a geometry the bug does
      not reach at all: the wrong `first` only matters when the ray's own
      path actually re-crosses back into the tile it started on, which is
      why the working version sweeps the tile's `y` edge under the same
      east-facing light the proven fixture already uses, rather than
      chasing a new light position by hand.
- [x] **4. A brute-force CPU oracle, independent of both DDA
      implementations.** A deliberately dumb ray sampler — fixed small steps
      along the ray, an occlusion lookup at each, no cell bookkeeping, no
      `floor()`/`fract()` reconstruction of any kind — compared per-pixel
      against `synthetic_stair`'s `View::Shadow` over a grid of light
      positions and angles. Shares no arithmetic with `walk_cells` or
      `mesh_face.wgsl`/`blit.wgsl`'s `walk()`, so it cannot inherit their bug
      the way a second DDA rewrite could. Where 3 catches *this* boundary, 4
      is the net for the next one, wherever it turns up.

      **Done as `a_brute_force_oracle_agrees_with_the_walk_over_a_grid_of_lights`
      in `tests/lighting.rs`, and against `light::sample` rather than a
      rendered picture** — `frame.rs`'s own `assert_parity`/`assert_parity_of`
      (decision 9) already ties `blit.wgsl`'s `walk` to `light::sample` byte
      for byte over dozens of scenes, so a second GPU readback here would
      only re-derive that tie, not add one. What decision 9's parity *cannot*
      catch is the bug this doc is about: `walk_cells` and `blit.wgsl`'s
      `walk` are two renderings of the *same* arithmetic, so a `floor()` both
      of them share is invisible to a test that holds them to each other. The
      oracle here shares no arithmetic with either — it is a point-in-box
      test against `Occlusion::solids_at`'s own boxes, stepped along the
      straight segment — so comparing it to `light::sample` is exactly as
      independent as comparing it to the picture would have been, at a
      fraction of the machinery and runnable with no GPU and no
      `OPENSHARD_CLIENT`.

      **The climbable stair was tried first and abandoned.** A brute-force
      point sampler can only state "this whole tile is exempt" — it has no
      way to ask which *surface* of a tile a ray's own end stands on, which
      is exactly what `Surface::shadowed_by_own_tile` and the `flame_end`/
      `on_surface` exemptions in `walk_cells` do ask. The stair packs three
      treads and their risers onto one tile, so a blanket per-tile exemption
      disagreed with the real walk on genuine self-occlusion (a lower tread's
      ray legitimately ducking through a higher tread's own body while still
      leaving its own tile) — real geometry, nothing to do with the boundary
      bug this oracle exists to catch, and it drowned out any signal in a
      wall of false disagreements. Swapped for a single whole-tile wall
      (`a_wall_stops_the_light_behind_it`'s own shape, `tests/frame.rs`): one
      solid on one tile, so the same blanket exemption is exactly right and
      the only question left is the boundary derivation itself.

      **Two more corner cases the grid had to be swept around, both logged
      because they are shapes a future oracle will hit again:**
      - *Grazing a box's corner.* A ray whose straight line only ever touches
        a solid's corner — never a length of its inside — is the case
        `corner_tie`'s own test already pins: the DDA gives a corner some
        resolution deliberately, a continuous point sampler finds nothing to
        stand inside. Not a bug in either; a sampler swept with light offsets
        wide enough to graze a spot's tile corner disagrees with the walk for
        a reason that has nothing to do with tile boundaries. Fixed by
        keeping spot `y` off the tile's own edges and light `dy` modest,
        rather than by teaching the oracle about corners.
      - *A flame standing on the occluder's own tile.* `walk_cells`'s
        far-end exemption (`flame_end`) is narrower than "the flame's tile is
        exempt" — it only fires when the flame's own `z` sits *on* the
        surface (`on_surface`), the same way a sconce is exempt because it
        stands on the wall it is bolted to. A flame floating at `z 25` over a
        wall whose body tops out at `20` is not on any surface of it, so the
        wall still blocks it — correctly — and a blanket per-tile brute-force
        exemption misreads that as an oracle bug. Fixed by keeping every
        light in the grid off the wall's own tile, which keeps the oracle
        inside the boundary question it was built to ask rather than asking
        it to model `on_surface` as well.

      **Verified against the same regression steps 2/3 pin**: reverting both
      `first = tile` and `boundary[axis]`'s edge back to `.floor()` (the same
      hand-revert step 3's own note used) turns every one of the oracle's 720
      spot/light pairs blocked-by-the-wall into open — the boundary point
      misreads as the wall's own tile and the wall exempts itself entirely —
      which both the oracle's disagreement check and its own "both outcomes
      have to appear" sanity assertion catch. `cargo test -p
      openshard-client-render`, `cargo check --workspace --all-targets` and
      `cargo clippy --workspace --all-targets` all green with the fix
      restored.
- [ ] **5. Diagnose the second, still-unexplained shape.** The white line
      over empty background in `View::Shadow`, confirmed present and
      unchanged by the `mesh_face.wgsl` fix — see `lighting.md`'s "Fixed: the
      shadow-raymarch anomaly" entry, "The second shape..." — cause unknown.
      Start only after 1 and 2 land — 1 removes the blocked/background
      ambiguity that made it hard to even look at, 2 removes one
      already-known confound (the tile-boundary bug) from the list of
      suspects. Bisect with `OPENSHARD_SCENE_PROFILE_FACE` the way the
      tread's outer edge was bisected in `lighting.md`'s own entry.

      **Found and fixed on the way, and it is not this shape: `blit.wgsl`'s own
      `walk` was never given step 2's fix.** `light.rs`'s `walk_cells` stopped
      flooring `from` in step 2 — `first` and `boundary[axis]`'s seed both
      read the caller's own `tile` now — but `walk`'s GPU twin, the one
      decision 9 requires to match it byte for byte, was not touched: its
      `first` was still `vec2<i32>(i32(floor(start.x)), i32(floor(start.y)))`
      and `boundary.x`/`.y`'s seed was still `floor(lit.x)`/`floor(lit.y)`,
      neither one ever given a tile to read instead. `walk` has no tile
      parameter at all — it takes `raw_start`/`raw_finish` as bare `vec3<f32>`
      and re-derives everything from them, exactly the shape step 2 closed on
      the CPU side. Fixed the same way: `walk` and `sunlight` (which calls it
      for the sun ray) both grew a `tile: vec2<f32>` parameter, read from the
      same local `fs_main` already builds `at` from, and `first`/
      `boundary[axis]`'s seed read it instead of flooring `start`/`lit`. Full
      `cargo test -p openshard-client-render` (411 tests, including decision
      9's `frame.rs` parity suite) green before and after — parity held where
      it already held, which is expected: see the backlog entry below for why
      that suite could not have caught this. Confirmed as a real change and
      not a no-op by rendering the exact scene below before and after and
      diffing the two `View::Shadow` pictures: 2,126 pixels moved, all of them
      the north-facing risers' own boundary with the flat tread above — a
      second, separate misread from the one already fixed, on an edge the
      existing regression tests do not sweep.

      **The white line itself is untouched by this fix.** Same scene, same
      pixels, same shape, confirmed by diffing before/after `View::Shadow`
      renders — the 2,126 pixels the fix did change do not include it.
      `View::Kind` at the line's own pixels reads the static/item colour, not
      the background one, and `View::Place`'s `sub.x` there reads `253/255 ≈
      126/127` — exactly `mesh_face.wgsl`'s own `INSIDE` clamp, meaning this
      is a `Flat` mesh fragment sitting right at its own tile's far edge, not
      background at all. A `Flat` stance's `outward` is `(0, 0, 1)` — no `x`/
      `y` nudge — so `floor(tile.x + 126/127)` was already `tile.x` before
      this fix, correctly, for exactly the reason that made this particular
      pixel immune to the bug just closed. Whatever reads it as fully open is
      a third thing, still to find. Reproduce:

      ```sh
      OPENSHARD_CLIENT=… \
          OPENSHARD_SCENE_AT=1497,1627,10 OPENSHARD_SCENE_RADIUS=1 \
          OPENSHARD_SCENE_TILES=0x0739 OPENSHARD_SCENE_GROUND=0 \
          OPENSHARD_SCENE_EXTRA=1498,1626,10,2852 \
          OPENSHARD_SCENE_ZOOM=2 OPENSHARD_FRAME_VIEW=7 \
          OPENSHARD_FRAME_DUMP=/tmp/shadow.ppm \
          cargo run --release -p openshard-client-render --example isolated_scene
      ```

      and look just above and left of the lamppost, along the topmost tread's
      own silhouette edge — `OPENSHARD_SCENE_GROUND=0` puts true background
      pixels at pure black, which is what makes `View::Kind`'s colour at the
      same pixels the fast way to tell the line is on-mesh rather than
      re-deriving it from `View::Place` by hand every time.

## Backlog

Findings go here as they turn up, same convention as `lighting.md`'s own
backlog: what the finding is, why it is worth touching, `file:line` where
there is one.

- **A new `walk_cells` miss, found by accident while showing the user a
  rendered picture, and confirmed not to be the already-documented `Spot`-tile
  bug.** `docs/lighting.md`'s "Still open" entry (line 150) is about a query
  point sitting *exactly* on a tile boundary with no tile to disambiguate it;
  this one is not that — every query point below shares the same explicit,
  unambiguous `Spot::tile`, computed by an ordinary `floor()` nowhere near an
  edge. A single `Shape::UNREAD` wall on `(100, 100)`, a flame at
  `(98.0, 100.0)` (due west, level with the wall's own north edge), sampled at
  `(102.5, y)` for `y` stepping through the wall's own row:
  ```
  (102.5, 99.9) tile (102, 99):  stopped_by: Some((100, 100)), through: 0.0
  (102.5, 100.0) tile (102, 100): stopped_by: Some((100, 100)), through: 0.0
  (102.5, 100.1) tile (102, 100): stopped_by: None,              through: 1.0
  (102.5, 100.2) tile (102, 100): stopped_by: None,              through: 1.0
  (102.5, 100.3) tile (102, 100): stopped_by: Some((100, 100)), through: 0.0
  (102.5, 101.0) tile (102, 100)/(102,101): stopped_by: Some((100, 100)), through: 0.0
  ```
  Four of six points on the same row, three sharing the exact same starting
  `Spot::tile`, correctly find the wall; two — `y` in roughly
  `(100.02, 100.22)`, a narrow band just south of the wall's own north edge —
  do not, and read fully lit instead. On a rendered `View::Lit` frame this is
  not a one-pixel speck: it is a visible bright streak cutting into the wall's
  own shadow, close enough to the light source's own row to read as a second,
  spurious "horn" beside the real shadow's edge — this is what the user
  spotted on sight in a picture built for an unrelated reason (showing what
  the existing rungs' scenes actually look like), not something found by
  sweeping for it.
  **Root-caused, same session, with a per-iteration DDA trace.** A throwaway
  `eprintln!` in `walk_cells`'s own step loop (guarded by an env var, not
  kept) printed `cell`, `boundary`, `entry` and `corner_tie(per_tile,
  out_by_x)` at every iteration for the `y 100.1` (fails) and `y 100.3`
  (passes) points side by side. The passing walk steps `(102,100) → (101,100)
  → (100,100)`, one axis at a time, and finds the wall on the third step. The
  failing walk steps `(102,100) → (101,99) → (100,99) → (99,99) → (98,99)` —
  **row 99, not row 100, from the very first step**, walking straight past
  the wall's own row entirely and reaching the flame's cell unobstructed.
  Step 0 is identical in both traces (`boundary [0.1111, 1.0]`, same cell,
  same physical geometry so far — the divergence is not in *where* the ray
  is, it is in *which step the walk takes next*.
  **The mechanism is `corner_tie` (`light.rs:1128`), and it is a real formula
  bug, not a tolerance that merely needed retuning.** `corner_tie`'s own
  derivation (`light.rs:1104`-`1127`) is sound for a ray that crosses both
  axes' boundaries somewhere inside the segment: it converts
  `PANEL_THICKNESS` world units into the `t` this DDA steps in by dividing by
  `|delta[far]|` (`per_tile[far]`), so the *closer* two boundary crossings are
  in `t`, the more likely they are the same physical corner. But
  `per_tile[far] = 1 / |delta[far]|` has no ceiling, and this scene's flame
  sits **exactly** on the wall row's own north edge (`flame.y == 100.0`,
  `tile.y == 100`) — for the `y 100.1` query, `delta.y` is `-0.1`, so
  `per_tile[1] = 10.0` and `corner_tie` comes out to `≈2.0`, an order of
  magnitude past `1.0`, the largest a `boundary` value inside the segment can
  legitimately be. `boundary[1]` for that same query is exactly `1.0` — not a
  coincidence, it is `ahead * per_tile[1]` where `ahead` is the *same*
  distance to the flame's own row-edge `y` that made `per_tile[1]` explode —
  meaning the far axis's boundary sits at the very end of the whole segment,
  at the flame itself, nowhere near the corner the walk is about to cross at
  `t = boundary[0] = 0.111`. The tie check (`light.rs:2008`,
  `(boundary[0] - boundary[1]).abs() <= corner_tie`) does not know that: it
  compares a raw difference in `t` against a threshold that grew without
  bound *because* the ray is shallow, and a threshold that large swallows any
  `boundary[0]`, so the walk treats "the ray happens to end almost exactly on
  a row line" as "a corner is imminent right now" and steps diagonally past
  both neighbours of the current cell — skipping row 100, including the
  wall, in one move. **The derivation's own assumption — that a small `t`
  gap implies a small world-space gap — silently inverts for a ray nearly
  parallel to the axis being compared against**, which is the same family of
  "a value that is fine near the middle of its domain breaks at an extreme"
  this doc's own entries have hit before (see the `floor`-vs-`round` harness
  bug, and step 5's own vertex-ring argument), just landing in a different
  formula this time.
  **Not yet fixed, and not attempted this session — `corner_tie`/the tie
  check is shared with `blit.wgsl`'s own mirror (decision 9's CPU/GPU
  parity), so a real fix has to land in both, verified against both, not
  patched CPU-side alone.** The shape of a fix is not obvious from this
  entry alone: bounding `corner_tie` at `1.0` (nothing past the segment's own
  end can be "imminent") is the first thing to try, but whether that is
  correct or just moves the false-positive threshold is unverified — the
  fault-injection discipline this doc already uses (revert the fix, confirm
  the six-point counter-example fails again; apply it, confirm the
  counter-example passes *and* the existing `a_wall_stops_the_light_behind_it`/
  `two_faces_sharing_an_edge_agree_with_light_sample` suite stays green) is
  the way to find out, not reading the formula harder.
  Reproduced with two throwaway `#[ignore]`d probes in `tests/frame.rs` (an
  ASCII heatmap, a six-point printout, and — this session — a per-iteration
  DDA trace via a temporary `eprintln!` in `light.rs` gated on
  `OPENSHARD_WALK_TRACE`) and a throwaway GPU picture dump; none were kept —
  this entry is the only trace left, on purpose, so the next session does not
  have to guess the repro back out of a screenshot. `cargo check --workspace
  --all-targets`, `cargo clippy --workspace --all-targets` clean after every
  revert.

  **Fixed, session 6.** The bound this entry's own last paragraph guessed at
  (`corner_tie` capped at `1.0`) turned out to be wrong when actually tried —
  it still left the six-point counter-example failing, because `1.0` bounds
  the tie against *the whole segment*, and this scene's spurious tie
  (`≈0.89` in `t`) was comfortably under that. The bound that actually works
  is capping `corner_tie` at `per_tile[near]` — one whole step of the axis
  *actually being crossed* right now — rather than at a segment-wide
  constant: `per_tile[far]` alone answers "how far can the far axis's
  boundary be from the near one, in `t`, and still be `PANEL_THICKNESS`
  away in world units," but says nothing about whether that far boundary is
  *contemporary* with the crossing about to happen, which is the only sense
  in which two boundaries share a corner. A ray that hugs a grid line for
  its whole length (this scene's shape exactly) keeps a small world-space
  gap to that line at *every* near-axis crossing along the way, not just
  near one true corner — `per_tile[near]` is what tells those apart, since a
  genuine corner's two boundaries are close in `t` because they are the same
  instant, not because one of them is a whole segment away. Landed in both
  `light.rs:1128`'s `corner_tie` and `blit.wgsl:547`'s mirror, verified with
  the discipline this entry called for: reverted, confirmed the
  counter-example (now a permanent test, see below) fails again; reapplied,
  confirmed it passes and `a_wall_stops_the_light_behind_it` /
  `two_faces_sharing_an_edge_agree_with_light_sample` / the rest of
  `cargo test -p openshard-client-render` stay green.

  **The six-point table above has one wrong entry, found re-deriving the
  ground truth rather than trusting the transcript.** `y = 99.9` is listed as
  correctly finding the wall, but the straight-line geometry says otherwise:
  parametrising the segment, `y(t) < 100` for every interior `t` — the ray
  never actually enters the wall's row, so the geometrically correct answer
  is *open*, not blocked. The old, buggy walk got there anyway by a second,
  unrelated route: at its very first boundary the inflated `corner_tie` fired
  immediately (the raw difference `0.89` was still under the old,
  unclamped-by-`per_tile[near]` threshold of `≈2.0`) and took a spurious
  diagonal step that happened to land back in the wall's own row, from which
  ordinary per-axis stepping found the wall the honest way. Two bugs, one
  coincidence, and the table conflated "looks consistent with its neighbours"
  with "is correct" — exactly what an independent oracle exists to catch
  instead of a hand-traced printout. `light::tests::
  a_wall_level_with_the_flame_is_not_skipped_by_a_shallow_ray` (`light.rs`)
  is the corrected, permanent version of this counter-example.

  **Fuzzed, not just fixed to the one fixture.** The grid-sweep oracle
  (`a_brute_force_oracle_agrees_with_the_walk_over_a_grid_of_lights`,
  `tests/lighting.rs`) explicitly keeps every ray clear of real corners by
  its own comment — this bug's whole shape lived in the region that
  deliberately excludes. `tests/lighting.rs`'s new
  `a_fuzzed_flame_near_a_row_edge_agrees_with_the_brute_force_oracle`
  (`proptest`, added as a workspace dev-dependency this session) covers that
  region instead: the flame's `y` is biased within three tenths of a whole
  number on purpose, everything else free to roam, shrunk to a minimal
  counter-example on failure. It is deliberately narrower than "any two
  points anywhere" — the spot's own `y` is kept inside the wall's row.
  Widening that once, to see how far the fuzz could reach, immediately
  surfaced a second, genuine disagreement: a spot near its own tile's edge in
  a *different* row than the wall, with the ray grazing the wall's diagonal
  corner within `PANEL_THICKNESS` without ever entering its box. That one is
  not a bug — it is exactly the corner-grazing ambiguity the grid oracle's
  own comment already carves out, `corner_tie`'s diagonal-neighbour check
  treating "within panel-thickness of a shared corner" as "as much in the way
  as the tile stepped into," by design, for two panels that physically
  overlap there. Confirmed unrelated to this session's clamp: the same
  disagreement reproduces against the *unclamped* formula too. Narrowing the
  spot back to the wall's own row is what keeps the fuzz on-topic without
  re-litigating that design choice; a future session wanting to fuzz the
  corner-grazing case itself needs an oracle that knows about the
  `PANEL_THICKNESS` slop, not a plain point-in-box test.

- **A first real-geometry parity fixture exists now, and it is a rung, not the
  ladder — proven blind to the boundary-walk bug class by construction, not by
  assumption.** `tests/frame.rs`'s
  `a_single_flat_face_agrees_with_light_sample_over_a_grid_of_lights` is the
  gap the entry below already named closed halfway: one hand-built
  `crate::mesh::Face` (`crate::mesh::Face::new`, no `Prism`, no risers) rendered
  through the real `GroundRenderer`/`MeshFaceRenderer`/`Blit` pipeline, swept
  over eight light angles and a grid of `(u, v)` points ending at `INSIDE`
  itself, each checked against `light::sample` fed the same clamped,
  seven-bit-quantised fraction the shader would compute. It is deliberately
  the smallest scene that exercises `mesh_face.wgsl`'s own vertex/fragment path
  at all — no `parity_frame` fixture above ever does, they all write the
  `place` texture by hand. **No occluder stands anywhere in this scene, and a
  fault-injection check proved that matters**: corrupting `mesh_face.wgsl`'s
  own `SUB_TILE` constant (`127.0` → `100.0`, a real CPU/GPU disagreement in
  the fraction every fragment writes) left the test green, because `walk()`
  returns `1.0` unconditionally when nothing can block it — the tile/fraction
  it was fed never gets asked a question whose answer could differ. The
  fixture is real and worth keeping (it does catch a broken ambient, falloff,
  cone or beam term, and it is the first parity test to touch mesh-face
  rendering at all), but it cannot yet be the tool that reproduces step 5's own
  white line, or the tile-boundary bug steps 1–4 already fixed — both need a
  ray that can be blocked.
- **The next rung is done too, and the same fault-injection check now catches
  what the first rung couldn't.**
  `a_single_flat_face_beside_an_occluder_agrees_with_light_sample`
  (`tests/frame.rs`) is the same single face, plus one whole-tile
  `Shape::UNREAD` occluder on its eastern neighbour —
  `a_wall_stops_the_light_behind_it`'s own wall, one tile over rather than
  three — swept through the same `assert_single_face_parity` helper the first
  rung now shares with it. Confirmed to actually exercise occlusion before
  trusting it (a temporary per-sample print, not left in): of 288 compared
  points, 92 came back blocked and 196 open, both `Reach::within` values
  appearing too — a scene that only ever produced one answer would pass this
  fixture for the wrong reason. The same `SUB_TILE` corruption
  (`127.0` → `100.0`) that the occluder-free fixture could not see now fails
  it immediately, at `(u 0.75, v INSIDE)` on the face's own edge shared with
  the wall's tile — the shader says `51` (a blocked-and-dark-red pixel),
  `light::sample` says `255` (open). Reverted before committing
  (`git checkout -- mesh_face.wgsl`); both fixtures green with the real file.
- **Even this cannot yet reach the exact bug steps 1–4 fixed or step 5's white
  line, and the reason is geometric rather than a gap to close by sweeping
  harder.** Both bugs are about a fragment sitting *exactly* on a whole tile
  number — `at.x` legitimately `1498.0`, not `1497.999...` — and a single flat
  `Face`'s own far edge is the quad's own vertex ring: no fragment's own
  centre is ever rasterised exactly there, which is the same reason the
  harness's own `floor`-vs-`round` bug (next entry) bit at `INSIDE`, one
  hundred-and-twenty-seventh of a tile short of that edge, rather than at the
  edge itself. Reaching a fragment that reads a *whole* tile coordinate needs
  two faces meeting at a shared seam — the real shape a stair's tread-to-tread
  edge or a wall's own corner is — so the next rung past this one is not a
  wider sweep of the same single quad, it is a second face on the
  neighbouring tile sharing an edge with the first, the smallest scene where
  a fragment can legitimately land on a coordinate that is a whole number
  rather than approach one.
- **The third rung is built, and the hypothesis behind it was wrong — a
  second face sharing the seam does not, by itself, let a fragment land on
  the seam.** `tests/frame.rs`'s
  `two_faces_sharing_an_edge_agree_with_light_sample` (via a new
  `assert_two_face_edge_parity` helper, deliberately not a generalisation of
  `assert_single_face_parity` — a two-face scene has two tile origins and two
  corner rings, and folding that into the one-face helper's signature would
  have been the kind of parameter creep this doc's own `PARITY_TILE` entry
  already warns about) renders a west face and an east face meeting at
  `west.0 + 1`, with a `Shape::UNREAD` wall two tiles further east giving both
  faces a genuine mix of blocked and open rays. Green against the real
  shader. **Before trusting that green, ran the same `SUB_TILE` fault
  injection the first two rungs used, and — separately — reverted
  `mesh_face.wgsl`'s `sub = in.world.xy - in.tile` back to `fract(in.world.xy)`,
  the exact bug steps 1–4 fixed. Both faces' own grid stays entirely on
  `[tile, tile + 1)` — `near_seam_from_west` tops out at `INSIDE`,
  `near_seam_from_east` bottoms out at `1.0 - INSIDE`, neither ever exactly
  `0.0` or `1.0` — and on that half-open interval `fract(world.xy)` and
  `world.xy - tile` are the same expression by construction, because `tile`
  is already the floor of every point either grid samples.** The `fract()`
  revert left the fixture green; so did running it against
  `a_single_flat_face_beside_an_occluder_agrees_with_light_sample` again as a
  sanity check on the pre-existing rung. **Having a second face did not
  change what the grid could reach — it was never the number of faces that
  mattered, it was that the query points still approach the seam without
  ever landing on it.** The session-4 hypothesis conflated two different
  things: a *scene* where a fragment on the seam is geometrically possible
  (true of two adjacent faces, false of one face's own vertex ring) and a
  *test harness* that actually produces such a fragment (neither rung's grid
  does, because both stop at the same half-pixel margin the `floor`-vs-`round`
  entry below already explains). Reaching the seam for real needs the query
  point chosen from the render itself — read back which screen pixel the
  seam's own projected position falls nearest to, then assert *that* pixel's
  tile-of-origin, rather than picking `(u, v)` values in advance and hoping
  one lands there. Not attempted this session; the reasoning above is
  offered so the next session does not re-arrive at "two faces" as the fix
  and re-spend the time this one did finding out it isn't.
  `SUB_TILE` reverted, `fract()` reverted, both confirmed clean with
  `git status` before either was touched again; `cargo test -p
  openshard-client-render` (43 tests in `frame.rs`, one new), `cargo clippy
  --workspace --all-targets` and `cargo check --workspace --all-targets` all
  clean with the real files.
- Also worth logging next time this fixture is extended: the harness itself
  had a real off-by-one, caught only because a query point was deliberately
  placed within a fraction of a pixel of the quad's own true edge (`INSIDE`
  itself, `1/127` of a tile short of the geometric boundary). Converting a
  continuous screen coordinate to the pixel index that covers it needs
  `floor`, not `round` — a fragment's own sample point is its pixel's centre
  (`i + 0.5`), and `round` reads as correct everywhere except within half a
  pixel of a true edge, which is exactly where a boundary oracle spends most
  of its samples by design. Cost a full debugging pass here (a bounding-box
  scan and a single-row coverage scan of the rendered frame) before the fix
  was obvious; worth remembering before building the next fixture in this
  family rather than re-discovering it.
- **`frame.rs`'s decision-9 parity suite never samples a sub-tile fraction
  past `112/127`, so it could not have caught the `walk` bug step 5 just
  fixed.** `PARITY_TILE = 8` (`tests/frame.rs:3592`) steps `sub_x`/`sub_y` in
  sixteenths — `0, 16, …, 112` — chosen so the fraction fits the seven-bit
  encoding exactly, but that stops three sixteenths short of `127`, and
  `mesh_face.wgsl`'s own `INSIDE = 126/127` clamp lives inside that gap. Every
  scene the suite runs is faceless-ground or a stated `Surface` at a stated
  height (`parity_frame`/`parity_place`), never a mesh face at all, so a
  `Spot` sitting exactly where a stair's own geometry does — which is where
  both this bug and the fixed one in steps 1–4 lived — is a case the suite
  structurally cannot generate. Worth its own step if this track continues:
  either widen `parity_place`'s sweep to include the `112..127` range (a
  `Face` surface there exercises `STAND_OFF`'s nudge the same way this bug
  needed) or build a mesh-face scene through `statics.rs`'s real `push_mesh`
  path and run `assert_parity` against it — the gap is specifically that no
  parity scene has ever gone through a mesh face's own vertex attributes
  rather than a synthetic per-pixel `place` write.

- **A true fixed-point world coordinate (tile + N bits of sub-tile
  resolution, one integer type, no `f32`) would remove this whole class of
  bug at the source instead of working around it.** Raised while doing step
  2: `Spot.tile` plus an `f32` fraction is already a *hybrid* of this — it
  mirrors `mesh_face.wgsl`/`blit.wgsl`'s own `(tile, sub)` pair — and once
  the tile is carried and never re-derived, the fraction sitting on an exact
  boundary is harmless: nothing branches on it for cell selection anymore.
  So a full fixed-point rewrite buys **nothing more for this specific bug
  class** than step 2 already closes. What it would buy is broader: no float
  epsilon anywhere a world position is stored or compared, which is a
  question about `geometry::Vec2`, the camera, movement and the protocol —
  not about lighting, and not scoped to one crate. Left here rather than
  turned into a step: worth a decision of its own, on its own track, if it
  is ever picked up — not a rider on this one.

- **The DDA's own stepping was untestable in isolation, and every bug this
  doc chases lived exactly there.** A testability audit (session 7) found
  that `light.rs` had direct numeric unit tests for `crosses` and
  `corner_tie` already, but every other pure helper in the walk —
  `pierces`, `inside`, `run_v`, `hole`, `pierced`, `stand_clear`,
  `on_surface`, `own_run`, `panel_stop`, `faces` — was exercised only
  through a full lit scene (`tests/lighting.rs`'s own suite), where a
  failure does not localise to which of them broke. Worse than any one of
  those: the stepping logic itself — which cell follows which, and whether
  two of them tie at a corner — was inline inside `walk_cells`'s ~400-line
  occlusion loop, sharing no boundary with `Occlusion` a test could stand
  on. That loop is where `Spot.tile` (step 2) and `corner_tie`'s clamp
  (session 6) both actually lived; a bug there could only ever be caught by
  building a scene and rendering or sampling it — a screenshot's own
  problem, one level removed.
- **Fixed by extraction, not by patching**: `dda_walk` (`light.rs:1751`),
  returning `Vec<DdaCell>` (`light.rs:1704`) with `DdaTransition`
  (`light.rs:1724`), is `walk_cells`'s own stepping — `per_tile`,
  `boundary`, the corner-tie decision, which cell comes next — with every
  dependency on `Occlusion` removed. `walk_cells` (`light.rs:1911`) is now a
  thin consumer: for each `DdaCell` it applies exactly the same occlusion
  arithmetic it always did, and reads `crossing` to know whether to run the
  corner-panel check. Behaviour-preserving by construction (the geometry of
  which cell follows which was already independent of any occlusion
  outcome, confirmed by tracing every call site before extracting), and
  verified rather than assumed: full `cargo test -p openshard-client-render`
  (411 pre-existing tests, including `frame.rs`'s GPU parity suite and the
  proptest fuzzer) green before touching `walk_cells`'s body and unchanged
  after, `cargo clippy --workspace --all-targets` and
  `cargo fmt -p openshard-client-render -- --check` clean.
- **Fault-injection confirmed the extraction actually carries both known
  bugs, not just their absence of symptoms.** Reverting `dda_walk`'s edge
  seed to `from.floor()` (step 2's bug) fails the new
  `a_from_on_its_own_tiles_far_edge_leaves_it_almost_immediately`
  (`light.rs:3077`) directly — `leaves` comes back `0.333` instead of near
  zero, a whole tile of the exact slack the bug always cost. Reverting
  `corner_tie`'s clamp to the pre-session-6 unclamped formula fails both the
  existing `a_wall_level_with_the_flame_is_not_skipped_by_a_shallow_ray`
  *and* the new pure-geometry
  `the_dda_walk_does_not_skip_the_wall_row_on_a_shallow_ray`
  (`light.rs:3049`) — and the pure test's own failure message reproduces
  the exact "two bugs, one coincidence" mechanism this backlog's "A new
  `walk_cells` miss" entry already documented for `y = 99.9`: cells
  `[(102, 99), (101, 100), (100, 100), (99, 100), (98, 100)]`, the spurious
  corner jumping straight from row 99 to row 100 and landing back in the
  wall's row by accident. Both reverts restored before committing.
- **New pure-numeric coverage, no scene required for any of it**: the
  six-point counter-example from "A new `walk_cells` miss" now exists twice
  — once as the original full-scene regression test, once as
  `the_dda_walk_does_not_skip_the_wall_row_on_a_shallow_ray` asking only
  which cells `dda_walk` visits — plus a 1024-case proptest,
  `dda_walk_visits_a_connected_path_of_cells_starting_at_the_callers_tile`
  (`light.rs:3099`), checking the walk starts at the caller's own `tile`
  (never a re-derived `floor()`), that consecutive cells are always
  Chebyshev-neighbours, that `entered`/`leaves` only move forward and stay
  in `0.0..=1.0`, and that an axis-aligned ray never takes a corner. All ten
  of the previously scene-only pure helpers listed above now have their own
  direct unit test (`inside`, `pierces`, `run_v`, `hole`, `pierced`,
  `own_run`, `stand_clear`, `on_surface`, `panel_stop`, `faces`, all in
  `light.rs`'s own `mod tests`), plus proptests for `inside` (clamp and
  symmetry about the interval's centre) alongside `dda_walk`'s own. 27 new
  test cases in `light.rs` in total, all green, none of them touching
  `Occlusion`, `Lighting`, or a rendered frame.
- **A design tradeoff worth naming rather than hiding**: `dda_walk` now
  computes every cell up to `MAX_WALK_STEPS` eagerly, where `walk_cells`
  used to stop lazily the moment `through <= RAY_CUTOFF`. This costs a
  handful of unused `DdaCell`s on an early-cutoff ray (bounded by
  `MAX_WALK_STEPS = 72`) and buys the separation above; cell *selection* has
  no dependency on occlusion outcomes to begin with; not measured for a
  real frame's worth of rays but expected negligible next to a walk that
  already runs per-flame, per-pixel.
- **A bigger idea, raised mid-audit and deliberately deferred to its own
  session, not started here**: `occlusion::Solid` (`occlusion.rs:556`)
  already stores each occluder as a real `WorldSpot`-cornered box — exact,
  continuous, no tile index anywhere in the record. `dda_walk` doesn't
  remove the float-boundary bug *class*, it only makes the existing walk's
  own instance of it testable: grid-DDA over a continuous position
  necessarily asks "which discrete tile am I in right now" at every step,
  regardless of how precise the `Solid` looked up per cell is, which is
  exactly why the bugs in this doc were all in `walk_cells`'s stepping and
  never in `Solid`'s own geometry. What *would* remove the class by
  construction is a different algorithm, not a better-tested version of
  this one: gather the `Solid`s near a ray (the tile grid stays as exactly
  that broad-phase) and intersect the ray against each directly — ray-vs-AABB,
  the slab method, continuous throughout, with no discrete "current tile"
  concept anywhere left to get wrong at a boundary. That would obsolete
  `dda_walk`, `corner_tie` and `PANEL_THICKNESS`'s corner-overlap tolerance
  outright rather than test them harder, which is exactly why it wants its
  own session to scope first: which solids the broad-phase should gather
  and how, whether the two adjoining panels `PANEL_THICKNESS` exists for
  still need an explicit overlap tolerance or fall out of continuous
  ray-vs-box intersection for free, and a rough sense of the cost against
  today's `MAX_WALK_STEPS`-bounded walk before committing to the rewrite.

## Handoff log

One entry per session, newest first. What changed, what was learned, what the
next session should read before touching anything. Append, do not rewrite —
a wrong turn kept and marked wrong is worth more than a tidied history.

### Session 7 — a testability audit, `dda_walk` extracted, step 5 untouched

Not a continuation of step 5 — a side session, asked for by name: "which
places in the walk/DDA can be made testable with unit tests and proptests,
so numbers can be compared instead of pictures." Full account in the
backlog's three new entries above; short version here.

- **Audited what already had direct numeric coverage versus what only had
  full-scene coverage.** `crosses` and `corner_tie` did; ten other pure
  helpers in `light.rs` and the DDA stepping itself, inline inside
  `walk_cells`, did not — a failure there could only be read off a rendered
  or CPU-sampled scene, one level removed from the actual arithmetic.
- **Extracted `dda_walk`/`DdaCell`/`DdaTransition` out of `walk_cells`** —
  the stepping (`per_tile`, `boundary`, the corner-tie decision, which cell
  follows which) with every dependency on `Occlusion` removed.
  `walk_cells` now consumes its output and applies the same occlusion
  arithmetic it always did. Confirmed behaviour-preserving by the full
  existing suite (411 tests, including `frame.rs`'s GPU parity) staying
  green before and after, not by inspection alone.
- **Verified the new tests actually catch what they claim to, this doc's
  own fault-injection discipline**: reverted step 2's tile-seed fix and
  session 6's `corner_tie` clamp in turn, confirmed the relevant new tests
  fail with the expected numbers (one of them reproducing the exact
  "two bugs, one coincidence" mechanism the backlog already documented for
  `y = 99.9`), reapplied, confirmed green. Both reverts were temporary and
  restored before this session's real diff was touched again.
- **27 new test cases**, all in `light.rs`'s own `mod tests`, none touching
  `Occlusion`, `Lighting`, or a rendered frame: direct unit tests for all
  ten previously scene-only helpers, a pure-geometry echo of the six-point
  counter-example, a boundary-seed regression test, and two proptests
  (`inside`'s symmetry, `dda_walk`'s own connectivity/monotonicity/start-tile
  invariants over 1024 random rays).
- **Raised, and deliberately deferred rather than started**: since
  `occlusion::Solid` already stores exact `WorldSpot` boxes, a ray-vs-`Solid`
  walk (gather candidates off the tile grid, intersect each directly by the
  slab method) would remove this whole *class* of float-boundary bug by
  construction rather than test the existing grid-DDA harder — a genuine
  architecture change, not a bugfix, and the user asked explicitly that it
  wait for its own session rather than ride on this one. Full reasoning and
  the open questions it would need scoped first are in the backlog's own
  entry.
- `cargo test -p openshard-client-render` (338 unit tests plus every
  integration file), `cargo clippy -p openshard-client-render --all-targets`,
  `cargo fmt -p openshard-client-render -- --check`, and
  `cargo check --workspace --all-targets` all clean at the end of the
  session.
- **Not touched**: step 5's own white line — this session never rendered a
  frame, and the audit's scope was testability of the walk in general, not
  diagnosis of that specific shape. Next session on the white line itself
  should still start where session 6 left it — see "Where the next session
  starts" above — and can read this session's ray-vs-`Solid` backlog entry
  as an available but unstarted alternative direction, not a redirect away
  from it.

### Session 6 — `corner_tie` fixed, and fuzzed rather than pinned to one fixture

Continued from session 5's handoff: the "A new `walk_cells` miss" lead was
root-caused but not fixed.

- **The fix session 5 guessed at (`corner_tie` clamped at `1.0`) was tried
  first and did not work** — the counter-example still failed, because `1.0`
  bounds against the whole segment and this scene's spurious tie (`≈0.89`)
  was comfortably under that. Re-derived from the mechanism instead of
  re-guessing: what actually distinguishes a real corner from this scene's
  shallow-ray false positive is not the *size* of the tie but whether the far
  axis's boundary is *contemporary* with the crossing about to happen —
  clamping at `per_tile[near]` (one step of the axis actually being crossed)
  encodes that directly. Landed in `light.rs:1128` and `blit.wgsl:547`.
- **Fault-injection discipline applied both ways**, not just checked once:
  reverted just the clamp (kept the new regression test in place) and
  confirmed both the unit test and the new fuzz test below fail again with
  the exact numbers this doc's backlog entry predicted; reapplied and
  confirmed both pass, plus the whole of `cargo test -p
  openshard-client-render` (416 test cases across `light.rs`'s own suite and
  every integration file).
- **Turning the six-point table into a permanent test caught a second,
  independent mistake — in the table itself, not the code.** Re-deriving
  `y = 99.9`'s expected answer from the segment's own parametrisation (rather
  than trusting session 5's hand-traced printout) shows the ray never
  actually enters the wall's row for any interior `t` — the geometrically
  correct answer is *open*. The old buggy walk got to "blocked" anyway by an
  unrelated coincidence: its very first boundary already tripped the
  (unclamped) tie and took a spurious diagonal step that happened to land
  back in the right row. `light::tests::
  a_wall_level_with_the_flame_is_not_skipped_by_a_shallow_ray` asserts the
  re-derived answers, not the transcribed ones.
- **Added `proptest` as a workspace dev-dependency** (user's own suggestion,
  mid-session) and a fuzz test,
  `a_fuzzed_flame_near_a_row_edge_agrees_with_the_brute_force_oracle`
  (`tests/lighting.rs`), biased into exactly the region the existing
  grid-sweep oracle's own comment says it deliberately avoids — a flame near
  a row's own grid line. First version let the spot's own `y` roam freely
  too and immediately shrunk to a *second* disagreement: a spot near its own
  tile edge, in a different row than the wall, with the ray grazing the
  wall's diagonal corner within `PANEL_THICKNESS` without entering its box.
  Traced this one too rather than assuming it was the same bug: it reproduces
  identically against the *unclamped* formula, so it predates this session
  and is not a regression from the fix — it is `corner_tie`'s
  diagonal-neighbour check doing exactly what it is for (the same
  panel-corner overlap tolerance two adjoining walls rely on), just applied
  to a body solid one tile diagonally away instead of a literal shared panel
  corner. Narrowed the fuzz to keep the spot inside the wall's own row, which
  keeps the test on-topic without adjudicating that separate design question;
  left for a future session, noted in the backlog entry's last paragraph, in
  case it is worth fuzzing on purpose with an oracle that knows about the
  `PANEL_THICKNESS` slop.
- `cargo test -p openshard-client-render`, `cargo check --workspace
  --all-targets`, `cargo clippy --workspace --all-targets`, `cargo fmt --all`
  all clean at the end of the session.
- **Not touched**: step 5's own white line — unrelated scene, unrelated bug
  class, left exactly where session 4/5 left it. See "Where the next session
  starts" above.

### Session 5 — the third rung, and the hypothesis it was built to confirm turned out wrong

Continued straight from session 4's handoff rather than re-diagnosing: two
things it named as unverified.

- **Re-ran session 4's own suggested pre-check first**: reverted
  `mesh_face.wgsl`'s `sub = in.world.xy - in.tile` to `fract(in.world.xy)`
  and confirmed `a_single_flat_face_beside_an_occluder_agrees_with_light_sample`
  (the existing second rung) stays green, matching what session 4 argued but
  had not measured. Reverted before touching anything else.
- **Built the third rung**: `two_faces_sharing_an_edge_agree_with_light_sample`
  and its own `assert_two_face_edge_parity` helper in `tests/frame.rs` — a
  west face and an east face meeting at a shared tile edge, a `Shape::UNREAD`
  wall two tiles further east for a genuine occluded/open mix on both faces.
  Green against the real shader, `cargo test -p openshard-client-render` (43
  tests), `cargo clippy --workspace --all-targets` and `cargo check
  --workspace --all-targets` all clean.
- **Ran the same `fract()` revert against the new rung, expecting it to catch
  what the first two couldn't — it did not.** Full reasoning in the
  backlog's new entry; short version: the grid both faces sample stays on
  their own half-open `[tile, tile+1)` interval by construction (`INSIDE` and
  `1.0 - INSIDE`, never the exact corner), and on that interval `fract()` and
  `world.xy - tile` are the same value regardless of how many faces share the
  seam. Session 4's hypothesis — "two faces is the smallest scene where a
  fragment can legitimately land on a whole number" — is true about the
  *scene* (the seam is geometrically reachable) but false about what this
  harness's grid actually samples (it never queries the seam itself, only
  approaches it from both sides). Also re-ran the `SUB_TILE` fault injection
  from the first two rungs against the new one as a sanity check — it fails
  as expected, so the new rung is not simply blind end-to-end.
- **Did not attempt the actual fix**: reading back which real screen pixel
  the seam's projected position falls nearest to and asserting that pixel
  specifically, rather than sampling `(u, v)` values chosen in advance. Left
  as the next session's starting point rather than started here — see "Where
  the next session starts" above, including the fallback option (a debug view
  that reports `(tile, sub)` directly) if that turns out not to be worth
  building.
- Showed the user the two existing rungs' own rendered frames on request, via
  a temporary `#[ignore]`d dump test that wrote RGBA readback to PPM and
  converted with `imagemagick`; deleted before this session's real changes
  were touched again, never part of the diff.
- **Found a new, real, unrelated `walk_cells` miss while showing the user a
  third picture — a continuous floor under a torch and a wall, built to
  answer "can I see the shadow actually fall?".** The user spotted the shadow's
  own shape was wrong on sight, before any deliberate bug hunt; confirmed with
  a CPU-oracle probe rather than trusting the picture. Full account,
  including the six-point counter-example and why it is not decision 9's
  `Spot`-tile question again, in the backlog's "A new `walk_cells` miss"
  entry. Session ended with the user asking to start root-causing this in the
  same sitting — continued below rather than in a new entry, since it is the
  same session's own work.

### Session 4 — the first real-geometry parity fixture, and proof of its own blind spot

Changed approach rather than diagnosis: session 3 left step 5 at "bisect the
white line's own screenshot further"; this session started building the
primitives-first family of parity fixtures the backlog already called for
instead — smallest scene first, the way a test suite is built rather than a
screenshot read harder.

- **Added `a_single_flat_face_agrees_with_light_sample_over_a_grid_of_lights`
  in `tests/frame.rs`.** One hand-built `crate::mesh::Face` (no `Prism`), no
  occluder, rendered through the real `GroundRenderer`/`MeshFaceRenderer`/
  `Blit` pipeline over eight light angles, checked at a `(u, v)` grid ending at
  `INSIDE` itself against `light::sample` fed the same clamped, quantised
  fraction the shader computes. First parity test of any kind to go through
  `mesh_face.wgsl`'s own vertex/fragment path rather than a synthetic
  per-pixel `place` write. Green, `cargo test -p openshard-client-render` (all
  323 tests), `cargo clippy --workspace --all-targets` and
  `cargo check --workspace --all-targets` all clean.
- **Proved, by fault injection rather than by reasoning about the code, that
  this fixture cannot yet catch the bug class it exists to eventually catch.**
  Corrupted `mesh_face.wgsl`'s `SUB_TILE` constant (`127.0` → `100.0`, a real
  disagreement between what the shader writes and what the CPU oracle
  expects) and the test stayed green: with no occluder anywhere in the scene,
  `walk()` returns `1.0` unconditionally, so the tile/fraction a fragment
  carries is never actually asked a question whose answer could differ.
  Reverted before committing (`git checkout -- mesh_face.wgsl`). Full
  reasoning and the concrete next scene (the same face plus one occluder on a
  neighbouring tile) are in the backlog's own entry — next session starts
  there, not back at the white line's screenshot.
- **A real harness bug on the way, logged as its own backlog entry**:
  converting a rendered frame's continuous screen coordinate to a pixel index
  needs `floor`, not `round` — a fragment's sample point is its pixel's own
  centre (`i + 0.5`), and `round` only disagrees with that within half a
  pixel of a true edge, which is exactly where this family of fixtures spends
  most of its samples on purpose. Found by a bounding-box scan and a
  single-row coverage scan of the actual rendered frame after a query point
  placed deliberately close to the face's own far edge came back reading
  background; worth reading before the next fixture in this family hits the
  same thing by surprise.
- `light.rs` picked up an unrelated three-line `cargo fmt` normalization
  (`sample`'s own ambient line) that predates this session — left in rather
  than reverted, since `cargo fmt --all` is expected silent and this closes
  one more place it was not.
- **Continued in the same session: built the next rung, and it catches what
  the first one proved it couldn't.** Refactored the render/compare loop into
  a shared `assert_single_face_parity` helper (`tile`, `face_z`, `occlusion`,
  `lights` as parameters) so the two fixtures cannot drift from each other's
  camera, grid or comparison logic, then added
  `a_single_flat_face_beside_an_occluder_agrees_with_light_sample`: the same
  face, plus one whole-tile `Shape::UNREAD` occluder one tile east —
  `a_wall_stops_the_light_behind_it`'s own wall, moved from three tiles away
  to one. Confirmed the occluder is actually exercised before trusting the
  fixture (a temporary per-sample print of `Reach::within`/`through`, not
  kept): of 288 compared points, 92 blocked and 196 open. The same
  `SUB_TILE` fault injection that the occluder-free rung could not see now
  fails immediately, at `(u 0.75, v INSIDE)` — the shader says `51` (blocked),
  `light::sample` says `255` (open). Reverted before committing; both
  fixtures green with the real shader, `cargo test -p openshard-client-render`
  (323 tests), `cargo clippy --workspace --all-targets` and
  `cargo check --workspace --all-targets` all clean, `clippy --fix` cleared 15
  `needless_borrow` warnings the refactor left behind (`device`/`queue`
  becoming reference parameters rather than owned locals).
- **Even the occluder rung cannot reach the exact bug this doc is chasing,
  and worked out why rather than reaching for a wider sweep.** Both bugs are
  about a fragment sitting *exactly* on a whole tile coordinate, and a single
  quad's own far edge is its vertex ring — no fragment is ever rasterised
  there, only arbitrarily close, which is the same geometric fact the
  `floor`-vs-`round` entry above is a symptom of. Reaching a fragment that
  reads a genuinely whole coordinate needs two faces sharing an edge, not a
  wider grid on one face. Logged as the backlog's own entry rather than
  chased this session — it is the next rung, and a session that has read it
  should not have to re-derive it.

### Session 3 — step 5, `OPENSHARD_CLIENT` reached it, found a real bug that isn't it

Committed session 2's already-written step 4 (was sitting uncommitted).
First session on this track with `OPENSHARD_CLIENT` available, so the first
to actually render the doc's own reproduction scene instead of reasoning
about it secondhand.

- **The white line is on-mesh, not background** — the premise "over empty
  background... where there is no geometry at all" doesn't hold under
  measurement. `View::Kind` at the line's own pixels reads the static/item
  colour; true background (`OPENSHARD_SCENE_GROUND=0` makes it pure black)
  is elsewhere in the same picture and looks nothing like it. Cost some time
  before being checked, because at a glance a thin bright sliver next to a
  dark region reads as "background poking through" — exactly the same
  reading-the-eye-instead-of-the-pixels trap `lighting.md`'s own "a thin,
  nearly-tangent lit strip" entry already named once.
- **Found and fixed a real bug on the way, confirmed it is a different one
  from the white line, and kept both facts in step 5's own entry rather than
  calling either "done."** `blit.wgsl`'s `walk` had the exact bug class steps
  1–4 fixed on the CPU side — `first` and `boundary[axis]`'s seed both
  floored a raw float instead of reading a carried tile — and had it because
  `walk` was never given a tile to read at all, not because the fix missed a
  spot. Full mechanism, the fix, and how it was confirmed to be a real
  change (a before/after picture diff, 2,126 pixels, none of them the white
  line) are in step 5's own "Found and fixed on the way" entry.
- **Why the existing parity suite never caught it, and it's a real gap, not
  bad luck**: `PARITY_TILE = 8` steps sub-tile fractions in sixteenths and
  stops at `112/127`, three short of `127`, and the `walk` bug lived in
  exactly that last stretch — `mesh_face.wgsl`'s own `INSIDE = 126/127`
  clamp sits inside it. Logged in the backlog rather than fixed this
  session: widening the sweep or building a real mesh-face parity scene is
  its own piece of work, not a rider on this one.
- **The white line survives being ruled out twice, which narrows it more
  than it sounds like**: it isn't background (measured), and it isn't the
  bug just fixed (the fragment's own stance is `Flat`, whose `outward` is
  `(0, 0, 1)` — no `x`/`y` nudge — so the fixed and unfixed formulas agree at
  exactly this pixel, and the before/after diff confirms it: this pixel
  isn't in it). Next session: now that a real scene renders in this sandbox,
  bisect the same way `lighting.md`'s own entry bisected the first shape —
  `OPENSHARD_SCENE_PROFILE_FACE` at the line's own real-world coordinates —
  but read the `own_shadows`/`admitted` exemption logic in `walk`
  (`blit.wgsl:899` onward) first: cell selection is now proven not to be the
  cause here, which leaves the *exemption* rules (which of a cell's sides may
  shadow a pixel standing on that same cell) as the next thing to doubt.
- `cargo check --workspace --all-targets`, `cargo clippy --workspace
  --all-targets` and `cargo test -p openshard-client-render` (411 tests) all
  green, before and after the `walk` fix.

### Session 2 — step 4, the brute-force oracle

Step 4 done, its own commit. `cargo check --workspace --all-targets`, `cargo
clippy --workspace --all-targets` and `cargo test -p openshard-client-render`
all green.

- **Compared against `light::sample`, not a rendered picture** — the plan's
  own wording said `synthetic_stair`'s `View::Shadow`, but `frame.rs`'s
  decision-9 parity suite already ties the GPU's `walk` to `light::sample`
  exactly, so a GPU readback here would have re-proven that tie rather than
  adding an independent one. The oracle's independence comes from sharing no
  arithmetic with *either* implementation, which holds just as well one level
  up. Left as a design note in step 4's own "Done" entry rather than buried
  here, since the next reader of this plan needs to know it before reaching
  for a GPU harness that isn't needed.
- **The stair fixture doesn't work for this** — tried it first, since it's
  what steps 2/3 already trust, and abandoned it once a wall of disagreements
  turned out to be real self-occlusion (`Surface::shadowed_by_own_tile`'s
  selective exemption) that a blanket per-tile brute-force sampler cannot
  model. Swapped for a single whole-tile wall, where the blanket exemption
  the oracle is capable of stating happens to be exactly right. Full
  reasoning in step 4's own note — worth reading before reaching for the
  stair again for a *different* oracle, since the same trap is waiting there.
- **Two false-disagreement shapes swept around rather than modelled**: a ray
  grazing a solid's corner (the DDA and a continuous sampler are not obliged
  to agree there — `corner_tie`'s own test already owns that case), and a
  flame floating above an occluder's own tile without standing *on* any
  surface of it (`walk_cells`'s `flame_end`/`on_surface` exemption is
  narrower than "the tile is exempt"). Both logged in step 4's "Done" entry
  with the fix (keep the grid off those configurations) rather than taught to
  the oracle, which would have meant re-deriving `on_surface` a second time —
  exactly the duplication decision 9's own parity suite exists to avoid.
- **Verified the oracle actually catches the regression**, the way step 3's
  own note insists on: hand-reverted `first = tile` and `boundary[axis]`'s
  edge fix back to `.floor()`, reran, and every one of 720 spot/light pairs
  flipped from "blocked by the wall" to "open" — the boundary spot misreads
  as the wall's own tile and the wall exempts itself. Restored before
  committing.
- **Step 5 still needs `OPENSHARD_CLIENT` and a real screenshot** — nothing
  in this session touched it, and nothing here narrows it.

### Session 1 — this doc's opening session

Steps 1, 2 and 3 done, each its own commit (`eb85ea6`, `24298d1`, `755ff99`;
doc scaffolding in `c0a306b` and `c7f4535`). `cargo check --workspace
--all-targets` and the full `openshard-client-render` suite are green.

- **Step 2 grew by one line the plan didn't name**: `boundary[axis]`'s seed
  had the same `.floor()` as `first` and needed the same fix, or `first`
  being right while `boundary` still assumed the old wrong tile would have
  been a *new* inconsistency, not a fix. See step 2's own "Done, and it grew"
  note for the reasoning.
- **A design question came up mid-session and was logged, not chased**:
  whether to replace `f32` world coordinates with a true fixed-point
  tile+sub-tile type everywhere. Answer, in the backlog below: it buys
  nothing more for *this* bug class than `Spot.tile` already closes, and
  it's a repo-wide question, not a lighting one.
- **The boundary test in step 3 does not follow the plan's own example
  literally** ("mirroring the real tread's `world.x = 1498.0`"). The first
  draft picked a light position that never re-crossed the boundary it was
  supposed to be testing and stayed green even with both fixes reverted by
  hand — worth remembering: **a boundary test has to make the ray travel
  back through the tile it started on, not just start on the boundary**.
  The version that shipped reuses the already-proven
  `a_treads_top_is_not_shadowed_by_its_own_riser` fixture instead of
  inventing new geometry, and was itself verified against a hand revert
  before being trusted (`1.000` → `0.513`, logged in step 3's own note).
- **Step 4 needs no `OPENSHARD_CLIENT`** — `synthetic_stair` is built with no
  client files at all — so it's reachable in a sandbox; step 5 does need one
  and a real screenshot, so it waits for a session that has both.
