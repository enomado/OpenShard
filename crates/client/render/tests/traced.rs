//! The reference path tracer, as a gate rather than as a tool.
//!
//! `examples/boxes.rs` has run this comparison since the tracer existed, and
//! `cargo test --workspace` has never reached it: an example is a thing a person
//! runs. So the strongest statement the lighting has — *a renderer sharing no
//! arithmetic with ours, and with no notion of a tile anywhere in it, agrees
//! about every interior pixel* — was one nobody would notice going false. This
//! file is that statement under `cargo test`.
//!
//! ```sh
//! cargo test -p openshard-client-render --test traced -- --nocapture
//! ```
//!
//! A machine with no GPU adapter skips it, which is the same bargain the rest of
//! the GPU suite makes. The tracer itself needs no GPU; what needs one is the
//! frame it is compared against.
//!
//! # Why it reaches into `examples/`
//!
//! The judging — what counts as a disagreement, and which of the four kinds it
//! is — is `examples/oracle/pathtrace.rs`, shared with the tool by `#[path]`
//! rather than copied. That module cannot be a library: it names
//! `openshard-client-pathtrace`, which is a **dev-dependency** of this crate
//! precisely so the shipped renderer cannot reach the thing that checks it, and
//! code naming a dev-dependency can live in `examples/` or `tests/` and nowhere
//! else.
//!
//! Given that, the choice was one copy reached by an unusual path or two copies
//! of the rule. Two copies is how a gate ends up green about a rule the tool no
//! longer applies — and the rule is exactly where a defect would hide, because
//! every one of its four splits is a decision about what *not* to report.
//!
//! What is **not** shared is the pipeline boilerplate below: building a scene,
//! rendering it, reading it back. Every GPU fixture in this crate has its own,
//! and a second one is a nuisance rather than a hazard — it cannot make a
//! disagreement disappear, only fail to produce one, which the non-triviality
//! assertions at the end are there to catch.

// Reached from `tests/`, so most of it is unused here — the slab oracle and the
// crosshair belong to the tool.
#[allow(dead_code)]
#[path = "../examples/oracle/mod.rs"]
mod oracle;

use openshard_client_pathtrace::trace as pt_trace;
use openshard_client_render::camera::{Camera, TileBounds, WorldSpot, Zoom, project_exact};
use openshard_client_render::cutaway::Cutaway;
use openshard_client_render::debug::View;
use openshard_client_render::depth;
use openshard_client_render::geometry::Vec2;
use openshard_client_render::light::{Light, Lighting, NIGHT};
use openshard_client_render::mesh_face::{MeshFaceRow, MeshFaceVertex};
use openshard_client_render::occlusion::{Builder, OwnerId};
use openshard_client_render::place::Stance;
use openshard_client_render::renderer::{self, GroundRenderer, MeshFaceRenderer, Target};
use openshard_protocol::wire::Graphic;
use openshard_uofiles::tiledata::{StaticTile, TileFlags};

use oracle::boxes::{BoxSpec, box_mesh, box_owner};

/// The frame the gate is measured over. Square, and large enough that a box's
/// own face is thousands of pixels rather than dozens: the comparison's whole
/// value is that it is a *picture* and not a point query.
const SIDE: u32 = 512;

/// A GPU to draw with, or `None` where there is none.
fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: openshard_client_render::gbuffer::required_limits(),
        ..Default::default()
    }))
    .ok()
}

/// `examples/boxes.rs`'s own `line` scene: two whole-tile boxes side by side due
/// east, four `z` units tall.
///
/// The same scene and the same flame the tool's own recorded numbers are from
/// (`docs/lighting_reference.md`), so a person reading a failure here can run
/// the tool on the same thing and get the picture. Two boxes rather than one
/// because a single box cannot produce the case that matters most: an occluder
/// that is not the one the fragment is standing on.
fn line_scene() -> Vec<BoxSpec> {
    let h = 4.0;
    vec![
        BoxSpec {
            tile: (100, 100),
            min: (100.0, 100.0, 0.0),
            max: (101.0, 101.0, h),
        },
        BoxSpec {
            tile: (101, 100),
            min: (101.0, 100.0, 0.0),
            max: (102.0, 101.0, h),
        },
    ]
}

