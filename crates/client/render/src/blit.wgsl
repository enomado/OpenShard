// The world image, onto the viewport — and the lighting, on the way past.
//
// No vertex buffer: four corners derived from the vertex index, which is a
// triangle strip covering clip space. The render pass sets the viewport rect, so
// "clip space" is whatever rectangle the UI left free for the world.
//
// The lighting here is in *world* coordinates and not in this image's pixels:
// every fragment reads which tile its picture came from out of the place
// attachment (see `crate::place`) and is lit as that tile is lit. The screen
// folds height into `y` and a wall's sprite stands above the tile it occludes
// from, so nothing decided from a screen position can tell the lit face of a
// wall from the shadow behind it. `docs/lighting.md` is the argument.

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    // (0,0) (1,0) (0,1) (1,1), the same unit quad the world passes use.
    let corner = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u));
    var out: VertexOut;
    // Clip space is y-up and the image is y-down, so the vertical axis flips
    // here rather than in the sampling — one negation, in the place where the
    // convention actually changes.
    out.position = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
    out.uv = corner;
    return out;
}

@group(0) @binding(0) var world: texture_2d<f32>;
@group(0) @binding(1) var world_sampler: sampler;
// Which tile each pixel of `world` came from: (x, y, z + 128, kind). Never
// sampled — an integer texture has no filtering, and a place averaged with its
// neighbour would name a third tile nothing was drawn on.
@group(0) @binding(3) var place_of: texture_2d<u32>;
// What stands in the light's way, one texel a tile over the rectangle in
// `lighting.grid`: (z_bottom + 128, z_top + 128, opacity, present). See
// `crate::occlusion`.
@group(0) @binding(4) var occluders: texture_2d<u32>;
// What each of those tiles *is*, over the same rectangle: (sky, aperture, body,
// unused). Only the sky channel is written today. See `crate::occlusion`, and
// `docs/lighting_world.md` for why this is a second plane rather than four more
// channels of the cell above.
@group(0) @binding(5) var field: texture_2d<u32>;

// One flame. Three `vec4`s rather than nine fields, because a uniform array's
// stride is rounded up to 16 bytes either way and this is what the CPU writes:
//
//   place = (tile x, tile y, z, radius in tiles)
//   color = (r, g, b, intensity)
//   beam  = (axis x, axis y, axis z, cosine of the half-angle)
//
// A fire standing in the world lights every direction and writes a `beam` of
// `(0, 0, 0, -1)`: no cosine is below -1, so the test below is one comparison
// and the dot product is never reached. `crate::light::Beam`.
struct Light {
    place: vec4<f32>,
    color: vec4<f32>,
    beam: vec4<f32>,
};

// Sized to `Lighting::MAX`, and the two have to agree — this is a uniform
// array, so its length is fixed at compile time and a mismatch is a buffer
// wgpu rejects rather than a wrong picture.
const MAX_LIGHTS: u32 = 64u;

struct Lighting {
    // What a tile with an open column above it is multiplied by away from every
    // flame, then how many of `lights` are real. Scaled per fragment by the sky
    // channel of `field`, which is what makes the inside of a house darker than
    // the street with nothing in either — `crate::light::Ambient`.
    sky: vec4<f32>,
    // And what every tile gets, roof or no roof: the floor under the darkness,
    // so that an unlit room is deep rather than pure black. `(1,1,1,·)` sky with
    // a `(0,0,0,·)` ground over an empty grid is no lighting at all, and the
    // identity below is exact for it.
    ground: vec4<f32>,
    // The occlusion grid's rectangle: its lowest tile `x` and `y`, then its
    // width and height in tiles. A tile outside it occludes nothing, which is
    // right by construction — the grid is grown by the widest pool's reach, so
    // nothing outside it could shadow anything this frame draws.
    grid: vec4<i32>,
    // Which picture to draw: `crate::debug::View` in the first component, the
    // rest padding. Zero is the lit frame and every other value is a diagnostic
    // drawn from the values this pass lit it with — see `docs/lighting.md`,
    // decision 8. The numbers are pinned in a test on the Rust side.
    view: vec4<u32>,
    // Which way the sun is — `x` and `y` in tiles, `z` in tiles as well — and in
    // the fourth component the height above which nothing in this frame's grid
    // can stop it. A ray that has climbed past that is in the sky and stops
    // walking, which is what makes a daylit frame affordable.
    sun: vec4<f32>,
    // Its colour, then how much it adds. An intensity of zero is "no sun", and
    // it is the only thing tested: a night frame never enters the walk.
    sun_color: vec4<f32>,
    lights: array<Light, MAX_LIGHTS>,
};

// `crate::debug::View`, and the same numbers.
const VIEW_LIT: u32 = 0u;
const VIEW_PLACE: u32 = 1u;
const VIEW_KIND: u32 = 2u;
const VIEW_HEIGHT: u32 = 3u;
const VIEW_OCCLUDERS: u32 = 4u;
const VIEW_LIGHT: u32 = 5u;
const VIEW_SHADOW: u32 = 6u;
const VIEW_REACH: u32 = 7u;
const VIEW_SUN: u32 = 8u;
const VIEW_SKY: u32 = 9u;
const VIEW_FLAMES: u32 = 10u;

@group(0) @binding(2) var<uniform> lighting: Lighting;

// The fourth channel of the place attachment: the kind in the low two bits,
// then seven bits of tile-local `x` and seven of tile-local `y`. The world
// passes pack it; `crate::place` documents it.
const KIND_MASK: u32 = 3u;
const SUB_TILE_MASK: u32 = 127u;
const SUB_TILE: f32 = 127.0;

// `crate::place::Kind::Nothing`: no world pixel here. The Rust side pins the
// number in a test; this is the only other place it appears.
const KIND_NOTHING: u32 = 0u;

