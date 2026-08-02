//! Rooms to point the lighting at.
//!
//! A scene here is a whole little world — flat ground, a `tiledata` where a
//! handful of graphics have flags, a list of items standing on it and a camera
//! looking at it — and it is *built* rather than loaded. That is not a
//! concession to the rule that no client files live in this repository; it is
//! better than a real house for the job these do. The wall is at a stated tile
//! with a stated height, so a test can name the cell that should have stopped a
//! ray, and a failure can print the room rather than a coordinate. See
//! `docs/lighting.md`, decision 10.
//!
//! # Why these are `pub` and not `#[cfg(test)]`
//!
//! Three things outside this crate want them: the GPU tests in
//! `tests/frame.rs`, which run the real blit over a scene's lighting; a future
//! benchmark, which needs a frame whose contents are known; and the playground,
//! where a person looking at a leak wants to stand inside the room rather than
//! read about it.
//!
//! # What a scene is not
//!
//! It has no art. Nothing here can be drawn — [`Scene::lighting`] is the whole
//! of what a scene produces, and what reads it is [`crate::light::sample`] or
//! the blit. A scene that also carried sprites would need a client install, and
//! then none of these tests would run anywhere.

use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::Point;
use openshard_uofiles::map::{LandCell, Map};
use openshard_uofiles::tiledata::{StaticTile, TileData, TileFlags};

use crate::camera::Camera;
use crate::cutaway::Cutaway;
use crate::items::GroundItem;
use crate::light::{self, Lighting};

/// A wall: what stops an arrow, and therefore what stops light. Twenty `z` units
/// tall, which is a storey.
pub const WALL: Graphic = Graphic(0x0006);

/// A pane of glass. `WINDOW` is what the reference's line of sight tests
/// alongside `NO_SHOOT` — see [`crate::occlusion`] — so today this stops light
/// exactly as a wall does, and the scene that uses it is the one that will say
/// so when it stops.
pub const PANE: Graphic = Graphic(0x0007);

/// A door, shut. The leaf fills its tile and nothing sees through it.
pub const DOOR_SHUT: Graphic = Graphic(0x0008);

/// The same door, open — a *different graphic*, which is the whole mechanism.
///
/// An open leaf is against the wall beside its doorway and you can shoot through
/// the gap, so its tiledata entry carries no `NO_SHOOT`; the tile therefore
/// leaves the occlusion grid on the frame the server changes the graphic, and
/// light fans out through the doorway with nothing in the lighting knowing what a
/// door is. `docs/lighting.md`, decision 11.
pub const DOOR_OPEN: Graphic = Graphic(0x0009);

/// A torch. Flagged `LIGHT_SOURCE`, which is the only reason anything burns —
/// see [`crate::light`].
pub const TORCH: Graphic = Graphic(0x0A12);

/// How tall the walls of these rooms are, in `z` units.
pub const WALL_HEIGHT: u8 = 20;

/// Where the rooms are built. Far from the map's edge so that a camera's bounds
/// are never clipped, and a round number so that a diagram is readable.
pub const CENTRE: (u16, u16) = (100, 100);

/// How far a room's wall is from its centre: a seven-by-seven house.
///
/// Big enough that the inside has tiles that are not the wall and not the torch,
/// which is where "the room is lit" is actually measured, and small enough that
/// a torch's six-tile pool reaches every wall of it.
pub const ROOM_HALF: u16 = 3;

/// One world to ask the lighting about.
#[derive(Debug)]
pub struct Scene {
    /// What it is, for the message a failing test prints.
    pub name: &'static str,
    /// Flat ground at `z = 0`, wide enough that no camera runs off it.
    pub map: Map,
    /// The flags of the graphics above, and nothing else: an unlisted graphic
    /// has no flags at all, so it neither burns nor occludes.
    pub tiledata: TileData,
    /// What stands in it. Statics come through the item list rather than the map
    /// because a built [`Map`] has none — see [`Map::from_blocks`].
    pub items: Vec<GroundItem>,
    /// Looking at [`CENTRE`], at the zoom every other test uses.
    pub camera: Camera,
    /// Open: these scenes have one storey, and a cutaway that hid half of it
    /// would be a second variable in every assertion.
    pub cutaway: Cutaway,
}

