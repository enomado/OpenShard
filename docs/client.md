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

**And one command runs both ends, with no network under them.**
`crates/e2e/playground` is that same arrangement with a window instead of
assertions:

```sh
cargo run -p openshard-playground -- --client "/path/to/Ultima Online Classic"
```

Every option is also the `OPENSHARD_*` variable it used to be, and a `.env` at
the workspace root is read before the command line, so in practice the install
is named once in `.env` (copied from `.env.example`, and never committed) and
the command is `cargo run -p openshard-playground`. `--help` is where both
spellings are written down; `--account` and `--character` pick which of the
stock development accounts to play.

A shard in a thread of its own, the window logged in to it, both ending
together — and **no port bound and no socket opened**. The two are joined by
`tokio::io::duplex`, a pair of in-memory pipes.

### The transport is a parameter at both ends

This is the part worth writing down, because it is not a shortcut for a
playground: it is the seam a world driven by something other than a person needs.
A virtual player walking, talking and being fuzzed at wants a connection per
player and no file descriptors, no ports and no kernel timing — and it must
exercise *this* login machine and *this* framing, not a second implementation
that agrees with them.

So each end names what it needs and nothing more:

- **The client asks a `Dial`** (`crates/client/net/src/transport.rs`) for its two
  connections. `Tcp` is a real client on a real network; `e2e`'s `InProcess`
  hands back a pipe. Two methods rather than one, because the two connections are
  not the same question: the first goes where the player said, the second goes
  where the *server* said in its `0x8C` relay — and an in-process shard
  advertises an address it never listens on, so it must be free to ignore the
  second without guessing which call it was looking at. `Socket` became
  `Socket<S>`; nothing above it changed.
- **The gateway serves a stream** (`gateway::Gate`). `ClientGatewayServer` is now
  that plus a listener, and `client_session_serve` is generic — so an in-process
  client goes *through* the gateway rather than around it.

One thing had to become explicit in the move. The write task used to close the
connection by dropping an `OwnedWriteHalf`, whose `Drop` shuts the socket's write
direction; `tokio::io::split`'s half does not, so the client's zero read — which
the whole teardown chain hangs on — would have arrived for a socket and never for
a pipe. The hang-up is now a `shutdown()` that is written down, and it means the
same thing for every stream. Two tests in `gateway::server` pin it.

What the pipes do *not* reproduce: segment boundaries, resets, Nagle, and a slow
reader filling a kernel queue rather than blocking a writer. The socket tests in
`crates/e2e/shard` cover those and stay exactly where they are —
`tests/in_process.rs` is the same login again with the transport swapped, and
its deadlines are there because a broken pipe arrangement hangs rather than
refusing.

### Stopping is one word, and everything hears it

A shard has three loops that never end on their own — the accept loop, a
read/write pair per connection, and the tick — and it used to have no way to end
any of them. `run_shard` left its loop on Ctrl-C, which the tick listened for
itself, and everything else ran until the process did.

So there is now a `gateway::Shutdown`: a value that is cloned and carried down
the call tree, not a signal handler and not a flag some module owns.
`ClientGatewayServer::bind` takes one, `Gate::new` takes one, every connection
task holds one, and `run_shard` takes the same one. Ctrl-C in the binary and a
handle in a test produce the same stop, on the same paths.

It is **level-triggered**, and that is the design rather than an implementation
detail: `requested()` resolves the moment the stop *has been asked for*, not the
moment it is asked for. A connection accepted one instant before the stop would
otherwise be served forever by a shard that had already saved and gone. It is
also what makes it safe in a `select!` loop — cancelling a waiter loses nothing,
because the thing waited for is a state and not an event.

What a stop does, in order: the listener stops accepting and is dropped, so the
port is free and a late client is refused rather than let into a world that is
saving; every connection task hangs up, which the client sees as the zero read
it would get from a process that had exited; and the tick leaves its loop, ends
every trade, takes one last full snapshot and **awaits** the save task. So
`run_shard` returns only once the world is on disk, which is what makes it
something a caller may wait for.

`crates/e2e` is where that last part matters. `spawn` hands back a `Running`
beside the address — stop it, or drop it, and the shard stops and its thread is
joined. Fifty worlds started and dropped is now fifty threads that end, which is
what a fuzzing run needs and what the old arrangement could not do at all.

What this does *not* yet do — `SIGTERM`, the bytes still in the outbox, and
telling the player why the world went away — is [`shutdown.md`](shutdown.md), a
plan of its own.

Two smaller decisions. The shard is given the same install the window reads
(`world.client_files`), because the client predicts each step's `z` from its own
copy of the facet and two ends reading different ground is a stream of `0x21`
rollbacks that looks like a client bug. And none of this lives in `client/app`,
which is why that crate is now a library with a thin binary — the move
`crates/server/server` already made, and for the same reason: something that
wants a client should call one rather than build one.

Backlog it leaves behind:

- **The facet is read twice in one process**, once by the shard and once by the
  window, because `Map` is loaded from a path by each end and neither knows the
  other is in the room. A few hundred megabytes, paid twice, and the same
  question the "a container is read whole into memory" item below is about. The
  honest fix is a `Map` that can be handed over rather than opened again, and
  M3b's `Arc<Map>` cache is where that belongs.
- **Nothing tests that the playground boots.** `tests/in_process.rs` covers the
  transport and `e2e/shard`'s others cover the wire; what the binary adds — a
  config with `client_files` set, and a window — is covered by running it. An
  `#[ignore]`d test that starts the in-process shard with a real install and
  enters the world *without* a window would cover everything but the GPU.
- ~~**The in-process shard has no way to stop.**~~ **Done.** A shard now stops
  on one word, and it is the same word wherever it comes from. See "Stopping"
  below.
- **A virtual player is now one type away.** `Dial` is the seam; what is missing
  is something that drives `client/net` without a window — the walk, the speech,
  and an oracle for what the world should have said. It belongs beside
  `crates/e2e/playground` rather than inside it.
- **It is the obvious place for M3b.** One process already holds a shard and a
  client, and `InProcess` clones: two sessions against one shard is one more
  dialler, and it would be a real test of "the files are loaded once per
  install".

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
`crates/client/app` puts it in a window: `cargo run -p openshard-client-app --
--client …` opens on Britain and the arrow keys walk the camera. The
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

**The window is joined to the wire.** `crates/client/app` logs in when it is
given an account and draws what the server has shown it — the character, and
everyone else on screen — with the arrow keys sending a `0x02` each and the
camera following the body the server confirms:

```sh
cargo run -p openshard-client-app -- --client … --account admin --password …
```

Without an account it stays the offline map viewer it was, which is the only
thing that runs against a facet nobody is serving.

The socket gets a thread and a current-thread runtime of its own
(`crates/client/app/src/link.rs`), because the event loop blocks on the
compositor and the runtime blocks on the socket, and neither can poll the
other. They exchange values: a `Facing` down, a whole `WorldView` back through
`EventLoopProxy`. Nothing about the protocol is decided there — `client/net`
owns the login, the walk and the view.

**Neither `0x22` nor `0x21` moves the body in `WorldView::apply`, and both move
it on screen.** The ack carries a sequence and no position, so the tile is the
one `Walk` asked for; the rejection is a rollback the view has no arm for. That
join is the one rule in `link.rs` and it is `fold`, tested without a socket or
a window: fold only one of the two and the client's own body stands still while
everyone else walks around it.

### What is still M3: a pass that blends

Everything drawn so far is opaque. Every pass writes depth and tests it, which
is what makes one ordering span three passes — and it is also the reason
`crates/client/render/src/cutaway.rs` *cuts* where the client *fades*. The port
of `UpdateMaxDrawZ` landed as booleans: a roof over the player is gone this
frame, where the client walks its alpha down 25 a frame and drops it at zero
about a fifth of a second later.

The blended pass is one step and it unlocks four things at once, which is why it
is written here rather than left as four backlog lines:

1. **`ProcessAlpha` whole.** `Cutaway::shows_static` and `shows_land` become the
   two ends of a ramp instead of a predicate, and `CalculateAlpha` is the ramp.
2. **`IsTranslucent`** — a window pane, a force field: alpha 178, and nothing to
   do with the cutaway.
3. **Foliage.** `CheckIfBehindATree`, `IsFoliageUnion` and `FOLIAGE_ALPHA`: a
   tree fades where a body walks behind it, and a tree is a *union* of graphics
   that has to fade as one or it fades in stripes.
4. **`HasSurfaceOverhead`.** A mobile under a `NoShoot` or `Window` static is
   drawn differently in the client (`AllowedToDraw`), which is what stops a body
   standing in a doorway from showing through the arch. It needs a 4x4 scan per
   mobile, cached on the mobile against `max_z`, and there is nothing to see it
   with until this pass exists.

The order matters: a blended quad must not write depth, or it blocks whatever is
underneath it — which is exactly the note ClassicUO leaves on its own mesh path,
where a fading static is pulled out of the GPU buffer and drawn through the CPU
transparent list *after* the mobiles. So the pass is a fourth one, after the
mobiles, reading the depth the other three wrote and writing none of its own.

## M3a — the camera, and a shell to look through

**Built.** `cargo run -p openshard-client-app -- --client …` opens on
Britain, the wheel zooms about the cursor, a middle-drag pans, `Home` re-locks
the camera to the body, and the three panels are on screen. What follows is the
design as it was argued, with the places the code went another way marked — each
of them found by writing it.

**How the eye follows the body has a plan of its own from here on:**
[`docs/camera.md`](camera.md). What is below is the projection, the zoom and the
shell — the geometry the camera is made of, which is settled. Which camera runs
on top of it is not, and the answer is a bench rather than an argument.

This client is deliberately not a copy of the client. The camera zooms, pans
freely, and can be unlocked from the body; the interface is egui windows and
panels rather than a wall of gumps. Those are decided together rather than one
at a time, because all three want the same thing the code does not have yet: an
honest, invertible map between where something is in the world and where it is
on the screen.

Today there is half of one. `project` turns a tile into pixels and
`Camera::to_screen` turns those into an offset inside the viewport, and there is
no way back — nothing here can answer *which tile is under the cursor*, which is
what a zoom about the cursor, a drag, and eventually M5's clicking all reduce
to. Both halves also return the same `ScreenPoint`, so world pixels and viewport
pixels are one type today and mixing them is not a compile error.

### Two spaces, and a type for each

Three coordinate spaces exist and only the first is named:

- **Tile space** — `Point { x: u16, y: u16, z: i8 }`, the server's, and the only
  one that ever goes on the wire.
- **World pixels** — what `project` returns. Origin at tile `(0, 0, 0)`,
  unbounded in both directions, `y` down, no camera in it at all.
- **Viewport pixels** — where a thing lands in the rectangle the world is drawn
  into: origin at its top-left, and *after* the zoom.

So `ScreenPoint` splits into `WorldPixel` and `ViewPixel`, which are the same two
fields and two different meanings — the newtype rule, applied to the one place in
this crate where a raw pair of `i32`s currently serves two masters. Neither gets
`From` or `Into`: the only thing allowed to move between them is a camera, and
a conversion that needs a camera is a method, not a coercion.

**Built, and the third space is real but has no type.** `ViewPixel` is a pixel of
the *image the world is drawn into* — the offscreen target, which is the viewport
only at zoom 1. A viewport pixel is therefore a third thing, and it exists: the
cursor arrives in one. It gets no newtype because it never travels —
`Camera::pick(x, y) -> WorldPixel` takes one and the zoom is undone inside that
call, so there is nothing to carry and nothing to confuse. A type for a value
that is born and consumed in one expression buys nothing.

The camera is then four things and one rule:

```rust
pub struct Camera {
    /// Where the middle of the viewport looks. Pixels and not a tile: a tile is
    /// 44 pixels across and a drag is one pixel at a time.
    eye: WorldPixel,
    zoom: Zoom,
    /// The viewport, in *physical* pixels — the rect the UI leaves free, not
    /// the window.
    width: u32,
    height: u32,
}
```

and the rule is that the two conversions are exact inverses:

