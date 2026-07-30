//! Frames, rendered on a real GPU and read back pixel by pixel.
//!
//! A renderer's usual problem is that it has no oracle: the output is looked at,
//! and "looks right" survives an off-by-one in the projection, a swapped colour
//! channel and a sprite sampled one texel over. Rendering to a texture instead
//! of a window removes that excuse — the frame is bytes, and bytes can be
//! compared with the art the frame was built from.
//!
//! Two things gate these tests, and both are honest skips rather than failures:
//! `OPENSHARD_CLIENT`, because no client files live in this repository, and the
//! presence of an adapter, because CI machines do not always have one.

use std::path::PathBuf;

use openshard_client_render::atlas::LandAtlas;
use openshard_client_render::camera::Camera;
use openshard_client_render::ground::{self, GroundQuad};
use openshard_client_render::renderer::{GroundRenderer, Target};
use openshard_protocol::wire::Graphic;
use openshard_protocol::world::Point;
use openshard_uofiles::art::{Art, LAND_TILE_SIZE, land_row};
use openshard_uofiles::map::Map;

/// The client's files, or `None` when the environment does not point at any.
fn client_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("OPENSHARD_CLIENT")?))
}

/// A GPU to draw with, or `None` where there is none.
fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    // The defaults are WebGL2's limits in wgpu's downlevel form, which is the
    // point: a pipeline that needs more than this would not run in a browser,
    // and finding that out here is cheaper than finding it out in one.
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// A rendered frame, as RGBA8 rows.
struct Frame {
    width: u32,
    pixels: Vec<u8>,
}

impl Frame {
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * self.width + x) * 4) as usize;
        [
            self.pixels[at],
            self.pixels[at + 1],
            self.pixels[at + 2],
            self.pixels[at + 3],
        ]
    }

    /// How many pixels the ground pass wrote. Anything drawn is opaque and the
    /// clear is fully transparent, so this counts exactly, with no threshold.
    fn drawn(&self) -> usize {
        self.pixels.chunks_exact(4).filter(|p| p[3] == u8::MAX).count()
    }
}

