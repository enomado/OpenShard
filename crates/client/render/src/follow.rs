//! Where the eye goes, given where the body is.
//!
//! One pipeline, and every camera this client will have is a [`Rig`] — a value
//! of parameters rather than an implementation of its own. The argument is in
//! `docs/camera.md`; the short of it is that two cameras written as two
//! implementations are two quantisers, two cut rules and two rounding habits,
//! and a bench comparing them compares those as much as it compares the feel.
//! [`Rig::HARD`] is the reference camera — the eye is the body, to the pixel,
//! every frame — expressed as every time constant at zero, and if this pipeline
//! could not say that as a degenerate parameter set the pipeline would be wrong.
//!
//! # The ground and the height are two signals
//!
//! [`crate::camera::project`] folds `z` into the vertical axis: a tile one unit
//! higher is four pixels further up the same screen column. So a camera handed a
//! projected pixel cannot smooth height at all — filter that value and the walk
//! is filtered with it, leave it and every stair is a four-pixel jump of the
//! whole world. [`Gaze`] therefore keeps the ground position and the lift apart
//! and folds them back together once, at the end, in [`Gaze::eye`].
//!
//! # The order, including the stages that are empty
//!
//! 1. **Anchor** — what is being looked at. The caller's: it is the one that
//!    knows what a player is, and it hands over a [`Gaze`].
//! 2. **Intent** — additive offsets that say where the player wants to look
//!    rather than where they are: velocity look-ahead, cursor lean. *Empty.*
//! 3. **Zone** — the dead zone and the idle recentre. *Empty.*
//! 4. **Filter** — per-channel damping. Live, and identity under [`Rig::HARD`].
//! 5. **Cut** — the discontinuities the filter must not be dragged across, which
//!    reset its state rather than being eased over. *Empty apart from
//!    [`Follower::cut`]*, which is the one a caller raises by hand.
//! 6. **Clamp** — map edges, camera volumes. *Empty*, and it stays empty while
//!    the anchor is a body, which is on the map by construction.
//! 7. **Impulse** — shake, recoil, a hit's kick: additive, on the pose, never on
//!    the filter's state. *Empty.*
//! 8. **Quantise** — round to whole world pixels and keep the remainder in the
//!    state.
//!
//! The empty ones are listed because order is where camera defects live: a clamp
//! above the filter springs off its boundary, an impulse mixed into the filter's
//! state drifts and never comes back, and a quantiser in the middle turns a
//! filter into a ratchet — the eye sits still until the accumulated error
//! crosses a pixel and then moves one. Fixing the whole order now means a later
//! milestone fills a stage in without anything else moving.

use std::time::Duration;

use openshard_protocol::world::Point;

use crate::camera::{WorldPixel, Z_STEP, project};

/// Where the eye is asked to look, before anything smooths it.
///
/// Three channels and not a point, because they are filtered on different
/// clocks — see this module's note on the fold. Sub-pixel, because this is a
/// filter's input rather than a sprite's placement: quantising it here is the
/// staircase that makes a smoother smooth nothing at low speed.
///
/// `f64` and not `f32`, and it is the rounding that decides it rather than the
/// filter. The far corner of a 7,168-tile facet is 157,000 world pixels out,
/// where an `f32` resolves to about a hundredth of a pixel — fine for a
/// smoother, and a hundred times the margin at which two roundings of the same
/// position disagree. The eye has to land on the pixel the sprite landed on.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Gaze {
    /// Where the body stands on the ground, in world pixels read at `z = 0`.
    /// Rightwards.
    pub x: f64,
    /// Downwards.
    pub y: f64,
    /// What its height lifts it by, in pixels — `z * Z_STEP`, positive upwards,
    /// kept apart so it can have its own clock.
    pub lift: f64,
}

impl Gaze {
    /// A body standing on a tile.
    pub fn on(point: Point) -> Self {
        // Read at `z = 0` so the lift is not in here twice.
        let plane = project(Point::new(point.x, point.y, 0));
        Self {
            x: f64::from(plane.x),
            y: f64::from(plane.y),
            lift: f64::from(point.z) * f64::from(Z_STEP),
        }
    }

