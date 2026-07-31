//! The client, as far as it goes: a window onto Britannia's ground.
//!
//! Run it against a real client install, which this repository never contains:
//!
//! ```sh
//! OPENSHARD_CLIENT="/path/to/Ultima Online Classic" cargo run -p openshard-client-app
//! ```
//!
//! Arrow keys walk a tile at a time, page up and down change the camera's
//! height, and escape closes it. There is no connection yet: this draws the
//! map's own ground and statics, not a world a server has shown us, and the one
//! mobile on screen is a body standing where the camera looks rather than a
//! character anybody logged in as. `WorldView` arrives when `client/net` and
//! `client/render` are joined, and the camera then follows `0x20` instead of
//! the keyboard.

use std::fmt;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use openshard_client_render::atlas::{AnimAtlas, LandAtlas, StaticAtlas, TexmapAtlas};
use openshard_client_render::camera::Camera;
use openshard_client_render::mobiles::{self, Mobile};
use openshard_client_render::renderer::{self, GroundRenderer, SpriteRenderer, Target};
use openshard_client_render::{ground, statics};
use openshard_protocol::direction::Direction;
use openshard_protocol::world::Point;
use openshard_uofiles::anim::Anim;
use openshard_uofiles::art::Art;
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

/// The facet to open. Felucca until there is a server to say otherwise.
const FACET: u8 = 0;

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
    eprintln!(
        "{} loaded: {}x{} tiles",
        map.facet_name(),
        map.width(),
        map.height()
    );

    let event_loop = match EventLoop::new() {
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

    let mut app = App {
        map,
        art,
        texmaps,
        tiledata,
        anim,
        camera: Camera::new(START, 1024, 768),
        player: Mobile {
            at: start,
            // 400 is the male human body and 4 is its standing group; frame 0
            // because nothing here advances an animation yet.
            body: 400,
            group: 4,
            facing: Direction::SouthEast,
            frame: 0,
        },
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
    /// The depth buffer the two passes share, which is what decides whether a
    /// hillside covers the wall behind it. Recreated on resize: it has to be
    /// exactly the size of the frame it is tested against.
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
    /// The animations, open but not read: `anim.mul` is 195MB and frames come
    /// out of it a body at a time. `&mut` because reading one seeks the file.
    anim: Anim,
    camera: Camera,
    /// The one mobile there is, until a server sends some.
    ///
    /// A real client draws what `0x77` and `0x78` told it about. This binary has
    /// no connection, so it draws the character the camera is standing on — which
    /// is enough to hold the animation reader, the frame atlas and the placement
    /// against a real install, and is honest about being a placeholder.
    player: Mobile,
    window: Option<Screen>,
}

impl ApplicationHandler for App {
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
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(window) = self.window.as_mut() {
                    window.config.width = size.width.max(1);
                    window.config.height = size.height.max(1);
                    window.surface.configure(&window.device, &window.config);
                    // The depth buffer is tested pixel for pixel against the
                    // frame, so it is the frame's size or it is nothing.
                    window.depth =
                        renderer::depth_texture(&window.device, window.config.width, window.config.height);
                    self.camera.width = window.config.width;
                    self.camera.height = window.config.height;
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
                if self.step(code) {
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                }
            }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }
}

impl App {
    /// Move the camera, answering whether anything changed.
    ///
    /// Movement is clamped to the map rather than wrapped: walking off the north
    /// edge in UO is impossible, and a camera that wrapped would draw a seam
    /// between two sides of the world.
    fn step(&mut self, code: KeyCode) -> bool {
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
        let (dx, dy, dz) = match (facing, code) {
            (Some(facing), _) => {
                let (dx, dy) = facing.step();
                (dx, dy, 0)
            }
            (None, KeyCode::PageUp) => (0, 0, 5),
            (None, KeyCode::PageDown) => (0, 0, -5),
            _ => return false,
        };
        if let Some(facing) = facing {
            // Turning is a step here, as it is not in a real client: there is
            // no server to say whether the step happened, so the body faces
            // wherever it was last sent. `client/net`'s `walk` is what will
            // decide this once the two are joined.
            self.player.facing = facing;
        }
        let x = (i32::from(self.camera.center.x) + dx).clamp(0, self.map.width() as i32 - 1);
        let y = (i32::from(self.camera.center.y) + dy).clamp(0, self.map.height() as i32 - 1);
        let z = (i32::from(self.camera.center.z) + dz).clamp(i8::MIN.into(), i8::MAX.into());
        self.camera.center = Point::new(x as u16, y as u16, z as i8);
        // The player stands where the camera looks, until a server says
        // otherwise: this binary has no connection yet. On the *ground* there,
        // not at the camera's height — a mobile below the terrain is correctly
        // hidden by it, which is what the depth buffer is for and what looks
        // exactly like a mobile that failed to draw.
        let ground = self
            .map
            .land(self.camera.center.x, self.camera.center.y)
            .map_or(self.camera.center.z, |cell| cell.z);
        self.player.at = Point::new(self.camera.center.x, self.camera.center.y, ground);
        true
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

        let atlases = self.build_atlases()?;
        let renderer = GroundRenderer::new(&device, &queue, format, &atlases.land, &atlases.texmaps);
        let statics = SpriteRenderer::new(&device, &queue, format, atlases.statics.pixels());
        let mobile_pass = SpriteRenderer::new(&device, &queue, format, atlases.mobiles.pixels());
        let depth = renderer::depth_texture(&device, config.width, config.height);

        Ok(Screen {
            window,
            surface,
            device,
            queue,
            config,
            renderer,
            statics,
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
        let wanted = mobiles::needed_animations(std::slice::from_ref(&self.player));
        let mobiles = AnimAtlas::build(&mut self.anim, wanted).map_err(StartupError::Atlas)?;
        Ok(Atlases {
            land,
            texmaps,
            statics,
            mobiles,
        })
    }

    fn draw(&mut self) {
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
        let stale = self.window.as_ref().is_some_and(|window| {
            wanted
                .iter()
                .any(|graphic| window.atlas.region(*graphic).is_none())
                || wanted_statics
                    .iter()
                    .any(|graphic| window.static_atlas.sprite(*graphic).is_none())
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
                );
                window.mobile_pass = SpriteRenderer::new(
                    &window.device,
                    &window.queue,
                    window.config.format,
                    atlases.mobiles.pixels(),
                );
                window.atlas = atlases.land;
                window.texmap_atlas = atlases.texmaps;
                window.static_atlas = atlases.statics;
                window.mobile_atlas = atlases.mobiles;
            }
            Some(Err(error)) => eprintln!("repacking art: {error}"),
            None => {}
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

        let quads = ground::collect(&self.map, &self.camera, &window.atlas, &window.texmap_atlas);
        let static_quads = statics::collect(&self.map, &self.camera, &self.tiledata, &window.static_atlas);
        let depth_view = window.depth.create_view(&wgpu::TextureViewDescriptor::default());
        let target = Target {
            view: &view,
            depth: &depth_view,
            width: window.config.width,
            height: window.config.height,
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
        window.queue.submit([encoder.finish()]);
        // Presentation moved onto the queue in wgpu 30; the texture is consumed.
        window.queue.present(frame);
    }
}