/// Draw `quads` into a `width` x `height` texture and read the result back.
///
/// `width * 4` must be a multiple of 256: that is the row alignment a buffer
/// copy demands, and padding it here would only hide the constraint from the
/// callers, which choose their own sizes.
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &LandAtlas,
    quads: &[GroundQuad],
    width: u32,
    height: u32,
) -> Frame {
    assert_eq!(width * 4 % 256, 0, "a row copy has to be 256-byte aligned");

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("frame"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(width) * u64::from(height) * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut renderer = GroundRenderer::new(device, queue, format, atlas);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let target_view = Target {
        view: &view,
        width,
        height,
    };
    renderer.render(device, queue, &mut encoder, target_view, quads);
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
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
    let pixels = slice
        .get_mapped_range()
        .expect("the map completed above")
        .to_vec();
    readback.unmap();

    Frame { width, pixels }
}

/// One sprite, drawn alone, compared to the art texel for texel.
///
/// This is the test that ties the three layers together: the atlas packed the
/// sprite somewhere, the instance carried texture coordinates, and the shader
/// sampled them. Any of the three being off by a texel moves the diamond, and
/// nothing else in the suite would notice — a whole frame of ground still looks
/// like ground when every tile samples its neighbour.
#[test]
fn a_lone_sprite_matches_the_art_it_came_from() {
    let (Some(dir), Some((device, queue))) = (client_dir(), gpu()) else {
        return;
    };
    let art = Art::open(&dir).expect("artLegacyMUL.uop");

    // The first land graphic the client actually ships. Which one it is does not
    // matter; that it is real art with a real shape does.
    let (graphic, image) = (0..0x4000u16)
        .map(Graphic)
        .find_map(|g| art.land(g).expect("reading land art").map(|image| (g, image)))
        .expect("a modern client ships thousands of land tiles");

    let atlas = LandAtlas::build(&art, [graphic]).expect("one graphic fits");
    let region = atlas.region(graphic).expect("just packed");

    // Level, and centred so its bounding square starts at the viewport's origin:
    // viewport coordinates are then the sprite's own. A tile whose four corners
    // share a height is drawn as the art's square, which is what makes this
    // comparison texel for texel possible at all — see `ground.wgsl`.
    let quads = [GroundQuad {
        x: f32::from(LAND_TILE_SIZE) / 2.0,
        y: f32::from(LAND_TILE_SIZE) / 2.0,
        corners: [0.0; 4],
        region,
    }];
    let side = u32::from(LAND_TILE_SIZE);
    let frame = render(&device, &queue, &atlas, &quads, 64, 64);

    let mut compared = 0;
    for y in 0..side {
        let row = land_row(y as u16);
        for x in 0..side {
            let got = frame.pixel(x, y);
            if !row.contains(&(x as u16)) {
                assert_eq!(got[3], 0, "({x}, {y}) is outside the diamond but was drawn");
                continue;
            }
            // Inside the diamond every pixel is drawn, black ones included:
            // ground has no transparency, and a tile that loses its zero pixels
            // is a tile with pinholes in it.
            let (r, g, b) = image.pixel(x as u16, y as u16).expect("inside the sprite").rgb8();
            assert_eq!(
                got,
                [r, g, b, u8::MAX],
                "({x}, {y}) does not match the art: the sprite is sampled from the wrong place",
            );
            compared += 1;
        }
    }

    // The diamond is 1,012 of the square's 1,936 pixels. Without this the loop
    // above would pass on a sprite that decoded to nothing at all.
    assert_eq!(compared, 1012, "the diamond should be 1,012 drawn pixels");

    // And nothing outside the sprite's own square was touched.
    for y in 0..64 {
        for x in 0..64 {
            if x < side && y < side {
                continue;
            }
            assert_eq!(frame.pixel(x, y)[3], 0, "({x}, {y}) is outside the quad");
        }
    }
}

/// Level ground tiles the viewport exactly, with no pixel left over.
///
/// This is the assertion the projection lives or dies by: the diamonds only
/// meet if a step is exactly 22 pixels on each axis and the sprite is exactly
/// 44 across. Any other numbers leave a lattice of gaps, and a lattice of gaps
/// against a black background is close to invisible on a screenshot.
///
/// It is deliberately *level* ground. Flat diamonds are only the whole truth
/// where the four corners of a tile share a height, and the sea is the largest
/// place that is true — see the sibling test for what happens on a hillside.
#[test]
fn level_ground_covers_every_pixel() {
    let (Some(dir), Some((device, queue))) = (client_dir(), gpu()) else {
        return;
    };
    let map = Map::load_facet(&dir, 0).expect("Felucca");
    let art = Art::open(&dir).expect("artLegacyMUL.uop");

    // Open sea off the north-west corner: 80 tiles square at a single height.
    let camera = Camera::new(Point::new(200, 200, -5), 768, 512);

    // The premise, checked rather than assumed. If this patch of Felucca ever
    // stopped being level the coverage assertion below would start measuring
    // the terrain instead of the projection, and would still be green.
    for y in 160..240u16 {
        for x in 160..240u16 {
            assert_eq!(
                map.land(x, y).map(|cell| cell.z),
                Some(-5),
                "({x}, {y}) is not at sea level; this test needs level ground",
            );
        }
    }

    let atlas = LandAtlas::build(&art, ground::visible_graphics(&map, &camera)).expect("fits");
    let quads = ground::collect(&map, &camera, &atlas);
    assert!(!quads.is_empty(), "the sea is made of land tiles too");

    let frame = render(&device, &queue, &atlas, &quads, camera.width, camera.height);
    let total = (camera.width * camera.height) as usize;
    assert_eq!(
        frame.drawn(),
        total,
        "level ground left holes: the diamonds do not meet",
    );

    // One flat colour would satisfy everything above, and is also what a broken
    // atlas produces.
    let first = frame.pixel(0, 0);
    assert!(
        (0..camera.height).any(|y| (0..camera.width).any(|x| frame.pixel(x, y) != first)),
        "the whole frame is one colour",
    );
}

/// A screen of Britain: hilly ground covers the viewport as completely as the
/// sea does.
///
/// This is the assertion stretched ground exists for. Flat 44x44 diamonds drawn
/// at different heights pull apart along a slope and leave a lattice of seams —
/// which is what this test used to pin, at 97.7% of the viewport. A tile
/// stretched over its four corner heights cannot do that: neighbours are built
/// from *the same* corners, so the mesh is watertight by construction rather
/// than by the projection's arithmetic coming out even.
///
/// Real terrain is also the only place the two shapes meet, so this covers the
/// join between them: a flat tile beside a sloped one, and no gap at the seam.
#[test]
fn hilly_ground_covers_every_pixel() {
    let (Some(dir), Some((device, queue))) = (client_dir(), gpu()) else {
        return;
    };
    let map = Map::load_facet(&dir, 0).expect("Felucca");
    let art = Art::open(&dir).expect("artLegacyMUL.uop");

    // Britain, near the bank: the ground here runs from z = -15 to z = 25.
    let camera = Camera::new(Point::new(1495, 1629, 0), 768, 512);
    let atlas = LandAtlas::build(&art, ground::visible_graphics(&map, &camera)).expect("fits");
    let quads = ground::collect(&map, &camera, &atlas);

    // Every cell the camera can see became a quad: the client ships art for all
    // of them, so a missing quad would mean the atlas or the lookup lost one.
    let bounds = camera.visible_tiles();
    let cells = (bounds.min_y.max(0)..=bounds.max_y)
        .flat_map(|y| (bounds.min_x.max(0)..=bounds.max_x).map(move |x| (x, y)))
        .filter(|&(x, y)| map.land(x as u16, y as u16).is_some())
        .count();
    assert_eq!(quads.len(), cells, "a visible tile was dropped");

    // The premise: this camera has to be looking at a hillside, or the test is
    // the level-ground one again under another name and would stay green
    // through the loss of everything it is here to protect.
    let sloped = quads
        .iter()
        .filter(|quad| quad.corners.iter().any(|z| *z != quad.corners[0]))
        .count();
    assert!(sloped > 100, "only {sloped} of {} quads slope", quads.len());

    let frame = render(&device, &queue, &atlas, &quads, camera.width, camera.height);
    let total = (camera.width * camera.height) as usize;
    assert_eq!(
        frame.drawn(),
        total,
        "hilly ground left holes: the corner heights do not meet",
    );
}

/// The same camera twice is the same bytes.
///
/// Determinism is not a nicety here: it is what makes every other assertion in
/// this file reproducible, and the ordering it depends on — the sort in
/// `ground::collect`, the `BTreeSet` in the atlas — is easy to lose to a
/// `HashMap` in a later change that looks harmless.
#[test]
fn the_same_camera_renders_the_same_frame() {
    let (Some(dir), Some((device, queue))) = (client_dir(), gpu()) else {
        return;
    };
    let map = Map::load_facet(&dir, 0).expect("Felucca");
    let art = Art::open(&dir).expect("artLegacyMUL.uop");
    let camera = Camera::new(Point::new(1495, 1629, 0), 768, 512);

    let mut frames = Vec::new();
    for _ in 0..2 {
        let atlas = LandAtlas::build(&art, ground::visible_graphics(&map, &camera)).expect("fits");
        let quads = ground::collect(&map, &camera, &atlas);
        frames.push(render(&device, &queue, &atlas, &quads, camera.width, camera.height).pixels);
    }
    assert_eq!(frames[0], frames[1]);

    // And a different camera is a different frame — otherwise the assertion
    // above is satisfied by a renderer that draws nothing at all.
    let moved = Camera::new(Point::new(1497, 1629, 0), 768, 512);
    let atlas = LandAtlas::build(&art, ground::visible_graphics(&map, &moved)).expect("fits");
    let quads = ground::collect(&map, &moved, &atlas);
    let other = render(&device, &queue, &atlas, &quads, moved.width, moved.height);
    assert_ne!(frames[0], other.pixels, "moving the camera changed nothing");
}
