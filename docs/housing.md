# A house, and the ground it stands on

Placement, the walls that stop you, the door that knows you, the decay that
takes it away — **one plan, because they are one object**. A house is not a
feature made of parts that could ship separately: a house you can place and walk
through is not a house, and a house with a lock and no decay is a shard that
fills up and never empties.

The engine has none of it. `crates/server/housing/src/lib.rs` is four lines
saying so.

> Read [`architecture.md`](architecture.md) for where a system crate sits and
> what it may depend on, and [`style.md`](style.md) before writing any of it.
> This document does not restate either.

## What a multi is, and what is already read

A **multi** is one item that draws as many. The wire carries a house as an
ordinary world item whose graphic is `0x4000 + id`; the client looks that id up
in its own `multi.mul` or `MultiCollection.uop` and draws the hundred and
forty-eight statics a villa is made of. **The shard sends none of them.**

That is the whole reason this is tractable. The picture is free — every client
already owns every house. What the shard owes is everything the picture does not
say: where the walls are for the purpose of stopping somebody, who may open the
door, what happens when nobody pays the upkeep.

[`openshard_uofiles::multi`](../crates/common/uofiles/src/multi.rs) reads the
components and is **built**. Three things about that format are written down in
[`findings.md`](findings.md) and are not worth re-deriving: the High Seas widening
and the arithmetic that detects it, the drawn/skip flag that runs *opposite ways*
in the two files, and the fact that the two files disagree about how many multis
exist (326 against 862 on one install, so the UOP wins).

## What is missing, in one table

| piece | server | client (ours) | classic client |
|---|---|---|---|
| multi components | **built** (`uofiles::multi`) | — | reads its own files |
| `0x99` multi target cursor | **no packet at all** | **no packet at all** | speaks it |
| a house as a world item | nothing places one | draws items already | draws multis already |
| the footprint blocking a step | `Obstructions` exists, nothing fills it from a multi | — | n/a, server-authoritative |
| the house sign, the deed | — | — | ordinary items |
| door locks | `KeyValue` and the lock rules exist (`.key`) | — | n/a |
| co-owners, friends, bans | — | — | n/a |
| decay | — | — | n/a |
| customisation (`0xD7` house design) | — | — | speaks it |

`0x99` is the one packet that has to be written from nothing on both ends. It is
ServUO's `MultiTargetReq`: 26 bytes classic, 30 post-High-Seas — a target
request with the multi id and an offset appended, so the client draws the house
under the cursor while the player picks a spot. The reply is an ordinary `0x6C`,
which this engine already reads and our client already sends.

## Decisions, taken here

**D1 — a house is an entity with a `Multi` component, not a new kind of thing.**
It has a `Position`, a `Drawn` whose graphic is `0x4000 | id`, and a serial from
the item pool. Everything that already walks items — the sector index, the save,
the `0x1A`/`0xF3` that draws it — works on it unchanged. What makes it a house is
a `House` component beside those, not a separate table.

**D2 — the footprint is an obstruction, computed at placement and stored.**
`FacetState::obstructions` is the index a step already asks. A house adds its
drawn components to it, each at its own `dz` with its tiledata height, exactly as
a static would be. Not computed per step from `multi.mul`: a step is ten a second
and a house does not move.

The consequence to accept up front: **a house is not in the map file**, so the
obstruction index is no longer purely a function of the client's files. It is the
files plus what the shard has placed. That is already true of doors and dropped
items; a house is the first thing that adds a *hundred* entries at once.

**D2a — `openshard-state` never holds the multi table.** It does not depend on
`openshard-uofiles` and must not start: the components are resolved **at
placement**, by a caller that has the table, and what is stored is the
obstruction entries and the `House` component. At boot the saved houses are
restored by the boot code, which already reads the client's files. This is D2's
"computed at placement and stored" spelled as a dependency rule, and it is what
keeps a *client file* out of the crate every gameplay system builds on.

**D2b — one entity blocks one tile at several heights.** `Obstructions::block`
keyed an obstacle by its entity, because the case it was written for was a door:
one thing, one height, re-registered to refine it. A house is one entity whose
walls stand on top of each other, so the key is the entity **and the z** — done,
with a test, before anything above it was written. Keyed by the entity alone the
second registration overwrote the first, which does not read as a missing wall
but as the wrong floor being sealed, since which one survived depended on the
order the components came out of the file in.

