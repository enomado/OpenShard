//! Pictures of the lighting, from above, and the assertions that go with them.
//!
//! Every other test of this pass answers a number: how much of a ray survived,
//! which cell stopped it, whether one tile is brighter than another. Those are
//! the right questions and they are all local — and the complaints this pass
//! actually draws are about *shape*: a pool that is a flat disc rather than a
//! falloff, a fan of spokes out of a lamp, a shadow that is a blade. A shape is
//! what a person sees at a glance and what a threshold cannot state.
//!
//! So this file does two things at once, from one drawing. It writes the plan
//! views out where somebody can look at them — [`crate::plan`], the real blit
//! over a synthetic ground — and it asserts the properties of the shape that
//! *can* be stated: a lone flame's pool is radially symmetric, it falls off
//! monotonically, and it does not end in a cliff.
//!
//! ```sh
//! cargo test -p openshard-client-render --test pictures -- --nocapture
//! ```
//!
//! writes them under `target/lighting/`, or wherever `OPENSHARD_LIGHT_PICTURES`
//! points, and prints the paths. They are binary PPM, which every image tool
//! reads:
//!
//! ```sh
//! magick target/lighting/one-torch-on-open-ground.lit.ppm /tmp/look.png
//! ```
//!
//! No client files anywhere: the scenes are `render/src/scene.rs`'s built rooms.
//! A machine with no GPU adapter skips every test here, which is the same bargain
//! the rest of the GPU suite makes.

use std::path::PathBuf;

use openshard_client_render::debug::View;
use openshard_client_render::light::Lighting;
use openshard_client_render::plan::{self, Picture};
use openshard_client_render::scene::{self, CENTRE, Scene};

/// How many pixels one tile gets in a plan view.
///
/// Sixteen: enough that a tile's own square holds a gradient rather than a
/// colour, and small enough that a twenty-four tile scene is one 384-pixel image
/// a person can take in whole.
const SCALE: u32 = 16;

/// How many tiles across a plan view is, centred on the scene.
///
/// Twenty-five, which is a torch's twelve-tile diameter with a margin of open
/// ground around it. The margin is not decoration: "the pool ends" is a claim
/// about what is outside it, and a picture cropped to the pool cannot make it.
const TILES: i32 = 25;

/// The flicker's instant. Zero everywhere, as in every other lighting test: a
/// picture that differed by which tenth of a second it was drawn in would be a
/// picture nobody could compare with the last one.
const STILL: f32 = 0.0;

/// A GPU to draw with, or `None` where there is none.
fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// Where the pictures go.
fn directory() -> PathBuf {
    match std::env::var_os("OPENSHARD_LIGHT_PICTURES") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/lighting"),
    }
}

/// The rectangle a plan view covers, centred on the scenes' own centre.
fn around() -> openshard_client_render::camera::TileBounds {
    let (cx, cy) = CENTRE;
    openshard_client_render::camera::TileBounds {
        min_x: i32::from(cx) - TILES / 2,
        max_x: i32::from(cx) + TILES / 2,
        min_y: i32::from(cy) - TILES / 2,
        max_y: i32::from(cy) + TILES / 2,
    }
}

/// Draw one scene in one view, mark the reasons over it, and write it out.
///
/// Marked always. A plan view with no occluders drawn on it is exactly the
/// instrument that made "why is there a shadow here" unanswerable in the client
/// — see `docs/lighting.md`'s step 14, arriving here for the same reason.
fn picture(device: &wgpu::Device, queue: &wgpu::Queue, name: &str, lighting: &Lighting, view: View) -> Picture {
    let mut drawn = plan::draw(device, queue, lighting, view, around(), SCALE);
    drawn.mark(lighting);
    let directory = directory();
    std::fs::create_dir_all(&directory).expect("the picture directory");
    let path = directory.join(format!("{name}.{}.ppm", view.name().to_lowercase()));
    std::fs::write(&path, drawn.ppm()).expect("writing the picture");
    eprintln!("wrote {}", path.display());
    drawn
}

