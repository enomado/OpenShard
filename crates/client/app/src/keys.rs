//! Which way the keyboard is asking to walk, and how often it may ask.
//!
//! # Why a key is held rather than pressed
//!
//! A step used to be sent from the key event itself, which made the *operating
//! system's* auto-repeat the walking speed: nothing for half a second, then
//! thirty steps a second. Neither is a pace a mobile has. The fast half is worse
//! than it looks — `WalkPace` refuses a sustained flood as a speedhack, so the
//! shard answers with a `0x21` and the body is pulled back to where it really
//! is, which reads as the walk stuttering rather than as the client asking for
//! too much.
//!
//! So the key is *state* and the clock is ours: a direction is held or it is
//! not, and while one is held a step is due every [`crowd::WALK_HOLD`] — or
//! every [`crowd::RUN_HOLD`] with shift down, which is what running is.
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
//! # The first press steps at once
//!
//! Waiting a whole step before the first one would put 400ms between the key and
//! the character. So a press that changes the direction is due immediately and
//! the timer is armed from there; a press of a key already down is the OS
//! repeating itself and is ignored.

use std::time::{Duration, Instant};

use openshard_protocol::direction::{Direction, Facing};
use winit::keyboard::KeyCode;

use crate::crowd::{RUN_HOLD, WALK_HOLD};

/// The arrow keys currently down, whether shift is, and when the next step is
/// due.
#[derive(Clone, Debug, Default)]
pub struct Held {
    /// Every arrow currently down, in the order they went down.
    ///
    /// A stack and not a single direction: pressing right without letting go of
    /// up walks east, and letting go of right again walks north-west rather than
    /// stopping. The last one pressed is the one obeyed, which is what every
    /// game does with a keyboard that reports two keys.
    down: Vec<Direction>,
    /// Whether either shift is down. The whole of "run" on this keyboard.
    running: bool,
    /// When the next step may be sent, or `None` when nothing is held.
    ///
    /// Absent means absent: no key is down, so there is no next step — not "not
    /// known yet".
    due: Option<Instant>,
}

impl Held {
    /// The direction an arrow key means, or `None` for any other key.
    ///
    /// The four arrows are the four diagonals of the map, which are the four
    /// *straight* directions on screen: UO's grid is turned 45 degrees, so "up"
    /// is north-west and there is no key that moves one axis alone.
    pub fn direction_of(code: KeyCode) -> Option<Direction> {
        match code {
            KeyCode::ArrowUp => Some(Direction::NorthWest),
            KeyCode::ArrowDown => Some(Direction::SouthEast),
            KeyCode::ArrowLeft => Some(Direction::SouthWest),
            KeyCode::ArrowRight => Some(Direction::NorthEast),
            _ => None,
        }
    }

    /// An arrow went down. Answers the step to send now, if any.
    pub fn press(&mut self, direction: Direction, now: Instant) -> Option<Facing> {
        if self.down.last() == Some(&direction) {
            // The key was already the one being obeyed: this is the OS
            // repeating it, and the repeat rate is not a walking speed.
            return None;
        }
        self.down.retain(|held| *held != direction);
        self.down.push(direction);
        Some(self.send(now))
    }

    /// An arrow came up.
    ///
    /// Releasing a key nobody is holding is not an error — a window that lost
    /// focus mid-step gets the release and never got the press.
    pub fn release(&mut self, direction: Direction) {
        self.down.retain(|held| *held != direction);
        if self.down.is_empty() {
            self.due = None;
        }
    }

    /// Shift went down or came up.
    ///
    /// Deliberately not re-timed: a walker that starts running mid-step keeps
    /// the deadline it already had, and the next one is a run's. Re-arming here
    /// would let a player tapping shift send a step per tap.
    pub fn set_running(&mut self, running: bool) {
        self.running = running;
    }

    /// Everything is up.
    ///
    /// The window losing focus is a key release that never arrives, and without
    /// this the character walks into the wall behind whatever the player alt-tabbed
    /// to.
    pub fn clear(&mut self) {
        self.down.clear();
        self.due = None;
    }

    /// The step due by now, if one is.
    ///
    /// Called from the wait loop, so it charges one step per call rather than
    /// catching up on a stall: a window that was minimised for a minute has not
    /// banked a hundred and fifty steps, and sending them would be the flood
    /// this module exists to prevent.
    pub fn due(&mut self, now: Instant) -> Option<Facing> {
        match self.due {
            Some(due) if now >= due => Some(self.send(now)),
            _ => None,
        }
    }

