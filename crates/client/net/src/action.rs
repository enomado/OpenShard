//! Intentions a client sends after it has entered the world.
//!
//! The app names an action; this module owns the correspondence between that
//! action and the protocol encoder.  A socket driver therefore has one entry
//! point for ordinary outgoing traffic, while walking remains separate because
//! its sequence and prediction state are owned by [`crate::walk::Walk`].

use openshard_protocol::gump::{RawButtonId, RawGumpId, RawGumpKey, RawSwitchId};
use openshard_protocol::serial::Serial;
use openshard_protocol::skill::SkillLock;
use openshard_protocol::wire::RawSkillId;

/// A reply to a shard-owned gump.
#[derive(Clone, Debug)]
pub struct GumpReply {
    pub key: RawGumpKey,
    pub gump_id: RawGumpId,
    pub button: RawButtonId,
    pub switches: Vec<RawSwitchId>,
    pub text_entries: Vec<(u16, String)>,
}

/// An outgoing action that has no walk-handshake state of its own.
#[derive(Clone, Debug)]
pub enum Outgoing {
    Say(String),
    AnswerGump(GumpReply),
    Use(Serial),
    WarMode(bool),
    Attack(Serial),
    StopAttacking,
    LogOut,
    Status(Serial),
    Skills(Serial),
    QuestLog,
    GuildMenu,
    Virtue(Serial),
    SkillLock { skill: RawSkillId, lock: SkillLock },
    UseSkill(RawSkillId),
}

impl Outgoing {
    /// Encode this action for `player`, whose identity only the established
    /// session knows. Quest, guild, and virtue requests use it on the wire.
    #[must_use]
    pub fn encode(self, player: Serial) -> Vec<u8> {
        match self {
            Self::Say(text) => crate::talk::say(&text),
            Self::AnswerGump(reply) => crate::talk::answer_gump(
                reply.key,
                reply.gump_id,
                reply.button,
                reply.switches,
                reply.text_entries,
            ),
            Self::Use(serial) => crate::interact::use_object(serial),
            Self::WarMode(war) => crate::doll::war_mode(war),
            Self::Attack(mobile) => crate::combat::attack(mobile),
            Self::StopAttacking => crate::combat::stop_attacking(),
            Self::LogOut => crate::doll::log_out(),
            Self::Status(mobile) => crate::doll::status(mobile),
            Self::Skills(mobile) => crate::doll::skills(mobile),
            Self::QuestLog => crate::doll::quest_log(player),
            Self::GuildMenu => crate::doll::guild_menu(player),
            Self::Virtue(mobile) => crate::doll::virtue(player, mobile),
            Self::SkillLock { skill, lock } => crate::skill::set_lock(skill, lock),
            Self::UseSkill(skill) => crate::skill::use_skill(skill),
        }
    }
}

#[cfg(test)]
mod tests {
    use openshard_protocol::serial::Serial;

    use super::*;

    #[test]
    fn a_world_action_uses_the_encoder_that_owns_its_packet() {
        let player = Serial::new(0x0000_002A).unwrap();
        let target = Serial::new(0x0000_002B).unwrap();
        assert_eq!(
            Outgoing::Attack(target).encode(player),
            crate::combat::attack(target)
        );
        assert_eq!(
            Outgoing::StopAttacking.encode(player),
            crate::combat::stop_attacking()
        );
        assert_eq!(Outgoing::QuestLog.encode(player), crate::doll::quest_log(player));
    }
}
