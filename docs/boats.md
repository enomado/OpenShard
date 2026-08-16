# A house that moves

`docs/housing.md` deferred boats in one sentence and it is the right one: *"a
boat is a house that **moves**, which is a different problem: every component's
position changes together and the obstruction index has to follow."*

Every hard decision below follows from the word **moves** rather than from the
word **boat**. The multi reader already reads them; the picture is free the same
way a house's is; the placement rules are housing's with the sign flipped on one
of them. What is new is that a boat's shape is somewhere different every few
seconds, and nothing in this engine was built for that.

> Read [`housing.md`](housing.md) first — H1's decisions about multis, footprints
> and the obstruction index are assumed here rather than restated, and two of
> them turn out not to survive contact with a thing that moves.

## What exists, which is almost nothing

No boat multi id, no constant, no comment. `FOUNDATION_IDS` is the only named
multi range in the tree and it is a refusal.

One thing does exist, and it turns out to matter: **`Feature::SmoothShip`**
(`protocol/src/feature.rs:87`) — *"Smooth boat movement (`0xF6`). Since
7.0.9.0."* The version gate is written and the packet behind it is not. That is
exactly the state `0x99` was in before housing's H2, and it means the wire
question below has a named answer rather than an open one.

## What is missing, in one table

| piece | server | client (ours) | classic client |
|---|---|---|---|
| multi components | **built** (`uofiles::multi`) — reads boats too, unnamed | **built** — `net_command::multi_pieces` | reads its own files |
| a boat on the water | nothing places one | draws multis already | draws multis already |
| the hull blocking a step | `Obstructions` **cannot express a deck** — B3 | — | n/a |
| water as a question gameplay can ask | **two notions, neither reachable** — B4 | — | n/a |
| `0xF6` smooth movement | **no packet**, `Feature::SmoothShip` names it | **no packet** | speaks it, ≥ 7.0.9.0 |
| a passenger moving with the deck | **no parent relation of any kind** — B1 | n/a | n/a |
| the tiller, the hold, the plank | — | — | ordinary items |
| decay, the deed | — | — | n/a |

## Decisions, taken here

### B1 — a passenger's position is absolute, moved the way `World::step` already moves one. No parent transform.

The alternative is real and worth naming before refusing it: a
`Carried { parent, dx, dy, dz }` and a resolver, so a passenger's position is
*derived* rather than written.

It is refused on the strongest evidence available, which is that **this engine
already tried the weaker version and declined it**. Mounting does not carry the
mount — it *removes it from the world*: `forget` from every watcher,
`sectors.remove`, `registry.remove::<Position>`
(`items/src/mounts.rs:82-83`). A ridden creature has no position at all, and the
saddle item is what the ride is rebuilt from at restore. Carrying was not
expressible, so the engine deleted instead.

The structural reason it was not expressible: `Position`, `Contained` and
`Equipped` are mutually exclusive and absolute, and **everything** reads
`Position` — `Sectors`, `watchers_of`, `broadcast_move`, `refresh_around`, the
save sweep, `region_at`, `house_at`, `evict_the_banned`, the step check's `from`.
A transform is a fourth kind of "where", and until every one of those learned it
each would answer the wrong tile *while looking correct*. That is `style.md`'s
argument against `Deref` in a different colour: the hole is spelled with the
empty string, and there is no line for a reviewer to object to.

