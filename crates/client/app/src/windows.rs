//! The player's own windows — containers, paperdolls, the skill window and
//! `0xB0` dialogs — and what the mouse is doing to them: [`Windows`].
//!
//! Every field here is about *this client's own* window layer rather than
//! about the world it draws over: what is open, where each sits, what the
//! last frame laid out for it and what a press on it is currently holding.
//! Pulled out of [`crate::App`] for the same reason [`crate::picking::Picking`]
//! and [`crate::input::Input`] were, and unlike those two the fields here
//! *are* read together — `dragging` and `held_doll` are checked side by side
//! on every press, `skills` and `held_skill` answer one gesture, and
//! `own_windows`/`dialogs`/`skills` are all asked in the same breath to
//! decide which window kind a click landed on.

use std::collections::HashSet;
use std::time::Instant;

use openshard_client_render::gump::{self as gump_art, GumpPixel};
use openshard_client_render::paperdoll;
use openshard_client_render::skills;
use openshard_protocol::containers::ContainedItem;
use openshard_protocol::gump::{GumpId, GumpPoint};
use openshard_protocol::serial::Serial;
use openshard_protocol::world::Point;

use crate::gump;

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

/// One of this client's own windows, and the one thing about it the shard
/// never says.
///
/// Neither packet carries a position: a `0x24` names a container and a gump,
/// a `0x88` names a mobile, and where the window goes is entirely the
/// client's — once the player has dragged one it is the player's. That is
/// the whole of this type. Everything else about the window is looked up in
/// the [`WorldView`](openshard_client_net::view::WorldView) by serial every
/// frame, so a window can never hold a stale copy of what is in the bag or
/// on the body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OwnWindow {
    /// What it is a window over.
    pub subject: WindowSubject,
    /// Its top-left corner on the surface.
    pub at: GumpPixel,
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
/// opinions about where every button is. See `crate::gump`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WindowSubject {
    /// A container the shard has opened, by its serial.
    Container(Serial),
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
    /// The one window kind whose *existence* is not in the view. A container
    /// window is open because the shard opened it and a dialog because the
    /// shard drew it, so `sync_own_windows` can read both off the view; this
    /// one is open because the player pressed Skills, and
    /// [`Windows::skills`] is where that fact lives.
    Skills,
    /// This character's status window. Like skills, its presence is local UI
    /// state: `0x11` updates its values but does not ask the client to open it.
    Status,
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
    /// A dialog: pictures, captions, hits and fields.
    Dialog(gump_art::Window),
    /// A container: the background and every icon in it.
    Container(Vec<gump_art::Picture>),
    /// A paperdoll: the frame, its furniture and the doll.
    Paperdoll(paperdoll::Doll),
    /// The skill window: the scroll, the rows inside its viewport, and the
    /// bar.
    Skills(skills::Sheet),
    /// The status frame and the numbers written over it.
    Status(openshard_client_render::status::Window),
}

/// A press on an item which becomes a drag only after the pointer actually
/// moves. Keeping it as an explicit state lets a normal click still
/// participate in the item's double-click "use" gesture.
#[derive(Clone, Copy, Debug)]
pub struct ItemPress {
    pub item: ContainedItem,
    /// The authoritative place the item is currently projected from.
    pub origin: DragOrigin,
    pub at: GumpPixel,
    pub grab: GumpPixel,
}

/// The source removed by a drag transaction. Rendering is a projection of the
/// authoritative view with this source subtracted until the server confirms a
/// destination or cancels the transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DragOrigin {
    Ground,
    Container(Serial),
}

/// A locally projected drop while the authoritative response is in flight.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PendingDrop {
    Container { container: Serial, at: GumpPoint },
    Ground(Point),
}

/// The item the client has asked the shard to put on its cursor.
#[derive(Clone, Copy, Debug)]
pub struct ItemDrag {
    pub item: ContainedItem,
    pub origin: DragOrigin,
    /// Offset from the item's top-left corner where the pointer grabbed it.
    pub grab: GumpPixel,
}

