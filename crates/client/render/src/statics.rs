//! Turning the statics on a patch of map into the quads that draw them.
//!
//! The CPU side, the way [`crate::ground`] is for land: read what stands on the
//! visible cells, place each sprite, look it up in the atlas, and give it the
//! depth that decides what it hides. No GPU type appears here.
//!
//! # A static is not a tile
//!
//! Ground is 44x44 whatever the art holds and its quad is the diamond. A static
//! is a picture of any size that stands *on* a tile, and where it goes is the
//! client's arithmetic rather than ours: the sprite is centred on the tile's
//! column and its bottom edge sits at the diamond's bottom vertex, so a tall
//! tree hangs up the screen out of a 44-pixel cell. `View.DrawStatic` writes
//! that as `x -= (width >> 1) - 22` and `y -= height - 44` against a screen
//! position that is the cell's top-left corner — the same two numbers, said
//! from the corner instead of the centre.

use std::collections::BTreeSet;

use openshard_protocol::wire::Graphic;
use openshard_protocol::world::Point;
use openshard_uofiles::map::Map;
use openshard_uofiles::tiledata::TileData;

use crate::atlas::{Sprite, StaticAtlas};
use crate::camera::{Camera, TILE_HEIGHT, TileBounds};
use crate::depth;
use crate::sprite::SpriteQuad;

/// Where a sprite standing on a tile lands, in viewport pixels.
///
/// The arithmetic in this module's own header, named so that it has one copy:
/// a static read out of the map and an item the server put on the ground are
/// the same picture standing the same way, and the second is
/// [`crate::items`]. Centred on the tile's column, bottom edge on the
/// diamond's bottom vertex — `View.DrawStatic`.
pub fn stand_on(camera: &Camera, at: Point, sprite: &Sprite) -> (f32, f32) {
    let at = camera.to_screen(at);
    (
        // `>> 1` and not `/ 2.0`: an odd-width sprite lands half a pixel off
        // centre in the client too, and rounding it the other way shifts every
        // one of them against the ground.
        (at.x - (i32::from(sprite.width) >> 1)) as f32,
        (at.y + TILE_HEIGHT / 2 - i32::from(sprite.height)) as f32,
    )
}

/// Every distinct static graphic standing on the cells the camera can see.
///
/// Called before building the atlas, for the same reason
/// [`ground::visible_graphics`](crate::ground::visible_graphics) is: a quad
/// cannot be given a region until the atlas holding it exists.
pub fn visible_graphics(map: &Map, camera: &Camera) -> BTreeSet<Graphic> {
    let mut seen = BTreeSet::new();
    graphics_in(map, camera.visible_tiles(), &mut seen);
    seen
}

/// Every distinct static graphic standing on the cells of one rectangle, added
/// to `out`. [`ground::graphics_in`](crate::ground::graphics_in) for the sprites
/// rather than the ground, and it exists for the same reason: an atlas grows by
/// the band the camera crossed, not by the viewport it is looking at.
pub fn graphics_in(map: &Map, bounds: TileBounds, out: &mut BTreeSet<Graphic>) {
    for_each_static_in(map, bounds, |item| {
        out.insert(Graphic(item.tile));
    });
}

/// The quads for every visible static.
///
/// A graphic the atlas does not hold is dropped — the client ships no art for
/// it, or the atlas was built for a different camera — which is the same
/// "nothing to draw here" the ground makes of a missing land sprite.
///
/// The order they come back in does not decide what covers what: every quad
/// carries its own depth and the pass tests it. They are sorted anyway, back to
/// front, so that the same camera produces the same buffer byte for byte —
/// which is what the frame tests assert on, and what a `HashMap` slipped in
/// later would quietly take away.
pub fn collect(map: &Map, camera: &Camera, tiledata: &TileData, atlas: &StaticAtlas) -> Vec<SpriteQuad> {
    let (eye_x, eye_y) = camera.eye_tile();
    let base = depth::base_for(eye_x, eye_y);
    let mut quads: Vec<(depth::Order, u16, SpriteQuad)> = Vec::new();

    for_each_static_in(map, camera.visible_tiles(), |item| {
        let graphic = Graphic(item.tile);
        let Some(sprite) = atlas.sprite(graphic) else {
            return;
        };
        let order = depth::Order {
            tile: i32::from(item.x) + i32::from(item.y),
            priority_z: depth::static_priority_z(item.z, tiledata.static_tile(item.tile)),
        };
        // The cell's centre, height folded in: `to_screen` already lifts `z` by
        // four pixels a unit, which is the same lift the ground gets.
        let (x, y) = stand_on(camera, Point::new(item.x, item.y, item.z), &sprite);
        quads.push((
            order,
            item.tile,
            SpriteQuad {
                x,
                y,
                width: f32::from(sprite.width),
                height: f32::from(sprite.height),
                region: sprite.region,
                depth: order.to_depth(base),
                hue: u32::from(item.hue),
            },
        ));
    });

    // Back to front, and the graphic breaks a tie: two identical statics on one
    // tile at one height are the same picture, so which is "first" is arbitrary
    // and only has to be *stable*.
    quads.sort_by_key(|(order, graphic, _)| (*order, *graphic));
    quads.into_iter().map(|(_, _, quad)| quad).collect()
}

