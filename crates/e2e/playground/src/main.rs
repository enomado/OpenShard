//! A shard and a window, in one process:
//!
//! ```sh
//! OPENSHARD_CLIENT="/path/to/Ultima Online Classic" cargo run -p openshard-playground
//! ```
//!
//! One command instead of two, no port to pick and no config file to keep in
//! step with one: the shard binds an ephemeral port, the window logs in to
//! whatever it got, and closing the window ends both. Nothing else about the two
//! ends changes — this is a real gateway, a real login conversation and a real
//! pair of sockets on the loopback, because that is the only arrangement in
//! which what runs here is what runs against ClassicUO.
//!
//! # Why a loopback socket and not a channel
//!
//! An in-memory duplex would be one fewer moving part, and it would also be a
//! second transport that only this binary uses. Framing across read boundaries,
//! the relay's second connection, and per-write compression are exactly what
//! `client/net` and `gateway` are careful about, and a transport that skipped
//! them would be quiet about the day one of them broke. The loopback costs a
//! syscall per packet and tests the thing.
//!
//! # What this is not
//!
//! Not a way to run a shard: it keeps nothing. The world is in memory and goes
//! away with the process, which is what makes it a playground — see
//! `cargo run -p openshard-server` for a shard that saves, and the `e2e` tests
//! beside this crate for the same arrangement with assertions instead of a
//! window.

use std::path::PathBuf;
use std::process::ExitCode;

use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let Some(dir) = std::env::var_os("OPENSHARD_CLIENT").map(PathBuf::from) else {
        eprintln!("set OPENSHARD_CLIENT to a client install directory");
        return ExitCode::FAILURE;
    };

    // The shard reads the same install the window does, and that is not a
    // convenience: `world.client_files` is what gives the server a map, and
    // without one every step is allowed at whatever height the client guessed.
    // The client predicts each step's `z` from its own copy of the facet, so two
    // ends reading different files disagree about the ground and the walk turns
    // into a stream of `0x21` rollbacks — which looks like a bug in the client.
    // It costs a second copy of the facet in this process; a playground can
    // afford one, and `docs/client_versions.md` is the standing rule it obeys.
    let files = dir.to_string_lossy().into_owned();
    let address = openshard_e2e_shard::spawn(move |bound| {
        let mut config = openshard_e2e_shard::stock_config(bound);
        config.world.client_files = files;
        config
    });
    eprintln!(
        "shard listening on {address}; logging in as {}",
        openshard_e2e_shard::ACCOUNT
    );

    // On this thread, because `winit` requires the event loop to own the one it
    // was built on. The shard is the one that moved.
    openshard_client_app::run(&dir, Some((address, openshard_e2e_shard::plan())))
}
