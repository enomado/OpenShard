//! Everything the server can say: one sum type.
//!
//! # Why an enum and not forty-seven functions
//!
//! The wire format is a fixed external contract with a closed set of messages.
//! A set of free `encode_*` functions returning `Vec<u8>` cannot say that: it
//! cannot be matched over, cannot be logged uniformly, cannot tell you that an
//! id is produced by two encoders that disagree about its length, and gives a
//! test no way to name "the packet the server sent" other than by its bytes.
//!
//! So the closed set is a type. Each variant wraps a payload struct that knows
//! its id, its length and how to write its body — see
//! [`EncodePacket`](crate::packet::EncodePacket) — and this enum is the only
//! thing that turns one into bytes.
//!
//! # It grows one group at a time
//!
//! Non-exhaustive and deliberately incomplete: the packets that have been
//! rewritten are here, the rest are still free functions elsewhere in the crate.
//! `docs/protocol_rewrite.md` tracks which group lands when.

use crate::codec::PacketWriter;
use crate::combat::{AttackTarget, HealthBar, WarMode};
use crate::containers::ContainerContents;
use crate::context::ContextMenu;
use crate::feedback::{Animation, GraphicalEffect, HuedEffect, NewAnimation, PlaySound};
use crate::gump::{CloseGump, GumpDisplay};
use crate::items::{DragCancel, EquipUpdate, WorldItem};
use crate::login::{CharacterList, CharacterListUpdate, DeleteReject, LoginDenied, Relay, ShardList};
use crate::mobile::{MobileIncoming, MobileMove, MobileStatus, OpenPaperdoll, Remove, StatLocks};
use crate::packet::{EncodePacket, PacketLength, frame_body};
use crate::properties::TooltipRevision;
use crate::skill::{SkillUpdate, SkillsFull};
use crate::speech::{LocalizedMessage, SpokenMessage, UnicodeMessage};
use crate::spellbook::SpellbookContent;
use crate::target::TargetCursor;
use crate::vendor::{BuyList, SellList};
use crate::version::ClientVersion;
use crate::world::{
    DeathStatus, LightLevel, LoginComplete, LogoutAck, MapChange, PlayMusic, PlayerStart, PlayerUpdate,
    SeasonChange, WalkAck, WalkReject,
};

