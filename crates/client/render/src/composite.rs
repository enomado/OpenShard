//! Cached, immutable map-block pictures for the far-zoom renderer.
//!
//! A composite is deliberately a *map* resource: its pixels are ground and
//! map statics only.  Server items, mobiles, effects, cursor/selection masks,
//! and UI have no field in [`CompositePixels`] and therefore cannot accidentally
//! become part of a cache entry.  They continue through their existing passes.
//!
//! The module does not decide *when* a block is rebuilt.  Session 2 Work 3
//! supplies that bounded, camera-prioritised queue.  It also does not write a
//! fake depth/G-buffer for one large quad; Work 4 owns that interleaving policy
//! for dynamic objects.  What is complete here is the durable texture cache and
//! its colour-only one-quad draw operation: producers can populate a block
//! asynchronously and a visible block can be drawn without rebuilding its
//! constituent ground/static quads.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use openshard_uofiles::map::BLOCK_SIZE;

use crate::blit::WORLD_FORMAT;
use crate::camera::{TILE_WIDTH, TileBounds};
use crate::geometry::Rect;
use crate::lod::BlockLod;

/// The fixed coordinate of one 8×8 map block.
///
/// This is a map-block address, not a tile coordinate: `(1, 0)` starts at tile
/// `(8, 0)`.  It stays independent of a [`Map`](openshard_uofiles::map::Map)
/// so a queue can hold requests while the map is being streamed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MapBlock {
    /// Block column.
    pub x: u16,
    /// Block row.
    pub y: u16,
}

impl MapBlock {
    /// The map block containing tile `(x, y)`.
    pub const fn containing_tile(x: u16, y: u16) -> Self {
        Self {
            x: x / BLOCK_SIZE as u16,
            y: y / BLOCK_SIZE as u16,
        }
    }

    /// The top-left tile of this block.
    pub const fn first_tile(self) -> (u16, u16) {
        (self.x * BLOCK_SIZE as u16, self.y * BLOCK_SIZE as u16)
    }
}

/// An inclusive rectangle of map blocks.
///
/// This is the queue's cell range.  It is deliberately separate from
/// [`TileBounds`]: the camera and streaming code work in tiles, while a
/// composite request has exactly one 8×8 map block as its unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapBlockBounds {
    /// Lowest block column, inclusive.
    pub min_x: u16,
    /// Highest block column, inclusive.
    pub max_x: u16,
    /// Lowest block row, inclusive.
    pub min_y: u16,
    /// Highest block row, inclusive.
    pub max_y: u16,
}

impl MapBlockBounds {
    /// Convert camera tile coverage to actual map blocks, clipping off-map
    /// camera slack before it can become a request.
    pub fn from_tiles(bounds: TileBounds, map_width: u32, map_height: u32) -> Option<Self> {
        let (xs, ys) = bounds.clamp_to(map_width, map_height)?;
        Some(Self {
            min_x: *xs.start() / BLOCK_SIZE as u16,
            max_x: *xs.end() / BLOCK_SIZE as u16,
            min_y: *ys.start() / BLOCK_SIZE as u16,
            max_y: *ys.end() / BLOCK_SIZE as u16,
        })
    }

    /// Number of blocks across, inclusive.
    pub const fn width(self) -> u16 {
        self.max_x - self.min_x + 1
    }

    /// Number of blocks down, inclusive.
    pub const fn height(self) -> u16 {
        self.max_y - self.min_y + 1
    }

    fn centre(self) -> (i32, i32) {
        (
            (i32::from(self.min_x) + i32::from(self.max_x)) / 2,
            (i32::from(self.min_y) + i32::from(self.max_y)) / 2,
        )
    }

    /// Iterate every block in deterministic row-major order.
    pub fn blocks(self) -> impl Iterator<Item = MapBlock> {
        (self.min_y..=self.max_y).flat_map(move |y| (self.min_x..=self.max_x).map(move |x| MapBlock { x, y }))
    }

    fn contains(self, block: MapBlock) -> bool {
        (self.min_x..=self.max_x).contains(&block.x) && (self.min_y..=self.max_y).contains(&block.y)
    }

    /// The rectangle protected from cache eviction while `self` is visible.
    ///
    /// This is deliberately expressed in map blocks rather than pixels.  A
    /// small pan must not immediately discard a just-left composite only to
    /// queue and upload it again on the next pan back.
    pub fn expanded_by(self, margin: u16) -> Self {
        Self {
            min_x: self.min_x.saturating_sub(margin),
            max_x: self.max_x.saturating_add(margin),
            min_y: self.min_y.saturating_sub(margin),
            max_y: self.max_y.saturating_add(margin),
        }
    }

    /// One viewport-sized rectangle immediately in the direction from `was`.
    ///
    /// The result is clamped by `map`; a pan with no block-level movement has
    /// no ahead work.  This leaves the full currently visible rectangle ahead
    /// of tiny one-block pans, which is both deterministic and enough time for
    /// a bounded worker to catch up before the camera arrives.
    fn ahead_of(self, was: Self, map: Self) -> Option<Self> {
        let (old_x, old_y) = was.centre();
        let (new_x, new_y) = self.centre();
        let dx = (new_x - old_x).signum();
        let dy = (new_y - old_y).signum();
        if dx == 0 && dy == 0 {
            return None;
        }
        let shift_x = dx * i32::from(self.width());
        let shift_y = dy * i32::from(self.height());
        let min_x = (i32::from(self.min_x) + shift_x).clamp(i32::from(map.min_x), i32::from(map.max_x));
        let max_x = (i32::from(self.max_x) + shift_x).clamp(i32::from(map.min_x), i32::from(map.max_x));
        let min_y = (i32::from(self.min_y) + shift_y).clamp(i32::from(map.min_y), i32::from(map.max_y));
        let max_y = (i32::from(self.max_y) + shift_y).clamp(i32::from(map.min_y), i32::from(map.max_y));
        (min_x <= max_x && min_y <= max_y).then_some(Self {
            min_x: min_x as u16,
            max_x: max_x as u16,
            min_y: min_y as u16,
            max_y: max_y as u16,
        })
    }
}

/// The two cached resolutions.  LOD 0 intentionally has no texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompositeTier {
    /// Two source pixels per cached texel.
    Lod1,
    /// Four source pixels per cached texel.
    Lod2,
}

impl CompositeTier {
    /// The composite tier corresponding to a selected block LOD.
    pub const fn from_lod(lod: BlockLod) -> Option<Self> {
        match lod {
            BlockLod::Lod0 => None,
            BlockLod::Lod1 => Some(Self::Lod1),
            BlockLod::Lod2 => Some(Self::Lod2),
        }
    }

    /// Source pixels represented by one cache texel in each direction.
    pub const fn source_pixels_per_texel(self) -> u32 {
        match self {
            Self::Lod1 => 2,
            Self::Lod2 => 4,
        }
    }
}

/// A revision of the immutable inputs to a composite.
///
/// A producer increments this for map/static mutation, art/atlas revision,
/// cutaway state, or output format changes.  Work 3 can then request only the
/// stale `(block, tier)` entries; no cache-wide synchronous rebuild is implied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImmutableRevision(pub u64);

