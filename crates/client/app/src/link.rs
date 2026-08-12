//! The shard, on a thread of its own.
//!
//! A window's event loop is not async and a socket is, so the two meet through
//! a channel in each direction: what the player does goes out as a [`Command`],
//! and what the server says comes back as a [`Update`] the event loop is woken
//! for.
//!
//! Nothing about the protocol is decided here. `client/net` owns the login
//! conversation, the walk handshake and the [`WorldView`]; this file owns the
//! thread they run on, and the rule that the renderer never sees a half-applied
//! packet — a snapshot is published after the whole of one has been folded in.
//!
//! # Why a thread and not a runtime in the event loop
//!
//! The event loop blocks on the compositor and the runtime blocks on the
//! socket, and neither can be asked to poll the other: a frame must not wait on
//! a packet, and a packet must not wait for the window to be uncovered. So the
//! socket gets a current-thread runtime of its own and the two exchange values.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use openshard_client_net::action::Outgoing;
use openshard_client_net::connection::Event;
use openshard_client_net::session::Plan;
use openshard_client_net::transport::{Dial, enter_world_with};
use openshard_client_net::view::WorldView;
use openshard_client_net::walk::{Moved, Walk};
use openshard_protocol::direction::Facing;
use openshard_protocol::feedback::Animation;
use openshard_protocol::gump::GumpId;
use openshard_protocol::serial::Serial;
use openshard_protocol::version::ClientVersion;
use openshard_protocol::world::ResyncRequest;
use openshard_uofiles::map::Map;
use openshard_uofiles::tiledata::TileData;

/// Where this client's own body is *drawn*, which is not where the
/// [`WorldView`] says it is.
///
/// The view is the record of what the server said, and the server says where a
/// step landed only once it has acked it — a round trip after the player asked.
/// Waiting for that is what makes a walk lag and stutter: the body stands still
/// for the latency, then crosses its tile, then stands still again.
///
/// So the picture runs on [`Walk::predicted`] instead: the tile the last `0x02`
/// asked for, which this end knows the instant it sends one. The two agree on
/// every step the server allows, and where it does not — a `0x21`, or a `0x20`
/// putting the body somewhere it did not walk to — the prediction is thrown away
/// and replaced by the server's word, which is the rollback and is flagged as
/// one.
///
/// Deliberately *not* done by moving the view: a record of what arrived that
/// contained a guess would have no way left to tell the two apart, which is the
/// argument in `client/net`'s `walk` module docs. The guess travels beside it.
#[derive(Clone, Copy, Debug)]
pub struct Body {
    /// The tile and facing to draw, ahead of the server's confirmation.
    pub predicted: openshard_client_net::walk::Predicted,
    /// Whether it got there by a correction rather than by walking.
    ///
    /// A correction is *jumped* to and never glided: the body is not walking
    /// back the tile it mispredicted, it was never there. It also ends the pace
    /// measurement — the gap between a step and a rollback is not a walking
    /// speed. See [`crate::crowd::Crowd::snap`].
    pub corrected: bool,
}

/// What the shard thread tells the window.
#[derive(Clone, Debug)]
pub enum Update {
    /// The world as it now stands. Sent whenever a packet changed anything —
    /// whole rather than as a delta, because a renderer wants what to draw and
    /// not what moved.
    World {
        /// What the server has said, entire.
        view: Box<WorldView>,
        /// Where our own body is drawn. See [`Body`].
        body: Body,
    },
    /// A decoded server packet. The event-loop thread applies it to its sole
    /// `WorldView` owner and then rebuilds the presentation projection.
    Mutation {
        packet: openshard_protocol::server_packet::ServerPacket,
        body: Body,
    },
    /// A locally accepted walk, before the server acknowledges it.
    Prediction(Body),
    /// The server asked one mobile to play a one-shot body animation.
    Animation(Animation),
    /// The connection ended, and why. Nothing further will arrive.
    ///
    /// The window stays open on one of these: a client that vanished when a
    /// shard restarted would take the reason with it.
    Lost(String),
}

const MAX_ORDERED_UPDATES: usize = 256;
const COMMAND_CAPACITY: usize = 16;

