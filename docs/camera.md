# The camera, and the bench it is chosen on

The client has a camera and it is the reference one: the eye is the body's, to
the pixel, every frame. That is what ClassicUO does and it is not what this
client will ship, because it inherits the walk's discontinuities whole — a step
starts and the world starts with it, a rollback puts the body back a tile and
the world jumps a tile, a kiting reversal is a hard stop and a hard start
120ms apart. None of that is a bug in the follow; it is the follow having no
opinion.

Several cameras are wanted, and which one is right is not knowable from a
document. So this plan is mostly not about a camera. It is about the two things
that make choosing one cheap: **one pipeline every camera is a parameter set
of**, and **a bench that scores a parameter set against a scripted walk**. The
cameras themselves are then a short list at the end, each of them a struct
literal.

Written against `crates/client/render/src/control.rs`, `camera.rs` and
`mobiles.rs`, and against the walk harness in `crates/client/app/src/dst.rs`,
which already produces the thing this needs most: a per-frame trace of where a
body was drawn, on a virtual clock, with a wire and a shard behind it.

## The decisions

Numbered so one can be argued with alone.

### D1 — one pipeline; a camera is data, not an implementation

There is no `trait Camera`. There is one ordered pipeline and a `Rig` — a plain
struct of numbers and two or three enums — and every camera named below is a
value of it.

The reason is the bench. Two cameras written as two implementations are two
bodies of code with two quantisers, two cut rules and two rounding habits, and a
bench comparing them compares those as much as the feel. As one pipeline they
differ in exactly the fields that differ, and a defect in the shared path shows
up in every row of the table at once rather than in whichever implementation
happened to get it wrong.

It also makes the reference camera honest. `Rig::HARD` is not a special case
written in a hurry — it is every filter's time constant at zero, every zone at
zero, the cut threshold at zero. If the pipeline cannot express "the eye is the
body" as a degenerate parameter set, the pipeline is wrong, and finding that out
on the first preset is the point of starting with it.

When something genuinely cannot be a parameter — the RTS anchor is the candidate
— the pipeline grows a **stage** that the other presets switch off. It never
grows a fork.

### D2 — the target is decomposed: the ground and the height are two signals

`project` folds `z` into the vertical axis: a tile one unit higher is four
pixels further up the same screen column. So a camera handed a projected pixel
cannot damp height at all — filter that value and the walk is filtered with it;
do not, and every stair is a four-pixel step in the world's position.

The rig is therefore told:

```rust
/// Where the eye is asked to look, before anything smooths it.
struct Gaze {
    /// The body's ground position in world pixels, read at `z = 0`. Sub-pixel:
    /// this is a filter's input, not a sprite's placement.
    plane: (f32, f32),
    /// What its height lifts it by, in pixels — `z * Z_STEP`, kept apart so it
    /// can have its own clock.
    lift: f32,
}
```

and the eye is `plane - (0, lift)` after each has been filtered on its own terms.

This is also why `mobiles::world_position` is not what feeds it. That function
rounds to whole pixels, because a sprite is drawn on a texel grid — and a filter
fed an already-quantised signal is a filter fed a staircase, which is the
classic way to build a smoother that smooths nothing at low speed. The camera
gets an unrounded, undecomposed-into-`z` sibling of it, and the rounding happens
once, at the very end of the pipeline, where D7 puts it.

**Built, and it went one step further than that.** `mobiles::gaze` is the
formula and `world_position` is now `gaze(m).eye()` — one arithmetic, not two.
Written as two they were a pixel apart on about one frame in five thousand,
wherever the exact answer landed near a rounding boundary, and a camera and a
body that round separately by a pixel is a shimmer nobody can name or reproduce.
The cost is that "the eye is on the body" can no longer be proved by comparing
two independent formulas; what pins the arithmetic instead is `project`, which
is older than both — see C0's gate.

### D3 — every distance is a fraction of the screen; only time is time

Dead zones, lead caps, lean caps and cut thresholds are stored as fractions of
the drawn image's half-extents and resolved to world pixels at the top of each
frame. A camera tuned at `1x` then feels the same at `1/2x` and at `4x`, which
it does not if the numbers are world pixels — at `4x` a 40-pixel dead zone is a
tenth of the screen and at `1/2x` it is a fortieth.

Time constants stay in seconds. They are the one quantity a zoom does not
change.

### D4 — the order of the pipeline, including the stages that are empty

Order is where camera bugs live: a clamp before the filter springs off the
boundary, a shake mixed into the follow state drifts and never comes back, a
quantiser in the middle turns a filter into a ratchet. So the whole order is
fixed now, empty stages included, and a stage is filled in later without
anything moving.

1. **Anchor** — what the rig is looking at: the body's `Gaze`, a pinned point, or
   later a centroid of several. This is the only stage that knows what a player
   is.
2. **Intent** — additive offsets that say where the player wants to look rather
   than where they are: velocity look-ahead, cursor lean, an RTS offset. Each
   is smoothed on its own, then summed, then capped as one.
3. **Zone** — the dead zone, plus the idle timer that recentres out of it.
4. **Filter** — per-channel damping: `plane.x`, `plane.y`, `lift`. Frame-rate
   independent by construction (D5).
