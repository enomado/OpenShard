//! Which edge of its tile a wall stands on, measured from the wall's own art.
//!
//! Nothing in `tiledata.mul` records this. `docs/lighting.md`'s decision 3 says
//! so and is right: there is no flag, no byte and no table for which of a tile's
//! four edges a `WALL` static occupies. The *picture* knows, because in this
//! projection a tile edge is half a cell wide with a 45° run, and a wall drawn on
//! one edge cannot be drawn on any of the other three without looking different.
//!
//! # What is measured
//!
//! The **base edge**: the lowest drawn pixel of each column of the sprite. That
//! is the line where the wall meets the ground, and it is the one feature of a
//! wall's silhouette with no ornament on it — the top of a wall carries
//! crenellations, eaves and antialiased tips, and the sides are cut by whatever
//! the artist drew standing against them. Two numbers come out of it:
//!
//! - **Which half of the tile's column the wall occupies.** A tile's diamond
//!   spans 22 pixels either side of the column the sprite is centred on; each of
//!   the four edges covers exactly one of those halves. North and east are the
//!   right half, south and west the left.
//! - **Which way the base edge runs.** It descends to the right for north and
//!   south, and to the left for east and west, because those are the two world
//!   axes and this projection turns them into the two screen diagonals.
//!
//! Those two bits are the four faces, and they are independent, which is what
//! makes the pair a measurement rather than a guess.
//!
//! # And what it refuses
//!
//! A detector that cannot say "I do not know" is the failure mode here: a wrong
//! face is a wall shaded along an axis it does not run on, and every graphic
//! this is offered is a graphic somebody's shard draws. Three gates, each of
//! which a real client graphic fails:
//!
//! - A **corner** (`0x0104`) is drawn as two faces at once, so both halves of
//!   its column are full. Whichever half is proposed, the other one is occupied
//!   past the sliver a wall's thickness accounts for, and the proposal dies.
//! - A **post** (`0x0101`) covers neither half: its base is a few columns wide
//!   with a level bottom, so no run of 45° can be fitted to it.
//! - Anything whose base is not straight — a tree, a barrel, a fence with a gap
//!   — fails the straightness test over the half it claims.
//!
//! Undecided is not a defect and costs nothing: [`crate::place::Stance`] falls
//! back to `Upright`, which is what every static did before this module existed.
//!
//! # Where the numbers came from
//!
//! Read off the client's own art rather than derived: `0x0100` "marble wall" has
//! its mass in columns 18..=43 of a 44-wide sprite with the base descending to
//! the left, which is the east face, and its base line lands on the predicted
//! `dy = 22 - across` to the pixel over the whole 22-column span. `0x0007` is
//! the south face of the same shape, and `0x0104` is the corner that has to come
//! back undecided. The sweep in `tests/facing.rs` is what says how much of a real
//! install this reads, because a detector with no coverage count is a green light
//! for having checked nothing.

use openshard_uofiles::image::Image;

/// Which edge of its tile a wall stands on.
///
/// Named for the world direction the edge faces *out* of the tile, which is the
/// same naming the map uses: the north edge is the one at `y` = the tile's own,
/// and a wall on it runs along `+x`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Face {
    /// The `y0` edge, running along `+x`. The upper-right side of the diamond.
    North,
    /// The `x1` edge, running along `+y`. The lower-right side.
    East,
    /// The `y1` edge, running along `+x`. The lower-left side.
    South,
    /// The `x0` edge, running along `+y`. The upper-left side.
    West,
}

impl Face {
    /// Where in its tile a point `run` of the way along this face is, as the
    /// fraction pair the place attachment carries.
    ///
    /// `run` is `0` at the edge's start and `1` at its end, following the world
    /// axis the edge lies along — so the *next* tile's face starts its run at 0
    /// where this one ended at 1, and the two name one world line. That is the
    /// whole point of measuring the face at all: a row of wall tiles stops being
    /// a row of sprites and becomes one continuous surface.
    ///
    /// The Rust copy of what `statics.wgsl` does per fragment, and it exists so
    /// the seam property can be stated in a unit test rather than only in a
    /// rendered frame.
    pub fn place_at(self, run: f32) -> (f32, f32) {
        let run = run.clamp(0.0, 1.0);
        match self {
            Self::North => (run, 0.0),
            Self::East => (1.0, run),
            Self::South => (run, 1.0),
            Self::West => (0.0, run),
        }
    }

