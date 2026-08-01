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

use std::time::Duration;

use openshard_client_render::bench::{Metrics, Reading};
use openshard_client_render::blit::ViewportRect;
use openshard_client_render::camera::Camera;
use openshard_client_render::follow::Rig;
use openshard_uofiles::hues::Hues;
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
    /// The camera, read for its zoom, eye and viewport.
    pub camera: Camera,
    /// Whether the camera is locked to the body.
    pub locked: bool,
    /// What the eye is following with — every number a camera is made of.
    pub rig: Rig,
    /// The last few seconds of the eye, one entry per frame.
    ///
    /// Owned rather than borrowed because the HUD is a snapshot and not a view
    /// of the app; a few hundred `f64`s a frame is what that costs, and it is
    /// what keeps the panels unable to reach back into the camera.
    pub readings: Vec<Reading>,
    /// What those frames come to, and `None` before there are enough of them to
    /// difference. Absent rather than zeroed: a metric over one frame is not a
    /// small number, it is not a number.
    pub metrics: Option<Metrics>,
    /// How long a window the scope keeps, for the chart's own axis.
    pub scope_span: Duration,
    /// The last few seconds of the event loop, one entry per drawn frame.
    pub frames: Vec<crate::frames::Frame>,
    /// How long a window those cover, for that chart's own axis.
    pub frames_span: Duration,
    /// The worst frame rate in that window, and `None` before there is a frame
    /// to have a rate.
    pub worst_fps: Option<f64>,
    /// What is currently asking for frames.
    ///
    /// Shown beside the rate because it is the *reason* for it: a client paced
    /// by the display and one paced by the animation clock report the same kind
    /// of number and mean opposite things by it, and a panel that only showed
    /// the rate would read the second as a fault.
    pub pacing: crate::frames::Pacing,
    /// The bench's scenarios, by name, in the order it ships them.
    pub scripts: Vec<&'static str>,
    /// The one being replayed, and how far through it is from zero to one.
    pub replay: Option<(&'static str, f32)>,
    /// Whether there is no shard, which is the only state a replay may run in:
    /// connected, the body goes where the `0x22` says, and a second writer is
    /// two clients fighting over one character.
    pub offline: bool,
    /// Everyone else on screen: serial, body, position.
    pub mobiles: Vec<(u32, u16, openshard_protocol::world::Point)>,
    /// The ground items the view is holding: serial, graphic, position.
    pub items: Vec<(u32, u16, openshard_protocol::world::Point)>,
    /// What tile the cursor is over right now, if it is over the world and on
    /// the map. Live, and gone the instant the cursor leaves — see `selected`
    /// for what a click keeps.
    pub hover: Option<PickedTile>,
    /// The tile a left click last landed on. Kept until the next click, which
    /// is what makes its numbers holdable still long enough to copy — the
    /// live hover moves out from under the cursor the moment it does.
    pub selected: Option<PickedTile>,
    /// The tile the body is walking to, while it still is.
    ///
    /// The one piece of feedback a move order needs: a click that named a tile
    /// the shard then refuses to walk to looks exactly like a click that was
    /// never registered, and this is what tells the two apart.
    pub goal: Option<PickedTile>,
    /// The dialogs the server has open on this client, waiting to be answered.
    pub gumps: Vec<openshard_client_net::view::OpenGump>,
    /// The last few lines the shard has said, oldest first.
    ///
    /// Not the journal M4 will build — see [`layout`]'s docs. What it is for is
    /// that a system message has no mobile behind it, so it is drawn over
    /// nobody's head and a client with only overhead speech never shows it. A
    /// refused `.admin` says "you are not a game master" and nothing else, and
    /// without this strip that answer is invisible.
    pub said: Vec<String>,
}

/// A tile, read straight from the map — for telling a rendering artifact apart
/// from a gameplay one: is the graphic under a glitch the tile the client
/// thinks is there, or something else entirely?
#[derive(Clone)]
pub struct PickedTile {
    /// The tile coordinate, resolved from the cursor via [`Camera::pick`] and
    /// [`unproject`](openshard_client_render::camera::unproject).
    pub x: u16,
    /// The tile coordinate's other half.
    pub y: u16,
    /// The land tile's graphic id, if the block loaded.
    pub land: Option<u16>,
    /// The ground's height here.
    pub land_z: i8,
    /// Everything standing on top of the ground here: graphic id, height, hue.
    pub statics: Vec<(u16, i8, u16)>,
}