/// A packet the server sends to a client.
///
/// Not `Copy`: the login group carries `Vec` payloads (the shard and character
/// lists), unlike Stage 1's fixed-size ones.
#[derive(Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ServerPacket {
    /// `0x6C` — raise a targeting cursor.
    TargetCursor(TargetCursor),
    /// `0x72` — the settled war stance.
    WarMode(WarMode),
    /// `0xAA` — which mobile's bar the client highlights.
    AttackTarget(AttackTarget),
    /// `0xA1` — a mobile's health bar.
    Health(HealthBar),
    /// `0x54` — a sound at a world location.
    PlaySound(PlaySound),
    /// `0x6E` — the classic mobile animation.
    Animation(Animation),
    /// `0xE2` — the 7.0.0.0+ mobile animation.
    NewAnimation(NewAnimation),
    /// `0x70` — an uncoloured graphical effect.
    Effect(GraphicalEffect),
    /// `0xC0` — a graphical effect with a hue and a render mode.
    HuedEffect(HuedEffect),
    /// `0x82` — refuse a login.
    LoginDenied(LoginDenied),
    /// `0xA8` — the shard list.
    ShardList(ShardList),
    /// `0x8C` — go connect to the game server.
    Relay(Relay),
    /// `0xA9` — the character list and starting cities.
    CharacterList(CharacterList),
    /// `0x85` — a character deletion was refused.
    DeleteReject(DeleteReject),
    /// `0x86` — resend the character list after a deletion.
    CharacterListUpdate(CharacterListUpdate),
    /// `0x1B` — put a body in the world.
    PlayerStart(PlayerStart),
    /// `0x20` — move or redraw the player's own body.
    PlayerUpdate(PlayerUpdate),
    /// `0x2C` — the player's own character died, or came back.
    DeathStatus(DeathStatus),
    /// `0x22` — a walk request is allowed.
    WalkAck(WalkAck),
    /// `0x21` — a walk request is refused.
    WalkReject(WalkReject),
    /// `0x55` — the client may start drawing.
    LoginComplete(LoginComplete),
    /// `0x4F` — overall light level.
    LightLevel(LightLevel),
    /// `0x6D` — play a music track.
    PlayMusic(PlayMusic),
    /// `0xBC` — which season the client draws.
    SeasonChange(SeasonChange),
    /// `0xD1` — a logout is granted.
    LogoutAck(LogoutAck),
    /// `0xBF` subcommand `0x08` — which map the client should draw.
    MapChange(MapChange),
    /// `0x1D` — take an object off the client's screen.
    Remove(Remove),
    /// `0x88` — open a mobile's paperdoll.
    OpenPaperdoll(OpenPaperdoll),
    /// `0x11` — a mobile's full status.
    MobileStatus(MobileStatus),
    /// `0x77` — move a mobile the client already knows about.
    MobileMove(MobileMove),
    /// `0x78` — draw a mobile the client has not seen.
    MobileIncoming(MobileIncoming),
    /// `0xBF` subcommand `0x19` type `2` — the three stat-training arrows.
    StatLocks(StatLocks),
    /// `0x1A` — draw an item on the ground the client has not seen.
    WorldItem(WorldItem),
    /// `0x27` — cancel a drag and bounce the item back.
    DragCancel(DragCancel),
    /// `0x2E` — a mobile is now wearing an item.
    EquipUpdate(EquipUpdate),
    /// `0x3C` — the full contents of a container, all at once.
    ContainerContents(ContainerContents),
    /// `0x74` — the prices and labels for a vendor's buy container.
    BuyList(BuyList),
    /// `0x9E` — what a vendor offers to buy from the player.
    SellList(SellList),
    /// `0xDC` — the tooltip revision for one object.
    TooltipRevision(TooltipRevision),
    /// `0x3A` — the whole skill list, to fill the window.
    SkillsFull(SkillsFull),
    /// `0x3A` — one skill's line, following a change.
    SkillUpdate(SkillUpdate),
    /// `0x1C` — speech drawn over a source and put in the journal.
    SpokenMessage(SpokenMessage),
    /// `0xC1` — a localized message: a cliloc and its substitutions.
    LocalizedMessage(LocalizedMessage),
    /// `0xAE` — Unicode speech drawn over a source.
    UnicodeMessage(UnicodeMessage),
    /// `0xBF` subcommand `0x14` — a context menu on an object.
    ContextMenu(ContextMenu),
    /// `0xBF` subcommand `0x1B` — the spells a spellbook holds.
    SpellbookContent(SpellbookContent),
    /// `0xBF` subcommand `0x04` — close an open gump on the client.
    CloseGump(CloseGump),
    /// `0xB0` — display a generic gump.
    GumpDisplay(GumpDisplay),
}

impl ServerPacket {
    /// The id byte this packet goes out under.
    ///
    /// Taken from the payload's own [`EncodePacket::ID`], so there is no second
    /// table to keep in step — and it makes an id available for logging without
    /// encoding anything.
    #[must_use]
    pub const fn id(&self) -> u8 {
        match self {
            Self::TargetCursor(_) => TargetCursor::ID,
            Self::WarMode(_) => <WarMode as EncodePacket>::ID,
            Self::AttackTarget(_) => AttackTarget::ID,
            Self::Health(_) => HealthBar::ID,
            Self::PlaySound(_) => PlaySound::ID,
            Self::Animation(_) => Animation::ID,
            Self::NewAnimation(_) => NewAnimation::ID,
            Self::Effect(_) => GraphicalEffect::ID,
            Self::HuedEffect(_) => HuedEffect::ID,
            Self::LoginDenied(_) => LoginDenied::ID,
            Self::ShardList(_) => ShardList::ID,
            Self::Relay(_) => Relay::ID,
            Self::CharacterList(_) => CharacterList::ID,
            Self::DeleteReject(_) => DeleteReject::ID,
            Self::CharacterListUpdate(_) => CharacterListUpdate::ID,
            Self::PlayerStart(_) => PlayerStart::ID,
            Self::PlayerUpdate(_) => PlayerUpdate::ID,
            Self::DeathStatus(_) => DeathStatus::ID,
            Self::WalkAck(_) => WalkAck::ID,
            Self::WalkReject(_) => WalkReject::ID,
            Self::LoginComplete(_) => LoginComplete::ID,
            Self::LightLevel(_) => LightLevel::ID,
            Self::PlayMusic(_) => PlayMusic::ID,
            Self::SeasonChange(_) => SeasonChange::ID,
            Self::LogoutAck(_) => LogoutAck::ID,
            Self::MapChange(_) => MapChange::ID,
            Self::Remove(_) => Remove::ID,
            Self::OpenPaperdoll(_) => OpenPaperdoll::ID,
            Self::MobileStatus(_) => MobileStatus::ID,
            Self::MobileMove(_) => MobileMove::ID,
            Self::MobileIncoming(_) => MobileIncoming::ID,
            Self::StatLocks(_) => StatLocks::ID,
            Self::WorldItem(_) => WorldItem::ID,
            Self::DragCancel(_) => DragCancel::ID,
            Self::EquipUpdate(_) => EquipUpdate::ID,
            Self::ContainerContents(_) => ContainerContents::ID,
            Self::BuyList(_) => BuyList::ID,
            Self::SellList(_) => SellList::ID,
            Self::TooltipRevision(_) => TooltipRevision::ID,
            Self::SkillsFull(_) => SkillsFull::ID,
            Self::SkillUpdate(_) => SkillUpdate::ID,
            Self::SpokenMessage(_) => SpokenMessage::ID,
            Self::LocalizedMessage(_) => LocalizedMessage::ID,
            Self::UnicodeMessage(_) => UnicodeMessage::ID,
            Self::ContextMenu(_) => ContextMenu::ID,
            Self::SpellbookContent(_) => SpellbookContent::ID,
            Self::CloseGump(_) => CloseGump::ID,
            Self::GumpDisplay(_) => GumpDisplay::ID,
        }
    }

