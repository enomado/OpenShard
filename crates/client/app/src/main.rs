//! The client binary: a command line, read into [`openshard_client_app::run`].
//!
//! ```sh
//! cargo run -p openshard-client-app -- --client "/path/to/Ultima Online Classic"
//! ```
//!
//! Every option is also an environment variable — the same `OPENSHARD_*` names
//! as before — and a `.env` beside the workspace root is read before the command
//! line is parsed, so an install can be named once and never again. `--help`
//! lists both spellings.
//!
//! With an account it logs in to `--server` (or the default port on this
//! machine); without one it is an offline map viewer. Everything else — the
//! window, the wire, the world — is in the library beside this file, so that a
//! caller with a shard of its own can start the same client without an
//! environment or a command line at all. `crates/e2e/playground` is that caller.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use openshard_client_net::session::{Pick, Plan};
use openshard_client_net::transport::Tcp;
use openshard_protocol::identity::{RawAccountName, RawPlaintextPassword};

/// Where a shard is, when one is asked for and no address is given.
const DEFAULT_SHARD: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 2593);

/// A window on a client install, and a shard to play if one was asked for.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// The client install to read.
    #[arg(short, long, env = "OPENSHARD_CLIENT", value_name = "DIR")]
    client: PathBuf,

    /// The account to log in as. Without one this is an offline map viewer.
    #[arg(short, long, env = "OPENSHARD_ACCOUNT")]
    account: Option<String>,

    /// Its password.
    ///
    /// Defaulted rather than required, because a shard in development accepts
    /// whatever it is given and asking for one nobody set would turn a map
    /// viewer into an error.
    #[arg(short, long, env = "OPENSHARD_PASSWORD", default_value = "")]
    password: String,

    /// The shard to connect to.
    #[arg(short, long, env = "OPENSHARD_SERVER", default_value_t = DEFAULT_SHARD, value_name = "ADDR:PORT")]
    server: SocketAddrV4,

    /// The character to play. Without one, the first on the account.
    #[arg(long, env = "OPENSHARD_CHARACTER")]
    character: Option<String>,

    /// Draw overhead speech through this TrueType or OpenType face instead of
    /// `fonts.mul`.
    ///
    /// `fonts.mul` only defines Latin text and a handful of symbols — no
    /// Cyrillic, no anything past `0xFF` — so a shard whose players type in
    /// one of those scripts needs this set, to a `.ttf`/`.otf` on this
    /// machine. Nothing is bundled with the engine — see
    /// `openshard_uofiles::ttf_font`'s doc for why. Unset draws the classic
    /// client's own bitmap faces, unchanged; there is no mixing the two within
    /// one line — see `openshard_client_render::text::collect_ttf`'s doc for
    /// why.
    #[arg(long, env = "OPENSHARD_TTF_FONT", value_name = "FILE")]
    ttf_font: Option<PathBuf>,
}

/// The login this run was asked to make, if it was asked for one.
///
/// The account is what decides: a client with no account has nobody to log in
/// as, and asking for a password nobody typed would be worse than drawing the
/// map on its own.
fn plan(cli: &Cli) -> Option<Plan> {
    let account = cli.account.clone()?;
    Some(Plan {
        account: RawAccountName(account),
        password: RawPlaintextPassword(cli.password.clone()),
        shard: Pick::First,
        character: cli.character.clone().map_or(Pick::First, Pick::Named),
    })
}

fn main() -> ExitCode {
    // Before the command line is parsed, because what the file holds is the
    // environment those `env =` options fall back to.
    openshard_client_app::load_env();
    let cli = Cli::parse();

    // A real client on a real network. `Tcp` is where the address goes: past
    // this line nothing knows what a socket is.
    let shard = plan(&cli).map(|plan| {
        eprintln!("logging in to {}", cli.server);
        (Tcp::at(cli.server), plan)
    });
    openshard_client_app::run(&cli.client, shard, cli.ttf_font)
}
