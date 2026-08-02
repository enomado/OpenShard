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

use openshard_client_render::animate::StaticAnimations;
use openshard_client_render::atlas::FrameKey;
use openshard_client_render::atlas::{AnimAtlas, LandAtlas, StaticAtlas, TexmapAtlas};
use openshard_client_render::blit::{Blit, ViewportRect};
use openshard_client_render::camera::{Camera, Projection, WorldPoint, Zoom};
use openshard_client_render::cutaway::Cutaway;
use openshard_client_render::geometry::{Rect, Vec2};
use openshard_client_render::ground::{self, GroundQuad};
use openshard_client_render::hue::HueRamp;
use openshard_client_render::light::{Light, Lighting};
use openshard_client_render::mobiles::{self, Mobile};
use openshard_client_render::outline::{self, Outline, Ring};
use openshard_client_render::place::Place;
use openshard_client_render::renderer::{self, GroundRenderer, SpriteRenderer, Target};
use openshard_client_render::sprite::SpriteQuad;
use openshard_client_render::statics;
use openshard_protocol::direction::Direction;
use openshard_protocol::wire::Graphic;
use openshard_protocol::world::Point;
use openshard_uofiles::anim::{Anim, AnimFrame};
use openshard_uofiles::art::{Art, LAND_TILE_SIZE, land_row};
use openshard_uofiles::color::Color16;
use openshard_uofiles::equipconv::EquipConv;
use openshard_uofiles::hues::Hues;
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
    let no_mobiles = AnimAtlas::pack([]).expect("nothing always fits");
    render_both(
        device,
        queue,
        atlas,
        texmaps,
        quads,
        &empty,
        &[],
        (no_mobiles.pixels(), &[]),
        width,
        height,
        Projection::one_to_one(width, height),
    )
}

