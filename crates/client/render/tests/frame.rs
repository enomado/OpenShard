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

use openshard_client_render::atlas::{LandAtlas, StaticAtlas, TexmapAtlas};
use openshard_client_render::camera::Camera;
use openshard_client_render::ground::{self, GroundQuad};
use openshard_client_render::renderer::{self, GroundRenderer, StaticRenderer, Target};
use openshard_client_render::statics::{self, StaticQuad};
use openshard_protocol::wire::Graphic;
use openshard_protocol::world::Point;
use openshard_uofiles::art::{Art, LAND_TILE_SIZE, land_row};
use openshard_uofiles::color::Color16;
use openshard_uofiles::image::Image;
use openshard_uofiles::map::Map;
use openshard_uofiles::texmaps::TexMaps;
use openshard_uofiles::tiledata::TileData;

/// The client's files, or `None` when the environment does not point at any.
fn client_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("OPENSHARD_CLIENT")?))
}

/// The texture atlas for a set of land graphics, read from a real install.
///
/// Two files rather than one: the textures themselves, and the `tiledata` that
/// says which of them a land graphic uses.
fn texmap_atlas(dir: &std::path::Path, wanted: impl IntoIterator<Item = Graphic>) -> TexmapAtlas {
    let texmaps = TexMaps::open(dir).expect("texidx.mul and texmaps.mul");
    let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");
    TexmapAtlas::build(&texmaps, &tiledata, wanted).expect("a screen of textures fits")
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

/// Draw ground into a `width` x `height` texture and read the result back.
///
/// The common case, and the one every test written before statics existed used.
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &LandAtlas,
    texmaps: &TexmapAtlas,
    quads: &[GroundQuad],
    width: u32,
    height: u32,
) -> Frame {
    let empty = StaticAtlas::pack([]).expect("nothing always fits");
    render_both(device, queue, atlas, texmaps, quads, &empty, &[], width, height)
}

/// Draw both passes into a `width` x `height` texture and read the result back.
///
/// `width * 4` must be a multiple of 256: that is the row alignment a buffer
/// copy demands, and padding it here would only hide the constraint from the
/// callers, which choose their own sizes.
#[allow(clippy::too_many_arguments)]
fn render_both(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &LandAtlas,
    texmaps: &TexmapAtlas,
    quads: &[GroundQuad],
    static_atlas: &StaticAtlas,
    static_quads: &[StaticQuad],
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

    // The depth buffer both passes share. Created here rather than inside the
    // renderer because a test that could not hand the two passes the same one
    // would not be testing the thing that makes them agree.
    let depth = renderer::depth_texture(device, width, height);
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());

    let mut renderer = GroundRenderer::new(device, queue, format, atlas, texmaps);
    let mut statics = StaticRenderer::new(device, queue, format, static_atlas);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let target_view = Target {
        view: &view,
        depth: &depth_view,
        width,
        height,
    };
    renderer.render(device, queue, &mut encoder, target_view, quads);
    statics.render(device, queue, &mut encoder, target_view, static_quads);
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
        texmap: None,
        // Anything inside clip space: this frame holds one quad, so there is
        // nothing for the depth test to decide.
        depth: 0.5,
    }];
    let side = u32::from(LAND_TILE_SIZE);
    let empty = TexmapAtlas::pack([]).expect("nothing always fits");
    let frame = render(&device, &queue, &atlas, &empty, &quads, 64, 64);

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

    let wanted = ground::visible_graphics(&map, &camera);
    let atlas = LandAtlas::build(&art, wanted.iter().copied()).expect("fits");
    let texmaps = texmap_atlas(&dir, wanted);
    let quads = ground::collect(&map, &camera, &atlas, &texmaps);
    assert!(!quads.is_empty(), "the sea is made of land tiles too");

    let frame = render(
        &device,
        &queue,
        &atlas,
        &texmaps,
        &quads,
        camera.width,
        camera.height,
    );
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
    let wanted = ground::visible_graphics(&map, &camera);
    let atlas = LandAtlas::build(&art, wanted.iter().copied()).expect("fits");
    let texmaps = texmap_atlas(&dir, wanted);
    let quads = ground::collect(&map, &camera, &atlas, &texmaps);

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

    // And most of those slopes are textured rather than falling back to the
    // stretched art, or the texture path is being exercised by nothing.
    let textured = quads
        .iter()
        .filter(|quad| quad.corners.iter().any(|z| *z != quad.corners[0]) && quad.texmap.is_some())
        .count();
    assert!(
        textured * 2 > sloped,
        "only {textured} of {sloped} sloped tiles have a texture map",
    );

    let frame = render(
        &device,
        &queue,
        &atlas,
        &texmaps,
        &quads,
        camera.width,
        camera.height,
    );
    let total = (camera.width * camera.height) as usize;
    assert_eq!(
        frame.drawn(),
        total,
        "hilly ground left holes: the corner heights do not meet",
    );
}

