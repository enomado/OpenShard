// The radar pass: one quad, one texture, over a finished frame.
//
// Deliberately not `gump.wgsl`. That shader draws *art* — many instances out of
// one packed atlas, with a hue lookup, because every picture it draws came out
// of a client file and may be tinted. This one draws a bitmap the client
// generated: one quad, no atlas, no instances, and no hue, because there is no
// `hues.mul` entry for a colour that was never in a client file.
//
// The quad's place is in the uniform rather than an instance buffer for the same
// reason: there is exactly one of it, and an instance buffer for a single
// instance is a buffer to keep in step for no benefit.

struct Radar {
    // The target's size in real pixels. Not `target`: WGSL reserves that word.
    screen: vec2<f32>,
    // The window's top-left corner, in gump pixels.
    origin: vec2<f32>,
    // Its size, in gump pixels.
    extent: vec2<f32>,
    // Real pixels per gump pixel — `gump.wgsl`'s `scale`, and the same value, so
    // the radar grows and shrinks with the rest of the interface.
    scale: f32,
    // Uniform blocks are sized in multiples of 16 bytes.
    _padding: f32,
};

@group(0) @binding(0) var<uniform> radar: Radar;
@group(0) @binding(1) var pixels: texture_2d<f32>;
@group(0) @binding(2) var pixel_sampler: sampler;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(
    // The unit quad: (0,0) top-left to (1,1) bottom-right.
    @location(0) corner: vec2<f32>,
) -> VertexOut {
    let pixel = (radar.origin + corner * radar.extent) * radar.scale;

    // Pixels to clip space; `y` flips because the interface counts down from the
    // top of the window and clip space counts up from the middle.
    let ndc = vec2<f32>(
        pixel.x / radar.screen.x * 2.0 - 1.0,
        1.0 - pixel.y / radar.screen.y * 2.0,
    );

    var out: VertexOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = corner;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Opaque everywhere. A radar tile with no colour is `radar::UNKNOWN` rather
    // than transparent — see that module — so there is nothing here to discard
    // and a hole in the window is not reachable from this end either.
    let colour = textureSample(pixels, pixel_sampler, in.uv);
    return vec4<f32>(colour.rgb, 1.0);
}
