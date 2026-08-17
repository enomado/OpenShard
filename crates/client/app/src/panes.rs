//! The client's own windows as components: one type per kind, each owning the
//! state that belongs to that window alone and answering the input that lands
//! on it.
//!
//! `docs/window_components.md` is the plan this implements and its decisions
//! are the vocabulary here. A pane **takes readonly context in ([`PaneCtx`])
//! and hands mutations out ([`Effect`])**: it never holds an `&mut App`, never
//! reaches the shard, and never decides where on the screen it sits. Which
//! windows exist, in what order, and at what coordinate stays with the manager
//! — see [`crate::windows`] and decisions 2 and 8.
//!
//! # What is here, and what is not yet
//!
//! The vocabulary and the router are complete; the panes are not. Every
//! [`AnyPane`] variant declines every input, and the six kinds' input is still
//! answered by the `App` methods in [`crate::own_windows`], which
//! [`crate::app::App::deliver`] calls once every pane has passed. That is the
//! plan's own migration order — step 1 moves the vendor in, and until a kind
//! has moved its window behaves exactly as it did before this module existed.
//!
//! [`Pane`] therefore has only its input half. `art` and `layout` (decision 6)
//! join the trait with the first pane that has a layout of its own: a pane with
//! no state cannot lay anything out, and `render_passes.rs` still lays out all
//! six kinds from the view.
//!
//! # Two names that differ from the plan's
//!
//! The plan writes `trait Pane` and `enum Pane` in the same breath, which
//! cannot both exist. The trait keeps the name — it is the concept — and the
//! static-dispatch enum over its implementors is [`AnyPane`], the ordinary Rust
//! spelling of that. The plan's `Panes::deliver` is [`crate::app::App::deliver`]
//! for as long as the router still has to reach the legacy handlers, which need
//! the whole `App`; the loop itself already takes nothing but the pane list.

use std::time::Instant;

use openshard_client_net::action::Outgoing;
use openshard_client_net::view::WorldView;
use openshard_client_render::gump::{GumpArt, GumpPixel};

use crate::resources::Resources;
use crate::windows::{Drawn, ItemDrag, WindowSubject};

mod route;
mod vendor;

/// Which mouse button an input is about.
///
/// No middle button: it pans the camera (`event_loop.rs`'s `MouseInput` arm)
/// and no window kind has ever wanted it, so a pane cannot be offered one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    Left,
    Right,
}

/// One input, as a pane is offered it.
///
/// Named `Input` after the plan, and always reached as `panes::Input`: the
/// crate already has an [`Input`](crate::input::Input) which is the *state* of
/// the pointer and the modifier keys rather than one event out of it. This is
/// the event; that is what the event is measured against.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Input {
    /// A button went down at [`PaneCtx::cursor`].
    Press(Button),
    /// And came back up. Where it came up is [`PaneCtx::cursor`] again, which
    /// is not where it went down — every "still on the same picture" rule in
    /// the window layer is that difference.
    Release(Button),
    /// The pointer moved to [`PaneCtx::cursor`].
    ///
    /// **Not an exclusive event.** A move is offered to panes so that a hover
    /// tint can go stale and a pane-private gesture can follow the pointer, but
    /// the manager's own gestures — the window being dragged, the item on its
    /// way out of a bag — run whether a pane took it or not, and the camera
    /// reads it too. `taken` on a move therefore says only "the world should
    /// not also act on this", and nothing in the client asks that yet.
    Move,
    /// A wheel notch, signed: positive is away from the player.
    ///
    /// A line on a wheel and a fraction of one on a touchpad — only the sign is
    /// meaningful, and how far a notch goes is the pane's own business.
    Wheel(f32),
}

/// The modifier keys, as a pane sees them.
///
/// Two of them, because two of them mean something to a window: Shift splits a
/// stack being dragged out of a bag, and Ctrl is a move order rather than a
/// heading. Passed in rather than read off `App::input`, for the same reason
/// [`PaneCtx::now`] is passed in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
}

