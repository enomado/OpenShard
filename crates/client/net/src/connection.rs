//! Bytes in, packets out. No socket.
//!
//! The mirror of `openshard_gateway::connection`, and it faces the same problem
//! from the other side: TCP delivers whatever it feels like, and a packet may
//! arrive in three reads or three packets in one. What is different here is
//! compression — the game connection's stream is Huffman-coded, and the client
//! is the end that has to undo it.

use openshard_protocol::huffman::{self, HuffmanError};
use openshard_protocol::packet::{Frame, FrameError, MAX_PACKET_SIZE};
use openshard_protocol::server_packet::{ServerDecodeError, ServerPacket, frame_server_packet};
use openshard_protocol::version::ClientVersion;

/// A packet arrived.
#[derive(Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Event {
    /// A packet this crate decodes.
    Packet(ServerPacket),
    /// A packet the framer sized but no decoder exists for yet.
    ///
    /// Not an error and not a dropped connection: the stream is intact, because
    /// framing is what keeps it so, and the caller can log the id and read on.
    /// This is the pile the client works through — every decoder written moves
    /// one id from here to [`Event::Packet`].
    Undecoded {
        /// The packet id.
        id: u8,
        /// The whole packet, id byte included, already decompressed.
        body: Vec<u8>,
    },
}

/// The stream cannot be read any further.
///
/// Every variant is fatal for the connection. There is no resynchronising a UO
/// stream: it has no frame markers, so once the cursor is off by a byte, every
/// byte after it is misread.
#[derive(Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ConnectionError {
    /// A packet id with no length, or a length that cannot be honoured.
    Frame(FrameError),
    /// The compressed stream held a bit sequence that decodes to nothing.
    Huffman(HuffmanError),
    /// A packet decoded to a body that was not what its id promised.
    Decode(ServerDecodeError),
    /// One compressed block held more than the one packet it should.
    ///
    /// The shard compresses every packet independently, so a decompressed block
    /// is exactly one packet. More than that means the sender does not
    /// compress the way this decoder assumes, and reading on would produce
    /// packets that look plausible and are not.
    Desynchronised {
        /// The id of the packet at the front of the block.
        id: u8,
        /// How long the block decompressed to.
        block: usize,
        /// How long the packet at its front claims to be.
        packet: usize,
    },
    /// Bytes are piling up that should have framed and did not.
    ///
    /// Unreachable unless framing has a bug: a variable packet cannot claim
    /// more than [`MAX_PACKET_SIZE`], so a well-behaved stream never buffers
    /// more than one packet's worth.
    Overflow {
        /// How many bytes are pending.
        buffered: usize,
    },
}

impl From<FrameError> for ConnectionError {
    fn from(error: FrameError) -> Self {
        Self::Frame(error)
    }
}

impl From<HuffmanError> for ConnectionError {
    fn from(error: HuffmanError) -> Self {
        Self::Huffman(error)
    }
}

impl From<ServerDecodeError> for ConnectionError {
    fn from(error: ServerDecodeError) -> Self {
        Self::Decode(error)
    }
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Frame(error) => error.fmt(f),
            Self::Huffman(error) => error.fmt(f),
            Self::Decode(error) => error.fmt(f),
            Self::Desynchronised { id, block, packet } => write!(
                f,
                "a compressed block of {block} bytes held a 0x{id:02X} of {packet}: \
                 the sender does not compress packet by packet"
            ),
            Self::Overflow { buffered } => {
                write!(f, "{buffered} bytes buffered and none of them frame")
            }
        }
    }
}

impl std::error::Error for ConnectionError {}

/// The bound on the inbox, for the same reason the gateway has one: it turns a
/// framing bug into a dropped connection instead of a growing buffer.
///
/// A compressed packet can be *longer* than its input — the table is tuned for
/// typical content, and incompressible bytes cost more than eight bits each —
/// so the cap is twice a packet rather than exactly one.
const MAX_BUFFERED: usize = MAX_PACKET_SIZE * 2;

/// How the stream is encoded.
///
/// # Which one a socket is, is not negotiated
///
/// The login connection is plain and the game connection is compressed from its
/// first byte, and there is no packet that says so — the client simply knows
/// which socket it opened. On the server the same fact comes off the login
/// state machine (`Session::send_packet` asks `LoginSession::is_game_login`),
/// which is why neither end carries a flag the other has to agree with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stream {
    /// The login connection: bytes as written.
    Plain,
    /// The game connection: every packet Huffman-coded on its own.
    Compressed,
}

