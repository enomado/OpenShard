//! Placing what the server has dropped on the ground.
//!
//! The fourth CPU-side collector, and the one that is two of the others put
//! together: an item's *picture* is a static's — the same art, the same atlas,
//! the same "centred on the column, standing on the diamond's bottom vertex" —
//! while its *source* is a mobile's, a list somebody else built out of what
//! arrived on the wire. Nothing in the map says an item is there; a `0x1A` does,
//! and a `0x1D` takes it away again.
//!
//! So the placement is [`crate::statics::stand_on`] rather than a second copy of
//! it, and what is written here is only the part that differs: where the list
//! comes from, and that it is sorted for the depth buffer.
//!
//! # Not the same thing as a static at the same tile
//!
//! Both draw through [`StaticAtlas`], and a client that merged the two lists
//! would be right until an item is picked up: the map's statics are the shard's
//! furniture and never move, while these come and go with every `0x1A`. Two
//! lists, one atlas.

use std::collections::BTreeSet;

use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::Point;
use openshard_uofiles::tiledata::TileData;

use crate::animate::StaticAnimations;
use crate::atlas::StaticAtlas;
use crate::camera::Camera;
use crate::cutaway::{self, Cutaway};
use crate::depth;
use crate::geometry::Rect;
use crate::sprite::SpriteQuad;
use crate::statics::{on_screen, stand_on};

/// One thing lying on the ground, as the client has been told about it.
///
/// A plain value and not a handle into a `WorldView`, for the reason
/// [`Mobile`](crate::mobiles::Mobile) is one: this crate renders what it is
/// given and owns no model of the world. The stack's *amount* is deliberately
/// absent — a pile of 500 gold is one sprite, and which sprite is the caller's
/// question, not the renderer's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GroundItem {
    /// Where it lies.
    pub at: Point,
    /// Its graphic, which is a static's graphic: the two share an art file and
    /// therefore an atlas.
    pub graphic: Graphic,
    /// Its hue, or [`Hue::NONE`] for none.
    pub hue: Hue,
}

/// Every distinct graphic a set of items needs packed.
///
/// A `BTreeSet` and not a `Vec`, to be unioned with
/// [`statics::visible_graphics`](crate::statics::visible_graphics) before the
/// atlas is built: one atlas serves both passes, so the two sets are asked for
/// together and packed once.
///
/// An item's whole cycle, for the reason
/// [`statics::graphics_in`](crate::statics::graphics_in) asks for one: a torch
/// dropped on the ground animates exactly as a torch built into the map does,
/// and the atlas has to hold every graphic it will turn into.
pub fn needed_graphics(items: &[GroundItem], animations: &StaticAnimations) -> BTreeSet<Graphic> {
    items
        .iter()
        .flat_map(|item| animations.cycle(item.graphic))
        .collect()
}

/// The quads for every item whose graphic the atlas holds.
///
/// One the atlas has no sprite for is dropped, exactly as in
/// [`statics::collect`](crate::statics::collect): the client ships no art for
/// it, or the atlas was built before this item arrived. Both are "nothing to
/// draw", and drawing it from a neighbouring graphic would be worse.
///
/// `cutaway` is the same one the statics are tested against, and it is the same
/// test: the client reads a ground item's tiledata row out of the static table,
/// so a barrel on the floor above is hidden with that floor exactly as a wall
/// built into the map is.
pub fn collect(
    items: &[GroundItem],
    camera: &Camera,
    tiledata: &TileData,
    animations: &StaticAnimations,
    atlas: &StaticAtlas,
    cutaway: &Cutaway,
) -> Vec<SpriteQuad> {
    let (eye_x, eye_y) = camera.eye_tile();
    let base = depth::base_for(eye_x, eye_y);
    let mut quads: Vec<(depth::Order, SpriteQuad)> = Vec::new();

    for item in items {
        // A ground item is ordered as a static is, and from the same table: the
        // client reads `tiledata`'s static entry for an item's graphic too, so a
        // wall lying on the floor and a wall built into the map sort alike.
        let tile = tiledata.static_tile(item.graphic.0);
        if !cutaway::shows(cutaway, item.at.z, tile) {
            continue;
        }
        // The frame on screen; the *placed* graphic still decides the sort and
        // the tiledata lookup above. See [`statics::collect`](crate::statics::collect).
        let Some(sprite) = atlas.sprite(animations.showing(item.graphic)) else {
            continue;
        };
        let order = depth::Order {
            tile: i32::from(item.at.x) + i32::from(item.at.y),
            priority_z: depth::static_priority_z(item.at.z, tile),
        };
        let at = stand_on(camera, item.at, &sprite);
        if !on_screen(camera, at, &sprite) {
            continue;
        }
        quads.push((
            order,
            SpriteQuad {
                rect: Rect {
                    x: at.x,
                    y: at.y,
                    width: f32::from(sprite.width),
                    height: f32::from(sprite.height),
                },
                region: sprite.region,
                depth: order.to_depth(base),
                hue: u32::from(item.hue.0),
            },
        ));
    }

    // Back to front, and a *stable* sort on the order alone: two items on one
    // tile at one `PriorityZ` keep the caller's order, which is by serial —
    // the order the server sent them and so the order the client's own
    // per-tile list holds them in. The depth test is `LessEqual`, so the later
    // one wins the tie; see `renderer::depth_state`.
    quads.sort_by_key(|(order, _)| *order);
    quads.into_iter().map(|(_, quad)| quad).collect()
}