    /// How far along this face a pixel `across` pixels from the tile's own
    /// column is, as a `0..=1` run. The inverse of the projection, for one edge.
    ///
    /// Outside the face's own half this saturates rather than extrapolating: a
    /// wall sprite carries a few pixels of its own *thickness* past the tile's
    /// centre column, and those pixels belong to the near end of the edge and
    /// not to a place outside the tile.
    pub fn run_at(self, across: f32) -> f32 {
        let v = across / HALF_TILE_WIDTH;
        match self {
            Self::North => v,
            Self::East => 1.0 - v,
            Self::South => 1.0 + v,
            Self::West => -v,
        }
        .clamp(0.0, 1.0)
    }
}

/// A tile's width in the drawn image, and half of it — which is also how wide
/// one edge of the diamond is, since each edge spans one half of the column.
///
/// `crate::camera::TILE_WIDTH` is the same 44 and this module cannot borrow it
/// without pulling a camera into a function that is handed nothing but pixels.
/// Pinned against it in the tests below.
const TILE_WIDTH: f32 = 44.0;
const HALF_TILE_WIDTH: f32 = TILE_WIDTH / 2.0;

/// How many of a face's 22 columns must actually be drawn for it to be a face.
///
/// Not all 22: the far end of an edge tapers to a point, and the last column or
/// two of a real sprite is antialiased away to nothing.
const MIN_FILLED: usize = 18;

/// How far past the tile's centre column the *other* half may be drawn on.
///
/// A wall is a solid with a thickness, and the picture shows that thickness: the
/// far side of the face is a sliver past the edge (3.5 pixels on `0x0100`, 2.5 on
/// `0x0007`), and where the wall is low enough to look down on, its whole *top*
/// surface is drawn as well — 8.5 pixels on `0x0063`, the low garden wall Britain
/// is fenced with. A thickness of `t` tiles projects to `22t` pixels across, so
/// twelve is a wall half a tile thick, which is thicker than any the client
/// ships.
///
/// The number that matters is the gap to the thing this has to refuse: a corner
/// is two faces and covers the *whole* other half, 21.5 pixels of it. Twelve
/// sits between the two with room on both sides, and it was chosen by measuring
/// both — at six, 40% of the walls standing in Britain were read; at twelve, 76%,
/// and the corners are still refused. See `tests/facing.rs`.
const SPILL: f32 = 12.0;

/// How far past the tile's own column anything may be drawn at all.
///
/// Two pixels, which is an antialiased edge. Beyond it the picture is of
/// something bigger than one cell — a whole building, a multi-tile tree — and
/// none of the four faces is an answer about it.
const OVERHANG: f32 = 2.0;

/// How far a base pixel may sit off the 45° line fitted through the ends.
///
/// One pixel is antialiasing and rounding; three is a different shape.
const STRAIGHT: i32 = 2;

/// How far the run of the fitted line may be from its rise, in pixels. A wall's
/// base is at 45° exactly — that is the projection, not a style — so this is
/// tolerance for the ends being blunt rather than for a slope being different.
const SQUARE: i32 = 3;

/// How tall the wall must stand over its base, in pixels, before this is willing
/// to call it a wall.
///
/// Four units of `z`. Under it are the slabs — a roof piece, a step, a low
/// railing — whose base can be a clean 45° run without the thing being a
/// billboard whose picture is height.
const MIN_STANDING: u16 = 16;

