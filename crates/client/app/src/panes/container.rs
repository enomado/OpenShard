//! A bag's window as a component: the last of the six kinds to move in, and
//! the one the hand runs through.
//!
//! Step 6 of `docs/window_components.md`. Every kind before this one could be
//! moved without touching what is on the cursor; this one cannot, because a
//! bag is where an item is lifted from and where it is put down, and both of
//! those are the *shard's* state as much as this client's. Decision 7 is the
//! seam that made it possible:
//!
//! - **The press is this pane's.** [`ItemPress`] has sent nothing. It is one
//!   window's gesture, it may still turn out to be the first half of a
//!   double-click use, and it dies with the window — exactly like a doll's
//!   held button or a dialog's.
//! - **The hand is the manager's.** Once a `0x07` has gone out the item is on
//!   the shard's cursor, one per connection, and no pane may fill or empty
//!   that slot. A pane asks with [`Effect::Lift`] and [`Effect::Drop`] and
//!   reads [`PaneFrame::hand`] to draw what it must.
//!
//! # What a bag is, on the wire
//!
//! A `0x24` names one gump graphic and a `0x3C` lists what is in it, each item
//! with a coordinate measured from the background's top-left corner. There is
//! no layout program, no page and no button — see
//! [`openshard_client_render::container`], which is the whole of the drawing.
//! Everything else this window does is a client-side gesture over that
//! picture: the double click that uses an icon, the drag that lifts one, the
//! two plates below the frame, and the tint under the pointer.

use std::time::Instant;

use openshard_client_net::action::Outgoing;
use openshard_client_net::view::WorldView;
use openshard_client_render::container;
use openshard_client_render::gump::{self as gump_art, GumpArt, GumpAtlas, GumpPixel, Picture};
use openshard_protocol::containers::ContainedItem;
use openshard_protocol::gump::GumpPoint;
use openshard_protocol::serial::Serial;
use openshard_protocol::speech::Font;
use openshard_protocol::wire::{Graphic, Hue, Layer, RawLayer};

use crate::DOUBLE_CLICK;
use crate::panes::{Answer, Button, Effect, Input, Line, Pane, PaneCtx, PaneFrame, Response, SplitPrompt};
use crate::windows::{DragOrigin, Dragged, Drawn, Hand, ItemPress, PendingDrop};

/// The face both of this window's labels are written in — `fonts.mul`'s face
/// 1, the same one every other label in this client uses.
const LABEL_FONT: Font = Font(1);

/// How far down and right of the pointer a hover label sits.
///
/// The paperdoll's is the same step, so the two hovers read as one behaviour
/// rather than two that happen to both be near the cursor.
const HOVER_OFFSET: GumpPixel = GumpPixel::new(14, 18);

/// Where the first swept item is put down in the backpack, and how far apart
/// the rest are laid.
///
/// The backpack stays authoritative about the grid slots and any stack merge;
/// this only keeps the icons from landing in one heap on the way there.
const SWEEP_ORIGIN: GumpPoint = GumpPoint::new(20, 20);
const SWEEP_STEP: i32 = 18;
const SWEEP_COLUMNS: usize = 6;

/// The one graphic this client wields on a double click instead of asking the
/// shard to *use* it.
///
/// A katana's ordinary double click is a wield, and the shard has no packet
/// that means "wear this from where it is": the reference client lifts the
/// item and equips it in one breath, which is the pair below. Every other
/// graphic is a `0x06` and the shard decides what using it means.
const KATANA: Graphic = Graphic(0x13FF);

