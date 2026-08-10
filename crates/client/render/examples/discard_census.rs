//! How much of each sprite hangs outside the box it is met against — before and
//! after the measured footprint narrowed it.
//!
//! `statics.wesl` discards a fragment whose view ray meets none of its static's
//! boxes, so **every pixel of art that overhangs its own volume is a pixel that
//! leaves the screen**. `docs/lighting_rebuild.md` measured that trade once, on
//! the whole-tile geometry, and stated it beside the `discard` itself: 4460
//! pixels of 187,086 at Britain's `(1501, 1659)`, 2.38%. `docs/footprints.md`'s
//! S4 is the other half of it — a narrower box is a box the art overhangs
//! *more*, and the plan says outright that "a footprint that eats a tabletop's
//! overhang is a finding, not a cost to accept quietly".
//!
//! So this counts the same thing twice over one neighbourhood: once with each
//! static's boxes as `boxes_of` gives them today, and once with
//! [`Shape::footprint`] forced to `None`, which is exactly the whole-tile
//! fallback S3 replaced. Both numbers come out of one run, so the comparison is
//! two boxes measured against one picture rather than two builds measured a
//! session apart.
//!
//! **The ray is the shader's own**, not a second reading of the projection:
//! [`impostor::ray_from`] and [`impostor::meets`] are the CPU side of the two
//! functions `statics.wesl` imports, and `across`/`down` are read off a sprite
//! in the convention [`crate::facing::measure_footprint`] already uses — the
//! sprite's middle column, and the tile's own centre row `HALF_TILE` above its
//! bottom edge (`statics::stand_on`). `examples/speck_probe.rs` asks a
//! neighbouring question of the same pair.
//!
//! **And the shadow, which is the plan's second number.** A narrower occluder
//! casts a narrower shadow — but only if it is an occluder at all, and every
//! graphic this class reaches is expected to be `CLEAR`, which `Builder::add`
//! drops before the grid ever sees it. That expectation is a *count* here
//! rather than a belief: a footprinted placement the grid holds a primitive for
//! is a placement whose shadow moved, and the report names it.
//!
//! Reads the client's own files; no GPU, and no shard database — see
//! `docs/parity.md`'s backlog on that, which this tool inherits from
//! `geometry_census.rs` beside it: what it counts is *the art's* geometry, so a
//! decoration the server placed is outside its answer.
//!
//! ```sh
//! OPENSHARD_CLIENT=… cargo run --release -p openshard-client-render \
//!     --example discard_census -- 1501 1659 60
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use openshard_client_render::atlas::StaticAtlas;
use openshard_client_render::facing::{Block, Blocks, Footprint, blocks_silhouette};
use openshard_client_render::impostor::{self, Volume};
use openshard_client_render::occlusion::{self, Shape};
use openshard_protocol::wire::Graphic;
use openshard_uofiles::art::Art;
use openshard_uofiles::image::Image;
use openshard_uofiles::map::Map;
use openshard_uofiles::tiledata::{StaticTile, TileData};

/// Half a tile in virtual pixels — `statics.wesl`'s `HALF_TILE_HEIGHT` and
/// `facing`'s `HALF_TILE_WIDTH`, which are the same twenty-two: a sprite's
/// bottom edge stands on the diamond's bottom vertex, that far below the row
/// the tile's own centre projects to.
const HALF_TILE: f32 = 22.0;

/// Every box one placement stands as, in the form the fragment shader meets
/// them in.
///
/// [`crate::statics::push_volumes`] is the live route and it does one thing
/// more: where the occlusion grid holds a *named* primitive for a piece, the
/// box a fragment is met against is the grid's own merged solid rather than
/// this per-tile one. That difference cannot reach the class under measurement
/// — a merged primitive exists only for a piece the grid took in, and the
/// report below counts how many of those there are precisely so that this
/// sentence stays true rather than being assumed.
fn volumes_of(x: i32, y: i32, z: i8, tile: &StaticTile, shape: &Shape) -> Vec<Volume> {
    let mut out = Vec::new();
    occlusion::boxes_of(x, y, z, tile, shape, |_, _, space| {
        out.push(Volume::of(&space, 0));
    });
    out
}

