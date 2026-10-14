//! Drawing of the board, front and back, at 1:1 millimetre geometry of the 51.6 x 42 mm PCB.
//!
//! The artwork is drawn for this project from the V2.2.1 schematic and the published board
//! dimensions; positions of parts are approximations to be refined against photos.

use crate::optics::Optics;
use egui::epaint::CornerRadius;
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, StrokeKind, TextureId, Vec2};

pub const BOARD_W: f32 = 51.6;
pub const BOARD_H: f32 = 42.0;

pub const PCB: Color32 = Color32::from_rgb(0x1c, 0x1d, 0x1f);
pub const PCB_EDGE: Color32 = Color32::from_rgb(0x2c, 0x2d, 0x30);
pub const GOLD: Color32 = Color32::from_rgb(0xd6, 0xae, 0x52);
pub const GOLD_DARK: Color32 = Color32::from_rgb(0x9c, 0x7a, 0x30);
pub const SILK: Color32 = Color32::from_rgb(0xee, 0xee, 0xea);
pub const LED_OFF: Color32 = Color32::from_rgb(0xd9, 0xcf, 0xbd);
pub const LED_RED: Color32 = Color32::from_rgb(0xff, 0x1e, 0x10);

pub const BUTTON_A: Pos2 = pos2(5.6, 18.0);
pub const BUTTON_B: Pos2 = pos2(46.0, 18.0);
pub const LOGO: Pos2 = pos2(25.8, 4.8);
pub const MIC_HOLE: Pos2 = pos2(33.6, 4.4);
pub const MIC_LED: Pos2 = pos2(35.4, 4.4);
pub const LED_ORIGIN: Pos2 = pos2(16.6, 9.8);
pub const LED_PITCH: Vec2 = vec2(4.6, 4.1);
/// Back side (seen from the back, so mirrored left-right).
pub const RESET_BTN: Pos2 = pos2(39.4, 2.9);
pub const POWER_LED: Pos2 = pos2(31.8, 3.2);
pub const USB_LED: Pos2 = pos2(19.8, 3.2);
pub const SPEAKER: Pos2 = pos2(25.8, 27.5);

/// Maps board millimetres to screen points.
#[derive(Clone, Copy)]
pub struct View {
    pub origin: Pos2,
    pub scale: f32,
}

impl View {
    pub fn fit(rect: Rect) -> View {
        let scale = (rect.width() / (BOARD_W + 6.0)).min(rect.height() / (BOARD_H + 6.0));
        let size = vec2(BOARD_W, BOARD_H) * scale;
        View { origin: rect.center() - size / 2.0, scale }
    }

    pub fn p(&self, mm: Pos2) -> Pos2 {
        self.origin + mm.to_vec2() * self.scale
    }

    pub fn len(&self, mm: f32) -> f32 {
        mm * self.scale
    }

    pub fn rect(&self, center: Pos2, size: Vec2) -> Rect {
        Rect::from_center_size(self.p(center), size * self.scale)
    }

    pub fn to_mm(self, p: Pos2) -> Pos2 {
        ((p - self.origin) / self.scale).to_pos2()
    }
}

pub fn led_center(r: usize, c: usize) -> Pos2 {
    LED_ORIGIN + vec2(c as f32 * LED_PITCH.x, r as f32 * LED_PITCH.y)
}

