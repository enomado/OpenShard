# Invariants nothing enforces

Living plan for the backlog that three finished sweeps left behind. Unlike
[`protocol_newtypes.md`](protocol_newtypes.md) and
[`connection_state.md`](connection_state.md), which each took one subject
through one crate, this one is a shape found in four places at once, so the
stages share a reason rather than a module.

As with both of those: when reality contradicts a decision here, change this
file in the same commit that changes the code.

## Why

Every item below is a rule the code obeys and cannot state. The rule is real,
it is written down, and the only thing keeping it true is that whoever last
touched the area had read the paragraph.

That is a specific failure mode and not a general complaint about
documentation. A rule in prose has three properties a rule in a type does not:
it is invisible to the person who never opens the file, it stays green when the
code stops obeying it, and it cannot be found by the search that would prove it
was broken. The newtype sweep spent thirteen stages on the first of those —
`Serial` instead of `u32` — and the sweep's own gate then found the second: a
coverage check that examined nothing had been green here before, and a check
that cannot see a function's parameters was green over four `cliloc: u32`
signatures for as long as they existed.

So the question this file asks about each item is not "is it correct today"
(all of them are) but **what would notice if it stopped being**.

| | the rule | what holds it today | what would notice |
|---|---|---|---|
| S1 | `restore_characters` runs before `restore_items` | two doc comments and the order of two lines in `run_shard` | nothing |
| S2 | every recipe names a group that exists and leads with its system's skill | an assertion in a test | `cargo test`, after a bad row has already been committed |
| S3 | closing a refused connection tears down six things in order | six doc comments, one per link | nothing |
| S4 | the ground renderer's projection and visible set are correct | `tests/frame.rs`, behind `OPENSHARD_CLIENT` **and** a GPU | nothing, on any machine without both |

## Decisions

Nailed down here so a session does not stop to ask.

**D1. A build-time check beats a test, and a type beats both.** In that order,
and the order is about *when the wrongness is visible*: a type is wrong while
the author is still typing, a build script is wrong before the artefact exists,
a test is wrong after a commit that may already have been pushed. Where an item
below can be moved a whole step, it is moved a whole step. Where it cannot, the
stage says why rather than settling quietly.

**D2. `Option`, never a zero.** Already settled in
[`style.md`](style.md) and applied twice in the cliloc work (`CraftGumpContext::
notice`); S2 finishes it. A cliloc `0` is a number the client would look up, so
"absent" and "message zero" are the same bits and only a comparison somewhere
tells them apart.

**D3. An e2e test is allowed to be slow and is not allowed to be flaky.**
`crates/e2e` exists precisely so that a test may stand up a real gateway and a
real shard in one process. S3 is the second thing to live there. A retry loop
with a deadline is fine; a `sleep` that happens to be long enough is not — that
is a test that passes on this machine.

**D4. The renderer's fixture is a map, not a mock.** S4 does not introduce a
`trait Map`. The renderer reads a concrete `Map` and should keep reading one;
what is missing is a way to *construct* one from cells in memory. A trait would
make the tests pass against something that is not what ships.

**D5. Nothing here touches a file another session is in.** This is a working
constraint, not a design one, and it is written down because it has already
cost a day: the tree is shared, and two of these stages sit in the hottest files
in the repository. See "Order" below.

## Steps

Each is a pull request. S1 and S2 are worth doing whether or not the rest
follows.

- [ ] **S1. The restore order stops being a comment.**
      `restore_characters` reserves the serials that `restore_items`' records
      point at as owners. Run them the other way round and a character's pack is
      filed under a serial the allocator is free to hand to something else — and
      nothing fails, then or later: the pack is simply somewhere else, and the
      first person to notice is a player. Both `restore_*` docs state the order
      and `run_shard` obeys it.

      The fix is to make the second unable to run first. The cheapest honest
      shape is a token: `restore_characters` returns a `RestoredCharacters`
      (whatever it already computes, plus the fact that it ran), and
      `restore_items` takes one. It cannot be constructed elsewhere, so the
      order is the signature. Prefer this to an assertion inside `restore_items`
      — an assertion is a fourth thing to keep true, and it fires at boot on a
      live shard rather than at compile time on a developer's machine.

      **DoD:** the two functions cannot be called out of order without a type
      error; the token carries something real rather than being a marker
      (whatever `restore_characters` already hands over, or the reserved-serial
      set itself); `run_shard` reads the same or shorter; the doc comments say
      what the type now says, or are deleted for repeating it; all four gates
      silent.

