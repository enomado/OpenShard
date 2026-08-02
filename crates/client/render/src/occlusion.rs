//! What stands between a flame and the ground it would light.
//!
//! A grid over the tiles this frame's flames can reach, one cell a tile, saying
//! how much of a ray crossing that tile survives it and between which heights.
//! [`crate::light`] hands it to the blit, which walks the cells between a
//! fragment and each flame — see `docs/lighting.md`, decisions 3 through 6.
//!
//! # Why a tile and not a wall's edge
//!
//! A wall stands on one edge of its tile, and **nothing in `tiledata.mul` says
//! which edge**: that is only in the shape of the sprite. So the occluder here
//! is the whole tile. It costs half a tile of reach at the wall, and it buys a
//! room whose wall tiles are a closed ring by construction — no corner of a
//! house leaks light into the street because two segments failed to meet.
//!
//! # What touches light is what stops an arrow — but not by as much
//!
//! `WINDOW | NO_SHOOT`, and not `BLOCK`. The two are different questions and the
//! reference keeps them apart: ServUO's `Map.LineOfSight` (`Server/Map.cs:3040`)
//! tests a static with `(flags & (TileFlag.Window | TileFlag.NoShoot)) != 0`
//! against the span `t.Z ..= t.Z + CalcHeight`, and impassability never enters
//! it. A barrel and a fence are `BLOCK` and you can see over both; a wall is
//! `NO_SHOOT` and you cannot see through it. Reading `BLOCK` instead would put a
//! shadow behind every crate on the street.
//!
//! Where this parts company with the reference is *how much* each stops. Line of
//! sight is a yes or a no, so a window is a wall in it; light is a fraction, and
//! a window is glass. So the grid carries an opacity byte — [`OPAQUE`] for a
//! wall, [`PANE`] for a window — and the shader multiplies by it either way.
//!
//! # Nothing occludes that was not drawn
//!
//! Every occluder is tested with the frame's [`Cutaway`], exactly as the flames
//! are. A shadow cast by a wall the cutaway took away is a dark band with
//! nothing in the picture making it, which is the worse bug of the two.

use openshard_uofiles::map::Map;
use openshard_uofiles::tiledata::{StaticTile, TileData, TileFlags};

use crate::camera::TileBounds;
use crate::cutaway::{self, Cutaway};
use crate::items::GroundItem;

/// A tile that stops light entirely.
pub const OPAQUE: u8 = 255;

/// A tile light crosses untouched.
pub const CLEAR: u8 = 0;

/// Whether a static touches light at all — a wall or a pane, and not a barrel.
///
/// The reference's line of sight test, and it is still the right *membership*
/// question: what is in the grid is what stops an arrow. How much of the light
/// each member stops is [`opacity`]'s answer and no longer the same one. See
/// this module's header for why it is not `BLOCK`.
pub fn stops_light(tile: &StaticTile) -> bool {
    tile.flags.has(TileFlags::WINDOW | TileFlags::NO_SHOOT)
}

/// How much of a ray crossing a pane of glass is stopped.
///
/// A fifth, which is a guess about glass and not a number from any file — the
/// client has none. What it is *not* is a guess about line of sight: an arrow is
/// stopped by a window and light is not, and `WINDOW` being in the same test as
/// `NO_SHOOT` in the reference is a fact about arrows. A window that stopped
/// light entirely is what makes a lit room read as a bunker, and it is the one
/// thing standing between a candle and the street it should be visible from.
pub const PANE: u8 = 51;

/// How much of a ray crossing this static is stopped, `0..=255`.
///
/// Three answers and not two: a wall stops everything, a pane dims, and
/// everything else — a barrel, a fence, a crate — passes light untouched even
/// where it stops an arrow. The byte was always here for this; what changed is
/// that `WINDOW` no longer borrows `NO_SHOOT`'s answer.
///
/// `NO_SHOOT` wins where a tile carries both. A shard's custom static that is
/// flagged as a solid window is more likely to be a shuttered one than a
/// transparent wall, and the union is the conservative direction — darkening is
/// visible, leaking a room into the street is a bug.
pub fn opacity(tile: &StaticTile) -> u8 {
    if tile.flags.has(TileFlags::NO_SHOOT) {
        return OPAQUE;
    }
    match tile.flags.has(TileFlags::WINDOW) {
        true => PANE,
        false => CLEAR,
    }
}

