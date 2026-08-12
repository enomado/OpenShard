// One cached immutable map block, restored into the ordinary deferred world
// attachments. Dynamic items and mobiles are drawn afterwards against the
// depth written here; they never enter this texture.

import package::place_format::{ids_kind, KIND_NOTHING, IDS_COMPOSITE_MAP};

struct Viewport { size: vec2<f32>, depth_adjust: f32, _padding: f32 };
@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(0) @binding(1) var color_of: texture_2d<f32>;
@group(0) @binding(2) var ids_of: texture_2d<u32>;
@group(0) @binding(3) var position_of: texture_2d<f32>;
@group(0) @binding(4) var normal_of: texture_2d<u32>;
@group(0) @binding(5) var depth_of: texture_2d<f32>;
@group(0) @binding(6) var nearest: sampler;

struct VertexOut { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex
fn vs_main(@location(0) corner: vec2<f32>, @location(1) rect: vec4<f32>) -> VertexOut {
    let at = rect.xy + corner * rect.zw;
    var out: VertexOut;
    out.position = vec4<f32>(at.x / viewport.size.x * 2.0 - 1.0, 1.0 - at.y / viewport.size.y * 2.0, 0.0, 1.0);
    out.uv = corner;
    return out;
}
struct FragmentOut {
    @location(0) color: vec4<f32>,
    @location(1) ids: u32,
    @location(2) position: vec4<f32>,
    @location(3) normal: u32,
    @builtin(frag_depth) depth: f32,
};
@fragment
fn fs_main(in: VertexOut) -> FragmentOut {
    let size = textureDimensions(ids_of);
    let at = min(vec2<i32>(in.uv * vec2<f32>(size)), vec2<i32>(size) - vec2<i32>(1));
    let ids = textureLoad(ids_of, at, 0).x;
    if ids_kind(ids) == KIND_NOTHING { discard; }
    var out: FragmentOut;
    out.color = textureSampleLevel(color_of, nearest, in.uv, 0.0);
    out.ids = ids | IDS_COMPOSITE_MAP;
    out.position = textureLoad(position_of, at, 0);
    out.normal = textureLoad(normal_of, at, 0).x;
    out.depth = clamp(textureLoad(depth_of, at, 0).x + viewport.depth_adjust, 0.0, 1.0);
    return out;
}