/// What a pane may read while it is being packed and laid out.
///
/// Decision 3's context, minus the four things that only mean something while
/// an *input* is being answered — see [`PaneCtx`], which is this plus those.
/// Split in two because the two callers are two different places: a frame is
/// laid out from `render_passes::draw_gump_windows`, which has the view, the
/// files and the pointer and has no business inventing a clock, a modifier
/// state or a z-order answer to put in a field a layout never reads.
///
/// No `&mut App`, no [`Link`](crate::link::Link), no `&mut WorldView`: a pane
/// that could reach any of the three would be able to answer a click by
/// changing the world, and the whole point of [`Effect`] is that the answer is
/// *returned* instead — where the manager can order it, log it, refuse it, or a
/// test can read it.
pub struct PaneFrame<'a> {
    /// The authoritative picture. A pane holds no copy of what is in the bag or
    /// on the body; it looks both up here every time it is asked, which is why
    /// a window can never draw or click a stale item.
    pub view: &'a WorldView,
    /// The client's own files: art, tiledata, the gump atlas, skill names.
    pub resources: &'a Resources,
    /// Where the manager has put this window, in absolute gump pixels.
    ///
    /// A pane **reads this and never writes it** — the cascade, the raise, the
    /// drag that moves a window and the close gesture are all the manager's,
    /// and a pane that wants to be moved declines the press instead. It is
    /// absolute rather than window-local for now (decision 2), so a pane
    /// hit-tests by subtracting this from [`PaneFrame::cursor`].
    pub at: GumpPixel,
    /// The pointer, in the same gump pixels [`PaneFrame::at`] is in.
    pub cursor: GumpPixel,
    /// What is on the cursor, if anything.
    ///
    /// Readonly, and the whole of decision 7: the hand is a **slot** and not a
    /// gesture — one per client because the shard has one per connection — so a
    /// pane reads it to answer "was something dropped on me" and to draw a
    /// wearable's preview, and no pane may fill or empty it. The two halves of
    /// a transfer are two ordinary [`Effect::Net`]s, possibly from two
    /// different panes, possibly with a walk between them.
    #[expect(
        dead_code,
        reason = "the container reads it for a drop onto itself at step 6, the paperdoll for a preview at step 5"
    )]
    pub hand: Option<ItemDrag>,
}

/// Everything a pane may read while it answers one input, and nothing it may
/// write.
///
/// [`PaneFrame`] and the four things that are true of an *event* rather than of
/// a frame.
pub struct PaneCtx<'a> {
    /// What this pane would be packed and laid out with, were the frame being
    /// drawn now.
    pub frame: PaneFrame<'a>,
    /// How this window was laid out on the last frame, or `None` for a window
    /// that has not been drawn yet.
    ///
    /// **What is clicked is what was drawn.** A pane could lay itself out again
    /// from [`PaneCtx::frame`] and hit-test that, and it would be asking the
    /// atlas and the view a second question whose answer is free to differ from
    /// the picture the player is pointing at — the rule
    /// [`Windows::drawn_windows`](crate::windows::Windows::drawn_windows)
    /// exists for, and the one that used to make a paperdoll whose two answers
    /// disagreed a window that could not be closed.
    pub drawn: Option<&'a Drawn>,
    /// This window is the one the pointer is on: it covers the cursor and no
    /// window above it does.
    ///
    /// The manager's answer and not the pane's, because **z-order is the
    /// manager's** (decision 2) — a pane knows where the cursor is inside
    /// itself and cannot know what is drawn over it. It is the same question
    /// `App::window_under_pointer` has always asked first, handed to the pane
    /// instead of asked again.
    ///
    /// A pane checks it before taking a *located* input — a press or a notch —
    /// and ignores it for the two that are not located that way: a release
    /// finishes a press this pane started, wherever the pointer has got to
    /// since, and a move is offered to every window.
    pub under_pointer: bool,
    /// Shift and Ctrl, as of this event.
    #[expect(
        dead_code,
        reason = "the container's Shift-split reads it at step 6; nothing else has a modifier"
    )]
    pub modifiers: Modifiers,
    /// Now, for every double-click pair.
    ///
    /// Passed rather than read from `Instant::now()` inside a pane, for the
    /// reason the tick's `Rng` is owned rather than ambient: a pair of clicks
    /// is a *timing* rule, and a rule that reads an ambient clock cannot be
    /// exercised by a test.
    #[expect(
        dead_code,
        reason = "the container's double-click use reads it at step 6, the paperdoll's scrolls at step 5"
    )]
    pub now: Instant,
}

