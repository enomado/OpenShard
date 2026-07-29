# Protocol rewrite: from free functions to packet enums

Living plan for a multi-session rewrite of `crates/common/protocol`. It records
the decisions so they are not re-litigated at the start of every session. When
reality contradicts a decision here, change this file in the same commit that
changes the code.

## Why

Today the crate has two shapes bolted together:

- **Client → server** is a set of unrelated structs, each with its own
  `const ID` and its own `decode`. Nothing ties them together, so
  `server/server/src/dispatch.rs` is a 719-line hand-written `match` over raw
  bytes — including `packet.get(5) == Some(&0x05)` reaching into an undecoded
  packet, and three different `0xBF` types (`context`, `casting`, `mobile`)
  that each re-read the same envelope and each decide independently whether the
  packet is "theirs".
- **Server → client** is 47 free functions named `encode_*`, each returning a
  fresh `Vec<u8>`, each writing the id byte by hand and each patching its own
  length field by hand.

Neither shape is checkable. Nothing tells you a packet id is handled twice, or
not at all; nothing stops a new encoder from forgetting its length patch; the
`match` in `dispatch` has no exhaustiveness to lean on because it matches on
`Option<&u8>`.

The wire format is a fixed external contract with a closed set of messages.
That is exactly a sum type. It should be one.

## The shape of the protocol (surveyed against ClassicUO)

ClassicUO is the reference: `src/ClassicUO.Client/Network/PacketsTable.cs` is
the 256-entry server-packet length table, `PacketHandlers.cs` (7161 lines) is
every server packet parsed, `OutgoingPackets.cs` (4671 lines) every client
packet built.

**There is no recursion.** The nesting is finite and shallow:

1. **id byte** — 256 slots.
2. **subcommand** — a handful of envelopes: `0xBF` general information (`u16`
   subcommand, ~40 defined), `0xD7` encoded command (`u16`), `0xB5`/`0xB3` chat
   (`u16`, `0x03E8..0x03F4`), `0x12` (type byte).
3. **third level, rare** — `0xBF 0x06` party has its own byte subcommand
   (`PartyManager.ParsePacket`); `0xBF 0x16` close-window keys on a window id;
   `0xBF 0x19` extended stats keys on a `version` byte (0/2/5), and inside
   version 5 branches again on `type2 == 0xFF` — effectively a fourth level.

So: an enum of enums, no `Box`, no cycles.

Three things complicate it beyond plain nesting, and the type design has to
carry all three from the start rather than retrofit them:

- **Repeated records** — container contents, skill lists, shard and character
  lists, buy/sell lists. These are `Vec<T>` of a named row type.
- **Fields conditional on bit flags** — `0x1A` world item hides the presence of
  count, hue and flags in the high bits of the serial and graphic; `0x77`/`0x78`
  do the same. Presence is data, so the optional parts are modelled as fields
  that are semantically absent, not as defaulted zeros.
- **Version-conditional tails** — `0x11` status, `0x78`/`0xD3`, `0xA9` change
  shape by era. The branch is on `ClientVersion::supports(Feature::…)`, never on
  `Era` and never on a version comparison (see the crate docs).

One genuine grammar exists: **gump layout** (`0xB0`, compressed `0xDD`) is a
text DSL — `{ gumppic 0 0 100 }{ page 1 }…` — plus a string table. It is not
recursive (pages are flat sections) but it is a language, and it gets its own
type and its own encoder rather than a variant that carries a pre-built string.

## Decisions

These are settled. Do not re-open them mid-rewrite.

**D1. Two root enums.** `ClientPacket` (decoded, client → server) and
`ServerPacket` (encoded, server → client). Both non-exhaustive.

**D2. Variant payloads.** Every variant is a newtype around a named payload
struct (`Status(MobileStatus)`, `WarMode(WarMode)`). The pilot disproved the
inline exception: [`EncodePacket`] is implemented on a payload type, so inline
variant fields would need a second body-writing path inside the root enum.
One shape for every variant keeps the framing layer mechanical.

