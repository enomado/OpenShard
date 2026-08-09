//! Contiguous pieces of one surface become one primitive —
//! `docs/occluders.md`'s D2b, and the last step of that plan.
//!
//! # What it is for, and what it is not
//!
//! A run of wall is one wall to the artist and N statics on N tiles to us; a
//! storey's floor is one slab and one box a tile. Every internal seam is a place
//! where two boxes meet, and `docs/occluders.md`'s own history is a list of rules
//! written to paper over that. This is the step that removes the seams instead of
//! ruling about them.
//!
//! **It is a pure optimisation and nothing else**, which is the plan's own
//! correction to its first draft: what cured the seam a person sees was
//! `docs/lighting_rebuild.md`'s phase 5b, and the gate here is therefore *not one
//! pixel moves*. So every condition below is an exact comparison and every doubt
//! is resolved by **refusing** to merge — a primitive that stays two primitives
//! costs a slab test, and one that merges when it should not costs a wrong
//! picture.
//!
//! # When two primitives merge
//!
//! Exactly when they **share a whole face** and are identical in every other
//! field. Sharing a whole face is the geometric half: the two boxes touch on one
//! axis (`a.max[axis] == b.min[axis]`, exact) and have equal extents on the other
//! two, which is what makes the componentwise union of them *exactly* their union
//! as point sets — no volume is invented and none is lost.
//!
//! The plan states the other half as "equal opacity, equal `edges` classification
//! and equal span". Three fields it does not name are required here as well, and
//! each is a thing the merge would otherwise break rather than a caution:
//!
//! - **The [`Owner`](super::Owner) and the [`Part`](super::Part).** They are the
//!   join from a drawn instance to the primitive it is met against —
//!   [`Occlusion::id_of`](super::Occlusion::id_of), which D6 is built on. A
//!   merged primitive carries one of each, so merging two pieces that disagree
//!   about either would leave the other instance unable to name its own solid,
//!   and the impostor would meet its pixels against nothing. With them equal the
//!   join is preserved **by construction**: both cells scan for the same pair and
//!   both find the merged primitive.
//!
//!   It is also less of a restriction than it looks. An `Owner` is a `(z,
//!   graphic)` and carries no tile, so a run of one wall graphic at one height is
//!   one owner however many tiles it stands on — which is exactly the run this
//!   step exists for. What it refuses is a run assembled out of *two* graphics,
//!   and that is a real refusal, written down in `docs/occluders.md` rather than
//!   hidden here.
//!
//! - **An [`Aperture`](super::Aperture), which stops a merge outright.** A hole
//!   is measured along its panel's run as a fraction of **one tile** —
//!   `light::run_v` is `along - along.floor()` — so it is the last rule in the
//!   pass that is still stated in a tile. Merge two windowed panels and the hole
//!   appears in every tile of the run. Refusing is exact; carrying the hole would
//!   need the aperture stated in the primitive's own coordinates first.
//!
//! - **Opacity is compared, and being *equal* is not enough for a translucent
//!   one.** Two panes crossed by one ray are dimmed twice —
//!   `lighting.rs`'s `a_segment_through_two_panes_on_one_tile_is_dimmed_by_both_of_them`
//!   is the fixture, and S5 settled that the product is the walk's rule — while
//!   one merged pane dims once. That is a moved pixel, so anything short of
//!   [`OPAQUE`](super::OPAQUE) is left alone. For an opaque primitive the two are
//!   the same number and the merge is provably invisible.
//!
//! # Why only the horizontal axes
//!
//! A merge along `z` would join two boxes of **different spans**, which the plan
//! forbids in the same sentence that says a flight's treads stay three
//! primitives. It also cannot fire: the fields above require an equal `Owner`,
//! half of which is the `z` the static stands at, so two primitives stacked in
//! `z` are two owners by construction. A rule that cannot fire is a rule not
//! worth writing.
//!
//! # Why the answer cannot change
//!
//! `ray_vs_solid` asks whether a primitive meets the **open** segment —
//! `enter < 1 && leave > 0`. For two boxes sharing a face the union's interval on
//! any ray is the union of the two intervals, and a union of two contiguous
//! intervals meets the open segment exactly when one of them does. So for opaque
//! primitives the walk's per-primitive product is `1 - 0` either way, and the
//! only thing that changes is how many slab tests it took to get there.
//!
//! The one thing that *does* change is identity: a fragment of one piece is a
//! point of the merged primitive, so it is exempt from the volume its neighbour
//! used to be. That is D6 arriving, it is the seam this plan was written for, and
//! after phase 5b there is measurably no ray left that reaches it — which is what
//! the gate reports rather than what this module claims.

