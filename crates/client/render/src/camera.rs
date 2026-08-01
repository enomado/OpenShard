//! Where a tile lands on the screen, and how to get back.
//!
//! UO's world is a square grid seen from a fixed diagonal, so the projection is
//! two multiplications and no matrix at all. Every number here is the client's,
//! and the client's numbers are not a choice we get to make: a tile is 44 pixels
//! across, a step in `x` moves half a tile right and half a tile down, a step in
//! `y` moves half a tile left and half a tile down, and a unit of height lifts
//! the tile four pixels. Change one of them and the art stops meeting itself at
//! the seams — which is visible only as a shimmer along the diagonals, not as an
//! error, so the values are pinned by tests rather than trusted.
//!
//! # Three spaces, and a type for two of them
//!
//! - **Tile space** — [`Point`], the server's, and the only one that goes on the
//!   wire.
//! - **World pixels** — [`WorldPixel`], what [`project`] returns. The origin is
//!   tile `(0, 0, 0)`, it is unbounded in both directions, and no camera is in
//!   it at all.
//! - **View pixels** — [`ViewPixel`], where a thing lands in the image the world
//!   is drawn into. Origin at its top-left.
//!
//! The two pixel spaces are the same two `i32`s and two different meanings, so
//! they are two types: adding a zoom to one of them while the other still means
//! the first is the kind of mistake a shared type cannot catch. Neither gets
//! `From` or `Into` — the only thing allowed to move between them is a
//! [`Camera`], and a conversion that needs a camera is a method.
//!
//! # Where the zoom is, and where it is not
//!
//! It is not here, apart from the size of the image. The world is drawn at 1:1
//! into an offscreen target of `ceil(viewport / zoom)` and *that* is blitted into
//! the viewport, so every quad, every atlas region and every pixel-exact
//! assertion downstream keeps meaning what it meant. [`ViewPixel`] is a pixel of
//! that offscreen image, which at zoom 1 is the viewport itself.
//!
//! The one place a viewport pixel enters is the cursor, and it leaves in the
//! same call — [`Camera::pick`] takes one and hands back a [`WorldPixel`]. That
//! is why the third space has no type: nothing carries it.

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

/// A position in the world's own pixel space.
///
/// The origin is tile `(0, 0, 0)` and it is unbounded in both directions: `x`
/// goes negative for anything east of the north corner. `y` grows downwards, as
/// it does in every window system and in the art.
///
/// It is the camera's job to turn this into somewhere in an image.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct WorldPixel {
    /// Rightwards.
    pub x: i32,
    /// Downwards.
    pub y: i32,
}

/// A position in the image the world is drawn into, from its top-left corner.
///
/// Outside it is ordinary and not an error — most of what a camera projects
/// lands off the edge.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ViewPixel {
    /// Rightwards.
    pub x: i32,
    /// Downwards.
    pub y: i32,
}

/// Where the centre of a tile's diamond falls in world pixel space.
pub fn project(point: Point) -> WorldPixel {
    // Widened before subtracting: `x - y` is negative across half the map, and
    // both are `u16` on the wire.
    let x = i32::from(point.x);
    let y = i32::from(point.y);
    WorldPixel {
        x: (x - y) * HALF_WIDTH,
        y: (x + y) * HALF_HEIGHT - i32::from(point.z) * Z_STEP,
    }
}

/// Which tile a world pixel falls on, given the height to read it at.
///
/// The named inverse of [`project`], and exact: `project` is a linear map with
/// determinant `2 * HALF_WIDTH * HALF_HEIGHT`, so `x` and `y` come back out of
/// `x - y` and `x + y` with nothing lost. `z` has to be supplied because the
/// projection folds it into the vertical axis — a pixel on the screen is a whole
/// column of tiles at different heights, which is the entire difficulty of
/// picking and is why this takes the height rather than guessing one.
///
/// Tiles come back as `i32` and not `u16` for the same reason [`TileBounds`]
/// holds `i32`: world pixel space is unbounded, so a pixel north of the map's
/// corner has a negative tile, and clamping here would invent one. The caller
/// knows its map; this knows arithmetic.
pub fn unproject(at: WorldPixel, z: i8) -> (i32, i32) {
    // Undo the lift first, and the rest is `u = x - y`, `v = x + y` scaled by a
    // half tile: `a + b` is `44x` and `b - a` is `44y`.
    let a = at.x;
    let b = at.y + i32::from(z) * Z_STEP;
    // Rounded to the nearest tile rather than floored, so a pixel one short of a
    // centre names the tile it is nearly on. `div_euclid` because the numerator
    // is negative across half the map and truncation would round towards the
    // origin from one side and away from it on the other.
    (
        (a + b + HALF_WIDTH).div_euclid(TILE_WIDTH),
        (b - a + HALF_HEIGHT).div_euclid(TILE_HEIGHT),
    )
}

