//! The estimator: what a pixel is worth, and how sure we are of it.
//!
//! One loop, two modes. With a point emitter, one sample and no bounces, every
//! random draw below is either never made or has a single possible outcome, and
//! the loop degenerates into "is the light visible from this surface point" —
//! exactly the question the renderer's hard shadows answer, with no variance to
//! average away and nothing to converge. Widen the emitter, raise the sample
//! count, allow a bounce, and the same loop is an ordinary path tracer.
//!
//! Keeping those one body of code is deliberate. A reference with a separate
//! "fast exact path" would be two implementations again, and the one that gets
//! compared against the renderer would be the one nobody looks at.

use crate::camera::Parallel;
use crate::light::Light;
use crate::rng::Stream;
use crate::scene::{SURFACE_BIAS, Scene, Surface};
use crate::vector::Vec3;

/// How hard to work, and what the world outside the scene is worth.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Settings {
    /// Paths per pixel.
    pub samples: u32,
    /// How many times a path may bounce off a surface after the first. `0` is
    /// direct lighting only — no indirect light, no ambient occlusion, which is
    /// the model the renderer being checked has.
    pub bounces: u32,
    /// The radiance a path picks up when it leaves the scene without hitting
    /// anything. Both an environment light and, at `bounces > 0`, the only
    /// thing that makes ambient occlusion visible: a crevice is dark because
    /// its bounces find geometry where an open surface's find sky.
    pub sky: [f64; 3],
    /// Names the render. The same seed and the same scene give the same image
    /// on any machine — see [`crate::rng`].
    pub seed: u64,
}

impl Settings {
    /// The mode that is a gate rather than a picture: one path per pixel, no
    /// bounces, no sky.
    ///
    /// Paired with [`crate::light::Emitter::Point`] this makes the whole tracer
    /// deterministic — [`Image::is_exact`] is what says so, and says it by
    /// asking the lights rather than by trusting this constructor.
    pub fn degenerate() -> Self {
        Self {
            samples: 1,
            bounces: 0,
            sky: [0.0; 3],
            seed: 0,
        }
    }
}

/// What the camera ray found at a pixel.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Seen {
    pub surface: Surface,
    /// The world point the pixel's own ray met — the point every shadow ray
    /// from this pixel starts at.
    pub at: Vec3,
    pub normal: Vec3,
}

/// How much of one emitter one pixel could see.
///
/// Kept apart from the pixel's colour because it is the only part of this
/// tracer that the renderer has an opinion about: brightness here comes from a
/// physical model the renderer does not implement, and comparing the two as
/// colours would be comparing light models. Comparing them as visibility is
/// comparing geometry, which is the point.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Visibility {
    /// The fraction of the **facing** part of the emitter that is
    /// unobstructed, `0.0..=1.0`. Exactly `0.0` or `1.0` for a point emitter.
    ///
    /// Zero when nothing faces, so this alone cannot tell a surface in shadow
    /// from one turned away — read it with the two flags below, which is why
    /// they are here.
    pub reached: f64,
    /// Whether the emitter's falloff reaches this point at all.
    ///
    /// Separate from `reached` being zero, and the distinction is load-bearing:
    /// a pixel outside a torch's radius is dark because of the *radius*, and a
    /// pixel inside it with nothing arriving is dark because of the
    /// *geometry*. Only the second is a claim a visibility oracle can check.
    pub within_reach: bool,
    /// Whether any of the emitter is on the lit side of the surface at all.
    ///
    /// The third way a pixel can be dark, and the one that is not about
    /// occlusion in the slightest: a wall's north face with the torch to the
    /// south is unlit because of where it *points*, and no shadow ray was ever
    /// a possibility. A comparison that folds this into "shadowed" reports
    /// every back-facing pixel in the scene as a disagreement with any renderer
    /// that does not apply a cosine — which is a real difference between the
    /// two models, and one worth naming rather than burying in a shadow count.
    pub faces_light: bool,
}

