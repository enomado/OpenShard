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
use openshard_client_render::follow::Gaze;
use openshard_client_render::mobiles::{Glide, Mobile};
use openshard_movement::{RUN_HOLD, WALK_HOLD};
use openshard_protocol::direction::{Direction, Facing};
use openshard_protocol::serial::Serial;
use openshard_protocol::speech::Font;
use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::Point;
use openshard_uofiles::anim::BodyKind;

/// How long a spoken line stays drawn above its speaker's head, once heard.
///
/// A fixed hold rather than one scaled by the message's length — which is what
/// `Mobile.m_SpeechTime` does in the reference client — because nothing that
/// reaches here carries an expiry: the wire's `0x1C` has none, and neither does
/// [`openshard_protocol::speech::SpokenMessage`]. Long enough to read a short
/// line before it is gone, and this is the one place to widen it once a real
/// expiry exists.
pub const SPEECH_HOLD: Duration = Duration::from_secs(5);

/// One line, and when [`Crowd::hear`] recorded it.
#[derive(Clone, Debug)]
struct Speech {
    text: String,
    font: Font,
    hue: Hue,
    started: Duration,
}

/// A step in progress: where it came from, when it started, and how long it
/// takes.
#[derive(Clone, Copy, Debug)]
struct Step {
    /// Where the body was drawn when the step began, or `None` when the move was
    /// not a step at all.
    ///
    /// Absent for a jump of more than one tile — a gate, a recall, a `0x22`
    /// putting a mispredicted body back. Interpolating one of those slides the
    /// character across the map over the length of a step, which is a far
    /// stranger picture than the teleport it is hiding.
    ///
    /// The drawn position and not the tile stepped off: see
    /// [`openshard_client_render::mobiles::Glide::from`], which this becomes.
    /// The two are the same number when the previous step ended exactly as this
    /// one began, and every millisecond they differ by is a jump on screen.
    from: Option<Gaze>,
    /// When it started, on [`Crowd::now`]'s clock: the instant it was heard.
    ///
    /// What is *not* read off the arrival is when it ends — for the body this
    /// client commands, that comes from the cadence. See [`crossing`].
    started: Duration,
    /// How long it takes — see [`glide_time`]. Never zero, which is what lets
    /// [`Tracked::glide`] divide.
    takes: Duration,
}

/// How long to spend crossing a tile, given the pace the wire claims and how
/// long ago the previous step was heard.
///
/// # Why the nominal length is not enough
///
/// A glide has to *end* exactly when the next step begins, or the walk is not
/// continuous: finish early and the body stands on its tile until the next
/// packet, finish late and the next step yanks it forward from wherever it had
/// got to. Both read as a stutter once a tile, which is the whole complaint.
///
/// [`WALK_HOLD`] is how long a step takes in theory, and it is the right answer
/// only for the first one. After that the *observed* gap between two steps is a
/// far better prediction of the gap to the next: it already contains the round
/// trip, the shard's own tick granularity, and — for everyone who is not this
/// client — whatever pace that creature actually walks at, which for an NPC is
/// nothing this end can look up. So the second step onwards is glided over the
/// measured gap.
///
/// Believed only within half and double the nominal length. Outside that band
/// the measurement is not a pace at all: a gap of four seconds is a body that
/// had stopped and started again, and a gap of nothing is two steps arriving in
/// one packet burst. Both are answered with the wire's own claim, which is at
/// least a walking speed.
fn glide_time(nominal: Duration, since: Option<Duration>) -> Duration {
    match since {
        Some(gap) if gap >= nominal / 2 && gap <= nominal * 2 => gap,
        // Nothing worth measuring: a body that was standing, one heard from for
        // the first time, or a gap that is not a pace.
        _ => nominal,
    }
}

/// How long a body keeps *playing* its walk after a step, given how long that
/// step takes to cross its tile.
///
/// Half a step longer than the crossing. The two used to be one number, and that
/// is a flicker: the hold expires the instant the body lands, so any latency at
/// all leaves a frame or two of standing between two steps — and standing is a
/// different animation group, so the walk's clock is restarted at frame zero
/// every single tile. Half a step of slack costs a body that has genuinely
/// stopped 200ms of walking on the spot, which nobody notices, and it is what
/// the reference client's own `m_AnimationInterval` slack is for.
///
/// What the slack must *not* do is play. See [`Tracked::striding`].
fn animation_hold(takes: Duration) -> Duration {
    takes * 3 / 2
}

