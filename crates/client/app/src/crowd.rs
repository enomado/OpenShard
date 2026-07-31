//! What the view does not keep: what each mobile was doing a moment ago.
//!
//! `WorldView` is a record of what arrived and deliberately nothing else, so it
//! cannot answer the two questions a picture needs — is this creature walking,
//! and how far into its walk is it. Both are about *history*: a `0x77` says
//! where a body is, and only the previous one says it moved.
//!
//! So this is the layer above the view that ages what it sees. It is small on
//! purpose: it holds a position, a group and a clock per serial, and it decides
//! nothing about the wire and nothing about a GPU.
//!
//! # A step is heard, not seen
//!
//! Nothing on the wire says "stopped walking". A shard sends a `0x77` per step
//! and then silence, so walking is inferred from a step having arrived recently
//! and standing from one not having. [`WALK_HOLD`] is how long "recently" is,
//! and it is one full step on foot rather than a number chosen to look right —
//! a body that took a step less than a step ago has not finished it.
//!
//! # And a step is a distance, not a jump
//!
//! The same history answers the other half: a step that has been running for
//! half its length is a body half a tile along, and drawing it on the tile the
//! packet named teleports it 44 pixels. So the step carries what it left and
//! when, and [`Crowd::glide_for`] turns that into the fraction
//! [`openshard_client_render::mobiles::Glide`] wants. The pixels are the
//! renderer's; the clock is here.
//!
//! # Why it lives here and not in `client/render`
//!
//! It reads `client/net`'s view and produces `client/render`'s `Mobile`. Putting
//! it in the renderer would make the renderer depend on the wire, which is the
//! one thing the crate layout forbids. It is the app's job to join the two, and
//! this is that join — with tests, because it is arithmetic over a clock and not
//! a picture.

use std::collections::HashMap;
use std::time::Duration;

use openshard_client_render::animation::AnimationClock;
use openshard_client_render::mobiles::{Glide, Mobile};
use openshard_movement::{RUN_INTERVAL, WALK_INTERVAL};
use openshard_protocol::direction::Facing;
use openshard_protocol::serial::Serial;
use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::Point;
use openshard_uofiles::anim::BodyKind;

/// How long a body keeps walking after the step that was last heard.
///
/// [`WALK_INTERVAL`] twice, and taken from there rather than written out: the
/// interval is how often the server will *allow* a step, and twice it is how
/// long one takes — 400ms, which `pace.rs`'s own test pins against ServUO's
/// `WalkFoot`. Shorter than a step and a walking body flickers between two
/// animations; much longer and it moonwalks on after it has stopped.
pub const WALK_HOLD: Duration = Duration::from_millis(2 * WALK_INTERVAL.as_millis() as u64);

/// The same, for a body the wire says is running.
///
/// [`RUN_INTERVAL`] doubled for the reason [`WALK_HOLD`] doubles its own: the
/// intervals in `common/movement` are anti-speedhack *floors*, deliberately half
/// the real rate, and ServUO's `RunFoot` is this. It has to be the real rate and
/// not the floor, because it is also how long the glide takes — held for twice
/// the step, a runner would be a whole tile behind itself and would jump forward
/// half a tile every time the next step arrived.
pub const RUN_HOLD: Duration = Duration::from_millis(2 * RUN_INTERVAL.as_millis() as u64);

/// A step in progress: where it came from, when it started, and how long it
/// takes.
#[derive(Clone, Copy, Debug)]
struct Step {
    /// The tile stepped off, or `None` when the move was not a step at all.
    ///
    /// Absent for a jump of more than one tile — a gate, a recall, a `0x22`
    /// putting a mispredicted body back. Interpolating one of those slides the
    /// character across the map over the length of a step, which is a far
    /// stranger picture than the teleport it is hiding.
    from: Option<Point>,
    /// When it was heard, on [`Crowd::now`]'s clock.
    started: Duration,
    /// How long it takes: [`WALK_HOLD`] or [`RUN_HOLD`], by the wire's own
    /// running flag. Never zero, which is what lets [`Tracked::glide`] divide.
    takes: Duration,
}

