//! The grid, kept per block instead of rebuilt per frame.
//!
//! `docs/lighting.md`'s decision 30.4 and step 21.5. Everything a [`Builder`]
//! holds is a fact about the **map** — decision 33 moved the frame's [`Cutaway`]
//! down to [`Builder::finish`], so what is above that line does not depend on
//! where the player is standing, on which way the camera points, or on anything
//! that changes between two frames. A fact about the map can be built once.
//!
//! # Why the block, and why there is no band
//!
//! The map's own unit is the 8×8 block: the statics of one are a contiguous slice
//! ([`Map::statics_in_block`]) and a static never stands outside the block it is
//! stored in, so a block's surfaces and its sky are entirely its own. Decision
//! 30.4 as written also wanted a *storey band* in the key, because the cutaway
//! used to be applied at the map walk and a builder was therefore one frame's;
//! decision 33 took that away, so the key is the block and nothing else.
//!
//! # What is baked and what is not
//!
//! Baked: every static of the block, through the same
//! [`place`](super::place) the uncached walk uses — the sky it takes and the
//! surfaces it is, refused only above the draw ceiling, which is a fact about the
//! static.
//!
//! Not baked, and each for a stated reason:
//!
//! - **The frame's [`Cutaway`]**, which is [`Builder::finish`]'s and applies to a
//!   packed list.
//! - **The server's ground items.** A door is one of them and a door changing its
//!   graphic changes the one occluder a player watches change. There are a
//!   handful in a frame.
//! - **The blur of the sky field.** It is a 3×3 over the *frame's* rectangle, so
//!   a block would need a one-tile apron to be blurred on its own. 0.145ms of a
//!   1.15ms build, and staying where it is.
//!
//! # The stale answer this could give, and what stops it
//!
//! A surface is derived from the art through [`StaticAtlas`], and **an atlas
//! grows**: a graphic the camera has not reached is not in it yet, so a block
//! baked before that graphic was packed holds the whole-tile fallback where the
//! atlas can now name a face. Nothing about the baked block would ever say so. So
//! a [`Bake`] carries the [`StaticAtlas::revision`] it was built under and throws
//! everything away when it moves — which is a handful of times in a session and
//! never in a steady frame.
//!
//! The other half of that is the map itself: a [`Bake`] is one map's, and a
//! caller that loads a second facet builds a second bake. Nothing here can check
//! it, and the client has one map.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use openshard_protocol::wire::Graphic;
use openshard_uofiles::map::{BLOCK_SIZE, Map};
use openshard_uofiles::tiledata::TileData;

use super::{Builder, NO_SURFACE, Occlusion, SKY_OPEN, Surface};
use crate::atlas::StaticAtlas;
use crate::camera::TileBounds;
use crate::cutaway::Cutaway;
use crate::items::GroundItem;

/// How many tiles a block is on a side — the map's own number, named here
/// because every index in this module is `cell = y * SIDE + x` over it.
const SIDE: i32 = BLOCK_SIZE as i32;

/// How many tiles a block holds.
const CELLS: usize = (SIDE * SIDE) as usize;

/// How many blocks a [`Bake`] keeps before it lets the coldest ones go.
///
/// A widest-zoom frame is about 550 blocks, so this is several frames' worth of
/// walking in every direction: a player crossing a city keeps the block behind
/// them long enough to turn round, and a `/teleport` across the facet does not
/// grow the cache without bound. What one costs is its surfaces — a city block
/// runs to a few dozen — so the whole cache is a few megabytes at this size.
const KEEP_BLOCKS: usize = 4096;

/// One block's contribution to a grid, derived from the map alone.
///
/// Not an [`Occlusion`] and not a [`Builder`]: it is what a builder *would have
/// been given* by that block, in a form that can be handed to any builder whose
/// rectangle overlaps it. A cell index rather than a tile coordinate, so the same
/// bytes serve a frame at any offset.
#[derive(Clone, PartialEq, Debug)]
pub struct Baked {
    /// Every surface the block stands, as `(cell, surface)` with the cell being
    /// `y * SIDE + x` inside the block — in the order the map walk found them,
    /// which is the order a tile's surfaces have to come out in.
    surfaces: Vec<(u8, Surface)>,
    /// How much of the sky each of the block's tiles can see, **before the
    /// blur**, which is the frame's.
    sky: [u8; CELLS],
    /// What the block's own build refused for want of room on a tile — see
    /// [`super::MAX_SURFACES_PER_TILE`].
    dropped: usize,
}