/// What a pane says about one input.
///
/// Decision 4, and the wheel defect stated as a type instead of as a
/// convention: `taken` and `redraw` are **two fields because they are two
/// questions**, and every one of the four combinations happens. A catalogue
/// scrolled to its last row took the notch and has nothing new to draw
/// ([`Response::consumed`]); a hover tint that changed took nothing and is
/// stale ([`Response::stale`]). The `bool` these replace could say neither, and
/// the `||` chain in `event_loop.rs` read it as whichever of the two the last
/// author had in mind — which is how a wheel over a shop window became a map
/// zoom.
#[derive(Debug, Default)]
#[must_use]
pub struct Response {
    /// The event stops here: neither the camera, nor the world, nor a window
    /// under this one ever sees it.
    pub taken: bool,
    /// The frame is stale and has to be drawn again.
    pub redraw: bool,
    /// What the pane is asking the manager to do, in order.
    pub out: Vec<Effect>,
}

impl Response {
    /// Not mine, and nothing changed. The pointer is somewhere else, or this is
    /// an input this kind of window has no use for.
    pub const fn ignored() -> Self {
        Self {
            taken: false,
            redraw: false,
            out: Vec::new(),
        }
    }

    /// Mine, and the frame still stands. The end-of-list wheel notch.
    pub const fn consumed() -> Self {
        Self {
            taken: true,
            redraw: false,
            out: Vec::new(),
        }
    }

    /// Mine, and it changed what is on the screen. The ordinary click.
    pub const fn changed() -> Self {
        Self {
            taken: true,
            redraw: true,
            out: Vec::new(),
        }
    }

    /// Not mine, but it changed what is on the screen. A hover tint.
    pub const fn stale() -> Self {
        Self {
            taken: false,
            redraw: true,
            out: Vec::new(),
        }
    }

    /// Ask for one thing, on the way out.
    pub fn with(mut self, effect: Effect) -> Self {
        self.out.push(effect);
        self
    }

    /// Fold another answer into this one: either says stale, either says taken,
    /// and the effects keep their order.
    ///
    /// For the two places one input has more than one answer — the manager's
    /// own gestures beside a pane's, and the legacy handlers beside both.
    pub fn absorb(&mut self, other: Self) {
        self.taken |= other.taken;
        self.redraw |= other.redraw;
        self.out.extend(other.out);
    }
}

/// One thing a pane asks the manager to do.
///
/// Decision 5. Most of what a window does is ask the shard for something, and
/// [`Outgoing`] is already the complete vocabulary of that — so a pane returns
/// [`Effect::Net`] and never touches the link. A second enum meaning the same
/// things would be a second place to add a packet to, and translating between
/// them would be a `match` whose arms are all identities.
///
/// The rest is what only the manager can do, because it is about the window
/// rather than about what the window is over.
///
/// Every arm is performed by `panes::route`'s `App::perform`.
#[derive(Debug)]
pub enum Effect {
    /// This window to the top of the pile.
    Raise,
    /// This window off the list. What that means per kind — an overlay entry, a
    /// `0xB1` for a dialog, nothing at all on the wire for a bag — stays with
    /// the manager: see `App::close_window`.
    Close,
    /// Start moving this window, grabbed this far into it.
    Grab(GumpPixel),
    /// The shard's half. Both halves of a transfer are this, separately: see
    /// decision 7 for why there is no lift/drop pair.
    Net(Outgoing),
    /// Make one of the two windows the shard does not know about exist.
    #[expect(
        dead_code,
        reason = "performed by App::perform; the paperdoll's Skills button asks for one at step 5"
    )]
    Open(LocalWindow),
}

/// A window that is open because the player asked for it, not because the shard
/// opened it.
///
/// The two kinds whose *existence* is local — a `0x3A` refreshes the skill
/// numbers and a `0x11` the status ones, and neither asks for a window. A
/// container or a paperdoll cannot be here: asking for one of those is
/// [`Effect::Net`], and the window appears when the view grows the entry that
/// `reconcile_own_windows` turns into one (decision 8).
#[expect(
    dead_code,
    reason = "performed by App::perform; the paperdoll's Skills button asks for one at step 5"
)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LocalWindow {
    Skills,
    Status,
}

