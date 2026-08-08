# Lighting, rebuilt — the renderer this should have been

A specification, not a repair. Everything in `docs/lighting_height.md`'s backlog
is a compensation for missing data, and this replaces the data instead of the
compensations.

The decision it rests on is stated once, at the top, because everything else
follows from it: **the art is albedo, and the light is ours.** UO's sprites are
drawn with light already in them, and every workaround below exists to avoid
arguing with that light. We stop avoiding it. The picture will not be "exactly
like UO", and that is the accepted price.

Stated once more in the form it was decided, because the difference is what makes
this a specification rather than another compensation: **the sprites are treated
as though they were already de-lit — perfectly clean albedo — and this renderer
is the ordinary one every other renderer is.** No invention of ours stands
between a sprite and a light. Where that assumption is false, it is false in the
picture, not in the code.

## The three roots

Not ten workarounds — three decisions, each with a family growing out of it.

**1. The art is pre-shaded, so a real BRDF was forbidden** *(retired at phase 3)*.
A Lambert term would be a second light fighting the painted one, so `light::faces`
is a half-space instead — and a half-space is a step, and a step has to be
softened, so `FACE_EDGE` is a band. That band is measured in *tiles along the
plane's normal* and `z` is divided by `Z_PER_TILE = 11`, which makes one constant
mean ±4 screen pixels across a wall and **±1.1 `z` above a lid** — more than half
a stair step. Measured 2026-08-08: with the flame between two treads, **7059
pixels** of a single flight sit inside that band against `3940` of genuine
penumbra, and the band's price peaks exactly where a flame lies in a surface's own
plane — `0.214` of a channel per pixel, against `0.020` half a step away.

**2. The `place` attachment packed a fragment's height into eight bits and a
four-bit fraction** *(retired at phase 2; the constants it justified went at
phase 4)*,
so a fragment's own position was not exactly known — so a
shadow ray must start away from where it really is. `STAND_OFF = 2/127` of a tile
and `ON_TOP = 1/128` of a `z` unit are **numbers taken from the byte layout**,
not from any statement about surfaces. Their price, measured with the light
oracle: the engine is brighter than the geometry allows on the top band of a
riser by up to **`0.51` of a channel**. And because heights cannot separate
surfaces at that precision, a whole apparatus grew to do it by identity instead
— `exemption`, `on_surface`, `own_run`, `mounted_at`'s height test. Phase 4 took
the bias to zero and dissolved `exemption`; two of that apparatus survived it and
the phase's own account says which, and why each was kept by a measurement.

**3. A static is drawn twice** — as a sprite, and as a mesh over it — so their
silhouettes differ, so the mesh is grown to hide the gap *(the border retired at
phase 6; the second draw goes with the rest of that phase)*.
`WIDTH_OVERLAP = 0.03` of a tile was a **1355-pixel border** around a single
flight at `4:1`, measured by zeroing it; and in a scene with no sprite it bought
nothing at all. It is the case `docs/style.md`'s *No fudge constants* was written
from.

## The target

Deferred shading, in the ordinary sense:

```
geometry pass ──► G-buffer ──► lighting pass ──► tonemap ──► screen
                  position      per light:        HDR → LDR
                  normal          BRDF × falloff
                  albedo          × shadow rays
                  ids
```

Every quantity lives in one place, in one unit, and is *data* rather than
something reconstructed downstream:

| quantity | where it lives | unit |
|---|---|---|
| fragment position | G-buffer, `Rgba32Float` | tiles, `z` **in tiles** |
| surface normal | G-buffer, `R32Uint`, octahedral (was to be `Rg16Snorm` — see phase 2) | unit vector |
| albedo | G-buffer, `Rgba8UnormSrgb` | linear after decode |
| primitive identity | G-buffer, `R32Uint` | opaque id |
| light accumulation | offscreen, `Rgba16Float` | linear radiance |
| screen | swapchain | tonemapped, sRGB |

**One metric.** `z` is divided by `Z_PER_TILE` **once**, where the map is read,
and never again. Nothing downstream knows that `z` was ever counted differently
from `x`. Half of `docs/world_coordinates.md` is this line.

## What goes

Named, so the plan can be checked against the tree:

| goes | what replaces it |
|---|---|
| ~~`light::faces`, `FACE_EDGE`~~ **gone, phase 3** | `light::lit_from`, `max(N · L, 0)` off the G-buffer normal |
| ~~`STAND_OFF`, `ON_TOP`~~ **gone, phase 4** | exact position + self-hit by primitive id, bias `0` |
| ~~`exemption`~~ **gone, phase 4**; `on_surface` and `own_run` **stay, measured** | the same id test, once — inline, in both walks. The other two are `same_run`, which is a *different* claim: see phase 4 |
| ~~`mounted_at`'s height test~~ **gone, phase 4**; `mounted_at` and `MOUNTED_CLEARANCE` **stay, measured** | a sconce burns where it hangs, which the map does not say and the art does — see phase 4 |
| ~~`WIDTH_OVERLAP`~~ **gone, phase 6**; `Prism::mesh`'s `widen_footprint` went with it | one silhouette: a static is drawn once and its geometry met by the view ray — see phase 6 |
| ~~`FLAME_DEPTH`, `pierces`, `crosses`'s softening, `SOFT_CROSSING_*`~~ **gone, phase 5**; `inside` and the `spread` parameter went with them | an area light and eight shadow rays at `FLAME_RADIUS` |
| `(1 − d)²` falloff | windowed inverse square |
| `knee()` | a tonemap on HDR |
| ~~`place`'s `z + 128` · fraction · stance packing~~ **gone, phase 2** | position and normal as data; an id word for what is left |

`RAY_CUTOFF` and `MAX_WALK_STEPS` survive: a ray does have a cutoff and a walk
does have a step budget. They stop being *stand-ins* for the things above.

**`FLAME_SPREAD` was to survive too — "a light does have a size" — and phase 5
found that it was not one.** Its `1.0` of a tile was the numerator of a penumbra
ratio, chosen for the edge it drew; the idea survived and the number did not.
`FLAME_RADIUS` is an eighth of a tile, off the art, and it is the size.

## How this is judged

**The instrument is a picture beside the path tracer's, looked at by a person.**
Not a number, and not a second implementation of our own arithmetic. Written down
here because it decides what a test in this crate is *for*, and because it retires
most of what the tree called a lighting test.

Twelve went on 2026-08-08 — nine `the_shader_…_agrees_with_light_sample` and the
three flat-face parity rungs, 1,172 lines, `tests/frame.rs` from 5,981 to 4,809.
The reason is one sentence: **their subject is the agreement of two of our own
implementations of the model phases 2–5 delete.** A sweep comparing `blit.wesl`
against `light::sample` cannot go red because the model is wrong — only because
the model is replaced, and both of its sides are being replaced. `assert_parity`,
`assert_parity_of`, `assert_single_face_parity`, `assert_two_face_edge_parity`,
`ring_of_lights` and `single_face_bounds` went with them.

Two of the twelve carried the `#[ignore]`d corner-tie tie-break, so the CPU/GPU
tie-break gap `lighting_raymarch.md` records is now recorded *only* in prose. It
was never going to be closed by a test that outlives the walk it is about.

What survives, and the rule that decides it — **does the test's subject survive
the rebuild?**

- **The brute-force oracles stay.** `tests/lighting.rs`'s `brute_force_blocked`
  and `frame.rs`'s `ground_truth_blocked` are dumb fixed-step point samplers
  against `solids_at`'s own boxes: no DDA, no `floor()`/`fract()` reconstruction,
  no shared arithmetic with either walk. Their subject is the occlusion grid and
  its boundary derivation, which phase 4 keeps. This is the one non-circular
  coverage in the tree and retiring it would be retiring a role, not an
  instrument.
- **World claims stay, as claims.** "A shut room keeps its light inside", "a hole
  in a floor lets the light through", "a torch does not light the storey above
  it" are statements about the world and survive the rebuild verbatim. Their
  *margins* do not: `> 0.2` and `< 1e-6` were calibrated against a pipeline that
  has already changed once under them — see `brighter_by`'s own account of what
  phase 1 did to every one of them. Expect to re-take them per phase, and expect
  that re-taking to be a judgement rather than a fix.
- **Pipeline mechanics stay untouched.** Blit texel-for-texel, the hue ramp,
  sprite silhouettes, depth order, the camera. None of it is about light.
- **Pictures are promoted.** `tests/pictures.rs`, `tests/traced.rs`,
  `dump_the_lighting_views`, `examples/synthetic_stair.rs` are no longer a side
  channel — they are the acceptance instrument. The engine's frame and the
  tracer's are side by side as of phase 0: `boxes.rs` writes
  `<base>_lit_vs_traced.png`, three strips — ours, theirs, and the difference
  amplified `8×` so that an agreement is a black rectangle and a disagreement is
  not.

## What arrives, in detail

### The G-buffer

Four attachments and a depth buffer. Position as `Rgba32Float` rather than
reconstructed from depth: the isometric projection is invertible and the
reconstruction is exact in principle, but it is also the single thing every
defect on the height track came from, and this plan does not re-earn that. (An
optimisation later, gated on a test that reconstruction equals the stored
position to `1e-5` of a tile, is welcome.)

`ids` carries what `place`'s `kind` and `id` carry today, because selection,
outlines and tooltips read them — plus a per-**primitive** index, which is the
thing shadow rays compare.

### The BRDF

`albedo × N·L × light colour × intensity × attenuation × shadow`, summed over
lights, plus `albedo × sky colour × sky visibility` for ambient.

**The art is clean albedo by decree, and there is no dial.** The pre-shading in
the sprites is not compensated for, not softened against and not measured — it is
declared to be albedo, and every surface is lit by the same textbook Lambert any
other renderer would use. No stylised wrap, no half-space, no width knob between
the two, and no term anywhere whose job is to argue with the artist.

That knob was in this document until the decision, and it is worth recording what
it was so it is not reinvented: `light::faces` is
`clamp(along / FACE_EDGE + 0.5)`, and that shape — `N·L × k + 0.5` — is *wrapped
diffuse*, the ordinary stylised BRDF, so a width of `2.0` would have made it
half-Lambert and the width a dial between "pre-shaded look" and "Lambert". Gone.

**What survives from that reading is a diagnosis, and it says phase 3 is smaller
than it looks.** What `faces` is passed is not a cosine: `along` is
`dot(normal, toward)` with `toward` left **unnormalised**, so today's argument is
a *distance*. That single missing normalisation is where root 1's two scales come
from — one constant meaning ±4 screen pixels across a wall and ±1.1 `z` above a
lid. Phase 3 normalises the argument and takes the full `N·L`; it does not have a
band to retune, because there is no band.

Lambert with no `1/π`, and the intensities calibrated to match: the constant is a
convention, and putting it in would mean re-tuning every flame in the tree for a
factor everyone then divides back out. Stated here so nobody re-derives it as a
bug.

### Attenuation

Windowed inverse square — the standard form, physical near the source and
finite at the rim:

```
let d2     = dot(L, L);              // squared distance, tiles
let window = saturate(1 - (d2 / (radius * radius))²);
let falloff = window * window / (d2 + 1);
```

The `+1` keeps a fragment at the flame's own position finite. `(1 − d)²` gave a
pool with a straight-edged gradient and no physical meaning; this gives the same
"soft pool with a hard end" the reference isometrics draw, and agrees with a path
tracer, which the old one cannot.

### Shadows

A ray per light per fragment, against a uniform grid of primitives — the
`occlusion` grid as it stands, with primitive ids added.

