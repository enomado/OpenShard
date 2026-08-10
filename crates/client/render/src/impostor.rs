//! Where a sprite's pixel is in the world, and which surface it is a pixel of.
//!
//! `docs/lighting_rebuild.md` phase 6. A static is drawn once, as its own
//! picture, and the geometry underneath that picture is found by *intersecting
//! the view ray with the boxes the occlusion grid already holds for it*. The
//! silhouette is the artist's; the volume is ours; and because there is only
//! ever one draw, there is no second silhouette to disagree with the first and
//! no border constant to hide the disagreement behind.
//!
//! # The ray is a constant
//!
//! [`crate::camera::project_exact`] is
//! `((x − y)·22, (x + y − 1)·22 − z·4)`. Hold a screen point fixed and the two
//! equations say `dx = dy` and `44·dx = 4·dz` — so every point that draws to one
//! pixel lies on a line in the single direction [`VIEW`], `(1, 1, 11)` in the
//! world's own units and exactly `(1, 1, 1)` once `z` is divided into tiles.
//! There is no per-fragment unprojection to write and no inverse matrix: a
//! fragment is a *slab test against a constant direction*, which is the whole of
//! why this is cheap enough to do per pixel.
//!
//! Two consequences worth stating because they simplify everything below. **No
//! axis is parallel to the ray** — every component of [`VIEW`] is non-zero — so
//! the parallel-axis branches [`crate::light::ray_vs_solid`] needs do not exist
//! here. And **the camera lies towards `+(1, 1, 1)`**: larger `x + y + z` draws
//! in front, which is what [`crate::depth`] sorts on and what
//! [`crate::solid::Side`] names when it says the three visible faces of a box
//! are `max.x`, `max.y` and `max.z`. So the surface a pixel shows is where the
//! ray *leaves* a box travelling along [`VIEW`], not where it enters one.
//!
//! # This is not a second `ray_vs_solid`
//!
//! [`crate::light::ray_vs_solid`] answers a *segment* between two given points,
//! clamped to `0..=1`, and reports how much of it a box swallowed. This answers
//! an unbounded ray in one fixed direction and reports **which face** it met,
//! which is the thing a normal comes from and the thing that function has no
//! notion of. They share the slab idea and nothing else; writing the shading
//! side in terms of the shadow side would mean threading an axis through a
//! function whose every caller wants a fraction.

use crate::light::Z_PER_TILE;

/// The direction every screen pixel's world line runs in, in world units —
/// tiles on `x` and `y`, `z` units on `z`.
///
/// See the module doc for the derivation. It points **towards the camera**: a
/// point further along it draws to the same pixel and stands in front.
pub const VIEW: [f32; 3] = [1.0, 1.0, Z_PER_TILE];

/// A tile's width in virtual pixels — `camera::TILE_WIDTH`, restated as a float
/// because every arithmetic here is one.
const TILE_WIDTH: f32 = crate::camera::TILE_WIDTH as f32;

/// How far outside a box, in tiles, a meeting may measure and still be a
/// meeting rather than a miss.
///
/// The same razor `blit.wesl`'s `RAY_TANGENT_TOLERANCE` names one level down,
/// and here for the same reason: a ray reaching a box's own corner leaves two
/// slabs at one `t`, the clamp that follows subtracts two nearly-equal `f32`s,
/// and what comes back is rounding rather than a gap. **Measured** — the corner
/// of a lid, dead on, reads `3.5e-6` of a tile out — so this is thirty times
/// that and still four hundredths of a *pixel*, which is far below anything a
/// picture can show. A fragment further out than this is genuinely off its own
/// volume, which is [`Meeting::outside`]'s whole subject.
pub const TANGENT: f32 = 1.0e-4;

/// One point of the view ray through a sprite fragment: the point at the
/// static's own base height.
///
/// `across` is how far the fragment is right of the column its sprite is centred
/// on, and `down` how far it is below the row its tile's own centre projects to,
/// both in virtual pixels — the two numbers `statics.wesl`'s vertex stage
/// already hands its fragment stage. `tile` and `base` are the static's own
/// place, which its instance already carries.
///
/// **This is exactly the inverse projection that shader's flat case already
/// spells**, and that is the point rather than a coincidence: a floor's pixel is
/// the point of the view ray at the tile's own height, so the branch that reads
/// a floor today is this ray at `t = 0`. Every other branch it carries — a face
/// pinned to an edge, a height recovered from `pixel_y` — is the same ray met
/// with a different plane, guessed from a stance instead of measured against a
/// box.
pub fn ray_from(tile: (i32, i32), base: f32, across: f32, down: f32) -> [f32; 3] {
    // `u − v = across/22` and `u + v = down/22 + 1`, solved. Written over the
    // whole tile width rather than the half so that neither line divides twice.
    [
        tile.0 as f32 + (across + down) / TILE_WIDTH + 0.5,
        tile.1 as f32 + (down - across) / TILE_WIDTH + 0.5,
        base,
    ]
}