/// One mobile's history: where it was, what it is playing, and since when.
#[derive(Clone, Copy, Debug)]
struct Tracked {
    /// Where the last packet put it.
    at: Point,
    /// Which body it was last seen as. Kept because a walk that ends has to
    /// know what "standing" means, and a horse and a player stop into different
    /// group numbers — see [`BodyKind::standing`].
    body: u16,
    /// Which animation group is playing.
    group: u8,
    /// The step it is in the middle of.
    ///
    /// `None` for a body that is standing. Not an "unknown": a standing body
    /// genuinely has no step to finish, which is what [`Option`] is for here.
    step: Option<Step>,
    /// Its own animation clock.
    ///
    /// Per mobile, and reset when the group changes: one clock for everybody
    /// makes a standing crowd breathe in unison, which is wrong and looks it,
    /// and a clock carried across a group change starts the new animation
    /// wherever the old one happened to be.
    clock: AnimationClock,
}

/// Who a tracked body is.
///
/// `None` is this client's own body *before a shard has named it* — the offline
/// map viewer walks a placeholder around with no serial, because nobody has
/// given it one. Absent rather than zero: a serial of zero is a real wire value
/// meaning "nothing", and a made-up one would collide with a real mobile the
/// moment the client logs in.
pub type Who = Option<Serial>;

/// Everyone on screen, aged.
#[derive(Clone, Debug, Default)]
pub struct Crowd {
    tracked: HashMap<Who, Tracked>,
    /// Real time since this crowd was built. Its own clock rather than an
    /// `Instant`, so every rule here can be tested by handing it durations.
    now: Duration,
}

impl Crowd {
    /// Move every clock forward, and stop whoever has finished their step.
    pub fn advance(&mut self, dt: Duration) {
        self.now += dt;
        let now = self.now;
        for tracked in self.tracked.values_mut() {
            tracked.clock.advance(dt);
            if tracked.step.is_some_and(|step| now >= step.started + step.takes) {
                tracked.step = None;
                tracked.change_to(BodyKind::of(tracked.body).standing());
            }
        }
    }

    /// What the server says about a body now, folded into what it said before.
    ///
    /// Returns the mobile to draw. The frame is left at zero — the atlas is what
    /// knows how many frames there are, and it is not built yet when this is
    /// called; [`Crowd::frame_for`] fills it in once it is.
    pub fn see(&mut self, who: Who, at: Point, body: Graphic, facing: Facing, hue: Hue) -> Mobile {
        let kind = BodyKind::of(body.0);
        let now = self.now;
        let tracked = self.tracked.entry(who).or_insert(Tracked {
            at,
            body: body.0,
            // A body first heard of is standing: it may well be mid-stride, but
            // the only thing that could say so is a previous packet and there
            // is none.
            group: kind.standing(),
            step: None,
            clock: AnimationClock::default(),
        });

        // A step is a *position* change. A turn on the spot is not one — the
        // client draws a turning body standing, and a facing change arrives for
        // every step too, so treating it as movement would keep everyone
        // walking forever.
        if tracked.at != at {
            let from = tracked.at;
            tracked.at = at;
            tracked.step = Some(Step {
                from: is_one_step(from, at).then_some(from),
                started: now,
                takes: if facing.running { RUN_HOLD } else { WALK_HOLD },
            });
            let moving = match (facing.running, kind.running()) {
                (true, Some(running)) => running,
                // A running monster walks: `HighAnimationGroup` has no run, and
                // this is the client's answer too.
                _ => kind.walking(),
            };
            tracked.change_to(moving);
        }
        tracked.body = body.0;

        Mobile {
            at,
            body: body.0,
            group: tracked.group,
            facing: facing.direction,
            frame: 0,
            hue,
            glide: tracked.glide(now),
        }
    }

