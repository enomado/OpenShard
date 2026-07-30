# The client

The shard has always been server-only: the client is ClassicUO, and
`crates/common/protocol` is the contract between them. This document is the plan
for a second client — ours — and it starts where every client starts, by
connecting and walking into the world.

The order below is not a wish list. Each milestone is the smallest thing that
can be run and disbelieved, and each one is a prerequisite of the next: nothing
renders before the bytes arrive, and nothing arrives before the protocol can be
read in the direction a client reads it.

## What is already here, and what is missing

The protocol crate is complete in *one direction*. The server decodes what a
client sends and encodes what a server sends, so every packet type implements
exactly the half the server needed:

- `ClientPacket::decode` exists; `ClientPacket::encode` does not.
- `ServerPacket::encode` exists; `ServerPacket::decode` does not.
- `client_packet_length` and `frame_client_packet` exist. There is no
  server-to-client length table at all — the server has never needed one,
  because it knows the length of what it writes.
- `huffman::compress` and `huffman::decompress` both exist, but `decompress`
  takes a whole buffer and expects it to end at a terminator. A client reads a
  socket, not a buffer.

The readers for the client's own data files — `map`, `statics`, `uop`,
`tiledata`, `terrain` — used to live in `crates/server/world`, where a client
crate may not depend on them: `server/*` and `client/*` never depend on each
other. They now live in `crates/common/uofiles`, which both sides may use, and
`crates/server/world` keeps the gameplay built on top of them.

None of this is a defect. It is what "sans-io, server-side" honestly produces,
and the missing halves are mostly mechanical. What matters is that they are
*missing*, and that they are the first milestone.

## M0 — the protocol, in the other direction

Nothing else can start. Four pieces:

1. **`server_packet_length` and `frame_server_packet`.** The mirror of the
   client table. The numbers are not re-derived from a reference: every
   server-to-client payload already declares `EncodePacket::LENGTH`, so the
   table reads those constants and holds no second copy to fall out of step —
   the same argument as `ServerPacket::id`.

   An id our server never sends is `None`, i.e. fatal for the connection. That
   is deliberate: guessing a length for a packet nobody here writes would put a
   made-up number in the one table whose whole purpose is to be right.

2. **The decode side of `ServerPacket`, and the encode side of `ClientPacket`.**
   Not all at once — the login set first (`0x82 0xA8 0x8C 0xA9 0x1B 0x55 0x11
   0x78 0x1A 0x20 0x22 0x21 0xBF`), the rest as a milestone needs them.

3. **Incremental Huffman.** A game connection compresses *every write
   independently, terminator and all* (see `Session::send_packet`), so a client
   needs a decoder that can say "this many input bytes produced this block, and
   the rest is the next one". That is a `decompress_prefix` returning the
   payload and the bytes consumed; `decompress` becomes the one-shot case of it.

   **A block is not a packet, and assuming it is passes every unit test.** The
   login server answers a `0x91` with the feature mask and the character list in
   one buffer, which is compressed as one 1,115-byte block carrying two packets.
   Decompression fills a byte stream and framing splits *that* — two layers,
   kept apart, the way ClassicUO does it. This cost nothing to fix and would
   have cost a day to find without the end-to-end test below, which is what
   found it.

4. **Round-trip tests.** Until now an encoder could only be checked against
   hand-written bytes or against itself. With both halves present, every packet
   gets `encode(decode(x)) == x` — which tests the server's encoders with a real
   inverse for the first time.

## M1 — `crates/client/net`: connect, and enter the world

The milestone the whole plan hangs on, and the one that can be finished and
believed on its own.

- A sans-io `Connection`, the mirror image of `gateway::Connection`:
  `receive(bytes)` in, `poll() -> Event` out, an outgoing queue, and no socket
  anywhere near it. Byte boundaries are what is hard here, and a real socket
  will not reproduce them on demand.
- The login state machine: seed → `0x80` → `0xA8` → `0xA0` → `0x8C` → **a second
  socket** → seed + `0x91` → `0xA9` → `0x5D` → `0x1B` / `0x55` → in the world.
  The auth key from the relay is the only thing linking the two sockets, and the
  version travels in the seed — both are findings the server already paid for,
  and the client has to honour the same two.
- A Tokio driver in its own file that decides nothing.
- `WorldView`: what the server has shown us — our own serial, position and
  direction, the mobiles and ground items we have been sent, light, season, map.
  It is the client's side of `World::seen`, and it is a record of what arrived,
  never a guess about what is there.