/// Draw ground through a camera's own projection, rather than 1:1.
///
/// The magnified path, which [`render`] cannot reach: it is the same ground pass
/// and the same quads, and what differs is the two numbers the vertex shader
/// ends on.
#[allow(clippy::too_many_arguments)]
fn render_projected(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &LandAtlas,
    texmaps: &TexmapAtlas,
    quads: &[GroundQuad],
    width: u32,
    height: u32,
    camera: Camera,
) -> Frame {
    let empty = StaticAtlas::pack([]).expect("nothing always fits");
    let no_mobiles = AnimAtlas::pack([]).expect("nothing always fits");
    render_both(
        device,
        queue,
        atlas,
        texmaps,
        quads,
        &empty,
        &[],
        (no_mobiles.pixels(), &[]),
        width,
        height,
        camera.projection(),
    )
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
    static_quads: &[SpriteQuad],
    mobiles: (&[u8], &[SpriteQuad]),
    width: u32,
    height: u32,
    projection: Projection,
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
    let place = openshard_client_render::place::texture(device, width, height);
    let place_view = place.create_view(&wgpu::TextureViewDescriptor::default());

    // None of these frames ask for a hue — every quad built below carries
    // `hue: 0` — so an empty ramp is a real texture the shader can bind rather
    // than a special case: it is never indexed because nothing here sets the
    // bit that would make the shader look.
    let hue_ramp = HueRamp::build(&Hues::parse(&[0u8; 708]).expect("one empty group"));

    let mut renderer = GroundRenderer::new(device, queue, format, atlas, texmaps);
    let mut statics = SpriteRenderer::new(device, queue, format, static_atlas.pixels(), &hue_ramp);
    // The mobiles are the same pass again with another atlas bound, which is
    // the whole of the difference between a static and a creature on the GPU.
    let mut people = SpriteRenderer::new(device, queue, format, mobiles.0, &hue_ramp);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let target_view = Target {
        place: &place_view,
        view: &view,
        depth: &depth_view,
        width,
        height,
        projection,
    };
    renderer.render(device, queue, &mut encoder, target_view, quads);
    statics.render(device, queue, &mut encoder, target_view, static_quads);
    people.render(device, queue, &mut encoder, target_view, mobiles.1);
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
        place: Place::land(0, 0),
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
    let quads = ground::collect(&map, &camera, &atlas, &texmaps, &Cutaway::OPEN);
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
    let quads = ground::collect(&map, &camera, &atlas, &texmaps, &Cutaway::OPEN);

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
            place: Place::land(1, 1),
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
            place: Place::land(2, 2),
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

/// Read an RGBA8 texture back into a [`Frame`].
///
/// `width * 4` must be a multiple of 256, the row alignment a buffer copy
/// demands. Split out of [`render_both`] because the blit test reads two
/// textures — the world image and the surface it was blitted onto — and
/// comparing them is the whole assertion.
fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Frame {
    let (width, height) = (texture.width(), texture.height());
    assert_eq!(width * 4 % 256, 0, "a row copy has to be 256-byte aligned");
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(width) * u64::from(height) * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
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

/// At zoom 1 the blit is a copy, byte for byte.
///
/// The property every pixel-exact assertion in this file depends on now that the
/// world is drawn offscreen and stretched onto the surface: if the blit is not
/// the identity at 1:1, then every other test here is measuring an image the
/// screen never shows. A half-texel of sampling error, a flipped vertical axis
/// or a filter left on all read as "slightly soft" on a screenshot and are exact
/// here.
///
/// No client files: the scene is two coloured diamonds made in memory, which is
/// enough to have edges — a flat field of one colour would survive any of those
/// mistakes.
#[test]
fn the_blit_at_zoom_one_is_the_world_image_texel_for_texel() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    const GRAPHIC: Graphic = Graphic(1);
    let side = usize::from(LAND_TILE_SIZE);
    let art = Image::new(
        LAND_TILE_SIZE,
        LAND_TILE_SIZE,
        (0..side * side)
            // A gradient rather than a wash: a filter left on averages
            // neighbours, and neighbours that differ are what makes that
            // visible.
            .map(|at| Color16(((at % 31) as u16) << 10 | ((at / 31 % 31) as u16) << 5 | 1))
            .collect(),
    );
    let atlas = LandAtlas::pack([(GRAPHIC, art)]).expect("one sprite fits");
    let texmaps = TexmapAtlas::pack([]).expect("nothing always fits");
    let region = atlas.region(GRAPHIC).expect("packed");
    let quads: Vec<GroundQuad> = [(40.0, 40.0), (150.0, 96.0)]
        .into_iter()
        .map(|(x, y)| GroundQuad {
            x,
            y,
            corners: [0.0; 4],
            region,
            texmap: None,
            depth: 0.5,
            place: Place::land(1, 1),
        })
        .collect();

    let (width, height) = (256, 256);
    let world = openshard_client_render::blit::world_texture(&device, width, height);
    let world_view = world.create_view(&wgpu::TextureViewDescriptor::default());
    let depth = renderer::depth_texture(&device, width, height);
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let place = openshard_client_render::place::texture(&device, width, height);
    let place_view = place.create_view(&wgpu::TextureViewDescriptor::default());
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut ground_pass = GroundRenderer::new(&device, &queue, format, &atlas, &texmaps);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    ground_pass.render(
        &device,
        &queue,
        &mut encoder,
        Target::whole(&world_view, &depth_view, &place_view, width, height),
        &quads,
    );
    queue.submit([encoder.finish()]);

    // The surface stands in for a window: same size, so a zoom of 1 asks the
    // blit for exactly the identity.
    let surface = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("surface"),
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
    let surface_view = surface.create_view(&wgpu::TextureViewDescriptor::default());
    let blit = Blit::new(&device, format);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    blit.render(
        &device,
        &queue,
        &mut encoder,
        // Qualified: this file has a `Frame` of its own, which is a read-back
        // picture rather than a blit's arguments.
        openshard_client_render::blit::Frame {
            target: &surface_view,
            world: &world_view,
            zoom: Zoom::ONE,
            rect: ViewportRect {
                x: 0,
                y: 0,
                width,
                height,
            },
        },
        // The identity: this test is about the blit being a copy, and lighting
        // is a multiplication by one for it.
        &Lighting::NONE,
    );
    queue.submit([encoder.finish()]);

    let drawn = read_back(&device, &queue, &world);
    let blitted = read_back(&device, &queue, &surface);

    // The scene has to be worth comparing. Two diamonds of a gradient cover a
    // couple of thousand pixels of a 65,536-pixel frame, and an empty frame
    // would compare equal to another empty frame.
    assert!(
        drawn.drawn() > 2000,
        "the world image holds only {} drawn pixels",
        drawn.drawn(),
    );
    for y in 0..height {
        for x in 0..width {
            assert_eq!(
                blitted.pixel(x, y),
                drawn.pixel(x, y),
                "({x}, {y}) came out of the blit changed",
            );
        }
    }
}

/// A light brightens the pixels under it and nothing else, and the ambient
/// darkens everything the light does not reach.
///
/// The only oracle the lighting shader has: everything else about it is CPU
/// arithmetic with tests of its own in `light.rs`, and the part that is neither
/// — the falloff, the multiply, the loop bound by a count in a uniform — exists
/// only as WGSL and can only be read back off a GPU.
///
/// The scene is a flat grey world image, deliberately: this test is about what
/// the *lighting* did to a pixel, and a gradient underneath would make every
/// comparison between two pixels a statement about the art as well.
#[test]
fn a_light_brightens_its_own_pool_and_the_ambient_darkens_the_rest() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let (width, height) = (256, 256);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let world = openshard_client_render::blit::world_texture(&device, width, height);
    let world_view = world.create_view(&wgpu::TextureViewDescriptor::default());
    let depth = renderer::depth_texture(&device, width, height);
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let place = openshard_client_render::place::texture(&device, width, height);
    let place_view = place.create_view(&wgpu::TextureViewDescriptor::default());

    // A flat grey field: one land graphic whose art is a single value, drawn
    // over the whole frame. Mid-grey and not white, so that "brighter" is
    // expressible in both directions.
    const GRAPHIC: Graphic = Graphic(1);
    let side = usize::from(LAND_TILE_SIZE);
    let grey = Color16(15 << 10 | 15 << 5 | 15);
    let art = Image::new(LAND_TILE_SIZE, LAND_TILE_SIZE, vec![grey; side * side]);
    let atlas = LandAtlas::pack([(GRAPHIC, art)]).expect("one sprite fits");
    let texmaps = TexmapAtlas::pack([]).expect("nothing always fits");
    let region = atlas.region(GRAPHIC).expect("packed");
    let mut quads = Vec::new();
    for y in (0..height as i32 + 44).step_by(22) {
        for x in (-44..width as i32 + 44).step_by(44) {
            quads.push(GroundQuad {
                x: (x + (y / 22 % 2) * 22) as f32,
                y: y as f32,
                corners: [0.0; 4],
                region,
                texmap: None,
                depth: 0.5,
                place: Place::land(1, 1),
            });
        }
    }
    let mut ground_pass = GroundRenderer::new(&device, &queue, format, &atlas, &texmaps);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    ground_pass.render(
        &device,
        &queue,
        &mut encoder,
        Target::whole(&world_view, &depth_view, &place_view, width, height),
        &quads,
    );
    queue.submit([encoder.finish()]);

    let surface = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("surface"),
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
    let surface_view = surface.create_view(&wgpu::TextureViewDescriptor::default());
    let blit = Blit::new(&device, format);

    // One flame in the middle, reaching a quarter of the frame.
    let lighting = Lighting {
        image: Vec2::new(width as f32, height as f32),
        ambient: openshard_client_render::light::NIGHT,
        lights: vec![Light {
            at: Vec2::new(128.0, 128.0),
            radius: 64.0,
            color: [1.0, 0.7, 0.35],
            intensity: 1.0,
        }],
    };
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    blit.render(
        &device,
        &queue,
        &mut encoder,
        openshard_client_render::blit::Frame {
            target: &surface_view,
            world: &world_view,
            zoom: Zoom::ONE,
            rect: ViewportRect {
                x: 0,
                y: 0,
                width,
                height,
            },
        },
        &lighting,
    );
    queue.submit([encoder.finish()]);

    let drawn = read_back(&device, &queue, &world);
    let lit = read_back(&device, &queue, &surface);
    // The scene has to have something in it, or every comparison below is
    // between two black pixels and holds for any shader at all.
    assert!(drawn.drawn() > 60_000, "the world image is mostly empty");

    let luma = |pixel: [u8; 4]| u32::from(pixel[0]) + u32::from(pixel[1]) + u32::from(pixel[2]);
    let centre = luma(lit.pixel(128, 128));
    // Half way out, not at the rim: the falloff is quadratic, so the last
    // fifth of the radius contributes less than a byte of the eight bits this
    // is read back at and would compare equal to the dark outside it.
    let edge_of_pool = luma(lit.pixel(128 + 32, 128));
    // Well outside the radius, and outside it in both axes so that a light
    // placed at the wrong one of them still fails here.
    let far = luma(lit.pixel(16, 240));
    let unlit = luma(drawn.pixel(16, 240));

    assert!(
        far < unlit,
        "the ambient did not darken the frame: {far} against {unlit}"
    );
    assert!(
        centre > far * 2,
        "the pool is not brighter than the dark around it: {centre} against {far}",
    );
    assert!(
        edge_of_pool > far && edge_of_pool < centre,
        "the falloff is not monotonic: centre {centre}, edge {edge_of_pool}, outside {far}",
    );
    // The pool is warm, not white: a light whose colour was dropped would pass
    // every brightness assertion above.
    let middle = lit.pixel(128, 128);
    assert!(
        middle[0] > middle[2],
        "the light's colour was ignored: {middle:?}",
    );
    // And nothing outside the radius is touched by the light at all — the
    // ambient alone accounts for it, which is what makes the pool a shape.
    let outside = lit.pixel(16, 240);
    for (channel, (got, (drawn, ambient))) in outside
        .iter()
        .zip(
            drawn
                .pixel(16, 240)
                .iter()
                .zip(openshard_client_render::light::NIGHT),
        )
        .take(3)
        .enumerate()
    {
        let expected = (f32::from(*drawn) * ambient).round() as i32;
        assert!(
            (i32::from(*got) - expected).abs() <= 1,
            "outside the pool, channel {channel} is {got} against the ambient's {expected}",
        );
    }
}

