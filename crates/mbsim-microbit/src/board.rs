//! The micro:bit V2 board around the nRF52833 (schematic V2.2.1): external pull-ups, buttons,
//! the touch logo, the microphone front end, the LSM303AGR on the internal I2C bus and the UART
//! to the interface MCU.

use crate::interface::{InterfaceMcu, CONFIG_ADDR, STORAGE_ADDR};
use crate::lsm303agr::{Lsm303agr, ACC_ADDR, MAG_ADDR};
use crate::pins::*;
use mbsim_core::time::{to_secs, TICKS_PER_SEC};
use mbsim_core::Ticks;
use mbsim_nrf52833::board::{BoardIo, I2cResult, PinDrive, NUM_PINS};
use std::collections::VecDeque;

/// Board orientation: heading of the top edge (degrees from north, clockwise), roll (right side
/// down positive) and pitch (edge connector down positive).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Orientation {
    pub heading: f64,
    pub roll: f64,
    pub pitch: f64,
}

/// Inputs of the physical world, set by the GUI, scenarios or the debugger.
#[derive(Clone, Debug)]
pub struct Inputs {
    pub button: [bool; 2],
    /// Capacitance of a finger on the logo in pF (0 = not touched).
    pub logo_finger_pf: f64,
    pub orientation: Orientation,
    /// Additional specific force in mg (board frame: x right, y to the top edge, z out of the
    /// front), e.g. for a shake.
    pub accel_extra_mg: [f64; 3],
    /// Earth field in mgauss in world coordinates (east, north, up).
    pub field_world: [f64; 3],
    /// Hard-iron offset of the board in mgauss (board frame).
    pub hard_iron: [f64; 3],
    pub temperature: f64,
    /// Sound at the microphone output: amplitude in mV and frequency (0 = off).
    pub sound_mv: f64,
    pub sound_hz: f64,
    /// RMS noise of the microphone output in mV.
    pub mic_noise_mv: f64,
}

impl Default for Inputs {
    fn default() -> Self {
        Inputs {
            button: [false; 2],
            logo_finger_pf: 0.0,
            orientation: Orientation::default(),
            accel_extra_mg: [0.0; 3],
            // about 0.49 gauss, inclination 66 degrees (central Europe / UK)
            field_world: [0.0, 200.0, -450.0],
            hard_iron: [0.0; 3],
            temperature: 21.0,
            sound_mv: 0.0,
            sound_hz: 440.0,
            mic_noise_mv: 0.2,
        }
    }
}

/// Electrical constants of the board.
#[derive(Clone, Debug)]
pub struct Electrical {
    pub vdd: f64,
    /// Logo pull-up R53.
    pub logo_r: f64,
    /// Capacitance of the logo pad (fitted to the untouched charge count of 194).
    pub logo_pad_pf: f64,
    /// Microphone bias at MIC_IN (R62/R63), measured 73 mV.
    pub mic_bias_v: f64,
    /// RUN_MIC supply filter 120 Ohm / 10 uF.
    pub mic_tau_s: f64,
}

impl Default for Electrical {
    fn default() -> Self {
        Electrical { vdd: 3.3, logo_r: 10e6, logo_pad_pf: 5.69, mic_bias_v: 0.073, mic_tau_s: 1.2e-3 }
    }
}

/// A change of the state the device drives on a pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PinEvent {
    pub t: Ticks,
    pub pin: u8,
    pub drive: PinDrive,
}

/// RC-charged pad (touch logo): voltage and the state that drives it.
#[derive(Clone, Copy, Debug)]
struct Pad {
    v: f64,
    t: Ticks,
    driven: Option<bool>,
    pull: u8,
}

/// A clap or knock: a decaying burst at the microphone.
#[derive(Clone, Copy, Debug)]
struct Burst {
    t: Ticks,
    amplitude_mv: f64,
    decay_s: f64,
}

/// Sound recorded live (the host microphone), placed on the simulated timeline. Blocks arrive as
/// the host records them and are appended to the stream, which plays a little behind its newest
/// sample so that a late block does not leave a gap. The stream is re-anchored when it has run dry
/// or runs too far ahead, as it does when the simulation is not in real time.
#[derive(Debug, Default)]
struct LiveSound {
    /// Samples per second.
    rate: f64,
    /// Simulated time of `samples[0]` in ticks.
    start: f64,
    /// mV at the microphone output.
    samples: VecDeque<f32>,
}

