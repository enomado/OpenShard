//! Where the player is asking to walk, and how often the ask may be sent.
//!
//! # Two idioms, one clock
//!
//! A held arrow says *which way*, and a click on the ground says *where to* —
//! the strategy game's idiom, and the one this client is asked for. They are the
//! same question a step at a time, so they are answered in one place: whichever
//! is asking, a step leaves every [`crowd::WALK_HOLD`] (or every
//! [`crowd::RUN_HOLD`] with shift down, which is what running is), and never two
//! at once. Two timers, one per input, would take two steps a beat the moment a
//! player nudged an arrow while walking to a destination.
//!
//! The keyboard wins where both are asking, and a press *drops* the destination
//! rather than queuing behind it: taking hold of the arrows is how a player says
//! they no longer want to go where they clicked.
//!
//! # The rate is the step's own length, not the anti-speedhack floor
//!
//! `common/movement`'s intervals are floors, deliberately half the real rate so
//! that jitter never trips the check (see `pace.rs`). Walking *at* the floor
//! would be moving twice as fast as a body moves, and the crowd — which glides a
//! step over its own length — would have a walker arrive half a tile before the
//! next step and stand there. The hold in `crowd` is already that real length,
//! for exactly this reason, so it is what is read here rather than a second
//! number that could disagree with it.
//!
//! # The first ask steps at once
//!
//! Waiting a whole step before the first one would put 400ms between the input
//! and the character. So a press that changes the direction, and a click that
//! names a new destination, are due immediately and the timer is armed from
//! there — unless one is already armed, which is what keeps a right-drag across
//! the ground from sending a step per pixel of mouse travel.
//!
//! # Walking to a tile is greedy, and knows when to stop
//!
//! The step taken toward a destination is the straight-line direction to it and
//! nothing cleverer: this end has no walkability to plan over — whether a step
//! is allowed is the server's answer, arriving as a `0x21` (see
//! `client/net`'s `walk`) — so a wall between here and there is discovered by
//! walking into it. What must not happen is walking into it for ever, so a
//! destination that has not moved the body in [`STUCK_STEPS`] steps is given up
//! on. Planning a route around the wall wants a `Terrain` over the client's own
//! map, which is a backlog item in `docs/client.md` and not a reason to have no
//! click-to-walk at all.

use std::time::{Duration, Instant};

use openshard_movement::direction_toward;
use openshard_protocol::direction::{Direction, Facing};
use openshard_protocol::world::Point;

use crate::crowd::{RUN_HOLD, WALK_HOLD};
use crate::keys::Held;

/// How many steps in a row may leave the body exactly where it was before a walk
/// to a destination gives up.
///
/// More than one, because a step that only turns the body is a legitimate one —
/// UO makes turning a whole step — and because the position this is measured
/// against is the server's last word, which lags whatever is in flight. Four
/// steps is a second and a half of walking on the spot, which is long enough to
/// be sure and short enough that nobody watches it happen twice.
const STUCK_STEPS: u8 = 4;

/// Which way the player is asking to walk, from every input that can ask.
#[derive(Clone, Debug, Default)]
pub struct Steering {
    /// The arrows, and shift.
    keys: Held,
    /// The tile the mouse last asked for.
    ///
    /// Absent means absent: nobody has clicked, or the body has arrived. Not a
    /// "no destination yet" — a body standing where it was told to stand has
    /// genuinely nowhere left to go.
    goal: Option<(u16, u16)>,
    /// When the next step may be sent, or `None` when nothing is asking.
    due: Option<Instant>,
    /// Where the body stood when the last step was sent, for [`STUCK_STEPS`].
    was: Option<Point>,
    /// How many steps in a row have left it there.
    stalled: u8,
    /// The direction of the last step sent, once one has been.
    ///
    /// Which way the body is *going* to face, which is a step ahead of the way
    /// it is drawn facing: the caller's facing comes back through the shard
    /// thread, and a second step decided from it would turn twice. Absent until
    /// this has asked for anything, and then the caller's facing is the only
    /// answer there is.
    asked: Option<Direction>,
}