/// The world passes draw into the world texture on a surface that is not
/// `Rgba8Unorm`.
///
/// The surface's format and the world texture's are two different values, and
/// the frame here is the arrangement that makes them differ: an HDR display
/// offers `Rgba16Float` first among its non-sRGB formats, and a world pipeline
/// built from the surface's format instead of `blit::WORLD_FORMAT` fails
/// validation at `set_pipeline` — the whole client dies on the first frame with
/// nothing drawn. Nothing is read back: the assertion is that the submission
/// validates at all, and a mismatch panics inside `wgpu` before it returns.
#[test]
fn the_world_passes_are_built_for_the_world_texture_not_the_surface() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let (width, height) = (64, 64);
    let world = openshard_client_render::blit::world_texture(&device, width, height);
    let world_view = world.create_view(&wgpu::TextureViewDescriptor::default());
    let depth = renderer::depth_texture(&device, width, height);
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let place = openshard_client_render::place::texture(&device, width, height);
    let place_view = place.create_view(&wgpu::TextureViewDescriptor::default());

    // A sprite made here rather than read from a client, and one real quad: a
    // pass handed nothing returns before it binds its pipeline, which is the
    // one step this test is about.
    const GRAPHIC: Graphic = Graphic(1);
    let art = Image::new(8, 8, vec![Color16(0b0_00000_11111_00000); 64]);
    let atlas = StaticAtlas::pack([(GRAPHIC, art)]).expect("one sprite fits");
    let sprite = atlas.sprite(GRAPHIC).expect("packed");
    let quads = [SpriteQuad {
        rect: Rect {
            x: 4.0,
            y: 4.0,
            width: f32::from(sprite.width),
            height: f32::from(sprite.height),
        },
        region: sprite.region,
        depth: 0.5,
        hue: 0,
        place: Place::NOWHERE,
    }];
    let hue_ramp = HueRamp::build(&Hues::parse(&[0u8; 708]).expect("one empty group"));
    let mut sprites = SpriteRenderer::new(
        &device,
        &queue,
        openshard_client_render::blit::WORLD_FORMAT,
        atlas.pixels(),
        &hue_ramp,
    );
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    sprites.render(
        &device,
        &queue,
        &mut encoder,
        Target::whole(&world_view, &depth_view, &place_view, width, height),
        &quads,
    );

    // The stand-in for the HDR surface, in the format the blit and the HUD —
    // and only they — are built for.
    let surface_format = wgpu::TextureFormat::Rgba16Float;
    let surface = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("surface"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: surface_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let surface_view = surface.create_view(&wgpu::TextureViewDescriptor::default());
    let blit = Blit::new(&device, surface_format);
    blit.render(
        &device,
        &queue,
        &mut encoder,
        // Qualified: this file has a `Frame` of its own, which is a read-back
        // picture rather than a blit's arguments.
        openshard_client_render::blit::Frame {
            target: &surface_view,
            world: &world_view,
            zoom: Zoom::ONE,
            rect: ViewportRect {
                x: 0,
                y: 0,
                width,
                height,
            },
        },
        // The identity: this test is about the blit being a copy, and lighting
        // is a multiplication by one for it.
        &Lighting::NONE,
    );
    queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("waiting on our own submission");
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

    let quads = [SpriteQuad {
        rect: Rect {
            x: 10.0,
            y: 20.0,
            width: f32::from(sprite.width),
            height: f32::from(sprite.height),
        },
        region: sprite.region,
        depth: 0.5,
        hue: 0,
        place: Place::NOWHERE,
    }];
    let land = LandAtlas::pack([]).expect("nothing always fits");
    let texmaps = TexmapAtlas::pack([]).expect("nothing always fits");
    let none = AnimAtlas::pack([]).expect("nothing always fits");
    let frame = render_both(
        &device,
        &queue,
        &land,
        &texmaps,
        &[],
        &atlas,
        &quads,
        (none.pixels(), &[]),
        128,
        128,
        Projection::one_to_one(128, 128),
    );

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

/// One `hues.mul` group, `Hue(1)`'s ramp set to `colors` and the other seven
/// entries left at zero — the same construction [`crate`]'s own unit tests
/// cannot reuse across crates, so this test file builds its own bytes from the
/// documented layout rather than from a private helper.
fn one_hue_group(colors: [Color16; 32]) -> Hues {
    const ENTRY_BYTES: usize = 32 * 2 + 2 + 2 + 20;
    let mut bytes = vec![0u8; 4 + 8 * ENTRY_BYTES];
    for (index, color) in colors.iter().enumerate() {
        let at = 4 + index * 2;
        bytes[at..at + 2].copy_from_slice(&color.0.to_le_bytes());
    }
    Hues::parse(&bytes).expect("one whole group")
}

/// The art is not tinted, it is replaced: a full hue looks a pixel up by its
/// own red channel and draws whatever `hues.mul` says, discarding the pixel's
/// original colour entirely — even a pixel that was never grey.
///
/// Both texels below carry the same 5-bit red value and different green and
/// blue, so a shader that multiplied by a tint would leave them visibly
/// different; one that replaces them by index draws them identically.
#[test]
fn a_full_hue_replaces_the_pixel_by_its_red_channel_regardless_of_its_own_colour() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    const GRAPHIC: Graphic = Graphic(1);
    const INDEX: u8 = 10;

    // Genuinely grey — all three channels equal `INDEX` — against a texel
    // whose red channel is the same `INDEX` but whose green and blue are not:
    // "partial" is decided by the *pixel*, not by the index alone, so the two
    // have to share an index and differ in colour for the test to mean anything.
    let index = u16::from(INDEX);
    let grey = Color16((index << 10) | (index << 5) | index);
    let coloured = Color16((index << 10) | 0b0_00000_00000_11111);
    assert_ne!(
        grey, coloured,
        "the two texels have to differ for this to test anything"
    );

    let art = Image::new(2, 1, vec![grey, coloured]);
    let atlas = StaticAtlas::pack([(GRAPHIC, art)]).expect("one sprite fits");
    let sprite = atlas.sprite(GRAPHIC).expect("packed");

    let mut ramp_colors = [Color16::TRANSPARENT; 32];
    ramp_colors[usize::from(INDEX)] = Color16(0b0_00000_00000_11111); // pure blue
    let hues = one_hue_group(ramp_colors);
    let hue_ramp = HueRamp::build(&hues);

    let land = LandAtlas::pack([]).expect("nothing always fits");
    let texmaps = TexmapAtlas::pack([]).expect("nothing always fits");

    let quad = |hue: u32| SpriteQuad {
        rect: Rect {
            x: 0.0,
            y: 0.0,
            width: f32::from(sprite.width),
            height: f32::from(sprite.height),
        },
        region: sprite.region,
        depth: 0.5,
        hue,
        place: Place::NOWHERE,
    };

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let render_with_ramp = |hue: u32| -> Frame {
        let quads = [quad(hue)];
        render_hued(
            &device, &queue, &land, &texmaps, &atlas, &quads, &hue_ramp, format,
        )
    };

    let (blue_r, blue_g, blue_b) = Color16(0b0_00000_00000_11111).rgb8();
    let (grey_r, grey_g, grey_b) = grey.rgb8();
    let (coloured_r, coloured_g, coloured_b) = coloured.rgb8();

    // Hue 1, no partial flag: both texels come back as the ramp's own colour,
    // not as anything blended with what was there.
    let full = render_with_ramp(1);
    assert_eq!(
        full.pixel(0, 0),
        [blue_r, blue_g, blue_b, u8::MAX],
        "the grey texel"
    );
    assert_eq!(
        full.pixel(1, 0),
        [blue_r, blue_g, blue_b, u8::MAX],
        "the coloured texel too — a full hue does not ask what a pixel looked like",
    );

    // The same hue, partial: only the grey texel is grey enough to tint: the
    // coloured one is left exactly as the art drew it.
    let partial = render_with_ramp(1 | 0x8000);
    assert_eq!(
        partial.pixel(0, 0),
        [blue_r, blue_g, blue_b, u8::MAX],
        "partial still tints a genuinely grey pixel",
    );
    assert_eq!(
        partial.pixel(1, 0),
        [coloured_r, coloured_g, coloured_b, u8::MAX],
        "partial leaves an already-coloured pixel alone",
    );

    // And hue 0 is "no hue": the ramp exists but nothing here samples it.
    let none_hue = render_with_ramp(0);
    assert_eq!(none_hue.pixel(0, 0), [grey_r, grey_g, grey_b, u8::MAX]);
    assert_eq!(
        none_hue.pixel(1, 0),
        [coloured_r, coloured_g, coloured_b, u8::MAX]
    );
}