/// Where a fragment of a **billboard** is: the point of its own view ray on the
/// vertical plane the sprite is drawn on.
///
/// `docs/lighting_rebuild.md` phase 7, and the half of it that is not a choice.
/// A mobile has no volume, so [`meets`] has nothing to meet and the pass falls
/// back to a *point* — the middle of the tile, with the height running down the
/// picture. That point is the same for **every pixel of a screen row**: a
/// billboard is forty pixels wide and all forty of them were told they are at
/// the tile's centre. Two things a person can see follow from it. The sprite is
/// lit flat across, because nothing about the light varies along a row. And
/// `blit.wesl`'s `dither` — which turns the sample pattern by an angle that
/// belongs to the *position* — hands one row one turn and the next row another,
/// so an eight-ray estimate lands in **horizontal bands** across the figure.
/// That is what a person standing next to a torch actually looks like today.
///
/// The plane is the one the picture is drawn on: it stands through the tile's
/// centre, it contains the vertical, and its horizontal direction is the one the
/// screen's own `x` axis runs along — `(1, -1)` in tiles, which is what
/// [`ray_from`] moves along when only `across` changes. So its normal is
/// `(1, 1, 0)`, the horizontal part of [`VIEW`]: the plane is turned towards the
/// camera, which is what a billboard *is*.
///
/// The arithmetic is [`ray_from`]'s ray met with that plane, and it is worth
/// stating that the height falls out unchanged: the meeting is at
/// `t = -down / TILE_WIDTH` along [`VIEW`], which takes `z` to
/// `base - down * Z_PER_TILE / TILE_WIDTH` — and `Z_PER_TILE / TILE_WIDTH` is
/// `1 / Z_STEP` exactly. The pass's old point already had the right `z`; what it
/// had wrong was `x` and `y`, and that is the whole of this function.
///
/// `statics.wesl`'s `billboard_at`, and the two are one formula.
pub fn billboard_at(tile: (i32, i32), base: f32, across: f32, down: f32) -> [f32; 3] {
    [
        tile.0 as f32 + 0.5 + across / TILE_WIDTH,
        tile.1 as f32 + 0.5 - across / TILE_WIDTH,
        base - down * Z_PER_TILE / TILE_WIDTH,
    ]
}

/// The billboard plane's own normal: `(1, 1, 0)` normalised, [`VIEW`]'s
/// horizontal part. `docs/lighting_rebuild.md` phase 7's "facing the camera"
/// candidate — never wrong, never interesting, and the one this phase picked:
/// the plane a mobile is drawn on has exactly one normal, so giving a fragment
/// of it *that* normal is not a choice among several so much as the plane's own
/// fact, stated rather than left as the zero vector.
///
/// `statics.wesl`'s equivalent, and the two are one formula — see
/// [`billboard_at`].
pub fn billboard_normal() -> [f32; 3] {
    let n = std::f32::consts::FRAC_1_SQRT_2;
    [n, n, 0.0]
}

/// Which run of a frame's [`Volume`] list belongs to one drawn static.
///
/// The association `docs/lighting_rebuild.md` phase 6 rests on: a fragment is
/// met against *these* boxes and no others, so one static's geometry cannot
/// reach a neighbour's picture. A range and not a list per instance because the
/// instance buffer is a flat row of words and this is two of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Range {
    /// Where this static's boxes start in the frame's list.
    pub offset: u32,
    /// How many it has. Zero for a static this frame's grid holds nothing for,
    /// which is a real answer and not a missing one — see
    /// `statics::push_volumes`.
    pub count: u32,
}

