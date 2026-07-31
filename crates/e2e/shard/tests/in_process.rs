//! The same login, with no network under it.
//!
//! `enter_world` over TCP is covered next door. What this file is about is that
//! the transport is genuinely a parameter: the client's `Dial` and the gateway's
//! stream are the only two things that changed, and the whole conversation —
//! seed, `0xA8`, the relay, a second connection, the character list, `0x1B` and
//! the `0x55` that ends it — comes out the same over a pair of in-memory pipes.
//!
//! It is also the test that would catch the one thing this arrangement can get
//! wrong on its own: a hang. Nothing here binds a port, so there is no
//! "connection refused" to fail with — a gate that captured the wrong runtime,
//! or a write half that never shut down, would simply wait forever. Hence a
//! deadline on every step, and a walk at the end that needs bytes to travel in
//! both directions.

use std::time::Duration;

use openshard_client_net::connection::Event;
use openshard_client_net::transport::enter_world_with;
use openshard_client_net::walk::{Moved, Walk};
use openshard_protocol::direction::Facing;

use openshard_e2e_shard::{in_process, plan, stock_config, version};

/// Generous, and only ever paid by a failure: every wait below finishes the
/// moment its answer arrives. What it bounds is a hang, which is this
/// arrangement's characteristic way of being broken.
const WAIT: Duration = Duration::from_secs(20);

#[tokio::test]
async fn a_client_enters_the_world_with_no_socket_anywhere() {
    let dial = in_process::spawn(stock_config);

    let (mut socket, mut view) = tokio::time::timeout(WAIT, enter_world_with(dial, plan(), version()))
        .await
        .expect("the login conversation finished inside the deadline")
        .expect("the client reached the world");

    // The shard's own answer about who we are: a `0x1B` was decoded, which means
    // both connections carried real packets and the second one was compressed.
    assert!(
        view.player.serial.raw() != 0,
        "the world sent a body, so the whole login conversation happened"
    );

    // And bytes travel in both directions afterwards: one step, one ack, and the
    // client's own prediction of where it landed.
    let start = view.player.position;
    let mut walk = Walk::new(start, view.player.facing);
    let heading = Facing::walking(view.player.facing.direction);
    let request = walk.step(heading, |_, _| None).expect("room on the map to walk");
    socket.send(&request).await.expect("the shard is listening");

    let stepped = tokio::time::timeout(WAIT, async {
        while let Some(event) = socket.next_event().await.expect("the pipe stayed up") {
            let Event::Packet(packet) = event else {
                continue;
            };
            match walk
                .on_packet(&packet)
                .expect("the shard acked the step in order")
            {
                Moved::Stepped { position, facing, .. } => {
                    view.player_stepped(position, facing);
                    return position;
                }
                // A refusal here is a failure, not an oracle: one step is well
                // inside the pace budget, so a `0x21` would mean the two ends
                // disagree about a walk nothing has stressed yet.
                Moved::Snapped { position, .. } => panic!("the first step was refused, back to {position:?}"),
                Moved::Idle => {
                    view.apply(&packet);
                }
            }
        }
        panic!("the shard closed the connection without acking the step");
    })
    .await
    .expect("the shard answered the step inside the deadline");

    assert_ne!(stepped, start, "an acked step moves the body");
    assert_eq!(view.player.position, stepped, "and the view follows it");
}
