//! RTC0..RTC2: 24-bit counter on LFCLK (32.768 kHz) with a 12-bit prescaler.

use crate::env::Env;
use mbsim_core::time::TICKS_PER_LFCLK;
use mbsim_core::{Slot, Ticks};

const TICK: u32 = 1 << 0;
const OVRFLW: u32 = 1 << 1;

#[derive(Debug)]
pub struct Rtc {
    pub id: usize,
    pub slot: Slot,
    pub ncc: usize,
    /// Events: bit 0 TICK, 1 OVRFLW, 16+n COMPARE[n] (INTEN/EVTEN positions).
    pub events: u32,
    /// Events generated since the last collection (bit n = EVENTS at 0x100 + 4n).
    pub pulses: u32,
    pub inten: u32,
    pub evten: u32,
    pub prescaler: u32,
    pub cc: [u32; 4],
    running: bool,
    base: u32,
    base_time: Ticks,
}

impl Rtc {
    /// Generates events: sets the event bits and records them as pulses for PPI.
    #[inline]
    fn ev(&mut self, bits: u32) {
        self.events |= bits;
        self.pulses |= bits;
    }

    pub fn new(id: usize, slot: Slot) -> Self {
        Rtc {
            id,
            slot,
            ncc: if id == 0 { 3 } else { 4 },
            events: 0,
            pulses: 0,
            inten: 0,
            evten: 0,
            prescaler: 0,
            cc: [0; 4],
            running: false,
            base: 0,
            base_time: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Rtc::new(self.id, self.slot);
    }

    pub fn irq(&self) -> bool {
        self.events & self.inten != 0
    }

    fn period(&self) -> Ticks {
        TICKS_PER_LFCLK * (self.prescaler as Ticks + 1)
    }

    pub fn counter(&self, now: Ticks) -> u32 {
        if self.running {
            (self.base as u64 + (now - self.base_time) / self.period()) as u32 & 0xFF_FFFF
        } else {
            self.base
        }
    }

    fn rebase(&mut self, now: Ticks) {
        let k = if self.running { (now - self.base_time) / self.period() } else { 0 };
        self.base = self.counter(now);
        self.base_time += k * self.period();
        if !self.running {
            self.base_time = now;
        }
    }

    fn reschedule(&mut self, env: &mut Env) {
        if !self.running {
            env.cancel(self.slot);
            return;
        }
        let now = env.now;
        let p = self.period();
        let k = (now - self.base_time) / p;
        let cur = (self.base as u64 + k) as u32 & 0xFF_FFFF;
        let mut next = Ticks::MAX;
        if self.inten & TICK != 0 || self.evten & TICK != 0 {
            next = self.base_time + (k + 1) * p;
        }
        let ovf = (0x100_0000 - cur) as u64;
        next = next.min(self.base_time + (k + ovf) * p);
        for i in 0..self.ncc {
            let mut d = (self.cc[i].wrapping_sub(cur) & 0xFF_FFFF) as u64;
            if d == 0 {
                d = 0x100_0000;
            }
            next = next.min(self.base_time + (k + d) * p);
        }
        env.schedule(self.slot, next);
    }

    pub fn read(&mut self, off: u32, now: Ticks) -> u32 {
        match off {
            0x100 => self.events & TICK,
            0x104 => (self.events >> 1) & 1,
            0x140..=0x14C => (self.events >> (16 + (off - 0x140) / 4)) & 1,
            0x304 | 0x308 => self.inten,
            0x340 | 0x344 | 0x348 => self.evten,
            0x504 => self.counter(now),
            0x508 => self.prescaler,
            0x540..=0x54C => self.cc[((off - 0x540) / 4) as usize],
            _ => 0,
        }
    }

    pub fn write(&mut self, off: u32, v: u32, env: &mut Env) {
        let now = env.now;
        match off {
            0x000 if v & 1 != 0 => {
                if !self.running {
                    self.base_time = now;
                    self.running = true;
                }
            }
            0x004 if v & 1 != 0 => {
                self.rebase(now);
                self.running = false;
            }
            0x008 if v & 1 != 0 => {
                self.base = 0;
                self.base_time = now;
            }
            0x00C if v & 1 != 0 => {
                self.rebase(now);
                self.base = 0xFF_FFF0;
            }
            0x100 => self.set_event(TICK, v),
            0x104 => self.set_event(OVRFLW, v),
            0x140..=0x14C => self.set_event(1 << (16 + (off - 0x140) / 4), v),
            0x304 => self.inten |= v & 0xF_0003,
            0x308 => self.inten &= !v,
            0x340 => self.evten = v & 0xF_0003,
            0x344 => self.evten |= v & 0xF_0003,
            0x348 => self.evten &= !v,
            0x508 => {
                if !self.running {
                    self.prescaler = v & 0xFFF;
                }
            }
            0x540..=0x54C => {
                let i = ((off - 0x540) / 4) as usize;
                if i < self.ncc {
                    self.cc[i] = v & 0xFF_FFFF;
                }
            }
            _ => return,
        }
        self.reschedule(env);
    }

    fn set_event(&mut self, bit: u32, v: u32) {
        if v & 1 == 0 {
            self.events &= !bit;
        } else {
            self.events |= bit;
        }
    }

    pub fn event(&mut self, env: &mut Env) {
        if !self.running {
            return;
        }
        let now = env.now;
        let p = self.period();
        let k = (now - self.base_time) / p;
        let cur = (self.base as u64 + k) as u32 & 0xFF_FFFF;
        if (now - self.base_time).is_multiple_of(p) {
            if self.inten & TICK != 0 || self.evten & TICK != 0 {
                self.ev(TICK);
            }
            if cur == 0 && k > 0 {
                self.ev(OVRFLW);
            }
            for i in 0..self.ncc {
                if self.cc[i] == cur {
                    self.ev(1 << (16 + i));
                }
            }
        }
        self.reschedule(env);
    }
}
