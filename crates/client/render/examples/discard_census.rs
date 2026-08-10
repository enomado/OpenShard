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
use openshard_client_render::camera::TileBounds;
use openshard_client_render::cutaway::Cutaway;
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
/// them in — **`statics::push_volumes`'s own rule, not `boxes_of`'s alone**.
///
/// The difference is one line and it is the whole reason this function exists
/// rather than a bare `boxes_of` call: where the occlusion grid holds a *named*
/// primitive for a piece, the box a fragment is met against is **the grid's own
/// merged solid**, continuous across every tile the piece runs over, and not
/// the per-tile box `boxes_of` yielded. A wall that runs ten tiles is one solid
/// there and ten boxes here, and a ray that misses the tenth box does not miss
/// the wall.
///
/// This tool claimed for a session that the difference could not reach what it
/// measures, on the grounds that only a piece the grid took in has a merged
/// primitive. That was too strong twice over: `docs/footprints.md`'s S4 found
/// forty-two placements of its own class inside the grid, and every panel and
/// whole-tile share this tool prints is about pieces that are mostly *walls*.
/// So the grid is built and consulted, which is what the frame does.
fn volumes_of(
    x: i32,
    y: i32,
    z: i8,
    graphic: Graphic,
    tile: &StaticTile,
    shape: &Shape,
    occlusion: &occlusion::Occlusion,
) -> Vec<Volume> {
    let owner = occlusion::Owner::new(z, graphic);
    let mut out = Vec::new();
    occlusion::boxes_of(x, y, z, tile, shape, |part, _, space| {
        let space = match occlusion.id_of(x, y, owner, part) {
            Some(id) => occlusion.solid(id).space,
            None => space,
        };
        out.push(Volume::of(&space, 0));
    });
    out
}

/// The occlusion grid over one window, built exactly as `light::collect` builds
/// the frame's: every static offered to [`occlusion::Builder::add`], which
/// drops the `CLEAR` ones itself.
///
/// The cutaway is [`Cutaway::OPEN`] — nothing hidden — because a census is about
/// what the geometry *is*, not about what one player standing somewhere can see.
/// A frame with a roof cut holds fewer solids and would merge differently, which
/// is a second question and not this one.
fn grid(
    map: &Map,
    tiledata: &TileData,
    atlas: &StaticAtlas,
    at: (i32, i32),
    radius: i32,
    footprints: bool,
) -> occlusion::Occlusion {
    let mut builder = occlusion::Builder::new(TileBounds {
        min_x: at.0 - radius,
        max_x: at.0 + radius,
        min_y: at.1 - radius,
        max_y: at.1 + radius,
    });
    for x in at.0 - radius..=at.0 + radius {
        for y in at.1 - radius..=at.1 + radius {
            for item in map.statics_at(x as u16, y as u16) {
                let graphic = Graphic(item.tile);
                let shape = occlusion::shape_of(Some(atlas), graphic);
                let shape = match footprints {
                    true => shape,
                    false => Shape {
                        footprint: None,
                        ..shape
                    },
                };
                builder.add(
                    x as u16,
                    y as u16,
                    item.z,
                    graphic,
                    tiledata.static_tile(graphic.0),
                    shape,
                );
            }
        }
    }
    builder.finish(&Cutaway::OPEN)
}

/// One picture's pixels against one set of boxes: how many are drawn, and how
/// many of those the impostor would discard.
///
/// The walk is the whole sprite and not the 44-wide tile column
/// `facing::silhouettes_agree` clips to: the pass draws every pixel of the art,
/// so a table top hanging past its own cell is exactly the pixel this is about.
fn overhang(image: &Image, at: (i32, i32), z: i8, volumes: &[Volume]) -> (u32, u32, Steps) {
    let (width, height) = (image.width(), image.height());
    let middle = f32::from(width) / 2.0;
    let centre_row = f32::from(height) - HALF_TILE;
    let (mut drawn, mut missed) = (0u32, 0u32);
    let mut steps = Steps::default();
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
            // **The nearest miss and not "did any box answer"** — the two are the
            // same predicate today and stop being one the moment the tolerance
            // moves, which is the question this histogram exists to settle. A
            // picture met against several boxes is as far outside as its *best*
            // box says, so the fold is a minimum and not a count.
            let nearest = volumes
                .iter()
                .map(|volume| impostor::meets(start, volume.lo, volume.hi))
                .fold(f32::INFINITY, |best, met| best.min(met.outside));
            if nearest > impostor::FRAGMENT {
                missed += 1;
                steps.count(nearest);
            }
        }
    }
    (drawn, missed, steps)
}

