//! **A real place, assembled and read back with no window in front of it.**
//!
//! `docs/parity.md`'s gate (P3) needs two frames of one place and had only one:
//! the tools could dump a picture and the client — the thing that is actually
//! broken — could not, so a defect visible in one and absent in the other said
//! nothing about either. The client's own half of that is F12
//! (`App::frame_dump`); this is the other shape the backlog named, a headless
//! run that assembles a frame the way the client assembles one and reads its
//! planes back.
//!
//! What it is *for* is the machinery underneath the gate, checked before the
//! gate is written on top of it:
//!
//! - one picture per [`View`], of the size that was asked for;
//! - the view actually reaching the shader, which is the positive control — a
//!   dump that returned thirteen copies of the lit frame would satisfy every
//!   count and answer no question;
//! - [`dump::read_rect`] surviving a width whose rows are not 256-byte aligned
//!   and an origin that is not the corner. Both are the client's ordinary case:
//!   a window is whatever size a person left it, and a docked panel moves the
//!   world's rect off the surface's corner.
//!
//! Gated on `OPENSHARD_CLIENT` like every test here that needs the client's own
//! files, and a no-op without it.

use std::path::PathBuf;

use openshard_client_render::animate::StaticAnimations;
use openshard_client_render::atlas::{LandAtlas, StaticAtlas, TexmapAtlas};
use openshard_client_render::blit::{self, Blit, ViewportRect};
use openshard_client_render::camera::Camera;
use openshard_client_render::cutaway::Cutaway;
use openshard_client_render::debug::View;
use openshard_client_render::frame::{self, Impostor};
use openshard_client_render::light::{self, Tuning};
use openshard_client_render::renderer::{self, GroundRenderer, MeshFaceRenderer, SpriteRenderer, Target};
use openshard_client_render::statics::StaticGeometry;
use openshard_client_render::{dump, ground, statics};
use openshard_protocol::direction::Direction;
use openshard_protocol::world::Point;
use openshard_uofiles::animdata::AnimData;
use openshard_uofiles::art::Art;
use openshard_uofiles::hues::Hues;
use openshard_uofiles::map::Map;
use openshard_uofiles::texmaps::TexMaps;
use openshard_uofiles::tiledata::TileData;

/// The house corner in Britain every lighting question in this repository has
/// been asked at — `docs/parity.md`'s own coordinate, so the frame this dumps is
/// the frame the plan talks about.
const AT: Point = Point::new(1501, 1659, 0);

/// Deliberately not 256-byte-row aligned (`900 * 4 = 3600`), and deliberately
/// not a round number of tiles: a readback that ignored the copy's padding would
/// return a sheared picture here, and one that panicked on the assertion the
/// tools used to carry would not get this far.
const VIEWPORT: (u32, u32) = (900, 700);

/// The client's files, or `None` when the environment does not point at any.
fn client_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("OPENSHARD_CLIENT")?))
}

/// A GPU to draw with, or `None` where there is none. The client's own limits —
/// see `tests/frame.rs`'s copy for why they are asked for rather than defaulted.
fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: openshard_client_render::gbuffer::required_limits(),
        ..Default::default()
    }))
    .ok()
}

/// Everything one drawn frame leaves behind: the world image, what the passes
/// said about each of its pixels, and the light that goes over it.
///
/// Held together because the blit needs all four and they are only valid
/// together — a G-buffer from one frame over another frame's world image is a
/// picture of nothing.
struct Drawn {
    world: wgpu::Texture,
    gbuffer: openshard_client_render::gbuffer::Gbuffer,
    lighting: light::Lighting,
    ground: GroundRenderer,
    statics: SpriteRenderer,
    mesh: MeshFaceRenderer,
}

