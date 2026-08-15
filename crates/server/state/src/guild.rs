//! Guilds: who belongs to one, and how two of them regard each other.
//!
//! Shared substrate, not rules. This is the membership and the relations the
//! packet path has to read; the *system* — founding a guild, invitations,
//! declaring a war, the roster gump — is `openshard-guilds` above it, the same
//! split [`Regions`](crate::Regions) and [`Dialogue`](crate::Dialogue) have.
//!
//! # Why it is here and not only in the guilds crate
//!
//! Because a `0x78` has a notoriety byte in it. What colour a mobile draws in
//! depends on who is looking — a guildmate is green, a mobile whose guild you are
//! at war with is orange — so the wire path itself has to be able to ask, and the
//! wire path is [`WorldState`](crate::WorldState)'s. See
//! [`WorldState::notoriety_toward`].
//!
//! # ServUO's rule, and its order
//!
//! `Scripts/Misc/Notoriety.cs`: a murderer is red and a criminal is grey
//! **before** any guild question is asked, and only then does the same guild or an
//! ally read green and a guild at war read orange. Guild colour loses to standing,
//! which is what stops a red hiding inside a guild tabard.

use std::collections::{BTreeMap, BTreeSet};

/// A guild's stable id — the key its members carry and its relations are named
/// by.
///
/// Distinct from every other `u32` in world state: it addresses a [`Guilds`]
/// entry and nothing else.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct GuildId(pub u32);

/// How one guild regards another.
///
/// There is no "neutral" variant: two guilds with no declared relation are simply
/// absent from each other's tables, and absence is the neutral case. A variant for
/// it would be a second way to spell the same thing and a third state to keep in
/// step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Relation {
    /// Allied — their members read green to each other, like one's own.
    Ally,
    /// At war — their members read orange, and may be attacked without the grey
    /// flag a criminal act would earn.
    War,
}

/// One guild.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Guild {
    /// Its id, which is also the key it is stored under.
    pub id: GuildId,
    /// What it calls itself — "The Order of the Silver Serpent".
    pub name: String,
    /// The short form the client draws in brackets after a member's name, three
    /// or four letters by convention: "OSS".
    pub abbreviation: String,
    /// Who leads it. A guild always has one; disbanding is what happens when it
    /// would not.
    pub leader: openshard_protocol::serial::Serial,
    /// How it regards other guilds. Only declared relations are here.
    pub relations: BTreeMap<GuildId, Relation>,
    /// What it has offered another guild and the other has not yet matched.
    ///
    /// Separate from [`relations`](Self::relations) because a proposal is not a
    /// relation: a guild that has declared war and been ignored is not at war,
    /// and its members must not turn orange on the strength of its own opinion.
    /// A relation exists once both sides hold the same proposal — see
    /// [`Guilds::propose`].
    pub proposals: BTreeMap<GuildId, Relation>,
}

impl Guild {
    /// How this guild regards `other` — `None` for a guild it has not declared
    /// anything about, and for itself.
    #[must_use]
    pub fn toward(&self, other: GuildId) -> Option<Relation> {
        self.relations.get(&other).copied()
    }

    /// What this guild has offered `other` and is still waiting on.
    #[must_use]
    pub fn offered(&self, other: GuildId) -> Option<Relation> {
        self.proposals.get(&other).copied()
    }
}

/// Every guild on the shard.
///
/// A map rather than a `Vec`: unlike a region, a guild's id is not its position
/// in a list — guilds are founded and disbanded while the shard runs, and an id
/// must not be reused by the next one to be founded. `next_id` never goes
/// backwards for that reason.
#[derive(Clone, Default, Debug)]
pub struct Guilds {
    guilds: BTreeMap<GuildId, Guild>,
    /// The next id to hand out. Monotonic, and saved with the world: a restart
    /// that restarted it would let a new guild inherit a disbanded one's id, and
    /// every member record still naming that id would silently join it.
    next_id: u32,
}

impl Guilds {
    /// Found a guild and return its id.
    pub fn found(
        &mut self,
        name: String,
        abbreviation: String,
        leader: openshard_protocol::serial::Serial,
    ) -> GuildId {
        self.next_id += 1;
        let id = GuildId(self.next_id);
        self.guilds.insert(
            id,
            Guild {
                id,
                name,
                abbreviation,
                leader,
                relations: BTreeMap::new(),
                proposals: BTreeMap::new(),
            },
        );
        id
    }

    /// The guild that calls itself `name`, case-insensitively.
    ///
    /// A scan, and deliberately: it is asked once when a guild is founded and
    /// never on a hot path, so an index would be a second thing to keep in step
    /// for no gain.
    #[must_use]
    pub fn by_name(&self, name: &str) -> Option<&Guild> {
        self.guilds.values().find(|g| g.name.eq_ignore_ascii_case(name))
    }

