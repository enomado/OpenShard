//! The client, as far as it goes: a window onto Britannia's ground.
//!
//! Run it against a real client install, which this repository never contains:
//!
//! ```sh
//! OPENSHARD_CLIENT="/path/to/Ultima Online Classic" cargo run -p openshard-client-app
//! ```
//!
//! # A library with a binary on top
//!
//! Everything here is [`run`], and `main.rs` is the environment read into its
//! two arguments. The split is the one `crates/server/server` already made and
//! for the same reason: something that wants a client should call one rather
//! than build one. Today that something is `crates/e2e/playground`, which starts
//! a shard and a window in one process — and which could not exist at all while
//! the client was a binary, because nothing can depend on a `main`.
//!
//! Arrow keys walk a tile at a time, and shift runs. A right click is a move
//! order — the body walks to that tile on its own, and holding the button steers
//! it to wherever the cursor is; taking hold of the arrows cancels it. The wheel
//! zooms about the cursor, a middle-drag pans, page up and down pan vertically,
//! `Home` puts the camera back on the body and locks it there, and escape closes
//! the window.
//!
//! # The panels
//!
//! egui, over the world: a status strip, a camera window and a list of what the
//! [`WorldView`] is holding. A *dev* HUD and not this client's interface —
//! whether that is egui or the `0xB0` gump layer is M4's decision and
//! `shell.rs` is careful not to take it. What the panels leave free is the
//! world's viewport, so docking one shrinks the world rather than covering it.
//!
//! # With a shard, and without one
//!
//! Given an account it logs in and draws what the server has shown it — the
//! character, everyone else on screen, and the ground under them:
//!
//! ```sh
//! OPENSHARD_CLIENT=… OPENSHARD_ACCOUNT=admin OPENSHARD_PASSWORD=… \
//!     cargo run -p openshard-client-app
//! ```
//!
//! Then the arrows are a `0x02` each and the camera follows the body the server
//! confirms, not the keyboard. Without an account it stays what it was: a
//! window onto the map's own ground and statics, with one placeholder body
//! standing wherever the camera looks. Both are worth having — the offline one
//! needs no shard to look at a hillside, and it is the only one that runs
//! against a facet nobody is serving.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

mod crowd;
/// The walk, held against an oracle. Tests only — see its module docs.
#[cfg(test)]
mod dst;
mod frames;
mod gump;
mod keys;
mod link;
mod replay;
mod shell;
mod steer;

/// The camera this client opens with: the reference one, the eye on the body to
/// the pixel.
///
/// Which rig it *ships* is undecided and is decided on a bench rather than here
/// — `docs/camera.md` D9.
const STARTUP_RIG: Rig = Rig::HARD;

/// And how far the drawn body may lag the walk it is doing.
///
/// **Not a default in the sense D9 refuses.** D9 is about naming a camera before
/// one has won; this one was looked at — `dst::dump_the_ramp` is the table and
/// `docs/camera.md` C3 records the sitting. It is also not a camera: the eye is
/// still `HARD` above, and what eases is where the body is drawn (D10).
///
/// Here rather than in `crowd.rs` on purpose. [`Ease::WALK`] is a *setting* — a
/// number that was found to be right — and which setting a window opens with is
/// a decision about this binary. The two being one line apart in one file is how
/// a setting quietly becomes a default.
const STARTUP_EASE: crowd::Ease = crowd::Ease::WALK;

/// Read a `.env` from the working directory or an ancestor of it, if there is
/// one, so that the binaries' `env =` options have something to fall back to.
///
/// Call it before parsing a command line — that is the whole of the contract —
/// and from every binary that puts a window on a client install, which is why
/// it lives here rather than in one of them: `crates/e2e/playground` starts the
/// same client from the same `.env`.
///
/// **A missing file is not a failure and a malformed one is.** The two are one
/// `Result` in `dotenvy` and collapsing them with `.ok()` is how a quoting
/// mistake becomes "set OPENSHARD_CLIENT" from a shell where it *is* set: a
/// path with a space in it needs quotes, and without them the whole file is
/// dropped without a word. The line is printed rather than returned, because
/// the caller has not built anything to fail out of yet.
pub fn load_env() {
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(error) if error.not_found() => {}
        Err(error) => eprintln!("ignoring .env: {error}"),
    }
}

use crowd::{Crowd, Who};
use openshard_client_net::session::Plan;
use openshard_client_net::transport::Dial;
use openshard_client_net::view::WorldView;
use openshard_client_render::animation::FRAME_DELAY;
use openshard_client_render::atlas::{AnimAtlas, AtlasError, FontAtlas, LandAtlas, StaticAtlas, TexmapAtlas};
use openshard_client_render::bench::{self, Metrics, Scope, Script};
use openshard_client_render::blit::{self, Blit, ViewportRect};
use openshard_client_render::camera::{self, Camera, TileBounds, ViewPixel};
use openshard_client_render::control::{Control, Follow};
use openshard_client_render::follow::{Gaze, Rig};
use openshard_client_render::hue::HueRamp;
use openshard_client_render::items::{self, GroundItem};
use openshard_client_render::mobiles::{self, Mobile};
use openshard_client_render::renderer::{self, GroundRenderer, SpriteRenderer, Target};
use openshard_client_render::text::{self, Label};
use openshard_client_render::{ground, statics};
use openshard_protocol::direction::{Direction, Facing};
use openshard_protocol::speech::Font;
use openshard_protocol::version::ClientVersion;
use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::Point;
use openshard_uofiles::anim::Anim;
use openshard_uofiles::art::Art;
use openshard_uofiles::font::AsciiFonts;
use openshard_uofiles::hues::Hues;
use openshard_uofiles::map::Map;
use openshard_uofiles::texmaps::TexMaps;
use openshard_uofiles::tiledata::TileData;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// Where the camera starts: Britain, by the bank.
const START: Point = Point::new(1495, 1629, 0);

/// The facet to open. Felucca: `0x1B` carries the facet's *size* and not its
/// number, so a shard serving another one is noticed by the size test in
/// [`App::entered`] rather than followed.
const FACET: u8 = 0;

/// Which client this claims to be. Every `Feature` gate on the server follows
/// from it, and this is the one ClassicUO opens with — see `docs/client.md`.
const VERSION: ClientVersion = ClientVersion::new(7, 0, 45, 65);

/// How often to redraw while somebody is mid-step. See [`App::redraw_interval`].
///
/// Roughly a 60Hz display, and deliberately a number of our own rather than the
/// monitor's: nothing here knows the refresh rate, and asking the surface would
/// tie the animation to the present mode the adapter happened to offer.
const GLIDE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);

/// How much of the event loop's recent past the frame panel keeps.
///
/// The same four seconds as the scope, and for the same reason: what is worth
/// looking at is the last few steps, not the session. Its own constant because
/// the two rings answer different questions and one of them is about to grow a
/// slider — see `docs/camera.md`.
const FRAMES_SPAN: std::time::Duration = std::time::Duration::from_secs(4);

/// How much of the eye's recent past the scope keeps.
///
/// Long enough to hold a whole reversal — a step is 400ms and `back_and_forth`
/// turns round every one of them — and short enough that the curve on screen is
/// the last thing that happened rather than a session's worth of ink. Four
/// seconds is ten steps at a walk.
const SCOPE_SPAN: std::time::Duration = std::time::Duration::from_secs(4);

/// How many of the shard's last lines the speech panel shows.
///
/// Small on purpose: this is not the journal — see [`shell::Hud::said`] — it is
/// enough to read the answer to what was just typed. The journal itself is kept
/// whole in the [`WorldView`], capped there, and M4 is what displays it.
const SPEECH_LINES: usize = 6;