**D3 — the placement rules are ServUO's five, and the fifth is the one to get
right.** From `HousePlacement.Check`: nothing impassable around the outside, no
impassable tile touching the house, five tiles clear front and back, the
foundation rests flat, and **no foundation tile over a road**. The road rule is
the one a player notices the absence of, because without it houses appear across
Britain's streets.

Staff place anywhere, which is ServUO's own first branch and is what makes the
rules testable before the deed exists.

**D4 — a region, and it comes free.** `Regions` already exists and already
carries flags. A house is a region with its own flags (no teleport in, no
recall out), placed and removed with the house. Nothing new is needed for this,
which is why it is a decision rather than a phase.

**D5 — the door is the door this engine already has.** `.key` and `KeyValue` and
the lock rules landed with the traps work, on the argument that Britannia locks
exactly one container and the rules would otherwise be unreachable. A house door
is that mechanism with the house's own key, and the reason it is D5 rather than
a phase is that there is nothing to build — only to connect.

**D6 — decay is a tick count, not a wall clock.** Everything in this engine that
measures duration counts ticks, because a tick count replays and a clock does
not. ServUO's five days becomes a tick count in `Gameplay`, an operator setting
like every other duration. A house refreshed by its owner walking in resets it.

**D7 — customisation is out of scope, by name.** ServUO's `HouseFoundation` and
the `0xD7` design packets are a second system the size of this one: a design
buffer, a preview state, a commit, and a whole editor on the client. A *classic*
house — placed from a deed, fixed shape — is the whole of this plan. The
foundation ids (`0x13EC`–`0x1D00`) are named here only so that the placement code
refuses them loudly rather than placing a house with no stairs.

**D8 — the moving crate is not deferred, it is the deletion rule.** When a house
goes, what was inside it has to go somewhere, and "somewhere" being the ground is
how a shard loses a player's belongings. ServUO's moving crate is a container the
contents land in. It is small and it belongs to the phase that can destroy a
house, not to a later one.

## The phases

Five, in the order they are worth having. Each leaves the shard strictly better
and none depends on a later one's shape.

---

### H1 — a house on the ground, placed by staff

**What a player sees:** a house, where a game master put it, and walls that stop
them.

1. A `House` component in `openshard-state`, and the multi table loaded at boot
   beside the map and the tiledata — in the *boot* code, not in `state`. See D2a.
   The obstruction key is already widened; see D2b.
2. `openshard-housing`: `place(state, at, facet, multi_id, owner)` — spawn the
   item, fold the components into `Obstructions`, refuse a footprint that does
   not fit.
3. D3's five rules, staff-exempt.
4. `.house <multi id>` — a staff command, `.add`'s shape.
5. Saved: a `HouseRecord` (serial, multi id, position, facet, owner) and the
   obstruction rebuilt at boot from it rather than saved. The components are a
   pure function of the id, so saving them would be saving a copy of the client's
   file.

**Done when:** `.house 0x64` puts a villa on the ground, both clients draw it,
walking into a wall is refused and walking through the door is not, and it is
still there after a restart.

#### Built

Items 1–4 are in. `openshard-housing` is the crate, `.house <multi id>` is the
command, and the footprint is folded into `Obstructions` at placement. Two things
came out differently from the plan and one is still open:

- **The multi table hangs off `Terrain`, not off world state.** D2a said the
  components are resolved by "a caller that has the table"; the code found a
  better answer, which is that `Terrain` is *already* the seam client-file facts
  reach gameplay through — `item_blocks` and `item_height` are how a placed
  static learns whether it stops anybody and how tall it is, and a multi's shape
  is the same kind of fact. So `Terrain::multi_components` is a new method with
  an empty default, `MapTerrain` carries an `Arc<Multis>`, and `openshard-housing`
  depends on neither `uofiles` nor a table on `WorldState`. It also answers the
  "what if the shard has no client files" question for free, the way every other
  method on that trait already does.
