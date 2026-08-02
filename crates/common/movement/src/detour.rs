//! Getting past what is directly in the way, without planning a route.
//!
//! # What this is, and what `find_path` is
//!
//! A body walking a *heading* — an arrow key, a held mouse direction, a
//! creature told to close on something — has no route to fall back on when the
//! tile ahead is shut. `find_path` answers a different question, and answers it
//! at a different price: it needs a destination, and it searches. A heading has
//! neither. What it has is the ground it is standing next to, and the answer to
//! "can I still move at all" is in that ground alone.
//!
//! So this is not a planner and it is not a fallback for one. It is the rule a
//! body uses to brush past furniture: try the way you were going; failing that,
//! the nearest way round; failing that, do not move.
//!
//! # Four tiles decide it
//!
//! The scene is [`Around`], and it is exactly four tiles: the one being stood
//! on, the one the intent points at, and the two the body could slide onto
//! instead. Not the whole eight-neighbourhood, and not by accident — which two
//! flanks are candidates is fixed by the intent, and the other three neighbours
//! cannot change the answer:
//!
//! - **The intent is a cardinal** — a wall dead ahead. There is no diagonal
//!   past it *at all*: a diagonal step may not cut the corner where two
//!   blockers meet (see [`step_allowed`]), both cardinals flanking it must be
//!   open, and the blocked intent is unconditionally one of those two for
//!   either diagonal beside it. So neither diagonal can ever pass, and the
//!   candidates are the two cardinals at ninety degrees — a step along the
//!   wall's face, which is what a body hugging a wall actually does.
//! - **The intent is a diagonal** — a corner, not a wall. The two cardinals it
//!   splits into have no corner of their own to cut, so those are the
//!   candidates.
//!
//! Either way: two candidates, one intended tile, and where you stand. Four.
//! That is the whole input, which is why [`Around::new`] can state a scene
//! outright and every case of this can be enumerated rather than sampled.
//!
//! # Three states: walking, sliding, standing
//!
//! [`Detour`] is the machine, and its states are what a body is doing about
//! what is in front of it. Two of them are moving — freely, or along the face
//! of something — and the third is not moving at all, which is a real thing a
//! body does and not an error. See [`Detour::Standing`] for why it is a state
//! rather than only an answer.
//!
//! The memory in [`Detour::Sliding`] — which flank got past the last obstacle —
//! exists because the tie-break between two open flanks has to be *stable
//! across tiles*, not merely deterministic. A fixed order — always the
//! clockwise one first — is deterministic and still loops: at a doorway or a
//! building corner the two flanks alternate which one is open from one tile to
//! the next, so tile A sends the body to tile B by its only open flank and tile
//! B sends it back to A by its only open flank, forever. A live corner did
//! exactly that for a second and a half before breaking out by chance.
//!
//! Remembering the flank that worked and preferring it again breaks the cycle
//! the moment either tile stops *requiring* the other one specifically — the
//! common case, since the two only disagree at a real pinch point. The memory
//! is dropped as soon as the intent stops being blocked, so an obstacle met
//! later is never biased by an unrelated one met before.

use openshard_protocol::direction::Direction;
use openshard_protocol::world::Point;

use crate::walk::{Terrain, step_allowed};

/// What is open around a body, as far as one step in one intended direction can
/// tell: the intended tile and the two flanks that could take its place.
///
/// See the module docs for why those two flanks, and why nothing else about the
/// neighbourhood is here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Around {
    /// The direction being asked for.
    intent: Direction,
    /// Whether the tile [`Around::intent`] points at can be stepped onto.
    ahead: bool,
    /// The two directions the body could slide onto instead, clockwise of the
    /// intent first, and whether each is open. Which two they are is
    /// [`flanks`]'s answer, and it depends on the intent.
    flanks: [(Direction, bool); 2],
}

impl Around {
    /// Read the four tiles from the world.
    ///
    /// [`step_allowed`] and not [`Terrain::can_step`], for every one of them: a
    /// terrain answers for the destination tile alone, and a diagonal that cuts
    /// a wall's corner is refused on top of that. Asking the terrain directly
    /// here is what once had a client believing a corner-cutting diagonal was
    /// open, sending it, and being rolled back for as long as the player held
    /// the key.
    #[must_use]
    pub fn read(terrain: &dyn Terrain, from: Point, intent: Direction) -> Self {
        let open = |direction| step_allowed(terrain, from, direction).is_some();
        Self {
            intent,
            ahead: open(intent),
            flanks: flanks(intent).map(|flank| (flank, open(flank))),
        }
    }