/// One open container window.
#[derive(Debug)]
pub struct ContainerPane {
    /// Which bag this is a window of.
    ///
    /// A shop's serial and a doll's mobile, one kind over: the subject carries
    /// a key, and the pane is handed it when the window opens because every
    /// packet this window sends names it — see
    /// [`AnyPane::of`](crate::panes::AnyPane::of).
    container: Serial,
    /// The press on one of this bag's icons, until it becomes a lift, a click
    /// or nothing.
    ///
    /// **Decision 7's first half, and the field this whole step was last in
    /// the plan for.** Nothing has been sent while this is alive: the item is
    /// still in the bag, still drawn there, and a release now is a click that
    /// may pair with the next one. It is the pane's for the reason
    /// [`PaperdollPane::held`](crate::panes::paperdoll) and
    /// [`DialogPane::held`](crate::panes::dialog) are — one window, one
    /// gesture, and a window that closes takes its unfinished gestures with
    /// it.
    pressed: Option<ItemPress>,
    /// The icon under the pointer, tinted on the next frame.
    ///
    /// `Windows::hovered_container_item` kept this for every bag at once,
    /// keyed by nothing at all — there was one field and the topmost window
    /// won. Per-pane, two bags can each remember their own, and the one the
    /// pointer is on is the only one that answers, because the tint is written
    /// only while [`PaneCtx::under_pointer`] says so.
    hovered: Option<Serial>,
    /// The last completed click on one of this bag's icons, and when.
    ///
    /// The pair that makes a double click a *use*. It was
    /// `Windows::last_container_click`, one field for every open bag, so two
    /// clicks on two icons in two windows could pair if they happened to be
    /// the same item — which is not a thing that can be arranged, but was not
    /// a thing the rule refused either. Per-pane, the window half is gone by
    /// construction and what is left is the icon and the clock.
    last_click: Option<(Instant, Serial)>,
}

/// A container, laid out for one frame: the pictures, what they *are*, and
/// what is written over them.
///
/// The middle field is what a bag's [`Drawn`] did not use to carry, and it is
/// the rule `drawn_windows` exists for, stated properly. A click used to be
/// turned into an item by picking an index out of the pictures and then
/// counting that far into a list rebuilt from the view — with the lifted icon
/// filtered out again, by hand, in the same order the layout had filtered it.
/// Two walks, one subtraction each, and nothing but care keeping them in step.
/// Now the list the pictures were built from travels with them, and
/// [`Window::item_at`] is a lookup.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Window {
    /// The background and every icon on it, in painter's order — what the
    /// pointer is tested against.
    pub pictures: Vec<Picture>,
    /// The icons, in the order the pictures after the background were built
    /// from them.
    pub contents: Vec<ContainedItem>,
    /// The plate's caption below the frame, and the name of whatever the
    /// pointer is resting on.
    pub lines: Vec<Line>,
}

impl Window {
    /// The icon at `cursor`, or `None` for the bag itself and for the pixels
    /// its art leaves transparent.
    ///
    /// Picture zero is the background — a press on it is a press on the
    /// *window*, which is the manager's business — so the answer is one place
    /// further along, and index arithmetic that goes the wrong way answers
    /// `None` rather than the last icon.
    pub fn item_at(&self, cursor: GumpPixel, atlas: &GumpAtlas) -> Option<ContainedItem> {
        let index = gump_art::pick(&self.pictures, cursor, atlas)?;
        self.contents.get(index.position().checked_sub(1)?).copied()
    }
}

/// The one client-side plate a bag may have drawn below it.
///
/// Neither is in the shard's art: a `0x24` names one gump and it has no spare
/// room for a control, so both are captions this client writes under the
/// window and tests as boxes. Which of the two a window has is decided in one
/// place — [`ContainerPane::plate`] — and read by both the layout that draws
/// the caption and the press that acts on it, where there used to be a
/// predicate in `render_passes.rs` and a second one in `App` that had to agree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Plate {
    /// Sweep the lot into the backpack. Every bag but the backpack itself:
    /// there is nowhere to sweep your own pack to.
    TakeAll,
    /// Compact the like piles. The backpack's own, because that is where the
    /// loot ends up.
    StackAll,
}

impl Plate {
    /// Where it sits and how big it is, or `None` until the window's own art
    /// has been packed — the plate hangs off the bottom edge, so its position
    /// is the background's height.
    fn button(self, atlas: &GumpAtlas, gump: Graphic, at: GumpPixel) -> Option<container::ActionButton> {
        match self {
            Self::TakeAll => container::take_all_button(atlas, gump, at),
            Self::StackAll => container::stack_all_button(atlas, gump, at),
        }
    }

    /// Its caption.
    const fn label(self) -> &'static str {
        match self {
            Self::TakeAll => container::TAKE_ALL_LABEL,
            Self::StackAll => container::STACK_ALL_LABEL,
        }
    }
}

/// Which plate a window has, as a function of the three things that decide it.
///
/// A free function of what it actually reads, so the rule can be pinned
/// without a view — and one rule, where there used to be three that had to
/// agree: two `*_under_pointer` walks that decided which press to answer, and a
/// pair of `if`s in the text pass that decided which caption to draw. A window
/// that drew "Take all" and answered nothing would look broken and be silent
/// about it.
fn plate_of(container: Serial, backpack: Option<Serial>, shop: bool) -> Option<Plate> {
    if shop {
        // A shop's crate is not loot to sweep and not a pile to compact: its
        // rows are bought with a `0x3B`.
        return None;
    }
    match backpack == Some(container) {
        true => Some(Plate::StackAll),
        false => Some(Plate::TakeAll),
    }
}

