//! A walk, held against an oracle, on a clock nobody sleeps on.
//!
//! # Two timelines
//!
//! Walking is answered in four places that never meet in a unit test: `steer.rs`
//! decides *when* a step is asked for, `client/net`'s [`Walk`] decides *where*
//! that step lands and predicts it, the shard decides whether it is allowed at
//! all, and `crowd.rs` decides where between two tiles the body is drawn this
//! frame. Each of the four has its own tests and each of them passed while the
//! walk on screen stuttered, because the bug was never in one of them — it was
//! in the *timing between* them. What is missing is an assertion about the thing
//! the player actually complains about: the position of the sprite, as a
//! function of the moment they pressed the key.
//!
//! So there are two timelines here.
//!
//! The **intent** timeline is the oracle: the body leaves the instant the key
//! goes down and crosses one tile per step, at a constant speed, for ever. It is
//! built from the script of inputs alone — it never sees a packet, a clock or a
//! `Crowd` — and it is deliberately the simplest kinematics that could be right.
//!
//! The **event** timeline is everything below: a step is asked for when the
//! event loop happens to wake, crosses an mpsc to the net task, is predicted,
//! crosses back, and is glided over whatever [`crate::crowd::glide_time`] made
//! of the gap since the last one. Meanwhile the `0x02` crosses a wire with
//! latency and jitter on it and the shard answers a round trip later.
//!
//! The claim under test is that the second reproduces the first. That is not a
//! tautology dressed as a test: the oracle's knots are at `k * WALK_HOLD` from
//! the *press*, and the system's are at whenever the loop woke and whatever the
//! wire did. Every walking bug this client has had was a divergence between
//! those two sets of knots.
//!
//! # Not even the turn is a delay
//!
//! Turning is a whole step in UO: a mobile asked for a direction it is not
//! facing turns, moves nowhere, and gets its own `0x22`
//! (`openshard_movement::intend`). It is tempting to write that into the oracle
//! as a hold of standing still, and it would be wrong — the shard answers a turn
//! *before* it charges the pace budget (`Walker::request`), so the step the turn
//! precedes is legal in the same instant. `steer.rs` sends both in one wake and
//! the body sets off on the frame the key went down. So this oracle is constant
//! velocity from the moment of the ask and nothing else: no turn tax, no ramp,
//! no easing.
//!
//! # What is deliberately not covered
//!
//! The loop below is a copy of the ten lines in `App::about_to_wait` and
//! `App::user_event` that drive the walk, because those live inside a `winit`
//! handler that needs a window and a real clock. The copy is the known weakness
//! of this harness: a divergence introduced *into the copy* would be invisible
//! here. If one of these tests ever needs a rule that is only in `App`, the
//! answer is to lift that loop into a headless unit both can drive, not to grow
//! the copy — see `docs/client.md`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use openshard_client_net::walk::{Moved, Predicted, Walk};
use openshard_client_render::mobiles::Mobile;
use openshard_movement::{OpenWorld, Terrain, Walk as Handled, Walker};
use openshard_protocol::direction::{Direction, Facing};
use openshard_protocol::mobile::Notoriety;
use openshard_protocol::packet::decode_packet;
use openshard_protocol::serial::Serial;
use openshard_protocol::server_packet::ServerPacket;
use openshard_protocol::version::ClientVersion;
use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::{Point, WalkAck, WalkReject, WalkRequest};

use crate::GLIDE_INTERVAL;
use crate::crowd::{Crowd, RUN_HOLD, WALK_HOLD, Who};

/// The body every scenario walks.
const BODY: Graphic = Graphic(0x0190);

/// Where every scenario starts. Far from the map's edges, so that no step is
/// refused for leaving the coordinate space.
const START: Point = Point::new(1000, 1000, 0);

/// A player-shaped `ClientVersion`, for the one packet that is decoded here.
fn version() -> ClientVersion {
    ClientVersion::new(7, 0, 45, 65)
}

