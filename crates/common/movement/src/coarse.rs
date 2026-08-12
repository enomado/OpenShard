//! A static HPA*-style guide for paths longer than one ordinary A* can afford.
//!
//! [`CoarseRouter`] is deliberately only a guide. It is built once from the
//! map's static terrain, then [`find_long_path`] refines the resulting corridor
//! through the terrain a body is actually walking on. That split is what makes
//! a shut door potentially passable to the graph without making a client send a
//! step through it: the client refines first through the real ground and, when
//! necessary, through its existing doors-open reading before cutting that route
//! at the shut leaf. A server-side caller can instead refine through its own
//! doors-open terrain. There is no door policy hidden here.
//!
//! The graph is HPA* in the useful, small sense: 32 by 32 clusters, transitions
//! at obstacle-free cluster borders, exact low-level costs between transitions
//! in one cluster, and a query that joins its endpoints to that graph. It does
//! not pretend that a UO tile has only four neighbours: every test of an edge
//! goes through [`find_path`] / [`step_allowed`], which keeps the shared height
//! and corner-cutting rules intact.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use openshard_protocol::direction::Direction;
use openshard_protocol::world::Point;

use crate::{Terrain, Tile, direction_toward, find_path, step_allowed};

/// The side of one coarse cluster, in tiles.
///
/// This is intentionally public: it is a tuning knob with a concrete current
/// value, rather than a 32 scattered through the builder and its callers.
pub const CLUSTER_SIZE: u16 = 32;

/// An entrance narrower than this gets one transition at its midpoint; a wider
/// one gets transitions at both ends, as HPA* prescribes.
const WIDE_ENTRANCE: u16 = 6;

/// The static graph never needs to search outside a cluster. The largest one
/// has this many tiles, so this is also an exhaustive search budget rather than
/// a guess copied from a gameplay caller.
const fn cluster_budget(width: u16, height: u16) -> usize {
    width as usize * height as usize
}

/// A route through the precomputed cluster graph.
///
/// The router has no terrain reference or copy. Keeping it that way lets a
/// client keep one beside its `Arc<Map>` and lets the shard put one beside its
/// type-erased `Terrain`; both call [`find_long_path`] with the exact static and
/// live readings they already own.
#[derive(Debug)]
pub struct CoarseRouter {
    width: u32,
    height: u32,
    clusters_wide: u32,
    clusters: Vec<Cluster>,
    /// Every transition's point and its owning cluster. An entrance has a node
    /// on each side of its border, because the intra-edges on those two sides
    /// are different questions.
    nodes: Vec<Node>,
    /// Node ids in each cluster, in construction order. That order, and the
    /// sorted edge lists below, keeps tie-breaking deterministic without ever
    /// depending on hash-map iteration.
    cluster_nodes: Vec<Vec<NodeId>>,
    /// Directed exact costs. Downhill can differ from uphill, so treating the
    /// graph as accidentally undirected would be a movement bug on a slope.
    edges: Vec<Vec<Edge>>,
}

#[derive(Clone, Copy, Debug)]
struct Cluster {
    left: u16,
    top: u16,
    width: u16,
    height: u16,
}

impl Cluster {
    fn contains(self, point: Point) -> bool {
        let x = u32::from(point.x);
        let y = u32::from(point.y);
        x >= self.left as u32
            && x < self.left as u32 + self.width as u32
            && y >= self.top as u32
            && y < self.top as u32 + self.height as u32
    }
}

#[derive(Clone, Copy, Debug)]
struct Node {
    point: Point,
    cluster: ClusterId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ClusterId(usize);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct NodeId(usize);

#[derive(Clone, Copy, Debug)]
struct Edge {
    to: NodeId,
    cost: u32,
}

/// The mutable half of one abstract A* query. Kept apart from
/// [`CoarseRouter`]'s immutable graph so the relax operation carries its own
/// queue, costs and endpoint instead of turning one small comparison into an
/// eight-argument helper.
struct GraphSearch {
    open: BinaryHeap<Reverse<(u32, u32, u32, usize)>>,
    cost: Vec<u32>,
    parent: Vec<Option<usize>>,
    goal: Point,
}

impl GraphSearch {
    fn new(nodes: usize, start: usize, from: Point, goal: Point) -> Self {
        let mut search = Self {
            open: BinaryHeap::new(),
            cost: vec![u32::MAX; nodes],
            parent: vec![None; nodes],
            goal,
        };
        let h = heuristic(from, goal);
        search.cost[start] = 0;
        search.open.push(Reverse((h, h, 0, start)));
        search
    }

