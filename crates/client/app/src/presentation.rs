//! What is drawn, and only that.
//!
//! [`App::draw`] and [`App::advance`] are the two halves of a frame — the
//! clock and the picture — and [`App::draw_from`] is the picture itself:
//! atlases grown, geometry assembled, every pass encoded. [`App::frame_facts`]
//! is the one place a pick happens, because the highlight and the tile marker
//! have to agree with what was actually drawn. [`assemble_geometry`] and the
//! free functions beside it are kept free rather than folded into
//! `draw_from` on purpose — see that method's own doc for the borrow-checker
//! reason.
//!
//! **A pure reader of command state.** Nothing here writes a walk target, a
//! gump's contents or anything a packet fills in — that is `net_command.rs`'s
//! and `ui_command.rs`'s and `own_windows.rs`'s job, upstream of a frame. What
//! this file *does* still mutate is purely presentational: animation clocks,
//! the atlases, the frame counters — state about how a picture is drawn, not
//! about what is true.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use openshard_client_render::animate::StaticAnimations;
use openshard_client_render::atlas::{AnimAtlas, AnimationKey, StaticAtlasPage};
use openshard_client_render::blit::{self, Blit, ViewportRect};
use openshard_client_render::camera::{Camera, TileBounds, ViewPixel};
use openshard_client_render::composite::{
    CaptureSource, CompositeProducerJob, CompositeTexture, CompositeWork, CompositeWorkQueue,
    ImmutableRevision, MapBlockBounds,
};
use openshard_client_render::cutaway::Cutaway;
use openshard_client_render::debug::View;
use openshard_client_render::frame::{self, Draw, Impostor};
use openshard_client_render::gbuffer::Gbuffer;
use openshard_client_render::gump::GumpPixel;
use openshard_client_render::items::{self};
use openshard_client_render::lod::BlockLod;
use openshard_client_render::mobiles::{self, Mobile};
use openshard_client_render::outline::{self};
use openshard_client_render::renderer::{self, Target};
use openshard_client_render::sprite::{SpriteQuad, split_corners};
use openshard_client_render::text::{self, Label};
use openshard_client_render::{ground, light, paperdoll, statics};
use openshard_protocol::speech::Font;
use openshard_protocol::wire::Hue;
use openshard_uofiles::map::Map;

use crate::app::App;
use crate::chat::draw_chat_and_speech;
use crate::crowd::{Crowd, Who};
use crate::diagnostics::Pick;
use crate::frame_geometry::{FrameFacts, assemble_geometry};
use crate::graphics::HighlightTarget;
use crate::picking::SelectedIdentity;
use crate::profile;
use crate::render_passes::{draw_gump_windows, encode_world_passes};
use crate::window::{prepare_composite_job, ready_atlases};
use crate::windows::{Drawn, WindowSubject};
use crate::world::{DAMAGE_NUMBER_HOLD, DAMAGE_NUMBER_RISE, PlayerMotion, advance_presentation_to};

/// Build one complete immutable map block without touching the camera frame.
///
/// The producer camera, targets and source rectangle all come from `work`.
/// Map land/statics (including mesh faces) are its only draw calls; server
/// items, mobiles, cutaway rows and every UI plane remain outside this command
/// buffer.  The cache entry is accepted only after the colour, G-buffer and
/// depth copier passes have all been recorded from those private attachments.
#[allow(clippy::too_many_arguments)]
fn produce_composite_block(
    resources: &crate::resources::Resources,
    animations: &StaticAnimations,
    tuning: &light::Tuning,
    sky: Option<light::Ambient>,
    fringe: openshard_client_render::impostor::Fringe,
    window: &mut crate::window::Screen,
    composite_work: &mut CompositeWorkQueue,
    work: CompositeWork,
) {
    let job = CompositeProducerJob::new(work.key);
    let camera = job.camera();
    let mut fades = openshard_client_render::cutaway::Fades::default();
    let (assembled, _) = frame::assemble_split_profiled(frame::Inputs {
        map: &resources.map,
        items: &[],
        camera: &camera,
        tiledata: &resources.tiledata,
        animations,
        cutaway: &Cutaway::OPEN,
        land: &window.atlases.land,
        texmaps: &window.atlases.texmaps,
        statics: &window.atlases.statics,
        sky,
        sun: None,
        carried: None,
        tuning,
        flame_time: 0.0,
        bake: None,
        highlight: None,
        impostor: match sky {
            Some(_) => Impostor::Met,
            None => Impostor::Billboards,
        },
        draw: Draw {
            // `assemble_split_profiled` supplies the padded source's lighting
            // and occlusion only. The owner-only geometry below is collected
            // separately, so neighbouring map rows never enter this producer.
            land: false,
            statics: false,
            items: false,
            mobiles: false,
        },
        view: View::Lit,
        dead: false,
        player_rect: None,
        player_mask: None,
        fades: &mut fades,
    });
    // The padded camera supplies the occlusion context, not picture ownership.
    // A composite is the sole producer of its own 8×8 tiles from the first
    // geometry list onward; neighbouring rows must not be rendered here and
    // later erased texel-by-texel by the capture shader.  The latter remains a
    // defensive assertion on the attachment, while this is the actual owner.
    let (first_x, first_y) = job.key().block.first_tile();
    let owner = TileBounds {
        min_x: i32::from(first_x),
        max_x: i32::from(first_x) + openshard_uofiles::map::BLOCK_SIZE as i32 - 1,
        min_y: i32::from(first_y),
        max_y: i32::from(first_y) + openshard_uofiles::map::BLOCK_SIZE as i32 - 1,
    };
    let ground = ground::collect_in(
        &resources.map,
        &camera,
        owner,
        &window.atlases.land,
        &window.atlases.texmaps,
        &Cutaway::OPEN,
    );
    let no_grid = openshard_client_render::occlusion::Occlusion::EMPTY;
    let occlusion = match sky {
        Some(_) => &assembled.lighting.occlusion,
        None => &no_grid,
    };
    let map_statics = statics::collect_in(
        &resources.map,
        &camera,
        owner,
        &resources.tiledata,
        animations,
        &window.atlases.statics,
        &Cutaway::OPEN,
        occlusion,
        None,
        None,
    );
    let openshard_client_render::statics::StaticGeometry {
        quads,
        mesh_vertices,
        mesh_rows,
        boxes,
        ..
    } = map_statics;
    let statics = split_corners(quads);

    let mut encoder = window
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("map block composite producer"),
        });
    // This buffer is submitted ahead of the camera frame, but its timestamp
    // query is resolved by that frame's encoder after the ordered submission.
    // It therefore appears as an independent GPU pass without a CPU wait.
    let timed = profile::begin(window.gpu.as_ref(), "map composite producer", &mut encoder);
    window.composite_producer.clear(&mut encoder);
    let world = window
        .composite_producer
        .world
        .create_view(&wgpu::TextureViewDescriptor::default());
    let depth = window
        .composite_producer
        .depth
        .create_view(&wgpu::TextureViewDescriptor::default());
    let gbuffer = window.composite_producer.gbuffer.views();
    let target = Target {
        view: &world,
        depth: &depth,
        gbuffer: &gbuffer,
        width: job.source_size().width,
        height: job.source_size().height,
        projection: camera.projection(),
    };
    window
        .renderer
        .render(&window.device, &window.queue, &mut encoder, target, &ground);
    window.statics.set_fringe(fringe);
    window.statics.render(
        &window.device,
        &window.queue,
        &mut encoder,
        target,
        &statics.rows,
        &boxes,
        Some(statics.drawn),
    );
    window.mesh_pass.render(
        &window.device,
        &window.queue,
        &mut encoder,
        target,
        &mesh_vertices,
        &mesh_rows,
    );
    let (eye_x, eye_y) = camera.eye_tile();
    let source = CaptureSource {
        color: &window.composite_producer.world,
        ids: window.composite_producer.gbuffer.ids(),
        position: window.composite_producer.gbuffer.position(),
        normal: window.composite_producer.gbuffer.normal(),
        depth: &depth,
        depth_base: openshard_client_render::depth::base_for(eye_x, eye_y),
        rect: ViewportRect {
            x: 0,
            y: 0,
            width: job.source_size().width,
            height: job.source_size().height,
        },
    };
    let captured = composite_work.finish_capture(
        &window.device,
        &window.queue,
        &mut encoder,
        &mut window.composite_pass,
        &mut window.composites,
        work.key,
        source,
    );
    profile::end(window.gpu.as_ref(), &mut encoder, timed);
    window.queue.submit([encoder.finish()]);
    if let Some(captured) = captured {
        audit_captured_composite_ids(
            &window.device,
            &window.queue,
            &resources.map,
            captured,
            &window.composite_producer.world,
            window.composite_producer.gbuffer.ids(),
        );
    }
}

/// Read one texture into packed rows for an opt-in producer/cache audit.
fn audit_texture_bytes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    bytes_per_texel: u32,
    label: &'static str,
) -> Option<Vec<u8>> {
    let row = texture.width() * bytes_per_texel;
    let stride = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: u64::from(stride) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(texture.height()),
            },
        },
        wgpu::Extent3d {
            width: texture.width(),
            height: texture.height(),
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let (sent, received) = mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
        let _ = sent.send(result);
    });
    if device.poll(wgpu::PollType::wait_indefinitely()).is_err()
        || received.recv().ok().and_then(Result::ok).is_none()
    {
        return None;
    }
    let mapped = readback.slice(..).get_mapped_range().ok()?;
    let packed = mapped
        .chunks_exact(stride as usize)
        .flat_map(|source_row| source_row[..row as usize].iter().copied())
        .collect();
    drop(mapped);
    readback.unmap();
    Some(packed)
}

