# Protocol newtypes: from bare integers to raw-then-validated fields

Living plan for a multi-session sweep of `crates/common/protocol`. It is the
sequel to [`protocol_rewrite.md`](protocol_rewrite.md), which turned 47 free
`encode_*` functions and a hand-written `match` into two root enums, and left
[D6](protocol_rewrite.md#decisions) — "newtypes on the wire" — deliberately
half-done: a newtype arrived only with the packet that first needed it, so
`Serial`, `Graphic`, `Hue`, `SoundId`, `CursorId` and `AuthKey` exist and
everything else is still a bare integer.

As with its predecessor: when reality contradicts a decision here, change this
file in the same commit that changes the code.

## Why

193 `pub <name>: u8|u16|u32|i8|…` fields remain in the crate's packet structs:

| module | bare int fields | module | bare int fields |
|---|---|---|---|
| `world.rs` | 39 | `login.rs` | 8 |
| `mobile.rs` | 37 | `skill.rs` | 6 |
| `speech.rs` | 22 | `context.rs` | 6 |
| `items.rs` | 16 | `version.rs` | 4 |
| `vendor.rs` | 14 | `spellbook.rs` | 3 |
| `feedback.rs` | 10 | `properties.rs`, `encoded.rs`, `combat.rs` | 2 each |
| `gump.rs` | 9 | `casting.rs`, `seed.rs` | 1 each |
| `containers.rs` | 9 | | |

Two separate problems hide in that number, and this sweep is about both.

**A bare integer does not say what it is.** A hue and a graphic are both `u16`;
a skill id and a stat value are both `u8`. Nothing but a reader's attention
stops `Hue(create.hair)` from compiling. This is what D6 already argued.

**A bare integer off the wire does not say whether anyone checked it.** This is
the sharper half, and the reason the sweep is worth a plan rather than a
`sed` run. `dispatch::create_character` today does:

```rust
strength: u16::from(create.strength),
…
.map(|choice| (choice.skill, u16::from(choice.value) * 10, SkillLock::Up, 0))
…
appearance: Some(Appearance { body: Graphic(create.body()), hue: Hue(create.skin_hue) }),
```

Every one of those values came straight off the wire and none of them was
checked. A client that sends `strength = 255, dexterity = 255, intelligence =
255` gets it. A client that sends `skill value = 255` gets a skill at 2550. A
client that sends any `u16` gets it as a skin hue, staff-only hues included.
CLAUDE.md's rule — *"a packet is not an invariant, it is a hostile input"* — is
stated and then not enforced, because the type system was never asked to carry
the distinction. Bare `u8` reads exactly the same whether it was validated or
not, so the absence is invisible at the call site and stays invisible in review.

So: every client-supplied field gets a `Raw*` type that can only become
something meaningful by passing through a named check. The check is the thing
being added; the newtype is what makes its absence a compile error.

## Decisions

Settled. Do not re-open mid-sweep.

**N1. Direction decides the shape.** A client → server field carries a `Raw*`
type. A server → client field carries the *validated* domain type directly
(`Hue`, `Graphic`, `Serial`, `Skill`…) — the server does not send itself
hostile input, and a `Raw` on an outbound packet would be a lie about where the
check happened. Packets that go both ways (`0x3A` skills, `0xBF` subcommands)
follow the direction of the *struct*, not the id.

**N2. Validation lives on the seam, never in `decode_body`.** Decoding stays
what it is: byte shape only. Promotion is a named method called by the code
that acts on the packet — `dispatch.rs`, `openshard_login`, a tick system.

Three reasons this is the split and not the other one:

- A value outside its domain is a *gameplay* refusal, not a framing failure. It
  answers with a `0x82`, or is ignored, or is clamped with a log line. Making
  `decode_body` return `Err` would drop the connection instead
  ([Stage 6 amendment 1](protocol_rewrite.md#amendments-forced-by-the-stage-6-pilot-clientpacket-dispatchrs)),
  which is right for bytes that do not parse and wrong for a hue nobody offered.
- Most domains are not the protocol's to know. A skill id's meaning lives in
  `openshard_state::Skill`; a starting-stat cap lives in `[gameplay]` config; the
  set of legal hairstyles is Community Pack content. `common/protocol` is below
  all of them and must not learn any of it.
- It is the shape the crate already uses and likes: `RawCharacterName` →
  `CharacterName` through `Accounts::create_character`, `RawAccessLevel` →
  `AccessLevel` in `openshard_config`. See `identity.rs`'s module docs.

**N3. Four classes of field, and the class fixes the recipe.** A cheap agent
must never have to decide what "meaningful" means. It classifies, and the class
says what to write.

| class | what it is | the type | the promotion |
|---|---|---|---|
| **A — already named** | the value is a `Serial`/`Graphic`/`Hue`/`SoundId`/… and the server chose it | the existing newtype, no `Raw` | none needed |
| **B — total interpretation** | every bit pattern means something, including "something odd" | `RawX(pub u8)` | `fn interpret(self) -> X` — total, no `Result`; the leftover arm is an explicit `X::Unknown(n)` or a documented safe default |
| **C — fallible validation** | out-of-domain values exist and must be refused, clamped, or ignored | `RawX(pub u16)` | `fn validate(self, …) -> Result<X, InvalidX>` — the context argument is whatever the rule needs (a config cap, a list length) |
| **D — opaque, never read** | the client claims it and the server has no use for it | `RawX(pub u32)` with **no** promotion method | none, on purpose; the doc comment says "never trusted / never read", and the type is the record of that decision |

Class D is why the sweep can be mechanical: `client_ip`, client `flags`, echoed
constants all get a named type and *no* second step, so "no bare integer in a
packet struct" stays a rule an agent can satisfy without judgement, and a
reviewer can grep for `Raw` types with no promotion to find every field the
server is choosing to ignore.

**N4. Where a type lives.**

- A `Raw*` used by **one** module lives in that module, next to its packet.
- A `Raw*` used by **two or more** lives in `wire.rs`, next to `Hue`/`Graphic`.
- A **validated** type lives where its *rule* lives: in `protocol` when the
  client's own wire format fixes the domain (`Sex`, `Race`), in the server crate
  that owns the rule when the shard does (`StartingStats` and its cap,
  `openshard_state::Skill`).

**N5. Field visibility follows the invariant.** A `Raw*` carries no invariant,
so its field is `pub`, exactly as `RawCharacterName(pub String)` is. A validated
type that carries one keeps its field private behind a named
constructor/accessor pair, the way `Serial::new`/`Serial::raw` do. Never `From`,
`Into`, or `Deref` on either — CLAUDE.md, non-negotiable.

**N6. Promotion methods have two names and only two.** `interpret` for class B,
`validate` for class C. Uniform on purpose: an agent that invents
`classify`/`check`/`resolve`/`to_domain` per module produces a crate nobody can
grep. A promotion that reads several fields at once (starting stats are one
rule across three bytes) is a method on the *packet*, still named `validate`
with a qualifier: `CreateCharacter::validate_stats`.

**N7. Errors are typed, per promotion.** `InvalidHue`, `InvalidStartingStats`,
`InvalidSkillChoice`. No `String`, no shared `InvalidValue` catch-all, and
**not** `DecodeError` — these are not decode failures and must not be
convertible into one, or N2's split collapses back into a dropped connection.

**N8. The byte-level tests do not change.** D10 from the rewrite still holds:
every existing encode/decode test asserts the same bytes, only the value
constructing them gains a wrapper. A stage that cannot keep the bytes identical
has found a bug — fix it deliberately and say so in the commit.

**N9. Every stage adds the test that proves the split.** For each class C field
introduced, one test that an out-of-domain value **decodes cleanly** and is
**refused at promotion**. That pair is the whole point of the design; a stage
without it has added wrappers and no checks.

**N10. Coverage is counted, not assumed.** Each stage's commit message records
the bare-int-field count in the files it touched, before and after. The final
stage adds a repo-level check that counts them across the crate and asserts the
number is zero — or that every remaining one is on an explicit allowlist with a
reason. "No violations found" from a detector that examined nothing has been
green here before; a count cannot be.

The allowlist so far, each entry argued where it was decided:

| field | why it stays a bare integer |
|---|---|
| `world::Point::{x, y, z}` | components of one geometric quantity — [N1 amendment 2](#amendments-forced-by-n1-the-rest-of-worldrs) |
| `world::MapSize::{width, height}` | same |
| `mobile::Vitals::{current, max}` | components of one bar — [N2 amendment 2](#amendments-forced-by-n2-mobilers) |
| `mobile::MobileStatus::{strength, dexterity, intelligence, gold, armor, weight, max_weight, stat_cap, followers, followers_max}` | the status bar's quantities — [N2 amendment 3](#amendments-forced-by-n2-mobilers) |
| `containers::ContainedItem::amount` | a stack size: a quantity, by the `MobileStatus` argument — [N4 amendment 8](#amendments-forced-by-n4-containersrs) |
| `items::WorldItem::amount`, `items::PickUpItem::amount` | the same quantity, outbound and in — [N4 amendment 9](#amendments-forced-by-n4-itemsrs) |
| `vendor::BuyLine::price`, `vendor::SellLine::price` | gold: the `MobileStatus::gold` argument — [N5 amendment 1](#amendments-forced-by-n5-vendorrs) |
| `vendor::{Purchase, Sale, SellLine}::amount` | the same stack size, inbound and out — [N5 amendment 1](#amendments-forced-by-n5-vendorrs) |

`containers::ContainedItem::{x, y}` came *off* this list in N5: they are one
`GumpPoint` now, as [N4 amendment 6](#amendments-forced-by-n4-containersrs)
promised — [N5 amendment 6](#amendments-forced-by-n5-gumprs).

**N11. No compatibility shims.** Same as D9: a stage wraps a group of fields
**and** updates every call site in the same commit.

## The pilot: `0x00`/`0xF8` create character, and `0x5D` character play

`CreateCharacter` is the right first packet: it is entirely client → server, it
has one field of every class, it already carries a `Raw` type
(`RawCharacterName`) so the pattern has a foothold, and its seam
(`dispatch::create_character`) is where three real unchecked values currently
enter the world. `CharacterPlay` joins it — two fields, and it proves a `Raw`
type is shared across packets rather than invented per struct (N4).

Field by field, as decided. `world.rs` unless noted.

| field | wire | class | type | promotion, and where |
|---|---|---|---|---|
| `name` | 30 bytes | C | `RawCharacterName` *(exists)* | `Accounts::create_character` *(exists)* |
| `flags` | `u32` | D | `ClientFlags` | none — never read |
| `profession` | `u8` | B | `RawProfession` | `interpret() -> Profession { Custom, Predefined(u8) }`, in `protocol`. **Do not invent the `prof.txt` table** — the only distinction the wire fixes is "0 means the advanced/custom option"; naming the professions is Community Pack content |
| `sex_race` | `u8` | B | `RawSexRace` | `interpret() -> (Sex, Race)`, in `protocol`. Replaces `is_female()`/`race()`; `body()` takes the interpreted pair. Keep the existing doc note that the SA encoding is assumed |
| `strength`, `dexterity`, `intelligence` | `u8`×3 | C | `RawStatValue` | `CreateCharacter::validate_stats(caps) -> Result<StartingStats, InvalidStartingStats>` — **one rule across three bytes** (per-stat floor/ceiling and the total). Lives in the server crate that reads `[gameplay]`, not in `protocol`. **This check does not exist today.** |
| `skills[].skill` | `u8` | C | `RawSkillId` | `openshard_state::Skill::from_id` *(exists, returns `Option`)*, at the seam |
| `skills[].value` | `u8` | C | `RawSkillValue` | validated against the shard's starting cap at the seam. **Does not exist today** — `value * 10` is applied to whatever arrived |
| `skin_hue`, `hair_hue`, `beard_hue`, `shirt_hue`, `pants_hue` | `u16`×5 | C | `RawHue` → `wire.rs` | `validate(&allowed) -> Result<Hue, InvalidHue>`. The allowed set is content, so it lives above `protocol`. **Does not exist today** |
| `hair`, `beard` | `u16`×2 | C | `RawGraphic` → `wire.rs` | `validate(&allowed) -> Result<Graphic, InvalidGraphic>`; allowed hairstyles are content |
| `start_location` | `u8` | C | `RawStartLocationIndex` | validated against `login.starts.len()`. Behaviour is unchanged — out of range still falls back to the default facet — but the fallback becomes a named branch on a `Result` instead of a `None` from `.get()` |
| `slot` | `u32` | D→C | `RawCharacterSlot` → `wire.rs` | none in the pilot: `create_character` fills the first free slot and ignores the client's pick. Document that as class D with a note; it becomes class C if slot choice is ever honoured |
| `CharacterPlay::name` | 30 bytes | C | `RawCharacterName` | existing lookup path |
| `CharacterPlay::slot` | `u32` | C | `RawCharacterSlot` | validated against the account's character count at the seam |
| `CharacterPlay::client_ip`, `CreateCharacter`'s claimed ip | `u32` | D | `RawClientIp` → `wire.rs` | none — never trusted, never read |

The pilot is done by hand, not by an agent, and it ends by writing an
"Amendments forced by the pilot" section below. Three of its rows are checks
the server is missing today; each one lands with a test that a hostile value
reaches the seam and is refused there.

## Amendments forced by the pilot

The pilot landed classes A, B and D in full — every row of the field table
above now has its named type — and class C's *type* half only: every
class-C field is a `Raw*` newtype wired through decode, encode and the seam,
but none of the three promotion methods the field table calls "does not exist
today" (`validate_stats`, the skill-value check, the hue/graphic allowlist)
were written. Each one needs a real gameplay-balance number — a starting stat
total and per-stat floor/ceiling, a starting skill-point budget, a set of
hairstyles/hues this shard actually allows — and none of those numbers exist
anywhere in this repo yet. Inventing them here would be a content decision, not
a mechanical refactor, so they are left as the concrete next step rather than
guessed at.

1. **Every class-C field's `.0` is unwrapped at the seam
   (`dispatch::create_character`) with a comment naming it as an unchecked
   pass-through.** This is deliberately worse-looking than the old bare `u16`
   it replaces — the point is that it is now *visible* and grep-able
   (`Raw` with no matching `validate`/`interpret` call at its one call site),
   where before the same gap was invisible. N9's test pair (decodes cleanly /
   refused at promotion) has nothing to attach to until a promotion method
   exists, so none were added this stage; the next stage that adds
   `validate_stats`, the skill check, or the hue/graphic check owes N9's pair
   for that field specifically.
2. **`CharacterPlay::name` moved to `RawCharacterName` even though it is not
   one of the three "missing check" rows.** `dispatch_world_packet` was
   building a `CharacterName` straight from a bare `String` with no type
   marking it as unchecked client input — the exact invisibility N3 exists to
   remove — so it got the same treatment as `CreateCharacter::name`, at no
   cost: the promotion is unchanged, `roster.get(&account, &name)` is still
   the check (a name nobody has is not an account's character, and the seam
   already handles that by falling back to a fresh spawn).
3. **`RawSexRace::interpret` and `CreateCharacter::body` are both `pub const
   fn`, matching the methods they replace (`is_female`, `race`, `body`).** No
   behaviour changed; `body` moved from a method reading `self.sex_race` twice
   (once through each of `is_female`/`race`) to an associated function taking
   the already-interpreted `(Sex, Race)` pair once, exactly as the field
   table's pilot row specifies.
4. **`ClientFlags`, `RawStatValue`, `RawSkillId`, `RawSkillValue` and
   `RawStartLocationIndex` all live in `world.rs`, not `wire.rs`** — each is
   used by exactly one packet, so N4's "one module" branch applies. Only
   `RawHue`, `RawGraphic`, `RawCharacterSlot` and `RawClientIp` went to
   `wire.rs`, matching the field table's own `→ wire.rs` column exactly.
5. **Bare-integer field count in `world.rs`: 39 before, 20 after** (N10) — the
   19 fields the pilot's two packets own (15 on `CreateCharacter`, 2 on
   `SkillChoice`, 2 on `CharacterPlay`) all gained a named type. The remaining
   20 belong to packets N1 (the rest of `world.rs`) has not touched yet.

## Amendments forced by N1 (the rest of `world.rs`)

N1 is entirely class A, B and D: the module's remaining packets are the outbound
entry sequence plus `0x02`, and the one inbound packet's two fields are an echo
and a value nobody reads. No class C field appeared, so N9's test pair is not
owed by this stage — what it added instead is a test per class-B promotion that
the promotion is *total*.

1. **Class B can be degenerate, and the walk sequence is.** `RawStepSequence::
   interpret` returns a structurally identical `StepSequence`; every one of the
   256 bytes maps to itself. That is not ceremony to be optimised away: the
   sequence is an **echo tag** — the client owns the number, the server sends it
   back so an ack can be matched to the step that asked for it — so the type
   pair records provenance, which is the only thing that differs between the two
   ends. There *is* a rule (a fresh connection must open at zero, a wrap skips
   it), it lives in `openshard_movement::WalkSequence::accept`, and it refuses
   the **step**, not the value: a `0x21` names the very sequence it is
   rejecting, so the reject echoes a byte the rule declined. N5's gump and
   button ids are the mirror image — server-chosen, echoed *by the client* — and
   are class C ("is this one I offered"), not this.
2. **N10 gains an allowlist, and its first entries are geometric.** `Point`'s
   `x`/`y`/`z` and `MapSize`'s `width`/`height` stay bare integers *by
   decision*: the struct is the named type, nothing reaches a component except
   through it, and the components are the one thing that is genuinely a number —
   they get added to, compared and clamped, in movement, sectors, pathfinding
   and line of sight. Wrapping them buys no confusion that the enclosing type
   does not already prevent, and costs a `.0` on every arithmetic site in the
   server. Reason recorded here so N8's counter can assert five, not zero, for
   this file.
3. **`map_width`/`map_height` became one `MapSize`.** Both call sites read the
   two together and both packets carry both halves; a client told a width
   without its height draws the edge of the world in the wrong place. The old
   `DEFAULT_MAP_WIDTH`/`DEFAULT_MAP_HEIGHT` pair became `MapSize::BRITANNIA`,
   which is what they always meant.
4. **A packet was renamed to free a name for its value: `Season` →
   `SeasonChange`.** The five seasons are a real domain — the client draws
   exactly five and nothing else — so `Season` had to become the enum, and the
   packet took ServUO's own name for it, which its doc comment already cited.
   The `ServerPacket` variant moved with it. `Season::from_bits` is total, with
   the same "fall back to what the client can always draw" argument as
   `Notoriety::from_bits`; `openshard_config` still refuses a sixth season at
   startup, so the fallback is for scripts and foreign saves, not for config.
5. **The stage reached into `mobile.rs` for one shared type: `StatusFlags`.** It
   was `pub type StatusFlags = u8`, an alias — which passes a bare-integer count
   while being exactly the invisibility N3 exists to remove — and `PlayerUpdate`
   needed it. It is now a newtype with a `NONE` constant, and it stayed in
   `mobile.rs` rather than moving to `wire.rs`: N4's `wire.rs` rule is written
   for `Raw*` types, and a *validated* type lives where its rule lives, which for
   a mobile's status bits is with the mobile packets. Same argument kept
   `Notoriety` where it is while `WalkAck` began using it.
6. **`WalkAck::notoriety` now goes out through `Notoriety::for_client`,** as
   `0x77` and `0x78` already did. Bytes are unchanged for every value this shard
   currently sends (`Innocent`, `0x01`); what changed is that a yellow bar can no
   longer reach a pre-4.0.0 client, which would have drawn the player's own
   health bar as nothing at all. `NOTORIETY_INNOCENT`, the loose `u8` constant in
   `tick/defaults.rs`, is gone.
7. **One real bug fell out of `Serial`.** `WorldState::teleport` built its
   `0x20` with `serial_of(entity).map_or(0, |s| s.raw())` — zero is not a
   serial, it is the wire's word for "no object", and `Serial::new` refuses it.
   The serial now joins the body and the facing in the `if let`, so a client
   whose entity has no serial gets no packet instead of a nonsense one.
8. **Bare-integer field count in `world.rs`: 20 before, 5 after** (N10), the
   five being the allowlisted geometric components in amendment 2. `mobile.rs`
   is unchanged at 37 — the `StatusFlags` alias was not one of them.

## Amendments forced by N2 (`mobile.rs`)

The direction rule paid for itself exactly as N2's stage line predicted: seven of
the module's serials, both body graphics and three hues are class A, and wrapping
them **deleted** code — ten `.raw()` calls and four `.0`s vanished from the
server, because the call sites already held a `Serial`, a `Graphic` and a `Hue`
and were unwrapping them to satisfy a `u32`. The stage's real content is the two
inbound packets and one question the class table does not answer.

1. **`RawSerial` is the pattern for every inbound object reference, and it
   returns `Option`, not `Result`.** `LookRequest` is the sweep's first client-
   chosen serial and there will be one in `items.rs`, `target.rs`, `vendor.rs`
   and `context.rs`, so the type went into `serial.rs` beside the rule rather
   than into the packet that first needed it. Its promotion is
   `validate(self) -> Option<Serial>`, wrapping the existing `Serial::new`: N7
   asks for typed errors, but the two values a client actually sends here — `0`
   and `0xFFFF_FFFF` — are the wire's own words for *no object*, which is an
   answer and not a malformed packet. An `InvalidSerial` would make every seam
   handle an error where all but one of them want to do nothing. This is the same
   licence the pilot took with `Skill::from_id`'s `Option`, written down.
2. **A current/max pair is one field: `Vitals`.** `MobileStatus` carried
   `hits`/`hits_max`, `stamina`/`stamina_max`, `mana`/`mana_max` — six bare
   `u16`s of which every source produces both halves at once (`Hitpoints`,
   `Mana`, `Stamina` each hold the pair) and the client draws a *ratio*, so half a
   pair is not a smaller number but a bar of the wrong length. The `MapSize`
   argument of N1 amendment 3, applied three times. `weight`/`max_weight` looks
   like a fourth pair and is not: the two come from different places (what is
   carried, versus a function of strength), so they stay separate fields.
3. **The status bar's ten remaining numbers stay bare, by decision.** `strength`,
   `dexterity`, `intelligence`, `gold`, `armor`, `weight`, `max_weight`,
   `stat_cap`, `followers`, `followers_max` are the case N1 amendment 2 opened
   for `Point`: they are genuinely numbers — added to, compared and clamped on
   every blow, every regeneration tick and every item picked up — and their rules
   (the caps, the carry limit, the training curve) live in `skills`, `items` and
   `[gameplay]` config, all far above `protocol`. Ten newtypes here would be ten
   types that only ever unwrap, and the packet's named fields already prevent the
   confusion a newtype would. They are on N10's allowlist, so the count for this
   file asserts twelve and not zero.
4. **Class C appeared where the plan expected only A, B and D**, and both of its
   fields are on the same packet: `0xBF 0x1A`'s `stat` and `lock`. `RawStat::
   validate -> Result<Stat, InvalidStat>` is a real refusal — the status bar has
   exactly three arrows — and it moved a `_ => return` out of
   `World::set_stat_lock` and into `dispatch.rs`, which now logs the byte it
   dropped. N9's pair is there: a `0xBF 0x1A` naming stat 3 decodes cleanly and
   is refused at promotion.
5. **A decoder that rewrote a value was the stage's one real finding.**
   `StatLockRequest::decode_body` folded `lock > 2` to `0` *while decoding* —
   ServUO's behaviour, in the wrong place: after it, nothing downstream could
   tell the `0` a client sent from the `0x63` it did not, so a log line about a
   nonsense arrow was impossible to write. The fold is now
   `RawStatLock::interpret`, class B and total, with a test that all 256 bytes
   interpret and that the byte survives decoding unchanged.
6. **Three-valued arrows are one type, bridged by name.** `StatLockBits`'s three
   `u8`s became `skill::SkillLock` — its own doc already called it "the mirror of
   `SkillLock`", so a second three-way enum was never needed. `openshard_state`
   keeps its separate `StatLock` (its gain path is not the skill one) and gained
   `to_wire`/`from_wire`, both directions named, no `From`. `StatLock::from_bits`
   stays, now documented as the *saved* byte's reader — a save written by an
   older build may hold anything, which is a different problem from a packet.
7. **`Layer` went to `wire.rs` although only one module uses it today.** N4's
   "two or more modules" rule is written for `Raw*` types; a validated type lives
   where its rule lives, and a layer's rule is the client's alone. Both packet
   modules that carry one — this module's `0x78` outfit list and `items.rs`'s
   `0x2E`/`0x13` — would otherwise have to import it from each other. It is a
   named byte and not an enum, for `StatusFlags`' reason: the twenty-odd layers
   this engine has never sent would be a guess. `openshard_state::Equipped.layer`
   stays a `u8` — that is a component, not a packet field, and N4's stage is
   where it becomes one question rather than two.
8. **`PaperdollFlags` replaced two loose `pub const u8`s** (`PAPERDOLL_WARMODE`,
   `PAPERDOLL_CAN_LIFT`), with a named `with` rather than a `BitOr` impl: an
   operator on a newtype is the same invisible coercion `Deref` is.
9. **One byte-level test changed its input, deliberately (N8).**
   `remove_is_five_bytes` built its `0x1D` with serial `0xDEAD_BEEF`, which is
   past the item pool and refused by `Serial::new` — an unaddressable serial the
   old bare `u32` let through. It now uses `0x4EAD_BEEF` and asserts the same
   shape: five bytes, the serial big-endian. Every other assertion in the crate
   is byte-for-byte what it was.
10. **Bare-integer field count in `mobile.rs`: 37 before, 12 after** (N10), the
    twelve being amendments 2 and 3's allowlisted quantities. `wire.rs` and
    `serial.rs` gained a type each and no bare fields.

## Amendments forced by N3 (`speech.rs`)

The module the plan called five packets is really one *header* sent five times —
mode, hue, font, and (outbound) a speaker — so the stage is where N1's direction
rule met the same field going both ways. It produced the sweep's first genuinely
shared class-B type and its second decoder-rewrites-a-value finding.

1. **`TalkMode` is an enum with a leftover arm, where `Layer` and `StatusFlags`
   are named bytes.** N2 amendment 7 argued a byte with a name beats an enum
   when the unnamed values would be a guess, and the modes look like that case —
   ServUO's `MessageType` has a dozen this engine has never sent. The difference
   is that something already *branches* on this byte: `speech_range` has always
   asked "whisper, yell, or neither", and the answer decides who hears it. A
   named byte cannot be matched exhaustively, so the branch stays a `_ =>` with
   no compiler behind it. Five variants are named — the ones this repo's own doc
   comments already name — and `Other(u8)` carries the rest, which is exactly
   what N3's class-B row prescribes. Nothing was guessed: `Other` is the record
   that the meaning is unknown, not a claim it has none.
2. **Three modules had each named the same domain, and none of them knew.**
   `mobile::LABEL_MODE`, `chat::TALKMODE_WHISPER` and `chat::TALKMODE_YELL` were
   loose `pub const u8`s in two crates, and `chat::DEFAULT_FONT` a third — the
   `PaperdollFlags` situation of N2 amendment 8, spread across a crate boundary.
   They are `TalkMode::Label`/`Whisper`/`Yell` and `Font::DEFAULT` now. A domain
   named in three places is a domain with no type; that a bare `u8` *travels*
   between crates is what let it happen.
3. **`0xAD`'s decoder rewrote the mode byte, and this is the second time.**
   `decode_body` stored `mode & !0xC0` — the keyword bits gone before anything
   downstream could see them, so the `0x00` a client sent and the `0xC0` it did
   not were indistinguishable, exactly `StatLockRequest`'s finding (N2 amendment
   5). The distinction here is sharper than that one, because the bits *are*
   framing: the decoder legitimately reads them to know which of two text shapes
   follows. So the read stayed (`RawTalkMode::has_keywords`, private, framing
   only) and the *fold* moved to `RawTalkMode::interpret`. Two findings of the
   same shape in two stages says this is a pattern to look for and not an
   accident: **wherever a decoder normalises, the raw byte is being destroyed.**
4. **A packet can have its own sentinel, and speech's is not the wire's usual
   one.** `serial`/`graphic` became `Option<Serial>`/`Option<Graphic>`, but the
   absent case encodes as `0xFFFF_FFFF`/`0xFFFF` — ServUO's `Serial.MinusOne` —
   and **not** as `serial::raw_or_none`'s `0`. Reusing the shared helper would
   have compiled, changed the bytes, and told the client the words came from an
   object it does not have; it would draw them nowhere. So `speech.rs` keeps a
   private `serial_or_system`/`graphic_or_none` pair beside its own constants.
   The lesson for later stages: `Option<Serial>` names the *shape* of a field,
   never the value it goes out as — check the packet's own sentinel every time.
5. **The same `map_or(0, …)` bug as N1 amendment 7, fixed the other way.** Both
   `private_overhead_cliloc` and `private_overhead_text` built their serial with
   `serial_of(source).map_or(0, |s| s.raw())`, and zero is not a serial. `0x20`'s
   fix was to send no packet; these send the line as a *system* message instead,
   because the text is feedback the watcher asked for (Item Identification saying
   what an item turned out to be) and a line drawn in the corner beats no line at
   all. Which way a nonsense serial degrades is the packet's question, not a rule
   the sweep can settle once.
6. **`ClilocId` went to `wire.rs` and `Font` stayed in `speech.rs`,** both by
   N4's counting rule as N2 amendment 7 read it for validated types: five modules
   carry a cliloc (`speech`, `context`, `properties`, `gump`, `login`) and one
   carries a font. Only `speech.rs`'s cliloc field was converted — the other four
   are their own stages' work, exactly as `Layer` landed in `wire.rs` for
   `mobile.rs` and left `items.rs` for N4.
7. **`WorldState::localized_message` keeps a bare `u32`, citing `play_sound`.**
   Carrying `ClilocId` up through it would have touched ~190 call sites across
   `skills`, `crafting`, `items` and `world`, every one a ported ServUO message
   *number* out of a table. `play_sound` already made and documented this
   decision for `SoundId` (`runtime.rs`), and the reasoning transfers whole: the
   newtype starts where the packet is built, nothing above unwraps one, and
   converting the tables is its own sweep. Recorded rather than done, so the next
   reader finds a decision and not an oversight.
8. **Class C appeared, and its promotions are the pilot's deferral again.** A
   client's `hue` and `font` are checked against sets that are content — which
   hues this shard allows, which faces the client has — and neither exists in the
   repo. They arrive as `RawHue`/`RawFont` and are unwrapped at
   `World::say` with the comment naming them unchecked, exactly as pilot
   amendment 1 established. N9's test pair is owed by whichever stage writes
   those checks, not by this one. `mode` is class B and *does* promote, so its
   totality test is here.
9. **The raw types reached the world's `Command` enum for the first time.**
   `Command::Say` carries `RawTalkMode`/`RawHue`/`RawFont` from `dispatch.rs`
   through the queue to `World::say`, which is the seam — the command queue is
   not a checkpoint, it is a delivery, and pretending otherwise would have put
   the promotion on whatever thread Tokio picked. `Command::Speak`, which a
   script raises, takes a validated `Hue`: the script bridge is a serialization
   seam like SQL or the wire, and that is where the JSON number becomes a type.
10. **Bare-integer field count in `speech.rs`: 22 before, 0 after** (N10) —
    nothing on the allowlist, the first module in the sweep to reach zero.
    `wire.rs` gained `ClilocId` and `mobile.rs` lost `LABEL_MODE`; neither file's
    count moved.

### Backlog from this stage

- **The cliloc-table sweep** (amendment 7): ~190 call sites pass a bare `u32`
  message id. Worth doing with the `SoundId` table sweep, which has the same
  shape and the same blocker — both want the numbers to come out of config
  already typed, which drags serde into `protocol`.
- **`0x03B2` is written out five times**: `gm::SYSTEM_HUE`, `npc::GREET_HUE`,
  `quests::progress::NPC_HUE`, `runtime::SYSTEM_HUE`,
  `tick::defaults::TEXT_HUE`. They are all "the client's muted grey", all four
  crates deep, and they are now five `Hue` constants rather than five `u16`s —
  which makes the duplication visible but not gone. Where a shard-wide default
  hue *lives* is a `[gameplay]`-config question this stage did not open.

## Amendments forced by N4 (`containers.rs`)

The stage's two packets are a two-line inbound `0x06` and an outbound item
record sent three ways, and between them they raised the sweep's first
*component* question — how far up a validated type travels once the packet
below it has one — and its first packet whose inbound field is not a value at
all but a flag riding on one.

1. **A `0x06`'s serial is a serial *and* a flag, and the split is the packet's,
   not the tick's.** Bit 31 is the client's paperdoll request — ServUO's
   `UseReq` routes it straight to `OnPaperdollRequest` and never to `Use`, and
   treating both alike is the bug where relogging mounted dismounted you a
   breath later. That knowledge lived in `tick.rs` as `serial & 0x8000_0000`,
   which is a rule in the file [architecture.md](architecture.md) says holds no
   rules. It is now `DoubleClick::interpret -> UseRequest`, class B and total:
   every one of the 2³² values is a paperdoll request or a use, and **both arms
   carry a [`RawSerial`]**, because stripping a flag bit does not make what is
   left address anything. The validation stays where N2 puts it, at the seam.
2. **A packet-level `interpret` may run at the network seam; a `validate` still
   may not.** `dispatch.rs` calls `interpret` and queues a `Command::DoubleClick
   { request: UseRequest }`. This does not contradict N3 amendment 9's "the
   queue is a delivery, not a checkpoint": a total interpretation cannot refuse
   anything, so running it early costs nothing and cannot drop a client's
   request on Tokio's thread. What crossed the queue is still raw, and
   `RawSerial::validate` runs in the tick.
3. **Wrapping deleted three guards.** `items::double_click`,
   `items::paperdoll_request`, `items::mobile_used` and `npc::open_shop` each
   opened with `Serial::new(serial)` and each now takes a `Serial`; the tick's
   arm validates **once** where it used to re-derive the same `Option` five
   times over. N2's amendment 1 result, in the other direction: there the
   outbound types deleted `.raw()` calls, here the inbound one deleted repeated
   checks.
4. **A validated type stops where the packet is built, and the component below
   it keeps its bare integer — for now.** `ContainedItem` gained `Serial`,
   `Graphic`, `Hue` and `GridSlot`, but `openshard_state::Contained.grid`,
   `Container.gump` and `components::Graphic`'s `id`/`hue` are all still bare.
   This is N3 amendment 7's `localized_message` decision applied to components
   rather than to a table: `Contained.grid` alone reaches the persistence
   record, both stores' SQL and a dozen test fixtures, and converting it is a
   sweep with its own shape — the newtype starts where the packet is built and
   nothing above unwraps one. Recorded rather than done. The exception the doc
   already promised, `Equipped.layer`, is N4's `items.rs` half and is decided
   there.
5. **`GridSlot` is a named byte, not an index type.** Same argument as `Layer`
   (N2 amendment 7): the grid's size is the client's, this engine has never had
   a reason to learn it, and a range check would be a guess. What the type buys
   is that the three `u16`s beside it on the record can no longer be handed to
   it.
6. **`x`/`y` on a container record stay bare, and the reason is a stage
   boundary.** They are the item's column and row in the gump — a pair, read
   and written together, which by N1 amendment 3 and N2 amendment 2 asks to
   become one named type. It is not made here: a gump coordinate is exactly
   what `gump.rs` carries, that module is N5's, and a `GumpPoint` invented in
   this stage is a name the next stage would have to either adopt or contradict.
   On N10's allowlist with that reason, and in N5's backlog.
7. **Two magic gump ids became constants.** `0xFFFF` — what makes a `0x24` draw
   a book rather than a bag — is `containers::BOOK_GUMP`, beside the packet
   whose behaviour it changes; `npc`'s `SHOP_GUMP` was already named and is now
   a `Graphic`.
8. **Bare-integer field count in `containers.rs`: 9 before, 3 after** (N10), the
   three being amendment 6's `x`/`y` and the stack `amount`, which is a
   quantity by N2 amendment 3's argument — added to, split and compared, with
   its rules in `items` far above `protocol`.

## Amendments forced by N4 (`items.rs`)

The module is the sweep's first genuinely two-directional one — the same item
drawn outbound and named inbound — and it is where `Layer`, parked by
[N2 amendment 7](#amendments-forced-by-n2-mobilers), had to become one answer
rather than two.

1. **`Equipped.layer` is a `Layer`, and that is where the component sweep stops
   this stage.** N2 left the question open; the answer is yes, and the reason is
   not symmetry with the packet but that *every* rule reading it is naming a
   slot, never doing arithmetic: what a corpse keeps, what armour covers, what
   may not be lifted, which hand a weapon is in. The type carried outward from
   there through `state::armor`, `state::weapon`, `combat`, `npc` and `world`
   with no `.0` except at the two seams that are supposed to have one — the
   persistence record's `u8` and the script bridge's JSON number. Contrast
   `Contained.grid` and `components::Graphic` in
   [containers amendment 4](#amendments-forced-by-n4-containersrs), which stayed
   bare: those are read as *numbers* nowhere either, but nothing in a packet
   forced the question, and a sweep with no forcing packet is its own stage.
2. **`RawLayer` lives in `wire.rs`, beside its twin, against N4's own counting
   rule.** Only `0x13` carries an inbound layer, so N4 would put it in
   `items.rs`. Every other `Raw*` in the crate sits beside the validated type it
   promotes to — `RawHue` beside `Hue`, `RawSerial` beside `Serial` — and a pair
   split across two modules is a pair the next reader has to be told about. The
   counting rule is for `Raw*` types with **no** twin (`RawStatValue`,
   `RawStartLocationIndex`); where there is one, the twin's home wins.
3. **`RawLayer::interpret` is degenerate, and deliberately so.** The second
   `RawStepSequence` (N1 amendment 1): a layer is a *name*, not a range — N2
   amendment 7 settled that — so every one of the 256 bytes interprets, and what
   the pair records is provenance. The refusal that does exist is a gameplay
   one and stayed where it was: `equip_item` still rejects layer `0` and
   anything past `MAX_WEARABLE_LAYER`, now stated in `Layer`s.
4. **`DROP_TO_GROUND` is a `RawSerial` constant, and `to_ground` compares
   against it rather than asking `validate`.** N3 amendment 4's lesson, met from
   the other side: `RawSerial::validate` answers `None` for `0xFFFFFFFF` *and*
   for `0`, but a `0` container is a confused client and `0xFFFFFFFF` is the
   floor. Folding the two would have compiled and silently turned every
   malformed drop into a ground drop.
5. **`BACKPACK_LAYER` was written out five times in two crates.** `world`'s
   `gm.rs`, `travel.rs`, `spells.rs` and `tick/defaults.rs` each declared their
   own `0x15`, and `npc/vendor.rs` a fifth, while `openshard_items` had the
   canonical one all along. Exactly N3 amendment 2's finding (`TALKMODE_WHISPER`
   in two crates) and N2 amendment 8's (`PAPERDOLL_WARMODE`), for the third
   time: **a bare integer that travels between crates gets re-declared at each
   stop.** The four copies are gone; there is one `Layer`.
6. **The paperdoll layers scattered as loose `pub const u8` are all `Layer`
   now** — `state::armor`'s seven coverage layers, `state::weapon`'s two hands,
   `npc::dress`'s seven garment slots, `items`' backpack/bank/mount/trade, and
   `world`'s corpse robe. `layer_coverage` and `hit_layer` take and return one,
   which is what stopped `hit_layer`'s roll and its layer from being the same
   type.
7. **`Terrain::item_layer` keeps its byte, wrapped at one call site.** The trait
   lives in `openshard_movement`, which is below `protocol`, and it reads the
   quality byte out of `tiledata.mul`. `skills::appraise::tiledata_layer` is the
   single place the byte meets a `Layer`, and it names the wrap. `weapon_layer`
   above it takes and returns `Layer`s.
8. **`WorldItem`'s stack-amount bit still masks a serial it cannot need to.**
   `serial & 0x7FFF_FFFF` in the unstacked branch is now provably a no-op —
   `Serial` cannot be built above the item pool — and it stayed, with a comment
   saying so. Removing it would make the encoder depend on `Serial`'s invariant
   at a distance for no byte saved.
9. **Bare-integer field count in `items.rs`: 16 before, 3 after** (N10). The
   three are `WorldItem::amount`, `PickUpItem::amount` and `Point`'s components
   inside `position`, all already allowlisted quantities;
   `PickUpItem::amount` joins the list explicitly because it is the first
   *client-supplied* one, and its check — is there that much in the stack —
   exists today in `items::pick_up`.

### Backlog from this stage

- **The component sweep.** `Contained.{x, y, grid}`, `Container.gump` and
  `components::Graphic.{id, hue}` are the bare integers directly under the
  packets this stage typed, and each is one `Layer`-sized job: `grid` and `gump`
  reach the persistence record and both stores' SQL, `Graphic` reaches most of
  the server. Worth doing as its own stage after N8, with the cliloc and
  `SoundId` table sweeps N3 left — they share the blocker, which is that the
  numbers should arrive from config already typed.
- **`GumpPoint` for N5.** Three modules now carry an `x`/`y` pair that is a
  *gump* coordinate rather than a world one: `containers::ContainedItem`,
  `gump::GumpDisplay`, and `Command::ShowGump`. N5 owns `gump.rs` and should
  name the type; `containers.rs`'s two fields join it then and come off the
  allowlist.
- **`state::Graphic` and `wire::Graphic` collide by name**, so four files now
  spell one of them out in full (`openshard_protocol::wire::Graphic(id)`) and
  `runtime.rs` imports it `as WireGraphic`. Neither name is wrong — one is the
  component an item is *drawn* by, the other the id on the wire — but three
  spellings of the same conversion across the server is a smell. Renaming the
  component (`Drawn`? `Art`?) is a `state` question, not a protocol one.

## Amendments forced by N5 (`vendor.rs`)

The module is four packets in two mirrored pairs, and N1's direction rule sorted
them with nothing left over: the two lists the server draws are class A, the two
replies are `RawSerial`. Its content is the *quantities*, which are the first
fields in the sweep to go on N10's allowlist because of what they are rather
than where their type would live.

1. **A price and an amount are quantities, and they stay bare.** `BuyLine::
   price`, `SellLine::{amount, price}`, `Purchase::amount` and `Sale::amount`
   are N2 amendment 3's case exactly: multiplied into a total, compared against
   what a purse holds, split off a stack — and their rules (what a vendor
   charges, what half price is, how much is on the shelf) live in
   `openshard_npc` and `openshard_items`, far above `protocol`. `Purchase::
   amount` is client-supplied and still bare, on `PickUpItem::amount`'s
   precedent (N4 items amendment 9): the check that matters is "is there that
   much", it exists in `vendor::buy` as `have.min(purchase.amount)`, and a
   newtype would not be it.
2. **A decoder that reads a byte and drops it is not the N2/N3 finding.**
   `BuyReply::decode_body` branches on `0x02` and keeps nothing. The two earlier
   findings (`StatLockRequest`, `0xAD`) *stored* a folded value, so the client's
   own byte was gone; here the byte is framing — it says whether a list
   follows — and the two answers it separates, "closed" and "bought nothing",
   are the same empty basket to everything downstream. The distinction is
   written in a comment beside it, because the shape looks identical at a
   glance. **What makes a normalising decoder a bug is that something
   downstream can no longer tell two inputs apart**; where nothing downstream
   cares, there is nothing to preserve.
3. **Wrapping deleted four `Serial::new` guards and two `.raw()` calls** —
   N2 amendment 1 and N4 containers amendment 3, in one module and both
   directions at once.
4. **Bare-integer field count in `vendor.rs`: 14 before, 5 after** (N10), the
   five being amendment 1's quantities.

## Amendments forced by N5 (`context.rs`)

1. **The tag is class C and its promotion is a `Result`.** A `0x15` echoes the
   entry's position in the list the `0x14` drew, so the count of entries is the
   whole domain: `RawContextMenuIndex::validate(offered)`. Unlike
   `RawSerial::validate`'s `Option` (N2 amendment 1) there is no wire value here
   that *means* "no entry", so every rejection is a refusal worth logging, and
   the error carries the tag and the count to log. The check itself is not new —
   `entries.get(index)` was doing it — but it was silent, and it could not be
   skipped by accident before only because one call site happened to be careful.
2. **`ContextMenuFlags` is a named byte, not an enum**, for `Layer`'s reason (N2
   amendment 7): ServUO's `CMEFlags` has a dozen bits this engine has never set.
3. **`ClilocId` reached its second module**, as N3 amendment 6 said the four
   remaining carriers would. The cliloc *constants* in `tick/context.rs` are
   typed with it; the ~190-call-site table sweep N3 recorded is still open.
4. **Bare-integer field count in `context.rs`: 6 before, 0 after** (N10).

## Amendments forced by N5 (`gump.rs`)

The stage the plan ordered N5 for. Six windows answer through one packet, and
every number in it is one the server chose — which makes this the module where
"is this one I offered" had to become three different checks rather than one.

1. **`RawGumpId::validate` takes a *list* and answers `Option`.** The list
   because the quest system draws two windows and claims a reply for either; the
   `Option` because the router asks each handler in turn and four of the five
   legitimately answer "not mine". A typed error would be an error nobody could
   act on. This is N2 amendment 1's licence extended from "the wire has a word
   for nothing" to "not-mine is an answer this control flow depends on" — and
   the reply that matches *no* engine dialog is not refused at all: it belongs to
   the script pack and is forwarded.
2. **A button id is class B, which the field table did not predict.**
   `RawButtonId::interpret -> GumpAnswer { Closed, Pressed(ButtonId) }` is
   `DoubleClick::interpret`'s shape (N4 containers amendment 1): one field
   carrying a value *and* an answer. The close box is `0`, and it was being
   compared against by hand in three handlers; `crafting::decode_button`'s own
   `if id == 0 { return None }` guard is gone with them, which is the third time
   in this sweep that wrapping a field deleted a guard.
3. **Two layouts deliberately give a button the close box's id, and that had to
   survive.** ServUO's `Buttons.Close = 0` (the quest window's `X`) and
   `CraftGumpItem`'s Back button both send `0`, so dismissing those windows and
   pressing their own button are the same answer *by construction* — pressing
   Back and closing the craft detail page both return to the list, in ServUO and
   here. The refactor's temptation was to treat `Closed` as "do nothing", which
   would have quietly changed both. They are now `ButtonId::CLOSE_BOX` constants
   with the collision stated, rather than a `0` that reads like a coincidence.
   `ButtonId::UNUSED` is the same value again with a third meaning — what a
   `Page` button writes where a reply button writes its id — and has its own
   name for the same reason.
4. **Whether a button was *offered* stays in each handler's `match`.** There is
   no list to check it against: the craft window's ids are computed
   (`1 + kind + index * 7`), the quest log's are a table plus a row offset, the
   runebook's are five ranges. So the sweep names the encodings instead —
   `quests::gump::{row_button, row_of}` and `travel::book_button`, both
   directions with names, the `to_wire`/`from_wire` shape of N2 amendment 6 —
   and the arithmetic stops being open code at five call sites.
5. **`RawSwitchId::validate` takes a count, because a radio group is its rows
   numbered from zero.** Both groups this engine draws are; the group's length
   is the one thing a handler still has when the reply arrives. The moongate
   list was already checking with `.get`; the resign dialog was not, and its
   `switches.contains(&YES)` would have accepted any id the client invented as
   long as one of them was `1`.
6. **`GumpPoint` closes N4's backlog item, and the wire widths differ.**
   `GumpDisplay`, `Command::ShowGump` and `containers::ContainedItem` all
   carried a loose `x`/`y` pair in *gump* space; they share one type now and
   `ContainedItem`'s two fields come off N10's allowlist. The two are measured
   from different origins (a window from the screen, an icon from the container
   art's corner) and go out four bytes wide and two — neither of which makes
   them different quantities, any more than a `Serial` stops being one where a
   packet writes it short. Signed, because the layout language needs it: the
   quest frame puts an element at `x = -16` and an unsigned type would send
   `4294967280`, which the client answers by dropping the whole layout.
7. **The first field of a `0xB0` is not a serial, and `GumpKey` says so.** The
   engine keys a window on the mobile it drew it for, which is why the field was
   called `serial` — but `0` is legal there and means a standalone dialog, the
   animal-lore window keys on its own dialog id, and ServUO puts `Gump.Serial`
   in it, a per-instance counter that is never an object. So it is `CursorId`'s
   twin: server-chosen, echoed, opaque. This also settles that the two
   `map_or(0, |s| s.raw())` sites here are **not** N1 amendment 7's bug — zero
   is a meaning in this field, not a nonsense serial — which is the answer to a
   pattern this sweep has otherwise found three times.
8. **The inbound key is class D.** `GumpResponse::serial` is echoed and nothing
   reads it: a reply is routed by its gump id, and each handler then matches
   against the context it *remembers* drawing, which is a stronger check than an
   echo can be. `RawGumpKey` therefore has no promotion, and the doc comment
   says why — the class-D record N3 asks for.
9. **The layout builder takes the typed ids, because it is the encoder.**
   `GumpLayout::button`/`radio`/`check` take `ButtonId`/`SwitchId` and unwrap
   inside, so one constant serves both the layout that drew a button and the
   arm that answers it — which is the whole loop N5 exists to close. Its other
   arguments (coordinates, gump art, hues, clilocs) stay bare: they are the
   client's positional format, nothing echoes them, and the cliloc column in
   particular is N3 amendment 7's parked table sweep, not this stage's.
10. **The engine types what the engine reads.** `GumpResponse::text_entries`
    stays `(u16, String)`: no window this engine draws has a text field, so
    every one of them is a *pack* gump, the id is one the pack chose, and "is
    this a field I drew" is a check only the pack can make. Typing it here would
    be a wrapper with no promotion and no reader — a `Raw` type that means
    nothing. This is the rule that decided the whole `Vec` question: `switches`
    got a type because `gates` and `quests` read them, `text_entries` did not.
11. **Raw ids cross the event bus to the pack.** `GumpAnswered` carries
    `RawGumpId`, `RawButtonId` and `Vec<RawSwitchId>`, and
    `openshard_server::scripting` unwraps them into JSON numbers. N3 amendment 9
    put raw types on `Command` going *in*; this is the same argument going out,
    and it is stronger: the engine drew none of these windows, so it is in no
    position to validate ids it never issued. The script bridge is the
    serialization seam, exactly as `Command::Speak` established.
12. **Bare-integer field count in `gump.rs`: 9 before, 0 after** (N10).
    `containers.rs` went 3 to 1 — the stack amount alone — when its `x`/`y`
    became a `GumpPoint`.

### Backlog from this stage

- **The admin menu is the one window still written as a layout string by hand**
  (`world/src/admin.rs`), so its six button ids appear twice: as literals inside
  the string and as `ButtonId` constants beside the handler. Every other window
  went through `GumpLayout` and now has one spelling. Building it through the
  builder would make the constants the only copy.
- **`ButtonId::CLOSE_BOX` and `ButtonId::UNUSED` are the same value with
  different meanings**, and a third would be one too many; if one appears, the
  type wants to be an enum with a `Reply(u32)` arm rather than a newtype with
  named zeroes.
- **`Command::ShowGump::serial` and its siblings are still bare `u32`s from the
  script bridge.** Roughly a dozen script-raised commands name a mobile that
  way and each re-does `Serial::new` in the tick. That is one sweep of its own,
  and it belongs with the component sweep N4 left rather than with a protocol
  stage.

## Stages

Each stage ends with all four silent: `cargo check --workspace --all-targets`,
`cargo test --workspace`, `cargo clippy --workspace --all-targets`,
`cargo fmt --all`. Each stage is one or more commits, landed through a pull
request (`main` is protected).

- **N-pilot — `CreateCharacter`, `CharacterPlay`.** By hand. Establishes the
  four classes, the two method names, the shared `wire.rs` types
  (`RawHue`, `RawGraphic`, `RawCharacterSlot`, `RawClientIp`), the first typed
  `Invalid*` errors, and the three missing checks.
- **N1 — the rest of `world.rs`** (movement, `0x02`, and the outbound world
  packets). The largest module, and the one whose inbound/outbound mix exercises
  N1's direction rule hardest.
- **N2 — `mobile.rs`.** Almost entirely outbound: mostly class A, the stage that
  proves the direction rule saves work rather than doubling it. It also lands
  `RawSerial` in `serial.rs` and `Layer` in `wire.rs`, both of which every later
  stage uses.
- **N3 — `speech.rs`.** One header sent five times, in both directions. Lands
  `TalkMode`/`RawTalkMode` and `Font`/`RawFont` in `speech.rs` and `ClilocId` in
  `wire.rs`; the four later stages that carry a cliloc use the last one.
- **N4 — `items.rs`, `containers.rs`.** Two directions on the same item, and the
  stage that answers N2's parked question about `Equipped.layer`.
- **N5 — `vendor.rs`, `gump.rs`, `context.rs`.** Gump ids and button ids are the
  interesting inbound case: a `0xB1` response echoes ids the *server* chose, so
  the check is "is this one I offered", not a range.
- **N6 — `login.rs`, `seed.rs`, `version.rs`.**
- **N7 — `feedback.rs`, `skill.rs`, `combat.rs`, `properties.rs`,
  `spellbook.rs`, `encoded.rs`, `casting.rs`.** The tail, one commit if it
  stays mechanical.
- **N8 — the sweep.** The counting check from N10, the allowlist with reasons,
  and a pass over `docs/style.md` if the four classes deserve a line in the
  canon.

Stages N1–N7 are agent work. They are ordered by module size rather than
dependency: `wire.rs`'s shared types all land in the pilot, so nothing after it
blocks anything else, and two stages can run in parallel when they touch
disjoint modules and disjoint call sites.

## The agent recipe

A stage is handed to a cheap agent with this, verbatim, plus the module list:

> Read `docs/protocol_newtypes.md` first — N1 through N11 are settled decisions,
> do not re-litigate them, and the four-class table in N3 is the whole recipe.
> Read `docs/style.md` for how code here reads. Use the
> `mcp__rust-code-mcp__*` tools (`search`, `find_definition`,
> `find_references`) rather than grep sweeps; they are deferred tools, so call
> them explicitly.
>
> For each module named: list every `pub` field of a bare integer type in a
> packet struct. For each one, decide the *direction* of the struct (N1) and the
> *class* (N3), then write exactly what the class says — no more. Reuse an
> existing type before adding one (N4); the wire newtypes are in `wire.rs` and
> `serial.rs`.
>
> Update every call site in the same commit (N11). Keep every byte-level test
> asserting the same bytes (N8). Add, for each class C field, the pair of tests
> N9 asks for. Report the bare-int field count for each file before and after
> (N10), and record anything the class table could not answer — that is an
> amendment for the doc, not a decision to make alone.
>
> Done when `cargo check --workspace --all-targets`, `cargo test --workspace`,
> `cargo clippy --workspace --all-targets` and `cargo fmt --all` are all silent.

A finding the class table cannot answer stops the stage and comes back as a
proposed amendment. That is the one thing an agent must not improvise: the
predecessor plan's value was that every stage's surprise got written down
(`0xB9` not fitting `EncodePacket`, `CreateCharacter`'s two ids), and a surprise
resolved silently in one module is a pattern the next module contradicts.

## Progress

| Stage | State | Commit |
| --- | --- | --- |
| pilot | types landed, promotions deferred (see amendments) | |
| N1 | done — `world.rs` 20 bare int fields → 5 allowlisted | |
| N2 | done — `mobile.rs` 37 bare int fields → 12 allowlisted | |
| N3 | done — `speech.rs` 22 bare int fields → 0 | |
| N4 | done — `containers.rs` 9 → 3, `items.rs` 16 → 3, all allowlisted | |
| N5 | done — `vendor.rs` 14 → 5 allowlisted, `context.rs` 6 → 0, `gump.rs` 9 → 0 | |
| N6 | not started | |
| N7 | not started | |
| N8 | not started | |
