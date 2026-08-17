# A window that owns itself: panes, not branches in `App`

The client draws six kinds of its own window — a container, a vendor's
catalogue, a paperdoll, a `0xB0` dialog, the skill sheet and the status frame —
and not one of them owns anything. Every gesture is a method on `App` that asks
"which window am I over" again and then takes the subject apart on the spot, and
every window's state is a public field of one shared struct that three different
files write to.

This plan gives each window kind a type that owns its own state and its own
input, and gives the client one place where an event is offered to windows in
order. A pane **takes readonly context in and hands mutations out**; it never
holds an `&mut App`, never reaches the shard, and never decides where it sits on
the screen.

## Why

Three things are wrong today, and they are one thing.

**Input is a chain of `bool`s that mean two things at once.** The wheel handler
is `scroll_skills() || scroll_vendor() || zoom()`
(`crates/client/app/src/event_loop.rs`), and that one `bool` answers both "this
event is mine" and "ask for a redraw". A shop list scrolled to its last row
answered "nothing moved", the chain fell through, and the wheel became a map
zoom under a pointer that had never left the window — fixed on 2026-08-17 by
making `scroll_vendor` answer the first question, which is a fix by convention
that the next window kind is free to get wrong again.

**No window has private state.** `Windows` (`crates/client/app/src/windows.rs`)
holds `vendor_scrolls`, `vendor_amounts`, `skills`, `held_skill`, `held_doll`,
`last_scroll`, `last_container_click` and `dialogs` as public fields. (The first
two left with S1 and the next two with S2 — the paragraph is what the plan was
written against, and the Steps below are what has gone.)
`own_windows.rs` writes them from the press path, `render_passes.rs` reads
*and* writes them while laying the frame out (`vendor_amounts.entry(..)` at
`render_passes.rs:190`), and `close_window` has to remember `vendor_scrolls
.remove(&serial)` by hand, because nothing ties a shop's scroll position to the
shop window's lifetime.

**A window talks to the shard from inside its own click handler.**
`press_on_own_window` calls `self.world.shard.link()` and sends `pick_up_item`,
`equip` and `use_object` in the middle of a 260-line function that also
branches on five window kinds. Nothing about a vendor's arithmetic can be tested
without a socket, an atlas and a whole `App` — which needs real client asset
files to construct at all.

## The shape this works toward

```rust
/// One of the client's own windows, as a component.
trait Pane {
    /// The art this pane will need packed before it can be laid out.
    fn art(&self, ctx: &PaneCtx) -> Vec<GumpArt>;

    /// Lay the pane out for this frame: the pictures the pass draws and the
    /// pointer is tested against.
    fn layout(&self, ctx: &PaneCtx) -> Option<Drawn>;

    /// Offer one input. The answer says whether the pane took it, whether the
    /// frame is stale because of it, and what it is asking the client to do.
    fn handle(&mut self, input: Input, ctx: &PaneCtx) -> Response;
}

struct Response {
    /// The event stops here: neither the camera, nor the world, nor a window
    /// under this one ever sees it.
    taken: bool,
    /// The frame is stale. **Separate from `taken` on purpose** — see D4.
    redraw: bool,
    /// What the pane wants done, in order. The manager performs these.
    out: Vec<Effect>,
}
```

Panes are stored in an `enum Pane`, one variant per kind, whose `match`
delegates to the type inside. The manager owns the list, the z-order, each
window's position, and the loop that offers an event to panes from the top down.

## Decisions

**D1. One type per window kind, held in an `enum`, not behind `dyn`.** Each kind
becomes a struct with private fields (`VendorPane { scroll, amounts }`,
`SkillsPane { tree, held }`, …) and an `impl Pane`. They are stored as
`enum Pane { Vendor(VendorPane), Skills(SkillsPane), … }` with one delegating
`match` per trait method.

`Box<dyn Pane>` was the alternative and is rejected. An `enum` is what makes a
seventh window kind a *compile error* everywhere the manager still needs to know
which kind it has — the same reason `WindowSubject` and `Drawn` are enums today
— and the delegating `match` is six lines per method, paid once. `dyn` would
also put an allocation and a vtable in the middle of a list this client walks
several times per frame, for a set of kinds that is known at compile time and
has grown by three in a year.