fn cr(v: f32) -> CornerRadius {
    CornerRadius::same(v.clamp(0.0, 255.0) as u8)
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// Glow sprite: additive radial falloff in the LED colour.
pub fn glow(p: &Painter, tex: TextureId, center: Pos2, radius: f32, color: Color32, intensity: f32) {
    if intensity <= 0.003 {
        return;
    }
    let i = intensity.min(2.5);
    let c = Color32::from_rgb(
        (color.r() as f32 * i.min(1.0)) as u8,
        (color.g() as f32 * i.min(1.0)) as u8,
        (color.b() as f32 * i.min(1.0)) as u8,
    )
    .additive();
    let r = radius * (0.6 + 0.4 * i.sqrt());
    p.image(tex, Rect::from_center_size(center, vec2(2.0 * r, 2.0 * r)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), c);
}

fn board_outline(p: &Painter, v: &View) {
    let r = Rect::from_min_size(v.p(pos2(0.0, 0.0)), vec2(BOARD_W, BOARD_H) * v.scale);
    // drop shadow
    p.rect_filled(r.translate(vec2(0.0, v.len(0.8))), cr(v.len(3.0)), Color32::from_black_alpha(90));
    p.rect_filled(r, cr(v.len(2.8)), PCB_EDGE);
    p.rect_filled(r.shrink(v.len(0.35)), cr(v.len(2.5)), PCB);
}

/// Edge connector: five large pads with holes and the 1.27 mm strips between them.
fn edge_connector(p: &Painter, v: &View, labels: bool, mirror: bool) {
    let pads = [("0", 6.0), ("1", 16.0), ("2", 25.8), ("3V", 35.6), ("GND", 45.6)];
    let mx = |x: f32| if mirror { BOARD_W - x } else { x };
    // small strips
    let mut x = 2.4;
    while x < BOARD_W - 2.4 {
        let near_pad = pads.iter().any(|(_, px)| (x - px).abs() < 4.2);
        if !near_pad {
            let r = v.rect(pos2(mx(x), 39.6), vec2(0.8, 4.6));
            p.rect_filled(r, cr(v.len(0.15)), GOLD);
        }
        x += 1.27;
    }
    for (label, px) in pads {
        let c = pos2(mx(px), 37.9);
        let r = v.rect(c, vec2(7.4, 8.0));
        p.rect_filled(r, cr(v.len(1.2)), GOLD);
        p.rect_stroke(r, cr(v.len(1.2)), Stroke::new(v.len(0.15), GOLD_DARK), StrokeKind::Inside);
        p.circle_filled(v.p(pos2(mx(px), 37.0)), v.len(2.0), Color32::from_rgb(0x30, 0x30, 0x30));
        p.circle_stroke(v.p(pos2(mx(px), 37.0)), v.len(2.0), Stroke::new(v.len(0.3), GOLD_DARK));
        if labels {
            p.text(v.p(pos2(mx(px), 31.6)), Align2::CENTER_CENTER, label, FontId::proportional(v.len(2.0)), SILK);
        }
    }
}

pub struct FrontState<'a> {
    pub optics: &'a Optics,
    pub buttons: [bool; 2],
    pub touched: bool,
    pub glow: TextureId,
    pub camera: bool,
    pub hover: Option<Hit>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    ButtonA,
    ButtonB,
    Logo,
    Reset,
    Led(usize, usize),
    /// Edge connector ring 0..2.
    Ring(usize),
}

const RING_X: [f32; 3] = [6.0, 16.0, 25.8];

pub fn hit_front(mm: Pos2) -> Option<Hit> {
    if mm.distance(BUTTON_A) < 3.6 {
        return Some(Hit::ButtonA);
    }
    if mm.distance(BUTTON_B) < 3.6 {
        return Some(Hit::ButtonB);
    }
    if (mm.x - LOGO.x).abs() < 5.5 && (mm.y - LOGO.y).abs() < 3.0 {
        return Some(Hit::Logo);
    }
    for r in 0..5 {
        for c in 0..5 {
            if mm.distance(led_center(r, c)) < 1.4 {
                return Some(Hit::Led(r, c));
            }
        }
    }
    hit_ring(mm.x, mm.y)
}

fn hit_ring(x: f32, y: f32) -> Option<Hit> {
    RING_X.iter().position(|px| (x - px).abs() < 3.7 && (y - 37.9).abs() < 4.0).map(Hit::Ring)
}

pub fn hit_back(mm: Pos2) -> Option<Hit> {
    if mm.distance(RESET_BTN) < 2.4 {
        return Some(Hit::Reset);
    }
    let mirrored = pos2(BOARD_W - mm.x, mm.y);
    match hit_front(mirrored) {
        Some(h @ (Hit::ButtonA | Hit::ButtonB | Hit::Ring(_))) => Some(h),
        _ => None,
    }
}