/// How far the world is magnified, as an exact ratio.
///
/// A fraction from a fixed ladder and not an `f32`, for three reasons and the
/// third decides it: [`Camera`] is `Copy + Eq` and several tests compare
/// cameras, which a float field takes away; the offscreen target's size has to
/// come out the same integer every frame or the world is reallocated on rounding
/// noise; and a ladder is what a wheel notch wants anyway.
///
/// The numerator and denominator are private and the only way along the ladder
/// is [`Zoom::scale_up`] and [`Zoom::scale_down`], so a zoom off the end of it
/// is not expressible.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Zoom {
    /// An index into [`LADDER`]. The value itself is never arithmetic.
    step: u8,
}

/// Every zoom the wheel can reach, magnifying left to right.
///
/// Below 1 the world is minified and the offscreen target grows, which is what
/// [`Camera::render_width`] and the GPU's texture limit have to agree about.
const LADDER: [(u32, u32); 9] = [
    (1, 2),
    (2, 3),
    (3, 4),
    (1, 1),
    (4, 3),
    (3, 2),
    (2, 1),
    (3, 1),
    (4, 1),
];

/// Where `1:1` sits in [`LADDER`].
const ONE_STEP: u8 = 3;

impl Zoom {
    /// One world pixel to one viewport pixel.
    pub const ONE: Self = Self { step: ONE_STEP };

    /// The magnification's numerator: viewport pixels per `den` world pixels.
    pub const fn numerator(self) -> u32 {
        LADDER[self.step as usize].0
    }

    /// Its denominator.
    pub const fn denominator(self) -> u32 {
        LADDER[self.step as usize].1
    }

    /// One notch in, stopping at the top of the ladder.
    pub const fn scale_up(self) -> Self {
        let step = if self.step + 1 < LADDER.len() as u8 {
            self.step + 1
        } else {
            self.step
        };
        Self { step }
    }

    /// One notch out, stopping at the bottom.
    pub const fn scale_down(self) -> Self {
        let step = if self.step > 0 { self.step - 1 } else { self.step };
        Self { step }
    }

    /// Whether this is the widest view the ladder offers.
    pub const fn is_widest(self) -> bool {
        self.step == 0
    }

    /// How many world pixels a run of `viewport` pixels covers, rounded up.
    ///
    /// Up, because the offscreen image is blitted at exactly this ratio and a
    /// short image would leave a strip of the viewport undrawn. Rounding up
    /// instead spills a fraction of a pixel past the edges, where it is clipped.
    const fn world_pixels(self, viewport: u32) -> u32 {
        let (num, den) = LADDER[self.step as usize];
        // At least one pixel: a zero-sized viewport is a minimised window, not
        // an error, and a texture of zero width is.
        let pixels = viewport.div_ceil(num).saturating_mul(den);
        if pixels == 0 { 1 } else { pixels }
    }
}

impl std::fmt::Display for Zoom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (num, den) = LADDER[self.step as usize];
        if den == 1 {
            write!(f, "{num}x")
        } else {
            write!(f, "{num}/{den}x")
        }
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

impl TileBounds {
    /// The same rectangle with everything outside a map of this size removed.
    ///
    /// `None` when nothing is left, which happens for a camera looking off the
    /// edge of a small facet — not an error, just an empty frame.
    ///
    /// Shared by the ground and the statics deliberately: they walk the same
    /// cells, and two copies of "clamp the bounds to the map" is two chances to
    /// draw a static on a tile whose ground was dropped.
    pub fn clamp_to(
        self,
        width: u32,
        height: u32,
    ) -> Option<(std::ops::RangeInclusive<u16>, std::ops::RangeInclusive<u16>)> {
        if width == 0 || height == 0 {
            return None;
        }
        let min_x = self.min_x.max(0) as u32;
        let min_y = self.min_y.max(0) as u32;
        let max_x = (self.max_x.max(0) as u32).min(width - 1);
        let max_y = (self.max_y.max(0) as u32).min(height - 1);
        if min_x > max_x || min_y > max_y {
            return None;
        }
        // Every bound fits a `u16` because it was clamped to the map's size, and
        // no facet is wider than 7,168 tiles.
        Some((min_x as u16..=max_x as u16, min_y as u16..=max_y as u16))
    }

