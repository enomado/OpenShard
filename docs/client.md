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

**The window is joined to the wire.** `crates/client/app` logs in when it is
given an account and draws what the server has shown it — the character, and
everyone else on screen — with the arrow keys sending a `0x02` each and the
camera following the body the server confirms:

```sh
OPENSHARD_CLIENT=… OPENSHARD_ACCOUNT=admin OPENSHARD_PASSWORD=… \
    cargo run -p openshard-client-app
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

## M3a — the camera, and a shell to look through

**Built.** `OPENSHARD_CLIENT=… cargo run -p openshard-client-app` opens on
Britain, the wheel zooms about the cursor, a middle-drag pans, `Home` re-locks
the camera to the body, and the three panels are on screen. What follows is the
design as it was argued, with the places the code went another way marked — each
of them found by writing it.

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

`OPENSHARD_CLIENT=… cargo run -p openshard-client-app` opens on Britain, the
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
- **Nothing in `client/app` is tested.** `pan`, `zoom`, `fit_zoom_to_device` and
  the follow lock are ordinary arithmetic with ordinary edge cases — the drag's
  remainder in particular, which is the thing most likely to be quietly wrong —
  and none of it can be reached from a test today because `App` owns a `Map`, a
  window and a GPU. The camera half of it would move to `client/render` with no
  loss; the lock would want an `App` that can be built without a device.
- **`Camera::pick` exists and nothing picks.** Screen to *ground tile* is
  `unproject(pick(cursor), z)` and is one line away; screen to *what you
  clicked* is the depth ordering read backwards, which is M5. Worth not
  designing around the easy half.
- **Zoom-out makes the whole-atlas rebuild fire far more often.** The existing
  item — "the atlas is rebuilt whole whenever the camera walks off it" — is
  cheap because a screen holds a few dozen graphics. Four times the screen is
  four times the graphics against a 2048 atlas that is also the browser's
  guaranteed floor, so the eviction policy and the zoom-out limit are the same
  question and should be answered together.
- **Picking is half-built the moment `to_world` exists.** Screen to *ground tile*
  is the inverse projection at a known `z`; screen to *what you clicked* is the
  depth ordering read backwards, and a static or a mobile in front of the ground
  is what M5 will actually want. Out of scope here, and worth not designing
  around a ground-only answer.
- **A free camera can lose the character entirely**, with nothing on screen
  saying which way to look. An edge marker, or a "return" button in the camera
  panel next to the lock, is the cheap answer; the lock key already exists so
  this is polish rather than a hole.

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

## Backlog, found while joining the window to the wire

- **Everything stands.** Every mobile is drawn in group 4 whatever it is doing,
  because choosing walk over stand wants a mobile's *history* — where it was on
  the previous packet, and how long ago — which `WorldView` deliberately does
  not keep, being a record of what arrived. The place for it is a layer above
  the view that ages what it sees, and that layer is also what smooths a step
  into a glide instead of a teleport.
- **One animation clock for everybody.** `client/app` holds a single
  `AnimationClock`, so a crowd standing still breathes in unison — which is
  wrong and looks it. A clock per mobile needs a mobile that survives between
  frames, and today the whole list is rebuilt from the view on every update.
- **Ground items are decoded, held, and not drawn.** `WorldView::items` fills
  up from `0x1A` and the renderer never sees it: a static sprite is placed from
  the map's `staidx`, and an item from the server has no cell to be read out of.
  It is the same `SpriteRenderer` with a different source of quads —
  `statics::collect` is the shape to follow.
- **The facet is a startup constant and `0x1B` only carries a size.** The app
  loads Felucca and compares the shard's map size once, warning when they
  differ rather than following. Following means decoding `0xBF 0x08` and
  reloading the facet, and the reload is the interesting half: `Map::load_facet`
  reads a few hundred megabytes.
- **A whole `WorldView` is cloned per changed packet.** Fine for the handful a
  standing character receives, and not fine beside a crowded bank: the thread
  clones the map of every mobile to say that one of them turned. The answer is
  probably not a delta protocol between the two threads but a shared snapshot
  the window reads — worth measuring before deciding.
- **`z` still drifts on a hill, and now it is visible.** The client's `Walk`
  has no map (see below), so a step predicts the height it started at, and the
  body is drawn at that height until a `0x20` or a `0x21` corrects it. Standing
  a body below the terrain is exactly what a mobile that failed to draw looks
  like. `uofiles` is now on the client's side of the wall, so the fix — handing
  `Walk` the map, or asking the map at draw time — is finally available.

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