/// Updates crossing from the shard thread to the application thread.
///
/// A network mutation is a fact in a sequence and is never merged with another
/// one. A prediction, by contrast, is only the newest answer to "where should
/// the next frame draw our body?"; while the application is busy, keeping each
/// older answer turns a delayed redraw into a visible catch-up animation. The
/// mailbox therefore retains mutation order, while coalescing consecutive
/// predictions within their own stage.
///
/// The producer asks the platform loop to wake only when this mailbox changes
/// from idle to non-idle. The loop drains it as one staged batch, rather than
/// carrying one platform user event per packet or frame update.
#[derive(Clone)]
pub struct Updates {
    mailbox: Arc<UpdateMailbox>,
}

struct UpdateMailbox {
    pending: Mutex<PendingUpdates>,
    /// Wakes the shard thread once the application has made room for another
    /// ordered update.
    space: Condvar,
    capacity: usize,
}

#[derive(Default)]
struct PendingUpdates {
    /// Whether a platform wake-up is already in flight for this batch.
    notified: bool,
    /// Every update whose order must be retained, across all mutation stages.
    ordered: usize,
    stages: VecDeque<UpdateStage>,
}

enum UpdateStage {
    /// Facts whose order is part of their meaning.
    Ordered(VecDeque<Update>),
    /// A latest-value frame update between two mutation boundaries.
    Prediction(Body),
}

impl Updates {
    /// Start an empty staged mailbox.
    pub fn new() -> Self {
        Self::with_capacity(MAX_ORDERED_UPDATES)
    }

    fn with_capacity(capacity: usize) -> Self {
        Self {
            mailbox: Arc::new(UpdateMailbox {
                pending: Mutex::new(PendingUpdates::default()),
                space: Condvar::new(),
                capacity,
            }),
        }
    }

    /// Publish an update and say whether the caller must wake the application
    /// loop. The caller owns the actual platform wake-up, so this module stays
    /// independent of `winit`.
    pub fn publish(&self, update: Update) -> bool {
        let mut pending = self
            .mailbox
            .pending
            .lock()
            .expect("the update mailbox is not poisoned");
        match update {
            Update::Prediction(body) => match pending.stages.back_mut() {
                Some(UpdateStage::Prediction(previous)) => *previous = body,
                _ => pending.stages.push_back(UpdateStage::Prediction(body)),
            },
            update => {
                // Mutations cannot be merged or dropped. Stopping the socket
                // reader here applies backpressure all the way to TCP instead
                // of allowing an unfocused or GPU-blocked window to consume
                // unbounded memory while it falls behind.
                while pending.ordered == self.mailbox.capacity {
                    pending = self
                        .mailbox
                        .space
                        .wait(pending)
                        .expect("the update mailbox is not poisoned");
                }
                pending.ordered += 1;
                match pending.stages.back_mut() {
                    Some(UpdateStage::Ordered(updates)) => updates.push_back(update),
                    _ => pending
                        .stages
                        .push_back(UpdateStage::Ordered(VecDeque::from([update]))),
                }
            }
        }
        if pending.notified {
            false
        } else {
            pending.notified = true;
            true
        }
    }

    /// Take every update staged before this call, in its original semantic
    /// order. Clearing `notified` while holding the lock closes the race with a
    /// producer that arrives between this drain and the next platform wait.
    pub fn take(&self) -> Vec<Update> {
        let mut pending = self
            .mailbox
            .pending
            .lock()
            .expect("the update mailbox is not poisoned");
        pending.notified = false;
        pending.ordered = 0;
        let stages = std::mem::take(&mut pending.stages);
        self.mailbox.space.notify_all();
        stages
            .into_iter()
            .flat_map(|stage| match stage {
                UpdateStage::Ordered(updates) => updates,
                UpdateStage::Prediction(body) => VecDeque::from([Update::Prediction(body)]),
            })
            .collect()
    }
}

impl Default for Updates {
    fn default() -> Self {
        Self::new()
    }
}

pub use openshard_client_net::action::GumpReply;

/// What the window asks the shard thread to send.
///
/// One variant per thing a player can do that leaves this process. Open rather
/// than a bare `Facing` because the three are unrelated: a step is answered by
/// the walk handshake, a line of speech is answered by everyone in earshot
/// hearing it, and a dialog answer is answered by whatever the shard does about
/// it. Nothing here is a packet yet — the thread builds those, so this side
/// never touches the wire.
#[derive(Clone, Debug)]
pub enum Command {
    /// Take one step, or turn.
    Step(Facing),
    /// An ordinary network action. Its packet mapping is owned by `client-net`.
    Outgoing(Outgoing),
}

