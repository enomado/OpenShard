//! The player's own windows — containers, paperdolls, the skill window and
//! `0xB0` dialogs — and what the mouse is doing to them: [`Windows`].
//!
//! Every field here is about *this client's own* window layer rather than
//! about the world it draws over: what is open, where each sits, what the
//! last frame laid out for it, and which of the screen's one-of-a-kind
//! devices each window holds. Pulled out of [`crate::App`] for the same reason
//! [`crate::picking::Picking`] and [`crate::input::Input`] were, and unlike
//! those two the fields here *are* read together — `dragging` and `hand` are
//! checked side by side on every press, and `own_windows` and `drawn_windows`
//! are asked in the same breath to decide which window a click landed on.
//!
//! `docs/window_components.md` is finished with this module: every step of it
//! took something from here into the window it belonged to, and what is left
//! is what is true of the *layer* rather than of one window — which windows
//! exist, in what order, where each sits, and who holds the three things there
//! is one of: the pointer ([`Windows::dragging`]), the keyboard
//! ([`Windows::keyboard`]) and the cursor ([`Windows::hand`], and
//! [`Windows::prompt`] for the press a modal is standing over).
//!
//! The odd one out is [`Windows::world_press`], which is not about a window at
//! all: an item lying on the ground is pressed exactly the way an icon in a
//! bag is, and the world has no pane to keep that press in.

use std::collections::HashSet;
use std::time::Instant;

use openshard_client_render::gump::{self as gump_art, GumpPixel};
use openshard_client_render::skills;
use openshard_client_render::vendor;
use openshard_protocol::containers::ContainedItem;
use openshard_protocol::gump::{GumpId, GumpPoint};
use openshard_protocol::serial::Serial;
use openshard_protocol::world::Point;

/// Where the first container window opens, and how far each one after it is
/// offset.
///
/// A cascade rather than a pile: the shard sends no position, and two windows
/// at one coordinate look like one window with the wrong contents. The
/// reference client remembers a per-container position across sessions; this
/// does not yet, and the note is in `docs/client.md`.
const CONTAINER_CASCADE: GumpPixel = GumpPixel::new(24, 24);

/// The corner the cascade starts from.
const CONTAINER_ORIGIN: GumpPixel = GumpPixel::new(120, 80);

/// How many windows the cascade steps before it starts over, so that a player
/// who opens a dozen bags does not push the last of them off the screen.
const CONTAINER_CASCADE_LENGTH: i32 = 8;

/// One of this client's own windows: what it is over, where it is, and the
/// state that belongs to it alone.
///
/// Neither packet carries a position: a `0x24` names a container and a gump,
/// a `0x88` names a mobile, and where the window goes is entirely the
/// client's — once the player has dragged one it is the player's. What the
/// window is *over* is looked up in the
/// [`WorldView`](openshard_client_net::view::WorldView) by serial every frame,
/// so a window can never hold a stale copy of what is in the bag or on the
/// body.
///
/// [`pane`](OwnWindow::pane) is the third thing, and it is here rather than in
/// a map on [`Windows`] **so that a window's private state cannot outlive the
/// window**. A shop's scroll position used to be an entry in
/// `Windows::vendor_scrolls` that [`crate::App::close_window`] had to remember
/// to remove by hand; anything that lives here is dropped by the same `retain`
/// that takes the window off the list. See `docs/window_components.md`.
#[derive(Debug)]
pub struct OwnWindow {
    /// What it is a window over.
    pub subject: WindowSubject,
    /// Its top-left corner on the surface.
    pub at: GumpPixel,
    /// The window's own state and its own input handling — see
    /// [`crate::panes`].
    pub pane: crate::panes::AnyPane,
}

