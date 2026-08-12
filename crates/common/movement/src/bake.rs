//! Stable, validated files containing an already-built [`NavigationGraph`].

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use openshard_protocol::world::{Facet, Point};

use crate::NavigationGraph;
use crate::navigation::{Edge, Node, NodeId, Region, RegionId};

const MAGIC: &[u8; 8] = b"OSNAV\0\r\n";
const FORMAT_VERSION: u32 = 1;
/// Increment whenever graph construction or static movement semantics change.
pub const ROUTING_VERSION: u32 = 3;
const MAX_COLLECTION: usize = 100_000_000;

/// Metadata for one input selected by the client-file loader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputStamp {
    pub name: String,
    pub bytes: u64,
    pub modified_ns: u128,
}

/// Everything cheap to inspect that identifies graph-producing inputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub facet: Facet,
    pub routing_version: u32,
    pub inputs: Vec<InputStamp>,
}

/// Why a baked graph cannot be used.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Missing { path: PathBuf },
    Io { path: PathBuf, source: io::Error },
    Incompatible { path: PathBuf, reason: String },
    Stale { path: PathBuf, reason: String },
    Corrupt { path: PathBuf, reason: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { path } => write!(f, "navigation artifact {} does not exist", path.display()),
            Self::Io { path, source } => write!(f, "navigation artifact {}: {source}", path.display()),
            Self::Incompatible { path, reason } => write!(
                f,
                "navigation artifact {} is incompatible: {reason}",
                path.display()
            ),
            Self::Stale { path, reason } => {
                write!(f, "navigation artifact {} is stale: {reason}", path.display())
            }
            Self::Corrupt { path, reason } => {
                write!(f, "navigation artifact {} is corrupt: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Default destination, overridable for read-only installs.
pub fn artifact_path(client_dir: &Path, facet: Facet) -> PathBuf {
    std::env::var_os("OPENSHARD_NAVIGATION")
        .map(PathBuf::from)
        .unwrap_or_else(|| client_dir.join(format!("openshard-navigation-{}.bin", facet.0)))
}

/// Inspect exactly the files `Map::load_facet` selects, plus tile data.
pub fn stamp_of(client_dir: &Path, facet: Facet) -> Result<Stamp, Error> {
    let uop_name = format!("map{}LegacyMUL.uop", facet.0);
    let map_name = if client_dir.join(&uop_name).exists() {
        uop_name
    } else {
        format!("map{}.mul", facet.0)
    };
    let names = [
        map_name,
        format!("staidx{}.mul", facet.0),
        format!("statics{}.mul", facet.0),
        "tiledata.mul".into(),
    ];
    let mut inputs = Vec::with_capacity(names.len());
    for name in names {
        let path = client_dir.join(&name);
        let metadata = fs::metadata(&path).map_err(|source| io_error(path.clone(), source))?;
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        inputs.push(InputStamp {
            name,
            bytes: metadata.len(),
            modified_ns,
        });
    }
    Ok(Stamp {
        facet,
        routing_version: ROUTING_VERSION,
        inputs,
    })
}

/// Atomically write a complete artifact in the destination directory.
pub fn save(path: &Path, graph: &NavigationGraph, stamp: &Stamp) -> Result<u64, Error> {
    if stamp.routing_version != ROUTING_VERSION {
        return Err(Error::Incompatible {
            path: path.into(),
            reason: "writer received an old routing stamp".into(),
        });
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_name().and_then(|s| s.to_str()).unwrap_or("navigation");
    let mut attempt = 0u32;
    let (temp, file) = loop {
        let temp = parent.join(format!(".{stem}.{}.{}.tmp", std::process::id(), attempt));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => break (temp, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => attempt += 1,
            Err(source) => return Err(io_error(temp, source)),
        }
    };
    let result = (|| {
        let mut out = BufWriter::new(file);
        let hash = {
            let mut hashed = HashWriter {
                inner: &mut out,
                hash: FNV_OFFSET,
            };
            encode(&mut hashed, graph, stamp).map_err(|source| io_error(temp.clone(), source))?;
            hashed.hash
        };
        out.write_all(&hash.to_le_bytes())
            .map_err(|source| io_error(temp.clone(), source))?;
        out.flush().map_err(|source| io_error(temp.clone(), source))?;
        out.get_ref()
            .sync_all()
            .map_err(|source| io_error(temp.clone(), source))?;
        drop(out);
        fs::rename(&temp, path).map_err(|source| io_error(path.into(), source))?;
        File::open(parent)
            .and_then(|d| d.sync_all())
            .map_err(|source| io_error(parent.into(), source))?;
        fs::metadata(path)
            .map(|m| m.len())
            .map_err(|source| io_error(path.into(), source))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Read and validate an artifact without consulting terrain or pathfinding.
pub fn load(path: &Path, expected: &Stamp) -> Result<NavigationGraph, Error> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|source| io_error(path.into(), source))?
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(path.into(), source))?;
    if bytes.len() < 8 {
        return Err(corrupt(path, "truncated checksum"));
    }
    let payload_len = bytes.len() - 8;
    let recorded = u64::from_le_bytes(bytes[payload_len..].try_into().unwrap());
    if hash(&bytes[..payload_len]) != recorded {
        return Err(corrupt(path, "checksum mismatch"));
    }
    decode(path, &bytes[..payload_len], expected).map_err(|error| match error {
        Error::Corrupt { path: empty, reason } if empty.as_os_str().is_empty() => Error::Corrupt {
            path: path.into(),
            reason,
        },
        other => other,
    })
}

fn io_error(path: PathBuf, source: io::Error) -> Error {
    if source.kind() == io::ErrorKind::NotFound {
        Error::Missing { path }
    } else {
        Error::Io { path, source }
    }
}

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

struct HashWriter<W> {
    inner: W,
    hash: u64,
}

impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(bytes)?;
        self.hash = hash_continue(self.hash, &bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn hash_continue(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME);
    }
    hash
}

fn encode(mut w: impl Write, g: &NavigationGraph, stamp: &Stamp) -> io::Result<()> {
    w.write_all(MAGIC)?;
    put_u32(&mut w, FORMAT_VERSION)?;
    put_u32(&mut w, ROUTING_VERSION)?;
    w.write_all(&[stamp.facet.0, 0, 0, 0])?;
    put_u32(&mut w, g.width)?;
    put_u32(&mut w, g.height)?;
    put_u64(&mut w, stamp.inputs.len() as u64)?;
    for input in &stamp.inputs {
        put_u32(&mut w, input.name.len() as u32)?;
        w.write_all(input.name.as_bytes())?;
        put_u64(&mut w, input.bytes)?;
        w.write_all(&input.modified_ns.to_le_bytes())?;
    }
    put_u64(&mut w, g.regions.len() as u64)?;
    put_u64(&mut w, g.at.len() as u64)?;
    put_u64(&mut w, g.nodes.len() as u64)?;
    put_u64(&mut w, g.region_nodes.len() as u64)?;
    put_u64(&mut w, g.edges.len() as u64)?;
    for r in &g.regions {
        put_u16(&mut w, r.left)?;
        put_u16(&mut w, r.top)?;
        put_u16(&mut w, r.width)?;
        put_u16(&mut w, r.height)?;
    }
    for id in &g.at {
        put_u64(&mut w, id.map_or(u64::MAX, |id| id.0 as u64))?;
    }
    for n in &g.nodes {
        put_u16(&mut w, n.point.x)?;
        put_u16(&mut w, n.point.y)?;
        w.write_all(&[n.point.z as u8, 0, 0, 0])?;
        put_u64(&mut w, n.region.0 as u64)?;
    }
    for ids in &g.region_nodes {
        put_u64(&mut w, ids.len() as u64)?;
        for id in ids {
            put_u64(&mut w, id.0 as u64)?;
        }
    }
    for edges in &g.edges {
        put_u64(&mut w, edges.len() as u64)?;
        for e in edges {
            put_u64(&mut w, e.to.0 as u64)?;
            put_u32(&mut w, e.cost)?;
        }
    }
    Ok(())
}

fn decode(path: &Path, bytes: &[u8], expected: &Stamp) -> Result<NavigationGraph, Error> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(8)? != MAGIC {
        return Err(incompatible(path, "wrong magic"));
    }
    let format = r.u32()?;
    if format != FORMAT_VERSION {
        return Err(incompatible(
            path,
            format!("format version {format}, expected {FORMAT_VERSION}"),
        ));
    }
    let routing = r.u32()?;
    if routing != ROUTING_VERSION {
        return Err(stale(
            path,
            format!("routing version {routing}, expected {ROUTING_VERSION}"),
        ));
    }
    let facet = Facet(r.take(4)?[0]);
    let width = r.u32()?;
    let height = r.u32()?;
    if facet != expected.facet {
        return Err(incompatible(
            path,
            format!("facet {}, expected {}", facet.0, expected.facet.0),
        ));
    }
    if width == 0 || height == 0 || width > u16::MAX as u32 || height > u16::MAX as u32 {
        return Err(incompatible(
            path,
            format!("invalid map dimensions {width}x{height}"),
        ));
    }
    let count = r.count()?;
    let mut inputs = Vec::with_capacity(count);
    for _ in 0..count {
        let len = r.u32()? as usize;
        let name =
            String::from_utf8(r.take(len)?.to_vec()).map_err(|_| corrupt(path, "non-UTF-8 input name"))?;
        let bytes = r.u64()?;
        let modified_ns = u128::from_le_bytes(r.take(16)?.try_into().unwrap());
        inputs.push(InputStamp {
            name,
            bytes,
            modified_ns,
        });
    }
    let actual = Stamp {
        facet,
        routing_version: routing,
        inputs,
    };
    if &actual != expected {
        return Err(stale(path, "client-file metadata changed"));
    }
    let nr = r.count()?;
    let na = r.count()?;
    let nn = r.count()?;
    let nrn = r.count()?;
    let ne = r.count()?;
    let minimum = nr
        .checked_mul(8)
        .and_then(|n| n.checked_add(na.checked_mul(8)?))
        .and_then(|n| n.checked_add(nn.checked_mul(16)?))
        .and_then(|n| n.checked_add(nrn.checked_mul(8)?))
        .and_then(|n| n.checked_add(ne.checked_mul(8)?))
        .ok_or_else(|| corrupt(path, "collection sizes overflow"))?;
    if minimum > r.remaining() {
        return Err(corrupt(path, "collection sizes exceed the payload"));
    }
    let cells = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| corrupt(path, "dimension overflow"))?;
    if na != cells || nrn != nr || ne != nn {
        return Err(corrupt(path, "inconsistent collection lengths"));
    }
    let mut regions = Vec::with_capacity(nr);
    for _ in 0..nr {
        regions.push(Region {
            left: r.u16()?,
            top: r.u16()?,
            width: r.u16()?,
            height: r.u16()?,
        });
    }
    let mut at = Vec::with_capacity(na);
    for _ in 0..na {
        let v = r.u64()?;
        at.push(if v == u64::MAX {
            None
        } else {
            Some(RegionId(index(path, v, nr, "region")?))
        });
    }
    let mut nodes = Vec::with_capacity(nn);
    for _ in 0..nn {
        let x = r.u16()?;
        let y = r.u16()?;
        let z = r.take(4)?[0] as i8;
        let region = RegionId(index(path, r.u64()?, nr, "node region")?);
        nodes.push(Node {
            point: Point::new(x, y, z),
            region,
        });
    }
    let mut region_nodes = Vec::with_capacity(nrn);
    for _ in 0..nrn {
        let n = r.count()?;
        r.require_items(n, 8)?;
        let mut ids = Vec::with_capacity(n);
        for _ in 0..n {
            ids.push(NodeId(index(path, r.u64()?, nn, "node")?));
        }
        region_nodes.push(ids);
    }
    let mut edges = Vec::with_capacity(ne);
    for _ in 0..ne {
        let n = r.count()?;
        r.require_items(n, 12)?;
        let mut list = Vec::with_capacity(n);
        for _ in 0..n {
            list.push(Edge {
                to: NodeId(index(path, r.u64()?, nn, "edge target")?),
                cost: r.u32()?,
            });
        }
        edges.push(list);
    }
    if r.at != bytes.len() {
        return Err(corrupt(path, "trailing payload bytes"));
    }
    for (i, region) in regions.iter().enumerate() {
        if region.width == 0
            || region.height == 0
            || u32::from(region.left) + u32::from(region.width) > width
            || u32::from(region.top) + u32::from(region.height) > height
        {
            return Err(corrupt(path, format!("region {i} is outside the map")));
        }
    }
    Ok(NavigationGraph {
        width,
        height,
        regions,
        at,
        nodes,
        region_nodes,
        edges,
    })
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len() - self.at
    }

    fn require_items(&self, count: usize, bytes: usize) -> Result<(), Error> {
        if count
            .checked_mul(bytes)
            .is_some_and(|size| size <= self.remaining())
        {
            Ok(())
        } else {
            Err(Error::Corrupt {
                path: PathBuf::new(),
                reason: "collection size exceeds the payload".into(),
            })
        }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self
            .at
            .checked_add(n)
            .filter(|&e| e <= self.bytes.len())
            .ok_or_else(|| Error::Corrupt {
                path: PathBuf::new(),
                reason: "truncated payload".into(),
            })?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn count(&mut self) -> Result<usize, Error> {
        let n = usize::try_from(self.u64()?).map_err(|_| Error::Corrupt {
            path: PathBuf::new(),
            reason: "collection length overflow".into(),
        })?;
        if n > MAX_COLLECTION {
            Err(Error::Corrupt {
                path: PathBuf::new(),
                reason: "unreasonable collection length".into(),
            })
        } else {
            Ok(n)
        }
    }
}
fn index(path: &Path, n: u64, len: usize, what: &str) -> Result<usize, Error> {
    let n = usize::try_from(n).map_err(|_| corrupt(path, format!("invalid {what} index")))?;
    if n < len {
        Ok(n)
    } else {
        Err(corrupt(path, format!("{what} index {n} is out of range")))
    }
}
fn put_u16(w: &mut impl Write, n: u16) -> io::Result<()> {
    w.write_all(&n.to_le_bytes())
}
fn put_u32(w: &mut impl Write, n: u32) -> io::Result<()> {
    w.write_all(&n.to_le_bytes())
}
fn put_u64(w: &mut impl Write, n: u64) -> io::Result<()> {
    w.write_all(&n.to_le_bytes())
}
fn incompatible(path: &Path, reason: impl Into<String>) -> Error {
    Error::Incompatible {
        path: path.into(),
        reason: reason.into(),
    }
}
fn stale(path: &Path, reason: impl Into<String>) -> Error {
    Error::Stale {
        path: path.into(),
        reason: reason.into(),
    }
}
fn corrupt(path: &Path, reason: impl Into<String>) -> Error {
    Error::Corrupt {
        path: path.into(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{Terrain, Tile, find_long_path};

    struct Grid {
        width: u16,
        height: u16,
        blocked: BTreeSet<(u16, u16)>,
    }

    impl Terrain for Grid {
        fn can_step(&self, _from: Point, to: Point) -> Option<Point> {
            (to.x < self.width && to.y < self.height && !self.blocked.contains(&(to.x, to.y))).then_some(to)
        }

        fn ground_z(&self, tile: Tile) -> Option<i8> {
            (tile.x < self.width && tile.y < self.height).then_some(0)
        }
    }

    fn stamp() -> Stamp {
        Stamp {
            facet: Facet(0),
            routing_version: ROUTING_VERSION,
            inputs: vec![InputStamp {
                name: "map0.mul".into(),
                bytes: 42,
                modified_ns: 7,
            }],
        }
    }
    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("openshard-nav-{}-{name}", std::process::id()))
    }
    #[test]
    fn round_trip_and_route_parity() {
        let mut blocked = BTreeSet::new();
        for y in 0..16 {
            if y != 11 {
                blocked.insert((12, y));
            }
        }
        let terrain = Grid {
            width: 24,
            height: 16,
            blocked,
        };
        let graph = NavigationGraph::build(&terrain, 24, 16).unwrap();
        let path = temp("round.bin");
        let s = stamp();
        save(&path, &graph, &s).unwrap();
        let loaded = load(&path, &s).unwrap();
        assert_eq!(loaded, graph);
        let from = Point::new(2, 2, 0);
        let to = Point::new(21, 2, 0);
        assert_eq!(
            find_long_path(&terrain, &terrain, &graph, from, to, 100),
            find_long_path(&terrain, &terrain, &loaded, from, to, 100),
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn incompatible_stale_and_corrupt_files_are_distinct() {
        let terrain = Grid {
            width: 8,
            height: 8,
            blocked: BTreeSet::new(),
        };
        let graph = NavigationGraph::build(&terrain, 8, 8).unwrap();
        let path = temp("reject.bin");
        let s = stamp();
        save(&path, &graph, &s).unwrap();
        let original = fs::read(&path).unwrap();
        let resign = |data: &mut Vec<u8>| {
            let payload_len = data.len() - 8;
            let checksum = hash(&data[..payload_len]);
            data[payload_len..].copy_from_slice(&checksum.to_le_bytes());
        };

        let mut wrong = s.clone();
        wrong.inputs[0].bytes += 1;
        assert!(matches!(load(&path, &wrong), Err(Error::Stale { .. })));

        let mut data = original.clone();
        data[0] ^= 1;
        resign(&mut data);
        fs::write(&path, &data).unwrap();
        assert!(matches!(load(&path, &s), Err(Error::Incompatible { .. })));
        data = original.clone();
        data[8..12].copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
        resign(&mut data);
        fs::write(&path, &data).unwrap();
        assert!(matches!(load(&path, &s), Err(Error::Incompatible { .. })));
        data = original.clone();
        data[12..16].copy_from_slice(&(ROUTING_VERSION + 1).to_le_bytes());
        resign(&mut data);
        fs::write(&path, &data).unwrap();
        assert!(matches!(load(&path, &s), Err(Error::Stale { .. })));
        data = original.clone();
        data[16] = 1;
        resign(&mut data);
        fs::write(&path, &data).unwrap();
        assert!(matches!(load(&path, &s), Err(Error::Incompatible { .. })));
        data = original.clone();
        data[20..24].copy_from_slice(&0u32.to_le_bytes());
        resign(&mut data);
        fs::write(&path, &data).unwrap();
        assert!(matches!(load(&path, &s), Err(Error::Incompatible { .. })));
        data = original;
        data.truncate(data.len() - 3);
        fs::write(&path, &data).unwrap();
        assert!(matches!(load(&path, &s), Err(Error::Corrupt { .. })));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn an_absent_artifact_is_reported_as_absent() {
        let path = temp("absent.bin");
        let _ = fs::remove_file(&path);
        assert!(matches!(load(&path, &stamp()), Err(Error::Missing { .. })));
    }
}