- `to_view(WorldPixel) -> ViewPixel` — `(w - eye) * zoom + half viewport`
- `to_world(ViewPixel) -> WorldPixel` — `(v - half viewport) / zoom + eye`
- `to_screen(Point) -> ViewPixel` keeps its meaning as `to_view(project(p))`, so
  every existing caller in `ground`, `statics` and `mobiles` is untouched.
- `unproject(WorldPixel, z) -> (u16, u16)` is the named inverse of `project`,
  which exists implicitly inside `visible_tiles` today and is written out here
  because picking needs it and because a formula with no name gets a second copy.

**Two of those came out differently, and both because the zoom is in the blit.**

`to_view` and `to_world` have *no zoom in them at all*: they are
`w - eye + half(image)` and its exact integer inverse. The formula above —
`(w - eye) * zoom + half viewport` — would scale the geometry as well as the
blit, drawing the world twice as large into a target that is already scaled. It
also cannot be an exact inverse at `2/3`, so the round-trip property the "done
when" asks for would have had to become a tolerance. The zoom enters exactly
twice: in the size of the image (`Camera::render_width`) and in `Camera::pick`.

`unproject` returns `(i32, i32)`, for the same reason `TileBounds` holds `i32`:
world pixel space is unbounded, a pixel north of the map's corner *is* a negative
tile, and a `u16` would have to clamp — which invents a tile rather than
reporting one. The caller knows its map; this knows arithmetic.

`eye` is private with `Camera::look_at`/`Camera::eye` either side of it, because
"where the camera looks" is the one piece of state two writers already fight
over: `App::entered` pins it to the player and `App::step` moves it offline.
One method, and the lock below decides which of them may call it.

**The eye is whole world pixels, and the remainder lives in the input handler.**
At zoom 2 a one-pixel drag is half a world pixel, and an eye that carried the
fraction would put every sprite on a half-texel boundary for half of all camera
positions — the same class of defect as the half-texel inset two sections up,
except spread across the whole frame instead of one edge. So the drag
accumulates its remainder where the mouse deltas are summed and commits whole
world pixels to the camera.

### Zoom is a ratio, and the scale is applied once

`Zoom` is a fraction from a fixed ladder — `1/2, 2/3, 3/4, 1, 4/3, 3/2, 2, 3, 4`
— and not an `f32`. Three reasons, and the third is the one that decides it:
`Camera` is `Copy + Eq` and several tests compare cameras, which an `f32` field
takes away; the offscreen target's size has to come out the same integer every
frame or the world is reallocated on rounding noise; and a ladder is what a wheel
notch wants anyway. The type keeps its numerator and denominator private and
hands out `Zoom::scale_up`/`scale_down`, so a zoom off the end of the ladder is
not expressible.

The scale itself is applied in exactly one place: the world is drawn at 1:1 into
an offscreen `Rgba8Unorm` texture of `ceil(viewport * den / num)`, and that
texture is blitted into the viewport rect. Every quad, every atlas region and
every pixel-exact assertion in `tests/frame.rs` therefore keeps meaning what it
meant, because nothing in the three world passes learns what a zoom is; what is
new is one fullscreen blit and one uniform. It is also what ClassicUO does in
substance, and it is the only arrangement where the UI stays crisp at 1:1 while
the world is magnified — scaling the geometry instead would resample five-bit art
through a filter at every fractional step.

**Nearest above 1, linear below.** Magnifying pixel art, a texel has to stay a
square; minifying it, nearest samples one texel in four and the ground shimmers
as the camera walks. Two rules rather than one filter, and the reason is written
next to them.

**The zoom-out limit is the GPU's, and it is small.** WebGL2 guarantees only
2048 in each dimension, so a 1024×768 viewport at `1/2` already wants
2048×1536 and a 1080p window wants more than the floor allows. The ladder is
therefore clamped at runtime against `limits.max_texture_dimension_2d` and the
clamp is *reported*, because a silently truncated target draws a smaller world
into a larger rect, which looks like a bug in the projection. If that limit turns
out to bite on real hardware, the fallback is scaling the geometry after all —
recorded here so the choice is a measurement rather than a rediscovery.

**Zooming is about the cursor, not the centre**, which is the first thing the
invertible pair buys: hold `to_world(cursor)` fixed across the change and solve
for the new eye. One line, and it is the difference between a camera that feels
placed and one that feels shoved.

**Except while locked, where it is about the centre.** An eye pinned to the
cursor would be moved by the zoom and moved straight back by the next
`WorldView`, which is a fight rather than a camera. Locked zooms about the
middle, which is where the body is; unlocked zooms about the cursor.

**And the device can refuse.** The clamp is not only on the way down the ladder:
the image is `viewport / zoom`, so *growing the window* at a zoom that fitted
asks for a texture that does not, and nobody zoomed. So the fit is checked where
the size is used — `App::fit_zoom_to_device`, once per frame — and it steps the
zoom back in rather than letting `world_texture` fail validation. Checking it
only in the wheel handler passes every test anybody would write and breaks when
a window is dragged wider.

### The lock

```rust
enum Follow {
    /// The eye is the body's, and the server moves it.
    Body,
    /// The eye is the mouse's, and the body may walk off screen.
    Free,
}
```

It lives in `App` and not in `Camera`: the camera does not know what a player is,
and giving it one would put `client/net` inside `client/render`.

- `Body` is what happens today — every `WorldView` update calls `look_at`.
- `Free` means the view no longer moves the eye at all. Drag pans; the arrows
  still walk the character, because walking and looking are different questions.
- Re-locking snaps rather than eases. Easing wants a per-frame clock over a
  mobile that survives between frames, which is exactly what the "everything
  stands" backlog item below is waiting for, and both should be built once.
- Middle-drag pans, the wheel zooms, `Home` re-locks. The camera panel shows
  which mode is on and can toggle it, so the state is never invisible.

### The shell: egui, and the wgpu version it does not have

`egui-winit 0.35` works with our `winit 0.30` untouched. `egui-wgpu 0.35` is on
**wgpu 29** and this client is on **wgpu 30**, and the two do not mix: a resolve
puts both in the graph and a `Device` from one is not a `Device` for the other.
Downgrading is not free either — `Instance::new`, `CurrentSurfaceTexture` and
`queue.present` are all wgpu 30 shapes here.

**The port turned out to be four lines.** `RequestAdapterOptions` gained
`apply_limit_buckets`, `VertexState::buffers` became a slice of `Option`,
`AdapterInfo` gained `limit_bucket` and its `transient_saves_memory` became an
`Option<bool>`. `renderer.rs` — the part that actually draws — needed one of
them. Each is marked `wgpu 30:` in the vendored source and listed in
`vendor/README.md`, which is also where the exit condition lives.

So: **vendor `egui-wgpu`, port it to wgpu 30, and send the port upstream.** The
vendored copy lives in a top-level `vendor/` directory rather than in
`crates/*/*`, because the group is part of the path here and a third-party crate
belongs to none of the three groups; it keeps its own MIT/Apache-2.0 licence
files. `[patch.crates-io]` points at it. The exit condition is written into the
crate's own README: when upstream releases wgpu 30 support, the directory and the
patch are deleted in one commit. The fallback, if the PR stalls and the vendored
copy starts to rot, is a paint pass of our own — egui's output is clipped
triangle meshes and texture deltas, which is `SpriteRenderer` with a scissor
rect and no depth attachment.

Four things the integration has to get right, each of which is silent when wrong:

1. **Colour.** The surface is deliberately non-sRGB, and egui's shader assumes
   an sRGB target unless told otherwise — the usual symptom is a UI that is
   merely *slightly* too bright, which nobody reports as a bug. A flat panel of a
   known colour, read back and compared to the byte egui asked for, is the test;
   this is the "colour is never converted" rule meeting somebody else's renderer.
2. **Depth.** The UI pass takes no depth attachment. The world's depth buffer
   orders the world.
3. **Input.** `egui_winit::State::on_window_event` answers whether it consumed
   the event, and a consumed event must reach neither the camera nor the walk
   keys. One `if`, in one place — otherwise a drag inside a panel pans the world
   underneath it.
4. **Points against pixels.** egui lays out in logical points and the world is
   drawn in physical pixels, so the rect egui leaves free is multiplied by
   `pixels_per_point` before it becomes the camera's viewport. Getting this
   wrong is invisible at scale factor 1 and wrong on every HiDPI screen.

Layout: `CentralPanel`'s available rect *is* the world's viewport, so docked
panels shrink the world and floating windows sit over it. The camera's `width`
and `height` therefore stop meaning "the window" — which is already how resize
works, so it is one more caller of a path that exists.

**In egui 0.35 that rect comes from the root `Ui`.** The frame is
`Context::run_ui(input, |ui| …)`, panels are `egui::Panel::top(id).show(ui, …)`
inside it, and what is left is `ui.available_rect_before_wrap()` — there is no
`CentralPanel` to ask. Windows still take the `Context`. The consequence is the
same and the call is not, which is worth writing down once rather than
rediscovering at the next version bump.

The wait loop grows one term: `about_to_wait` currently re-arms from the
animation clock, and egui asks for repaints of its own, so the deadline becomes
the earlier of the two.

### What the first panels are, and what they are not

egui is a **dev-HUD** for now. Whether the client's real interface is egui or the
`0xB0` gump layer is M4's decision and this milestone must not take it by
accident, so nothing built here may reach into `client/net` or `WorldView` beyond
reading them:

- a status strip — connection state, our serial and position, frame time;
- a camera window — the zoom, the lock, the eye, the viewport size;
- a world window — the mobiles and ground items `WorldView` is holding, which are
  decoded and completely invisible today.

Deliberately absent: the journal, the paperdoll, containers. Those are M4, and
building them in egui now would decide M4 without arguing it.

### Done when

`cargo run -p openshard-client-app -- --client …` opens on Britain, the
wheel zooms about the cursor, a middle-drag pans, `Home` re-locks the camera to
the body, and the three panels are on screen. `cargo test --workspace` is green,
including: `to_world(to_view(w)) == w` over the whole ladder,
`unproject(project(p), p.z) == (p.x, p.y)`, the existing "every tile that lands
on screen is inside the bounds" property re-run at each zoom, and a frame test
that the blit at zoom 1 is texel-for-texel what the world pass drew.

**All of the tests are.** The blit test is the load-bearing one: with the world
drawn offscreen, every other pixel-exact assertion in `tests/frame.rs` is about
an image the screen never shows unless the blit is the identity at 1:1 — a
half-texel of sampling error, a flipped vertical axis or a filter left on all
read as "slightly soft" on a screenshot and are exact there. It needs no client
files, because a scene made of two gradient diamonds has the edges a flat wash
would not.

## Backlog, found while planning the camera and the shell

- ~~**`ScreenPoint` is two spaces under one name.**~~ Split into `WorldPixel`
  and `ViewPixel`, neither with `From` or `Into`.
- ~~**Two writers move the camera.**~~ `eye` is private and `look_at` is the one
  door; `Follow` decides which writer may open it.
- **`depth::base_for` takes a tile and a free camera has none.** It now gets the
  eye's unprojected tile, and it takes `i32` because that tile may be off the
  map. `DEPTH_TILES` is 512 either side and a zoomed-out viewport spans a few
  hundred, so the margin holds — but the slack is now a function of the zoom
  rather than a constant, and the test that pins it still does not say so.
- ~~**`the_bounds_do_not_grow_without_limit` asserts a constant.**~~ Now
  `the_bounds_do_not_grow_faster_than_the_image`, re-run at every rung against a
  bound derived from the image's size. So is
  `every_tile_that_lands_on_screen_is_inside_the_bounds`, whose "did anything
  land on screen" floor had to become the image's area in tiles for the same
  reason: a constant there is either a failure at 4x or an assertion about
  nothing at 1/2x.
- **The world texture and its depth buffer are recreated on every zoom step and
  every resize.** Correct and not free — two allocations of a few megabytes on a
  wheel notch. Nothing has measured whether it matters; a pool keyed by size
  would be the answer if it does.