    /// The parts of this rectangle that `covered` does not already contain.
    ///
    /// Up to four rectangles — a band above, a band below, and what is left of
    /// the rows between them on either side — and none at all when `covered`
    /// contains this one, which is the ordinary frame.
    ///
    /// This is what makes the atlases' growth proportional to the camera's
    /// *movement* rather than to the viewport. Asking "does the atlas hold
    /// every graphic on screen" walks nine thousand cells at 1080p on every
    /// frame to answer a question about the edge the camera just crossed; a step
    /// of one tile crosses one row of it. The invariant that makes the answer
    /// sound is positional and belongs to the caller: every cell inside
    /// `covered` has already been offered to the atlas, so a cell outside it is
    /// the only place a graphic can be new.
    ///
    /// Saturating, because tile space is unbounded in both directions here — see
    /// this type's own note — and `covered.min_x - 1` at `i32::MIN` is not a
    /// rectangle anybody asked for.
    pub fn difference(self, covered: TileBounds) -> [Option<TileBounds>; 4] {
        // Disjoint: nothing to subtract, and doing the arithmetic anyway would
        // produce the two bands *and* the two sides of an empty middle.
        if self.max_x < covered.min_x
            || self.min_x > covered.max_x
            || self.max_y < covered.min_y
            || self.min_y > covered.max_y
        {
            return [Some(self), None, None, None];
        }

        let rect = |min_x: i32, max_x: i32, min_y: i32, max_y: i32| {
            (min_x <= max_x && min_y <= max_y).then_some(TileBounds {
                min_x,
                max_x,
                min_y,
                max_y,
            })
        };
        // The rows the two rectangles share, which are the only ones with a left
        // and a right piece: above and below them the whole width is uncovered.
        let (from, to) = (self.min_y.max(covered.min_y), self.max_y.min(covered.max_y));
        [
            rect(
                self.min_x,
                self.max_x,
                self.min_y,
                self.max_y.min(covered.min_y.saturating_sub(1)),
            ),
            rect(
                self.min_x,
                self.max_x,
                self.min_y.max(covered.max_y.saturating_add(1)),
                self.max_y,
            ),
            rect(
                self.min_x,
                self.max_x.min(covered.min_x.saturating_sub(1)),
                from,
                to,
            ),
            rect(
                self.min_x.max(covered.max_x.saturating_add(1)),
                self.max_x,
                from,
                to,
            ),
        ]
    }
}

/// What the view is looking at, how magnified, and how big the viewport is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Camera {
    /// Where the middle of the viewport looks.
    ///
    /// Pixels and not a tile: a tile is 44 pixels across and a drag is one pixel
    /// at a time. Whole world pixels, too — an eye carrying a fraction would put
    /// every sprite on a half-texel boundary for half of all camera positions,
    /// so a drag accumulates its remainder in the input handler and commits
    /// whole pixels here.
    ///
    /// Private, because "where the camera looks" is the one piece of state two
    /// writers fight over: the thing that pins it to the player, and the thing
    /// that pans it. [`Camera::look_at_pixel`] is the one door — and it takes a
    /// pixel rather than a tile because everything upstream of it has one: a
    /// body mid-step is between two tiles, and naming the tile throws away the
    /// part of the answer the whole glide exists to produce.
    eye: WorldPixel,
    zoom: Zoom,
    /// The viewport's width in *physical* pixels — the rect the UI leaves free,
    /// which is not the window.
    pub width: u32,
    /// Its height, likewise.
    pub height: u32,
}

impl Camera {
    /// A camera on a tile, unmagnified, for a viewport of this size.
    pub fn new(center: Point, width: u32, height: u32) -> Self {
        Self {
            eye: project(center),
            zoom: Zoom::ONE,
            width,
            height,
        }
    }

