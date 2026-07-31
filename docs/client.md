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
and static), `texmaps`, `gumpart`, `anim` with `animdata`, `unifont`, `cliloc`,
`multi`, `light`, `radarcol`, `sound`, `verdata`. In that order the first picture
needs hues, land art, and the tiledata and map readers that already exist; the
first *hillside* needs `texmaps` as well.

No client files enter this repository, now or ever. Tests read
`OPENSHARD_CLIENT` and skip when it is unset.

## M3 — the first picture

Isometric 44×44 diamonds, ground only to begin with: no statics, no mobiles.
A flat green field in the right place proves the block loading, the coordinate
system and the hue table, and proves them separately from the sorting problem.

Then statics, then mobiles, then the labels — and with them UO's draw order,
which is the part of a UO renderer that is actually hard. The camera follows
`0x20`, and blocks load around the player.

**Statics and mobiles are drawn.** `crates/client/render` has three passes now:
the ground, and two sprite passes that differ only in where the quad goes —
`statics::collect` from the map's own `staidx`, `mobiles::collect` from a list
somebody else built. `crates/common/uofiles/src/anim.rs` reads the frames the
second one draws.

**What decides overlap is a depth buffer, not a draw order.** This is the part
worth writing down. Ground is drawn whole before any static, so painter's order
*within* a pass says nothing about the pass next door: without a shared depth,
every wall would be in front of every hill. So all three passes compute one
ordering on the CPU — `crates/client/render/src/depth.rs` — and all three test
it, which makes the pass order decide nothing but who clears.

The ordering is ClassicUO's, taken apart rather than copied.
`Chunk.AddGameObject` gives ground its average height less two, a static its
own height with one down for a background tile and one up for anything with a
height, and a mobile one above everything; `View.CalculateDepthZ` folds that
together with `x + y` into `(x + y) + (127 + z) * 0.01f`. That float form
overflows into the next tile at large `z`, so what is kept here is the integer
pair — sorted tile first — normalised around the camera, which puts the visible
frame where a 24-bit buffer has resolution to spare. A step of one priority is
1e-6 apart where the buffer resolves 6e-8, and `depth.rs` asserts the margin
rather than trusting it.

**A static sprite's zero pixel is absent; a land sprite's is black.** Opposite
rules in two files, and both are the client's: `ArtLoader.ReadStaticArt` writes
a run's pixel only `if (val != 0)`. Getting it backwards on statics draws every
sprite's bounding box as a rectangle.

**A mobile is placed from its frame, not from its tile.** Five directions are
stored and three are mirrors of them, so half of every creature is drawn
backwards — and flipping a picture moves its anchor to the other edge:
`MobileView.Draw` is `x -= flipped ? width - center_x : center_x`, `y -= height
+ center_y`. Using `center_x` for both makes every west-facing creature stand a
body's width from where it is. The flip itself costs nothing: a region with a
negative width samples its own texels backwards, which is asserted on a real
GPU rather than argued about.

`anim.mul` is 195MB and is the first reader here that does **not** read its
container into memory — the index is held and frames are read on demand, which
is why `Anim::frames` takes `&mut self`. The browser is the reason the rest of
`uofiles` will follow.

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

**A slope is textured from `texmaps.mul`**, which is the difference between
terrain and a 44×44 diamond pulled out of shape. The art tile is drawn for level
ground; on a steep quad it smears, and the client does not use it there at all —
it takes a square 64×64 or 128×128 picture from `texmaps.mul` and maps it corner
to corner onto whatever the four heights make. Which corner is which is
ClassicUO's `DrawStretchedLand`, and it is the identity: the quad's top vertex is
the texture's top-left. `crates/common/uofiles/src/texmaps.rs` reads the pair,
and `tiledata`'s land entry — whose texture id this reader had been skipping past
for its whole life — is what says which texture belongs to which tile.

Nothing in either file relates a land graphic to a texture id, so reading that
field two bytes out gives *a* texture for every tile and the ground comes out
textured with somebody else's terrain: a picture, and one that reads as a
seasonal variant rather than as a bug. The test is a comparison rather than a
threshold — a tile's own texture and its own art have close average colours, and
the same measurement against a shifted pairing is three times worse (5.5 against
18.4 across 3,806 tiles). A file decides, not a number somebody chose.

Two things the half-texel inset is not: a nicety, and ours. A quad's corner
texture coordinates *are* the region's edges, and an edge is the boundary between
two texels — so at `u + du` the sample lands on the first texel of whatever was
packed next door, a one-texel fringe of the wrong terrain along two edges of
every sloped tile. The frame test caught it on its first run. ClassicUO insets
by half a texel in `CalculateHalfPixelUVs` for the same reason.