/// What a window is over: a bag's contents, a body, or a dialog the shard
/// drew.
///
/// One list holds all three, because dragging, raising, hit-testing and
/// closing are the same gesture over any of them — decision 5 in
/// `docs/client.md`, and the reason the container's window machinery was
/// written in this client's own gump pixels rather than as an egui window.
/// They differ in exactly two places, and each is a `match` three arms long:
/// what is laid out for it (see [`Windows::drawn_windows`], which is also
/// what the pointer is tested against), and what closing one means.
///
/// The dialog is the newest of the three and the one that had to *leave*
/// somewhere to get here: a `0xB0` was an egui window with the shard's art
/// drawn underneath it, which is two windows' worth of frame and two
/// opinions about where every button is. See [`crate::panes::dialog`], which
/// owns everything about one that no packet carries.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WindowSubject {
    /// A container the shard has opened, by its serial.
    Container(Serial),
    /// A vendor catalogue, whose controls are client-side rather than ordinary
    /// container dragging. It exists for both buy (`0x74`) and sell (`0x9E`).
    Vendor(Serial),
    /// A mobile whose paperdoll the shard has opened, by its serial. The same
    /// serial may name a container *and* a paperdoll — a player is both —
    /// which is why this is the identity and not the serial alone.
    Paperdoll(Serial),
    /// A `0xB0` dialog, by the id the shard filed it under.
    Dialog(GumpId),
    /// This character's skills. No key at all: a `0x3A` carries no serial, so
    /// there is one skill window and it is always about the body at this end
    /// of the connection — see `view::Player::skills`.
    ///
    /// One of the two window kinds whose *existence* is not in the view. A
    /// container window is open because the shard opened it and a dialog
    /// because the shard drew it, so `sync_own_windows` can read both off the
    /// view; this one is open because the player pressed Skills, and **being
    /// in [`Windows::own_windows`] is the whole of that fact** — see
    /// [`open_local_window`], which is the only thing that puts it there.
    ///
    /// It used to be `Windows::skills` being `Some`, which was the tree and
    /// the openness in one field: closing the window and forgetting which
    /// headings were shut were one write, and four files did it. The tree is
    /// a field of `panes::skills::SkillsPane` now.
    Skills,
    /// This character's status window. No key, for the skill window's reason:
    /// a `0x11` is about the one player this connection is.
    ///
    /// The other of the two kinds whose *existence* is not in the view, and it
    /// was the last field in the client that said "this window is open"
    /// anywhere but in [`Windows::own_windows`]. `Windows::status` was that
    /// `bool`, written by five places and read by a sixth; being in the list is
    /// the whole of the fact now, the same as every other kind's.
    Status,
}

impl WindowSubject {
    /// Whether this window exists because the player asked for it rather than
    /// because the shard opened it.
    ///
    /// The two kinds [`open_local_window`] puts in the list and
    /// [`reconcile_own_windows`] cannot answer for: there is no container, no
    /// mobile and no gump in the view to hold either of them open, so the view
    /// going away does not take them with it. Everything that has to drop
    /// *every* window when the world ends — the disconnect — asks this instead
    /// of naming the kinds, which is a list that would otherwise have to be
    /// kept in step by hand.
    pub const fn is_local(self) -> bool {
        matches!(self, Self::Skills | Self::Status)
    }
}

/// What the last frame drew for one window, and what it answers to.
///
/// Three shapes because the three window kinds answer different questions
/// about a click: a dialog's picture may be a button or a switch, a
/// paperdoll's may be one of the frame's own buttons, and a container's is an
/// item or the bag. What they have in common is the list of pictures, which
/// is what the pointer is tested against — see
/// [`crate::App::window_under_pointer`].
pub enum Drawn {
    /// A dialog: the pictures, hits and boxes, and the text resolved over them
    /// — see [`crate::panes::dialog::Window`], which is why this is not the
    /// render crate's `gump::Window` alone.
    Dialog(crate::panes::dialog::Window),
    /// A container: the background, every icon in it, and what the icons were
    /// — see [`crate::panes::container::Window`], which carries the list the
    /// pictures were built from so that a click maps to an item without a
    /// second walk that is free to disagree.
    Container(crate::panes::container::Window),
    Vendor(vendor::Window),
    /// A paperdoll: the frame, its furniture and the doll, and the text
    /// resolved over them — see [`crate::panes::paperdoll::Window`], which is
    /// why this is not the render crate's `paperdoll::Doll` alone.
    Paperdoll(crate::panes::paperdoll::Window),
    /// The skill window: the scroll, the rows inside its viewport, and the
    /// bar.
    Skills(skills::Sheet),
    /// The status frame and the numbers written over it.
    Status(openshard_client_render::status::Window),
}