    fn relax(&mut self, from: usize, to: usize, edge_cost: u32, point: Point) {
        let next_cost = self.cost[from].saturating_add(edge_cost);
        if next_cost >= self.cost[to] {
            return;
        }
        self.cost[to] = next_cost;
        self.parent[to] = Some(from);
        let h = heuristic(point, self.goal);
        self.open
            .push(Reverse((next_cost.saturating_add(h), h, next_cost, to)));
    }
}

/// One cluster's view of a terrain. An intra-edge must stay inside its cluster:
/// allowing the low-level A* to leave and re-enter would give an edge a cost the
/// abstract graph cannot faithfully represent.
struct InCluster<'a> {
    terrain: &'a dyn Terrain,
    cluster: Cluster,
}

impl Terrain for InCluster<'_> {
    fn can_step(&self, from: Point, to: Point) -> Option<Point> {
        (self.cluster.contains(from) && self.cluster.contains(to))
            .then(|| self.terrain.can_step(from, to))
            .flatten()
    }
}

impl CoarseRouter {
    /// Build a static route graph over a `width` by `height` facet.
    ///
    /// `Point` has `u16` coordinates, so a facet larger than that coordinate
    /// space cannot be represented by this movement crate and returns `None`.
    /// Empty maps do too. Normal UO facets (including 7168×4096 Britannia) are
    /// well inside that limit.
    #[must_use]
    pub fn build(terrain: &dyn Terrain, width: u32, height: u32) -> Option<Self> {
        let coordinate_limit = u32::from(u16::MAX) + 1;
        if width == 0 || height == 0 || width > coordinate_limit || height > coordinate_limit {
            return None;
        }

        let clusters_wide = width.div_ceil(u32::from(CLUSTER_SIZE));
        let clusters_high = height.div_ceil(u32::from(CLUSTER_SIZE));
        let mut clusters = Vec::with_capacity((clusters_wide * clusters_high) as usize);
        for cy in 0..clusters_high {
            for cx in 0..clusters_wide {
                let left = cx * u32::from(CLUSTER_SIZE);
                let top = cy * u32::from(CLUSTER_SIZE);
                clusters.push(Cluster {
                    left: left as u16,
                    top: top as u16,
                    width: (width - left).min(u32::from(CLUSTER_SIZE)) as u16,
                    height: (height - top).min(u32::from(CLUSTER_SIZE)) as u16,
                });
            }
        }

        let mut router = Self {
            width,
            height,
            clusters_wide,
            cluster_nodes: vec![Vec::new(); clusters.len()],
            clusters,
            nodes: Vec::new(),
            edges: Vec::new(),
        };

        // Vertical borders, read top-to-bottom. The node on the left belongs to
        // this cluster; the right one to the neighbour.
        for cy in 0..clusters_high {
            for cx in 0..clusters_wide.saturating_sub(1) {
                let left = router.cluster_id(cx, cy);
                let right = router.cluster_id(cx + 1, cy);
                let left_cluster = router.clusters[left.0];
                let x = u32::from(left_cluster.left) + u32::from(left_cluster.width) - 1;
                router.add_vertical_entrances(terrain, left, right, x as u16);
            }
        }

        // Horizontal borders, read left-to-right. The node above belongs to this
        // cluster; the lower one to the neighbour.
        for cy in 0..clusters_high.saturating_sub(1) {
            for cx in 0..clusters_wide {
                let top = router.cluster_id(cx, cy);
                let bottom = router.cluster_id(cx, cy + 1);
                let top_cluster = router.clusters[top.0];
                let y = u32::from(top_cluster.top) + u32::from(top_cluster.height) - 1;
                router.add_horizontal_entrances(terrain, top, bottom, y as u16);
            }
        }

        router.add_intra_edges(terrain);
        for edges in &mut router.edges {
            edges.sort_unstable_by_key(|edge| (edge.to, edge.cost));
            edges.dedup_by(|first, second| {
                if first.to != second.to {
                    return false;
                }
                first.cost = first.cost.min(second.cost);
                true
            });
        }
        Some(router)
    }

