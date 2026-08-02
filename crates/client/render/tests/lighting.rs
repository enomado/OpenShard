//! What the light does in a room, tile by tile.
//!
//! These are the questions a screenshot cannot answer: is that tile dark because
//! no flame reaches it, or because a wall stopped one that would? The scenes are
//! built rather than loaded (`render/src/scene.rs`), the numbers come from
//! `light::sample` — the shader's own arithmetic in Rust, held to it by
//! `frame.rs`'s parity test — and a failure prints the room.
//!
//! No GPU and no client files: everything here runs everywhere, which is the
//! point of a scene that is a `Map` with three items on it.

use openshard_client_render::debug;
use openshard_client_render::geometry::Vec2;
use openshard_client_render::light::{self, Lighting, Spot};
use openshard_client_render::occlusion;
use openshard_client_render::scene::{self, CENTRE, DOORWAY, Scene};

/// The flicker's instant. Always zero: a flame's brightness swings by a tenth,
/// and an assertion about a leak must not depend on which tenth of a second it
/// was asked in.
const STILL: f32 = 0.0;

/// The ambient alone at one tile, as one number — what that tile comes out at
/// with no flame reaching it.
///
/// Per tile and no longer per frame, which is `docs/lighting_world.md`'s
/// decision 1 arriving in the tests: a tile under a roof, a wall tile whose own
/// column is shaded, and the tile beside it that the blur touched all get
/// different shares of the sky. A single constant would now be the right answer
/// only in the open, and every "this tile is the ambient exactly" assertion
/// below would be measuring the field instead of the leak it is about.
fn ambient(lighting: &Lighting, tile: (u16, u16)) -> f32 {
    let sky = lighting.occlusion.sky_at(i32::from(tile.0), i32::from(tile.1));
    let lit = lighting.ambient.at(sky);
    lit.iter().sum::<f32>() / lit.len() as f32
}

/// How bright the middle of a tile is, at a height.
fn at(lighting: &Lighting, tile: (u16, u16), z: f32) -> f32 {
    light::sample(spot(tile, z), lighting).brightness()
}

/// The middle of a tile, at a height.
fn spot(tile: (u16, u16), z: f32) -> Spot {
    Spot {
        at: Vec2::new(f32::from(tile.0) + 0.5, f32::from(tile.1) + 0.5),
        z,
    }
}

/// The room, drawn, for the message a failing assertion carries.
fn picture(scene: &Scene, lighting: &Lighting) -> String {
    format!(
        "\n{}:\n{}",
        scene.name,
        debug::diagram(lighting, debug::around(CENTRE, 6), 0.0)
    )
}

/// A torch in a shut room lights the room and nothing outside it.
///
/// The claim the whole pass was built for. Both halves matter and they fail
/// separately: an inside that is not lit means the flame was never collected,
/// an outside that is means the ring of wall has a hole in it — and until there
/// was a diagram, those two looked the same from a screenshot.
#[test]
fn a_shut_room_keeps_its_light_inside() {
    let scene = scene::room();
    let lighting = scene.lighting(STILL);
    let picture = picture(&scene, &lighting);

    let lit_tile = (CENTRE.0 + 1, CENTRE.1);
    let inside = at(&lighting, lit_tile, 0.0);
    assert!(
        inside > ambient(&lighting, lit_tile) + 0.2,
        "the room is not lit: {inside}{picture}"
    );

    // Every tile outside the ring, on all four sides, is the ambient exactly —
    // not merely dimmer. "Stops" is a different claim from "falls off", and a
    // radius that happened to be short would pass the weaker one.
    for tile in [
        (CENTRE.0 + scene::ROOM_HALF + 1, CENTRE.1),
        (CENTRE.0 - scene::ROOM_HALF - 1, CENTRE.1),
        (CENTRE.0, CENTRE.1 + scene::ROOM_HALF + 1),
        (CENTRE.0, CENTRE.1 - scene::ROOM_HALF - 1),
    ] {
        let outside = at(&lighting, tile, 0.0);
        assert!(
            (outside - ambient(&lighting, tile)).abs() < 1e-6,
            "light leaks out at {tile:?}: {outside} against the ambient's {}{picture}",
            ambient(&lighting, tile),
        );
    }
}