/// Walk every static on the visible cells, calling back for each.
///
/// The cells are the ones the ground walks — the same clamped rectangle — and
/// that is not quite the same set as "every static whose sprite touches the
/// viewport": a tree is 250 pixels tall and stands up to five tiles further
/// down the screen than its own cell. [`Camera::visible_tiles`] already widens
/// by the whole `z` range in both directions, which is 512 pixels either way,
/// so the sprites are covered by a margin that exists for another reason. Said
/// here because it is a dependency between two modules and not an accident.
fn for_each_static_in(
    map: &Map,
    bounds: TileBounds,
    mut each: impl FnMut(&openshard_uofiles::map::StaticItem),
) {
    let Some((xs, ys)) = bounds.clamp_to(map.width(), map.height()) else {
        return;
    };
    for y in ys {
        for x in xs.clone() {
            for item in map.statics_at(x, y) {
                each(item);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where a sprite of a given size lands on a given tile, stated in numbers
    /// rather than by drawing it.
    ///
    /// This is the arithmetic the whole layer rests on, and it is the kind that
    /// looks right at a glance in either of two wrong forms — centred on the
    /// cell instead of standing on it, or standing on the cell's *top*. Both
    /// draw a plausible town and put every wall half a tile out of place.
    #[test]
    fn a_sprite_stands_centred_on_the_bottom_of_its_cell() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let at = camera.to_screen(Point::new(100, 100, 0));
        assert_eq!((at.x, at.y), (400, 300), "the camera centres its own tile");

        // A 44x44 sprite — a floor tile — covers the cell exactly: the same
        // square the ground's flat art is drawn in.
        let (x, y) = place(&camera, Point::new(100, 100, 0), 44, 44);
        assert_eq!((x, y), (400 - 22, 300 + 22 - 44));

        // A tall, narrow sprite hangs up the screen from the same bottom edge.
        let (x, y) = place(&camera, Point::new(100, 100, 0), 30, 120);
        assert_eq!((x, y), (400 - 15, 300 + 22 - 120));

        // And height lifts it four pixels a unit, exactly as it lifts ground.
        let (_, lifted) = place(&camera, Point::new(100, 100, 10), 44, 44);
        assert_eq!(lifted, 300 + 22 - 44 - 40);
    }

    /// The same placement the collector does, without needing a `Map`.
    fn place(camera: &Camera, point: Point, width: u16, height: u16) -> (i32, i32) {
        let at = camera.to_screen(point);
        (
            at.x - (i32::from(width) >> 1),
            at.y + TILE_HEIGHT / 2 - i32::from(height),
        )
    }

    /// On a real town, the quads come back sorted and every depth agrees with
    /// the sort — the ordering the depth buffer will enforce is the ordering
    /// the collector believes in.
    ///
    /// Two things could break independently here: the sort key, and the
    /// arithmetic that turns it into a depth. Asserting that the depths are
    /// non-increasing across a sorted list is what ties them together, and it
    /// is measured on Britain rather than on a fixture because a fixture's
    /// stack of statics would be one this module's own understanding wrote.
    ///
    /// Skipped without the client's files, like everything else needing a map.
    #[test]
    fn britains_statics_come_back_sorted_from_the_back() {
        let Some(dir) = std::env::var_os("OPENSHARD_CLIENT").map(std::path::PathBuf::from) else {
            return;
        };
        let map = Map::load_facet(&dir, 0).expect("Felucca");
        let art = openshard_uofiles::art::Art::open(&dir).expect("artLegacyMUL.uop");
        let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");

        // Britain by the bank: buildings, walls, floors and signs, which is
        // what makes the ordering worth checking here rather than in a field.
        let camera = Camera::new(Point::new(1495, 1629, 0), 768, 512);
        let wanted = visible_graphics(&map, &camera);
        assert!(
            wanted.len() > 50,
            "only {} static graphics in the middle of Britain",
            wanted.len(),
        );
        let atlas = StaticAtlas::build(&art, wanted).expect("a screen of statics fits");
        let quads = collect(&map, &camera, &tiledata, &atlas);
        assert!(quads.len() > 500, "only {} statics on screen", quads.len());

        let mut previous = f32::INFINITY;
        for quad in &quads {
            assert!(
                quad.depth <= previous,
                "a quad at depth {} came after one at {previous}: the sort and the depth disagree",
                quad.depth,
            );
            previous = quad.depth;
        }

        // And the frame actually spans depths, or the assertion above is
        // satisfied by every static sharing one.
        let nearest = quads.last().expect("not empty").depth;
        let furthest = quads.first().expect("not empty").depth;
        assert!(
            furthest - nearest > 1e-4,
            "every static came back at the same depth ({nearest})",
        );
    }
}