**D2. Position, z-order and the drag that moves a window belong to the manager.**
A pane reads `at` out of its context and **never writes it**: the cascade,
`raise_window`, `dragging` and the close gesture are all the manager's, and a
pane that wants to be moved simply declines the press.

`OwnWindow` keeps `subject` and `at` and **gains the pane beside them** —
`OwnWindow { subject, at, pane }`, built by `reconcile_own_windows` when the
window opens. An earlier draft of this decision said the record stays exactly as
it is and the panes live in a list of their own; that would be a second list
keyed by subject, and a subject in one and not the other is precisely the bug
class this plan is closing. State that lives in the record is dropped by the
`retain` that closes the window, which is what makes `close_window`'s manual
`vendor_scrolls.remove` deletable at all. This is what the user asked for in the phrase this plan was
commissioned with — *"позиция менеджится внешне"* — and it is also why a pane
does not need to know it is a window at all.

Coordinates stay **absolute gump pixels** for now, the way `Picture`, `Drawn`
and `gump_art::pick` already are, with the pane subtracting `ctx.at` where it
hit-tests — which is what `vendor::Window::contains` does today. Converting the
whole layer to window-local coordinates is a real improvement and a different
plan; it is in the Backlog, not here, because it touches the render crate and
this plan deliberately does not.

**D3. The context is readonly, and it carries the clock.**

```rust
struct PaneCtx<'a> {
    view: &'a WorldView,        // the authoritative picture
    resources: &'a Resources,   // art, tiledata, skill names, the gump atlas
    at: GumpPixel,              // where the manager has put this window
    cursor: GumpPixel,          // the pointer, in gump pixels
    modifiers: Modifiers,       // shift/ctrl, for the split-drag
    now: Instant,               // for every double-click pair
    hand: Option<ItemDrag>,     // what is on the cursor, if anything — D7
}
```

No `&mut App`, no `Link`, no `&mut WorldView`. `now` is passed rather than read
from `Instant::now()` inside a pane, for the reason the tick's `Rng` is owned
rather than ambient: a pair of clicks is a *timing* rule, and a rule read from
an ambient clock cannot be exercised by a test. `hand` is readonly for the same
reason it is in the context at all: a pane needs it to answer "was something
dropped on me" and to draw a wearable's preview, and no pane may fill or empty
it — see D7.

**D4. `taken` and `redraw` are two fields, because they are two questions.**
This is the whole of the wheel defect, stated as a type instead of a comment.
A pane at the end of its list took the notch (`taken: true`) and has nothing new
to draw (`redraw: false`); a pane whose hover tint changed took nothing
(`taken: false`) and is stale (`redraw: true`). Today's `bool` cannot say either
of those, and the chain in `event_loop.rs` reads it as whichever one the last
author had in mind.

**D5. Effects are the mutation, and `Outgoing` is most of them.**
`client-net`'s `Outgoing` (`crates/client/net/src/action.rs`) is already the
complete vocabulary of "what this client asks the shard for" — `Buy`, `Sell`,
`PickUp`, `Equip`, `Use`, `SkillLock`, `AnswerGump`. A pane returns
`Effect::Net(Outgoing)` and never touches the link; the manager sends it. The
rest of `Effect` is what only the manager can do:

```rust
enum Effect {
    Raise,                       // this pane to the top of the pile
    Close,                       // and off the list; overlay handled by the manager
    Grab(GumpPixel),             // start dragging this window, grabbed here
    Net(Outgoing),               // the shard's half — both halves of a transfer
                                 // among them, as two ordinary effects (D7)
    Open(LocalWindow),           // the skill sheet or the status frame
    Prompt(SplitPrompt),         // the client-side amount dialog, whose answer
                                 // has to find its way back — see the Backlog
}
```

Reusing `Outgoing` rather than minting a pane vocabulary is deliberate: a second
enum that means the same things is a second place to add a packet to, and the
translation between them would be a `match` whose arms are all identities.

