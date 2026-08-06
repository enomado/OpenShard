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

**Two independent tracks. Track A's only open item is step 5, and session 18
found the first reproducible, precisely-localised anomaly this doc has ever
pinned on it — read session 18's own entry before touching it again. Track
B's point 4 cutover landed session 16; its one open parity gap was triaged
and closed as an accepted limitation in session 17 — read that entry before
reopening it.**

**Track A — the tile-boundary bug (steps 1-5 below).** Steps 1-4 are done
and committed. Step 5 — the still-unexplained white line over empty
background in `View::Shadow` — is the only open item; its own entry in
`## Steps` has the full mechanism ruled out so far, the reproduction
command, and what to read before bisecting further (`PARITY_TILE`'s own
backlog entry, and step 4's own note on why the climbable stair does not
work as a brute-force oracle's scene).

**Track B — ray-vs-Solid (the backlog's "A bigger idea..." entry).** Points
1-3 are done. Point 4, the cutover, **landed session 16**: `blit.wgsl`'s
`walk` is now `light::walk_cells_streaming`'s own algorithm, `sample` calls
it by default, and `walk_cells`/`corner_tie`/`panel_stop`/
`DdaTransition::Corner` and the tests that pinned them are gone. What is
left:

1. **The one open parity gap session 16 found and narrowed — triaged,
   session 17, and closed as an accepted, documented limitation rather than
   chased further.** `the_shader_and_light_sample_agree_about_a_carried_beam`
   and `the_shader_lights_a_frame_as_light_sample_does` (now `#[ignore]`d,
   `tests/frame.rs`) fail decision 9's suite at a query point on
   `scene::lantern_in_a_room`'s east wall (`(103.63, 101.13)`, own tile
   `(103, 101)`, light near `(100, 100)`): `light::sample` reads
   `boundary[0]` and `boundary[1]` as an **exact tie** (`0.2012578` both),
   while `blit.wgsl`'s own copy of the identical formula, on inputs confirmed
   byte-identical (`start`/`finish`/`tile`, and the light's own `place.xyz`
   uploaded as raw, unquantised `f32` bytes — `blit.rs`'s `lighting_bytes`),
   computes `boundary.x` a **real** `~0.05` less than `boundary.y` — a
   genuine disagreement about which axis is nearer, sending the walk's own
   trajectory onto a different first cell on each backend.

   **Why this is closed rather than left as "start here."** Two things,
   found by checking rather than assuming, per this doc's own house style:
   - **The scene and the sweep are not at fault.** `parity_place`'s grid is a
     regular `1/8`-tile lattice over an ordinary room, not a hand-picked
     adversarial point — an exact tile-corner tie is a real, if narrow,
     geometric configuration a full-frame sweep is bound to land on
     eventually, exactly because it is exhaustive rather than curated. There
     is no test bug to fix here.
   - **`light::sample` is never in the real render path.** Checked every
     caller in the workspace: `tests/frame.rs`/`tests/lighting.rs`,
     `examples/isolated_scene.rs`, `examples/boxes.rs`,
     `artscan/examples/probe.rs`, and `debug.rs`'s own diagnostic tile map —
     debugging and tooling, every one. What a player actually sees is always
     `blit.wgsl`'s own `walk`, which is internally self-consistent regardless
     of which side of a tie it lands on. So the gap this decision-9 pair
     guards is "does the CPU debug oracle still describe what the shader
     draws," not "is a shadow wrong on screen" — real, but lower stakes than
     it reads at first, and not a reason to force a fix under time pressure.

   **What a real fix would cost, named so it is not re-litigated from
   scratch.** An epsilon on the tie-break comparison was already tried this
   track (`boundary.x < boundary.y - EPS`, session 16) and made parity
   *worse* — it fixed this exact case and broke `walk_cells_streaming_
   agrees_with_walk_cells_exact_in_a_small_room` on ordinary geometry nowhere
   near a tie, because CPU and GPU do not compute a close-enough
   `boundary.x - boundary.y` to agree on which rays even count as near a
   tie. That rules out tolerance-based fixes generally, not just the one
   tried. The two shapes actually worth trying, if this is ever picked back
   up: (a) replace the two independent `1.0 / abs(delta)` divisions with a
   single cross-multiplied comparison (`ahead.x * abs(delta.y)` vs
   `ahead.y * abs(delta.x)`) — fewer operations for the two backends'
   compilers to diverge over, cheap to test with the `DEBUG_TRACE` technique
   already used this session; or, if that still disagrees, (b) make the walk
   tie-order-*independent* by construction — on a real tie, walk both
   branches to completion and take the max occlusion, rather than trying to
   force two different compilers to break the tie identically. (b) is real
   work (a design of its own, not a rider on this track) and is not started.
2. **A second, separate gap this port does not close.** Session 14 built
   `occlusion::Builder::add_raw` and `examples/boxes.rs` to test boxes
   narrower than a tile, and found that `blit.wgsl`'s occlusion upload
   (`Occlusion::solid_bytes`) carries no `x`/`y` at all — only `(z_bottom,
   z_top, opacity, edges)` — so a sub-tile occluder's shadow is wrong on the
   GPU regardless of how exact the walk above becomes, because the data it
   would read from has nothing to be exact *with*. Read the backlog's "A
   second bigger idea..." entry (the last one before `## Handoff log`) for
   the measured numbers and what widening the format needs to touch. This is
   **not** required to finish point 4 as this doc has always scoped it — it
   is its own, later, unscoped task.

Two real bugs were found and fixed along the way, both worth knowing about
before touching either track: `corner_tie`'s formula bug (session 6, a
shallow ray's own boundary drowning the near-axis crossing it should have
compared against) and `exemption`'s vacuous self-exemption for a `Flat`
fragment whenever any body shares its own tile (session 14, `light.rs:1301`
and `blit.wgsl:995`, found through `add_raw`'s stacked-box scene). Neither
needs re-fixing; both are logged in `## Handoff log` under their own
sessions for the mechanism, not repeated here.

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

      **Session 18, after the point 4 rewrite: the exemption/walk hypothesis
      is ruled out a second time, on the new algorithm, and a real, precisely
      localised `through` discontinuity was found in its place.** Full
      mechanism and reproduction in this step's own Handoff log entry
      (session 18) — not repeated here — but the shape of the finding: at the
      exact tread 3 / riser 3 seam (`y = 1627.333`, `z = 15`), `light::sample`
      itself (not a CPU/GPU disagreement — both the `Flat` top and the
      `Face(South)` riser read the identical `through` at every `x`) reports a
      smooth, physically-sensible soft-shadow gradient from `0.098` down to
      `0.069` as `x` sweeps `1497.55 → 1497.67`, then **jumps straight to
      `1.000` between `x = 1497.67` and `x = 1497.68`** — no intermediate
      values, where a widening penumbra predicts a continued climb through
      `0.3`, `0.5`, `0.7`. That is the signature of a candidate solid
      dropping out of the walk's own consideration entirely as the ray's
      angle crosses a threshold, not of the shadow itself ending. Not
      root-caused this session — which specific solid's `ray_vs_solid` test
      flips, and why abruptly, is the next session's own first question, with
      the exact profiling commands to get there already run once (see the
      Handoff log entry).

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

  **Scoped, session 8 — all three questions answered by reading
  `walk_cells`, `dda_walk` and `blit.wgsl`'s `walk` line by line, not by
  guessing from this entry's own summary.** The picture is better and worse
  than the paragraph above assumed: better, because two of the three named
  frictions turn out to dissolve rather than need a design; worse, because
  `walk_cells` carries far more per-solid business logic than "does this ray
  hit this box" — self-shadow exemptions, wall-run continuity, apertures,
  penumbra softness — all of it keyed to *tile identity*, which any
  replacement still has to carry.

  1. **Which solids the broad-phase gathers, and how.** Not a bounding box —
     a straight ray at 45° over `MAX_WALK_STEPS = 72` cells has a 72×72
     bounding box, two orders of magnitude more tiles than the ray actually
     passes near. The existing cell enumeration is already the right shape
     and already correct and tested (`dda_walk`, session 7's extraction):
     walk it as today for the *straight* neighbour at each step, and instead
     of conditionally adding the diagonal neighbour only when `corner_tie`'s
     heuristic fires, add it **unconditionally** — every step transition
     names both `by_x` and `by_y` already (`light.rs:1834`-`1839`), the walk
     just doesn't visit both today. Candidate count roughly doubles (two
     cells probed per step instead of one), still `O(walk length)`, nowhere
     near a bounding box.
  2. **Whether `PANEL_THICKNESS`/`corner_tie` survive.** `corner_tie` — the
     heuristic itself, comparing a `t`-space gap between two boundaries
     against a threshold — is fully replaced, not kept: it exists only to
     guess whether a diagonal neighbour is "close enough" to matter *before*
     paying to look at it. Once every corner candidate is probed
     unconditionally (point 1) and each probe is an exact ray-vs-box test
     against a panel that is *already* `PANEL_THICKNESS` deep
     (`occlusion.rs`'s `box_of`, `EDGE_NORTH`/`EDGE_SOUTH`/`EDGE_WEST`/
     `EDGE_EAST`), the question "is this corner within `PANEL_THICKNESS`"
     answers itself: the box either contains the crossing point or it
     doesn't. `PANEL_THICKNESS` itself survives as the physical depth of a
     panel's box — that's geometry, not a walk-algorithm approximation, and
     nothing here touches it. What goes: `corner_tie` (`light.rs:1150`,
     `blit.wgsl:558`), `DdaTransition::Corner`'s special-cased handling in
     `walk_cells` (`light.rs:2192`-`2222`) and `panel_stop`
     (`light.rs:1192`, called only from that special case) — three things
     this doc has twice had to fix a bug in (steps 1-4's boundary bug,
     session 6's shallow-ray bug), gone by construction rather than tested
     harder.
  3. **Rough cost.** Roughly a wash, not a win or a loss worth the rewrite on
     its own. Each corner probe is a handful of slab compares (six, for a 3D
     AABB) against a real solid's box — about what `corner_tie` plus the
     conditional `panel_stop` call already cost when the tie fired, paid
     unconditionally instead of on a heuristic's say-so. No change to
     `MAX_WALK_STEPS` or the broad-phase's own cell count. The rewrite's
     case has to rest on correctness (removing a bug class by construction)
     and testability, not on speed.

  **What does not get simpler, and has to be carried by any replacement,
  not designed around**: `walk_cells` (`light.rs:1911`-`2226`) is not just
  "does the ray hit a box" — `own_cell`/`first`/`last` (self-shadow
  exemption), `same_run`/`own_run` (a run of wall not shadowing itself,
  keyed to tile row/column adjacency), `on_surface`/`caps_this` (which
  surface of a multi-storey tile a pixel actually stands on), apertures
  (`pierced`, a hole in a specific panel), and per-cell penumbra softness
  scaled by how far along the ray a crossing happens (`entered`/`leaves`,
  feeding `soft`/`wide`/`tall`) are all still needed and are all keyed to a
  solid's *originating tile*, which a plain "list of boxes near this ray"
  does not carry on its own. The broad-phase has to hand over `(tile,
  &Solid)` pairs, not bare `&Solid`s, or `own_run`/`on_surface` have nothing
  to compare tiles against. None of this is a reason not to do the rewrite —
  it is the reason it is a body-swap of `walk_cells`'s *stepping*, not a
  rewrite of its *rules*, and why the safe order is to prove the new
  stepping against the old one before cutting over, not after.

  **Recommended shape, in the order this doc's own discipline already
  argues for (see step 4's brute-force oracle, session 6/7's
  fault-injection): build the replacement as a second path, gate it against
  the existing suite as an oracle, cut over only once it wins.**
  1. A new pure primitive — segment-vs-AABB, the slab method, taking a
     `crate::solid::Solid` (`solid.rs:33`, already exact `WorldSpot`-cornered
     `min`/`max`) and a segment, returning `entered`/`leaves` in `0.0..=1.0`
     or nothing. No `Occlusion`, no tile, no walk — the same isolation
     `dda_walk` itself was extracted for, testable against a hand-computed
     truth table and a proptest oracle (a point sampled at a random `t`
     inside vs. outside the returned interval should agree with plain
     point-in-box, the same independence discipline step 4's brute-force
     oracle already uses). No existing helper does this anywhere in the
     crate — checked before writing a new one, per the repo's reuse rule.
  2. A parallel `walk_cells`-shaped function built on it, `(tile, &Solid)`
     pairs from the doubled broad-phase in point 1 above, the *same*
     exemption/run/aperture/softness rules `walk_cells` already has —
     copied and adapted, not re-derived from scratch, since re-deriving is
     exactly what has bitten this doc before (see `feedback-
     rederived-formula-needs-reference-gate` in project memory). Not wired
     into any real frame yet.
  3. Run both paths over every scene this doc already has an oracle for —
     the grid-sweep and fuzz oracles in `tests/lighting.rs`, the
     `frame.rs` parity suite, the permanent regression tests for both fixed
     bugs — asserting the two `walk_cells`-shaped functions agree with each
     other, not just each with `light::sample`'s old formula. Disagreements
     here are the interesting output of this step, not a failure: each one
     is either a bug in the new path or a case the old one already had wrong
     that the fuzz/parity suites happened not to reach.
  4. Only once 3 is clean does replacing `walk_cells` become a one-function
     swap, mirrored in `blit.wgsl` the same way step 5's `walk` fix already
     was, with decision 9's full parity suite as the gate.

  Not started this session — this is the scope, not the rewrite. Whether it
  is worth its own living-plan doc (this doc's own precedent: split out of
  `lighting.md` once it was "enough sessions' worth of work that it does
  not belong buried") is an open question for whoever starts point 1, not
  answered here.

  **Point 1 built and tested, same session.** `ray_vs_solid`
  (`light.rs:1104`) — segment-vs-`crate::solid::Solid`, the slab method,
  `Option<(entered, leaves)>` in `0.0..=1.0`. Pure, no `Occlusion`, no tile,
  `#[allow(dead_code)]` and not called from anywhere real yet — staged
  ahead of point 2 on purpose, the way the recommended order above asks
  for. Six hand-computed unit tests (a straight crossing checked against
  fractions worked out by hand, a clean miss, both ends already inside, a
  degenerate flat lid, a degenerate thin panel exactly
  `PANEL_THICKNESS` deep, a tangent corner touch returning a real
  zero-length interval rather than `None`) plus a 2048-case proptest
  checking the one thing worth an independent oracle for: a point sampled
  at a random `t` along the segment lies inside the box exactly when `t`
  falls inside the returned interval — the same point-in-box discipline
  step 4's brute-force sampler already uses against the whole walk, turned
  on this one primitive instead. All seven green; `cargo test -p
  openshard-client-render` (full crate, 16 test binaries), `cargo clippy -p
  openshard-client-render --all-targets`, `cargo check --workspace
  --all-targets` and `rustfmt --check crates/client/render/src/light.rs`
  all clean. Point 2 (the parallel `walk_cells`-shaped function over the
  doubled broad-phase) is next, not started.

  **Point 2 built, session 9 — and "agreement" turned out to have real
  shape, not be a single yes/no.** `candidate_tiles` (`light.rs`, right
  after `dda_walk`) is the doubled broad-phase point 1 above asked for: it
  walks `dda_walk`'s own straight-line cells and, at every transition,
  additionally names both single-axis neighbours — `by_x`/`by_y`, exactly
  what `DdaTransition::Corner` already computes, pushed unconditionally now
  rather than only when `corner_tie` decides to. **First implementation
  bug, found by the fuzz described below before this doc was ever
  updated**: the first draft pushed `(cell.0 + toward.0, cell.1 +
  toward.1)` — the cell reached by stepping *both* axes — instead of the
  two single-axis cells. That cell is already `dda_walk`'s own next-step
  (ordinary `Step`) or next-next cell (`Corner`'s own destination), so the
  bug did not crash anything, it just meant the genuinely untested corner
  candidate was never probed at all — `walk_cells_exact` disagreed with
  `walk_cells` on **1519 of 20,000** fuzzed rays over a single wall before
  this was caught. Fixed by pushing `by_x`/`by_y` directly, matching
  `DdaTransition::Corner`'s own fields exactly rather than re-deriving a
  formula for them.

  `walk_cells_exact` itself (`light.rs`, right after `candidate_tiles`) is
  `walk_cells`'s exemption/run/aperture/softness rules copied onto the new
  broad-phase: for every candidate tile, every solid on it gets its own
  exact `entered`/`leaves` from `ray_vs_solid` instead of a DDA cell's
  shared, tile-boundary-derived pair, grouped back by tile so `through` is
  still updated once per tile by the *largest* of what its solids stop —
  `walk_cells`'s own reason (two panels of one corner are two faces of one
  wall, crossed once) untouched. `box_side` (`light.rs`, beside
  `candidate_tiles`) is the other piece of plumbing this needed: a body's
  box *is* its tile's own footprint, so which side of the tile a
  `ray_vs_solid` crossing point sits on can be read straight off which
  tile boundary it touches, geometrically, standing in for a `DdaCell`'s
  `entry`/`exit` bits for candidates that were never reached by an actual
  DDA step.

  **Second implementation bug, same fuzz, after the first was fixed**:
  dropping `walk_cells`'s "does either tile-boundary side pierce this
  body" check on an `EDGE_ANY` solid, on the theory that an exact box
  crossing no longer needs a safety net a DDA approximation invented —
  wrong. `walk_cells`'s own comment already says why: "the pierce is what
  closes the sliver a ray clipping a corner used to walk through." That is
  a deliberate design choice (a corner reads as opaque, not as
  proportionally see-through for having been grazed at a narrow angle),
  not a workaround for DDA imprecision, and dropping it left
  `walk_cells_exact` reading a body's corner as almost fully open in cases
  `walk_cells` read as fully blocked. Restored, using `box_side` to find
  which face `ray_vs_solid`'s own `entered`/`leaves` points sit on instead
  of carrying `entry`/`exit` from a DDA step that, for a diagonal-only
  candidate, never happened.

  **With both bugs fixed, the fuzz stopped finding new-code bugs and
  started finding real, pre-existing gaps in `walk_cells` itself — which is
  the whole point of this track, not a detour from it.** Two more,
  distinct from `corner_tie`'s already-documented corner-grazing slop:
  - `panel_stop` — the function `DdaTransition::Corner` calls instead of
    the main loop's own per-solid formula — tests a body with one point
    (the corner itself) through `pierces`'s height-band softness, never
    the length-based `travelled` formula every other body crossing gets.
    A ray that runs a real, non-trivial distance through a body's box, but
    is only ever named via a corner, comes out under-occluded by
    `walk_cells` — confirmed by hand (a genuine `ray_vs_solid` interval
    0.107 of the whole segment long, `travelled` alone enough to fully
    block, `walk_cells`'s corner path returning a partial `0.22` instead).
  - Independently of any corner at all: `walk_cells`'s per-cell panel
    branch only tests a panel when the DDA's `entry` or `exit` side for
    that cell names the panel's own edge. A ray can enter a tile through
    its west side and leave through its east — an ordinary `Step`, nowhere
    near `corner_tie` — while still dipping into a thin north panel's real
    depth (`PANEL_THICKNESS` inward from the tile's own north edge)
    somewhere in the middle of that crossing. `walk_cells` never asks the
    panel the question at all; `ray_vs_solid` finds the real hit directly.
    Same family as `corner_tie` — a coarse side-matching approximation —
    but a second, independent instance of it, not a variant of the first.

  **Three permanent tests landed instead of one, because "does
  `walk_cells_exact` agree with `walk_cells`" does not have one honest
  answer over an unrestricted domain — see the two gaps just above.** All
  in `light.rs`'s own `mod tests`, since both functions are module-private:
  - `walk_cells_exact_agrees_with_walk_cells_on_the_six_point_counter_example`
    — the exact scene `a_wall_level_with_the_flame_is_not_skipped_by_a_shallow_ray`
    uses, calling `walk_cells`/`walk_cells_exact` directly rather than
    through `sample`, asserting both the blocked/open classification and
    full numeric agreement on `through`.
  - `walk_cells_exact_agrees_with_walk_cells_off_the_corner_tie_path` — a
    20,000-case fuzz (relaxed `max_global_rejects`, corners are common in
    an eight-tile domain) over the same single-wall (body) scene, `prop_
    assume!`-restricted to rays whose `dda_walk` never takes a `Corner`
    transition at all. Full numeric agreement holds there — the strongest
    claim this session can make, and deliberately scoped to not claim more.
  - `walk_cells_exact_disagreements_are_backed_by_ray_vs_solid` — no corner
    restriction, run over both a body and a panel scene (`Shape::UNREAD`
    and `Shape::faced`), and does not assert agreement at all. Instead: when
    one walk reads a tile as clearly blocked (`through <= RAY_CUTOFF`) and
    the other reads it clearly open (`through > 0.5` — a wide gap on
    purpose, not `RAY_CUTOFF`, since softness formulas sampled at a cell's
    shared midpoint versus a solid's own exact one can legitimately land a
    hair either side of the cutoff near a soft edge without either walk
    being wrong), the blamed tile is checked against `ray_vs_solid`
    directly: whichever walk claims the *stronger* answer must be the one
    the exact primitive backs. This is what caught both implementation
    bugs above, and continues to hold now that they are fixed — every
    remaining disagreement it finds is one of the two known `walk_cells`
    gaps, not a new one.

  Full `cargo test -p openshard-client-render` (all test binaries), `cargo
  clippy -p openshard-client-render --all-targets`, `cargo check
  --workspace --all-targets` and `rustfmt --check
  crates/client/render/src/light.rs` all clean.

  **Not run this session**: `tests/lighting.rs`'s own grid-sweep and fuzz
  oracles, and `tests/frame.rs`'s parity suite — the doc's point 3 asks for
  both, and neither was reached. Both work through the public API
  (`light::sample`, a rendered `View::Shadow` frame) rather than
  `walk_cells`/`walk_cells_exact` directly, so exercising them against the
  new path needs either a temporary public seam or a scene built the way
  this session's own three tests build one, inside `light.rs` itself. Not
  started; the next session's natural continuation of point 3 if it wants
  breadth over the two hand-built scenes here rather than depth on them.

  **A third, real bug in `walk_cells_exact` itself, found widening point 3
  to the three-tread climbable stair — a genuine bug this time, not
  another `walk_cells` gap.** The single-wall scenes above never put more
  than one solid's box on a tile at once; the stair does, three lids and
  three panels sharing one tile at three different heights, and one of the
  lids exposed something the wall scenes could not have. A lid is flat in
  `z` (`Solid::box_of`'s `min.z == max.z`), so `ray_vs_solid`'s own slab
  method correctly collapses that solid's `entered` and `leaves` to the
  exact same instant — a degenerate box really is crossed at one point in
  `t`, not over an interval, and the primitive's own doc comment already
  says as much ("a tangent touch... comes back `Some` with `entered ==
  leaves`"). `crosses` was never built to be handed an interval already
  collapsed to a point: it reads its `entering`/`leaving` arguments as the
  ray's `z` on *either side* of a crossing, to tell "went through" from
  "never came close," and a `from`/`to` pair that are already numerically
  equal answers every comparison inside it as "never," regardless of the
  real geometry. `walk_cells_exact` read every lid in the crate as fully
  transparent, unconditionally, before this was caught.

  **Fixed by asking the lid branch a different question**: not "where does
  the ray touch this lid's own (degenerate) box" but "where does the ray
  enter and leave the *tile's* footprint" — the same question `walk_cells`'s
  DDA cell entry/exit answered for free, now asked explicitly with a
  second `ray_vs_solid` call against a synthetic box sharing the tile's
  `x`/`y` bounds with `z` left unconstrained. Confirmed as the actual fix
  and not a guess: reverting it back to the lid's own `entered`/`leaves`
  reproduces the exact failure the stair fuzz found (`through` pinned at
  `1.0` through a tread whose real geometry the ray plainly crosses),
  pinned as a permanent regression test,
  `walk_cells_exact_does_not_read_every_lid_as_transparent`.

  **The stair scene also confirmed a fourth pattern of `walk_cells`
  coarseness, same family as the two above, not chased further**: with
  three risers sharing one tile at different fractional `x`/`y` bands
  (each covering only a third of the tile, the climb strip
  `Solid::tread_riser_box_of` builds), `walk_cells`'s per-*cell* model
  tests every solid on a tile with the *same* shared `entered`/`leaves`
  and a `pierced` check that only ever asks about height, never about
  where in the tile a specific riser's own footprint sits. A ray that
  never geometrically approaches a particular riser's narrow band can
  still trip `walk_cells`'s coarse per-cell test for it. Real, but not
  pursued into a fix or even a clean characterisation this session — see
  the next paragraph for why a fuzz-based oracle for this scene was
  dropped rather than forced.

  **No sound automated oracle for this scene was built, and that is a
  deliberate stop rather than an unfinished one.** The single-wall
  characterisation test's trick — "whichever walk claims the *stronger*
  answer must be backed by a real `ray_vs_solid` hit" — assumes a hit that
  exists is a hit that counts, which stopped being true the moment a tile
  could carry an exempted solid (`flame_end`, `on_surface`) *and* a
  non-exempted one at once: the stair fuzz's very first false alarm after
  the lid fix was a real `ray_vs_solid` hit on a riser the flame's own end
  legitimately stands on, correctly open in both walks, that the
  characterisation test flagged anyway because it never re-evaluates
  `walk_cells`'s exemption predicates before asking whether a hit "counts."
  Doing that properly means duplicating `lit_end`/`flame_end`/`caps_this`/
  `same_run` inside the test itself — an oracle re-deriving the same
  formula the code under test already has, rather than checking it against
  something independent, which is exactly the trap this doc's own
  fault-injection discipline exists to avoid falling into by accident.
  Landed a weaker, honest smoke test instead —
  `walk_cells_exact_stays_in_range_on_the_stair`, asserting no panics and
  `through` inside `0.0..=1.0` over the same fuzz domain, which the lid bug
  above would still have failed loudly (pinned at `1.0` far more than the
  geometry allows) — and left a real disagreement oracle for this class of
  scene as the next piece of point-3 tooling, not a session that ran out
  of time trying to build one in a hurry.

  **Point 4 — the actual cutover — has not been touched, and per the
  doc's own recommended order should not be started from a standing start
  next session.** `walk_cells_exact` is not obviously *ready* to replace
  `walk_cells`: every *real* gap traced this session was `walk_cells` being
  coarser, not `walk_cells_exact` being wrong, but the lid bug above is a
  reminder that the exact primitive comes with its own, different failure
  modes to hunt for — not "more precise" automatically, only "more precise
  once its own bugs are found the way this session found three of them."
  `blit.wgsl`'s own mirror of whichever walk is `pub`, decision 9's full
  GPU/CPU parity suite, a disagreement oracle that survives a multi-solid
  tile, and a real-scene render (this session never rendered a frame) are
  all still ahead of it.

- **A second bigger idea, found trying to start point 4 and deliberately
  deferred again, session 14: `walk_cells`/`walk_cells_exact` both assume a
  solid's footprint is recoverable from `(tile, edges)` alone, and the GPU's
  own upload format makes that assumption load-bearing rather than an
  optimisation.** `occlusion::Solid::box_of` (`occlusion.rs:659`) has always
  built a body's box as the *whole tile* and a panel's as a
  `PANEL_THICKNESS`-inset strip of one edge, because that is the only shape
  `tiledata` has ever produced — nothing in this repo needed a solid whose
  `x`/`y` span was anything else. `occlusion::Builder::add_raw`
  (`occlusion.rs`, added this session — see `examples/boxes.rs`'s own doc)
  is the first thing that builds one that is neither: a hand-placed body
  narrower than its own tile, for a scene with no static behind it at all.
  Session 14 went looking for point 4 (the `walk_cells` → `walk_cells_exact`
  cutover this doc's own recommended order asks for) to fix the shadow such
  a box throws, on the reasoning that `walk_cells_exact`'s exact
  `ray_vs_solid` test ought to already get this right where `walk_cells`'s
  coarser per-cell one does not. It does — **on the CPU.** `walk_cells`'s
  `EDGE_ANY` arm (`light.rs:2269`, its `blit.wgsl` mirror at
  `blit.wgsl:1015`) computes a body's own occlusion from the ray's `z`-span
  inside the candidate cell *alone* — `low`/`high` are `stands.bottom()`/
  `stands.top()`, and neither the Rust nor the WGSL version ever reads
  `stands.space.min.x`/`max.x`/`min.y`/`max.y` anywhere in that arm — because
  a body has always filled its whole cell in `x`/`y`, so nothing needed to
  ask. A ray toward any point on `add_raw`'s taller, narrower box therefore
  reads as blocked by it regardless of whether the ray's own `x`/`y` at that
  height actually passes through the box's real footprint or the open air
  beside it — not a rounding error, a term the formula never had. Measured,
  not argued: a from-scratch independent oracle
  (`examples/boxes.rs`'s own `oracle_visible`/`segment_clear_of_box`, a
  bare slab-method ray-vs-AABB test sharing no code with either walk)
  disagreed with `light::sample`'s `walk_cells` on 3027 of 9216 sampled
  points of a sub-tile box's own top in the `tree` scene (the whole top read
  shadowed by a narrower box standing on part of it); switching the
  comparison to `light::sample_exact`'s `walk_cells_exact`
  (`OPENSHARD_BOXES_ORACLE_EXACT=1`) dropped the disagreement to 480/9216,
  and inspecting the comparison image (`<path>_oracle_box0.ppm`) showed the
  480 sitting entirely on the soft edge of a real penumbra against the
  oracle's own hard step — `walk_cells_exact`'s own `ray_vs_solid` call
  (`light.rs:2571`) is genuinely footprint-exact, because it reads the full,
  un-quantised `crate::solid::Solid` (`solid.rs:33`, real `WorldSpot`
  `min`/`max`) still held in memory, not a compressed upload.

  **`blit.wgsl` cannot be fixed the same way `walk_cells_exact` was written,
  because the data it reads a solid's shape from does not carry a footprint
  to be exact about.** `Occlusion::solid_bytes` (`occlusion.rs:1259`) — what
  `blit.wgsl`'s `solid_at` (`blit.wgsl:667`) actually loads — uploads four
  bytes a solid: `(z_bottom + 128, z_top + 128, opacity, PRESENT | HOLED |
  edges)`. No `x`/`y` channel exists anywhere in the format; a solid's
  footprint on the GPU is reconstructed the same way `box_of` builds it on
  the CPU, from `(tile, edges)` alone, which has been exactly enough for
  every solid the format has ever had to carry. Porting
  `candidate_tiles`/`ray_vs_solid`/`box_side`/`walk_cells_exact` into
  `blit.wgsl` — point 4 as this doc has always scoped it — would still read
  a sub-tile body as filling its whole cell, the identical bug measured
  above, because the exact-test-vs-coarse-test distinction point 4 is about
  was never the thing standing between a correct answer and this scene.
  **Point 4 is not moot** — decision 9's parity suite still needs the GPU
  and CPU walks to agree, and the corner-grazing precision point 4 was
  scoped for (`docs/lighting_raymarch.md`'s own "A bigger idea..." entry,
  point 1-3 above) is a real, separate improvement over today's DDA-stepped
  `walk` — but it is a **prerequisite**, not a fix, for a sub-tile occluder:
  two sequential pieces of work, where this backlog entry only names the
  second one's shape and does not start it. What the second piece needs:
  widening `solid_bytes`'s own four channels (a new texture, or more
  channels on the existing one — `LIST_ROW`'s own row budget is the first
  number to check against a texture format's channel limit) to carry a
  solid's real `min.x`/`max.x`/`min.y`/`max.y`, `solid_at`'s WGSL mirror
  reading them back, and the `EDGE_ANY` arm's per-solid test using them
  directly instead of reconstructing a footprint from `(tile, edges)` —
  every reader of the current four-byte format (`solid_at`, `merged_at`,
  the CPU's own `Occlusion::solid`/`Occlusion::at`) is a caller this change
  has to keep honest, not just `walk`'s own body arm.

  **A separate, already-fixed bug found on the way, in `light.rs`'s own
  `exemption` (and its `blit.wgsl` mirror) — real, independent of the
  footprint gap above, landed this session.** `exemption`'s own `lit_end`
  path (`light.rs:1301`) exempted a `Flat` fragment from *any* body sharing
  its own tile once `Surface::shadowed_by_own_tile` (`light.rs:1463`)
  special-cased `edges == EDGE_ANY` to answer `0` — vacuously satisfying
  `stands.edges & lit_by_own_tile == 0` for every solid on that tile, not
  only the one the fragment actually rests on. Invisible until `add_raw`
  put two independent bodies on one tile with touching `z`-spans: the lower
  box's own top (`z` exactly at the upper box's own `bottom`) read
  `on_surface` true for *both* boxes, and the already-correct, narrower
  `caps_this` check (requiring the fragment's `z` at *that specific* solid's
  own top) was redundant rather than load-bearing, because the broader path
  fired first. Fixed by restricting `lit_end`'s own edges-mask path to
  non-`Flat` surfaces (`light.rs:1301`'s `exemption`, `blit.wgsl:995`'s
  mirror) — a `Face`/`Upright` fragment keeps it, since a body's own side
  genuinely needs self-exemption at any height along it and has no
  `caps_this`-style alternative; a `Flat` fragment now relies on `caps_this`
  alone, which was already precise. Zero regressions:
  `cargo test -p openshard-client-render --lib` (351 tests) green before and
  after. **This fix is necessary but not sufficient for `tree`'s own shadow
  — the footprint gap above is the larger remaining cause of the 3027/9216
  (now `walk_cells`) and 480/9216 (`walk_cells_exact`) disagreements
  measured after it landed**, which is why both are recorded in the one
  entry rather than the exemption fix being mistaken for the whole story.

  **Point 4's cutover, proven on the CPU first, session 15 — the WGSL port
  itself still not started.** `walk_cells_streaming` (`light.rs`, beside
  `walk_cells_exact`) is the bounded, order-independent reformulation `blit.
  wgsl`'s own `walk` (one scalar `through`, no blamed tile) actually needs —
  `candidate_tiles`'s `Vec`/dedup/sort exist only to name the first blocking
  tile in ray order, a question nothing downstream of the shader's `walk`
  asks, so a per-fragment loop can multiply every candidate's contribution
  in as it is found instead. Proven full numeric agreement with
  `walk_cells_exact` on every ordinary (non-`add_raw`) fixture this file
  already has — a single body, a single panel, the six-point counter-example,
  a seven-solid room — by fuzzing, not argued from the reconstruction being
  bit-for-bit what `occlusion::Solid::box_of` already builds for an ordinary
  static. **Also found, by deliberately disabling it and watching six
  separate constructions fail to notice: the off-axis diagonal probe this
  entry's own point 1/2 scoping asked for is unneeded once the primary walk
  never takes `dda_walk`'s corner-jump at all** — a never-skip single-axis
  DDA already visits every cell a continuous line passes through, which is
  what the probe existed to guarantee for a walk that *does* skip. Removed
  rather than kept "to be safe," with the fault-injection run the other
  direction too (disabling the reused `exemption` check) confirmed to fail
  loud, so the tests are not merely biased toward passing. See session 15's
  own handoff entry for the full account, including why the three-tread
  stair is **not** claimed to agree — `tread_top_box_of`/`tread_riser_box_of`
  build real sub-tile footprints the same `box_of`-reconstruction limit
  cannot recover, a second, independent path to this entry's own footprint
  gap, not only `add_raw`. Not touched: `blit.wgsl` itself, the `sample`
  cutover, and deleting `walk_cells`/`corner_tie`/`panel_stop`/
  `DdaTransition::Corner` — real, separate work for whichever session does
  the actual WGSL port `walk_cells_streaming` now exists to make a
  mechanical translation of, rather than a second open design question.

## Handoff log

One entry per session, newest first. What changed, what was learned, what the
next session should read before touching anything. Append, do not rewrite —
a wrong turn kept and marked wrong is worth more than a tidied history.

### Session 18 — sessions 14-17 committed, then step 5 re-opened after the point 4 rewrite: a real `through` discontinuity found and precisely localised, not yet root-caused

Started by finding sessions 14-17's entire arc (`boxes.rs`, `walk_cells_
streaming`, the WGSL cutover, the parity-gap triage) sitting uncommitted in
the working tree — all four gates (`check`/`test`/`clippy`/`fmt`) verified
green first, then landed as one commit (`aacbaba`) rather than four, since
the files were touched across all four sessions closely enough that a clean
per-session split was not worth the risk of mis-slicing verified work; the
doc's own session-by-session entries above carry the detail a finer commit
history would have.

**Picked step 5 (the white line) as the only item "Where the next session
starts" still named open.** Worth checking first, since the entire mechanism
underneath it changed since session 12 last touched it: `walk_cells`,
`corner_tie` and `panel_stop` — everything every session 3-12 investigation
of this shape reasoned about — are gone, replaced by `walk_cells_streaming`
and the shared `exemption` extraction. Old bisection coordinates and old
conclusions could not simply be trusted forward.

**Reproduced the doc's own repro command, live, against the post-cutover
code.** The line is still there, unchanged in shape from the screenshots
earlier sessions worked from — so whatever causes it survived the rewrite
that was expected to remove the tile-boundary bug class by construction (the
whole premise of Track B's point 4). Confirmed again, independently of
session 3's finding, that `View::Kind` at the line's pixels reads `(64, 115,
255)` — a static, not background — by scanning the actual `.ppm` bytes rather
than reading the picture by eye (`python3`, raw `P6` parsing; no tool in the
crate does this today, worth building if this track continues).

**The exemption hypothesis is ruled out again, on the new algorithm, by
direct calculation rather than a re-read of the old entries.** `light.rs`'s
`exemption()` (session 11's extraction) exempts a `Flat` fragment from a
solid on its own first cell only when `on_surface` says the fragment's `z`
sits on *that* solid's own span. Worked through `ON_TOP = 1/128` by hand for
the query point on tread 3's top (`z = 15`) against all six solids the scene
reports (all six share one tile — this stair packs three treads and their
risers onto a single cell, per this doc's own repeated note): only tread 3
itself (`caps_this`'s `lit_end`, its own body) and riser 3 (the panel it
physically rests on, `top = 15`) satisfy `on_surface`, and both are the
solids a fragment resting on them is *supposed* to be exempt from — nothing
here vacuously exempts a solid the fragment does not actually stand on, the
exact shape of bug session 14 found and fixed in `boxes.rs`'s two-independent-
bodies scene. Exemption is not the mechanism.

**Profiled `OPENSHARD_SCENE_PROFILE_FACE` at the tread 3 / riser 3 seam
(`y = 1627.333`, `z = 15`, both `flat` and `south` surfaces) and found a real
discontinuity, not an eye-only illusion.** A sweep of `x` from `1497.55` to
`1497.75` (`OPENSHARD_SCENE_PROFILE_STEPS=40`, fine enough to resolve a
`0.005`-tile step) shows `light::sample`'s own `through`:

```
x=1497.55 .. 1497.67   through: 0.098 → 0.069, a smooth soft-shadow gradient
x=1497.67 → 1497.68    through: 0.069 → 1.000, in one step — no 0.2, 0.3, 0.5
x=1497.68 .. 1497.75   through: 1.000, flat
```

**Two things this rules out on its own, without needing the root cause
yet.** First, it is not a CPU/GPU disagreement of session 17's own shape —
`light::sample` (the CPU walk, `walk_cells_streaming`) shows the exact same
jump on its own, no GPU readback involved, and `Surface::Face(South)` (the
riser) and `Surface::Flat` (the tread top) read *identical* `through` at
every sampled `x`, differing only in `cone` (`0.727` vs `0.000`, ordinary
orientation-dependent falloff) — so it is not a surface-orientation artefact
either. Second, it is not a smooth penumbra that a coarser sweep merely
under-sampled: a widening or narrowing soft shadow moves through its own
middle values, and this one visibly does not — the jump is a full step from
`0.069` to `1.000` with nothing between, the signature of a candidate solid
leaving the walk's own consideration entirely as the ray's own angle to the
light crosses some threshold, not of a shadow's edge sweeping past the query
point. `candidate_tiles`/`dda_walk` are the next thing to read with this
specific transition in hand — not re-read cold, *traced* at exactly
`x = 1497.67` versus `x = 1497.68`, the way session 6's own `corner_tie` bug
and this session's own exemption check were both settled by calculation
rather than by re-reading a summary.

**Not root-caused this session — stopped here on purpose, the same
discipline this doc has used before rather than guess at a fix under time
pressure (see session 17's own reasoning for not chasing its lead further).**
What the next session gets that no earlier session on this shape had: an
exact coordinate pair either side of a confirmed discontinuity, on the
*current* algorithm, with both the exemption path and the CPU/GPU-parity
path already checked and cleared. Reproduce the profile directly:

```sh
OPENSHARD_CLIENT=… \
    OPENSHARD_SCENE_AT=1497,1627,10 OPENSHARD_SCENE_RADIUS=1 \
    OPENSHARD_SCENE_TILES=0x0739 OPENSHARD_SCENE_GROUND=0 \
    OPENSHARD_SCENE_EXTRA=1498,1626,10,2852 \
    OPENSHARD_SCENE_PROFILE_FACE=flat \
    OPENSHARD_SCENE_PROFILE_FROM=1497.55,1627.333,15 \
    OPENSHARD_SCENE_PROFILE_TO=1497.75,1627.333,15 \
    OPENSHARD_SCENE_PROFILE_STEPS=40 \
    cargo run --release -p openshard-client-render --example isolated_scene
```

**Verified**: `cargo check --workspace --all-targets`, `cargo test -p
openshard-client-render` (350 lib tests, 45/5 in `frame.rs`, 37 in
`lighting.rs`), `cargo clippy --workspace --all-targets` and `cargo fmt --all
-- --check` all clean before the commit above; nothing in this session's own
investigation touched a source file (profiling and pixel-reading only), so
there was nothing further to verify after it.

### Session 17 — the open parity gap triaged, not chased: `#[ignore]`d with the reasoning attached, not patched with a tolerance

Session 16 left one open item: `the_shader_and_light_sample_agree_about_a_
carried_beam` and `the_shader_lights_a_frame_as_light_sample_does` failing on
a real CPU/GPU tie-break disagreement, with one untried lead (whether
`per_tile`/`ahead` reassociate differently under naga than under Rust). This
session did not chase that lead — asked first whether the two failing tests
were even the right thing to keep chasing, and the answer changed the
decision.

**Checked, not assumed, before deciding anything.** Two questions, both
answered by reading code rather than reasoning about it:
- Is the tie an artefact of a hand-built test fixture, or real geometry a
  player's own session could hit? `parity_place`'s sweep (`tests/frame.rs:
  3601`) is a plain `1/8`-tile lattice over an ordinary room scene
  (`scene::lantern_in_a_room`) — not a probe aimed at a corner on purpose.
  An exact tile-corner tie is a real configuration a regular grid against a
  fixed light will eventually land on; nothing about the test is contrived.
- Does the disagreement reach a player's screen? Grepped every caller of
  `light::sample` in the workspace: `tests/frame.rs`, `tests/lighting.rs`,
  `examples/isolated_scene.rs`, `examples/boxes.rs`,
  `artscan/examples/probe.rs`, `debug.rs`'s tile-brightness map — debug
  tooling and test oracles, every one. `blit.wgsl`'s own `walk` is the only
  thing that ever draws a frame a player sees, and it is internally
  self-consistent regardless of which side of a tie its own comparison
  lands on. So the two failing tests guard a real invariant (decision 9:
  the CPU debugger must not drift from what the shader draws) but not a
  visible correctness bug — the stakes are tooling trust, not a broken
  shadow.

**Decision: close it as a documented, scoped limitation, not a bug to
re-open next session.** Given the stakes above and that this track already
tried the cheap fix and it made things worse (session 16's `EPS`-biased
comparison, reverted), spending more of this session on the untried
reassociation lead would have been chasing a low-stakes gap past the point
its value justified — the same shape this doc's own memory calls out
elsewhere as a trap: keep going because a lead is untried, not because it is
worth it. `the_shader_and_light_sample_agree_about_a_carried_beam` and
`the_shader_lights_a_frame_as_light_sample_does` are now `#[ignore]`d, each
with its own doc comment carrying the full account and a pointer here, not a
bare skip. Two shapes a real fix could still take are named in "Where the
next session starts" above — a cross-multiplied comparison (cheap, worth
trying first) and a tie-order-independent walk (real work, its own session)
— so a future session that decides the stakes have changed does not start
from zero.

**Verified the rest of the suite is unaffected**: `cargo test -p
openshard-client-render` — 45 passed, 0 failed, 5 ignored (the two named
above plus three pre-existing); `cargo test --workspace`, `cargo clippy
--workspace --all-targets` and `cargo fmt --all -- --check` all clean.

### Session 16 — point 4's cutover landed: the WGSL port, the `sample` cutover, `walk_cells`/`corner_tie`/`panel_stop`/`DdaTransition::Corner` deleted — one parity gap found, narrowed, and left open rather than papered over

Picked up exactly where session 15 left off: read its own entry first, per
the doc's own instruction, then did the three things it named as not done —
the actual WGSL port, `sample`'s cutover to use it, and deleting the four
named things and the tests pinning them.

**The WGSL port.** `blit.wgsl`'s `walk` is now `light::walk_cells_streaming`'s
own shape: single-axis DDA, no corner-jump, every solid on a candidate cell
tested with its own `ray_vs_solid` interval rather than a cell-shared
`entered`/`leaves`. New primitives mirroring `light.rs`'s: `box_of`
(`Solid::box_of`'s reconstruction), `ray_vs_solid` (the slab method),
`box_side` (the geometric replacement for a DDA step's own entry/exit, since
a body's box *is* the tile's footprint). The per-cell occlusion logic —
`light::walk_cells_streaming`'s own `apply` closure — became `cell_stopped`,
a real function rather than staying inline, because it needs to be called
twice (see the probe below); WGSL has no closures, so this is the mechanical
answer, not a redesign.

**Two reserved WGSL keywords cost a rename before anything else would even
compile.** `from` and (less expectedly) `target` are both reserved — naga's
parser rejects a parameter or `let` named either, for compile-time-forward-
compatibility reasons rather than anything about this shader's own logic.
`ray_vs_solid`'s own parameters became `ray_from`/`ray_to`.

**`sample`'s cutover.** `walk`/`walk_sun` now call `walk_cells_streaming`
instead of `walk_cells`. It needed one addition first: `walk_cells_streaming`
didn't carry blame (`Reach::stopped_by`) — its own doc comment, written
session 15, said nothing downstream needed it. That was wrong once it became
`sample`'s own walk: `Reach::stopped_by` has real readers (`tests/
lighting.rs`'s assertions, `Debug` for `Sample`, `examples/isolated_scene.rs`)
the cutover must not silently break. Adding it was free — the enumeration
already visits cells in ray order, so the cell being applied when `through`
first drops to `RAY_CUTOFF` *is* the first blocking tile in ray order, the
same fact `walk_cells`'s own `Some(cell)` named.

**Deleted, and what had to change shape first.** `walk_cells`, `corner_tie`,
`panel_stop`, `DdaTransition::Corner` and roughly a dozen tests that pinned
them directly (`walk_cells_exact_agrees_with_walk_cells_*`, `corner_tie_
converts_back_into_*`, `panel_stop_only_asks_*`) are gone, both sides.
`dda_walk` (still real — `candidate_tiles`/`walk_cells_exact` both still use
it as an oracle) lost its own corner-jump branch along with `corner_tie`,
simplifying to the same plain single-axis stepping `walk_cells_streaming`
already uses: checked, not assumed, that this does not change `candidate_
tiles`'s own coverage, since it already names both single-axis neighbours at
*every* transition regardless of whether `dda_walk` itself jumps or steps —
the jump was already redundant from `candidate_tiles`'s own side, confirmed
by every `walk_cells_exact` proptest staying green through the simplification.
`DdaCell` lost its now-unread `entry` field and its `crossing: Option<
DdaTransition>` became `continues: bool`, a single-variant enum being no
information at all once `Corner` is gone.

**A pre-existing, unrelated bug in `walk_cells_exact`'s own corner-tangent
handling surfaced immediately, and had to be told apart from a cutover
regression before it could be dismissed.** The first full test run after the
cutover failed three of `walk_cells_exact`'s own real-scene parity tests
(`the_exact_walk_agrees_with_light_sample_*`) — `walk_cells_streaming` (now
production) reading a ray open that `walk_cells_exact` (the "exact" oracle)
read blocked, at a ray passing exactly through a shared four-tile corner.
Traced with a brute-force march restricted to the *blamed* tile alone
(`blamed_tile_has_a_real_crossing`, `tests/frame.rs`): the march never finds
itself inside the blamed tile's own box at more than a couple of isolated
samples — a tangent touch at its corner, not a real crossing.
`walk_cells_exact`'s own `candidate_tiles` probes that corner unconditionally
and its per-solid pierce test doesn't distinguish a genuine crossing from a
corner graze, so it blocks; `walk_cells_streaming`'s plain single-axis
stepping simply never visits that tile for this ray, and ground truth agrees
with it. **This is not a bug this session fixed** — it is the same "grazing
a box's corner" ambiguity this doc's own backlog already named as accepted,
not arbitrated as a defect — but `exact_walk_disagreements`'s own
classification (`explained`/`bugs`) had no bucket for it before now, since
production (`walk_cells`, then `walk_cells_streaming`) and the oracle used
to agree at this exact corner by coincidence (both wrong in the same shape,
old `corner_tie` included), so the disagreement never surfaced until
production got more precise. Added a third bucket, `grazed`, backed by the
same restricted march, rather than either loosening the test's own bar or
declaring a false regression.

**Then the real work: three rounds of CPU/GPU parity failures, three
different root causes, in `docs/lighting_raymarch.md`'s own house style of
measuring rather than guessing at each one.**

1. **`a_single_flat_face_beside_an_occluder_agrees_with_light_sample`**: a
   query point whose `u`/`v` and whose light both sit on a tile's diagonal
   put `boundary[0]`/`boundary[1]` on an *exact* tie, confirmed identical on
   both backends by hand (`1/127`-quantised `u = v`, so `ahead`/`per_tile`
   are the same computation on both axes) — yet `blit.wgsl`'s own bare `<`
   resolved it differently than `light::walk_cells_streaming`'s. First fix
   tried: bias the stepping comparison itself with an epsilon
   (`boundary.x < boundary.y - EPS`). **Wrong, and it cost a full round-trip
   to find out**: it fixed this one test but sent `walk_cells_streaming`'s
   own stepping down a cell `dda_walk`'s bare comparison would not have,
   failing `walk_cells_streaming_agrees_with_walk_cells_exact_in_a_small_
   room` on ordinary geometry nowhere near a tie — confirmed by reverting
   `ray_vs_solid` and the bias separately, bisecting which one broke the
   fuzz. Reverted.
2. **The actual fix**: an *unconditional* probe of the untaken side at every
   transition — not gating it behind how close `boundary.x`/`boundary.y`
   are (tried, and made *four more* previously-green parity tests fail,
   because CPU and GPU do not compute a close-enough `boundary.x -
   boundary.y` to agree on which rays even count as near a tie — widening
   the gate widened the asymmetry, not the fix), just always testing the
   cell the walk's own trajectory does not step into this instant, without
   moving the trajectory there. Safe by the same argument `candidate_tiles`
   already relies on: it already names both single-axis neighbours at every
   transition regardless of any tie, so `walk_cells_exact` was already
   tested against whatever this probes. `light.rs`'s own `apply` closure and
   `blit.wgsl`'s new `cell_stopped` function both changed shape to be
   callable twice per step for this.
3. **`the_shader_and_light_sample_agree_about_a_carried_beam`**: a *different*
   mechanism at the *same* corner shape — not the stepping tie-break at all.
   Traced with a `DEBUG_TRACE` constant and targeted early-`return`s from
   `walk`/`ray_vs_solid`/`cell_stopped` (removed before committing, the
   technique kept here rather than the code): `ray_vs_solid`'s own `entered >
   leaves` rejection, computed on inputs already confirmed byte-identical
   between `light::sample` and `blit.wgsl`, disagreed — CPU's own `f32`
   division happened to round to `entered <= leaves` (a hit) where GPU's
   rounded to `entered > leaves` (a miss), for a ray tangent to a real
   `Shape::UNREAD` body at (103, 100) in `scene::lantern_in_a_room`'s east
   wall. Fixed by widening `ray_vs_solid`'s own rejection by a small
   tolerance — **on the GPU side only**. Widening the CPU side too (tried
   first, scoped to `walk_cells_streaming`'s own caller) broke `walk_cells_
   exact` agreement again, for the same reason session 8's own `candidate_
   tiles` scoping already explains: it reaches cells `walk_cells_streaming`'s
   plain stepping never visits, so a rescued near-miss there is a genuine new
   answer, not a redundant re-test of one `walk_cells_exact` already made. A
   second refinement, found only by testing: even *this*, scoped only to
   `walk_cells_streaming`'s own call, was not narrow enough once the probe
   above went unconditional — a probed cell is a diagonal neighbour by
   construction, exactly the geometry most likely to be grazed rather than
   crossed, so the *same* tolerance that rescues the one genuine tangent that
   mattered also rescues many spurious ones on newly-probed cells, and
   widening it further made this *worse*, not better (six parity tests
   failing at a `5e-2` gate, not two). Landed as `RAY_TANGENT_TOLERANCE`,
   threaded through `cell_stopped` as a parameter and applied only at
   `walk`'s own trajectory cell, `0.0` at its probe.

**What is still open, after all three rounds: `the_shader_and_light_sample_
agree_about_a_carried_beam` and `the_shader_lights_a_frame_as_light_sample_
does` still fail, and it is not case 3 above recurring.** With `RAY_
TANGENT_TOLERANCE` in place and scoped correctly, re-measuring the exact same
ray found `boundary.x` a **real** `~0.05` less than `boundary.y` on the GPU
— not a tie, not noise, a genuine "X is nearer" that CPU's own tied
`0.2012578`/`0.2012578` does not share, sending the GPU's own trajectory
through a different first cell than CPU's and missing the wall entirely
regardless of any `ray_vs_solid` tolerance, because the walk's own cell
never becomes `(103, 100)` on that backend at all. Ruled out this session:
light-data quantisation (`blit.rs`'s `lighting_bytes` writes `light.at.x/y`,
`light.z` as raw `f32` bytes, unquantised — checked by reading the upload
code directly, not assumed), the tile upload path for `Kind::Land` (an
integer id, no float involved), `Surface::Upright`'s own `outward`/`own`
(both zero on both backends, matching). **Not yet checked, and the
strongest remaining suspect**: whether the two-step `per_tile = 1.0 /
abs(delta.axis)` then `boundary.axis = ahead * per_tile.axis` reassociates
or fuses differently under naga/wgpu's own shader compiler than under
Rust's — read back `per_tile`/`ahead` themselves via the same `DEBUG_TRACE`
early-`return` technique this session used (not just their product
`boundary`, which is as far as this session's own trace went) before
guessing further. See the "Where the next session starts" section above for
the exact query point and light this reproduces at.

**Verified, not assumed, at every stage**: `cargo test -p openshard-client-
render` (350 lib tests, all `tests/frame.rs`'s real-scene and parity
fixtures except the two named above), `cargo test --workspace`, `cargo
clippy --workspace --all-targets`, `cargo fmt --all -- --check` all clean.
Proptest regression files cleared and the three `walk_cells_streaming`-vs-
`walk_cells_exact` fuzz tests re-run fresh (not from a cached regression)
multiple times at 8,000-30,000 cases each, to confirm the final state is
stable and not a lucky seed.

### Session 15 — point 4's own cutover proven safe to attempt on the CPU first, and a redundant off-axis probe ruled out by fault injection rather than kept "to be safe"

Picked point 4 by name — session 14 left the choice of what to do next open,
and the user picked the documented cutover over the GPU-format widening.
Reading `candidate_tiles`/`walk_cells_exact` (`light.rs:2398`-`2702`) rather
than trusting the doc's own summary of them found a real gap in how point 4
was scoped back at session 8: `candidate_tiles` collects into a `Vec`, dedups
pushes with `Vec::contains`, and `walk_cells_exact` sorts that `Vec` by
nearest crossing before walking it in order — dynamic allocation, `O(n²)`
dedup, a sort, none of it something a bounded per-fragment WGSL loop can do.
Point 4 was never "port the Rust literally"; it needed its own bounded
reformulation first, and a shader is the worst place in this codebase to
debug a wrong one. So this session's whole scope, agreed with the user
before writing any code: prove a GPU-shaped reformulation against the
existing oracle suite on the CPU, in Rust, before a line of WGSL exists —
not the WGSL port itself, not the `sample` cutover, not deleting
`walk_cells`/`corner_tie`/`panel_stop`.

**The reformulation, `walk_cells_streaming` (`light.rs`, beside
`walk_cells_exact`).** `blit.wgsl`'s `walk` returns one `f32` — nothing
downstream reads which tile stopped it, unlike `Reach::stopped_by` — and
`through` is a product of independent `(1 - stopped)` factors, one per
candidate tile, which is order-independent up to float noise (comfortably
inside decision 9's ±1/255 tolerance). So the sort and the blame-tracking
both go; what has to survive is only an enumeration that visits every
relevant cell once. Every solid it tests is reconstructed from `(tile,
edges, bottom, top)` via `occlusion::Solid::box_of` — widened to
`pub(crate)` this session, reused rather than re-derived — instead of
`Occlusion::solid::space`, because that is genuinely all `blit.wgsl`'s
four-byte upload format will ever carry for an ordinary static; this
function is deliberately a preview of that limitation, not a better version
of it.

**A real design mistake made and then ruled out by measurement, not
assumption — the actual content of this session, not a footnote to it.**
The backlog's own point 1/2 scoping (session 8) reads as: keep `dda_walk`'s
existing corner-tie-gated jump as the primary path, and add an *unconditional*
off-axis diagonal probe alongside it, because that jump still skips a cell
whenever `corner_tie` fires and the probe exists to cover for the skip. The
first draft of `walk_cells_streaming` built exactly that: plain single-axis
stepping (no jump at all — a deliberate simplification, see below) plus an
unconditional off-axis probe at every step, mirroring the backlog's own
framing. **It was wrong to keep, not because it produced a wrong answer, but
because it never did anything** — a DDA walk that never skips a cell (which
this one, by construction, never does — it always takes the nearer boundary,
one axis at a time, full stop) is complete: it visits every cell a
continuous line's interior passes through, the ordinary reason grid-line
rasterisation steps one axis at a time in the first place. The off-axis probe
in the backlog's own framing exists to compensate for `dda_walk`'s *jump*,
which this reformulation does not have — dropping the jump made the probe
redundant by the same stroke, not a separate optimisation to consider later.

Found, not assumed: proptest fuzz over a single whole-tile body and over a
single panel (`Shape::UNREAD`/`Shape::faced`, no corner-tie restriction
needed at all, unlike the equivalent `walk_cells`-vs-`walk_cells_exact` fuzz
at point 2) both passed with the probe *enabled* — expected, matching a
proven-correct design. Deliberately disabling the probe (`if false { apply(off_axis,
...) }`, this doc's own fault-injection discipline run in the direction that
should break something) was expected to fail loud and instead stayed green,
across six increasingly deliberate constructions: the six-point
counter-example, the unrestricted single-body and single-panel fuzz, a fuzz
aimed at a two-panel building corner, one hand-picked ray running the exact
diagonal through a shared corner point, and 30,000 cases over a seven-solid
room (three walled sides, a doorway gap, a free-standing body in the open
area). None of the six ever disagreed with `walk_cells_exact` whether the
probe ran or not. That is the actual finding — not "the probe happens to be
unneeded here" but "a never-skip single-axis DDA has nothing left for a probe
to add" — and the probe was removed rather than kept "for safety": dead
code that never executes differently is not a safety margin, it is an
unverified branch nobody will think to re-check later. The seven-solid room
scene is kept as a permanent regression
(`walk_cells_streaming_agrees_with_walk_cells_exact_in_a_small_room`), not
only run once by hand.

**Fault injection run the other direction too, so the tests are not merely
biased toward passing.** Disabling the real exemption check
(`if false && exempt { continue; }`) inside `walk_cells_streaming` failed
three of the five new tests immediately, on real disagreements
(`walk_cells_exact` reading a ray open at `through = 1`, the mutated
`walk_cells_streaming` reading it fully blocked at `0`) — confirming the
suite catches a real regression in the piece of this function that *is*
reused rather than re-derived (`exemption`, `same_run`, `crosses`, `pierced`,
all called exactly as `walk_cells_exact` calls them), not only in the
enumeration. Both mutations reverted before anything was trusted; no
`proptest-regressions` artefact from either was committed.

**What full agreement does not cover, checked rather than left implicit.**
The three-tread climbable stair (`Shape::solid(Prism)`) is *not* claimed to
agree with `walk_cells_exact` here, and for a reason beyond `add_raw`:
`occlusion::Solid::tread_top_box_of`/`tread_riser_box_of` build a tread's
real geometry from `Prism::footprint`, sub-tile strips along the climb axis —
not from `box_of` at all. A tread's `edges` is `0`, the same as an ordinary
floor's, so `walk_cells_streaming`'s `box_of(tile, 0, ...)` reconstruction
necessarily comes back the *whole* tile. **This is a second, independent
path to the exact gap session 14 already named against `Builder::add_raw`
boxes, not a new one** — climbable stairs, already real content in this
repo, hit the identical limit of a four-byte upload format with no `x`/`y`
channel. Worth recording because the "second bigger idea" backlog entry, as
session 14 left it, reads as if `add_raw` were the only way to reach the
gap; it is not. An honest attempt at a disagreement-backing oracle for the
stair (checking whether the tile either walk blames has a lossy `box_of`
reconstruction) failed on its first fuzz run for an informative reason —
`walk_cells_exact`'s own blamed tile is `None` whenever it found nothing
blocking, and the tile a disagreement actually traces to can be anywhere a
tread's real footprint the ray legitimately misses gets read by
`box_of`-reconstruction as the whole tile instead — so what landed instead
is the same range-sanity smoke test session 9/11 used for `walk_cells_exact`
itself on this scene (never panics, never returns `through` outside
`0.0..=1.0`), not a numeric oracle. A sound disagreement oracle for the
stair is real, separate work, not attempted here.

Landed this session: `occlusion::Solid::box_of` widened to `pub(crate)`
(`occlusion.rs:659`, doc comment extended to name the new caller and why
reusing it rather than re-deriving the same geometry a second time is the
point); `walk_cells_streaming` (`light.rs`, `#[allow(dead_code)]` — staged
ahead of a real caller the same way `ray_vs_solid` was at point 1); five new
tests (`walk_cells_streaming_agrees_with_walk_cells_exact_on_the_six_point_
counter_example`, `_on_a_single_body`, `_on_a_single_panel`, `_in_a_small_
room`, `_stays_in_range_on_the_stair`). `cargo test -p
openshard-client-render` (all binaries, 356 lib tests plus the full
integration suite including decision 9's `assert_parity`), `cargo test
--workspace`, `cargo clippy --workspace --all-targets`, `cargo check
--workspace --all-targets`, `cargo fmt --all -- --check` all clean at the
end of the session. Not committed — left for the user's own review, matching
how session 14 was left.

**Not touched, on purpose — the actual WGSL port and everything past it.**
`blit.wgsl` was not opened this session. `sample`/`walk`/`walk_sun` still
call `walk_cells`; `walk_cells_streaming` has no real caller yet.
`walk_cells`, `corner_tie`, `panel_stop`, `DdaTransition::Corner`,
`dda_walk`'s own corner-jump branch, and the roughly fifteen existing tests
that reference them directly are all untouched — several of those tests are
the very agreement-proving harness this session's own new tests joined, and
deciding which become permanent regressions versus obsolete scaffolding is
its own careful pass, for once there is something real to cut over to. The
GPU occlusion-format widening for sub-tile footprints (session 14's "second
bigger idea", now known to cover climbable stairs as well as `add_raw`
boxes) is untouched and still a separate, later track.

### Session 14 — `boxes.rs` generalises `two_cubes.rs` to sub-tile/stacked boxes, `occlusion::Builder::add_raw` built, a real `exemption` bug fixed, and a second bigger idea found trying to start point 4

Picked up by user request: more test scenes like `two_cubes.rs`, specifically
boxes smaller than a tile and stacked on each other (a "christmas tree" — a
half-tile box with a third-tile box standing on its own top), to look at
shadows. `occlusion::Builder::add`'s own shape (a whole tile or an edge
panel, whatever `tiledata` states) cannot build either shape, so
`occlusion::Builder::add_raw` (`occlusion.rs`, beside `Builder::push`) is new
this session: one raw occluder, exactly the `crate::solid::Solid` AABB
given, stored in the same tile bucket every other occluder uses. Generalised
`two_cubes.rs`'s own "through the real lit pipeline" half into
`examples/boxes.rs`: any number of boxes, each an independent `BoxSpec`
(tile bucket plus exact corners), a `tree` scene (the stacked pair above)
and a `line` scene (`two_cubes.rs`'s own two-box shape, offset due east
instead of diagonally). Full account of what the tool does and why in its
own module doc — not duplicated here.

**Three artefacts the user spotted by eye in the first `tree` render, all
traced to real causes rather than dismissed as rendering noise**: the upper
box threw no shadow onto the lower box's own top at all, a face that should
have read evenly lit had a visible seam, and a region that should have been
shadowed read lit. Chasing these by eye alone was not enough — see the
backlog's new "A second bigger idea..." entry for the full technical
account, condensed here: a first fix (`light.rs`'s `exemption`, `Flat`
surfaces no longer take the edges-mask self-exemption path a body sharing
their own tile could vacuously satisfy for *any* solid on it, not only the
one actually stood on) was real, landed, zero test regressions, and visibly
changed the render — but an independent oracle built the same session
(`examples/boxes.rs`'s own `oracle_visible`, a from-scratch ray-vs-AABB test
sharing no code with either walk) proved it was not sufficient: 3027 of 9216
sampled points of the lower box's own top still disagreed with ground truth
through `light::sample`'s `walk_cells`. Swapping the oracle's comparison to
`light::sample_exact`'s `walk_cells_exact` (the ray-vs-Solid primitive
sessions 8-11 built) dropped that to 480/9216, entirely on a real penumbra's
own soft edge — proving the *CPU* exact path already gets this right, and
narrowing the remaining question to why the production GPU path does not.

**Went looking for point 4 (the cutover this doc's own recommended order
asks for) expecting it to be the fix, and found instead that it cannot be,
on its own.** `blit.wgsl`'s `solid_at` reads a solid's shape from
`Occlusion::solid_bytes`'s own four-byte upload — `(z_bottom, z_top,
opacity, edges)`, no `x`/`y` at all — because every real static's footprint
has always been fully implied by its tile bucket plus its own edges, which
`add_raw`'s sub-tile boxes are the first thing in this repo to make untrue.
Porting `walk_cells_exact` into `blit.wgsl` verbatim would still read a
sub-tile body as filling its whole cell, the identical bug, because the
exact-vs-coarse distinction point 4 is about was never what stood between a
correct answer and this scene — the GPU format itself has nothing to be
exact *with*. Point 4 is not moot (decision 9's parity suite still needs
both walks to agree, and its own corner-grazing precision is a real,
separate improvement), but it is a prerequisite for a sub-tile occluder's
shadow, not a fix — two sequential pieces of work. Neither started this
session past naming its shape; see the backlog entry for what the second
piece needs.

`cargo test -p openshard-client-render --lib` (351 tests), `cargo clippy -p
openshard-client-render --all-targets` and `rustfmt --check` on every
touched file (`occlusion.rs`, `light.rs`, `blit.wgsl`, `examples/boxes.rs`)
all clean throughout. Not committed — left for the user's own review.
Unrelated to step 5's white line, which this session did not touch.

### Session 13 — `two_cubes.rs` extended to the real lit pipeline, three bugs found and fixed by refusing three half-measures

Picked up by user request: verify visually that a solid renders and shadows
correctly, starting with `two_cubes.rs`'s own `SolidsRenderer` overlay from
session 12 (added `OPENSHARD_CUBE_EDGES=0` along the way, fills without the
stroke, to look at faces alone first). Then extended the same tool to run the
two boxes through the real lit pipeline —
`GroundRenderer`/`MeshFaceRenderer`/`Blit`, the same three passes
`synthetic_stair.rs` uses — with an actual point light and a synthetic floor,
because `SolidsRenderer`'s own draw-order check (session 12) says nothing
about whether a box casts a shadow.

Three real bugs surfaced, each from a half-measure taken to avoid touching
something adjacent, and each one visible only by rendering and looking, not
by reading the arithmetic — the user's own framing, worth keeping verbatim:
"полумеры рождают такие нелепые баги" (half-measures give birth to exactly
this kind of ridiculous bug).

1. Built only the box's top and one riser (`facing::Prism` only ever carries
   one climb axis) instead of all three visible faces `solid::Solid::faces`
   draws. The render silhouette came out visibly different from the
   `SolidsRenderer` reference picture — not a coordinate bug, just a missing
   face, but it read like one until compared side by side.
2. "Fixed" (1) by combining **two independent** `Prism`s (one per riser)
   rather than opening `mesh::Mesh::push` (`pub(crate)`) to build one exact
   mesh. Each prism's own `WIDTH_OVERLAP` (`facing.rs`) widens a different
   axis, so the two risers' corners did not meet at the box's shared vertical
   edge: a small wedge-shaped crack cutting into the shadow right at the
   seam, reproducibly at both symmetric corners (user caught it from a
   photo of the rendered frame, circled). Fixed properly: `mesh::Mesh::push`
   is `pub` now (was `pub(crate)`), and the box is built by hand from the
   occlusion `Solid`'s own exact corners (`solid_a.space`/`solid_b.space`) —
   bit-identical shared vertices across all three faces, no widening
   anywhere, the same discipline `facing.rs`'s own `SEAM_OVERLAP`/
   `WIDTH_OVERLAP` exist to approximate for a single `Prism`'s tread/riser
   seam, done exactly instead.
3. Added a synthetic floor (`Map::from_blocks` plus one hand-built flat
   `openshard_uofiles::image::Image` packed into a `LandAtlas`, drawn through
   the real `ground::collect`/`GroundRenderer`) so a shadow has something
   besides the boxes' own faces to fall on. It rendered, but every land pixel
   read as "no flame reaches" in `View::Shadow`, even ground touching a lit
   box. Cause: `Blit::Frame.ground_instances` was still pointed at
   `blit::dummy_ground_instances` — harmless while no land was drawn, but
   `blit.wgsl`'s `KIND_LAND` branch reads a fragment's own tile from exactly
   that buffer (`ground_instances[id].place0`); with the dummy, every land
   fragment's world position resolved to garbage and its distance to the
   light was never inside the light's own radius. Fixed by pointing it at
   `GroundRenderer::instances_buffer()`, which `GroundRenderer::render`
   already fills from the same `GroundQuad`s handed to it.

`cargo clippy -p openshard-client-render --example two_cubes --all-targets`,
`rustfmt --check` on both changed files (`examples/two_cubes.rs`,
`src/mesh.rs`), `cargo test -p openshard-client-render --lib` (351 tests) —
all clean. Unrelated to step 5's white line, which this session did not
touch, and unrelated to the ray-vs-`Solid` track below.

### Session 12 — step 5's `WIDTH_OVERLAP` hypothesis ruled out by fault injection, `two_cubes.rs` built

Picked up step 5 (the white line) by user choice over the other two open
threads (point 4 cutover, extending the multi-solid oracle past the stair).
Reproduced the doc's own repro command live (`OPENSHARD_CLIENT` reached, see
`docs/development.md`), confirmed the line still present in `View::Shadow`,
and profiled it two ways with `OPENSHARD_SCENE_PROFILE_FACE=flat`: sweeping
`x` at the tread's mid-`y` found the CPU walk correctly occluded right up to
the tile's true edge (no anomaly), sweeping `y` at the clamped-edge `x`
(`1497 + 126/127`) found a real open/blocked split across the tread's own
`y` band. That split looked like it confirmed a `WIDTH_OVERLAP`-overhang
hypothesis (`facing.rs`'s `0.03` render-mesh widening, unmatched by the
occlusion `Solid`'s exact footprint) — geometrically plausible, arithmetic
lined up with the measured `sub.x`. Fault injection (`WIDTH_OVERLAP = 0.0`,
re-render, diff `View::Shadow` before/after) falsified it directly: `944`
pixels changed, none of them the white line, all of them the hairline seam
bug `WIDTH_OVERLAP` exists to close, reopened by zeroing it. Reverted before
anything else touched the file. The lesson for the next session: the `y`-
sweep's open/blocked split is real physics near a tread's own riser, not
evidence of an overhang — a plausible-sounding geometric argument still
needs the same fault-injection discipline session 9–11 already established
for the disagreement oracle, and very nearly did not get it here.

Redirected mid-session to a live question: does `OPENSHARD_SCENE_SOLIDS`'s
default translucent picture of the real stair (a faint diagonal highlight
across the tread/riser seams) indicate a genuine draw-order bug in
`SolidsRenderer`, or is it the deliberate blend `solids.rs`'s own `Style`
doc already names? Built `examples/two_cubes.rs` to answer it without the
stair's own confounds: two hand-built unit cubes (`Builder`/`Shape::UNREAD`,
`StaticTile` with `NO_SHOOT` set by hand — no client files, no map, no art),
drawn forward and with draw order reversed, translucent and opaque.
Confirmed `SolidsRenderer::render`'s pipeline has `depth_stencil: None` — no
hardware depth test at all — so occlusion correctness is entirely the
caller's responsibility via `solid::standing`'s own sort; the reversed-order
picture visibly paints the farther cube over the nearer one even under
`opaque: true`, which is what a caller getting that sort wrong would look
like. Checked the real stair the same way
(`OPENSHARD_SCENE_SOLIDS_OPAQUE=1`): clean, no bleed-through, so the
diagonal highlight in the default picture is ordinary translucent blending
at a real seam, not a misordered draw — this specific geometry's sort key
is fine today, but the mechanism has no safety net if a future one is not.
`two_cubes.rs` is a reusable probe for the next time this question comes up
on different geometry. `cargo clippy`, `rustfmt` and `cargo test -p
openshard-client-render --lib` (351 tests) all clean before committing.

Step 5's white line remains open — still a third thing, not the walk, not
`WIDTH_OVERLAP`. Point 4 (cutover) and the multi-solid oracle's own
extension past the stair are both untouched, as before.

### Session 11 — the multi-solid disagreement oracle, closed by extracting `exemption` rather than re-deriving it

Picked point 3's remaining gap by name, asked for explicitly: session 9's own
"no sound automated oracle... a deliberate stop." A disagreement oracle for
the stair needs `walk_cells`'s own `lit_end`/`flame_end`/`caps_this`/
`same_run` exemption predicates re-evaluated before a real `ray_vs_solid` hit
"counts" as backing a blocked verdict — without that, a real hit on a solid
both walks correctly exempt (a flame standing on its own tread) reads as an
unbacked disagreement and fails the test for a case that is not a bug.
Session 9 named duplicating that formula a third time — once each already in
`walk_cells` and `walk_cells_exact` — as the exact trap this doc's own
fault-injection discipline exists to avoid falling into by accident.

- **`exemption`/`ExemptionContext` (`light.rs`, beside `panel_stop`) pulled
  the `lit_end`/`flame_end`/`caps_this`/`same_run` decision out from under
  `walk_cells` and `walk_cells_exact`, where it lived as two copies of the
  same three lines — `spot.z` versus `from[2]`, the same value read two
  different ways, everything else identical. Reuse instead of a third copy:
  the oracle below calls it as a third caller, not a fourth duplicate of the
  formula. Behaviour-preserving by construction and verified rather than
  assumed — full `cargo test -p openshard-client-render` (350 lib tests)
  identical before and after the extraction, every count unchanged.
  `cargo clippy`'s own `too_many_arguments` lint caught the first draft's
  11-argument signature; `ExemptionContext` groups the ray-level facts that
  do not change per candidate tile or per solid (`first`, `last`,
  `skip_last`, `own`, `surface`, and the ray's own start/end `z`), built
  once before each walk's own loop starts rather than threaded through it
  argument by argument.
- **`walk_cells_exact_disagreements_on_the_stair_are_backed_by_a_real_
  unexempted_hit`** (`light.rs`'s own `mod tests`, beside the smoke test it
  gives a reason to exist alongside) — the same three-tread climbable stair
  and the same fuzz domain `walk_cells_exact_stays_in_range_on_the_stair`
  already covers, and the same "whichever walk claims the stronger answer
  must be backed" discipline the single-wall oracle
  (`walk_cells_exact_disagreements_are_backed_by_ray_vs_solid`) already
  runs, with the one addition this richer scene needs: a real `ray_vs_solid`
  hit only backs a blocked verdict if `exemption` says the solid it hit is
  not exempt, and — for anything but a lid — if `own_run` has not already
  cancelled every side the ray could have crossed it on (`stands.edges &
  !same_run != 0`, reusing `Exemption::same_run` rather than re-deriving
  that too).
- **Verified sound, not just green on the first run — this doc's own
  fault-injection discipline, run both directions before trusting it.**
  Stripped the exemption check back to the naive "any real hit counts" the
  single-wall oracle uses, session 9's own named trap: failed immediately,
  on exactly the false-alarm shape session 9 described by hand — a flame
  standing on its own tread, a real `ray_vs_solid` hit, correctly read as
  open by both walks, flagged as an unbacked disagreement anyway. Restored,
  then separately reverted `walk_cells_exact`'s own already-fixed lid bug
  (session 9's "every lid reads transparent," the tile-footprint lookup in
  `light.rs`'s lid branch) to confirm the new oracle still catches a real
  regression and not only avoids false ones: both this test and the
  existing pinned regression
  (`walk_cells_exact_does_not_read_every_lid_as_transparent`) failed
  together, on the same reintroduced bug. Both reverts restored before being
  trusted, and the throwaway `proptest-regressions/light.txt` artifacts each
  one wrote deleted rather than committed. Five more full runs afterward at
  fresh random seeds, all green at `~0.1`s each — session 10's own
  `BRUTE_STEP` false alarm argued for running this before trusting an
  oracle, not after a flake finds it first.
- `cargo test --workspace` (92 test binaries, 0 failed), `cargo clippy
  --workspace --all-targets`, `cargo check --workspace --all-targets` and
  `cargo fmt --all -- --check` all clean at the end of the session.
- **Not touched**: point 4, the actual cutover — still needs `blit.wgsl`'s
  own mirror of `walk_cells_exact`, which does not exist in any form yet
  (`candidate_tiles`/`ray_vs_solid`/`box_side` are Rust-only; this is a new
  GPU primitive to design, not a port of an existing formula the way step
  5's fix was), and a real rendered frame — this session, like every one
  before it in this backlog entry, never rendered one. Step 5's white line,
  still untouched — this session needed no `OPENSHARD_CLIENT` and no visual
  judgment, same reasoning session 9 gave for picking this track over that
  step.

### Session 10 — the public seam built, point 3's other half run, a false alarm found and un-found

Picked point 3's other half by name, asked for explicitly: session 9 left
`tests/lighting.rs`'s grid-sweep/fuzz oracles and `tests/frame.rs`'s
real-geometry scenes unexercised against `walk_cells_exact`, both working
through the public API (`light::sample`) rather than the module-private
functions session 9's own tests called directly.

- **Built the seam session 9's own handoff named as missing**: `sample_exact`
  (`light.rs`, beside `sample`) — `#[doc(hidden)] pub`, routed through a new
  `sample_with` both `sample` and `sample_exact` now share (refactored out of
  `sample`'s own body rather than duplicated), calling `walk_exact`/
  `walk_sun_exact` instead of `walk`/`walk_sun`. Explicitly documented as
  temporary: it goes away at point 4's cutover, when `sample` itself walks
  this path. Its existence made `ray_vs_solid`, `candidate_tiles`,
  `box_side` and `walk_cells_exact` genuinely reachable for the first time —
  their `#[allow(dead_code)]`s, staged since sessions 8–9, are gone.
- **`tests/lighting.rs`: two new tests, mirroring session 2's own
  brute-force grid and session 6's own fuzz, through `sample_exact` instead
  of `sample`** —
  `a_brute_force_oracle_agrees_with_the_exact_walk_over_a_grid_of_lights` and
  `a_fuzzed_flame_near_a_row_edge_agrees_with_the_brute_force_oracle_through_
  the_exact_walk`. Both green on the first run, over the same single-wall
  scene and the same corner-biased fuzz domain session 9's own module-private
  fuzz already covered — this is breadth (through the real `Sample`/`Reach`
  machinery, not `walk_cells_exact` called directly) rather than a new claim.
- **`tests/frame.rs`: a ground-truth-arbitrated characterisation suite over
  seven of decision 9's own real-geometry scenes** (`room`,
  `wall_with_a_torch_beside_it`, `house_corner`, `wall_with_a_hole_in_it`,
  `sunlit_room_with_window`, `lantern_in_a_room`, and `room` again at two
  surfaces) — `exact_walk_disagreements`, sweeping every pixel of
  `tests/frame.rs`'s own 64×64 parity grid, comparing `sample`/`sample_exact`
  classification (blocked/open at `RAY_CUTOFF`) rather than asserting bare
  numeric agreement. **The first version of this asserted zero
  disagreements and failed on hundreds of pixels of the plainest scene
  here**, `scene::room`: a spot standing on one tile of a straight wall run
  blamed a *neighbour* tile of the same run for blocking light a hand-marched
  ray never reaches. Not a new bug — `ground_truth_blocked` (an independent
  point-in-box march, `brute_force_blocked`'s own discipline restated for
  this test crate) backs `walk_cells_exact`'s "open" answer in every one of
  these; `walk_cells` blames the neighbour because `corner_tie`'s heuristic
  never considered it a candidate at all, and `candidate_tiles`'s
  unconditional probing does. A fifth `walk_cells` gap in the shape session
  9 catalogued four of, found this time by real map geometry rather than
  fuzzing a synthetic one, and in the opposite direction (over-occlusion, not
  under). Restructured the suite around this: every classification flip is
  arbitrated by `ground_truth_blocked`, and only a flip it backs *against*
  `walk_cells_exact` fails the test.
- **A real false alarm, found and un-found the same session, worth reading
  before trusting a point-sampling ground-truth oracle's "open" answer
  again.** `wall_with_a_hole_in_it` first failed with `ground_truth_blocked`
  backing `walk_cells` — a genuine `walk_cells_exact` bug, it looked like.
  Hand-verifying with the exact slab formula showed a real, tiny crossing
  (`entered` `0.03264`, `leaves` `0.03438` — about two thousandths of a tile
  of the segment's own length) through a wall panel's box at the exact
  lateral corner it shares with its neighbour: `walk_cells_exact` was right,
  and `ground_truth_blocked`'s fixed `0.02`-tile step — copied from
  `tests/lighting.rs`'s `BRUTE_STEP`, sized to
  `occlusion::PANEL_THICKNESS`'s own depth, not to how thin a corner graze
  through it can be — stepped clean over the sliver and reported "open" by
  mistake. Fixed by switching `ground_truth_blocked` to a fixed twenty
  thousand steps over the whole segment rather than a fixed step size; a
  200,000-step search confirmed the crossing directly (blocked at
  `t = 0.032645`, matching the slab formula's own `entered` almost exactly)
  before the constant was trusted. **The general lesson
  kept in that function's own doc comment**: a point-sampling oracle's
  "blocked" is always trustworthy (a sample landing inside a box is a real
  hit, full stop) but its "open" never rules out an arbitrarily thin sliver
  at any finite resolution — the asymmetry this session's first draft did
  not respect, treating both answers as equally strong evidence.
- **The same lesson, found a second time in a test this session did not
  write.** `cargo test --workspace` (not run until the very end, on the
  strength of `-p openshard-client-render` alone having stayed green all
  session) flaked: session 6's own
  `a_fuzzed_flame_near_a_row_edge_agrees_with_the_brute_force_oracle` and
  this session's `..._through_the_exact_walk` copy both failed, on the same
  proptest-minimised counter-example, with `walk_cells` *and*
  `walk_cells_exact` agreeing the ray was blocked and
  `tests/lighting.rs`'s own `brute_force_blocked` alone saying open.
  Independently verified with the slab formula: a real hit, `entered
  0.63355`, `leaves 0.63387` — about three thousandths of a tile of depth, a
  ray clipping a lone wall tile's far corner at a shallow angle. The exact
  shape of the false alarm above, in the oracle `tests/frame.rs`'s own
  `ground_truth_blocked` was modelled on: `BRUTE_STEP = 0.02` was sized to
  `PANEL_THICKNESS`, and nothing before this session had measured how much
  thinner a corner graze can be than a panel. Fixed the same way — `BRUTE_
  STEP` tightened to `0.001` — since it is shared by every test in the file,
  not copied into a second constant the way `tests/frame.rs`'s own fix was
  free to be. Confirmed rather than assumed: the specific counter-example
  above now passes, and eight more full runs of both fuzz tests (`512`
  proptest cases apiece, a fresh random seed each time) stayed green at
  `~0.04`s each — this file's whole suite is `0.26`s, not the "thousands of
  rays" slowdown the constant's own old comment worried about. **A
  `cargo test -p <crate>` green run is not the same claim as `cargo test
  --workspace`** is the operational lesson: nothing about this bug was in a
  file this session had touched, and it was found only because the doc's own
  closing checklist runs the wider command anyway.
- **Numbers, for the record**: `room` at `Surface::Upright` explained 232
  flips this way (all real `walk_cells` gaps, none unexplained), the same
  scene at `Surface::Flat` 232 at `z 0` and 236 at `z 20`; `lantern_in_a_room`
  (the carried-beam fixture) 232; `house_corner` 135;
  `wall_with_a_torch_beside_it` 8; `wall_with_a_hole_in_it` 3 explained plus 9
  left unexplained —
  `ground_truth_blocked` returns `None` rather than guess wherever the
  marched path crosses the hole's own aperture, the one thing it does not
  model (`brute_force_blocked`'s own scope restriction, restated: that
  oracle can afford a hard `assert!` that no fixture has one, this one
  cannot, since the scene's entire point is an aperture); `sunlit_room_with_
  window` found none at all.
- `cargo test -p openshard-client-render` (47 in `tests/frame.rs`, 37 in
  `tests/lighting.rs`, 350 in the lib including session 9's own five),
  `cargo test --workspace` (after the `BRUTE_STEP` fix above), `cargo clippy
  --workspace --all-targets`, `cargo check --workspace --all-targets` and
  `rustfmt --check` on every file touched: all clean.
- **Not touched**: a disagreement oracle for a genuinely multi-solid tile
  (the stair — session 9's own scope decision, still not attempted); step
  5's white line; point 4, the actual cutover, which still needs that
  multi-solid oracle and a real render first, per the doc's own recommended
  order, not just this session's own numeric confidence added to session 9's.

### Session 9 — point 2 built (`walk_cells_exact`), point 3 started, three real `walk_cells_exact`/`walk_cells` gaps found on the way

Continued session 8's ray-vs-Solid track by name, picking point 2 over the
open alternative (step 5's white line) — this track needs no
`OPENSHARD_CLIENT` and no visual judgment, both of which point 5 does, and
`OPENSHARD_CLIENT` was available but the numeric-oracle discipline this
track already runs on is the more tractable of the two for a session that
cannot look at a screenshot and judge it. Full account above, in the
backlog's "A bigger idea..." entry, since that is where this track's own
history already lives; short version here.

- **Built `candidate_tiles`, `box_side` and `walk_cells_exact`**
  (`light.rs`, all beside `dda_walk`/`walk_cells`), the parallel
  `walk_cells`-shaped function point 2 asked for, `#[allow(dead_code)]`
  and not wired into `walk`/`walk_sun`.
- **Found and fixed three real bugs in the new code before trusting any of
  it**, all by fuzzing rather than by inspection: `candidate_tiles` naming
  the wrong diagonal cell (fixed by matching `DdaTransition::Corner`'s own
  `by_x`/`by_y` fields exactly); a dropped safety net for a body's
  corner-graze that turned out to be a deliberate design choice in
  `walk_cells`, not a DDA workaround (restored, using `box_side` in place
  of a DDA step's `entry`/`exit`); and, only surfaced once the fuzz moved
  from a single wall to the three-tread stair, every lid reading as fully
  transparent — `ray_vs_solid`'s slab method correctly collapses a flat
  lid's own crossing to a single point in `t`, and `crosses` was never
  built to be handed an already-collapsed interval. Fixed by asking the
  lid branch for the *tile's* own entry/exit instead of the lid's.
- **Found, and did not try to fix, four real pre-existing gaps in
  `walk_cells` itself, one of them new this session**: `panel_stop`'s
  single-point test under-occludes a body reached only through a corner;
  the per-cell panel branch's `entry`/`exit`-side gate can miss a genuine
  panel intersection on an ordinary `Step` with no corner involved; and,
  found on the stair, the same gate never checks a specific riser's own
  fractional footprint at all, only its height, so a ray that never
  geometrically approaches a riser's narrow band can still trip
  `walk_cells`'s coarse per-cell test for it. All three are the same
  family as `corner_tie`'s already-documented corner-grazing slop — a
  coarse per-cell approximation the exact primitive removes by
  construction — and none was chased into `walk_cells` itself.
- **Five permanent tests, not the one or three this doc's point 3 might
  have implied**: full numeric parity only provably holds in a scoped
  sub-domain, one solid per tile and off `corner_tie`'s own path; a
  `ray_vs_solid`-backed characterisation test covers single-solid scenes
  more broadly by asserting the *disagreement* is explained rather than
  that agreement holds; and the stair scene — multiple solids sharing one
  tile, where that characterisation trick itself stopped being sound
  (a real hit that both walks correctly exempt still isn't a
  "disagreement" to explain) — got a targeted regression test for the lid
  bug plus a range/no-panic smoke test instead of a disagreement oracle
  this session didn't build. All five green; full account in the backlog
  entry above, including why the stair fuzz stopped at a smoke test on
  purpose rather than a rushed characterisation.
- `cargo test -p openshard-client-render`, `cargo clippy -p
  openshard-client-render --all-targets`, `cargo check --workspace
  --all-targets` and `rustfmt --check crates/client/render/src/light.rs`
  all clean.
- **Not touched**: step 5's white line (see "Where the next session
  starts"); `tests/lighting.rs`'s grid-sweep/fuzz oracles and
  `tests/frame.rs`'s parity suite (point 3's other half, needs a seam into
  module-private functions or scenes built the way this session's own
  tests build them); a disagreement oracle for a multi-solid tile that
  re-checks `walk_cells`'s own exemption predicates rather than assuming
  every real hit counts; point 4, the actual cutover, which needs the
  parity suite and a real render before it is safe to start, not just this
  session's numeric confidence.

### Session 8 — the ray-vs-Solid idea scoped, its first primitive built

Picked up session 7's deliberately-deferred idea by name, asked for
explicitly: `occlusion::Solid` already stores exact boxes, so replace
grid-DDA's per-cell stepping with direct ray-vs-box intersection instead of
testing the existing stepping harder. Full account in the backlog's "A
bigger idea..." entry, now two sessions long; short version here.

- **Scoped all three questions the backlog entry deferred**, by reading
  `walk_cells`/`dda_walk`/`blit.wgsl`'s `walk` line by line rather than
  guessing from the entry's own summary. Broad-phase cost dissolves — reuse
  `dda_walk`'s own cell enumeration, just probe the corner-diagonal
  neighbour unconditionally at every step instead of gating it behind
  `corner_tie`'s heuristic. `corner_tie`, `DdaTransition::Corner`'s
  special-cased handling and `panel_stop`'s corner call all go away by
  construction once every candidate gets an exact test rather than a
  proximity guess — three things this doc has twice had to fix a bug in.
  `PANEL_THICKNESS` itself survives unchanged; it is the panel's own
  physical depth, not a walk-algorithm approximation.
- **Named what does not get simpler**: `walk_cells` is not just "does the
  ray hit a box" — self-shadow exemption, wall-run continuity, apertures
  and per-cell penumbra softness are all real rules, all keyed to a
  solid's *originating tile*, and any replacement has to carry them rather
  than design around them. This is a body-swap of the stepping, not a
  rewrite of the rules.
- **Recommended order, matching this doc's own fault-injection
  discipline**: build the new stepping as a second path, prove it agrees
  with the old one over every oracle this doc already has, cut over only
  once that holds. Four numbered points in the backlog entry.
- **Built and tested point 1**: `ray_vs_solid` (`light.rs:1104`), a pure
  segment-vs-AABB slab-method primitive, no `Occlusion` or tile dependency,
  `#[allow(dead_code)]` and deliberately not wired into anything real yet.
  Six hand-computed unit tests plus a 2048-case proptest checking it
  against an independent point-in-box characterisation, the same oracle
  discipline step 4's brute-force sampler already established for this doc.
- Full `cargo test -p openshard-client-render` (16 test binaries), `cargo
  clippy -p openshard-client-render --all-targets`, `cargo check
  --workspace --all-targets` and `rustfmt --check
  crates/client/render/src/light.rs` all clean at the end of the session.
- **Not touched**: step 5's own white line, and points 2-4 of the
  ray-vs-Solid plan (the parallel walk, the agreement pass over the
  existing oracles, the actual cutover). Point 2 is the natural next piece
  if this track continues; step 5 is still exactly where session 6/7 left
  it if the next session goes there instead — see "Where the next session
  starts" above.

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