/// Read the completed cache entry itself, after the producer command buffer
/// has run and before any camera frame can restore it.  This is deliberately
/// opt-in: it waits for a GPU map, which is appropriate for the injected field
/// scenario but never for ordinary play.
fn audit_captured_composite_ids(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    map: &Map,
    captured: &CompositeTexture,
    source_color: &wgpu::Texture,
    source_ids: &wgpu::Texture,
) {
    if std::env::var_os("OPENSHARD_COMPOSITE_AUDIT").is_none() {
        return;
    }
    let Some((ids, _, _, _)) = captured.deferred_textures_for_audit() else {
        return;
    };
    let row = ids.width() * 4;
    let stride = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let bytes = u64::from(stride) * u64::from(ids.height());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("map composite IDs audit readback"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("map composite IDs audit"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: ids,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(ids.height()),
            },
        },
        wgpu::Extent3d {
            width: ids.width(),
            height: ids.height(),
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let (sent, received) = mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
        let _ = sent.send(result);
    });
    if device.poll(wgpu::PollType::wait_indefinitely()).is_err()
        || received.recv().ok().and_then(Result::ok).is_none()
    {
        tracing::warn!(key = ?captured.key(), "could not read captured map-composite IDs for audit");
        return;
    }
    let mapped = readback
        .slice(..)
        .get_mapped_range()
        .expect("completed map-composite audit has mapped bytes");
    let (mut nothing, mut land, mut statics, mut mobile, mut invalid) = (0_u64, 0_u64, 0_u64, 0_u64, 0_u64);
    for source_row in mapped.chunks_exact(stride as usize) {
        for word in source_row[..row as usize].chunks_exact(4) {
            match openshard_client_render::gbuffer::ids_kind(u32::from_le_bytes(
                word.try_into().expect("four ID bytes"),
            )) {
                Some(openshard_client_render::place::Kind::Nothing) => nothing += 1,
                Some(openshard_client_render::place::Kind::Land) => land += 1,
                Some(openshard_client_render::place::Kind::Static) => statics += 1,
                Some(openshard_client_render::place::Kind::Mobile) => mobile += 1,
                None => invalid += 1,
            }
        }
    }
    let job = CompositeProducerJob::new(captured.key());
    let divisor = captured.key().tier.source_pixels_per_texel();
    let mut missing_owner_centres = Vec::new();
    let (first_x, first_y) = captured.key().block.first_tile();
    for y in first_y..first_y + openshard_uofiles::map::BLOCK_SIZE as u16 {
        for x in first_x..first_x + openshard_uofiles::map::BLOCK_SIZE as u16 {
            let Some(land) = map.land(x, y) else {
                continue;
            };
            let at = job
                .camera()
                .to_screen(openshard_protocol::world::Point::new(x, y, land.z));
            let sample_x = (at.x.max(0) as u32 / divisor).min(ids.width() - 1);
            let sample_y = (at.y.max(0) as u32 / divisor).min(ids.height() - 1);
            let offset = sample_y as usize * stride as usize + sample_x as usize * 4;
            let id = u32::from_le_bytes(
                mapped[offset..offset + 4]
                    .try_into()
                    .expect("one cached ID texel"),
            );
            if openshard_client_render::gbuffer::ids_kind(id)
                == Some(openshard_client_render::place::Kind::Nothing)
                && missing_owner_centres.len() < 12
            {
                missing_owner_centres.push((x, y));
            }
        }
    }
    drop(mapped);
    readback.unmap();
    tracing::info!(
        key = ?captured.key(),
        width = ids.width(),
        height = ids.height(),
        nothing,
        land,
        statics,
        mobile,
        invalid,
        missing_owner_centres = ?missing_owner_centres,
        "captured map-composite IDs before restore"
    );
    if captured.key().tier == openshard_client_render::composite::CompositeTier::Lod1 {
        let Some(source_color) =
            audit_texture_bytes(device, queue, source_color, 4, "composite source colour audit")
        else {
            tracing::warn!(key = ?captured.key(), "could not read LOD1 producer colour for audit");
            return;
        };
        let Some(captured_color) = audit_texture_bytes(
            device,
            queue,
            captured.texture(),
            4,
            "composite cached colour audit",
        ) else {
            tracing::warn!(key = ?captured.key(), "could not read LOD1 cached colour for audit");
            return;
        };
        let Some(source_ids) =
            audit_texture_bytes(device, queue, source_ids, 4, "composite source IDs audit")
        else {
            tracing::warn!(key = ?captured.key(), "could not read LOD1 producer IDs for audit");
            return;
        };
        let Some(captured_ids) =
            audit_texture_bytes(device, queue, ids, 4, "composite cached IDs equality audit")
        else {
            tracing::warn!(key = ?captured.key(), "could not read LOD1 cached IDs for equality audit");
            return;
        };
        let color_difference = source_color
            .iter()
            .zip(&captured_color)
            .position(|(source, captured)| source != captured);
        let ids_difference = source_ids
            .iter()
            .zip(&captured_ids)
            .position(|(source, captured)| source != captured);
        if source_color.len() == captured_color.len()
            && source_ids.len() == captured_ids.len()
            && color_difference.is_none()
            && ids_difference.is_none()
        {
            tracing::info!(key = ?captured.key(), "lossless LOD1 cache bytes match producer source");
        } else {
            tracing::error!(
                key = ?captured.key(),
                source_color_bytes = source_color.len(),
                captured_color_bytes = captured_color.len(),
                source_ids_bytes = source_ids.len(),
                captured_ids_bytes = captured_ids.len(),
                ?color_difference,
                ?ids_difference,
                "lossless LOD1 cache bytes differ from producer source"
            );
        }
    }
}

/// Compare the resident static-atlas texture against the bytes that the CPU
/// atlas says belong there. The injected max-zoom soak calls this sparingly,
/// after all dirty-row uploads for that frame have been queued.
fn audit_static_atlas_pages(window: &crate::window::Screen) {
    fn digest(bytes: &[u8]) -> u64 {
        let mut hash = DefaultHasher::new();
        bytes.hash(&mut hash);
        hash.finish()
    }

    for index in 0..window.atlases.statics.page_count() {
        let page = StaticAtlasPage(index as u8);
        let cpu = window
            .atlases
            .statics
            .page(page)
            .expect("static atlas page_count owns every page");
        let Some(texture) = window.statics.atlas_page_texture_for_audit(page) else {
            tracing::error!(page = index, "static atlas CPU page has no GPU texture");
            continue;
        };
        let row = texture.width() * 4;
        let stride = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = window.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("static atlas soak readback"),
            size: u64::from(stride) * u64::from(texture.height()),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = window
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("static atlas soak audit"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(texture.height()),
                },
            },
            wgpu::Extent3d {
                width: texture.width(),
                height: texture.height(),
                depth_or_array_layers: 1,
            },
        );
        window.queue.submit([encoder.finish()]);
        let (sent, received) = mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = sent.send(result);
        });
        if window.device.poll(wgpu::PollType::wait_indefinitely()).is_err()
            || received.recv().ok().and_then(Result::ok).is_none()
        {
            tracing::error!(page = index, "could not read static atlas GPU texture");
            continue;
        }
        let mapped = readback
            .slice(..)
            .get_mapped_range()
            .expect("completed static atlas audit has mapped bytes");
        let gpu = mapped
            .chunks_exact(stride as usize)
            .flat_map(|source_row| source_row[..row as usize].iter().copied())
            .collect::<Vec<_>>();
        drop(mapped);
        readback.unmap();
        let cpu_hash = digest(cpu.pixels());
        let gpu_hash = digest(&gpu);
        if cpu_hash == gpu_hash && cpu.pixels() == gpu {
            tracing::info!(
                page = index,
                revision = window.atlases.statics.revision(),
                bytes = gpu.len(),
                hash = cpu_hash,
                "static atlas CPU and GPU state agree"
            );
        } else {
            tracing::error!(
                page = index,
                revision = window.atlases.statics.revision(),
                cpu_hash,
                gpu_hash,
                "static atlas GPU state differs from CPU source"
            );
        }
    }

    let (land, texmaps) = window.renderer.atlas_textures_for_audit();
    audit_atlas_texture(window, "land", land, window.atlases.land.pixels());
    audit_atlas_texture(window, "texmaps", texmaps, window.atlases.texmaps.pixels());
}

/// Compare one ordinary RGBA atlas texture with its CPU packing bytes.
fn audit_atlas_texture(
    window: &crate::window::Screen,
    label: &'static str,
    texture: &wgpu::Texture,
    cpu: &[u8],
) {
    let row = texture.width() * 4;
    let stride = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback = window.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("atlas soak readback"),
        size: u64::from(stride) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = window
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("atlas soak audit"),
        });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(texture.height()),
            },
        },
        wgpu::Extent3d {
            width: texture.width(),
            height: texture.height(),
            depth_or_array_layers: 1,
        },
    );
    window.queue.submit([encoder.finish()]);
    let (sent, received) = mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
        let _ = sent.send(result);
    });
    if window.device.poll(wgpu::PollType::wait_indefinitely()).is_err()
        || received.recv().ok().and_then(Result::ok).is_none()
    {
        tracing::error!(atlas = label, "could not read atlas GPU texture");
        return;
    }
    let mapped = readback
        .slice(..)
        .get_mapped_range()
        .expect("completed atlas audit has mapped bytes");
    let gpu = mapped
        .chunks_exact(stride as usize)
        .flat_map(|source_row| source_row[..row as usize].iter().copied())
        .collect::<Vec<_>>();
    drop(mapped);
    readback.unmap();
    let mut cpu_hasher = DefaultHasher::new();
    cpu.hash(&mut cpu_hasher);
    let mut gpu_hasher = DefaultHasher::new();
    gpu.hash(&mut gpu_hasher);
    let (cpu_hash, gpu_hash) = (cpu_hasher.finish(), gpu_hasher.finish());
    if cpu_hash == gpu_hash && cpu == gpu {
        tracing::info!(
            atlas = label,
            bytes = gpu.len(),
            hash = cpu_hash,
            "atlas CPU and GPU state agree"
        );
    } else {
        tracing::error!(
            atlas = label,
            cpu_hash,
            gpu_hash,
            "atlas GPU state differs from CPU source"
        );
    }
}

/// Compare the bytes the scene renderer will fetch for this frame against the
/// current CPU serialization. This is the direct oracle for a suspected
/// circular/staging overwrite of sprite placement rather than atlas pixels.
fn audit_scene_instance_buffers(window: &crate::window::Screen) {
    for (label, (source, expected)) in [
        ("map statics", window.statics.instance_state_for_audit()),
        ("items", window.items_pass.instance_state_for_audit()),
        ("mobiles", window.mobile_pass.instance_state_for_audit()),
    ] {
        if expected.is_empty() {
            continue;
        }
        let readback = window.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene instance soak readback"),
            size: expected.len() as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = window
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("scene instance soak audit"),
            });
        encoder.copy_buffer_to_buffer(source, 0, &readback, 0, expected.len() as u64);
        window.queue.submit([encoder.finish()]);
        let (sent, received) = mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = sent.send(result);
        });
        if window.device.poll(wgpu::PollType::wait_indefinitely()).is_err()
            || received.recv().ok().and_then(Result::ok).is_none()
        {
            tracing::error!(scene = label, "could not read scene instance buffer");
            continue;
        }
        let actual = readback
            .slice(..)
            .get_mapped_range()
            .expect("completed scene audit has mapped bytes")
            .to_vec();
        readback.unmap();
        if actual == expected {
            tracing::info!(
                scene = label,
                bytes = expected.len(),
                "scene instance CPU and GPU state agree"
            );
        } else {
            let first_difference = actual
                .iter()
                .zip(expected)
                .position(|(actual, expected)| actual != expected);
            tracing::error!(
                scene = label,
                bytes = expected.len(),
                ?first_difference,
                "scene instance GPU state differs from current CPU rows"
            );
        }
    }
}