// The third channel is `z + 128` in its low eight bits and the sprite's stance in
// four of the eight above — `crate::place::STANCE_SHIFT`, and `statics.wgsl`
// writes it. The four faces are the only values this pass reads: a flat or
// faceless surface has no direction to be lit from, and a **corner** never
// arrives here at all — `statics.wgsl` has already resolved it to the face of the
// half the fragment was drawn on, because a corner's two surfaces are two halves
// of one picture and one pixel is on one of them.
const PLACE_STANCE_SHIFT: u32 = 8u;
const PLACE_STANCE_MASK: u32 = 15u;
const PLACE_Z_MASK: u32 = 255u;
const STANCE_FACE_NORTH: u32 = 2u;
const STANCE_FACE_EAST: u32 = 3u;
const STANCE_FACE_SOUTH: u32 = 4u;
const STANCE_FACE_WEST: u32 = 5u;

// Which way a face looks, in tiles. Zero for everything that has no face.
//
// The direction is the *drawn* one, which is the same thing: the art only ever
// draws the two faces an isometric camera can see — a wall stands on its tile's
// `y1` or `x1` edge and its picture is the surface turned towards the viewer —
// so a south face looks towards `+y` and an east face towards `+x`. North and
// west are five graphics out of 1197 and are here because the geometry has four
// edges. `docs/lighting.md`, step 15.
fn outward(stance: u32) -> vec2<f32> {
    switch stance {
        case STANCE_FACE_NORTH: { return vec2<f32>(0.0, -1.0); }
        case STANCE_FACE_EAST: { return vec2<f32>(1.0, 0.0); }
        case STANCE_FACE_SOUTH: { return vec2<f32>(0.0, 1.0); }
        case STANCE_FACE_WEST: { return vec2<f32>(-1.0, 0.0); }
        default: { return vec2<f32>(0.0); }
    }
}

// How wide the band is, in tiles, over which a flame passing behind a face stops
// lighting it.
//
// Not a step, for the reason a beam's rim is not a step: a hard edge is what the
// eye finds first, and a lamp walking past the end of a wall would switch its
// face off between two frames. A fifth of a tile is narrow enough that a wall is
// still plainly one-sided and wide enough that nothing pops.
const FACE_EDGE: f32 = 0.2;

// How much of a flame at `toward` reaches a surface facing `normal`: 1 in front
// of it, 0 behind it, and a gradient `FACE_EDGE` wide across the plane.
//
// **The one thing a fraction cannot say.** A wall's two faces are one tile and
// one plane — the same tile, the same fraction, the same height — so without a
// facing a torch inside a room lights the outside of the house exactly as
// brightly as the inside. That was reported from the client as a wall behaving
// like glass, and it is what this fixes.
//
// `light::faces`, and the two are one formula.
fn faces(normal: vec2<f32>, toward: vec2<f32>) -> f32 {
    return clamp(dot(normal, toward) / FACE_EDGE + 0.5, 0.0, 1.0);
}

// How many `z` units make one tile — `light::Z_PER_TILE`, and the same
// derivation: 44 virtual pixels of tile over 4 pixels a unit of height.
const Z_PER_TILE: f32 = 11.0;

// How many cells of the grid one ray may look at.
//
// One number for both rays, because they are one walk. A pool reaches nine tiles
// at the widest (`light::CAMPFIRE`) and a sunbeam's segment runs `MAX_SUN_TILES`,
// and `walk` visits every cell the segment crosses rather than a fixed number of
// samples — which on a diagonal is both axes' worth, so this is twice the longer
// of the two and a little. It is not meant to be reached: a fragment outside a
// pool never walks at all, and a sunbeam ends where it leaves the grid's ceiling.
// It exists so that a loop over data cannot be made unbounded by a radius
// somebody widens later. `light::MAX_WALK_STEPS`, and the two are one number.
const MAX_WALK_STEPS: i32 = 72;

// How far a ray must travel inside an occluding cell for that cell to stop all
// it can, in tiles.
//
// The walk knows how long the ray's crossing of each cell is, and using it is
// what makes a shadow's edge a gradient instead of a step: a ray that clips the
// corner of a wall tile passes almost all of its light, and one that crosses the
// tile squarely passes none. Without it a cell is all or nothing and the edge of
// every shadow lands exactly on a tile boundary — which is the blockiness the
// pools were accused of, arriving from the second of its three directions.
//
// It is not one length, though, because a shadow's edge is not equally soft
// everywhere: a flame is a body rather than a point, so an occluder close to the
// thing it shadows draws a sharp edge and a distant one draws a wide penumbra.
// The width of that penumbra is the flame's own size times `t / (1 - t)`, where
// `t` is how far along the ray the occluder is from the *lit* end — the ordinary
// similar-triangles answer, and it costs one division rather than a second ray.
// A wall a fragment is standing against is crisp; the doorpost four tiles away
// is soft, which is what a torch in a room actually looks like.
//
// `FLAME_SPREAD` is that size, in tiles, and the bounds keep the ends of the
// ratio finite. Invented here, like `occlusion::PANE` — no client file has a
// number for how big a flame is.
const FLAME_SPREAD: f32 = 1.0;
const SOFT_CROSSING_MIN: f32 = 0.05;
const SOFT_CROSSING_MAX: f32 = 0.7;

// The four sides of a tile, in the low bits of a cell's fourth channel, and the
// `PRESENT` bit above them. `crate::occlusion` writes them; the numbers are
// pinned there and nothing but a person reading both files can check they agree.
//
// A mask of zero is a **lid** — a floor, a roof — whose occlusion is entirely its
// `z` span and which no vertical side describes. All four is "it stands up and
// the art would not say which way", which is the whole-tile occluder decision 3
// started with and the answer everything falls back to.
const EDGE_NORTH: u32 = 1u;
const EDGE_EAST: u32 = 2u;
const EDGE_SOUTH: u32 = 4u;
const EDGE_WEST: u32 = 8u;
const EDGE_MASK: u32 = 15u;

// How much of a panel a ray pierces at height `z` runs into: 1 well inside the
// span it occupies, 0 well outside, and a gradient `tall` `z` units wide across
// its edges.
//
// The gradient is the vertical half of decision 14's penumbra, and it is all that
// is left of it: a flame is a body rather than a point, so a ray grazing the top
// of a wall is dimmed rather than switched.
//
// **The band is centred on the top edge and hangs below the bottom one**, which is
// the one asymmetry here and it is not cosmetic. A wall is based at the height of
// the ground it stands on, and the ray a person actually looks at — a torch and a
// floor, both at `z = 0` — runs exactly along that bottom edge. Centred there too,
// it would meet half the wall and every wall in the frame would pass half its
// light along the ground. Measured: a shut room's shadow read `0.378` against the
// `0.356` of its ambient before this line said so.
//
// `light::pierces`, and the two are one formula.
fn pierces(z: f32, low: f32, high: f32, tall: f32) -> f32 {
    let band = max(tall, 1.0e-3);
    let inside = min(z - low + band * 0.5, high - z);
    return clamp(inside / band + 0.5, 0.0, 1.0);
}