- Walking: `0x02` with its sequence and fastwalk key, `0x22` to confirm, `0x21`
  to roll back to what the server says. This is the first place the client must
  be *right* rather than merely plausible — a mishandled reject desynchronises
  the position and everything drawn after it.

Done when an integration test drives the real server through the whole
conversation with this crate, and when the binary walks a character around a
live `cargo run -p openshard-server`.

That test lives in **`crates/e2e`**, a group of its own beside `common`,
`server` and `client`. It needs both ends in one process, and putting it on
either side would make that side depend on the other — the rule those two live
by. So it sits outside both, ships no code of its own, and nothing depends on
it. It is also what turned `crates/server/server` into a library with a
four-line binary: a test that wants a shard should call one, not build one.

Only what cannot be tested on one side belongs there. Framing, the login machine
and the tick all have better tests of their own; what is left for `e2e` is that
two correct ends actually agree — which is exactly what caught the compression
mistake above, on the first run.

## M2 — `crates/common/uofiles`: the data files

The move first: `map`, `uop`, `tiledata`, and the format-reading half of
`terrain` leave `crates/server/world` for a crate both sides may depend on. The
world crate keeps the gameplay built on top of them.

Then the readers a renderer needs and a server never did: `hues`, `art` (land
and static), `gumpart`, `anim` with `animdata`, `unifont`, `cliloc`, `multi`,
`texmaps`, `light`, `radarcol`, `sound`, `verdata`. In that order the first
picture needs hues, land art, and the tiledata and map readers that already
exist.

No client files enter this repository, now or ever. Tests read
`OPENSHARD_CLIENT` and skip when it is unset.

## M3 — the first picture

Isometric 44×44 diamonds, ground only to begin with: no statics, no mobiles.
A flat green field in the right place proves the block loading, the coordinate
system and the hue table, and proves them separately from the sorting problem.

Then statics, then mobiles, then the labels — and with them UO's draw order,
which is the part of a UO renderer that is actually hard. The camera follows
`0x20`, and blocks load around the player.

**The ground half is done.** `crates/client/render` draws it and
`crates/client/app` puts it in a window: `OPENSHARD_CLIENT=… cargo run -p
openshard-client-app` opens on Britain and the arrow keys walk the camera. The
crate is `wgpu`, and it is browser-shaped on purpose — WebGL2's ceiling, no
compute, no storage buffers, instancing through vertex buffers, every device
request `async`, and a 2048 atlas because that is the only texture size WebGL2
guarantees. It compiles for `wasm32-unknown-unknown`; nothing runs there yet,
because a browser has no filesystem and every reader in `uofiles` opens a path.

What made it worth doing rather than looking at: the renderer draws into an
offscreen texture just as readily as into a surface, so `tests/frame.rs` reads
frames back and asserts on the bytes. A lone sprite is compared to the art it
came from texel for texel, and level ground is asserted to cover **every** pixel
of the viewport — 393,216 of 393,216. Both found real defects on their first
run: `visible_tiles` was widened for `z` in only one direction, which loses a
band of ground wherever the terrain goes negative, and the atlas treated a black
pixel as a transparent one. Neither would have been visible on a screenshot.

Deliberately not done here: **hue**. Ground carries no hue — `LandCell` has a
graphic and a height and nothing else — so the hue table has no consumer until
statics arrive, and building the plumbing for it now would be building it
untested.

## M4 — the gump layer

The journal and the speech line, the status bar, the paperdoll, containers, and
generic gumps: `0xB0` is already encoded by the server, so the client needs the
decoder and a layout parser for it.

## M5 — interaction

Single and double click (`0x09`, `0x06`), drag and drop (`0x07`, `0x08`),
targeting (`0x6C`, `0x6B`), speech (`0xAD`), war mode.

## Decisions to take before they are taken by accident

- **Crates.** `crates/client/net`, `crates/client/render`, `crates/client/app`,
  plus `crates/common/uofiles`. The direction rule stands: a client crate
  depends on `common`, never on `server`.
- **What we draw with.** `wgpu`, directly — no engine. Bevy and its neighbours
  would supply a window, input and sprite batching, and none of what is actually
  hard here: UO's draw order, the hue table, and streaming map blocks out of a
  155MB container. What they *would* impose is a second ECS beside `WorldView`,
  ownership of the main loop, and a frame that cannot be rendered inside
  `cargo test`. ClassicUO does not use an engine either, and not out of poverty.
- **The browser is a target, so it constrains the design now.** WebGL2's ceiling
  rather than native Vulkan: no compute, no storage buffers, instancing through
  vertex buffers, a 2048 atlas, and `async` device requests because a browser
  cannot be blocked on. Cheap to honour from the first triangle and painful to
  retrofit. What is *not* done: `uofiles` still opens paths, and a browser has
  no filesystem — the parsing is already separate from the reading, so the fix
  is byte-taking constructors and `std::fs` behind `cfg`, not a rewrite.