    /// This gaze moved back towards `from` by the part of a step not yet walked.
    ///
    /// Backwards from the destination rather than forwards from the origin,
    /// which is the form that lands *exactly* on the destination when the step
    /// is over: a lerp forwards leaves a rounding pixel behind at the end of
    /// every step, and a body that never quite arrives shimmers.
    pub fn back_towards(self, from: Self, left: f64) -> Self {
        Self {
            x: self.x - (self.x - from.x) * left,
            y: self.y - (self.y - from.y) * left,
            lift: self.lift - (self.lift - from.lift) * left,
        }
    }

    /// Where this lands in world pixel space, height folded back in, to a
    /// fraction of a pixel.
    pub fn exact(self) -> (f64, f64) {
        (self.x, self.y - self.lift)
    }

    /// The one world pixel this asks for.
    ///
    /// The quantiser: whole pixels leave, the fraction stays in whatever held
    /// this value. An eye carrying a fraction puts every sprite on a half-texel
    /// boundary for half of all camera positions, which does not show on a
    /// screenshot and boils the whole frame in motion.
    pub fn eye(self) -> WorldPixel {
        let (x, y) = self.exact();
        WorldPixel {
            x: x.round() as i32,
            y: y.round() as i32,
        }
    }
}

/// Every number the eye's motion is made of. A camera is one of these.
///
/// `Copy + PartialEq` and kept apart from the state, for two reasons that are
/// both about the bench: a preset can be swapped while the client is running
/// without the world jumping, and a slider edit is a value that can be printed,
/// pasted into the source and committed as the preset it turned out to be.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rig {
    /// Seconds for the ground plane to close all but `1/e` of its gap.
    ///
    /// Zero is no filter at all. One knob for two channels today — the filter
    /// keeps `x` and `y` apart regardless, so splitting the knob later moves no
    /// state.
    pub plane_tau: f32,
    /// The same for the height, and its own because it is not the same
    /// question: a stair wants to be smoothed away and a walk does not.
    pub lift_tau: f32,
}

impl Rig {
    /// The reference camera: the eye is the body, to the pixel, every frame.
    ///
    /// What ClassicUO does, and what this client did before there was a
    /// pipeline. It is here as the baseline every other rig is scored against,
    /// not as a default — see `docs/camera.md`, D9.
    pub const HARD: Self = Self {
        plane_tau: 0.0,
        lift_tau: 0.0,
    };
}

/// A rig, and where the eye it drives has got to.
///
/// The state is a [`Gaze`] because it has exactly the channels the target has:
/// the filter runs on each of them, and the fraction the quantiser does not
/// spend stays in it.
#[derive(Clone, Copy, Debug)]
pub struct Follower {
    rig: Rig,
    /// Where the eye is, to a fraction of a pixel.
    ///
    /// [`Option`] in its proper sense: before the first frame the eye is not at
    /// a stale position, it has no position, and the frame that finds it so
    /// places it rather than easing from a zero nobody chose.
    at: Option<Gaze>,
}

impl Follower {
    /// A follower that has not looked anywhere yet.
    pub fn new(rig: Rig) -> Self {
        Self { rig, at: None }
    }

    /// The parameters in force.
    pub fn rig(&self) -> Rig {
        self.rig
    }

    /// Change them, keeping where the eye is.
    ///
    /// Keeping it deliberately: swapping a preset mid-flight is the whole point
    /// of the bench being a picker, and a swap that also moved the eye would
    /// make every comparison start with a jump.
    pub fn set_rig(&mut self, rig: Rig) {
        self.rig = rig;
    }

    /// Where the eye is, to a fraction of a pixel — what the quantiser rounded,
    /// and `None` before the first frame.
    ///
    /// For a bench and for a scope, and the reason it is worth exposing is that
    /// the two traces answer different questions: at one-pixel quantisation and
    /// sixty frames a second, the acceleration of the *rounded* eye is
    /// dominated by the rounding and says nothing about the rig. See
    /// [`crate::bench`].
    pub fn exact(&self) -> Option<(f64, f64)> {
        self.at.map(Gaze::exact)
    }

    /// Forget where the eye was: the next [`Follower::advance`] places it on the
    /// gaze instead of easing to it.
    ///
    /// A cut, in the sense of stage 5 — a teleport, a facet change, a relock
    /// from across the map. It resets the state rather than moving it, so that
    /// nothing of the old position survives to be eased away afterwards.
    pub fn cut(&mut self) {
        self.at = None;
    }