/// Open a window on `dir`'s files, and log in to `shard` if one is given.
///
/// The two arguments are the whole of what this run was asked for: which client
/// install to read, and whether there is a shard to play. Everything else — the
/// facet, the version claimed, where the camera starts — is a constant above,
/// because none of it is a decision a caller has ever needed to make differently.
///
/// A shard is a [`Dial`] and a [`Plan`] rather than an address and a plan: how
/// the connection is opened is the caller's, which is what lets
/// `crates/e2e/playground` hand over a shard in this same process. Nothing in
/// this crate knows what a socket is any more; `client/net` does not either.
///
/// This is a `-> ExitCode` and not a `-> Result`, because every failure here is
/// terminal for a *window*: no client files, no window system, no GPU. There is
/// nothing a caller could do with a typed error except print it, and printing it
/// is what the reasons already do. [`StartupError`] is the exception that proves
/// it — the failures *after* a window exists are types, because that is where
/// the same failure means different things.
///
/// It must be called on the main thread: `winit` says so on macOS and iOS, and
/// the event loop it builds is what enforces it.
pub fn run<D: Dial + Send + 'static>(dir: &Path, shard: Option<(D, Plan)>) -> ExitCode {
    // Reading the whole facet takes a moment and a few hundred megabytes. That
    // is the shape `uofiles` has today — see the backlog in docs/client.md — and
    // it is honest to do it up front rather than to stall on the first frame.
    let map = match Map::load_facet(dir, FACET) {
        Ok(map) => map,
        Err(error) => {
            eprintln!("loading facet {FACET}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let art = match Art::open(dir) {
        Ok(art) => art,
        Err(error) => {
            eprintln!("opening artLegacyMUL.uop: {error}");
            return ExitCode::FAILURE;
        }
    };
    // The two files a slope needs: the square textures, and the table that says
    // which of them a land graphic uses.
    let texmaps = match TexMaps::open(dir) {
        Ok(texmaps) => texmaps,
        Err(error) => {
            eprintln!("opening texmaps.mul: {error}");
            return ExitCode::FAILURE;
        }
    };
    let tiledata = match TileData::load(dir.join("tiledata.mul")) {
        Ok(tiledata) => tiledata,
        Err(error) => {
            eprintln!("opening tiledata.mul: {error}");
            return ExitCode::FAILURE;
        }
    };
    let hues = match Hues::load(dir.join("hues.mul")) {
        Ok(hues) => hues,
        Err(error) => {
            eprintln!("opening hues.mul: {error}");
            return ExitCode::FAILURE;
        }
    };
    // Built once: `hues.mul` does not change while the camera walks, unlike
    // the sprite atlases it is bound alongside.
    let hue_ramp = HueRamp::build(&hues);
    // `hues` itself is kept too, alongside the ramp built from it: the ramp is
    // an RGBA8 texture for the GPU passes, and `gump.rs` wants the same table
    // read as `Color16`s to pick a *solid* colour for hued text — see
    // `gump::text_color`. Building a second reader of `hues.mul` to avoid
    // holding both would be the duplication `docs/style.md` warns against, not
    // less of it.
    let fonts = match AsciiFonts::open(dir) {
        Ok(fonts) => fonts,
        Err(error) => {
            eprintln!("opening fonts.mul: {error}");
            return ExitCode::FAILURE;
        }
    };
    // Built once, the same as the hue ramp: `fonts.mul` is ten faces of 224
    // glyphs, all of them a few pixels, and there is no "visible set" of
    // characters the way there is a visible set of graphics — any speech line
    // can hold any of them.
    let font_atlas = match FontAtlas::build(&fonts) {
        Ok(atlas) => atlas,
        Err(error) => {
            eprintln!("packing fonts.mul: {error}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "{} loaded: {}x{} tiles",
        map.facet_name(),
        map.width(),
        map.height()
    );

    // With user events, because the shard thread wakes the loop with them.
    let event_loop = match EventLoop::<link::Update>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            eprintln!("no window system: {error}");
            return ExitCode::FAILURE;
        }
    };
    event_loop.set_control_flow(ControlFlow::Wait);

    let anim = match Anim::open(dir) {
        Ok(anim) => anim,
        Err(error) => {
            eprintln!("opening anim.idx and anim.mul: {error}");
            return ExitCode::FAILURE;
        }
    };

    // Where the character stands at boot: the camera's tile, at the height the
    // ground there actually is.
    let start = Point::new(
        START.x,
        START.y,
        map.land(START.x, START.y).map_or(START.z, |cell| cell.z),
    );

    // The connection, if this run was asked for one. Started before the window
    // exists: the login is several round trips, and there is a map to draw
    // while it happens.
    // Shared with the shard thread, which predicts the height of every step
    // from it: plain data, read by both and written by neither.
    let map = Arc::new(map);
    let link = shard.map(|(dial, plan)| {
        eprintln!("logging in as {}", plan.account.0);
        link::connect(dial, plan, VERSION, Arc::clone(&map), event_loop.create_proxy())
    });

    let mut app = App {
        map,
        art,
        texmaps,
        tiledata,
        hues,
        hue_ramp,
        font_atlas,
        anim,
        // The device's own limit replaces WebGL2's floor once there is a device
        // to ask; the floor is the smallest thing this has to run on.
        control: Control::new(Camera::new(START, 1024, 768), 2048, STARTUP_RIG),
        zoom_limit_reported: false,
        // 400 is the male human body. Its group and frame come from the crowd
        // on the first redraw, which is also what decides that a placeholder
        // nobody is walking stands.
        player: Mobile {
            at: start,
            body: 400,
            group: openshard_uofiles::anim::BodyKind::of(400).standing(),
            facing: Direction::SouthEast,
            frame: 0,
            hue: Hue::NONE,
            drawn: Gaze::on(start),
        },
        others: Vec::new(),
        items: Vec::new(),
        view: None,
        connection: String::from("offline"),
        shell: None,
        link,
        facet_checked: false,
        steer: steer::Steering::default(),
        aiming: false,
        crowd: {
            // The body's ease, which is not the camera's — see `STARTUP_EASE`.
            let mut crowd = Crowd::default();
            crowd.set_ease(STARTUP_EASE);
            crowd
        },
        next_tick: Instant::now(),
        last_advance: Instant::now(),
        last_frame: Instant::now(),
        window: None,
        selected_tile: None,
        covered: None,
        scope: Scope::new(SCOPE_SPAN),
        frames: frames::Frames::new(FRAMES_SPAN),
        focused: true,
        occluded: false,
        scripts: bench::scripts(),
        replay: None,
    };
    match event_loop.run_app(&mut app) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("event loop: {error}");
            ExitCode::FAILURE
        }
    }
}

/// How far page up and page down move the eye, in viewport pixels.
///
/// Half a tile's height per press, which is what the old "camera height" keys
/// moved when a step of `z` was five units: `5 * Z_STEP` is 20 pixels.
const PAGE_PIXELS: i32 = 20;

/// Why the client could not start.
///
/// A binary can afford to print and exit, but the reasons are still types: a
/// `String` error loses which of these happened the moment it is formatted, and
/// "no GPU" and "no client files" want different answers from whoever hits them.
#[derive(Debug)]
enum StartupError {
    /// No window could be created.
    Window(winit::error::OsError),
    /// The window has no surface wgpu can draw to.
    Surface(wgpu::CreateSurfaceError),
    /// No adapter, or no device from it.
    NoDevice(String),
    /// The surface offers only sRGB formats, which would change the art's
    /// colours on their way to the screen.
    OnlySrgb,
    /// The land art would not pack.
    Atlas(openshard_client_render::atlas::AtlasError),
}

impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Window(source) => write!(f, "creating a window: {source}"),
            Self::Surface(source) => write!(f, "creating a surface: {source}"),
            Self::NoDevice(detail) => write!(f, "no GPU to draw with: {detail}"),
            Self::OnlySrgb => write!(
                f,
                "this surface offers only sRGB formats, which would alter the art's colours",
            ),
            Self::Atlas(source) => write!(f, "packing land art: {source}"),
        }
    }
}

/// Every picture a frame can sample, packed together.
///
/// One value rather than four fields because they are grown together and used
/// together: a frame drawn from a land atlas of one camera and a static atlas
/// of another is a frame with things standing on ground that is not there.
///
/// # They grow; they are not rebuilt
///
/// An atlas used to be thrown away and packed again the moment the camera asked
/// for a graphic it did not hold, which is a full re-read of the art plus three
/// new pipelines — during a scroll, every few tiles, because a scroll is exactly
/// what keeps introducing graphics. Now [`Atlases::grow`] adds what is new to
/// what is already there and [`Atlases::upload`] sends the rows that changed.
///
/// The rebuild survives as the answer to *full* — see [`Atlases::grow`]'s note —
/// which is the one thing growing cannot do for itself.
struct Atlases {
    land: LandAtlas,
    texmaps: TexmapAtlas,
    statics: StaticAtlas,
    mobiles: AnimAtlas,
}

/// What a frame wants packed, gathered before anything is read from disk.
///
/// Three sets rather than three arguments, because they travel together
/// everywhere and two of them are keyed by numbers that look alike: a land
/// graphic and a static graphic are both a `Graphic` and are different index
/// spaces, which is a mistake a positional argument list would accept in
/// silence.
#[derive(Default)]
struct Wanted {
    /// Land graphics, which feed the land atlas and the texture atlas both.
    land: BTreeSet<Graphic>,
    /// Static graphics: what the map has standing on the ground, and what the
    /// server has dropped on top of it.
    statics: BTreeSet<Graphic>,
    /// Body, group and stored direction for everyone on screen.
    animations: BTreeSet<(u16, u8, u8)>,
}

impl Atlases {
    /// Pack a set from nothing.
    ///
    /// The startup path, and the recovery path: an atlas that has filled up is
    /// replaced by one built for what is on screen *now*, which is where the
    /// eviction lives. Growing has no other way to reclaim a graphic the camera
    /// walked away from ten minutes ago, and rebuilding used to do it by
    /// accident on every miss.
    fn build(
        art: &Art,
        texmaps: &TexMaps,
        tiledata: &TileData,
        anim: &mut Anim,
        wanted: &Wanted,
    ) -> Result<Self, AtlasError> {
        Ok(Self {
            land: LandAtlas::build(art, wanted.land.iter().copied())?,
            texmaps: TexmapAtlas::build(texmaps, tiledata, wanted.land.iter().copied())?,
            statics: StaticAtlas::build(art, wanted.statics.iter().copied())?,
            mobiles: AnimAtlas::build(anim, wanted.animations.iter().copied())?,
        })
    }

    /// Add whatever of `wanted` is not packed yet, reading only that.
    ///
    /// A graphic already offered costs a lookup in a `BTreeSet` and no file
    /// access at all — including one the client ships no art for, which is the
    /// case that used to make "is the atlas stale" answer yes for ever.
    ///
    /// [`AtlasError::Full`] leaves the atlases holding whatever fitted, and the
    /// caller is expected to throw them away and [`build`](Self::build) for the
    /// current frame. That is not a lost cause: it is the eviction, and it is
    /// the only thing that stops an atlas which only ever grows from filling up
    /// and staying full.
    fn grow(
        &mut self,
        art: &Art,
        texmaps: &TexMaps,
        tiledata: &TileData,
        anim: &mut Anim,
        wanted: &Wanted,
    ) -> Result<(), AtlasError> {
        // Both halves of a ground quad from the same set, in the same growth: a
        // land graphic in one atlas and not the other draws a slope textured
        // with the terrain next door.
        self.land.add(art, wanted.land.iter().copied())?;
        self.texmaps.add(texmaps, tiledata, wanted.land.iter().copied())?;
        self.statics.add(art, wanted.statics.iter().copied())?;
        self.mobiles.add(anim, wanted.animations.iter().copied())?;
        Ok(())
    }

    /// Send whatever grew to the textures already bound.
    ///
    /// Nothing at all on the ordinary frame, and a band of rows on the frame a
    /// camera crossed a tile — where this used to be three pipelines and 48MB.
    fn upload(
        &mut self,
        queue: &wgpu::Queue,
        ground: &GroundRenderer,
        statics: &SpriteRenderer,
        mobiles: &SpriteRenderer,
    ) {
        ground.upload_changes(queue, &mut self.land, &mut self.texmaps);
        if let Some(rows) = self.statics.take_dirty() {
            statics.upload_rows(queue, self.statics.pixels(), rows);
        }
        if let Some(rows) = self.mobiles.take_dirty() {
            mobiles.upload_rows(queue, self.mobiles.pixels(), rows);
        }
    }
}

/// What a set of tile rectangles wants packed, gathered from field references.
///
/// Free rather than a method on `App` because the frame that needs it most is
/// the one holding a `&mut` borrow of the window, where no `&self` method can be
/// called — and threading the pieces explicitly is cheaper than splitting the
/// struct to please the borrow checker.
fn wanted_in(
    map: &Map,
    bands: impl IntoIterator<Item = TileBounds>,
    items: &[GroundItem],
    drawn: &[Mobile],
) -> Wanted {
    let mut wanted = Wanted::default();
    for band in bands {
        ground::graphics_in(map, band, &mut wanted.land);
        statics::graphics_in(map, band, &mut wanted.statics);
    }
    wanted.statics.extend(items::needed_graphics(items));
    wanted.animations.extend(mobiles::needed_animations(drawn));
    wanted
}

