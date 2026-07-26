//! The skill window's wire side: filling it on login, following a gain, and
//! reading the arrows — the skill window's, and the status bar's three stat ones.
//!
//! The rules — the roll, the gain curve, the lock's meaning — are the `skills`
//! crate's. This is only the `0x3A` traffic that shows them to a client, kept
//! here in `world` because drawing a mobile's state on a screen is the world's
//! job, the same seam `send_status` (`0x11`) sits on.

use super::*;
use openshard_protocol::{
    encode_skill_update, encode_skills_full, skill_count, Feature, SkillEntry, SkillLock,
};
use openshard_skills::SkillChanged;
use openshard_state::components::{Skills, StatLocks};
use openshard_state::StatLock;

impl World {
    /// A client moved a skill's up/down/lock arrow: store it. ServUO's
    /// `SetLockNoRelay` — the client already redrew its own arrow, so nothing is
    /// sent back.
    pub(super) fn set_skill_lock(&mut self, connection: ConnectionId, skill: u8, lock: SkillLock) {
        let Some(&player) = self.state.players.get(&connection) else {
            return;
        };
        let mut skills = self
            .state
            .registry
            .get::<Skills>(player)
            .cloned()
            .unwrap_or_default();
        skills.set_lock(skill, lock);
        self.state.registry.insert(player, skills);
    }

    /// A client moved one of the status bar's three stat arrows: store it, and
    /// send the bits back so every client agrees on what it drew.
    ///
    /// Unlike a *skill* arrow this is relayed. ServUO's skill window redraws its
    /// own arrow before the packet leaves, so it needs no answer; the status bar's
    /// arrows come out of a separate `0xBF 0x19` the server has to send, and a
    /// client that never receives one draws all three pointing up.
    pub(super) fn set_stat_lock(&mut self, connection: ConnectionId, stat: u8, lock: StatLock) {
        let Some(&player) = self.state.players.get(&connection) else {
            return;
        };
        let mut locks = self
            .state
            .registry
            .get::<StatLocks>(player)
            .copied()
            .unwrap_or_default();
        match stat {
            0 => locks.strength = lock,
            1 => locks.dexterity = lock,
            2 => locks.intelligence = lock,
            _ => return, // there is no fourth stat
        }
        self.state.registry.insert(player, locks);
        self.send_stat_locks(connection, player);
    }

    /// Tell a client which way its stats train, so the bar draws the arrows.
    pub(super) fn send_stat_locks(&mut self, connection: ConnectionId, entity: EntityId) {
        let Some(serial) = self.state.registry.serial_of(entity) else {
            return;
        };
        let locks = self
            .state
            .registry
            .get::<StatLocks>(entity)
            .copied()
            .unwrap_or_default();
        let packet = openshard_protocol::encode_stat_locks(
            serial.raw(),
            openshard_protocol::StatLockBits {
                strength: locks.strength.to_bits(),
                dexterity: locks.dexterity.to_bits(),
                intelligence: locks.intelligence.to_bits(),
            },
        );
        self.state.send(connection, packet);
    }

    /// One skill's line for the window.
    ///
    /// `value` and `base` are genuinely different numbers now: the base is what is
    /// trained, and the value is what the skill is *worth* once the mobile's stats
    /// have lent it their share (`skills::skill_value`). Both fields existed on the
    /// wire from the start and carried the same number, which is the sort of gap a
    /// client cannot report — it simply draws two identical figures.
    fn skill_entry(&self, entity: EntityId, id: u8) -> SkillEntry {
        let skills = self.state.registry.get::<Skills>(entity);
        SkillEntry {
            id,
            value: openshard_skills::skill_value(&self.state, entity, id),
            base: skills.map_or(0, |s| s.get(id)),
            lock: skills.map_or(SkillLock::Up, |s| s.lock(id)),
            // Per skill, not one number for the sheet: a cap is the mobile's own,
            // and only falls back to the shard's for a mobile with no sheet yet.
            cap: skills.map_or(self.state.gameplay.skill_cap, |s| s.cap(id)),
        }
    }

    /// A mobile's whole skill line-up for the `0x3A` window: every skill the
    /// client of `version` knows, trained or not, at its value, lock and cap.
    fn skill_entries(&self, entity: EntityId, version: ClientVersion) -> Vec<SkillEntry> {
        (0..skill_count(version) as u8)
            .map(|id| self.skill_entry(entity, id))
            .collect()
    }

    /// Send a player its whole skill list, to fill the window — on login, the
    /// way ServUO sends `SkillUpdate` on world entry.
    pub(super) fn send_skills(&mut self, connection: ConnectionId, entity: EntityId) {
        let Some(&Client { version, .. }) = self.state.registry.get::<Client>(entity) else {
            return;
        };
        let entries = self.skill_entries(entity, version);
        let packet = encode_skills_full(&entries, version.supports(Feature::SkillCaps));
        self.state.send(connection, packet);
    }

    /// Push the single-line `0x3A` update for each skill that moved this tick, so
    /// an open window follows live. Reads the `SkillChanged` the gain emits — for
    /// a skill that *fell* as much as one that rose, since a "down" arrow gives
    /// ground the moment another skill needs the room.
    pub(super) fn send_skill_updates(&mut self) {
        let moved: Vec<SkillChanged> = self.state.bus.read(&mut self.changed).copied().collect();
        for event in moved {
            let Some(&Client {
                connection,
                version,
            }) = self.state.registry.get::<Client>(event.entity)
            else {
                continue; // a creature training a skill has no window to update
            };
            let entry = self.skill_entry(event.entity, event.skill);
            let packet = encode_skill_update(&entry, version.supports(Feature::SkillCaps));
            self.state.send(connection, packet);
        }
    }
}

impl World {
    /// The core's own answer to a double-clicked item that a *skill* knows what to
    /// do with — ServUO's per-item `OnDoubleClick` overrides.
    ///
    /// The counterpart of `handlers::start` for the items that are their own skill
    /// button: an instrument is struck up rather than pressed for. It runs after
    /// the `ItemUsed` event the pack sees, so the split stays "default in core,
    /// customise in the pack" — a pack that gives a graphic its own meaning has
    /// already been heard.
    pub(super) fn use_item_skill(&mut self, player: EntityId, item: EntityId) {
        let Some(graphic) = self.state.registry.get::<Graphic>(item).map(|g| g.id) else {
            return;
        };
        if openshard_state::instrument::instrument_data(graphic).is_some() {
            skills::play_instrument(&mut self.state, player, item);
        }
    }
}
