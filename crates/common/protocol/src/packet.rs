//! Packet identity and framing.
//!
//! # Why a length table exists at all
//!
//! The UO protocol has no self-describing frame. A packet is one id byte and
//! then a body whose length you are simply expected to know. Most packets are
//! fixed-length; the rest carry a big-endian `u16` length at offset 1 that
//! *includes* the id and the length field itself.
//!
//! So a server cannot even split a TCP stream into packets without knowing, for
//! every id, which kind it is and how long it is. That table is the first thing
//! any UO server needs and the last thing anyone wants to rediscover by hand.
//!
//! The numbers here are ported from SphereServer's `network/receive.h` and
//! `receive.cpp`, where each handler declares its own size. That is two decades
//! of observed client behaviour and it is exactly the part of Sphere worth
//! keeping.

use std::fmt;

use crate::codec::{PacketReader, PacketWriter};
use crate::error::{expect_id, DecodeError};
use crate::feature::Feature;
use crate::version::ClientVersion;

/// How long a packet is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PacketLength {
    /// Always this many bytes, including the id byte.
    Fixed(u16),
    /// A big-endian `u16` at offset 1 gives the total length, id and length
    /// field included.
    Variable,
}

impl PacketLength {
    /// The smallest a packet of this kind can be.
    ///
    /// A variable packet needs at least an id and a length field before the
    /// length can even be read.
    pub const fn minimum(self) -> usize {
        match self {
            Self::Fixed(size) => size as usize,
            Self::Variable => 3,
        }
    }
}

/// Length of the modern login seed handshake, including its `0xEF` byte.
///
/// # This is not a packet
///
/// The seed is the very first thing a client sends, before packet framing means
/// anything, and it does not play by the rules the table above describes:
///
/// - Old clients send four raw bytes with **no id byte at all** — a bare IPv4
///   address. There is nothing to look up.
/// - New clients send `0xEF` plus a seed and four version dwords.
/// - Sphere's `CNetworkInput.cpp` notes the `0xEF` byte "sometimes it's
///   received on its own", i.e. it can arrive in a TCP segment by itself, and
///   tracks a `m_newseed` flag across reads to cope.
///
/// So the handshake is a *connection state*, not a packet, and `0xEF` is
/// deliberately missing from [`client_packet_length`]. A gateway reads the seed
/// first and only then starts framing. Treating it as a normal packet is a
/// trap: a client that sends the lone `0xEF` byte would look like a truncated
/// 21-byte packet forever.
pub const SEED_LENGTH_NEW: usize = 21;

/// Length of the legacy seed: four raw bytes, no id.
pub const SEED_LENGTH_OLD: usize = 4;

/// The largest packet the server will accept.
///
/// Matches Sphere's `MAX_BUFFER`. A variable-length packet claiming more than
/// this is a client trying to make the server allocate, so it is rejected at
/// the framing layer rather than anywhere that could be tricked into honouring
/// it.
pub const MAX_PACKET_SIZE: usize = 18_000;

/// Framing could not proceed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum FrameError {
    /// The id is not one this server knows how to size, so the stream cannot be
    /// advanced past it. Fatal for the connection.
    UnknownPacket(u8),
    /// A variable-length packet declared a length below its own header, or above
    /// [`MAX_PACKET_SIZE`].
    BadLength {
        /// The packet id.
        id: u8,
        /// The length the client claimed.
        claimed: usize,
    },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPacket(id) => write!(f, "unknown packet 0x{id:02X}"),
            Self::BadLength { id, claimed } => {
                write!(f, "packet 0x{id:02X} claims an impossible length {claimed}")
            }
        }
    }
}

impl std::error::Error for FrameError {}

/// What a framing attempt found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Frame {
    /// A whole packet is present, this many bytes long including the id.
    Complete(usize),
    /// Not enough bytes yet. Try again once the buffer holds at least this many.
    Incomplete {
        /// Bytes needed before another attempt can make progress.
        needed: usize,
    },
}