    /// The facet dimensions this graph was built for.
    #[must_use]
    pub const fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Return the static corridor's transition points, excluding the two query
    /// endpoints. `None` means the static map has no abstract route between
    /// their clusters.
    ///
    /// The supplied terrain must be the same static reading used to build this
    /// router. It supplies the temporary endpoint-to-transition costs; it is
    /// intentionally separate from the live terrain refined by
    /// [`find_long_path`].
    #[must_use]
    pub fn waypoints(&self, terrain: &dyn Terrain, from: Point, to: Point) -> Option<Vec<Point>> {
        let forbidden = vec![false; self.nodes.len()];
        self.abstract_path(terrain, from, to, &forbidden)
            .map(|path| path.into_iter().map(|id| self.nodes[id.0].point).collect())
    }

    fn cluster_id(&self, x: u32, y: u32) -> ClusterId {
        ClusterId((y * self.clusters_wide + x) as usize)
    }

    fn cluster_at(&self, point: Point) -> Option<ClusterId> {
        (u32::from(point.x) < self.width && u32::from(point.y) < self.height).then(|| {
            self.cluster_id(
                u32::from(point.x) / u32::from(CLUSTER_SIZE),
                u32::from(point.y) / u32::from(CLUSTER_SIZE),
            )
        })
    }

    fn point_at(terrain: &dyn Terrain, x: u16, y: u16) -> Point {
        let tile = Tile::new(x, y);
        let near = terrain.ground_z(tile).unwrap_or(0);
        let z = terrain
            .spawn_z(tile, i32::from(near))
            .and_then(|z| i8::try_from(z).ok())
            .unwrap_or(near);
        Point::new(x, y, z)
    }

    /// Add every maximal run on one east-west cluster border.
    fn add_vertical_entrances(&mut self, terrain: &dyn Terrain, left: ClusterId, right: ClusterId, x: u16) {
        let cluster = self.clusters[left.0];
        let first = cluster.top;
        let end = u32::from(cluster.top) + u32::from(cluster.height);
        let mut y = u32::from(first);
        while y < end {
            let Some(pair) = self.vertical_pair(terrain, x, y as u16) else {
                y += 1;
                continue;
            };
            let mut pairs = vec![pair];
            y += 1;
            while y < end {
                match self.vertical_pair(terrain, x, y as u16) {
                    Some(pair) => {
                        pairs.push(pair);
                        y += 1;
                    }
                    None => break,
                }
            }
            self.add_entrance(left, right, &pairs);
        }
    }

    /// Add every maximal run on one north-south cluster border.
    fn add_horizontal_entrances(&mut self, terrain: &dyn Terrain, top: ClusterId, bottom: ClusterId, y: u16) {
        let cluster = self.clusters[top.0];
        let first = cluster.left;
        let end = u32::from(cluster.left) + u32::from(cluster.width);
        let mut x = u32::from(first);
        while x < end {
            let Some(pair) = self.horizontal_pair(terrain, x as u16, y) else {
                x += 1;
                continue;
            };
            let mut pairs = vec![pair];
            x += 1;
            while x < end {
                match self.horizontal_pair(terrain, x as u16, y) {
                    Some(pair) => {
                        pairs.push(pair);
                        x += 1;
                    }
                    None => break,
                }
            }
            self.add_entrance(top, bottom, &pairs);
        }
    }

    /// The two landing points across one vertical border, only when both ways
    /// across it really work. The reverse landing becomes the left node so both
    /// nodes carry a height that the crossing itself already proved valid.
    fn vertical_pair(&self, terrain: &dyn Terrain, x: u16, y: u16) -> Option<(Point, Point)> {
        let left = Self::point_at(terrain, x, y);
        let right = step_allowed(terrain, left, Direction::East)?;
        let returned = step_allowed(terrain, right, Direction::West)?;
        (returned.x == x && returned.y == y && right.x == x + 1 && right.y == y).then_some((returned, right))
    }