/// A press on an item which becomes a drag only after the pointer actually
/// moves. Keeping it as an explicit state lets a normal click still
/// participate in the item's double-click "use" gesture.
///
/// **Held by whoever the press landed on**, which is decision 7's first half:
/// a press on an icon is `panes::container::ContainerPane`'s, a press on a worn
/// item is `panes::paperdoll::PaperdollPane`'s, and a press on the ground is
/// [`Windows::world_press`] because the world has no pane. Nothing has been
/// sent while one of these is alive — that is what makes it private to its
/// holder — and what it *becomes* is one rule for all three:
/// [`ItemPress::dragged`].
#[derive(Clone, Copy, Debug)]
pub struct ItemPress {
    pub item: ContainedItem,
    /// The authoritative place the item is currently projected from.
    pub origin: DragOrigin,
    pub at: GumpPixel,
    pub grab: GumpPixel,
}

/// How far the pointer may wander before a press stops being a click.
///
/// Three pixels, so that the hand shaking on a double click does not lift the
/// item out from under the second one.
const DRAG_SLOP: i32 = 3;

/// Where the pointer takes hold of an item that has no gump position of its
/// own.
///
/// A worn item and an item lying in the world are both drawn as something
/// other than their icon — a paperdoll layer, a ground sprite — so there is no
/// "this far into the picture" to remember. The icon's own centre goes under
/// the pointer instead, and that same offset is what a drop into a bag is
/// measured by, so the picture does not jump when it lands.
///
/// Zero for art this install does not ship, which draws the icon from its
/// corner: a missing graphic is not a reason to refuse the drag.
pub fn centre_of(graphic: openshard_protocol::wire::Graphic, art: &openshard_uofiles::art::Art) -> GumpPixel {
    art.static_art(graphic)
        .ok()
        .flatten()
        .map(|art| GumpPixel::new(i32::from(art.width()) / 2, i32::from(art.height()) / 2))
        .unwrap_or_default()
}

/// What a press becomes once the pointer has moved off it.
///
/// One rule with three holders — a bag's pane, a doll's pane, and the manager
/// for the world's own press — so the answer is a value they each act on rather
/// than three copies of the same `if`. Each turns it into what it can: a pane
/// into [`Effect`](crate::panes::Effect)s, the manager into its own writes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dragged {
    /// Not far enough yet. The press is still a click, and a second one within
    /// [`DOUBLE_CLICK`](crate::DOUBLE_CLICK) is still a use.
    Still,
    /// Shift, and a stack worth dividing: the client's own amount prompt goes
    /// up and the press waits for its answer. The number is the most that can
    /// be taken — the whole pile less the one that stays behind.
    Ask(u16),
    /// The lift itself: this is what goes onto the cursor.
    Lift(ItemDrag),
}

impl ItemPress {
    /// What this press has become, now that the pointer is at `cursor`.
    ///
    /// Shift is asked *after* the slop, in that order and not the other way
    /// round: a Shift-click that never moved is still a click, and putting the
    /// prompt in front of the slop would open one on every press of a stack.
    pub fn dragged(self, cursor: GumpPixel, shift: bool) -> Dragged {
        if (cursor.x - self.at.x).abs() <= DRAG_SLOP && (cursor.y - self.at.y).abs() <= DRAG_SLOP {
            return Dragged::Still;
        }
        if shift && self.item.amount.0 > 1 {
            return Dragged::Ask(self.item.amount.0 - 1);
        }
        Dragged::Lift(ItemDrag {
            item: self.item,
            origin: self.origin,
            grab: self.grab,
        })
    }

    /// The part of this press the player chose to take, or `None` for a stack
    /// that cannot be divided at all.
    ///
    /// Clamped into `1..=total - 1`: taking none of a pile is not a split and
    /// taking all of it is a lift, and the prompt's own bounds are the player's
    /// rather than a promise — the pile can have changed since it went up.
    pub fn split(self, amount: u16) -> Option<ItemDrag> {
        let total = self.item.amount.0;
        let amount = (total > 1).then(|| amount.clamp(1, total - 1))?;
        Some(ItemDrag {
            item: ContainedItem {
                amount: openshard_protocol::items::ItemAmount(amount),
                ..self.item
            },
            origin: self.origin,
            grab: self.grab,
        })
    }
}

/// The source removed by a drag transaction. Rendering is a projection of the
/// authoritative view with this source subtracted until the server confirms a
/// destination or cancels the transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DragOrigin {
    Ground,
    Container(Serial),
    Equipment {
        mobile: Serial,
        layer: openshard_protocol::wire::Layer,
    },
}