impl Steering {
    /// An arrow went down. Answers the step to send now, if any.
    ///
    /// The destination goes with it: see the module docs.
    pub fn press(&mut self, direction: Direction, now: Instant, facing: Direction) -> Option<Facing> {
        if !self.keys.press(direction) {
            // The operating system repeating a key that is already the one being
            // obeyed. Its rate is not a walking speed.
            return None;
        }
        self.goal = None;
        self.stalled = 0;
        self.was = None;
        let asking = self.keys.asking();
        self.charge(asking, now, facing);
        asking
    }

    /// An arrow came up.
    pub fn release(&mut self, direction: Direction) {
        self.keys.release(direction);
        if self.keys.asking().is_none() && self.goal.is_none() {
            self.disarm();
        }
    }

    /// Shift went down or came up.
    ///
    /// Deliberately not re-timed: a walker that starts running mid-step keeps
    /// the deadline it already had, and the next one is a run's. Re-arming here
    /// would let a player tapping shift send a step per tap.
    pub fn set_running(&mut self, running: bool) {
        self.keys.set_running(running);
    }

    /// Walk to `tile`, from wherever the body is standing now. Answers the step
    /// to send this instant, if the clock is free for one.
    ///
    /// Called on a click and again on every mouse move while the button is held,
    /// which is what makes dragging steer: the destination is replaced and the
    /// cadence is untouched.
    pub fn go_to(
        &mut self,
        tile: (u16, u16),
        from: Point,
        now: Instant,
        facing: Direction,
    ) -> Option<Facing> {
        self.goal = Some(tile);
        self.stalled = 0;
        self.was = None;
        // Only when nothing is timed already — otherwise a drag across the
        // ground would send a step per mouse event.
        if self.due.is_some() {
            return None;
        }
        self.take(from, now, facing)
    }

    /// Everything is up and nowhere is asked for.
    ///
    /// The window losing focus is a key release that never arrives, and a
    /// character that keeps walking while its player is in another window is not
    /// what any of these inputs meant.
    pub fn clear(&mut self) {
        self.keys.clear();
        self.goal = None;
        self.disarm();
    }

    /// The tile being walked to, for the marker the HUD draws on it.
    pub const fn goal(&self) -> Option<(u16, u16)> {
        self.goal
    }

    /// The step due by now, if one is.
    ///
    /// `from` is where the body stands — the server's last word on it, which is
    /// also what a destination is steered from. Called from the wait loop, so it
    /// charges one step per call rather than catching up on a stall: a window
    /// that was minimised for a minute has not banked a hundred and fifty steps,
    /// and sending them would be the flood this pacing exists to prevent.
    pub fn due(&mut self, now: Instant, from: Point, facing: Direction) -> Option<Facing> {
        match self.due {
            Some(due) if now >= due => self.take(from, now, facing),
            _ => None,
        }
    }

    /// When the next step is due, for the event loop's deadline.
    pub const fn deadline(&self) -> Option<Instant> {
        self.due
    }

    /// Take the step that is due: work out which way, arm the next one, and give
    /// up on a destination that is not getting any closer.
    fn take(&mut self, from: Point, now: Instant, facing: Direction) -> Option<Facing> {
        // The stall check is the destination's alone. An arrow held against a
        // wall is the player's own doing and stops when they let go.
        if self.keys.asking().is_none() && self.goal.is_some() {
            self.stalled = match self.was {
                Some(was) if was == from => self.stalled + 1,
                _ => 0,
            };
            if self.stalled >= STUCK_STEPS {
                self.goal = None;
            }
        }

        let asking = self.asking(from);
        match asking {
            Some(step) => {
                self.was = Some(from);
                self.charge(asking, now, facing);
                Some(step)
            }
            // Nothing left to ask for: the arrows are up and the body is
            // standing where it was sent. The clock stops with them, so the
            // event loop goes back to sleeping on the animation.
            None => {
                self.disarm();
                None
            }
        }
    }

