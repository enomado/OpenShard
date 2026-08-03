//! A box in world coordinates, and the three faces of it a camera can see.
//!
//! `docs/lighting.md` decision 39: the scene this client draws is already
//! three-dimensional — world space, a per-pixel world position, an orthographic
//! projection and a hardware depth buffer are all in place — and what is missing
//! is a primitive that is not a billboard. This is that primitive's geometry,
//! and it is deliberately only the geometry: a [`Solid`] knows where it stands
//! and hands back polygons, and whether those polygons are painted by egui over
//! the frame or by an instanced quad pass is the caller's business.
//!
//! **Three faces, always the same three.** There is no rotation anywhere in this
//! renderer, so an axis-aligned box always shows `+x`, `+y` and its top, its
//! outline is a hexagon, and the three faces tile that hexagon without
//! overlapping. That is what makes this a handful of arithmetic rather than a
//! mesh pipeline: no index buffer, no back-face culling, nothing to cull against.

use crate::camera::{Camera, WorldSpot, project_exact};
use crate::geometry::Vec2;

/// A box standing in the world: two opposite corners in the world's own units.
///
/// `min` is the low corner on all three axes and `max` the high one, in
/// [`WorldSpot`]'s corner lattice — so the solid that fills tile `(x, y)` from
/// the ground to a height of 20 is `min = (x, y, 0)`, `max = (x+1, y+1, 20)`.
/// Nothing here clips a solid to a tile, and that is decision 38: a solid is a
/// shape the world holds, and the tile grid only *finds* it.
///
/// A degenerate extent is allowed and means what it says — a `min.z == max.z`
/// box is the plane a lid is today, and it draws as a flat diamond rather than
/// as an error. What the geometry cannot express is a rotation, and it never
/// will: see the module doc.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Solid {
    /// The low corner: west, north and bottom.
    pub min: WorldSpot,
    /// The high corner: east, south and top.
    pub max: WorldSpot,
}

/// Which of a solid's three visible faces a polygon is.
///
/// The two vertical ones are [`facing::Face::East`](crate::facing::Face) and
/// `South` — the same two edges of a tile the art draws, for the same reason:
/// this camera looks from the north-west, so `+x` and `+y` are what is turned
/// towards it. The top has no edge and so no `Face`, which is why this is its
/// own small enum rather than a reuse.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    /// The lid, at `max.z`. The face that looks at the sky, and the one that
    /// makes a solid's thickness legible: a wall with no top face drawn is
    /// indistinguishable from the plane it used to be.
    Top,
    /// The `x = max.x` face, on the lower-right of the hexagon.
    East,
    /// The `y = max.y` face, on the lower-left.
    South,
}

