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
//! last packet put it teleports 44 pixels every step. [`Mobile::drawn`] is the
//! difference: the sub-pixel position the sprite actually hangs at, which
//! mid-step is between two tiles and under an eased rig is behind the tile
//! altogether. Nothing else changes — the *tile* is still where the server put
//! it, which is what everything that is not a pixel keeps asking.
//!
//! **What that position is a function of is deliberately not here.** A step's
//! progress is a clock, an ease is a filter with state, and both belong to the
//! layer that ages what it sees — `client/app`'s `crowd.rs`. This crate reads no
//! clocks (see the crate docs), so a mobile arrives with its position already
//! decided and the renderer places a picture at it. `docs/camera.md` D10 is the
//! argument for the ease living on the body rather than on the eye.

use openshard_protocol::direction::Direction;
use openshard_protocol::wire::Hue;
use openshard_protocol::world::Point;

use crate::atlas::{AnimAtlas, FrameKey};
use crate::camera::{Camera, ViewPixel, WorldPoint};
use crate::depth;
use crate::follow::Gaze;
use crate::sprite::SpriteQuad;

/// One creature to draw, as the client knows it.
///
/// Everything here comes from the server — `0x77`, `0x78` and `0x20` — except
/// the frame, which is whatever the animation has advanced to, and
/// [`Mobile::drawn`], which is *history*: only the previous packet says a body
/// moved. Deliberately a plain value rather than a handle into a `WorldView`:
/// this crate renders what it is given and owns no model of the world.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Mobile {
    /// The tile the server put it on — or, mid-step, the one it is going to.
    ///
    /// Everything that is not a pixel reads this and not [`Mobile::drawn`]: the
    /// depth order, so a body half a tile into a step is already *on* the tile it
    /// stepped onto as far as anything it can walk behind is concerned; and the
    /// height a mounted or gliding body is sorted at.
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
    /// Where to actually draw it, sub-pixel and with its height kept apart.
    ///
    /// **Not derived from [`Mobile::at`], and that is the point.** A body
    /// between two tiles is part way through a step; a body being eased is
    /// behind its tile by whatever the filter is holding (`docs/camera.md` D10).
    /// Neither is a function of the tile and both are a function of a clock, so
    /// the caller that owns the clock owns this — `client/app`'s `crowd.rs` — and
    /// this crate draws where it is told.
    ///
    /// A [`Gaze`] and not a [`WorldPixel`] because the camera filters it: the
    /// height is kept out of the vertical axis so it can have its own clock, and
    /// the rounding happens once, at the end (D2, D7).
    pub drawn: Gaze,
}

/// Where a mobile is asking to be looked at.
///
/// One line, and it stays a function rather than a field access because the
/// camera and the sprite must read the *same* number: a body and an eye that
/// round separately disagree by a pixel, and the disagreement is a shimmer
/// nobody can name.
pub fn gaze(mobile: &Mobile) -> Gaze {
    mobile.drawn
}

/// Where a mobile's sprite lands in the world's own pixel space.
///
/// World pixels and not view pixels, and public for it: the camera that follows
/// a body has to follow it *between* tiles too, or the world jumps a tile at a
/// time under a character sliding smoothly across it — and an eye is a
/// [`WorldPoint`], with no camera to convert with.
pub fn world_position(mobile: &Mobile) -> WorldPoint {
    gaze(mobile).eye()
}

/// The same, where the camera puts it in the drawn image, to a fraction.
///
/// **Snapped to the same lattice the eye is on**, which is the half of
/// `docs/camera.md` D11 that is not about the camera. Two things decide where a
/// sprite lands on the display — where the body is and where the eye is — and
/// if only one of them is on the real pixel's grid, the difference is not: the
/// sprite is resampled by a fraction of a texel against a world that is not, so
/// its texels change width as it walks. Under `Rig::HARD` the two are the same
/// number and this is a no-op; under any easing rig, and under D10's eased
/// *body*, they are not.
///
/// Fractional and not a [`ViewPixel`], because a third of a virtual pixel is a
/// whole real one at `3x`: rounding here would put back exactly the quantum the
/// snap above was chosen to keep.
fn cell_centre(mobile: &Mobile, camera: &Camera) -> (f32, f32) {
    camera.to_view_exact(camera.snap(world_position(mobile)))
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

/// Where a mobile's sprite lands, before hue and hold decide anything about
/// how it is drawn.
///
/// Shared by [`collect`] and [`head_anchor`]: both need the same rectangle —
/// one to draw it, the other to hang a label above it — and computing it twice
/// is two chances for the anchor arithmetic to disagree with itself.
struct Placement {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    region: crate::atlas::Region,
    order: depth::Order,
}

/// The mobile's frame in the atlas, placed on screen — or `None` if the atlas
/// holds no such frame.
fn place(mobile: &Mobile, camera: &Camera, atlas: &AnimAtlas) -> Option<Placement> {
    let (direction, mirrored) = openshard_uofiles::anim::facing(mobile.facing);
    let key = FrameKey {
        body: mobile.body,
        group: mobile.group,
        direction,
        frame: mobile.frame,
    };
    let packed = atlas.frame(key)?;

    // The tile it is going to, not the pixels it is between: a body half a
    // tile into a step is *on* the tile it stepped onto as far as anything it
    // can walk behind is concerned, and an order that interpolated would have
    // a mobile change sides of a wall in the middle of a step.
    let order = depth::Order {
        tile: i32::from(mobile.at.x) + i32::from(mobile.at.y),
        priority_z: depth::mobile_priority_z(mobile.at.z),
    };
    let at = cell_centre(mobile, camera);
    let (width, height) = (i32::from(packed.sprite.width), i32::from(packed.sprite.height));
    // The anchor moves to the far edge when the picture is flipped, which is
    // `MobileView.Draw`'s `isFlipped ? width - center_x : center_x`.
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

    Some(Placement {
        x: at.0 - anchor_x as f32,
        y: at.1 - (height + i32::from(packed.center_y)) as f32,
        width: width as f32,
        height: height as f32,
        region,
        order,
    })
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
        let Some(placement) = place(mobile, camera, atlas) else {
            continue;
        };
        quads.push((
            placement.order,
            mobile.body,
            SpriteQuad {
                x: placement.x,
                y: placement.y,
                width: placement.width,
                height: placement.height,
                region: placement.region,
                depth: placement.order.to_depth(base),
                hue: u32::from(mobile.hue.0),
            },
        ));
    }

    // Back to front, body breaking the tie. The depth buffer decides overlap;
    // this is for determinism, so the same world produces the same buffer.
    quads.sort_by_key(|(order, body, _)| (*order, *body));
    quads.into_iter().map(|(_, _, quad)| quad).collect()
}

