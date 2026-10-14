//! Light output of the board, integrated exactly from the pin-change log.
//!
//! Each LED conducts while its row is driven high and its column driven low. The perceived
//! brightness is the on-state filtered by a first-order low-pass (persistence of vision), which
//! is integrated piecewise between pin changes, so the 1 kHz row multiplexing cannot alias with
//! the frame rate.

use mbsim_core::time::to_secs;
use mbsim_core::Ticks;
use mbsim_microbit::board::PinEvent;
use mbsim_microbit::pins;
use mbsim_nrf52833::board::{PinDrive, NUM_PINS};

/// Duty cycle that appears as full brightness (one of five rows).
pub const FULL_DUTY: f32 = 0.2;

pub struct Optics {
    pub tau: f64,
    pub leds: [[f32; 5]; 5],
    pub mic_led: f32,
    drive: [PinDrive; NUM_PINS],
    last_t: Ticks,
    pending: Vec<PinEvent>,
    /// Speaker gate level changes (for the audio).
    pub speaker_edges: Vec<(Ticks, bool)>,
    /// Instantaneous LED state (camera view).
    pub instant: [[bool; 5]; 5],
}

impl Default for Optics {
    fn default() -> Self {
        Self::new()
    }
}

impl Optics {
    pub fn new() -> Self {
        Optics {
            tau: 0.012,
            leds: [[0.0; 5]; 5],
            mic_led: 0.0,
            drive: [PinDrive::default(); NUM_PINS],
            last_t: 0,
            pending: Vec::new(),
            speaker_edges: Vec::new(),
            instant: [[false; 5]; 5],
        }
    }

    fn led_on(&self, r: usize, c: usize) -> bool {
        self.drive[pins::ROW[r] as usize].driven() == Some(true) && self.drive[pins::COL[c] as usize].driven() == Some(false)
    }

    fn advance(&mut self, t: Ticks) {
        if t <= self.last_t {
            return;
        }
        let dt = to_secs(t - self.last_t);
        let k = (-dt / self.tau).exp() as f32;
        for r in 0..5 {
            for c in 0..5 {
                let s = if self.led_on(r, c) { 1.0 } else { 0.0 };
                self.leds[r][c] = s + (self.leds[r][c] - s) * k;
            }
        }
        let s = if self.drive[pins::RUN_MIC as usize].driven() == Some(true) { 1.0 } else { 0.0 };
        self.mic_led = s + (self.mic_led - s) * k;
        self.last_t = t;
    }

    /// Feeds new pin events and integrates up to `now`.
    pub fn update(&mut self, mut events: Vec<PinEvent>, now: Ticks) {
        self.pending.append(&mut events);
        self.pending.sort_by_key(|e| e.t);
        let split = self.pending.partition_point(|e| e.t <= now);
        let due: Vec<PinEvent> = self.pending.drain(..split).collect();
        for e in due {
            self.advance(e.t);
            let was = self.drive[e.pin as usize].driven() == Some(true);
            self.drive[e.pin as usize] = e.drive;
            if e.pin == pins::SPEAKER {
                let is = e.drive.driven() == Some(true);
                if is != was {
                    self.speaker_edges.push((e.t, is));
                }
            }
        }
        self.advance(now);
        for r in 0..5 {
            for c in 0..5 {
                self.instant[r][c] = self.led_on(r, c);
            }
        }
        if self.speaker_edges.len() > 100_000 {
            let n = self.speaker_edges.len() - 50_000;
            self.speaker_edges.drain(..n);
        }
    }

    pub fn drive(&self, pin: u8) -> PinDrive {
        self.drive[pin as usize]
    }

    /// Relative luminance of an LED (1.0 = full brightness of the multiplexed display).
    pub fn luminance(&self, r: usize, c: usize) -> f32 {
        self.leds[r][c] / FULL_DUTY
    }
}