/// Everything a window needs, built once the window exists.
struct Screen {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: GroundRenderer,
    /// The pass that draws what stands on the ground.
    statics: SpriteRenderer,
    /// What the world is drawn into, at 1:1 and at the camera's render size —
    /// which is the viewport only at zoom 1. [`Screen::blit`] puts it on the
    /// surface.
    world: wgpu::Texture,
    /// The pass that does that, and the only place a zoom exists.
    blit: Blit,
    /// The depth buffer the three world passes share, which is what decides
    /// whether a hillside covers the wall behind it. Recreated with
    /// [`Screen::world`]: it has to be exactly the size of the image it is
    /// tested against.
    depth: wgpu::Texture,
    /// The pass that draws the mobiles, which is the statics pass again with
    /// another atlas bound: a sprite is a sprite, and the two differ only in
    /// where the quad goes.
    mobile_pass: SpriteRenderer,
    /// Everything currently packed, grown as the camera walks into ground it
    /// has not seen. Beside the passes rather than inside them because the CPU
    /// side of an atlas is what builds a quad and the texture is what draws it.
    atlases: Atlases,
    /// The pass that draws overhead speech, bound to `App::font_atlas` once:
    /// unlike `statics` and `mobile_pass`, nothing ever rebuilds it — the
    /// glyph atlas it is bound to is the whole of `fonts.mul` and does not go
    /// stale the way a camera-scoped atlas does.
    text_pass: SpriteRenderer,
}

struct App {
    /// The facet, shared with the shard thread — see [`link::connect`].
    map: Arc<Map>,
    art: Art,
    texmaps: TexMaps,
    tiledata: TileData,
    /// Every hue the client ships, read as `hues.mul` stores it — a 32-step
    /// `Color16` ramp per hue. `hue_ramp` beside it is the same table packed
    /// for the GPU; this is what `gump.rs` reads to colour a `{ text }`
    /// element, which wants one CPU-side `egui::Color32` and not a texture row.
    hues: Hues,
    /// Every hue the client ships, packed once: unlike the sprite atlases it
    /// tints, nothing about it depends on where the camera is standing.
    hue_ramp: HueRamp,
    /// Every glyph `fonts.mul` ships, packed once for the reason `hue_ramp` is:
    /// nothing about it depends on the camera, and unlike a graphic there is no
    /// "not currently visible" character to leave unpacked.
    font_atlas: FontAtlas,
    /// The animations, open but not read: `anim.mul` is 195MB and frames come
    /// out of it a body at a time. `&mut` because reading one seeks the file.
    anim: Anim,
    /// The camera, who is allowed to move it, and what a drag has not yet spent.
    ///
    /// All of it arithmetic, and all of it in `client/render` where it can be
    /// reached by a test: this crate owns a window, a GPU and a `Map`, and none
    /// of the three has anything to say about a wheel notch.
    control: Control,
    /// Whether the device's refusal to hold a zoom's image has been said out
    /// loud. A silently truncated target draws a smaller world into a larger
    /// rect, which looks exactly like a bug in the projection — so it is
    /// reported, and once.
    zoom_limit_reported: bool,
    /// This client's own body.
    ///
    /// Connected, it is what the server says: `0x1B` puts it somewhere and
    /// every ack, `0x20` and `0x21` moves it. Offline it is a placeholder
    /// standing wherever the camera looks, which is enough to hold the
    /// animation reader, the frame atlas and the placement against a real
    /// install.
    player: Mobile,
    /// Everyone else on screen, as `0x77` and `0x78` last described them, each
    /// beside the serial the crowd's clocks are keyed by.
    ///
    /// Empty offline, and rebuilt whole from the [`WorldView`] on every update:
    /// the view is the record of what arrived and this is a projection of it,
    /// so there is nothing here to keep in step by hand.
    others: Vec<(Who, Mobile)>,
    /// Everything lying on the ground, as `0x1A` and `0x1D` last left it.
    ///
    /// A projection of the view like [`App::others`], and drawn through the
    /// same atlas and the same pass as the map's own statics: an item's picture
    /// is a static's picture. Two lists rather than one because the map's
    /// furniture never moves and these come and go with every packet.
    items: Vec<GroundItem>,
    /// The last thing the server said, whole.
    ///
    /// Kept only for the HUD's world window, which lists what has been decoded
    /// with the serials the three projections above drop. The renderer reads
    /// those.
    view: Option<Box<WorldView>>,
    /// What the connection is doing, for the status strip.
    connection: String,
    /// The dev HUD, once there is a window to put it on.
    shell: Option<shell::Shell>,
    /// The shard, if this run logged in to one.
    ///
    /// `None` is the offline viewer, and it is what the keyboard asks: a step
    /// is a `0x02` when there is somebody to send it to, and a camera move when
    /// there is not.
    link: Option<link::Link>,
    /// Whether the shard's facet has been compared with the one loaded. See
    /// [`App::entered`]: once, because it cannot change without a `0xBF 0x08`
    /// nothing here reads yet.
    facet_checked: bool,
    /// Where the player is asking to walk — the arrows, and the tile the mouse
    /// last sent the body to.
    ///
    /// A step is not sent from the input event: the operating system's
    /// auto-repeat is not a walking speed, a shard refuses a flood of steps as a
    /// speedhack, and a mouse held over the ground reports a move a pixel. One
    /// clock paces all of them. See `steer.rs`.
    steer: steer::Steering,
    /// Whether the right button is down, which is what makes dragging steer: the
    /// destination is restated at every tile the cursor crosses while it is.
    aiming: bool,
    /// What everyone on screen was doing a moment ago: which animation each is
    /// playing, and how far into it.
    ///
    /// The layer above [`WorldView`] that ages what it sees — see `crowd.rs`.
    /// Real time and not the world tick: there is no world here to tick, and a
    /// real client's body animation is a wall-clock timer too.
    crowd: Crowd,
    /// When the clock next advances a frame.
    next_tick: Instant,
    /// When it last did.
    ///
    /// The crowd is moved by *measured* time and not by the interval that was
    /// waited for: `WaitUntil` is a floor and the compositor overshoots it, so a
    /// clock fed the nominal step would run slow by however much it did — which
    /// a stepping animation hides and a glide does not.
    last_advance: Instant,
    /// When the last frame was *drawn*, for the frame panel's interval.
    ///
    /// Not [`App::last_advance`], which is the clock the world is advanced on
    /// and is moved by an arriving packet as well as by a frame. Measured
    /// against that, a frame that followed a packet by a millisecond would be
    /// reported as a thousand a second, and the one number the panel exists to
    /// show — the gap between two pictures — would be the one it does not.
    last_frame: Instant,
    window: Option<Screen>,
    /// The tile a left click last landed on, kept until the next click — see
    /// [`App::pick_tile`]. Separate from the live hover so a diagnosis does not
    /// slide off the tile the moment the mouse does.
    selected_tile: Option<(u16, u16)>,
    /// The tile rectangle whose land and statics have been offered to the
    /// atlases, or `None` when nothing has.
    ///
    /// The state the band walk in [`App::draw`] is built on, and the one thing
    /// here that is wrong in silence: an atlas rebuilt behind this field's back
    /// forgets graphics that this still claims were offered, and the tiles that
    /// needed them simply stop being drawn — along one edge, at one camera
    /// position. So it is set from exactly two places, both of which have just
    /// finished packing, and cleared before anything that forgets.
    covered: Option<TileBounds>,
    /// The last few seconds of the eye, for the scope in the HUD.
    ///
    /// Recorded every frame the camera is advanced, from the same three values
    /// the offline bench records, so the panel's numbers and the table's are one
    /// arithmetic. See [`Scope`].
    scope: Scope,
    /// The last few seconds of the event loop, for the frame panel.
    ///
    /// Recorded every frame that is actually drawn, locked or not: this is a
    /// number about the loop and not about the camera. See [`frames::Frames`]
    /// for why it is not the scope.
    frames: frames::Frames,
    /// Whether the window has the keyboard.
    ///
    /// Half of [`App::watched`], and true at construction: a window is mapped
    /// focused and winit sends no event to say the thing it has just done.
    focused: bool,
    /// Whether the compositor says the window is entirely covered.
    ///
    /// The other half of [`App::watched`]. Its own field rather than folded into
    /// the first, because the two arrive as two events in an order nothing
    /// promises, and one `bool` written by both would read the second one's
    /// answer to the first one's question.
    occluded: bool,
    /// The bench's scenarios, built once.
    ///
    /// Held rather than rebuilt per frame because the HUD lists their names, and
    /// a scenario is a `Vec` of knots: building nine of them to print nine
    /// strings would be a small allocation storm on every frame that draws.
    scripts: Vec<Script>,
    /// The one being walked in the window, while it is.
    replay: Option<replay::Replay>,
}

