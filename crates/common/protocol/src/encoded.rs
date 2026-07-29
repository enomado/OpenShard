//! `0xD7` — the AoS "encoded command", a family of client requests keyed by a
//! subcommand word.
//!
//! The paperdoll's Quest and Guild buttons are here, and nowhere else: they are
//! not gump replies (the paperdoll is drawn entirely client-side and has no
//! server-sent layout to answer), so a shard that does not read `0xD7` has a
//! paperdoll with two dead buttons and no way to tell. The layout is
//! `[0xD7][length u16][serial u32][subcommand u16][payload]`, from ServUO's
//! `PacketHandlers.EncodedCommand` and Sphere's `Event_ExtCmd` equivalent — the
//! two agree.

use crate::error::{DecodeError, expect_id};

/// `0xD7` — a client request named by its subcommand.
///
/// Only the header is decoded. Every subcommand this engine acts on carries no
/// payload; one that does can read the rest itself rather than making this type
/// know about all of them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EncodedCommand {
    /// The entity the command is about — the player's own serial, for the
    /// paperdoll buttons.
    pub serial: u32,
    /// Which command. See the `*_REQUEST` constants.
    pub subcommand: u16,
}

impl EncodedCommand {
    /// The packet id.
    pub const ID: u8 = 0xD7;

    /// Set a weapon's special ability (AoS). Not acted on: combat has no
    /// abilities yet. Named so the byte layout is not re-derived when it does.
    pub const SET_ABILITY: u16 = 0x19;

    /// The paperdoll's Guild button. Not acted on: `guilds` is a stub.
    pub const GUILD_GUMP_REQUEST: u16 = 0x28;

    /// The paperdoll's Quest button — open the quest log.
    pub const QUEST_GUMP_REQUEST: u16 = 0x32;

    /// Decode the header of a `0xD7`.
    ///
    /// Every field is read through the bounds-checked reader, so a truncated
    /// packet is an error rather than a panic — the length on the wire is the
    /// client's word, not this end's.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut reader = expect_id(bytes, Self::ID)?;
        // The packet carries its own u16 length at offset 1; the framer already
        // sized the slice, so it is read past rather than trusted.
        reader.u16()?;
        let serial = reader.u32()?;
        let subcommand = reader.u16()?;
        Ok(Self { serial, subcommand })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(serial: u32, subcommand: u16) -> Vec<u8> {
        let mut bytes = vec![0xD7u8, 0, 0];
        bytes.extend_from_slice(&serial.to_be_bytes());
        bytes.extend_from_slice(&subcommand.to_be_bytes());
        let length = u16::try_from(bytes.len()).unwrap();
        bytes[1..3].copy_from_slice(&length.to_be_bytes());
        bytes
    }

    #[test]
    fn an_encoded_command_reads_its_serial_and_subcommand() {
        let got = EncodedCommand::decode(&packet(0x0000_1234, EncodedCommand::QUEST_GUMP_REQUEST)).unwrap();
        assert_eq!(got.serial, 0x0000_1234);
        assert_eq!(got.subcommand, EncodedCommand::QUEST_GUMP_REQUEST);
    }

    #[test]
    fn a_truncated_encoded_command_is_refused_not_panicked() {
        let full = packet(1, EncodedCommand::GUILD_GUMP_REQUEST);
        for cut in 1..full.len() {
            assert!(
                EncodedCommand::decode(&full[..cut]).is_err(),
                "a {cut}-byte packet must not decode"
            );
        }
    }

    #[test]
    fn another_packet_id_is_not_an_encoded_command() {
        let mut bytes = packet(1, EncodedCommand::SET_ABILITY);
        bytes[0] = 0xD6;
        assert!(EncodedCommand::decode(&bytes).is_err());
    }
}