/// One picture's pixels against one set of boxes: how many are drawn, and how
/// many of those the impostor would discard.
///
/// The walk is the whole sprite and not the 44-wide tile column
/// `facing::silhouettes_agree` clips to: the pass draws every pixel of the art,
/// so a table top hanging past its own cell is exactly the pixel this is about.
fn overhang(image: &Image, at: (i32, i32), z: i8, volumes: &[Volume]) -> (u32, u32) {
    let (width, height) = (image.width(), image.height());
    let middle = f32::from(width) / 2.0;
    let centre_row = f32::from(height) - HALF_TILE;
    let (mut drawn, mut missed) = (0u32, 0u32);
    for row in 0..height {
        for column in 0..width {
            let opaque = image
                .pixel(column, row)
                .is_some_and(|pixel| !pixel.is_transparent());
            if !opaque {
                continue;
            }
            drawn += 1;
            // The two numbers `statics.wesl`'s vertex stage hands its fragment
            // stage, at this pixel's own centre.
            let across = f32::from(column) + 0.5 - middle;
            let down = f32::from(row) + 0.5 - centre_row;
            let start = impostor::ray_from(at, f32::from(z), across, down);
            let met = volumes
                .iter()
                .any(|volume| impostor::meets(start, volume.lo, volume.hi).hit());
            missed += u32::from(!met);
        }
    }
    (drawn, missed)
}

/// What this instrument reads off a picture whose box is **known exactly**,
/// which is the floor every number below stands on.
///
/// A share of discarded pixels is only worth reading if the walk agrees with
/// the projection in the first place: an `across` off by a sign or a `down` off
/// by a row would put the hit region somewhere else entirely and report a large,
/// confident, meaningless share. So the same walk is run over
/// [`blocks_silhouette`]'s drawing of a whole-tile block against that block's
/// own box — the reference `docs/footprints.md`'s D6 already round-trips the
/// measurement through — and against the same box moved a hundred tiles away.
///
/// The first is the positive control and its answer is the instrument's own
/// noise floor. The second is the negative one, and it must be everything: a
/// walk that reported no misses there would be one that is not reading the
/// boxes at all.
///
/// **Run at two heights, because a floor that grows with the box is not a
/// floor.** A constant miss says the two disagree about one edge — a rounding
/// convention, `blocks_silhouette` painting from `head.round()` — and a
/// proportional one would say they disagree about the projection, which is the
/// error that would make every share below meaningless. Measured: forty-four
/// pixels at either height, which is one row of the tile's own width.
fn controls(at: (i32, i32)) -> Vec<(u8, u32, u32, u32)> {
    [5u8, 10]
        .into_iter()
        .map(|top| {
            let block = Block::new((0, 8), (0, 8), (0, top)).expect("a whole tile with a height");
            let image = blocks_silhouette(&Blocks::new(&[block]).expect("one block"));
            let own = Volume {
                lo: [at.0 as f32, at.1 as f32, 0.0],
                hi: [at.0 as f32 + 1.0, at.1 as f32 + 1.0, f32::from(top)],
                solid: 0,
            };
            let elsewhere = Volume {
                lo: [own.lo[0] + 100.0, own.lo[1] + 100.0, own.lo[2]],
                hi: [own.hi[0] + 100.0, own.hi[1] + 100.0, own.hi[2]],
                solid: 0,
            };
            let (drawn, missed) = overhang(&image, at, 0, std::slice::from_ref(&own));
            let (_, missed_far) = overhang(&image, at, 0, std::slice::from_ref(&elsewhere));
            (top, drawn, missed, missed_far)
        })
        .collect()
}

