//! The dev HUD: egui panels over the world.
//!
//! # What this is, and what it is deliberately not
//!
//! It is a *dev* HUD. Whether this client's real interface is egui or the
//! `0xB0` gump layer is M4's decision, and building a journal or a paperdoll
//! here would take that decision by accident — so what is here is what tells you
//! whether the client is working: the connection, the camera, and the contents
//! of the [`WorldView`](openshard_client_net::view::WorldView), which are
//! decoded and otherwise invisible.
//!
//! Nothing here reaches into `client/net` or the view beyond reading them. What
//! the panels display arrives as a [`Hud`], built by the caller, and what they
//! ask for goes back as a [`Request`] — so the panels cannot move a camera or
//! send a packet, only say that somebody pressed something.
//!
//! # Four things that are silent when wrong
//!
//! Each of these is a mistake nobody reports as a bug, so each is written down
//! where it is made:
//!
//! 1. **Colour.** The surface is deliberately non-sRGB and egui's shader assumes
//!    an sRGB target unless it is told otherwise. It reads the format and picks
//!    the gamma entry point itself, which is why the format handed to
//!    [`egui_wgpu::Renderer::new`] has to be the surface's own — the usual
//!    symptom of getting it wrong is a UI that is merely *slightly* too bright.
//! 2. **Depth.** The UI pass takes no depth attachment. The world's depth buffer
//!    ordered the world; the UI is drawn over the result of that.
//! 3. **Input.** A consumed event must reach neither the camera nor the walk
//!    keys, or a drag inside a panel pans the world underneath it.
//!    [`Shell::on_window_event`] answers that question and its caller obeys.
//! 4. **Points against pixels.** egui lays out in logical points and the world
//!    is drawn in physical pixels, so the rect egui leaves free is multiplied by
//!    `pixels_per_point` before it becomes the camera's viewport. Getting this
//!    wrong is invisible at scale factor 1 and wrong on every HiDPI screen.

use openshard_client_render::blit::ViewportRect;
use openshard_client_render::camera::Camera;
use winit::window::Window;

/// What the panels are asked to display.
///
/// A snapshot built by the caller each frame rather than a borrow of the app:
/// the HUD is a projection of state it does not own, and this is the list of
/// what it is allowed to know.
pub struct Hud {
    /// The shard, if there is one, and what it is doing.
    pub connection: String,
    /// Our own serial, once a shard has given us one.
    pub serial: Option<u32>,
    /// Where our body stands, as the server last said.
    pub position: openshard_protocol::world::Point,
    /// How long the last frame took to build and submit.
    pub frame_time: std::time::Duration,
    /// The camera, read for its zoom, eye and viewport.
    pub camera: Camera,
    /// Whether the camera is locked to the body.
    pub locked: bool,
    /// Everyone else on screen: serial, body, position.
    pub mobiles: Vec<(u32, u16, openshard_protocol::world::Point)>,
    /// The ground items the view is holding: serial, graphic, position.
    pub items: Vec<(u32, u16, openshard_protocol::world::Point)>,
}

/// What the panels asked for this frame.
#[derive(Clone, Copy, Default, Debug)]
pub struct Request {
    /// Put the eye back on the body and lock it there.
    pub relock: bool,
    /// Let go of the body.
    pub unlock: bool,
}

/// egui, and the two crates that put it on a window and on a GPU.
pub struct Shell {
    context: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    /// Where the world may be drawn: what [`egui::CentralPanel`] left free,
    /// converted to physical pixels. Held between frames because the camera is
    /// resized from it before the next frame's UI has run.
    viewport: ViewportRect,
    /// What the last [`Shell::run`] asked to be woken after.
    repaint_after: std::time::Duration,
}

impl Shell {
    /// Build the HUD for a window and a surface format.
    ///
    /// `format` must be the surface's own: egui picks its fragment entry point
    /// from whether that format is sRGB, and a guess here is the "slightly too
    /// bright" failure in the module docs.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, window: &Window) -> Self {
        let context = egui::Context::default();
        let state = egui_winit::State::new(
            context.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let renderer = egui_wgpu::Renderer::new(
            device,
            format,
            egui_wgpu::RendererOptions {
                // No depth attachment: see the module docs.
                depth_stencil_format: None,
                ..Default::default()
            },
        );
        let size = window.inner_size();
        Self {
            context,
            state,
            renderer,
            viewport: ViewportRect {
                x: 0,
                y: 0,
                width: size.width.max(1),
                height: size.height.max(1),
            },
            // Until the first frame has run there is nothing to wait for; the
            // animation clock is what wakes the loop.
            repaint_after: std::time::Duration::MAX,
        }
    }