/// Which edge of its tile this wall stands on, or `None` if the art does not say.
///
/// Pure: an image in, a verdict out, no files and no state. Called once per
/// graphic while the atlas packs it — see
/// [`StaticAtlas::insert`](crate::atlas::StaticAtlas) — because the answer is a
/// property of the picture and a picture is packed once.
///
/// The cost is one pass over the sprite's pixels, which is the pass that
/// [`copy_sprite`](crate::atlas) is making anyway.
pub fn face_of(image: &Image) -> Option<Face> {
    let width = image.width();
    // Narrower than a tile cannot hold a whole edge in the half it belongs to:
    // the sprite is centred on the tile's column, so a 22-wide picture reaches
    // 11 pixels either side and covers no edge at all.
    if f32::from(width) < TILE_WIDTH {
        return None;
    }
    let base = base_edge(image);
    // The right half first, then the left. They are exclusive by construction —
    // each rules the other out through `SPILL` — so the order decides nothing,
    // and a graphic that somehow satisfied both would be a corner and is refused
    // by the check inside each.
    for half in [Half::Right, Half::Left] {
        if let Some(face) = half.read(&base, width) {
            return Some(face);
        }
    }
    None
}

/// Which half of the tile's column a face occupies.
#[derive(Clone, Copy)]
enum Half {
    /// `across` in `(0, 22]` — the north and east edges.
    Right,
    /// `across` in `[-22, 0)` — the south and west edges.
    Left,
}

impl Half {
    /// Which way `across` counts on this half: right is positive, left negative.
    fn sign(self) -> f32 {
        match self {
            Self::Right => 1.0,
            Self::Left => -1.0,
        }
    }

    /// The face on this half of the column, or `None` if the art is not a wall
    /// standing on it.
    fn read(self, base: &BaseEdge, width: u16) -> Option<Face> {
        let middle = f32::from(width) / 2.0;
        // The columns of this half, and everything the other half may not hold.
        let mut mine: Vec<(i32, u16)> = Vec::new();
        for (column, bottom) in base.columns() {
            let across = f32::from(column) + 0.5 - middle;
            // Drawn outside the tile's own column altogether. A picture wider
            // than one cell is not one wall standing on one edge of it — the
            // client ships whole buildings and multi-tile trees as single
            // graphics, and the first version of this read a 106-pixel statue as
            // a north face because it only ever looked at the half it had
            // proposed. Whatever is on the *other* side of the sprite has to be
            // looked at too, and that is what this line does.
            if across.abs() > HALF_TILE_WIDTH + OVERHANG {
                return None;
            }
            // How far *into* this half the column is: positive on the half being
            // proposed and negative on the other one. Written as one signed
            // number rather than two mirrored comparisons, so that the two halves
            // cannot drift apart — a tolerance loosened on one side only is
            // exactly the shape of bug this whole module is a defence against.
            let into = across * self.sign();
            if into > 0.0 && into <= HALF_TILE_WIDTH {
                mine.push((i32::from(column), bottom));
                continue;
            }
            // Drawn on the wrong side of the tile's centre. A little is the
            // wall's own thickness showing past the edge it stands on; more is a
            // second face, which is a corner. A column past the *far* vertex is
            // neither — it is the antialiased tip `OVERHANG` allows — and it is
            // left out of the fit rather than counted against it.
            if -into > SPILL {
                return None;
            }
        }
        if mine.len() < MIN_FILLED {
            return None;
        }

        let (first_column, first_bottom) = *mine.first().unwrap();
        let (last_column, last_bottom) = *mine.last().unwrap();
        let run = last_column - first_column;
        let rise = i32::from(last_bottom) - i32::from(first_bottom);
        // At 45°, and steeply enough that the sign means something. A level base
        // has a rise of zero and names no direction at all.
        if (rise.abs() - run).abs() > SQUARE || run < MIN_FILLED as i32 {
            return None;
        }
        let descending_right = rise > 0;
        // Straight, not merely straight at the ends: a chevron has the same two
        // endpoints as the line through them.
        let step = if descending_right { 1 } else { -1 };
        for (column, bottom) in &mine {
            let want = first_bottom as i32 + step * (column - first_column);
            if (i32::from(*bottom) - want).abs() > STRAIGHT {
                return None;
            }
        }
        // And it stands up. A slab whose base happens to be a clean 45° run —
        // a roof piece, a low step — is not a billboard whose picture is height,
        // and shading it as one would be worse than leaving it alone.
        if base.standing(last_column.min(first_column) + run / 2) < MIN_STANDING {
            return None;
        }

        Some(match (self, descending_right) {
            (Self::Right, true) => Face::North,
            (Self::Right, false) => Face::East,
            (Self::Left, true) => Face::South,
            (Self::Left, false) => Face::West,
        })
    }
}

