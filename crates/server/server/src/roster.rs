use super::*;

/// A character's place in the roster: the account and the character name, both
/// case-folded.
///
/// A type rather than the `(String, String)` it replaces, because that pair was
/// built by hand at four call sites and each one had to remember to fold the
/// case itself — one of them folding a raw string off the wire rather than a
/// [`CharacterName`], which is the same thing right up until it isn't. Built
/// only by [`RosterKey::new`], so there is one place the rule lives.
#[derive(PartialEq, Eq, Hash, Debug)]
struct RosterKey {
    account: String,
    character: String,
}

impl RosterKey {
    fn new(account: &AccountName, character: &CharacterName) -> Self {
        Self {
            account: account.normalized(),
            character: character.normalized(),
        }
    }
}

/// Where every stored character was when it was last seen.
///
/// # Why the shard keeps this at all
///
/// The store has the same rows, but the store is written *later* — a snapshot is
/// handed to a task nobody waits for, so a player who logs out and straight back
/// in can beat their own save. This is the copy that closes that gap: seeded from
/// the store at boot, and kept current from `World::drain_departed` on every
/// logout.
///
/// # It is not the account's character list
///
/// That lives on `Accounts`, and it is the authority on which characters exist.
/// This is the authority on *where they were* — serial, spot, look, sheet — and a
/// character can be on the list with nothing here at all: one created during this
/// run has never logged out, so nothing has been recorded about it yet. Code that
/// treats "no record" as "no character" is how a brand-new character got deleted
/// out from under the connection playing it; see `Sessions::is_playing`.
pub(crate) struct Roster(HashMap<RosterKey, CharacterRecord>);

impl Roster {
    /// An empty roster — a shard whose store has not been read yet.
    pub(crate) fn new() -> Self {
        Self(HashMap::new())
    }

    /// File a character's state, replacing whatever was known about it.
    ///
    /// The key comes off the record rather than from the caller, so a record
    /// cannot be filed under the wrong name.
    pub(crate) fn remember(&mut self, record: CharacterRecord) {
        self.0
            .insert(RosterKey::new(&record.account, &record.name), record);
    }

    /// Where this character was last seen, or `None` if nothing ever recorded it.
    pub(crate) fn get(&self, account: &AccountName, character: &CharacterName) -> Option<&CharacterRecord> {
        self.0.get(&RosterKey::new(account, character))
    }

    /// Drop what was known about a character — it has been deleted.
    ///
    /// Hands the record back, because the caller needs the serial off it to tell
    /// the world to forget the store row too.
    pub(crate) fn forget(
        &mut self,
        account: &AccountName,
        character: &CharacterName,
    ) -> Option<CharacterRecord> {
        self.0.remove(&RosterKey::new(account, character))
    }

    /// How many characters are on file, for the boot log.
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether nothing has been restored, for the boot log.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record with nothing on it but an identity. Everything the roster does
    /// keys on those three fields; the rest is what it carries, not what it is
    /// found by. Written out rather than `..Default::default()` because
    /// `CharacterRecord` deliberately has no `Default` — a zeroed character is
    /// not a character.
    fn record(account: &str, name: &str, serial: u32) -> CharacterRecord {
        CharacterRecord {
            serial,
            account: AccountName::new(account),
            name: CharacterName::new(name),
            body: 400,
            hue: 0,
            facet: 0,
            x: 0,
            y: 0,
            z: 0,
            facing: 0,
            strength: 10,
            dexterity: 10,
            intelligence: 10,
            skills: Vec::new(),
            stat_locks: openshard_persistence::StatLockRecord::default(),
            effects: Vec::new(),
            dead: false,
            fame: 0,
            karma: 0,
            murders: 0,
            quests: Vec::new(),
            done_quests: Vec::new(),
        }
    }

    #[test]
    fn a_character_is_found_however_the_client_spells_it() {
        // The client sends the name back as the player typed it, and the account
        // name came off a `0x91` field. Both halves are folded, which is the
        // whole reason the key is a type: four call sites used to fold by hand.
        let mut roster = Roster::new();
        roster.remember(record("Admin", "Lord British", 7));

        assert_eq!(
            roster
                .get(&AccountName::new("admin"), &CharacterName::new("lord british"))
                .map(|record| record.serial),
            Some(7)
        );
        assert!(
            roster
                .get(&AccountName::new("admin"), &CharacterName::new("Dupre"))
                .is_none(),
            "and a character nobody saved is absent, not a default"
        );
    }

    #[test]
    fn two_accounts_may_hold_the_same_character_name() {
        // The account is half the key. Folding only the character name would
        // have one player's logout position overwrite another's.
        let mut roster = Roster::new();
        roster.remember(record("alice", "Dupre", 1));
        roster.remember(record("bob", "Dupre", 2));

        assert_eq!(roster.len(), 2);
        assert_eq!(
            roster
                .get(&AccountName::new("bob"), &CharacterName::new("Dupre"))
                .map(|record| record.serial),
            Some(2)
        );
    }

    #[test]
    fn forgetting_hands_back_the_serial_the_world_must_drop() {
        let mut roster = Roster::new();
        roster.remember(record("admin", "Lord British", 7));

        let dropped = roster.forget(&AccountName::new("ADMIN"), &CharacterName::new("LORD BRITISH"));
        assert_eq!(dropped.map(|record| record.serial), Some(7));
        assert!(roster.is_empty());
    }
}
