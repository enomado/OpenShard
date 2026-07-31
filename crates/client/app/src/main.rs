//! The client, as far as it goes: a window onto Britannia's ground.
//!
//! Run it against a real client install, which this repository never contains:
//!
//! ```sh
//! OPENSHARD_CLIENT="/path/to/Ultima Online Classic" cargo run -p openshard-client-app
//! ```
//!
//! Arrow keys walk a tile at a time. The wheel zooms about the cursor, a
//! middle-drag pans, page up and down pan vertically, `Home` puts the camera
//! back on the body and locks it there, and escape closes the window.
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

use std::fmt;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

mod link;
mod shell;

use openshard_client_net::session::{Pick, Plan};
use openshard_client_net::view::WorldView;
use openshard_client_render::animation::{AnimationClock, FRAME_DELAY};
use openshard_client_render::atlas::{AnimAtlas, LandAtlas, StaticAtlas, TexmapAtlas};
use openshard_client_render::blit::{self, Blit, ViewportRect};
use openshard_client_render::camera::{Camera, WorldPixel};
use openshard_client_render::hue::HueRamp;
use openshard_client_render::mobiles::{self, Mobile};
use openshard_client_render::renderer::{self, GroundRenderer, SpriteRenderer, Target};
use openshard_client_render::{ground, statics};
use openshard_protocol::direction::{Direction, Facing};
use openshard_protocol::identity::{RawAccountName, RawPlaintextPassword};
use openshard_protocol::version::ClientVersion;
use openshard_protocol::wire::Hue;
use openshard_protocol::world::Point;
use openshard_uofiles::anim::Anim;
use openshard_uofiles::art::Art;
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

/// Where a shard is, when one is asked for and no address is given.
const DEFAULT_SHARD: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 2593);

/// The login this run was asked to make, if it was asked for one.
///
/// The account is what decides: a client with no account has nobody to log in
/// as, and asking for a password on a command line nobody typed would be worse
/// than drawing the map on its own.
fn plan_from_environment() -> Option<(SocketAddrV4, Plan)> {
    let account = std::env::var("OPENSHARD_ACCOUNT").ok()?;
    let password = std::env::var("OPENSHARD_PASSWORD").unwrap_or_default();
    let address = match std::env::var("OPENSHARD_SERVER") {
        Ok(text) => match text.parse() {
            Ok(address) => address,
            Err(error) => {
                eprintln!("OPENSHARD_SERVER is not an address:port: {error}");
                return None;
            }
        },
        Err(_) => DEFAULT_SHARD,
    };
    Some((
        address,
        Plan {
            account: RawAccountName(account),
            password: RawPlaintextPassword(password),
            shard: Pick::First,
            character: std::env::var("OPENSHARD_CHARACTER").map_or(Pick::First, Pick::Named),
        },
    ))
}

