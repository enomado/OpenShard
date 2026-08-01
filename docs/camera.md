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

A `Script` is a list of timed acts — the `Act` from `dst.rs` generalised to
carry cursor motion as well as arrows. One type, three consumers, which is what
keeps the three from measuring different things:

- the **pure bench** drives `Follower::advance` from a synthetic gaze;
- the **DST sim** drives the real `steer.rs`, `Walk`, `Crowd` and a shard over a
  wire with latency and jitter on it, and feeds the camera what actually came
  out;
- the **playground** replays the same script in the window, so a number and a
  picture are about the same event.

The scenarios, and what each is for:

| Script | What it is for |
|---|---|
| `stand_still` | A flat line. Any motion at all is shimmer, and it is the cheapest possible test of D7. |
| `ten_east` | The baseline walk: lag, and whether the eye's speed is constant. |
| `back_and_forth` | The kite. A reversal every few steps — overshoot, settle time, and the reason the spring exists. |
| `mash` | Direction changes faster than the walk can answer, which is where the queue rule shows through into the camera. |
| `rollback` | A `0x21` mid-step. The correction must be absorbed, and *not* cut across. |
| `teleport` | A recall. Must be cut, and must leave no tail. |
| `stairs` | `z` up and down a few units at a time — the whole of C2. |
| `dungeon` | A large drop at once: the boundary between the lift filter and a cut. |
| `mouse_swirl` | The cursor circling while the body stands. Lean's own jitter, with nothing else moving. |
| `frame_jitter` | Any of the above, with `dt` drawn from a jittery distribution. D5's property. |

### What is measured

Everything in **screen pixels at the current zoom**, because that is what an eye
sees, and everything over the *eye's* trace, not the body's:

- **lag** — max and RMS distance from the eye to the gaze. How far the camera
  trails.
- **velocity, acceleration, jerk** — the first, second and third differences per
  unit time. Jerk is the number that means "ragged": a camera that changes its
  acceleration abruptly is one the eye reads as stuttering even when its path is
  smooth.
- **step histogram** — the distribution of per-frame whole-pixel movements. This
  is the one a continuous metric misses: at a constant body speed, an eye that
  moves `0,0,3,0,0,3` and one that moves `1,1,1,1,1,1` have the same mean
  velocity and only the first is visibly a ratchet. The metric is the variance
  of the per-frame step over a stretch where the body's speed is constant.
- **overshoot and settle** — after a reversal, how far past the body the eye
  goes and how long until it is within a pixel of steady state.
- **cut count** — how many times the pipeline decided to cut. A camera that
  smooths beautifully by cutting whenever it is behind has not smoothed
  anything, and this is the number that says so.

Every one of them is asserted with a companion that says the data is real: the
body moved at least *n* tiles, the trace has more than *k* frames, the reversal
actually happened, the rollback was actually delivered. A metric over a scene
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
   the picture means the metric is wrong, and that has to be visible.
3. **A scope in the window** — a strip chart of the last few seconds of eye
   velocity and jerk, drawn with `egui::Painter` lines, beside a preset picker
   and a slider per `Rig` field. From the moment this exists, choosing a camera
   is looking rather than arguing.

The metric functions take a slice of samples and nothing else, so the offline
runner and the live scope compute the same numbers from the same code.

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

### C1 — the bench

`Script`, `Trace`, the metrics, the table, the CSV and the SVG, and D5's
frame-rate property. Runs against the pure function only — no window, no shard.

Green on `HARD`, with the metrics recording **how bad it is**. The baseline is
the deliverable: every later milestone is a diff against it.

### C2 — the lift

Damp `z` on its own clock, with its own cut threshold for a drop that is a
change of floor rather than a stair. Scripts `stairs` and `dungeon`. The table
grows a row and the first real decision — how slow the lift may be before a
stair feels like a lift shaft — is taken by looking at two curves.

### C3 — the spring

Plane damping, the dead zone, and the idle recentre that stops the dead zone
stranding the body off centre. Scored on `back_and_forth` and `rollback`: the
claim is that overshoot stays under a bound while the rollback's jerk drops by
an order of magnitude against `HARD`, and both halves are asserted, because a
camera that absorbs a rollback by never keeping up is not the camera anybody
asked for.

### C4 — the scope

Sliders, presets and the strip chart in the shell, and a script runner that
walks a virtual player through the bench's scenarios in the window. Placed after
C3 in the numbering and worth pulling forward the moment C2 lands: from here on
every remaining decision is a matter of looking, and a slider is faster than a
rebuild.

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
- **`redraw_interval` knows about gliding bodies and not about a settling eye.**
  The moment the filter exists, a frame is worth drawing while the eye is still
  converging even if nothing else moved — otherwise the tail of every ease
  arrives 80ms late, in one jump. `Follower::settling()` is the term to add.
- **`relock` snaps unconditionally.** With a cut threshold it should ease when
  the body is on screen and cut when it is not, which is the same rule D6
  already states and one fewer special case.
- **The DST harness copies ten lines of `App::about_to_wait`.** Its own module
  docs say so. The camera adds a second reason to lift that loop into a headless
  unit both can drive, and the bench is the thing that would notice the copy
  drifting.
- **`Camera::look_at(Point)` has one caller and takes a tile.** Once the gaze is
  decomposed, looking at a tile is a lossy way to say what the camera wants; the
  pixel form is the one to keep.
- **A free camera has no map clamp** and can be panned into the void. Harmless
  today, a stage-6 job when it stops being.