    /// When the next step is due, for the event loop's deadline.
    pub fn deadline(&self) -> Option<Instant> {
        self.due
    }

    /// How long a step takes at the pace being asked for.
    fn interval(&self) -> Duration {
        if self.running { RUN_HOLD } else { WALK_HOLD }
    }

    /// Take the step that is due: arm the next one and say what to send.
    ///
    /// Only called with something held, which is what makes the `unwrap`
    /// honest — `press` has just pushed, and `due` is `None` whenever the stack
    /// is empty.
    fn send(&mut self, now: Instant) -> Facing {
        let direction = *self.down.last().unwrap();
        self.due = Some(now + self.interval());
        if self.running {
            Facing::running(direction)
        } else {
            Facing::walking(direction)
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

    #[test]
    fn a_press_steps_at_once_and_then_at_the_walking_rate() {
        let start = Instant::now();
        let mut held = Held::default();

        assert_eq!(
            held.press(Direction::NorthWest, start),
            Some(Facing::walking(Direction::NorthWest))
        );
        // Nothing is due until a whole step has passed.
        assert_eq!(held.due(at(start, 399)), None);
        assert_eq!(
            held.due(at(start, 400)),
            Some(Facing::walking(Direction::NorthWest))
        );
        assert_eq!(held.due(at(start, 401)), None);
    }

    /// The defect this module was written for: the OS repeats a held key at its
    /// own rate, and that rate is not a walking speed.
    #[test]
    fn the_operating_systems_repeat_is_not_a_step() {
        let start = Instant::now();
        let mut held = Held::default();

        held.press(Direction::NorthWest, start).unwrap();
        for repeat in 1..30 {
            assert_eq!(held.press(Direction::NorthWest, at(start, repeat * 30)), None);
        }
    }

    #[test]
    fn shift_is_the_running_flag_and_halves_the_gap() {
        let start = Instant::now();
        let mut held = Held::default();
        held.set_running(true);

        assert_eq!(
            held.press(Direction::SouthEast, start),
            Some(Facing::running(Direction::SouthEast))
        );
        assert_eq!(held.due(at(start, 199)), None);
        assert_eq!(
            held.due(at(start, 200)),
            Some(Facing::running(Direction::SouthEast))
        );
    }

    /// Shift pressed mid-walk does not itself send anything: the deadline in
    /// flight is kept and the pace changes from the next step on.
    #[test]
    fn shift_mid_step_does_not_send_a_step_of_its_own() {
        let start = Instant::now();
        let mut held = Held::default();

        held.press(Direction::North, start).unwrap();
        held.set_running(true);
        assert_eq!(held.due(at(start, 200)), None, "the walk's deadline stands");
        assert_eq!(held.due(at(start, 400)), Some(Facing::running(Direction::North)));
        // And from there it is a runner's.
        assert_eq!(held.due(at(start, 600)), Some(Facing::running(Direction::North)));
    }

    #[test]
    fn the_last_key_pressed_is_the_one_obeyed_and_releasing_it_falls_back() {
        let start = Instant::now();
        let mut held = Held::default();

        held.press(Direction::NorthWest, start).unwrap();
        assert_eq!(
            held.press(Direction::NorthEast, at(start, 50)),
            Some(Facing::walking(Direction::NorthEast))
        );
        held.release(Direction::NorthEast);
        assert_eq!(
            held.due(at(start, 450)),
            Some(Facing::walking(Direction::NorthWest)),
            "the key still down keeps walking"
        );
    }

    #[test]
    fn nothing_held_is_nothing_due() {
        let start = Instant::now();
        let mut held = Held::default();

        held.press(Direction::West, start).unwrap();
        held.release(Direction::West);
        assert_eq!(held.deadline(), None);
        assert_eq!(held.due(at(start, 10_000)), None);
        // A release nobody pressed is not an error.
        held.release(Direction::West);
    }

    #[test]
    fn losing_focus_lets_go_of_everything() {
        let start = Instant::now();
        let mut held = Held::default();

        held.press(Direction::South, start).unwrap();
        held.clear();
        assert_eq!(held.due(at(start, 10_000)), None);
    }
}