    /// The equivalent pair across one horizontal border.
    fn horizontal_pair(&self, terrain: &dyn Terrain, x: u16, y: u16) -> Option<(Point, Point)> {
        let top = Self::point_at(terrain, x, y);
        let bottom = step_allowed(terrain, top, Direction::South)?;
        let returned = step_allowed(terrain, bottom, Direction::North)?;
        (returned.x == x && returned.y == y && bottom.x == x && bottom.y == y + 1)
            .then_some((returned, bottom))
    }

    fn add_entrance(
        &mut self,
        first_cluster: ClusterId,
        second_cluster: ClusterId,
        pairs: &[(Point, Point)],
    ) {
        debug_assert!(!pairs.is_empty());
        let indices = match pairs.len() as u16 {
            0 => Vec::new(),
            1..WIDE_ENTRANCE => {
                // The earlier of an even run's two middle tiles is deliberate:
                // it makes the graph's answer fixed without inventing a half-tile.
                let middle = (pairs.len() - 1) / 2;
                vec![middle]
            }
            _ => vec![0, pairs.len() - 1],
        };
        for index in indices {
            let (first, second) = pairs[index];
            let first_id = self.add_node(first_cluster, first);
            let second_id = self.add_node(second_cluster, second);
            self.add_edge(first_id, second_id, 1);
            self.add_edge(second_id, first_id, 1);
        }
    }