impl Scene {
    /// This scene's flames and occluders, at night.
    ///
    /// `time` is the flicker's, and every test passes `0.0`: a flame's
    /// brightness swings by a tenth and an assertion about a leak should not
    /// depend on which tenth of a second it was asked in.
    pub fn lighting(&self, time: f32) -> Lighting {
        light::collect(
            &self.map,
            &self.items,
            &self.camera,
            &self.tiledata,
            &self.cutaway,
            light::NIGHT,
            time,
        )
    }

    /// The scene with `graphic` standing at `at` as well.
    fn with(mut self, at: (u16, u16), graphic: Graphic) -> Self {
        self.items.push(GroundItem {
            at: Point::new(at.0, at.1, 0),
            graphic,
            hue: Hue::NONE,
        });
        self
    }
}

/// The ground everything stands on: flat, at `z = 0`, 128 tiles square.
///
/// Wide enough to hold [`CENTRE`] with room for a camera's bounds on every side.
/// The land graphic is `0`, which the scenes' `tiledata` gives no flags, so the
/// ground itself neither burns nor stops anything.
fn ground() -> Map {
    Map::from_blocks(16, 16, |_, _| LandCell { tile: 0, z: 0 })
}

/// The flags the graphics above carry, and no others.
fn tiledata() -> TileData {
    let mut tiledata = TileData::empty();
    let mut set = |graphic: Graphic, flags: u64, height: u8| {
        tiledata.set_static_tile(
            graphic.0,
            StaticTile {
                flags: TileFlags::new(flags),
                height,
                ..StaticTile::default()
            },
        );
    };
    set(WALL, TileFlags::NO_SHOOT, WALL_HEIGHT);
    set(PANE, TileFlags::WINDOW, WALL_HEIGHT);
    set(DOOR_SHUT, TileFlags::NO_SHOOT, WALL_HEIGHT);
    // The open leaf: `BLOCK` because you cannot walk through the leaf itself,
    // and *not* `NO_SHOOT`, because you can see and shoot past it. The two flags
    // are different questions and this is the tile where that matters most —
    // see `docs/lighting.md`, decision 4.
    set(DOOR_OPEN, TileFlags::BLOCK, WALL_HEIGHT);
    set(TORCH, TileFlags::LIGHT_SOURCE, 0);
    tiledata
}

/// An empty world: ground, a camera on [`CENTRE`] and nothing standing anywhere.
///
/// Every scene below is this plus what it puts in it, which keeps the ground,
/// the flags and the camera stated once.
pub fn empty(name: &'static str) -> Scene {
    Scene {
        name,
        map: ground(),
        tiledata: tiledata(),
        items: Vec::new(),
        camera: Camera::new(Point::new(CENTRE.0, CENTRE.1, 0), 800, 600),
        cutaway: Cutaway::OPEN,
    }
}

/// The tiles of a closed ring of wall around [`CENTRE`], [`ROOM_HALF`] out.
///
/// A ring and not four walls: decision 3's occluder is a whole tile, so the
/// corners are tiles like any other and the ring closes by construction. That is
/// the property the room is here to demonstrate.
pub fn room_wall_tiles() -> Vec<(u16, u16)> {
    let (cx, cy) = CENTRE;
    let mut tiles = Vec::new();
    for x in cx - ROOM_HALF..=cx + ROOM_HALF {
        for y in cy - ROOM_HALF..=cy + ROOM_HALF {
            let edge =
                x == cx - ROOM_HALF || x == cx + ROOM_HALF || y == cy - ROOM_HALF || y == cy + ROOM_HALF;
            if edge {
                tiles.push((x, y));
            }
        }
    }
    tiles
}

/// Where a room's door is: the middle of its south wall.
pub const DOORWAY: (u16, u16) = (CENTRE.0, CENTRE.1 + ROOM_HALF);

/// A shut room with a torch in the middle of it.
///
/// The base case, and the one the whole pass was built for: inside is lit,
/// outside is exactly the ambient, and the wall's own tiles are the brightest
/// thing in the picture.
pub fn room() -> Scene {
    let mut scene = empty("a shut room with a torch in it");
    for tile in room_wall_tiles() {
        scene = scene.with(tile, WALL);
    }
    scene.with(CENTRE, TORCH)
}