impl LiveSound {
    /// Lead over the simulated time on top of two blocks: the jitter of the host's audio
    /// callbacks and of the real-time pacing.
    const MARGIN_S: f64 = 0.03;

    fn push(&mut self, now: Ticks, rate: f64, mv: &[f32]) {
        if mv.is_empty() || !rate.is_finite() || rate <= 0.0 {
            return;
        }
        if rate != self.rate {
            self.rate = rate;
            self.samples.clear();
        }
        let now = now as f64;
        let tick = TICKS_PER_SEC as f64 / rate;
        // drop what has been played, keeping one sample before `now` for the interpolation
        let played = ((now - self.start) / tick).floor() - 1.0;
        if played >= 1.0 {
            let n = (played as usize).min(self.samples.len());
            self.samples.drain(..n);
            self.start += n as f64 * tick;
        }
        let fresh = self.samples.is_empty();
        self.samples.extend(mv);
        let block = mv.len() as f64 * tick;
        let target = 2.0 * block + Self::MARGIN_S * TICKS_PER_SEC as f64;
        let lead = self.start + self.samples.len() as f64 * tick - now;
        // a lead below one block means the samples before this block ran out
        if fresh || lead < block || lead > 2.0 * target {
            self.start = now + target - self.samples.len() as f64 * tick;
        }
    }

    /// mV at time `now`, linearly interpolated; 0 outside the stream.
    fn at(&self, now: Ticks) -> f64 {
        let x = (now as f64 - self.start) * self.rate / TICKS_PER_SEC as f64;
        if x < 0.0 {
            return 0.0;
        }
        let i = x as usize;
        if i + 1 >= self.samples.len() {
            return 0.0;
        }
        let f = x - i as f64;
        self.samples[i] as f64 * (1.0 - f) + self.samples[i + 1] as f64 * f
    }
}

pub struct MicrobitBoard {
    pub inputs: Inputs,
    pub elec: Electrical,
    pub lsm: Lsm303agr,
    pub interface: InterfaceMcu,
    drives: [PinDrive; NUM_PINS],
    /// Pin changes for the GUI (only collected when `log_pins` is set).
    pub pin_log: Vec<PinEvent>,
    pub log_pins: bool,
    /// Bytes the target sent to the interface MCU (USB serial).
    pub uart_out: Vec<u8>,
    logo: Pad,
    mic_on_since: Option<Ticks>,
    bursts: Vec<Burst>,
    live: LiveSound,
}

impl Default for MicrobitBoard {
    fn default() -> Self {
        Self::new()
    }
}

/// 3x3 rotation matrices (row major).
type Mat = [[f64; 3]; 3];

fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

fn transpose_apply(m: &Mat, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|j| (0..3).map(|i| m[i][j] * v[i]).sum())
}

impl Orientation {
    /// Rotation from board to world coordinates.
    pub fn matrix(&self) -> Mat {
        let (h, a, b) = (-self.heading.to_radians(), self.roll.to_radians(), self.pitch.to_radians());
        let rz = [[h.cos(), -h.sin(), 0.0], [h.sin(), h.cos(), 0.0], [0.0, 0.0, 1.0]];
        let ry = [[a.cos(), 0.0, a.sin()], [0.0, 1.0, 0.0], [-a.sin(), 0.0, a.cos()]];
        let rx = [[1.0, 0.0, 0.0], [0.0, b.cos(), -b.sin()], [0.0, b.sin(), b.cos()]];
        mul(&rz, &mul(&ry, &rx))
    }
}

/// Mounting of the LSM303AGR (on the back of the board): sensor axes in board coordinates,
/// x to the left, y to the top edge, z into the board. Lying face up the sensor reads -1 g on z,
/// as measured on a V2.2 board.
pub fn board_to_sensor(v: [f64; 3]) -> [f64; 3] {
    [-v[0], v[1], -v[2]]
}