// How near two boundaries have to be, along the ray, for the ray to be crossing
// a **corner** rather than a side.
//
// A share of the whole segment, so a hundredth of a tile on a six-tile ray. What
// it decides is whether the walk looks at one of the two cells that share the
// corner or at both — see `walk`. It has to be well above the last bits of a
// float and well below anything a person could see, and it is both: the two ends
// of this comparison are the same arithmetic in two languages, and a
// ten-thousandth of a ray is a thirtieth of a pixel.
const CORNER_TIE: f32 = 1.0e-4;

// How much one cell stops a ray that crosses the sides in `crossed` at height
// `z`, where the cell is a panel. Zero for open ground, for a lid and for a panel
// on a side the ray does not go through.
//
// Split out of `walk` for the corner case, which has two cells to ask about and
// no crossing length to speak of. `light::panel_stop`.
fn panel_stop(cell: vec2<i32>, crossed: u32, z: f32, tall: f32) -> f32 {
    let stands = occluder_at(cell.x, cell.y);
    let sides = stands.w & EDGE_MASK;
    if stands.w == 0u || sides == 0u || (sides & crossed) == 0u {
        return 0.0;
    }
    return f32(stands.z) / 255.0 * pierces(z, f32(stands.x) - 128.0, f32(stands.y) - 128.0, tall);
}

// Which of a cell's sides are **the same wall the lit end is part of**, and
// therefore must not shadow it.
//
// A wall's face lies *on* the panel it is the face of — that is what decision 16
// is about — so a pixel of a wall is a point in the plane of its own tile's
// panel, and the panels of the tiles either side of it are in that same plane.
// Every ray leaving such a pixel along the wall therefore *grazes* the
// neighbours' panels, and whether it counts as crossing one is decided by the
// last bits of a float. It showed as a thin dark stroke down the wall at every
// tile seam, one per corner, on any wall lit by a lamp standing near it —
// the same corner ambiguity that used to draw bright spokes, arriving with the
// sign reversed.
//
// A run of wall is one surface, and no part of a surface shadows another part of
// it. So a panel on the same side of its tile as the lit end's own, on the same
// *line* — the same row for a north or south face, the same column for an east
// or west one — is not an occluder for this ray. Anything else about that cell
// still is: a wall tile that also carries the perpendicular face of a corner
// stops the ray on that face as it always did.
//
// `light::own_run`.
fn own_run(own: u32, cell: vec2<i32>, first: vec2<i32>) -> u32 {
    var line = 0u;
    if cell.y == first.y {
        line = line | EDGE_NORTH | EDGE_SOUTH;
    }
    if cell.x == first.x {
        line = line | EDGE_EAST | EDGE_WEST;
    }
    return own & line;
}

// What stands on one tile, or all zeros for open ground and for anything
// outside the grid.
fn occluder_at(x: i32, y: i32) -> vec4<u32> {
    let cell = vec2<i32>(x - lighting.grid.x, y - lighting.grid.y);
    if cell.x < 0 || cell.y < 0 || cell.x >= lighting.grid.z || cell.y >= lighting.grid.w {
        return vec4<u32>(0u);
    }
    return textureLoad(occluders, cell, 0);
}

// How much of the sky one tile can see, `0..=1`.
//
// Open sky outside the rectangle, which is `Occlusion::sky_at`'s own answer and
// the honest one in the direction that matters: the grid is grown by the widest
// pool's reach, so a tile outside it is one the frame does not draw, and
// answering "dark" there would put a band of night around every frame.
fn sky_at(x: i32, y: i32) -> f32 {
    let cell = vec2<i32>(x - lighting.grid.x, y - lighting.grid.y);
    if cell.x < 0 || cell.y < 0 || cell.x >= lighting.grid.z || cell.y >= lighting.grid.w {
        return 1.0;
    }
    return f32(textureLoad(field, cell, 0).x) / 255.0;
}

// The side of the neighbouring tile that touches this one's `side`.
//
// One line, and it is the whole of how the walk carries an edge across a
// boundary: the line a ray crosses is one cell's east and the next one's west.
fn opposite(side: u32) -> u32 {
    switch side {
        case EDGE_NORTH: { return EDGE_SOUTH; }
        case EDGE_SOUTH: { return EDGE_NORTH; }
        case EDGE_EAST: { return EDGE_WEST; }
        case EDGE_WEST: { return EDGE_EAST; }
        default: { return 0u; }
    }
}