/// Inspect the actual frame G-buffer at every visible ground-tile centre.
///
/// This catches the failure a picture can only suggest: a map block was marked
/// ready (therefore its LOD0 rows were omitted), but its restored deferred
/// rectangle wrote `Kind::Nothing` at a tile it owns.  The check is opt-in
/// because mapping a full screen attachment intentionally fences the device.
fn audit_visible_ground_centres(window: &crate::window::Screen, map: &Map, camera: Camera) {
    let ids = window.gbuffer.ids();
    let row = ids.width() * 4;
    let stride = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let position = window.gbuffer.position();
    let position_row = position.width() * 16;
    let position_stride =
        position_row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback = window.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("LOD screen G-buffer audit readback"),
        size: u64::from(stride) * u64::from(ids.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let position_readback = window.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("LOD screen G-buffer position audit readback"),
        size: u64::from(position_stride) * u64::from(position.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = window
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("LOD screen G-buffer audit"),
        });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: ids,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(ids.height()),
            },
        },
        wgpu::Extent3d {
            width: ids.width(),
            height: ids.height(),
            depth_or_array_layers: 1,
        },
    );
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: position,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &position_readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(position_stride),
                rows_per_image: Some(position.height()),
            },
        },
        wgpu::Extent3d {
            width: position.width(),
            height: position.height(),
            depth_or_array_layers: 1,
        },
    );
    window.queue.submit([encoder.finish()]);
    let (sent, received) = mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
        let _ = sent.send(result);
    });
    let (position_sent, position_received) = mpsc::channel();
    position_readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = position_sent.send(result);
        });
    if window.device.poll(wgpu::PollType::wait_indefinitely()).is_err()
        || received.recv().ok().and_then(Result::ok).is_none()
        || position_received.recv().ok().and_then(Result::ok).is_none()
    {
        tracing::error!("could not read frame G-buffer for LOD bounds audit");
        return;
    }
    let samples: Vec<_> = camera
        .visible_tiles()
        .clamp_to(map.width(), map.height())
        .into_iter()
        .flat_map(|(xs, ys)| {
            ys.flat_map(move |y| {
                xs.clone().filter_map(move |x| {
                    let land = map.land(x, y)?;
                    let at = camera.to_screen(openshard_protocol::world::Point::new(x, y, land.z));
                    (at.x >= 0 && at.y >= 0 && at.x < ids.width() as i32 && at.y < ids.height() as i32)
                        .then_some((x, y, at.x as u32, at.y as u32))
                })
            })
        })
        .collect();
    let mapped = readback
        .slice(..)
        .get_mapped_range()
        .expect("completed LOD screen audit has mapped bytes");
    let mapped_position = position_readback
        .slice(..)
        .get_mapped_range()
        .expect("completed LOD screen position audit has mapped bytes");
    let mut composite = 0_u64;
    let mut missing = Vec::new();
    let mut misplaced_land = Vec::new();
    for (x, y, screen_x, screen_y) in &samples {
        let offset = *screen_y as usize * stride as usize + *screen_x as usize * 4;
        let id = u32::from_le_bytes(mapped[offset..offset + 4].try_into().expect("one ID texel"));
        if id & openshard_client_render::gbuffer::IDS_COMPOSITE_MAP != 0 {
            composite += 1;
        }
        if openshard_client_render::gbuffer::ids_kind(id)
            == Some(openshard_client_render::place::Kind::Nothing)
        {
            if missing.len() < 12 {
                missing.push((*x, *y));
            }
        }
        if id & openshard_client_render::gbuffer::IDS_COMPOSITE_MAP != 0
            && openshard_client_render::gbuffer::ids_kind(id)
                == Some(openshard_client_render::place::Kind::Land)
        {
            let position_offset = *screen_y as usize * position_stride as usize + *screen_x as usize * 16;
            let actual_x = f32::from_le_bytes(
                mapped_position[position_offset..position_offset + 4]
                    .try_into()
                    .expect("cached land x position"),
            )
            .floor() as i32;
            let actual_y = f32::from_le_bytes(
                mapped_position[position_offset + 4..position_offset + 8]
                    .try_into()
                    .expect("cached land y position"),
            )
            .floor() as i32;
            if (actual_x != i32::from(*x) || actual_y != i32::from(*y)) && misplaced_land.len() < 12 {
                misplaced_land.push(((*x, *y), (actual_x, actual_y)));
            }
        }
    }
    drop(mapped);
    drop(mapped_position);
    readback.unmap();
    position_readback.unmap();
    // A ground diamond's visual centre can belong to either neighbouring
    // triangle, so this optional position sample is diagnostic context rather
    // than a coverage failure. `missing` alone is the LOD readiness invariant.
    if missing.is_empty() {
        tracing::info!(
            samples = samples.len(),
            composite,
            misplaced_land = ?misplaced_land,
            "LOD screen G-buffer covers every visible tile centre"
        );
    } else {
        tracing::error!(
            samples = samples.len(),
            composite,
            missing = ?missing,
            misplaced_land = ?misplaced_land,
            "LOD screen G-buffer has uncovered visible tile centres"
        );
    }
}

/// LOD2 stays held back while the direct field scenario validates LOD1.
///
/// The first tier uses the same canonical source and deferred planes as LOD2,
/// but preserves twice as many cache texels.  Its producer cache is audited
/// directly before restore; only after this field gate is clean can the next
/// minified tier take over.
const fn visible_composite_lod(selected: BlockLod) -> BlockLod {
    match selected {
        BlockLod::Lod2 => BlockLod::Lod1,
        lod => lod,
    }
}

/// The immutable boundary between advancing the client and presenting one
/// frame. It contains the one camera and the read-only facts every pass,
/// overlay and next-frame click must agree on.
struct PreparedFrame {
    started: Instant,
    camera: Camera,
    facts: FrameFacts,
}

impl App {
    /// Everyone to draw, each beside the serial their clock is keyed by.
    ///
    /// Our own body first, and `None` for it while no shard has named us.
    ///
    /// The group is refreshed from the crowd here and not in
    /// [`App::advance_to_clocks`] alone, because this list is what *packs* the
    /// atlas as well as what draws from it — see [`App::wanted_in`]. `self.world.presentation.player`
    /// and `self.world.presentation.others` hold the group as of the last packet, and
    /// [`Crowd::advance`] changes it without one: a body that walked into view
    /// and then stopped is drawn standing while the packet-time list still says
    /// walking. Pack one group and draw another and [`mobiles::place`] finds no
    /// frame, so the body simply vanishes — and stays vanished for as long as it
    /// stands still, there being no further packet to correct the list with.
    pub(crate) fn drawn_mobiles(&self) -> Vec<(Who, Mobile)> {
        Self::everyone_drawn(
            &self.world.presentation.crowd,
            self.world.me(),
            &self.world.presentation.player,
            &self.world.presentation.others,
            &self.world.presentation.corpses,
        )
    }

    /// [`App::drawn_mobiles`] over the four fields it reads, so a test can build
    /// the list the atlases are grown from without a window, a device or a
    /// shard. The snapshot clones each mobile's cheap immutable equipment
    /// handle; its time-varying fields are then advanced below.
    pub(crate) fn everyone_drawn(
        crowd: &Crowd,
        me: Who,
        player: &Mobile,
        others: &[(Who, Mobile)],
        corpses: &[(Who, Mobile)],
    ) -> Vec<(Who, Mobile)> {
        let mut mobiles = Vec::with_capacity(others.len() + corpses.len() + 1);
        mobiles.push((me, player.clone()));
        mobiles.extend_from_slice(others);
        mobiles.extend_from_slice(corpses);
        Self::advance_groups(crowd, &mut mobiles);
        mobiles
    }

    /// Refresh each body's animation group from the crowd's clock.
    ///
    /// Split out of [`App::advance_to_clocks`] because the group is the one part
    /// of a mobile that has to be right *before* the atlases are grown, and the
    /// growth happens with no atlas to ask for a frame count. Both paths go
    /// through here so there is one statement of "which group is playing".
    pub(crate) fn advance_groups(crowd: &Crowd, drawn: &mut [(Who, Mobile)]) {
        for (who, mobile) in drawn.iter_mut() {
            // `Crowd::advance` drops a walking body to standing on its own
            // timer, with nothing that looks like a packet to refresh
            // `mobile.group` from — a group read once and left stale plays the
            // walking sprite for ever, timed by a clock that has moved on to
            // the standing group's.
            if let Some(group) = crowd.group_for(*who) {
                mobile.group = group;
            }
        }
    }

    /// Fill in the time-varying presentation state. `Crowd` provides animation
    /// groups and frames for everyone; the local player's pose and sorting
    /// source come exclusively from [`PlayerMotion`].
    ///
    /// An associated function taking the two fields it reads rather than a
    /// method, because both callers hold a borrow of one of `App`'s fields
    /// while they ask: the frame holds `self.window` mutably, and the pick
    /// holds it shared. A `&self` method would borrow all of `App` and neither
    /// could call it.
    ///
    /// `atlas` is asked for the frame *count*: a group's length is the
    /// animation's, and taking it from anywhere else makes "frame 7 of a
    /// 6-frame walk" expressible. Under the body the atlas packed — for a ghost
    /// the living body it borrows its pictures from — or a ghost counts zero
    /// frames, lands on frame 0 for ever and slides along standing still.
    pub(crate) fn advance_to_clocks(
        crowd: &Crowd,
        atlas: &AnimAtlas,
        me: Who,
        motion: &PlayerMotion,
        drawn: &mut [(Who, Mobile)],
    ) {
        // The group is read back first and not only the frame and the glide —
        // the frame count below is asked *under* it. Idempotent when the caller
        // is [`App::drawn_mobiles`], which is every caller today; here so this
        // function is right on its own terms rather than on its callers'.
        Self::advance_groups(crowd, drawn);
        for (who, mobile) in drawn.iter_mut() {
            let (direction, _) = openshard_uofiles::anim::facing(mobile.facing);
            let frame_count = atlas.frame_count(AnimationKey::new(
                openshard_uofiles::anim::animation_body(mobile.body),
                mobile.group,
                direction,
            ));
            mobile.frame = openshard_uofiles::anim::AnimationFrameIndex(crowd.frame_for(*who, frame_count));
            if *who == me {
                // This is the boundary that used to reintroduce the bug: the
                // frame builder overwrote the local `GameMotion` pose from
                // `Crowd`, so a stuck crowd made a moving HUD look detached
                // from a stationary body.
                Self::project_local_motion(motion, mobile);
            } else {
                if let Some(at) = crowd.drawn_for(*who) {
                    mobile.drawn = at;
                }
                // A remote mobile has no local movement core. Its crowd entry
                // remains the presentation source for sort order.
                mobile.from = crowd.stepping_from(*who);
            }
        }
        if let Some((_, player)) = drawn.iter().find(|(who, _)| *who == me) {
            debug_assert_eq!(player.drawn, motion.drawn());
            debug_assert_eq!(player.from, motion.transition_from());
        }
    }

    /// Apply the only two movement-owned fields of the local render mobile.
    /// Kept separate so this boundary is testable without a window or atlas.
    fn project_local_motion(motion: &PlayerMotion, mobile: &mut Mobile) {
        mobile.drawn = motion.drawn();
        mobile.from = motion.transition_from();
    }