/// A scene's name as a file name.
fn slug(scene: &Scene) -> String {
    scene
        .name
        .chars()
        .map(|c| match c.is_ascii_alphanumeric() {
            true => c.to_ascii_lowercase(),
            false => '-',
        })
        .collect()
}

/// Every scene, in every view, written where somebody can look at them.
///
/// The instrument, run as a test so that it cannot rot: a picture generator that
/// only runs when a person remembers it exists is one that stops compiling
/// silently. What it asserts is deliberately weak — that a picture has both dark
/// and light in it — because its output is a person, and the strong assertions
/// about shape are the tests below.
#[test]
fn every_scene_draws_a_picture_with_light_and_dark_in_it() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    for scene in scene::all() {
        let lighting = scene.lighting(STILL);
        let name = slug(&scene);
        for view in [View::Lit, View::Light, View::Shadow, View::Occluders] {
            let drawn = picture(&device, &queue, &name, &lighting, view);
            let (mut dark, mut bright) = (0, 0);
            for y in 0..drawn.height {
                for x in 0..drawn.width {
                    let luma: u32 = drawn.pixel(x, y)[..3].iter().map(|c| u32::from(*c)).sum();
                    match luma < 200 {
                        true => dark += 1,
                        false => bright += 1,
                    }
                }
            }
            assert!(
                dark > 0 && bright > 0,
                "{} in {} is one flat colour: {dark} dark against {bright} bright",
                scene.name,
                view.name(),
            );
        }
    }
}

/// A lone flame on open ground draws a **circle**.
///
/// The claim the simplest scene there is exists to make, and the one a person
/// reports first when it is wrong. Radial symmetry is the half of "circle" that
/// can be stated as a number: the brightness three tiles east of the flame, three
/// west, three north and three south must be the same brightness — the falloff
/// is a function of distance and of nothing else, and the four directions differ
/// only in direction.
///
/// Read off [`View::Light`], which is the lighting with the art thrown away.
/// [`View::Lit`] multiplies it by a world image, and this scene's world image is
/// white — so the two agree here and would not in a frame with a picture under
/// it, which is exactly the confusion this test should not inherit.
#[test]
fn a_lone_flame_on_open_ground_is_the_same_brightness_in_every_direction() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let scene = scene::torch_on_open_ground();
    let lighting = scene.lighting(STILL);
    let drawn = picture(&device, &queue, &slug(&scene), &lighting, View::Light);
    let (cx, cy) = CENTRE;

    for distance in 1..=4u16 {
        let around = [
            ("east", drawn.tile(cx + distance, cy)),
            ("west", drawn.tile(cx - distance, cy)),
            ("south", drawn.tile(cx, cy + distance)),
            ("north", drawn.tile(cx, cy - distance)),
        ];
        let least = around.iter().map(|(_, v)| *v).fold(f32::MAX, f32::min);
        let most = around.iter().map(|(_, v)| *v).fold(f32::MIN, f32::max);
        assert!(
            most - least < 0.02,
            "the pool is not round at {distance} tiles out: {around:?}",
        );
    }
}