/// The edge of a shadow is a gradient, not a step at a tile boundary.
///
/// A fully opaque wall is what makes this an oracle rather than an impression:
/// while a cell was all-or-nothing, the only two answers a ray could come back
/// with were `1.0` and `0.0`, whatever the fraction of the tile a fragment was
/// at — so a sweep across the edge of the spill from an open door stepped from
/// lit to black between two neighbouring samples, and every shadow in the frame
/// had a tile's straight side. What the walk spends now is the *length* of its
/// crossing, so a ray clipping the doorpost's corner keeps most of its light and
/// the sweep passes through the values in between.
///
/// Across the spill and not along it, at a tenth of a tile: the sweep is over
/// what a wall did to the ray, so it reads `Reach::through` — the shadow term
/// alone — rather than the brightness, which falls off with distance and would
/// show a gradient even if every ray were binary.
#[test]
fn the_edge_of_a_shadow_passes_through_the_values_in_between() {
    let scene = scene::room_with_open_door();
    let lighting = scene.lighting(STILL);
    let picture = picture(&scene, &lighting);

    // A line across the fan of light on the ground two tiles south of the
    // doorway, from well inside the spill to well inside the shadow of the
    // wall beside it.
    let y = f32::from(DOORWAY.1) + 1.5;
    let sweep: Vec<(f32, f32)> = (-100..=100)
        .map(|step| f32::from(DOORWAY.0) + 0.5 + step as f32 / 100.0)
        .map(|x| {
            let spot = Spot {
                at: Vec2::new(x, y),
                z: 0.0,
            };
            let through = light::sample(spot, &lighting)
                .reaches
                .iter()
                .find(|reach| reach.within)
                .map_or(0.0, |reach| reach.through);
            (x, through)
        })
        .collect();
    let partial = sweep
        .iter()
        .filter(|(_, through)| *through > 0.02 && *through < 0.98)
        .count();

    assert!(
        partial >= 4,
        "the spill's edge steps straight from lit to black: {partial} samples in between\n{sweep:?}{picture}",
    );
}

/// And the report says *which* wall stopped it.
///
/// The half of observability that a picture cannot carry: a tile is dark, and
/// this is the cell that made it dark. If the shadow ever comes from a tile
/// nobody built, that is the assertion that says so.
#[test]
fn the_report_names_the_cell_that_stopped_the_ray() {
    let scene = scene::room();
    let lighting = scene.lighting(STILL);
    let east = (CENTRE.0 + scene::ROOM_HALF + 1, CENTRE.1);
    let sample = light::sample(spot(east, 0.0), &lighting);

    let reach = sample
        .reaches
        .iter()
        .find(|reach| reach.within)
        .unwrap_or_else(|| panic!("the torch does not even reach the tile:\n{sample}"));
    assert_eq!(
        reach.stopped_by,
        Some((i32::from(CENTRE.0 + scene::ROOM_HALF), i32::from(CENTRE.1))),
        "stopped somewhere other than the east wall:\n{sample}",
    );
}

/// A shut door is a wall; an open one is a hole, and the light goes through it.
///
/// The two scenes differ in exactly one graphic — nothing in the lighting knows
/// what a door is, and this is what says the mechanism is the tiledata flag and
/// not a special case. See `docs/lighting.md`, decision 11.
#[test]
fn opening_a_door_spills_light_onto_the_ground_outside() {
    let shut = scene::room_with_shut_door();
    let open = scene::room_with_open_door();
    let (shut_light, open_light) = (shut.lighting(STILL), open.lighting(STILL));

    // Straight out of the doorway, one tile past the wall. Asserted through the
    // *reason* rather than through a brightness threshold: at four tiles of a
    // torch's six the pool has fallen to a twentieth, so any number chosen here
    // would be a number about the falloff, and what this test is about is
    // whether the ray got through at all.
    let outside = (DOORWAY.0, DOORWAY.1 + 1);
    let shut_reach = light::sample(spot(outside, 0.0), &shut_light).reaches[0];
    let open_reach = light::sample(spot(outside, 0.0), &open_light).reaches[0];
    assert_eq!(
        shut_reach.stopped_by,
        Some((i32::from(DOORWAY.0), i32::from(DOORWAY.1))),
        "a shut door is not a wall{}",
        picture(&shut, &shut_light),
    );
    assert_eq!(
        (open_reach.stopped_by, open_reach.through),
        (None, 1.0),
        "the open door still stops light{}",
        picture(&open, &open_light),
    );
    let (closed, opened) = (at(&shut_light, outside, 0.0), at(&open_light, outside, 0.0));
    assert!(
        (closed - ambient(&shut_light, outside)).abs() < 1e-6 && opened > closed,
        "the ground outside the doorway: {opened} open against {closed} shut{}",
        picture(&open, &open_light),
    );

    // And it is a fan through the opening rather than a glow around the house:
    // the tile diagonally out from the doorway is behind the wall beside it, and
    // stays dark. Without this the test would pass for a door that stopped
    // occluding the whole ring.
    let beside = (DOORWAY.0 + 2, DOORWAY.1 + 1);
    let spill = at(&open_light, beside, 0.0);
    assert!(
        (spill - ambient(&open_light, beside)).abs() < 1e-6,
        "the open door lit the ground behind the wall beside it: {spill}{}",
        picture(&open, &open_light),
    );
}

