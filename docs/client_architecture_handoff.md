# Client architecture handoff

This is a long-running refactor track for `crates/client`.

## Current checkpoint

The working tree was clean after:

```text
3e33a6c separate client presentation projection
```

Relevant earlier checkpoints:

```text
e6e0c82 separate client prediction state
cf06c8b isolate authoritative client world
505e4ed stage client frame delivery
5d1cf8c trim frame copies and isolate update reducer
ffa66ce keep local window mutations on app thread
1e48536 consolidate world mutations in client reducer
4dd8bad refactor client world update ownership
```

Validation currently passes:

```text
cargo check -p openshard-client-app
cargo test -p openshard-client-app --lib
cargo test -p openshard-client-render --lib
```

Result: 169 app tests passed, 2 ignored; 482 render tests passed, 1 ignored.

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
- Render-mobile equipment is `Rc<[EquipmentLayer]>`, so a frame snapshot
  retains immutable equipment rather than allocating and copying its layers.
- Cross-thread delivery is staged: ordered updates are bounded (with socket
  backpressure), consecutive predictions coalesce, and winit receives one
  wake-up per pending batch.
- `WorldState` names its three state kinds directly: `authoritative`,
  `prediction` and `presentation`.

## Remaining small copy cost

The frame-level equipment copy has been removed: a `Mobile` clone now only
increments the single-threaded `Rc` handle. `drawn_layers` still builds a small
ordered list of `EquipmentLayer` values for each renderer pass; that list is
not a clone of the mobile's equipment allocation and is bounded by the worn
slots.

Other clones are either small/local or semantically required by protocol state
updates (paperdoll, skills, container contents, login plan). Atlas rebuild
copies are on a rare eviction path. Do not reintroduce `Arc<WorldView>`.

## Next work items

1. Decide whether the short-lived `drawn_layers` lists merit a borrowed ordered
   iterator; measure first, because they are not a per-mobile equipment clone.
2. Exercise the staged mailbox against a real stalled-window/network workload
   and tune its ordered-update capacity if needed.
3. Keep the three state boundaries explicit as new fields are added:

   ```text
   authoritative world state
   prediction state
   presentation projection
   ```

4. Keep commits small and run the client app check/tests after each stage.

## Important caution

The repository may contain unrelated user changes when work resumes. Inspect
`git status` first and preserve them. Avoid broad mechanical rewrites of the
large render files until the mobile ownership design is settled.
