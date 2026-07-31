# crates/client

Our own client, in three parts:

- [`net`](net) — the client's side of the wire: framing, decompression, the
  login conversation, the walk handshake, and a `WorldView` of what the server
  has shown us.
- [`render`](render) — `wgpu`, and no engine. Ground, statics and mobiles, with
  one depth ordering shared between the passes.
- [`app`](app) — the binary: a window, a surface, and the thread the socket runs
  on. The only crate here allowed to touch the platform.

[`docs/client.md`](../../docs/client.md) is the plan they are built against, and
its backlog is where the next session in this area starts.

The stock 2D client and ClassicUO remain first-class: the server is written to
the protocol, not to this.

Everything here talks through `crates/common` — `protocol` for the wire,
`uofiles` for the client's own data files — and never depends on
`crates/server`.