    /// Which frame this body is on, out of what the atlas turned out to pack.
    ///
    /// Asked after the atlas is built, for the reason the frame is not filled in
    /// by [`Crowd::see`]: the count belongs to the atlas and the atlas belongs
    /// to the frame being drawn.
    pub fn frame_for(&self, who: Who, frame_count: u16) -> u16 {
        self.tracked
            .get(&who)
            .map_or(0, |tracked| tracked.clock.frame(frame_count))
    }

    /// How far into its step this body is, if it is in one.
    ///
    /// Asked every frame and not only when a packet arrives, which is the whole
    /// point: a glide read once and stored would freeze at whatever it was when
    /// the `0x77` landed, and the body would jump a tile on the next one — the
    /// teleport this exists to remove, arriving 400ms late.
    pub fn glide_for(&self, who: Who) -> Option<Glide> {
        self.tracked.get(&who)?.glide(self.now)
    }

    /// Whether anybody is part way through a step.
    ///
    /// What the window asks to decide how often to redraw: a crowd standing
    /// still changes a pixel once every `FRAME_DELAY` and one mid-step changes
    /// several every frame. A glide drawn on the animation clock arrives in
    /// five visible jumps, which is the teleport it exists to remove.
    pub fn anyone_gliding(&self) -> bool {
        self.tracked
            .values()
            .any(|tracked| tracked.glide(self.now).is_some())
    }

    /// Forget everyone not in this set.
    ///
    /// Called with the serials the view still holds: a mobile that walked out of
    /// range is gone, and a `HashMap` that kept it would grow for as long as the
    /// client is connected. Forgetting is also the right *behaviour* — one that
    /// comes back is a body seen for the first time again, and pretending to
    /// remember what it was doing while off screen would be inventing it.
    pub fn retain(&mut self, present: impl Fn(Who) -> bool) {
        self.tracked.retain(|who, _| present(*who));
    }
}

impl Tracked {
    /// Start playing a group, restarting the clock if it is a different one.
    fn change_to(&mut self, group: u8) {
        if self.group != group {
            self.group = group;
            self.clock = AnimationClock::default();
        }
    }

    /// Where between two tiles this body is, at a moment on the crowd's clock.
    fn glide(&self, now: Duration) -> Option<Glide> {
        let step = self.step?;
        let from = step.from?;
        // Saturating rather than asserted: `now` is the crowd's own clock and
        // only ever moves forward, but a step heard *this* instant divides
        // zero by the step's length, which is the honest zero.
        let elapsed = now.saturating_sub(step.started);
        Some(Glide {
            from,
            progress: (elapsed.as_secs_f32() / step.takes.as_secs_f32()).clamp(0.0, 1.0),
        })
    }
}

/// Whether a move of one tile is what happened, in the distance UO measures.
///
/// Chebyshev, because the grid's eight directions are all one step: a diagonal
/// moves both axes and is no further than a straight one. Anything longer is a
/// teleport wearing a step's clothes.
fn is_one_step(from: Point, to: Point) -> bool {
    let dx = i32::from(to.x) - i32::from(from.x);
    let dy = i32::from(to.y) - i32::from(from.y);
    dx.abs().max(dy.abs()) <= 1
}

#[cfg(test)]
mod tests {
    use openshard_protocol::direction::Direction;

    use super::*;

    const PLAYER: u16 = 400;
    const HORSE: u16 = 204;
    const DRAGON: u16 = 12;

    /// A serial, as the crowd keys them.
    fn serial(raw: u32) -> Who {
        Some(Serial::new(raw).expect("a nonzero serial"))
    }

    /// A body nobody has seen before is standing, in its own kind's numbering.
    #[test]
    fn a_body_first_heard_of_is_standing() {
        let mut crowd = Crowd::default();
        let human = crowd.see(
            serial(1),
            Point::new(10, 10, 0),
            Graphic(PLAYER),
            Facing::walking(Direction::South),
            Hue::NONE,
        );
        assert_eq!(human.group, 4, "PeopleAnimationGroup.Stand");
        let horse = crowd.see(
            serial(2),
            Point::new(10, 10, 0),
            Graphic(HORSE),
            Facing::walking(Direction::South),
            Hue::NONE,
        );
        assert_eq!(horse.group, 2, "LowAnimationGroup.Stand");
        let dragon = crowd.see(
            serial(3),
            Point::new(10, 10, 0),
            Graphic(DRAGON),
            Facing::walking(Direction::South),
            Hue::NONE,
        );
        assert_eq!(dragon.group, 1, "HighAnimationGroup.Stand");
    }