/// How long the client-to-server packet with this id is, if we know.
///
/// `None` means unknown, which is fatal for a connection: without a length there
/// is no way to find where the next packet starts.
///
/// `version` is the connection's negotiated client version, once known. Almost
/// every length is the same for every client and ignores it; the exception is the
/// drop packet, whose body grew a byte across an era with no change of id, so the
/// framer cannot tell the two forms apart without it. `None` — the state before a
/// game login resolves the version — takes the older, shorter form; a client
/// cannot drag an item before it is in the world, so a real `0x08` never arrives
/// while the version is still unknown.
///
/// Ported from Sphere's `network/receive.h`. Server-to-client packets are not
/// here — the server knows the length of what it writes.
///
/// **`0xD1` is the one entry the two references disagree about.** Sphere reads
/// the logout notification as one byte (`PacketLogout : Packet(1)`); ServUO
/// registers it as two (`Register(0xD1, 2, …)`). Neither reads a payload, so
/// neither is self-correcting, and a wrong length here desynchronises the stream
/// rather than erroring. Two is taken: it is what ServUO and the client's own
/// packet table have carried for two decades, and the packet the *server* sends
/// back on the same id is two bytes in both references — an id whose two
/// directions are the same length is the norm, not the exception. The cost of
/// being wrong is bounded to the moment a player is leaving anyway.
// The column alignment is load-bearing: this is a lookup table that gets read
// against Sphere's, and rustfmt would reflow it into an unscannable list.
#[rustfmt::skip]
pub fn client_packet_length(id: u8, version: Option<ClientVersion>) -> Option<PacketLength> {
    use PacketLength::{Fixed, Variable};
    // The drop packet slipped a one-byte grid-location index in before the
    // container serial in 6.0.1.7 (`Feature::ItemGrid`): fifteen bytes for a
    // grid-capable client, fourteen for an older one. Same id, so only the
    // version tells them apart — and framing it wrong desynchronises the whole
    // client-to-server stream, one stray byte at a time.
    if id == 0x08 {
        let grid = version.is_some_and(|v| v.supports(Feature::ItemGrid));
        return Some(Fixed(if grid { 15 } else { 14 }));
    }
    Some(match id {
        0x00 => Fixed(104),  // create character
        0x02 => Fixed(7),    // movement request
        0x03 => Variable,    // talk
        0x05 => Fixed(5),    // attack request
        0x06 => Fixed(5),    // double click
        0x07 => Fixed(7),    // pick up item
        0x09 => Fixed(5),    // single click
        0x12 => Variable,    // text command
        0x13 => Fixed(10),   // equip item
        0x22 => Fixed(3),    // resynchronise
        0x2C => Fixed(2),    // death status
        0x34 => Fixed(10),   // status request
        0x3A => Variable,    // skill lock change
        0x3B => Variable,    // vendor buy
        0x3F => Variable,    // static update (UltimaLive)
        0x56 => Fixed(11),   // map edit
        0x5D => Fixed(73),   // character select
        0x66 => Variable,    // book page edit
        0x6C => Fixed(19),   // target
        0x6F => Variable,    // secure trade
        0x71 => Variable,    // bulletin board
        0x72 => Fixed(5),    // war mode
        0x73 => Fixed(2),    // ping
        0x75 => Fixed(35),   // rename character
        0x7D => Fixed(13),   // menu choice
        0x80 => Fixed(62),   // account login
        0x83 => Fixed(39),   // delete character
        0x8D => Fixed(146),  // create character (KR/SA)
        0x91 => Fixed(65),   // game server login
        0x93 => Fixed(99),   // book header edit
        0x95 => Fixed(9),    // dye object
        0x98 => Variable,    // all names (ctrl+shift)
        0x9A => Variable,    // prompt response (ascii)
        0x9B => Fixed(258),  // GM help page
        0x9F => Variable,    // vendor sell
        0xA0 => Fixed(3),    // select server
        0xA4 => Fixed(149),  // system info
        0xA7 => Fixed(4),    // tip request
        0xAC => Variable,    // gump text input
        0xAD => Variable,    // talk (unicode)
        0xB1 => Variable,    // gump button
        0xB3 => Variable,    // chat command
        0xB5 => Fixed(64),   // chat button
        0xB6 => Fixed(9),    // tooltip request
        0xB8 => Variable,    // profile request
        0xBB => Fixed(9),    // mail message
        0xBD => Variable,    // client version
        0xBE => Variable,    // assist version
        0xBF => Variable,    // extended command
        0xC2 => Variable,    // prompt response (unicode)
        0xC8 => Fixed(2),    // view range
        0xD1 => Fixed(2),    // logout notification — see the note below
        0xD4 => Variable,    // book header edit (new)
        0xD6 => Variable,    // AoS tooltip request
        0xD7 => Variable,    // encoded command
        0xD9 => Fixed(268),  // hardware info
        0xE0 => Variable,    // bug report
        0xE1 => Variable,    // client type (KR/SA)
        0xE8 => Fixed(13),   // remove UI highlight
        0xEB => Fixed(11),   // use hotbar
        0xEC => Variable,    // equip macro (KR)
        0xED => Variable,    // unequip macro (KR)
        // 0xEF is deliberately absent — see SEED_LENGTH_NEW.
        0xF0 => Variable,    // movement request (KR/SA)
        0xF1 => Fixed(9),    // time sync request
        0xF4 => Variable,    // crash report
        0xF8 => Fixed(106),  // create character (HS)
        0xF9 => Variable,    // global chat
        0xFA => Fixed(1),    // ultima store button
        0xFB => Fixed(2),    // public house content
        _ => return None,
    })
}

