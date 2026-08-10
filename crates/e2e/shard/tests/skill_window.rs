//! The skill list, over a real socket, both ends.
//!
//! # Why this needs both ends
//!
//! `0x3A` is the packet where the id is not the whole answer: two different
//! packets share it and the byte after the length says which. Every unit test on
//! either side of that can pass while the client drops the answer — which is
//! exactly what it did before this window was built, and what `0x72` did before
//! the paperdoll's toggle was gated the same way. A framed packet with no arm in
//! `ServerPacket::decode` is `Ok(None)`: stepped over, and silent.
//!
//! So what is under test here is the whole path and not the decoder: the button
//! writes `0x34` with the right subcommand byte, the shard routes it, answers
//! with the whole list, and the client folds it into the one table a window
//! would draw from.

use std::time::Duration;

use openshard_client_net::connection::Event;
use openshard_client_net::transport::enter_world;
use openshard_client_net::view::WorldView;
use openshard_client_net::{doll, skill};
use openshard_protocol::server_packet::ServerPacket;
use openshard_protocol::skill::SkillLock;
use openshard_protocol::wire::{ClilocId, RawSkillId};

use openshard_e2e_shard::{plan, shard, version};

/// Alchemy: id 0, and the one the whole list's *one-based* numbering exists to
/// keep apart from its terminator. A client that read the ids as sent would file
/// every skill one place up and lose the last one to the terminator.
const ALCHEMY: u8 = 0;

/// Mining: id 45, far enough down the list that a list truncated at its
/// terminator would not reach it.
const MINING: u8 = 45;

/// How many skills a pre-AoS client knows, and the fewest any client's list can
/// hold — `skill_count`'s floor.
const FEWEST: usize = 49;

/// Read packets into `view` until `done` says so, or fail. `paperdoll_buttons`'
/// own helper, and for its reason: everything is folded on the way past, since a
/// client that skipped what it was not waiting for would be a client that never
/// applied it either.
async fn until<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut openshard_client_net::transport::Socket<S>,
    view: &mut WorldView,
    what: &str,
    mut done: impl FnMut(&WorldView, &ServerPacket) -> bool,
) {
    let waited = tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(event) = socket.next_event().await.expect("the socket stayed up") {
            let Event::Packet(packet) = event else {
                continue;
            };
            view.apply(&packet);
            if done(view, &packet) {
                return;
            }
        }
        panic!("the shard closed the connection before answering {what}");
    })
    .await;
    assert!(waited.is_ok(), "the shard never answered {what}");
}

/// The list a character is sent for entering the world, decoded and folded.
///
/// The assertions are about the *shape* of the numbering rather than about any
/// value, because that is what the two ends can disagree about while both look
/// right: the ids ride one-based and terminate on a zero, so a decoder that
/// forgot either would come back with a table shifted by one, or a table one row
/// long.
#[tokio::test]
async fn entering_the_world_fills_the_skill_table() {
    let (address, _shard) = shard();
    let entered = tokio::time::timeout(Duration::from_secs(20), enter_world(address, plan(), version()))
        .await
        .expect("the login conversation finished inside the timeout")
        .expect("the client reached the world");
    let (mut socket, mut view) = entered;

    until(&mut socket, &mut view, "the skill list", |view, _| {
        view.player.skills.len() >= FEWEST
    })
    .await;

    assert!(
        view.player.skills.contains_key(&ALCHEMY),
        "skill 0 is missing, which is what reading the ids as sent looks like"
    );
    assert!(
        view.player.skills.contains_key(&MINING),
        "the list stopped short of skill {MINING}"
    );
    let mining = view.player.skills[&MINING];
    assert!(
        mining.cap > 0,
        "every row carries this character's own cap, and a zero one is the field \
         being read out of the wrong place"
    );
    assert!(
        mining.value >= mining.base,
        "a skill is worth at least what is trained in it — value {} against base {}",
        mining.value,
        mining.base
    );
}