/// Which of a locally-closed window's state [`Command::CloseWindow`] drops
/// from this thread's [`WorldView`].
///
/// One variant per kind [`WorldView`] itself distinguishes a close for — see
/// [`WorldView::paperdoll_closed`], [`WorldView::container_closed`] and
/// [`WorldView::gump_closed`]. Not [`WindowSubject`][crate::WindowSubject]:
/// that type also names a skills tree, which is this client's own state and
/// has nothing in the view to forget.
#[derive(Clone, Copy, Debug)]
pub enum CloseTarget {
    /// A paperdoll, named by the mobile it draws.
    Paperdoll(Serial),
    /// A container, named by its own serial.
    Container(Serial),
    /// A dialog, named by the gump id the shard opened it under.
    Gump(GumpId),
}

/// The handle the window keeps: somewhere to send commands.
///
/// Dropping it closes the command channel, which is what ends the thread's
/// loop when the window goes away.
#[derive(Debug)]
pub struct Link {
    commands: tokio::sync::mpsc::Sender<Command>,
}

impl Link {
    /// Queue one command without making the window event loop wait for a slow
    /// socket. The walking controller already rate-limits steps and the other
    /// commands are button presses, so reaching this bound means the shard task
    /// cannot currently make progress; keeping an unbounded backlog would only
    /// replay stale input later. The server remains authoritative either way.
    fn send(&self, command: Command) {
        match self.commands.try_send(command) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!("shard command queue is full; dropping stale input");
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {}
        }
    }

    /// Ask the shard for one step. Unanswered until an `Update` says otherwise.
    ///
    /// A closed channel is ignored rather than reported: it means the shard
    /// thread has already ended, and it has already said why. The same holds
    /// for everything below.
    pub fn step(&self, facing: Facing) {
        self.send(Command::Step(facing));
    }

    /// Say a line out loud.
    pub fn say(&self, text: String) {
        self.send(Command::Outgoing(Outgoing::Say(text)));
    }

    /// Answer an open dialog.
    pub fn answer_gump(&self, reply: GumpReply) {
        self.send(Command::Outgoing(Outgoing::AnswerGump(reply)));
    }

    /// Use an object — the double-click.
    pub fn use_object(&self, serial: Serial) {
        self.send(Command::Outgoing(Outgoing::Use(serial)));
    }

    /// Ask for a stance. See [`Outgoing::WarMode`].
    pub fn war_mode(&self, war: bool) {
        self.send(Command::Outgoing(Outgoing::WarMode(war)));
    }

    /// Aim at a mobile. See [`Outgoing::Attack`].
    pub fn attack(&self, mobile: Serial) {
        self.send(Command::Outgoing(Outgoing::Attack(mobile)));
    }

    /// Announce that the player is leaving.
    pub fn log_out(&self) {
        self.send(Command::Outgoing(Outgoing::LogOut));
    }

    /// Ask for a mobile's status bar.
    pub fn status(&self, mobile: Serial) {
        self.send(Command::Outgoing(Outgoing::Status(mobile)));
    }

    /// Ask for a mobile's skill list.
    pub fn skills(&self, mobile: Serial) {
        self.send(Command::Outgoing(Outgoing::Skills(mobile)));
    }

    /// Ask for our own quest log.
    pub fn quest_log(&self) {
        self.send(Command::Outgoing(Outgoing::QuestLog));
    }

    /// Ask for our own guild menu.
    pub fn guild_menu(&self) {
        self.send(Command::Outgoing(Outgoing::GuildMenu));
    }

    /// Ask about a mobile's virtues.
    pub fn virtue(&self, mobile: Serial) {
        self.send(Command::Outgoing(Outgoing::Virtue(mobile)));
    }

    /// Ask to set a skill's lock. See [`Outgoing::SkillLock`].
    pub fn set_skill_lock(
        &self,
        skill: openshard_protocol::wire::RawSkillId,
        lock: openshard_protocol::skill::SkillLock,
    ) {
        self.send(Command::Outgoing(Outgoing::SkillLock { skill, lock }));
    }

    /// Ask to use a skill — its own button, not the lock arrow.
    pub fn use_skill(&self, skill: openshard_protocol::wire::RawSkillId) {
        self.send(Command::Outgoing(Outgoing::UseSkill(skill)));
    }
}