    /// One frame: where the eye goes, given where it is asked to look.
    ///
    /// Two arguments and not a `Frame` struct, which is what `docs/camera.md`
    /// first wrote: the pipeline's input is a gaze and an elapsed time until
    /// stages 2, 3 and 5 fill, and a wrapper around two values earns nothing.
    /// What the shape does say is what is *not* an argument — no `Instant`, no
    /// `Camera`, no window, no map. That is what lets the bench run ten thousand
    /// frames in under a millisecond and the DST harness drive the same code the
    /// window does.
    pub fn advance(&mut self, gaze: Gaze, dt: Duration) -> WorldPixel {
        // Stages 2 and 3 sit here when they exist: the intent offsets are
        // summed onto `gaze`, and the zone reduces the gap before the filter
        // sees it.
        let at = match self.at {
            // Stage 4, per channel. `x` and `y` share a time constant and not a
            // state: the isometric screen is twice as wide as it is tall, so the
            // day one of them wants a slower clock than the other, the split is
            // a field and not a rewrite.
            Some(was) => Gaze {
                x: approach(was.x, gaze.x, self.rig.plane_tau, dt),
                y: approach(was.y, gaze.y, self.rig.plane_tau, dt),
                lift: approach(was.lift, gaze.lift, self.rig.lift_tau, dt),
            },
            // Stage 5, in its one live form: nothing to ease from.
            None => gaze,
        };
        self.at = Some(at);
        // Stages 6 and 7 sit here — the clamp on the filtered position, then the
        // impulse on the pose and never on what was just stored.
        at.eye()
    }
}