    fn add_node(&mut self, cluster: ClusterId, point: Point) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node { point, cluster });
        self.edges.push(Vec::new());
        self.cluster_nodes[cluster.0].push(id);
        id
    }

    fn add_edge(&mut self, from: NodeId, to: NodeId, cost: u32) {
        self.edges[from.0].push(Edge { to, cost });
    }

    fn add_intra_edges(&mut self, terrain: &dyn Terrain) {
        for cluster_id in 0..self.clusters.len() {
            let nodes = self.cluster_nodes[cluster_id].clone();
            let cluster = self.clusters[cluster_id];
            let budget = cluster_budget(cluster.width, cluster.height);
            let local = InCluster { terrain, cluster };
            for &from in &nodes {
                for &to in &nodes {
                    if from != to {
                        let Some(route) =
                            find_path(&local, self.nodes[from.0].point, self.nodes[to.0].point, budget)
                        else {
                            continue;
                        };
                        self.add_edge(from, to, route.len() as u32);
                    }
                }
            }
        }
    }

    /// The abstract route, with endpoint connector costs recalculated over the
    /// static terrain for this query. `forbidden` lets refinement discard a
    /// transition a live obstruction made unusable and take another corridor.
    fn abstract_path(
        &self,
        terrain: &dyn Terrain,
        from: Point,
        to: Point,
        forbidden: &[bool],
    ) -> Option<Vec<NodeId>> {
        let from_cluster = self.cluster_at(from)?;
        let to_cluster = self.cluster_at(to)?;
        debug_assert_eq!(forbidden.len(), self.nodes.len());

        let source = self.local_costs(terrain, from_cluster, from, forbidden, false);
        let target = self.local_costs(terrain, to_cluster, to, forbidden, true);
        if source.is_empty() || target.is_empty() {
            return None;
        }
        let mut target_cost = vec![None; self.nodes.len()];
        for (id, cost) in target {
            target_cost[id.0] = Some(cost);
        }

        let start = self.nodes.len();
        let goal = start + 1;
        // `(f, h, g, id)`: the same useful tie-breaking shape as `path.rs`,
        // with an indexed id as the final stable comparison.
        let mut search = GraphSearch::new(goal + 1, start, from, to);

        while let Some(Reverse((_f, _h, here_cost, here))) = search.open.pop() {
            if here_cost != search.cost[here] {
                continue;
            }
            if here == goal {
                let mut path = Vec::new();
                let mut at = goal;
                while at != start {
                    at = search.parent[at]?;
                    if at < self.nodes.len() {
                        path.push(NodeId(at));
                    }
                }
                path.reverse();
                return Some(path);
            }

            if here == start {
                for &(next, edge_cost) in &source {
                    search.relax(here, next.0, edge_cost, self.nodes[next.0].point);
                }
                continue;
            }

            for edge in &self.edges[here] {
                if !forbidden[edge.to.0] {
                    search.relax(here, edge.to.0, edge.cost, self.nodes[edge.to.0].point);
                }
            }
            if let Some(edge_cost) = target_cost[here] {
                search.relax(here, goal, edge_cost, to);
            }
        }
        None
    }

    fn local_costs(
        &self,
        terrain: &dyn Terrain,
        cluster_id: ClusterId,
        endpoint: Point,
        forbidden: &[bool],
        toward_endpoint: bool,
    ) -> Vec<(NodeId, u32)> {
        let cluster = self.clusters[cluster_id.0];
        let local = InCluster { terrain, cluster };
        let budget = cluster_budget(cluster.width, cluster.height);
        self.cluster_nodes[cluster_id.0]
            .iter()
            .copied()
            .filter(|&node| !forbidden[node.0])
            .filter_map(|node| {
                let (from, to) = match toward_endpoint {
                    true => (self.nodes[node.0].point, endpoint),
                    false => (endpoint, self.nodes[node.0].point),
                };
                find_path(&local, from, to, budget).map(|route| (node, route.len() as u32))
            })
            .collect()
    }

    /// Refine a fixed abstract path through `terrain`, returning the transition
    /// that live terrain made unusable on failure. The caller can then exclude
    /// it and run the cheap graph search again.
    fn refine(
        &self,
        terrain: &dyn Terrain,
        from: Point,
        to: Point,
        nodes: &[NodeId],
        hop_budget: usize,
    ) -> Result<Vec<Direction>, NodeId> {
        if nodes.is_empty() {
            return find_path(terrain, from, to, hop_budget).ok_or(NodeId(usize::MAX));
        }

        let mut result = Vec::new();
        let mut at = from;
        let mut previous = None::<NodeId>;
        for &node in nodes {
            let route = match previous {
                None => self.in_cluster_path(
                    terrain,
                    at,
                    self.nodes[node.0].point,
                    self.nodes[node.0].cluster,
                    hop_budget,
                ),
                Some(previous) if self.nodes[previous.0].cluster == self.nodes[node.0].cluster => self
                    .in_cluster_path(
                        terrain,
                        at,
                        self.nodes[node.0].point,
                        self.nodes[node.0].cluster,
                        hop_budget,
                    ),
                Some(_) => self.cross_border(terrain, at, self.nodes[node.0].point),
            };
            let Some(route) = route else {
                return Err(node);
            };
            let Some(next) = append(terrain, at, &route, &mut result) else {
                return Err(node);
            };
            at = next;
            previous = Some(node);
        }

        let last = *nodes.last().expect("the empty path returned above");
        let Some(route) = self.in_cluster_path(terrain, at, to, self.nodes[last.0].cluster, hop_budget)
        else {
            // A different entrance into the target cluster may still reach the
            // goal around the new obstruction, so excluding the last one is a
            // useful retry rather than treating the target as globally blocked.
            return Err(last);
        };
        append(terrain, at, &route, &mut result).ok_or(last)?;
        Ok(result)
    }

    fn in_cluster_path(
        &self,
        terrain: &dyn Terrain,
        from: Point,
        to: Point,
        cluster: ClusterId,
        budget: usize,
    ) -> Option<Vec<Direction>> {
        let local = InCluster {
            terrain,
            cluster: self.clusters[cluster.0],
        };
        find_path(&local, from, to, budget)
    }

    fn cross_border(&self, terrain: &dyn Terrain, from: Point, to: Point) -> Option<Vec<Direction>> {
        let direction = direction_toward(from, to)?;
        (!direction.is_diagonal())
            .then(|| step_allowed(terrain, from, direction))
            .flatten()
            .filter(|landing| landing.x == to.x && landing.y == to.y)
            .map(|_| vec![direction])
    }
}