/// Log in on a thread of its own, and report back through `proxy`.
///
/// Returns as soon as the thread is spawned: the login conversation is several
/// round trips and a window that waited for it would open blank and frozen.
///
/// The map and the tile definitions come along because the walk predicts a
/// height and the server does not send one — see [`Walk::step`], which needs
/// both: `tiledata.mul` is what tells a pier or a bridge's deck apart from
/// the water it stands over. Shared rather than loaded twice: plain data,
/// read by both threads and written by neither.
///
/// `dial` is how the connection is opened and the only thing here that knows
/// what a socket is: `Tcp` for a shard on a network, and something else for one
/// in this process. It is moved onto the thread, so it is `Send`.
pub fn connect<D, F>(
    dial: D,
    plan: Plan,
    version: ClientVersion,
    map: Arc<Map>,
    tiles: Arc<TileData>,
    report: F,
) -> Link
where
    D: Dial + Send + 'static,
    F: Fn(Update) + Send + 'static,
{
    let (sender, commands) = tokio::sync::mpsc::channel(COMMAND_CAPACITY);
    std::thread::Builder::new()
        .name("shard".to_owned())
        .spawn(move || run(dial, plan, version, &map, &tiles, &report, commands))
        // The thread is the connection; a client that could not spawn it has
        // nothing to fall back to, and the OS refusing a thread at startup is
        // not a condition worth a variant in `Update`.
        .expect("the shard thread starts");
    Link { commands: sender }
}

/// The thread body: one runtime, one login, then packets and steps until either
/// end stops.
fn run<D: Dial, F: Fn(Update) + Send>(
    dial: D,
    plan: Plan,
    version: ClientVersion,
    map: &Map,
    tiles: &TileData,
    report: &F,
    commands: tokio::sync::mpsc::Receiver<Command>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            report(Update::Lost(format!("no runtime for the shard: {error}")));
            return;
        }
    };
    runtime.block_on(async move {
        let reason = play(dial, plan, version, map, tiles, report, commands).await;
        report(Update::Lost(reason));
    });
}

