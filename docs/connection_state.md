# The connection state machine: one owner per phase

Living plan for a multi-session refactor of what a *connection* is. It is the
sequel to the design already written down in
[`architecture.md` → "Sessions and the character registry"](architecture.md), and
it does the one thing that design left unresolved: it says **where the
connection's state lives**, rather than which of its questions each existing
table answers.

As with [`protocol_newtypes.md`](protocol_newtypes.md): when reality contradicts
a decision here, change this file in the same commit that changes the code.

## Why

A connection's state is kept in two tables that have to agree, and nothing
checks that they do.

| | `server/src/session.rs` | `openshard-state` |
|---|---|---|
| which socket, which client version | `Session::login` | `Client { version }` on the entity |
| is it playing, and what | `Session::playing` | `WorldState::players` |
| what it is dragging, watching, has open | — | `held`, `open_containers`, `open_quest_gumps`, `open_craft_gumps`, `pending_targets`, `last_status`, `last_light`, `last_music` |

Five things follow from the split, and each is a reason on its own:

1. **Presence is a bool, and it is set optimistically.** `Session::playing` is
   set as `Command::Enter` is *queued* (`server/src/dispatch.rs`), and
   `World::enter` refuses silently in three places — already in the world, a
   saved serial that will not bind, an exhausted mobile serial pool
   (`world/src/tick/enter.rs`). After any of them the session says it is playing,
   the world has no entity, and every world packet the client sends is queued
   into a tick that drops it on a `players.get` miss. The client is told nothing
   and waits on "logging into shard". `Option<PlayedCharacter>` cannot spell
   *asked to enter, not yet in* — the state that genuinely exists between the
   queue and the tick — so the lie is not a slip, it is the only thing the type
   can say.
2. **`in_world()` is a second copy of `players.contains_key`.** Thirty arms of
   `dispatch_world_packet` open by consulting the copy rather than the world.
   *(S3: it is asked once now. It stays a projection — D4 says why it must be —
   but there is one place that reads it, and one place a new packet can forget
   to.)*
3. **The world cannot answer a connection that has no entity.**
   `WorldState::send_packet` resolves the client version through
   `players → Client`, so a connection on the character screen is unreachable
   from inside a tick, and unreachable *silently*. This is the structural reason
   the character screen cannot become world commands as
   [`roadmap.md` §2](roadmap.md) plans: the version has to live on the
   connection, not on the entity.
4. **The login crate already made this argument about itself.**
   `LoginSession`'s own doc: *"a state machine with facts kept outside it is a
   state machine that can disagree with itself"* — which is what `playing` is.
5. **Teardown is hand-written.** `World::disconnect` clears eight maps by name.
   The ninth one added without a line there leaks, and nothing catches it.

## The shape this works toward

The seam is **authentication**. Everything before it is the login conversation;
everything after it is the world's, character screen included.

| | owner today | owner after |
|---|---|---|
| credentials, auth keys, `0x80`/`0xA0`/`0x8C` | `openshard-login` | unchanged |
| a character *exists* | `login.accounts` | the world, with the roster |
| a character is *present* | `Sessions::playing` (binary) | the world, as a phase |
| client version | `Client` on the entity | the connection record |
| the socket, and whether it is compressed | `Session` (binary) | unchanged — it is transport |

The login crate ends its life at `Authenticated { account, version, access }` and
hands the connection over with one command. From there the connection is a row in
the world with a phase:

```text
   Authenticated ──> Entering ──> Playing { entity } ──> LoggingOut
        │                │
        │                └─ Command::Enter queued, the tick has not applied it
        └─ character screen: list, create, select, delete
```

## Decisions

Numbered so a later session can argue with one without reopening all of them.

**D1. Login does not move into the world.** Accounts, argon2, auth keys, the
`0x8C` relay and the shard list are not simulation. `Argon2::default()` is 19 MiB
and two passes against a 50 ms `TICK_INTERVAL`; a password check inside a tick
stalls the whole shard for one client's benefit. What moves is everything *after*
the `0x91`.

**D2. The world's record of a connection is `openshard_state::connection::Connection`.**
What the world knows about a client that is not its character: its version
today, the per-connection state of S7 later. It is deliberately *not* called a
session — the session is the binary's, see D4 — and the two must not share a
name, or a reader has to work out each time which of them is authoritative.

**D3. The phase is an enum, and `Entering` is a state of its own.** It is the
distinction `Option<PlayedCharacter>` could not carry, and the one that makes
point 1 above unspellable rather than merely fixed.

**D4. The phase lives in the binary and is moved only by the world.**

The first half is forced: the packet router has to decide *now* whether a packet
may reach the world, and the world answers no synchronous question — only
`queue(Command)` in, `drain_*` and the bus out. That rule is why `World::is_online`
was deleted (see `architecture.md`), and a phase held in `WorldState` would put it
straight back by making the router read the world on every packet.