/// The full identity of one cached image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CompositeKey {
    /// The map block pictured by the texture.
    pub block: MapBlock,
    /// The cache's intentional sampling resolution.
    pub tier: CompositeTier,
    /// Immutable source revision used to produce its pixels.
    pub revision: ImmutableRevision,
}

/// Dimensions of one already-rasterised composite image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompositeSize {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
}

impl CompositeSize {
    /// A non-empty texture extent.
    pub const fn new(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            None
        } else {
            Some(Self { width, height })
        }
    }

    /// A square extent for the ground diamond plus a caller-provided static
    /// overhang, downsampled exactly for `tier`.
    ///
    /// `overhang_source_pixels` is normally the largest map-static dimension
    /// known from the atlas.  Keeping it in the extent makes a tall tree at a
    /// block edge part of exactly one cached image instead of being clipped or
    /// forcing its neighbours to rebuild.
    pub const fn for_block(tier: CompositeTier, overhang_source_pixels: u32) -> Self {
        let source = BLOCK_SIZE * TILE_WIDTH as u32 + overhang_source_pixels * 2;
        let divisor = tier.source_pixels_per_texel();
        // ceil(source / divisor), preserving the right/bottom edge.
        let side = (source + divisor - 1) / divisor;
        Self {
            width: side,
            height: side,
        }
    }

    /// RGBA8 upload length, or `None` when an input has overflowed `usize`.
    pub fn rgba_bytes(self) -> Option<usize> {
        usize::try_from(self.width)
            .ok()?
            .checked_mul(usize::try_from(self.height).ok()?)?
            .checked_mul(4)
    }
}

/// The already composed RGBA8 image of immutable map data.
///
/// Construction verifies the exact texture byte length.  That check keeps a
/// failed or partial worker result from becoming a drawable cache entry.
#[derive(Clone, Debug, PartialEq)]
pub struct CompositePixels {
    size: CompositeSize,
    rgba: Vec<u8>,
    /// The cache starts colour-only while Work 2 is being built.  A composite
    /// is eligible to replace map geometry only once its producer supplied the
    /// deferred planes below; otherwise using it would leave dynamic sprites
    /// testing against a made-up depth/G-buffer.
    deferred: Option<DeferredPixels>,
}

/// The per-texel facts a cached map block must retain to participate in the
/// ordinary deferred world pass.
///
/// `ids` contains the normal `gbuffer::IDS_FORMAT` word except that producers
/// reserve the high bit of its row id for the cached-map route.  The eventual
/// blit branch reads `position` directly for that route, rather than indexing
/// the current frame's transient ground/static instance buffers.  `depth` is
/// the producer's depth value and `depth_base` is the camera tile depth it was
/// based on; a draw adjusts the value by the current camera base before writing
/// fragment depth.  This is deliberately source data, not a lossy screenshot.
#[derive(Clone, Debug, PartialEq)]
pub struct DeferredPixels {
    ids: Vec<u32>,
    position: Vec<f32>,
    normal: Vec<u32>,
    depth: Vec<f32>,
    depth_base: i32,
}

impl DeferredPixels {
    /// Validate the four exact per-texel planes from a completed producer.
    pub fn new(
        size: CompositeSize,
        ids: Vec<u32>,
        position: Vec<f32>,
        normal: Vec<u32>,
        depth: Vec<f32>,
        depth_base: i32,
    ) -> Option<Self> {
        let texels = size.width.checked_mul(size.height)? as usize;
        (ids.len() == texels
            && position.len() == texels.checked_mul(4)?
            && normal.len() == texels
            && depth.len() == texels)
            .then_some(Self {
                ids,
                position,
                normal,
                depth,
                depth_base,
            })
    }

    pub fn ids(&self) -> &[u32] {
        &self.ids
    }
    pub fn position(&self) -> &[f32] {
        &self.position
    }
    pub fn normal(&self) -> &[u32] {
        &self.normal
    }
    pub fn depth(&self) -> &[f32] {
        &self.depth
    }
    pub const fn depth_base(&self) -> i32 {
        self.depth_base
    }
}

impl CompositePixels {
    /// Validate one RGBA8 composite result.
    pub fn new(size: CompositeSize, rgba: Vec<u8>) -> Option<Self> {
        (rgba.len() == size.rgba_bytes()?).then_some(Self {
            size,
            rgba,
            deferred: None,
        })
    }

    /// Texture dimensions.
    pub const fn size(&self) -> CompositeSize {
        self.size
    }

    /// Pixels in row-major RGBA8 order.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Attach the deferred facts produced from the same immutable source.
    /// A size mismatch is impossible because [`DeferredPixels::new`] validates
    /// it against the exact size passed here, but taking the size again keeps a
    /// caller from pairing two unrelated completed jobs by accident.
    pub fn with_deferred(mut self, deferred: DeferredPixels) -> Option<Self> {
        let texels = self.size.width.checked_mul(self.size.height)? as usize;
        (deferred.ids.len() == texels).then(|| {
            self.deferred = Some(deferred);
            self
        })
    }

    /// Deferred data makes this a candidate for geometry replacement.  Plain
    /// RGBA work remains drawable only as a diagnostic overlay, never as the
    /// authoritative map representation beneath mobiles or server items.
    pub fn deferred(&self) -> Option<&DeferredPixels> {
        self.deferred.as_ref()
    }
}

/// A GPU-resident cached composite.
#[derive(Debug)]
pub struct CompositeTexture {
    key: CompositeKey,
    /// CPU pixels are retained for worker-produced entries.  GPU captures do
    /// not read the image back merely to upload it again, so they have no CPU
    /// copy here.
    pixels: Option<CompositePixels>,
    size: CompositeSize,
    depth_base: i32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    deferred: Option<DeferredTextures>,
    /// Monotonic cache-local use stamp.  It uses interior mutability so the
    /// hot rendering lookup can remain `&self` and still feed the LRU policy.
    last_used: Cell<u64>,
}

/// GPU planes for a completed deferred composite.  Keeping the owning textures
/// beside their views makes this a real cache entry rather than a frame-local
/// bind group with dangling sources.
#[derive(Debug)]
struct DeferredTextures {
    _ids: wgpu::Texture,
    ids_view: wgpu::TextureView,
    _position: wgpu::Texture,
    position_view: wgpu::TextureView,
    _normal: wgpu::Texture,
    normal_view: wgpu::TextureView,
    _depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
}