/// Draw one static pass with a real hue ramp bound, and read the result back.
///
/// [`render_both`] always binds an empty ramp — nothing it draws asks for a
/// hue — so a test that needs a real one calls this instead rather than
/// growing every caller of `render_both` an argument none of them use.
#[allow(clippy::too_many_arguments)]
fn render_hued(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    land: &LandAtlas,
    texmaps: &TexmapAtlas,
    static_atlas: &StaticAtlas,
    quads: &[SpriteQuad],
    hue_ramp: &HueRamp,
    format: wgpu::TextureFormat,
) -> Frame {
    let (width, height) = (64u32, 64u32);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hued frame"),
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
        label: Some("hued readback"),
        size: u64::from(width) * u64::from(height) * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let depth = renderer::depth_texture(device, width, height);
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let place = openshard_client_render::place::texture(device, width, height);
    let place_view = place.create_view(&wgpu::TextureViewDescriptor::default());

    let mut ground = GroundRenderer::new(device, queue, format, land, texmaps);
    let mut statics = SpriteRenderer::new(device, queue, format, static_atlas.pixels(), hue_ramp);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let target_view = Target::whole(&view, &depth_view, &place_view, width, height);
    ground.render(device, queue, &mut encoder, target_view, &[]);
    statics.render(device, queue, &mut encoder, target_view, quads);
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

/// Every pixel says which tile it came from, and a wall's pixels say the
/// wall's tile rather than the ground's.
///
/// The attachment `docs/lighting.md` turns on, and the claim that makes it worth
/// having: a wall's picture stands 44 pixels above the tile it is on, so the
/// ground behind it and the wall itself are neighbouring pixels of one image
/// that belong to different tiles at different heights. Everything the lighting
/// pass does rests on being able to tell those two apart, and nothing else in
/// this suite would notice if the channel held the ground's tile everywhere.
#[test]
fn every_pixel_names_the_tile_it_came_from() {
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
    let statics = StaticAtlas::pack([(GRAPHIC, Image::new(20, 20, vec![red; 20 * 20]))]).expect("fits");
    let region = land.region(GRAPHIC).expect("packed");
    let sprite = statics.sprite(GRAPHIC).expect("packed");

    // The ground tile fills the middle of the image; the wall stands on the
    // *next* tile at a height of its own and is drawn over part of it, which is
    // exactly the pair of pixels this is about.
    let ground = [GroundQuad {
        x: 64.0,
        y: 64.0,
        corners: [0.0; 4],
        region,
        texmap: None,
        depth: 0.6,
        place: Place::land(300, 400),
    }];
    let wall = [SpriteQuad {
        rect: Rect {
            x: 60.0,
            y: 60.0,
            width: f32::from(sprite.width),
            height: f32::from(sprite.height),
        },
        region: sprite.region,
        depth: 0.4,
        hue: 0,
        place: Place::of_static(Point::new(301, 400, 15)),
    }];

    let places = render_places(&device, &queue, &land, &texmaps, &ground, &statics, &wall, 128);

    // A pixel of the wall: its own tile, its own height, and the static's kind.
    assert_eq!(
        places.at(64, 64),
        [301, 400, (15 + 128) as u16, 2],
        "a wall's pixel named something else",
    );
    // A pixel of the ground beside it: the tile under the wall, at the height
    // the corners gave it, and the land kind.
    assert_eq!(
        places.at(64, 84),
        [300, 400, 128, 1],
        "the ground beside the wall named something else",
    );
    // And a corner nothing was drawn on stays the clear value, whose kind is
    // `Nothing` — a background the lighting must leave alone.
    assert_eq!(places.at(2, 2)[3], 0, "an untouched pixel claimed a tile");
}

/// Draw ground and one sprite and read back the *place* attachment rather than
/// the picture. `size * 8` must be a multiple of 256, as every readback here.
#[allow(clippy::too_many_arguments)]
fn render_places(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &LandAtlas,
    texmaps: &TexmapAtlas,
    quads: &[GroundQuad],
    static_atlas: &StaticAtlas,
    static_quads: &[SpriteQuad],
    size: u32,
) -> Places {
    assert_eq!(size * 8 % 256, 0, "a row copy has to be 256-byte aligned");
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let world = openshard_client_render::blit::world_texture(device, size, size);
    let world_view = world.create_view(&wgpu::TextureViewDescriptor::default());
    let depth = renderer::depth_texture(device, size, size);
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let place = openshard_client_render::place::texture(device, size, size);
    let place_view = place.create_view(&wgpu::TextureViewDescriptor::default());
    let hue_ramp = HueRamp::build(&Hues::parse(&[0u8; 708]).expect("one empty group"));

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("places"),
        size: u64::from(size) * u64::from(size) * 8,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut ground_pass = GroundRenderer::new(device, queue, format, atlas, texmaps);
    let mut sprite_pass = SpriteRenderer::new(device, queue, format, static_atlas.pixels(), &hue_ramp);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let target = Target::whole(&world_view, &depth_view, &place_view, size, size);
    ground_pass.render(device, queue, &mut encoder, target, quads);
    sprite_pass.render(device, queue, &mut encoder, target, static_quads);
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &place,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 8),
                rows_per_image: Some(size),
            },
        },
        wgpu::Extent3d {
            width: size,
            height: size,
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
    Places { width: size, bytes }
}

/// The place attachment read back: four `u16` channels a texel.
struct Places {
    width: u32,
    bytes: Vec<u8>,
}

impl Places {
    /// `(x, y, z + 128, kind)` at one pixel.
    fn at(&self, x: u32, y: u32) -> [u16; 4] {
        let start = ((y * self.width + x) * 8) as usize;
        let mut out = [0u16; 4];
        for (channel, slot) in out.iter_mut().enumerate() {
            let at = start + channel * 2;
            *slot = u16::from_le_bytes([self.bytes[at], self.bytes[at + 1]]);
        }
        out
    }
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
        place: Place::land(1, 1),
    }];
    let wall = [SpriteQuad {
        rect: Rect {
            x: 64.0 - 30.0,
            y: 64.0 - 30.0,
            width: f32::from(sprite.width),
            height: f32::from(sprite.height),
        },
        region: sprite.region,
        depth: 0.6,
        hue: 0,
        place: Place::NOWHERE,
    }];
    let none = AnimAtlas::pack([]).expect("nothing always fits");
    let frame = render_both(
        &device,
        &queue,
        &land,
        &texmaps,
        &ground,
        &statics,
        &wall,
        (none.pixels(), &[]),
        128,
        128,
        Projection::one_to_one(128, 128),
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
    let front = [SpriteQuad {
        depth: 0.2,
        ..wall[0]
    }];
    let frame = render_both(
        &device,
        &queue,
        &land,
        &texmaps,
        &ground,
        &statics,
        &front,
        (none.pixels(), &[]),
        128,
        128,
        Projection::one_to_one(128, 128),
    );
    let covered = (0..128u32)
        .flat_map(|y| (0..128u32).map(move |x| (x, y)))
        .filter(|&(x, y)| frame.pixel(x, y) == [green_r, green_g, green_b, u8::MAX])
        .count();
    assert_eq!(covered, 0, "a static in front left ground showing through");
}