    /// A step starts a walk, and the walk ends by itself: nothing on the wire
    /// says "stopped".
    #[test]
    fn a_step_walks_and_silence_stands() {
        let mut crowd = Crowd::default();
        let step = |crowd: &mut Crowd, x: u16| {
            crowd.see(
                serial(1),
                Point::new(x, 10, 0),
                Graphic(PLAYER),
                Facing::walking(Direction::South),
                Hue::NONE,
            )
        };
        assert_eq!(step(&mut crowd, 10).group, 4, "standing to begin with");
        assert_eq!(step(&mut crowd, 11).group, 0, "a step is a walk");

        // Most of a step later it is still walking, which is what keeps a body
        // that is genuinely walking from flickering between two animations.
        crowd.advance(WALK_HOLD / 2);
        assert_eq!(step(&mut crowd, 11).group, 0, "no new step, but not done yet");
        crowd.advance(WALK_HOLD);
        assert_eq!(step(&mut crowd, 11).group, 4, "and then it stands");
    }

    /// A body that keeps stepping keeps walking, however long it goes on.
    #[test]
    fn a_body_that_keeps_stepping_never_stands() {
        let mut crowd = Crowd::default();
        for x in 10..30u16 {
            crowd.see(
                serial(1),
                Point::new(x, 10, 0),
                Graphic(PLAYER),
                Facing::walking(Direction::South),
                Hue::NONE,
            );
            // Each step arrives before the previous one has finished, which is
            // what a real walk looks like.
            crowd.advance(WALK_HOLD * 3 / 4);
        }
        let drawn = crowd.see(
            serial(1),
            Point::new(30, 10, 0),
            Graphic(PLAYER),
            Facing::walking(Direction::South),
            Hue::NONE,
        );
        assert_eq!(drawn.group, 0);
    }

    /// Turning on the spot is not a step.
    ///
    /// A facing change arrives with every step too, so a layer that watched the
    /// facing instead of the position would keep a standing crowd walking on
    /// the spot forever — and would still pass the test above.
    #[test]
    fn a_turn_on_the_spot_is_not_a_step() {
        let mut crowd = Crowd::default();
        let at = Point::new(10, 10, 0);
        crowd.see(
            serial(1),
            at,
            Graphic(PLAYER),
            Facing::walking(Direction::South),
            Hue::NONE,
        );
        let turned = crowd.see(
            serial(1),
            at,
            Graphic(PLAYER),
            Facing::walking(Direction::North),
            Hue::NONE,
        );
        assert_eq!(turned.group, 4, "still standing");
        assert_eq!(turned.facing, Direction::North, "and facing the new way");
    }

    /// Running is the wire's own flag, and a monster has no run to play.
    #[test]
    fn running_is_a_group_of_its_own_where_the_kind_has_one() {
        let mut crowd = Crowd::default();
        let run = |crowd: &mut Crowd, who: u32, body: u16, x: u16| {
            crowd.see(
                serial(who),
                Point::new(x, 10, 0),
                Graphic(body),
                Facing::running(Direction::South),
                Hue::NONE,
            )
        };
        run(&mut crowd, 1, PLAYER, 10);
        assert_eq!(run(&mut crowd, 1, PLAYER, 11).group, 2, "RunUnarmed");
        run(&mut crowd, 2, HORSE, 10);
        assert_eq!(run(&mut crowd, 2, HORSE, 11).group, 1, "LowAnimationGroup.Run");
        run(&mut crowd, 3, DRAGON, 10);
        assert_eq!(run(&mut crowd, 3, DRAGON, 11).group, 0, "High walks instead");
    }