5. **Cut** — the discontinuities the filter must not be dragged across. Decided
   *before* the filter runs and it resets the filter's state, so a cut leaves no
   tail.
6. **Clamp** — empty. Nothing needs it while the anchor is a body, which is on
   the map by construction. It is in the order so that the day a free camera
   wants a map edge, the clamp does not get written above the filter.
7. **Impulse** — empty. Shake, recoil, a hit's kick: additive, on the pose,
   never on the filter's state. Reserved here for that reason and no other.
8. **Quantise** — round the pose to whole world pixels and keep the remainder in
   the state (D7).

### D5 — frame-rate independence is a property, and it is testable

The filter is `alpha = 1 - exp(-dt / tau)` per channel, or the critically-damped
spring of the same time constant where a spring's overshoot is wanted. Never
`lerp(x, target, 0.1)` per frame: that ties the feel to the frame rate, and this
client already has two frame rates on purpose — `FRAME_DELAY` when nothing
glides and `GLIDE_INTERVAL` when something does — so the naive form would
change the camera's character at the exact moment somebody starts walking.

This is not a matter of taste that has to be remembered. The bench runs each
script at 8ms, 16ms, 33ms and a jittered dt and compares the eye at matching
timestamps; the naive form fails it by a wide margin and nothing else does.

### D6 — a cut is an event, not a distance, and the distance is a backstop

A teleport, a facet change, a resurrection and a relock from across the map are
all *cuts*: there is no path between the two poses that anyone wants to watch.
They arrive as events, from the code that knows what happened.

The distance backstop — a gap wider than the cut fraction of the screen — exists
for the ones nobody remembered to raise, and it earns its place with an
argument, not with caution: if the body is off screen, easing to it draws a
smear of world nobody is looking at, over a distance nothing bounds.

The mirror of this is the one the reference camera gets wrong and which is half
of what this whole plan is for: a **server correction is not a cut**. The
rollback puts the body back onto a tile it did cross; a filter should absorb it
over a hundred milliseconds and the current camera relays it whole. So the cut
threshold has a floor to stay above: several tiles, not one.

### D7 — the fraction lives in the state; the pose is whole pixels

The pose the camera is given is whole world pixels, and the remainder stays in
the filter's state. An eye carrying a fraction puts every sprite on a half-texel
boundary for half of all camera positions, which does not show on a screenshot
and boils the whole frame in motion. This is the same rule the drag remainder in
`control.rs` already follows, applied one stage later, and it is why the
quantiser is last: quantise before the filter and a slow camera ratchets — it
sits still until the accumulated error crosses a pixel and then jumps one.

**The state is `f64`, and the filter is not what decides that.** `f32` is
plenty for a smoother: the far corner of a 7,168-tile facet is about 157,000
world pixels out, where an `f32` still resolves to a hundredth of a pixel. It is
the *rounding* that wants more. The eye has to land on the pixel the sprite
landed on, and a hundredth of a pixel of slack is a hundred times the margin at
which two roundings of the same position disagree — which is a shimmer that
appears only far from the origin, on some frames, and never in a test written
near tile zero.

### D8 — the rig is a pure function of one frame's input

```rust
fn advance(&mut self, gaze: Gaze, dt: Duration) -> WorldPixel
```

What the pipeline is told is the gaze, the cursor's offset from the viewport
centre, the image's half-extents, the zoom, `dt`, and any cut raised this frame.
No `Instant`, no `Camera`, no window, no `Map`.

**Two arguments and not a `Frame` struct, until there are more of them.** The
plan said `advance(&Frame) -> Pose` and the code that came out of C0 says the
line above: stages 2, 3 and 5 are empty, so the input really is a gaze and an
elapsed time, and a wrapper around two values and a pose of one field is
structure that carries nothing. The list above is what those arguments *become*
— it is the shape of C5, not of today.

That signature is the whole bench. It runs ten thousand frames in under a
millisecond, it is what DST drives, it is what the app calls once per frame, and
there is no second copy of the arithmetic anywhere for the three of them to
disagree over. `Control` keeps its present job — it arbitrates who may move the
eye — and delegates the following to a `Follower` that owns a `Rig` and the
state.

`Rig` is `Copy + PartialEq` and the state is separate from it, for two reasons
that are both about the bench: a preset can be swapped while the client is
running without the world jumping, and a slider edit is a value that can be
printed, pasted into the source and committed as the preset it turned out to be.

### D9 — nothing here decides the default camera

The presets are named after their mechanism — `HARD`, `LIFT`, `SPRING`, `LEAD` —
and none of them is called `DEFAULT` until one has won on the bench and in the
window. A plan that names a winner in advance builds the bench to confirm it.

## The bench

The camera's failures have names, and each name is a script.

### Scripts

**Built, and it is a body's path rather than a player's inputs.** The plan said
one `Script` type for all three consumers, generalising `dst.rs`'s `Act`. That
turned out to be two different things wearing one name: the bench needs a *body*
(a gaze as a function of time, with no steer, no wire and no shard, which is what
makes it fast enough to sweep), and the DST harness needs *inputs* (arrows, at
instants, driving the real four units). Forcing one type would have meant either
dragging a walk pipeline into `client/render` or measuring a camera against a
body that arrived by magic.

