//! Placing a creature's animation frame on the screen.
//!
//! The CPU side, as [`crate::statics`] is for what stands on the ground — and
//! deliberately smaller, because a mobile is not read out of a file the way a
//! static is. The map says where every static is; nothing says where a mobile
//! is except the server, so what arrives here is a list somebody else built.
//!
//! # A mobile is placed from its frame, not from its tile
//!
//! A static sits in the middle of its cell. A mobile's sprite is placed by the
//! two centre offsets stored with the frame — `MobileView.Draw`:
//!
//! ```text
//! x -= flipped ? width - center_x : center_x
//! y -= height + center_y
//! ```
//!
//! against the cell's centre. That asymmetry is what the mirrored facings are
//! for: flipping a picture moves its anchor to the other edge, and using
//! `center_x` for both makes every west-facing creature stand a body's width
//! from where it is.
//!
//! # And between two tiles while it is walking
//!
//! The wire moves a body a whole tile at a time, so a mobile drawn where the
//! last packet put it teleports 44 pixels every step. [`Glide`] is the
//! difference: the tile stepped off and how far into the step the body is, and
//! the sprite hangs between the two projections. Nothing else changes — the
//! *tile* is still where the server put it, which is what everything that is
//! not a pixel keeps asking.

use openshard_protocol::direction::Direction;
use openshard_protocol::wire::Hue;
use openshard_protocol::world::Point;

use crate::atlas::{AnimAtlas, FrameKey};
use crate::camera::{self, Camera, ViewPixel, WorldPixel};
use crate::depth;
use crate::sprite::SpriteQuad;

/// A step still being taken, in the only two numbers drawing one needs.
///
/// Where the body came from and how far along it is. The tile it is going to is
/// [`Mobile::at`] — a step's destination is where the server says the body *is*,
/// and the glide is the part of it the eye has not caught up with yet.
///
/// Not `Eq`, and neither is [`Mobile`] any more: [`Glide::progress`] is a
/// fraction of a step and there is no honest integer for it. Time is what
/// advances it, and time does not land on tile boundaries.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Glide {
    /// The tile the body stepped off, height and all.
    ///
    /// The height too, so a step up a hill rises over the step rather than
    /// snapping four pixels at the end of it — [`crate::camera::project`] is
    /// linear in `z`, so this costs nothing extra.
    pub from: Point,
    /// How far into the step: `0.0` at [`Glide::from`], `1.0` at [`Mobile::at`].
    ///
    /// Clamped by whoever reads it rather than by construction, because a value
    /// outside the range means a clock ran past the step and the honest picture
    /// of that is a body standing on its destination.
    pub progress: f32,
}

/// One creature to draw, as the client knows it.
///
/// Everything here comes from the server — `0x77`, `0x78` and `0x20` — except
/// the frame, which is whatever the animation has advanced to, and the glide,
/// which is *history*: only the previous packet says a body moved. Deliberately
/// a plain value rather than a handle into a `WorldView`: this crate renders
/// what it is given and owns no model of the world.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Mobile {
    /// Where it stands — or, mid-step, where it is going.
    pub at: Point,
    /// Its body id.
    pub body: u16,
    /// Which animation group is playing.
    pub group: u8,
    /// Which way it is looking.
    ///
    /// The facing, not the stored direction: turning one into the other is
    /// [`openshard_uofiles::anim::facing`], and doing it here rather than in the
    /// caller is what keeps the mirror and the placement from being decided in
    /// two different places.
    pub facing: Direction,
    /// Which frame of that animation.
    pub frame: u16,
    /// Its hue, or [`Hue::NONE`] for none.
    pub hue: Hue,
    /// The step it is in the middle of, or `None` for a body standing still.
    ///
    /// [`Option`] in its proper sense: a standing body is not missing a glide,
    /// it genuinely has none.
    pub glide: Option<Glide>,
}