/// Whether a click on `item` at `now` is the second of a pair.
///
/// The same shape as the paperdoll's scroll pairing, and for the same reason
/// the window half of it is missing: each bag keeps its own, so two clicks in
/// two windows cannot pair by construction. What is left is the icon and the
/// clock — a click on one bottle and then on the next is two first clicks, and
/// the second does not drink the first.
fn pairs(last: Option<(Instant, Serial)>, now: Instant, item: Serial) -> bool {
    last.is_some_and(|(at, serial)| serial == item && now.duration_since(at) <= DOUBLE_CLICK)
}

/// What one completed pair of clicks on an icon asks for.
///
/// A `0x06` for everything the shard knows how to use, and for [`KATANA`] the
/// lift-and-equip a drag onto the doll would have sent. **Both halves of that
/// pair are ordinary [`Effect::Net`]s and neither is [`Effect::Lift`]**: there
/// is no moment at which the player is holding the sword, so filling the hand
/// would draw an icon under a pointer nobody is dragging and leave it there if
/// the shard refused the equip.
fn double_clicked(item: ContainedItem, worn: RawLayer, player: Serial) -> Response {
    let answer = Response::changed();
    if item.graphic != KATANA {
        return answer.with(Effect::Net(Outgoing::Use(item.serial)));
    }
    answer
        .with(Effect::Net(Outgoing::PickUp {
            item: item.serial,
            amount: item.amount,
        }))
        .with(Effect::Net(Outgoing::Equip {
            item: item.serial,
            layer: worn,
            mobile: player,
        }))
}

/// The sweep behind "Take all": one ordinary lift and drop per item, in the
/// order the bag listed them.
///
/// Every pair stays on the server-authoritative drag path, so reach,
/// ownership and weight are all checked the way they would be for a hand
/// drag. The grid is only to keep the icons apart on the way in — the backpack
/// decides where they actually land, and whether two of them merge.
///
/// Nothing touches the hand, for [`double_clicked`]'s reason: the player never
/// holds any of this.
fn take_all(contents: &[ContainedItem], backpack: Serial) -> Vec<Effect> {
    contents
        .iter()
        .enumerate()
        .flat_map(|(index, item)| {
            let column = (index % SWEEP_COLUMNS) as i32;
            let row = (index / SWEEP_COLUMNS) as i32;
            [
                Effect::Net(Outgoing::PickUp {
                    item: item.serial,
                    amount: item.amount,
                }),
                Effect::Net(Outgoing::DropInto {
                    item: item.serial,
                    container: backpack,
                    at: GumpPoint::new(
                        SWEEP_ORIGIN.x + column * SWEEP_STEP,
                        SWEEP_ORIGIN.y + row * SWEEP_STEP,
                    ),
                }),
            ]
        })
        .collect()
}

/// The backpack this character is wearing, if any.
fn backpack_of(view: &WorldView) -> Option<Serial> {
    view.player
        .equipment
        .iter()
        .find(|item| item.layer == Layer::BACKPACK)
        .map(|item| item.serial)
}

/// What the hover label says: the item's name, and how many of it.
fn hover_text(item: &ContainedItem, name: &str) -> String {
    format!("{name} ({})", item.amount.0)
}

impl ContainerPane {
    /// The pane for the bag `container` names.
    pub const fn new(container: Serial) -> Self {
        Self {
            container,
            pressed: None,
            hovered: None,
            last_click: None,
        }
    }

    /// The icon this window is tinting, if the pointer is on one.
    ///
    /// The one thing outside the window layer that asks a bag a question: the
    /// tooltip pick order puts an icon in an open bag in front of anything in
    /// the world behind it, and that order is `picking_query`'s rather than
    /// this pane's. `App::hovered_container_item` is the walk that asks.
    pub const fn hovered(&self) -> Option<Serial> {
        self.hovered
    }