/// Up and to the boxes' `+x`, `-y` side, above them — the tool's own default for
/// this scene, picked there by looking at a rendered frame.
fn flame() -> WorldSpot {
    WorldSpot {
        x: 102.5,
        y: 98.5,
        z: 6.0,
    }
}

const FLAME_RADIUS: f32 = 8.0;

/// One frame of a scene of boxes, and everything a comparison needs to read it.
///
/// Two tests draw a frame here — the shadow gate and the brightness gate — and
/// they differ in the scene, the ambient and which view they read back. Building
/// the pipeline twice would be two fixtures that can drift apart in every way
/// that is not under test; the camera in particular, which the reference tracer
/// *measures* through this fixture's own projection.
struct Rendered {
    /// What the world passes left on each pixel: the surface, and where its
    /// fragment is.
    drawn: Vec<oracle::Drawn>,
    /// The requested [`View`], read back, `RGBA8`.
    surface: Vec<u8>,
    /// The art itself, before any light — what the albedo of a comparison has to
    /// be taken from.
    world: Vec<u8>,
    /// Which box and stance each mesh-face row is, recorded as the rows were
    /// pushed rather than re-derived.
    face_rows: Vec<(usize, Stance, u32)>,
    /// The frame's own world-to-pixel map. Boxed because the tracer's camera is
    /// recovered from it as a black box and this fixture is what owns the
    /// camera it closes over.
    to_pixel: Box<dyn Fn(WorldSpot) -> (f64, f64)>,
}

/// A frame to draw: the scene, the light on it, and how close the camera is.
///
/// The zoom is a field and not a constant because it is **what decides how much
/// of the world a 512-pixel canvas holds**, and the two gates want opposite
/// things from that. The shadow gate wants a box's own face to be thousands of
/// pixels, which is 4:1. The brightness gate wants the flame's whole pool —
/// bright centre *and* dark rim — inside the frame, and at 4:1 a canvas is three
/// tiles across, so every pixel of it sits near the middle of an eight-tile pool
/// and the picture is one flat brightness. That version of the test agreed with
/// the tracer and would have agreed with almost any falloff curve; its own
/// non-triviality assertion is what caught it.
struct Shot<'a> {
    boxes: &'a [BoxSpec],
    flame: WorldSpot,
    /// The flame's reach, in tiles — `light::Light::radius`.
    radius: f32,
    /// How brightly it burns — `light::Light::intensity`.
    ///
    /// A field since phase 3, and the reason is the brightness gate: with a
    /// cosine in the term a pool's *bright* half is a disc under the flame rather
    /// than most of the canvas, and how wide that disc is is what decides whether
    /// a comparison is asking the falloff curve about its whole range or about
    /// its tail. A shadow comparison does not care and passes `1.0`.
    intensity: f32,
    ambient: openshard_client_render::light::Ambient,
    view: View,
    /// Notches up `camera::LADDER` from 1:1.
    zoom: u32,
}