- **Colour is never converted.** Textures and targets are `Rgba8Unorm`, never
  `…Srgb`. The files hold five bits a channel with no colour space attached, and
  a gamma conversion anywhere means the pixel that went into the atlas is not
  the pixel in the frame — which turns every exact test assertion into a
  tolerance nobody can justify.
- **Which version we claim to be.** The client announces one in its seed, and
  every `Feature` gate on the server follows from it. 7.0.45.65 — what
  ClassicUO opens with — keeps us on the modern packet set instead of the legacy
  branches of every encoder.
- **A client that only speaks what our server happens to send is a mirror, not
  a UO client.** That is the right scope for M1–M3, and it is also how both
  ends quietly agree on the same mistake. Every packet this client learns should
  be checked against a real client's behaviour, not only against our own server.

## Backlog, found while pointing the readers at a real install

`crates/common/uofiles/tests/client_files.rs` is the suite that found these: the
readers had good tests, and every one of them was against a fixture the reader's
own understanding had written.

- ~~**Ter Mur loaded as Malas.**~~ Fixed. Malas is 320×256 blocks and Ter Mur is
  160×512, and both are **81,920** — so the block count, the only thing the file
  offers, names the wrong facet. Their `staidx` files are the same length too,
  which means the one consistency check that exists also passes. The result was a
  facet read at 256 blocks per column instead of 512: everything past the first
  column somewhere else, no error, no complaint. `facet_size` now takes the facet
  number `load_facet` already had. `Map::load`, which has only a path, still
  cannot tell them apart, and its doc now says so.
- **`Map::load` is public and called only by its own tests.** It is also the one
  entry point that cannot resolve the collision above. Either it grows a facet
  argument or it stops being `pub` — but that is a decision about who is supposed
  to call it, and nobody does yet.
- **The land table's record 0 is written in the pre-High-Seas shape.** Its name
  sits six bytes into a 30-byte record, so read at the modern offsets tile 0 has
  flags `0x4E55_0000_0000_0000` and the name `"ED"` — the tail of `"UNUSED"`.
  Every other record in the file is fine, so this is the file's quirk and not the
  stride. It is deliberately not special-cased: the junk lands entirely above bit
  32 and every flag movement reads is below it, so tile 0 cannot come out
  walkable, water, or a floor — which is what the test asserts instead.
- ~~**`hues` and `art` are missing.**~~ Written. `hues.mul` is 3,000 ramps of 32
  colours; `artLegacyMUL.uop` holds the land diamonds and the run-length encoded
  statics in one index space. What is still missing: `gumpart`, `anim`,
  `unifont`, `cliloc`, `multi`, `texmaps`, `light`, `radarcol`, `sound`,
  `verdata`. The first picture no longer needs any of them.
- **Gump art is deflated and nothing here can inflate it.** Every one of
  `gumpartLegacyMUL.uop`'s 5,556 entries has compression flag 3, where the map
  and art containers have none. `UopError::Compressed` says so rather than
  skipping. Whoever writes the gump reader — M4 — is the one who brings an
  inflater into the workspace, and it is worth deciding then whether that is a
  dependency or forty lines of stored-block DEFLATE.
- **`.mul` art is not read, only the UOP.** A modern install ships no
  `art.mul`/`artidx.mul` at all, so there is nothing here to test a `.mul` path
  against. The index is the same twelve-byte entry `staidx` uses, so it is an
  hour's work — but an hour spent writing something nobody can run, and the
  engine claims to support old clients, so this needs an old install rather than
  confidence.
- **A container is read whole into memory.** `Uop::open` holds all 155MB of the
  art. Harmless for a shard, which never opens it, and not obviously right for a
  renderer holding several containers at once. The place to fix it is
  `Uop::open`, once something actually draws.

## Backlog, found while drawing the ground

- **Ground is drawn flat, so hillsides have seams.** A tile is one 44×44 diamond
  at its own `z`, and neighbours at different heights pull apart: a screen of
  Britain covers 97–98% of its pixels, the sea covers 100%. The client stretches
  ground onto its four corner heights and textures a sloped tile from
  `texmaps.mul`, which no reader here touches yet. That is the next piece of M3
  and it needs a new reader, not just a shader.
  `broken_terrain_drops_no_tile_but_does_leave_seams` fails on purpose once this
  lands.