/// One box a static stands as, with the frame's own name for it.
///
/// **A volume, and for a fitted climbable that is not the same thing as the
/// grid's own solids.** `occlusion::Builder::add` decomposes a flight into
/// *surfaces* — a lid per tread, degenerate in `z`, and a riser per tread,
/// degenerate on the climb axis — whose union encloses nothing. That is right
/// for a shadow ray, which only ever asks what it crossed, and useless for a
/// view ray, which has to land on something for every pixel the art drew. For a
/// flight climbing *away* from the camera those surfaces do happen to cover the
/// visible side; for one climbing *towards* it every riser faces away and is
/// hidden behind its own tread, and the grid holds nothing at all where the art
/// draws the whole front of the staircase. So the volume here is one box a
/// tread — its strip, from the static's own base up to that tread's height —
/// and the grid is joined to rather than read from. Making the grid hold the
/// volume is `docs/lighting_geometry.md`'s question.
///
/// Everything that is not a fitted climbable *is* one of the grid's own boxes,
/// unchanged: a wall's panel, a floor's lid, a body's tile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    /// The low corner, in world units.
    pub lo: [f32; 3],
    /// And the high one.
    pub hi: [f32; 3],
    /// Which solid of this frame's grid a fragment of this box is a point of —
    /// `occlusion::SolidId::word`, so `SolidId::NOBODY` where the grid has none
    /// (a static the cutaway hid, or one past the draw ceiling).
    ///
    /// **One name a box, and for a tread that name is its own lid rather than
    /// its riser.** A fragment standing on the riser needs no name of its own:
    /// it lies exactly in that riser's plane, so a ray leaving it touches the
    /// riser at `t = 0` and nowhere else, which is precisely what phase 5's
    /// origin-touch rule already excuses. Carrying a second id a box would be a
    /// second answer to a question one rule has already closed.
    pub solid: u32,
}

impl Volume {
    /// One of the grid's own boxes as a volume a fragment can be met against.
    ///
    /// The only conversion between the two, so that
    /// [`crate::statics::push_volumes`] and a fixture stating a wall by hand
    /// cannot disagree about which end of a `Solid` is which — see
    /// [`crate::occlusion::Occlusion::box_of`], which is the one place a kind
    /// becomes geometry.
    pub fn of(space: &crate::solid::Solid, solid: u32) -> Self {
        Self {
            lo: [space.min.x as f32, space.min.y as f32, space.min.z as f32],
            hi: [space.max.x as f32, space.max.y as f32, space.max.z as f32],
            solid,
        }
    }

    /// What one box costs on the wire, and the stride `statics.wesl`'s own
    /// `Volume` has.
    ///
    /// Thirty-two rather than the twenty-eight the fields add up to: a
    /// `vec3<f32>` aligns to sixteen, so the shader's struct has a word of
    /// padding after `lo` whatever this side writes — and `solid` rides in the
    /// word after `hi` that the same rule would leave empty. It is free, which
    /// is why it is carried before anything reads it.
    pub const STRIDE: u64 = 32;

    /// This box as the shader's `Volume`, appended to `out`.
    ///
    /// The one place the layout is spelled on this side; the other is the struct
    /// in `statics.wesl`, and `a_volume_is_thirty_two_bytes_the_shader_can_read`
    /// is what holds the two together.
    pub fn write(&self, out: &mut Vec<u8>) {
        for value in self.lo {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&0f32.to_le_bytes());
        for value in self.hi {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&self.solid.to_le_bytes());
    }
}

/// Where the view ray meets one box, and which of its faces that is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Meeting {
    /// The point, in world units, clamped into the box — see [`Meeting::outside`].
    pub at: [f32; 3],
    /// Which way the surface it met looks: the box's own camera-facing face on
    /// one axis, so always one of `+x`, `+y`, `+z`.
    pub normal: [f32; 3],
    /// How far, in **tiles**, the ray passed outside the box before the clamp
    /// pulled it back — at most [`TANGENT`] for a ray that genuinely went
    /// through it, and [`Meeting::hit`] is that comparison spelled once.
    ///
    /// The measurement `docs/lighting_rebuild.md` phase 6 asks its own "done
    /// when" to carry, and the reason this is a number rather than an
    /// `Option<[f32; 3]>`: a sprite overhangs its own volume by a pixel or two
    /// wherever the fitted box is narrower than the art, and what matters about
    /// those pixels is not that they exist but *how far out* they are. A frame
    /// where this reaches a third of a tile is a frame whose boxes are wrong.
    pub outside: f32,
    /// How far along [`VIEW`] the meeting is, from the point [`ray_from`] gave.
    ///
    /// Only [`nearest`] reads it — the box with the largest `t` is the one in
    /// front — and it is on the struct rather than recomputed there because the
    /// slab test has already worked it out.
    pub t: f32,
}

impl Meeting {
    /// Whether the ray went through the box at all, as against passing beside
    /// it and being clamped onto it.
    pub fn hit(self) -> bool {
        self.outside <= TANGENT
    }
}

