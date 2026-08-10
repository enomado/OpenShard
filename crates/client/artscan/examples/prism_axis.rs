//! Scratch: **how sure the prism search is about which way a picture climbs,
//! and how far its own steps sit from the joints the art actually draws.**
//!
//! Two questions grew out of one staircase in Britain — see
//! `docs/lighting_pitfalls.md` and `docs/lighting_state.md`'s 🚩 entry "A fit is
//! scored on its outline, and the surfaces are inside it":
//!
//! - **The axis.** `facing::best_prism` returns one prism and one score, and a
//!   score alone cannot say whether the winner won by a landslide or a coin
//!   flip. The `report` mode below counts both, over every picture the wall
//!   detector reads as a corner (the population `facing::best_prism` is ever
//!   run against — see `occlusion::Shape::of`'s own comment).
//! - **The interior.** `facing::silhouettes_agree` compares two *filled*
//!   silhouettes, so a riser standing a tile's worth of pixels away from the
//!   art's own joint line scores exactly as well as one that lands on it — the
//!   surfaces are inside the outline, where the measure cannot see. `report`
//!   projects every internal step boundary a fitted prism has into the
//!   sprite's own pixel space and measures how far the nearest strong
//!   brightness edge in the art sits from it.
//!
//! ```sh
//! OPENSHARD_CLIENT=… cargo run -p openshard-client-artscan --example prism_axis
//! OPENSHARD_CLIENT=… cargo run -p openshard-client-artscan --example prism_axis -- 1873 1874
//! OPENSHARD_CLIENT=… cargo run -p openshard-client-artscan --example prism_axis -- --debug 1873 1874
//! ```
//!
//! No arguments runs the population report. One or more graphic ids (decimal or
//! `0x`-prefixed hex) runs the original per-picture ranking, unchanged. `--debug`
//! followed by ids writes a marked picture per graphic instead — the art, scaled
//! up, with each internal boundary's own row in red and the strongest brightness
//! edge nearby in lime — under `OPENSHARD_ART_OUT` (`target/art` by default).
//! **The report's own doc says why this exists**: a brightness-edge detector is
//! noisy on stone, and the residual distribution below is not to be trusted
//! until a handful of its worst offenders have been looked at with this.
//!
//! The candidate sweep is `facing::candidates`' own, restated here because that
//! function is private and this is a scratch tool rather than a second reader of
//! the search: boxes to `MAX_PRISM`, then an even climb per (face, treads, top).
//! `MAX_PRISM` is private too and is the one number copied — a disagreement
//! shows up as a candidate this tool ranks and the search never saw, which is
//! visible in the printed count. `HALF_TILE_WIDTH` and `Z_STEP` are the same
//! copy for the same reason: [`facing::silhouette`] and [`facing::prism_silhouette`]
//! are built from them and this tool has to project into the identical space to
//! compare against what they draw.

use std::path::{Path, PathBuf};

use openshard_client_render::facing::{self, Face, Facing, Prism};
use openshard_protocol::wire::Graphic;
use openshard_uofiles::art::Art;
use openshard_uofiles::image::Image;
use openshard_uofiles::tiledata::TileData;

/// `facing::MAX_PRISM`, which is private. See the module doc.
const MAX_PRISM: u8 = 20;

/// `facing`'s own `HALF_TILE_WIDTH`, private there for the same reason
/// `MAX_PRISM` is: a tile's diamond is 44 pixels wide, so a half is 22.
const HALF_TILE_WIDTH: f32 = 22.0;

/// The full width, `2 * HALF_TILE_WIDTH` — [`facing::silhouettes_agree`]'s own
/// tile column count, and every projection here lands in the same 44 columns.
const WIDTH: u16 = 44;

/// `facing`'s own `Z_STEP`, private there: view pixels per unit of `z`.
const Z_STEP: f32 = 4.0;

/// How small the margin over the best rival axis has to be before a fit counts
/// as a coin flip between climb directions.
///
/// Ten thousandths — an order of magnitude under the smallest *confident*
/// margin the handoff measured on a real flight (`0x0751`'s `+0.0775`) and two
/// above the one picture that showed the signature (`0x0756`'s `+0.0024`). The
/// open question this exists to settle: whether "almost a tie between axes" is
/// its own shape class rather than noise. See `docs/lighting_state.md`'s 🚩
/// entry.
const TIE_MARGIN: f32 = 0.01;