// How much of a ray survives a segment of the world: 1 for nothing in the way,
// 0 for a wall.
//
// **One walk for both the flame and the sun.** They differ in their *ends* and in
// nothing else, so the ends are the parameters: `skip_last` is the flame's own
// tile, which must not shadow it (a sconce stands *on* a wall), and `spread` is
// how big the source is — see `FLAME_SPREAD`. A sunbeam passes `0` for both: its
// far end is a point in the sky rather than a tile, and the sun subtends half a
// degree, so its penumbra is the narrowest this walk draws.
//
// They were two functions until the sun's was measured. The sun's sampled one
// point per tile — the arrangement this walk replaced for flames — and a sealed
// roofed room therefore had a full-strength column of sunlight down the inside of
// its sunward wall: at noon a ray climbs 11 `z` a tile, so it crossed the wall's
// plane at 16 and was looked at, one tile later, at 22. It stepped over the top of
// a wall it had gone through. Two implementations of one idea, one of them a
// generation behind, is what that failure is made of; hence one.
//
// A grid traversal of the cells the segment crosses — every one, in order, with
// the length of each crossing. Not a fixed number of samples along the ray: at
// two tiles apart that was one interior point, so whether a fragment was in
// shadow was decided at the resolution of a tile and every shadow's edge was a
// staircase.
//
// The starting cell is always left out: the tile being lit must not shadow
// itself, which is what keeps a wall's own face the brightest thing near a torch.
//
// **A cell in between stops the ray only where the ray crosses the side the
// thing actually stands on.** That is decision 3 revised: an occluder used to be
// a whole tile, because nothing said which edge a wall was built on, and a lamp
// mounted on a house was therefore shadowed by the next tile of its own wall —
// the light could not run *along* the street it hung over. The walk already
// knows which boundary it crossed to enter a cell and which it will cross to
// leave, so the test costs two bit operations and no extra sampling.
//
// The height is interpolated along the ray, so a cell only stops the light where
// the ray actually passes through the span it occupies — and it is the *share*
// of the crossing that is inside that span which counts, so a ray grazing the
// top of a wall is dimmed rather than switched. That is what keeps a cellar's
// wall out of the street above it, without a step where the two meet.
fn walk(start: vec3<f32>, finish: vec3<f32>, skip_last: bool, spread: f32) -> f32 {
    let delta = finish - start;
    let ground = length(delta.xy);
    if ground < 1.0e-6 {
        // Straight up or down: the only cells on the line are the exempt ones,
        // and there is no direction to walk in.
        return 1.0;
    }
    let lit = start;
    let first = vec2<i32>(i32(floor(start.x)), i32(floor(start.y)));
    let last = vec2<i32>(i32(floor(finish.x)), i32(floor(finish.y)));
    var cell = first;
    // Which sides the lit end's *own* tile has a wall on. What it is for is
    // `own_run` below: a wall does not shadow the rest of the wall it is part of.
    let own = occluder_at(first.x, first.y).w & EDGE_MASK;
    // Which way each axis steps, how much of the whole segment one tile of it is
    // worth, and how far along the segment the first boundary is. An axis the
    // ray does not move along never reaches its boundary, which is what the
    // enormous `t` says.
    let toward = vec2<i32>(select(-1, 1, delta.x >= 0.0), select(-1, 1, delta.y >= 0.0));
    var per_tile = vec2<f32>(1.0e30, 1.0e30);
    var boundary = vec2<f32>(1.0e30, 1.0e30);
    if abs(delta.x) > 1.0e-6 {
        per_tile.x = 1.0 / abs(delta.x);
        let ahead = select(lit.x - floor(lit.x), floor(lit.x) + 1.0 - lit.x, delta.x >= 0.0);
        boundary.x = ahead * per_tile.x;
    }
    if abs(delta.y) > 1.0e-6 {
        per_tile.y = 1.0 / abs(delta.y);
        let ahead = select(lit.y - floor(lit.y), floor(lit.y) + 1.0 - lit.y, delta.y >= 0.0);
        boundary.y = ahead * per_tile.y;
    }

    var entered = 0.0;
    var through = 1.0;
    // Which side of the current cell the ray came in through. Nothing for the
    // first cell, where the ray begins inside — and that cell is exempt anyway.
    var entry = 0u;
    for (var i = 0; i < MAX_WALK_STEPS; i = i + 1) {
        let next = min(boundary.x, boundary.y);
        let leaves = min(next, 1.0);
        // And which side it will leave through: the boundary about to be
        // crossed, named by the direction of travel. Nothing where the segment
        // ends inside this cell — which for a flame is its own tile.
        let out_by_x = boundary.x < boundary.y;
        var exit = 0u;
        if next < 1.0 {
            if out_by_x {
                exit = select(EDGE_WEST, EDGE_EAST, toward.x > 0);
            } else {
                exit = select(EDGE_NORTH, EDGE_SOUTH, toward.y > 0);
            }
        }
        let stands = occluder_at(cell.x, cell.y);
        let sides = stands.w & EDGE_MASK;
        // **Neither end of the ray is shadowed by the tile it is on.**
        //
        // The lit end, so that a wall's own face stays the brightest thing beside
        // a torch: a pixel of a wall claims a fraction clamped *inside* its tile
        // whichever face of the wall it is on — the two faces are one tile — so
        // testing its own panel would darken whichever of them the flame is not
        // behind, and there is no way to know which that is.
        //
        // And the flame's end (`skip_last`), because a sconce is mounted *on* a
        // wall. This was tried the other way for one commit — the flame sits at
        // its tile's centre, which is inside the panel, so a ray leaving it does
        // cross the wall — and the picture is what settled it: every lamp in
        // Britain is on a building, so the city came out with its walls lit from
        // inside and not one pool of light on any street. A lamp that lights
        // nothing is a worse answer than a lamp that lights both sides of its own
        // wall, which is the defect this keeps and `docs/lighting.md` names.
        let exempt = (cell.x == first.x && cell.y == first.y)
            || (skip_last && cell.x == last.x && cell.y == last.y);
        if !exempt && stands.w != 0u {
            let low = f32(stands.x) - 128.0;
            let high = f32(stands.y) - 128.0;
            let opacity = f32(stands.z) / 255.0;
            // How soft this cell's own edge is: the penumbra a source of this
            // size casts from this far along the ray. See `FLAME_SPREAD`; a
            // `spread` of zero is a point source, and the clamp below leaves it
            // the narrowest edge the walk draws.
            let middle = (entered + leaves) * 0.5;
            let soft = clamp(
                spread * middle / max(1.0 - middle, 1.0e-3),
                SOFT_CROSSING_MIN,
                SOFT_CROSSING_MAX,
            );
            var stopped = 0.0;
            if sides == 0u || sides == EDGE_MASK {
                // A **body** — a lid (a floor, a roof, a plank) or a whole tile
                // that stands up and whose art would not say which way (a corner,
                // a post, a tree). Either way it is a solid the ray travels
                // *through*, so what it stops is scaled by how far the ray ran
                // inside the span it occupies.
                //
                // All four sides has to be here and not below with the panels,
                // and the sun is what says so: a roof is a slab five `z` deep, and
                // a ray at 45° that entered its cell at 19 and left at 22 pierces
                // neither side inside the span while passing straight through the
                // middle of it. That is the "stepped over the top of a wall"
                // failure `docs/lighting.md`'s backlog names, arriving from the
                // other direction — and it lit the floor of a sealed house.
                //
                // Which is why the length stays and the pierce below is taken
                // *beside* it rather than instead of it. See decision 24.
                let entering = lit.z + delta.z * entered;
                let leaving = lit.z + delta.z * leaves;
                let bottom = min(entering, leaving);
                let top = max(entering, leaving);
                var share = 0.0;
                if top - bottom > 1.0e-6 {
                    share = max(0.0, min(top, high) - max(bottom, low)) / (top - bottom);
                } else if bottom >= low && bottom <= high {
                    // A level ray: it is inside the span or it is not, and there
                    // is no length of it to take a share of.
                    share = 1.0;
                }
                let crossed = (leaves - entered) * ground * share;
                stopped = opacity * clamp(crossed / soft, 0.0, 1.0);
                // And a thing that **stands up** is a surface on every side of
                // its tile as well as a solid inside it, so the sides the ray is
                // crossed by are pierced too, and the larger of the two answers
                // is taken. Decision 24, and it is decision 18's own sentence
                // arriving at the answer everything falls back to.
                //
                // The length has to stay: it is what keeps a slab five `z` deep
                // opaque to a ray that pierces neither of its sides while going
                // straight through the middle, which is the roof case above. What
                // the pierce adds is the sliver — a ray clipping the corner of a
                // whole-tile occluder leaves it sideways over almost no length, so
                // `crossed / soft` rounded to nothing and the ray went through a
                // house corner into the room behind it. That is exactly the spoke
                // decision 18 named, surviving where a run of wall has to turn:
                // `facing` refuses a corner graphic, a refused graphic is all four
                // sides, and all four sides was the one branch still scaled by a
                // length.
                //
                // A lid names no side, so `sides == 0u` skips this entirely: a
                // floor has no vertical surface for a ray to pierce.
                if sides == EDGE_MASK {
                    let tall = soft * Z_PER_TILE;
                    let stops = EDGE_MASK & ~own_run(own, cell, first);
                    if (stops & entry) != 0u {
                        stopped = max(stopped, opacity * pierces(lit.z + delta.z * entered, low, high, tall));
                    }
                    if (stops & exit) != 0u {
                        stopped = max(stopped, opacity * pierces(lit.z + delta.z * leaves, low, high, tall));
                    }
                }
            } else {
                // A **panel** — a wall standing on one side of its tile. It is a
                // *surface*, and what a surface does to a ray is decided where the
                // ray pierces it: at a point, at a height, once. Not by how far
                // the ray ran inside the cell.
                //
                // That distinction is the whole of this branch and it is not
                // pedantry. Scaling a panel by the length of the crossing lets a
                // ray that clips the corner between two panels through *both* of
                // them: it leaves the first cell sideways, so that cell's own
                // face was never crossed, and it enters the second across the
                // corner, where the crossing is a hair long and `crossed / soft`
                // rounds to nothing. The result is a fan of bright spokes out of
                // any lamp near a wall, one per tile corner, which is exactly what
                // it looked like — a wall with no hole in it leaking light in
                // stripes. See `docs/lighting.md`, decision 18.
                let tall = soft * Z_PER_TILE;
                // Less whatever of this cell is the same run of wall the lit end
                // stands in — see `own_run`, and the seam it was drawing.
                let stops = sides & ~own_run(own, cell, first);
                if (stops & entry) != 0u {
                    stopped = max(stopped, opacity * pierces(lit.z + delta.z * entered, low, high, tall));
                }
                if (stops & exit) != 0u {
                    stopped = max(stopped, opacity * pierces(lit.z + delta.z * leaves, low, high, tall));
                }
            }
            through = through * (1.0 - stopped);
            if through <= 0.004 {
                // Under a byte's worth of light: nothing further can matter,
                // and a wall is the common case.
                return 0.0;
            }
        }
        if next >= 1.0 {
            break;
        }
        entered = next;
        // Which sides the neighbours touch this corner or this boundary by. The
        // ray moving east leaves through an east side and enters a west one.
        let enter_x = select(EDGE_EAST, EDGE_WEST, toward.x > 0);
        let enter_y = select(EDGE_SOUTH, EDGE_NORTH, toward.y > 0);
        if abs(boundary.x - boundary.y) <= CORNER_TIE {
            // **A corner.** Four tiles meet at the point the ray is leaving by,
            // and the two the walk does not step into are as much in the way as
            // the one it does: a ray running the diagonal of a room's corner used
            // to pass between two walls that touch there, because whichever cell
            // the comparison picked second was crossed over no length at all and
            // stopped nothing. So both are asked, at the corner's own height, and
            // then the walk steps diagonally past them into the cell beyond.
            //
            // The exemptions are the same two as above and are repeated rather
            // than shared, because the cells are: a flame standing at a corner of
            // its own tile must not be shadowed by the tile it stands on.
            let by_x = vec2<i32>(cell.x + toward.x, cell.y);
            let by_y = vec2<i32>(cell.x, cell.y + toward.y);
            let z = lit.z + delta.z * next;
            let tall = clamp(
                spread * next / max(1.0 - next, 1.0e-3),
                SOFT_CROSSING_MIN,
                SOFT_CROSSING_MAX,
            ) * Z_PER_TILE;
            var corner = 0.0;
            if !(by_x.x == first.x && by_x.y == first.y)
                && !(skip_last && by_x.x == last.x && by_x.y == last.y) {
                let crossed = (enter_x | opposite(enter_y)) & ~own_run(own, by_x, first);
                corner = max(corner, panel_stop(by_x, crossed, z, tall));
            }
            if !(by_y.x == first.x && by_y.y == first.y)
                && !(skip_last && by_y.x == last.x && by_y.y == last.y) {
                let crossed = (enter_y | opposite(enter_x)) & ~own_run(own, by_y, first);
                corner = max(corner, panel_stop(by_y, crossed, z, tall));
            }
            through = through * (1.0 - corner);
            if through <= 0.004 {
                return 0.0;
            }
            cell = vec2<i32>(by_x.x, by_y.y);
            boundary.x = boundary.x + per_tile.x;
            boundary.y = boundary.y + per_tile.y;
            // The cell beyond is entered by *both* of the sides that meet at the
            // corner, so a wall on either of them stops the ray there too.
            entry = enter_x | enter_y;
            continue;
        }
        // The neighbour's own entry is this cell's exit seen from the other
        // side: leaving east is entering west.
        entry = opposite(exit);
        // Into the neighbour across whichever boundary is nearer.
        if out_by_x {
            cell.x = cell.x + toward.x;
            boundary.x = boundary.x + per_tile.x;
        } else {
            cell.y = cell.y + toward.y;
            boundary.y = boundary.y + per_tile.y;
        }
    }
    return through;
}