/// Deterministic noise in [-1, 1] for a time slot.
fn noise(slot: u64, seed: u64) -> f64 {
    let mut x = slot.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed;
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

impl MicrobitBoard {
    pub fn new() -> Self {
        MicrobitBoard {
            inputs: Inputs::default(),
            elec: Electrical::default(),
            lsm: Lsm303agr::new(),
            interface: InterfaceMcu::new(),
            drives: [PinDrive::default(); NUM_PINS],
            pin_log: Vec::new(),
            log_pins: false,
            uart_out: Vec::new(),
            logo: Pad { v: 3.3, t: 0, driven: None, pull: 0 },
            mic_on_since: None,
            bursts: Vec::new(),
            live: LiveSound::default(),
        }
    }

    /// The state the device drives on a pin.
    pub fn drive(&self, pin: u8) -> PinDrive {
        self.drives[pin as usize]
    }

    /// True when LED (row, col) conducts: row driven high and column driven low.
    pub fn led_on(&self, row: usize, col: usize) -> bool {
        self.drive(ROW[row]).driven() == Some(true) && self.drive(COL[col]).driven() == Some(false)
    }

    /// The microphone LED D70 lights while RUN_MIC powers the microphone.
    pub fn mic_led_on(&self) -> bool {
        self.drive(RUN_MIC).driven() == Some(true)
    }

    /// Specific force in mg in board coordinates.
    pub fn accel_board_mg(&self) -> [f64; 3] {
        let m = self.inputs.orientation.matrix();
        let f = transpose_apply(&m, [0.0, 0.0, 1000.0]);
        [0, 1, 2].map(|i| f[i] + self.inputs.accel_extra_mg[i])
    }

    /// Magnetic field in mgauss in board coordinates.
    pub fn field_board_mgauss(&self) -> [f64; 3] {
        let m = self.inputs.orientation.matrix();
        let b = transpose_apply(&m, self.inputs.field_world);
        [0, 1, 2].map(|i| b[i] + self.inputs.hard_iron[i])
    }

    fn update_sensors(&mut self) {
        self.lsm.accel_mg = board_to_sensor(self.accel_board_mg());
        self.lsm.mag_mgauss = board_to_sensor(self.field_board_mgauss());
        self.lsm.temperature = self.inputs.temperature;
    }

    /// The shared interrupt line (P0.25) is pulled low by the interface MCU or the LSM303AGR.
    pub fn combined_int(&self, now: Ticks) -> bool {
        self.interface.irq() || self.lsm.int1_active(now) || self.lsm.drdy_active(now)
    }

    /// A clap at time `t` (decaying burst at the microphone).
    pub fn clap(&mut self, t: Ticks, amplitude_mv: f64) {
        self.bursts.retain(|b| to_secs(t.saturating_sub(b.t)) < 1.0);
        self.bursts.push(Burst { t, amplitude_mv, decay_s: 0.015 });
    }

    /// A block of sound recorded live at `rate` samples per second, in mV at the microphone
    /// output, that arrived at time `now`. The microphone hears it in real time, a few tens of
    /// milliseconds later.
    pub fn live_sound(&mut self, now: Ticks, rate: f64, mv: &[f32]) {
        self.live.push(now, rate, mv);
    }

    fn logo_tau(&self) -> f64 {
        let c = (self.elec.logo_pad_pf + self.inputs.logo_finger_pf) * 1e-12;
        let r = match self.logo.pull {
            3 | 1 => 1.0 / (1.0 / self.elec.logo_r + 1.0 / 13e3),
            _ => self.elec.logo_r,
        };
        r * c
    }

    fn logo_target(&self) -> f64 {
        let vdd = self.elec.vdd;
        match self.logo.pull {
            // internal pull-down against the external pull-up
            1 => vdd * 13e3 / (13e3 + self.elec.logo_r),
            _ => vdd,
        }
    }

    fn logo_voltage(&self, now: Ticks) -> f64 {
        if let Some(l) = self.logo.driven {
            return if l { self.elec.vdd } else { 0.0 };
        }
        let dt = to_secs(now.saturating_sub(self.logo.t));
        let target = self.logo_target();
        target + (self.logo.v - target) * (-dt / self.logo_tau()).exp()
    }

    /// Time at which the charging logo pad crosses the input threshold (half VDD).
    fn logo_crossing(&self, now: Ticks) -> Ticks {
        if self.logo.driven.is_some() {
            return Ticks::MAX;
        }
        let vth = 0.5 * self.elec.vdd;
        let target = self.logo_target();
        let v = self.logo_voltage(now);
        if (v - vth) * (target - vth) >= 0.0 && (v > vth) == (target > vth) {
            return Ticks::MAX;
        }
        // V(t) = target + (v - target) e^(-dt/tau)
        let dt = self.logo_tau() * ((target - v) / (target - vth)).ln();
        if !dt.is_finite() || dt < 0.0 {
            return Ticks::MAX;
        }
        now + mbsim_core::time::from_secs(dt).max(1)
    }

    /// Re-anchors the pad charge, e.g. when the finger capacitance changes.
    pub fn touch_changed(&mut self, now: Ticks) {
        self.logo.v = self.logo_voltage(now);
        self.logo.t = now;
    }

    fn mic_voltage(&mut self, now: Ticks) -> f64 {
        let Some(on) = self.mic_on_since else { return 0.0 };
        let dt = to_secs(now.saturating_sub(on));
        let supply = 1.0 - (-dt / self.elec.mic_tau_s).exp();
        let t = to_secs(now);
        let mut v = self.elec.mic_bias_v * supply;
        if self.inputs.sound_mv > 0.0 && self.inputs.sound_hz > 0.0 {
            v += self.inputs.sound_mv * 1e-3 * (2.0 * std::f64::consts::PI * self.inputs.sound_hz * t).sin() * supply;
        }
        for b in &self.bursts {
            if now >= b.t {
                let age = to_secs(now - b.t);
                v += b.amplitude_mv * 1e-3 * (-age / b.decay_s).exp() * noise(now / 512, 0xC1A9) * supply;
            }
        }
        v += self.live.at(now) * 1e-3 * supply;
        // white noise, bandwidth about 16 kHz (new value every 61 us), Gaussian-like (sum of 3)
        let slot = now / (TICKS_PER_SEC / 16384);
        let n = (noise(slot, 1) + noise(slot, 2) + noise(slot, 3)) * 1.0;
        v + self.inputs.mic_noise_mv * 1e-3 * n * supply
    }
}

impl BoardIo for MicrobitBoard {
    fn pin_drive(&mut self, now: Ticks, pin: u8, drive: PinDrive) {
        if pin == LOGO {
            let v = self.logo_voltage(now);
            self.logo = Pad { v, t: now, driven: drive.driven(), pull: drive.pull };
        }
        if pin == RUN_MIC {
            match (drive.driven() == Some(true), self.mic_on_since) {
                (true, None) => self.mic_on_since = Some(now),
                (false, Some(_)) => self.mic_on_since = None,
                _ => {}
            }
        }
        self.drives[pin as usize] = drive;
        if self.log_pins {
            self.pin_log.push(PinEvent { t: now, pin, drive });
        }
    }

    fn pin_input(&mut self, now: Ticks, pin: u8, drive: PinDrive) -> bool {
        if let Some(level) = drive.driven() {
            return level;
        }
        match pin {
            BTN_A => !self.inputs.button[0],
            BTN_B => !self.inputs.button[1],
            LOGO => self.logo_voltage(now) > 0.5 * self.elec.vdd,
            I2C_INT_SCL | I2C_INT_SDA | I2C_EXT_SCL | I2C_EXT_SDA | UART_RX => true,
            p if RING.contains(&p) => true,
            I2C_INT_INT => !self.combined_int(now),
            _ => drive.pull_up(),
        }
    }

    fn analog_input(&mut self, now: Ticks, pin: u8) -> f64 {
        let d = self.drive(pin);
        if let Some(level) = d.driven() {
            return if level { self.elec.vdd } else { 0.0 };
        }
        match pin {
            MIC_IN => self.mic_voltage(now),
            p if RING.contains(&p) => self.elec.vdd,
            _ => {
                if d.pull_up() {
                    self.elec.vdd
                } else {
                    0.0
                }
            }
        }
    }

    fn vdd(&self) -> f64 {
        self.elec.vdd
    }

    fn i2c_write(&mut self, now: Ticks, scl: u8, sda: u8, addr: u8, data: &[u8]) -> I2cResult {
        if (scl, sda) != (I2C_INT_SCL, I2C_INT_SDA) {
            return I2cResult::AddrNack;
        }
        self.update_sensors();
        match addr {
            ACC_ADDR => {
                self.lsm.acc_write(data);
                I2cResult::Ack
            }
            MAG_ADDR => {
                self.lsm.mag_write(data, now);
                I2cResult::Ack
            }
            CONFIG_ADDR => {
                self.interface.config_write(data);
                I2cResult::Ack
            }
            STORAGE_ADDR => {
                self.interface.storage_write(data);
                I2cResult::Ack
            }
            _ => I2cResult::AddrNack,
        }
    }

    fn i2c_read(&mut self, now: Ticks, scl: u8, sda: u8, addr: u8, buf: &mut [u8]) -> I2cResult {
        if (scl, sda) != (I2C_INT_SCL, I2C_INT_SDA) {
            return I2cResult::AddrNack;
        }
        self.update_sensors();
        match addr {
            ACC_ADDR => {
                self.lsm.acc_read(buf, now);
                I2cResult::Ack
            }
            MAG_ADDR => {
                self.lsm.mag_read(buf, now);
                I2cResult::Ack
            }
            CONFIG_ADDR => {
                self.interface.config_read(buf);
                I2cResult::Ack
            }
            STORAGE_ADDR => {
                self.interface.storage_read(buf);
                I2cResult::Ack
            }
            _ => I2cResult::AddrNack,
        }
    }

    fn uart_tx(&mut self, _now: Ticks, pin: u8, byte: u8) {
        if pin == UART_TX {
            self.uart_out.push(byte);
        }
    }

    fn temperature(&mut self, _now: Ticks) -> f64 {
        self.inputs.temperature
    }

    fn next_event(&mut self, now: Ticks) -> Ticks {
        self.lsm.next_interrupt(now).min(self.logo_crossing(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_frames() {
        let mut b = MicrobitBoard::new();
        let f = b.accel_board_mg();
        assert!((f[2] - 1000.0).abs() < 1e-9);
        b.inputs.orientation.roll = 90.0;
        let f = b.accel_board_mg();
        assert!((f[0] + 1000.0).abs() < 1e-6, "{f:?}");
        let s = board_to_sensor(f);
        assert!(s[0] > 999.0, "right side down gives positive sensor x");
        b.inputs.orientation.roll = 0.0;
        b.inputs.orientation.pitch = 90.0;
        let s = board_to_sensor(b.accel_board_mg());
        assert!(s[1] > 999.0, "edge connector down gives positive sensor y");
    }

    const MS: Ticks = TICKS_PER_SEC / 1000;

    /// Pushes block `k` (480 samples at 48 kHz, each sample its index in the stream) at `now`.
    fn push_block(s: &mut LiveSound, now: Ticks, k: usize) {
        let block: Vec<f32> = (0..480).map(|j| (k * 480 + j) as f32).collect();
        s.push(now, 48_000.0, &block);
    }

    #[test]
    fn live_sound_plays_in_real_time() {
        let mut s = LiveSound::default();
        for k in 0..200 {
            push_block(&mut s, k as Ticks * 10 * MS, k);
            // two blocks plus the margin behind: sample 0 plays at 50 - 10 = 40 ms
            let t = k as Ticks * 10 * MS + 3 * MS;
            let want = if t < 40 * MS { 0.0 } else { (t - 40 * MS) as f64 * 48_000.0 / TICKS_PER_SEC as f64 };
            assert!((s.at(t) - want).abs() < 1e-3, "block {k}: {} != {want}", s.at(t));
        }
        assert!(s.samples.len() < 4000, "played samples are dropped: {}", s.samples.len());
    }

    #[test]
    fn live_sound_reanchors() {
        let mut s = LiveSound::default();
        for k in 0..10 {
            push_block(&mut s, k as Ticks * 10 * MS, k);
        }
        // the host stops for 200 ms: the stream runs dry, then restarts with the next block
        assert_eq!(s.at(150 * MS), 0.0);
        push_block(&mut s, 300 * MS, 10);
        assert_eq!(s.at(300 * MS), 0.0, "already played samples do not come back");
        assert!((s.at(341 * MS) - 4848.0).abs() < 1e-3, "{}", s.at(341 * MS));
        // the simulation stands still (paused): the stream stays bounded
        for k in 11..1000 {
            push_block(&mut s, 300 * MS, k);
        }
        assert!(s.samples.len() < 7200, "{}", s.samples.len());
    }

    #[test]
    fn live_sound_at_the_microphone() {
        let mut b = MicrobitBoard::new();
        b.inputs.mic_noise_mv = 0.0;
        b.pin_drive(0, RUN_MIC, PinDrive { output: true, level: true, ..Default::default() });
        let bias = b.mic_voltage(100 * MS);
        // a 10 ms block at 100 ms plays from 140 ms to 150 ms
        b.live_sound(100 * MS, 48_000.0, &[20.0; 480]);
        assert_eq!(b.mic_voltage(135 * MS), bias);
        let v = b.mic_voltage(145 * MS);
        assert!((v - bias - 0.020).abs() < 1e-9, "{v} {bias}");
    }
}
