# Automatic navigation graph

## Decision

Long-distance routing is an automatically derived navigation graph, not HPA*'s
fixed square-cluster decomposition. A cluster grid makes the map's topology
follow an arbitrary 32-tile ruler: an open field receives hundreds of synthetic
boundaries while a doorway exactly on a boundary receives special treatment.
Neither fact belongs to the world.

The graph is built once from the static `Terrain`, which intentionally contains
neither doors nor placed obstacles. Every actual step is still refined through
the caller's live terrain. This keeps the established client/server door policy:
the graph can suggest a doorway, and only the caller decides whether the body
may open it or must stop at it.

## Graph construction

1. Scan static terrain into walkable cells, preserving the resolved point each
   cell represents. A shared side is a portal only when `step_allowed` succeeds
   in both directions, so a graph edge cannot invent a height transition or
   diagonal corner cut.
2. Partition contiguous walkable cells into rectangles derived by merging
   identical row runs. These are
   topology-derived navigation regions, not a fixed tessellation: a wholly open
   facet becomes one region; walls, water and impassable statics create the
   boundaries. Exact row runs are merged vertically, which keeps regions dense
   without guessing through an irregular obstacle.
3. For each maximal contiguous run of valid crossings shared by two regions,
   create one portal. A portal has one midpoint transition when narrow and two
   endpoint transitions when wide. The transitions are vertices of the actual
   indexed graph. An inter-edge crosses the portal; an intra-edge is an exact
   low-level route through one navigation region.

The graph is thus sparse where the world is simple and gains vertices only at
real passages. It has no cluster size or hierarchy level.

## Query and refinement

Endpoints join the transitions in their own navigation region. A* runs over the
indexed transition graph, then existing `find_path` refines each graph segment
against live terrain. A very large open region is deliberately one graph node;
its long, unobstructed segment is divided into bounded low-level hops before it
is handed to `find_path`. This preserves the normal planning budget without
putting artificial graph vertices every few tiles.

If live terrain rejects a portal, the query excludes that portal and searches a
bounded number of alternative graph corridors. A block that splits the interior
of a static region remains a caller-side refusal: the static graph is not
rebuilt for live doors, crates or mobiles. The client then falls through to its
existing doors-open attempt, which cuts the resulting route at the real refusal.

## Out of scope

- Multiple graph levels. A single automatic graph is enough for this pass.
- Hand-authored waypoints or map-editor metadata.
- Live rebuilding for doors, crates, portals or housing changes.