/// Where a held item has been put, and the projection that draws it there
/// while the authoritative answer is in flight.
///
/// It is also *what a pane asks for* — see
/// [`Effect::Drop`](crate::panes::Effect::Drop) — because the three places an
/// item can be put down are three packets and nothing else: a window says
/// where, and [`PendingDrop::packet`] is the only translation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PendingDrop {
    Container {
        container: Serial,
        at: GumpPoint,
    },
    Ground(Point),
    Equipment {
        mobile: Serial,
        layer: openshard_protocol::wire::Layer,
    },
}

impl PendingDrop {
    /// The packet that puts `item` here.
    ///
    /// One place, so that a fourth kind of destination is a compile error here
    /// rather than a `match` somebody forgot in the router. Equipping is a
    /// `0x13` and the other two are `0x08` with different coordinates — the
    /// wire's own distinction, not this client's.
    pub fn packet(self, item: Serial) -> openshard_client_net::action::Outgoing {
        use openshard_client_net::action::Outgoing;
        match self {
            Self::Container { container, at } => Outgoing::DropInto { item, container, at },
            Self::Ground(at) => Outgoing::DropOnGround { item, at },
            Self::Equipment { mobile, layer } => Outgoing::Equip {
                item,
                layer: openshard_protocol::wire::RawLayer(layer.0),
                mobile,
            },
        }
    }
}

/// The item the client has asked the shard to put on its cursor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ItemDrag {
    pub item: ContainedItem,
    pub origin: DragOrigin,
    /// Offset from the item's top-left corner where the pointer grabbed it.
    pub grab: GumpPixel,
}

/// What is on the cursor, and where it is on its way to.
///
/// **The client's mirror of the shard's own slot** — `Connection::held`, one
/// per connection, because a cursor holds one thing — which is the whole of
/// decision 7 in `docs/window_components.md`. The press that may *become* one
/// of these is not here: it belongs to whichever pane the press landed on (see
/// [`ItemPress`]), because nothing has been sent while a press is only a press,
/// and a lift is what puts the shard and this end into the same state.
///
/// Two states rather than three for that reason, and the pair is not a
/// gesture: [`Held`](Hand::Held) can outlive the window the item came out of,
/// survive a walk across the map, and be put down in a bag that was not open
/// when it was picked up.
#[derive(Clone, Copy, Debug)]
pub enum Hand {
    /// The shard has been asked for it and has not refused.
    Held(ItemDrag),
    /// It has been put somewhere and the answer is still in flight. The source
    /// stays subtracted and the destination is drawn until a packet settles it
    /// — see `App::apply_packet`.
    Dropped {
        drag: ItemDrag,
        destination: PendingDrop,
    },
}

/// Whose press the client's own modal is standing over.
///
/// A prompt suspends exactly one [`ItemPress`], and the answer has to find its
/// way back to whoever is holding it — by *identity*, never by "whichever
/// window is on top", because the player can raise a bag over the prompt while
/// it is up. That is decision 9 for a third exclusive device: the manager says
/// where the answer goes, the holder says what it means.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Asking {
    /// The press on a world item, which [`Windows::world_press`] holds.
    World,
    /// A press inside one of this client's own windows, which that window's
    /// pane holds. The answer reaches it as
    /// [`Input::Answered`](crate::panes::Input::Answered), routed the way a
    /// keystroke is.
    Window(WindowSubject),
}

/// A client-side multi-pass compaction of one open container.
#[derive(Clone, Debug)]
pub struct StackPass {
    pub container: Serial,
    /// Source serial of the last merge, until a full container refresh removes it.
    pub awaiting: Option<(Serial, Instant)>,
}

impl Hand {
    /// What is on the cursor. Not an `Option` any more, and that is the point:
    /// there used to be a third state in this enum that held nothing, so every
    /// reader had to ask twice — `owns_cursor` beside `drag` — and the answer
    /// to "is the hand full" was a method rather than the field being `Some`.
    pub const fn drag(self) -> ItemDrag {
        match self {
            Self::Held(drag) | Self::Dropped { drag, .. } => drag,
        }
    }

    /// Where it has been put, while the shard is still deciding.
    pub const fn pending_drop(self) -> Option<PendingDrop> {
        match self {
            Self::Dropped { destination, .. } => Some(destination),
            Self::Held(_) => None,
        }
    }
}

impl Drawn {
    /// What was drawn, in painter's order — the one question every window
    /// kind answers the same way.
    pub fn pictures(&self) -> &[gump_art::Picture] {
        match self {
            Self::Dialog(window) => &window.art.pictures,
            Self::Container(window) => &window.pictures,
            Self::Vendor(window) => &window.pictures,
            Self::Paperdoll(window) => &window.doll.pictures,
            Self::Skills(sheet) => &sheet.pictures,
            Self::Status(status) => &status.pictures,
        }
    }
}

