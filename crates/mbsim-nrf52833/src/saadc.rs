//! SAADC: successive-approximation ADC with EasyDMA.
//!
//! RESULT = (V(P) - V(N)) * GAIN / REFERENCE * 2^(RESOLUTION - m), m = 0 single-ended, 1
//! differential. Input voltages come from the board at the end of the acquisition time.

use crate::env::{inten_write, Env};
use mbsim_core::time::us;
use mbsim_core::{Slot, Ticks};

const STARTED: u32 = 1 << 0;
const END: u32 = 1 << 1;
const DONE: u32 = 1 << 2;
const RESULTDONE: u32 = 1 << 3;
const CALIBRATEDONE: u32 = 1 << 4;
const STOPPED: u32 = 1 << 5;

/// Analog input to pin: AIN0..AIN7.
pub const AIN_PINS: [u8; 8] = [2, 3, 4, 5, 28, 29, 30, 31];

#[derive(Clone, Copy, Debug, Default)]
pub struct Channel {
    pub pselp: u32,
    pub pseln: u32,
    pub config: u32,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Started,
    Conversion,
    Calibration,
    Stopped,
}

#[derive(Debug)]
pub struct Saadc {
    pub slot: Slot,
    pub events: u32,
    /// Events generated since the last collection (bit n = EVENTS at 0x100 + 4n).
    pub pulses: u32,
    pub inten: u32,
    pub enable: u32,
    pub ch: [Channel; 8],
    pub resolution: u32,
    pub oversample: u32,
    pub samplerate: u32,
    pub result_ptr: u32,
    pub result_maxcnt: u32,
    pub result_amount: u32,
    started: bool,
    busy: bool,
    pending: Option<Pending>,
    buf_ptr: u32,
    buf_max: u32,
    count: u32,
    /// Next channel of a scan in progress.
    scan_index: usize,
    /// Accumulated oversampling samples.
    os_acc: i64,
    os_n: u32,
    /// RESULT.AMOUNT restarts with the first sample written after START, so the value of the
    /// previous buffer stays readable when END restarts the SAADC through PPI.
    amount_reset_pending: bool,
}

impl Saadc {
    /// Generates events: sets the event bits and records them as pulses for PPI.
    #[inline]
    fn ev(&mut self, bits: u32) {
        self.events |= bits;
        self.pulses |= bits;
    }

    pub fn new(slot: Slot) -> Self {
        Saadc {
            slot,
            events: 0,
            pulses: 0,
            inten: 0,
            enable: 0,
            ch: [Channel { config: 0x0002_0000, ..Default::default() }; 8],
            resolution: 1,
            oversample: 0,
            samplerate: 0,
            result_ptr: 0,
            result_maxcnt: 0,
            result_amount: 0,
            started: false,
            busy: false,
            pending: None,
            buf_ptr: 0,
            buf_max: 0,
            count: 0,
            scan_index: 0,
            os_acc: 0,
            os_n: 0,
            amount_reset_pending: false,
        }
    }

    pub fn reset(&mut self) {
        *self = Saadc::new(self.slot);
    }

    pub fn irq(&self) -> bool {
        self.events & self.inten != 0
    }

    fn event_bit(off: u32) -> Option<u32> {
        match off {
            0x100 => Some(STARTED),
            0x104 => Some(END),
            0x108 => Some(DONE),
            0x10C => Some(RESULTDONE),
            0x110 => Some(CALIBRATEDONE),
            0x114 => Some(STOPPED),
            0x118..=0x154 => Some(1 << (6 + (off - 0x118) / 4)),
            _ => None,
        }
    }

    pub fn read(&mut self, off: u32) -> u32 {
        if let Some(b) = Self::event_bit(off) {
            return (self.events & b != 0) as u32;
        }
        match off {
            0x300 | 0x304 | 0x308 => self.inten,
            0x400 => self.busy as u32,
            0x500 => self.enable,
            0x510..=0x58C => {
                let c = &self.ch[((off - 0x510) / 16) as usize];
                match (off - 0x510) % 16 {
                    0 => c.pselp,
                    4 => c.pseln,
                    8 => c.config,
                    _ => c.limit,
                }
            }
            0x5F0 => self.resolution,
            0x5F4 => self.oversample,
            0x5F8 => self.samplerate,
            0x62C => self.result_ptr,
            0x630 => self.result_maxcnt,
            0x634 => self.result_amount,
            _ => 0,
        }
    }

    pub fn write(&mut self, off: u32, v: u32, env: &mut Env) {
        if let Some(b) = Self::event_bit(off) {
            if v & 1 == 0 {
                self.events &= !b;
            } else {
                self.events |= b;
            }
            return;
        }
        if inten_write(&mut self.inten, off, v) {
            self.inten &= 0x003F_FFFF;
            return;
        }
        let now = env.now;
        match off {
            0x000 if v & 1 != 0 && self.enable & 1 != 0 => {
                self.buf_ptr = self.result_ptr;
                self.buf_max = self.result_maxcnt & 0x7FFF;
                self.count = 0;
                self.amount_reset_pending = true;
                self.started = true;
                self.pending = Some(Pending::Started);
                env.schedule(self.slot, now + us(1) / 4);
            }
            0x004 if v & 1 != 0 && self.enable & 1 != 0 && self.started && !self.busy => {
                self.busy = true;
                self.scan_index = 0;
                self.schedule_conversion(now, env);
            }
            0x008 if v & 1 != 0 => {
                self.busy = false;
                self.started = false;
                self.pending = Some(Pending::Stopped);
                env.schedule(self.slot, now + us(1) / 4);
            }
            0x00C if v & 1 != 0 => {
                self.pending = Some(Pending::Calibration);
                env.schedule(self.slot, now + us(20));
            }
            0x500 => self.enable = v & 1,
            0x510..=0x58C => {
                let c = &mut self.ch[((off - 0x510) / 16) as usize];
                match (off - 0x510) % 16 {
                    0 => c.pselp = v & 0x1F,
                    4 => c.pseln = v & 0x1F,
                    8 => c.config = v & 0x0117_1733,
                    _ => c.limit = v,
                }
            }
            0x5F0 => self.resolution = v & 7,
            0x5F4 => self.oversample = v & 0xF,
            0x5F8 => {
                self.samplerate = v & 0x17FF;
                if self.samplerate & (1 << 12) != 0 {
                    env.warn.warn(now, "SAADC: SAMPLERATE MODE=Timers is not modelled");
                }
            }
            0x62C => self.result_ptr = v,
            0x630 => self.result_maxcnt = v & 0x7FFF,
            _ => {}
        }
    }