/// A pane of glass dims light; a wall stops it.
///
/// `WINDOW` sits beside `NO_SHOOT` in the reference's line of sight
/// (`Map.LineOfSight`, `Server/Map.cs:3040`) — and that is a fact about arrows.
/// A window that stopped light makes a lit room read as a bunker and hides the
/// one thing a candle is for at night, so the pane keeps four fifths of what
/// crosses it. The three cases are asserted together because what matters is
/// that they are *ordered*: through the wall, through the glass, and through the
/// doorway are three different numbers and always in that order.
#[test]
fn a_pane_of_glass_dims_light_where_a_wall_stops_it() {
    let outside = (DOORWAY.0, DOORWAY.1 + 1);
    let walled = scene::room();
    let glazed = scene::room_with_window();
    let opened = scene::room_with_open_door();
    let (walled_light, glazed_light, open_light) = (
        walled.lighting(STILL),
        glazed.lighting(STILL),
        opened.lighting(STILL),
    );

    // What the *flame* added, and not what the tile came out at: the three scenes
    // differ by one graphic, and a graphic that shades its column differently
    // gives the same tile a different share of the sky in each of them. Comparing
    // the totals would be comparing the ambient as much as the light through the
    // opening, which is the question this test is not asking.
    let added = |lighting: &Lighting| at(lighting, outside, 0.0) - ambient(lighting, outside);
    let (wall, glass, doorway) = (added(&walled_light), added(&glazed_light), added(&open_light));
    assert!(
        wall.abs() < 1e-6,
        "the wall no longer stops light: {wall} arrives through it{}",
        picture(&walled, &walled_light),
    );
    assert!(
        wall < glass && glass < doorway,
        "the glass is not between the wall and the open door: \
         {wall} walled, {glass} glazed, {doorway} open{}",
        picture(&glazed, &glazed_light),
    );

    // And by about the fraction `occlusion::PANE` states, rather than by
    // whatever a threshold would tolerate: the light through the pane is what
    // the open doorway passes, less a fifth.
    let want = doorway * 0.8;
    assert!(
        (glass - want).abs() < 1e-3,
        "the pane passes {glass}, not the {want} its opacity says{}",
        picture(&glazed, &glazed_light),
    );
}

/// The wall of a lit room is lit on the inside, at every height up its face.
///
/// The bug the world-coordinate pass was written against: in screen space the
/// wall's own sprite stands over the ground it shadows, so the face turned
/// towards the flame was the darkest thing in the picture. Here the wall's
/// pixels carry the wall's tile, and its own tile never shadows it.
#[test]
fn the_face_of_a_wall_is_lit_from_inside_the_room() {
    let scene = scene::room();
    let lighting = scene.lighting(STILL);
    let wall = (CENTRE.0 + scene::ROOM_HALF, CENTRE.1);
    // The foot of the wall, halfway up it, and the top: a sprite's pixels differ
    // in `z` and nothing else, and all three are inside the pool.
    for z in [
        0.0,
        f32::from(scene::WALL_HEIGHT) / 2.0,
        f32::from(scene::WALL_HEIGHT),
    ] {
        let lit = at(&lighting, wall, z);
        assert!(
            lit > ambient(&lighting, wall) + 0.1,
            "the wall is dark at z {z}: {lit}{}",
            picture(&scene, &lighting),
        );
    }
}