- **Only the floor question is decided at all.** A component is folded into the
  footprint when its tiledata says it blocks, which keeps a floor and a roof
  walkable — a house whose floor blocked would be sealed shut from the inside.
  That is the *component* half.
- **D3's rules are in, and two of the five turned out to be one question.**
  ServUO's rule two (nothing impassable in contact) and rule four (the foundation
  rests on a surface) are both "is there an open gap with a floor here", which is
  exactly what `Terrain::can_fit` already answers against the map's own statics —
  so they are one call and one refusal, `BadGround`. Rule five is the road, a
  land-tile id against ServUO's nine ranges. Rule three is the yard, and it is
  measured **wall to wall against the other house's own footprint** rather than
  against a stored rectangle: a footprint is what a house *is*, and a rectangle
  would be a second copy of it to keep in step.

  One divergence, deliberate: ServUO's yard is a *strip* five tiles off the front
  and back, because a foundation knows which way it faces. A classic multi does
  not carry a facing, so the yard here is a square. Named in the code rather than
  left to be discovered as a bug.
- **A house is two facts and they are undone separately.** `unblock` takes the
  walls out of the obstruction index; the *entity* still owns a yard until it is
  despawned. A demolition that called only the first would leave a plot nobody
  could ever build on again — asserted in the tests, and it is the shape H5's
  moving crate has to get right.
- **The save is in, and what it saves is where the house stands.** Not the
  components: a multi's shape is a pure function of its id and lives in the
  client's files, so a copy would go stale the day the operator updates their
  install — and then the shard's walls and the client's picture disagree with
  nothing to say which is right. The footprint is recomputed at boot from the
  same table placement read it from.

  A restore does **not** go through `place`. That decides whether a house *may*
  stand somewhere, and a house legal when it was built stays built: a shard that
  changed its yard size would otherwise demolish half of Britannia at the next
  restart, silently.

  Schema **v27**, and it is the first bump that is not about reading. A v26
  database opens fine and holds no houses, which is true of it. What it must not
  do is keep being written by a build that does not know about them — an older
  engine would agree about the version, ignore the table, and hand out item
  serials one of which a saved house already holds. The bump is for the *writer*.

  A shard booted **without** client files restores the houses and gives them no
  walls, rather than dropping them. Losing somebody's property over a
  misconfigured `world.client_files` is the worse failure, and it is gated.

---

### H2 — the deed, and the cursor that shows the house

**What a player sees:** they buy a deed, double-click it, and the house follows
the cursor until they pick a spot.

1. `MultiTargetRequest` in `openshard_protocol` — `0x99`, both lengths, the
   version boundary the other High Seas packets already use.
2. The client half: `WorldView` folds it, and the app draws the multi under the
   pointer. Our own client owns `multi.mul` too, so the picture is a lookup.
3. `TargetPurpose::PlaceHouse { deed }`, answered by the `0x6C` this engine
   already reads.
4. The deed as an item, and the vendor that sells one.

**Done when:** a deed bought from a vendor places a house where the player
clicked and is consumed; a refused placement says which of D3's rules it broke
and keeps the deed.

#### Built on the server; the client half is not drawn yet

Items 1, 3 and 4 are in, plus `.deed <multi id>` so the path is reachable on a
running shard before a vendor sells one. `.house` remains the staff shortcut that
places directly; a deed goes through every placement rule with the house drawn
under the pointer, which is the whole reason `0x99` exists.

- **`0x99` is not an `EncodePacket`.** Its `LENGTH` is a `const` and the packet
  has two — 26 classic, 30 from High Seas, the same bytes with four zeroes on the
  end. Declaring either would be a lie the framer's own assertion catches, so it
  follows `OpenContainer`'s precedent: an inherent `write_body`, a
  `multi_target_length(version)` beside it, and `ServerPacket::length` (which
  *does* see a version) picking between them.
- **`MultiId` exists now**, because the N10 gate refused a bare `u16` and was
  right to: a multi id and the graphic a placed multi draws as are two `u16`s
  that overlap, `0x99` writes the bare one and `0x1A` the masked one, and a value
  holding neither type cannot say which it had.
