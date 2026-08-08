//! The reference tracer's own side of a scene, and the judging of the two
//! pictures against each other.
//!
//! Shared by `examples/boxes.rs` — the tool a person points at a scene — and
//! `tests/traced.rs`, the gate that runs the same comparison under
//! `cargo test`. **The judging is the part that must not be duplicated.** The
//! pipeline boilerplate around it (build a scene, render it, read it back) is
//! copied all over this crate's GPU tests already and a second copy is a
//! nuisance; a second copy of *what counts as a disagreement* is a second
//! opinion, and the day the two drift, the gate is green about a rule the tool
//! no longer applies.
//!
//! What lives here is therefore: the tracer's view of a scene the renderer just
//! drew ([`Mirror`]), and the pixel-by-pixel verdict ([`compare`]). What does
//! not: anything about how the frame was produced, and anything about what to
//! do with the answer.
//!
//! The reasoning behind each split — why an out-of-reach pixel, a back-facing
//! pixel and a silhouette pixel are three different things and none of them is
//! a shadow — is in `docs/lighting_reference.md`.

use std::collections::BTreeMap;

use openshard_client_pathtrace::aabb as pt_aabb;
use openshard_client_pathtrace::camera as pt_camera;
use openshard_client_pathtrace::light as pt_light;
use openshard_client_pathtrace::scene as pt_scene;
use openshard_client_pathtrace::trace as pt_trace;
use openshard_client_pathtrace::vector as pt_vector;
use openshard_client_render::camera::WorldSpot;
use openshard_client_render::light;
use openshard_client_render::place::Stance;

// `super`, not `crate`: this module is reached from two different crate roots —
// an example and a test — and only a relative path is true in both.
use super::boxes::BoxSpec;
use super::{Drawn, Shade};

/// What the surfaces of a scene are worth, in linear reflectance.
///
/// **Taken from the caller because the caller is the side that knows.** These
/// were two literals in [`Mirror::of`] until `docs/lighting_rebuild.md`'s phase
/// 0 asked for the two shaded pictures side by side, at which point they became
/// the largest difference between them and one that is not about light at all: a
/// reference whose ground is a different colour from the ground in the frame
/// disagrees about every pixel of it, brightly, for a reason no phase of the
/// rebuild will ever fix.
///
/// A comparison of *shadows* never cared — [`compare`] reads visibility, not
/// radiance — which is exactly why the literals survived as long as they did.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Albedos {
    /// The ground plane's, linear. The one a shaded comparison can take from the
    /// frame itself: the tool reads the art the world pass actually drew and
    /// decodes it, so "the same albedo on both sides" is a measurement rather
    /// than a claim.
    pub ground: [f64; 3],
    /// Every box's, linear. **Still a stand-in**, and the reason phase 0's
    /// scene has no boxes in it: `mesh_face.wesl` writes no colour into the
    /// world texture at all, so a box's face has nothing on the engine's side
    /// for this to be compared against. Phase 6 is where a mesh face gets an
    /// albedo; until then this number is what the *reference* thinks a box
    /// looks like and nothing checks it.
    pub body: [f64; 3],
}

impl Albedos {
    /// What the tracer assumed before anybody asked it to agree about
    /// brightness: a pale body on a duller ground, both invented.
    ///
    /// Kept as a named constant rather than spread back through the callers
    /// that do not care, so that "this scene never checked its own colours" is
    /// a thing a call site *says*.
    pub const INVENTED: Self = Self {
        ground: [0.42, 0.44, 0.40],
        body: [0.72, 0.70, 0.66],
    };
}

/// What an intensity is multiplied by to cross from the engine's Lambert into
/// [`pt_trace::Brdf::Lambert`]'s.
///
/// The engine's diffuse term is `albedo × N·L` and the reference's is
/// `albedo × N·L / π`. **The missing `1/π` is deliberate on our side and stated
/// in `docs/lighting_rebuild.md`**: putting it in would mean re-tuning every
/// authored flame in the tree for a constant everyone then divides back out. The
/// reference is physics and keeps it.
///
/// So a comparison across the two is a comparison of two conventions, and this is
/// the conversion between them — applied to the *flame's intensity*, which is the
/// one place it can go without either side's own formula being rewritten to suit
/// the other. It belongs to the comparison and not to `Mirrored`, which is why a
/// call site multiplies rather than `Mirror::of` dividing.
///
/// Nothing needs it against [`pt_trace::Brdf::Flat`], which has no cosine and no
/// `π` either — that variant is a description of the engine *before* phase 3.
pub const LAMBERT_PI: f64 = std::f64::consts::PI;