    /// Whether this window is an ordinary bag rather than a shop's crate.
    ///
    /// A shop is a [`WindowSubject::Vendor`](crate::windows::WindowSubject)
    /// and never a `Container`: `reconcile_own_windows` refuses to open one for
    /// a serial the view holds a buy catalogue for, and it runs at the top of
    /// every frame — so a *layout* can take this for granted, and does.
    ///
    /// A press cannot, and that is the only reason this exists. Input arrives
    /// between frames, so a `0x74` folded since the last reconcile leaves this
    /// window standing as a bag for one gap, and a press landing in it would
    /// put a `0x07` on the wire for a shop's stock — which is bought with a
    /// `0x3B` or not at all.
    fn ordinary(&self, frame: &PaneFrame<'_>) -> bool {
        !frame.view.vendor_buys.contains_key(&self.container)
    }

    /// Which client-side plate this window has below it, if any.
    fn plate(&self, frame: &PaneFrame<'_>) -> Option<Plate> {
        plate_of(self.container, backpack_of(frame.view), !self.ordinary(frame))
    }

    /// The icons this window draws, which is the view's list with the hand's
    /// two corrections applied.
    ///
    /// **A successful lift is not echoed back to the client that made it** —
    /// that client has already taken the icon out of its own gump — so the
    /// held item is subtracted here for as long as the hand holds it. And a
    /// drop this window is waiting on is added at the place it was dropped, so
    /// there is no gap between letting go and the shard's answer.
    fn contents(&self, frame: &PaneFrame<'_>) -> Vec<ContainedItem> {
        let held = frame.hand.map(|hand| hand.drag().item.serial);
        let mut contents: Vec<ContainedItem> = frame
            .view
            .contents
            .get(&self.container)
            .into_iter()
            .flatten()
            .filter(|item| Some(item.serial) != held)
            .copied()
            .collect();
        if let Some(Hand::Dropped {
            drag,
            destination: PendingDrop::Container { container, at },
        }) = frame.hand
        {
            if container == self.container {
                contents.push(ContainedItem { at, ..drag.item });
            }
        }
        contents
    }

    /// The two things written over this window: the plate's caption, and the
    /// name of the icon the pointer is resting on.
    ///
    /// Resolved here rather than in `client/render` because both need what
    /// that crate has never heard of — the atlas the plate's position comes
    /// out of, and the `tiledata` an item's name does. The pass draws what the
    /// layout produced, the way it draws a dialog's captions and a doll's name
    /// plate.
    fn lines(&self, frame: &PaneFrame<'_>, gump: Graphic, contents: &[ContainedItem]) -> Vec<Line> {
        let mut lines = Vec::new();
        if let Some(button) = self
            .plate(frame)
            .and_then(|plate| Some((plate, plate.button(&frame.resources.gump_atlas, gump, frame.at)?)))
        {
            let (plate, button) = button;
            lines.push(Line {
                at: button.label_at(),
                font: LABEL_FONT,
                hue: Hue::LABEL,
                clip: None,
                text: plate.label().to_owned(),
            });
        }
        if let Some(item) = self
            .hovered
            .and_then(|serial| contents.iter().find(|item| item.serial == serial))
        {
            lines.push(Line {
                at: frame.cursor.offset(HOVER_OFFSET),
                font: LABEL_FONT,
                hue: Hue::LABEL,
                clip: None,
                text: hover_text(item, &frame.resources.tiledata.static_tile(item.graphic.0).name),
            });
        }
        lines
    }

    /// Whether this was the second click on the same icon, and the bookkeeping
    /// either way: a completed pair arms nothing and a first click arms the
    /// next, so three quick clicks are a pair and a single.
    fn paired(&mut self, item: Serial, now: Instant) -> bool {
        let paired = pairs(self.last_click, now, item);
        self.last_click = (!paired).then_some((now, item));
        paired
    }

