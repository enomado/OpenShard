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
`last_scroll`, `last_container_click` and `dialogs` as public fields.
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
`OwnWindow { subject, at }` stays exactly as it is. A pane reads `at` out of its
context and **never writes it**: the cascade, `raise_window`, `dragging` and the
close gesture are all the manager's, and a pane that wants to be moved simply
declines the press. This is what the user asked for in the phrase this plan was
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
}
```

No `&mut App`, no `Link`, no `&mut WorldView`. `now` is passed rather than read
from `Instant::now()` inside a pane, for the reason the tick's `Rng` is owned
rather than ambient: a pair of clicks is a *timing* rule, and a rule read from
an ambient clock cannot be exercised by a test.

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
    Net(Outgoing),               // the shard's half
    Open(WindowSubject),         // a paperdoll's backpack button, say
    Lift(ItemPress),             // D7
    Drop(PendingDrop),           // D7
    Prompt(SplitPrompt),         // the client-side amount dialog
}
```

Reusing `Outgoing` rather than minting a pane vocabulary is deliberate: a second
enum that means the same things is a second place to add a packet to, and the
translation between them would be a `match` whose arms are all identities.

**D6. Art packing is a phase of its own, before layout.** `layout` takes `&self`
and a *shared* atlas, which is only possible because packing is a separate call:
`vendor::art_of()`, `container::art_of(..)`, `paperdoll::art_of(..)` and
`gump::art_of(..)` already exist as free functions and are already called ahead
of layout in `render_passes.rs`. The trait states that order instead of leaving
it to the caller: `art` for every pane, pack once, then `layout` for every pane.

**D7. An item drag crosses two panes, so it stays with the manager.** Lifting a
sword out of a bag and dropping it on a paperdoll is one gesture over two
windows and possibly the world; `ItemDragTransaction` is therefore *not* moved
into a pane. Panes report `Effect::Lift`/`Effect::Drop` and the manager owns the
transaction — the same shape egui uses for a drag payload, and the same reason:
the only thing that can own a gesture spanning two components is the thing that
contains both.

**D8. What is not a pane.** `own_windows`, `locally_closed`,
`reconcile_own_windows` and the cascade stay where they are — they are about
*which* windows exist, which is the manager's question and is already settled by
`docs/client_window_state.md`. `Drawn` stays the layout type; panes produce it.

## Steps

Each step compiles and the client runs at the end of it. Panes move in one at a
time, and until a kind has moved, its variant delegates to the existing `App`
method behind the same trait — so the router is real from S1 onward and there is
never a half-routed frame.

- [ ] **S0. The trait, the context, the response, the router.** `panes/mod.rs`:
      `Pane`, `PaneCtx`, `Input`, `Response`, `Effect`, `enum Pane`, and
      `Panes::deliver` — top-down, first `taken` wins. Every variant is a shim
      that calls the `App` method that handles that kind today. `event_loop.rs`
      stops chaining `||` and calls `deliver` once per input.
- [ ] **S1. Vendor.** The kind the wheel defect was found in. `vendor_scrolls`
      and `vendor_amounts` leave `Windows` and become private fields;
      `render_passes.rs` stops entering those maps; `close_window`'s manual
      `vendor_scrolls.remove` goes away with them.
- [ ] **S2. Skills.** `skills`/`held_skill` follow. `Windows::skills` being
      `Some` is what "the window is open" means today (see its doc comment);
      that fact becomes the pane's presence in the list, which is where every
      other kind already keeps it.
- [ ] **S3. Status.** The smallest kind, and the one that proves a pane with no
      input at all is still a pane.
- [ ] **S4. Dialog.** `gump::Dialogs` is already close to a pane — it owns the
      page, the switches, the typed text and the held button. Mostly a move.
- [ ] **S5. Paperdoll.** `held_doll`, `last_scroll`, `doll_clicked`, and the
      seven buttons. First kind whose effects are mostly `Open` and `Net`.
- [ ] **S6. Container.** Last, because it is the one the item drag runs through
      — D7's `Lift`/`Drop` are exercised here for the first time against a real
      second pane.
- [ ] **S7. Delete the branches.** `press_on_own_window`,
      `release_on_own_window`, `scroll_vendor`, `scroll_skills`,
      `hover_container_item`, `hover_paperdoll_item` and the `WindowSubject`
      matches inside them stop existing. `App` no longer knows what a vendor is.
- [ ] **S8. The test the wheel defect would have failed.** A pane exercised with
      a `PaneCtx` and no `App`: scroll a catalogue to its end, offer one more
      notch, assert `taken` and `!redraw`. Impossible to write today at any
      price — `App` needs client asset files to exist.

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
  still reaches the camera. Panes make this a per-pane decision that is *visible*
  as a decision, but it does not settle it — somebody has to say which is right.
  ClassicUO claims the whole window.
- **`Drawn` is produced by a pane but consumed by a pass that knows all six
  kinds.** After S7 the pass's `match` is the last place with a per-kind branch.
  It is a drawing question rather than an input one, so it is out of scope here,
  but it is where a seventh window kind will still cost a branch.

## Status

Nothing built. The wheel defect this plan grew out of is fixed on its own terms
(`App::scroll_vendor` answers "was the notch taken", 2026-08-17) — that fix is
the convention S0 replaces with a type, and it should not be extended to a
third scrolling window while this plan is open.
