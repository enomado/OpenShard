//! A topology-derived, single-level navigation graph.
//!
//! Unlike a cluster hierarchy, this graph has no ruler imposed on the map.
//! Its regions come from the static terrain's merged walkable row runs;
//! walls and water create portals, an open field does not manufacture any.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::time::Instant;

use openshard_protocol::direction::Direction;
use openshard_protocol::world::Point;

use crate::{Terrain, Tile, find_path, find_path_toward, step_allowed};

const WIDE_PORTAL: usize = 6;
/// A region stays well inside the normal 600-cell refinement budget, while the
/// whole facet has only a few thousand regions. Obstacles inside one are live
/// terrain, not graph boundaries, so a forest does not emit a node per tree.
const REGION_SIZE: u32 = 32;

#[derive(Debug, PartialEq, Eq)]
pub struct NavigationGraph {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) regions: Vec<Region>,
    pub(crate) at: Vec<Option<RegionId>>,
    pub(crate) nodes: Vec<Node>,
    pub(crate) region_nodes: Vec<Vec<NodeId>>,
    pub(crate) edges: Vec<Vec<Edge>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Region {
    pub(crate) left: u16,
    pub(crate) top: u16,
    pub(crate) width: u16,
    pub(crate) height: u16,
}

impl Region {
    fn contains(self, point: Point) -> bool {
        let x = u32::from(point.x);
        let y = u32::from(point.y);
        x >= u32::from(self.left)
            && x < u32::from(self.left) + u32::from(self.width)
            && y >= u32::from(self.top)
            && y < u32::from(self.top) + u32::from(self.height)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RegionId(pub(crate) usize);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct NodeId(pub(crate) usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Node {
    pub(crate) point: Point,
    pub(crate) region: RegionId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Edge {
    pub(crate) to: NodeId,
    pub(crate) cost: u32,
}

struct InRegion<'a> {
    terrain: &'a dyn Terrain,
    region: Region,
}

impl Terrain for InRegion<'_> {
    fn can_step(&self, from: Point, to: Point) -> Option<Point> {
        (self.region.contains(from) && self.region.contains(to))
            .then(|| self.terrain.can_step(from, to))
            .flatten()
    }
}

impl NavigationGraph {
    /// Extract a static graph from one facet. Empty and unrepresentable facets
    /// cannot be addressed by `Point` and therefore have no graph.
    #[must_use]
    pub fn build(terrain: &dyn Terrain, width: u32, height: u32) -> Option<Self> {
        let limit = u32::from(u16::MAX) + 1;
        if width == 0 || height == 0 || width >= limit || height >= limit {
            return None;
        }
        let started = Instant::now();
        eprintln!("navigation graph: sampling {width}x{height} terrain");
        let cells = width as usize * height as usize;
        let mut points = vec![None; cells];
        for y in 0..height as u16 {
            for x in 0..width as u16 {
                let tile = Tile::new(x, y);
                let near = terrain.ground_z(tile).unwrap_or(0);
                let point = Point::new(x, y, near);
                points[usize::from(y) * width as usize + usize::from(x)] = terrain.can_step(point, point);
            }
        }
        eprintln!(
            "navigation graph +{:.3}s: terrain sampled",
            started.elapsed().as_secs_f64()
        );

        let mut graph = Self {
            width,
            height,
            regions: Vec::new(),
            at: vec![None; cells],
            nodes: Vec::new(),
            region_nodes: Vec::new(),
            edges: Vec::new(),
        };
        graph.partition(&points);
        eprintln!(
            "navigation graph +{:.3}s: partitioned into {} regions",
            started.elapsed().as_secs_f64(),
            graph.regions.len()
        );
        graph.portals(terrain, &points);
        eprintln!(
            "navigation graph +{:.3}s: {} portal nodes found; calculating intra-region routes",
            started.elapsed().as_secs_f64(),
            graph.nodes.len()
        );
        graph.intra_edges(terrain);
        for edges in &mut graph.edges {
            edges.sort_unstable_by_key(|edge| (edge.to, edge.cost));
            edges.dedup_by_key(|edge| edge.to);
        }
        eprintln!(
            "navigation graph +{:.3}s: ready ({} nodes, {} edges)",
            started.elapsed().as_secs_f64(),
            graph.nodes.len(),
            graph.edges.iter().map(Vec::len).sum::<usize>()
        );
        Some(graph)
    }

    #[must_use]
    pub const fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Counts useful to an offline builder and its progress report.
    #[must_use]
    pub fn counts(&self) -> (usize, usize, usize) {
        (
            self.regions.len(),
            self.nodes.len(),
            self.edges.iter().map(Vec::len).sum(),
        )
    }

    fn index(&self, x: u16, y: u16) -> usize {
        usize::from(y) * self.width as usize + usize::from(x)
    }

    fn region_at(&self, point: Point) -> Option<RegionId> {
        (u32::from(point.x) < self.width && u32::from(point.y) < self.height)
            .then(|| self.at[self.index(point.x, point.y)])
            .flatten()
    }

    fn partition(&mut self, points: &[Option<Point>]) {
        for top in (0..self.height).step_by(REGION_SIZE as usize) {
            for left in (0..self.width).step_by(REGION_SIZE as usize) {
                let id = RegionId(self.regions.len());
                let width = REGION_SIZE.min(self.width - left) as u16;
                let height = REGION_SIZE.min(self.height - top) as u16;
                self.regions.push(Region {
                    left: left as u16,
                    top: top as u16,
                    width,
                    height,
                });
                self.region_nodes.push(Vec::new());
                for y in top as u16..top as u16 + height {
                    for x in left as u16..left as u16 + width {
                        let index = self.index(x, y);
                        if points[index].is_some() {
                            self.at[index] = Some(id);
                        }
                    }
                }
            }
        }
    }

    fn portals(&mut self, terrain: &dyn Terrain, points: &[Option<Point>]) {
        for x in 0..(self.width as u16).saturating_sub(1) {
            let mut y = 0;
            while y < self.height as u16 {
                let Some(pair) = self.vertical_pair(terrain, points, x, y) else {
                    y += 1;
                    continue;
                };
                let first = self.region_at(pair.0).unwrap();
                let second = self.region_at(pair.1).unwrap();
                if first == second {
                    y += 1;
                    continue;
                }
                let mut run = vec![pair];
                y += 1;
                while y < self.height as u16 {
                    let Some(next) = self.vertical_pair(terrain, points, x, y) else {
                        break;
                    };
                    if self.region_at(next.0) != Some(first) || self.region_at(next.1) != Some(second) {
                        break;
                    }
                    run.push(next);
                    y += 1;
                }
                self.add_portal(first, second, &run);
            }
        }
        for y in 0..(self.height as u16).saturating_sub(1) {
            let mut x = 0;
            while x < self.width as u16 {
                let Some(pair) = self.horizontal_pair(terrain, points, x, y) else {
                    x += 1;
                    continue;
                };
                let first = self.region_at(pair.0).unwrap();
                let second = self.region_at(pair.1).unwrap();
                if first == second {
                    x += 1;
                    continue;
                }
                let mut run = vec![pair];
                x += 1;
                while x < self.width as u16 {
                    let Some(next) = self.horizontal_pair(terrain, points, x, y) else {
                        break;
                    };
                    if self.region_at(next.0) != Some(first) || self.region_at(next.1) != Some(second) {
                        break;
                    }
                    run.push(next);
                    x += 1;
                }
                self.add_portal(first, second, &run);
            }
        }
    }

    fn vertical_pair(
        &self,
        terrain: &dyn Terrain,
        points: &[Option<Point>],
        x: u16,
        y: u16,
    ) -> Option<(Point, Point)> {
        let left = points[self.index(x, y)]?;
        points[self.index(x + 1, y)]?;
        let right = step_allowed(terrain, left, Direction::East)?;
        let left = step_allowed(terrain, right, Direction::West)?;
        (left.x == x && left.y == y && right.x == x + 1 && right.y == y).then_some((left, right))
    }

    fn horizontal_pair(
        &self,
        terrain: &dyn Terrain,
        points: &[Option<Point>],
        x: u16,
        y: u16,
    ) -> Option<(Point, Point)> {
        let top = points[self.index(x, y)]?;
        points[self.index(x, y + 1)]?;
        let bottom = step_allowed(terrain, top, Direction::South)?;
        let top = step_allowed(terrain, bottom, Direction::North)?;
        (top.x == x && top.y == y && bottom.x == x && bottom.y == y + 1).then_some((top, bottom))
    }

    fn add_portal(&mut self, first: RegionId, second: RegionId, run: &[(Point, Point)]) {
        let ids: Vec<_> = match run.len() {
            0 => return,
            1..WIDE_PORTAL => vec![(run.len() - 1) / 2],
            _ => vec![0, run.len() - 1],
        };
        for index in ids {
            let first_id = self.add_node(first, run[index].0);
            let second_id = self.add_node(second, run[index].1);
            self.add_edge(first_id, second_id, 1);
            self.add_edge(second_id, first_id, 1);
        }
    }

    fn add_node(&mut self, region: RegionId, point: Point) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node { point, region });
        self.edges.push(Vec::new());
        self.region_nodes[region.0].push(id);
        id
    }

