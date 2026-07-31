# Stopping a shard: one word, heard everywhere — and what it owes the player

Living plan. The stop itself landed: a `gateway::Shutdown` is cloned into the
accept loop, every connection task and the tick, and `run_shard` returns only
once the world is on disk. What is written down here is the four ways that stop
is still not what it claims to be, and the order to fix them in.

As with [`connection_state.md`](connection_state.md): when reality contradicts a
decision here, change this file in the same commit that changes the code.

## Why

The mechanism is right and the manners are missing. Each of these is a way a
correct stop still costs somebody something.

1. **Only Ctrl-C is listened for.** `run()` wires `tokio::signal::ctrl_c` and
   nothing else. A shard under systemd — which stops a unit with `SIGTERM` — is
   *killed*, not asked, and loses everything since the last save cadence. That
   is precisely the loss the save-on-stop path was built to prevent, so the
   feature is currently absent in the one deployment that matters most.
2. **Bytes queued at the instant of the stop are dropped.** The connection task
   aborts its writer, so whatever the world had already handed the outbox never
   reaches the wire. Nothing a player can lose depends on it *today* — which is
   the trap: it is the transport for (3), so it has to be fixed first or the
   next item cannot work at all.
3. **The player is told nothing.** A clean, deliberate stop looks from the
   client exactly like the shard crashing: the screen freezes and the connection
   drops. Every other visible action in this engine plays a sound and says
   something (`CLAUDE.md`, "what the client actually does"); the shard's own
   departure is the one event that says nothing.
4. **A shard thread that dies during a test's teardown is a printed line.**
   `Running::halt` reports a panicked join with `eprintln!` because panicking
   inside `Drop` while another panic is unwinding aborts the process. Right
   instinct, wrong resolution: a test whose shard died should fail, and there is
   a standard way to have both.

## The shape this works toward

| | today | after |
|---|---|---|
| how a stop is asked for | Ctrl-C | Ctrl-C, `SIGTERM`, and one day a GM command |
| a save that will not finish | `SIGKILL`, and the operator guesses | a second signal exits loudly, saying what was lost |
| bytes in flight at the stop | dropped | drained, under a deadline |
| what the player sees | the connection dies | a system line, then the hang-up |
| a dead shard thread in a test | a line on stderr | a failed test |
| "the stop saved the world" | asserted nowhere | an end-to-end test that reopens the store |

## Decisions

Numbered so a later session can argue with one without reopening all of them.

**D1. Signals are watched in one function, and `cfg(unix)` lives inside it.**
`SIGTERM` and Ctrl-C mean the same thing to this process, so they end at the same
`Shutdown::stop()`; what differs is only how they are heard. Keeping the `cfg`
inside one small `stop.rs` keeps `run()` a straight read and keeps the
non-unix build from being a second arrangement nobody exercises.

`SIGHUP` is deliberately not a stop and not anything else. It conventionally
means "reload your config", this shard cannot reload one, and mapping it to a
stop would surprise an operator whose terminal closed.

**D2. The second signal is a force-exit, not a second polite stop.** A stop
awaits the save task, and the whole point of that task is that it may be slow —
a wedged Postgres, a disk that has gone away. Today the operator's only escape is
`SIGKILL`, which is indistinguishable to them from the shard having hung on its
own. So: the first signal asks, the second exits with a loud line naming how many
writes were abandoned and a non-zero code. Two deliberate signals is a clear
instruction; anything more patient would be pretending the choice is ours.

*Arguable.* The counter-case is that an operator who fat-fingers Ctrl-C twice
loses the save they were trying to take. The line must therefore say what it is
about to do the *first* time, so the second is informed.

**D3. A stop stops reading immediately and drains what is already written.**
The two halves of a connection are not symmetric at a stop. A packet *read*
after the stop is work queued for a tick that will not run — worse than useless,
because it can still mutate the session it passes through. A packet already in
the *outbox* is something the world decided to say while it was still the
authority, and the client is entitled to it.

