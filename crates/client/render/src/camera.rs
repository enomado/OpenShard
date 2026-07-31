//! Where a tile lands on the screen.
//!
//! UO's world is a square grid seen from a fixed diagonal, so the projection is
//! two multiplications and no matrix at all. Every number here is the client's,
//! and the client's numbers are not a choice we get to make: a tile is 44 pixels
//! across, a step in `x` moves half a tile right and half a tile down, a step in
//! `y` moves half a tile left and half a tile down, and a unit of height lifts
//! the tile four pixels. Change one of them and the art stops meeting itself at
//! the seams — which is visible only as a shimmer along the diagonals, not as an
//! error, so the values are pinned by tests rather than trusted.

use openshard_protocol::world::Point;

/// A land tile's sprite is this wide. Statics vary; the ground never does.
pub const TILE_WIDTH: i32 = 44;

/// And this tall. The diamond fills the square corner to corner.
pub const TILE_HEIGHT: i32 = 44;

/// Half a tile: the screen distance one step in `x` or `y` covers on each axis.
const HALF_WIDTH: i32 = TILE_WIDTH / 2;
const HALF_HEIGHT: i32 = TILE_HEIGHT / 2;

/// Pixels a single unit of `z` lifts a tile up the screen.
pub const Z_STEP: i32 = 4;

/// The tallest lift `z` can produce, in pixels.
///
/// `z` is an `i8`, so the whole range is 255 units, and a tile at the bottom of
/// a dungeon and one on a mountain differ by a thousand pixels of screen space.
/// This is the slack [`Camera::visible_tiles`] has to allow for, because a tile
/// whose *ground position* is below the viewport can still be drawn inside it.
const MAX_Z_LIFT: i32 = 128 * Z_STEP;

/// A position in pixels. The origin is wherever the caller's space begins —
/// world space for [`project`], the viewport's top-left for [`Camera::to_screen`].
///
/// `y` grows downwards, as it does in every window system and in the art.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ScreenPoint {
    /// Rightwards.
    pub x: i32,
    /// Downwards.
    pub y: i32,
}

/// Where the centre of a tile's diamond falls in world screen space.
///
/// World screen space has its origin at tile `(0, 0, 0)` and is unbounded in
/// both directions: `x` goes negative for anything east of the north corner.
/// It is the camera's job to turn this into somewhere in a window.
pub fn project(point: Point) -> ScreenPoint {
    // Widened before subtracting: `x - y` is negative across half the map, and
    // both are `u16` on the wire.
    let x = i32::from(point.x);
    let y = i32::from(point.y);
    ScreenPoint {
        x: (x - y) * HALF_WIDTH,
        y: (x + y) * HALF_HEIGHT - i32::from(point.z) * Z_STEP,
    }
}

/// The tiles a viewport could show, as an inclusive rectangle in tile space.
///
/// Deliberately a rectangle and not a set: the visible region is a diamond, so
/// this over-covers by roughly half. Drawing a few hundred extra tiles costs
/// less than being clever about it, and *under*-covering is a hole in the world
/// that appears only at one camera angle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TileBounds {
    /// Lowest `x`, inclusive. May be negative: the caller clamps to its map.
    pub min_x: i32,
    /// Highest `x`, inclusive.
    pub max_x: i32,
    /// Lowest `y`, inclusive.
    pub min_y: i32,
    /// Highest `y`, inclusive.
    pub max_y: i32,
}

/// What the view is looking at, and how big the window is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Camera {
    /// The tile at the centre of the viewport. The client follows `0x20`.
    pub center: Point,
    /// Viewport width in pixels.
    pub width: u32,
    /// Viewport height in pixels.
    pub height: u32,
}

impl Camera {
    /// A camera on a tile, for a viewport of this size.
    pub const fn new(center: Point, width: u32, height: u32) -> Self {
        Self {
            center,
            width,
            height,
        }
    }

    /// Where a tile's centre falls inside the viewport, in pixels from its
    /// top-left corner. Outside the viewport is ordinary and not an error.
    pub fn to_screen(&self, point: Point) -> ScreenPoint {
        let origin = project(self.center);
        let at = project(point);
        ScreenPoint {
            x: at.x - origin.x + self.width as i32 / 2,
            y: at.y - origin.y + self.height as i32 / 2,
        }
    }

