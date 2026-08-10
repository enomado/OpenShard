//! Scratch: **how sure the prism search is about which way a picture climbs.**
//!
//! `facing::best_prism` returns one prism and one score, and a score alone
//! cannot say whether the winner won. The question a staircase raises is the
//! other one: the six graphics of one flight at Britain's `(1454, 1728)` come
//! out fitted `prism N`, `prism W`, `prism E`, `prism E`, a plain box and no
//! prism at all — four climb axes on one staircase — and the boxes that
//! produces stand at right angles to each other under one picture. See
//! `docs/lighting_rebuild.md`'s backlog.
//!
//! So this prints the whole ranking rather than the winner: every candidate the
//! search considers, best first, with the *margin* over the best candidate that
//! climbs a **different** way. A margin of a few thousandths means the axis was
//! a coin flip and the answer is noise; a wide one means the pieces genuinely
//! differ and what is missing is a rule above one picture.
//!
//! ```sh
//! OPENSHARD_CLIENT=… cargo run -p openshard-client-artscan --example prism_axis -- 1873 1874
//! ```
//!
//! The candidate sweep is `facing::candidates`' own, restated here because that
//! function is private and this is a scratch tool rather than a second reader of
//! the search: boxes to `MAX_PRISM`, then an even climb per (face, treads, top).
//! `MAX_PRISM` is private too and is the one number copied — a disagreement
//! shows up as a candidate this tool ranks and the search never saw, which is
//! visible in the printed count.

use openshard_client_render::facing::{self, Face, Prism};
use openshard_protocol::wire::Graphic;
use openshard_uofiles::art::Art;

/// `facing::MAX_PRISM`, which is private. See the module doc.
const MAX_PRISM: u8 = 20;

fn main() {
    let dir = std::path::PathBuf::from(std::env::var_os("OPENSHARD_CLIENT").expect("client"));
    let art = Art::open(&dir).expect("art");

    let mut sweep: Vec<Prism> = (0..=MAX_PRISM).map(Prism::box_of).collect();
    for up in [Face::North, Face::East, Face::South, Face::West] {
        for treads in 2..=facing::MAX_TREADS {
            for top in 1..=u16::from(MAX_PRISM) {
                let profile: Vec<u8> = (1..=treads).map(|i| (top * i / treads) as u8).collect();
                sweep.push(Prism::new(up, &profile).expect("a legal profile by construction"));
            }
        }
    }
    let sweep: Vec<(Prism, openshard_uofiles::image::Image)> = sweep
        .into_iter()
        .map(|prism| {
            let silhouette = facing::prism_silhouette(&prism);
            (prism, silhouette)
        })
        .collect();
    println!("{} candidates in the sweep", sweep.len());

    for arg in std::env::args().skip(1) {
        let id: u16 = arg.parse().expect("graphic");
        let Ok(Some(image)) = art.static_art(Graphic(id)) else {
            println!("{id}: no art");
            continue;
        };
        let mut ranked: Vec<(f32, &Prism)> = sweep
            .iter()
            .map(|(prism, silhouette)| (facing::silhouettes_agree(&image, silhouette), prism))
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));

        // A **box** has no climb: `Prism::up` answers for a one-tread prism the
        // way `Prism::new` was called, and nothing about the shape depends on
        // it. So the axis question is only about the multi-tread candidates, and
        // a box is printed as its own kind rather than as a fifth direction.
        let axis = |prism: &Prism| match prism.treads().len() {
            1 => None,
            _ => Some(prism.up()),
        };
        let winner = ranked[0].1;
        let rival = ranked
            .iter()
            .find(|(_, prism)| axis(prism) != axis(winner))
            .expect("a sweep with boxes and four faces in it always has a rival");
        println!(
            "\n{id} (0x{id:04X})  {}x{}  fits {}",
            image.width(),
            image.height(),
            match ranked[0].0 >= facing::PRISM_FITS {
                true => "yes",
                false => "NO — the table holds no prism for this picture",
            },
        );
        println!(
            "  margin over the best candidate that climbs another way: {:+.4}",
            ranked[0].0 - rival.0
        );
        for (score, prism) in ranked.iter().take(6) {
            println!(
                "   {score:.4}  {:<9} {:?}",
                match axis(prism) {
                    Some(face) => format!("{face:?}"),
                    None => "box".to_string(),
                },
                prism.treads(),
            );
        }
    }
}