- **The cursor carries the *deed*, not the multi.** A deed sold, dropped or
  destroyed while the cursor was up must not still place a house, and a player
  with one deed and a fast hand must not place two — so the deed is re-read when
  the click lands, and the multi comes off it then.
- **The deed is spent on success and kept on a refusal.** A player who picked a
  bad spot has lost a click, not a house.

**Still open in H2:** the preview's *picture*. `0x99` is folded into
`WorldView` now — `OpenTarget` carries the cursor and the house as one value, so
a plain `0x6C` arriving after a house cursor cannot leave a villa following the
pointer. Two `Option`s side by side would have been a packet away from exactly
that, which is `combat.md`'s D1 in a different colour.

What is left is drawing it, and the subtlety worth knowing before starting: the
draw geometry is cached against `items_fingerprint(&presentation.items)`
(`frame_geometry.rs`), and a preview moves with the *pointer* rather than with
the item list. Appending the pieces to `presentation.items` would also desync
`item_serials`, which picking indexes by position — and the preview must not be
pickable anyway, since it is not a thing in the world. So it wants a list of its
own, chained at the collect call and left out of the fingerprint's stead by
including the pointer tile.

---

### H3 — who may come in

**What a player sees:** their door opens for them and not for a stranger.

1. The house sign, the ownership it names, and the gump it opens.
2. Co-owners, friends and bans — three lists with ServUO's own limits, and the
   same "an invitation is a consent" rule guilds already have.
3. The door: locked to the house, opened by owner and friends, and the key.
4. The ban: a banned player standing inside is moved out, which is the only rule
   here that acts on somebody rather than refusing them.

---

### H4 — lockdowns and secures

**What a player sees:** they can put things down inside and find them there.

An item inside a house is ordinarily loose and decays. A **lockdown** is an item
pinned in place; a **secure** is a container only named people may open. Both are
counts against a house's own storage allowance, which is what stops a house being
a bank box with a roof.

`items::capacity` is the shape this reuses — the ceiling exists, and a secure is
a container with an owner list on top of one.

---

### H5 — decay, and the crate

**What a player sees:** a house nobody has visited falls down, and what was in it
is not lost.

1. D6's tick count, the refresh, and the five stages ServUO names.
2. D8's moving crate.
3. Demolition by the owner, which is the same path arriving deliberately.

## What this plan does not cover

- **Customisation** — D7, by name.
- **Boats.** They are multis too and this reader already reads them, and a boat
  is a house that *moves*, which is a different problem: every component's
  position changes together and the obstruction index has to follow. Worth doing
  and not here.
- **Ilshenar, T2A and Eodon's no-housing rules.** They are region flags (D4) and
  this engine has one facet loaded.

## Backlog, found while planning this

- **`Obstructions` has never had a hundred entries added at once.** It is filled
  from the map at boot and poked at by doors since. Placing a house is the first
  bulk write, and whether it wants one it can undo cheaply — a demolition is the
  same hundred entries coming back out — has not been asked.
- **A multi's components carry a `dz` and the obstruction index is per tile.** A
  two-storey house has two floors over one tile and the step check has to pick
  the one the walker is on. This is the same question `can_step`'s `GetStartZ`
  answers for statics, and it should be checked that it answers it for a floor
  ten tiles up rather than assumed.
- **The five multis that draw nothing** (`findings.md`) are treasure-site markers,
  and placement must refuse an id with no drawn components rather than spawn an
  invisible house.
- ~~**Our own client would draw a house as one unrelated sprite.**~~ Fixed, and
  it was as bad as it looked: `render::items::collect` had no notion of a multi,
  and a static id space running to `0x10000` means `0x4064` is a *valid* art id,
  so a villa drew as whatever static happened to sit there — silently, with no
  error anywhere.

  `net_command::multi_pieces` is the expansion, at the seam where the view
  becomes a draw list, so the renderer never learns what a multi is: it is handed
  more items and nothing else changes. Every piece takes the *house's* serial,
  which is what makes clicking any wall pick the house.

  The load-bearing detail is that it answers `None` and not an empty list when
  the client has no multi table. Falling through to the ordinary item path is
  precisely the old bug.