use crate::occlusion::{Solid, SolidId};

/// The frame's primitives with every contiguous piece of one surface folded into
/// one, and — for each primitive that went in, in order — the name of the
/// primitive it is now part of.
///
/// That second list is exactly [`Occlusion::ids`](super::Occlusion): a cell's
/// references are unchanged in number and in order, and what they point at is
/// the merged primitive. Two cells naming one primitive is the first thing in
/// this crate to do so, which is what the reference level was built for.
///
/// A group is named by its **lowest** member, and the output is in the order
/// those names first appear — so the list a frame uploads does not depend on
/// which axis merged what, and a build with nothing to merge hands back exactly
/// what it was given.
pub fn merged(solids: Vec<Solid>) -> (Vec<Solid>, Vec<SolidId>) {
    // Union-find over the primitives: `parent[i] == i` is a group's own name,
    // and a group's name is always the lowest index in it.
    let mut parent: Vec<u32> = (0..solids.len() as u32).collect();
    // The box each group covers, valid at the group's name and stale elsewhere.
    let mut space: Vec<crate::solid::Solid> = solids.iter().map(|solid| solid.space).collect();

    // Both axes, until neither finds anything: a floor merges into rows along one
    // axis and those rows into a slab along the other, and a row that has just
    // grown is a fresh candidate for the axis that ran before it. Each pass
    // strictly reduces the number of groups, so the loop ends.
    while join(0, &solids, &mut parent, &mut space) || join(1, &solids, &mut parent, &mut space) {}

    let mut out: Vec<Solid> = Vec::with_capacity(solids.len());
    let mut named: Vec<Option<u32>> = vec![None; solids.len()];
    let mut ids: Vec<SolidId> = Vec::with_capacity(solids.len());
    for (at, solid) in solids.iter().enumerate() {
        let name = root(&mut parent, at as u32) as usize;
        let id = match named[name] {
            Some(id) => id,
            None => {
                let id = out.len() as u32;
                // The group's own representative, and every field but the box is
                // equal across the group by the merge's own conditions — so this
                // is the whole primitive rather than a choice between pieces.
                // The one field that differs is the box, which is the union.
                out.push(Solid {
                    space: space[name],
                    ..*solid
                });
                named[name] = Some(id);
                id
            }
        };
        ids.push(SolidId::new(id));
    }
    (out, ids)
}

/// Whether a primitive may be folded into a surface at all, before anything
/// about its neighbours is asked.
///
/// Both refusals are the header's, and both are about a rule stated somewhere
/// other than in the box: an [`Aperture`](super::Aperture) is measured in its own
/// tile, and a partial opacity is applied once per primitive a ray crosses.
fn mergeable(solid: &Solid) -> bool {
    solid.aperture.is_none() && solid.opacity == super::OPAQUE
}

/// Everything two primitives must agree about before their boxes are allowed to
/// meet — the module header's list, as one comparable key.
///
/// The four coordinates are the box's extents on the two axes that are **not**
/// the one being merged along, as bits: what is asked of them is exact equality,
/// and a total order over the bits is all a sort needs to bring equal ones
/// together.
///
/// Opacity is *not* in here, and its absence is the plan's own condition rather
/// than an omission: [`mergeable`] has already refused everything that is not
/// [`OPAQUE`](super::OPAQUE), so a key comparing it would be comparing a
/// constant.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Surface {
    edges: u8,
    roof: bool,
    /// The owner, spelled out: a `(z, graphic)` pair — see the header.
    z: i8,
    graphic: u16,
    part: u8,
    across: [u64; 4],
}