    /// Everyone as they are drawn *this instant*, clocks and all — the list
    /// [`mobiles::pick`] and [`mobiles::collect`] both index into.
    ///
    /// Built twice a frame, once for the pick and once for the picture, rather
    /// than threaded between them: the two happen either side of the atlas
    /// growth and of a mutable borrow of the window, and the work is a handful
    /// of map lookups over whoever is on screen. What matters is that the
    /// *order* is [`App::drawn_mobiles`]'s both times, so an index answered by
    /// the pick still names the same creature to the passes below.
    pub(crate) fn drawn_now(&self, atlas: &AnimAtlas) -> Vec<(Who, Mobile)> {
        let mut drawn = self.drawn_mobiles();
        Self::advance_to_clocks(
            &self.world.presentation.crowd,
            atlas,
            self.world.me(),
            &self.world.motion,
            &mut drawn,
        );
        drawn
    }

    pub(crate) fn draw(&mut self) {
        // The assets and GPU must exist for the login conversation, but a shard
        // has not yet named a world while that conversation is under way. Keep
        // the surface untouched until its first complete view: the startup
        // placeholder at `START` is for offline inspection, not a temporary
        // online character position.
        if !self.world.render_ready {
            return;
        }
        // Movement is advanced before the HUD is assembled below. Clear the
        // frame-local plan here so both consumers share at most one search.
        self.steer.begin_frame();
        let started = Instant::now();
        // The frame boundary the flamegraph is cut on, put at the same place
        // `started` is sampled so that a frame in `puffin_viewer` and a frame in
        // the `frames` panel are the same span of time. Free when nobody is
        // recording — see [`profile`].
        profile::frame();
        puffin::profile_scope!("draw");
        // What the shard has opened, and what it has taken away: the view is
        // filled by `client/net`, which knows nothing about screens, so a
        // window appearing is this end noticing.
        self.sync_own_windows();
        // # The frame is three steps, and this is the first of them
        //
        // Everything that writes runs in `Self::advance`, before anything
        // reads — see that method's own doc for why the clock and the eye
        // move there and not here. After it returns, nothing in the frame
        // moves the world or the camera again; the snapshot below is what
        // every reader from here on is handed.
        self.advance(started);
        let camera = *self.control.camera();
        self.draw_from(started, camera);
    }

    /// **Step one of three**: everything that writes. What the shell asked
    /// for last frame, then every clock, then the eye.
    ///
    /// The animation clock moves here, at the top of the frame that is about
    /// to show its answer — not when the timer that asked for this frame
    /// fired.
    ///
    /// A glide is a position read off a clock, so the moment that clock is
    /// read has to be the moment the picture is built or the walk judders:
    /// the timer fires, the loop then lays out the UI, grows an atlas and
    /// waits on the swapchain, and however long that took is error in the
    /// body's position — error that varies frame to frame, which is exactly
    /// what an eye reads as a stutter. It also puts the sampling back in step
    /// with the display: `WaitUntil` is a floor, the timer's 16ms beats
    /// against a 60Hz refresh, and a frame drawn from the previous tick's
    /// clock lands on the wrong side of that beat every second or so.
    ///
    /// Whatever really passed — see `App::last_advance`. A stall longer than
    /// a frame, the window minimised or the machine asleep, moves the clock
    /// the whole way rather than queuing a burst of catch-up frames for time
    /// nobody watched: a body that was walking through it has long since
    /// arrived.
    ///
    /// The defect this staging is written against: the HUD used to be built
    /// at the top of the frame and the eye moved a few lines further down, so
    /// the overlay egui laid out — the tile highlight, the hover, the walk
    /// goal — was drawn against the *previous* frame's camera while the world
    /// pass below drew from this one's. The gap between them is one frame of
    /// camera motion, which is not a constant: it is whatever the display
    /// gave this frame, so the markers shivered against the ground they were
    /// meant to be lying on, and every missed interval made them jump.
    /// Reordering two calls would have fixed today's version of it and left
    /// the shape that produced it, which is a second reader picking the
    /// camera up at a different moment. So the frame is staged instead.
    pub(crate) fn advance(&mut self, started: Instant) {
        let elapsed = started.saturating_duration_since(self.last_advance);
        let asked = std::mem::take(&mut self.pending);
        self.apply(asked);
        // The viewport the last frame's layout left free — `Shell` holds it
        // between frames for exactly this. It has to be settled before the eye
        // is, because it is what decides how much world a camera can see.
        if let Some(shell) = self.shell.as_ref() {
            let viewport = shell.viewport();
            self.control.resize(viewport.width, viewport.height);
        }
        advance_presentation_to(
            &mut self.world.presentation,
            &mut self.world.motion,
            &mut self.last_advance,
            started,
        );
        self.project_player_motion();
        // Whatever scenario is being walked delivers its knots for the span that
        // just passed, before the eye is asked where the body is: a step that
        // arrived this frame is one the camera has to answer this frame.
        let prediction_before_replay = self.world.motion.planning_state();
        self.advance_replay(elapsed);
        self.advance_lod_sweep(elapsed);
        if self.world.motion.planning_state() != prediction_before_replay {
            if let Some(trace) = self.movement_trace.as_mut() {
                trace.record(
                    "frame_replay_changed_prediction",
                    &self.world,
                    self.control.camera(),
                );
            }
        }
        // A viewport that grew may have taken the world texture past what the
        // device allows, which no zoom step asked for.
        self.fit_zoom_to_device();
        // And the eye goes where the body is *this frame*: a step arrives once
        // and is then walked across for the next 400ms, so every frame in
        // between has a different answer.
        self.follow_player(elapsed);
        if let Some(trace) = self.movement_trace.as_mut() {
            trace.record("frame", &self.world, self.control.camera());
        }
    }

    /// **Step two**: one snapshot, and it is a value.
    ///
    /// Every question this frame's picture and HUD are built from, asked
    /// once against one camera and one cutaway and answered as a plain
    /// value — purely a function of `&self`, so a caller cannot mistake this
    /// for a place the frame's state changes. It has none: `on_static`,
    /// `on_mobile` and `on_item` still have to land in `self.picking` for the
    /// click to read next frame, but that write happens in the three lines at
    /// `draw_from`'s call site instead, which is the "mutations applied
    /// separately" half of the shape `Self::advance` set up for the first
    /// step.
    pub(crate) fn frame_facts(&self, camera: Camera) -> FrameFacts {
        // Read before the window is borrowed below, for the same reason the
        // pacing at the foot of the frame is a fact about the whole app
        // rather than about it.
        let watched = self.watched();
        // The same, for the two the item highlight needs — both are questions
        // about the whole of `self` and are asked once, here.
        let owns_pointer = self.world_owns_pointer();
        let cursor = self.control.cursor();

        // What this frame does not draw, read once from the tile the player is
        // standing on. Once, and from the *player's* tile rather than the
        // camera's: a free camera looking at a rooftop three streets away has
        // not walked indoors, and the client's rule is about where the body is.
        // See `openshard_client_render::cutaway`.
        //
        // `self.world.presentation.cutaway_at`, not `self.world.presentation.player.at`: the latter is this end's
        // own unconfirmed prediction, which for one frame can be a tile a
        // held direction was refused on — see the field's own doc.
        //
        // Here, in the snapshot, and not beside the passes that draw from it:
        // the item pick below needs it, and the pick has to be answered before
        // the HUD is built — see the next paragraph.
        let cutaway = if self.graphics.cutaway_disabled {
            Cutaway::OPEN
        } else {
            Cutaway::at(
                &self.resources.map,
                &self.resources.tiledata,
                self.world.presentation.cutaway_at,
                true,
            )
        };
        // The ground tile under the cursor, and its ring — asked here beside
        // the picks below rather than a second time when the HUD is built:
        // this used to be `App::hud`'s own call to `Self::pick_tile`, a second
        // "what is the cursor over" answered from a *different* camera in
        // spirit even when it happened to be the same value in practice. One
        // frame's worth of picks belongs in one place — this function — same
        // as `on_mobile`/`on_item`/`on_static` below.
        let hover = owns_pointer.then(|| self.pick_tile(camera)).flatten();
        let neighbours = hover.as_ref().map_or_else(Vec::new, |tile| self.tile_ring(tile));
        // What the cursor is over, asked here rather than remembered from the
        // last click: the picture moves under a still mouse — the body walks,
        // the camera follows, a door swings — so where the cursor is pointing is
        // a question about *this* frame's picture and has to be asked against
        // this frame's camera. The same `items::pick` a double-click asks, so
        // what is lit is what would be used.
        //
        // Asked once and answered to three readers: the hue the picture is drawn
        // in, the silhouette the ring is grown from, and whether the HUD marks
        // the tile under the cursor at all. Two picks would be two chances to
        // disagree about what the cursor is on, and the visible form of that
        // disagreement is a barrel ringed with the ground under it diamonded.
        //
        // Against the atlas as it stands *before* this frame grows it, which is
        // the one thing given up by asking this early. An item that came on
        // screen this very frame has no sprite packed yet and so no rectangle to
        // be pointed at, and is pickable a frame later; the alternative was a
        // tile marker that decides whether to draw itself from the previous
        // frame's answer, which flickers along every item's edge.
        // **The picks are the frame's *facts*, and the mode decides only what is
        // drawn from them.** They used to be skipped under
        // `HighlightTarget::Tiles`, which folded two questions into one field:
        // "what is the cursor on" and "what may light up". A click reads the
        // first — see the `MouseInput` arm — so with the two folded together a
        // player who had pinned the highlight to tiles could not select a wall at
        // all, and the reason was invisible. The mode is applied to `lit_*`
        // below instead, where it is about lighting and nothing else.
        //
        // Creatures are asked first and they win: a mobile stands *on* the
        // clutter of its tile — it is sorted above whatever is lying there, and
        // it is what a player pointing at a shopkeeper standing on a rug means.
        // Then the server's items, then the map's own furniture. One chain, and
        // every later question is asked only where the earlier ones found
        // nothing — so "what is under the cursor" has exactly one answer and the
        // ring, the wash, the tile marker and the click cannot disagree about it.
        // Kept whole, and not just picked from: the click reads a mobile back by
        // [`Who`] rather than by this index, which is only ever good for this
        // one frame's own `Vec` — see `FrameFacts::on_mobile`.
        let drawn_mobiles = self
            .window
            .as_ref()
            .map(|window| self.drawn_now(&window.atlases.mobiles));
        let on_mobile = match (owns_pointer, self.window.as_ref(), &drawn_mobiles) {
            (true, Some(window), Some(drawn)) => mobiles::pick_iter(
                drawn.iter().map(|(_, mobile)| mobile),
                &camera,
                &window.atlases.mobiles,
                &cutaway,
                &self.resources.equip_conv,
                cursor,
            ),
            _ => None,
        };
        let on_item = match owns_pointer && on_mobile.is_none() {
            true => self.window.as_ref().and_then(|window| {
                items::pick(
                    &self.world.presentation.items,
                    &camera,
                    &self.resources.tiledata,
                    &self.world.presentation.tile_animations,
                    &window.atlases.statics,
                    &cutaway,
                    cursor,
                )
            }),
            false => None,
        };
        // And the map's own furniture last, which is the one a wall is: it has no
        // serial and cannot be used, so it loses to anything that can. Asked
        // every frame rather than at the click, because it is what the *tile
        // marker* has to know — a wall under the cursor takes the highlight, and
        // the diamond drawn on the ground behind it was the client answering the
        // same question twice with two different tiles.
        //
        // This is the one pick that walks the map: `statics::pick` covers the
        // cells `statics::collect` is about to draw. It is a second walk of them
        // per frame with the pointer over the world, and the placement it does
        // per static is the collector's own — see the Frames tab if it ever
        // shows.
        let on_static = match owns_pointer && on_mobile.is_none() && on_item.is_none() {
            true => self.window.as_ref().and_then(|window| {
                statics::pick(
                    &self.resources.map,
                    &camera,
                    &self.resources.tiledata,
                    &self.world.presentation.tile_animations,
                    &window.atlases.statics,
                    &cutaway,
                    cursor,
                )
            }),
            false => None,
        };
        // What the mode allows to light up. `Tiles` lights neither, which is the
        // whole of that setting; the facts above are unchanged by it.
        let lit_mobile = on_mobile.filter(|_| self.graphics.highlight != HighlightTarget::Tiles);
        let lit_item = on_item.filter(|_| self.graphics.highlight != HighlightTarget::Tiles);

        // The server-confirmed combat target owns the persistent mobile ring.
        // It takes precedence over a local click selection: selection may move
        // to a tile or an item while combat continues, but the target marker
        // must stay on the body the shard says we are fighting.
        let targeted_mobile = self
            .world
            .authoritative
            .view
            .as_ref()
            .and_then(|view| view.player.attacking)
            .filter(|_| self.graphics.drawing.mobiles)
            .and_then(|who| {
                drawn_mobiles.as_ref().and_then(|drawn| {
                    drawn
                        .iter()
                        .position(|(candidate, _)| *candidate == Some(who))
                        .map(openshard_client_render::mobiles::MobileIndex::new)
                })
            });
        // What a click is *holding*, turned from identity back into this
        // frame's index — the reverse of `on_mobile`/`on_item` just above.
        // This is the held ring's own pick, asked once here rather than at
        // every reader, for the reason `lit_item`'s doc gives for asking
        // `on_item` once: two lookups are two chances to disagree about which
        // creature a `Who` still names.
        //
        // Valid only while the crowd is actually drawn: `drawn` below is
        // emptied whole when `self.graphics.drawing.mobiles` is off, and an index into
        // `drawn_mobiles` would then point at a `Vec` the held ring never
        // sees.
        let selected_mobile = self
            .picking
            .selected
            .and_then(SelectedIdentity::as_mobile)
            .filter(|_| self.graphics.drawing.mobiles)
            .and_then(|who| {
                drawn_mobiles.as_ref().and_then(|drawn| {
                    drawn
                        .iter()
                        .position(|(candidate, _)| *candidate == who)
                        .map(openshard_client_render::mobiles::MobileIndex::new)
                })
            });
        let held_mobile = targeted_mobile.or(selected_mobile);
        let held_item = self
            .picking
            .selected
            .and_then(SelectedIdentity::as_item)
            .and_then(|serial| {
                self.world
                    .presentation
                    .item_serials
                    .iter()
                    .position(|candidate| *candidate == serial)
                    .map(openshard_client_render::items::ItemIndex::new)
            });
        FrameFacts {
            watched,
            cutaway,
            pick: Pick {
                tile: hover,
                neighbours,
                static_: on_static,
                mobile: lit_mobile,
                item: lit_item,
            },
            drawn_mobiles,
            on_mobile,
            on_item,
            held_mobile,
            held_item,
        }
    }

