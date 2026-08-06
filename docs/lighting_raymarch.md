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

Steps 1–4 are done and committed. **Start at step 5** — the still-unexplained
second shape, the white line over empty background in `View::Shadow`. It
needs a real screenshot and `OPENSHARD_CLIENT`, unlike every step before it:
leave it for a session that has both. Step 4's own "Done" entry has a design
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

## Backlog

Findings go here as they turn up, same convention as `lighting.md`'s own
backlog: what the finding is, why it is worth touching, `file:line` where
there is one.

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