/// The player's own window layer, and what the mouse is doing to it — see the
/// module docs.
pub struct Windows {
    /// The windows this client has open of its own — containers and
    /// paperdolls alike — bottom to top.
    ///
    /// Painter's order *is* z-order here, the same as the pictures inside
    /// one: the pass has no depth, so the last window in the list is the one
    /// drawn over the others and the first one picking finds. One list and
    /// not two, because a bag dragged over a paperdoll has to stay over it.
    pub own_windows: Vec<OwnWindow>,
    /// A window this end has closed, ahead of the shard thread's own
    /// [`view::WorldView`](openshard_client_net::view::WorldView) agreeing.
    ///
    /// [`crate::link::Body::predicted`]'s counterpart for a window's
    /// openness rather than a body's tile: `close_window`/`answer_gump`
    /// insert here, and [`reconcile_own_windows`] treats a subject in this
    /// set as closed regardless of what the view still says, dropping the
    /// entry once the view itself agrees the subject is gone — the same
    /// reconciliation `Folded::corrected` runs for a mispredicted step, one
    /// layer down. See `docs/client_window_state.md`'s D2.
    ///
    /// # There is no packet and no command behind this
    ///
    /// This used to say the two callers "send the `link::Command::CloseWindow`"
    /// as well. There is no such variant: it was S0's patch in that plan and
    /// S2 retired it in favour of this overlay. Nothing tells the link thread's
    /// own `WorldView` that a window closed, and nothing needs to — that copy
    /// is cloned across exactly once, in the `Update::World` published at world
    /// entry, and every packet after it is folded into *this* thread's copy
    /// instead. The comment outlived the mechanism by four months and named a
    /// symbol a reader could not find.
    pub locally_closed: HashSet<WindowSubject>,
    /// Every open window as the last frame laid it out: its subject, and the
    /// pictures that were drawn for it in painter's order.
    ///
    /// **What is clicked is what was drawn**, which is why this is
    /// remembered rather than recomputed at the press. A paperdoll's layout
    /// is not a function of the window alone — it reads the view, the
    /// tiledata and the client's own `gumpart` to decide which picture a
    /// worn item is — and a second walk asking those questions again is a
    /// second answer waiting to disagree with the one on the screen. It is
    /// the same rule [`crate::items::place`] follows in the world, one layer
    /// up.
    ///
    /// A frame behind, therefore: a window that has just opened is not
    /// pickable until it has been drawn once, which is also the frame its
    /// art is packed on and so the frame it first has any pixels to be
    /// picked by.
    pub drawn_windows: Vec<(WindowSubject, Drawn)>,
    /// The window being dragged and where inside it the player grabbed it, or
    /// `None` when nothing is being dragged.
    ///
    /// Keyed by subject rather than by index: raising a window on the press
    /// reorders the list, so an index taken at the press names a different
    /// window by the time the mouse moves.
    pub dragging: Option<(WindowSubject, GumpPixel)>,
    /// What is on the cursor, or `None` for an empty hand.
    ///
    /// **One resource with one owner**, decision 7 — the mirror of the shard's
    /// own one-item slot. A pane reads it out of
    /// [`PaneFrame::hand`](crate::panes::PaneFrame::hand) to answer "was
    /// something dropped on me" and to draw a preview, and no pane fills or
    /// empties it: both halves of a transfer are asked for as
    /// [`Effect::Lift`](crate::panes::Effect::Lift) and
    /// [`Effect::Drop`](crate::panes::Effect::Drop), which this end performs.
    ///
    /// It used to be `item_drag`, an `ItemDragTransaction` whose first state
    /// was a press that had sent nothing. That state is a pane's now — see
    /// [`ItemPress`] — which is why the field is called what it is: what is
    /// left here is the hand.
    pub hand: Option<Hand>,
    /// The press on an item lying in the *world*, which no pane holds because
    /// the ground is not a window.
    ///
    /// The manager's copy of what a bag's pane and a doll's pane each keep for
    /// their own icons — one press, three possible holders, one rule for what
    /// it becomes ([`ItemPress::dragged`]). It is here rather than beside the
    /// picking state because the hand it turns into is here.
    pub world_press: Option<ItemPress>,
    /// Who the client's own amount prompt is addressed to, or `None` while no
    /// prompt is up.
    ///
    /// The keyboard's shape one modal over (see [`Windows::keyboard`]): the
    /// manager owns *which* press a modal's answer belongs to, because a player
    /// can raise another window while the prompt stands, and the holder of that
    /// press owns what the answer means. Without it the answer would go to
    /// whoever happened to be on top — the plan's Backlog entry about who a
    /// modal's answer is addressed to, settled the same way
    /// [`Input::Key`](crate::panes::Input::Key) was.
    ///
    /// It replaces `split_pending`, which was a `bool` on a client that could
    /// only ever have one presser.
    pub prompt: Option<Asking>,
    /// An automatic sequence of ordinary lift/drop requests, one per fresh snapshot.
    pub stack_pass: Option<StackPass>,
    /// Which window the keys are going to, or `None` for the world.
    ///
    /// **One resource with one owner**, the shape decision 7 gives the hand and
    /// decision 2 gives z-order: there is one keyboard, so no pane can be
    /// trusted with the question and no pane can see another's answer. What a
    /// window does with the keys once they arrive is its own —
    /// [`DialogPane`](crate::panes::dialog) is the only kind that has anywhere
    /// to put them, and which of *its* boxes is a field of the pane.
    ///
    /// It replaces `Dialogs::focus`, which was the window *and* the field in one
    /// tuple on a struct that held every dialog's state at once. Read through
    /// [`App::keyboard_window`](crate::app::App::keyboard_window) rather than
    /// directly, because a window can leave the list between the answer that
    /// took it down and the frame that tidies up.
    pub keyboard: Option<WindowSubject>,
}

