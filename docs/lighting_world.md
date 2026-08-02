# Lighting, part two: the light a place has

A living plan, in the shape the others here have: the decisions numbered so one
can be argued with alone, the steps, and a backlog. It stands on
[`lighting.md`](lighting.md), which is where the pass moved out of the screen and
into the world and learned that a wall stops a flame. That is the hard half and
it is built. This is the other half, and its subject is not shadows — it is
**where the light in a frame comes from when nothing is burning**.

## The thing worth copying

Nox (Westwood, 2000) is the isometric game whose lighting still reads as right,
and it is worth being exact about *why*, because the obvious answer is wrong. The
obvious answer is "shadows": light stopped at walls, beams came out of doorways.
We already do that, per fragment, with a real grid traversal — strictly more than
Nox, which flood-filled a per-vertex lightmap once and interpolated it.

What Nox actually had that this client does not:

- **A room was darker than the street**, with nothing in the room and nothing on
  the street. Not because of a "dungeon flag" — because a roof is between the
  floor and the sky. Walking through a door was a change in light, and that one
  fact is most of the atmosphere.
- **Light moved.** Your torch, a fireball in flight, a burning corpse, a spell.
  The pool travelled with the thing, smoothly, and the world changed around it.
  Ours is nailed to map statics and to items lying on the ground.
- **A fire was the brightest thing in the frame.** Emitters were not subject to
  the darkness they were dispelling.
- **The picture had one tonal response.** Dark was blue and detailless, light was
  warm and blew out at the centre, and the two met on a curve. Ours multiplies
  the art by a colour, which can only ever darken, and clips whichever channel
  the ambient is poorest in.

None of these is a shadow. All four are about the light a place has before
anything happens in it — which is why this is a second plan and not a step in the
first one.

**Out of scope, deliberately.** Everything is computed on the client; the server
is not asked for a single new byte and `0x4F`/`0x4E` remain the whole protocol
surface (decision 3 says what that costs). The raggedness of a lit wall — the
per-pixel fraction along an upright sprite's face — is
[`lighting.md`](lighting.md)'s decision 13 and is being fixed elsewhere; nothing
here depends on it, and nothing here should touch `statics.wgsl`.

## Decisions

**1. A tile that cannot see the sky does not get the sky's light.**
The ambient is one colour for the whole frame today, so the inside of a house is
lit exactly as brightly as the street outside it, and a dungeon is dark only
because the server said the whole world was. Split the ambient in two:

```
ambient(tile) = SKYLIGHT * sky(tile) * daylight  +  GROUND_AMBIENT
```

`sky(tile)` in `0..=1` is how much of the sky that tile can see; `GROUND_AMBIENT`
is a small, cold floor that a windowless cellar still gets, so that a room with
no torch in it is deep rather than pure black — an unlit black rectangle is not
atmosphere, it is a bug report.

`sky` is the cheapest question the occlusion grid can be asked: a column test.
Anything opaque standing over the tile above its floor takes the sky away. The
grid already carries a `z` span and an opacity per tile, so a roof is an occluder
that happens to be overhead, and the answer is a byte.

**2. The sky term is blurred by a tile, and that blur is the doorway.**
A raw column test steps from 1 to 0 at the wall line, and a step is the artefact
this whole track exists to remove. A 3×3 average over the sky field — one pass,
on the CPU, over a grid that is already a few hundred tiles — makes the threshold
of an open door brighter than the middle of the room and the eave of a roof
brighter than under it. It is not a simulation of anything; it is the shape the
right answer has, for one blur of a small array.

**3. The cutaway takes a roof away from the eye, not from the sun.**
The cutaway removes the storeys the player is not on so that a house has an
inside. If the sky test read the *drawn* set of statics, standing indoors would
delete the roof and flood the room with noon — the player would carry daylight
into every building. So `sky` is computed from the map as it is, not as it is
drawn, and it is the one consumer of the occlusion walk that ignores
`cutaway::shows`.

This is a real inversion of [`lighting.md`](lighting.md)'s decision 4, whose whole
argument is that a static the cutaway removed must not cast a shadow — and the
inversion is right, for a reason worth stating: a *shadow* is a thing the player
would see falling from something that is not there, and a *missing ambient* is
the absence of light from a thing the player knows is there, because they walked
under it. One is an artefact, the other is the point.