// How far in from the rim of a beam its edge finishes softening, as a share of
// the way from the rim to the axis. `light::BEAM_EDGE`, and the two are one
// number — a cone with a hard rim reads as a stencil laid over the scene rather
// than as light.
const BEAM_EDGE: f32 = 0.25;

// How much of a beamed flame escapes it in every other direction: a hand is not
// a shutter, and a cone with nothing outside it leaves the character carrying the
// light as the only black thing in the frame. `light::BEAM_SPILL`, and the two
// are one number.
const BEAM_SPILL: f32 = 0.25;

// How much of a flame's beam falls on a spot `offset` away from it — `x` and `y`
// in tiles and `z` in tiles as well, pointing *from* the flame *to* the spot.
//
// `light::Beam::lights`, arithmetic for arithmetic, and the parity test of
// `docs/lighting.md`'s decision 9 is what holds the two together. The smoothstep
// is written out rather than called for the same reason: two texts that mean to
// be one polynomial should be one polynomial.
fn cone(beam: vec4<f32>, offset: vec3<f32>) -> f32 {
    let distance = length(offset);
    if distance < 1.0e-6 {
        // A spot at the flame itself is inside every beam: there is no direction
        // from a point to itself, and a lantern's own tile is not the place to
        // start refusing light.
        return 1.0;
    }
    let along = dot(offset / distance, beam.xyz);
    let inner = beam.w + (1.0 - beam.w) * BEAM_EDGE;
    let t = clamp((along - beam.w) / max(inner - beam.w, 1.0e-6), 0.0, 1.0);
    return BEAM_SPILL + (1.0 - BEAM_SPILL) * t * t * (3.0 - 2.0 * t);
}