/// Draw a [`Shot`] and read the frame back.
fn render(device: &wgpu::Device, queue: &wgpu::Queue, shot: Shot<'_>) -> Rendered {
    let Shot {
        boxes,
        flame,
        radius,
        intensity,
        ambient,
        view,
        zoom: zoom_notches,
    } = shot;
    let bounds = TileBounds {
        min_x: 95,
        max_x: 107,
        min_y: 95,
        max_y: 106,
    };
    // NO_SHOOT so a box occludes light at all (`occlusion::opacity`'s own doc: a
    // graphic's own flags decide it, not the shape). `height` here is only what
    // `depth::static_priority_z` reads off it; the occluder's real span comes
    // from `add_raw`'s own `space`.
    let cube_tile = StaticTile {
        flags: TileFlags::new(TileFlags::NO_SHOOT),
        height: 1,
        ..StaticTile::default()
    };
    let mut builder = Builder::new(bounds);
    for (index, b) in boxes.iter().enumerate() {
        builder.add_raw(b.tile.0, b.tile.1, b.solid(), box_owner(index, b));
    }
    let occlusion = builder.finish(&Cutaway::OPEN);
    let owners: Vec<OwnerId> = boxes
        .iter()
        .enumerate()
        .map(|(index, b)| {
            let owner = box_owner(index, b);
            let id = occlusion.owner_at(i32::from(b.tile.0), i32::from(b.tile.1), owner.z, owner.graphic);
            assert_ne!(
                id,
                OwnerId::NONE,
                "box {index} is not in the grid this test just built — the comparison would then be \
                 measuring a scene with one box missing, and would pass for it"
            );
            id
        })
        .collect();

    let (centre_x, centre_y) = (100, 100);
    let mut camera = Camera::new(
        openshard_protocol::world::Point::new(centre_x as u16, centre_y as u16, 0),
        SIDE,
        SIDE,
    );
    let mut zoom = Zoom::ONE;
    for _ in 0..zoom_notches {
        zoom = zoom.scale_up();
    }
    camera.zoom_about((SIDE / 2) as i32, (SIDE / 2) as i32, zoom);
    let projection = camera.projection();
    // Where a world position lands in this frame, in real pixels. The tracer's
    // camera is *measured* through this closure and never restates it — see
    // `oracle::pathtrace::Mirror::of`. Its own copy of the camera, so the
    // fixture can hand it back to a caller that outlives this function.
    let mapped = camera;
    let to_pixel = move |at: WorldSpot| -> (f64, f64) {
        let screen = mapped.to_view_exact(project_exact(at));
        (
            f64::from((screen.x - projection.origin.x) * projection.scale + SIDE as f32 * 0.5),
            f64::from((screen.y - projection.origin.y) * projection.scale + SIDE as f32 * 0.5),
        )
    };

    let base_tile = depth::base_for(centre_x, centre_y);
    let mut rows: Vec<MeshFaceRow> = Vec::new();
    let mut vertices: Vec<MeshFaceVertex> = Vec::new();
    // Which row each box's each face was pushed as, kept while it is pushed
    // rather than re-derived from `rows.len()` arithmetic later: it is what the
    // comparison matches the rendered `place` attachment's own id against, so
    // "this pixel is box 1's south face" is the renderer's answer and not this
    // test's guess about the order it built its own list in.
    let mut face_rows: Vec<(usize, Stance, u32)> = Vec::new();
    for (box_index, b) in boxes.iter().enumerate() {
        let solid = b.solid();
        let d = depth::Order {
            tile: i32::from(b.tile.0) + i32::from(b.tile.1),
            priority_z: depth::static_priority_z(solid.min.z.round() as i8, &cube_tile),
        }
        .to_depth(base_tile);
        for face in box_mesh(solid).faces() {
            let id = rows.len() as u32;
            let stance = Stance::of_normal(face.normal).expect("a box face's own axis-aligned normal");
            face_rows.push((box_index, stance, id));
            rows.push(MeshFaceRow {
                tile: (b.tile.0, b.tile.1),
                stance,
                owner: u32::from(owners[box_index].raw()),
            });
            for corner in face.fan() {
                vertices.push(MeshFaceVertex {
                    screen: camera.to_view_exact(project_exact(corner)),
                    world: [corner.x as f32, corner.y as f32, corner.z as f32],
                    depth: d,
                    id,
                    tile: [f32::from(b.tile.0), f32::from(b.tile.1)],
                    normal: face.normal,
                });
            }
        }
    }

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let world = openshard_client_render::blit::world_texture(device, SIDE, SIDE);
    let world_view = world.create_view(&wgpu::TextureViewDescriptor::default());
    let gbuffer = openshard_client_render::gbuffer::Gbuffer::new(device, SIDE, SIDE);
    let gbuffer_views = gbuffer.views();
    let depth_tex = renderer::depth_texture(device, SIDE, SIDE);
    let depth_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());

    // A floor for a shadow to fall on: one flat synthetic land tile, repeated
    // over the same bounds the occlusion grid covers.
    const FLOOR: Graphic = Graphic(3);
    let floor_pixel = openshard_uofiles::color::Color16((20 << 10) | (20 << 5) | 20);
    let floor_image = openshard_uofiles::image::Image::new(
        openshard_uofiles::art::LAND_TILE_SIZE,
        openshard_uofiles::art::LAND_TILE_SIZE,
        vec![floor_pixel; usize::from(openshard_uofiles::art::LAND_TILE_SIZE).pow(2)],
    );
    let blocks = (bounds.max_x as u32).div_ceil(openshard_uofiles::map::BLOCK_SIZE) + 1;
    let synthetic_map = openshard_uofiles::map::Map::from_blocks(blocks, blocks, |_x, _y| {
        openshard_uofiles::map::LandCell { tile: FLOOR.0, z: 0 }
    });
    let land = openshard_client_render::atlas::LandAtlas::pack([(FLOOR, floor_image)])
        .expect("one flat tile always fits");
    let texmaps = openshard_client_render::atlas::TexmapAtlas::pack([]).expect("nothing always fits");
    let ground_quads =
        openshard_client_render::ground::collect(&synthetic_map, &camera, &land, &texmaps, &Cutaway::OPEN);

    let mut ground_pass = GroundRenderer::new(device, queue, format, &land, &texmaps);
    let mut mesh_pass = MeshFaceRenderer::new(device);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let target = Target {
        gbuffer: &gbuffer_views,
        view: &world_view,
        depth: &depth_view,
        width: SIDE,
        height: SIDE,
        projection,
    };
    ground_pass.render(device, queue, &mut encoder, target, &ground_quads);
    mesh_pass.render(device, queue, &mut encoder, target, &vertices, &rows);
    queue.submit([encoder.finish()]);

    // What the world passes left on each pixel: which surface owns it, and where
    // in the world that surface's own fragment is.
    let drawn = oracle::read_gbuffer(device, queue, &gbuffer, SIDE, SIDE);

    let lighting = Lighting {
        ambient,
        lights: vec![Light {
            at: Vec2::new(flame.x as f32, flame.y as f32),
            z: flame.z as f32,
            radius,
            color: [1.0, 1.0, 1.0],
            intensity,
            beam: None,
        }],
        occlusion,
        sun: None,
        view,
    };

    let surface = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("surface"),
        size: wgpu::Extent3d {
            width: SIDE,
            height: SIDE,
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
    let dummy_instances = openshard_client_render::blit::dummy_instances(device);
    let mut blit = openshard_client_render::blit::Blit::new(device, format);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    blit.render(
        device,
        queue,
        &mut encoder,
        openshard_client_render::blit::Frame {
            target: &surface_view,
            world: &world_view,
            gbuffer: &gbuffer_views,
            face_instances: &dummy_instances,
            mobile_instances: &dummy_instances,
            mesh_instances: mesh_pass.rows_buffer(),
            ground_instances: ground_pass.instances_buffer(),
            zoom: Zoom::ONE,
            rect: openshard_client_render::blit::ViewportRect {
                x: 0,
                y: 0,
                width: SIDE,
                height: SIDE,
            },
        },
        &lighting,
    );
    queue.submit([encoder.finish()]);

    Rendered {
        drawn,
        surface: oracle::read_surface(device, queue, &surface, SIDE, SIDE),
        world: oracle::read_surface(device, queue, &world, SIDE, SIDE),
        face_rows,
        to_pixel: Box::new(to_pixel),
    }
}

