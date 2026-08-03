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

use openshard_protocol::direction::Direction;
use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::Point;
use openshard_uofiles::map::{LandCell, Map};
use openshard_uofiles::tiledata::{StaticTile, TileData, TileFlags};

use crate::atlas::StaticAtlas;
use crate::camera::Camera;
use crate::cutaway::Cutaway;
use crate::items::GroundItem;
use crate::light::{self, Lighting, Sun};

/// A wall: what stops an arrow, and therefore what stops light. Twenty `z` units
/// tall, which is a storey.
pub const WALL: Graphic = Graphic(0x0006);

/// The same wall standing on its tiles' **east** edge, for the scene that needs
/// a house to turn a corner.
///
/// A second graphic and not a second flag, because a face is a property of a
/// *picture*: [`crate::facing`] measures it off the art, an atlas holds one
/// answer per graphic, and a run going the other way is therefore a different
/// graphic in the client's files exactly as it is here. Britain's own corner is
/// built that way — `0x0037` along the south of its row, `0x0035` up the east of
/// its column.
pub const WALL_EAST: Graphic = Graphic(0x000B);

/// And the piece where the two runs meet, whose art names no edge at all.
///
/// **The whole point of it is that it is faceless.** A corner is two faces in one
/// picture, and `facing::face_of` refuses rather than guesses — so the occlusion
/// grid falls back to [`crate::occlusion::EDGE_ANY`], the whole-tile answer, and
/// the sprite falls back to [`crate::place::Stance::Upright`]. Measured on the
/// install: Britain's corner at `(1441, 1692)` is graphic `0x0033`, and the atlas
/// answers `None` for it. This scene reproduces that by leaving the graphic out
/// of its own atlas, which is the same `None` by the same path — see
/// `occlusion::collect`, where the face of a graphic the atlas does not hold and
/// the face of a picture the detector refused are one expression.
pub const CORNER: Graphic = Graphic(0x000C);

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

/// A roof tile. What makes a room a room as far as the *sun* is concerned: four
/// walls and no roof is a courtyard, and the sun floods it — which is correct,
/// and is why the sunlit scenes have one and the firelit ones do not.
pub const ROOF: Graphic = Graphic(0x000A);

/// How high a room's roof sits: on top of its walls.
pub const ROOF_Z: i8 = WALL_HEIGHT as i8;

/// And how thick it is. Anything above nothing: what matters is that a ray
/// climbing out of the room passes through the span, not how deep it is.
pub const ROOF_THICKNESS: u8 = 5;

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
    /// The sky, where the scene has one. `None` is night — which is what every
    /// scene about firelight wants, because a sun would put a second term in
    /// every brightness they assert on.
    pub sun: Option<Sun>,
    /// Where the character stands and which way they face, when they are the one
    /// holding the light.
    ///
    /// Not a [`light::Light`] in the item list, and not one stored ready-made:
    /// no walk of a map could find this flame — nothing on the wire says a hand
    /// is carrying anything — and the flicker has to be the same instant every
    /// other flame in the frame is at, which is [`Scene::lighting`]'s argument
    /// and not this field's. See [`light::carried`] and [`Lighting::hold`].
    pub carried: Option<(Point, Direction)>,
    /// The pictures, where the scene is about which *edge* a wall stands on.
    ///
    /// `None` for almost every scene, and that is not a gap: these scenes have no
    /// art at all — a `Map` of flat ground, a `TileData` where a few graphics
    /// have flags, and a list of items — and without pictures every occluder is
    /// the whole tile decision 3 made it. Which is the answer those scenes were
    /// written against and still assert.
    ///
    /// A scene that *is* about facing packs one synthetic silhouette per graphic
    /// with [`facing::silhouette`](crate::facing::silhouette) and hands it here.
    /// See [`lamp_against_a_wall`], which is the case a whole-tile occluder gets
    /// wrong: a light mounted on a house, shining along the street it hangs over.
    pub art: Option<StaticAtlas>,
}

