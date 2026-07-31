//! A real shard on a real port, for the tests that need one.
//!
//! Shared rather than copied because every test here starts the same way, and a
//! second copy of `config_for` would go stale the first time the stock config's
//! wording changed — the assertions inside it are the thing that notices.

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
/// Its password, and the one every account below is given: they are all
/// development accounts and none of them is testing a password.
pub const PASSWORD: &str = "hunter2";
/// The character on it.
pub const CHARACTER: &str = "Lord British";

/// A second account, added by [`config_for`] — see there for why it is not a
/// second character on [`ACCOUNT`].
pub const WITNESS: &str = "witness";
/// The character on it.
pub const NYSTUL: &str = "Nystul";

/// The stock config, pointed at `address`, plus the one account a test cannot
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
fn config_for(address: SocketAddr) -> Config {
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
/// # Why a thread and not a `tokio::spawn`
///
/// The shard owns a V8 isolate, so its future is not `Send` and cannot be
/// spawned onto a multi-threaded runtime — the binary does not spawn it either,
/// it awaits it in `main`. A thread with its own current-thread runtime is that
/// same arrangement, next door to the test.
pub fn shard() -> SocketAddrV4 {
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
            // the life of the test process. Leaking one config is the honest way
            // to say that here; the binary gets the same lifetime from `main`'s
            // stack frame.
            let config: &'static Config = Box::leak(Box::new(config_for(address)));
            let world = openshard_server::boot::load_world(config).expect("a world with no client files");
            let store = openshard_server::boot::open_store(config)
                .await
                .expect("an in-memory store");

            tokio::spawn(gateway.run());
            ready.send(address).expect("the test is still waiting");
            openshard_server::shard::run_shard(events, config, world, store).await;
        });
    });

    match listening.recv().expect("the shard came up") {
        SocketAddr::V4(address) => address,
        SocketAddr::V6(_) => unreachable!("bound to a v4 loopback address"),
    }
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