Where we knowingly differ from the client: a tile whose graphic has **no**
texture. The client refuses to stretch such a tile at all and draws it flat,
seams and all; we stretch it and texture it with the art, because the geometry
here is watertight by construction and giving that up would put the holes back.
`Land.ApplyStretch` is the reference, and the backlog has the rest of what it
does that we do not.

**Ground is stretched over its four corner heights**, which is the difference
between terrain and a mosaic of flat diamonds. A land cell stores one height and
it belongs to the diamond's *top* corner; the other three are the neighbours'.
Two tiles therefore do not merely abut, they are built from the same vertices,
and a seam between them is not expressible — which is why hilly Britain now
covers every one of its 393,216 pixels too, where flat diamonds left 2.3% of the
viewport in gaps. A tile whose four corners agree keeps the old path exactly: it
is drawn as the art's own square with the diamond cut out by alpha, so the
texel-for-texel comparison still holds where it can. The choice between the two
shapes is made in the shader from the heights themselves, so it cannot disagree
with them.

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
  statics in one index space. `texmaps` followed, for the slopes. What is still
  missing: `gumpart`, `anim`, `unifont`, `cliloc`, `multi`, `light`, `radarcol`,
  `sound`, `verdata`. The first picture no longer needs any of them.
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
  art and `TexMaps::open` another 45MB. Harmless for a shard, which never opens
  either, and not obviously right for a renderer holding several at once — the
  client app now reads 200MB of pictures before its first frame. The place to fix
  it is `Uop::open` and `TexMaps::from_files`, and the browser is the deadline:
  neither can call `std::fs` there anyway.

## Backlog, found while drawing the ground

- ~~**A sloped tile is textured with the stretched land sprite, not
  `texmaps.mul`.**~~ Read and drawn. `uofiles::texmaps` reads the
  `texidx.mul`/`texmaps.mul` pair — 4,116 squares of 64 or 128, and the *length*
  is what says which — and `TexmapAtlas` packs them on a 64-pixel cell grid,
  where a 128 takes a 2×2 block. Both atlases are keyed by the land graphic, so a
  quad asks them the same question.
- ~~**A `tiledata` flag may force a tile flat.**~~ Answered, and it is not a
  flag. `Land.ApplyStretch` stretches a tile only when it *has* a texture and is
  not wet; `IsStretched` is initialised to `TexID == 0 && IsWet` and then read as
  "do not". So "no texture" is the whole of the rule, and `WATER` is the only
  flag in it.
- **The client stretches a wider neighbourhood than we do.** `ApplyStretch` sets
  `IsStretched` from the four corner *normals*, each of which reads the tile
  beyond the corner — so a tile whose own four corners agree is still stretched,
  and therefore textured, when a neighbour differs. `ground.wgsl` decides from
  the four corners alone, so such a tile is drawn from its art here. The shapes
  agree exactly (a flat quad is the diamond), so this is a difference in
  *texture* along the edge of every slope, and closing it means computing the
  normals — which is also what lighting will need.
- **Nothing computes a normal, so nothing is lit.** The client shades stretched
  land from the four corner normals it already has. Ours is flat-lit, which is
  right for a first picture and stops being right beside a real client's
  screenshot.
- **`TexTerr.def` is not read.** The client remaps texture entries through it
  (`2500 {3} 1645` — entry 2500 is entry 3, hued), which matters for a land tile
  whose texture id lands in the aliased range and has no entry of its own. Such
  a tile falls back to its art here. The `.def` format is shared with several
  other files, so whoever needs the first of them writes the reader.
- **The stretched-art fallback has no half-texel inset.** Where a sloped tile has
  no texture it samples the land atlas at the diamond's own coordinates, and the
  bottom vertex lands exactly on `v + dv` — one texel into the sprite packed
  below it. One vertex of one triangle pair, so it is a hairline rather than a
  fringe, and the fix is the same inset `TexmapAtlas` already applies.
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
  *(Planned: [`unenforced.md`](unenforced.md) S4.)*
  Every assertion about `ground::collect` lives in `tests/frame.rs` behind
  `OPENSHARD_CLIENT` and a GPU, because the only way to get a `Map` is to load
  one from a file. A constructor taking cells — or a small fixture facet — would
  let the projection and the visible-set logic be tested with neither. Both
  atlases now take pictures directly (`LandAtlas::pack`, `TexmapAtlas::pack`), so
  the *art* half of that no longer needs an install: the test that a slope is
  drawn from its texture and a level tile from its art is green with no client at
  all. The map is what is left.
- **Nothing reads `Feature` or the client version yet.** The renderer draws what
  the files hold. That is right for ground, and it stops being right at the
  first packet the client draws from.

## Backlog, found while drawing the statics and the mobiles