    fn active_channels(&self) -> Vec<usize> {
        (0..8).filter(|&i| self.ch[i].pselp != 0).collect()
    }

    fn tacq(&self, ch: usize) -> Ticks {
        match (self.ch[ch].config >> 16) & 7 {
            0 => us(3),
            1 => us(5),
            2 => us(10),
            3 => us(15),
            4 => us(20),
            _ => us(40),
        }
    }

    fn schedule_conversion(&mut self, now: Ticks, env: &mut Env) {
        let chans = self.active_channels();
        if let Some(&c) = chans.get(self.scan_index) {
            self.pending = Some(Pending::Conversion);
            env.schedule(self.slot, now + self.tacq(c) + us(2));
        } else {
            self.busy = false;
        }
    }

    fn input_voltage(&self, sel: u32, env: &mut Env, t: Ticks) -> f64 {
        match sel {
            1..=8 => env.board.analog_input(t, AIN_PINS[(sel - 1) as usize]),
            9 => env.board.vdd(),
            0x0D => env.board.vdd() / 5.0,
            _ => 0.0,
        }
    }

    fn convert(&mut self, ch: usize, env: &mut Env) -> i16 {
        let t = env.now;
        let c = self.ch[ch];
        let vp = self.input_voltage(c.pselp, env, t);
        let diff = (c.config >> 20) & 1 == 1;
        let vn = if diff { self.input_voltage(c.pseln, env, t) } else { 0.0 };
        let gain = match (c.config >> 8) & 7 {
            0 => 1.0 / 6.0,
            1 => 1.0 / 5.0,
            2 => 1.0 / 4.0,
            3 => 1.0 / 3.0,
            4 => 1.0 / 2.0,
            5 => 1.0,
            6 => 2.0,
            _ => 4.0,
        };
        let reference = if (c.config >> 12) & 1 == 1 { env.board.vdd() / 4.0 } else { 0.6 };
        let bits = match self.resolution {
            0 => 8,
            1 => 10,
            2 => 12,
            _ => 14,
        };
        let m = if diff { 1 } else { 0 };
        let scale = (1i64 << (bits - m)) as f64;
        let raw = ((vp - vn) * gain / reference * scale).round() as i64;
        let (lo, hi) = if diff { (-(1i64 << (bits - 1)), (1i64 << (bits - 1)) - 1) } else { (-(1i64 << bits), (1i64 << bits) - 1) };
        raw.clamp(lo, hi) as i16
    }

    pub fn event(&mut self, env: &mut Env) {
        match self.pending.take() {
            Some(Pending::Started) => self.ev(STARTED),
            Some(Pending::Stopped) => {
                if self.count > 0 {
                    self.result_amount = self.count;
                }
                self.ev(STOPPED | END);
            }
            Some(Pending::Calibration) => self.ev(CALIBRATEDONE),
            Some(Pending::Conversion) => {
                let chans = self.active_channels();
                let ch = chans[self.scan_index.min(chans.len() - 1)];
                let v = self.convert(ch, env);
                let os = 1u32 << self.oversample.min(8);
                let single = chans.len() == 1;
                if single && os > 1 && (self.ch[ch].config >> 24) & 1 == 0 {
                    // oversampling without BURST: one SAMPLE task per sample
                    self.os_acc += v as i64;
                    self.os_n += 1;
                    self.ev(DONE);
                    if self.os_n < os {
                        self.busy = false;
                        return;
                    }
                }
                let result = if self.os_n > 0 {
                    let r = (self.os_acc / self.os_n as i64) as i16;
                    self.os_acc = 0;
                    self.os_n = 0;
                    r
                } else {
                    v
                };
                self.ev(DONE | RESULTDONE);
                // limit events
                let lim = self.ch[ch].limit;
                let (low, high) = ((lim & 0xFFFF) as i16, (lim >> 16) as i16);
                if result > high {
                    self.ev(1 << (6 + 2 * ch));
                }
                if result < low {
                    self.ev(1 << (7 + 2 * ch));
                }
                if self.count < self.buf_max {
                    env.dma_write(self.buf_ptr + 2 * self.count, &result.to_le_bytes(), "SAADC");
                    self.count += 1;
                    self.amount_reset_pending = false;
                    self.result_amount = self.count;
                }
                self.scan_index += 1;
                if self.scan_index < chans.len() {
                    let now = env.now;
                    self.schedule_conversion(now, env);
                    return;
                }
                self.busy = false;
                if self.count >= self.buf_max {
                    self.ev(END);
                    self.started = false;
                }
            }
            None => {}
        }
    }
}