    /// Everybody keeps their own clock, so a crowd does not breathe in unison.
    #[test]
    fn two_bodies_that_started_at_different_times_are_on_different_frames() {
        let mut crowd = Crowd::default();
        let stand = |crowd: &mut Crowd, who: u32| {
            crowd.see(
                serial(who),
                Point::new(10, 10, 0),
                Graphic(PLAYER),
                Facing::walking(Direction::South),
                Hue::NONE,
            );
        };
        stand(&mut crowd, 1);
        crowd.advance(Duration::from_millis(80 * 3));
        stand(&mut crowd, 2);
        assert_eq!(crowd.frame_for(serial(1), 6), 3);
        assert_eq!(crowd.frame_for(serial(2), 6), 0, "the newcomer starts at zero");
        // And a serial nobody is tracking answers with a frame rather than
        // nothing: the atlas may hold a body the crowd has forgotten.
        assert_eq!(crowd.frame_for(serial(3), 6), 0);
    }

    /// A group change restarts the clock, so a walk begins at its first frame
    /// rather than wherever the stand had got to.
    #[test]
    fn changing_group_restarts_the_animation() {
        let mut crowd = Crowd::default();
        crowd.see(
            serial(1),
            Point::new(10, 10, 0),
            Graphic(PLAYER),
            Facing::walking(Direction::South),
            Hue::NONE,
        );
        crowd.advance(Duration::from_millis(80 * 5));
        assert_eq!(crowd.frame_for(serial(1), 6), 5);
        crowd.see(
            serial(1),
            Point::new(11, 10, 0),
            Graphic(PLAYER),
            Facing::walking(Direction::South),
            Hue::NONE,
        );
        assert_eq!(crowd.frame_for(serial(1), 6), 0, "the walk starts at its start");
    }

    /// A step is walked across, not jumped: the glide starts on the tile left
    /// behind and reaches the new one exactly when the walk ends.
    #[test]
    fn a_step_is_glided_across_and_ends_on_its_tile() {
        let mut crowd = Crowd::default();
        let step = |crowd: &mut Crowd, x: u16| {
            crowd.see(
                serial(1),
                Point::new(x, 10, 0),
                Graphic(PLAYER),
                Facing::walking(Direction::SouthEast),
                Hue::NONE,
            )
        };
        assert_eq!(step(&mut crowd, 10).glide, None, "standing still is not a step");

        let stepped = step(&mut crowd, 11);
        let glide = stepped.glide.expect("mid-step");
        assert_eq!(glide.from, Point::new(10, 10, 0), "from where it was");
        assert_eq!(glide.progress, 0.0, "and it has not moved yet");

        crowd.advance(WALK_HOLD / 4);
        assert_eq!(crowd.glide_for(serial(1)).expect("still stepping").progress, 0.25);
        crowd.advance(WALK_HOLD / 4);
        assert_eq!(crowd.glide_for(serial(1)).expect("still stepping").progress, 0.5);
        // The instant the walk ends the body is on its tile and has no step
        // left to be part of the way through — the two have to agree, or the
        // sprite jumps the remaining pixels as the animation stops.
        crowd.advance(WALK_HOLD / 2);
        assert_eq!(crowd.glide_for(serial(1)), None);
        assert_eq!(step(&mut crowd, 11).group, 4, "standing");
    }