/// One channel, a step closer to where it is asked to be.
///
/// `alpha = 1 - exp(-dt / tau)`, which is frame-rate independent by
/// construction: the same span of time moves the eye the same distance whether
/// it arrives in one frame or ten. Never `from + (to - from) * 0.1` per frame —
/// that ties the feel to the frame rate, and this client already has two of them
/// on purpose, so the naive form would change the camera's character at the
/// exact moment somebody starts walking.
///
/// The `tau <= 0` branch is not an optimisation and not defensiveness. `dt / 0`
/// is infinity, `0 / 0` is `NaN`, and a `NaN` here does not fall over: it is
/// stored, every comparison against it downstream is false, and the camera
/// silently takes the other branch of every decision for the rest of the
/// session. The reference rig is exactly this branch.
fn approach(from: f64, to: f64, tau: f32, dt: Duration) -> f64 {
    if tau <= 0.0 {
        return to;
    }
    let alpha = 1.0 - (-dt.as_secs_f64() / f64::from(tau)).exp();
    from + (to - from) * alpha
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tile far enough out that `f32` would have started rounding.
    const FAR: Point = Point::new(6000, 4000, 0);

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    /// The reference camera, and the property the whole of C0 is a transplant
    /// under: whatever the frame rate and wherever the body is, the eye is on it.
    #[test]
    fn the_reference_rig_is_the_body() {
        let mut follower = Follower::new(Rig::HARD);
        for (step, dt) in [(0, 0), (1, 16), (2, 33), (3, 0), (4, 400)] {
            let gaze = Gaze::on(Point::new(FAR.x + step, FAR.y, 12));
            assert_eq!(follower.advance(gaze, ms(dt)), gaze.eye());
        }
    }

    /// A zero time constant with a zero frame is the one division that would be
    /// `0 / 0`. A `NaN` here is not caught by anything downstream — it is
    /// stored, and every comparison against it is quietly false ever after.
    #[test]
    fn a_still_frame_at_no_time_constant_is_not_a_nan() {
        let mut follower = Follower::new(Rig::HARD);
        follower.advance(Gaze::on(FAR), Duration::ZERO);
        let state = follower.at.unwrap();
        assert!(state.x.is_finite() && state.y.is_finite() && state.lift.is_finite());
    }

    /// The definition of the time constant, held to its own arithmetic: one
    /// `tau` of elapsed time closes all but `1/e` of the gap.
    #[test]
    fn one_time_constant_closes_all_but_one_over_e() {
        let mut follower = Follower::new(Rig {
            plane_tau: 0.5,
            lift_tau: 0.5,
        });
        follower.advance(Gaze::default(), ms(16));
        let target = Gaze {
            x: 1000.0,
            y: 0.0,
            lift: 0.0,
        };
        follower.advance(target, ms(500));
        let left = 1000.0 - follower.at.unwrap().x;
        assert!(
            (left - 1000.0 / std::f64::consts::E).abs() < 0.5,
            "{left} left of 1000 after one tau",
        );
    }

    /// D5, and the reason the naive per-frame lerp is banned: the same span of
    /// time has to move the eye the same distance however many frames it
    /// arrives in. A `lerp(0.1)` fails this by a factor of three.
    #[test]
    fn the_same_span_lands_in_the_same_place_at_any_frame_rate() {
        let rig = Rig {
            plane_tau: 0.2,
            lift_tau: 0.2,
        };
        let target = Gaze {
            x: 900.0,
            y: -400.0,
            lift: 60.0,
        };
        let mut slow = Follower::new(rig);
        slow.advance(Gaze::default(), ms(1));
        slow.advance(target, ms(100));

        let mut fast = Follower::new(rig);
        fast.advance(Gaze::default(), ms(1));
        for _ in 0..10 {
            fast.advance(target, ms(10));
        }

        let (slow, fast) = (slow.at.unwrap(), fast.at.unwrap());
        assert!(
            (slow.x - fast.x).abs() < 1e-9 && (slow.lift - fast.lift).abs() < 1e-9,
            "{slow:?} in one frame against {fast:?} in ten",
        );
    }

    /// The height has its own clock, which is the whole reason [`Gaze`] is three
    /// numbers: a rig can hold the ground exact and still smooth a stair away.
    #[test]
    fn the_height_is_filtered_apart_from_the_ground() {
        let mut follower = Follower::new(Rig {
            plane_tau: 0.0,
            lift_tau: 0.4,
        });
        let ground = Gaze::on(Point::new(500, 500, 0));
        follower.advance(ground, ms(16));
        // The same tile, twenty units up: only the lift moved.
        let stair = Gaze::on(Point::new(500, 500, 20));
        follower.advance(stair, ms(16));
        let at = follower.at.unwrap();
        assert_eq!((at.x, at.y), (ground.x, ground.y), "the ground did not wait");
        assert!(at.lift > 0.0 && at.lift < stair.lift, "{} of 80", at.lift);
    }

    /// A cut leaves no tail: what the eye was doing before one is not eased
    /// away afterwards, it is gone.
    #[test]
    fn a_cut_places_the_eye_rather_than_easing_to_it() {
        let mut follower = Follower::new(Rig {
            plane_tau: 0.3,
            lift_tau: 0.3,
        });
        follower.advance(Gaze::on(Point::new(100, 100, 0)), ms(16));
        let away = Gaze::on(Point::new(3000, 3000, 0));
        let eased = follower.advance(away, ms(16));
        assert_ne!(eased, away.eye(), "an ease is what it does without a cut");
        follower.cut();
        assert_eq!(follower.advance(away, ms(16)), away.eye());
    }

    /// Swapping a preset must not move the eye, or every comparison the bench
    /// is for would start with a jump.
    #[test]
    fn changing_the_rig_keeps_where_the_eye_is() {
        let mut follower = Follower::new(Rig {
            plane_tau: 0.3,
            lift_tau: 0.3,
        });
        follower.advance(Gaze::on(Point::new(100, 100, 0)), ms(16));
        follower.advance(Gaze::on(Point::new(140, 100, 0)), ms(16));
        let mid = follower.at.unwrap();
        follower.set_rig(Rig::HARD);
        assert_eq!(follower.at.unwrap(), mid);
        assert_eq!(follower.rig(), Rig::HARD);
    }

    /// `Gaze::on` is `project`, taken apart and put back together — held against
    /// `project` itself, which is pinned by its own tests and written from the
    /// client's numbers rather than from this file's.
    #[test]
    fn a_gaze_folds_back_into_its_projection() {
        for z in [i8::MIN, -40, -1, 0, 1, 44, i8::MAX] {
            for (x, y) in [(0, 0), (1, 0), (0, 1), (1495, 1629), (6143, 4095)] {
                let point = Point::new(x, y, z);
                assert_eq!(Gaze::on(point).eye(), project(point), "{point}");
            }
        }
    }

    /// And the step between two tiles ends exactly on the destination, at every
    /// height either side of it — the property `back_towards` is written the way
    /// it is for.
    #[test]
    fn a_step_ends_exactly_on_the_tile_it_was_going_to() {
        let from = Gaze::on(Point::new(1000, 1000, 0));
        let to = Point::new(1001, 1000, 20);
        assert_eq!(Gaze::on(to).back_towards(from, 0.0).eye(), project(to));
        assert_eq!(
            Gaze::on(to).back_towards(from, 1.0).eye(),
            project(Point::new(1000, 1000, 0)),
        );
    }
}
