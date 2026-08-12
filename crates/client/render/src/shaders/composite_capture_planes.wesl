// Resample a map-only source rectangle into the colour and G-buffer planes of
// one cached block.  This is four attachments (the normal world-pass budget),
// while depth is captured by the companion one-target pass.

struct Capture {
    target_size: vec2<f32>,
    source_origin: vec2<f32>,
    source_size: vec2<f32>,
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
    let at = vec2<i32>(min(
        capture.source_origin + floor(in.uv * capture.source_size),
        capture.source_origin + capture.source_size - vec2<f32>(1.0),
    ));
    var out: FragmentOut;
    out.color = textureLoad(color_of, at, 0);
    out.ids = textureLoad(ids_of, at, 0).x;
    out.position = textureLoad(position_of, at, 0);
    out.normal = textureLoad(normal_of, at, 0).x;
    return out;
}