/// One pixel's worth of answer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Pixel {
    /// [`None`] where the camera ray left the scene without meeting anything.
    pub seen: Option<Seen>,
    /// Linear radiance, per channel, before any tone mapping. Unbounded above:
    /// clamping here would throw away the one thing an exposure knob is for.
    pub radiance: [f64; 3],
}

/// A rendered frame.
#[derive(Clone, PartialEq, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Row-major, `width * height` of them.
    pub pixels: Vec<Pixel>,
    lights: usize,
    /// `width * height * lights`, pixel-major.
    visibility: Vec<Visibility>,
    exact: bool,
}

impl Image {
    /// How much of `light` the pixel at `(x, y)` could see.
    ///
    /// # Panics
    ///
    /// On an out-of-range pixel or light index — both are programmer error in a
    /// caller that rendered the image it is now reading.
    pub fn visibility(&self, x: u32, y: u32, light: usize) -> Visibility {
        assert!(
            x < self.width && y < self.height,
            "pixel ({x}, {y}) is outside the frame"
        );
        assert!(light < self.lights, "light {light} of {}", self.lights);
        self.visibility[((y * self.width + x) as usize) * self.lights + light]
    }

    /// Whether this image is an exact answer rather than an estimate.
    ///
    /// True only when every emitter was exact in the number of samples the
    /// settings actually spent and no bounce was allowed. A caller comparing
    /// against a renderer's hard shadows should assert this: a soft-shadow
    /// render disagreeing with a hard-shadow one is not a finding, and a
    /// comparison that cannot tell the two cases apart will report it as one.
    pub fn is_exact(&self) -> bool {
        self.exact
    }
}

/// One pixel's shadow-ray bookkeeping for one emitter, while it is being
/// rendered.
///
/// Three counts and not one fraction, because the three ways a pixel ends up
/// dark are not interchangeable — see [`Visibility`], which is what these
/// become.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
struct Tally {
    /// Samples the emitter produced at all: the rest were out of its own reach.
    arrived: u32,
    /// Of those, the ones on the lit side of the surface.
    facing: u32,
    /// And of those, the ones nothing stood in the way of.
    reached: u32,
}

/// Render `scene`, from `camera`, at `width × height`.
///
/// The pixel grid is the caller's: pixel `(x, y)` is sampled at its centre,
/// `(x + 0.5, y + 0.5)`, in whatever coordinates the camera's own map produced.
/// One sample at the centre and not a jittered box filter — the renderer this
/// is compared against does not antialias either, and a filter would blur the
/// exact thing under examination, a shadow's own edge.
pub fn render(
    scene: &Scene,
    camera: &Parallel,
    lights: &[Light],
    settings: &Settings,
    width: u32,
    height: u32,
) -> Image {
    let exact = settings.bounces == 0
        && lights.iter().all(|light| {
            light
                .exact_in_samples()
                .is_some_and(|needed| needed <= settings.samples)
        });
    let mut pixels = Vec::with_capacity((width * height) as usize);
    let mut visibility = vec![Visibility::default(); (width * height) as usize * lights.len()];
    // One scratch buffer for the whole frame rather than one allocation a
    // pixel: `reached` and `considered` per light, reset at each pixel.
    // Per light: how many emitter samples arrived, how many faced the
    // surface at all, and how many were in reach — three counts because
    // there are three ways a pixel can end up dark.
    let mut tally = vec![Tally::default(); lights.len()];
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            let mut stream = Stream::new(settings.seed, index as u64);
            let ray = camera.ray((f64::from(x) + 0.5, f64::from(y) + 0.5));
            let Some(first) = scene.hit(ray.at, ray.direction, f64::NEG_INFINITY) else {
                pixels.push(Pixel {
                    seen: None,
                    radiance: settings.sky,
                });
                continue;
            };
            tally.iter_mut().for_each(|counter| *counter = Tally::default());
            let mut total = [0.0; 3];
            for _ in 0..settings.samples {
                let path = walk(scene, lights, settings, first, &mut stream, &mut tally);
                for (channel, value) in total.iter_mut().zip(path) {
                    *channel += value;
                }
            }
            let samples = f64::from(settings.samples);
            for (light_index, light) in lights.iter().enumerate() {
                let counted = tally[light_index];
                visibility[index * lights.len() + light_index] = Visibility {
                    reached: match counted.facing {
                        0 => 0.0,
                        facing => f64::from(counted.reached) / f64::from(facing),
                    },
                    // Asked of the emitter's own centre, and so a statement
                    // about the falloff alone: a sphere half in reach is a
                    // shape this flag cannot describe and must not pretend to.
                    within_reach: light.falloff.at((light.at - first.at).length()).is_some(),
                    faces_light: counted.facing > 0,
                };
            }
            pixels.push(Pixel {
                seen: Some(Seen {
                    surface: first.surface,
                    at: first.at,
                    normal: first.normal,
                }),
                radiance: [total[0] / samples, total[1] / samples, total[2] / samples],
            });
        }
    }
    Image {
        width,
        height,
        pixels,
        lights: lights.len(),
        visibility,
        exact,
    }
}