/// What one primitive's key is for a merge along `axis` — `0` for `x`, `1` for
/// `y`, and there is no third (see the header).
fn surface(axis: usize, solid: &Solid, space: &crate::solid::Solid) -> Surface {
    let across = match axis {
        0 => [space.min.y, space.max.y, space.min.z, space.max.z],
        _ => [space.min.x, space.max.x, space.min.z, space.max.z],
    };
    Surface {
        edges: solid.edges,
        roof: solid.roof,
        z: solid.owner.z,
        graphic: solid.owner.graphic.0,
        part: solid.part.raw(),
        across: across.map(f64::to_bits),
    }
}

/// Where a box begins and ends along the axis being merged along.
fn span(axis: usize, space: &crate::solid::Solid) -> (f64, f64) {
    match axis {
        0 => (space.min.x, space.max.x),
        _ => (space.min.y, space.max.y),
    }
}

/// One pass along one axis: every group that can grow along it does, and the
/// answer is whether anything did.
fn join(axis: usize, solids: &[Solid], parent: &mut [u32], space: &mut [crate::solid::Solid]) -> bool {
    // A windowed primitive is not offered at all — see the header — so it is
    // never anybody's neighbour either, and a run of wall with one window in it
    // comes out as three primitives rather than as a hole in the wrong tile.
    // Nor is a translucent one: two panes dim a ray twice and one merged pane
    // dims it once, which is a moved pixel whatever the geometry says.
    let mut runs: Vec<(Surface, f64, u32)> = (0..solids.len() as u32)
        .filter(|at| parent[*at as usize] == *at && mergeable(&solids[*at as usize]))
        .map(|at| {
            let space = space[at as usize];
            (
                surface(axis, &solids[at as usize], &space),
                span(axis, &space).0,
                at,
            )
        })
        .collect();
    // Equal surfaces adjacent, and within one surface in the order they stand
    // along the axis, which is what lets the coalescing below be one scan.
    runs.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));

    let mut joined = false;
    let mut at = 0;
    while at < runs.len() {
        let mut into = runs[at].2;
        let mut next = at + 1;
        while next < runs.len() && runs[next].0 == runs[at].0 {
            let (begins, _) = span(axis, &space[runs[next].2 as usize]);
            // Exact, and the whole of what "contiguous" means: a gap leaves two
            // primitives and an overlap is not a shared face at all.
            if span(axis, &space[into as usize]).1 != begins {
                break;
            }
            into = unite(parent, space, into, runs[next].2);
            joined = true;
            next += 1;
        }
        at = next.max(at + 1);
    }
    joined
}

/// Put two groups together and answer with the name of the one that survives.
///
/// The **lower** index survives, which is what makes a group's name its lowest
/// member and [`merged`]'s output order a fact about the input rather than about
/// the passes.
fn unite(parent: &mut [u32], space: &mut [crate::solid::Solid], a: u32, b: u32) -> u32 {
    let (keep, gone) = match a < b {
        true => (a, b),
        false => (b, a),
    };
    let (one, other) = (space[keep as usize], space[gone as usize]);
    // The componentwise union, which is the *exact* union of the two point sets
    // because they share a whole face: they are equal on two axes and touch on
    // the third, so nothing between them is empty and nothing outside them is
    // covered.
    space[keep as usize] = crate::solid::Solid {
        min: crate::camera::WorldSpot {
            x: one.min.x.min(other.min.x),
            y: one.min.y.min(other.min.y),
            z: one.min.z.min(other.min.z),
        },
        max: crate::camera::WorldSpot {
            x: one.max.x.max(other.max.x),
            y: one.max.y.max(other.max.y),
            z: one.max.z.max(other.max.z),
        },
    };
    parent[gone as usize] = keep;
    keep
}