What is shared instead is the thing that has to be: `Sample` and `Metrics`. The
DST harness records the same samples from the real pipeline and runs the same
metrics over them, and a test holds the two walks against each other — the
scripted body and the real one peak within five per cent. That is the claim
"one type, three consumers" was really after, and it is the one that can be
checked.

- the **pure bench** drives `Follower::advance` from a scripted gaze;
- the **DST sim** drives the real `steer.rs`, `Walk`, `Crowd` and a shard over a
  wire with latency and jitter on it, and feeds the camera what actually came
  out;
- the **window** replays a script's *knots* as events into the real `Crowd`
  (C4), so what the eye follows is the client's own glide rather than a formula.

The scenarios, and what each is for. `mash` and `mouse_swirl` are not built:
the first needs the input-level script the DST harness has and the bench does
not, and the second needs the cursor, which is C5. `frame_jitter` turned out to
be an axis rather than a scenario — `Cadence` — so every script can be run
jittered.

| Script | What it is for |
|---|---|
| `stand_still` | A flat line. Any motion at all is shimmer, and it is the cheapest possible test of D7. |
| `ten_east` | The baseline walk: lag, and whether the eye's speed is constant. |
| `back_and_forth` | The kite. A reversal every few steps — overshoot, settle time, and the reason the spring exists. |
| `mash` | Direction changes faster than the walk can answer, which is where the queue rule shows through into the camera. |
| `rollback` | A `0x21` mid-step. The correction must be absorbed, and *not* cut across. |
| `teleport` | A recall. Must be cut, and must leave no tail. |
| `stairs` | `z` up a few units at a time, walked — which the glide already smooths, as C2 found out. |
| `kerb` | Two units arriving *whole*, as a correction does. What the lift filter is really for, and what keeps the cut from firing on everything sudden. |
| `ledge` | Fifteen units arriving whole — one short of the cut, so the worst case a filter is ever asked to absorb. What picked `lift_tau`. |
| `dungeon` | A large drop at once: past the cut, and the boundary the rule is drawn at. |
| `mouse_swirl` | The cursor circling while the body stands. Lean's own jitter, with nothing else moving. |
| `frame_jitter` | Any of the above, with `dt` drawn from a jittery distribution. D5's property. |

### What is measured

Everything in **world pixels**, which is screen pixels at zoom 1 and the bench
does not zoom, and everything over the *eye's* trace rather than the body's —
but over **which** eye's trace is not a detail, and getting it wrong makes half
of these measure the quantiser. See C1's first finding; the split is written
next to each:

- **lag** — max and RMS distance from the drawn eye to the body. How far the
  camera trails.
- **overshoot** (`ahead_max`) — the furthest the drawn eye got *past* the body
  along its direction of travel. Negative everywhere means it never overshot,
  and `NaN` means nothing walked, which is a different claim from zero.
- **speed, acceleration, jerk** — the first, second and third differences per
  unit time, off the **unrounded** trace. Jerk is the number that means
  "ragged": a camera that changes its acceleration abruptly is one the eye reads
  as stuttering even when its path is smooth.
- **step variance** — the unevenness of the *drawn* eye's per-frame movement.
  The one a continuous metric misses: at a constant body speed, an eye that
  moves `0,0,3,0,0,3` and one that moves `1,1,1,1,1,1` have the same mean
  velocity and only the first is a ratchet. Measured only over the frames where
  the body was moving, and paired with `still_frames` — how often the body moved
  and the eye did not.
- **travel** — how far the drawn eye went in total. Half a metric and half a
  companion: it is what says the run was a run.
- **cut count** — not built, because nothing cuts yet. C3's, and the number that
  catches a camera that smooths beautifully by cutting whenever it falls behind.

Every one of them is asserted with a companion that says the data is real: more
than *k* frames drawn, more than *n* pixels travelled, the rollback actually
delivered, the two rigs given the same number of frames. A metric over a scene
where nothing moved is green and means nothing, and this repository has produced
that result before.

### What comes out

Three outputs, and the third is the one that decides anything:

1. **A table** — presets down, scripts across, one metric per cell, printed by
   the runner. The comparison is the primitive: a single camera's jerk figure is
   uninterpretable, and the same figure next to `HARD`'s is not.
2. **CSV and SVG** per preset and script, under `target/camera/`. The SVG is a
   polyline this repository draws itself — no plotting dependency for six lines
   of `<path>` — and presets are **overlaid on one chart**, because two curves on
   one axis is how raggedness stops being a feeling. A number that disagrees with
   the picture means the metric is wrong, and that has to be visible. Two panels
   per script: the eye's own speed, where a reversal is a square corner or a
   rounded one, and how far behind the body it was, which is what that corner
   cost.
3. **A scope in the window** — a strip chart of the last few seconds of eye
   velocity and jerk, drawn with `egui::Painter` lines, beside a preset picker
   and a slider per `Rig` field. From the moment this exists, choosing a camera
   is looking rather than arguing. **Built** — C4.

The metric functions take a slice of samples and nothing else, so the offline
runner and the live scope compute the same numbers from the same code. C4 took
that one step further: `bench::readings` is the *only* place a difference is
taken, `Metrics` is its peaks and every curve — the SVG's, the window's — is its
values. A number that disagrees with the picture beside it now means the metric
is wrong, which was the claim; two differencing loops would have made it an
argument.