/// How many rows either side of a model boundary's own row this searches for
/// the art's nearest brightness edge.
///
/// Generous enough to find a joint the model mis-locates by a whole tread's
/// worth of `z` (the handoff's own flight was off by `10.5 - 2.5 = 8` view px)
/// without wandering far enough to lock onto a neighbouring step instead.
const SEARCH: i32 = 16;

/// The smallest brightness step, summed over the widened 0..=765 range of the
/// three channels, that counts as an edge at all rather than texture noise.
///
/// Not measured against a picture — chosen, the way `report`'s own doc insists
/// on checking: it wants to clear the graininess a stone stair's own shading has
/// between adjacent pixels while still catching a real material change. That is
/// exactly the judgement the `--debug` picture exists to verify before this
/// number, or the distribution it produces, is trusted.
const STRONG_EDGE: i32 = 60;

fn main() {
    let dir = PathBuf::from(std::env::var_os("OPENSHARD_CLIENT").expect("client"));
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.first().map(String::as_str) == Some("--debug") {
        let ids: Vec<u16> = args[1..].iter().map(|arg| parse_id(arg)).collect();
        let out = PathBuf::from(std::env::var_os("OPENSHARD_ART_OUT").unwrap_or_else(|| "target/art".into()));
        debug_pictures(&dir, &out, &ids);
        return;
    }

    if args.is_empty() {
        report(&dir);
        return;
    }

    let art = Art::open(&dir).expect("art");
    let sweep = sweep();
    println!("{} candidates in the sweep", sweep.len());

    for arg in args {
        let id = parse_id(&arg);
        let Ok(Some(image)) = art.static_art(Graphic(id)) else {
            println!("{id}: no art");
            continue;
        };
        let ranked = rank(&sweep, &image);
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
            margin(&ranked),
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

/// A graphic id off the command line, decimal or `0x`-prefixed hex.
fn parse_id(arg: &str) -> u16 {
    match arg.strip_prefix("0x").or_else(|| arg.strip_prefix("0X")) {
        Some(hex) => u16::from_str_radix(hex, 16).expect("graphic in hex"),
        None => arg.parse().expect("graphic"),
    }
}

/// Every prism the search considers, in the order `facing::candidates` does —
/// restated because that function is private. See the module doc.
fn sweep() -> Vec<(Prism, Image)> {
    let mut sweep: Vec<Prism> = (0..=MAX_PRISM).map(Prism::box_of).collect();
    for up in [Face::North, Face::East, Face::South, Face::West] {
        for treads in 2..=facing::MAX_TREADS {
            for top in 1..=u16::from(MAX_PRISM) {
                let profile: Vec<u8> = (1..=treads).map(|i| (top * i / treads) as u8).collect();
                sweep.push(Prism::new(up, &profile).expect("a legal profile by construction"));
            }
        }
    }
    sweep
        .into_iter()
        .map(|prism| {
            let silhouette = facing::prism_silhouette(&prism);
            (prism, silhouette)
        })
        .collect()
}

/// Which way a prism climbs, or `None` for a box — [`Prism::up`] answers for a
/// one-tread prism the way it happened to be constructed, and nothing about the
/// shape depends on it, so a box is its own kind rather than a fifth direction.
fn axis(prism: &Prism) -> Option<Face> {
    match prism.treads().len() {
        1 => None,
        _ => Some(prism.up()),
    }
}

/// Every candidate scored against `image`, best first.
fn rank<'a>(sweep: &'a [(Prism, Image)], image: &Image) -> Vec<(f32, &'a Prism)> {
    let mut ranked: Vec<(f32, &Prism)> = sweep
        .iter()
        .map(|(prism, silhouette)| (facing::silhouettes_agree(image, silhouette), prism))
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    ranked
}

/// How far the winner's score sits above the best candidate that climbs a
/// *different* axis — the number the open question is about.
///
/// # Panics
///
/// If `ranked` holds no rival axis to the winner, which cannot happen for
/// [`sweep`]'s own output: it always carries boxes and all four faces.
fn margin(ranked: &[(f32, &Prism)]) -> f32 {
    let winner = ranked[0].1;
    let rival = ranked
        .iter()
        .find(|(_, prism)| axis(prism) != axis(winner))
        .expect("a sweep with boxes and four faces in it always has a rival");
    ranked[0].0 - rival.0
}

/// Where a point at height `z`, run `run` along `up`'s climb and `perp` of the
/// way across the tile on the other axis, projects — [`facing::drawn_at`]'s own
/// space, restated because that function is private: `column` is the tile
/// column [`facing::silhouettes_agree`] compares, and `row_from_bottom` counts
/// up from the picture's own last row the way a real sprite and a drawn
/// silhouette are both anchored.
///
/// The inverse of the `run` match in [`facing::prism_silhouette`]: that
/// function turns a sample `(u, v)` into a run along `up`'s climb; this turns a
/// chosen run back into the `(u, v)` the same projection would place it at.
fn project(up: Face, run: f32, perp: f32, z: f32) -> (f32, f32) {
    let (u, v) = match up {
        Face::East => (run, perp),
        Face::West => (1.0 - run, perp),
        Face::South => (perp, run),
        Face::North => (perp, 1.0 - run),
    };
    let column = (u - v) * HALF_TILE_WIDTH + f32::from(WIDTH) / 2.0;
    let row_from_bottom = HALF_TILE_WIDTH - (u + v - 1.0) * HALF_TILE_WIDTH + z * Z_STEP;
    (column, row_from_bottom)
}

/// One internal step boundary's own crest — the plane at `run`, height `z` —
/// swept across the tile and bucketed into the 44 columns [`project`] can land
/// in, averaging where several samples share a column.
///
/// 128 samples of the perpendicular axis, the same density
/// [`facing::prism_silhouette`] sweeps both axes at, which is fine enough that
/// no column of a 44-wide picture is skipped.
fn boundary_columns(up: Face, run: f32, z: u8) -> Vec<(u16, f32)> {
    const SAMPLES: i32 = 128;
    let mut buckets: Vec<Vec<f32>> = vec![Vec::new(); usize::from(WIDTH)];
    for i in 0..=SAMPLES {
        let perp = i as f32 / SAMPLES as f32;
        let (column, row) = project(up, run, perp, f32::from(z));
        let bucket = column.floor();
        if bucket < 0.0 || bucket >= f32::from(WIDTH) {
            continue;
        }
        buckets[bucket as usize].push(row);
    }
    buckets
        .into_iter()
        .enumerate()
        .filter_map(|(column, rows)| match rows.is_empty() {
            true => None,
            false => Some((column as u16, rows.iter().sum::<f32>() / rows.len() as f32)),
        })
        .collect()
}

/// The luma at one pixel of `image`, or `None` off its edge — the sum of the
/// widened 8-bit channels, `0..=765`, which is all a vertical-gradient search
/// needs and is cheaper than a weighted one.
fn luma(image: &Image, x: i32, y: i32) -> Option<i32> {
    if x < 0 || y < 0 {
        return None;
    }
    let pixel = image.pixel(u16::try_from(x).ok()?, u16::try_from(y).ok()?)?;
    let (r, g, b) = pixel.rgb8();
    Some(i32::from(r) + i32::from(g) + i32::from(b))
}

/// The row nearest `near` in column `x` where the art's own brightness takes
/// its sharpest step, and how big that step is — `None` off the picture.
///
/// Searched rather than read off a fixed edge list because the art has none:
/// this is a plain vertical Sobel-style difference between neighbouring rows,
/// maximised over [`SEARCH`] rows either side of where the model says the joint
/// ought to be.
fn strongest_edge(image: &Image, x: i32, near: i32) -> Option<(i32, i32)> {
    if x < 0 || x >= i32::from(image.width()) {
        return None;
    }
    let mut best: Option<(i32, i32)> = None;
    for y in (near - SEARCH)..=(near + SEARCH) {
        let (Some(a), Some(b)) = (luma(image, x, y), luma(image, x, y + 1)) else {
            continue;
        };
        let strength = (a - b).abs();
        if best.is_none_or(|(_, s)| strength > s) {
            best = Some((y, strength));
        }
    }
    best
}

/// Every column of one internal boundary's residual against the art, in view
/// pixels — empty where the picture has nothing there or nothing strong enough
/// to count as an edge (see [`STRONG_EDGE`]).
fn edge_residuals(image: &Image, up: Face, run: f32, crest: u8) -> Vec<f32> {
    let offset = (i32::from(WIDTH) - i32::from(image.width())) / 2;
    let mut out = Vec::new();
    for (column, model_row) in boundary_columns(up, run, crest) {
        let x = i32::from(column) - offset;
        let model_y = i32::from(image.height()) - 1 - model_row.round() as i32;
        if let Some((y, strength)) = strongest_edge(image, x, model_y) {
            if strength >= STRONG_EDGE {
                out.push((y - model_y).abs() as f32);
            }
        }
    }
    out
}

/// A fitted prism's mean residual across every internal boundary it has, and
/// how many columns that mean was taken over — `None` if not one column found a
/// confident edge anywhere.
///
/// A box has no internal boundary and is never offered here — see
/// [`report`]'s own gate.
fn measure_residual(prism: &Prism, image: &Image) -> Option<(f32, usize)> {
    let treads = prism.treads();
    let mut all = Vec::new();
    for index in 1..treads.len() {
        let run = index as f32 / treads.len() as f32;
        all.extend(edge_residuals(image, prism.up(), run, treads[index]));
    }
    match all.is_empty() {
        true => None,
        false => Some((all.iter().sum::<f32>() / all.len() as f32, all.len())),
    }
}

/// Step 1 of the handoff's plan: every picture the wall detector reads as a
/// corner — the population [`facing::best_prism`] is ever run against, per
/// `occlusion::Shape::of`'s own gate — scored, tied off against its rival axis,
/// and, where it fits, measured against the art at every internal step.
fn report(dir: &Path) {
    let art = Art::open(dir).expect("art");
    let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");
    let sweep = sweep();

    let (mut accepted, mut rejected) = (0usize, 0usize);
    let (mut accepted_ties, mut rejected_ties) = (0usize, 0usize);
    let mut hit_cap = 0usize;
    let mut multi_tread = 0usize;

    struct Row {
        graphic: u16,
        name: String,
        treads: usize,
        mean_px: f32,
        columns: usize,
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut undetected: Vec<(u16, String)> = Vec::new();

    for id in 0..=u16::MAX {
        let Ok(Some(image)) = art.static_art(Graphic(id)) else {
            continue;
        };
        // Only what the wall detector calls a corner is ever offered to the
        // prism search — see `occlusion::Shape::of`'s own comment — so this is
        // the population the report's whole errand is about, not a filter of
        // convenience.
        let Some(Facing::Corner { .. }) = facing::facing_of(&image) else {
            continue;
        };

        let ranked = rank(&sweep, &image);
        let (score, winner) = (ranked[0].0, *ranked[0].1);
        let is_fit = score >= facing::PRISM_FITS;
        match is_fit {
            true => accepted += 1,
            false => rejected += 1,
        }
        if margin(&ranked) < TIE_MARGIN {
            match is_fit {
                true => accepted_ties += 1,
                false => rejected_ties += 1,
            }
        }
        if !is_fit {
            continue;
        }
        if winner.treads().len() == usize::from(facing::MAX_TREADS) {
            hit_cap += 1;
        }
        // A box has no internal boundary — nothing for the interior term to
        // measure — so it counts towards the axis tally above and stops here.
        if winner.treads().len() < 2 {
            continue;
        }
        multi_tread += 1;
        let name = tiledata.static_tile(id).name.clone();
        match measure_residual(&winner, &image) {
            Some((mean_px, columns)) => rows.push(Row {
                graphic: id,
                name,
                treads: winner.treads().len(),
                mean_px,
                columns,
            }),
            None => undetected.push((id, name)),
        }
    }

    rows.sort_by(|a, b| b.mean_px.total_cmp(&a.mean_px));

    println!("{} pictures read as a corner", accepted + rejected);
    println!(
        "  {accepted:>5} fit a prism (score >= {:.2}) — 'the table holds a prism for them'",
        facing::PRISM_FITS,
    );
    println!("  {rejected:>5} did not");

    println!("\ncoin-flip axis margin (< {TIE_MARGIN:.3}), see docs/lighting_state.md's 🚩 entry:");
    println!("  {accepted_ties:>5} of the {accepted} fitted");
    println!("  {rejected_ties:>5} of the {rejected} refused");

    println!(
        "\n{hit_cap:>5} of {multi_tread} multi-tread prisms use every one of facing::MAX_TREADS = {} treads",
        facing::MAX_TREADS,
    );

    println!("\ninternal-boundary residual — model crest vs. nearest strong art edge, view px:");
    println!(
        "  {:>5} multi-tread prisms measured\n  {:>5} found no confident edge anywhere on their own steps",
        rows.len(),
        undetected.len(),
    );
    if !rows.is_empty() {
        let mut values: Vec<f32> = rows.iter().map(|row| row.mean_px).collect();
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        values.sort_by(f32::total_cmp);
        let percentile = |p: f32| values[((values.len() - 1) as f32 * p).round() as usize];
        println!(
            "  mean {mean:.2}  median {:.2}  p90 {:.2}  max {:.2}",
            percentile(0.5),
            percentile(0.9),
            percentile(1.0),
        );
        println!("\n  worst by mean residual:");
        for row in rows.iter().take(15) {
            println!(
                "   {:>6.2} px  0x{:04X}  {} treads  {:>3} cols  {}",
                row.mean_px, row.graphic, row.treads, row.columns, row.name,
            );
        }
    }
    if !undetected.is_empty() {
        println!("\n  no confident edge at all:");
        for (id, name) in undetected.iter().take(15) {
            println!("   0x{id:04X}  {name}");
        }
    }
}

/// Scale a marked picture is drawn at — `tests/artshot.rs`'s own, so a person
/// comparing the two tools is comparing the same picture at the same size.
const SCALE: u32 = 6;

/// One picture no real 16-bit art pixel can be, so transparency and the marked
/// background are never confused with a colour the artist drew — the same
/// background `tests/artshot.rs` uses.
const CANVAS: [u8; 3] = [64, 0, 96];

/// `--debug`: one marked picture per graphic, art scaled up with each internal
/// boundary's own row in red and the nearest strong brightness edge found for
/// it in lime — the eyeball check [`report`]'s own doc says has to happen
/// before its distribution is believed.
fn debug_pictures(dir: &Path, out_dir: &Path, ids: &[u16]) {
    let art = Art::open(dir).expect("art");
    let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");
    let sweep = sweep();
    std::fs::create_dir_all(out_dir).expect("the output directory");

    for &id in ids {
        let Ok(Some(image)) = art.static_art(Graphic(id)) else {
            println!("0x{id:04X}: no art");
            continue;
        };
        let ranked = rank(&sweep, &image);
        let (score, prism) = (ranked[0].0, *ranked[0].1);
        println!(
            "0x{id:04X} {}  score {score:.4}  margin {:+.4}  {:?} {:?}",
            tiledata.static_tile(id).name,
            margin(&ranked),
            prism.up(),
            prism.treads(),
        );
        if prism.treads().len() < 2 {
            println!("  a box: no internal boundary to mark");
            continue;
        }

        let (w, h) = (u32::from(image.width()), u32::from(image.height()));
        let (sw, sh) = (w * SCALE, h * SCALE);
        let mut rgb = vec![CANVAS; (sw * sh) as usize];
        for y in 0..h {
            for x in 0..w {
                let Some(pixel) = image.pixel(x as u16, y as u16) else {
                    continue;
                };
                if pixel.is_transparent() {
                    continue;
                }
                let (r, g, b) = pixel.rgb8();
                stroke(&mut rgb, sw, sh, x as i32, y as i32, [r, g, b]);
            }
        }

        let offset = (i32::from(WIDTH) - i32::from(image.width())) / 2;
        let treads = prism.treads();
        let mut columns_marked = 0usize;
        let mut columns_confident = 0usize;
        for (index, &crest) in treads.iter().enumerate().skip(1) {
            let run = index as f32 / treads.len() as f32;
            for (column, model_row) in boundary_columns(prism.up(), run, crest) {
                let x = i32::from(column) - offset;
                let model_y = i32::from(image.height()) - 1 - model_row.round() as i32;
                stroke(&mut rgb, sw, sh, x, model_y, [255, 64, 64]);
                columns_marked += 1;
                if let Some((edge_y, strength)) = strongest_edge(&image, x, model_y) {
                    if strength >= STRONG_EDGE {
                        stroke(&mut rgb, sw, sh, x, edge_y, [64, 255, 64]);
                        columns_confident += 1;
                    }
                }
            }
        }
        println!("  {columns_confident} of {columns_marked} columns found a confident edge");

        let bytes: Vec<u8> = rgb.iter().flat_map(|p| p.iter().copied()).collect();
        let path = out_dir.join(format!("0x{id:04X}_interior.png"));
        openshard_client_render::png::write(&path, sw, sh, &bytes).expect("the picture writes");
        println!("  -> {}", path.display());
    }
}

/// One art pixel of overlay, at art coordinates, in the scaled image —
/// `tests/artshot.rs`'s own `stroke`, restated because that one is not `pub`.
fn stroke(rgb: &mut [[u8; 3]], sw: u32, sh: u32, x: i32, y: i32, colour: [u8; 3]) {
    if x < 0 || y < 0 {
        return;
    }
    let (x, y) = (x as u32 * SCALE, y as u32 * SCALE);
    for dy in 0..SCALE {
        for dx in 0..SCALE {
            let (px, py) = (x + dx, y + dy);
            if px < sw && py < sh {
                rgb[(py * sw + px) as usize] = colour;
            }
        }
    }
}