    /// The same scene stated outright, for a caller that already knows what is
    /// around it — and for enumerating every scene there is, which is what
    /// makes this rule testable exhaustively rather than at a handful of
    /// hand-drawn walls.
    ///
    /// `clockwise` and `counter` are the two flanks in the order [`flanks`]
    /// gives them; which directions those are is the intent's business, not the
    /// caller's.
    #[must_use]
    pub const fn new(intent: Direction, ahead: bool, clockwise: bool, counter: bool) -> Self {
        let [cw, ccw] = flanks(intent);
        Self {
            intent,
            ahead,
            flanks: [(cw, clockwise), (ccw, counter)],
        }
    }
}

/// Where a body actually goes, given where it wanted to go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing was in the way. The direction is the intent itself.
    Ahead(Direction),
    /// The intent was blocked and this flank was not: one step to the side,
    /// along the face of whatever is in the way.
    Aside(Direction),
    /// Neither the intent nor either flank is open — the inside corner of a
    /// building, with the body pushed at the corner. There is no step.
    ///
    /// Which is not the same as "ask anyway and let the server sort it out". A
    /// step the caller has already proven will be refused is answered with a
    /// rollback, and a rollback a hold is a body shuddering in a corner rather
    /// than standing in one. What a caller may still do with this is *turn*:
    /// turning costs no ground and no shard refuses it.
    Stuck,
}

/// How a body is getting along with what is in front of it: walking freely,
/// sliding along something, or standing because there is nowhere to go.
///
/// Three states, and the third is not bookkeeping. **Not moving is one of the
/// things a body does**, and it is a different thing from moving freely — a
/// machine that says `Clear` for both is telling the caller that nothing was in
/// the way while the body is wedged in the corner of a building. It was written
/// that way first, and every question worth asking of it afterwards ("is this
/// walk getting anywhere", "why was nothing sent") had to be answered by
/// re-deriving the scene, because the state had thrown the answer away.
///
/// The transitions are decided entirely by the scene handed to
/// [`Detour::step`]; see the module docs for why the memory in
/// [`Detour::Sliding`] is here at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Detour {
    /// Walking freely: nothing was in the way the last time this was asked, so
    /// nothing is owed to what got past it.
    #[default]
    Clear,
    /// Sliding along something, on the flank that got past it — preferred again
    /// while the obstacle lasts.
    Sliding(Direction),
    /// Standing: the intent is blocked and so is every flank of it. The inside
    /// corner of a building, with the body pushed at the corner.
    ///
    /// A state and not just an answer, because it *persists* — the player is
    /// still leaning on the key and every beat asks again — and because what a
    /// caller does about it differs from what it does about a step. It is left
    /// the moment the scene offers anything, which is what makes a walk pick
    /// itself back up when a door opens with no fresh input.
    Standing,
}

impl Detour {
    /// Where to actually step, given what is around and what was wanted.
    ///
    /// The transitions, in full. An open intent goes to [`Detour::Clear`] —
    /// whatever was being slid along is behind the body now, and biasing the
    /// next obstacle by it would be memory of the wrong thing. A blocked intent
    /// with an open flank goes to [`Detour::Sliding`] on that flank, preferring
    /// the one already committed to when it is still a candidate. Nothing open
    /// at all goes to [`Detour::Standing`]: there is no slide to remember, and
    /// no pretending there was nothing in the way either.
    pub fn step(&mut self, around: &Around) -> Step {
        if around.ahead {
            *self = Self::Clear;
            return Step::Ahead(around.intent);
        }
        let ordered = match *self {
            Self::Sliding(preferred) if preferred == around.flanks[1].0 => {
                [around.flanks[1], around.flanks[0]]
            }
            _ => around.flanks,
        };
        for (direction, open) in ordered {
            if open {
                *self = Self::Sliding(direction);
                return Step::Aside(direction);
            }
        }
        *self = Self::Standing;
        Step::Stuck
    }

    /// The walk stopped — the key came up, the window lost focus. Forget both
    /// the flank and the corner, so a heading picked up later somewhere else
    /// starts from the fixed order and from no assumption about being stuck.
    pub fn forget(&mut self) {
        *self = Self::Clear;
    }
}