## The milestones

### C0 — the seam — **built**

`crates/client/render/src/follow.rs`: `Gaze`, `Rig`, `Follower::advance`, with
the order of D4 written out and stages 2, 3, 6 and 7 empty. `Control` keeps
arbitrating who may move the eye and delegates how. `mobiles::gaze` is the
decomposed target and `world_position` is it rounded. Nothing changed on screen,
which was the point, and the pixel-exact frame tests agreeing is most of the
evidence for that.

**The gate came out differently, and the reason is worth keeping.** The plan
said: run a DST script through the old path and the new one and assert the two
eye traces are equal. That is not available once `world_position` is derived from
`gaze` (D2) — the two paths are one formula, so the comparison is a tautology.
The gate is therefore two assertions that are not:

- `Gaze::on(p).eye() == project(p)` over the whole `z` range, and a step's ends
  landing exactly on the two tiles it is between. `project` is independently
  written and pinned by its own tests, so the fold and the decomposition are held
  against something older than either.
- `the_reference_rig_puts_the_eye_on_the_body_every_frame` in `dst.rs`: over a
  perfect wire, a jittery one, a rollback into a wall and a reversal every 270ms,
  the eye is exactly the drawn body on every frame. The arithmetic is shared, so
  what this pins is the *wiring* — that the camera is advanced on every frame the
  body is, from the same gaze the sprite is placed from, with nothing accumulated
  between frames and nothing a frame late. Every one of those is a way to break
  the transplant that no test inside `client/render` would notice.

Both carry the companion assertions the metrics will: the run drew more than a
hundred frames and the eye travelled more than four hundred pixels, because an
eye that never moved sits exactly on a body that never moved.

### C1 — the bench — **built**

`crates/client/render/src/bench.rs` is the arithmetic — `Script`, `Cadence`,
`Sample`, `Trace`, `Metrics` — and `tests/camera.rs` is the runner, because this
crate opens no files. `cargo test -p openshard-client-render --test camera --
--ignored --nocapture` prints the table and writes a CSV per run and a chart per
script under `target/camera`.

**The baseline, at 16ms a frame.** Every number is world pixels, and the two
`hard` rows that matter are the last three columns:

| script | rig | lag max | speed max | accel max | jerk rms | step σ² |
|---|---|---|---|---|---|---|
| `ten_east` | hard | 0.68 | 77.8 | 4,861 | 26,006 | 0.21 |
| `ten_east` | probe | 9.39 | 77.8 | 607 | 2,426 | 0.25 |
| `back_and_forth` | hard | 0.68 | 77.8 | 9,723 | 160,383 | 0.21 |
| `back_and_forth` | probe | 8.82 | 75.0 | 1,192 | 14,663 | 0.49 |
| `rollback` | hard | 0.68 | 1,867 | 121,534 | 1,563,869 | 8.17 |
| `rollback` | probe | 18.38 | 165 | 15,171 | 120,015 | 0.40 |
| `dungeon` | hard | 0.68 | 5,055 | 315,956 | 5,187,447 | 122.6 |
| `teleport` | hard | 0.00 | 140,223 | 8,763,940 | 144,679,101 | 0.00 |
| `teleport` | probe | 1,963 | 17,504 | 1,093,974 | 11,099,251 | 0.00 |

`probe` is not a preset and not a proposal — it is one filtered rig, there so
that a table with one row and a chart with one curve cannot pretend to show a
difference. What the baseline says, before anybody argues about feel:

- **The reference camera's raggedness is all in the discontinuities.** A held
  walk is 4,861 px/s² of acceleration and a reversal is 9,723 — exactly twice,
  which is what a velocity that flips rather than stopping means.
- **A filter of 0.12s buys an order of magnitude and costs 9.4 pixels** —
  `speed × tau`, to two decimal places, which is also the arithmetic checking
  out.
- **`rollback` and `dungeon` are where the reference camera is worst by two
  orders of magnitude**, and they are the two the player never asked for: a
  correction is a tile the body did cross, and a floor changing is not a walk.
- **`teleport` is why D6 exists, in numbers.** The filtered rig trails the body
  by 1,963 pixels — most of a screen — for a second, which is the smear a cut
  removes. Nobody has to be persuaded of the cut stage now; the row is there.

**Two findings that changed how it measures.**

The first: **derivatives cannot be taken on the drawn eye.** At one-pixel
quantisation and sixty frames a second, a body walking at 78 px/s moves the eye
1.2 pixels a frame, so the drawn eye moves `1, 1, 2, 1, 1, 2` — and the
acceleration of *that* is thousands of px/s² of pure rounding, the same order as
the reversal a camera exists to smooth. Differentiating it measures the
quantiser and calls it the rig. So `Sample` carries the eye twice: the whole
pixel the screen was given, and what the filter had before the quantiser. Speed,
acceleration and jerk come off the second; lag, overshoot and travel — which are
what the player sees — come off the first; and the quantiser gets its own metric,
`step_var`, where the unevenness *is* the quantity.

The second: **a bench that only measured smoothness would score a camera that
never keeps up as the best one there is.** So the test that proves the bench
discriminates asserts both directions on one run — the filtered rig's worst
acceleration is a third of the reference's *and* it trails by ten times as much.
Either one alone is passed by a rig nobody would ship.

