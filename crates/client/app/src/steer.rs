//! Where the player is asking to walk, and how often the ask may be sent.
//!
//! # Two idioms, one clock
//!
//! A held arrow says *which way*, and a click on the ground says *where to* —
//! the strategy game's idiom, and the one this client is asked for. They are the
//! same question a step at a time, so they are answered in one place: whichever
//! is asking, a step leaves every [`WALK_HOLD`] (or every
//! [`RUN_HOLD`] with shift down, which is what running is), and never two
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
//! # The first ask steps at once, and every other one waits its turn
//!
//! Waiting a whole step before the first one would put 400ms between the input
//! and the character. So a press that changes the direction, and a click that
//! names a new destination, are due immediately — *if the body is standing*. If
//! it is not, they are not.
//!
//! That second half is the queue rule, and it is the whole of this module's
//! contract with the player's eye:
//!
//! **An input joins the queue or rebuilds it. A step already begun ticks out.**
//!
//! An input never moves the deadline earlier. It changes which way the step the
//! walk already owes will go — [`Steering::take`] reads the keys at the moment
//! the step leaves and not at the moment they were pressed, so the queue is one
//! deep and is rebuilt by every press for free — and that step leaves when it
//! was always going to. The reasons are three, and each of them was a complaint:
//!
//! - **The picture.** The body is drawn crossing the tile its last step asked
//!   for, and `crowd.rs` starts each glide from the tile the previous step
//!   *ended* on. A step issued halfway through the last one therefore yanks the
//!   body to a tile it has not reached, which is half a tile in one frame — and
//!   the camera is locked to the body, so the whole world jumps with it. Pressing
//!   the opposite arrow mid-stride is how a player finds this in one second.
//! - **The pace.** The shard's `WalkPace` refuses a body that asks for steps
//!   faster than a body walks, and answers with a `0x21` that puts it back where
//!   it really is. A client that sent a step per keypress hands a key-masher a
//!   burst of steps, a rollback, and a body that flies off and is dragged back.
//! - **The wire.** The rollback races the steps still in flight, and their acks
//!   arrive for a sequence this end has already forgotten — which
//!   `client/net`'s [`Walk`](openshard_client_net::walk::Walk) reports as a
//!   desync it cannot repair. Not asking for those steps is what stops it.
//!
//! The rate floor is a floor and not a lockout: it *outlives the release*, so
//! letting go of the arrow and pressing it again does not buy a step, and it is
//! only ever a floor, so a walk that genuinely stopped sets off the instant the
//! arrow next goes down.
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

