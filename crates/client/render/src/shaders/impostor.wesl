// Where a sprite's pixel is in the world, and which surface it is a pixel of.
//
// `crate::impostor` is the Rust twin and carries the argument at length: the
// projection has one null direction, so every point that draws to a given pixel
// lies on a line in the constant direction `VIEW`, and a fragment is a *slab
// test* rather than an unprojection. `docs/lighting_rebuild.md` phase 6.
//
// **Two spellings, and one thing compares them.** This file and `impostor.rs`
// are the same arithmetic written twice, the way `normal_format.wesl` and
// `gbuffer::pack_normal` are — and as there, what keeps them honest is a frame
// test that renders a sprite and asserts the *word the GPU wrote* equals what
// the Rust side answers for the same boxes, rather than two readers agreeing
// about a picture.
//
// The **selection** rule — which of a static's boxes a fragment belongs to — is
// not here: it needs the storage buffer the boxes live in, which is a pass's own
// binding, so `statics.wesl` spells the loop and this spells one meeting. See
// `impostor::nearest` for the rule that loop implements.

// How many `z` units make one tile.
//
// **The metric, and this is its one spelling in a shader.** `light::Z_PER_TILE`
// is the Rust side of it; `blit.wesl` imports this rather than declaring a
// second, because `docs/lighting_rebuild.md`'s *One metric* is the line half of
// `docs/world_coordinates.md` is about.
const Z_PER_TILE: f32 = 11.0;

// A tile's width in virtual pixels — `camera::TILE_WIDTH`, which in this
// projection is also how many pixels a whole tile of `x - y` covers.
const TILE_WIDTH: f32 = 44.0;

// The direction every screen pixel's world line runs in, in world units — tiles
// on `x` and `y`, `z` units on `z`, and exactly `(1, 1, 1)` once `z` is divided
// into tiles.
//
// It points **towards the camera**: a point further along it draws to the same
// pixel and stands in front. So the surface a pixel shows is where the ray
// *leaves* a box, not where it enters one, and the three faces it can ever name
// are `max.x`, `max.y` and `max.z` — `crate::solid::Side`'s own three.
const VIEW: vec3<f32> = vec3<f32>(1.0, 1.0, Z_PER_TILE);

// How far outside a box, in tiles, a meeting may measure and still be a meeting
// rather than a miss. `crate::impostor::TANGENT`, and see it for the
// measurement: a ray reaching a box's own corner leaves two slabs at one `t` and
// the clamp below then subtracts two nearly-equal floats, which reads `3.5e-6`
// of a tile out rather than zero.
const TANGENT: f32 = 1.0e-4;

// Where the view ray through one sprite fragment starts: the point at the
// static's own base height.
//
// `across` is how far the fragment is right of the column its sprite is centred
// on and `down` how far below the row its tile's own centre projects to, both in
// virtual pixels — the two numbers the vertex stage already hands the fragment
// stage. **This is the inverse projection the flat case used to spell**, and
// that is the point rather than a coincidence: a floor's pixel is this ray at
// `t = 0`, and every other case was the same ray met with a plane guessed from a
// stance instead of measured against a box.
fn ray_from(tile: vec2<f32>, base: f32, across: f32, down: f32) -> vec3<f32> {
    // `u - v = across/22` and `u + v = down/22 + 1`, solved. Written over the
    // whole tile width rather than the half so that neither line divides twice.
    return vec3<f32>(
        tile.x + (across + down) / TILE_WIDTH + 0.5,
        tile.y + (down - across) / TILE_WIDTH + 0.5,
        base,
    );
}

