//! The shard, on a thread of its own.
//!
//! A window's event loop is not async and a socket is, so the two meet through
//! a channel in each direction: keys go down as a [`Facing`] to step, and what
//! the server says comes back as a [`Update`] the event loop is woken for.
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

use std::sync::Arc;

use openshard_client_net::connection::Event;
use openshard_client_net::session::Plan;
use openshard_client_net::transport::{Dial, enter_world_with};
use openshard_client_net::view::WorldView;
use openshard_client_net::walk::{Moved, Walk};
use openshard_protocol::direction::Facing;
use openshard_protocol::version::ClientVersion;
use openshard_uofiles::map::Map;
use winit::event_loop::EventLoopProxy;

/// What the shard thread tells the window.
#[derive(Clone, Debug)]
pub enum Update {
    /// The world as it now stands. Sent whenever a packet changed anything —
    /// whole rather than as a delta, because a renderer wants what to draw and
    /// not what moved.
    World(Box<WorldView>),
    /// The connection ended, and why. Nothing further will arrive.
    ///
    /// The window stays open on one of these: a client that vanished when a
    /// shard restarted would take the reason with it.
    Lost(String),
}

/// The handle the window keeps: somewhere to send steps.
///
/// Dropping it closes the command channel, which is what ends the thread's
/// loop when the window goes away.
#[derive(Debug)]
pub struct Link {
    steps: tokio::sync::mpsc::UnboundedSender<Facing>,
}

impl Link {
    /// Ask the shard for one step. Unanswered until an `Update` says otherwise.
    ///
    /// A closed channel is ignored rather than reported: it means the shard
    /// thread has already ended, and it has already said why.
    pub fn step(&self, facing: Facing) {
        let _ = self.steps.send(facing);
    }
}

/// Log in on a thread of its own, and report back through `proxy`.
///
/// Returns as soon as the thread is spawned: the login conversation is several
/// round trips and a window that waited for it would open blank and frozen.
///
/// The map comes along because the walk predicts a height and the server does
/// not send one — see [`Walk::step`]. Shared rather than loaded twice: it is a
/// few hundred megabytes of plain data, read by both threads and written by
/// neither.
///
/// `dial` is how the connection is opened and the only thing here that knows
/// what a socket is: `Tcp` for a shard on a network, and something else for one
/// in this process. It is moved onto the thread, so it is `Send`.
pub fn connect<D: Dial + Send + 'static>(
    dial: D,
    plan: Plan,
    version: ClientVersion,
    map: Arc<Map>,
    proxy: EventLoopProxy<Update>,
) -> Link {
    let (steps, commands) = tokio::sync::mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name("shard".to_owned())
        .spawn(move || run(dial, plan, version, &map, &proxy, commands))
        // The thread is the connection; a client that could not spawn it has
        // nothing to fall back to, and the OS refusing a thread at startup is
        // not a condition worth a variant in `Update`.
        .expect("the shard thread starts");
    Link { steps }
}

/// The thread body: one runtime, one login, then packets and steps until either
/// end stops.
fn run<D: Dial>(
    dial: D,
    plan: Plan,
    version: ClientVersion,
    map: &Map,
    proxy: &EventLoopProxy<Update>,
    commands: tokio::sync::mpsc::UnboundedReceiver<Facing>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            report(proxy, Update::Lost(format!("no runtime for the shard: {error}")));
            return;
        }
    };
    runtime.block_on(async move {
        let reason = play(dial, plan, version, map, proxy, commands).await;
        report(proxy, Update::Lost(reason));
    });
}

/// Everything after the runtime exists, up to the reason it ended.
async fn play<D: Dial>(
    dial: D,
    plan: Plan,
    version: ClientVersion,
    map: &Map,
    proxy: &EventLoopProxy<Update>,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<Facing>,
) -> String {
    let (mut socket, mut view) = match enter_world_with(dial, plan, version).await {
        Ok(entered) => entered,
        Err(error) => return error.to_string(),
    };
    // Where the server put us, which is where the next `0x02` is computed from.
    let mut walk = Walk::new(view.player.position, view.player.facing);
    report(proxy, Update::World(Box::new(view.clone())));

    loop {
        tokio::select! {
            // Cancel-safe on both arms: `read` loses no bytes when the other
            // branch wins, and an unbounded receiver loses no messages.
            event = socket.next_event() => {
                let packet = match event {
                    Ok(Some(Event::Packet(packet))) => packet,
                    // A packet with no decoder yet, or one added since this was
                    // written: framing already said where the next one starts.
                    Ok(Some(_)) => continue,
                    Ok(None) => return "the shard closed the connection".to_owned(),
                    Err(error) => return error.to_string(),
                };
                let changed = match fold(&mut view, &mut walk, &packet) {
                    Ok(changed) => changed,
                    // The ends have lost track of each other and only the
                    // server can repair it. Reported rather than guessed at.
                    Err(error) => return error.to_string(),
                };
                if changed {
                    report(proxy, Update::World(Box::new(view.clone())));
                }
            }
            step = commands.recv() => {
                // `None` is the window closing: the `Link` was dropped.
                let Some(facing) = step else {
                    return "the window closed".to_owned();
                };
                // The land under the target: without it every step predicts
                // the height it started at, and a body drawn below the terrain
                // is hidden by it — which looks exactly like one that failed to
                // draw. The server lands the step on the ground and says
                // nothing, since a `0x22` carries no position.
                match walk.step(facing, |x, y| map.land(x, y).map(|cell| cell.z)) {
                    Ok(bytes) => {
                        if let Err(error) = socket.send(&bytes).await {
                            return error.to_string();
                        }
                    }
                    // A step off the edge of the map. The server would refuse it
                    // too; not sending it saves the round trip and the rollback.
                    Err(edge) => tracing::debug!(%edge, "not stepping"),
                }
            }
        }
    }
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
    view: &mut WorldView,
    walk: &mut Walk,
    packet: &openshard_protocol::server_packet::ServerPacket,
) -> Result<bool, openshard_client_net::walk::UnexpectedAck> {
    let mut changed = view.apply(packet);
    match walk.on_packet(packet)? {
        Moved::Stepped { position, facing, .. } | Moved::Snapped { position, facing } => {
            changed |= view.player_stepped(position, facing);
        }
        Moved::Idle => {}
    }
    Ok(changed)
}

/// Wake the event loop with an update, unless it has already gone.
fn report(proxy: &EventLoopProxy<Update>, update: Update) {
    let _ = proxy.send_event(update);
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
        assert!(fold(&mut view, &mut walk, &ack).unwrap());
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
        assert!(!fold(&mut view, &mut walk, &reject).unwrap());
        assert_eq!(
            view.player.position,
            Point::new(100, 100, 0),
            "the refused step never happened"
        );
    }

    #[test]
    fn an_ack_for_a_step_nobody_took_is_an_error() {
        // The two ends have lost track of each other. Nothing local repairs
        // that, so the thread reports it rather than guessing.
        let (mut view, mut walk) = entered();
        let ack = ServerPacket::WalkAck(WalkAck {
            sequence: StepSequence(3),
            notoriety: Notoriety::Innocent,
        });
        assert!(fold(&mut view, &mut walk, &ack).is_err());
    }
}