- ~~**Nothing in `client/app` is tested.**~~ The arithmetic moved:
  `crates/client/render/src/control.rs` is the camera, who may move it, and the
  fraction of a world pixel a drag has not yet spent, with thirteen tests and no
  device anywhere near them. The lock came with it — `follow_body` is the rule
  `App::step` and `App::entered` used to write out as an `if` each — and the
  device's refusal became a `TooLarge` value the caller prints, because a
  renderer with a stderr cannot be run twice in one process. Writing the tests
  found one defect: `fit_to_device` had no exit at the top of the ladder, so a
  viewport larger than `max_texture_dimension_2d` — which no zoom can answer —
  spun. What is still untested in `client/app` is the glue: which egui rect
  becomes the viewport, and when a redraw is asked for.
- **`Camera::pick` exists and nothing picks.** Screen to *ground tile* is
  `unproject(pick(cursor), z)` and is one line away; screen to *what you
  clicked* is the depth ordering read backwards, which is M5. Worth not
  designing around the easy half.
- ~~**Zoom-out makes the whole-atlas rebuild fire far more often.**~~ There is
  no whole-atlas rebuild on a miss any more — the atlases grow, and a zoom step
  is one band of new tiles like any other camera move. What survives of this
  entry is the half it was really about: four times the screen is four times the
  graphics against a 2048 atlas that is also the browser's guaranteed floor, so
  zooming out is still the fastest way to *fill* one. That now lands on the
  eviction rather than on the frame rate.
- **Picking is half-built the moment `to_world` exists.** Screen to *ground tile*
  is the inverse projection at a known `z`; screen to *what you clicked* is the
  depth ordering read backwards, and a static or a mobile in front of the ground
  is what M5 will actually want. Out of scope here, and worth not designing
  around a ground-only answer.
- **A free camera can lose the character entirely**, with nothing on screen
  saying which way to look. An edge marker, or a "return" button in the camera
  panel next to the lock, is the cheap answer; the lock key already exists so
  this is polish rather than a hole.

## M3b — many shards, many characters

This client holds **sessions, plural**, and a session is a triple: a shard, an
account on it, and a character. The natural shape is therefore `[shard →
{characters}]` and not a flat list — several characters logged in to one shard
is the common case, several shards at once is the same machinery with one more
level, and neither is a special case of the other. A character select to say
which session is on screen, a map of the facet to say where the bodies are, and
the keyboard driving one of them or all of them at once, the way a strategy game
drives a group.

It is written down here, before M4, because it decides what "the client's state"
means. Everything the gump layer builds — a journal, a paperdoll, a container —
belongs to *a* character, and a status bar built against `App`'s fields rather
than against a session is a status bar that has to be written twice.

**A serial is unique on a shard and nowhere else.** `0x2A` on one shard and
`0x2A` on another are two creatures, so every map keyed by `Serial` — `Crowd`'s
clocks, `WorldView`'s mobiles and items — is inside a session or it is wrong.
This is the one mistake here that would not look like a bug: two characters on
two shards standing near each other's serials would simply animate each other.
The atlases are the other way round and are fine, because a graphic id belongs
to the *files*, not to the shard.

### One copy of the files per install, N of everything downstream of a socket

The whole design is that split, and it is not a preference: `App` holds a facet
of a few hundred megabytes plus `Art`, `TexMaps`, `TileData`, `HueRamp` and
`Anim` — 200MB of pictures read before the first frame. Ten sessions holding ten
of those is not a client. So the client's own data is loaded once and shared, and
everything that comes off a socket is per session and shared with nobody:

- **Shared, immutable, `Arc`, and keyed by install.** `Map`, `Art`, `TexMaps`,
  `TileData`, `HueRamp`. The precedent exists — the facet is already an
  `Arc<Map>` handed to the shard thread so `Walk::step` can predict a height.
  `Anim` is the exception and the awkward one: `Anim::frames` takes `&mut self`
  because reading a frame seeks the file, so it is shared behind a lock or it is
  read behind the atlas that already caches what it produced.

  *Keyed by install* is what multi-shard adds, and it is the reason
  `OPENSHARD_CLIENT` cannot stay a single environment variable. A 5.x shard and a
  7.x shard are not read from the same files, and a custom shard ships its own
  map and its own art — `docs/client_versions.md` is the standing rule that
  server and client must read the same `.mul`, and it applies once per shard
  rather than once per process. So the cache key is `(install, facet)`, two
  shards on the same install share everything, and two shards on different
  installs share nothing but the process.
- **Per shard.** The login server address, the relay it hands back, the feature
  mask, the version this client claims, and the `.def`/cliloc set the install
  supplies. **The version is the one that will bite**: it is a startup constant
  today, and every `Feature` gate on both ends follows from it — see "Which
  version we claim to be" below, which stops being one decision and becomes one
  per shard.
- **Per session, and never merged.** The connection, the `WorldView`, the
  `Walk`, the `Crowd`, the eye. `Walk` in particular: the step sequence and the
  fastwalk key are properties of *one* connection, and a shared one would ack the
  wrong session's step and desynchronise every character but one. This is the
  same rule the server lives by from the other side.

### What assumes one today

All of it, and none of it deeply. `App` (`crates/client/app/src/main.rs`) holds
one `link: Option<Link>`, one `crowd: Crowd`, one `control: Control`, one
`player`, one `others`, one `items`, one `view`, one `connection` string. The
window is woken with an `Update` that names no session, because there is only
one to name. Outside the struct the same assumption is in the environment:
`OPENSHARD_CLIENT`, `OPENSHARD_ACCOUNT` and `OPENSHARD_PASSWORD` are one install
and one account, and the shard address and the claimed version are constants in
`main`.

The shape that replaces it:

- A `Shard` — an address, an install, the version claimed to it, and the
  accounts on it. This is the level `[shard → {characters}]` names, and the
  level the file cache is keyed by.
- A `Session` — the link, the last `WorldView`, the crowd, the projections the
  renderer reads, and the account it logged in as. `App` holds a list of them and
  which one is drawn.
- `Update` becomes `(SessionId, Update)`, and `EventLoopProxy` carries the pair.
  A `Lost` is then one session ending, not the client ending — which is already
  what `link.rs` promises ("the window stays open on one of these") and cannot
  currently deliver, because there is nothing left to look at.
- **One runtime, N tasks — not N threads.** `link.rs` argues for a thread
  because the event loop blocks on the compositor and the runtime blocks on the
  socket. That argument buys *one* thread, not one per character: ten idle
  sockets do not want ten current-thread runtimes. The seam stays exactly where
  it is — a thread that is not the event loop — and the connections become tasks
  on it.

### Only what is drawn needs a renderer

The saving that makes the whole thing cheap. A session nobody is looking at
needs its `WorldView` and its `Crowd` — both plain data, both advanced by
packets and a clock — and no GPU state at all. The atlases, the world texture,
the depth buffer and the three passes belong to the *view*, of which there is
one, or two if a split screen ever happens.

This matters because the atlas is the tightest resource here already: it is
rebuilt whole whenever the camera walks off it, WebGL2 guarantees only 2048, and
zooming out was already making that fire more often. N simultaneous worlds would
turn an open question into a wall. N connections against one atlas does not.

### Where everybody is: the facet map

"Control several at once" is unusable without one picture that shows all of
them, and it is nearly free: one pixel per tile from `radarcol.mul` — still
unread, on the missing-readers list — plus a marker per session. It is an egui
image and shares nothing with the isometric renderer, which is what makes it
cheap. It also answers the standing backlog item that a free camera can lose the
character entirely, for every character at once.

One map per `(install, facet)` and not one per client: two shards are two
worlds even where the files agree, and a marker is placed on the map its session
is standing in. Which is also the honest answer to what the character select
shows — a tree of shards, each with the characters logged in to it, because that
is the shape the state already has.

### The keyboard, and who hears it

Three modes, and they are the same question the camera lock already asks:

- the drawn session only, which is what happens today;
- a selected group;
- everyone.

Broadcasting a step is N independent `0x02`s with N sequences, whose acks come
back interleaved and are folded per session. Nothing is shared and nothing is
synchronised — the client does not decide that two characters stepped together,
it decides that one key sent two packets. Anything cleverer (formation, waiting
for the slowest) is a layer above this and must not be built into the fan-out.

### Two things that stop being backlog and become blocking

- **The facet is a startup constant.** Two sessions may stand on different
  facets, so the single `Arc<Map>` becomes a cache keyed by facet, loaded on
  demand and shared by whoever is on it. `0xBF 0x08` is what says a session
  moved between them.
- **A whole `WorldView` is cloned per changed packet.** One standing character
  makes this invisible; ten characters beside a bank multiply it by ten, and the
  clone is of the map of every mobile each of them can see. Worth measuring
  before the count goes up, not after.

### What the shard permits is the shard's business

Each session is its own account and its own pair of sockets. A shard may refuse
several connections from one account or one address, and whether it should is
the operator's rule, not this client's — what this client owes is to report the
refusal *per session*, so one login failing is one row in the character select
and not the client giving up. Across shards the question does not even arise:
they have never heard of each other.

### The list has to live somewhere, and that is a decision

Three environment variables are a single session's worth of configuration. A
list of shards, each with an install path, a claimed version and its accounts,
is a file — and the moment it holds accounts it holds credentials, which is not
a thing to arrive at by accident:

- a password in a plaintext config is what every UO launcher has always done,
  and it is still the thing that leaks;
- the platform keyring is right and is a dependency and a headless problem;
- asking at connect time is free, correct, and unusable for the ten-character
  case this milestone exists to serve.

Deliberately unresolved here. What is decided is that the file names shards and
installs and *may* name accounts, and that whatever holds the password is behind
one seam rather than read wherever a login is built — because there will be a
lot of logins.

### Done when

Two accounts log in from one process, the character select switches which one is
drawn, the arrows drive the drawn one or all of them, and the facet map shows
every body. Two shards are configured and at least one test drives both.
`cargo test --workspace` is green, including a test with neither a window nor a
GPU that two sessions on one shard share one facet and two sessions on different
installs do not — `Arc::ptr_eq` both ways, because "the files are loaded once per
install" is the property the whole milestone rests on and it regresses silently
in either direction: a second copy is invisible until the memory runs out, and a
wrongly shared one draws a 5.x shard's world out of a 7.x shard's art.

## M4 — the gump layer

The journal and the speech line, the status bar, the paperdoll, containers, and
generic gumps.

**Partly built, ahead of the rest of M4**, because the shard's staff commands
are unreachable without it: they are `.`-prefixed *speech*, and `.admin` is
answered with a gump. What landed is the whole path and none of the art —

- `0xAD` encodes (`UnicodeTalkRequest::encode`) and `0xB0` decodes; `0xB1`
  encodes. Each is the direction the engine had never needed until it grew a
  client of its own.
- `protocol::gump::layout::parse` reads the layout language the builder next
  door writes, and is tested against it element by element. Total: an unknown
  keyword is an `Element::Unknown`, never a lost window.
- `WorldView::gumps` holds what is open, and `gump_closed` is the one thing the
  wire never says — a reply button closes a window client-side.
- `client/app/src/gump.rs` draws a layout with egui's own widgets, and
  `shell.rs` has a speech line. Both are the *dev HUD's* rendering, in the same
  spirit as the rest of that file.

What is still M4 proper: the gump art (`gumpart.mul`), which is why a
`{ gumppic }` is drawn as a placeholder naming its graphic; hue lookup for gump
text; and the journal, of which the speech strip is a six-line stand-in.

## M5 — interaction

Single and double click (`0x09`, `0x06`), drag and drop (`0x07`, `0x08`),
targeting (`0x6C`, `0x6B`), war mode. Speech (`0xAD`) landed early — see M4.

## Decisions to take before they are taken by accident

