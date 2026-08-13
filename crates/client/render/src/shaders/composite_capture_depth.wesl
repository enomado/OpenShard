// Copy one source depth rectangle into an R32Float cached-composite plane.
// The map target's `Depth24Plus` attachment is sampleable but not copyable to
// a float colour texture, so this tiny pass is the lossless GPU-side crossing.

import package::place_format::{ids_kind, KIND_NOTHING};

struct Capture {
    target_size: vec2<f32>,
    source_origin: vec2<f32>,
    source_size: vec2<f32>,
    block_first_tile: vec2<f32>,
    _padding: vec2<f32>,
};

@group(0) @binding(0) var<uniform> capture: Capture;
@group(0) @binding(1) var source_depth: texture_depth_2d;
@group(0) @binding(2) var ids_of: texture_2d<u32>;

struct VertexOut { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs_main(@location(0) corner: vec2<f32>) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
    out.uv = corner;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) f32 {
    // Must choose the same representative as composite_capture_planes: a
    // cached G-buffer record and its depth are one atomic deferred fact.
    let output_at = vec2<i32>(floor(in.uv * capture.target_size));
    let footprint = max(
        vec2<i32>(capture.source_size / capture.target_size),
        vec2<i32>(1),
    );
    let start = vec2<i32>(capture.source_origin) + output_at * footprint;
    let source_last = vec2<i32>(capture.source_origin + capture.source_size) - vec2<i32>(1);
    for (var y = 0; y < 4; y += 1) {
        for (var x = 0; x < 4; x += 1) {
            if x >= footprint.x || y >= footprint.y {
                continue;
            }
            let at = min(start + vec2<i32>(x, y), source_last);
            let ids = textureLoad(ids_of, at, 0).x;
            if ids_kind(ids) != KIND_NOTHING {
                return textureLoad(source_depth, at, 0);
            }
        }
    }
    return 1.0;
}