impl Scene {
    /// This scene's flames and occluders, at night.
    ///
    /// `time` is the flicker's, and every test passes `0.0`: a flame's
    /// brightness swings by a tenth and an assertion about a leak should not
    /// depend on which tenth of a second it was asked in.
    pub fn lighting(&self, time: f32) -> Lighting {
        let mut lighting = light::collect(
            &self.map,
            &self.items,
            &self.camera,
            &self.tiledata,
            &self.cutaway,
            match self.sun {
                // A sunlit scene is not lit by the night ambient: the sun is what
                // brightens it, and an ambient that already did would hide every
                // shadow the sun casts.
                Some(_) => light::SKYLIGHT,
                None => light::NIGHT,
            },
            time,
            self.art.as_ref(),
        );
        lighting.sun = self.sun;
        if let Some((at, facing)) = self.carried {
            lighting.hold(light::carried(at, facing, time));
        }
        lighting
    }

    /// The scene with `graphic` standing at `at`, on the ground.
    fn with(self, at: (u16, u16), graphic: Graphic) -> Self {
        self.with_at(at, 0, graphic)
    }

    /// The same room with a roof on: [`ROOF`] over every tile inside the ring
    /// of wall, at the top of it.
    ///
    /// Over the *inside* and not over the wall tiles, which is how a real house
    /// is built and is what leaves the ring of wall the only tile a roofed room
    /// has that the sky can still reach. Decision 1 of `docs/lighting_world.md`
    /// is measured on the difference.
    fn roofed(mut self) -> Self {
        let (cx, cy) = CENTRE;
        for x in cx - ROOM_HALF + 1..=cx + ROOM_HALF - 1 {
            for y in cy - ROOM_HALF + 1..=cy + ROOM_HALF - 1 {
                self = self.with_at((x, y), ROOF_Z, ROOF);
            }
        }
        self
    }

