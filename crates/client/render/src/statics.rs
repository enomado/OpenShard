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

use crate::animate::StaticAnimations;
use crate::atlas::{Sprite, StaticAtlas};
use crate::camera::{Camera, TILE_HEIGHT, TileBounds};
use crate::cutaway::{self, Cutaway};
use crate::depth;
use crate::geometry::{Rect, Vec2};
use crate::sprite::SpriteQuad;

/// Where a sprite standing on a tile lands, in viewport pixels.
///
/// The arithmetic in this module's own header, named so that it has one copy:
/// a static read out of the map and an item the server put on the ground are
/// the same picture standing the same way, and the second is
/// [`crate::items`]. Centred on the tile's column, bottom edge on the
/// diamond's bottom vertex — `View.DrawStatic`.
pub fn stand_on(camera: &Camera, at: Point, sprite: &Sprite) -> Vec2 {
    let at = camera.to_screen(at);
    Vec2::new(
        // `>> 1` and not `/ 2.0`: an odd-width sprite lands half a pixel off
        // centre in the client too, and rounding it the other way shifts every
        // one of them against the ground.
        (at.x - (i32::from(sprite.width) >> 1)) as f32,
        (at.y + TILE_HEIGHT / 2 - i32::from(sprite.height)) as f32,
    )
}

/// Whether a sprite placed here touches the drawn image at all.
///
/// `AddTileToRenderList` rejects an object whose screen position falls outside
/// `_minPixel`/`_maxPixel` before it asks anything else about it, and the
/// reason there is a reject at all is [`for_each_static_in`]'s: the cells walked
/// are widened by the whole `z` range in both directions, which is 512 pixels
/// either way, so a screenful of tiles is walked with a frame of cells around
/// it that cannot draw anything.
///
/// The client tests the cell's own corner against bounds grown by a tile. This
/// tests the sprite's actual rectangle, which is the same question asked
/// exactly — a 250-pixel tree hangs five tiles up the screen out of its own
/// cell, and a margin is what the client needs because it is testing a point
/// that is not where the picture is.
pub fn on_screen(camera: &Camera, at: Vec2, sprite: &Sprite) -> bool {
    at.x + f32::from(sprite.width) > 0.0
        && at.x < camera.render_width() as f32
        && at.y + f32::from(sprite.height) > 0.0
        && at.y < camera.render_height() as f32
}

/// Every distinct static graphic standing on the cells the camera can see.
///
/// Called before building the atlas, for the same reason
/// [`ground::visible_graphics`](crate::ground::visible_graphics) is: a quad
/// cannot be given a region until the atlas holding it exists.
pub fn visible_graphics(map: &Map, camera: &Camera, animations: &StaticAnimations) -> BTreeSet<Graphic> {
    let mut seen = BTreeSet::new();
    graphics_in(map, camera.visible_tiles(), animations, &mut seen);
    seen
}