    /// How the packet is framed: a fixed size, or a length field to patch.
    #[must_use]
    pub const fn length(&self) -> PacketLength {
        match self {
            Self::TargetCursor(_) => TargetCursor::LENGTH,
            Self::WarMode(_) => <WarMode as EncodePacket>::LENGTH,
            Self::AttackTarget(_) => AttackTarget::LENGTH,
            Self::Health(_) => HealthBar::LENGTH,
            Self::PlaySound(_) => PlaySound::LENGTH,
            Self::Animation(_) => Animation::LENGTH,
            Self::NewAnimation(_) => NewAnimation::LENGTH,
            Self::Effect(_) => GraphicalEffect::LENGTH,
            Self::HuedEffect(_) => HuedEffect::LENGTH,
            Self::LoginDenied(_) => LoginDenied::LENGTH,
            Self::ShardList(_) => ShardList::LENGTH,
            Self::Relay(_) => Relay::LENGTH,
            Self::CharacterList(_) => CharacterList::LENGTH,
            Self::DeleteReject(_) => DeleteReject::LENGTH,
            Self::CharacterListUpdate(_) => CharacterListUpdate::LENGTH,
            Self::PlayerStart(_) => PlayerStart::LENGTH,
            Self::PlayerUpdate(_) => PlayerUpdate::LENGTH,
            Self::DeathStatus(_) => DeathStatus::LENGTH,
            Self::WalkAck(_) => WalkAck::LENGTH,
            Self::WalkReject(_) => WalkReject::LENGTH,
            Self::LoginComplete(_) => LoginComplete::LENGTH,
            Self::LightLevel(_) => LightLevel::LENGTH,
            Self::PlayMusic(_) => PlayMusic::LENGTH,
            Self::SeasonChange(_) => SeasonChange::LENGTH,
            Self::LogoutAck(_) => LogoutAck::LENGTH,
            Self::MapChange(_) => MapChange::LENGTH,
            Self::Remove(_) => Remove::LENGTH,
            Self::OpenPaperdoll(_) => OpenPaperdoll::LENGTH,
            Self::MobileStatus(_) => MobileStatus::LENGTH,
            Self::MobileMove(_) => MobileMove::LENGTH,
            Self::MobileIncoming(_) => MobileIncoming::LENGTH,
            Self::StatLocks(_) => StatLocks::LENGTH,
            Self::WorldItem(_) => WorldItem::LENGTH,
            Self::DragCancel(_) => DragCancel::LENGTH,
            Self::EquipUpdate(_) => EquipUpdate::LENGTH,
            Self::ContainerContents(_) => ContainerContents::LENGTH,
            Self::BuyList(_) => BuyList::LENGTH,
            Self::SellList(_) => SellList::LENGTH,
            Self::TooltipRevision(_) => TooltipRevision::LENGTH,
            Self::SkillsFull(_) => SkillsFull::LENGTH,
            Self::SkillUpdate(_) => SkillUpdate::LENGTH,
            Self::SpokenMessage(_) => SpokenMessage::LENGTH,
            Self::LocalizedMessage(_) => LocalizedMessage::LENGTH,
            Self::UnicodeMessage(_) => UnicodeMessage::LENGTH,
            Self::ContextMenu(_) => ContextMenu::LENGTH,
            Self::SpellbookContent(_) => SpellbookContent::LENGTH,
            Self::CloseGump(_) => CloseGump::LENGTH,
            Self::GumpDisplay(_) => GumpDisplay::LENGTH,
        }
    }