    /// A left press inside the window's own art.
    ///
    /// Three answers, and every one of them raises: a press on a window is
    /// what puts it on top. An icon either completes a pair — a use, and
    /// nothing is held — or starts a press this pane keeps. Anything else is
    /// the bag itself, which is the grip the window is moved by.
    fn press(&mut self, window: &Window, ctx: &PaneCtx<'_>) -> Response {
        let raised = Response::changed().with(Effect::Raise);
        let grab = || {
            Effect::Grab(GumpPixel::new(
                ctx.frame.cursor.x - ctx.frame.at.x,
                ctx.frame.cursor.y - ctx.frame.at.y,
            ))
        };
        if !self.ordinary(&ctx.frame) {
            return raised.with(grab());
        }
        let Some(item) = window.item_at(ctx.frame.cursor, &ctx.frame.resources.gump_atlas) else {
            return raised.with(grab());
        };
        if self.paired(item.serial, ctx.now) {
            let worn = RawLayer(ctx.frame.resources.tiledata.static_tile(item.graphic.0).layer);
            let mut answer = raised;
            answer.absorb(double_clicked(item, worn, ctx.frame.view.player.serial));
            return answer;
        }
        // Where inside the icon the player took hold of it, so that the item
        // keeps its place under the pointer once it is on the cursor — and so
        // that a drop lands where the icon looks like it is, rather than where
        // the pointer happens to be inside it.
        let icon = ctx.frame.at.offset(GumpPixel::new(item.at.x, item.at.y));
        self.pressed = Some(ItemPress {
            item,
            origin: DragOrigin::Container(self.container),
            at: ctx.frame.cursor,
            grab: GumpPixel::new(ctx.frame.cursor.x - icon.x, ctx.frame.cursor.y - icon.y),
        });
        raised
    }

    /// A left press on the plate below the window.
    ///
    /// **Reached when the pointer is on no window at all**, which is what a
    /// plate is: it hangs below the background art, so nothing of the window
    /// is under the cursor while it is pressed. That is also the whole of the
    /// covering rule the two old `*_button_under_pointer` walks were reaching
    /// for — they asked `window_under_pointer()` again inside their own loop —
    /// because the router stops a press at the window it landed on, so a plate
    /// hidden under another window is never offered this press at all.
    fn press_plate(&self, ctx: &PaneCtx<'_>) -> Response {
        let Some(plate) = self.plate(&ctx.frame) else {
            return Response::ignored();
        };
        let Some(gump) = ctx.frame.view.containers.get(&self.container).copied() else {
            return Response::ignored();
        };
        let on = plate
            .button(&ctx.frame.resources.gump_atlas, gump, ctx.frame.at)
            .is_some_and(|button| button.contains(ctx.frame.cursor));
        if !on {
            return Response::ignored();
        }
        let raised = Response::changed().with(Effect::Raise);
        match plate {
            Plate::StackAll => raised.with(Effect::StackAll),
            Plate::TakeAll => match backpack_of(ctx.frame.view) {
                Some(backpack) => take_all(&self.contents(&ctx.frame), backpack)
                    .into_iter()
                    .fold(raised, Response::with),
                // Nothing to sweep into. The plate is drawn — a body can lose
                // its pack — and pressing it asks for nothing.
                None => raised,
            },
        }
    }

    /// The release that ends whatever this window was in the middle of.
    ///
    /// Two things can be, and never both at once: the hand can be full over
    /// this bag, or this pane can be holding a press. The manager's own gate
    /// is what makes the pair impossible — a press never reaches a pane while
    /// the hand is full — which is step 2's argument with the case it was
    /// waiting for.
    fn release(&mut self, ctx: &PaneCtx<'_>) -> Response {
        if let (Some(hand), true) = (ctx.frame.hand, ctx.under_pointer) {
            // The gesture targets the *bag*, never an icon drawn in it: a
            // floor item released over a nested pack would otherwise enter
            // that pack, with its coordinates measured in the wrong window.
            // Merging two stacks is a destination gesture of its own and is
            // not this one.
            let grab = hand.drag().grab;
            let at = GumpPoint::new(
                ctx.frame.cursor.x - ctx.frame.at.x - grab.x,
                ctx.frame.cursor.y - ctx.frame.at.y - grab.y,
            );
            return Response::changed().with(Effect::Drop(PendingDrop::Container {
                container: self.container,
                at,
            }));
        }
        // The press is suspended under the amount prompt: letting the button
        // go is what *opened* it, and putting the press down here would leave
        // the answer with nothing to divide.
        if ctx.frame.has_prompt {
            return Response::consumed();
        }
        match self.pressed.take() {
            // A click that never became a drag. The double-click decision was
            // made on the press, so there is nothing left to do but let go —
            // and nothing new to draw, which the answer says.
            Some(_) => Response::consumed(),
            None => Response::ignored(),
        }
    }