/// A sconce lights through the wall it is mounted on. **This is wrong, and it is
/// pinned so that fixing it is visible.**
///
/// A light's own tile is exempted from occluding it — decision 3 — because a
/// torch standing in a doorway must not shadow itself. For a sconce that
/// exemption lights the street outside as brightly as the room inside, and the
/// fix wants the wall's *facing*, which is nowhere in `tiledata.mul`. Until
/// there is one, this test states the behaviour rather than pretending it is not
/// there; it fails the day a facing arrives, and that failure is the point.
#[test]
fn a_sconce_lights_through_its_own_wall() {
    let scene = scene::sconce_on_wall();
    let lighting = scene.lighting(STILL);
    let (in_front, behind) = ((CENTRE.0, CENTRE.1 - 1), (CENTRE.0, CENTRE.1 + 1));
    let north = at(&lighting, in_front, 0.0);
    let south = at(&lighting, behind, 0.0);
    assert!(
        (north - south).abs() < 1e-6,
        "a facing has appeared — update this test and decision 3: {north} against {south}{}",
        picture(&scene, &lighting),
    );
    assert!(
        north > ambient(&lighting, in_front) + 0.2,
        "the sconce lights nothing at all: {north}"
    );
}

/// A light carried in a hand lights what the character is facing far more
/// brightly than what is behind them.
///
/// The whole claim of a beam, and the reason it is not simply a torch on the
/// player's tile: an omnidirectional pool centred on a body lights the wall
/// behind it exactly as brightly as the one it is walking towards, which reads as
/// the character glowing rather than as the character carrying something.
///
/// Stated as a ratio and not as "behind is the ambient exactly", because a hand
/// is not a shutter: `light::BEAM_SPILL` of the flame goes every way, so the
/// character and the ground at their feet are lit. What the cone has to buy is
/// the *difference*, and that is what is measured.
#[test]
fn a_carried_light_lights_the_way_it_is_pointed() {
    let scene = scene::lantern_in_a_room();
    let lighting = scene.lighting(STILL);
    let picture = picture(&scene, &lighting);

    // East is the facing, so `+x` is ahead. Two tiles out on either side: the
    // same distance from the flame, so the falloff is the same for both and the
    // only difference between them is the direction.
    let (ahead, behind) = ((CENTRE.0 + 2, CENTRE.1), (CENTRE.0 - 2, CENTRE.1));
    let lit = at(&lighting, ahead, 0.0) - ambient(&lighting, ahead);
    let dark = at(&lighting, behind, 0.0) - ambient(&lighting, behind);
    assert!(
        lit > 0.2,
        "the beam lights nothing ahead of it: {lit} over the ambient{picture}",
    );
    assert!(
        dark > 0.0,
        "nothing spills out of the beam, so the character is in a black hole: \
         {dark}{picture}",
    );
    assert!(
        lit > dark * 3.0,
        "the beam is not pointed anywhere: {lit} ahead against {dark} behind{picture}",
    );

    // And the same for the two walls the room has on those sides, up their whole
    // height: a beam that lit the floor and left every wall in the frame at the
    // ambient would look like a decal on the ground. The east face is what says
    // the light has hit something.
    let (front_wall, back_wall) = (
        (CENTRE.0 + scene::ROOM_HALF, CENTRE.1),
        (CENTRE.0 - scene::ROOM_HALF, CENTRE.1),
    );
    for z in [0.0, f32::from(scene::WALL_HEIGHT) / 2.0] {
        let face = at(&lighting, front_wall, z) - ambient(&lighting, front_wall);
        let back = at(&lighting, back_wall, z) - ambient(&lighting, back_wall);
        assert!(
            face > 0.1,
            "the wall the beam points at is dark at z {z}: {face}{picture}",
        );
        assert!(
            face > back * 3.0,
            "both walls are lit the same at z {z}: {face} against {back}{picture}",
        );
    }
}