/// Which group a primitive is in, by its name.
fn root(parent: &mut [u32], of: u32) -> u32 {
    let mut at = of;
    while parent[at as usize] != at {
        // Path halving: a group is a handful of pieces, and this keeps the walk
        // short without a second pass over the list.
        parent[at as usize] = parent[parent[at as usize] as usize];
        at = parent[at as usize];
    }
    at
}

#[cfg(test)]
mod tests {
    use openshard_protocol::wire::Graphic;

    use super::*;
    use crate::occlusion::{Aperture, EDGE_ANY, EDGE_SOUTH, OPAQUE, Owner, PANE, Part};

    /// One primitive as a test here states one: where it stands, what shape it is,
    /// and **whose** it is.
    ///
    /// The box comes from [`Solid::box_of`] rather than from six corners written
    /// out here, for `occlusion`'s own reason — a test that spelled the corners
    /// itself would be a second opinion about where a panel's plane is. The
    /// graphic *is* a parameter, which it is not there: telling two owners apart
    /// is half of what this module is about.
    fn stands_at(x: i32, y: i32, top: i32, edges: u8, graphic: u16) -> Solid {
        Solid {
            space: Solid::box_of(x, y, 0, top, edges),
            opacity: OPAQUE,
            edges,
            aperture: None,
            roof: false,
            owner: Owner::new(0, Graphic(graphic)),
            part: Part::ONLY,
        }
    }

    /// A run of one wall along a row is one primitive, and it is exactly the run.
    ///
    /// The whole of D2b in one assertion: four panels, one box, and the box is the
    /// union of the four rather than a bound around them — which is what makes the
    /// merge a change of *how many* primitives there are and not of what stands
    /// where.
    #[test]
    fn a_run_of_one_wall_becomes_one_primitive() {
        let run: Vec<Solid> = (100..104).map(|x| stands_at(x, 100, 20, EDGE_SOUTH, 7)).collect();
        let (solids, ids) = merged(run.clone());
        assert_eq!(solids.len(), 1, "four panels of one wall are one wall");
        // Every reference still names it — a cell's run is unchanged in length and
        // in order, which is what `Builder::finish` rests on.
        assert_eq!(ids, vec![SolidId::new(0); 4]);
        let space = solids[0].space;
        assert_eq!((space.min.x, space.max.x), (100.0, 104.0));
        // And nothing moved on the axes it was not merged along: a panel is still
        // `PANEL_THICKNESS` deep and still twenty tall.
        assert_eq!(
            (space.min.y, space.max.y),
            (run[0].space.min.y, run[0].space.max.y)
        );
        assert_eq!((space.min.z, space.max.z), (0.0, 20.0));
    }

    /// A floor is one slab, which takes **both** axes.
    ///
    /// Three by three lids: the first pass folds each row into a strip and the
    /// second folds the strips into the slab. A merge that ran one axis only would
    /// leave three primitives here and would still pass the run above, which is
    /// why this fixture is square rather than long.
    #[test]
    fn a_floor_becomes_one_slab() {
        let floor: Vec<Solid> = (100..103)
            .flat_map(|y| (100..103).map(move |x| stands_at(x, y, 0, 0, 9)))
            .collect();
        let (solids, _) = merged(floor);
        assert_eq!(solids.len(), 1, "nine planks of one floor are one floor");
        let space = solids[0].space;
        assert_eq!(
            (space.min.x, space.max.x, space.min.y, space.max.y),
            (100.0, 103.0, 100.0, 103.0)
        );
    }

    /// A gap in a run leaves two primitives, and that is the whole of what
    /// "contiguous" means here.
    #[test]
    fn a_gap_in_a_run_is_two_primitives() {
        let run = vec![
            stands_at(100, 100, 20, EDGE_SOUTH, 7),
            stands_at(101, 100, 20, EDGE_SOUTH, 7),
            // One tile of open ground, and then the wall goes on.
            stands_at(103, 100, 20, EDGE_SOUTH, 7),
        ];
        let (solids, ids) = merged(run);
        assert_eq!(solids.len(), 2);
        assert_eq!(ids, vec![SolidId::new(0), SolidId::new(0), SolidId::new(1)]);
    }