**4. Day is a curve with a colour, not a level with a key.**
The server sends `0x4F` as a number from 0 to 31 in steps, and F10 is a switch
between two constants. Neither is a sunset. The client keeps its own
time-of-day scalar, driven by the server's level and **eased towards it over a
few seconds**, and maps it through a ramp that is a colour and not a brightness:
amber at dawn, white at noon, amber and lower at dusk, and at night the blue that
`light::NIGHT` already is. A step in the server's level then reads as the sun
moving rather than as somebody flipping a switch, and no new packet is needed to
get it.

The sun's direction ([`lighting.md`](lighting.md) decision 12) comes from the same
scalar: elevation and azimuth are the curve's other two outputs, so the shadows
on the street turn as the day passes and lengthen into the evening — for free,
because the machine that walks them is built.

**5. Anything that burns carries its light, and it carries it smoothly.**
`light::collect` walks map statics and ground items. It must also walk:

- **mobiles** — an equipped light source on a mobile's layer, which is how the
  reference does it (`GameScene.AddLight(this, item, ...)` from `MobileView`),
  and which is what finally makes a player holding a torch light the room;
- **effects** — a spell, a projectile, an explosion, each a light for as long as
  it draws;
- the player, whose torch replaces the "personal light level" fudge: `0x4E` is a
  floor under the darkness, so it brightens the whole screen including the far
  side of a wall. A real light on the player's own tile is the honest form of the
  same intent and gets shadows for nothing.

**Smoothly** is the part that is easy to get wrong. A mobile's *sprite* is
interpolated between tiles; if its light is placed at its tile, the pool jumps a
whole tile at a time while the thing carrying it slides. The light takes the same
interpolated world position the sprite is drawn from — which the renderer already
computes — and the flicker phase comes from the mobile's serial rather than from
its tile, or the pool changes its character as it walks.

**6. An emitter is not subject to the dark it dispels.**
A campfire at night is currently art multiplied by a night ambient, plus its own
light at distance zero. If that sum comes out below 1 the fire is a *dim* fire,
which no fire is. The rule: a fragment whose tile hosts an emitter is lit by at
least that emitter's own colour at full intensity — the multiplier is clamped
from below rather than accumulated to. It is one `max` in the loop, and it is
what makes a torch look like a torch instead of like an orange sprite.

**7. Falloff is a shape that reaches zero, and the light set fades at its edge.**
Two pops to remove, both structural:

- A falloff that is still bright at the radius switches off at the rim. The
  window `(1 - (d/r)^2)^2` — smooth, exactly zero at `r`, inverse-square-ish in
  the middle — has no rim.
- `Lighting::MAX` is 64 and `collect` truncates by distance from the eye, so the
  65th torch appears and vanishes as the camera moves. The last few in the sorted
  list fade out over the tail rather than being cut, so a light leaves by getting
  dimmer.

**8. The frame is composed in linear light and mapped once, at the end.**
Multiplying the art by a colour can only darken it, so there is no such thing as
a bright pool — only a less dark one — and the channel the ambient is poorest in
clips first, which is why a blue night makes warm art go grey before it goes
dark. What is wanted is the ordinary photographic answer: accumulate in linear,
then one tonal curve with a shoulder, so a flame's centre rolls off warm instead
of clipping, plus a *toned* lift in the shadows (the ambient's own colour, not
grey), plus a triangular dither before the 8-bit write — a large smooth pool on a
dark floor is exactly the picture 8-bit banding is visible in.

**The trap, said out loud:** the client's art is already lit. Every tile and
sprite has baked highlights implying a fixed sun, so real coloured light on top of
it is a double count, and the more saturated the ambient the more obviously
wrong it looks. Nox's art was drawn for Nox's lighting; ours was not. The
practical consequence is that the curve's job is as much restraint as reach, and
that any value here is held by a scene rather than by a formula.

**9. A body between a flame and a wall makes a shadow.**
Mobiles are not in the occlusion grid, so a crowd around a campfire is a crowd of
things standing in a light that goes straight through them. A mobile is a short,
soft occluder — a partial opacity over a span of about a body's height — and the
grid takes it the same way it takes a static. The reference does not do this;
that is not an argument against it, it is the reason it needs its own scene and
its own value.

**10. Sight is not light, and this client cannot enforce either.**
Nox's other famous half is that you see only what your character sees. It could
do that because it was authoritative. Here the server has already sent everything
in range, so any "fog of war" drawn on the client is a curtain over data the
player's own memory holds — cosmetic, cheatable, and dishonest if presented as
anything else. It is worth having as an *option*, because dimming what is behind
a wall looks superb and costs one more use of the walk that is already there, and
it is worth never letting it decide anything. If it should ever be a rule, the
rule lives on the server and this pass is not where it starts.