/// The beam's own edge is a gradient, and it is sixty degrees wide.
///
/// Two claims one sweep answers. A cone with a hard rim reads as a stencil laid
/// over the scene — the same complaint the tile-edged shadows drew — so the
/// values between the two ends have to exist; and the *width* is the number
/// somebody will change by accident, so it is measured rather than trusted: at
/// four tiles out, a sixty-degree beam's rim is `4 * tan(30°)` ≈ 2.3 tiles off
/// the axis, and a spot three tiles across is outside it by any softening.
#[test]
fn the_edge_of_a_beam_is_a_gradient_of_the_stated_width() {
    let scene = scene::lantern_in_a_room();
    let lighting = scene.lighting(STILL);
    let picture = picture(&scene, &lighting);
    let carried = lighting.lights[0];
    let beam = carried.beam.expect("the scene's character carries a beam");

    // Straight along the axis and then off it, in tenths of a tile, four tiles
    // out — the distance is fixed so that only the angle changes.
    let mut values = Vec::new();
    for step in 0..=50 {
        let across = step as f32 / 10.0;
        values.push(beam.lights([4.0, across, 0.0]));
    }
    let spill = light::BEAM_SPILL;
    let partial = values.iter().filter(|v| **v > spill + 0.01 && **v < 0.99).count();
    assert!(
        partial >= 3,
        "the beam's rim is a step and not a gradient: {values:?}{picture}",
    );
    assert!(
        values[0] > 0.99,
        "the beam's own axis is not fully lit: {}{picture}",
        values[0],
    );
    // `4 * tan(30°)` is 2.31 tiles: inside two, outside three, whatever the
    // softening does in between.
    assert!(
        values[20] > spill,
        "the beam is narrower than the sixty degrees it says: {}{picture}",
        values[20],
    );
    assert_eq!(
        values[30], spill,
        "the beam is wider than the sixty degrees it says{picture}",
    );
}

/// A torch in a cellar does not light the street above it, with nothing in
/// between.
///
/// Distance alone, in three dimensions: `z` divided into tiles at eleven units
/// each. There is no floor in this scene on purpose — a test that also had one
/// would pass even if the height were being ignored entirely.
#[test]
fn a_cellar_does_not_light_the_street_above_it() {
    let scene = scene::cellar_under_street();
    let lighting = scene.lighting(STILL);
    let street = at(&lighting, CENTRE, 0.0);
    assert!(
        (street - ambient(&lighting, CENTRE)).abs() < 1e-6,
        "the cellar lights the street: {street}{}",
        picture(&scene, &lighting),
    );
    // And the flame is real: it lights its own floor. Without this the test
    // above would pass for a scene where the torch was never collected.
    let cellar = at(&lighting, CENTRE, f32::from(scene::CELLAR_DEPTH));
    assert!(
        cellar > ambient(&lighting, CENTRE) + 0.2,
        "the cellar itself is dark: {cellar}"
    );
}

/// A ray slips between two walls that touch only at their corners. **Also
/// wrong, also pinned.**
///
/// The walk is Chebyshev-sampled, one cell a step, so a ray running along a
/// diagonal passes through the corner where two wall tiles meet. Real walls are
/// rows and this has not been seen in a house; the backlog's fix is a supercover
/// walk that visits both cells of every crossing, at about twice the samples.
/// This is what will say the fix worked.
#[test]
fn a_ray_slips_between_two_walls_that_touch_at_a_corner() {
    let scene = scene::diagonal_gap();
    let lighting = scene.lighting(STILL);
    let behind = at(&lighting, CENTRE, 0.0);
    assert!(
        behind > ambient(&lighting, CENTRE) + 0.1,
        "the diagonal gap has been closed — update this test: {behind}{}",
        picture(&scene, &lighting),
    );
}

/// A wall throws a shadow across the ground away from the sun, and the ground
/// on the sun's side stays lit.
///
/// The first half of decision 12. Both sides are asserted because only one of
/// them fails on its own: a walk that stepped the wrong way would darken the
/// wrong tile and a test that only looked at the dark one would be green for it.
#[test]
fn a_wall_throws_its_shadow_away_from_the_sun() {
    let scene = scene::wall_in_the_sun();
    let lighting = scene.lighting(STILL);
    let sun = scene.sun.expect("the scene has a sun");

    // The sun is towards +x, so the shadow lies at lower `x`.
    let shaded = (CENTRE.0 - 1, CENTRE.1);
    let sunlit = (CENTRE.0 + 1, CENTRE.1);
    let dark = light::sample(spot(shaded, 0.0), &lighting);
    let bright = light::sample(spot(sunlit, 0.0), &lighting);

    assert_eq!(
        dark.sun.expect("a sunlit frame").stopped_by,
        Some((i32::from(CENTRE.0), i32::from(CENTRE.1))),
        "the tile away from the sun is not shadowed by the wall:\n{dark}{}",
        picture(&scene, &lighting),
    );
    assert_eq!(
        bright.sun.expect("a sunlit frame").through,
        1.0,
        "the tile towards the sun is shadowed:\n{bright}{}",
        picture(&scene, &lighting),
    );
    assert!(
        bright.brightness() > dark.brightness(),
        "the shadow is not darker than the ground beside it{}",
        picture(&scene, &lighting),
    );

    // And the shadow is as long as the wall is tall, at 45°: twenty units of
    // height is under two tiles, so the second tile out is clear and the third
    // certainly is. The number is the *geometry*, which is what a slope buys —
    // a shadow that ignored the wall's height would run to the edge of the grid.
    assert_eq!(sun.rise_per_tile(), 1.0, "the scene's sun is no longer at 45°");
    let past = light::sample(spot((CENTRE.0 - 3, CENTRE.1), 0.0), &lighting);
    assert_eq!(
        past.sun.expect("a sunlit frame").through,
        1.0,
        "the shadow runs further than the wall is tall:\n{past}{}",
        picture(&scene, &lighting),
    );
}