/// Where a mobile's cell centre falls in world pixels, the step it is mid-way
/// through included.
///
/// The whole of the glide, in one place so that only the *position* moves: the
/// depth order, the atlas key and the anchor arithmetic all still read
/// [`Mobile::at`], which is the tile the server named.
///
/// World pixels and not view pixels, and public for it: the camera that follows
/// a body has to follow it *between* tiles too, or the world jumps a tile at a
/// time under a character sliding smoothly across it — and an eye is a
/// [`WorldPixel`], with no camera to convert with.
pub fn world_position(mobile: &Mobile) -> WorldPixel {
    let at = camera::project(mobile.at);
    let Some(glide) = mobile.glide else {
        return at;
    };
    // Backwards from the destination by the part of the step not yet walked,
    // which is the form that lands *exactly* on `at` when the step is over — a
    // lerp forwards from `from` would leave a rounding pixel behind at the end
    // of every step, and a body that never quite arrives shimmers.
    let left = 1.0 - glide.progress.clamp(0.0, 1.0);
    let from = camera::project(glide.from);
    WorldPixel {
        x: at.x - (((at.x - from.x) as f32) * left).round() as i32,
        y: at.y - (((at.y - from.y) as f32) * left).round() as i32,
    }
}

/// The same, where the camera puts it in the drawn image.
fn cell_centre(mobile: &Mobile, camera: &Camera) -> ViewPixel {
    camera.to_view(world_position(mobile))
}

/// The body, group and stored direction a set of mobiles needs packed.
///
/// Called before building the atlas, like its neighbours: the atlas has to
/// exist before a quad can be given a region. The frame index is absent on
/// purpose — [`AnimAtlas::build`] packs a whole animation at a time, because
/// reading one frame of it costs the same as reading all of them.
pub fn needed_animations(mobiles: &[Mobile]) -> Vec<(u16, u8, u8)> {
    let mut wanted: Vec<(u16, u8, u8)> = mobiles
        .iter()
        .map(|mobile| {
            let (direction, _) = openshard_uofiles::anim::facing(mobile.facing);
            (mobile.body, mobile.group, direction)
        })
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    wanted
}

/// The quads for every mobile whose frame the atlas holds.
///
/// A mobile the atlas has no frame for is dropped: the client ships no
/// animation for that body and group, or the atlas was built before this one
/// arrived. Both are "nothing to draw", and neither is worth failing a frame
/// over — the alternative is a creature drawn from another creature's picture.
pub fn collect(mobiles: &[Mobile], camera: &Camera, atlas: &AnimAtlas) -> Vec<SpriteQuad> {
    let (eye_x, eye_y) = camera.eye_tile();
    let base = depth::base_for(eye_x, eye_y);
    let mut quads: Vec<(depth::Order, u16, SpriteQuad)> = Vec::new();

    for mobile in mobiles {
        let (direction, mirrored) = openshard_uofiles::anim::facing(mobile.facing);
        let key = FrameKey {
            body: mobile.body,
            group: mobile.group,
            direction,
            frame: mobile.frame,
        };
        let Some(packed) = atlas.frame(key) else {
            continue;
        };

        // The tile it is going to, not the pixels it is between: a body half a
        // tile into a step is *on* the tile it stepped onto as far as anything
        // it can walk behind is concerned, and an order that interpolated would
        // have a mobile change sides of a wall in the middle of a step.
        let order = depth::Order {
            tile: i32::from(mobile.at.x) + i32::from(mobile.at.y),
            priority_z: depth::mobile_priority_z(mobile.at.z),
        };
        let at = cell_centre(mobile, camera);
        let (width, height) = (i32::from(packed.sprite.width), i32::from(packed.sprite.height));
        // The anchor moves to the far edge when the picture is flipped, which
        // is `MobileView.Draw`'s `isFlipped ? width - center_x : center_x`.
        let anchor_x = if mirrored {
            width - i32::from(packed.center_x)
        } else {
            i32::from(packed.center_x)
        };
        let region = if mirrored {
            SpriteQuad::mirrored(packed.sprite.region)
        } else {
            packed.sprite.region
        };

        quads.push((
            order,
            mobile.body,
            SpriteQuad {
                x: (at.x - anchor_x) as f32,
                y: (at.y - (height + i32::from(packed.center_y))) as f32,
                width: width as f32,
                height: height as f32,
                region,
                depth: order.to_depth(base),
                hue: u32::from(mobile.hue.0),
            },
        ));
    }

    // Back to front, body breaking the tie. The depth buffer decides overlap;
    // this is for determinism, so the same world produces the same buffer.
    quads.sort_by_key(|(order, body, _)| (*order, *body));
    quads.into_iter().map(|(_, _, quad)| quad).collect()
}