/// Assemble the frame at [`AT`] the way `App::draw` assembles one, draw its
/// three world passes, and stop before the blit.
///
/// The map's own statics and no server items, the player's own cutaway, night
/// with a flame in hand: the client's values, because a fixture that quietly
/// chose easier ones is the coincidence `docs/parity.md` is about.
fn draw_britain(device: &wgpu::Device, queue: &wgpu::Queue, dir: &std::path::Path) -> Drawn {
    let map = Map::load_facet(dir, 0).expect("Felucca");
    let art = Art::open(dir).expect("artLegacyMUL.uop");
    let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");
    let animdata = AnimData::load(dir).expect("animdata.mul");
    let animations = StaticAnimations::build(&animdata, &tiledata);

    let camera = Camera::new(AT, VIEWPORT.0, VIEWPORT.1);
    let cutaway = Cutaway::at(&map, &tiledata, AT, true);

    let land_wanted = ground::visible_graphics(&map, &camera);
    let land = LandAtlas::build(&art, land_wanted.iter().copied()).expect("a screen of land fits");
    let texmaps = TexmapAtlas::build(
        &TexMaps::open(dir).expect("texidx.mul and texmaps.mul"),
        &tiledata,
        land_wanted,
    )
    .expect("a screen of textures fits");
    let static_atlas = StaticAtlas::build(&art, statics::visible_graphics(&map, &camera, &animations))
        .expect("a screen of statics fits");

    let tuning = Tuning::DEFAULT;
    let inputs = frame::Inputs {
        map: &map,
        items: &[],
        camera: &camera,
        tiledata: &tiledata,
        animations: &animations,
        cutaway: &cutaway,
        land: &land,
        texmaps: &texmaps,
        statics: &static_atlas,
        // Night and flat, which is what the client draws with F10 on: a lit
        // frame is the one whose planes disagree with each other, and a daylight
        // frame's blit is a copy.
        sky: Some(light::NIGHT.flattened()),
        sun: None,
        carried: Some((AT, Direction::South)),
        tuning: &tuning,
        flame_time: 0.0,
        bake: None,
        highlight: None,
        impostor: Impostor::Met,
        // Set per plane by `dump::planes`; what it is here is what a caller that
        // never dumped would draw.
        view: View::Lit,
    };
    // The summary is the other half of a dump — see `Inputs::summary`. Asked for
    // here so that a change that breaks it breaks a test rather than a person's
    // afternoon, and printed so a failing run says which place it was at.
    let asked_for = inputs.summary();
    assert!(
        asked_for.lines().count() >= 18,
        "a summary shorter than `Inputs` has fields is a summary that has stopped naming all of them:\n{asked_for}",
    );
    println!("{asked_for}");

    let frame::Frame {
        lighting,
        ground: ground_quads,
        statics:
            StaticGeometry {
                quads: static_quads,
                mesh_vertices,
                mesh_rows,
                boxes,
            },
    } = frame::assemble(inputs);

    let (width, height) = camera.image_size();
    let format = blit::WORLD_FORMAT;
    let world = blit::world_texture(device, width, height);
    let world_view = world.create_view(&wgpu::TextureViewDescriptor::default());
    let gbuffer = openshard_client_render::gbuffer::Gbuffer::new(device, width, height);
    let gbuffer_views = gbuffer.views();
    let depth = renderer::depth_texture(device, width, height);
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());

    let mut ground_pass = GroundRenderer::new(device, queue, format, &land, &texmaps);
    let mut statics_pass = SpriteRenderer::new(
        device,
        queue,
        format,
        static_atlas.pixels(),
        &openshard_client_render::hue::HueRamp::build(&Hues::load(dir.join("hues.mul")).expect("hues.mul")),
    );
    let mut mesh_pass = MeshFaceRenderer::new(device);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let target = Target {
        gbuffer: &gbuffer_views,
        view: &world_view,
        depth: &depth_view,
        width,
        height,
        projection: camera.projection(),
    };
    ground_pass.render(device, queue, &mut encoder, target, &ground_quads);
    statics_pass.render(device, queue, &mut encoder, target, &static_quads, &boxes, None);
    mesh_pass.render(device, queue, &mut encoder, target, &mesh_vertices, &mesh_rows);
    queue.submit([encoder.finish()]);

    Drawn {
        world,
        gbuffer,
        lighting,
        ground: ground_pass,
        statics: statics_pass,
        mesh: mesh_pass,
    }
}