/// What the panels asked for this frame.
///
/// No longer `Copy`: two of these carry what the player typed. A request is
/// built fresh each frame and spent by the caller, so cloning it is not a thing
/// that happens on any path.
#[derive(Clone, Default, Debug)]
pub struct Request {
    /// Put the eye back on the body and lock it there.
    pub relock: bool,
    /// Let go of the body.
    pub unlock: bool,
    /// A line the player pressed Enter on. Sent as speech exactly as typed —
    /// a `.`-prefixed line is a staff command *on the server*, and a client that
    /// recognised its own would be deciding what a shard's commands are.
    pub say: Option<String>,
    /// A dialog the player answered. See [`crate::gump`].
    pub gump: Option<crate::link::GumpReply>,
    /// Follow with these numbers from now on.
    ///
    /// Sent on the frame a slider moved or a preset was clicked, and not every
    /// frame: the eye is not moved by a rig arriving, but a scope that cleared
    /// its trace on every frame would never have one to draw.
    pub rig: Option<Rig>,
    /// Start or stop a scripted walk.
    pub script: Option<ScriptRequest>,
    /// How long a window the scope should keep from now on.
    ///
    /// Four seconds holds a reversal and is wrong for both ends of the range a
    /// scenario can be: a `teleport` is over in one, and a `back_and_forth`
    /// worth reading is longer than the window that shows it.
    pub scope_span: Option<Duration>,
}

