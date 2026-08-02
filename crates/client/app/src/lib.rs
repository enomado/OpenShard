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
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

mod clutter;
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
use openshard_client_render::animate::StaticAnimations;
use openshard_client_render::animation::FRAME_DELAY;
use openshard_client_render::atlas::{
    AnimAtlas, AtlasError, FontAtlas, LandAtlas, StaticAtlas, TexmapAtlas, TtfAtlas,
};
use openshard_client_render::bench::{self, Metrics, Scope, Script};
use openshard_client_render::blit::{self, Blit, ViewportRect};
use openshard_client_render::camera::{self, Camera, TileBounds, ViewPixel};
use openshard_client_render::control::{Control, Follow};
use openshard_client_render::cutaway::Cutaway;
use openshard_client_render::follow::{Gaze, Rig};
use openshard_client_render::hue::HueRamp;
use openshard_client_render::items::{self, GroundItem};
use openshard_client_render::light::{self, Lighting};
use openshard_client_render::mobiles::{self, Mobile};
use openshard_client_render::outline::{self, Outline, Ring};
use openshard_client_render::place;
use openshard_client_render::renderer::{self, GroundRenderer, SpriteRenderer, Target};
use openshard_client_render::text::{self, Label};
use openshard_client_render::{ground, statics};
use openshard_movement::{Heading, Lean, Leeway, Terrain};
use openshard_protocol::direction::{Direction, Facing};
use openshard_protocol::serial::Serial;
use openshard_protocol::speech::Font;
use openshard_protocol::version::ClientVersion;
use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::Point;
use openshard_uofiles::anim::Anim;
use openshard_uofiles::animdata::AnimData;
use openshard_uofiles::art::Art;
use openshard_uofiles::equipconv::EquipConv;
use openshard_uofiles::font::AsciiFonts;
use openshard_uofiles::hues::Hues;
use openshard_uofiles::map::Map;
use openshard_uofiles::texmaps::TexMaps;
use openshard_uofiles::tiledata::TileData;
use openshard_uofiles::ttf_font::TtfFont;
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

/// How close together two left clicks have to land to be a double-click.
///
/// ClassicUO's `Mouse.MOUSE_DELAY_DOUBLE_CLICK`
/// (`src/ClassicUO.Client/Input/Mouse.cs`), taken as it stands: 350ms is what
/// players' hands are used to on this game, and a client that picked its own
/// number would be one where doors sometimes do not open. Distance is
/// deliberately *not* part of the test — the reference does not check it
/// either, and a mouse that slips a pixel between two clicks has not stopped
/// double-clicking.
const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(350);

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

/// The pixel height [`TtfAtlas`] rasterizes at, before the window's own
/// [`winit::window::Window::scale_factor`] scales it up for a dense display —
/// see where [`App::create_window`] builds one. Chosen to sit near
/// `fonts.mul`'s own faces (its glyphs run roughly 8 to 14 pixels tall), not
/// measured against any one of them: see the "One face, not ten" note on
/// [`openshard_uofiles::ttf_font`] for why there is only one size to choose at
/// all.
const TTF_BASE_PIXEL_HEIGHT: f32 = 16.0;