impl CompositeTexture {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, key: CompositeKey, pixels: CompositePixels) -> Self {
        let size = pixels.size();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("map block composite"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: WORLD_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels.rgba(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * size.width),
                rows_per_image: Some(size.height),
            },
            wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let deferred = pixels
            .deferred()
            .map(|planes| DeferredTextures::new(device, queue, size, planes));
        Self {
            key,
            size,
            depth_base: pixels.deferred().map_or(0, DeferredPixels::depth_base),
            pixels: Some(pixels),
            texture,
            view,
            deferred,
            last_used: Cell::new(0),
        }
    }

    /// Allocate an entry whose planes are filled by a GPU copy from the
    /// map-only portion of a normal frame.  This deliberately never maps a
    /// buffer: a queue job becomes useful on a later frame without inserting a
    /// CPU readback stall between the source draw and the cache upload.
    fn capture(device: &wgpu::Device, key: CompositeKey, size: CompositeSize, depth_base: i32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("captured map block composite"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: WORLD_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            key,
            pixels: None,
            size,
            depth_base,
            texture,
            view,
            deferred: Some(DeferredTextures::capture(device, size)),
            last_used: Cell::new(0),
        }
    }

    /// Immutable identity of the image.
    pub const fn key(&self) -> CompositeKey {
        self.key
    }

    /// Camera depth base used when this entry's stored depths were written.
    pub const fn depth_base(&self) -> i32 {
        self.depth_base
    }

    /// Whether this entry owns all planes required to replace map geometry.
    pub fn has_deferred(&self) -> bool {
        self.deferred.is_some()
    }

    /// Texture size and source pixels retained for deterministic replacement.
    pub fn pixels(&self) -> Option<&CompositePixels> {
        self.pixels.as_ref()
    }

    /// The texture view bound by [`CompositeRenderer`].
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    fn deferred_views(
        &self,
    ) -> Option<(
        &wgpu::TextureView,
        &wgpu::TextureView,
        &wgpu::TextureView,
        &wgpu::TextureView,
    )> {
        let planes = self.deferred.as_ref()?;
        Some((
            &planes.ids_view,
            &planes.position_view,
            &planes.normal_view,
            &planes.depth_view,
        ))
    }

    fn deferred_textures(&self) -> Option<(&wgpu::Texture, &wgpu::Texture, &wgpu::Texture, &wgpu::Texture)> {
        let planes = self.deferred.as_ref()?;
        Some((&planes._ids, &planes._position, &planes._normal, &planes._depth))
    }

    /// The underlying texture, for diagnostics and GPU-memory accounting.
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// GPU bytes retained by this composite.
    pub fn gpu_bytes(&self) -> u64 {
        let rgba = self.size.rgba_bytes().unwrap_or(0) as u64;
        rgba + self.deferred.as_ref().map_or(0, |_| rgba * 7)
    }

    fn mark_used(&self, stamp: u64) {
        self.last_used.set(stamp);
    }

    fn last_used(&self) -> u64 {
        self.last_used.get()
    }
}

impl DeferredTextures {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, size: CompositeSize, planes: &DeferredPixels) -> Self {
        let texture = |label, format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let ids = texture("map composite ids", crate::gbuffer::IDS_FORMAT);
        let position = texture("map composite position", crate::gbuffer::POSITION_FORMAT);
        let normal = texture("map composite normal", crate::gbuffer::NORMAL_FORMAT);
        let depth = texture("map composite depth", wgpu::TextureFormat::R32Float);
        let write = |texture: &wgpu::Texture, bytes: &[u8], bytes_per_texel: u32| {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size.width * bytes_per_texel),
                    rows_per_image: Some(size.height),
                },
                wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                },
            );
        };
        let words = |values: &[u32]| {
            values
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>()
        };
        let floats = |values: &[f32]| {
            values
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>()
        };
        write(&ids, &words(planes.ids()), 4);
        write(&position, &floats(planes.position()), 16);
        write(&normal, &words(planes.normal()), 4);
        write(&depth, &floats(planes.depth()), 4);
        Self {
            ids_view: ids.create_view(&wgpu::TextureViewDescriptor::default()),
            _ids: ids,
            position_view: position.create_view(&wgpu::TextureViewDescriptor::default()),
            _position: position,
            normal_view: normal.create_view(&wgpu::TextureViewDescriptor::default()),
            _normal: normal,
            depth_view: depth.create_view(&wgpu::TextureViewDescriptor::default()),
            _depth: depth,
        }
    }

    /// Empty GPU planes for a captured entry.  Colour, ids, position and
    /// normal arrive through `copy_texture_to_texture`; depth is rasterised by
    /// [`CompositeRenderer::capture`] because a depth attachment cannot be
    /// copied into the `R32Float` sampling plane used by the restore shader.
    fn capture(device: &wgpu::Device, size: CompositeSize) -> Self {
        let texture = |label, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let sampled = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT;
        let ids = texture("captured map composite ids", crate::gbuffer::IDS_FORMAT, sampled);
        let position = texture(
            "captured map composite position",
            crate::gbuffer::POSITION_FORMAT,
            sampled,
        );
        let normal = texture(
            "captured map composite normal",
            crate::gbuffer::NORMAL_FORMAT,
            sampled,
        );
        let depth = texture(
            "captured map composite depth",
            wgpu::TextureFormat::R32Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        Self {
            ids_view: ids.create_view(&wgpu::TextureViewDescriptor::default()),
            _ids: ids,
            position_view: position.create_view(&wgpu::TextureViewDescriptor::default()),
            _position: position,
            normal_view: normal.create_view(&wgpu::TextureViewDescriptor::default()),
            _normal: normal,
            depth_view: depth.create_view(&wgpu::TextureViewDescriptor::default()),
            _depth: depth,
        }
    }
}

/// The hard cache retention policy.
///
/// The default is 128 MiB for the colour and deferred planes together.  It is
/// independent of the static atlas's 128 MiB page limit: a deferred composite
/// has eight RGBA-sized planes, so conflating the two would silently retain
/// much more GPU memory than its number suggests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompositeCacheLimits {
    /// Maximum bytes retained for entries outside the protected viewport
    /// margin.  Visible and near-visible entries are never evicted merely to
    /// satisfy this limit; they are the working set, not the cache tail.
    pub max_gpu_bytes: u64,
    /// Number of map blocks kept on every side of the visible rectangle.
    pub viewport_margin_blocks: u16,
}

impl CompositeCacheLimits {
    /// The shipped 128 MiB tail budget and one-block pan hysteresis margin.
    pub const DEFAULT_MAX_GPU_BYTES: u64 = 128 * 1024 * 1024;
    pub const DEFAULT_VIEWPORT_MARGIN_BLOCKS: u16 = 1;

    /// A non-zero tail budget and its viewport hysteresis margin.
    pub const fn new(max_gpu_bytes: u64, viewport_margin_blocks: u16) -> Option<Self> {
        if max_gpu_bytes == 0 {
            None
        } else {
            Some(Self {
                max_gpu_bytes,
                viewport_margin_blocks,
            })
        }
    }
}

impl Default for CompositeCacheLimits {
    fn default() -> Self {
        Self {
            max_gpu_bytes: Self::DEFAULT_MAX_GPU_BYTES,
            viewport_margin_blocks: Self::DEFAULT_VIEWPORT_MARGIN_BLOCKS,
        }
    }
}

/// What one cache-maintenance pass discarded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompositeEviction {
    /// Entries discarded from the least-recently-used tail.
    pub entries: usize,
    /// GPU bytes released by those entries.
    pub freed_gpu_bytes: u64,
    /// Bytes still retained after the pass.
    pub retained_gpu_bytes: u64,
    /// Bytes above the configured tail budget that are protected by the
    /// visible viewport margin.  This is reported rather than evicting a
    /// near-visible image and defeating the hysteresis guarantee.
    pub protected_over_budget_bytes: u64,
}

/// A cache of immutable block pictures with a bounded LRU tail.
#[derive(Debug)]
pub struct CompositeCache {
    entries: BTreeMap<CompositeKey, CompositeTexture>,
    limits: CompositeCacheLimits,
    use_clock: Cell<u64>,
}