/// The button: `0x34` with the skills subcommand, and the whole list back.
///
/// The table is emptied first, so that what is asserted afterwards is this
/// exchange and not the one the login already did. That matters more than it
/// looks: with the login's own list still standing, every assertion below passes
/// on a shard that ignores the request entirely.
#[tokio::test]
async fn the_skills_button_asks_for_the_list_and_the_client_folds_the_answer() {
    let (address, _shard) = shard();
    let entered = tokio::time::timeout(Duration::from_secs(20), enter_world(address, plan(), version()))
        .await
        .expect("the login conversation finished inside the timeout")
        .expect("the client reached the world");
    let (mut socket, mut view) = entered;

    until(&mut socket, &mut view, "the skill list", |view, _| {
        view.player.skills.len() >= FEWEST
    })
    .await;
    view.player.skills.clear();

    socket
        .send(&doll::skills(view.player.serial))
        .await
        .expect("the shard is listening");
    until(&mut socket, &mut view, "the skills button", |view, packet| {
        matches!(packet, ServerPacket::SkillsFull(_)) && view.player.skills.len() >= FEWEST
    })
    .await;
}

/// The lock arrow: `set_skill_lock` sends nothing back — see its own doc in
/// `crates/server/world/src/tick/skills_wire.rs` — so the only way to observe
/// it from a bare socket is to ask for the list again and read the change
/// back off it, the same request the second test above already uses.
#[tokio::test]
async fn the_lock_arrow_is_stored_and_reflected_in_the_next_full_list() {
    let (address, _shard) = shard();
    let entered = tokio::time::timeout(Duration::from_secs(20), enter_world(address, plan(), version()))
        .await
        .expect("the login conversation finished inside the timeout")
        .expect("the client reached the world");
    let (mut socket, mut view) = entered;

    until(&mut socket, &mut view, "the skill list", |view, _| {
        view.player.skills.len() >= FEWEST
    })
    .await;
    assert_ne!(
        view.player.skills[&MINING].lock,
        SkillLock::Locked,
        "the fixture must not already be locked, or the assertion below proves nothing"
    );

    socket
        .send(&skill::set_lock(RawSkillId(MINING), SkillLock::Locked))
        .await
        .expect("the shard is listening");
    view.player.skills.clear();
    socket
        .send(&doll::skills(view.player.serial))
        .await
        .expect("the shard is listening");
    until(
        &mut socket,
        &mut view,
        "the re-fetched skill list",
        |view, packet| matches!(packet, ServerPacket::SkillsFull(_)) && view.player.skills.len() >= FEWEST,
    )
    .await;

    assert_eq!(
        view.player.skills[&MINING].lock,
        SkillLock::Locked,
        "the lock click has to outlive the request that reads it back"
    );
}

/// Tactics (id 27) is passive in `openshard_state::skill` — `usable == false`
/// — so `use_skill_button` refuses it out loud with cliloc 500014 ("that
/// skill cannot be used directly") rather than starting anything. That refusal
/// is the whole path this end can observe without an individual skill's own
/// effect being built: decode → dispatch → `use_skill_button`'s gate.
#[tokio::test]
async fn a_passive_skill_refuses_the_use_button() {
    const TACTICS: u8 = 27;

    let (address, _shard) = shard();
    let entered = tokio::time::timeout(Duration::from_secs(20), enter_world(address, plan(), version()))
        .await
        .expect("the login conversation finished inside the timeout")
        .expect("the client reached the world");
    let (mut socket, mut view) = entered;

    socket
        .send(&skill::use_skill(RawSkillId(TACTICS)))
        .await
        .expect("the shard is listening");
    until(
        &mut socket,
        &mut view,
        "the cannot-use-directly message",
        |_, packet| {
            matches!(
                packet,
                ServerPacket::LocalizedMessage(message) if message.cliloc == ClilocId(500_014)
            )
        },
    )
    .await;
}
