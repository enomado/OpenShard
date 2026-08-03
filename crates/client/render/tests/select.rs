//! The selection wash, on a real GPU: what it paints and — the half that
//! matters — what it leaves alone.
//!
//! The scene is built out of the two records the pass actually reads, uploaded
//! rather than rendered: a place attachment written texel by texel, and a
//! silhouette mask written the same way. That is deliberate and it is
//! `crate::place`'s own argument for making the attachment `COPY_DST` — this
//! test is about a rule over those two records, and drawing sprites to produce
//! them would only be a slower way of producing the same bytes, with a whole art
//! pipeline in between the assertion and what it is about.
//!
//! The rule, in one sentence: **the wash is the mask, plus the ground of the
//! selected tile — and nothing else standing on that tile.** The last clause is
//! what a screen-space pass keyed on the tile alone gets wrong, and it is the
//! reason the mask exists at all, so it is the assertion this file is built
//! around.
//!
//! Gated on an adapter, like every other GPU test here, and an honest skip
//! rather than a failure where there is none.

use openshard_client_render::blit::ViewportRect;
use openshard_client_render::place::{self, Kind, STANCE_SHIFT, Stance};
use openshard_client_render::select::{Frame as SelectFrame, Select, Selection};

/// The world image, and the surface: one size, so the pass's `uv` mapping is the
/// identity and every assertion is about the rule rather than about sampling.
/// A multiple of 64, which is what makes a row copy 256-byte aligned.
const SIZE: u32 = 64;

/// The tile the selection is on, and its neighbour — which must come out
/// untouched however the comparison is written.
const SELECTED: (u16, u16) = (1000, 2000);
const NEIGHBOUR: (u16, u16) = (1001, 2000);

/// What the target holds before the wash, so that "unchanged" is a value and not
/// an absence. Mid-grey rather than black: a wash blended into black is the wash
/// itself, and the two would be indistinguishable.
const UNDER: [u8; 4] = [90, 90, 90, 255];

/// A GPU to draw with, or `None` where there is none.
fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// One texel of the place attachment, packed the way the world passes write it
/// — `crate::place` and `statics.wgsl`, whose fourth channel is the kind in the
/// low two bits and the sub-tile fraction above it.
fn place_texel(tile: (u16, u16), kind: Kind, stance: Stance) -> [u16; 4] {
    [
        tile.0,
        tile.1,
        // A height of zero, offset as the attachment carries it, with the stance
        // in the eight bits above.
        128 | ((stance as u16) << STANCE_SHIFT),
        // Mid-tile, which is where the fraction is irrelevant to this pass: it
        // reads the kind and the stance out of these channels and nothing else.
        (kind as u16) | (64 << 2) | (64 << 9),
    ]
}

/// Which band of rows each kind of pixel occupies. Bands rather than scattered
/// texels so that a failure can be read as "this band is wrong" instead of as a
/// coordinate.
const BANDS: [(u32, u32); 4] = [(0, 16), (16, 32), (32, 48), (48, 64)];

/// The four bands, in order:
///
/// 0. the land of the selected tile — the outdoor case;
/// 1. a *flat static* on it, which is a wooden floor or a rug: the indoor case,
///    where the land is under a floor and never drawn;
/// 2. an *upright static* on it — a second wall standing on the very same tile,
///    which is the thing that must not be washed;
/// 3. the land of the tile next door.
fn scene() -> Vec<[u16; 4]> {
    let mut texels = Vec::with_capacity((SIZE * SIZE) as usize);
    for y in 0..SIZE {
        let texel = match y {
            y if y < BANDS[0].1 => place_texel(SELECTED, Kind::Land, Stance::Flat),
            y if y < BANDS[1].1 => place_texel(SELECTED, Kind::Static, Stance::Flat),
            y if y < BANDS[2].1 => place_texel(SELECTED, Kind::Static, Stance::Upright),
            _ => place_texel(NEIGHBOUR, Kind::Land, Stance::Flat),
        };
        for _ in 0..SIZE {
            texels.push(texel);
        }
    }
    texels
}

/// The left half of band 2: the picked wall's own visible pixels, as the
/// silhouette pass would have left them. The right half of that band is the
/// *other* wall on the same tile.
fn mask_id(x: u32, y: u32) -> u8 {
    let (top, bottom) = BANDS[2];
    match (top..bottom).contains(&y) && x < SIZE / 2 {
        true => 1,
        false => 0,
    }
}