    fn add_edge(&mut self, from: NodeId, to: NodeId, cost: u32) {
        self.edges[from.0].push(Edge { to, cost });
    }

    fn intra_edges(&mut self, terrain: &dyn Terrain) {
        for region in 0..self.regions.len() {
            let nodes = self.region_nodes[region].clone();
            let region = self.regions[region];
            let local = InRegion { terrain, region };
            let budget = usize::from(region.width) * usize::from(region.height);
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

    fn abstract_path(
        &self,
        terrain: &dyn Terrain,
        from: Point,
        to: Point,
        forbidden: &[bool],
    ) -> Option<Vec<NodeId>> {
        let from_region = self.region_at(from)?;
        let to_region = self.region_at(to)?;
        let source = self.local_costs(terrain, from_region, from, forbidden, false);
        let target = self.local_costs(terrain, to_region, to, forbidden, true);
        if source.is_empty() || target.is_empty() {
            return None;
        }
        let start = self.nodes.len();
        let goal = start + 1;
        let mut cost = vec![u32::MAX; goal + 1];
        let mut parent = vec![None; goal + 1];
        let mut target_cost = vec![None; self.nodes.len()];
        for (node, cost) in target {
            target_cost[node.0] = Some(cost);
        }
        let mut open = BinaryHeap::new();
        cost[start] = 0;
        open.push(Reverse((distance(from, to), 0, start)));
        while let Some(Reverse((_f, here_cost, here))) = open.pop() {
            if here_cost != cost[here] {
                continue;
            }
            if here == goal {
                break;
            }
            let mut relax = |next: usize, edge_cost: u32, point: Point| {
                let next_cost = here_cost.saturating_add(edge_cost);
                if next_cost < cost[next] {
                    cost[next] = next_cost;
                    parent[next] = Some(here);
                    open.push(Reverse((next_cost + distance(point, to), next_cost, next)));
                }
            };
            if here == start {
                for &(node, edge_cost) in &source {
                    relax(node.0, edge_cost, self.nodes[node.0].point);
                }
                continue;
            }
            for edge in &self.edges[here] {
                if !forbidden[edge.to.0] {
                    relax(edge.to.0, edge.cost, self.nodes[edge.to.0].point);
                }
            }
            if let Some(edge_cost) = target_cost[here] {
                relax(goal, edge_cost, to);
            }
        }
        parent[goal]?;
        let mut path = Vec::new();
        let mut here = goal;
        while let Some(previous) = parent[here] {
            here = previous;
            if here < self.nodes.len() {
                path.push(NodeId(here));
            }
        }
        path.reverse();
        Some(path)
    }

    fn local_costs(
        &self,
        terrain: &dyn Terrain,
        region_id: RegionId,
        endpoint: Point,
        forbidden: &[bool],
        toward_endpoint: bool,
    ) -> Vec<(NodeId, u32)> {
        let region = self.regions[region_id.0];
        let local = InRegion { terrain, region };
        let budget = usize::from(region.width) * usize::from(region.height);
        self.region_nodes[region_id.0]
            .iter()
            .copied()
            .filter(|node| !forbidden[node.0])
            .filter_map(|node| {
                let (from, to) = match toward_endpoint {
                    true => (self.nodes[node.0].point, endpoint),
                    false => (endpoint, self.nodes[node.0].point),
                };
                find_path(&local, from, to, budget).map(|route| (node, route.len() as u32))
            })
            .collect()
    }

    fn refine(
        &self,
        terrain: &dyn Terrain,
        from: Point,
        to: Point,
        nodes: &[NodeId],
        budget: usize,
    ) -> Result<Vec<Direction>, NodeId> {
        let mut route = Vec::new();
        let mut at = from;
        let mut region = self
            .region_at(from)
            .expect("the query was checked before refinement");
        for &node in nodes {
            let next = self.nodes[node.0];
            let segment = match next.region == region {
                true => region_route(terrain, self.regions[region.0], at, next.point, budget),
                false => cross_portal(terrain, at, next.point),
            };
            let Some(segment) = segment else {
                return Err(node);
            };
            let Some(next_at) = append(terrain, at, &segment, &mut route) else {
                return Err(node);
            };
            at = next_at;
            region = next.region;
        }
        let last = *nodes
            .last()
            .expect("different regions always need graph transitions");
        let Some(segment) = region_route(terrain, self.regions[region.0], at, to, budget) else {
            return Err(last);
        };
        append(terrain, at, &segment, &mut route).ok_or(last)?;
        Ok(route)
    }

    fn forbid_portal(&self, node: NodeId, forbidden: &mut [bool]) {
        forbidden[node.0] = true;
        for edge in &self.edges[node.0] {
            if edge.cost == 1 {
                forbidden[edge.to.0] = true;
            }
        }
    }
}

fn distance(from: Point, to: Point) -> u32 {
    i32::from(from.x)
        .abs_diff(i32::from(to.x))
        .max(i32::from(from.y).abs_diff(i32::from(to.y)))
}

/// Refine a route proposed by a static navigation graph through live terrain.
#[must_use]
pub fn find_long_path(
    guide: &dyn Terrain,
    terrain: &dyn Terrain,
    graph: &NavigationGraph,
    from: Point,
    to: Point,
    budget: usize,
) -> Option<Vec<Direction>> {
    const LIVE_REROUTES: usize = 8;
    let from_region = graph.region_at(from)?;
    let to_region = graph.region_at(to)?;
    if from_region == to_region {
        return region_route(terrain, graph.regions[from_region.0], from, to, budget);
    }
    let mut forbidden = vec![false; graph.nodes.len()];
    for _ in 0..=LIVE_REROUTES {
        let path = graph.abstract_path(guide, from, to, &forbidden)?;
        match graph.refine(terrain, from, to, &path, budget) {
            Ok(route) => return Some(route),
            Err(node) if !forbidden[node.0] => graph.forbid_portal(node, &mut forbidden),
            Err(_) => return None,
        }
    }
    None
}

fn cross_portal(terrain: &dyn Terrain, from: Point, to: Point) -> Option<Vec<Direction>> {
    let direction = match (to.x.cmp(&from.x), to.y.cmp(&from.y)) {
        (std::cmp::Ordering::Greater, std::cmp::Ordering::Equal) => Direction::East,
        (std::cmp::Ordering::Less, std::cmp::Ordering::Equal) => Direction::West,
        (std::cmp::Ordering::Equal, std::cmp::Ordering::Greater) => Direction::South,
        (std::cmp::Ordering::Equal, std::cmp::Ordering::Less) => Direction::North,
        _ => return None,
    };
    step_allowed(terrain, from, direction)
        .filter(|landing| landing.x == to.x && landing.y == to.y)
        .map(|_| vec![direction])
}

fn region_route(
    terrain: &dyn Terrain,
    region: Region,
    from: Point,
    to: Point,
    budget: usize,
) -> Option<Vec<Direction>> {
    let local = InRegion { terrain, region };
    let hop = u16::try_from((budget / 2).max(1)).unwrap_or(u16::MAX);
    let mut route = Vec::new();
    let mut at = from;
    while distance(at, to) > u32::from(hop) {
        // Aim at the real destination and keep the closest result when the
        // bounded search runs out. A synthetic point exactly `hop` tiles away
        // can itself be a tree, which must not make a whole forest unroutable.
        let segment = find_path_toward(&local, at, to, budget)?;
        at = append(terrain, at, &segment, &mut route)?;
    }
    let segment = find_path(&local, at, to, budget)?;
    append(terrain, at, &segment, &mut route)?;
    Some(route)
}

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

    #[derive(Clone, Default)]
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

    fn end(terrain: &dyn Terrain, from: Point, route: &[Direction]) -> Point {
        route.iter().fold(from, |at, &direction| {
            step_allowed(terrain, at, direction).unwrap()
        })
    }

    #[test]
    fn an_open_facet_has_only_bounded_coarse_regions() {
        let terrain = Grid::open(704, 32);
        let graph = NavigationGraph::build(&terrain, 704, 32).unwrap();
        assert_eq!(graph.regions.len(), 22);
        assert_eq!(graph.nodes.len(), 84);
        let from = Point::new(1, 1, 0);
        let to = Point::new(702, 30, 0);
        let route = find_long_path(&terrain, &terrain, &graph, from, to, 100).unwrap();
        assert_eq!(end(&terrain, from, &route), to);
    }

    #[test]
    fn a_wall_opening_becomes_a_portal_between_derived_regions() {
        let mut terrain = Grid::open(96, 64);
        for y in 0..64 {
            if y != 40 {
                terrain.block(48, y);
            }
        }
        let graph = NavigationGraph::build(&terrain, 96, 64).unwrap();
        let from = Point::new(2, 2, 0);
        let to = Point::new(93, 2, 0);
        let route = find_long_path(&terrain, &terrain, &graph, from, to, 100).unwrap();
        assert_eq!(end(&terrain, from, &route), to);
        let mut at = from;
        for direction in route {
            at = step_allowed(&terrain, at, direction).unwrap();
            assert!(at.x != 48 || at.y == 40);
        }
    }

    #[test]
    fn a_forest_does_not_put_portals_around_every_tree() {
        let mut terrain = Grid::open(128, 128);
        for y in (4..124).step_by(4) {
            for x in (4..124).step_by(4) {
                terrain.block(x, y);
            }
        }
        let graph = NavigationGraph::build(&terrain, 128, 128).unwrap();
        assert_eq!(graph.regions.len(), 16);
        assert!(
            graph.nodes.len() < 500,
            "{} nodes for 900 trees",
            graph.nodes.len()
        );
        let from = Point::new(1, 1, 0);
        let to = Point::new(126, 126, 0);
        let route = find_long_path(&terrain, &terrain, &graph, from, to, 600).unwrap();
        assert_eq!(end(&terrain, from, &route), to);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// The graph may choose a different corridor, but it must agree with
        /// exhaustive A* about whether the two fixed endpoints connect at all.
        #[test]
        fn randomized_static_maps_keep_a_star_reachability(
            blocked in prop::collection::vec(any::<bool>(), 20 * 14),
        ) {
            let mut terrain = Grid::open(20, 14);
            for (index, blocked) in blocked.into_iter().enumerate() {
                let x = (index % 20) as u16;
                let y = (index / 20) as u16;
                if blocked && (x, y) != (1, 1) && (x, y) != (18, 12) {
                    terrain.block(x, y);
                }
            }
            let from = Point::new(1, 1, 0);
            let to = Point::new(18, 12, 0);
            let exact = find_path(&terrain, from, to, 20 * 14);
            let graph = NavigationGraph::build(&terrain, 20, 14).unwrap();
            let route = find_long_path(&terrain, &terrain, &graph, from, to, 20 * 14);
            prop_assert_eq!(route.is_some(), exact.is_some());
            if let Some(route) = route {
                prop_assert_eq!(end(&terrain, from, &route), to);
            }
        }
    }
}
