//! Advancing a mobile's animation frame.
//!
//! Deliberately not the world tick. A body's frame is a real-time clock in the
//! client — `Mobile.ProcessAnimation` re-picks it every
//! `Constants.CHARACTER_ANIMATION_DELAY` (80ms) of wall time, unscaled unless a
//! server explicitly set the animation (`0x6E`/`0xE2`, which carry their own
//! interval byte) — where the world's tick is deliberately not: nothing here
//! feeds gameplay, so there is nothing that has to replay from a seed.
//!
//! `animdata.mul` is a different file for a different thing: it times an
//! *animated static* (a torch, a fire — `AnimatedStaticsManager`) and a spell
//! *effect* graphic (`GameEffect`), each cycling through a short run of
//! consecutive graphic ids. Neither is a mobile's body, which is why this clock
//! does not read it.

use std::time::Duration;

use openshard_protocol::feedback::AnimationFrameCount;

/// How long one frame of a mobile's body animation is shown.
///
/// `Constants.CHARACTER_ANIMATION_DELAY` in the client. Fixed: nothing here
/// scales it, because scaling only happens for an animation the server set
/// explicitly, and this crate draws a `Mobile` it was handed rather than
/// modelling why it is playing.
pub const FRAME_DELAY: Duration = Duration::from_millis(80);

/// Which frame of a playing animation is on screen, advanced by real time.
///
/// Holds elapsed time rather than a frame index: the frame is a function of
/// time and the animation's own length, and a length that changes — a shorter
/// packed group, a body swap — should not leave the clock pointing past the
/// end of the new one.
#[derive(Clone, Copy, Debug, Default)]
pub struct AnimationClock {
    elapsed: Duration,
}

impl AnimationClock {
    /// Move the clock forward.
    pub fn advance(&mut self, dt: Duration) {
        self.elapsed += dt;
    }

    /// Which frame to show, out of `frame_count`.
    ///
    /// `0` when `frame_count` is `0`, so a caller with no packed frames gets a
    /// number rather than a division by nothing.
    pub fn frame(&self, frame_count: AnimationFrameCount) -> u16 {
        if frame_count.0 == 0 {
            return 0;
        }
        let ticks = self.elapsed.as_millis() / FRAME_DELAY.as_millis();
        (ticks % u128::from(frame_count.0)) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_clock_shows_the_first_frame() {
        let clock = AnimationClock::default();
        assert_eq!(clock.frame(AnimationFrameCount(6)), 0);
    }

    #[test]
    fn the_frame_advances_one_step_per_delay() {
        let mut clock = AnimationClock::default();
        clock.advance(FRAME_DELAY);
        assert_eq!(clock.frame(AnimationFrameCount(6)), 1);
        clock.advance(FRAME_DELAY);
        assert_eq!(clock.frame(AnimationFrameCount(6)), 2);
    }

    /// Sub-delay time is not enough to move the frame — the clock steps, it
    /// does not interpolate.
    #[test]
    fn less_than_a_delay_does_not_advance_the_frame() {
        let mut clock = AnimationClock::default();
        clock.advance(FRAME_DELAY / 2);
        assert_eq!(clock.frame(AnimationFrameCount(6)), 0);
    }

    /// A six-frame walk loops back to its first frame rather than running off
    /// the end of what the atlas packed.
    #[test]
    fn the_frame_loops_over_the_animations_own_length() {
        let mut clock = AnimationClock::default();
        clock.advance(FRAME_DELAY * 6);
        assert_eq!(clock.frame(AnimationFrameCount(6)), 0);
        clock.advance(FRAME_DELAY);
        assert_eq!(clock.frame(AnimationFrameCount(6)), 1);
    }

    /// An animation with nothing packed answers `0` rather than dividing by a
    /// frame count of zero.
    #[test]
    fn no_frames_packed_is_frame_zero() {
        let mut clock = AnimationClock::default();
        clock.advance(FRAME_DELAY * 3);
        assert_eq!(clock.frame(AnimationFrameCount(0)), 0);
    }
}