/// Open one of the windows the shard does not know about, if it is not open
/// already.
///
/// The skill sheet and the status frame: nothing in the view asks for either,
/// so nothing in [`reconcile_own_windows`] can put them there — the player
/// pressed a button on their paperdoll, and this is that press arriving. See
/// [`crate::panes::LocalWindow`], which is the effect a pane asks for.
///
/// **Idempotent, and that is the contract**: pressing Skills a second time
/// while the sheet is up must leave the window it finds alone, scroll position,
/// shut headings and all. A window is its pane, so re-opening one would be
/// throwing that away — which is exactly what the old
/// `skills.get_or_insert_with(Tree::default)` was careful not to do, in two
/// places that each had to remember.
///
/// Cascaded like a bag, for want of anywhere better: the reference client
/// remembers where each window was left, which is the backlog entry every kind
/// here shares.
pub fn open_local_window(own_windows: &mut Vec<OwnWindow>, subject: WindowSubject) {
    if own_windows.iter().any(|window| window.subject == subject) {
        return;
    }
    let step = own_windows.len() as i32 % CONTAINER_CASCADE_LENGTH;
    own_windows.push(OwnWindow {
        subject,
        at: GumpPixel::new(
            CONTAINER_ORIGIN.x + CONTAINER_CASCADE.x * step,
            CONTAINER_ORIGIN.y + CONTAINER_CASCADE.y * step,
        ),
        pane: crate::panes::AnyPane::of(subject),
    });
}