`Open` names a `LocalWindow` — the skill sheet or the status frame — and not a
`WindowSubject`. Those two are the only kinds whose *existence* is this client's
own: a container or a paperdoll is open because the shard opened it, so asking
for one is `Net(Use)` or `Net(Paperdoll)` and the window appears when the view
grows the entry `reconcile_own_windows` turns into one. An `Open(WindowSubject)`
would have two unanswerable arms and would read as though a pane could conjure a
bag.

**D6. Art packing is a phase of its own, before layout.** `layout` takes `&self`
and a *shared* atlas, which is only possible because packing is a separate call:
`vendor::art_of()`, `container::art_of(..)`, `paperdoll::art_of(..)` and
`gump::art_of(..)` already exist as free functions and are already called ahead
of layout in `render_passes.rs`. The trait states that order instead of leaving
it to the caller: `art` for every pane, pack once, then `layout` for every pane.

**D7. The hand is a slot, not a gesture: a transfer is two transactions.** An
earlier draft of this decision said that lifting a sword out of a bag and
dropping it on a paperdoll is "one gesture over two windows", by analogy with an
egui drag payload, and kept the transaction with the manager for that reason.
The conclusion stands; the reason was wrong, and the reason is what decides the
shape.

Lifting and dropping are two independent requests with **server state between
them**. `0x07` puts the item into a slot on the connection, `0x08` or `0x13`
takes it out, and `0x27` bounces it back to where it came from. This shard's own
server is already built exactly that way: `Connection::held: Option<HeldItem>`
(`crates/server/state/src/connection.rs:107`) holds an item that is "in limbo
until a `0x08` lands it", one per connection because "a cursor holds one thing",
and a second lift is answered by bouncing the first
(`DragCancelReason::AlreadyHolding`, `crates/server/items/src/drag.rs:31`).

So the hand stays full for as long as the player likes. They can walk, open a
third bag, close the window the item came out of, or drop the connection — which
the server handles by name, because an item in the hand is off every sector and
out of everyone's `seen`. The pane that sourced the item need not exist when the
drop happens, and there is no gesture spanning anything.

That splits today's `ItemDragTransaction` along the seam `owns_cursor` already
admits — "after `Held` the shard may already have detached the item from its
source":

- `Pressed` — a press that has not moved yet and may still become a double-click
  "use". One window, one gesture, nothing sent. **Private to the pane**, like
  every other press-and-release pair the window layer already keeps
  (`held_doll`, `held_skill`, `gump::Dialogs::holding`).
- `Held` and `Dropped` — the hand, and a drop whose answer is in flight. A
  mirror of the server's slot, so **the manager's**, and readonly in `PaneCtx`.

There is one hand because the *server* has one, not because this plan picked a
number: the invariant is `AlreadyHolding`, read off the other end of the wire.
ClassicUO agrees about the owner — `ItemHold` is a field of `GameCursor`
(`GameCursor.cs:144`), not of any gump, and the eight gumps that mention it only
read it.

What this buys is that there is no `Lift`/`Drop` effect *pair*. A pane emits
`Net(Outgoing::PickUp)`; later, and possibly from another pane, some pane emits
`Net(Outgoing::DropInto | Equip | DropOnGround)`. Two ordinary effects with
nothing paired about them. What stays with the manager is the part that was never
a pane's: the precondition that a press does nothing at all while the hand is
full — today the first question `press_on_own_window` asks
(`own_windows.rs:724`) — the local projection that subtracts the item from its
source until the shard answers (`reproject_item_drag`), and drawing the item on
the cursor, which is the cursor's job and not a window's.

**D8. What is not a pane.** `own_windows`, `locally_closed`,
`reconcile_own_windows` and the cascade stay where they are — they are about
*which* windows exist, which is the manager's question and is already settled by
`docs/client_window_state.md`. `Drawn` stays the layout type; panes produce it.

## Steps

Each step compiles and the client runs at the end of it. Panes move in one at a
time, and until a kind has moved, its variant delegates to the existing `App`
method behind the same trait — so the router is real from S1 onward and there is
never a half-routed frame.

