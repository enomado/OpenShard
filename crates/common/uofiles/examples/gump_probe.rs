//! Print how big a gump is — the question every window layout asks and no
//! source file answers.
//!
//! A window's layout is arithmetic over the sizes of the pictures it is built
//! from: a scroll's body starts under its top piece, a row of a list is as tall
//! as the button on it, and none of those numbers is written down anywhere but
//! in `gumpartLegacyMUL.uop` itself. The reference client asks the same question
//! at run time (`GetGump(graphic).UV.Height`); this is that question from a
//! terminal, so a constant in a layout can be checked rather than guessed.
//!
//! ```sh
//! cargo run -p openshard-uofiles --example gump_probe -- 0x1F40 0x1F41 0x0834
//! ```

use openshard_protocol::wire::Graphic;
use openshard_uofiles::gumpart::Gumps;

fn main() {
    let dir = std::path::PathBuf::from(std::env::var_os("OPENSHARD_CLIENT").expect("OPENSHARD_CLIENT"));
    let gumps = Gumps::open(&dir).expect("a client ships gump art");
    for argument in std::env::args().skip(1) {
        let graphic = match argument.strip_prefix("0x") {
            Some(hex) => u16::from_str_radix(hex, 16).expect("a hex gump id"),
            None => argument.parse().expect("a decimal gump id"),
        };
        match gumps.gump(Graphic(graphic)) {
            Ok(Some(image)) => println!("0x{graphic:04X} ({graphic}) {}x{}", image.width(), image.height()),
            Ok(None) => println!("0x{graphic:04X} ({graphic}) — the client ships none"),
            Err(error) => println!("0x{graphic:04X} ({graphic}) — {error}"),
        }
    }
}