/// Every distinct static graphic standing on the cells of one rectangle, added
/// to `out`. [`ground::graphics_in`](crate::ground::graphics_in) for the sprites
/// rather than the ground, and it exists for the same reason: an atlas grows by
/// the band the camera crossed, not by the viewport it is looking at.
///
/// An animated static contributes its **whole cycle** and not the graphic it is
/// showing — see [`StaticAnimations::cycle`]. Offering the current one instead
/// packs less and grows the atlas every time a fire ticks over, which is a band
/// of rows uploaded to the GPU on whichever frame that happened to be.
pub fn graphics_in(
    map: &Map,
    bounds: TileBounds,
    animations: &StaticAnimations,
    out: &mut BTreeSet<Graphic>,
) {
    for_each_static_in(map, bounds, |item| {
        out.extend(animations.cycle(Graphic(item.tile)));
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
///
/// `cutaway` is what the frame has decided not to draw — the roof over the
/// player and the storey above them. It is a parameter and not a lookup because
/// it is one answer per frame: it is read from the tile the player is standing
/// on and every quad in the frame is tested against the same three numbers. See
/// [`crate::cutaway`].
pub fn collect(
    map: &Map,
    camera: &Camera,
    tiledata: &TileData,
    animations: &StaticAnimations,
    atlas: &StaticAtlas,
    cutaway: &Cutaway,
) -> Vec<SpriteQuad> {
    let (eye_x, eye_y) = camera.eye_tile();
    let base = depth::base_for(eye_x, eye_y);
    let mut quads: Vec<(depth::Order, SpriteQuad)> = Vec::new();

    for_each_static_in(map, camera.visible_tiles(), |item| {
        let tile = tiledata.static_tile(item.tile);
        if !cutaway::shows(cutaway, item.z, tile) {
            return;
        }
        // What this static is showing at the instant the frame was sampled. The
        // *placed* graphic still decides the sort and the depth below: a fire's
        // frames are different art of the same size standing in the same place,
        // and ordering by whichever one is on screen would let a stack reshuffle
        // itself every hundred milliseconds.
        let graphic = animations.showing(Graphic(item.tile));
        let Some(sprite) = atlas.sprite(graphic) else {
            return;
        };
        let order = depth::Order {
            tile: i32::from(item.x) + i32::from(item.y),
            priority_z: depth::static_priority_z(item.z, tile),
        };
        // The cell's centre, height folded in: `to_screen` already lifts `z` by
        // four pixels a unit, which is the same lift the ground gets.
        let at = stand_on(camera, Point::new(item.x, item.y, item.z), &sprite);
        if !on_screen(camera, at, &sprite) {
            return;
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
                hue: u32::from(item.hue),
            },
        ));
    });

    // Back to front, and a *stable* sort on the order alone: two statics on one
    // tile at one `PriorityZ` keep the order the file has them in, which is the
    // order the client inserted them into its per-tile list and therefore the
    // order it draws them. The depth test is `LessEqual`, so later drawn wins
    // the tie — see `renderer::depth_state`. Sorting by the graphic as well
    // would be just as deterministic and would resolve those ties by an
    // accident of the art's numbering.
    quads.sort_by_key(|(order, _)| *order);
    quads.into_iter().map(|(_, quad)| quad).collect()
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

    /// The contract between the animation clock and the atlas, on a real town:
    /// every graphic a static will *show* over a whole cycle is one the atlas was
    /// *offered*.
    ///
    /// Breaking it does not fail loudly — [`collect`] drops a graphic the atlas
    /// has no sprite for, exactly as it does for art the client does not ship —
    /// so a fire would simply vanish for five frames out of six and come back.
    ///
    /// The scene is checked for having something to prove first. A view of
    /// Britain with no animated statics in it would pass this in silence, which
    /// is the false green this repository keeps rediscovering.
    ///
    /// **What it does and does not catch, measured rather than assumed.** It
    /// catches the wiring: `graphics_in` offering the frame on screen instead of
    /// the cycle fails it. It does *not* catch a cycle that is short by one
    /// frame, and that was checked by mutation rather than reasoned about — the
    /// offer is a union over everything on screen, and a fire's neighbours cycle
    /// through the same six graphics, so a frame this static did not ask for was
    /// packed on its neighbour's behalf. The per-graphic property that has no
    /// union to hide in lives beside the clock, in
    /// [`animate`](crate::animate)'s own tests, and both of those do fail on
    /// that mutation. This one is the integration: that the two ends are
    /// connected on a real map.
    #[test]
    fn britain_offers_the_atlas_every_frame_its_fires_will_show() {
        use crate::animate::{FRAME_STEP, StaticAnimations};
        use openshard_uofiles::animdata::AnimData;

        let Some(dir) = std::env::var_os("OPENSHARD_CLIENT").map(std::path::PathBuf::from) else {
            return;
        };
        let map = Map::load_facet(&dir, 0).expect("Felucca");
        let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");
        let animdata = AnimData::load(&dir).expect("animdata.mul");
        let mut animations = StaticAnimations::build(&animdata, &tiledata);

        // The forge and the smithy east of the bank, which is where Britain
        // keeps its fires.
        let camera = Camera::new(Point::new(1420, 1683, 0), 768, 512);
        let offered = visible_graphics(&map, &camera, &animations);

        // The scene has something to say. Counted over the *placed* graphics, so
        // this is "there are animated statics on screen" and not "the offer is
        // bigger than the placed set", which the offer is by construction.
        let mut placed = BTreeSet::new();
        graphics_in(
            &map,
            camera.visible_tiles(),
            &StaticAnimations::default(),
            &mut placed,
        );
        let animating = placed
            .iter()
            .filter(|graphic| tiledata.static_tile(graphic.0).flags.is_animated())
            .count();
        assert!(
            animating > 0,
            "nothing on this screen animates: the test proves nothing"
        );
        assert!(
            offered.len() > placed.len(),
            "the cycles added no graphics at all"
        );

        let art = openshard_uofiles::art::Art::open(&dir).expect("artLegacyMUL.uop");
        let atlas = StaticAtlas::build(&art, offered.iter().copied()).expect("a screen of statics fits");
        // Ten seconds, which is longer than the slowest cycle in the file. The
        // count of quads must not move: a graphic that was shown and not packed
        // is a sprite that silently stops being drawn.
        let first = collect(&map, &camera, &tiledata, &animations, &atlas, &Cutaway::OPEN).len();
        assert!(first > 300, "only {first} statics on screen");
        for step in 1..=100 {
            animations.advance(FRAME_STEP);
            let now = collect(&map, &camera, &tiledata, &animations, &atlas, &Cutaway::OPEN).len();
            assert_eq!(
                now, first,
                "a static vanished {step} steps in: shown but never packed"
            );
        }
    }

    /// The four edges of the screen reject, and a pixel either side of each one
    /// is the difference.
    ///
    /// Stated as a boundary rather than as "far away is out": the whole risk in
    /// a cull is that it is one sprite too eager, and a test that places things
    /// a hundred pixels off screen passes with any of the four comparisons
    /// written the wrong way round.
    #[test]
    fn a_sprite_one_pixel_onto_the_screen_is_kept() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let sprite = Sprite {
            width: 30,
            height: 50,
            region: crate::atlas::Region {
                u: 0.0,
                v: 0.0,
                du: 0.0,
                dv: 0.0,
            },
        };
        let on = |x: f32, y: f32| on_screen(&camera, Vec2::new(x, y), &sprite);

        // Off the left: the sprite's right edge is at x + 30, so -30 is the
        // first placement with nothing on screen and -29 is the last with one
        // column of it showing.
        assert!(!on(-30.0, 300.0));
        assert!(on(-29.0, 300.0));
        // Off the right: 800 is the first column past the image.
        assert!(!on(800.0, 300.0));
        assert!(on(799.0, 300.0));
        // Above, where a 250-pixel tree hangs out of its own cell.
        assert!(!on(400.0, -50.0));
        assert!(on(400.0, -49.0));
        // And below.
        assert!(!on(400.0, 600.0));
        assert!(on(400.0, 599.0));
    }

    /// Standing under a roof in Britain draws fewer statics than standing
    /// outside it, and the picture is not empty either way.
    ///
    /// The integration the unit tests in [`crate::cutaway`] cannot do: those
    /// assert what a `Cutaway` decides, this asserts that the collector asks it
    /// — which is a line that can be deleted with every one of them still
    /// green.
    ///
    /// Skipped without the client's files.
    #[test]
    fn a_cutaway_takes_statics_out_of_the_frame() {
        let Some(dir) = std::env::var_os("OPENSHARD_CLIENT").map(std::path::PathBuf::from) else {
            return;
        };
        let map = Map::load_facet(&dir, 0).expect("Felucca");
        let art = openshard_uofiles::art::Art::open(&dir).expect("artLegacyMUL.uop");
        let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");

        let camera = Camera::new(Point::new(1495, 1629, 0), 768, 512);
        let animations = StaticAnimations::default();
        let wanted = visible_graphics(&map, &camera, &animations);
        let atlas = StaticAtlas::build(&art, wanted).expect("a screen of statics fits");
        let open = collect(&map, &camera, &tiledata, &animations, &atlas, &Cutaway::OPEN).len();

        // A tile in this quarter of Britain that is under something. Found
        // rather than named, for the reason `cutaway`'s own map test searches:
        // a coordinate written down here is one more thing to be wrong about.
        let indoors = (1620..1640u16)
            .flat_map(|y| (1485..1505u16).map(move |x| (x, y)))
            .find_map(|(x, y)| {
                let z = map.land(x, y)?.z;
                let cutaway = Cutaway::at(&map, &tiledata, Point::new(x, y, z), true);
                (cutaway != Cutaway::OPEN).then_some(cutaway)
            })
            .expect("something in Britain is under a roof");
        let cut = collect(&map, &camera, &tiledata, &animations, &atlas, &indoors).len();

        assert!(cut < open, "the cutaway removed nothing: {cut} of {open}");
        assert!(cut > 0, "the cutaway removed the whole town");
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
        let wanted = visible_graphics(&map, &camera, &StaticAnimations::default());
        assert!(
            wanted.len() > 50,
            "only {} static graphics in the middle of Britain",
            wanted.len(),
        );
        let atlas = StaticAtlas::build(&art, wanted).expect("a screen of statics fits");
        let quads = collect(
            &map,
            &camera,
            &tiledata,
            &StaticAnimations::default(),
            &atlas,
            &Cutaway::OPEN,
        );
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