#[test]
fn the_frame_and_the_path_tracer_agree_about_every_interior_pixel() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let boxes = line_scene();
    let at = flame();
    let frame = render(
        &device,
        &queue,
        Shot {
            boxes: &boxes,
            flame: at,
            radius: FLAME_RADIUS,
            // Visibility, so brightness is not what is being read: any intensity
            // that lights the scene at all answers the same question.
            intensity: 1.0,
            ambient: NIGHT,
            view: View::Shadow,
            // Three notches is the top of `camera::LADDER` — 4:1 — where a
            // whole-tile box fills a 512-pixel canvas comfortably.
            zoom: 3,
        },
    );

    // `Albedos::INVENTED`, and it says so: this comparison is about *visibility*
    // — which pixels the flame reaches — and nothing in it reads a colour. The
    // brightness comparison that does is
    // `the_frame_and_the_path_tracer_agree_about_brightness_on_open_ground`,
    // which takes its ground albedo off the frame itself.
    let mirror = oracle::pathtrace::Mirror::of(oracle::pathtrace::Mirrored {
        boxes: &boxes,
        light_at: at,
        light_radius: f64::from(FLAME_RADIUS),
        colour: [1.0, 1.0, 1.0],
        intensity: 1.0,
        albedos: oracle::pathtrace::Albedos::INVENTED,
        to_pixel: frame.to_pixel.as_ref(),
    });
    let verdict = oracle::pathtrace::compare(
        &mirror.render(pt_trace::Brdf::Flat, SIDE, SIDE),
        &mirror.render(pt_trace::Brdf::Lambert, SIDE, SIDE),
        oracle::pathtrace::Frame {
            width: SIDE,
            height: SIDE,
            drawn: &frame.drawn,
            shadow: &frame.surface,
            face_rows: &frame.face_rows,
        },
    );
    eprint!("{}", verdict.report());

    // The scene has to be one where the answer could have been wrong. All three
    // of these are the same guard from different sides: a frame that drew
    // nothing, a torch that reached everything, and a torch that reached nothing
    // would each pass the assertion that matters while measuring no shadow at
    // all.
    assert!(
        verdict.compared > 200_000,
        "only {} of {} pixels were compared — a detector that compares nothing reads exactly like a \
         detector that found nothing",
        verdict.compared,
        SIDE * SIDE,
    );
    let lit = verdict.traced_lit.iter().flatten().filter(|lit| **lit).count();
    let dark = verdict.traced_lit.iter().flatten().filter(|lit| !**lit).count();
    assert!(
        lit > 10_000 && dark > 10_000,
        "the tracer saw {lit} lit and {dark} shadowed pixels: a scene that is all one or the other \
         agrees with anything"
    );
    assert!(
        verdict.back_facing > 1_000,
        "only {} back-facing pixels: the comparison is not reaching the surfaces the walk's own \
         exemption decides, which are the ones it is most worth reaching",
        verdict.back_facing,
    );

    // And the gate. Every pixel where the two renderers agree what surface is
    // there, and where neither picture has an edge running through the pixel's
    // own neighbourhood, they agree about whether the flame reaches it — against
    // a renderer that shares no arithmetic with ours and has no notion of a tile.
    assert_eq!(
        verdict.interior,
        0,
        "the path tracer and the frame disagree about {} pixels that no edge and no surface \
         disagreement explains\n{}",
        verdict.interior,
        verdict.report(),
    );
}