/// One path, from a surface the camera already found out to wherever it ends.
///
/// The visibility tally is filled from the **first** surface only. That is the
/// one the pixel is a picture of, and the one the renderer has an opinion
/// about; counting a bounce's shadow rays into it would mix "can this pixel see
/// the torch" with "can somewhere else the light came from see it".
fn walk(
    scene: &Scene,
    lights: &[Light],
    settings: &Settings,
    first: crate::scene::Hit,
    stream: &mut Stream,
    tally: &mut [Tally],
) -> [f64; 3] {
    let mut radiance = [0.0; 3];
    let mut throughput = [1.0; 3];
    let mut hit = first;
    let mut bounces = 0;
    loop {
        let direct = direct_light(
            scene,
            lights,
            hit,
            stream,
            match bounces {
                0 => Some(tally),
                _ => None,
            },
        );
        for channel in 0..3 {
            radiance[channel] += throughput[channel] * direct[channel];
        }
        if bounces == settings.bounces {
            break;
        }
        bounces += 1;
        // Cosine-weighted, which is what makes the throughput update the albedo
        // alone: the Lambertian `albedo / π` and the cosine both cancel against
        // this density, leaving no `cos / pdf` term to get wrong.
        let direction = cosine_hemisphere(hit.normal, stream);
        for (channel, albedo) in throughput.iter_mut().zip(hit.albedo) {
            *channel *= albedo;
        }
        let from = hit.at + hit.normal * SURFACE_BIAS;
        let Some(next) = scene.hit(from, direction, 0.0) else {
            // The path left the scene: it picks up the sky, weighted by
            // everything it bounced off on the way out.
            for channel in 0..3 {
                radiance[channel] += throughput[channel] * settings.sky[channel];
            }
            break;
        };
        hit = next;
    }
    radiance
}

/// What the emitters deliver to one surface point directly.
///
/// When `tally` is given, each emitter's shadow rays are counted into it: how
/// many were considered — that is, how many could have arrived if nothing were
/// in the way — and how many did.
fn direct_light(
    scene: &Scene,
    lights: &[Light],
    hit: crate::scene::Hit,
    stream: &mut Stream,
    mut tally: Option<&mut [Tally]>,
) -> [f64; 3] {
    let mut arriving = [0.0; 3];
    let from = hit.at + hit.normal * SURFACE_BIAS;
    for (index, light) in lights.iter().enumerate() {
        let Some(sample) = light.sample(from, stream) else {
            continue;
        };
        let towards = sample.at - from;
        let cosine = hit.normal.dot(towards.normalized());
        if let Some(tally) = tally.as_deref_mut() {
            tally[index].arrived += 1;
        }
        if cosine <= 0.0 {
            // Behind the surface. Counted separately from an obstruction: a
            // shadow ray was never a possibility here, and folding the two
            // together would report a wall's own back as being in shadow.
            continue;
        }
        if let Some(tally) = tally.as_deref_mut() {
            tally[index].facing += 1;
        }
        if scene.blocked(from, sample.at) {
            continue;
        }
        if let Some(tally) = tally.as_deref_mut() {
            tally[index].reached += 1;
        }
        // Lambertian: `albedo / π` times the irradiance, which is the emitter's
        // own arriving term times this surface's cosine.
        for ((channel, albedo), incoming) in arriving.iter_mut().zip(hit.albedo).zip(sample.arriving) {
            *channel += albedo * std::f64::consts::FRAC_1_PI * incoming * cosine;
        }
    }
    arriving
}