/// Our own serial, as a `0x1B` would have named it.
fn me() -> Who {
    Some(Serial::new(0x0000_0001).unwrap())
}

// --- The oracle -----------------------------------------------------------

/// One thing the player did.
///
/// Scripts here hold one arrow at a time on purpose: which of several held
/// arrows wins is `keys.rs`'s rule, it has its own tests, and restating it in
/// the oracle would make the oracle a second copy of the thing under test.
#[derive(Clone, Copy, Debug)]
enum Input {
    /// An arrow went down.
    Press(Direction),
    /// The held arrow came up.
    Release,
    /// Shift went down or came up.
    Running(bool),
}

/// A scripted input and the moment it happens.
#[derive(Clone, Copy, Debug)]
struct Act {
    /// How long after the scenario started.
    at: Duration,
    /// What the player did.
    input: Input,
}

/// A press at `millis`.
const fn press(millis: u64, direction: Direction) -> Act {
    Act {
        at: Duration::from_millis(millis),
        input: Input::Press(direction),
    }
}

/// A release at `millis`.
const fn release(millis: u64) -> Act {
    Act {
        at: Duration::from_millis(millis),
        input: Input::Release,
    }
}

/// One crossing on the intent timeline: where the body goes, from when, over
/// how long.
#[derive(Clone, Copy, Debug)]
struct Knot {
    /// When the crossing starts.
    at: Duration,
    /// How long it takes — the hold of the pace being walked at.
    takes: Duration,
    /// The tile it leaves, in tiles.
    from: (f64, f64),
    /// The tile it arrives at.
    to: (f64, f64),
}

/// Where the body would be, if the ask reached the screen with nothing in
/// between.
#[derive(Clone, Debug)]
struct Oracle {
    /// Where it stood before the first knot.
    start: (f64, f64),
    /// Every crossing, in order.
    knots: Vec<Knot>,
}

impl Oracle {
    /// Replay a script into crossings.
    ///
    /// The rules, all of them:
    ///
    /// - a press takes a step *now* and the next is due a hold later — waiting a
    ///   whole step before the first one would put 400ms between the player and
    ///   their character, which is the thing this whole file exists to refuse;
    /// - every step crosses one tile over one hold at a constant speed,
    ///   whichever way the body was facing when it was asked for — a turn is a
    ///   packet, not a delay (see the module docs);
    /// - shift changes the pace of the *next* step and does not re-time the one
    ///   already due.
    fn build(start: Point, script: &[Act], until: Duration) -> Self {
        let mut knots = Vec::new();
        let mut position = (f64::from(start.x), f64::from(start.y));
        let mut running = false;
        let mut held: Option<Direction> = None;
        let mut due: Option<Duration> = None;
        let mut acts = script.iter().copied().peekable();
        let mut now;

        loop {
            // Whichever comes first: the player doing something, or the step
            // already owed falling due.
            let next_act = acts.peek().map(|act| act.at);
            now = match (next_act, due) {
                (Some(act), Some(step)) => act.min(step),
                (Some(act), None) => act,
                (None, Some(step)) => step,
                (None, None) => break,
            };
            if now > until {
                break;
            }

            if next_act == Some(now) {
                match acts.next().unwrap().input {
                    Input::Press(direction) => held = Some(direction),
                    Input::Release => {
                        held = None;
                        due = None;
                        continue;
                    }
                    Input::Running(shift) => {
                        running = shift;
                        continue;
                    }
                }
            } else if held.is_none() {
                due = None;
                continue;
            }

            let Some(direction) = held else { continue };
            let takes = if running { RUN_HOLD } else { WALK_HOLD };
            // A turn costs nothing. It is a `0x02` of its own — turning is a
            // step in UO and the shard acks it — but it is not a *delay*: the
            // shard answers a turn before it charges the pace budget, so the
            // step it precedes leaves in the same instant, and the body sets off
            // on the frame the key went down. So the direction asked for is the
            // direction walked, from the first millisecond, and this oracle is
            // constant velocity and nothing else.
            let (dx, dy) = direction.step();
            let arrives = (position.0 + f64::from(dx), position.1 + f64::from(dy));
            knots.push(Knot {
                at: now,
                takes,
                from: position,
                to: arrives,
            });
            position = arrives;
            due = Some(now + takes);
        }

        Self {
            start: (f64::from(start.x), f64::from(start.y)),
            knots,
        }
    }

