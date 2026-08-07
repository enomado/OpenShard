# A position is not a coordinate

**The world is already continuous.** `camera::WorldSpot` is three `f64` in tiles,
`occlusion::Solid::space` is two of them, `light::Spot` carries an `f32` pair and
an `f32` height, and `docs/lighting_height.md`'s phases 1 and 2 made height
continuous on both sides of the wire. There is no integer world to migrate off.

What is still integer is the **cell**, and the defect is not that it exists — a
grid is an index and an index is discrete — but that it is *derived from the
coordinate*, by `floor`, at every site that needs it. That derivation is wrong on
a boundary, and this world's geometry is built on boundaries: a wall's plane is a
tile edge, a stair's riser is a tile edge, a tread's own strip ends on one. The
common case is the failing case.

This plan makes a position carry its cell instead of implying it, so that `floor`
has nobody left to lie to.

## The defect this comes from, five times

Each of these is the same sentence written again, and none of them was found by
the same means as the one before it:

| where | what it says | how it was found |
|---|---|---|
| `light::Spot::tile` | "**not** `at.x.floor()`/`at.y.floor()`" — a stair tread's outer corner sits at a whole `x`, and `floor` picks the side that rounds down | a lit pixel in the wrong tile |
| `mesh_face::MeshFaceVertex::tile` | "the CPU twin of `MeshFaceVertex::tile`'s fix to the same class of bug on the GPU side" | an isolated lit pixel on an evenly shadowed face |
| `light::walk_cells_streaming`'s `boundary[axis]` | "the known tile's own edge, not `from.floor()`" — flooring seeds a whole tile of slack that was never there | a wall row the walk stepped past |
| `occlusion::Solid::footprint` | `false if far && min.fract() == 0.0 => min.floor() - 1` — a degenerate axis at a whole coordinate belongs to the tile *below* | decision 38.2's spill, argued in the doc |
| `occlusion::Solid::fraction` | the same branch, missing — a flight's bottom riser was rebuilt a tile's width away from its own cell, and the front face of every bottom step shadowed nothing | **rendering the scene twice and looking at the two pictures** |

Three of the five are "carry the cell beside the position"; two are "spell the
boundary rule by hand". Nothing connects them in the type system, so a sixth site
is a sixth chance, and the fifth one stood for as long as it did because
`walk_cells_exact` reads `Solid::space` directly and was right the whole time —
the two walks disagreed on a scene no fixture could pose, since the agreement
proptests build panels with `Solid::box_of`, whose slab is `PANEL_THICKNESS` deep
and therefore never a plane.

**That is the shape of the argument, and it is not "use floats".** Every one of
these five already has floats. What none of them has is a type that makes
"which cell is this" answerable without arithmetic.

## What this cannot fix, and must not pretend to

- **`world::Point` is `u16, u16, i8` and stays.** It is the protocol's position
  and the client's: mobiles and statics stand on whole tiles because Ultima
  Online says so. This plan is about the render and lighting side's own
  positions, not about giving an entity a fractional home.
- **The `place` attachment's quantum stays a quantum.** It carries a sub-tile
  fraction to a hundred-and-twenty-seventh and a height to a sixteenth. That is
  a wire format; `light::ON_TOP` (`1/128`) and `light::STAND_OFF` (`2/127`) are
  sized against it and go on being sized against it. A fragment's position is
  quantised no matter what type holds it.
- **The occlusion grid stays indexed by whole cells.** A spatial index is
  supposed to be discrete. What changes is who computes the index and from what.

So the win is not precision. It is that a whole class of boundary bug stops
being writable, and that five hand-written statements of one rule become one.

## The design

A world position becomes a pair, and the pair is the type:

```rust
/// Where something is: the cell it belongs to, and where inside that cell.
pub struct Spot {
    cell: Cell,          // (i32, i32) — an index, not a coordinate
    within: Vec2,        // 0.0..=1.0 in each axis, the fraction of the cell
    z: f32,
}
```

- **`cell` is given, never derived.** Every producer already knows it — a mesh
  face knows the static's tile, a fragment's row carries it, a walk is started
  from a caller that named it. Today they know it and then throw it away by
  handing on a bare coordinate; this keeps it.
- **`within` is a fraction and can legitimately be `0.0` or `1.0`.** A point on a
  cell's far edge is `1.0` *of that cell*, which is a different fact from `0.0`
  of the next one, and today those two are the same number.
- **The absolute coordinate is a projection of the pair**, `cell + within`, and
  it is what goes to arithmetic — a slab test, a projection, a distance. It is
  the `.0` of this newtype, and like every `.0` in this repo it is unwrapped at
  the edge and not carried through the call tree.
- **A solid's box is a pair too**: `Solid::space` becomes corners relative to the
  solid's own cell, which is what `fraction`/`box_from_footprint` already ship
  over the wire and reconstruct — badly, because the cell they reconstruct
  against is re-derived rather than carried.

## Order, and what gates what

The ritual this repo already uses, and this plan's own phase 0 is not optional:
the last two lighting phases each spent a session discovering that the residual
they were chasing was the instrument.

- **Phase 0 — a gate on the old layer.** A round trip is already in
  (`occlusion.rs`'s `every_solid_comes_back_off_the_wire_on_the_cell_it_was_put_on`)
  and it covers one producer. Widen it to a property over *every* solid any
  builder makes, and add its twin for positions: a `Spot` that survives
  `place`'s pack/unpack and names the same cell. Both must be green before
  anything moves, and both must be able to go red — mutate `fraction` back to a
  bare `floor` and the first one names the solid; that is the standard.
- **Phase 1 — the type, beside the old one.** Introduce the pair and its
  constructors, convert `light::Spot` first (it already carries `tile` beside
  `at`, so the change is deleting a redundancy rather than adding a field), and
  leave every other producer converting at its boundary.
- **Phase 2 — the solid.** `Solid::space` relative to its own cell, `fraction`
  and `box_from_footprint` become the identity on that representation rather
  than a lossy round trip through an absolute coordinate. This is where the
  fifth row of the table above stops being possible.
- **Phase 3 — the walks.** `walk_cells_*` and `blit.wesl` take the pair, and
  `first`/`last`/`boundary[axis]` stop being three different opinions about
  where a cell begins.
- **Phase 4 — delete the hand-written rules.** `Solid::footprint`'s `far`
  branch, `Spot::tile`'s doc warning, `MeshFaceVertex::tile`'s note. If any of
  them is still load-bearing at that point, the type did not do its job and the
  plan says so instead of leaving the branch in beside it.

Each phase lands with `examples/synthetic_stair`'s reference frame beside the
rendered one, because the two pictures are what caught the fifth row and a count
is what missed it for as long as it stood.

## Status

Not started. This document exists because the question has been asked more times
than it has been answered, and the answer is yes — with the correction that the
thing to move is not the coordinates.