/// Find the first whole packet at the front of `buffer`.
///
/// Does not copy and does not consume: it reports how long the packet is, and
/// the caller decides what to do with it. That keeps framing testable in
/// isolation from any socket.
///
/// `version` is the connection's client version once known, `None` before a game
/// login resolves it. It only changes the length of the drop packet — see
/// [`client_packet_length`].
///
/// ```
/// use openshard_protocol::packet::{frame_client_packet, Frame};
///
/// // 0x73 ping is 2 bytes.
/// assert_eq!(frame_client_packet(&[0x73, 0x00], None), Ok(Frame::Complete(2)));
///
/// // Half a packet: wait for more.
/// assert_eq!(
///     frame_client_packet(&[0x73], None),
///     Ok(Frame::Incomplete { needed: 2 }),
/// );
///
/// // 0xAD talk is variable; the u16 at offset 1 is the total length.
/// let talk = [0xAD, 0x00, 0x05, 0xAA, 0xBB];
/// assert_eq!(frame_client_packet(&talk, None), Ok(Frame::Complete(5)));
/// ```
pub fn frame_client_packet(
    buffer: &[u8],
    version: Option<ClientVersion>,
) -> Result<Frame, FrameError> {
    let Some(&id) = buffer.first() else {
        return Ok(Frame::Incomplete { needed: 1 });
    };

    let length = client_packet_length(id, version).ok_or(FrameError::UnknownPacket(id))?;

    match length {
        PacketLength::Fixed(size) => {
            let size = size as usize;
            if buffer.len() < size {
                Ok(Frame::Incomplete { needed: size })
            } else {
                Ok(Frame::Complete(size))
            }
        }
        PacketLength::Variable => {
            if buffer.len() < 3 {
                return Ok(Frame::Incomplete { needed: 3 });
            }
            let claimed = u16::from_be_bytes([buffer[1], buffer[2]]) as usize;
            // Two distinct attacks, one check. Under 3 is nonsense — the
            // declared length covers the id and the length field themselves —
            // and would advance the caller by 0 or 2 bytes, re-framing the same
            // packet forever. Over the cap is a client trying to make the
            // server reserve 64KB per connection.
            #[allow(
                clippy::manual_range_contains,
                reason = "two failure modes, not one range"
            )]
            if claimed < 3 || claimed > MAX_PACKET_SIZE {
                return Err(FrameError::BadLength { id, claimed });
            }
            if buffer.len() < claimed {
                Ok(Frame::Incomplete { needed: claimed })
            } else {
                Ok(Frame::Complete(claimed))
            }
        }
    }
}

// -- payload traits and the framing layer ---------------------------------

/// A server-to-client payload: it writes its body and nothing else.
///
/// # The header is written once
///
/// An encoder that writes its own id byte and patches its own length field is
/// an encoder that can forget to, and forty-seven of them are forty-seven
/// chances. So a payload writes **body only**, [`encode_packet`] writes the
/// header, and "the length field is wrong" stops being a thing an author can do.
///
/// [`Self::LENGTH`] is not decoration: for a fixed packet it is the size the
/// body is checked against in debug builds, which catches a field added to a
/// struct and forgotten in the encoder — the other half of the same bug class.
///
/// `version` is passed even to payloads that ignore it. A packet that grows a
/// version-conditional tail later must not change the signature of every call
/// site with it.
pub trait EncodePacket {
    /// The packet id byte.
    const ID: u8;
    /// How long the framed packet is, id and length field included.
    const LENGTH: PacketLength;

    /// Write the body — everything after the header.
    fn encode_body(&self, out: &mut PacketWriter, version: ClientVersion);
}

/// A client-to-server payload: it reads its body from a reader already past the
/// id byte.
///
/// Fallible all the way down, because every byte came off a socket. See
/// [`crate::codec`].
pub trait DecodePacket: Sized {
    /// The packet id byte.
    const ID: u8;