/// What shape the reference gives the flame, and how hard it works at it.
///
/// **The engine's flame is a sphere as of `docs/lighting_rebuild.md`'s phase 5**,
/// and a reference that kept it a point would report the whole penumbra as a
/// disagreement — which is what it did, on one pixel, the moment the eight rays
/// landed. So this is a field of the scene rather than a constant of the tracer:
/// a comparison says which body it is about, out loud, at the call site.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Body {
    /// A point. One shadow ray a pixel answers it exactly, which is what
    /// [`Mirror::render`]'s own `is_exact` assertion is about — and it is still
    /// the right question for a scene with no occluder in it, where the flame's
    /// size cannot cast anything.
    Point,
    /// The sphere the engine casts at: [`light::FLAME_RADIUS`] in tiles, sampled
    /// `samples` ways a pixel.
    ///
    /// The count is the reference's own and has nothing to do with
    /// [`light::SHADOW_RAYS`]: this side is the thing being measured *against*,
    /// so it wants enough samples that its own noise is small beside the
    /// engine's, and the comparison measures that noise rather than assuming it.
    Sphere { radius: f64, samples: u32 },
}

/// The engine's own flame, as the reference has to model it to be judging the
/// same scene: [`light::FLAME_RADIUS`], read across rather than written down
/// again.
///
/// **Sixty-four samples against the engine's eight**, and the ratio is the whole
/// point of the number: this side is the ruler, so its own noise has to be small
/// beside what it is measuring. Eight times the rays is roughly a third of the
/// standard error, and [`penumbra`] measures what is left rather than trusting
/// the arithmetic — see its `noise` field.
///
/// One constant so that the tool a person points at a scene and the gate that
/// runs under `cargo test` cannot drift into judging two different flames.
pub const ENGINE_FLAME: Body = Body::Sphere {
    radius: light::FLAME_RADIUS as f64,
    samples: 64,
};

/// The seed every picture a person looks at is rendered under, and the second
/// one [`penumbra`] measures the reference's own noise with.
///
/// Two named constants rather than two literals at a call site, because the
/// *pair* is the instrument: one seed is a render and two seeds are an error bar,
/// and a caller passing the same number twice would get an error bar of zero and
/// a gate that believes itself exactly.
pub const FIRST_SEED: u64 = 0;
pub const SECOND_SEED: u64 = 1;

/// How far apart the two penumbras may be, as a fraction of the whole flame.
///
/// **A sampling bound, not a tolerance**, and it is one ray of the engine's own
/// [`light::SHADOW_RAYS`]. Eight stratified samples of the share of a disc that
/// is visible give a count that is the true share times eight, plus or minus the
/// one sample the pattern's rotation moves across the edge — so an eighth is what
/// the estimator can be wrong by before anything is wrong with the *model*. The
/// reference's own noise is measured beside it rather than folded into it: see
/// [`Penumbra::noise`], which is what says whether this number is doing any work.
pub const PENUMBRA_ALLOWED: f64 = 1.0 / light::SHADOW_RAYS as f64;

/// The same scene, in the tracer's own terms.
///
/// Two things are taken from the renderer to build it, and both arrive as
/// **values rather than as formulas** — see `docs/lighting_reference.md`.
pub struct Mirror {
    pub scene: pt_scene::Scene,
    pub camera: pt_camera::Parallel,
    pub flame: pt_light::Light,
    /// How many paths a pixel [`Mirror::render`] asks for, which the emitter
    /// decides: a point is exact in one and a sphere is an estimate in any
    /// number.
    samples: u32,
}