/// Two things at one depth are decided by which is drawn later, and that is the
/// client's own tie-break rather than an accident of the pass order.
///
/// `Chunk.AddGameObject` inserts by `PriorityZ` and, on a tie, puts the land
/// tile *first* in the per-tile list — so the flagstone lying at exactly the
/// height of the ground under it is drawn second, and covers it. Here that is
/// `LessEqual` in `renderer::depth_state` plus the order the passes already
/// run in: the ground pass, then the statics, then the mobiles.
///
/// It needs a frame because the depth *state* is what is being asserted. Every
/// number this crate computes can be right and this still be backwards, and
/// under `Less` it was: the depths agreed with the client and the first writer
/// kept the pixel, so the ground won every tie it should have lost.
#[test]
fn at_one_depth_the_later_pass_wins() {
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

    // The same depth, to the bit: not "very close", which the test would pass
    // under either comparison.
    const TIED: f32 = 0.5;
    let ground = [GroundQuad {
        x: 64.0,
        y: 64.0,
        corners: [0.0; 4],
        region,
        texmap: None,
        depth: TIED,
        place: Place::land(1, 1),
    }];
    let flagstone = [SpriteQuad {
        rect: Rect {
            x: 64.0 - 30.0,
            y: 64.0 - 30.0,
            width: f32::from(sprite.width),
            height: f32::from(sprite.height),
        },
        region: sprite.region,
        depth: TIED,
        hue: 0,
        place: Place::NOWHERE,
    }];
    let none = AnimAtlas::pack([]).expect("nothing always fits");
    let frame = render_both(
        &device,
        &queue,
        &land,
        &texmaps,
        &ground,
        &statics,
        &flagstone,
        (none.pixels(), &[]),
        128,
        128,
        Projection::one_to_one(128, 128),
    );

    let (green_r, green_g, green_b) = green.rgb8();
    let showing = (0..128u32)
        .flat_map(|y| (0..128u32).map(move |x| (x, y)))
        .filter(|&(x, y)| frame.pixel(x, y) == [green_r, green_g, green_b, u8::MAX])
        .count();
    assert_eq!(showing, 0, "the ground kept a pixel from the static tied with it");

    // And the static really covered those pixels rather than the frame being
    // empty: the sprite's whole rectangle is its own colour.
    let (red_r, red_g, red_b) = red.rgb8();
    let covered = (0..128u32)
        .flat_map(|y| (0..128u32).map(move |x| (x, y)))
        .filter(|&(x, y)| frame.pixel(x, y) == [red_r, red_g, red_b, u8::MAX])
        .count();
    assert_eq!(covered, 60 * 60, "the static did not draw its whole rectangle");
}