**Self-intersection is solved by identity, not by epsilon.** `if hit.primitive ==
origin.primitive { continue }` — the textbook answer, exact, with no tolerance
anywhere. A ray leaving a tread *does* cross its own flight's riser when the
geometry puts one in the way, and it *should*: that is a real occluder standing
in a real place. Every "my own static does not shadow me" rule goes.

Bias is `0`. If a grazing case ever needs one, it is `normal * k * pixel_scale`
where `pixel_scale` is `length(fwidth(world))` — a nudge in units of *how big
this pixel is*, which is the only honest unit for one.

### Soft shadows

A flame is a sphere of radius `FLAME_RADIUS`. `N` shadow rays per light towards
stratified points on it, `N = 8`. Sample positions rotated by a per-fragment
offset so the error is high-frequency rather than banded. No temporal accumulation
in the first pass; if `8` rays is too noisy or too slow, that is the moment to add
it, and not before.

This deletes the entire `pierces`/`crosses`-softening apparatus, whose band is
`soft × FLAME_DEPTH` with `FLAME_DEPTH = Z_PER_TILE/4` — a penumbra sized for a
wall's top edge three tiles away, applied to an edge a fifth of a tile away.

**Landed at phase 5, with two things this paragraph got wrong.** The radius is
`FLAME_RADIUS` and not `FLAME_SPREAD`, which was never a size — see that phase.
And the blue noise is *world*-space rather than per-pixel, because the CPU twin
has no pixel and a float hash is what two backends disagree about; the
hard-shadow debug view was not built, since the sun still casts one ray through
the same walk and is the hard-shadow case a frame already shows.

### One silhouette: the sprite is the shape, the prism is the geometry