impl Default for CompositeCache {
    fn default() -> Self {
        Self::with_limits(CompositeCacheLimits::default())
    }
}

impl CompositeCache {
    /// Create a cache with an explicit GPU-tail budget.
    pub fn with_limits(limits: CompositeCacheLimits) -> Self {
        Self {
            entries: BTreeMap::new(),
            limits,
            use_clock: Cell::new(0),
        }
    }

    /// The configured cache retention policy.
    pub const fn limits(&self) -> CompositeCacheLimits {
        self.limits
    }

    /// Number of ready textures.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no texture has been produced yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// A ready composite for the exact immutable revision.
    pub fn get(&self, key: CompositeKey) -> Option<&CompositeTexture> {
        let entry = self.entries.get(&key)?;
        let stamp = self.use_clock.get().wrapping_add(1);
        self.use_clock.set(stamp);
        entry.mark_used(stamp);
        Some(entry)
    }

    /// The selected cached representation, or its immediate detailed fallback.
    ///
    /// A newly visible LOD 2 block may keep drawing a ready LOD 1 texture while
    /// its LOD 2 job waits.  A LOD 1 miss falls through to LOD 0 (`None`), so a
    /// caller keeps its established detailed geometry instead of composing a
    /// full map block synchronously in the camera frame.
    pub fn selected_or_more_detailed(
        &self,
        block: MapBlock,
        selected: BlockLod,
        revision: ImmutableRevision,
    ) -> Option<&CompositeTexture> {
        let tier = CompositeTier::from_lod(selected)?;
        let key = CompositeKey {
            block,
            tier,
            revision,
        };
        self.get(key).or_else(|| {
            let detailed = selected.next_more_detailed()?;
            let tier = CompositeTier::from_lod(detailed)?;
            self.get(CompositeKey {
                block,
                tier,
                revision,
            })
        })
    }

    /// Upload a completed immutable map composite.
    ///
    /// Replacing the exact same key is intentional: a worker may retry after a
    /// device loss, and revision equality says its source content still is the
    /// same.  Revision changes create a distinct entry so Work 3 can choose
    /// when the old image becomes unreachable rather than flashing a block.
    pub fn insert(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: CompositeKey,
        pixels: CompositePixels,
    ) -> &CompositeTexture {
        let composite = CompositeTexture::new(device, queue, key, pixels);
        self.entries.insert(key, composite);
        self.get(key).expect("the cache has just inserted this key")
    }

    /// Allocate a GPU-only capture of one already-drawn map-only rectangle.
    ///
    /// `source` belongs to the current frame and the entry becomes visible to
    /// the next one.  No caller can hand in dynamic attachment textures: the
    /// source is the world target at the exact point before server items and
    /// mobiles are rendered. [`CompositeRenderer`] fills the returned planes
    /// with a GPU resample in the same command encoder.
    fn capture(
        &mut self,
        device: &wgpu::Device,
        key: CompositeKey,
        source: CaptureSource<'_>,
    ) -> &CompositeTexture {
        let divisor = key.tier.source_pixels_per_texel();
        let size = CompositeSize::new(
            source.rect.width.div_ceil(divisor),
            source.rect.height.div_ceil(divisor),
        )
        .expect("a non-empty capture rectangle has a non-empty tier");
        let composite = CompositeTexture::capture(device, key, size, source.depth_base);
        self.entries.insert(key, composite);
        self.get(key).expect("the captured entry was just inserted")
    }

    /// Forget one exact entry.  This is intentionally narrow: mutation code
    /// can invalidate affected block/tier pairs without a global cache clear.
    pub fn remove(&mut self, key: CompositeKey) -> Option<CompositeTexture> {
        self.entries.remove(&key)
    }

    /// Forget every cached resolution and revision of one changed map block.
    ///
    /// A map/static mutation changes the source pixels for both cached LODs;
    /// keeping another revision around would make it too easy for a fallback
    /// lookup to show stale map state.
    pub fn invalidate_block(&mut self, block: MapBlock) -> usize {
        self.invalidate_matching(|key| key.block == block)
    }

    /// Forget selected cached tiers of one changed map block.
    pub fn invalidate_block_tiers(&mut self, block: MapBlock, tiers: &[CompositeTier]) -> usize {
        self.invalidate_matching(|key| key.block == block && tiers.contains(&key.tier))
    }

    /// Forget every cached resolution/revision in an affected block rectangle.
    pub fn invalidate_blocks(&mut self, blocks: MapBlockBounds) -> usize {
        self.invalidate_matching(|key| blocks.contains(key.block))
    }

    /// Forget selected cached tiers in an affected block rectangle.
    pub fn invalidate_block_tiers_in(&mut self, blocks: MapBlockBounds, tiers: &[CompositeTier]) -> usize {
        self.invalidate_matching(|key| blocks.contains(key.block) && tiers.contains(&key.tier))
    }

    /// Forget every entry whose immutable input changed globally, such as a
    /// world-output format change.  Callers should use the block variants for
    /// ordinary map/static or newly packed-art changes.
    pub fn clear(&mut self) -> usize {
        let removed = self.entries.len();
        self.entries.clear();
        removed
    }

    fn invalidate_matching(&mut self, mut stale: impl FnMut(&CompositeKey) -> bool) -> usize {
        let before = self.entries.len();
        self.entries.retain(|key, _| !stale(key));
        before - self.entries.len()
    }

    /// Enforce the configured GPU-tail budget by evicting LRU entries outside
    /// the viewport's hysteresis margin.  Call once per rendered frame after
    /// the cache's completed captures have been accepted.
    pub fn evict_lru_outside_viewport(&mut self, visible: Option<MapBlockBounds>) -> CompositeEviction {
        let protected = visible.map(|bounds| bounds.expanded_by(self.limits.viewport_margin_blocks));
        let mut retained = self.gpu_bytes();
        let mut result = CompositeEviction {
            retained_gpu_bytes: retained,
            ..CompositeEviction::default()
        };
        if retained <= self.limits.max_gpu_bytes {
            return result;
        }

        let mut candidates: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(key, entry)| {
                (!protected.is_some_and(|bounds| bounds.contains(key.block)))
                    .then_some((entry.last_used(), *key))
            })
            .collect();
        candidates.sort_unstable();
        for (_, key) in candidates {
            if retained <= self.limits.max_gpu_bytes {
                break;
            }
            let removed = self
                .entries
                .remove(&key)
                .expect("LRU candidate still belongs to the cache");
            let bytes = removed.gpu_bytes();
            retained = retained.saturating_sub(bytes);
            result.entries += 1;
            result.freed_gpu_bytes += bytes;
        }
        result.retained_gpu_bytes = retained;
        result.protected_over_budget_bytes = retained.saturating_sub(self.limits.max_gpu_bytes);
        result
    }

    /// Total retained RGBA8 texture bytes.
    pub fn gpu_bytes(&self) -> u64 {
        self.entries.values().map(CompositeTexture::gpu_bytes).sum()
    }
}