/// [`crate::App::sync_own_windows`]'s membership logic, pulled out to a free
/// function so it can be exercised without an `App` — which needs real
/// client asset files to construct at all, the same reason `dst.rs` mirrors
/// `App`'s walk loop rather than driving the real thing in a test.
///
/// Opens a window for everything `view` has that `own_windows` does not, and
/// drops every window whose subject `view` no longer has — except a subject in
/// `locally_closed`, which stays dropped and stays un-reopened regardless of
/// what `view` says, until `view` itself agrees the subject is gone. That is
/// the reconciliation: an overlay entry survives only until the view it is
/// ahead of catches up, the same moment `Folded::corrected` would clear a
/// mispredicted step in `link.rs`, one layer down. A subject the view never
/// lists in the first place — the two [`WindowSubject::is_local`] kinds — has
/// nothing to reconcile against and is not put in the overlay at all.
///
/// **Neither local kind is passed in any more, in either direction.** Both are
/// opened by [`open_local_window`] and closed by the `retain` in
/// `App::close_window`, so being in `own_windows` *is* the fact and there is no
/// second copy of it here to disagree with. This function took a `status_open`
/// argument until step 3 of `docs/window_components.md`, and a `skills_open`
/// beside it until step 2; what they were is a window's openness kept somewhere
/// other than the list of open windows.
pub fn reconcile_own_windows(
    view: &openshard_client_net::view::WorldView,
    own_windows: &mut Vec<OwnWindow>,
    locally_closed: &mut HashSet<WindowSubject>,
) {
    locally_closed.retain(|subject| match *subject {
        WindowSubject::Container(serial) => view.containers.contains_key(&serial),
        WindowSubject::Vendor(serial) => {
            view.vendor_buys.contains_key(&serial) || view.vendor_sells.contains_key(&serial)
        }
        WindowSubject::Paperdoll(serial) => view.paperdolls.contains_key(&serial),
        WindowSubject::Dialog(gump_id) => view.gumps.iter().any(|gump| gump.gump_id == gump_id),
        WindowSubject::Skills => false,
        WindowSubject::Status => false,
    });
    own_windows.retain(|window| {
        if locally_closed.contains(&window.subject) {
            return false;
        }
        match window.subject {
            WindowSubject::Container(serial) => {
                view.containers.contains_key(&serial) && !view.vendor_buys.contains_key(&serial)
            }
            WindowSubject::Vendor(serial) => {
                view.vendor_buys.contains_key(&serial) || view.vendor_sells.contains_key(&serial)
            }
            WindowSubject::Paperdoll(serial) => view.paperdolls.contains_key(&serial),
            WindowSubject::Dialog(gump_id) => view.gumps.iter().any(|gump| gump.gump_id == gump_id),
            // Nothing to reconcile against, and nothing to ask: the window is
            // open because it is here. `close_window`'s own `retain` is what
            // takes it away, and anything here would be a second opinion about
            // that — the two fields this replaced could each say the window was
            // shut while the window was still in this list.
            WindowSubject::Skills | WindowSubject::Status => true,
        }
    });
    // Containers first and paperdolls after, and both in the view's own
    // iteration order — which is a `HashMap`'s and therefore not stable. That
    // decides only where two windows opened on the *same frame* cascade to,
    // and nothing else: a window's position is its own from the moment it is
    // placed.
    let wanted = view
        .containers
        .keys()
        .filter(|serial| !view.vendor_buys.contains_key(serial))
        .map(|serial| WindowSubject::Container(*serial))
        .chain(
            view.paperdolls
                .keys()
                .map(|serial| WindowSubject::Paperdoll(*serial)),
        );
    let wanted = wanted.chain(
        view.vendor_buys
            .keys()
            .chain(view.vendor_sells.keys())
            .map(|serial| WindowSubject::Vendor(*serial)),
    );
    for subject in wanted.collect::<Vec<_>>() {
        if own_windows.iter().any(|window| window.subject == subject) {
            continue;
        }
        // Still overlaid: the view has not caught up with the close yet, and
        // re-opening it here is exactly the reopen this overlay exists to
        // stop.
        if locally_closed.contains(&subject) {
            continue;
        }
        let step = own_windows.len() as i32 % CONTAINER_CASCADE_LENGTH;
        own_windows.push(OwnWindow {
            subject,
            at: GumpPixel::new(
                CONTAINER_ORIGIN.x + CONTAINER_CASCADE.x * step,
                CONTAINER_ORIGIN.y + CONTAINER_CASCADE.y * step,
            ),
            pane: crate::panes::AnyPane::of(subject),
        });
    }
    // No arm for either local kind here, and that is step 3's whole shape: the
    // values on a status frame are authoritative, but the decision to look at
    // them is the player's, and a `0x11` arrives at every login — so a window
    // opened from the data would be a window nobody asked for. What opens one
    // is `open_local_window`, called from the button that was pressed.
    //
    // A dialog is placed where the shard asked for it, and it is the only
    // window kind that is: a `0xB0` carries a coordinate and a `0x24` does
    // not. So no cascade — two dialogs the shard put in one place are two
    // dialogs the shard put in one place, and moving them would be this
    // client second-guessing a layout it was handed.
    let dialogs: Vec<(GumpId, GumpPixel)> = view
        .gumps
        .iter()
        .map(|gump| (gump.gump_id, GumpPixel::new(gump.at.x, gump.at.y)))
        .collect();
    for (gump_id, at) in dialogs {
        let subject = WindowSubject::Dialog(gump_id);
        if own_windows.iter().any(|window| window.subject == subject) {
            continue;
        }
        // Overlaid the same as a container or paperdoll: `answer_gump` sets
        // this before the view has forgotten the dialog, and the view is
        // what is stale here — see `App::answer_gump`.
        if locally_closed.contains(&subject) {
            continue;
        }
        own_windows.push(OwnWindow {
            subject,
            at,
            pane: crate::panes::AnyPane::of(subject),
        });
    }
}

