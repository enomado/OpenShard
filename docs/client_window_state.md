# The client's window state: one owner, not two

Living plan for a client-side refactor. It starts from a bug fixed on
2026-08-11 — a paperdoll, container or dialog closed on screen and reopened
itself a second or so later — and the plan is what closing that bug the
honest way would take, versus the patch that actually shipped.

As with [`protocol_newtypes.md`](protocol_newtypes.md): when reality
contradicts a decision here, change this file in the same commit that changes
the code.

## Why

A window close is deliberately client-only. No packet carries it — the
reference client does not send one either, `App::close_window`'s own doc
says so, and it is correct as protocol behaviour. But "client-only" turned
out to mean something narrower than intended: only *one of two* mutable
copies of the fact "what does this player currently have open" heard about
it.

| | `App::view` (`crates/client/app/src/lib.rs`) | `view` (`crates/client/app/src/link.rs`) |
|---|---|---|
| owner | the window / event-loop thread | the shard thread, one per connection |
| written by | server packets (via `entered`), *and* local closes (`close_window`, `answer_gump`) | server packets only (`fold`), until 2026-08-11 |
| read by | `sync_own_windows`, every frame's draw | `snapshot()`, on every packet that changes anything |

Both are `openshard_client_net::view::WorldView`. The link thread's copy is
the one every `Update::World` is cloned *from*, whole, not as a diff —
`App::entered` then overwrites its own copy with that clone outright
(`self.view = Some(Box::new(view.clone()))`). So a fact the link thread's
copy does not have is a fact the next snapshot erases from `App`'s copy too,
however recently `App` learned it locally. A closed paperdoll is exactly
that fact: `App` marked it closed, the link thread's copy still called it
open, and the next packet that changed *anything nearby* — an NPC's own
step was enough, no server message about the paperdoll at all — cloned the
still-open copy over the closed one. The window reopened itself with no
packet logged for it, which is what made the bug hard to place: nothing
server-side was wrong, and the two client-side copies never once got a
chance to be compared, only overwritten in one direction.

**What shipped instead of the honest fix.** `link::Command::CloseWindow` /
`link::CloseTarget`: a command that, like every other `Command` variant,
crosses the channel but not the wire, and the link thread applies it to its
own `view` in the `commands.recv()` arm — the same `paperdoll_closed` /
`container_closed` / `gump_closed` methods `App` already called on its own
copy. `App::close_window` and `App::answer_gump` now call both. It is a
correct patch — closing a container, a paperdoll or a dialog reaches both
copies now, and the reopen is gone — and it is a patch: the fix is "remember
to write to both," which is exactly the invariant nothing enforces. The next
local-only fact this client learns about a window — and there is at least
one already, see the backlog below — gets to fail the same way once before
somebody remembers this file.

## The shape this works toward

**There should be one mutable `WorldView`, not two kept in step by
convention.** The candidates:

**Option A — the link thread is the only writer; `App` reads a snapshot and
predicts nothing.** `App` stops calling `paperdoll_closed`/etc. on its own
copy at all; a close is *only* a `Command`, and `App`'s picture of "what is
open" updates on the next `Update::World`, same as every other fact it
learns. Simplest to state, and it introduces a visible lag: the window a
player just closed stays drawn for up to one round trip through the
channel — which, same-process, is a handful of commands' width, not a
network RTT, but it is a frame or more of a window the player just told to
go away still being there to click through.

**Option B — `App`'s copy is a prediction, reconciled the way `Walk` and
`Body` already are.** This client already has the shape for "we know
something locally before the authoritative side confirms it" —
`link::Body::predicted` / `corrected`, `client/net`'s `walk` module — built
for exactly the same problem one layer down: the body's position, not a
window's openness. A close would set a local, provisional "not open"
overlay (`App::locally_closed: HashSet<WindowSubject>`, say) *and* send the
`Command`; `sync_own_windows` would check the overlay beside `view.paperdolls`
when deciding what is wanted; the overlay entry would clear the moment a
fresh snapshot agrees the window is gone, the same reconciliation `Body`
does on every corrected step. No visible lag, and the duplication moves from
"two functions that must independently stay in sync" to "one overlay type
with a documented reconciliation rule," which is the same trade the walk
handshake already made once.

