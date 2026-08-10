//! The last few seconds of the event loop: how often a frame was drawn, and
//! where the time in each one went.
//!
//! # Why the pacing and the cost are separate numbers
//!
//! "The frame rate dropped" is two different complaints and they have opposite
//! fixes. One is *cost*: the frame took longer to build than the display gave it,
//! and something in `draw` is too slow. The other is *pacing*: nothing asked for
//! a frame, so none was drawn — which is what this client does the moment the
//! window stops being watched, when [`App::pacing`](crate::App) hands the loop
//! back to the animation clock's 80ms.
//!
//! Told apart by looking at both at once: a build time that climbed is the first,
//! an interval that jumped while the build time stayed flat is the second. With
//! only one of them on screen, every drop looks like the same drop.
//!
//! # Why the cost is four numbers and not one
//!
//! A frame is drawn by two independent things, waits on a third, and is drawn
//! *again* by a fourth this thread cannot time at all. A single "build time"
//! hides which one moved.
//!
//! - [`Frame::ui`] is `egui`: a layout of every open panel and the mesh that
//!   comes out of it. It is charged for the panels, so it grows when a window is
//!   opened and not when the camera moves — and a HUD that costs more than the
//!   world it is describing is a real outcome worth being able to see.
//! - [`Frame::scene`] is the world: growing the atlases, walking the map for
//!   quads, and the four passes. It grows with what is on screen.
//! - [`Frame::wait`] is the stall inside `get_current_texture`. Under
//!   `PresentMode::Fifo` it makes up most of an *idle* frame, and counted as
//!   build time it would report a client that does nothing as a client at 100%
//!   load. Read the caveat on the field before calling it slack: it is the one
//!   number here that means two different things.
//! - [`Frame::gpu`] is what the device spent on the commands the other three
//!   produced. It is not measured by a clock on this thread and could not be:
//!   `queue.submit` returns without waiting, so `scene` above stops when the
//!   *encoding* does. See [`crate::profile`], which reads it back off the device
//!   a frame or two late, and `None` when the adapter cannot time itself.
//!
//! The pair that matters when a rate is low and nothing looks busy is `wait` and
//! `gpu`. A large `wait` with a small `gpu` is a client asleep on vsync with room
//! to spare. A large `wait` with a `gpu` near the interval is a client blocked on
//! its own last frame — the same reading, the opposite diagnosis, and before
//! `gpu` existed the panel could not tell them apart.
//!
//! There is one *rate*, though, and there is meant to be: the UI and the world
//! go through one encoder into one surface texture, so both are on screen the
//! same number of times a second by construction. Splitting the two costs
//! answers "who ate the frame"; splitting the rate would be a different client.
//!
//! # Why not [`bench::Scope`](openshard_client_render::bench::Scope)
//!
//! The scope beside it holds what the *camera* did, is fed only while the eye is
//! the body's, and is cleared whenever a rig is swapped — all of which is right
//! for a metric about a rig and wrong for one about the loop. A frame drawn with
//! the camera unlocked is still a frame.

use std::time::Duration;

/// What is deciding when the next frame is drawn.
///
/// The other half of any answer about a frame rate: 12.5 frames a second is a
/// slow client under [`Pacing::Display`] and a correct one under
/// [`Pacing::Timer`], and the reading is the same number either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pacing {
    /// The display. A frame is asked for as soon as the last is queued, and
    /// `PresentMode::Fifo` blocks until the display has taken it — so the rate
    /// is the refresh rate, and what shows up in [`Frame::wait`] is the slack.
    Display,
    /// The animation clock, because nobody is watching the window. The interval
    /// is what the loop is sleeping for between frames.
    Timer(Duration),
}