    /// The move that turns a press into a lift, and keeps the tint honest.
    ///
    /// Both, in that order and not one or the other: the pointer that lifts an
    /// icon has also left it, so the tint it was wearing has to go with it.
    /// Neither is `taken` — a move is not an exclusive event — and the answer
    /// is [`Response::stale`] exactly when something on the screen changed.
    fn moved(&mut self, ctx: &PaneCtx<'_>) -> Response {
        let mut answer = self.hover(ctx);
        if ctx.frame.has_prompt {
            return answer;
        }
        let Some(press) = self.pressed else {
            return answer;
        };
        match press.dragged(ctx.frame.cursor, ctx.modifiers.shift) {
            Dragged::Still => answer,
            // The prompt goes up once and the press waits under it. `Still` is
            // what every following move answers, because `has_prompt` above is
            // true from here until the number arrives.
            Dragged::Ask(most) => {
                answer.absorb(Response::stale().with(Effect::Prompt(SplitPrompt { most })));
                answer
            }
            Dragged::Lift(drag) => {
                self.pressed = None;
                self.hovered = None;
                answer.absorb(Response::stale().with(Effect::Lift(drag)));
                answer
            }
        }
    }

    /// Follow the pointer with the icon the next frame will tint.
    ///
    /// Gated on [`PaneCtx::under_pointer`] so that a bag covered by another
    /// window does not light up through it — unlike the vendor's plate tint,
    /// which is not z-gated because its *layout* is not either. The rule is
    /// the same one stated both times: what the pane remembers and what the
    /// layout draws have to be computed by the same predicate.
    fn hover(&mut self, ctx: &PaneCtx<'_>) -> Response {
        let hovered = match (ctx.under_pointer, ctx.drawn) {
            (true, Some(Drawn::Container(window))) => window
                .item_at(ctx.frame.cursor, &ctx.frame.resources.gump_atlas)
                .map(|item| item.serial),
            _ => None,
        };
        if self.hovered == hovered {
            return Response::ignored();
        }
        self.hovered = hovered;
        Response::stale()
    }

    /// The amount prompt has been answered, and the press it suspended is
    /// this pane's.
    ///
    /// The split is finished here rather than left on the cursor, and that is
    /// deliberate: the lift path suppresses the `Remove` for the client that
    /// made it, so a part left hanging on the pointer after a modal can hide
    /// the old serial indefinitely. Dropping it back beside its remainder
    /// produces an `Add` for that serial, and asking for the container again
    /// replaces the whole list — including the remainder the shard has just
    /// made.
    fn answered(&mut self, answer: Answer) -> Response {
        let Some(press) = self.pressed.take() else {
            return Response::ignored();
        };
        let Answer::Split(amount) = answer else {
            // Dismissed, and the press goes with it: the player said no to the
            // only question this gesture was asking.
            return Response::changed();
        };
        let Some(drag) = press.split(amount) else {
            return Response::changed();
        };
        // Beside the pile it came out of, one icon down and along, so the two
        // halves do not sit on top of each other while the shard decides.
        let at = GumpPoint::new(press.item.at.x + SWEEP_STEP, press.item.at.y + SWEEP_STEP);
        Response::changed()
            .with(Effect::Lift(drag))
            .with(Effect::Drop(PendingDrop::Container {
                container: self.container,
                at,
            }))
            .with(Effect::Net(Outgoing::Use(self.container)))
    }
}

impl Pane for ContainerPane {
    /// The background and every icon in it.
    ///
    /// Asked of the view every frame rather than remembered, because what is
    /// in a bag changes without the window hearing about it — and the atlas
    /// answers a repeat with nothing, so asking for the same list twice costs
    /// a comparison.
    fn art(&self, frame: &PaneFrame<'_>) -> Vec<GumpArt> {
        match frame.view.containers.get(&self.container) {
            Some(gump) => container::art_of(*gump, &self.contents(frame)),
            // The window is open and the view has no `0x24` for it — one frame
            // between the packet that took the bag away and the reconcile that
            // will take the window with it.
            None => Vec::new(),
        }
    }

    fn layout(&self, frame: &PaneFrame<'_>) -> Option<Drawn> {
        let gump = *frame.view.containers.get(&self.container)?;
        let contents = self.contents(frame);
        let pictures = container::window_highlighted(gump, &contents, frame.at, self.hovered);
        let lines = self.lines(frame, gump, &contents);
        Some(Drawn::Container(Window {
            pictures,
            contents,
            lines,
        }))
    }

