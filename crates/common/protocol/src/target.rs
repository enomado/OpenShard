//! The targeting cursor: `0x6C`, sent to raise a crosshair and read back where it
//! was clicked.
//!
//! The server sends a `0x6C` to ask the client to target something; the client
//! answers with a `0x6C` of the same shape carrying what was clicked — a mobile,
//! an item, or a spot on the ground. It is one packet id in both directions,
//! nineteen bytes each way.

use crate::codec::{PacketReader, PacketWriter};
use crate::error::DecodeError;
use crate::packet::{DecodePacket, EncodePacket, PacketLength};
use crate::serial::Serial;
use crate::version::ClientVersion;
use crate::wire::{CursorId, Graphic};
use crate::world::Point;

/// The `cursorType` byte a cancelled (right-clicked) target comes back as.
const CURSOR_CANCEL: u8 = 3;

/// The `x` a cancelled target may come back as instead of, or as well as, a
/// cancelling cursor type.
const CANCEL_X: u16 = 0xFFFF;

/// What a raised cursor is allowed to pick.
///
/// The client enforces this itself, which is the point: "whom shall I examine?"
/// has no answer in a patch of grass, and refusing the click in the client saves
/// the player a wasted one. The server still checks what comes back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum TargetKind {
    /// An object — a mobile or an item. A click on bare ground is refused.
    Object = 0,
    /// Either an object or a spot on the ground; a tile is reported when that is
    /// what was clicked.
    Location = 1,
}

/// `0x6C` — raise a targeting cursor. 19 bytes.
///
/// `cursor_id` is echoed back in the response so the server can match a click to
/// the request that asked for it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TargetCursor {
    /// Echoed back by the client, opaque to it.
    pub cursor_id: CursorId,
    /// What the cursor may pick.
    pub kind: TargetKind,
}

impl EncodePacket for TargetCursor {
    const ID: u8 = 0x6C;
    const LENGTH: PacketLength = PacketLength::Fixed(19);

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.u8(self.kind as u8);
        out.u32(self.cursor_id.0);
        out.u8(0); // cursor type: neutral
                   // The rest the client fills in on the way back: object serial(4),
                   // x(2), y(2), a pad byte, z(1), tile graphic(2) — twelve bytes.
        out.zeros(12);
    }
}

/// `0x6C` — the client's answer: what the cursor picked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TargetResponse {
    /// The id the request carried, echoed back.
    pub cursor_id: CursorId,
    /// The object clicked, or `None` for a bare spot on the ground.
    ///
    /// Absence is the real thing here, not a zero to be checked for later: a
    /// ground target *has* no object, and a location spell wants exactly that.
    pub object: Option<Serial>,
    /// Where — the clicked tile, meaningful for a ground target.
    pub location: Point,
    /// The tile graphic clicked, `None` for none.
    pub graphic: Option<Graphic>,
    /// Whether the target was cancelled — right-clicked away rather than picked.
    pub cancelled: bool,
}

impl DecodePacket for TargetResponse {
    const ID: u8 = 0x6C;

    /// Layout: type, cursor id, cursor type, clicked serial, x, y, a pad byte, z,
    /// tile graphic. A cursor type of 3 — or an `x` of `0xFFFF` — is a cancel.
    fn decode_body(
        reader: &mut PacketReader<'_>,
        _version: ClientVersion,
    ) -> Result<Self, DecodeError> {
        let _kind = reader.u8()?;
        let cursor_id = CursorId(reader.u32()?);
        let cursor_type = reader.u8()?;
        let object = Serial::new(reader.u32()?);
        let x = reader.u16()?;
        let y = reader.u16()?;
        let _pad = reader.u8()?;
        let z = reader.u8()? as i8;
        let graphic = match reader.u16()? {
            0 => None,
            id => Some(Graphic(id)),
        };

        let cancelled = cursor_type == CURSOR_CANCEL || x == CANCEL_X;
        Ok(Self {
            cursor_id,
            object,
            location: Point::new(x, y, z),
            graphic,
            cancelled,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{decode_packet, encode_packet};

    fn version() -> ClientVersion {
        ClientVersion::new(7, 0, 45, 65)
    }

    #[test]
    fn a_cursor_request_is_nineteen_bytes() {
        let bytes = encode_packet(
            &TargetCursor {
                cursor_id: CursorId(0x0000_002A),
                kind: TargetKind::Location,
            },
            version(),
        );
        assert_eq!(bytes.len(), 19);
        assert_eq!(bytes[0], 0x6C);
        assert_eq!(bytes[1], TargetKind::Location as u8);
        assert_eq!(
            u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
            0x0000_002A
        );
    }

    #[test]
    fn an_object_cursor_differs_by_one_byte() {
        let object = encode_packet(
            &TargetCursor {
                cursor_id: CursorId(1),
                kind: TargetKind::Object,
            },
            version(),
        );
        let location = encode_packet(
            &TargetCursor {
                cursor_id: CursorId(1),
                kind: TargetKind::Location,
            },
            version(),
        );
        assert_eq!(object[1], 0x00);
        assert_eq!(location[1], 0x01);
        assert_eq!(object[2..], location[2..], "nothing else differs");
    }

    #[test]
    fn a_ground_click_decodes_to_a_location() {
        let mut p = vec![0x6C, TargetKind::Location as u8];
        p.extend_from_slice(&0x2Au32.to_be_bytes()); // cursor id
        p.push(0); // cursor type: neutral
        p.extend_from_slice(&0u32.to_be_bytes()); // serial: ground, none
        p.extend_from_slice(&1436u16.to_be_bytes()); // x
        p.extend_from_slice(&1559u16.to_be_bytes()); // y
        p.push(0); // pad
        p.push(30i8 as u8); // z
        p.extend_from_slice(&0x07C1u16.to_be_bytes()); // tile graphic
        assert_eq!(p.len(), 19);

        let got: TargetResponse = decode_packet(&p, version()).unwrap();
        assert_eq!(got.cursor_id, CursorId(0x2A));
        assert_eq!(got.object, None, "a ground click hit no object");
        assert_eq!(got.location, Point::new(1436, 1559, 30));
        assert_eq!(got.graphic, Some(Graphic(0x07C1)));
        assert!(!got.cancelled);
    }

    #[test]
    fn an_object_click_carries_the_serial() {
        let mut p = vec![0x6C, TargetKind::Object as u8];
        p.extend_from_slice(&0x2Au32.to_be_bytes());
        p.push(0);
        p.extend_from_slice(&0x0000_1234u32.to_be_bytes());
        p.extend_from_slice(&[0u8; 8]);
        assert_eq!(p.len(), 19);

        let got: TargetResponse = decode_packet(&p, version()).unwrap();
        assert_eq!(got.object, Serial::new(0x1234));
        assert_eq!(got.graphic, None, "no tile graphic on an object click");
    }

    #[test]
    fn a_right_click_is_a_cancel() {
        let mut p = vec![0x6C, TargetKind::Location as u8];
        p.extend_from_slice(&0x2Au32.to_be_bytes());
        p.push(CURSOR_CANCEL);
        p.extend_from_slice(&[0u8; 12]);
        assert_eq!(p.len(), 19);
        let got: TargetResponse = decode_packet(&p, version()).unwrap();
        assert!(got.cancelled);
    }
}