/// Everything after the runtime exists, up to the reason it ended.
async fn play<D: Dial, F: Fn(Update) + Send>(
    dial: D,
    plan: Plan,
    version: ClientVersion,
    map: &Map,
    tiles: &TileData,
    report: &F,
    mut commands: tokio::sync::mpsc::Receiver<Command>,
) -> String {
    let (mut socket, view) = match enter_world_with(dial, plan, version).await {
        Ok(entered) => entered,
        Err(error) => return error.to_string(),
    };
    // Where the server put us, which is where the next `0x02` is computed from.
    let mut walk = Walk::new(view.player.position, view.player.facing);
    let player_serial = view.player.serial;
    // Entering the world is not a step, so the body is placed rather than walked
    // there — the same statement a rollback makes.
    report(snapshot(view, &walk, true));

    loop {
        tokio::select! {
            // Cancel-safe on both arms: `read` loses no bytes when the other
            // branch wins. The bounded command receiver applies backpressure
            // at the window boundary instead of growing without limit.
            event = socket.next_event() => {
                let packet = match event {
                    Ok(Some(Event::Packet(packet))) => packet,
                    // A packet with no decoder yet, or one added since this was
                    // written: framing already said where the next one starts.
                    Ok(Some(_)) => continue,
                    Ok(None) => return "the shard closed the connection".to_owned(),
                    Err(error) => return error.to_string(),
                };
                // "You may go." The shard answers the paperdoll's Log Out button
                // with this and then leaves the character standing until the
                // socket closes — closing it is the client's half, and both
                // references do it here. Nothing after this packet is worth
                // reading, so the loop ends and the window is told why.
                if matches!(packet, openshard_protocol::server_packet::ServerPacket::LogoutAck(_)) {
                    return "logged out".to_owned();
                }
                // Before folding, because folding is what sets it: asking twice
                // for the same disagreement is the burst ClassicUO's
                // `ResendPacketResync` guards against.
                let was_out_of_step = walk.out_of_step();
                let folded = match fold(&mut walk, &packet) {
                    Ok(folded) => folded,
                    // The two ends have lost track of each other over the walk,
                    // and this end cannot repair it: the ack names a step it is
                    // not holding, and guessing which one was meant would turn a
                    // diagnosable desync into a silent one.
                    //
                    // What it is *not* is a reason to close the window. It used
                    // to be, and the ordinary answers to steps a rollback had
                    // voided reached here — so a wall and a slow link dropped the
                    // player's own connection. Those are counted off in `Walk`
                    // now, and what is left is a genuine disagreement, which has
                    // an answer on the wire: ask where we are. `Walk` has already
                    // stopped sending steps; this is the other half of that, and
                    // it has to happen or the walk never starts again.
                    Err(desync) => {
                        if !was_out_of_step {
                            tracing::warn!(%desync, "the walk is out of step: asking for a resync");
                            if let Err(error) = socket.send(&ResyncRequest.encode()).await {
                                return error.to_string();
                            }
                        }
                        continue;
                    }
                };
                if let openshard_protocol::server_packet::ServerPacket::Animation(animation) = packet {
                    report(Update::Animation(animation));
                }
                // A correction is worth sending even when the view is unchanged:
                // the view never held the prediction, so rolling one back moves
                // the *drawn* body and nothing else.
                report(Update::Mutation {
                    packet,
                    body: Body {
                        predicted: walk.predicted(),
                        corrected: folded.corrected,
                    },
                });
            }
            command = commands.recv() => {
                // `None` is the window closing: the `Link` was dropped.
                let Some(command) = command else {
                    return "the window closed".to_owned();
                };
                // Every command becomes bytes here and nowhere else: the window
                // side asks for a step or a line, and what that is on the wire
                // is this thread's business.
                let bytes = match command {
                    Command::Step(facing) => {
                        // The surface under the target: without it every step predicts
                        // the height it started at, and a body drawn below the terrain
                        // is hidden by it — which looks exactly like one that failed to
                        // draw. The server lands the step wherever a body actually
                        // stands — the ground, or a platform static — and says
                        // nothing, since a `0x22` carries no position.
                        //
                        // `MapTerrain::predict_step` is the shard's own step rule run
                        // on this end: it weighs the land's *average* (the same number
                        // the shard's own `ground_z` computes — on a slope the raw
                        // corner differs by most of the tile's relief, and a body
                        // predicted at the corner is drawn sunk into the hill and
                        // sorted behind it) against every platform static on the tile,
                        // a pier's or a bridge's deck among them, reaching from the top
                        // of the surface underfoot and standing on the highest surface
                        // within a step. That last part is what climbs a staircase, and
                        // it is why this is not `predict_z`: the nearest-height guess
                        // stays on the floor a stair tile also carries, and the body
                        // walks *through* the stairs while the shard has it half way
                        // up. Never a refusal — see `predict_step`'s own doc — so it
                        // cannot desync from a server that disagrees; it can only draw
                        // the wrong deck for one step, corrected by the next `0x20`.
                        let terrain = openshard_movement::MapTerrain::new(map, tiles);
                        match walk.step(facing, |from, tile| {
                            i8::try_from(terrain.predict_step(from, tile.x, tile.y)).ok()
                        }) {
                            Ok(bytes) => {
                                // The body moves *now*, on this end's own
                                // prediction, rather than a round trip later
                                // when the `0x22` says it may. That is the whole
                                // of the lag compensation: the ack changes
                                // nothing on screen, and only a refusal does.
                                report(Update::Prediction(Body {
                                    predicted: walk.predicted(),
                                    corrected: false,
                                }));
                                bytes
                            }
                            // A step this end refused on its own: the edge of the
                            // map, which the server would refuse too, or a shard
                            // that has stopped answering and is five steps behind
                            // already. Neither is worth a round trip, and the
                            // body simply stays where it is.
                            Err(refusal) => {
                                tracing::debug!(%refusal, "not stepping");
                                continue;
                            }
                        }
                    }
                    Command::Outgoing(action) => action.encode(player_serial),
                };
                if let Err(error) = socket.send(&bytes).await {
                    return error.to_string();
                }
            }
        }
    }
}