/// How far the misses miss by, in **fragments** rather than in tiles.
///
/// The two are one division apart and the division is the whole point. A
/// fragment is a virtual pixel, and one step of the screen grid moves a floor's
/// point by `sqrt(2) / TILE_WIDTH` of a tile — the world line
/// [`impostor::ray_from`] walks when `across` alone changes is `(1, −1) / 44`.
/// So a miss of a fraction of a step is a **sample landing between two
/// fragments**, which no box anywhere can fix, and a miss of several steps is
/// art genuinely hanging off its own volume. Read in tiles the two are 0.03 and
/// 0.3 and look like the same kind of number; read in steps one is under 1 and
/// the other is over 10.
///
/// `docs/pixels.md` owns the pair of grids this divides between.
#[derive(Default)]
struct Steps {
    /// Under half a fragment out: the sample-grid case in full, since half a
    /// step is the furthest a fragment's centre can sit from an edge that
    /// genuinely passes through that fragment.
    half: u64,
    /// Under one.
    one: u64,
    /// Under two.
    two: u64,
    /// Under eight.
    eight: u64,
    /// And the rest — art that overhangs by an object's worth.
    beyond: u64,
    /// The largest miss seen, in steps.
    worst: f32,
}

/// One step of the fragment grid, in tiles: what a change of one virtual pixel
/// in `across` does to [`impostor::ray_from`]'s answer.
const STEP: f32 = std::f32::consts::SQRT_2 / (2.0 * HALF_TILE);

impl Steps {
    fn count(&mut self, outside: f32) {
        let steps = outside / STEP;
        self.worst = self.worst.max(steps);
        *match steps {
            s if s < 0.5 => &mut self.half,
            s if s < 1.0 => &mut self.one,
            s if s < 2.0 => &mut self.two,
            s if s < 8.0 => &mut self.eight,
            _ => &mut self.beyond,
        } += 1;
    }

    fn add(&mut self, other: &Steps) {
        self.half += other.half;
        self.one += other.one;
        self.two += other.two;
        self.eight += other.eight;
        self.beyond += other.beyond;
        self.worst = self.worst.max(other.worst);
    }

    fn total(&self) -> u64 {
        self.half + self.one + self.two + self.eight + self.beyond
    }
}