/// A single item transfer, including its local projection while the shard is
/// deciding it.  This is deliberately one state machine: an item cannot be
/// both pressed and held, nor can a completed drop still render at its source.
#[derive(Clone, Copy, Debug)]
pub enum ItemDragTransaction {
    Pressed(ItemPress),
    Held(ItemDrag),
    Dropped {
        drag: ItemDrag,
        destination: PendingDrop,
    },
}

impl ItemDragTransaction {
    pub fn drag(self) -> Option<ItemDrag> {
        match self {
            Self::Pressed(_) => None,
            Self::Held(drag) | Self::Dropped { drag, .. } => Some(drag),
        }
    }

    pub fn pending_drop(self) -> Option<PendingDrop> {
        match self {
            Self::Dropped { destination, .. } => Some(destination),
            Self::Pressed(_) | Self::Held(_) => None,
        }
    }
}

impl Drawn {
    /// What was drawn, in painter's order — the one question every window
    /// kind answers the same way.
    pub fn pictures(&self) -> &[gump_art::Picture] {
        match self {
            Self::Dialog(window) => &window.pictures,
            Self::Container(pictures) => pictures,
            Self::Paperdoll(doll) => &doll.pictures,
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
    /// insert here and send the [`crate::link::Command::CloseWindow`], the
    /// same instant, instead of mutating the view directly — that copy is
    /// never the source of truth, see `docs/client_window_state.md`'s D2.
    /// `sync_own_windows` treats a subject in this set as closed regardless
    /// of what the view still says, and drops the entry once a fresh
    /// `Update::World` agrees — the same reconciliation `Folded::corrected`
    /// runs for a mispredicted step, one layer down.
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
    /// The container item currently under the pointer, tinted on the next frame.
    pub hovered_container_item: Option<Serial>,
    /// The one local item-transfer transaction, from mouse press through
    /// authoritative confirmation or cancellation.
    pub item_drag: Option<ItemDragTransaction>,
    /// The first click of a potential double-click use inside a container.
    pub last_container_click: Option<(Instant, Serial)>,
    /// The paperdoll button the mouse went down on, and whose doll it is.
    ///
    /// [`gump::Dialogs::holding`]'s counterpart for the one window kind that
    /// is not a layout, and the same three things it buys: the pressed
    /// picture is drawn while the finger is down, the release acts only if
    /// the pointer is still on the *same* button, and a press on a button
    /// does not also drag the frame under it.
    ///
    /// Keyed by subject, not by picture index: the doll is laid out afresh
    /// every frame — a hat coming off changes how many pictures are in front
    /// of the buttons — so an index taken at the press names a different
    /// picture by the time the button comes up. The button itself is stable.
    pub held_doll: Option<(WindowSubject, paperdoll::DollButton)>,
    /// The last completed click on one of a paperdoll's three scrolls, and
    /// when.
    ///
    /// The scrolls answer a *double* click where the seven buttons answer a
    /// single one (`GumpPic.MouseDoubleClick` against `Button`'s
    /// `OnButtonClick`), and this is that pair. Separate from
    /// [`last_click`](crate::input::Input::last_click), which pairs clicks
    /// on the *world*: a click on a window never reaches that one, and a
    /// pair has to be two clicks on the same picture of the same window
    /// rather than two clicks anywhere.
    pub last_scroll: Option<(Instant, WindowSubject, paperdoll::DollButton)>,
    /// The skill window, when it is open: which headings are shut and how
    /// far down it is scrolled.
    ///
    /// `Some` *is* the window being open. Two facts in one field on purpose:
    /// every other window kind is open because the view holds its subject,
    /// and this one has no subject in the view to be open because of — so a
    /// separate `skills_open: bool` beside a `Tree` would be a second answer
    /// to the same question, able to say the window is shut while its
    /// scroll position stands.
    pub skills: Option<skills::Tree>,
    /// Whether the player's status window is open.
    ///
    /// A status reply refreshes numbers but does not open a window: the shard
    /// sends one at world entry, so only the Status button may set this true.
    pub status: bool,
    /// What the mouse went down on in the skill window, if anything.
    ///
    /// [`held_doll`](Windows::held_doll)'s twin, and keyed the same way — by
    /// what was pressed rather than by which picture, because the window is
    /// laid out afresh every frame and an index would name a different row
    /// by the time the button came up. A held [`skills::Hit::Thumb`] is also
    /// what makes the bar follow the pointer: see
    /// [`crate::App::drag_thumb`].
    pub held_skill: Option<skills::Hit>,
    /// What every open `0xB0` dialog is holding that no packet carries: the
    /// page it is showing, the switches the player has set, what has been
    /// typed into its fields and which button the finger is on. See
    /// [`crate::gump`].
    pub dialogs: gump::Dialogs,
}

/// [`crate::App::sync_own_windows`]'s membership logic, pulled out to a free
/// function so it can be exercised without an `App` — which needs real
/// client asset files to construct at all, the same reason `dst.rs` mirrors
/// `App`'s walk loop rather than driving the real thing in a test.
///
/// Opens a window for everything `view` has that `own_windows` does not, and
/// drops every window whose subject `view` (and, for the one kind it cannot
/// answer for, `skills_open`) no longer has — except a subject in
/// `locally_closed`, which stays dropped and stays un-reopened regardless of
/// what `view` says, until `view` itself agrees the subject is gone. That is
/// the reconciliation: an overlay entry survives only until the view it is
/// ahead of catches up, the same moment `Folded::corrected` would clear a
/// mispredicted step in `link.rs`, one layer down. A subject the view never
/// lists in the first place (`Skills`) has nothing to reconcile against and
/// is not put in the overlay at all.
pub fn reconcile_own_windows(
    view: &openshard_client_net::view::WorldView,
    own_windows: &mut Vec<OwnWindow>,
    locally_closed: &mut HashSet<WindowSubject>,
    skills_open: bool,
    status_open: bool,
) {
    locally_closed.retain(|subject| match *subject {
        WindowSubject::Container(serial) => view.containers.contains_key(&serial),
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
            WindowSubject::Container(serial) => view.containers.contains_key(&serial),
            WindowSubject::Paperdoll(serial) => view.paperdolls.contains_key(&serial),
            WindowSubject::Dialog(gump_id) => view.gumps.iter().any(|gump| gump.gump_id == gump_id),
            // The one kind the view cannot answer for — see the variant's
            // docs. Closing it is what empties `skills`, so this is that fact
            // read back rather than a second copy of it.
            WindowSubject::Skills => skills_open,
            WindowSubject::Status => status_open,
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
        .map(|serial| WindowSubject::Container(*serial))
        .chain(
            view.paperdolls
                .keys()
                .map(|serial| WindowSubject::Paperdoll(*serial)),
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
        });
    }
    // The skill window, which nothing in the view asked for: the player did,
    // by pressing Skills. Cascaded like a bag, for want of anywhere better —
    // the reference remembers where this one was left, which is the backlog
    // entry every window kind here shares.
    if skills_open
        && !own_windows
            .iter()
            .any(|window| window.subject == WindowSubject::Skills)
    {
        let step = own_windows.len() as i32 % CONTAINER_CASCADE_LENGTH;
        own_windows.push(OwnWindow {
            subject: WindowSubject::Skills,
            at: GumpPixel::new(
                CONTAINER_ORIGIN.x + CONTAINER_CASCADE.x * step,
                CONTAINER_ORIGIN.y + CONTAINER_CASCADE.y * step,
            ),
        });
    }
    // The status window has the skills window's ownership shape: the values
    // are authoritative, but the decision to look at them is local. A `0x11`
    // is sent at entry, so opening on data would surprise every login.
    if status_open
        && !own_windows
            .iter()
            .any(|window| window.subject == WindowSubject::Status)
    {
        let step = own_windows.len() as i32 % CONTAINER_CASCADE_LENGTH;
        own_windows.push(OwnWindow {
            subject: WindowSubject::Status,
            at: GumpPixel::new(
                CONTAINER_ORIGIN.x + CONTAINER_CASCADE.x * step,
                CONTAINER_ORIGIN.y + CONTAINER_CASCADE.y * step,
            ),
        });
    }
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
        own_windows.push(OwnWindow { subject, at });
    }
}