/// The world and the body to draw, together, at one instant.
///
/// One function so the two can never be published out of step: the view is
/// cloned and the prediction read in the same breath, which is what "the
/// renderer never sees a half-applied packet" means once the body is drawn
/// ahead of the view.
fn snapshot(view: WorldView, walk: &Walk, corrected: bool) -> Update {
    Update::World {
        view: Box::new(view),
        body: Body {
            predicted: walk.predicted(),
            corrected,
        },
    }
}

/// What one packet did: whether the world changed, and whether the prediction
/// was thrown away.
///
/// Two answers rather than one because they are independent — a `0x21` that
/// rolls the body back to where the *view* already had it changes nothing in the
/// view and everything on screen.
struct Folded {
    /// The server put the body somewhere: whatever was predicted is void.
    corrected: bool,
}

/// One packet into both records of where we are, answering whether anything
/// the window draws has changed.
///
/// The whole rule of this file, and the only part of it worth a test: a
/// [`WorldView`] does not learn its own body's position from a `0x22` or a
/// `0x21`, because neither packet carries one. A `0x22` names a sequence and
/// [`Walk`] is what knows which tile that step was asking for; a `0x21` is a
/// rollback to what the server says, and the view has no arm for either. Fold
/// only one of the two and the client's own body stands still while everyone
/// else moves around it.
fn fold(
    walk: &mut Walk,
    packet: &openshard_protocol::server_packet::ServerPacket,
) -> Result<Folded, openshard_client_net::walk::UnexpectedAck> {
    let mut corrected = false;
    match walk.on_packet(packet)? {
        Moved::Stepped { .. } => {}
        Moved::Snapped { .. } => {
            corrected = true;
        }
        Moved::Idle => {}
    }
    Ok(Folded { corrected })
}

#[cfg(test)]
mod tests {
    use openshard_protocol::direction::Direction;
    use openshard_protocol::mobile::Notoriety;
    use openshard_protocol::serial::Serial;
    use openshard_protocol::server_packet::ServerPacket;
    use openshard_protocol::wire::Graphic;
    use openshard_protocol::world::{MapSize, PlayerStart, Point, StepSequence, WalkAck, WalkReject};

    use super::*;

    fn entered() -> (WorldView, Walk) {
        let start = PlayerStart {
            serial: Serial::new(0x0000_002A).unwrap(),
            body: Graphic(0x0190),
            position: Point::new(100, 100, 0),
            facing: Facing::walking(Direction::North),
            map: MapSize::BRITANNIA,
        };
        let view = WorldView::entered(start);
        let walk = Walk::new(view.player.position, view.player.facing);
        (view, walk)
    }

    fn prediction(x: u16) -> Update {
        Update::Prediction(Body {
            predicted: openshard_client_net::walk::Predicted {
                position: Point::new(x, 100, 0),
                facing: Facing::walking(Direction::East),
            },
            corrected: false,
        })
    }

    #[test]
    fn a_busy_frame_keeps_only_its_newest_prediction() {
        let updates = Updates::new();
        assert!(
            updates.publish(prediction(101)),
            "the idle mailbox needs one wake-up"
        );
        assert!(
            !updates.publish(prediction(102)),
            "the wake-up already covers this frame"
        );

        let staged = updates.take();
        let [Update::Prediction(body)] = staged.as_slice() else {
            panic!("one latest prediction should remain");
        };
        assert_eq!(body.predicted.position, Point::new(102, 100, 0));
        assert!(
            updates.publish(prediction(103)),
            "a drained mailbox needs a new wake-up"
        );
    }

    #[test]
    fn mutations_stay_ordered_on_both_sides_of_a_prediction() {
        let updates = Updates::new();
        updates.publish(Update::Lost("before".to_owned()));
        updates.publish(prediction(101));
        updates.publish(Update::Lost("after".to_owned()));

        let staged = updates.take();
        assert!(matches!(&staged[0], Update::Lost(reason) if reason == "before"));
        assert!(
            matches!(&staged[1], Update::Prediction(body) if body.predicted.position == Point::new(101, 100, 0))
        );
        assert!(matches!(&staged[2], Update::Lost(reason) if reason == "after"));
    }