/// A direction around `normal`, distributed as the cosine against it.
fn cosine_hemisphere(normal: Vec3, stream: &mut Stream) -> Vec3 {
    let (u, v) = normal.orthonormal_basis();
    let radius_squared = stream.unit();
    let azimuth = std::f64::consts::TAU * stream.unit();
    let radius = radius_squared.sqrt();
    u * (radius * azimuth.cos()) + v * (radius * azimuth.sin()) + normal * (1.0 - radius_squared).sqrt()
}

#[cfg(test)]
mod tests {
    use super::{Settings, cosine_hemisphere, render};
    use crate::aabb::Aabb;
    use crate::camera::Parallel;
    use crate::light::{Emitter, Falloff, Light};
    use crate::rng::Stream;
    use crate::scene::{Body, Ground, Scene, Surface};
    use crate::vector::Vec3;

    /// Straight down, one world unit to a pixel, `(0, 0)` at world `(0, 0)`.
    ///
    /// A camera whose answers can be checked in one's head, which is what a
    /// test of the estimator wants — the isometric one has its own tests in
    /// `camera.rs`, and using it here would put two things under test at once.
    fn top_down() -> Parallel {
        Parallel::measure(|at| (at.x, at.y), Vec3::ZERO, 16.0, 1e-9)
    }

    /// Every case below renders this much of the world.
    const FRAME: u32 = 24;

    /// A three-unit box on the ground, in the middle of the frame rather than
    /// against its corner.
    ///
    /// Where the box stands decides whether this file tests anything. Its
    /// shadow falls away from the light, so a box in the corner nearest the
    /// light throws its shadow straight out of frame, and every assertion below
    /// would be about pixels the shadow never reached. That is the same trap as
    /// asserting on an empty picture, one step less obvious, and it cost this
    /// file two green tests before the numbers were worked out on paper.
    fn one_box_scene() -> Scene {
        Scene {
            bodies: vec![Body {
                shape: Aabb::between(Vec3::new(10.0, 10.0, 0.0), Vec3::new(12.0, 12.0, 3.0)),
                albedo: [0.8; 3],
            }],
            ground: Some(Ground {
                z: 0.0,
                albedo: [0.5; 3],
            }),
        }
    }

    /// Up and to the `+x +y` side of the box, at three times its height.
    ///
    /// So the shadow runs from the box's own footprint at `(10, 10)` back to
    /// about `(7, 7)`: several pixels long, inside the frame at both ends, and
    /// short of the frame's own edge — a shadow clipped by the frame would make
    /// "how many pixels are partly lit" a fact about the crop.
    const TORCH: Vec3 = Vec3::new(16.0, 16.0, 9.0);

    /// Ground the box shadows: the segment from here to [`TORCH`] enters the box
    /// across `x = 10` at a third of its height and leaves across `y = 12`.
    const SHADOWED: (u32, u32) = (8, 9);

    /// Ground it does not: the segment from here passes six units north of the
    /// box's own footprint.
    const LIT: (u32, u32) = (2, 20);

    /// And a pixel of the box's own lid.
    const ON_THE_BOX: (u32, u32) = (11, 11);

    fn pixel_at(image: &super::Image, (x, y): (u32, u32)) -> super::Pixel {
        image.pixels[(y * FRAME + x) as usize]
    }

    fn torch(at: Vec3, emitter: Emitter) -> Light {
        Light {
            at,
            emitter,
            falloff: Falloff::Windowed { reach: 60.0 },
            colour: [1.0; 3],
            intensity: 10.0,
        }
    }

