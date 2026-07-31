//! A shard and a window, in one process:
//!
//! ```sh
//! OPENSHARD_CLIENT="/path/to/Ultima Online Classic" cargo run -p openshard-playground
//! ```
//!
//! One command instead of two, and no network at all: **no port is bound and no
//! socket is opened**. The client dials the shard through
//! [`openshard_e2e_shard::in_process`], which is a pair of in-memory pipes, and
//! closing the window ends both ends.
//!
//! # What that does and does not remove
//!
//! Not the protocol. Both ends run exactly the code they run against ClassicUO
//! — the client's framing and login machine, the relay's second connection,
//! per-write compression, and the gateway's own `client_session_serve`. The
//! transport is a type parameter on either side (`Dial` for the client, any
//! stream for the gateway) and everything above it is untouched, which is the
//! only arrangement where this is worth having: a second implementation that
//! agreed with the first would be the thing that goes quietly out of step.
//!
//! What is gone is the kernel — segment boundaries, resets, and anything about
//! timing that a real network decides. The socket tests in `crates/e2e/shard`
//! cover that and stay where they are.
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
    let (dial, shard) = openshard_e2e_shard::in_process::spawn(move |stated| {
        let mut config = openshard_e2e_shard::stock_config(stated);
        config.world.client_files = files;
        config
    });
    eprintln!(
        "shard up in this process; logging in as {}",
        openshard_e2e_shard::ACCOUNT
    );

    // On this thread, because `winit` requires the event loop to own the one it
    // was built on. The shard is the one that moved.
    let code = openshard_client_app::run(&dir, Some((dial, openshard_e2e_shard::plan())));

    // The window is gone, so the shard is asked to stop and waited for. It keeps
    // nothing — the world is in memory and goes away with it — but the wait is
    // still worth having: it is the same path an operator's Ctrl-C takes, so a
    // stop that hangs or panics shows up here rather than only in production.
    shard.stop();
    code
}
