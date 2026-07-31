//! A real shard in this process, and the tests that talk to one.
//!
//! # Why this crate exists at all
//!
//! `server/*` and `client/*` never depend on each other — that rule is what
//! keeps the wire the only thing they agree on, and it is worth keeping. But a
//! test that a *client* can log in to a *shard* needs both ends in one process,
//! and putting it on either side would make that side depend on the other,
//! dev-dependency or not.
//!
//! So it lives outside both. This crate is the only place in the workspace
//! allowed to name both ends, and nothing outside `crates/e2e/*` depends on it.
//!
//! # What belongs here
//!
//! In `tests/`: only what cannot be tested on one side alone. The gateway's
//! framing, the client's login machine and the world's tick all have their own
//! tests, and those are better tests — pure state machines, no ports, no timing.
//! What is left for this crate is the seam: that the two ends, each correct,
//! actually agree.
//!
//! In `src/` — here — the shard those tests need, and nothing that is a test
//! itself. It used to be `tests/common/mod.rs`, and it moved out for one reason:
//! `crates/e2e/playground` wants that same shard with a window in front of it
//! rather than a test, and a `tests/` module cannot be shared with a binary.
//! What is left below is *how to start a shard*, with the one thing a caller
//! legitimately differs on — the config — left to it.

use std::net::{SocketAddr, SocketAddrV4};

use openshard_client_net::session::{Pick, Plan};
use openshard_config::{Config, DEFAULT_TOML};
use openshard_gateway::ClientGatewayServer;
use openshard_protocol::identity::{RawAccountName, RawPlaintextPassword};
use openshard_protocol::version::ClientVersion;

/// The client we claim to be. ClassicUO's own opening version, which is what
/// keeps the shard on the modern packet shapes.
pub fn version() -> ClientVersion {
    ClientVersion::new(7, 0, 45, 65)
}

/// The account the stock config ships with, and which these tests log in as.
pub const ACCOUNT: &str = "admin";
/// Its password, and the one every account here is given: they are all
/// development accounts and none of them is testing a password.
pub const PASSWORD: &str = "hunter2";
/// The character on it.
pub const CHARACTER: &str = "Lord British";

/// A second account, added by [`stock_config`] — see there for why it is not a
/// second character on [`ACCOUNT`].
pub const WITNESS: &str = "witness";
/// The character on it.
pub const NYSTUL: &str = "Nystul";

/// The stock config, pointed at `address`, plus the one account a caller cannot
/// get from it.
///
/// The stock account is not replaced or invented: the shipped config already
/// carries the development account these tests log in as, and a test that made
/// up its own would stop noticing when that changed.
///
/// The port matters twice — the shard listens on it, and the `0x8C` relay tells
/// the client to dial `advertise`. Get the second wrong and the client
/// disconnects politely and never comes back, so both are asserted rather than
/// assumed: they are produced by editing text, and text drifts.
pub fn stock_config(address: SocketAddr) -> Config {
    let port = address.port();
    let text = DEFAULT_TOML
        .replace(
            "listen = \"0.0.0.0:2593\"",
            &format!("listen = \"127.0.0.1:{port}\""),
        )
        .replace(
            "advertise = \"127.0.0.1:2593\"",
            &format!("advertise = \"127.0.0.1:{port}\""),
        );

    // The second account, for the tests that need two players in the world at
    // once. Appended rather than given to `admin` as a second character: two
    // characters on one account would rest on the shard letting one account hold
    // two connections, which nothing states and nothing enforces — an accident
    // to build a fixture on. A whole `[[accounts]]` table at the end of the file
    // is a complete table wherever the sections above it move to.
    let text = format!(
        "{text}\n[[accounts]]\nname = \"{WITNESS}\"\npassword = \"{PASSWORD}\"\ncharacters = [\"{NYSTUL}\"]\n"
    );

    let config: Config = toml::from_str(&text).expect("the stock config parses");
    assert_eq!(
        config.server.listen.port(),
        port,
        "the listen address was not replaced: the stock config's wording changed"
    );
    assert_eq!(
        config.server.advertise.port(),
        port,
        "the advertised address was not replaced: the relay would send the client elsewhere"
    );
    assert!(
        config.accounts.iter().any(|account| {
            account.name == ACCOUNT && account.characters.iter().any(|name| name == CHARACTER)
        }),
        "the stock config no longer ships {ACCOUNT} with {CHARACTER}"
    );
    assert!(
        config.accounts.iter().any(|account| {
            account.name == WITNESS && account.characters.iter().any(|name| name == NYSTUL)
        }),
        "the appended account did not come back out of the parse: the shape of an \
         [[accounts]] table changed"
    );
    config
}

/// Start a shard on an ephemeral port and hand back where it listens.
///
/// `config` is called with the address the gateway actually bound, because the
/// port is in the config twice — `listen`, and the `advertise` the `0x8C` relay
/// hands out — and neither can be filled in before the bind. That is the whole
/// of what a caller varies: [`shard`] is what the tests want, and
/// `crates/e2e/playground` builds one that reads a client install as well.
///
/// # Why a thread and not a `tokio::spawn`
///
/// The shard owns a V8 isolate, so its future is not `Send` and cannot be
/// spawned onto a multi-threaded runtime — the binary does not spawn it either,
/// it awaits it in `main`. A thread with its own current-thread runtime is that
/// same arrangement, next door to the caller.
pub fn spawn(config: impl FnOnce(SocketAddr) -> Config + Send + 'static) -> SocketAddrV4 {
    let (ready, listening) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime for the shard");

        runtime.block_on(async move {
            let (gateway, events) = ClientGatewayServer::bind("127.0.0.1:0".parse().unwrap())
                .await
                .expect("a loopback port");
            let address = gateway.local_address().expect("the bound address");

            // `run_shard` borrows the config for as long as it runs, which is
            // the life of the process. Leaking one config is the honest way to
            // say that here; the binary gets the same lifetime from `main`'s
            // stack frame.
            let config: &'static Config = Box::leak(Box::new(config(address)));
            let world = openshard_server::boot::load_world(config).expect("a world");
            let store = openshard_server::boot::open_store(config).await.expect("a store");

            tokio::spawn(gateway.run());
            ready.send(address).expect("the caller is still waiting");
            openshard_server::shard::run_shard(events, config, world, store).await;
        });
    });

    match listening.recv().expect("the shard came up") {
        SocketAddr::V4(address) => address,
        SocketAddr::V6(_) => unreachable!("bound to a v4 loopback address"),
    }
}

/// A shard on the stock config: no map, no database, two development accounts.
pub fn shard() -> SocketAddrV4 {
    spawn(stock_config)
}

/// Log in as the stock account and play its character.
pub fn plan() -> Plan {
    plan_for(ACCOUNT, CHARACTER)
}

/// Log in as `account` and play the character called `character`.
///
/// The password is [`PASSWORD`] whoever the account is — see it for why there is
/// no parameter for one.
pub fn plan_for(account: &str, character: &str) -> Plan {
    Plan {
        account: RawAccountName(account.to_owned()),
        password: RawPlaintextPassword(PASSWORD.to_owned()),
        shard: Pick::First,
        character: Pick::Named(character.to_owned()),
    }
}