    /// The bytes to put on the wire, framed for `version`.
    ///
    /// The header — id, and the length field where there is one — is written by
    /// [`frame_body`] and by nothing else, so no payload can forget it.
    #[must_use]
    pub fn encode(&self, version: ClientVersion) -> Vec<u8> {
        frame_body(self.id(), self.length(), |out| {
            self.encode_body(out, version);
        })
    }

    /// Dispatch the body write to the payload.
    fn encode_body(&self, out: &mut PacketWriter, version: ClientVersion) {
        match self {
            Self::TargetCursor(packet) => packet.encode_body(out, version),
            Self::WarMode(packet) => packet.encode_body(out, version),
            Self::AttackTarget(packet) => packet.encode_body(out, version),
            Self::Health(packet) => packet.encode_body(out, version),
            Self::PlaySound(packet) => packet.encode_body(out, version),
            Self::Animation(packet) => packet.encode_body(out, version),
            Self::NewAnimation(packet) => packet.encode_body(out, version),
            Self::Effect(packet) => packet.encode_body(out, version),
            Self::HuedEffect(packet) => packet.encode_body(out, version),
            Self::LoginDenied(packet) => packet.encode_body(out, version),
            Self::ShardList(packet) => packet.encode_body(out, version),
            Self::Relay(packet) => packet.encode_body(out, version),
            Self::CharacterList(packet) => packet.encode_body(out, version),
            Self::DeleteReject(packet) => packet.encode_body(out, version),
            Self::CharacterListUpdate(packet) => packet.encode_body(out, version),
            Self::PlayerStart(packet) => packet.encode_body(out, version),
            Self::PlayerUpdate(packet) => packet.encode_body(out, version),
            Self::DeathStatus(packet) => packet.encode_body(out, version),
            Self::WalkAck(packet) => packet.encode_body(out, version),
            Self::WalkReject(packet) => packet.encode_body(out, version),
            Self::LoginComplete(packet) => packet.encode_body(out, version),
            Self::LightLevel(packet) => packet.encode_body(out, version),
            Self::PlayMusic(packet) => packet.encode_body(out, version),
            Self::SeasonChange(packet) => packet.encode_body(out, version),
            Self::LogoutAck(packet) => packet.encode_body(out, version),
            Self::MapChange(packet) => packet.encode_body(out, version),
            Self::Remove(packet) => packet.encode_body(out, version),
            Self::OpenPaperdoll(packet) => packet.encode_body(out, version),
            Self::MobileStatus(packet) => packet.encode_body(out, version),
            Self::MobileMove(packet) => packet.encode_body(out, version),
            Self::MobileIncoming(packet) => packet.encode_body(out, version),
            Self::StatLocks(packet) => packet.encode_body(out, version),
            Self::WorldItem(packet) => packet.encode_body(out, version),
            Self::DragCancel(packet) => packet.encode_body(out, version),
            Self::EquipUpdate(packet) => packet.encode_body(out, version),
            Self::ContainerContents(packet) => packet.encode_body(out, version),
            Self::BuyList(packet) => packet.encode_body(out, version),
            Self::SellList(packet) => packet.encode_body(out, version),
            Self::TooltipRevision(packet) => packet.encode_body(out, version),
            Self::SkillsFull(packet) => packet.encode_body(out, version),
            Self::SkillUpdate(packet) => packet.encode_body(out, version),
            Self::SpokenMessage(packet) => packet.encode_body(out, version),
            Self::LocalizedMessage(packet) => packet.encode_body(out, version),
            Self::UnicodeMessage(packet) => packet.encode_body(out, version),
            Self::ContextMenu(packet) => packet.encode_body(out, version),
            Self::SpellbookContent(packet) => packet.encode_body(out, version),
            Self::CloseGump(packet) => packet.encode_body(out, version),
            Self::GumpDisplay(packet) => packet.encode_body(out, version),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::encode_packet;
    use crate::serial::Serial;
    use crate::target::TargetKind;
    use crate::wire::{AuthKey, CursorId};

    fn version() -> ClientVersion {
        ClientVersion::new(7, 0, 45, 65)
    }

    /// One of every variant, so a new variant that lies about its id or length
    /// has to be added here to compile.
    fn one_of_each() -> Vec<ServerPacket> {
        let serial = Serial::new(0x0000_002A).unwrap();
        let effect = GraphicalEffect {
            kind: crate::feedback::EffectKind::Moving,
            from: Some(serial),
            to: None,
            art: crate::wire::Graphic(0x36D4),
            from_point: crate::world::Point::new(1, 2, 3),
            to_point: crate::world::Point::new(4, 5, 6),
            speed: 7,
            duration: 0,
            fixed_direction: false,
            explode: false,
        };
        vec![
            ServerPacket::TargetCursor(TargetCursor {
                cursor_id: CursorId(1),
                kind: TargetKind::Object,
            }),
            ServerPacket::WarMode(WarMode { war: true }),
            ServerPacket::AttackTarget(AttackTarget { target: Some(serial) }),
            ServerPacket::Health(HealthBar::exact(serial, 100, 50)),
            ServerPacket::PlaySound(PlaySound {
                sound: crate::wire::SoundId(0x28),
                at: crate::world::Point::new(1, 2, 3),
            }),
            ServerPacket::Animation(Animation {
                serial,
                action: 1,
                frame_count: 5,
                repeat_count: 1,
                forward: true,
                repeat: false,
                delay: 0,
            }),
            ServerPacket::NewAnimation(NewAnimation {
                serial,
                animation_type: 1,
                action: 0,
                delay: 0,
            }),
            ServerPacket::Effect(effect),
            ServerPacket::HuedEffect(HuedEffect {
                effect,
                hue: crate::wire::Hue(0x26),
                render_mode: 0,
            }),
            ServerPacket::LoginDenied(LoginDenied {
                reason: crate::login::DenyReason::BadPassword,
            }),
            ServerPacket::ShardList(ShardList {
                shards: vec![crate::login::ShardEntry {
                    name: "Britannia".to_owned(),
                    percent_full: 10,
                    timezone: 5,
                    address: std::net::Ipv4Addr::new(127, 0, 0, 1),
                }],
            }),
            ServerPacket::Relay(Relay {
                address: std::net::Ipv4Addr::new(127, 0, 0, 1),
                port: 2593,
                auth_key: AuthKey(0xDEAD_BEEF),
            }),
            ServerPacket::CharacterList(CharacterList {
                characters: vec![crate::login::CharacterEntry {
                    name: crate::identity::CharacterName("Lord British".to_owned()),
                }],
                starts: Vec::new(),
                flags: 0,
            }),
            ServerPacket::DeleteReject(DeleteReject {
                result: crate::login::DeleteResult::CharNotExist,
            }),
            ServerPacket::CharacterListUpdate(CharacterListUpdate {
                characters: vec![crate::login::CharacterEntry {
                    name: crate::identity::CharacterName("Lord British".to_owned()),
                }],
            }),
            ServerPacket::PlayerStart(PlayerStart {
                serial,
                body: crate::wire::Graphic(0x0190),
                position: crate::world::Point::new(1475, 1774, 0),
                facing: crate::direction::Facing::walking(crate::direction::Direction::South),
                map: crate::world::MapSize::BRITANNIA,
            }),
            ServerPacket::PlayerUpdate(PlayerUpdate {
                serial,
                body: crate::wire::Graphic(0x0190),
                hue: crate::wire::Hue(0x83EA),
                flags: crate::mobile::StatusFlags::NONE,
                position: crate::world::Point::new(1475, 1774, 0),
                facing: crate::direction::Facing::walking(crate::direction::Direction::South),
            }),
            ServerPacket::DeathStatus(DeathStatus { dead: true }),
            ServerPacket::WalkAck(WalkAck {
                sequence: crate::world::StepSequence(1),
                notoriety: crate::mobile::Notoriety::Innocent,
            }),
            ServerPacket::WalkReject(WalkReject {
                sequence: crate::world::StepSequence(1),
                position: crate::world::Point::new(1475, 1774, 0),
                facing: crate::direction::Facing::walking(crate::direction::Direction::South),
            }),
            ServerPacket::LoginComplete(LoginComplete),
            ServerPacket::LightLevel(LightLevel {
                level: crate::world::Light(0),
            }),
            ServerPacket::PlayMusic(PlayMusic {
                track: crate::world::MusicId(11),
            }),
            ServerPacket::SeasonChange(SeasonChange {
                season: crate::world::Season::Spring,
                play_sound: false,
            }),
            ServerPacket::LogoutAck(LogoutAck),
            ServerPacket::MapChange(MapChange {
                map: crate::world::MapId(0),
            }),
            ServerPacket::Remove(Remove { serial }),
            ServerPacket::OpenPaperdoll(OpenPaperdoll {
                serial,
                text: "Lord British".to_owned(),
                flags: crate::mobile::PaperdollFlags::NONE,
            }),
            ServerPacket::MobileStatus(MobileStatus {
                serial,
                name: "Lord British".to_owned(),
                hits: crate::mobile::Vitals {
                    current: 100,
                    max: 100,
                },
                female: false,
                strength: 100,
                dexterity: 90,
                intelligence: 80,
                stamina: crate::mobile::Vitals { current: 90, max: 90 },
                mana: crate::mobile::Vitals { current: 80, max: 80 },
                gold: 1234,
                armor: 0,
                weight: 14,
                max_weight: 390,
                stat_cap: 225,
                followers: 0,
                followers_max: 5,
            }),
            ServerPacket::MobileMove(MobileMove {
                serial,
                body: crate::wire::Graphic(0x0190),
                position: crate::world::Point::new(1475, 1774, 0),
                facing: crate::direction::Facing::walking(crate::direction::Direction::South),
                hue: crate::wire::Hue(0x83EA),
                flags: crate::mobile::StatusFlags::NONE,
                notoriety: crate::mobile::Notoriety::Innocent,
            }),
            ServerPacket::MobileIncoming(MobileIncoming {
                serial,
                body: crate::wire::Graphic(0x0190),
                position: crate::world::Point::new(1475, 1774, 0),
                facing: crate::direction::Facing::walking(crate::direction::Direction::South),
                hue: crate::wire::Hue(0x83EA),
                flags: crate::mobile::StatusFlags::NONE,
                notoriety: crate::mobile::Notoriety::Innocent,
                equipment: Vec::new(),
            }),
            ServerPacket::StatLocks(StatLocks {
                serial,
                locks: crate::mobile::StatLockBits::default(),
            }),
            ServerPacket::WorldItem(crate::items::WorldItem {
                serial: crate::serial::Serial::new(0x4000_0001).unwrap(),
                graphic: crate::wire::Graphic(0x0EED),
                amount: 1,
                position: crate::world::Point::new(1000, 2000, 5),
                hue: crate::wire::Hue::NONE,
            }),
            ServerPacket::DragCancel(crate::items::DragCancel {
                reason: crate::items::DragCancelReason::OutOfRange,
            }),
            ServerPacket::EquipUpdate(crate::items::EquipUpdate {
                item: crate::serial::Serial::new(0x4000_0002).unwrap(),
                graphic: crate::wire::Graphic(0x13B9),
                layer: crate::wire::Layer(1),
                mobile: crate::serial::Serial::new(0x0000_0001).unwrap(),
                hue: crate::wire::Hue(0x0021),
            }),
            ServerPacket::ContainerContents(crate::containers::ContainerContents {
                container: crate::serial::Serial::new(0x4000_0001).unwrap(),
                items: Vec::new(),
            }),
            ServerPacket::BuyList(crate::vendor::BuyList {
                container: crate::serial::Serial::new(0x4000_0010).unwrap(),
                lines: vec![crate::vendor::BuyLine {
                    price: 3,
                    name: "black pearl".to_owned(),
                }],
            }),
            ServerPacket::SellList(crate::vendor::SellList {
                vendor: crate::serial::Serial::new(0x0000_0BBB).unwrap(),
                lines: vec![crate::vendor::SellLine {
                    serial: crate::serial::Serial::new(0x4000_0033).unwrap(),
                    graphic: crate::wire::Graphic(0x0F7A),
                    hue: crate::wire::Hue::NONE,
                    amount: 20,
                    price: 2,
                    name: "black pearl".to_owned(),
                }],
            }),
            ServerPacket::TooltipRevision(crate::properties::TooltipRevision {
                serial: 0x0000_00AB,
                hash: 0x1234_5678,
            }),
            ServerPacket::SkillsFull(crate::skill::SkillsFull {
                entries: vec![crate::skill::SkillEntry {
                    id: 0,
                    value: 755,
                    base: 700,
                    lock: crate::skill::SkillLock::Locked,
                    cap: 1000,
                }],
            }),
            ServerPacket::SkillUpdate(crate::skill::SkillUpdate {
                entry: crate::skill::SkillEntry {
                    id: 25,
                    value: 501,
                    base: 501,
                    lock: crate::skill::SkillLock::Up,
                    cap: 1000,
                },
            }),
            ServerPacket::SpokenMessage(crate::speech::SpokenMessage {
                serial: crate::serial::Serial::new(0x0000_0002),
                graphic: Some(crate::wire::Graphic(0x0190)),
                mode: crate::speech::TalkMode::Regular,
                hue: crate::wire::Hue(0x0384),
                font: crate::speech::Font(3),
                name: "British".to_owned(),
                text: "hail".to_owned(),
            }),
            ServerPacket::LocalizedMessage(crate::speech::LocalizedMessage {
                serial: None,
                graphic: None,
                mode: crate::speech::TalkMode::Regular,
                hue: crate::wire::Hue(0x03B2),
                font: crate::speech::Font(3),
                cliloc: crate::wire::ClilocId(1_042_764),
                name: "System".to_owned(),
                arguments: "Iolo".to_owned(),
            }),
            ServerPacket::UnicodeMessage(crate::speech::UnicodeMessage {
                serial: crate::serial::Serial::new(0x0000_0002),
                graphic: Some(crate::wire::Graphic(0x0190)),
                mode: crate::speech::TalkMode::Regular,
                hue: crate::wire::Hue(0x0384),
                font: crate::speech::Font(3),
                language: "PTB".to_owned(),
                name: "Cidadão".to_owned(),
                text: "olá".to_owned(),
            }),
            ServerPacket::ContextMenu(crate::context::ContextMenu {
                serial: crate::serial::Serial::new(0x0000_00AB).unwrap(),
                entries: vec![crate::context::ContextMenuEntry {
                    cliloc: crate::wire::ClilocId(3_000_362),
                    flags: crate::context::ContextMenuFlags::NONE,
                }],
            }),
            ServerPacket::SpellbookContent(crate::spellbook::SpellbookContent {
                serial: 0x4000_0001,
                graphic: 0x0EFA,
                offset: 1,
                content: 1,
            }),
            ServerPacket::CloseGump(crate::gump::CloseGump {
                gump_id: crate::gump::GumpId(0x0051_0001),
                button: crate::gump::ButtonId::CLOSE_BOX,
            }),
            ServerPacket::GumpDisplay(crate::gump::GumpDisplay {
                serial: crate::gump::GumpKey::STANDALONE,
                gump_id: crate::gump::GumpId(0x0051_0001),
                at: crate::gump::GumpPoint::new(75, 25),
                layout: "{ page 0 }".to_owned(),
                lines: Vec::new(),
            }),
        ]
    }

    #[test]
    fn every_variant_writes_the_id_it_claims() {
        for packet in one_of_each() {
            let bytes = packet.encode(version());
            assert_eq!(bytes[0], packet.id(), "{packet:?}");
        }
    }

    #[test]
    fn every_fixed_variant_writes_exactly_its_declared_length() {
        // The check `frame_body` makes in debug builds, made unconditional and
        // over every variant: a field added to a payload and forgotten in its
        // encoder shows up here.
        for packet in one_of_each() {
            let bytes = packet.encode(version());
            match packet.length() {
                PacketLength::Fixed(size) => {
                    assert_eq!(bytes.len(), size as usize, "{packet:?}");
                }
                PacketLength::Variable => {
                    assert_eq!(
                        u16::from_be_bytes([bytes[1], bytes[2]]) as usize,
                        bytes.len(),
                        "{packet:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_enum_and_the_payload_agree_byte_for_byte() {
        // Going through the enum must not add or reorder anything: the variant is
        // a wrapper, not a second encoder.
        let war = WarMode { war: true };
        assert_eq!(
            ServerPacket::WarMode(war).encode(version()),
            encode_packet(&war, version())
        );
    }
}