/// Where the view ray through `from` meets the box `lo..=hi`.
///
/// Always answers. A ray that misses the box outright is answered with the
/// point on its camera-facing corner nearest the ray, and [`Meeting::outside`]
/// is what says so — one expression rather than a branch, because at tangency
/// the two cases are the same point and a branch there would be a seam.
///
/// The face is the one whose *exit* comes first: travelling from the camera
/// inwards a box is entered through whichever of its three visible faces the ray
/// reaches first, and `min` over the three exit parameters is that face.
///
/// **Ties go to `z`, then `y`, then `x`, and that order is a decision.** Two
/// exits are equal exactly on an edge or a corner of the box. Preferring `z` is
/// what gives a shared edge to the surface above it; the same choice, one
/// dimension down, is why [`nearest`] gives a tread's shared edge to the tread's
/// top.
///
/// **A face with no area is not a face, and that is a rule rather than a tie.**
/// A lid is flat in `z`, so the faces its `x` and `y` slabs would present are
/// *lines*: the only face of a lid a ray can meet is its own plane. Ordering the
/// tie towards `z` used to stand in for that and it only ever covered the exact
/// tie — a ray that leaves the footprint one rounding *before* it reaches the
/// plane is not a tie, it is a miss, and it was answered with a side face's
/// normal. On the isometric grid that is one pixel per tile corner, since the
/// projection puts a tile's corner on a whole pixel: a lattice of dots over a
/// roof, each shaded as a wall in the middle of a floor. Reported twice, and it
/// is the second cause `docs/lighting_rebuild.md` phase 6i's floor entry names.
pub fn meets(from: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> Meeting {
    // The `z` face, which every box in this grid has: `Solid::box_of` gives a
    // panel `PANEL_THICKNESS` and everything else a whole tile, so `x` and `y`
    // are never degenerate and this face never has zero area.
    let mut axis = 2;
    let mut exit = (hi[2] - from[2]) / VIEW[2];
    // And the two side faces, which a lid does not have: both of their areas
    // carry the `z` extent as a factor, so one test retires both.
    if hi[2] > lo[2] {
        for a in [1, 0] {
            // No `abs(delta) < eps` case: every component of `VIEW` is positive,
            // so `lo` is always the near end of the slab and `hi` the far one,
            // and neither division can be by zero.
            let far = (hi[a] - from[a]) / VIEW[a];
            if far < exit {
                exit = far;
                axis = a;
            }
        }
    }

    // The exit axis's own coordinate is **stated** and not computed. `exit` is
    // the `t` at which this ray reaches the plane `hi[axis]`, so the meeting is
    // on that plane by definition; `from + exit * VIEW` is a division and a
    // multiplication that only round back to it, and `VIEW[2]` is `Z_PER_TILE`
    // — eleven, and no power of two — so the `z` round trip has nothing exact
    // about it and a GPU contracting the pair into an `fma` need not land where
    // a CPU does. Everything downstream leans on a fragment being a point *of*
    // the box it names, so the plane is taken from the number that chose it.
    // `tests/frame.rs`'s `a_sprite_fragment_is_a_point_of_the_primitive_it_
    // names` holds it as an equality, and `impostor.wesl`'s `meets` is the twin.
    let mut at = [
        from[0] + exit * VIEW[0],
        from[1] + exit * VIEW[1],
        from[2] + exit * VIEW[2],
    ];
    at[axis] = hi[axis];
    let clamped = [
        at[0].clamp(lo[0], hi[0]),
        at[1].clamp(lo[1], hi[1]),
        at[2].clamp(lo[2], hi[2]),
    ];
    // In tiles, which means the `z` term is divided into them: a third of a tile
    // out sideways and a third of a tile out in height are the same miss, and
    // three `z` units are not.
    let miss = [
        at[0] - clamped[0],
        at[1] - clamped[1],
        (at[2] - clamped[2]) / Z_PER_TILE,
    ];
    let outside = (miss[0] * miss[0] + miss[1] * miss[1] + miss[2] * miss[2]).sqrt();

    let mut normal = [0.0; 3];
    normal[axis] = 1.0;
    Meeting {
        at: clamped,
        normal,
        // A distance rather than the slab test's own `enter <= exit` predicate,
        // and the two cannot disagree because this one *is* the clamp above: a
        // ray that goes through the box is clamped by nothing and measures zero.
        outside,
        t: exit,
    }
}

/// Which of a static's own boxes a sprite fragment is a pixel of.
///
/// The box in front wins — largest `t` along [`VIEW`] — among those the ray
/// genuinely goes through. If it goes through none of them, the one it passes
/// closest to wins instead, which is the sprite overhanging its own volume and
/// is the case [`Meeting::outside`] exists to measure.
///
/// Ties go to the box the iterator yielded first, on both counts. For a flight
/// that is the grid's own order — a tread's top before its own riser, see
/// `occlusion::Part` — so the shared edge where a tread's lid stops and its
/// riser begins reads as the lid. That is a decision and not an accident: the
/// two surfaces meet along one line, and a horizontal one is what a person
/// looking at a step expects its outer edge to be.
///
/// `None` only for an empty iterator, which is a static the grid holds nothing
/// for — dropped by the cutaway, or past the draw ceiling. Its caller has to
/// have an answer for that; there is no box to invent one from here.
pub fn nearest<T, I>(from: [f32; 3], boxes: I) -> Option<(T, Meeting)>
where
    I: IntoIterator<Item = (T, [f32; 3], [f32; 3])>,
{
    let mut best: Option<(T, Meeting)> = None;
    for (what, lo, hi) in boxes {
        let met = meets(from, lo, hi);
        let better = match &best {
            None => true,
            // A real hit always beats a miss, however near the miss came; two
            // hits are decided by which is in front, and two misses by which
            // came closer.
            Some((_, had)) => match (had.hit(), met.hit()) {
                (false, true) => true,
                (true, false) => false,
                (true, true) => met.t > had.t,
                (false, false) => met.outside < had.outside,
            },
        };
        if better {
            best = Some((what, met));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The unit cube on tile `(100, 101)`, ten `z` tall.
    fn cube() -> ([f32; 3], [f32; 3]) {
        ([100.0, 101.0, 0.0], [101.0, 102.0, 10.0])
    }

    /// **A billboard's fragment is on the plane, and the plane is turned towards
    /// the camera.**
    ///
    /// Stated as the two properties that make it one rather than as the numbers
    /// it comes out with: every point is in the plane through the tile's centre
    /// whose normal is `VIEW`'s horizontal part, and the point is on the
    /// fragment's *own* view ray, so it draws where it was drawn. A formula that
    /// satisfies both is the meeting, and nothing else is.
    #[test]
    fn a_billboards_fragment_is_its_view_ray_met_with_a_plane_facing_the_camera() {
        let tile = (100, 101);
        let base = 7.0;
        let centre = [tile.0 as f32 + 0.5, tile.1 as f32 + 0.5];
        for across in [-20.0, -3.0, 0.0, 11.5, 22.0] {
            for down in [-8.0, 0.0, 4.0, 37.0] {
                let at = billboard_at(tile, base, across, down);
                // In the plane: the normal is `(1, 1, 0)` and the centre is on it.
                let off_plane = (at[0] - centre[0]) + (at[1] - centre[1]);
                assert!(off_plane.abs() < 1e-5, "{across}/{down}: {off_plane}");
                // And on the fragment's own view ray, which is what says it draws
                // on the pixel it was drawn on: the two differ by a multiple of
                // `VIEW` and by nothing else.
                let start = ray_from(tile, base, across, down);
                let along = [at[0] - start[0], at[1] - start[1], at[2] - start[2]];
                let t = along[0] / VIEW[0];
                for axis in 0..3 {
                    assert!(
                        (along[axis] - t * VIEW[axis]).abs() < 1e-4,
                        "{across}/{down}: axis {axis} of {along:?} is not {t} of {VIEW:?}",
                    );
                }
            }
        }
    }

    /// And the height is the one the pass already drew with, to the bit.
    ///
    /// The point of the entry is what phase 7 does *not* change: `z` was right
    /// before — `base - down / Z_STEP` — and a new derivation that moved it would
    /// be lifting every mobile off the ground it stands on. `Z_PER_TILE /
    /// TILE_WIDTH` is `1 / Z_STEP` exactly, which is why the two spellings agree
    /// rather than nearly agree.
    #[test]
    fn a_billboards_height_is_what_the_pass_already_drew() {
        for down in [-8.0, 0.0, 4.0, 37.0, 130.0] {
            let at = billboard_at((100, 101), 7.0, 13.0, down);
            assert_eq!(at[2], 7.0 - down / crate::camera::Z_STEP as f32, "at {down}");
        }
    }

    #[test]
    fn a_volume_is_thirty_two_bytes_the_shader_can_read() {
        // The two spellings of one layout: this side's `write` and
        // `statics.wesl`'s `struct Volume`. Nothing but a person reading both
        // compares the *order*, so what is asserted here is every field's own
        // offset — a row that grew a byte would silently read every box past the
        // first at the wrong place, which is the failure `SpriteQuad::write`'s
        // own test was written from.
        let mut out = Vec::new();
        Volume {
            lo: [100.0, 101.0, 3.0],
            hi: [101.0, 102.0, 7.0],
            solid: 9,
        }
        .write(&mut out);
        assert_eq!(out.len() as u64, Volume::STRIDE);
        assert_eq!(&out[0..4], &100f32.to_le_bytes(), "lo.x");
        assert_eq!(&out[8..12], &3f32.to_le_bytes(), "lo.z");
        assert_eq!(&out[12..16], &0f32.to_le_bytes(), "the vector's own padding");
        assert_eq!(&out[16..20], &101f32.to_le_bytes(), "hi.x");
        assert_eq!(&out[24..28], &7f32.to_le_bytes(), "hi.z");
        assert_eq!(&out[28..32], &9u32.to_le_bytes(), "and which solid it is");
    }

    #[test]
    fn the_view_direction_is_one_tile_per_tile_per_z_per_tile() {
        // The derivation in the module doc, restated as the thing a reader can
        // check: `Z_PER_TILE` is the whole of the third component, so a step of
        // one tile along `x` and along `y` is a step of one *tile* of height.
        assert_eq!(VIEW, [1.0, 1.0, 11.0]);
        assert_eq!(VIEW[2] / Z_PER_TILE, 1.0);
    }

    #[test]
    fn a_point_of_the_ray_projects_to_the_offsets_it_was_built_from() {
        // The inverse is only worth having if it is one, so this goes forward
        // again through `project_exact` and compares — not against a second copy
        // of the algebra, against the projection itself.
        for (across, down) in [(0.0, 0.0), (7.0, 3.0), (-11.0, 20.0), (21.5, -9.25)] {
            let at = ray_from((100, 101), 5.0, across, down);
            let point = crate::camera::project_exact(crate::camera::WorldSpot {
                x: f64::from(at[0]),
                y: f64::from(at[1]),
                z: f64::from(at[2]),
            });
            let centre = crate::camera::project_exact(crate::camera::WorldSpot {
                x: 100.5,
                y: 101.5,
                z: 5.0,
            });
            // A thousandth of a pixel, and the bound is `f32`'s rather than the
            // algebra's: a world coordinate of a hundred has about eight
            // millionths of a tile between neighbouring floats, which is two
            // ten-thousandths of a pixel once projected. The arithmetic itself
            // is exact — the same two divisions run backwards.
            assert!(
                (point.x - centre.x - f64::from(across)).abs() < 1e-3,
                "across: {across} gave {}",
                point.x - centre.x
            );
            assert!(
                (point.y - centre.y - f64::from(down)).abs() < 1e-3,
                "down: {down} gave {}",
                point.y - centre.y
            );
        }
    }

    #[test]
    fn every_point_of_one_ray_draws_to_one_pixel() {
        // The claim the whole module rests on: `VIEW` is the projection's own
        // null direction, so walking along it does not move the picture.
        let from = ray_from((100, 101), 0.0, 4.0, -6.0);
        let start = crate::camera::project_exact(crate::camera::WorldSpot {
            x: f64::from(from[0]),
            y: f64::from(from[1]),
            z: f64::from(from[2]),
        });
        for t in [-3.0f32, 0.25, 1.0, 17.5] {
            let at = crate::camera::project_exact(crate::camera::WorldSpot {
                x: f64::from(from[0] + t * VIEW[0]),
                y: f64::from(from[1] + t * VIEW[1]),
                z: f64::from(from[2] + t * VIEW[2]),
            });
            assert!(
                (at.x - start.x).abs() < 1e-9 && (at.y - start.y).abs() < 1e-9,
                "t: {t} moved the pixel to {at:?} from {start:?}"
            );
        }
    }

    #[test]
    fn a_pixel_over_the_middle_of_a_cube_meets_its_lid() {
        let (lo, hi) = cube();
        // Forty pixels above the row the tile's own centre projects to, which on
        // a cube ten `z` tall — forty pixels of lift, at four a unit — is the
        // middle of its top face.
        let met = meets(ray_from((100, 101), 0.0, 0.0, -40.0), lo, hi);
        assert_eq!(met.normal, [0.0, 0.0, 1.0], "at: {:?}", met.at);
        assert!(met.hit(), "{met:?}");
        assert!(
            (met.at[0] - 100.5).abs() < 1e-4 && (met.at[1] - 101.5).abs() < 1e-4,
            "the middle of the lid: {:?}",
            met.at
        );
        assert!((met.at[2] - 10.0).abs() < 1e-4, "at: {:?}", met.at);
    }

    #[test]
    fn a_pixel_on_the_lower_right_of_a_cube_meets_its_east_face() {
        let (lo, hi) = cube();
        // Right of the sprite's own column and below the lid's near corner: the
        // hexagon's lower-right facet, which `crate::solid::Side::East` names.
        let met = meets(ray_from((100, 101), 0.0, 14.0, 0.0), lo, hi);
        assert_eq!(met.normal, [1.0, 0.0, 0.0], "at: {:?}", met.at);
        assert!(met.hit(), "{met:?}");
        assert!((met.at[0] - 101.0).abs() < 1e-4, "at: {:?}", met.at);
    }

    #[test]
    fn a_pixel_on_the_lower_left_of_a_cube_meets_its_south_face() {
        let (lo, hi) = cube();
        let met = meets(ray_from((100, 101), 0.0, -14.0, 0.0), lo, hi);
        assert_eq!(met.normal, [0.0, 1.0, 0.0], "at: {:?}", met.at);
        assert!(met.hit(), "{met:?}");
        assert!((met.at[1] - 102.0).abs() < 1e-4, "at: {:?}", met.at);
    }

    #[test]
    fn the_three_faces_a_meeting_can_name_are_the_three_a_camera_sees() {
        // Not a case list: a sweep over the cube's own screen rectangle, every
        // pixel of it, asserting that no meeting ever names a face turned away
        // from the viewer. `crate::solid::Side` is the claim being checked.
        let (lo, hi) = cube();
        let mut seen = [false; 3];
        let mut outside = 0.0f32;
        for step_down in -60..=60 {
            for step_across in -30..=30 {
                let from = ray_from((100, 101), 0.0, step_across as f32, step_down as f32);
                let met = meets(from, lo, hi);
                assert!(
                    met.normal.iter().all(|n| *n >= 0.0),
                    "a face turned away from the camera: {:?}",
                    met.normal
                );
                if met.hit() {
                    let axis = met.normal.iter().position(|n| *n == 1.0).unwrap();
                    seen[axis] = true;
                } else {
                    outside = outside.max(met.outside);
                }
            }
        }
        assert_eq!(seen, [true; 3], "the sweep should reach all three faces");
        assert!(outside > 0.0, "a sweep this wide should also miss the cube");
    }

    #[test]
    fn a_ray_that_misses_takes_the_nearest_point_of_the_box_and_says_how_far() {
        let (lo, hi) = cube();
        // Two tiles east of the cube's own column: nothing of it is on this
        // line at all.
        let from = ray_from((100, 101), 0.0, 3.0 * TILE_WIDTH, 0.0);
        let met = meets(from, lo, hi);
        assert!(met.outside > 0.0, "this ray misses: {met:?}");
        // Whatever it answers is a point *of the box* — that is what the clamp
        // buys, and it is the property every reader downstream leans on.
        for a in 0..3 {
            assert!(
                met.at[a] >= lo[a] - 1e-5 && met.at[a] <= hi[a] + 1e-5,
                "axis {a} left the box: {:?}",
                met.at
            );
        }
    }

    #[test]
    fn a_lid_with_no_thickness_is_met_on_its_own_plane() {
        // A floor: `min.z == max.z`, which `crate::solid::Solid`'s own doc says
        // is a legal box and means the plane it lies at.
        let lo = [100.0, 101.0, 7.0];
        let hi = [101.0, 102.0, 7.0];
        let met = meets(ray_from((100, 101), 0.0, 0.0, -28.0), lo, hi);
        assert!(met.hit(), "{met:?}");
        assert_eq!(met.normal, [0.0, 0.0, 1.0]);
        assert!((met.at[2] - 7.0).abs() < 1e-4, "at: {:?}", met.at);

        // A pixel just inside the south-west rim, where the `z` slab and the `y`
        // slab all but run out together: still a lid.
        let rim = meets(ray_from((100, 101), 0.0, -21.12, -28.0), lo, hi);
        assert!(rim.hit(), "{rim:?}");
        assert_eq!(rim.normal, [0.0, 0.0, 1.0], "at: {:?}", rim.at);

        // And **on** the corner itself, which is where this test used to stop.
        // It said: the two slabs do not tie at all, their `t`s differ by
        // `3.5e-6` of a tile, so which face is named there is decided by
        // rounding rather than by the tie rule — "that is one pixel of a picture
        // and no claim is made about it".
        //
        // **One pixel of a picture per tile corner** is what that came to. The
        // isometric projection puts a tile's corner on a whole pixel, so the ray
        // that leaves the footprint a rounding before it reaches the plane is
        // drawn once per corner and nowhere else, and it was answered with a
        // *side* face's normal — a wall's cosine in the middle of a roof. A
        // person reported it twice as holes in the roofs, and the second report
        // named the tile: `1492, 1643`. Measured on that frame in `View::Light`:
        // fourteen isolated bright pixels on a lattice of exactly `TILE_WIDTH`,
        // now none, and thirty pixels of the whole frame changed.
        //
        // It is a claim now, and it is not the tie rule doing the work: a lid's
        // side faces have no area, so `meets` does not offer them at all. Both
        // corners of the sprite, since the two halves of the lattice run out on
        // different slabs — the `y` one first and then the `x` one.
        //
        // The `3.5e-6` is gone from the answer as well, and that is the same
        // sentence rather than a second effect: the exit is solved for the plane
        // the lid *is*, so a corner ray lands on it exactly and `outside` reads
        // `0.0` instead of a rounding's worth of miss.
        for (across, which) in [(-22.0, "the west corner"), (22.0, "the east corner")] {
            let corner = meets(ray_from((100, 101), 0.0, across, -28.0), lo, hi);
            assert!(corner.hit(), "{which}: {corner:?}");
            assert_eq!(
                corner.normal,
                [0.0, 0.0, 1.0],
                "{which}: a lid has no side face to be met on: {corner:?}",
            );
            assert_eq!(
                corner.at[2], hi[2],
                "{which}: off the lid's own plane: {corner:?}"
            );
        }
    }

    #[test]
    fn the_box_in_front_wins() {
        // Two cubes on one line of sight, one a tile further along `VIEW` than
        // the other. The near one is the one with the larger `x + y + z`, which
        // is what `crate::depth` sorts on.
        let far = ([100.0, 101.0, 0.0], [101.0, 102.0, 10.0]);
        let near = ([101.0, 102.0, 11.0], [102.0, 103.0, 21.0]);
        let from = ray_from((100, 101), 0.0, 0.0, 0.0);
        let (which, met) = nearest(from, [("far", far.0, far.1), ("near", near.0, near.1)]).unwrap();
        assert_eq!(which, "near", "{met:?}");
        // And the order it is offered in does not decide it.
        let (which, _) = nearest(from, [("near", near.0, near.1), ("far", far.0, far.1)]).unwrap();
        assert_eq!(which, "near");
    }

    #[test]
    fn a_hit_beats_a_miss_however_near_the_miss_came() {
        let (lo, hi) = cube();
        // A box the ray passes a hair beside, offered first, and the cube it
        // goes through, offered second.
        let beside = ([103.0, 101.0, 0.0], [103.01, 102.0, 10.0]);
        let from = ray_from((100, 101), 0.0, 0.0, 0.0);
        let (which, met) = nearest(from, [("beside", beside.0, beside.1), ("cube", lo, hi)]).unwrap();
        assert_eq!(which, "cube", "{met:?}");
        assert!(met.hit(), "{met:?}");
    }

    #[test]
    fn nothing_to_meet_is_answered_with_nothing() {
        let empty: [(u32, [f32; 3], [f32; 3]); 0] = [];
        assert!(nearest([100.5, 101.5, 0.0], empty).is_none());
    }

    #[test]
    fn a_tread_and_its_neighbour_s_riser_meet_with_no_pixel_between_them() {
        // The whole of what the phase is for, one seam of it: a step's top and
        // the rise in front of it are two primitives sharing one line, and every
        // pixel of a sweep across that line is a pixel *of* one of them. A hole
        // here — a fragment answered by the nearest-point fallback — is what
        // `WIDTH_OVERLAP` grew a mesh to cover; there is nothing to grow when the
        // two shapes are met by one ray.
        //
        // A **north**-climbing flight of two treads on tile `(100, 101)`, three
        // and six `z` tall, exactly as `occlusion::Builder::add` pushes it: a
        // top and a riser a tread, in climb order. North because the climb then
        // runs *away* from the camera, which is the case with a seam in it —
        // see the sibling test for the other direction, where the flight turns
        // its own back and there is no vertical surface at all.
        let lid_low = ([100.0, 101.5, 3.0], [101.0, 102.0, 3.0]);
        let riser_low = ([100.0, 102.0, 0.0], [101.0, 102.0, 3.0]);
        let lid_high = ([100.0, 101.0, 6.0], [101.0, 101.5, 6.0]);
        let riser_high = ([100.0, 101.5, 3.0], [101.0, 101.5, 6.0]);

        let mut surfaces = Vec::new();
        // A quarter of a pixel a step, down the sprite's own middle column,
        // over the whole of what the four solids cover.
        for step in -184i16..=88 {
            let down = f32::from(step) / 4.0;
            let from = ray_from((100, 101), 0.0, 0.0, down);
            let (which, met) = nearest(
                from,
                [
                    ("lid_low", lid_low.0, lid_low.1),
                    ("riser_low", riser_low.0, riser_low.1),
                    ("lid_high", lid_high.0, lid_high.1),
                    ("riser_high", riser_high.0, riser_high.1),
                ],
            )
            .unwrap();
            assert!(met.hit(), "a pixel of no surface at down {down}: {met:?}");
            if surfaces.last() != Some(&which) {
                surfaces.push(which);
            }
        }
        // Down the screen: the upper tread, the rise below its lip, the lower
        // tread, the rise below *its* lip. Each once — a surface coming back a
        // second time would be two of them disagreeing about where the seam is,
        // which is the defect a grown mesh hides rather than fixes.
        assert_eq!(surfaces, ["lid_high", "riser_high", "lid_low", "riser_low"]);
    }
}
