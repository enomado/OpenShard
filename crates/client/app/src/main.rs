//! The client binary: the environment, read into [`openshard_client_app::run`].
//!
//! ```sh
//! OPENSHARD_CLIENT="/path/to/Ultima Online Classic" cargo run -p openshard-client-app
//! ```
//!
//! With `OPENSHARD_ACCOUNT` it logs in to `OPENSHARD_SERVER` (or the default
//! port on this machine); without one it is an offline map viewer. Everything
//! else — the window, the wire, the world — is in the library beside this file,
//! so that a caller with a shard of its own can start the same client without an
//! environment at all. `crates/e2e/playground` is that caller.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::process::ExitCode;

use openshard_client_net::session::{Pick, Plan};
use openshard_protocol::identity::{RawAccountName, RawPlaintextPassword};

/// Where a shard is, when one is asked for and no address is given.
const DEFAULT_SHARD: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 2593);

/// The login this run was asked to make, if it was asked for one.
///
/// The account is what decides: a client with no account has nobody to log in
/// as, and asking for a password on a command line nobody typed would be worse
/// than drawing the map on its own.
fn plan_from_environment() -> Option<(SocketAddrV4, Plan)> {
    let account = std::env::var("OPENSHARD_ACCOUNT").ok()?;
    let password = std::env::var("OPENSHARD_PASSWORD").unwrap_or_default();
    let address = match std::env::var("OPENSHARD_SERVER") {
        Ok(text) => match text.parse() {
            Ok(address) => address,
            Err(error) => {
                eprintln!("OPENSHARD_SERVER is not an address:port: {error}");
                return None;
            }
        },
        Err(_) => DEFAULT_SHARD,
    };
    Some((
        address,
        Plan {
            account: RawAccountName(account),
            password: RawPlaintextPassword(password),
            shard: Pick::First,
            character: std::env::var("OPENSHARD_CHARACTER").map_or(Pick::First, Pick::Named),
        },
    ))
}

fn main() -> ExitCode {
    let Some(dir) = std::env::var_os("OPENSHARD_CLIENT").map(PathBuf::from) else {
        eprintln!("set OPENSHARD_CLIENT to a client install directory");
        return ExitCode::FAILURE;
    };
    openshard_client_app::run(&dir, plan_from_environment())
}