/// The client's half of one connection.
///
/// ```
/// use openshard_client_net::connection::{Connection, Event, Stream};
/// use openshard_protocol::server_packet::ServerPacket;
/// use openshard_protocol::version::ClientVersion;
/// use openshard_protocol::world::LoginComplete;
///
/// let version = ClientVersion::new(7, 0, 45, 65);
/// let mut connection = Connection::new(Stream::Plain, version);
///
/// // 0x55 "you may start drawing", delivered one byte at a time.
/// connection.receive(&ServerPacket::LoginComplete(LoginComplete).encode(version));
///
/// assert_eq!(
///     connection.poll().unwrap(),
///     Some(Event::Packet(ServerPacket::LoginComplete(LoginComplete))),
/// );
/// assert_eq!(connection.poll().unwrap(), None);
/// ```
#[derive(Debug)]
pub struct Connection {
    stream: Stream,
    version: ClientVersion,
    /// Bytes received and not yet turned into a packet.
    inbox: Vec<u8>,
}

impl Connection {
    /// A connection reading `stream`, speaking as `version`.
    ///
    /// The version is not optional and never changes: a client knows what it is
    /// before it opens a socket, and it is what it announced in its own seed.
    /// The server's connection carries an `Option` because it has to frame
    /// packets before it has been told what is on the other end.
    #[must_use]
    pub const fn new(stream: Stream, version: ClientVersion) -> Self {
        Self {
            stream,
            version,
            inbox: Vec::new(),
        }
    }

    /// Hand over bytes that arrived. Never fails; [`Connection::poll`] reports.
    pub fn receive(&mut self, bytes: &[u8]) {
        self.inbox.extend_from_slice(bytes);
    }

    /// How many bytes are waiting to be parsed.
    #[must_use]
    pub fn buffered(&self) -> usize {
        self.inbox.len()
    }

    /// Take the next event, if one is ready.
    ///
    /// `Ok(None)` means "nothing complete yet, feed me more". Loop until it
    /// returns `None`: one read routinely carries several packets, and handling
    /// only the first stalls the rest until the socket happens to deliver again.
    pub fn poll(&mut self) -> Result<Option<Event>, ConnectionError> {
        let event = match self.stream {
            Stream::Plain => self.poll_plain()?,
            Stream::Compressed => self.poll_compressed()?,
        };

        // Checked after, not before: a full buffer that just yielded a packet is
        // healthy. Only a full buffer that yields nothing is a bug.
        if event.is_none() && self.inbox.len() > MAX_BUFFERED {
            return Err(ConnectionError::Overflow {
                buffered: self.inbox.len(),
            });
        }
        Ok(event)
    }

    fn poll_plain(&mut self) -> Result<Option<Event>, ConnectionError> {
        match frame_server_packet(&self.inbox, self.version)? {
            Frame::Incomplete { .. } => Ok(None),
            Frame::Complete(length) => {
                let packet: Vec<u8> = self.inbox.drain(..length).collect();
                self.interpret(packet).map(Some)
            }
        }
    }

    fn poll_compressed(&mut self) -> Result<Option<Event>, ConnectionError> {
        let Some((packet, consumed)) = huffman::decompress_prefix(&self.inbox)? else {
            return Ok(None);
        };
        self.inbox.drain(..consumed);

        // The decompressed block must be exactly one packet. Framing it is how
        // that is checked rather than assumed: a block that frames short means
        // the sender batches packets into one compressed run, and every packet
        // after the first would be read out of the middle of a buffer nobody
        // split.
        let id = *packet
            .first()
            .expect("a compressed block decoded to nothing, which the terminator makes impossible");
        match frame_server_packet(&packet, self.version)? {
            Frame::Complete(length) if length == packet.len() => self.interpret(packet).map(Some),
            Frame::Complete(length) => Err(ConnectionError::Desynchronised {
                id,
                block: packet.len(),
                packet: length,
            }),
            Frame::Incomplete { needed } => Err(ConnectionError::Desynchronised {
                id,
                block: packet.len(),
                packet: needed,
            }),
        }
    }

    /// Decode one whole, uncompressed packet.
    fn interpret(&self, packet: Vec<u8>) -> Result<Event, ConnectionError> {
        let id = packet[0];
        match ServerPacket::decode(&packet, self.version)? {
            Some(decoded) => Ok(Event::Packet(decoded)),
            None => Ok(Event::Undecoded { id, body: packet }),
        }
    }
}

#[cfg(test)]
mod tests {
    use openshard_protocol::login::{DenyReason, LoginDenied};
    use openshard_protocol::world::{Light, LightLevel, LoginComplete};

    use super::*;

    fn version() -> ClientVersion {
        ClientVersion::new(7, 0, 45, 65)
    }

    fn denied() -> ServerPacket {
        ServerPacket::LoginDenied(LoginDenied {
            reason: DenyReason::BadPassword,
        })
    }