/// How many drawn pixels lie outside the screen columns this footprint's box
/// can reach.
///
/// A world-axis-aligned box spans `across` from `(x0 − y1) · 22` to
/// `(x1 − y0) · 22` and nothing outside that, at any height — the projection's
/// own `across = (u − v) · 22`, inverted. So this asks one height-free question
/// of a measurement: *is the box the art drew the box of this picture, or of
/// something under it?*
fn outside_the_band(image: &Image, footprint: Footprint) -> u32 {
    let (min_x, max_x, min_y, max_y) = footprint.spans();
    let low = (min_x - max_y) * HALF_TILE;
    let high = (max_x - min_y) * HALF_TILE;
    let (width, height) = (image.width(), image.height());
    let middle = f32::from(width) / 2.0;
    let mut outside = 0;
    for row in 0..height {
        for column in 0..width {
            let drawn = image
                .pixel(column, row)
                .is_some_and(|pixel| !pixel.is_transparent());
            let across = f32::from(column) + 0.5 - middle;
            outside += u32::from(drawn && (across < low || across > high));
        }
    }
    outside
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
/// error that would make every share below meaningless.
///
/// **It measured forty-four at either height and now measures none**, and the
/// forty-four are why [`impostor::FRAGMENT`] exists. One row of the tile's own
/// width, each pixel `1 / TILE_WIDTH` of a tile outside a box it is plainly a
/// pixel of — the sample grid, not a disagreement — and on the screen they were
/// the dashed line along every tile seam that a person reported as a glowing
/// grid over the floor. A tolerance of one fragment takes the whole row, which
/// is what makes this control a *gate* on that constant rather than a noise
/// floor to subtract: it is zero, and nothing but the tolerance shrinking can
/// put a pixel back into it.
fn controls(at: (i32, i32)) -> Vec<(u8, u32, u32, u32, Steps)> {
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
            let (drawn, missed, steps) = overhang(&image, at, 0, std::slice::from_ref(&own));
            let (_, missed_far, _) = overhang(&image, at, 0, std::slice::from_ref(&elsewhere));
            (top, drawn, missed, missed_far, steps)
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
    /// Those same misses, sorted by how far out they are in *fragments* — see
    /// [`Steps`], and it is the number the hit tolerance has to be chosen from.
    steps: Steps,
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
    /// **Drawn pixels whose own screen column no box of this picture can ever
    /// reach**, and the share of them.
    ///
    /// Height-free by construction, which is what makes it a candidate gate
    /// under D5: a box's projected *column* band depends only on its horizontal
    /// extent — `across = (u − v) · 22` — so a pixel outside that band misses
    /// whatever the box's top is. A picture whose base edge describes the whole
    /// object has almost nothing outside it; one whose base edge describes a
    /// *leg* has its whole top outside, which is the table at Britain's
    /// `(1499, 1664)` — `0x0B80` measures 5/8 by 5/8 while its two neighbours in
    /// the same table stand as whole tiles.
    outside_band: u64,
    /// Which kind of claim its box is — `geometry_census.rs`'s own vocabulary,
    /// because a share of discarded pixels that does not say *whose* pixels is
    /// a number nobody can act on.
    claim: &'static str,
    /// **A picture the client calls a `PLATFORM` that stands as the box its art
    /// fits** — a table, a counter, a display case.
    ///
    /// The class this tool was pointed at when a person said the table at
    /// Britain's `(1496, 1663)` had been chopped. `Shape::of` fits a prism to
    /// every picture the wall detector called a corner, a tabletop drawn as a
    /// diamond is one, and `boxes_of` used to read a prism only under
    /// `CLIMBABLE` — so `0x0B06` stood as two `PANEL_THICKNESS` slabs with
    /// `prism E 4` measured and unread beside it. It reads one under `PLATFORM`
    /// now, and this counts what that moved: the share still overhanging its own
    /// box is the column to watch, since it went from 53.3% to 2.6% on that
    /// graphic.
    platform_body: bool,
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
    // `boxes_of`'s own order: a climbable or a PLATFORM whose art fits a prism
    // is a body, a climbable that fits none is a whole tile, a BACKGROUND piece
    // is a lid, and the rest are read off the silhouette.
    let body = tile.flags.is_climbable() || (tile.flags.is_platform() && !tile.flags.is_background());
    if body && shape.prism.is_some() {
        return "a fitted prism, one body a tread";
    }
    if tile.flags.is_climbable() {
        return "whole tile, a climbable that would not fit";
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

    // The two grids: the one the frame builds today, and the one it built
    // before S3 gave `boxes_of` a footprint to read. Two rather than one because
    // merging is a function of the boxes — a narrowed box may stop touching its
    // neighbour — so a "before" measured against today's grid would be half of
    // each answer.
    let live = grid(&map, &tiledata, &atlas, (cx, cy), radius, true);
    let before = grid(&map, &tiledata, &atlas, (cx, cy), radius, false);

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

                let now = volumes_of(x, y, item.z, graphic, tile, &shape, &live);
                let (drawn, missed_now, steps) = overhang(image, (x, y), item.z, &now);
                let missed_wide = match shape.footprint {
                    // Nothing to compare: the two shapes are the same shape, and
                    // walking the picture twice for one answer is only slower.
                    // The two *grids* can still differ here — a neighbour's
                    // narrowed box merges differently — but not for this piece,
                    // whose own solids are the same solids either way.
                    None => missed_now,
                    Some(_) => {
                        let boxes = volumes_of(x, y, item.z, graphic, tile, &wide, &before);
                        overhang(image, (x, y), item.z, &boxes).1
                    }
                };

                // How much of the picture is outside the band its own box can
                // reach, in columns alone. Only asked of the class a footprint
                // narrowed: for every other claim the box is the whole tile and
                // the band is the tile's own.
                let outside = match shape.footprint {
                    None => 0,
                    Some(footprint) => outside_the_band(image, footprint),
                };

                placements += 1;
                let tally = tallies.entry(graphic.0).or_default();
                tally.placements += 1;
                tally.drawn += u64::from(drawn);
                tally.missed_now += u64::from(missed_now);
                tally.steps.add(&steps);
                tally.missed_wide += u64::from(missed_wide);
                tally.footprint = shape.footprint;
                tally.claim = claim_of(tile, &shape);
                tally.outside_band += u64::from(outside);
                tally.art = (image.width(), image.height());
                tally.roof = tile.flags.is_roof();
                tally.platform_body = tile.flags.is_platform()
                    && !tile.flags.is_climbable()
                    && !tile.flags.is_background()
                    && shape.prism.is_some();
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

    report(&tiledata, &images, &tallies, placements, packed, (cx, cy), radius);
}

/// The three questions in the order S4 asks them: what the discard is now, what
/// the footprint added to it, and whether any of it can move a shadow.
fn report(
    tiledata: &TileData,
    images: &BTreeMap<u16, Image>,
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
    for (top, drew, missed, far, steps) in controls(at) {
        println!(
            "    {top:>2} z units tall   {missed:>4} of {drew:>4} miss ({:>5.2}%)   \
             worst {:>5.2} fragments, {} of them under one   \
             moved a hundred tiles: {far} of {drew} ({:.2}%)",
            pct(u64::from(missed), u64::from(drew)),
            steps.worst,
            steps.half + steps.one,
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

    // **How far the misses miss by**, which is the question a share cannot
    // answer and the one the tolerance is chosen from: a miss under one
    // fragment is the sample grid and a miss over ten is an object. See
    // [`Steps`].
    let mut steps = Steps::default();
    for tally in tallies.values() {
        steps.add(&tally.steps);
    }
    let all = steps.total();
    println!("  how far out the misses are, in fragments of the screen grid:");
    for (name, count) in [
        ("under half a fragment", steps.half),
        ("half to one", steps.one),
        ("one to two", steps.two),
        ("two to eight", steps.eight),
        ("eight and beyond", steps.beyond),
    ] {
        println!("    {name:>22}   {count:>9}  {:>5.2}%", pct(count, all));
    }
    println!("    {:>22}   {:>9.2}\n", "the worst", steps.worst);

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
    let mut by_claim: BTreeMap<&'static str, (u32, u64, u64, u64, Steps)> = BTreeMap::new();
    for tally in tallies.values() {
        let row = by_claim.entry(tally.claim).or_default();
        row.0 += tally.placements;
        row.1 += tally.drawn;
        row.2 += tally.missed_wide;
        row.3 += tally.missed_now;
        row.4.add(&tally.steps);
    }
    println!("  the same pixels by the kind of box that answered them:\n");
    println!("    placements     drawn      before       now   under a fragment   claim");
    for (claim, (count, drew, before, now, steps)) in &by_claim {
        // **And how much of each class's own discard is the sample grid.** The
        // shares beside it say how much art a box loses; this column says how
        // much of that loss no box could have prevented, because the sample sits
        // between two fragments rather than off the volume.
        let near = steps.half + steps.one;
        println!(
            "    {count:>10}  {drew:>8}  {:>9.2}%  {:>7.2}%   {near:>8} {:>6.2}%   {claim}",
            pct(*before, *drew),
            pct(*now, *drew),
            pct(near, *now),
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

    // **The measurement that is already made and thrown away.** See
    // `Tally::prism_unused`: how much of the world is stood up as panels or as
    // a whole tile while its own art has a prism fitted to it.
    let bodies: Vec<(&u16, &Tally)> = tallies.iter().filter(|(_, tally)| tally.platform_body).collect();
    let unused_placements: u32 = bodies.iter().map(|(_, tally)| tally.placements).sum();
    let unused_shadowing: u32 = bodies.iter().map(|(_, tally)| tally.in_the_grid).sum();
    println!(
        "  {unused_placements:>9}  placements of {} graphics the client calls a PLATFORM whose art fits\n\
         \x20            a prism, so `boxes_of` stands them as that body rather than as two\n\
         \x20            panels — {unused_shadowing} of them are occluders, which is how many shadows\n\
         \x20            that moved\n",
        bodies.len(),
    );
    let mut worst_unused: Vec<&(&u16, &Tally)> = bodies.iter().collect();
    worst_unused.sort_by_key(|(_, tally)| std::cmp::Reverse(tally.missed_now));
    println!("    graphic  placements   discarded   prism fit   name");
    for (graphic, tally) in worst_unused.iter().take(10) {
        // **The score, because a threshold is what any such decision would have
        // to be made of.** `facing::PRISM_FITS` is 0.9 and every graphic here
        // already passed it; what separates a display case from a wall — if
        // anything does — is how much *further* past it they sit, and a list
        // with no scores in it cannot say whether a threshold exists.
        let fit = images
            .get(graphic)
            .map_or(0.0, |image| openshard_client_render::facing::best_prism(image).1);
        println!(
            "    0x{graphic:04X}  {:>10}  {:>10}  {fit:>10.3}   {} ({:.1}% of its art)",
            tally.placements,
            tally.missed_now,
            tiledata.static_tile(**graphic).name,
            pct(tally.missed_now, tally.drawn),
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
    // **The candidate gate, measured on the class it would judge.** See
    // `Tally::outside_band`: a picture whose base edge described the whole
    // object has almost nothing outside the columns its box can reach; one
    // whose base edge described a leg has its top outside.
    println!("  the class a footprint reached, by how much of each picture its box cannot reach:\n");
    println!("    graphic  placements   outside the box's own columns   name");
    let mut band: Vec<(&u16, &Tally)> = tallies
        .iter()
        .filter(|(_, tally)| tally.claim == NARROWED)
        .collect();
    band.sort_by_key(|(_, tally)| std::cmp::Reverse(tally.outside_band * 1000 / tally.drawn.max(1)));
    for (graphic, tally) in band.iter().take(10) {
        println!(
            "    0x{graphic:04X}  {:>10}  {:>27.1}%   {}",
            tally.placements,
            pct(tally.outside_band, tally.drawn),
            tiledata.static_tile(**graphic).name,
        );
    }
    let (band_out, band_drawn): (u64, u64) = band.iter().fold((0, 0), |(out, all), (_, tally)| {
        (out + tally.outside_band, all + tally.drawn)
    });
    println!(
        "    the class together: {:.1}% of its art is outside the columns its own box reaches\n",
        pct(band_out, band_drawn),
    );

    // **And the sweep the threshold has to come out of**, in `PLATEAU`'s own
    // manner: what each cap keeps and what it gives up. A cap that refuses a
    // measurement hands that picture back the whole tile, which is the answer
    // that shipped before S3 and is never *wrong*, only wide.
    println!("  what a cap on that share would keep and give up:\n");
    println!("       cap   placements kept   art still outside its box   measurements given up");
    for cap in [5u64, 8, 10, 12, 15, 20, 25, 30] {
        let (mut kept, mut given, mut out, mut all) = (0u32, 0u32, 0u64, 0u64);
        for (_, tally) in &band {
            let share = pct(tally.outside_band, tally.drawn);
            match share <= cap as f64 {
                true => {
                    kept += tally.placements;
                    out += tally.outside_band;
                    all += tally.drawn;
                }
                false => given += tally.placements,
            }
        }
        println!(
            "    {cap:>4}%   {kept:>15}   {:>24.1}%   {given:>21}",
            pct(out, all),
        );
    }
    println!();

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