    /// A press on an icon or on the plate below, the release that finishes
    /// either gesture, the move that lifts, and the prompt's answer.
    ///
    /// The press is *located*, so which of the two it is depends on
    /// [`PaneCtx::under_pointer`]: on the window it is an icon or the bag, and
    /// off it, it can only be the plate. The release and the move are offered
    /// to every window — a press finishes wherever the pointer has got to, and
    /// a tint left behind on the bag the pointer just left has to be cleared.
    ///
    /// **No wheel.** Nothing in a bag scrolls: the shard sends coordinates and
    /// the window is exactly as big as its art, so a notch over one goes past
    /// it to the camera, as it always has.
    fn handle(&mut self, input: Input, ctx: &PaneCtx<'_>) -> Response {
        match input {
            Input::Press(Button::Left) => {
                if !ctx.under_pointer {
                    return self.press_plate(ctx);
                }
                // Never drawn, so there is nothing to hit-test against and no
                // pixels the player can have meant. The window is still raised
                // and moved by the manager's own path.
                let Some(Drawn::Container(window)) = ctx.drawn else {
                    return Response::ignored();
                };
                self.press(window, ctx)
            }
            Input::Release(Button::Left) => self.release(ctx),
            Input::Move => self.moved(ctx),
            Input::Answered(answer) => self.answered(answer),
            Input::Wheel(_) | Input::Press(Button::Right) | Input::Release(Button::Right) | Input::Key(_) => {
                Response::ignored()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use openshard_protocol::containers::GridSlot;
    use openshard_protocol::items::ItemAmount;

    use super::*;

    fn serial(raw: u32) -> Serial {
        Serial::new(raw).expect("an item serial")
    }

    fn item(raw: u32, graphic: Graphic, amount: u16) -> ContainedItem {
        ContainedItem {
            serial: serial(raw),
            graphic,
            amount: ItemAmount(amount),
            at: GumpPoint::new(0, 0),
            grid: GridSlot(0),
            hue: Hue::NONE,
        }
    }

    /// The pairing rule, minus the half the shape now answers: two clicks in
    /// two bags cannot pair, because each window keeps its own.
    #[test]
    fn two_clicks_on_one_icon_are_a_pair_and_two_icons_are_not() {
        let first = Instant::now();
        let soon = first + DOUBLE_CLICK / 2;
        let bottle = serial(0x4000_0001);

        assert!(
            !pairs(None, first, bottle),
            "the first click of all pairs with nothing"
        );
        assert!(pairs(Some((first, bottle)), soon, bottle));
        assert!(
            !pairs(Some((first, bottle)), soon, serial(0x4000_0002)),
            "the bottle beside it is not the same bottle"
        );
        assert!(
            !pairs(
                Some((first, bottle)),
                first + DOUBLE_CLICK + std::time::Duration::from_millis(1),
                bottle
            ),
            "and a click a breath too late is a first click"
        );
    }

    /// A completed pair disarms the state, so three quick clicks are a pair
    /// and a single rather than a pair and a half.
    #[test]
    fn a_completed_pair_starts_over() {
        let mut pane = ContainerPane::new(serial(0x4000_0000));
        let first = Instant::now();
        let step = DOUBLE_CLICK / 4;
        let bottle = serial(0x4000_0001);

        assert!(!pane.paired(bottle, first), "armed");
        assert!(pane.paired(bottle, first + step), "paired");
        assert!(
            !pane.paired(bottle, first + step * 2),
            "the third click starts a new pair rather than riding the old one"
        );
    }

    /// The ordinary double click asks the shard to use the item, and the one
    /// graphic this client wields itself asks for the two packets a drag onto
    /// the doll would have sent — as `Net`, so nothing is ever on the cursor.
    #[test]
    fn a_double_click_uses_the_icon_and_a_katana_is_worn() {
        let me = serial(0x0000_002A);
        let bottle = item(0x4000_0001, Graphic(0x0F0E), 1);
        let answer = double_clicked(bottle, RawLayer(0), me);
        assert!(matches!(
            answer.out.as_slice(),
            [Effect::Net(Outgoing::Use(used))] if *used == bottle.serial
        ));

        let katana = item(0x4000_0002, KATANA, 1);
        let answer = double_clicked(katana, RawLayer(1), me);
        assert!(
            matches!(
                answer.out.as_slice(),
                [
                    Effect::Net(Outgoing::PickUp { item, .. }),
                    Effect::Net(Outgoing::Equip { item: worn, layer, mobile })
                ] if *item == katana.serial && *worn == katana.serial && layer.0 == 1 && *mobile == me
            ),
            "the wield is a lift and an equip, and neither is the hand"
        );
    }

    /// A sweep is one lift and one drop per item, in the bag's own order and
    /// laid out six to a row so the icons do not land in one heap.
    #[test]
    fn a_sweep_names_every_item_twice_and_the_backpack_once() {
        let pack = serial(0x4000_0100);
        let contents: Vec<ContainedItem> = (0..7)
            .map(|index| item(0x4000_0001 + index, Graphic(0x0EED), 1))
            .collect();
        let effects = take_all(&contents, pack);
        assert_eq!(effects.len(), 14, "a lift and a drop each");

        let Effect::Net(Outgoing::DropInto { container, at, .. }) = &effects[1] else {
            panic!("the second effect of a pair is the drop");
        };
        assert_eq!(*container, pack);
        assert_eq!(*at, GumpPoint::new(20, 20), "the first lands at the origin");

        let Effect::Net(Outgoing::DropInto { at, .. }) = &effects[13] else {
            panic!("the seventh pair's drop");
        };
        assert_eq!(
            *at,
            GumpPoint::new(20, 38),
            "the seventh item starts the second row"
        );
    }

    /// An empty bag sweeps nothing, rather than asking for a lift of nothing.
    #[test]
    fn a_sweep_of_an_empty_bag_asks_for_nothing() {
        assert!(take_all(&[], serial(0x4000_0100)).is_empty());
    }

    /// The hover label names what is under the pointer and how many of it —
    /// the shard's own tooltip is a separate thing and arrives later.
    #[test]
    fn a_hover_names_the_item_and_its_amount() {
        let gold = item(0x4000_0001, Graphic(0x0EED), 137);
        assert_eq!(hover_text(&gold, "gold coins"), "gold coins (137)");
    }

    /// Which plate a window has, in the one place that decides it: the
    /// backpack compacts, every other bag is swept into it, and a shop's crate
    /// has neither.
    #[test]
    fn the_backpack_compacts_and_every_other_bag_is_swept() {
        let pack = serial(0x4000_0100);
        let chest = serial(0x4000_0200);

        assert_eq!(plate_of(pack, Some(pack), false), Some(Plate::StackAll));
        assert_eq!(plate_of(chest, Some(pack), false), Some(Plate::TakeAll));
        assert_eq!(
            plate_of(chest, None, false),
            Some(Plate::TakeAll),
            "a body with no pack is still offered the plate, and pressing it asks for nothing"
        );
        assert_eq!(
            plate_of(chest, Some(pack), true),
            None,
            "a shop's rows are bought, not swept"
        );
    }

    /// **What is clicked is what was drawn.** The list the pictures were built
    /// from travels with them, so an index into the pictures is a lookup and
    /// not a second walk — and picture zero is the bag itself, which is the
    /// window rather than anything in it.
    #[test]
    fn a_click_lands_on_the_icon_that_was_drawn_there() {
        use openshard_client_render::gump::{GumpArt, GumpAtlas};
        use openshard_uofiles::color::Color16;
        use openshard_uofiles::image::Image;

        const BAG: Graphic = Graphic(0x003C);
        const CANDLE: Graphic = Graphic(0x0A28);
        let block = |width: u16, height: u16| {
            Image::new(
                width,
                height,
                vec![Color16(0x7FFF); usize::from(width) * usize::from(height)],
            )
        };
        let atlas = GumpAtlas::pack([
            (GumpArt::Gump(BAG), block(140, 100)),
            (GumpArt::Item(CANDLE), block(10, 20)),
        ])
        .expect("two small blocks fit an atlas 2048 on a side");

        let contents = vec![
            ContainedItem {
                at: GumpPoint::new(20, 20),
                ..item(0x4000_0001, CANDLE, 1)
            },
            ContainedItem {
                at: GumpPoint::new(60, 20),
                ..item(0x4000_0002, CANDLE, 1)
            },
        ];
        let at = GumpPixel::new(100, 50);
        let window = Window {
            pictures: container::window_highlighted(BAG, &contents, at, None),
            contents: contents.clone(),
            lines: Vec::new(),
        };

        assert_eq!(
            window.item_at(GumpPixel::new(165, 75), &atlas),
            Some(contents[1]),
            "the second candle, at its own place in the bag"
        );
        assert_eq!(window.item_at(GumpPixel::new(125, 75), &atlas), Some(contents[0]));
        assert_eq!(
            window.item_at(GumpPixel::new(200, 130), &atlas),
            None,
            "the bag itself is the window, not something in it"
        );
        assert_eq!(
            window.item_at(GumpPixel::new(10, 10), &atlas),
            None,
            "and nothing at all outside the window"
        );
    }
}