    /// Offer an event to the UI, answering whether it took it.
    ///
    /// A `true` here means the camera and the walk keys must not see the event.
    pub fn on_window_event(&mut self, window: &Window, event: &winit::event::WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    /// How long the UI is content to wait before it wants drawing again.
    ///
    /// An animating widget asks for the next frame soon and a still one asks
    /// for eternity, so the event loop's deadline is the earlier of this and the
    /// animation clock's — two terms, because they are two independent reasons
    /// for a frame.
    pub fn repaint_after(&self) -> std::time::Duration {
        self.repaint_after
    }

    /// The rectangle of the surface the world may be drawn into.
    ///
    /// Physical pixels, and not the window: a docked panel shrinks it, which is
    /// the same path a resize already takes.
    pub fn viewport(&self) -> ViewportRect {
        self.viewport
    }

    /// Lay the panels out, and hand back what they asked for.
    ///
    /// Splitting this from [`Shell::paint`] is what lets the camera be resized
    /// from the viewport this leaves *before* the world is drawn into it: a
    /// frame that laid out its UI after drawing the world would size the world
    /// from the previous frame's panels.
    pub fn run(&mut self, window: &Window, hud: &Hud) -> (Request, egui::FullOutput) {
        let input = self.state.take_egui_input(window);
        let mut request = Request::default();
        // What the panels leave behind, taken from the root `Ui` *after* they
        // have claimed their edges. That rectangle is the world's viewport, so
        // a docked panel shrinks the world and a floating window sits over it.
        let mut free = egui::Rect::from_min_size(egui::Pos2::ZERO, self.context.content_rect().size());
        let output = self.context.run_ui(input, |ui| {
            request = layout(ui, hud);
            free = ui.available_rect_before_wrap();
        });
        self.state
            .handle_platform_output(window, output.platform_output.clone());
        self.repaint_after = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(std::time::Duration::MAX, |viewport| viewport.repaint_delay);

        // Points to physical pixels: the one conversion that is invisible at
        // scale factor 1 and wrong on every HiDPI screen.
        let scale = self.context.pixels_per_point();
        let size = window.inner_size();
        let clamp = |value: f32, limit: u32| (value.max(0.0) as u32).min(limit);
        let x = clamp(free.min.x * scale, size.width);
        let y = clamp(free.min.y * scale, size.height);
        self.viewport = ViewportRect {
            x,
            y,
            width: clamp(free.width() * scale, size.width - x),
            height: clamp(free.height() * scale, size.height - y),
        };
        (request, output)
    }

    /// Draw what [`Shell::run`] produced, over whatever is already on the
    /// surface.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        output: egui::FullOutput,
        size_in_pixels: [u32; 2],
    ) {
        let pixels_per_point = self.context.pixels_per_point();
        let jobs = self.context.tessellate(output.shapes, pixels_per_point);
        for (id, delta) in &output.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, delta);
        }
        let descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels,
            pixels_per_point,
        };
        self.renderer
            .update_buffers(device, queue, encoder, &jobs, &descriptor);

        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("egui"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Over the world, not instead of it.
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        self.renderer
            .render(&mut pass.forget_lifetime(), &jobs, &descriptor);

        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}

/// The three panels, and nothing else.
///
/// Deliberately absent: the journal, the paperdoll, containers. Those are M4 —
/// see `docs/client.md` — and building them here would decide M4 without
/// arguing it.
fn layout(root: &mut egui::Ui, hud: &Hud) -> Request {
    let mut request = Request::default();
    // egui 0.35 hands the frame a root `Ui`: panels are shown inside it and
    // what is left of it is the world's viewport, while windows float over the
    // context. The two are laid out here in that order for exactly that reason.
    let context = root.ctx().clone();

    egui::Panel::top("status").show(root, |ui| {
        ui.horizontal(|ui| {
            ui.label(&hud.connection);
            ui.separator();
            match hud.serial {
                Some(serial) => ui.label(format!("serial 0x{serial:08X}")),
                None => ui.label("no serial"),
            };
            ui.separator();
            ui.label(format!(
                "{}, {}, {}",
                hud.position.x, hud.position.y, hud.position.z
            ));
            ui.separator();
            // Milliseconds with one decimal: a frame is a millisecond or two
            // here and an integer would read as zero.
            ui.label(format!("{:.1} ms", hud.frame_time.as_secs_f64() * 1_000.0));
        });
    });

    egui::Window::new("Camera")
        .default_pos([16.0, 48.0])
        .show(&context, |ui| {
            let eye = hud.camera.eye();
            egui::Grid::new("camera").num_columns(2).show(ui, |ui| {
                ui.label("zoom");
                ui.label(hud.camera.zoom().to_string());
                ui.end_row();
                ui.label("eye");
                ui.label(format!("{}, {} px", eye.x, eye.y));
                ui.end_row();
                ui.label("tile");
                let (x, y) = hud.camera.eye_tile();
                ui.label(format!("{x}, {y}"));
                ui.end_row();
                ui.label("viewport");
                ui.label(format!("{}x{}", hud.camera.width, hud.camera.height));
                ui.end_row();
                ui.label("drawn");
                // The offscreen image, which is the viewport only at zoom 1 and
                // is what the GPU's texture limit applies to.
                ui.label(format!(
                    "{}x{}",
                    hud.camera.render_width(),
                    hud.camera.render_height()
                ));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                // The lock is state the player can otherwise only infer from
                // the camera not moving, which is why it is shown as well as
                // toggled.
                let mut locked = hud.locked;
                if ui.checkbox(&mut locked, "follow the body").changed() {
                    request.relock = locked;
                    request.unlock = !locked;
                }
                if ui.button("return (Home)").clicked() {
                    request.relock = true;
                    request.unlock = false;
                }
            });
        });

    egui::Window::new("World")
        .default_pos([16.0, 240.0])
        .show(&context, |ui| {
            ui.label(format!(
                "{} mobiles, {} ground items",
                hud.mobiles.len(),
                hud.items.len()
            ));
            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                for (serial, body, at) in &hud.mobiles {
                    ui.label(format!(
                        "0x{serial:08X}  body {body}  {}, {}, {}",
                        at.x, at.y, at.z
                    ));
                }
                if !hud.items.is_empty() {
                    ui.separator();
                }
                for (serial, graphic, at) in &hud.items {
                    ui.label(format!(
                        "0x{serial:08X}  item {graphic}  {}, {}, {}",
                        at.x, at.y, at.z
                    ));
                }
            });
        });

    request
}