/// A scene to mirror, and everything about it the reference cannot invent.
///
/// A struct rather than seven positional arguments, and every field is a value
/// the *engine's* own scene already holds: the flame's colour and intensity are
/// `light::Light`'s own two fields, and the albedos are the art. A reference
/// that made up any of them would be judging a different scene and reporting the
/// difference as the renderer's.
pub struct Mirrored<'a> {
    pub boxes: &'a [BoxSpec],
    pub light_at: WorldSpot,
    pub light_radius: f64,
    /// `light::Light::color`, widened. Per channel, a plain multiplier.
    pub colour: [f64; 3],
    /// `light::Light::intensity`, widened. **Not the reference's own knob**: a
    /// tracer rendering the same scene at six times the brightness makes a
    /// picture nobody can lay beside a frame.
    pub intensity: f64,
    pub albedos: Albedos,
    /// What shape the flame is, and how hard the reference works at it. See
    /// [`Body`] — a comparison with an occluder in it wants the engine's own
    /// sphere, and one without cannot tell the difference.
    pub body: Body,
    /// The *renderer's* own world-to-pixel map, handed over as a black box for
    /// [`pt_camera::Parallel::measure`] to recover. This is the one thing the
    /// tracer takes from the render crate, and taking it as values rather than
    /// as a formula is what stops the reference camera from drifting into being
    /// nobody's camera.
    pub to_pixel: &'a dyn Fn(WorldSpot) -> (f64, f64),
}

impl Mirror {
    /// Build the tracer's view of a scene of boxes lit by one flame.
    pub fn of(mirrored: Mirrored<'_>) -> Self {
        let Mirrored {
            boxes,
            light_at,
            light_radius,
            colour,
            intensity,
            albedos,
            body,
            to_pixel,
        } = mirrored;
        // **The metric.** The tracer's world is isotropic and this one is not: a
        // step of one in `x` is a tile and a step of one in `z` is a tile's
        // eleventh (`light::Z_PER_TILE`, which the renderer states and uses for
        // exactly this reason — its own falloff needs all three axes in one
        // unit). Read from the engine's constant rather than written down again:
        // a second copy of it here would be a reference measuring a
        // differently-shaped world.
        //
        // Visibility alone would survive getting this wrong — it is invariant
        // under any affine change of coordinates — which is precisely why it has
        // to be right anyway: the soft-shadow and bounce modes are not, and a
        // scale error would show up there as a plausible-looking picture rather
        // than as a failure.
        let z_per_tile = f64::from(light::Z_PER_TILE);
        let isotropic = |x: f64, y: f64, z: f64| pt_vector::Vec3::new(x, y, z / z_per_tile);

        let scene = pt_scene::Scene {
            bodies: boxes
                .iter()
                .map(|b| pt_scene::Body {
                    shape: pt_aabb::Aabb::between(
                        isotropic(b.min.0, b.min.1, b.min.2),
                        isotropic(b.max.0, b.max.1, b.max.2),
                    ),
                    albedo: albedos.body,
                })
                .collect(),
            ground: Some(pt_scene::Ground {
                z: 0.0,
                albedo: albedos.ground,
            }),
        };

        // The camera, measured through the renderer's own projection. `about`
        // sits in the scene and the span is wide, both for precision: the map
        // narrows to `f32` at world-pixel magnitudes in the thousands, so a
        // central difference over a wide baseline is what keeps that noise out
        // of the recovered columns. The tolerance is a hundredth of a pixel —
        // far under anything visible, far over the noise.
        let camera = pt_camera::Parallel::measure(
            |at| {
                to_pixel(WorldSpot {
                    x: at.x,
                    y: at.y,
                    z: at.z * z_per_tile,
                })
            },
            isotropic(light_at.x, light_at.y, 0.0),
            32.0,
            1e-2,
        );

        let (emitter, samples) = match body {
            Body::Point => (pt_light::Emitter::Point, 1),
            Body::Sphere { radius, samples } => (pt_light::Emitter::Sphere { radius }, samples),
        };
        let flame = pt_light::Light {
            at: isotropic(light_at.x, light_at.y, light_at.z),
            emitter,
            // The renderer's own windowed curve and its own radius, so "outside
            // the torch's reach" means the same thing on both sides of the
            // comparison. A physical inverse square here would darken the far
            // half of the frame and every one of those pixels would read as a
            // disagreement about geometry, which is not what any of them would
            // be about.
            falloff: pt_light::Falloff::Windowed { reach: light_radius },
            colour,
            intensity,
        };

        Self {
            scene,
            camera,
            flame,
            samples,
        }
    }

