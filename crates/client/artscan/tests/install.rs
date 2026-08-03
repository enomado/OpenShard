//! What the tool says about a real install, and whether the file it writes says
//! the same thing.
//!
//! Two different claims, and the second is the one this crate exists to make
//! safe: `docs/lighting.md`'s decision 31 moves a measurement out of the frame,
//! so from now on what the client believes about a wall is *a file* rather than a
//! function. A round trip that lost corners, or a coverage count that quietly
//! meant something else, would be a renderer answering confidently out of a table
//! nobody checked.
//!
//! Ignored and gated on `OPENSHARD_CLIENT`: no client files live in this
//! repository, ever.
//!
//! ```sh
//! OPENSHARD_CLIENT=… cargo test --release -p openshard-client-artscan \
//!     --test install -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use openshard_client_artscan as artscan;
use openshard_client_render::arttable::{ArtTable, Stamp};
use openshard_client_render::facing;
use openshard_protocol::wire::Graphic;
use openshard_uofiles::art::Art;

/// How many pictures a sweep of a real install has to have looked at.
///
/// A modern client ships tens of thousands of static graphics. The floor is
/// there for the failure this cannot otherwise see: a sweep that stopped early —
/// a `break` where a `continue` belongs — would write a table whose *absent* rows
/// mean "refused" about fifty thousand pictures it never opened.
const MUST_EXAMINE: usize = 10_000;

/// And how many it has to have read something off.
///
/// `tests/facing.rs` in the render crate reads 943 wall graphics with faces and
/// 297 corners; this sweep offers every graphic rather than the walls alone, so
/// its number is at least that. Measured rather than chosen, and the floor is
/// under it by a margin for the same reason that file's are.
const MUST_READ: usize = 800;

/// The corners as their own count, because a tail hidden in a percentage is a
/// tail that can go to zero unnoticed — decision 25's, and the same floor
/// `tests/facing.rs` puts under it.
const MUST_READ_CORNERS: usize = 100;

/// **The table says exactly what a live measurement says.**
///
/// Every graphic the install ships, twice: once through [`artscan::measure`] and
/// the file it writes, and once through [`facing::facing_of`] on the spot. The
/// two must agree for all of them, including the ones both refuse — where the
/// table's answer is the *absence* of a row and the contract is that absence
/// means "measured and refused" rather than "never looked at".
///
/// What it catches, in the order the mistakes are easy to make: a verdict that
/// does not survive being written down (a corner's two halves are two words on
/// one line and swapping them lights a wall from inside), a sweep that skipped
/// part of the graphic space, and a reader that resolves a missing row by
/// re-measuring — which would work and would quietly undo the whole step.
#[test]
#[ignore]
fn a_written_table_says_what_a_live_measurement_says() {
    let Some(dir) = client() else { return };
    let art = Art::open(&dir).expect("the client's art");
    let stamp = artscan::stamp_of(&dir).expect("the art file's own length");

    let table = artscan::measure(&art, stamp.clone(), &[artscan::overrides()]);
    println!("pictures with art: {}", table.examined());
    println!("read:              {}", table.decided());
    println!("corners:           {}", table.corners());
    assert!(
        table.examined() >= MUST_EXAMINE,
        "a sweep that looked at {} pictures did not look at an install",
        table.examined(),
    );
    assert!(
        table.decided() >= MUST_READ,
        "the tool read {} pictures, under the floor of {MUST_READ} — a gate has been tightened \
         until the feature stops applying",
        table.decided(),
    );
    assert!(
        table.corners() >= MUST_READ_CORNERS,
        "only {} corners, under the floor of {MUST_READ_CORNERS}",
        table.corners(),
    );

    // Through the text, because the text is what the client reads. A comparison
    // against the in-memory table would test the sweep and nothing about the
    // file, which is the half that is new here.
    let read = ArtTable::parse(&table.to_text()).expect("its own text");
    assert!(read.fresh(&stamp), "the table it wrote describes this install");

    let mut checked = 0usize;
    for id in 0..=u16::MAX {
        let graphic = Graphic(id);
        let Ok(Some(image)) = art.static_art(graphic) else {
            continue;
        };
        checked += 1;
        assert_eq!(
            read.facing(graphic),
            facing::facing_of(&image),
            "{graphic:?} — the table and a live measurement disagree",
        );
    }
    assert_eq!(checked, table.examined(), "the sweep counted what it looked at");
}

/// **A stale table is not loaded**, over a real install and through the real
/// reader.
///
/// Decision 31.4. The unit tests hold the `==` on the stamp; what this holds is
/// the path from a file on disk to a refusal — including the one thing a unit
/// test cannot reach, which is that [`artscan::stamp_of`] reads the *same* art
/// file the tool measured.
#[test]
#[ignore]
fn a_table_from_another_install_is_not_loaded() {
    let Some(dir) = client() else { return };
    let stamp = artscan::stamp_of(&dir).expect("the art file's own length");

    let table = std::env::temp_dir().join("openshard-artscan-stale.table");
    // Safety of a sort: the reader's path is `OPENSHARD_ART_TABLE` when it is
    // set, so this never touches the table beside a person's own install.
    std::env::set_var("OPENSHARD_ART_TABLE", &table);

    let wrong = ArtTable::measured(Stamp {
        bytes: stamp.bytes + 1,
        ..stamp.clone()
    });
    artscan::save(&table, &wrong).expect("writing a stale table");
    let error = artscan::load(&dir).expect_err("a table from another install");
    println!("refused, and says why: {error}");

    let right = ArtTable::measured(stamp);
    artscan::save(&table, &right).expect("writing a fresh one");
    artscan::load(&dir).expect("and this one loads");

    std::fs::remove_file(&table).expect("cleaning up");
    std::env::remove_var("OPENSHARD_ART_TABLE");
}

fn client() -> Option<PathBuf> {
    std::env::var_os("OPENSHARD_CLIENT").map(PathBuf::from)
}