    /// Freeze the values presentation may read this frame. Writers publish
    /// their small, deliberate aftermath through [`Self::publish_frame_picks`]
    /// before any pass is encoded; no pass reaches back into live input or a
    /// newly moved camera.
    fn prepare_frame(&self, started: Instant, camera: Camera) -> PreparedFrame {
        PreparedFrame {
            started,
            camera,
            facts: self.frame_facts(camera),
        }
    }

    /// Publish the identities the next input event must read from a prepared
    /// frame. The facts remain otherwise immutable: this is the only bridge
    /// from the current picture to next-frame click handling.
    fn publish_frame_picks(&mut self, facts: &FrameFacts) {
        self.picking.on_static = facts.pick.static_;
        self.picking.on_mobile = facts.on_mobile.and_then(|index| {
            facts
                .drawn_mobiles
                .as_ref()
                .and_then(|drawn| drawn.get(index.position()))
                .map(|(who, _)| *who)
        });
        self.picking.on_item = facts
            .on_item
            .map(|index| self.world.presentation.item_serials[index.position()]);
    }

    /// **Steps two and three**: the frame `Self::advance` staged for. Takes
    /// the camera as a parameter rather than reading `self.control` again —
    /// a `&Camera` handed to five collectors is five reads of a field that
    /// something between them might have moved, which is the defect
    /// `Self::advance`'s doc is written against, expressed as a borrow. A
    /// `Camera` is `Copy`, so the one read in `draw` costs nothing and cannot
    /// be stale in one place and fresh in another.
    pub(crate) fn draw_from(&mut self, started: Instant, camera: Camera) {
        // # Step two: one snapshot, and it is a value
        //
        // `Self::frame_facts` asks every question this frame's picture and HUD
        // are built from, purely against `&self` — and answers three of them,
        // `on_static`/`on_mobile`/`on_item`, into `self.picking` right here,
        // which is the "mutations applied separately" half of the shape
        // `Self::advance` set up for the first: a function that only *asks*
        // stays a function that only asks, and the one write this frame still
        // owes `self.picking` is named where it happens rather than folded into
        // the asking.
        let facts_started = Instant::now();
        let prepared = self.prepare_frame(started, camera);
        let facts_cost = facts_started.elapsed();
        self.publish_frame_picks(&prepared.facts);
        let PreparedFrame {
            started,
            camera,
            facts,
        } = prepared;
        let FrameFacts {
            watched,
            cutaway,
            pick,
            drawn_mobiles,
            on_mobile: _,
            on_item: _,
            held_mobile,
            held_item,
        } = facts;

        // # Step three: present. Nothing below this line writes the world.
        //
        // The UI first, because it is what the surface is composited from
        // bottom-up and because its layout is what next frame's viewport comes
        // from. Its request is *held* rather than applied — see [`App::pending`].
        //
        // Timed, and separately from the world below: the two halves of a frame
        // are built by two things that grow for different reasons, and a single
        // build time cannot say which of them ate the frame. See [`frames`].
        //
        // The `Instant`s from here down are instrumentation and not a clock the
        // picture depends on: they measure what this frame cost, and no position
        // in it is a function of them. The one sampling of time that the frame is
        // built from is `started`, at the top.
        let ui_started = Instant::now();
        let (hud, hud_timings) = self.hud(camera, &pick, &cutaway, drawn_mobiles.as_deref());
        let ui_hud_cost = ui_started.elapsed();
        let painting = self.window.as_ref().map(|screen| Arc::clone(&screen.window));
        let ui_layout_started = Instant::now();
        let ui = match (self.shell.as_mut(), painting.as_ref()) {
            (Some(shell), Some(window)) => {
                let (request, output) = shell.run(window, &hud, camera, &self.world);
                let viewport = shell.viewport();
                Some((request, output, viewport))
            }
            _ => None,
        };
        let ui_layout_cost = ui_layout_started.elapsed();
        let mut ui_cost = ui_hud_cost + ui_layout_cost;
        if let Some((request, _, _)) = &ui {
            self.pending = request.clone();
        }

        // The Light tab's own numbers, which live in the shell — read here,
        // once for the whole frame: the flames, the ambient and the sun below
        // are all turned by them, and so is `want` just below, since the
        // atlases have to be grown for the same bound `light::collect` reads
        // them over.
        let tuning = self.tuning();
        // The producer needs the same static-impostor mode as the camera
        // frame. Lighting itself is applied later from the restored G-buffer,
        // but a lit frame's map statics still need their real box intersection
        // instead of the daylight billboard fallback.
        let producer_sky = match (self.graphics.night, self.graphics.sunlit) {
            (true, _) => Some(light::NIGHT),
            (false, true) => Some(light::SKYLIGHT),
            (false, false) => self.graphics.show_solids.then_some(light::Ambient::DAY),
        };
        let producer_sky = self
            .graphics
            .sky_field
            .then_some(producer_sky)
            .unwrap_or_else(|| producer_sky.map(light::Ambient::flattened));
        // The Chat tab's own numbers, the same reason and the same place:
        // gathered before the window is borrowed below, since `App::chat_style`
        // also reads the whole of `self`.
        let chat_style = self.chat_style();
        // What the camera has walked onto since the atlases were last grown.
        // Gathered before the window is borrowed, and not inside the borrow: it
        // reads the whole of `self`, and the window is part of it.
        let want = light::lit_tiles(&camera, &tuning);
        let wanted = self.wanted_since(camera, &tuning, self.graphics.covered);
        // Only schedule immutable map-block work here.  `refresh` merely
        // reprioritises bounded requests; it does not build or upload pixels,
        // so a newly exposed far-zoom block continues through the detailed
        // representation until an idle producer has completed its composite.
        // The completed image enters `Screen::composites` through this queue;
        // Work 4 owns drawing that ready texture in the depth-aware world pass.
        let map_width = self.resources.map.width() as u32;
        let map_height = self.resources.map.height() as u32;
        let map_tiles = openshard_client_render::camera::TileBounds {
            min_x: 0,
            max_x: map_width.saturating_sub(1) as i32,
            min_y: 0,
            max_y: map_height.saturating_sub(1) as i32,
        };
        let composite_visible = MapBlockBounds::from_tiles(camera.visible_tiles(), map_width, map_height);
        // Producer coverage is proven through the real-map capture/restore
        // oracle in `tests/frame.rs`. Roll out the first cache tier only: a
        // far enough camera may request LOD1, while the selector continues to
        // retain its LOD2 hysteresis state for that tier's later validation.
        let selected_composite_lod = self.composite_lod.update_camera(&camera);
        let composite_lod = visible_composite_lod(selected_composite_lod);
        // A composite stores final map pixels and deferred facts, not atlas
        // UVs. Static-atlas pages are append-only, so packing art for a newly
        // entered block cannot alter a completed block composite. In
        // particular, do not key this cache to the atlas's growth revision:
        // at far zoom each scroll would otherwise discard the whole visible
        // LOD working set merely because one new sprite was packed.
        let composite_revision = ImmutableRevision(self.graphics.fringe as u64);
        if let (Some(visible), Some(map)) = (
            composite_visible,
            MapBlockBounds::from_tiles(map_tiles, map_width, map_height),
        ) {
            let composites = self.window.as_ref().map(|window| &window.composites);
            self.composite_work
                .refresh(visible, map, selected_composite_lod, composite_revision, |key| {
                    composites.is_some_and(|cache| cache.get(key).is_some())
                });
        }
        // The producer owns its own command buffer below. Keep this empty so
        // `encode_world_passes` cannot revive the retired camera-frame capture
        // path.
        let composite_jobs = Vec::new();
        let mut drawn = self.drawn_mobiles();
        // Likewise: the cut the solids view is drawn under reads the player, and
        // the pass that uses it runs inside the window's borrow.
        let solid_cut = self.solid_cut();

        let Some(window) = self.window.as_mut() else {
            return;
        };
        let atlases_started = Instant::now();
        let (repacked, atlas_work) = ready_atlases(
            &mut self.resources,
            &mut self.graphics,
            &mut self.world,
            &mut self.repacks,
            window,
            want,
            &wanted,
            &drawn,
        );
        // Full GPU readback fences the device.  It is useful for the explicit
        // field audit, but must not change the timing of the ordinary injected
        // slow-pan scenario whose purpose is to expose asynchronous churn.
        let atlas_audit_due = std::env::var_os("OPENSHARD_ATLAS_AUDIT").is_some()
            && self.lod_sweep.as_mut().is_some_and(|sweep| {
                if !sweep.atlas_soak || sweep.elapsed < sweep.next_atlas_audit {
                    return false;
                }
                sweep.next_atlas_audit = sweep.elapsed + Duration::from_secs(2);
                true
            });
        if window.composite_output_format != blit::WORLD_FORMAT {
            window.composites.clear();
            self.composite_work.clear();
            window.composite_output_format = blit::WORLD_FORMAT;
        }
        // Prepare at most one immutable block's art in the same stable order
        // the eventual producer will dispatch.  This appends to atlas pages
        // and uploads only their dirty rows; a full/page-limited atlas does
        // not take the ordinary frame's rebuild route for a background job.
        // The job remains pending until an independent offscreen map draw can
        // consume the prepared inputs, so this does not re-enable the former
        // camera-frame capture path.
        if cutaway == Cutaway::OPEN {
            for work in self.composite_work.preparation_candidates() {
                let prepared = prepare_composite_job(
                    &mut self.resources,
                    window,
                    &self.world.presentation.tile_animations,
                    CompositeProducerJob::new(work.key),
                );
                if prepared {
                    self.composite_work.mark_prepared(work.key);
                }
            }
            let producer_jobs = self.composite_work.take_marked_prepared_for_frame();
            for work in producer_jobs {
                produce_composite_block(
                    &self.resources,
                    &self.world.presentation.tile_animations,
                    &tuning,
                    producer_sky,
                    self.graphics.fringe,
                    window,
                    &mut self.composite_work,
                    work,
                );
            }
        }
        // Three time-varying halves of a mobile, filled in per frame rather
        // than per packet: the crowd is the only thing that knows what a
        // clock — and a group — has done since the `0x77` landed, and
        // `self.world.presentation.player`/`self.world.presentation.others` were built when it did. Against the atlas
        // as it stands *after* this frame's growth, which is the one the
        // picture below is drawn from.
        Self::advance_to_clocks(
            &self.world.presentation.crowd,
            &window.atlases.mobiles,
            self.world.me(),
            &self.world.motion,
            &mut drawn,
        );
        // Whoever the crowd is still holding a line for, hung above whichever
        // of `drawn`'s mobiles their serial belongs to. Read out here, before
        // `who` is dropped below: a label with no mobile to anchor to has
        // nothing to draw either way, so the two share the same "still on
        // screen" question `mobiles::head_anchor` answers.
        let mut overhead: Vec<(ViewPixel, String, Font, Hue)> = drawn
            .iter()
            .filter_map(|(who, mobile)| {
                let (text, font, hue) = self.world.presentation.crowd.speaking(*who)?;
                let anchor = mobiles::head_anchor(mobile, &camera, &window.atlases.mobiles)?;
                Some((anchor, text.to_string(), font, hue))
            })
            .collect();
        // A combat number follows the same mobile anchor as speech, but its
        // y-coordinate is aged every frame so it rises smoothly rather than
        // moving only when the network sends another packet.
        for number in &self.world.presentation.damage_numbers {
            if let Some((_, mobile)) = drawn.iter().find(|(who, _)| *who == Some(number.serial)) {
                if let Some(mut anchor) = mobiles::head_anchor(mobile, &camera, &window.atlases.mobiles) {
                    let progress = number.elapsed.as_secs_f32() / DAMAGE_NUMBER_HOLD.as_secs_f32();
                    anchor.y -= (DAMAGE_NUMBER_RISE as f32 * progress) as i32;
                    overhead.push((anchor, number.amount.to_string(), Font::DEFAULT, Hue(0x0022)));
                }
            }
        }
        // **The crowd, or none of it** — `frame::Draw::mobiles`, which this
        // function honours because `frame::assemble` does not collect mobiles at
        // all. Emptied here and not at each of the three uses below, so that the
        // picture, the ring and the outline cannot disagree about who is in the
        // frame: a body left out of the world image and still ringed would be a
        // halo round nothing.
        //
        // The speech above is deliberately *not* filtered by it. A label is not a
        // thing standing in the street — `Kind::Nothing`, see `crate::place::Kind`
        // — and turning the crowd off to look at a wall is not a request to go
        // deaf.
        let drawn: Vec<Mobile> = match self.graphics.drawing.mobiles {
            true => drawn.into_iter().map(|(_, mobile)| mobile).collect(),
            false => Vec::new(),
        };
        let atlases_cost = atlases_started.elapsed();

        // The vsync wait, and the reason it is timed on its own: under
        // `PresentMode::Fifo` this call blocks until the display has taken the
        // frame before it, which on an idle client is most of the interval.
        // Counted as build time it would report a client that is asleep as one
        // at full load, and the panel exists to tell those two apart.
        let acquire_started = Instant::now();
        let frame = match window.surface.get_current_texture() {
            // Suboptimal still draws: the surface wants reconfiguring, and the
            // next resize event will do it.
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                frame
            }
            // The swapchain no longer matches the window. Rebuild it and let the
            // next redraw use it; drawing into a stale one is a crash on some
            // backends and a stretched frame on others.
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                window.surface.configure(&window.device, &window.config);
                return;
            }
            // Nothing was acquired and nothing is wrong: the window is hidden,
            // or the compositor took too long. Skipping the frame is the answer.
            other => {
                if !matches!(
                    other,
                    wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded
                ) {
                    eprintln!("acquiring a frame: {other:?}");
                }
                return;
            }
        };
        let wait = acquire_started.elapsed();
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Where the world goes on the surface: the rect the panels left free, so
        // a docked panel shrinks the world rather than covering it.
        let viewport = ui.as_ref().map_or(
            ViewportRect {
                x: 0,
                y: 0,
                width: window.config.width,
                height: window.config.height,
            },
            |(_, _, viewport)| *viewport,
        );

        // The image the world is drawn into. Its size is the camera's, so a
        // resize and a zoom step are the same event here — and recreating it is
        // the only thing either of them costs.
        //
        // Magnified it is the *viewport's* size and the magnification rides in
        // the vertex transform, so the world is drawn at the display's own
        // resolution and the blit below is a copy; minified it is the world's
        // own larger extent and the blit shrinks it. `docs/camera.md` D11 is the
        // argument, and the short of it is that an image of virtual resolution
        // cannot express an offset of one real pixel — which is the whole of
        // what made a magnified scroll coarser than the screen it was on.
        let targets_started = Instant::now();
        let (render_width, render_height) = camera.image_size();
        if window.world.width() != render_width || window.world.height() != render_height {
            window.world = blit::world_texture(&window.device, render_width, render_height);
            window.cutaway_world = blit::world_texture(&window.device, render_width, render_height);
            // Tested pixel for pixel against that image, so it is exactly its
            // size or it is nothing.
            window.depth = renderer::depth_texture(&window.device, render_width, render_height);
            // And the mask with it: it is the colour attachment of a pass whose
            // depth attachment is that buffer, and wgpu requires the two to be
            // one size.
            window.outline_mask = outline::mask_texture(&window.device, render_width, render_height);
            window.select_mask = outline::mask_texture(&window.device, render_width, render_height);
            window.held_mask = outline::mask_texture(&window.device, render_width, render_height);
            // And the G-buffer, whose planes are attachments of those same
            // passes and are read texel for texel against that image.
            window.gbuffer = Gbuffer::new(&window.device, render_width, render_height);
            window.cutaway_gbuffer = Gbuffer::new(&window.device, render_width, render_height);
        }
        let world_view = window.world.create_view(&wgpu::TextureViewDescriptor::default());
        let cutaway_world_view = window
            .cutaway_world
            .create_view(&wgpu::TextureViewDescriptor::default());
        let targets_cost = targets_started.elapsed();

        // **The frame's occluders are built before its pictures are collected**,
        // and that ordering is `docs/lighting_height.md` phase 3's one real cost.
        // A static's drawn row now carries the number this grid gave it
        // (`occlusion::Occlusion::owner_at`), so that a fragment of it can say
        // which occluder it is a point of instead of having that guessed from its
        // height; collecting the pictures first would stamp numbers off the grid
        // of the frame before. Nothing else about either step changed — the
        // statics used to go first for no reason anyone recorded.
        //
        // The lights come from the same camera, cutaway and item list the passes
        // below draw from, so a torch that was not drawn casts nothing and a
        // torch that was is lighting the pixels it is standing in rather than the
        // pixels it stood in last frame.
        //
        // `assemble_geometry` is a free function for the same reason
        // `ready_atlases` is one: it is handed `&mut self.graphics` only for
        // the one field it writes (`occlusion_bake`, through
        // `frame::Inputs::bake`), and every other field it reads is a plain
        // `&`, so the signature alone says this is not a place `self.world`
        // or `self.resources` change.
        let geometry_started = Instant::now();
        let geometry = assemble_geometry(
            &self.resources,
            &mut self.graphics,
            &mut self.world,
            &self.picking,
            window,
            camera,
            &cutaway,
            &tuning,
            pick.item,
            pick.mobile,
            held_item,
            held_mobile,
            &drawn,
        );
        let geometry_cost = geometry_started.elapsed();
        let assembly_costs = geometry.assembly_costs;
        // `geometry` is kept whole rather than destructured here: it travels
        // to `encode_world_passes` and, on the F12 path below, to the dump —
        // both read it as the one value `assemble_geometry` built, not as a
        // dozen loose slices that happen to have arrived together.
        // `fonts.mul` or the operator-supplied TrueType face, never a mix
        // within one frame — see `run`'s doc for why `ttf_font` is an
        // all-or-nothing switch. `fonts.mul` still draws into the world
        // image, at the world's own camera-scaled zoom — a bitmap font's
        // blocky nearest-sampled magnification is the look every other
        // sprite already has. A TrueType face does not go through the world
        // passes at all any more: `screen_speech` is collected in real
        // screen pixels instead, held until the HUD block further down folds
        // it into `hud_quads` for `Screen::ttf_gump_pass`'s one call — see
        // `text::ScreenLabel`'s doc for why the pass and `hud_quads`'s own
        // comment for why it has to be one call.
        let encode_started = Instant::now();
        let (text_quads, screen_speech): (Vec<SpriteQuad>, Vec<text::ScreenLabel<'_>>) =
            match &self.resources.ttf_font {
                Some(font) => {
                    let atlas = window
                        .ttf_atlas
                        .as_mut()
                        .expect("create_window builds ttf_atlas whenever ttf_font is set");
                    // Unlike `font_atlas`, `ttf_atlas` is grown a line at a time:
                    // there is no bounded "whole file" to pack up front for a
                    // face that answers to all of Unicode, so this asks it to
                    // rasterize whatever of this frame's speech it has not seen
                    // yet, the way `window.atlases` grows for graphics newly on
                    // screen.
                    if let Err(error) =
                        atlas.add(font, overhead.iter().flat_map(|(.., line, _, _)| line.chars()))
                    {
                        // `eprintln!` and a frame that draws anyway, the same
                        // corner `AtlasError::Full` already cuts for the map's
                        // own atlases — see docs/client.md. Unreachable in
                        // practice: a shard's whole spoken character set is a
                        // few hundred glyphs at most, nowhere near one 2048
                        // texture.
                        eprintln!("packing ttf glyphs: {error}");
                    }
                    let screen_speech = overhead
                        .iter()
                        .map(|(anchor, line, _font, hue)| {
                            // `to_viewport` and not the projection directly:
                            // it is the one place that already undoes both a
                            // magnifying zoom's vertex-shader scale *and* a
                            // minifying one's blit-shrink with the same number
                            // — see its own doc. `viewport`'s own corner is
                            // added because `to_viewport` answers in pixels of
                            // the rect the world goes into, not the surface.
                            let real = camera.to_viewport(*anchor);
                            text::ScreenLabel {
                                anchor: GumpPixel::new(
                                    viewport.x as i32 + real.x.round() as i32,
                                    viewport.y as i32 + real.y.round() as i32,
                                ),
                                text: line.as_str(),
                                hue: *hue,
                            }
                        })
                        .collect();
                    (Vec::new(), screen_speech)
                }
                None => {
                    let labels: Vec<Label<'_>> = overhead
                        .iter()
                        .map(|(anchor, line, font, hue)| Label {
                            anchor: *anchor,
                            text: line.as_str(),
                            font: *font,
                            hue: *hue,
                            // Nearer than anything the world draws, rather than
                            // an `Order` of its own: speech reads as an overlay
                            // above whoever said it in every reference client,
                            // and there is no real case here of a wall in front
                            // of the speaker hiding it that a viewer would want
                            // honoured. Worth revisiting with a
                            // `depth::text_priority_z` alongside the mobile's
                            // own if that ever stops being true.
                            depth: 0.0,
                        })
                        .collect();
                    (text::collect(&labels, &self.resources.font_atlas), Vec::new())
                }
            };
        // Uploads whatever the `add` above (and the HUD's own, further down
        // this frame) packed fresh — see `Screen::upload_ttf_dirty`'s doc for
        // why this is the one place both call through rather than each taking
        // `TtfAtlas::take_dirty` for itself.
        window.upload_ttf_dirty();
        let depth_view = window.depth.create_view(&wgpu::TextureViewDescriptor::default());
        let gbuffer_views = window.gbuffer.views();
        let cutaway_gbuffer_views = window.cutaway_gbuffer.views();
        let target = Target {
            view: &world_view,
            depth: &depth_view,
            gbuffer: &gbuffer_views,
            width: render_width,
            height: render_height,
            projection: camera.projection(),
        };
        let mut encoder = window
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        // `encode_world_passes` is a free function for the same reason
        // `assemble_geometry` is one: every world pass it records is drawn
        // from the values handed to it, and the one thing on `self` it
        // writes — `graphics.solids_held`/`graphics.solids_drawn` — is
        // written through the `&mut GraphicsSettings` its signature already
        // names, not through a `&mut self` that would let it touch anything
        // else too.
        encode_world_passes(
            &mut self.graphics,
            &self.picking,
            window,
            &mut encoder,
            target,
            &view,
            &world_view,
            &gbuffer_views,
            &cutaway_world_view,
            &cutaway_gbuffer_views,
            viewport,
            camera,
            solid_cut,
            &geometry,
            &text_quads,
            render_width,
            render_height,
            composite_lod,
            composite_revision,
            composite_visible,
            &composite_jobs,
            &mut self.composite_work,
        );
        // The shard's dialogs, in the client's own art, over the finished
        // picture and under egui's.
        //
        // Under egui and not over it, deliberately: the widgets that *answer* a
        // gump are still egui's, laid out at the same coordinates in the same
        // units — one gump pixel is one egui point, and the scale below is the
        // window's own scale factor, which is what makes those two spaces the
        // same one. So the art draws the window and egui's transparent widgets
        // sit exactly on it. See `client/app/src/gump.rs`.
        //
        // The atlas grows here rather than when the packet arrived: a page
        // button flips pages inside the client, so what a window needs is every
        // page's art and not the showing one's — `gump::art_of` is that list,
        // and it is asked for on the frame the window is drawn on because that
        // is the frame that knows the window is open at all.
        // `draw_gump_windows` is a free function for the same reason as its
        // neighbours above: `resources.gump_atlas` and `windows.drawn_windows`
        // are the two things on `self` it really writes, and both are named
        // in its signature rather than reached through `&mut self`.
        draw_gump_windows(
            &mut self.resources,
            &self.world,
            &mut self.windows,
            self.shell.as_ref(),
            window,
            &mut encoder,
            &view,
        );
        // Every line of gump-space text this frame, from both the blocks below.
        //
        // **One list because there is one pass.** `GumpRenderer` holds a single
        // instance buffer, and `queue.write_buffer` lands before the encoder is
        // submitted — so two `render` calls in a frame do not draw twice, they
        // draw the *second* call's instances twice and lose the first's. That
        // is what happened to every window's text for as long as there was a
        // line in the journal to overwrite it with: a paperdoll's name plate
        // was written, cut, submitted, and then quietly replaced by the chat.
        let mut text_quads: Vec<SpriteQuad> = Vec::new();
        // What the windows have written on them: a dialog's captions and
        // fields, and a paperdoll's name.
        //
        // A second pass over the same surface rather than more quads in the one
        // above, because a draw call binds one texture and a glyph lives in the
        // font atlas. After the art and therefore over it, which is the order
        // the reference draws a window's text controls in — and after the block
        // above rather than inside it, because that one holds the gump pass
        // mutably and this needs the text pass.
        //
        // Read off `drawn_windows` — the list the frame was just drawn from and
        // the one the pointer will be tested against — so a caption cannot end
        // up on a window laid out differently from the pictures under it.
        {
            let mut labels: Vec<openshard_client_render::text::GumpLabel<'_>> = Vec::new();
            // The lines that are cut to a box before they are drawn, already
            // quads: they cannot travel with the labels above, because what
            // cuts them is the window they belong to and a label does not carry
            // one. Extended onto the same draw call, which is the point — one
            // texture, one pass, whatever produced the quads.
            let mut cut: Vec<SpriteQuad> = Vec::new();
            for (subject, drawn) in &self.windows.drawn_windows {
                match (subject, drawn) {
                    (WindowSubject::Dialog(gump_id), Drawn::Dialog(laid_out)) => {
                        if let Some(gump) = self
                            .world
                            .authoritative
                            .view
                            .as_ref()
                            .and_then(|view| view.gumps.iter().find(|gump| gump.gump_id == *gump_id))
                        {
                            labels.extend(self.windows.dialogs.lines(
                                gump,
                                laid_out,
                                &self.resources.font_atlas,
                                self.resources.cliloc.as_ref(),
                            ));
                        }
                    }
                    (WindowSubject::Paperdoll(serial), Drawn::Paperdoll(_)) => {
                        // The name the `0x88` carried, which is the only place
                        // this string exists — see `view::Paperdoll::name`. The
                        // window's own origin comes off `own_windows` rather
                        // than the doll, for the reason the plate is part of the
                        // frame: it moves with the window and not with the body.
                        let at = self
                            .windows
                            .own_windows
                            .iter()
                            .find(|window| window.subject == *subject)
                            .map(|window| window.at);
                        if let (Some(at), Some(doll)) = (
                            at,
                            self.world
                                .authoritative
                                .view
                                .as_ref()
                                .and_then(|view| view.paperdolls.get(serial)),
                        ) {
                            labels.push(paperdoll::name(&doll.name, at));
                        }
                    }
                    // The skill window writes its own lines — a heading, a
                    // name, a value, the total — and they are the one text in
                    // this client that is *cut*: a row half out of the viewport
                    // is drawn half, which is what a bar the player drags means.
                    // Collected and cut here rather than added to `labels`,
                    // because a glyph is a quad in the font atlas and has no
                    // picture to carry a box on — see `Scissor::cut`.
                    (WindowSubject::Skills, Drawn::Skills(sheet)) => {
                        for line in &sheet.lines {
                            let mut quads = openshard_client_render::text::collect_gump(
                                &[line.label()],
                                &self.resources.font_atlas,
                            );
                            // Its own box, and not the window's: the rows are cut
                            // to the list and the total written under them is not
                            // — see `skills::Line::scissor`, which is a
                            // difference this found out by drawing it wrong.
                            if let Some(scissor) = line.scissor {
                                scissor.cut(&mut quads);
                            }
                            cut.extend(quads);
                        }
                    }
                    (WindowSubject::Status, Drawn::Status(status)) => {
                        labels.extend(status.lines.iter().map(|line| line.label()));
                    }
                    _ => {}
                }
            }
            text_quads.extend(openshard_client_render::text::collect_gump(
                &labels,
                &self.resources.font_atlas,
            ));
            text_quads.extend(cut);
        }
        // `draw_chat_and_speech` is a free function like its neighbours
        // above, though a plainer one: nothing it is handed is written back
        // to `self` at all, only appended to the caller's own `text_quads`.
        draw_chat_and_speech(
            &self.resources,
            &self.world,
            &self.chat,
            self.shell.as_ref(),
            window,
            &mut encoder,
            &view,
            chat_style,
            &screen_speech,
            &mut text_quads,
        );
        let encode_cost = encode_started.elapsed();
        // The UI over it, with no depth attachment: the world's depth buffer
        // ordered the world, and this is drawn on the result.
        if let (Some(shell), Some((_, output, _))) = (self.shell.as_mut(), ui) {
            let painting = Instant::now();
            let timed = profile::begin(window.gpu.as_ref(), "egui", &mut encoder);
            shell.paint(
                &window.device,
                &window.queue,
                &mut encoder,
                &view,
                output,
                [window.config.width, window.config.height],
            );
            profile::end(window.gpu.as_ref(), &mut encoder, timed);
            ui_cost += painting.elapsed();
        }
        let ui_paint_cost = ui_cost.saturating_sub(ui_hud_cost).saturating_sub(ui_layout_cost);
        // Every query closed above, copied out of its set and into the buffer
        // the next frame will map — recorded into this encoder, so it has to
        // happen before the submit and after the last `profile::end`.
        if let Some(gpu) = window.gpu.as_mut() {
            gpu.resolve(&mut encoder);
        }
        window.queue.submit([encoder.finish()]);
        if atlas_audit_due {
            audit_static_atlas_pages(window);
            audit_scene_instance_buffers(window);
            if std::env::var_os("OPENSHARD_LOD_SCREEN_AUDIT").is_some() {
                audit_visible_ground_centres(window, &self.resources.map, camera);
            }
        }
        // And the frame closed, which is what makes those buffers eligible to be
        // mapped. What comes back is an older frame's timings — see [`profile`]
        // for why that is the right trade and not a defect.
        if let Some(gpu) = window.gpu.as_mut() {
            gpu.end_frame(&window.device, &window.queue);
        }
        // **This frame, written out** — F12, and `docs/parity.md`'s first
        // backlog item. After the submit above and not beside the blit, because
        // what is read back has to be pixels the device has actually been given
        // the commands for; the world image, the G-buffer and the instance
        // buffers all still hold this frame's own, since nothing writes them
        // again until the next one.
        //
        // Not the surface: what is presented has the HUD, the panels and the
        // solids overlay on top of it, and a tool's frame has none of those.
        // What a comparison wants is the world as the blit left it, so the blit
        // is run again into a texture of its own — the same pass, the same
        // lighting, the same rect — once per plane. `docs/parity.md` D5.
        if let Some(into) = self.graphics.frame_dump.take() {
            let dump = window.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("frame dump"),
                size: wgpu::Extent3d {
                    width: window.config.width,
                    height: window.config.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // **The world's format and not the surface's**, which is what
                // the first press of F12 found: a surface is whatever the
                // compositor offered — here `Rgba16Float`, eight bytes a texel
                // — and reading it back as RGBA8 is a copy `wgpu` refuses. Even
                // where it is four (`Bgra8Unorm`) it is the wrong four: the
                // picture would come out with its red and blue swapped, and
                // nothing would say so. `isolated_scene` has always drawn into
                // this format, and a dump exists to be compared with that one.
                format: blit::WORLD_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let dump_view = dump.create_view(&wgpu::TextureViewDescriptor::default());
            // A second pipeline for that format, built here and dropped with the
            // dump: `Screen::blit` is bound to the surface's format and cannot
            // draw into this target. The same shader and the same uniforms — the
            // format is the whole of the difference — and it is built per press
            // rather than kept, because every frame that is not being dumped
            // would otherwise carry a pipeline nobody draws with.
            let mut dump_blit = Blit::new(&window.device, blit::WORLD_FORMAT);
            let planes = openshard_client_render::dump::planes(
                &window.device,
                &window.queue,
                &mut dump_blit,
                &dump,
                blit::Frame {
                    // A view of `dump`, made one line above it: `dump::planes`'s
                    // own contract, and the whole of what this call site has to
                    // keep true.
                    target: &dump_view,
                    world: &world_view,
                    gbuffer: &gbuffer_views,
                    face_instances: window.statics.instances_buffer(),
                    item_instances: window.items_pass.instances_buffer(),
                    mobile_instances: window.mobile_pass.instances_buffer(),
                    mesh_instances: window.mesh_pass.rows_buffer(),
                    ground_instances: window.renderer.instances_buffer(),
                    zoom: camera.zoom(),
                    rect: viewport,
                },
                &geometry.lighting,
                // Every one of them. A dump taken because something looked wrong
                // is taken once, and a plane left out is a plane somebody has to
                // reproduce the moment they want it — by which time the frame is
                // gone.
                &View::ALL,
            );
            match write_frame_dump(
                &into,
                &planes,
                geometry
                    .asked_for
                    .as_deref()
                    .expect("the summary is taken whenever a dump is armed, above"),
            ) {
                Ok(()) => tracing::info!(
                    into = %into.display(),
                    planes = planes.len(),
                    "frame dumped",
                ),
                // A dump that could not be written is a diagnostic that failed,
                // not a frame that failed: the client goes on drawing.
                Err(error) => tracing::warn!(into = %into.display(), %error, "dumping the frame"),
            }
        }
        // Presentation moved onto the queue in wgpu 30; the texture is consumed.
        window.queue.present(frame);
        // And the next frame is asked for here rather than through the timer,
        // unconditionally while somebody is watching. This is the pacer: the
        // surface presents in FIFO, so `get_current_texture` above blocks the
        // next frame until the display has taken this one, and asking again
        // straight away runs the loop at the display's own rate instead of at a
        // 16ms timer that beats against it.
        //
        // Every frame and not only the gliding ones, which is the change: a
        // client that only redrew when something moved dropped to 12.5 frames a
        // second the moment the player stood still, and however correct the
        // reason was, what it looked like was a stall. The timer stays for the
        // window nobody is looking at — see [`App::pacing`].
        if watched {
            window.window.request_redraw();
        }
        let took = started.elapsed();
        // The interval between two *drawn* frames, and where this one's time
        // went: the pacing and the price, which are the two things a drop in
        // frame rate can be — and the price split between the panels and the
        // world, which are the two things the price can be. See [`frames`].
        //
        // The scene is what is left after the UI and the wait rather than a
        // fourth clock, so the three always add up to the frame exactly: a
        // fourth `Instant` would leave a remainder nobody could account for.
        let scene = took.saturating_sub(ui_cost).saturating_sub(wait);
        // The device's own number, which is *not* about this frame: it is
        // whichever frame the timestamps have come back for, two or three ago.
        // Recorded against this one anyway, because what it answers — "is the
        // wait above slack or a stall" — is a question about a standing cost and
        // not about one frame's spike. See [`profile`].
        let gpu = self
            .window
            .as_ref()
            .and_then(|window| window.gpu.as_ref())
            .map(profile::Gpu::total);
        self.frames.record(
            started.saturating_duration_since(self.last_frame),
            ui_cost,
            scene,
            wait,
            gpu,
            repacked,
        );
        if let Some(frame) = self.frames.frames().last().copied() {
            let gpu_passes = self
                .window
                .as_ref()
                .and_then(|window| window.gpu.as_ref())
                .map_or(&[][..], crate::profile::Gpu::passes);
            crate::jank::record(
                frame,
                crate::jank::CpuPasses {
                    ui_hud: ui_hud_cost,
                    ui_terrain: hud_timings.terrain,
                    ui_route: hud_timings.route,
                    ui_occluders: hud_timings.occluders,
                    ui_picking: hud_timings.picking,
                    ui_perf: hud_timings.perf,
                    ui_layout: ui_layout_cost,
                    ui_paint: ui_paint_cost,
                    facts: facts_cost,
                    atlases: atlases_cost,
                    targets: targets_cost,
                    geometry: geometry_cost,
                    lighting: assembly_costs.lighting,
                    ground: assembly_costs.ground,
                    statics: assembly_costs.statics,
                    static_walk: assembly_costs.static_walk,
                    static_sort: assembly_costs.static_sort,
                    items: assembly_costs.items,
                    encode: encode_cost,
                },
                atlas_work,
                gpu_passes,
            );
        }
        self.last_frame = started;
    }
}