#[cfg(test)]
mod tests {
    use openshard_protocol::containers::GridSlot;
    use openshard_protocol::items::ItemAmount;
    use openshard_protocol::wire::{Graphic, Hue};

    use super::*;

    fn press(amount: u16) -> ItemPress {
        ItemPress {
            item: ContainedItem {
                serial: Serial::new(0x4000_0001).expect("an item serial"),
                graphic: Graphic(0x0EED),
                amount: ItemAmount(amount),
                at: GumpPoint::new(0, 0),
                grid: GridSlot(0),
                hue: Hue::NONE,
            },
            origin: DragOrigin::Container(Serial::new(0x4000_0100).expect("a bag serial")),
            at: GumpPixel::new(100, 100),
            grab: GumpPixel::new(4, 4),
        }
    }

    /// The slop is what keeps a double click from lifting the item out from
    /// under its own second half: a hand that shakes three pixels is still
    /// clicking.
    #[test]
    fn a_press_becomes_a_lift_only_once_the_pointer_has_really_moved() {
        let press = press(1);
        assert_eq!(press.dragged(GumpPixel::new(103, 97), false), Dragged::Still);
        assert!(matches!(
            press.dragged(GumpPixel::new(104, 100), false),
            Dragged::Lift(_)
        ));
        assert!(matches!(
            press.dragged(GumpPixel::new(100, 96), false),
            Dragged::Lift(_)
        ));
    }

    /// Shift divides a pile and nothing else: a single item has nothing to
    /// divide, and a Shift-press that never moved is still a click.
    #[test]
    fn shift_asks_for_an_amount_only_when_there_is_a_pile_to_divide() {
        assert_eq!(
            press(20).dragged(GumpPixel::new(200, 200), true),
            Dragged::Ask(19),
            "the most that can be taken is the pile less the one left behind"
        );
        assert!(matches!(
            press(1).dragged(GumpPixel::new(200, 200), true),
            Dragged::Lift(_)
        ));
        assert_eq!(
            press(20).dragged(GumpPixel::new(101, 101), true),
            Dragged::Still,
            "the slop is asked first, so a Shift-click is a click"
        );
    }

    /// Taking none of a pile is not a split and taking all of it is a lift, so
    /// the answer is clamped between them — and a single item cannot be
    /// divided at all.
    #[test]
    fn a_split_never_takes_none_or_the_whole_stack() {
        let amount = |press: &ItemPress, want| press.split(want).map(|drag| drag.item.amount.0);
        let pile = press(10);
        assert_eq!(amount(&pile, 0), Some(1));
        assert_eq!(amount(&pile, 4), Some(4));
        assert_eq!(amount(&pile, 10), Some(9));
        assert_eq!(amount(&press(1), 1), None);
    }

    /// A split keeps everything about the press but the number: the same
    /// serial, the same source and the same grip.
    #[test]
    fn a_split_carries_its_press_forward() {
        let pile = press(10);
        let drag = pile.split(3).expect("a pile of ten divides");
        assert_eq!(drag.item.serial, pile.item.serial);
        assert_eq!(drag.origin, pile.origin);
        assert_eq!(drag.grab, pile.grab);
    }

    /// Where an item is put down decides which packet says so, and there is
    /// one place that decides it.
    #[test]
    fn a_destination_names_its_own_packet() {
        use openshard_client_net::action::Outgoing;
        let item = Serial::new(0x4000_0001).expect("an item serial");
        let bag = Serial::new(0x4000_0100).expect("a bag serial");
        let me = Serial::new(0x0000_002A).expect("a mobile serial");

        assert!(matches!(
            PendingDrop::Container {
                container: bag,
                at: GumpPoint::new(7, 9),
            }
            .packet(item),
            Outgoing::DropInto { container, .. } if container == bag
        ));
        assert!(matches!(
            PendingDrop::Ground(Point::new(1, 2, 3)).packet(item),
            Outgoing::DropOnGround { .. }
        ));
        assert!(matches!(
            PendingDrop::Equipment {
                mobile: me,
                layer: openshard_protocol::wire::Layer(5),
            }
            .packet(item),
            Outgoing::Equip { mobile, layer, .. } if mobile == me && layer.0 == 5
        ));
    }
}
