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

## Handoff log

One entry per session, newest first. What changed, what was learned, what the
next session should read before touching anything. Append, do not rewrite —
a wrong turn kept and marked wrong is worth more than a tidied history.

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