The mesh is *narrower* than the art (`best_prism`'s score is never exactly `1.0`),
which is why it was grown. Neither growing it nor clipping the sprite to it is
right: the art is the artist's statement of the shape and the prism is our
approximation of its volume.

So: **draw the sprite, and get position and normal by intersecting the view ray
with the prism analytically in the fragment shader.** Impostor rendering, the
ordinary kind. The silhouette is the sprite's, exactly as today; the depth and
the normal are the prism's; there is no second draw and therefore no seam and no
overlap constant. A pixel of the sprite whose ray misses the prism takes the
nearest point on it — the art overhangs its own volume by a pixel or two and that
is what it means.

### Billboards

A mobile is a sprite with no volume, and `N·L` needs a normal. Two candidates,
both cheap, and the phase picks by looking:

- **facing the camera** — a flat billboard lit as a plane turned towards the
  viewer. Never wrong, never interesting;
- **inflated from the silhouette** — the signed distance field of the sprite's
  alpha gives a gradient, and `normalize(vec3(∇sdf, k))` rounds the figure off.
  This is what 2D games with dynamic lighting do, it is computed once per art
  frame at load, and it makes a person standing beside a torch look like a person
  beside a torch.

Mobiles remain non-occluders. A billboard is no volume, so it casts nothing.

### Colour

sRGB in, linear throughout, tonemap out — the thing the current pipeline does not
do at all, and the reason it cannot agree with any reference renderer even in
principle. Art atlases and hue ramps are `…UnormSrgb` so the hardware decodes
them; accumulation is `Rgba16Float`; the final pass applies exposure and an ACES
fit.

Hue tinting is untouched: it indexes a 32-entry ramp by the art's own red channel
(`statics.wesl`'s `round(r * 31)`), and this plan never rewrites the art. What
changes is that the ramp's colour is decoded to linear before the light
multiplies it — which is what makes a tinted cloak in torchlight the same colour
as a tinted cloak in daylight, only dimmer.

## Phases

Each is landable alone and leaves the tree working.

**Phase 0 — the reference, and it must judge the same model.**
`crates/client/pathtrace` (in flight in a parallel session) becomes the oracle,
with a **BRDF switch**: it has to be able to compute what the engine computes, or
the choice of model is made by the choice of instrument rather than by us.
`synthetic_stair`'s light oracle (`write_light_reference`,
`write_light_difference`) is the comparison harness and already reports by class.
*Done when:* the path tracer and the engine agree on a scene with one flame and
no occluders, to within the frame's own quantisation — which is a statement about
falloff and colour handling alone, and is the calibration everything else rests
on. **Done.** The scene is `boxes.rs`'s `flat`, the gate is
`the_frame_and_the_path_tracer_agree_about_brightness_on_open_ground`, and the
measurement is **262,144 pixels compared, worst channel one step of 255** —
257,972 of them identical and the remaining 4,172 exactly one step apart. The
tolerance is `2` and the residual is `1`, so it is a quantisation rather than a
margin sized to fit; at `0` the gate goes red, which is how that was checked
rather than argued.

*What had to become true first*, and each was a difference that is not about
light:

- **the albedo is the same on both sides.** `oracle::ground_albedo` reads it off
  the world texture the ground pass drew and decodes it, so it is a measurement
  and not two authors writing the same constant down. `Mirror::of`'s
  `[0.42, 0.44, 0.40]` is now `Albedos::INVENTED` — still the value where a
  comparison does not read colour, but a call site has to *say* so.
- **the flame is the same flame.** `Light`'s own colour and intensity travel to
  the reference through `Mirrored`; the tracer's own `intensity: 6.0` was picked
  to make its own picture readable and made every shaded comparison meaningless.
- **one curve.** `tonemap::encode` is the radiance-only half of `shade` —
  `linear_to_srgb(tonemap(x))` — and both pictures go through it.
  `pathtrace_comparison`'s hand-rolled sRGB, with a `clamp` where the shoulder
  is, was a second spelling of it and phase 1's own rule forbids one.
- **the ambient is nothing, deliberately.** A degenerate path trace is direct
  light and has no ambient term, so `NIGHT` would be a constant on one side of
  the comparison only — and not one that could be subtracted back out, since the
  sum passes through a tonemap. Giving the tracer an ambient instead would put
  this renderer's own model inside the thing that checks it.

The scene has no boxes for the fourth reason the backlog named: `mesh_face.wesl`
writes no colour, so a box's face has nothing on the engine's side to compare a
body albedo against. That is phase 6's, and `Albedos::body` stays invented until
then.

*And it found a defect in the instrument on its first run.* Both pixel oracles in
`boxes.rs` read `Shade::lit()`, which answers `false` for a fragment **outside
every flame's radius** as well as for a shadowed one — and compared it against
`oracle_visible`, which is pure geometry and knows nothing of a torch's range.
`Shade` exists to make exactly that distinction available and its own doc says a
caller that must not count it has to match on the variant. Every scene until now
had its flame reaching the whole canvas, so the conflation never fired; `flat` at
1:1 reported **67,728 of 262,144 ground pixels "rendered too dark"**, every one
of them simply out of reach. Both oracles now skip `Shade::Unreached` and report
how many they skipped.

**Phase 1 — linear and HDR.** *(Landed.)* sRGB decode, the multiplication in
linear radiance, exposure and an ACES curve, encoded once.
`shaders/tonemap.wesl` and `src/tonemap.rs` are the pair, and nothing else in the
crate may spell those curves again.

What it cost, which was not the shader: **every authored light value silently
changed meaning.** `NIGHT.sky = 0.20` was a fraction of a *displayed* value, and
`0.20` of radiance is an overcast afternoon — the first frame after the change
had no night in it at all. So every one of them is now `srgb_to_linear` of the
number a person chose, with the chosen number kept in
`the_authored_light_values_are_their_own_srgb_intent`: the artistic intent stays
written down beside its conversion, and a constant nudged by hand to make a
picture look right turns that test red instead of quietly redefining what "night"
was. `GROUND_AMBIENT`, `NIGHT`, `SKYLIGHT`, `TORCH`, `CAMPFIRE` and `midday`'s sun
all moved; the campfire's `1.25` is past sRGB's domain and carries the exponent
alone.

Three tests changed rather than broke, and each got stronger for it. The blit's
"copy, byte for byte" is now `tonemap::shade_u8` of the world texel — it catches a
blit that shifts by a texel *and* a colour pipeline that has drifted from its own
twin. The CPU/GPU parity sweep predicts through the same pipeline. And the pool
test's ratios are taken in **linear** light, because "twice as bright" was being
asserted about sRGB bytes, where it means nothing.

*Done when:* two equal flames are twice one flame in linear light
(`two_equal_lights_are_twice_one_in_linear_light`), and the picture baselines are
re-taken deliberately. **Both done.**

What phase 1 deliberately did *not* do: `Rgba16Float` accumulation. The whole
composition happens in one shader pass, in `f32` registers, so there is nothing to
store at intermediate precision yet — the moment a second pass appears (bloom, or
the glow layer), that is when the target format matters.

And what the pictures say, three ways, on `one-torch-on-open-ground` and
`a-shut-room-with-a-torch-in-it`: the old pipeline and the restated one put the
**night at the same level** — which is the whole claim of the restatement — while
the pool between them is wider, warmer and no longer burnt to a white core, since
the light now sums physically and the shoulder holds the top instead of a clamp
flattening it. The middle picture, linear light with the old numbers, is what
"the constants silently changed meaning" looks like: no night at all.

**Phase 2 — the G-buffer.** Position, normal, ids, albedo. `place`'s packing goes;
its readers (select, outline, tooltips, every oracle in `examples/`) move to `ids`.
*Done when:* a `View::Normal` shows the geometry's own normals, and a test asserts
the stored position equals the world position the mesh pass computed, exactly.
**Both done** — position and normal below, and the id plane after them retired
the `place` attachment outright. What is left of the phase is albedo, which is
phase 6's: a mesh face has none.

*Position landed.* `crates/client/render/src/gbuffer.rs` is the set — a `Gbuffer`
owning the planes and a `Views` lending them, so the two still to come are one
edit each and not thirty. The plane is `Rgba32Float`, written by all three world
passes and read by `blit.wesl` as `at`; `unpack_place_z`, the seven-bit fraction
decode and the whole `tile + sub` reconstruction are gone from that shader.
`a_mesh_face_pixel_carries_its_exact_world_position` is the phase's own "done
when", half of it: the mesh pass is the producer whose vertices carry true world
positions, and the test picks a point at `15.1` above a tile at `0.3, 0.7` —
a height no sixteenth and a fraction no hundred-and-twenty-seventh can hold — so
that it fails if anything on the path quantises. It asserts the packed height
beside it, to compare the two rather than merely have both.

Three things it deliberately did not do. **`z` stays in `z` units:** the
occlusion grid, every solid's span and the whole walk are stated in them, and a
G-buffer that alone counted in tiles would be a second metric rather than one.
**The tile stays a row lookup:** it is what the walk starts cell stepping from,
and `floor`ing a position back into it is the class of bug `walk`'s own comment
records. **The position is clamped into its tile** exactly where `pack_place`
clamps the fraction, so this step changed precision and nothing else; the clamp
went at phase 4 — not because the cell stopped being a separate fact, which it
has not, but because nothing floors that position and eight thousandths of a tile
of error in a ray's origin is the largest thing left once the bias is zero.

*Normal landed.* The plane, `View::Normal`, and — the thing worth saying first —
**a normal is written by the pass that knows it now, not derived by the pass that
reads it.** `blit.wesl`'s `outward(stance)` is gone from the lighting entirely:
`statics.wesl` writes `outward` of the stance it has *just* resolved a corner
into, `mesh_face.wesl` carries `mesh::Face::normal` on its own vertices —
measured geometry, the one producer whose normal was never a stance — and
`ground.wesl` writes a zero outright. That last one closes a `select` on the kind
that had been sitting in the reader: land and a wall's flat cap are one stance
and only one of them wants the half-space gate, and the pass that knows which it
is drawing is the one that says so now. `Stance::normal` is the Rust twin,
`Stance::of_normal`'s inverse, and the two round-trip in a test.
`two_mesh_faces_carry_their_own_two_normals` is the phase's other half of "done
when": a tread's top and its riser, one draw, two normals — and the place
attachment asserted beside them holding `MeshFace` for **both**, which is the
measure of it. The attachment cannot tell those two surfaces apart. The plane
can.

Two things it did not do the way this document said. **The format is
`Rgba32Float` and not `Rg16Snorm`, octahedral.** Every 16-bit norm format is
behind `wgpu::Features::TEXTURE_FORMAT_16BIT_NORM`, which is native-only and not
in WebGPU's core set — so the row in the table above was never available. The
nearest compact renderable format, `Rgba16Float`, is not taken either: the
hand-written producers (`plan.rs`'s diagnostic pictures, `tests/`' fixtures)
*write* this plane from the CPU and there is no `f16` on that side, so it would
mean a hand-rolled encoder — a second spelling of a float format with no compiler
comparing the two. Octahedral has a second problem of its own besides: it has no
zero, and **the zero vector is a value here** — a billboard has no side, and
phases 6 and 7 are the work of leaving less of that in a frame, not of pretending
it is absent today.

And **the client asked an adapter for more than WebGPU's guaranteed minimum for
the length of this phase.** A world pass writes the picture and every plane in
one draw, and `maxColorAttachmentBytesPerSample` bounds the total: the floor is
32 and the set was already at exactly 32 before this — picture 8, `place` 8,
position 16 — so *no* fourth plane fitted, in any format.
`gbuffer::required_limits` is the one place that asks,
`attachment_bytes_per_sample` sums the real per-format table rather than the
widths a person reads off the names, and `a_g_buffer_costs_what_it_says` pins
the total. The cost was stated plainly rather than absorbed while it stood: a
device reporting only the minimum could not run this client. Both later steps of
this phase gave it back — the id plane four (48 → 44) and the packed normal
plane twelve (44 → 32) — so the assertion now reads the other way, and the
target layout phase 6 has to hit is that same 32 with no separate picture beside
the albedo.

*The id plane landed, and it is where the attachment ended.* `place`'s eight
bytes a fragment were an id, a height in whole units and sixteenths, a stance
and seven bits of tile-local `x` and `y`. The position plane had already taken
the height and the fraction and the normal plane the facing the stance stood in
for, so what was left was **six bits and an id** — `gbuffer::pack_ids`, one
`R32Uint`, kind in the low two bits, stance in four above it, the row in the
twenty-six above that. `crate::place` keeps `Kind`, `Stance` and `Place` — the
vocabulary and the *instance row*'s own two words — and carries no attachment
format at all; `packed_height`, `unpacked_height`, `Z_FRAC_*`, `SUB_TILE`,
`STANCE_SHIFT`, `FORMAT`, `texture` and `CLEAR` are gone, along with
`place_format.wesl`'s `pack_place` and `unpack_place_z`.

**The kind is at the bottom of the word on purpose.** The clear value is zero
and `Kind::Nothing` is zero, so a pixel nothing drew and a pixel a pass stamped
as nothing are the same number — which is the invariant every reader's first
branch rests on, and the one thing a layout can quietly break.
`nothing_drawn_and_nothing_cleared_are_one_kind` and
`an_id_word_holds_three_things_and_gives_all_three_back` are the two halves of
it.

**It bought a third of the budget back, which is why it went next.**
`ATTACHMENT_BYTES_PER_SAMPLE` was 44 against 48, and the twelve still over
WebGPU's floor of 32 were the normal plane's — packed next, below, which is what
brought the total to 32 exactly. And the stance survived the move, so the phase
did not retire it: `blit.wesl` still reads it to route a mesh face's id to its
own instance buffer and to ask the shadow walk's own-run test which edge a
fragment stands on. **Phase 4 is what retires the second**; the first goes when
a mesh face stops being a pass of its own.

Two things it changed that are not the format. `parity_place`'s sub-tile
fraction is an `f32` rather than sixteen-of-a-hundred-and-twenty-seven, kept at
the same grain so that no parity margin moved for a reason that is not under
test. And `View::Place`'s checkerboard is drawn from the **tile** now: it was
taken from the two halves of the *id*, so a frame's squares counted instance
rows rather than tiles, and it went unnoticed because a diagnostic is read for
whether a gradient is there and both versions have one.

*And the normal plane was packed, which is what put the whole set under
WebGPU's floor.* The plane was `Rgba32Float` — sixteen bytes a fragment for a
unit vector and a coverage bit — and those twelve extra bytes were, after the id
plane, the entire remainder of what this client asked an adapter for above the
guaranteed 32. `ATTACHMENT_BYTES_PER_SAMPLE` is **32 exactly** now, so
`a_g_buffer_costs_what_it_says` asserts `<= floor` where it used to assert
`> floor`, and the sentence "a device reporting only the minimum cannot run this
client" is retired rather than softened.

**It went before phase 6 because it is a term of phase 6's own sum.** That
phase's target layout is position `16` + normal + albedo `8` + ids `4` with no
separate picture beside them — which comes to 32 only if the normal is 4. It was
never a tidiness item.

Four decisions, and the second is the one that had to be made rather than
looked up:

- **`R32Uint`, octahedral, integers on both sides.** `Rg16Snorm` is behind
  `TEXTURE_FORMAT_16BIT_NORM`, native-only; `Rgba16Float` is renderable and was
  refused for the reason phase 2 refused it — this plane is *written from the
  CPU* by `plan.rs` and by two fixtures, there is no `f16` there, and a
  hand-rolled encoder is a second spelling of a format with no compiler
  comparing the two. An integer word has neither problem and turns the encoding
  into the thing this crate already keeps honest.
- **The two non-vectors stayed in the plane, in two bits of their own.** A
  fragment nothing drew and a fragment with no facing are different answers, and
  the four-float plane separated them with its fourth channel. Fifteen bits an
  axis leaves two over, so `NORMAL_DRAWN` and `NORMAL_FACING` carry that split at
  no cost — rather than being inferred from the id word beside it (`KIND_NOTHING`
  and `STANCE_UPRIGHT` do name the same two states today). The plane still means
  something read alone, which is how `View::Normal` and every test that copies it
  back read it.
- **The span is even.** Each axis quantises to `32766` steps and not to the
  `32767` its bits allow, so that zero lands on a code instead of half a step off
  one. Nearly every normal this renderer writes is cardinal — every wall face,
  every lid, every level tile — and an odd span moves all of them by a
  ten-thousandth for nothing. With an even one, all six round-trip bit-for-bit,
  and the sweep's worst over the whole sphere is **`0.0068°`**, against a `0.01°`
  bound taken from what a channel can show rather than from the mapping.
- **The gate is an integer against an integer, not a tolerance.**
  `two_mesh_faces_carry_their_own_two_normals` renders a face and asserts the
  word the GPU wrote equals `gbuffer::pack_normal`'s. `normal_format.wesl` and
  its Rust twin are two spellings no compiler compares, and this is the only
  thing that does — fault-injected by moving the span on one side alone, which
  turns it red. The test grew a **third** face, a slope off every axis, for
  exactly that: the two cardinal ones go through the packing's exact cases and
  would survive a fold spelled differently.

*And one thing the first version of that sweep got wrong, which is worth
keeping.* It measured the angle as `acos` of a dot product and reported the
packing losing `0.028°` — four times the truth. Near zero, `acos`'s derivative
is infinite, so a dot carrying `f32`'s own `1e-7` comes back as `sqrt(2e-7)`,
which is `0.026°`: the number being read was the *instrument's* noise floor and
nothing of the packing was visible under it. The chord is well conditioned —
subtracting two nearby `f32`s is exact — and `2·asin(|a − b| / 2)` is the same
angle.

Left: albedo for a mesh face, which has none — phase 6.

**Phase 3 — the BRDF.** `N·L` replaces `faces`. `FACE_EDGE` is deleted.
*Done when:* the light oracle's "inside FACE_EDGE" class no longer exists, and its
residual against the path tracer is quantisation only. **Both done.**
`light::lit_from` and `blit.wesl`'s twin are `max(N · L, 0)` — `clamp`, one
`normalize`, no constant of any kind between them — and the class the difference
picture spent a colour on is gone from the code rather than reading zero.

*The change was one line and the argument to it.* `dot(normal, toward)` divided
by a width became `dot(normal, normalize(toward))`, and every consequence in this
phase follows from that division: the term stops being a distance in tiles, so it
stops needing a width to be measured against, so `FACE_EDGE` has nothing left to
be. `MOUNTED_CLEARANCE` was `0.5 + FACE_EDGE` and is a plain `0.7` — the same
number on purpose, so that phase 3 moved the picture through the shading term and
through nothing else. **Phase 4 did not delete it** — see that phase for the
measurement that kept it.

*The reference had to be asked a different question, and that is what says the
term is right.* `Brdf::Flat` is a description of the engine **before** this phase
— no cosine, no `1/π`, no notion of a normal — so a brightness gate against it
would have judged us against the renderer we had just replaced.
`the_frame_and_the_path_tracer_agree_about_brightness_on_open_ground` renders
`Brdf::Lambert` now, and the two conventions meet in one place: the reference's
flame carries `oracle::pathtrace::LAMBERT_PI`, because our Lambert has no `1/π`
and physics does. **262,144 pixels compared, 23,564 bright and 238,580 dim, worst
channel one step of 255, nothing past the two-step quantisation.** The engine's
cosine and a path tracer's are the same cosine, measured rather than argued.

The *visibility* comparison beside it stays in `Brdf::Flat`, and the split is
worth stating because it looks like an inconsistency and is not: that variant's
three clauses are one fact — there is no normal — and the third of them, "a
surface point's own body does not occlude it", is still exactly what the shipped
walk does. Phase 4 is what turns that into identity and is where the visibility
gate moves too.

*The scene had to move as well, twice, and each time because a cosine made a
degenerate configuration visible.* A flame at `z: 0.0` is **in** the ground's own
plane, where the cosine is zero everywhere and no pool exists at all;
`light::gather` never builds one there — it adds `FLAME_LIFT` to every light —
so two frame tests were writing "on the ground" and meaning "where a fire on the
ground burns". `FLAME_LIFT` is `pub` now and they say the second. And the
brightness gate's flame went from three `z` to a whole tile up, because a source
a quarter of a tile over flat ground grazes it: the frame had 812 bright pixels
against the ten thousand the gate needs before it is measuring a curve rather
than its tail.

Three world claims were re-taken, and the rule from *How this is judged* held —
each was a judgement about the scene rather than a margin nudged to fit:

- **the pool test and the wall test** got the lift above. The wall test's radius
  went from four tiles to six besides: the far tile was still *inside* the pool
  and no longer said anything there a byte could hold, so the walled and open
  frames read alike and the test would have passed by measuring nothing.
- **the wall-run seam test** asserted a floor of `0.2` on the face beside the
  lamp. A lamp standing *along* a wall grazes it, so the whole face went dimmer
  without the claim under test changing at all. It is a *range* now — the east
  end at least twice the west — which is what "lit from one end" says and what a
  level never did.

*And the ground has normals*, which answers open question 3. `ground.wesl` writes
the bilinear patch's own — the cross product of the two tangents of the surface
its vertices are already lifted to, with the corner heights divided into tiles in
the vertex stage so the fragment stage needs no `viewport` binding for it. A flat
tile's four heights are equal, both derivatives are zero and the answer is exactly
`(0, 0, 1)`, arrived at rather than special-cased. The deliberate zero it replaced
was a defect of the half-space and not of the normal: a floor is the one surface a
flame is routinely almost in the plane of, and gating it blacked out every ground
pixel a fixture was not comfortably above. A cosine is *small* at a grazing flame
rather than absent, which is what a floor lit by a torch standing on it looks
like.

What it costs, and it is the phase's own finding rather than a surprise: **a
surface a flame grazes goes markedly darker, and walls are what a lamp grazes.**
On `a-wall-run-with-a-lamp-along-it`'s elevation the face is plainly dimmer than
the half-space drew it and the gradient is tighter, while `one-torch-on-open-
ground`'s pool is barely changed — which is the shape open question 3 predicted
for land and open question 1 is still about. Nothing here compensates for it and
nothing here should: exposure and ambient are ordinary exposure and ordinary
ambient, and neither has been touched yet.

**Phase 4 — shadows by identity.** Primitive ids in the grid, self-hit by id,
bias `0`. *Done when:* the light oracle reports zero brighter-than-geometry
pixels on the whole flame-height sweep. **Done.** The sweep read
`31 / 15 / 13 / 0 / 0 / 0 / 0` at flame heights `0..6` when the phase started and
read **zero at every one of them**, worst channel `0.000`. (The `175 at z 0`
this line used to quote was measured before phase 3; the cosine had already taken
most of it.)

*Re-read after phase 6 reshaped a tread*, which is the first thing the sweep is
for: `0 / 2 / 0 / 0 / 0 / 0 / 0`, worst channel `0.022` — **two pixels at `z 1`
alone**, one brighter and one darker, and both at a place the reshaping created.
They are in the backlog with their addresses. The face oracle is `0` of `10,824`
at every height, so it is not a disagreement about visibility.

*The rule is one comparison, and every arm of the apparatus it replaced was a
proxy for a name a fragment did not have.*

```
if hit.primitive == origin.primitive { continue }
```

Three readings in order, because each failure says what the next had to be. **A
height inside a span** — two things stacked on one tile meet at a single plane, so
no precision separates them, and two side by side span the same heights outright,
so each was excused from the other while standing squarely in front of it
(`examples/boxes.rs`'s `pair`, three oracles fully red). **An `OwnerId`** — the
*static*, `lighting_height.md`'s own phase 3, right for a wall and one level too
coarse for a flight: one `Builder::add` pushes a lid and a panel per tread, all
wearing one owner, so a tread was excused from the riser that genuinely stands
between it and the flame, and the height came back as `drawn_on` to patch it.
**A `SolidId`** — the primitive itself. A flight's treads shadow each other
because they are different solids, which is what different solids do.

*What the fragment carries it in, and the split is the part worth keeping.* A
mesh face is one primitive by construction, so `MeshFaceRow` carries its
`SolidId` outright; the join is `occlusion::Part`, the `n`th solid one
`Builder::add` pushed, and the `n`th face of `Prism::mesh` is that solid because
both walk the same treads from `treads()` and `up()`.
`a_flight_draws_its_own_solids_in_the_grid_s_own_order` holds that against the
geometry for all four climb directions rather than leaving it as two loops that
agree. A **sprite** instance is not one primitive — a corner is two panels and one
picture, and only a fragment's own stance says which — so `blit.wesl`'s
`own_solid` narrows the instance's owner by that stance, once per fragment. It is
exact for everything but a fitted climbable, whose pixels the mesh pass draws.

*The bias is zero, and the two constants had already lost both of their reasons.*
`STAND_OFF` was `2/127` of a tile and `ON_TOP` `1/128` of a `z` — numbers off the
retired attachment's byte layout. One thing they bought was a ray not starting
inside the surface it was drawn on, which is identity's job. The other was a face
pixel walked from *in front of* its plane, because the attachment placed it
behind one and because a crossing could be found on the wrong cell; phase 2's
exact position and `lighting_raymarch.md`'s per-solid `ray_vs_solid` removed both.
`mesh_face.wesl`'s `INSIDE` clamp on the position it writes went with them — eight
thousandths of a tile of error in the ray's origin on exactly a flight's outer
corner, which is where every stair defect is found.

*Three of the plan's deletions did not happen, and each was settled by injecting
the fault rather than by reading the code.*

- **`own_run` stays**, as `same_run` with its height gate folded in. Identity
  cannot answer it: a run of wall is *N different statics* cut on tile
  boundaries, so the panel next along the run is a different solid however
  exactly a fragment names its own. Neutralised, `light_runs_along_a_wall_and_
  stops_across_it` and `the_two_faces_of_a_corner_are_lit_from_the_side_each_
  looks_at` go red. Restricting it to *neighbouring* cells — leaving the
  fragment's own cell to identity, which reads like the tidier rule — turns the
  same two red. What retires it is the grid merging a run of coplanar panels into
  one solid: `lighting_geometry.md`'s question, not this phase's.
- **`on_surface` stays** as that gate's own test, and is exact now: its `ON_TOP`
  tolerance was the nudge handed back, and both went together.
- **`mounted_at` and `MOUNTED_CLEARANCE` stay.** "A sconce burns where it is"
  means, in practice, a flame at its tile's *centre* — behind the plane of the
  face it is bolted to, where the cosine is zero along the whole face, so every
  wall carrying one would come out black top to bottom. It is not a compensation
  for a missing rule but the client's reading of where a wall-mounted static
  hangs, which the map does not say. Neutralised, `a_sconce_lights_the_street_
  and_not_the_room_behind_it` and the wall-run test go red. What would retire it
  honestly is the *art*: the sprite shows the sconce standing out from the wall,
  and nothing measures that.

*What the phase's own text meant by "`mounted_at`'s height test" is `flame_end`*,
and that **is** deleted: `skip_last && cell == last && on_surface(to_z, …)`
excused a panel on the cell a flame *ends* in. `mounted_at` moving the flame onto
the next tile is what made it unnecessary — neutralised, the suite stayed green
and the oracle stayed at zero on every flame height. `skip_last`, both walks'
`last`, `ExemptionContext` and `Exemption` went with it. What it covered and
nothing now does: a flame standing inside a whole-tile body, a lantern in a
tree's box — which is a wrong box rather than a rule the walk owes it.

*And the identity compare itself was fault-injected*, because nothing else would
have said whether it is load-bearing. Forced to `false`, three tests go red: the
flight fixture, `the_face_of_a_wall_is_lit_from_inside_the_room` and
`a_carried_light_lights_the_way_it_is_pointed`. The last two are also the only
place the `None` half of it is measured — a flat fragment's own solid is a lid,
and `crosses`'s strictness already answers a ray leaving a plane exactly; a face
fragment's own solid is a panel, and `same_run` masks its own cell's side
whatever the fragment carries.

*Three world claims were re-taken, and the rule from *How this is judged* held —
each was a judgement about the scene.* Two were the same graze: **a flame exactly
level with a tread**, whose riser stops at exactly the tread's height, so the ray
runs along the riser's top edge and a flame of real depth is half cut by it —
`0.5`, exactly, where the nudge had made it `1.0`. Both flames are `FLAME_LIFT`
above the tread now, which is where a torch burns. The third is **the floor
line**: a wall pixel at exactly a storey's floor height, which now names the wall
it is a point of instead of leaning on two constants to be lifted above the
boards. Above the line it is dark a sixteenth of a `z` up; *at* the line it is a
graze, recorded as a range rather than dropped — one mathematical plane, not the
four screen pixels the original defect was.

*What it cost, measured:* 88 pixels of a tread's outer corner read shadowed where
the face oracle's point-source geometry says lit — the same coplanar-edge graze,
at the line where a tread's lid meets its riser's plane. Both walks agree about
them; it is the engine's area light against a point source, and phase 5 is where
those become comparable. Against 473 "rendered too light" before the phase.

**That last sentence was wrong, and phase 5 is what measured it.** Making the
oracle an area light left all 88 exactly where they were. What they are is a ray
touching the riser's box at `t = 0` — the fragment stands *in* that box's own top
plane, so no interval separates them and identity cannot excuse a different
primitive. Phase 5's own account has the rule that closed them; the number is `0`
of 11,469 now.

**Phase 5 — area lights.** N rays to a sphere. `FLAME_DEPTH`, `pierces` and
`crosses`'s softening are deleted.
*Done when:* the penumbra matches the path tracer's within sampling noise, and the
noise is measured rather than asserted away. **Done.** The gate is
`the_frame_and_the_path_tracer_agree_about_every_interior_pixel`'s second half:
**11,896 pixels partly lit on both sides, the frame's penumbra `+0.0070` of a
flame from the reference's on average against the `0.025` a model difference would
have to clear, and `0.0676` of mean absolute difference against the `0.0995` half
a ray of eight plus the reference's own measured noise allows.** The noise is the
reference rendered twice under two seeds: worst `0.3125`, mean `0.0547`.

*The four constants were one apparatus, and the size in them was not a size.*
`FLAME_SPREAD` was `1.0` of a tile, `SOFT_CROSSING_MIN`/`MAX` bounded the
`t / (1 - t)` ratio it multiplied, and `FLAME_DEPTH` converted the width that
produced into a height because every edge softened vertically is horizontal. That
is the textbook penumbra formula with the source's own size in it — and the size
was a tile because a tile drew an edge somebody liked, which made the flame a
pancake: a tile across and a quarter of a tile tall. `FLAME_RADIUS` is **an eighth
of a tile**, and it is the one measurement in the pile that was ever taken from
the art — `FLAME_DEPTH`'s own, a torch's drawn flame at eight or ten screen pixels
and four pixels to a `z`, which is exactly twice this as a diameter. `pierces`,
`inside`, `crosses`'s band and the `spread` parameter every walk threaded went
with them; `hole` is a rectangle and `pierced` is what is left of a panel after
one.

*What the pictures cost, and it is the whole visible change:* **shadows are about
eight times crisper.** On `torch_before_a_wall`, the band up the wall's top edge
went from about eight `z` to one, measured by the same sweep that asserts it —
`a_ray_grazing_the_top_of_a_wall_is_dimmed_rather_than_switched` steps an eighth
of a `z` now to have anything to look at. That is not a regression: an eighth of a
tile is what the flame is, and the old width was the number that made the picture
rather than the number in it.

*The sampling, and the one place two backends had to be made to agree.* Eight rays
at a Vogel spiral on the disc the sphere presents to the fragment — the
silhouette, which is what `pathtrace::Emitter::Sphere` samples too, `sqrt` of the
index for equal area, laid out in tile space and multiplied back into `z`. Only
visibility is sampled; the falloff and the cosine stay at the flame's centre,
where at an eighth of a tile they move by well under a byte. The pattern is
rotated per fragment, or eight rays are eight visible bands — and the rotation is
**an integer hash of the world position quantised to a hundred-and-twenty-eighth
of a tile**, because the obvious thing (a hash of the pixel) cannot be spelled on
the CPU side at all and the usual `fract(sin(dot(…)))` is the arithmetic two
backends are least likely to agree about. Being stable in the *world* is worth
having on its own: a panning camera does not make a penumbra crawl.

What it costs, and the rotation is what buys it: **a sweep is monotone only to
within one ray.** Two neighbouring points of a sweep are two different rotations of
the same eight directions, so `a_ray_grazing_the_top_of_a_wall_…` allows
`1 / SHADOW_RAYS` of slack and says so as the ray count rather than as a number.

*The cost, measured on Britain at 4:1 by holding the frame still and moving the
ray count alone* — `tests/cost.rs`, seven flames, two million pixels:

| | one ray | eight rays |
|---|---|---|
| `night` (the seven flames) | 1.276 ms | 2.818 ms |
| above the `dark` floor | 0.255 ms | 1.819 ms |
| `sun` (still one ray, and unmoved) | 1.344 ms | 1.347 ms |

Seven times the work for eight times the rays, and 2.8 ms of a 16.7 ms frame. The
sun's row is the control: it casts one ray either way and does not move, which is
what says the measurement is about the rays and not about the day's weather.

*Three oracles were asked about a point and had to be asked about the sphere.*
None of them is the walk, and each was reporting the difference between a point
source and a body as the renderer's:

- **the brute-force fuzz** (`tests/lighting.rs`) shrank to a case where the centre
  ray clips a wall tile's far corner and most of the eight miss it. It asks
  `light::flame_points` where the rays end now — the *scene* shared, not the
  answer; its own dumb 0.001-tile stepper is untouched. It then found a corner
  clip under a thousandth of a tile deep on its first run, which is the second
  time that step has been defeated by a fixture, so it is five times tighter
  again and the file still runs in a second.
- **`boxes.rs`'s box-top and face oracles**, and **`synthetic_stair.rs`'s**: a
  share of the flame rather than a bit. The stair's *light* reference multiplies
  by that share instead of gating on it, which is what the engine does.

*And the 88 pixels phase 4 left on the table are gone — for a reason that phase
named wrongly.* Phase 4 recorded 88 pixels of a tread's outer corner drawn
shadowed where the face oracle said lit, and wrote that they were "the engine's
area light against a point source, and phase 5 is where those become comparable."
They are not. Making the oracle an area light left all 88 exactly where they were,
which is what says the cause is not the light's body — and the report named it
outright: a fragment at `(100.01, 101.00, z 1.0)` stopped by *a panel spanning
`z 0.00..1.00` on its own cell*, which is the riser directly under the tread's
lip. A riser is a plane on the climb axis and a tread's lid stops exactly at it,
so a fragment on that lip stands in the riser's own plane at exactly the riser's
top, and every ray it sends touches that box at `t = 0` and nowhere else. Identity
cannot excuse it: the riser is genuinely a different primitive.

What closes it is one line in each walk and in the shader, and it is `crosses`'s
own strictness said about a box instead of a plane:

```
if entered == 0.0 && leaves == 0.0 { continue; }
```

**A ray that only touches a solid at the point it starts from has not gone through
it.** No epsilon — both ends are exact numbers off the slab test, and the rule is
narrow by construction: a ray that starts *inside* a box leaves it at some `t > 0`
and a lid the ray genuinely crosses is found at the `t` of its own plane, so
neither is touched. The face oracle reads `0 of 11,469` after it, against 88
before, and the light oracle stays at zero on every flame height.

**Phase 6 — the impostor.** Sprite silhouette, analytic prism for depth and
normal, one draw. `WIDTH_OVERLAP` is deleted.
*Done when:* the difference frame's "drawn by one side only" classes are zero
except for rasteriser fill-rule dashes, against today's 1370.

*Three things this paragraph does not settle, written down before the phase
starts rather than discovered inside it:*

- **Where the prism comes from.** A fragment needs its static's own boxes, which
  means a range into a storage buffer per instance. `statics.wesl`'s header said
  it kept "inside WebGL2's ceiling" and that was a stale objection — the crate's
  ceiling is WebGPU (`lib.rs`, `docs/lighting.md` decision 30.5) and `blit.wesl`
  beside it reads eleven storage buffers. The comment is corrected; the design
  is allowed.
- **The view ray is a constant and the phase should say so.** From
  `camera::project_exact`, holding a screen point fixed gives `dx = dy` and
  `22·(dx + dy) = 4·dz`, so `dz = 11·dx` — and at `Z_PER_TILE = 11` the
  direction in the isotropic metric is exactly **`(1, 1, 1)`**. So there is no
  per-fragment unprojection to write: it is a slab test against a constant
  direction. Writing an inverse projection here would be the sixth spelling of
  one, which is already a backlog entry of its own.
- **The miss case needs its own measurement.** "A pixel whose ray misses the
  prism takes the nearest point on it" lives on a pixel or two of silhouette and
  no picture gate will ever fail on it. The phase's "done when" should carry a
  second number: how many fragments took the nearest-point path, and how far off
  the prism they were.

*Phase 6a landed: the arithmetic, on its own.* `crate::impostor` is `ray_from`,
`meets` and `nearest` with thirteen tests and no pipeline change. The three
points above are settled by it in order — the ray is `VIEW`, one constant
direction; the miss is `Meeting::outside`, how far in **tiles** a fragment fell
outside its own volume, a number rather than a branch; and the boxes come from
the instance, below. Two decisions the geometry does not force are written at
their own definitions: ties between two exit faces go to `z`, then `y`, then
`x`, so a lid reads as a lid to its own rim; and `TANGENT` is `1e-4` of a tile
against a *measured* `3.5e-6` of `f32` rounding at a corner.

*And the scope widened, deliberately: the impostor is for **every** static, not
only a fitted climbable.* A wall, a floor and a body are boxes too, so the same
meeting answers them — which retires `statics.wesl`'s whole inverse projection
(the stance switch, `INSIDE`, the corner branch, the height recovered from
`pixel_y`) rather than leaving it beside the new path for the majority of
statics. Three backlog entries go with it: the `INSIDE` clamp that still sits a
hundred-and-twenty-seventh of a tile behind every east and south face,
`own_solid`'s ambiguity on a fitted climbable, and `parity_frame`'s fixture
naming an owner where the shader compares a solid.

**A fragment meets only its own static's boxes, and that is what makes the seam
disappear by construction rather than by a border.** Today a mesh face is a
polygon in a shared buffer, rasterised among everyone else's, and which object a
pixel belongs to is settled afterwards by the depth test — so where two
silhouettes disagree there is a pixel belonging to neither, and nothing but
growing a shape can cover it. Under the impostor a pixel is *already* one
instance's, because that instance's own quad drew it, and the boxes it is met
against are that instance's alone. A neighbour's geometry is not reachable. So
"the silhouettes disagree" stops being an event between two objects and becomes
a property of one: `Meeting::outside`, which is a number to fix the geometry by.
See `docs/style.md`'s *No fudge constants* — this phase is where that rule was
written down, and `WIDTH_OVERLAP` is the case it was written from.

*What the mesh pass is, which this phase settles rather than deletes.* It writes
no colour and takes its depth from the sprite beneath it: it is a **layer over**
a static, not a pass that draws one. With the impostor no real static needs that
layer. What still does is the four hand-built scenes — `examples/boxes.rs`,
`two_cubes.rs`, `tests/traced.rs`, `examples/synthetic_stair.rs` — which have no
sprites at all and exist to watch rays travel through geometry. So the pass
stays as **the hand-built-geometry layer**, off every real static, and gains a
colour target: without one a box in `boxes.rs` has no albedo and the comparison
against the path tracer there still runs on the invented `Albedos::body`. That
is phase 2's leftover closed at the diagnostic layer, where it actually lives —
not, as this document first had it, as a side effect of deleting a pass.

*And the grid's own climbable geometry was wrong for this, so it was fixed rather
than worked around.* A fitted climbable's occluders were **surfaces, not a
volume**: a lid per tread (degenerate in `z`) and a riser per tread (degenerate
on the climb axis), whose union encloses nothing. For a flight climbing *away*
from the camera that happens to cover the visible side; for one climbing
*towards* it every riser faces away and is hidden behind its own tread, so the
grid held no vertical surface at all where the art draws the whole front of the
staircase.

The impostor spent one commit meeting a volume rebuilt from the `Prism` and
joining back to the grid by `occlusion::Part` — which is a second statement of
one shape, and therefore the thing this phase's own rule forbids. **`Builder::
add` pushes one body a tread now**, its strip from the static's base to its own
height, and `push_volumes` is the grid's boxes copied. The split's reason is on
the record in `gbuffer_archive.md` step 4b — "the representation the render pass
(step 4c) needs to walk" — and both halves of it are retired: phase 2 gave the
normal a plane of its own, and this phase takes the mesh pass off every real
static.

Three things came with it. `WIDTH_OVERLAP` and `widen_footprint` are **deleted**,
since there is no second silhouette left for a border to reach across. The
**vertical-ray shortcut** in both walks and in `blit.wesl` now looks at bodies as
well as lids — it skipped everything with an `edges`, which was already a gap for
every body in the world (a ray straight up out of a tree's box left it
unstopped), and treads-as-bodies is what exposed it. And one world claim was
**retired by the geometry rather than re-taken**: a fragment on a tread's top
used to be shadowed by "the riser that tread stands against", which is a surface
of the tread's own body — a surface cannot shadow a point of itself, and that
assertion was measuring the split. What replaced it is a fragment on the flight's
front shadowed by the tread above, and a fragment on the bottom tread shadowed by
the two climbing away from it.

*And the staircase's own oracle followed, which is what a gate on a scene's
geometry is built to make happen.* `examples/synthetic_stair.rs` panicked on its
first run after the reshaping — "the grid holds 3 solids and this oracle derived
6 planes" — because its whole check was a plane-for-solid pairing, and the two
lists stopped having the same length. What it states now is **both** shapes and
the join between them: a `Body` a tread, derived from the profile and held
against the grid's own solid corner for corner; a `Slab` a drawn face, held
against the mesh as before; and each face held against the body it is a face of,
so the surface list and the volume list cannot say different staircases. Two
things the panic pulled up with it. The example rebuilds `push_mesh`'s loop by
hand and so still asked for `Part::nth(part)` when the real pipeline had already
divided — six faces against three bodies, a second failure waiting one line
below the first. And the fragment's exemption changed *kind*: what the oracle
drops is the primitive the engine drops, `lit.solid`, which is now the tread's
whole body rather than the one plane the pixel sits on. Dropping nothing at all
was tried first — a body has an inside, so "the ray leaves at `t == 0`" excuses
a fragment's own surface with no name needed — and the sweep priced it at nine
pixels across the seven flame heights, which is the engine's own rule showing up
as a measurement. The rule was never an epsilon; phase 4 wrote it as one
sentence because for a *plane* the name and the tangency are one sentence.

*Phase 6c landed: the pass meets its own boxes, and the inverse projection is
gone.* `statics.wesl` reads a storage buffer of `impostor::Volume`s, takes the
run of it its own instance names, and answers a fragment's **position and
normal** with the face of the box its view ray leaves. What that replaced is
five branches: a floor's two fractions from the pixel's offset from the tile's
centre, a wall's one fraction pinned to the edge its stance names and the other
run along it, and a height recovered from `pixel_y` at four screen pixels a `z`.
`INSIDE` went with them — the hundred-and-twenty-seventh of a tile every one of
those was clamped by — and `place_format.wesl`'s `outward` went with the
*normal*, which is the whole of what that table was for.

**The wrong table it was, and the impostor cannot spell it.** `outward` answered
`(0, −1, 0)` for a north face, the side turned *away* from the viewer, where
what a camera sees of a north wall is its `+y` surface — this document's own
backlog, five graphics out of 1197, which is why nothing had caught it. A ray
from the camera leaves a box through `+x`, `+y` or `+z` and through nothing
else, so there is no row left to be wrong. `crate::place::Stance::normal` keeps
the table with its defect written down beside it: its readers are the hand-built
G-buffers of `plan.rs` and the fixtures, which state a scene by naming a stance
and have no boxes to meet.

*The gate is `a_sprite_pixel_meets_the_same_box_on_both_sides`, and it is the
only thing comparing the two spellings.* `impostor.wesl` and `impostor.rs` are
one arithmetic written twice with no compiler between them —
`normal_format.wesl`'s situation one plane down — so the test renders a sprite
over three boxes and asserts, for **every one of 4,620 fragments**, that the
normal word the GPU wrote equals `gbuffer::pack_normal` of what this side
answers and that the point agrees to `1e-4` of a tile. It reports what it
reached rather than leaving that to a reader: 638 east faces, 913 south faces,
3,069 lids, and 2,684 fragments answered by the nearest-point fallback. Two
fault injections turn it red — the `+y` normal flipped, and `far` shifted by two
thousandths — and one deliberately does *not*: the tie between two exit faces is
a **line** across a box rather than an area, so a sweep of whole pixels reaches
it only by luck, and `impostor.rs`'s own lid test is where that case is
constructed instead.

**And it found that the grid was the wrong place to ask what shape a static is.**
`push_volumes` read the occlusion grid, and `Builder::add` answers two questions
at once: what shape is this, and does it stop light. It refuses outright
everything the tiledata does not mark `NO_SHOOT` or `WINDOW` — so on one real
place at radius 6, **nineteen of thirty-nine drawn pictures stood as no box at
all, twelve of them south-facing walls**. Read through the grid, every one of
those became a billboard: the middle of its tile, no facing, lit from every
side. That is a *worse* answer than the stance it replaced, and it would have
undone `docs/lighting.md`'s decision 27 — a lamp beside a wall lighting its cap
as fully as one standing over it — for every wall cap in the world.

So the two questions came apart. `occlusion::boxes_of` is what shape a static
standing at a place *is*, one function with two readers: `Builder::add` is now
the opacity gate, the owner, the roof bit and the hole's placement wrapped
around it, and `push_volumes` is the same shapes joined to the grid by `Part`
for their `SolidId` — or `NOBODY` where the grid refused them, which is the
honest name for a shape no shadow ray will meet. **A pane of glass has a shape
whether or not it casts a shadow.** The census reads `0 of 39` now, and
`examples/isolated_scene.rs` prints it every run.

Two things it changed that are not the pass. `blit.wesl`'s ambient takes the sky
share from **the tile the instance carries** rather than `floor(position)`: a
south or east face's fragment now lies exactly on its tile's boundary, where
flooring reads the neighbour, so a street wall would have taken the room's
ambient along one of its two sides and not the other. The walk beside it already
took the carried tile, and says why at its own `first`. And
`StaticGeometry::absorb` is one place for joining the map's furniture to the
server's dropped items, because **three of the four lists are addressed by
index** — which turned up a live defect the phase did not go looking for: the
mesh rows were concatenated without shifting the vertices that name them, so a
climbable *item* drew its faces against whichever of the map's rows its own
numbering landed on. It needs a climbable item to show, which is why nothing had.

Two world claims were re-taken, and both were the same claim: **a face lies on
its own edge, exactly.** They asserted `120/127` of a tile — one step of the
retired clamp short of it — and the reason was that `blit.wgsl` found a cell by
flooring a position. Neither half survives: the walk takes the cell from the
tile the instance carries, and what a fragment is exempt from has been primitive
identity since phase 4. So the number is the plane the geometry states, asserted
to the float.

*What is left of phase 6, and it is not small.* The mesh pass still runs over
every fitted climbable (6d is where it comes off real statics and gains a colour
target, which is phase 2's albedo). The corner's two panels are still told apart
by the **screen half** rather than by the box the ray met — the impostor picks
between them for the normal, but the id has to follow `split_corners`' twin row
and a box carries no row number. `own_solid` still scans a cell to name a
sprite's solid, where the box the fragment met already carries one. And the
phase's own second number — how far a *real* static's art overhangs its own
fitted prism — is still unmeasured: the gate's fixture is a plain rectangle
nobody fitted to anything, so its overhang is a property of the fixture.

*A person has now looked at a lit frame of it*, which is what *How this is
judged* says the instrument is. `examples/isolated_scene.rs` at Britain's
`(1497, 1626, 10)`, radius 6, one lamp post added by hand so the scene has a
flame in it at all, read in `View::Lit`, `View::Light`, `View::Normal` and
`View::Shadow`. Three things it says. **The census holds on a scene 6c never
ran** — `0 of 340` drawn pictures stand as no box, against the `19 of 39` the
grid answered with before the split. **Nothing reads as a seam**: no border, no
pixel of a silhouette belonging to neither side, which is the phase's own claim
by construction and is now a claim somebody has checked with their eyes.
`View::Normal` over the same place is what it was — every fragment carries a
facing, a wall reads as a green face, a red end cap and a blue top. And it
found one defect, which is its own backlog entry below: **a flame's own sprite
is black.**

**Phase 7 — billboards.** Normals for mobiles, chosen by looking at both.
*Done when:* a person standing beside a torch reads as lit from the torch's side,
in a frame a human being has looked at.

**Phase 8 — the sun.** A direction, the same BRDF, the same rays, sky visibility
as ambient occlusion.

## Accepted costs

- **The picture changes, and nothing in the renderer compensates.** Pre-shaded art
  multiplied by our light is double-contrast: a face already darkened by the
  artist and turned away from a flame goes darker than UO ever showed it. Exposure
  and ambient are ordinary exposure and ordinary ambient and they are all there
  is — neither is tuned *against* the art. If a scene still reads wrong, the
  answers are content (a shard ships better art) or de-lighting as a project of
  its own, and never a term in the BRDF.
- **Statics without a good prism** get a rougher volume, and their impostor normal
  is an approximation of an approximation. Visible on the odd tree and fence.
- **Cost.** Eight shadow rays a light a pixel is more work than one, and the
  lighting pass is already the expensive one. The phase that adds them measures
  it; if it does not fit, the answer is fewer rays plus temporal accumulation, not
  a return to an analytic fudge.

## Open questions

Written down rather than guessed at:

1. **How much does exposure have to give back?** Double contrast is a global
   effect and a global exposure may absorb most of it. **Still open, and it now
   has a picture under it rather than a guess.** Phase 3's frames say the loss is
   not global at all: open ground barely moves and a *grazed vertical face* moves
   a great deal, which is the case a global exposure is worst at absorbing. The
   experiment is still one evening; it is no longer inside phase 3, because
   nothing in phase 3 is what a knob would be turned against.
2. ~~**Do statics need per-face albedo?**~~ **Closed by the decree.** A prism's
   four sides sample the same sprite through one projection, so a wall's two
   visible faces carry the art's own two shadings and we multiply both. Flattening
   them per face would be de-lighting through the back door, and the answer is the
   same as to de-lighting itself: not in this renderer. Whatever the sprite says
   is albedo.
3. ~~**Does the ground want normals at all?**~~ **Answered by having them, phase
   3.** UO's terrain is a height field with per-corner heights, so it has real
   normals, and `ground.wesl` writes the bilinear patch's own. It was as close to
   free as the question hoped: the one-torch-on-open-ground pool is barely changed
   from the half-space's, because on level land the normal is `(0, 0, 1)` and a
   flame above it is nearly overhead. What the normal buys is the *slope*, which
   had no lighting at all before and now catches a flame the way the hill it is
   faces it.

## The plans this consolidates

Seven documents describe how the current lighting was built, and a session that
starts by reading them starts by reading five thousand lines to find out which
paragraphs are still true. **This is the entry point now.** Each of them stays as
the record of how something was built and why — nothing is deleted, and the
reasoning in them is worth more than the code it justified — but the *live work*
is here, in one list.

| document | what it is | what happens to it |
|---|---|---|
| [`lighting.md`](lighting.md) | the current system, end to end: place attachment, occlusion grid, ray walk, sun, beams, doors, art measurement | **the thing being replaced.** Its mechanisms are retired phase by phase; its *content* work (below) survives untouched |
| [`lighting_world.md`](lighting_world.md) | ambient, the sky field, the day curve, tonal response | **mostly survives.** The sky field is ambient occlusion by another name and phase 8 adopts it; the day curve and the tonal response become phase 1's and phase 8's business |
| [`lighting_raymarch.md`](lighting_raymarch.md) | the DDA walk, CPU/GPU parity, the tile-boundary hazard | **survives as the walk.** Phase 4 changes what a hit *means* (identity, no bias), not how cells are stepped. Its corner-tie parity gap outlives the rebuild |
| [`lighting_geometry.md`](lighting_geometry.md) | box → mesh occluders, never started | **cheaper after phase 4**, which makes primitives addressable by id, and **started at phase 6**: a tread is one body rather than two degenerate surfaces, which is the first time the grid's own shape was chosen for what a *view* ray needs as well as a shadow ray. `facing::Blocks` — an authored list of up to four boxes, written and wired to nothing — is where the generic form continues |
| [`lighting_height.md`](lighting_height.md) | the height track: four landed phases and a long backlog | **the backlog is mostly deleted rather than fixed** — see the mapping below |
| [`lighting_reference.md`](lighting_reference.md) | the path tracer, a third opinion with no shared arithmetic | **becomes phase 0**, the oracle everything else is judged by |
| [`gbuffer.md`](gbuffer.md) | the `place` attachment's format, ids, per-face mesh geometry | **phase 2 replaced the format** and inherited every one of its readers. Its open question — how to encode a normal for a non-axis-aligned face — is answered there: an octahedral pair packed as integers into an `R32Uint`, with two bits over for the two answers that are not directions. (`Rg16Snorm`, which this document first named, is not a format wgpu will render to under WebGPU's core set; the plane spent one phase as three floats before it was packed) |
| [`world_coordinates.md`](world_coordinates.md) | a position should carry its own cell; one metric | **half of it is phase 2** (positions as data, `z` in tiles once). The CPU-side type stays its own track |

### What each phase deletes from `lighting_height.md`'s backlog

So that backlog can be read as "work" rather than as a list of things that may or
may not still matter:

| backlog entry | fate |
|---|---|
| ~~`FACE_EDGE`'s two scales; the flame at a surface's own height~~ | **done, phase 3** — there is no band, and a flame in a surface's own plane is a cosine of zero rather than a half |
| `STAND_OFF`/`ON_TOP` at a grazing corner; the `ON_TOP` twin | **done, phase 4** — there is no nudge |
| risers excused as a group; `flame_end`'s height test; a mobile shadowed by its own wall | **done, phase 4** — identity answers all three |
| `own_run` | **survives phase 4, measured** — a run of wall is N statics, which no identity merges. `lighting_geometry.md`'s, when a run becomes one solid |
| the `ground < 1e-6` shortcut ignoring a lid's footprint | **fixed** — it was worth fixing alone, and was |
| `WIDTH_OVERLAP`'s border | **done, phase 6** — there is no second silhouette for a border to reach across |
| the riser penumbra graded over a third of a face | **done, phase 5** — there is no band; a penumbra is eight rays disagreeing |
| the wire's span rounding to nearest; the exact-tangent definition | **phase 4** — a primitive is not a byte range any more |
| `boxes.rs` reading `Unreached` as shadowed; `two_cubes.rs`'s old idiom; the projection idiom stated five times; `mesh::Face`/`facing::Face` colliding | **survive** — instrument work, still worth doing. One of the five spellings of the projection went at phase 6c: `statics.wesl`'s inverse of it is `impostor::ray_from` now, which is a forward ray rather than an unprojection |
| `Occlusion::owner_at`'s linear scan; `selected`/`outlined` stamping `OwnerId::NONE` | **survive**, reshaped by phase 4's ids |
| `tests/cost.rs` measuring three planes of five; `plan::Wall::top` as an `i32`; hand-copies of the third channel | **survive** — the third channel's copies went with the channel, and the other two are still work |

### Wanted after the model works, and deliberately not before

Asked for while this document was being settled, and parked on purpose: each of
them is a *second* answer to "what does a lit frame look like", and a second
answer is only readable once the first one produces a picture worth comparing
against. None of them is a reason to soften a phase above.

- **UO's own light, as a mode you can pick.** The reference client draws light by
  blending sprites from `light.mul`, keyed by `lightidx.mul` and by a light id in
  the tiledata entry — a source's *shape* is a picture, not a radius, which is
  where a window's light patch on the floor comes from. Neither file is read by
  this client at all; `light::flame` is a stand-in of one warm default and a
  wider campfire, and it is the only invention left in the pass. Reading them is
  worth doing on its own — it replaces that function and nothing above it — and
  on top of that, a *native* mode that blends the sprites the way the client does
  instead of shading with ours belongs beside the deferred pipeline as a switch,
  not as a fork. See `lighting_archive.md`'s account of the reference client's
  arrangement, and `docs/client.md`'s own backlog line.
- **The stylised end, revisited as an experiment.** The dial between a half-space
  and Lambert is deleted from the plan, and the alternatives it came from are
  recorded in `lighting_archive.md`. Once phases 3–6 give frames a person is
  happy with, trying a stylised BRDF against them is a comparison with a baseline,
  which is the only form in which it is worth anything. Not a knob shipped
  half-tuned in the meantime.
- **The circle of transparency** — a radius around the body inside which walls go
  translucent. It is not a lighting feature at all: it is the fifth item of the
  blended pass `docs/client.md`'s "What is still M3" describes, recorded here only
  because it was asked for in the same breath and belongs written down somewhere.

### Carried over: work no phase here deletes

Gathered from every document above, because these are the things that would
otherwise be lost between plans. None of it blocks the rebuild; all of it is
still wanted.

**Content and features**
- The day curve — until it lands, a default frame carries no ambient split at
  all and a house reads as bright as the street (`lighting_world.md`).
- Light carried by mobiles other than the local player; a serial-derived flicker
  phase (`lighting_world.md`).
- The screen-space glow for a flame's own halo, and the sunbeam shaft through a
  window (`lighting.md`).
- Doors, the ported open/shut occlusion table — built, and untouched by any of
  this.
- Land as an occluder: a hill casts no shadow today (`lighting.md`).
- Leaded/lattice window apertures, refused rather than measured; the aperture
  channel of the field is reserved and always zero (`lighting.md`,
  `lighting_world.md`).
- `Builder::add` consuming an authored `Blocks` list — the table format supports
  arches and lintels, nothing wires one into the live grid (`lighting.md`).
- Night Sight's interaction with a real day curve is undecided
  (`lighting_world.md`).
- A mobile as a soft sub-tile occluder; a body's diagonal footprint the
  axis-aligned `Solid` cannot state (`lighting.md`, `lighting_world.md`).

**Known gaps that outlive the rebuild**
- The corner-tie CPU/GPU parity gap, with two `#[ignore]`d tests
  (`lighting_raymarch.md`). Phase 4 does not touch stepping, so it stays.
- Nothing runs the tracer over a real map — all four scenes are hand-built boxes,
  and the fifth is hand-built flat ground (`lighting_reference.md`). The
  brightness calibration beside this entry **is done** (phase 0); a real map is
  not, and is now the whole of what is left of it.
- The tracer is single-threaded, 13 s a frame — too slow for a sweep, and a
  sweep is how the last three defects were found (`lighting_reference.md`).
- Buffer capacity is one flat `INITIAL_QUADS = 4096` for all kinds, and the
  widest real frame reallocates on its first frame, every run (`gbuffer.md`).
- A climbable the prism-fit cannot decompose still occludes as a whole-tile
  body (`gbuffer.md`).
- A courtyard overhang can make the sky-column test misread a tile; 28 of 2,560
  outdoor tiles in Britain read dark (`lighting_world.md`).

## Backlog

Things noticed while writing this, not blocking any phase:

- 🚩 **An emitter is black in its own light, and every free-standing one taller
  than `FLAME_LIFT` is.** Found by looking at a lit frame after phase 6c — the
  one instrument *How this is judged* names — and reproduced at one item and
  nothing else: `OPENSHARD_SCENE_RADIUS=0`, no ground, no statics, one lamp post
  by hand, `0 standing cells`, and the lamp lit by its own flame is a black
  silhouette with a green wick. The chain is three facts that were each right
  on their own. `light::burns` answers only for statics light gets *through*
  (`opacity == CLEAR`), so **an emitter is by definition not in the occlusion
  grid**. Phase 6c gives a shape to exactly those too — "a pane of glass has a
  shape whether or not it casts a shadow" — so a lamp post now has a volume.
  And `light::place` burns at the tile's own centre a `FLAME_LIFT` up, which is
  **inside** that volume: the impostor answers each of the sprite's own
  fragments with the camera-facing plane of its own box, whose normal points
  away from the flame, so `N · L ≤ 0` on every visible face. `View::Shadow`
  reads those pixels *visible*, which is what says it is the cosine and not the
  walk. `mounted_at` rescues the sconce alone — it moves a flame
  `MOUNTED_CLEARANCE` clear of a *panel's* plane, and a panel is another
  static's edge in the same cell, which a lamp standing in the open has none
  of. A campfire is unhurt because its box stops below half a tile and the
  flame clears its lid. **It arrived with 6c rather than being uncovered by
  it**: before the impostor a lamp post was `Stance::Upright`, whose normal is
  the zero vector, and the zero vector is the one value `blit.wesl` skips the
  cosine for. Three candidate answers, none of them measured yet: place the
  flame where the *art* draws it rather than at the tile's centre (the honest
  one, and the same unmeasured sprite reading `MOUNTED_CLEARANCE` wants);
  give an emitter no volume, which trades this for the billboard 6c retired;
  or say that a surface containing its own light source has no cosine, which is
  an exemption and therefore the shape this document exists to refuse. **Phase
  7's billboard question is this question one object over**, and the two should
  be answered by the same reading of a sprite.
- **A wall a lamp stands against is barely lit, on a real place now.** Open
  question 1 had phase 3's synthetic frames under it; the lit frame above is the
  same shape at Britain: the plaster wall the lamp post is bolted beside takes
  almost nothing from it while the cobbles under it carry a full pool, because a
  flame half a tile out from a plane grazes it. Nothing here is a defect — it is
  the accepted cost, seen at last on art somebody drew — but it is the picture
  the exposure-and-ambient experiment should be judged against, and it is a
  better scene for that than any fixture in the tree.
- **The CPU's `Surface` is four fixed normals and land now has a fifth kind.**
  `light::sample`'s `Surface::Flat` looks straight up, which is exactly right for
  level land and wrong for a hillside — `ground.wesl` writes the bilinear patch's
  own normal per fragment and the CPU side cannot state one. It is not a
  regression: before phase 3 the two disagreed about *every* ground pixel, because
  the GPU wrote a zero there and the CPU wrote `(0, 0, 1)`. It is a new, smaller
  disagreement with a name, and what closes it is a `Surface` that can carry a
  measured vector rather than choose between four.
- **The reference tracer samples its own disc at random, and could stratify.**
  That single fact is why phase 5's penumbra gate is three aggregate statistics
  rather than a per-pixel one: at sixty-four samples the reference disagrees with
  *itself* by a third of a flame at the middle of a soft edge (measured — worst
  `0.3125`), so a per-pixel comparison there is a gate on the ruler. Stratified
  over its own sample index the error would be `O(1/N)` instead of `O(1/√N)` and
  sixty-four samples would be sharper than the engine's eight rays by an order of
  magnitude, at no extra cost — which would make the per-pixel claim available and
  would sharpen `penumbra`'s `over` count from a diagnostic into a gate. It needs
  `pathtrace::Emitter::sample` to know which of `settings.samples` it is being
  asked for, which is a signature and three of that crate's own tests.
- **Nothing on the GPU side tests the shader's own identity compare.** Forced to
  `false`, `tests/frame.rs` stays green from end to end while three tests in
  `light.rs` and `tests/lighting.rs` go red — so the rule the *shipped* walk uses
  is covered only through its CPU twin, which the phase's own commits also
  rewrote. What the one frame test in that shape reaches instead is `crosses`'s
  strictness: its fragment is flat and its own solid is a lid.
- **`parity_frame`'s `Fixture` names an owner, and the shader compares a solid.**
  Every pixel it writes is a sprite, so `own_solid` narrows that owner by the
  pixel's stance — and for a *flight* that narrowing is ambiguous by construction,
  three lids and one flat stance. `the_shader_does_not_stop_a_vertical_ray_with_a_
  lid_it_is_not_under` passes because the grid's reference order happens to put
  the bottom tread first, which is written down at the field and nowhere else. The
  honest fix is for the fixture to write a **mesh** row, which is what the real
  pipeline draws a flight through and what can carry a `SolidId` outright; it is
  a third row table in a function that already has two, which is why it was not
  done in the phase that found it.
- ~~**`statics.wesl` still clamps a face fragment to `INSIDE`.**~~ **Done, phase
  6c** — there is no clamp and no inverse projection to clamp: a face fragment is
  the point where its own view ray leaves its own panel's box, which is that
  panel's camera-facing plane exactly. The asymmetry the entry worried about is
  gone in the direction it did not expect — a *south* or *east* face lies on the
  tile boundary and a north or west one a `PANEL_THICKNESS` inside it, because
  that is where the slab's visible side is. See the next entry.
- **A north or west panel's visible face is a fifth of a tile inside its own
  tile, and that is the geometry rather than a bug.** `Solid::box_of` fattens a
  panel *inward* from the plane the art draws it on, so the camera-facing side of
  a north panel is at `y + PANEL_THICKNESS` while a south panel's is at `y + 1`.
  Phase 6c answers the picture with the volume, so a north wall's fragments moved
  a fifth of a tile into the room. Nothing compensates and nothing should — but
  the *geometry* has a better shape available: two neighbouring walls on a shared
  edge are two 0.2 slabs meeting, a doubled wall, where one 0.2 slab straddling
  the boundary would be one wall and would put both faces on the plane the art
  draws. `PANEL_THICKNESS`'s own doc argues the inward choice from the doubling
  it was avoiding, which the straddling form avoids better. Worth measuring
  before it is changed: it moves every panel in the grid.
- **Three scans a drawn static now, where there was one.** `statics::collect`
  asks `Occlusion::owner_at` for the quad, `Occlusion::id_of` per mesh face and —
  since phase 6c — `id_of` again per *box*, all linear scans of the cell; see
  `owner_at`'s own note about the join this design pays for. A four-tread flight
  scans its cell thirteen times. Nothing measures it as a cost yet, and
  `tests/cost.rs` cannot: it builds its frame against `Occlusion::EMPTY`, so no
  static there has a box to meet and the statics pass it times is the billboard
  fallback. **Both halves of that are work** — the scans, and a cost harness that
  prices the pass the client actually runs.
- **A corner is told apart by the screen half, and the boxes could say it.**
  `statics.wesl` still resolves a corner's two panels by `across > 0.0` — the
  half of the picture a pixel was drawn on — where the impostor already meets
  both boxes and picks between them for the *normal*. What stops the box from
  deciding is the **id**: the left half takes `in.twin`, the row
  `split_corners` appended, and a `Volume` carries a `SolidId` and no row
  number. Two answers to one question, and today they can disagree — the box in
  front and the screen half are not the same test near the tile's own corner,
  where the two slabs overlap. What would close it is the volume carrying which
  *instance row* it belongs to, which is one word in a struct that has a spare
  one.
- **`own_solid` scans a cell to name a solid the fragment already met.**
  `blit.wesl` narrows a sprite instance's owner by the fragment's stance, once
  per fragment, walking the cell's list — and `impostor::Volume::solid` is the
  answer outright, carried by the very box the ray met and written on the wire
  since phase 6b. What is missing is a way to get it from the pass that knows it
  to the pass that asks: the id word is full (kind, stance, twenty-six bits of
  row) and a `SolidId` will not fit beside them. It is also the *exact* answer
  where `own_solid` is a guess — a fitted climbable's treads are one owner and
  one shape, which that function's own doc calls the case it cannot answer.
- **A run of wall wants to be one solid, and until it is, `same_run` stands in.**
  Phase 4 measured that identity cannot retire it — the panel next along a run is
  a different static — which makes "merge coplanar panels of a run into one solid"
  the thing that *would*, and moves it from a tidiness idea to a named
  prerequisite. `lighting_geometry.md`'s question, with a reason attached now.
- **A sconce's own art says how far it stands out from its wall, and nothing reads
  it.** `MOUNTED_CLEARANCE` is `0.7` of a tile because half a tile reaches the
  plane and a fifth clears it; the sprite shows the real overhang and
  `crate::facing` already measures silhouettes for a living. That is what retires
  the constant honestly, and phase 4 found that deleting it without a replacement
  blacks out every wall carrying one.
- **A slope's normal now nudges its own shadow ray sideways.** `walk`'s `ahead`
  spends the normal's `x` and `y` on `STAND_OFF`, and until phase 3 a ground
  fragment's was zero on both. A hillside's is not, so a slope's ray starts a
  fiftieth of a tile out along the hill. That is more nearly right than not
  nudging at all — it is the direction out of the surface — but it is a behaviour
  nobody asked for arriving through a constant phase 4 deletes. **Closed at
  phase 4**: there is no `STAND_OFF` and no nudge of any kind, so a slope's ray
  starts where the slope is.
- **Two scenes moved because a flame stood in a surface's own plane, and the
  shape of that is worth keeping.** `z: 0.0` in a hand-built `Light` read as "a
  fire on the ground" for as long as the shading term was a half-space, which
  gave such a flame the band's own half. Under a cosine it gives nothing, and the
  tests said so at once. **Every hand-built `Light` in the tree should be asked
  whether it means a tile's `z` or `FLAME_LIFT` above it**; two were found by
  failing, and a scene that merely goes dim would not have said anything.
- **The origin-touch rule is stated three times and tested through none of them
  directly.** `if entered == 0.0 && leaves == 0.0 { continue; }` lives in both
  walks and in `blit.wesl`, and what says it is right is a *tool's* count going
  from 88 to 0 — `synthetic_stair`'s face oracle, which nothing runs under `cargo
  test`. The claim it makes is small enough to state as a unit test of
  `walk_cells_*` on a two-solid fixture (a lid whose edge is another solid's own
  plane, a ray leaving that edge), and that is what would catch it being deleted.
- ~~**A north or west face's normal contradicts the argument `outward` itself
  makes for it.**~~ **Done, phase 6c.** The impostor names the face the ray met
  and a ray from the camera can only meet `+x`, `+y` or `+z`, so there is no row
  left to be wrong; `place_format.wesl`'s `outward` is **deleted**, nothing else
  read it. `crate::place::Stance::normal` keeps the same table on the Rust side
  with the defect written down at its definition — its readers are hand-built
  G-buffers stating a scene by naming a stance, which is a question about the
  edge rather than about the picture.
- **Two pixels at flame height `1` survive the light oracle, and both sit where
  the reshaped tread put them.** `[tread 2's riser] at (100.80, 100.33, z 3.10)`
  — the engine reads it fully blocked and the geometry gives it `0.022`, a tenth
  of a tile above the top of the body that blocks it; and `[tread 1's riser] at
  (100.97, 100.67, z 1.02)` — both sides agree the flame is fully visible
  (`through 255/255`) and they differ by `0.017` in what it is lit *to*, four
  parts in 255. Neither is a visibility disagreement, so the face oracle is
  silent on both. What they share is a flame level with a tread's own top
  (`z 1` is tread 0's height), which is exactly the case
  `segment_clear_of_box`'s own doc calls out: every ray from that height runs
  *along* the plane of every surface at it. Worth one measurement each before
  phase 6d moves these fragments anyway — the mesh pass comes off real statics
  there, and their positions change with it.
- **`synthetic_stair.rs` rebuilds `statics::push_mesh`'s loop by hand**, and that
  is why it still asked the grid for `Part::nth(part)` a commit after the real
  pipeline started asking for `Part::nth(part / 2)`. It cannot call the real one
  — `push_mesh` is `pub(crate)` and an example is an external consumer — so the
  duplication is structural rather than lazy, and the join between a drawn face
  and the solid it names is now written in two places that have already
  disagreed once. The same shape as the seventh hand-built flight below.
- **The three-tread flight is now rebuilt by hand in a seventh place.**
  `statics::tests::flight` joined the five in `light.rs` and the one in
  `frame.rs`, and it is the same `Prism::new(Face::North, &[1, 3, 5])` again. The
  backlog entry below asking for one constructor is a line longer every time the
  scene is used, which is the argument for it.
- **A flame's size is a constant and belongs on the `Light`.** `FLAME_RADIUS` is
  one number for a candle, a torch and a campfire, and `Flame` already carries the
  reach, the colour, the intensity and the flicker — a size is the field that is
  missing, and a campfire is visibly wider than a candle. What stops it today is
  the uniform: `Light` on the GPU is three `vec4`s with no spare lane, so a fourth
  is 1 KB more at 64 lights. Worth doing when something else needs that lane.
- **`boxes.rs` now builds two mirrors of one scene** — the same `Mirrored` twice,
  differing only in the `LAMBERT_PI` on the flame's intensity — because the
  visibility comparison is in `Brdf::Flat` and the shaded strip in
  `Brdf::Lambert`. Phase 4 retires the first, and the second mirror should go with
  it rather than become a habit.
- ~~**The normal plane is sixteen bytes a fragment and needs four.**~~ **Done —
  see phase 2's own account.** An octahedral pair in an `R32Uint`, integers on
  both sides, and the two spare bits carry "nothing drew this" and "no facing"
  rather than the id word doing it. `ATTACHMENT_BYTES_PER_SAMPLE` is 32.
- `docs/lighting_height.md`'s backlog does not disappear — most of its entries
  are *deleted* by a phase here rather than fixed, and each should be marked with
  which phase kills it rather than left reading as work.
- ~~The `ground < 1e-6` shortcut (both walks and the shader) is a real defect
  today and becomes moot at phase 4; if phase 4 slips, it is worth fixing
  alone.~~ **Fixed.** All three copies gate on the lid's own footprint now, by
  the horizontal half of `ray_vs_solid`'s parallel-axis rule — `light::
  over_footprint` and `blit.wesl`'s twin. Only the horizontal half, because a
  vertical ray's height answer is `crosses`'s soft one and `ray_vs_solid` would
  answer it hard, erasing the penumbra.
- ~~**There is no lit-against-lit picture, and three separate things stop one
  being drawn.**~~ **Done — this is phase 0, and its account is up there.**
  `<base>_lit_vs_traced.png` is the engine's shaded frame, the tracer's, and the
  difference amplified `8×`; `boxes.rs`'s `flat` scene is where it means
  something. All four blockers went: the albedos come from the frame, the flame
  is the engine's own, the encodes share `tonemap::encode`, and the ambient is
  nothing on both sides. The fourth — a mesh face has no albedo — is not fixed
  but *avoided*, by a scene with no boxes in it, and it is still phase 6's.
- **A body's albedo is still invented, and one scene is not a calibration.**
  What phase 0 now proves is that the engine and the reference agree about *one
  surface, flat, unoccluded, unhued*. Three things it says nothing about: a
  vertical face (no albedo on the engine's side until phase 6), a hued sprite
  (the ramp is decoded to linear before the light multiplies it, and nothing
  compares that against anything), and land that is not flat (`ground_albedo`
  panics on a textured floor rather than handling one — deliberately, because a
  single-albedo reference cannot judge one). Each is a scene the tracer could
  hold once the engine's side has a colour to compare.
- **A scene's flame reaching the whole canvas hid a conflation in two oracles for
  as long as every scene had one.** Fixed — see phase 0's account — but the shape
  of it is worth keeping: the oracles were right about every pixel they compared
  and wrong about *which pixels they had an opinion on*, and no amount of looking
  at their disagreement counts would have shown it, because the count was the
  thing that was wrong. What found it was a scene whose flame does not cover the
  frame. **Every detector in this crate that reads a `View::Shadow` pixel should
  be asked the same question**, and the two here are unlikely to be all of them.
- `examples/two_cubes.rs` still projects world points without asking whose pixel
  it got. Phase 2 moves every other reader to `ids`; this one should go with them.
- **`tests/traced.rs` and `examples/boxes.rs` still build the same scene twice.**
  The two gates inside `traced.rs` now share one `render(Shot)` fixture — which
  is what made the brightness gate cheap to add — but the tool has its own copy
  of the whole pipeline (floor art, synthetic map, atlases, mesh rows, blit), and
  a scene is authored in one and restated in the other. `line_scene` and
  `flat`'s flame are already two spellings of numbers that have to agree for a
  failure in the gate to be reproducible by the tool. The same argument as the
  three-tread flight below, one layer up.
- **The parity harness could not see a sub-tile lid, and still barely can.** The
  shader's copy of the shortcut above was fixed and forty-seven frame tests
  stayed green with the fix deleted again: no parity scene had a solid narrower
  than its tile, so the branch was never run. It has one now, and `Fixture` can
  state an *owner* — without which a fragment on a tread is shadowed by the step
  it stands on and every finer question about a flight is unreachable. What is
  still true is that this is one scene and one pixel of it: the vertical case
  needs the flame exactly over a swept fragment, so one flame buys one comparison.
  A sweep that varied the flame across the tile would buy the whole strip.
- ~~**Parity is circular for any defect both walks share.**~~ **Acted on.** It
  compared the shader against `light::sample`, so a rule wrong in the same way on
  both sides reported agreement — and the whole family is now deleted, see *How
  this is judged*. What is left of that test is its *direct* half:
  `the_shader_does_not_stop_a_vertical_ray_with_a_lid_it_is_not_under` reads two
  of the frame's own pixels and no longer calls a sweep at all. It is the direct
  claim that fires when the shader's gate is removed, and it was always the only
  half that could.
- ~~**The parity apparatus was built on `place`'s packing, which is why it could
  not have survived phase 2 anyway.**~~ **Done.** `parity_frame` and `plan.rs`'s
  `drawn` both go through `gbuffer::Fragment` for all three planes now, and
  neither spells a layout. The one thing that changed shape rather than moving:
  an **id is not a fact a fragment knows** — a world pass has one per instance
  from the rasteriser, and a fixture's is a row number it can only hand out once
  it has seen every fragment it means to draw. So both harnesses gather their
  fragments whole, key a row per distinct tile, and only then pack. `Fragment`
  carries the tile and `Fragment::ids` takes the id, which is that asymmetry
  stated in the type.
- The three-tread flight is rebuilt by hand in five tests in `light.rs` and now a
  sixth in `frame.rs`, each restating the same `Prism::new(Face::North, &[1, 3,
  5])` and the same tile bounds. It is the scene every stair defect is found on
  and it should be one constructor.
- ~~`renderer.rs`'s `depth_state()` has lost its doc comment: `PLACE_TARGET` was
  inserted between the comment and the function.~~ **Fixed.** The constant moved
  below the function it had been spliced into, and both have their own doc again.
- ~~Hand-copies of the third channel.~~ **Fixed, and then the channel went.**
  `gbuffer::Fragment` is what a G-buffer texel *is* — tile, sub, `z`, kind,
  stance — and `ids()`/`position()`/`normal()` are the only three spellings of
  the layout outside the shaders. `plan.rs`'s two closures and `frame.rs`'s
  `parity_frame` went through it; they had three copies of the fraction's
  `<< 2`/`<< 9` between them, and the id plane deleted the fraction outright.
