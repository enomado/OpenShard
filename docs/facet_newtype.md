# Facet newtypes: carrying a wrap that already exists

Living plan for a multi-session sweep of the bare `facet: u8` left across
`crates/server/*` and a few of `crates/common/*`, opened by
[`HANDOFF_newtype_hunt_server_common_render_2026-08-11`](../../open_shard_files/handoffs/HANDOFF_newtype_hunt_server_common_render_2026-08-11.md)'s
newtype hunt as the single largest finding, out of scope for that pass on
purpose. It is a sibling to [`protocol_newtypes.md`](protocol_newtypes.md) —
same shape, same "stages plus a machine-checked gate" discipline — but a
narrower problem: `protocol::world::Facet(pub u8)` already exists, already
carries the right derives, and is already the type at both ends of most call
chains. What is missing is the middle.

As with its predecessor: when reality contradicts a decision here, change this
file in the same commit that changes the code.

## Why

`docs/roadmap.md`'s "Two types for one facet byte" entry (closed 2026-08-xx,
before this hunt) already unified two competing types into one
`protocol::world::Facet` and converted every *state-owning* call — `state.
facet_of`, `state.facet_state`, `WorldState::move_to`, the `facets:
BTreeMap<Facet, FacetState>` table itself — to the wrapped type. That closed
fix did not, and was not asked to, touch the *functions in between*: a caller
holds a `Facet` from `state.facet_of(entity)`, unwraps it to `.0` to satisfy a
neighbour's `fn foo(facet: u8, ...)`, and that neighbour immediately rewraps
it (`Facet(facet)`) to call back into `state`. The type is never wrong, never
absent at either end — it is thrown away and rebuilt at every function
boundary in between, which is exactly the shape
[N2's `Serial` finding](protocol_newtypes.md#amendments-forced-by-n2-mobilers)
described: "the call sites already held a `Serial`... and were unwrapping them
to satisfy a `u32`."

By grep, `facet: u8` (as a function parameter, struct field, or enum variant
field) appears in:

| crate | files | occurrences |
|---|---|---|
| `world` | `tick.rs`, `tick/{command,gates,decor,regions,travel,fields,death,tests}.rs`, `gm.rs`, `spawner.rs`, `events.rs` | ~38 |
| `scripting` | `lib.rs`, `engine/ops.rs` | ~16 |
| `persistence` | `record.rs`, `sqlite.rs`, `pg.rs` | ~10, **not in scope — see Decisions** |
| `ai` | `lib.rs` | 7 |
| `magic` | `travel.rs` | 5 |
| `npc` | `guards.rs`, `live.rs`, `spawn.rs` | 5 |
| `items` | `spawn.rs` | 4 |
| `state` | `harvest.rs` | 2 |
| `skills` | `handlers/harvest.rs` | 1 |
| `uofiles` | `map.rs` | 1, **not in scope — see Decisions** |
| examples (`render`, `uofiles`) | 2, follow their crate's fix |

Every occurrence outside the two carve-outs below is the same shape checked
by hand while scoping this plan: `ai::lib.rs`'s `foe_in_sight`, `probe`,
`chase_step`, `flee_step`, `kite_step` and `step_toward` all take `facet: u8`
while every one of their callers holds a `Facet` and writes `facet.0` to call
in; `npc::guards.rs`'s `call_guards`/`guarded_here`/`nearest_candidate` and
`npc::spawn.rs`, `items::spawn.rs`, `magic::travel.rs`'s `may_travel`/
`describe`, `world::gm.rs`'s admin commands and every `world::tick/*.rs` file
repeat it. Nowhere does a bare `facet: u8` in this set hold a value that
*isn't* a `Facet` a moment before or after — the wrap is never the question;
carrying it is.

## Decisions

Settled. Do not re-open mid-sweep.

**F1. This is not `protocol_newtypes.md`'s problem, so it does not get that
plan's machinery.** `Facet` has no domain to validate — every one of the 256
values a `u8` can hold is a legal facet number in principle (an unloaded one
is refused at the point that matters, `state.facets.contains_key`, which is a
*shard-config* fact, not a wire-format one). There is no `RawFacet`, no
`interpret`, no `validate`, no class A/B/C/D split. A field either already
carries `Facet` or gets changed to, full stop. This is why the sweep is worth
a plan and not a `sed` run despite being simpler than the protocol one: it
touches ~70 call sites across eight crates, and a plan is what keeps an agent
from "fixing" a boundary that is supposed to stay bare (F2).

**F2. Two boundaries stay bare, on purpose, and are not part of this sweep's
count.**

- **`persistence::{record, sqlite, pg}.rs` is the disk/SQL boundary.**
  `tick/regions.rs`'s own comment already says it — `// .0 at the record
  seam: a saved facet is a SQL column.` A `u8` is what SQLite and Postgres
  columns hold; `record.rs`'s structs are what serde reads a save file into.
  Converting there is not a bug, it is the seam, exactly
  [N3 amendment 7](protocol_newtypes.md#amendments-forced-by-n3-speechrs)'s
  `localized_message` argument and the `Contained.grid` exception in
  [N4 amendment 4](protocol_newtypes.md#amendments-forced-by-n4-containersrs).
  Left as-is; not on this sweep's task list and not on its coverage count
  either — it was never bare by omission.
- **`uofiles::map::{load_facet, candidate_shapes, facet_size,
  largest_facet_within}`'s `facet: Option<u8>` indexes a fixed-size array
  (`FACET_SHAPES.get(facet as usize)`) and formats client filenames
  (`format!("map{facet}LegacyMUL.uop")`).** Both uses want the raw number,
  not the domain type — an array index and a path component are exactly
  [N1 amendment 2](protocol_newtypes.md#amendments-forced-by-n1-the-rest-of-worldrs)'s
  `Point` case: the *number itself* is what is being used, not "a value
  believed to be a legal facet." `uofiles` is below `protocol` in the
  dependency graph for everything else it parses (`Graphic`, `Hue`), but
  `Facet` is the one type this file would gain nothing from taking a
  dependency on. Stays bare; documented here so a future scan does not
  rediscover it as an open question.

**F3. Order: the crate that discards it first, to the crate with the most
call sites.** `ai` (pilot, below) is small, self-contained, and every one of
its callers already holds a `Facet` — proving the pattern costs one file. From
there: `npc`, `items`, `magic` (each under ten occurrences, each the same
shape as the pilot); `state::harvest` and `skills::handlers::harvest`
together, since the harvest handler is `state::harvest`'s only caller; `world`
last, because it is both the largest (`tick.rs` and eight `tick/*.rs` files)
and the one place `Command`'s enum variants live — the type has to exist on
every upstream caller (`ai`, `npc`, `items`, `magic`) before `Command`'s own
fields can stop needing a `.0` to build. `scripting` closes the sweep, not
because it is hardest technically but because its `facet: u8` fields are a
**third kind of boundary**, not yet decided (F4).

**F4. `scripting`'s boundary is not decided yet, and starts as an open
question, not a default answer.** Every `facet: u8` in `scripting::lib.rs`
(`ScriptEvent` variants, notifications *to* Rhai) and the `#[derive(serde::
Deserialize)] *Spec` structs in `engine/ops.rs` (`SpawnSpec`, `ContainerSpec`,
region/door specs, read *from* Rhai) is a genuine serialization boundary in
the sense F2 already carved out for persistence — but unlike `persistence`,
this one is plausibly closable: `ClilocId` and `SoundId` already gained
`#[serde(transparent)]` `Serialize`/`Deserialize` for exactly this reason
(`protocol_newtypes.md`'s N3 backlog, closed by "N-tables"). If `Facet` gains
the same derives, the `*Spec` structs can hold `pub facet: Facet` directly and
lose their per-call `Facet(spec.facet)` conversions; `ScriptEvent`'s outbound
variants gain the same for free. What is *not* decided yet is whether the
handful of `op_*` functions that take `facet: u8` as a **direct** Rhai-bound
argument (`op_clear_regions`, `public_gate_at`'s script-facing callers, if
any) can bind `Facet` the same way — that depends on how this crate's Rhai
integration resolves native function parameter types, which the sweep has not
inspected yet. The `scripting` stage starts by answering that question, not
by assuming the persistence answer transfers.

**F5. No compatibility shims.** Same as `protocol_newtypes.md`'s N11: a stage
wraps a group of signatures **and** updates every call site in the same
commit. A `.0` left in place "to keep the diff small" is the exact
invisibility this sweep exists to remove.

**F6. Coverage is counted, not assumed.** Each stage's commit message records
the file's `facet: u8`/`facet:u8` occurrence count before and after, the same
discipline as N10. The sweep's own gate (below) is added once the crate order
in F3 is far enough along that most of the workspace is clean — early stages
would spend more effort maintaining a mostly-red gate than the gate is worth.

## The gate

A text scan in the same spirit as
[`bare_integer_fields.rs`](../crates/common/protocol/tests/bare_integer_fields.rs),
but simpler: `Facet` is one name, not a class hierarchy, so the check is "does
`facet: u8` (or `facet:u8`) appear in this fixed list of files" rather than a
directory walk with a type-shape matcher. It lives in
`crates/common/protocol/tests/facet_bare_fields.rs` — `protocol` is where
`Facet` is defined, and the test reaches out to the other crates' source by a
workspace-relative path (`CARGO_MANIFEST_DIR/../../..`), the same way this
plan's own survey did, because the thing being asserted is a property of the
workspace, not of `protocol`'s own `src/`. The two F2 carve-outs
(`persistence::{record,sqlite,pg}.rs`, `uofiles::map.rs`) are the allowlist,
each with the reason from F2. Added at the end of F3's order, not the start —
see F6.

## The pilot: `ai::lib.rs`

Smallest file with the pattern, seven occurrences, one file plus two call
sites in `npc::live.rs` (`openshard_ai::step_toward`). Every function already
receives a `Facet` from its own caller and unwraps it on the way in:

| function | today | becomes |
|---|---|---|
| `step_toward` (pub) | `facet: u8` | `facet: Facet` |
| `foe_in_sight` | `facet: u8`, compares `state.facet_of(entity) == Facet(facet)` | `facet: Facet`, compares `== facet` |
| `probe` | `facet: u8`, calls `state.facet_state(Facet(facet))` | `facet: Facet`, calls `state.facet_state(facet)` |
| `flee_step` | `facet: u8` | `facet: Facet` |
| `chase_step` | `facet: u8` | `facet: Facet` |
| `kite_step` | `facet: u8` | `facet: Facet` |

Every caller inside `lib.rs` currently writes `facet.0` to call one of these
and gets a `Facet` back moments later from `state.facet_of`/`state.facet_state`
— all six become `facet` with no `.0`. The two external callers
(`npc::live.rs::nearest_player`/`step_toward` call site) go the same way:
`facet.0` becomes `facet`, and `nearest_player`'s own `facet: u8` parameter
picks up the type too, since it exists for the same reason.

## Amendments forced by the pilot

1. **`step_toward`'s `Option<u8>` return stayed `Option<u8>` — direction, not
   facet, and the two were never confused. This sweep is about the *facet*
   parameter only; a heading/direction sweep is a separate, already-recorded
   backlog item** (`docs/roadmap.md`'s "server/common/render newtype hunt"
   entry #2, `Direction` unwrapped through `ai`'s pathing core). Touching it
   here would have widened the pilot's blast radius for no reason connected
   to `Facet`.
2. **`npc::live.rs::nearest_player` picked up `Facet` at the same time as its
   one caller, `live.rs`'s own tick function**, because both sides of that
   call already held a `Facet` and were unwrapping/rewrapping across a single
   crate-internal boundary — the same shape F3 predicted `npc` would be, one
   function early.
3. **A third external caller of `step_toward` turned up outside both crates
   the survey named: `quests::progress.rs`'s escort-following beat.** F3's
   `npc`-stage estimate only counted `npc`'s own call sites; `quests` depends
   on `ai` directly for the same "walk toward, planned around obstacles"
   behaviour an escortable uses. Its one call site (`facet.0` → `facet`, the
   variable already a `Facet` from `state.facet_of(npc)` two lines above)
   went with the pilot rather than waiting for a `quests` stage that F3 never
   scheduled — the fix was one line and the alternative was leaving a known
   call site broken until some later stage happened to notice it. Any stage
   after this one should grep a function's callers workspace-wide before
   trusting a single crate's occurrence count.
4. Bare-integer `facet` count in `ai/lib.rs`: 7 before, 0 after. `npc/live.rs`:
   2 before (two call-site `.0`s: `nearest_player`'s call and `walk_home`'s)
   plus `nearest_player`'s own parameter, all 0 after — `nearest_player`
   folded into the pilot per amendment 2. `quests/progress.rs`: 1 before, 0
   after, per amendment 3.

## `npc::guards.rs` — done

`call_guards`, `guarded_here`, `nearest_candidate` → `facet: Facet`, same
shape as the pilot: every caller already held one (`guard_keywords`,
`hunt_with_guards`, both from `state.facet_of`). `guarded_here`'s
`Facet(facet)` construction and `nearest_candidate`'s `Facet(facet)` map key
both dropped. `hunt_with_guards`/`guard_keywords` are unchanged in signature
(neither took a bare `facet`), so the one external caller
(`world::tick/regions.rs`'s `hunt_with_guards`) needed no edit. Count: 3
before, 0 after (`guarded_here`'s param, `nearest_candidate`'s param,
`call_guards`'s param — `guard_keywords`/`hunt_with_guards` already wrote
`facet.0` at call sites, now write `facet`).

## Amendment forced by `guards.rs`: `SpawnSpec`-shaped structs are a fourth
## kind of boundary, not counted by F3

`npc::spawn.rs`'s `SpawnSpec.facet: u8` (and, by the same shape,
`items::spawn.rs`'s and `scripting::engine::ops.rs`'s own `SpawnSpec`s) is
**not** the pilot's pattern of "every caller already holds a `Facet` a moment
before." It is populated from `world`'s `Command::SpawnMobile` variant
(`world::tick.rs:851`, `world::tick/spawners.rs:80` via `area.facet`), which
is itself read off a bare `facet: u8` `Command` field — still unconverted,
`world`'s own stage. Converting `SpawnSpec.facet` now would force `world`'s
two call sites to convert early, which is exactly the dependency F3 already
named ("the type has to exist on every upstream caller... before `Command`'s
own fields can stop needing a `.0`") — just discovered one level lower than
F3's text described it, at a struct field instead of at `Command` itself.
`guards.rs`'s own `make_guard` keeps writing `facet: facet.0` into
`SpawnSpec` for this reason; it is not an oversight, it is the same boundary
F2/F4 already carve out, one more instance of it. **Any `SpawnSpec`-shaped
struct (`npc`, `items`, `scripting`) stays bare until `world`'s stage converts
`Command::SpawnMobile`/`RegisterRegions`/etc. — do not convert these fields as
part of the `npc`/`items`/`magic` stages F3 scheduled them under.** `npc`'s
count is `guards.rs` 3→0, `live.rs` (done in the pilot) — `spawn.rs`'s field
stays bare on purpose, not pending.

## Amendment: `magic::travel.rs` is blocked the same way, one level further out

Checked before starting the `magic` stage. `magic::may_travel`/`describe`'s
`facet: u8` params look like the pilot shape from inside `magic` — every
`world::tick/{travel,gates}.rs` call site already holds a `Facet` a moment
before (`facet.0`, `self.state.facet_of(entity).0`) — but `magic::
destination_of` reads `mark.facet` off `state::components::RuneMark {
pub facet: u8, .. }`, and `world::tick/travel.rs`'s own `travel_to`/`recall`
keep `facet: u8` all the way through (`Facet(facet) != here`,
`self.state.facets.contains_key(&Facet(facet))`, `can_stand_at(facet, ..)`) —
that function is `world`'s own, not `magic`'s, and is explicitly the large
stage F3 puts last. Converting `magic`'s signatures now would force
`RuneMark` (a `state` component, not in the F3 count at all) and a slice of
`world::tick/travel.rs` to convert early — the same shape as the `SpawnSpec`
finding above, one hop further from `magic`. `PublicGate.facet`/`gate()`/
`public_gate_at` are a separate, self-contained const table with no such
chain (`world::tick/gates.rs`'s two call sites already hold `Facet` outright)
and could move alone, but doing only that and leaving `may_travel`/
`describe`/`standing_at`/`destination_of` bare would split one file's stage
across two sessions for a handful of lines — deferred to when `magic`'s stage
is picked up properly, alongside `RuneMark`. **`RuneMark.facet` needs adding
to `state`'s stage list** (it was not in the original ~87-occurrence survey;
`state` table above only counted `state::harvest.rs`).

## `items::spawn.rs` — half done, the other half blocked the same way

`spawn_leftover`, `place_on_ground` → `facet: Facet`. Both had every caller
already holding one: `items::drag.rs` (three call sites, one from a local
`facet: Facet` already in scope, two from `state.facet_of(..)`) and
`items::trade.rs`'s one call site (`state.facet_of(receiver)`) — none crossed
a `Command`-shaped boundary, so both converted clean, pilot pattern. `spawn_item`/
`spawn_container` did **not**: `spawn_item` has one caller
(`world::tick.rs:813`, `Command::SpawnItem`'s handler) that is still bare,
same as `npc::spawn.rs`'s `SpawnSpec` finding — left untouched, on purpose,
not an oversight. `world::gm.rs`'s three `spawn_item` calls and
`skills::handlers::harvest.rs`'s one already write `facet.0` and would have
converted trivially had the fourth (blocked) caller not existed — F5 (no
partial signatures) is why the whole function stays bare rather than
converting the three ready callers and leaving `tick.rs` as a lone `.0`
holdout. Count: `items/spawn.rs` 2 of 4 signatures converted (`spawn_item`/
`spawn_container` stay bare, blocked); `drag.rs` 3→0, `trade.rs` 1→0.

## What's next

`state::harvest.rs` + `skills::handlers::harvest.rs` — check each function
for the same `Command`/`SpawnSpec`-shaped blocker before converting; already
know `skills::handlers::harvest.rs`'s `spawn_item` call site itself will stay
`facet.0` regardless (see above), so this stage is about whichever of its own
signatures are self-contained. `magic` moves after `state`/`world` clear
`RuneMark` and `travel_to`, not in F3's original position — see the amendment
above. `world` remains the big one, and is now also where `Command::
SpawnMobile`/`SpawnItem`/`SpawnContainer`/`RegisterRegions` need to convert
to unblock `SpawnSpec`, `spawn_item`, `spawn_container` — record that
explicitly in `world`'s stage notes when it starts, so it isn't rediscovered
a third time.
`state::harvest` + `skills::handlers::harvest` together after that. `world`
(`tick.rs` and eight `tick/*.rs` files, plus `gm.rs`, `spawner.rs`,
`events.rs`) is the big one and should not be attempted in less than two
sessions of its own — `tick/command.rs`'s `Command` enum alone is six
variants, each read by `tick.rs`'s own dispatch `match`, each written by a
`scripting::engine::ops.rs` `op_*` function, so its stage cannot land without
`scripting` deciding F4 first for the fields that cross into it. Converting
`world`'s `Command` fields is also what unblocks the `SpawnSpec` fields left
bare above — record that in `world`'s stage notes when it starts. `scripting`
closes the sweep. The gate (above) lands once `world` and `scripting` are
both done. Amendment 3 (pilot) above is a standing reminder for every
remaining stage: grep a function's callers workspace-wide, not just within
the crate F3 assigned it to.