// How far along the ground one sunbeam may run, in tiles. `light::MAX_SUN_TILES`.
//
// The bound that matters is the ceiling below — a ray that has climbed above
// everything in the grid is looking at sky and stops there, which over open
// ground is two or three tiles. This is what is left for a sun so low that it
// never climbs out: a shadow thirty-two tiles long is already longer than any
// frame, and an unbounded segment would be a walk with no end at sunset.
const MAX_SUN_TILES: f32 = 32.0;

// How much of the sun reaches a fragment: 1 in the open, 0 in a wall's shadow.
//
// The same walk as a flame's, which is the whole of it now: what the sun has
// instead of a position is a *direction*, so the far end of the segment is
// computed rather than given. It is the point at which the ray leaves the
// grid's ceiling — `lighting.sun.w`, the height above which nothing this frame
// holds can stop anything — because from there on the ray is in the sky. A
// fragment already above the ceiling is in open air and the walk never starts.
//
// `false` and `0.0`: there is no far tile to exempt, the far end being a point in
// the sky, and the sun is half a degree wide, so its penumbra is the narrowest
// `walk` draws. The fragment's own tile is skipped there as it is for a flame,
// which is what lights a wall on the side the sun is on instead of shadowing it
// with itself.
fn sunlight(at: vec3<f32>) -> f32 {
    let horizontal = length(lighting.sun.xy);
    if horizontal < 1.0e-6 {
        // Straight overhead: nothing but the fragment's own tile is in the way,
        // and that one is exempt.
        return 1.0;
    }
    // One tile of ground a unit, so `z` climbs by the sun's own slope.
    let step = vec3<f32>(
        lighting.sun.x / horizontal,
        lighting.sun.y / horizontal,
        lighting.sun.z / horizontal * Z_PER_TILE,
    );
    var tiles = MAX_SUN_TILES;
    if step.z > 1.0e-6 {
        tiles = min(tiles, (lighting.sun.w - at.z) / step.z);
    }
    if tiles <= 0.0 {
        // Above everything the grid holds — or the grid holds nothing at all,
        // which arrives here as a ceiling below every fragment there is.
        return 1.0;
    }
    return walk(at, at + step * tiles, false, 0.0);
}

// Where the light view stops being the multiplier and starts being a curve.
//
// The view's problem is that the interesting range is not `0..=1`. A night
// ambient is `0.36`, a torch adds `0.95` on top of it and a lit day is past
// `1.1`, so a clamp — which is what this was — painted the middle of every pool
// one flat white disc, and the middle is exactly the part of a flame's shape
// nothing else shows: the lit frame multiplies the art by it, and the art is
// dark, so the clipping never appears there.
//
// A tone map over the whole range is the other wrong answer, and it was tried:
// pulling `0.36` down to `0.26` and `1.27` to `0.56` leaves a picture where the
// lamp does not read as a lamp. So the curve is identity below `KNEE` and an
// exponential approach to `1.0` above it — everything a person reads as a level
// is untouched, and only what was being clipped is bent. Monotone and unbounded:
// nothing reaches white, so a step or a seam anywhere above the knee is still a
// step, and two flames overlapping still differ from one.
const KNEE: f32 = 0.6;

fn knee(value: f32) -> f32 {
    if value <= KNEE {
        return value;
    }
    let headroom = 1.0 - KNEE;
    return KNEE + headroom * (1.0 - exp(-(value - KNEE) / headroom));
}