The drain is bounded, because it cannot depend on the world being well: the
writer ends when every `OutboxTx` is dropped, which is the tick dropping its
sessions, which is the thing that might be broken in the first place. A deadline
turns "the shard did not stop" into "the shard stopped rudely".

**D4. The goodbye is a plain system line, not a protocol invention.** The client
already draws `SpokenMessage` from the system serial in system hue, and
`WorldState::system_message` already sends one. A shutdown notice is that, to
every entry in `WorldState::players`. No new packet, no new era question, no
`Feature::since`.

The text is a constant for now and becomes config the day there is an operator
command to schedule a stop (S7) — a message nobody can vary is not a setting,
it is a string.

**D5. The hang-up is the tick's; the connection's deadline is only a backstop.**
Both ends can close a connection, and if that is not settled somewhere it will be
settled differently by each of them. The world is the one that knows it has
finished talking, so *it* hangs up, by dropping its sessions after the flush. The
connection task's bounded wait exists for the case where that never happens.

**D6. The order after the loop is fixed, and the flush is welded to the
announcement.** Announce → flush the outbound queue → drop the sessions → end
every trade → last full snapshot → await the save task. Anything inserted
between the announcement and the flush drops the announcement, silently, and the
test in S3 exists because that is a one-line mistake.

**D7. A shard thread that panicked is a failure, not a diagnostic.**
`std::thread::panicking()` distinguishes the two cases `Running::halt` currently
conflates: unwinding already, so print; not unwinding, so `resume_unwind` and
let the payload reach the test harness.

## Steps

Each is a pull request. S2 must precede S3 — the notice needs a wire to travel
on. The rest are independent.

- [x] **S1. `SIGTERM` stops a shard, and a second signal exits it.**
      `crates/server/server/src/stop.rs`: `install() -> io::Result<Signals>` and
      `watch(signals, shutdown)`, replacing the inline `ctrl_c` task in `run()`.
      On unix `Signals` holds an installed `SIGINT` and `SIGTERM` stream and
      selects between them; elsewhere it is Ctrl-C alone. After the first, it
      keeps waiting, and the second exits with `2` — see D2, including the line
      the first one prints.

      Installation is a separate, synchronous step rather than the first line of
      the spawned task, which is a change from how this step was first written.
      Until the handler is installed, `SIGTERM`'s default disposition kills the
      process, so `spawn(watch(..))` followed by anything that could signal is a
      window in which the shard dies instead of stopping — in the binary as well
      as in the test. The two streams are also held across the first signal: one
      created fresh for the second wait would be deaf to a signal delivered
      between them.
      **DoD (met):** `stop::tests::a_sigterm_asks_the_shard_to_stop`, unix-only,
      sends itself `SIGTERM` (`kill -TERM` through `std::process`, so no new
      dependency) and sees the `Shutdown` flip inside a deadline. It installs
      before it signals, and says why.

- [x] **S2. A stop drains the outbox before it hangs up.** In
      `client_session_serve`, the shutdown arm stops reading and awaits the write
      task under `DRAIN_ON_STOP` (a constant beside it, 2 s) instead of aborting
      it; the abort stays as what happens when the deadline passes, and is
      harmless after a drain that finished.
      **DoD (met):** `a_stop_drains_what_the_world_queued_before_hanging_up`
      queues on the outbox *after* `stop()` — which is the order a shutdown
      really happens in, the world hearing the stop before it says anything — and
      reads the bytes, then the zero read. Checked to fail without the drain
      (`early eof`), which is the point of writing it first.

      Note for S3: `a_stop_hangs_up_on_a_client_that_is_already_connected` holds
      its outbox for the whole test and so now takes the full `DRAIN_ON_STOP`
      before hanging up. That is the deadline path working, and it is written
      down in the test.

