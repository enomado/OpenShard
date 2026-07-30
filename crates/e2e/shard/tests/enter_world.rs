//! A client logs in to a shard and ends up standing in the world.
//!
//! Both ends are real: the shard is the same `run_shard` the binary runs, and
//! the client is `openshard-client-net` driving two actual sockets. What this
//! catches is the only thing neither side's own tests can — that the two agree.
//! Every layer below has better tests than this one; this is the seam.
//!
//! The shard runs with no client files and no database: the world is a bare
//! grid where every step is allowed, kept in memory. That is enough to be
//! logged into, and it keeps the test runnable on a machine that has no copy of
//! the client's data.

use std::net::{SocketAddr, SocketAddrV4};
use std::time::Duration;

use openshard_client_net::session::{Pick, Plan};
use openshard_client_net::transport::enter_world;
use openshard_config::{Config, DEFAULT_TOML};
use openshard_gateway::ClientGatewayServer;
use openshard_protocol::identity::{RawAccountName, RawPlaintextPassword};
use openshard_protocol::version::ClientVersion;

/// The client we claim to be. ClassicUO's own opening version, which is what
/// keeps the shard on the modern packet shapes.
fn version() -> ClientVersion {
    ClientVersion::new(7, 0, 45, 65)
}

/// The account the stock config ships with, and which this test logs in as.
const ACCOUNT: &str = "admin";
/// Its password, likewise from the stock config.
const PASSWORD: &str = "hunter2";
/// The character on it.
const CHARACTER: &str = "Lord British";

/// The stock config, pointed at `address`.
///
/// Nothing is added to it: the shipped config already carries the development
/// account this logs in as, and a test that invented its own would stop
/// noticing when that changed.
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
fn shard() -> SocketAddrV4 {
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

fn plan() -> Plan {
    Plan {
        account: RawAccountName(ACCOUNT.to_owned()),
        password: RawPlaintextPassword(PASSWORD.to_owned()),
        shard: Pick::First,
        character: Pick::Named(CHARACTER.to_owned()),
    }
}

#[tokio::test]
async fn a_client_logs_in_and_stands_in_the_world() {
    let address = shard();

    // A bound, rather than waiting forever: every step of this conversation is
    // one side waiting for the other, so the failure mode of a mismatch is a
    // hang, not an error.
    let entered = tokio::time::timeout(Duration::from_secs(20), enter_world(address, plan(), version()))
        .await
        .expect("the login conversation finished inside the timeout");

    let (_socket, view) = entered.expect("the client reached the world");

    // The shard put a body somewhere on the map and told us which one it is.
    // Not asserted against fixed coordinates: where a new character starts is
    // config, and this test is about the conversation, not the map.
    assert!(
        view.map.width > 0 && view.map.height > 0,
        "the facet has a size: {:?}",
        view.map
    );
    assert_ne!(view.player.body.0, 0, "a body graphic was chosen");
}

#[tokio::test]
async fn a_wrong_password_comes_back_as_a_refusal() {
    // The other end of the same seam: a refusal has to reach the client as the
    // reason it was given, not as a socket that closed. The shard sends `0x82`
    // and hangs up immediately after, so a client that only noticed the close
    // would lose the reason it had already been handed.
    let address = shard();
    let plan = Plan {
        password: RawPlaintextPassword("wrong".to_owned()),
        ..plan()
    };

    let outcome = tokio::time::timeout(Duration::from_secs(20), enter_world(address, plan, version()))
        .await
        .expect("the shard answered inside the timeout");

    let error = outcome.expect_err("a wrong password cannot reach the world");
    assert!(
        matches!(
            error,
            openshard_client_net::transport::TransportError::Refused(
                openshard_protocol::login::DenyReason::BadPassword
            )
        ),
        "{error}"
    );
}