/// Refine a path through a static coarse graph on live terrain.
///
/// `guide` is the static terrain used to build `router`; `terrain` is the
/// terrain that must allow the actual steps. The latter may include a closed
/// door, a crate or a mobile. When it makes one transition unusable, up to eight
/// alternate abstract corridors are tried before the function says there is no
/// whole route. The caller decides what that means — client `steer::plan`, for
/// example, next tries its doors-open terrain and splits that route at the first
/// real refusal.
#[must_use]
pub fn find_long_path(
    guide: &dyn Terrain,
    terrain: &dyn Terrain,
    router: &CoarseRouter,
    from: Point,
    to: Point,
    hop_budget: usize,
) -> Option<Vec<Direction>> {
    const LIVE_REROUTES: usize = 8;
    let mut forbidden = vec![false; router.nodes.len()];
    for _ in 0..=LIVE_REROUTES {
        let path = router.abstract_path(guide, from, to, &forbidden)?;
        match router.refine(terrain, from, to, &path, hop_budget) {
            Ok(route) => return Some(route),
            Err(node) if node.0 < forbidden.len() && !forbidden[node.0] => forbidden[node.0] = true,
            Err(_) => return None,
        }
    }
    None
}

/// The graph heuristic: an eight-way path cannot cost less than its Chebyshev
/// distance, and every graph edge is an exact path length, so this stays
/// admissible across both edge kinds.
fn heuristic(from: Point, to: Point) -> u32 {
    let dx = i32::from(from.x).abs_diff(i32::from(to.x));
    let dy = i32::from(from.y).abs_diff(i32::from(to.y));
    dx.max(dy)
}

