//! Guilds across the door: what goes to disk, and what comes back at boot.
//!
//! # Two halves, saved apart
//!
//! A guild is written here, as a record of its own. Who is *in* it is written
//! with the character, as a [`CharacterRecord`] field. That is not an accident of
//! layout: the two change on different schedules — a character is swept whenever
//! it is touched, a guild only on the full sweep — and a roster held in both
//! places would be two answers to one question.
//!
//! So the roster is derived on the way back in: every character restored with a
//! `guild` id names the guild, and nothing else does. A guild whose every member
//! is gone is a guild with no members, which is what it actually is.
//!
//! # What the high-water mark is for
//!
//! `Guilds::high_water` is restored from the world row, not re-derived from the
//! guilds themselves — see [`WorldRecord::guild_high_water`]. The maximum id in
//! the table is not the maximum ever issued, because a disbanded guild leaves no
//! row behind. A shard that re-derived it would hand the next guild founded an id
//! a disbanded one had already used, and every character record still naming that
//! id — a member who was offline when it disbanded, and so was never swept —
//! would silently find itself in the new guild.

use openshard_persistence::{GuildRecord, GuildStanding};
use openshard_state::guild::{Guild, GuildId, Relation};
use tracing::info;

use super::World;

impl World {
    /// Every guild as a saveable record.
    ///
    /// A straight copy across the record seam, like a region: a guild is exactly
    /// its data, with no live timer to translate.
    pub(super) fn guild_records(&self) -> Vec<GuildRecord> {
        self.state
            .guilds
            .iter()
            .map(|guild| GuildRecord {
                id: guild.id.0,
                name: guild.name.clone(),
                abbreviation: guild.abbreviation.clone(),
                leader: guild.leader,
                relations: standings(guild.relations.iter()),
                proposals: standings(guild.proposals.iter()),
            })
            .collect()
    }

    /// Re-create the guilds from saved records at boot.
    ///
    /// Call once, before anyone connects, and **before** the characters are
    /// restored: a `GuildMember` component whose guild is not in the table yet
    /// reads as no membership, and while nothing at boot asks, the day something
    /// does the failure would be a whole shard of players quietly unguilded.
    pub fn restore_guilds(&mut self, records: Vec<GuildRecord>) {
        for record in &records {
            self.state.guilds.restore(Guild {
                id: GuildId(record.id),
                name: record.name.clone(),
                abbreviation: record.abbreviation.clone(),
                leader: record.leader,
                relations: relations(&record.relations),
                proposals: relations(&record.proposals),
            });
        }
        if !records.is_empty() {
            info!(guilds = records.len(), "restored the shard's guilds");
        }
    }

    /// Restore the id counter from the world row.
    ///
    /// Separate from [`restore_guilds`](Self::restore_guilds) because the number
    /// is in a different row and arrives later in the boot — and it is safe in
    /// either order: `restore` has already raised the counter past every id it
    /// put back, and `set_high_water` never lowers it. So a store whose world row
    /// is missing or stale still cannot re-issue an id that is plainly in use;
    /// the row is the authority, the restored guilds are the floor.
    #[must_use]
    pub fn with_guild_high_water(mut self, id: u32) -> Self {
        self.state.guilds.set_high_water(id);
        self
    }
}

/// A guild's relations, as they go to disk.
fn standings<'a>(relations: impl Iterator<Item = (&'a GuildId, &'a Relation)>) -> Vec<GuildStanding> {
    relations
        .map(|(&other, &relation)| GuildStanding {
            other: other.0,
            at_war: relation == Relation::War,
        })
        .collect()
}

/// And back again. A `bool` is a total answer — see [`GuildStanding::at_war`] —
/// so there is nothing here to refuse.
fn relations(standings: &[GuildStanding]) -> std::collections::BTreeMap<GuildId, Relation> {
    standings
        .iter()
        .map(|standing| {
            (
                GuildId(standing.other),
                if standing.at_war {
                    Relation::War
                } else {
                    Relation::Ally
                },
            )
        })
        .collect()
}