**D3. The header is written once.** Payload encoders write **body only**. A
single framing layer writes the id and, for variable packets, back-patches the
`u16` length. This deletes the whole class of "forgot to patch the length"
and makes the length table the single source of truth for both directions.

**D4. Traits.**

```rust
pub trait EncodePacket {
    const ID: u8;
    const LENGTH: PacketLength;
    fn encode_body(&self, out: &mut PacketWriter, version: ClientVersion);
}

pub trait DecodePacket: Sized {
    const ID: u8;
    fn decode_body(reader: &mut PacketReader, version: ClientVersion)
        -> Result<Self, DecodeError>;
}
```

`ClientVersion` is passed to every encoder and decoder uniformly, even where it
is unused — a packet that grows a version-conditional tail later must not
change its signature and every call site with it.

`EncodePacket::LENGTH` tells the framing layer whether a payload is fixed or
variable length. For fixed packets, the framer debug-asserts that the encoded
body matches the declared size, catching a field added to a struct but forgotten
in its encoder.

**D5. Nothing is silently dropped.** `ClientPacket` has an
`Unknown { id: u8, body: Vec<u8> }` variant. An unhandled id is a logged fact,
not a dropped connection and not a silent `true` return.

**D6. Newtypes on the wire.** `Serial`, `Graphic`, `Hue`, `Layer`, `SoundId`,
`GumpId`, `CursorId`, `CliLocId`, `MusicId`. Bare `u32`/`u16` fields
are gone from packet definitions; `.0` is unwrapped only inside the codec.
`Serial` already exists in `common/entities` and **moves into
`common/protocol`** — it is a wire concept first; `entities` depends on
`protocol` for it, never the reverse.

Each newtype is introduced in the stage that first needs it, in `wire.rs`, not
all of them up front: a type nothing uses yet is a guess about a packet nobody
has read closely, and it hardens before it is right.

`Option<Serial>` is the packet shape for an absent object field. A zero object
serial on the wire decodes to `None`, and an absent object encodes as zero.
Sound ids stop at the packet boundary for now: gameplay sound tables still carry
plain `u16` and wrap them in `SoundId` only when building packets.

**D7. `LoginDecodeError` is renamed `DecodeError`.** It was never login-specific
(56 references across the workspace); the name lies about scope.

**D8. No re-exports.** `lib.rs` stops being a wall of `pub use` (CLAUDE.md:
re-exports hide where a type lives). Modules become `pub`, call sites import
from the defining module. This lands in Stage 6 as one mechanical sweep, not
drip-fed.

**D9. No compatibility shims.** Each stage rewrites a group of packets **and**
updates every call site of that group in the same commit. No `#[deprecated]`
wrapper layer: a half-migrated crate with two ways to send a packet is worse
than a bigger diff.

**D10. Byte-level tests are the contract.** Every existing encoder test keeps
asserting the same bytes; only the call that produces them changes. A stage
that cannot keep the bytes identical has found a bug — fix it deliberately and
say so in the commit, do not adjust the expectation quietly.

## Amendments forced by the Stage 1 pilot

1. **D2 loses its inline case.** "At most four flat scalars go inline in the
   variant" cannot hold: `EncodePacket` has to be implemented *on a type*, so a
   variant with inline fields has no payload to implement it on and would need
   a second body-writing path inside `ServerPacket`. Every payload is now a
   named struct and every variant is a newtype around one.
2. **`EncodePacket` gained `const LENGTH: PacketLength`.** D3 wants the
   framing layer to write the length field, and the framer cannot know
   whether there is one without asking the payload. It pays twice:
   `frame_body` debug-asserts a fixed packet's body size, catching a field
   added to a struct and forgotten in its encoder.