/// Extend a route while preserving the actual landing heights from `terrain`.
/// It is a defensive check as well as a conversion: the graph's static answer
/// and live ground may have changed between its two reads.
fn append(
    terrain: &dyn Terrain,
    from: Point,
    route: &[Direction],
    out: &mut Vec<Direction>,
) -> Option<Point> {
    let mut at = from;
    for &direction in route {
        at = step_allowed(terrain, at, direction)?;
        out.push(direction);
    }
    Some(at)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use proptest::prelude::*;

    use super::*;

    #[derive(Default)]
    struct Grid {
        width: u16,
        height: u16,
        blocked: BTreeSet<(u16, u16)>,
    }

    impl Grid {
        fn open(width: u16, height: u16) -> Self {
            Self {
                width,
                height,
                blocked: BTreeSet::new(),
            }
        }

        fn block(&mut self, x: u16, y: u16) {
            self.blocked.insert((x, y));
        }
    }

    impl Terrain for Grid {
        fn can_step(&self, _from: Point, to: Point) -> Option<Point> {
            (to.x < self.width && to.y < self.height && !self.blocked.contains(&(to.x, to.y))).then_some(to)
        }
    }

    fn end(from: Point, route: &[Direction]) -> Point {
        route.iter().fold(from, |at, &direction| {
            let (x, y) = direction.step();
            Point::new((i32::from(at.x) + x) as u16, (i32::from(at.y) + y) as u16, at.z)
        })
    }

    #[test]
    fn a_wide_border_has_two_transitions_at_its_ends() {
        let terrain = Grid::open(64, 32);
        let router = CoarseRouter::build(&terrain, 64, 32).expect("a valid facet");
        assert_eq!(
            router.nodes.len(),
            4,
            "two transition pairs, one on each side of the border"
        );
        let points: Vec<_> = router
            .nodes
            .iter()
            .map(|node| (node.point.x, node.point.y))
            .collect();
        assert!(points.contains(&(31, 0)) && points.contains(&(32, 0)));
        assert!(points.contains(&(31, 31)) && points.contains(&(32, 31)));
    }

    #[test]
    fn two_narrow_doorways_are_two_entrances_not_one_long_run() {
        let mut terrain = Grid::open(64, 32);
        for y in 0..32 {
            if y != 5 && y != 23 {
                terrain.block(32, y);
            }
        }
        let router = CoarseRouter::build(&terrain, 64, 32).expect("a valid facet");
        assert_eq!(
            router.nodes.len(),
            4,
            "one transition pair for each one-tile doorway"
        );
        let points: Vec<_> = router
            .nodes
            .iter()
            .map(|node| (node.point.x, node.point.y))
            .collect();
        assert!(points.contains(&(31, 5)) && points.contains(&(32, 5)));
        assert!(points.contains(&(31, 23)) && points.contains(&(32, 23)));
    }

    #[test]
    fn a_long_open_walk_is_refined_in_small_exact_hops() {
        let terrain = Grid::open(192, 96);
        let router = CoarseRouter::build(&terrain, 192, 96).expect("a valid facet");
        let from = Point::new(1, 1, 0);
        let to = Point::new(190, 94, 0);
        assert!(
            find_path(&terrain, from, to, 100).is_none(),
            "one ordinary bounded A* gives up"
        );
        let route =
            find_long_path(&terrain, &terrain, &router, from, to, 100).expect("the coarse corridor arrives");
        assert_eq!(end(from, &route), to);
    }

    #[test]
    fn a_coarse_route_follows_the_only_gap_round_a_large_wall() {
        let mut terrain = Grid::open(192, 96);
        for y in 0..80 {
            terrain.block(96, y);
        }
        let router = CoarseRouter::build(&terrain, 192, 96).expect("a valid facet");
        let from = Point::new(2, 2, 0);
        let to = Point::new(188, 2, 0);
        let route = find_long_path(&terrain, &terrain, &router, from, to, 150).expect("the gap is a route");
        assert_eq!(end(from, &route), to);
        let mut at = from;
        for direction in route {
            at = step_allowed(&terrain, at, direction).expect("every refined step remains legal");
            assert!(at.x != 96 || at.y >= 80, "the route never stands in the wall");
        }
    }

    #[test]
    fn a_live_door_is_not_silently_walked_through() {
        let guide = Grid::open(192, 64);
        let mut shut = Grid::open(192, 64);
        for y in 0..64 {
            shut.block(96, y);
        }
        let router = CoarseRouter::build(&guide, 192, 64).expect("a valid facet");
        let from = Point::new(2, 2, 0);
        let to = Point::new(188, 2, 0);
        assert!(
            find_long_path(&guide, &shut, &router, from, to, 100).is_none(),
            "the static graph may see an opening, but the live walk cannot pass a shut leaf"
        );
    }

    /// The graph is allowed to choose a different corridor from exhaustive A*,
    /// but it must not lose one through a sequence of narrow, randomly placed
    /// cluster-border gates. Replaying each returned direction through the
    /// common movement rule makes this both a connectivity and a refinement
    /// parity check — no abstract edge can turn into an illegal walk.
    #[test]
    fn fuzzed_cluster_gates_match_exhaustive_reachability() {
        const WIDTH: u16 = 192;
        const HEIGHT: u16 = 96;
        const WALLS: [u16; 5] = [32, 64, 96, 128, 160];

        proptest!(ProptestConfig::with_cases(64), |(
            from_y in 1u16..HEIGHT - 1,
            to_y in 1u16..HEIGHT - 1,
            gates in prop::collection::vec(1u16..HEIGHT - 1, WALLS.len()),
        )| {
            let mut terrain = Grid::open(WIDTH, HEIGHT);
            for (&wall, &gate) in WALLS.iter().zip(&gates) {
                for y in 0..HEIGHT {
                    if !(gate.saturating_sub(1)..=gate.saturating_add(1)).contains(&y) {
                        terrain.block(wall, y);
                    }
                }
            }

            let from = Point::new(1, from_y, 0);
            let to = Point::new(WIDTH - 2, to_y, 0);
            let exact = find_path(&terrain, from, to, usize::from(WIDTH) * usize::from(HEIGHT));
            prop_assert!(exact.is_some(), "the generated gates always connect the two sides");

            let router = CoarseRouter::build(&terrain, u32::from(WIDTH), u32::from(HEIGHT))
                .expect("the generated facet is representable");
            let route = find_long_path(&terrain, &terrain, &router, from, to, 600)
                .expect("the coarse graph preserves the gated route");

            let mut at = from;
            for direction in route {
                at = step_allowed(&terrain, at, direction)
                    .expect("the refined route contains only executable directions");
            }
            prop_assert_eq!(at, to);
        });
    }
}