- [x] **S3. The world says why.** `World::announce(&str)` beside
      `cancel_all_trades` in `world/src/tick/persist.rs` — walk
      `WorldState::players`, `system_message` each — and in `run_shard`, after
      the loop: `Shard::announce_shutdown`, which announces and flushes as one
      call, *then* the destructuring `let Shard { mut world, saves, .. }`, which
      is what drops the sessions. The flush is `Shard::flush_outbound`, lifted
      out of `tick` rather than copied, so there is one loop that sends and not
      two to keep in step.
      **DoD (met):** `a_stop_tells_the_player_before_it_hangs_up` in
      `crates/e2e/shard/tests/in_process.rs` reads events until the close and
      asserts the line was among them — so the assertion is the *order*, not the
      presence. Checked to fail with the flush moved before the announcement,
      which is the one-line mistake it exists for.

      **It also needed a decoder.** Our own client could not read `0x1C`:
      `ServerPacket::decode` had no arm for it, so the notice arrived as
      `Event::Undecoded` and the test could only have asserted on raw bytes. A
      shard announcing something its own client cannot read is not a feature, so
      `DecodePacket for SpokenMessage` is part of this step, with the two
      sentinels (`0xFFFFFFFF` speaker, `0xFFFF` graphic) folded back to `None`
      where the encoder folds them out.

- [x] **S4. `Running` raises what it currently prints.** The
      `std::thread::panicking()` guard of D7: unwinding already, so `eprintln!`;
      not unwinding, so `std::panic::resume_unwind(payload)` — the shard's own
      payload, not a message about it, so the test reports what actually failed.
      **DoD (met):** `a_shard_thread_that_panicked_fails_the_test` in
      `crates/e2e/shard/src/lib.rs` builds a `Running` over a thread that panics
      — the fields are private to the crate, so this is the one place it can be
      done — and `#[should_panic(expected = ...)]` on `stop()` names the thread's
      message, which is what pins the payload travelling rather than a panic
      merely happening.

      The not-double-panicking half is not tested: a test that panics inside a
      panic aborts the runner rather than failing, so there is nothing to assert
      on. What makes it safe is that `halt` is idempotent — the unwind out of
      `stop` leaves the handle already taken, so the `Drop` on the way out joins
      nothing — and that argument lives in the comment on `halt`.

- [x] **S5. A gate that has closed does not spawn onto a dead runtime.**
      `InProcess` is `Clone` and outlives its `Running`, so a dial after the stop
      reaches a `tokio::runtime::Handle` whose runtime is gone. `Gate::serve`
      returns early when `is_stopping()`, dropping the stream so the caller sees a
      closed pipe. Its return type is now `Option<ConnectionId>` — `None` means
      "not served", and no id is minted for a session that will never exist.

      **What `Handle::spawn` does there, checked rather than assumed:** it does
      not panic and does not hang. The future is dropped without ever being
      polled and the `JoinHandle` resolves to `JoinError::Cancelled`. So the
      stream *was* already being closed — by dropping the task that owned it,
      silently and by accident. The refusal makes the same outcome deliberate,
      and covers the case the accident does not: a gate that is stopping while
      its runtime is still alive, where a client would get a whole login
      conversation whose events go onto a channel the tick has stopped draining.
      The accept loop takes the same answer as its `biased` select does, for the
      stop that lands between the two lines.
      **DoD (met):** `a_gate_that_is_stopping_serves_nobody` in
      `crates/server/gateway/src/server.rs` — no id, a closed stream, and no
      `Connected` on the channel. Checked to fail without the guard.

      ⚠️ The e2e half, `dialling_a_shard_that_has_stopped_gets_a_closed_pipe`,
      **passed before the change too**, and is kept knowing that: the cancelled
      task closes the pipe, so the client sees the same thing either way. It pins
      the caller-visible contract — a dial after a stop ends rather than hangs —
      and the unit test beside the code is what pins the mechanism.