/// The sun comes through a window and not through the wall beside it.
///
/// The picture decision 12 exists for: a room whose floor is in shadow, with a
/// brighter band behind the pane. Asserted as an ordering rather than as a
/// level, because how bright the band is depends on `occlusion::PANE` and on the
/// sun's intensity, and neither of those is what this is about.
#[test]
fn the_sun_reaches_the_floor_through_a_window() {
    let scene = scene::sunlit_room_with_window();
    let lighting = scene.lighting(STILL);

    // The floor tile just inside the pane, and one three tiles further in. The
    // sun travels towards +x, so a ray leaving the first goes out through the
    // window and a ray leaving the second meets the roof.
    let behind_pane = (scene::WINDOW_TILE.0 - 1, scene::WINDOW_TILE.1);
    let behind_wall = (scene::WINDOW_TILE.0 - 3, scene::WINDOW_TILE.1);
    let lit = light::sample(spot(behind_pane, 0.0), &lighting);
    let dark = light::sample(spot(behind_wall, 0.0), &lighting);
    let sun_of = |sample: &light::Sample| sample.sun.expect("a sunlit frame");

    assert!(
        sun_of(&lit).through > sun_of(&dark).through,
        "the pane passes no more sun than the wall does:\n{lit}\n{dark}{}",
        picture(&scene, &lighting),
    );
    assert!(
        sun_of(&lit).through > 0.0 && sun_of(&dark).through == 0.0,
        "the window is not a window, or the wall is not a wall:\n{lit}\n{dark}{}",
        picture(&scene, &lighting),
    );
}

/// A frame with no sun pays nothing and says so.
///
/// `None` and `0.0` are different answers — "there is no sky" against "the sky
/// is dark here" — and the report has to keep them apart, because the first is
/// where a person looking for a missing sunbeam should stop looking.
#[test]
fn a_frame_without_a_sun_reports_none_rather_than_nothing() {
    let scene = scene::room();
    let lighting = scene.lighting(STILL);
    assert!(light::sample(spot(CENTRE, 0.0), &lighting).sun.is_none());
}

/// How much of the sky a tile of a scene can see.
fn sky(scene: &Scene, tile: (u16, u16)) -> u8 {
    scene
        .lighting(STILL)
        .occlusion
        .sky_at(i32::from(tile.0), i32::from(tile.1))
}

/// A tile well outside the house, for the "and the street is open" half of
/// everything below.
const STREET: (u16, u16) = (CENTRE.0, CENTRE.1 + scene::ROOM_HALF + 3);

/// A room under a roof does not get the sky's light, and the street outside it
/// does.
///
/// `docs/lighting_world.md`, decision 1, and it is the largest visible change
/// this plan makes: today a room is lit exactly as brightly as the road, because
/// the ambient is one colour for the whole frame. Nothing about a flame is in
/// this test — the field is what a *place* has before anything burns in it.
#[test]
fn a_roof_takes_the_sky_from_the_room_under_it() {
    let house = scene::roofed_room();
    let street = sky(&house, STREET);
    let room = sky(&house, CENTRE);
    assert_eq!(street, occlusion::SKY_OPEN, "the street is not open sky");
    assert_eq!(room, 0, "the middle of a roofed room still sees the sky");
}