    /// Read the body. The reader is positioned past the id, and for a
    /// variable-length packet past the length field too.
    fn decode_body(
        reader: &mut PacketReader<'_>,
        version: ClientVersion,
    ) -> Result<Self, DecodeError>;
}

/// Write `id`, the length field if the packet has one, then the body.
///
/// The single place a packet header is produced. A variable-length packet gets
/// a placeholder length that is back-patched once the body's size is known, so
/// no payload ever counts its own bytes.
///
/// Panics if a variable packet's body pushes the total past `u16::MAX`: the
/// length field could not describe it, and truncating instead would desynchronise
/// the client's stream in silence. That is a server bug in packet construction,
/// and it costs one connection.
pub fn frame_body(
    id: u8,
    length: PacketLength,
    write_body: impl FnOnce(&mut PacketWriter),
) -> Vec<u8> {
    let mut writer = PacketWriter::with_capacity(length.minimum());
    writer.u8(id);
    match length {
        PacketLength::Fixed(size) => {
            write_body(&mut writer);
            debug_assert_eq!(
                writer.len(),
                size as usize,
                "packet 0x{id:02X} declares {size} bytes and wrote {}",
                writer.len()
            );
        }
        PacketLength::Variable => {
            writer.u16(0); // placeholder, patched below
            write_body(&mut writer);
            let total = writer.len();
            assert!(
                total <= u16::MAX as usize,
                "packet 0x{id:02X} is {total} bytes: too long for its own length field"
            );
            writer.patch_u16(1, total as u16);
        }
    }
    writer.into_bytes()
}

/// Frame one payload for a client.
///
/// The only way a `ServerPacket` variant reaches the wire; see [`EncodePacket`].
pub fn encode_packet<P: EncodePacket>(packet: &P, version: ClientVersion) -> Vec<u8> {
    frame_body(P::ID, P::LENGTH, |out| packet.encode_body(out, version))
}