/// One of the client's own windows, as a component.
///
/// Implemented once per window kind, and reached through [`AnyPane`] rather
/// than through `dyn` — see decision 1 for why the vtable was refused.
///
/// The three methods are three phases of a frame, and decision 6 is that they
/// are called in this order for *every* pane before the next one starts:
/// [`art`](Pane::art) for all, pack once, [`layout`](Pane::layout) for all.
/// That order is why the last two take `&self` and a *shared* atlas — a pane
/// cannot be laid out against an atlas that is still growing.
pub trait Pane {
    /// The art this pane needs packed before it can be laid out.
    ///
    /// Asked every frame and not once at open: what a window needs depends on
    /// what is in it, and the atlas answers a repeat with nothing — see
    /// [`GumpAtlas::add`](openshard_client_render::gump::GumpAtlas::add), which
    /// filters what it already holds.
    fn art(&self, frame: &PaneFrame<'_>) -> Vec<GumpArt>;

    /// Lay this pane out for this frame: the pictures the pass draws and the
    /// pointer is tested against.
    ///
    /// `None` is a window with nothing to draw — a catalogue whose subject has
    /// gone out of the view between the packet and the frame — and not an
    /// error: it simply contributes no entry to
    /// [`Windows::drawn_windows`](crate::windows::Windows::drawn_windows), so
    /// nothing of it is drawn and nothing of it is clickable.
    fn layout(&self, frame: &PaneFrame<'_>) -> Option<Drawn>;

    /// Offer one input. The answer says whether the pane took it, whether the
    /// frame is stale because of it, and what it is asking the manager to do.
    fn handle(&mut self, input: Input, ctx: &PaneCtx<'_>) -> Response;
}

/// Every window kind, as one type.
///
/// Decision 1: an `enum` and not `Box<dyn Pane>`, because this is what makes a
/// seventh window kind a *compile error* everywhere the manager still has to
/// know which kind it has — the same reason
/// [`WindowSubject`](crate::windows::WindowSubject) and
/// [`Drawn`](crate::windows::Drawn) are enums — and because a vtable and an
/// allocation in a list this client walks several times a frame buys nothing
/// for a set of kinds that is known at compile time.
///
/// It lives in [`OwnWindow`](crate::windows::OwnWindow), beside the position:
/// **a pane's state is tied to its window's lifetime by construction**, which
/// is the other half of the defect this plan was written for. A shop's scroll
/// position used to be an entry in a map on `Windows` that `close_window` had
/// to remember to remove by hand.
#[derive(Debug)]
pub enum AnyPane {
    Container(ContainerPane),
    Vendor(vendor::VendorPane),
    Paperdoll(PaperdollPane),
    Dialog(DialogPane),
    Skills(SkillsPane),
    Status(StatusPane),
}

impl AnyPane {
    /// The pane a window of this subject needs, built when the window opens.
    ///
    /// A kind whose subject carries a key is handed it here and keeps it: a
    /// shop names its vendor in the `0x3B` it sends, and the pane that builds
    /// that packet has to know which mobile it is trading with. That is not the
    /// pane knowing it is a window — the position, the z-order and the list are
    /// still the manager's — it is the pane knowing what it is a pane *of*.
    pub fn of(subject: WindowSubject) -> Self {
        match subject {
            WindowSubject::Container(_) => Self::Container(ContainerPane::default()),
            WindowSubject::Vendor(serial) => Self::Vendor(vendor::VendorPane::new(serial)),
            WindowSubject::Paperdoll(_) => Self::Paperdoll(PaperdollPane::default()),
            WindowSubject::Dialog(_) => Self::Dialog(DialogPane::default()),
            WindowSubject::Skills => Self::Skills(SkillsPane::default()),
            WindowSubject::Status => Self::Status(StatusPane::default()),
        }
    }
}

impl Pane for AnyPane {
    /// The delegating `match` decision 1 pays for the enum with: six lines per
    /// trait method, once.
    fn art(&self, frame: &PaneFrame<'_>) -> Vec<GumpArt> {
        match self {
            Self::Container(pane) => pane.art(frame),
            Self::Vendor(pane) => pane.art(frame),
            Self::Paperdoll(pane) => pane.art(frame),
            Self::Dialog(pane) => pane.art(frame),
            Self::Skills(pane) => pane.art(frame),
            Self::Status(pane) => pane.art(frame),
        }
    }

    fn layout(&self, frame: &PaneFrame<'_>) -> Option<Drawn> {
        match self {
            Self::Container(pane) => pane.layout(frame),
            Self::Vendor(pane) => pane.layout(frame),
            Self::Paperdoll(pane) => pane.layout(frame),
            Self::Dialog(pane) => pane.layout(frame),
            Self::Skills(pane) => pane.layout(frame),
            Self::Status(pane) => pane.layout(frame),
        }
    }

    fn handle(&mut self, input: Input, ctx: &PaneCtx<'_>) -> Response {
        match self {
            Self::Container(pane) => pane.handle(input, ctx),
            Self::Vendor(pane) => pane.handle(input, ctx),
            Self::Paperdoll(pane) => pane.handle(input, ctx),
            Self::Dialog(pane) => pane.handle(input, ctx),
            Self::Skills(pane) => pane.handle(input, ctx),
            Self::Status(pane) => pane.handle(input, ctx),
        }
    }
}

/// A bag's window. Step 6 moves `last_container_click` here, and with it the
/// press that becomes either a lift or a double-click use.
#[derive(Debug, Default)]
pub struct ContainerPane {}

/// A body's paperdoll. Step 5 moves `held_doll` and `last_scroll` here.
#[derive(Debug, Default)]
pub struct PaperdollPane {}

/// A `0xB0` dialog. Step 4 moves the entry `gump::Dialogs` keeps for this gump
/// here — the page, the switches, the typed text and the held button.
#[derive(Debug, Default)]
pub struct DialogPane {}

/// This character's skill sheet. Step 2 moves the `skills::Tree` and
/// `held_skill` here, and `Windows::skills` being `Some` stops being what "the
/// window is open" means: the pane's presence in the list is.
#[derive(Debug, Default)]
pub struct SkillsPane {}

/// This character's status frame. Step 3, and the one that proves a pane with
/// no input at all is still a pane.
#[derive(Debug, Default)]
pub struct StatusPane {}

// Every kind that has not moved in yet declines all three questions. Each of
// these is replaced whole by the pane that step moves in; until then the `App`
// method named in each comment still answers that kind's input, and
// `App::deliver` calls it after the panes have all passed, while
// `render_passes::draw_gump_windows` still packs and lays that kind out from
// the view. A shim that answered anything at all would be a second opinion
// about the same click, and a pane that packed its own art while the pass still
// laid it out would be two answers about the same picture.
//
// Written out per kind rather than left to a defaulted trait method: a default
// saying "no art, no layout" is exactly what a pane that has moved in and
// forgotten to implement one would silently get, and the failure mode of that
// is a window that draws nothing.

impl Pane for ContainerPane {
    /// Still `container::art_of`, called from `render_passes` — step 6.
    fn art(&self, _frame: &PaneFrame<'_>) -> Vec<GumpArt> {
        Vec::new()
    }

    /// Still `container::window_highlighted`, laid out by `render_passes` —
    /// step 6.
    fn layout(&self, _frame: &PaneFrame<'_>) -> Option<Drawn> {
        None
    }

    /// Still `App::press_on_own_window`, `App::release_container_item` and
    /// `App::hover_container_item` — step 6.
    fn handle(&mut self, _input: Input, _ctx: &PaneCtx<'_>) -> Response {
        Response::ignored()
    }
}

impl Pane for PaperdollPane {
    /// Still `paperdoll::art_of`, called from `render_passes` — step 5.
    fn art(&self, _frame: &PaneFrame<'_>) -> Vec<GumpArt> {
        Vec::new()
    }

    /// Still `paperdoll::doll`, laid out by `render_passes` — step 5.
    fn layout(&self, _frame: &PaneFrame<'_>) -> Option<Drawn> {
        None
    }

    /// Still `App::press_on_own_window`, `App::release_on_own_window` and
    /// `App::hover_paperdoll_item` — step 5.
    fn handle(&mut self, _input: Input, _ctx: &PaneCtx<'_>) -> Response {
        Response::ignored()
    }
}

impl Pane for DialogPane {
    /// Still `gump::art_of`, called from `render_passes` for every open gump in
    /// the view rather than per window — step 4.
    fn art(&self, _frame: &PaneFrame<'_>) -> Vec<GumpArt> {
        Vec::new()
    }

    /// Still `gump::Dialogs::layout`, laid out by `render_passes` — step 4.
    fn layout(&self, _frame: &PaneFrame<'_>) -> Option<Drawn> {
        None
    }

    /// Still `gump::Dialogs::press`/`release`, reached from
    /// `App::press_on_own_window` — step 4.
    fn handle(&mut self, _input: Input, _ctx: &PaneCtx<'_>) -> Response {
        Response::ignored()
    }
}

impl Pane for SkillsPane {
    /// The skill sheet packs nothing of its own even today: its frame is one of
    /// the gumps the atlas is grown with elsewhere — step 2.
    fn art(&self, _frame: &PaneFrame<'_>) -> Vec<GumpArt> {
        Vec::new()
    }

    /// Still `skills::window`, laid out by `render_passes` — step 2.
    fn layout(&self, _frame: &PaneFrame<'_>) -> Option<Drawn> {
        None
    }

    /// Still `App::skill_hit_under_pointer`, `App::drag_thumb` and
    /// `App::scroll_skills` — step 2.
    fn handle(&mut self, _input: Input, _ctx: &PaneCtx<'_>) -> Response {
        Response::ignored()
    }
}

impl Pane for StatusPane {
    /// Step 3.
    fn art(&self, _frame: &PaneFrame<'_>) -> Vec<GumpArt> {
        Vec::new()
    }

    /// Still `status::window`, laid out by `render_passes` — step 3.
    fn layout(&self, _frame: &PaneFrame<'_>) -> Option<Drawn> {
        None
    }

    /// Nothing answers this one today either: the status frame has no input at
    /// all, and step 3 is what says so in a type.
    fn handle(&mut self, _input: Input, _ctx: &PaneCtx<'_>) -> Response {
        Response::ignored()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four corners of decision 4 exist as four values, which is the whole
    /// of the fix: a `bool` has two.
    #[test]
    fn taken_and_redraw_are_four_answers_and_not_two() {
        let corners = [
            Response::ignored(),
            Response::consumed(),
            Response::changed(),
            Response::stale(),
        ]
        .map(|response| (response.taken, response.redraw));
        assert_eq!(
            corners,
            [(false, false), (true, false), (true, true), (false, true)],
            "the wheel defect was `consumed` and `ignored` being one value"
        );
    }

    /// Folding is what lets one input have more than one answer without either
    /// of the two questions being lost — a pane's hover beside the manager's
    /// own drag, say.
    #[test]
    fn absorbing_keeps_either_yes() {
        let mut response = Response::consumed();
        response.absorb(Response::stale());
        assert!(response.taken, "the pane took it");
        assert!(response.redraw, "and the other answer says the frame is stale");

        let mut response = Response::ignored();
        response.absorb(Response::ignored());
        assert!(!response.taken);
        assert!(!response.redraw);
    }

    /// Effects come out in the order they were asked for: a pane that raises
    /// itself and then sends a packet means that order.
    #[test]
    fn effects_keep_their_order() {
        let mut response = Response::changed().with(Effect::Raise);
        response.absorb(Response::consumed().with(Effect::Close));
        assert!(matches!(response.out.as_slice(), [Effect::Raise, Effect::Close]));
    }

    /// A subject gets the pane its kind names, which is what makes a seventh
    /// window kind a compile error here rather than a silent nothing.
    #[test]
    fn every_subject_has_a_pane() {
        let serial = openshard_protocol::serial::Serial::new(0x0000_002A).unwrap();
        assert!(matches!(
            AnyPane::of(WindowSubject::Container(serial)),
            AnyPane::Container(_)
        ));
        assert!(matches!(
            AnyPane::of(WindowSubject::Vendor(serial)),
            AnyPane::Vendor(_)
        ));
        assert!(matches!(
            AnyPane::of(WindowSubject::Paperdoll(serial)),
            AnyPane::Paperdoll(_)
        ));
        assert!(matches!(
            AnyPane::of(WindowSubject::Dialog(openshard_protocol::gump::GumpId(7))),
            AnyPane::Dialog(_)
        ));
        assert!(matches!(AnyPane::of(WindowSubject::Skills), AnyPane::Skills(_)));
        assert!(matches!(AnyPane::of(WindowSubject::Status), AnyPane::Status(_)));
    }
}
