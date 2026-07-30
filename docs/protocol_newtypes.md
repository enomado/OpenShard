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
  proves the direction rule saves work rather than doubling it.
- **N3 — `speech.rs`.**
- **N4 — `items.rs`, `containers.rs`.**
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
| N1 | not started | |
| N2 | not started | |
| N3 | not started | |
| N4 | not started | |
| N5 | not started | |
| N6 | not started | |
| N7 | not started | |
| N8 | not started | |
