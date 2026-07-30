//! Container packets: opening a container and listing what is inside it.
//!
//! A container is an item that holds other items. Three server packets draw it:
//! `0x24` opens the gump window, `0x3C` fills it with everything inside at once,
//! and `0x25` adds one more item to a gump already open. The client asks to open
//! one by double-clicking it — `0x06`.
//!
//! # Two client-version seams
//!
//! - The `0x24` open packet gained a one-word *container type* on High Seas
//!   clients ([`Feature::HsPackets`]). Older clients stop after the gump id.
//! - Every item record inside a container gained a one-byte *grid index* on
//!   6.0.1.7 ([`Feature::ItemGrid`]) — the slot in the enhanced grid view. The
//!   classic 2D client positions items by their `x`/`y` and ignores it; a grid
//!   client reads it and desynchronises if it is missing.

use crate::codec::{PacketReader, PacketWriter};
use crate::error::DecodeError;
use crate::feature::Feature;
use crate::gump::GumpPoint;
use crate::packet::{DecodePacket, EncodePacket, PacketLength};
use crate::serial::{RawSerial, Serial};
use crate::version::ClientVersion;
use crate::wire::{Graphic, Hue};

/// The container-type byte a High Seas client expects in `0x24` for a normal
/// container (a vendor's is `0x00`, which is not this).
const CONTAINER_TYPE: u16 = 0x7D;

/// The gump id that makes a `0x24` draw a *book* rather than a container.
///
/// A spellbook, a runebook and a book of gate travel are all containers on the
/// wire, and this is the one value that tells the client to open the page view
/// instead of a bag — see [`crate::spellbook`].
pub const BOOK_GUMP: Graphic = Graphic(0xFFFF);

/// Bit 31 of a `0x06`'s serial: the client is asking for a *paperdoll*, not a
/// use.
///
/// Nothing addressable ever has this bit — the item pool stops at
/// `0x7FFF_FFFF` — so it is free for the client to flag with, and it flags the
/// paperdoll macro and the paperdoll the client opens for itself at login.
const PAPERDOLL_REQUEST: u32 = 0x8000_0000;

/// `0x06` — the client double-clicked an object. 5 bytes.
///
/// Double-click is "use this": a container opens, a door swings, a food is
/// eaten. The server decides what the object does; this only says which.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DoubleClick {
    /// The object's serial, with the paperdoll bit still on it —
    /// [`interpret`](Self::interpret) is what takes the two apart.
    pub serial: RawSerial,
}

impl DecodePacket for DoubleClick {
    const ID: u8 = 0x06;

    fn decode_body(reader: &mut PacketReader<'_>, _version: ClientVersion) -> Result<Self, DecodeError> {
        Ok(Self {
            serial: RawSerial(reader.u32()?),
        })
    }
}

/// What a `0x06` is actually asking for.
///
/// The two are *not* the same request and answering one with the other is a
/// bug this engine has already had: ServUO's `UseReq` routes a paperdoll
/// request straight to `OnPaperdollRequest` and never to `Use`, so treating
/// the login-time paperdoll open as a self-double-click dismounted a rider a
/// breath after they logged in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UseRequest {
    /// Open this mobile's paperdoll, and do nothing else.
    Paperdoll(RawSerial),
    /// Use the object: open the container, swing the door, eat the food.
    Use(RawSerial),
}

impl DoubleClick {
    /// Split the paperdoll bit off the serial.
    ///
    /// Total, and deliberately so: both arms carry a [`RawSerial`], because
    /// stripping a flag bit does not make what is left address anything. The
    /// check that it does is [`RawSerial::validate`], at whichever seam acts on
    /// the request — see `docs/protocol_newtypes.md` N2.
    #[must_use]
    pub const fn interpret(self) -> UseRequest {
        if self.serial.0 & PAPERDOLL_REQUEST == 0 {
            UseRequest::Use(self.serial)
        } else {
            UseRequest::Paperdoll(RawSerial(self.serial.0 & !PAPERDOLL_REQUEST))
        }
    }
}