- [ ] **S2. The craft tables fail the build, not the test run.**
      `defs/mod.rs` asserts that every recipe names a group that exists and
      leads with its system's own skill. Both are properties of the *data*, and
      `crafting/build.rs` is where a bad row should stop being a build. What
      blocks it is that the group count and the main skill live in `SYSTEMS`,
      which is hand-written Rust the build script does not read.

      **The decision: the five headers join the data.** `SYSTEMS` becomes
      `data/craft_systems.json` beside the eight tables that already moved,
      generated by the same `build.rs`, which then has both halves in hand and
      can check them. The alternative — teaching the script to parse `SYSTEMS`
      out of Rust source — is a second parser for a language we already have a
      compiler for, and it is the shape `architecture.md` moved 18,155 lines of
      tables away from in the first place.

      Two things ride along, because they are in the same rows and the same
      commit is the cheapest place for them:

      - `CraftSystemDef::needs_message` is `ClilocId(0)` on four systems of five
        (`defs/mod.rs`) — D2's zero. `Option<ClilocId>`.
      - `Recipe::amount` has a column and no data: all 485 rows are 1. Leave the
        column and say so in the doc, or drop it — but decide, and record which,
        rather than carrying a field nothing sets. The stacking recipes that
        would use it (`DefBowFletching`'s arrows and bolts) are not ported.

      **Note before starting:** the roadmap entry for this also claims
      `Text::Cliloc(0)` appears in the generated tables as a null. It does not —
      of 11,448 generated clilocs, zero are `0`. The entry is stale; delete that
      half of it. This is the second stale backlog claim in two sessions, which
      is itself worth remembering: **check a backlog entry against the code
      before planning work around it.**

      **DoD:** `SYSTEMS` is `data/craft_systems.json`; both invariants are
      checked in `build.rs` and fail the build with a message naming the row;
      the two assertions in `defs/mod.rs` are deleted, not kept as belt and
      braces (a check in two places drifts); `needs_message` is an `Option`;
      `Recipe::amount` is decided either way with the reason in its doc; the
      stale roadmap half is gone; all four gates silent.

- [ ] **S3. The teardown chain gets a test that walks it.**
      A refused connection is closed by a chain of six: `Sessions::close` drops
      the session, which drops the outbox, which ends the gateway's write task,
      which closes the socket, which makes the gateway emit `Disconnected`,
      which queues `Command::Disconnect` so the world lets go of what it held.
      Every link is real, each is documented where it lives, and no file states
      the chain. Breaking any link leaves a connection the world still thinks is
      present — the leak the whole of `connection_state.md` was written against.

      This one cannot become a type: the links are a socket, a task and a
      channel, and what joins them is `Drop`. So it becomes the second thing in
      `crates/e2e`: a real gateway, a real shard, a connection refused entry,
      and an assertion that the world has forgotten it — with the *whole* chain
      exercised rather than any single link mocked.

      Write the chain down in one place while doing it. The natural home is a
      module doc on the e2e test itself: the test is the only artefact that
      knows all six links, so the prose belongs beside it rather than in a
      seventh file that can go stale on its own.

      **DoD:** a test in `crates/e2e` that opens a real connection, is refused,
      and observes the world drop it; the test asserts the *state before* as
      well as after, because a leak test on something that was never there is
      green for the wrong reason (`connection_state.md` S7 learned this); no
      fixed `sleep` — a deadline and a poll (D3); the chain named link by link
      in one doc comment; all four gates silent.

- [ ] **S4. A `Map` can be built without a client install.**
      Every assertion about `ground::collect` lives in `client/render/tests/
      frame.rs`, behind `OPENSHARD_CLIENT` and a GPU, because the only way to
      obtain a `Map` is to load one from a file. So the projection and the
      visible-set logic — which are arithmetic, and wrong in ways a picture does
      not show — are untested on every machine that has neither, which includes
      CI.

      Both atlases already take pictures directly (`LandAtlas::pack`,
      `TexmapAtlas::pack`), so the *art* half of this is done: that a slope is
      drawn from its texture and a level tile from its art is already green with
      no install. What is left is the map. Per D4: a constructor taking cells,
      or a small fixture facet built in memory — not a trait.

      **DoD:** `ground::collect`'s visible set and the projection are asserted
      in a test that runs with neither `OPENSHARD_CLIENT` nor a GPU; the
      existing frame tests keep working against a real install; the constructor
      is honest about what a hand-built `Map` does not have (`Map::load`'s facet
      ambiguity, noted in `client.md`, is not made worse); all four gates
      silent.

## Order

S1 and S2 first, and in either order — they are small, they are in `server` and
`crafting`, and neither has been touched by a parallel session in a week.

S3 next. It is the largest and it is new code in a crate with one occupant, so
it collides with nothing.

**S4 last, and only when the tree is quiet.** `client/render` and
`common/uofiles` are where the parallel work has been living — `atlas.rs`,
`ground.rs`, `texmaps.rs`, `frame.rs` all moved under this session's feet. D5
is about this: check `git status` for foreign edits in `crates/client/*` before
starting, and if there are any, take S1 or S2 instead. There is no deadline
here and there is no reason to reproduce a mixed working tree on purpose.

## Deliberately not in this plan

- **`world/src/tick/tests.rs` is 12,964 lines** — the largest file in the
  repository against a stated ~2k rule, and `architecture.md` has the split
  mechanics written for exactly it. It is left out because it is a *mechanical*
  change to the single hottest file in the tree: every parallel session touches
  it, and a 13,000-line move conflicts with all of them. It is worth doing in a
  session that owns the tree outright, and it is worth doing then rather than
  never — `state/src/runtime.rs` (2,169) and `state/src/components.rs` (2,108)
  are over the line too and are the easier warm-up.
- **The three body-keyed tables** (`body_types.json`, `creature_names.json`,
  `creature_sounds.json`) drifting apart. Real, and it needs a format decision
  first — "these four bodies share a sound but not a name" has to be expressible
  before the merge is an improvement rather than a second problem.
- **`Uop::open` reads 155MB whole.** Harmless for a shard, wrong for a browser,
  and the browser is the deadline. It belongs to whoever takes the renderer to
  the web, not here.

## Status

| Stage | State |
| --- | --- |
| S1 — restore order in the types | not started |
| S2 — craft tables fail the build | not started |
| S3 — teardown chain, end to end | not started |
| S4 — a `Map` without an install | not started |