    /// Render this scene in one light model, deterministically.
    ///
    /// `seed` names the render: the same seed and the same scene give the same
    /// image on any machine, and *two different seeds are how the noise of an
    /// estimate is measured* rather than argued about — see [`penumbra`].
    ///
    /// # Panics
    ///
    /// If a [`Body::Point`] render comes back an estimate. That combination is
    /// the one the shadow gate's own `interior == 0` rests on, and an estimate
    /// disagreeing with a hard-shadow frame would not be a finding. A
    /// [`Body::Sphere`] render *is* an estimate by construction and says so by
    /// not being asked.
    pub fn render(&self, brdf: pt_trace::Brdf, seed: u64, width: u32, height: u32) -> pt_trace::Image {
        let image = pt_trace::render(
            &self.scene,
            &self.camera,
            std::slice::from_ref(&self.flame),
            &pt_trace::Settings {
                brdf,
                samples: self.samples,
                seed,
                ..pt_trace::Settings::degenerate()
            },
            width,
            height,
        );
        assert!(
            image.is_exact() || self.samples > 1,
            "a one-sample render of an area emitter is an estimate with nothing to average, which is \
             neither of the two things a caller can mean"
        );
        image
    }
}

/// What the renderer left on the pixels, as the comparison needs it.
pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    /// The `place` attachment, decoded — who drew each pixel.
    pub drawn: &'a [Drawn],
    /// The `View::Shadow` picture, `RGBA8`.
    pub shadow: &'a [u8],
    /// Which box and stance each mesh-face instance row is, as the tool that
    /// pushed the rows recorded it rather than as anybody's guess about the
    /// order they went in.
    pub face_rows: &'a [(usize, Stance, u32)],
}

/// Every count the comparison produces, and the two maps it produced them from.
///
/// One struct rather than a printed line, because two callers want different
/// things out of it: the tool prints and draws, the gate asserts.
pub struct Verdict {
    /// Pixels where both sides agree what surface is there and the comparison
    /// therefore had an opinion. Asserted non-trivial by [`compare`]: a
    /// detector that silently compares nothing reads exactly like a detector
    /// that found nothing.
    pub compared: usize,
    /// Of those, the ones that disagree away from any edge. **This is the
    /// number.** Anything else has an explanation that is not the walk.
    pub interior: usize,
    /// And the ones that disagree within a pixel of a shadow's own edge, which
    /// half a pixel of sampling difference explains.
    pub edge: usize,
    /// Pixels the two disagree about *which surface is there*, on a silhouette
    /// — the same half a pixel, deciding which of two surfaces a ray meets.
    pub silhouette: usize,
    /// And the ones that do not sit on one, which nothing sub-pixel explains.
    pub interior_surface: usize,
    /// What those were, by name.
    pub surface_pairs: BTreeMap<String, usize>,
    /// Pixels of a surface whose own normal points away from the flame, and how
    /// many of those the frame draws lit.
    pub back_facing: usize,
    pub back_facing_lit: usize,
    /// How many compared pixels the *choice of light model* decides — the two
    /// models rendered and subtracted, rather than inferred from a back-face
    /// count.
    pub model_decides: usize,
    /// Pixels with nothing the two have a common vocabulary for: the cleared
    /// background, or a stance the tracer's scene has no counterpart for.
    pub nothing_drawn: usize,
    /// The frame's own lit/shadowed decision and the tracer's, one bit a pixel,
    /// `None` where the pixel was not compared. The picture and the comparison
    /// are the same data.
    pub engine_lit: Vec<Option<bool>>,
    pub traced_lit: Vec<Option<bool>>,
    /// A handful of each kind of disagreement, in words, for a person.
    pub examples: Vec<String>,
    pub surface_examples: Vec<String>,
}

