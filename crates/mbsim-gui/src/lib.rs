//! GUI of mbsim: the board as it looks (front and back, LED light output,
//! microphone, power and USB LEDs, speaker), the physical inputs, the UART terminal and the
//! simulation controls.

mod artwork;
mod audio;
mod optics;

use artwork::{draw_back, draw_front, hit_back, hit_front, BackState, FrontState, Hit, View};
use crossbeam_channel::Receiver;
use egui::{Color32, Key, RichText, Sense, TextureHandle, TextureOptions};
use mbsim_core::time::{from_secs, to_secs};
use mbsim_core::Ticks;
use mbsim_microbit::board::Orientation;
use mbsim_microbit::runtime::{Cmd, Input, Simulator, Speed, Status};
use optics::Optics;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct GuiOptions {
    pub title: String,
    /// Start with the host microphone feeding the board's microphone.
    pub mic: bool,
}

/// Runs the GUI on the current (main) thread until the window closes.
pub fn run(sim: Simulator, opts: GuiOptions) -> Result<(), String> {
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(opts.title.clone())
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([640.0, 480.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    let title = opts.title.clone();
    eframe::run_native(
        &title,
        native,
        Box::new(move |cc| {
            let mut app = App::new(cc, sim);
            if opts.mic {
                app.set_mic(true);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Default of mV at the microphone output for a full-scale host sample: a host MEMS microphone
/// at -26 dBFS/Pa against the 12.6 mV/Pa (-38 dBV/Pa) of the SPU0410.
const MIC_FULL_SCALE_MV: f32 = 250.0;

/// Hint when the host microphone delivers only zeros: the operating system withholds it.
const MIC_SILENT: &str = if cfg!(target_os = "macos") {
    "no sound yet: allow microphone access for the app that started mbsim (System Settings › Privacy & Security › Microphone)"
} else if cfg!(target_os = "windows") {
    "no sound yet: allow microphone access for desktop apps (Settings › Privacy & security › Microphone)"
} else {
    "no sound yet: check the input device and its level"
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Front,
    Back,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LineEnd {
    None,
    Lf,
    Cr,
    CrLf,
}

struct App {
    sim: Simulator,
    status: Status,
    optics: Optics,
    synth: audio::Synth,
    glow: TextureHandle,
    side: Side,
    camera: bool,
    // inputs (GUI side)
    buttons: [bool; 2],
    key_buttons: [bool; 2],
    mouse_hit: Option<Hit>,
    touch: bool,
    finger_pf: f64,
    orientation: Orientation,
    temperature: f64,
    sound_mv: f64,
    sound_hz: f64,
    mic_noise: f64,
    /// Host microphone, while it feeds the board's microphone.
    mic: Option<audio::Mic>,
    mic_error: Option<String>,
    mic_full_scale_mv: f32,
    /// `mic_full_scale_mv` for the audio thread (f32 bits).
    mic_gain: Arc<AtomicU32>,
    /// Peak level for the meter (full scale 1.0), whether any sound came yet, and since when.
    mic_level: f32,
    mic_heard: bool,
    mic_since: Instant,
    shake_until: Option<Instant>,
    // terminal
    uart_rx: Receiver<Vec<u8>>,
    terminal: String,
    input_line: String,
    line_end: LineEnd,
    usb_activity: Instant,
    // misc
    last_time: Ticks,
    message: Option<(String, Instant)>,
    show_warnings: bool,
    /// `MBSIM_SCREENSHOT=path.png[:seconds]`: save a screenshot and exit (documentation, tests).
    screenshot: Option<(String, Instant, bool)>,
    started: Instant,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, sim: Simulator) -> App {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let glow = cc.egui_ctx.load_texture("led-glow", artwork::glow_image(128), TextureOptions::LINEAR);
        let uart_rx = sim.subscribe_uart();
        let status = sim.status();
        let inputs = status.inputs.clone().unwrap_or_default();
        App {
            sim,
            status,
            optics: Optics::new(),
            synth: audio::Synth::new(),
            glow,
            // `MBSIM_GUI_VIEW=back` starts with the back of the board (screenshots)
            side: if std::env::var("MBSIM_GUI_VIEW").is_ok_and(|v| v.eq_ignore_ascii_case("back")) { Side::Back } else { Side::Front },
            camera: false,
            buttons: [false; 2],
            key_buttons: [false; 2],
            mouse_hit: None,
            touch: false,
            finger_pf: 100.0,
            orientation: inputs.orientation,
            temperature: inputs.temperature,
            sound_mv: inputs.sound_mv,
            sound_hz: inputs.sound_hz,
            mic_noise: inputs.mic_noise_mv,
            mic: None,
            mic_error: None,
            mic_full_scale_mv: MIC_FULL_SCALE_MV,
            mic_gain: Arc::new(AtomicU32::new(MIC_FULL_SCALE_MV.to_bits())),
            mic_level: 0.0,
            mic_heard: false,
            mic_since: Instant::now(),
            shake_until: None,
            uart_rx,
            terminal: String::new(),
            input_line: String::new(),
            line_end: LineEnd::Lf,
            usb_activity: Instant::now() - Duration::from_secs(10),
            last_time: 0,
            message: None,
            show_warnings: false,
            screenshot: std::env::var("MBSIM_SCREENSHOT").ok().map(|v| {
                let (path, secs) = match v.rsplit_once(':') {
                    Some((p, s)) if s.parse::<f64>().is_ok() => (p.to_string(), s.parse::<f64>().unwrap()),
                    _ => (v.clone(), 2.0),
                };
                (path, Instant::now() + Duration::from_secs_f64(secs), false)
            }),
            started: Instant::now(),
        }
    }

    fn screenshot(&mut self, ctx: &egui::Context) {
        let Some((path, at, requested)) = self.screenshot.clone() else { return };
        if !requested && Instant::now() >= at {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.screenshot = Some((path.clone(), at, true));
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let [w, h] = img.size;
            let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            let file = std::fs::File::create(&path).expect("screenshot file");
            let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().expect("png header");
            wr.write_image_data(&rgba).expect("png data");
            drop(wr);
            eprintln!("screenshot saved to {path}");
            std::process::exit(0);
        }
        let _ = self.started;
    }

    fn input(&self, i: Input) {
        self.sim.send(Cmd::Input(i));
    }

    fn set_button(&mut self, i: usize, pressed: bool) {
        if self.buttons[i] != pressed {
            self.buttons[i] = pressed;
            self.input(Input::Button(i, pressed));
        }
    }

    fn set_touch(&mut self, on: bool) {
        if self.touch != on {
            self.touch = on;
            self.input(Input::Touch(if on { self.finger_pf } else { 0.0 }));
        }
    }

    /// Switches the host microphone on or off. Its blocks go from the audio thread straight to
    /// the simulation, so they keep coming while the window is not redrawn.
    fn set_mic(&mut self, on: bool) {
        self.mic = None;
        self.mic_error = None;
        if !on {
            return;
        }
        let sim = self.sim.clone();
        let gain = self.mic_gain.clone();
        let sink = move |rate: f64, block: &[f32]| {
            let g = f32::from_bits(gain.load(Ordering::Relaxed));
            sim.send(Cmd::Input(Input::LiveSound { rate, mv: block.iter().map(|s| s * g).collect() }));
        };
        match audio::Mic::open(sink) {
            Ok(m) => {
                self.mic = Some(m);
                self.mic_level = 0.0;
                self.mic_heard = false;
                self.mic_since = Instant::now();
            }
            Err(e) => {
                log::warn!("microphone not available: {e}");
                self.mic_error = Some(e);
            }
        }
    }

    fn poll(&mut self) {
        self.status = self.sim.status();
        if let Some(m) = &self.mic {
            let p = m.take_peak();
            self.mic_level = p.max(self.mic_level * 0.85);
            self.mic_heard |= p > 0.0;
        }
        let now = self.status.time;
        if now < self.last_time {
            // reset of the timeline (new image or power cycle)
            self.optics = Optics::new();
            self.synth.resync(now);
        }
        self.last_time = now;
        let ev = self.sim.take_pin_events();
        self.optics.update(ev, now);
        let realtime = !self.status.speed_max && (self.status.speed_factor - 1.0).abs() < 1e-6;
        for (t, high) in self.optics.speaker_edges.drain(..) {
            self.synth.edge(t, high);
        }
        if realtime && self.status.halted.is_none() && !self.status.paused {
            self.synth.advance(now);
        } else {
            self.synth.resync(now);
        }
        while let Ok(data) = self.uart_rx.try_recv() {
            self.usb_activity = Instant::now();
            self.terminal.push_str(&String::from_utf8_lossy(&data).replace('\r', ""));
            if self.terminal.len() > 200_000 {
                let cut = self.terminal.len() - 150_000;
                let cut = self.terminal.char_indices().map(|(i, _)| i).find(|&i| i >= cut).unwrap_or(0);
                self.terminal.drain(..cut);
            }
        }
        if let Some(t) = self.shake_until {
            if Instant::now() >= t {
                self.shake_until = None;
                self.input(Input::AccelExtra([0.0; 3]));
            } else {
                let phase = Instant::now().elapsed().as_secs_f64();
                let a = 1500.0 * (phase * 60.0).sin();
                self.input(Input::AccelExtra([a, a * 0.6, 0.0]));
            }
        }
    }

    fn handle_files(&mut self, ctx: &egui::Context) {
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        for f in files {
            let path = f.path().to_path_buf();
            match mbsim_loader::Image::load(&path) {
                Ok(img) => {
                    let r = self.sim.load(img, false);
                    let msg = match r {
                        Ok(()) => format!("programmed {}", path.display()),
                        Err(e) => format!("{}: {e}", path.display()),
                    };
                    self.message = Some((msg, Instant::now()));
                }
                Err(e) => self.message = Some((e.to_string(), Instant::now())),
            }
        }
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (a, b, t) = ctx.input(|i| (i.key_down(Key::A), i.key_down(Key::B), i.key_down(Key::T)));
        let space = ctx.input(|i| i.key_down(Key::Space));
        self.key_buttons = [a || space, b || space];
        if ctx.input(|i| i.key_pressed(Key::R)) {
            self.input(Input::ResetButton);
        }
        if ctx.input(|i| i.key_pressed(Key::F)) {
            self.side = if self.side == Side::Front { Side::Back } else { Side::Front };
        }
        let mouse_touch = self.mouse_hit == Some(Hit::Logo);
        self.set_touch(t || mouse_touch);
        let ma = self.mouse_hit == Some(Hit::ButtonA);
        let mb = self.mouse_hit == Some(Hit::ButtonB);
        let ka = self.key_buttons[0];
        let kb = self.key_buttons[1];
        self.set_button(0, ma || ka);
        self.set_button(1, mb || kb);
    }

    fn board_view(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_size();
        let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
        let view = View::fit(rect);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(0x2a, 0x2e, 0x34));
        let hover = resp.hover_pos().map(|p| view.to_mm(p)).and_then(|mm| match self.side {
            Side::Front => hit_front(mm),
            Side::Back => hit_back(mm),
        });
        // primary button: press parts
        let down = resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_down());
        let hit = if down {
            resp.interact_pointer_pos().map(|p| view.to_mm(p)).and_then(|mm| match self.side {
                Side::Front => hit_front(mm),
                Side::Back => hit_back(mm),
            })
        } else {
            None
        };
        if hit == Some(Hit::Reset) && self.mouse_hit != Some(Hit::Reset) {
            self.input(Input::ResetButton);
        }
        self.mouse_hit = hit;
        // secondary drag: tilt the board
        if resp.dragged_by(egui::PointerButton::Secondary) {
            let d = resp.drag_delta();
            self.orientation.roll = (self.orientation.roll + d.x as f64 * 0.5).clamp(-180.0, 180.0);
            self.orientation.pitch = (self.orientation.pitch - d.y as f64 * 0.5).clamp(-180.0, 180.0);
            self.input(Input::Orientation(self.orientation));
        }
        // tilt visualisation: shift the board slightly
        let tilt = egui::vec2(self.orientation.roll.to_radians().sin() as f32, -self.orientation.pitch.to_radians().sin() as f32)
            * view.scale
            * 1.2;
        let view = View { origin: view.origin + tilt, ..view };
        let powered = !self.status.system_off;
        match self.side {
            Side::Front => {
                let s = FrontState {
                    optics: &self.optics,
                    buttons: self.buttons,
                    touched: self.touch,
                    glow: self.glow.id(),
                    camera: self.camera,
                    hover,
                };
                draw_front(&painter, &view, &s);
            }
            Side::Back => {
                let usb = 1.0 - (self.usb_activity.elapsed().as_secs_f32() / 0.06).min(1.0);
                let s = BackState {
                    optics: &self.optics,
                    glow: self.glow.id(),
                    powered,
                    usb_activity: usb,
                    speaker_activity: self.synth.activity,
                    buttons: self.buttons,
                    reset_pressed: self.mouse_hit == Some(Hit::Reset),
                    hover,
                };
                draw_back(&painter, &view, &s);
            }
        }
        if resp.hovered() {
            if let Some(h) = hover {
                let text = match h {
                    Hit::ButtonA => "button A (key A)".to_string(),
                    Hit::ButtonB => "button B (key B)".to_string(),
                    Hit::Logo => "touch logo (key T)".to_string(),
                    Hit::Reset => "reset button (key R)".to_string(),
                    Hit::Led(r, c) => format!("LED row {} column {}: {:.0} %", r + 1, c + 1, self.optics.luminance(r, c) * 100.0),
                    Hit::Ring(i) => ring_text(i, self.optics.drive(mbsim_microbit::pins::RING[i])),
                };
                painter.text(
                    rect.left_bottom() + egui::vec2(10.0, -10.0),
                    egui::Align2::LEFT_BOTTOM,
                    text,
                    egui::FontId::proportional(14.0),
                    Color32::from_gray(200),
                );
            }
        }
        if let Some((m, t)) = &self.message {
            if t.elapsed() < Duration::from_secs(5) {
                painter.text(
                    rect.center_top() + egui::vec2(0.0, 12.0),
                    egui::Align2::CENTER_TOP,
                    m,
                    egui::FontId::proportional(15.0),
                    Color32::from_rgb(0xf0, 0xd0, 0x80),
                );
            }
        }
    }

    fn status_line(&self) -> String {
        let s = &self.status;
        let state = if s.system_off {
            "System OFF".to_string()
        } else if let Some(r) = s.halted {
            format!("halted ({r:?})")
        } else if s.paused {
            "paused".to_string()
        } else {
            "running".to_string()
        };
        format!(
            "{state}{}   t = {:.3} s   {:.1} MIPS   {:.2}x real time",
            if s.debugger { "   debugger attached" } else { "" },
            to_secs(s.time),
            s.mips,
            s.realtime_factor
        )
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Simulation");
        ui.horizontal(|ui| {
            let paused = self.status.paused;
            if ui.button(if paused { "Resume" } else { "Pause" }).clicked() {
                self.sim.send(Cmd::Pause(!paused));
            }
            if ui.button("Reset").on_hover_text("reset button (pin reset)").clicked() {
                self.input(Input::ResetButton);
            }
            if self.status.halted.is_some() && !self.status.debugger && ui.button("Continue").clicked() {
                self.sim.send(Cmd::Continue);
            }
        });
        let cur = if self.status.speed_max { 0.0 } else { self.status.speed_factor };
        let mut sel = cur;
        egui::ComboBox::from_label("speed").selected_text(if cur == 0.0 { "max".to_string() } else { format!("{cur}x") }).show_ui(
            ui,
            |ui| {
                for v in [0.01, 0.1, 0.5, 1.0, 2.0, 10.0, 0.0] {
                    let label = if v == 0.0 { "max".to_string() } else { format!("{v}x") };
                    ui.selectable_value(&mut sel, v, label);
                }
            },
        );
        if sel != cur {
            self.sim.send(Cmd::Speed(if sel == 0.0 { Speed::Max } else { Speed::Realtime(sel) }));
        }
        ui.separator();
        ui.heading("View");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.side, Side::Front, "Front");
            ui.selectable_value(&mut self.side, Side::Back, "Back");
            ui.label("(F)");
        });
        ui.checkbox(&mut self.camera, "camera view (instantaneous LED state)");
        let mut tau_ms = self.optics.tau * 1000.0;
        if ui.add(egui::Slider::new(&mut tau_ms, 2.0..=60.0).text("persistence ms")).changed() {
            self.optics.tau = tau_ms / 1000.0;
        }
        ui.separator();
        ui.heading("Inputs");
        ui.horizontal(|ui| {
            let mut a = self.buttons[0];
            let mut b = self.buttons[1];
            if ui.toggle_value(&mut a, "A").changed() {
                self.set_button(0, a);
            }
            if ui.toggle_value(&mut b, "B").changed() {
                self.set_button(1, b);
            }
            let mut t = self.touch;
            if ui.toggle_value(&mut t, "logo").changed() {
                self.set_touch(t);
            }
        });
        ui.add(egui::Slider::new(&mut self.finger_pf, 10.0..=500.0).text("finger pF"));
        let mut o = self.orientation;
        let r1 = ui.add(egui::Slider::new(&mut o.roll, -180.0..=180.0).text("roll (right side down)"));
        let r2 = ui.add(egui::Slider::new(&mut o.pitch, -180.0..=180.0).text("pitch (connector down)"));
        let r3 = ui.add(egui::Slider::new(&mut o.heading, 0.0..=360.0).text("heading"));
        if r1.changed() || r2.changed() || r3.changed() {
            self.orientation = o;
            self.input(Input::Orientation(o));
        }
        ui.horizontal(|ui| {
            if ui.button("Level").clicked() {
                self.orientation = Orientation { heading: self.orientation.heading, ..Default::default() };
                self.input(Input::Orientation(self.orientation));
            }
            if ui.button("Face down").clicked() {
                self.orientation.pitch = 180.0;
                self.orientation.roll = 0.0;
                self.input(Input::Orientation(self.orientation));
            }
            if ui.button("Shake").clicked() {
                self.shake_until = Some(Instant::now() + Duration::from_millis(400));
            }
        });
        ui.label(RichText::new("right-drag the board to tilt it").small().weak());
        if ui.add(egui::Slider::new(&mut self.temperature, -20.0..=60.0).text("temperature °C")).changed() {
            self.input(Input::Temperature(self.temperature));
        }
        ui.separator();
        ui.heading("Microphone");
        let mut live = self.mic.is_some();
        if ui
            .checkbox(&mut live, "host microphone")
            .on_hover_text("the computer's default input device feeds the board's microphone in real time")
            .changed()
        {
            self.set_mic(live);
        }
        if let Some(mic) = &self.mic {
            let db = 20.0 * self.mic_level.max(1e-5).log10();
            ui.add(egui::ProgressBar::new(((db + 60.0) / 60.0).clamp(0.0, 1.0)).text(format!("{db:.0} dBFS")));
            if ui
                .add(egui::Slider::new(&mut self.mic_full_scale_mv, 1.0..=2000.0).logarithmic(true).text("mV full scale"))
                .on_hover_text("mV at the board's microphone output for a full-scale sample of the host microphone")
                .changed()
            {
                self.mic_gain.store(self.mic_full_scale_mv.to_bits(), Ordering::Relaxed);
            }
            ui.label(RichText::new(&mic.device).small().weak());
            let warn = Color32::from_rgb(0xe0, 0xb0, 0x60);
            if let Some(e) = mic.error() {
                ui.label(RichText::new(e).small().color(warn));
            } else if !self.mic_heard && self.mic_since.elapsed() > Duration::from_secs(2) {
                ui.label(RichText::new(MIC_SILENT).small().color(warn));
            }
            if self.status.speed_max || (self.status.speed_factor - 1.0).abs() > 1e-6 {
                ui.label(RichText::new("live sound needs speed 1x: at other speeds parts of it are lost").small().weak());
            }
        } else if let Some(e) = &self.mic_error {
            ui.label(RichText::new(format!("no microphone: {e}")).small().color(Color32::from_rgb(0xe0, 0xb0, 0x60)));
        }
        let s1 = ui.add(egui::Slider::new(&mut self.sound_mv, 0.0..=40.0).text("sound mV"));
        let s2 = ui.add(egui::Slider::new(&mut self.sound_hz, 50.0..=4000.0).logarithmic(true).text("Hz"));
        if s1.changed() || s2.changed() {
            self.input(Input::Sound { mv: self.sound_mv, hz: self.sound_hz });
        }
        if ui.add(egui::Slider::new(&mut self.mic_noise, 0.0..=2.0).text("noise mV rms")).changed() {
            self.input(Input::MicNoise(self.mic_noise));
        }
        if ui.button("Clap").clicked() {
            self.input(Input::Clap(13.0));
        }
        ui.separator();
        ui.heading("Speaker");
        if self.synth.available {
            ui.add(egui::Slider::new(&mut self.synth.volume, 0.0..=1.0).text("volume"));
            ui.checkbox(&mut self.synth.muted, "mute");
        } else {
            ui.label("no audio output");
        }
        let n = self.status.warnings.len();
        if n > 0 {
            ui.separator();
            ui.toggle_value(&mut self.show_warnings, format!("{n} model warning(s)"));
            if self.show_warnings {
                for w in &self.status.warnings {
                    ui.label(RichText::new(w).small().color(Color32::from_rgb(0xe0, 0xb0, 0x60)));
                }
            }
        }
        ui.separator();
        ui.label(RichText::new("Drop a .hex or .elf on the window to program the board.").small().weak());
    }

    fn terminal(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("UART (USB serial)").strong());
            if ui.small_button("clear").clicked() {
                self.terminal.clear();
            }
            egui::ComboBox::from_id_salt("line-end")
                .selected_text(match self.line_end {
                    LineEnd::None => "no line end",
                    LineEnd::Lf => "LF",
                    LineEnd::Cr => "CR",
                    LineEnd::CrLf => "CR LF",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.line_end, LineEnd::None, "no line end");
                    ui.selectable_value(&mut self.line_end, LineEnd::Lf, "LF");
                    ui.selectable_value(&mut self.line_end, LineEnd::Cr, "CR");
                    ui.selectable_value(&mut self.line_end, LineEnd::CrLf, "CR LF");
                });
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.input_line).hint_text("send to the target").desired_width(ui.available_width() - 60.0),
            );
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if ui.button("Send").clicked() || enter {
                let mut text = std::mem::take(&mut self.input_line);
                text.push_str(match self.line_end {
                    LineEnd::None => "",
                    LineEnd::Lf => "\n",
                    LineEnd::Cr => "\r",
                    LineEnd::CrLf => "\r\n",
                });
                self.sim.send(Cmd::UartIn(text.into_bytes()));
                resp.request_focus();
            }
        });
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(&self.terminal).monospace().size(12.5)).wrap());
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll();
        self.screenshot(&ctx);
        self.handle_files(&ctx);
        self.keyboard(&ctx);
        egui::Panel::top("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("BBC micro:bit V2").strong());
                ui.label(self.status_line());
            });
        });
        egui::Panel::right("controls").resizable(true).default_size(300.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.controls(ui));
        });
        egui::Panel::bottom("terminal").resizable(true).default_size(200.0).show(ui, |ui| self.terminal(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.board_view(ui));
        ctx.request_repaint_after(Duration::from_millis(16));
        let _ = from_secs(0.0);
    }
}

/// Hover text of an edge connector ring: what the nRF52833 drives on its pin.
fn ring_text(ring: usize, d: mbsim_nrf52833::board::PinDrive) -> String {
    let pin = mbsim_microbit::pins::RING[ring];
    let state = if d.output {
        format!("output {}", if d.level { "high" } else { "low" })
    } else if d.input {
        match d.pull {
            1 => "input, pull-down".to_string(),
            3 => "input, pull-up".to_string(),
            _ => "input, floating".to_string(),
        }
    } else {
        "disconnected".to_string()
    };
    format!("pin {} (P{}.{:02}): {}", ring, pin >> 5, pin & 31, state)
}