/// How tall a static stands, for the purpose of what it hides.
///
/// ServUO's `ItemData.CalcHeight` (`Server/TileData.cs:112`): a climbable
/// (`Bridge`) tile counts as half its stated height, because that is the height
/// you end up standing at on it. `movement`'s `platform_surface` halves the same
/// number for the same reason.
fn calc_height(tile: &StaticTile) -> i32 {
    let height = i32::from(tile.height);
    match tile.flags.is_climbable() {
        true => height / 2,
        false => height,
    }
}

/// One tile's worth of occlusion: how much it stops, and between which heights.
///
/// The span is in `z` units — the map's own, not pixels — and it is inclusive of
/// `bottom` and `top`. A wall based at `z = 0` and 20 tall stops a ray passing
/// through `0..=20` and no other, which is what keeps a cellar's wall out of the
/// street and an upper storey's out of the ground floor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    /// The lowest `z` this tile stops anything at.
    pub bottom: i32,
    /// The highest.
    pub top: i32,
    /// How much of a ray crossing the span is stopped.
    pub opacity: u8,
}

/// The occluders of one frame, as a rectangle of tiles.
///
/// Absent cells are the ordinary case — most of a street is open sky — and
/// [`Occlusion::at`] answers `None` for them and for anything outside the
/// rectangle, so a caller never has to know where the edge is.
#[derive(Clone, PartialEq, Debug)]
pub struct Occlusion {
    bounds: TileBounds,
    /// Row-major over `bounds`, `x` fastest: the order [`Occlusion::bytes`]
    /// uploads and the shader indexes.
    cells: Vec<Option<Cell>>,
}

impl Occlusion {
    /// A grid covering no tiles at all, which occludes nothing anywhere.
    ///
    /// A `const` and therefore an empty `Vec`, which allocates nothing: it is
    /// what [`Lighting::NONE`](crate::light::Lighting::NONE) is built from, and
    /// a daylit frame must not pay for a grid it will not read.
    pub const EMPTY: Self = Self {
        bounds: TileBounds {
            min_x: 0,
            max_x: -1,
            min_y: 0,
            max_y: -1,
        },
        cells: Vec::new(),
    };

    /// An empty grid over `bounds`: nothing stops anything.
    pub fn new(bounds: TileBounds) -> Self {
        let cells = vec![None; (bounds.width() * bounds.height()) as usize];
        Self { bounds, cells }
    }

    /// The rectangle of tiles this covers.
    pub fn bounds(&self) -> TileBounds {
        self.bounds
    }

    /// Add one occluder, merging with whatever already stands on that tile.
    ///
    /// The merge is the **union** of the two spans, and it is deliberately the
    /// conservative direction: two walls on one tile with a gap between them
    /// close the gap. After the cutaway has removed the storeys the player is
    /// not on, a tile holding two opaque statics is a doorframe and a lintel far
    /// more often than it is two walls with air between, and darkening a foot of
    /// air is invisible where leaking a room into the street is not.
    ///
    /// A tile outside [`Occlusion::bounds`] is dropped rather than clamped: it
    /// is a caller walking wider than it asked the grid for, and folding it onto
    /// the edge would put a wall where the map has none.
    pub fn add(&mut self, x: u16, y: u16, z: i8, tile: &StaticTile) {
        let opacity = opacity(tile);
        if opacity == CLEAR {
            return;
        }
        let Some(index) = self.index(i32::from(x), i32::from(y)) else {
            return;
        };
        let bottom = i32::from(z);
        let span = Cell {
            bottom,
            top: bottom + calc_height(tile),
            opacity,
        };
        self.cells[index] = Some(match self.cells[index] {
            None => span,
            Some(had) => Cell {
                bottom: had.bottom.min(span.bottom),
                top: had.top.max(span.top),
                opacity: had.opacity.max(span.opacity),
            },
        });
    }

    /// What stands on one tile, or `None` for open ground and for anything
    /// outside the rectangle.
    pub fn at(&self, x: i32, y: i32) -> Option<Cell> {
        self.cells[self.index(x, y)?]
    }

    /// The highest `z` anything in this grid stops light at, or `None` for a
    /// grid with nothing standing in it.
    ///
    /// What sunlight is bounded by. A flame's ray ends at the flame; the sun's
    /// has no end, so it needs something to stop walking at — and the honest
    /// answer is "as soon as the ray is above everything that could stop it".
    /// One number for the frame rather than a per-cell test, because the walk is
    /// leaving the grid upwards and what it has to beat is the tallest thing
    /// anywhere ahead of it.
    pub fn tallest(&self) -> Option<i32> {
        self.cells.iter().flatten().map(|cell| cell.top).max()
    }

