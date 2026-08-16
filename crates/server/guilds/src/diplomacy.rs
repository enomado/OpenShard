//! War and alliance: the two-declaration handshake, and peace.
//!
//! One function declares both, because they are the same act with a different
//! word on it — an offer that becomes a relation when the other guild makes the
//! matching one. See the crate doc for why there is no separate accept path.

use openshard_entities::EntityId;
use openshard_state::{GuildId, Relation, WorldState};

use crate::{RankFlags, Refusal, announce, may, recolour_guild};

/// Which rank flag governs a relation.
///
/// The two are not one permission: a Warlord declares and ends wars and may not
/// ally the guild, and an alliance is the Leader's alone. Written here so the
/// three callers below cannot disagree about which flag a relation wants.
const fn flag_for(relation: Relation) -> RankFlags {
    match relation {
        Relation::War => RankFlags::CONTROL_WAR_STATUS,
        Relation::Ally => RankFlags::ALLIANCE_CONTROL,
    }
}

/// What a declaration did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// The other guild has not said the same yet. Nothing has changed but the
    /// list they see.
    Offered,
    /// They had already said it, so it is now true of both.
    Declared,
}

/// Declare war on, or offer an alliance to, another guild.
///
/// Nothing happens on the strength of one guild's word: this leaves an offer,
/// and the relation exists when `other`'s leader makes the same one back. A
/// guild that answers a standing offer with a *different* one has not agreed to
/// anything, and replaces its own offer rather than accepting theirs.
pub fn propose(
    state: &mut WorldState,
    leader: EntityId,
    other: GuildId,
    relation: Relation,
) -> Result<Outcome, Refusal> {
    let (own, _) = may(state, leader, flag_for(relation))?;
    if own == other || state.guilds.get(other).is_none() {
        return Err(Refusal::NoSuchGuild);
    }

    let (ours, theirs) = names(state, own, other);
    if !state.guilds.propose(own, other, relation) {
        // Told to both sides. A declaration nobody is told about is a declaration
        // the other guild cannot answer, and answering is the whole mechanism.
        let (mine, yours) = match relation {
            Relation::War => (
                format!("You have declared war on {theirs}."),
                format!("{ours} has declared war on you."),
            ),
            Relation::Ally => (
                format!("You have offered an alliance to {theirs}."),
                format!("{ours} has offered you an alliance."),
            ),
        };
        announce(state, own, &mine);
        announce(state, other, &yours);
        return Ok(Outcome::Offered);
    }

    let text = match relation {
        Relation::War => format!("{ours} and {theirs} are at war."),
        Relation::Ally => format!("{ours} and {theirs} are allied."),
    };
    announce(state, own, &text);
    announce(state, other, &text);
    // Both rosters: the colour moved on every screen where a member of one can
    // see a member of the other, and that is two directions.
    recolour_guild(state, own);
    recolour_guild(state, other);
    Ok(Outcome::Declared)
}

/// End a war or an alliance, both ways — and take back an offer nobody
/// answered, which is the same button on the same row.
///
/// One guild's decision, not a second handshake. A war that took two guilds to
/// start but only one to end is the guildstone's rule and the right one: the
/// alternative is a guild that cannot stop being attacked because the guild
/// attacking it will not agree to stop.
///
/// Which flag it takes depends on **what is being ended**, and is read off the
/// world rather than passed in — the button is one button, and the row it sits
/// on already says whether it is a war or an alliance. A Warlord can therefore
/// end a war and not an alliance, through the same call.
pub fn make_peace(state: &mut WorldState, actor: EntityId, other: GuildId) -> Result<(), Refusal> {
    let own = state.guild_of(actor).ok_or(Refusal::NotInAGuild)?.id;
    if own == other || state.guilds.get(other).is_none() {
        return Err(Refusal::NoSuchGuild);
    }
    // The standing relation, or failing that the offer this would withdraw.
    // Nothing at all is still permitted, to whoever could have declared one:
    // ending a relation that is not there changes nothing, and refusing it would
    // make a stale gump row unclickable rather than harmless.
    let ending = state
        .guilds
        .get(own)
        .and_then(|guild| guild.toward(other).or_else(|| guild.offered(other)));
    match ending {
        Some(relation) => may(state, actor, flag_for(relation))?,
        None => may(state, actor, RankFlags::CONTROL_WAR_STATUS)
            .or_else(|_| may(state, actor, RankFlags::ALLIANCE_CONTROL))?,
    };
    let (ours, theirs) = names(state, own, other);
    state.guilds.undeclare(own, other);
    let text = format!("{ours} and {theirs} are at peace.");
    announce(state, own, &text);
    announce(state, other, &text);
    recolour_guild(state, own);
    recolour_guild(state, other);
    Ok(())
}

/// Both guilds' names, for a message that has to read as one sentence.
///
/// Taken before the change and returned owned, because every caller goes on to
/// borrow the world mutably and the names live in it.
fn names(state: &WorldState, own: GuildId, other: GuildId) -> (String, String) {
    let name = |id| {
        state
            .guilds
            .get(id)
            .map_or_else(String::new, |guild| guild.name.clone())
    };
    (name(own), name(other))
}