    /// A running body crosses its tile in half the time, because it takes half
    /// as long to take the step.
    ///
    /// The hold and the glide are one number for exactly this reason: held for
    /// a walk's length, a runner would still be half a tile back when the next
    /// step arrived and would jump forward to catch up, every step.
    #[test]
    fn a_runner_glides_in_half_the_time() {
        let mut crowd = Crowd::default();
        let run = |crowd: &mut Crowd, x: u16| {
            crowd.see(
                serial(1),
                Point::new(x, 10, 0),
                Graphic(PLAYER),
                Facing::running(Direction::SouthEast),
                Hue::NONE,
            )
        };
        run(&mut crowd, 10);
        run(&mut crowd, 11);
        crowd.advance(RUN_HOLD / 2);
        assert_eq!(crowd.glide_for(serial(1)).expect("mid-step").progress, 0.5);
        crowd.advance(RUN_HOLD / 2);
        assert_eq!(crowd.glide_for(serial(1)), None, "and it is there");
        assert_eq!(run(&mut crowd, 11).group, 4, "standing, half a walk early");
        assert_eq!(RUN_HOLD * 2, WALK_HOLD, "ServUO's RunFoot against its WalkFoot");
    }

    /// A jump of more than one tile is a teleport, and is not glided.
    ///
    /// A gate, a recall, or a `0x22` putting a mispredicted body back: sliding
    /// smoothly across half a facet takes the same 400ms as a step and looks
    /// far stranger than the teleport it is hiding.
    #[test]
    fn a_teleport_is_not_glided() {
        let mut crowd = Crowd::default();
        let go = |crowd: &mut Crowd, x: u16, y: u16| {
            crowd.see(
                serial(1),
                Point::new(x, y, 0),
                Graphic(PLAYER),
                Facing::walking(Direction::SouthEast),
                Hue::NONE,
            )
        };
        go(&mut crowd, 10, 10);
        // A diagonal is one step: both axes move, and UO measures Chebyshev.
        assert!(go(&mut crowd, 11, 11).glide.is_some(), "a diagonal is a step");
        assert_eq!(go(&mut crowd, 1500, 1500).glide, None, "and this is not");
        assert_eq!(crowd.glide_for(serial(1)), None);
    }

    /// The window redraws fast only while there is something to redraw fast
    /// for, and a teleport is not one: it moves the body once, in one frame.
    #[test]
    fn only_a_step_asks_the_window_for_frames() {
        let mut crowd = Crowd::default();
        let go = |crowd: &mut Crowd, x: u16| {
            crowd.see(
                serial(1),
                Point::new(x, 10, 0),
                Graphic(PLAYER),
                Facing::walking(Direction::NorthEast),
                Hue::NONE,
            );
        };
        go(&mut crowd, 10);
        assert!(!crowd.anyone_gliding(), "standing still");
        go(&mut crowd, 11);
        assert!(crowd.anyone_gliding(), "mid-step");
        crowd.advance(WALK_HOLD);
        assert!(!crowd.anyone_gliding(), "arrived");
        go(&mut crowd, 900);
        assert!(!crowd.anyone_gliding(), "a teleport is drawn once and done");
    }

    /// Turning on the spot moves nothing, so there is nothing to glide across.
    #[test]
    fn a_turn_is_not_glided() {
        let mut crowd = Crowd::default();
        let at = Point::new(10, 10, 0);
        crowd.see(
            serial(1),
            at,
            Graphic(PLAYER),
            Facing::walking(Direction::South),
            Hue::NONE,
        );
        let turned = crowd.see(
            serial(1),
            at,
            Graphic(PLAYER),
            Facing::walking(Direction::North),
            Hue::NONE,
        );
        assert_eq!(turned.glide, None);
    }

    /// Whoever the view no longer holds is forgotten, or the map grows for as
    /// long as the client is connected.
    #[test]
    fn a_mobile_the_view_dropped_is_forgotten() {
        let mut crowd = Crowd::default();
        for who in 1..=3 {
            crowd.see(
                serial(who),
                Point::new(10, 10, 0),
                Graphic(PLAYER),
                Facing::walking(Direction::South),
                Hue::NONE,
            );
        }
        crowd.advance(Duration::from_millis(80));
        crowd.retain(|who| who == serial(2));
        assert_eq!(crowd.tracked.len(), 1);
        // And the one that came back is new: its clock starts again rather than
        // resuming a walk nobody watched.
        assert_eq!(crowd.frame_for(serial(1), 6), 0);
        assert_eq!(crowd.frame_for(serial(2), 6), 1);
    }
}