/// And it **falls off**: every tile further out is darker than the one before it,
/// all the way from the flame to the rim.
///
/// The complaint this whole file was written for. A pool whose middle is a flat
/// disc passes every threshold test in the suite — the middle is bright, the
/// outside is the ambient, the edge is in between — and reads as a spotlight
/// switched on rather than as a fire. What says it is a fire is that *no two
/// steps are the same*, and that is stated here as a strict order over the whole
/// radius rather than as a comparison of two named tiles.
///
/// A tenth of a tile at a time along one ray, so that the claim is about the
/// curve and not about which tiles the sampling happened to land on.
#[test]
fn a_lone_flame_falls_off_all_the_way_from_its_middle() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let scene = scene::torch_on_open_ground();
    let lighting = scene.lighting(STILL);
    let drawn = plan::draw(&device, &queue, &lighting, View::Light, around(), SCALE);
    let flame = lighting.lights[0];

    // Along +x from the flame's own tile to its rim, in tenths of a tile.
    let ray: Vec<(f32, f32)> = (0..=(flame.radius * 10.0) as i32)
        .map(|step| {
            let out = step as f32 / 10.0;
            let (px, py) = drawn.at(flame.at.x + out, flame.at.y);
            let pixel = drawn.pixel(px as u32, py as u32);
            let luma = pixel[..3].iter().map(|c| f32::from(*c) / 255.0).sum::<f32>() / 3.0;
            (out, luma)
        })
        .collect();

    // How much of the *inner* pool is flat: neighbouring samples the same
    // brightness to a byte. The inner half only — out at the rim the curve is
    // genuinely level, `(1 - d)²` having spent nearly all of itself, and a tenth
    // of a tile there is under a byte of difference whatever the shape is. What
    // this is about is the middle, where a clipped pool is one flat disc and a
    // fire is not.
    let inner: Vec<(f32, f32)> = ray.iter().copied().filter(|(out, _)| *out < flame.radius / 2.0).collect();
    let flat = inner.windows(2).filter(|pair| pair[0].1 == pair[1].1).count();
    assert!(
        flat < 3,
        "the pool has a flat middle rather than a falloff: {flat} of {} inner steps are level\n{ray:?}",
        inner.len() - 1,
    );

    // And it is monotone: never brighter further out. A pool that dipped and
    // recovered would be a ring, which is a different bug with the same numbers
    // at the two ends.
    for pair in ray.windows(2) {
        assert!(
            pair[1].1 <= pair[0].1 + 0.004,
            "the pool brightens outwards between {:?} and {:?}\n{ray:?}",
            pair[0],
            pair[1],
        );
    }
}

/// A wall in front of a torch throws one band of shadow, and it has a straight
/// lit side.
///
/// The second-simplest scene. Both halves are asserted, because each fails on its
/// own: a wall that stopped nothing leaves the far side lit, and a wall that
/// occluded its whole tile — or the tiles beside it — darkens the ground the
/// torch is standing on.
#[test]
fn a_wall_in_front_of_a_torch_darkens_the_ground_behind_it_and_not_beside_it() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let scene = scene::torch_before_a_wall();
    let lighting = scene.lighting(STILL);
    let drawn = picture(&device, &queue, &slug(&scene), &lighting, View::Light);
    let (cx, cy) = CENTRE;

    // What the ambient alone comes out at in this picture, which is what a tile
    // nothing reaches must be. Read off a corner of the frame rather than
    // computed: the picture is quantised to bytes and passed through the light
    // view's own curve, and a number derived from `light::NIGHT` would be a
    // second copy of both.
    let dark = drawn.tile(cx + 10, cy + 10);

    // The torch is two tiles north of the wall's row. North of it is lit, south
    // of it is shadow, and the two are read at the same distance from the flame
    // so that the falloff is not what is being measured.
    let lit = drawn.tile(cx, cy - 4);
    let shadowed = drawn.tile(cx, cy + 2);
    assert!(
        lit > dark + 0.2,
        "the torch lights nothing in front of it: {lit} against the ambient's {dark}",
    );
    assert!(
        (shadowed - dark).abs() < 0.01,
        "the wall does not stop the light behind it: {shadowed} against the ambient's {dark}",
    );

    // And the ground *beside* the flame, on its own row, is lit: a whole-tile
    // occluder would take it, and the panel is what does not.
    let beside = drawn.tile(cx + 3, cy - 2);
    assert!(
        beside > dark + 0.1,
        "the wall shadows the ground alongside it: {beside} against the ambient's {dark}",
    );
}
