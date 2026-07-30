//! The shard, as a library.
//!
//! Two loops and a channel between them:
//!
//! ```text
//!   gateway tasks ──> ServerEvent ──> [ this loop ] ──> Command ──> World::tick
//!                                            │                          │
//!                                            ├──────  Outbound  <───────┤
//!                                            │                          │
//!                                            └──> [ save task ]  <──  Snapshot
//!                                                      │
//!                                                    a disk
//! ```
//!
//! This crate owns neither half. The gateway is a state machine with its own
//! tests; the world is a tick with its own. What is here is the wiring: read
//! events, decide whether they are login's or the world's, and drive the clock.
//!
//! It is deliberately thin. If logic starts collecting here it belongs in a
//! crate.
//!
//! # Why a library and not only a binary
//!
//! [`shard::run_shard`] is the whole shard, and a test that wants to *be a
//! client* needs one running. Out of process that means building a binary,
//! writing a config file, guessing a port and waiting for a log line; in
//! process it means calling a function. So the wiring is a library, `main.rs`
//! is the few lines that start it, and `crates/e2e` logs in over a real socket
//! against the same code an operator runs.
//!
//! # The `use` list below is load-bearing
//!
//! Every module here opens with `use super::*`, so the crate root is where
//! their shared imports live. Removing one because this file does not name it
//! breaks a module that does.

use std::collections::HashMap;
use std::net::{SocketAddr, SocketAddrV4};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use openshard_config::{Config, DEFAULT_TOML};
use openshard_gateway::{
    ClientGatewayServer, ConnectionId, Event, OutboxTx, Packet, PacketError, ServerEvent, ServerEventRx,
    VersionTx,
};
use openshard_login::{Accounts, DevAccounts, LoginServer, LoginSession, Response};
use openshard_persistence::{
    AccountRecord, CharacterRecord, MemoryStore, PgStore, Snapshot, SqliteStore, Store,
};
use openshard_protocol::client_packet::ClientPacket;
use openshard_protocol::encoded::EncodedSubcommand;
use openshard_protocol::extended::ExtendedRequest;
use openshard_protocol::identity::{AccountName, CharacterName};
use openshard_protocol::login::{
    CharacterListFlags, CharacterListUpdate, ClientLoginDecodeError, DeleteCharacter, DeleteReject,
    DeleteResult, LoginDenied, LoginStagePacket, StartLocation, SupportedFeatures,
};
use openshard_protocol::mobile::StatusQueryKind;
use openshard_protocol::server_packet::ServerPacket;
use openshard_protocol::skill::SkillLock;
use openshard_protocol::trade::SecureTradeAction;
use openshard_protocol::wire::{ClilocId, Graphic, Hue};
use openshard_protocol::world::{CreateCharacter, Facet, Point};
use openshard_protocol::{access::AccessLevel, huffman};
use openshard_world::{
    Appearance, Character, CharacterSheet, Command, Entering, FreshCharacter, Gameplay, Map, MapTerrain,
    StatLock, StoredCharacter, TICK_INTERVAL, TileData, World,
};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

pub mod boot;
pub mod shard;

mod dispatch;
mod roster;
mod scripting;
mod session;
#[cfg(test)]
mod testing;

use boot::{load_config, load_world, open_store};
use dispatch::{create_character, delete_character, dispatch_world_packet, start_cities};
use roster::Roster;
use scripting::Scripts;
use session::{Session, Sessions};
use shard::run_shard;

/// Where the config lives, relative to the working directory.
pub const CONFIG_PATH: &str = "openshard.toml";

/// Load the config, open the store, bind the port, and serve until the process
/// ends.
///
/// The binary's whole body, kept here so that what an operator starts and what
/// a test could start are the same code.
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config(CONFIG_PATH)?;

    let (gateway_server, events) = ClientGatewayServer::bind(config.server.listen).await?;
    info!(
        shard = config.server.name,
        listen = %config.server.listen,
        advertise = %config.server.advertise,
        accounts = config.accounts.len(),
        "OpenShard starting"
    );
    if config.server.advertise.ip().is_loopback() {
        warn!(
            "server.advertise is loopback: only clients on this machine can reach the shard. \
             Set it to the address clients dial."
        );
    }

    let world = load_world(&config)?;
    let store = open_store(&config).await?;

    tokio::spawn(gateway_server.run());
    run_shard(events, &config, world, store).await;

    Ok(())
}