    /// Where the middle of the viewport looks.
    pub fn eye(&self) -> WorldPixel {
        self.eye
    }

    /// Look at a world pixel.
    pub fn look_at_pixel(&mut self, eye: WorldPixel) {
        self.eye = eye;
    }

    /// The tile the eye is over, read at ground level.
    ///
    /// What [`crate::depth`] wants for its `base`: the ordering is centred on
    /// the camera so the visible frame sits where the depth buffer has
    /// resolution to spare, and `z` does not matter to that at all — it moves
    /// the answer by a tile or two out of a margin of five hundred.
    pub fn eye_tile(&self) -> (i32, i32) {
        unproject(self.eye, 0)
    }

    /// The magnification.
    pub fn zoom(&self) -> Zoom {
        self.zoom
    }

    /// The width of the image the world is drawn into, in world pixels.
    ///
    /// Bigger than the viewport when minifying, smaller when magnifying. This is
    /// the offscreen texture's size, and the GPU's `max_texture_dimension_2d` is
    /// what bounds how far the ladder can be walked down — see the caller that
    /// clamps it.
    pub fn render_width(&self) -> u32 {
        self.zoom.world_pixels(self.width)
    }

    /// The height of that image.
    pub fn render_height(&self) -> u32 {
        self.zoom.world_pixels(self.height)
    }

    /// Where a world pixel lands in the drawn image.
    pub fn to_view(&self, at: WorldPixel) -> ViewPixel {
        ViewPixel {
            x: at.x - self.eye.x + self.render_width() as i32 / 2,
            y: at.y - self.eye.y + self.render_height() as i32 / 2,
        }
    }

    /// And back. The exact inverse of [`Camera::to_view`] — integers throughout,
    /// because the zoom is not in either of them.
    pub fn to_world(&self, at: ViewPixel) -> WorldPixel {
        WorldPixel {
            x: at.x - self.render_width() as i32 / 2 + self.eye.x,
            y: at.y - self.render_height() as i32 / 2 + self.eye.y,
        }
    }

    /// Where a tile's centre falls in the drawn image, in pixels from its
    /// top-left corner. Outside it is ordinary and not an error.
    pub fn to_screen(&self, point: Point) -> ViewPixel {
        self.to_view(project(point))
    }

    /// Where a drawn-image pixel lands in the viewport, after the blit's scale.
    ///
    /// [`Camera::to_view`] stops at the offscreen image, which is drawn 1:1
    /// and then blitted into the viewport at exactly the ratio
    /// [`Zoom::world_pixels`] used to size it — so the inverse of that ratio is
    /// what carries a render-space point the rest of the way to a pixel a
    /// painter can use. `f32` because a highlight is drawn at whatever
    /// magnification the blit lands on, not on a texel grid.
    pub fn to_viewport(&self, at: ViewPixel) -> (f32, f32) {
        let num = self.zoom.numerator() as f32;
        let den = self.zoom.denominator() as f32;
        let scale = num / den;
        (
            (at.x - self.render_width() as i32 / 2) as f32 * scale + self.width as f32 / 2.0,
            (at.y - self.render_height() as i32 / 2) as f32 * scale + self.height as f32 / 2.0,
        )
    }

    /// The four corners of a tile's diamond, in viewport pixels — top, right,
    /// bottom, left.
    ///
    /// Read off the same square every ground quad is drawn from: the art is
    /// 44 pixels on a side in render space regardless of zoom, and only the
    /// blit in [`Camera::to_viewport`] scales it, so the offsets below are
    /// taken before that conversion and not after.
    pub fn tile_diamond(&self, point: Point) -> [(f32, f32); 4] {
        let centre = self.to_screen(point);
        let half = TILE_WIDTH / 2;
        [
            ViewPixel {
                x: centre.x,
                y: centre.y - half,
            },
            ViewPixel {
                x: centre.x + half,
                y: centre.y,
            },
            ViewPixel {
                x: centre.x,
                y: centre.y + half,
            },
            ViewPixel {
                x: centre.x - half,
                y: centre.y,
            },
        ]
        .map(|corner| self.to_viewport(corner))
    }