    /// Where the body should be at `when`.
    fn at(&self, when: Duration) -> (f64, f64) {
        let mut position = self.start;
        for knot in &self.knots {
            if when < knot.at {
                break;
            }
            let elapsed = when - knot.at;
            if elapsed >= knot.takes {
                position = knot.to;
                continue;
            }
            let progress = elapsed.as_secs_f64() / knot.takes.as_secs_f64();
            position = (
                knot.from.0 + (knot.to.0 - knot.from.0) * progress,
                knot.from.1 + (knot.to.1 - knot.from.1) * progress,
            );
            break;
        }
        position
    }
}

// --- The simulated world --------------------------------------------------

/// Everything between the key and the sprite that is not the client: how long a
/// packet takes, and how late the event loop wakes.
///
/// All four are deliberately separate knobs. Latency the client is supposed to
/// hide entirely — that is what predicting the step is *for* — and wake jitter
/// it cannot hide at all, so a test that mixed them could not say which one a
/// divergence came from.
#[derive(Clone, Copy, Debug, Default)]
struct Net {
    /// One way. A `0x02` takes this to reach the shard and its answer takes it
    /// to come back.
    latency: Duration,
    /// Added to each crossing, uniformly in `[0, jitter]`.
    jitter: Duration,
    /// How late the event loop wakes, uniformly in `[0, wake_jitter]`. A real
    /// one is never early.
    wake_jitter: Duration,
}

/// A seeded generator, so a failing scenario is a seed and not a story.
///
/// SplitMix64: four lines, no dependency, and the sequence is pinned by the
/// tests that use it.
#[derive(Clone, Debug)]
struct Rng(u64);

impl Rng {
    /// A uniform duration in `[0, span]`.
    fn upto(&mut self, span: Duration) -> Duration {
        if span.is_zero() {
            return Duration::ZERO;
        }
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        Duration::from_nanos(z % (span.as_nanos() as u64 + 1))
    }
}

/// A world with a wall in it, or none.
///
/// `can_step` is the whole `Terrain` contract, and the only reason there is one
/// here rather than [`OpenWorld`] is the rollback scenario: a refusal is the one
/// event the oracle cannot predict, and the client's behaviour around it is
/// worth pinning anyway.
struct Field {
    /// Tiles no step may land on. Empty is an open field.
    walls: Vec<(u16, u16)>,
}

impl Terrain for Field {
    fn can_step(&self, from: Point, to: Point) -> Option<Point> {
        match self.walls.contains(&(to.x, to.y)) {
            true => None,
            false => OpenWorld.can_step(from, to),
        }
    }
}

/// The client, the wire and a shard, on one virtual clock.
struct Sim {
    /// The virtual clock: everything here is an offset from the start.
    now: Duration,
    /// One arbitrary real instant, because [`crate::steer::Steering`] and
    /// `WalkPace` take `Instant`s. Neither *reads* one — they only ever
    /// subtract two — so a base plus the virtual clock is a real clock as far as
    /// they can tell, and nothing in this file sleeps.
    base: Instant,

    steering: crate::steer::Steering,
    /// The client's prediction. On the net task in the real client, which is
    /// why the hop to it is modelled below rather than called inline.
    walk: Walk,
    crowd: Crowd,
    /// The body as `App` holds it: the tile, and the glide it was last drawn
    /// with.
    player: Mobile,

    /// The shard's side of the same walk — the real rules, sequence check and
    /// anti-speedhack bucket included.
    shard: Walker,
    field: Field,