impl ApplicationHandler<link::Update> for App {
    /// The shard thread had something to say.
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, update: link::Update) {
        // The crowd's clock first, and before the packet is folded in. A step is
        // timestamped with `Crowd`'s own `now` — that is what the *next* step's
        // crossing is measured against (`crowd::glide_time`) — and this handler
        // used to fold packets in between two `advance` calls, so every step was
        // recorded at the previous frame's instant: up to 16ms in the past
        // mid-walk and up to a whole `FRAME_DELAY` for a body that had stopped.
        // The measurement is a difference of two of those, so the error lands on
        // the crossing *length*: the walk oracle in `dst.rs` caught a tile after
        // a turn taking 416ms instead of 400, which is a body a frame behind
        // itself and then yanked forward.
        let now = Instant::now();
        self.crowd
            .advance(now.saturating_duration_since(self.last_advance));
        self.last_advance = now;
        match update {
            link::Update::World { view, body } => self.entered(&view, body),
            // The window stays open: whatever is on screen is still the last
            // thing the server said, and closing it would take the reason with
            // it. The map viewer is what is left, which is a fair description
            // of a client that has lost its shard.
            link::Update::Lost(reason) => {
                eprintln!("disconnected: {reason}");
                self.link = None;
                return;
            }
        }
        // A step that arrives while nobody was moving finds the animation clock
        // armed for the *standing* rate, up to a whole `FRAME_DELAY` away — so
        // the first 80ms of the glide would be drawn frozen at its start, once
        // per tile. Pulling the tick forward is what makes a walk continuous
        // from its first frame; it is a `min` rather than an assignment because
        // a clock already running at the glide rate is the earlier of the two.
        let soon = now + GLIDE_INTERVAL;
        if self.crowd.anyone_gliding() && self.next_tick > soon {
            self.next_tick = soon;
        }
        if let Some(window) = self.window.as_ref() {
            window.window.request_redraw();
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        match self.create_window(event_loop) {
            Ok(window) => self.window = Some(window),
            Err(error) => {
                eprintln!("{error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // The UI sees everything first, and what it takes reaches neither the
        // camera nor the walk keys — otherwise a drag inside a panel pans the
        // world underneath it. egui never claims a close or a resize, so
        // returning here cannot swallow one.
        let consumed = match (self.shell.as_mut(), self.window.as_ref()) {
            (Some(shell), Some(screen)) => shell.on_window_event(&screen.window, &event),
            _ => false,
        };
        if consumed {
            // A key the UI took is a key this will never hear come up, and a
            // held direction that is never released walks for ever. Typing into
            // a panel should stop the character anyway, so letting go of
            // everything is both the fix and the behaviour.
            if matches!(event, WindowEvent::KeyboardInput { .. }) {
                self.steer.clear();
                self.aiming = false;
            }
            if let Some(window) = self.window.as_ref() {
                window.window.request_redraw();
            }
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(window) = self.window.as_mut() {
                    window.config.width = size.width.max(1);
                    window.config.height = size.height.max(1);
                    window.surface.configure(&window.device, &window.config);
                    self.control.resize(window.config.width, window.config.height);
                    // The world texture and the depth buffer follow the
                    // *camera's* size and not the window's, which are the same
                    // thing only at zoom 1. `draw` resizes them together.
                    window.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                // An arrow is *held*, not pressed: while it is down a step is
                // due every step's length, and that clock is ours rather than
                // the operating system's repeat rate. See `keys.rs`.
                if let Some(direction) = keys::Held::direction_of(code) {
                    let step = match event.state {
                        ElementState::Pressed => {
                            self.steer.press(direction, Instant::now(), self.player.facing)
                        }
                        ElementState::Released => {
                            self.steer.release(direction);
                            None
                        }
                    };
                    if let Some(facing) = step {
                        if self.walk(facing) {
                            if let Some(window) = self.window.as_ref() {
                                window.window.request_redraw();
                            }
                        }
                    }
                    return;
                }
                if event.state != ElementState::Pressed {
                    return;
                }
                if code == KeyCode::Escape {
                    event_loop.exit();
                    return;
                }
                let changed = match code {
                    KeyCode::Home => {
                        self.relock();
                        true
                    }
                    // Page up and down lift the eye rather than the body,
                    // which is a pan: the map has no vertical axis to walk
                    // along, only a projection that folds `z` into `y`.
                    KeyCode::PageUp => self.control.pan(0, PAGE_PIXELS),
                    KeyCode::PageDown => self.control.pan(0, -PAGE_PIXELS),
                    _ => false,
                };
                if changed {
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                }
            }
            // Shift is the whole of "run", and it arrives here rather than as a
            // key: `ModifiersChanged` is what winit reports a held modifier
            // with, and a `KeyboardInput` for the shift itself would miss the
            // case of it going down between two steps.
            WindowEvent::ModifiersChanged(modifiers) => {
                self.steer.set_running(modifiers.state().shift_key());
            }
            // A window that loses focus never hears the key come up, and a
            // character that keeps walking into a wall while its player is in
            // another window is not what the key meant. The destination goes
            // with it, for the same reason: nobody is watching it be walked to.
            //
            // It is also half of what paces the loop — see [`App::watched`] —
            // and regaining focus has to ask for a frame, because the redraw
            // that would have asked for the next one is the one that stopped
            // being drawn.
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if focused {
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                } else {
                    self.steer.clear();
                    self.aiming = false;
                }
            }
            // Entirely covered by another window: the compositor will not show
            // anything drawn, so the loop stops drawing at the display's rate
            // and falls back to the animation clock. Uncovered, it restarts the
            // same way focus does.
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;
                if !occluded {
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                // Relative to the *viewport* and not the window: the camera's
                // own centre is the viewport's, so a cursor measured from the
                // window would zoom about a point half a panel away.
                let origin = self.shell.as_ref().map_or((0, 0), |shell| {
                    (shell.viewport().x as i32, shell.viewport().y as i32)
                });
                let (x, y) = (position.x as i32 - origin.0, position.y as i32 - origin.1);
                let mut changed = self.control.cursor_moved(x, y);
                // Held, the button steers rather than picking one tile: the
                // destination follows the cursor, which is the walk-where-I-am-
                // pointing every UO client has and every strategy game's
                // move-order held down.
                if self.aiming {
                    changed |= self.walk_to_cursor();
                }
                if changed {
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == winit::event::MouseButton::Middle {
                    self.control.set_panning(state == ElementState::Pressed);
                }
                // A left click selects the tile under the cursor for the Tile
                // panel — reached here and not through egui, because `consumed`
                // above already sent every click the UI wanted to it.
                if button == winit::event::MouseButton::Left && state == ElementState::Pressed {
                    self.selected_tile = self.pick_tile().map(|tile| (tile.x, tile.y));
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                }
                // And a right click is a move order: walk there, and keep
                // walking there while the button is held. Left is spoken for by
                // the Tile panel above, and the middle button pans.
                if button == winit::event::MouseButton::Right {
                    self.aiming = state == ElementState::Pressed;
                    if self.aiming && self.walk_to_cursor() {
                        if let Some(window) = self.window.as_ref() {
                            window.window.request_redraw();
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // A notch is a line on a wheel and a fraction of one on a
                // touchpad, and only the sign is asked for here: the ladder is
                // what decides how far a notch goes.
                let notches = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(position) => position.y as f32,
                };
                if notches != 0.0 && self.zoom(notches > 0.0) {
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                }
            }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }

    /// Re-arm the animation clock and ask for a redraw when it has advanced.
    ///
    /// `winit`'s idiomatic timer: `ControlFlow::WaitUntil` sleeps the event
    /// loop rather than spinning it, and returning here every
    /// [`App::redraw_interval`] is what stands in for a real client's own
    /// `Mobile.ProcessAnimation` poll.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        // A held arrow — or a tile the mouse sent the body to — asks for a step
        // every step's length. Here and not in the input event: the operating
        // system repeats a held key at a rate that is not a walking speed, a
        // mouse held over the ground reports a move a pixel, and the fast half
        // of either is refused by the shard as a speedhack — which reads as the
        // walk stuttering. See `steer.rs`.
        //
        // Twice at most, because a turn is a step that covers no ground and
        // costs no time against the shard's pace budget: the step it precedes is
        // due the same instant, and holding that back to the next wake would put
        // a frame of standing still exactly where the player asked for movement.
        // Two and not a loop — the second ask is the step the turn was for, and
        // anything past it is a rate, which is what the clock is for.
        let mut moved = false;
        for _ in 0..2 {
            let Some(facing) = self.steer.due(now, self.player.at, self.player.facing) else {
                break;
            };
            moved |= self.walk(facing);
        }
        if moved {
            if let Some(window) = self.window.as_ref() {
                window.window.request_redraw();
            }
        }
        // The animation clock. Watched, this is a safety net rather than the
        // pacer — `draw` asks for the next frame itself and the display answers
        // — and it is kept for the paths where that ask does not happen: `draw`
        // returns early with no window, with a swapchain it had to rebuild, and
        // on a compositor that refused to hand over a texture. Without it, one
        // of those would stop the loop dead until the next input event. The
        // redraw requests coalesce, so a net that fires while the display is
        // already pacing costs a wake and no frame.
        if now >= self.next_tick {
            self.next_tick = now + self.redraw_interval();
            if let Some(window) = self.window.as_ref() {
                window.window.request_redraw();
            }
        }
        // Three reasons to come back, so three terms: the animation clock,
        // whatever the UI is animating, and the next step a held key is owed.
        // The deadline is the earliest — a loop that slept past the step would
        // walk at whatever rate it happened to wake at.
        // `checked_add`, because a still UI asks for eternity
        // (`Duration::MAX`, see `Shell::repaint_after`) and `now + MAX`
        // overflows the instant rather than meaning "never". An overflow is
        // exactly the case where the UI wants no frame of its own, so it falls
        // back to the animation clock.
        let deadline = match self.shell.as_ref().map(shell::Shell::repaint_after) {
            Some(after) => match now.checked_add(after) {
                Some(ui) => self.next_tick.min(ui),
                None => self.next_tick,
            },
            None => self.next_tick,
        };
        let deadline = match self.steer.deadline() {
            Some(step) => deadline.min(step),
            None => deadline,
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
    }
}

impl App {
    /// Take a step, answering whether anything on screen changed.
    ///
    /// Movement is clamped to the map rather than wrapped: walking off the north
    /// edge in UO is impossible, and a camera that wrapped would draw a seam
    /// between two sides of the world.
    fn walk(&mut self, facing: Facing) -> bool {
        // A hand on the body outranks a scenario, the same way a hand on the
        // camera outranks the lock: the two would otherwise both write the
        // player's position and the picture would be neither.
        self.replay = None;

        // Connected, the keyboard moves nothing: it asks. The body goes where
        // the `0x22` says it went, which is the whole point of the walk
        // handshake — a client that stepped locally and corrected later would
        // be predicting, and the prediction lives in `Walk` where it can be
        // rolled back.
        if let Some(link) = self.link.as_ref() {
            link.step(facing);
            return false;
        }

        // Turning is a step here, as it is not in a real client: there is no
        // server to say whether the step happened, so the body faces wherever
        // it was last sent. `client/net`'s `walk` is what will decide this once
        // the two are joined.
        let (dx, dy) = facing.direction.step();
        let x = (i32::from(self.player.at.x) + dx).clamp(0, self.map.width() as i32 - 1);
        let y = (i32::from(self.player.at.y) + dy).clamp(0, self.map.height() as i32 - 1);
        let (x, y) = (x as u16, y as u16);
        // On the *ground* there, not at some height of the camera's — a mobile
        // below the terrain is correctly hidden by it, which is what the depth
        // buffer is for and what looks exactly like a mobile that failed to
        // draw.
        let ground = self.map.land(x, y).map_or(self.player.at.z, |cell| cell.z);
        // The crowd's clock first, before the step is folded in, and for the
        // same reason `App::user_event` does it for a step off the wire: a step
        // is timestamped with `Crowd`'s own `now`, and this is called from
        // `about_to_wait` — where that clock is as old as the last frame. A step
        // recorded up to a frame in the past starts its crossing there, and
        // `crowd::crossing` then measures the time it has left from the same
        // stale instant. This is the offline half of the walk and it had the
        // defect the online half was already fixed for.
        let now = Instant::now();
        self.crowd
            .advance(now.saturating_duration_since(self.last_advance));
        self.last_advance = now;
        // Through the crowd like anyone else, so the placeholder walks when it
        // walks and stands when it stops. `None` is who it is: no shard has
        // named it, so it has no serial.
        self.player = self.crowd.see(
            None,
            Point::new(x, y, ground),
            Graphic(self.player.body),
            facing,
            self.player.hue,
        );
        // Offline the body is what the camera is locked to, exactly as the
        // server's is when there is a server. Unlocked, walking still walks and
        // the body may leave the screen — walking and looking are different
        // questions, and `Home` is the answer to the second.
        //
        // No time has passed: this is an input, not a frame. A rig that filters
        // integrates over the span it is given, and time passes in `App::draw`.
        self.follow_player(std::time::Duration::ZERO);
        true
    }

    /// Send the body to whatever tile the cursor is over, answering whether
    /// anything on screen changed.
    ///
    /// The mouse's whole share of walking: a click names a destination and a
    /// drag restates it, and `steer.rs` is what turns either into one step every
    /// step's length. A cursor that is off the map or outside the world's
    /// viewport names no tile and is left alone rather than treated as the
    /// nearest one — a move order nobody gave is worse than one that did
    /// nothing.
    fn walk_to_cursor(&mut self) -> bool {
        let Some(tile) = self.pick_tile() else {
            return false;
        };
        match self.steer.go_to(
            (tile.x, tile.y),
            self.player.at,
            Instant::now(),
            self.player.facing,
        ) {
            Some(facing) => {
                // The marker under the destination has moved even when the step
                // itself changes nothing on screen, so the redraw is not the
                // step's to decide.
                self.walk(facing);
                true
            }
            None => true,
        }
    }

    /// Say a line out loud, if there is a shard to hear it.
    ///
    /// Nothing is echoed locally. A shard sends every speaker their own words
    /// back — that is what makes `0xAE` exist — so a client that also drew them
    /// itself would show everything twice, and a line that never reached the
    /// server would look exactly like one that did.
    ///
    /// Offline the line goes nowhere and says so in the log rather than
    /// silently: the map viewer has nobody to talk to, and a chat box that
    /// swallowed what was typed would read as a broken connection.
    fn say(&mut self, line: String) {
        match self.link.as_ref() {
            Some(link) => link.say(line),
            None => tracing::info!(%line, "nothing said: no shard is connected"),
        }
    }

    /// Answer an open dialog and take it off the screen.
    ///
    /// The close is this end's, and it is why the view is touched here rather
    /// than waiting for a packet: the server sends one `0xB0` and waits for one
    /// `0xB1`, and nothing ever arrives to say the window is gone. See
    /// [`WorldView::gump_closed`](openshard_client_net::view::WorldView::gump_closed).
    fn answer_gump(&mut self, reply: link::GumpReply) {
        let gump_id = openshard_protocol::gump::GumpId(reply.gump_id.0);
        if let Some(link) = self.link.as_ref() {
            link.answer_gump(reply);
        }
        if let Some(view) = self.view.as_mut() {
            view.gump_closed(gump_id);
        }
    }

    /// Put the eye back on the body and lock it there.
    ///
    /// Where the body is *drawn* this frame, not the tile it is nominally on:
    /// a relock mid-step would otherwise land up to half a tile from the sprite
    /// and be corrected on the frame after.
    fn relock(&mut self) {
        self.player.drawn = self.drawn_player();
        self.control.relock(mobiles::gaze(&self.player));
    }

    /// Where our own body is drawn this instant, off the crowd's clock.
    ///
    /// Read rather than stored, and this is the one place that reads it: the
    /// position is a function of a clock and an ease's state, so one read once a
    /// frame is what keeps the sprite, the camera and the scope on the same
    /// number. A crowd that has never heard of us — before a shard names the
    /// body, and for the frame a placeholder is created on — answers with the
    /// tile, which is where a body nobody is easing stands.
    fn drawn_player(&self) -> Gaze {
        self.crowd
            .drawn_for(self.me())
            .unwrap_or_else(|| Gaze::on(self.player.at))
    }

    /// Whether there is anybody to show a frame to: the window has the keyboard
    /// and is not covered.
    ///
    /// What the loop's pacing hangs on, and the whole of what this client does
    /// about power. A window in the background still ages its animations — the
    /// crowd has to be where it would have been when the player comes back —
    /// but it does it on the animation clock rather than at the display's rate.
    fn watched(&self) -> bool {
        self.focused && !self.occluded
    }

    /// What is deciding when the next frame is drawn.
    ///
    /// Watched, it is the display and nothing else: [`App::draw`] asks for the
    /// next frame the moment it has queued one, and `PresentMode::Fifo` blocks
    /// the frame after that until the display has taken it. That is the loop
    /// every other real-time client runs, and it is what makes a still screen
    /// cost the same sixty frames a second as a moving one — which is the point,
    /// because "the frame rate drops when I stand still" was true here and read
    /// as a stall no matter how correct the reason was.
    ///
    /// Unwatched, there is nobody to show a frame to, and the timer below is
    /// what the loop falls back to. Two rates there, because there are two
    /// reasons for a frame and they are an order of magnitude apart: a body's
    /// animation steps once every [`FRAME_DELAY`] and nothing between two of
    /// those changes a pixel, while a *glide* moves a body a couple of pixels at
    /// a time and drawn on the animation clock would arrive in five visible
    /// jumps — the teleport it exists to remove, in instalments. Three reasons
    /// for the fast one and not one, because they are three independent things
    /// that move a pixel: a body mid-step, an eye still converging on one that
    /// has stopped, and a scenario waiting to deliver its next knot.
    ///
    /// The eye is the one that was missing. A rig that filters is still settling
    /// on frames where nothing else moved, and a loop that only woke for gliding
    /// bodies delivered the tail of every ease 80ms late and whole — the stutter
    /// the filter exists to remove, arriving just after it.
    fn pacing(&self) -> frames::Pacing {
        if self.watched() {
            return frames::Pacing::Display;
        }
        frames::Pacing::Timer(self.redraw_interval())
    }

    /// The fallback timer's interval. See [`App::pacing`] for when it is the one
    /// that decides.
    fn redraw_interval(&self) -> std::time::Duration {
        let moving = self.crowd.anyone_gliding() || self.control.settling() || self.replay.is_some();
        if moving { GLIDE_INTERVAL } else { FRAME_DELAY }
    }

    /// Start walking one of the bench's scenarios in the window.
    ///
    /// Offline only: with a shard connected the body goes where the `0x22` says
    /// it went, and a second writer would be two clients fighting over one
    /// character. The panel does not offer the buttons in that state and this
    /// refuses anyway, because a guard that only lives in a widget is a guard
    /// until somebody adds a keybinding.
    fn start_replay(&mut self, name: &str) {
        if self.link.is_some() {
            return;
        }
        let Some(script) = self.scripts.iter().find(|script| script.name == name).cloned() else {
            return;
        };
        // The height the script's own `z = 0` means here. Read once, from the
        // tile it starts on — see `Replay`'s docs on why not per tile.
        let ground = script.knots().first().map_or(self.player.at.z, |knot| {
            Self::in_bounds(i32::from(knot.from.x), i32::from(knot.from.y), &self.map)
                .and_then(|(x, y)| self.map.land(x, y))
                .map_or(self.player.at.z, |cell| cell.z)
        });
        let replay = replay::Replay::new(script, ground);
        if let Some(start) = replay.start() {
            // Put down rather than walked, and the camera cut to it: a body
            // that strolled to the start of a scenario would be measured on the
            // way there, and an eye that eased across a facet is a second
            // motion on top of the one being looked at.
            let (body, hue) = (Graphic(self.player.body), self.player.hue);
            self.player = self
                .crowd
                .snap(self.me(), start, body, Facing::walking(self.player.facing), hue);
            self.control.relock(mobiles::gaze(&self.player));
        }
        // The frames either side of a start are two different runs, and a metric
        // over both is a number about nothing.
        self.scope.clear();
        self.replay = Some(replay);
    }

    /// One frame of whatever scenario is being walked.
    ///
    /// Every knot the span covered, in order, each handed to the crowd as the
    /// packet it stands for: a crossing is glided and a jump is put down.
    fn advance_replay(&mut self, elapsed: std::time::Duration) {
        let Some(replay) = self.replay.as_mut() else {
            return;
        };
        let moves = replay.advance(elapsed);
        let finished = replay.finished();
        for step in moves {
            let (body, hue) = (Graphic(self.player.body), self.player.hue);
            self.player = match step.glided {
                true => self.crowd.see(self.me(), step.to, body, step.facing, hue),
                false => self.crowd.snap(self.me(), step.to, body, step.facing, hue),
            };
        }
        if finished {
            self.replay = None;
        }
    }

    /// Who the crowd knows our own body as.
    ///
    /// Our serial once a shard has named us, and `None` for the offline
    /// placeholder — see [`Who`].
    fn me(&self) -> Who {
        self.view.as_ref().map(|view| view.player.serial)
    }

    /// Point the eye at our own body, wherever the glide has it this instant.
    ///
    /// Called every frame and not only when a step arrives: the glide moves the
    /// body a few pixels per frame, and an eye that moved a tile at a time would
    /// jerk the whole world under it. Reads the crowd's clock straight, so it is
    /// also what keeps the eye and the sprite from disagreeing by a frame.
    ///
    /// `elapsed` is the same span the crowd's clock was just advanced by, and
    /// deliberately the same value: a rig that filters is integrating over it,
    /// and a camera integrating a different amount of time than the body moved
    /// through lags by whatever the difference was — which varies frame to
    /// frame, and varying lag is what an eye reads as a stutter.
    fn follow_player(&mut self, elapsed: std::time::Duration) {
        self.player.drawn = self.drawn_player();
        let gaze = mobiles::gaze(&self.player);
        self.control.follow_body(gaze, elapsed);
        // What the eye was asked for, what the screen was given, and what the
        // filter had before the quantiser — the three the bench records, from
        // the one place the camera is advanced.
        //
        // Only while the eye is the body's: unlocked, the camera is wherever a
        // hand left it and a lag against a body it is not following is not a
        // number about the rig.
        if let Some(state) = self.control.eye_exact() {
            if self.control.follow() == Follow::Body {
                self.scope
                    .record(elapsed, gaze, self.control.camera().eye(), state);
            }
        }
    }

    /// A viewport that grew may have taken the world texture past what the
    /// device allows, which no zoom step asked for.
    fn fit_zoom_to_device(&mut self) {
        if let Some(refusal) = self.control.fit_to_device() {
            self.report_limit(format_args!(
                "a {}x{} world texture at {} is more than this GPU's {}: zooming in to {}",
                refusal.width, refusal.height, refusal.wanted, refusal.max, refusal.settled,
            ));
        }
    }

    /// One notch of the wheel, answering whether anything changed.
    ///
    /// At either end of the ladder nothing does, and zooming out can be refused
    /// by the device — which is said out loud rather than truncated.
    fn zoom(&mut self, inwards: bool) -> bool {
        match self.control.zoom(inwards) {
            Ok(changed) => changed,
            Err(refusal) => {
                self.report_limit(format_args!(
                    "{} would want a {}x{} world texture and this GPU allows {}: staying at {}",
                    refusal.wanted, refusal.width, refusal.height, refusal.max, refusal.settled,
                ));
                false
            }
        }
    }

    /// Say what the device refused, once.
    ///
    /// Once, because the wheel is held down and a line per notch is a wall of
    /// the same sentence — and because the second one tells nobody anything the
    /// first did not.
    fn report_limit(&mut self, message: std::fmt::Arguments<'_>) {
        if !self.zoom_limit_reported {
            self.zoom_limit_reported = true;
            eprintln!("{message}");
        }
    }

    /// Redraw from what the server has shown us.
    ///
    /// A projection of the whole [`WorldView`], rebuilt each time rather than
    /// patched: the view is the record of what arrived, and anything kept in
    /// step with it by hand would be a second record that could disagree.
    fn entered(&mut self, view: &WorldView, body: link::Body) {
        // The facet is chosen at startup and `0x1B` names only its size, so a
        // shard serving a different one draws this client the wrong ground with
        // no complaint from either end. Said once, because it is a
        // misconfiguration and not an event.
        if !self.facet_checked {
            self.facet_checked = true;
            if u32::from(view.map.width) != self.map.width()
                || u32::from(view.map.height) != self.map.height()
            {
                eprintln!(
                    "the shard's facet is {}x{} and {} is {}x{}: the ground drawn is not the ground you are standing on",
                    view.map.width,
                    view.map.height,
                    self.map.facet_name(),
                    self.map.width(),
                    self.map.height(),
                );
            }
        }

        // Our own body is drawn where this end *predicted* it, not where the
        // last ack put it: the step leaves the moment the player asks for it and
        // the `0x22` confirming it arrives a round trip later, so a body drawn
        // from the view stands still for the latency and then crosses its tile
        // in a hurry. See `link::Body`.
        //
        // A correction is the one thing that is not walked into: the tile it
        // puts the body back on was never crossed.
        let me = Some(view.player.serial);
        // Ours is the one body whose pace is not guessed at: we send its steps.
        // Said every update rather than once, because the serial is the shard's
        // to name and nothing here is told when it does.
        self.crowd.commanding(me);
        // A rollback is also the one thing that makes `steer.rs`'s idea of which
        // way this body was last sent a lie — it is a step ahead of the shard on
        // purpose, and a refusal is the shard saying that step never happened.
        // Left uncorrected, the step after a `0x21` is decided against a facing
        // nobody has: it is timed as a turn when it is a step, or as a step when
        // it is a turn, and either is a beat of the walk in the wrong place.
        if body.corrected {
            self.steer.corrected(body.predicted.facing.direction);
        }
        self.player = match body.corrected {
            true => self.crowd.snap(
                me,
                body.predicted.position,
                view.player.body,
                body.predicted.facing,
                view.player.hue,
            ),
            false => self.crowd.see(
                me,
                body.predicted.position,
                view.player.body,
                body.predicted.facing,
                view.player.hue,
            ),
        };
        // Sorted by serial: a `HashMap`'s order is not one, and an atlas built
        // in a different order every frame is a rebuild every frame.
        let mut others: Vec<_> = view.mobiles.iter().collect();
        others.sort_unstable_by_key(|(serial, _)| serial.raw());
        self.others = others
            .into_iter()
            .map(|(serial, mobile)| {
                let who = Some(*serial);
                (
                    who,
                    self.crowd
                        .see(who, mobile.position, mobile.body, mobile.facing, mobile.hue),
                )
            })
            .collect();
        // Whoever the view no longer holds walked out of range, and their clock
        // goes with them. Our own body is kept by its serial like anyone else's;
        // the placeholder's `None` is gone the moment a shard names us, which is
        // right — it was never a mobile.
        self.crowd.retain(|who| {
            who.is_some_and(|serial| serial == view.player.serial || view.mobiles.contains_key(&serial))
        });
        // Sorted by serial for the same reason, and for one more: two items on
        // one tile at one height are drawn in the order they arrive here, so an
        // order that changed every frame would flicker.
        let mut items: Vec<_> = view.items.iter().collect();
        items.sort_unstable_by_key(|(serial, _)| serial.raw());
        self.items = items
            .into_iter()
            .map(|(_, item)| GroundItem {
                at: item.position,
                graphic: item.graphic,
                hue: item.hue,
            })
            .collect();
        self.connection = format!("in world as 0x{:08X}", view.player.serial.raw());
        // The newest line in the journal, heard once and hung over its
        // speaker's head for a while — compared against the old view, still
        // in `self.view` at this point, so a redraw that changed nothing else
        // does not restart the hold on the same sentence. A system line
        // (`serial: None`) has no mobile to hang over and is left for the
        // HUD's world window instead, which is not built yet.
        if let Some(latest) = view.journal.back() {
            let already_heard = self
                .view
                .as_ref()
                .is_some_and(|previous| previous.journal.back() == Some(latest));
            if !already_heard {
                if let Some(serial) = latest.serial {
                    self.crowd
                        .hear(Some(serial), latest.text.clone(), latest.font, latest.hue);
                }
            }
        }
        // Whole, for the HUD's world window: the three projections above are
        // what the renderer wants, and none of them keeps a serial.
        self.view = Some(Box::new(view.clone()));
        // The camera follows the body, which is what `0x20` is for — unless it
        // has been unlocked, in which case the eye is the mouse's and the body
        // is free to walk off the screen. `Home` puts it back. After the view is
        // stored, because that is what says who we are, and the glide is keyed
        // by it.
        //
        // Zero, for the reason `App::walk_offline` says: a packet is not a
        // frame. The crowd's clock was brought up to date before this fold, so
        // there is no elapsed time left to hand a rig anyway.
        self.follow_player(std::time::Duration::ZERO);
    }

    /// Common code for the two lookups in [`App::pick_tile`]: `unproject` hands
    /// back a signed pair that may be off the map in any direction, and a
    /// negative one is not expressible as the `u16` [`Map::land`] wants.
    fn in_bounds(x: i32, y: i32, map: &Map) -> Option<(u16, u16)> {
        if x < 0 || y < 0 || x as u32 >= map.width() || y as u32 >= map.height() {
            return None;
        }
        Some((x as u16, y as u16))
    }

    /// Everything the Tile panel shows about one tile, read straight from the
    /// map. Shared by the live hover and a click's frozen selection, so the two
    /// can never disagree about what a tile contains.
    fn tile_info(&self, x: u16, y: u16) -> shell::PickedTile {
        let land = self.map.land(x, y);
        let statics = self
            .map
            .statics_at(x, y)
            .map(|item| (item.tile, item.z, item.hue))
            .collect();
        shell::PickedTile {
            x,
            y,
            land: land.map(|cell| cell.tile),
            land_z: land.map_or(0, |cell| cell.z),
            statics,
        }
    }

    /// What tile the cursor is over, read straight from the map.
    ///
    /// `unproject` needs the height the pixel is meant to be read at, and the
    /// ground is not flat — so this picks once at the player's height to find
    /// a candidate tile, then re-picks at *that* tile's own height, which is
    /// exact wherever the two tiles agree and wrong only at a slope's edge,
    /// same as the client's own click-to-walk.
    fn pick_tile(&self) -> Option<shell::PickedTile> {
        let (cursor_x, cursor_y) = self.control.cursor();
        let world_px = self.control.camera().pick(cursor_x, cursor_y);
        let mut z = self.player.at.z;
        let (mut x, mut y) = camera::unproject(world_px, z);
        if let Some((ux, uy)) = Self::in_bounds(x, y, &self.map) {
            if let Some(cell) = self.map.land(ux, uy) {
                z = cell.z;
                (x, y) = camera::unproject(world_px, z);
            }
        }
        let (x, y) = Self::in_bounds(x, y, &self.map)?;
        Some(self.tile_info(x, y))
    }

    /// What the panels are allowed to know, gathered each frame.
    fn hud(&self) -> shell::Hud {
        let (mobiles, items) = match self.view.as_ref() {
            Some(view) => {
                let mut mobiles: Vec<_> = view
                    .mobiles
                    .iter()
                    .map(|(serial, mobile)| (serial.raw(), mobile.body.0, mobile.position))
                    .collect();
                // Sorted, so a `HashMap`'s iteration order does not reshuffle
                // the list under the reader's eyes every frame.
                mobiles.sort_unstable_by_key(|(serial, _, _)| *serial);
                let mut items: Vec<_> = view
                    .items
                    .iter()
                    .map(|(serial, item)| (serial.raw(), item.graphic.0, item.position))
                    .collect();
                items.sort_unstable_by_key(|(serial, _, _)| *serial);
                (mobiles, items)
            }
            None => (Vec::new(), Vec::new()),
        };
        shell::Hud {
            ease: self.crowd.ease(),
            connection: self.connection.clone(),
            serial: self.view.as_ref().map(|view| view.player.serial.raw()),
            position: self.player.at,
            camera: *self.control.camera(),
            locked: self.control.follow() == Follow::Body,
            rig: self.control.rig(),
            readings: bench::readings(self.scope.samples()),
            // Two frames is one difference and no derivative of it. Absent
            // rather than a zero, which would read as "the eye was perfectly
            // smooth" on the frame the window opened.
            metrics: (self.scope.samples().len() > 2).then(|| Metrics::of(self.scope.samples())),
            scope_span: self.scope.span(),
            frames: self.frames.frames().to_vec(),
            frames_span: self.frames.span(),
            worst_fps: self.frames.worst_fps(),
            // What is currently *asking* for frames, which is the other half of
            // any answer about the frame rate: a picture drawn every 80ms is not
            // a slow frame if the loop is on the animation clock, it is a frame
            // nobody asked for sooner.
            pacing: self.pacing(),
            scripts: self.scripts.iter().map(|script| script.name).collect(),
            replay: self.replay.as_ref().map(|replay| {
                let length = replay.length().as_secs_f32().max(0.001);
                (replay.name(), replay.at().as_secs_f32() / length)
            }),
            offline: self.link.is_none(),
            mobiles,
            items,
            hover: self.pick_tile(),
            selected: self.selected_tile.map(|(x, y)| self.tile_info(x, y)),
            goal: self.steer.goal().map(|(x, y)| self.tile_info(x, y)),
            gumps: self
                .view
                .as_ref()
                .map(|view| view.gumps.clone())
                .unwrap_or_default(),
            said: self
                .view
                .as_ref()
                .map(|view| {
                    view.journal
                        .iter()
                        .rev()
                        .take(SPEECH_LINES)
                        .rev()
                        .map(|line| match line.name.is_empty() {
                            true => line.text.clone(),
                            false => format!("{}: {}", line.name, line.text),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// Everyone to draw, each beside the serial their clock is keyed by.
    ///
    /// Our own body first, and `None` for it while no shard has named us.
    fn drawn_mobiles(&self) -> Vec<(Who, Mobile)> {
        let mut mobiles = Vec::with_capacity(self.others.len() + 1);
        mobiles.push((self.me(), self.player));
        mobiles.extend_from_slice(&self.others);
        mobiles
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> Result<Screen, StartupError> {
        // Physical pixels, not logical: a `LogicalSize` here would ask for the
        // same *point* size on every monitor and come out small on a dense
        // one, exactly backwards from what "respect the density" means. Sized
        // off the monitor rather than the `Camera` default (1024x768, meant as
        // a viewport floor, not a window request) so the window opens large on
        // whatever screen it is on.
        let attributes = Window::default_attributes().with_title("OpenShard");
        let attributes = match event_loop.primary_monitor().map(|monitor| monitor.size()) {
            Some(size) if size.width > 0 && size.height > 0 => {
                attributes.with_inner_size(winit::dpi::PhysicalSize::new(
                    (size.width as f32 * 0.9) as u32,
                    (size.height as f32 * 0.9) as u32,
                ))
            }
            _ => attributes.with_inner_size(winit::dpi::LogicalSize::new(
                self.control.camera().width,
                self.control.camera().height,
            )),
        };
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(StartupError::Window)?,
        );

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(Arc::clone(&window))
            .map_err(StartupError::Surface)?;

        // Blocking here is fine on the desktop and would not be in a browser,
        // where this whole function becomes an `async` one driven by the event
        // loop. Nothing below cares which way it was awaited.
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|error| StartupError::NoDevice(error.to_string()))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(|error| StartupError::NoDevice(error.to_string()))?;

        let capabilities = surface.get_capabilities(&adapter);
        // A non-sRGB format, deliberately: `client/render` writes the art's own
        // bytes and an sRGB surface would gamma-correct them into something
        // else. See that crate's docs.
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .ok_or(StartupError::OnlySrgb)?;

        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            // `Auto` is the only value guaranteed for every format, and it means
            // "whatever the format says" — which for a non-sRGB format is the
            // pass-through this renderer needs.
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            // Named, and not `present_modes[0]`. This is the loop's pacer: a
            // frame is drawn, `request_redraw` asks for the next one at once,
            // and what makes that a rate rather than a spin is `get_current_texture`
            // blocking here until the display has taken the last one. Whatever
            // the adapter happened to offer first is `Mailbox` on some drivers
            // and `Immediate` on others — neither of which blocks, so the same
            // code is a 60Hz walk on one machine and a busy loop at a thousand
            // frames a second on the next. `Fifo` is the one mode `wgpu`
            // guarantees on every backend, which is why it can be asked for
            // outright rather than searched for.
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        // How far the zoom may be walked out. Asked once, because it is a
        // property of the device and not of the frame.
        self.control
            .set_max_texture(device.limits().max_texture_dimension_2d);
        self.control.resize(config.width, config.height);

        let wanted = self.wanted_now();
        let atlases = Atlases::build(&self.art, &self.texmaps, &self.tiledata, &mut self.anim, &wanted)
            .map_err(StartupError::Atlas)?;
        // What the atlases were built for, which is what the band walk in
        // `draw` subtracts from on the next frame.
        self.covered = Some(self.control.camera().visible_tiles());
        // The world passes draw into the world texture, so they take *its*
        // format and not the surface's — the two differ on an HDR display,
        // where the first non-sRGB surface format is `Rgba16Float`.
        let renderer = GroundRenderer::new(
            &device,
            &queue,
            blit::WORLD_FORMAT,
            &atlases.land,
            &atlases.texmaps,
        );
        let statics = SpriteRenderer::new(
            &device,
            &queue,
            blit::WORLD_FORMAT,
            atlases.statics.pixels(),
            &self.hue_ramp,
        );
        let mobile_pass = SpriteRenderer::new(
            &device,
            &queue,
            blit::WORLD_FORMAT,
            atlases.mobiles.pixels(),
            &self.hue_ramp,
        );
        // Built once, unlike `statics` and `mobile_pass`: `font_atlas` is never
        // rebuilt, so neither is what draws it.
        let text_pass = SpriteRenderer::new(
            &device,
            &queue,
            blit::WORLD_FORMAT,
            self.font_atlas.pixels(),
            &self.hue_ramp,
        );
        // The world is drawn at 1:1 into a texture of the camera's render size,
        // which is the viewport only at zoom 1 — see `client/render`'s `blit`.
        let world = blit::world_texture(
            &device,
            self.control.camera().render_width(),
            self.control.camera().render_height(),
        );
        let depth = renderer::depth_texture(
            &device,
            self.control.camera().render_width(),
            self.control.camera().render_height(),
        );
        let blit = Blit::new(&device, format);
        // The HUD, with the surface's own format: egui picks its fragment entry
        // point from whether that format is sRGB, and this one deliberately is
        // not.
        self.shell = Some(shell::Shell::new(&device, format, &window));

        Ok(Screen {
            window,
            surface,
            device,
            queue,
            config,
            renderer,
            statics,
            world,
            blit,
            depth,
            mobile_pass,
            atlases,
            text_pass,
        })
    }

    /// Everything on screen right now, whatever the atlases already hold.
    ///
    /// The whole-viewport walk, which is what a rebuild needs and what an
    /// ordinary frame must not do: [`App::wanted_since`] is the frame's version
    /// of the same question and walks only the band the camera crossed.
    fn wanted_now(&self) -> Wanted {
        self.wanted_in([self.control.camera().visible_tiles()])
    }

    /// What the camera has walked onto since `covered` was the visible
    /// rectangle, plus everything that is not a question about the map at all.
    ///
    /// The saving this whole arrangement is for. A frame used to walk the
    /// visible rectangle twice — once for the land graphics and once for the
    /// statics — purely to ask whether the atlases were still good for it, which
    /// is ~9,800 cells at 1080p against a camera that had moved one tile. The
    /// bands [`TileBounds::difference`] hands back are that tile's worth of
    /// cells.
    ///
    /// The invariant it rests on: every cell inside `covered` has already been
    /// offered to the atlases, and an atlas never forgets what it was offered —
    /// not even a graphic the client ships no art for. So a graphic can only be
    /// new outside `covered`, and anything that *does* make an atlas forget has
    /// to set `covered` back to `None` in the same breath.
    fn wanted_since(&self, covered: Option<TileBounds>) -> Wanted {
        let bounds = self.control.camera().visible_tiles();
        let bands = match covered {
            Some(covered) => bounds.difference(covered),
            None => [Some(bounds), None, None, None],
        };
        self.wanted_in(bands.into_iter().flatten())
    }

    /// The graphics on some set of tiles, and everything that is on screen
    /// regardless of where the camera is.
    ///
    /// Items the server has dropped and the bodies walking about are short lists
    /// held in memory, so they are asked in full however small the bands are —
    /// an item that arrives while the camera stands still is on no band at all.
    /// They go into the *static* set deliberately: one atlas serves the map's
    /// statics and the server's items, because a floor tile packed twice is a
    /// floor tile twice.
    fn wanted_in(&self, bands: impl IntoIterator<Item = TileBounds>) -> Wanted {
        let drawn: Vec<Mobile> = self
            .drawn_mobiles()
            .into_iter()
            .map(|(_, mobile)| mobile)
            .collect();
        wanted_in(&self.map, bands, &self.items, &drawn)
    }

    fn draw(&mut self) {
        let started = Instant::now();
        // The animation clock moves here, at the top of the frame that is about
        // to show its answer — not when the timer that asked for this frame
        // fired.
        //
        // A glide is a position read off a clock, so the moment that clock is
        // read has to be the moment the picture is built or the walk judders:
        // the timer fires, the loop then lays out the UI, grows an atlas and
        // waits on the swapchain, and however long that took is error in the
        // body's position — error that varies frame to frame, which is exactly
        // what an eye reads as a stutter. It also puts the sampling back in step
        // with the display: `WaitUntil` is a floor, the timer's 16ms beats
        // against a 60Hz refresh, and a frame drawn from the previous tick's
        // clock lands on the wrong side of that beat every second or so.
        //
        // Whatever really passed — see `App::last_advance`. A stall longer than
        // a frame, the window minimised or the machine asleep, moves the clock
        // the whole way rather than queuing a burst of catch-up frames for time
        // nobody watched: a body that was walking through it has long since
        // arrived.
        let elapsed = started.saturating_duration_since(self.last_advance);
        self.crowd.advance(elapsed);
        self.last_advance = started;

        // The UI first, because what it leaves free is the world's viewport and
        // therefore the size of everything below. A frame that laid its panels
        // out after drawing the world would size the world from the previous
        // frame's panels — which is right until the first resize.
        // Gathered before the shell is borrowed: the HUD is a projection of the
        // whole app and the shell is part of it.
        //
        // Timed, and separately from the world below: the two halves of a frame
        // are built by two things that grow for different reasons, and a single
        // build time cannot say which of them ate the frame. See [`frames`].
        let ui_started = Instant::now();
        let hud = self.hud();
        let painting = self.window.as_ref().map(|screen| Arc::clone(&screen.window));
        let ui = match (self.shell.as_mut(), painting.as_ref()) {
            (Some(shell), Some(window)) => {
                let (request, output) = shell.run(window, &hud, &self.hues);
                let viewport = shell.viewport();
                Some((request, output, viewport))
            }
            _ => None,
        };
        let mut ui_cost = ui_started.elapsed();
        if let Some((request, _, viewport)) = &ui {
            if request.relock {
                self.relock();
            } else if request.unlock {
                self.control.unlock();
            }
            self.control.resize(viewport.width, viewport.height);
            if let Some(rig) = request.rig {
                // The eye does not move — that is what `set_rig` promises — but
                // the frames before the swap were flown by another camera, and
                // measuring them together would average two rigs.
                self.control.set_rig(rig);
                self.scope.clear();
            }
            // The window the metrics are taken over, and not a clear: the
            // frames already held were flown by the same rig.
            // The body's ease is not the rig and does not clear the scope: the
            // frames either side of it were flown by the same camera, and what
            // the scope measures is the eye against the body it was given.
            if let Some(ease) = request.ease {
                self.crowd.set_ease(ease);
            }
            if let Some(span) = request.scope_span {
                self.scope.set_span(span);
            }
            match request.script {
                Some(shell::ScriptRequest::Run(name)) => self.start_replay(name),
                Some(shell::ScriptRequest::Stop) => self.replay = None,
                None => {}
            }
            if let Some(line) = request.say.clone() {
                self.say(line);
            }
            if let Some(reply) = request.gump.clone() {
                self.answer_gump(reply);
            }
        }
        // Whatever scenario is being walked delivers its knots for the span that
        // just passed, before the eye is asked where the body is: a step that
        // arrived this frame is one the camera has to answer this frame.
        self.advance_replay(elapsed);
        // A viewport that grew may have taken the world texture past what the
        // device allows, which no zoom step asked for.
        self.fit_zoom_to_device();
        // And the eye goes where the body is *this frame*, before anything asks
        // the camera what is visible: a step arrives once and is then walked
        // across for the next 400ms, so every frame in between has a different
        // answer.
        self.follow_player(elapsed);

        // Read before the window is borrowed below, for the same reason the two
        // lines above are: the borrow is of `self`, and the pacing at the foot
        // of this frame is a fact about the whole app rather than about it.
        let watched = self.watched();

        // What the camera has walked onto since the atlases were last grown.
        // Gathered before the window is borrowed, and not inside the borrow: it
        // reads the whole of `self`, and the window is part of it.
        let want = self.control.camera().visible_tiles();
        let wanted = self.wanted_since(self.covered);
        let mut drawn = self.drawn_mobiles();

        let Some(window) = self.window.as_mut() else {
            return;
        };
        // Grow rather than rebuild. What is new is added to the textures
        // already bound, a band of rows at a time, and a frame where the camera
        // stood still reads four `BTreeSet`s and touches no file and no GPU.
        let grown = window
            .atlases
            .grow(&self.art, &self.texmaps, &self.tiledata, &mut self.anim, &wanted);
        // Whatever was packed is uploaded, including on the way out of a failure:
        // a growth that stopped part way still wrote pixels, and pixels the
        // device has not been told about are sampled as whatever was there
        // before. Cheap to do unconditionally — the band is empty when nothing
        // grew — and it is one fewer path where an atlas and its texture can
        // disagree.
        window.atlases.upload(
            &window.queue,
            &window.renderer,
            &window.statics,
            &window.mobile_pass,
        );
        match grown {
            Ok(()) => self.covered = Some(want),
            // Full, and this is the eviction: pack an atlas for what is on
            // screen now and throw away everything the camera has walked past.
            // Costly and rare — where the old arrangement paid it every few
            // tiles — and it is the *only* thing that reclaims space, so an
            // atlas that only ever grew would eventually stay full for ever.
            //
            // The passes are rebuilt with it, because the texture a bind group
            // points at is the one the old atlas was uploaded to.
            Err(AtlasError::Full { .. }) => {
                // `covered` is cleared first: a rebuild forgets, so the next
                // frame may not assume anything about what the atlases hold.
                // Set again below only if the rebuild succeeds.
                self.covered = None;
                match Atlases::build(
                    &self.art,
                    &self.texmaps,
                    &self.tiledata,
                    &mut self.anim,
                    &wanted_in(
                        &self.map,
                        [self.control.camera().visible_tiles()],
                        &self.items,
                        &drawn.iter().map(|(_, mobile)| *mobile).collect::<Vec<_>>(),
                    ),
                ) {
                    Ok(atlases) => {
                        window.renderer = GroundRenderer::new(
                            &window.device,
                            &window.queue,
                            blit::WORLD_FORMAT,
                            &atlases.land,
                            &atlases.texmaps,
                        );
                        window.statics = SpriteRenderer::new(
                            &window.device,
                            &window.queue,
                            blit::WORLD_FORMAT,
                            atlases.statics.pixels(),
                            &self.hue_ramp,
                        );
                        window.mobile_pass = SpriteRenderer::new(
                            &window.device,
                            &window.queue,
                            blit::WORLD_FORMAT,
                            atlases.mobiles.pixels(),
                            &self.hue_ramp,
                        );
                        window.atlases = atlases;
                        self.covered = Some(want);
                    }
                    // One screen does not fit one atlas, which is a different
                    // statement from "the atlas filled up": no eviction can help
                    // and the frame draws with sprites missing. Named here
                    // rather than hidden, and it is what the standing backlog
                    // item about a failed repack is about.
                    Err(error) => eprintln!("packing the art on screen: {error}"),
                }
            }
            Err(error) => eprintln!("growing the atlases: {error}"),
        }

        // Both time-varying halves of a mobile, filled in per frame rather than
        // per packet: the crowd is the only thing that knows what a clock has
        // done since the `0x77` landed, and `self.player`/`self.others` were
        // built when it did. The frame comes from how many the atlas actually
        // packed — asking the atlas rather than remembering the count is what
        // keeps "frame 7 of a 6-frame walk" from being expressible — and the
        // glide is how far into its step the body has walked.
        for (who, mobile) in &mut drawn {
            let (direction, _) = openshard_uofiles::anim::facing(mobile.facing);
            let frame_count = window
                .atlases
                .mobiles
                .frame_count(mobile.body, mobile.group, direction);
            mobile.frame = self.crowd.frame_for(*who, frame_count);
            if let Some(drawn) = self.crowd.drawn_for(*who) {
                mobile.drawn = drawn;
            }
        }
        // Whoever the crowd is still holding a line for, hung above whichever
        // of `drawn`'s mobiles their serial belongs to. Read out here, before
        // `who` is dropped below: a label with no mobile to anchor to has
        // nothing to draw either way, so the two share the same "still on
        // screen" question `mobiles::head_anchor` answers.
        let speech: Vec<(ViewPixel, String, Font, Hue)> = drawn
            .iter()
            .filter_map(|(who, mobile)| {
                let (text, font, hue) = self.crowd.speaking(*who)?;
                let anchor = mobiles::head_anchor(mobile, self.control.camera(), &window.atlases.mobiles)?;
                Some((anchor, text.to_string(), font, hue))
            })
            .collect();
        let drawn: Vec<Mobile> = drawn.into_iter().map(|(_, mobile)| mobile).collect();

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

        // The image the world is drawn into. Its size is the camera's, so a
        // resize and a zoom step are the same event here — and recreating it is
        // the only thing either of them costs.
        let (render_width, render_height) = (
            self.control.camera().render_width(),
            self.control.camera().render_height(),
        );
        if window.world.width() != render_width || window.world.height() != render_height {
            window.world = blit::world_texture(&window.device, render_width, render_height);
            // Tested pixel for pixel against that image, so it is exactly its
            // size or it is nothing.
            window.depth = renderer::depth_texture(&window.device, render_width, render_height);
        }
        let world_view = window.world.create_view(&wgpu::TextureViewDescriptor::default());

        let quads = ground::collect(
            &self.map,
            self.control.camera(),
            &window.atlases.land,
            &window.atlases.texmaps,
        );
        let static_quads = statics::collect(
            &self.map,
            self.control.camera(),
            &self.tiledata,
            &window.atlases.statics,
        );
        // Through the same pass as the map's statics, because they are the same
        // atlas: one draw call binds one texture, and what covers what is the
        // depth these carry rather than the order they are appended in.
        let static_quads = {
            let mut quads = static_quads;
            quads.extend(items::collect(
                &self.items,
                self.control.camera(),
                &self.tiledata,
                &window.atlases.statics,
            ));
            quads
        };
        let mobile_quads = mobiles::collect(&drawn, self.control.camera(), &window.atlases.mobiles);
        let labels: Vec<Label<'_>> = speech
            .iter()
            .map(|(anchor, line, font, hue)| Label {
                anchor: *anchor,
                text: line.as_str(),
                font: *font,
                hue: *hue,
                // Nearer than anything the world draws, rather than an
                // `Order` of its own: speech reads as an overlay above
                // whoever said it in every reference client, and there is no
                // real case here of a wall in front of the speaker hiding it
                // that a viewer would want honoured. Worth revisiting with a
                // `depth::text_priority_z` alongside the mobile's own if that
                // ever stops being true.
                depth: 0.0,
            })
            .collect();
        let text_quads = text::collect(&labels, &self.font_atlas);
        let depth_view = window.depth.create_view(&wgpu::TextureViewDescriptor::default());
        let target = Target {
            view: &world_view,
            depth: &depth_view,
            width: render_width,
            height: render_height,
        };
        let mut encoder = window
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        // Ground first, because it clears; statics after, into what it left.
        // Which covers which is decided by the depth they share, not by this
        // order — the order only decides who clears.
        window
            .renderer
            .render(&window.device, &window.queue, &mut encoder, target, &quads);
        window
            .statics
            .render(&window.device, &window.queue, &mut encoder, target, &static_quads);
        window
            .mobile_pass
            .render(&window.device, &window.queue, &mut encoder, target, &mobile_quads);
        window
            .text_pass
            .render(&window.device, &window.queue, &mut encoder, target, &text_quads);
        // And the world onto the surface, which is where the zoom happens and
        // the only place it does. Into the rect the panels left free, so a
        // docked panel shrinks the world rather than covering it.
        let viewport = ui.as_ref().map_or(
            ViewportRect {
                x: 0,
                y: 0,
                width: window.config.width,
                height: window.config.height,
            },
            |(_, _, viewport)| *viewport,
        );
        window.blit.render(
            &window.device,
            &mut encoder,
            &view,
            &world_view,
            self.control.camera().zoom(),
            viewport,
        );
        // The UI over it, with no depth attachment: the world's depth buffer
        // ordered the world, and this is drawn on the result.
        if let (Some(shell), Some((_, output, _))) = (self.shell.as_mut(), ui) {
            let painting = Instant::now();
            shell.paint(
                &window.device,
                &window.queue,
                &mut encoder,
                &view,
                output,
                [window.config.width, window.config.height],
            );
            ui_cost += painting.elapsed();
        }
        window.queue.submit([encoder.finish()]);
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
        self.frames.record(
            started.saturating_duration_since(self.last_frame),
            ui_cost,
            scene,
            wait,
        );
        self.last_frame = started;
    }
}