- **Void land tiles are drawn as black diamonds.** Under a building the map's
  ground is a "nothing here" graphic that the real client covers with statics.
  Until statics are drawn this leaves black holes in the picture, which is
  honest, but it is worth checking whether `tiledata`'s flags name these
  explicitly before deciding whether the renderer should skip them.
- **The atlas is rebuilt whole whenever the camera walks off it.**
  `client/app` repacks and recreates the pipeline on a miss. Free for ground —
  a screen holds a few dozen graphics of the 2,116 that fit — and not free once
  statics share the atlas. The place to fix it is an eviction policy in
  `LandAtlas`, once something needs one.
- **`Map` cannot be built in memory, so the renderer has no offline tests.**
  Every assertion about `ground::collect` lives in `tests/frame.rs` behind
  `OPENSHARD_CLIENT` and a GPU, because the only way to get a `Map` is to load
  one from a file. A constructor taking cells — or a small fixture facet — would
  let the projection and the visible-set logic be tested with neither.
- **Nothing reads `Feature` or the client version yet.** The renderer draws what
  the files hold. That is right for ground, and it stops being right at the
  first packet the client draws from.

## Backlog, found while building M0, M1 and M1a

Each is a seam the work made visible. None blocks the next milestone.

- **A walking client has no map, so `z` never changes under it.**
  `openshard_movement::intend` carries the height over unchanged — height is
  the terrain's answer and neither end of `intend` has terrain — so
  `client_net::walk::Walk` predicts a flat world. The server, which does read
  the map, lands the step wherever the ground is, and says nothing: `0x22`
  carries no position. A client walking up a hill therefore drifts in `z` until
  a `0x20` or a `0x21` corrects it. The fix is the map, which is M2; until then
  the drift is documented rather than papered over, because a guessed height
  would be indistinguishable from a real one.
- **`enter_world` drops everything between `0x1B` and `0x55`.** It builds the
  `WorldView` from the entry packet and then reads until the `0x55`, without
  applying anything that arrives in between — and that window is exactly where
  a shard sends the player's own `0x20`, the `0x78` for everyone already on
  screen, and the ground items. Harmless for a login test, wrong for a client
  that wants to draw the moment it is allowed to. The loop needs to `apply`
  what it reads, not skip it.
- **Nothing on the client models a status bar, and two packets are waiting for
  one.** `MobileStatus` (`0x11`) decodes and is deliberately not folded into
  `WorldView`; `WalkAck`'s notoriety reaches the caller through
  `walk::Moved::Stepped` and has nowhere to go either. Both are the same
  missing thing — health-bar colour and paperdoll numbers are not positions —
  and both should land wherever M4's status bar does.

- **The shard sends the feature mask and the character list as one write.**
  Correct, and worth naming: it means "one compressed block" and "one packet"
  are different things on this wire, and any future reader — a proxy, a packet
  logger, a second client — has to keep the two layers apart.
- ~~**`ServerPacket::decode` covers the login set only.**~~ Fixed: `0x20`,
  `0x11`, `0x77`, `0x78`, `0x1A` and `0x1D` all decode now. `WorldView` folds
  five of them in — a client's own body, every other mobile, every ground
  item, and what `0x1D` takes back off screen. `MobileStatus` (`0x11`) decodes
  too but stays out of `WorldView`: it is paperdoll data, not a position, and
  belongs with whatever eventually models the status bar. Its `max_weight` is
  honestly lossy below status type 5 — the wire never carries it that old, so
  decoding gets `0` back rather than a guess at a real value.
- **`CharacterList` decodes only the post-7.0.13.0 form.** The older start list
  carries no coordinates, so there is no honest `StartLocation` to build; the
  decoder says so rather than inventing zeros. If this engine ever wants to be
  a client to an *old* shard, that is where to start.
- ~~**The client-to-server encoders are still labelled "test fixtures only".**~~
  Fixed: `AccountLogin::encode`, `SelectShard::encode`, `GameServerLogin::encode`
  and `CharacterPlay::encode` now say what `crates/client/net`'s login state
  machine (`session.rs`) actually calls them for. Only `ClientVersionReport::encode`
  is genuinely still test-fixtures only, since the client does not announce its
  version yet.
- **`Login` fixes the seed value at `0x0A000001`.** It is never read — see
  `RawSeedValue` — but a client that will one day face a shard implementing
  login encryption will need it to be the value that keys the cipher.
- **The `0x82` refusal loses why.** `DenyReason::from_wire_code` returns one
  reason per wire code because the wire has five, and the server collapsed
  fifteen into them. Nothing to fix on this side; worth remembering before
  anyone builds a UI that explains a failed login.
