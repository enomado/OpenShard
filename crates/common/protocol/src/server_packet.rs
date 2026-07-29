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
use crate::feedback::{Animation, GraphicalEffect, HuedEffect, NewAnimation, PlaySound};
use crate::packet::{frame_body, EncodePacket, PacketLength};
use crate::target::TargetCursor;
use crate::version::ClientVersion;

/// A packet the server sends to a client.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::encode_packet;
    use crate::serial::Serial;
    use crate::target::TargetKind;
    use crate::wire::CursorId;

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
            ServerPacket::AttackTarget(AttackTarget {
                target: Some(serial),
            }),
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