- [x] **S0. The trait, the context, the response, the router.** ✅ `panes.rs`
      and `panes/route.rs`: `Pane`, `PaneCtx`, `Input`, `Response`, `Effect`,
      `AnyPane`, and `App::deliver` — top-down, first `taken` wins.
      `event_loop.rs` no longer chains `||` for a window: the five `CursorMoved`
      calls, the three-term release, the press, the right-button close and the
      wheel are one `deliver` each. **Five things landed differently from the
      shape above, and each is written down where it belongs:**
      - `trait Pane` and `enum Pane` cannot both exist. The trait keeps the
        name; the enum is `AnyPane`.
      - The trait has **only `handle`** for now. `art` and `layout` (D6) join it
        with the first pane that has a layout of its own — a pane with no state
        cannot lay anything out, and `render_passes.rs` still lays out all six.
      - `Panes::deliver` is `App::deliver`, because the router still has to
        reach the legacy handlers and those need the whole `App`. The loop over
        the panes itself takes nothing but the list.
      - The panes live in `OwnWindow` (see D2), so the list *is*
        `Windows::own_windows` and there is no second one to keep in step.
      - A shim cannot call `press_on_own_window` from inside `handle`, because
        the context is readonly by construction and that method is not per-kind
        anyway — it hit-tests all six itself. So the router has **three rungs**:
        the manager's own gestures, then the panes, then the legacy chain
        reached only when no pane answered. The third rung is what S7 deletes,
        and while it stands the conflation the wheel defect was made of lives in
        one function instead of five call sites.

      `Effect::Prompt` is not there yet: nothing emits it until S6, and *who a
      modal's answer is addressed to* is a Backlog entry that has to be settled
      first.
- [x] **S1. Vendor.** ✅ `panes/vendor.rs`: `VendorPane { vendor, scroll,
      amounts }`, an `impl Pane` with all three methods, and every question
      about a shop gone from `App` — `scroll_vendor`, `confirm_vendor`, the
      vendor arm of
      `press_on_own_window`, the vendor arm of `render_passes.rs`'s layout loop
      and `Link::buy`/`Link::sell` are all deleted. `close_window`'s two manual
      `remove`s went with the two maps. **Six things landed differently from
      the shape above:**
      - **The context is two structs.** `PaneFrame` is what a pane may read
        while it is packed and laid out (`view`, `resources`, `at`, `cursor`,
        `hand`); `PaneCtx` is that plus what is only true of an *event*. D3
        wrote one, and one does not survive contact with `draw_gump_windows`:
        that function has the view, the files and the pointer, and would have to
        invent a clock, a modifier state and a z-order answer to fill fields no
        layout reads. `PaneCtx { frame, .. }`, so a pane says `ctx.frame.view`.
      - **`PaneCtx::drawn`** — how this window was laid out on the *last* frame.
        Not in D3, and it has to be: **what is clicked is what was drawn**
        (`Windows::drawn_windows`), so a pane that hit-tested a layout it worked
        out at press time would be asking a second question whose answer is free
        to differ from the picture the player is pointing at.
      - **`PaneCtx::under_pointer`** — this window covers the cursor and no
        window above it does. A pane knows where the cursor is *inside* itself
        and cannot know what is drawn over it, and z-order is the manager's by
        D2. It is `App::window_under_pointer`, which every legacy handler opens
        with, handed over instead of asked again.
      - **A located input stops at the window the pointer is on.** The walk in
        `offer_to_panes` breaks after that window for a press and a notch —
        nothing below it may answer either. Without it a moved-in pane two
        windows down takes a click that landed on a bag drawn over it, because
        the kinds that have not moved in decline everything. A release and a
        move stay unbounded: a release finishes a press wherever the pointer has
        got to, and a move is offered to every window.
      - **The hand-full gate moved to the manager**, out of
        `press_on_own_window`'s first lines and into `manager_gestures` — D7 said
        it stays with the manager, and it has to run ahead of the *panes* and not
        only ahead of the legacy chain: a shop that answered a press while the
        hand was full would count up a row instead of doing nothing.
      - **A pane knows what it is a pane of.** `AnyPane::of` hands the vendor's
        serial to `VendorPane::new`: a `0x3B` names the mobile it is addressed
        to, and that is not the subject, the position or the z-order.

      Two things this fixed by construction rather than on purpose. **The
      catalogue is chosen once**: `Stall::of` prefers the buy list, and the
      three old readers disagreed — the frame drew the shop's stock while
      Confirm sold the player's own goods, for a serial in both maps. And the
      order is zipped against the *lines*, so a quantity left over from a
      catalogue that has since shrunk cannot travel.