    /// `0x02`s crossing to the shard, and answers crossing back, by arrival.
    to_shard: VecDeque<(Duration, Vec<u8>)>,
    to_client: VecDeque<(Duration, ServerPacket)>,
    /// The prediction crossing the mpsc back into the window — `link::Update`.
    to_window: VecDeque<(Duration, Predicted, bool)>,

    net: Net,
    rng: Rng,

    /// The window's own clocks, exactly as `App` keeps them.
    next_tick: Duration,
    last_advance: Duration,

    /// How many steps the shard refused, for the assertions.
    refused: u32,
    /// Where the body was *drawn*, every frame.
    trace: Vec<(Duration, (f64, f64))>,
}

impl Sim {
    /// A client standing at [`START`], facing `facing`, connected to a shard
    /// that agrees.
    fn new(facing: Direction, net: Net, seed: u64, walls: Vec<(u16, u16)>) -> Self {
        let facing = Facing::walking(facing);
        let mut crowd = Crowd::default();
        crowd.commanding(me());
        let player = crowd.see(me(), START, BODY, facing, Hue::NONE);
        Self {
            now: Duration::ZERO,
            base: Instant::now(),
            steering: crate::steer::Steering::default(),
            walk: Walk::new(START, facing),
            crowd,
            player,
            shard: Walker::new(START, facing),
            field: Field { walls },
            to_shard: VecDeque::new(),
            to_client: VecDeque::new(),
            to_window: VecDeque::new(),
            net,
            rng: Rng(seed),
            next_tick: Duration::ZERO,
            last_advance: Duration::ZERO,
            refused: 0,
            trace: Vec::new(),
        }
    }

    /// The virtual clock as an `Instant`, for the two units that take one.
    fn instant(&self) -> Instant {
        self.base + self.now
    }

    /// How long one crossing of the wire takes this time.
    fn hop(&mut self) -> Duration {
        self.net.latency + self.rng.upto(self.net.jitter)
    }

    /// Run the script, waking the way the event loop wakes, until `until`.
    fn run(&mut self, script: &[Act], until: Duration) {
        let mut acts = script.iter().copied().peekable();
        loop {
            // Every reason to come back, exactly as `about_to_wait` computes the
            // deadline: the animation clock, the step a held key is owed, a
            // packet arriving — and, here only, the player doing something.
            let mut wake = self.next_tick.min(until);
            if let Some(step) = self.steering.deadline() {
                wake = wake.min(step - self.base);
            }
            for arrival in [
                self.to_shard.front().map(|(at, _)| *at),
                self.to_client.front().map(|(at, _)| *at),
                self.to_window.front().map(|(at, ..)| *at),
                acts.peek().map(|act| act.at),
            ]
            .into_iter()
            .flatten()
            {
                wake = wake.min(arrival);
            }
            // A loop that wakes on time is the easy case; a real one is woken by
            // the operating system whenever it gets round to it, and never early.
            let late = self.rng.upto(self.net.wake_jitter);
            self.now = (wake + late).max(self.now);
            if self.now > until {
                self.now = until;
            }

            while acts.peek().is_some_and(|act| act.at <= self.now) {
                let act = acts.next().unwrap();
                match act.input {
                    Input::Press(direction) => {
                        if let Some(facing) =
                            self.steering.press(direction, self.instant(), self.player.facing)
                        {
                            self.send(facing);
                        }
                    }
                    Input::Release => self.steering.clear(),
                    Input::Running(shift) => self.steering.set_running(shift),
                }
            }
            self.deliver();
            self.about_to_wait();

            if self.now >= until {
                break;
            }
        }
    }