impl Verdict {
    /// The whole verdict as the two lines a run prints, plus a line per named
    /// disagreement.
    pub fn report(&self) -> String {
        let mut out = format!(
            "path tracer vs rendered View::Shadow: {} pixels compared, {} disagree in the interior, {} \
             on an edge (expected); {} pixels the two disagree about which surface is there ({} of them \
             on a silhouette, {} not), {} with nothing drawn\n",
            self.compared,
            self.interior,
            self.edge,
            self.silhouette + self.interior_surface,
            self.silhouette,
            self.interior_surface,
            self.nothing_drawn,
        );
        out.push_str(&format!(
            "  compared in the engine's own light model (no cosine, no self-occlusion): {} of those \
             pixels are surfaces facing away from the flame, the frame draws {} of them lit, and the \
             choice of model decides {} pixels of the whole comparison — that last number is what a \
             physical `N·L` would move\n",
            self.back_facing, self.back_facing_lit, self.model_decides,
        ));
        for (pair, count) in &self.surface_pairs {
            out.push_str(&format!(
                "  [different surface, not on a silhouette] {count}: {pair}\n"
            ));
        }
        for example in self.surface_examples.iter().chain(&self.examples) {
            out.push_str(example);
            out.push('\n');
        }
        out
    }
}

/// What a penumbra comparison found, and what the reference's own noise was
/// while it found it.
///
/// **The noise is a field and not a footnote.** Both sides of this comparison are
/// estimates — the engine's eight stratified rays and the reference's own random
/// ones — so "they agree to within `d`" means nothing until `d` is laid beside
/// how much the reference disagrees *with itself*. That second number is measured
/// by rendering the same scene under two seeds, which is the only way to get it
/// that does not involve believing an argument about variance.
pub struct Penumbra {
    /// Pixels where both sides say the flame partly reaches: the penumbra, which
    /// is the only place this comparison has anything to be about.
    pub compared: usize,
    /// The largest and the mean difference over those, as fractions of the whole
    /// flame.
    pub worst: f64,
    pub mean: f64,
    /// Where the worst one is, for a person with the picture open.
    pub worst_at: (u32, u32),
    /// And the same two numbers for the reference against itself, one seed
    /// against another, over the same pixels.
    pub noise: f64,
    pub noise_mean: f64,
    /// The mean difference **with its sign**: how much brighter the engine's
    /// penumbra is than the reference's, on average.
    ///
    /// **The sharp number, and the reason the two means above are not enough.**
    /// Noise on either side is symmetric, so it cancels here and thousands of
    /// pixels average it away — the standard error of this over a few thousand
    /// pixels is a thousandth of a flame. What does *not* cancel is a difference
    /// in the model: a flame of the wrong radius, a disc sampled off centre, a
    /// penumbra sitting a pixel to one side of where it belongs. Those are what
    /// this phase could actually have got wrong, and they show up here at ten
    /// times their size in [`Penumbra::mean`].
    pub bias: f64,
    /// How many of the compared pixels are past what sampling explains: the
    /// caller's own `allowed` for the engine's side, plus the reference's
    /// measured disagreement with itself at that same pixel.
    pub over: usize,
}

impl Penumbra {
    /// The whole finding as the line a run prints.
    pub fn report(&self, allowed: f64) -> String {
        format!(
            "penumbra vs path tracer: {} pixels partly lit on both sides, worst {:.4} of the flame at \
             {:?}, mean {:.4}, signed mean {:+.4}; the reference against itself over the same pixels is \
             worst {:.4}, mean {:.4}; {} past {:.4} plus that pixel's own reference noise\n",
            self.compared,
            self.worst,
            self.worst_at,
            self.mean,
            self.bias,
            self.noise,
            self.noise_mean,
            self.over,
            allowed,
        )
    }
}