D5's property is tested with a mirror: the same script at 4ms and at 32ms lands
within two pixels, and the banned form — `lerp` by a constant per frame, written
out in the test — lands **fourteen** pixels apart on the same comparison. A
tolerance nobody has shown to catch anything is not a tolerance.

**And the bench is held against the real walk.** `dst.rs` records the same
`Sample`s from the real `steer`/`Walk`/`Crowd`/shard pipeline and runs the same
`Metrics` over them: the scripted walk and the real one peak within five per cent
of each other. A rig fitted to a synthetic body with no wire behind it would be
fitted to nothing, and this is what says the body is the right one.

### C2 — the lift — **built**

`Rig::LIFT` — the reference plane, `lift_tau: 0.15`, `lift_cut: FLOOR` — and
stage 5 is live on the one channel that has a rule yet.

**The milestone's premise was wrong, and the bench is what said so.** The plan
was "a stair rises over its step instead of jerking the world four pixels at a
time". It already does: `Glide::from` carries the tile's height, so a climbed
step's rise is spread across the whole 400ms by the glide, and there is nothing
left in it for a filter to smooth. Worse, filtering it makes a climbed stair
*slightly worse* — 4,643 px/s² against the reference's 3,452 — because the rise
was **cancelling** part of the walk's own motion: walking east moves the eye down
the screen and climbing moves it up, and delaying one half of a cancellation is a
transient where there was none. That is pinned as an assertion rather than
written down, so it fails if the shape ever changes.

**What the lift filter is actually for is height that did not come from
walking.** A `0x22` revising the ground under a standing body, a surface a hair
different from the one predicted, a correction: those arrive *whole*, in one
frame, and the reference camera relays every pixel of them. The `kerb` script is
two units arriving that way, and it is 31,250 px/s² for `HARD` and 4,641 for
`LIFT` — where 4,641 is the transient the *walk* itself makes, which the plane
owns and C3 answers. The correction disappears under it.

**The cut is a body's height, in one frame, and the constant is Sphere's.** A
walked change of height comes through the glide a few pixels at a time, so
everything that arrives whole is a floor changing, a teleporter, or a correction
— and the ones worth easing are the small ones. `PLAYER_HEIGHT` is 16 units, so
`FLOOR` is 64 pixels: below it, absorb; above it, cut. This is the one place D3's
"every distance is a fraction of the screen" does not apply, and the reason is
worth stating — whether a change of height is a stair or a storey is a fact about
the world, and it does not become a different fact because somebody zoomed in.

The cut takes the height alone. A floor giving way under a body does not move it
sideways, and jerking the whole world across for a vertical event would be a
second defect answering the first.

**The time constant, from the sweep it was chosen on.** `stair lag` is how far
the eye trails while climbing; `ledge px/s` is how fast it slides absorbing the
largest jump it is allowed to see — fifteen units, one short of the cut:

| `lift_tau` | stair lag | `kerb` accel | `ledge` px/s |
|---|---|---|---|
| 0.05 | 2.1 | 8,559 | 1,083 |
| 0.10 | 4.6 | 4,635 | 612 |
| **0.15** | **7.1** | **4,641** | **438** |
| 0.20 | 9.6 | 4,733 | 348 |
| 0.30 | 14.6 | 4,802 | 256 |
| 0.50 | 24.6 | 4,839 | 182 |

A riser is 20 pixels, so 0.15 trails by a third of one; at 0.30 it is
three-quarters of a riser and that is the lift-shaft feel the milestone was
named after. Below 0.10 a correction stops being absorbed at all — the `kerb`
column climbs back above the walk's own transient. And the worst absorbable jump
slides at 438 px/s, five times a walk and over in under half a second, which is
fast enough not to float and slow enough not to be a jump. The sweep is printed
by the dump, so disagreeing with the choice costs one run.

**A third finding, about the metric rather than the camera.** `accel_max` is not
a physical quantity where the target *jumps*: a filter's answer to a step has a
velocity that changes instantly in continuous time, so what a frame-to-frame
difference samples is `size / (tau * dt)` — it ranks two time constants correctly
and it doubles if the frame rate doubles. Where the input is a jump, `speed_max`
is the number that means something, and that is what the sweep's last column is.

### C3 — the spring

Plane damping, the dead zone, and the idle recentre that stops the dead zone
stranding the body off centre. Scored on `back_and_forth` and `rollback`: the
claim is that overshoot stays under a bound while the rollback's jerk drops by
an order of magnitude against `HARD`, and both halves are asserted, because a
camera that absorbs a rollback by never keeping up is not the camera anybody
asked for.

### C4 — the scope — **built, pulled ahead of C3**

Sliders, presets and the strip chart in the shell, and a script runner that
walks a virtual player through the bench's scenarios in the window. Placed after
C3 in the numbering and pulled forward the moment C2 landed, for the reason it
was worth pulling forward: from here on every remaining decision is a matter of
looking, and a slider is faster than a rebuild.

`crates/client/app/src/shell.rs`'s **Rig** window is the panel — `HARD` and
`LIFT`, a slider per field, the live `Metrics`, two strip charts, and a button
per scenario. `crates/client/app/src/replay.rs` walks the scenarios.
`bench::Scope` is the ring the panel draws, fed one frame at a time from
`App::follow_player`, which is the single place the camera is advanced.