/// Where a label belongs above this mobile's head, in view pixels — the
/// horizontal centre of its sprite and its topmost edge, or `None` if
/// [`collect`] would draw nothing for it either.
///
/// Not a fixed offset above [`cell_centre`]: a dragon and a rat stand on the
/// same tile centre and hold their heads at wildly different heights, and only
/// the packed frame's own rectangle knows which.
pub fn head_anchor(mobile: &Mobile, camera: &Camera, atlas: &AnimAtlas) -> Option<ViewPixel> {
    let placement = place(mobile, camera, atlas)?;
    Some(ViewPixel {
        x: (placement.x + placement.width / 2.0).round() as i32,
        y: placement.y.round() as i32,
    })
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
                drawn: Gaze::on(Point::new(100, 100, 0)),
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
                    drawn: Gaze::on(Point::new(100, 100, 0)),
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
            drawn: Gaze::on(Point::new(100, 100, 0)),
        };
        assert!(collect(&[missing], &camera, &atlas).is_empty());
        // And the same mobile at frame 0 is not, so the drop above is a
        // decision about the frame rather than about the body.
        assert_eq!(
            collect(&[Mobile { frame: 0, ..missing }], &camera, &atlas).len(),
            1,
        );
    }

    /// The label anchor is the top-centre of exactly the rectangle `collect`
    /// draws — computed once and read two ways, not two formulas that happen
    /// to agree today.
    #[test]
    fn head_anchor_is_the_drawn_quads_top_centre() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let atlas = atlas(400, 0, 40, 60, (12, -3));
        let mobile = Mobile {
            at: Point::new(100, 100, 0),
            body: 400,
            group: 4,
            facing: Direction::SouthEast,
            frame: 0,
            hue: Hue::NONE,
            drawn: Gaze::on(Point::new(100, 100, 0)),
        };
        let quads = collect(&[mobile], &camera, &atlas);
        let anchor = head_anchor(&mobile, &camera, &atlas).expect("packed");
        assert_eq!(anchor.x as f32, quads[0].x + quads[0].width / 2.0);
        assert_eq!(anchor.y as f32, quads[0].y);
    }

    /// A mobile the atlas has no frame for gets no anchor either — the same
    /// case `collect` drops, asked the other way.
    #[test]
    fn head_anchor_is_none_for_an_unpacked_frame() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let atlas = atlas(400, 0, 40, 60, (12, -3));
        let missing = Mobile {
            at: Point::new(100, 100, 0),
            body: 400,
            group: 4,
            facing: Direction::SouthEast,
            frame: 1,
            hue: Hue::NONE,
            drawn: Gaze::on(Point::new(100, 100, 0)),
        };
        assert!(head_anchor(&missing, &camera, &atlas).is_none());
    }

    /// A body drawn between two tiles still sorts by the tile it is on.
    ///
    /// Which is what keeps it from changing sides of a wall halfway through a
    /// step — the depth buffer would draw it in front for two frames and behind
    /// for the next two. The same rule covers an eased body, which is drawn
    /// *behind* its tile by however much the filter is holding (D10): the tile
    /// is what the server named and the drawn position is a picture.
    #[test]
    fn where_a_body_is_drawn_does_not_move_its_depth_order() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let atlas = atlas(400, 0, 40, 60, (12, -3));
        let on_its_tile = Mobile {
            at: Point::new(101, 100, 0),
            body: 400,
            group: 4,
            facing: Direction::SouthEast,
            frame: 0,
            hue: Hue::NONE,
            drawn: Gaze::on(Point::new(101, 100, 0)),
        };
        let standing = collect(&[on_its_tile], &camera, &atlas);
        // Half a tile back the way it came: a step east is eleven pixels right
        // and eleven down, so half of one is eleven of each.
        let mid_step = collect(
            &[Mobile {
                drawn: Gaze::on(Point::new(101, 100, 0)).back_towards(Gaze::on(Point::new(100, 100, 0)), 0.5),
                ..on_its_tile
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
            drawn: Gaze::on(Point::new(0, 0, 0)),
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
