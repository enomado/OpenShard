//! Structured diagnostics for frames that missed the 60 Hz rendering budget.
//!
//! The event is intentionally emitted at the frame boundary rather than from a
//! particular pass: it keeps the CPU, swapchain and delayed GPU measurements in
//! one log record. The target is `jank`, so `RUST_LOG=jank=warn` selects these
//! records without enabling every warning from the client. A caller that needs
//! a persistent trace can initialise [`start_log`] before starting the app.

use crate::frames::{Frame, JANK_BUDGET};
use crate::profile::Pass;
use crate::window::AtlasWork;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

static LOG: OnceLock<Mutex<File>> = OnceLock::new();

/// CPU phases within [`crate::presentation::App::draw_from`] that are useful
/// when the single [`Frame::scene`] total says the world is slow.
///
/// They deliberately do not add up to a frame: UI, swapchain acquisition and
/// the advance step are accounted for separately by [`Frame`]. These are the
/// expensive, zoom-sensitive world-building phases where a more specific
/// answer is needed.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CpuPasses {
    pub facts: Duration,
    pub atlases: Duration,
    pub targets: Duration,
    pub geometry: Duration,
    pub lighting: Duration,
    pub ground: Duration,
    pub statics: Duration,
    pub static_walk: Duration,
    pub static_sort: Duration,
    pub items: Duration,
    pub encode: Duration,
}

/// Create a fresh file for jank records from this process.
///
/// Called by the playground before it opens the window, so each invocation has
/// one self-contained trace. The event path never opens files or reports write
/// errors: I/O must not turn a slow frame into a slower one.
pub fn start_log(path: &Path) -> io::Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    // A client app owns one event loop and its caller initializes this once.
    // Treat a second request in the same process as an ordinary no-op rather
    // than replacing a log another thread might be writing.
    let _ = LOG.set(Mutex::new(file));
    Ok(())
}

/// Write one event for a frame whose CPU or measured GPU work exceeded the
/// budget. GPU timestamps belong to an earlier submitted frame; they remain in
/// the record because a standing GPU cost, rather than exact per-frame pairing,
/// is what identifies a saturated device.
pub fn record(frame: Frame, cpu: CpuPasses, atlas: AtlasWork, gpu_passes: &[Pass]) {
    if !frame.janks() {
        return;
    }
    let ms = |duration: Duration| duration.as_secs_f64() * 1_000.0;
    let gpu_ms = frame.gpu.map(ms);
    let passes: Vec<_> = gpu_passes
        .iter()
        .map(|pass| (&pass.label, ms(pass.cost)))
        .collect();
    let cpu_passes = [
        ("facts", ms(cpu.facts)),
        ("atlases", ms(cpu.atlases)),
        ("targets", ms(cpu.targets)),
        ("geometry", ms(cpu.geometry)),
        ("lighting", ms(cpu.lighting)),
        ("ground", ms(cpu.ground)),
        ("statics", ms(cpu.statics)),
        ("static_walk", ms(cpu.static_walk)),
        ("static_sort", ms(cpu.static_sort)),
        ("items", ms(cpu.items)),
        ("encode", ms(cpu.encode)),
    ];
    let atlas_overflowed = atlas.overflow.map(|overflow| overflow.atlas);
    let atlas_packed_graphics = atlas.overflow.map(|overflow| overflow.packed_graphics);
    let atlas_newly_requested_graphics = atlas.overflow.map(|overflow| overflow.newly_requested_graphics);
    if let Some(log) = LOG.get() {
        if let Ok(mut log) = log.lock() {
            let _ = writeln!(
                log,
                "jank frame budget_ms={:.3} interval_ms={:.3} build_ms={:.3} ui_ms={:.3} \
                 scene_ms={:.3} wait_ms={:.3} gpu_ms={gpu_ms:?} repacked={} atlas_uploaded_bytes={} atlas_overflowed={atlas_overflowed:?} atlas_packed_graphics={atlas_packed_graphics:?} atlas_newly_requested_graphics={atlas_newly_requested_graphics:?} cpu_passes={cpu_passes:?} gpu_passes={passes:?}",
                ms(JANK_BUDGET),
                ms(frame.interval),
                ms(frame.build()),
                ms(frame.ui),
                ms(frame.scene),
                ms(frame.wait),
                frame.repacked,
                atlas.uploaded_bytes,
            );
            let _ = log.flush();
        }
    }
    tracing::warn!(
        target: "jank",
        budget_ms = ms(JANK_BUDGET),
        interval_ms = ms(frame.interval),
        build_ms = ms(frame.build()),
        ui_ms = ms(frame.ui),
        scene_ms = ms(frame.scene),
        wait_ms = ms(frame.wait),
        gpu_ms = ?gpu_ms,
        repacked = frame.repacked,
        atlas_uploaded_bytes = atlas.uploaded_bytes,
        atlas_overflowed = ?atlas_overflowed,
        atlas_packed_graphics = ?atlas_packed_graphics,
        atlas_newly_requested_graphics = ?atlas_newly_requested_graphics,
        cpu_passes = ?cpu_passes,
        gpu_passes = ?passes,
        "jank frame"
    );
}
