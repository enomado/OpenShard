//! Scratch: what a point of a real house actually receives, and what stopped
//! the ray that did not reach it.
//!
//! The instrument for "the floor is dark but the wall above it is not": a column
//! of spots up one tile, each asked as the surface it is — a face, a flat — with
//! the flame that reached it and the cell that stopped the ones that did not.

use openshard_client_render::atlas::StaticAtlas;
use openshard_client_render::camera::Camera;
use openshard_client_render::cutaway::Cutaway;
use openshard_client_render::facing::Face;
use openshard_client_render::geometry::Vec2;
use openshard_client_render::light::{self, Spot};
use openshard_protocol::wire::Graphic;
use openshard_protocol::world::Point;
use openshard_uofiles::art::Art;
use openshard_uofiles::map::Map;
use openshard_uofiles::tiledata::TileData;

fn main() {
    let dir = std::path::PathBuf::from(std::env::var_os("OPENSHARD_CLIENT").expect("client"));
    let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata");
    let map = Map::load_facet(&dir, 0).expect("felucca");
    let art = Art::open(&dir).expect("art");

    let args: Vec<String> = std::env::args().skip(1).collect();
    let x: u16 = args[0].parse().expect("x");
    let y: u16 = args[1].parse().expect("y");

    let camera = Camera::new(Point::new(x, y, 0), 1200, 900);
    // The pictures the client would have packed for this frame: every static the
    // camera can see. Without them every occluder is a whole tile, which is a
    // different branch of the walk from the one a real client takes.
    let mut wanted = Vec::new();
    for tile_x in x - 12..=x + 12 {
        for tile_y in y - 12..=y + 12 {
            for item in map.statics_at(tile_x, tile_y) {
                wanted.push(Graphic(item.tile));
            }
        }
    }
    let atlas = StaticAtlas::build(&art, wanted).expect("atlas");

    let lighting = light::collect(
        &map,
        &[],
        &camera,
        &tiledata,
        &Cutaway::OPEN,
        light::NIGHT,
        0.0,
        Some(&atlas),
    );
    println!("{} flames in the frame", lighting.lights.len());
    for (index, flame) in lighting.lights.iter().enumerate() {
        println!("  [{index}] at {:?} z {}", flame.at, flame.z);
    }

    let at = Vec2::new(f32::from(x) + 0.5, f32::from(y) + 0.5);
    for z in [20.0, 30.0, 38.0, 40.0, 42.0, 45.0, 50.0, 55.0] {
        for (name, spot) in [
            ("flat ", Spot::flat(at, z)),
            ("south", Spot::face(at, z, Face::South)),
            ("east ", Spot::face(at, z, Face::East)),
        ] {
            let sample = light::sample(spot, &lighting);
            let reached: Vec<String> = sample
                .reaches
                .iter()
                .filter(|reach| reach.within)
                .map(|reach| {
                    format!(
                        "[{}] through {:.2} cone {:.2}{}",
                        reach.light,
                        reach.through,
                        reach.cone,
                        match reach.stopped_by {
                            Some(cell) => format!(" stopped by {cell:?}"),
                            None => String::new(),
                        }
                    )
                })
                .collect();
            println!(
                "z {z:>5}  {name}  brightness {:.4}   {}",
                sample.brightness(),
                reached.join("  "),
            );
        }
    }
}