/// One frame: when it landed, how long since the last one, and where its time
/// went. See the module docs for why the cost is four numbers.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// When it landed, on this ring's own clock.
    pub at: Duration,
    /// The gap since the frame before it — the *interval*, which is what a frame
    /// rate is the reciprocal of. Never zero: the ring will not record one, so
    /// [`Frame::fps`] can divide.
    pub interval: Duration,
    /// What `egui` cost: laying the panels out, and turning them into a mesh.
    pub ui: Duration,
    /// What the world cost: atlases, quads, and the passes that draw them.
    pub scene: Duration,
    /// Time blocked acquiring the surface texture.
    ///
    /// **Two things wear this number.** Under `PresentMode::Fifo` the acquire
    /// blocks until the display has taken the frame before it — the pacer
    /// working, and not work this client did. It also blocks when the swapchain
    /// has no image free because the *GPU* is still drawing into the last one,
    /// which is not slack at all: it is the device being the bottleneck,
    /// arriving one frame late and wearing the pacer's clothes.
    ///
    /// Nothing about the number distinguishes them. [`Frame::gpu`] is what does.
    pub wait: Duration,
    /// What the device spent on this client's commands, if it can say.
    ///
    /// Not a clock on this thread: `queue.submit` hands the driver a command
    /// buffer and returns, so [`Frame::scene`] stops when the encoding does and
    /// every pass is still ahead. This is read back out of timestamp queries a
    /// frame or two later — see [`crate::profile`] for why the lag is the right
    /// trade — and it is the number that says whether a large [`Frame::wait`] is
    /// slack or a stall.
    ///
    /// `None` when the adapter has no timestamp queries. Absent and not zero: a
    /// GPU whose cost is unknown is not a GPU that cost nothing, and the whole
    /// point of the field is to be believed.
    pub gpu: Option<Duration>,
    /// Whether this frame paid for a full atlas repack — the synchronous
    /// eviction `AtlasError::Full` triggers, rebuilding every pass from
    /// scratch. Its cost lands inside [`Frame::scene`] like any other world
    /// work, and without this flag a repack and a merely heavy screen are the
    /// same number: this is the counter `docs/camera.md` asks for, so the
    /// panel can name the stall instead of just showing it.
    pub repacked: bool,
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

    /// What this client spent building the frame — the UI and the world, and not
    /// the wait, which is the display's.
    ///
    /// This is the number to compare against the interval: build under interval
    /// is a client keeping up, whatever the wait was.
    pub fn build(self) -> Duration {
        self.ui + self.scene
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

    /// One frame, `interval` after the last, and where its time went.
    ///
    /// A zero interval is dropped rather than recorded. Two frames at the same
    /// instant is not a rate — it is a redraw requested twice for one wake —
    /// and it is the one value the reciprocal cannot be taken of.
    pub fn record(
        &mut self,
        interval: Duration,
        ui: Duration,
        scene: Duration,
        wait: Duration,
        gpu: Option<Duration>,
        repacked: bool,
    ) {
        if interval.is_zero() {
            return;
        }
        self.at += interval;
        self.frames.push(Frame {
            at: self.at,
            interval,
            ui,
            scene,
            wait,
            gpu,
            repacked,
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

    /// A frame costing `ui` in the panels and `scene` in the world, with no wait.
    fn frame(interval: u64, ui: u64, scene: u64) -> (Duration, Duration, Duration, Duration) {
        (
            Duration::from_millis(interval),
            Duration::from_millis(ui),
            Duration::from_millis(scene),
            Duration::ZERO,
        )
    }

    /// The window is a window: what fell out of the back is gone, and what is
    /// held still covers the span.
    #[test]
    fn a_ring_keeps_its_span_and_drops_what_fell_out_of_it() {
        let mut frames = Frames::new(Duration::from_millis(500));
        for _ in 0..100 {
            let (interval, ui, scene, wait) = frame(16, 1, 1);
            frames.record(interval, ui, scene, wait, None, false);
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
        let (interval, ui, scene, wait) = frame(16, 0, 1);
        frames.record(interval, ui, scene, wait, None, false);
        let (interval, ui, scene, wait) = frame(80, 0, 1);
        frames.record(interval, ui, scene, wait, None, false);
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
        let (interval, ui, scene, wait) = frame(0, 1, 1);
        frames.record(interval, ui, scene, wait, None, false);
        assert!(frames.frames().is_empty());
        assert_eq!(frames.worst_fps(), None);
    }

    /// The reason the cost is split: a frame the panels ate and a frame the world
    /// ate cost the same and are two different bugs. The total is the two of
    /// them and never the wait, which is the display holding the last frame.
    #[test]
    fn the_ui_and_the_world_are_charged_separately_and_the_wait_is_neither() {
        let mut frames = Frames::new(Duration::from_secs(4));
        // A HUD with every panel open over a world with nothing on screen.
        frames.record(
            Duration::from_millis(16),
            Duration::from_millis(9),
            Duration::from_millis(1),
            Duration::from_millis(6),
            None,
            false,
        );
        // The same total, the other way round.
        frames.record(
            Duration::from_millis(16),
            Duration::from_millis(1),
            Duration::from_millis(9),
            Duration::from_millis(6),
            None,
            false,
        );
        let held = frames.frames();
        assert_eq!(held[0].build(), held[1].build(), "the same frame time");
        assert!(held[0].ui > held[0].scene, "the panels ate the first");
        assert!(held[1].scene > held[1].ui, "the world ate the second");
        // Ten of sixteen milliseconds, and not sixteen of sixteen: a client that
        // slept out the vsync is idle, and reporting the sleep as build time
        // would call it saturated.
        assert_eq!(held[0].build(), Duration::from_millis(10));
    }

    /// **The defect this field was added for.** Two frames with the same rate,
    /// the same build time and the same wait: one is a client asleep on vsync
    /// with room to spare, the other is a client blocked on its own last frame.
    /// Before [`Frame::gpu`] the panel held exactly the same numbers for both.
    #[test]
    fn a_client_with_room_and_a_client_blocked_on_its_gpu_differ_only_in_the_gpu() {
        let mut frames = Frames::new(Duration::from_secs(4));
        let idle = (
            Duration::from_millis(16),
            Duration::from_millis(1),
            Duration::from_millis(1),
            Duration::from_millis(14),
        );
        // Idle: fourteen milliseconds of the sixteen were the display holding
        // the last frame, and the device did almost nothing.
        frames.record(
            idle.0,
            idle.1,
            idle.2,
            idle.3,
            Some(Duration::from_millis(2)),
            false,
        );
        // Saturated: every number above is identical, and the fourteen
        // milliseconds were the device still drawing.
        frames.record(
            idle.0,
            idle.1,
            idle.2,
            idle.3,
            Some(Duration::from_millis(15)),
            false,
        );
        let held = frames.frames();
        assert_eq!(held[0].interval, held[1].interval, "the same rate");
        assert_eq!(held[0].build(), held[1].build(), "the same build time");
        assert_eq!(held[0].wait, held[1].wait, "and the same wait");
        assert!(
            held[0].gpu.unwrap() < held[0].interval / 2,
            "the first has room: {:?}",
            held[0].gpu,
        );
        assert!(
            held[1].gpu.unwrap() > held[1].interval - held[1].build(),
            "the second is the bottleneck: {:?}",
            held[1].gpu,
        );
    }

    /// An adapter that cannot time itself says so, and the ring carries the
    /// absence through rather than substituting a zero somewhere along the way.
    #[test]
    fn a_device_that_cannot_time_itself_records_no_gpu_rather_than_none_of_it() {
        let mut frames = Frames::new(Duration::from_secs(4));
        let (interval, ui, scene, wait) = frame(16, 1, 1);
        frames.record(interval, ui, scene, wait, None, false);
        assert_eq!(frames.frames()[0].gpu, None);
    }

    /// A repack is a fact about *one* frame, not about the ring: a screen that
    /// is merely heavy never sets it, so the panel can tell "the world is
    /// full" apart from "the atlas just evicted" even though both are large
    /// numbers in [`Frame::scene`].
    #[test]
    fn a_repack_marks_only_the_frame_that_paid_for_it() {
        let mut frames = Frames::new(Duration::from_secs(4));
        let (interval, ui, scene, wait) = frame(16, 0, 1);
        frames.record(interval, ui, scene, wait, None, false);
        let (interval, ui, scene, wait) = frame(16, 0, 40);
        frames.record(interval, ui, scene, wait, None, true);
        let held = frames.frames();
        assert!(!held[0].repacked, "an ordinary frame");
        assert!(held[1].repacked, "the frame that evicted the atlas");
    }
}