use openshard_movement::{RUN_HOLD, WALK_HOLD, direction_toward};
use openshard_protocol::direction::{Direction, Facing};
use openshard_protocol::world::Point;

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
    /// The earliest the next step may leave: the deadline of the step in flight.
    ///
    /// The rate floor, and the queue rule's whole mechanism. Armed by every step
    /// that costs time and cleared by *nothing* — not a release, not a lost
    /// focus, not a new destination — because everything that clears it is a way
    /// for a player to step over it. `None` only before the first step of a
    /// session.
    ///
    /// It is not a "the walk is running" flag, which is what it used to be:
    /// whether anything is being asked for is [`Steering::asking_for_anything`],
    /// and that is what decides whether the event loop is woken for it.
    due: Option<Instant>,
    /// Where the body stood when the last step was sent, for [`STUCK_STEPS`].
    was: Option<Point>,
    /// How many steps in a row have left it there.
    stalled: u8,
    /// Whether [`Steering::due`] is the deadline of a walk still under way,
    /// rather than one that has since stopped.
    ///
    /// The two are the same instant and mean opposite things to the cadence. A
    /// deadline a step was taken at is what the *next* one is measured from, so
    /// that a late wake does not push the whole walk back (see
    /// [`Steering::next_due`]). A deadline that came and went with the arrows up
    /// is not a cadence at all — the player pressed again some time later, and
    /// measuring from it would make the step after that one due a fraction of a
    /// hold away, which cuts the glide short and jumps the body.
    walking: bool,
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
    ///
    /// `None` is the ordinary answer mid-walk and not a refusal — the press has
    /// already changed which way the step the walk owes will go, and that step
    /// leaves at its own deadline. See the queue rule in the module docs.
    pub fn press(&mut self, direction: Direction, now: Instant, facing: Direction) -> Option<Facing> {
        if !self.keys.press(direction) {
            // The operating system repeating a key that is already the one being
            // obeyed. Its rate is not a walking speed.
            return None;
        }
        self.goal = None;
        self.stalled = 0;
        self.was = None;
        if !self.free(now) {
            return None;
        }
        let asking = self.keys.asking();
        self.charge(asking, now, facing);
        asking
    }

    /// An arrow came up.
    ///
    /// The rate floor stays armed: a player who lets go of an arrow and presses
    /// it again 60ms later has not earned a step, and a release that disarmed the
    /// clock is exactly how the old cadence was stepped over.
    pub fn release(&mut self, direction: Direction) {
        self.keys.release(direction);
        if self.keys.asking().is_none() && self.goal.is_none() {
            self.stand();
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
        // Only when the step in flight has run its course — otherwise a drag
        // across the ground would send a step per mouse event, and a click
        // mid-stride would cut the stride short. The same queue rule the
        // keyboard obeys: the destination is rebuilt now and walked toward at
        // the next deadline.
        if !self.free(now) {
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
        self.stand();
    }

    /// The server put the body somewhere this end did not walk it to.
    ///
    /// A `0x21` refusing a step, or a `0x20` moving the body — `link::Body`'s
    /// `corrected`. What it invalidates here is [`Steering::asked`]: the facing
    /// this walk believed it had asked for is void, and the next step decided
    /// against it would be mis-timed as a turn or, worse, mis-timed as not one
    /// and sent a hold late. The server's word replaces it.
    ///
    /// The rate floor is deliberately left alone. A rollback is not a step and
    /// does not buy one, and a client that re-armed its clock on every refusal
    /// would walk faster into a wall than away from one.
    pub fn corrected(&mut self, facing: Direction) {
        self.asked = Some(facing);
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
        match self.free(now) {
            true => self.take(from, now, facing),
            false => None,
        }
    }

    /// When the next step is due, for the event loop's deadline.
    ///
    /// Only while something is asking for one. The floor outlives the asking
    /// (see [`Steering::due`]'s field) and a loop woken by a floor nobody is
    /// waiting on would wake for a step it then declines to take, over and over.
    pub fn deadline(&self) -> Option<Instant> {
        match self.asking_for_anything() {
            true => self.due,
            false => None,
        }
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
            // standing where it was sent. Nothing is woken for it any more —
            // `deadline` sees that nothing is asking — and the event loop goes
            // back to sleeping on the animation.
            None => {
                self.stand();
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
            self.stand();
            return;
        };
        // What the body will be facing, which is a step ahead of what the caller
        // can see: our own last ask has not been round the shard thread yet. A
        // rollback is what makes this stale, and `corrected` is where it is put
        // right.
        let facing = self.asked.unwrap_or(facing);
        self.asked = Some(step.direction);
        if step.direction == facing {
            // Read before the walk is declared under way: what `next_due` needs
            // to know is whether the deadline it is chaining from belongs to a
            // walk that was still going.
            let due = self.next_due(now);
            self.walking = true;
            self.due = Some(due);
            return;
        }
        self.walking = true;
        // A turn, and the step it precedes leaves in the same wake. So the clock
        // is left exactly where it was and the *step* is what charges it: the
        // pair is one ask against the rate floor, which is what stops a player
        // spinning through the arrows from buying a step per press.
        //
        // Where it was is either a deadline that has just passed — charging from
        // `now` instead would fold this wake's lateness into the cadence, which
        // is the drift `next_due` exists to refuse — or nothing at all, which is
        // the first ask of a walk and is due this instant.
        self.due = Some(self.due.unwrap_or(now));
    }

    /// Whether the step in flight has run its course, so that the next one may
    /// leave.
    ///
    /// The queue rule, in one line. Everything that asks for a step goes through
    /// here, and nothing anywhere clears [`Steering::due`] — so this is false for
    /// exactly as long as a step is being walked, however the asking arrived.
    fn free(&self, now: Instant) -> bool {
        self.due.is_none_or(|due| now >= due)
    }

    /// Whether any input is asking to walk at all.
    fn asking_for_anything(&self) -> bool {
        self.keys.asking().is_some() || self.goal.is_some()
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
            Some(due) if self.walking && now < due + interval => due + interval,
            _ => now + interval,
        }
    }

    /// Nothing is being asked for any more: forget what the destination's
    /// patience was measuring.
    ///
    /// Deliberately not a reset. [`Steering::due`] stays — it is the rate floor
    /// and a walk that ended does not refund it — and so does
    /// [`Steering::asked`], which is still the truth about which way the body
    /// was last sent. Only a rollback makes that false, and only
    /// [`Steering::corrected`] says so.
    fn stand(&mut self) {
        self.was = None;
        self.stalled = 0;
        self.walking = false;
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
    /// where they clicked — and it takes effect at the next step, not at the
    /// press. The step already under way is the destination's last.
    #[test]
    fn the_keyboard_takes_over_from_a_destination() {
        let start = Instant::now();
        let mut steering = Steering::default();

        steering
            .go_to((200, 100), here(), start, Direction::East)
            .unwrap();
        assert_eq!(
            steering.press(Direction::NorthWest, at(start, 50), Direction::East),
            None,
            "the step under way is not cut short"
        );
        assert_eq!(steering.goal(), None, "but the destination is dropped at once");
        assert_eq!(steering.due(at(start, 399), here(), Direction::East), None);
        assert_eq!(
            steering.due(at(start, 400), here(), Direction::NorthWest),
            Some(Facing::walking(Direction::NorthWest)),
            "the keyboard's step, not the destination's"
        );
    }

    /// The queue rule, stated on its own: a press mid-step changes which way the
    /// step already owed will go, and moves nothing else.
    ///
    /// The complaint it comes from is a jump — `crowd.rs` glides from the tile
    /// the last step ended on, so a step issued half a hold early yanks the body
    /// to a tile it has not reached and takes the camera with it. The DST in
    /// `dst.rs` is what holds the picture to that; this is the rule at the unit.
    #[test]
    fn a_press_mid_step_waits_for_the_step_to_tick_out() {
        let start = Instant::now();
        let mut steering = Steering::default();

        assert_eq!(
            steering.press(Direction::East, start, Direction::East),
            Some(Facing::walking(Direction::East))
        );
        // Halfway across the tile, the player asks for the opposite direction.
        assert_eq!(
            steering.press(Direction::West, at(start, 200), Direction::East),
            None
        );
        assert_eq!(
            steering.deadline(),
            Some(at(start, 400)),
            "the deadline the step already had, not an earlier one"
        );
        // And it is the *new* direction that leaves at it: the queue is one step
        // deep and every press rebuilds it.
        assert_eq!(
            steering.due(at(start, 400), here(), Direction::East),
            Some(Facing::walking(Direction::West))
        );
    }

    /// The mash: pressing every direction in turn, faster than a body walks,
    /// buys exactly one step per hold.
    ///
    /// A turn used to be the way through the floor — it costs the shard nothing,
    /// so it was sent the instant it was asked for and the step behind it went
    /// with it. Spinning through four arrows was therefore four steps in one
    /// frame, which the shard's `WalkPace` answers with a `0x21` and a body
    /// dragged back to where it really is.
    #[test]
    fn spinning_through_the_arrows_does_not_buy_a_step_each() {
        let start = Instant::now();
        let mut steering = Steering::default();
        let arrows = [
            Direction::East,
            Direction::South,
            Direction::West,
            Direction::North,
        ];

        steering.press(arrows[0], start, Direction::East).unwrap();
        for (tick, direction) in arrows.iter().cycle().take(30).enumerate() {
            let now = at(start, 10 * tick as u64 + 10);
            assert_eq!(
                steering.press(*direction, now, Direction::East),
                None,
                "at {now:?}"
            );
            assert_eq!(
                steering.due(now, here(), Direction::East),
                None,
                "nor by asking the clock at {now:?}"
            );
        }
        assert_eq!(steering.deadline(), Some(at(start, 400)));
    }

    /// Letting go of the arrow does not refund the step being walked: a tapped
    /// key is a held key as far as the cadence is concerned.
    ///
    /// This is the other half of the mash. The clock used to be disarmed on the
    /// last release, so press-release-press was read as the first ask of a fresh
    /// walk and stepped at once — a step per tap, at whatever rate a finger can
    /// manage.
    #[test]
    fn a_release_does_not_refund_the_step_in_flight() {
        let start = Instant::now();
        let mut steering = Steering::default();

        steering.press(Direction::East, start, Direction::East).unwrap();
        for tap in 1..8 {
            let now = at(start, 40 * tap);
            steering.release(Direction::East);
            assert_eq!(
                steering.press(Direction::East, now, Direction::East),
                None,
                "tap {tap} bought a step"
            );
        }
        // And the floor is a floor: a walk that has genuinely stopped sets off on
        // the next press, in that instant.
        steering.release(Direction::East);
        assert_eq!(
            steering.press(Direction::East, at(start, 2_000), Direction::East),
            Some(Facing::walking(Direction::East))
        );
        assert_eq!(
            steering.deadline(),
            Some(at(start, 2_400)),
            "and the step after it is a whole hold away, measured from the press"
        );
    }

    /// A rollback makes the facing this end believed it had asked for a lie, and
    /// the shard's word replaces it. Without that, the step after a `0x21` is
    /// decided against a direction nobody is facing: it is timed as a turn when
    /// it is a step, or as a step when it is a turn.
    #[test]
    fn a_correction_replaces_the_facing_this_end_believed_in() {
        let start = Instant::now();
        let mut steering = Steering::default();

        steering.press(Direction::East, start, Direction::East).unwrap();
        // The shard refuses it and says the body is still facing north.
        steering.corrected(Direction::North);
        // Facing north, asking east: a turn, so the step it precedes is due in
        // the same wake rather than a hold after it.
        assert_eq!(
            steering.due(at(start, 400), here(), Direction::North),
            Some(Facing::walking(Direction::East))
        );
        assert_eq!(
            steering.due(at(start, 400), here(), Direction::North),
            Some(Facing::walking(Direction::East)),
            "the step the turn was for, in the same wake"
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