    /// The guild that draws as `abbreviation`, case-insensitively.
    #[must_use]
    pub fn by_abbreviation(&self, abbreviation: &str) -> Option<&Guild> {
        self.guilds
            .values()
            .find(|g| g.abbreviation.eq_ignore_ascii_case(abbreviation))
    }

    /// One guild, if it exists.
    #[must_use]
    pub fn get(&self, id: GuildId) -> Option<&Guild> {
        self.guilds.get(&id)
    }

    /// One guild, to change.
    pub fn get_mut(&mut self, id: GuildId) -> Option<&mut Guild> {
        self.guilds.get_mut(&id)
    }

    /// Every guild, in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Guild> {
        self.guilds.values()
    }

    /// How many there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.guilds.len()
    }

    /// Whether none are founded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.guilds.is_empty()
    }

    /// Declare a relation **both ways**.
    ///
    /// A war is not one-sided: ServUO's `IsEnemy` is asked of either guild and
    /// both answer yes, so storing it on one and not the other would make the
    /// colour depend on which of the two a client happened to ask about. The
    /// same for an alliance.
    pub fn declare(&mut self, from: GuildId, to: GuildId, relation: Relation) {
        if from == to {
            return;
        }
        if let Some(guild) = self.guilds.get_mut(&from) {
            guild.relations.insert(to, relation);
        }
        if let Some(guild) = self.guilds.get_mut(&to) {
            guild.relations.insert(from, relation);
        }
    }

    /// Offer `to` a relation, and declare it if they have offered the same back.
    ///
    /// Returns whether it took effect. This is the classic guildstone handshake:
    /// declaring war on a guild that has not declared war on you puts you on
    /// their list and changes nothing else, and the war begins when they declare
    /// in return. An alliance is the same shape, which is why one function does
    /// both rather than a war path and an invitation path that would drift.
    ///
    /// A proposal that *matches* is consumed — the relation is the record, and a
    /// proposal left standing beside it would be a second answer to the same
    /// question. A proposal that meets a **different** standing offer replaces
    /// it: a guild that offered an alliance and then declared war means the war.
    pub fn propose(&mut self, from: GuildId, to: GuildId, relation: Relation) -> bool {
        if from == to || !self.guilds.contains_key(&from) || !self.guilds.contains_key(&to) {
            return false;
        }
        if self.guilds[&to].offered(from) == Some(relation) {
            self.withdraw(to, from);
            self.declare(from, to, relation);
            return true;
        }
        if let Some(guild) = self.guilds.get_mut(&from) {
            guild.proposals.insert(to, relation);
        }
        false
    }

    /// Take back an offer. Silent if there was none.
    pub fn withdraw(&mut self, from: GuildId, to: GuildId) {
        if let Some(guild) = self.guilds.get_mut(&from) {
            guild.proposals.remove(&to);
        }
    }

    /// Withdraw a declaration, both ways, and any offer either side was still
    /// holding.
    ///
    /// Peace clears the offers too: a guild that made peace while the other's
    /// war declaration still stood would go back to war the moment it declared
    /// anything, without either side deciding to.
    pub fn undeclare(&mut self, from: GuildId, to: GuildId) {
        if let Some(guild) = self.guilds.get_mut(&from) {
            guild.relations.remove(&to);
            guild.proposals.remove(&to);
        }
        if let Some(guild) = self.guilds.get_mut(&to) {
            guild.relations.remove(&from);
            guild.proposals.remove(&from);
        }
    }

    /// Disband a guild, and take every declaration about it with it.
    ///
    /// The sweep is the point: a relation left pointing at a disbanded id would
    /// make the *next* guild founded under a reused id inherit a war. Ids are not
    /// reused, so this cannot actually happen — and it is swept anyway, because
    /// the invariant it protects is one line away from being broken by a future
    /// change to `found`.
    pub fn disband(&mut self, id: GuildId) -> Option<Guild> {
        let gone = self.guilds.remove(&id)?;
        for guild in self.guilds.values_mut() {
            guild.relations.remove(&id);
            guild.proposals.remove(&id);
        }
        Some(gone)
    }

    /// The ids in use, for the save and for a test.
    #[must_use]
    pub fn ids(&self) -> BTreeSet<GuildId> {
        self.guilds.keys().copied().collect()
    }

    /// The highest id handed out so far, saved and restored with the world.
    #[must_use]
    pub const fn high_water(&self) -> u32 {
        self.next_id
    }

    /// Restore the id counter after a load. Never lowers it: an id already handed
    /// out must not be handed out again.
    pub fn set_high_water(&mut self, id: u32) {
        self.next_id = self.next_id.max(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openshard_protocol::serial::Serial;

    fn leader() -> Serial {
        Serial::new(0x0000_0001).expect("a mobile serial")
    }

    #[test]
    fn a_declaration_binds_both_guilds() {
        // The colour must not depend on which of the two a client asks about.
        let mut guilds = Guilds::default();
        let a = guilds.found("A".to_owned(), "A".to_owned(), leader());
        let b = guilds.found("B".to_owned(), "B".to_owned(), leader());
        guilds.declare(a, b, Relation::War);
        assert_eq!(guilds.get(a).unwrap().toward(b), Some(Relation::War));
        assert_eq!(guilds.get(b).unwrap().toward(a), Some(Relation::War));
    }

    #[test]
    fn a_guild_declares_nothing_about_itself() {
        let mut guilds = Guilds::default();
        let a = guilds.found("A".to_owned(), "A".to_owned(), leader());
        guilds.declare(a, a, Relation::War);
        assert_eq!(
            guilds.get(a).unwrap().toward(a),
            None,
            "a guild at war with itself"
        );
    }

    #[test]
    fn a_declaration_nobody_answered_is_not_a_relation() {
        // The whole reason proposals are a separate map: a guild that declared war
        // and was ignored is not at war, and its members must not read orange on
        // the strength of its own opinion.
        let mut guilds = Guilds::default();
        let a = guilds.found("A".to_owned(), "A".to_owned(), leader());
        let b = guilds.found("B".to_owned(), "B".to_owned(), leader());
        assert!(!guilds.propose(a, b, Relation::War), "a war with one side");
        assert_eq!(guilds.get(a).unwrap().toward(b), None);
        assert_eq!(guilds.get(b).unwrap().toward(a), None);
        assert_eq!(guilds.get(a).unwrap().offered(b), Some(Relation::War));

        // And the answer is what makes it one, on either side's turn.
        assert!(guilds.propose(b, a, Relation::War));
        assert_eq!(guilds.get(a).unwrap().toward(b), Some(Relation::War));
        assert_eq!(guilds.get(b).unwrap().toward(a), Some(Relation::War));
        assert_eq!(
            guilds.get(a).unwrap().offered(b),
            None,
            "a proposal still standing beside the relation it became"
        );
    }

    #[test]
    fn an_answer_to_a_different_offer_is_not_an_answer() {
        // A offers an alliance; B declares war. Neither is agreement, and reading
        // the *presence* of an offer rather than its kind would make them one.
        let mut guilds = Guilds::default();
        let a = guilds.found("A".to_owned(), "A".to_owned(), leader());
        let b = guilds.found("B".to_owned(), "B".to_owned(), leader());
        guilds.propose(a, b, Relation::Ally);
        assert!(!guilds.propose(b, a, Relation::War));
        assert_eq!(guilds.get(a).unwrap().toward(b), None);
    }

    #[test]
    fn peace_takes_the_standing_offer_with_it() {
        // Otherwise the next thing either side declares silently restores the war
        // the two of them just ended.
        let mut guilds = Guilds::default();
        let a = guilds.found("A".to_owned(), "A".to_owned(), leader());
        let b = guilds.found("B".to_owned(), "B".to_owned(), leader());
        guilds.propose(a, b, Relation::War);
        guilds.propose(b, a, Relation::War);
        guilds.undeclare(a, b);
        assert_eq!(guilds.get(a).unwrap().offered(b), None);
        assert_eq!(guilds.get(b).unwrap().offered(a), None);
    }

    #[test]
    fn disbanding_takes_every_declaration_about_it() {
        let mut guilds = Guilds::default();
        let a = guilds.found("A".to_owned(), "A".to_owned(), leader());
        let b = guilds.found("B".to_owned(), "B".to_owned(), leader());
        guilds.declare(a, b, Relation::Ally);
        guilds.disband(a);
        assert_eq!(guilds.get(b).unwrap().toward(a), None, "a war with a ghost");
    }

    #[test]
    fn an_id_is_never_handed_out_twice() {
        // Every member record names a guild by id. Reusing one would silently
        // move a disbanded guild's members into the guild founded after it.
        let mut guilds = Guilds::default();
        let a = guilds.found("A".to_owned(), "A".to_owned(), leader());
        guilds.disband(a);
        let b = guilds.found("B".to_owned(), "B".to_owned(), leader());
        assert_ne!(a, b);

        // And the counter survives a restore, which is the case the save exists
        // for: a shard that restarted at zero would re-issue every id.
        let high = guilds.high_water();
        let mut restored = Guilds::default();
        restored.set_high_water(high);
        let c = restored.found("C".to_owned(), "C".to_owned(), leader());
        assert!(c > b, "{c:?} was handed out again after {b:?}");
    }

    #[test]
    fn the_high_water_mark_never_goes_backwards() {
        let mut guilds = Guilds::default();
        guilds.found("A".to_owned(), "A".to_owned(), leader());
        let high = guilds.high_water();
        guilds.set_high_water(0);
        assert_eq!(guilds.high_water(), high, "an older save lowered the counter");
    }
}