/// How much of the flame each side says a pixel can see, compared where they
/// both say "some of it".
///
/// **`docs/lighting_rebuild.md`'s phase 5 "done when".** [`compare`] reads the
/// two pictures as one bit a pixel, which is the right question for a hard shadow
/// and says nothing at all about the shape of a soft one: a penumbra is exactly
/// the region where that bit is arbitrary. This reads the *fraction* instead —
/// the engine's `View::Shadow` grey, which is `nearest_through`, against the
/// reference's own `Visibility::reached`.
///
/// `second` is the same scene under a second seed and is what measures the
/// reference's noise; `allowed` is the *engine's* own share of the budget, and it
/// is the caller's to state because it is a ray count that belongs to the engine.
/// A pixel counts as over when the two are further apart than `allowed` plus what
/// the reference disagrees with itself by at that same pixel — which is the whole
/// of "within sampling noise, and the noise is measured rather than asserted
/// away".
///
/// # Panics
///
/// If it found no penumbra to compare. A soft-shadow gate over a scene with no
/// soft shadow in it is green for every renderer there is.
pub fn penumbra(
    traced: &pt_trace::Image,
    second: &pt_trace::Image,
    frame: Frame<'_>,
    allowed: f64,
) -> Penumbra {
    let Frame {
        width,
        height,
        drawn,
        shadow,
        face_rows,
    } = frame;
    let mut found = Penumbra {
        compared: 0,
        worst: 0.0,
        mean: 0.0,
        worst_at: (0, 0),
        noise: 0.0,
        noise_mean: 0.0,
        bias: 0.0,
        over: 0,
    };
    let (mut total, mut noise_total, mut signed) = (0.0, 0.0, 0.0);
    for pixel in 0..(width * height) as usize {
        // The same precondition [`compare`] has: the two must agree what surface
        // is there, or the difference is about depth order and not about light.
        if traced_surface(&drawn[pixel], face_rows) != traced.pixels[pixel].seen.map(|seen| seen.surface) {
            continue;
        }
        let (x, y) = ((pixel as u32) % width, (pixel as u32) / width);
        let visibility = traced.visibility(x, y, 0);
        if !visibility.within_reach {
            continue;
        }
        // The penumbra, and nothing else: the umbra and the open ground are what
        // [`compare`]'s own bit already covers, and including them here would
        // bury the number this exists to produce under a field of exact zeros.
        if visibility.reached <= 0.0 || visibility.reached >= 1.0 {
            continue;
        }
        let Shade::Through(grey) =
            Shade::of([shadow[pixel * 4], shadow[pixel * 4 + 1], shadow[pixel * 4 + 2]])
        else {
            // `Blocked` and `Unreached` are the two the engine says in a colour
            // rather than in a level — see `Shade`. Neither is a fraction, and a
            // caller that read them as one would be reading `0.2` of a channel as
            // a fifth of a flame.
            continue;
        };
        let difference = f64::from(grey) / 255.0 - visibility.reached;
        let apart = difference.abs();
        let own = (second.visibility(x, y, 0).reached - visibility.reached).abs();
        found.compared += 1;
        total += apart;
        signed += difference;
        noise_total += own;
        found.noise = found.noise.max(own);
        // The engine's own bound plus what the *reference* is worth at this
        // pixel, measured. A pixel in the middle of a soft edge is where both
        // estimators are noisiest, and a budget that ignored the reference's
        // share would be a gate on the ruler.
        found.over += usize::from(apart > allowed + own);
        if apart > found.worst {
            found.worst = apart;
            found.worst_at = (x, y);
        }
    }
    assert!(
        found.compared > 0,
        "no pixel of this scene is partly lit on both sides — a soft-shadow gate over a scene with no \
         soft shadow in it is green for every renderer there is"
    );
    found.mean = total / found.compared as f64;
    found.noise_mean = noise_total / found.compared as f64;
    found.bias = signed / found.compared as f64;
    found
}

/// Which surface of the tracer's own scene the renderer says drew a pixel.
///
/// [`None`] where the two have no common vocabulary for it — the cleared
/// background, or a stance the tracer's scene has no counterpart for. A pixel
/// with no answer here is not compared, and is counted as not compared.
fn traced_surface(texel: &Drawn, face_rows: &[(usize, Stance, u32)]) -> Option<pt_scene::Surface> {
    if texel.kind == openshard_client_render::place::Kind::Land as u32 {
        return Some(pt_scene::Surface::Ground);
    }
    if texel.kind == openshard_client_render::place::Kind::Static as u32
        && texel.stance == Stance::MeshFace as u32
    {
        return face_rows
            .iter()
            .find(|(_, _, id)| *id == texel.id)
            .map(|(box_index, _, _)| pt_scene::Surface::Body(*box_index));
    }
    None
}