/// A sloped tile is drawn from its texture map, and a level one from its art.
///
/// The one assertion that says the branch in `ground.wgsl` is on the *heights*
/// and reads the *right* atlas. Both pictures are made here rather than read
/// from a client, and they are told apart by colour alone: green art, red
/// texture. Nothing subtler is needed and nothing subtler would survive a
/// reader's understanding writing both sides of the comparison — which is
/// exactly the trap `uofiles` fell into.
#[test]
fn a_sloped_tile_is_drawn_from_its_texture_and_a_level_one_from_its_art() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    const GRAPHIC: Graphic = Graphic(1);
    let green = Color16(0b0_00000_11111_00000);
    let red = Color16(0b0_11111_00000_00000);

    let side = usize::from(LAND_TILE_SIZE);
    let art = Image::new(LAND_TILE_SIZE, LAND_TILE_SIZE, vec![green; side * side]);
    let texture = Image::new(64, 64, vec![red; 64 * 64]);
    let atlas = LandAtlas::pack([(GRAPHIC, art)]).expect("one sprite fits");
    let texmaps = TexmapAtlas::pack([(GRAPHIC, texture)]).expect("one texture fits");
    let region = atlas.region(GRAPHIC).expect("packed");
    let texmap = texmaps.region(GRAPHIC).expect("packed");

    // Two tiles in one frame, far enough apart not to touch: one level, one
    // over four different corner heights. Same graphic, same regions — only the
    // heights differ, which is the whole claim.
    let quads = [
        GroundQuad {
            x: 64.0,
            y: 128.0,
            corners: [0.0; 4],
            region,
            texmap: Some(texmap),
            depth: 0.5,
        },
        GroundQuad {
            // Its own corner raised and its neighbours level: a hillock, and
            // the one direction that makes the quad *bigger* than the diamond
            // rather than shearing it into something smaller.
            x: 192.0,
            y: 128.0,
            corners: [4.0, 0.0, 0.0, 0.0],
            region,
            texmap: Some(texmap),
            depth: 0.5,
        },
    ];
    let frame = render(&device, &queue, &atlas, &texmaps, &quads, 256, 256);

    let (mut art_pixels, mut textured_pixels) = (0, 0);
    for y in 0..256 {
        for x in 0..256 {
            let pixel = frame.pixel(x, y);
            if pixel[3] == 0 {
                continue;
            }
            // The left half is the level tile and the right half the slope, so
            // a colour on the wrong side is a tile drawn from the wrong atlas.
            let expected = if x < 128 { green } else { red };
            let (r, g, b) = expected.rgb8();
            assert_eq!(
                pixel,
                [r, g, b, u8::MAX],
                "({x}, {y}) was drawn from the wrong atlas",
            );
            if x < 128 {
                art_pixels += 1;
            } else {
                textured_pixels += 1;
            }
        }
    }

    // A level tile is the art's diamond, exactly as the lone-sprite test pins
    // it. Anything else here and the comparison above was made against an empty
    // frame.
    assert_eq!(art_pixels, 1012, "the level tile is not the art's diamond");
    assert!(
        textured_pixels > 1012,
        "the sloped tile covered only {textured_pixels} pixels; it should be a stretched diamond",
    );
}

