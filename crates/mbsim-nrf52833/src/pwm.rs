//! PWM0..PWM3 with the EasyDMA sequence decoder (PS v1.3 section 6.16).
//!
//! Every PWM period is one scheduled event: the decoder loads the next value and the output edges
//! of the period are reported to the board with their exact times.
//! Polarity (bit 15 of a value): 0 = RisingEdge (period starts low), 1 = FallingEdge (starts high).

use crate::env::{inten_write, Env};
use crate::gpio::{psel_pin, PinOverride};
use mbsim_core::time::TICKS_PER_16MHZ;
use mbsim_core::{Slot, Ticks};

const STOPPED: u32 = 1 << 1;
const SEQSTARTED0: u32 = 1 << 2;
const SEQEND0: u32 = 1 << 4;
const PWMPERIODEND: u32 = 1 << 6;
const LOOPSDONE: u32 = 1 << 7;

#[derive(Clone, Copy, Debug, Default)]
pub struct SeqRegs {
    pub ptr: u32,
    pub cnt: u32,
    pub refresh: u32,
    pub enddelay: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// Playing value `index` of the sequence, `refresh_left` more periods with it.
    Value { index: u32, refresh_left: u32 },
    /// End delay after the last value.
    EndDelay { left: u32 },
    /// Holding the last value (sequence complete).
    Hold,
}

#[derive(Debug)]
pub struct Pwm {
    pub id: usize,
    pub slot: Slot,
    pub events: u32,
    /// Events generated since the last collection (bit n = EVENTS at 0x100 + 4n).
    pub pulses: u32,
    pub inten: u32,
    pub shorts: u32,
    pub enable: u32,
    pub mode: u32,
    pub countertop: u32,
    pub prescaler: u32,
    pub decoder: u32,
    pub loop_cnt: u32,
    pub seq: [SeqRegs; 2],
    pub psel: [u32; 4],
    running: bool,
    period_start: Ticks,
    comp: [u16; 4],
    top: u16,
    cur_seq: usize,
    stage: Stage,
    loops_left: u32,
    start_request: Option<usize>,
    stop_request: bool,
    nextstep_request: bool,
    pins: [Option<u8>; 4],
}

impl Pwm {
    /// Generates events: sets the event bits and records them as pulses for PPI.
    #[inline]
    fn ev(&mut self, bits: u32) {
        self.events |= bits;
        self.pulses |= bits;
    }

    pub fn new(id: usize, slot: Slot) -> Self {
        Pwm {
            id,
            slot,
            events: 0,
            pulses: 0,
            inten: 0,
            shorts: 0,
            enable: 0,
            mode: 0,
            countertop: 0x3FF,
            prescaler: 0,
            decoder: 0,
            loop_cnt: 0,
            seq: [SeqRegs::default(); 2],
            psel: [0xFFFF_FFFF; 4],
            running: false,
            period_start: 0,
            comp: [0; 4],
            top: 0x3FF,
            cur_seq: 0,
            stage: Stage::Hold,
            loops_left: 0,
            start_request: None,
            stop_request: false,
            nextstep_request: false,
            pins: [None; 4],
        }
    }

    pub fn reset(&mut self) {
        *self = Pwm::new(self.id, self.slot);
    }

    pub fn irq(&self) -> bool {
        self.events & self.inten != 0
    }

    fn event_bit(off: u32) -> Option<u32> {
        Some(match off {
            0x104 => STOPPED,
            0x108 => SEQSTARTED0,
            0x10C => SEQSTARTED0 << 1,
            0x110 => SEQEND0,
            0x114 => SEQEND0 << 1,
            0x118 => PWMPERIODEND,
            0x11C => LOOPSDONE,
            _ => return None,
        })
    }