    /// Arm the clock for whatever comes after the step just sent, and remember
    /// which way that step went.
    ///
    /// # A turn is a step that costs no time
    ///
    /// Turning is a whole step in UO — a mobile asked for a direction it is not
    /// facing turns and moves nowhere, and the shard answers it with its own
    /// `0x22`. What it is *not* is a step against the pace budget: the reference
    /// returns a turn before the bucket is touched, and so does ours
    /// (`openshard_movement::Walker::request`), because spinning on the spot is
    /// something clients genuinely do and throttling it would be absurd.
    ///
    /// So the step a turn precedes is due at once rather than a hold later. The
    /// hold was ours and nothing asked for it: it put 400ms of standing still
    /// between the player pressing a new direction and their character setting
    /// off, which is not what the game does and not what anyone remembers it
    /// doing. The caller takes both in one wake, so the two `0x02`s leave
    /// together and the body starts moving on the frame the key went down.
    fn charge(&mut self, asking: Option<Facing>, now: Instant, facing: Direction) {
        let Some(step) = asking else {
            self.disarm();
            return;
        };
        // What the body will be facing, which is a step ahead of what the caller
        // can see: our own last ask has not been round the shard thread yet.
        let facing = self.asked.unwrap_or(facing);
        self.asked = Some(step.direction);
        self.due = Some(match step.direction == facing {
            true => self.next_due(now),
            false => now,
        });
    }

    /// Which way to step from `from`, keyboard first. Clears a destination the
    /// body is already standing on.
    fn asking(&mut self, from: Point) -> Option<Facing> {
        if let Some(facing) = self.keys.asking() {
            return Some(facing);
        }
        let (x, y) = self.goal?;
        // The height is the body's own: `direction_toward` reads the two axes a
        // map has and a destination clicked on the ground carries no third one.
        match direction_toward(from, Point::new(x, y, from.z)) {
            Some(direction) => Some(match self.keys.running() {
                true => Facing::running(direction),
                false => Facing::walking(direction),
            }),
            // Arrived.
            None => {
                self.goal = None;
                None
            }
        }
    }

    /// When the step after this one is due.
    ///
    /// Measured from the deadline that has just passed and not from the moment
    /// the event loop got round to it. The loop is woken by the operating
    /// system whenever it gets round to it and never early, so arming from
    /// `now` folds every wake's lateness into the cadence — where it
    /// *accumulates*: a handful of milliseconds a step is a body a fifth of a
    /// tile behind after ten and a whole tile behind after fifty, and nothing
    /// ever gives it back. Found by the walk oracle in `dst.rs`, which is
    /// exactly the divergence it exists to see: every unit involved was right
    /// about its own rate and the body still fell behind the player's hand.
    ///
    /// A wake later than a whole step is not jitter — the window was minimised
    /// or the machine asleep — and those steps are deliberately not banked (see
    /// [`Steering::due`]), so the cadence starts again from `now`.
    fn next_due(&self, now: Instant) -> Instant {
        let interval = self.interval();
        match self.due {
            Some(due) if now < due + interval => due + interval,
            _ => now + interval,
        }
    }

    /// Stop the clock, and forget what it was measuring.
    fn disarm(&mut self) {
        self.due = None;
        self.was = None;
        self.stalled = 0;
        // And the facing goes with the clock: nothing is in flight, so the
        // body's own is the truthful answer again by the time anything asks.
        self.asked = None;
    }