- [ ] **S6. Prove that a stop saves.** "`run_shard` returns only once the world
      is on disk" is the claim the whole shutdown tail exists for, and nothing
      asserts it. An end-to-end test with a SQLite file: log in, take a step,
      stop, reopen the store, and find the character at the new position.
      **DoD:** the test, and a temporary path that does not need a new dependency
      (`std::env::temp_dir` plus the pid) — and it must fail if the snapshot is
      moved after the `drop(saves)`, which is the reordering it is really there
      to catch.

- [ ] **S7. Later: an operator's stop, from inside the world.** A GM command that
      asks for a stop, optionally in N minutes, with the countdown as tick counts
      and the announcements of D4 along the way. The sketch: the world must not
      hold the `Shutdown` — nothing writes to the world from outside the tick and
      the world should not reach outside it either — so the command becomes an
      event the shard reads after the tick and turns into `Shutdown::stop()`.
      S1 through S6 are what make this safe to add rather than a second stop path.

## Backlog, found on the way

- **`Box::leak` on the config in both `e2e` spawns.** Both say `run_shard`
  borrows the config for the life of the process, but the future is awaited
  inside the same `block_on` scope, so a local and a `&` may well compile. Fifty
  worlds started and dropped is fifty leaked configs otherwise. Verify, and if it
  compiles, delete the leak and the paragraph that justified it.
- **`DRAIN_ON_STOP` is a constant, not a setting.** Right until an operator has a
  shard where it is wrong; the number belongs beside `save_every` in
  `[persistence]` if it ever moves.
- **Nothing says how long a stop took.** The tail does real work — a full sweep
  and however many queued writes — and the log currently goes quiet between
  "shutdown requested" and "world saved". One `took = ?elapsed` on the last line
  is the whole fix, and it is what would tell an operator whether their stop
  timeout is too tight.
- **`save_loop` has no bound and `run_shard` awaits it forever.** D2's
  force-exit is the mitigation, not the fix: a store that never returns leaves
  the shard in a state where the only honest thing left is to say which snapshots
  were abandoned. That means the save task counting what it has not written.
- **The force-exit of D2 is untested, and structurally hard to test.** The second
  signal ends the process, so proving it takes a child process — the shard binary
  started, signalled twice, and its exit status read — which is the out-of-process
  test this repository has otherwise avoided. Worth doing once the binary has any
  other reason to be driven from a test; not worth building the harness for alone.
- **The client decodes `0x1C` and draws nothing with it.** S3 wrote the decoder
  because the shutdown notice had to be readable; nothing in `WorldView` keeps
  what was said, so a client built on it still has no journal. The next thing
  that speaks to a player — a GM line, an NPC — will want the same, and the place
  for it is `view.rs` beside the rest of what the server has shown.
- **`Shard::announce_shutdown` is the only caller of `World::announce`.** A GM
  broadcast is the obvious second, and S7's countdown is the third; until one of
  them exists the method is a one-use seam and its shape is unproven.
- **A stop mid-`Entering` is untested.** A client whose `Command::Enter` is
  queued when the stop arrives has no entity, so it gets no announcement — only
  the hang-up. That is correct, and nothing pins it.
- **Nothing tests that the playground boots** — carried over from
  [`client.md`](client.md), and now with one more thing to get wrong, since the
  playground stops its shard after the window closes.

## Status

S1 through S4 are in: a shard under systemd is asked rather than killed, an
operator with a wedged save has a way out that is not `SIGKILL`, what the world
queues on its way out reaches the wire, a player is told why their screen is
about to go, a shard thread that dies during a test's teardown fails that test
instead of printing at it, and a gate that has been asked to stop refuses rather
than spawning onto a runtime that is going away. S6 is what is left: the test
that the whole shutdown tail exists for. The commit that created this plan is the one that landed the stop
itself; [`docs/client.md`](client.md) → "Stopping is one word, and everything
hears it" is the design it is built on, and [`roadmap.md`](roadmap.md) §8 points
here rather than repeating the list.