impl Baked {
    /// Build one block: every static stored in it, through the walk's own
    /// [`place`](super::place).
    ///
    /// The builder used is over the block's eight-by-eight rectangle and nothing
    /// wider, so its row-major index *is* the cell index, which is what makes the
    /// read-out below a copy rather than an arithmetic.
    fn of(map: &Map, tiledata: &TileData, atlas: Option<&StaticAtlas>, block_x: u32, block_y: u32) -> Self {
        let (origin_x, origin_y) = origin(block_x, block_y);
        let mut grid = Builder::new(TileBounds {
            min_x: origin_x,
            max_x: origin_x + SIDE - 1,
            min_y: origin_y,
            max_y: origin_y + SIDE - 1,
        });
        for item in map.statics_in_block(block_x, block_y) {
            super::place(
                &mut grid,
                map,
                tiledata,
                atlas,
                item.x,
                item.y,
                item.z,
                Graphic(item.tile),
            );
        }

        // The same read-out `Builder::finish` makes and for the same reason: the
        // list is built by pushing at the front, so walking it hands back the
        // newest first, and reversing what one tile contributed is what puts the
        // surfaces back in the order the map walk found them.
        let mut surfaces = Vec::with_capacity(grid.arena.len());
        for cell in 0..CELLS {
            let first = surfaces.len();
            let mut at = grid.heads[cell];
            while at != NO_SURFACE {
                let (surface, next) = grid.arena[at as usize];
                surfaces.push((cell as u8, surface));
                at = next;
            }
            surfaces[first..].reverse();
        }

        let mut sky = [SKY_OPEN; CELLS];
        sky.copy_from_slice(&grid.sky);
        Self {
            surfaces,
            sky,
            dropped: grid.dropped,
        }
    }
}

/// Where a block's north-west tile is.
fn origin(block_x: u32, block_y: u32) -> (i32, i32) {
    (block_x as i32 * SIDE, block_y as i32 * SIDE)
}

/// Where a cell of a block sits on the map.
fn tile_of(origin: (i32, i32), cell: usize) -> (i32, i32) {
    let cell = cell as i32;
    (origin.0 + cell % SIDE, origin.1 + cell / SIDE)
}

impl Builder {
    /// Lay one baked block's surfaces and sky into this frame's grid.
    ///
    /// A tile outside the frame's rectangle is dropped, exactly as
    /// [`Builder::add`] drops one: a block on the rim of a frame is mostly inside
    /// it and the cache holds the whole of it either way.
    ///
    /// The sky is **assigned and not multiplied in**, and that is a claim about
    /// the order rather than an approximation. A block's tiles are its own, no
    /// two blocks share one, and this runs before any ground item has shaded
    /// anything — so the value a tile has here is the value the walk would have
    /// left it at, to the byte, rounding included.
    fn paste(&mut self, block_x: u32, block_y: u32, baked: &Baked) {
        let origin = origin(block_x, block_y);
        for (cell, sky) in baked.sky.iter().enumerate() {
            let (x, y) = tile_of(origin, cell);
            if let Some(index) = self.index(x, y) {
                self.sky[index] = *sky;
            }
        }
        for (cell, surface) in &baked.surfaces {
            let (x, y) = tile_of(origin, usize::from(*cell));
            if let Some(index) = self.index(x, y) {
                self.push(index, *surface);
            }
        }
        // Counted whole rather than per tile. A block that overflowed a tile did
        // so whether or not that tile is inside this frame's rectangle, and the
        // number is a diagnostic about the map — the worst tile in Britain holds
        // twenty-one surfaces against a cap of two hundred and fifty-five.
        self.dropped += baked.dropped;
    }
}

/// What the shapes in a cache were derived from, so that a change to them can
/// throw it away.
///
/// `None` is a caller with no atlas at all — a built scene, a test — which is a
/// state of its own and not "revision zero": a bake handed an atlas on one frame
/// and none on the next has to let go of what the atlas said.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct Stamp(Option<u64>);