/// What the script picker asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScriptRequest {
    /// Walk this scenario from its start.
    Run(&'static str),
    /// Stop wherever it got to.
    Stop,
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
    /// What is in the chat line and not yet said. Lives here rather than in the
    /// app for the reason [`Windows`](crate::gump::Windows) does: it is what a
    /// widget is holding between frames, and nothing outside the UI reads it.
    typed: String,
    /// The state of the open dialogs — which page, which switches.
    gumps: crate::gump::Windows,
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
            typed: String::new(),
            gumps: crate::gump::Windows::default(),
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
    pub fn run(&mut self, window: &Window, hud: &Hud, hues: &Hues) -> (Request, egui::FullOutput) {
        let input = self.state.take_egui_input(window);
        let mut request = Request::default();
        // What the panels leave behind, taken from the root `Ui` *after* they
        // have claimed their edges. That rectangle is the world's viewport, so
        // a docked panel shrinks the world and a floating window sits over it.
        let mut free = egui::Rect::from_min_size(egui::Pos2::ZERO, self.context.content_rect().size());
        let typed = &mut self.typed;
        let gumps = &mut self.gumps;
        let output = self.context.run_ui(input, |ui| {
            request = layout(ui, hud, typed, gumps, hues);
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

/// The panels, the speech line, and the server's own dialogs.
///
/// Deliberately absent: the paperdoll, containers, and a journal worth the name.
/// Those are M4 — see `docs/client.md` — and building them here would decide M4
/// without arguing it. The speech line is not one of them: it is the only way to
/// reach a shard's staff commands, which are `.`-prefixed *speech*, and a client
/// that cannot say `.admin` cannot open the menu the server already draws.
fn layout(
    root: &mut egui::Ui,
    hud: &Hud,
    typed: &mut String,
    gumps: &mut crate::gump::Windows,
    hues: &Hues,
) -> Request {
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
            // What the frame cost to *build*, and not how long it took: paced by
            // the display, every frame takes a refresh interval whatever it was
            // doing, and the strip would read 16.7ms on an idle client for ever.
            // Milliseconds with one decimal, because a frame is a millisecond or
            // two here and an integer would read as zero.
            ui.label(format!(
                "{:.1} ms",
                hud.frames
                    .last()
                    .map_or(0.0, |frame| frame.build().as_secs_f64() * 1_000.0)
            ));
        });
    });

    // What the status panel above left free — the same rect `Shell::run` reads
    // back afterwards, since only a panel narrows it and no window does. Read
    // here rather than passed in, so this stays the one place that decides it.
    let viewport_origin = root.available_rect_before_wrap().min;
    if let Some(tile) = &hud.hover {
        draw_tile_highlight(
            root,
            &hud.camera,
            tile,
            viewport_origin,
            egui::Color32::from_rgba_unmultiplied(255, 255, 0, 40),
            egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(255, 255, 0, 180)),
        );
    }
    if let Some(tile) = &hud.selected {
        draw_tile_highlight(
            root,
            &hud.camera,
            tile,
            viewport_origin,
            egui::Color32::from_rgba_unmultiplied(0, 220, 255, 60),
            egui::Stroke::new(2.5, egui::Color32::from_rgb(0, 220, 255)),
        );
    }
    // Where the body is walking to, and gone the moment it arrives or gives up.
    if let Some(tile) = &hud.goal {
        draw_tile_highlight(
            root,
            &hud.camera,
            tile,
            viewport_origin,
            egui::Color32::from_rgba_unmultiplied(0, 255, 120, 50),
            egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 255, 120)),
        );
    }

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

    egui::Window::new("Rig")
        .default_pos([16.0, 200.0])
        .default_width(320.0)
        .show(&context, |ui| {
            rig_panel(ui, hud, &mut request);
        });

    egui::Window::new("Frames")
        .default_pos([16.0, 220.0])
        .default_width(320.0)
        .show(&context, |ui| {
            frames_panel(ui, hud);
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

    egui::Window::new("Tile")
        .default_pos([16.0, 520.0])
        .show(&context, |ui| {
            ui.label("hover — glows yellow, moves with the cursor");
            tile_panel(ui, hud.hover.as_ref());
            ui.separator();
            ui.label("selected — glows cyan, click a tile to hold it here");
            tile_panel(ui, hud.selected.as_ref());
        });

    request.say = speech_line(root, hud, typed);
    // Over everything, and last: a dialog the shard opened is the one thing on
    // screen that is waiting for an answer.
    request.gump = gumps.show(&context, &hud.gumps, hues);

    request
}

/// The scope: what the eye is doing, what it is doing it with, and a scenario
/// to make it do it.
///
/// `docs/camera.md`, C4. From here on every remaining decision about the camera
/// is a matter of looking rather than arguing, and this is what there is to look
/// at: a preset and a slider per number, the last few seconds of the eye's own
/// speed and jerk, the same [`Metrics`] the offline bench prints, and the bench's
/// scenarios walked by the real body.
///
/// The numbers and the curves come off one arithmetic — `bench::readings` — so a
/// figure that disagrees with the shape beside it means the metric is wrong,
/// which is a thing to be able to see rather than to reason about.
fn rig_panel(ui: &mut egui::Ui, hud: &Hud, request: &mut Request) {
    let mut rig = hud.rig;
    ui.horizontal(|ui| {
        ui.label("preset");
        // The two that exist, and neither is called `DEFAULT`: which camera
        // this client ships is decided on this panel, not in a name.
        if ui.button("HARD").clicked() {
            rig = Rig::HARD;
        }
        if ui.button("LIFT").clicked() {
            rig = Rig::LIFT;
        }
    });
    egui::Grid::new("rig").num_columns(2).show(ui, |ui| {
        ui.label("plane τ");
        ui.add(egui::Slider::new(&mut rig.plane_tau, 0.0..=0.5).suffix(" s"));
        ui.end_row();
        ui.label("lift τ");
        ui.add(egui::Slider::new(&mut rig.lift_tau, 0.0..=0.5).suffix(" s"));
        ui.end_row();
        ui.label("lift cut");
        ui.horizontal(|ui| {
            // Infinity is a real setting — it never cuts — and it is not a
            // point on a slider, so it is the checkbox and the slider holds
            // what the last finite value was.
            let mut cuts = rig.lift_cut.is_finite();
            let mut pixels = match rig.lift_cut.is_finite() {
                true => rig.lift_cut,
                false => openshard_client_render::follow::FLOOR,
            };
            ui.checkbox(&mut cuts, "");
            ui.add_enabled(cuts, egui::Slider::new(&mut pixels, 0.0..=256.0).suffix(" px"));
            rig.lift_cut = match cuts {
                true => pixels,
                false => f32::INFINITY,
            };
        });
        ui.end_row();
    });
    if rig != hud.rig {
        request.rig = Some(rig);
    }
    ui.horizontal(|ui| {
        // The whole point of the sliders: a setting that felt right is a value
        // that can be pasted into `follow.rs` and committed as the preset it
        // turned out to be.
        let literal = literal(&rig);
        ui.label(egui::RichText::new(&literal).monospace().small());
        if ui.small_button("copy").clicked() {
            ui.ctx().copy_text(literal);
        }
    });

    ui.horizontal(|ui| {
        ui.label("scope");
        let mut span = hud.scope_span.as_secs_f32();
        // Logarithmic: the useful settings are a second apart at one end and
        // several seconds apart at the other, and a linear slider spends most
        // of its length on the end nobody is looking at.
        if ui
            .add(
                egui::Slider::new(&mut span, 0.5..=20.0)
                    .logarithmic(true)
                    .suffix(" s"),
            )
            .changed()
        {
            request.scope_span = Some(Duration::from_secs_f32(span));
        }
    });

    ui.separator();
    match hud.metrics {
        Some(metrics) => {
            egui::Grid::new("metrics").num_columns(4).show(ui, |ui| {
                ui.label("lag");
                ui.label(format!("{:.1} px", metrics.lag_max));
                ui.label("speed");
                ui.label(format!("{:.0} px/s", metrics.speed_max));
                ui.end_row();
                ui.label("accel");
                ui.label(format!("{:.0}", metrics.accel_max));
                ui.label("jerk rms");
                ui.label(format!("{:.0}", metrics.jerk_rms));
                ui.end_row();
                ui.label("step σ²");
                ui.label(format!("{:.2}", metrics.step_var));
                // The two companions, and they are on the panel rather than in
                // a comment: a metric over a scene where nothing moved is
                // green and means nothing, and this repository has produced
                // that result before.
                ui.label("travel");
                ui.label(format!("{:.0} px / {} frames", metrics.travel, metrics.frames));
                ui.end_row();
            });
        }
        None => {
            ui.label("no frames yet");
        }
    }

    let span = hud.scope_span.as_secs_f32().max(0.001);
    let last = hud
        .readings
        .last()
        .map_or(0.0, |reading| reading.at.as_secs_f32());
    let series = |of: fn(&Reading) -> Option<f64>| -> Vec<(f32, f32)> {
        hud.readings
            .iter()
            .filter_map(|reading| {
                of(reading).map(|value| (reading.at.as_secs_f32() - (last - span), value as f32))
            })
            .collect()
    };
    strip(
        ui,
        "the eye's speed, px/s",
        &series(|reading| Some(reading.speed)),
        span,
        egui::Color32::from_rgb(80, 170, 255),
    );
    strip(
        ui,
        "jerk — what ragged is, as a number",
        &series(|reading| reading.jerk),
        span,
        egui::Color32::from_rgb(255, 140, 90),
    );

    ui.separator();
    match hud.replay {
        Some((name, progress)) => {
            ui.add(egui::ProgressBar::new(progress).text(name));
            if ui.button("stop").clicked() {
                request.script = Some(ScriptRequest::Stop);
            }
        }
        None => {
            ui.horizontal_wrapped(|ui| {
                for name in &hud.scripts {
                    if ui.add_enabled(hud.offline, egui::Button::new(*name)).clicked() {
                        request.script = Some(ScriptRequest::Run(name));
                    }
                }
            });
            if !hud.offline {
                ui.label(
                    egui::RichText::new(
                        "a scenario walks the body itself, so it needs a client with no shard",
                    )
                    .weak()
                    .small(),
                );
            }
        }
    }
}

/// The frame rate, what is setting it, and which half of the frame the time went
/// into.
///
/// A drop is either *cost* — the frame took too long to build — or *pacing*:
/// nothing asked for a frame sooner. Watched, this client is paced by the display
/// and a drop is a cost; unwatched it falls back to the animation clock on
/// purpose, and 12.5 frames a second there looks exactly like a stall and is not
/// one. So the pacer is printed beside the rate.
///
/// And the cost is two curves rather than one, because a frame is built by two
/// independent things: `egui` laying out the panels, and the world. The wait is
/// neither — it is the display holding the last frame — and it is the number
/// that says how much of the frame was still free. See [`crate::frames`].
fn frames_panel(ui: &mut egui::Ui, hud: &Hud) {
    let ms = |duration: Duration| duration.as_secs_f64() * 1_000.0;
    let last = hud.frames.last();
    egui::Grid::new("frames").num_columns(4).show(ui, |ui| {
        ui.label("fps");
        match last {
            // The last frame's own rate, not an average: the thing worth seeing
            // is the one frame that took 80ms, and a mean over a second is
            // exactly what hides it.
            Some(frame) => ui.label(format!("{:.0}", frame.fps())),
            None => ui.label("—"),
        };
        ui.label("worst");
        match hud.worst_fps {
            Some(worst) => ui.label(format!("{worst:.0}")),
            None => ui.label("—"),
        };
        ui.end_row();
        ui.label("ui");
        ui.label(last.map_or("—".to_string(), |frame| format!("{:.1} ms", ms(frame.ui))));
        ui.label("world");
        ui.label(last.map_or("—".to_string(), |frame| format!("{:.1} ms", ms(frame.scene))));
        ui.end_row();
        ui.label("build");
        ui.label(last.map_or("—".to_string(), |frame| format!("{:.1} ms", ms(frame.build()))));
        // The vsync sleep, named as such: it is the slack in the frame and not
        // work, and a client whose wait is most of the interval has room.
        ui.label("waited");
        ui.label(last.map_or("—".to_string(), |frame| format!("{:.1} ms", ms(frame.wait))));
        ui.end_row();
    });
    // The sentence that turns "the frame rate dropped" from a bug report into a
    // reading. What is asking for frames is the whole answer, and when it is the
    // animation clock that is a rule rather than a symptom — see `App::pacing`.
    ui.label(
        egui::RichText::new(match hud.pacing {
            crate::frames::Pacing::Display => {
                "the display is the pacer: a frame is asked for as soon as the last is queued, and the surface presents in FIFO"
            }
            crate::frames::Pacing::Timer(_) => {
                "nobody is watching the window: the loop is on the animation clock and draws only what the animation needs"
            }
        })
        .weak()
        .small(),
    );

    let span = hud.frames_span.as_secs_f32().max(0.001);
    let end = hud.frames.last().map_or(0.0, |frame| frame.at.as_secs_f32());
    let series = |of: fn(&crate::frames::Frame) -> f64| -> Vec<(f32, f32)> {
        hud.frames
            .iter()
            .map(|frame| (frame.at.as_secs_f32() - (end - span), of(frame) as f32))
            .collect()
    };
    strip(
        ui,
        "frames per second",
        &series(|frame| frame.fps()),
        span,
        egui::Color32::from_rgb(120, 220, 120),
    );
    // One chart and one scale for the two halves, deliberately: the question is
    // which of them is the bigger, and two charts each normalised to their own
    // peak would draw a tenth of a millisecond exactly as tall as ten.
    strips(
        ui,
        "what a frame cost, ms",
        &[
            Curve {
                name: "ui",
                points: series(|frame| frame.ui.as_secs_f64() * 1_000.0),
                colour: egui::Color32::from_rgb(150, 180, 240),
            },
            Curve {
                name: "world",
                points: series(|frame| frame.scene.as_secs_f64() * 1_000.0),
                colour: egui::Color32::from_rgb(220, 200, 90),
            },
        ],
        span,
    );
}

/// A rig as the source line it would be, for pasting into `follow.rs`.
///
/// The one output of this panel that outlives the session, which is why it is a
/// function with a test rather than a `format!` in the middle of a widget.
fn literal(rig: &Rig) -> String {
    let cut = match rig.lift_cut.is_finite() {
        true => format!("{:?}", rig.lift_cut),
        // `inf` is not Rust, and a preset pasted with it in would not compile —
        // which is a thing to find out here rather than in a build.
        false => "f32::INFINITY".to_string(),
    };
    format!(
        "Rig {{ plane_tau: {:?}, lift_tau: {:?}, lift_cut: {cut} }}",
        rig.plane_tau, rig.lift_tau,
    )
}

/// One strip chart: a curve of the last few seconds, scaled to its own peak.
///
/// Scaled to the peak of what is on screen and the peak printed on it, because
/// the axis is not the point — the *shape* is, and a reversal that is a square
/// corner on one rig and a rounded one on another is the whole reason this is
/// drawn rather than tabulated. A fixed axis would flatten every scenario that
/// is not a walk.
fn strip(ui: &mut egui::Ui, title: &str, series: &[(f32, f32)], span: f32, colour: egui::Color32) {
    strips(
        ui,
        title,
        &[Curve {
            // Unnamed, because a chart with one curve names it in the title.
            name: "",
            points: series.to_vec(),
            colour,
        }],
        span,
    );
}

/// One named curve of a strip chart: a point per frame, as (seconds into the
/// window, value).
struct Curve<'a> {
    /// What to call it in the legend, or empty for the one-curve chart.
    name: &'a str,
    points: Vec<(f32, f32)>,
    colour: egui::Color32,
}

