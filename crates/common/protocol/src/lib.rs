//! Ultima Online wire protocol: client versioning, feature gates, packet encode/decode.
//!
//! OpenShard aims to be compatible with the UO protocol, not with SphereServer.
//! That distinction is the whole design: the protocol is a fixed external
//! contract two decades of clients already implement, while Sphere's internals
//! are one team's answer to it and we are free to give a different one.
//!
//! # Multi-era from the start
//!
//! There is no single "the protocol". A 2.0 client and a 7.0.95 client speak
//! meaningfully different dialects, and a shard picks which ones it accepts.
//! Rather than hard-coding one era and retrofitting the rest later — which means
//! auditing every packet encoder twice — versioning is the first thing this
//! crate models.
//!
//! ```
//! use openshard_protocol::{ClientVersion, Era, Feature};
//!
//! // The client sends its version in the 0xBD seed packet.
//! let client: ClientVersion = "4.0.3.0".parse().unwrap();
//!
//! assert_eq!(client.era(), Era::Aos);
//! assert!(client.supports(Feature::Tooltips));
//! assert!(!client.supports(Feature::TooltipHash));
//! ```
//!
//! # The rule
//!
//! Gameplay and encoder code asks [`ClientVersion::supports`]. It never compares
//! version numbers, and it never branches on [`Era`].
//!
//! Features did not land in era-sized batches — tooltips at 4.0.0a, stat locks
//! at 4.0.1a, tooltip hashes at 4.0.5a, all within "AoS" — so an era check is
//! wrong for most of the clients it covers, and wrong in the worst way: the
//! client drops the packet in silence rather than complaining. Keeping every
//! boundary in [`Feature::since`] means one table to fix when a boundary turns
//! out to be off by a patch.
//!
//! [`Era`] is for coarse decisions only: which map set to load, whether housing
//! is customisable.
//!
//! # Where the numbers come from
//!
//! The version boundaries are ported from SphereServer's `MINCLIVER_*` table.
//! That table is observed protocol behaviour — two decades of finding out which
//! client breaks on what — and it is the one part of Sphere worth carrying
//! across. The architecture around it is not.
//!
//! # Status
//!
//! Versioning, feature gates, the client packet length table, framing and the
//! byte codec are written. Individual packet types are not; see
//! `docs/roadmap.md`.

mod access;
pub mod casting;
pub mod client_packet;
mod codec;
pub mod combat;
pub mod containers;
pub mod context;
mod direction;
pub mod encoded;
mod error;
pub mod extended;
mod feature;
pub mod feedback;
pub mod gump;
pub mod huffman;
pub mod items;
pub mod login;
pub mod mobile;
pub mod packet;
pub mod properties;
mod seed;
pub mod serial;
pub mod server_packet;
pub mod skill;
pub mod speech;
pub mod spellbook;
pub mod target;
pub mod vendor;
mod version;
pub mod wire;
pub mod world;

pub use access::{AccessLevel, UnknownAccessLevel};
pub use codec::{CodecError, CodecResult, PacketReader, PacketWriter};
// `casting`, `client_packet`, `combat`, `containers`, `context`, `encoded`,
// `extended`, `feedback`, `gump`, `items`, `login`, `mobile`, `properties`,
// `serial`, `server_packet`, `skill`, `speech`, `spellbook`, `target`,
// `vendor`, `wire` and `world` are deliberately absent from this wall: they
// are the rewritten groups, and their call sites import from the module the
// type lives in. The wall itself goes in Stage 7 (`docs/protocol_rewrite.md`,
// D8).
pub use direction::{Direction, Facing, RUNNING_BIT};
pub use error::{DecodeError, WrongPacket};
pub use feature::{Feature, FeatureSet};
pub use packet::{
    client_packet_length, frame_client_packet, Frame, FrameError, PacketLength, MAX_PACKET_SIZE,
    SEED_LENGTH_NEW, SEED_LENGTH_OLD,
};
pub use seed::{Seed, SeedReader, SEED_COMMAND};
pub use version::{ClientVersion, Era, ParseVersionError};