/// A static sprite is drawn at its own size, in its own place, and its
/// transparent pixels are not drawn at all.
///
/// The statics counterpart of the lone-sprite test, and it needs no client: the
/// picture is made here, so the frame can be compared against it exactly. What
/// it pins is the whole chain — the shelf packer put the sprite somewhere, the
/// instance carried a rectangle and a region, and the shader sampled one texel
/// per pixel. A sprite drawn at the wrong scale still looks like a sprite.
#[test]
fn a_static_sprite_is_drawn_texel_for_texel_with_its_shape_intact() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    const GRAPHIC: Graphic = Graphic(1);
    let (width, height) = (17u16, 23u16);

    // A picture with a hole in it: the middle column is absent, which is what
    // a sprite's shape is made of and what the pass has to discard rather than
    // draw black.
    let mut pixels = vec![Color16(0b0_00000_11111_00000); usize::from(width) * usize::from(height)];
    for row in 0..usize::from(height) {
        pixels[row * usize::from(width) + 8] = Color16::TRANSPARENT;
    }
    let art = Image::new(width, height, pixels.clone());
    let atlas = StaticAtlas::pack([(GRAPHIC, art)]).expect("one sprite fits");
    let sprite = atlas.sprite(GRAPHIC).expect("packed");

    let quads = [StaticQuad {
        x: 10.0,
        y: 20.0,
        width: f32::from(sprite.width),
        height: f32::from(sprite.height),
        region: sprite.region,
        depth: 0.5,
    }];
    let land = LandAtlas::pack([]).expect("nothing always fits");
    let texmaps = TexmapAtlas::pack([]).expect("nothing always fits");
    let frame = render_both(&device, &queue, &land, &texmaps, &[], &atlas, &quads, 128, 128);

    let (green_r, green_g, green_b) = Color16(0b0_00000_11111_00000).rgb8();
    let mut drawn = 0;
    for y in 0..128u32 {
        for x in 0..128u32 {
            let got = frame.pixel(x, y);
            let inside =
                (10..10 + u32::from(width)).contains(&x) && (20..20 + u32::from(height)).contains(&y);
            let transparent = inside && x - 10 == 8;
            if !inside || transparent {
                assert_eq!(got[3], 0, "({x}, {y}) should not have been drawn");
                continue;
            }
            assert_eq!(
                got,
                [green_r, green_g, green_b, u8::MAX],
                "({x}, {y}) is not the sprite's own pixel",
            );
            drawn += 1;
        }
    }
    // Every pixel of the rectangle except the absent column, and nothing else:
    // a sprite drawn at the wrong scale fails this even when every pixel it did
    // draw was the right colour.
    assert_eq!(drawn, usize::from(width - 1) * usize::from(height));
}