impl Stamp {
    fn of(atlas: Option<&StaticAtlas>) -> Self {
        Self(atlas.map(StaticAtlas::revision))
    }
}

/// One block, and when it was last wanted.
#[derive(Clone, PartialEq, Debug)]
struct Cached {
    baked: Baked,
    /// The value of [`Bake::frame`] the last time a frame asked for it.
    used: u64,
}

/// The blocks a client has already derived, kept between frames.
///
/// Owned by whoever draws — one per map — and handed to
/// [`bake::collect`](collect) as the only difference between a cached frame and
/// an uncached one. A caller that has none uses
/// [`occlusion::collect`](super::collect), which is the same grid built from
/// nothing and is what the tests compare against.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Bake {
    /// What the atlas said when these blocks were derived. A change clears them
    /// all — see this module's header.
    stamp: Stamp,
    /// Keyed by `(block_x, block_y)` packed into a word. A facet is at most 896
    /// blocks on a side, so sixteen bits each is room to spare.
    blocks: HashMap<u32, Cached>,
    /// How many frames this bake has served, which is what [`Cached::used`] is
    /// counted in.
    frame: u64,
    /// How many blocks were served from the cache, and how many were built. The
    /// companion the measurement needs: a cache that is never hit reads exactly
    /// like a cache that works.
    hits: usize,
    misses: usize,
}

impl Bake {
    /// A bake holding nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many blocks are kept right now.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Whether it is holding nothing at all.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// How many blocks have been served from the cache, and how many built.
    ///
    /// Cumulative over the life of the bake. **Print it wherever the build cost
    /// is printed**: the two readings a cache can give — every block served and
    /// every block rebuilt — cost the same to take and look identical in a
    /// frame's total until they are put side by side.
    pub fn served(&self) -> (usize, usize) {
        (self.hits, self.misses)
    }

    /// Start a frame: let go of everything if the art has moved under us.
    fn begin(&mut self, atlas: Option<&StaticAtlas>) {
        let stamp = Stamp::of(atlas);
        if stamp != self.stamp {
            self.blocks.clear();
            self.stamp = stamp;
        }
        self.frame += 1;
    }

    /// One block, built if this is the first frame to want it.
    fn block(
        &mut self,
        map: &Map,
        tiledata: &TileData,
        atlas: Option<&StaticAtlas>,
        block_x: u32,
        block_y: u32,
    ) -> &Baked {
        let frame = self.frame;
        match self.blocks.entry(block_x << 16 | block_y) {
            Entry::Occupied(entry) => {
                self.hits += 1;
                let cached = entry.into_mut();
                cached.used = frame;
                &cached.baked
            }
            Entry::Vacant(entry) => {
                self.misses += 1;
                &entry
                    .insert(Cached {
                        baked: Baked::of(map, tiledata, atlas, block_x, block_y),
                        used: frame,
                    })
                    .baked
            }
        }
    }

    /// Let the coldest blocks go, once there are more than [`KEEP_BLOCKS`].
    ///
    /// Everything this frame touched is kept whatever the cap says, because
    /// dropping a block a frame is *using* would make the next frame rebuild it —
    /// a cache that thrashes is worse than no cache, and it is a state a plain
    /// count cannot tell itself out of.
    fn retire(&mut self) {
        if self.blocks.len() <= KEEP_BLOCKS {
            return;
        }
        let frame = self.frame;
        self.blocks.retain(|_, cached| cached.used == frame);
    }
}