**The loop got the same treatment as the camera, for the same reason.** A frame
rate is two independent quantities — the interval between two drawn frames, and
what one cost to build — and a drop in the first with the second flat is a
*pacing* decision rather than a cost. `crates/client/app/src/frames.rs` is the
ring and the **Frames** window draws the curves plus what is currently asking for
frames, which is what turns "it stutters when I stop" from an argument into a
reading. It is not the scope: a frame drawn with the camera unlocked is still a
frame, so it is fed every frame and never cleared by a rig swap.

**And then the instrument answered its own question: the display is the pacer.**
The loop used to be a timer — 16ms while something glided, the animation clock's
80ms otherwise — and the panel is what made that decision look like what it was.
Every argument for it was correct and none of them survived being looked at: a
still screen at 12.5 frames a second reads as a stall, whatever is true about the
pixels not having changed. So the surface asks for `PresentMode::Fifo` by name
rather than taking whatever the adapter offered first, and every frame ends by
asking for the next one while the window is watched. What makes that a rate and
not a spin is `get_current_texture` blocking until the display has taken the last
frame, which is the loop every other real-time client runs.

*Watched* is focused and not occluded, and it is the whole of what this client
does about power: in the background the animation clock takes over, because a
window nobody can see still has to age its animations to be in the right state
when it comes back. The timer stays behind that as a safety net — `draw` returns
early with no window, with a swapchain it had to rebuild, and when the compositor
refuses a texture, and on each of those the frame that would have asked for the
next one is the frame that did not happen.

**The cost split in two when the pacing stopped being interesting.** Under vsync
a frame always takes a refresh interval, so "how long did the frame take" stopped
being a number about this client and the panel now charges `egui` and the world
separately, with the vsync sleep as a third figure that is neither. Counted as
build time that sleep would report an idle client at full load; separated, it is
the slack, and `ui` against `world` is the one reading that says which half to go
and look at. The rates are not split and are not meant to be: both halves go
through one encoder into one surface texture, so they are on screen the same
number of times a second by construction.

**A rig is copied out as a source line, and that is the output that lasts.** The
panel prints `Rig { plane_tau: 0.15, .. }` beside a copy button, so a setting
that felt right is pasted into `follow.rs` and committed as the preset it turned
out to be. It is a function with a test rather than a `format!` inside a widget,
because `f32::INFINITY` prints as `inf` and `inf` is not Rust — a failure that
would surface hours later, in another file, as a build error.

**The scope is fed only while the eye is the body's.** Unlocked, the camera is
wherever a hand left it, and a lag measured against a body it is not following
is not a number about the rig. The trace is also cleared when a preset is
swapped or a scenario started: the frames either side of either are two
different runs, and metrics over both are a number about nothing.

**The finding, and it was the harness rather than the camera.** The first
replay of `ten_east` peaked at exactly twice the bench's speed, which is the
shape of a body being yanked. `Replay::advance` was incrementing its clock
*before* reading it, so the knot at zero fired on the frame ending at 16ms and
the knot at 400 on the frame ending at 400 — the first gap a frame short. The
crowd then started a new step while the last one still had a frame to run, and
the eye covered two frames of ground in one. A harness that manufactures the
stutter it is meant to measure is worse than no harness, so what pins it now is
a cross-check rather than a comment: the replayed walk's peak is within five per
cent of the bench's own, driven through the real `Crowd`, `mobiles::gaze` and a
real `Follower`. That is the same claim C1 makes about the DST harness, at the
other end of the pipeline.

**And `redraw_interval` grew its third term** — the backlog item this milestone
could not be built without. `Follower::settling` answers whether the eye still
owes the screen a pixel, and the test is exact rather than a tolerance: each
channel approaches monotonically, so if the eye and its target already round to
the same world pixel, no later frame can change what the screen is given. Before
it, the tail of every ease arrived 80ms late and whole — the stutter the filter
exists to remove, arriving just after it.

**And then it was pointed at the complaint it was built for, and found it.** A
straight ten-tile walk should be flat: one tile per hold, a constant speed, and
a rigid eye that is the body. Under a punctual loop it is — the dump's `perfect`
run is 77.8 px/s to the decimal and never a pixel off the oracle. Add eight
milliseconds of wake jitter, which is a quiet desktop, and one frame per tile
covered 1.6 times a walk's ground.

Neither the camera nor the frame rate: the *phase* of every tile. `steer.rs`
arms each step from the previous deadline, so the asks are an exact metronome —
but the news of each one reaches `crowd.rs` when the loop wakes, and a crossing
timestamped there is the right length starting at the wrong instant, by a
different wrong instant every tile. The body's position therefore stepped by the
difference of two latenesses at every boundary, and the eye is the body.