/// A mobile is drawn from its own atlas, in front of the ground it stands on,
/// and a mirrored facing is the same picture backwards.
///
/// The mirror is what needs a frame rather than an assertion on numbers: the
/// region arithmetic is checked in `sprite`, but whether a *negative* region
/// width actually samples backwards is the GPU's answer and not ours. A shader
/// that clamped it instead would leave every west-facing creature looking east,
/// which is a bug a screenshot of one direction cannot show.
#[test]
fn a_mobile_is_drawn_over_the_ground_and_mirrors_with_its_facing() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    const BODY: u16 = 400;
    let green = Color16(0b0_00000_11111_00000);
    let red = Color16(0b0_11111_00000_00000);

    // A two-pixel-wide frame: red on the left, green on the right. Mirrored,
    // the two swap, and nothing else about the quad changes.
    let frame = AnimFrame {
        center_x: 1,
        center_y: 0,
        image: Image::new(2, 1, vec![red, green]),
    };
    let atlas = AnimAtlas::pack([(
        FrameKey {
            body: BODY,
            group: 4,
            direction: 1,
            frame: 0,
        },
        frame,
    )])
    .expect("one frame fits");

    // Ground under it, at the same tile: the mobile has to win, and the ground
    // is what makes that a claim rather than a drawing on an empty frame.
    let side = usize::from(LAND_TILE_SIZE);
    let blue = Color16(0b0_00000_00000_11111);
    let land = LandAtlas::pack([(
        Graphic(1),
        Image::new(LAND_TILE_SIZE, LAND_TILE_SIZE, vec![blue; side * side]),
    )])
    .expect("one sprite fits");
    let texmaps = TexmapAtlas::pack([]).expect("nothing always fits");
    let statics = StaticAtlas::pack([]).expect("nothing always fits");
    let centre = Point::new(100, 100, 0);
    let camera = Camera::new(centre, 256, 256);

    // The ground quad is built here rather than collected: `Map` cannot be
    // constructed in memory — see the backlog in docs/client.md — and what this
    // test needs is one tile under the mobile's feet at the depth `depth` would
    // have given it.
    let at = camera.to_screen(centre);
    let ground = [GroundQuad {
        x: at.x as f32,
        y: at.y as f32,
        corners: [0.0; 4],
        region: land.region(Graphic(1)).expect("packed"),
        texmap: None,
        depth: openshard_client_render::depth::Order {
            tile: 200,
            priority_z: openshard_client_render::depth::land_priority_z([0; 4]),
        }
        .to_depth(openshard_client_render::depth::base_for(100, 100)),
        place: Place::land(100, 100),
    }];

    let colours = |facing| {
        let quads = mobiles::collect(
            &[Mobile {
                at: centre,
                body: BODY,
                group: 4,
                facing,
                frame: 0,
                from: None,
                hue: openshard_protocol::wire::Hue::NONE,
                drawn: openshard_client_render::follow::Gaze::on(centre),
                equipment: Vec::new(),
            }],
            &camera,
            &atlas,
            &Cutaway::OPEN,
            &EquipConv::default(),
        );
        assert_eq!(quads.len(), 1, "the frame is packed, so it draws");
        let frame = render_both(
            &device,
            &queue,
            &land,
            &texmaps,
            &ground,
            &statics,
            &[],
            (atlas.pixels(), &quads),
            256,
            256,
            Projection::one_to_one(256, 256),
        );
        // The two pixels the sprite covers, left and right.
        let x = quads[0].rect.x as u32;
        let y = quads[0].rect.y as u32;
        (frame.pixel(x, y), frame.pixel(x + 1, y))
    };

    let (red_r, red_g, red_b) = red.rgb8();
    let (green_r, green_g, green_b) = green.rgb8();
    // South is stored direction 1 unflipped, East is the same picture mirrored.
    assert_eq!(
        colours(Direction::South),
        (
            [red_r, red_g, red_b, u8::MAX],
            [green_r, green_g, green_b, u8::MAX]
        ),
        "the mobile is not drawn over the ground, or not from its own atlas",
    );
    assert_eq!(
        colours(Direction::East),
        (
            [green_r, green_g, green_b, u8::MAX],
            [red_r, red_g, red_b, u8::MAX]
        ),
        "a mirrored facing drew the picture the same way round",
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
        let wanted = ground::visible_graphics(&map, &camera);
        let atlas = LandAtlas::build(&art, wanted.iter().copied()).expect("fits");
        let texmaps = texmap_atlas(&dir, wanted);
        let quads = ground::collect(&map, &camera, &atlas, &texmaps, &Cutaway::OPEN);
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
    let quads = ground::collect(&map, &moved, &atlas, &texmaps, &Cutaway::OPEN);
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

/// The gate `docs/camera.md` D11 asks for, on the GPU: magnified, moving the
/// eye by `1/zoom` of a virtual pixel moves the picture by exactly one real one.
///
/// Everything else about D11 is arithmetic that can be asserted without a
/// device. This is the claim that cannot: that the shader's last two lines,
/// the rasteriser and `nearest` sampling together produce a frame that is the
/// other frame *translated*, rather than one resampled by a fraction of a texel.
/// The second is what a magnification usually costs, it looks like a slight
/// change in the art, and no arithmetic in `camera.rs` would notice it.
///
/// Two cameras a third of a virtual pixel apart at `3x`, which is one real pixel
/// and is the finest step the display has. The quads are built once and shared
/// deliberately: `to_view` measures from the eye *rounded*, and both eyes round
/// to the same virtual pixel, so the only difference between the two frames is
/// `Projection::origin` — which is exactly the claim.
///
/// A third and not a half, because a half is the one fraction that could be
/// right for the wrong reason: it is on the lattice of `2x` as well, so a
/// rounding that quietly went to the nearest *even* real pixel would pass it.
#[test]
fn a_third_of_a_virtual_pixel_moves_a_magnified_frame_one_real_pixel() {
    let (Some(dir), Some((device, queue))) = (client_dir(), gpu()) else {
        return;
    };
    let map = Map::load_facet(&dir, 0).expect("Felucca");
    let art = Art::open(&dir).expect("artLegacyMUL.uop");

    let mut camera = Camera::new(Point::new(1495, 1629, 0), 512, 256);
    let mut zoom = Zoom::ONE;
    for _ in 0..2 {
        zoom = zoom.scale_up();
    }
    camera.zoom_about(256, 128, zoom);
    assert_eq!(camera.zoom().to_string(), "3x", "the rung this test is about");
    assert!(
        !camera.minifies(),
        "and the world is drawn at the display's own size"
    );

    let wanted = ground::visible_graphics(&map, &camera);
    let land = LandAtlas::build(&art, wanted.iter().copied()).expect("fits");
    let texmaps = texmap_atlas(&dir, wanted);
    let quads = ground::collect(&map, &camera, &land, &texmaps, &Cutaway::OPEN);

    // Along `x` only: a diagonal move would pass on a frame shifted the right
    // distance the wrong way round, and the two axes are separate lines of
    // shader.
    let mut shifted = camera;
    let at = camera.eye_at();
    shifted.look_at(WorldPoint {
        x: at.x + camera.quantum(),
        y: at.y,
    });
    assert_eq!(shifted.eye(), camera.eye(), "the same whole virtual pixel");
    assert_ne!(shifted.projection(), camera.projection(), "and a different frame");

    let (width, height) = (camera.width, camera.height);
    let before = render_projected(&device, &queue, &land, &texmaps, &quads, width, height, camera);
    let after = render_projected(&device, &queue, &land, &texmaps, &quads, width, height, shifted);
    assert_ne!(before.pixels, after.pixels, "a real pixel moved nothing at all");

    // The eye moved right, so the world moved left: what is at `x` in the second
    // frame was at `x + 1` in the first. Compared over the interior, because the
    // column the shift walks in from has no counterpart to be compared with.
    let mut checked = 0usize;
    let mut moved = 0usize;
    let mut resampled = 0usize;
    for y in 0..height {
        for x in 0..width - 1 {
            checked += 1;
            if after.pixel(x, y) == before.pixel(x + 1, y) {
                moved += 1;
            } else {
                resampled += 1;
            }
        }
    }
    // Counted and asserted, because "every pixel matched" and "no pixel was
    // looked at" are the same green — this repository has produced the second
    // one before.
    assert_eq!(checked, (width as usize - 1) * height as usize);

    // What is *not* an exact translation, and why it cannot be. A sloped tile is
    // textured by stretching a square texmap over a diamond, so its `uv` is
    // interpolated across a quad that is not axis-aligned and a fragment centre
    // a third of a texel along lands on the other side of a texel boundary here
    // and there. There is no placement of the quantiser that fixes that: it is
    // what stretching a texture means. Everything drawn from the *art* — flat
    // ground, statics, sprites — is texel-aligned and translates exactly, which
    // is what the sprite half of this gate below asserts with no allowance at
    // all.
    //
    // One in a thousand is a ceiling and not a measurement (it is one in seven
    // thousand over Britain), and the mutation is what says a ceiling is enough:
    // an origin that rounds its fraction away draws the *same* frame twice, so
    // the number this is separating a correct camera from is not 1 in 7,000 but
    // 130,815 in 130,816.
    assert!(
        resampled * 1000 < checked,
        "{resampled} of {checked} pixels are not the frame before it, translated",
    );
    assert!(
        moved > checked / 2,
        "a frame that agreed nowhere is not a translation"
    );

    // And the guard against all of that holding vacuously. Comparing the two
    // frames *without* the translation is not a good enough test on its own —
    // ground is large flat regions of colour, so more than half the pixels have
    // the same value one pixel over regardless — so what is asserted is that the
    // translation explains strictly more of the frame than standing still does.
    // Under the mutation the two are equal, which is what makes this the
    // discriminating comparison rather than a restatement of the one above.
    let still = (0..height)
        .flat_map(|y| (0..width - 1).map(move |x| (x, y)))
        .filter(|&(x, y)| after.pixel(x, y) == before.pixel(x, y))
        .count();
    assert!(
        moved > still,
        "translating explains {moved} pixels and standing still explains {still}",
    );
}

/// And the half of that gate with no allowance in it: a *sprite* at `3x`,
/// shifted a third of a virtual pixel, is the same picture one real pixel over.
///
/// Everything drawn from the art rather than from a texmap is texel-aligned —
/// the quad is the sprite's own rectangle, so a fragment centre lands on a
/// texel centre at every magnification — and a translation of the quad by a
/// whole real pixel is therefore a translation of the picture, exactly, with no
/// resampling anywhere. This is the claim the character's own smoothness rests
/// on, so it is asserted without a tolerance.
#[test]
fn a_magnified_sprite_translates_texel_for_texel() {
    let (Some(dir), Some((device, queue))) = (client_dir(), gpu()) else {
        return;
    };
    let art = Art::open(&dir).expect("artLegacyMUL.uop");

    let mut camera = Camera::new(Point::new(200, 200, 0), 512, 256);
    let mut zoom = Zoom::ONE;
    for _ in 0..2 {
        zoom = zoom.scale_up();
    }
    camera.zoom_about(256, 128, zoom);
    assert_eq!(camera.zoom().to_string(), "3x");

    // A static's sprite, drawn through the same pass a mobile uses: what is
    // being asserted is the pass and the transform, and a static is the one this
    // suite can build without an animation file.
    let graphic = Graphic(0x0CE3);
    let atlas = StaticAtlas::build(&art, [graphic]).expect("one sprite fits");
    let sprite = atlas.sprite(graphic).expect("just packed");
    let quads = vec![SpriteQuad {
        rect: Rect {
            x: (camera.render_width() as i32 / 2) as f32,
            y: (camera.render_height() as i32 / 2) as f32,
            width: f32::from(sprite.width),
            height: f32::from(sprite.height),
        },
        region: sprite.region,
        depth: 0.5,
        hue: 0,
        place: Place::NOWHERE,
    }];

    let mut shifted = camera;
    let at = camera.eye_at();
    shifted.look_at(WorldPoint {
        x: at.x + camera.quantum(),
        y: at.y,
    });

    let land = LandAtlas::build(&art, []).expect("nothing always fits");
    let texmaps = TexmapAtlas::pack([]).expect("nothing always fits");
    let none = AnimAtlas::pack([]).expect("nothing always fits");
    let (width, height) = (camera.width, camera.height);
    let frame = |camera: Camera| {
        render_both(
            &device,
            &queue,
            &land,
            &texmaps,
            &[],
            &atlas,
            &quads,
            (none.pixels(), &[]),
            width,
            height,
            camera.projection(),
        )
    };
    let before = frame(camera);
    let after = frame(shifted);

    let mut drawn = 0usize;
    for y in 0..height {
        for x in 0..width - 1 {
            assert_eq!(
                after.pixel(x, y),
                before.pixel(x + 1, y),
                "({x}, {y}) is not the frame before it, translated",
            );
            if after.pixel(x, y)[3] != 0 {
                drawn += 1;
            }
        }
    }
    // The sprite has to actually be on screen, or the assertion above compared
    // a cleared frame with a cleared frame and passed for it.
    assert!(
        drawn > 1_000,
        "only {drawn} pixels of sprite: a blank frame agrees"
    );
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
    let quads = ground::collect(&map, &camera, &land, &texmaps, &Cutaway::OPEN);

    let wanted_statics = statics::visible_graphics(&map, &camera, &StaticAnimations::default());
    let static_atlas = StaticAtlas::build(&art, wanted_statics).expect("a screen of statics fits");
    let static_quads = statics::collect(
        &map,
        &camera,
        &tiledata,
        &StaticAnimations::default(),
        &static_atlas,
        &Cutaway::OPEN,
    );
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
    let none = AnimAtlas::pack([]).expect("nothing always fits");
    let frame = render_both(
        &device,
        &queue,
        &land,
        &texmaps,
        &quads,
        &static_atlas,
        &static_quads,
        (none.pixels(), &[]),
        camera.width,
        camera.height,
        camera.projection(),
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
    let centre = Point::new(1495, 1629, 0);
    let camera = Camera::new(centre, 768, 512);

    let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");
    let wanted = ground::visible_graphics(&map, &camera);
    let atlas = LandAtlas::build(&art, wanted.iter().copied()).expect("fits");
    let texmaps = texmap_atlas(&dir, wanted);
    let quads = ground::collect(&map, &camera, &atlas, &texmaps, &Cutaway::OPEN);

    let static_atlas = StaticAtlas::build(
        &art,
        statics::visible_graphics(&map, &camera, &StaticAnimations::default()),
    )
    .expect("statics fit");
    let static_quads = statics::collect(
        &map,
        &camera,
        &tiledata,
        &StaticAnimations::default(),
        &static_atlas,
        &Cutaway::OPEN,
    );

    // A character standing where the camera looks, facing each way in turn, so
    // the picture shows both the placement and the mirrored facings.
    let mut anim = Anim::open(&dir).expect("anim.idx and anim.mul");
    let people: Vec<Mobile> = Direction::ALL
        .iter()
        .enumerate()
        .map(|(index, facing)| {
            let (x, y) = (centre.x - 3 + index as u16 % 4, centre.y - 3 + index as u16 / 4);
            // On the ground rather than at the camera's height: a mobile
            // standing below the terrain is *correctly* hidden by it, which is
            // what the first run of this dump showed.
            //
            // The tile's average and not its stored corner, which is where a
            // body actually stands (`Map::average_land_z`): the corner is the
            // diamond's northern vertex, and on a slope standing at it is
            // standing under the floor — the ground sorts at that same average,
            // less two, so it is drawn over the body rather than beside it.
            let ground = Point::new(x, y, map.average_land_z(x, y).expect("inside the facet"));
            Mobile {
                at: ground,
                body: 400,
                group: 4,
                facing: *facing,
                frame: 0,
                // Standing, so there is no second tile to sort between.
                from: None,
                hue: openshard_protocol::wire::Hue::NONE,
                // Standing where the server put them: nothing here is walking.
                drawn: openshard_client_render::follow::Gaze::on(ground),
                equipment: Vec::new(),
            }
        })
        .collect();
    let equip_conv = EquipConv::default();
    let mobile_atlas =
        AnimAtlas::build(&mut anim, mobiles::needed_animations(&people, &equip_conv)).expect("a body fits");
    let mobile_quads = mobiles::collect(&people, &camera, &mobile_atlas, &Cutaway::OPEN, &equip_conv);

    let frame = render_both(
        &device,
        &queue,
        &atlas,
        &texmaps,
        &quads,
        &static_atlas,
        &static_quads,
        (mobile_atlas.pixels(), &mobile_quads),
        camera.width,
        camera.height,
        camera.projection(),
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

/// A sprite packed into an atlas *after* its renderer was built is drawn, and
/// drawn from its own pixels.
///
/// The load-bearing test for growing an atlas instead of rebuilding it. The
/// whole saving is that a growth uploads a band of rows rather than a 16MB
/// texture, and the band is the one thing in that arrangement with arithmetic in
/// it: `write_rows` cuts a slice out of the atlas and names a `y` to start it at,
/// and the two have to agree. If they do not, the sprite is drawn from whatever
/// the texture held there — which on a fresh atlas is transparent, so the
/// failure is a graphic that silently does not appear rather than one that
/// appears wrong.
///
/// The first sprite is 2,040 wide on purpose: it fills the shelf, so the second
/// starts a new row and the band has a non-zero origin. A band starting at zero
/// passes with the offset arithmetic missing entirely.
///
/// No client files: the pictures are this test's own.
#[test]
fn a_sprite_added_after_the_pass_was_built_is_drawn_from_the_rows_uploaded() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    const SHELF_FILLER: Graphic = Graphic(1);
    const LATE: Graphic = Graphic(2);
    let (width, height) = (24u16, 18u16);
    let color = Color16(0b0_11111_00000_00000);

    let mut atlas = StaticAtlas::pack([(
        SHELF_FILLER,
        Image::new(2040, 40, vec![Color16(0b0_00000_11111_00000); 2040 * 40]),
    )])
    .expect("one wide sprite fits");

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let hue_ramp = HueRamp::build(&Hues::parse(&[0u8; 708]).expect("one empty group"));
    // Built from the atlas as it stands, which is the point: the pass below is
    // never rebuilt, exactly as `client/app` no longer rebuilds it.
    let mut statics = SpriteRenderer::new(&device, &queue, format, atlas.pixels(), &hue_ramp);

    atlas
        .pack_more([(
            LATE,
            Image::new(
                width,
                height,
                vec![color; usize::from(width) * usize::from(height)],
            ),
        )])
        .expect("a second sprite fits");
    let rows = atlas.take_dirty().expect("the growth wrote something");
    assert!(
        rows.start > 0,
        "the second sprite should have started a new shelf"
    );
    statics.upload_rows(&queue, atlas.pixels(), rows);

    let sprite = atlas.sprite(LATE).expect("packed");
    let quads = [SpriteQuad {
        rect: Rect {
            x: 10.0,
            y: 12.0,
            width: f32::from(sprite.width),
            height: f32::from(sprite.height),
        },
        region: sprite.region,
        depth: 0.5,
        hue: 0,
        place: Place::NOWHERE,
    }];

    let (frame_width, frame_height) = (128u32, 128u32);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("frame"),
        size: wgpu::Extent3d {
            width: frame_width,
            height: frame_height,
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
    let depth = renderer::depth_texture(&device, frame_width, frame_height);
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let place = openshard_client_render::place::texture(&device, frame_width, frame_height);
    let place_view = place.create_view(&wgpu::TextureViewDescriptor::default());
    // The ground pass clears; the sprite pass loads what it left. Given nothing
    // to draw, it is the clear on its own.
    let land = LandAtlas::pack([]).expect("nothing always fits");
    let texmaps = TexmapAtlas::pack([]).expect("nothing always fits");
    let mut ground = GroundRenderer::new(&device, &queue, format, &land, &texmaps);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let target_view = Target::whole(&view, &depth_view, &place_view, frame_width, frame_height);
    ground.render(&device, &queue, &mut encoder, target_view, &[]);
    statics.render(&device, &queue, &mut encoder, target_view, &quads);
    queue.submit([encoder.finish()]);
    let frame = read_back(&device, &queue, &target);

    let (r, g, b) = color.rgb8();
    let mut drawn = 0;
    for y in 0..frame_height {
        for x in 0..frame_width {
            let inside =
                (10..10 + u32::from(width)).contains(&x) && (12..12 + u32::from(height)).contains(&y);
            let got = frame.pixel(x, y);
            if !inside {
                assert_eq!(got[3], 0, "({x}, {y}) is outside the sprite and was drawn");
                continue;
            }
            assert_eq!(
                got,
                [r, g, b, u8::MAX],
                "({x}, {y}) is not the late sprite's pixel"
            );
            drawn += 1;
        }
    }
    assert_eq!(drawn, usize::from(width) * usize::from(height));
}

/// Draw `quads` into a world image, ring the ones in `outlined`, blit the lot
/// onto a surface and read the surface back.
///
/// The whole outline pipeline in one helper, in the order the client runs it:
/// the picture, then the silhouette mask against the picture's own depth, then
/// the blit, then the ring over it. A test that skipped a step would be
/// asserting about a pipeline nothing draws.
#[allow(clippy::too_many_arguments)]
fn render_outlined(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &StaticAtlas,
    quads: &[SpriteQuad],
    outlined: &[SpriteQuad],
    width: u32,
    height: u32,
    ring: Ring,
) -> Frame {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let world = openshard_client_render::blit::world_texture(device, width, height);
    let world_view = world.create_view(&wgpu::TextureViewDescriptor::default());
    let depth = renderer::depth_texture(device, width, height);
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let place = openshard_client_render::place::texture(device, width, height);
    let place_view = place.create_view(&wgpu::TextureViewDescriptor::default());
    let mask = outline::mask_texture(device, width, height);
    let mask_view = mask.create_view(&wgpu::TextureViewDescriptor::default());
    let hue_ramp = HueRamp::build(&Hues::parse(&[0u8; 708]).expect("one empty group"));

    let target = Target::whole(&world_view, &depth_view, &place_view, width, height);
    let empty_land = LandAtlas::pack([]).expect("nothing always fits");
    let empty_texmaps = TexmapAtlas::pack([]).expect("nothing always fits");
    // The ground pass with nothing in it, purely to clear the world image: it is
    // the pass that owns the clear, and a world texture nobody cleared holds
    // whatever the driver left there.
    let mut ground_pass = GroundRenderer::new(device, queue, format, &empty_land, &empty_texmaps);
    let mut sprites = SpriteRenderer::new(device, queue, format, atlas.pixels(), &hue_ramp);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    ground_pass.render(device, queue, &mut encoder, target, &[]);
    sprites.render(device, queue, &mut encoder, target, quads);
    sprites.render_mask(device, queue, &mut encoder, target, &mask_view, outlined);

    let surface = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("surface"),
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
    let surface_view = surface.create_view(&wgpu::TextureViewDescriptor::default());
    let rect = ViewportRect {
        x: 0,
        y: 0,
        width,
        height,
    };
    Blit::new(device, format).render(
        device,
        queue,
        &mut encoder,
        openshard_client_render::blit::Frame {
            target: &surface_view,
            world: &world_view,
            zoom: Zoom::ONE,
            rect,
        },
        &Lighting::NONE,
    );
    Outline::new(device, format).render(
        device,
        queue,
        &mut encoder,
        openshard_client_render::outline::Frame {
            target: &surface_view,
            mask: &mask_view,
            mask_size: (width, height),
            rect,
        },
        ring,
    );
    queue.submit([encoder.finish()]);

    read_back(device, queue, &surface)
}

/// A solid square of one colour, packed alone.
fn square(graphic: Graphic, side: u16, color: Color16) -> StaticAtlas {
    StaticAtlas::pack([(
        graphic,
        Image::new(side, side, vec![color; usize::from(side) * usize::from(side)]),
    )])
    .expect("one sprite fits")
}

/// The ring is exactly the pixels next to the silhouette and outside it — and
/// the sprite itself is left alone.
///
/// Both halves are the assertion. A dilation that drew the *whole* grown shape
/// instead of the grown-minus-original ring passes any test that only looks at
/// the border, and it covers the art it was supposed to be pointing at.
#[test]
fn a_ring_is_drawn_around_a_silhouette_and_not_over_it() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    const GRAPHIC: Graphic = Graphic(1);
    const SIDE: u16 = 16;
    let green = Color16(0b0_00000_11111_00000);
    let atlas = square(GRAPHIC, SIDE, green);
    let sprite = atlas.sprite(GRAPHIC).expect("packed");
    let (x, y) = (40.0, 50.0);
    let quads = [SpriteQuad {
        rect: Rect {
            x,
            y,
            width: f32::from(SIDE),
            height: f32::from(SIDE),
        },
        region: sprite.region,
        depth: 0.5,
        hue: 0,
        place: Place::NOWHERE,
    }];

    let (width, height) = (128, 128);
    let frame = render_outlined(
        &device,
        &queue,
        &atlas,
        &quads,
        &quads,
        width,
        height,
        Ring::DEFAULT,
    );

    let (green_r, green_g, green_b) = green.rgb8();
    let white = [u8::MAX; 4];
    let (left, top) = (x as u32, y as u32);
    let (right, bottom) = (left + u32::from(SIDE), top + u32::from(SIDE));
    let mut ringed = 0;
    for py in 0..height {
        for px in 0..width {
            let inside = (left..right).contains(&px) && (top..bottom).contains(&py);
            // One pixel out on every side, corners included: an eight-tap
            // neighbourhood rings the diagonal too, and a four-tap one does not
            // — which is the difference between a closed ring and one with four
            // holes in it.
            let bordering = (left - 1..right + 1).contains(&px) && (top - 1..bottom + 1).contains(&py);
            let got = frame.pixel(px, py);
            if inside {
                assert_eq!(
                    got,
                    [green_r, green_g, green_b, u8::MAX],
                    "({px}, {py}) is inside the sprite and the ring painted over it",
                );
            } else if bordering {
                assert_eq!(got, white, "({px}, {py}) borders the sprite and was not ringed");
                ringed += 1;
            } else {
                assert_eq!(got[3], 0, "({px}, {py}) is nowhere near the sprite");
            }
        }
    }
    // The frame of a 16x16 square grown by one: 18² - 16².
    assert_eq!(ringed, 18 * 18 - 16 * 16);
}

/// Two outlined sprites that touch keep one ring each.
///
/// This is the whole reason the mask holds an *id* rather than a coverage bit.
/// With coverage the shared edge is interior to the union — every neighbour of
/// it is "drawn" — so no ring is grown there and the pair comes out outlined as
/// a single blob. The seam below is the pixel column where that shows.
#[test]
fn two_touching_silhouettes_are_ringed_separately() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    const GRAPHIC: Graphic = Graphic(1);
    const SIDE: u16 = 16;
    let green = Color16(0b0_00000_11111_00000);
    let atlas = square(GRAPHIC, SIDE, green);
    let sprite = atlas.sprite(GRAPHIC).expect("packed");
    let (x, y) = (40.0f32, 50.0f32);
    // Edge to edge, sharing no pixel: the left one ends where the right begins.
    let quads: Vec<SpriteQuad> = [x, x + f32::from(SIDE)]
        .into_iter()
        .map(|at| SpriteQuad {
            rect: Rect {
                x: at,
                y,
                width: f32::from(SIDE),
                height: f32::from(SIDE),
            },
            region: sprite.region,
            depth: 0.5,
            hue: 0,
            place: Place::NOWHERE,
        })
        .collect();

    let (width, height) = (128, 128);
    let frame = render_outlined(
        &device,
        &queue,
        &atlas,
        &quads,
        &quads,
        width,
        height,
        Ring::DEFAULT,
    );

    let white = [u8::MAX; 4];
    let seam = x as u32 + u32::from(SIDE);
    let middle = y as u32 + u32::from(SIDE) / 2;
    assert_eq!(
        frame.pixel(seam - 1, middle),
        white,
        "the left sprite's own edge against the right one was not ringed — \
         the mask is behaving like coverage rather than an identity",
    );
    assert_eq!(
        frame.pixel(seam, middle),
        white,
        "and neither was the right sprite's edge against the left one",
    );
    // The outer edges are still there: a rule that only ever fired between two
    // ids would ring the seam and nothing else.
    assert_eq!(frame.pixel(x as u32 - 1, middle), white, "the pair's left edge");
    assert_eq!(
        frame.pixel(x as u32 + 2 * u32::from(SIDE), middle),
        white,
        "the pair's right edge",
    );
}
