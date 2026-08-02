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
use openshard_client_render::scene::{self, CENTRE, DOORWAY, Scene};

/// The flicker's instant. Always zero: a flame's brightness swings by a tenth,
/// and an assertion about a leak must not depend on which tenth of a second it
/// was asked in.
const STILL: f32 = 0.0;

/// The ambient alone, as one number — what an unlit tile comes out at.
fn ambient() -> f32 {
    light::NIGHT.iter().sum::<f32>() / light::NIGHT.len() as f32
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

    let inside = at(&lighting, (CENTRE.0 + 1, CENTRE.1), 0.0);
    assert!(inside > ambient() + 0.2, "the room is not lit: {inside}{picture}");

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
            (outside - ambient()).abs() < 1e-6,
            "light leaks out at {tile:?}: {outside} against the ambient's {}{picture}",
            ambient(),
        );
    }
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
        (closed - ambient()).abs() < 1e-6 && opened > closed,
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
        (spill - ambient()).abs() < 1e-6,
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

    let wall = at(&walled_light, outside, 0.0);
    let glass = at(&glazed_light, outside, 0.0);
    let doorway = at(&open_light, outside, 0.0);
    assert!(
        (wall - ambient()).abs() < 1e-6,
        "the wall no longer stops light: {wall}{}",
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
    let want = ambient() + (doorway - ambient()) * 0.8;
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
            lit > ambient() + 0.1,
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
    let north = at(&lighting, (CENTRE.0, CENTRE.1 - 1), 0.0);
    let south = at(&lighting, (CENTRE.0, CENTRE.1 + 1), 0.0);
    assert!(
        (north - south).abs() < 1e-6,
        "a facing has appeared — update this test and decision 3: {north} against {south}{}",
        picture(&scene, &lighting),
    );
    assert!(
        north > ambient() + 0.2,
        "the sconce lights nothing at all: {north}"
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
        (street - ambient()).abs() < 1e-6,
        "the cellar lights the street: {street}{}",
        picture(&scene, &lighting),
    );
    // And the flame is real: it lights its own floor. Without this the test
    // above would pass for a scene where the torch was never collected.
    let cellar = at(&lighting, CENTRE, f32::from(scene::CELLAR_DEPTH));
    assert!(cellar > ambient() + 0.2, "the cellar itself is dark: {cellar}");
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
        behind > ambient() + 0.1,
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