    /// What world pixel the cursor is over, given where it is in the viewport.
    ///
    /// The one place a *viewport* pixel is spoken about, and it does not escape:
    /// the zoom is undone here and a world pixel comes out. Lossy in the
    /// magnifying direction by construction — several viewport pixels share one
    /// world pixel at zoom 4 — which is the honest answer, since the world has
    /// no finer position to name.
    pub fn pick(&self, x: i32, y: i32) -> WorldPixel {
        let den = self.zoom.denominator() as i32;
        let num = self.zoom.numerator() as i32;
        // About the centre, because that is where the blit is anchored: the
        // offscreen image is drawn over the viewport rect whole, so the two
        // centres coincide whatever the rounding did to the edges.
        let dx = (x - self.width as i32 / 2) * den / num;
        let dy = (y - self.height as i32 / 2) * den / num;
        WorldPixel {
            x: self.eye.x + dx,
            y: self.eye.y + dy,
        }
    }

    /// Change the magnification, keeping whatever is under the cursor there.
    ///
    /// The whole reason the inverse exists. Hold `pick(cursor)` fixed across the
    /// change and solve for the new eye — one line, and it is the difference
    /// between a camera that feels placed and one that feels shoved.
    pub fn zoom_about(&mut self, cursor_x: i32, cursor_y: i32, zoom: Zoom) {
        let before = self.pick(cursor_x, cursor_y);
        self.zoom = zoom;
        let after = self.pick(cursor_x, cursor_y);
        self.eye = WorldPixel {
            x: self.eye.x + before.x - after.x,
            y: self.eye.y + before.y - after.y,
        };
    }