fn button(p: &Painter, v: &View, c: Pos2, pressed: bool, hover: bool, label: Option<&str>) {
    let body = v.rect(c, vec2(6.2, 6.2));
    p.rect_filled(body, cr(v.len(0.8)), Color32::from_rgb(0x3a, 0x3a, 0x3c));
    p.rect_stroke(body, cr(v.len(0.8)), Stroke::new(v.len(0.2), Color32::from_rgb(0x70, 0x70, 0x72)), StrokeKind::Inside);
    for (dx, dy) in [(-2.6, -2.6), (2.6, -2.6), (-2.6, 2.6), (2.6, 2.6)] {
        p.circle_filled(v.p(c + vec2(dx, dy)), v.len(0.35), Color32::from_rgb(0x9a, 0x9a, 0x9a));
    }
    let cap = if pressed {
        Color32::from_rgb(0x05, 0x05, 0x05)
    } else if hover {
        Color32::from_rgb(0x26, 0x26, 0x28)
    } else {
        Color32::from_rgb(0x14, 0x14, 0x16)
    };
    let off = if pressed { vec2(0.0, 0.15) } else { Vec2::ZERO };
    p.circle_filled(v.p(c + off), v.len(2.3), cap);
    if !pressed {
        p.circle_stroke(v.p(c + vec2(-0.5, -0.6)), v.len(1.3), Stroke::new(v.len(0.25), Color32::from_white_alpha(28)));
    }
    if let Some(l) = label {
        p.text(v.p(c + vec2(0.0, -5.4)), Align2::CENTER_CENTER, l, FontId::proportional(v.len(2.6)), SILK);
    }
}

fn stadium(p: &Painter, v: &View, c: Pos2, size: Vec2, stroke: f32, color: Color32) {
    let r = v.rect(c, size);
    p.rect_stroke(r, cr(r.height() / 2.0), Stroke::new(v.len(stroke), color), StrokeKind::Middle);
}

pub fn draw_front(p: &Painter, v: &View, s: &FrontState) {
    board_outline(p, v);
    edge_connector(p, v, true, false);
    // touch logo
    let gold = if s.touched {
        Color32::from_rgb(0xff, 0xd2, 0x6e)
    } else if s.hover == Some(Hit::Logo) {
        Color32::from_rgb(0xe8, 0xc0, 0x60)
    } else {
        GOLD
    };
    stadium(p, v, LOGO, vec2(9.6, 4.6), 0.9, gold);
    p.circle_filled(v.p(LOGO + vec2(-2.2, 0.0)), v.len(0.85), gold);
    p.circle_filled(v.p(LOGO + vec2(2.2, 0.0)), v.len(0.85), gold);
    if s.touched {
        p.circle_filled(v.p(LOGO + vec2(0.6, 0.8)), v.len(3.2), Color32::from_rgba_unmultiplied(0xf0, 0xc8, 0xa8, 70));
    }
    // microphone port and LED
    p.circle_filled(v.p(MIC_HOLE), v.len(0.75), Color32::from_rgb(0x55, 0x55, 0x55));
    p.circle_filled(v.p(MIC_HOLE), v.len(0.4), Color32::BLACK);
    let mic = s.optics.mic_led.clamp(0.0, 1.0);
    p.rect_filled(v.rect(MIC_LED, vec2(0.9, 1.4)), cr(v.len(0.15)), lerp(LED_OFF, LED_RED, mic));
    glow(p, s.glow, v.p(MIC_LED), v.len(2.4), LED_RED, mic);
    // buttons
    button(p, v, BUTTON_A, s.buttons[0], s.hover == Some(Hit::ButtonA), Some("A"));
    button(p, v, BUTTON_B, s.buttons[1], s.hover == Some(Hit::ButtonB), Some("B"));
    // LED matrix
    for r in 0..5 {
        for c in 0..5 {
            let lum = if s.camera {
                if s.optics.instant[r][c] {
                    1.0
                } else {
                    0.0
                }
            } else {
                s.optics.luminance(r, c)
            };
            let center = led_center(r, c);
            let body = v.rect(center, vec2(1.15, 1.75));
            p.rect_filled(body.expand(v.len(0.12)), cr(v.len(0.2)), Color32::from_rgb(0x55, 0x52, 0x4c));
            let core = lerp(LED_OFF, LED_RED, (lum * 1.6).min(1.0));
            p.rect_filled(body, cr(v.len(0.15)), core);
            if lum > 0.02 {
                let hot = lerp(LED_RED, Color32::from_rgb(0xff, 0xd8, 0xc0), ((lum - 0.6) / 1.4).clamp(0.0, 1.0));
                p.rect_filled(body.shrink(v.len(0.25)), cr(v.len(0.1)), hot);
            }
            glow(p, s.glow, v.p(center), v.len(3.4), LED_RED, lum);
        }
    }
    p.text(v.p(pos2(3.0, 28.8)), Align2::LEFT_CENTER, "V2.2", FontId::proportional(v.len(1.6)), Color32::from_gray(150));
}