- [x] **S2. Skills.** ✅ `panes/skills.rs`: `SkillsPane { tree, held }`, an
      `impl Pane` with all three methods, and `own_windows/skills.rs` deleted
      whole — `skill_hit_under_pointer`, `skill_content`, `skill_clicked`,
      `drag_thumb` and `scroll_skills` with it, along with the skills arms of
      `press_on_own_window` and `release_on_own_window`, the skills arm of
      `render_passes.rs`'s layout loop, and `Link::set_skill_lock`/`use_skill`.
      `legacy_window_input`'s wheel arm is **empty**, which is the milestone
      this plan was commissioned for: the `||` chain that made a notch over a
      window a zoom of the map has no terms left.
      **Five things landed differently from the shape above:**
      - **`Windows::skills` did not become a field of the pane; it stopped
        existing.** The `Option<Tree>` was the tree *and* the openness, so this
        step had to answer both. The tree is `SkillsPane::tree`, and the
        openness is the window being in `own_windows` — which means
        `reconcile_own_windows` lost its `skills_open` argument rather than
        having it renamed: its `Skills` arm is now `true`, because a window is
        open by being in the list it is being asked about. Four writers of the
        old field (`close_window`, `sync_own_windows`, the disconnect arm,
        the paperdoll's button) are down to two callers of one door.
      - **That door is `windows::open_local_window`, and it is a free
        function.** `Effect::Open(LocalWindow::Skills)` performs it, and the
        paperdoll's Skills button — legacy until S5 — calls the same one, so
        there is no second way to open the sheet while the migration is half
        done. Its contract is that it is **idempotent**: pressing Skills again
        leaves the window it finds alone, which is what the old
        `get_or_insert_with(Tree::default)` was carefully spelling in two
        places. The status window's cascade in `reconcile_own_windows` is the
        same call now, so S3 is a `bool` to delete rather than a block to
        write.
      - **🚨 A local window has to be closed by name where the view is not the
        authority.** Everything else is dropped by `reconcile_own_windows`
        because the view stopped listing its subject; a skill sheet has no
        subject in the view, so the disconnect arm of `net_command.rs` retains
        it out of the list explicitly. That is the price of "presence is the
        fact" for a kind the view cannot answer for, and it is the honest
        version of the old `windows.skills = None`. S3 inherits the same line.
      - **The wheel's answer changed, and it is the only behaviour this step
        did not merely move.** `scroll_skills` returned one `bool` and returned
        it `true`, so a sheet at the bottom of its list took the notch *and*
        asked for a frame with nothing new on it. `SkillsPane::wheel` answers
        `consumed` there and `changed` otherwise — the harmless half of the
        conflation whose other half was the vendor's visible defect.
      - **The sheet's layout is unconditional, unlike a shop's.** A vendor pane
        answers `None` for a catalogue that has left the view, so
        `render_passes.rs` keeps a reachable arm for it; a skill sheet draws its
        own frame with nothing at all in the view, so its arm is unreachable and
        says so.

      **One ordering this step changed, and why it is safe.** The sheet's
      release used to be the *last* question asked on the way up — the third
      term of `release_container_item() || release_container_press() ||
      release_on_own_window()` — and it is now the first, because panes are
      offered an input ahead of the legacy chain. Nothing can be held by both:
      a press only reaches a pane while the hand is empty (the manager's gate,
      S1), and the press that fills the hand is a press on a container, so
      `SkillsPane::held` and an item transaction cannot be live at the same
      time. Any later kind that keeps a press of its own inherits this
      question, and S6's container is where it stops being trivially true.
- [x] **S3. Status.** ✅ `panes/status.rs`: `StatusPane`, a **unit struct** with
      an `impl Pane` — the kind that proves a pane with no state and no input at
      all is still a pane. The layout moved out of `render_passes.rs`, and
      `Windows::status` is gone: it was the last field in the client that said
      "this window is open" anywhere but in the list of open windows.
      **Four things landed differently from the shape above, and one backlog
      entry closed with the step:**
      - **`reconcile_own_windows` has no openness argument left at all.** S2 took
        `skills_open` and this takes `status_open`, so its signature is the view,
        the list and the overlay — nothing else. Both local kinds answer `true`
        in its `retain` for the same one reason, written once: *the window is
        open because it is here*.
      - **`WindowSubject::is_local()`, which is the Backlog entry this step was
        told to do with itself rather than after itself.** The disconnect arm of
        `net_command.rs` had to name `Skills` to drop it, and S3 would have added
        `Status` beside it — two names to keep in step with
        `open_local_window`'s. It is a predicate now: *nothing in the view holds
        this window open*. A third local kind adds a variant to that `matches!`
        and nothing else.
      - **`Effect::Open` is one arm, through `LocalWindow::subject()`.** It was
        two, and the second of them was `self.windows.status = true` — the field
        rather than the door. Both go through `windows::open_local_window` now,
        and so does the paperdoll's legacy button, whose two `bool`s became one
        `Option<WindowSubject>` for the same reason: one request, one door, one
        difference between them.
      - **The `None` arm of this pane's layout is reachable, unlike the sheet's.**
        The Status button asks for a fresh `0x11` and opens the window in one
        press, so there are frames in which the window is open and this client
        has not a single number to write on the frame. Drawing the empty gump
        then would be a status window belonging to nobody, so the pane answers
        `None` and the window appears with its numbers on the frame the reply
        lands — which is a shop's shape (`render_passes.rs` keeps a reachable
        arm for it), not a sheet's.

      **One ordering changed, the same one S2 changed.** The frame used to be
      appended by the *reconcile* on the frame after the press, because the
      press only set a `bool`; it is appended by the press now. Two windows
      opened on one frame can therefore cascade and stack in a different order
      than before — which was already true of the skill sheet, and is the same
      "position is the manager's" it always was.