/// The lowest and highest drawn pixel of every column of a sprite.
///
/// One pass, kept as two rows of `Option` rather than as a list of runs: the
/// reads below are by column index and the sprite is at most a few hundred wide.
struct BaseEdge {
    /// Per column: the last drawn row, or `None` for a column with nothing in it.
    bottom: Vec<Option<u16>>,
    /// Per column: the first drawn row. Only the difference is read — see
    /// [`BaseEdge::standing`].
    top: Vec<Option<u16>>,
}

impl BaseEdge {
    /// Every column that has anything drawn in it, left to right, with the row
    /// its lowest pixel is on.
    fn columns(&self) -> impl Iterator<Item = (u16, u16)> + '_ {
        self.bottom
            .iter()
            .enumerate()
            .filter_map(|(column, bottom)| bottom.map(|row| (column as u16, row)))
    }

    /// How tall the picture is in one column, in pixels — nothing for a column
    /// with nothing in it.
    fn standing(&self, column: i32) -> u16 {
        let Ok(column) = usize::try_from(column) else {
            return 0;
        };
        match (self.top.get(column), self.bottom.get(column)) {
            (Some(Some(top)), Some(Some(bottom))) => bottom - top + 1,
            _ => 0,
        }
    }
}

/// Walk the sprite once and record where each column's picture starts and ends.
///
/// "Drawn" is the same question the fragment shader asks and the same one
/// [`StaticAtlas::opaque_at`](crate::atlas::StaticAtlas::opaque_at) asks: a
/// transparent pixel is absent. That is the client's own rule for static art —
/// `ArtLoader.ReadStaticArt` writes a run's pixel only when it is non-zero — so
/// a hole inside a run and a column no run covered are one thing here.
fn base_edge(image: &Image) -> BaseEdge {
    let width = usize::from(image.width());
    let mut edge = BaseEdge {
        bottom: vec![None; width],
        top: vec![None; width],
    };
    for y in 0..image.height() {
        for x in 0..image.width() {
            // `pixel` is `None` only outside the rectangle, which this loop is
            // not; the transparency is the question being asked.
            if image.pixel(x, y).unwrap().is_transparent() {
                continue;
            }
            let column = usize::from(x);
            edge.top[column].get_or_insert(y);
            edge.bottom[column] = Some(y);
        }
    }
    edge
}

/// A wall's silhouette, drawn the way the projection draws one: a parallelogram
/// standing on one edge of the tile's diamond.
///
/// `face` decides which edge, `height` how far the wall rises above it. The
/// picture is 44 wide, which is what the client ships, and its bottom row is the
/// diamond's bottom vertex — [`statics::stand_on`](crate::statics::stand_on) puts
/// every static's bottom edge there whatever the art holds, so a north face,
/// whose lowest pixel is at the diamond's *right* vertex, genuinely has 22 blank
/// rows under it.
///
/// An ordinary `pub` item rather than a `#[cfg(test)]` one, for the reason
/// [`crate::scene`]'s rooms are: the readers are outside this crate. The GPU
/// frame test needs a picture the atlas will read a known face off, and it needs
/// it to be the *same* picture the unit tests below decide against — a second
/// hand-drawn parallelogram in `tests/frame.rs` would be a second opinion about
/// what a wall looks like, and the day the two drifted the frame test would be
/// asserting about a shape this module never sees.
///
/// No client files, and none needed: the shape is the projection, and the
/// projection is arithmetic.
pub fn silhouette(face: Face, height: u16) -> Image {
    use openshard_uofiles::color::Color16;

    let width = 44u16;
    let rows = height + 45;
    let mut pixels = vec![Color16::TRANSPARENT; usize::from(width) * usize::from(rows)];
    for column in 0..width {
        let across = f32::from(column) + 0.5 - f32::from(width) / 2.0;
        // Only the half this face stands on is drawn — the thickness sliver
        // a real sprite has is the subject of its own test below.
        let into = match face {
            Face::North | Face::East => across,
            Face::South | Face::West => -across,
        };
        if into <= 0.0 || into > HALF_TILE_WIDTH {
            continue;
        }
        // Where this column's base pixel is: the edge's own descent, which
        // is what `Face::run_at` inverts.
        let run = face.run_at(across);
        let base = match face {
            // `dy = 22 * (run - 1)` for the two edges whose apex is the
            // diamond's top vertex, `22 * run` for the two whose apex is its
            // bottom one — see `docs/lighting.md`, step 15.
            Face::North | Face::West => HALF_TILE_WIDTH * (run - 1.0),
            Face::East | Face::South => HALF_TILE_WIDTH * run,
        };
        let bottom = (base + f32::from(height) + 22.0).round() as u16;
        let top = bottom.saturating_sub(height);
        for row in top..=bottom.min(rows - 1) {
            pixels[usize::from(row) * usize::from(width) + usize::from(column)] =
                Color16(0b0_11111_00000_00000);
        }
    }
    Image::new(width, rows, pixels)
}