- **The client is multi-shard and multi-session**, `[shard → {characters}]` —
  see M3b. It is a decision and not a feature, because it says three things that
  are cheap now and an audit later: everything downstream of a socket is per
  session, the data files are loaded once *per install* rather than once per
  process, and the claimed version belongs to a shard rather than to the client.
  The same argument as multi-era on the server, and for the same reason.
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
  branches of every encoder. **Per shard, not per client** (M3b): the whole point
  of `Feature::since` is that a connection asks its own version, and a client
  facing two shards at two versions is exactly the case an era check gets wrong
  silently.
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
- ~~**The atlas is rebuilt whole whenever the camera walks off it.**~~ It grows
  instead: `LandAtlas::add` and its three neighbours pack what is new beside
  what is there, and only the rows that changed are uploaded. The eviction this
  entry asked for exists too, as the answer to an atlas that has filled up
  rather than as the answer to a miss.
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
- **A pier or bridge's deck has no ground plane of its own.** `GroundQuad`
  (`ground.rs:17-24`) builds its four heights from the land layer only —
  documented as deliberate ("there is no single height to fold" from a single
  land tile) but a platform static (a pier, a bridge, stairs) is exactly a
  second surface at its own height, and `statics.rs` draws it as a sprite over
  the land quad without ever raising the quad or adding a plane at the static's
  own top. Combined with the walk backlog entry below (the avatar's predicted Z
  on such a tile also comes from the land layer, never the static), a walk onto
  a pier draws the avatar sinking into a ground plane that was never the deck to
  begin with — reported by a player 2026-08-02 as falling underground,
  specifically on piers and bridges. The two entries are one bug with two
  causes: the drawn floor is wrong here, the predicted Z is wrong in
  [`Walk::step`](#backlog-found-while-joining-the-window-to-the-wire) below.

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
- ~~**Equipment is not drawn.**~~ Worn items are, now. A worn item is not
  drawn from its own art — its *default* picture is its own tiledata entry's
  `AnimID` field (`StaticTile::anim_id`, `crates/common/uofiles/src/tiledata.rs`
  — present in the file and in this reader's own layout comment the whole
  time, but never actually read until now), the same index space and the same
  `anim.mul` machinery the body itself draws through. `Equipconv.def` only
  *overrides* that default, for the `(body, AnimID)` pairs where this body
  needs a different picture — chiefly a race or gender variant of the same
  garment — which is why an ordinary shirt has no entry in it at all. Getting
  this backwards was the first cut at this feature: treating "no entry" as
  "draw nothing" instead of "the default already draws right" silently
  dropped every piece of plain clothing on every NPC, caught only by looking
  at a live client rather than by any test, because every test here packs its
  own atlas and never has an item *without* a conversion entry to notice the
  gap. `openshard_uofiles::equipconv::EquipConv` reads `Equipconv.def` — text,
  not one of this crate's binary formats, the same shape as `Body.def` — and
  `mobiles::collect` pushes one extra `SpriteQuad` per layer at the *same*
  `depth::Order` as the body, so the existing stable sort draws it on top with
  no second depth pass. Only a layer whose resolved `AnimID` the atlas has no
  frame for this frame is dropped, the same rule a missing body animation
  gets.

  **Mounts and corpses are still not drawn** — a mount replaces or extends the
  body draw rather than layering over it, and neither was touched here.
  Left out of the same pass, deliberately:
  - **`mobtypes.txt` and the gargoyle graphic offset.** Nothing here tells a
    gargoyle body from a human one, so a gargoyle's equipment resolves through
    the same table a human's does and comes out wrong or absent.
  - **Paperdoll layer ordering.** `PaperdollOrder`'s T1/T2/T3 tables and
    point-fix rules (cloak-by-facing, helmet-over-hair) have no counterpart
    here — `Layer` is a bare `u8` on purpose (see its own doc comment), so
    layers draw in whatever order the server listed them in `0x78`, which is
    usually close to right and not guaranteed to be.
  - **Incremental equip/unequip.** `Equipment` only ever changes as a
    full-list replacement inside a fresh `0x78`; there is still no `0x2E` or
    per-item wear/drop decoder, so a shard that only updates one worn slot
    rather than resending the whole list will not be reflected here.
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

  *And read it from the wrong place, which is the second half of the same
  entry.* A land cell stores **one** height and it is the diamond's northern
  vertex, not the height of the tile: a body stands at the average of the four
  corners (`Map::average_land_z`, RunUO's `GetAverageZ`, ClassicUO's
  `Land.AverageZ`). `link.rs` predicted each step's `z` from the raw corner while
  `MapTerrain::ground_z` on the server had always used the average — the two ends
  each land their own step because a `0x22` carries no `z`, so the disagreement
  was silent and permanent, up to the whole relief of a tile. On a slope that
  draws the character sunk into the hillside *and* behind it: the ground's own
  depth is that same average less two, so once the body's `z + 1` falls under it
  the land is not merely near the walker, it is in front of him. One formula now,
  on the map where both sides can reach it, with the corner walk beside it.
- **A body between two tiles sorts at the nearer of them, and taking the
  destination is wrong for half the compass.** `View.CalculateDepthZ` hides the
  rule behind eight arms of a `switch` over the sprite's drift, because the
  reference keeps `Mobile.X/Y` on the tile a step *started* from until the step
  lands. Read whole, the eight arms are `max(from, to)` — plus one where the two
  are equal, which is the north-east/south-west pair that moves straight across
  the screen. Sorting at the destination unconditionally is that maximum only for
  the four headings that go *down* the screen; for north, west and north-west the
  destination is the farther tile, so the ground being stepped off — and every
  wall standing on it — was drawn over the walker for the length of the step.
  `depth::mobile_tile` is the rule, `Mobile::from` is what it needs, and
  `Crowd::stepping_from` is where a step's clock decides it: tied to the *glide*
  and not to `Tracked::step`, which outlives the crossing by the half step the
  animation is deliberately held for.
- **`CalculateDepthZ`'s `z += max(0, Offset.Z)` is not ported.** The reference
  raises a body's priority by its mid-step *height* offset, on the east and south
  arms only. Two of eight is the kind of asymmetry that is either load-bearing or
  a leftover, and nothing here can tell which without stairs to walk up; the tile
  half of the ordering is ported and this is not. Revisit when a body first walks
  a staircase facing east.
- **A tile with no texmap is stretched here and flat in the reference, and that
  now reaches the *ordering* too.** `ground.wgsl` already records the divergence
  in what is drawn. What is new is that `Land.ApplyStretch` gives such a tile
  `AverageZ = MinZ = z` — the raw corner — so ClassicUO both draws it flat and
  sorts it at the corner, where we draw it stretched and sort it at the average.
  Ours agrees with the *server*, which is the agreement that matters for where a
  body stands; but a client-side height that disagreed with the reference client
  on the same map is worth knowing about before the flat/stretched decision is
  ever revisited.
- ~~**Two sprite passes mean two atlases and two pipelines... if a third
  sprite layer arrives, does it need a third?**~~ It arrived, and it did not:
  an equipment layer's resolved body-anim graphic reads through `Anim::frames`
  exactly the way a body does, so it packs into the *same* `AnimAtlas` under
  its own `FrameKey.body` — no tag needed, because two different resolved
  graphics are already two different keys, and two equipment layers that
  happen to resolve to the *same* graphic correctly share one packed frame,
  the way two mobiles of the same body already do. Still two passes: the third
  layer turned out to be more entries in the mobile atlas, not a third pass.
- ~~**The labels still are not drawn.**~~ They are now, above whoever the
  crowd last heard from. `crates/common/uofiles::font::AsciiFonts` reads
  `fonts.mul`; `crate::atlas::FontAtlas` packs every glyph it defines — all
  ten faces, unconditionally, because unlike a graphic there is no "not
  currently visible" character — into a fixed grid cell-sized to the tallest
  one packed, the way the backlog entry below guessed. `crate::text::collect`
  turns a line into glyph quads, left to right by each glyph's own width
  (`fonts.mul` carries no kerning table) and centred on an anchor rather than
  left-aligned to it, drawn through the same `SpriteRenderer` and `statics.wgsl`
  as everything else: font pixels are grey and a wire hue replaces them exactly
  the way a partial hue replaces a static's, so no shader of its own was
  needed. `crates/client/app/src/crowd.rs` gained the other half — `Crowd::hear`
  and `Crowd::speaking`, a serial's last line and when it arrived — and
  `App::entered` calls `hear` once per new journal tail line, matched to a
  drawn mobile through `mobiles::head_anchor`, the top-centre of exactly the
  rectangle `mobiles::collect` would draw for it (refactored out from under
  both so the two cannot disagree about where a body's head is).

  Three corners cut, each because the wire does not yet carry what a correct
  answer needs:
  - **`fonts.mul` is still unconfirmed against a shipped file.** No client
    install was on hand while writing either the reader or the atlas above
    it, so — alone among `uofiles`'s readers — it has no counterpart in
    `tests/client_files.rs`, and the byte layout is the widely-documented
    community format rather than a fact this crate has confirmed on real
    bytes. First thing to do with a client tree in hand.
  - **The hold is a flat five seconds, not the message's own length.** The
    reference client's `Mobile.m_SpeechTime` grows with the text; nothing
    that reaches here — not the wire's `0x1C`, not `SpokenMessage` — carries
    an expiry, so `crowd::SPEECH_HOLD` is a constant standing in for one.
    Threading a real duration through means deciding whether it is computed
    at the packet boundary or carried on the wire, which is `client/net`'s
    question and not this pass's.
  - **A label is always drawn nearest, not depth-tested against the world.**
    `Label::depth` is wired but `client/app` always hands it `0.0`: there is
    no `depth::text_priority_z` the way there is a `mobile_priority_z`, so a
    line is never hidden by a wall in front of its speaker. Reads right for
    every case tried so far — overhead text is an overlay in the reference
    client too — and is the thing to revisit first if that stops being true.
  - **Only the newest journal line is ever heard.** `App::entered` compares
    `view.journal.back()` against the previous view's, so two speakers
    between one `WorldView` update and the next share one slot and the
    older one's line never appears, however briefly. The journal itself
    drops nothing — this is a gap in what gets a bubble, not in what gets
    remembered.

  `unifont.mul` — the Unicode faces gump text and gump title bars use — is a
  separate reader again and is not started; `fonts.mul` was picked first
  because it is what `speech::Font` already names.

- **A repack that fails is a `eprintln!` and a frame that draws anyway.**
  *Half answered — the atlases now evict, so "full" is recoverable.* When a
  growth returns `AtlasError::Full`, `App::draw` packs an atlas for what is on
  screen *now* and throws away everything the camera has walked past. That is
  the eviction policy this entry asked for, and it was not optional: an atlas
  that only ever grows has no other way to reclaim a graphic, where the old
  rebuild-on-every-miss did it by accident. Measured against a real install, the
  start tile (1495, 1629 on Felucca) sees 187 distinct static graphics and 136
  tiles out it is 588, so the fill is reachable by walking rather than a corner
  case. What is *still* an `eprintln!` and a frame that renders is the case
  underneath it — one screen's statics not fitting one 2048 atlas, which no
  eviction can help. That wants a texture array or several atlases, and it is a
  different statement from "the atlas filled up".
- ~~**`stale` cannot become false when a visible static has no art.**~~ Fixed,
  and the fix is that the question is no longer asked. Every atlas records what
  it was *offered*, not only what it packed (`StaticAtlas`'s `asked`), so a
  graphic the client ships no art for is read once and skipped for ever after —
  where before, one such tile on screen repacked every atlas on every frame,
  because a graphic that cannot be packed is never packed. The staleness check
  it fed is gone too: what a frame needs that the atlas has not seen is now a
  question about the tiles the camera crossed, not about the graphics on screen.
  See the entry on the four walks below.

## Backlog, found while joining the window to the wire

- ~~**Everything stands.**~~ ~~**One animation clock for everybody.**~~ Both
  were the same missing thing, and it is `crates/client/app/src/crowd.rs`: a
  position, a group and a clock per serial, above the view and below the
  renderer. It lives in the app because it reads `client/net` and writes
  `client/render`, which may not depend on each other.

  What that turned up is worth more than the animation: **the three body kinds
  number their groups differently, and group 4 is not "standing".** It is
  `PeopleAnimationGroup.Stand` for a player, `HighAnimationGroup.Attack1` for a
  monster and one past `LowAnimationGroup.Eat` for a horse — all three exist, so
  the old constant failed at nothing and drew the wrong action forever.
  `BodyKind::standing`/`walking`/`running` answer it now, pinned in a test
  beside the enums; a monster's `running` is `None`, because `High` has no run.

  Nothing on the wire says "stopped walking", so a step is heard rather than
  seen: a position change starts a walk and `WALK_HOLD` — `WALK_INTERVAL`
  twice, from `common/movement` rather than chosen to look right — ends it. A
  *turn* is not a step, which is what a layer watching the facing instead of the
  position would get wrong while passing every other test.

- ~~**A step is a teleport of one tile.**~~ Glided. `Mobile` carries a `Glide` —
  the tile stepped off and how far along the body is — and
  `mobiles::world_position` hangs the sprite between the two projections.
  Everything else still reads `Mobile::at`: the depth order is the *destination*
  tile, or a body would change sides of a wall halfway through a step. Three
  things it turned up, each of which the glide alone would have looked wrong
  without:

  - **The eye has to glide too.** `Control::follow_body` took a tile, so a
    character sliding smoothly across the world had the whole world jumping a
    tile at a time underneath it — worse than the teleport, because it is the
    *ground* that jerks. It takes a `WorldPixel` now, and there is still one door
    to the eye.
  - **The hold is also the step's length, so it is not one number.** A runner
    steps every 200ms — ServUO's `RunFoot`, `RUN_INTERVAL` doubled the way
    `WALK_HOLD` doubles `WALK_INTERVAL` — and glided over a walk's 400ms it would
    be half a tile behind itself and jump forward at every step. `RUN_HOLD` is
    the wire's own running flag applied to both.
  - **The window redrew on the animation clock**, 80ms, which is right when the
    only thing that changes is a frame index and gives a glide five visible jumps
    instead of a slide. `App::redraw_interval` has two rates and
    `Crowd::anyone_gliding` picks between them; the crowd is advanced by
    *measured* time now rather than by the interval that was waited for, since
    `WaitUntil` is a floor and a stepping animation hides the overshoot where a
    glide does not.

  A move of more than one tile is not glided at all: a gate, a recall or a `0x22`
  putting a mispredicted body back would otherwise slide the character across
  half a facet over 400ms, which is a stranger picture than the teleport it hides.
- ~~**Nothing ever asks to run.**~~ Shift does. Everything downstream of the
  wire's running bit was already built — the server's `WalkPace` charges
  `RUN_INTERVAL`, `Crowd` holds and glides a runner for `RUN_HOLD`, and
  `BodyKind::running` picks the group — and the only thing missing was a client
  that set the bit. What writing it turned up is that **the pace is input, not
  output**: a step used to be sent from the key event, which made the operating
  system's auto-repeat the walking speed — half a second of nothing, then thirty
  steps a second, and the fast half is exactly what `WalkPace` refuses as a
  speedhack, so the shard answers `0x21` and the body is pulled back. That reads
  as the walk stuttering rather than as the client asking for too much, which is
  the wrong bug to go looking for.

  So a direction is *held* rather than pressed, and the clock is ours:
  `crates/client/app/src/keys.rs` says which way the keyboard is pointing and
  `steer.rs` sends one step every `WALK_HOLD`, or every `RUN_HOLD` with shift
  down. The rate is the hold and not `common/movement`'s interval on purpose —
  those are anti-speedhack *floors*, deliberately half the real rate, and walking
  at the floor would move a body twice as fast as the crowd glides it. Two
  releases that never arrive have to be caught for a held key not to walk for
  ever: the window losing focus, and egui taking the event.

- ~~**The mouse cannot walk.**~~ A right click is a move order: the body walks to
  that tile on its own, and holding the button steers it to wherever the cursor
  is — the strategy game's idiom and the 2D client's right-hold, which turn out
  to be the same feature stated twice. Left stays the Tile panel's and the middle
  button still pans.

  What writing it decided: **the pace belongs to neither input.** A second timer
  beside `keys.rs`'s would take two steps a beat the moment somebody nudged an
  arrow while walking to a destination, so `steer.rs` owns *one* clock and the
  two inputs are only sources of a direction — the keyboard winning, and a press
  dropping the destination rather than queuing behind it. `keys.rs` is now the
  arrow stack and nothing else.

  ~~The route is greedy — the straight-line direction, a step at a time —
  because this end has no walkability to plan over.~~ It plans now — see the
  next entry.

- ~~**A click-to-walk destination does not route around anything.**~~ Planned.
  `Steering::go_to`/`due` take a `&dyn Terrain` and run `common/movement::find_path`
  on the click, then walk the returned route one direction per step instead of
  `direction_toward(from, goal)` (`client/app/src/steer.rs`). The three decisions
  the entry named, and how each landed:

  - **Where the check lives.** Lifted: `MapTerrain` and its `check` moved from
    `server/world/src/terrain.rs` into `common/movement/src/terrain.rs`, beside
    `find_path`, generic over `M: AsRef<Map>, T: AsRef<TileData>` so the server
    keeps building one owned at boot (`MapTerrain::new(map, tiles)`) and the
    client builds one borrowing (`MapTerrain::new(self.map.as_ref(),
    &self.tiledata)`) fresh per click rather than cloning the facet. `world`'s
    own `terrain.rs` is now a two-line re-export plus the one test
    (`the_layer_byte_reads_the_hand_a_weapon_is_held_in`) that needs
    `openshard-state`'s layer constants, which `common/movement` may not depend
    on.
  - **A client cannot see the dynamic half.** Unchanged, and still the reason a
    plan is a guess: `Obstructions`/`LiveTerrain` (`server/state/src/obstruct.rs`)
    stay server-side. The `0x21` stall detector (`STUCK_STEPS`) is still the
    correction for what the plan could not know.
  - **Replanning cadence.** Plan on the click (`Steering::go_to`); on a step that
    left the body exactly where it was — a refusal — `Steering::take` replans from
    the body's real position before trying again, rather than repeating the same
    refused step until `STUCK_STEPS` gives up.

  Two things the fix turned up that were not in the plan:

  - `find_path`'s A* is tied on Chebyshev cost between a straight cardinal line
    and any equal-length route that drifts diagonally off it and back, so an
    axis-aligned click could come back zig-zagging. `common/movement::path`'s
    open list now breaks ties by Manhattan distance to the goal, which is
    smaller for the route that stayed straight — a tie-break, not a second
    heuristic, so it does not change *whether* a shortest route is found, only
    which equally-short one A* settles on. This also straightens whatever else
    calls `find_path` (an NPC's chase), not only the click.
  - **The first cut of this ran `find_path` on every mouse-move, not every
    step, and froze on some routes.** `go_to` is called on the click *and*
    again on every raw `CursorMoved` event while the button stays down —
    `client/app/src/lib.rs`'s `walk_to_cursor` — which is tens of events a
    second while dragging, not one. Planning eagerly there meant an A* search
    that many times a second, and a destination expensive to search (out of
    reach, so every search burns the whole node budget) froze the window for
    as long as dragging lasted. `go_to` now only ever restates *where* — it
    drops the stale route and leaves the new destination unplanned — and
    `Steering::take` is the sole place a search runs, gated by the same step
    cadence as a step itself, so a plan costs at most once per
    `WALK_HOLD`/`RUN_HOLD` no matter how fast the cursor moves.
    `restating_a_destination_mid_step_does_not_search_the_terrain`
    (`client/app/src/steer.rs`) pins it with a terrain that counts its own
    calls. `PLAN_BUDGET` also came down from a first guess of 4,000 to 600,
    in line with `common/movement`'s own "a few hundred is ample for a town" —
    the eager version's cost per search was the bigger problem, but an
    unreachable destination still pays the full budget once a step, `STUCK_STEPS`
    times over, and a smaller cap bounds that too.
  - **The default right-hold was still `go_to` — a destination, not a
    heading — which is the wrong idiom for "run toward the cursor" and is its
    own entry next: a body doing nothing but chase the cursor around a room
    would occasionally lurch at a pillar directly under it, refuse, snap back,
    and restart the walk animation for it.

- **The mouse's held-right-button idiom was one input made to do two jobs, and
  the seam showed as a lurch into a pillar.** The click-to-walk fix above made
  a destination's refusal-and-fallback behaviour reasoned about and tested,
  but a player dragging the mouse to say "run this way" was still, underneath,
  issuing a stream of *destination* orders — one per cursor tile — and a destination
  that cannot be reached degrades to walking at it anyway, refusal by refusal,
  which is exactly the honest behaviour a real move order wants and exactly
  the wrong one for a heading a player is only pointing, not aiming.
  `client/app/src/steer.rs` now answers two different questions from the
  mouse, matching what was already true of the keyboard versus a click:

  - **`Steering::steer(direction)`** — the default right-hold, no modifier.
    Not an order to reach a tile: a compass heading from the body to the
    cursor, recomputed every move and driven exactly like a held arrow key.
    It has no notion of arrival or of being stuck, and it never touches
    `find_path` or the map — but a blocked direction is no longer walked into
    forever either: [`detour`](../crates/client/app/src/steer.rs) tries the
    nearest way still legal past it (an O(1) `Terrain::can_step` look, not a
    search), so a runner slides past an obstacle instead of standing against
    it. What "legal" means is not symmetric: a wall dead ahead of a held
    *cardinal* has no diagonal past it at all — the server's own corner rule
    (`LiveTerrain::can_step`, and `find_path`'s `corner_open`) requires both
    cardinal tiles flanking a diagonal step to be open, and the blocked
    direction is unconditionally one of those two flanks for either diagonal
    beside it, so neither ever passes; `detour` offers the cardinal along the
    wall's face instead, the same sidestep a body hugging a wall makes. A
    blocked *diagonal*, pinned by a corner rather than a wall, has the
    opposite shape: the two cardinals it splits into have no corner to cut,
    so those are what is tried. Offering the wrong one of the two — a
    diagonal past a wall, which an earlier version of this did — is not a
    cosmetic bug: the body is drawn slipping through the wall's corner for a
    round trip and rubber-banded back, worse than the stand-and-bump this
    replaced, on every retry for as long as the direction is held. This
    applies from the very first ask, not just the steps `Steering::due`
    answers afterward: `Steering::steer`/`press` now take a `terrain` too
    (constructed on demand at their call sites in `App`, the same
    `MapTerrain` `due`/`go_to` already built) and route through the same
    `Steering::take` `due` does, rather than answering directly. That first
    ask is not the rare case for the mouse heading in particular — a player
    working a corner is actively moving the cursor, and every sector change
    is a fresh `steer()` call; answering those without the detour meant a
    player *trying* to route around a corner hit the undetoured path on
    almost every attempt, and only the occasional still-held, re-asked-at-
    the-next-hold heading ever saw the fix. Released, it stops at once;
    unlike a destination, there is nothing behind a heading once nobody is
    pointing it any more.
  - **`Steering::go_to(tile)`** — unchanged in mechanism, now reached only by
    holding Ctrl. The real move order: `find_path` plans a route, a refusal
    replans, and — the one behavioural change here — a destination `find_path`
    proves has no route at all no longer gives up outright. It falls back to
    the same straight-line heading `steer` would use, still under the
    `STUCK_STEPS` patience, because Ctrl+drag is an explicit "go to this exact
    spot" and walking up to an obstacle and stopping (classic UO's own answer
    to clicking on a wall) is the right honest answer for *that* idiom — the
    lurch was never the fallback itself, it was the fallback firing for input
    that never meant to ask a pathfinding question in the first place.

  `keys` still outranks both, and `go_to`/`steer` clear each other, so exactly
  one of "arrows", "heading" or "destination" drives a step at a time — see
  `Steering::asking`. `a_heading_never_gives_up`,
  `releasing_the_mouse_stops_the_heading_but_not_the_keyboard`,
  `the_keyboard_takes_over_from_a_heading` and
  `a_destination_with_no_route_falls_back_to_a_heading_then_gives_up`
  (`client/app/src/steer.rs`) are what pin the split.

- ~~**The walk stuttered once a tile.**~~ Three causes, all of them in the same
  400ms:

  - **The glide's length was the nominal step and the steps do not arrive
    nominally.** Finish early and the body stands on its tile waiting for the
    next packet; finish late and that packet yanks it forward from wherever it
    had got to. So a walk already under way crosses each tile in the time the
    *last* crossing took (`crowd::glide_time`), believed only within half and
    double the wire's own claim — outside that band the gap is not a pace but a
    body that had stopped, or two steps in one burst. This is also the only thing
    that can glide an NPC correctly: nothing on the wire says what pace a
    creature walks at.
  - **The animation hold was that same one number**, so it expired in the gap
    between two steps — one frame of *standing*, which is a different group, so
    the walk's clock restarted at frame zero every tile. The crossing and the
    animation are two numbers now: `animation_hold` keeps the walk playing half a
    step past the landing, and a body that has genuinely stopped walks on the
    spot for 200ms that nobody notices.
  - **The animation clock was armed at the standing rate when the step
    arrived.** `FRAME_DELAY` is 80ms and a crowd where nobody is gliding waits
    it out, so the first 80ms of every glide was drawn frozen at its start.
    `App::user_event` pulls the tick forward to `GLIDE_INTERVAL` when the packet
    it just folded in started somebody moving.

- ~~**Our own body waited for the round trip.**~~ It is predicted now, which is
  the fourth and largest share of that same stutter: a step used to be drawn when
  the `0x22` acked it, so the body stood still for the latency, crossed its tile,
  and stood still again — and the *jitter* of that latency, not the latency
  itself, is what no interpolation can smooth out, because it moves the start of
  each crossing rather than its length.

  `Walk` already kept a `predicted` and deliberately refused to write it into the
  `WorldView`; that refusal is right and stands. What was missing was a second
  channel: `link::Update::World` now carries a `link::Body` — the prediction, and
  whether it got there by a correction — beside the view, and the app draws the
  body from *that* while everything else still reads the view. The ack becomes
  invisible: it confirms a position the screen already has. Only a `0x21` or a
  `0x20` changes anything, and it is a rollback.

  Two things the rollback turned up:

  - **A rollback must not be glided.** It is one tile, so the "more than one tile
    is a teleport" rule does not catch it — and glided, a body refused by a wall
    strolls *backwards* a tile every step. `Crowd::snap` puts it there instead,
    and deliberately leaves the animation alone: a walker whose third step is
    refused is still walking.
  - **A rollback is not a pace sample.** The gap between a step and its refusal
    is latency, and feeding it to `glide_time` would have the next crossing take
    a quarter of a step. `snap` drops the measurement with the step.

  What is deliberately *not* predicted: whether a step is allowed at all. That
  needs every rule about statics, doors and mounts to agree exactly with the
  server's, and being wrong about it costs a rollback where being wrong about a
  height costs a few pixels. The server is the authority and the `0x21` is how it
  says so.

- ~~**Every unit of the walk is tested and the walk itself is not.**~~ It is now:
  `crates/client/app/src/dst.rs` runs the whole path — `steer.rs`,
  `client/net`'s `Walk`, a real `openshard_movement::Walker` for the shard, and
  the `Crowd` — on a virtual clock, over a wire with latency, jitter and a wall
  in it, and holds the position of the *sprite* against an oracle.

  The oracle is the **intent** timeline: the body leaves the instant the key goes
  down and crosses one tile per hold, for ever. It is built from the script of
  inputs alone, it is constant velocity and nothing else — no turn tax, no ramp,
  no easing. Everything under test is the **event** timeline — when the loop
  woke, what the wire did — and the claim is that the second reproduces the
  first. Not a tautology: every walking bug this client has had
  is a divergence between those two sets of knots, and the harness found three
  more that four green unit suites did not.

  - **A turn stopped the pace measurement without being one.** `glide_time`
    measures the gap since the last *position* change, and a turn changes no
    position — so the step after a turn was measured across two holds, which the
    band was just wide enough to believe, and the tile after every turn was
    crossed at half speed. A turn records the pace sample now: it is a step in
    UO, it just covers no ground.
  - **The crowd's clock was a frame behind whenever a packet was folded into
    it.** A step is timestamped with `Crowd`'s own `now`, and `user_event` folded
    packets in between two `advance` calls — so every step was recorded at the
    *previous* frame's instant, up to a whole `FRAME_DELAY` for a body that had
    stopped. `glide_time` takes a difference of two of those, so the error landed
    on the crossing's length. `user_event` advances the crowd before it folds.
  - **The event loop's wake jitter accumulated into the walking speed.**
    `steer.rs` armed the next step at `now + interval`, and the loop is woken by
    the operating system whenever it gets round to it and never early. A few
    milliseconds a step is a body a fifth of a tile behind after ten and a whole
    tile behind after fifty, and nothing ever gives it back. The next step is
    armed from the deadline that has just passed; a wake later than a whole step
    is a stall rather than jitter and restarts the cadence, because those steps
    are deliberately not banked.

  One design decision came out of it: **the pace of our own body is not measured,
  it is commanded** (`Crowd::commanding`). `glide_time` exists because nothing on
  the wire says how fast a creature walks — but we send our own steps, so the
  nominal hold is not an estimate of that walk, it *is* the walk. Measuring it
  anyway feeds the loop's wake jitter into the crossing length, and consecutive
  gaps jitter in opposite directions, so the estimate was worse than the constant
  it replaced.

- ~~**A turn cost the player 400ms of standing still.**~~ It costs nothing.
  Turning is a whole step in UO — the mobile turns, moves nowhere, and gets its
  own `0x22` — and `steer.rs` used to charge it a whole hold, so pressing a new
  direction stood the character still for a step before it set off. Nothing asked
  for that: the shard answers a turn *before* it charges the pace budget
  (`Walker::request`, and the reference it is ported from does the same, because
  spinning on the spot is something clients do and throttling it would be
  absurd), so the step a turn precedes is legal in the same instant. `steer.rs`
  arms it at once and `App::about_to_wait` takes up to two steps in one wake, so
  the turn and the step it is for leave together. The oracle in `dst.rs` is what
  states the requirement: it charges nothing for a turn, and a walk that starts
  facing the wrong way tracks it from the first millisecond.

- ~~**The animation clock was read when the timer fired, not when the frame was
  built.**~~ A glide is a position read off a clock, so the moment that clock is
  read has to be the moment the picture is built. `App::about_to_wait` advanced
  the crowd and then asked for a redraw; between the two the loop laid out the
  UI, grew an atlas and waited on the swapchain, and however long that took was
  error in the body's position — error that varies frame to frame, which is what
  an eye reads as a stutter rather than as lag. `App::draw` advances the clock at
  the top of the frame now.

  With it, the other half of the same judder: the timer's 16ms beat against the
  display's 60Hz, so a frame landed on the wrong side of the beat about once a
  second. A body mid-step asks for its next frame at the end of `draw` instead,
  and the surface's FIFO presentation paces the walk at the display's own rate.
  The timer stays for everything else — a still world redraws on the animation
  clock and sleeps in between.

- **The camera is on whole world pixels.** `Camera::eye` is an integer
  `WorldPixel` and a step east crosses 22 of them in 400ms, so at 60Hz the ground
  moves 1, 1, 0, 1 pixels a frame rather than 0.92 — a half-pixel wobble at zoom
  1 and a whole one at zoom 2, which is the last quantisation left in a walk.

  **Worth knowing before changing it: ClassicUO does not solve this, it has the
  same quantisation.** `Mobile.Offset` is three `sbyte`s and
  `GameSceneDrawingSorting.UpdateDrawPosition` builds the scene's origin out of
  `int`s (`winGameCenterX -= (int) Player.Offset.X`), so the reference client
  everyone calls smooth is also stepping the ground a whole pixel at a time. Its
  `Renderer/Camera.cs` keeps floats and lerps — but only for the *peek* offset
  and the zoom, and `Camera.Transform` casts to `int` at the end. So a fractional
  world is ours to invent rather than ours to copy, and the reason to want it is
  zoom: at 2× a whole world pixel is two screen pixels, which is the case where
  the judder is actually visible. The cost is that a sprite quad on a fractional
  boundary samples its atlas unevenly and the art shimmers inside the sprite,
  which is why pixel-art engines that do this snap *sprites* to whole pixels and
  let only the ground carry the remainder.

- **The camera tracks `z` exactly, and stairs bob.** A step up is four world
  pixels of vertical (`camera::project`'s `Z_STEP`), and the eye follows every
  one of them. ClassicUO does the same thing —
  `winGameCenterY = ... + (Player.Z << 2)` plus the interpolated `Offset.Z`, no
  damping anywhere — so a bobbing camera on a staircase is what UO looks like,
  not a defect we introduced. Damping it is a deliberate improvement and it is
  cheap: the eye already takes a `WorldPixel`, so a critically-damped follow on
  the vertical axis alone (spring, or a first-order lag with a time constant
  around a step) leaves the horizontal walk exactly as it is. It has to be
  *bounded* — an eye that lags a recall or a teleport is worse than one that
  bobs — which is the same "more than one tile is not glided" rule the crowd
  already has. `Camera::eye` is an integer
  `WorldPixel` and a step east crosses 22 of them in 400ms, so at 60Hz the ground
  moves 1, 1, 0, 1 pixels a frame rather than 0.92 — a half-pixel wobble at zoom
  1 and a whole one at zoom 2, which is the last quantisation left in a walk.
  Fixing it means a fractional eye whose remainder is applied to the ground
  diamonds and the sprite quads (both are already `f32` at the GPU), and it is
  not free: a sprite quad on a fractional boundary samples its atlas unevenly, so
  the art shimmers inside the sprite. Worth measuring before doing — the two
  clock defects above were the visible share of this complaint.

- **The picture and the truth are the same number, and they should not be.**
  This is one mechanism from modern third-person and isometric action games —
  Diablo, Path of Exile and everything shaped like them — and it is the answer to
  three separate complaints here. Those games are continuous where UO is a grid,
  but the part worth taking is not the continuity, it is the *split*:

  - An entity has an **authoritative** position — what the server said, plus
    whatever the client has predicted on top — and a **drawn** position, which is
    the authoritative one plus an error that is decaying toward zero. Everything
    that is not the picture reads the authoritative one: the depth order, what
    the body can walk behind, what the camera says is on screen.
  - A correction never moves the picture. It moves the authoritative position at
    once and *puts the difference into the error*, which then shrinks with a
    half-life of something like 100–150ms. The body is where the server says it
    is immediately, and it arrives there smoothly. Unreal calls this
    `NetworkSmoothingMode`/`SmoothCorrection`, Source calls it `cl_smooth` with a
    `cl_smoothtime` of a tenth of a second, and every engine has one.
  - The error is **bounded**: past a threshold the correction is a teleport and is
    snapped, because sliding a body across half a facet is a stranger picture than
    the jump it hides. The same rule the crowd already applies to a move of more
    than one tile.
  - The decay is **frame-rate independent**: a half-life and `0.5^(dt/half)`, not
    a per-frame `lerp(a, b, 0.1)`, which is a different curve at 30fps and 144.

  For us that is a drift in world pixels on `Mobile`, set by `Crowd::snap` from
  where the body was actually drawn, decayed in `Crowd::advance`, and added in
  `mobiles::world_position` — where the camera reads it too, so both follow. It
  replaces "a rollback must not be glided" with something better than either
  answer: the tile the body was put back on is still never *walked* across, but
  the picture is not yanked either. It also absorbs whatever jitter is left in
  the arrival of a step, which is the same defect wearing a different hat.

- **The eye follows `z` exactly, and a staircase bobs.** The second thing those
  games all do: the camera is a *smoother over* the character, per axis, with
  different time constants — the horizontal tight or exact, the height loose,
  a few hundred milliseconds, because terrain height is the axis that steps. A
  camera locked to the character's exact height turns every stair into a jolt of
  the whole world, and no isometric game since about 2005 ships that.

  Cheap here, and independent of everything else: `Control` keeps the eye's
  height as an `f32`, pulls it toward the height the body is drawn at with a
  half-life around 300ms, and lifts the eye by the difference times
  `camera::Z_STEP`. The horizontal axis stays exact for now — a lagging eye on
  the walking axis is a separate decision with its own feel, and it is not what
  the stairs complaint is about. `Home` and `relock` stay instant: a rubber band
  on a deliberate jump is a bug, not a feel.

- **A fractional world, eventually.** With the two above done, what is left of the
  judder is the whole-pixel eye, and the fix is the one modern pixel-art engines
  use: keep the camera's position fractional, give the *ground* the remainder,
  and snap *sprites* to whole pixels so their art does not shimmer. Worth doing
  for zoom, where one world pixel is two screen pixels or more. Last, because it
  touches every quad and the two above touch a dozen lines each.

- **Nothing is predicted about whether a step is allowed, so a wall is a
  rollback.** This is the largest remaining gap against the reference, and it is
  what "the client does not respect being pushed back" and "obstacles are not
  walked around" both come down to. What ClassicUO does, from the clone:

  - **It never asks for a step it knows is illegal.** `PlayerMobile.Walk` calls
    `Pathfinder.CanWalk` before it queues anything and returns `false` if the
    ground refuses — so walking into a wall produces *no packet at all* and no
    rollback. The body walks on the spot, which is what a player expects, rather
    than lurching a tile and being pulled back.
  - **A refusal stops the walking.** `WalkerManager.DenyWalk` clears the step
    queue, resets the sequence, forces the position, and
    `SyncServerDirection()`s the facing; a `0x22` for a sequence it is not
    waiting on sets `WalkingFailed` and sends a resync — and `WalkingFailed` is
    the *first* condition in `Walk`, so nothing more is sent until the resync
    lands.
  - **The queue is capped.** `Constants.MAX_STEP_COUNT = 5` unconfirmed steps,
    and `Walk` refuses to add a sixth. Ours caps nothing on purpose (`Walk::in_flight`
    says so), which is fine while the shard is the only limit and wrong the
    moment the shard stops answering.
  - **Click-to-walk is A\*.** `Pathfinder.FindPath` with
    `PATHFINDER_MAX_NODES = 10000` and a Chebyshev heuristic, over the same
    `CanWalk`. Ours is greedy-with-a-stall-counter, which is the honest thing to
    do with no terrain and the wrong thing to keep once there is one.

  We can do better than the reference here rather than copy it: ClassicUO
  re-implements UO's walkability in the client and can therefore *disagree* with
  the shard, while `openshard_movement::Terrain` is one trait both ends already
  speak and `crates/server/world/src/terrain.rs`'s `MapTerrain` implements it out
  of `Map` + `TileData` and nothing else — no world state, no server crate. Moving
  it below `server/` (it is `common/*` material sitting in the wrong group, and
  `crates/common/movement` already owns the trait) gives the client the shard's
  own rules, byte for byte. Then, in order:

  1. `MapTerrain` moves to `common`, with the server importing it from its new
     home. No behaviour changes; the test suite that pins its Sphere and RunUO
     arithmetic moves with it.
  2. `Walk::step` gains the terrain and refuses locally, exactly where its doc
     comment currently explains why it does not. That comment stops being true
     the moment the two ends share an implementation, which was its own premise.
  3. `steer.rs`'s greedy route becomes `openshard_movement::find_path` — already
     A\* with a Chebyshev heuristic over the same `Terrain` — and `STUCK_STEPS`
     stops being the only answer to a wall.
  4. ~~A rollback tells `Steering`, which it currently does not.~~ Done with the
     queue rule below: `Steering::corrected` takes the shard's facing and
     `App::entered` calls it whenever `link::Body::corrected` is set.
  5. ~~A cap on steps in flight, with the reference's five as the number.~~ Done:
     `walk::MAX_IN_FLIGHT`, checked first thing in `Walk::step` as the reference
     checks it first thing in `PlayerMobile.Walk`. It is not a second pace limit
     — the shard's budget is the only judge of how fast a body walks — it is the
     answer to a shard that has *stopped answering*, where every further step is
     another tile of correction when the link comes back. `Walk::step` now
     answers `NotSent`, which is that refusal and the world's edge in one type;
     `link.rs` logs either and sends nothing, so the body waits where it is.

- ~~**An input takes a step whenever it arrives, and the step under way is cut
  short.**~~ Fixed, and the rule it was fixed with is worth stating on its own
  because everything about walking now depends on it:

  > **An input joins the queue or rebuilds it. A step already begun ticks out.**

  Two complaints, one defect. Walking east and pressing west mid-stride jumped
  the camera; mashing the arrows sent the body flying off its own position and
  being dragged back. Both were `Steering` sending a step at the moment an input
  arrived rather than at the moment the walk was free for one — a turn costs the
  shard nothing, so the turn *and the step behind it* went out on every press,
  and a release disarmed the clock entirely, so press-release-press bought a step
  per tap.

  Three things go wrong at once when a step leaves early, which is why the rule is
  one rule and not three fixes:

  - **The picture.** `crowd.rs` starts each glide at the tile the *previous* step
    ended on, so a step issued half a hold early yanks the body forward to a tile
    it has not reached — half a tile in one frame — and the camera is locked to
    the drawn body, so the world jumps with it.
  - **The pace.** The shard's `WalkPace` refuses a body asking for steps faster
    than a body walks and answers `0x21`, which is the flying-off-and-being-
    dragged-back.
  - **The wire.** That rollback races the steps still in flight; their acks arrive
    for a sequence this end has forgotten, and `Walk::on_packet` calls that an
    `UnexpectedAck`. `link.rs` treats one as fatal, so a determined key-masher
    could *drop their own connection* — see the backlog below.

  The mechanism is small: `Steering::due` stops being "the walk is running" and
  becomes a floor that nothing clears, `Steering::free` is the one gate every ask
  goes through, and a turn no longer moves the deadline — it leaves in the same
  wake as the step it precedes and that step is what charges the clock, so the
  pair is one ask against the floor. What the queue *is* is `Steering::take`
  reading the keys at the moment the step leaves rather than when they were
  pressed: one step deep, rebuilt by every press for nothing.

  The oracle in `dst.rs` gained the same rule — a press while a step is under way
  moves no knot — and four scenarios hold the picture to it: the reversal, twenty
  reversals at 270ms so every phase of a step is interrupted, thirty presses a
  second through three directions, and one arrow tapped. Two assertions beyond
  the corridor, because a corridor is blind to a jump forwards and back inside
  it: `continuous` bounds how far the drawn body may move between two frames by
  what a walk covers in that time, and `paced` bounds how close together two
  crossings may be asked for. All four failed before the fix, by 0.5 tiles, 1.89
  tiles, a dropped connection and 0.74 tiles respectively.

  One trap found on the way, and it is the reason `Steering::walking` exists: the
  next step is measured from the *deadline* rather than from the wake, which is
  what stops a late loop accumulating drift — but a deadline that came and went
  with the arrows up is not a cadence. Measuring from it made the step after a
  fresh press due a fraction of a hold later, which cut the glide short and
  jumped the body exactly like the defect being fixed. A deadline is only a
  cadence if a step was taken at it.

- ~~**An ack that arrives after a rollback ends the session.**~~ Fixed, and it was
  two bugs wearing one hat. Found by the key-mashing scenario in `dst.rs` before
  the queue rule removed the flood that provoked it: a `0x21` voids everything in
  flight and resets both sequences, but the shard owes one answer per `0x02` and
  the steps already on the wire are still answered — so an answer lands for a
  sequence this end has forgotten.

  Both halves were wrong. `Walk` called those answers a desync, which they are
  not: the wire delivers in order, so while anything is owed from before the last
  correction the next answer is one of *those*. `Walk::draining` counts them and
  they are swallowed — including a stale `0x21`, which is the half that had no
  symptom anybody would have named: applying it rolls the body back a second
  time, onto a tile it has already walked away from, and clears the steps sent
  since. The DST scenario measures exactly that, and without the counter the
  drawn body ends four tiles behind the shard. An answer owed to *nobody* is
  still an `UnexpectedAck`, so a real desync is still reported.

  And `link.rs` turned any error out of `fold` into `Update::Lost`, so the window
  closed. What is left after the drain is a genuine disagreement, and that has an
  answer on the wire rather than a reason to hang up — which is the item below.

- ~~**Neither end speaks the resync request.**~~ Both do now, and it is what makes
  stopping the walk safe. The whole cycle, and it is a request/response rather
  than a hope — the argument, read out of both references, is in
  `docs/findings.md`:

  1. An answer this end cannot place sets `Walk::out_of_step`, and while it holds
     `Walk::step` sends nothing (`NotSent::OutOfStep`). It has to: `predicted` is
     a chain of asks the server has stopped agreeing with, so every step on top
     widens the disagreement, and a `0x22` ack carries no position to correct it
     with.
  2. `link.rs` sends one `ResyncRequest` — the client's `0x22`, three bytes —
     guarded on the flag not already being set, which is ClassicUO's
     `ResendPacketResync` in one line.
  3. The shard decodes it as `ClientPacket::ResyncRequest`, queues
     `Command::Resync` like every other packet, and `WorldState::resync` answers
     out of a tick: the walk sequence back to zero, this client's screen
     forgotten so that `refresh_around` sends it again, and a `0x20` with the
     real position. That is ServUO's `Resynchronize` list in our own terms.
  4. The `0x20` snaps the client, which clears the flag, and the walk is free —
     from a fresh sequence on both ends, which is why the step after a resync is
     not refused.

  `0x22` is **two different packets**, one per direction, three bytes each, with
  nothing in the body to tell them apart. It costs us nothing because
  `ClientPacket` and `ServerPacket` are separate tables, but it is exactly the
  sort of thing a single id-to-type map gets silently wrong, so both types name
  each other and sit together in `world.rs`. `crates/e2e/shard/tests/resync.rs`
  is what proves the packet one end sends is the packet the other end decodes;
  neither unit test can.

- **The walk has no home.** Two of those three defects lived *between*
  `App::user_event` and `App::about_to_wait` rather than in any of the four units
  the walk is made of, and the harness had to copy those handlers' ten lines to
  reach them — which is exactly the thing a test must not do, because a
  divergence introduced into the copy is invisible to it. What is wanted is a
  headless unit that owns the walk end to end: the steering clock, the
  prediction, the crowd's clock and the order the three are touched in, driven by
  `(now, input, update)` and answering `(steps to send, where the body is drawn,
  when to come back)`. `App` becomes the window's adapter to it, and the oracle
  drives the shipped code rather than a copy of it. Everything in this entry
  argues for it; nothing here is a reason to have waited for it.

- **The crowd cannot tell a mount from a body on foot, and a mount steps twice as
  fast.** `WALK_HOLD`/`RUN_HOLD` are the two on-foot rates; ServUO's other two,
  `WalkMount` (200ms) and `RunMount` (100ms), have nothing here to select them —
  the mount is not on `0x77` at all, it is an equipment layer on `0x78`. So a
  mounted mobile is held and glided at half the speed it is really moving, which
  looks exactly like the runner case above. This wants the same `MobileView`
  layering the "equipment, mounts and corpses" entry does, and should land with it.
- **`Home` still snaps.** `Control::relock` takes a tile and jumps the eye to it;
  the next frame's `follow_body` then puts it back on the glided pixel. Harmless
  and visible for one frame — the snap is deliberate, the *inconsistency* between
  the two doors is not.
- ~~**Ground items are decoded, held, and not drawn.**~~ Drawn.
  `crates/client/render/src/items.rs` is two of the existing collectors put
  together: an item's picture is a static's, and its source is a mobile's — a
  list somebody else built out of what arrived on the wire. The placement has
  one copy, `statics::stand_on`, and one atlas serves both, because a floor tile
  packed twice is a floor tile twice. What that made visible: "does the atlas
  cover this frame" has to be *one* question — asked twice with the item half
  forgotten in one of them, the atlas is rebuilt every frame an item is on
  screen and never holds it, which is a stutter rather than an error.
  Deliberately not done: an item's *amount*. A pile of 500 gold is one sprite
  here, where the client picks a different graphic per size band.
- **The facet is a startup constant and `0x1B` only carries a size.** The app
  loads Felucca and compares the shard's map size once, warning when they
  differ rather than following. Following means decoding `0xBF 0x08` and
  reloading the facet, and the reload is the interesting half: `Map::load_facet`
  reads a few hundred megabytes. **M3b makes this blocking**: two sessions may
  stand on two facets, so the single shared `Arc<Map>` has to become a cache
  keyed by facet.
- **A whole `WorldView` is cloned per changed packet.** Fine for the handful a
  standing character receives, and not fine beside a crowded bank: the thread
  clones the map of every mobile to say that one of them turned. The answer is
  probably not a delta protocol between the two threads but a shared snapshot
  the window reads — worth measuring before deciding, and **M3b multiplies it by
  the number of sessions**, so the measurement should happen before the count
  goes up rather than after.
- ~~**`z` still drifts on a hill, and now it is visible.**~~ Fixed by handing
  `Walk::step` the ground as a function of a tile; the window shares the facet
  it already loaded with the shard thread through an `Arc`, since it is plain
  data read by both and written by neither. Deliberately the *ground* and not
  `movement::Terrain`: this predicts a height and must not predict a refusal —
  whether a step is allowed is the server's answer, and deciding it here would
  need every rule about statics, doors and mounts to agree exactly. What is
  still flat: a step onto a *floor* — a building's second storey is a static,
  not land, and the height predicted for it is the ground underneath. A pier
  or a bridge is the same case with a visible symptom rather than a subtle
  one: reported by a player 2026-08-02 as falling underground specifically on
  piers and bridges, because the predicted Z sits at the water or ravine floor
  under the deck rather than the deck itself, and `ground.rs` draws no plane
  at the deck's height either — see the matching entry in "found while
  drawing the ground". `App::walk`'s offline path (`lib.rs:1167`) has the
  identical gap: `self.map.land(x, y)`, no static.

## Backlog, found while building M0, M1 and M1a

Each is a seam the work made visible. None blocks the next milestone.

- ~~**A walking client has no map, so `z` never changes under it.**~~ It has
  one now: the height is an argument to `Walk::step`, and `|_, _| None` is what
  a caller without a map passes — the e2e walk test, which is about the sequence
  seam. See the entry above.
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

## Backlog, found while giving the client a speech line and the gump reader

- ~~**`0xAE` does not decode, so a client never sees its own words.**~~ Fixed.
  `UnicodeMessage` decodes, and the journal is now `VecDeque<Heard>`: `0x1C` and
  `0xAE` are one event in two encodings, so the journal holds a type that says
  so rather than one of the two packets standing in for both. Both fold in
  through the same cap. Nothing above `client/net` had to change for it — the
  overhead speech and the HUD's strip read the same fields — which is the sign
  the type was the right seam.
- ~~**The gump reader ignores hue.**~~ Fixed for text: a `{ text }` or
  `{ croppedtext }` hue is looked up in `hues.mul` and the label is drawn in it,
  which is what makes `.admin`'s "lay the world down" verbs read green and its
  "clear" verbs red. The column is ClassicUO's `HuesLoader.GetUnicodeFontColor`
  — `ColorTable[8]`, cited beside the constant, and pinned by a test on a ramp
  with a different colour in every column, so a wrong column reads as a wrong
  colour rather than a plausible one. `Hue(0)` stays "no colour", not row zero.
- **The gump reader draws no art.** `gumppic`, `tilepic` and `gumppictiled` are
  still placeholders naming their graphic. For a menu of labelled buttons
  nothing is lost; for a paperdoll or a shop everything is. The hue of *those*
  elements is dropped with them — art is tinted per pixel through the ramp the
  way statics are (`client/render/src/hue.rs`), not with the single solid
  colour text takes, so it lands with the art and not before. M4.
- ~~**A gump is drawn in points, not in the client's pixels.**~~ Withdrawn: this
  was written as a bug and is a decision, now argued in `client/app/src/gump.rs`
  and localized there. A layout's coordinates *are* the reference client's
  pixels, but converting them to physical pixels would be wrong on both counts —
  what is drawn here is egui widgets, whose text and padding are measured in
  points, so scaling the coordinates alone pulls the rows together underneath
  text that did not shrink with them; and the reference client predates display
  scaling entirely, so "what it does" is nothing and copying that gives postage
  stamps on a 4K screen. It stops being a decision the day gump *art* is drawn,
  because a bitmap cannot be reinterpreted — and then one scale has to apply to
  the coordinates **and** the font sizes together. Every layout number now
  passes through `gump::point` or `gump::size`, which is where that day's change
  goes.
- **A radio group is every radio in the layout, not every radio on the page.**
  `client/app/src/gump.rs` clears the other radios when one is set, across the
  whole window: no dialog this engine draws has two groups, so nothing shows the
  difference yet. The client's own rule is per page, and a pack's gump with two
  groups on one page would answer with both set.
- **`{ nodispose }` is not honoured and right-click dismisses nothing.** There
  is no right-click dismissal on this side to suppress, so the flag is read and
  dropped. It becomes real the moment the window gets one.
- **The speech line has no history and no modes.** No up-arrow recall, and
  everything is said as `TalkMode::Regular`: emote, whisper and yell are the
  same packet with another mode byte, and there is nothing in the UI to pick one.

## Backlog, found while chasing a slow debug build

- ~~**A frame walks the visible rectangle four times.**~~ Twice now, and the two
  that went were the expensive ones: `ground::visible_graphics` and
  `statics::visible_graphics` walked ~9,800 cells at 1080p on every frame purely
  to answer "is the atlas stale", against a camera that had moved one tile.
  `TileBounds::difference` subtracts the rectangle the atlases were last grown
  for from the one the camera wants and hands back the two or three thin bands
  between them — a step of one tile is one row — and `ground::graphics_in` /
  `statics::graphics_in` walk those. `App::covered` is the rectangle, and the
  invariant it carries is positional: every cell inside it has been offered to
  the atlases, so a graphic can only be new outside it. Which is why anything
  that makes an atlas *forget* has to clear it in the same breath, and why the
  rebuild path does.

  The other two walks are `ground::collect` and `statics::collect`, which build
  the quads and therefore have to see every visible cell. They are what the
  entry below is about.
- ~~**One new graphic at the edge of the view repacks every atlas.**~~ The
  atlases grow instead. Each one keeps its allocator — the land grid's next
  slot, the texture grid's cells, the shelf the sprites and the animation frames
  are packed on — plus the set of keys it has been *offered*, and `add` reads
  only what is genuinely new. What was written is recorded as a band of rows, so
  the upload is `write_texture` over that band into the texture already bound
  rather than three new `SpriteRenderer`s and 48MB: `Atlases::grow` then
  `Atlases::upload`, and a frame where the camera stood still touches no file
  and no GPU at all.

  Two things that had to come with it. The packers lose their global sort — a
  shelf is tallest-first *within one growth* now — which costs waste and not
  correctness, and a single `pack` still lays out exactly as it did, which is
  what keeps the frame tests exact. And growing needed an eviction, because
  rebuilding on every miss *was* one: see the entry on a failed repack above.
- **A growing shelf wastes more than a packed one, and nothing measures it.**
  `StaticAtlas` and `AnimAtlas` sort tallest-first, which is what makes a shelf
  worth using — and a growth can only sort *within itself*, so a frame that adds
  one 12-pixel sprite starts a row that no 200-pixel tree can share. The waste
  is bounded by the number of growths rather than by the art, and what it
  decides is how soon the eviction fires. Nothing reports how full an atlas is,
  so the first sign of this is a rebuild, which is invisible. A `used`/`capacity`
  line in the camera panel would cost nothing and is the honest place to start.
- **The dirty band is a bounding box, and one atlas can defeat it.**
  `TexmapAtlas` allocates first-fit over a cell grid, so a growth that lands in
  the first free cell near the top and another near the bottom widens the band
  to almost the whole texture — a 16MB upload for two textures. The land grid
  and the two shelves fill downwards and cannot do this. Worth a list of bands
  rather than one, if a profile ever shows the texmap upload at all.
- **`ground::visible_graphics` and `statics::visible_graphics` have no callers
  outside their own tests.** They are the whole-viewport form of
  `graphics_in`, which is what the app uses now. Either they are the public
  spelling and `graphics_in` is the private one, or they should go — a `pub fn`
  that only tests call is a decision nobody has taken.
- **`visible_tiles` widens by the whole `z` range on both axes and then takes an
  axis-aligned box.** `MAX_Z_LIFT` either way is 512 pixels of margin for a
  mountain that is rarely there, and the bounding box of a rotated rectangle is
  about twice its area — so most of the ~9,800 cells walked at 1080p are not on
  screen. Correct and generous; worth measuring against a `u`/`v` walk if the
  per-frame cost ever matters in release.

## Backlog, found while porting the client's cutaway and its culling

`crates/client/render/src/cutaway.rs` is `GameScene.UpdateMaxDrawZ`,
`Map.CalculateNearZ` and `CalculateObjectHeight`; the tie-break inside a tile is
`LessEqual` in `renderer::depth_state` plus the pass order. What was found on
the way and not done:

- **The client fades where this cuts.** Promoted out of this backlog and into
  the plan — see *What is still M3: a pass that blends*, which is where
  `ProcessAlpha`, `IsTranslucent`, foliage and `HasSurfaceOverhead` all land
  together, because they are one pass and not four features. What stays here is
  the pair that is neither a fade nor blocked on one: the season test
  (`IsFoliageVisibleAtSeason`) and the `TreeToStumps`/`HideVegetation` profile
  settings, both of which decide whether a graphic is drawn at all and neither
  of which has a profile to read yet.
- **The ground is not screen-culled and the statics now are.**
  `statics::on_screen` rejects a sprite whose rectangle misses the image, which
  is where most of the ±512-pixel `MAX_Z_LIFT` band goes. A land quad's screen
  extent is its four corner heights rather than a sprite's size, so the same
  test needs the stretched diamond's bounds — worth doing, and it is the same
  band being walked.
- **The atlas is still built from what the cutaway would hide.** `collect` drops
  a roof; `visible_graphics` still packs its art. Deliberate for now — the
  cutaway changes as the player walks and an atlas that shrank with it would
  repack every time somebody stepped through a door — but it means the atlas is
  sized for the widest case, which is worth remembering if packing ever fails.
- **`Cutaway::at` is recomputed every frame.** The client caches it against the
  player's `x`/`y`/`z` and recomputes on change. Two tiles and a flood fill is
  cheap, but `near_roof_z` allocates a 4,096-entry visited grid per call, which
  is a `Vec` per frame for nothing.
- **`Chunk.AddGameObject`'s `state == 1` arm is multis, and there are none.**
  When multis land, a multi at an equal `PriorityZ` sorts after the land and
  before everything else — which the current scheme (pass order plus
  `LessEqual`) cannot express, because it has no pass of its own between ground
  and statics. Either multis draw in the statics pass with an explicit sub-key
  in `depth::Order`, or they get their own pass.
- **`depth::mobile_priority_z` has no corpse or effect arm.** The client's
  `AddGameObject` gives a corpse `z + 1` like a mobile and a `GameEffect`
  `z + 2`. Both belong with whatever draws them.
- ~~**`Cutaway::at` was fed the unconfirmed prediction, not a trusted
  position.**~~ `App::draw` (`client/app/src/lib.rs`) read `self.player.at` —
  `link::Body`'s own optimistic guess, published the instant a step is sent
  and corrected only a round trip later — straight into `Cutaway::at` every
  frame. Deliberate for the body's own drawn position (`docs/camera.md`'s
  "follow the prediction"), but roof visibility flipping on an unconfirmed
  guess was never weighed as its own question; a held direction retried
  against a wall (`Steering::detour`, above) made it visible — a building's
  roof popped for one frame on every retry, for as long as the direction was
  held. Fixed with a second field, `App::cutaway_at`, that only advances to a
  tile the client's own static map agrees is reachable from the one it
  already held, and is snapped outright on a correction — same trust level
  as `player.at`, minus the one case (a step this end already knows is
  doomed) that was never worth predicting through for a roof.
