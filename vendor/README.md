# `vendor/`

Third-party source, kept here because a patched dependency has to live
somewhere, and because the group is part of the path in this workspace:
`crates/common`, `crates/server` and `crates/client` each mean something about
what may depend on what, and a crate that is nobody's belongs to none of them.
Nothing here is a workspace member — `[patch.crates-io]` in the root manifest is
what makes the build use it.

Everything here keeps its own licence files and its own copyright. This
repository's licence covers what this repository wrote.

## `egui-wgpu` — ported from wgpu 29 to wgpu 30

`egui-wgpu 0.35`, the latest release, depends on `wgpu ^29`. This client is on
`wgpu 30` and the two cannot be mixed: with both in the graph a `Device` from
one is not a `Device` for the other, and every call across the boundary fails to
compile. Downgrading is not free either — `Instance::new`,
`CurrentSurfaceTexture` and `queue.present` are all wgpu 30 shapes in
`crates/client/app`.

The port is four changes, all of them mechanical, each marked `wgpu 30:` in the
source:

- `RequestAdapterOptions` gained `apply_limit_buckets`, set to `false` — the
  adapter's own limits, which is what this crate reported before the field
  existed.
- `VertexState::buffers` is now `&[Option<VertexBufferLayout>]`, because a slot
  may be empty.
- `AdapterInfo` gained `limit_bucket`, which `adapter_info_summary` now prints.
  Named in the destructuring rather than skipped with `..`, so the next field
  upstream adds is a compile error here too — which is how that function stays a
  summary of *all* of them.
- `AdapterInfo::transient_saves_memory` became an `Option<bool>` and is
  formatted with `{:?}`.

**The exit condition:** when upstream releases a version of `egui-wgpu` on
wgpu 30, this directory and the `[patch.crates-io]` entry in the root
`Cargo.toml` are deleted in one commit. The port is meant to go upstream rather
than to be maintained here; if that stalls and this copy starts to rot, the
fallback is a paint pass of our own — egui's output is clipped triangle meshes
and texture deltas, which is `crates/client/render`'s `SpriteRenderer` with a
scissor rect and no depth attachment.