/// Open a window on `dir`'s files, and log in to `shard` if one is given.
///
/// The three arguments are the whole of what this run was asked for: which
/// client install to read, whether there is a shard to play, and which face
/// draws overhead speech. Everything else — the facet, the version claimed,
/// where the camera starts — is a constant above, because none of it is a
/// decision a caller has ever needed to make differently.
///
/// A shard is a [`Dial`] and a [`Plan`] rather than an address and a plan: how
/// the connection is opened is the caller's, which is what lets
/// `crates/e2e/playground` hand over a shard in this same process. Nothing in
/// this crate knows what a socket is any more; `client/net` does not either.
///
/// `ttf_font`, given, switches every line drawn through [`text::collect`] to
/// [`text::collect_ttf`] instead, and `fonts.mul` off entirely — see that
/// function's doc for why it is the whole line or none of it. `None` is the
/// classic client's own bitmap faces, unchanged; `Some` names a TrueType or
/// OpenType face on disk for a shard whose players type in a script
/// `fonts.mul` never shipped, Cyrillic today — nothing is bundled with the
/// engine, see [`openshard_uofiles::ttf_font`]'s doc for why.
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
pub fn run<D: Dial + Send + 'static>(
    dir: &Path,
    shard: Option<(D, Plan)>,
    ttf_font: Option<PathBuf>,
) -> ExitCode {
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
    // What the animated statics cycle through. Read here and folded into
    // `tile_animations` below, because it takes both files to know which
    // graphics animate: the flag is `tiledata.mul`'s and the cycle is this one's.
    // A client without the file animates nothing rather than failing to start —
    // see `AnimData::load`.
    let animdata = match AnimData::load(dir) {
        Ok(animdata) => animdata,
        Err(error) => {
            eprintln!("opening animdata.mul: {error}");
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
    // Read and parsed once, only when asked for: a shard that never sets
    // `ttf_font` has no reason to hold a second face in memory beside
    // `fonts.mul`'s, and one that does is naming a file on this operator's
    // machine — nothing here is bundled with the engine.
    let ttf_font = match ttf_font {
        Some(path) => match TtfFont::open(&path) {
            Ok(font) => Some(font),
            Err(error) => {
                eprintln!("opening {}: {error}", path.display());
                return ExitCode::FAILURE;
            }
        },
        None => None,
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

    // What a worn item draws as. Read alongside `anim`, which is what its
    // entries resolve into.
    let equip_conv = match EquipConv::load(dir.join("Equipconv.def")) {
        Ok(equip_conv) => equip_conv,
        Err(error) => {
            eprintln!("opening Equipconv.def: {error}");
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
    let tiledata = Arc::new(tiledata);
    let link = shard.map(|(dial, plan)| {
        eprintln!("logging in as {}", plan.account.0);
        link::connect(
            dial,
            plan,
            VERSION,
            Arc::clone(&map),
            Arc::clone(&tiledata),
            event_loop.create_proxy(),
        )
    });

    let mut app = App {
        tile_animations: StaticAnimations::build(&animdata, &tiledata),
        // Daylight until asked otherwise: the lighting pass is then exactly the
        // copy the blit has always been.
        night: false,
        flame_clock: std::time::Duration::ZERO,
        map,
        art,
        texmaps,
        tiledata,
        hues,
        hue_ramp,
        font_atlas,
        ttf_font,
        anim,
        equip_conv,
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
            from: None,
            hue: Hue::NONE,
            drawn: Gaze::on(start),
            equipment: Vec::new(),
        },
        cutaway_at: start,
        others: Vec::new(),
        items: Vec::new(),
        item_serials: Vec::new(),
        clutter: clutter::Clutter::default(),
        view: None,
        connection: String::from("offline"),
        shell: None,
        link,
        facet_checked: false,
        steer: {
            // The one decision about walking that is a player's taste rather
            // than a rule: whether a body that has walked into something
            // slides past it or stops against it. Stated here, at the top,
            // because this is the line a client config replaces when there is
            // one — nothing further down the walk has to learn about it.
            //
            // Stopping is the default and is written out rather than left
            // implicit: it is the classic client's own behaviour, and a body
            // that only ever goes where it was pointed is the one that
            // surprises nobody. Sliding is what a player opts into.
            let mut steer = steer::Steering::default();
            steer.set_leeway(Leeway::Eighth);
            steer
        },
        aiming: false,
        ctrl_held: false,
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
        pending: shell::Request::default(),
        selected_tile: None,
        // No click has landed, so the next one cannot be the second of a pair.
        last_click: None,
        // Nobody has pointed at anything yet, and a window that opens under a
        // resting cursor hears `CursorEntered` on the first move.
        pointer_inside: false,
        show_terrain: false,
        // The item under the cursor, ringed and lit, and the ground otherwise:
        // see `shell::HighlightTarget` and `shell::HighlightStyle`.
        highlight: shell::HighlightTarget::default(),
        highlight_style: shell::HighlightStyle::default(),
        covered: None,
        scope: Scope::new(SCOPE_SPAN),
        frames: frames::Frames::new(FRAMES_SPAN),
        repacks: 0,
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
    animations: &StaticAnimations,
    equip_conv: &EquipConv,
) -> Wanted {
    let mut wanted = Wanted::default();
    for band in bands {
        ground::graphics_in(map, band, &mut wanted.land);
        // Every graphic of every cycle, and not the frame on screen: an atlas
        // grown for what a fire is showing this instant is an atlas grown again
        // when it stops showing it. See `StaticAnimations::cycle`.
        statics::graphics_in(map, band, animations, &mut wanted.statics);
    }
    wanted.statics.extend(items::needed_graphics(items, animations));
    wanted
        .animations
        .extend(mobiles::needed_animations(drawn, equip_conv));
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
    /// Which tile each world pixel came from, written by the same three passes
    /// and read by the blit to light the frame in world coordinates — see
    /// `openshard_client_render::place`. Recreated with [`Screen::world`] for
    /// the reason [`Screen::depth`] is: it is an attachment of the same passes
    /// and must be exactly that image's size.
    place: wgpu::Texture,
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
    /// The TrueType glyphs asked for so far, when `App::ttf_font` is set.
    /// Grown a line at a time — see [`App::draw`] — the way [`Screen::atlases`]
    /// grows as the camera walks, because a face with all of Unicode to answer
    /// for has no "whole file" to pack up front the way `fonts.mul` does.
    ttf_atlas: Option<TtfAtlas>,
    /// The pass bound to [`Screen::ttf_atlas`]'s texture, rebuilt whenever that
    /// atlas is (see `App::draw`'s handling of [`AtlasError::Full`] there).
    /// `None` exactly when `ttf_atlas` is.
    ttf_pass: Option<SpriteRenderer>,
    /// Which outlined object each world pixel belongs to, or zero for none.
    ///
    /// Filled by the statics pass drawing silhouettes into it and read by
    /// [`Screen::outline`] after the blit. Recreated with [`Screen::world`] for
    /// the reason [`Screen::depth`] is: it is a colour attachment of a pass whose
    /// depth attachment is that buffer, and the two must be the same size.
    outline_mask: wgpu::Texture,
    /// The pass that turns that mask into a ring on the surface — see
    /// `openshard_client_render::outline`.
    outline: Outline,
}

struct App {
    /// The facet, shared with the shard thread — see [`link::connect`].
    map: Arc<Map>,
    art: Art,
    texmaps: TexMaps,
    /// Shared with the shard thread, the same way [`App::map`] is — see
    /// [`link::connect`]: the walk prediction weighs a pier's or a bridge's
    /// deck now, not only the land, and that needs `tiledata.mul` on both ends
    /// of the channel.
    tiledata: Arc<TileData>,
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
    /// The operator-supplied TrueType face, when `run` was asked to draw
    /// through one instead — `None` is the ordinary, `fonts.mul`-only run. Held here
    /// rather than only in [`Screen`] because it does not depend on a window
    /// existing: it is what [`Screen::ttf_atlas`] is grown from, every frame
    /// [`App::draw`] sees new characters in what is being said.
    ttf_font: Option<TtfFont>,
    /// The animations, open but not read: `anim.mul` is 195MB and frames come
    /// out of it a body at a time. `&mut` because reading one seeks the file.
    anim: Anim,
    /// What a worn item's own graphic resolves to for drawing — see
    /// [`EquipConv`]. Read once at startup like [`App::hues`]: unlike `anim`,
    /// the whole table is small enough to hold rather than seek into.
    equip_conv: EquipConv,
    /// The statics that move on their own — fires, torches, water wheels — and
    /// how far into their cycles they are.
    ///
    /// One of the clocks this app owns, and it is advanced from the same sampled
    /// instant as the crowd and the eye. Its own module argues why it is a system
    /// rather than a flag on a quad: see [`StaticAnimations`].
    tile_animations: StaticAnimations,
    /// Whether the world is drawn as if it were night: dark ambient, and the
    /// fires on the map lighting what is around them. Toggled with F10.
    ///
    /// A local switch and not the shard's clock, because there is no time of day
    /// on the wire yet. When there is, this is the field it writes to and
    /// nothing below it changes — the ambient is already a colour per frame
    /// rather than a constant read by the shader.
    night: bool,
    /// How long the flames have been burning, in the same span every other clock
    /// in the frame is advanced by.
    ///
    /// Its own accumulator rather than an `Instant`, for the reason
    /// [`StaticAnimations`] has one: `openshard-client-render` reads no clock,
    /// so the time arrives as a number, and a number sampled once per frame is
    /// what keeps a torch's flicker on the same instant as the body walking
    /// past it.
    flame_clock: std::time::Duration,
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
    /// The tile roof-cutaway is computed from — see `draw`'s use of it with
    /// [`openshard_client_render::cutaway::Cutaway`].
    ///
    /// Deliberately not always `player.at`: that is this end's own optimistic
    /// *prediction*, published the instant a step is sent and corrected only
    /// a round trip later (see `link::Body`), and `Steering::detour`
    /// (`steer.rs`) means a held direction pinned against an obstacle asks
    /// for the very tile it is going to be refused on, every hold, for as
    /// long as it is held. Feeding that straight to `Cutaway::at` flips which
    /// roof is drawn hidden for exactly the frame between sending the doomed
    /// step and the `0x21` undoing it — a real defect this field exists to
    /// close, not the deliberate lag-compensation `player.at` is for the
    /// body's own drawn position. This only ever advances to a tile the
    /// client's own static map agrees is reachable from the last one it
    /// held, so a refusal is never drawn from; a correction snaps it the same
    /// way it snaps `player.at`.
    cutaway_at: Point,
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
    /// What each of those items is called on the wire, at the same index.
    ///
    /// The renderer drops the serial — it draws pictures and owns no model of
    /// the world — and a click has to put it back, because "use this" is a
    /// serial and nothing else. Built in the same pass as [`App::items`] and
    /// never separately: two loops over one map is how the lists drift, and a
    /// drifted index sends the shard a double-click on whatever was next.
    item_serials: Vec<Serial>,
    /// Which of those items a step cannot go through, indexed by tile.
    ///
    /// A third projection of the view beside [`App::items`] and [`App::others`],
    /// rebuilt with them: the map's own files hold no barrel, so without this
    /// every terrain check here looks straight through one and the shard refuses
    /// the step this end thought was open. See `clutter.rs`.
    clutter: clutter::Clutter,
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
    /// Whether the right button is down, which is what makes dragging steer: a
    /// heading (or, with Ctrl, a destination) is restated on every cursor move
    /// while it is.
    aiming: bool,
    /// Whether Ctrl is held, which is what turns the right-hold from a heading
    /// — the default "run toward the cursor" idiom, no map involved — into a
    /// move order that plans a route with `find_path`. See `steer.rs`'s
    /// module docs.
    ctrl_held: bool,
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
    /// What the last frame's HUD asked for, waiting to be applied at the top of
    /// the next one.
    ///
    /// **The shell's output is the next frame's input, and that is the rule the
    /// frame's ordering rests on.** A request is laid out from a snapshot and
    /// therefore only exists after that snapshot has been taken; applying it
    /// straight away — which is what this used to do — mutates the world and the
    /// camera *between* the readers of one frame, so the overlay egui had already
    /// laid out was drawn against a camera the world pass no longer had. Held for
    /// a frame instead, every writer runs before the snapshot and there is
    /// nothing left in a frame that can move underneath it.
    ///
    /// The delay is a frame on a button press, which is the same latency every
    /// keyboard and mouse event here already has: they arrive between frames and
    /// land on the next one.
    pending: shell::Request,
    /// The tile a left click last landed on, kept until the next click — see
    /// [`App::pick_tile`]. Separate from the live hover so a diagnosis does not
    /// slide off the tile the moment the mouse does.
    selected_tile: Option<(u16, u16)>,
    /// When the last left click landed, or `None` when the one before it
    /// already made a pair.
    ///
    /// The whole of this client's double-click detection, and the reason it is
    /// here rather than asked of the window system: the world's clicks do not go
    /// through egui — see the `MouseInput` arm — and `winit` reports presses,
    /// not gestures. Cleared when a pair fires, which is what stops three clicks
    /// from being two double-clicks; ClassicUO's `GameController` zeroes its own
    /// `lastClickTime` in the same place and for the same reason.
    last_click: Option<Instant>,
    /// Whether the cursor is inside the window at all.
    ///
    /// The other half of "does the world own the mouse", and the half no egui
    /// state can answer: a cursor that has left the window stops sending
    /// positions, so the last one it sent stays true for ever and the highlight
    /// it picked sits on the ground with nobody pointing at it. `CursorLeft` is
    /// the only event that says so.
    pointer_inside: bool,
    /// Whether the HUD is drawing what `common/movement` thinks of the ground —
    /// see [`App::terrain_overlay`].
    ///
    /// Off by default and paid for only while it is on: the overlay asks a
    /// walkability question of every tile in view and plans a route every frame,
    /// which is a bill worth a debugging picture and not worth a frame nobody is
    /// looking at.
    show_terrain: bool,
    /// What the cursor is allowed to light up, and how an item says it is the
    /// one lit. Both are the HUD's to set — see [`shell::HighlightTarget`].
    highlight: shell::HighlightTarget,
    highlight_style: shell::HighlightStyle,
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
    /// How many full atlas repacks this session has paid for — the eviction
    /// `AtlasError::Full` triggers, named in `docs/camera.md`: "costly and
    /// rare" was a claim nothing counted, and each one's cost otherwise reads
    /// as an ordinary heavy frame. See [`Frame::repacked`](frames::Frame) for
    /// which frame paid it.
    repacks: u64,
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
                            let terrain = self.clutter.over(&self.map, &self.tiledata);
                            self.steer.press(
                                direction,
                                self.player.at,
                                Instant::now(),
                                self.player.facing,
                                &terrain,
                            )
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
                    // A diagnostic, not a feature: a fixed mixed-case ASCII
                    // line, sent without ever going through the keyboard —
                    // no xkb group, no IME, no `shell`'s `TextEdit`. Whatever
                    // shows up over the head from this key is exactly what
                    // `0xAD` → `0xAE` → `text::collect` do with known-good
                    // bytes, with typing entirely ruled out as a variable.
                    KeyCode::F9 => {
                        self.say("AbCdEfGh The Quick Brown Fox 123".to_owned());
                        false
                    }
                    // Night on and off. A key and not a setting because the
                    // only honest test of firelight is the two pictures side
                    // by side, and there is no time of day on the wire yet for
                    // it to follow — see `App::night`.
                    KeyCode::F10 => {
                        self.night = !self.night;
                        true
                    }
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
                // Toggling Ctrl mid-drag switches the right-hold from a
                // heading to a move order (or back) on the next cursor move —
                // no special-casing needed, `walk_toward_cursor` reads this
                // fresh every call.
                self.ctrl_held = modifiers.state().control_key();
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
            // A cursor that has left says so once and then goes quiet, so the
            // flag is what stands in for the positions that stop arriving. It
            // reaches here even when egui consumed the move that preceded it:
            // `on_window_event` does not claim these.
            WindowEvent::CursorEntered { .. } => {
                self.pointer_inside = true;
            }
            WindowEvent::CursorLeft { .. } => {
                self.pointer_inside = false;
                if let Some(window) = self.window.as_ref() {
                    window.window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer_inside = true;
                // Relative to the *viewport* and not the window: the camera's
                // own centre is the viewport's, so a cursor measured from the
                // window would zoom about a point half a panel away.
                let origin = self.shell.as_ref().map_or((0, 0), |shell| {
                    (shell.viewport().x as i32, shell.viewport().y as i32)
                });
                let (x, y) = (position.x as i32 - origin.0, position.y as i32 - origin.1);
                let mut changed = self.control.cursor_moved(x, y);
                // Held, the button steers: a heading toward wherever the cursor
                // is, by default, or a Ctrl-held move order — see
                // `walk_toward_cursor` and `steer.rs`'s module docs for why
                // those are two different things and not one idiom stated
                // twice.
                if self.aiming {
                    changed |= self.walk_toward_cursor();
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
                    // The camera as it stands, which between two frames is the
                    // one the last frame was drawn with — the picture the player
                    // is clicking on.
                    let camera = *self.control.camera();
                    self.selected_tile = self.pick_tile(camera).map(|tile| (tile.x, tile.y));
                    // And the second click of a pair is a *use*: a door opens, a
                    // container opens, food is eaten. Which of those it is, is
                    // the shard's answer and not this end's — see
                    // `openshard_client_net::interact`.
                    let now = Instant::now();
                    let paired = self
                        .last_click
                        .is_some_and(|last| now.duration_since(last) <= DOUBLE_CLICK);
                    // Cleared on a pair rather than restarted, so a third click
                    // starts a fresh one — ClassicUO's own reset.
                    self.last_click = (!paired).then_some(now);
                    if paired {
                        self.use_under_cursor(camera);
                    }
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                }
                // A right hold is a heading toward the cursor by default, or a
                // Ctrl-held move order — either way it stays under way while
                // the button is, driven from `CursorMoved`. Left is spoken for
                // by the Tile panel above, and the middle button pans.
                if button == winit::event::MouseButton::Right {
                    self.aiming = state == ElementState::Pressed;
                    if self.aiming {
                        if self.walk_toward_cursor() {
                            if let Some(window) = self.window.as_ref() {
                                window.window.request_redraw();
                            }
                        }
                    } else {
                        // A heading stops the instant the button does — unlike
                        // a move order, which keeps walking itself there after
                        // the button that gave it is gone. `mouse_up` only
                        // touches the heading; a Ctrl-held destination in
                        // flight is untouched.
                        self.steer.mouse_up();
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
            let terrain = self.clutter.over(&self.map, &self.tiledata);
            let Some(facing) = self.steer.due(now, self.player.at, self.player.facing, &terrain) else {
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

        // Turning costs no ground here either, now decided by the same rule
        // the online handshake and the server share
        // (`openshard_movement::intend`) rather than the simplification this
        // used to be — every call moving the body, turn or not, because there
        // was no server round trip to tell the two apart. That was rarely
        // visible when a fresh direction changed once in a while; it stopped
        // being rare once `Steering::detour` started sending several
        // direction changes a hold's worth apart in real cadence, but one
        // right after another within a single event-loop wake — and moving
        // the body on every one of them was a real body covering twice the
        // ground its pace implied.
        let turn = matches!(
            openshard_movement::intend(self.player.at, Facing::walking(self.player.facing), facing),
            openshard_movement::Intent::Turned { .. }
        );
        let (x, y) = match turn {
            true => (self.player.at.x, self.player.at.y),
            false => {
                let (dx, dy) = facing.direction.step();
                let x = (i32::from(self.player.at.x) + dx).clamp(0, self.map.width() as i32 - 1);
                let y = (i32::from(self.player.at.y) + dy).clamp(0, self.map.height() as i32 - 1);
                (x as u16, y as u16)
            }
        };
        // On the surface there — the ground's average, or a platform static's
        // deck, whichever is nearest where the body already stands — not at
        // some height of the camera's, and not the land alone: a mobile below
        // the terrain is correctly hidden by it, which is what the depth
        // buffer is for and what looks exactly like a mobile that failed to
        // draw, and the same held for a pier or a bridge before `predict_z`
        // weighed their deck. See `link.rs`'s online `Command::Step`, which
        // wants the identical answer once a server is involved.
        let terrain = openshard_movement::MapTerrain::new(self.map.as_ref(), &self.tiledata);
        let ground =
            i8::try_from(terrain.predict_z(x, y, i32::from(self.player.at.z))).unwrap_or(self.player.at.z);
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
        // `Crowd::see` starts a fresh `Mobile` with no equipment — nobody
        // sent this placeholder a `0x78` — so whatever it was already wearing
        // is carried across by hand, the way `WorldView` carries it across a
        // `0x77`/`0x20` that names none either.
        let equipment = std::mem::take(&mut self.player.equipment);
        self.player = self.crowd.see(
            None,
            Point::new(x, y, ground),
            Graphic(self.player.body),
            facing,
            self.player.hue,
        );
        self.player.equipment = equipment;
        // Offline there is no shard to refuse a step, so nothing here is
        // speculative the way an online prediction is — trusted outright,
        // same as a correction is.
        self.cutaway_at = self.player.at;
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
    /// The mouse's whole share of walking, one call for both of its idioms:
    /// `self.ctrl_held` says which. Without Ctrl this is a heading — no map
    /// touched, no route planned, the same "run toward the cursor" a strategy
    /// game's held mouse button means. With it, a move order: a route planned
    /// with `find_path` to the exact tile. See `steer.rs`'s module docs for why
    /// they are not the same thing wearing one name.
    fn walk_toward_cursor(&mut self) -> bool {
        // As above: between frames, what is on screen is what the last frame drew.
        let Some(tile) = self.pick_tile(*self.control.camera()) else {
            return false;
        };
        let facing = if self.ctrl_held {
            let terrain = self.clutter.over(&self.map, &self.tiledata);
            self.steer.go_to(
                (tile.x, tile.y),
                self.player.at,
                Instant::now(),
                self.player.facing,
                &terrain,
            )
        } else {
            let terrain = self.clutter.over(&self.map, &self.tiledata);
            self.steer.steer(
                self.heading_to_cursor(*self.control.camera()),
                self.player.at,
                Instant::now(),
                self.player.facing,
                &terrain,
            )
        };
        match facing {
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

    /// Which way the cursor is asking the body to walk — measured **on the
    /// screen**, from where the body is drawn, not in the world's grid.
    ///
    /// The two are not the same question, and the screen one is the only one
    /// the player is actually asking. A player pushes the mouse away from the
    /// character in the direction they want it to go; what "that direction"
    /// means is a bearing on a flat picture. The grid is where the answer has
    /// to land — one of eight tile steps — but it is not where the ask lives,
    /// and measuring in the grid quietly swaps the isometric projection for
    /// nothing. That the two happen to agree for the projection drawn today
    /// (`camera::project` is a rotation and a uniform scale, and rounding to a
    /// sector survives that) is a coincidence of the numbers in it, not a
    /// property of the idea — change the tile to a 2:1 diamond, which is what
    /// most isometric art is, and the grid answer starts naming a direction
    /// the cursor is nowhere near.
    ///
    /// The origin is the body's own projected pixel and not the middle of the
    /// viewport, which is what makes this survive a camera that is not locked
    /// to the body: with a free eye the character is off-centre, sometimes far
    /// off-centre, and "away from the middle of the screen" would be a
    /// different direction from "away from the character". Both are defensible
    /// idioms and a shard may one day want the other; this is the one that
    /// keeps meaning what it means while the eye wanders.
    ///
    /// The sector is picked by the largest dot product against the eight
    /// directions' *projected* steps — normalised, since a diagonal projects to
    /// a longer screen vector than a cardinal and the unnormalised comparison
    /// would hand it sectors it has not earned. Those steps come from
    /// `camera::project` itself rather than from constants copied out of it, so
    /// there is one projection in this client and this reads it.
    ///
    /// `None` when the cursor is exactly on the body: no bearing exists, and
    /// picking one would be inventing an ask.
    fn heading_to_cursor(&self, camera: Camera) -> Option<Heading> {
        let (cursor_x, cursor_y) = self.control.cursor();
        // The body's *drawn* pixel, height and all: what a player aims relative
        // to is the sprite they can see, not the tile beneath it.
        heading_between(camera::project(self.player.at), camera.pick(cursor_x, cursor_y))
    }

    /// Double-click whatever the cursor is over: ask the shard to use it.
    ///
    /// **Picked against the picture, not against the tile.** A door's leaf is
    /// drawn two tiles up the screen from the tile it stands on, so the tile
    /// under the cursor is the one *behind* it — the answer
    /// [`App::pick_tile`] gives, which is right for the Tile panel and wrong for
    /// this. [`items::pick`] hits the sprite's own opaque texels instead, which
    /// is what the player thinks they clicked on.
    ///
    /// Ground items only, so far: the map's statics are not entities and have no
    /// serial to name, and a mobile is a paperdoll request rather than a use —
    /// a different arm of the same packet, waiting on a paperdoll to show. What
    /// this covers is doors, containers and everything else the shard has put on
    /// the ground.
    ///
    /// Nothing is done locally on the way out. The door swings when the `0x1A`
    /// that redraws it arrives; a client that also opened it itself would show
    /// a door the shard may have refused (a lock, or reach) standing open.
    fn use_under_cursor(&self, camera: Camera) {
        // The same question the highlight is drawn from, so the two cannot
        // disagree about whether the world owns the mouse: a click that arrives
        // while a panel holds the pointer is the panel's.
        if !self.world_owns_pointer() {
            return;
        }
        // The atlas is the frame's, and it is where the art the click is tested
        // against lives — offline, or before the first frame, there is nothing
        // drawn to have clicked on.
        let Some(window) = self.window.as_ref() else {
            return;
        };
        // The same cutaway the frame was drawn with, computed the same way: a
        // barrel hidden under a roof this client is not drawing is not something
        // the player can have pointed at.
        let cutaway = Cutaway::at(&self.map, &self.tiledata, self.cutaway_at, true);
        let Some(index) = items::pick(
            &self.items,
            &camera,
            &self.tiledata,
            &self.tile_animations,
            &window.atlases.statics,
            &cutaway,
            self.control.cursor(),
        ) else {
            return;
        };
        let serial = self.item_serials[index];
        match self.link.as_ref() {
            Some(link) => link.use_object(serial),
            None => tracing::info!(serial = serial.raw(), "nothing used: no shard is connected"),
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
            let equipment = std::mem::take(&mut self.player.equipment);
            self.player = self
                .crowd
                .snap(self.me(), start, body, Facing::walking(self.player.facing), hue);
            self.player.equipment = equipment;
            self.cutaway_at = self.player.at;
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
            let equipment = std::mem::take(&mut self.player.equipment);
            self.player = match step.glided {
                true => self.crowd.see(self.me(), step.to, body, step.facing, hue),
                false => self.crowd.snap(self.me(), step.to, body, step.facing, hue),
            };
            self.player.equipment = equipment;
            self.cutaway_at = self.player.at;
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
        self.player.equipment = crowd::worn(&view.player.equipment, &self.tiledata);
        // Sorted by serial for the same reason, and for one more: two items on
        // one tile at one height are drawn in the order they arrive here, so an
        // order that changed every frame would flicker.
        //
        // Before the cutaway guard below, and not with the other projections
        // further down, because that guard asks what this client can already see
        // in its way — and a barrel it was told about in the very packet being
        // folded in is part of that.
        let mut items: Vec<_> = view.items.iter().collect();
        items.sort_unstable_by_key(|(serial, _)| serial.raw());
        self.items.clear();
        self.item_serials.clear();
        for (serial, item) in items {
            self.items.push(GroundItem {
                at: item.position,
                graphic: item.graphic,
                hue: item.hue,
            });
            self.item_serials.push(*serial);
        }
        // The same list read for a second question — not what to draw, but what
        // a step cannot go through. Rebuilt here rather than per decision: one
        // click plans a route over hundreds of tiles, and each of them would
        // otherwise rescan everything on screen. See `clutter.rs`.
        self.clutter = clutter::Clutter::of(&self.items, &self.tiledata);
        // `cutaway_at` follows the same prediction `player.at` does, with one
        // guard: it only ever advances to a tile the client's own static map
        // agrees is reachable from the one it already held. A correction is
        // the server's own word and is trusted outright, same as `player.at`
        // is; an optimistic step is only trusted here when it is not one
        // `Steering::detour` is going to have offered into a wall this end
        // can already see — see the field's own doc for why.
        self.cutaway_at = match body.corrected {
            true => body.predicted.position,
            false => {
                let terrain = self.clutter.over(&self.map, &self.tiledata);
                match terrain.can_step(self.cutaway_at, body.predicted.position) {
                    Some(_) => body.predicted.position,
                    None => self.cutaway_at,
                }
            }
        };
        // Sorted by serial: a `HashMap`'s order is not one, and an atlas built
        // in a different order every frame is a rebuild every frame.
        let mut others: Vec<_> = view.mobiles.iter().collect();
        others.sort_unstable_by_key(|(serial, _)| serial.raw());
        self.others = others
            .into_iter()
            .map(|(serial, mobile)| {
                let who = Some(*serial);
                let mut drawn = self
                    .crowd
                    .see(who, mobile.position, mobile.body, mobile.facing, mobile.hue);
                drawn.equipment = crowd::worn(&mobile.equipment, &self.tiledata);
                (who, drawn)
            })
            .collect();
        // Whoever the view no longer holds walked out of range, and their clock
        // goes with them. Our own body is kept by its serial like anyone else's;
        // the placeholder's `None` is gone the moment a shard names us, which is
        // right — it was never a mobile.
        self.crowd.retain(|who| {
            who.is_some_and(|serial| serial == view.player.serial || view.mobiles.contains_key(&serial))
        });
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
        // The height anything drawn *on* this tile belongs at: the surface a body
        // would stand on, not the ground under it. On a pier those are thirteen
        // z-units apart — the land is water at -15 and the planks are at -3 — and
        // a marker drawn at the land's height sits a tile and a half down the
        // screen from the boards it is meant to be lying on, which is what made
        // the cursor unable to hit a pier tile at all. `predict_z` is the same
        // "which surface, coming from here" the walk itself uses, asked from the
        // body's own height so a floor overhead does not win over the street.
        let terrain = openshard_movement::MapTerrain::new(self.map.as_ref(), &self.tiledata);
        let stand = terrain.predict_z(x, y, i32::from(self.player.at.z));
        shell::PickedTile {
            x,
            y,
            land: land.map(|cell| cell.tile),
            land_z: land.map_or(0, |cell| cell.z),
            // Clamped rather than unwrapped: a `z` outside `i8` is a corrupt
            // block, and a diamond at the wrong height beats a panic in a HUD.
            stand_z: stand.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8,
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
    ///
    /// That height is the *surface*, not the land: a pier's planks stand at `-3`
    /// over water at `-15`, and reading the pixel at the water's height resolved
    /// every pier tile to one more than a tile away — the cursor could not be
    /// put on the boards at all, which is what this is written against. The
    /// same `predict_z` the walk uses, so the tile the cursor names and the tile
    /// a step lands on are one answer rather than two.
    ///
    /// `camera` is the frame's own and not `self.control`'s, for the reason
    /// [`App::hud`] takes one: what tile a pixel is over is a question about the
    /// picture being drawn, and reading it from a camera that has moved since is
    /// how the highlight ends up a frame away from the ground under it.
    fn pick_tile(&self, camera: Camera) -> Option<shell::PickedTile> {
        let (cursor_x, cursor_y) = self.control.cursor();
        let world_px = camera.pick(cursor_x, cursor_y);
        let near = i32::from(self.player.at.z);
        let (mut x, mut y) = camera::unproject(world_px, self.player.at.z);
        if let Some((ux, uy)) = Self::in_bounds(x, y, &self.map) {
            let terrain = openshard_movement::MapTerrain::new(self.map.as_ref(), &self.tiledata);
            let z = terrain.predict_z(ux, uy, near);
            let z = z.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8;
            (x, y) = camera::unproject(world_px, z);
        }
        let (x, y) = Self::in_bounds(x, y, &self.map)?;
        Some(self.tile_info(x, y))
    }

    /// What `common/movement` makes of the ground on screen, and the way through
    /// it — the HUD's terrain overlay, gathered only while it is switched on.
    ///
    /// **Not a second opinion about walkability.** Every answer here comes from
    /// the same [`Terrain`] every step decision on this end asks — the client's
    /// map with the shard's items laid over it — so a tile the picture calls
    /// blocked is a tile the walk will refuse. A private "is this passable"
    /// written for the overlay would be a second policy, and the first bug it hid
    /// would be one of its own.
    ///
    /// Passability is asked per *tile* and not per step: `spawn_z` finds the
    /// surface a body would stand on regardless of how far that is from the
    /// player's own height — so a building's upper floor reads open from the
    /// street rather than blocked — and `can_fit` is what says nothing solid is
    /// standing in the body's space there, the clutter included.
    ///
    /// The route is the plan being walked, if there is one. When there is not,
    /// it is the plan that *would* be walked to the tile under the cursor, which
    /// is the question actually being asked while dragging the mouse over a
    /// building looking for the way in. One [`find_path`] per frame, and only
    /// while the overlay is on.
    fn terrain_overlay(&self, camera: Camera, hover: Option<&shell::PickedTile>) -> shell::TerrainOverlay {
        use openshard_movement::{PLAYER_HEIGHT, Tile, find_path, step_allowed};

        let terrain = self.clutter.over(&self.map, &self.tiledata);
        let near = i32::from(self.player.at.z);
        let mut open = Vec::new();
        let mut blocked = Vec::new();
        // The same clamp the ground pass uses, so the wash covers exactly the
        // tiles that were drawn and no strip of it hangs off the map.
        if let Some((xs, ys)) = camera
            .visible_tiles()
            .clamp_to(self.map.width(), self.map.height())
        {
            for y in ys {
                for x in xs.clone() {
                    let tile = Tile::new(x, y);
                    // The height the diamond is drawn at, and the height the
                    // question is asked about, are one number — the surface a
                    // body would stand on here. A *blocked* tile has one too:
                    // the barrels on a pier stand on the planks, and washing
                    // their tile at the land's height (water, thirteen units
                    // down) drew the refusal a tile and a half away from the
                    // barrel that caused it. `ground_z` is only the fallback for
                    // a tile with no surface at all.
                    let surface = terrain.spawn_z(tile, near);
                    // `clamp` rather than `unwrap`: a `z` outside `i8` is a
                    // corrupt block and not an invariant of ours, and a diamond
                    // drawn at the wrong height is a better answer than a panic
                    // in a debugging overlay.
                    let drawn_z = |z: i32| z.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8;
                    match surface.filter(|&z| terrain.can_fit(tile, z, PLAYER_HEIGHT)) {
                        Some(z) => open.push(Point { x, y, z: drawn_z(z) }),
                        None => blocked.push(Point {
                            x,
                            y,
                            z: surface.map_or_else(|| terrain.ground_z(tile).unwrap_or(0), drawn_z),
                        }),
                    }
                }
            }
        }

        // Directions, from wherever they come from, walked out into the tiles
        // they land on — `step_allowed` because it is what corrects a step's `z`
        // to the surface it lands on, which is the height the marker is drawn at.
        let mut steps: Vec<Direction> = self.steer.route().collect();
        if steps.is_empty() {
            if let Some(tile) = hover {
                // The surface, like everything else here: a route planned to the
                // water under a pier is a route to somewhere nobody is standing.
                let to = Point {
                    x: tile.x,
                    y: tile.y,
                    z: tile.stand_z,
                };
                steps = find_path(&terrain, self.player.at, to, steer::PLAN_BUDGET).unwrap_or_default();
            }
        }
        let mut at = self.player.at;
        let mut route = vec![at];
        for direction in steps {
            let Some(next) = step_allowed(&terrain, at, direction) else {
                // The plan and the ground disagree, which is a thing worth
                // seeing rather than papering over: the line stops where they
                // parted company.
                break;
            };
            at = next;
            route.push(at);
        }

        shell::TerrainOverlay { open, blocked, route }
    }

    /// Do what the HUD asked for on the frame before this one.
    ///
    /// Every writer the shell has, in one place and at one moment: the top of a
    /// frame, before anything reads. See [`App::pending`] for why it is a frame
    /// late and why that is the point rather than a compromise.
    ///
    /// The viewport is deliberately not in here. It is not something a widget
    /// *asked* for — it is what the layout left over, which `Shell` holds between
    /// frames — and it is applied beside this call rather than through it.
    fn apply(&mut self, request: shell::Request) {
        if request.relock {
            self.relock();
        } else if request.unlock {
            self.control.unlock();
        }
        if let Some(rig) = request.rig {
            // The eye does not move — that is what `set_rig` promises — but the
            // frames before the swap were flown by another camera, and measuring
            // them together would average two rigs.
            self.control.set_rig(rig);
            self.scope.clear();
        }
        // The body's ease is not the rig and does not clear the scope: the frames
        // either side of it were flown by the same camera, and what the scope
        // measures is the eye against the body it was given.
        if let Some(ease) = request.ease {
            self.crowd.set_ease(ease);
        }
        if let Some(show) = request.show_terrain {
            self.show_terrain = show;
        }
        if let Some(target) = request.highlight {
            self.highlight = target;
        }
        if let Some(style) = request.highlight_style {
            self.highlight_style = style;
        }
        // The window the metrics are taken over, and not a clear: the frames
        // already held were flown by the same rig.
        if let Some(span) = request.scope_span {
            self.scope.set_span(span);
        }
        match request.script {
            Some(shell::ScriptRequest::Run(name)) => self.start_replay(name),
            Some(shell::ScriptRequest::Stop) => self.replay = None,
            None => {}
        }
        if let Some(line) = request.say {
            self.say(line);
        }
        if let Some(reply) = request.gump {
            self.answer_gump(reply);
        }
    }

    /// What the panels are allowed to know, gathered each frame.
    ///
    /// `camera` is the frame's own, handed in rather than read back from
    /// [`App::control`]: the overlay the shell draws from this and the world pass
    /// below it are two readers of one picture, and the only way they cannot
    /// disagree is for there to be one value. See [`App::draw`].
    /// Whether the world may read the cursor at all.
    ///
    /// Asked once and answered for the whole frame. A pointer over a panel picks
    /// no tile and lights no item, so nothing is highlighted under the panel and
    /// nothing is highlighted where the pointer *was* when it went over one; a
    /// pointer that has left the window is the other half, and the one no egui
    /// state can answer — see [`App::pointer_inside`] and
    /// [`shell::Shell::holds_pointer`].
    fn world_owns_pointer(&self) -> bool {
        self.pointer_inside && !self.shell.as_ref().is_some_and(shell::Shell::holds_pointer)
    }

    /// `lit_item` is what [`items::pick`] answered for this frame, handed in
    /// rather than asked again: the HUD and the world passes are two readers of
    /// one picture, and the tile marker is drawn or not drawn on the strength of
    /// whether an item took the highlight. Asking twice would be two answers to
    /// "what is the cursor on", and the frame where they disagree is the frame a
    /// barrel is ringed *and* the ground under it is diamonded.
    fn hud(&self, camera: Camera, lit_item: Option<usize>) -> shell::Hud {
        let hover = match self.world_owns_pointer() {
            true => self.pick_tile(camera),
            false => None,
        };
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
            camera,
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
            repacks: self.repacks,
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
            show_terrain: self.show_terrain,
            // The tile is lit when nothing else took the highlight. Under
            // `Items` nothing ever does, which is the mode's whole content; the
            // ground is still hovered and the panel still reads it.
            hover_lit: match self.highlight {
                shell::HighlightTarget::Auto => lit_item.is_none(),
                shell::HighlightTarget::Items => false,
                shell::HighlightTarget::Tiles => true,
            },
            highlight: self.highlight,
            highlight_style: self.highlight_style,
            terrain: self
                .show_terrain
                .then(|| self.terrain_overlay(camera, hover.as_ref())),
            hover,
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
        mobiles.push((self.me(), self.player.clone()));
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
        // Without this, the compositor never starts an IME session for this
        // window, and on Wayland that is what feeds `egui-winit` composed
        // text: a layout that needs one (Cyrillic under a caps-lock layout
        // switch, an East Asian input method) either loses every keystroke or
        // the raw keysym instead of the composed character, silently, while a
        // plain Latin layout still works because it needs no composition —
        // the shell's "say" box looked fine to type in for exactly that
        // reason and nothing else.
        window.set_ime_allowed(true);

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
        // Scaled by the window's own density: a `TtfAtlas` bakes one pixel
        // size into every glyph it packs (see its doc), so the size has to be
        // picked once, here, where a real `Window` first exists to ask —
        // `run` cannot ask before one does, and rebuilding a size already
        // packed at is exactly the "ten faces" cost `ttf_font`'s doc explains
        // this engine does not pay.
        let (ttf_atlas, ttf_pass) = match &self.ttf_font {
            Some(_) => {
                let atlas = TtfAtlas::empty(TTF_BASE_PIXEL_HEIGHT * window.scale_factor() as f32);
                let pass = SpriteRenderer::new(
                    &device,
                    &queue,
                    blit::WORLD_FORMAT,
                    atlas.pixels(),
                    &self.hue_ramp,
                );
                (Some(atlas), Some(pass))
            }
            None => (None, None),
        };
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
        let outline_mask = outline::mask_texture(
            &device,
            self.control.camera().render_width(),
            self.control.camera().render_height(),
        );
        let place = place::texture(
            &device,
            self.control.camera().render_width(),
            self.control.camera().render_height(),
        );
        let blit = Blit::new(&device, format);
        // The surface's format and not the world's: the ring is drawn over the
        // blit's output, so that a highlight is not dimmed by the night the way
        // the picture under it is.
        let outline = Outline::new(&device, format);
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
            place,
            mobile_pass,
            atlases,
            text_pass,
            ttf_atlas,
            ttf_pass,
            outline_mask,
            outline,
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
    ///
    /// `camera` is the frame's snapshot — see [`App::hud`]. What the atlases are
    /// grown for has to be what the passes below then draw, or a band is packed
    /// for one rectangle and sampled for another.
    fn wanted_since(&self, camera: Camera, covered: Option<TileBounds>) -> Wanted {
        let bounds = camera.visible_tiles();
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
        wanted_in(
            &self.map,
            bands,
            &self.items,
            &drawn,
            &self.tile_animations,
            &self.equip_conv,
        )
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

        // # The frame is three steps, and this is the first of them
        //
        // Everything that writes runs here, before anything reads. What the
        // shell asked for last frame, then every clock, then the eye — and after
        // this block nothing in the frame moves the world or the camera again.
        //
        // The defect it is written against: the HUD used to be built at the top
        // of the frame and the eye moved a few lines further down, so the
        // overlay egui laid out — the tile highlight, the hover, the walk goal —
        // was drawn against the *previous* frame's camera while the world pass
        // below drew from this one's. The gap between them is one frame of camera
        // motion, which is not a constant: it is whatever the display gave this
        // frame, so the markers shivered against the ground they were meant to be
        // lying on, and every missed interval made them jump. Reordering two
        // calls would have fixed today's version of it and left the shape that
        // produced it, which is a second reader picking the camera up at a
        // different moment. So the frame is staged instead, and the snapshot
        // below is what both readers are handed.
        let asked = std::mem::take(&mut self.pending);
        self.apply(asked);
        // The viewport the last frame's layout left free — `Shell` holds it
        // between frames for exactly this. It has to be settled before the eye
        // is, because it is what decides how much world a camera can see.
        if let Some(shell) = self.shell.as_ref() {
            let viewport = shell.viewport();
            self.control.resize(viewport.width, viewport.height);
        }
        self.crowd.advance(elapsed);
        // The statics that move on their own, on the same span as everybody
        // else. Its own clock inside — a fire's cycle has nothing to do with a
        // walk's — and one *sample*, which is the whole rule: two clocks read
        // from two `Instant::now()`s a few hundred microseconds apart would put
        // a torch and the body that walks past it on two different instants.
        self.tile_animations.advance(elapsed);
        // And the flames, off the same span: a fire's animation frame and the
        // brightness of the pool it casts are two clocks describing one fire,
        // and they are advanced together or they describe two.
        self.flame_clock += elapsed;
        self.last_advance = started;
        // Whatever scenario is being walked delivers its knots for the span that
        // just passed, before the eye is asked where the body is: a step that
        // arrived this frame is one the camera has to answer this frame.
        self.advance_replay(elapsed);
        // A viewport that grew may have taken the world texture past what the
        // device allows, which no zoom step asked for.
        self.fit_zoom_to_device();
        // And the eye goes where the body is *this frame*: a step arrives once
        // and is then walked across for the next 400ms, so every frame in
        // between has a different answer.
        self.follow_player(elapsed);

        // # Step two: one snapshot, and it is a value
        //
        // The camera the whole frame is built from, copied out rather than read
        // back from `self.control` at each use. A `&Camera` handed to five
        // collectors is five reads of a field that something between them might
        // have moved — which is the defect above, expressed as a borrow. A
        // `Camera` is `Copy`, so this costs nothing and cannot be stale in one
        // place and fresh in another.
        let camera = *self.control.camera();

        // Read before the window is borrowed below, for the same reason the line
        // above is: the borrow is of `self`, and the pacing at the foot of this
        // frame is a fact about the whole app rather than about it.
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
        // `self.cutaway_at`, not `self.player.at`: the latter is this end's
        // own unconfirmed prediction, which for one frame can be a tile a
        // held direction was refused on — see the field's own doc.
        //
        // Here, in the snapshot, and not beside the passes that draw from it:
        // the item pick below needs it, and the pick has to be answered before
        // the HUD is built — see the next paragraph.
        let cutaway = Cutaway::at(&self.map, &self.tiledata, self.cutaway_at, true);
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
        let lit_item = match owns_pointer && self.highlight != shell::HighlightTarget::Tiles {
            true => self.window.as_ref().and_then(|window| {
                items::pick(
                    &self.items,
                    &camera,
                    &self.tiledata,
                    &self.tile_animations,
                    &window.atlases.statics,
                    &cutaway,
                    cursor,
                )
            }),
            false => None,
        };

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
        let hud = self.hud(camera, lit_item);
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
        if let Some((request, _, _)) = &ui {
            self.pending = request.clone();
        }

        // What the camera has walked onto since the atlases were last grown.
        // Gathered before the window is borrowed, and not inside the borrow: it
        // reads the whole of `self`, and the window is part of it.
        let want = camera.visible_tiles();
        let wanted = self.wanted_since(camera, self.covered);
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
        // Set only in the branch below, on a successful rebuild — this is the
        // counter `docs/camera.md` asks for, so the frame that stalled for it
        // can be told apart from one that is merely heavy. See
        // [`Frame::repacked`](frames::Frame).
        let mut repacked = false;
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
                        [camera.visible_tiles()],
                        &self.items,
                        &drawn.iter().map(|(_, mobile)| mobile.clone()).collect::<Vec<_>>(),
                        &self.tile_animations,
                        &self.equip_conv,
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
                        repacked = true;
                        self.repacks += 1;
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

        // Three time-varying halves of a mobile, filled in per frame rather
        // than per packet: the crowd is the only thing that knows what a
        // clock — and a group — has done since the `0x77` landed, and
        // `self.player`/`self.others` were built when it did. The group is
        // read back first and not only the frame and the glide: `Crowd::advance`
        // drops a walking body to standing on its own timer, with nothing
        // that looks like a packet to refresh `mobile.group` from — a group
        // read once here and left stale plays the walking group's sprite
        // forever, timed by a clock that already moved on to the standing
        // group's, which is a body that has stopped walking but not stopped
        // *looking* like it is. The frame comes from how many the atlas
        // actually packed — asking the atlas rather than remembering the
        // count is what keeps "frame 7 of a 6-frame walk" from being
        // expressible — and the glide is how far into its step the body has
        // walked.
        for (who, mobile) in &mut drawn {
            if let Some(group) = self.crowd.group_for(*who) {
                mobile.group = group;
            }
            let (direction, _) = openshard_uofiles::anim::facing(mobile.facing);
            // Under the body the *atlas* packed, which for a ghost is the
            // living body it borrows its pictures from — the same
            // `anim::animation_body` `mobiles::collect` looks its frame up with.
            // Asked under the wire's body instead, a ghost counts zero frames
            // and every clock lands on frame 0: the sprite is drawn and never
            // moves, which is a walking body that slides along standing still.
            let frame_count = window.atlases.mobiles.frame_count(
                openshard_uofiles::anim::animation_body(mobile.body),
                mobile.group,
                direction,
            );
            mobile.frame = self.crowd.frame_for(*who, frame_count);
            if let Some(drawn) = self.crowd.drawn_for(*who) {
                mobile.drawn = drawn;
            }
            // And which tile it sorts at, which is a step's own clock too: the
            // crossing ends without a packet to say so, and a body still sorted
            // on the tile it left would keep drawing over the ground behind it.
            mobile.from = self.crowd.stepping_from(*who);
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
                let anchor = mobiles::head_anchor(mobile, &camera, &window.atlases.mobiles)?;
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
        let (render_width, render_height) = camera.image_size();
        if window.world.width() != render_width || window.world.height() != render_height {
            window.world = blit::world_texture(&window.device, render_width, render_height);
            // Tested pixel for pixel against that image, so it is exactly its
            // size or it is nothing.
            window.depth = renderer::depth_texture(&window.device, render_width, render_height);
            // And the mask with it: it is the colour attachment of a pass whose
            // depth attachment is that buffer, and wgpu requires the two to be
            // one size.
            window.outline_mask = outline::mask_texture(&window.device, render_width, render_height);
            // And the place channel, which is an attachment of those same
            // passes and is read texel for texel against that image.
            window.place = place::texture(&window.device, render_width, render_height);
        }
        let world_view = window.world.create_view(&wgpu::TextureViewDescriptor::default());

        let quads = ground::collect(
            &self.map,
            &camera,
            &window.atlases.land,
            &window.atlases.texmaps,
            &cutaway,
        );
        let static_quads = statics::collect(
            &self.map,
            &camera,
            &self.tiledata,
            &self.tile_animations,
            &window.atlases.statics,
            &cutaway,
        );
        // Through the same pass as the map's statics, because they are the same
        // atlas: one draw call binds one texture, and what covers what is the
        // depth these carry rather than the order they are appended in.
        // One pick (`lit_item`, at the top of the frame), two effects, and the
        // style decides which of them is asked for. `None` is how each is
        // switched off, so neither pass has a mode to branch on: the hue pass
        // draws an item that is not highlighted, and the silhouette pass is
        // handed an empty list.
        let hued = self.highlight_style.hues().then_some(lit_item).flatten();
        let ringed = self.highlight_style.rings().then_some(lit_item).flatten();
        // The same quads as the picture's, so the ring lands on the sprite
        // rather than beside it — see `items::outlined`.
        let outline_quads = items::outlined(
            &self.items,
            &camera,
            &self.tiledata,
            &self.tile_animations,
            &window.atlases.statics,
            &cutaway,
            ringed,
        );
        let static_quads = {
            let mut quads = static_quads;
            quads.extend(items::collect(
                &self.items,
                &camera,
                &self.tiledata,
                &self.tile_animations,
                &window.atlases.statics,
                &cutaway,
                hued,
            ));
            quads
        };
        let mobile_quads = mobiles::collect(
            &drawn,
            &camera,
            &window.atlases.mobiles,
            &cutaway,
            &self.equip_conv,
        );
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
        // `fonts.mul` or the operator-supplied TrueType face, never a mix
        // within one frame — see `run`'s doc for why `ttf_font` is an all-or-nothing
        // switch. Unlike `font_atlas`, `ttf_atlas` is grown a line at a time:
        // there is no bounded "whole file" to pack up front for a face that
        // answers to all of Unicode, so this asks it to rasterize whatever of
        // this frame's speech it has not seen yet, the way `window.atlases`
        // grows for graphics newly on screen.
        let text_quads = if let Some(font) = &self.ttf_font {
            let atlas = window
                .ttf_atlas
                .as_mut()
                .expect("create_window builds ttf_atlas whenever ttf_font is set");
            if let Err(error) = atlas.add(font, labels.iter().flat_map(|label| label.text.chars())) {
                // `eprintln!` and a frame that draws anyway, the same corner
                // `AtlasError::Full` already cuts for the map's own atlases —
                // see docs/client.md. Unreachable in practice: a shard's whole
                // spoken character set is a few hundred glyphs at most, nowhere
                // near one 2048 texture.
                eprintln!("packing ttf glyphs: {error}");
            }
            if let Some(rows) = atlas.take_dirty() {
                window
                    .ttf_pass
                    .as_ref()
                    .expect("create_window builds ttf_pass whenever ttf_atlas is")
                    .upload_rows(&window.queue, atlas.pixels(), rows);
            }
            text::collect_ttf(&labels, atlas)
        } else {
            text::collect(&labels, &self.font_atlas)
        };
        let depth_view = window.depth.create_view(&wgpu::TextureViewDescriptor::default());
        let place_view = window.place.create_view(&wgpu::TextureViewDescriptor::default());
        let target = Target {
            view: &world_view,
            depth: &depth_view,
            place: &place_view,
            width: render_width,
            height: render_height,
            projection: camera.projection(),
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
        // The silhouettes, here and not later: the mask is depth-tested against
        // what the three world passes have drawn, so a barrel behind a wall is
        // kept out of it — and the text pass below writes depth at the near
        // plane over everything, which would punch the mask through.
        let mask_view = window
            .outline_mask
            .create_view(&wgpu::TextureViewDescriptor::default());
        window.statics.render_mask(
            &window.device,
            &window.queue,
            &mut encoder,
            target,
            &mask_view,
            &outline_quads,
        );
        // `ttf_pass` when the run is drawing through it — bound to a
        // different texture than `text_pass`, so a mix of the two within one
        // frame would sample one atlas with quads packed for the other.
        let text_renderer = match &mut window.ttf_pass {
            Some(pass) => pass,
            None => &mut window.text_pass,
        };
        text_renderer.render(&window.device, &window.queue, &mut encoder, target, &text_quads);
        // And the world image onto the surface, into the rect the panels left
        // free. Magnified this is a copy — the image is already the viewport's
        // size and the magnification happened in the vertex transform — and
        // minified it is where the shrinking happens, which is why the zoom is
        // still what picks the sampler.
        //
        // The lights are collected here, from the same camera, cutaway and item
        // list the passes above drew from — so a torch that was not drawn casts
        // nothing, and a torch that was is lighting the pixels it is standing
        // in rather than the pixels it stood in last frame.
        let lighting = match self.night {
            true => light::collect(
                &self.map,
                &self.items,
                &camera,
                &self.tiledata,
                &cutaway,
                light::NIGHT,
                self.flame_clock.as_secs_f32(),
            ),
            false => Lighting::NONE,
        };
        window.blit.render(
            &window.device,
            &window.queue,
            &mut encoder,
            blit::Frame {
                target: &view,
                world: &world_view,
                place: &place_view,
                zoom: camera.zoom(),
                rect: viewport,
            },
            &lighting,
        );
        // And the ring on top of that, over the same rectangle — after the blit
        // so it is drawn in screen pixels and unlit: a highlight that dimmed at
        // night would stop working exactly when the picture is hardest to read.
        // Skipped entirely on the ordinary frame, where nothing is under the
        // cursor and the mask is empty.
        if !outline_quads.is_empty() {
            window.outline.render(
                &window.device,
                &window.queue,
                &mut encoder,
                outline::Frame {
                    target: &view,
                    mask: &mask_view,
                    mask_size: (render_width, render_height),
                    rect: viewport,
                },
                // The soft ring — an edge with a glow behind it — widened when
                // the world is minified, where one mask texel is less than one
                // screen pixel and a hairline breaks into a dashed line. See
                // `Ring::for_zoom`.
                Ring::SOFT.for_zoom(camera.zoom()),
            );
        }
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
            repacked,
        );
        self.last_frame = started;
    }
}

/// The heading from one point on the screen to another, as one of the eight
/// ways a body can walk plus which side of that way it actually points.
///
/// Split out of [`App::heading_to_cursor`] because it is the whole of the
/// arithmetic and none of the state — a thing that can be checked against a
/// drawn picture rather than against a running window.
///
/// The sector is the largest dot product against the eight directions'
/// *projected* steps, normalised: a diagonal projects to a longer screen vector
/// than a cardinal (44 pixels against 31), and comparing unnormalised would
/// hand the diagonals sectors they have not earned. Those steps come from
/// [`camera::project`] rather than from constants copied out of it, so there is
/// one projection in this client and this reads it.
fn heading_between(body: camera::WorldPixel, cursor: camera::WorldPixel) -> Option<Heading> {
    let (dx, dy) = (cursor.x - body.x, cursor.y - body.y);
    if dx == 0 && dy == 0 {
        return None;
    }
    let direction = Direction::ALL.into_iter().max_by(|a, b| {
        let cosine = |direction| {
            let (sx, sy) = on_screen(direction);
            let dot = f64::from(dx) * f64::from(sx) + f64::from(dy) * f64::from(sy);
            dot / f64::from(sx * sx + sy * sy).sqrt()
        };
        cosine(*a).total_cmp(&cosine(*b))
    })?;
    let (sx, sy) = on_screen(direction);
    Some(Heading {
        direction,
        // A cross product needs no normalising, so the lean stays exact: a
        // cursor squarely on a direction's screen bearing leans neither way and
        // says so without a tolerance. The projection turns the plane without
        // flipping it, so "clockwise" means on the screen what it means on the
        // grid — see `Lean::of`.
        lean: Lean::of(sx, sy, dx, dy),
    })
}

/// One step's worth of the projection, taken from the projection.
///
/// The origin tile is arbitrary and cancels in the subtraction; it is away from
/// the map's edges only so that neither end of it has to clamp.
fn on_screen(direction: Direction) -> (i32, i32) {
    let origin = Point::new(1000, 1000, 0);
    let (sx, sy) = direction.step();
    let stepped = Point::new(
        (i32::from(origin.x) + sx) as u16,
        (i32::from(origin.y) + sy) as u16,
        0,
    );
    let (a, b) = (camera::project(origin), camera::project(stepped));
    (b.x - a.x, b.y - a.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The screen bearings of the eight directions, as the isometric actually
    /// draws them — and they are not the grid's. On screen the diamond is
    /// turned an eighth: north-east points due right, south-east due down.
    /// Anything reading a cursor has to answer in *these* terms, which is the
    /// whole reason the heading is measured here rather than on the grid.
    #[test]
    fn the_screen_bearings_are_the_grid_turned_an_eighth() {
        assert_eq!(on_screen(Direction::NorthEast), (44, 0), "due right");
        assert_eq!(on_screen(Direction::SouthEast), (0, 44), "due down");
        assert_eq!(on_screen(Direction::SouthWest), (-44, 0), "due left");
        assert_eq!(on_screen(Direction::NorthWest), (0, -44), "due up");
        assert_eq!(on_screen(Direction::East), (22, 22), "down and right");
        assert_eq!(on_screen(Direction::North), (22, -22));
        assert_eq!(on_screen(Direction::South), (-22, 22));
        assert_eq!(on_screen(Direction::West), (-22, -22));
    }

    /// A cursor held away from the body in each of those eight screen bearings
    /// asks for that direction — including the one that catches a heading
    /// measured on the grid by mistake: straight down the screen is
    /// *south-east*, and a grid reading would call it south.
    #[test]
    fn a_cursor_on_a_screen_bearing_asks_for_that_direction() {
        let body = camera::WorldPixel { x: 0, y: 0 };
        for direction in Direction::ALL {
            let (sx, sy) = on_screen(direction);
            let cursor = camera::WorldPixel { x: sx * 7, y: sy * 7 };
            let heading = heading_between(body, cursor).expect("the cursor is not on the body");
            assert_eq!(heading.direction, direction, "screen bearing {sx},{sy}");
            assert_eq!(
                heading.lean,
                Lean::Centred,
                "squarely on the bearing leans neither way"
            );
        }
    }

    /// And off the bearing, the lean says which side — which is the thing the
    /// eight sectors throw away and the only thing that can settle a corner
    /// with two open ways round it. Straight down the screen is south-east;
    /// nudged to the right of that, the ask is still south-east but is leaning
    /// toward east, which is where east is drawn.
    #[test]
    fn a_cursor_off_the_bearing_leans_toward_the_side_it_is_on() {
        let body = camera::WorldPixel { x: 0, y: 0 };
        let down_and_right = heading_between(body, camera::WorldPixel { x: 6, y: 300 }).unwrap();
        assert_eq!(down_and_right.direction, Direction::SouthEast);
        assert_eq!(down_and_right.lean, Lean::Counter);

        let down_and_left = heading_between(body, camera::WorldPixel { x: -6, y: 300 }).unwrap();
        assert_eq!(down_and_left.direction, Direction::SouthEast);
        assert_eq!(down_and_left.lean, Lean::Clockwise);
    }

    /// The cursor on the body names no direction at all, rather than the
    /// nearest one: an ask nobody made.
    #[test]
    fn a_cursor_on_the_body_asks_for_nothing() {
        let body = camera::WorldPixel { x: 17, y: -3 };
        assert_eq!(heading_between(body, body), None);
    }
}