    /// Everything whose time has come, in the order it would really arrive.
    fn deliver(&mut self) {
        while self.to_shard.front().is_some_and(|(at, _)| *at <= self.now) {
            let (_, bytes) = self.to_shard.pop_front().unwrap();
            let request: WalkRequest = decode_packet(&bytes, version()).unwrap();
            let sequence = request.sequence.interpret();
            let answer = match self.shard.request(request, &self.field, self.instant(), false) {
                Handled::Turned { .. } | Handled::Moved { .. } => ServerPacket::WalkAck(WalkAck {
                    sequence,
                    notoriety: Notoriety::Innocent,
                }),
                Handled::Refused => {
                    self.refused += 1;
                    ServerPacket::WalkReject(WalkReject {
                        sequence,
                        position: self.shard.position,
                        facing: self.shard.facing,
                    })
                }
            };
            let at = self.now + self.hop();
            self.to_client.push_back((at, answer));
        }

        while self.to_client.front().is_some_and(|(at, _)| *at <= self.now) {
            let (_, packet) = self.to_client.pop_front().unwrap();
            // The net task's fold: `link.rs`'s `fold`, minus the `WorldView`,
            // which holds nothing our own body is drawn from.
            let moved = self.walk.on_packet(&packet).unwrap();
            // Only a correction is worth publishing. A `0x22` confirms a
            // position the screen already has — see `link::Body`.
            if let Moved::Snapped { .. } = moved {
                let at = self.now;
                self.to_window.push_back((at, self.walk.predicted(), true));
            }
        }

        while self.to_window.front().is_some_and(|(at, ..)| *at <= self.now) {
            let (_, predicted, corrected) = self.to_window.pop_front().unwrap();
            // `App::user_event`: the crowd's clock is brought up to date before
            // the packet is folded in, or the step is timestamped at the last
            // frame and the *next* crossing is measured from there.
            self.crowd.advance(self.now - self.last_advance);
            self.last_advance = self.now;
            // `App::entered`, for our own body.
            self.player = match corrected {
                true => self
                    .crowd
                    .snap(me(), predicted.position, BODY, predicted.facing, Hue::NONE),
                false => self
                    .crowd
                    .see(me(), predicted.position, BODY, predicted.facing, Hue::NONE),
            };
            // `App::user_event`: a step arriving while nobody was moving finds
            // the animation clock armed at the standing rate, and the first
            // 80ms of the glide would be drawn frozen.
            let soon = self.now + GLIDE_INTERVAL;
            if self.crowd.anyone_gliding() && self.next_tick > soon {
                self.next_tick = soon;
            }
        }
    }

    /// The ten lines of `App::about_to_wait` that walking goes through.
    fn about_to_wait(&mut self) {
        // Twice at most, exactly as `App::about_to_wait` does it: a turn costs
        // no time, so the step it precedes leaves in the same wake.
        for _ in 0..2 {
            let Some(facing) = self
                .steering
                .due(self.instant(), self.player.at, self.player.facing)
            else {
                break;
            };
            self.send(facing);
        }
        if self.now >= self.next_tick {
            self.crowd.advance(self.now - self.last_advance);
            self.last_advance = self.now;
            self.next_tick = self.now + self.redraw_interval();
            self.sample();
        }
    }

    /// `App::redraw_interval`.
    fn redraw_interval(&self) -> Duration {
        match self.crowd.anyone_gliding() {
            true => GLIDE_INTERVAL,
            false => openshard_client_render::animation::FRAME_DELAY,
        }
    }

    /// `App::walk` with a link: ask, predict, and publish the prediction.
    ///
    /// Two hops, both of them an mpsc in the real client: the command out to
    /// the net task, and the snapshot back. Modelled with no delay of their own
    /// — they are a channel between two threads of one process — but modelled,
    /// because the *order* they impose is real.
    fn send(&mut self, facing: Facing) {
        // No map, so no height: `|_, _| None` is what a caller without one
        // passes, and the flat prediction is the honest answer.
        let bytes = self.walk.step(facing, |_, _| None).unwrap();
        let at = self.now + self.hop();
        self.to_shard.push_back((at, bytes));
        self.to_window.push_back((self.now, self.walk.predicted(), false));
    }