/// The immutable attachments captured at the map/dynamic boundary.
#[derive(Clone, Copy, Debug)]
pub struct CaptureSource<'a> {
    pub color: &'a wgpu::Texture,
    pub ids: &'a wgpu::Texture,
    pub position: &'a wgpu::Texture,
    pub normal: &'a wgpu::Texture,
    pub depth: &'a wgpu::TextureView,
    pub depth_base: i32,
    pub rect: crate::blit::ViewportRect,
}

/// Why one composite job is waiting.
///
/// The order is intentional: every visible block is dispatched before a block
/// merely predicted to enter the view.  The queue then sorts by distance and
/// stable key, so its output does not depend on `HashMap` iteration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompositePriority {
    /// The camera can see the block in this frame.
    Visible,
    /// The block is one viewport ahead of the camera's block-level motion.
    Ahead,
}

/// One bounded asynchronous composition request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompositeWork {
    /// The immutable image to build or refresh.
    pub key: CompositeKey,
    /// Why the work was scheduled.
    pub priority: CompositePriority,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct QueueOrder {
    priority: CompositePriority,
    distance: u32,
    key: CompositeKey,
}

/// The bounded map-cell queue shared by streamed map work and composites.
///
/// This value never composes pixels.  `take_for_frame` hands a small fixed
/// number of jobs to the background/idle producer, and `finished` marks a job
/// available for another revision.  That separation is the guarantee that a
/// newly exposed large block never becomes a synchronous camera-frame build.
#[derive(Debug)]
pub struct CompositeWorkQueue {
    max_pending: usize,
    builds_per_frame: usize,
    pending: BTreeMap<CompositeKey, QueueOrder>,
    in_flight: BTreeSet<CompositeKey>,
    previous_visible: Option<MapBlockBounds>,
}

impl Default for CompositeWorkQueue {
    fn default() -> Self {
        Self::new(128, 1).expect("the shipped composite queue limits are non-zero")
    }
}

impl CompositeWorkQueue {
    /// Construct a queue with explicit pending and per-frame bounds.
    pub fn new(max_pending: usize, builds_per_frame: usize) -> Option<Self> {
        (max_pending != 0 && builds_per_frame != 0).then_some(Self {
            max_pending,
            builds_per_frame,
            pending: BTreeMap::new(),
            in_flight: BTreeSet::new(),
            previous_visible: None,
        })
    }

    /// Requests waiting to be handed to a producer.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Requests a producer currently owns.
    pub fn in_flight_len(&self) -> usize {
        self.in_flight.len()
    }

    /// Enqueue the selected tier for every visible block, then one viewport of
    /// prefetch work in the camera's movement direction.
    ///
    /// `ready` is normally the composite cache's exact-key lookup.  It keeps a
    /// completed composite out of both the pending and in-flight sets, while a
    /// different immutable revision is naturally a new request.
    pub fn refresh(
        &mut self,
        visible: MapBlockBounds,
        map: MapBlockBounds,
        selected: BlockLod,
        revision: ImmutableRevision,
        mut ready: impl FnMut(CompositeKey) -> bool,
    ) {
        let Some(tier) = CompositeTier::from_lod(selected) else {
            self.pending.clear();
            self.previous_visible = Some(visible);
            return;
        };
        let centre = visible.centre();
        let ahead = self.previous_visible.and_then(|was| visible.ahead_of(was, map));
        // A camera can reverse before its prefetch has started.  Pending work
        // for the old direction is neither visible nor ahead now, so retaining
        // it would let stale prefetch starve the entered blocks.  In-flight
        // work is left to its producer: completion is cheap and cannot be
        // cancelled safely after it has begun.
        self.pending.retain(|key, _| {
            key.tier == tier
                && key.revision == revision
                && (visible.contains(key.block) || ahead.is_some_and(|bounds| bounds.contains(key.block)))
        });
        for block in visible.blocks() {
            self.request(
                block,
                tier,
                revision,
                CompositePriority::Visible,
                centre,
                &mut ready,
            );
        }
        if let Some(ahead) = ahead {
            for block in ahead.blocks() {
                self.request(
                    block,
                    tier,
                    revision,
                    CompositePriority::Ahead,
                    centre,
                    &mut ready,
                );
            }
        }
        self.previous_visible = Some(visible);
    }

    fn request(
        &mut self,
        block: MapBlock,
        tier: CompositeTier,
        revision: ImmutableRevision,
        priority: CompositePriority,
        centre: (i32, i32),
        ready: &mut impl FnMut(CompositeKey) -> bool,
    ) {
        let key = CompositeKey {
            block,
            tier,
            revision,
        };
        if ready(key) || self.in_flight.contains(&key) {
            return;
        }
        let distance =
            i32::abs(i32::from(block.x) - centre.0) as u32 + i32::abs(i32::from(block.y) - centre.1) as u32;
        let order = QueueOrder {
            priority,
            distance,
            key,
        };
        self.pending
            .entry(key)
            .and_modify(|was| *was = (*was).min(order))
            .or_insert(order);
        while self.pending.len() > self.max_pending {
            let drop = self
                .pending
                .values()
                .max()
                .expect("the queue is non-empty while enforcing its bound")
                .key;
            self.pending.remove(&drop);
        }
    }

    /// Gives at most the configured work budget to an asynchronous producer.
    ///
    /// Calling this does no rasterisation or upload.  A caller that has no
    /// idle/worker producer leaves the requests pending; it must not call a
    /// large compose operation from its camera frame to empty this queue.
    pub fn take_for_frame(&mut self) -> Vec<CompositeWork> {
        let mut ordered: Vec<_> = self.pending.values().copied().collect();
        ordered.sort();
        ordered.truncate(self.builds_per_frame);
        let mut work = Vec::with_capacity(ordered.len());
        for order in ordered {
            self.pending.remove(&order.key);
            self.in_flight.insert(order.key);
            work.push(CompositeWork {
                key: order.key,
                priority: order.priority,
            });
        }
        work
    }

    /// Releases an asynchronous job after its result has been accepted or
    /// discarded.  The next `refresh` can request a retry if its exact key is
    /// still not in the cache.
    pub fn finished(&mut self, key: CompositeKey) {
        self.in_flight.remove(&key);
    }

    /// Cancel all pending and dispatched work for one changed map block.
    ///
    /// Removing an in-flight key is intentional: a producer that completes
    /// after the mutation reaches [`finish_into_cache`](Self::finish_into_cache)
    /// or [`finish_capture`](Self::finish_capture), sees that its reservation
    /// is gone, and discards its stale result instead of reviving old pixels.
    pub fn invalidate_block(&mut self, block: MapBlock) -> usize {
        self.invalidate_matching(|key| key.block == block)
    }

    /// Cancel selected cached LOD jobs for one changed map block.
    pub fn invalidate_block_tiers(&mut self, block: MapBlock, tiers: &[CompositeTier]) -> usize {
        self.invalidate_matching(|key| key.block == block && tiers.contains(&key.tier))
    }

    /// Cancel all work in an affected map-block rectangle.
    pub fn invalidate_blocks(&mut self, blocks: MapBlockBounds) -> usize {
        self.invalidate_matching(|key| blocks.contains(key.block))
    }

    /// Cancel selected LOD work in an affected map-block rectangle.
    pub fn invalidate_block_tiers_in(&mut self, blocks: MapBlockBounds, tiers: &[CompositeTier]) -> usize {
        self.invalidate_matching(|key| blocks.contains(key.block) && tiers.contains(&key.tier))
    }

