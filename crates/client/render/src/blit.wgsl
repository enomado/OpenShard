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

// How many `z` units make one tile — `light::Z_PER_TILE`, and the same
// derivation: 44 virtual pixels of tile over 4 pixels a unit of height.
const Z_PER_TILE: f32 = 11.0;

// How many cells of the grid one shadow ray may look at.
//
// A pool reaches nine tiles at the widest (`light::CAMPFIRE`), and the walk
// below visits every cell the ray crosses rather than a fixed number of samples
// — which on a diagonal is both axes' worth, so the bound is twice the radius
// and a little. A fragment further away than the radius never gets here at all;
// this exists so that a loop over data cannot be made unbounded by a radius
// somebody widens later. `light::MAX_RAY_STEPS`, and the two are one number.
const MAX_RAY_STEPS: i32 = 24;

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

// How much of a flame reaches a tile: 1 for nothing in the way, 0 for a wall.
//
// A grid traversal of the cells between the two — every cell the segment
// actually crosses, in order, with the length of each crossing. Not a fixed
// number of samples along the ray: at two tiles apart that was one interior
// point, so whether a fragment was in shadow was decided at the resolution of a
// tile and every shadow's edge was a staircase.
//
// Both end cells are left out: the flame's own tile must not shadow it (a sconce
// stands *on* a wall), and the tile being lit must not shadow itself, which is
// what keeps a wall's own face the brightest thing near a torch.
//
// The height is interpolated along the ray, so a cell only stops the light where
// the ray actually passes through the span it occupies — and it is the *share*
// of the crossing that is inside that span which counts, so a ray grazing the
// top of a wall is dimmed rather than switched. That is what keeps a cellar's
// wall out of the street above it, without a step where the two meet.
fn reaches(lit: vec3<f32>, flame: vec3<f32>) -> f32 {
    let delta = flame - lit;
    let ground = length(delta.xy);
    if ground < 1.0e-6 {
        // Straight up or down: the only cells on the line are the two exempt
        // ones, and there is no direction to walk in.
        return 1.0;
    }
    let first = vec2<i32>(i32(floor(lit.x)), i32(floor(lit.y)));
    let last = vec2<i32>(i32(floor(flame.x)), i32(floor(flame.y)));
    var cell = first;
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
    for (var i = 0; i < MAX_RAY_STEPS; i = i + 1) {
        let next = min(boundary.x, boundary.y);
        let leaves = min(next, 1.0);
        let exempt = (cell.x == first.x && cell.y == first.y)
            || (cell.x == last.x && cell.y == last.y);
        if !exempt {
            let stands = occluder_at(cell.x, cell.y);
            if stands.w != 0u {
                let low = f32(stands.x) - 128.0;
                let high = f32(stands.y) - 128.0;
                // The ray's own height over this crossing, against the span the
                // tile occupies: what counts is how much of the two overlap.
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
                // How soft this cell's own edge is: the penumbra of an occluder
                // this far along the ray. See `FLAME_SPREAD`.
                let middle = (entered + leaves) * 0.5;
                let soft = clamp(
                    FLAME_SPREAD * middle / max(1.0 - middle, 1.0e-3),
                    SOFT_CROSSING_MIN,
                    SOFT_CROSSING_MAX,
                );
                let stopped = f32(stands.z) / 255.0 * clamp(crossed / soft, 0.0, 1.0);
                through = through * (1.0 - stopped);
                if through <= 0.004 {
                    // Under a byte's worth of light: nothing further can matter,
                    // and a wall is the common case.
                    return 0.0;
                }
            }
        }
        if next >= 1.0 {
            break;
        }
        entered = next;
        // Into the neighbour across whichever boundary is nearer. A tie is a
        // corner: either order visits both cells, and the one taken second is
        // crossed over a zero length and stops nothing — which is the diagonal
        // gap `docs/lighting.md` names, kept deliberately rather than closed by
        // an accident of which comparison ran first.
        if boundary.x < boundary.y {
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

// How many tiles of grid one sunbeam's ray may walk. `light::MAX_SUN_STEPS`.
const MAX_SUN_STEPS: i32 = 32;

// How much of the sun reaches a fragment: 1 in the open, 0 in a wall's shadow.
//
// The same grid as a flame's walk and the same test against a cell's span, with
// three differences, all of them from the sun having no position: the ray has no
// endpoint, so it is bounded by `MAX_SUN_STEPS`; one step is one tile along the
// ground, so the height climbs by the sun's own slope; and the walk stops as
// soon as the ray is above everything in the grid, because from there on it is
// looking at sky. The fragment's own tile is skipped, so a wall is lit on the
// side the sun is on rather than shadowed by itself.
fn sunlight(at: vec3<f32>) -> f32 {
    let horizontal = length(lighting.sun.xy);
    if horizontal < 1.0e-6 {
        // Straight overhead: nothing but the fragment's own tile is in the way,
        // and that one is exempt.
        return 1.0;
    }
    let step = vec3<f32>(
        lighting.sun.x / horizontal,
        lighting.sun.y / horizontal,
        lighting.sun.z / horizontal * Z_PER_TILE,
    );
    var through = 1.0;
    for (var tile = 1; tile <= MAX_SUN_STEPS; tile = tile + 1) {
        let along = at + step * f32(tile);
        if along.z > lighting.sun.w {
            break;
        }
        let cell = occluder_at(i32(floor(along.x)), i32(floor(along.y)));
        if cell.w == 0u {
            continue;
        }
        let bottom = f32(cell.x) - 128.0;
        let top = f32(cell.y) - 128.0;
        if along.z >= bottom && along.z <= top {
            through = through * (1.0 - f32(cell.z) / 255.0);
            if through <= 0.004 {
                return 0.0;
            }
        }
    }
    return through;
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
        // seam or a step has nothing to hide behind.
        return min(lit, vec3<f32>(1.0));
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
    let at = vec3<f32>(f32(place.x) + sub.x, f32(place.y) + sub.y, f32(place.z) - 128.0);

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
        let through = reaches(at, vec3<f32>(to.x, to.y, to.z));
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
        lit = lit + light.color.rgb * (light.color.w * fall * fall * through * lit_by);
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
                view, place, at, sub, lit, reached, nearest, nearest_through, sun_through, share,
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