    /// Where a tile lives in [`Occlusion::cells`].
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let bounds = self.bounds;
        if x < bounds.min_x || x > bounds.max_x || y < bounds.min_y || y > bounds.max_y {
            return None;
        }
        let (column, row) = (x - bounds.min_x, y - bounds.min_y);
        Some((row * bounds.width() + column) as usize)
    }

    /// The grid as the texture the shader reads: `Rgba8Uint`, one texel a tile,
    /// row-major from the rectangle's `(min_x, min_y)` corner.
    ///
    /// `(bottom + 128, top + 128, opacity, present)`. The `z` offset is what
    /// makes an `i8` fit an unsigned channel, and both ends are clamped into it:
    /// a map's `z` is an `i8`, but `z + height` is not — a 255-tall static based
    /// at 100 has a top no channel holds, and a wall that reaches past the top of
    /// the world may as well stop there.
    ///
    /// `present` is `0` or `255` rather than a `bool`, because the shader reads
    /// four channels of one type and a cell with nothing on it must not be a
    /// wall from `z = -128` to `z = -128`.
    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.cells.len() * 4);
        for cell in &self.cells {
            match cell {
                None => bytes.extend_from_slice(&[0, 0, 0, 0]),
                Some(cell) => {
                    let channel = |z: i32| (z.clamp(-128, 127) + 128) as u8;
                    bytes.extend_from_slice(&[channel(cell.bottom), channel(cell.top), cell.opacity, 255]);
                }
            }
        }
        bytes
    }
}

/// Everything on `bounds` that stands between a flame and the ground.
///
/// The same two sources the flames themselves come from — the map's statics and
/// the items the server has put on the ground — walked with the same bounds and
/// tested with the same [`Cutaway`]. Both halves matter: a wall is a static, and
/// **a door is an item**, sent by the server and swapped for its open graphic
/// when it is opened. A closed door that let the light through would be the one
/// occluder a player watches change.
pub fn collect(
    map: &Map,
    items: &[GroundItem],
    bounds: TileBounds,
    tiledata: &TileData,
    cutaway: &Cutaway,
) -> Occlusion {
    let mut occlusion = Occlusion::new(bounds);

    crate::statics::for_each_static_in(map, bounds, |item| {
        let tile = tiledata.static_tile(item.tile);
        if cutaway::shows(cutaway, item.z, tile) {
            occlusion.add(item.x, item.y, item.z, tile);
        }
    });

    for item in items {
        let tile = tiledata.static_tile(item.graphic.0);
        if cutaway::shows(cutaway, item.at.z, tile) {
            occlusion.add(item.at.x, item.at.y, item.at.z, tile);
        }
    }

    occlusion
}

#[cfg(test)]
mod tests {
    use openshard_protocol::wire::{Graphic, Hue};
    use openshard_protocol::world::Point;
    use openshard_uofiles::map::{LandCell, Map};

    use super::*;

    /// A static tile with the flags and height a test is about.
    fn tile(flags: u64, height: u8) -> StaticTile {
        StaticTile {
            flags: TileFlags::new(flags),
            height,
            ..StaticTile::default()
        }
    }

    /// A rectangle big enough for a few tiles around the origin of a test.
    fn bounds() -> TileBounds {
        TileBounds {
            min_x: 100,
            max_x: 110,
            min_y: 100,
            max_y: 110,
        }
    }

    /// The rule, said in every direction that matters. A wall stops light; a
    /// pane dims it; a barrel, which is `BLOCK` and nothing else, does not touch
    /// it. Reading impassability instead of the shooting flags is the mistake
    /// this was written against — it would put a shadow behind every crate on
    /// the street — and treating a window as a wall is the one beside it, which
    /// makes a lit room invisible from the road.
    #[test]
    fn a_wall_stops_light_a_pane_dims_it_and_a_barrel_does_not() {
        assert_eq!(opacity(&tile(TileFlags::NO_SHOOT, 20)), OPAQUE);
        assert_eq!(opacity(&tile(TileFlags::WINDOW, 20)), PANE);
        assert_eq!(opacity(&tile(TileFlags::BLOCK, 10)), CLEAR);
        // A real wall carries both, and the rule must not need the pair.
        assert_eq!(opacity(&tile(TileFlags::NO_SHOOT | TileFlags::BLOCK, 20)), OPAQUE);
        // And a static flagged as both a window and solid is the solid one: the
        // union darkens, which is the direction that cannot leak a room.
        assert_eq!(
            opacity(&tile(TileFlags::NO_SHOOT | TileFlags::WINDOW, 20)),
            OPAQUE
        );
        const { assert!(PANE > CLEAR && PANE < OPAQUE, "a pane is neither open nor a wall") };
    }