#[cfg(test)]
mod tests {
    use openshard_uofiles::anim::AnimFrame;
    use openshard_uofiles::color::Color16;
    use openshard_uofiles::image::Image;

    use super::*;

    /// A frame packed at a known size, for placing.
    fn atlas(body: u16, direction: u8, width: u16, height: u16, center: (i16, i16)) -> AnimAtlas {
        AnimAtlas::pack([(
            FrameKey {
                body,
                group: 4,
                direction,
                frame: 0,
            },
            AnimFrame {
                center_x: center.0,
                center_y: center.1,
                image: Image::new(
                    width,
                    height,
                    vec![Color16(0x7C00); usize::from(width) * usize::from(height)],
                ),
            },
        )])
        .expect("one frame fits")
    }

    /// The placement arithmetic, in numbers rather than in a picture.
    ///
    /// Two ways to get this wrong look identical on a screenshot until a mobile
    /// walks: anchoring by the sprite's own corner instead of its centre, and
    /// anchoring a flipped sprite by the same edge as an unflipped one. The
    /// second is asserted below as the *difference* between the two facings,
    /// which is the only place it shows.
    #[test]
    fn a_mobile_hangs_from_its_frames_centre() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let atlas = atlas(400, 0, 40, 60, (12, -3));
        // Facing 3 is the stored direction 0, unflipped.
        let quads = collect(
            &[Mobile {
                at: Point::new(100, 100, 0),
                body: 400,
                group: 4,
                facing: Direction::SouthEast,
                frame: 0,
                hue: Hue::NONE,
                glide: None,
            }],
            &camera,
            &atlas,
        );
        assert_eq!(quads.len(), 1);
        // The camera puts its own tile's centre at (400, 300).
        assert_eq!(quads[0].x, 400.0 - 12.0);
        assert_eq!(quads[0].y, 300.0 - (60.0 - 3.0));
        assert_eq!(quads[0].width, 40.0);
    }

    /// A mirrored facing samples its picture backwards and hangs from the other
    /// edge. Both halves, because either alone draws a creature that faces the
    /// right way and stands in the wrong place — or the reverse.
    #[test]
    fn a_mirrored_facing_flips_the_picture_and_the_anchor() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        // Facings 2 and 4 share stored direction 1; 2 is the flipped one.
        let atlas = atlas(400, 1, 40, 60, (12, -3));
        let quads = |facing: Direction| {
            collect(
                &[Mobile {
                    at: Point::new(100, 100, 0),
                    body: 400,
                    group: 4,
                    facing,
                    frame: 0,
                    hue: Hue::NONE,
                    glide: None,
                }],
                &camera,
                &atlas,
            )
        };
        let plain = quads(Direction::South);
        let flipped = quads(Direction::East);
        assert_eq!(plain.len(), 1);
        assert_eq!(flipped.len(), 1);

        assert_eq!(plain[0].x, 400.0 - 12.0);
        assert_eq!(flipped[0].x, 400.0 - (40.0 - 12.0), "the anchor is the far edge");
        assert_eq!(plain[0].y, flipped[0].y, "only x is mirrored");
        assert!(flipped[0].region.du < 0.0, "the picture is sampled backwards");
        assert!(plain[0].region.du > 0.0);
    }

    /// A mobile the atlas has no frame for is dropped rather than drawn from
    /// whatever else was packed. Which matters more here than for statics: the
    /// atlas is keyed by four numbers, so a near miss is another creature.
    #[test]
    fn a_mobile_with_no_packed_frame_is_dropped() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let atlas = atlas(400, 0, 40, 60, (12, -3));
        let missing = Mobile {
            at: Point::new(100, 100, 0),
            body: 400,
            group: 4,
            facing: Direction::SouthEast,
            // One past the only frame packed.
            frame: 1,
            hue: Hue::NONE,
            glide: None,
        };
        assert!(collect(&[missing], &camera, &atlas).is_empty());
        // And the same mobile at frame 0 is not, so the drop above is a
        // decision about the frame rather than about the body.
        assert_eq!(
            collect(&[Mobile { frame: 0, ..missing }], &camera, &atlas).len(),
            1,
        );
    }

    /// A body mid-step hangs between the two tiles, and the two ends of the
    /// step are exact.
    ///
    /// The ends are the half worth pinning: a glide that ended a pixel short
    /// leaves the body permanently off its tile, and one that started a pixel
    /// late is the teleport this exists to remove, one pixel smaller.
    #[test]
    fn a_gliding_body_hangs_between_the_two_tiles() {
        let from = Point::new(100, 100, 0);
        let to = Point::new(101, 100, 0);
        let stepping = |progress: f32| {
            world_position(&Mobile {
                at: to,
                body: 400,
                group: 0,
                facing: Direction::NorthEast,
                frame: 0,
                hue: Hue::NONE,
                glide: Some(Glide { from, progress }),
            })
        };
        assert_eq!(stepping(0.0), camera::project(from), "it starts where it was");
        assert_eq!(stepping(1.0), camera::project(to), "and arrives exactly");
        // A step east is half a tile right and half a tile down; half of it is
        // half of that.
        assert_eq!(stepping(0.5), WorldPixel { x: 11, y: 4411 });
        assert_eq!(
            camera::project(from),
            WorldPixel { x: 0, y: 4400 },
            "the ends the halfway point is between",
        );
        assert_eq!(camera::project(to), WorldPixel { x: 22, y: 4422 });
        // A clock that overran is a body standing on its destination, not one
        // sliding past it.
        assert_eq!(stepping(4.0), camera::project(to));
        assert_eq!(stepping(-1.0), camera::project(from));
    }

    /// A step up a hill rises over the step, rather than snapping at the end.
    ///
    /// Free, because the projection is linear in `z` — and worth a test because
    /// a glide that interpolated only `x` and `y` would look right on the flat
    /// and jerk vertically on every stair in the game.
    #[test]
    fn a_glide_carries_the_height_with_it() {
        let flat = Point::new(100, 100, 0);
        let up = Point::new(101, 100, 10);
        let half = world_position(&Mobile {
            at: up,
            body: 400,
            group: 0,
            facing: Direction::NorthEast,
            frame: 0,
            hue: Hue::NONE,
            glide: Some(Glide {
                from: flat,
                progress: 0.5,
            }),
        });
        let (low, high) = (camera::project(flat).y, camera::project(up).y);
        assert_eq!(half.y, (low + high) / 2);
        assert!(half.y < low, "and it is on the way up");
    }

    /// The glide moves the sprite and nothing else: a body half a tile into a
    /// step still sorts by the tile it stepped onto.
    ///
    /// Which is what keeps it from changing sides of a wall halfway through a
    /// step — the depth buffer would draw it in front for two frames and behind
    /// for the next two.
    #[test]
    fn a_glide_does_not_move_the_depth_order() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let atlas = atlas(400, 0, 40, 60, (12, -3));
        let mobile = Mobile {
            at: Point::new(101, 100, 0),
            body: 400,
            group: 4,
            facing: Direction::SouthEast,
            frame: 0,
            hue: Hue::NONE,
            glide: None,
        };
        let standing = collect(&[mobile], &camera, &atlas);
        let mid_step = collect(
            &[Mobile {
                glide: Some(Glide {
                    from: Point::new(100, 100, 0),
                    progress: 0.5,
                }),
                ..mobile
            }],
            &camera,
            &atlas,
        );
        assert_eq!(standing.len(), 1);
        assert_eq!(mid_step.len(), 1);
        assert_eq!(standing[0].depth, mid_step[0].depth, "the order is the tile's");
        assert_eq!(mid_step[0].x, standing[0].x - 11.0, "and the sprite has moved");
        assert_eq!(mid_step[0].y, standing[0].y - 11.0);
    }

    /// Every distinct animation a set of mobiles needs, once each.
    #[test]
    fn the_needed_animations_are_deduplicated_by_stored_direction() {
        let mobile = |facing: Direction| Mobile {
            at: Point::new(0, 0, 0),
            body: 400,
            group: 4,
            facing,
            frame: 0,
            hue: Hue::NONE,
            glide: None,
        };
        // East and South share a picture, so they are one animation to read.
        let wanted = needed_animations(&[
            mobile(Direction::East),
            mobile(Direction::South),
            mobile(Direction::SouthEast),
        ]);
        assert_eq!(wanted, vec![(400, 4, 0), (400, 4, 1)]);
    }
}
