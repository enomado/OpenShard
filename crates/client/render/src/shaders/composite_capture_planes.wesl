// Resample a map-only source rectangle into the colour and G-buffer planes of
// one cached block.  This is four attachments (the normal world-pass budget),
// while depth is captured by the companion one-target pass.

import package::place_format::{ids_kind, KIND_NOTHING};

struct Capture {
    target_size: vec2<f32>,
    source_origin: vec2<f32>,
    source_size: vec2<f32>,
    // The tile at the north-west corner of the map block this entry owns.
    // The sampled screen rectangle includes static overhang, so it may also
    // contain a neighbour's pixels; they must remain with that neighbour.
    block_first_tile: vec2<f32>,
    _padding: vec2<f32>,
};

@group(0) @binding(0) var<uniform> capture: Capture;
@group(0) @binding(1) var color_of: texture_2d<f32>;
@group(0) @binding(2) var ids_of: texture_2d<u32>;
@group(0) @binding(3) var position_of: texture_2d<f32>;
@group(0) @binding(4) var normal_of: texture_2d<u32>;

struct VertexOut { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex
fn vs_main(@location(0) corner: vec2<f32>) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
    out.uv = corner;
    return out;
}

struct FragmentOut {
    @location(0) color: vec4<f32>,
    @location(1) ids: u32,
    @location(2) position: vec4<f32>,
    @location(3) normal: u32,
};
@fragment
fn fs_main(in: VertexOut) -> FragmentOut {
    var out: FragmentOut;
    out.color = vec4<f32>(0.0);
    out.ids = 0u;
    out.position = vec4<f32>(0.0);
    out.normal = 0u;

    // LOD1's footprint is exactly one source pixel, preserving the atomic
    // colour/ID/position/depth correspondence. LOD2 (currently disabled)
    // covers 4x4 source pixels and selects one valid representative.
    let output_at = vec2<i32>(floor(in.uv * capture.target_size));
    let footprint = max(
        vec2<i32>(capture.source_size / capture.target_size),
        vec2<i32>(1),
    );
    let start = vec2<i32>(capture.source_origin) + output_at * footprint;
    let source_last = vec2<i32>(capture.source_origin + capture.source_size) - vec2<i32>(1);
    var found = false;
    for (var y = 0; y < 4; y += 1) {
        for (var x = 0; x < 4; x += 1) {
            if x >= footprint.x || y >= footprint.y || found {
                continue;
            }
            let at = min(start + vec2<i32>(x, y), source_last);
            let ids = textureLoad(ids_of, at, 0).x;
            let position = textureLoad(position_of, at, 0);
            // The producer itself owns geometry: it collects exactly this
            // block's tiles before rasterisation.  Re-testing ownership from
            // interpolated G-buffer position is not equivalent, and used to
            // erase water/land fragments along 8x8 block boundaries.
            if ids_kind(ids) != KIND_NOTHING {
                out.color = textureLoad(color_of, at, 0);
                out.ids = ids;
                out.position = position;
                out.normal = textureLoad(normal_of, at, 0).x;
                found = true;
            }
        }
    }
    return out;
}