// One of the pass's own values, as a colour.
//
// Every argument here is something the lit path already computed — none of these
// views measures anything of its own, which is what makes them evidence about
// *this* frame rather than about a second frame drawn like it. See
// `crate::debug::View` for what each one is for.
fn debug_color(
    view: u32,
    place: vec4<u32>,
    at: vec3<f32>,
    sub: vec2<f32>,
    lit: vec3<f32>,
    flames: vec3<f32>,
    reached: u32,
    nearest: f32,
    nearest_through: f32,
    sun_through: f32,
    share: f32,
) -> vec3<f32> {
    if view == VIEW_SKY {
        // The sky field on the ground it is a field of: white under open air,
        // black under a roof, and a gradient across a doorway. Drawn over the
        // picture rather than as a wireframe of the boxes, because the failure
        // this field actually has is a tile that is *wrongly open* — an eave that
        // did not cover the floor under it — and a box is drawn for what stands,
        // not for what does not. `docs/lighting_world.md`'s backlog says so.
        return vec3<f32>(share, share, 0.15 + share * 0.85);
    }
    if view == VIEW_SUN {
        // How much of the sun arrived: white in the open, black in a wall's
        // shadow, and the sun's own colour where a pane dimmed it rather than
        // stopping it — which is the picture a person looking for a lit patch on
        // a floor is actually after.
        if lighting.sun_color.w <= 0.0 {
            return vec3<f32>(0.0, 0.0, 0.35);
        }
        return vec3<f32>(sun_through);
    }
    if view == VIEW_PLACE {
        // A checkerboard of tiles with the sub-tile fraction laid over it: the
        // squares say which tile a pixel claims, and the gradient inside each
        // one says the fraction is being written at all. A wall reads as one
        // flat square with its sprite standing out of it — which is exactly the
        // property the lighting depends on.
        let checker = f32((place.x + place.y) & 1u) * 0.35 + 0.15;
        return vec3<f32>(sub.x, sub.y, checker);
    }
    if view == VIEW_KIND {
        let kind = place.w & KIND_MASK;
        if kind == 1u {
            return vec3<f32>(0.20, 0.65, 0.30);  // land
        }
        if kind == 2u {
            return vec3<f32>(0.25, 0.45, 1.00);  // a static or an item
        }
        return vec3<f32>(1.00, 0.40, 0.15);      // a mobile
    }
    if view == VIEW_HEIGHT {
        // A ramp over the `z` a map actually uses, with a band every tile's
        // worth of height: the ramp is for reading a slope, the bands are for
        // counting storeys, and eleven units is one tile — the ratio that
        // decides whether a cellar's flame reaches the street.
        let ramp = clamp((at.z + 64.0) / 128.0, 0.0, 1.0);
        let band = fract(at.z / Z_PER_TILE);
        return vec3<f32>(ramp, band * 0.8, 1.0 - ramp);
    }
    if view == VIEW_OCCLUDERS {
        let cell = occluder_at(i32(floor(at.x)), i32(floor(at.y)));
        if cell.w == 0u {
            return vec3<f32>(0.06, 0.06, 0.10);
        }
        let opacity = f32(cell.z) / 255.0;
        let bottom = f32(cell.x) - 128.0;
        let top = f32(cell.y) - 128.0;
        // Red where this pixel's own height is inside the span the tile stops
        // light at, blue where the tile occludes but not here. The second colour
        // is the one worth having: a wall of an upper storey standing over an
        // open street is blue, and a shadow appearing under it would be the bug.
        if at.z >= bottom && at.z <= top {
            return vec3<f32>(0.25 + 0.75 * opacity, 0.05, 0.05);
        }
        return vec3<f32>(0.05, 0.10, 0.25 + 0.55 * opacity);
    }
    if view == VIEW_LIGHT {
        // The lighting with the art thrown away: the pools' own shapes, where a
        // seam or a step has nothing to hide behind. `KNEE` is why it is not a
        // clamp and not a tone map either.
        return vec3<f32>(knee(lit.r), knee(lit.g), knee(lit.b));
    }
    if view == VIEW_FLAMES {
        // And the same pools with the ambient taken out, on black: what the
        // *flames* added and nothing else. No curve here — a flame's whole range
        // is already `0..=1` and a little over, and bending it is what makes the
        // view above unable to answer whether a pool has a shape at all.
        return clamp(flames, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    if view == VIEW_SHADOW {
        // Blue for "no flame reaches here at all", which is a different fact
        // from "a wall is in the way" and the one people confuse.
        if nearest >= 1.0 {
            return vec3<f32>(0.0, 0.0, 0.35);
        }
        return vec3<f32>(nearest_through);
    }
    // VIEW_REACH: how many flames got through, green through red, and the same
    // blue as above for none.
    if reached == 0u {
        return vec3<f32>(0.0, 0.0, 0.35);
    }
    let many = min(f32(reached) / 4.0, 1.0);
    return vec3<f32>(many, 1.0 - many * 0.7, 0.15);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Straight through: both sides are `Rgba8Unorm` and this crate never
    // converts colour. Whatever filtering happens is the sampler's, chosen by
    // the caller from which way the zoom went.
    let color = textureSample(world, world_sampler, in.uv);

    // Which tile this pixel's picture came from. `textureLoad` and not a
    // sampler, and the texel is the nearest one whatever the zoom: a place is an
    // identity, and half of one is nobody.
    let size = vec2<f32>(textureDimensions(place_of));
    let texel = vec2<i32>(clamp(in.uv * size, vec2<f32>(0.0), size - vec2<f32>(1.0)));
    let place = textureLoad(place_of, texel, 0);

    let view = lighting.view.x;

    // Nothing was drawn here — the cleared background, or the letters over a
    // speaker's head, which are a message and not a thing standing in the
    // street. Neither is lit and neither is dimmed. See `crate::place::Kind`.
    //
    // The diagnostics leave it alone too, and deliberately: a view that painted
    // the background would make the world's own silhouette hard to find, and
    // every one of these is read by comparing a shape against the picture it
    // came from. The one exception is the kind view, whose whole subject is
    // which pixels are nothing.
    if (place.w & KIND_MASK) == KIND_NOTHING {
        if view == VIEW_KIND {
            return vec4<f32>(0.0, 0.0, 0.0, color.a);
        }
        return color;
    }

    // Where in the world this pixel is, tile *and* the fraction of it the world
    // pass wrote. The fraction is what makes a pool a gradient: without it every
    // pixel of a tile is the same distance from the flame, and a tile is 44
    // pixels of ground that would all be one brightness with a step at its edge.
    let sub = vec2<f32>(
        f32((place.w >> 2u) & SUB_TILE_MASK) / SUB_TILE,
        f32((place.w >> 9u) & SUB_TILE_MASK) / SUB_TILE,
    );
    let at = vec3<f32>(f32(place.x) + sub.x, f32(place.y) + sub.y, f32(place.z & PLACE_Z_MASK) - 128.0);
    // And which way the surface drawn here looks, where it is a wall's face. Zero
    // for the ground, for a mobile and for anything standing up whose art would
    // not name an edge — none of those has a side to be lit from.
    let normal = outward((place.z >> PLACE_STANCE_SHIFT) & PLACE_STANCE_MASK);

    // The ambient this *tile* has: the sky's share of it scaled by how much of
    // the sky the column over the tile can see, plus the floor everything gets.
    // `docs/lighting_world.md`, decision 1, and it is the whole of why a room is
    // darker than the road outside it before anything burns.
    let share = sky_at(i32(floor(at.x)), i32(floor(at.y)));
    var lit = lighting.ground.rgb + lighting.sky.rgb * share;
    // What the diagnostics are made of, gathered while the frame is lit rather
    // than computed a second time: how many flames actually reached this
    // fragment, and what the *nearest* of them lost on the way. Nearest and not
    // an average, because a shadow view is read as "is this pixel behind
    // something", and the flame that answers that is the one lighting it most.
    var reached = 0u;
    var nearest = 1.0e9;
    var nearest_through = 0.0;
    // What the flames added, apart from the ambient they were added to. Only
    // `VIEW_FLAMES` reads it, and it is accumulated here rather than recomputed
    // there for the reason every one of these values is: a diagnostic that lit its
    // own copy of the frame would answer about that copy.
    var flames = vec3<f32>(0.0);
    let count = u32(lighting.sky.w);
    for (var i = 0u; i < count; i = i + 1u) {
        let light = lighting.lights[i];
        let to = light.place.xyz;
        // All three axes in one unit: `z` is divided into tiles, so a flame
        // reaches as far up and down as it does sideways. Without it the screen's
        // own folding comes back — a cellar lighting the street.
        let offset = vec3<f32>(to.x - at.x, to.y - at.y, (to.z - at.z) / Z_PER_TILE);
        let d = length(offset) / max(light.place.w, 0.001);
        if d >= 1.0 {
            // Outside the pool entirely, which most fragments are for most
            // flames: no ray is walked and nothing is sampled. This is why the
            // grid walk below is affordable at all.
            continue;
        }
        // Which way it is pointing, where it points one way at all. A dot
        // product and a branch: a fire standing in the world writes a rim below
        // every cosine there is and never reaches the arithmetic.
        //
        // `offset` runs from the fragment to the flame and with `z` already
        // divided into tiles, which is the space the axis is stated in; a beam's
        // axis runs the other way, so the sign flips here.
        var lit_by = 1.0;
        if light.beam.w > -1.0 {
            lit_by = cone(light.beam, -offset);
        }
        // And whether the flame is on the side this surface looks at. A wall is
        // one-sided and its two faces are one tile, so this is the only thing that
        // keeps a torch in a room from lighting the outside of the house.
        //
        // Geometry and nothing else. This used to exempt a flame standing in the
        // wall's own row or column, because a lamp mounted on a house sits at its
        // tile's centre — behind the plane of the face it is bolted to — and
        // testing it blacked out the very wall it hangs on. But a *line* is the
        // whole length of a street: a lamp post standing south of a house is in
        // the column of its east wall, and that wall's faces came out fully lit
        // from a flame half a tile behind their plane, for the length of the run.
        // Reported from the client at Britain's `(1441, 1692)`. What answers it is
        // moving the mounted flame instead of excusing it — `light::mounted_at`,
        // and `docs/lighting.md`'s decision 26.
        if any(normal != vec2<f32>(0.0)) {
            lit_by = lit_by * faces(normal, vec2<f32>(to.x - at.x, to.y - at.y));
        }
        // The flame's own tile is exempt, and a flame is a body a tile wide:
        // `walk`'s two parameters, and the only two things that make this ray
        // different from the sun's.
        let through = walk(at, vec3<f32>(to.x, to.y, to.z), true, FLAME_SPREAD);
        // Recorded before the shadow is tested, so that a fragment inside a pool
        // and behind a wall is *nearest, stopped* rather than indistinguishable
        // from open ground nothing reaches. That difference is the one the whole
        // shadow view exists to show.
        if d < nearest {
            nearest = d;
            nearest_through = through;
        }
        if through <= 0.0 {
            continue;
        }
        reached = reached + 1u;
        // Linear distance, squared falloff: `1 - d` alone gives a cone with a
        // visible straight edge in the gradient, and an inverse-square law with
        // no cutoff has no radius at all and tints the whole frame. This is the
        // soft pool with a hard end — the shape the reference isometrics draw.
        let fall = 1.0 - d;
        let added = light.color.rgb * (light.color.w * fall * fall * through * lit_by);
        lit = lit + added;
        flames = flames + added;
    }

    // And the sun, which is one direction rather than a place: no distance and no
    // falloff, only whether anything stands between this pixel and the sky.
    var sun_through = 0.0;
    if lighting.sun_color.w > 0.0 {
        sun_through = sunlight(at);
        lit = lit + lighting.sun_color.rgb * (lighting.sun_color.w * sun_through);
    }

    if view != VIEW_LIT {
        return vec4<f32>(
            debug_color(
                view, place, at, sub, lit, flames, reached, nearest, nearest_through, sun_through,
                share,
            ),
            color.a,
        );
    }

    // Multiplicative, so a flame brightens whatever is under it — ground, wall,
    // the body standing in it — rather than laying a coloured film over it. A
    // pixel the art left black stays black, which is what makes the light look
    // like it is falling on the scene instead of floating above it.
    //
    // With no lights and a white ambient every channel is multiplied by exactly
    // `1.0` and this is the copy it has always been. `light::Lighting::NONE` is
    // that case, and a frame test asserts the whole surface texel for texel.
    // Alpha carried through untouched, not written as `1.0`: this pass copies
    // an image and lighting is about how bright a pixel is, not about whether it
    // is there. The world passes already write an opaque frame; a surface whose
    // alpha this decided would differ from the world image in a channel nobody
    // lit, which is exactly what the copy test reads.
    return vec4<f32>(min(color.rgb * lit, vec3<f32>(1.0)), color.a);
}