/// What one graphic's pictures cost, summed over every placement of it.
#[derive(Default)]
struct Tally {
    /// How many times it stands in the window.
    placements: u32,
    /// Its drawn pixels, once per placement.
    drawn: u64,
    /// Of those, the ones discarded against the whole-tile box.
    missed_wide: u64,
    /// And against the box we give it today.
    missed_now: u64,
    /// Whether today's box is a measured footprint — the class S4 is about.
    footprint: Option<Footprint>,
    /// Placements of it the occlusion grid holds a primitive for, which is where
    /// a narrower box would also mean a narrower *shadow*.
    in_the_grid: u32,
    /// The picture's own size in pixels, and how many `z` units tall the box it
    /// is met against is. Two numbers rather than a share, because the first
    /// question a large discard raises is whether the art is simply taller than
    /// the height `tiledata` states for it — `docs/footprints.md`'s D1 measures
    /// the footprint and leaves the height alone, so a picture overhanging its
    /// own *lid* is that carried item showing up in pixels.
    art: (u16, u16),
    /// How tall its box is, in `z` units, from `occlusion::calc_height`.
    height: i32,
    /// Whether the client's own `ROOF` bit is on it — the class a player
    /// standing indoors is not shown at all, and the class the only recorded
    /// measurement of this discard was taken **without**
    /// (`docs/lighting_rebuild.md`: "at Britain's `(1501, 1659)` with the roof
    /// cut"). A roof is a sloped slab given a whole-tile box three `z` units
    /// tall under art seventy-six pixels high, so it overhangs enormously and
    /// for a reason that belongs to that document's phase 6i rather than here.
    roof: bool,
    /// Which kind of claim its box is — `geometry_census.rs`'s own vocabulary,
    /// because a share of discarded pixels that does not say *whose* pixels is
    /// a number nobody can act on.
    claim: &'static str,
}

/// The one claim this plan is about, named once so that the class is defined by
/// **what `boxes_of` did** rather than by what was measured.
///
/// The two are not the same set and reading them as one understates the cost by
/// diluting it: 371 placements in Britain's window carry a measured footprint
/// and only 219 of them are given it, because a climbable takes the prism
/// branch and a `BACKGROUND` piece takes the lid, both before the branch that
/// reads one. The other 152 draw exactly the box they always did.
const NARROWED: &str = "a measured footprint, narrower than the whole tile";

/// What kind of box `boxes_of` gives this picture, in the words
/// `examples/geometry_census.rs` counts them by. Kept in the same order and the
/// same names deliberately: the two tools answer about one class between them.
fn claim_of(tile: &StaticTile, shape: &Shape) -> &'static str {
    if tile.flags.is_climbable() {
        return match shape.prism.is_some() {
            true => "a fitted prism, one body a tread",
            false => "whole tile, a climbable that would not fit",
        };
    }
    if tile.flags.is_background() {
        return "a lid — measured, but a plane with no thickness";
    }
    match (shape.facing.is_some(), shape.footprint.is_some()) {
        (true, _) => "panels on the named edges, PANEL_THICKNESS deep",
        (false, true) => NARROWED,
        (false, false) => "whole tile, the art would not say",
    }
}