The second half is what keeps it honest: `Session::enter_world` may only move
`Outside → Entering`, and nothing but a world event moves it further.
`PlayerEntered`, `PlayerLeft` and the new `PlayerRefused` carry the connection,
and the shard loop drains them beside `drain_departed`. The binary's phase is a
*projection* of the world's fact, not a second copy of it: the one direction it
can be wrong in is being one tick behind, and `Entering` is the name of exactly
that gap.

Commands queued while `Entering` are still queued, not dropped — the queue is
ordered, so they apply after the `Enter` that precedes them. This is why the
phase must exist rather than the gate simply being moved later.

*Changed after S1, which had this backwards.* The first draft put the phase in
`WorldState` and had the binary read it. It cannot: see above.

**D5. The character screen becomes world commands only after the roster moves
in.** Ordering, not preference: until the world owns the saved records it cannot
answer `0x5D`, and until the connection record exists it cannot answer anybody
who has no entity. See [`roadmap.md` §2, "The roster belongs in the
world"](roadmap.md).

**D6. This does not reopen the decision in `architecture.md`.** That decision —
create, select and delete stay out of `openshard-login` — was about not dragging
`openshard-persistence` (and with it bundled SQLite and `tokio-postgres`) into a
crate whose whole value is that it has neither. Moving them *into the world*,
which already depends on persistence, is the other direction and the objection
does not apply. What it does retire is "all three live in the binary": they live
in the binary today because the world could not be asked, and D2 is what changes
that.

**D7. No new crate.** The state goes into `openshard-state` beside the rest of
the runtime; the rules go into `openshard-world`. A `crates/server/session` crate
was considered and rejected: it would sit between login and world with a
dependency on both, which is the same seam this file is trying to delete, only
with a `Cargo.toml` around it.

## Steps

Each is a pull request. The first three are worth doing whether or not the rest
follows.

- [x] **S1. The world can address a connection that has no entity.** Done.
      `WorldState::sessions: HashMap<ConnectionId, Session>` carries the client
      version and `version_of` reads it; the binary sends
      `Command::Authenticated { connection, version }` at the one-way transition
      into `LoginSession`'s game state, `World::attach` writes the row and
      `World::disconnect` removes it. `Client` is down to its connection —
      `WorldState::client_of` returns the (connection, version) pair every packet
      path wanted, and `tick/context.rs` lost a second private copy of
      `version_of` that had been written beside it.
      Guarded by `a_connection_can_be_answered_before_it_has_a_character` and
      `a_disconnect_forgets_the_connection_itself` in `world/src/tick/tests.rs`.

- [x] **S2. The phase replaces the bool.** Done. `WorldPhase` — `Outside →
      Entering → Playing → Left` — on the binary's `Session`, in place of
      `playing: Option<PlayedCharacter>`. `PlayerEntered`/`PlayerLeft` carry the
      connection, the new `PlayerRefused` carries why an entry did not happen,
      and `PhaseSync` (cursors, drained at the top of `world_tick`) is the only
      thing that moves a phase past `Entering` — see D4. A refusal comes back
      from `PhaseSync::apply` as a connection to close, because the protocol has
      no way to tell a client its `0x5D` failed: the socket dropping is what
      turns an indefinite hang into a reconnect.
      Guarded by `the_phase_moves_on_only_when_the_world_says_it_did`,
      `an_entry_the_world_refuses_takes_the_connection_down` and
      `a_character_the_world_let_go_of_is_no_longer_being_played` in
      `server/src/session.rs`.

- [x] **S3. One gate instead of thirty.** Done. `dispatch_world_packet` is
      `fn(ClientPacket, ConnectionId) -> Option<Command>`: it takes neither the
      session nor the world, so an arm cannot reach past the packet it was
      handed, and `None` is a packet the world has nothing to do about. The
      phase is matched once, in `handle_world_packet`, which queues what comes
      back — the rule `roadmap.md` §2 asks for on the character screen, applied
      to the whole dispatcher.
      `0x5D` moved out to `dispatch::play_character` and is routed *before* the
      gate: it is the one packet a connection outside the world may send, and
      the only one that needed the roster and the account's access. That is what
      leaves the gate a single `if` and the dispatcher total.
      Guarded by `a_world_packet_from_outside_the_world_becomes_no_command`,
      `the_same_packet_becomes_a_command_once_the_connection_is_in` and
      `the_character_screen_packet_is_the_one_the_gate_does_not_stop` in
      `server/src/shard.rs`. They assert on `World::queued`, added for them: a
      refused packet's whole story is that no work was created, and every other
      observation of the world is downstream of a tick that would have had
      nothing to apply either way.

- [ ] **S4. The roster moves into the world.** Already planned — see
      [`roadmap.md` §2](roadmap.md). Collapses `Roster`, `departed` and
      `pending_inventories`; "exists" and "is played" become two states of one
      record; `is_playing` is answered by `(account, name)` rather than by a
      serial nobody has yet.

- [ ] **S5. The character screen is world commands.** `0xA9`, `0x00`/`0xF8`,
      `0x83`, `0x5D` become commands answered out of a tick.
      `create_character`/`delete_character` leave the binary. The login crate
      ends at `Authenticated`.

- [ ] **S6. The binary is glue again.** The `select!` loop, the transport, and
      boot. `restore_*` into `boot.rs`, `keys.expire` into its own `select!` arm,
      argon2 onto a blocking task.

- [ ] **S7. The rest of the per-connection state joins the row.** `held`,
      `open_containers`, `open_quest_gumps`, `open_craft_gumps`,
      `pending_targets`, `last_status`, `last_light`, `last_music`. Teardown
      becomes one `remove`, and forgetting a field stops being possible.

## Backlog, found while doing S1, S2 and S3

None is a blocker; each is written down where the next step through this area
will read it.

- **A helper written beside the one that already exists.** S1 found two:
  `tick/context.rs` had a private `client_version(connection)` that was
  `WorldState::version_of` reimplemented next to it, and `items/containers.rs`
  walked `players → Client` inline for the same answer. Both predate the version
  moving, and both would have had to be found and changed anyway — the point is
  that neither was findable by grepping for `version_of`. A duplicate helper is
  invisible to the search that would prove it is a duplicate.
- **`world_tick` now takes seven parameters** and grows one per step: S2 added
  two. S6 should gather the loop's state into one value; otherwise the next
  drain is an eighth argument and the signature stops being readable before it
  stops compiling.
- **Closing a refused connection relies on a chain nothing states.**
  `Sessions::close` drops the session, which drops the outbox, which ends the
  gateway's write task, which closes the socket, which makes the gateway emit
  `Disconnected`, which queues `Command::Disconnect` so the world lets go of
  whatever it had. Every link is real and none is written down in one place;
  there is no test that walks it end to end, because it needs a real gateway —
  which is what `crates/e2e` is for.
- **`Entering` has no timeout.** A session stays there until the world says
  `PlayerEntered` or `PlayerRefused`, and today every exit from `World::enter`
  emits exactly one of the two — by construction, and only checkable by reading
  the function. A test that asserts "every early return from `enter` emits a
  refusal" would pin it; a fourth failure path added without one would strand a
  client in a phase nothing ever moves.
- **A logout leaves the phase on `Playing`.** `Command::LogoutRequest` answers
  the ack and stops; the character is not let go of until the socket closes and
  `Disconnect` runs. So between the ack and the client hanging up, in-world
  packets are still accepted from a connection that has announced it is leaving.
  Harmless today — the client sends nothing in that window — but it is the state
  the first draft called `LoggingOut`, and it is unnamed.

- **`ClientPacket` mixes the character screen in with the world.** S3 left one
  `unreachable!` behind: `0x5D` is a `ClientPacket` variant that
  `dispatch_world_packet` can never legitimately see, because the caller matches
  it out first. The invariant is real but it lives in a `match` arm in another
  file, which is exactly the shape this whole plan is trying to delete. The fix
  is at the decode seam — `parse_packet` already splits `Packet::Login` from
  `Packet::World`, and `0x5D` belongs on the screen side of that split with
  `0x00`/`0xF8` and `0x83`, at which point the arm cannot be written at all. Not
  done here because it moves a public protocol enum for what is today a
  one-line comment, and S5 rewrites this seam anyway.
- **The gate no longer says which packet it dropped.** Thirty arms could each
  name their own; one gate has only the connection. Naming the packet would mean
  either `ClientPacket`'s `Debug` — which carries bodies, so a `0x03` would put
  the player's typing in the log — or a per-variant name table, a second list to
  keep in step with the enum. Worth revisiting if a real client ever hits this
  path, because today it means a misbehaving client is one indistinguishable
  line per packet.

## To verify with a real client

- **S5 delays `0xA9` by up to one tick** (50 ms). It answers `0x91`
  synchronously today. The client is already waiting at that point, but this is
  the kind of thing that is fine in theory and a hang in practice.
- **Compression must not follow the phase.** A game socket is Huffman-compressed
  from the moment its `0x91` is read, refusal included; the flag stays in the
  binary's transport and is set once, irreversibly, at the hand-off. Reading it
  off a phase that lives in the world would put a channel round-trip between the
  socket and the question "is this stream compressed".

## Status

S1, S2 and S3 landed; S4 is next. Findings are recorded in
[`roadmap.md` §2](roadmap.md) under "A connection's state is kept in two tables
that must agree".