/// Where a frame dump goes: `OPENSHARD_FRAME_DUMP_DIR`, or a directory of our
/// own under the system temp.
///
/// Never the source tree — one dump is thirteen uncompressed pictures and none
/// of them belongs in a diff. The same rule, and the same shape, as
/// [`dst::dump_dir`](crate::dst)'s.
///
/// **Not `OPENSHARD_FRAME_DUMP`**, which the render crate's own tools already
/// read as the *file* their one picture is written to
/// (`examples/isolated_scene.rs`, `tests/cost.rs`). One name meaning a file to
/// one caller and a directory to another is precisely the quiet difference
/// `docs/parity.md` exists to stop, so the client's knob is its own name — and a
/// directory, because what the client has to dump is every plane at once plus
/// the inputs they came from.
pub(crate) fn frame_dump_root() -> std::path::PathBuf {
    std::env::var_os("OPENSHARD_FRAME_DUMP_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("openshard-frame"))
}

/// One dump: a directory holding a picture per plane, named for the plane, and
/// the inputs the frame was assembled from.
///
/// The summary is written last on purpose — a directory that has `inputs.txt` in
/// it has every picture beside it, so a reader never compares a half-written
/// dump against a whole one.
///
/// **`inputs.txt` is written verbatim and gets no line of its own from here**,
/// which is worth stating because one line of it reads oddly beside the
/// directory: `view` is what the *window* was showing when the key was pressed,
/// while each picture beside it is named for the plane it actually is. Adding a
/// note to explain that would be a line the tool's own summary does not have,
/// and the two are written to be diffed — an extra line here is a difference in
/// every comparison, forever, to save one sentence of documentation.
pub(crate) fn write_frame_dump(
    into: &std::path::Path,
    planes: &[(View, Vec<u8>)],
    asked_for: &str,
) -> std::io::Result<()> {
    std::fs::create_dir_all(into)?;
    for (view, png) in planes {
        std::fs::write(into.join(format!("{}.png", view.name())), png)?;
    }
    std::fs::write(into.join("inputs.txt"), asked_for)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openshard_client_render::follow::Gaze;
    use openshard_protocol::direction::{Direction, Facing};
    use openshard_protocol::wire::{Graphic, Hue};
    use openshard_protocol::world::Point;
    use openshard_uofiles::anim::BodyKind;

    #[test]
    fn local_render_projection_uses_game_motion_not_a_presentation_clock() {
        let start = Point::new(100, 100, 0);
        let end = Point::new(101, 100, 0);
        let east = Facing::walking(Direction::East);
        let mut motion = PlayerMotion::new(start, east);
        let mut player = Mobile {
            at: end,
            body: Graphic(0x0190),
            group: BodyKind::of(Graphic(0x0190)).standing(),
            facing: Direction::East,
            frame: openshard_uofiles::anim::AnimationFrameIndex(0),
            from: None,
            hue: Hue::NONE,
            // Deliberately an impossible stale presentation pose: the test
            // proves the frame projection replaces it from GameMotion alone.
            drawn: Gaze::on(start),
            equipment: Vec::new().into(),
        };

        motion.accept_trusted_step(end, east);
        motion.advance(openshard_movement::WALK_HOLD / 2);
        App::project_local_motion(&motion, &mut player);

        assert_eq!(player.drawn, motion.drawn());
        assert_ne!(player.drawn, Gaze::on(start));
        assert_eq!(player.from, Some(start));
    }

    #[test]
    fn visible_composite_gate_allows_lod_one_but_holds_lod_two() {
        assert_eq!(visible_composite_lod(BlockLod::Lod0), BlockLod::Lod0);
        assert_eq!(visible_composite_lod(BlockLod::Lod1), BlockLod::Lod1);
        assert_eq!(visible_composite_lod(BlockLod::Lod2), BlockLod::Lod1);
    }
}
