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

**D2. Variant payloads.** A variant carries its fields inline when it has at
most four flat scalar fields (`WalkAck { sequence, notoriety }`). Anything with
a list, a nested enum, a conditional field, or logic of its own becomes a named
struct and the variant is a newtype around it (`Status(MobileStatus)`). Reason:
inline is unreadable past four fields, and a struct that needs methods needs a
name anyway.

**D3. The header is written once.** Payload encoders write **body only**. A
single framing layer writes the id and, for variable packets, back-patches the
`u16` length. This deletes the whole class of "forgot to patch the length"
and makes the length table the single source of truth for both directions.

**D4. Traits.**

```rust
pub trait EncodePacket {
    const ID: u8;
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

**D5. Nothing is silently dropped.** `ClientPacket` has an
`Unknown { id: u8, body: Vec<u8> }` variant. An unhandled id is a logged fact,
not a dropped connection and not a silent `true` return.

**D6. Newtypes on the wire.** `Serial`, `Graphic`, `Hue`, `Layer`, `SoundId`,
`BodyId`, `GumpId`, `CursorId`, `CliLocId`, `MusicId`. Bare `u32`/`u16` fields
are gone from packet definitions; `.0` is unwrapped only inside the codec.
`Serial` already exists in `common/entities` and **moves into
`common/protocol`** — it is a wire concept first; `entities` depends on
`protocol` for it, never the reverse.

Each newtype is introduced in the stage that first needs it, in `wire.rs`, not
all of them up front: a type nothing uses yet is a guess about a packet nobody
has read closely, and it hardens before it is right.

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
  newtypes (`SoundId`, `CursorId`, `Graphic`, `Hue`, `BodyId`), the two traits
  (D4) and the framing layer (D3), plus the `ServerPacket` root enum with its
  first variants. Proves the shape end to end, including a variable-length
  packet and a list-carrying one. If D2/D3 are wrong, this is where it shows
  and this document changes before anything else is migrated.
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
| 1 | not started | |
| 2 | not started | |
| 3 | not started | |
| 4 | not started | |
| 5 | not started | |
| 6 | not started | |
| 7 | not started | |