    /// Every tile that could land inside the viewport, over-covered.
    ///
    /// The inverse of [`project`] is exact — `x - y` and `x + y` recover `x` and
    /// `y` — but only for a known `z`, and `z` is what is stored per tile and
    /// therefore unknown until the tile is read. So the vertical span is widened
    /// by the whole range `z` can lift a tile through, and by one tile for the
    /// sprite's own size. The result is a superset, which is the safe direction.
    pub fn visible_tiles(&self) -> TileBounds {
        let origin = project(self.center);
        let half_w = self.width as i32 / 2;
        let half_h = self.height as i32 / 2;

        // The viewport's rectangle in world screen space, grown by a tile so a
        // diamond straddling the edge still counts, and by the `z` range in
        // *both* directions. Both, because `z` is signed and the two cases look
        // nothing alike: a mountain lifts a tile whose ground position is below
        // the viewport up into it, and a dungeon floor drops a tile from above
        // the viewport down into it. Widening only downwards passes every test
        // written at `z = 0` and loses a band of ground the moment the ground
        // goes negative.
        let left = origin.x - half_w - TILE_WIDTH;
        let right = origin.x + half_w + TILE_WIDTH;
        let top = origin.y - half_h - TILE_HEIGHT - MAX_Z_LIFT;
        let bottom = origin.y + half_h + TILE_HEIGHT + MAX_Z_LIFT;

        // `u = x - y` and `v = x + y`, in tiles. Dividing rounds towards zero,
        // which shrinks the range on the negative side, so each bound is pushed
        // out by one rather than reasoned about.
        let u_min = left / HALF_WIDTH - 1;
        let u_max = right / HALF_WIDTH + 1;
        let v_min = top / HALF_HEIGHT - 1;
        let v_max = bottom / HALF_HEIGHT + 1;

        // `x = (u + v) / 2`, `y = (v - u) / 2`, each extreme taken from the
        // corner of the `(u, v)` rectangle that maximises it.
        TileBounds {
            min_x: (u_min + v_min).div_euclid(2),
            max_x: (u_max + v_max).div_euclid(2) + 1,
            min_y: (v_min - u_max).div_euclid(2),
            max_y: (v_max - u_min).div_euclid(2) + 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four numbers the whole projection is made of. If these move, the art
    /// no longer tiles, so they are written out rather than derived.
    #[test]
    fn a_step_moves_half_a_tile_on_each_axis() {
        assert_eq!(project(Point::new(0, 0, 0)), ScreenPoint { x: 0, y: 0 });
        // East: right and down.
        assert_eq!(project(Point::new(1, 0, 0)), ScreenPoint { x: 22, y: 22 });
        // South: left and down.
        assert_eq!(project(Point::new(0, 1, 0)), ScreenPoint { x: -22, y: 22 });
        // Both: straight down one full tile, and back to the same column.
        assert_eq!(project(Point::new(1, 1, 0)), ScreenPoint { x: 0, y: 44 });
    }

    #[test]
    fn height_lifts_four_pixels_per_unit() {
        assert_eq!(project(Point::new(0, 0, 10)).y, -40);
        assert_eq!(project(Point::new(0, 0, -10)).y, 40);
        // And never sideways: a cliff would shear otherwise.
        assert_eq!(
            project(Point::new(5, 3, 100)).x,
            project(Point::new(5, 3, -100)).x
        );
    }

    #[test]
    fn the_camera_puts_its_own_tile_in_the_middle() {
        let camera = Camera::new(Point::new(1000, 1000, 5), 800, 600);
        assert_eq!(camera.to_screen(camera.center), ScreenPoint { x: 400, y: 300 });
    }

    /// The property that matters: `visible_tiles` may over-cover, but it may
    /// never miss. Anything `to_screen` puts inside the viewport has to be in
    /// the bounds — checked by walking tiles and projecting them, which is the
    /// other formula, so agreement is evidence and not a restatement.
    #[test]
    fn every_tile_that_lands_on_screen_is_inside_the_bounds() {
        let camera = Camera::new(Point::new(1000, 1000, 0), 800, 600);
        let bounds = camera.visible_tiles();

        let mut on_screen = 0;
        for x in 900..1100u16 {
            for y in 900..1100u16 {
                for z in [-120i8, -10, 0, 10, 120] {
                    let point = Point::new(x, y, z);
                    let at = camera.to_screen(point);
                    let inside =
                        at.x >= 0 && at.x < camera.width as i32 && at.y >= 0 && at.y < camera.height as i32;
                    if !inside {
                        continue;
                    }
                    on_screen += 1;
                    assert!(
                        i32::from(x) >= bounds.min_x
                            && i32::from(x) <= bounds.max_x
                            && i32::from(y) >= bounds.min_y
                            && i32::from(y) <= bounds.max_y,
                        "{point} lands at {at:?} but {bounds:?} excludes it",
                    );
                }
            }
        }

        // A pass with nothing on screen would assert nothing at all, and would
        // stay green through any change to either formula.
        assert!(on_screen > 1000, "only {on_screen} tiles landed on screen");
    }

    /// And the over-covering has to stay bounded, or "superset" becomes an
    /// excuse for drawing the map. At 800x600 the viewport holds roughly
    /// 800*600/(44*22) ~ 500 tiles; the `z` slack widens that a lot, but not
    /// without limit.
    #[test]
    fn the_bounds_do_not_grow_without_limit() {
        let bounds = Camera::new(Point::new(1000, 1000, 0), 800, 600).visible_tiles();
        let tiles = (bounds.max_x - bounds.min_x + 1) as i64 * (bounds.max_y - bounds.min_y + 1) as i64;
        assert!(tiles < 40_000, "{bounds:?} covers {tiles} tiles");
    }
}