#[cfg(test)]
mod tests {
    use openshard_uofiles::color::Color16;

    use super::*;

    /// The tile is the camera's tile and not a second opinion about it.
    #[test]
    fn a_tile_is_the_width_the_camera_draws_one_at() {
        assert_eq!(TILE_WIDTH as i32, crate::camera::TILE_WIDTH);
    }

    /// Each of the four, told apart from a picture of it. The property that
    /// matters is that all four are distinguished — a detector that answered
    /// `North` always would pass any one of these on its own.
    #[test]
    fn each_face_is_read_back_off_its_own_silhouette() {
        for face in [Face::North, Face::East, Face::South, Face::West] {
            assert_eq!(face_of(&silhouette(face, 80)), Some(face), "{face:?}");
        }
    }

    /// A corner is two faces at once, and both of them have to lose.
    ///
    /// `0x0104` is the client's own, and this is the shape of it: every column
    /// of the tile drawn, with the base descending both ways from the middle.
    /// The gate that catches it is `SPILL` — whichever half is proposed, the
    /// other is occupied far past a wall's thickness.
    #[test]
    fn a_corner_is_undecided() {
        let east = silhouette(Face::East, 80);
        let south = silhouette(Face::South, 80);
        let mut pixels = Vec::with_capacity(east.pixels().len());
        for (a, b) in east.pixels().iter().zip(south.pixels()) {
            pixels.push(if a.is_transparent() { *b } else { *a });
        }
        let corner = Image::new(east.width(), east.height(), pixels);
        assert_eq!(face_of(&corner), None);
    }

    /// A post covers no edge: a few columns at the tile's centre with a level
    /// base. `0x0101` is the client's, and the gate is that nothing 45° can be
    /// fitted through a level line.
    #[test]
    fn a_post_is_undecided() {
        let width = 44u16;
        let rows = 90u16;
        let mut pixels = vec![Color16::TRANSPARENT; usize::from(width) * usize::from(rows)];
        for column in 18..26u16 {
            for row in 4..86u16 {
                pixels[usize::from(row) * usize::from(width) + usize::from(column)] =
                    Color16(0b0_11111_00000_00000);
            }
        }
        assert_eq!(face_of(&Image::new(width, rows, pixels)), None);
    }

    /// A wall's own thickness, drawn past the edge it stands on, does not stop it
    /// being read — and enough of it does.
    ///
    /// This is the tolerance a real graphic needs: `0x0100` draws 3.5 pixels of
    /// its far side past the tile's centre column, `0x0063` draws 8.5 because it
    /// is low enough that you look down on its top, and a detector that demanded
    /// an empty half would refuse most of the walls a city is built out of. The
    /// second half of the test is what keeps that tolerance from swallowing the
    /// corner above — the two are stated together because loosening one without
    /// looking at the other is exactly how this gate stops working.
    #[test]
    fn a_sliver_of_thickness_is_allowed_and_a_second_face_is_not() {
        for by in [4, 10] {
            let sliver = smeared(&silhouette(Face::East, 80), by);
            assert_eq!(face_of(&sliver), Some(Face::East), "{by} pixels of thickness");
        }
        let wide = smeared(&silhouette(Face::East, 80), 20);
        assert_eq!(face_of(&wide), None, "twenty pixels is another face");
    }

