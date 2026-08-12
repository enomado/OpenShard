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
use openshard_protocol::direction::Facing;
use openshard_protocol::serial::Serial;
use openshard_protocol::world::Point;
use openshard_uofiles::map::Map;
use openshard_uofiles::tiledata::TileData;

use crate::crowd::{Crowd, Who};
use crate::{clutter, link, resources};

/// What the connection has told this client the world looks like — see the
/// module docs.
pub struct WorldState {
    /// The server's last complete word, and only that. It is owned and mutated
    /// on the application thread; render data is rebuilt from it below rather
    /// than sharing or mutating this record from another thread.
    pub authoritative: AuthoritativeWorld,
    /// The local answer to where our own body should be before the shard has
    /// confirmed it. Presentation projects this into its `player`; it never
    /// changes [`AuthoritativeWorld::view`].
    pub prediction: PredictionState,
    /// The renderer-facing projection rebuilt from authoritative state and
    /// prediction before a frame is drawn.
    pub presentation: PresentationWorld,
    /// Whether a world picture is safe to show. The offline viewer starts
    /// ready; a connected client becomes ready only when the shard has sent its
    /// first complete [`WorldView`]. Until then the presentation's placeholder
    /// is state for startup mechanics, not a picture for the player.
    pub render_ready: bool,
    /// What the connection is doing, for the status strip.
    pub connection: String,
    /// The shard, if this run logged in to one.
    ///
    /// `None` is the offline viewer, and it is what the keyboard asks: a step
    /// is a `0x02` when there is somebody to send it to, and a camera move when
    /// there is not.
    pub link: Option<link::Link>,
}

/// Render-facing data rebuilt from the authoritative view and local prediction.
/// This is the only `WorldState` section a frame reads.
pub struct PresentationWorld {
    /// Static animation state and its flame clock, advanced with the frame.
    pub tile_animations: StaticAnimations,
    pub flame_clock: Duration,
    /// The player's rendered body. Its position and facing are projected from
    /// [`PredictionState`]; body, hue and equipment come from the shard.
    pub player: Mobile,
    /// The guarded cutaway tile. It may deliberately lag a doomed prediction.
    pub cutaway_at: Point,
    /// Render mobiles beside the identity their animation clocks use.
    pub others: Vec<(Who, Mobile)>,
    /// Ground-item render data and the parallel wire serials used for picks.
    pub items: Vec<GroundItem>,
    pub item_serials: Vec<Serial>,
    /// The corresponding transient obstacles used by local movement.
    pub clutter: clutter::Clutter,
    /// Animation and glide history, which belongs to presentation rather than
    /// authoritative state.
    pub crowd: Crowd,
}

/// The one authoritative record the shard updates, kept apart from the
/// presentation projection and local prediction state in [`WorldState`].
pub struct AuthoritativeWorld {
    /// The last thing the server said, whole. Kept for the HUD and as the sole
    /// source from which this app rebuilds render projections.
    pub view: Option<Box<WorldView>>,
    /// Whether the shard's facet has been compared with the one loaded. See
    /// `App::entered`: once, because it cannot change without a `0xBF 0x08`
    /// nothing here reads yet.
    pub facet_checked: bool,
}

/// The local movement state that may be ahead of the server's last confirmed
/// position. It is deliberately smaller than a render [`Mobile`]: no body,
/// equipment or animation clock belongs to a prediction.
#[derive(Clone, Copy, Debug)]
pub struct PredictionState {
    /// The tile the last accepted step expects us to reach.
    pub at: Point,
    /// The facing that step asked for, including its walking/running mode.
    pub facing: Facing,
}

impl PredictionState {
    /// Record the prediction the shard thread paired with an update.
    pub fn apply(&mut self, body: link::Body) {
        self.at = body.predicted.position;
        self.facing = body.predicted.facing;
    }

    /// Record a movement made by the offline viewer or replay, which has no
    /// shard handshake to produce a [`link::Body`].
    pub fn set(&mut self, at: Point, facing: Facing) {
        self.at = at;
        self.facing = facing;
    }
}

impl WorldState {
    /// Who the crowd knows our own body as.
    ///
    /// Our serial once a shard has named us, and `None` for the offline
    /// placeholder — see [`Who`].
    pub fn me(&self) -> Who {
        self.authoritative.view.as_ref().map(|view| view.player.serial)
    }

    /// Where the body is drawn this instant, wherever the glide has it —
    /// [`crate::App::follow_player`]'s reason for calling this every frame.
    pub fn drawn_player(&self) -> Gaze {
        self.presentation
            .crowd
            .drawn_for(self.me())
            .unwrap_or_else(|| Gaze::on(self.presentation.player.at))
    }
}

/// The client's own map, unclutted by anything the shard has stood on it —
/// [`cluttered`] is what a step decision should actually ask.
///
/// A facade over [`resources::Resources::map`] and
/// [`resources::Resources::tiledata`], which travel together in every caller
/// that wants either: the split that put them in `Resources` and
/// [`WorldState::clutter`] in `WorldState` is about what changes
/// together (disk-read once against updated every packet), not about who
/// asks for them together, and this is the seam that reunites the two.
///
/// **A free function taking `&Resources`, not an `App` method taking
/// `&self`.** A method on `App` borrows the whole of it, so a caller that
/// also holds `&mut self.steer` beside the terrain — every arrow key and
/// every replanned step does — could no longer compile: the borrow checker
/// sees disjoint *fields* through a chain of `.` projections but not through
/// a method call, which is opaque to it. Passing `&self.resources` here is
/// the same projection the field access always was, just wrapped.
pub(crate) fn terrain(resources: &resources::Resources) -> openshard_movement::MapTerrain<&Map, &TileData> {
    openshard_movement::MapTerrain::new(resources.map.as_ref(), &resources.tiledata)
}

/// [`terrain`] with the shard's own items laid over it — what every step
/// decision on this end should actually ask. See [`clutter::Clutter::over`],
/// and `terrain`'s own docs for why this takes references rather than being
/// an `App` method.
pub(crate) fn cluttered<'a>(
    world: &'a WorldState,
    resources: &'a resources::Resources,
) -> clutter::Cluttered<'a, openshard_movement::MapTerrain<&'a Map, &'a TileData>> {
    world
        .presentation
        .clutter
        .over(&resources.map, &resources.tiledata)
}

/// The same, read as though every shut door on it stood open: what a route
/// may be *planned* through, and never what decides a step. See
/// [`clutter::Clutter::over_with_doors_open`].
pub(crate) fn cluttered_with_doors_open<'a>(
    world: &'a WorldState,
    resources: &'a resources::Resources,
) -> clutter::Cluttered<'a, openshard_movement::MapTerrain<&'a Map, &'a TileData>> {
    world
        .presentation
        .clutter
        .over_with_doors_open(&resources.map, &resources.tiledata)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prediction_keeps_its_position_outside_the_authoritative_view() {
        let mut prediction = PredictionState {
            at: Point::new(100, 100, 0),
            facing: Facing::walking(openshard_protocol::direction::Direction::North),
        };
        prediction.apply(link::Body {
            predicted: openshard_client_net::walk::Predicted {
                position: Point::new(101, 100, 7),
                facing: Facing::running(openshard_protocol::direction::Direction::East),
            },
            corrected: false,
        });

        assert_eq!(prediction.at, Point::new(101, 100, 7));
        assert_eq!(
            prediction.facing,
            Facing::running(openshard_protocol::direction::Direction::East)
        );
    }
}
