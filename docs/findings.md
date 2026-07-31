# Findings

Things about the client, the reference emulators and the client's data files that
cost somebody a day to learn. Every entry here is here because the code was wrong
first: the rule is in [`../CLAUDE.md`](../CLAUDE.md) in one line, and this file is
the argument behind it.

None of it is architecture. For that, [`architecture.md`](architecture.md).

## Reference sources

Two other emulators. Neither is vendored, neither is a dependency, and neither is
copied — they are read. Where your checkouts of them are is your own business:
put the paths in `CLAUDE.local.md`, which is gitignored beside this file.

**SphereServer**, if a checkout is available: `Source-X/` (the C++ engine) and
`Scripts-X/` (the .scp scriptpack). Read it for **observed protocol behaviour**,
which is two decades of finding out which client breaks on what and is genuinely
hard-won. `Source-X/src/common/sphereproto.h` is the single most valuable file in
it.

**ServUO** (C#, on GitHub), for a second opinion on the same problems. Where the
two agree about the client, that is as close to a specification as this genre has.

Do **not** read either for architecture. Copying their structure is the one thing
this project exists to avoid — and where they agree about *engine* design, that
is often the strongest available argument for doing it differently. Both stop the
world to save it; `crates/server/persistence/src/journal.rs` explains at length
why this one does not.

## Reading the reference emulators

**Take Sphere's numbers; audit its arithmetic.** The `MINCLIVER_*` table, the
step vectors, the Huffman key, the 200ms walk interval — all hard-won, all worth
copying verbatim. But its walk speed check compares a duration against a count
and does not survive being read closely, so `WalkPace` is a token bucket instead.
Copying something that does not add up is worse than not copying it.

**Read Sphere's shifts, never Sphere's comments.** Its IP comments say the
opposite of what its code does — `send.cpp` calls the branch emitting
`C0 A8 0B 06` "in reverse", because it reverses the *dword*, and the dword is an
`s_addr` that is already network order. Both readings are articulate and one is
wrong. Trace the bytes for a concrete address, in C if need be; the answer takes
a minute and the alternative shipped a shard nobody could log into.

**A tiledata flag means what the reference *reads* it for, not what its comment
says.** Sphere's header calls `UFLAG2_WINDOW` "window/arch/door can walk thru it",
and Sphere never once consults it in `CWorldMap`: the only three uses in the whole
engine are line-of-sight tests in `CCharLOS.cpp`, gated on `LOS_NB_WINDOWS`.
Honouring the comment in the *movement* check let anything the server moved walk
out through every wall segment with a window in it. It never showed for a player,
because the client refuses the step before it is ever sent — which is the general
trap: a server-side movement hole is invisible from the only end normally tested,
and surfaces as NPCs strolling through walls. (`NO_SHOOT` was mis-valued at `0x20`
in the same file, which is `UFLAG1_DAMAGE`; there is no `UFLAG1_NOSHOOT` at all.
Pin a flag's value in a test next to the constant.)

## The client, as observed

**The game connection never says what the client is.** The version arrives in
the seed sent to the *login* server; the second connection opens with four bytes
of auth key and a `0x91`, and carries no version at all. A game session that only
knows its own socket defaults to the oldest dialect and sends a 1997 character
list to a modern client, which reads past the end of it and desynchronises —
surfacing as a garbage packet id hundreds of bytes later, looking nothing like a
version problem. The auth key is the only thing linking the two connections, so
anything that has to cross the gap rides on it. Sphere stashes it on the account
instead, which races when two clients share one.

**The 0x8C relay and the 0xA8 shard list carry the address in opposite orders.**
Relay: octets in order, always, no version gate. Shard list: reversed from 4.0.0
on, in order below it. Both SphereServer and ServUO agree exactly, which is as
close to a specification as this genre gets. A change that makes the two packets
consistent has broken one of them. And the relay is the expensive one: get it
wrong and the server sees a clean login and a clean disconnect, because the
client dialled a well-formed address that was not this machine and never came
back — the failure happens where this end cannot see it.

**Never trust a length off the wire.** Check it against the buffer before
reserving anything. `frame_client_packet` rejecting a claim above
`MAX_PACKET_SIZE` is what bounds gateway memory; nothing downstream re-checks.

**The server remembers what is on each client's screen.** There is no "what can
you see" packet — only "draw this" (`0x78`) and "forget that" (`0x1D`) — so the
only way to send a mobile exactly once is to know what was sent before. That is
what `World::seen` is. Skip it and every step redraws every neighbour, which
looks fine with two players and melts at two hundred.

**Distance in UO is Chebyshev, `max(|dx|, |dy|)`.** The client draws a *square*
region. A circle here leaves the corners of every screen empty, and the bug
looks like mobiles popping in and out at the edges.

**Every visible action plays a sound and an animation, not just a state change.**
A swing that lands, a spell that resolves, a door that opens, a potion that is
drunk, a mobile that dies — each one, to a real client, has a sound (`0x54`), a
mobile action animation (`0x6E`, or `0xE2` gated on
`Feature::NewMobileAnimation`) and often a graphical/particle effect
(`0x70`/`0xC0`/`0xC7`). Sphere and ServUO fire these on essentially every action,
and their action/`SpellInfo` tables already carry the ids. A state-only system
passes its test and feels dead in the client — a fireball with no bolt reads as
broken even when the damage is right. So when you build or review a gameplay
action, emit its feedback too: broadcast through the same `seen`/interest
machinery as `0x78`, encoder in `crates/common/protocol`, default in core and
overridable in the pack off the domain event — the split combat and magic already
use. This was a systemic miss for most of the project's life; do not add to it.

## The client's data files

**The map is in the `.uop`, not the `.mul`.** Modern clients ship both and the
`.mul` may be a stub full of zeroes. `Map::load_facet` prefers the UOP. See
`world::uop`.

**A zero pixel inside a land diamond is black, not transparent.** Statics carry
their transparency as `0x0000` pixels, and applying the same rule to the ground
is wrong: a land tile's shape is the diamond and nothing else, and real tiles
contain a handful of genuinely black pixels inside it — three to nine on a
typical one. `Image` cannot tell the two apart, because it stores the corners
outside the diamond as `Color16::TRANSPARENT` too, so the shape has to come from
`art::land_row` rather than from the colours. Reading it out of the colours
instead punches pinholes through the ground, which look exactly like dark
texture: the first renderer to do it covered 97.7% of a viewport instead of
100%, and the missing 2.3% was invisible on a screenshot.

**A land cell's `z` is the height of one corner, not of the tile.** It belongs to
the corner the tile shares with its neighbours to the north — the top of the
diamond on screen — and the other three corners are read from the cells at
`(x+1, y)`, `(x, y+1)` and `(x+1, y+1)`. The client stretches the tile over those
four points, so adjacent tiles are built from *the same* vertices and a gap
between them cannot occur. Drawing each tile as a flat 44×44 diamond at its own
`z` instead is not merely an approximation: neighbours pull apart along every
slope, and a screen of Britain loses 2.3% of its pixels to seams while the sea
still covers 100%, so a level-ground test says nothing about it.

**A sloped tile's texture comes from `texmaps.mul`, not from `art`.** The 44×44
land sprite is what the client draws when the four corners share a height; on a
slope it binds a square texture from `texmaps.mul` and maps it corner to corner,
because stretching the art diamond onto a steep quad smears it. Two shapes and
two texture sources, chosen per tile. Corner to corner is the identity — the
quad's top vertex takes the texture's top-left, the right vertex its top-right —
which is `_cornerOffsetX/Y` in ClassicUO's `DrawStretchedLand`.

**Which texture is not the land graphic.** It is a separate id in `tiledata`'s
land entry, two bytes between the flags and the name, in an index space of its
own. Nothing relates the two numbers, so reading that field at the wrong offset
still names *a* texture for every tile in the game: the ground comes out textured
with somebody else's terrain, which reads as a seasonal variant rather than as a
bug. The size is not stored either — 64 or 128 is decided by the entry's
*length*, and ClassicUO reads anything that is not `0x2000` as a 128.

**No texture means the client never stretches the tile at all.** `IsStretched` is
initialised to `TexID == 0 && IsWet` and then read as "do not", and
`ApplyStretch` gives up immediately when the texture entry is empty — so a tile
with no texture is drawn as a flat diamond however the ground around it stands,
seams and all, and water is never stretched. The decision is also made over a
wider neighbourhood than the tile's own four corners: it comes from the four
corner *normals*, each of which reads a cell beyond the corner.

**A quad's corner texture coordinates need a half-texel inset.** A region's edge
is the boundary *between* two texels, so a vertex at `u + du` samples the first
texel of whatever is packed next door in an atlas — a one-texel fringe of foreign
terrain along two edges of every stretched tile. ClassicUO insets by half a texel
in `CalculateHalfPixelUVs`, which makes the four corners sample the texture's own
first and last texel centres. This does not arise for a tile drawn 1:1 from its
own sprite, which is why it appears exactly when stretching starts.

**No client files are in this repository and none ever will be.** They are
copyrighted and they are not ours to redistribute. `world.client_files` points
at whatever install the operator already has; the tests that need one read
`OPENSHARD_CLIENT` and skip when it is unset. Do not commit a path to anyone's
machine, and do not name whose files you tested against — this crate reads a
*format*, not a particular shard's data.

## Traps in tests and benchmarks

**A benchmark where nothing moves measures nothing.** A player who does not walk is
drawn once and never redrawn, so a standing world never pays interest management —
no `refresh_around`, no first-sight draw, none of the per-draw work of assembling
what a neighbour is wearing. `examples/town_bench.rs` reports standing and walking
side by side because the gap between them was three orders of magnitude: 0.107 ms
against 8.9 ms for the same town. The same applies to what a benchmark *builds* —
its predecessor spawned every creature with `equipment: Vec::new()` and placed no
decoration, so it exercised neither of the two columns a real facet spends its tick
in.

**A statistical test needs a companion that says the data is real.** The map test
asserting "neighbouring tiles have similar heights, so the block order is right"
passed against a `map0.mul` that was 90MB of zeroes — all-zero terrain is
perfectly smooth however you index it. `terrain::tests::the_map_is_not_degenerate`
exists to stop that. Any test that measures a property of real data can pass
vacuously on absent data.