Neither is chosen yet — see Decisions. Both retire the CloseWindow patch's
actual defect, which is not that it is wrong, but that it is *invisible*:
nothing stops a third window kind from being added with only one of the two
copies told about its close, the way this one nearly was.

## Decisions

**D1. Not decided: Option A vs. Option B.** Option A is less code and costs a
frame of staleness nobody may ever notice in a same-process channel; Option B
costs a new piece of state but matches the project's own established pattern
for this exact class of problem (`docs/style.md`'s "no fudge constants"
section argues the general case: two representations of one thing are made
one, or the disagreement between them is carried as data and measured — an
overlay reconciled against the next snapshot is the second shape, not a
third one invented for this file). Whoever picks this up should read
`link::walk`'s doc for how `Body::corrected` decides a prediction was wrong,
before deciding whether a window's openness deserves the same machinery a
tile does.

**D2. `App`'s copy is not the source of truth for anything, in either
option.** Whichever shape wins, drawing code reads `App::view` because it is
where the last-known picture lives, never because `App` is where a fact
about the world is *decided*. The link thread's `view` — or, if D1 lands on
Option A, the snapshot it last sent — is what a disagreement is checked
against.

## Steps

Nothing beyond the immediate patch is built. Each of these is a session on
its own.

- [x] **S0. Stop the reopen.** Done, 2026-08-11. `link::Command::CloseWindow`
      / `link::CloseTarget`, applied from `App::close_window` and
      `App::answer_gump`. This is the patch the rest of this plan proposes to
      retire, not extend — a fourth window kind should not get a third
      call site added to it while this plan is open.
- [ ] **S1. Pick A or B.** Resolves D1. Whichever is picked, write the
      choice into D1 above with the reasoning, the way `connection_state.md`
      numbers and argues its own decisions rather than presenting the
      result with the argument thrown away.
- [ ] **S2. Build it, and delete the patch.** `CloseWindow`/`CloseTarget`
      either become the whole mechanism (Option A: `App` stops writing
      `self.view` locally at all, the command is the only writer anywhere)
      or become the second half of a pair with the new overlay (Option B).
      Either way, `App::close_window` and `App::answer_gump` should end up
      calling *one* thing per close, not two.
- [ ] **S3. A test that would have caught the original bug.** Something in
      `client/app` or `client/net` that opens a window, closes it, folds in
      an unrelated world change (a mobile stepping, say — the exact trigger
      the bug report used), and asserts the window is still closed after.
      None of today's `view.rs` tests exercise two `WorldView` instances at
      once, which is exactly the shape that hid this.

## Backlog

- **Skills is the one window kind `WorldView` cannot answer for at all.**
  `WindowSubject::Skills` closes by clearing `App::skills`/`held_skill`
  directly — there is no `view.skills_closed` to call because the tree is
  never server state to begin with (`close_window`'s own comment: "the
  skills stay where they are, the way a paperdoll's equipment does"). Not a
  bug of this shape — there is only one copy of that fact, so it cannot
  disagree with itself — but worth naming here so whoever does S1/S2 does
  not go looking for a `WorldView` method that was never supposed to exist.
- **`sync_own_windows` re-derives `wanted` from `view.paperdolls`/`containers`
  every frame** (`crates/client/app/src/lib.rs`). Cheap today — the maps are
  small — but Option B's overlay check has to live in exactly this function,
  and whoever adds it should confirm the frame cost is still nothing before
  assuming it is.

## Status

**S0 has landed; S1 through S3 are open.** This plan exists because the
patch that fixed the visible bug was flagged, correctly, as a shape rather
than a cause — see [`roadmap.md`](roadmap.md), under "The client — planned,"
which points back here for what to do about it.