pub struct BackState<'a> {
    pub optics: &'a Optics,
    pub glow: TextureId,
    pub powered: bool,
    pub usb_activity: f32,
    pub speaker_activity: f32,
    pub buttons: [bool; 2],
    pub reset_pressed: bool,
    pub hover: Option<Hit>,
}

fn chip(p: &Painter, v: &View, c: Pos2, size: Vec2, name: &str) {
    let r = v.rect(c, size);
    p.rect_filled(r.expand(v.len(0.35)), cr(v.len(0.2)), Color32::from_rgb(0x80, 0x80, 0x7a));
    p.rect_filled(r, cr(v.len(0.3)), Color32::from_rgb(0x12, 0x12, 0x13));
    p.circle_filled(r.left_top() + vec2(v.len(0.7), v.len(0.7)), v.len(0.25), Color32::from_gray(70));
    p.text(r.center(), Align2::CENTER_CENTER, name, FontId::proportional(v.len(1.0).max(6.0)), Color32::from_gray(150));
}

pub fn draw_back(p: &Painter, v: &View, s: &BackState) {
    board_outline(p, v);
    edge_connector(p, v, false, true);
    // USB connector at the top edge
    let usb = v.rect(pos2(BOARD_W / 2.0, 1.4), vec2(8.0, 4.0));
    p.rect_filled(usb, cr(v.len(0.6)), Color32::from_rgb(0xb4, 0xb6, 0xba));
    p.rect_filled(usb.shrink(v.len(0.8)), cr(v.len(0.3)), Color32::from_rgb(0x50, 0x52, 0x55));
    // battery connector
    let bat = v.rect(pos2(10.0, 3.6), vec2(6.4, 4.6));
    p.rect_filled(bat, cr(v.len(0.4)), Color32::from_rgb(0xec, 0xe8, 0xdc));
    p.rect_filled(v.rect(pos2(10.0, 3.0), vec2(4.8, 2.0)), cr(v.len(0.2)), Color32::from_rgb(0xc8, 0xc2, 0xb2));
    p.text(v.p(pos2(10.0, 7.2)), Align2::CENTER_CENTER, "BATTERY", FontId::proportional(v.len(1.2)), SILK);
    // reset button
    let pressed = s.reset_pressed;
    let rb = v.rect(RESET_BTN, vec2(3.4, 3.4));
    p.rect_filled(rb, cr(v.len(0.4)), Color32::from_rgb(0xc0, 0xc0, 0xc0));
    p.circle_filled(
        v.p(RESET_BTN),
        v.len(1.1),
        if pressed {
            Color32::from_gray(20)
        } else if s.hover == Some(Hit::Reset) {
            Color32::from_gray(80)
        } else {
            Color32::from_gray(45)
        },
    );
    p.text(v.p(RESET_BTN + vec2(0.0, 3.0)), Align2::CENTER_CENTER, "RESET", FontId::proportional(v.len(1.2)), SILK);
    // power and USB LEDs
    let pw = if s.powered { 1.0 } else { 0.0 };
    p.rect_filled(v.rect(POWER_LED, vec2(1.0, 1.6)), cr(v.len(0.15)), lerp(LED_OFF, LED_RED, pw));
    glow(p, s.glow, v.p(POWER_LED), v.len(2.6), LED_RED, pw * 0.8);
    let yellow = Color32::from_rgb(0xff, 0xb0, 0x10);
    p.rect_filled(v.rect(USB_LED, vec2(1.0, 1.6)), cr(v.len(0.15)), lerp(LED_OFF, yellow, s.usb_activity));
    glow(p, s.glow, v.p(USB_LED), v.len(2.6), yellow, s.usb_activity);
    p.text(v.p(POWER_LED + vec2(0.0, 2.0)), Align2::CENTER_CENTER, "PWR", FontId::proportional(v.len(1.0)), SILK);
    p.text(v.p(USB_LED + vec2(0.0, 2.0)), Align2::CENTER_CENTER, "USB", FontId::proportional(v.len(1.0)), SILK);
    // processors and sensor
    chip(p, v, pos2(BOARD_W / 2.0, 16.0), vec2(7.0, 7.0), "nRF52833");
    chip(p, v, pos2(15.0, 12.5), vec2(5.0, 5.0), "nRF52820");
    chip(p, v, pos2(36.8, 12.2), vec2(2.4, 2.4), "");
    p.text(v.p(pos2(36.8, 14.6)), Align2::CENTER_CENTER, "LSM303AGR", FontId::proportional(v.len(0.9)), Color32::from_gray(160));
    // speaker
    let sp = v.p(SPEAKER);
    p.circle_filled(sp, v.len(4.6), Color32::from_rgb(0x22, 0x22, 0x24));
    p.circle_stroke(sp, v.len(4.6), Stroke::new(v.len(0.35), Color32::from_gray(90)));
    p.circle_filled(sp, v.len(1.0), Color32::from_gray(8));
    if s.speaker_activity > 0.002 {
        let a = (s.speaker_activity * 4000.0).min(160.0) as u8;
        p.circle_stroke(sp, v.len(5.6), Stroke::new(v.len(0.3), Color32::from_rgba_unmultiplied(0x9a, 0xc8, 0xff, a)));
        p.circle_stroke(sp, v.len(6.8), Stroke::new(v.len(0.25), Color32::from_rgba_unmultiplied(0x9a, 0xc8, 0xff, a / 2)));
    }
    p.text(v.p(SPEAKER + vec2(0.0, 5.9)), Align2::CENTER_CENTER, "SPEAKER", FontId::proportional(v.len(1.1)), SILK);
    // buttons from the back (mirrored)
    for (i, c) in [BUTTON_A, BUTTON_B].into_iter().enumerate() {
        let m = pos2(BOARD_W - c.x, c.y);
        let body = v.rect(m, vec2(6.2, 6.2));
        let col = if s.buttons[i] { Color32::from_gray(150) } else { Color32::from_gray(110) };
        p.rect_filled(body, cr(v.len(0.6)), col);
        p.text(v.p(m), Align2::CENTER_CENTER, if i == 0 { "A" } else { "B" }, FontId::proportional(v.len(2.0)), Color32::from_gray(40));
    }
    let _ = s.optics;
}

/// Radial falloff for the glow sprite (premultiplied white).
pub fn glow_image(size: usize) -> egui::ColorImage {
    let mut px = Vec::with_capacity(size * size);
    let c = (size as f32 - 1.0) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let d = (((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt() / c).min(1.0);
            let a = ((1.0 - d).powf(2.2) * 0.9 + (1.0 - d).powi(8) * 0.6).min(1.0);
            let v = (a * 255.0) as u8;
            px.push(Color32::from_rgba_premultiplied(v, v, v, v));
        }
    }
    egui::ColorImage::new([size, size], px)
}