/// The two directions a blocked `intent` may be answered with, clockwise first.
///
/// A cardinal's are the cardinals at ninety degrees and a diagonal's are the
/// two cardinals it splits into — never a diagonal in either case. The module
/// docs have the argument; the short of it is that there is no diagonal past a
/// wall dead ahead, because the blocked tile is itself a flank of both
/// diagonals beside it and a diagonal needs both of its flanks open.
const fn flanks(intent: Direction) -> [Direction; 2] {
    let bits = intent.to_bits();
    let turn = match intent.is_diagonal() {
        true => 1,
        false => 2,
    };
    [
        Direction::from_bits(bits + turn),
        Direction::from_bits(bits + 8 - turn),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every state the machine can be in, for the enumerations below: the two
    /// slides are both flanks, because which one is remembered is exactly what
    /// the tie-break turns on.
    fn every_state(intent: Direction) -> [Detour; 4] {
        let [cw, ccw] = flanks(intent);
        [
            Detour::Clear,
            Detour::Sliding(cw),
            Detour::Sliding(ccw),
            Detour::Standing,
        ]
    }

    /// Every scene there is, at every intent, from every state the machine can
    /// be in: 8 directions x 8 open/shut combinations x 4 states. The claim is
    /// the one that matters on the wire — **what comes back is never a
    /// direction the scene says is shut** — and it is checked by enumeration
    /// rather than by drawing walls and hoping the interesting one was drawn.
    #[test]
    fn no_scene_at_any_intent_is_ever_answered_with_a_shut_direction() {
        for &intent in &Direction::ALL {
            let [cw, ccw] = flanks(intent);
            for scene in 0..8u8 {
                let (ahead, clockwise, counter) = (scene & 1 != 0, scene & 2 != 0, scene & 4 != 0);
                let around = Around::new(intent, ahead, clockwise, counter);
                for mut detour in every_state(intent) {
                    let open = |direction| match direction {
                        d if d == intent => ahead,
                        d if d == cw => clockwise,
                        _ => counter,
                    };
                    match detour.step(&around) {
                        Step::Ahead(direction) => {
                            assert_eq!(direction, intent, "{intent:?}/{scene}: not the intent");
                            assert!(ahead, "{intent:?}/{scene}: walked into a shut tile");
                        }
                        Step::Aside(direction) => {
                            assert!(
                                direction == cw || direction == ccw,
                                "{intent:?}/{scene}: {direction:?} is not a flank of the intent"
                            );
                            assert!(open(direction), "{intent:?}/{scene}: slid onto a shut tile");
                            assert!(!ahead, "{intent:?}/{scene}: slid aside with the way open");
                        }
                        Step::Stuck => assert!(
                            !ahead && !clockwise && !counter,
                            "{intent:?}/{scene}: gave up with somewhere to go"
                        ),
                    }
                }
            }
        }
    }

    /// And the state it is left in says which of those three happened, from
    /// every state and at every scene. Not bookkeeping: a machine that
    /// answered `Stuck` and then called itself `Clear` was claiming nothing had
    /// been in the way of a body wedged in a corner — the one question the
    /// state exists to answer, answered wrong. Standing is a state a body is
    /// *in*, for as long as the player leans on the key.
    #[test]
    fn the_state_left_behind_says_which_of_the_three_the_body_is_doing() {
        for &intent in &Direction::ALL {
            for scene in 0..8u8 {
                let (ahead, clockwise, counter) = (scene & 1 != 0, scene & 2 != 0, scene & 4 != 0);
                let around = Around::new(intent, ahead, clockwise, counter);
                for mut detour in every_state(intent) {
                    let was = detour;
                    let step = detour.step(&around);
                    let expected = match step {
                        Step::Ahead(_) => Detour::Clear,
                        Step::Aside(direction) => Detour::Sliding(direction),
                        Step::Stuck => Detour::Standing,
                    };
                    assert_eq!(
                        detour, expected,
                        "{intent:?}/{scene} from {was:?}: answered {step:?} and calls itself {detour:?}"
                    );
                }
            }
        }
    }

    /// A cardinal intent is never answered with a diagonal, whatever the scene.
    /// This is the one that is not merely "a legal tile": the tile beyond a
    /// wall's corner can be perfectly good ground, and the step onto it is
    /// still refused because of the corner it cuts. A rule that read the tiles
    /// alone would offer it.
    #[test]
    fn a_wall_dead_ahead_is_never_answered_with_a_diagonal() {
        for intent in Direction::ALL.iter().filter(|d| !d.is_diagonal()) {
            for flank in flanks(*intent) {
                assert!(
                    !flank.is_diagonal(),
                    "{intent:?} offered {flank:?}, which would cut the corner it is walled against"
                );
            }
        }
        // And the diagonal case is the mirror image: its flanks are the two
        // cardinals it splits into, which have no corner of their own to cut.
        for intent in Direction::ALL.iter().filter(|d| d.is_diagonal()) {
            for flank in flanks(*intent) {
                assert!(!flank.is_diagonal(), "{intent:?} offered another diagonal");
            }
        }
    }

    /// The doorway, as a scene rather than as a map: the same intent blocked at
    /// two tiles in a row, each of which opens the *other* flank. A fixed order
    /// takes the clockwise one at the first and the clockwise one at the second
    /// — which walks straight back — and repeats forever. The memory takes the
    /// flank that worked and keeps taking it.
    #[test]
    fn a_pinch_point_that_alternates_its_flanks_does_not_flip_flop() {
        let intent = Direction::East;
        let [cw, ccw] = flanks(intent);
        let mut detour = Detour::default();

        // First tile: only the counter-clockwise flank is open, so that is
        // forced whatever the tie-break would have preferred.
        assert_eq!(
            detour.step(&Around::new(intent, false, false, true)),
            Step::Aside(ccw)
        );
        assert_eq!(detour, Detour::Sliding(ccw));
        // Second tile: *both* flanks are open. The fixed order would take the
        // clockwise one, which is the way back.
        for tile in 0..4 {
            assert_eq!(
                detour.step(&Around::new(intent, false, true, true)),
                Step::Aside(ccw),
                "tile {tile}: the flank that worked is preferred over the way back"
            );
        }
        // The way ahead opens: the slide is over and nothing is owed to it.
        assert_eq!(
            detour.step(&Around::new(intent, true, true, true)),
            Step::Ahead(intent)
        );
        assert_eq!(
            detour,
            Detour::Clear,
            "an unrelated obstacle is not biased by this one"
        );
        assert_ne!(cw, ccw);
    }

    /// Boxed in: nothing to send, the flank that is no longer working is not
    /// kept — and the body is *standing*, which is what it stays until the
    /// scene offers something, however long the player leans on the key.
    #[test]
    fn nothing_open_is_stuck_and_stands_there() {
        let corner = Around::new(Direction::East, false, false, false);
        let mut detour = Detour::Sliding(Direction::North);

        assert_eq!(detour.step(&corner), Step::Stuck);
        assert_eq!(detour, Detour::Standing);
        for beat in 0..10 {
            assert_eq!(detour.step(&corner), Step::Stuck, "beat {beat}");
            assert_eq!(detour, Detour::Standing, "beat {beat}: still nowhere to go");
        }
        // A door opens somewhere in front of it, with nothing else asked for:
        // the walk resumes and the standing is over.
        assert_eq!(
            detour.step(&Around::new(Direction::East, true, false, false)),
            Step::Ahead(Direction::East)
        );
        assert_eq!(detour, Detour::Clear, "the standing is over");
    }

    /// Letting go is not the same as being stuck: a heading picked up later,
    /// somewhere else, must not start out believing it is in a corner.
    #[test]
    fn forgetting_leaves_the_corner_behind() {
        let mut detour = Detour::Standing;
        detour.forget();
        assert_eq!(detour, Detour::Clear);
        assert_eq!(detour, Detour::Clear, "the standing is over");
    }

    /// The scene read from a world agrees with the scene stated outright — the
    /// two constructors are one rule, or the exhaustive test above proves
    /// something about a fiction.
    #[test]
    fn a_scene_read_from_the_world_is_a_scene_stated_outright() {
        use crate::walk::OpenWorld;

        // East is walled, and so is the tile north of the body — which is the
        // counter-clockwise flank of East. South, the clockwise one, is open.
        struct Corner;
        impl Terrain for Corner {
            fn can_step(&self, from: Point, to: Point) -> Option<Point> {
                match (to.x, to.y) {
                    (101, 100) | (100, 99) => None,
                    _ => OpenWorld.can_step(from, to),
                }
            }
        }

        let from = Point::new(100, 100, 0);
        assert_eq!(
            Around::read(&Corner, from, Direction::East),
            Around::new(Direction::East, false, true, false)
        );
        // And a diagonal whose tile is open ground but whose corner is cut:
        // read must refuse it, which `Terrain::can_step` alone would not.
        assert!(Corner.can_step(from, Point::new(101, 101, 0)).is_some());
        assert_eq!(
            Around::read(&Corner, from, Direction::SouthEast),
            Around::new(Direction::SouthEast, false, true, false)
        );
    }
}