fn main() {
    let dir = PathBuf::from(std::env::var_os("OPENSHARD_CLIENT").expect("OPENSHARD_CLIENT"));
    let mut args = std::env::args().skip(1);
    let cx: i32 = args.next().expect("x").parse().expect("x is a number");
    let cy: i32 = args.next().expect("y").parse().expect("y is a number");
    let radius: i32 = args.next().map_or(20, |v| v.parse().expect("radius"));

    let map = Map::load_facet(&dir, 0).expect("Felucca");
    let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");
    let art = Art::open(&dir).expect("artLegacyMUL.uop");

    let mut graphics: Vec<Graphic> = Vec::new();
    for x in cx - radius..=cx + radius {
        for y in cy - radius..=cy + radius {
            for item in map.statics_at(x as u16, y as u16) {
                graphics.push(Graphic(item.tile));
            }
        }
    }
    graphics.sort_unstable_by_key(|g| g.0);
    graphics.dedup();
    let packed = graphics.len();
    // The atlas is what a client would have packed for this window, and reading
    // the shape back out of it is the live route (`occlusion::shape_of`) rather
    // than a second call to `Shape::of` — the same lookup the frame uses.
    let atlas = StaticAtlas::build(&art, graphics.iter().copied()).expect("a quarter fits");

    // One decoded picture a graphic, not one a placement: the art is the same
    // picture wherever it stands.
    let mut images: BTreeMap<u16, Image> = BTreeMap::new();
    for graphic in &graphics {
        if let Ok(Some(image)) = art.static_art(*graphic) {
            images.insert(graphic.0, image);
        }
    }

    let mut tallies: BTreeMap<u16, Tally> = BTreeMap::new();
    let mut placements = 0u32;
    for x in cx - radius..=cx + radius {
        for y in cy - radius..=cy + radius {
            for item in map.statics_at(x as u16, y as u16) {
                let graphic = Graphic(item.tile);
                let Some(image) = images.get(&graphic.0) else {
                    continue;
                };
                let tile = tiledata.static_tile(graphic.0);
                let shape = occlusion::shape_of(Some(&atlas), graphic);
                // The whole-tile fallback S3 replaced, stated by taking the
                // measurement away and changing nothing else: `boxes_of` reads
                // a footprint in exactly one branch, so a `None` here is the
                // box that shipped before it.
                let wide = Shape {
                    footprint: None,
                    ..shape
                };

                let now = volumes_of(x, y, item.z, tile, &shape);
                let (drawn, missed_now) = overhang(image, (x, y), item.z, &now);
                let missed_wide = match shape.footprint {
                    // Nothing to compare: the two shapes are the same shape, and
                    // walking the picture twice for one answer is only slower.
                    None => missed_now,
                    Some(_) => {
                        let boxes = volumes_of(x, y, item.z, tile, &wide);
                        overhang(image, (x, y), item.z, &boxes).1
                    }
                };

                placements += 1;
                let tally = tallies.entry(graphic.0).or_default();
                tally.placements += 1;
                tally.drawn += u64::from(drawn);
                tally.missed_now += u64::from(missed_now);
                tally.missed_wide += u64::from(missed_wide);
                tally.footprint = shape.footprint;
                tally.claim = claim_of(tile, &shape);
                tally.art = (image.width(), image.height());
                tally.roof = tile.flags.is_roof();
                tally.height = now
                    .iter()
                    .map(|volume| (volume.hi[2] - volume.lo[2]) as i32)
                    .max()
                    .unwrap_or(0);
                if occlusion::opacity(graphic, tile) != occlusion::CLEAR {
                    tally.in_the_grid += 1;
                }
            }
        }
    }

    report(&tiledata, &tallies, placements, packed, (cx, cy), radius);
}