- [ ] **S4. Dialog.** `gump::Dialogs` is already close to a pane — it owns the
      page, the switches, the typed text and the held button. Mostly a move.
- [ ] **S5. Paperdoll.** `held_doll`, `last_scroll`, `doll_clicked`, and the
      seven buttons. First kind whose effects are mostly `Open` and `Net`.
- [ ] **S6. Container.** Last, because it is the one the hand runs through: the
      first pane to own a `Pressed` of its own, to read `ctx.hand` for a drop
      onto itself, and to emit both halves of D7's transfer as separate effects.
- [ ] **S7. Delete the branches.** `press_on_own_window`,
      `release_on_own_window`, `hover_container_item`, `hover_paperdoll_item`
      and the `WindowSubject` matches inside them stop existing. `App` no longer
      knows what a vendor is. (`scroll_vendor` and `scroll_skills` were named
      here too and are already gone, with S1 and S2 — the wheel arm they were
      the two terms of is empty.)
- [ ] **S8. The test the wheel defect would have failed.** A pane exercised with
      a `PaneCtx` and no `App`: scroll a catalogue to its end, offer one more
      notch, assert `taken` and `!redraw`.
      **Half of it exists as of S1, and twice over as of S2**: three tests in
      `panes/vendor.rs` and three more in `panes/skills.rs`, each asking its
      pane's `wheel` for a `Response` directly, and each asserting the defect —
      `taken` at the end of the list, `!redraw` beside it. Injecting
      `Response::ignored()` into either turns that pane's three red, which is
      how both sets were checked.
      What is still missing is the same assertion *through* `handle`, which is
      what proves the pointer tests in front of it agree. That needs a
      `PaneCtx`, which needs a `&Resources` — and `Resources` is built in one
      place, out of real client asset files. It is the whole of what blocks
      this step now; "`App` needs asset files" was the old, larger version of
      the same sentence.