Two rules replaced it, and they answer different halves. **A step starts from
where the body is drawn**, not from the tile it is leaving — `Glide::from` is a
`Gaze` now — so no arrival can move the sprite at all, whatever the wire did;
that is the general one, and it covers NPCs, rollbacks and anything else that
arrives when it arrives. **And the crossing this client commands ends when the
cadence says**, not a nominal hold after it was heard, so the lateness comes out
of the crossing's *speed*, where two per cent is invisible, instead of its
position, where a pixel is not. The worst frame over eight seeds went from 1.28
–1.6 times a walk to 1.036, which is the two per cent and nothing else, and
`dst.rs`'s `wake_up_jitter_does_not_reach_the_speed` is the gate — a corridor
around the oracle never caught this, because a body that parks for a frame and
then covers two sits inside every corridor in the file.

What is left is named: the client learns about its *own* step one turn of the
event loop late, because the prediction is made on the net task and comes back
through an mpsc. So there are frames between the scheduled boundary and the news
where the honest picture is a body parked on its tile — a stall of up to a frame,
once per tile, where there used to be a jump. Drawing anything else there would
be inventing motion; shortening the news is the fix, and it is a backlog item
about the seam rather than about the camera.

### C5 — the intent

Velocity look-ahead and cursor lean, each smoothed separately and capped
together. `mouse_swirl` is the one that says whether the lean needs its own
filter, and it will.

A note that pays for itself here: the lead is also a **prefetch**. The atlases
grow from `Camera::visible_tiles`, so an eye that leads the body by a third of a
screen asks for the ground the body is walking into before it gets there, for
free.

### C6 — the anchors

The free camera as a first-class anchor rather than a lock that is off: origin
plus offset, edge scroll, a spring return, and the rule that a hand on the
camera outranks the automation until it lets go. This is the RTS and HotS
camera, and it is deliberately last, because it is the one whose shape is least
constrained by anything above.

## What of the general practice is taken, and what is not

The catalogue this was cut from is the standard one for isometric ARPGs. What
this client takes:

**Taken** — the target as an entity of its own rather than "the character"
(D2, C0); frame-rate-independent damping (D5); dead zone with an idle recentre
(C3); velocity look-ahead and cursor lean (C5); cuts as events with a distance
backstop (D6); every screen-shaped quantity in screen fractions (D3); the camera
as a pure function (D8); the sub-pixel accumulator (D7); following the
*predicted* body so the frame does not lag by a round trip, with the correction
absorbed rather than relayed (D6, C3).

**Deferred, with a slot kept** — impulses and shake (stage 7, empty, so that it
is never added to the filter's state); bounds and camera volumes (stage 6, empty,
because a body-anchored eye is on the map by construction); multi-target framing
(the anchor stage is where a centroid goes).

**Not taken** — composition offsets under a HUD, because there is no fixed HUD
to compose around and the panels move; occlusion and roof fading, which is a
real UO feature and a rendering one, not a camera one; dynamic combat zoom,
which fights a discrete ladder and would breathe.

## Backlog

Found while planning this, and not to be lost in it.

- ~~**`Control::follow_body` takes a rounded, `z`-folded pixel.**~~ It takes a
  `Gaze` (C0), and `world_position` is that rounded rather than a second formula.
- **A packet is not a frame, and two call sites now say so with a zero.**
  `App::entered` and `App::walk_offline` call `follow_player` with
  `Duration::ZERO`, which is right — time passes in `draw` — and means that under
  any rig but `HARD` those two calls move the eye not at all and are there only
  to refresh the glide. When C3 lands, they want splitting into "the target
  changed" and "a frame passed", which is a seam this plan has not argued yet.
- ~~**`redraw_interval` knows about gliding bodies and not about a settling
  eye.**~~ It has three terms now (C4): a gliding body, a settling eye, and a
  scenario waiting to deliver its next knot.
- **A replay and the keyboard both write the player's position.** Walking
  cancels the scenario, which is the same rule as a hand on the camera
  outranking the lock — but it is enforced in `App::walk` rather than by the
  types, so a third writer would have to remember. The offline placeholder is
  the only body with two owners; naming who may move it is the fix.
- ~~**`crowd.commanding` is called when a replay starts and never unset.**~~
  `Crowd::commanded` is a `Who` rather than an `Option<Who>`: there is no state
  where this client commands nobody, and `None` is the offline placeholder
  rather than "not named yet". The test walks a placeholder half a step late and
  asserts the crossing still takes exactly one step.
- ~~**The scope's span is a constant and the panel cannot change it.**~~ A
  logarithmic slider, half a second to twenty. `Scope::set_span` deliberately
  does not clear the trace: the frames already held were flown by the same
  camera, which is what makes it different from a rig swap.
- **`relock` snaps unconditionally.** With a cut threshold it should ease when
  the body is on screen and cut when it is not, which is the same rule D6
  already states and one fewer special case.
- **The DST harness copies ten lines of `App::about_to_wait`.** Its own module
  docs say so. The camera adds a second reason to lift that loop into a headless
  unit both can drive, and the bench is the thing that would notice the copy
  drifting. It has drifted once already in a way that is *safe* and should be
  said out loud: the harness has no surface to block on, so it walks the timer's
  16ms where the window now walks the display's refresh. That makes it the
  coarser of the two — smooth under the harness implies smooth at 60Hz and not
  the other way round — but it also means no test in this repository exercises
  the loop the player actually runs.
- ~~**`Camera::look_at(Point)` has one caller and takes a tile.**~~ Gone.
  `Control::relock` takes a `Gaze` and `look_at_pixel` is the one door into the
  eye: a body relocked mid-step is between two tiles, and the tile it is
  nominally on is up to half a tile from where its sprite is drawn.