/// Which cell of the enhanced client's grid view an item sits in.
///
/// The server picks it — [`ContainedItem`] goes out only — and the classic 2D
/// client ignores it entirely, positioning by `x`/`y` instead. A named byte
/// rather than an index type with a range: the grid's size is the client's, and
/// this engine has never had a reason to learn it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct GridSlot(pub u8);

/// One item as it sits inside a container: what `0x25` and `0x3C` write per item.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ContainedItem {
    /// The item's serial.
    pub serial: Serial,
    /// Its graphic.
    pub graphic: Graphic,
    /// Its stack size.
    pub amount: u16,
    /// Where its icon sits inside the container's gump art.
    ///
    /// The pair N4 left on the allowlist for N5 to name: it is a gump
    /// coordinate, not a world one, and [`GumpPoint`] is the type — measured
    /// from the art's top left here, and from the screen's for a window. Two
    /// bytes go out where a window's four do; the value is the server's, and a
    /// container's art is a few hundred pixels wide.
    pub at: GumpPoint,
    /// Its slot in the enhanced grid view. Sent only to grid clients.
    pub grid: GridSlot,
    /// Its hue.
    pub hue: Hue,
}

impl ContainedItem {
    /// Write one item record: the shared body of `0x25` and `0x3C`.
    fn write(&self, writer: &mut PacketWriter, container: Serial, grid: bool) {
        writer.u32(self.serial.raw());
        writer.u16(self.graphic.0);
        writer.u8(0); // graphic offset, always zero
        writer.u16(self.amount);
        writer.u16(self.at.x as u16);
        writer.u16(self.at.y as u16);
        if grid {
            writer.u8(self.grid.0);
        }
        writer.u32(container.raw());
        writer.u16(self.hue.0);
    }
}

/// `0x24` — open a container gump on the client. 7 bytes, 9 on High Seas.
///
/// # Not an `EncodePacket`
///
/// This is `0xB9`'s problem from Stage 2 (`docs/protocol_rewrite.md`) again: the
/// packet is fixed-length, but *which* fixed length depends on
/// [`Feature::HsPackets`], and [`EncodePacket::LENGTH`] is a `const` that cannot
/// ask a payload's own `version`. Neither `Fixed` nor `Variable` describes it, so
/// it stays a hand-written free function rather than forced into a model it does
/// not fit.
pub fn encode_open_container(serial: Serial, gump: Graphic, version: ClientVersion) -> Vec<u8> {
    let mut writer = PacketWriter::with_capacity(open_container_length(version).minimum());
    writer.u8(0x24);
    writer.u32(serial.raw());
    writer.u16(gump.0);
    if version.supports(Feature::HsPackets) {
        writer.u16(CONTAINER_TYPE);
    }
    debug_assert_eq!(writer.len(), open_container_length(version).minimum());
    writer.into_bytes()
}

/// How [`encode_open_container`] is framed, for the client version it was
/// written for.
///
/// The rule — High Seas adds a two-byte container type — lives here, next to the
/// encoder that obeys it, so a framer can ask rather than carry its own copy of
/// the same `if`.
#[must_use]
pub fn open_container_length(version: ClientVersion) -> PacketLength {
    PacketLength::Fixed(if version.supports(Feature::HsPackets) {
        9
    } else {
        7
    })
}

/// `0x25` — add one item to a container gump the client already has open.
///
/// The same version-dependent-fixed-size shape as [`encode_open_container`], this
/// time gated on [`Feature::ItemGrid`], and not an `EncodePacket` for the same
/// reason.
pub fn encode_add_to_container(item: ContainedItem, container: Serial, version: ClientVersion) -> Vec<u8> {
    let grid = version.supports(Feature::ItemGrid);
    let mut writer = PacketWriter::with_capacity(add_to_container_length(version).minimum());
    writer.u8(0x25);
    item.write(&mut writer, container, grid);
    debug_assert_eq!(writer.len(), add_to_container_length(version).minimum());
    writer.into_bytes()
}