    #[test]
    fn ordered_delivery_waits_for_the_application_instead_of_growing() {
        let updates = Updates::with_capacity(1);
        updates.publish(Update::Lost("first".to_owned()));
        let producer = updates.clone();
        let (sent, received) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            sent.send(producer.publish(Update::Lost("second".to_owned())))
                .expect("the test is listening");
        });

        assert!(
            received
                .recv_timeout(std::time::Duration::from_millis(20))
                .is_err(),
            "the second ordered update must wait for capacity"
        );
        assert!(matches!(&updates.take()[0], Update::Lost(reason) if reason == "first"));
        assert!(
            received.recv_timeout(std::time::Duration::from_secs(1)).is_ok(),
            "draining must release the shard thread"
        );
        worker.join().expect("the shard-side publisher exits");
    }

    #[test]
    fn command_delivery_has_a_fixed_bound() {
        let (commands, mut received) = tokio::sync::mpsc::channel(1);
        let link = Link { commands };
        link.log_out();
        link.log_out();

        assert!(matches!(
            received.try_recv(),
            Ok(Command::Outgoing(Outgoing::LogOut))
        ));
        assert!(
            received.try_recv().is_err(),
            "the second command was not queued without limit"
        );
    }

    #[test]
    fn an_ack_moves_the_body_the_window_draws() {
        // The 0x22 carries a sequence and a health-bar colour and no position
        // at all, so the tile is the one `Walk` asked for. A client that only
        // fed packets to the view would walk on the server and stand still on
        // the screen.
        let (mut view, mut walk) = entered();
        walk.step(Facing::walking(Direction::North), |_, _| None).unwrap();

        let ack = ServerPacket::WalkAck(WalkAck {
            sequence: StepSequence(0),
            notoriety: Notoriety::Innocent,
        });
        let folded = fold(&mut walk, &ack).unwrap();
        view.apply(&ack);
        view.player_stepped(walk.predicted().position, walk.predicted().facing);
        assert!(!folded.corrected, "an allowed step is not a rollback");
        assert_eq!(view.player.position, Point::new(100, 99, 0));
    }

    #[test]
    fn a_rejection_puts_the_body_back_where_the_server_says() {
        // And the other direction: a 0x21 is the server disagreeing, and the
        // view has no arm for it — only `Walk` knows the step it undoes.
        let (mut view, mut walk) = entered();
        walk.step(Facing::walking(Direction::North), |_, _| None).unwrap();

        let reject = ServerPacket::WalkReject(WalkReject {
            sequence: StepSequence(0),
            position: Point::new(100, 100, 0),
            facing: Facing::walking(Direction::North),
        });
        let folded = fold(&mut walk, &reject).unwrap();
        view.apply(&reject);
        assert_eq!(view.player.position, Point::new(100, 100, 0));
        assert!(
            folded.corrected,
            "and the drawn body has to be told, or it stays a tile ahead for ever"
        );
        assert_eq!(
            walk.predicted().position,
            Point::new(100, 100, 0),
            "and the prediction is thrown away with it"
        );
    }

    /// The whole of the lag compensation, stated once: what is drawn is the
    /// prediction, and it is a tile ahead of the view for as long as the ack
    /// takes to arrive.
    #[test]
    fn a_step_is_predicted_before_the_server_has_answered() {
        let (view, mut walk) = entered();
        walk.step(Facing::walking(Direction::North), |_, _| None).unwrap();

        let Update::World {
            view: published,
            body,
        } = snapshot(view.clone(), &walk, false)
        else {
            panic!("a snapshot is a world");
        };
        assert_eq!(
            published.player.position,
            Point::new(100, 100, 0),
            "the view is still what the server said"
        );
        assert_eq!(
            body.predicted.position,
            Point::new(100, 99, 0),
            "and the body is drawn where the step asked to be"
        );
        assert!(!body.corrected);
    }

    #[test]
    fn an_ack_for_a_step_nobody_took_is_an_error() {
        // The two ends have lost track of each other. Nothing local repairs
        // that, so the thread reports it rather than guessing.
        let (_, mut walk) = entered();
        let ack = ServerPacket::WalkAck(WalkAck {
            sequence: StepSequence(3),
            notoriety: Notoriety::Innocent,
        });
        assert!(fold(&mut walk, &ack).is_err());
    }
}