/// One mobile's history: where it was, what it is playing, and since when.
#[derive(Clone, Copy, Debug)]
struct Tracked {
    /// Where the last packet put it.
    at: Point,
    /// Which way it was last seen facing.
    ///
    /// Kept for one rule only: a facing change with no position change is a
    /// turn, and a turn is a step that covers no ground — see [`Crowd::see`].
    /// The run flag is not kept with it, because it belongs to the step being
    /// taken rather than to the body.
    facing: Direction,
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
    /// When the previous step was heard, kept after that step has finished.
    ///
    /// What the gap in [`glide_time`] is measured against, which is why it
    /// outlives [`Tracked::step`]: the pace a body is walking at is a property
    /// of the last two packets, not of the one still being drawn. `None` until a
    /// body has been seen to move at all.
    stepped_at: Option<Duration>,
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
    /// The last line each serial was heard saying, if it is still within
    /// [`SPEECH_HOLD`]. Separate from `tracked`: the system talks (`who` is
    /// `None` for it too, same as the offline placeholder) and a body nobody
    /// has otherwise seen can still say something the moment it is heard from.
    speech: HashMap<Who, Speech>,
    /// Real time since this crowd was built. Its own clock rather than an
    /// `Instant`, so every rule here can be tested by handing it durations.
    now: Duration,
    /// Whose steps this client *commands* rather than merely hears about.
    ///
    /// Our own body: the offline placeholder to begin with, and our serial from
    /// the moment a shard names us. The distinction is a pace: see
    /// [`glide_time`], and [`Crowd::commanding`] for why one measurement is
    /// right and the other is not.
    ///
    /// A [`Who`] and not an `Option<Who>`, which is to say there is no state
    /// where this client commands nobody. The placeholder's steps are always
    /// ours to issue — nothing else can move it — so `None` is not "not named
    /// yet", it is the body we walk before a shard has given it a serial. As an
    /// `Option<Who>` it had to be armed by hand, which meant the offline walk
    /// was measured through the event loop's wake jitter on every path that
    /// forgot to.
    commanded: Who,
}

impl Crowd {
    /// Name the body this client walks itself.
    ///
    /// Everybody else's pace has to be *measured*, because nothing on the wire
    /// says how fast a creature walks. Our own we already know: `steer.rs` sends
    /// one step every hold and `client/net` predicts it here the same
    /// millisecond, so the nominal length is not an estimate of that walk — it
    /// is the walk. Measuring it anyway feeds the event loop's own wake jitter
    /// into the crossing *length*, and consecutive gaps jitter in opposite
    /// directions (late, then early), so the estimate is worse than the number
    /// it replaced: the body arrives early, stands, and is yanked. The walk
    /// oracle in `dst.rs` is what put a number on it.
    ///
    /// Said when a shard names our body, and not before: a client without one is
    /// already commanding the offline placeholder — see the field.
    pub fn commanding(&mut self, who: Who) {
        self.commanded = who;
    }

