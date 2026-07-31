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

use openshard_protocol::direction::Direction;
use openshard_protocol::wire::Hue;
use openshard_protocol::world::Point;

use crate::atlas::{AnimAtlas, FrameKey};
use crate::camera::Camera;
use crate::depth;
use crate::sprite::SpriteQuad;

/// One creature to draw, as the client knows it.
///
/// Everything here comes from the server — `0x77`, `0x78` and `0x20` — except
/// the frame, which is whatever the animation has advanced to. Deliberately a
/// plain value rather than a handle into a `WorldView`: this crate renders what
/// it is given and owns no model of the world.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mobile {
    /// Where it stands.
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

        let order = depth::Order {
            tile: i32::from(mobile.at.x) + i32::from(mobile.at.y),
            priority_z: depth::mobile_priority_z(mobile.at.z),
        };
        let at = camera.to_screen(mobile.at);
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
        };
        assert!(collect(&[missing], &camera, &atlas).is_empty());
        // And the same mobile at frame 0 is not, so the drop above is a
        // decision about the frame rather than about the body.
        assert_eq!(
            collect(&[Mobile { frame: 0, ..missing }], &camera, &atlas).len(),
            1,
        );
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