- ~~**The walk's pace is written down in two crates.**~~ `WALK_HOLD` and
  `RUN_HOLD` live in `crates/common/movement`, beside the anti-speedhack floors
  they are twice — which is the only place the two numbers make sense next to
  each other. The equality test in `dst.rs` asserted nothing once there was one
  constant, so it went, and `pace.rs` gained the pin against ServUO's
  `WalkFoot`/`RunFoot` instead.
- **The bench has its own SplitMix64 and so does `dst.rs`.** Six lines each, in
  two crates, for the same job. Worth one home if a third appears.
- **`step_var` is a variance and the plan asked for a histogram.** The variance
  catches the ratchet the metric exists for; what it cannot show is *which* step
  sizes a rig produces, which is the thing to look at when two rigs have the same
  variance and look different.
- **The DST harness walks a flat field, so the lift is never exercised on the
  real pipeline.** `Field` answers `can_step` and nothing about height, which
  means C2's whole subject — a `0x22` revising the ground, a stair, a trapdoor —
  is tested only against the bench's scripted body. A terrain with heights in it
  would close that, and it is the same gap that will hide the first real bug in
  the lift cut.
- **The lift cut is a detector where an event is available.** `mobiles::gaze`
  can see whether the body is mid-glide, and `App::entered` knows a correction
  arrived (`Moved::Snapped`) — either of which says "this height did not come
  from walking" outright, where the threshold has to infer it. D6 says the
  distance is a backstop and the event is the rule; C3 is where the event
  channel appears, and the lift's cut should move onto it then.
- **A free camera has no map clamp** and can be panned into the void. Harmless
  today, a stage-6 job when it stops being.
- **The loop's own cadence had no instrument, and the camera's did.** "The frame
  rate drops when the character stops" is a true observation about a rule this
  plan already relied on — `redraw_interval` fell back to the animation clock's
  80ms the moment nobody was walking — and there was nothing on screen that said
  so, which makes a design decision read as a stall. The `Frames` panel is the
  answer: the interval between two *drawn* frames and what each cost to build,
  side by side, because a drop is either pacing or cost and the fixes are
  opposite. Measured on its own `last_frame` and not on `App::last_advance`,
  which an arriving packet also moves.
- ~~**And whether 80ms standing is *good* is still an open question.**~~ It was
  looked at with the switch on, and the answer was no. The loop is paced by the
  display now (C4): `PresentMode::Fifo` by name, a redraw asked for at the foot
  of every frame while the window is watched, and the animation clock kept for
  the window nobody is looking at. The checkbox went with the question.
- **The safety-net timer and the display both ask for frames.** With the display
  pacing, `about_to_wait`'s animation clock is only there for the paths where
  `draw` returns before it can ask for the next frame — no window, a rebuilt
  swapchain, a refused texture. The requests coalesce so it costs a wake and no
  frame, but "two things ask and one of them is redundant most of the time" is
  the shape of a rule nobody can state later. Each early return asking for its
  own frame would let the net go.
- **`Frame::wait` is a lower bound on what was spent waiting for the display.**
  It times `get_current_texture`, which is where `Fifo` blocks — but a backend is
  free to make `submit` or `present` block instead, and that time lands in the
  world's column as if the client had spent it. Worth pinning against a frame
  counter from the surface if the world's cost ever reads high with nothing on
  screen to explain it.
- **Nothing throttles a watched window that is doing nothing.** Focused and idle
  is now sixty frames a second of the same picture, which is what every other
  client does and is still a laptop battery. The honest form of a fix is a
  throttle on *unchanged output* rather than on "nothing is moving", which is a
  question this client cannot currently answer — the world texture is rebuilt
  whether or not it would differ.
- **This client hears about its own step one turn of the event loop late.** The
  prediction is made by `client/net`'s `Walk` on the net task and published back
  through an mpsc, so `about_to_wait` sends the `0x02` and the window learns
  where the body went on a later wake. With the crossing now on the cadence's
  schedule that is the whole of what is left of the walk's unevenness: the frames
  between the scheduled boundary and the news draw a body parked on its tile,
  which is honest and is still a stall of up to a frame per tile. The offline
  path does not have it — `App::walk` folds the step in the same wake — and the
  fix for the online one is to predict on the window's side, which is a question
  about the `link` seam and `docs/client.md`, not about the camera.
- **The walk's unevenness has a gate and the eye's does not.**
  `never_outran_a_walk` is over the drawn *body*; the eye's own `step_var` and
  `still_frames` are computed by `Metrics` and asserted nowhere. Under `HARD`
  they are the body's numbers plus the quantiser, so there is nothing to catch
  yet — under C3's filter there will be, and that is the moment to bound them.
- **`Crowd::crossing` trusts the same band as `glide_time` and says so twice.**
  Half to double the nominal, written out in both, because they are answering
  different questions with the same arithmetic. If a third one appears they want
  one home.
- **`FRAMES_SPAN` is a constant the panel cannot change, again.** The same item
  the scope's span just stopped being, and left as a constant deliberately: the
  slider belongs to whichever of the two rings turns out to be looked at
  longest, and one of them should probably drive both.
