//! What the lighting knows about a named place in the world the client ships.
//!
//! The scenes in `lighting.rs` are built, which is what makes them readable and
//! is also what they cannot answer: a report from a player is a *coordinate*, and
//! between that coordinate and a built scene there is a chain of guesses — which
//! graphics stand there, which of them the art names a face for, what the
//! occlusion grid ends up holding. Every one of those is somewhere a synthetic
//! reproduction can quietly differ from the thing complained about, and a scene
//! built from a wrong guess is a green test beside a defect that is still there.
//!
//! So this file is the instrument in between: point it at a tile, and it prints
//! what stands there, what the atlas read off each picture, what the grid ended
//! up with, and what a ray from a given flame does on the way. It asserts almost
//! nothing — it is the eye, in the same sense `frame.rs`'s frame dump is — and
//! what it is *for* is turning a coordinate into the handful of facts a built
//! scene has to reproduce.
//!
//! Ignored and gated on `OPENSHARD_CLIENT`: no client files live in this
//! repository. Run it with one:
//!
//! ```sh
//! OPENSHARD_CLIENT=… cargo test -p openshard-client-render --test onsite -- \
//!     --ignored --nocapture
//! ```
//!
//! `OPENSHARD_AT=x,y` moves it; the default is the corner of the house a lamp
//! was reported leaking round — see `docs/lighting.md`, and the backlog entry
//! "found at a house corner in Britain".

use std::path::PathBuf;

use openshard_client_render::animate::StaticAnimations;
use openshard_client_render::atlas::StaticAtlas;
use openshard_client_render::camera::{Camera, TileBounds};
use openshard_client_render::cutaway::Cutaway;
use openshard_client_render::geometry::Vec2;
use openshard_client_render::items::GroundItem;
use openshard_client_render::light::{self, Spot};
use openshard_client_render::occlusion;
use openshard_client_render::place::Stance;
use openshard_client_render::statics;
use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::Point;
use openshard_uofiles::art::Art;
use openshard_uofiles::map::Map;
use openshard_uofiles::tiledata::TileData;

/// The place the reports came from: the lamp is on `(1441, 1693)` and the house
/// corner it stands against is the tile north of it.
const REPORTED: (u16, u16) = (1441, 1692);

/// And where the flame stands: the tile south of it, in the street.
///
/// Put there by this test rather than found on the map — nothing burns within
/// forty tiles of that corner in Felucca, and what the report was about is a
/// light *a player was carrying*. A ground item is the same flame to everything
/// downstream, and it means the ray this prints is the one complained about
/// rather than one from whichever lamp happened to be nearest.
const LAMP_AT: (u16, u16) = (1441, 1693);

/// What that flame is: a lantern, which the client's own tiledata flags
/// `LIGHT_SOURCE` and gives no `NO_SHOOT` — so it burns rather than being one of
/// decision 19's windows. Printed below, because a graphic that stopped burning
/// in a later client would otherwise turn this whole probe blank.
const LAMP: Graphic = Graphic(0x0A15);

/// How many tiles either side of the place are printed.
const AROUND: i32 = 3;

/// The client's files, or `None` when the environment does not point at any.
fn client_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("OPENSHARD_CLIENT")?))
}

/// The tile to look at: `OPENSHARD_AT=x,y`, or [`REPORTED`].
fn place() -> (u16, u16) {
    let Some(text) = std::env::var_os("OPENSHARD_AT") else {
        return REPORTED;
    };
    let text = text.to_string_lossy().to_string();
    let (x, y) = text.split_once(',').expect("OPENSHARD_AT is x,y");
    (x.trim().parse().expect("an x"), y.trim().parse().expect("a y"))
}