3. **Stage 1 does not introduce `BodyId`,** and proves neither a
   variable-length packet nor a list-carrying one — none of
   `target`/`combat`/`feedback` has any of the three. The variable-length path
   is exercised by a unit test in `packet.rs` instead; the real proof moves to
   Stage 2 (`login`: shard list, character list). D6's own rule — a newtype
   arrives with the packet that needs it — beats the stage bullet that
   promised `BodyId`.
4. **`Option<Serial>` is the shape of an empty object field.**
   `TargetResponse.object`, `AttackRequest.target`, `AttackTarget.target`,
   `GraphicalEffect.from`/`to`.
5. **Sound ids stop at the packet boundary.** `WorldState::play_sound` still
   takes a bare `u16` and wraps it in `SoundId` where the packet is built.
   Converting the sound *tables* (spell definitions, creature voices,
   instrument notes, the scripting op) is its own sweep and would drag serde
   into the protocol newtypes. Same for `Graphic` at the spell-visual sites.
6. **`EffectPoint` is gone** — it was `world::Point` field for field, and the
   effect packets now use `Point`.

## Amendments forced by the Stage 2 pilot (`login`)

Stage 2 is where the variable-length path first carries a real payload — the
shard list and the character list are both `Vec<T>` bodies, not the unit test
Stage 1 covered it with — and where a packet finally does not fit [`D3`](#decisions)'s
Fixed/Variable split at all.

1. **`decode_packet` now skips a variable packet's length field itself,**
   rather than leaving it to each `decode_body`. `ClientVersionReport` (`0xBD`)
   is the first variable client-to-server packet migrated, and without this
   its body would start two bytes early. The check belongs in exactly one
   place: `frame_client_packet` has already validated the claimed length
   against the buffer and `MAX_PACKET_SIZE` by the time a decoder runs, so
   `decode_body` gets bytes that are already known-good and never re-checks
   the length itself. One consequence worth being explicit about: a
   `decode_packet` call fed raw bytes that skip framing (as a unit test can)
   no longer rejects a length field that lies — that check now lives once, at
   the framing layer, not duplicated in every variable decoder.
2. **`0xB9` (`encode_supported_features`) stays a free function, not an
   `EncodePacket`.** It has no length field at all — unlike every other
   variable packet — and its size (3 or 5 bytes) depends on the client
   version, which `EncodePacket::LENGTH` cannot ask about because it is a
   `const`. Neither `Fixed` nor `Variable` describes it. This is the
   server-to-client mirror of `0x08`'s problem on the decode side
   (`client_packet_length` takes a version for exactly this reason); until the
   framing layer can express "fixed, but which fixed size depends on the
   version" for both directions at once, `0xB9` is written by hand rather than
   forced into a model it does not fit.

## Amendments forced by the Stage 3 pilot (`world`, `mobile`)

Stage 3 is the first to hit a packet whose *id* is shared by two logically
different bodies (`CreateCharacter`), and the first to find `0xBF` packets
that are fixed-size despite the id's own table entry saying `Variable`.

1. **`CreateCharacter` (`0x00` / `0xF8`) is not a `DecodePacket`, for the same
   reason `0xB9` is not an `EncodePacket` (Stage 2).** `DecodePacket` assumes
   one `const ID`; this packet is one logical decode across *two* ids with two
   different fixed lengths (104 bytes/three skills vs. 106 bytes/four). Bending
   the trait to accept an id list, or picking one id arbitrarily, would either
   complicate every other decoder for one packet's sake or silently stop
   accepting the id it didn't pick. `CreateCharacter::decode` stays a plain
   inherent method, exactly as surveyed.
2. **Two more `0xBF` packets turned out fixed, not variable, and both still
   hand-write their own length field.** `world::MapChange` (subcommand `0x08`,
   always 6 bytes) and `mobile::StatLocks` (subcommand `0x19` type `2`, always
   12 bytes) never carry a list or a version branch, so `EncodePacket::LENGTH`
   is `Fixed`, not `Variable` — simpler, and it gets the `frame_body` debug
   assert on total size for free. The one wrinkle: `frame_body` only
   back-patches a length field for `Variable`, so these two bodies still write
   their own constant `u16` length literal by hand, in exactly the spot the
   `0xBF` envelope always puts one. That hand-written literal and
   `EncodePacket::LENGTH` now have to agree by construction rather than by a
   shared mechanism — the same kind of two-places-that-could-disagree gap D3
   exists to close, just not one the trait as designed can close for an id
   whose *table* entry is `Variable` but whose *body* never is. Noted here
   rather than silently declaring `Variable` (which would insert a length field
   the client already gets from the subcommand's fixed shape, doubling it).
3. **`MobileStatus` and `MobileIncoming` matched the plan exactly:** both were
   already self-patching their length by hand at the same offset `frame_body`
   patches for `Variable`; converting them to `EncodePacket` with
   `LENGTH = PacketLength::Variable` let the manual `writer.u16(0)` placeholder
   and the closing `bytes[1..3].copy_from_slice(...)` come out unchanged in
   behaviour, byte for byte.
4. **`StatLockRequest` was left exactly as surveyed:** it already had the
   `0xBF`-envelope shape (`decode(bytes) -> Result<Option<Self>, DecodeError>`)
   that several unrelated logical packets share one id under, and forcing it
   into `DecodePacket` would be Stage 6's unification arriving four stages
   early.

## Stages

Each stage ends with all four silent: `cargo check --workspace --all-targets`,
`cargo test --workspace`, `cargo clippy --workspace --all-targets`,
`cargo fmt --all`. Each stage is one or more commits on `main`.

- **Stage 0 — the rename.** `DecodeError`, `WrongPacket` and `expect_id` out of
  `login.rs` into `error.rs` (D7). Nothing else: the traits and the newtypes
  land with the first packets that use them, per D6, rather than as a layer
  written against packets nobody has re-read yet.
- **Stage 1 — pilot: `target`, `combat`, `feedback`.** Smallest groups, fewest
  call sites. Brings in with them: the `Serial` move (D6), the first wire
  newtypes (`SoundId`, `CursorId`, `Graphic`, `Hue`), the two traits (D4) and
  the framing layer (D3), plus the `ServerPacket` root enum with its first
  variants. The variable-length path is covered by a packet unit test; the real
  variable/list packet proof moves to Stage 2's login lists. If D2/D3 are wrong,
  this is where it shows and this document changes before anything else is
  migrated.
- **Stage 2 — `login`.** The most version-conditional group (shard list,
  character list, feature flags) and its own dispatch path in `server/login`.
- **Stage 3 — `world`, `mobile`.** The largest and hottest: movement, status,
  `MobileIncoming`, equipment. Flag-conditional fields land here.
- **Stage 4 — `items`, `containers`, `vendor`, `properties`, `skill`.**
  List-heavy, mostly mechanical after Stage 3.
- **Stage 5 — `speech`, `gump`, `spellbook`, `context`, `casting`.** Includes
  the gump layout DSL as its own type.
- **Stage 6 — `ClientPacket` and `dispatch.rs`.** Decode once at the edge, then
  a single exhaustive `match` over `ClientPacket`. The `0xBF`/`0xD7` envelopes
  collapse into `ExtendedRequest`/`EncodedRequest` sub-enums here — the three
  separate `0xBF` types (`context`, `casting`, `mobile`) merge.
- **Stage 7 — cleanup.** Drop the `pub use` wall (D8), delete the last
  `encode_*`, update `docs/architecture.md` and the crate docs.

## Progress

| Stage | State | Commit |
| --- | --- | --- |
| 0 | done | `153e1f8` |
| 1 | done | `daad3e0` |
| 2 | done | `77ba897` |
| 3 | done | |
| 4 | not started | |
| 5 | not started | |
| 6 | not started | |
| 7 | not started | |