/// One frame's grid, built out of blocks this bake already holds.
///
/// The same [`Occlusion`] [`occlusion::collect`](super::collect) builds, to the
/// byte, for the same arguments — which is the property the whole of this module
/// rests on and is held by
/// `occlusion::bake::tests::a_baked_grid_is_the_one_the_walk_builds`.
///
/// What it does in order: the blocks the rectangle overlaps, then the server's
/// ground items, then the blur, then the pack with the frame's [`Cutaway`]. The
/// first is where the saving is and the last three are per frame whatever is
/// cached.
// Seven, and it is `occlusion::collect`'s six plus the cache. Grouping them
// would be one more type to keep in step for no fewer facts.
#[allow(clippy::too_many_arguments)]
pub fn collect(
    bake: &mut Bake,
    map: &Map,
    items: &[GroundItem],
    bounds: TileBounds,
    tiledata: &TileData,
    cutaway: &Cutaway,
    atlas: Option<&StaticAtlas>,
) -> Occlusion {
    bake.begin(atlas);
    let mut grid = Builder::new(bounds);

    // The same clamp the walk makes, and it has to be the same one: a rectangle
    // that runs off the facet names blocks the map does not have, and a bake that
    // kept an entry for each of them would grow a cache of nothing.
    if let Some((xs, ys)) = bounds.clamp_to(map.width(), map.height()) {
        let side = BLOCK_SIZE;
        let columns = (u32::from(*xs.start()) / side)..=(u32::from(*xs.end()) / side);
        let rows = (u32::from(*ys.start()) / side)..=(u32::from(*ys.end()) / side);
        for block_x in columns {
            for block_y in rows.clone() {
                let baked = bake.block(map, tiledata, atlas, block_x, block_y);
                grid.paste(block_x, block_y, baked);
            }
        }
    }
    super::put_items(&mut grid, map, items, tiledata, atlas);

    bake.retire();
    grid.blur_sky();
    grid.finish(cutaway)
}

#[cfg(test)]
mod tests {
    use openshard_protocol::wire::Hue;
    use openshard_protocol::world::Point;
    use openshard_uofiles::map::{LandCell, StaticItem};
    use openshard_uofiles::tiledata::{StaticTile, TileFlags};

    use super::*;
    use crate::facing::Hole;

    /// A tiledata entry that stops light: what a wall is, for the tests here.
    fn wall(height: u8) -> StaticTile {
        StaticTile {
            flags: TileFlags::new(TileFlags::NO_SHOOT),
            height,
            ..StaticTile::default()
        }
    }

    /// Four blocks of ground with a few walls scattered over them, deliberately
    /// including a tile with two statics on it and a tile on a block boundary.
    ///
    /// Built rather than loaded — `docs/lighting.md`'s decision 10 — so that a
    /// disagreement between the two builds names a tile rather than a
    /// coordinate in Britain.
    fn town() -> (Map, TileData) {
        let mut map = Map::from_blocks(2, 2, |_, _| LandCell { tile: 3, z: 0 });
        // A run of wall along one row, crossing the boundary between the two
        // block columns at x = 8: the case a per-block bake could get wrong and a
        // whole-rectangle walk cannot.
        for x in 5..12 {
            map.place_static(StaticItem {
                x,
                y: 6,
                z: 0,
                tile: WALL.0,
                hue: 0,
            });
        }
        // Two statics on one tile, at two heights: the list has to keep both, in
        // the order the file has them.
        map.place_static(StaticItem {
            x: 3,
            y: 3,
            z: 0,
            tile: WALL.0,
            hue: 0,
        });
        map.place_static(StaticItem {
            x: 3,
            y: 3,
            z: 30,
            tile: OTHER.0,
            hue: 0,
        });
        // One in the far block, so that a bake that only ever built the first
        // block would be caught.
        map.place_static(StaticItem {
            x: 12,
            y: 13,
            z: 0,
            tile: WALL.0,
            hue: 0,
        });

        let mut tiledata = TileData::empty();
        tiledata.set_static_tile(WALL.0, wall(20));
        tiledata.set_static_tile(OTHER.0, wall(6));
        (map, tiledata)
    }

    const WALL: Graphic = Graphic(0x1000);
    const OTHER: Graphic = Graphic(0x1001);

    /// The rectangle the tests build over: the whole of the four blocks and a
    /// margin outside them, so that the rim blocks and the clamp are exercised.
    fn bounds() -> TileBounds {
        TileBounds {
            min_x: -3,
            max_x: 18,
            min_y: -3,
            max_y: 18,
        }
    }