/// What the frame says drew a pixel, in words, for a disagreement report.
///
/// Not [`traced_surface`]'s answer: this one keeps the *face* — a box's own east
/// side and its lid are one body to the tracer and two rows to the renderer, and
/// which of them won a pixel is the whole content of a "which surface is there"
/// disagreement.
fn engine_side(texel: &Drawn, face_rows: &[(usize, Stance, u32)]) -> String {
    if texel.kind == openshard_client_render::place::Kind::Land as u32 {
        return "the ground".to_owned();
    }
    match face_rows.iter().find(|(_, _, id)| *id == texel.id) {
        Some((box_index, stance, _)) => format!("box {box_index}'s {stance:?}"),
        None => format!("kind {} stance {} row {}", texel.kind, texel.stance, texel.id),
    }
}

/// Whether the pixel at `(x, y)` has anything but its own answer in its eight-
/// neighbourhood — the boundary of whatever `map` is a map of.
///
/// The two renderers answer about *different points*: the tracer about the world
/// point under a pixel's centre, the shader about the fragment the rasteriser
/// wrote, quantised to a hundred-and-twenty-eighth of a tile. Half a pixel
/// decides the answer exactly at a boundary and nowhere else, so a disagreement
/// on one is not a finding and a disagreement away from one cannot be explained
/// by sub-pixel anything.
///
/// One function over any map, because that argument is not about *what* is being
/// compared: it is as true of "which surface is there" along a silhouette as it
/// is of "is this lit" along a shadow's edge, and a second copy of it for the
/// second map would be a second place for the neighbourhood to change.
fn on_an_edge<T: PartialEq>(map: &[Option<T>], width: u32, height: u32, x: u32, y: u32) -> bool {
    let own = &map[(y * width + x) as usize];
    (-1i32..=1).any(|dy| {
        (-1i32..=1).any(|dx| {
            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
            if nx < 0 || ny < 0 || nx as u32 >= width || ny as u32 >= height {
                return false;
            }
            &map[(ny as u32 * width + nx as u32) as usize] != own
        })
    })
}