    pub fn read(&mut self, off: u32) -> u32 {
        if let Some(b) = Self::event_bit(off) {
            return (self.events & b != 0) as u32;
        }
        match off {
            0x200 => self.shorts,
            0x300 | 0x304 | 0x308 => self.inten,
            0x500 => self.enable,
            0x504 => self.mode,
            0x508 => self.countertop,
            0x50C => self.prescaler,
            0x510 => self.decoder,
            0x514 => self.loop_cnt,
            0x520..=0x54C => {
                let s = &self.seq[((off - 0x520) / 0x20) as usize];
                match (off - 0x520) % 0x20 {
                    0 => s.ptr,
                    4 => s.cnt,
                    8 => s.refresh,
                    12 => s.enddelay,
                    _ => 0,
                }
            }
            0x560..=0x56C => self.psel[((off - 0x560) / 4) as usize],
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
            self.inten &= 0xFE;
            return;
        }
        match off {
            0x004 if v & 1 != 0 => {
                if self.running {
                    self.stop_request = true;
                } else {
                    self.ev(STOPPED);
                }
            }
            0x008 | 0x00C if v & 1 != 0 => self.seq_start(((off - 0x008) / 4) as usize, env),
            0x010 if v & 1 != 0 => self.nextstep_request = true,
            0x200 => self.shorts = v & 0x1F,
            0x500 => {
                self.enable = v & 1;
                if self.enable == 0 {
                    self.halt(env);
                }
            }
            0x504 => self.mode = v & 1,
            0x508 => self.countertop = v & 0x7FFF,
            0x50C => self.prescaler = v & 7,
            0x510 => self.decoder = v & 0x103,
            0x514 => self.loop_cnt = v & 0xFFFF,
            0x520..=0x54C => {
                let s = &mut self.seq[((off - 0x520) / 0x20) as usize];
                match (off - 0x520) % 0x20 {
                    0 => s.ptr = v,
                    4 => s.cnt = v & 0x7FFF,
                    8 => s.refresh = v & 0xFF_FFFF,
                    12 => s.enddelay = v & 0xFF_FFFF,
                    _ => {}
                }
            }
            0x560..=0x56C => self.psel[((off - 0x560) / 4) as usize] = v,
            _ => {}
        }
    }

    fn halt(&mut self, env: &mut Env) {
        self.running = false;
        env.cancel(self.slot);
        for p in self.pins.iter_mut() {
            if let Some(pin) = p.take() {
                env.gpio.set_override(pin, None, env.now, env.board);
            }
        }
    }

    fn halfwords_per_step(&self) -> u32 {
        match self.decoder & 3 {
            0 => 1,
            1 => 2,
            _ => 4,
        }
    }

    fn steps(&self, s: usize) -> u32 {
        self.seq[s].cnt / self.halfwords_per_step()
    }

    fn clk(&self) -> Ticks {
        TICKS_PER_16MHZ << self.prescaler
    }

    fn period(&self) -> Ticks {
        let top = self.top.max(3) as Ticks;
        if self.mode & 1 == 1 {
            2 * top * self.clk()
        } else {
            top * self.clk()
        }
    }

    fn load_value(&mut self, s: usize, index: u32, env: &mut Env) {
        let n = self.halfwords_per_step();
        let mut buf = vec![0u8; 2 * n as usize];
        let addr = self.seq[s].ptr + 2 * n * index;
        env.dma_read(addr, &mut buf, "PWM");
        let hw: Vec<u16> = buf.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        match self.decoder & 3 {
            0 => self.comp = [hw[0]; 4],
            1 => self.comp = [hw[0], hw[0], hw[1], hw[1]],
            2 => self.comp = [hw[0], hw[1], hw[2], hw[3]],
            _ => {
                self.comp = [hw[0], hw[1], hw[2], 0];
                self.top = hw[3] & 0x7FFF;
            }
        }
    }

    fn begin_sequence(&mut self, s: usize, env: &mut Env) {
        self.cur_seq = s;
        if self.steps(s) == 0 {
            self.stage = Stage::Hold;
            return;
        }
        self.load_value(s, 0, env);
        self.ev(SEQSTARTED0 << s);
        if self.steps(s) == 1 {
            self.seq_end(s);
        }
        self.stage = Stage::Value { index: 0, refresh_left: self.seq[s].refresh };
    }

    fn seq_start(&mut self, s: usize, env: &mut Env) {
        if self.enable & 1 == 0 {
            return;
        }
        if self.running {
            self.start_request = Some(s);
            return;
        }
        self.pins = [psel_pin(self.psel[0]), psel_pin(self.psel[1]), psel_pin(self.psel[2]), psel_pin(self.psel[3])];
        if self.decoder & 3 != 3 {
            self.top = self.countertop as u16;
        }
        self.loops_left = self.loop_cnt;
        self.running = true;
        self.stop_request = false;
        self.begin_sequence(s, env);
        self.period_start = env.now;
        self.emit_period(env);
        let t = self.period_start + self.period();
        env.schedule(self.slot, t);
    }

