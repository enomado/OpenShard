# OpenShard

A modern Ultima Online server engine, compatible with the original 2D client and
ClassicUO.

**Not a SphereServer clone.** The goal is the engine SphereServer would likely be
if it were designed today: compatible with the UO *protocol*, and with nothing
else about Sphere.

The gameplay content lives in a second repository, the **OpenShard Community
Pack**.

## Reference sources

Two other emulators. Neither is vendored, neither is a dependency, and neither is
copied — they are read. Where your checkouts of them are is your own business:
put the paths in `CLAUDE.local.md`, which is gitignored beside this file.

**SphereServer**, if a checkout is available: `Source-X/` (the C++ engine) and
`Scripts-X/` (the .scp scriptpack). Read it for **observed protocol behaviour**,
which is two decades of finding out which client breaks on what and is genuinely
hard-won. `Source-X/src/common/sphereproto.h` is the single most valuable file in
it.

**ServUO** (C#, on GitHub), for a second opinion on the same problems. Where the
two agree about the client, that is as close to a specification as this genre has.

Do **not** read either for architecture. Copying their structure is the one thing
this project exists to avoid — and where they agree about *engine* design, that
is often the strongest available argument for doing it differently. Both stop the
world to save it; `crates/server/persistence/src/journal.rs` explains at length
why this one does not.

## Decisions already made

These are settled. Don't relitigate them without being asked.

| Decision | Choice | Why |
|---|---|---|
| Client eras | **Multi-era from day one** | Retrofitting versioning means auditing every packet encoder twice. |
| Scripting runtime | **`deno_core` (V8) embedded** | Real JIT in-process. QuickJS is too slow for hot gameplay code; a Node sidecar puts IPC latency inside the tick. |
| Sphere scriptpack | **One-shot `.scp` → TS/TOML converter** | Keeps years of balance data without a runtime SphereScript parser. A build tool, not an engine feature. |
| First milestone | **Foundation first** (workspace, ECS, events) | Chosen over a login-to-walk vertical slice. |
| Language | Rust + Tokio | |
| Persistence | **SQLite or PostgreSQL**, operator's choice | Same `Store` trait; neither is a tier — SQLite runs a live shard fine. Never queried inside a tick. |
| Tooling | TypeScript, React, Next.js | |

## Where things stand

**The docs are the source of truth for this, and they are kept current.** Read
them rather than trusting a summary here — a status paragraph in this file is a
copy that goes stale silently, which is worse than no copy at all.

| | |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | The shape: layers, dependency rules, the crate map, how entities/events/protocol/persistence fit together, and what belongs in which file. |
| [`docs/roadmap.md`](docs/roadmap.md) | The order, and what is built. §6 is gameplay, system by system, with the protocol findings and reference-emulator arguments behind each one. |
| [`docs/style.md`](docs/style.md) | Comments explain *why*, tests name the behaviour they protect. |

The short version: the shard runs. `cargo run -p openshard-server` loads the
client's map and takes clients through login and character creation into a
shared, ticking world that saves itself to SQLite or PostgreSQL without ever
pausing. Combat, skills, magic, crafting, items, NPCs, quests and the creature
AI are real systems; `housing`, `guilds`, `metrics` and `plugins` are stubs — a
`Cargo.toml` and a `lib.rs` with a module doc, so the dependency graph is
visible. Gameplay is TypeScript on an embedded V8, hot-reloaded on save.

The workspace is three groups, and the group is part of the path:

- `crates/common/*` — `entities`, `events`, `protocol`, `movement`, `config`,
  `metrics`. Below the server; nothing here knows what a tick is.
- `crates/server/*` — `gateway`, `login`, `world` (the tick, the client file
  formats, the persistence journal), `state` (`WorldState` and the tables two or
  more systems read), `persistence`, `scripting`, `server` (the binary, glue
  only), and the gameplay systems: `chat`, `skills`, `magic`, `combat`, `items`,
  `crafting`, `npc`, `quests`, `ai`.
- `crates/client/*` — not started.

A gameplay system is a set of `fn(&mut WorldState)` in its own crate, owning its
domain events. `world/tick.rs` sequences them; it is orchestration, never rules.

## Rules that matter

**Never branch on `Era` for protocol decisions.** Ask `version.supports(Feature::X)`.
Features did not land in era-sized batches — tooltips at 4.0.0a, stat locks at
4.0.1a, tooltip hashes at 4.0.5a, all inside "AoS". An era check is wrong for
most clients it covers, and wrong silently: the client drops the packet rather
than complaining. Every boundary lives in `Feature::since`, once.

**No global mutable state.** `Registry` and `EventBus` are plain values the world
server owns. Nothing is a `static`, nothing is a singleton. This is what lets
tests spin up worlds freely and what will let the simulation shard across cores.

**Systems emit events; they do not call each other.** Combat does not call the
guild system — it emits `NpcKilled`. This is what keeps crates decoupled and makes
logging, metrics, replay and plugins fall out for free.

**Gameplay rules live in a domain crate, not in `tick.rs`.** A system is a
`fn(&mut WorldState)` in its own crate — the shape `combat`, `chat`, `skills`,
`magic`, `items`, `npc` already use. `world/tick.rs` is *orchestration*: the
command dispatch, the fixed system order, and the drain/queue plumbing. It once
reached 8,116 lines by absorbing tests, banker, persistence and door logic
inline; it is a fraction of that now, over child modules under `tick/`
(`command.rs`, `persist.rs`, `enter.rs`, `motion.rs`, `spawners.rs`, `decor.rs`,
`speech.rs`, `staff.rs`, … and the test files) — keep it that way. It has crept
back up and is due another pass. **A file over ~2k lines is overdue for
a split**: child modules of the owning module, one `impl` block per file,
`pub(super)` only on what crosses files; tests that read private state stay
child modules. See `docs/architecture.md` § "The shape of a file". When a
cohesive domain accretes in the tick, migrate it out. For movement the split is
decide-then-apply: a crate returns an intent (like `ai::think_one -> Option<dir>`)
and the tick calls `self.step(...)`; everything else the crate does directly on
`WorldState`.

**Domain events live with the crate that owns the rule.** `openshard-events` is
machinery only. `PlayerLogin` belongs to login, `HouseCreated` to housing. Keeping
event types out of the events crate is what stops it becoming a hub every crate
has to agree on.

**The database is never touched inside a tick.** The whole world is in memory.
`Journal::drain` takes a memcpy of what changed at one instant; a task nothing
waits on writes it. Both of the other emulators stop the world to save it —
ServUO literally pauses the network and broadcasts "please wait" — and the whole
of `crates/server/persistence/src/journal.rs` is the argument for why this one
does not.

**Persistence marks dirty from the event bus, not from each mutation.** A
`journal.touch()` beside every `registry.insert` works and decays: the first
system that moves a mobile without knowing persistence exists loses data
silently, and no test without a restart in it will catch that. Emitting the
event *is* the touch — a system already has to say a mobile moved, or the client
never hears about it. The exception is logout: `touch` is a promise to read the
entity later, and there will be no entity, so `Journal::keep` records it before
the despawn.

**Nothing writes to the world from outside the tick.** Network tasks queue a
`Command`; the tick applies it. Acting on a packet the moment it arrives would
run world code on whatever thread Tokio picked at whatever moment bytes landed,
and two clients racing would produce a different world depending on which packet
won. Every reply comes out of a tick, which is what keeps the two ends in one
order.

**`unsafe` is denied workspace-wide.** If you need two mutable borrows into one
structure, split a slice — see `Registry::for_each2_mut`.

**Protocol logic is sans-io.** Parsing and state machines take bytes and return
events; they do not touch sockets. The async layer is a thin adapter — see
`gateway::Connection` versus `gateway::Server`. What is hard here is byte
boundaries, and a real socket will not reproduce those on demand.

**Never trust a length off the wire.** Check it against the buffer before
reserving anything. `frame_client_packet` rejecting a claim above
`MAX_PACKET_SIZE` is what bounds gateway memory; nothing downstream re-checks.

**Take Sphere's numbers; audit its arithmetic.** The `MINCLIVER_*` table, the
step vectors, the Huffman key, the 200ms walk interval — all hard-won, all worth
copying verbatim. But its walk speed check compares a duration against a count
and does not survive being read closely, so `WalkPace` is a token bucket instead.
Copying something that does not add up is worse than not copying it.

**Read Sphere's shifts, never Sphere's comments.** Its IP comments say the
opposite of what its code does — `send.cpp` calls the branch emitting
`C0 A8 0B 06` "in reverse", because it reverses the *dword*, and the dword is an
`s_addr` that is already network order. Both readings are articulate and one is
wrong. Trace the bytes for a concrete address, in C if need be; the answer takes
a minute and the alternative shipped a shard nobody could log into.

**The game connection never says what the client is.** The version arrives in
the seed sent to the *login* server; the second connection opens with four bytes
of auth key and a `0x91`, and carries no version at all. A game session that only
knows its own socket defaults to the oldest dialect and sends a 1997 character
list to a modern client, which reads past the end of it and desynchronises —
surfacing as a garbage packet id hundreds of bytes later, looking nothing like a
version problem. The auth key is the only thing linking the two connections, so
anything that has to cross the gap rides on it. Sphere stashes it on the account
instead, which races when two clients share one.

**The 0x8C relay and the 0xA8 shard list carry the address in opposite orders.**
Relay: octets in order, always, no version gate. Shard list: reversed from 4.0.0
on, in order below it. Both SphereServer and ServUO agree exactly, which is as
close to a specification as this genre gets. A change that makes the two packets
consistent has broken one of them. And the relay is the expensive one: get it
wrong and the server sees a clean login and a clean disconnect, because the
client dialled a well-formed address that was not this machine and never came
back — the failure happens where this end cannot see it.

**The server remembers what is on each client's screen.** There is no "what can
you see" packet — only "draw this" (`0x78`) and "forget that" (`0x1D`) — so the
only way to send a mobile exactly once is to know what was sent before. That is
what `World::seen` is. Skip it and every step redraws every neighbour, which
looks fine with two players and melts at two hundred.

**Distance in UO is Chebyshev, `max(|dx|, |dy|)`.** The client draws a *square*
region. A circle here leaves the corners of every screen empty, and the bug
looks like mobiles popping in and out at the edges.

**Every visible action plays a sound and an animation, not just a state change.**
A swing that lands, a spell that resolves, a door that opens, a potion that is
drunk, a mobile that dies — each one, to a real client, has a sound (`0x54`), a
mobile action animation (`0x6E`, or `0xE2` gated on
`Feature::NewMobileAnimation`) and often a graphical/particle effect
(`0x70`/`0xC0`/`0xC7`). Sphere and ServUO fire these on essentially every action,
and their action/`SpellInfo` tables already carry the ids. A state-only system
passes its test and feels dead in the client — a fireball with no bolt reads as
broken even when the damage is right. So when you build or review a gameplay
action, emit its feedback too: broadcast through the same `seen`/interest
machinery as `0x78`, encoder in `crates/common/protocol`, default in core and
overridable in the pack off the domain event — the split combat and magic already
use. This was a systemic miss for most of the project's life; do not add to it.

**A tiledata flag means what the reference *reads* it for, not what its comment
says.** Sphere's header calls `UFLAG2_WINDOW` "window/arch/door can walk thru it",
and Sphere never once consults it in `CWorldMap`: the only three uses in the whole
engine are line-of-sight tests in `CCharLOS.cpp`, gated on `LOS_NB_WINDOWS`.
Honouring the comment in the *movement* check let anything the server moved walk
out through every wall segment with a window in it. It never showed for a player,
because the client refuses the step before it is ever sent — which is the general
trap: a server-side movement hole is invisible from the only end normally tested,
and surfaces as NPCs strolling through walls. (`NO_SHOOT` was mis-valued at `0x20`
in the same file, which is `UFLAG1_DAMAGE`; there is no `UFLAG1_NOSHOOT` at all.
Pin a flag's value in a test next to the constant.)

**A benchmark where nothing moves measures nothing.** A player who does not walk is
drawn once and never redrawn, so a standing world never pays interest management —
no `refresh_around`, no first-sight draw, none of the per-draw work of assembling
what a neighbour is wearing. `examples/town_bench.rs` reports standing and walking
side by side because the gap between them was three orders of magnitude: 0.107 ms
against 8.9 ms for the same town. The same applies to what a benchmark *builds* —
its predecessor spawned every creature with `equipment: Vec::new()` and placed no
decoration, so it exercised neither of the two columns a real facet spends its tick
in.

**A statistical test needs a companion that says the data is real.** The map test
asserting "neighbouring tiles have similar heights, so the block order is right"
passed against a `map0.mul` that was 90MB of zeroes — all-zero terrain is
perfectly smooth however you index it. `terrain::tests::the_map_is_not_degenerate`
exists to stop that. Any test that measures a property of real data can pass
vacuously on absent data.

**The map is in the `.uop`, not the `.mul`.** Modern clients ship both and the
`.mul` may be a stub full of zeroes. `Map::load_facet` prefers the UOP. See
`world::uop`.

**No client files are in this repository and none ever will be.** They are
copyrighted and they are not ours to redistribute. `world.client_files` points
at whatever install the operator already has; the tests that need one read
`OPENSHARD_CLIENT` and skip when it is unset. Do not commit a path to anyone's
machine, and do not name whose files you tested against — this crate reads a
*format*, not a particular shard's data.

## Working on this

```sh
cargo test --workspace          # includes doctests
cargo clippy --workspace --all-targets
cargo fmt --all
```

All three are expected to be silent. They are today; keep them that way. CI runs
exactly these on every pull request, so a red build is one of the three and
nothing subtler.

**Work lands through a pull request.** `main` is protected: no direct pushes, no
force-push, a review, and a merge commit (squash and rebase are off, so the
branch's history is what lands). Branch from `main`, open the PR, keep it to one
subject. `CONTRIBUTING.md` is the short version of this paragraph for people who
have not read this file.

**Commit messages carry no signature.** The message text only — never a
`Co-Authored-By:`, `Claude-Session:`, or any line mentioning Claude, Fable, Opus,
or any model or tool. This holds for every repo (the engine and the Community
Pack alike), and for PR bodies too.

**No Rust toolchain? Install one without root.** `rustup` is unreachable from the
sandbox — `static.rust-lang.org` is blocked — but Ubuntu ships versioned
toolchain debs that `apt-get download` can fetch and `dpkg -x` can unpack
anywhere:

```sh
cd /tmp && mkdir -p rdl88 r88 && cd rdl88
apt-get download rustc-1.88 cargo-1.88 libstd-rust-1.88 libstd-rust-1.88-dev \
                 rust-1.88-clippy rustfmt-1.88 libssh2-1 libhttp-parser2.9
for d in *.deb; do dpkg -x "$d" /tmp/r88; done
export PATH=/tmp/r88/usr/lib/rust-1.88/bin:$PATH
export LD_LIBRARY_PATH=/tmp/r88/usr/lib/x86_64-linux-gnu:/tmp/r88/usr/lib:$LD_LIBRARY_PATH
export CARGO_HOME=/tmp/cargohome CARGO_TARGET_DIR=/tmp/os-target
cargo test --workspace --exclude openshard-scripting
```

crates.io itself is reachable, so dependencies download fine. Only `rustup`'s
host is blocked. `openshard-scripting` is excluded because `deno_core` pulls a
prebuilt V8 from GitHub release assets, which this sandbox blocks (`403`) — that
crate builds on a normal dev machine, not here. It is also what holds the
workspace MSRV at 1.88: `deno_core`'s tree does not build below it.

**Building in a small sandbox? Watch `target/`.** It reached 2.7GB and filled the
disk hard enough that the sandbox could no longer start a shell to clean itself —
a wedge with no way out from inside. `[profile.dev.package."*"] debug = false` in
the workspace manifest is most of the fix and helps everyone. On top of that, in
a container and not in the repo, because they trade away things a human working
locally wants:

```sh
export CARGO_INCREMENTAL=0            # the incremental cache is per-crate and large
export CARGO_PROFILE_DEV_DEBUG=0      # no symbols at all, if backtraces are not needed
du -sh "$CARGO_TARGET_DIR"            # check it before it checks you
```

**`Cargo.lock` is committed and that is load-bearing.** `rust-version = "1.88"`
only holds because the lock pins dependency versions that respect it — a bare
`cargo update` will happily pull a transitive dep that wants a newer MSRV or a
newer edition and break the build on the stated one. If that happens, pin it:
`cargo update -p <crate> --precise <older>`.

There is no live pin today. There was one — `tokio-postgres` held at 0.7.12,
because from 0.7.13 it pulls a crypto stack (RustCrypto 0.11, `rand` 0.10) that
wanted Rust 1.85, above the old 1.82 MSRV. The scripting spike raised the MSRV to
1.88, which dissolved the reason for the pin, so it was dropped: the crate floats
on its declared `"0.7"` again (currently 0.7.18, `postgres-protocol` 0.6.12). The
mechanism above is what to reach for if a future update pulls something past 1.88.

Style: `docs/style.md`. In short — comments explain *why*, tests name the
behaviour they protect, and public items say what they are for rather than
restating their signature.

## Non-goals

Reimplementing SphereScript. Parsing `.scp` at runtime. Source compatibility with
Sphere. Legacy save formats. Mimicking Sphere's internals.
