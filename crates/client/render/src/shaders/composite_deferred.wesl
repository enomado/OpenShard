// One cached immutable map block, restored into the ordinary deferred world
// attachments. Dynamic items and mobiles are drawn afterwards against the
// depth written here; they never enter this texture.

import package::place_format::{ids_kind, KIND_NOTHING, IDS_COMPOSITE_MAP, IDS_ID_SHIFT};

struct Viewport { size: vec2<f32>, _padding: vec2<f32> };
@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(0) @binding(1) var color_of: texture_2d<f32>;
@group(0) @binding(2) var ids_of: texture_2d<u32>;
@group(0) @binding(3) var position_of: texture_2d<f32>;
@group(0) @binding(4) var normal_of: texture_2d<u32>;
@group(0) @binding(5) var depth_of: texture_2d<f32>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) depth_adjust: f32,
};
@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) depth_adjust: f32,
) -> VertexOut {
    let at = rect.xy + corner * rect.zw;
    var out: VertexOut;
    out.position = vec4<f32>(at.x / viewport.size.x * 2.0 - 1.0, 1.0 - at.y / viewport.size.y * 2.0, 0.0, 1.0);
    // The adjustment belongs to the captured block, not to the camera frame:
    // one far-zoom frame commonly restores blocks captured from different
    // source eyes. Carrying it per instance keeps every block in one upload
    // and one render pass instead of creating a queue write for each depth.
    out.uv = corner;
    out.depth_adjust = depth_adjust;
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
    // Every plane must come from one integer texel. Sampling `color_of` with
    // normalized UVs — even with a nearest sampler — has different edge/tie
    // rules from the integer G-buffer loads below, and can borrow a colour
    // from an adjacent texel while retaining this texel's IDs and depth.
    out.color = textureLoad(color_of, at, 0);
    // The route bits live in the *id field*, not directly in the packed
    // G-buffer word. `ids_id` shifts by `IDS_ID_SHIFT` before `blit.wesl`
    // checks this marker. OR-ing the unshifted bit made a producer-local row
    // look like a current-frame ground-row index after restore.
    out.ids = ids | (IDS_COMPOSITE_MAP << IDS_ID_SHIFT);
    out.position = textureLoad(position_of, at, 0);
    out.normal = textureLoad(normal_of, at, 0).x;
    out.depth = clamp(textureLoad(depth_of, at, 0).x + in.depth_adjust, 0.0, 1.0);
    return out;
}