#[cfg(test)]
mod tests {
    use openshard_uofiles::color::Color16;
    use openshard_uofiles::image::Image;

    use super::*;

    /// An atlas holding one graphic at a known size.
    fn atlas(graphic: Graphic, width: u16, height: u16) -> StaticAtlas {
        StaticAtlas::pack([(
            graphic,
            Image::new(
                width,
                height,
                vec![Color16(0x7C00); usize::from(width) * usize::from(height)],
            ),
        )])
        .expect("one sprite fits")
    }

    /// An item lands where a static of the same size on the same tile does.
    ///
    /// The assertion is the *comparison* rather than two numbers: the placement
    /// has one copy now, and this is what says the item pass is using it. Two
    /// numbers here would go on passing if a second copy appeared and drifted.
    #[test]
    fn an_item_stands_exactly_where_a_static_of_its_size_would() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let graphic = Graphic(0x0EED);
        let atlas = atlas(graphic, 30, 50);
        let tiledata = TileData::empty();

        let quads = collect(
            &[GroundItem {
                at: Point::new(100, 100, 0),
                graphic,
                hue: Hue::NONE,
            }],
            &camera,
            &tiledata,
            &StaticAnimations::default(),
            &atlas,
            &Cutaway::OPEN,
        );
        assert_eq!(quads.len(), 1);
        let sprite = atlas.sprite(graphic).expect("packed");
        let at = stand_on(&camera, Point::new(100, 100, 0), &sprite);
        assert_eq!((quads[0].rect.x, quads[0].rect.y), (at.x, at.y));
        assert_eq!((quads[0].rect.width, quads[0].rect.height), (30.0, 50.0));
    }

    /// Height lifts an item off the floor the way it lifts everything else, and
    /// it lifts its depth with it — a coin on a table is in front of the table's
    /// own tile, not behind it.
    #[test]
    fn a_higher_item_is_drawn_higher_and_nearer() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let graphic = Graphic(0x0EED);
        let atlas = atlas(graphic, 30, 50);
        let tiledata = TileData::empty();
        let at = |z: i8| GroundItem {
            at: Point::new(100, 100, z),
            graphic,
            hue: Hue::NONE,
        };
        let floor = collect(
            &[at(0)],
            &camera,
            &tiledata,
            &StaticAnimations::default(),
            &atlas,
            &Cutaway::OPEN,
        );
        let table = collect(
            &[at(10)],
            &camera,
            &tiledata,
            &StaticAnimations::default(),
            &atlas,
            &Cutaway::OPEN,
        );
        assert_eq!(table[0].rect.y, floor[0].rect.y - 40.0, "four pixels a unit");
        assert!(table[0].depth < floor[0].depth, "smaller is nearer");
    }

    /// An item whose graphic is not packed is dropped rather than drawn from
    /// whatever else is in the atlas.
    #[test]
    fn an_item_with_no_sprite_is_dropped() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let atlas = atlas(Graphic(0x0EED), 30, 50);
        let tiledata = TileData::empty();
        let quads = collect(
            &[GroundItem {
                at: Point::new(100, 100, 0),
                graphic: Graphic(0x0EEE),
                hue: Hue::NONE,
            }],
            &camera,
            &tiledata,
            &StaticAnimations::default(),
            &atlas,
            &Cutaway::OPEN,
        );
        assert!(quads.is_empty());
    }

    /// Two items on different tiles come back with the further one first, so a
    /// pass that ignored the depth buffer entirely would still paint them in the
    /// right order.
    #[test]
    fn the_quads_come_back_from_the_back() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let graphic = Graphic(0x0EED);
        let atlas = atlas(graphic, 30, 50);
        let tiledata = TileData::empty();
        let item = |x: u16, y: u16| GroundItem {
            at: Point::new(x, y, 0),
            graphic,
            hue: Hue::NONE,
        };
        // Given nearest first, on purpose.
        let quads = collect(
            &[item(101, 101), item(99, 99)],
            &camera,
            &tiledata,
            &StaticAnimations::default(),
            &atlas,
            &Cutaway::OPEN,
        );
        assert_eq!(quads.len(), 2);
        assert!(quads[0].depth > quads[1].depth, "the far one is drawn first");
    }

    /// The graphics a list needs, once each, ready to be unioned with the map's.
    #[test]
    fn the_needed_graphics_are_deduplicated() {
        let item = |graphic: u16| GroundItem {
            at: Point::new(0, 0, 0),
            graphic: Graphic(graphic),
            hue: Hue::NONE,
        };
        let wanted = needed_graphics(
            &[item(0x0EED), item(0x0EED), item(0x0EEA)],
            &StaticAnimations::default(),
        );
        assert_eq!(
            wanted.into_iter().collect::<Vec<_>>(),
            vec![Graphic(0x0EEA), Graphic(0x0EED)],
        );
    }
}