    /// Cancel every queued and dispatched job, for a global source change such
    /// as a world-output-format reconfiguration.
    pub fn clear(&mut self) -> usize {
        let removed = self.pending.len() + self.in_flight.len();
        self.pending.clear();
        self.in_flight.clear();
        removed
    }

    fn invalidate_matching(&mut self, mut stale: impl FnMut(&CompositeKey) -> bool) -> usize {
        let pending_before = self.pending.len();
        let flight_before = self.in_flight.len();
        self.pending.retain(|key, _| !stale(key));
        self.in_flight.retain(|key| !stale(key));
        pending_before - self.pending.len() + flight_before - self.in_flight.len()
    }

    /// Accept a completed asynchronous image into the cache and release its
    /// queue slot.
    ///
    /// The exact key must have been handed out by [`Self::take_for_frame`].
    /// This prevents a late result for a cancelled/stale request from quietly
    /// replacing a newer cache entry.  Rasterising `pixels` is deliberately
    /// outside this method: producers may use an idle worker or a streamed map
    /// cell budget, while this small upload is the one atomic completion step.
    pub fn finish_into_cache<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cache: &'a mut CompositeCache,
        key: CompositeKey,
        pixels: CompositePixels,
    ) -> Option<&'a CompositeTexture> {
        self.in_flight
            .remove(&key)
            .then(|| cache.insert(device, queue, key, pixels))
    }

    /// Complete one dispatched job by copying the immutable map portion of the
    /// current frame into GPU-resident cache planes.  This is intentionally a
    /// no-op for a key that was not dispatched: a late capture must not make a
    /// stale block authoritative.
    pub fn finish_capture<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        renderer: &mut CompositeRenderer,
        cache: &'a mut CompositeCache,
        key: CompositeKey,
        source: CaptureSource<'_>,
    ) -> Option<&'a CompositeTexture> {
        if !self.in_flight.remove(&key) {
            return None;
        }
        let captured = cache.capture(device, key, source);
        renderer.capture_planes(device, queue, encoder, source, captured);
        renderer.capture_depth(device, queue, encoder, source, captured);
        Some(captured)
    }
}

/// One cached image placed in the current world target.
#[derive(Clone, Copy, Debug)]
pub struct CompositeQuad<'a> {
    /// The cache image.  A caller obtains this only after a background producer
    /// has completed it; requesting or composing work does not happen here.
    pub texture: &'a CompositeTexture,
    /// The image's full screen-space rectangle in virtual target pixels.
    pub rect: Rect,
}

/// Draws each cached map block as one textured quad.
///
/// This pass writes only the colour image and uses source-over alpha blending.
/// The main world's G-buffer and depth are deliberately not guessed at a block
/// granularity; the next work item owns the policy that lets dynamic objects
/// interleave with cached map pixels.
#[derive(Debug)]
pub struct CompositeRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    deferred_pipeline: wgpu::RenderPipeline,
    deferred_layout: wgpu::BindGroupLayout,
    capture_depth_pipeline: wgpu::RenderPipeline,
    capture_depth_layout: wgpu::BindGroupLayout,
    capture_planes_pipeline: wgpu::RenderPipeline,
    capture_planes_layout: wgpu::BindGroupLayout,
    capture_uniform: wgpu::Buffer,
    viewport: wgpu::Buffer,
    quad: wgpu::Buffer,
    instances: wgpu::Buffer,
    capacity: u64,
    sampler: wgpu::Sampler,
}

fn write_capture_uniform(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    size: CompositeSize,
    source: crate::blit::ViewportRect,
) {
    let values = [
        size.width as f32,
        size.height as f32,
        source.x as f32,
        source.y as f32,
        source.width as f32,
        source.height as f32,
        0.0,
        0.0,
    ];
    let bytes: Vec<u8> = values.iter().flat_map(|value| value.to_le_bytes()).collect();
    queue.write_buffer(buffer, 0, &bytes);
}