    /// Where the body is drawn this frame, in tiles.
    ///
    /// `App::draw` reads the glide from the crowd every frame rather than from
    /// the `Mobile` it last stored, and so does this: the stored one is as old
    /// as the last packet.
    fn sample(&mut self) {
        let at = self.player.at;
        let position = match self.crowd.glide_for(me()) {
            Some(glide) => {
                let progress = f64::from(glide.progress.clamp(0.0, 1.0));
                (
                    f64::from(glide.from.x) + (f64::from(at.x) - f64::from(glide.from.x)) * progress,
                    f64::from(glide.from.y) + (f64::from(at.y) - f64::from(glide.from.y)) * progress,
                )
            }
            None => (f64::from(at.x), f64::from(at.y)),
        };
        self.trace.push((self.now, position));
    }

    /// The worst the drawn body ever was from where the oracle says it should
    /// have been, in tiles, and when.
    fn worst(&self, oracle: &Oracle) -> (Duration, f64) {
        let mut worst = (Duration::ZERO, 0.0);
        for (when, drawn) in &self.trace {
            let want = oracle.at(*when);
            let off = ((drawn.0 - want.0).powi(2) + (drawn.1 - want.1).powi(2)).sqrt();
            if off > worst.1 {
                worst = (*when, off);
            }
        }
        worst
    }
}

/// Assert the drawn walk never left a corridor of `tiles` around the oracle.
#[track_caller]
fn tracks(sim: &Sim, oracle: &Oracle, tiles: f64) {
    let (when, off) = sim.worst(oracle);
    assert!(
        off <= tiles,
        "the body was {off:.4} tiles from where it should have been at {when:?}, \
         which is more than the {tiles} this scenario allows"
    );
    assert!(
        sim.trace.len() > 100,
        "only {} frames were drawn: a corridor nothing walked down is not an assertion",
        sim.trace.len()
    );
}

/// Ten steps east, held from the first millisecond.
fn ten_steps_east() -> Vec<Act> {
    vec![press(0, Direction::East), release(4_000)]
}

// --- The scenarios ---------------------------------------------------------

/// The oracle is worth exactly as much as its own arithmetic, so pin it first:
/// ten steps, four seconds, one tile per 400ms, and nothing in between.
#[test]
fn the_oracle_walks_ten_tiles_in_ten_holds() {
    let oracle = Oracle::build(START, &ten_steps_east(), Duration::from_millis(4_000));
    assert_eq!(oracle.at(Duration::ZERO), (1000.0, 1000.0));
    assert_eq!(oracle.at(Duration::from_millis(200)), (1000.5, 1000.0));
    assert_eq!(oracle.at(WALK_HOLD), (1001.0, 1000.0));
    assert_eq!(oracle.at(WALK_HOLD * 10), (1010.0, 1000.0));
    // And the speed is constant across every one of the ten, which is the whole
    // claim: no dwell at a tile boundary, no catch-up after one.
    for step in 0..10 {
        let quarter = WALK_HOLD * step + WALK_HOLD / 4;
        assert!((oracle.at(quarter).0 - (1000.25 + f64::from(step))).abs() < 1e-9);
    }
}

/// A body facing north, asked to go east, is walking east this instant — it does
/// not stand still for a hold first. See the module docs.
#[test]
fn the_oracle_charges_nothing_for_the_turn() {
    let oracle = Oracle::build(START, &ten_steps_east(), Duration::from_millis(4_000));
    // The same walk as above, and the body was facing the other way when the
    // key went down. It makes no difference: the turn is a packet, not a wait.
    assert_eq!(oracle.at(WALK_HOLD / 2), (1000.5, 1000.0));
    assert_eq!(oracle.at(WALK_HOLD * 10), (1010.0, 1000.0));
}

/// The headline: on a perfect wire and a punctual loop, the drawn body *is* the
/// oracle.
#[test]
fn ten_steps_on_a_perfect_wire_are_the_oracle() {
    let script = ten_steps_east();
    let until = Duration::from_millis(4_000);
    let oracle = Oracle::build(START, &script, until);
    let mut sim = Sim::new(Direction::East, Net::default(), 1, Vec::new());
    sim.run(&script, until);

    tracks(&sim, &oracle, 0.02);
    assert_eq!(sim.refused, 0, "an open field refuses nothing");
    assert_eq!(sim.shard.position, Point::new(1010, 1000, 0));
}