impl Solid {
    /// The three faces, in viewport pixels, each as four corners in ring order.
    ///
    /// One projection for all twelve points and it is
    /// [`project_exact`] — the same arithmetic every sprite in the frame is
    /// placed by, so a solid drawn round a wall lands on the wall's own pixels
    /// with nothing fitted. The corners run through the float path end to end
    /// ([`Camera::to_viewport_exact`]): a face a fifth of a tile deep has ends
    /// between the virtual pixels, and rounding each corner on its own would
    /// bend a plane.
    ///
    /// The three polygons tile the solid's hexagonal outline and never overlap,
    /// so a caller painting them translucent gets one weight of colour per pixel
    /// and may draw them in any order.
    pub fn faces(&self, camera: &Camera) -> [(Side, [Vec2; 4]); 3] {
        let at = |x: f64, y: f64, z: f64| {
            camera.to_viewport_exact(camera.to_view_exact(project_exact(WorldSpot { x, y, z })))
        };
        let (lo, hi) = (self.min, self.max);
        [
            // The diamond, in `Camera::tile_facet`'s own order — north corner,
            // east, south, west — so a unit solid's top face and the tile
            // diamond under it are the same four points. The test beside this
            // asserts exactly that.
            (
                Side::Top,
                [
                    at(lo.x, lo.y, hi.z),
                    at(hi.x, lo.y, hi.z),
                    at(hi.x, hi.y, hi.z),
                    at(lo.x, hi.y, hi.z),
                ],
            ),
            // Both vertical faces run top edge first and then down, so the pair
            // of corners a face shares with the top is its first two — which is
            // what a caller wants when it strokes only the silhouette.
            (
                Side::East,
                [
                    at(hi.x, lo.y, hi.z),
                    at(hi.x, hi.y, hi.z),
                    at(hi.x, hi.y, lo.z),
                    at(hi.x, lo.y, lo.z),
                ],
            ),
            (
                Side::South,
                [
                    at(lo.x, hi.y, hi.z),
                    at(hi.x, hi.y, hi.z),
                    at(hi.x, hi.y, lo.z),
                    at(lo.x, hi.y, lo.z),
                ],
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use openshard_protocol::world::Point;

    fn camera() -> Camera {
        Camera::new(
            Point {
                x: 1500,
                y: 1600,
                z: 0,
            },
            800,
            600,
        )
    }

    /// The top of a solid that fills one tile is the tile's own diamond, to the
    /// float — which is the whole claim of decision 39.1: geometry placed in
    /// world coordinates lands in the same pixels the sprite for that tile
    /// lands in, with nothing fitted.
    ///
    /// This is also the test that catches the corner-versus-centre lattice
    /// getting confused, and it catches it as half a tile rather than as
    /// anything subtle.
    #[test]
    fn a_unit_solid_tops_its_tile_diamond() {
        let camera = camera();
        let point = Point {
            x: 1501,
            y: 1602,
            z: 7,
        };
        let solid = Solid {
            min: WorldSpot {
                x: f64::from(point.x),
                y: f64::from(point.y),
                z: 0.0,
            },
            max: WorldSpot {
                x: f64::from(point.x) + 1.0,
                y: f64::from(point.y) + 1.0,
                z: f64::from(point.z),
            },
        };
        let (side, top) = solid.faces(&camera)[0];
        assert_eq!(side, Side::Top);
        let diamond = camera.tile_diamond(point);
        for (got, want) in top.iter().zip(&diamond) {
            assert!(
                (got.x - want.x).abs() < 1e-3 && (got.y - want.y).abs() < 1e-3,
                "corner {got:?} against the tile's own {want:?}"
            );
        }
    }

    /// A solid one `z` tall is 4 pixels tall on the screen and 44 wide, not 44
    /// tall: the projection is anisotropic and the trap decision 39.1 names is
    /// authoring a "real cube" and getting something five and a half times too
    /// high. Pinned here so the number is in a test rather than in a comment.
    #[test]
    fn height_is_four_pixels_a_unit_and_the_ground_is_twenty_two() {
        let camera = camera();
        let flat = |z: f64| Solid {
            min: WorldSpot {
                x: 1500.0,
                y: 1600.0,
                z: 0.0,
            },
            max: WorldSpot {
                x: 1501.0,
                y: 1601.0,
                z,
            },
        };
        let north_of = |solid: Solid| solid.faces(&camera)[0].1[0];
        let ground = north_of(flat(0.0));
        let lifted = north_of(flat(1.0));
        assert!(
            (ground.y - lifted.y - 4.0).abs() < 1e-3,
            "one z should lift 4 pixels, lifted {}",
            ground.y - lifted.y
        );
        // The same solid's north corner against its east one: one tile of `+x`,
        // which is 22 across and 22 down and nothing like the 4 a `z` is.
        let corners = flat(0.0).faces(&camera)[0].1;
        assert!(
            (corners[1].x - corners[0].x - 22.0).abs() < 1e-3
                && (corners[1].y - corners[0].y - 22.0).abs() < 1e-3,
            "one tile east should move 22 across and 22 down, moved {:?}",
            (corners[1].x - corners[0].x, corners[1].y - corners[0].y)
        );
    }

    /// The three faces meet at the solid's own centre-top corner and share their
    /// edges: the top's east corner is the east face's first, the top's south
    /// corner is the south face's first, and both verticals meet at the top's
    /// bottom corner. A box drawn from faces that do not share corners reads as
    /// three loose rhombi, which is the failure this catches.
    #[test]
    fn the_faces_share_their_edges() {
        let camera = camera();
        let solid = Solid {
            min: WorldSpot {
                x: 1500.0,
                y: 1600.0,
                z: 0.0,
            },
            max: WorldSpot {
                x: 1500.2,
                y: 1601.0,
                z: 20.0,
            },
        };
        let faces = solid.faces(&camera);
        let (top, east, south) = (faces[0].1, faces[1].1, faces[2].1);
        assert_eq!(top[1], east[0], "top's east corner is the east face's start");
        assert_eq!(top[2], east[1], "and they share the whole top edge");
        assert_eq!(top[3], south[0], "top's west corner is the south face's start");
        assert_eq!(top[2], south[1], "and the two verticals meet at the near corner");
    }
}
