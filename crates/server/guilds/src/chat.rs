//! Guild chat: a line to the guild, and a line to everybody it is allied with.
//!
//! # It is speech, and it is not spoken
//!
//! A guild line arrives as ordinary `0xAD` speech with the mode byte set to
//! [`TalkMode::Guild`], and goes back out as an ordinary `0xAE` with the same
//! mode — so the client draws it as a line from a named speaker, in the guild
//! colour, and *not* over anybody's head. Nothing about earshot applies: the
//! listeners are the roster, and `World::say` branches here before it measures a
//! distance. See [`speech_range`](openshard_chat::speech_range), which answers
//! zero for both modes so that a routing failure is silence rather than a
//! private line shouted down the street.
//!
//! # Alliance chat means something slightly different here
//!
//! ServUO's alliance is a **named** object — several guilds, a leader guild, an
//! invitation handshake of its own — and this engine has none of that; what it
//! has is [`Relation::Ally`], a pairwise declaration between two guilds. So
//! [`say_to_alliance`] reaches every guild yours has allied with, which is the
//! natural reading of a pairwise model and is not the reference's:
//!
//! - **ServUO**: A and B are in one alliance, so a line from A reaches B, and B
//!   sees the alliance's own name.
//! - **Here**: A is allied with B and with C, so a line from A reaches both —
//!   even though B and C have declared nothing about each other.
//!
//! Named alliances stay deferred (`docs/roadmap.md` §6). When they land this is
//! the function that changes, and the difference is written here so that whoever
//! does it knows what behaviour they are replacing.

use openshard_entities::EntityId;
use openshard_protocol::server_packet::ServerPacket;
use openshard_protocol::speech::{Font, TalkMode, UnicodeMessage};
use openshard_protocol::wire::Hue;
use openshard_state::{GuildId, Relation, WorldState};

use crate::{Refusal, roster};

/// The language tag every line this engine sends carries — `openshard-chat`'s
/// own, repeated rather than shared because that crate does not depend on this
/// one and a guild line is not routed through it.
const LANGUAGE: &str = "ENU";

/// Say something to every member of your guild who is online.
pub fn say_to_guild(
    state: &mut WorldState,
    speaker: EntityId,
    hue: Hue,
    font: Font,
    text: &str,
) -> Result<(), Refusal> {
    let guild = state.guild_of(speaker).ok_or(Refusal::NotInAGuild)?.id;
    let packet = line(state, speaker, TalkMode::Guild, hue, font, text)?;
    tell_guild(state, guild, &packet);
    Ok(())
}

/// Say something to every guild yours has allied with — and to your own.
///
/// Your own as well, which ServUO also does (its alliance includes the speaker's
/// guild): a line meant for the allies that your own guildmates could not see
/// would read, to them, as their guildmate having gone quiet.
pub fn say_to_alliance(
    state: &mut WorldState,
    speaker: EntityId,
    hue: Hue,
    font: Font,
    text: &str,
) -> Result<(), Refusal> {
    let own = state.guild_of(speaker).ok_or(Refusal::NotInAGuild)?;
    let allies: Vec<GuildId> = own
        .relations
        .iter()
        .filter(|(_, relation)| **relation == Relation::Ally)
        .map(|(id, _)| *id)
        .collect();
    if allies.is_empty() {
        return Err(Refusal::NoAllies);
    }
    let own = own.id;
    let packet = line(state, speaker, TalkMode::Alliance, hue, font, text)?;
    tell_guild(state, own, &packet);
    for ally in allies {
        tell_guild(state, ally, &packet);
    }
    Ok(())
}

/// Send one packet to every member of `guild` who is online.
///
/// [`tell_party`](openshard_party::tell_party)'s counterpart, and the second
/// tenant of the same idea: a line goes to a set of people picked by membership
/// rather than by where they are standing.
pub fn tell_guild(state: &mut WorldState, guild: GuildId, packet: &ServerPacket) {
    for member in roster(state, guild) {
        state.send_to(member, packet);
    }
}

/// Build the `0xAE` a guild line goes out as.
///
/// The speaker's own serial, body and name ride along, exactly as they do for
/// ordinary speech — which is what lets a client draw "Lord British: regroup"
/// rather than an anonymous system line.
fn line(
    state: &WorldState,
    speaker: EntityId,
    mode: TalkMode,
    hue: Hue,
    font: Font,
    text: &str,
) -> Result<ServerPacket, Refusal> {
    let serial = state.registry.serial_of(speaker).ok_or(Refusal::NotAMobile)?;
    Ok(ServerPacket::UnicodeMessage(UnicodeMessage {
        serial: Some(serial),
        graphic: state.registry.get::<openshard_state::Body>(speaker).map(|b| b.id),
        mode,
        hue,
        font,
        language: LANGUAGE.to_owned(),
        name: state
            .registry
            .get::<openshard_state::Name>(speaker)
            .map_or_else(String::new, |name| name.0.clone()),
        text: text.to_owned(),
    }))
}