    #[test]
    fn a_packet_split_across_reads_arrives_once() {
        // The reason this type is sans-io: a socket cannot be asked to deliver
        // a packet one byte at a time, and this is exactly what one does.
        let bytes = denied().encode(version());
        let mut connection = Connection::new(Stream::Plain, version());

        for byte in &bytes[..bytes.len() - 1] {
            connection.receive(&[*byte]);
            assert_eq!(connection.poll().unwrap(), None, "not whole yet");
        }
        connection.receive(&bytes[bytes.len() - 1..]);
        assert_eq!(connection.poll().unwrap(), Some(Event::Packet(denied())));
        assert_eq!(connection.poll().unwrap(), None);
    }

    #[test]
    fn several_packets_in_one_read_all_come_out() {
        // The other half of the same problem, and the reason `poll` is a loop
        // rather than a call: TCP coalesces, and a client that handles one
        // packet per read falls behind and never catches up.
        let mut bytes = denied().encode(version());
        bytes.extend(ServerPacket::LoginComplete(LoginComplete).encode(version()));

        let mut connection = Connection::new(Stream::Plain, version());
        connection.receive(&bytes);

        assert_eq!(connection.poll().unwrap(), Some(Event::Packet(denied())));
        assert_eq!(
            connection.poll().unwrap(),
            Some(Event::Packet(ServerPacket::LoginComplete(LoginComplete)))
        );
        assert_eq!(connection.poll().unwrap(), None);
    }

    #[test]
    fn a_compressed_stream_comes_apart_packet_by_packet() {
        // What the game connection actually delivers: each packet compressed on
        // its own, then whatever TCP chose to hand over.
        let mut wire = huffman::compress(&denied().encode(version()));
        wire.extend(huffman::compress(
            &ServerPacket::LoginComplete(LoginComplete).encode(version()),
        ));

        let mut connection = Connection::new(Stream::Compressed, version());
        connection.receive(&wire);

        assert_eq!(connection.poll().unwrap(), Some(Event::Packet(denied())));
        assert_eq!(
            connection.poll().unwrap(),
            Some(Event::Packet(ServerPacket::LoginComplete(LoginComplete)))
        );
        assert_eq!(connection.poll().unwrap(), None);
    }

    #[test]
    fn a_compressed_packet_split_across_reads_waits() {
        // Half a compressed packet is not an error: the terminator simply has
        // not arrived. Treating it as one would drop the connection every time
        // a packet spanned two segments.
        let wire = huffman::compress(&denied().encode(version()));
        let mut connection = Connection::new(Stream::Compressed, version());

        connection.receive(&wire[..wire.len() - 1]);
        assert_eq!(connection.poll().unwrap(), None);

        connection.receive(&wire[wire.len() - 1..]);
        assert_eq!(connection.poll().unwrap(), Some(Event::Packet(denied())));
    }

    #[test]
    fn a_packet_with_no_decoder_still_advances_the_stream() {
        // The point of `Undecoded`: an id nobody has written a decoder for must
        // not cost the packets behind it.
        let light = ServerPacket::LightLevel(LightLevel { level: Light(0x10) });
        let mut bytes = light.encode(version());
        bytes.extend(ServerPacket::LoginComplete(LoginComplete).encode(version()));

        let mut connection = Connection::new(Stream::Plain, version());
        connection.receive(&bytes);

        assert_eq!(
            connection.poll().unwrap(),
            Some(Event::Undecoded {
                id: 0x4F,
                body: light.encode(version()),
            })
        );
        assert_eq!(
            connection.poll().unwrap(),
            Some(Event::Packet(ServerPacket::LoginComplete(LoginComplete)))
        );
    }

    #[test]
    fn an_id_the_shard_never_sends_ends_the_connection() {
        // There is no resynchronising: without a length there is no way to know
        // where the next packet starts, so the only honest move is to stop.
        let mut connection = Connection::new(Stream::Plain, version());
        connection.receive(&[0xD6, 0x00, 0x05, 0x01, 0x02]);
        assert!(matches!(
            connection.poll(),
            Err(ConnectionError::Frame(FrameError::UnknownPacket(0xD6)))
        ));
    }

    #[test]
    fn a_batched_compressed_block_is_refused() {
        // If a server compressed two packets into one run, the block would
        // decompress to more than one packet. Reading only the first and
        // discarding the rest would silently drop packets; this says so.
        let mut both = denied().encode(version());
        both.extend(ServerPacket::LoginComplete(LoginComplete).encode(version()));
        let wire = huffman::compress(&both);

        let mut connection = Connection::new(Stream::Compressed, version());
        connection.receive(&wire);
        assert!(matches!(
            connection.poll(),
            Err(ConnectionError::Desynchronised { id: 0x82, .. })
        ));
    }
}