/// Check the id byte, skip the length field if this id is variable-length, and
/// decode the body behind it.
///
/// A mismatched id is a dispatch bug — the packet was routed to the wrong
/// decoder — and is reported as one rather than being read as if it fitted.
///
/// The length field itself is never handed to [`DecodePacket::decode_body`]:
/// `bytes` has already passed through [`frame_client_packet`], which is what
/// checks a variable packet's claimed length against the buffer and against
/// [`MAX_PACKET_SIZE`]. By the time a body decoder runs, `bytes` already *is*
/// exactly one packet, so there is nothing left for the length field to tell
/// the body — re-checking it here would be the same validation twice, in two
/// places that could disagree.
pub fn decode_packet<P: DecodePacket>(
    bytes: &[u8],
    version: ClientVersion,
) -> Result<P, DecodeError> {
    let mut reader = expect_id(bytes, P::ID)?;
    if client_packet_length(P::ID, Some(version)) == Some(PacketLength::Variable) {
        reader.skip(2)?;
    }
    P::decode_body(&mut reader, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed-length payload with nothing but a byte in it.
    struct Ping(u8);

    impl EncodePacket for Ping {
        const ID: u8 = 0x73;
        const LENGTH: PacketLength = PacketLength::Fixed(2);

        fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
            out.u8(self.0);
        }
    }

    /// A variable-length payload, so the length patch has something to patch.
    /// `0xAD` because the round-trip test below frames it back through the
    /// client table, which is the only length table there is.
    struct Talk(&'static str);

    impl EncodePacket for Talk {
        const ID: u8 = 0xAD;
        const LENGTH: PacketLength = PacketLength::Variable;

        fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
            out.null_terminated_string(self.0);
        }
    }

    fn any_version() -> ClientVersion {
        ClientVersion::new(7, 0, 45, 65)
    }

    #[test]
    fn framing_writes_the_id_and_the_body() {
        assert_eq!(encode_packet(&Ping(0x2A), any_version()), vec![0x73, 0x2A]);
    }

    #[test]
    fn framing_patches_a_variable_length() {
        // id + length field + "hi\0" = 6 bytes, and the length field says so.
        let bytes = encode_packet(&Talk("hi"), any_version());
        assert_eq!(bytes, vec![0xAD, 0x00, 0x06, b'h', b'i', 0x00]);
        assert_eq!(
            u16::from_be_bytes([bytes[1], bytes[2]]) as usize,
            bytes.len()
        );
    }

    #[test]
    fn a_framed_packet_frames_back() {
        // The property that makes the length table one source of truth for both
        // directions: what the framer writes, the framer can read.
        let bytes = encode_packet(&Talk("well met"), any_version());
        assert_eq!(
            frame_client_packet(&bytes, None),
            Ok(Frame::Complete(bytes.len())),
            "0xAD is variable in the table and self-describing on the wire"
        );
    }

    #[test]
    fn decoding_rejects_a_foreign_id() {
        struct Attack(u32);
        impl DecodePacket for Attack {
            const ID: u8 = 0x05;
            fn decode_body(
                reader: &mut PacketReader<'_>,
                _version: ClientVersion,
            ) -> Result<Self, DecodeError> {
                Ok(Self(reader.u32()?))
            }
        }

        let packet: Attack = decode_packet(&[0x05, 0, 0, 0, 0x2A], any_version()).unwrap();
        assert_eq!(packet.0, 0x2A);
        assert!(decode_packet::<Attack>(&[0x06, 0, 0, 0, 0x2A], any_version()).is_err());
    }

    #[test]
    fn known_packets_have_plausible_lengths() {
        for id in 0..=u8::MAX {
            let Some(length) = client_packet_length(id, None) else {
                continue;
            };
            match length {
                PacketLength::Fixed(size) => {
                    assert!(size >= 1, "0x{id:02X} is fixed at {size} bytes");
                    assert!(
                        size as usize <= MAX_PACKET_SIZE,
                        "0x{id:02X} exceeds the buffer cap"
                    );
                }
                PacketLength::Variable => assert_eq!(length.minimum(), 3),
            }
        }
    }

    #[test]
    fn spot_checks_against_spheres_table() {
        // A handful pinned by hand from Sphere's receive.h/receive.cpp. If the
        // table is ever regenerated, these are what catch a shift.
        assert_eq!(
            client_packet_length(0x00, None),
            Some(PacketLength::Fixed(104))
        );
        assert_eq!(
            client_packet_length(0x02, None),
            Some(PacketLength::Fixed(7))
        );
        assert_eq!(
            client_packet_length(0x03, None),
            Some(PacketLength::Variable)
        );
        assert_eq!(
            client_packet_length(0x5D, None),
            Some(PacketLength::Fixed(73))
        );
        assert_eq!(
            client_packet_length(0x80, None),
            Some(PacketLength::Fixed(62))
        );
        assert_eq!(
            client_packet_length(0x91, None),
            Some(PacketLength::Fixed(65))
        );
        assert_eq!(
            client_packet_length(0xBD, None),
            Some(PacketLength::Variable)
        );
        assert_eq!(
            client_packet_length(0xBF, None),
            Some(PacketLength::Variable)
        );
        assert_eq!(
            client_packet_length(0xD9, None),
            Some(PacketLength::Fixed(268))
        );
        assert_eq!(
            client_packet_length(0xF8, None),
            Some(PacketLength::Fixed(106))
        );
    }

    #[test]
    fn the_drop_packet_length_follows_the_client_version() {
        // The bug this guards: a modern client sends a fifteen-byte 0x08 with a
        // grid-index byte, and framing it as fourteen leaves a stray byte that
        // desynchronises the whole stream.
        let modern = ClientVersion::new(7, 0, 45, 65);
        let ancient = ClientVersion::new(5, 0, 0, 0); // before ItemGrid (6.0.1.7)
        assert_eq!(
            client_packet_length(0x08, Some(modern)),
            Some(PacketLength::Fixed(15)),
            "a grid-capable client sends fifteen"
        );
        assert_eq!(
            client_packet_length(0x08, Some(ancient)),
            Some(PacketLength::Fixed(14)),
            "a pre-6.0.1.7 client sends fourteen"
        );
        assert_eq!(
            client_packet_length(0x08, None),
            Some(PacketLength::Fixed(14)),
            "before a version is known, the older form — a real 0x08 never arrives that early"
        );
    }

    #[test]
    fn unknown_ids_are_unknown() {
        // 0x01 and 0x04 have no client-to-server meaning.
        assert_eq!(client_packet_length(0x01, None), None);
        assert_eq!(client_packet_length(0x04, None), None);
        assert_eq!(client_packet_length(0xFF, None), None);
    }

    #[test]
    fn the_seed_is_not_a_framable_packet() {
        // 0xEF arrives before framing starts and can turn up as a lone byte in
        // its own TCP segment. In the table it would look like a permanently
        // truncated 21-byte packet, and the gateway would wait forever.
        assert_eq!(client_packet_length(0xEF, None), None);
        assert_eq!(
            frame_client_packet(&[0xEF], None),
            Err(FrameError::UnknownPacket(0xEF)),
            "the gateway must read the seed before it starts framing"
        );
    }

    #[test]
    fn frames_a_fixed_packet() {
        assert_eq!(
            frame_client_packet(&[0x73, 0x00], None),
            Ok(Frame::Complete(2))
        );
    }

    #[test]
    fn a_fixed_packet_with_trailing_bytes_reports_only_its_own_length() {
        // TCP delivers whatever it likes; two packets often arrive together.
        let buffer = [0x73, 0x00, 0x73, 0x00];
        assert_eq!(frame_client_packet(&buffer, None), Ok(Frame::Complete(2)));
        assert_eq!(
            frame_client_packet(&buffer[2..], None),
            Ok(Frame::Complete(2))
        );
    }

    #[test]
    fn an_empty_buffer_is_incomplete_not_an_error() {
        assert_eq!(
            frame_client_packet(&[], None),
            Ok(Frame::Incomplete { needed: 1 })
        );
    }

    #[test]
    fn a_partial_fixed_packet_asks_for_its_full_length() {
        assert_eq!(
            frame_client_packet(&[0x00, 0x01, 0x02], None),
            Ok(Frame::Incomplete { needed: 104 })
        );
    }

    #[test]
    fn a_variable_packet_without_its_length_field_asks_for_three() {
        assert_eq!(
            frame_client_packet(&[0xAD, 0x00], None),
            Ok(Frame::Incomplete { needed: 3 })
        );
    }

    #[test]
    fn frames_a_variable_packet() {
        let talk = [0xAD, 0x00, 0x05, 0xAA, 0xBB];
        assert_eq!(frame_client_packet(&talk, None), Ok(Frame::Complete(5)));
        assert_eq!(
            frame_client_packet(&talk[..4], None),
            Ok(Frame::Incomplete { needed: 5 })
        );
    }

    #[test]
    fn an_unknown_id_is_fatal() {
        // There is no way to skip a packet of unknown length: the stream is
        // desynchronised from here on, so the connection has to go.
        assert_eq!(
            frame_client_packet(&[0x01, 0x00, 0x00], None),
            Err(FrameError::UnknownPacket(0x01))
        );
    }

    #[test]
    fn a_length_below_the_header_is_rejected() {
        // Honouring this would advance the caller by 0 or 2 bytes and re-frame
        // the same packet forever.
        for claimed in 0u16..3 {
            let [high, low] = claimed.to_be_bytes();
            assert_eq!(
                frame_client_packet(&[0xAD, high, low, 0x00], None),
                Err(FrameError::BadLength {
                    id: 0xAD,
                    claimed: claimed as usize
                }),
                "0xAD claiming {claimed} must be rejected"
            );
        }
    }

    #[test]
    fn an_oversized_length_is_rejected_before_anything_allocates() {
        let [high, low] = u16::MAX.to_be_bytes();
        assert_eq!(
            frame_client_packet(&[0xBF, high, low], None),
            Err(FrameError::BadLength {
                id: 0xBF,
                claimed: u16::MAX as usize
            })
        );
    }

    #[test]
    fn the_largest_legal_length_is_accepted() {
        // MAX_PACKET_SIZE itself must be inside the bound, not outside it.
        let claimed = MAX_PACKET_SIZE as u16;
        let [high, low] = claimed.to_be_bytes();
        let mut buffer = vec![0xBF, high, low];
        buffer.resize(MAX_PACKET_SIZE, 0);
        assert_eq!(
            frame_client_packet(&buffer, None),
            Ok(Frame::Complete(MAX_PACKET_SIZE))
        );
    }

    #[test]
    fn framing_always_advances() {
        // The property the read loop depends on: a Complete frame is never zero
        // bytes, or the caller spins.
        for id in 0..=u8::MAX {
            if client_packet_length(id, None).is_none() {
                continue;
            }
            let mut buffer = vec![id, 0x46, 0x50];
            buffer.resize(MAX_PACKET_SIZE, 0);
            match frame_client_packet(&buffer, None) {
                Ok(Frame::Complete(size)) => {
                    assert!(size > 0, "0x{id:02X} framed a zero-length packet")
                }
                Ok(Frame::Incomplete { needed }) => {
                    assert!(needed > buffer.len(), "0x{id:02X} asked for no progress")
                }
                Err(error) => panic!("0x{id:02X} should frame, got {error}"),
            }
        }
    }
}
