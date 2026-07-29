//! Combat packets: war mode, attacking, and a mobile's health.

use crate::codec::{PacketReader, PacketWriter};
use crate::error::DecodeError;
use crate::packet::{DecodePacket, EncodePacket, PacketLength};
use crate::serial::{raw_or_none, Serial};
use crate::version::ClientVersion;

/// `0x72` — enter or leave war mode. 5 bytes, the same shape both ways.
///
/// The client sends its desired stance and the server sends back the settled
/// one. The trailing `00 32 00` is fixed padding Sphere sends verbatim; only the
/// first byte, the war flag, means anything.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WarMode {
    /// True for war, false for peace.
    pub war: bool,
}

impl EncodePacket for WarMode {
    const ID: u8 = 0x72;
    const LENGTH: PacketLength = PacketLength::Fixed(5);

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.bool(self.war);
        out.u8(0x00);
        out.u8(0x32);
        out.u8(0x00);
    }
}

impl DecodePacket for WarMode {
    const ID: u8 = 0x72;

    fn decode_body(
        reader: &mut PacketReader<'_>,
        _version: ClientVersion,
    ) -> Result<Self, DecodeError> {
        Ok(Self {
            war: reader.bool()?,
        })
    }
}

/// `0x05` — the client asks to attack a mobile. 5 bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AttackRequest {
    /// Whom to attack, or `None` when the serial names nothing that can be
    /// addressed.
    ///
    /// A client sends this with an out-of-range serial — a zero, a `0xFFFFFFFF` —
    /// to mean "stop attacking", and that is not a malformed packet: the answer
    /// is to clear the aim, not to drop the connection.
    pub target: Option<Serial>,
}

impl DecodePacket for AttackRequest {
    const ID: u8 = 0x05;

    fn decode_body(
        reader: &mut PacketReader<'_>,
        _version: ClientVersion,
    ) -> Result<Self, DecodeError> {
        Ok(Self {
            target: Serial::new(reader.u32()?),
        })
    }
}

/// `0xAA` — set the client's attack target, the mobile whose bar it highlights.
/// 5 bytes. `None` clears it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AttackTarget {
    /// Whose bar to highlight, or `None` to un-highlight.
    pub target: Option<Serial>,
}

impl EncodePacket for AttackTarget {
    const ID: u8 = 0xAA;
    const LENGTH: PacketLength = PacketLength::Fixed(5);

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.u32(raw_or_none(self.target));
    }
}

/// `0xA1` — a mobile's health bar. 9 bytes.
///
/// # Two truths, by who is looking
///
/// You see your own hit points exactly; you see everyone else's only as a bar.
/// So the numbers on the wire depend on the recipient, and the two constructors
/// name that choice: [`HealthBar::exact`] for the packet that goes to the mobile
/// itself, [`HealthBar::scaled`] for the watchers, which sends `100` and a
/// percentage so a client can draw a stranger's health without ever being told
/// the numbers.
///
/// Ported from Sphere's `PacketHealthUpdate`, which despite its `STAT_STR` name
/// is the hit-points bar — UO maps the two.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HealthBar {
    /// Whose bar.
    pub serial: Serial,
    /// The maximum the bar is drawn against — the real one, or `100`.
    pub max: u16,
    /// The filled part, in the same units as `max`.
    pub current: u16,
}

impl HealthBar {
    /// The real numbers, for the mobile's own client.
    #[must_use]
    pub const fn exact(serial: Serial, max: u16, current: u16) -> Self {
        Self {
            serial,
            max,
            current,
        }
    }

    /// A 0–100 bar, for everyone else.
    #[must_use]
    pub const fn scaled(serial: Serial, max: u16, current: u16) -> Self {
        // Clamped by a max of at least one so a zero-max mobile does not divide
        // by zero. Written out rather than `max.max(1)`: `Ord` is not const.
        let divisor = if max == 0 { 1 } else { max as u32 };
        let percent = (current as u32 * 100 / divisor) as u16;
        Self {
            serial,
            max: 100,
            current: percent,
        }
    }
}

impl EncodePacket for HealthBar {
    const ID: u8 = 0xA1;
    const LENGTH: PacketLength = PacketLength::Fixed(9);

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.u32(self.serial.raw());
        out.u16(self.max);
        out.u16(self.current);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{decode_packet, encode_packet};

    fn version() -> ClientVersion {
        ClientVersion::new(7, 0, 45, 65)
    }

    fn mobile(raw: u32) -> Serial {
        Serial::new(raw).unwrap()
    }

    #[test]
    fn the_owner_sees_real_numbers() {
        let packet = encode_packet(&HealthBar::exact(mobile(1), 120, 45), version());
        assert_eq!(packet[0], 0xA1);
        assert_eq!(&packet[1..5], &0x0000_0001u32.to_be_bytes());
        assert_eq!(u16::from_be_bytes([packet[5], packet[6]]), 120);
        assert_eq!(u16::from_be_bytes([packet[7], packet[8]]), 45);
        assert_eq!(packet.len(), 9);
    }

    #[test]
    fn everyone_else_sees_a_percentage() {
        // 45 of 120 is 37%. Max goes out as 100 so the bar is cur/100.
        let packet = encode_packet(&HealthBar::scaled(mobile(1), 120, 45), version());
        assert_eq!(u16::from_be_bytes([packet[5], packet[6]]), 100);
        assert_eq!(u16::from_be_bytes([packet[7], packet[8]]), 37);
    }

    #[test]
    fn a_full_bar_reads_full_either_way() {
        assert_eq!(HealthBar::scaled(mobile(1), 200, 200).current, 100);
    }

    #[test]
    fn a_zero_max_does_not_divide_by_zero() {
        assert_eq!(HealthBar::scaled(mobile(1), 0, 0).current, 0);
    }

    #[test]
    fn war_mode_round_trips() {
        assert!(
            decode_packet::<WarMode>(&[0x72, 0x01, 0, 0x32, 0], version())
                .unwrap()
                .war
        );
        assert!(
            !decode_packet::<WarMode>(&[0x72, 0x00, 0, 0x32, 0], version())
                .unwrap()
                .war
        );
        assert_eq!(
            encode_packet(&WarMode { war: true }, version()),
            vec![0x72, 0x01, 0x00, 0x32, 0x00]
        );
        assert_eq!(
            encode_packet(&WarMode { war: false }, version()),
            vec![0x72, 0x00, 0x00, 0x32, 0x00]
        );
    }

    #[test]
    fn an_attack_request_is_a_serial() {
        let request: AttackRequest =
            decode_packet(&[0x05, 0x00, 0x00, 0x00, 0x2A], version()).unwrap();
        assert_eq!(request.target, Serial::new(0x2A));
    }

    #[test]
    fn an_attack_request_for_nothing_is_not_malformed() {
        // The client's way of saying "stop": a serial no object can have. It
        // clears the aim, and it must not cost the connection.
        let request: AttackRequest =
            decode_packet(&[0x05, 0x00, 0x00, 0x00, 0x00], version()).unwrap();
        assert_eq!(request.target, None);
    }

    #[test]
    fn setting_the_attack_target_is_five_bytes() {
        assert_eq!(
            encode_packet(
                &AttackTarget {
                    target: Serial::new(0x2A)
                },
                version()
            ),
            vec![0xAA, 0x00, 0x00, 0x00, 0x2A]
        );
    }

    #[test]
    fn clearing_the_attack_target_writes_a_zero() {
        assert_eq!(
            encode_packet(&AttackTarget { target: None }, version()),
            vec![0xAA, 0x00, 0x00, 0x00, 0x00]
        );
    }
}