/// Run the pass over the uploaded scene and read the surface back as RGBA8 rows.
fn wash(device: &wgpu::Device, queue: &wgpu::Queue, selection: Selection) -> Vec<[u8; 4]> {
    let place_texture = place::texture(device, SIZE, SIZE);
    let mut place_bytes = Vec::with_capacity((SIZE * SIZE * 8) as usize);
    for texel in scene() {
        for channel in texel {
            place_bytes.extend_from_slice(&channel.to_le_bytes());
        }
    }
    queue.write_texture(
        place_texture.as_image_copy(),
        &place_bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE * 8),
            rows_per_image: Some(SIZE),
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );

    // The mask, in the same format the silhouette pass writes and with
    // `COPY_DST` on top of it: `outline::mask_texture` does not ask for that
    // usage, because nothing but a test ever writes one from the CPU.
    let mask = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("selection mask"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: openshard_client_render::outline::MASK_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let ids: Vec<u8> = (0..SIZE)
        .flat_map(|y| (0..SIZE).map(move |x| mask_id(x, y)))
        .collect();
    queue.write_texture(
        mask.as_image_copy(),
        &ids,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE),
            rows_per_image: Some(SIZE),
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let surface = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("surface"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let surface_view = surface.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    // The picture the wash goes over, which here is one flat colour: this pass
    // never reads the world image, only writes over it, so what is under the
    // wash is the whole of the scene it needs.
    encoder
        .begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("under"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &surface_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(UNDER[0]) / 255.0,
                        g: f64::from(UNDER[1]) / 255.0,
                        b: f64::from(UNDER[2]) / 255.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        })
        .forget_lifetime();

    Select::new(device, format).render(
        device,
        queue,
        &mut encoder,
        SelectFrame {
            target: &surface_view,
            mask: &mask.create_view(&wgpu::TextureViewDescriptor::default()),
            place: &place_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            size: (SIZE, SIZE),
            rect: ViewportRect {
                x: 0,
                y: 0,
                width: SIZE,
                height: SIZE,
            },
        },
        selection,
    );
    queue.submit([encoder.finish()]);

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(SIZE) * u64::from(SIZE) * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        surface.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| {
        result.expect("mapping a buffer this test just wrote");
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("waiting on our own submission");
    let bytes = slice
        .get_mapped_range()
        .expect("the map completed above")
        .to_vec();
    readback.unmap();
    bytes
        .chunks_exact(4)
        .map(|pixel| [pixel[0], pixel[1], pixel[2], pixel[3]])
        .collect()
}

/// What a premultiplied wash of `color` over [`UNDER`] comes to.
///
/// The blend the pipeline is built with — `dst = src.rgb + dst * (1 - src.a)`
/// with `src.rgb` already scaled by its alpha — written out here rather than
/// trusted: this is the one number in the pass a person can be wrong about and
/// still see a plausible highlight.
fn washed(color: [f32; 4]) -> [u8; 3] {
    let over = |channel: usize, under: u8| {
        let under = f32::from(under) / 255.0;
        let value = color[channel] * color[3] + under * (1.0 - color[3]);
        (value * 255.0).round() as u8
    };
    [over(0, UNDER[0]), over(1, UNDER[1]), over(2, UNDER[2])]
}

/// Every pixel of the scene, against the rule stated in the module docs.
///
/// One test and not four, because the four bands are one claim: what is washed
/// is what is washed *instead of* the others, and a test that only looked at the
/// pixels it expected to change would pass on a pass that washed the whole
/// screen.
#[test]
fn the_wash_covers_the_selected_sprite_and_its_ground_and_nothing_else() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let selection = Selection::DEFAULT.on(SELECTED);
    let frame = wash(&device, &queue, selection);

    let sprite = washed(selection.sprite);
    let ground = washed(selection.ground);
    // The three outcomes are distinguishable, or the assertions below are
    // satisfied by a pass that cannot tell them apart. A companion, and this
    // repository has produced the green without one before.
    assert_ne!(sprite, ground, "the two washes are the same colour");
    assert_ne!(
        ground,
        [UNDER[0], UNDER[1], UNDER[2]],
        "the ground wash is invisible"
    );

    let mut counted = [0usize; 3];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let got = frame[(y * SIZE + x) as usize];
            let (want, band) = match (mask_id(x, y) != 0, y) {
                // The picked wall's own pixels.
                (true, _) => (sprite, 0),
                // The land of its tile, and the floor lying on that same tile:
                // both are "the ground here", and indoors only the second is
                // ever drawn.
                (false, y) if y < BANDS[0].1 || (BANDS[1].0..BANDS[1].1).contains(&y) => (ground, 1),
                // Everything else: the *other* wall standing on the selected
                // tile, and the tile next door. Untouched — the first is what
                // separates this from a wash keyed on the tile alone.
                _ => ([UNDER[0], UNDER[1], UNDER[2]], 2),
            };
            counted[band] += 1;
            for channel in 0..3 {
                let (got, want) = (i32::from(got[channel]), i32::from(want[channel]));
                assert!(
                    (got - want).abs() <= 1,
                    "({x}, {y}) channel {channel}: {got} against {want} — band {band}",
                );
            }
            assert_eq!(got[3], 255, "({x}, {y}) lost the frame's own alpha");
        }
    }
    // And each band had something in it. Without this the loop above is happy
    // with a scene where two of the three cases never occur, which is exactly
    // how a rule test passes without testing its rule.
    for (band, count) in counted.iter().enumerate() {
        assert!(*count > 0, "band {band} was never reached");
    }
}

/// A selection with nothing under the cursor washes nothing at all.
///
/// The frame the client draws almost always: worth an assertion because the
/// *absence* is what the wash costs nothing for, and a pass that washed the
/// whole screen when no tile was named would be a client that turns cyan on a
/// click on open ground.
#[test]
fn a_selection_with_no_tile_leaves_the_ground_alone() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let frame = wash(&device, &queue, Selection::DEFAULT);
    let sprite = washed(Selection::DEFAULT.sprite);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let got = frame[(y * SIZE + x) as usize];
            // The mask is still the mask: what "no tile" switches off is the
            // ground, and the thing itself is washed either way. That is the
            // distinction the uniform's third word carries.
            let want = match mask_id(x, y) != 0 {
                true => sprite,
                false => [UNDER[0], UNDER[1], UNDER[2]],
            };
            for channel in 0..3 {
                let (got, want) = (i32::from(got[channel]), i32::from(want[channel]));
                assert!((got - want).abs() <= 1, "({x}, {y}) channel {channel}");
            }
        }
    }
}