## Backlog

- **Window-local coordinates.** D2 keeps absolute gump pixels so this plan does
  not reach into the render crate. Every pane hit-test then begins by
  subtracting `ctx.at`, which is a line that can be forgotten — and a pane that
  forgets it hit-tests against the top-left of the screen, which looks like
  "the window is dead" from the outside. Worth doing after S7, when there are
  six panes to convert at once and one place that measures a cursor.
- **The vendor window and the skill window disagree about what a wheel over a
  window means.** Skills claims its whole frame; the vendor claims only its
  catalogue viewport (`catalogue_contains`), so a notch over the shop's buttons
  still reaches the camera. Both are panes now (S1, S2), so this is a per-pane
  decision that is *visible* as a decision — two `handle` arms, four lines
  apart in shape — but that does not settle it: somebody has to say which is
  right. ClassicUO claims the whole window.
- **`if let Some(window) = self.window.as_ref() { window.window.request_redraw() }`,
  twenty times.** `event_loop.rs` asks for a frame that way in every arm, and S0
  left the idiom alone rather than sweeping it while it was also changing what
  decides the ask. Now that a `Response` says whether the frame is stale, the
  honest shape is one `App::ask_redraw()` and arms that call it — or, better, an
  arm that returns its `Response` to one place that acts on it. Mechanical, and
  worth doing when S7 has finished moving what the arms *say*.
- **`stack_all_button_under_pointer` and `take_all_button_under_pointer` walk the
  window list and then ask `window_under_pointer()` inside the loop.** Which
  means the answer depends on a *second* top-down walk taken per iteration, and
  reads as "stop if this window is the one the pointer is on" — the opposite of
  what a control drawn on that window wants. Both are container furniture and go
  into `ContainerPane` at S6; whatever that predicate was meant to say has to be
  stated then, because a pane hit-tests itself and has no second walk to consult.
- **Who a modal's answer is addressed to.** D7 leaves `Pressed` inside the pane,
  and a Shift-drag suspends exactly that state while the client's own amount
  prompt is open: `split_pending` is set, and the answer arrives later from the
  shell as `finish_stack_split(decision)`, which reads `item_drag` back out of
  `Windows`. Once the press lives in a pane, that answer has to be *delivered* to
  the pane that asked — an `Input::Answered(..)` routed by identity rather than
  by "whoever is at the top", because the player can raise another window while
  the prompt is up. The same question will be asked again by any other
  client-side modal, so it is worth settling once rather than at S6.
- **The window under the pointer is worked out up to three times per mouse
  move.** `offer_to_panes` asks `window_under_pointer` for every input now, and
  the legacy `Move` arm's `hover_container_item` and `hover_paperdoll_item` each
  ask again. It was already twice before S1, and each walk is the window list
  against the pointer through `gump_art::pick`, which reads the atlas per texel.
  One answer per event, worked out once, when S7 has deleted the two other
  askers.
- **A vendor's ACCEPT and CLEAR tint on hover, and nothing asks for a frame when
  it changes.** The tint is decided in the layout from `ctx.frame.cursor`, so it
  is right whenever a frame is drawn — and what draws one is the animation
  clock, not the move that changed it. `VendorPane` declines `Input::Move`,
  which is honest about today and is not the answer: a pane whose picture
  depends on the pointer owes a `Response::stale()`, and that needs the pane to
  remember what the tint was. Cheap, and worth doing with S5's paperdoll, which
  has the same shape and an `App::hover_paperdoll_item` to delete.