/// The one that pays for the prediction: a third of a second of latency, and
/// jitter on top of it, must not reach the screen at all. The corridor is the
/// same as the perfect wire's.
#[test]
fn latency_and_jitter_do_not_reach_the_screen() {
    let script = ten_steps_east();
    let until = Duration::from_millis(4_000);
    let oracle = Oracle::build(START, &script, until);
    for seed in 0..8 {
        let net = Net {
            latency: Duration::from_millis(150),
            jitter: Duration::from_millis(60),
            wake_jitter: Duration::ZERO,
        };
        let mut sim = Sim::new(Direction::East, net, seed, Vec::new());
        sim.run(&script, until);
        tracks(&sim, &oracle, 0.02);
        assert_eq!(
            sim.refused, 0,
            "seed {seed}: a walk at the hold is not a speedhack"
        );
    }
}

/// The pace the client asks at is the shard's business, and the shard here is
/// the real `Walker` with the real bucket. Running is the fast end of what a
/// player can legitimately ask for, so it is what the anti-speedhack floor is
/// tested against.
#[test]
fn running_the_whole_way_is_never_refused_as_a_speedhack() {
    let script = vec![
        Act {
            at: Duration::ZERO,
            input: Input::Running(true),
        },
        press(0, Direction::East),
        release(4_000),
    ];
    let until = Duration::from_millis(4_000);
    let oracle = Oracle::build(START, &script, until);
    let net = Net {
        latency: Duration::from_millis(80),
        jitter: Duration::from_millis(40),
        wake_jitter: Duration::ZERO,
    };
    let mut sim = Sim::new(Direction::East, net, 7, Vec::new());
    sim.run(&script, until);

    tracks(&sim, &oracle, 0.02);
    assert_eq!(sim.refused, 0, "twenty steps at RUN_HOLD");
    assert_eq!(sim.shard.position, Point::new(1020, 1000, 0));
}

/// The complaint this rule came from: the body is facing one way and the player
/// presses another. There is a turn in front of the walk, and it must cost
/// nothing — the oracle is moving from the first millisecond, and so is the body.
#[test]
fn a_walk_that_starts_with_a_turn_leaves_at_once() {
    let script = ten_steps_east();
    let until = Duration::from_millis(4_000);
    let oracle = Oracle::build(START, &script, until);
    let net = Net {
        latency: Duration::from_millis(120),
        jitter: Duration::from_millis(40),
        wake_jitter: Duration::ZERO,
    };
    // Facing north, asked for east.
    let mut sim = Sim::new(Direction::North, net, 5, Vec::new());
    sim.run(&script, until);

    tracks(&sim, &oracle, 0.02);
    assert_eq!(sim.refused, 0, "a turn is not charged to the pace budget");
    assert_eq!(
        sim.shard.position,
        Point::new(1010, 1000, 0),
        "ten tiles, not nine"
    );
}

/// A walk that changes direction: five east, then five south-east. The turn
/// costs its hold on both timelines, and the diagonal is one step like any
/// other.
#[test]
fn a_walk_that_turns_tracks_the_oracle_through_the_turn() {
    let script = vec![
        press(0, Direction::East),
        release(2_000),
        press(2_000, Direction::SouthEast),
        release(4_800),
    ];
    let until = Duration::from_millis(4_800);
    let oracle = Oracle::build(START, &script, until);
    let net = Net {
        latency: Duration::from_millis(120),
        jitter: Duration::from_millis(30),
        wake_jitter: Duration::ZERO,
    };
    let mut sim = Sim::new(Direction::East, net, 3, Vec::new());
    sim.run(&script, until);

    tracks(&sim, &oracle, 0.02);
    assert_eq!(sim.refused, 0);
}