/// The same room with the door on its south wall shut.
///
/// Identical to [`room`] in what it does to the light, and that is the point:
/// this is the *before* of the pair, and a difference between the two would mean
/// a shut door is not a wall.
pub fn room_with_shut_door() -> Scene {
    let mut scene = empty("a room whose door is shut");
    for tile in room_wall_tiles() {
        scene = scene.with(tile, if tile == DOORWAY { DOOR_SHUT } else { WALL });
    }
    scene.with(CENTRE, TORCH)
}

/// And with the door open: the same room, one graphic different.
///
/// What should come out is a fan of light on the ground south of the doorway and
/// darkness everywhere else outside — the shape of decision 11, and the picture a
/// person actually asked for.
pub fn room_with_open_door() -> Scene {
    let mut scene = empty("a room whose door is open");
    for tile in room_wall_tiles() {
        scene = scene.with(tile, if tile == DOORWAY { DOOR_OPEN } else { WALL });
    }
    scene.with(CENTRE, TORCH)
}

/// A room with a pane of glass where its door would be.
///
/// Today the pane stops light exactly as the wall does, because `WINDOW` is what
/// the reference's line of sight tests alongside `NO_SHOOT`. The scene exists to
/// hold that fact still while the rule changes: when a pane starts to dim rather
/// than stop, this is where the number is looked at.
pub fn room_with_window() -> Scene {
    let mut scene = empty("a room with a window in it");
    for tile in room_wall_tiles() {
        scene = scene.with(tile, if tile == DOORWAY { PANE } else { WALL });
    }
    scene.with(CENTRE, TORCH)
}

/// A straight wall with a torch standing on one of its tiles: a sconce.
///
/// The known-wrong case. Decision 3 exempts a light's own tile from occluding
/// it, which is right for a torch in a doorway and wrong for a sconce: both
/// sides of the wall are lit. There is no facing in `tiledata.mul` to fix it
/// with, so the scene pins the current behaviour and will fail the day somebody
/// invents one — which is what a backlog entry with a test looks like.
pub fn sconce_on_wall() -> Scene {
    let (cx, cy) = CENTRE;
    let mut scene = empty("a sconce on a straight wall");
    for x in cx - ROOM_HALF..=cx + ROOM_HALF {
        scene = scene.with((x, cy), WALL);
    }
    scene.with(CENTRE, TORCH)
}

/// How far below the street the cellar's torch burns, in `z` units.
///
/// Seven tiles' worth, at eleven units a tile: past a torch's six-tile reach
/// even after the half-tile the flame is lifted by, so this scene is about the
/// *distance* being three-dimensional and not about anything occluding.
pub const CELLAR_DEPTH: i8 = -(7 * 11);

/// A torch four tiles below an empty street.
///
/// Nothing stands anywhere: what keeps the street dark is only that `z` is
/// divided into tiles and the flame is therefore four tiles away. Decision 7, on
/// its own, with the occluders removed as a variable.
pub fn cellar_under_street() -> Scene {
    let mut scene = empty("a torch in a cellar under an empty street");
    scene.items.push(GroundItem {
        at: Point::new(CENTRE.0, CENTRE.1, CELLAR_DEPTH),
        graphic: TORCH,
        hue: Hue::NONE,
    });
    scene
}

/// Two wall tiles touching at one corner, with a torch on the far diagonal.
///
/// The gap the backlog names: the ray is Chebyshev-sampled one cell a step, so a
/// ray running almost exactly along a diagonal passes between two tiles that
/// touch only at their corners. Real walls are rows and this has not been seen in
/// a house — the scene is here so that the day it is fixed, something says the
/// fix worked.
pub fn diagonal_gap() -> Scene {
    let (cx, cy) = CENTRE;
    empty("two walls touching at a corner")
        .with((cx + 1, cy), WALL)
        .with((cx, cy + 1), WALL)
        .with((cx + 2, cy + 2), TORCH)
}

/// Every scene above, for a test that wants to sweep them.
pub fn all() -> Vec<Scene> {
    vec![
        room(),
        room_with_shut_door(),
        room_with_open_door(),
        room_with_window(),
        sconce_on_wall(),
        cellar_under_street(),
        diagonal_gap(),
    ]
}