fn main() -> ExitCode {
    let Some(dir) = std::env::var_os("OPENSHARD_CLIENT").map(PathBuf::from) else {
        eprintln!("set OPENSHARD_CLIENT to a client install directory");
        return ExitCode::FAILURE;
    };

    // Reading the whole facet takes a moment and a few hundred megabytes. That
    // is the shape `uofiles` has today — see the backlog in docs/client.md — and
    // it is honest to do it up front rather than to stall on the first frame.
    let map = match Map::load_facet(&dir, FACET) {
        Ok(map) => map,
        Err(error) => {
            eprintln!("loading facet {FACET}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let art = match Art::open(&dir) {
        Ok(art) => art,
        Err(error) => {
            eprintln!("opening artLegacyMUL.uop: {error}");
            return ExitCode::FAILURE;
        }
    };
    // The two files a slope needs: the square textures, and the table that says
    // which of them a land graphic uses.
    let texmaps = match TexMaps::open(&dir) {
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

    let anim = match Anim::open(&dir) {
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
    let link = plan_from_environment().map(|(address, plan)| {
        eprintln!("logging in to {address} as {}", plan.account.0);
        link::connect(address, plan, VERSION, event_loop.create_proxy())
    });

    let mut app = App {
        map,
        art,
        texmaps,
        tiledata,
        hue_ramp,
        anim,
        camera: Camera::new(START, 1024, 768),
        follow: Follow::Body,
        drag: Drag::default(),
        // Replaced by the device's own limit once there is one. WebGL2's floor
        // until then, which is the smallest thing this has to run on.
        max_texture: 2048,
        zoom_limit_reported: false,
        player: Mobile {
            at: start,
            // 400 is the male human body and 4 is its standing group; the
            // clock below picks the frame every redraw.
            body: 400,
            group: 4,
            facing: Direction::SouthEast,
            frame: 0,
            hue: Hue::NONE,
        },
        others: Vec::new(),
        view: None,
        connection: String::from("offline"),
        frame_time: std::time::Duration::ZERO,
        shell: None,
        link,
        facet_checked: false,
        animation: AnimationClock::default(),
        next_tick: Instant::now(),
        window: None,
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

/// Whether the camera is tied to the body or the mouse.
///
/// It lives here and not in `Camera`: the camera does not know what a player is,
/// and giving it one would put `client/net` inside `client/render`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Follow {
    /// The eye is the body's, and the server moves it.
    Body,
    /// The eye is the mouse's, and the body may walk off screen.
    Free,
}

/// What the mouse is doing to the camera.
///
/// The fraction is the reason this is a struct and not a flag. At zoom 2 a
/// one-pixel drag is half a world pixel, and an eye that carried the fraction
/// would put every sprite on a half-texel boundary for half of all camera
/// positions — the same class of defect as the half-texel inset the atlases
/// apply, spread across the whole frame instead of one edge. So the remainder
/// is accumulated here, in the input handler, and only whole world pixels reach
/// [`Camera::look_at_pixel`].
#[derive(Clone, Copy, Default, Debug)]
struct Drag {
    /// Where the cursor was last seen, in physical pixels from the viewport's
    /// top-left. Needed by the wheel, which is told a delta and not a position.
    cursor: (i32, i32),
    /// Whether the middle button is down.
    panning: bool,
    /// Viewport pixels dragged and not yet spent, numerator over the zoom's.
    remainder: (i32, i32),
}

/// The animation group a body that is doing nothing plays.
///
/// Every mobile stands here. Walking, running and everything else are groups of
/// their own, and choosing between them wants a mobile's *history* — where it
/// was on the previous packet — which `WorldView` deliberately does not keep.
/// See the backlog in `docs/client.md`.
const STANDING: u8 = 4;

/// One of the server's mobiles, as the renderer wants it.
///
/// The frame is left at zero: the clock picks it in [`App::draw`], from however
/// many the atlas turned out to hold.
fn as_mobile(at: Point, body: openshard_protocol::wire::Graphic, facing: Facing, hue: Hue) -> Mobile {
    Mobile {
        at,
        body: body.0,
        group: STANDING,
        facing: facing.direction,
        frame: 0,
        hue,
    }
}

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

/// The three atlases one camera needs, packed together.
///
/// One value rather than a tuple because they are rebuilt together and used
/// together: a frame drawn from a land atlas of one camera and a static atlas
/// of another is a frame with things standing on ground that is not there.
struct Atlases {
    land: LandAtlas,
    texmaps: TexmapAtlas,
    statics: StaticAtlas,
    mobiles: AnimAtlas,
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
    /// The graphics currently packed. Rebuilt when the camera moves somewhere
    /// the atlas does not cover.
    atlas: LandAtlas,
    /// Their textures, packed the same way and rebuilt with them. The two are
    /// always built from the same set of graphics, so one of them missing a
    /// tile is the other one's business too.
    texmap_atlas: TexmapAtlas,
    /// The static sprites currently packed, rebuilt on the same trigger.
    static_atlas: StaticAtlas,
    /// The pass that draws the mobiles, which is the statics pass again with
    /// another atlas bound: a sprite is a sprite, and the two differ only in
    /// where the quad goes.
    mobile_pass: SpriteRenderer,
    /// The animation frames currently packed.
    mobile_atlas: AnimAtlas,
}

struct App {
    map: Map,
    art: Art,
    texmaps: TexMaps,
    tiledata: TileData,
    /// Every hue the client ships, packed once: unlike the sprite atlases it
    /// tints, nothing about it depends on where the camera is standing.
    hue_ramp: HueRamp,
    /// The animations, open but not read: `anim.mul` is 195MB and frames come
    /// out of it a body at a time. `&mut` because reading one seeks the file.
    anim: Anim,
    camera: Camera,
    /// Whether the camera is following the body or the mouse.
    follow: Follow,
    /// What the mouse is doing to it, and the fraction of a world pixel a drag
    /// has not yet paid for.
    drag: Drag,
    /// How far the ladder may be walked down before the offscreen texture is
    /// larger than the GPU allows, and whether that has been said out loud.
    ///
    /// WebGL2 guarantees only 2048 in each dimension and a 1080p window at
    /// `1/2` wants more, so the ladder has a runtime end that depends on the
    /// device. A silently truncated target draws a smaller world into a larger
    /// rect, which looks exactly like a bug in the projection — so it is
    /// reported instead, once.
    max_texture: u32,
    zoom_limit_reported: bool,
    /// This client's own body.
    ///
    /// Connected, it is what the server says: `0x1B` puts it somewhere and
    /// every ack, `0x20` and `0x21` moves it. Offline it is a placeholder
    /// standing wherever the camera looks, which is enough to hold the
    /// animation reader, the frame atlas and the placement against a real
    /// install.
    player: Mobile,
    /// Everyone else on screen, as `0x77` and `0x78` last described them.
    ///
    /// Empty offline, and rebuilt whole from the [`WorldView`] on every update:
    /// the view is the record of what arrived and this is a projection of it,
    /// so there is nothing here to keep in step by hand.
    others: Vec<Mobile>,
    /// The last thing the server said, whole.
    ///
    /// Kept only for the HUD's world window, which lists what has been decoded
    /// and is otherwise invisible — the ground items in particular, which
    /// nothing draws yet. The renderer reads [`App::player`] and
    /// [`App::others`], which are projections of this.
    view: Option<Box<WorldView>>,
    /// What the connection is doing, for the status strip.
    connection: String,
    /// How long the last frame took. Wall clock, like the animation.
    frame_time: std::time::Duration,
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
    /// How long the player's body animation has played.
    ///
    /// Real time, not the world tick — there is no world here to tick, and a
    /// real client's own body animation is a wall-clock timer too: see
    /// [`openshard_client_render::animation`].
    animation: AnimationClock,
    /// When the clock next advances a frame.
    next_tick: Instant,
    window: Option<Screen>,
}

impl ApplicationHandler<link::Update> for App {
    /// The shard thread had something to say.
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, update: link::Update) {
        match update {
            link::Update::World(view) => self.entered(&view),
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
                    self.camera.width = window.config.width;
                    self.camera.height = window.config.height;
                    // The world texture and the depth buffer follow the
                    // *camera's* size and not the window's, which are the same
                    // thing only at zoom 1. `draw` resizes them together.
                    window.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
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
                    KeyCode::PageUp => self.pan(0, PAGE_PIXELS),
                    KeyCode::PageDown => self.pan(0, -PAGE_PIXELS),
                    _ => self.step(code),
                };
                if changed {
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
                let (dx, dy) = (x - self.drag.cursor.0, y - self.drag.cursor.1);
                self.drag.cursor = (x, y);
                if self.drag.panning && self.pan(dx, dy) {
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == winit::event::MouseButton::Middle {
                    self.drag.panning = state == ElementState::Pressed;
                    // Whatever a previous drag was saving up is not owed to
                    // this one.
                    self.drag.remainder = (0, 0);
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
    /// `animation::FRAME_DELAY` is what stands in for a real client's own
    /// `Mobile.ProcessAnimation` poll.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_tick {
            self.animation.advance(FRAME_DELAY);
            self.next_tick += FRAME_DELAY;
            // A stall longer than one frame — the window minimised, the
            // machine asleep — re-arms from now rather than queuing up a
            // burst of catch-up redraws for time nobody watched.
            if self.next_tick < now {
                self.next_tick = now;
            }
            if let Some(window) = self.window.as_ref() {
                window.window.request_redraw();
            }
        }
        // Two reasons for a frame, so two terms: the animation clock, and
        // whatever the UI is animating. The deadline is the earlier.
        let deadline = match self.shell.as_ref().map(shell::Shell::repaint_after) {
            Some(after) => self.next_tick.min(now + after),
            None => self.next_tick,
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
    }
}

impl App {
    /// Move the camera, answering whether anything changed.
    ///
    /// Movement is clamped to the map rather than wrapped: walking off the north
    /// edge in UO is impossible, and a camera that wrapped would draw a seam
    /// between two sides of the world.
    fn step(&mut self, code: KeyCode) -> bool {
        // Connected, the keyboard moves nothing: it asks. The body goes where
        // the `0x22` says it went, which is the whole point of the walk
        // handshake — a client that stepped locally and corrected later would
        // be predicting, and the prediction lives in `Walk` where it can be
        // rolled back. Page up and down are a camera control the protocol has
        // no packet for, so they do nothing here.
        if let Some(link) = self.link.as_ref() {
            let facing = match code {
                KeyCode::ArrowUp => Direction::NorthWest,
                KeyCode::ArrowDown => Direction::SouthEast,
                KeyCode::ArrowLeft => Direction::SouthWest,
                KeyCode::ArrowRight => Direction::NorthEast,
                _ => return false,
            };
            link.step(Facing::walking(facing));
            return false;
        }

        // The four arrows are the four diagonals of the map, which are the four
        // *straight* directions on screen: UO's grid is turned 45 degrees, so
        // "up" is north-west and there is no key that moves one axis alone.
        let facing = match code {
            KeyCode::ArrowUp => Some(Direction::NorthWest),
            KeyCode::ArrowDown => Some(Direction::SouthEast),
            KeyCode::ArrowLeft => Some(Direction::SouthWest),
            KeyCode::ArrowRight => Some(Direction::NorthEast),
            _ => None,
        };
        let Some(facing) = facing else {
            return false;
        };
        // Turning is a step here, as it is not in a real client: there is no
        // server to say whether the step happened, so the body faces wherever
        // it was last sent. `client/net`'s `walk` is what will decide this once
        // the two are joined.
        self.player.facing = facing;
        let (dx, dy) = facing.step();
        let x = (i32::from(self.player.at.x) + dx).clamp(0, self.map.width() as i32 - 1);
        let y = (i32::from(self.player.at.y) + dy).clamp(0, self.map.height() as i32 - 1);
        let (x, y) = (x as u16, y as u16);
        // On the *ground* there, not at some height of the camera's — a mobile
        // below the terrain is correctly hidden by it, which is what the depth
        // buffer is for and what looks exactly like a mobile that failed to
        // draw.
        let ground = self.map.land(x, y).map_or(self.player.at.z, |cell| cell.z);
        self.player.at = Point::new(x, y, ground);
        // Offline the body is what the camera is locked to, exactly as the
        // server's is when there is a server. Unlocked, walking still walks and
        // the body may leave the screen — walking and looking are different
        // questions, and `Home` is the answer to the second.
        if self.follow == Follow::Body {
            self.camera.look_at(self.player.at);
        }
        true
    }

    /// Put the eye back on the body and lock it there.
    ///
    /// Snaps rather than eases. Easing wants a per-frame clock over a mobile
    /// that survives between frames, which is what the "everything stands"
    /// backlog item in `docs/client.md` is waiting for; both should be built
    /// once.
    fn relock(&mut self) {
        self.follow = Follow::Body;
        self.camera.look_at(self.player.at);
    }

    /// Move the eye by a drag, in viewport pixels, spending whole world pixels.
    ///
    /// Answers whether the eye actually moved: at zoom 4 most one-pixel drags
    /// move nothing, and asking for a redraw for each of them would be a frame
    /// per mouse report showing the same picture.
    fn pan(&mut self, dx: i32, dy: i32) -> bool {
        self.follow = Follow::Free;
        let num = self.camera.zoom().numerator() as i32;
        let den = self.camera.zoom().denominator() as i32;
        // Viewport pixels times the denominator, kept as a numerator over
        // `num`: the fraction stays here rather than in the eye.
        let owed_x = self.drag.remainder.0 + dx * den;
        let owed_y = self.drag.remainder.1 + dy * den;
        // Towards zero, so the remainder keeps the sign of the debt and a drag
        // back and forth ends where it started.
        let (whole_x, whole_y) = (owed_x / num, owed_y / num);
        self.drag.remainder = (owed_x - whole_x * num, owed_y - whole_y * num);
        if whole_x == 0 && whole_y == 0 {
            return false;
        }
        let eye = self.camera.eye();
        // The world follows the cursor, so the eye goes the other way.
        self.camera.look_at_pixel(WorldPixel {
            x: eye.x - whole_x,
            y: eye.y - whole_y,
        });
        true
    }

    /// Step the zoom back in until the world texture fits this device.
    ///
    /// [`App::zoom`] refuses a step that would not fit, and that is not the
    /// whole of it: the offscreen image is `viewport / zoom`, so *growing the
    /// window* at a zoom that fitted asks for a texture that does not. Nobody
    /// zooms in that path, so the check has to live where the size is used and
    /// not only where the zoom changes — without this, dragging a window wider
    /// at `1/2` is a validation error from `world_texture` rather than a camera
    /// that stops zooming out.
    fn fit_zoom_to_device(&mut self) {
        while (self.camera.render_width() > self.max_texture
            || self.camera.render_height() > self.max_texture)
            && self.camera.zoom().scale_up() != self.camera.zoom()
        {
            let tighter = self.camera.zoom().scale_up();
            if !self.zoom_limit_reported {
                self.zoom_limit_reported = true;
                eprintln!(
                    "a {}x{} world texture is more than this GPU's {}: zooming in to {tighter}",
                    self.camera.render_width(),
                    self.camera.render_height(),
                    self.max_texture,
                );
            }
            // About the middle: this is not somebody's wheel, it is the device
            // saying no, and there is no cursor that asked for it.
            let (cx, cy) = (self.camera.width as i32 / 2, self.camera.height as i32 / 2);
            self.camera.zoom_about(cx, cy, tighter);
        }
    }

    /// One notch of the wheel, about the cursor.
    ///
    /// Answers whether anything changed: at either end of the ladder nothing
    /// does, and zooming out can be refused by the GPU — see
    /// [`App::max_texture`].
    fn zoom(&mut self, inwards: bool) -> bool {
        let wanted = if inwards {
            self.camera.zoom().scale_up()
        } else {
            self.camera.zoom().scale_down()
        };
        if wanted == self.camera.zoom() {
            return false;
        }
        // Locked to the body, the zoom is about the middle: an eye held to the
        // cursor would be moved here and moved back by the next `WorldView`,
        // which is a fight rather than a camera. Unlocked, it is about the
        // cursor, which is the difference between a camera that feels placed
        // and one that feels shoved.
        let (anchor_x, anchor_y) = match self.follow {
            Follow::Body => (self.camera.width as i32 / 2, self.camera.height as i32 / 2),
            Follow::Free => self.drag.cursor,
        };
        // The offscreen image the new zoom would want, against what this device
        // can allocate. Refused rather than truncated: a smaller world drawn
        // into a larger rect looks like a projection bug and reads as one.
        let mut probe = self.camera;
        probe.zoom_about(anchor_x, anchor_y, wanted);
        if probe.render_width() > self.max_texture || probe.render_height() > self.max_texture {
            if !self.zoom_limit_reported {
                self.zoom_limit_reported = true;
                eprintln!(
                    "{wanted} would want a {}x{} world texture and this GPU allows {}: staying at {}",
                    probe.render_width(),
                    probe.render_height(),
                    self.max_texture,
                    self.camera.zoom(),
                );
            }
            return false;
        }
        self.camera = probe;
        // A zoom about the cursor moves the eye, so it is a manual camera move
        // like any other. Zooming while locked and staying locked would fight
        // the next `WorldView`.
        // The fraction a drag was saving up belongs to the old zoom.
        self.drag.remainder = (0, 0);
        true
    }

    /// Redraw from what the server has shown us.
    ///
    /// A projection of the whole [`WorldView`], rebuilt each time rather than
    /// patched: the view is the record of what arrived, and anything kept in
    /// step with it by hand would be a second record that could disagree.
    fn entered(&mut self, view: &WorldView) {
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

        self.player = as_mobile(
            view.player.position,
            view.player.body,
            view.player.facing,
            view.player.hue,
        );
        // Sorted by serial: a `HashMap`'s order is not one, and an atlas built
        // in a different order every frame is a rebuild every frame.
        let mut others: Vec<_> = view.mobiles.iter().collect();
        others.sort_unstable_by_key(|(serial, _)| serial.raw());
        self.others = others
            .into_iter()
            .map(|(_, mobile)| as_mobile(mobile.position, mobile.body, mobile.facing, mobile.hue))
            .collect();
        // The camera follows the body, which is what `0x20` is for — unless it
        // has been unlocked, in which case the eye is the mouse's and the body
        // is free to walk off the screen. `Home` puts it back.
        if self.follow == Follow::Body {
            self.camera.look_at(view.player.position);
        }
        self.connection = format!("in world as 0x{:08X}", view.player.serial.raw());
        // Whole, for the HUD's world window: the two projections above are what
        // the renderer wants, and the ground items are in neither of them.
        self.view = Some(Box::new(view.clone()));
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
            connection: self.connection.clone(),
            serial: self.view.as_ref().map(|view| view.player.serial.raw()),
            position: self.player.at,
            frame_time: self.frame_time,
            camera: self.camera,
            locked: self.follow == Follow::Body,
            mobiles,
            items,
        }
    }

    /// The player and everyone else, in one slice for the atlas and the pass.
    fn drawn_mobiles(&self) -> Vec<Mobile> {
        let mut mobiles = Vec::with_capacity(self.others.len() + 1);
        mobiles.push(self.player);
        mobiles.extend_from_slice(&self.others);
        mobiles
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> Result<Screen, StartupError> {
        let attributes = Window::default_attributes()
            .with_title("OpenShard")
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.camera.width,
                self.camera.height,
            ));
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
            present_mode: capabilities.present_modes[0],
            desired_maximum_frame_latency: 2,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        // How far the zoom may be walked out. Asked once, because it is a
        // property of the device and not of the frame.
        self.max_texture = device.limits().max_texture_dimension_2d;
        self.camera.width = config.width;
        self.camera.height = config.height;

        let atlases = self.build_atlases()?;
        let renderer = GroundRenderer::new(&device, &queue, format, &atlases.land, &atlases.texmaps);
        let statics = SpriteRenderer::new(&device, &queue, format, atlases.statics.pixels(), &self.hue_ramp);
        let mobile_pass =
            SpriteRenderer::new(&device, &queue, format, atlases.mobiles.pixels(), &self.hue_ramp);
        // The world is drawn at 1:1 into a texture of the camera's render size,
        // which is the viewport only at zoom 1 — see `client/render`'s `blit`.
        let world = blit::world_texture(&device, self.camera.render_width(), self.camera.render_height());
        let depth = renderer::depth_texture(&device, self.camera.render_width(), self.camera.render_height());
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
            atlas: atlases.land,
            texmap_atlas: atlases.texmaps,
            static_atlas: atlases.statics,
            mobile_pass,
            mobile_atlas: atlases.mobiles,
        })
    }

    /// Pack what the camera can see: the flat land art, the textures its slopes
    /// are stretched over, and the sprites of everything standing on it.
    ///
    /// One set of land graphics feeds the first two, which is what lets a quad
    /// ask them the same question — and what makes "the atlas does not cover
    /// this" one decision rather than two that could disagree. The statics are
    /// a different index space and therefore a set of their own.
    fn build_atlases(&mut self) -> Result<Atlases, StartupError> {
        let wanted = ground::visible_graphics(&self.map, &self.camera);
        let land = LandAtlas::build(&self.art, wanted.iter().copied()).map_err(StartupError::Atlas)?;
        let texmaps =
            TexmapAtlas::build(&self.texmaps, &self.tiledata, wanted).map_err(StartupError::Atlas)?;
        let statics = StaticAtlas::build(&self.art, statics::visible_graphics(&self.map, &self.camera))
            .map_err(StartupError::Atlas)?;
        // Every animation the mobiles on screen need. `&mut` because the
        // frames are read from the file rather than held in memory.
        let wanted = mobiles::needed_animations(&self.drawn_mobiles());
        let mobiles = AnimAtlas::build(&mut self.anim, wanted).map_err(StartupError::Atlas)?;
        Ok(Atlases {
            land,
            texmaps,
            statics,
            mobiles,
        })
    }

    fn draw(&mut self) {
        let started = Instant::now();

        // The UI first, because what it leaves free is the world's viewport and
        // therefore the size of everything below. A frame that laid its panels
        // out after drawing the world would size the world from the previous
        // frame's panels — which is right until the first resize.
        // Gathered before the shell is borrowed: the HUD is a projection of the
        // whole app and the shell is part of it.
        let hud = self.hud();
        let painting = self.window.as_ref().map(|screen| Arc::clone(&screen.window));
        let ui = match (self.shell.as_mut(), painting.as_ref()) {
            (Some(shell), Some(window)) => {
                let (request, output) = shell.run(window, &hud);
                let viewport = shell.viewport();
                Some((request, output, viewport))
            }
            _ => None,
        };
        if let Some((request, _, viewport)) = &ui {
            if request.relock {
                self.relock();
            } else if request.unlock {
                self.follow = Follow::Free;
            }
            self.camera.width = viewport.width.max(1);
            self.camera.height = viewport.height.max(1);
        }
        // A viewport that grew may have taken the world texture past what the
        // device allows, which no zoom step asked for.
        self.fit_zoom_to_device();

        // The atlases are built for what was visible when they were built, so a
        // camera that has walked far enough will ask for a graphic they do not
        // hold. Rebuilding whenever that happens is the simplest correct answer,
        // and the ground is not what makes it expensive — statics will be, and
        // that is when this deserves an eviction policy instead.
        //
        // Repacked before the window is borrowed, and not inside the borrow: the
        // pack reads the whole of `self`, and the window is part of it.
        let wanted = ground::visible_graphics(&self.map, &self.camera);
        let wanted_statics = statics::visible_graphics(&self.map, &self.camera);
        let mut drawn = self.drawn_mobiles();
        let stale = self.window.as_ref().is_some_and(|window| {
            wanted
                .iter()
                .any(|graphic| window.atlas.region(*graphic).is_none())
                || wanted_statics
                    .iter()
                    .any(|graphic| window.static_atlas.sprite(*graphic).is_none())
                // The atlas holds one stored direction of one body at a time,
                // packed for whoever was on screen facing whichever way when it
                // was last built. A mobile turning, or a new one arriving, is
                // the same miss as walking off the land atlas.
                || drawn.iter().any(|mobile| {
                    let (direction, _) = openshard_uofiles::anim::facing(mobile.facing);
                    window
                        .mobile_atlas
                        .frame_count(mobile.body, mobile.group, direction)
                        == 0
                })
        });
        let repacked = stale.then(|| self.build_atlases());

        let Some(window) = self.window.as_mut() else {
            return;
        };
        match repacked {
            Some(Ok(atlases)) => {
                window.renderer = GroundRenderer::new(
                    &window.device,
                    &window.queue,
                    window.config.format,
                    &atlases.land,
                    &atlases.texmaps,
                );
                window.statics = SpriteRenderer::new(
                    &window.device,
                    &window.queue,
                    window.config.format,
                    atlases.statics.pixels(),
                    &self.hue_ramp,
                );
                window.mobile_pass = SpriteRenderer::new(
                    &window.device,
                    &window.queue,
                    window.config.format,
                    atlases.mobiles.pixels(),
                    &self.hue_ramp,
                );
                window.atlas = atlases.land;
                window.texmap_atlas = atlases.texmaps;
                window.static_atlas = atlases.statics;
                window.mobile_atlas = atlases.mobiles;
            }
            Some(Err(error)) => eprintln!("repacking art: {error}"),
            None => {}
        }

        // The clock picks the frame from how many the atlas actually packed —
        // asking the atlas rather than remembering the count is what keeps
        // "frame 7 of a 6-frame walk" from being expressible. One clock for
        // everybody: a standing crowd animating in step is wrong and looks it,
        // and fixing it wants a clock per mobile, which wants a mobile that
        // survives between frames. See the backlog.
        for mobile in &mut drawn {
            let (direction, _) = openshard_uofiles::anim::facing(mobile.facing);
            let frame_count = window
                .mobile_atlas
                .frame_count(mobile.body, mobile.group, direction);
            mobile.frame = self.animation.frame(frame_count);
        }

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
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // The image the world is drawn into. Its size is the camera's, so a
        // resize and a zoom step are the same event here — and recreating it is
        // the only thing either of them costs.
        let (render_width, render_height) = (self.camera.render_width(), self.camera.render_height());
        if window.world.width() != render_width || window.world.height() != render_height {
            window.world = blit::world_texture(&window.device, render_width, render_height);
            // Tested pixel for pixel against that image, so it is exactly its
            // size or it is nothing.
            window.depth = renderer::depth_texture(&window.device, render_width, render_height);
        }
        let world_view = window.world.create_view(&wgpu::TextureViewDescriptor::default());

        let quads = ground::collect(&self.map, &self.camera, &window.atlas, &window.texmap_atlas);
        let static_quads = statics::collect(&self.map, &self.camera, &self.tiledata, &window.static_atlas);
        let mobile_quads = mobiles::collect(&drawn, &self.camera, &window.mobile_atlas);
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
            self.camera.zoom(),
            viewport,
        );
        // The UI over it, with no depth attachment: the world's depth buffer
        // ordered the world, and this is drawn on the result.
        if let (Some(shell), Some((_, output, _))) = (self.shell.as_mut(), ui) {
            shell.paint(
                &window.device,
                &window.queue,
                &mut encoder,
                &view,
                output,
                [window.config.width, window.config.height],
            );
        }
        window.queue.submit([encoder.finish()]);
        // Presentation moved onto the queue in wgpu 30; the texture is consumed.
        window.queue.present(frame);
        self.frame_time = started.elapsed();
    }
}
