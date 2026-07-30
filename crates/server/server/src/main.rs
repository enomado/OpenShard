//! The shard binary.
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
//! This file owns neither half. The gateway is a state machine with its own
//! tests; the world is a tick with its own. What is here is the wiring: read
//! events, decide whether they are login's or the world's, and drive the clock.
//!
//! It is deliberately thin. If logic starts collecting here it belongs in a
//! crate.

use std::collections::HashMap;
use std::net::{SocketAddr, SocketAddrV4};
use std::path::Path;
use std::process::ExitCode;
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
use openshard_protocol::encoded::EncodedCommand;
use openshard_protocol::extended::ExtendedRequest;
use openshard_protocol::identity::{AccountName, CharacterName};
use openshard_protocol::login::{
    CharacterListUpdate, ClientLoginDecodeError, DeleteCharacter, DeleteReject, DeleteResult, LoginDenied,
    LoginStagePacket, StartLocation,
};
use openshard_protocol::mobile::StatusQueryKind;
use openshard_protocol::server_packet::ServerPacket;
use openshard_protocol::skill::SkillLock;
use openshard_protocol::trade::SecureTradeAction;
use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::{CreateCharacter, Point};
use openshard_protocol::{access::AccessLevel, huffman};
use openshard_world::components::Facet;
use openshard_world::{
    Appearance, Character, CharacterSheet, Command, Entering, FreshCharacter, Gameplay, Map, MapTerrain,
    StatLock, StoredCharacter, TICK_INTERVAL, TileData, World,
};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

mod scripting;
use scripting::Scripts;

mod boot;
mod dispatch;
mod roster;
mod session;
mod shard;
#[cfg(test)]
mod testing;

use boot::{load_config, load_world, open_store};
use dispatch::{create_character, delete_character, dispatch_world_packet, start_cities};
use roster::Roster;
use session::{Session, Sessions};
use shard::run_shard;

/// Where the config lives, relative to the working directory.
const CONFIG_PATH: &str = "openshard.toml";

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Printed rather than returned as a `Result`: `main` returning `Err`
            // renders it with `Debug`, which for a config error is a wall of
            // struct fields instead of the sentence that says what to fix.
            error!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
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

    // better to push to server more semantic actions an do all decompression in ClientGatewayServer before packet go to shard
    run_shard(events, &config, world, store).await;

    Ok(())
}
