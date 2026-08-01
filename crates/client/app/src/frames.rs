//! The last few seconds of the event loop: how often a frame was drawn, and how
//! long each took to build.
//!
//! # Why the two are separate numbers
//!
//! "The frame rate dropped" is two different complaints and they have opposite
//! fixes. One is *cost*: the frame took longer to build than the display gave it,
//! and something in `draw` is too slow. The other is *pacing*: nothing asked for
//! a frame, so none was drawn — which is what this client does on purpose the
//! moment nobody is walking, since [`App::redraw_interval`](crate::App) drops the
//! loop to the animation clock's 80ms and standing still costs 12.5 frames a
//! second by design.
//!
//! Told apart by looking at both at once: a build time that climbed is the first,
//! an interval that jumped while the build time stayed flat is the second. With
//! only one of them on screen, every drop looks like the same drop.
//!
//! # Why not [`bench::Scope`](openshard_client_render::bench::Scope)
//!
//! The scope beside it holds what the *camera* did, is fed only while the eye is
//! the body's, and is cleared whenever a rig is swapped — all of which is right
//! for a metric about a rig and wrong for one about the loop. A frame drawn with
//! the camera unlocked is still a frame.

use std::time::Duration;

/// One frame: when it landed, how long since the last one, and what it cost.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// When it landed, on this ring's own clock.
    pub at: Duration,
    /// The gap since the frame before it — the *interval*, which is what a frame
    /// rate is the reciprocal of. Never zero: the ring will not record one, so
    /// [`Frame::fps`] can divide.
    pub interval: Duration,
    /// How long `draw` spent building and submitting it.
    pub build: Duration,
}

impl Frame {
    /// Frames a second, if every frame were this one.
    ///
    /// An instantaneous rate rather than an average over a window, and
    /// deliberately: the thing worth seeing here is the one frame that took
    /// 80ms, and a mean over a second hides exactly that.
    pub fn fps(self) -> f64 {
        1.0 / self.interval.as_secs_f64()
    }
}

/// The last [`Frames::span`] of frames, and nothing older.
///
/// Its own clock, advanced by the interval each frame reported, for the reason
/// `bench::Scope` has one: a structure that reads [`std::time::Instant`] cannot
/// be handed a cadence by a test.
#[derive(Clone, Debug)]
pub struct Frames {
    span: Duration,
    at: Duration,
    frames: Vec<Frame>,
}

impl Frames {
    /// A ring holding `span` of frames.
    pub fn new(span: Duration) -> Self {
        Self {
            span,
            at: Duration::ZERO,
            frames: Vec::new(),
        }
    }

    /// One frame, `interval` after the last, which took `build` to draw.
    ///
    /// A zero interval is dropped rather than recorded. Two frames at the same
    /// instant is not a rate — it is a redraw requested twice for one wake —
    /// and it is the one value the reciprocal cannot be taken of.
    pub fn record(&mut self, interval: Duration, build: Duration) {
        if interval.is_zero() {
            return;
        }
        self.at += interval;
        self.frames.push(Frame {
            at: self.at,
            interval,
            build,
        });
        let cutoff = self.at.saturating_sub(self.span);
        let keep = self
            .frames
            .iter()
            .position(|frame| frame.at >= cutoff)
            .unwrap_or(self.frames.len());
        self.frames.drain(..keep);
    }

    /// Every frame still held, oldest first.
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    /// How long a window this keeps.
    pub fn span(&self) -> Duration {
        self.span
    }

    /// The worst interval in the window, as a frame rate — the number a player
    /// means by "it dropped".
    ///
    /// `None` when there are no frames yet, and absent rather than zero: no
    /// frames is not a rate of nothing, it is not a rate.
    pub fn worst_fps(&self) -> Option<f64> {
        self.frames
            .iter()
            .map(|frame| frame.interval)
            .max()
            .map(|interval| 1.0 / interval.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window is a window: what fell out of the back is gone, and what is
    /// held still covers the span.
    #[test]
    fn a_ring_keeps_its_span_and_drops_what_fell_out_of_it() {
        let mut frames = Frames::new(Duration::from_millis(500));
        for _ in 0..100 {
            frames.record(Duration::from_millis(16), Duration::from_millis(2));
        }
        let held = frames.frames();
        assert_eq!(held.last().unwrap().at, Duration::from_millis(1_600));
        assert!(held.len() < 100, "{} frames, and nothing dropped", held.len());
        let span = held.last().unwrap().at - held.first().unwrap().at;
        assert!(span <= frames.span(), "{span:?} of a 500ms window");
        assert!(span > Duration::from_millis(450), "{span:?}, and not a stub");
    }

    /// The whole point of the panel: a frame that arrived late is a low rate,
    /// whatever it cost to build.
    #[test]
    fn the_rate_is_the_interval_and_not_the_cost() {
        let mut frames = Frames::new(Duration::from_secs(4));
        frames.record(Duration::from_millis(16), Duration::from_millis(1));
        frames.record(Duration::from_millis(80), Duration::from_millis(1));
        let held = frames.frames();
        assert!((held[0].fps() - 62.5).abs() < 0.01, "{}", held[0].fps());
        assert!((held[1].fps() - 12.5).abs() < 0.01, "{}", held[1].fps());
        // The standing cadence, which is the answer to "why did it drop when I
        // stopped walking" — and it is not the build time, which never moved.
        assert!((frames.worst_fps().unwrap() - 12.5).abs() < 0.01);
    }

    /// Two redraws for one wake is not a rate of infinity.
    #[test]
    fn a_frame_at_no_interval_at_all_is_not_recorded() {
        let mut frames = Frames::new(Duration::from_secs(4));
        frames.record(Duration::ZERO, Duration::from_millis(1));
        assert!(frames.frames().is_empty());
        assert_eq!(frames.worst_fps(), None);
    }
}