    /// Move every clock forward, and stop whoever has finished their step.
    pub fn advance(&mut self, dt: Duration) {
        let was = self.now;
        self.now += dt;
        let now = self.now;
        for tracked in self.tracked.values_mut() {
            // Only the part of the span the body was actually covering ground
            // in — see [`Tracked::mid_stride`]. Not a whole-span test against
            // either end: a frame that straddles the moment the crossing ended
            // would either play the whole span or none of it, and at 400ms a
            // step the second one freezes the entire walk.
            tracked.clock.advance(tracked.striding(was, dt));
            // The *animation* outlives the crossing by half a step — see
            // `animation_hold`. The glide itself ends on time; this is only what
            // decides when the walk gives way to standing.
            if tracked
                .step
                .is_some_and(|step| now >= step.started + animation_hold(step.takes))
            {
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
        let commanded = self.commanded == who;
        let tracked = self.tracked.entry(who).or_insert(Tracked {
            at,
            facing: facing.direction,
            body: body.0,
            // A body first heard of is standing: it may well be mid-stride, but
            // the only thing that could say so is a previous packet and there
            // is none.
            group: kind.standing(),
            step: None,
            stepped_at: None,
            clock: AnimationClock::default(),
        });

        // A step is a *position* change. A turn on the spot is not one — the
        // client draws a turning body standing, and a facing change arrives for
        // every step too, so treating it as movement would keep everyone
        // walking forever.
        if tracked.at != at {
            let was = tracked.at;
            let nominal = if facing.running { RUN_HOLD } else { WALK_HOLD };
            // Two ways to know how long a tile takes, and which one is available
            // is exactly what [`Crowd::commanded`] answers.
            //
            // A body we only hear about is glided over the gap *measured* since
            // its last step — that already contains the round trip, the shard's
            // tick and whatever pace the creature actually walks at, none of
            // which this end can look up. See `glide_time`.
            //
            // The one we command has a cadence instead of a measurement: we sent
            // the step and we know when the next is owed, so what is chained is
            // the instant this crossing should *end*. See `crossing`.
            let takes = match commanded {
                true => crossing(tracked.step, commanded, now, nominal),
                false => {
                    let since = tracked.stepped_at.map(|heard| now.saturating_sub(heard));
                    glide_time(nominal, since)
                }
            };
            // Where the body is drawn *now*, which is where the new step has to
            // pick up from. Read before `at` is moved, because it is a question
            // about the step that is ending.
            let from = tracked.gaze_at(now);
            tracked.at = at;
            tracked.step = Some(Step {
                from: is_one_step(was, at).then_some(from),
                started: now,
                takes,
            });
            tracked.stepped_at = Some(now);
            let moving = match (facing.running, kind.running()) {
                (true, Some(running)) => running,
                // A running monster walks: `HighAnimationGroup` has no run, and
                // this is the client's answer too.
                _ => kind.walking(),
            };
            tracked.change_to(moving);
        } else if tracked.facing != facing.direction {
            // A turn is a whole step in UO: it covers no ground, and it costs
            // exactly as long as one that does. So it is not movement — the body
            // above is deliberately not walked — but it *is* a pace sample, and
            // recording it is what keeps the walk after a turn continuous.
            //
            // Found by the walk oracle in `dst.rs`: without this the gap the
            // next step measures spans the turn as well, two holds rather than
            // one, and `glide_time`'s band is just wide enough to believe it.
            // The tile after a turn was then crossed at half speed — half a tile
            // behind where the player had asked for the body to be — and the
            // step after it yanked the body forward to catch up.
            tracked.stepped_at = Some(now);
        }
        tracked.facing = facing.direction;
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

    /// Put a body somewhere without walking it there.
    ///
    /// What a rollback is: this client steps on its own prediction and the
    /// server may refuse — a `0x21`, or a `0x20` that moves the body somewhere it
    /// did not walk to. The tile it is put back on was never walked across, so
    /// gliding into it would draw the character strolling backwards; and the gap
    /// between the step and the refusal is not a pace, so the measurement
    /// [`glide_time`] makes is dropped along with it.
    ///
    /// The animation is deliberately left alone: a walker whose third step is
    /// refused is still walking, and restarting the group here would drop it to
    /// standing for the one frame between the refusal and the next step.
    pub fn snap(&mut self, who: Who, at: Point, body: Graphic, facing: Facing, hue: Hue) -> Mobile {
        let kind = BodyKind::of(body.0);
        let tracked = self.tracked.entry(who).or_insert(Tracked {
            at,
            facing: facing.direction,
            body: body.0,
            group: kind.standing(),
            step: None,
            stepped_at: None,
            clock: AnimationClock::default(),
        });
        tracked.at = at;
        tracked.facing = facing.direction;
        tracked.body = body.0;
        tracked.step = None;
        tracked.stepped_at = None;

        Mobile {
            at,
            body: body.0,
            group: tracked.group,
            facing: facing.direction,
            frame: 0,
            hue,
            glide: None,
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
        self.speech.retain(|who, _| present(*who));
    }

    /// Record that `who` said `text`, replacing whatever they were saying
    /// before.
    ///
    /// Not folded into [`Crowd::see`]: a `0x1C` and a `0x77` are different
    /// packets that arrive on their own schedules, and a speaker does not have
    /// to have moved for a line to be worth showing.
    pub fn hear(&mut self, who: Who, text: String, font: Font, hue: Hue) {
        let started = self.now;
        self.speech.insert(
            who,
            Speech {
                text,
                font,
                hue,
                started,
            },
        );
    }

    /// What `who` is still saying, or `None` once [`SPEECH_HOLD`] has passed.
    ///
    /// Checked against the clock here rather than expired in [`Crowd::advance`]:
    /// nothing downstream needs to know the *moment* a line goes stale, only
    /// whether it still is one, and a lazy check is one fewer place that has to
    /// agree with [`SPEECH_HOLD`].
    pub fn speaking(&self, who: Who) -> Option<(&str, Font, Hue)> {
        let speech = self.speech.get(&who)?;
        (self.now.saturating_sub(speech.started) < SPEECH_HOLD).then_some((
            speech.text.as_str(),
            speech.font,
            speech.hue,
        ))
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

    /// How much of a frame's span this body spent actually crossing its tile.
    ///
    /// The whole span for a body that is walking or standing, and only the part
    /// before the crossing ended for one that arrived part way through it. `was`
    /// is where the crowd's clock stood before the span.
    ///
    /// # Why a body that has arrived stops playing
    ///
    /// The window between the crossing ending and [`animation_hold`] expiring is
    /// a foot in the air over ground already covered. The hold exists so that a
    /// walker whose next step is a round trip away does not drop to standing for
    /// a frame and restart the walk at frame zero — and *holding a group* is all
    /// it was ever meant to do. Advancing the clock through it plays the rest of
    /// the stride on the spot: 200ms at a walk, two and a half frames of a body
    /// striding while its feet cover no ground, at the end of every walk. Which
    /// is the complaint it came from — a step that plays itself out after the
    /// walking has stopped.
    ///
    /// The reference draws exactly this line. `Mobile.NoIterateAnimIndex` is
    /// true while `LastStepTime > Time.Ticks - Constants.WALKING_DELAY &&
    /// Steps.Count == 0` — still counted as walking, no step left to walk, so
    /// the frame index does not advance. The group is held and the picture is
    /// frozen, and the next step carries the stride on from where it stopped.
    fn striding(&self, was: Duration, dt: Duration) -> Duration {
        let Some(step) = self.step else {
            // Standing, and a standing animation plays: this is the walk's
            // freeze, not a general one.
            return dt;
        };
        let ends = step.started + step.takes;
        match ends.checked_sub(was) {
            Some(left) => dt.min(left),
            // The crossing was already over when the span began.
            None => Duration::ZERO,
        }
    }

    /// Where this body is drawn at a moment on the crowd's clock, sub-tile.
    ///
    /// The whole of what makes a step continuous: a new step starts here rather
    /// than on the tile boundary, so an arrival that is early or late changes
    /// the *speed* of the crossing that follows it and never the position. The
    /// picture cannot jump, by construction, whatever the wire does.
    ///
    /// `when` is normally the instant a step is beginning, which for a chained
    /// one is in the past — see [`began`]. A body with nothing in flight is on
    /// its tile.
    fn gaze_at(&self, when: Duration) -> Gaze {
        // The renderer's own formula and not a copy of it: a step that started
        // from a position a fraction of a pixel off where the sprite was drawn
        // is the jump this exists to remove, one pixel smaller.
        openshard_client_render::mobiles::glided(self.at, self.glide(when))
    }

    /// Where between two tiles this body is, at a moment on the crowd's clock.
    fn glide(&self, now: Duration) -> Option<Glide> {
        let step = self.step?;
        let from = step.from?;
        // Saturating rather than asserted: `now` is the crowd's own clock and
        // only ever moves forward, but a step heard *this* instant divides
        // zero by the step's length, which is the honest zero.
        let elapsed = now.saturating_sub(step.started);
        // Arrived. Not a glide of progress 1.0, which is the same picture and a
        // different answer to "is anybody mid-step": the window redraws at 60Hz
        // for as long as one is, and the body would keep it there for the half
        // step its animation goes on playing.
        if elapsed >= step.takes {
            return None;
        }
        Some(Glide {
            from,
            progress: elapsed.as_secs_f32() / step.takes.as_secs_f32(),
        })
    }
}

/// How long this crossing has left to run, for a body whose cadence is known.
///
/// # The arrival is noise and the cadence is not
///
/// A step is handed to this layer when the event loop wakes with it — after the
/// wire, after an mpsc, after however late the operating system got round to the
/// thread. The *cadence* underneath that has none of the noise in it: `steer.rs`
/// arms each step from the previous one's deadline rather than from the wake it
/// happened to be taken at, precisely so that lateness does not accumulate.
///
/// Crossing every tile in the nominal time *from the arrival* therefore
/// re-randomises the phase of every tile: each crossing is the right length and
/// starts a few milliseconds late, by a different few each time, so the body's
/// position steps by the difference of two of them at every tile boundary. Eight
/// milliseconds of wake jitter is a pixel a tile, once every 400ms — and the
/// camera is locked to the body, so the whole world takes it. That is the jerk
/// the walk dump in `dst.rs` was written to measure.
///
/// So what is chained is the *end* of the crossing and not its start. The step
/// begins now, from where the body is now — which is what keeps the picture
/// continuous, see [`Step::from`] — and it is given however long is left until
/// the instant the cadence says it should arrive. The few milliseconds of
/// lateness come out of the crossing's *speed*, where at eight milliseconds in
/// four hundred they are two per cent and nobody can see them, instead of out of
/// its position, where they are a pixel and everybody can.
///
/// Chaining the start instead would put it in the past, which is arithmetically
/// the same schedule and a jump on screen: the body was drawn parked on its tile
/// for the frames between the boundary and the arrival, and a retroactive start
/// makes the next frame catch up all at once.
///
/// Believed only within half and double the nominal length, which is the band
/// [`glide_time`] trusts a measurement in and for the same reason: outside it
/// there is no cadence to chain from — a body that had stopped, a step answered
/// after a stall, a rollback — and the wire's own claim is at least a walking
/// speed. Everyone else gets the nominal length too, because nothing on the wire
/// says when an NPC set off and a schedule invented for a guessed pace would
/// move it somewhere it never was.
fn crossing(previous: Option<Step>, commanded: bool, now: Duration, nominal: Duration) -> Duration {
    if !commanded {
        return nominal;
    }
    let Some(previous) = previous else { return nominal };
    // Where the cadence says this crossing ends: the previous one's scheduled
    // end, which is when this step set off, plus a step.
    let ends = previous.started + previous.takes + nominal;
    let Some(left) = ends.checked_sub(now) else {
        return nominal;
    };
    match left >= nominal / 2 && left <= nominal * 2 {
        true => left,
        false => nominal,
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
        assert_eq!(
            glide.from,
            Gaze::on(Point::new(10, 10, 0)),
            "from where it was standing"
        );
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
        // And it keeps *playing* the walk for half a step longer, which is what
        // stops a body that is genuinely walking from standing for a frame
        // between two steps. See `animation_hold`.
        assert_eq!(step(&mut crowd, 11).group, 0, "still playing the walk");
        crowd.advance(WALK_HOLD / 2);
        assert_eq!(step(&mut crowd, 11).group, 4, "standing");
    }

    /// The defect the split between the crossing and the animation was written
    /// for: a walking client's steps arrive one step apart give or take the
    /// round trip, and a hold of exactly one step expires in that gap — so the
    /// body stands for a frame every tile, and standing is a different group,
    /// which restarts the walk's clock at frame zero.
    #[test]
    fn a_step_that_arrives_late_does_not_drop_the_walk_to_standing() {
        let mut crowd = Crowd::default();
        let step = |crowd: &mut Crowd, x: u16| {
            crowd.see(
                serial(1),
                Point::new(x, 10, 0),
                Graphic(PLAYER),
                Facing::walking(Direction::East),
                Hue::NONE,
            )
        };
        step(&mut crowd, 10);
        step(&mut crowd, 11);
        for x in 12..20u16 {
            // A tenth of a step late, every step: the jitter of a real
            // connection, and enough to have flickered.
            crowd.advance(WALK_HOLD + WALK_HOLD / 10);
            assert_eq!(step(&mut crowd, x).group, 0, "walking at tile {x}");
        }
    }

    /// The last step of a walk does not play itself out on the spot.
    ///
    /// The complaint: the character finishes walking and takes one more stride
    /// where it stands. It is the animation hold — the walk's group is kept for
    /// half a step past the crossing so that a walker does not flicker into
    /// standing between two tiles — and the clock was running through it, which
    /// is two and a half frames of stride over ground already covered.
    ///
    /// So the frame is frozen instead, which is what the reference does
    /// (`Mobile.NoIterateAnimIndex`). Both halves are asserted: the frame stops
    /// where the crossing ended, and the group is still the walk while it does —
    /// a body that dropped to standing here would also have a still frame, and
    /// it is the flicker the hold exists to prevent.
    #[test]
    fn the_last_step_of_a_walk_does_not_play_itself_out() {
        let mut crowd = Crowd::default();
        let step = |crowd: &mut Crowd, x: u16| {
            crowd.see(
                serial(1),
                Point::new(x, 10, 0),
                Graphic(PLAYER),
                Facing::walking(Direction::East),
                Hue::NONE,
            )
        };
        step(&mut crowd, 10);
        assert_eq!(step(&mut crowd, 11).group, 0, "walking");

        // The crossing: the frame moves, because the body is covering ground.
        crowd.advance(WALK_HOLD);
        let arrived = crowd.frame_for(serial(1), 6);
        assert!(arrived > 0, "the walk played while it walked");
        assert_eq!(crowd.glide_for(serial(1)), None, "and the tile is crossed");

        // The hold: the group is held and the picture is not. Three eighths of
        // a step and not four, because the fourth lands exactly on
        // `animation_hold` — where the body is *meant* to drop to standing, and
        // a test that straddled the boundary would be asserting about which
        // side of it a rounding fell on.
        for _ in 0..3 {
            crowd.advance(WALK_HOLD / 8);
            assert_eq!(
                crowd.frame_for(serial(1), 6),
                arrived,
                "the stride played on after the walking stopped",
            );
            assert_eq!(
                step(&mut crowd, 11).group,
                0,
                "and it is still holding the walk rather than standing",
            );
        }
        // And then it stands, which is a different group and its own clock.
        crowd.advance(WALK_HOLD / 2);
        assert_eq!(step(&mut crowd, 11).group, 4);
        assert_eq!(crowd.frame_for(serial(1), 6), 0);
    }

    /// The companion, and it is the whole of what makes the test above an
    /// assertion: a walk that is still walking keeps playing.
    ///
    /// A freeze applied to every frame would pass the test above and produce a
    /// character that slides across the map with its feet still — which is the
    /// same false green a still scene always is.
    #[test]
    fn a_walk_that_is_still_walking_keeps_playing() {
        let mut crowd = Crowd::default();
        let step = |crowd: &mut Crowd, x: u16| {
            crowd.see(
                serial(1),
                Point::new(x, 10, 0),
                Graphic(PLAYER),
                Facing::walking(Direction::East),
                Hue::NONE,
            );
        };
        step(&mut crowd, 10);
        let mut frames = std::collections::BTreeSet::new();
        for x in 11..16u16 {
            step(&mut crowd, x);
            for _ in 0..5 {
                crowd.advance(WALK_HOLD / 5);
                frames.insert(crowd.frame_for(serial(1), 6));
            }
        }
        assert!(
            frames.len() >= 5,
            "a walk of five tiles showed {} distinct frames",
            frames.len(),
        );
    }

    /// And the next step picks the stride up where it was frozen rather than
    /// restarting it.
    ///
    /// Free, because the group has not changed and only a change restarts the
    /// clock — and worth pinning, because the alternative reads as a body that
    /// hesitates every time the wire is slow.
    #[test]
    fn a_step_after_the_freeze_carries_the_stride_on() {
        let mut crowd = Crowd::default();
        let step = |crowd: &mut Crowd, x: u16| {
            crowd.see(
                serial(1),
                Point::new(x, 10, 0),
                Graphic(PLAYER),
                Facing::walking(Direction::East),
                Hue::NONE,
            );
        };
        step(&mut crowd, 10);
        step(&mut crowd, 11);
        crowd.advance(WALK_HOLD + WALK_HOLD / 4);
        let frozen = crowd.frame_for(serial(1), 6);
        step(&mut crowd, 12);
        assert_eq!(crowd.frame_for(serial(1), 6), frozen, "no hesitation");
        crowd.advance(Duration::from_millis(80));
        assert_ne!(crowd.frame_for(serial(1), 6), frozen, "and it walks on");
    }

    /// The offline placeholder's steps are ours from the first frame, without
    /// anything having to say so.
    ///
    /// A body we command crosses its tile in the nominal time — we sent the step
    /// and we know when — where a body we merely hear about is glided over the
    /// gap that was measured, wake jitter and all. Nothing but this client can
    /// move the placeholder, so it is the first case, and it used to be armed by
    /// hand in `App::start_replay`: every other path that walks it offline, and
    /// there is one per key, measured the walk through the event loop instead.
    #[test]
    fn a_client_with_no_serial_commands_the_body_it_walks() {
        let mut crowd = Crowd::default();
        let step = |crowd: &mut Crowd, x: u16| {
            crowd.see(
                None,
                Point::new(x, 10, 0),
                Graphic(PLAYER),
                Facing::walking(Direction::East),
                Hue::NONE,
            );
        };
        step(&mut crowd, 10);
        // Half a step late, which is what a wake the loop slept through looks
        // like. Measured, the next crossing would take one and a half steps.
        crowd.advance(WALK_HOLD + WALK_HOLD / 2);
        step(&mut crowd, 11);
        crowd.advance(WALK_HOLD - Duration::from_millis(1));
        let glide = crowd.glide_for(None).expect("still crossing");
        assert!(glide.progress > 0.99, "all but arrived: {}", glide.progress);
        crowd.advance(Duration::from_millis(1));
        assert_eq!(
            crowd.glide_for(None),
            None,
            "the nominal step, not the gap the loop happened to wake at",
        );
    }

    /// The step we command ends when the cadence says, whenever we heard about
    /// it.
    ///
    /// The jerk this whole pair of rules came from: `steer.rs` arms each step
    /// from the previous deadline, so the *asks* are an exact metronome, but the
    /// news of each one reaches this layer whenever the loop wakes. A crossing
    /// measured from the arrival is the right length starting at the wrong
    /// instant, by a different wrong instant every tile — so the body's position
    /// steps at every boundary and the camera, locked to it, takes the whole
    /// world with it.
    ///
    /// Here the second step is heard a tenth of a step late and must still
    /// arrive on the beat, which it does by being that much shorter.
    #[test]
    fn a_step_we_command_arrives_on_the_beat_however_late_it_was_heard() {
        let mut crowd = Crowd::default();
        crowd.commanding(None);
        let step = |crowd: &mut Crowd, x: u16| {
            crowd.see(
                None,
                Point::new(x, 10, 0),
                Graphic(PLAYER),
                Facing::walking(Direction::East),
                Hue::NONE,
            );
        };
        let late = WALK_HOLD / 10;
        step(&mut crowd, 10);
        step(&mut crowd, 11);
        crowd.advance(WALK_HOLD + late);
        step(&mut crowd, 12);

        // A hold after the *first* step's own deadline, less the lateness, the
        // body is on its tile — which is to say two tiles in two holds, and not
        // two tiles in two holds plus the two latenesses.
        crowd.advance(WALK_HOLD - late - Duration::from_millis(1));
        let glide = crowd.glide_for(None).expect("still crossing");
        assert!(glide.progress > 0.99, "all but arrived: {}", glide.progress);
        crowd.advance(Duration::from_millis(1));
        assert_eq!(crowd.glide_for(None), None, "on the beat");
    }

    /// A step starts from where the body is drawn, not from the tile it is
    /// leaving — so nothing an arrival does can move the sprite.
    ///
    /// The case that names the difference is the opposite of the one above: a
    /// step heard *early*, while the previous crossing still has a quarter to
    /// run. Anchored to the tile boundary the sprite jumps a quarter of a tile
    /// backwards to start the new step; anchored to itself it carries on from
    /// where it is and covers the extra ground by going a little faster.
    #[test]
    fn a_step_picks_up_from_where_the_body_is_drawn() {
        let mut crowd = Crowd::default();
        let step = |crowd: &mut Crowd, x: u16| {
            crowd.see(
                serial(1),
                Point::new(x, 10, 0),
                Graphic(PLAYER),
                Facing::walking(Direction::East),
                Hue::NONE,
            )
        };
        step(&mut crowd, 10);
        step(&mut crowd, 11);
        crowd.advance(WALK_HOLD * 3 / 4);
        let before = drawn(&crowd, serial(1), Point::new(11, 10, 0));

        let stepped = step(&mut crowd, 12);
        let after = drawn(&crowd, serial(1), Point::new(12, 10, 0));
        assert_eq!(before, after, "the sprite moved on an arrival, not on a clock");
        assert_eq!(
            stepped.glide.expect("mid-step").progress,
            0.0,
            "and the new step has not run yet",
        );
    }

    /// Where a body is drawn, through the renderer's own placement.
    fn drawn(crowd: &Crowd, who: Who, at: Point) -> (i64, i64) {
        let gaze = openshard_client_render::mobiles::glided(at, crowd.glide_for(who));
        // Rounded to a thousandth of a pixel: the comparison is about a quarter
        // of a tile, and an exact one would be asserting about the last bit of
        // an `f64` division.
        let (x, y) = gaze.exact();
        ((x * 1_000.0).round() as i64, (y * 1_000.0).round() as i64)
    }

    /// And a walk already under way crosses each tile in the time the last one
    /// took, so a body whose steps arrive slower than the nominal rate glides
    /// the whole way rather than arriving early and waiting.
    #[test]
    fn the_crossing_takes_as_long_as_the_last_step_did() {
        let mut crowd = Crowd::default();
        let step = |crowd: &mut Crowd, x: u16| {
            crowd.see(
                serial(1),
                Point::new(x, 10, 0),
                Graphic(PLAYER),
                Facing::walking(Direction::East),
                Hue::NONE,
            );
        };
        step(&mut crowd, 10);
        step(&mut crowd, 11);
        // Half a step late. The next crossing is measured from it, so it takes
        // one and a half steps and is still going when the one after arrives.
        let gap = WALK_HOLD + WALK_HOLD / 2;
        crowd.advance(gap);
        step(&mut crowd, 12);
        crowd.advance(gap - Duration::from_millis(1));
        let glide = crowd.glide_for(serial(1)).expect("still crossing");
        assert!(glide.progress > 0.99, "all but arrived: {}", glide.progress);
        crowd.advance(Duration::from_millis(1));
        assert_eq!(crowd.glide_for(serial(1)), None, "and arrived exactly on time");
    }

    /// A gap that is not a pace is not believed: a body that had stopped, or two
    /// steps arriving in one burst, are glided at the rate the wire claims.
    #[test]
    fn a_gap_that_is_not_a_pace_falls_back_to_the_wires_own_rate() {
        assert_eq!(glide_time(WALK_HOLD, None), WALK_HOLD);
        assert_eq!(
            glide_time(WALK_HOLD, Some(Duration::from_secs(30))),
            WALK_HOLD,
            "a body that had stopped and started again"
        );
        assert_eq!(
            glide_time(WALK_HOLD, Some(Duration::from_millis(1))),
            WALK_HOLD,
            "two steps in one burst"
        );
        assert_eq!(
            glide_time(WALK_HOLD, Some(WALK_HOLD + WALK_HOLD / 4)),
            WALK_HOLD + WALK_HOLD / 4,
            "and a plausible pace is taken at its word"
        );
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
        crowd.advance(RUN_HOLD / 2);
        assert_eq!(run(&mut crowd, 11).group, 4, "standing, half a walk early");
        assert_eq!(RUN_HOLD * 2, WALK_HOLD, "ServUO's RunFoot against its WalkFoot");
    }

    /// A rollback is put, not walked: the tile the body is sent back to was
    /// never crossed, so gliding into it would draw the character strolling
    /// backwards a tile at a time for as long as a wall refuses it.
    #[test]
    fn a_refused_step_is_snapped_back_rather_than_glided() {
        let mut crowd = Crowd::default();
        let at = Point::new(10, 10, 0);
        let facing = Facing::walking(Direction::East);
        crowd.see(serial(1), at, Graphic(PLAYER), facing, Hue::NONE);
        // The client predicts a step east and the server refuses it.
        let stepped = crowd.see(
            serial(1),
            Point::new(11, 10, 0),
            Graphic(PLAYER),
            facing,
            Hue::NONE,
        );
        assert!(stepped.glide.is_some(), "the prediction is walked into");
        crowd.advance(WALK_HOLD / 4);

        let back = crowd.snap(serial(1), at, Graphic(PLAYER), facing, Hue::NONE);
        assert_eq!(back.at, at);
        assert_eq!(back.glide, None, "and back is a jump");
        assert_eq!(crowd.glide_for(serial(1)), None);
        assert_eq!(back.group, 0, "still walking: the next step is already coming");

        // The gap between a step and a refusal is not a pace, so the next step
        // is glided at the wire's own rate rather than at a quarter of one.
        crowd.advance(WALK_HOLD / 4);
        crowd.see(
            serial(1),
            Point::new(10, 11, 0),
            Graphic(PLAYER),
            facing,
            Hue::NONE,
        );
        crowd.advance(WALK_HOLD / 2);
        assert_eq!(
            crowd.glide_for(serial(1)).expect("mid-step").progress,
            0.5,
            "a full walk's crossing"
        );
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

    /// A line is there the instant it is heard, and gone once the hold runs
    /// out.
    #[test]
    fn a_line_is_spoken_and_then_expires() {
        let mut crowd = Crowd::default();
        crowd.hear(serial(1), "hello".to_string(), Font(0), Hue::NONE);
        assert_eq!(crowd.speaking(serial(1)).map(|(text, ..)| text), Some("hello"));
        crowd.advance(SPEECH_HOLD - Duration::from_millis(1));
        assert!(crowd.speaking(serial(1)).is_some(), "not yet");
        crowd.advance(Duration::from_millis(2));
        assert!(crowd.speaking(serial(1)).is_none(), "and now it has");
    }

    /// A second line replaces the first rather than queuing behind it: only
    /// one line is ever drawn over a head at a time.
    #[test]
    fn a_new_line_replaces_the_old_one_and_restarts_the_hold() {
        let mut crowd = Crowd::default();
        crowd.hear(serial(1), "first".to_string(), Font(0), Hue::NONE);
        crowd.advance(SPEECH_HOLD - Duration::from_millis(1));
        crowd.hear(serial(1), "second".to_string(), Font(0), Hue::NONE);
        assert_eq!(crowd.speaking(serial(1)).map(|(text, ..)| text), Some("second"));
        // The old line would have expired by now; the new one has its own
        // clock and has not.
        crowd.advance(Duration::from_millis(2));
        assert_eq!(crowd.speaking(serial(1)).map(|(text, ..)| text), Some("second"));
    }

    /// Nobody not yet heard from is saying anything.
    #[test]
    fn a_serial_never_heard_is_not_speaking() {
        let crowd = Crowd::default();
        assert!(crowd.speaking(serial(1)).is_none());
    }

    /// `retain` forgets a stale line along with the rest of what a departed
    /// mobile was doing.
    #[test]
    fn retain_forgets_speech_along_with_position() {
        let mut crowd = Crowd::default();
        crowd.hear(serial(1), "bye".to_string(), Font(0), Hue::NONE);
        crowd.retain(|who| who != serial(1));
        assert!(crowd.speaking(serial(1)).is_none());
    }
}