impl CompositeRenderer {
    /// Create the colour-only cached-composite pass.
    pub fn new(device: &wgpu::Device) -> Self {
        let viewport = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("composite viewport"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map block composite"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("map block composite sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("map block composite"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(env!("OUT_DIR"), "/composite.wgsl")).into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("map block composite"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map block composite"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 8,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        }],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 16,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 0,
                            shader_location: 1,
                        }],
                    }),
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: WORLD_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let deferred_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map block deferred composite"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let deferred_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("map block deferred composite"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(env!("OUT_DIR"), "/composite_deferred.wgsl")).into(),
            ),
        });
        let deferred_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("map block deferred composite"),
            bind_group_layouts: &[Some(&deferred_layout)],
            immediate_size: 0,
        });
        let vertex_buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: 8,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                }],
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: 16,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &[wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 1,
                }],
            }),
        ];
        let deferred_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map block deferred composite"),
            layout: Some(&deferred_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &deferred_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &vertex_buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: &deferred_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: WORLD_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(crate::renderer::IDS_TARGET),
                    Some(crate::renderer::POSITION_TARGET),
                    Some(crate::renderer::NORMAL_TARGET),
                ],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(crate::renderer::depth_state()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let capture_depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map composite depth capture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let capture_depth_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("map composite depth capture"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(env!("OUT_DIR"), "/composite_capture_depth.wgsl")).into(),
            ),
        });
        let capture_depth_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("map composite depth capture"),
            bind_group_layouts: &[Some(&capture_depth_layout)],
            immediate_size: 0,
        });
        let capture_depth_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map composite depth capture"),
            layout: Some(&capture_depth_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &capture_depth_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 8,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    }],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &capture_depth_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R32Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let capture_planes_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map composite plane capture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let capture_planes_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("map composite plane capture"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(env!("OUT_DIR"), "/composite_capture_planes.wgsl")).into(),
            ),
        });
        let capture_planes_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("map composite plane capture"),
            bind_group_layouts: &[Some(&capture_planes_layout)],
            immediate_size: 0,
        });
        let capture_planes_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map composite plane capture"),
            layout: Some(&capture_planes_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &capture_planes_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 8,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    }],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &capture_planes_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: WORLD_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(crate::renderer::IDS_TARGET),
                    Some(crate::renderer::POSITION_TARGET),
                    Some(crate::renderer::NORMAL_TARGET),
                ],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let capture_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map composite depth capture"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let quad = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map block composite unit quad"),
            size: 4 * 2 * 4,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        let mut bytes = Vec::with_capacity(4 * 2 * 4);
        for value in [0.0_f32, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        quad.slice(..)
            .get_mapped_range_mut()
            .expect("a freshly mapped buffer has its whole range")
            .copy_from_slice(&bytes);
        quad.unmap();
        let instances = Self::instance_buffer(device, 1);
        Self {
            pipeline,
            layout,
            deferred_pipeline,
            deferred_layout,
            capture_depth_pipeline,
            capture_depth_layout,
            capture_planes_pipeline,
            capture_planes_layout,
            capture_uniform,
            viewport,
            quad,
            instances,
            capacity: 1,
            sampler,
        }
    }

    fn instance_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map block composite instances"),
            size: capacity * 16,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Draw all ready blocks as one quad each over an already-cleared colour
    /// target.  No rebuild, upload, or cache lookup occurs in this method.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        target_size: CompositeSize,
        blocks: &[CompositeQuad<'_>],
    ) {
        if blocks.is_empty() {
            return;
        }
        let mut viewport = Vec::with_capacity(16);
        for value in [target_size.width as f32, target_size.height as f32, 0.0, 0.0] {
            viewport.extend_from_slice(&value.to_le_bytes());
        }
        queue.write_buffer(&self.viewport, 0, &viewport);
        if blocks.len() as u64 > self.capacity {
            self.capacity = (blocks.len() as u64).next_power_of_two();
            self.instances = Self::instance_buffer(device, self.capacity);
        }
        let mut instances = Vec::with_capacity(blocks.len() * 16);
        for block in blocks {
            for value in [block.rect.x, block.rect.y, block.rect.width, block.rect.height] {
                instances.extend_from_slice(&value.to_le_bytes());
            }
        }
        queue.write_buffer(&self.instances, 0, &instances);

        // A render pass borrows every bind group it uses until the pass ends.
        // Build all groups first so a short-lived loop variable cannot leave a
        // texture binding dangling between two block draws.
        let bind_groups: Vec<_> = blocks
            .iter()
            .map(|block| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("map block composite"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: self.viewport.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(block.texture.view()),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                })
            })
            .collect();

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("map block composites"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.quad.slice(..));
        pass.set_vertex_buffer(1, self.instances.slice(..));
        for (index, bind_group) in bind_groups.iter().enumerate() {
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..4, index as u32..index as u32 + 1);
        }
    }

    /// Restore completed map composites into the normal world attachments.
    ///
    /// This is deliberately a depth-writing pass, not an overlay: callers run
    /// it before server items and mobiles, so those live producers still test
    /// against exactly the map surface they would have met at LOD 0.
    pub fn render_deferred(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: crate::renderer::Target<'_>,
        depth_adjust: f32,
        blocks: &[CompositeQuad<'_>],
    ) {
        let blocks: Vec<_> = blocks
            .iter()
            .filter(|block| block.texture.deferred_views().is_some())
            .collect();
        if blocks.is_empty() {
            return;
        }
        let mut viewport = Vec::with_capacity(16);
        for value in [target.width as f32, target.height as f32, depth_adjust, 0.0] {
            viewport.extend_from_slice(&value.to_le_bytes());
        }
        queue.write_buffer(&self.viewport, 0, &viewport);
        if blocks.len() as u64 > self.capacity {
            self.capacity = (blocks.len() as u64).next_power_of_two();
            self.instances = Self::instance_buffer(device, self.capacity);
        }
        let mut instances = Vec::with_capacity(blocks.len() * 16);
        for block in &blocks {
            for value in [block.rect.x, block.rect.y, block.rect.width, block.rect.height] {
                instances.extend_from_slice(&value.to_le_bytes());
            }
        }
        queue.write_buffer(&self.instances, 0, &instances);
        let bind_groups: Vec<_> = blocks
            .iter()
            .map(|block| {
                let (ids, position, normal, depth) = block.texture.deferred_views().expect("filtered above");
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("map block deferred composite"),
                    layout: &self.deferred_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: self.viewport.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(block.texture.view()),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(ids),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(position),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(normal),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::TextureView(depth),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                })
            })
            .collect();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("map block deferred composites"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &target.gbuffer.ids,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &target.gbuffer.position,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &target.gbuffer.normal,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: target.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.deferred_pipeline);
        pass.set_vertex_buffer(0, self.quad.slice(..));
        pass.set_vertex_buffer(1, self.instances.slice(..));
        for (index, bind_group) in bind_groups.iter().enumerate() {
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..4, index as u32..index as u32 + 1);
        }
    }

    /// Resample colour and the three G-buffer planes into one cached texture.
    fn capture_planes(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: CaptureSource<'_>,
        captured: &CompositeTexture,
    ) {
        let Some((ids, position, normal, _)) = captured.deferred_views() else {
            return;
        };
        let size = captured.size;
        write_capture_uniform(queue, &self.capture_uniform, size, source.rect);
        let color = source.color.create_view(&wgpu::TextureViewDescriptor::default());
        let source_ids = source.ids.create_view(&wgpu::TextureViewDescriptor::default());
        let source_position = source
            .position
            .create_view(&wgpu::TextureViewDescriptor::default());
        let source_normal = source.normal.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("map composite plane capture"),
            layout: &self.capture_planes_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.capture_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&color),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&source_ids),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&source_position),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&source_normal),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("map composite plane capture"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: captured.view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: ids,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(crate::gbuffer::IDS_CLEAR),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: position,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: normal,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: None,
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.capture_planes_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_vertex_buffer(0, self.quad.slice(..));
        pass.draw(0..4, 0..1);
    }

    /// Write the source depth rectangle into a captured entry's float plane.
    /// The colour and G-buffer planes were resampled by [`Self::capture_planes`];
    /// this pass exists solely because WebGPU
    /// does not permit a direct `Depth24Plus` to `R32Float` texture copy.
    fn capture_depth(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: CaptureSource<'_>,
        captured: &CompositeTexture,
    ) {
        let Some((_, _, _, depth)) = captured.deferred_textures() else {
            return;
        };
        let size = captured.size;
        write_capture_uniform(queue, &self.capture_uniform, size, source.rect);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("map composite depth capture"),
            layout: &self.capture_depth_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.capture_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source.depth),
                },
            ],
        });
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("map composite depth capture"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &depth_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.capture_depth_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_vertex_buffer(0, self.quad.slice(..));
        pass.draw(0..4, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_capture_pipelines_construct_when_a_renderable_adapter_is_available() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Some(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()
        else {
            return;
        };
        if !adapter
            .get_texture_format_features(crate::gbuffer::POSITION_FORMAT)
            .allowed_usages
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return;
        }
        let Ok((device, _)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: crate::gbuffer::required_limits(),
            ..Default::default()
        })) else {
            return;
        };
        let _ = CompositeRenderer::new(&device);
    }

    #[test]
    fn lod_zero_cannot_become_a_composite_key() {
        assert_eq!(CompositeTier::from_lod(BlockLod::Lod0), None);
        assert_eq!(CompositeTier::from_lod(BlockLod::Lod1), Some(CompositeTier::Lod1));
        assert_eq!(CompositeTier::from_lod(BlockLod::Lod2), Some(CompositeTier::Lod2));
    }

    #[test]
    fn block_coordinates_are_not_tile_coordinates() {
        assert_eq!(MapBlock::containing_tile(0, 7).first_tile(), (0, 0));
        assert_eq!(MapBlock::containing_tile(8, 15).first_tile(), (8, 8));
        assert_eq!(MapBlock::containing_tile(23, 24).first_tile(), (16, 24));
    }

    #[test]
    fn tiers_downsample_the_same_padded_source_extent() {
        let lod1 = CompositeSize::for_block(CompositeTier::Lod1, 64);
        let lod2 = CompositeSize::for_block(CompositeTier::Lod2, 64);
        assert_eq!(lod1, CompositeSize::new(240, 240).unwrap());
        assert_eq!(lod2, CompositeSize::new(120, 120).unwrap());
    }

    #[test]
    fn pixels_require_an_exact_rgba_image() {
        let size = CompositeSize::new(3, 2).unwrap();
        assert!(CompositePixels::new(size, vec![0; 23]).is_none());
        let pixels = CompositePixels::new(size, vec![7; 24]).unwrap();
        assert_eq!(pixels.size(), size);
        assert_eq!(pixels.rgba(), vec![7; 24]);
    }

    #[test]
    fn only_a_complete_deferred_result_can_replace_map_geometry() {
        let size = CompositeSize::new(2, 1).unwrap();
        let plain = CompositePixels::new(size, vec![0; 8]).unwrap();
        assert!(plain.deferred().is_none());
        assert!(DeferredPixels::new(size, vec![0; 1], vec![0.0; 8], vec![0; 2], vec![1.0; 2], 17).is_none());
        let deferred =
            DeferredPixels::new(size, vec![1; 2], vec![0.0; 8], vec![2; 2], vec![0.5; 2], 17).unwrap();
        let ready = plain.with_deferred(deferred).unwrap();
        assert_eq!(ready.deferred().unwrap().depth_base(), 17);
    }

    fn blocks(min_x: u16, max_x: u16, min_y: u16, max_y: u16) -> MapBlockBounds {
        MapBlockBounds {
            min_x,
            max_x,
            min_y,
            max_y,
        }
    }

    #[test]
    fn queue_dispatches_visible_blocks_before_blocks_ahead_of_the_camera() {
        let mut queue = CompositeWorkQueue::new(32, 32).unwrap();
        let map = blocks(0, 9, 0, 9);
        let revision = ImmutableRevision(7);
        queue.refresh(blocks(1, 2, 1, 2), map, BlockLod::Lod2, revision, |_| false);
        // A one-block pan right predicts a full viewport to the right.  The
        // first four jobs are still the newly visible rectangle, regardless of
        // the ahead range's nearer distance.
        queue.refresh(blocks(2, 3, 1, 2), map, BlockLod::Lod2, revision, |_| false);
        let work = queue.take_for_frame();
        let visible: BTreeSet<_> = blocks(2, 3, 1, 2).blocks().collect();
        let first_visible = work.iter().take(visible.len()).collect::<Vec<_>>();
        assert!(
            first_visible
                .iter()
                .all(|job| job.priority == CompositePriority::Visible)
        );
        assert!(first_visible.iter().all(|job| visible.contains(&job.key.block)));
        assert!(work.iter().skip(visible.len()).all(|job| {
            job.priority == CompositePriority::Visible || job.priority == CompositePriority::Ahead
        }));
    }

    #[test]
    fn queue_is_bounded_and_visible_work_evicts_the_furthest_prefetch() {
        let mut queue = CompositeWorkQueue::new(2, 1).unwrap();
        let map = blocks(0, 9, 0, 9);
        queue.refresh(
            blocks(0, 0, 0, 0),
            map,
            BlockLod::Lod1,
            ImmutableRevision(0),
            |_| false,
        );
        queue.refresh(
            blocks(1, 2, 0, 0),
            map,
            BlockLod::Lod1,
            ImmutableRevision(0),
            |_| false,
        );
        assert_eq!(queue.pending_len(), 2);
        let work = queue.take_for_frame();
        assert_eq!(work[0].priority, CompositePriority::Visible);
        assert_eq!(work[0].key.block, MapBlock { x: 1, y: 0 });
    }

    #[test]
    fn ready_or_in_flight_work_is_not_composed_again() {
        let mut queue = CompositeWorkQueue::new(8, 1).unwrap();
        let visible = blocks(1, 1, 1, 1);
        let map = blocks(0, 9, 0, 9);
        let revision = ImmutableRevision(4);
        queue.refresh(visible, map, BlockLod::Lod1, revision, |_| false);
        let first = queue.take_for_frame();
        assert_eq!(first.len(), 1);
        queue.refresh(visible, map, BlockLod::Lod1, revision, |_| false);
        assert_eq!(queue.pending_len(), 0, "in-flight work is deduplicated");
        queue.finished(first[0].key);
        queue.refresh(visible, map, BlockLod::Lod1, revision, |key| key == first[0].key);
        assert_eq!(queue.pending_len(), 0, "ready work is not re-requested");
    }

    #[test]
    fn tile_bounds_are_clipped_before_becoming_block_requests() {
        let bounds = MapBlockBounds::from_tiles(
            TileBounds {
                min_x: -8,
                max_x: 17,
                min_y: -1,
                max_y: 8,
            },
            16,
            16,
        )
        .unwrap();
        assert_eq!(bounds, blocks(0, 1, 0, 1));
    }

    #[test]
    fn invalidating_a_block_cancels_its_pending_and_in_flight_lods_only() {
        let mut queue = CompositeWorkQueue::new(8, 2).unwrap();
        let map = blocks(0, 9, 0, 9);
        queue.refresh(
            blocks(2, 3, 2, 2),
            map,
            BlockLod::Lod1,
            ImmutableRevision(9),
            |_| false,
        );
        let dispatched = queue.take_for_frame();
        assert_eq!(dispatched.len(), 2);
        let changed = dispatched[0].key.block;
        assert!(queue.invalidate_block(changed) >= 1);
        assert!(
            !queue.in_flight.contains(&dispatched[0].key),
            "a late result for a changed block must no longer own a cache slot"
        );
        assert!(queue.in_flight.iter().all(|key| key.block != changed));
        assert!(queue.pending.keys().all(|key| key.block != changed));
    }

    #[test]
    fn cache_eviction_keeps_the_viewport_margin_and_discards_the_lru_tail() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Some(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()
        else {
            return;
        };
        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        else {
            return;
        };
        let limits = CompositeCacheLimits::new(32, 0).unwrap();
        let mut cache = CompositeCache::with_limits(limits);
        let size = CompositeSize::new(2, 2).unwrap();
        let key = |x| CompositeKey {
            block: MapBlock { x, y: 0 },
            tier: CompositeTier::Lod1,
            revision: ImmutableRevision(0),
        };
        for x in 0..3 {
            cache.insert(
                &device,
                &queue,
                key(x),
                CompositePixels::new(size, vec![x as u8; 16]).unwrap(),
            );
        }
        // Make block zero newer than block one.  Block two is visible and so
        // protected even though it is the oldest entry after these insertions.
        cache.get(key(0));
        let evicted = cache.evict_lru_outside_viewport(Some(blocks(2, 2, 0, 0)));
        assert_eq!(evicted.entries, 1);
        assert!(cache.get(key(0)).is_some());
        assert!(
            cache.get(key(1)).is_none(),
            "the oldest non-visible entry is the LRU tail"
        );
        assert!(cache.get(key(2)).is_some(), "the visible block is protected");
        assert_eq!(evicted.retained_gpu_bytes, 32);
    }

    #[test]
    fn viewport_margin_is_hysteresis_not_an_eager_eviction_target() {
        let bounds = blocks(10, 11, 20, 21);
        assert!(bounds.expanded_by(1).contains(MapBlock { x: 9, y: 19 }));
        assert!(bounds.expanded_by(1).contains(MapBlock { x: 12, y: 22 }));
        assert!(!bounds.expanded_by(1).contains(MapBlock { x: 8, y: 20 }));
    }
}
