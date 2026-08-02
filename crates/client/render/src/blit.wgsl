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

// One flame. Two `vec4`s rather than six fields, because a uniform array's
// stride is rounded up to 16 bytes either way and this is what the CPU writes:
//
//   place = (tile x, tile y, z, radius in tiles)
//   color = (r, g, b, intensity)
struct Light {
    place: vec4<f32>,
    color: vec4<f32>,
};

// Sized to `Lighting::MAX`, and the two have to agree — this is a uniform
// array, so its length is fixed at compile time and a mismatch is a buffer
// wgpu rejects rather than a wrong picture.
const MAX_LIGHTS: u32 = 64u;

struct Lighting {
    // What the whole frame is multiplied by away from every flame, then how
    // many of `lights` are real. `(1,1,1,0)` is no lighting at all, and the
    // identity below is exact for it.
    ambient: vec4<f32>,
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
// A pool reaches nine tiles at the widest (`light::CAMPFIRE`), and a fragment
// further away than its radius never gets here at all — so this is a bound that
// is never actually reached, and it exists so that a loop over data cannot be
// made unbounded by a radius somebody widens later.
const MAX_RAY_STEPS: i32 = 16;

// What stands on one tile, or all zeros for open ground and for anything
// outside the grid.
fn occluder_at(x: i32, y: i32) -> vec4<u32> {
    let cell = vec2<i32>(x - lighting.grid.x, y - lighting.grid.y);
    if cell.x < 0 || cell.y < 0 || cell.x >= lighting.grid.z || cell.y >= lighting.grid.w {
        return vec4<u32>(0u);
    }
    return textureLoad(occluders, cell, 0);
}

// How much of a flame reaches a tile: 1 for nothing in the way, 0 for a wall.
//
// A walk of the cells between the two, Chebyshev-stepped — the same distance the
// game itself measures in, so one step moves one tile on the longer axis and the
// walk visits every tile the ray crosses on that axis. Positions are fractional
// and a cell is the *floor* of one, which is what makes the endpoints exactly
// the two tiles the ray starts and ends in. Both ends are left out:
// the flame's own tile must not shadow it (a sconce stands *on* a wall), and the
// tile being lit must not shadow itself, which is what keeps a wall's own face
// the brightest thing near a torch.
//
// The height is interpolated along the ray, so a cell only stops the light where
// the ray actually passes through the span it occupies. That is what keeps a
// cellar's wall out of the street above it.
fn reaches(lit: vec3<f32>, flame: vec3<f32>) -> f32 {
    let delta = flame - lit;
    let steps = min(i32(max(abs(delta.x), abs(delta.y))), MAX_RAY_STEPS);
    var through = 1.0;
    for (var i = 1; i < steps; i = i + 1) {
        let t = f32(i) / f32(steps);
        let at = lit + delta * t;
        let cell = occluder_at(i32(floor(at.x)), i32(floor(at.y)));
        if cell.w == 0u {
            continue;
        }
        let bottom = f32(cell.x) - 128.0;
        let top = f32(cell.y) - 128.0;
        if at.z >= bottom && at.z <= top {
            through = through * (1.0 - f32(cell.z) / 255.0);
            if through <= 0.004 {
                // Under a byte's worth of light: nothing further can matter,
                // and a wall is the common case.
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
) -> vec3<f32> {
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

    var lit = lighting.ambient.rgb;
    // What the diagnostics are made of, gathered while the frame is lit rather
    // than computed a second time: how many flames actually reached this
    // fragment, and what the *nearest* of them lost on the way. Nearest and not
    // an average, because a shadow view is read as "is this pixel behind
    // something", and the flame that answers that is the one lighting it most.
    var reached = 0u;
    var nearest = 1.0e9;
    var nearest_through = 0.0;
    let count = u32(lighting.ambient.w);
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
        lit = lit + light.color.rgb * (light.color.w * fall * fall * through);
    }

    if view != VIEW_LIT {
        return vec4<f32>(debug_color(view, place, at, sub, lit, reached, nearest, nearest_through), color.a);
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
