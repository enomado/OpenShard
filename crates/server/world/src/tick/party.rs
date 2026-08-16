//! The tick's end of `0xBF` subcommand `0x06`: one request in, the party
//! system's answer out.
//!
//! Nothing here is a rule. Every arm resolves the serials the client sent into
//! entities and hands off to `openshard-party`, which is where "may this player
//! do that" lives — the same shape `guilds.rs` beside it has for the guild
//! window's replies.

use openshard_gateway::ConnectionId;
use openshard_protocol::party::PartyRequest;
use openshard_state::TargetPurpose;

use super::World;

impl World {
    /// Act on one party request.
    pub(super) fn party_request(&mut self, connection: ConnectionId, request: &PartyRequest) {
        let Some(&actor) = self.state.players.get(&connection) else {
            return;
        };
        match request {
            // The only arm that answers with a cursor rather than an act: the
            // client is asking *who*, and the answer arrives as an ordinary
            // target reply — see `staff.rs`'s `PartyInvite`.
            PartyRequest::Add => {
                self.state.raise_target(actor, TargetPurpose::PartyInvite);
                self.state.system_message(actor, "Whom shall we ask along?");
            }
            PartyRequest::Remove(serial) => {
                let target = serial
                    .validate()
                    .and_then(|serial| self.state.registry.entity_of(serial));
                // A serial naming nobody is read as "myself", which is what
                // leaving looks like on a client that has forgotten who it is
                // naming — and is harmless, because leaving is the one thing
                // anybody may always do.
                let target = target.unwrap_or(actor);
                if let Err(refusal) = openshard_party::remove(&mut self.state, actor, target) {
                    self.state.system_message(actor, refusal.message());
                }
            }
            PartyRequest::PrivateMessage { to, text } => {
                let Some(listener) = to
                    .validate()
                    .and_then(|serial| self.state.registry.entity_of(serial))
                else {
                    return;
                };
                if let Err(refusal) = openshard_party::say_privately(&mut self.state, actor, listener, text) {
                    self.state.system_message(actor, refusal.message());
                }
            }
            PartyRequest::PublicMessage(text) => {
                if let Err(refusal) = openshard_party::say_to_party(&mut self.state, actor, text) {
                    self.state.system_message(actor, refusal.message());
                }
            }
            PartyRequest::SetCanLoot(can_loot) => {
                if let Err(refusal) = openshard_party::set_can_loot(&mut self.state, actor, *can_loot) {
                    self.state.system_message(actor, refusal.message());
                }
            }
            // The leader the client names is ignored on both of these. It is the
            // leader *it* thinks invited them, and the shard already knows —
            // the `PartyCandidate` component is the record, and trusting the
            // packet would let a client accept an invitation it never had by
            // naming somebody who is inviting anybody at all.
            PartyRequest::Accept(_) => {
                if let Err(refusal) = openshard_party::accept(&mut self.state, actor) {
                    self.state.system_message(actor, refusal.message());
                }
            }
            PartyRequest::Decline(_) => {
                if let Err(refusal) = openshard_party::decline(&mut self.state, actor) {
                    self.state.system_message(actor, refusal.message());
                }
            }
            PartyRequest::Unknown(_) => {}
            // `PartyRequest` is `#[non_exhaustive]` for callers outside this
            // workspace; every variant that exists today is matched above.
            _ => {}
        }
    }
}
