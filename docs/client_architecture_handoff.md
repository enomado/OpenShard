# Client architecture handoff

This is a long-running refactor track for `crates/client`.

## Current checkpoint

The working tree was clean after:

```text
5d1cf8c trim frame copies and isolate update reducer
```

Relevant earlier checkpoints:

```text
ffa66ce keep local window mutations on app thread
1e48536 consolidate world mutations in client reducer
4dd8bad refactor client world update ownership
```

Validation currently passes:

```text
cargo check -p openshard-client-app
cargo test -p openshard-client-app --lib
```

Result: 163 passed, 2 ignored.

## Architecture agreed with the owner

- `WorldView` has one owner: the client event-loop/App model.
- No `Arc<WorldView>` and no shared mutable world ownership.
- The network thread decodes protocol and maintains wire/walk state only.
- Network updates are applied in the App reducer.
- Frame processing is staged:

  ```text
  receive event
    -> mutate authoritative model
    -> rebuild presentation projection
    -> read-only frame snapshot
    -> render
  ```

- Local UI mutations remain on the App thread. `CloseWindow` is not a network
  command anymore.
- `event_loop.rs` should remain a platform dispatcher, not a gameplay or world
  reducer.

## What has been changed

- `link.rs` no longer imports `winit`.
- `Update` distinguishes initial world state, server mutations and local
  movement prediction.
- `App::on_update` owns cross-thread update orchestration.
- `WorldView` is moved into the reducer, mutated, projected and moved back;
  there is no per-update `WorldView` clone.
- Mobile picking uses `mobiles::pick_iter`, avoiding a second owned
  `Vec<Mobile>` solely to adapt `(Who, Mobile)` pairs to the picker.

## Remaining known copy cost

The main remaining frame-level copy is:

```text
drawn_mobiles()
  -> clone Mobile values
  -> clone Vec<EquipmentLayer> for every mobile
```

This occurs because the renderer currently receives owned `Mobile` values and
the per-frame code updates animation fields (`group`, `frame`, `drawn`,
`from`). The correct fix is architectural, not another local clone removal:

1. Prefer an immutable/shared equipment representation for render mobiles,
   probably `Rc<[EquipmentLayer]>` because this client is single-threaded; or
2. Introduce a borrowed `FrameMobile`/render snapshot that borrows immutable
   body/equipment data and owns only the time-varying fields.

Do not reintroduce `Arc<WorldView>` to solve this.

Other clones found are either small/local or semantically required by protocol
state updates (paperdoll, skills, container contents, login plan). Atlas
rebuild copies are on a rare eviction path.

## Next work items

1. Solve the mobile frame snapshot/equipment ownership issue and measure that
   the per-frame equipment allocation is gone.
2. Add tests around frame snapshot reuse or borrowed mobile rendering.
3. Replace the unbounded command/update delivery with explicit staged delivery
   semantics. Preserve ordering for mutations and avoid accumulating stale
   frame updates.
4. Continue separating:

   ```text
   authoritative world state
   prediction state
   presentation projection
   ```

5. Keep commits small and run the client app check/tests after each stage.

## Important caution

The repository may contain unrelated user changes when work resumes. Inspect
`git status` first and preserve them. Avoid broad mechanical rewrites of the
large render files until the mobile ownership design is settled.