    /// Re-reads PSEL.OUT (it takes effect immediately) and releases pins no longer used.
    fn update_pins(&mut self, env: &mut Env) {
        let new = [psel_pin(self.psel[0]), psel_pin(self.psel[1]), psel_pin(self.psel[2]), psel_pin(self.psel[3])];
        for p in self.pins.into_iter().flatten() {
            if !new.contains(&Some(p)) {
                env.gpio.set_override(p, None, env.now, env.board);
            }
        }
        self.pins = new;
    }

    /// Reports the output edges of the period starting at `period_start`.
    fn emit_period(&mut self, env: &mut Env) {
        self.update_pins(env);
        let top = self.top as u32;
        let clk = self.clk();
        let t0 = self.period_start;
        let updown = self.mode & 1 == 1;
        for ch in 0..4 {
            let Some(pin) = self.pins[ch] else { continue };
            let c = self.comp[if self.decoder & 3 == 3 && ch == 3 { 2 } else { ch }];
            let falling = c & 0x8000 != 0;
            let cmp = (c & 0x7FFF) as u32;
            let start_level = if falling { cmp > 0 } else { cmp == 0 };
            env.gpio.set_override(pin, Some(PinOverride::value(start_level)), t0.max(env.now), env.board);
            env.gpio.report_edge(pin, t0.max(env.now), start_level, env.board);
            if cmp > 0 && cmp < top {
                let e1 = t0 + cmp as Ticks * clk;
                env.gpio.report_edge(pin, e1, !start_level, env.board);
                if updown {
                    let e2 = t0 + (2 * top - cmp) as Ticks * clk;
                    env.gpio.report_edge(pin, e2, start_level, env.board);
                }
            }
        }
    }

    /// End of a PWM period.
    pub fn event(&mut self, env: &mut Env) {
        if !self.running {
            return;
        }
        let now = env.now;
        self.ev(PWMPERIODEND);
        if self.stop_request {
            self.halt(env);
            self.ev(STOPPED);
            return;
        }
        if self.decoder & 3 != 3 {
            self.top = self.countertop as u16;
        }
        if let Some(s) = self.start_request.take() {
            self.loops_left = self.loop_cnt;
            self.begin_sequence(s, env);
        } else {
            self.advance(env);
        }
        if !self.running {
            return;
        }
        self.period_start = now;
        self.emit_period(env);
        let t = now + self.period();
        env.schedule(self.slot, t);
    }

    fn advance(&mut self, env: &mut Env) {
        let s = self.cur_seq;
        let nextstep = self.decoder & 0x100 != 0;
        match self.stage {
            Stage::Value { index, refresh_left } => {
                if nextstep {
                    if !std::mem::take(&mut self.nextstep_request) {
                        return;
                    }
                } else if refresh_left > 0 {
                    self.stage = Stage::Value { index, refresh_left: refresh_left - 1 };
                    return;
                }
                let next = index + 1;
                if next < self.steps(s) {
                    self.load_value(s, next, env);
                    if next + 1 == self.steps(s) {
                        self.seq_end(s);
                    }
                    self.stage = Stage::Value { index: next, refresh_left: self.seq[s].refresh };
                } else if self.seq[s].enddelay > 0 && !nextstep {
                    self.stage = Stage::EndDelay { left: self.seq[s].enddelay - 1 };
                } else {
                    self.sequence_complete(env);
                }
            }
            Stage::EndDelay { left } => {
                if left > 0 {
                    self.stage = Stage::EndDelay { left: left - 1 };
                } else {
                    self.sequence_complete(env);
                }
            }
            Stage::Hold => {}
        }
    }

    fn seq_end(&mut self, s: usize) {
        self.ev(SEQEND0 << s);
        if self.shorts & (1 << s) != 0 {
            self.stop_request = true;
        }
    }

    fn sequence_complete(&mut self, env: &mut Env) {
        if self.loop_cnt == 0 {
            self.stage = Stage::Hold;
            return;
        }
        if self.cur_seq == 0 {
            self.begin_sequence(1, env);
            return;
        }
        self.loops_left = self.loops_left.saturating_sub(1);
        if self.loops_left > 0 {
            self.begin_sequence(0, env);
            return;
        }
        self.ev(LOOPSDONE);
        self.stage = Stage::Hold;
        if self.shorts & (1 << 2) != 0 {
            self.loops_left = self.loop_cnt;
            self.begin_sequence(0, env);
        } else if self.shorts & (1 << 3) != 0 {
            self.loops_left = self.loop_cnt;
            self.begin_sequence(1, env);
        } else if self.shorts & (1 << 4) != 0 {
            self.stop_request = true;
        }
    }
}