    #[test]
    fn the_degenerate_mode_is_exact_and_a_soft_or_bounced_one_is_not() {
        // The flag is what a caller gates its "these must agree" comparison on,
        // so what makes it true has to be the emitter and the settings
        // together, not a constructor's promise.
        let (scene, camera) = (one_box_scene(), top_down());
        let point = [torch(TORCH, Emitter::Point)];
        let sphere = [torch(TORCH, Emitter::Sphere { radius: 1.0 })];
        let render_with = |lights: &[Light], settings: &Settings| {
            render(&scene, &camera, lights, settings, 8, 8).is_exact()
        };
        assert!(
            render_with(&point, &Settings::degenerate()),
            "a point, no bounces"
        );
        assert!(
            !render_with(&sphere, &Settings::degenerate()),
            "a sphere is an estimate"
        );
        assert!(
            !render_with(
                &point,
                &Settings {
                    bounces: 1,
                    ..Settings::degenerate()
                }
            ),
            "a bounce is an estimate whatever the emitter is"
        );
    }

    #[test]
    fn a_point_the_box_stands_in_front_of_is_shadowed_and_one_beside_it_is_not() {
        // The whole tracer, on a case worked out by hand rather than read off
        // its own output.
        let image = render(
            &one_box_scene(),
            &top_down(),
            &[torch(TORCH, Emitter::Point)],
            &Settings::degenerate(),
            FRAME,
            FRAME,
        );
        let shadowed = image.visibility(SHADOWED.0, SHADOWED.1, 0);
        assert_eq!(shadowed.reached, 0.0, "the box stands in the way");
        assert!(shadowed.within_reach, "and the torch does reach this far");
        assert_eq!(
            image.visibility(LIT.0, LIT.1, 0).reached,
            1.0,
            "nothing between it and the torch"
        );
        // And each pixel really is the surface the test thinks it is — a
        // shadowed pixel that turned out to be the box's own side would agree
        // with the assertion above for the wrong reason.
        assert_eq!(
            pixel_at(&image, SHADOWED).seen.map(|seen| seen.surface),
            Some(Surface::Ground)
        );
        assert_eq!(
            pixel_at(&image, LIT).seen.map(|seen| seen.surface),
            Some(Surface::Ground)
        );
        assert_eq!(
            pixel_at(&image, ON_THE_BOX).seen.map(|seen| seen.surface),
            Some(Surface::Body(0)),
            "and the box's own lid is where the box is"
        );
    }

    #[test]
    fn out_of_reach_is_told_apart_from_shadowed() {
        // Two dark pixels, opposite reasons. A comparison that collapses them
        // counts a torch's own radius as a shadow bug — which is exactly what
        // the renderer's own debug view spends a colour to avoid.
        let far = Light {
            falloff: Falloff::Windowed { reach: 4.0 },
            ..torch(TORCH, Emitter::Point)
        };
        let image = render(
            &one_box_scene(),
            &top_down(),
            &[far],
            &Settings::degenerate(),
            FRAME,
            FRAME,
        );
        let out = image.visibility(0, 0, 0);
        assert!(!out.within_reach, "the corner is nowhere near a four-unit torch");
        assert_eq!(out.reached, 0.0, "so nothing arrives");
    }

    #[test]
    fn a_surface_turned_away_from_the_light_is_not_reported_as_shadowed() {
        // The third way to be dark, and the one that is not about occlusion at
        // all. A torch below the floor leaves every ground pixel unlit because
        // of where the ground *points*, and a comparison that read that as a
        // shadow would find a disagreement with any renderer that skips the
        // cosine — on every pixel of the frame at once, which is a finding
        // about the two light models and not about anybody's geometry.
        let cellar = Light {
            at: Vec3::new(12.0, 12.0, -4.0),
            ..torch(TORCH, Emitter::Point)
        };
        let image = render(
            &one_box_scene(),
            &top_down(),
            &[cellar],
            &Settings::degenerate(),
            FRAME,
            FRAME,
        );
        let ground = image.visibility(LIT.0, LIT.1, 0);
        assert!(!ground.faces_light, "the ground's own normal points away from it");
        assert!(ground.within_reach, "and it is well inside the torch's reach");
        assert_eq!(ground.reached, 0.0, "so nothing arrives, for that reason");
    }