/// A hill in front hides a wall behind it.
///
/// The assertion the depth buffer exists for, and the one no pass order can
/// satisfy: all the ground is drawn before any static, so without a shared
/// depth every static would be in front of every tile. Both quads are built
/// here rather than read from a map, because what is being checked is the
/// *ordering*, and a real hillside would decide the geometry as well.
#[test]
fn ground_in_front_hides_a_static_behind_it() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    const GRAPHIC: Graphic = Graphic(1);
    let green = Color16(0b0_00000_11111_00000);
    let red = Color16(0b0_11111_00000_00000);

    let side = usize::from(LAND_TILE_SIZE);
    let land = LandAtlas::pack([(
        GRAPHIC,
        Image::new(LAND_TILE_SIZE, LAND_TILE_SIZE, vec![green; side * side]),
    )])
    .expect("one sprite fits");
    let texmaps = TexmapAtlas::pack([]).expect("nothing always fits");
    let statics = StaticAtlas::pack([(GRAPHIC, Image::new(60, 60, vec![red; 60 * 60]))]).expect("fits");
    let region = land.region(GRAPHIC).expect("packed");
    let sprite = statics.sprite(GRAPHIC).expect("packed");

    // Same pixels on screen, and the ground is nearer. A wall standing behind
    // a hill: the sprite's rectangle covers the tile's diamond entirely, so
    // every pixel of the diamond is a pixel both quads want.
    let ground = [GroundQuad {
        x: 64.0,
        y: 64.0,
        corners: [0.0; 4],
        region,
        texmap: None,
        depth: 0.4,
    }];
    let wall = [StaticQuad {
        x: 64.0 - 30.0,
        y: 64.0 - 30.0,
        width: f32::from(sprite.width),
        height: f32::from(sprite.height),
        region: sprite.region,
        depth: 0.6,
    }];
    let frame = render_both(
        &device, &queue, &land, &texmaps, &ground, &statics, &wall, 128, 128,
    );

    let (green_r, green_g, green_b) = green.rgb8();
    let mut ground_pixels = 0;
    for y in 0..128u32 {
        for x in 0..128u32 {
            if frame.pixel(x, y) == [green_r, green_g, green_b, u8::MAX] {
                ground_pixels += 1;
            }
        }
    }
    // The diamond, whole: not one of its pixels was overwritten by the static
    // that came after it.
    assert_eq!(ground_pixels, 1012, "the wall drew over the hill in front of it");

    // And the reverse ordering does the opposite, or the assertion above is
    // satisfied by a statics pass that draws nothing at all.
    let front = [StaticQuad {
        depth: 0.2,
        ..wall[0]
    }];
    let frame = render_both(
        &device, &queue, &land, &texmaps, &ground, &statics, &front, 128, 128,
    );
    let covered = (0..128u32)
        .flat_map(|y| (0..128u32).map(move |x| (x, y)))
        .filter(|&(x, y)| frame.pixel(x, y) == [green_r, green_g, green_b, u8::MAX])
        .count();
    assert_eq!(covered, 0, "a static in front left ground showing through");
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
        let wanted = ground::visible_graphics(&map, &camera);
        let atlas = LandAtlas::build(&art, wanted.iter().copied()).expect("fits");
        let texmaps = texmap_atlas(&dir, wanted);
        let quads = ground::collect(&map, &camera, &atlas, &texmaps);
        frames.push(
            render(
                &device,
                &queue,
                &atlas,
                &texmaps,
                &quads,
                camera.width,
                camera.height,
            )
            .pixels,
        );
    }
    assert_eq!(frames[0], frames[1]);

    // And a different camera is a different frame — otherwise the assertion
    // above is satisfied by a renderer that draws nothing at all.
    let moved = Camera::new(Point::new(1497, 1629, 0), 768, 512);
    let wanted = ground::visible_graphics(&map, &moved);
    let atlas = LandAtlas::build(&art, wanted.iter().copied()).expect("fits");
    let texmaps = texmap_atlas(&dir, wanted);
    let quads = ground::collect(&map, &moved, &atlas, &texmaps);
    let other = render(
        &device,
        &queue,
        &atlas,
        &texmaps,
        &quads,
        moved.width,
        moved.height,
    );
    assert_ne!(frames[0], other.pixels, "moving the camera changed nothing");
}