- **The press that picks a window up is the manager's, and it lives in the
  legacy chain's tail.** A press that hit no furniture ends
  `press_on_own_window` with `raise_window` and a `dragging`, and that is what
  moves a status frame today — the pane declines every input, exactly as
  decision 2 says it should. But `manager_gestures` runs *ahead* of the panes,
  and this has to run *behind* them: a shop's Confirm button and a sheet's
  thumb have to be asked first. So S7 cannot simply move it up there; the
  router grows a fourth rung — the manager's gestures that are the *fallback*
  rather than the precondition — or `deliver` learns to run one after the
  chain has passed. Worth settling when S7 writes it, and worth knowing now
  that "delete the branches" leaves one behind that is not a branch.
- **A scroll that could not move still asks for a frame.** `SkillsPane::wheel`
  answers `consumed` at either end, and the arrows and the track beside it
  answer `changed` unconditionally: pressing Up at the top of the list is a
  redraw of a picture that has not changed. The wheel is the one that mattered
  — it is the one whose answer decides whether the camera hears the event — and
  the buttons are the same conflation with nothing riding on it. One line each,
  and the shape is already there to copy.
- **`Drawn` is produced by a pane but consumed by passes that know all six
  kinds — and there are two of them.** `render_passes.rs` walks
  `drawn_windows` to turn each kind's text into labels, and `presentation.rs`
  has a second walk with the same `(WindowSubject, Drawn)` arms. **They already
  disagree**: the vendor's arm draws its lines in one and is deliberately empty
  in the other, with a comment explaining that the labels are drawn beside their
  own art instead. That is `docs/parity.md`'s defect class exactly — one frame
  assembled in more than one place, so agreement is a coincidence rather than a
  property. After S7 these are the last per-kind branches left, and whichever
  one is right, a seventh window kind costs two branches and a chance to forget
  the second. A drawing question rather than an input one, so it is out of scope
  here.

## Status

**S0 through S3 built** (2026-08-17). The router is real and every input the
window layer sees goes through it. **Three kinds have moved in**: a shop owns
its scroll position, its chosen quantities, its art, its layout and its input;
the skill sheet owns its tree, the control the mouse is holding, its layout and
its input; and the status frame owns its layout, which is all it has. `App` no
longer knows what a vendor, a skill window or a status window is except to close
one. The other three behave exactly as they did — the third rung of
`App::deliver` is their old handlers, called only when no pane answered, and
`render_passes.rs` still lays their three kinds out.

What this changed for a player, in one line each. **The wheel** (S0): `taken`
rather than "did anything move" decides whether the camera hears a notch — the
defect this plan grew out of, stated as a type instead of as a convention.
**A shop that is in both catalogues** (S1): what is drawn and what Confirm sends
are now the same list; they were not. **Nothing at all** (S2): the skill sheet
behaves as it did, minus a frame it used to ask for at the end of its list.
**Nothing at all** (S3): the status frame opens on the press rather than on the
frame after it, which is a cascade order and not a picture.

The `||` chain the plan was written against is **gone**.
`legacy_window_input`'s wheel arm has no terms left, because both windows with a
wheel own it, and each answers the two questions as two fields.

**No window's openness is kept outside the list of open windows any more.**
`Windows::skills` went with S2 and `Windows::status` with S3, and
`reconcile_own_windows` — which took one `bool` per local kind — now takes the
view, the list and the overlay. The two kinds the view cannot answer for say so
with `WindowSubject::is_local()` rather than by name.

The `#[expect(dead_code)]` checklist is four: three fields nothing reads yet
(`PaneFrame::hand`, `PaneCtx::modifiers`, `PaneCtx::now` — the container's and
the paperdoll's) and `Effect::Open` with `LocalWindow` beside it, which is
*performed* but not yet asked for by a pane. The paperdoll asks for it at S5,
through the same `windows::open_local_window` its legacy button already calls.

Next is S4, the `0xB0` dialog. It is the kind that is already closest to a pane
— `gump::Dialogs` owns the page, the switches, the typed text and the held
button, keyed by gump id — so the step is mostly a move: the entry per gump
becomes the pane, `Dialogs::sync` becomes the `retain` that already runs, and
`press_on_own_window`'s dialog arm (including the `{ nomove }` rule, which is a
press that is taken and does *not* grab) becomes `DialogPane::handle`.
