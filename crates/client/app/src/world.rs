//! What the shard — or its absence — has said: [`WorldState`].
//!
//! Every field here is a projection of the last `WorldView` the connection
//! produced, or of the clocks that age it, and none of it is read from disk —
//! see [`crate::resources::Resources`] for that half, and
//! [`crate::graphics::GraphicsSettings`] for the person's own view of it.
//! Pulled out of [`crate::App`] for the same reason both of those were: the
//! fields here change together, on every `Update::World`, and a method that
//! only touches this half can be written and tested against it alone.

use std::time::Duration;

use openshard_client_net::view::WorldView;
use openshard_client_render::animate::StaticAnimations;
use openshard_client_render::follow::Gaze;
use openshard_client_render::items::GroundItem;
use openshard_client_render::mobiles::Mobile;
use openshard_protocol::serial::Serial;
use openshard_protocol::world::Point;

use crate::crowd::{Crowd, Who};
use crate::{clutter, link};

/// What the connection has told this client the world looks like — see the
/// module docs.
pub struct WorldState {
    /// The statics that move on their own — fires, torches, water wheels — and
    /// how far into their cycles they are.
    ///
    /// One of the clocks this app owns, and it is advanced from the same sampled
    /// instant as the crowd and the eye. Its own module argues why it is a system
    /// rather than a flag on a quad: see [`StaticAnimations`].
    pub tile_animations: StaticAnimations,
    /// How long the flames have been burning, in the same span every other clock
    /// in the frame is advanced by.
    ///
    /// Its own accumulator rather than an `Instant`, for the reason
    /// [`StaticAnimations`] has one: `openshard-client-render` reads no clock,
    /// so the time arrives as a number, and a number sampled once per frame is
    /// what keeps a torch's flicker on the same instant as the body walking
    /// past it.
    pub flame_clock: Duration,
    /// This client's own body.
    ///
    /// Connected, it is what the server says: `0x1B` puts it somewhere and
    /// every ack, `0x20` and `0x21` moves it. Offline it is a placeholder
    /// standing wherever the camera looks, which is enough to hold the
    /// animation reader, the frame atlas and the placement against a real
    /// install.
    pub player: Mobile,
    /// The tile roof-cutaway is computed from — see `App::draw`'s use of it with
    /// [`openshard_client_render::cutaway::Cutaway`].
    ///
    /// Deliberately not always `player.at`: that is this end's own optimistic
    /// *prediction*, published the instant a step is sent and corrected only
    /// a round trip later (see `link::Body`), and `Steering::detour`
    /// (`steer.rs`) means a held direction pinned against an obstacle asks
    /// for the very tile it is going to be refused on, every hold, for as
    /// long as it is held. Feeding that straight to `Cutaway::at` flips which
    /// roof is drawn hidden for exactly the frame between sending the doomed
    /// step and the `0x21` undoing it — a real defect this field exists to
    /// close, not the deliberate lag-compensation `player.at` is for the
    /// body's own drawn position. This only ever advances to a tile the
    /// client's own static map agrees is reachable from the last one it
    /// held, so a refusal is never drawn from; a correction snaps it the same
    /// way it snaps `player.at`.
    pub cutaway_at: Point,
    /// Everyone else on screen, as `0x77` and `0x78` last described them, each
    /// beside the serial the crowd's clocks are keyed by.
    ///
    /// Empty offline, and rebuilt whole from the [`WorldView`] on every update:
    /// the view is the record of what arrived and this is a projection of it,
    /// so there is nothing here to keep in step by hand.
    pub others: Vec<(Who, Mobile)>,
    /// Everything lying on the ground, as `0x1A` and `0x1D` last left it.
    ///
    /// A projection of the view like [`WorldState::others`], and drawn through
    /// the same atlas and the same pass as the map's own statics: an item's
    /// picture is a static's picture. Two lists rather than one because the
    /// map's furniture never moves and these come and go with every packet.
    pub items: Vec<GroundItem>,
    /// What each of those items is called on the wire, at the same index.
    ///
    /// The renderer drops the serial — it draws pictures and owns no model of
    /// the world — and a click has to put it back, because "use this" is a
    /// serial and nothing else. Built in the same pass as [`WorldState::items`]
    /// and never separately: two loops over one map is how the lists drift, and
    /// a drifted index sends the shard a double-click on whatever was next.
    pub item_serials: Vec<Serial>,
    /// Which of those items a step cannot go through, indexed by tile.
    ///
    /// A third projection of the view beside [`WorldState::items`] and
    /// [`WorldState::others`], rebuilt with them: the map's own files hold no
    /// barrel, so without this every terrain check here looks straight through
    /// one and the shard refuses the step this end thought was open. See
    /// `clutter.rs`.
    pub clutter: clutter::Clutter,
    /// The last thing the server said, whole.
    ///
    /// Kept only for the HUD's world window, which lists what has been decoded
    /// with the serials the three projections above drop. The renderer reads
    /// those.
    pub view: Option<Box<WorldView>>,
    /// What the connection is doing, for the status strip.
    pub connection: String,
    /// The shard, if this run logged in to one.
    ///
    /// `None` is the offline viewer, and it is what the keyboard asks: a step
    /// is a `0x02` when there is somebody to send it to, and a camera move when
    /// there is not.
    pub link: Option<link::Link>,
    /// Whether the shard's facet has been compared with the one loaded. See
    /// `App::entered`: once, because it cannot change without a `0xBF 0x08`
    /// nothing here reads yet.
    pub facet_checked: bool,
    /// What everyone on screen was doing a moment ago: which animation each is
    /// playing, and how far into it.
    ///
    /// The layer above [`WorldView`] that ages what it sees — see `crowd.rs`.
    /// Real time and not the world tick: there is no world here to tick, and a
    /// real client's body animation is a wall-clock timer too.
    pub crowd: Crowd,
}

impl WorldState {
    /// Who the crowd knows our own body as.
    ///
    /// Our serial once a shard has named us, and `None` for the offline
    /// placeholder — see [`Who`].
    pub fn me(&self) -> Who {
        self.view.as_ref().map(|view| view.player.serial)
    }

    /// Where the body is drawn this instant, wherever the glide has it —
    /// [`crate::App::follow_player`]'s reason for calling this every frame.
    pub fn drawn_player(&self) -> Gaze {
        self.crowd
            .drawn_for(self.me())
            .unwrap_or_else(|| Gaze::on(self.player.at))
    }
}
