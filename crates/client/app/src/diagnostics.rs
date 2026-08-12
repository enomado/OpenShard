//! Read-only diagnostic facts about the world.
//!
//! These values sit between the application queries and the dev HUD.  They do
//! not depend on egui: a panel, a frame dump, or a future remote inspector can
//! all consume the same answer without the query layer depending on its view.

use openshard_client_render::facing::Prism;
use openshard_protocol::serial::Serial;
use openshard_protocol::wire::{Graphic, Hue};

/// A z-height in the wire's own unit.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Height(pub i8);

/// A draw-order key's tile component alone.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct TileDepth(pub i32);

/// A static's draw-order priority within its tile.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct PriorityZ(pub i32);

/// Everything the client knows about one map tile for inspection.
#[derive(Clone)]
pub struct PickedTile {
    pub at: openshard_movement::Tile,
    pub land: Option<Graphic>,
    pub land_z: Height,
    pub stand_z: Height,
    pub corners: [Height; 4],
    pub levels: Vec<(Height, bool)>,
    pub ceiling: Option<Height>,
    pub statics: Vec<(Graphic, Height, Hue, PriorityZ)>,
    pub items: Vec<(Graphic, Height, Hue, PriorityZ)>,
    pub tile_depth: TileDepth,
    pub mobile_order: Option<openshard_client_render::depth::Order>,
}

/// A mobile identified by a click and resolved afresh for each frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PickedMobile {
    pub you: bool,
    pub serial: Option<Serial>,
    pub body: Graphic,
    pub hue: Hue,
    pub at: openshard_protocol::world::Point,
    pub order: openshard_client_render::depth::Order,
}

/// A server-owned ground item identified by a click.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PickedItem {
    pub serial: Serial,
    pub graphic: Graphic,
    pub hue: Hue,
    pub at: openshard_protocol::world::Point,
    pub priority_z: PriorityZ,
}

/// What a left click landed on, with dynamic objects resolved from identity.
pub enum Selection {
    Tile(PickedTile),
    Static {
        static_: openshard_client_render::statics::PickedStatic,
        tile: PickedTile,
        prism: Option<Prism>,
    },
    Mobile(Option<(PickedMobile, PickedTile)>),
    Item(Option<(PickedItem, PickedTile)>),
}

impl Selection {
    /// The tile column associated with the selected subject, if it remains in
    /// the current presentation.
    pub fn tile(&self) -> Option<&PickedTile> {
        match self {
            Self::Tile(tile) => Some(tile),
            Self::Static { tile, .. } => Some(tile),
            Self::Mobile(live) => live.as_ref().map(|(_, tile)| tile),
            Self::Item(live) => live.as_ref().map(|(_, tile)| tile),
        }
    }
}

/// The walkability of the tiles currently in view.
pub struct TerrainOverlay {
    pub open: Vec<openshard_protocol::world::Point>,
    pub blocked: Vec<openshard_protocol::world::Point>,
}

/// A planned route, split at the first obstacle the path cannot cross.
pub struct Route {
    pub open: Vec<openshard_protocol::world::Point>,
    pub barred: Vec<openshard_protocol::world::Point>,
}
