//! What the world knows about a connection, apart from the character on it.
//!
//! # Why a connection is a row and not a component
//!
//! Everything the world knew about a client used to hang off its *entity* — the
//! [`Client`](crate::components::Client) component — which quietly made "has a
//! character" the precondition for "can be spoken to". A connection sitting on
//! the character screen has no entity, so the world could not address it at all:
//! [`WorldState::send_packet`](crate::WorldState::send_packet) resolves the
//! client version through the player table and drops the packet, silently, when
//! the lookup misses.
//!
//! That is why the character screen is answered by the binary today rather than
//! out of a tick, and it is the first thing in the way of moving it in — see
//! `docs/connection_state.md`. A connection is a thing in its own right, with a
//! lifetime that starts before its character exists and ends after it is gone, so
//! it gets a row of its own keyed by
//! [`ConnectionId`](openshard_gateway::ConnectionId).

use openshard_protocol::version::ClientVersion;

/// One connected client, as the world sees it.
///
/// Opened by `Command::Authenticated` when the login conversation hands the
/// connection over, and closed by `Command::Disconnect`. A row exists for a
/// connection that is playing nothing — that is the point of it.
///
/// # Not a session
///
/// The *session* — which character this connection is playing, and whether it is
/// in the world yet — is the binary's, and stays there: the packet router has to
/// answer "may this reach the world" synchronously, and the world answers no
/// synchronous question. This is only what the world itself has to remember about
/// a socket. The per-connection state currently spread across eight maps on
/// [`WorldState`](crate::WorldState) belongs here and is step S7 of the plan in
/// `docs/connection_state.md`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Connection {
    /// What the client claims to be. Every feature gate and every encoder reads
    /// it, and this is the only place it lives: the game socket never states its
    /// version, so this is what the login socket carried across on the auth key.
    pub version: ClientVersion,
}

impl Connection {
    /// A connection that has just been handed over by the login conversation.
    pub const fn new(version: ClientVersion) -> Self {
        Self { version }
    }
}
