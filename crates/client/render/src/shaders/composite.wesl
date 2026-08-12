// One cached map-block texture, placed as one screen-aligned quad.
//
// This is deliberately only colour.  Work 4 decides how a cached map block
// participates in the main frame's depth and G-buffer so server objects can
// still interleave with it.  Treating a cached block as one fabricated surface
// here would make a mobile or item pick a false depth, which is worse than
// keeping that policy at the caller boundary for one work item.

struct Viewport {
    size: vec2<f32>,
    _padding: vec2<f32>,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(0) @binding(1) var composite: texture_2d<f32>;
@group(0) @binding(2) var composite_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) corner: vec2<f32>, @location(1) rect: vec4<f32>) -> VertexOut {
    let at = rect.xy + corner * rect.zw;
    var out: VertexOut;
    out.position = vec4<f32>(
        at.x / viewport.size.x * 2.0 - 1.0,
        1.0 - at.y / viewport.size.y * 2.0,
        0.0,
        1.0,
    );
    out.uv = corner;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return textureSample(composite, composite_sampler, in.uv);
}