    /// How long a step takes at the pace being asked for.
    fn interval(&self) -> Duration {
        match self.keys.running() {
            true => RUN_HOLD,
            false => WALK_HOLD,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The clock is a parameter here as it is in `WalkPace`, so a rate can be
    /// tested without sleeping through one.
    fn at(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    /// Where the body stands in the tests below. Nothing here reads a map, so
    /// the height is only carried through.
    fn here() -> Point {
        Point::new(100, 100, 0)
    }

    #[test]
    fn a_press_steps_at_once_and_then_at_the_walking_rate() {
        let start = Instant::now();
        let mut steering = Steering::default();

        assert_eq!(
            steering.press(Direction::NorthWest, start, Direction::NorthWest),
            Some(Facing::walking(Direction::NorthWest))
        );
        // Nothing is due until a whole step has passed.
        assert_eq!(steering.due(at(start, 399), here(), Direction::NorthWest), None);
        assert_eq!(
            steering.due(at(start, 400), here(), Direction::NorthWest),
            Some(Facing::walking(Direction::NorthWest))
        );
        assert_eq!(steering.due(at(start, 401), here(), Direction::NorthWest), None);
    }

    /// The operating system repeats a held key at its own rate, and that rate is
    /// not a walking speed — so a repeat neither sends a step nor re-arms the
    /// clock.
    #[test]
    fn the_operating_systems_repeat_is_not_a_step() {
        let start = Instant::now();
        let mut steering = Steering::default();

        steering
            .press(Direction::NorthWest, start, Direction::NorthWest)
            .unwrap();
        for repeat in 1..30 {
            assert_eq!(
                steering.press(Direction::NorthWest, at(start, repeat * 10), Direction::NorthWest),
                None
            );
        }
        assert_eq!(steering.deadline(), Some(at(start, 400)), "the first press's");
    }

    #[test]
    fn shift_is_the_running_flag_and_halves_the_gap() {
        let start = Instant::now();
        let mut steering = Steering::default();
        steering.set_running(true);

        assert_eq!(
            steering.press(Direction::SouthEast, start, Direction::SouthEast),
            Some(Facing::running(Direction::SouthEast))
        );
        assert_eq!(steering.due(at(start, 199), here(), Direction::SouthEast), None);
        assert_eq!(
            steering.due(at(start, 200), here(), Direction::SouthEast),
            Some(Facing::running(Direction::SouthEast))
        );
    }

    /// Shift pressed mid-walk does not itself send anything: the deadline in
    /// flight is kept and the pace changes from the next step on.
    #[test]
    fn shift_mid_step_does_not_send_a_step_of_its_own() {
        let start = Instant::now();
        let mut steering = Steering::default();

        steering.press(Direction::North, start, Direction::North).unwrap();
        steering.set_running(true);
        assert_eq!(
            steering.due(at(start, 200), here(), Direction::North),
            None,
            "the walk's deadline stands"
        );
        assert_eq!(
            steering.due(at(start, 400), here(), Direction::North),
            Some(Facing::running(Direction::North))
        );
        // And from there it is a runner's.
        assert_eq!(
            steering.due(at(start, 600), here(), Direction::North),
            Some(Facing::running(Direction::North))
        );
    }

    #[test]
    fn nothing_asked_for_is_nothing_due() {
        let start = Instant::now();
        let mut steering = Steering::default();

        steering.press(Direction::West, start, Direction::West).unwrap();
        steering.release(Direction::West);
        assert_eq!(steering.deadline(), None);
        assert_eq!(steering.due(at(start, 10_000), here(), Direction::West), None);
    }

    /// A click walks toward the tile, a step at a time, and stops on it.
    #[test]
    fn a_destination_is_walked_to_and_let_go_of_on_arrival() {
        let start = Instant::now();
        let mut steering = Steering::default();

        // Three tiles east, at the same row.
        assert_eq!(
            steering.go_to((103, 100), here(), start, Direction::East),
            Some(Facing::walking(Direction::East)),
            "the first step leaves at once"
        );
        let mut now = start;
        for x in 101..=102 {
            now = at(start, 400 * u64::from(x - 100));
            assert_eq!(
                steering.due(now, Point::new(x, 100, 0), Direction::East),
                Some(Facing::walking(Direction::East)),
                "still short of it"
            );
        }
        // Standing on it: nothing more is asked for, and the clock stops with
        // the asking.
        assert_eq!(
            steering.due(at(now, 400), Point::new(103, 100, 0), Direction::East),
            None
        );
        assert_eq!(steering.goal(), None);
        assert_eq!(steering.deadline(), None);
    }

    /// A diagonal is one step, so a destination off both axes is walked to
    /// diagonally rather than in two moves.
    #[test]
    fn a_destination_off_both_axes_is_stepped_diagonally() {
        let start = Instant::now();
        let mut steering = Steering::default();
        assert_eq!(
            steering.go_to((105, 105), here(), start, Direction::SouthEast),
            Some(Facing::walking(Direction::SouthEast))
        );
    }

    /// Dragging the mouse across the ground restates the destination on every
    /// move, and must not send a step on every one of them.
    #[test]
    fn restating_a_destination_does_not_restart_the_cadence() {
        let start = Instant::now();
        let mut steering = Steering::default();

        steering
            .go_to((110, 100), here(), start, Direction::East)
            .unwrap();
        for tick in 1..20 {
            assert_eq!(
                steering.go_to(
                    (110, 100 + tick as u16),
                    here(),
                    at(start, tick * 10),
                    Direction::East
                ),
                None
            );
        }
        assert_eq!(steering.deadline(), Some(at(start, 400)));
    }

    /// A wall this end cannot see is discovered by walking into it, and the walk
    /// gives up rather than shuffling against it for ever.
    #[test]
    fn a_destination_that_never_gets_closer_is_given_up_on() {
        let start = Instant::now();
        let mut steering = Steering::default();

        steering
            .go_to((200, 100), here(), start, Direction::East)
            .unwrap();
        // The body never moves: every step is refused by the server, which
        // snaps it back to where it was. The click's own step is the first of
        // the four, so three more are tried after it.
        for step in 1..u64::from(STUCK_STEPS) {
            assert!(
                steering
                    .due(at(start, 400 * step), here(), Direction::East)
                    .is_some(),
                "step {step} is still worth trying"
            );
        }
        assert_eq!(
            steering.due(at(start, 400 * u64::from(STUCK_STEPS)), here(), Direction::East),
            None
        );
        assert_eq!(steering.goal(), None, "and the destination is let go of");
    }

    /// A body that *is* making progress keeps its destination however long the
    /// walk takes.
    #[test]
    fn progress_resets_the_patience() {
        let start = Instant::now();
        let mut steering = Steering::default();
        steering
            .go_to((100, 130), here(), start, Direction::South)
            .unwrap();

        for y in 101..=120u16 {
            let now = at(start, 400 * u64::from(y - 100));
            // Every other step is refused, which is what a body squeezing past
            // furniture looks like — it must not add up to a stall.
            let position = Point::new(100, y - u16::from(y % 2 == 0), 0);
            assert!(steering.due(now, position, Direction::South).is_some(), "row {y}");
        }
        assert_eq!(steering.goal(), Some((100, 130)));
    }

    /// Taking hold of the arrows is how a player says they no longer want to go
    /// where they clicked.
    #[test]
    fn the_keyboard_takes_over_from_a_destination() {
        let start = Instant::now();
        let mut steering = Steering::default();

        steering
            .go_to((200, 100), here(), start, Direction::East)
            .unwrap();
        assert_eq!(
            steering.press(Direction::NorthWest, at(start, 50), Direction::East),
            Some(Facing::walking(Direction::NorthWest))
        );
        assert_eq!(steering.goal(), None);
        assert_eq!(
            steering.due(at(start, 450), here(), Direction::NorthWest),
            Some(Facing::walking(Direction::NorthWest)),
            "the keyboard's step, not the destination's"
        );
    }

    /// Losing focus lets go of everything, keyboard and destination alike.
    #[test]
    fn losing_focus_stops_the_walk() {
        let start = Instant::now();
        let mut steering = Steering::default();

        steering.press(Direction::South, start, Direction::South).unwrap();
        steering.go_to((200, 200), here(), start, Direction::South);
        steering.clear();
        assert_eq!(steering.goal(), None);
        assert_eq!(steering.deadline(), None);
        assert_eq!(steering.due(at(start, 10_000), here(), Direction::South), None);
    }
}