/// And the room is *darker* for it, before anything burns in it.
///
/// The other half of decision 1, and the half a field alone does not give: the
/// sky byte is only a number until an ambient is split in two and scaled by it.
/// What this asserts is the whole visible change of step 2 — a room under a roof
/// is deep, the street outside is not, and neither is black.
///
/// Held as an ordering and a floor rather than as levels: how dark the room is
/// is `light::GROUND_AMBIENT`, which is a number tuned against a picture, and a
/// test that pinned it would fail every time somebody looked at the picture.
#[test]
fn a_roof_makes_the_room_under_it_darker_than_the_street() {
    let house = scene::roofed_room();
    let lighting = house.lighting(STILL);

    let room = ambient(&lighting, CENTRE);
    let street = ambient(&lighting, STREET);
    assert!(
        street > room + 0.1,
        "the room is lit as brightly as the road outside it: {room} against {street}",
    );
    // And not a hole: an unlit black rectangle is not atmosphere, it is a bug
    // report — which is the whole of what the ground term is for.
    // The scene is at noon, so it is lit by `light::SKYLIGHT` — and a tile with
    // no sky at all gets that ambient's ground term and nothing else.
    let floor: f32 = light::GROUND_AMBIENT.iter().sum::<f32>() / 3.0;
    assert!(
        (room - floor).abs() < 1e-6,
        "the roofed room is not the ground ambient exactly: {room} against {floor}",
    );

    // And the same through the whole formula rather than through its first term:
    // what a person actually sees is the ambient plus whatever reaches the tile,
    // and nothing here would show up in a frame if the sun happened to make the
    // difference up.
    let (room, street) = (at(&lighting, CENTRE, 0.0), at(&lighting, STREET, 0.0));
    assert!(
        street > room + 0.1,
        "lit, the room is as bright as the road: {room} against {street}{}",
        picture(&house, &lighting),
    );
}

/// The threshold of an open door is brighter than the room and darker than the
/// street.
///
/// Decision 2: the blur is what makes a doorway a gradient. Without it the field
/// steps from 1 to 0 at the wall line, which is the artefact the whole track
/// exists to remove — and the two scenes differ by exactly one graphic, so a
/// difference between them is the door and nothing else.
#[test]
fn a_doorway_is_a_threshold_and_not_a_step() {
    let open = scene::roofed_room_with_open_door();
    let shut = scene::roofed_room();

    let threshold = sky(&open, DOORWAY);
    assert!(
        threshold > 0 && threshold < occlusion::SKY_OPEN,
        "the doorway of {} reads {threshold}, which is the room or the street",
        open.name,
    );
    assert!(
        threshold > sky(&shut, DOORWAY),
        "an open door is worth no more sky than a shut one",
    );
    assert!(
        threshold < sky(&open, STREET),
        "the doorway is as bright as the road outside it",
    );
}

/// A glazed wall is worth some of the sky, and a solid one is worth none.
///
/// The crude half of decision 14, and the whole of what stands in for
/// `docs/lighting.md`'s step 16 until it lands: a pane passes its share in the
/// column, and the blur is what carries it inwards. What is asserted is the
/// ordering and not the level — how much a pane passes is `occlusion::PANE`,
/// which is a guess about glass and not a number from any file.
#[test]
fn a_window_is_worth_more_sky_than_the_wall_it_replaces() {
    let glazed = scene::roofed_room_with_window();
    let solid = scene::roofed_room();
    let inside = (scene::WINDOW_TILE.0 - 1, scene::WINDOW_TILE.1);

    assert!(
        sky(&glazed, scene::WINDOW_TILE) > sky(&solid, scene::WINDOW_TILE),
        "the pane itself is as dark as a wall",
    );
    assert!(
        sky(&glazed, inside) > sky(&solid, inside),
        "the room behind the window is no lighter than a cellar",
    );
    assert_eq!(sky(&solid, inside), 0, "and the windowless room is a cellar");
}

/// Every scene draws a diagram with something in it.
///
/// A weak assertion on purpose: what it guards is the failure mode of a
/// diagnostic, which is being silently empty. A diagram of nothing but spaces
/// would still make every message above *look* informative, and that is worse
/// than no diagram at all.
#[test]
fn every_scene_prints_a_diagram_that_is_not_blank() {
    for scene in scene::all() {
        let lighting = scene.lighting(STILL);
        let drawn = debug::diagram(&lighting, debug::around(CENTRE, 6), 0.0);
        // A flame, an occluder, or lit ground: every scene has at least one of
        // the three, and a sunlit one has no flame at all.
        assert!(
            drawn.contains('*') || drawn.contains('#'),
            "nothing stands in the diagram of {}:\n{drawn}",
            scene.name,
        );
        assert!(
            drawn.lines().count() > 12,
            "the diagram of {} is too small to read:\n{drawn}",
            scene.name,
        );
    }
}