**11. What each of these is held by is a scene, not a number.**
`render/src/scene.rs` is the pattern: a built map, a built tiledata, a list of
items, a camera, and an ASCII diagram a failing test prints. Every decision above
that invents a constant — `GROUND_AMBIENT`, the day ramp, the body's opacity, the
tonal curve's shoulder — gets one, and the constant is tuned against the picture
rather than argued into existence. The existing list of invented values
(`occlusion::PANE`, `light::flame`, `FLAME_SPREAD`) is already the longest
section of the other plan's backlog; this one should not lengthen it silently.

**12. Nothing here lands ahead of the measurement.**
[`lighting.md`](lighting.md)'s step 6 — a frame time at the widest zoom — is
still open, and three decisions here add per-fragment or per-frame cost (1 and 2
a grid pass, 5 more lights, 8 a curve on every pixel). The number comes first,
and each step states what it cost.

## Steps

- [ ] **1. The sky field.** `occlusion.rs` gains a per-tile sky byte from the
      un-cut column test of decision 1/3, and the blur of decision 2. Uploaded as
      an `R8Unorm` over the same bounds as the occlusion texture. Unit-tested
      without client files on `scene`'s room: floor tiles under a roof read 0,
      the doorway reads between, the street reads 1.
- [ ] **2. The two ambients.** `Lighting::ambient` splits into a sky colour and a
      ground colour; `blit.wgsl` reads the sky field and mixes. `light::sample`
      gains the same term in the same commit — the parity test of the other
      plan's decision 9 is what keeps the two honest, and it fails loudly if only
      one side learns this.
- [ ] **3. The day curve.** A `Daylight` in `light.rs`: the server's `0x4F` level
      in, an eased scalar, and out of it the ambient pair *and* the sun's
      direction. F10 becomes an override of the scalar rather than a swap of two
      constants, so the debug key and the real path are one code path.
- [ ] **4. Emitters that move.** `collect` takes mobiles and effects; a light
      takes an interpolated position and a serial-derived flicker phase. The
      player's torch lights the player's room; `0x4E`'s floor comes out.
- [ ] **5. Emissive emitters.** Decision 6's clamp, in the shader and in
      `sample`, with a night scene whose only subject is that the fire is the
      brightest thing in it.
- [ ] **6. Falloff and the fade at the cut.** Decision 7, both halves, plus a
      test that walks a camera past a 65th light and asserts no discontinuity.
- [ ] **7. The tonal response.** Decision 8: linear accumulation, a shoulder, a
      toned shadow lift, dither. This is the step most likely to be argued about
      and the one most obviously judged by a screenshot — a before/after pair of
      the same scene belongs in the commit.
- [ ] **8. Bodies as occluders.** Decision 9, behind its own scene.
- [ ] **9. The optional curtain.** Decision 10, off by default, and documented as
      cosmetic where a reader will see it.

## Backlog

Written while drafting this, and not to be lost:

- **The sky field and the sun are asking one question twice.** Decision 1's
  column test is "can this tile see straight up"; `walk_sun` is "can this tile
  see the sun's direction". At noon they are the same walk with a different
  vector, and a shared traversal would answer both — which is also what the other
  plan's backlog wants for the sun's tile-at-a-time stepping. Worth doing when
  both are built, not before: two callers is when the shape of the shared thing
  is visible.
- **A roof over a courtyard is a lie the map tells.** Some UO houses have tiles
  that are roofed in the art but whose statics do not stand over the floor tile —
  overhangs are drawn on the tile *next* to the one they cover, because a static
  is a picture rising from its own diamond. Decision 1 will read those as sky.
  Whether it matters is a question for a real house, and the scene that answers
  it does not exist yet.
- **`Lighting::MAX` at 64 is a guess that nobody has hit.** Britain at the widest
  zoom with every window burning (the other plan's backlog: 80 window graphics
  carry `LIGHT_SOURCE`) is the case that finds out. The truncation is only worth
  fading (step 6) if it happens; the measurement of step 12 will say.
- **The personal light level has a second meaning.** `0x4E` is also how a shard
  says "this player has night sight" — a spell, an item. Replacing it with a
  torch on the player's tile (decision 5) is right for the torch and wrong for
  night sight, which is not a light at all but a change to how dark the dark is
  *for one viewer*. Both want to exist: a source, and a floor under the ambient.
- **Nothing in this plan knows about weather.** An overcast sky is exactly the
  sky term of decision 1 multiplied by a number, and rain is the same with a
  colour — which is to say this arrives almost for free once the sky field is a
  field, and it is worth not designing it away in the meantime.
