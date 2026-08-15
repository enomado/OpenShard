//! War and alliance: the two-declaration handshake, and peace.
//!
//! One function declares both, because they are the same act with a different
//! word on it — an offer that becomes a relation when the other guild makes the
//! matching one. See the crate doc for why there is no separate accept path.

use openshard_entities::EntityId;
use openshard_state::{GuildId, Relation, WorldState};

use crate::{Refusal, announce, may_lead, recolour_guild};

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
    let own = may_lead(state, leader)?;
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

/// Take back an offer the other guild has not answered.
///
/// Only an offer. A relation both guilds declared is ended by [`make_peace`],
/// which says so to both of them.
pub fn withdraw(state: &mut WorldState, leader: EntityId, other: GuildId) -> Result<(), Refusal> {
    let own = may_lead(state, leader)?;
    if own == other || state.guilds.get(other).is_none() {
        return Err(Refusal::NoSuchGuild);
    }
    state.guilds.withdraw(own, other);
    Ok(())
}

/// End a war or an alliance, both ways.
///
/// One guild's decision, not a second handshake. A war that took two guilds to
/// start but only one to end is the guildstone's rule and the right one: the
/// alternative is a guild that cannot stop being attacked because the guild
/// attacking it will not agree to stop.
pub fn make_peace(state: &mut WorldState, leader: EntityId, other: GuildId) -> Result<(), Refusal> {
    let own = may_lead(state, leader)?;
    if own == other || state.guilds.get(other).is_none() {
        return Err(Refusal::NoSuchGuild);
    }
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
