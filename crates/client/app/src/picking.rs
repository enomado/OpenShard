//! What is under the cursor, and what a click named: [`Picking`].
//!
//! Every field here is filled by the same one chain — creatures first, then
//! the shard's own items, then the map's own furniture, see `App::draw`'s
//! `on_mobile`/`on_item`/`on_static` walk — or by the click that reads it
//! back a frame later. Pulled out of [`crate::App`] for the same reason
//! [`crate::world::WorldState`] was: these four fields are written together,
//! by one pass, and read together by [`crate::App::resolve_selection`] and
//! the held-selection ring.

use openshard_client_render::statics::PickedStatic;
use openshard_protocol::serial::Serial;

use crate::crowd::Who;
use crate::diagnostics::PickedTile;

/// What a left click named, kept as identity rather than as data — see
/// [`Picking::selected`] for why. [`crate::App::resolve_selection`] is the
/// only reader.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SelectedIdentity {
    /// Bare ground: nothing with its own identity was under the cursor.
    Tile { x: u16, y: u16 },
    /// The map's own furniture — never moves, so the pick itself is kept
    /// rather than just a reference to re-look-up.
    Static(PickedStatic),
    /// A creature, by [`Who`] — `None` for the player's own body.
    Mobile(Who),
    /// An item lying on the ground, by its serial.
    Item(Serial),
}

impl SelectedIdentity {
    /// The static half alone, for the two render passes that wash and mask
    /// it — [`openshard_client_render::select`] and `statics::selected`.
    /// `None` whenever a click landed on anything else, which is what
    /// switches both passes off.
    ///
    /// A free function on the value rather than a method on `App`: both call
    /// sites read `self.picking.selected` while `self.window` is already
    /// borrowed mutably, and a method taking `&self` would borrow the whole
    /// struct where a direct field read borrows only the one field.
    pub fn as_static(self) -> Option<PickedStatic> {
        match self {
            SelectedIdentity::Static(picked) => Some(picked),
            _ => None,
        }
    }

    /// The mobile half alone, for the held-selection ring — see
    /// `Screen::held_mask`.
    pub fn as_mobile(self) -> Option<Who> {
        match self {
            SelectedIdentity::Mobile(who) => Some(who),
            _ => None,
        }
    }

    /// The item half alone, for the held-selection ring.
    pub fn as_item(self) -> Option<Serial> {
        match self {
            SelectedIdentity::Item(serial) => Some(serial),
            _ => None,
        }
    }
}

/// What the cursor is on, and what the last click named — see the module
/// docs.
#[derive(Default)]
pub struct Picking {
    /// What the last drawn frame found the cursor on, when it was the map's
    /// own furniture and nothing nearer.
    ///
    /// A frame behind, and that is what makes it right rather than what it
    /// costs: a click arrives *between* frames, so the picture it is a click
    /// on is the one already drawn. Picking again at the click would ask a
    /// camera that has moved since — see the `MouseInput` arm, where this is
    /// read.
    ///
    /// It is also the tile marker's reason for going out: a wall under the
    /// cursor is what the click would take, so the diamond on the ground
    /// behind it must not be drawn as well. See
    /// [`shell::Hud::hover_lit`](crate::shell::Hud::hover_lit).
    pub on_static: Option<PickedStatic>,
    /// The same, one rung up the pick order: a creature under the cursor, by
    /// identity rather than by the frame's own transient index — [`Who`]
    /// survives to the click, an index into a `Vec` rebuilt every frame does
    /// not.
    pub on_mobile: Option<Who>,
    /// The same, for the shard's own items lying on the ground.
    pub on_item: Option<Serial>,
    /// What a left click last landed on, kept by *identity* until the next
    /// click — a coordinate, a static's own graphic-and-place, or a
    /// creature's or item's serial. Never the data itself:
    /// [`crate::App::hud`] turns this into a
    /// [`shell::Selection`](crate::shell::Selection) fresh every frame, the
    /// same way [`crate::App::tile_info`] always re-reads the column rather
    /// than remembering one — so a selected mobile's row keeps up with it
    /// walking, and a selected item's row goes away the moment it is picked
    /// up, instead of the panel quietly lying about where either still is.
    pub selected: Option<SelectedIdentity>,
}

/// Everything this frame's cursor is over, answered once in
/// [`crate::App::frame_facts`] and carried whole into
/// [`shell::Hud`](crate::shell::Hud) rather than unpacked into a handful of
/// its fields: `tile`/`static_`/`mobile` are exactly the "picked tile +
/// picked static + picked mobile" three the HUD used to gather separately,
/// one of them (`tile`) computed a second time under a different name
/// (`hover`) by `App::hud` itself. One pick, one place, one reader at a time
/// can disagree with itself.
#[derive(Clone)]
pub struct Pick {
    /// The ground tile under the cursor — the fact the HUD's tile marker,
    /// route preview and terrain overlay all read, whether or not a mobile,
    /// item or static also took the highlight this frame. See
    /// [`crate::App::pick_tile`].
    pub tile: Option<PickedTile>,
    /// The eight tiles around [`Pick::tile`], for the wireframe ring drawn
    /// beside it.
    pub neighbours: Vec<PickedTile>,
    /// The map's own static the cursor is over, when no mobile or item is.
    /// Statics have no highlight mode to filter through: a wall is either
    /// under the cursor or it is not.
    pub static_: Option<PickedStatic>,
    /// The creature under the cursor, filtered by whether the highlight mode
    /// allows one to light up at all — the unfiltered form is
    /// `FrameFacts::on_mobile`, read back into [`Picking::on_mobile`]
    /// regardless of the mode: what a click selects is not a question about
    /// lighting.
    pub mobile: Option<openshard_client_render::mobiles::MobileIndex>,
    /// The item under the cursor, filtered the same way.
    pub item: Option<openshard_client_render::items::ItemIndex>,
}