/// Several curves in one chart, on one scale.
///
/// One scale and not one each, which is the whole reason this exists: two costs
/// worth comparing are worth comparing, and a chart that normalised each curve
/// to its own peak would draw a tenth of a millisecond exactly as tall as ten
/// and answer the question backwards. Each curve is named in its own colour
/// beside the peak they share.
fn strips(ui: &mut egui::Ui, title: &str, series: &[Curve<'_>], span: f32) {
    let width = ui.available_width().max(180.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 56.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    let peak = series
        .iter()
        .flat_map(|curve| curve.points.iter().map(|(_, value)| *value))
        .fold(0.0f32, f32::max);
    // A flat run has a peak of zero and would divide by it. Drawn along the
    // floor instead, which is what a still eye *is*.
    let scale = match peak > 0.0 {
        true => rect.height() / peak,
        false => 0.0,
    };
    for curve in series {
        let points: Vec<egui::Pos2> = curve
            .points
            .iter()
            .map(|(at, value)| {
                egui::pos2(
                    rect.left() + rect.width() * (at / span).clamp(0.0, 1.0),
                    rect.bottom() - value * scale,
                )
            })
            .collect();
        if points.len() > 1 {
            painter.add(egui::Shape::line(points, egui::Stroke::new(1.0, curve.colour)));
        }
    }
    let mut at = rect.left_top() + egui::vec2(4.0, 2.0);
    let font = egui::FontId::proportional(10.0);
    let colour = series
        .first()
        .map_or(ui.visuals().text_color(), |curve| curve.colour);
    let head = painter.text(
        at,
        egui::Align2::LEFT_TOP,
        format!("{title} — peak {peak:.0}"),
        font.clone(),
        colour,
    );
    at.x = head.right() + 8.0;
    // The legend, and only where there is one: a single curve is named by the
    // title it was drawn under.
    for curve in series.iter().filter(|curve| !curve.name.is_empty()) {
        let drawn = painter.text(at, egui::Align2::LEFT_TOP, curve.name, font.clone(), curve.colour);
        at.x = drawn.right() + 8.0;
    }
}

/// The speech line, docked at the bottom, with what the shard last said above
/// it.
///
/// Answers with a line to say, once, on the frame Enter was pressed.
///
/// # Why the field is refocused by hand
///
/// egui drops focus on Enter, which is right for a form and wrong for a chat
/// box: a player says two things in a row. So the field asks for focus back on
/// the same frame it loses it — which also means the walk keys stay out of the
/// way while typing, since a focused text field consumes them (see
/// [`Shell::on_window_event`], and `App::window_event`, which lets go of every
/// held direction when the UI takes a key).
fn speech_line(root: &mut egui::Ui, hud: &Hud, typed: &mut String) -> Option<String> {
    let mut said = None;
    egui::Panel::bottom("speech").show(root, |ui| {
        // What the shard has said lately, newest last, so the eye ends up beside
        // the line it is about to type into.
        for line in &hud.said {
            ui.label(egui::RichText::new(line).weak());
        }
        ui.horizontal(|ui| {
            ui.label("say");
            let field = ui.add(
                egui::TextEdit::singleline(typed)
                    .desired_width(f32::INFINITY)
                    .hint_text("type, and Enter to speak — a shard's staff commands start with '.'"),
            );
            let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if entered {
                let line = std::mem::take(typed);
                // An empty line is a stray Enter, not silence worth sending:
                // the server would draw an empty message over the player's head.
                if !line.trim().is_empty() {
                    said = Some(line);
                }
                field.request_focus();
            }
        });
    });
    said
}

/// One tile's numbers, each beside a button that puts it on the clipboard —
/// the whole point of holding a selection still is being able to paste one of
/// these into a bug report.
fn tile_panel(ui: &mut egui::Ui, tile: Option<&PickedTile>) {
    let Some(tile) = tile else {
        ui.label("(none)");
        return;
    };
    ui.horizontal(|ui| {
        ui.label(format!("tile {}, {}", tile.x, tile.y));
    });
    ui.horizontal(|ui| match tile.land {
        Some(graphic) => {
            ui.label(format!("land {graphic} (0x{graphic:04X})  z {}", tile.land_z));
            if ui.small_button("copy").clicked() {
                ui.ctx().copy_text(graphic.to_string());
            }
        }
        None => {
            ui.label("land: block not loaded");
        }
    });
    for (graphic, z, hue) in &tile.statics {
        ui.horizontal(|ui| {
            ui.label(format!("static {graphic} (0x{graphic:04X})  z {z}  hue {hue}"));
            if ui.small_button("copy").clicked() {
                ui.ctx().copy_text(graphic.to_string());
            }
        });
    }
}

/// The glow over a tile's diamond: [`Camera::tile_diamond`] gives the corners
/// in *viewport* pixels, physical and post-blit, so they are scaled by
/// `1 / pixels_per_point` and offset by where the viewport starts in the root
/// `Ui`'s own space before a painter can use them — the same points-against-
/// pixels conversion `Shell::run` does for the rect the other direction.
fn draw_tile_highlight(
    ui: &egui::Ui,
    camera: &Camera,
    tile: &PickedTile,
    viewport_origin: egui::Pos2,
    fill: egui::Color32,
    stroke: egui::Stroke,
) {
    let point = openshard_protocol::world::Point {
        x: tile.x,
        y: tile.y,
        z: tile.land_z,
    };
    let scale = 1.0 / ui.ctx().pixels_per_point();
    let corners = camera
        .tile_diamond(point)
        .map(|(x, y)| viewport_origin + egui::vec2(x * scale, y * scale))
        .to_vec();
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("tile-highlight"),
    ));
    painter.add(egui::Shape::convex_polygon(corners, fill, stroke));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rig printed by the panel is a line that compiles.
    ///
    /// The promise the sliders are for: a setting that felt right in the window
    /// is pasted into `follow.rs` and committed as the preset it turned out to
    /// be. Pinned because the failure is silent at the point it is made — the
    /// paste is a build error hours later, in another file.
    #[test]
    fn a_rig_prints_as_the_source_line_it_would_be() {
        assert_eq!(
            literal(&Rig::LIFT),
            "Rig { plane_tau: 0.0, lift_tau: 0.15, lift_cut: 64.0 }",
        );
        // `inf` is what `Display` would give, and it is not Rust.
        let never = Rig {
            lift_cut: f32::INFINITY,
            ..Rig::HARD
        };
        assert_eq!(
            literal(&never),
            "Rig { plane_tau: 0.0, lift_tau: 0.0, lift_cut: f32::INFINITY }",
        );
    }
}