/// A screen of Britain with its statics on it: the buildings cover a real part
/// of the frame, and the ground still covers all of it.
///
/// Two claims in one, and they are the two ways this layer fails as a whole.
/// Statics covering nothing means the sprites, the atlas or the placement
/// dropped everything and the frame is the old ground-only one; ground no
/// longer covering the viewport means the depth buffer or the second pass took
/// pixels away from it, which is a hole in the world rather than a wall.
#[test]
fn britains_statics_cover_part_of_a_frame_that_is_still_whole() {
    let (Some(dir), Some((device, queue))) = (client_dir(), gpu()) else {
        return;
    };
    let map = Map::load_facet(&dir, 0).expect("Felucca");
    let art = Art::open(&dir).expect("artLegacyMUL.uop");
    let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");
    let camera = Camera::new(Point::new(1495, 1629, 0), 768, 512);

    let wanted = ground::visible_graphics(&map, &camera);
    let land = LandAtlas::build(&art, wanted.iter().copied()).expect("fits");
    let texmaps = texmap_atlas(&dir, wanted);
    let quads = ground::collect(&map, &camera, &land, &texmaps);

    let wanted_statics = statics::visible_graphics(&map, &camera);
    let static_atlas = StaticAtlas::build(&art, wanted_statics).expect("a screen of statics fits");
    let static_quads = statics::collect(&map, &camera, &tiledata, &static_atlas);
    assert!(
        static_quads.len() > 500,
        "only {} statics in the middle of Britain",
        static_quads.len(),
    );

    let ground_only = render(
        &device,
        &queue,
        &land,
        &texmaps,
        &quads,
        camera.width,
        camera.height,
    );
    let frame = render_both(
        &device,
        &queue,
        &land,
        &texmaps,
        &quads,
        &static_atlas,
        &static_quads,
        camera.width,
        camera.height,
    );

    // Still whole: every pixel drawn, exactly as with ground alone.
    let total = (camera.width * camera.height) as usize;
    assert_eq!(frame.drawn(), total, "the statics pass left holes in the world");

    // And a real part of it changed. A tenth is a floor rather than a
    // measurement — the point is that it is not a handful of pixels, which is
    // what a placement off by a tile or an atlas that packed nothing produces.
    let changed = (0..camera.height)
        .flat_map(|y| (0..camera.width).map(move |x| (x, y)))
        .filter(|&(x, y)| frame.pixel(x, y) != ground_only.pixel(x, y))
        .count();
    assert!(
        changed > total / 10,
        "the statics changed only {changed} of {total} pixels",
    );
}

/// Write a frame of Britain out as a picture, for a person to look at.
///
/// Ignored: it is not an assertion, it is the eye. Every other test here counts
/// pixels, and counting is what catches a sprite sampled one texel over — it is
/// not what catches ground that is the right shape and the wrong terrain. Run it
/// with a client and look:
///
/// ```sh
/// OPENSHARD_CLIENT=… cargo test -p openshard-client-render --test frame -- \
///     --ignored dump_a_frame
/// ```
///
/// Plain PPM so that nothing has to be added to the workspace to write it.
#[test]
#[ignore = "writes a picture for a person, and asserts nothing"]
fn dump_a_frame_of_britain() {
    let (Some(dir), Some((device, queue))) = (client_dir(), gpu()) else {
        return;
    };
    let map = Map::load_facet(&dir, 0).expect("Felucca");
    let art = Art::open(&dir).expect("artLegacyMUL.uop");
    let camera = Camera::new(Point::new(1495, 1629, 0), 768, 512);

    let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");
    let wanted = ground::visible_graphics(&map, &camera);
    let atlas = LandAtlas::build(&art, wanted.iter().copied()).expect("fits");
    let texmaps = texmap_atlas(&dir, wanted);
    let quads = ground::collect(&map, &camera, &atlas, &texmaps);

    let static_atlas =
        StaticAtlas::build(&art, statics::visible_graphics(&map, &camera)).expect("statics fit");
    let static_quads = statics::collect(&map, &camera, &tiledata, &static_atlas);

    let frame = render_both(
        &device,
        &queue,
        &atlas,
        &texmaps,
        &quads,
        &static_atlas,
        &static_quads,
        camera.width,
        camera.height,
    );

    let mut ppm = format!("P6\n{} {}\n255\n", camera.width, camera.height).into_bytes();
    for pixel in frame.pixels.chunks_exact(4) {
        ppm.extend_from_slice(&pixel[..3]);
    }
    let path = std::env::var_os("OPENSHARD_FRAME_DUMP")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("britain.ppm"));
    std::fs::write(&path, ppm).expect("writing the frame");
    eprintln!("wrote {}", path.display());
}