/// Everything the lighting reads about a place, printed.
///
/// Four blocks, and the order is the order a leak is diagnosed in:
///
/// 1. **What stands there** — every static on every tile of the square, with the
///    height and flags `occlusion` reads and the face the atlas measured off the
///    picture. A graphic whose face is `None` is a whole-tile occluder and an
///    `Upright` sprite, which is the answer everything falls back to and the
///    answer two of the reported defects turned out to be made of.
/// 2. **The grid** — one line a tile: the span, the opacity, and the edge mask.
/// 3. **The flames** — what `light::collect` found and where it put them.
/// 4. **A sweep of rays** — the ground around the place, sampled at a third of a
///    tile, printed as how much of the nearest flame reaches it. A leak through a
///    corner is a stripe in that picture and is invisible in a per-tile diagram,
///    which samples one point a tile and would step straight over it.
#[test]
#[ignore = "reads a real install and prints for a person; asserts almost nothing"]
fn what_the_lighting_knows_about_a_place() {
    let Some(dir) = client_dir() else {
        return;
    };
    let (at_x, at_y) = place();
    let map = Map::load_facet(&dir, 0).expect("Felucca");
    let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");
    let art = Art::open(&dir).expect("artLegacyMUL.uop");

    // The camera is only here to give `light::collect` the same bounds the client
    // would have: a place is looked at from where a player stands on it.
    let centre = Point::new(at_x, at_y, map.average_land_z(at_x, at_y).unwrap_or(0));
    let camera = Camera::new(centre, 768, 512);
    let atlas = StaticAtlas::build(
        &art,
        statics::visible_graphics(&map, &camera, &StaticAnimations::default()),
    )
    .expect("the statics of one screen fit");

    println!("\n=== what stands at ({at_x}, {at_y}) ± {AROUND} ===");
    for y in at_y as i32 - AROUND..=at_y as i32 + AROUND {
        for x in at_x as i32 - AROUND..=at_x as i32 + AROUND {
            let (x, y) = (x as u16, y as u16);
            for item in map.statics_at(x, y) {
                let graphic = Graphic(item.tile);
                let tile = tiledata.static_tile(item.tile);
                let face = atlas.sprite(graphic).and_then(|sprite| sprite.face);
                println!(
                    "({x}, {y}) z {:>4}  {graphic:?}  h {:>3}  face {:?}  stance {:?}  \
                     opacity {:>3}  burns {}",
                    item.z,
                    tile.height,
                    face,
                    Stance::of(tile, face),
                    occlusion::opacity(graphic, tile),
                    light::burns(graphic, tile),
                );
            }
        }
    }

    // The flame the report is about, standing in the street where the player was.
    let lamp = [GroundItem {
        at: Point::new(LAMP_AT.0, LAMP_AT.1, 0),
        graphic: LAMP,
        hue: Hue::NONE,
    }];
    println!(
        "\nthe lamp is {LAMP:?} at {LAMP_AT:?}, and it burns: {}",
        light::burns(LAMP, tiledata.static_tile(LAMP.0)),
    );

    let bounds = TileBounds {
        min_x: i32::from(at_x) - AROUND - 8,
        max_x: i32::from(at_x) + AROUND + 8,
        min_y: i32::from(at_y) - AROUND - 8,
        max_y: i32::from(at_y) + AROUND + 8,
    };
    let grid = occlusion::collect(&map, &lamp, bounds, &tiledata, &Cutaway::OPEN, Some(&atlas));
    println!("\n=== the grid ===");
    for y in at_y as i32 - AROUND..=at_y as i32 + AROUND {
        for x in at_x as i32 - AROUND..=at_x as i32 + AROUND {
            let Some(cell) = grid.at(x, y) else {
                continue;
            };
            println!(
                "({x}, {y})  z {}..={}  opacity {:>3}  edges {}",
                cell.bottom,
                cell.top,
                cell.opacity,
                edges(cell.edges),
            );
        }
    }

    let lighting = light::collect(
        &map,
        &lamp,
        &camera,
        &tiledata,
        &Cutaway::OPEN,
        light::NIGHT,
        0.0,
        Some(&atlas),
    );
    println!("\n=== the flames, nearest first ===");
    let mut flames: Vec<_> = lighting
        .lights
        .iter()
        .map(|flame| {
            let (dx, dy) = (flame.at.x - f32::from(at_x), flame.at.y - f32::from(at_y));
            ((dx * dx + dy * dy).sqrt(), flame)
        })
        .collect();
    flames.sort_by(|a, b| a.0.total_cmp(&b.0));
    println!("{} in the frame", flames.len());
    for (away, flame) in flames.iter().take(8) {
        println!(
            "({}, {}) z {}  radius {}  {away:.1} tiles away",
            flame.at.x, flame.at.y, flame.z, flame.radius,
        );
    }

    // A third of a tile, because a leak through a corner is thinner than a tile
    // and a per-tile diagram walks straight over it. `#` is a ray that is stopped,
    // a space is one that arrives whole, and the digits in between are tenths.
    println!("\n=== how much of the nearest flame reaches the ground ===");
    let step = 1.0 / 3.0;
    let ramp = |through: f32| match through {
        t if t <= 0.0 => '#',
        t if t >= 1.0 => ' ',
        t => char::from_digit((t * 10.0) as u32, 10).unwrap_or('?'),
    };
    for row in -AROUND * 3..=AROUND * 3 {
        let y = f32::from(at_y) + 0.5 + row as f32 * step;
        let line: String = (-AROUND * 3..=AROUND * 3)
            .map(|column| {
                let x = f32::from(at_x) + 0.5 + column as f32 * step;
                let spot = Spot::at(Vec2::new(x, y), 0.0);
                let sample = light::sample(spot, &lighting);
                // The nearest flame that reaches this far at all. `reaches` holds
                // one entry per flame in the frame, most of them out of range, so
                // the first is whichever the map listed first and says nothing.
                let nearest = sample
                    .reaches
                    .iter()
                    .filter(|reach| reach.within)
                    .min_by(|a, b| a.distance.total_cmp(&b.distance));
                match nearest {
                    None => '.',
                    Some(reach) => ramp(reach.through),
                }
            })
            .collect();
        println!("{line}");
    }
}

/// An edge mask as the four letters, for a line a person reads.
fn edges(mask: u8) -> String {
    let named = [
        (occlusion::EDGE_NORTH, 'N'),
        (occlusion::EDGE_EAST, 'E'),
        (occlusion::EDGE_SOUTH, 'S'),
        (occlusion::EDGE_WEST, 'W'),
    ];
    let text: String = named
        .iter()
        .map(|(bit, letter)| match mask & bit != 0 {
            true => *letter,
            false => '-',
        })
        .collect();
    match mask {
        0 => format!("{text} (a lid)"),
        occlusion::EDGE_ANY => format!("{text} (a whole tile)"),
        _ => text,
    }
}