    /// A wall occupies the heights it occupies, and the grid says which.
    #[test]
    fn a_wall_carries_the_span_it_stands_in() {
        let mut occlusion = Occlusion::new(bounds());
        occlusion.add(102, 103, 5, &tile(TileFlags::NO_SHOOT, 20));
        assert_eq!(
            occlusion.at(102, 103),
            Some(Cell {
                bottom: 5,
                top: 25,
                opacity: OPAQUE
            })
        );
        assert_eq!(occlusion.at(103, 103), None, "its neighbour is open ground");
    }

    /// Stairs count as half their height, the way every other reader of this
    /// field here does. A stair that occluded its full height would shadow the
    /// landing it leads to.
    #[test]
    fn a_climbable_static_occludes_half_its_height() {
        let mut occlusion = Occlusion::new(bounds());
        occlusion.add(100, 100, 0, &tile(TileFlags::NO_SHOOT | TileFlags::CLIMBABLE, 20));
        assert_eq!(occlusion.at(100, 100).unwrap().top, 10);
    }

    /// Two occluders on one tile become one span covering both. The union is the
    /// conservative direction and the doc comment on `add` argues for it; this
    /// pins that it is what happens.
    #[test]
    fn two_occluders_on_one_tile_span_both() {
        let mut occlusion = Occlusion::new(bounds());
        occlusion.add(105, 105, 0, &tile(TileFlags::NO_SHOOT, 20));
        occlusion.add(105, 105, 40, &tile(TileFlags::NO_SHOOT, 20));
        assert_eq!(
            occlusion.at(105, 105),
            Some(Cell {
                bottom: 0,
                top: 60,
                opacity: OPAQUE
            })
        );
    }

    /// Outside the rectangle is not the edge of it. A caller walking wider than
    /// the grid was built for must lose the occluder rather than fold it onto
    /// the border, where it would be a wall the map does not have.
    #[test]
    fn a_tile_outside_the_bounds_is_dropped_and_not_clamped() {
        let mut occlusion = Occlusion::new(bounds());
        occlusion.add(99, 100, 0, &tile(TileFlags::NO_SHOOT, 20));
        assert_eq!(occlusion.at(99, 100), None);
        assert_eq!(occlusion.at(100, 100), None, "and did not land on the edge");
    }

    /// The upload is the grid, in the order the shader indexes it, with `z`
    /// offset into an unsigned channel and clamped at both ends of it.
    #[test]
    fn the_bytes_are_the_grid_row_major() {
        let mut occlusion = Occlusion::new(TileBounds {
            min_x: 0,
            max_x: 1,
            min_y: 0,
            max_y: 1,
        });
        occlusion.add(1, 0, -10, &tile(TileFlags::NO_SHOOT, 20));
        occlusion.add(0, 1, 120, &tile(TileFlags::NO_SHOOT, 60));
        let bytes = occlusion.bytes();
        assert_eq!(bytes.len(), 4 * 4);
        assert_eq!(&bytes[0..4], &[0, 0, 0, 0], "(0,0) is open");
        assert_eq!(&bytes[4..8], &[118, 138, OPAQUE, 255], "(1,0) is x-fastest");
        assert_eq!(
            &bytes[8..12],
            &[248, 255, OPAQUE, 255],
            "(0,1) reaches past the top of the world and stops there",
        );
    }

    /// A wall the cutaway has taken away casts no shadow. The storey above the
    /// player is not drawn, and a dark band under a wall that is not in the
    /// picture is worse than the light leaking.
    #[test]
    fn a_hidden_wall_occludes_nothing() {
        let map = Map::from_blocks(1, 1, |_, _| LandCell { tile: 0, z: 0 });
        let graphic = Graphic(0x0006);
        let mut tiledata = TileData::empty();
        tiledata.set_static_tile(graphic.0, tile(TileFlags::NO_SHOOT, 20));
        let items = [GroundItem {
            at: Point::new(4, 4, 40),
            graphic,
            hue: Hue::NONE,
        }];
        let bounds = TileBounds {
            min_x: 0,
            max_x: 7,
            min_y: 0,
            max_y: 7,
        };

        let open = collect(&map, &items, bounds, &tiledata, &Cutaway::OPEN);
        assert!(open.at(4, 4).is_some(), "with nothing cut away it occludes");

        let cut = collect(
            &map,
            &items,
            bounds,
            &tiledata,
            &Cutaway {
                max_z: 20,
                ..Cutaway::OPEN
            },
        );
        assert_eq!(cut.at(4, 4), None);
    }
}