So a boat move computes the delta once and then moves each occupant absolutely,
reusing the tail `World::step` (`tick/motion.rs:207`) already reuses —
`disrupt`, `move_to` (which sends the player's own `0x20`), `refresh_around`,
`broadcast_move`.

**The cost, named:** a passenger's deck position is authoritative and rewritten
every move. Standing on a deck is not *derived* from the boat; it is
re-established each time. If the two ever disagree, the position wins, because
the position is what every other system reads.

### B1a — the manifest is derived per move, not stored.

Who moves when the boat moves is answered by *who is standing on a tile the boat
covers*, derived at the moment of the move from `tiles_of` and `Sectors::nearby`.

Not an `OnDeck` component. That is a second copy of a fact `Position` already
holds, and a copy that goes stale the moment somebody steps aboard, is teleported
aboard, logs in on a deck, or dies on one.

This is `adopt_doors`' rule reused rather than restated — *a door inside your
house is your house's door; a body on the deck is a passenger* — and
`evict_the_banned` is the worked example of the same scan. It is over one sector,
not the registry, and it runs on the move cadence rather than per tick.

### B2 — the wire is forget-then-reveal, and that is the reference's own answer for a classic client.

There is no incremental item-move packet in this repo. The only precedent for a
ground item changing tile is `items::doors::set_door`
(`items/src/doors.rs:173-253`): `forget` (`0x1D`) from every watcher, write
`Position`, `sectors.insert`, swap the obstruction, `state.reveal` (`0x1A`). It
flickers by construction, and its own doc says why — *"a client only redraws what
it was told to forget."*

Three facts settle this rather than one preference:

- **ServUO does the same thing.** Its `BaseBoat` pre-High-Seas removes and
  re-sends its components on each move. The flicker is not this engine's
  shortcoming; it is what a 2D client without `0xF6` gets from any server.
- **`0xF6` exists and is already gated.** So the smooth path is available — to
  High Seas clients only, and this shard's floor is AoS. It therefore cannot be
  the *only* answer, and it must be reached through
  `version.supports(Feature::SmoothShip)` and never through an era comparison,
  which is `architecture.md`'s rule with a table of counterexamples behind it.
- **The cadence is the mitigation, and it is a decision rather than a detail.**
  ServUO steps a boat on a timer. Here that is a `ticks.is_multiple_of(N)` gate
  at the call site in `tick.rs` — the existing idiom, beside `collapse_houses`
  (`tick.rs:707`), which is its nearest neighbour in kind. A redraw every N ticks
  is a boat that shudders; a redraw every tick is a boat nobody can look at.

The phase that lands this owes a **number**: packets per move is one
forget-and-reveal for the hull per watcher, plus one `move_to` per occupant.
Bound it and write it down.

### B3 — the hull stays **out** of `Obstructions`, and a boat gets an index of its own.

`Obstructions` is `HashMap<(u16,u16), Vec<Obstacle>>` keyed by *(entity, z)*,
with no translate, no bulk write and no entity→tiles reverse index. Moving an
N-tile footprint through it is 2N hashed vector operations plus two
`footprint_of` derivations, every move, every boat, for ever.

Refusing to add a bulk API is a design argument and not a performance one. The
index's own reason for existing says so, and so does housing's D2: *"a step is
ten a second and a house does not move."* A boat is the counter-case to the
premise that put houses in there. Bolting a fast path onto a structure whose
whole justification is that its contents are static is `style.md`'s fudge
constant one level up — a second mechanism closing a gap the first mechanism's
premise opened.

**And there is a stronger reason, which is the real one: `Obstructions` only ever
subtracts.** A house's entry says *this tile is closed*. A boat has to say two
things — the hull is closed, **and the deck is somewhere to stand, at height z,
over water that is otherwise not ground at all**. A house never had to add a
floor, because its floors sit on land the map already calls walkable.
`Obstructions` has no way to say "there is now somewhere to stand here", and
giving it one would make it a different structure with a different name.

So: a per-facet `Boats` index — entity → origin plus multi id — consulted by
`LiveTerrain`, which is already the composition seam (map + obstructions) and is
exactly the shape a third source belongs in.

**The hot-path warning, stated before anyone measures it:** `LiveTerrain::can_step`
runs for every step by every mobile, and its diagonal rule re-enters it twice
more. The boat consultation must be an integer comparison against an empty index
in the overwhelming case, and the phase that lands it owes a measurement rather
than an assurance.

### B4 — `MapTerrain::swimming` stays false, and is not deleted.

`swimming` (`movement/src/terrain.rs:65`) is a property of a *terrain*, and a
facet has one. Setting it true makes water walkable for **every mobile on the
shard** — that is not "boats work", it is "everybody walks on water". It is
documented "A boat or a fish says yes", has never been set true on any server
path, and false is the correct state for it.

What a boat needs is two narrower answers, and they are different questions.

**(i) May a boat be placed here?** A water test at placement, `is_road`'s shape
(`housing/src/lib.rs:645`) with the sign flipped. But there are already **two**
notions of water in this tree and neither is reachable from where a boat would
ask:

- `TileFlags::is_water()` in `openshard-uofiles` — the client's own truth —
  reachable only from inside `MapTerrain::land_is_ground`
  (`terrain.rs:386`), which is private.
- `WATER_TILES`, id ranges generated by `state/build.rs` and consumed by fishing.

Writing a third is what `style.md`'s "look for it before writing it" forbids, and
the fix is small: **`Terrain::land_is_water(tile) -> bool`**, defaulting `false`,
implemented on `MapTerrain` over the flag it already reads. That is the seam
`item_blocks`, `item_height` and `multi_components` all came through, and it
answers "what if the shard has no client files" for free the way every other
method on that trait does.

Fishing's `WATER_TILES` then becomes the no-client-files fallback rather than a
second truth — named as a backlog item, not done here.

**(ii) May a mobile stand on the deck?** Not a water question at all. A deck is a
climbable platform static at a z above the water, and `MapTerrain::check` already
stands bodies on platform statics — it simply never sees this one, because a
multi's components are not in the map file. That gap is B3's index, and it is
B3's *positive* half.

### B5 — the deck is the open pier/bridge bug, and this is the phase that supplies its repro.

`docs/roadmap.md:405-420` records an **open** movement defect: `MapTerrain::check`'s
`landCheck` guard, ported variable-for-variable from the reference and audited
rather than slipped, discards a climbable platform static when the land beneath
it is walkable and its average height reads close to the deck. What saves piers
and bridges today is that they sit over water, where `land_is_ground` is false
and the guard never fires. The roadmap says it is unfixed because it needs a
repro against real client files rather than a silent patch.

Two things follow, and the first is the concrete reason B4 goes the way it does:

- **Turning `swimming` on would fire the guard under every deck.**
  `land_is_ground` becomes true over water, the deck static is discarded from the
  candidate list, and a player walking aboard lands on the water's `land_center`
  — in the sea, under their own boat. That is not a tidiness argument against the
  flag; it is a measurable fall.
- **A boat with a deck is the repro the roadmap asks for, and it needs no client
  files.** A synthetic multi with a climbable platform component at a known z
  over a land tile of known height is constructible in a test — `Multi::new` is
  public and `Component`'s fields are public — and it reproduces the *shore-end*
  case (deck over walkable land) the player report of 2026-08-02 describes.

So the boat phases do not have to *fix* the divergence, and must not make it
worse. But they are what finally hands the fix its evidence, and that is named as
an output rather than left to be noticed.

### B6 — control is speech, and the tiller is a double-click.

The reference's tillerman answers speech keywords — forward, back, left, unfurl
sail, stop. This engine has the machinery already: `tick/speech.rs` routes speech
to keyword answers, and `npc`'s keyword answers are the precedent. The tiller is
an ordinary double-click target with `HouseSign`'s exact shape — a component
naming the boat by serial, so a tiller left standing over a boat that has sunk
opens nothing.

**No packet numbers or keyword strings are asserted in this document.**
`style.md`'s "ports name their source" applies: they come out of the reference at
implementation time and are cited at the constant, not guessed in a plan.

### B7 — a boat's own footprint is not a `no_housing` region, and does not need to be.

It might look as though housing's H6 gives "no house on a boat" for free by
setting a flag on a region the boat carries. It does not need to, and a boat does
not carry a region at all: `check_yard` already keeps five tiles between a house
and anything, measured wall to wall, and a boat that is not in `Obstructions`
(B3) is not in the yard scan either — so the placement question is answered by
B-1's own water rule instead. A house may not go on water; that is one mechanism,
not two. Named so nobody adds the second.

## The phases

Four, and the first two are the honest split: the **index** is one phase's whole
content, and the **motion** is the next one's.

### B1 — a ship on the water, moored

**What a player sees:** a ship in the harbour, and they can walk its deck.

1. `Terrain::land_is_water`, and `MapTerrain`'s implementation over the flag it
   already reads.
2. `openshard-boats`: `place(state, actor, at, facet, multi, owner)` —
   `housing::place`'s shape, refusing anything not over water, staff-exempt on
   the judgement refusals the same way H6's is.
3. The `Boats` index on `FacetState`, and `LiveTerrain` consulting it for **both**
   questions — the hull blocks, the deck is a surface.
4. Saved: a `BoatRecord` (serial, multi, position, facet, owner), the index
   rebuilt at boot. Components **not** saved, for `HouseRecord`'s reason
   unchanged — a boat's shape *is* a pure function of its id, so unlike a
   customised house it is exactly the case that rule was written for.
5. `.boat <multi id>`, `.house`'s shape.

**Done when** `.boat` puts a ship on the water, both clients draw it, walking
onto the deck lands the player *on* the deck at the right z and is not refused,
walking into the hull is refused, and it is still there after a restart.

Nothing here needs a single decision from B1, B2 or B5. That is the point of the
boundary.

### B2 — it moves

1. `Boats::step(state, boat, direction)` — decide-then-apply, `World::step`'s
   structure with the terrain check replaced by *does the whole translated
   footprint fit*.
2. The manifest derived per move (B1a), each occupant moved absolutely (B1).
3. The wire: forget-and-reveal for the hull, `move_to` per occupant (B2).
4. The cadence gate in `tick.rs`, beside `collapse_houses` and
   `items::close_doors` — the two systems that already do the halves of this and
   never together.
5. Speech control and the tiller (B6).

**Done when** "forward" moves the ship a tile, everyone standing on it arrives
with it, a player's own camera follows, and a ship steered into a rock stops
rather than passing through it.

**The one collision test this phase owes:** two boats. A hull is not in
`Obstructions`, so two hulls do not see each other through the mechanism that
stops everything else, and *two ships in one tile* is the failure that mechanism
would have caught for free. The step check must ask the boat index about **other
boats**, and the test is named `two_boats_do_not_occupy_one_tile`.

### B3 — smooth, for the clients that can

`0xF6`, behind `version.supports(Feature::SmoothShip)`. A High Seas client gets
one packet per move; a 4.0 client keeps B2's redraw, unchanged and still correct.
Strictly better, and it removes nothing.

### B4 — the boat as property

The hold as a container, the plank as a door, the deed, decay. All of it is
housing's H2–H5 with a different noun, and none of it is on the critical path for
a ship that sails.

## What this plan does not cover

- **Docking, and mooring to a pier.** It is a relationship between two multis and
  it wants B5's bug fixed first.
- **Pets and NPCs following aboard.** The manifest carries whoever is *standing*
  on the deck at the moment of the move, which is already right for a pet that
  happens to be there. A pet that should re-board after being left behind is an
  AI rule, not a boat rule.
- **The tillerman as an NPC.** The reference's is a mobile with dialogue. Here
  the tiller is an item and the answers are speech keywords, which is the same
  intent out of machinery that exists.
- **Multi-facet oceans.** `WorldConfig.facets` defaults to `vec![0]` and the
  checked-in `openshard.toml` does not override it. The index is per-facet like
  everything else on `FacetState`, so this costs nothing to leave.
- **Fixing the pier/bridge divergence.** B5 supplies the repro and says so; the
  fix is a deliberate deviation from the reference's `Movement.cs` and stays the
  roadmap's decision to take.
- **A translate or bulk API on `Obstructions`.** B3, and the reason is written
  down so the next reader who notices the missing API knows it was declined
  rather than overlooked.

## Backlog, found while planning this

- **Two notions of water, and a third would have been written.**
  `TileFlags::is_water()` is the client's truth and is private behind
  `MapTerrain::land_is_ground`; `WATER_TILES` is a generated id-range table that
  fishing uses. B4 adds the seam that reaches the first; making the second its
  documented fallback is a separate change and is not done here.
- **`MapTerrain::swimming` has been dead since it was written.** It is set true
  only in movement's own test helper. B4 keeps it and says what it is for, which
  is better than either deleting a correct abstraction or enabling it by
  accident — but it has now been unread long enough to be worth a note.
- **`World::step`'s tail is the reusable part and it is not factored out.** B1
  reuses `disrupt` → `move_to` → `refresh_around` → `broadcast_move` by copying
  the sequence, which is the third caller of that sequence after `npc::live` and
  `quests::advance_escorts`. A fourth would be the point at which it wants a name.