/// The event loop woken late is the one thing prediction cannot hide: the step
/// is asked for when the loop wakes, so a loop that is 20ms late is a body 20ms
/// behind. What must not happen is that the lateness *accumulates* — ten steps
/// each armed from a late wake would leave the body a fifth of a tile behind
/// for ever, and a hundred steps two whole tiles.
#[test]
fn wake_up_jitter_does_not_accumulate() {
    // Forty steps rather than ten, because accumulation is the whole question:
    // the few milliseconds a late wake costs are invisible in one step and a
    // whole tile by the fortieth. A corridor a walk of ten fits down is not an
    // assertion about drift.
    let until = Duration::from_millis(16_000);
    let script = vec![press(0, Direction::East), release(16_000)];
    let oracle = Oracle::build(START, &script, until);
    let late = Duration::from_millis(20);
    for seed in 0..8 {
        let net = Net {
            latency: Duration::from_millis(50),
            jitter: Duration::ZERO,
            wake_jitter: late,
        };
        let mut sim = Sim::new(Direction::East, net, seed, Vec::new());
        sim.run(&script, until);
        // Three late wakes' worth of tile and no more, ever. Three because the
        // lateness lands in three places and none of them is the client's to
        // fix: the loop wakes late to *ask* for the step, it wakes late again to
        // fold the prediction the net task sent back, and what is left is the
        // difference between the hold the body is glided over and the interval
        // the steps actually left at. What matters is that the three do not add
        // up over forty steps, which is what arming the next step from the
        // deadline rather than from the wake is for.
        let corridor = 3.0 * late.as_secs_f64() / WALK_HOLD.as_secs_f64() + 0.02;
        tracks(&sim, &oracle, corridor);
    }
}

/// A wall the client cannot see is walked into, and the refusal is a rollback:
/// the body is *put* back rather than glided back, and the walk goes on.
///
/// The oracle has nothing to say here — it does not know about the wall — so
/// what is asserted is the two things a rollback must not do: draw the body
/// walking backwards, and leave it anywhere but where the shard says.
#[test]
fn a_refusal_puts_the_body_back_without_walking_it_back() {
    let script = ten_steps_east();
    let until = Duration::from_millis(4_000);
    let net = Net {
        latency: Duration::from_millis(100),
        jitter: Duration::ZERO,
        wake_jitter: Duration::ZERO,
    };
    let mut sim = Sim::new(Direction::East, net, 11, vec![(1004, 1000)]);
    sim.run(&script, until);

    assert!(sim.refused > 0, "the wall is on the way");
    assert_eq!(
        sim.shard.position,
        Point::new(1003, 1000, 0),
        "the shard stopped it at the wall"
    );
    let (_, last) = *sim.trace.last().unwrap();
    assert!(
        (last.0 - 1003.0).abs() < 0.01 && (last.1 - 1000.0).abs() < 0.01,
        "the screen agrees with the shard: {last:?}"
    );
    // A held arrow against a wall keeps asking, so the body keeps leaning into
    // the tile it cannot have and keeps being put back — that is the player's
    // own doing and it is what the 2D client does too. The two things that must
    // not happen are that it is drawn *past* the wall, and that the way back is
    // *walked*: a rollback is a snap, and gliding into it would draw the
    // character strolling backwards a whole tile. Backwards in one frame is the
    // snap; backwards in two consecutive frames is a glide.
    let mut back_to_back = 0;
    for pair in sim.trace.windows(2) {
        let ((_, before), (when, after)) = (pair[0], pair[1]);
        let moved = after.0 - before.0;
        assert!(moved > -1.01, "jumped a whole tile backwards at {when:?}");
        back_to_back = match moved < -0.001 {
            true => back_to_back + 1,
            false => 0,
        };
        assert!(back_to_back < 2, "walked backwards at {when:?}");
    }
    assert!(
        sim.trace.iter().all(|(_, drawn)| drawn.0 <= 1004.0),
        "drawn past the wall"
    );
}