/// How [`encode_add_to_container`] is framed. The grid byte is the whole
/// difference; see [`open_container_length`] for why this lives here.
#[must_use]
pub fn add_to_container_length(version: ClientVersion) -> PacketLength {
    PacketLength::Fixed(if version.supports(Feature::ItemGrid) {
        21
    } else {
        20
    })
}

/// `0x3C` — the full contents of a container, all at once. Variable length.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ContainerContents {
    /// The container being filled.
    pub container: Serial,
    /// Everything inside it.
    pub items: Vec<ContainedItem>,
}

impl EncodePacket for ContainerContents {
    const ID: u8 = 0x3C;
    const LENGTH: PacketLength = PacketLength::Variable;

    fn encode_body(&self, out: &mut PacketWriter, version: ClientVersion) {
        let grid = version.supports(Feature::ItemGrid);
        out.u16(self.items.len() as u16);
        for item in &self.items {
            item.write(out, self.container, grid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{decode_packet, encode_packet};

    /// A version with the grid index and the High Seas container type.
    fn modern() -> ClientVersion {
        ClientVersion::new(7, 0, 9, 0)
    }

    /// A version with neither.
    fn classic() -> ClientVersion {
        ClientVersion::new(5, 0, 0, 0)
    }

    /// The one container every test in here fills.
    fn container() -> Serial {
        Serial::new(0x4000_0001).unwrap()
    }

    #[test]
    fn a_double_click_is_a_serial() {
        let bytes = [0x06, 0x40, 0x00, 0x00, 0x2A];
        let click: DoubleClick = decode_packet(&bytes, classic()).unwrap();
        assert_eq!(click.serial, RawSerial(0x4000_002A));
        assert_eq!(click.interpret(), UseRequest::Use(RawSerial(0x4000_002A)));
    }

    #[test]
    fn the_top_bit_of_a_double_click_asks_for_a_paperdoll() {
        // The same object, asked for the other way: bit 31 set, and what is
        // left is the mobile whose paperdoll is wanted.
        let bytes = [0x06, 0x80, 0x00, 0x00, 0x2A];
        let click: DoubleClick = decode_packet(&bytes, classic()).unwrap();
        assert_eq!(
            click.serial,
            RawSerial(0x8000_002A),
            "the bit survives decoding — the packet is not normalised on the way in"
        );
        assert_eq!(click.interpret(), UseRequest::Paperdoll(RawSerial(0x2A)));
    }

    #[test]
    fn every_double_click_interprets() {
        // Class B is total: the bit is either set or it is not, and both arms
        // hand back a serial nobody has checked yet.
        for high in [0u32, 1] {
            for low in [0u32, 1, 0x4000_002A, 0x7FFF_FFFF] {
                let raw = (high << 31) | low;
                let click = DoubleClick {
                    serial: RawSerial(raw),
                };
                let expected = RawSerial(low);
                match click.interpret() {
                    UseRequest::Use(serial) => {
                        assert_eq!(high, 0);
                        assert_eq!(serial, expected);
                    }
                    UseRequest::Paperdoll(serial) => {
                        assert_eq!(high, 1);
                        assert_eq!(serial, expected);
                    }
                }
            }
        }
    }

    #[test]
    fn a_double_click_on_nothing_decodes_and_is_refused_at_promotion() {
        // `docs/protocol_newtypes.md` N9: the hostile value gets all the way
        // through the framer — dropping the connection over it would be wrong —
        // and dies where it would have addressed something.
        let bytes = [0x06, 0x00, 0x00, 0x00, 0x00];
        let click: DoubleClick = decode_packet(&bytes, classic()).unwrap();
        let UseRequest::Use(serial) = click.interpret() else {
            panic!("bit 31 is clear");
        };
        assert_eq!(serial.validate(), None, "zero addresses nothing");
    }

    #[test]
    fn opening_a_container_is_seven_bytes_on_a_classic_client() {
        let packet = encode_open_container(container(), Graphic(0x003C), classic());
        assert_eq!(packet[0], 0x24);
        assert_eq!(&packet[1..5], &0x4000_0001u32.to_be_bytes());
        assert_eq!(&packet[5..7], &0x003Cu16.to_be_bytes());
        assert_eq!(packet.len(), 7, "no container-type word before High Seas");
    }

    #[test]
    fn opening_a_container_gains_the_type_word_on_high_seas() {
        let packet = encode_open_container(container(), Graphic(0x003C), modern());
        assert_eq!(packet.len(), 9);
        assert_eq!(u16::from_be_bytes([packet[7], packet[8]]), CONTAINER_TYPE);
    }

    #[test]
    fn a_classic_container_item_record_has_no_grid_byte() {
        let item = ContainedItem {
            serial: Serial::new(0x4000_0002).unwrap(),
            graphic: Graphic(0x0EED),
            amount: 3,
            at: GumpPoint::new(44, 65),
            grid: GridSlot(7),
            hue: Hue::NONE,
        };
        let packet = encode_add_to_container(item, container(), classic());
        // 0x25 + serial + graphic + 0 + amount + x + y + container + hue = 20
        assert_eq!(packet.len(), 20);
        assert_eq!(packet[0], 0x25);
        assert_eq!(&packet[1..5], &0x4000_0002u32.to_be_bytes());
        assert_eq!(&packet[5..7], &0x0EEDu16.to_be_bytes());
        assert_eq!(packet[7], 0); // graphic offset
        assert_eq!(&packet[8..10], &3u16.to_be_bytes());
        assert_eq!(&packet[10..12], &44u16.to_be_bytes());
        assert_eq!(&packet[12..14], &65u16.to_be_bytes());
        // straight to the container serial, no grid byte
        assert_eq!(&packet[14..18], &0x4000_0001u32.to_be_bytes());
    }

    #[test]
    fn a_grid_client_item_record_carries_the_grid_byte() {
        let item = ContainedItem {
            serial: Serial::new(0x4000_0002).unwrap(),
            graphic: Graphic(0x0EED),
            amount: 3,
            at: GumpPoint::new(44, 65),
            grid: GridSlot(7),
            hue: Hue::NONE,
        };
        let packet = encode_add_to_container(item, container(), modern());
        assert_eq!(packet.len(), 21);
        assert_eq!(packet[14], 7, "the grid index sits before the container serial");
        assert_eq!(&packet[15..19], &0x4000_0001u32.to_be_bytes());
    }

    #[test]
    fn container_contents_counts_its_items_and_patches_its_length() {
        let items = [
            ContainedItem {
                serial: Serial::new(0x4000_0002).unwrap(),
                graphic: Graphic(0x0EED),
                amount: 1,
                at: GumpPoint::new(10, 10),
                grid: GridSlot(0),
                hue: Hue::NONE,
            },
            ContainedItem {
                serial: Serial::new(0x4000_0003).unwrap(),
                graphic: Graphic(0x0F0E),
                amount: 5,
                at: GumpPoint::new(20, 20),
                grid: GridSlot(1),
                hue: Hue(0x21),
            },
        ];
        let packet = encode_packet(
            &ContainerContents {
                container: container(),
                items: items.to_vec(),
            },
            classic(),
        );
        assert_eq!(packet[0], 0x3C);
        assert_eq!(u16::from_be_bytes([packet[1], packet[2]]), packet.len() as u16);
        assert_eq!(u16::from_be_bytes([packet[3], packet[4]]), 2, "two items");
        // header 5 + two classic records of 19 each = 43
        assert_eq!(packet.len(), 5 + 2 * 19);
    }

    #[test]
    fn an_empty_container_is_just_a_header() {
        let packet = encode_packet(
            &ContainerContents {
                container: container(),
                items: Vec::new(),
            },
            classic(),
        );
        assert_eq!(u16::from_be_bytes([packet[3], packet[4]]), 0);
        assert_eq!(packet.len(), 5);
    }
}