/// Lay the frame and the tracer's two renders beside each other and count where
/// they disagree.
///
/// `engine_model` is the render in [`pt_trace::Brdf::Flat`] and `physical` the
/// same scene in [`pt_trace::Brdf::Lambert`]; the second is used for exactly one
/// thing, subtracting the two to measure how many pixels the choice of model
/// decides.
///
/// **This compares visibility, and `Flat` is still the engine's visibility
/// model** after phase 3 turned its *shading* term into a cosine. The variant's
/// three clauses are one fact — there is no normal — and the third of them is
/// "a surface point's own body does not occlude it", which the shipped walk
/// still states as an exemption. `docs/lighting_rebuild.md`'s phase 4 is what
/// restates that as identity and is where this argument has to be re-taken.
/// Brightness is judged in `Lambert` already: `tests/traced.rs`'s own gate and
/// `boxes.rs`'s shaded strip both moved there.
///
/// # Panics
///
/// If it compared too few pixels to mean anything.
pub fn compare(engine_model: &pt_trace::Image, physical: &pt_trace::Image, frame: Frame<'_>) -> Verdict {
    let Frame {
        width,
        height,
        drawn,
        shadow,
        face_rows,
    } = frame;

    // Both pictures as one bit a pixel, so the comparison and the image are the
    // same data. `None` where the pixel is not one the two can be compared on.
    let mut engine_lit: Vec<Option<bool>> = vec![None; (width * height) as usize];
    let mut traced_lit: Vec<Option<bool>> = vec![None; (width * height) as usize];
    // And what each side says is *there*, kept as maps for the same reason the
    // lit bits are: whether a surface disagreement sits on a silhouette or in
    // the middle of a face is a question about the pixel's neighbours, and it
    // cannot be asked one pixel at a time.
    let mut engine_surface: Vec<Option<pt_scene::Surface>> = vec![None; (width * height) as usize];
    let mut traced_surfaces: Vec<Option<pt_scene::Surface>> = vec![None; (width * height) as usize];
    let mut verdict = Verdict {
        compared: 0,
        interior: 0,
        edge: 0,
        silhouette: 0,
        interior_surface: 0,
        surface_pairs: BTreeMap::new(),
        back_facing: 0,
        back_facing_lit: 0,
        model_decides: 0,
        nothing_drawn: 0,
        engine_lit: Vec::new(),
        traced_lit: Vec::new(),
        examples: Vec::new(),
        surface_examples: Vec::new(),
    };

    for pixel in 0..(width * height) as usize {
        engine_surface[pixel] = traced_surface(&drawn[pixel], face_rows);
        traced_surfaces[pixel] = engine_model.pixels[pixel].seen.map(|seen| seen.surface);
        if engine_surface[pixel].is_none() {
            verdict.nothing_drawn += 1;
            continue;
        }
        if engine_surface[pixel] != traced_surfaces[pixel] {
            // Not a lighting disagreement, and counting it as one would file a
            // depth-sorting difference under the wrong defect entirely. The
            // isometric painter's order and a ray's own nearest hit are two
            // different answers to "what is in front", and where they differ
            // neither picture is about the other's surface. Counted below,
            // where the neighbourhood is available to say which kind it is.
            continue;
        }
        let frame_lit = Shade::of([shadow[pixel * 4], shadow[pixel * 4 + 1], shadow[pixel * 4 + 2]]).lit();
        let (x, y) = ((pixel as u32) % width, (pixel as u32) / width);
        let visibility = engine_model.visibility(x, y, 0);
        let lit = |seen: pt_trace::Visibility| seen.within_reach && seen.reached > 0.5;
        if !visibility.faces_light {
            verdict.back_facing += 1;
            verdict.back_facing_lit += usize::from(frame_lit);
        }
        // Asked of every pixel and not only of the back-facing ones: "the model
        // decides exactly the pixels facing away" is the expectation, and an
        // expectation a detector only ever measures where it already holds is
        // not one the detector can report on.
        verdict.model_decides += usize::from(lit(visibility) != lit(physical.visibility(x, y, 0)));
        engine_lit[pixel] = Some(frame_lit);
        traced_lit[pixel] = Some(lit(visibility));
    }

    for y in 0..height {
        for x in 0..width {
            let pixel = (y * width + x) as usize;
            if engine_surface[pixel].is_some() && engine_surface[pixel] != traced_surfaces[pixel] {
                let rim = on_an_edge(&engine_surface, width, height, x, y)
                    || on_an_edge(&traced_surfaces, width, height, x, y);
                match rim {
                    true => verdict.silhouette += 1,
                    false => {
                        verdict.interior_surface += 1;
                        let what = format!(
                            "the frame draws {}, the tracer sees {}",
                            engine_side(&drawn[pixel], face_rows),
                            match traced_surfaces[pixel] {
                                Some(surface) => format!("{surface:?}"),
                                None => "nothing at all".to_owned(),
                            }
                        );
                        if verdict.surface_examples.len() < 8 {
                            verdict
                                .surface_examples
                                .push(format!("  [pixel ({x}, {y})] {what}"));
                        }
                        *verdict.surface_pairs.entry(what).or_default() += 1;
                    }
                }
                continue;
            }
            let (Some(engine), Some(traced)) = (engine_lit[pixel], traced_lit[pixel]) else {
                continue;
            };
            verdict.compared += 1;
            if engine == traced {
                continue;
            }
            if on_an_edge(&traced_lit, width, height, x, y) || on_an_edge(&engine_lit, width, height, x, y) {
                verdict.edge += 1;
                continue;
            }
            verdict.interior += 1;
            if verdict.examples.len() < 8 {
                let seen = engine_model.pixels[pixel].seen.expect("compared, so it was seen");
                verdict.examples.push(format!(
                    "  [pixel ({x}, {y})] {:?} at ({:.3}, {:.3}, {:.3} tiles): the tracer says {}, the \
                     frame says {}",
                    seen.surface,
                    seen.at.x,
                    seen.at.y,
                    seen.at.z,
                    if traced { "lit" } else { "shadowed" },
                    if engine { "lit" } else { "shadowed" },
                ));
            }
        }
    }

    assert!(
        verdict.compared > 1000,
        "the path tracer compared only {} pixels against the frame — a detector that compares nothing \
         reads exactly like a detector that found nothing",
        verdict.compared
    );
    verdict.engine_lit = engine_lit;
    verdict.traced_lit = traced_lit;
    verdict
}