/// The three questions in the order S4 asks them: what the discard is now, what
/// the footprint added to it, and whether any of it can move a shadow.
fn report(
    tiledata: &TileData,
    tallies: &BTreeMap<u16, Tally>,
    placements: u32,
    packed: usize,
    at: (i32, i32),
    radius: i32,
) {
    let sum = |pick: fn(&Tally) -> u64| tallies.values().map(pick).sum::<u64>();
    let drawn = sum(|t| t.drawn);
    let missed_now = sum(|t| t.missed_now);
    let missed_wide = sum(|t| t.missed_wide);
    let pct = |n: u64, of: u64| 100.0 * n as f64 / of.max(1) as f64;

    println!(
        "{placements} statics on {}x{} tiles around ({}, {}), {packed} distinct graphics\n",
        radius * 2 + 1,
        radius * 2 + 1,
        at.0,
        at.1,
    );

    // The instrument's own floor and its own ceiling, before anything it
    // measured is read. See `controls`.
    println!("  control  a whole-tile block's own silhouette against its own box:");
    for (top, drew, missed, far) in controls(at) {
        println!(
            "    {top:>2} z units tall   {missed:>4} of {drew:>4} miss ({:>5.2}%)   \
             moved a hundred tiles: {far} of {drew} ({:.2}%)",
            pct(u64::from(missed), u64::from(drew)),
            pct(u64::from(far), u64::from(drew)),
        );
    }
    println!();

    println!("  {drawn:>9}  drawn sprite pixels, every placement counted");
    println!(
        "  {missed_wide:>9}  {:>5.2}%  discarded against the whole-tile box (before S3)",
        pct(missed_wide, drawn),
    );
    println!(
        "  {missed_now:>9}  {:>5.2}%  discarded against the box we give it today",
        pct(missed_now, drawn),
    );
    println!(
        "  {:>9}  {:>+5.2} points  what the measured footprint added\n",
        missed_now as i64 - missed_wide as i64,
        pct(missed_now, drawn) - pct(missed_wide, drawn),
    );

    // **And the same three with the roof cut**, which is the only condition the
    // one recorded measurement of this discard was ever taken under and is what
    // a player standing indoors is looking at. A roof overhangs its own box by
    // half its art, so leaving it in makes every share here a statement about
    // roofs — see [`Tally::roof`].
    let indoors = |pick: fn(&Tally) -> u64| {
        tallies
            .values()
            .filter(|tally| !tally.roof)
            .map(pick)
            .sum::<u64>()
    };
    let (in_drawn, in_wide, in_now) = (
        indoors(|t| t.drawn),
        indoors(|t| t.missed_wide),
        indoors(|t| t.missed_now),
    );
    println!("  and with the roof cut, which is how the 2.38% on record was measured:");
    println!("  {in_drawn:>9}  drawn sprite pixels");
    println!(
        "  {in_wide:>9}  {:>5.2}%  discarded against the whole-tile box (before S3)",
        pct(in_wide, in_drawn),
    );
    println!(
        "  {in_now:>9}  {:>5.2}%  discarded against the box we give it today\n",
        pct(in_now, in_drawn),
    );

    // **Whose pixels those are.** The total is a mixture of six different boxes
    // and a share of it names none of them: a panel inset by `PANEL_THICKNESS`
    // and a lid with no thickness at all overhang their art for reasons that
    // have nothing to do with this plan, and reading their cost as the
    // footprint's would be the same mistake in the other direction.
    let mut by_claim: BTreeMap<&'static str, (u32, u64, u64, u64)> = BTreeMap::new();
    for tally in tallies.values() {
        let row = by_claim.entry(tally.claim).or_default();
        row.0 += tally.placements;
        row.1 += tally.drawn;
        row.2 += tally.missed_wide;
        row.3 += tally.missed_now;
    }
    println!("  the same pixels by the kind of box that answered them:\n");
    println!("    placements     drawn      before       now   claim");
    for (claim, (count, drew, before, now)) in &by_claim {
        println!(
            "    {count:>10}  {drew:>8}  {:>9.2}%  {:>7.2}%   {claim}",
            pct(*before, *drew),
            pct(*now, *drew),
        );
    }
    println!();

    // The class on its own. A share of the whole world hides it: a footprint
    // reaches 2% of the placements, so a cost that would be alarming *there* is
    // a rounding error in the total above.
    let class: Vec<&Tally> = tallies.values().filter(|tally| tally.claim == NARROWED).collect();
    let class_placements: u32 = class.iter().map(|t| t.placements).sum();
    let class_drawn: u64 = class.iter().map(|t| t.drawn).sum();
    let class_now: u64 = class.iter().map(|t| t.missed_now).sum();
    let class_wide: u64 = class.iter().map(|t| t.missed_wide).sum();
    println!(
        "  the class a footprint reached: {class_placements} placements of {} graphics",
        class.len(),
    );
    println!("  {class_drawn:>9}  drawn pixels among them");
    println!(
        "  {class_wide:>9}  {:>5.2}%  discarded when they were whole tiles",
        pct(class_wide, class_drawn),
    );
    println!(
        "  {class_now:>9}  {:>5.2}%  discarded now\n",
        pct(class_now, class_drawn),
    );

    // **What is discarded today, before this plan touched anything.** The
    // biggest contributors by pixel, with the two numbers that explain most of
    // them beside each: how tall the art is, and how tall the box under it is.
    // `Z_STEP` pixels of picture stand on one `z` unit of box, so a picture
    // whose height in pixels far exceeds its box's height in units is a picture
    // hanging over its own lid — `docs/footprints.md`'s D1, in pixels.
    let mut standing: Vec<(&u16, &Tally)> = tallies.iter().collect();
    standing.sort_by_key(|(_, tally)| std::cmp::Reverse(tally.missed_now));
    println!("  what the impostor discards today, the twelve largest by pixel:\n");
    println!("    graphic   discarded   of its art   art px   box z   claim / name");
    for (graphic, tally) in standing.iter().take(12) {
        println!(
            "    0x{graphic:04X}  {:>10}  {:>9.1}%  {:>3}x{:<3}  {:>5}   {}",
            tally.missed_now,
            pct(tally.missed_now, tally.drawn),
            tally.art.0,
            tally.art.1,
            tally.height,
            tiledata.static_tile(**graphic).name,
        );
    }
    println!();

    // Which pictures pay it. A share says how much was lost and never says
    // whether it was a tabletop.
    let mut worst: Vec<(&u16, &Tally)> = tallies
        .iter()
        .filter(|(_, tally)| tally.missed_now > tally.missed_wide)
        .collect();
    worst.sort_by_key(|(_, tally)| std::cmp::Reverse(tally.missed_now - tally.missed_wide));
    println!("  the pictures that lost the most, by pixels over all their placements:");
    for (graphic, tally) in worst.iter().take(12) {
        let lost = tally.missed_now - tally.missed_wide;
        let each = lost / u64::from(tally.placements.max(1));
        println!(
            "    0x{graphic:04X}  {lost:>7} px  {each:>5}/placement  {:>4} placements  \
             {:>5.1}% of its art  {}",
            tally.placements,
            pct(lost, tally.drawn),
            tiledata.static_tile(**graphic).name,
        );
    }
    if worst.is_empty() {
        println!("    none — no footprint took a pixel off the screen");
    }

    // And the shadow, which is the other number S4 wants and is a count rather
    // than a picture: a piece the grid holds nothing for casts nothing, so a
    // narrower box for it cannot move a shadow by one ray. Named rather than
    // counted, because the plan's expectation is *zero* and a bare number would
    // leave whoever reads it to go and find out which graphic broke it.
    let shadowing: Vec<(&u16, &Tally)> = tallies
        .iter()
        .filter(|(_, tally)| tally.claim == NARROWED && tally.in_the_grid > 0)
        .collect();
    let moved: u32 = shadowing.iter().map(|(_, tally)| tally.in_the_grid).sum();
    println!(
        "\n  {moved:>9}  footprinted placements the grid holds a primitive for — each of them\n\
         \x20            casts a shadow this plan has narrowed, and `docs/footprints.md`'s S4\n\
         \x20            expects none at all"
    );
    for (graphic, tally) in &shadowing {
        println!(
            "    0x{graphic:04X}  {:>4} placements  {:?}  {}",
            tally.in_the_grid,
            tally.footprint.expect("a footprinted graphic"),
            tiledata.static_tile(**graphic).name,
        );
    }
}