    #[test]
    fn a_wider_emitter_puts_partly_lit_pixels_where_a_point_had_a_hard_edge() {
        // The soft mode doing the one thing it exists to do. Counted rather
        // than eyeballed: with a point emitter every pixel is fully lit or
        // fully dark, and with a sphere there is a band that is neither.
        let scene = one_box_scene();
        let camera = top_down();
        let partly = |emitter, samples| {
            let image = render(
                &scene,
                &camera,
                &[torch(TORCH, emitter)],
                &Settings {
                    samples,
                    ..Settings::degenerate()
                },
                FRAME,
                FRAME,
            );
            (0..FRAME)
                .flat_map(|y| (0..FRAME).map(move |x| (x, y)))
                .filter(|(x, y)| {
                    let seen = image.visibility(*x, *y, 0).reached;
                    seen > 0.02 && seen < 0.98
                })
                .count()
        };
        assert_eq!(partly(Emitter::Point, 1), 0, "a point emitter has no penumbra");
        assert!(
            partly(Emitter::Sphere { radius: 2.0 }, 256) > 10,
            "a two-unit emitter should soften a shadow edge across several pixels"
        );
    }

    #[test]
    fn the_same_seed_renders_the_same_image_twice() {
        let (scene, camera) = (one_box_scene(), top_down());
        let lights = [torch(TORCH, Emitter::Sphere { radius: 1.5 })];
        let settings = Settings {
            samples: 16,
            bounces: 2,
            sky: [0.2; 3],
            seed: 4242,
        };
        let first = render(&scene, &camera, &lights, &settings, FRAME, FRAME);
        let again = render(&scene, &camera, &lights, &settings, FRAME, FRAME);
        assert_eq!(first, again, "a reference that moves between runs is not one");
    }

    #[test]
    fn indirect_light_reaches_where_direct_light_cannot() {
        // What the full mode is for, as a measurement rather than a look: with
        // no bounces the shadowed ground beside the box is exactly black, and
        // with a bounce it is not, because light comes off the lit ground and
        // off the box's own faces. Nothing in the renderer being checked can
        // produce this, which is the point of being able to see it.
        let (scene, camera) = (one_box_scene(), top_down());
        let lights = [torch(TORCH, Emitter::Point)];
        let shadowed_pixel = |bounces| {
            let image = render(
                &scene,
                &camera,
                &lights,
                &Settings {
                    samples: 512,
                    bounces,
                    sky: [0.0; 3],
                    seed: 7,
                },
                FRAME,
                FRAME,
            );
            pixel_at(&image, SHADOWED).radiance[0]
        };
        assert_eq!(shadowed_pixel(0), 0.0, "direct light alone leaves it black");
        assert!(
            shadowed_pixel(3) > 0.0,
            "and a bounce off the lit ground finds it"
        );
    }

    #[test]
    fn a_cosine_hemisphere_stays_in_its_hemisphere_and_leans_on_the_normal() {
        // The two ways this sampler fails silently: leaking below the surface
        // (a path that starts inside the geometry it just left) and being
        // uniform instead of cosine-weighted, which makes the throughput update
        // in `walk` wrong by a factor that looks like a slightly darker image.
        let mut stream = Stream::new(9, 9);
        let normal = Vec3::new(0.0, 0.0, 1.0);
        let draws = 40_000;
        let mut mean_cosine = 0.0;
        for _ in 0..draws {
            let direction = cosine_hemisphere(normal, &mut stream);
            assert!(
                (direction.length() - 1.0).abs() < 1e-9,
                "{direction:?} is not a unit vector"
            );
            let cosine = direction.dot(normal);
            assert!(cosine >= 0.0, "{direction:?} went below the surface");
            mean_cosine += cosine;
        }
        mean_cosine /= f64::from(draws);
        assert!(
            (mean_cosine - 2.0 / 3.0).abs() < 0.01,
            "a cosine-weighted hemisphere averages 2/3, got {mean_cosine}"
        );
    }
}