    /// **The one this module rests on.** A grid assembled out of baked blocks is
    /// the grid the walk builds — not similar to it, equal to it: the same
    /// surfaces in the same order at the same offsets, the same sky, the same
    /// count of what was dropped.
    ///
    /// Ordinary `==` on the packed [`Occlusion`], because everything a frame
    /// uploads is in it and a field that differed would be a texel that differed.
    #[test]
    fn a_baked_grid_is_the_one_the_walk_builds() {
        let (map, tiledata) = town();
        // On a tile the map has nothing on, and inside a block the bake serves:
        // a ground item that vanished under a baked block would be a door the
        // grid stopped knowing about.
        let items = [GroundItem {
            graphic: WALL,
            at: Point::new(9, 9, 0),
            hue: Hue::NONE,
        }];
        let mut bake = Bake::new();

        // Both of the cutaway's two cuts, because the whole of decision 33 is
        // that they happen *after* the bake: a height, and roofs going whatever
        // height they are at.
        let indoors = Cutaway {
            max_z: 20,
            no_draw_roofs: true,
            ..Cutaway::OPEN
        };
        for cutaway in [Cutaway::OPEN, indoors] {
            let walked = super::super::collect(&map, &items, bounds(), &tiledata, &cutaway, None);
            let baked = collect(&mut bake, &map, &items, bounds(), &tiledata, &cutaway, None);
            assert_eq!(walked, baked, "cutaway {cutaway:?}");
            assert!(
                walked.surface_count() > 0,
                "a grid with nothing in it proves nothing"
            );
        }
    }

    /// The frame after the first builds nothing: every block it wants is one it
    /// already has.
    ///
    /// The companion the measurement needs. A bake that rebuilt every block
    /// would pass every equality test above and cost exactly what it did before.
    #[test]
    fn a_block_is_built_once_and_then_served() {
        let (map, tiledata) = town();
        let mut bake = Bake::new();

        collect(&mut bake, &map, &[], bounds(), &tiledata, &Cutaway::OPEN, None);
        let (hits, misses) = bake.served();
        assert_eq!(hits, 0, "nothing was cached before the first frame");
        assert_eq!(misses, 4, "four blocks of map under the rectangle");

        collect(&mut bake, &map, &[], bounds(), &tiledata, &Cutaway::OPEN, None);
        assert_eq!(bake.served(), (4, 4), "the second frame built nothing");
    }

    /// A change to what the art says throws the cache away.
    ///
    /// The quiet failure this module's header names: an atlas grows, so a block
    /// derived before a graphic was packed holds a poorer answer than the one
    /// available now, and nothing about the block itself would say so. The test
    /// states a *hole* rather than packing a picture, because the hole is the
    /// shape answer with the most visible consequence and
    /// [`StaticAtlas::state_hole`] moves the revision the same way packing does.
    #[test]
    fn a_bake_lets_go_when_the_atlas_says_something_new() {
        let (map, tiledata) = town();
        let mut atlas = StaticAtlas::pack(Vec::new()).expect("an empty atlas");
        let mut bake = Bake::new();

        collect(
            &mut bake,
            &map,
            &[],
            bounds(),
            &tiledata,
            &Cutaway::OPEN,
            Some(&atlas),
        );
        assert_eq!(bake.len(), 4);

        atlas.state_hole(
            WALL,
            Hole {
                near: 64,
                far: 191,
                bottom: 4,
                top: 12,
            },
        );
        collect(
            &mut bake,
            &map,
            &[],
            bounds(),
            &tiledata,
            &Cutaway::OPEN,
            Some(&atlas),
        );
        assert_eq!(
            bake.served(),
            (0, 8),
            "the second frame rebuilt every block rather than serving a stale one"
        );
    }

    /// A bake handed no atlas at all is a different state from one handed an
    /// atlas that has said nothing yet, and swapping between them lets go.
    #[test]
    fn no_atlas_is_not_the_same_as_an_empty_one() {
        let (map, tiledata) = town();
        let atlas = StaticAtlas::pack(Vec::new()).expect("an empty atlas");
        let mut bake = Bake::new();

        collect(&mut bake, &map, &[], bounds(), &tiledata, &Cutaway::OPEN, None);
        collect(
            &mut bake,
            &map,
            &[],
            bounds(),
            &tiledata,
            &Cutaway::OPEN,
            Some(&atlas),
        );
        assert_eq!(bake.served(), (0, 8));
    }
}
