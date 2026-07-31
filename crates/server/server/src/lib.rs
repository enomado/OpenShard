//! The shard, as a library.
//!
//! Two loops and a channel between them:
//!
//! ```text
//!   gateway tasks ──> ServerEvent ──> [ this loop ] ──> Command ──> World::tick
//!                                            │                          │
//!                                            ├──────  Outbound  <───────┤
//!                                            │                          │
//!                                            ├──> [ save task ]  <──  Snapshot
//!                                            │           │
//!                                            │         a disk
//!                                            │
//!                                            ├──> [ argon2 ] ──> Verdict ──┐
//!                                            └─────────────────────────────┘
//! ```
//!
//! Everything that leaves this loop leaves because waiting for it here would
//! stop the world: a disk that is slow, a hash that is slow on purpose. Both
//! come back as something to react to, on the same `select!` as a packet.
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
use std::time::{Duration, Instant};

use openshard_config::{Config, DEFAULT_TOML};
use openshard_gateway::{
    ClientGatewayServer, ConnectionId, Event, OutboxTx, Packet, PacketError, ServerEvent, ServerEventRx,
    Shutdown, VersionTx,
};
use openshard_login::{Accounts, DevAccounts, LoginServer, LoginSession, Outcome, Response};
use openshard_persistence::{AccountRecord, MemoryStore, PgStore, Snapshot, SqliteStore, Store};
use openshard_protocol::client_packet::ClientPacket;
use openshard_protocol::encoded::EncodedSubcommand;
use openshard_protocol::extended::ExtendedRequest;
use openshard_protocol::login::{
    CharacterListFlags, ClientLoginDecodeError, LoginStagePacket, StartLocation, SupportedFeatures,
};
use openshard_protocol::mobile::StatusQueryKind;
use openshard_protocol::trade::SecureTradeAction;
use openshard_protocol::wire::ClilocId;
use openshard_protocol::world::{Facet, Point};
use openshard_protocol::{access::AccessLevel, huffman};
use openshard_uofiles::map::Map;
use openshard_uofiles::tiledata::TileData;
use openshard_world::tick::screen::CharacterScreen;
use openshard_world::{
    Command, Gameplay, MapTerrain, PlayerEntered, PlayerLeaving, PlayerLeft, PlayerRefused,
    RestoredCharacters, RestoredItems, StatLock, TICK_INTERVAL, World,
};
use tokio::sync::{Semaphore, mpsc};
use tracing::{debug, error, info, warn};

pub mod boot;
pub mod shard;
pub mod stop;

mod dispatch;
mod scripting;
mod session;
#[cfg(test)]
mod testing;
mod verify;

use boot::{load_config, load_world, open_store};
use dispatch::{dispatch_world_packet, start_cities};
use scripting::Scripts;
use session::{PhaseSync, Session, Sessions};
use shard::{Unwritten, run_shard};
use verify::{Verdict, Verifier};

/// Where the config lives, relative to the working directory.
pub const CONFIG_PATH: &str = "openshard.toml";

/// Load the config, open the store, bind the port, and serve until asked to stop.
///
/// The binary's whole body, kept here so that what an operator starts and what
/// a test could start are the same code.
///
/// Returns once the world has been saved: [`run_shard`] does not return before
/// that, and neither does this.
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config(CONFIG_PATH)?;

    // One stop for the whole process — the listener, every connection, and the
    // tick. It is made here rather than inside any of them, because a shard that
    // stopped in pieces would be a shard whose parts each decided when to go.
    let shutdown = Shutdown::new();

    let (gateway_server, events) = ClientGatewayServer::bind(config.server.listen, shutdown.clone()).await?;
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

    // What the save task has been handed and not yet written. Made here rather
    // than inside the shard because both sides of a forced stop need it: the
    // shard counts into it, and the signal watcher — which exits the process
    // without waiting for the shard — is what reads it, and it has to be able to
    // do that at a moment when `run_shard` is not going to return.
    let unwritten = Unwritten::new();

    // A signal is the operator's way to ask, and this is the only place the
    // process listens for one: the shard loop watches the same `Shutdown` a test
    // would use, so a stop is one thing that happens rather than two paths that
    // have to agree. Installing the handlers here rather than inside the spawned
    // task is deliberate — see [`stop::install`]; until they are installed a
    // `SIGTERM` kills the shard instead of stopping it.
    match stop::install() {
        Ok(signals) => {
            tokio::spawn(stop::watch(signals, shutdown.clone(), unwritten.clone()));
        }
        Err(error) => {
            error!(%error, "cannot listen for stop signals; this shard will only stop when killed")
        }
    }

    tokio::spawn(gateway_server.run());
    run_shard(events, &config, world, store, shutdown, unwritten).await;

    Ok(())
}