// Where a fragment of a **billboard** is: the point of its own view ray on the
// vertical plane the sprite is drawn on.
//
// `docs/lighting_rebuild.md` phase 7, and the half of it that is not a choice.
// A mobile has no volume, so `meets` has nothing to meet and the pass fell back
// to a *point* — the middle of the tile, with the height running down the
// picture — which is the same point for every pixel of a screen row. A figure
// lit flat across, and `blit.wesl`'s `dither` turning the sample pattern by an
// angle that belongs to the position, so an eight-ray estimate lands in
// horizontal bands across it. Both are one cause.
//
// The plane stands through the tile's centre, contains the vertical, and runs
// along the screen's own `x` axis — `(1, -1)` in tiles, which is what `ray_from`
// moves along when only `across` changes. Its normal is therefore `(1, 1, 0)`,
// `VIEW`'s horizontal part: turned towards the camera, which is what a billboard
// is. The height falls out unchanged, `Z_PER_TILE / TILE_WIDTH` being `1 /
// Z_STEP` exactly — what the old point had wrong was `x` and `y` and nothing
// else.
//
// `impostor::billboard_at`, and the two are one formula.
fn billboard_at(tile: vec2<f32>, base: f32, across: f32, down: f32) -> vec3<f32> {
    return vec3<f32>(
        tile.x + 0.5 + across / TILE_WIDTH,
        tile.y + 0.5 - across / TILE_WIDTH,
        base - down * Z_PER_TILE / TILE_WIDTH,
    );
}

// The billboard plane's own normal — `(1, 1, 0)` normalised, `VIEW`'s
// horizontal part. `docs/lighting_rebuild.md` phase 7's "facing the camera"
// candidate; `impostor::billboard_normal`, and the two are one formula.
fn billboard_normal() -> vec3<f32> {
    return normalize(vec3<f32>(1.0, 1.0, 0.0));
}

// Where the view ray meets one box, and which of its faces that is.
struct Meeting {
    // The point, in world units, clamped into the box.
    at: vec3<f32>,
    // Which way the surface it met looks — one of `+x`, `+y`, `+z`.
    normal: vec3<f32>,
    // How far, in **tiles**, the ray passed outside the box before that clamp
    // pulled it back. At most `TANGENT` for a ray that genuinely went through
    // it; `hit` is that comparison spelled once. This is the number
    // `docs/lighting_rebuild.md` phase 6 asks its own "done when" to carry — a
    // sprite overhangs its own volume wherever the fitted box is narrower than
    // the art, and what matters is not that such pixels exist but how far out
    // they are.
    outside: f32,
    // How far along `VIEW` the meeting is, from `ray_from`'s own point. Only the
    // selection loop reads it: the box with the largest `t` is the one in front.
    t: f32,
}

// Whether the ray went through the box at all, as against passing beside it and
// being clamped onto it.
fn hit(met: Meeting) -> bool {
    return met.outside <= TANGENT;
}

// Where the view ray through `start` meets the box `lo..=hi`.
//
// Always answers. A ray that misses is answered with the point of the box
// nearest it and `Meeting::outside` is what says so — one expression rather than
// a branch, because at tangency the two cases are the same point and a branch
// there would be a seam.
//
// The face is the one whose *exit* comes first. **Ties go to `z`, then `y`, then
// `x`**, which is a decision and not an accident: two exits are equal exactly on
// an edge or a corner, and on a degenerate box — a lid, whose `z` slab is a
// point — they are equal over the whole of it. Preferring `z` is what makes a
// lid read as a lid all the way to its own rim. `crate::impostor::meets`.
fn meets(start: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> Meeting {
    // No parallel-axis branch: every component of `VIEW` is positive, so `lo` is
    // always the near end of a slab and `hi` the far one, and no division here
    // can be by zero. A degenerate box gives one `t` twice, which is a plane met
    // at a point and correct as it stands.
    let far = (hi - start) / VIEW;
    var exit = far.z;
    var normal = vec3<f32>(0.0, 0.0, 1.0);
    if far.y < exit {
        exit = far.y;
        normal = vec3<f32>(0.0, 1.0, 0.0);
    }
    if far.x < exit {
        exit = far.x;
        normal = vec3<f32>(1.0, 0.0, 0.0);
    }

    let at = start + exit * VIEW;
    let clamped = clamp(at, lo, hi);
    // In tiles, which means the `z` term is divided into them: a third of a tile
    // out sideways and a third of a tile out in height are the same miss, and
    // three `z` units are not.
    let miss = at - clamped;

    var met: Meeting;
    met.at = clamped;
    met.normal = normal;
    // A distance rather than the slab test's own `enter <= exit` predicate, and
    // the two cannot disagree because this one *is* the clamp above: a ray that
    // goes through the box is clamped by nothing and measures zero.
    met.outside = length(vec3<f32>(miss.x, miss.y, miss.z / Z_PER_TILE));
    met.t = exit;
    return met;
}