/// A texture the blit can draw into and a copy can read out of, at the size the
/// surface would be.
fn dump_target(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dump target"),
        size: wgpu::Extent3d {
            width: VIEWPORT.0,
            height: VIEWPORT.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// The width and height a PNG declares, off its own `IHDR` — read rather than
/// trusted, because a picture of the wrong size that opens is exactly what a
/// readback with the padding left in produces.
fn png_size(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[..8], &[137, 80, 78, 71, 13, 10, 26, 10], "not a PNG");
    assert_eq!(&png[12..16], b"IHDR", "the first chunk of a PNG is its header");
    let width = u32::from_be_bytes(png[16..20].try_into().expect("four bytes"));
    let height = u32::from_be_bytes(png[20..24].try_into().expect("four bytes"));
    (width, height)
}

/// **A row is measured in the texture's own texels, not in four bytes.**
///
/// The defect the first press of F12 found, and the reason this test needs
/// neither client files nor a drawn frame: the client dumped into a texture of
/// the *surface's* format, and this machine's compositor offers `Rgba16Float` —
/// eight bytes a texel. A row measured as `width * 4` against that is not a
/// shorter row, it is a copy `wgpu` refuses outright, and the client died on the
/// keypress.
///
/// Both halves matter and both are here: the format's own texel size, and the
/// alignment padding on top of it. `301 * 8 = 2408` is aligned to neither.
#[test]
fn a_readback_measures_a_row_in_the_textures_own_texels() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let rect = ViewportRect {
        x: 0,
        y: 0,
        width: 301,
        height: 97,
    };
    for (format, texel) in [
        (blit::WORLD_FORMAT, 4),
        (wgpu::TextureFormat::Bgra8Unorm, 4),
        (wgpu::TextureFormat::Rgba16Float, 8),
    ] {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("a texture of some format"),
            size: wgpu::Extent3d {
                width: rect.width,
                height: rect.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        assert_eq!(
            dump::read_rect(&device, &queue, &texture, rect).len(),
            (rect.width * rect.height * texel) as usize,
            "{format:?} is {texel} bytes a texel and the readback came back a different length",
        );
    }
}

#[test]
fn a_frame_dumps_one_picture_per_view_at_the_size_asked_for() {
    let (Some(dir), Some((device, queue))) = (client_dir(), gpu()) else {
        return;
    };
    let drawn = draw_britain(&device, &queue, &dir);
    let format = blit::WORLD_FORMAT;
    let into = dump_target(&device, format);
    let into_view = into.create_view(&wgpu::TextureViewDescriptor::default());
    let world_view = drawn.world.create_view(&wgpu::TextureViewDescriptor::default());
    let gbuffer_views = drawn.gbuffer.views();
    let mut blit = Blit::new(&device, format);
    let dummy_mobiles = blit::dummy_instances(&device);

    let rect = ViewportRect {
        x: 0,
        y: 0,
        width: VIEWPORT.0,
        height: VIEWPORT.1,
    };
    let planes = dump::planes(
        &device,
        &queue,
        &mut blit,
        &into,
        blit::Frame {
            target: &into_view,
            world: &world_view,
            gbuffer: &gbuffer_views,
            face_instances: drawn.statics.instances_buffer(),
            mobile_instances: &dummy_mobiles,
            mesh_instances: drawn.mesh.rows_buffer(),
            ground_instances: drawn.ground.instances_buffer(),
            zoom: openshard_client_render::camera::Zoom::ONE,
            rect,
        },
        &drawn.lighting,
        &View::ALL,
    );

    assert_eq!(
        planes.iter().map(|(view, _)| *view).collect::<Vec<_>>(),
        View::ALL.to_vec(),
        "a dump is every plane, in the order it was asked for",
    );
    for (view, png) in &planes {
        assert_eq!(
            png_size(png),
            VIEWPORT,
            "the {} plane came back a different size than the rect it was read from",
            view.name(),
        );
    }

    // **The positive control.** Thirteen pictures of the right size are what a
    // dump that ignored the view would also produce, and it would answer
    // nothing: these three planes are three different questions about one frame
    // — what it looks like, which place each pixel belongs to, and which way
    // each pixel faces — and on a real street they cannot agree.
    let plane = |want: View| {
        &planes
            .iter()
            .find(|(view, _)| *view == want)
            .expect("every view was asked for")
            .1
    };
    for (left, right) in [
        (View::Lit, View::Place),
        (View::Lit, View::Normal),
        (View::Place, View::Normal),
    ] {
        // `assert!` and not `assert_ne!`: the operands are megabytes of PNG, and
        // a failure that prints them is a failure nobody can read.
        assert!(
            plane(left) != plane(right),
            "the {} and {} planes came back identical: the view is not reaching the shader",
            left.name(),
            right.name(),
        );
    }
}

#[test]
fn a_readback_off_the_corner_is_the_same_pixels_shifted() {
    let (Some(dir), Some((device, queue))) = (client_dir(), gpu()) else {
        return;
    };
    let drawn = draw_britain(&device, &queue, &dir);
    let format = blit::WORLD_FORMAT;
    let into = dump_target(&device, format);
    let into_view = into.create_view(&wgpu::TextureViewDescriptor::default());
    let world_view = drawn.world.create_view(&wgpu::TextureViewDescriptor::default());
    let gbuffer_views = drawn.gbuffer.views();
    let mut blit = Blit::new(&device, format);
    let dummy_mobiles = blit::dummy_instances(&device);

    let whole = ViewportRect {
        x: 0,
        y: 0,
        width: VIEWPORT.0,
        height: VIEWPORT.1,
    };
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    blit.render(
        &device,
        &queue,
        &mut encoder,
        blit::Frame {
            target: &into_view,
            world: &world_view,
            gbuffer: &gbuffer_views,
            face_instances: drawn.statics.instances_buffer(),
            mobile_instances: &dummy_mobiles,
            mesh_instances: drawn.mesh.rows_buffer(),
            ground_instances: drawn.ground.instances_buffer(),
            zoom: openshard_client_render::camera::Zoom::ONE,
            rect: whole,
        },
        &drawn.lighting,
    );
    queue.submit([encoder.finish()]);

    // A rect off the corner, of a width that is not aligned either: what a
    // docked panel does to the client's own viewport, and the case the tools'
    // hand-rolled readbacks never had to handle because they always read from
    // `(0, 0)`.
    let corner = ViewportRect {
        x: 37,
        y: 11,
        width: 301,
        height: 97,
    };
    let all = dump::read_rect(&device, &queue, &into, whole);
    let part = dump::read_rect(&device, &queue, &into, corner);
    assert_eq!(
        part.len(),
        (corner.width * corner.height * 4) as usize,
        "a readback is tight rows of the rect asked for, padding stripped",
    );
    for row in 0..corner.height {
        let from = (((row + corner.y) * whole.width + corner.x) * 4) as usize;
        let took = (row * corner.width * 4) as usize;
        assert!(
            part[took..took + (corner.width * 4) as usize] == all[from..from + (corner.width * 4) as usize],
            "row {row} of the offset readback is not row {} of the whole picture: the copy's \
             origin is not being honoured",
            row + corner.y,
        );
    }
}