    /// The same picture with every drawn column copied `by` columns to its left,
    /// which is what a wall's thickness looks like on the far side of its edge.
    fn smeared(image: &Image, by: u16) -> Image {
        let (width, height) = (image.width(), image.height());
        let mut pixels = image.pixels().to_vec();
        for y in 0..height {
            for x in by..width {
                let from = usize::from(y) * usize::from(width) + usize::from(x);
                let to = from - usize::from(by);
                if !pixels[from].is_transparent() && pixels[to].is_transparent() {
                    pixels[to] = pixels[from];
                }
            }
        }
        Image::new(width, height, pixels)
    }

    /// A slab whose base is a clean 45° run is still not a wall.
    ///
    /// The shape a roof piece has: the right geometry along the ground and no
    /// height above it. Without the standing gate this would come back `North`
    /// and be shaded along an axis it does not run on.
    #[test]
    fn a_low_slab_is_undecided() {
        assert_eq!(face_of(&silhouette(Face::North, 6)), None);
    }

    /// Nothing drawn at all, and a picture too narrow to hold an edge. Neither is
    /// a wall and neither may panic.
    #[test]
    fn an_empty_or_narrow_picture_is_undecided() {
        assert_eq!(
            face_of(&Image::new(44, 44, vec![Color16::TRANSPARENT; 44 * 44])),
            None
        );
        assert_eq!(
            face_of(&Image::new(20, 60, vec![Color16(0b0_11111_00000_00000); 20 * 60])),
            None
        );
    }

    /// **The seam**, which is the whole reason a face is worth measuring.
    ///
    /// The end of one tile's face and the start of the next tile's along the same
    /// run name one world line. Stated in world coordinates, because that is
    /// where the lighting reads them: tile `(x, y)`'s north face at run 1 is the
    /// point `(x + 1, y)`, and tile `(x + 1, y)`'s north face at run 0 is the same
    /// point. Without this a row of wall tiles is a row of separately lit
    /// sprites, which is exactly what it looked like.
    #[test]
    fn one_tile_s_face_ends_where_the_next_one_s_begins() {
        for (face, step) in [
            (Face::North, (1.0, 0.0)),
            (Face::South, (1.0, 0.0)),
            (Face::East, (0.0, 1.0)),
            (Face::West, (0.0, 1.0)),
        ] {
            let (end_x, end_y) = face.place_at(1.0);
            let (start_x, start_y) = face.place_at(0.0);
            assert_eq!(
                (end_x, end_y),
                (start_x + step.0, start_y + step.1),
                "{face:?} does not join its neighbour along the axis it runs on",
            );
        }
    }

    /// The run is the inverse of the place, over the half the face occupies —
    /// the property `statics.wgsl` depends on, since it computes the run from a
    /// pixel's offset and the place from the run.
    #[test]
    fn a_run_and_a_place_are_one_mapping() {
        for face in [Face::North, Face::East, Face::South, Face::West] {
            for step in 0..=22u8 {
                let across = match face {
                    Face::North | Face::East => f32::from(step),
                    Face::South | Face::West => -f32::from(step),
                };
                let run = face.run_at(across);
                assert!((0.0..=1.0).contains(&run), "{face:?} at {across}: {run}");
                let (x, y) = face.place_at(run);
                // The fixed coordinate is the edge the face is, exactly — not
                // nearly. A wall's pixels are *on* the tile boundary, and a
                // fraction that drifted off it would put the lit surface inside
                // the tile.
                match face {
                    Face::North => assert_eq!(y, 0.0),
                    Face::South => assert_eq!(y, 1.0),
                    Face::East => assert_eq!(x, 1.0),
                    Face::West => assert_eq!(x, 0.0),
                }
            }
        }
    }
}