/// How far apart the two pictures may be, in steps of an eight-bit channel.
///
/// **A quantisation, not a tolerance.** The two sides compute the same product
/// of the same numbers in different precisions — the shader in `f32` through a
/// rasteriser, the tracer in `f64` through an analytic intersection — and then
/// round to a byte. One step is two answers landing either side of a rounding
/// boundary; anything above it is a difference in what was computed.
///
/// Two rather than one because the *fragment* differs as well as the arithmetic:
/// the shader lights the point the rasteriser wrote at the pixel's centre and
/// the tracer lights the point its own ray meets, and on a plane seen at 4:1
/// those are the same place to within a fraction of a tile — but a fraction of a
/// tile is a real distance to a falloff curve.
const QUANTISATION: u8 = 2;

/// The engine's shaded frame and the path tracer's agree about *brightness*, on
/// the one scene where nothing else can explain a difference.
///
/// **`docs/lighting_rebuild.md`'s phase 0 "done when", as a gate.** The scene is
/// the one that phase names: one flame, flat ground, no occluders. What that
/// buys is that everything the two renderers could disagree about *except* the
/// light is gone — no silhouette, so no rasteriser fill rule; no box, so no
/// invented albedo; no occluder, so no shadow ray. What is left is the falloff
/// curve, the intensity and the colour pipeline, which is exactly the
/// calibration every later phase rests on.
///
/// The three things that had to become true for it to be possible to write,
/// each of which was a difference of its own until phase 0:
///
/// - **the albedo is the same on both sides** — read off the world texture the
///   ground pass drew, not written down twice;
/// - **the flame is the same flame** — `Light`'s own colour and intensity, where
///   the reference used to carry a `6.0` picked to make its own picture
///   readable;
/// - **one curve** — both encoded by `tonemap::encode`, the second half of what
///   `blit.wesl` ends a lit frame with.
///
/// And the ambient is *nothing*, deliberately: a degenerate path trace is direct
/// light and has no ambient term, so `NIGHT` here would be a constant on one
/// side of the comparison only — and not one that could be subtracted back out
/// afterwards, since the sum passes through a tonemap.
#[test]
fn the_frame_and_the_path_tracer_agree_about_brightness_on_open_ground() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    // Over the middle of the anchor tile, so the pool's brightest point is in the
    // frame and its rim is too: a comparison that saw only the tail of the curve
    // would agree about a curve of nearly any shape.
    //
    // A tile up, and it used to be a quarter of one. **Phase 3 is what moved
    // it**, and not by a margin: the shading term is a cosine now, and a source
    // three `z` above flat ground is nearly *in* that ground — its cosine is
    // under a tenth over most of a 512-pixel canvas, so the picture had 812
    // bright pixels against the ten thousand this gate needs to be measuring a
    // curve at all. The height is what puts a bright core back under the flame;
    // the curve either side of it is unchanged and is still what is being judged.
    let at = WorldSpot {
        x: 100.5,
        y: 100.5,
        z: f64::from(openshard_client_render::light::Z_PER_TILE),
    };
    let dark = openshard_client_render::light::Ambient {
        sky: [0.0; 3],
        ground: [0.0; 3],
    };
    // Brighter than a torch on purpose, and not to make the picture pretty: the
    // guard below needs ten thousand pixels either side of the frame's own
    // midpoint, and with a cosine in the term the bright half is a disc under the
    // flame. At `1.0` it was four thousand — a comparison of the curve's tail
    // alone, which is a curve nearly any other curve also fits.
    const BRIGHTNESS: f32 = 3.0;
    let frame = render(
        &device,
        &queue,
        Shot {
            boxes: &[],
            flame: at,
            radius: FLAME_RADIUS,
            intensity: BRIGHTNESS,
            ambient: dark,
            view: View::Lit,
            zoom: 0,
        },
    );

    // What the ground reflects, off the picture itself — the same decode the
    // tool uses, so the gate and the pictures a person looks at are about one
    // albedo rather than two.
    let land = openshard_client_render::place::Kind::Land as u32;
    let albedo = oracle::ground_albedo(&frame.drawn, &frame.world);

    let mirror = oracle::pathtrace::Mirror::of(oracle::pathtrace::Mirrored {
        boxes: &[],
        light_at: at,
        light_radius: f64::from(FLAME_RADIUS),
        colour: [1.0, 1.0, 1.0],
        // The engine's Lambert has no `1/π` and the reference's does — see
        // `LAMBERT_PI`. This is the whole of the conversion between the two
        // conventions, and it is stated once, here, rather than by either side
        // being rewritten to look like the other.
        intensity: f64::from(BRIGHTNESS) * oracle::pathtrace::LAMBERT_PI,
        albedos: oracle::pathtrace::Albedos {
            ground: albedo,
            // No body in the scene to have one. Left at the invented value
            // rather than at zero so that a box arriving here later fails loudly
            // as a difference rather than quietly as a black shape.
            ..oracle::pathtrace::Albedos::INVENTED
        },
        to_pixel: frame.to_pixel.as_ref(),
    });
    // **`Lambert`, and phase 3 is what changed it from `Flat`.** `Brdf::Flat` is
    // a description of the engine *before* this phase — no cosine and no notion
    // of a normal anywhere in it — so leaving the gate there would have made the
    // reference agree with the renderer we have just replaced.
    let traced = mirror.render(pt_trace::Brdf::Lambert, SIDE, SIDE);

    // Compared where the tracer sees the ground and the frame drew it: the two
    // agreeing what surface is there is the same precondition the shadow gate
    // has, and on this scene it is the whole of the ground plane in view.
    let (mut compared, mut worst, mut over) = (0usize, 0u8, 0usize);
    let mut worst_at = (0u32, 0u32);
    // Of the compared pixels, how many are in the bright half of the pool and
    // how many in the dark one — counted here rather than over the whole canvas,
    // because a picture is only evidence about a curve where the curve was
    // actually asked.
    let (mut bright, mut dim) = (0usize, 0usize);
    for pixel in 0..(SIDE * SIDE) as usize {
        if frame.drawn[pixel].kind != land {
            continue;
        }
        let Some(seen) = traced.pixels[pixel].seen else {
            continue;
        };
        if seen.surface != openshard_client_pathtrace::scene::Surface::Ground {
            continue;
        }
        compared += 1;
        let engine = [
            frame.surface[pixel * 4],
            frame.surface[pixel * 4 + 1],
            frame.surface[pixel * 4 + 2],
        ];
        let reference = openshard_client_render::tonemap::encode_u8(
            traced.pixels[pixel].radiance.map(|channel| channel as f32),
        );
        let apart = (0..3)
            .map(|channel| engine[channel].abs_diff(reference[channel]))
            .max()
            .expect("three channels");
        if apart > worst {
            worst = apart;
            worst_at = ((pixel as u32) % SIDE, (pixel as u32) / SIDE);
        }
        over += usize::from(apart > QUANTISATION);
        match engine[0] > 128 {
            true => bright += 1,
            false => dim += 1,
        }
    }

    eprintln!(
        "shaded frame vs path tracer, flat ground: {compared} pixels compared ({bright} bright, \
         {dim} dim), worst channel {worst} steps of 255 at {worst_at:?}, {over} past the \
         {QUANTISATION}-step quantisation"
    );

    // The scene has to be one where the answer could have been wrong: a frame
    // that drew no ground, or a flame that reached none of it, would agree
    // perfectly about nothing. The pool's own rim is inside the canvas, so both
    // a bright band and a dark one have to be there.
    assert!(
        compared > 200_000,
        "only {compared} of {} pixels were compared",
        SIDE * SIDE
    );
    assert!(
        bright > 10_000 && dim > 10_000,
        "the frame has {bright} bright and {dim} dim ground pixels: a picture that is all one \
         brightness agrees with any falloff curve at all"
    );

    assert_eq!(
        over, 0,
        "the path tracer and the shaded frame disagree about brightness on {over} of {compared} \
         ground pixels by more than {QUANTISATION} steps of 255 — worst {worst} at {worst_at:?}. \
         The scene has one flame, flat ground and no occluders, so nothing but the falloff curve, \
         the flame's own intensity and the colour pipeline can be what differs.\n\
         `cargo run --release -p openshard-client-render --example boxes` with \
         `OPENSHARD_BOXES_SCENE=flat` draws the two pictures side by side."
    );
}