    /// Two graphics standing in one line stay two primitives.
    ///
    /// **The join is why**, and it is not a caution: a merged primitive carries one
    /// [`Owner`] and one [`Part`], and they are what
    /// [`Occlusion::id_of`](crate::occlusion::Occlusion::id_of) scans a cell for.
    /// Fold two owners into one primitive and the other instance can no longer
    /// name the box its own pixels are met against — D6's join, broken by an
    /// optimisation.
    #[test]
    fn a_run_of_two_graphics_stays_two_primitives() {
        let run = vec![
            stands_at(100, 100, 20, EDGE_SOUTH, 7),
            stands_at(101, 100, 20, EDGE_SOUTH, 8),
        ];
        let (solids, _) = merged(run);
        assert_eq!(
            solids.len(),
            2,
            "two owners are two primitives whatever their boxes do"
        );
    }

    /// Two heights standing in one line stay two primitives — the plan's "equal
    /// span", and what keeps a flight of steps three treads.
    #[test]
    fn a_step_up_in_a_run_stays_two_primitives() {
        let run = vec![
            stands_at(100, 100, 20, EDGE_ANY, 7),
            stands_at(101, 100, 25, EDGE_ANY, 7),
        ];
        let (solids, _) = merged(run);
        assert_eq!(solids.len(), 2, "two spans do not share a whole face");
    }

    /// A pane is never folded into its neighbour, and the reason is arithmetic
    /// rather than geometry.
    ///
    /// A ray crossing two panes is dimmed twice — `lighting.rs`'s
    /// `a_segment_through_two_panes_on_one_tile_is_dimmed_by_both_of_them`, and
    /// the walk's product is one factor a primitive since S5. One merged pane
    /// would dim it once, which is a moved pixel, so anything short of
    /// [`OPAQUE`] is left alone.
    #[test]
    fn a_run_of_panes_is_never_folded() {
        let run: Vec<Solid> = (100..104)
            .map(|x| Solid {
                opacity: PANE,
                ..stands_at(x, 100, 20, EDGE_SOUTH, 7)
            })
            .collect();
        let (solids, _) = merged(run);
        assert_eq!(solids.len(), 4, "two dimmings are not one dimming");
    }

    /// Nor is a windowed panel, and that is the aperture being measured in a tile.
    ///
    /// `light::run_v` is `along - along.floor()`, so a hole is a fraction of
    /// **one** tile of the run. Merge two windowed panels and the window appears
    /// in every tile of the result. Refusing costs a slab test; carrying the hole
    /// would need it stated in the primitive's own coordinates first, which is
    /// this plan's backlog and not this step.
    #[test]
    fn a_run_of_windows_is_never_folded() {
        let run: Vec<Solid> = (100..104)
            .map(|x| Solid {
                aperture: Some(Aperture::new(0.25, 0.75, 5, 15)),
                ..stands_at(x, 100, 20, EDGE_SOUTH, 7)
            })
            .collect();
        let (solids, _) = merged(run);
        assert_eq!(solids.len(), 4, "a hole is measured in its own tile");
    }

    /// A frame with nothing to merge comes back exactly as it went in.
    ///
    /// The identity case, and it is what says the step is a *deletion* of
    /// primitives rather than a rewrite of the list: the order is the cells' own,
    /// and a build that folds nothing hands the walk what it always had.
    #[test]
    fn a_list_with_nothing_to_merge_is_unchanged() {
        let apart: Vec<Solid> = (0..4)
            .map(|n| stands_at(100 + n * 2, 100, 20, EDGE_ANY, 7))
            .collect();
        let (solids, ids) = merged(apart.clone());
        assert_eq!(solids, apart);
        assert_eq!(ids, (0..4).map(SolidId::new).collect::<Vec<_>>());
    }
}
