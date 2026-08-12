// Copy one source depth rectangle into an R32Float cached-composite plane.
// The map target's `Depth24Plus` attachment is sampleable but not copyable to
// a float colour texture, so this tiny pass is the lossless GPU-side crossing.

struct Capture {
    target_size: vec2<f32>,
    source_origin: vec2<f32>,
    source_size: vec2<f32>,
    _padding: vec2<f32>,
};

@group(0) @binding(0) var<uniform> capture: Capture;
@group(0) @binding(1) var source_depth: texture_depth_2d;

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
    let at = vec2<i32>(min(
        capture.source_origin + floor(in.uv * capture.source_size),
        capture.source_origin + capture.source_size - vec2<f32>(1.0),
    ));
    return textureLoad(source_depth, at, 0);
}