    /// Every tile that could land inside the drawn image, over-covered.
    ///
    /// The inverse of [`project`] is exact — see [`unproject`] — but only for a
    /// known `z`, and `z` is what is stored per tile and therefore unknown until
    /// the tile is read. So the vertical span is widened by the whole range `z`
    /// can lift a tile through, and by one tile for the sprite's own size. The
    /// result is a superset, which is the safe direction.
    ///
    /// Zoomed out this covers more, because the image it is covering *is*
    /// bigger: nothing here reads the zoom, only the size it produced.
    pub fn visible_tiles(&self) -> TileBounds {
        let half_w = self.render_width() as i32 / 2;
        let half_h = self.render_height() as i32 / 2;

        // The image's rectangle in world pixel space, grown by a tile so a
        // diamond straddling the edge still counts, and by the `z` range in
        // *both* directions. Both, because `z` is signed and the two cases look
        // nothing alike: a mountain lifts a tile whose ground position is below
        // the viewport up into it, and a dungeon floor drops a tile from above
        // the viewport down into it. Widening only downwards passes every test
        // written at `z = 0` and loses a band of ground the moment the ground
        // goes negative.
        let left = self.eye.x - half_w - TILE_WIDTH;
        let right = self.eye.x + half_w + TILE_WIDTH;
        let top = self.eye.y - half_h - TILE_HEIGHT - MAX_Z_LIFT;
        let bottom = self.eye.y + half_h + TILE_HEIGHT + MAX_Z_LIFT;

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
        assert_eq!(project(Point::new(0, 0, 0)), WorldPixel { x: 0, y: 0 });
        // East: right and down.
        assert_eq!(project(Point::new(1, 0, 0)), WorldPixel { x: 22, y: 22 });
        // South: left and down.
        assert_eq!(project(Point::new(0, 1, 0)), WorldPixel { x: -22, y: 22 });
        // Both: straight down one full tile, and back to the same column.
        assert_eq!(project(Point::new(1, 1, 0)), WorldPixel { x: 0, y: 44 });
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

    /// The inverse is an inverse — over the whole `z` range, because that is the
    /// axis it has to be told about and therefore the one that can be wired up
    /// wrongly and still pass at `z = 0`.
    #[test]
    fn unproject_undoes_project() {
        for x in [0u16, 1, 2, 511, 1495, 6143] {
            for y in [0u16, 1, 3, 512, 1629, 4095] {
                for z in [i8::MIN, -37, -1, 0, 1, 44, i8::MAX] {
                    let point = Point::new(x, y, z);
                    assert_eq!(
                        unproject(project(point), z),
                        (i32::from(x), i32::from(y)),
                        "{point} did not come back",
                    );
                }
            }
        }
    }

    /// A pixel that is not a tile centre names the tile it is nearest, and the
    /// north-west of the map is where truncation would have named a different
    /// one — which is the whole reason for `div_euclid`.
    #[test]
    fn unproject_rounds_to_the_nearest_tile_on_both_sides_of_the_origin() {
        // A few pixels either side of a centre still name that tile.
        let centre = project(Point::new(100, 100, 0));
        for (dx, dy) in [(0, 0), (5, 0), (-5, 0), (0, 5), (0, -5), (-10, -10)] {
            let near = WorldPixel {
                x: centre.x + dx,
                y: centre.y + dy,
            };
            assert_eq!(unproject(near, 0), (100, 100), "{near:?}");
        }
        // North of tile (0, 0) is a negative tile, and it is reported as one
        // rather than clamped into the map.
        let above = WorldPixel { x: 0, y: -44 };
        assert_eq!(unproject(above, 0), (-1, -1));
    }

    #[test]
    fn the_camera_puts_its_own_tile_in_the_middle() {
        let camera = Camera::new(Point::new(1000, 1000, 5), 800, 600);
        assert_eq!(
            camera.to_screen(Point::new(1000, 1000, 5)),
            ViewPixel { x: 400, y: 300 }
        );
    }

    /// The rule the whole camera hangs on. Checked at every rung, because the
    /// image's size changes with the zoom and a half that used the viewport's
    /// would agree with the other half only at 1:1.
    #[test]
    fn to_world_is_the_inverse_of_to_view_at_every_zoom() {
        let mut camera = Camera::new(Point::new(1495, 1629, 0), 1024, 768);
        let mut zoom = Zoom::ONE;
        while !zoom.is_widest() {
            zoom = zoom.scale_down();
        }
        loop {
            camera.zoom = zoom;
            for at in [
                WorldPixel { x: 0, y: 0 },
                camera.eye(),
                WorldPixel { x: -12_345, y: 6 },
                WorldPixel {
                    x: 100_000,
                    y: -70_000,
                },
            ] {
                assert_eq!(camera.to_world(camera.to_view(at)), at, "at {zoom}");
            }
            let next = zoom.scale_up();
            if next == zoom {
                break;
            }
            zoom = next;
        }
    }

    /// The ladder is walked, not indexed: nothing outside it is reachable.
    #[test]
    fn the_zoom_ladder_has_two_ends() {
        let mut zoom = Zoom::ONE;
        for _ in 0..20 {
            zoom = zoom.scale_up();
        }
        assert_eq!((zoom.numerator(), zoom.denominator()), (4, 1));
        assert!(!zoom.is_widest());
        for _ in 0..20 {
            zoom = zoom.scale_down();
        }
        assert_eq!((zoom.numerator(), zoom.denominator()), (1, 2));
        assert!(zoom.is_widest());
    }

    /// Zoomed out the offscreen image is bigger than the viewport, zoomed in
    /// smaller, and never short — a short image leaves a strip of the viewport
    /// with nothing blitted into it.
    #[test]
    fn the_drawn_image_covers_the_viewport_at_every_zoom() {
        let mut camera = Camera::new(Point::new(1000, 1000, 0), 1024, 768);
        let mut zoom = Zoom::ONE;
        while !zoom.is_widest() {
            zoom = zoom.scale_down();
        }
        loop {
            camera.zoom = zoom;
            let (num, den) = (zoom.numerator(), zoom.denominator());
            assert!(
                camera.render_width() * num >= camera.width * den,
                "{zoom} leaves {}px short of {}",
                camera.render_width(),
                camera.width,
            );
            // And not wastefully long: one world pixel of slack at most.
            assert!(
                camera.render_width() * num < (camera.width + num) * den,
                "{zoom} overshoots"
            );
            let next = zoom.scale_up();
            if next == zoom {
                break;
            }
            zoom = next;
        }
    }

    /// Zooming holds the cursor still. Not exactly — a world pixel is coarser
    /// than a viewport pixel when magnified, so the answer is only as precise as
    /// the space it is expressed in — but within one world pixel at every rung,
    /// which is what "feels placed" means.
    #[test]
    fn zooming_keeps_what_is_under_the_cursor_under_it() {
        let mut camera = Camera::new(Point::new(1495, 1629, 0), 1024, 768);
        let (cx, cy) = (200, 700);
        let before = camera.pick(cx, cy);
        let mut zoom = camera.zoom();
        for _ in 0..8 {
            zoom = zoom.scale_up();
            camera.zoom_about(cx, cy, zoom);
            let after = camera.pick(cx, cy);
            assert!(
                (after.x - before.x).abs() <= 1 && (after.y - before.y).abs() <= 1,
                "{zoom}: {before:?} drifted to {after:?}",
            );
        }
    }

    /// The property that matters: `visible_tiles` may over-cover, but it may
    /// never miss. Anything `to_screen` puts inside the image has to be in the
    /// bounds — checked by walking tiles and projecting them, which is the other
    /// formula, so agreement is evidence and not a restatement.
    ///
    /// Re-run at every rung of the ladder, because the image the bounds cover
    /// grows with it.
    #[test]
    fn every_tile_that_lands_on_screen_is_inside_the_bounds() {
        let mut camera = Camera::new(Point::new(1000, 1000, 0), 800, 600);
        let mut zoom = Zoom::ONE;
        while !zoom.is_widest() {
            zoom = zoom.scale_down();
        }
        loop {
            camera.zoom = zoom;
            let bounds = camera.visible_tiles();
            let (width, height) = (camera.render_width() as i32, camera.render_height() as i32);

            let mut on_screen = 0;
            for x in 900..1100u16 {
                for y in 900..1100u16 {
                    for z in [-120i8, -10, 0, 10, 120] {
                        let point = Point::new(x, y, z);
                        let at = camera.to_screen(point);
                        if at.x < 0 || at.x >= width || at.y < 0 || at.y >= height {
                            continue;
                        }
                        on_screen += 1;
                        assert!(
                            i32::from(x) >= bounds.min_x
                                && i32::from(x) <= bounds.max_x
                                && i32::from(y) >= bounds.min_y
                                && i32::from(y) <= bounds.max_y,
                            "at {zoom}, {point} lands at {at:?} but {bounds:?} excludes it",
                        );
                    }
                }
            }

            // A pass with nothing on screen would assert nothing at all, and
            // would stay green through any change to either formula. The floor
            // is the image's own area in tiles — one diamond covers
            // `TILE_WIDTH * HALF_HEIGHT` pixels — because a magnified image
            // genuinely holds fewer tiles and a constant here would either fail
            // at 4x or assert nothing at 1/2x.
            let floor = width as i64 * height as i64 / (TILE_WIDTH * HALF_HEIGHT) as i64;
            assert!(
                i64::from(on_screen) > floor,
                "at {zoom}, only {on_screen} tiles landed on screen, against {floor}",
            );
            let next = zoom.scale_up();
            if next == zoom {
                break;
            }
            zoom = next;
        }
    }

    /// And the over-covering has to stay bounded, or "superset" becomes an
    /// excuse for drawing the map.
    ///
    /// A constant would not do it: zoomed out the image is four times the area,
    /// so the bound has to be a statement about the image's size rather than a
    /// number that happens to hold at 1:1. This is that statement, derived from
    /// the formula's own terms — the `u` and `v` spans it computes, converted to
    /// a square of tiles — with a tile of slack per bound for the roundings.
    #[test]
    fn the_bounds_do_not_grow_faster_than_the_image() {
        let mut camera = Camera::new(Point::new(1000, 1000, 0), 800, 600);
        let mut zoom = Zoom::ONE;
        while !zoom.is_widest() {
            zoom = zoom.scale_down();
        }
        loop {
            camera.zoom = zoom;
            let bounds = camera.visible_tiles();
            let tiles = (bounds.max_x - bounds.min_x + 1) as i64 * (bounds.max_y - bounds.min_y + 1) as i64;

            let u_span = (camera.render_width() as i64 + 2 * TILE_WIDTH as i64) / HALF_WIDTH as i64 + 4;
            let v_span = (camera.render_height() as i64 + 2 * TILE_HEIGHT as i64 + 2 * MAX_Z_LIFT as i64)
                / HALF_HEIGHT as i64
                + 4;
            let side = (u_span + v_span) / 2 + 4;
            assert!(
                tiles <= side * side,
                "at {zoom}, {bounds:?} covers {tiles} tiles against {}",
                side * side,
            );
            let next = zoom.scale_up();
            if next == zoom {
                break;
            }
            zoom = next;
        }
    }
}

#[cfg(test)]
mod difference_tests {
    use super::TileBounds;

