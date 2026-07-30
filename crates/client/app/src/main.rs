//! The client, as far as it goes: a window onto Britannia's ground.
//!
//! Run it against a real client install, which this repository never contains:
//!
//! ```sh
//! OPENSHARD_CLIENT="/path/to/Ultima Online Classic" cargo run -p openshard-client-app
//! ```
//!
//! Arrow keys walk the camera a tile at a time, page up and down change its
//! height, and escape closes it. There is no connection yet: this draws the
//! map's own ground, not a world a server has shown us. `WorldView` arrives when
//! `client/net` and `client/render` are joined, and the camera then follows
//! `0x20` instead of the keyboard.

use std::fmt;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use openshard_client_render::atlas::LandAtlas;
use openshard_client_render::camera::Camera;
use openshard_client_render::ground;
use openshard_client_render::renderer::{GroundRenderer, Target};
use openshard_protocol::world::Point;
use openshard_uofiles::art::Art;
use openshard_uofiles::map::Map;
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

    let mut app = App {
        map,
        art,
        camera: Camera::new(START, 1024, 768),
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

/// Everything a window needs, built once the window exists.
struct Screen {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: GroundRenderer,
    /// The graphics currently packed. Rebuilt when the camera moves somewhere
    /// the atlas does not cover.
    atlas: LandAtlas,
}

struct App {
    map: Map,
    art: Art,
    camera: Camera,
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
        let (dx, dy, dz) = match code {
            KeyCode::ArrowUp => (-1, -1, 0),
            KeyCode::ArrowDown => (1, 1, 0),
            KeyCode::ArrowLeft => (-1, 1, 0),
            KeyCode::ArrowRight => (1, -1, 0),
            KeyCode::PageUp => (0, 0, 5),
            KeyCode::PageDown => (0, 0, -5),
            _ => return false,
        };
        let x = (i32::from(self.camera.center.x) + dx).clamp(0, self.map.width() as i32 - 1);
        let y = (i32::from(self.camera.center.y) + dy).clamp(0, self.map.height() as i32 - 1);
        let z = (i32::from(self.camera.center.z) + dz).clamp(i8::MIN.into(), i8::MAX.into());
        self.camera.center = Point::new(x as u16, y as u16, z as i8);
        true
    }

    fn create_window(&self, event_loop: &ActiveEventLoop) -> Result<Screen, StartupError> {
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

        let atlas = self.build_atlas()?;
        let renderer = GroundRenderer::new(&device, &queue, format, &atlas);

        Ok(Screen {
            window,
            surface,
            device,
            queue,
            config,
            renderer,
            atlas,
        })
    }

    fn build_atlas(&self) -> Result<LandAtlas, StartupError> {
        LandAtlas::build(&self.art, ground::visible_graphics(&self.map, &self.camera))
            .map_err(StartupError::Atlas)
    }

    fn draw(&mut self) {
        let Some(window) = self.window.as_mut() else {
            return;
        };

        // The atlas is built for what was visible when it was built, so a camera
        // that has walked far enough will ask for a graphic it does not hold.
        // Rebuilding whenever that happens is the simplest correct answer, and
        // the ground is not what makes it expensive — statics will be, and that
        // is when this deserves an eviction policy instead.
        let wanted = ground::visible_graphics(&self.map, &self.camera);
        if wanted
            .iter()
            .any(|graphic| window.atlas.region(*graphic).is_none())
        {
            match LandAtlas::build(&self.art, wanted) {
                Ok(atlas) => {
                    window.renderer =
                        GroundRenderer::new(&window.device, &window.queue, window.config.format, &atlas);
                    window.atlas = atlas;
                }
                Err(error) => eprintln!("repacking land art: {error}"),
            }
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

        let quads = ground::collect(&self.map, &self.camera, &window.atlas);
        let mut encoder = window
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        window.renderer.render(
            &window.device,
            &window.queue,
            &mut encoder,
            Target {
                view: &view,
                width: window.config.width,
                height: window.config.height,
            },
            &quads,
        );
        window.queue.submit([encoder.finish()]);
        // Presentation moved onto the queue in wgpu 30; the texture is consumed.
        window.queue.present(frame);
    }
}