    /// And with it standing at a height — a roof, or a storey above one.
    fn with_at(mut self, at: (u16, u16), z: i8, graphic: Graphic) -> Self {
        self.items.push(GroundItem {
            at: Point::new(at.0, at.1, z),
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
    set(WALL_EAST, TileFlags::NO_SHOOT, WALL_HEIGHT);
    set(CORNER, TileFlags::NO_SHOOT, WALL_HEIGHT);
    set(PANE, TileFlags::WINDOW, WALL_HEIGHT);
    set(DOOR_SHUT, TileFlags::NO_SHOOT, WALL_HEIGHT);
    // The open leaf: `BLOCK` because you cannot walk through the leaf itself,
    // and *not* `NO_SHOOT`, because you can see and shoot past it. The two flags
    // are different questions and this is the tile where that matters most —
    // see `docs/lighting.md`, decision 4.
    set(DOOR_OPEN, TileFlags::BLOCK, WALL_HEIGHT);
    set(TORCH, TileFlags::LIGHT_SOURCE, 0);
    set(ROOF, TileFlags::NO_SHOOT, ROOF_THICKNESS);
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
        sun: None,
        carried: None,
        art: None,
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

/// One torch on open ground, and nothing else in the world at all.
///
/// **The simplest scene there is, and the one to look at first.** No wall, no
/// roof, no second flame, no sun: what a picture of this scene shows is the
/// falloff and nothing else, so a pool that is not a circle here is not a circle
/// anywhere. Every other scene adds exactly one thing to it.
///
/// It exists because every scene before it had a room around it. A shape that is
/// wrong in the middle of a pool — flat where it should fall away — is invisible
/// against four walls and a shadow, and it was: the flat disc
/// `docs/lighting.md`'s "where the next session starts" describes was in every
/// one of these scenes and none of them was looking at it.
pub fn torch_on_open_ground() -> Scene {
    empty("one torch on open ground").with(CENTRE, TORCH)
}

/// A straight wall with a torch two tiles in front of it: one shadow, and
/// nothing else.
///
/// The second-simplest scene, and the one shadows are read in. The wall runs east
/// to west on its tiles' **south** side — where the client draws nearly all of
/// them — and the torch stands north of it, so the shadow is the ground south of
/// the run and the lit side is everything north.
///
/// Long enough (nine tiles) that the pool does not simply reach round both ends,
/// which is what makes the shadow a band rather than a notch. The art is the
/// synthetic silhouette [`wall_with_a_torch_beside_it`] uses, for the same
/// reason: without it nothing names an edge and every occluder is a whole tile.
pub fn torch_before_a_wall() -> Scene {
    let (cx, cy) = CENTRE;
    let mut scene = empty("a torch two tiles in front of a straight wall");
    for x in cx - 4..=cx + 4 {
        scene = scene.with((x, cy), WALL);
    }
    scene.art = Some(south_faced_wall());
    scene.with((cx, cy - 2), TORCH)
}

/// How many tiles of wall [`wall_run_lit_from_along_it`] has. Four, so that the
/// picture holds three seams and the near end and the far end of the run are lit
/// differently enough to tell apart.
pub const RUN: u16 = 4;

/// A run of wall with a lamp standing in front of it and off to one end, so that
/// the light runs **along** the face rather than into it.
///
/// **The scene a seam is read in**, and the arrangement matters far more than the
/// length. A wall's face lies *on* the panel it is the face of — decision 16 — so
/// a ray leaving a pixel of the face towards a lamp that is only half a tile out
/// from the wall crosses the wall's own plane almost immediately, and *where* it
/// crosses is a little further along the run than the pixel is. For the pixels
/// near the far end of each tile that crossing point lands in the **next** tile,
/// whose panel is a wall — so the ray is stopped by a wall it is standing on the
/// face of, and every tile of the run gets a dark stroke down its last few
/// hundredths. Reported from the client as a strange artefact between walls.
///
/// How wide the stroke is is the ratio of the two offsets: a lamp far along the
/// run and close in to the wall draws a wide one, and a lamp straight out in
/// front of the wall draws none at all — which is exactly the difference the
/// report described, and the reason this scene is not the perpendicular one.
pub fn wall_run_lit_from_along_it() -> Scene {
    let (cx, cy) = CENTRE;
    let mut scene = empty("a wall run with a lamp along it");
    for x in cx..cx + RUN {
        scene = scene.with((x, cy), WALL);
    }
    scene.art = Some(south_faced_wall());
    // In front of the wall by one tile — the flame lands half a tile out from the
    // face — and a tile past the far end of the run.
    scene.with((cx + RUN, cy + 1), TORCH)
}

/// The picture a scene hands the occlusion grid when it wants a wall that stands
/// on one *side* of its tile rather than filling it.
///
/// A synthetic silhouette and not a client graphic: this workspace ships no art,
/// and a hand-drawn parallelogram is the projection and nothing else — so what a
/// scene using it demonstrates is the grid's edge mask, with the art table's own
/// coverage taken out of the question. See [`crate::facing`].
fn south_faced_wall() -> StaticAtlas {
    StaticAtlas::pack([(
        WALL,
        crate::facing::silhouette(crate::facing::Face::South, WALL_HEIGHT.into()),
    )])
    .expect("one silhouette fits")
}

/// The art for a house that turns a corner: a south-faced run, an east-faced run
/// and **nothing at all for the corner between them**.
///
/// The absence is the fixture. [`CORNER`] says why it is an absence rather than a
/// picture the detector refuses: the two arrive at the same `None` through the
/// same expression in `occlusion::collect`, and a synthetic silhouette that a
/// detector was *meant* to refuse would be a fixture anchored to the thing the
/// detector might one day read — see the corner entry in `docs/lighting.md`'s
/// backlog, which is what would then have to change.
fn corner_of_a_house() -> StaticAtlas {
    StaticAtlas::pack([
        (
            WALL,
            crate::facing::silhouette(crate::facing::Face::South, WALL_HEIGHT.into()),
        ),
        (
            WALL_EAST,
            crate::facing::silhouette(crate::facing::Face::East, WALL_HEIGHT.into()),
        ),
    ])
    .expect("two silhouettes fit")
}

/// How long each of [`house_corner`]'s two runs is, in tiles past the corner.
///
/// Three, which is the shortest that makes the point: a ray leaking through the
/// corner has to arrive from *behind* the run rather than round the end of it,
/// and a one-tile run is reached round in the picture as well as through.
pub const CORNER_RUN: u16 = 3;

/// **The corner of a house, with a lamp in the street outside it.** Britain at
/// `(1441, 1692)`, built.
///
/// Two runs of wall meeting at a right angle — one along the south of its row,
/// one up the east of its column — with the piece where they meet a graphic whose
/// art names no edge, and a flame on the tile diagonally outside the corner. That
/// arrangement is not a curiosity: it is how every building in the client's own
/// map is put together, and it is where two of the three answers this pass has
/// for an occluder meet.
///
/// # What it is for
///
/// A ray from inside the house to the lamp used to arrive at **85% strength**,
/// and the path is worth being able to recognise again. It enters the last wall
/// tile of the south run through that tile's *north* side and leaves it
/// *eastwards* — so it never crosses the panel that tile stands on, which is
/// correct and is what lets light run along a street. It then clips the corner
/// tile over a hair's length; that tile is faceless, so it is a **body** and what
/// it stops is scaled by how far the ray ran inside it, which for a sliver is
/// nothing. Two cells, both holding wall, and the ray passes both — decision 18's
/// spoke, arriving where a run of wall has to turn.
///
/// The flame is placed south of the corner and the leak is read to its
/// north-west, so the ray runs the diagonal: it is the 45° direction the report
/// came in as, and it is the direction that maximises the sliver.
///
/// `docs/lighting.md`, decision 24.
pub fn house_corner() -> Scene {
    let (cx, cy) = CENTRE;
    let mut scene = empty("the corner of a house with a lamp outside it");
    // The south run, west of the corner: panels on the `y1` line, so the house is
    // north of them.
    for x in cx - CORNER_RUN..cx {
        scene = scene.with((x, cy), WALL);
    }
    // The east run, north of the corner: panels on the `x1` line, so the house is
    // west of them.
    for y in cy - CORNER_RUN..cy {
        scene = scene.with((cx, y), WALL_EAST);
    }
    scene = scene.with(CENTRE, CORNER);
    scene.art = Some(corner_of_a_house());
    // Outside, in the street, on the tile diagonally off the corner.
    scene.with((cx, cy + 1), TORCH)
}

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

/// A straight run of wall with a torch at one end of it, and the art to say
/// which side of its tiles the wall stands on.
///
/// **The scene decision 3 could not have.** An occluder used to be the whole
/// tile, so light could not travel *alongside* a wall — a lamp mounted on a
/// house was shadowed by the next tile of its own wall, and the street it hung
/// over stayed dark in a band nothing visible was casting. Step 15 measures which
/// edge a wall stands on, and this is the pair of rays that says the grid now
/// uses it:
///
/// - **Along** the wall — the torch and the lit spot both on the wall's own row,
///   so the ray enters each tile through one side and leaves through the other
///   without ever crossing the face. Nothing should stop it.
/// - **Across** it — the torch north of the row and the spot south of it, so the
///   ray goes through the face. Everything should stop it.
///
/// One scene and not two, because the two rays differ in nothing but direction:
/// a mistake that let the second through would be invisible in a scene that only
/// asked the first.
///
/// The wall is on its tiles' **south** side, which is where the client draws
/// nearly all of them — see `facing`'s own sweep. The art is a synthetic
/// silhouette rather than a real graphic, for the reason every scene here is
/// built rather than loaded: a hand-drawn parallelogram is the projection and
/// nothing else, so a failure prints a room rather than a coordinate.
pub fn wall_with_a_torch_beside_it() -> Scene {
    let (cx, cy) = CENTRE;
    let mut scene = empty("a straight wall with a torch at one end");
    for x in cx - 3..=cx + 3 {
        scene = scene.with((x, cy), WALL);
    }
    scene.art = Some(south_faced_wall());
    // On the wall's own row, at its east end: the sconce a house carries. Its own
    // tile is exempt from shadowing it, as every flame's is, so what this scene
    // is about is the *other six* tiles of the same wall.
    scene.with((cx + 3, cy), TORCH)
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

/// A character in the middle of a shut room, holding a light and facing east.
///
/// Nothing burns in this room. The only flame in it is the one in the hand, so
/// every bright pixel of the scene is the beam's and there is no pool to confuse
/// it with — which is what makes the four assertions worth making: the floor
/// ahead is lit, the floor behind is exactly the ambient, the east wall's face is
/// lit, and the west wall's is not.
///
/// East on purpose. It is [`Direction::East`]'s `(1, 0)`, the one step whose
/// beam runs along a map axis, so a tile is either in front of the character or
/// behind them and no assertion has to reason about a diagonal.
pub fn lantern_in_a_room() -> Scene {
    let mut scene = empty("a character holding a light in a shut room");
    for tile in room_wall_tiles() {
        scene = scene.with(tile, WALL);
    }
    scene.carried = Some((Point::new(CENTRE.0, CENTRE.1, 0), Direction::East));
    scene
}

/// The sun these scenes stand under: the client's own, so that a scene is a
/// picture of what a player would see rather than of a sky invented for a test.
///
/// [`light::midday`] is towards `+x` at 45°, which is what the assertions about
/// shadow length are stated in terms of — and a test says so, so that changing
/// the client's sun fails loudly here instead of quietly weakening a scene.
pub fn noon() -> Sun {
    light::midday()
}

/// One wall tile in the open, under a sun: the shadow, with nothing else in the
/// frame to explain it.
///
/// No torch at all. What lights this scene is the sky and the sun, and the only
/// dark thing in it is the ground the wall keeps the sun off — so a shadow that
/// is in the wrong place, or missing, or on both sides, has nothing else to be
/// blamed on.
pub fn wall_in_the_sun() -> Scene {
    let mut scene = empty("a wall in the open, under a sun");
    scene.sun = Some(noon());
    scene.with(CENTRE, WALL)
}

/// Where a sunlit room's window is: the middle of the wall the sun is on.
///
/// The east wall, because [`noon`] shines towards `+x` and a window in any other
/// wall is a window the sun does not enter through — which is a true fact about
/// windows and a useless scene.
pub const WINDOW_TILE: (u16, u16) = (CENTRE.0 + ROOM_HALF, CENTRE.1);

/// A roofed room with a window in its sunward wall: the patch of light on the
/// floor.
///
/// The picture the whole of decision 12 is for, and it needs the roof: a room
/// with four walls and open sky is a courtyard, and at 45° the sun clears a
/// two-tile wall in two tiles and floods the floor. With a roof on, the only way
/// in is the pane — so what appears inside is a band along the sunward wall and
/// shadow everywhere else.
pub fn sunlit_room_with_window() -> Scene {
    let mut scene = empty("a roofed, sunlit room with a window");
    scene.sun = Some(noon());
    for tile in room_wall_tiles() {
        scene = scene.with(tile, if tile == WINDOW_TILE { PANE } else { WALL });
    }
    scene.roofed()
}

/// A shut, roofed house in the daylight, with nothing burning in it.
///
/// The base case of `docs/lighting_world.md`'s decision 1, and the one the whole
/// step is judged on: nothing here is dark because a flame failed to reach it —
/// what is dark is dark because a roof is between the floor and the sky. The
/// pair it is read against is [`roofed_room_with_open_door`], which differs by
/// one graphic.
pub fn roofed_room() -> Scene {
    let mut scene = empty("a shut, roofed house at noon");
    scene.sun = Some(noon());
    for tile in room_wall_tiles() {
        scene = scene.with(tile, WALL);
    }
    scene.roofed()
}

/// The same house with its door standing open: the threshold.
///
/// The open leaf carries no `NO_SHOOT` — see [`DOOR_OPEN`] — so the doorway tile
/// is the one tile of the ring that the sky reaches, and decision 2's blur is
/// what turns that into a threshold brighter than the room and darker than the
/// street. Without the blur the two are one step at the wall line, which is the
/// artefact this whole track exists to remove.
pub fn roofed_room_with_open_door() -> Scene {
    let mut scene = empty("a roofed house at noon with its door open");
    scene.sun = Some(noon());
    for tile in room_wall_tiles() {
        scene = scene.with(tile, if tile == DOORWAY { DOOR_OPEN } else { WALL });
    }
    scene.roofed()
}

/// A roofed house with a window in one wall, and nothing burning in it.
///
/// [`sunlit_room_with_window`] without the sun's own beam: the same glazed wall,
/// read for what it does to the *sky* the room gets rather than to the patch of
/// light on its floor. The crude half of decision 14 — a pane passes its share
/// of the sky where a wall passes none — and the scene it is measured against is
/// [`roofed_room`], which differs by that one graphic.
pub fn roofed_room_with_window() -> Scene {
    let mut scene = empty("a roofed house at noon with a window");
    scene.sun = Some(noon());
    for tile in room_wall_tiles() {
        scene = scene.with(tile, if tile == WINDOW_TILE { PANE } else { WALL });
    }
    scene.roofed()
}

/// Every scene above, for a test that wants to sweep them.
pub fn all() -> Vec<Scene> {
    vec![
        torch_on_open_ground(),
        torch_before_a_wall(),
        wall_run_lit_from_along_it(),
        house_corner(),
        room(),
        room_with_shut_door(),
        room_with_open_door(),
        room_with_window(),
        sconce_on_wall(),
        lantern_in_a_room(),
        cellar_under_street(),
        diagonal_gap(),
        wall_in_the_sun(),
        sunlit_room_with_window(),
        roofed_room(),
        roofed_room_with_open_door(),
        roofed_room_with_window(),
    ]
}