    fn bounds(min_x: i32, max_x: i32, min_y: i32, max_y: i32) -> TileBounds {
        TileBounds {
            min_x,
            max_x,
            min_y,
            max_y,
        }
    }

    /// Every cell in a rectangle, as a set a test can compare.
    fn cells(rect: TileBounds) -> std::collections::BTreeSet<(i32, i32)> {
        let mut out = std::collections::BTreeSet::new();
        for y in rect.min_y..=rect.max_y {
            for x in rect.min_x..=rect.max_x {
                out.insert((x, y));
            }
        }
        out
    }

    /// The property the whole band walk rests on, stated over every pair of
    /// rectangles in a small window: the pieces are exactly the cells of the
    /// first that the second does not hold, and no cell appears twice.
    ///
    /// Both halves matter and they fail differently. A piece that is *missing*
    /// is a graphic never offered to the atlas, which draws nothing where it
    /// should have drawn a wall — silently, and only along one edge, and only
    /// for the camera direction that produced it. A cell counted *twice* is
    /// merely work done twice, which nothing would ever notice; asserting it
    /// anyway is what keeps a lazily-widened rectangle from becoming the
    /// "walk the whole viewport" this exists to replace.
    #[test]
    fn the_pieces_are_exactly_what_is_not_covered() {
        let range = -2..=2;
        for min_x in range.clone() {
            for max_x in min_x..=2 {
                for min_y in range.clone() {
                    for max_y in min_y..=2 {
                        let want = bounds(min_x, max_x, min_y, max_y);
                        for cx in range.clone() {
                            for cy in range.clone() {
                                let covered = bounds(cx, cx + 1, cy, cy + 2);
                                let pieces = want.difference(covered);

                                let mut union = std::collections::BTreeSet::new();
                                let mut total = 0;
                                for piece in pieces.into_iter().flatten() {
                                    let piece = cells(piece);
                                    total += piece.len();
                                    union.extend(piece);
                                }
                                assert_eq!(union.len(), total, "{want:?} minus {covered:?} overlaps itself");

                                let expected: std::collections::BTreeSet<(i32, i32)> =
                                    cells(want).difference(&cells(covered)).copied().collect();
                                assert_eq!(union, expected, "{want:?} minus {covered:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    /// A camera that has not moved asks for nothing, which is the frame this
    /// whole arrangement exists to make free.
    #[test]
    fn a_rectangle_inside_the_covered_one_has_no_pieces() {
        let covered = bounds(0, 100, 0, 100);
        assert!(
            covered
                .difference(covered)
                .into_iter()
                .all(|piece| piece.is_none())
        );
        let inside = bounds(10, 20, 10, 20);
        assert!(
            inside
                .difference(covered)
                .into_iter()
                .all(|piece| piece.is_none())
        );
    }

    /// A step of one tile is one row, not a viewport. The number is the whole
    /// point of the band walk, so it is asserted rather than described.
    #[test]
    fn a_step_of_one_tile_uncovers_one_row() {
        let covered = bounds(0, 99, 0, 99);
        let moved = bounds(0, 99, 1, 100);
        let cells: usize = moved
            .difference(covered)
            .into_iter()
            .flatten()
            .map(|piece| cells(piece).len())
            .sum();
        assert_eq!(cells, 100, "one row of a 100-wide rectangle");
    }

    /// Nothing in common: the whole rectangle is new. A teleport, or a facet
    /// wide enough that the camera left everything it knew.
    #[test]
    fn a_disjoint_rectangle_is_uncovered_whole() {
        let covered = bounds(0, 10, 0, 10);
        let elsewhere = bounds(100, 110, 100, 110);
        let pieces: Vec<TileBounds> = elsewhere.difference(covered).into_iter().flatten().collect();
        assert_eq!(pieces, vec![elsewhere]);
    }

    /// The saturating arithmetic, at the edge that would wrap without it.
    #[test]
    fn the_extremes_do_not_wrap() {
        let covered = bounds(i32::MIN, i32::MAX, i32::MIN, i32::MAX);
        let want = bounds(-5, 5, -5, 5);
        assert!(want.difference(covered).into_iter().all(|piece| piece.is_none()));
    }
}