- ~~**Nothing is hued.**~~ Statics and mobiles are now, from a real
  `HueRamp` (`crates/client/render/src/hue.rs`) built once from `hues.mul` and
  bound alongside every sprite atlas. No second texture carries the palette
  index: the atlas already stores each texel's red channel widened to eight
  bits — `Color16::rgb8`, the same widening every other reader here uses — and
  `statics.wgsl` recovers the file's 5-bit index from it exactly with
  `round(r * 31.0)`, because that widening is a bijection on 0..=31. A full hue
  replaces the pixel outright; a partial one (the wire hue's own top bit) only
  where the sampled pixel is genuinely grey (`r == g == b`). Not done:
  `tiledata`'s own `PartialHue` flag, which forces the same grey-only rule on
  an item regardless of what the wire hue asks — nothing in `crate::atlas`
  carries a tiledata reference for a static's sprite yet, so this needs a
  second graphic to test against before it is worth wiring in.
- ~~**Nothing animates.**~~ Given a clock. `crates/client/render/src/animation.rs`'s
  `AnimationClock` advances by real time and picks a frame out of however many
  the atlas actually packed, looping over the animation's own length rather than
  a constant. What times it turned out not to be `animdata.mul` — that file
  times an *animated static* (`AnimatedStaticsManager`, a torch or a fire) and a
  spell *effect* graphic (`GameEffect`), each cycling a short run of consecutive
  graphic ids, and neither is a mobile's body. `Mobile.ProcessAnimation` reads
  `Constants.CHARACTER_ANIMATION_DELAY` instead: a fixed 80ms, unscaled unless a
  server explicitly set the animation with its own interval byte (`0x6E`/`0xE2`)
  — and that constant, not the file, is what `FRAME_DELAY` cites. `client/app`
  also did not draw the mobile pass at all until now: the atlas and pipeline
  existed and nothing called `mobile_pass.render`.
- **The clock assumes a packed animation has no gaps.** `AnimAtlas::build`
  keeps a frame's own index and only drops the entry when the frame is blank,
  so a blank frame in the *middle* of a real group — which the file format
  allows, see `AnimFrame`'s own docs — leaves a hole in the key space that
  `frame_count` does not report. `AnimationClock::frame` cycles `0..frame_count`
  assuming those are the packed indices, so a caller unlucky enough to hit such
  a body would have the mobile vanish for one tick rather than loop cleanly.
  Body 400 group 4 does not hit this, which is why the clock does not
  compensate for it; the fix is `AnimAtlas` exposing the actual packed indices
  rather than a count, whenever a body that needs it turns up.
- **Equipment, mounts and corpses are not drawn.** `MobileView` layers a body,
  its clothes and what it is riding, each from its own animation, and this draws
  the body alone. That is the next thing a real character needs and it is
  entirely additional: layers are more sprites at the same depth.
- **`anim2` through `anim5` are openable and not addressable.** `Anim::from_files`
  takes a pair, but the index arithmetic implemented is the first file's, and the
  others re-base the three body kinds differently. Left undone rather than
  guessed: a wrong base reads a real creature's frames.
- **The UOP animations are not read at all.** A modern install ships both
  `anim.mul` and `AnimationFrame1.uop`, and the client prefers the UOP where a
  body exists in it. Everything a human needs is in the `.mul` on 7.0.116.0, so
  this is not blocking — but a body added after the `.mul` stopped being updated
  would be missing here and present in ClassicUO, which is a confusing way to
  find out.
- **A mobile's own `z` is the caller's problem, and getting it wrong looks like
  a bug in the renderer.** A body at the camera's height rather than the
  ground's is *correctly* hidden by the terrain in front of it, which is
  indistinguishable from a mobile that failed to draw — it cost a debugging pass
  here. Whatever eventually feeds `WorldView` into this will get the height from
  the server and be fine; `client/app` reads it from the map.
- **Two sprite passes mean two atlases and two pipelines.** They are the same
  pipeline built twice, which is the cost of a draw call binding one texture. If
  a third sprite layer arrives — equipment — it is worth asking whether one
  atlas keyed by a tagged id beats three of these.

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
- ~~**`enter_world` drops everything between `0x1B` and `0x55`.**~~ Applied.
  That window *is* the world being handed over — the player's own `0x20` and
  `0x78`, a `0x78` for everyone already on screen, the ground items — and none
  of it is sent again, so the loop that waited for permission to draw was
  discarding what it was going to draw. What it exposed is that two of those
  packets name the client's *own* serial and mean something different when they
  do: a `0x78` about ourselves is the one paperdoll a shard ever sends us (the
  reveal pass shows a mobile to everyone except itself), so it dresses
  `Player`, which now carries an equipment list and is no longer `Copy`; a
  `0x77` about ourselves is not a move at all and is dropped, because acting on
  it would fight `Walk`'s prediction. Both are routed by serial, which is what
  keeps `WorldView::mobiles` the *other* mobiles it claims to be. The e2e test
  asserts the backpack every character wears, since the only packet that
  mentions it lives inside that window.
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
